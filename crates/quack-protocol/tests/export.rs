//! Regenerates the SPA's protocol bindings as one file,
//! `crates/quack-gui/ui/src/lib/protocol.ts` (docs/plan/15-desktop-client.md). Run with
//! `cargo test -p quack-protocol --features ts export`; CI checks the checked-in file is
//! current with `git diff --exit-code crates/quack-gui/ui/src/lib/protocol.ts`.

#![cfg(feature = "ts")]

use std::path::PathBuf;

use quack_protocol::{
    Answer, ApprovalDecision, ApprovalRequest, CallId, CheckStatus, CheckpointId, CommandInfo,
    ContextReport, ContextSegment, DetectedServer, DoctorCheck, EngineError, EngineErrorKind,
    Event, EventIndex, ExportFormat, Image, InputItem, InstructionFile, KeySource, LoadState,
    LspServerInfo, McpServerInfo, MemoryNote, ModelId, ModelInfo, Op, ProfileInfo, ProviderId,
    ProviderModels, PruneReport, Question, QuestionOption, ReplayItem, RequestStat, RewindPoint,
    SecretText, ServiceInfo, SessionId, SessionInfo, SessionSummary, SkillInfo, StatsReport,
    Subsystem, ToolCallInfo, ToolResultSummary, ToolStat, TurnId, TurnOverrides, Usage, WriteScope,
};
use ts_rs::TS;

/// `u64` and friends ride as JSON numbers on the wire (`serde_json`), so they must be
/// `number` here and not ts-rs's `bigint` default.
fn cfg() -> ts_rs::Config {
    ts_rs::Config::new().with_large_int("number")
}

