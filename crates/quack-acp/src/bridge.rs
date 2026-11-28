//! ACP editor client-capability bridge: delegates `fs/read_text_file`,
//! `fs/write_text_file`, and `terminal/*` requests to the host editor when both
//! `[tools] acp_client_fs` / `acp_client_terminal` and the editor's `clientCapabilities`
//! enable them.

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use futures::future::BoxFuture;
use quack_core::{ClientBridge, TerminalOutput};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::rpc::Peer;
use crate::wire::ClientCapabilities;

#[derive(Debug)]
pub(crate) struct AcpBridge {
    peer: Peer,
    caps: ClientCapabilities,
    session_id: Arc<OnceLock<String>>,
}

impl AcpBridge {
    pub fn new(peer: Peer, caps: ClientCapabilities) -> Arc<Self> {
        Arc::new(Self {
            peer,
            caps,
            session_id: Arc::new(OnceLock::new()),
        })
    }

    pub fn set_session_id(&self, id: String) {
        let _ = self.session_id.set(id);
    }

    fn session_id(&self) -> Option<&str> {
        self.session_id.get().map(String::as_str)
    }
}

impl ClientBridge for AcpBridge {
    fn read_text_file<'a>(
        &'a self,
        path: &'a Path,
    ) -> BoxFuture<'a, Option<Result<String, String>>> {
        Box::pin(async move {
            if !self.caps.fs.read_text_file {
                return None;
            }
            let sid = self.session_id()?;
            let req = json!({
                "sessionId": sid,
                "path": path.display().to_string(),
            });
            match self.peer.request("fs/read_text_file", req).await {
                Ok(v) => match v.get("content").and_then(|c| c.as_str()) {
                    Some(content) => Some(Ok(content.to_owned())),
                    None => Some(Err(
                        "editor fs/read_text_file response missing `content`".into()
                    )),
                },
                Err(e) => Some(Err(format!("editor fs/read_text_file: {}", e.message))),
            }
        })
    }

    fn write_text_file<'a>(
        &'a self,
        path: &'a Path,
        content: &'a str,
    ) -> BoxFuture<'a, Option<Result<(), String>>> {
        Box::pin(async move {
            if !self.caps.fs.write_text_file {
                return None;
            }
            let sid = self.session_id()?;
            let req = json!({
                "sessionId": sid,
                "path": path.display().to_string(),
                "content": content,
            });
            match self.peer.request("fs/write_text_file", req).await {
                Ok(_) => Some(Ok(())),
                Err(e) => Some(Err(format!("editor fs/write_text_file: {}", e.message))),
            }
        })
    }

    fn run_terminal<'a>(
        &'a self,
        command: &'a str,
        cwd: &'a Path,
        timeout: Duration,
        cancel: CancellationToken,
    ) -> BoxFuture<'a, Option<Result<TerminalOutput, String>>> {
        Box::pin(async move {
            if !self.caps.terminal {
                return None;
            }
            let sid = self.session_id()?;
            let created = match self
                .peer
                .request(
                    "terminal/create",
                    json!({
                        "sessionId": sid,
                        "command": command,
                        "cwd": cwd.display().to_string(),
                    }),
                )
                .await
            {
                Ok(v) => v,
                Err(e) => return Some(Err(format!("editor terminal/create: {}", e.message))),
            };
            let Some(tid) = created.get("terminalId").and_then(|v| v.as_str()) else {
                return Some(Err(
                    "editor terminal/create response missing `terminalId`".into()
                ));
            };
            let tid = tid.to_owned();
            let base = json!({ "sessionId": sid, "terminalId": tid });

            let mut timed_out = false;
            let mut cancelled = false;
            let mut exit_code: Option<i32> = None;

            tokio::select! {
                res = self.peer.request("terminal/wait_for_exit", base.clone()) => {
                    if let Ok(v) = res {
                        exit_code = v
                            .get("exitCode")
                            .and_then(|c| c.as_i64())
                            .and_then(|c| i32::try_from(c).ok());
                    }
                }
                () = tokio::time::sleep(timeout) => {
                    timed_out = true;
                    let _ = self.peer.request("terminal/kill", base.clone()).await;
                }
                () = cancel.cancelled() => {
                    cancelled = true;
                    let _ = self.peer.request("terminal/kill", base.clone()).await;
                }
            }

            let output_resp = self.peer.request("terminal/output", base.clone()).await;
            let _ = self.peer.request("terminal/release", base).await;

            match output_resp {
                Ok(v) => {
                    let output = v
                        .get("output")
                        .and_then(|o| o.as_str())
                        .unwrap_or_default()
                        .to_owned();
                    if exit_code.is_none() {
                        exit_code = v
                            .get("exitStatus")
                            .and_then(|s| s.get("exitCode"))
                            .and_then(|c| c.as_i64())
                            .and_then(|c| i32::try_from(c).ok());
                    }
                    Some(Ok(TerminalOutput {
                        output,
                        exit_code,
                        timed_out,
                        cancelled,
                    }))
                }
                Err(e) => Some(Err(format!("editor terminal/output: {}", e.message))),
            }
        })
    }
}
