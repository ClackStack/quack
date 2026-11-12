// Generated from crates/quack-protocol by `cargo test -p quack-protocol --features ts export`. Do not edit.
// CI checks this file with `git diff --exit-code crates/quack-gui/ui/src/lib/protocol.ts`.

/**
 * Session identifier (UUID v7 once `quack-store` lands).
 */
export type SessionId = string;
/**
 * Tool call identifier, as reported by the provider or synthesised.
 */
export type CallId = string;
/**
 * Provider id: the `id` key of a `providers.d/*.toml` file.
 */
export type ProviderId = string;
/**
 * Model id as the provider names it.
 */
export type ModelId = string;
/**
 * Checkpoint identifier.
 */
export type CheckpointId = string;
/**
 * Monotonic turn counter within a session.
 */
export type TurnId = number;
/**
 * Index of an event in a session log.
 */
export type EventIndex = number;
/**
 * Text the user typed that must not be shown: serialized as is, but `Debug` hides it, so
 * it never reaches a log.
 */
export type SecretText = string;
export type ExportFormat = "markdown" | "json";
export type WriteScope = "session" | "project" | "user";
export type CheckStatus = "ok" | "info" | "warn" | "fail";
export type EngineErrorKind = "config" | "provider" | "tool" | "internal";
/**
 * A subsystem that reports progress during startup. One row each in the startup panel.
 */
export type Subsystem = "config" | "project" | "provider" | "profile" | "tokenizer" | "instructions" | "skills" | "compat" | "memory" | "mcp" | "lsp";
/**
 * Progress of one subsystem.
 */
export type LoadState = { "state": "pending" } | { "state": "running", detail: string, } | { "state": "ready", detail: string, } | { "state": "needs_attention", detail: string, } | { "state": "failed", error: string, } | { "state": "disabled", reason: string, };
/**
 * What the agent may do. Planning never authorizes changes or test execution.
 */
export type WorkMode = "plan" | "edit";
/**
 * Approval defaults, ordered so the existing system policy can cap them.
 * Read-only and auto-edit remain available for existing configurations.
 */
export type ApprovalPolicy = "read-only" | "ask" | "auto-edit" | "allow" | "yolo";
export type HookCredentialChange = { "action": "keep" } | { "action": "remove" } | { "action": "replace", value: SecretText, } | { "action": "external", source: string, location: SecretText, };
/**
 * Per-turn overrides of session settings.
 */
export type TurnOverrides = { model?: string, profile?: string, };
export type Usage = { prompt_tokens: number, completion_tokens: number, cached_prompt_tokens?: number,
/**
 * Prompt tokens written to the server's cache, for servers that bill writes apart
 * (Anthropic). Part of `prompt_tokens`.
 */
cache_write_tokens?: number, };
/**
 * One image as it travels through the history: base64 (standard, padded), its media
 * type, and its pixel size when the file's header reported it.
 */
export type Image = {
/**
 * `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
 */
media_type: string,
/**
 * The whole file, base64-encoded.
 */
data: string, width: number | null, height: number | null, };
export type QuestionOption = { label: string, description?: string, };
/**
 * A question from the model to the user.
 */
export type Question = {
/**
 * A word or two naming the topic, e.g. `Storage`.
 */
header?: string, question: string,
/**
 * Choices; empty means a free-text answer.
 */
options?: Array<QuestionOption>,
/**
 * The option the model recommends.
 */
recommended?: number, };
/**
 * The user's answer to one [`Question`].
 */
