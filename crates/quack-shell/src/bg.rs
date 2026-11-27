//! Managed background shell processes (`shell` with `background: true`).
//!
//! Each job runs inside the configured shell and OS sandbox, buffers merged stdout/stderr
//! in a bounded ring buffer, tracks its live exit code, and kills its entire process tree
//! when explicitly stopped or when the owning [`BackgroundManager`] is dropped.

use std::collections::{BTreeMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::sync::{Notify, mpsc};
use tokio_util::sync::CancellationToken;

use crate::{ShellKind, Tree};

/// Maximum concurrent active background jobs per session.
const MAX_ACTIVE_JOBS: usize = 8;
/// Maximum output kept in memory per background job (512 KiB ring buffer).
const MAX_RING_BYTES: usize = 512 * 1024;
/// Default bytes returned when inspecting a job's output without an explicit limit.
const DEFAULT_TAIL_BYTES: usize = 16 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgStatus {
    pub id: String,
    pub command: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub elapsed_secs: u64,
    pub output: String,
    pub dropped_bytes: usize,
    /// `true` when this snapshot returns only new output since an earlier read.
    pub is_delta: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BgSummary {
    pub id: String,
    pub command: String,
    pub running: bool,
    pub exit_code: Option<i32>,
    pub elapsed_secs: u64,
}

#[derive(Debug)]
struct JobState {
    running: bool,
    /// Set once the process has exited (or been terminated) and output draining has finished.
    finished: bool,
    exit_code: Option<i32>,
    ring: VecDeque<u8>,
    /// Total bytes ever written by the job (`ring.len() + dropped_bytes == total_written`).
    total_written: usize,
    dropped_bytes: usize,
    /// Byte offset up to which the caller has already read.
    read_cursor: usize,
    has_read: bool,
}

impl JobState {
    fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.ring.len() == MAX_RING_BYTES {
                self.ring.pop_front();
                self.dropped_bytes += 1;
            }
            self.ring.push_back(b);
            self.total_written += 1;
        }
    }

    fn take_delta(&mut self, max_bytes: usize) -> (String, usize, bool) {
        let is_delta = self.has_read;
        let ring_start = self.dropped_bytes;
        let unread_dropped = ring_start.saturating_sub(self.read_cursor);
        let unread_start = self.read_cursor.max(ring_start);
        let unread_len = self.total_written.saturating_sub(unread_start);
        let take = unread_len.min(max_bytes);
        let skip = self.ring.len().saturating_sub(take);
        let bytes: Vec<u8> = self.ring.iter().skip(skip).copied().collect();
        self.read_cursor = self.total_written;
        self.has_read = true;
        (
            String::from_utf8_lossy(&bytes).into_owned(),
            unread_dropped,
            is_delta,
        )
    }
}

#[derive(Debug)]
struct BgJob {
    id: String,
    command: String,
    started: Instant,
    tree: Tree,
    cancel: CancellationToken,
    done: Notify,
    state: Mutex<JobState>,
}

impl BgJob {
    fn is_finished(&self) -> bool {
        self.state.lock().is_ok_and(|s| s.finished)
    }

    fn snapshot_delta(&self, tail_bytes: usize) -> BgStatus {
        let mut guard = self.state.lock().expect("job state mutex");
        let (output, dropped_bytes, is_delta) = guard.take_delta(tail_bytes);
        BgStatus {
            id: self.id.clone(),
            command: self.command.clone(),
            running: guard.running,
            exit_code: guard.exit_code,
            elapsed_secs: self.started.elapsed().as_secs(),
            output,
            dropped_bytes,
            is_delta,
        }
    }

    fn summary(&self) -> BgSummary {
        let guard = self.state.lock().expect("job state mutex");
        BgSummary {
            id: self.id.clone(),
            command: self.command.clone(),
            running: guard.running,
            exit_code: guard.exit_code,
            elapsed_secs: self.started.elapsed().as_secs(),
        }
    }

    fn terminate(&self) {
        self.cancel.cancel();
        self.tree.kill();
        if let Ok(mut guard) = self.state.lock() {
            guard.running = false;
            guard.finished = true;
        }
        self.done.notify_waiters();
    }
}

/// Session-scoped registry of managed background shell jobs. Dropping it kills every
/// running process tree so no background server outlives its session.
#[derive(Debug, Default)]
pub struct BackgroundManager {
    jobs: Mutex<BTreeMap<String, Arc<BgJob>>>,
    next: AtomicU64,
}

impl BackgroundManager {
    pub fn new() -> Self {
        Self::default()
    }

