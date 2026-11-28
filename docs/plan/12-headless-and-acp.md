# 12 — Headless Mode & ACP

Both are clients of the engine protocol ([00](00-architecture.md#engine-protocol)), so the agent loop is shared.

## Headless (`quack exec`)

For CI, scripts, git hooks, and wrapping by other tools.

```
quack exec "fix the failing test in crates/core" \
  --model corp-vllm/qwen3-coder \
  --mode edit \
  --approvals allow \
  --output-format stream-json \
  --max-steps 50 \
  --resume <id> | --continue
echo "summarize this diff" | git diff | quack exec -    # prompt + stdin
```

Aliases: `quack -p "<prompt>"` / `--print`.

### Output formats

| Format | Content |
|---|---|
| `text` (default) | Final assistant message only, to stdout. Progress goes to stderr if `-v` |
| `json` | One object at the end: final message, session id, usage, tool calls summary, exit reason |
| `stream-json` | NDJSON of engine events (a stable, documented subset). The schema is versioned |

We'll evaluate making `stream-json` compatible with Claude Code's stream-json format, as maki does, so existing wrappers work unchanged. Check its licence and spec stability first. If not, publish our own JSON Schema.

### Behaviour

- New sessions use the same Plan + Allow defaults as interactive sessions. Pass `--mode edit` to permit changes and test execution. Anything that still needs approval is **denied** in headless mode, with feedback to the model. Explicit ask rules and approval for leaving the sandbox still apply; approvals cannot override Plan restrictions. See the [user guide](../modes.md).
- `ask_user` and MCP elicitation are unavailable unless `--allow-elicitation` is given, in which case they read from the TTY if one is present.
- Exit codes: `0` success, `1` agent error / max steps, `2` config error, `3` provider unreachable, `4` denied by policy, `130` interrupted.
- Sessions are saved like interactive ones (`--no-session` to skip).
- Startup output appears only with `-v`, as structured lines.

## ACP (Agent Client Protocol)

[ACP](https://agentclientprotocol.com) lets editors (Zed, and others adopting it) drive an agent over stdio JSON-RPC. The editor owns the UI, and the agent streams updates, tool calls and permission requests.

### Plan

- Crate `quack-acp`, cargo feature `acp`, with its own wire types (see "As implemented" for why not the official crates).
- `quack acp` runs as an ACP agent on stdio.
- Mapping:

| ACP | Engine |
|---|---|
| `initialize` | Capabilities: load session, prompt content types (text, image, resource links), MCP passthrough |
| `session/new` (with cwd, MCP servers from the editor) | Start session; editor-supplied MCP servers merged with `mcp.d/` |
| `session/load` | `Op::ResumeSession` |
| `session/prompt` | `Op::UserTurn` |
| `session/cancel` | `Op::Interrupt` |
| `session/update` notifications | `AssistantDelta`, `ReasoningDelta`, `ToolCallBegin/End`, plans (todo) |
| `session/request_permission` | `ApprovalRequest` ↔ `ApprovalResponse` |
| Client `fs/read_text_file`, `fs/write_text_file` | Optional: route file I/O through the editor so unsaved buffers are seen. Behind a setting |
| Client terminal methods | Optional: run `shell` in the editor's terminal |

- Model and profile selection is exposed through ACP session modes/config options where the spec allows. Otherwise it comes from config.
- Test with Zed, plus the reference client from the ACP repo.

maki (`maki-acp`) and goose both implement ACP and are the main references.

### As implemented (2026-10-24)

- **No new crates.** `quack-acp/src/wire.rs` holds serde types for the messages quack reads (protocol version 1), replies are built with `json!`, and `rpc.rs` is about 120 lines of newline-delimited JSON-RPC over tokio. The official crates were measured first: the `agent-client-protocol` SDK is about 14k lines of its own connection framework, and `agent-client-protocol-schema` (4 more crates per OS) turns on serde_json's `preserve_order`, which through feature unification would change the key order of every request body sent to models, and differently in builds without `acp`. The wire names were taken from schema 1.12.0.
- **One engine per ACP session.** `quack acp` loads the config of the folder it starts in, as every command does, then each `session/new` loads the config of the session's `cwd` with the same command line (`-m`, `--profile`, `-c`, `--mode`, `--approvals`) through a `Spawn` closure from `quack-cli`. Sessions are saved as usual, so `session/load` is `Resume::Id`, and the saved transcript is replayed as `user_message_chunk`, `agent_message_chunk` and completed `tool_call` updates before the response.
- **Editor MCP servers** go into `LoadOptions::mcp` and load after every layer's `mcp.d` (replacing a file with the same id), with the policy allowlist applied. stdio and streamable HTTP are taken; SSE servers are skipped with a log line, since `rmcp` here speaks streamable HTTP only. `/reload` keeps them, because a reload re-applies the `LoadOptions`.
- **Mapping as built:**
  - `session/prompt` → `Op::UserTurn`, answered on `TurnComplete` (`end_turn`) or `TurnAborted` (`cancelled` after `session/cancel`, otherwise a JSON-RPC error with the turn's error). The first prompt waits for MCP startup, as `quack exec` does. A second prompt while one runs is refused.
  - Text, reasoning, `ToolCallBegin`/`End` (kind guessed from the tool name, so every profile's names work; output or diff as fenced content), `Plan`, `ContextUpdate` (as `usage_update`), `PermissionMode` (as `current_mode_update`), and `Compacting` and warnings during a turn (as thought text, so they stay out of the answer).
  - `ApprovalRequest` → `session/request_permission` with allow once, allow for this session (`AllowSession`, never written to config) and deny. It runs on its own task, so a cancel meanwhile still gets through; a cancelled or failed request is a denial.
  - Work modes: `plan` and `edit`; the independent `approvals` config option offers Ask/Allow. An active legacy policy (or yolo from the command line) appears only while selected, because the current value must be among the options. `ModeChanged` updates both controls. Legacy `session/set_mode` approval names still work without leaving Plan. Leaving Plan does not run it; the user says so in the next prompt.
  - The model is a `session/set_config_option` select (category `model`), filled from `Op::ListModels` when the session starts: the available providers' models.
  - `ask_user` questions are answered as skipped, as in `quack exec`. ACP's elicitation is newer than most clients; it is the way to add them.
- **Completed (2026-11-26):**
  - **Images:** `promptCapabilities.image = true`; inline base64 `image` blocks decode into a session-scoped temp folder (removed on session drop) and flow as `InputItem::Image` (plus `file://` image URIs).
  - **Slash commands & session titles:** `Event::Commands` and `Event::McpPrompts` emit `available_commands_update`; `Event::SessionTitle` emits `session_info_update`.
  - **`session/list`:** advertised under `sessionCapabilities.list` and served from `quack_core::list_sessions` with RFC 3339 `updatedAt`.
  - **Sub-agent tool calls:** `Event::AgentStarted` / `Event::Agent` / `Event::AgentFinished` stream sub-agent tool calls (`[explore] read ...`) and progress to the editor.
  - **Editor `fs/*` and `terminal/*` delegation:** opt-in via `[tools] acp_client_fs` and `[tools] acp_client_terminal` when the editor advertises `clientCapabilities.fs` / `clientCapabilities.terminal`, wired through `ClientBridge` so `read`/`write` see unsaved editor buffers and foreground `shell` commands run inside the editor terminal.
- **Tests:** `quack-acp/tests/acp.rs` drives `serve` over an in-memory pipe against the real engine and a wiremock model: initialize, a streamed turn, an approval answered by the editor that writes the file, `session/load` replay in a new connection, cancel, unknown methods/sessions, inline images + commands + titles + `session/list`, editor `fs/*` + `terminal/*` delegation, and sub-agent tool-call streaming. `convert.rs` unit-tests tool kinds, plans, `file://` paths (Windows drive letters), fences, base64 images and editor MCP servers. Checked by hand over real stdio; not yet run inside Zed.

## Open questions

- Should a **local socket server** (daemon) be offered so the TUI can attach to a headless-started session? This is deferred; the protocol allows it.
- Should we also support the **A2A** protocol for agent-to-agent use? Proposed: not planned.
