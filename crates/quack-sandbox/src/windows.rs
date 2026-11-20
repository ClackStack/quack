//! Windows OS sandbox: Restricted Token (`DISABLE_MAX_PRIVILEGE` + `LUA_TOKEN` fallback)
//! lowered to Low Integrity (`S-1-16-4096`, `NO_WRITE_UP`) combined with a Job Object
//! (`KILL_ON_JOB_CLOSE` + `BasicUIRestrictions`).
//!
//! Normal user directories and system paths on NTFS sit at Medium Integrity with
//! `NO_WRITE_UP`, so a Low-Integrity process can read and run dev toolchains (`pwsh`, `cmd`,
//! `git`, `cargo`, `python`) while the kernel refuses writes, deletes and renames outside
//! the paths granted a Low-Integrity mandatory label (`Policy::writable`).
//!
//! Note: Windows Low Integrity enforces filesystem write, privilege, and UI/clipboard
//! isolation without requiring Administrator rights or AppContainer toolchain breakage;
//! network sockets remain unrestricted on Windows without an AppContainer/WFP policy.
//!
//! The user's temp folder is never labelled: a temp folder in `Policy::writable` becomes
//! `<temp>\quack-sandbox`, and the child's `TEMP`/`TMP` point there. Labelling a folder
//! relabels everything already inside it and lasts after quack exits, so each path is
//! labelled once per process.
#![allow(unsafe_code)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{FromRawHandle, RawHandle};
use std::os::windows::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{ExitStatus, Stdio};
use std::ptr;
use std::sync::{Mutex, OnceLock, PoisonError};

use windows_sys::Win32::Foundation::{
    CloseHandle, FALSE, GetLastError, HANDLE, HANDLE_FLAG_INHERIT, INVALID_HANDLE_VALUE, LocalFree,
    SetHandleInformation, TRUE, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, ConvertStringSidToSidW, SDDL_REVISION_1,
    SE_FILE_OBJECT, SetNamedSecurityInfoW,
};
use windows_sys::Win32::Security::{
    CreateRestrictedToken, DISABLE_MAX_PRIVILEGE, GetSecurityDescriptorSacl,
    LABEL_SECURITY_INFORMATION, LUA_TOKEN, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES,
    SID_AND_ATTRIBUTES, SetTokenInformation, TOKEN_ADJUST_DEFAULT, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TokenIntegrityLevel,
};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    OPEN_EXISTING,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOB_OBJECT_UILIMIT_DESKTOP, JOB_OBJECT_UILIMIT_DISPLAYSETTINGS, JOB_OBJECT_UILIMIT_EXITWINDOWS,
    JOB_OBJECT_UILIMIT_GLOBALATOMS, JOB_OBJECT_UILIMIT_HANDLES, JOB_OBJECT_UILIMIT_READCLIPBOARD,
    JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS, JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    JOBOBJECT_BASIC_UI_RESTRICTIONS, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectBasicUIRestrictions, JobObjectExtendedLimitInformation, SetInformationJobObject,
    TerminateJobObject,
};
use windows_sys::Win32::System::Pipes::CreatePipe;
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_SUSPENDED, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW,
    GetCurrentProcess, GetExitCodeProcess, INFINITE, OpenProcessToken, PROCESS_INFORMATION,
    ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOW, TerminateProcess, WaitForSingleObject,
};

use crate::Policy;

/// Low Integrity Level SID (`SECURITY_MANDATORY_LOW_RID`).
const LOW_INTEGRITY_SID: &str = "S-1-16-4096";
/// Inheritable Low-Integrity mandatory label SACL (`NO_WRITE_UP` for Low Integrity).
const LOW_INTEGRITY_SACL_SDDL: &str = "S:(ML;OICI;NW;;;LW)";
/// Win32 `SE_GROUP_INTEGRITY` attribute for `TOKEN_MANDATORY_LABEL`.
const SE_GROUP_INTEGRITY: u32 = 0x0000_0020;
/// What [`available`] reports when the probe passes.
const MECHANISM: &str = "Restricted Token (Low Integrity; filesystem write isolated)";
/// How long the probe's `cmd /c exit 0` may take.
const PROBE_TIMEOUT_MS: u32 = 10_000;
/// The folder inside the temp folder that sandboxed processes use as their temp folder.
const SANDBOX_TEMP: &str = "quack-sandbox";

