use super::keys::{file_url, split_paths};
use super::slash::last_code_block;
use super::*;

pub(crate) fn info() -> AppInfo {
    AppInfo {
        version: "0.1.0".into(),
        cwd: "~/src/project".into(),
        branch: Some("main".into()),
        model: None,
        default_model: None,
        small_model: None,
        default_locked: false,
        small_locked: false,
        profile: "empty".into(),
        permission_mode: "ask".into(),
        work_mode: WorkMode::Edit,
        ui: UiSettings::default(),
        no_color: false,
        settings: Vec::new(),
        project_config: String::new(),
        project_status: ProjectStatus::None,
        layers: Vec::new(),
        rules: Vec::new(),
        grammar_dirs: Vec::new(),
        bundles: Vec::new(),
        bundles_disabled: Vec::new(),
        hooks: Vec::new(),
        hooks_disabled: Vec::new(),
        mcp_items: Vec::new(),
        history: Vec::new(),
        history_path: None,
    }
}

#[test]
fn window_title_is_the_duck_and_the_session_title() {
    let mut app = App::new(info());
    assert_eq!(app.window_title().as_deref(), Some("_(•)> quack"));
    app.on_engine(Event::SessionTitle {
        title: Some("fix the login bug".into()),
    });
    assert_eq!(
        app.window_title().as_deref(),
        Some("_(•)> fix the login bug")
    );
    app.turn_running = true;
    assert_eq!(
        app.window_title().as_deref(),
        Some("✻ _(•)> fix the login bug")
    );
    app.on_engine(Event::SessionTitle { title: None });
    app.turn_running = false;
    assert_eq!(app.window_title().as_deref(), Some("_(•)> quack"));
    app.info.ui.terminal_title = false;
    assert_eq!(app.window_title(), None);
}

// Without a runtime (a plain `#[test]`), highlighting happens on the spot.
#[test]
fn code_blocks_are_highlighted_when_they_close() {
    if !quack_core::grammar::Grammars::compiled_in() {
        return;
    }
    let mut app = App::new(info());
    app.on_engine(Event::AssistantDelta {
        text: "Here:\n\n```rust\nfn a() {}\n".into(),
    });
    assert!(app.highlights.get("rust", "fn a() {}\n").is_none());
    app.on_engine(Event::AssistantDelta {
        text: "```\n".into(),
    });
    assert!(app.highlights.get("rust", "fn a() {}\n").is_some());
}

#[tokio::test]
async fn large_code_blocks_are_highlighted_off_thread() {
    if !quack_core::grammar::Grammars::compiled_in() {
        return;
    }
    let mut app = App::new(info());
    let mut results = app.take_highlight_results().expect("receiver");
    let code = "let x = 1;\n".repeat(2_000);
    app.on_engine(Event::AssistantDelta {
        text: format!("```rust\n{code}```\n"),
    });
    assert!(app.highlights.get("rust", &code).is_none());
    let done = results.recv().await.expect("a result");
    app.on_highlighted(done);
    assert!(app.highlights.get("rust", &code).is_some());
}

fn press(app: &mut App, code: KeyCode) {
    app.on_terminal(TermEvent::Key(KeyEvent::new(code, KeyModifiers::NONE)));
}

fn type_str(app: &mut App, s: &str) {
    for c in s.chars() {
        press(app, KeyCode::Char(c));
    }
}

#[test]
fn custom_commands_in_the_popup_and_sent_as_turns() {
    use quack_protocol::CommandInfo;
    let mut app = App::new(info());
    let cmd = |name: &str| CommandInfo {
        name: name.into(),
        description: format!("{name} things"),
        args: "[focus]".into(),
        source: "user".into(),
        path: format!("/cfg/commands/{name}.md"),
    };
    app.on_engine(Event::Commands {
        commands: vec![cmd("review"), cmd("models")],
    });
    // A built-in name wins, and the user is told.
    assert!(app.transcript.iter().any(
        |e| matches!(e, Entry::Warning(w) if w.contains("/models in /cfg/commands/models.md"))
    ));
    type_str(&mut app, "/revi");
    let popup = app.popup_matches();
    assert!(
        popup
            .iter()
            .any(|p| p.name == "review" && p.hint == Some("custom")),
        "{popup:?}"
    );
    app.input.clear();
    type_str(&mut app, "/review the cache");
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.take_ops().as_slice(),
        [Op::UserTurn { items, .. }] if items == &vec![InputItem::text("/review the cache")]
    ));
}

#[test]
fn mcp_prompts_in_the_popup_and_sent_as_turns() {
    let mut app = App::new(info());
    app.on_engine(Event::McpPrompts {
        prompts: vec![quack_protocol::CommandInfo {
            name: "gh:review".into(),
            description: "Review a pull request".into(),
            args: "<pr>".into(),
            source: "mcp: gh".into(),
            path: String::new(),
        }],
    });
    type_str(&mut app, "/gh:re");
    let popup = app.popup_matches();
    assert!(
        popup
            .iter()
            .any(|p| p.name == "gh:review" && p.hint == Some("mcp")),
        "{popup:?}"
    );
    app.input.clear();
    type_str(&mut app, "/gh:review 12");
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.take_ops().as_slice(),
        [Op::UserTurn { items, .. }] if items == &vec![InputItem::text("/gh:review 12")]
    ));
}

#[test]
fn copy_last_response_or_code_block() {
    let mut app = App::new(info());
    type_str(&mut app, "/copy");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_clipboard(), None);
    app.on_engine(Event::AssistantDelta {
        text: "Run:\n```sh\ncargo test\n```\nthen\n```\nb\n```".into(),
    });
    type_str(&mut app, "/copy code");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_clipboard().as_deref(), Some("b"));
    type_str(&mut app, "/copy");
    press(&mut app, KeyCode::Enter);
    assert!(app.take_clipboard().is_some_and(|t| t.starts_with("Run:")));
    assert_eq!(
        last_code_block("```\na\n```\n```\nb\nc\n```").as_deref(),
        Some("b\nc")
    );
}

