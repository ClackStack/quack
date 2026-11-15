//! Concurrency and webview replacement against the real engine, with isolated
//! config and controlled gates rather than timing-dependent slow model calls.
mod common;

use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use quack_core::Resume;
use quack_gui::registry::Loader;
use quack_gui::{OpenError, Payload, Registry, Sink, TabMessage};
use quack_protocol::Event;
use tokio::sync::Notify;

use common::{collect_until, collecting_sink, fixed_loader, ws_of};

#[derive(Default)]
struct Gate {
    released: Mutex<bool>,
    wake: Condvar,
    entered: Notify,
}

impl Gate {
    fn block(&self) {
        self.entered.notify_one();
        let mut released = self.released.lock().expect("gate lock");
        while !*released {
            released = self.wake.wait(released).expect("gate wait");
        }
    }

    fn release(&self) {
        *self.released.lock().expect("gate lock") = true;
        self.wake.notify_all();
    }

    async fn entered(&self) {
        tokio::time::timeout(Duration::from_secs(5), self.entered.notified())
            .await
            .expect("operation reached gate");
    }
}

/// Release blocking workers even if a test assertion panics.
struct Release(Arc<Gate>);
impl Drop for Release {
    fn drop(&mut self) {
        self.0.release();
    }
}

fn loaded(root: &Path) -> quack_config::Loaded {
    // There are no inference calls in these lifecycle tests. This configured
    // loopback endpoint will only be used if a test incorrectly submits a turn.
    common::workspace(root, "http://127.0.0.1:1")
}

#[tokio::test]
async fn slow_config_load_does_not_block_another_tabs_controls() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let fast = loaded(&tmp.path().join("fast"));
    let slow = loaded(&tmp.path().join("slow"));
    let fast_cwd = fast.dirs.workspace_root.clone();
    let slow_cwd = slow.dirs.workspace_root.clone();
    let gate = Arc::new(Gate::default());
    let _release = Release(gate.clone());
    let loader: Loader = Arc::new({
        let gate = gate.clone();
        let fast_cwd = fast_cwd.clone();
        move |options| {
            if options.cwd == fast_cwd {
                Ok(fast.clone())
            } else {
                gate.block();
                Ok(slow.clone())
            }
        }
    });
    let registry = Arc::new(Registry::with_parts(loader, tmp.path().join("recent")));
    let (sink, mut rx) = collecting_sink();
    let tab = registry
        .open(fast_cwd, Resume::New, sink)
        .await
        .expect("fast open");
    collect_until(&mut rx, |event| matches!(event, Event::StartupComplete)).await;
    let opening = tokio::spawn({
        let registry = registry.clone();
        async move {
            registry
                .open(slow_cwd, Resume::New, collecting_sink().0)
                .await
        }
    });
    gate.entered().await;
    tokio::time::timeout(Duration::from_secs(1), async {
        registry
            .send_op(tab, serde_json::json!({"op": "interrupt"}))
            .await
            .expect("interrupt");
        registry
            .send_op(tab, serde_json::json!({"op": "list_sessions"}))
            .await
            .expect("list sessions");
        collect_until(&mut rx, |event| matches!(event, Event::SessionList { .. })).await;
    })
    .await
    .expect("other tab stays responsive");
    gate.release();
    opening.await.expect("open task").expect("slow open");
    registry.close_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn closing_keeps_workspace_reserved_without_blocking_other_tabs() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let first = loaded(&tmp.path().join("first"));
    let second = loaded(&tmp.path().join("second"));
    let first_cwd = first.dirs.workspace_root.clone();
    let second_cwd = second.dirs.workspace_root.clone();
    let loader: Loader = Arc::new({
        let first_cwd = first_cwd.clone();
        move |options| {
            Ok(if options.cwd == first_cwd {
                first.clone()
            } else {
                second.clone()
            })
        }
    });
    let registry = Arc::new(Registry::with_parts(loader, tmp.path().join("recent")));
    let gate = Arc::new(Gate::default());
    let _release = Release(gate.clone());
    let sink: Arc<dyn Sink> = Arc::new({
        let gate = gate.clone();
        move |message: TabMessage| {
            if matches!(message.payload, Payload::Closed { .. }) {
                gate.block();
            }
        }
    });
    let first_tab = registry
        .open(first_cwd.clone(), Resume::New, sink)
        .await
        .expect("first open");
    let (sink, mut rx) = collecting_sink();
    let second_tab = registry
        .open(second_cwd, Resume::New, sink)
        .await
        .expect("second open");
    collect_until(&mut rx, |event| matches!(event, Event::StartupComplete)).await;
    let closing = tokio::spawn({
        let registry = registry.clone();
        async move { registry.close(first_tab).await }
    });
    gate.entered().await;
    let reopening = tokio::spawn({
        let registry = registry.clone();
        async move {
            registry
                .open(first_cwd, Resume::New, collecting_sink().0)
                .await
        }
    });
    tokio::time::timeout(Duration::from_secs(1), async {
        registry
            .send_op(second_tab, serde_json::json!({"op": "interrupt"}))
            .await
            .expect("interrupt");
        registry
            .send_op(second_tab, serde_json::json!({"op": "list_sessions"}))
            .await
            .expect("list sessions");
        collect_until(&mut rx, |event| matches!(event, Event::SessionList { .. })).await;
    })
    .await
    .expect("other tab stays responsive during close");
    assert!(!closing.is_finished());
    assert!(
        !reopening.is_finished(),
        "workspace ownership survives until shutdown completes"
    );
    gate.release();
    closing.await.expect("close task");
    let replacement = reopening.await.expect("reopen task").expect("reopen");
    assert_ne!(replacement, first_tab);
    registry.close_all().await;
}

