//! Permission modes, allow/ask/deny rules and the compiled-in deny floor.
//!
//! Rules are `kind:pattern` strings: `shell:git status*` (`*` glob over the whole command),
//! `edit:src/**` and `read:docs/**` (path globs relative to the workspace root).
//! Order: work-mode restriction → deny floor → deny → ask → session/allow → approval default.
//! Yolo bypasses asks after the work-mode restriction, floor and denies.
//!
//! A shell command is split into its simple commands first (`git status && rm -rf x` is
//! `git status` and `rm -rf x`, and `$(...)` counts too). Deny and ask rules, and the deny
//! floor, apply when the whole line or any one command matches. An allow rule applies only
//! when every command is allowed (by a rule, or because it only reads, with at least one
//! rule match) and nothing is redirected to a file. A line the splitter can't read
//! (backticks, subshells, heredocs, `if`/`for`) gets no allow rule at all.

use std::path::{Component, Path, PathBuf};

use quack_config::glob_match;
use quack_config::settings::{PermissionMode, PermissionSettings, WorkMode};

use crate::paths;
use crate::registry::Subject;
use crate::shell_split::{self, Split};

/// Commands refused in every mode, even `full-auto`. Matched after collapsing whitespace.
const DENY_FLOOR: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "rm -rf ~/*",
    "rm -rf $HOME*",
    "sudo rm -rf /*",
    ":(){ :|:& };:",
    "mkfs*",
    "* > /dev/sda*",
    "dd * of=/dev/sd*",
    "dd * of=/dev/nvme*",
    "format c:*",
    "Remove-Item -Recurse -Force C:\\*",
];

/// Commands that only look, allowed while planning. Never matched with control operators.
const READ_ONLY_COMMANDS: &[&str] = &[
    "ls",
    "dir",
    "cat",
    "type",
    "head",
    "tail",
    "wc",
    "grep",
    "rg",
    "find",
    "tree",
    "file",
    "stat",
    "du",
    "pwd",
    "which",
    "where",
    // Tests, which the bash parser now finds inside `if` and `while` conditions.
    "[",
    "[[",
    "test",
    "true",
    "false",
    "Get-ChildItem",
    "Get-Content",
    "Select-String",
    // PowerShell's own, matched ignoring case. Not `ForEach-Object`, which can call a
    // method on every input (`gci | % Delete`), nor `sort`, which writes with `-o`.
    "gci",
    "gc",
    "gi",
    "gl",
    "gcm",
    "sls",
    "Get-Item",
    "Get-ItemProperty",
    "Get-Location",
    "Get-Command",
    "Get-FileHash",
    "Get-Date",
    "Test-Path",
    "Resolve-Path",
    "Split-Path",
    "Join-Path",
    "Select-Object",
    "Sort-Object",
    "Where-Object",
    "Measure-Object",
    "Format-Table",
    "Format-List",
    "Out-String",
];
const READ_ONLY_GIT: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "blame",
    "grep",
    "ls-files",
    "rev-parse",
];