#[test]
fn work_mode_and_approvals_change_independently() {
    let mut app = App::new(info());
    press(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.take_ops(),
        vec![Op::SetWorkMode {
            mode: WorkMode::Plan
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    app.execute("/permissions allow");
    assert_eq!(
        app.take_ops(),
        vec![Op::SetPermissionMode {
            mode: PermissionMode::Allow
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Allow,
    });
    assert_eq!(app.info.work_mode, WorkMode::Plan);
    assert_eq!(app.info.permission_mode, "allow");
    assert_eq!(
        app.review_choices()[0].1,
        "Implement plan · allow approvals"
    );
    app.execute("/plan accept");
    assert_eq!(
        app.take_ops(),
        vec![Op::AcceptPlan {
            mode: String::new(),
            clear_context: false,
        }]
    );
    app.execute("/plan accept fresh");
    assert_eq!(
        app.take_ops(),
        vec![Op::AcceptPlan {
            mode: String::new(),
            clear_context: true,
        }]
    );
    press(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.take_ops(),
        vec![Op::SetWorkMode {
            mode: WorkMode::Edit
        }]
    );
}

#[test]
fn plan_mode_note_is_shown_only_once_when_cycling_modes() {
    let mut app = App::new(info());
    let plan_notes = |app: &App| {
        app.transcript
            .iter()
            .filter(|e| matches!(e, Entry::System(s) if s.starts_with("plan mode:")))
            .count()
    };

    // First switch to plan mode explains what it does.
    press(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.take_ops(),
        vec![Op::SetWorkMode {
            mode: WorkMode::Plan
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    assert_eq!(plan_notes(&app), 1);

    // Cycle back to edit, then back to plan: the explanation is not repeated.
    press(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.take_ops(),
        vec![Op::SetWorkMode {
            mode: WorkMode::Edit
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Edit,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    assert_eq!(plan_notes(&app), 1);

    press(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.take_ops(),
        vec![Op::SetWorkMode {
            mode: WorkMode::Plan
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    assert_eq!(app.info.work_mode, WorkMode::Plan);
    assert_eq!(plan_notes(&app), 1);
}

fn plan_turn_ends(app: &mut App) {
    use quack_protocol::{TurnId, Usage};
    app.on_engine(Event::TurnComplete {
        turn: TurnId(1),
        usage: Usage {
            prompt_tokens: 1,
            completion_tokens: 1,
            cached_prompt_tokens: None,
            cache_write_tokens: None,
            reasoning_tokens: None,
        },
    });
}

#[test]
fn plan_review_bar_accepts_or_takes_a_revision() {
    let mut app = App::new(info());
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Allow,
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, Some(0));
    assert_eq!(app.review_choices().len(), 3);
    // The third choice keeps planning without changing approvals.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert!(app.take_ops().is_empty());
    assert_eq!(app.info.permission_mode, "allow");
    plan_turn_ends(&mut app);
    type_str(&mut app, "use sqlite instead");
    press(&mut app, KeyCode::Enter);
    assert!(
        matches!(app.take_ops().as_slice(), [Op::UserTurn { items, .. }] if items == &[InputItem::text("use sqlite instead")])
    );
    assert_eq!(app.review, None);
    plan_turn_ends(&mut app);
    press(&mut app, KeyCode::Esc);
    assert!(app.take_ops().is_empty());
    plan_turn_ends(&mut app);
    press(&mut app, KeyCode::Char('1'));
    assert_eq!(
        app.take_ops(),
        vec![Op::AcceptPlan {
            mode: String::new(),
            clear_context: false,
        }]
    );
    plan_turn_ends(&mut app);
    press(&mut app, KeyCode::Char('2'));
    assert_eq!(
        app.take_ops(),
        vec![Op::AcceptPlan {
            mode: String::new(),
            clear_context: true,
        }]
    );
}

#[test]
fn finished_or_executed_plan_does_not_open_review_bar_unless_followup_created() {
    use quack_protocol::TurnId;
    let mut app = App::new(info());
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    // Initial planning turn creates pending steps -> review bar opens.
    app.on_engine(Event::TurnStarted { turn: TurnId(1) });
    app.on_engine(Event::Plan {
        checklist: "[ ] step 1\n[ ] step 2".into(),
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, Some(0));

    // Accept the plan and carry it out in Edit mode until every step is done.
    press(&mut app, KeyCode::Enter);
    let _ = app.take_ops();
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Edit,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    app.on_engine(Event::TurnStarted { turn: TurnId(2) });
    app.on_engine(Event::Plan {
        checklist: "[x] step 1\n[x] step 2".into(),
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, None);

    // Returning to Plan mode and checking in when the plan is finished does not reopen review.
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    assert_eq!(app.review, None);
    app.on_engine(Event::TurnStarted { turn: TurnId(3) });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, None);

    // Explicitly clearing the checklist during a planning turn also does not open review.
    app.on_engine(Event::TurnStarted { turn: TurnId(4) });
    app.on_engine(Event::Plan {
        checklist: String::new(),
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, None);

    // Creating a follow-up plan in Plan mode opens the review bar for the new steps.
    app.on_engine(Event::TurnStarted { turn: TurnId(5) });
    app.on_engine(Event::Plan {
        checklist: "[ ] add caching\n[ ] add regression test".into(),
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, Some(0));
}

#[test]
fn stale_plan_edit_turn_all_done_plan_and_compact_do_not_open_review_bar() {
    use quack_protocol::TurnId;
    let mut app = App::new(info());
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Allow,
    });
    app.on_engine(Event::TurnStarted { turn: TurnId(1) });
    app.on_engine(Event::Plan {
        checklist: "[ ] edit parser\n[ ] run tests".into(),
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, Some(0));

    // Accept the plan; while the Edit turn runs, the user switches back to Plan mode early,
    // and the model finishes without ticking the last step.
    press(&mut app, KeyCode::Enter);
    let _ = app.take_ops();
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Edit,
        approvals: quack_protocol::ApprovalPolicy::Allow,
    });
    app.on_engine(Event::TurnStarted { turn: TurnId(2) });
    app.on_engine(Event::Plan {
        checklist: "[x] edit parser\n[~] run tests".into(),
    });
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Allow,
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, None, "finishing Edit turn must not open review");

    // A follow-up check in Plan mode that leaves the stale checklist untouched still does not reopen it.
    app.on_engine(Event::TurnStarted { turn: TurnId(3) });
    plan_turn_ends(&mut app);
    assert_eq!(
        app.review, None,
        "stale pre-execution checklist must not reopen review"
    );

    // A Plan turn whose steps are all completed (read-only audit) does not open review.
    app.on_engine(Event::TurnStarted { turn: TurnId(4) });
    app.on_engine(Event::Plan {
        checklist: "[x] inspect auth\n[x] summarize findings".into(),
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, None);

    // Manual /compact in Plan mode with a pending plan does not open review.
    app.on_engine(Event::TurnStarted { turn: TurnId(5) });
    app.on_engine(Event::Plan {
        checklist: "[ ] follow-up step".into(),
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, Some(0));
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.review, None);

    app.on_engine(Event::TurnStarted { turn: TurnId(6) });
    app.on_engine(Event::Compacting {
        reason: "/compact".into(),
    });
    plan_turn_ends(&mut app);
    assert_eq!(app.review, None, "/compact must not open plan review");
}

#[test]
fn step_limit_bar_continues_stops_or_takes_a_redirect() {
    use quack_protocol::TurnId;
    let ask = |app: &mut App| {
        app.on_engine(Event::StepLimitReached {
            turn: TurnId(1),
            limit: 100,
        });
    };
    let mut app = App::new(info());
    app.on_engine(Event::TurnStarted { turn: TurnId(1) });
    // Enter confirms the highlighted choice: keep going.
    ask(&mut app);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::ContinueTurn {
            turn: TurnId(1),
            proceed: true
        }]
    );
    // Esc stops the turn at the limit.
    ask(&mut app);
    press(&mut app, KeyCode::Esc);
    assert_eq!(
        app.take_ops(),
        vec![Op::ContinueTurn {
            turn: TurnId(1),
            proceed: false
        }]
    );
    // A typed redirect stops the turn and sends the message after it.
    ask(&mut app);
    type_str(&mut app, "use sqlite instead");
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.take_ops().as_slice(),
        [Op::ContinueTurn { proceed: false, .. }, Op::UserTurn { items, .. }]
            if items == &[InputItem::text("use sqlite instead")]
    ));
    assert_eq!(app.step_limit, None);
    // The turn ending clears the bar.
    ask(&mut app);
    app.on_engine(Event::TurnAborted {
        turn: TurnId(1),
        reason: "interrupted".into(),
    });
    assert_eq!(app.step_limit, None);
    assert!(app.take_ops().is_empty());
}

#[test]
fn questions_are_picked_typed_or_skipped() {
    use quack_protocol::QuestionOption;
    let mut app = App::new(info());
    let questions = vec![
        Question {
            header: "Storage".into(),
            question: "Where should tokens live?".into(),
            options: vec![
                QuestionOption {
                    label: "OS keyring".into(),
                    description: None,
                },
                QuestionOption {
                    label: "A dotfile".into(),
                    description: None,
                },
            ],
            recommended: Some(1),
        },
        Question {
            header: String::new(),
            question: "Which database?".into(),
            options: Vec::new(),
            recommended: None,
        },
    ];
    let ask = |app: &mut App| {
        app.on_engine(Event::Questions {
            call_id: CallId("q1".into()),
            questions: questions.clone(),
        });
    };

    ask(&mut app);
    assert_eq!(
        app.asking.as_ref().map(|a| a.selected),
        Some(1),
        "the recommendation"
    );
    press(&mut app, KeyCode::Char('1'));
    type_str(&mut app, "sqlite");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::AnswerQuestions {
            call_id: CallId("q1".into()),
            answers: vec![
                Answer::Picked { index: 0 },
                Answer::Text {
                    text: "sqlite".into()
                }
            ],
            stop_asking: false,
        }]
    );
    assert!(app.asking.is_none());
    assert!(app.input.is_empty());

    // Enter takes the recommendation; Ctrl-S skips the rest and stops the asking.
    ask(&mut app);
    press(&mut app, KeyCode::Enter);
    app.on_terminal(TermEvent::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(
        app.take_ops(),
        vec![Op::AnswerQuestions {
            call_id: CallId("q1".into()),
            answers: vec![Answer::Picked { index: 1 }, Answer::Skipped],
            stop_asking: true,
        }]
    );

    // Esc skips one question.
    ask(&mut app);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.asking.as_ref().map(|a| a.current), Some(1));
}

#[test]
fn yolo_toggles_back_to_the_previous_mode() {
    let mut app = App::new(info());
    let mode = |m: &str| Op::SetPermissionMode {
        mode: m.parse().expect("mode"),
    };
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Edit,
        approvals: quack_protocol::ApprovalPolicy::AutoEdit,
    });
    type_str(&mut app, "/yolo");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![mode("yolo")]);
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Edit,
        approvals: quack_protocol::ApprovalPolicy::Yolo,
    });
    type_str(&mut app, "/yolo");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![mode("auto-edit")]);
    type_str(&mut app, "/yolo off");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![mode("auto-edit")]);
}