export type Answer = { "answer": "picked", index: number, } | { "answer": "text", text: string, } | { "answer": "skipped" };
export type ApprovalDecision = { "decision": "allow_once" } | { "decision": "allow_session" } | { "decision": "always_allow", scope: WriteScope, } | { "decision": "deny" } | { "decision": "deny_with_feedback", feedback: string, };
export type ApprovalRequest = { "kind": "shell", command: string, cwd: string,
/**
 * The sandbox is on, but this command would run outside it.
 */
outside_sandbox?: boolean, } | { "kind": "edit", path: string, diff: string, } | { "kind": "tool", name: string, arguments: string, };
export type HookField = { id: string, label: string,
/**
 * text, url, integer, boolean, choice, or secret.
 */
kind: string, required: boolean, default: string, help: string, choices: Array<string>, advanced: boolean,
/**
 * Changing this value requires an explicit credential change.
 */
credential_scope: boolean, required_when: { [key in string]: Array<string> }, };
export type HookPreset = { id: string, label: string, help: string, values: { [key in string]: string }, choices: { [key in string]: Array<string> },
/**
 * Field templates expanded from the connection values, e.g. {web_url}/api/v4.
 */
derived: { [key in string]: string }, };
export type HookDescriptor = { version: number,
/**
 * Connection value containing the web base used to scope access checks.
 */
scope_field: string, fields: Array<HookField>, presets: Array<HookPreset>, };
export type HookConnection = { id: string, preset: string, enabled: boolean, values: { [key in string]: string }, };
export type HookConnectionInfo = { connection: HookConnection, credentials: { [key in string]: KeySource }, };
export type HookConnectionDraft = { connection: HookConnection, credentials: { [key in string]: HookCredentialChange }, };
export type HookSetupInfo = { id: string, revision: string, descriptor: HookDescriptor, connections: Array<HookConnectionInfo>, enabled: boolean, source: string, writable: boolean, problem: string | null, };
export type ToolCallInfo = { call_id: CallId, name: string,
/**
 * One-line description for the transcript, e.g. `edit src/main.rs`.
 */
summary: string, };
export type ToolResultSummary = { ok: boolean, summary: string,
/**
 * Full diff or output for display.
 */
detail?: string, };
export type ToolStat = { name: string, calls: number, failed: number, ms: number, };
/**
 * One model call, for the charts in `/stats`.
 */
export type RequestStat = { prompt: number, cached: number, completion: number, cost?: number,
/**
 * Absent for requests from before a resume, and for replies too short to time.
 */
ttft_ms?: number, tokens_per_sec?: number, };
export type ContextSegment = { name: string, tokens: number,
/**
 * One line, e.g. `12 messages, 3 pruned`.
 */
detail: string, };
export type PruneReport = { removed_tokens: number, items: Array<string>, };
export type MemoryNote = {
/**
 * `global` or `project`.
 */
scope: string, name: string, description: string, path: string, text: string, };
export type SkillInfo = { name: string, description: string,
/**
 * Where it was found, e.g. `user` or `project (.agents/skills)`.
 */
source: string, dir: string, };
export type DoctorCheck = { name: string, status: CheckStatus, detail: string, };
/**
 * A custom slash command (`commands/<name>.md`).
 */
export type CommandInfo = {
/**
 * Without the slash, e.g. `review` or `test:flaky`.
 */
name: string, description: string,
/**
 * What to type after the name, e.g. `[focus]`.
 */
args: string,
/**
 * The layer it came from.
 */
source: string, path: string, };
/**
 * One instruction file the model has seen.
 */
export type InstructionFile = {
/**
 * The path as shown to the model, relative to the workspace when inside it.
 */
source: string,
/**
 * In the system prompt (`false`: attached to a tool result when the model first
 * worked in its folder).
 */
in_prompt: boolean,
/**
 * Estimated tokens.
 */
tokens: number, text: string, };
export type ModelInfo = { id: string, alias?: string, context_window?: number,
/**
 * For showing, e.g. `$0.30 in · $1.20 out per 1M tokens`.
 */
price?: string,
/**
 * Selectable thinking levels from the provider file's `reasoning.efforts`, in picker
 * order. Empty means the model declares none.
 */
reasoning_efforts?: Array<string>, };
/**
 * Where a provider's `api_key` is read from.
 */