#[tokio::test]
async fn reconnect_stops_old_engines_and_refuses_stale_commands() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let registry =
        Registry::with_parts(fixed_loader(loaded(tmp.path())), tmp.path().join("recent"));
    let old = registry.connect().await.expect("connect");
    let (sink, mut rx) = collecting_sink();
    let tab = registry
        .open_for(old, ws_of(tmp.path()), Resume::New, sink)
        .await
        .expect("open");
    registry.disconnect();
    assert!(
        registry.check_client(old).is_err(),
        "navigation invalidates the discarded page immediately"
    );
    let current = registry.connect().await.expect("reconnect");
    assert_ne!(old, current);
    assert!(
        registry.is_empty(),
        "old engines finished before reconnect returns"
    );
    common::until(&mut rx, |m| matches!(m.payload, Payload::Closed { .. })).await;
    assert!(
        registry
            .send_op_for(old, tab, serde_json::json!({"op": "interrupt"}))
            .await
            .is_err()
    );
    assert!(registry.close_for(old, tab).await.is_err());
    assert!(matches!(
        registry
            .open_for(old, ws_of(tmp.path()), Resume::New, collecting_sink().0)
            .await,
        Err(OpenError::Engine(_))
    ));
    registry
        .open_for(current, ws_of(tmp.path()), Resume::New, collecting_sink().0)
        .await
        .expect("new connection owns workspace");
    registry.close_all().await;
}

#[tokio::test]
async fn config_load_started_before_reconnect_cannot_spawn_an_orphan_engine() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = loaded(tmp.path());
    let gate = Arc::new(Gate::default());
    let _release = Release(gate.clone());
    let loader: Loader = Arc::new({
        let gate = gate.clone();
        move |_| {
            gate.block();
            Ok(loaded.clone())
        }
    });
    let registry = Arc::new(Registry::with_parts(loader, tmp.path().join("recent")));
    let client = registry.connect().await.expect("connect");
    let (sink, mut rx) = collecting_sink();
    let opening = tokio::spawn({
        let registry = registry.clone();
        let cwd = ws_of(tmp.path());
        async move { registry.open_for(client, cwd, Resume::New, sink).await }
    });
    gate.entered().await;
    registry
        .connect()
        .await
        .expect("reconnect does not await stale config IO");
    gate.release();
    assert!(matches!(
        opening.await.expect("open task"),
        Err(OpenError::Engine(_))
    ));
    assert!(registry.is_empty());
    assert!(
        rx.try_recv().is_err(),
        "discarded open never emitted an engine snapshot"
    );
    assert!(registry.recent_folders(10).is_empty());
}

#[tokio::test]
async fn simultaneous_opens_reserve_one_workspace_before_startup() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let registry =
        Registry::with_parts(fixed_loader(loaded(tmp.path())), tmp.path().join("recent"));
    let (first, second) = tokio::join!(
        registry.open(ws_of(tmp.path()), Resume::New, collecting_sink().0),
        registry.open(ws_of(tmp.path()), Resume::New, collecting_sink().0)
    );
    assert!(
        matches!((&first, &second), (Ok(tab), Err(OpenError::AlreadyOpen { tab: existing }))
        | (Err(OpenError::AlreadyOpen { tab: existing }), Ok(tab)) if tab == existing),
        "{first:?}, {second:?}"
    );
    registry.close_all().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelling_an_open_request_shuts_down_its_reserved_engine() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let registry = Arc::new(Registry::with_parts(
        fixed_loader(loaded(tmp.path())),
        tmp.path().join("recent"),
    ));
    let gate = Arc::new(Gate::default());
    let _release = Release(gate.clone());
    let (collected, mut rx) = collecting_sink();
    let sink: Arc<dyn Sink> = Arc::new({
        let gate = gate.clone();
        move |message: TabMessage| {
            if matches!(message.payload, Payload::Info { .. }) {
                gate.block();
            }
            collected.send(message);
        }
    });
    let opening = tokio::spawn({
        let registry = registry.clone();
        let cwd = ws_of(tmp.path());
        async move { registry.open(cwd, Resume::New, sink).await }
    });
    gate.entered().await;
    opening.abort();
    assert!(opening.await.expect_err("cancelled open").is_cancelled());
    gate.release();
    common::until(&mut rx, |m| matches!(m.payload, Payload::Closed { .. })).await;
    tokio::time::timeout(Duration::from_secs(1), async {
        while !registry.is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("abandoned engine stopped without a separate close request");
    registry
        .open(ws_of(tmp.path()), Resume::New, collecting_sink().0)
        .await
        .expect("workspace released");
    registry.close_all().await;
}