/// One declaration per exported type: ids and small types first, then structs, then the
/// tagged enums clients match on. `decl()` renders `type X = ...;` without an access
/// modifier, and the SPA imports these types, so each is prefixed with `export`.
#[test]
fn export_protocol_ts() {
    let cfg = cfg();
    let types: Vec<(&'static str, String, String)> = vec![
        // ids and small types
        (
            "SessionId",
            SessionId::docs().unwrap_or_default(),
            SessionId::decl(&cfg),
        ),
        (
            "CallId",
            CallId::docs().unwrap_or_default(),
            CallId::decl(&cfg),
        ),
        (
            "ProviderId",
            ProviderId::docs().unwrap_or_default(),
            ProviderId::decl(&cfg),
        ),
        (
            "ModelId",
            ModelId::docs().unwrap_or_default(),
            ModelId::decl(&cfg),
        ),
        (
            "CheckpointId",
            CheckpointId::docs().unwrap_or_default(),
            CheckpointId::decl(&cfg),
        ),
        (
            "TurnId",
            TurnId::docs().unwrap_or_default(),
            TurnId::decl(&cfg),
        ),
        (
            "EventIndex",
            EventIndex::docs().unwrap_or_default(),
            EventIndex::decl(&cfg),
        ),
        (
            "SecretText",
            SecretText::docs().unwrap_or_default(),
            SecretText::decl(&cfg),
        ),
        (
            "ExportFormat",
            ExportFormat::docs().unwrap_or_default(),
            ExportFormat::decl(&cfg),
        ),
        (
            "WriteScope",
            WriteScope::docs().unwrap_or_default(),
            WriteScope::decl(&cfg),
        ),
        (
            "CheckStatus",
            CheckStatus::docs().unwrap_or_default(),
            CheckStatus::decl(&cfg),
        ),
        (
            "EngineErrorKind",
            EngineErrorKind::docs().unwrap_or_default(),
            EngineErrorKind::decl(&cfg),
        ),
        (
            "Subsystem",
            Subsystem::docs().unwrap_or_default(),
            Subsystem::decl(&cfg),
        ),
        (
            "LoadState",
            LoadState::docs().unwrap_or_default(),
            LoadState::decl(&cfg),
        ),
        // structs
        (
            "TurnOverrides",
            TurnOverrides::docs().unwrap_or_default(),
            TurnOverrides::decl(&cfg),
        ),
        (
            "Usage",
            Usage::docs().unwrap_or_default(),
            Usage::decl(&cfg),
        ),
        (
            "Image",
            Image::docs().unwrap_or_default(),
            Image::decl(&cfg),
        ),
        (
            "QuestionOption",
            QuestionOption::docs().unwrap_or_default(),
            QuestionOption::decl(&cfg),
        ),
        (
            "Question",
            Question::docs().unwrap_or_default(),
            Question::decl(&cfg),
        ),
        (
            "Answer",
            Answer::docs().unwrap_or_default(),
            Answer::decl(&cfg),
        ),
        (
            "ApprovalDecision",
            ApprovalDecision::docs().unwrap_or_default(),
            ApprovalDecision::decl(&cfg),
        ),
        (
            "ApprovalRequest",
            ApprovalRequest::docs().unwrap_or_default(),
            ApprovalRequest::decl(&cfg),
        ),
        (
            "ToolCallInfo",
            ToolCallInfo::docs().unwrap_or_default(),
            ToolCallInfo::decl(&cfg),
        ),
        (
            "ToolResultSummary",
            ToolResultSummary::docs().unwrap_or_default(),
            ToolResultSummary::decl(&cfg),
        ),
        (
            "ToolStat",
            ToolStat::docs().unwrap_or_default(),
            ToolStat::decl(&cfg),
        ),
        (
            "RequestStat",
            RequestStat::docs().unwrap_or_default(),
            RequestStat::decl(&cfg),
        ),
        (
            "ContextSegment",
            ContextSegment::docs().unwrap_or_default(),
            ContextSegment::decl(&cfg),
        ),
        (
            "PruneReport",
            PruneReport::docs().unwrap_or_default(),
            PruneReport::decl(&cfg),
        ),
        (
            "MemoryNote",
            MemoryNote::docs().unwrap_or_default(),
            MemoryNote::decl(&cfg),
        ),
        (
            "SkillInfo",
            SkillInfo::docs().unwrap_or_default(),
            SkillInfo::decl(&cfg),
        ),
        (
            "DoctorCheck",
            DoctorCheck::docs().unwrap_or_default(),
            DoctorCheck::decl(&cfg),
        ),
        (
            "CommandInfo",
            CommandInfo::docs().unwrap_or_default(),
            CommandInfo::decl(&cfg),
        ),
        (
            "InstructionFile",
            InstructionFile::docs().unwrap_or_default(),
            InstructionFile::decl(&cfg),
        ),
        (
            "ModelInfo",
            ModelInfo::docs().unwrap_or_default(),
            ModelInfo::decl(&cfg),
        ),
        (
            "KeySource",
            KeySource::docs().unwrap_or_default(),
            KeySource::decl(&cfg),
        ),
        (
            "SessionSummary",
            SessionSummary::docs().unwrap_or_default(),
            SessionSummary::decl(&cfg),
        ),
        (
            "RewindPoint",
            RewindPoint::docs().unwrap_or_default(),
            RewindPoint::decl(&cfg),
        ),
        (
            "ProfileInfo",
            ProfileInfo::docs().unwrap_or_default(),
            ProfileInfo::decl(&cfg),
        ),
        (
            "ProviderModels",
            ProviderModels::docs().unwrap_or_default(),
            ProviderModels::decl(&cfg),
        ),
        (
            "LspServerInfo",
            LspServerInfo::docs().unwrap_or_default(),
            LspServerInfo::decl(&cfg),
        ),
        (
            "McpServerInfo",
            McpServerInfo::docs().unwrap_or_default(),
            McpServerInfo::decl(&cfg),
        ),
        (
            "DetectedServer",
            DetectedServer::docs().unwrap_or_default(),
            DetectedServer::decl(&cfg),
        ),
        (
            "ServiceInfo",
            ServiceInfo::docs().unwrap_or_default(),
            ServiceInfo::decl(&cfg),
        ),
        (
            "SessionInfo",
            SessionInfo::docs().unwrap_or_default(),
            SessionInfo::decl(&cfg),
        ),
        (
            "EngineError",
            EngineError::docs().unwrap_or_default(),
            EngineError::decl(&cfg),
        ),
        (
            "ContextReport",
            ContextReport::docs().unwrap_or_default(),
            ContextReport::decl(&cfg),
        ),
        (
            "StatsReport",
            StatsReport::docs().unwrap_or_default(),
            StatsReport::decl(&cfg),
        ),
        // transcript, then the two enums the SPA matches on
        (
            "ReplayItem",
            ReplayItem::docs().unwrap_or_default(),
            ReplayItem::decl(&cfg),
        ),
        (
            "InputItem",
            InputItem::docs().unwrap_or_default(),
            InputItem::decl(&cfg),
        ),
        (
            "Event",
            Event::docs().unwrap_or_default(),
            Event::decl(&cfg),
        ),
        ("Op", Op::docs().unwrap_or_default(), Op::decl(&cfg)),
    ];

    let mut out = String::from(
        "// Generated from crates/quack-protocol by `cargo test -p quack-protocol --features ts export`. Do not edit.\n// CI checks this file with `git diff --exit-code crates/quack-gui/ui/src/lib/protocol.ts`.\n\n",
    );
    for (name, docs, decl) in &types {
        assert!(
            decl.starts_with(&format!("type {name} = ")),
            "unexpected declaration for {name}: {decl}"
        );
        out.push_str(docs);
        out.push_str("export ");
        out.push_str(decl);
        out.push('\n');
    }

    // The generated types must describe exactly what serde puts on the wire: tags as
    // literal fields with the serde renames, transparent string ids, tuples as arrays,
    // skipped fields optional. The wire side of this agreement is pinned by
    // `wire_shapes_agree_with_the_typescript_types` in src/lib.rs.
    assert!(
        out.contains(
            r#"{ "op": "user_turn", items: Array<InputItem>, overrides: TurnOverrides, }"#
        ),
        "Op must tag its variants with a literal op field:\n{out}"
    );
    assert!(
        out.contains(r#"{ "decision": "allow_once" }"#),
        "ApprovalDecision must tag with a literal decision field"
    );
    assert!(
        out.contains("export type SessionId = string;"),
        "string ids are transparent"
    );
    assert!(
        out.contains("export type SecretText = string;"),
        "SecretText is transparent"
    );
    assert!(
        out.contains("export type TurnId = number;"),
        "u64 ids ride as JSON numbers"
    );
    assert!(
        out.contains("call_id: CallId"),
        "ToolCallInfo keeps its field names"
    );
    assert!(
        out.contains(r#"{ "kind": "reasoning", text: string, }"#),
        "ReplayItem must carry the reasoning variant"
    );
    assert!(
        out.contains("detail?: string"),
        "ReplayItem::Tool.detail must be optional, never null"
    );
    assert!(
        out.contains("ttft_ms?: [number, number]"),
        "u32 tuples are [number, number]"
    );
    assert!(
        out.contains("sampling?: Array<[string, number]>"),
        "(String, f64) pairs are [string, number]"
    );
    assert!(
        out.contains("inner: Event"),
        "Event::Agent recurses through Box"
    );

    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../quack-gui/ui/src/lib/protocol.ts");
    let dir = path
        .parent()
        .expect("protocol.ts path has a parent directory");
    std::fs::create_dir_all(dir).expect("create the ui/src/lib directory");
    // Leave the file alone when unchanged, so editor timers and file watchers keep quiet.
    if std::fs::read_to_string(&path).is_ok_and(|current| current == out) {
        return;
    }
    std::fs::write(&path, &out).expect("write protocol.ts");
}
