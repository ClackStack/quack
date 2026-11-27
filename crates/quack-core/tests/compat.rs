mod common;

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use quack_core::protocol::{CompatFolder, CompatKind, Event, Op, WriteScope};
use quack_core::{Engine, EngineHandle};
use wiremock::{Mock, MockServer, matchers::method};

fn write(root: &Path, file: &str, text: &str) {
    let file = root.join(file);
    std::fs::create_dir_all(file.parent().expect("parent directory")).expect("create directory");
    std::fs::write(file, text).expect("write fixture");
}

async fn until(engine: &mut EngineHandle, stop: impl Fn(&Event) -> bool) -> Vec<Event> {
    tokio::time::timeout(Duration::from_secs(10), async {
        let mut events = Vec::new();
        loop {
            let event = engine.events.recv().await.expect("event channel open");
            let done = stop(&event);
            events.push(event);
            if done {
                return events;
            }
        }
    })
    .await
    .expect("event arrived")
}

async fn startup(engine: &mut EngineHandle) -> Vec<Event> {
    until(engine, |event| matches!(event, Event::StartupComplete)).await
}

fn folders(events: &[Event]) -> &[CompatFolder] {
    events
        .iter()
        .find_map(|e| match e {
            Event::CompatFolders { folders } => Some(folders.as_slice()),
            _ => None,
        })
        .expect("compat folders reported")
}

async fn import(engine: &EngineHandle, folder: &CompatFolder, save: bool) {
    engine
        .submit(Op::ImportCompat {
            kind: folder.kind,
            path: folder.path.clone(),
            save,
        })
        .await;
}

#[tokio::test]
async fn loads_explicitly_then_only_restores_saved_folders_on_restart() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(common::sse_text("ok"))
        .mount(&server)
        .await;
    let temp = tempfile::tempdir().expect("test directory");
    let loaded = common::workspace(temp.path(), &server.uri(), r#"["tiny"]"#, "");
    write(
        &loaded.dirs.user_config,
        "compat.toml",
        "[skills]\nproject = ['.claude/skills/']\n[commands]\nproject = ['.claude/commands/']\n",
    );
    let root = &loaded.dirs.workspace_root;
    write(
        root,
        ".claude/skills/foreign-skill/SKILL.md",
        "---\nname: foreign-skill\ndescription: Imported skill description\n---\nImported skill body.",
    );
    write(
        root,
        ".claude/commands/git/foreign.md",
        "Imported command: $ARGUMENTS",
    );
    let mut engine = Engine::spawn(Arc::new(loaded.clone()));

    let events = startup(&mut engine).await;
    let [skill, command] = folders(&events) else {
        panic!("expected two folders")
    };
    assert_eq!(
        (skill.kind, command.kind),
        (CompatKind::Skills, CompatKind::Commands)
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Skills { skills } if skills.is_empty()))
    );
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Commands { commands } if commands.is_empty()))
    );
    common::turn(&mut engine, "First turn").await;

    import(&engine, skill, false).await;
    let enabled = until(
        &mut engine,
        |e| matches!(e, Event::Notice { message } if message.ends_with("for this run")),
    )
    .await;
    assert!(enabled.iter().any(|e| matches!(e, Event::Skills { skills } if skills.iter().any(|s| s.name == "foreign-skill"))));
    assert!(!loaded.dirs.project.join("config.toml").exists());
    common::turn(&mut engine, "Second turn").await;

    import(&engine, command, true).await;
    until(
        &mut engine,
        |e| matches!(e, Event::Notice { message } if message.contains("saved its path")),
    )
    .await;
    assert_eq!(
        loaded.reload().expect("reload").project,
        quack_config::ProjectStatus::SettingsOnly
    );
    engine.submit(Op::ReloadConfig).await;
    let reloaded = startup(&mut engine).await;
    assert!(folders(&reloaded).is_empty());
    assert!(reloaded.iter().any(|e| matches!(e, Event::Skills { skills } if skills.iter().any(|s| s.name == "foreign-skill"))));
    common::turn(&mut engine, "/git:foreign details").await;

    let bodies = common::bodies(&server).await;
    assert!(!bodies[0].contains("foreign-skill"));
    assert!(bodies[1].contains("Imported skill description"));
    assert!(bodies[2].contains("Imported command: details"));
    let first: serde_json::Value = serde_json::from_str(&bodies[0]).expect("first request");
    let second: serde_json::Value = serde_json::from_str(&bodies[1]).expect("second request");
    let prefix = first["messages"].as_array().expect("messages");
    assert!(
        second["messages"]
            .as_array()
            .expect("messages")
            .starts_with(prefix)
    );
    engine.shutdown().await;

    let mut restarted = Engine::spawn(Arc::new(loaded.reload().expect("reload")));
    let events = startup(&mut restarted).await;
    assert_eq!(folders(&events), std::slice::from_ref(skill));
    assert!(
        events
            .iter()
            .any(|e| matches!(e, Event::Skills { skills } if skills.is_empty()))
    );
    assert!(events.iter().any(|e| matches!(e, Event::Commands { commands } if commands.iter().any(|c| c.name == "git:foreign"))));

    common::turn(&mut restarted, "Start without the skill").await;
    write(
        &loaded.dirs.user_config,
        "config.toml",
        "[skills]\ncompat_paths = ['.claude/skills/']\n",
    );
    restarted.submit(Op::ReloadConfig).await;
    assert!(folders(&startup(&mut restarted).await).is_empty());
    common::turn(&mut restarted, "Use the updated index").await;
    assert!(common::bodies(&server).await[4].contains("The available skills have changed"));
    restarted.shutdown().await;
}