#[test]
fn sandbox_toggles_without_changing_the_mode() {
    let mut app = App::new(info());
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    type_str(&mut app, "/sandbox");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::SetSandbox { off: true }]);
    app.on_engine(Event::Sandbox { off: true });
    type_str(&mut app, "/sandbox");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::SetSandbox { off: false }]);
    type_str(&mut app, "/sandbox off");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::SetSandbox { off: true }]);
    assert_eq!(app.info.work_mode, WorkMode::Plan);
}

#[test]
fn trust_prompt_opens_and_sends_the_decision() {
    let mut app = App::new(info());
    app.on_engine(Event::ProjectTrust {
        path: "/r/.quack".into(),
        summary: "1 MCP server".into(),
        changed: false,
    });
    assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Trust"));
    press(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert_eq!(
        app.take_ops(),
        vec![Op::TrustProject {
            decision: "trusted".into()
        }]
    );
    type_str(&mut app, "/reload");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::ReloadConfig]);
}

#[test]
fn compat_review_loads_only_after_enter_or_save() {
    use quack_protocol::{CompatFolder, CompatKind};

    let mut app = App::new(info());
    let path = "/r/.claude/commands";
    app.on_engine(Event::CompatFolders {
        folders: vec![CompatFolder {
            kind: CompatKind::Commands,
            path: path.into(),
            scope: WriteScope::Project,
            can_save: true,
        }],
    });
    assert!(app.modal.is_none());
    assert!(app.take_ops().is_empty());
    for save in [false, true] {
        type_str(&mut app, "/compat");
        press(&mut app, KeyCode::Enter);
        assert!(app.modal.is_some());
        assert!(app.take_ops().is_empty());
        if save {
            app.on_terminal(TermEvent::Key(KeyEvent::new(
                KeyCode::Char('s'),
                KeyModifiers::CONTROL,
            )));
        } else {
            press(&mut app, KeyCode::Enter);
        }
        assert_eq!(
            app.take_ops(),
            vec![Op::ImportCompat {
                kind: CompatKind::Commands,
                path: path.into(),
                save
            }]
        );
    }
}

#[test]
fn double_esc_opens_rewind_and_rewound_cuts_the_transcript() {
    let mut app = App::new(info());
    for text in ["one", "two", "three"] {
        app.on_engine(Event::AssistantDelta { text: "x".into() });
        app.transcript.push(Entry::User(text.into()));
    }
    press(&mut app, KeyCode::Esc);
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Rewind"));
    assert_eq!(app.take_ops(), vec![Op::ListRewindPoints]);
    app.modal = None;
    app.on_engine(Event::Rewound {
        back: 2,
        text: "two".into(),
    });
    assert!(!app.transcript.contains(&Entry::User("two".into())));
    assert!(app.transcript.contains(&Entry::User("one".into())));
    assert_eq!(app.input.text(), "two");
    type_str(&mut app, "");
    app.input.clear();
    type_str(&mut app, "/rewind 1 files --force");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::Rewind {
            back: 1,
            conversation: false,
            files: true,
            force: true
        }]
    );
}

#[test]
fn release_events_are_ignored() {
    let mut app = App::new(info());
    let mut ev = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    ev.kind = KeyEventKind::Release;
    app.on_terminal(TermEvent::Key(ev));
    assert!(app.input.is_empty());
}

#[test]
fn slash_popup_runs_help_and_quit() {
    let mut app = App::new(info());
    type_str(&mut app, "/hel");
    assert_eq!(
        app.popup_matches().first().map(|c| c.name.as_str()),
        Some("help")
    );
    press(&mut app, KeyCode::Enter);
    assert!(app.modal.is_some());
    press(&mut app, KeyCode::Esc);
    assert!(app.modal.is_none());
    type_str(&mut app, "/exit");
    assert_eq!(
        app.popup_matches().first().map(|c| c.name.as_str()),
        Some("quit")
    );
    press(&mut app, KeyCode::Enter);
    assert!(app.should_quit());
}

#[test]
fn messages_become_user_turns_and_stream_back() {
    let mut app = App::new(info());
    type_str(&mut app, "hello");
    press(&mut app, KeyCode::Enter);
    assert!(matches!(app.take_ops().as_slice(), [Op::UserTurn { .. }]));
    app.on_engine(Event::TurnStarted { turn: TurnId(1) });
    app.on_engine(Event::AssistantDelta { text: "hi ".into() });
    app.on_engine(Event::AssistantDelta {
        text: "there".into(),
    });
    assert_eq!(
        app.transcript.last(),
        Some(&Entry::Assistant("hi there".into()))
    );
    press(&mut app, KeyCode::Esc);
    assert_eq!(app.take_ops(), vec![Op::Interrupt]);
}

