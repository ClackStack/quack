use std::hash::{Hash, Hasher};
use std::sync::Arc;

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use quack_protocol::LoadState;

use quack_config::ProjectStatus;
use quack_config::settings::ShowReasoning;
use quack_protocol::{ApprovalRequest, WriteScope};

use crate::app::{App, Entry, PlanState, PopupItem, ToolEntry};
use crate::fuzzy;
use crate::select::Pane;
use crate::wrap::wrap;

mod chrome;
mod overlays;
mod transcript;

pub(crate) use chrome::plan_mark;
use chrome::{
    draw_attachments, draw_header, draw_input, draw_panel, draw_plan, draw_plan_list, draw_status,
    input_width, plan_open, welcome,
};
pub(crate) use overlays::key_hints;
use overlays::{choices, draw_approval, draw_choices, draw_popup, draw_prompt};
pub(crate) use transcript::{TranscriptCache, transcript_rows_with_gutters};
use transcript::{assistant_lead, draw_transcript};

const MAX_INPUT_LINES: usize = 6;
const MAX_POPUP_ROWS: usize = 8;

pub(crate) fn draw(f: &mut Frame<'_>, app: &App) {
    let (main, sidebar) = crate::sidebar::areas(f.area(), app.info.ui.sidebar);
    // The sidebar's startup section replaces the panel: one place shows the diagnostics,
    // so they never appear and vanish a moment later.
    let startup_sidebar = sidebar.is_some() && app.startup_status_visible();
    let sidebar_view = sidebar.map(|area| crate::sidebar::build(app, area));
    let sidebar_plan = sidebar_view.as_ref().is_some_and(|s| s.shows_plan);
    // The question or plan review bar takes rows from the transcript, keeping a few; the
    // startup panel gives way to it.
    let choices = choices(app, usize::from(main.width).saturating_sub(2));
    let panel_rows = if app.panel_visible() && !startup_sidebar && choices.is_none() {
        u16::try_from(app.startup.len()).unwrap_or(0) + 1
    } else {
        0
    };
    let input_rows = u16::try_from(
        app.input
            .layout(input_width(main.width))
            .rows
            .len()
            .min(MAX_INPUT_LINES),
    )
    .unwrap_or(1)
        + 2;
    let plan_rows = if app.plan_expanded && !app.plan.is_empty() {
        // The whole plan, up to a third of the screen.
        let cap = (main.height / 3).max(2);
        u16::try_from(app.plan.len() + 1)
            .unwrap_or(u16::MAX)
            .min(cap)
    } else if sidebar_plan {
        0
    } else {
        u16::from(plan_open(app))
    };
    let attach_rows = u16::from(!app.attachments.is_empty());
    let choice_rows = choices.as_ref().map_or(0, |c| {
        let room = main
            .height
            .saturating_sub(1 + panel_rows + plan_rows + input_rows + 1 + 3);
        u16::try_from(c.height()).unwrap_or(u16::MAX).min(room)
    });
    let [
        header,
        panel,
        transcript,
        choice_area,
        plan,
        attach,
        input,
        status,
    ] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(panel_rows),
        Constraint::Min(1),
        Constraint::Length(choice_rows),
        Constraint::Length(plan_rows),
        Constraint::Length(attach_rows),
        Constraint::Length(input_rows),
        Constraint::Length(1),
    ])
    .areas(main);

    draw_header(f, app, header, sidebar.is_some());
    if panel_rows > 0 {
        draw_panel(f, app, panel);
    }
    draw_transcript(f, app, transcript);
    if plan_rows > 1 {
        draw_plan_list(f, app, plan);
    } else if plan_rows > 0 {
        draw_plan(f, app, plan);
    }
    if attach_rows > 0 {
        draw_attachments(f, app, attach);
    }
    draw_input(f, app, input);
    draw_status(
        f,
        app,
        status,
        sidebar.is_some() && !startup_sidebar,
        startup_sidebar,
    );
    if let (Some(area), Some(view)) = (sidebar, sidebar_view) {
        view.draw(f, app, area);
    } else {
        app.sidebar_view.set(crate::sidebar::View::default());
        app.sidebar_scroll.set(0);
    }
    draw_popup(f, app, transcript);
    if let Some(c) = choices {
        draw_choices(f, app, choice_area, c);
    }
    draw_approval(f, app, transcript);
    if let Some(modal) = &app.modal {
        modal.draw(f, &app.theme, &app.glyphs);
    }
    draw_prompt(f, app);
    if let Some(form) = &app.hook_form {
        form.draw(f, &app.theme, &app.glyphs, app.info.ui.ascii);
    }
}