    /// Spawn `script` in the background and return its job id (`bg-1`, `bg-2`, …).
    pub fn start(
        &self,
        shell: &ShellKind,
        script: &str,
        cwd: PathBuf,
        sandbox: Option<Arc<quack_sandbox::Policy>>,
    ) -> Result<String, String> {
        {
            let jobs = self.jobs.lock().map_err(|_| "background lock poisoned")?;
            let active = jobs
                .values()
                .filter(|j| j.state.lock().is_ok_and(|s| s.running))
                .count();
            if active >= MAX_ACTIVE_JOBS {
                return Err(format!(
                    "too many active background jobs ({active}/{MAX_ACTIVE_JOBS}); stop an existing job first"
                ));
            }
        }

        let mut cmd = match sandbox {
            Some(policy) => {
                quack_sandbox::sandboxed(&policy, shell.program().as_os_str(), shell.args(script))?
            }
            None => quack_sandbox::unsandboxed(shell.program().as_os_str(), shell.args(script)),
        };
        cmd.current_dir(&cwd)
            .stdin_null()
            .stdout_piped()
            .stderr_piped()
            .kill_on_drop(true)
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("DEBIAN_FRONTEND", "noninteractive");
        #[cfg(unix)]
        cmd.process_group(0);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);

        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start {}: {e}", shell.program().display()))?;
        let tree = Tree::from_sandbox_child(&child);
        let id = format!("bg-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        let cancel = CancellationToken::new();

        let (tx, mut rx) = mpsc::unbounded_channel::<Vec<u8>>();
        if let Some(out) = child.take_stdout() {
            tokio::spawn(pump(out, tx.clone()));
        }
        if let Some(err) = child.take_stderr() {
            tokio::spawn(pump(err, tx.clone()));
        }
        drop(tx);

        let job = Arc::new(BgJob {
            id: id.clone(),
            command: script.to_owned(),
            started: Instant::now(),
            tree,
            cancel: cancel.clone(),
            done: Notify::new(),
            state: Mutex::new(JobState {
                running: true,
                finished: false,
                exit_code: None,
                ring: VecDeque::new(),
                total_written: 0,
                dropped_bytes: 0,
                read_cursor: 0,
                has_read: false,
            }),
        });

        {
            let mut jobs = self.jobs.lock().map_err(|_| "background lock poisoned")?;
            jobs.insert(id.clone(), Arc::clone(&job));
        }

        let watcher = Arc::clone(&job);
        tokio::spawn(async move {
            let mut output_open = true;
            let mut status = None;
            loop {
                tokio::select! {
                    chunk = rx.recv(), if output_open => match chunk {
                        Some(bytes) => {
                            if let Ok(mut s) = watcher.state.lock() {
                                s.push(&bytes);
                            }
                        }
                        None => output_open = false,
                    },
                    res = child.wait(), if status.is_none() => {
                        status = Some(res.ok().and_then(|s| s.code()));
                        if let Ok(mut s) = watcher.state.lock() {
                            s.exit_code = status.flatten();
                            s.running = false;
                        }
                    },
                    () = cancel.cancelled() => {
                        watcher.tree.kill();
                        let _ = child.kill().await;
                        if let Ok(mut s) = watcher.state.lock() {
                            s.running = false;
                        }
                        break;
                    }
                }
                if status.is_some() && !output_open {
                    break;
                }
                if status.is_some() && output_open {
                    match tokio::time::timeout(Duration::from_millis(150), rx.recv()).await {
                        Ok(Some(bytes)) => {
                            if let Ok(mut s) = watcher.state.lock() {
                                s.push(&bytes);
                            }
                        }
                        Ok(None) | Err(_) => break,
                    }
                }
            }
            watcher.tree.kill();
            if let Ok(mut s) = watcher.state.lock() {
                s.running = false;
                s.finished = true;
            }
            watcher.done.notify_waiters();
        });

        Ok(id)
    }

    /// Read the current status and new output of `id` since the last read.
    pub fn output(&self, id: &str, tail_bytes: Option<usize>) -> Result<BgStatus, String> {
        let job = self.get(id)?;
        Ok(job.snapshot_delta(tail_bytes.unwrap_or(DEFAULT_TAIL_BYTES)))
    }

    /// Wait up to `wait` for `id` to finish (or until `cancel` fires), then return its
    /// status and any new output since the previous read.
    pub async fn wait_output(
        &self,
        id: &str,
        tail_bytes: Option<usize>,
        wait: Duration,
        cancel: CancellationToken,
    ) -> Result<BgStatus, String> {
        let job = self.get(id)?;
        let tail = tail_bytes.unwrap_or(DEFAULT_TAIL_BYTES);
        if !wait.is_zero() {
            let deadline = tokio::time::Instant::now() + wait;
            loop {
                let notified = job.done.notified();
                tokio::pin!(notified);
                notified.as_mut().enable();
                if job.is_finished() {
                    break;
                }
                tokio::select! {
                    () = notified => {
                        if job.is_finished() {
                            break;
                        }
                    }
                    () = cancel.cancelled() => break,
                    () = tokio::time::sleep_until(deadline) => break,
                }
            }
        }
        Ok(job.snapshot_delta(tail))
    }