#[test]
fn providers_modal_edits_connection_settings_in_place() {
    use quack_protocol::{KeySource, ProviderConnection, ProviderHttpVersion, ProviderModels};
    let mut app = App::new(info());
    app.execute("/providers");
    assert_eq!(
        app.take_ops(),
        vec![Op::ListModels, Op::ListProviderConnections]
    );
    // Connections arrive before network probing finishes: both providers are immediately
    // visible and editable in the single list without switching tabs.
    app.on_engine(Event::ProviderConnections {
        connections: vec![
            ProviderConnection {
                id: "openai".into(),
                http_version: ProviderHttpVersion::Inherit,
                no_proxy: false,
                inherited_http1_only: false,
            },
            ProviderConnection {
                id: "vllm".into(),
                http_version: ProviderHttpVersion::Inherit,
                no_proxy: false,
                inherited_http1_only: false,
            },
        ],
    });
    // Select vllm while catalog is still loading; when catalog arrives in a different order,
    // vllm stays selected.
    press(&mut app, KeyCode::Down);
    assert_eq!(
        app.modal
            .as_ref()
            .and_then(|m| m.selected_item())
            .map(|i| i.label.as_str()),
        Some("vllm")
    );
    app.on_engine(Event::ModelCatalog {
        providers: vec![
            ProviderModels {
                id: "vllm".into(),
                available: false,
                note: "connection refused".into(),
                models: Vec::new(),
                bundled: false,
                key: None,
            },
            ProviderModels {
                id: "openai".into(),
                available: true,
                note: "1 model".into(),
                models: Vec::new(),
                bundled: false,
                key: Some(KeySource::Keyring {
                    name: "openai".into(),
                }),
            },
        ],
        current: None,
    });
    assert_eq!(
        app.modal
            .as_ref()
            .and_then(|m| m.selected_item())
            .map(|i| i.label.as_str()),
        Some("vllm")
    );
    // Keyless provider: Enter opens its single-list connection editor directly.
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.modal.as_ref().map(|m| m.title.as_str()),
        Some("Connection: vllm")
    );
    // Down 4 times reaches "Proxy: Direct connection" on the same screen.
    for _ in 0..4 {
        press(&mut app, KeyCode::Down);
    }
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveProviderConnection {
            id: "vllm".into(),
            http_version: None,
            no_proxy: Some(true),
        }]
    );
    // Back on the single Providers list with vllm still selected.
    assert_eq!(
        app.modal.as_ref().map(|m| m.title.as_str()),
        Some("Providers")
    );
    assert_eq!(
        app.modal
            .as_ref()
            .and_then(|m| m.selected_item())
            .map(|i| i.label.as_str()),
        Some("vllm")
    );

    // Keyed provider: Ctrl-S opens its connection editor, and Esc returns to Providers on it.
    press(&mut app, KeyCode::Down);
    assert_eq!(
        app.modal
            .as_ref()
            .and_then(|m| m.selected_item())
            .map(|i| i.label.as_str()),
        Some("openai")
    );
    app.on_terminal(TermEvent::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(
        app.modal.as_ref().map(|m| m.title.as_str()),
        Some("Connection: openai")
    );
    press(&mut app, KeyCode::Esc);
    assert_eq!(
        app.modal.as_ref().map(|m| m.title.as_str()),
        Some("Providers")
    );
    assert_eq!(
        app.modal
            .as_ref()
            .and_then(|m| m.selected_item())
            .map(|i| i.label.as_str()),
        Some("openai")
    );
}

#[test]
fn models_modal_lists_catalog_and_switches() {
    use quack_protocol::{ModelInfo, ProviderModels};
    let mut app = App::new(info());
    type_str(&mut app, "/models");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::ListModels]);
    app.on_engine(Event::ModelCatalog {
        providers: vec![
            ProviderModels {
                bundled: false,
                key: None,
                id: "charm".into(),
                available: true,
                note: "2 models".into(),
                models: vec![
                    ModelInfo {
                        id: "glm-5.3".into(),
                        alias: None,
                        context_window: Some(1_000_000),
                        price: None,
                        reasoning_efforts: Vec::new(),
                    },
                    ModelInfo {
                        id: "qwen3.8-flash".into(),
                        alias: None,
                        context_window: None,
                        price: None,
                        reasoning_efforts: Vec::new(),
                    },
                ],
            },
            ProviderModels {
                bundled: false,
                key: None,
                id: "novita".into(),
                available: false,
                note: "set NOVITA_API_KEY to use this provider".into(),
                models: Vec::new(),
            },
        ],
        current: Some("charm/glm-5.3".into()),
    });
    type_str(&mut app, "qwen");
    press(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert_eq!(
        app.take_ops(),
        vec![Op::SetModel {
            provider: ProviderId("charm".into()),
            model: ModelId("qwen3.8-flash".into()),
        }]
    );
}

#[test]
fn selecting_a_reasoning_model_opens_the_thinking_picker() {
    use quack_protocol::{ModelInfo, ProviderModels, SessionId, SessionInfo};
    let mut app = App::new(info());
    type_str(&mut app, "/models");
    press(&mut app, KeyCode::Enter);
    app.take_ops();
    app.on_engine(Event::ModelCatalog {
        providers: vec![ProviderModels {
            bundled: false,
            key: None,
            id: "charm".into(),
            available: true,
            note: "1 model".into(),
            models: vec![ModelInfo {
                id: "glm-5.3".into(),
                alias: None,
                context_window: None,
                price: None,
                reasoning_efforts: vec!["low".into(), "medium".into(), "high".into()],
            }],
        }],
        current: None,
    });
    type_str(&mut app, "glm");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SetModel {
            provider: ProviderId("charm".into()),
            model: ModelId("glm-5.3".into()),
        }]
    );
    // No picker before the engine confirms the switch, and none for a model without
    // levels.
    assert!(app.modal.is_none());
    app.on_engine(Event::SessionConfigured {
        session: Box::new(SessionInfo {
            id: SessionId("test".into()),
            cwd: String::new(),
            model: Some("charm/glm-5.3".into()),
            profile: "generic".into(),
            reasoning_effort: Some("medium".into()),
            reasoning_efforts: vec!["low".into(), "medium".into(), "high".into()],
        }),
    });
    let thinking = app.modal.as_ref().expect("picker open");
    assert_eq!(thinking.title, "Thinking");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SetReasoningEffort {
            effort: Some("medium".into())
        }]
    );
    // The picker does not reopen on its own afterwards.
    app.on_engine(Event::SessionConfigured {
        session: Box::new(SessionInfo {
            id: SessionId("test".into()),
            cwd: String::new(),
            model: Some("charm/glm-5.3".into()),
            profile: "generic".into(),
            reasoning_effort: Some("medium".into()),
            reasoning_efforts: vec!["low".into(), "medium".into(), "high".into()],
        }),
    });
    assert!(app.modal.is_none());
}