fn wide(s: &OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

fn wide_str(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

struct OwnedHandle(HANDLE);

// SAFETY: Windows kernel handles (tokens, jobs, processes) can be closed from any thread.
unsafe impl Send for OwnedHandle {}
// SAFETY: Win32 handle APIs used here are thread-safe.
unsafe impl Sync for OwnedHandle {}

impl OwnedHandle {
    fn new(raw: HANDLE) -> Option<Self> {
        (!raw.is_null() && raw != INVALID_HANDLE_VALUE).then_some(Self(raw))
    }

    fn raw(&self) -> HANDLE {
        self.0
    }
}

impl Drop for OwnedHandle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 != INVALID_HANDLE_VALUE {
            // SAFETY: we exclusively own this handle and close it once.
            unsafe {
                CloseHandle(self.0);
            }
        }
    }
}

/// Probe whether Windows Restricted Token + Low Integrity sandboxing works on this host.
///
/// Creating the token and the job proves little: on some hosts (a non-interactive
/// service session, for one) a Low Integrity process can't even start. So the first
/// call runs `cmd /c exit 0` in the sandbox and every later call reuses the answer.
pub(crate) fn available() -> Result<&'static str, String> {
    static PROBE: OnceLock<Result<&'static str, String>> = OnceLock::new();
    PROBE.get_or_init(probe).clone()
}

fn probe() -> Result<&'static str, String> {
    let cmd = WindowsCommand::unchecked(
        &Policy::default(),
        system_cmd().as_os_str(),
        ["/D", "/C", "exit 0"],
    );
    let child = cmd
        .launch()
        .map_err(|e| format!("a Low Integrity process could not start: {e}"))?;
    // SAFETY: `child.process` is a valid process handle owned here.
    let wait = unsafe { WaitForSingleObject(child.process.raw(), PROBE_TIMEOUT_MS) };
    if wait != WAIT_OBJECT_0 {
        // SAFETY: `child.job` is a valid job handle owned here.
        unsafe {
            TerminateJobObject(child.job.raw(), 1);
        }
        return Err(format!(
            "a Low Integrity test process did not finish within {} s",
            PROBE_TIMEOUT_MS / 1000
        ));
    }
    let mut code: u32 = 1;
    // SAFETY: the process has exited; `code` receives its status.
    if unsafe { GetExitCodeProcess(child.process.raw(), &mut code) } == 0 {
        return Err(format!(
            "could not read the test process's exit code: {}",
            io::Error::last_os_error()
        ));
    }
    if code != 0 {
        return Err(exit_problem(code));
    }
    Ok(MECHANISM)
}

/// `cmd.exe` from the Windows folder, so the probe doesn't depend on `PATH`.
fn system_cmd() -> PathBuf {
    std::env::var_os("SystemRoot")
        .filter(|root| !root.is_empty())
        .map_or_else(
            || PathBuf::from("cmd.exe"),
            |root| PathBuf::from(root).join("System32").join("cmd.exe"),
        )
}

/// Why the probe's `cmd /c exit 0` failed, from its exit code.
fn exit_problem(code: u32) -> String {
    let why = match code {
        0xC000_0142 => {
            ": STATUS_DLL_INIT_FAILED, it could not open the window station or desktop \
             (common in non-interactive service sessions)"
        }
        0xC000_0022 => ": STATUS_ACCESS_DENIED",
        0xC000_0135 => ": STATUS_DLL_NOT_FOUND",
        _ => "",
    };
    format!("a Low Integrity test process exited with {code:#010x}{why}")
}