/// Truncate to `max` columns with an ellipsis.
pub(crate) fn truncate(s: &str, max: usize) -> String {
    if s.width() <= max {
        return s.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > max {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::test_info;
    use quack_protocol::{Event, Subsystem};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    /// Startup ends; these tests are not about the first-run `/setup` offer.
    fn started(app: &mut App) {
        app.skip_setup_offer();
        app.dismiss_startup();
        app.on_engine(Event::StartupComplete);
    }

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(w, h)).expect("terminal");
        terminal.draw(|f| draw(f, app)).expect("draw");
        terminal.backend().to_string()
    }

    fn started_app() -> App {
        let mut app = App::new(test_info());
        let rows = [
            (
                Subsystem::Config,
                LoadState::Ready {
                    detail: "2 config files · 1 providers · 3ms".into(),
                },
            ),
            (
                Subsystem::Project,
                LoadState::Disabled {
                    reason: "no .quack/ in this workspace".into(),
                },
            ),
            (
                Subsystem::Provider,
                LoadState::NeedsAttention {
                    detail: "no model selected · set [model].default or use /models".into(),
                },
            ),
            (
                Subsystem::Profile,
                LoadState::Ready {
                    detail: "empty (compiled-in)".into(),
                },
            ),
            (
                Subsystem::Tokenizer,
                LoadState::Running {
                    detail: "selecting tokenizer".into(),
                },
            ),
            (
                Subsystem::Lsp,
                LoadState::Failed {
                    error: "rust-analyzer: not found on PATH".into(),
                },
            ),
        ];
        for (subsystem, state) in rows {
            app.on_engine(Event::StartupProgress { subsystem, state });
        }
        app
    }

    #[test]
    fn status_bar_says_when_the_sandbox_is_off() {
        let mut app = App::new(test_info());
        assert!(!render(&app, 100, 30).contains("no sandbox"));
        app.on_engine(Event::Sandbox { off: true });
        assert!(render(&app, 100, 30).contains("no sandbox"));
        // Yolo's pill already says so.
        app.on_engine(Event::ModeChanged {
            work_mode: quack_protocol::WorkMode::Edit,
            approvals: quack_protocol::ApprovalPolicy::Yolo,
        });
        assert!(!render(&app, 100, 30).contains("no sandbox"));
    }

    #[test]
    fn status_bar_counts_language_server_problems() {
        let mut app = App::new(test_info());
        assert!(!render(&app, 100, 30).contains('⚠'));
        app.on_engine(Event::Diagnostics {
            errors: 1,
            warnings: 3,
        });
        assert!(render(&app, 100, 30).contains("✗1 ⚠3"));
        app.on_engine(Event::Diagnostics {
            errors: 0,
            warnings: 2,
        });
        let s = render(&app, 100, 30);
        assert!(s.contains("⚠2") && !s.contains('✗'), "{s}");
        app.info.ui.ascii = true;
        assert!(render(&app, 100, 30).contains("W2"));
        app.on_engine(Event::Diagnostics {
            errors: 0,
            warnings: 0,
        });
        assert!(!render(&app, 100, 30).contains("W2"));
    }

    #[test]
    fn startup_panel_shows_only_problems_until_the_user_starts_working() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let ready = |subsystem| Event::StartupProgress {
            subsystem,
            state: LoadState::Ready {
                detail: "ready-detail".into(),
            },
        };
        // A slow, clean start never shows the panel: the status bar's "starting" covers
        // the wait, and the flash of a panel that vanishes on completion is gone.
        let mut app = App::new(test_info());
        app.on_engine(ready(Subsystem::Config));
        for _ in 0..120 {
            app.on_tick();
        }
        assert!(!render(&app, 80, 24).contains("ready-detail"));
        started(&mut app);
        assert!(!render(&app, 80, 24).contains("ready-detail"));

        // A problem shows it at once and keeps it after startup.
        let mut app = App::new(test_info());
        app.on_engine(Event::StartupProgress {
            subsystem: Subsystem::Provider,
            state: LoadState::NeedsAttention {
                detail: "no model selected".into(),
            },
        });
        assert!(render(&app, 80, 24).contains("no model selected"));
        // Completion alone does not pull it away.
        app.skip_setup_offer();
        app.on_engine(Event::StartupComplete);
        assert!(render(&app, 80, 24).contains("no model selected"));

        // Once the user starts working, the panel goes and the status bar carries the
        // issue count instead.
        app.execute("/help");
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        let s = render(&app, 80, 24);
        assert!(!s.contains("no model selected"), "{s}");
        assert!(s.contains("! 1 startup issue"), "{s}");
    }

    #[test]
    fn startup_panel_80x24() {
        insta::assert_snapshot!(render(&started_app(), 80, 24));
    }

    #[test]
    fn help_modal_100x30() {
        let mut app = started_app();
        app.modal = Some(crate::modal::help(&app.theme));
        insta::assert_snapshot!(render(&app, 100, 30));
    }

    #[test]
    fn hook_setup_connections_and_masked_input() {
        use crate::theme::{Glyphs, Theme};
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        let mut app = App::new(test_info());
        started(&mut app);
        app.hook_form = Some(crate::hook_form::HookForm::new(crate::hook_form::example()));
        insta::assert_snapshot!("hook_setup_80x24", render(&app, 80, 24));
        insta::assert_snapshot!("hook_setup_60x20", render(&app, 60, 20));
        let form = app.hook_form.as_mut().expect("form");
        form.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        for _ in 0..3 {
            form.key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        }
        form.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        form.paste("private-test-token");
        app.info.ui.ascii = true;
        app.glyphs = Glyphs::new(true);
        app.theme = Theme::resolve("light", false);
        let shown = render(&app, 60, 20);
        assert!(!shown.contains("private-test-token"));
        insta::assert_snapshot!("hook_setup_secret_light_ascii_60x20", shown);
    }

    #[test]
    fn context_modal_100x30() {
        use quack_protocol::{ContextReport, ContextSegment};
        let mut app = started_app();
        let seg = |name: &str, tokens, detail: &str| ContextSegment {
            name: name.into(),
            tokens,
            detail: detail.into(),
            zone: None,
            prunable_indices: Vec::new(),
        };
        let report = ContextReport {
            model: "charm/qwen3.8-flash".into(),
            window: Some(131_072),
            budget: Some(111_411),
            threshold: Some(89_128),
            used: 41_870,
            strategy: "auto".into(),
            segments: vec![
                seg("System prompt", 1_210, "profile generic, with instructions"),
                seg("Tool definitions", 1_650, "7 tools"),
                seg("Summary", 2_400, "earlier conversation, summarized"),
                seg("Your messages", 610, "6 messages"),
                seg("Model replies and tool calls", 4_300, "31 messages"),
                seg("Tool results", 31_700, "29 results, 8 pruned"),
            ],
            tail_items: Vec::new(),
            note: None,
        };
        let growth = [9_000, 16_000, 22_500, 30_000, 41_870];
        app.modal = Some(crate::modal::context(
            Some(&report),
            &growth,
            false,
            &app.theme,
        ));
        insta::assert_snapshot!(render(&app, 100, 30));
        // The biggest part, with its share drawn in its band colour.
        for _ in 0..6 {
            key(
                &mut app,
                crossterm::event::KeyCode::Down,
                crossterm::event::KeyModifiers::NONE,
            );
        }
        let s = render(&app, 100, 30);
        assert!(s.contains("of budget"), "{s}");
        insta::assert_snapshot!("context_modal_segment_100x30", s);
    }

    #[test]
    fn stats_modal_100x30() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{RequestStat, StatsReport, ToolStat};
        let mut app = App::new(test_info());
        started(&mut app);
        let tool = |name: &str, calls, failed, ms| ToolStat {
            name: name.into(),
            calls,
            failed,
            ms,
        };
        let report = StatsReport {
            model: "charm/qwen3.8-flash".into(),
            turns: 6,
            requests: 23,
            unreported: 0,
            prompt_tokens: 412_000,
            completion_tokens: 9_800,
            cached_tokens: 301_000,
            cache_write_tokens: 0,
            reasoning_tokens: 0,
            cache_reported: 23,
            cost: Some(0.1374),
            unpriced: 0,
            ttft_ms: Some((640, 2_900)),
            tokens_per_sec: Some(52.0),
            open_ms: 600_000,
            model_ms: 210_000,
            tools_ms: 45_000,
            approval_ms: 30_000,
            tools: vec![
                tool("read", 9, 0, 400),
                tool("shell", 6, 2, 40_000),
                tool("edit", 4, 0, 4_600),
            ],
            compactions: 1,
            pruned: 8,
            resumed: false,
            sampling: vec![("temperature".into(), 0.7), ("top_k".into(), 20.0)],
            // A context that grows, with a dip at the compaction.
            history: (0..23_u32)
                .map(|i| RequestStat {
                    prompt: 8_000 + (i % 14) * 2_000,
                    cached: 6_000 + (i % 14) * 1_500,
                    cache_write: 0,
                    completion: 200 + (i * 37) % 700,
                    reasoning: 0,
                    cost: Some(0.002 + f64::from(i % 14) * 0.0005),
                    ttft_ms: Some(400 + (i * 113) % 900),
                    tokens_per_sec: Some(40.0 + f64::from((i * 7) % 20) as f32),
                })
                .collect(),
        };
        app.modal = Some(crate::modal::stats(Some(&report), false, &app.theme));
        insta::assert_snapshot!(render(&app, 100, 30));
        for pane in ["tokens", "cost", "time", "tools"] {
            key(&mut app, KeyCode::Down, KeyModifiers::NONE);
            let s = render(&app, 100, 30);
            if pane == "time" {
                assert!(s.contains("Where the time went"), "{s}");
                assert!(s.contains("the model"), "{s}");
                assert!(s.contains("35%"), "model is 210 of 600 s: {s}");
            }
            insta::assert_snapshot!(format!("stats_modal_{pane}_100x30"), s);
        }

        // Small terminals squeeze the charts or drop them, without panicking.
        for (w, h) in [(60, 20), (40, 12), (20, 8)] {
            for _ in 0..5 {
                render(&app, w, h);
                key(&mut app, KeyCode::Up, KeyModifiers::NONE);
            }
        }

        // ASCII mode draws the same charts from plain characters.
        app.modal = Some(crate::modal::stats(Some(&report), true, &app.theme));
        for _ in 0..5 {
            let s = render(&app, 100, 30);
            let fancy = s
                .chars()
                .find(|c| matches!(c, '▁'..='█' | '━' | '\u{2800}'..='\u{28ff}'));
            assert_eq!(fancy, None, "{s}");
            key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        }
    }

    #[test]
    fn doctor_and_models_modals_100x30() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{CheckStatus, DoctorCheck, ModelInfo, ProviderModels};
        let mut app = started_app();
        let check = |name: &str, status, detail: &str| DoctorCheck {
            name: name.into(),
            status,
            detail: detail.into(),
        };
        let checks = [
            check("config", CheckStatus::Ok, "2 files"),
            check("charm", CheckStatus::Ok, "ready"),
            check("ollama", CheckStatus::Warn, "not running"),
            check("sandbox", CheckStatus::Fail, "Landlock is not available"),
        ];
        app.modal = Some(crate::modal::doctor(Some(&checks), &app.theme));
        let s = render(&app, 100, 30);
        assert!(s.contains("Summary") && s.contains("1 failed"), "{s}");
        insta::assert_snapshot!("doctor_modal_100x30", s);

        let model = |id: &str, window, efforts: &[&str]| ModelInfo {
            id: id.into(),
            alias: None,
            context_window: window,
            price: None,
            reasoning_efforts: efforts.iter().map(|e| (*e).to_owned()).collect(),
        };
        let catalog = [ProviderModels {
            bundled: false,
            key: None,
            id: "charm".into(),
            available: true,
            note: String::new(),
            models: vec![
                model("glm-5.3", Some(1_000_000), &["low", "medium", "high"]),
                model("qwen3.8-flash", Some(262_144), &["low", "high"]),
            ],
        }];
        app.modal = Some(crate::modal::models(
            Some(&catalog),
            crate::modal::ModelRoles {
                current: Some("charm/glm-5.3"),
                ..app.model_roles()
            },
            false,
            WriteScope::User,
            &app.theme,
        ));
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        let s = render(&app, 100, 30);
        assert!(s.contains("26% of largest"), "{s}");
        assert!(s.contains("thinking") && s.contains("low · high"), "{s}");
        assert!(s.contains("quack then asks for a thinking level"), "{s}");
        insta::assert_snapshot!("models_modal_100x30", s);
    }

    #[test]
    fn long_modal_list_shows_a_window_around_the_selection() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{ModelInfo, ProviderModels};
        let mut app = started_app();
        let models: Vec<_> = (0..60)
            .map(|n| ModelInfo {
                id: format!("m{n:02}"),
                alias: None,
                context_window: Some(1_000),
                price: None,
                reasoning_efforts: Vec::new(),
            })
            .collect();
        let catalog = [ProviderModels {
            bundled: false,
            key: None,
            id: "charm".into(),
            available: true,
            note: String::new(),
            models,
        }];
        app.modal = Some(crate::modal::models(
            Some(&catalog),
            crate::modal::ModelRoles {
                current: Some("charm/m00"),
                ..app.model_roles()
            },
            false,
            WriteScope::User,
            &app.theme,
        ));
        // 40 downs is far past the list pane's height: the rendered window
        // must follow the selection instead of starting at the top.
        for _ in 0..40 {
            key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        }
        let s = render(&app, 100, 30);
        assert!(s.contains("charm/m40"), "{s}");
        assert!(!s.contains("charm/m00"), "{s}");
        // Holding Down past the end clamps to the last item, still visible.
        for _ in 0..80 {
            key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        }
        let s = render(&app, 100, 30);
        assert!(s.contains("charm/m59"), "{s}");
        assert!(!s.contains("charm/m00"), "{s}");
    }

    #[test]
    fn instructions_modal_100x30() {
        use quack_protocol::InstructionFile;
        let mut app = started_app();
        let file = |source: &str, in_prompt, text: &str| InstructionFile {
            source: source.into(),
            in_prompt,
            tokens: 1_200,
            text: text.into(),
        };
        let files = [
            file(
                "AGENTS.md",
                true,
                "# Project\n\nRun `cargo test` before finishing.",
            ),
            file("web/AGENTS.md", false, "Use pnpm."),
        ];
        app.modal = Some(crate::modal::instructions(Some(&files), &app.theme));
        let s = render(&app, 100, 30);
        assert!(
            s.contains("in the system prompt") && s.contains("Run `cargo test`"),
            "{s}"
        );
        insta::assert_snapshot!("instructions_modal_100x30", s);
        app.modal = Some(crate::modal::instructions(Some(&[]), &app.theme));
        let s = render(&app, 100, 30);
        assert!(s.contains("no instruction files"), "{s}");
    }

    #[test]
    fn profiles_modal_100x30() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{Event, Op, ProfileInfo};
        let mut app = started_app();
        let profile = |id: &str, matches, tools: &[&str], system: &str| ProfileInfo {
            id: id.into(),
            description: format!("The {id} profile"),
            source: "Original".into(),
            matches,
            edit_format: "str_replace".into(),
            tools: tools.iter().map(|t| (*t).to_owned()).collect(),
            system: system.into(),
        };
        for c in "/profiles".chars() {
            key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        }
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.take_ops(), vec![Op::ListProfiles]);
        app.on_engine(Event::Profiles {
            active: "generic".into(),
            forced: false,
            profiles: vec![
                profile(
                    "generic",
                    true,
                    &["read", "edit", "shell"],
                    "You are a coding agent.\nBe brief.",
                ),
                profile(
                    "qwen-code",
                    false,
                    &["read_file", "edit", "run_shell_command"],
                    "You are an interactive agent.",
                ),
            ],
        });
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        let s = render(&app, 100, 30);
        assert!(s.contains("read_file, edit, run_shell_command"), "{s}");
        assert!(s.contains("You are an interactive agent."), "{s}");
        insta::assert_snapshot!("profiles_modal_100x30", s);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            app.take_ops(),
            vec![Op::SetProfile {
                id: "qwen-code".into()
            }]
        );
    }

    #[test]
    fn status_bar_shows_context_growing_to_maximum() {
        let mut app = started_app();
        app.context = Some((60_000, 104_000));
        app.compact_at = Some(83_200);
        let s = render(&app, 100, 24);
        assert!(s.contains("60k/104k"), "{s}");
        app.context = Some((90_000, 104_000));
        let s = render(&app, 100, 24);
        assert!(s.contains("90k/104k"), "{s}");
        app.context = Some((12_345, 0));
        let s = render(&app, 100, 24);
        assert!(s.contains("12.3k in context"), "{s}");
    }

    #[test]
    fn sidebar_shows_session_details_and_follows_live_updates() {
        let mut app = App::new(test_info());
        app.info.model = Some("charm/qwen3.8-flash".into());
        app.info.profile = "generic".into();
        started(&mut app);
        app.transcript.push(Entry::User("fix the parser".into()));
        app.transcript.push(Entry::Assistant(
            "I found the failing case and am checking the parser.".into(),
        ));
        app.take_dirty();
        app.on_engine(Event::SessionTitle {
            title: Some("Fix the parser".into()),
        });
        assert!(app.take_dirty(), "a title update must redraw the sidebar");
        app.on_engine(Event::ContextUpdate {
            used: 60_000,
            budget: 88_400,
            window: Some(104_000),
            cache_hit_tokens: None,
            cache_hit_percent: None,
            threshold: Some(83_200),
        });
        app.on_engine(Event::Speed {
            ttft_ms: 120,
            tokens_per_sec: Some(48.0),
        });
        let s = render(&app, 140, 24);
        for text in [
            "Fix the parser",
            "Working folder",
            "~/src/project",
            "Git branch  main",
            "charm/qwen3.8-flash",
            "Profile  generic",
            "60k / 104k tokens (57%)",
            "Summarize at 83.2k tokens",
            "48 tokens/s",
        ] {
            assert!(s.contains(text), "missing {text}: {s}");
        }
        assert_eq!(s.matches("~/src/project").count(), 1);
        assert_eq!(s.matches("charm/qwen3.8-flash").count(), 2);
        insta::assert_snapshot!("sidebar_140x24", s);

        app.set_branch(Some("fix/parser".into()));
        assert!(app.take_dirty());
        assert!(render(&app, 140, 24).contains("Git branch  fix/parser"));
        app.set_branch(None);
        assert!(!render(&app, 140, 24).contains("Git branch"));
        app.on_engine(Event::SessionTitle { title: None });
        assert!(render(&app, 140, 24).contains("New session"));
    }

    #[test]
    fn sidebar_shows_cache_hit_all_skills_and_active_lsp() {
        use quack_protocol::{CallId, LspServerInfo, LspServerState, SkillInfo};

        let mut app = App::new(test_info());
        app.info.model = Some("charm/qwen3.8-flash".into());
        app.info.profile = "generic".into();
        started(&mut app);
        app.transcript.push(Entry::User("check the parser".into()));
        app.on_engine(Event::SessionTitle {
            title: Some("Fix the parser".into()),
        });
        app.on_engine(Event::ContextUpdate {
            used: 60_000,
            budget: 88_400,
            window: Some(104_000),
            cache_hit_tokens: Some(44_000),
            cache_hit_percent: Some(73),
            threshold: Some(83_200),
        });
        app.on_engine(Event::Speed {
            ttft_ms: 120,
            tokens_per_sec: Some(48.0),
        });
        app.on_engine(Event::Skills {
            skills: ["rust-best-practices", "writing"]
                .into_iter()
                .map(|name| SkillInfo {
                    name: name.into(),
                    description: String::new(),
                    source: String::new(),
                    dir: String::new(),
                })
                .collect(),
        });
        app.on_engine(Event::LoadedSkills {
            skills: vec!["rust-best-practices".into(), "release".into()],
        });
        app.on_engine(Event::LspServers {
            servers: vec![
                LspServerInfo {
                    id: "rust-analyzer".into(),
                    command: "rust-analyzer".into(),
                    extensions: vec!["rs".into()],
                    state: LspServerState::Ready,
                    detail: String::new(),
                    root: ".".into(),
                    errors: 0,
                    warnings: 0,
                    log: Vec::new(),
                },
                LspServerInfo {
                    id: "typescript".into(),
                    command: "typescript-language-server --stdio".into(),
                    extensions: vec!["ts".into()],
                    state: LspServerState::Ready,
                    detail: String::new(),
                    root: "ui".into(),
                    errors: 2,
                    warnings: 1,
                    log: Vec::new(),
                },
                LspServerInfo {
                    id: "pyright".into(),
                    command: "pyright-langserver --stdio".into(),
                    extensions: vec!["py".into()],
                    state: LspServerState::Indexing,
                    detail: "analyzing".into(),
                    root: ".".into(),
                    errors: 0,
                    warnings: 0,
                    log: Vec::new(),
                },
                // Idle and uninstalled servers stay hidden from the sidebar.
                LspServerInfo {
                    id: "gopls".into(),
                    command: "gopls".into(),
                    extensions: vec!["go".into()],
                    state: LspServerState::Idle,
                    detail: "/usr/bin/gopls".into(),
                    root: String::new(),
                    errors: 0,
                    warnings: 0,
                    log: Vec::new(),
                },
                LspServerInfo {
                    id: "clangd".into(),
                    command: "clangd".into(),
                    extensions: vec!["c".into()],
                    state: LspServerState::NotInstalled,
                    detail: "not installed".into(),
                    root: String::new(),
                    errors: 0,
                    warnings: 0,
                    log: Vec::new(),
                },
            ],
        });

        let s = render(&app, 140, 30);
        for text in [
            "48 tokens/s · 73% cache hit",
            "Skills",
            "✓ rust-best-practices",
            "· writing",
            "✓ release",
            "LSP",
            "✓ rust-analyzer  ready",
            "✓ typescript (ui)  ✗2 ⚠1",
            "pyright  indexing",
        ] {
            assert!(s.contains(text), "missing {text}: {s}");
        }
        assert_eq!(s.matches("rust-best-practices").count(), 1);
        assert!(
            !s.contains("gopls") && !s.contains("clangd"),
            "idle and uninstalled servers must stay hidden: {s}"
        );
        insta::assert_snapshot!("sidebar_cache_skills_lsp_140x30", s);

        // Cache hit still renders even when speed wasn't measured yet.
        app.speed = None;
        let cache_only = render(&app, 140, 30);
        assert!(
            cache_only.contains("73% cache hit") && !cache_only.contains("tokens/s"),
            "{cache_only}"
        );

        // ASCII mode uses ASCII separators and diagnostic markers.
        app.speed = Some(48.0);
        app.info.ui.ascii = true;
        app.glyphs = crate::theme::Glyphs::new(true);
        let ascii = render(&app, 140, 30);
        assert!(
            ascii.contains("48 tokens/s - 73% cache hit")
                && ascii.contains("typescript (ui)  E2 W1"),
            "{ascii}"
        );

        app.on_engine(Event::Plan {
            checklist: "[~] check the parser".into(),
        });
        app.on_engine(Event::AgentStarted {
            id: "explore-1".into(),
            agent: "explore".into(),
            description: "find the failing case".into(),
            call_id: CallId("t-explore-1".into()),
            model: "mock/small".into(),
        });
        let full = render(&app, 140, 40);
        let headings = [
            "Session",
            "Working folder",
            "Model",
            "Context",
            "Plan",
            "Sub-agents",
            "LSP",
            "Skills",
        ]
        .map(|heading| full.find(&format!("|  {heading}")).expect(heading));
        assert!(headings.is_sorted(), "{full}");

        // Starting a new session clears loaded markers but keeps available skills and LSPs.
        app.execute("/new");
        let after_new = render(&app, 140, 30);
        assert!(
            !after_new.contains("cache hit")
                && !after_new.contains("tokens/s")
                && !after_new.contains("release")
                && after_new.contains(". rust-best-practices")
                && after_new.contains("rust-analyzer"),
            "{after_new}"
        );
    }

    #[test]
    fn sidebar_lists_sub_agents_while_they_run_and_after_they_finish() {
        use quack_protocol::CallId;
        let mut app = started_app();
        started(&mut app);
        let start = |app: &mut App, id: &str, agent: &str, description: &str| {
            app.on_engine(Event::AgentStarted {
                id: id.into(),
                agent: agent.into(),
                description: description.into(),
                call_id: CallId(format!("t-{id}")),
                model: "mock/small".into(),
            });
        };
        start(
            &mut app,
            "explore-1",
            "explore",
            "find the empty-input case",
        );
        app.on_engine(Event::Agent {
            id: "explore-1".into(),
            inner: Box::new(Event::ToolCallBegin {
                call: quack_protocol::ToolCallInfo {
                    call_id: CallId("explore-1/c1".into()),
                    name: "grep".into(),
                    summary: "grep fn parse".into(),
                },
            }),
        });
        let s = render(&app, 140, 24);
        assert!(
            s.contains("Sub-agents")
                && s.contains("find the empty-input case")
                && s.contains("explore · 1 steps"),
            "{s}"
        );
        app.on_engine(Event::AgentFinished {
            id: "explore-1".into(),
            ok: true,
            summary: "2 steps · 34.5k tokens".into(),
        });
        start(
            &mut app,
            "general-1",
            "general",
            "fix the parser and add a test",
        );
        let s = render(&app, 140, 24);
        assert!(
            s.contains("explore · 2 steps") && s.contains("fix the parser and add a test"),
            "{s}"
        );
    }

    #[test]
    fn sidebar_scrolls_to_every_skill_and_sub_agent_run() {
        use crossterm::event::{
            Event as TermEvent, KeyCode, KeyModifiers, MouseEvent, MouseEventKind,
        };
        use quack_protocol::{CallId, SkillInfo};

        let mut app = started_app();
        started(&mut app);
        app.on_engine(Event::Skills {
            skills: (0..12)
                .map(|i| SkillInfo {
                    name: format!("sidebar skill {i:02}"),
                    description: String::new(),
                    source: String::new(),
                    dir: String::new(),
                })
                .collect(),
        });
        for i in 0..8 {
            let id = format!("explore-{i}");
            app.on_engine(Event::AgentStarted {
                call_id: CallId(format!("t-{id}")),
                id: id.clone(),
                agent: "explore".into(),
                description: format!("sidebar agent {i:02}"),
                model: "mock/small".into(),
            });
            if i != 2 {
                app.on_engine(Event::AgentFinished {
                    id,
                    ok: i % 2 == 0,
                    summary: "finished".into(),
                });
            }
        }
        let tall = render(&app, 140, 55);
        for i in 0..12 {
            assert!(tall.contains(&format!("sidebar skill {i:02}")), "{tall}");
        }
        for i in 0..8 {
            assert!(tall.contains(&format!("sidebar agent {i:02}")), "{tall}");
        }

        let top = render(&app, 140, 24);
        assert!(
            top.contains("sidebar agent 00") && top.contains('┃'),
            "{top}"
        );
        assert!(!top.contains("sidebar skill 11"), "{top}");
        app.on_terminal(TermEvent::Mouse(MouseEvent {
            kind: MouseEventKind::ScrollDown,
            column: 110,
            row: 20,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.sidebar_scroll.get(), 3);
        assert_eq!(app.scroll.get(), 0);
        key(&mut app, KeyCode::PageDown, KeyModifiers::ALT);
        let bottom = render(&app, 140, 24);
        assert!(bottom.contains("sidebar agent 07"), "{bottom}");
        assert!(bottom.contains("sidebar skill 11"), "{bottom}");
        assert!(!bottom.contains("sidebar agent 00"), "{bottom}");

        key(&mut app, KeyCode::PageUp, KeyModifiers::ALT);
        assert!(render(&app, 140, 24).contains("sidebar agent 00"));
        key(&mut app, KeyCode::PageDown, KeyModifiers::ALT);
        assert!(render(&app, 140, 55).contains("sidebar agent 00"));
        assert_eq!(app.sidebar_scroll.get(), 0, "clamp after a resize");
    }

    #[test]
    fn sidebar_shows_plan_steps_and_progress_and_follows_updates() {
        let mut app = App::new(test_info());
        app.info.model = Some("charm/qwen3.8-flash".into());
        app.info.profile = "generic".into();
        started(&mut app);
        app.transcript.push(Entry::User("fix the parser".into()));
        app.transcript.push(Entry::Assistant(
            "I found the failing case and am checking the parser.".into(),
        ));
        app.on_engine(Event::SessionTitle {
            title: Some("Fix the parser".into()),
        });
        app.on_engine(Event::ContextUpdate {
            used: 60_000,
            budget: 88_400,
            window: Some(104_000),
            cache_hit_tokens: None,
            cache_hit_percent: None,
            threshold: Some(83_200),
        });
        app.on_engine(Event::Speed {
            ttft_ms: 120,
            tokens_per_sec: Some(48.0),
        });
        assert!(!render(&app, 140, 24).contains("Plan"));

        app.take_dirty();
        app.on_engine(Event::Plan {
            checklist: "[x] read the parser\n[~] cache the token table\n[ ] run the benchmarks"
                .into(),
        });
        assert!(app.take_dirty(), "a plan update must redraw the sidebar");
        let s = render(&app, 140, 24);
        for text in [
            "Plan · 1/3 (33%)",
            "✓ read the parser",
            "▶ cache the token table",
            "· run the benchmarks",
        ] {
            assert!(s.contains(text), "missing {text}: {s}");
        }
        assert!(
            !s.contains("  plan  "),
            "one-line plan strip yields to the sidebar: {s}"
        );
        insta::assert_snapshot!("sidebar_plan_140x24", s);

        // Resizing narrower restores the one-line plan strip above the input.
        let narrow = render(&app, 100, 24);
        assert!(narrow.contains("  plan  "), "{narrow}");
        assert!(narrow.contains("cache the token table"), "{narrow}");

        // `/todo` still expands the full checklist above the input when asked.
        app.execute("/todo");
        let expanded = render(&app, 140, 24);
        assert!(expanded.contains("  plan  "), "{expanded}");
        app.execute("/todo");
        assert!(!render(&app, 140, 24).contains("  plan  "));

        // Completed plans stay visible in the sidebar at 100%.
        app.on_engine(Event::Plan {
            checklist: "[x] read the parser\n[x] cache the token table\n[x] run the benchmarks"
                .into(),
        });
        let done = render(&app, 140, 24);
        assert!(
            done.contains("Plan · 3/3 (100%)")
                && done.contains("~~~~~_(•)>")
                && done.contains("✓ run the benchmarks"),
            "{done}"
        );

        // Clearing the plan removes the section.
        app.on_engine(Event::Plan {
            checklist: String::new(),
        });
        assert!(!render(&app, 140, 24).contains("Plan"));

        // Starting a new session clears any active plan immediately.
        app.on_engine(Event::Plan {
            checklist: "[~] step one".into(),
        });
        assert!(render(&app, 140, 24).contains("Plan · 0/1 (0%)"));
        app.execute("/new");
        assert!(!render(&app, 140, 24).contains("Plan"));
    }

    #[test]
    fn sidebar_plan_wraps_windows_long_lists_and_stays_compact_when_crowded() {
        use quack_protocol::CallId;
        let mut app = App::new(test_info());
        app.info.model = Some("charm/qwen3.8-flash".into());
        app.info.profile = "generic".into();
        started(&mut app);
        app.transcript.push(Entry::User("fix the parser".into()));
        app.on_engine(Event::SessionTitle {
            title: Some("Fix the parser".into()),
        });
        app.on_engine(Event::ContextUpdate {
            used: 60_000,
            budget: 88_400,
            window: Some(104_000),
            cache_hit_tokens: None,
            cache_hit_percent: None,
            threshold: Some(83_200),
        });
        app.on_engine(Event::Speed {
            ttft_ms: 120,
            tokens_per_sec: Some(48.0),
        });

        // With vertical room, a long step wraps onto two indented lines instead of truncating.
        app.on_engine(Event::Plan {
            checklist:
                "[x] inspect lexer\n[~] update token table and keep cache aligned\n[ ] run suite"
                    .into(),
        });
        let tall = render(&app, 140, 32);
        assert!(
            tall.contains("update token table and keep") && tall.contains("cache aligned"),
            "long step wraps cleanly when room allows: {tall}"
        );

        // A long plan at 24 rows keeps Context intact and windows around the active step.
        app.on_engine(Event::Plan {
            checklist: [
                "[x] step one",
                "[x] step two",
                "[x] step three",
                "[~] step four active",
                "[ ] step five",
                "[ ] step six",
                "[ ] step seven",
                "[ ] step eight",
            ]
            .join("\n"),
        });
        let windowed = render(&app, 140, 24);
        assert!(
            windowed.contains("48 tokens/s")
                && windowed.contains("Plan · 3/8 (37%)")
                && windowed.contains("▶ step four active")
                && windowed.contains("+3 earlier · +2 more")
                && !windowed.contains("  plan  "),
            "{windowed}"
        );
        insta::assert_snapshot!("sidebar_plan_windowed_140x24", windowed);

        // ASCII mode and NO_COLOR use ASCII marks, separator and river.
        app.info.ui.ascii = true;
        app.glyphs = crate::theme::Glyphs::new(true);
        app.theme = crate::theme::Theme::resolve("dark", true);
        let ascii = render(&app, 140, 24);
        assert!(
            ascii.contains("Plan - 3/8 (37%)")
                && ascii.contains("[~] step four active")
                && ascii.contains("+3 earlier, +2 more")
                && ascii.contains("~_(o)>...."),
            "{ascii}"
        );

        // With one sub-agent on a 24-row screen (3 rows left), the sidebar uses its compact
        // heading meter + active step row. With more runs, the sidebar can scroll.
        app.info.ui.ascii = false;
        app.glyphs = crate::theme::Glyphs::new(false);
        app.theme = crate::theme::Theme::resolve("dark", false);
        app.on_engine(Event::AgentStarted {
            id: "explore-1".into(),
            agent: "explore".into(),
            description: "search for explore-1".into(),
            call_id: CallId("t-explore-1".into()),
            model: "mock/small".into(),
        });
        let compact = render(&app, 140, 24);
        assert!(
            compact.contains("Plan ▰▰▰▱▱▱▱▱ 3/8")
                && compact.contains("▶ step four active")
                && !compact.contains("  plan  "),
            "compact plan row when 3 rows remain: {compact}"
        );
        app.on_engine(Event::AgentStarted {
            id: "explore-2".into(),
            agent: "explore".into(),
            description: "search for explore-2".into(),
            call_id: CallId("t-explore-2".into()),
            model: "mock/small".into(),
        });
        let crowded = render(&app, 140, 24);
        assert!(
            crowded.contains("Plan ▰▰▰▱▱▱▱▱ 3/8") && crowded.contains('┃'),
            "a scrollable list of runs leaves the compact plan in the sidebar: {crowded}"
        );
    }

    #[test]
    fn sidebar_carries_the_startup_diagnostics_until_the_user_starts_working() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut app = started_app();
        let s = render(&app, 140, 24);
        assert!(s.contains("Startup"), "{s}");
        assert!(
            s.contains("rust-analyzer") && s.contains("no model selected"),
            "the rows show their detail: {s}"
        );
        assert!(!s.contains("Working folder"), "{s}");
        insta::assert_snapshot!("sidebar_startup_140x24", s);

        // The extra width on larger terminals is the point: details stop wrapping.
        let wide = render(&app, 200, 24);
        assert!(wide.contains("no .quack/ in this workspace"), "{wide}");

        // Startup finishing does not pull the diagnostics away; the first message does.
        app.skip_setup_offer();
        app.on_engine(Event::StartupComplete);
        assert!(render(&app, 140, 24).contains("Startup"));
        app.input.set("fix the parser");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let s = render(&app, 140, 24);
        assert!(s.contains("Working folder"), "{s}");
        assert!(!s.contains("rust-analyzer"), "{s}");
    }

    #[test]
    fn sidebar_adapts_to_terminal_size_and_can_be_disabled_live() {
        let mut app = App::new(test_info());
        app.info.model = Some("mock/tiny".into());
        started(&mut app);
        // The sidebar takes one of every four columns past 140, up to 48; the
        // conversation keeps the rest and never drops below 104.
        for (width, height, main_width, visible) in [
            (139, 24, 139, false),
            (140, 24, 104, true),
            (143, 24, 107, true),
            (160, 24, 119, true),
            (200, 30, 152, true),
            (300, 30, 252, true),
            (140, 23, 140, false),
            (80, 24, 80, false),
            (140, 24, 104, true),
        ] {
            let s = render(&app, width, height);
            assert_eq!(s.contains("Working folder"), visible, "{s}");
            assert_eq!(app.transcript_area.get().width, main_width);
            assert_eq!(app.input_view.get().0.width, main_width - 2);
            assert!(
                s.contains("mock/tiny"),
                "metadata must survive resizing: {s}"
            );
        }
        app.on_engine(Event::SettingSaved {
            key: "ui.sidebar".into(),
            value: "false".into(),
            path: "/u/config.toml".into(),
            note: None,
        });
        let s = render(&app, 200, 30);
        assert!(!s.contains("Working folder"), "{s}");
        assert_eq!(app.transcript_area.get().width, 200);
        assert!(
            !s.contains("next start"),
            "the setting takes effect immediately"
        );

        app.reconfigure(test_info());
        assert!(render(&app, 140, 24).contains("Working folder"));
    }

    #[test]
    fn sidebar_bounds_long_values_and_supports_ascii_and_unreported_context() {
        let mut info = test_info();
        info.ui.ascii = true;
        info.ui.theme = "light".into();
        info.no_color = true;
        info.cwd = format!("~/{}my-project", "very-long-directory/".repeat(6));
        info.branch = Some("feature/a-very-long-branch-name-for-the-parser".into());
        info.model = Some(format!("mock/{}", "long-model-id-".repeat(6)));
        info.profile = "generic".into();
        let mut app = App::new(info);
        started(&mut app);
        app.on_engine(Event::SessionTitle {
            title: Some("A long session title that should wrap and stop before it crowds out the working folder and context".into()),
        });
        app.context = Some((90_000, 104_000));
        app.compact_at = Some(83_200);
        app.speed = Some(48.0);
        let s = render(&app, 140, 24);
        assert!(
            s.contains("my-project") && s.contains("90k / 104k tokens (86%)"),
            "{s}"
        );
        assert!(
            s.contains("48 tokens/s"),
            "long values leave room for all context rows: {s}"
        );
        insta::assert_snapshot!("sidebar_long_values_ascii_140x24", s);

        app.info.branch = None;
        app.info.model = None;
        app.info.profile = "empty".into();
        app.context = None;
        let s = render(&app, 140, 24);
        assert!(
            s.contains("Usage not reported yet") && s.contains("Select with /models"),
            "{s}"
        );
        assert!(!s.contains("Git branch") && !s.contains("Profile"), "{s}");
        app.context = Some((12_345, 0));
        let s = render(&app, 140, 24);
        assert!(
            s.contains("12.3k tokens in context") && s.contains("Window size unknown"),
            "{s}"
        );
    }

    #[test]
    fn sidebar_keeps_input_wrapping_and_mouse_coordinates_in_the_conversation() {
        use crossterm::event::{
            Event as TermEvent, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let mut app = App::new(test_info());
        started(&mut app);
        app.input.set(&"x".repeat(130));
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).expect("terminal");
        terminal.draw(|f| draw(f, &app)).expect("draw");
        let pos = terminal.get_cursor_position().expect("cursor");
        assert_eq!((pos.x, pos.y), (33, 21));
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        terminal.draw(|f| draw(f, &app)).expect("draw");
        let pos = terminal.get_cursor_position().expect("cursor");
        assert_eq!((pos.x, pos.y), (33, 20));
        app.on_terminal(TermEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 110,
            row: 20,
            modifiers: KeyModifiers::NONE,
        }));
        assert!(
            app.selection.is_none(),
            "clicking the sidebar must not select transcript or input"
        );
        key(&mut app, KeyCode::Char('y'), KeyModifiers::NONE);
        assert_eq!(
            app.input.text(),
            format!("{}y{}", "x".repeat(30), "x".repeat(100))
        );
    }

    #[test]
    fn config_modal_100x30() {
        use quack_config::SettingRow;
        let mut app = started_app();
        let row = |key: &str, value: &str, origin: &str, description: &str, is_bool| SettingRow {
            key: key.into(),
            value: value.into(),
            origin: origin.into(),
            description: description.into(),
            is_bool,
            locked: false,
        };
        let rows = vec![
            row(
                "model.default",
                "\"charm/qwen3.8-flash\"",
                "user: ~/.config/quack/config.toml",
                "`<provider-id>/<model-id>` or `<provider-id>/<alias>`. Empty: pick with `/models`.",
                false,
            ),
            row(
                "ui.ascii",
                "false",
                "default",
                "Use ASCII instead of box-drawing and spinner glyphs.",
                true,
            ),
        ];
        app.modal = Some(crate::modal::config(
            &rows,
            &[],
            WriteScope::User,
            ProjectStatus::None,
            &app.theme,
        ));
        insta::assert_snapshot!(render(&app, 100, 30));
    }

    #[test]
    fn config_layers_spacing_and_copy() {
        use crossterm::event::{Event as TerminalEvent, KeyCode, KeyEvent, KeyModifiers};
        let mut app = started_app();
        let path = "/home/developer/projects/a-long-workspace-name/.quack/config.toml";
        let layer = |name: &str, status: &str, file: Option<&str>| crate::app::ConfigLayer {
            layer: name.into(),
            status: status.into(),
            path: file.map(str::to_owned),
        };
        let files = vec![
            layer(
                "defaults",
                "loaded",
                Some("/usr/share/quack/defaults/config.toml"),
            ),
            layer("system", "no config.toml", Some("/etc/quack/config.toml")),
            layer(
                "user",
                "failed to parse (see config check)",
                Some("/home/developer/.config/quack/config.toml"),
            ),
            layer("project", "skipped: not trusted yet", Some(path)),
            layer("env", "loaded", None),
        ];
        app.modal = Some(
            crate::modal::config(
                &[],
                &files,
                WriteScope::User,
                ProjectStatus::None,
                &app.theme,
            )
            .with_tab(1),
        );
        insta::assert_snapshot!("config_layers_100x30", render(&app, 100, 30));
        insta::assert_snapshot!("config_layers_60x20", render(&app, 60, 20));
        let key = |code| TerminalEvent::Key(KeyEvent::new(code, KeyModifiers::NONE));
        for _ in 0..3 {
            app.on_terminal(key(KeyCode::Down));
        }
        insta::assert_snapshot!("config_layers_long_path_60x20", render(&app, 60, 20));
        app.on_terminal(key(KeyCode::Enter));
        assert_eq!(app.take_clipboard().as_deref(), Some(path));
        assert!(app.modal.is_some());
        app.on_terminal(key(KeyCode::Down));
        app.on_terminal(key(KeyCode::Enter));
        assert_eq!(app.take_clipboard(), None);
    }

    #[test]
    fn help_modal_narrow_60x20() {
        let mut app = started_app();
        app.modal = Some(crate::modal::help(&app.theme));
        insta::assert_snapshot!(render(&app, 60, 20));
    }

    #[test]
    fn tool_calls_and_approval_80x24() {
        use crate::app::Approval;
        use quack_protocol::{CallId, ToolCallInfo, ToolResultSummary};
        let mut app = App::new(test_info());
        started(&mut app);
        app.transcript.push(Entry::User("rename the helper".into()));
        app.on_engine(Event::ReasoningDelta {
            text: "Need to find the helper first.".into(),
        });
        app.on_engine(Event::ToolCallBegin {
            call: ToolCallInfo {
                call_id: CallId("1".into()),
                name: "grep".into(),
                summary: "grep fn helper".into(),
            },
        });
        app.on_engine(Event::ToolCallEnd {
            call_id: CallId("1".into()),
            result: ToolResultSummary {
                ok: true,
                summary: "1 matches in 1 files".into(),
                detail: None,
            },
        });
        app.on_engine(Event::ToolCallBegin {
            call: ToolCallInfo {
                call_id: CallId("2".into()),
                name: "edit".into(),
                summary: "edit src/lib.rs".into(),
            },
        });
        app.on_engine(Event::ToolCallEnd {
            call_id: CallId("2".into()),
            result: ToolResultSummary {
                ok: true,
                summary: "+1 -1".into(),
                detail: Some("--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1,2 +1,2 @@\n-fn helper() {}\n+fn assist() {}\n fn main() {}\n".into()),
            },
        });
        app.approval = Some(Approval {
            call_id: CallId("3".into()),
            request: ApprovalRequest::Shell {
                command: "cargo test".into(),
                cwd: ".".into(),
                outside_sandbox: false,
            },
            always: vec!["shell:cargo test".into()],
            always_scope: WriteScope::User,
        });
        app.context = Some((12_345, 104_000));
        insta::assert_snapshot!(render(&app, 80, 24));
    }

    #[test]
    fn todo_pane_lists_the_whole_plan_80x24() {
        use crate::app::PlanState;
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut app = started_app();
        app.plan = vec![
            (PlanState::Done, "read the parser".into()),
            (PlanState::Active, "cache the token table".into()),
            (PlanState::Pending, "run the benchmarks".into()),
        ];
        let s = render(&app, 80, 24);
        assert!(!s.contains("run the benchmarks"), "only the summary: {s}");
        for c in "/todo".chars() {
            key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        }
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let s = render(&app, 80, 24);
        assert!(
            s.contains("read the parser") && s.contains("run the benchmarks"),
            "{s}"
        );
        insta::assert_snapshot!("todo_pane_80x24", s);
    }

    #[test]
    fn permissions_modal_100x30() {
        use crate::modal;

        let mut app = App::new(test_info());
        started(&mut app);
        app.modal = Some(modal::permissions("ask", &[], &app.theme));
        let s = render(&app, 100, 30);
        insta::assert_snapshot!("permissions_modal_100x30", s);
        for (current, label) in [
            ("auto-edit", "auto-edit (legacy)"),
            ("read-only", "read-only (legacy)"),
            ("yolo", "yolo"),
        ] {
            app.modal = Some(modal::permissions(current, &[], &app.theme));
            let s = render(&app, 100, 30);
            assert!(s.contains(label), "the active policy stays visible: {s}");
        }
        app.modal = Some(modal::permissions("allow", &[], &app.theme));
        let s = render(&app, 60, 20);
        assert!(s.contains("ask") && s.contains("allow"), "{s}");
        assert!(
            !s.contains("auto-edit") && !s.contains("read-only") && !s.contains("yolo"),
            "{s}"
        );
    }

    #[test]
    fn plan_strip_and_turn_summary_80x24() {
        use quack_protocol::{CallId, ToolCallInfo, ToolResultSummary};
        let mut info = test_info();
        info.model = Some("charm/qwen3.8-flash".into());
        info.permission_mode = "allow".into();
        let mut app = App::new(info);
        started(&mut app);
        app.transcript
            .push(Entry::User("make the parser faster".into()));
        let call = |app: &mut App,
                    id: &str,
                    name: &str,
                    summary: &str,
                    ok,
                    result: &str,
                    detail: Option<&str>| {
            app.on_engine(Event::ToolCallBegin {
                call: ToolCallInfo {
                    call_id: CallId(id.into()),
                    name: name.into(),
                    summary: summary.into(),
                },
            });
            app.on_engine(Event::ToolCallEnd {
                call_id: CallId(id.into()),
                result: ToolResultSummary {
                    ok,
                    summary: result.into(),
                    detail: detail.map(Into::into),
                },
            });
        };
        call(
            &mut app,
            "1",
            "todo",
            "plan 1/3 done · profile the hot loop",
            true,
            "plan 1/3 done",
            Some("[x] read the parser\n[~] profile the hot loop\n[ ] cache token lookups"),
        );
        call(
            &mut app,
            "2",
            "shell",
            "shell cargo bench",
            false,
            "exit 101",
            Some("error: no bench target"),
        );
        app.transcript.push(Entry::Assistant(
            "There is no bench target yet; adding one.".into(),
        ));
        app.transcript.push(Entry::TurnSummary(
            "14s · 2 tools · 1.2k tokens out · 48 t/s".into(),
        ));
        app.context = Some((70_000, 104_000));
        app.speed = Some(48.0);
        insta::assert_snapshot!(render(&app, 80, 24));
    }

    #[test]
    fn plan_river_tracks_completed_steps_without_animating() {
        let mut app = App::new(test_info());
        started(&mut app);
        app.transcript.push(Entry::User("fix the parser".into()));
        app.on_engine(Event::Plan {
            checklist: "[~] Inspect the parser\n[ ] Change the parser\n[ ] Run tests".into(),
        });
        let start = render(&app, 80, 24);
        assert!(
            start.contains("_(•)>····· 0/3  ▶ Inspect the parser"),
            "{start}"
        );

        // Completing a later step moves the duck, even though the current step is unchanged.
        app.on_engine(Event::Plan {
            checklist: "[~] Inspect the parser\n[x] Change the parser\n[ ] Run tests".into(),
        });
        assert!(app.take_dirty(), "a checklist update must redraw");
        let progress = render(&app, 80, 24);
        assert!(
            progress.contains("~_(•)>···· 1/3  ▶ Inspect the parser"),
            "{progress}"
        );

        for _ in 0..120 {
            app.on_tick();
        }
        assert!(
            !app.take_dirty(),
            "the river must not request redraws between updates"
        );
        assert_eq!(render(&app, 80, 24), progress);

        app.on_engine(Event::Plan {
            checklist: "[x] Inspect the parser\n[x] Change the parser\n[x] Run tests".into(),
        });
        assert!(!render(&app, 80, 24).contains("  plan  "));
        app.execute("/todo");
        let done = render(&app, 80, 24);
        assert!(done.contains("~~~~~_(•)> 3/3"), "{done}");
    }

    #[test]
    fn plan_river_yields_to_step_text_on_narrow_terminals() {
        let mut app = App::new(test_info());
        started(&mut app);
        app.on_engine(Event::Plan {
            checklist: "[ ] 修正 parser and run tests\n[x] Inspect tests".into(),
        });
        let narrow = render(&app, 42, 24);
        assert!(
            narrow.contains("plan  1/2  · 修正 parser and run tests"),
            "{narrow}"
        );
        assert!(!narrow.contains(app.glyphs.assistant), "{narrow}");
        let wide = render(&app, 80, 24);
        assert!(
            wide.contains("~~_(•)>··· 1/2  · 修正 parser and run tests"),
            "{wide}"
        );
    }

    #[test]
    fn plan_river_respects_theme_ascii_and_reduced_motion() {
        use crate::theme::{Glyphs, Theme};
        use ratatui::style::Color;

        let mut app = App::new(test_info());
        app.on_engine(Event::Plan {
            checklist: "[x] Inspect the parser\n[~] Change the parser\n[ ] Run tests".into(),
        });
        for (theme, ascii, no_color, reduced_motion, water, duck) in [
            ("dark", false, false, false, Color::Cyan, Color::Yellow),
            (
                "light",
                false,
                false,
                true,
                Color::Blue,
                Color::Rgb(160, 110, 0),
            ),
            ("dark", true, true, true, Color::Reset, Color::Reset),
        ] {
            app.info.ui.ascii = ascii;
            app.info.ui.reduced_motion = reduced_motion;
            app.glyphs = Glyphs::new(ascii);
            app.theme = Theme::resolve(theme, no_color);
            let mut terminal = Terminal::new(TestBackend::new(64, 1)).expect("terminal");
            terminal
                .draw(|f| {
                    let area = f.area();
                    draw_plan(f, &app, area);
                })
                .expect("draw");
            let buf = terminal.backend().buffer();
            assert_eq!((buf[(8, 0)].symbol(), buf[(8, 0)].fg), ("~", water));
            assert_eq!((buf[(9, 0)].symbol(), buf[(9, 0)].fg), ("_", duck));
            let row = terminal.backend().to_string();
            let expected = if ascii {
                "~_(o)>.... 1/3  [~] Change the parser"
            } else {
                "~_(•)>···· 1/3  ▶ Change the parser"
            };
            assert!(row.contains(expected), "{row}");
        }
    }

    #[test]
    fn a_sub_agent_shows_under_its_task_and_in_tasks() {
        use crate::app::PlanState;
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{CallId, ToolCallInfo, ToolResultSummary};
        let mut app = started_app();
        started(&mut app);
        app.on_engine(Event::ToolCallBegin {
            call: ToolCallInfo {
                call_id: CallId("t1".into()),
                name: "Task".into(),
                summary: "explore: find the parser".into(),
            },
        });
        app.on_engine(Event::Plan {
            checklist: "[~] find the parser\n[ ] fix it".into(),
        });
        assert_eq!(app.plan[0].0, PlanState::Active);
        app.on_engine(Event::AgentStarted {
            id: "explore-1".into(),
            agent: "explore".into(),
            description: "find the parser".into(),
            call_id: CallId("t1".into()),
            model: "mock/small".into(),
        });
        let inner = |e: Event| Event::Agent {
            id: "explore-1".into(),
            inner: Box::new(e),
        };
        app.on_engine(inner(Event::ToolCallBegin {
            call: ToolCallInfo {
                call_id: CallId("explore-1/c1".into()),
                name: "grep".into(),
                summary: "grep fn parse".into(),
            },
        }));
        app.on_engine(inner(Event::ToolCallEnd {
            call_id: CallId("explore-1/c1".into()),
            result: ToolResultSummary {
                ok: true,
                summary: "3 matches".into(),
                detail: None,
            },
        }));
        app.on_engine(inner(Event::AssistantDelta {
            text: "Reading src/parse.rs".into(),
        }));
        let s = render(&app, 80, 24);
        assert!(
            s.contains("› grep fn parse · 3 matches") && s.contains("› Reading src/parse.rs"),
            "{s}"
        );
        app.on_engine(inner(Event::AssistantRewrite {
            text: "The parser is parse() in src/parse.rs:12.".into(),
        }));
        app.on_engine(Event::AgentFinished {
            id: "explore-1".into(),
            ok: true,
            summary: "2 steps".into(),
        });
        app.on_engine(Event::ToolCallEnd {
            call_id: CallId("t1".into()),
            result: ToolResultSummary {
                ok: true,
                summary: "2 steps".into(),
                detail: Some("The parser is parse() in src/parse.rs:12.".into()),
            },
        });
        let s = render(&app, 80, 24);
        assert!(
            s.contains("› The parser is parse() in src/parse.rs:12.")
                && s.contains("/tasks shows what it did"),
            "{s}"
        );
        for c in "/tasks".chars() {
            key(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
        }
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        let s = render(&app, 100, 30);
        assert!(
            s.contains("explore-1 · find the parser")
                && s.contains("mock/small")
                && s.contains("grep fn parse · 3 matches")
                && s.contains("src/parse.rs:12"),
            "{s}"
        );
    }

    #[test]
    fn question_and_plan_review_bars() {
        use crate::app::PlanState;
        use quack_protocol::{CallId, Question, QuestionOption, TurnId, Usage};
        let mut app = started_app();
        app.on_engine(Event::Questions {
            call_id: CallId("q1".into()),
            questions: vec![
                Question {
                    header: "Storage".into(),
                    question: "Where should the login tokens live?".into(),
                    options: vec![
                        QuestionOption {
                            label: "OS keyring".into(),
                            description: Some("safest; needs secret-tool on Linux".into()),
                        },
                        QuestionOption {
                            label: "A dotfile".into(),
                            description: None,
                        },
                    ],
                    recommended: Some(0),
                },
                Question {
                    header: String::new(),
                    question: "Which database?".into(),
                    options: Vec::new(),
                    recommended: None,
                },
            ],
        });
        let s = render(&app, 80, 24);
        assert!(
            s.contains("Storage · 1/2") && s.contains("(recommended)"),
            "{s}"
        );
        insta::assert_snapshot!("question_bar_80x24", s);

        app.asking = None;
        app.on_engine(Event::ModeChanged {
            work_mode: quack_protocol::WorkMode::Plan,
            approvals: quack_protocol::ApprovalPolicy::Ask,
        });
        app.plan = vec![
            (PlanState::Pending, "add the token store".into()),
            (PlanState::Pending, "wire the login command".into()),
            (PlanState::Pending, "test it against the mock server".into()),
        ];
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
        let s = render(&app, 100, 30);
        assert!(
            s.contains("Ready to carry out this plan?") && s.contains("wire the login command"),
            "{s}"
        );
        insta::assert_snapshot!("plan_review_bar_100x30", s);
    }

    #[test]
    fn plan_review_bar_displays_only_unfinished_followup_steps() {
        use quack_protocol::{TurnId, Usage};
        let mut app = started_app();
        app.on_engine(Event::ModeChanged {
            work_mode: quack_protocol::WorkMode::Plan,
            approvals: quack_protocol::ApprovalPolicy::Ask,
        });
        app.on_engine(Event::TurnStarted { turn: TurnId(1) });
        app.on_engine(Event::Plan {
            checklist: "[x] old finished step\n[ ] follow-up step one\n[ ] follow-up step two"
                .into(),
        });
        app.on_engine(Event::TurnComplete {
            turn: TurnId(1),
            usage: Usage::default(),
        });
        let s = render(&app, 100, 30);
        assert!(
            s.contains("Ready to carry out this plan?")
                && s.contains("follow-up step one")
                && s.contains("follow-up step two"),
            "{s}"
        );
        assert!(!s.contains("old finished step"), "{s}");

        app.on_engine(Event::TurnStarted { turn: TurnId(2) });
        app.on_engine(Event::Plan {
            checklist: "[x] old finished step\n[x] follow-up step one\n[x] follow-up step two"
                .into(),
        });
        app.on_engine(Event::TurnComplete {
            turn: TurnId(2),
            usage: Usage::default(),
        });
        let s = render(&app, 100, 30);
        assert!(!s.contains("Ready to carry out this plan?"), "{s}");
    }

    #[test]
    fn step_limit_bar() {
        use quack_protocol::TurnId;
        let mut app = started_app();
        app.on_engine(Event::TurnStarted { turn: TurnId(1) });
        app.on_engine(Event::StepLimitReached {
            turn: TurnId(1),
            limit: 100,
        });
        let s = render(&app, 80, 24);
        assert!(
            s.contains("Keep going?") && s.contains("Keep going for 100 more"),
            "{s}"
        );
        insta::assert_snapshot!("step_limit_bar_80x24", s);
    }

    #[test]
    fn turn_complete_adds_a_summary() {
        use quack_protocol::{TurnId, Usage};
        let mut app = App::new(test_info());
        app.on_engine(Event::TurnStarted { turn: TurnId(1) });
        app.on_engine(Event::TurnComplete {
            turn: TurnId(1),
            usage: Usage {
                prompt_tokens: 10,
                completion_tokens: 1234,
                cached_prompt_tokens: None,
                cache_write_tokens: None,
                reasoning_tokens: None,
            },
        });
        let Some(Entry::TurnSummary(s)) = app.transcript.last() else {
            panic!("no summary: {:?}", app.transcript);
        };
        assert!(s.ends_with("s · 1.2k tokens out"), "{s}");
    }

    #[test]
    fn slash_popup_and_transcript_ascii() {
        let mut info = test_info();
        info.ui.ascii = true;
        let mut app = App::new(info);
        started(&mut app);
        app.transcript
            .push(Entry::User("fix the failing test".into()));
        app.transcript.push(Entry::Assistant(
            "Looking at the test output now; the assertion compares a usize to a u32.".into(),
        ));
        app.input.set("/re");
        insta::assert_snapshot!(render(&app, 80, 24));
    }

    #[test]
    fn long_input_wraps_and_grows_the_box() {
        let mut app = App::new(test_info());
        started(&mut app);
        // 40 columns leave 36 for text.
        app.input
            .set("please read the failing test and then fix the bug it points at");
        let mut terminal = Terminal::new(TestBackend::new(40, 16)).expect("terminal");
        terminal.draw(|f| draw(f, &app)).expect("draw");
        let s = terminal.backend().to_string();
        assert!(s.contains("please read the failing test and "), "{s}");
        assert!(s.contains("  then fix the bug it points at"), "{s}");
        // The cursor follows the text onto the second row, just past its end.
        let pos = terminal.get_cursor_position().expect("cursor");
        assert_eq!((pos.x, pos.y), (1 + 2 + 29, 16 - 3));
    }

    #[test]
    fn arrows_and_clicks_move_the_cursor_in_wrapped_input() {
        use crossterm::event::{
            Event as TermEvent, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let mut app = App::new(test_info());
        started(&mut app);
        app.input.set("earlier prompt");
        app.input.submit();
        app.input
            .set("please read the failing test and then fix the bug it points at");
        let mut terminal = Terminal::new(TestBackend::new(40, 16)).expect("terminal");
        let mut cursor = |app: &App| {
            terminal.draw(|f| draw(f, app)).expect("draw");
            let p = terminal.get_cursor_position().expect("cursor");
            (p.x, p.y)
        };
        assert_eq!(cursor(&app), (32, 13));
        // Up moves to the first row, and only then reaches history.
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(cursor(&app), (32, 12));
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        assert_eq!(cursor(&app), (32, 13));
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        assert_eq!(app.input.text(), "earlier prompt");
        key(&mut app, KeyCode::Down, KeyModifiers::NONE);
        app.input
            .set("please read the failing test and then fix the bug it points at");
        cursor(&app);

        // A click on "read" puts the cursor there; one outside the box does nothing.
        let click = |app: &mut App, column, row| {
            app.on_terminal(TermEvent::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }));
        };
        click(&mut app, 10, 12);
        assert_eq!(cursor(&app), (10, 12));
        click(&mut app, 10, 2);
        assert_eq!(cursor(&app), (10, 12));
        key(&mut app, KeyCode::Char('x'), KeyModifiers::NONE);
        assert!(
            app.input.text().starts_with("please xread"),
            "{}",
            app.input.text()
        );
    }

    #[test]
    fn dragging_selects_and_copies_from_the_transcript_and_the_input() {
        use crossterm::event::{
            Event as TermEvent, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let mut app = App::new(test_info());
        started(&mut app);
        app.transcript
            .push(Entry::Assistant("alpha beta\n\ngamma delta".into()));
        app.input.set("hello brave world");
        let mut terminal = Terminal::new(TestBackend::new(40, 16)).expect("terminal");
        terminal.draw(|f| draw(f, &app)).expect("draw");
        // Where a word was drawn.
        let find = |terminal: &Terminal<TestBackend>, word: &str| -> (u16, u16) {
            let buf = terminal.backend().buffer();
            (0..buf.area.height)
                .find_map(|y| {
                    let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                    row.find(word)
                        .map(|x| (u16::try_from(row[..x].width()).expect("narrow"), y))
                })
                .expect(word)
        };
        let mouse = |app: &mut App, kind, (column, row): (u16, u16)| {
            app.on_terminal(TermEvent::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }));
        };
        let left = MouseButton::Left;

        // From "beta" back up past the duck to the row's start, then down to "delta".
        let beta = find(&terminal, "beta");
        let delta = find(&terminal, "delta");
        mouse(&mut app, MouseEventKind::Down(left), beta);
        mouse(&mut app, MouseEventKind::Drag(left), (0, beta.1));
        mouse(&mut app, MouseEventKind::Drag(left), (delta.0 + 4, delta.1));
        mouse(&mut app, MouseEventKind::Up(left), (delta.0 + 4, delta.1));
        assert_eq!(app.take_clipboard().as_deref(), Some("beta\n\ngamma delta"));
        terminal.draw(|f| draw(f, &app)).expect("draw");
        let buf = terminal.backend().buffer();
        let marked = |x, y| buf[(x, y)].modifier.contains(Modifier::REVERSED);
        assert!(marked(beta.0, beta.1) && marked(delta.0 + 4, delta.1));
        assert!(!marked(delta.0 + 5, delta.1) && !marked(beta.0 - 1, beta.1));
        let s = render(&app, 40, 16);
        assert!(s.contains("copied 17 chars"), "{s}");

        // A key lets go of it; a plain click copies nothing.
        key(&mut app, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(app.selection, None);
        mouse(&mut app, MouseEventKind::Down(left), beta);
        mouse(&mut app, MouseEventKind::Up(left), beta);
        assert_eq!(app.take_clipboard(), None);

        // In the input, the drag copies, and Ctrl-C copies again instead of quitting.
        let brave = find(&terminal, "brave");
        mouse(&mut app, MouseEventKind::Down(left), brave);
        mouse(&mut app, MouseEventKind::Drag(left), (brave.0 + 4, brave.1));
        mouse(&mut app, MouseEventKind::Up(left), (brave.0 + 4, brave.1));
        assert_eq!(app.take_clipboard().as_deref(), Some("brave"));
        key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.take_clipboard().as_deref(), Some("brave"));
        assert_eq!(app.selection, None);
        assert!(!app.should_quit());
        assert_eq!(app.input.text(), "hello brave world");
    }

    #[test]
    fn ctrl_shift_c_copies_selection_and_never_interrupts_running_turn() {
        use crossterm::event::{
            Event as TermEvent, KeyCode, KeyModifiers, ModifierKeyCode, MouseButton, MouseEvent,
            MouseEventKind,
        };
        let mut info = test_info();
        info.model = Some("charm-hyper/claude-sonnet-4-thinking".into());
        let mut app = App::new(info);
        started(&mut app);
        app.on_engine(Event::TurnStarted {
            turn: quack_protocol::TurnId(1),
        });
        app.on_engine(Event::AssistantDelta {
            text: "alpha beta\n\ngamma delta".into(),
        });
        app.input.set("draft");

        let mut terminal = Terminal::new(TestBackend::new(80, 16)).expect("terminal");
        terminal.draw(|f| draw(f, &app)).expect("draw");
        let find = |terminal: &Terminal<TestBackend>, word: &str| -> (u16, u16) {
            let buf = terminal.backend().buffer();
            (0..buf.area.height)
                .find_map(|y| {
                    let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                    row.find(word)
                        .map(|x| (u16::try_from(row[..x].width()).expect("narrow"), y))
                })
                .expect(word)
        };
        let mouse = |app: &mut App, kind, (column, row): (u16, u16)| {
            app.on_terminal(TermEvent::Mouse(MouseEvent {
                kind,
                column,
                row,
                modifiers: KeyModifiers::NONE,
            }));
        };
        let left = MouseButton::Left;
        let beta = find(&terminal, "beta");
        let delta = find(&terminal, "delta");

        // Highlighting while a turn runs copies on release and keeps the confirmation visible.
        mouse(&mut app, MouseEventKind::Down(left), beta);
        mouse(&mut app, MouseEventKind::Drag(left), (delta.0 + 4, delta.1));
        mouse(&mut app, MouseEventKind::Up(left), (delta.0 + 4, delta.1));
        assert_eq!(app.take_clipboard().as_deref(), Some("beta\n\ngamma delta"));
        let frame = render(&app, 80, 16);
        assert!(frame.contains("copied 17 chars"), "{frame}");

        // Standalone modifier presses must not clear the selection before the copy chord lands.
        key(
            &mut app,
            KeyCode::Modifier(ModifierKeyCode::LeftControl),
            KeyModifiers::CONTROL,
        );
        key(
            &mut app,
            KeyCode::Modifier(ModifierKeyCode::LeftShift),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert!(app.selection.is_some());

        // Ctrl-Shift-C (uppercase 'C') copies without interrupting the running turn.
        key(
            &mut app,
            KeyCode::Char('C'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(app.take_clipboard().as_deref(), Some("beta\n\ngamma delta"));
        assert!(app.take_ops().is_empty());
        assert_eq!(app.input.text(), "draft");

        // Ctrl-Shift-c (lowercase 'c') also copies without interrupting.
        mouse(&mut app, MouseEventKind::Down(left), beta);
        mouse(&mut app, MouseEventKind::Drag(left), (beta.0 + 3, beta.1));
        mouse(&mut app, MouseEventKind::Up(left), (beta.0 + 3, beta.1));
        assert_eq!(app.take_clipboard().as_deref(), Some("beta"));
        key(
            &mut app,
            KeyCode::Char('c'),
            KeyModifiers::CONTROL | KeyModifiers::SHIFT,
        );
        assert_eq!(app.take_clipboard().as_deref(), Some("beta"));
        assert!(app.take_ops().is_empty());

        // With no active selection, dedicated copy chords never interrupt, clear input, or quit.
        for (code, mods) in [
            (
                KeyCode::Char('C'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            (
                KeyCode::Char('c'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            (KeyCode::Char('c'), KeyModifiers::SUPER),
            (KeyCode::Insert, KeyModifiers::CONTROL),
        ] {
            key(&mut app, code, mods);
            assert_eq!(app.take_clipboard(), None);
            assert!(app.take_ops().is_empty());
            assert_eq!(app.input.text(), "draft");
            assert!(!app.should_quit());
        }

        // Plain Ctrl-C with a selection copies instead of interrupting or clearing input.
        mouse(&mut app, MouseEventKind::Down(left), beta);
        mouse(&mut app, MouseEventKind::Drag(left), (beta.0 + 3, beta.1));
        mouse(&mut app, MouseEventKind::Up(left), (beta.0 + 3, beta.1));
        assert_eq!(app.take_clipboard().as_deref(), Some("beta"));
        key(&mut app, KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(app.take_clipboard().as_deref(), Some("beta"));
        assert!(app.take_ops().is_empty());
        assert_eq!(app.input.text(), "draft");
    }

    #[test]
    fn streaming_while_selecting_holds_the_transcript_viewport_steady() {
        use crossterm::event::{
            Event as TermEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
        };
        let mut app = App::new(test_info());
        started(&mut app);
        app.on_engine(Event::TurnStarted {
            turn: quack_protocol::TurnId(1),
        });
        let initial: String = (1..=20)
            .map(|i| format!("- line-{i:02}"))
            .collect::<Vec<_>>()
            .join("\n");
        app.on_engine(Event::AssistantDelta { text: initial });

        let mut terminal = Terminal::new(TestBackend::new(60, 12)).expect("terminal");
        terminal.draw(|f| draw(f, &app)).expect("draw");
        assert_eq!(app.scroll.get(), 0);

        let target = {
            let buf = terminal.backend().buffer();
            (0..buf.area.height)
                .find_map(|y| {
                    let row: String = (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect();
                    row.find("line-18")
                        .map(|x| (u16::try_from(row[..x].width()).expect("narrow"), y))
                })
                .expect("line-18")
        };
        app.on_terminal(TermEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: target.0,
            row: target.1,
            modifiers: KeyModifiers::NONE,
        }));
        app.on_terminal(TermEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Drag(MouseButton::Left),
            column: target.0 + 6,
            row: target.1,
            modifiers: KeyModifiers::NONE,
        }));

        // More output arrives while dragging: the viewport pins so the target stays put.
        app.on_engine(Event::AssistantDelta {
            text: "\n- line-21\n- line-22\n- line-23".into(),
        });
        terminal.draw(|f| draw(f, &app)).expect("draw");
        assert_eq!(app.scroll.get(), 3);

        app.on_terminal(TermEvent::Mouse(MouseEvent {
            kind: MouseEventKind::Up(MouseButton::Left),
            column: target.0 + 6,
            row: target.1,
            modifiers: KeyModifiers::NONE,
        }));
        assert_eq!(app.take_clipboard().as_deref(), Some("line-18"));
    }

    #[test]
    fn ui_mouse_setting_applies_live_and_clears_selection() {
        let mut app = App::new(test_info());
        assert!(app.wants_mouse());
        app.selection = Some(crate::select::Selection::new(Pane::Transcript, (0, 0)));
        app.on_engine(Event::SettingSaved {
            key: "ui.mouse".into(),
            value: "false".into(),
            path: "/tmp/config.toml".into(),
            note: None,
        });
        assert!(!app.wants_mouse());
        assert_eq!(app.selection, None);
        app.on_engine(Event::SettingSaved {
            key: "ui.mouse".into(),
            value: "true".into(),
            path: "/tmp/config.toml".into(),
            note: None,
        });
        assert!(app.wants_mouse());
    }

    #[test]
    fn slash_popup_highlights_matched_letters() {
        let mut app = App::new(test_info());
        started(&mut app);
        app.input.set("/rw");
        let mut terminal = Terminal::new(TestBackend::new(80, 24)).expect("terminal");
        terminal.draw(|f| draw(f, &app)).expect("draw");
        let buf = terminal.backend().buffer();
        let row = (0..24)
            .find(|y| {
                (0..80)
                    .map(|x| buf[(x, *y)].symbol())
                    .collect::<String>()
                    .contains("/rewind")
            })
            .expect("rewind row");
        let slash = (0..80u16)
            .find(|x| buf[(*x, row)].symbol() == "/" && buf[(x + 1, row)].symbol() == "r")
            .expect("slash");
        let underlined = |x: u16| buf[(x, row)].modifier.contains(Modifier::UNDERLINED);
        let r = slash + 1;
        assert!(underlined(r), "r of /rewind is a match");
        assert!(!underlined(r + 1), "e is not");
        assert!(underlined(r + 2), "w is a match");
    }

    fn key(app: &mut App, code: crossterm::event::KeyCode, mods: crossterm::event::KeyModifiers) {
        app.on_terminal(crossterm::event::Event::Key(
            crossterm::event::KeyEvent::new(code, mods),
        ));
    }

    #[test]
    fn transcript_scrolls_to_top_and_bottom_and_holds_still_while_streaming() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut app = started_app();
        for n in 1..=40 {
            app.transcript.push(Entry::User(format!("message {n}")));
        }
        let s = render(&app, 80, 24);
        assert!(s.contains("message 40") && !s.contains("message 1 "), "{s}");
        assert!(!s.contains("more lines"), "no hint at the bottom");

        key(&mut app, KeyCode::Home, KeyModifiers::CONTROL);
        let s = render(&app, 80, 24);
        assert!(s.contains("message 1 ") && !s.contains("message 40"), "{s}");
        assert!(s.contains("more lines · ctrl-end"), "{s}");

        // New output arrives below: the top stays in view.
        app.transcript.push(Entry::Assistant("a new reply".into()));
        let s = render(&app, 80, 24);
        assert!(s.contains("message 1 "), "{s}");

        key(&mut app, KeyCode::PageDown, KeyModifiers::NONE);
        let s = render(&app, 80, 24);
        assert!(!s.contains("message 1 "), "{s}");

        key(&mut app, KeyCode::End, KeyModifiers::CONTROL);
        let s = render(&app, 80, 24);
        assert!(
            s.contains("a new reply") && !s.contains("more lines"),
            "{s}"
        );

        // Plain Home and End still move the input cursor.
        key(&mut app, KeyCode::Up, KeyModifiers::SHIFT);
        assert_eq!(app.scroll.get(), 1);
        key(&mut app, KeyCode::End, KeyModifiers::NONE);
        assert_eq!(app.scroll.get(), 1);
    }

    #[test]
    fn ctrl_t_shows_and_hides_thinking() {
        use crossterm::event::{KeyCode, KeyModifiers};
        let mut app = started_app();
        app.transcript.push(Entry::User("hi".into()));
        app.on_engine(Event::ReasoningDelta {
            text: "The user greets me; greet back.".into(),
        });
        app.transcript.push(Entry::Assistant("Hello!".into()));
        let s = render(&app, 80, 24);
        assert!(s.contains("thought for 6 words · ctrl-t shows"), "{s}");
        assert!(!s.contains("greet back"));

        key(&mut app, KeyCode::Char('t'), KeyModifiers::CONTROL);
        let s = render(&app, 80, 24);
        assert!(s.contains("The user greets me; greet back."), "{s}");

        app.execute("/thinking hide");
        let s = render(&app, 80, 24);
        assert!(!s.contains("greet back"), "{s}");
        assert!(s.contains("thought for 6 words"), "{s}");
    }

    #[test]
    fn thinking_levels_picker_updates_and_displays_the_active_models_choices() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{Op, SessionId, SessionInfo};
        let mut app = started_app();
        started(&mut app);
        let configured = |effort: Option<&str>, levels: &[&str]| Event::SessionConfigured {
            session: Box::new(SessionInfo {
                id: SessionId("test".into()),
                cwd: String::new(),
                model: Some("mock/tiny".into()),
                profile: "generic".into(),
                reasoning_effort: effort.map(str::to_owned),
                reasoning_efforts: levels.iter().map(|s| (*s).to_owned()).collect(),
            }),
        };
        app.on_engine(configured(Some("high"), &["low", "medium", "high"]));
        for (width, height) in [(80, 24), (100, 30), (140, 24)] {
            let s = render(&app, width, height);
            assert!(s.contains("mock/tiny · high thinking"), "{s}");
            if width == 140 {
                assert!(s.contains("Model · high thinking"), "{s}");
                insta::assert_snapshot!("thinking_sidebar_140x24", s);
            }
        }
        app.execute("/thinking");
        let s = render(&app, 100, 30);
        assert!(
            s.contains("high") && s.contains("current") && !s.contains("xhigh"),
            "{s}"
        );
        insta::assert_snapshot!("thinking_picker_100x30", s);
        // The current level starts selected; moving up chooses medium.
        key(&mut app, KeyCode::Up, KeyModifiers::NONE);
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            app.take_ops(),
            vec![Op::SetReasoningEffort {
                effort: Some("medium".into())
            }]
        );
        assert!(app.modal.is_none());
        app.execute("/thinking default");
        assert_eq!(
            app.take_ops(),
            vec![Op::SetReasoningEffort { effort: None }]
        );

        // An open picker refreshes when a model with different choices becomes active.
        app.execute("/thinking");
        app.on_engine(configured(None, &[]));
        let s = render(&app, 100, 30);
        assert!(s.contains("No selectable thinking levels"), "{s}");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(app.take_ops().is_empty());
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(!render(&app, 80, 24).contains("high thinking"));
        app.on_engine(configured(None, &["low", "high"]));
        assert!(render(&app, 80, 24).contains("default thinking"));

        app.info.ui.ascii = true;
        app.on_engine(configured(Some("none"), &["none", "low", "high"]));
        let s = render(&app, 140, 24);
        assert!(s.contains("Model - none thinking"), "{s}");
        app.info.model = Some(format!("mock/{}", "long-model-name-".repeat(8)));
        assert!(render(&app, 80, 24).contains("none thinking"));
        app.on_engine(configured(Some("off"), &["off", "high"]));
        app.execute("/thinking");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            app.take_ops(),
            vec![Op::SetReasoningEffort {
                effort: Some("off".into())
            }]
        );
    }

    #[test]
    fn welcome_duck_animates_and_honors_reduced_motion() {
        let mut app = App::new(test_info());
        started(&mut app);

        // Frame 0: static duck, closed beak, open eye, resting wave
        let s0 = render(&app, 80, 24);
        assert!(s0.contains("<(o )___"));
        assert!(s0.contains("._> /"));
        assert!(s0.contains("~~~~~~~~~~"));

        // Advance 56 frames (step 7): quack with open beak and water ripple
        for _ in 0..56 {
            app.on_tick();
        }
        let s_quack = render(&app, 80, 24);
        assert!(s_quack.contains("<(o )___"));
        assert!(s_quack.contains(". > /"));
        assert!(s_quack.contains("~≈~≈~≈~≈~≈"));

        // Advance to step 13 (frame 104 = 56 + 48 more): eye blink
        for _ in 0..48 {
            app.on_tick();
        }
        let s_blink = render(&app, 80, 24);
        assert!(s_blink.contains("<(- )___"));

        // With reduced motion, step is locked at 0
        let mut info_rm = test_info();
        info_rm.ui.reduced_motion = true;
        let mut app_rm = App::new(info_rm);
        started(&mut app_rm);
        for _ in 0..104 {
            app_rm.on_tick();
        }
        let s_rm = render(&app_rm, 80, 24);
        assert!(s_rm.contains("<(o )___"));
        assert!(s_rm.contains("._> /"));
        assert!(s_rm.contains("~~~~~~~~~~"));
    }

    fn subscription_setup() -> App {
        let mut app = App::new(test_info());
        app.on_engine(Event::StartupComplete);
        app.take_ops();
        app.on_engine(Event::Services {
            services: vec![quack_protocol::ServiceInfo {
                id: "plan".into(),
                name: "Test subscription".into(),
                group: "Hosted".into(),
                base_url: "https://plan.example/v1".into(),
                env: None,
                key: None,
                subscription: true,
                ready: false,
            }],
        });
        key(
            &mut app,
            crossterm::event::KeyCode::Tab,
            crossterm::event::KeyModifiers::NONE,
        );
        app
    }

    #[test]
    fn setup_subscription_sign_in_copies_url_and_continues_to_models() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::Op;
        let mut app = subscription_setup();
        assert!(render(&app, 100, 30).contains("needs sign-in"));
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            app.take_ops(),
            vec![Op::SignIn {
                id: Some("plan".into())
            }]
        );
        assert!(
            app.prompt.is_none(),
            "subscriptions never ask for an API key"
        );
        let url = "https://issuer.example/authorize?state=private-state";
        app.on_engine(Event::SignInUrl {
            id: "plan".into(),
            url: url.into(),
        });
        assert_eq!(app.take_browser().as_deref(), Some(url));
        let shown = render(&app, 100, 30);
        assert!(shown.contains("copy URL"), "{shown}");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.take_clipboard().as_deref(), Some(url));
        assert!(app.modal.is_some() && app.input.is_empty());
        app.on_engine(Event::SignInResult {
            id: "plan".into(),
            error: None,
        });
        assert_eq!(app.take_ops(), vec![Op::ListServices, Op::ListModels]);
        assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Models"));
        assert!(!render(&app, 100, 30).contains("private-state"));
    }

    #[test]
    fn setup_subscription_cancel_ignores_late_events_and_failure_allows_retry() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::Op;
        for cancel in [
            (KeyCode::Esc, KeyModifiers::NONE),
            (KeyCode::Char('c'), KeyModifiers::CONTROL),
        ] {
            let mut app = subscription_setup();
            key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
            app.take_ops();
            key(&mut app, cancel.0, cancel.1);
            assert_eq!(
                app.take_ops(),
                vec![Op::SignIn { id: None }, Op::ListServices]
            );
            app.on_engine(Event::SignInUrl {
                id: "plan".into(),
                url: "https://late.example".into(),
            });
            app.on_engine(Event::SignInResult {
                id: "plan".into(),
                error: None,
            });
            assert!(app.take_browser().is_none());
            assert!(app.take_ops().is_empty());
            assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Setup"));
        }
        let mut app = subscription_setup();
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        app.take_ops();
        app.on_engine(Event::SignInResult {
            id: "plan".into(),
            error: Some("Plan usage was declined.".into()),
        });
        assert_eq!(app.take_ops(), vec![Op::ListServices]);
        assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Setup"));
        assert!(app.prompt.is_none());
        let shown = render(&app, 100, 30);
        assert!(shown.contains("Plan usage was declined."), "{shown}");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(
            app.take_ops(),
            vec![Op::SignIn {
                id: Some("plan".into())
            }]
        );
    }

    #[test]
    fn first_run_offers_setup_and_adds_a_server_by_url() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{Op, SecretText, ServiceInfo};
        let none = KeyModifiers::NONE;
        let mut app = App::new(test_info());
        app.on_engine(Event::StartupComplete);
        // Setup opens on its own, and lists the services without probing this machine.
        assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Setup"));
        assert_eq!(app.take_ops(), vec![Op::ListServices]);
        app.on_engine(Event::Services {
            services: vec![ServiceInfo {
                subscription: false,
                id: "example".into(),
                name: "Example".into(),
                group: "Hosted".into(),
                base_url: "https://api.example.com/v1".into(),
                env: Some("EXAMPLE_KEY".into()),
                ready: false,
                key: Some(quack_protocol::KeySource::Env {
                    var: "EXAMPLE_KEY".into(),
                }),
            }],
        });
        key(&mut app, KeyCode::Tab, none);
        let s = render(&app, 100, 30);
        assert!(s.contains("needs a key"), "{s}");
        assert!(s.contains("Or set EXAMPLE_KEY"), "{s}");

        // Any server → its URL → its key.
        key(&mut app, KeyCode::Tab, none);
        key(&mut app, KeyCode::Enter, none);
        assert!(app.modal.is_none());
        for c in "gpu-box:8000/v1".chars() {
            key(&mut app, KeyCode::Char(c), none);
        }
        key(&mut app, KeyCode::Enter, none);
        assert!(render(&app, 100, 30).contains("must start with http://"));
        key(&mut app, KeyCode::Char('u'), KeyModifiers::CONTROL);
        app.on_terminal(crossterm::event::Event::Paste(
            "http://gpu-box:8000/v1\n".into(),
        ));
        key(&mut app, KeyCode::Enter, none);
        for c in "sk-secret".chars() {
            key(&mut app, KeyCode::Char(c), none);
        }
        let s = render(&app, 100, 30);
        assert!(s.contains("Add a server · 2 of 2"), "{s}");
        assert!(s.contains("•••••••••"), "{s}");
        assert!(!s.contains("sk-secret"), "{s}");
        assert!(!format!("{app:?}").contains("sk-secret"));
        key(&mut app, KeyCode::Enter, none);
        let ops = app.take_ops();
        assert_eq!(
            ops,
            vec![Op::AddServer {
                base_url: "http://gpu-box:8000/v1".into(),
                key: Some(SecretText("sk-secret".into())),
            }]
        );
        assert!(!format!("{ops:?}").contains("sk-secret"));
        assert!(
            app.input.is_empty(),
            "the key never reaches the input or its history"
        );

        // Once added, its models are offered.
        app.on_engine(Event::ProviderAdded {
            id: "gpu-box".into(),
            path: Some("providers.d/gpu-box.toml".into()),
            note: None,
        });
        assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Models"));
        assert!(app.prompt.is_none());
    }

    #[test]
    fn a_providers_key_is_replaced_from_providers_or_setup() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{KeySource, Op, ProviderModels, SecretText, ServiceInfo};
        let none = KeyModifiers::NONE;
        let mut info = test_info();
        info.model = Some("example/tiny".into());
        let mut app = App::new(info);
        app.on_engine(Event::StartupComplete);

        // /providers says where the key is read from; ⏎ asks for a new one.
        app.input.set("/providers");
        key(&mut app, KeyCode::Enter, none);
        assert_eq!(
            app.take_ops(),
            vec![Op::ListModels, Op::ListProviderConnections]
        );
        app.on_engine(Event::ModelCatalog {
            providers: vec![ProviderModels {
                id: "example".into(),
                available: true,
                note: "1 model".into(),
                models: Vec::new(),
                bundled: false,
                key: Some(KeySource::Keyring {
                    name: "example".into(),
                }),
            }],
            current: Some("example/tiny".into()),
        });
        let s = render(&app, 100, 30);
        assert!(s.contains("OS credential store"), "{s}");
        assert!(s.contains("(`example`)"), "{s}");
        assert!(s.contains("replace its key"), "{s}");
        assert!(!s.contains(" Advanced "), "{s}");
        app.on_engine(Event::ProviderConnections {
            connections: vec![quack_protocol::ProviderConnection {
                id: "example".into(),
                http_version: quack_protocol::ProviderHttpVersion::Http1Only,
                no_proxy: true,
                inherited_http1_only: false,
            }],
        });
        let s = render(&app, 100, 30);
        assert!(s.contains("HTTP      HTTP/1.1 only"), "{s}");
        assert!(s.contains("proxy     Direct connection"), "{s}");
        assert!(s.contains("^s connection settings"), "{s}");
        assert!(s.contains("^s connection"), "{s}");
        // Ctrl-S opens the grouped connection editor; Esc returns to Providers.
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        let s = render(&app, 100, 30);
        assert!(s.contains("Connection: example"), "{s}");
        assert!(s.contains("HTTP version ─"), "{s}");
        assert!(s.contains("HTTP/1.1 only"), "{s}");
        assert!(s.contains("Proxy ─"), "{s}");
        assert!(s.contains("Direct connection"), "{s}");
        // Blank separator line between the last HTTP option and the Proxy header.
        let lines: Vec<&str> = s.lines().collect();
        let neg_row = lines
            .iter()
            .rposition(|l| l.contains("Negotiate"))
            .expect("Negotiate row");
        let proxy_row = lines
            .iter()
            .position(|l| l.contains("Proxy ─"))
            .expect("Proxy header row");
        assert_eq!(proxy_row, neg_row + 2, "{s}");
        // Filtering by section name keeps only that section's items.
        for c in "proxy".chars() {
            key(&mut app, KeyCode::Char(c), none);
        }
        let filtered = render(&app, 100, 30);
        assert!(filtered.contains("Proxy ─"), "{filtered}");
        assert!(!filtered.contains("HTTP version ─"), "{filtered}");
        assert!(filtered.contains("Direct connection"), "{filtered}");
        // First Esc clears the filter; second Esc returns to Providers.
        key(&mut app, KeyCode::Esc, none);
        key(&mut app, KeyCode::Esc, none);
        let s = render(&app, 100, 30);
        assert!(s.contains("Providers"), "{s}");
        assert!(s.contains("HTTP      HTTP/1.1 only"), "{s}");
        key(&mut app, KeyCode::Enter, none);
        assert!(app.modal.is_none());
        let s = render(&app, 100, 30);
        assert!(s.contains("New example key"), "{s}");
        assert!(s.contains("provider file stays as it is"), "{s}");
        assert!(s.contains("replace"), "{s}");
        key(&mut app, KeyCode::Enter, none);
        assert!(render(&app, 100, 30).contains("paste the new key"));
        for c in "sk-rotated".chars() {
            key(&mut app, KeyCode::Char(c), none);
        }
        assert!(!render(&app, 100, 30).contains("sk-rotated"));
        key(&mut app, KeyCode::Enter, none);
        assert_eq!(
            app.take_ops(),
            vec![Op::SetProviderKey {
                id: "example".into(),
                key: SecretText("sk-rotated".into()),
            }]
        );
        // Replaced in place: no model to pick.
        app.on_engine(Event::ProviderAdded {
            id: "example".into(),
            path: None,
            note: None,
        });
        assert!(app.modal.is_none());
        assert!(render(&app, 100, 30).contains("replaced the example key"));

        // In /setup, ^s on a service whose key is set asks for a new one; esc just closes.
        app.input.set("/setup");
        key(&mut app, KeyCode::Enter, none);
        app.on_engine(Event::Services {
            services: vec![ServiceInfo {
                subscription: false,
                id: "hosted".into(),
                name: "Hosted".into(),
                group: "Hosted".into(),
                base_url: "https://api.example.org/v1".into(),
                env: Some("HOSTED_KEY".into()),
                ready: true,
                key: Some(KeySource::Env {
                    var: "HOSTED_KEY".into(),
                }),
            }],
        });
        key(&mut app, KeyCode::Tab, none);
        let s = render(&app, 100, 30);
        assert!(s.contains("read from the HOSTED_KEY"), "{s}");
        key(&mut app, KeyCode::Char('s'), KeyModifiers::CONTROL);
        let s = render(&app, 100, 30);
        assert!(s.contains("New hosted key"), "{s}");
        assert!(s.contains("To keep using HOSTED_KEY"), "{s}");
        key(&mut app, KeyCode::Esc, none);
        assert!(app.prompt.is_none());
        assert!(app.modal.is_none());
    }

    #[test]
    fn setup_is_not_offered_when_a_model_is_ready_and_esc_backs_out_of_a_prompt() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::Op;
        let mut info = test_info();
        info.model = Some("mock/tiny".into());
        let mut app = App::new(info);
        app.on_engine(Event::StartupComplete);
        assert!(app.modal.is_none());
        assert!(app.take_ops().is_empty());

        // Asked for, /setup looks for servers at once.
        app.input.set("/setup");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.take_ops(), vec![Op::DetectServers, Op::ListServices]);
        assert!(render(&app, 100, 30).contains("looking for servers"));

        app.input.set("/setup key example");
        app.modal = None;
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(render(&app, 100, 30).contains("example API key"));
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        assert!(render(&app, 100, 30).contains("paste the key"));
        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        assert!(app.prompt.is_none());
        assert_eq!(app.modal.as_ref().map(|m| m.title.as_str()), Some("Setup"));
    }

    #[test]
    fn project_scope_renders_in_config_modal_and_approval_bar() {
        use crate::app::Approval;
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_config::SettingRow;
        use quack_protocol::{ApprovalRequest, CallId};

        let mut info = test_info();
        info.project_status = ProjectStatus::Trusted;
        info.settings = vec![SettingRow {
            key: "model.default".into(),
            value: "\"mock/tiny\"".into(),
            origin: "user: ~/.config/quack/config.toml".into(),
            description: "Default model.".into(),
            is_bool: false,
            locked: false,
        }];
        let mut app = App::new(info);
        started(&mut app);

        app.input.set("/config");
        key(&mut app, KeyCode::Enter, KeyModifiers::NONE);
        key(&mut app, KeyCode::Char('l'), KeyModifiers::CONTROL);
        let config_render = render(&app, 100, 30);
        assert!(config_render.contains("scope: .quack"), "{config_render}");
        assert!(
            config_render.contains(".quack/config.toml"),
            "{config_render}"
        );

        key(&mut app, KeyCode::Esc, KeyModifiers::NONE);
        app.approval = Some(Approval {
            call_id: CallId("9".into()),
            request: ApprovalRequest::Shell {
                command: "cargo test".into(),
                cwd: ".".into(),
                outside_sandbox: false,
            },
            always: vec!["shell:cargo test".into()],
            always_scope: WriteScope::Project,
        });
        let approval_render = render(&app, 80, 24);
        assert!(
            approval_render.contains("always allow (.quack) saves shell:cargo test"),
            "{approval_render}"
        );
        assert!(approval_render.contains("^l scope"), "{approval_render}");
    }

    #[test]
    fn secret_approval_keeps_details_and_actions_visible_in_a_narrow_terminal() {
        use crossterm::event::{KeyCode, KeyModifiers};
        use quack_protocol::{ApprovalDecision, CallId, Op, SecretFinding, SecretFindingContext};

        let mut app = App::new(test_info());
        started(&mut app);
        let call_id = CallId::from("egress-1");
        app.on_engine(Event::ApprovalRequest {
            call_id: call_id.clone(),
            request: ApprovalRequest::EgressSecrets {
                provider: "work-model".into(),
                host: "api.example.test".into(),
                findings: (0..8)
                    .map(|i| SecretFinding {
                        id: format!("sec-{i}"),
                        kind: "github_token".into(),
                        location: "tool:shell".into(),
                        preview: "ghp_••••9876".into(),
                        fingerprint: "sha256:placeholder".into(),
                        line: Some(12),
                        context: Some(SecretFindingContext {
                            label: "Possible GitHub token".into(),
                            source: "Tool output: shell".into(),
                            impact: "If valid, it could grant access to GitHub resources within the token's permissions.".into(),
                        }),
                    })
                    .collect(),
            },
            always: vec![],
        });

        let shown = render(&app, 52, 23);
        for text in [
            "work-model (api.example.test)",
            "Possible GitHub token",
            "Tool output: shell",
            "scanned line 12",
            "If valid, it could grant access",
            "ghp_••••9876",
            "findings not shown",
            "unredacted text again",
            "redact & send",
            "send unredacted",
            "allow for session",
            "withhold",
        ] {
            assert!(shown.contains(text), "missing {text:?}:\n{shown}");
        }
        key(&mut app, KeyCode::Char('r'), KeyModifiers::NONE);
        assert_eq!(
            app.take_ops(),
            vec![Op::ApprovalResponse {
                call_id,
                decision: ApprovalDecision::RedactAndSend,
            }]
        );
    }
}