#[test]
fn models_modal_ctrl_s_saves_default() {
    use quack_protocol::{ModelInfo, ProviderModels};
    let mut app = App::new(info());
    type_str(&mut app, "/models");
    press(&mut app, KeyCode::Enter);
    app.take_ops();
    app.on_engine(Event::ModelCatalog {
        providers: vec![ProviderModels {
            bundled: false,
            key: None,
            id: "charm".into(),
            available: true,
            note: String::new(),
            models: vec![
                ModelInfo {
                    id: "true".into(),
                    alias: None,
                    context_window: None,
                    price: None,
                    reasoning_efforts: Vec::new(),
                },
                ModelInfo {
                    id: "flash".into(),
                    alias: None,
                    context_window: None,
                    price: None,
                    reasoning_efforts: Vec::new(),
                },
            ],
        }],
        current: None,
    });
    // Ctrl-S opens the default role picker; Enter on the first item saves Large.
    app.on_terminal(TermEvent::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(
        app.modal.as_ref().map(|m| m.title.as_str()),
        Some("Default role")
    );
    press(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveSetting {
            key: "model.default".into(),
            value: "\"charm/true\"".into(),
            scope: WriteScope::User,
        }]
    );
    app.on_engine(Event::SettingSaved {
        key: "model.default".into(),
        value: "\"charm/true\"".into(),
        path: "/u/config.toml".into(),
        note: None,
    });
    assert_eq!(app.info.default_model.as_deref(), Some("charm/true"));
    assert!(matches!(app.transcript.last(), Some(Entry::System(t)) if t.contains("next start")));

    // Selecting Small (second item) saves model.small and applies immediately.
    type_str(&mut app, "/models");
    press(&mut app, KeyCode::Enter);
    app.take_ops();
    app.on_engine(Event::ModelCatalog {
        providers: vec![ProviderModels {
            bundled: false,
            key: None,
            id: "charm".into(),
            available: true,
            note: String::new(),
            models: vec![
                ModelInfo {
                    id: "true".into(),
                    alias: None,
                    context_window: None,
                    price: None,
                    reasoning_efforts: Vec::new(),
                },
                ModelInfo {
                    id: "flash".into(),
                    alias: None,
                    context_window: None,
                    price: None,
                    reasoning_efforts: Vec::new(),
                },
            ],
        }],
        current: None,
    });
    press(&mut app, KeyCode::Down);
    app.on_terminal(TermEvent::Key(KeyEvent::new(
        KeyCode::Char('s'),
        KeyModifiers::CONTROL,
    )));
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveSetting {
            key: "model.small".into(),
            value: "\"charm/flash\"".into(),
            scope: WriteScope::User,
        }]
    );
    app.on_engine(Event::SettingSaved {
        key: "model.small".into(),
        value: "\"charm/flash\"".into(),
        path: "/u/config.toml".into(),
        note: None,
    });
    assert_eq!(app.info.small_model.as_deref(), Some("charm/flash"));
    assert!(matches!(app.transcript.last(), Some(Entry::System(t)) if !t.contains("next start")));

    // Selecting Both saves both model.default and model.small.
    type_str(&mut app, "/models default both charm/flash");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![
            Op::SaveSetting {
                key: "model.default".into(),
                value: "\"charm/flash\"".into(),
                scope: WriteScope::User,
            },
            Op::SaveSetting {
                key: "model.small".into(),
                value: "\"charm/flash\"".into(),
                scope: WriteScope::User,
            },
        ]
    );
}

#[test]
fn resume_picker_switches_and_replaces_transcript() {
    use quack_protocol::{SessionInfo, SessionSummary};
    let mut app = App::new(info());
    app.transcript.push(Entry::User("old".into()));
    type_str(&mut app, "/resume");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::ListSessions]);
    let summary = |id: &str, title: &str, current| SessionSummary {
        id: SessionId(id.into()),
        title: title.into(),
        model: "charm/x".into(),
        turns: 2,
        age: "1h ago".into(),
        started: "2026-10-17".into(),
        current,
    };
    app.on_engine(Event::SessionList {
        sessions: vec![
            summary("aaa", "now", true),
            summary("bbb", "fix the parser", false),
        ],
    });
    type_str(&mut app, "parser");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::ResumeSession {
            id: SessionId("bbb".into())
        }]
    );
    app.on_engine(Event::SessionResumed {
        session: Box::new(SessionInfo {
            id: SessionId("bbb".into()),
            cwd: String::new(),
            model: None,
            profile: "generic".into(),
            reasoning_effort: None,
            reasoning_efforts: Vec::new(),
        }),
        items: vec![ReplayItem::User {
            text: "fix it".into(),
        }]
        .into(),
    });
    assert!(!app.transcript.contains(&Entry::User("old".into())));
    assert!(app.transcript.contains(&Entry::User("fix it".into())));
}

#[test]
fn session_alias_resumes_or_lists() {
    let mut app = App::new(info());
    type_str(&mut app, "/session");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::ListSessions]);
    assert!(app.modal.is_some());

    let mut app = App::new(info());
    type_str(&mut app, "/session s-123");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::ResumeSession {
            id: SessionId("s-123".into())
        }]
    );
    assert!(app.resuming);
}

#[test]
fn compact_aliases_summarize_and_summary() {
    for alias in ["/summarize", "/summary"] {
        let mut app = App::new(info());
        type_str(&mut app, alias);
        assert_eq!(
            app.popup_matches().first().map(|c| c.name.as_str()),
            Some("compact")
        );
        press(&mut app, KeyCode::Enter);
        assert_eq!(app.take_ops(), vec![Op::Compact { focus: None }]);

        let mut app = App::new(info());
        type_str(&mut app, &format!("{alias} keep the test plan"));
        press(&mut app, KeyCode::Enter);
        assert_eq!(
            app.take_ops(),
            vec![Op::Compact {
                focus: Some("keep the test plan".into()),
            }]
        );
    }

    // Typing a prefix of the alias surfaces `/compact` in the popup.
    let mut app = App::new(info());
    type_str(&mut app, "/summ");
    assert_eq!(
        app.popup_matches().first().map(|c| c.name.as_str()),
        Some("compact")
    );
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.input.text(), "/compact ");

    // Custom commands cannot reuse the aliases.
    let mut app = App::new(info());
    app.on_engine(Event::Commands {
        commands: vec![quack_protocol::CommandInfo {
            name: "summarize".into(),
            description: "custom".into(),
            args: String::new(),
            source: "user".into(),
            path: "/cfg/commands/summarize.md".into(),
        }],
    });
    assert!(app.custom.is_empty());
    assert!(app.transcript.iter().any(
        |e| matches!(e, Entry::Warning(w) if w.contains("/summarize in /cfg/commands/summarize.md"))
    ));
}

#[test]
fn config_set_saves_to_user_layer_and_applies_ui_live() {
    let mut app = App::new(info());
    type_str(&mut app, "/config set ui.ascii true");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveSetting {
            key: "ui.ascii".into(),
            value: "true".into(),
            scope: WriteScope::User,
        }]
    );
    app.on_engine(Event::SettingSaved {
        key: "ui.ascii".into(),
        value: "true".into(),
        path: "/u/config.toml".into(),
        note: None,
    });
    assert!(app.info.ui.ascii);
}

#[test]
fn double_ctrl_c_quits() {
    let mut app = App::new(info());
    let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    app.on_terminal(TermEvent::Key(ctrl_c));
    assert!(!app.should_quit());
    app.on_terminal(TermEvent::Key(ctrl_c));
    assert!(app.should_quit());
}

#[test]
fn dropped_paths_split_like_a_shell() {
    assert_eq!(split_paths("a.png  b.png"), ["a.png", "b.png"]);
    assert_eq!(
        split_paths("'/tmp/my shot.png' \"x y.png\""),
        ["/tmp/my shot.png", "x y.png"]
    );
    if !cfg!(windows) {
        assert_eq!(split_paths("/tmp/my\\ shot.png"), ["/tmp/my shot.png"]);
    }
    assert_eq!(file_url("file:///tmp/my%20shot.png"), "/tmp/my shot.png");
    assert_eq!(file_url("plain%20text"), "plain%20text");
}