export type KeySource = { "from": "keyring", name: string, } | { "from": "env", var: string, } | { "from": "file", path: string, } | { "from": "cmd" } | { "from": "value" };
export type SessionSummary = { id: SessionId, title: string, model: string, turns: number,
/**
 * Human age of the last update, e.g. `3h ago`.
 */
age: string,
/**
 * Start date, `YYYY-MM-DD`.
 */
started: string,
/**
 * The session this client is in.
 */
current: boolean, };
/**
 * A user message the conversation can go back to.
 */
export type RewindPoint = {
/**
 * 1 for the last user message, 2 for the one before, and so on.
 */
back: number, text: string,
/**
 * Files changed by write and edit calls since this message, workspace-relative.
 */
files: Array<string>, };
/**
 * A profile as it would shape the current model's session.
 */
export type ProfileInfo = { id: string, description: string,
/**
 * Where its prompt and tool shapes come from.
 */
source: string,
/**
 * `auto` would pick it for the current model.
 */
matches: boolean, edit_format: string,
/**
 * The tool names the model would see.
 */
tools: Array<string>,
/**
 * The rendered system prompt, or why it does not render.
 */
system: string, };
export type ProviderModels = { id: string,
/**
 * False when the provider cannot be used right now (missing key, unreachable).
 */
available: boolean,
/**
 * Why it is unavailable, or a short status.
 */
note: string, models: Array<ModelInfo>,
/**
 * From the defaults bundle and not changed by the user: when unavailable it is an
 * offer (a key not set, a server not running), not a problem.
 */
bundled: boolean,
/**
 * Where its key is read from, when it takes one.
 */
key?: KeySource, };
/**
 * A language server from `lsp.d/` and its state (one per project folder it serves).
 */
export type LspServerInfo = { id: string,
/**
 * The command line.
 */
command: string, extensions: Array<string>,
/**
 * `not installed`, `idle`, `starting`, `ready`, `indexing`, `failed` or `exited`.
 */
state: string,
/**
 * The program's path, what it is working on, or why it failed.
 */
detail?: string,
/**
 * The project folder it serves, relative to the workspace.
 */
root?: string, errors: number, warnings: number,
/**
 * Last stderr lines.
 */
log: Array<string>, };
/**
 * An MCP server from `mcp.d/` and its state.
 */
export type McpServerInfo = { id: string,
/**
 * `stdio` or `http`.
 */
transport: string,
/**
 * The command line or the URL.
 */
target: string, description: string,
/**
 * `starting`, `ready`, `failed` or `stopped`.
 */
state: string,
/**
 * Why it failed.
 */
error?: string, tools: Array<string>,
/**
 * Prompts the user can run as `/<server>:<prompt>`.
 */
prompts?: Array<string>,
/**
 * Resource URIs and URI templates the model can read.
 */
resources?: Array<string>,
/**
 * Whether its tools are behind `tool_search` to save context.
 */
deferred: boolean,
/**
 * Estimated tokens of its tool definitions.
 */
tool_tokens: number,
/**
 * Last stderr lines.
 */
log: Array<string>, };
/**
 * A model server found on this machine.
 */
export type DetectedServer = {
/**
 * Suggested provider id, e.g. `ollama`.
 */
id: string,
/**
 * Human name, e.g. `Ollama`.
 */
name: string, base_url: string, api?: string,
/**
 * What this engine needs for tool calls, e.g. a server flag. May be empty.
 */
note?: string, models: Array<ModelInfo>,
/**
 * The configured provider that already points here, if any.
 */
configured?: string, };
/**
 * A hosted service from `providers.d` using an API key or subscription.
 */