    /// Stop `id` (killing its whole process tree) and return its final status and unread output.
    pub fn stop(&self, id: &str) -> Result<BgStatus, String> {
        let job = self.get(id)?;
        job.terminate();
        Ok(job.snapshot_delta(DEFAULT_TAIL_BYTES))
    }

    /// List every job started in this session (running and exited).
    pub fn list(&self) -> Vec<BgSummary> {
        let Ok(jobs) = self.jobs.lock() else {
            return Vec::new();
        };
        jobs.values().map(|j| j.summary()).collect()
    }

    /// Kill every running job immediately.
    pub fn stop_all(&self) {
        let Ok(jobs) = self.jobs.lock() else {
            return;
        };
        for job in jobs.values() {
            job.terminate();
        }
    }

    fn get(&self, id: &str) -> Result<Arc<BgJob>, String> {
        let jobs = self.jobs.lock().map_err(|_| "background lock poisoned")?;
        jobs.get(id)
            .cloned()
            .ok_or_else(|| format!("unknown background job `{id}`"))
    }
}

impl Drop for BackgroundManager {
    fn drop(&mut self) {
        self.stop_all();
    }
}

async fn pump(mut r: impl AsyncRead + Unpin, tx: mpsc::UnboundedSender<Vec<u8>>) {
    let mut buf = vec![0u8; 8192];
    loop {
        match r.read(&mut buf).await {
            Ok(0) | Err(_) => return,
            Ok(n) => {
                if tx.send(buf[..n].to_vec()).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_manager_and_unknown_job() {
        let mgr = BackgroundManager::new();
        assert!(mgr.list().is_empty());
        assert!(mgr.output("bg-999", None).is_err());
        assert!(mgr.stop("bg-999").is_err());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn start_poll_and_stop_background_job() {
        let mgr = BackgroundManager::new();
        let sh = ShellKind::detect("sh", None);
        let id = mgr
            .start(&sh, "echo ready; sleep 30", std::env::temp_dir(), None)
            .expect("start");
        tokio::time::sleep(Duration::from_millis(150)).await;
        let st = mgr.output(&id, None).expect("output");
        assert!(st.running, "{st:?}");
        assert!(st.output.contains("ready"), "{st:?}");
        assert_eq!(mgr.list().len(), 1);

        let stopped = mgr.stop(&id).expect("stop");
        assert!(!stopped.running);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn wait_output_blocks_until_exit_and_returns_deltas() {
        let mgr = BackgroundManager::new();
        let sh = ShellKind::detect("sh", None);
        let id = mgr
            .start(
                &sh,
                "echo first; sleep 0.25; echo second",
                std::env::temp_dir(),
                None,
            )
            .expect("start");
        tokio::time::sleep(Duration::from_millis(80)).await;
        let first = mgr.output(&id, None).expect("first read");
        assert!(first.running, "{first:?}");
        assert!(!first.is_delta);
        assert!(first.output.contains("first"), "{first:?}");

        // Immediate second read sees no new output (delta).
        let empty = mgr.output(&id, None).expect("empty delta");
        assert!(empty.is_delta);
        assert!(empty.output.is_empty(), "{empty:?}");

        // wait_output blocks until the command exits and returns only the new line.
        let done = mgr
            .wait_output(&id, None, Duration::from_secs(5), CancellationToken::new())
            .await
            .expect("wait");
        assert!(!done.running, "{done:?}");
        assert_eq!(done.exit_code, Some(0));
        assert!(done.is_delta);
        assert!(done.output.contains("second"), "{done:?}");
        assert!(
            !done.output.contains("first"),
            "delta should not repeat earlier output: {done:?}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn wait_output_honors_cancellation() {
        let mgr = BackgroundManager::new();
        let sh = ShellKind::detect("sh", None);
        let id = mgr
            .start(&sh, "sleep 30", std::env::temp_dir(), None)
            .expect("start");
        let cancel = CancellationToken::new();
        let trigger = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(80)).await;
            trigger.cancel();
        });
        let started = Instant::now();
        let st = mgr
            .wait_output(&id, None, Duration::from_secs(10), cancel)
            .await
            .expect("wait");
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(st.running);
        let _ = mgr.stop(&id);
    }
}