#[test]
fn a_paste_of_image_paths_attaches_them() {
    const PNG: &[u8] = &[
        0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 0x0D, b'I', b'H', b'D', b'R', 0,
        0, 0, 2, 0, 0, 0, 1, 8, 2, 0, 0, 0, 0x7D, 0x0C, 0xC0, 0xFA,
    ];
    let spaced = std::env::temp_dir().join(format!("quack-tui-{} shot.png", std::process::id()));
    std::fs::write(&spaced, PNG).expect("png");
    let shown = spaced.display().to_string();
    let mut app = App::new(info());
    // Quoted, the way terminals send a dropped file with a space in its name.
    assert!(app.attach_pasted(&format!("'{shown}'")));
    // Unquoted with a bare space, as one path.
    assert!(app.attach_pasted(&shown));
    assert_eq!(app.attachments.len(), 2);
    assert_eq!(app.attachments[0].path, shown);
    assert_eq!(app.attachments[0].width, Some(2));
    // Text that merely mentions the image is pasted as text.
    assert!(!app.attach_pasted(&format!("look at {shown}")));
    assert_eq!(app.attachments.len(), 2);
    std::fs::remove_file(&spaced).ok();
}

#[test]
fn at_file_popup_filters_and_completes_on_tab_or_enter() {
    let mut app = App::new(info());
    app.set_files(vec![
        "Cargo.toml".into(),
        "src/lib.rs".into(),
        "src/main.rs".into(),
    ]);
    type_str(&mut app, "inspect @ma");
    let popup = app.popup_matches();
    assert_eq!(popup.len(), 1);
    assert_eq!(popup[0].prefix, '@');
    assert_eq!(popup[0].name, "src/main.rs");

    // Enter completes the `@` mention without submitting the turn.
    press(&mut app, KeyCode::Enter);
    assert!(app.take_ops().is_empty());
    assert_eq!(app.input.text(), "inspect @src/main.rs ");
    assert!(app.popup_matches().is_empty());

    // Tab completes a second `@` mention in the same prompt.
    type_str(&mut app, "and @lib");
    press(&mut app, KeyCode::Tab);
    assert_eq!(app.input.text(), "inspect @src/main.rs and @src/lib.rs ");
}

#[test]
fn persistent_history_saves_and_reloads_across_sessions() {
    let path = std::env::temp_dir().join(format!("quack-tui-history-{}.jsonl", std::process::id()));
    std::fs::remove_file(&path).ok();
    let mut i1 = info();
    i1.history_path = Some(path.clone());
    let mut app1 = App::new(i1);
    type_str(&mut app1, "fix the parser");
    press(&mut app1, KeyCode::Enter);
    assert_eq!(app1.take_ops().len(), 1);

    let mut i2 = info();
    i2.history = quack_core::load_history(&path);
    i2.history_path = Some(path.clone());
    let mut app2 = App::new(i2);
    press(&mut app2, KeyCode::Up);
    assert_eq!(app2.input.text(), "fix the parser");
    std::fs::remove_file(&path).ok();
}

#[test]
fn theme_keys_and_hooks_commands_open_modals_and_save() {
    let mut app = App::new(info());
    type_str(&mut app, "/keys");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Help"));
    press(&mut app, KeyCode::Esc);

    type_str(&mut app, "/theme light");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveSetting {
            key: "ui.theme".into(),
            value: "\"light\"".into(),
            scope: WriteScope::User,
        }]
    );

    app.on_engine(Event::HookRan {
        id: "lint".into(),
        on: "post_tool".into(),
        outcome: HookOutcome::Ok,
        ms: 12,
        message: None,
        context_tokens: 0,
    });
    assert_eq!(app.hook_runs.len(), 1);
    type_str(&mut app, "/hooks");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Hooks"));
}

#[test]
fn hook_payload_choice_can_keep_replace_or_cancel() {
    for (key, replace_payload) in [
        (KeyCode::Enter, Some(false)),
        (KeyCode::Down, Some(true)),
        (KeyCode::Esc, None),
    ] {
        let mut app = App::new(info());
        app.on_engine(Event::HookPayloadChoice {
            id: "jira".into(),
            path: "user/hooks.d/jira".into(),
        });
        press(&mut app, key);
        if key == KeyCode::Down {
            press(&mut app, KeyCode::Enter);
        }
        let expected = replace_payload.map(|replace| Op::EnableHook {
            id: "jira".into(),
            replace_payload: Some(replace),
        });
        assert_eq!(app.take_ops(), expected.into_iter().collect::<Vec<_>>());
    }
}

#[test]
fn hook_setup_uses_engine_ops_and_keeps_pasted_secrets_out_of_the_composer() {
    let mut app = App::new(info());
    type_str(&mut app, "/hooks configure forge-review");
    press(&mut app, KeyCode::Enter);
    assert!(matches!(
        app.take_ops().as_slice(),
        [Op::InspectHookSetup { request: 1, .. }]
    ));
    app.on_engine(Event::HookSetup {
        request: 1,
        info: Box::new(crate::hook_form::example()),
    });
    assert!(app.hook_form.is_some());
    press(&mut app, KeyCode::Enter);
    for _ in 0..3 {
        press(&mut app, KeyCode::Tab);
    }
    press(&mut app, KeyCode::Enter);
    app.on_terminal(TermEvent::Paste("private-test-token".into()));
    let mut released = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    released.kind = KeyEventKind::Release;
    app.on_terminal(TermEvent::Key(released));
    assert!(app.input.is_empty());
    press(&mut app, KeyCode::Enter);
    app.on_terminal(TermEvent::Key(KeyEvent::new(
        KeyCode::Char('e'),
        KeyModifiers::CONTROL,
    )));
    let ops = app.take_ops();
    assert!(!format!("{ops:?}").contains("private-test-token"));
    assert!(
        matches!(ops.as_slice(),[Op::SaveHookSetup{enable:Some(true),connections,..}] if matches!(connections[0].credentials.get("token"),Some(quack_protocol::HookCredentialChange::Replace{value}) if value.0=="private-test-token"))
    );
    app.on_engine(Event::HookSetupFailed {
        request: 999,
        message: "wrong response".into(),
    });
    assert!(
        !app.hook_form
            .as_ref()
            .expect("form")
            .status
            .contains("wrong response")
    );
    app.on_engine(Event::HookSetupFailed {
        request: 2,
        message: "Retry after unlocking the store.".into(),
    });
    assert!(
        app.hook_form
            .as_ref()
            .expect("form")
            .status
            .contains("unlocking")
    );
}

#[test]
fn toggling_a_hook_off_lists_it_and_on_goes_through_the_engine() {
    let hook = |enabled: bool| quack_config::Item {
        id: "jira".into(),
        path: "/tmp/defaults/hooks.d/jira.toml".into(),
        layer: quack_config::LayerKind::Defaults,
        state: if enabled {
            quack_config::ItemState::Loaded(quack_config::hooks::HookFile {
                id: None,
                enabled: true,
                description: String::new(),
                event: quack_config::hooks::HookEvent::UserPrompt,
                matcher: Default::default(),
                command: vec!["x".into()],
                env: Default::default(),
                setup: None,
                connections: None,
                inherit_env: None,
                cwd: String::new(),
                timeout_secs: 10,
                full_context: false,
                on_error: Default::default(),
            })
        } else {
            quack_config::ItemState::Disabled("enabled = false".into())
        },
    };

    // Shipped with `enabled = false`: the engine enables it, it does not merely
    // rewrite `[hooks] disabled`.
    let mut i = info();
    i.hooks = vec![hook(false)];
    let mut app = App::new(i);
    type_str(&mut app, "/hooks toggle jira");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::EnableHook {
            id: "jira".into(),
            replace_payload: None
        }]
    );

    // An enabled hook is turned off by listing it.
    let mut i = info();
    i.hooks = vec![hook(true)];
    let mut app = App::new(i);
    type_str(&mut app, "/hooks toggle jira");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![
            Op::SaveSetting {
                key: "hooks.disabled".into(),
                value: "[\"jira\"]".into(),
                scope: WriteScope::User,
            },
            Op::ReloadConfig,
        ]
    );
}