fn create_low_integrity_token() -> Result<OwnedHandle, String> {
    let mut current: HANDLE = ptr::null_mut();
    let access = TOKEN_DUPLICATE
        | TOKEN_QUERY
        | TOKEN_ASSIGN_PRIMARY
        | TOKEN_ADJUST_DEFAULT
        | TOKEN_ADJUST_PRIVILEGES;
    // SAFETY: `GetCurrentProcess` returns a pseudo-handle; `current` is an out-param.
    let ok = unsafe { OpenProcessToken(GetCurrentProcess(), access, &mut current) };
    if ok == 0 {
        return Err(format!(
            "OpenProcessToken failed: {}",
            io::Error::last_os_error()
        ));
    }
    let current = OwnedHandle::new(current).ok_or("OpenProcessToken returned null handle")?;

    let mut restricted: HANDLE = ptr::null_mut();
    // Try `DISABLE_MAX_PRIVILEGE | LUA_TOKEN` first (drops Administrators SID when elevated),
    // falling back to `DISABLE_MAX_PRIVILEGE` on standard tokens where `LUA_TOKEN` is rejected.
    // SAFETY: valid primary token handle and null optional arrays with zero counts.
    let mut ok = unsafe {
        CreateRestrictedToken(
            current.raw(),
            DISABLE_MAX_PRIVILEGE | LUA_TOKEN,
            0,
            ptr::null(),
            0,
            ptr::null(),
            0,
            ptr::null(),
            &mut restricted,
        )
    };
    if ok == 0 {
        // SAFETY: same as above, without `LUA_TOKEN`.
        ok = unsafe {
            CreateRestrictedToken(
                current.raw(),
                DISABLE_MAX_PRIVILEGE,
                0,
                ptr::null(),
                0,
                ptr::null(),
                0,
                ptr::null(),
                &mut restricted,
            )
        };
    }
    if ok == 0 {
        return Err(format!(
            "CreateRestrictedToken failed: {}",
            io::Error::last_os_error()
        ));
    }
    let restricted =
        OwnedHandle::new(restricted).ok_or("CreateRestrictedToken returned null handle")?;

    let sid_w = wide_str(LOW_INTEGRITY_SID);
    let mut sid: PSID = ptr::null_mut();
    // SAFETY: `sid_w` is NUL-terminated UTF-16; `sid` is freed via `LocalFree`.
    if unsafe { ConvertStringSidToSidW(sid_w.as_ptr(), &mut sid) } == 0 || sid.is_null() {
        return Err(format!(
            "ConvertStringSidToSidW failed: {}",
            io::Error::last_os_error()
        ));
    }
    let mut label = TOKEN_MANDATORY_LABEL {
        Label: SID_AND_ATTRIBUTES {
            Sid: sid,
            Attributes: SE_GROUP_INTEGRITY,
        },
    };
    let len = u32::try_from(size_of::<TOKEN_MANDATORY_LABEL>()).unwrap_or(u32::MAX);
    // SAFETY: `label` references the live `sid` for the duration of `SetTokenInformation`.
    let set_ok = unsafe {
        SetTokenInformation(
            restricted.raw(),
            TokenIntegrityLevel,
            (&raw mut label).cast(),
            len,
        )
    };
    // SAFETY: `sid` was allocated by `ConvertStringSidToSidW`.
    unsafe {
        LocalFree(sid.cast());
    }
    if set_ok == 0 {
        return Err(format!(
            "SetTokenInformation(Low Integrity) failed: {}",
            io::Error::last_os_error()
        ));
    }
    Ok(restricted)
}

fn create_sandbox_job() -> Result<OwnedHandle, String> {
    // SAFETY: anonymous job object.
    let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
    let job = OwnedHandle::new(raw)
        .ok_or_else(|| format!("CreateJobObjectW failed: {}", io::Error::last_os_error()))?;

    // SAFETY: plain-data struct; zeroed sets unused limit fields cleanly.
    let mut ext: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
    ext.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    let ext_len = u32::try_from(size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>()).unwrap_or(0);
    // SAFETY: `ext` is valid for `ext_len` bytes.
    if unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectExtendedLimitInformation,
            (&raw const ext).cast(),
            ext_len,
        )
    } == 0
    {
        return Err(format!(
            "SetInformationJobObject(extended) failed: {}",
            io::Error::last_os_error()
        ));
    }

    let ui = JOBOBJECT_BASIC_UI_RESTRICTIONS {
        UIRestrictionsClass: JOB_OBJECT_UILIMIT_DESKTOP
            | JOB_OBJECT_UILIMIT_DISPLAYSETTINGS
            | JOB_OBJECT_UILIMIT_EXITWINDOWS
            | JOB_OBJECT_UILIMIT_GLOBALATOMS
            | JOB_OBJECT_UILIMIT_HANDLES
            | JOB_OBJECT_UILIMIT_READCLIPBOARD
            | JOB_OBJECT_UILIMIT_SYSTEMPARAMETERS
            | JOB_OBJECT_UILIMIT_WRITECLIPBOARD,
    };
    let ui_len = u32::try_from(size_of::<JOBOBJECT_BASIC_UI_RESTRICTIONS>()).unwrap_or(0);
    // SAFETY: `ui` is valid for `ui_len` bytes.
    let _ = unsafe {
        SetInformationJobObject(
            job.raw(),
            JobObjectBasicUIRestrictions,
            (&raw const ui).cast(),
            ui_len,
        )
    };
    Ok(job)
}

