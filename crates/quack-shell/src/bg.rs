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
use tokio::sync::mpsc;
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
    exit_code: Option<i32>,
    ring: VecDeque<u8>,
    dropped_bytes: usize,
}

impl JobState {
    fn push(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.ring.len() == MAX_RING_BYTES {
                self.ring.pop_front();
                self.dropped_bytes += 1;
            }
            self.ring.push_back(b);
        }
    }

    fn tail_string(&self, max_bytes: usize) -> String {
        let take = self.ring.len().min(max_bytes);
        let skip = self.ring.len().saturating_sub(take);
        let bytes: Vec<u8> = self.ring.iter().skip(skip).copied().collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[derive(Debug)]
struct BgJob {
    id: String,
    command: String,
    started: Instant,
    tree: Tree,
    cancel: CancellationToken,
    state: Mutex<JobState>,
}

impl BgJob {
    fn snapshot(&self, tail_bytes: usize) -> BgStatus {
        let guard = self.state.lock().expect("job state mutex");
        BgStatus {
            id: self.id.clone(),
            command: self.command.clone(),
            running: guard.running,
            exit_code: guard.exit_code,
            elapsed_secs: self.started.elapsed().as_secs(),
            output: guard.tail_string(tail_bytes),
            dropped_bytes: guard.dropped_bytes,
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
        }
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
            state: Mutex::new(JobState {
                running: true,
                exit_code: None,
                ring: VecDeque::new(),
                dropped_bytes: 0,
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
            }
        });

        Ok(id)
    }

    /// Read the current status and tail output of `id`.
    pub fn output(&self, id: &str, tail_bytes: Option<usize>) -> Result<BgStatus, String> {
        let job = self.get(id)?;
        Ok(job.snapshot(tail_bytes.unwrap_or(DEFAULT_TAIL_BYTES)))
    }

    /// Stop `id` (killing its whole process tree) and return its final status and output.
    pub fn stop(&self, id: &str) -> Result<BgStatus, String> {
        let job = self.get(id)?;
        job.terminate();
        Ok(job.snapshot(DEFAULT_TAIL_BYTES))
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
}