#[test]
fn resume_id_only_reported_for_saved_sessions() {
    let session = |id: &str| quack_protocol::SessionInfo {
        id: SessionId(id.into()),
        cwd: "/tmp".into(),
        model: Some("mock/m".into()),
        profile: "generic".into(),
        reasoning_effort: None,
        reasoning_efforts: vec![],
    };

    let mut app = App::new(info());
    app.on_engine(Event::SessionConfigured {
        session: Box::new(session("0123456789ab")),
    });
    // No turn yet -> not saved on disk.
    assert_eq!(app.resume_id(), None);

    // First turn starts -> saved.
    app.on_engine(Event::TurnStarted { turn: TurnId(1) });
    app.on_engine(Event::TurnComplete {
        turn: TurnId(1),
        usage: quack_protocol::Usage::default(),
    });
    assert_eq!(app.resume_id().as_deref(), Some("0123456789ab"));

    // `/fork` replaces the session id while staying saved.
    app.on_engine(Event::SessionConfigured {
        session: Box::new(session("fedcba987654")),
    });
    assert_eq!(app.resume_id().as_deref(), Some("fedcba987654"));

    // `/new` clears saved state until a turn starts in the new session.
    type_str(&mut app, "/new ");
    press(&mut app, KeyCode::Enter);
    app.on_engine(Event::SessionConfigured {
        session: Box::new(session("aaaaaaaaaaaa")),
    });
    assert_eq!(app.resume_id(), None);

    // Resuming a saved session marks it saved immediately.
    app.on_engine(Event::SessionResumed {
        session: Box::new(session("bbbbbbbbbbbb")),
        items: Vec::new().into(),
    });
    assert_eq!(app.resume_id().as_deref(), Some("bbbbbbbbbbbb"));

    // A turn that aborts because no model is ready creates no session log on disk.
    let mut unconfigured = App::new(info());
    unconfigured.on_engine(Event::SessionConfigured {
        session: Box::new(quack_protocol::SessionInfo {
            model: None,
            ..session("cccccccccccc")
        }),
    });
    unconfigured.on_engine(Event::TurnStarted { turn: TurnId(1) });
    unconfigured.on_engine(Event::TurnAborted {
        turn: TurnId(1),
        reason: "no model".into(),
    });
    assert_eq!(unconfigured.resume_id(), None);
}

#[test]
fn mcp_modal_enables_examples_disables_active_and_runs_add_prompts() {
    let mut info = info();
    info.mcp_items = vec![quack_config::McpSummary {
        id: "context7".into(),
        layer: quack_config::LayerKind::Defaults,
        path: "/tmp/defaults/mcp.d/context7.toml".into(),
        enabled: false,
        disabled_reason: Some("enabled = false".into()),
        parse_error: None,
        description: "Library docs".into(),
        transport: "http".into(),
        target: "https://mcp.context7.com/mcp".into(),
        env_vars: Vec::new(),
    }];

    let mut app = App::new(info);
    type_str(&mut app, "/mcp");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::ListMcp]);

    // With no active servers yet, the modal shows the bundled example and Enter enables it.
    app.on_engine(Event::McpServers {
        servers: Vec::new(),
    });
    assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("MCP"));
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::EnableMcp {
            id: "context7".into(),
        }]
    );

    // Remote HTTP add prompt: URL -> ID (prefilled) -> env:ROOST_TOKEN.
    type_str(&mut app, "/mcp add http");
    press(&mut app, KeyCode::Enter);
    type_str(&mut app, "https://roost.internal:8484/mcp");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    type_str(&mut app, "env:ROOST_TOKEN");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::AddMcp {
            id: "roost".into(),
            url: "https://roost.internal:8484/mcp".into(),
            command: String::new(),
            args: Vec::new(),
            bearer_env: Some("ROOST_TOKEN".into()),
            bearer_token: None,
            scope: Some(WriteScope::User),
        }]
    );

    // Local stdio add prompt: command -> prefilled id -> Op::AddMcp.
    type_str(&mut app, "/mcp add stdio");
    press(&mut app, KeyCode::Enter);
    type_str(&mut app, "npx -y @playwright/mcp@latest");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::AddMcp {
            id: "playwright".into(),
            url: String::new(),
            command: "npx".into(),
            args: vec!["-y".into(), "@playwright/mcp@latest".into()],
            bearer_env: None,
            bearer_token: None,
            scope: Some(WriteScope::User),
        }]
    );
}

#[test]
fn scope_toggle_and_project_writes_work_across_modals_prompts_and_approvals() {
    use quack_config::SettingRow;
    use quack_protocol::{ApprovalRequest, ModelInfo, ProfileInfo, ProviderModels};

    let mut i = info();
    i.project_config = "/repo/.quack/config.toml".into();
    i.project_status = ProjectStatus::Trusted;
    i.settings = vec![SettingRow {
        key: "ui.ascii".into(),
        value: "false".into(),
        origin: "default".into(),
        description: "Use ASCII glyphs".into(),
        is_bool: true,
        locked: false,
    }];
    let mut app = App::new(i);
    let ctrl_l = KeyEvent::new(KeyCode::Char('l'), KeyModifiers::CONTROL);
    let ctrl_s = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::CONTROL);

    // /config modal: Ctrl-L flips scope to project (.quack) and Enter saves `--project`.
    type_str(&mut app, "/config");
    press(&mut app, KeyCode::Enter);
    app.on_terminal(TermEvent::Key(ctrl_l));
    assert_eq!(app.write_scope, WriteScope::Project);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveSetting {
            key: "ui.ascii".into(),
            value: "true".into(),
            scope: WriteScope::Project,
        }]
    );
    // Saved project setting updates row.origin to `project: ...` instead of `user: ...`.
    app.on_engine(Event::SettingSaved {
        key: "ui.ascii".into(),
        value: "true".into(),
        path: "/repo/.quack/config.toml".into(),
        note: None,
    });
    assert_eq!(
        app.info.settings[0].origin,
        "project: /repo/.quack/config.toml"
    );

    // `/scope user` switches back; `/models` modal Ctrl-L + Ctrl-S uses Project again.
    type_str(&mut app, "/scope user");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.write_scope, WriteScope::User);

    type_str(&mut app, "/models");
    press(&mut app, KeyCode::Enter);
    app.take_ops();
    app.on_engine(Event::ModelCatalog {
        providers: vec![ProviderModels {
            bundled: false,
            key: None,
            id: "charm".into(),
            available: true,
            note: String::new(),
            models: vec![ModelInfo {
                id: "true".into(),
                alias: None,
                context_window: None,
                price: None,
                reasoning_efforts: Vec::new(),
            }],
        }],
        current: None,
    });
    app.on_terminal(TermEvent::Key(ctrl_l));
    app.on_terminal(TermEvent::Key(ctrl_s));
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveSetting {
            key: "model.default".into(),
            value: "\"charm/true\"".into(),
            scope: WriteScope::Project,
        }]
    );

    // `/profiles` modal Ctrl-L toggles back to User and Ctrl-S saves to User.
    type_str(&mut app, "/profiles");
    press(&mut app, KeyCode::Enter);
    app.take_ops();
    app.on_engine(Event::Profiles {
        active: "generic".into(),
        forced: false,
        profiles: vec![ProfileInfo {
            id: "qwen-code".into(),
            source: "defaults".into(),
            edit_format: "edit".into(),
            description: "Qwen".into(),
            tools: vec!["read".into()],
            system: "hi".into(),
            matches: false,
        }],
    });
    app.on_terminal(TermEvent::Key(ctrl_l));
    assert_eq!(app.write_scope, WriteScope::User);
    app.on_terminal(TermEvent::Key(ctrl_s));
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveSetting {
            key: "model.profile".into(),
            value: "\"qwen-code\"".into(),
            scope: WriteScope::User,
        }]
    );

    // `/theme --project light` saves directly to Project scope.
    type_str(&mut app, "/theme --project light");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::SaveSetting {
            key: "ui.theme".into(),
            value: "\"light\"".into(),
            scope: WriteScope::Project,
        }]
    );

    // `/mcp add stdio` inherits User then Ctrl-L inside the prompt flips it to Project.
    type_str(&mut app, "/mcp add stdio");
    press(&mut app, KeyCode::Enter);
    app.on_terminal(TermEvent::Key(ctrl_l));
    type_str(&mut app, "npx -y @playwright/mcp@latest");
    press(&mut app, KeyCode::Enter);
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.take_ops(),
        vec![Op::AddMcp {
            id: "playwright".into(),
            url: String::new(),
            command: "npx".into(),
            args: vec!["-y".into(), "@playwright/mcp@latest".into()],
            bearer_env: None,
            bearer_token: None,
            scope: Some(WriteScope::Project),
        }]
    );

    // Approval in a trusted project defaults to Project and Ctrl-L flips to User.
    app.on_engine(Event::ApprovalRequest {
        call_id: CallId("c1".into()),
        request: ApprovalRequest::Shell {
            command: "cargo test".into(),
            cwd: ".".into(),
            outside_sandbox: false,
        },
        always: vec!["shell:cargo test".into()],
    });
    assert_eq!(
        app.approval.as_ref().map(|a| a.always_scope),
        Some(WriteScope::Project)
    );
    app.on_terminal(TermEvent::Key(ctrl_l));
    assert_eq!(
        app.approval.as_ref().map(|a| a.always_scope),
        Some(WriteScope::User)
    );
    press(&mut app, KeyCode::Char('A'));
    assert_eq!(
        app.take_ops(),
        vec![Op::ApprovalResponse {
            call_id: CallId("c1".into()),
            decision: ApprovalDecision::AlwaysAllow {
                scope: WriteScope::User,
            },
        }]
    );
}