/// Apply an inheritable Low-Integrity mandatory label to each existing writable path so
/// the Low-Integrity child can write inside the workspace and temp directories while
/// remaining blocked from Medium-Integrity directories elsewhere. A path that can't be
/// labelled is an error: the command would otherwise fail in confusing ways.
fn label_writable_paths(paths: &[PathBuf]) -> io::Result<()> {
    /// Paths already labelled by this process. Labelling a folder walks everything in it.
    static LABELLED: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());
    let mut labelled = LABELLED.lock().unwrap_or_else(PoisonError::into_inner);
    let todo: Vec<&PathBuf> = paths
        .iter()
        .filter(|p| p.exists() && !labelled.iter().any(|l| same_path(l, p)))
        .collect();
    if todo.is_empty() {
        return Ok(());
    }

    let sddl = wide_str(LOW_INTEGRITY_SACL_SDDL);
    let mut sd: PSECURITY_DESCRIPTOR = ptr::null_mut();
    // SAFETY: NUL-terminated SDDL; `sd` freed below via `LocalFree`.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut sd,
            ptr::null_mut(),
        )
    };
    if ok == 0 || sd.is_null() {
        return Err(io::Error::other(format!(
            "could not build the Low Integrity label: {}",
            io::Error::last_os_error()
        )));
    }
    let result = label_with(sd, &todo, &mut labelled);
    // SAFETY: `sd` was allocated by `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
    unsafe {
        LocalFree(sd.cast());
    }
    result
}

fn label_with(
    sd: PSECURITY_DESCRIPTOR,
    paths: &[&PathBuf],
    labelled: &mut Vec<PathBuf>,
) -> io::Result<()> {
    let mut present = FALSE;
    let mut defaulted = FALSE;
    let mut sacl = ptr::null_mut();
    // SAFETY: `sd` is valid; `sacl` points inside `sd`.
    let got = unsafe { GetSecurityDescriptorSacl(sd, &mut present, &mut sacl, &mut defaulted) };
    if got == 0 || present == 0 || sacl.is_null() {
        return Err(io::Error::other(
            "the Low Integrity label has no SACL to apply",
        ));
    }
    for path in paths {
        let mut path_w = wide(path.as_os_str());
        // SAFETY: `path_w` is NUL-terminated UTF-16; `sacl` is valid for the call.
        let err = unsafe {
            SetNamedSecurityInfoW(
                path_w.as_mut_ptr(),
                SE_FILE_OBJECT,
                LABEL_SECURITY_INFORMATION,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
                sacl,
            )
        };
        if err != 0 {
            let cause = io::Error::from_raw_os_error(i32::try_from(err).unwrap_or(i32::MAX));
            return Err(io::Error::other(format!(
                "could not let the sandbox write to {}: {cause}",
                path.display()
            )));
        }
        labelled.push((*path).clone());
    }
    Ok(())
}

/// Whether two paths name the same place, ignoring case and trailing separators.
fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| -> Vec<OsString> {
        p.components()
            .map(|c| c.as_os_str().to_ascii_lowercase())
            .collect()
    };
    norm(a) == norm(b)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StdioCfg {
    Inherit,
    Null,
    Piped,
}

impl From<Stdio> for StdioCfg {
    fn from(_: Stdio) -> Self {
        // Callers use our explicit helper methods (`stdin_null`, `stdin_piped`, etc.).
        Self::Inherit
    }
}

#[derive(Debug)]
pub(crate) struct WindowsCommand {
    policy: Policy,
    program: OsString,
    args: Vec<OsString>,
    cwd: Option<PathBuf>,
    env_clear: bool,
    envs: BTreeMap<OsString, OsString>,
    stdin: StdioCfg,
    stdout: StdioCfg,
    stderr: StdioCfg,
    kill_on_drop: bool,
    creation_flags: u32,
}