#[tokio::test]
async fn refuses_to_save_into_untrusted_project_config() {
    let temp = tempfile::tempdir().expect("test directory");
    let loaded = common::workspace(temp.path(), "http://127.0.0.1:1", r#"["tiny"]"#, "");
    write(
        &loaded.dirs.user_config,
        "compat.toml",
        "[commands]\nproject = ['.claude/commands/']\n",
    );
    let root = &loaded.dirs.workspace_root;
    let config = "[ui]\nascii = true\n";
    write(root, ".quack/config.toml", config);
    write(root, ".claude/commands/review.md", "Review these changes.");
    let mut engine = Engine::spawn(Arc::new(loaded.reload().expect("reload")));
    let events = startup(&mut engine).await;
    let [folder] = folders(&events) else {
        panic!("expected one folder")
    };
    assert_eq!(folder.scope, WriteScope::Project);
    assert!(!folder.can_save);
    import(&engine, folder, true).await;
    until(&mut engine, |e| matches!(e, Event::Warning { message } if message.contains("Project settings are not active"))).await;
    assert_eq!(
        std::fs::read_to_string(loaded.dirs.project.join("config.toml")).expect("project config"),
        config
    );
    engine.shutdown().await;
}

#[tokio::test]
async fn saves_home_paths_in_user_config_without_replacing_the_list() {
    let temp = tempfile::tempdir().expect("test directory");
    let initial = common::workspace(temp.path(), "http://127.0.0.1:1", r#"["tiny"]"#, "");
    let home = temp.path().join("home");
    let dirs = &initial.dirs;
    write(&home, ".claude/commands/review.md", "Review these changes.");
    write(
        &dirs.user_config,
        "compat.toml",
        "[commands]\nuser = ['~/.claude/commands/']\n",
    );
    write(
        &dirs.user_config,
        "config.toml",
        "[commands]\n# Keep this path\ncompat_paths = ['prior']\n",
    );
    let loaded = quack_config::load_with(
        &|key| {
            let path = match key {
                "HOME" | "USERPROFILE" => &home,
                "QUACK_CONFIG_DIR" => &dirs.user_config,
                "QUACK_SYSTEM_DIR" => &dirs.system,
                "QUACK_DATA_DIR" => &dirs.user_data,
                "QUACK_CACHE_DIR" => &dirs.user_cache,
                "QUACK_DEFAULTS_DIR" => Path::new("none"),
                _ => return None,
            };
            Some(path.as_os_str().to_owned())
        },
        &initial.opts,
    )
    .expect("load test config");
    let mut engine = Engine::spawn(Arc::new(loaded.clone()));
    let events = startup(&mut engine).await;
    let [folder] = folders(&events) else {
        panic!("expected one folder")
    };
    assert_eq!(folder.scope, WriteScope::User);
    import(&engine, folder, true).await;
    let events = until(
        &mut engine,
        |e| matches!(e, Event::Notice { message } if message.contains("saved its path")),
    )
    .await;
    assert!(events.iter().any(
        |e| matches!(e, Event::Commands { commands } if commands.iter().any(|c| c.name == "review"))
    ));
    let settings = loaded.reload().expect("reload config").settings;
    assert_eq!(
        settings.commands.compat_paths,
        ["prior", "~/.claude/commands/"]
    );
    let saved = std::fs::read_to_string(dirs.user_config.join("config.toml")).expect("user config");
    assert!(saved.contains("# Keep this path"));
    assert!(!dirs.project.exists());
    engine.shutdown().await;
}

#[test]
fn saving_preserves_trust_of_an_existing_text_only_project() {
    let temp = tempfile::tempdir().expect("test directory");
    let loaded = common::workspace(temp.path(), "http://127.0.0.1:1", r#"["tiny"]"#, "");
    write(
        &loaded.dirs.project,
        "commands/existing.md",
        "An existing project command.",
    );
    let loaded = loaded.reload().expect("load text-only project");
    assert_eq!(loaded.project, quack_config::ProjectStatus::Trusted);
    loaded
        .save_compat_path(
            quack_config::LayerKind::Project,
            "commands.compat_paths",
            ".claude/commands",
        )
        .expect("save project setting");
    assert_eq!(
        loaded.reload().expect("reload project").project,
        quack_config::ProjectStatus::Trusted
    );
}