#[test]
fn probe_opens_modal_and_updates_on_report() {
    let mut app = App::new(info());
    type_str(&mut app, "/probe");
    press(&mut app, KeyCode::Enter);
    assert_eq!(app.take_ops(), vec![Op::ProbeToolCalling]);
    assert_eq!(
        app.modal.as_ref().map(|m| m.title.as_str()),
        Some("Tool Probe")
    );
    app.on_engine(Event::ToolProbeResult {
        report: quack_protocol::ToolProbeReport {
            model: "test/m".into(),
            configured_mode: "native".into(),
            recommended_mode: "native".into(),
            recommended_text_format: None,
            cases: vec![quack_protocol::ToolProbeCase {
                name: "native_tool".into(),
                passed: true,
                latency_ms: 42,
                detail: "tool invoked cleanly".into(),
            }],
            summary: "Ready for native tool calls".into(),
        },
    });
    assert!(app.modal.is_some());
}

#[test]
fn gui_handoff_quits_only_when_binary_available() {
    let mut app = App::new(info());
    app.request_gui_handoff(false);
    assert!(!app.wants_gui_handoff());
    assert!(!app.should_quit());

    app.request_gui_handoff(true);
    assert!(app.wants_gui_handoff());
    assert!(app.should_quit());
}

#[test]
fn changing_work_mode_and_approvals_directly_preserves_input_text() {
    use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    let mut app = App::new(info());
    type_str(&mut app, "refactor the authentication middleware");
    assert_eq!(app.input.text(), "refactor the authentication middleware");

    // 1. Ctrl-Y toggles approvals (ask -> allow -> ask) without touching input text.
    app.on_terminal(TermEvent::Key(KeyEvent::new(
        KeyCode::Char('y'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(
        app.take_ops(),
        vec![Op::SetPermissionMode {
            mode: PermissionMode::Allow,
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Edit,
        approvals: quack_protocol::ApprovalPolicy::Allow,
    });
    assert_eq!(app.input.text(), "refactor the authentication middleware");

    // 2. Shift-Tab toggles work mode (edit -> plan) without touching input text.
    press(&mut app, KeyCode::BackTab);
    assert_eq!(
        app.take_ops(),
        vec![Op::SetWorkMode {
            mode: WorkMode::Plan,
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Plan,
        approvals: quack_protocol::ApprovalPolicy::Allow,
    });
    assert_eq!(app.input.text(), "refactor the authentication middleware");

    // 3. Clicking the status-bar work-mode and approval pills toggles them without touching input.
    let mut terminal = Terminal::new(TestBackend::new(100, 24)).expect("terminal");
    terminal
        .draw(|f| crate::ui::draw(f, &app))
        .expect("draw status bar");
    let work_rect = app.work_mode_area.get();
    let perm_rect = app.permission_area.get();
    assert!(work_rect.width > 0 && perm_rect.width > 0);

    app.on_terminal(TermEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: work_rect.x + 1,
        row: work_rect.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        app.take_ops(),
        vec![Op::SetWorkMode {
            mode: WorkMode::Edit,
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Edit,
        approvals: quack_protocol::ApprovalPolicy::Allow,
    });
    assert_eq!(app.input.text(), "refactor the authentication middleware");

    app.on_terminal(TermEvent::Mouse(MouseEvent {
        kind: MouseEventKind::Down(MouseButton::Left),
        column: perm_rect.x + 1,
        row: perm_rect.y,
        modifiers: KeyModifiers::NONE,
    }));
    assert_eq!(
        app.take_ops(),
        vec![Op::SetPermissionMode {
            mode: PermissionMode::Ask,
        }]
    );
    app.on_engine(Event::ModeChanged {
        work_mode: WorkMode::Edit,
        approvals: quack_protocol::ApprovalPolicy::Ask,
    });
    assert_eq!(app.input.text(), "refactor the authentication middleware");

    // 4. Selecting `/plan` or `/permissions` from Ctrl-P (Command palette) executes immediately
    // and leaves the input text untouched instead of replacing it with `/plan ` or `/permissions `.
    app.on_terminal(TermEvent::Key(KeyEvent::new(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    )));
    assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Help"));
    type_str(&mut app, "plan");
    press(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert_eq!(
        app.take_ops(),
        vec![Op::SetWorkMode {
            mode: WorkMode::Plan,
        }]
    );
    assert_eq!(app.input.text(), "refactor the authentication middleware");

    app.on_terminal(TermEvent::Key(KeyEvent::new(
        KeyCode::Char('p'),
        KeyModifiers::CONTROL,
    )));
    type_str(&mut app, "permissions");
    press(&mut app, KeyCode::Enter);
    assert_eq!(
        app.modal.as_ref().map(|m| m.title.as_str()),
        Some("Permissions")
    );
    assert_eq!(app.input.text(), "refactor the authentication middleware");
    // Select `allow` in the Permissions modal and press Enter: applies immediately, input stays intact.
    press(&mut app, KeyCode::Down);
    press(&mut app, KeyCode::Enter);
    assert!(app.modal.is_none());
    assert_eq!(
        app.take_ops(),
        vec![Op::SetPermissionMode {
            mode: PermissionMode::Allow,
        }]
    );
    assert_eq!(app.input.text(), "refactor the authentication middleware");
}