export type ServiceInfo = {
/**
 * Uses browser sign-in instead of an API key.
 */
subscription: boolean, id: string, name: string,
/**
 * Heading it is listed under.
 */
group: string, base_url: string,
/**
 * The variable its key is read from, when it is read from one.
 */
env?: string,
/**
 * Its key is configured, or its saved sign-in allows plan usage.
 */
ready: boolean,
/**
 * Where its key is read from.
 */
key?: KeySource, };
export type SessionInfo = { id: SessionId, cwd: string,
/**
 * `<provider-id>/<model-id>`, if a model is selected.
 */
model: string | null, profile: string,
/**
 * Effective effort, when the model has an explicit default or session selection.
 */
reasoning_effort?: string | null,
/**
 * Supported levels for the active model, in picker order.
 */
reasoning_efforts?: Array<string>, };
export type EngineError = { kind: EngineErrorKind, message: string, };
export type ContextReport = {
/**
 * `<provider>/<model>`.
 */
model: string, window?: number, budget?: number,
/**
 * Requests estimated above this get pruned, then compacted.
 */
threshold?: number,
/**
 * Estimated prompt tokens of the next request.
 */
used: number,
/**
 * `[context] cache_strategy`.
 */
strategy: string, segments: Array<ContextSegment>,
/**
 * Caveats, e.g. that the history is in use by a running turn.
 */
note?: string, };
/**
 * Usage of one session. Token totals include earlier runs of a resumed session; timings
 * and tool counts cover only the time since it was opened (`resumed`).
 */
export type StatsReport = {
/**
 * The current `<provider>/<model>`.
 */
model: string, turns: number,
/**
 * Model calls; a turn makes one per step.
 */
requests: number,
/**
 * Requests whose server reported no usage, so their tokens are not counted.
 */
unreported: number,
/**
 * Prompt tokens summed over requests: what was billed, not the context size.
 */
prompt_tokens: number, completion_tokens: number,
/**
 * Prompt tokens the server served from its cache.
 */
cached_tokens: number,
/**
 * Requests that said how much was cached; zero when the server never does.
 */
cache_reported: number,
/**
 * US dollars, from the `cost` of the models used. `None` when none had one.
 */
cost?: number,
/**
 * Requests to models without a `cost`, left out of `cost`.
 */
unpriced: number,
/**
 * Time to first token, median and slowest.
 */
ttft_ms?: [number, number],
/**
 * Median generation speed, tokens per second.
 */
tokens_per_sec?: number,
/**
 * Since the session was opened: in the session, waiting for the model, running tools,
 * and waiting for the user to approve.
 */
open_ms: number, model_ms: number, tools_ms: number, approval_ms: number,
/**
 * Busiest first.
 */
tools: Array<ToolStat>, compactions: number,
/**
 * Tool results replaced by a placeholder to save room.
 */
pruned: number,
/**
 * A saved session that was resumed: timings start at the resume.
 */
resumed: boolean,
/**
 * The sampling settings the current model's requests carry (`temperature`, `top_p`,
 * ...), from its model entry and profile. Empty: the server's defaults apply.
 */
sampling?: Array<[string, number]>,
/**
 * Each request, oldest first; at most the last [`StatsReport::HISTORY`].
 */
history?: Array<RequestStat>, };
/**
 * A transcript entry of a resumed session.
 */
export type ReplayItem = { "kind": "user", text: string, } | { "kind": "assistant", text: string, } | { "kind": "tool", summary: string, ok: boolean, result: string,
/**
 * The full diff or output, for a collapsible card in the desktop client.
 */
detail?: string, } | { "kind": "reasoning", text: string, } | { "kind": "compacted" };
export type InputItem = { "type": "text", text: string, } | { "type": "image", path: string, } | { "type": "file", path: string, };
/**
 * A notification from the engine to its clients.
 */