/// A simple command that only reads: `git status`, `rg foo src`, but not `find -delete`.
fn read_only_command(cmd: &str) -> bool {
    let cmd = normalize(cmd);
    let mut words = cmd.split(' ');
    match words.next() {
        Some("git") => {
            let Some(subcommand) = words.next().filter(|w| READ_ONLY_GIT.contains(w)) else {
                return false;
            };
            let args: Vec<_> = words.collect();
            if args.iter().any(|a| {
                a.starts_with("--output")
                    || a.starts_with("--ext-diff")
                    || a.starts_with("--textconv")
                    || a.starts_with("--open-files-in-pager")
                    || (subcommand == "grep" && a.starts_with("-O"))
            }) {
                return false;
            }
            subcommand != "branch"
                || args.iter().all(|a| {
                    matches!(
                        *a,
                        "-a" | "--all"
                            | "-r"
                            | "--remotes"
                            | "-v"
                            | "-vv"
                            | "--verbose"
                            | "--list"
                            | "--show-current"
                            | "--no-color"
                            | "--color=never"
                    )
                })
        }
        Some("find") => !words.any(|w| {
            matches!(
                w,
                "-exec"
                    | "-execdir"
                    | "-delete"
                    | "-ok"
                    | "-okdir"
                    | "-fprint"
                    | "-fprint0"
                    | "-fprintf"
                    | "-fls"
            )
        }),
        Some("rg") => !words.any(|w| {
            w == "--pre"
                || w.starts_with("--pre=")
                || w == "--hostname-bin"
                || w.starts_with("--hostname-bin=")
        }),
        Some("file") => !words.any(|w| w == "--compile" || w.starts_with("-C")),
        Some(first) => READ_ONLY_COMMANDS
            .iter()
            .any(|c| c.eq_ignore_ascii_case(first)),
        None => false,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny(String),
}

#[derive(Debug, Clone)]
enum Rule {
    Shell(String),
    Edit(globset::GlobMatcher),
    Read(globset::GlobMatcher),
    /// `mcp:<server>/<tool>` globs, e.g. `mcp:github/get_*`.
    Mcp(String),
}

impl Rule {
    fn parse(s: &str) -> Result<Self, String> {
        let (kind, pattern) = s.split_once(':').ok_or_else(|| {
            format!("rule `{s}` needs a kind prefix: shell:, edit:, read: or mcp:")
        })?;
        let glob = |p: &str| {
            globset::GlobBuilder::new(p.trim())
                .literal_separator(true)
                .case_insensitive(cfg!(any(windows, target_os = "macos")))
                .build()
                .map(|g| g.compile_matcher())
                .map_err(|e| format!("rule `{s}`: {e}"))
        };
        match kind.trim() {
            "shell" | "bash" => Ok(Self::Shell(normalize(pattern))),
            "edit" | "write" => Ok(Self::Edit(glob(pattern)?)),
            "read" => Ok(Self::Read(glob(pattern)?)),
            "mcp" => Ok(Self::Mcp(pattern.trim().to_owned())),
            other => Err(format!("rule `{s}`: unknown kind `{other}`")),
        }
    }

    fn matches(&self, subject: &Subject, root: &Path) -> bool {
        let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_path_buf();
        match (self, subject) {
            (Self::Shell(pat), Subject::Shell(cmd)) => glob_match(pat, &normalize(cmd)),
            (Self::Edit(g), Subject::Edit(p)) => g.is_match(rel(p)),
            (Self::Read(g), Subject::Read(p)) => g.is_match(rel(p)),
            (Self::Mcp(pat), Subject::Mcp { server, tool, .. }) => {
                glob_match(pat, &format!("{server}/{tool}"))
            }
            _ => false,
        }
    }
}

fn normalize(cmd: &str) -> String {
    cmd.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// PowerShell names for the same command, for deny and ask rules: `shell:rm *` also
/// stops `Remove-Item x` and `del x`, and `shell:curl *` stops `iwr`.
const PS_ALIASES: &[&[&str]] = &[
    &["remove-item", "rm", "del", "erase", "rd", "ri", "rmdir"],
    &["get-childitem", "ls", "dir", "gci"],
    &["get-content", "cat", "type", "gc"],
    &["set-content", "sc"],
    &["add-content", "ac"],
    &["copy-item", "cp", "copy", "cpi"],
    &["move-item", "mv", "move", "mi"],
    &["rename-item", "ren", "rni"],
    &["new-item", "ni"],
    &["invoke-webrequest", "iwr", "curl", "wget"],
    &["invoke-restmethod", "irm"],
    &["invoke-expression", "iex"],
    &["invoke-command", "icm"],
    &["invoke-item", "ii"],
    &["start-process", "start", "saps"],
    &["stop-process", "kill", "spps"],
    &["set-location", "cd", "chdir", "sl"],
    &["write-output", "echo", "write"],
];

/// A lowercased PowerShell command and the same command under each of its aliases.
fn ps_variants(cmd: &str) -> Vec<String> {
    let lower = cmd.to_lowercase();
    let (first, rest) = lower.split_once(' ').unwrap_or((lower.as_str(), ""));
    let program = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let program = program.strip_suffix(".exe").unwrap_or(program);
    let mut out = vec![lower.clone()];
    if let Some(group) = PS_ALIASES.iter().find(|g| g.contains(&program)) {
        for name in group.iter().filter(|n| **n != first) {
            out.push(if rest.is_empty() {
                (*name).to_owned()
            } else {
                format!("{name} {rest}")
            });
        }
    }
    out
}

/// A shell command line as the rules see it.
struct ShellLine {
    whole: String,
    split: Option<Split>,
    /// Run by PowerShell: names match ignoring case, and deny and ask rules see aliases.
    powershell: bool,
}

impl ShellLine {
    fn new(cmd: &str, powershell: bool) -> Self {
        Self {
            whole: normalize(cmd),
            split: shell_split::split_for(cmd, powershell),
            powershell,
        }
    }

    /// The whole line and each simple command in it.
    fn pieces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.whole.as_str()).chain(
            self.split
                .iter()
                .flat_map(|s| s.parts.iter().map(String::as_str)),
        )
    }

    /// A shell pattern matches one command; for PowerShell, ignoring case and, with
    /// `aliases`, under any of the command's names.
    fn matches(&self, pattern: &str, piece: &str, aliases: bool) -> bool {
        if !self.powershell {
            return glob_match(pattern, piece);
        }
        let pattern = pattern.to_lowercase();
        if aliases {
            ps_variants(piece).iter().any(|v| glob_match(&pattern, v))
        } else {
            glob_match(&pattern, &piece.to_lowercase())
        }
    }

    fn rule_matches(&self, rule: &Rule, piece: &str, aliases: bool) -> bool {
        match rule {
            Rule::Shell(pattern) => self.matches(pattern, piece, aliases),
            _ => false,
        }
    }

    /// The whole line or one of its commands is on the built-in deny list.
    fn on_floor(&self) -> bool {
        self.pieces()
            .any(|p| DENY_FLOOR.iter().any(|f| self.matches(f, p, true)))
    }

    /// Some rule matches the whole line or one of its commands (deny and ask rules).
    fn hit(&self, rules: &[Rule]) -> bool {
        self.pieces()
            .any(|p| rules.iter().any(|r| self.rule_matches(r, p, true)))
    }

    /// Every command is allowed by a rule or only reads, at least one by a rule, and
    /// nothing is written through a redirection.
    fn allowed(&self, rules: &[Rule]) -> bool {
        let Some(split) = self.split.as_ref().filter(|s| !s.writes) else {
            return false;
        };
        let mut matched = false;
        for p in &split.parts {
            if rules.iter().any(|r| self.rule_matches(r, p, false)) {
                matched = true;
            } else if !read_only_command(p) {
                return false;
            }
        }
        matched
    }

    /// Every command only reads and nothing is redirected to a file.
    fn read_only(&self) -> bool {
        self.split
            .as_ref()
            .is_some_and(|s| !s.writes && s.parts.iter().all(|p| read_only_command(p)))
    }
}