impl WindowsCommand {
    pub(crate) fn new<I, S>(policy: &Policy, program: &OsStr, args: I) -> Result<Self, String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        available()?;
        Ok(Self::unchecked(policy, program, args))
    }

    /// A command that skips [`available`] (which itself runs one of these).
    fn unchecked<I, S>(policy: &Policy, program: &OsStr, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Self {
            policy: policy.clone(),
            program: program.to_os_string(),
            args: args
                .into_iter()
                .map(|a| a.as_ref().to_os_string())
                .collect(),
            cwd: None,
            env_clear: false,
            envs: BTreeMap::new(),
            stdin: StdioCfg::Inherit,
            stdout: StdioCfg::Inherit,
            stderr: StdioCfg::Inherit,
            kill_on_drop: false,
            creation_flags: CREATE_NO_WINDOW,
        }
    }

    pub(crate) fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.cwd = Some(dir.as_ref().to_path_buf());
        self
    }

    pub(crate) fn env_clear(&mut self) -> &mut Self {
        self.env_clear = true;
        self.envs.clear();
        self
    }

    pub(crate) fn env(&mut self, k: impl AsRef<OsStr>, v: impl AsRef<OsStr>) -> &mut Self {
        self.envs
            .insert(k.as_ref().to_os_string(), v.as_ref().to_os_string());
        self
    }

    pub(crate) fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        for (k, v) in vars {
            self.env(k, v);
        }
        self
    }

    pub(crate) fn stdin(&mut self, cfg: StdioCfg) -> &mut Self {
        self.stdin = cfg;
        self
    }

    pub(crate) fn stdout(&mut self, cfg: StdioCfg) -> &mut Self {
        self.stdout = cfg;
        self
    }

    pub(crate) fn stderr(&mut self, cfg: StdioCfg) -> &mut Self {
        self.stderr = cfg;
        self
    }

    pub(crate) fn kill_on_drop(&mut self, yes: bool) -> &mut Self {
        self.kill_on_drop = yes;
        self
    }

    pub(crate) fn creation_flags(&mut self, flags: u32) -> &mut Self {
        self.creation_flags = flags;
        self
    }

    pub(crate) fn spawn(&self) -> io::Result<WindowsChild> {
        let launched = self.launch()?;
        let to_file = |h: Option<OwnedHandle>| -> Option<tokio::fs::File> {
            let owned = h?;
            let raw = owned.raw();
            std::mem::forget(owned);
            // SAFETY: ownership transferred exclusively to `std::fs::File`.
            let std_file = unsafe { std::fs::File::from_raw_handle(raw.cast()) };
            Some(tokio::fs::File::from_std(std_file))
        };
        Ok(WindowsChild {
            process: launched.process,
            job: launched.job,
            pid: launched.pid,
            stdin: to_file(launched.stdin),
            stdout: to_file(launched.stdout),
            stderr: to_file(launched.stderr),
            kill_on_drop: self.kill_on_drop,
        })
    }

    /// The writable paths with the temp folder swapped for its `quack-sandbox` folder,
    /// and that folder (when there is one) for the child's `TEMP` and `TMP`.
    fn writable_paths(&self) -> io::Result<(Vec<PathBuf>, Option<PathBuf>)> {
        let temp = std::env::temp_dir();
        let mut child_temp = None;
        let mut writable = Vec::with_capacity(self.policy.writable.len());
        for path in &self.policy.writable {
            if same_path(path, &temp) {
                let dir = temp.join(SANDBOX_TEMP);
                std::fs::create_dir_all(&dir)?;
                writable.push(dir.clone());
                child_temp = Some(dir);
            } else {
                writable.push(path.clone());
            }
        }
        Ok((writable, child_temp))
    }

    /// Starts the process suspended under the Low Integrity token, puts it in the job,
    /// then lets it run.
    fn launch(&self) -> io::Result<Launched> {
        let token = create_low_integrity_token().map_err(io::Error::other)?;
        let job = create_sandbox_job().map_err(io::Error::other)?;
        let (writable, child_temp) = self.writable_paths()?;
        label_writable_paths(&writable)?;

        let mut envs = self.envs.clone();
        if let Some(dir) = child_temp {
            for key in ["TEMP", "TMP"] {
                if !envs.keys().any(|k| k.eq_ignore_ascii_case(key)) {
                    envs.insert(key.into(), dir.clone().into_os_string());
                }
            }
        }

        let (child_in, parent_in) = make_stdio(self.stdin, true)?;
        let (child_out, parent_out) = make_stdio(self.stdout, false)?;
        let (child_err, parent_err) = make_stdio(self.stderr, false)?;

        let mut cmdline = build_command_line(&self.program, &self.args);
        let cwd_w = self.cwd.as_ref().map(|p| wide(p.as_os_str()));
        let env_block = build_env_block(self.env_clear, &envs);

        // SAFETY: `STARTUPINFOW` is plain data; `cb` holds its exact byte size.
        let mut si: STARTUPINFOW = unsafe { std::mem::zeroed() };
        si.cb = u32::try_from(size_of::<STARTUPINFOW>()).unwrap_or(0);
        si.dwFlags = STARTF_USESTDHANDLES;
        si.hStdInput = child_in
            .as_ref()
            .map_or(INVALID_HANDLE_VALUE, OwnedHandle::raw);
        si.hStdOutput = child_out
            .as_ref()
            .map_or(INVALID_HANDLE_VALUE, OwnedHandle::raw);
        si.hStdError = child_err
            .as_ref()
            .map_or(INVALID_HANDLE_VALUE, OwnedHandle::raw);

        // SAFETY: out-parameter filled by `CreateProcessAsUserW`.
        let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        let flags = self.creation_flags | CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT;
        let env_ptr = env_block
            .as_ref()
            .map_or(ptr::null(), |b| b.as_ptr().cast());
        let cwd_ptr = cwd_w.as_ref().map_or(ptr::null(), Vec::as_ptr);

        // SAFETY: all pointers live across the Win32 call; `cmdline` is mutable UTF-16 NUL-terminated.
        let ok = unsafe {
            CreateProcessAsUserW(
                token.raw(),
                ptr::null(),
                cmdline.as_mut_ptr(),
                ptr::null(),
                ptr::null(),
                TRUE,
                flags,
                env_ptr,
                cwd_ptr,
                &si,
                &mut pi,
            )
        };
        // Drop child ends in parent immediately so pipe EOF propagates cleanly.
        drop(child_in);
        drop(child_out);
        drop(child_err);

        if ok == 0 {
            let code = unsafe { GetLastError() };
            return Err(io::Error::from_raw_os_error(code as i32));
        }

        let process = OwnedHandle::new(pi.hProcess)
            .ok_or_else(|| io::Error::other("CreateProcessAsUserW returned null process handle"))?;
        let thread = OwnedHandle::new(pi.hThread)
            .ok_or_else(|| io::Error::other("CreateProcessAsUserW returned null thread handle"))?;

        // Bind to the sandbox Job Object while suspended so even the first instruction runs
        // inside it. A process outside the job would escape its limits, so failing is fatal.
        // SAFETY: both handles are valid and owned here.
        if unsafe { AssignProcessToJobObject(job.raw(), process.raw()) } == 0 {
            let err = io::Error::last_os_error();
            // SAFETY: the suspended process is owned here and never ran.
            unsafe {
                TerminateProcess(process.raw(), 1);
            }
            return Err(io::Error::other(format!(
                "could not put the process in the sandbox job: {err}"
            )));
        }
        // SAFETY: valid primary thread handle.
        unsafe {
            ResumeThread(thread.raw());
        }
        drop(thread);

        Ok(Launched {
            process,
            job,
            pid: pi.dwProcessId,
            stdin: parent_in,
            stdout: parent_out,
            stderr: parent_err,
        })
    }
}