export type Event = { "event": "hook_setup", request: number, info: HookSetupInfo, } | { "event": "hook_setup_saved", request: number, info: HookSetupInfo, note: string | null, } | { "event": "hook_setup_pending", request: number, } | { "event": "hook_setup_failed", request: number, message: string, } | { "event": "hook_check_result", request: number, status: string, message: string, } | { "event": "startup_progress", subsystem: Subsystem, state: LoadState, } | { "event": "startup_complete" } | { "event": "session_configured", session: SessionInfo, } | { "event": "session_title", title?: string, } | { "event": "model_catalog", providers: Array<ProviderModels>,
/**
 * `<provider>/<model>` in use, if any.
 */
current: string | null, } | { "event": "instructions", files: Array<InstructionFile>, } | { "event": "commands", commands: Array<CommandInfo>, } | { "event": "mcp_prompts", prompts: Array<CommandInfo>, } | { "event": "profiles",
/**
 * The profile in use.
 */
active: string,
/**
 * Set with `/profiles <id>` or `[model] profile`, rather than chosen per model.
 */
forced: boolean, profiles: Array<ProfileInfo>, } | { "event": "session_resumed", session: SessionInfo, items: Array<ReplayItem>, } | { "event": "session_list", sessions: Array<SessionSummary>, } | { "event": "setting_saved", key: string, value: string, path: string, note?: string, } | { "event": "hook_payload_choice", id: string, path: string, } | { "event": "hook_enabled", id: string, path: string, note?: string | null, } | { "event": "shutdown_complete" } | { "event": "config_reloaded" } | { "event": "project_trust", path: string, summary: string, changed: boolean, } | { "event": "turn_started", turn: TurnId, } | { "event": "turn_queued", waiting_for: string, } | { "event": "assistant_delta", text: string, } | { "event": "reasoning_delta", text: string, } | { "event": "assistant_rewrite", text: string, } | { "event": "tool_call_begin", call: ToolCallInfo, } | { "event": "tool_call_output_delta", call_id: CallId, chunk: string, } | { "event": "tool_call_end", call_id: CallId, result: ToolResultSummary, } | { "event": "approval_request", call_id: CallId, request: ApprovalRequest,
/**
 * The allow rules "always allow" would save (`shell:cargo test`); empty when no
 * rule can allow this call, so only "allow for this session" is offered.
 */
always?: Array<string>, } | { "event": "questions", call_id: CallId, questions: Array<Question>, } | { "event": "turn_complete", turn: TurnId, usage: Usage, } | { "event": "turn_aborted", turn: TurnId, reason: string, } | { "event": "context_update", used: number, budget: number, window?: number, cache_hit_tokens: number | null,
/**
 * Above this the next request is pruned, then compacted.
 */
threshold?: number, } | { "event": "context_pruned", report: PruneReport, } | { "event": "speed", ttft_ms: number,
/**
 * Output tokens per second; `None` when the reply was too short to measure.
 */
tokens_per_sec?: number, } | { "event": "compacting", reason: string, } | { "event": "context_report", report: ContextReport, } | { "event": "stats", report: StatsReport, } | { "event": "memory_notes", notes: Array<MemoryNote>,
/**
 * How full the memory index is, as a percentage of `memory.index_max_tokens`.
 */
index_fill: number, } | { "event": "doctor_report", checks: Array<DoctorCheck>, } | { "event": "mcp_servers", servers: Array<McpServerInfo>, } | { "event": "lsp_servers", servers: Array<LspServerInfo>, } | { "event": "servers_detected", servers: Array<DetectedServer>, } | { "event": "services", services: Array<ServiceInfo>, } | { "event": "sign_in_url", id: string, url: string, } | { "event": "sign_in_result", id: string, error: string | null, } | { "event": "provider_added", id: string, path?: string, note?: string | null, } | { "event": "skills", skills: Array<SkillInfo>, } | { "event": "rewind_points", points: Array<RewindPoint>, } | { "event": "rewound", back: number, text: string, } | { "event": "files_restored", restored: Array<string>, conflicts: Array<string>, } | { "event": "permission_mode", mode: string, } | { "event": "mode_changed", work_mode: WorkMode, approvals: ApprovalPolicy, } | { "event": "sandbox", off: boolean, } | { "event": "compacted", summary_tokens: number,
/**
 * History messages the summary replaced.
 */
replaced: number, } | { "event": "hook_ran", id: string,
/**
 * The hook's event, e.g. `user_prompt`.
 */
on: string, outcome: string, ms: number,
/**
 * What the hook said, or why it failed.
 */
message?: string,
/**
 * Estimated tokens of context it added.
 */
context_tokens: number, } | { "event": "agent_started", id: string, agent: string, description: string, call_id: CallId,
/**
 * `<provider>/<model>` it runs on.
 */
model: string, } | { "event": "agent", id: string, inner: Event, } | { "event": "agent_finished", id: string, ok: boolean,
/**
 * e.g. `12 steps · 34.5k tokens`.
 */
summary: string, } | { "event": "plan", checklist: string, } | { "event": "diagnostics", errors: number, warnings: number, } | { "event": "notice", message: string, } | { "event": "update_available", current: string, latest: string, } | { "event": "warning", message: string, } | { "event": "error", error: EngineError, };
/**
 * A request from a client to the engine.
 */