#[derive(Debug, Clone)]
pub struct Permissions {
    pub mode: PermissionMode,
    root: PathBuf,
    allow: Vec<Rule>,
    ask: Vec<Rule>,
    deny: Vec<Rule>,
    session: Vec<Subject>,
    /// `[memory] writes = "ask"`.
    pub memory_writes_ask: bool,
    /// Plan mode: look but don't change anything (docs/plan/04-agent-loop-and-tools.md#plan-mode).
    pub work_mode: WorkMode,
    /// `/sandbox off`: shell commands run without the OS sandbox this session.
    pub sandbox_off: bool,
    /// Commands run in PowerShell (`pwsh`, `powershell`), so they are read as PowerShell.
    pub powershell: bool,
}

impl Permissions {
    /// Build from settings. Invalid rules are returned as warnings and skipped.
    pub fn new(settings: &PermissionSettings, root: &Path) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut parse = |list: &[String]| {
            list.iter()
                .filter_map(|r| Rule::parse(r).map_err(|e| warnings.push(e)).ok())
                .collect::<Vec<_>>()
        };
        let perms = Self {
            mode: settings.mode,
            root: root.to_path_buf(),
            allow: parse(&settings.allow),
            ask: parse(&settings.ask),
            deny: parse(&settings.deny),
            session: Vec::new(),
            memory_writes_ask: false,
            work_mode: settings.work_mode,
            sandbox_off: false,
            powershell: false,
        };
        (perms, warnings)
    }

    /// Remember an "allow for this session" answer.
    pub fn allow_session(&mut self, subject: Subject) {
        match subject {
            Subject::Edits(paths) => self.session.extend(paths.into_iter().map(Subject::Edit)),
            other => self.session.push(other),
        }
    }

    /// The allow rules that "always allow" saves for `subject`: exact, so they allow this
    /// call and nothing broader. Empty when no rule can allow it (a line the splitter
    /// can't read, a redirection, a command outside the sandbox, a name with a wildcard).
    pub fn rules_for(&self, subject: &Subject) -> Vec<String> {
        let path = |kind: &str, p: &Path| {
            // Relative to the workspace when inside it, always with `/`.
            let rel = p.strip_prefix(&self.root).unwrap_or(p);
            let mut text = String::new();
            for c in rel.components() {
                match c {
                    Component::Prefix(pre) => text.push_str(&pre.as_os_str().to_string_lossy()),
                    Component::RootDir => text.push('/'),
                    other => {
                        if !text.is_empty() && !text.ends_with('/') {
                            text.push('/');
                        }
                        text.push_str(&other.as_os_str().to_string_lossy());
                    }
                }
            }
            let plain = !text.is_empty() && !text.contains(['*', '?', '[', ']', '{', '}', '\\']);
            plain.then(|| format!("{kind}:{text}"))
        };
        match subject {
            Subject::Shell(cmd) => {
                let Some(split) =
                    shell_split::split_for(cmd, self.powershell).filter(|s| !s.writes)
                else {
                    return Vec::new();
                };
                let parts: Vec<String> = split
                    .parts
                    .iter()
                    .map(|p| normalize(p))
                    .filter(|p| !read_only_command(p))
                    .collect();
                if parts.is_empty() {
                    // Only reading commands: one rule for the first lets the line through.
                    return split
                        .parts
                        .first()
                        .map(|p| normalize(p))
                        .filter(|p| !p.contains('*'))
                        .map(|p| vec![format!("shell:{p}")])
                        .unwrap_or_default();
                }
                if parts.iter().any(|p| p.contains('*')) {
                    return Vec::new();
                }
                let mut out: Vec<String> = Vec::new();
                for p in parts {
                    let rule = format!("shell:{p}");
                    if !out.contains(&rule) {
                        out.push(rule);
                    }
                }
                out
            }
            Subject::Edit(p) => path("edit", p).into_iter().collect(),
            Subject::Edits(ps) => {
                let rules: Vec<_> = ps.iter().map(|p| path("edit", p)).collect();
                if rules.iter().any(Option::is_none) {
                    return Vec::new();
                }
                rules.into_iter().flatten().collect()
            }
            Subject::Read(p) => path("read", p).into_iter().collect(),
            Subject::Mcp { server, tool, .. } if !format!("{server}{tool}").contains('*') => {
                vec![format!("mcp:{server}/{tool}")]
            }
            _ => Vec::new(),
        }
    }

    pub fn check(&self, subject: &Subject) -> Decision {
        // Several files: the strictest answer for any of them.
        if let Subject::Edits(paths) = subject {
            let mut out = Decision::Allow;
            for p in paths {
                match self.check(&Subject::Edit(p.clone())) {
                    deny @ Decision::Deny(_) => return deny,
                    Decision::Ask => out = Decision::Ask,
                    Decision::Allow => {}
                }
            }
            return out;
        }
        let shell = match subject {
            Subject::Shell(cmd) | Subject::ShellOutsideSandbox(cmd) => {
                Some(ShellLine::new(cmd, self.powershell))
            }
            _ => None,
        };
        // Work-mode restrictions run before rules, remembered approvals and yolo.
        if self.work_mode == WorkMode::Plan {
            let changes = match subject {
                Subject::Edit(_)
                | Subject::Memory { write: true }
                | Subject::Agent { read_only: false }
                | Subject::Mcp {
                    read_only: false, ..
                } => true,
                Subject::Shell(_) | Subject::ShellOutsideSandbox(_) => {
                    shell.as_ref().is_some_and(|line| !line.read_only())
                }
                _ => false,
            };
            if changes {
                let reason = if matches!(subject, Subject::Agent { .. }) {
                    "plan mode runs only read-only agents; accept the plan to use this one"
                } else {
                    "plan mode: no changes or test/build commands; list the proposed tests for the user and accept the plan before running them"
                };
                return Decision::Deny(reason.into());
            }
        }
        // Memory is not part of the workspace: reads are free, and writes ask only when
        // configured to (they show in the transcript and are easy to undo).
        // Installed skills are instructions the user put there.
        if matches!(
            subject,
            Subject::Agent { .. } | Subject::Skill | Subject::ToolSearch | Subject::Plan
        ) {
            return Decision::Allow;
        }
        if let Subject::Memory { write } = subject {
            let ask = *write
                && self.memory_writes_ask
                && self.mode < PermissionMode::Allow
                && !self.session.contains(subject);
            return if ask { Decision::Ask } else { Decision::Allow };
        }
        if let Some(line) = &shell
            && line.on_floor()
        {
            return Decision::Deny("blocked by the built-in safety list".into());
        }
        let hit = |rules: &[Rule]| match &shell {
            Some(line) => line.hit(rules),
            None => rules.iter().any(|r| r.matches(subject, &self.root)),
        };
        let allowed = || match &shell {
            Some(line) => line.allowed(&self.allow),
            None => self.allow.iter().any(|r| r.matches(subject, &self.root)),
        };
        if hit(&self.deny) {
            return Decision::Deny("denied by a [permissions] deny rule".into());
        }
        // Yolo: nothing asks. Only the floor and deny rules above (and plan mode) stop a call.
        if self.mode == PermissionMode::Yolo {
            return Decision::Allow;
        }
        // Leaving the sandbox is the user's call in every other mode, allow rules included.
        if let (Subject::ShellOutsideSandbox(_), Some(line)) = (subject, &shell) {
            if self.mode == PermissionMode::ReadOnly && !line.read_only() {
                return Decision::Deny(
                    "only read-only commands run outside the sandbox in this mode".into(),
                );
            }
            return if self.session.contains(subject) {
                Decision::Allow
            } else {
                Decision::Ask
            };
        }
        // Only tools the server marks read-only run while planning or in read-only mode.
        if let Subject::Mcp {
            read_only: false, ..
        } = subject
            && self.mode == PermissionMode::ReadOnly
        {
            return Decision::Deny("read-only mode: this MCP tool may change things".into());
        }
        if self.mode == PermissionMode::ReadOnly && matches!(subject, Subject::Edit(_)) {
            return Decision::Deny("read-only mode: file changes are not allowed".into());
        }
        if hit(&self.ask) {
            return Decision::Ask;
        }
        if self.session.contains(subject) || allowed() {
            return Decision::Allow;
        }
        if self.work_mode == WorkMode::Plan && matches!(subject, Subject::Shell(_)) {
            return Decision::Allow;
        }
        let inside = |p: &Path| paths::inside(&self.root, p);
        match (self.mode, subject) {
            (PermissionMode::Allow, _) => Decision::Allow,
            (_, Subject::Read(p)) if inside(p) => Decision::Allow,
            (PermissionMode::ReadOnly, Subject::Shell(_)) => Decision::Deny(
                "read-only mode: only commands matching an allow rule can run".into(),
            ),
            (PermissionMode::AutoEdit, Subject::Edit(p)) if inside(p) => Decision::Allow,
            (
                _,
                Subject::Mcp {
                    auto_approve: true, ..
                },
            ) => Decision::Allow,
            _ => Decision::Ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(mode: PermissionMode, allow: &[&str], deny: &[&str]) -> Permissions {
        let settings = PermissionSettings {
            mode,
            allow: allow.iter().map(|s| (*s).to_owned()).collect(),
            ask: Vec::new(),
            deny: deny.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        };
        let (p, warnings) = Permissions::new(&settings, Path::new("/repo"));
        assert!(warnings.is_empty(), "{warnings:?}");
        p
    }

    fn shell(c: &str) -> Subject {
        Subject::Shell(c.into())
    }

    fn mcp(tool: &str, read_only: bool, auto_approve: bool) -> Subject {
        Subject::Mcp {
            server: "gh".into(),
            tool: tool.into(),
            read_only,
            auto_approve,
        }
    }

    #[test]
    fn mcp_tools_follow_rules_modes_and_auto_approve() {
        let p = perms(PermissionMode::Ask, &["mcp:gh/get_*"], &["mcp:gh/delete_*"]);
        assert_eq!(p.check(&mcp("get_issue", false, false)), Decision::Allow);
        assert!(matches!(
            p.check(&mcp("delete_repo", false, true)),
            Decision::Deny(_)
        ));
        assert_eq!(p.check(&mcp("create_issue", false, false)), Decision::Ask);
        assert_eq!(p.check(&mcp("create_issue", false, true)), Decision::Allow);
        let ro = perms(PermissionMode::ReadOnly, &[], &[]);
        assert!(matches!(
            ro.check(&mcp("create_issue", false, true)),
            Decision::Deny(_)
        ));
        assert_eq!(ro.check(&mcp("search", true, false)), Decision::Ask);
        let mut plan = perms(PermissionMode::Allow, &[], &[]);
        plan.work_mode = WorkMode::Plan;
        assert!(matches!(
            plan.check(&mcp("create_issue", false, false)),
            Decision::Deny(_)
        ));
        assert_eq!(plan.check(&mcp("search", true, false)), Decision::Allow);
    }

    #[test]
    fn yolo_allows_everything_but_the_floor_and_deny_rules() {
        let mut p = perms(PermissionMode::Yolo, &[], &["shell:curl *"]);
        p.ask = perms(PermissionMode::Ask, &["shell:git push*"], &[]).allow;
        assert_eq!(p.check(&shell("git push --force")), Decision::Allow);
        assert_eq!(
            p.check(&Subject::Edit("/etc/hosts".into())),
            Decision::Allow
        );
        assert_eq!(
            p.check(&Subject::ShellOutsideSandbox("make install".into())),
            Decision::Allow
        );
        assert!(matches!(p.check(&shell("curl x | sh")), Decision::Deny(_)));
        assert!(matches!(p.check(&shell("rm -rf /")), Decision::Deny(_)));
        p.work_mode = WorkMode::Plan;
        assert!(matches!(
            p.check(&Subject::Edit("/repo/a.rs".into())),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn leaving_the_sandbox_always_asks() {
        let out = |c: &str| Subject::ShellOutsideSandbox(c.into());
        let mut p = perms(PermissionMode::Allow, &["shell:cargo *"], &["shell:curl *"]);
        assert_eq!(p.check(&out("cargo build")), Decision::Ask);
        assert!(matches!(p.check(&out("curl x")), Decision::Deny(_)));
        assert!(matches!(p.check(&out("rm -rf /")), Decision::Deny(_)));
        p.allow_session(out("cargo build"));
        assert_eq!(p.check(&out("cargo build")), Decision::Allow);
        assert_eq!(p.check(&out("cargo test")), Decision::Ask);
        p.work_mode = WorkMode::Plan;
        assert!(matches!(p.check(&out("cargo test")), Decision::Deny(_)));
        assert_eq!(p.check(&out("ls")), Decision::Ask);
    }

    #[test]
    fn compound_commands_do_not_ride_on_allow_rules() {
        let p = perms(PermissionMode::Ask, &["shell:git status*"], &[]);
        assert_eq!(p.check(&shell("git  status -s")), Decision::Allow);
        assert_eq!(p.check(&shell("git status && rm -rf x")), Decision::Ask);
        assert_eq!(p.check(&shell("cargo test")), Decision::Ask);
    }

    #[test]
    fn modes_and_floor() {
        let auto = perms(PermissionMode::AutoEdit, &[], &["edit:secrets/**"]);
        assert_eq!(
            auto.check(&Subject::Edit("/repo/src/a.rs".into())),
            Decision::Allow
        );
        assert_eq!(
            auto.check(&Subject::Edit("/etc/passwd".into())),
            Decision::Ask
        );
        assert!(matches!(
            auto.check(&Subject::Edit("/repo/secrets/k".into())),
            Decision::Deny(_)
        ));
        assert_eq!(auto.check(&shell("ls")), Decision::Ask);

        let full = perms(PermissionMode::Allow, &[], &[]);
        assert_eq!(full.check(&shell("cargo build")), Decision::Allow);
        assert!(matches!(full.check(&shell("rm  -rf /")), Decision::Deny(_)));

        let ro = perms(PermissionMode::ReadOnly, &["shell:git log*"], &[]);
        assert!(matches!(
            ro.check(&Subject::Edit("/repo/a".into())),
            Decision::Deny(_)
        ));
        assert_eq!(ro.check(&shell("git log -3")), Decision::Allow);
        assert!(matches!(ro.check(&shell("touch a")), Decision::Deny(_)));
        assert_eq!(ro.check(&Subject::Read("/repo/a".into())), Decision::Allow);
    }

    #[test]
    fn compound_commands_are_checked_per_command() {
        let p = perms(
            PermissionMode::Ask,
            &["shell:git status*", "shell:cargo test*", "shell:echo *"],
            &[],
        );
        assert_eq!(p.check(&shell("git status && rm -rf x")), Decision::Ask);
        assert_eq!(p.check(&shell("git status && cargo test")), Decision::Allow);
        // Commands that only read may join an allowed one, but not stand alone.
        assert_eq!(
            p.check(&shell("cargo test 2>&1 | tail -50")),
            Decision::Allow
        );
        assert_eq!(p.check(&shell("cat a | head")), Decision::Ask);
        // Operators inside quotes are text.
        assert_eq!(p.check(&shell("echo 'a && rm b'")), Decision::Allow);
        // Substitutions run too; redirections write; unreadable lines get no allow.
        assert_eq!(p.check(&shell("echo $(rm x)")), Decision::Ask);
        assert_eq!(p.check(&shell("echo hi > notes.txt")), Decision::Ask);
        assert_eq!(p.check(&shell("echo `rm x`")), Decision::Ask);

        // Deny rules and the floor see every command, even in full-auto.
        let full = perms(PermissionMode::Allow, &[], &["shell:rm *"]);
        assert!(matches!(
            full.check(&shell("git status && rm -rf x")),
            Decision::Deny(_)
        ));
        assert!(matches!(
            full.check(&shell("echo \"$(rm x)\"")),
            Decision::Deny(_)
        ));
        let full = perms(PermissionMode::Allow, &[], &[]);
        assert!(matches!(
            full.check(&shell("cd /tmp; rm -rf /")),
            Decision::Deny(_)
        ));
        assert_eq!(
            full.check(&shell("cargo build && cargo test")),
            Decision::Allow
        );
    }

    #[test]
    fn session_approvals() {
        let mut p = perms(PermissionMode::Ask, &[], &[]);
        p.allow_session(shell("make"));
        assert_eq!(p.check(&shell("make")), Decision::Allow);
        assert_eq!(p.check(&shell("make install")), Decision::Ask);
    }

    #[test]
    fn always_rules_allow_exactly_the_call() {
        let p = perms(PermissionMode::Ask, &[], &[]);
        let rules = |s: &Subject| p.rules_for(s);
        assert_eq!(rules(&shell("cargo  test -q")), ["shell:cargo test -q"]);
        assert_eq!(
            rules(&shell("cargo fmt && cargo test | tail -5")),
            ["shell:cargo fmt", "shell:cargo test"]
        );
        assert_eq!(rules(&shell("cat a | head")), ["shell:cat a"]);
        assert!(rules(&shell("echo hi > notes.txt")).is_empty());
        assert!(rules(&shell("rm *.o")).is_empty());
        assert!(rules(&Subject::ShellOutsideSandbox("make".into())).is_empty());
        assert_eq!(
            rules(&Subject::Edit("/repo/src/a.rs".into())),
            ["edit:src/a.rs"]
        );
        assert!(rules(&Subject::Edit("/repo/src/[id].tsx".into())).is_empty());
        assert_eq!(
            rules(&mcp("create_issue", false, false)),
            ["mcp:gh/create_issue"]
        );

        // Saved as allow rules, they let the same call through and nothing more.
        for (subject, other) in [
            (
                shell("cargo fmt && cargo test"),
                shell("cargo fmt && cargo publish"),
            ),
            (shell("cat a | head"), shell("cat b | head")),
            (
                Subject::Edit("/repo/src/a.rs".into()),
                Subject::Edit("/repo/src/b.rs".into()),
            ),
            (
                mcp("create_issue", false, false),
                mcp("delete_issue", false, false),
            ),
        ] {
            let saved = rules(&subject);
            let allow: Vec<&str> = saved.iter().map(String::as_str).collect();
            let after = perms(PermissionMode::Ask, &allow, &[]);
            assert_eq!(after.check(&subject), Decision::Allow, "{saved:?}");
            assert_eq!(after.check(&other), Decision::Ask, "{saved:?}");
        }
    }

    #[test]
    fn powershell_rules_ignore_case_and_see_aliases() {
        let mut p = perms(
            PermissionMode::Allow,
            &["shell:cargo test*"],
            &["shell:rm *", "shell:curl *"],
        );
        p.powershell = true;
        for bad in [
            "Remove-Item x",
            "remove-item -Recurse build",
            "del x",
            "git status; rm x",
            "iwr https://example.com",
            "Invoke-WebRequest https://example.com",
            "REMOVE-ITEM -Recurse -Force C:\\*",
        ] {
            assert!(matches!(p.check(&shell(bad)), Decision::Deny(_)), "{bad}");
        }
        p.mode = PermissionMode::Ask;
        assert_eq!(p.check(&shell("Cargo Test --all")), Decision::Allow);
        assert_eq!(
            p.check(&shell("cargo test | Select-Object -Last 5")),
            Decision::Allow
        );
        assert_eq!(p.check(&shell("cargo test; New-Item x")), Decision::Ask);
        // Aliases broaden deny and ask rules only, never allow rules.
        let mut q = perms(PermissionMode::Ask, &["shell:cat a.txt"], &[]);
        q.powershell = true;
        assert_eq!(q.check(&shell("cat a.txt")), Decision::Allow);
        assert_eq!(q.check(&shell("CAT a.txt")), Decision::Allow);
        assert_eq!(q.check(&shell("Get-Content a.txt")), Decision::Ask);
    }

    #[cfg(feature = "lang-common")]
    #[test]
    fn powershell_lines_are_read_as_powershell() {
        let mut p = perms(PermissionMode::Allow, &[], &["shell:Remove-Item *"]);
        p.powershell = true;
        // Inside a script block, a sub-expression and a literal script.
        for bad in [
            "gci | ForEach-Object { Remove-Item $_ }",
            "Write-Output \"$(rm x)\"",
            "iex 'Remove-Item x'",
            "pwsh -c 'Get-ChildItem; rm x'",
        ] {
            assert!(matches!(p.check(&shell(bad)), Decision::Deny(_)), "{bad}");
        }
        // Planning: reading cmdlets run, .NET calls and writes don't.
        p.work_mode = WorkMode::Plan;
        for ok in [
            "Get-ChildItem -Recurse src | Select-String foo",
            "Get-Content a.txt | Select-Object -First 10",
            "if (Test-Path x) { Get-Content x }",
            "git log -3 2>&1",
        ] {
            assert_eq!(p.check(&shell(ok)), Decision::Allow, "{ok}");
        }
        for bad in [
            "[IO.File]::Delete('x')",
            "(Get-Item x).Delete()",
            "gci | % Delete",
            "Get-Content a > b",
            "Set-Content a hi",
            "& $cmd",
            "${C:\\x.txt} = 1",
        ] {
            assert!(matches!(p.check(&shell(bad)), Decision::Deny(_)), "{bad}");
        }
    }

    #[test]
    fn plan_mode_only_looks() {
        let (mut p, _) = Permissions::new(
            &PermissionSettings {
                mode: PermissionMode::Allow,
                allow: vec!["shell:cargo check*".into()],
                ..Default::default()
            },
            Path::new("/repo"),
        );
        p.work_mode = WorkMode::Plan;
        assert!(matches!(
            p.check(&Subject::Edit("/repo/a".into())),
            Decision::Deny(_)
        ));
        assert_eq!(p.check(&shell("git status")), Decision::Allow);
        assert_eq!(p.check(&shell("rg foo src")), Decision::Allow);
        assert!(matches!(
            p.check(&shell("cargo check -q")),
            Decision::Deny(_)
        ));
        assert_eq!(p.check(&shell("rg foo | head -5")), Decision::Allow);
        assert_eq!(p.check(&shell("git log 2>/dev/null")), Decision::Allow);
        for bad in [
            "git log | xargs rm",
            "ls $(touch x)",
            "git push",
            "find . -delete",
            "ls; rm x",
            "cat a > b",
            "npm install",
        ] {
            assert!(matches!(p.check(&shell(bad)), Decision::Deny(_)), "{bad}");
        }
        assert_eq!(p.check(&Subject::Read("/repo/a".into())), Decision::Allow);
    }

    #[test]
    fn work_mode_and_approval_matrix() {
        for (work_mode, policy, edit, mcp_read) in [
            (
                WorkMode::Plan,
                PermissionMode::Ask,
                Decision::Deny(String::new()),
                Decision::Ask,
            ),
            (
                WorkMode::Plan,
                PermissionMode::Allow,
                Decision::Deny(String::new()),
                Decision::Allow,
            ),
            (
                WorkMode::Edit,
                PermissionMode::Ask,
                Decision::Ask,
                Decision::Ask,
            ),
            (
                WorkMode::Edit,
                PermissionMode::Allow,
                Decision::Allow,
                Decision::Allow,
            ),
        ] {
            let mut p = perms(policy, &[], &[]);
            p.work_mode = work_mode;
            let actual = p.check(&Subject::Edit("/repo/a".into()));
            assert_eq!(
                std::mem::discriminant(&actual),
                std::mem::discriminant(&edit),
                "{work_mode:?}/{policy:?}: {actual:?}"
            );
            assert_eq!(
                p.check(&mcp("search", true, false)),
                mcp_read,
                "{work_mode:?}/{policy:?}"
            );
        }
    }

    #[test]
    fn planning_rules_and_grants_cannot_authorize_changes_or_tests() {
        for policy in [
            PermissionMode::Ask,
            PermissionMode::Allow,
            PermissionMode::Yolo,
        ] {
            let mut p = perms(policy, &["shell:*", "edit:**", "mcp:*"], &[]);
            for subject in [
                shell("cargo test"),
                shell("cargo check"),
                shell("npm test"),
                shell("python check.py"),
                shell("git branch new-branch"),
                shell("git log --output=log.txt"),
                shell("find . -fprintf result %p"),
                shell("rg --pre=script.py pattern"),
                shell("file -C"),
                Subject::ShellOutsideSandbox("cargo test".into()),
                Subject::Edit("/repo/a".into()),
                Subject::Memory { write: true },
                Subject::Agent { read_only: false },
                mcp("create_issue", false, true),
            ] {
                p.allow_session(subject.clone());
                p.work_mode = WorkMode::Plan;
                assert!(
                    matches!(p.check(&subject), Decision::Deny(_)),
                    "{policy:?}: {subject:?}"
                );
            }
        }
    }

    #[test]
    fn explicit_asks_and_denies_still_apply_to_plan_allow() {
        let mut p = perms(
            PermissionMode::Allow,
            &["shell:git status*"],
            &["mcp:gh/private*"],
        );
        p.work_mode = WorkMode::Plan;
        p.ask = vec![Rule::parse("shell:git status*").expect("rule")];
        p.allow_session(shell("git status"));
        assert_eq!(p.check(&shell("git status")), Decision::Ask);
        assert!(matches!(
            p.check(&mcp("private_search", true, true)),
            Decision::Deny(_)
        ));
    }
}