/// A started sandboxed process and the parent ends of its pipes.
struct Launched {
    process: OwnedHandle,
    job: OwnedHandle,
    pid: u32,
    stdin: Option<OwnedHandle>,
    stdout: Option<OwnedHandle>,
    stderr: Option<OwnedHandle>,
}

fn make_stdio(
    cfg: StdioCfg,
    is_stdin: bool,
) -> io::Result<(Option<OwnedHandle>, Option<OwnedHandle>)> {
    let sa = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
        lpSecurityDescriptor: ptr::null_mut(),
        bInheritHandle: TRUE,
    };
    match cfg {
        StdioCfg::Piped => {
            let mut read_h: HANDLE = ptr::null_mut();
            let mut write_h: HANDLE = ptr::null_mut();
            // SAFETY: creates an anonymous pipe pair with inheritable handles.
            if unsafe { CreatePipe(&mut read_h, &mut write_h, &sa, 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
            let read_h = OwnedHandle::new(read_h).ok_or_else(io::Error::last_os_error)?;
            let write_h = OwnedHandle::new(write_h).ok_or_else(io::Error::last_os_error)?;
            let (child, parent) = if is_stdin {
                (read_h, write_h)
            } else {
                (write_h, read_h)
            };
            // Ensure the parent end is NOT inherited by the child so EOF closes properly.
            // SAFETY: `parent` is a valid pipe handle.
            if unsafe { SetHandleInformation(parent.raw(), HANDLE_FLAG_INHERIT, 0) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok((Some(child), Some(parent)))
        }
        StdioCfg::Null | StdioCfg::Inherit => {
            let nul = wide_str("NUL");
            let access = if is_stdin {
                FILE_GENERIC_READ
            } else {
                FILE_GENERIC_WRITE
            };
            // SAFETY: opens the Windows NUL device with an inheritable handle.
            let h = unsafe {
                CreateFileW(
                    nul.as_ptr(),
                    access,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    &sa,
                    OPEN_EXISTING,
                    0,
                    ptr::null_mut(),
                )
            };
            let h = OwnedHandle::new(h).ok_or_else(io::Error::last_os_error)?;
            Ok((Some(h), None))
        }
    }
}

/// Build a UTF-16 Windows command line following MSVCRT quoting rules, except for the
/// script of a `cmd` call, which goes after `/S /C` unescaped (see `crate::cmd_script`).
fn build_command_line(program: &OsStr, args: &[OsString]) -> Vec<u16> {
    let mut out = Vec::new();
    append_quoted(&mut out, program);
    let (switches, script) = match crate::cmd_script(program, args) {
        Some((switches, script)) => (switches, Some(script)),
        None => (args.to_vec(), None),
    };
    for arg in &switches {
        out.push(b' ' as u16);
        append_quoted(&mut out, arg);
    }
    if let Some(script) = script {
        out.push(b' ' as u16);
        out.push(b'"' as u16);
        out.extend(script.encode_wide());
        out.push(b'"' as u16);
    }
    out.push(0);
    out
}

fn append_quoted(out: &mut Vec<u16>, arg: &OsStr) {
    let units: Vec<u16> = arg.encode_wide().collect();
    let needs_quotes = units.is_empty()
        || units
            .iter()
            .any(|&c| c == b' ' as u16 || c == b'\t' as u16 || c == b'"' as u16);
    if !needs_quotes {
        out.extend_from_slice(&units);
        return;
    }
    out.push(b'"' as u16);
    let mut backslashes = 0usize;
    for &c in &units {
        if c == b'\\' as u16 {
            backslashes += 1;
        } else if c == b'"' as u16 {
            for _ in 0..(backslashes * 2 + 1) {
                out.push(b'\\' as u16);
            }
            out.push(b'"' as u16);
            backslashes = 0;
        } else {
            for _ in 0..backslashes {
                out.push(b'\\' as u16);
            }
            backslashes = 0;
            out.push(c);
        }
    }
    for _ in 0..(backslashes * 2) {
        out.push(b'\\' as u16);
    }
    out.push(b'"' as u16);
}

fn build_env_block(clear: bool, overrides: &BTreeMap<OsString, OsString>) -> Option<Vec<u16>> {
    if !clear && overrides.is_empty() {
        return None;
    }
    let mut merged: BTreeMap<String, (OsString, OsString)> = BTreeMap::new();
    if !clear {
        for (k, v) in std::env::vars_os() {
            merged.insert(k.to_string_lossy().to_ascii_uppercase(), (k, v));
        }
    }
    for (k, v) in overrides {
        merged.insert(
            k.to_string_lossy().to_ascii_uppercase(),
            (k.clone(), v.clone()),
        );
    }
    let mut block = Vec::new();
    for (_, (k, v)) in merged {
        block.extend(k.encode_wide());
        block.push(b'=' as u16);
        block.extend(v.encode_wide());
        block.push(0);
    }
    block.push(0);
    Some(block)
}

pub(crate) struct WindowsChild {
    process: OwnedHandle,
    job: OwnedHandle,
    pid: u32,
    pub(crate) stdin: Option<tokio::fs::File>,
    pub(crate) stdout: Option<tokio::fs::File>,
    pub(crate) stderr: Option<tokio::fs::File>,
    kill_on_drop: bool,
}

impl std::fmt::Debug for WindowsChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowsChild")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl WindowsChild {
    pub(crate) fn id(&self) -> Option<u32> {
        Some(self.pid)
    }

    pub(crate) fn raw_handle(&self) -> Option<RawHandle> {
        Some(self.process.raw().cast())
    }

    pub(crate) async fn wait(&mut self) -> io::Result<ExitStatus> {
        // Wrap raw handle as usize so the blocking closure is Send while `self.process` stays alive.
        let handle_val = self.process.raw() as usize;
        let code = tokio::task::spawn_blocking(move || -> io::Result<u32> {
            let h = handle_val as HANDLE;
            // SAFETY: `self.process` remains valid for the entire `.await`.
            let wait = unsafe { WaitForSingleObject(h, INFINITE) };
            if wait != WAIT_OBJECT_0 {
                return Err(io::Error::last_os_error());
            }
            let mut exit_code: u32 = 1;
            // SAFETY: `h` has exited; `exit_code` receives its status.
            if unsafe { GetExitCodeProcess(h, &mut exit_code) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(exit_code)
        })
        .await
        .map_err(io::Error::other)??;
        Ok(ExitStatus::from_raw(code))
    }

    pub(crate) async fn kill(&mut self) -> io::Result<()> {
        // SAFETY: `self.job` and `self.process` are valid handles owned by `self`.
        unsafe {
            TerminateJobObject(self.job.raw(), 1);
            TerminateProcess(self.process.raw(), 1);
        }
        Ok(())
    }
}

impl Drop for WindowsChild {
    fn drop(&mut self) {
        if self.kill_on_drop {
            // SAFETY: `self.job` is owned by `self`.
            unsafe {
                TerminateJobObject(self.job.raw(), 1);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::windows::ffi::OsStringExt;

    use super::*;

    #[test]
    fn quotes_windows_command_line_arguments() {
        let line = build_command_line(
            OsStr::new("C:\\Program Files\\pwsh.exe"),
            &[
                OsString::from("-Command"),
                OsString::from("echo \"hello world\" \\path\\"),
            ],
        );
        let text = OsString::from_wide(&line[..line.len() - 1])
            .to_string_lossy()
            .into_owned();
        assert_eq!(
            text,
            "\"C:\\Program Files\\pwsh.exe\" -Command \"echo \\\"hello world\\\" \\path\\\\\""
        );
    }

    #[test]
    fn cmd_scripts_keep_their_quotes() {
        let line = build_command_line(
            OsStr::new("cmd.exe"),
            &[
                OsString::from("/D"),
                OsString::from("/C"),
                OsString::from(r#"echo ok > "C:\a b\inside.txt""#),
            ],
        );
        let text = OsString::from_wide(&line[..line.len() - 1])
            .to_string_lossy()
            .into_owned();
        assert_eq!(text, r#"cmd.exe /D /S /C "echo ok > "C:\a b\inside.txt"""#);
    }

    /// Runs a `cmd` script in the sandbox: whether it succeeded, and a report for asserts.
    async fn run_cmd(policy: &Policy, cwd: &Path, script: &str) -> (bool, String) {
        use tokio::io::AsyncReadExt;

        let mut cmd =
            WindowsCommand::new(policy, OsStr::new("cmd.exe"), ["/D", "/C", script]).expect("cmd");
        cmd.current_dir(cwd)
            .stdin(StdioCfg::Null)
            .stdout(StdioCfg::Piped)
            .stderr(StdioCfg::Piped);
        let mut child = cmd.spawn().expect("spawn");
        let mut stdout = child.stdout.take().expect("stdout");
        let mut stderr = child.stderr.take().expect("stderr");
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let (o, e) = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err));
        o.expect("read stdout");
        e.expect("read stderr");
        let status = child.wait().await.expect("wait");
        let code = status.code().map_or(0, i32::cast_unsigned);
        let report = format!(
            "script: {script}\nexit: {code:#010x}\nstdout: {}\nstderr: {}",
            String::from_utf8_lossy(&out).trim(),
            String::from_utf8_lossy(&err).trim()
        );
        (status.success(), report)
    }

    /// Skips where the sandbox can't run, except on CI, where that is the bug to report.
    #[allow(clippy::print_stderr)]
    fn sandbox_or_skip() -> bool {
        match available() {
            Ok(_) => true,
            Err(why) => {
                assert!(
                    std::env::var_os("CI").is_none(),
                    "the Windows sandbox must work on CI: {why}"
                );
                eprintln!("skipped, the sandbox is not available: {why}");
                false
            }
        }
    }

    #[tokio::test]
    async fn restricted_token_blocks_writes_outside_writable_dir() {
        if !sandbox_or_skip() {
            return;
        }
        // Not under the temp folder, so the result doesn't depend on how it is labelled.
        let exe = std::env::current_exe().expect("test binary");
        let base = tempfile::tempdir_in(exe.parent().expect("deps folder")).expect("tempdir");
        let ws = base.path().join("ws");
        let outside = base.path().join("outside");
        std::fs::create_dir_all(&ws).expect("ws");
        std::fs::create_dir_all(&outside).expect("outside");
        let policy = Policy {
            writable: vec![ws.clone()],
            network: false,
        };

        let inside = ws.join("inside.txt");
        let (ok, report) =
            run_cmd(&policy, &ws, &format!("echo ok > \"{}\"", inside.display())).await;
        assert!(ok, "writing in the workspace failed:\n{report}");
        assert!(inside.exists(), "nothing was written:\n{report}");

        let blocked = outside.join("blocked.txt");
        let (_, report) = run_cmd(
            &policy,
            &ws,
            &format!("echo no > \"{}\"", blocked.display()),
        )
        .await;
        assert!(
            !blocked.exists(),
            "a write outside the workspace got through:\n{report}"
        );
    }

    #[tokio::test]
    async fn the_temp_folder_is_a_sandbox_folder_inside_it() {
        if !sandbox_or_skip() {
            return;
        }
        let exe = std::env::current_exe().expect("test binary");
        let ws = tempfile::tempdir_in(exe.parent().expect("deps folder")).expect("tempdir");
        let policy = Policy {
            writable: vec![ws.path().to_path_buf(), std::env::temp_dir()],
            network: false,
        };
        let name = format!("quack-temp-{}.txt", std::process::id());
        let script = format!("echo ok > \"%TEMP%\\{name}\" && echo %TEMP%");
        let (ok, report) = run_cmd(&policy, ws.path(), &script).await;
        let written = std::env::temp_dir().join(SANDBOX_TEMP).join(&name);
        let found = written.exists();
        let _ = std::fs::remove_file(&written);
        assert!(ok, "writing to %TEMP% failed:\n{report}");
        assert!(
            report.contains(SANDBOX_TEMP),
            "%TEMP% was not redirected:\n{report}"
        );
        assert!(
            found,
            "the file is not in the sandbox temp folder:\n{report}"
        );
    }
}