export type Op = { "op": "user_turn", items: Array<InputItem>, overrides: TurnOverrides, } | { "op": "interrupt" } | { "op": "approval_response", call_id: CallId, decision: ApprovalDecision, } | { "op": "answer_questions", call_id: CallId, answers: Array<Answer>, stop_asking?: boolean, } | { "op": "set_model", provider: ProviderId, model: ModelId, } | { "op": "set_reasoning_effort", effort: string | null, } | { "op": "list_models" } | { "op": "list_instructions" } | { "op": "list_profiles" } | { "op": "set_profile", id: string, } | { "op": "compact", focus?: string, } | { "op": "undo", checkpoint: CheckpointId | null, force: boolean, } | { "op": "resume_session", id: SessionId, } | { "op": "new_session" } | { "op": "list_sessions" } | { "op": "get_context" } | { "op": "get_stats" } | { "op": "set_permission_mode", mode: string, } | { "op": "set_work_mode", mode: WorkMode, } | { "op": "set_approval_policy", policy: ApprovalPolicy, } | { "op": "set_sandbox", off: boolean, } | { "op": "accept_plan",
/**
 * Empty preserves the current approval policy; legacy callers may choose one.
 */
mode: string, } | { "op": "run_doctor" } | { "op": "list_mcp" } | { "op": "restart_mcp", id: string, } | { "op": "list_lsp" } | { "op": "init_instructions" } | { "op": "detect_servers" } | { "op": "add_provider", id: string, base_url: string, api?: string, model: string, } | { "op": "list_services" } | { "op": "sign_in", id: string | null, } | { "op": "set_provider_key", id: string, key: SecretText, } | { "op": "add_server", base_url: string, key?: SecretText, } | { "op": "list_memory" } | { "op": "delete_memory", scope: string, name: string, } | { "op": "set_title", title: string, } | { "op": "save_setting", key: string, value: string, scope: WriteScope, } | { "op": "fork_session" } | { "op": "export", format: ExportFormat, path?: string, } | { "op": "list_rewind_points" } | { "op": "rewind", back: number, conversation: boolean, files: boolean, force: boolean, } | { "op": "reload_config" } | { "op": "trust_project", decision: string, } | { "op": "enable_hook", id: string, replace_payload?: boolean | null, } | { "op": "inspect_hook_setup", id: string, request: number, } | { "op": "save_hook_setup", id: string, request: number, revision: string, connections: Array<HookConnectionDraft>,
/**
 * None keeps the current enabled state.
 */
enable: boolean | null, replace_payload: boolean | null, } | { "op": "check_hook_setup", id: string, request: number, revision: string, connection: HookConnectionDraft, url: string, } | { "op": "cancel_hook_check", request: number, } | { "op": "shutdown" };
