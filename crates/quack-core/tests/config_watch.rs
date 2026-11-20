//! Editing a config file during a session is noticed, with `/reload` to apply it.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::workspace;
use quack_core::Engine;
use quack_core::protocol::Event;

#[tokio::test]
async fn an_edited_config_file_is_noticed() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = workspace(tmp.path(), "http://127.0.0.1:9", "[\"tiny\"]", "");
    let user = loaded.dirs.user_config.clone();
    let mut engine = Engine::spawn(Arc::new(loaded));
    // Let the watcher take its first stamp, then change a file it watches.
    tokio::time::sleep(Duration::from_millis(500)).await;
    std::fs::write(user.join("config.toml"), "[ui]\nascii = true\n").expect("write");
    let notice = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match engine.events.recv().await {
                Ok(Event::Notice { message }) if message.contains("/reload") => break message,
                Ok(_) => {}
                Err(e) => panic!("events ended: {e}"),
            }
        }
    })
    .await
    .expect("noticed within 15s");
    assert!(notice.contains("config files changed"), "{notice}");
    engine.shutdown().await;
}

#[tokio::test]
async fn skill_edits_hot_reload_automatically_without_dropping_session() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let loaded = workspace(tmp.path(), "http://127.0.0.1:9", "[\"tiny\"]", "");
    let user = loaded.dirs.user_config.clone();
    let mut engine = Engine::spawn(Arc::new(loaded));
    // Wait for startup and initial watcher snapshot.
    loop {
        if matches!(
            tokio::time::timeout(Duration::from_secs(10), engine.events.recv())
                .await
                .expect("startup")
                .expect("open"),
            Event::StartupComplete
        ) {
            break;
        }
    }
    tokio::time::sleep(Duration::from_millis(500)).await;

    let skill_dir = user.join("skills").join("hot-skill");
    std::fs::create_dir_all(&skill_dir).expect("mkdir");
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: hot-skill\ndescription: added live during session\n---\nRun live check.\n",
    )
    .expect("write skill");

    let (skills, notice) = tokio::time::timeout(Duration::from_secs(15), async {
        let mut seen_skills = None;
        let mut seen_notice = None;
        while seen_skills.is_none() || seen_notice.is_none() {
            match engine.events.recv().await {
                Ok(Event::Skills { skills }) => seen_skills = Some(skills),
                Ok(Event::Notice { message }) if message.contains("hot-reloaded") => {
                    seen_notice = Some(message);
                }
                Ok(_) => {}
                Err(e) => panic!("events ended: {e}"),
            }
        }
        (seen_skills.expect("skills"), seen_notice.expect("notice"))
    })
    .await
    .expect("hot-reloaded within 15s");
    assert!(notice.contains("skills"), "{notice}");
    assert!(
        skills.iter().any(|s| s.name == "hot-skill"),
        "expected hot-reloaded skill to be active immediately: {skills:?}"
    );
    engine.shutdown().await;
}
