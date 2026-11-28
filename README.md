# _(•)> quack

```
   __
<(o )___
 ( ._> /
  `---'
```

A fast, terminal-native coding agent designed for open-weight models. Run it with local servers
(**llama.cpp, Ollama, vLLM, SGLang, LM Studio, Docker Model Runner**) or hosted services (**OpenRouter,
Groq, Cerebras, DeepSeek, Fireworks, Together, Alibaba Cloud, Google Gemini, xAI, and more**). It
also works with Claude and OpenAI APIs or ChatGPT accounts.

Written in Rust with a full-screen Ratatui UI, quack delivers sub-10ms startup, OS-level
sandboxing, AST-aware command security, Tree-sitter syntax validation, and background LSP code
intelligence.

[![License](https://img.shields.io/badge/license-Apache--2.0-blue.svg)](LICENSE)
[![Status](https://img.shields.io/badge/status-active%20development-green.svg)](docs/plan/roadmap.md)

---

## Quick Start

### 1. Install

The installer asks what to install: both the desktop GUI (`quack-gui`) and the terminal UI (`quack`), the default, or just one (pass `--tui` / `--gui`, `-Tui` / `-Gui` on Windows, to skip the question). They are separate apps sharing one core, config, sessions, memory, and prompt history. Re-running the installer defaults to updating whatever is already installed. On Linux it updates existing `.deb`/`.rpm` installations using that same package format (never silently falling back to the archive), and on fresh installs uses `.deb`/`.rpm` when the machine can, falling back to the self-contained archive in `~/.local`; `--pkg archive` forces the archive.

**Linux and macOS:**
```sh
curl -fsSL https://quack.clackstack.dev/install.sh | sh
```

**Windows (PowerShell):**
```powershell
irm https://quack.clackstack.dev/install.ps1 | iex
```

*Prebuilt `.deb`, `.rpm`, `.msi`, `.AppImage`, `.dmg`, `-setup.exe`, Homebrew `quack.rb` formula, keyless GitHub build provenance attestations, and platform archives are available on the [releases page](https://github.com/ClackStack/quack/releases).*

### 2. Run

```sh
quack      # terminal UI
quack-gui  # desktop app
```

On first launch, the interactive `/setup` wizard helps you configure a model in seconds:
- **Local servers**: Automatically detects running instances of Ollama, llama.cpp, vLLM, LM Studio, SGLang, or Docker Model Runner.
- **Hosted providers**: Configure API keys for OpenRouter, DeepSeek, Fireworks, and others (securely saved in your OS keyring; on headless Linux such as WSL, where no Secret Service runs, in a file only your user can read instead). Setup writes only a minimal override to your config, so new models a quack update brings in the defaults bundle reach you on their own; if your own file pins a model list that has moved on, `quack doctor` says so.
- **Subscriptions**: In **Hosted services**, choose **ChatGPT plan** to sign in through your browser, allow plan usage, and pick a model. This also works in `quack setup`; `quack login chatgpt` remains available for standalone sign-in.
- **Custom endpoints**: Connect any OpenAI-compatible server URL.

ChatGPT `gpt-6.1-sol` has been [evaluated live](evals/results/2026-11-11/README.md).
The `codex` profile is selected automatically for the current Sol, Astra, Luna and Terra
model ids (6.2 and 6.1 generations) on ChatGPT
and OpenAI providers; use `--profile generic` to override it. It includes tracked
reads before patches, sub-agents and image viewing on vision-capable models.
It prefers Quack's built-in `grep` and `glob` for routine search, so neither
`rg` nor shell `grep` needs to be installed.
All shipped profiles have mock-tested tracked-read/edit workflows; incompatible
exclusive-profile edit-format overrides report an actionable startup error.
[An upstream Codex review](evals/results/2026-11-11/codex-comparison.md) informed
compatibility fixes; Astra passed a [single smoke task](evals/results/2026-11-11/terra-astra.md),
and Luna passed all four runs with `codex` in a [targeted profile comparison](docs/plan/roadmap.md#proprietary-model-apis-2026-10-19).
Terra passed four tasks with each of three edit formats. `apply_patch` used the fewest model requests in that sample. [Terra comparison](evals/results/2026-11-18/terra-edit-formats/README.md).
Evaluations consume plan allowance or API credit, depending on the provider. `quack eval`
checkpoints its JSON after each trial; `--stop-on-incomplete` stops after a result with an
execution error or missing accounting. See the [offline preflight and pilot checklist](evals/README.md#before-a-paid-profile-pilot).

The bundled OpenAI and ChatGPT providers use native Responses compaction for long
conversations (preserving session context across the checkpoint), keep histories strictly
append-only with delta-only reminders for warm KV prefix caches, and set `compact_at = 250000`
on 500k models to compact below the 272k long-context cliff. Quack replays the encrypted
checkpoint after resume and uses a text summary (routed through `[model] small` at minimal
reasoning effort when configured) for focused `/compact <focus>` requests or if the native
endpoint fails. Set `native_compaction = false` in a provider override to select text compaction.

The bundled `chatgpt` and `openai` providers use Responses WebSockets for conversation
turns. Quack keeps the full local history and sends only new items while the connection
can continue it. If the upgrade is refused, quack uses HTTP streaming. Set
`responses_websocket = false` in a provider override to use HTTP for every turn.

Prompt caching is tuned across hosted and local providers: histories stay strictly
append-only (including across `/resume`, which replays the pinned opening `<context>`),
MCP tool schemas stay deterministically ordered across reconnects and deferred search,
Anthropic/OpenRouter (`anthropic/*`)/Alibaba DashScope emit `cache_control` breakpoints
with optional TTL, Moonshot/Kimi/Fireworks/Mistral/OpenRouter carry sticky routing keys,
and Ollama sends `keep_alive = "15m"` by default. Pass `--cache-key <KEY>` (or set
`QUACK_PROMPT_CACHE_KEY` / `[context] cache_key`) to share cache routing affinity across
parallel `quack exec` or CI runs on the same repository.

### 3. Common Commands

| Command | Description |
|---|---|
| `quack` | Launch interactive terminal UI (supports `--worktree <name>`, `--continue`, `-s <id>`) |
| `quack setup` | Terminal wizard to add providers, keys, subscriptions, and models |
| `quack doctor` | Diagnose environment, policy locks, audit log, sandbox, tools, and provider connections |
| `quack config check\|show\|paths\|schema` | Validate config layers, inspect effective values with `--origin`, print paths, or export JSON schemas |
| `quack trust [trusted\|settings-only\|ignored]` | Inspect or decide whether the current repo's `.quack/` loads |
| `quack login [provider]` / `quack logout` | Sign in to or revoke a browser-authenticated provider (`[auth]`, e.g. ChatGPT plan) |
| `quack mcp ...` | List, add, enable, disable, remove, or OAuth sign in/out of MCP servers (`list`, `add`, `enable`, `disable`, `remove`, `login`, `logout`) |
| `quack sessions [prune]` | List saved sessions, or prune older ones (`--older-than 30d`, `--all`, `--dry-run`) |
| `quack exec "..."` | Run a single turn headlessly for scripts or CI pipelines (`-p` shorthand) |
| `quack acp` | Start an Agent Client Protocol server for Zed, JetBrains, Neovim, or VS Code |
| `quack gui [PATH...]` | Launch `quack-gui` on one or more folders (merging with restored tabs unless `--no-restore`; also `/gui` inside the TUI) |
| `quack-gui [PATH...]` | Open the desktop app directly: the same engine in a window, one tab per folder |
| `quack keyring set\|delete <id>` | Store or remove an API key securely in the OS credential store |
| `quack update` | Say whether a new version is out and how to get it; changes nothing |
| `quack catalog refresh` | Download current model prices and context windows |

Keys can also be kept out of the keyring entirely: `api_key = { env = "VAR" }` in the provider file reads a shell variable, `{ file = "path" }` a file (mode 0600 recommended), and `{ cmd = [..] }` a command. On Linux without a running Secret Service (WSL, servers, containers), `quack setup` and the TUI save keys to a file under your config folder automatically and say so.

---

## Highlights

- **Open-weight first**: Built from the ground up for open models with family profiles (Qwen, DeepSeek, GLM, Kimi, Mistral), grammar-guided decoding, and reasoning preservation.
- **Fast and lightweight**: Native Rust binary, sub-10ms startup, low memory footprint, and zero telemetry or implicit network egress.
- **Sandboxed by default**: Commands run in an OS-level sandbox (Landlock and seccomp on Linux, Seatbelt on macOS, Restricted Token + Low Integrity + Job Objects on Windows) restricted to workspace writes with no network access (on Windows, writes only: the network is not restricted).
- **AST-aware command security**: Tree-sitter parses Bash and PowerShell commands down to AST nodes, ensuring compound commands, subshells, and aliases cannot bypass deny rules.
- **Compiler and syntax aware**: Immediate Tree-sitter syntax validation on edits and background LSP diagnostics that guide the model to fix type or compile errors immediately.
- **Smart context distillation**: Automatically folds passing test sequences (Cargo, Pytest, Go, Jest) and verbose build logs, keeping context compact and prompt caches warm.
- **Transactional multi-file refactoring**: Atomic multi-file changes automatically roll back if any write fails. Revert edits instantly with `/undo` or `/rewind`.
- **Plan with fewer prompts**: Plan + Allow reads and searches automatically, then presents proposed changes and test commands for review. Accepting the plan starts implementation with the same approval policy. A separate planner model is optional.
- **Extensible with MCP & LSP**: Native Model Context Protocol (stdio & HTTP) support, background language servers for 10+ languages, sub-agents, generic standard Agent Skills (`SKILL.md`), and custom slash commands.
- **Editor integration (ACP)**: Connect quack directly into your editor panel via the Agent Client Protocol.
- **Desktop app**: `quack-gui` opens the same engine in a native window (`quack gui [PATH...]` or `/gui` from the TUI) with persistent multi-tab session restore (`[ui] gui_restore_tabs`) — first-run provider setup (API keys or browser subscription sign-in), project trust, a clickable **Network & Sandbox Boundary** header passport with strict-offline toggle, **Pre-Egress Secret Shield** controls (`redact-secret`), and live **Session Egress Ledger**, a `Cmd/Ctrl-K` **Command Palette** and built-in slash command dispatch, streamed transcript with tree-sitter fenced code highlighting and inline headings, turn rewind (`Files + conversation`, `Conversation only`, `Files only`, external-conflict overwrite) and *Try another model* session branching, collaborative approvals (`Redact & send` on credential findings, `DenyWithFeedback`, scoped `AlwaysAllow`, plain-English shell risk summaries), step-limit pause recovery, unfocused OS attention notifications, clickable header LSP diagnostics (`DiagnosticsModal`), `DoctorModal`, `InstructionsModal`, `ProfileModal`, `ModelModal` synthetic tool-calling probe (`/probe`), `@file` / `@file#symbol` / `@file:Lstart-Lend` precision context slices, KV-cache prefix vs uncached tail `ContextModal` with manual selective prune (`Op::PruneContext`), `/` slash autocomplete, image paste/drop/picker, sidebar file reveal and Markdown/JSON field-note export, separate plan/edit and planner-model controls, thinking-level picker, session sidebar with live duck-on-a-river plan progress and steps, paddling launch ducks and weighted decision-plate transitions honoring `ui.reduced_motion`, a project & global memory viewer/editor (`MemoryModal`), a unified **Settings** page (resolved paths like `memory.path` and `sessions.path`, `quack setup` options, profiles, bundles, rules, Hooks & MCP server setup/toggles, and raw `config.toml` key editing), and workspace prompt input history (`history.jsonl`) shared with the TUI. Failed sends preserve drafts and pending answers; session changes wait for engine confirmation. Slow tab startup or shutdown leaves other tabs responsive, and reloading the UI shuts down its previous engines before reconnecting. Both the TUI and desktop GUI read the shared on-disk `defaults/` folder; login-shell PATH recovery has bounded time and output. See [desktop progress and remaining work](docs/plan/15-desktop-client.md#boundary-shield-precision-context--desktop-craft-2026-11-15).

---

<a id="features"></a>

## Features

### Models & Reasoning

- **Local and hosted engines**: Works with 6 local servers (Ollama, llama.cpp, vLLM, SGLang, LM Studio, Docker Model Runner) and 36 hosted provider presets (42 bundled provider files total: Anthropic, OpenAI, ChatGPT plan, Google Gemini, xAI, Cohere, OpenRouter, Groq, Cerebras, Fireworks, Together, DeepInfra, Novita, Nebius, Baseten, Chutes, Hugging Face, NVIDIA NIM, SambaNova, SiliconFlow, Alibaba Cloud, DeepSeek, Moonshot, Kimi, Z.ai, MiniMax, Xiaomi MiMo, Mistral, Vercel AI Gateway, OpenCode Zen, GitHub Models, Venice, Hyperbolic, Scaleway, Synthetic, Charm Hyper).
- **Multi-endpoint failover (`base_urls`)**: Configure `base_urls = [...]` in a provider file to round-robin across internal GPU nodes with automatic exponential health cooldown (15 s → 120 s) on retryable failures across streams, `/models`, Ollama `/api/show`, and Responses WebSockets. `[policy.providers] hosts` validates every endpoint in `base_urls`.
- **Thinking and reasoning models**: `/thinking` picks the active model's supported effort level; `/thinking high` sets it directly and `/thinking default` restores its configured default. The toolbar and sidebar show the level beside the model, and each model's selection survives reload, session resume and restarts. `Ctrl-T` or `/thinking show|hide` controls the reasoning display. Reasoning tokens are preserved across tool steps as required by DeepSeek, Kimi, MiniMax, GLM, Claude, and OpenAI.
- **Tool-call repair**: Automatically parses and heals calls returned as text or markup (Hermes, Qwen-XML, GLM, DeepSeek, Kimi formats), recovers loose arguments, and deduplicates call IDs.
- **Guided decoding**: Enforces schema-guided grammar decoding (`ToolCallMode::Grammar`) on supported local engines to eliminate JSON malformations.
- **Family profiles**: Automatically tunes prompt templates, tool definitions, edit formats, and sampling parameters for specific model families (8 shipped profiles: `generic`, `qwen-code`, `cc-style`, `zcode`, `kimi-code`, `codex`, `dsh`, `vibe`).
- **Live switching**: Change active models mid-conversation with `/models` or inspect provider health with `/providers` without losing conversation history. Models that declare `reasoning.efforts` show their levels in `/models`, and quack asks for a thinking level right after you switch to one, since the level changes both behaviour and cost.
- **Token telemetry**: Real-time tokens per second and time-to-first-token tracking in the status bar.

### Tools & Editing

- **Core tool suite**: `read` (500-line default cap with automatic outlining for large files and `symbol` reads), `write`, `glob`, `grep` (regex or `literal: true`, match-centered excerpts, and `content`/`files`/`count` modes), `list_dir`, `shell` (including managed background jobs via `background: true`, `job_action: "wait" | "output" | "stop" | "list"`, blocking `wait_secs`, and delta output on polls), `lsp` (definition, references, hover, symbols, diagnostics, rename, and `code_action` listing/applying), `index`, and opt-in SSRF-protected `web_fetch` (`[tools.web_fetch] enabled = true`, off by default and disabled when offline). Allowed read-only tools execute in parallel while preserving result order.
- **Intelligent code indexing**: `index` generates Tree-sitter symbol outlines (functions, types, methods, and line ranges, cached per file snapshot) for Rust, Python, Go, TypeScript, JavaScript, C, C++, and Bash. Large file reads automatically return outlines so the model reads only relevant definition spans.
- **Flexible edit formats**: Configurable via `[tools] edit_format`:
  - `str_replace`: Forgiving single-snippet replacement (with optional `symbol` scoping), or batch independent exact replacements in one file with `edits`.
  - `hashline`: File snapshot tags and numbered lines, with function-level block targeting and shown-line guards on replacements and insertions.
  - `apply_patch`: Multi-file transactional patches with ambiguity rejection, post-edit LSP diagnostics, and nested `AGENTS.md` discovery.
  - `search_replace_blocks`: Multi-block SEARCH/REPLACE operations.
  - `whole_file`: Full file rewrites.
- On OpenAI-compatible Responses and ChatGPT plan routes, optional tool arguments can be omitted. An `edit` batch can send just `path` and `edits`.
- **Transactional guarantees**: Multi-file refactors check all changes before writing. If any single change fails, all touched files roll back to their original state.
- `str_replace` and `apply_patch` require a prior read or write before changing an existing file and reject a stale read. Hashline tracks file versions and the numbered ranges shown to the model.
- **Distilled test output**: Build and test outputs (Cargo, Pytest, unittest, Go test, Jest, TAP, and compiler progress lines) strip ANSI escape codes, fold passing tests (`[... 42 passed tests omitted ...]`), collapse Cargo file-lock waits and repeated manifest warnings, spill truncated output to a temp file when trimmed, and retain assertion failures and stack traces while preserving context space.

### Code Intelligence (LSP)

- **Zero-config language servers**: Automatic background integration for Rust (`rust-analyzer`), Go (`gopls`), Python (`pyright`/`basedpyright`), TypeScript/JavaScript, C/C++ (`clangd`), C#, Java, Lua, Bash, TOML, and YAML. Uses existing servers installed on your `PATH`.
- **Immediate edit diagnostics**: As soon as a file is edited, newly introduced compile or type errors are fed back into the tool result so the model fixes them before continuing.
- **Model LSP navigation & edits**: Enables the model to find definitions, locate references, view documentation hovers, inspect diagnostics, list/apply LSP `code_action` quickfixes and refactors, and perform project-wide symbol renames — all with diff previews, approvals, and undo checkpoints.
- **Status bar counters**: Displays live error and warning counts (`✗1 ⚠3`) directly in the UI. Inspect indexing progress and server logs with `/lsp`.

### Safety & Sandboxing

- **Independent work mode and approvals**: choose **Plan / Edit** and **Ask / Allow** separately. Plan reads and searches; Edit may change files and run tests. Ask prompts for unapproved actions; Allow runs them within the configured sandbox. `/yolo` or `--yolo` explicitly disables approvals and sandboxing.
- **OS-level sandboxing**: Shell commands (and opt-in `sandbox = true` MCP/LSP servers) execute in an isolated sandbox (Landlock + seccomp on Linux, Seatbelt on macOS, Restricted Token + Low Integrity + Job Objects on Windows) with workspace-only write access and network access disabled (Windows restricts writes but not the network).
- **AST-aware command verification**: Tree-sitter parses shell commands to verify that compound expressions (`&&`, `||`, subshells, command substitutions, aliases) do not violate deny rules.
- **Transparent approvals**: When an edit needs approval, the prompt shows a diff preview with options to allow once (`y`), allow for session (`a`), permanently allow rule (`A`), or deny (`n`).
- **Project trust**: Untrusted cloned repositories execute no `.quack/` configurations until explicitly trusted via `/trust`.
- **Quick sandbox toggle**: Toggle the sandbox on or off during a session with `/sandbox` while preserving work mode and approvals.
- **Pre-egress Secret Shield & Session Egress Ledger**: Locally scans outbound prompts, `@` file mentions/slices, `AGENTS.md` / session instructions, tool results, and previously local history on mid-session switches to remote providers with `redact-secret` plus active resolved credentials (`[network] secret_guard = "ask"`) before HTTP egress, while logging every outbound/blocked hop into a live session ledger.
- **Enterprise JSONL audit log (`[audit]`)**: Optional append-only mode-`0600` JSONL log (`ts`, `session_id`, `agent`, `call_id`, `tool`, `subject`, `decision`, `decided_by`, `ok`, `duration_ms`) recording every allowed, denied, hook-blocked, or interrupted tool execution, lockable via `[policy]`. Tool arguments and result summaries are omitted by default (`include_content = false`).
- **Windows machine-level registry policy**: Reads `HKLM\SOFTWARE\Policies\quack` (`SystemDir` and `PolicyToml`) natively on Windows so GPO/MDM/Intune policies cannot be redirected via `%ProgramData%` or `QUACK_SYSTEM_DIR`, and reports active locks and audit health in `quack doctor`.

### Planning & Sub-Agents

Choose what the agent can do and when it asks independently. New sessions default to **Plan + Allow**.
Change either default in the desktop app’s **Settings → Permissions**, in the TUI’s `/config`,
or in `config.toml` under `[permissions]`.

| Work mode | Ask | Allow |
|---|---|---|
| **Plan** | Read and search; unapproved tools ask | Read and search automatically |
| **Edit** | Read and search; unapproved edits and commands ask | Read, search, edit and run commands automatically |

Plan proposes the exact test commands and what each verifies; it never runs tests, builds or
scripts, or changes files or memory. Allow keeps the configured sandbox, deny rules and explicit
ask rules; leaving the sandbox still needs approval. Yolo disables approvals and sandboxing,
while Plan restrictions and deny rules still apply.

```sh
quack --mode plan --approvals allow   # investigate automatically; propose tests for review
quack --mode edit --approvals ask     # implement with approvals
```

In the TUI, **Shift-Tab** toggles Plan/Edit; `/permissions ask` or `/permissions allow` changes
approvals. `/plan accept` implements the plan with the current approvals (`/plan accept fresh`
clears planning exploration into a concise handoff brief first); `/edit` leaves planning
without starting implementation. See [work modes and approvals](docs/modes.md) for commands,
configuration and migration from older modes.

- **Interactive plan review**: `/plan` reads and searches, asks clarifying questions, and presents proposed changes and tests. The review bar offers **Implement plan**, **Implement plan (fresh context)**, or **Keep planning**. Work mode and approvals survive reload and resume, subject to system policy.
- **Dual-model planning**: Use a large model to architect the plan (`[model] planner = "..."`) while your daily model carries out the implementation steps.
- **Sub-agents**: Delegate work to isolated sub-agents using the `task` tool:
  - `explore`: Read-only scout that searches the codebase and returns file paths and line ranges, keeping exploratory reads out of the primary context.
  - `general`: Executes focused, self-contained implementation tasks.
- **Live checklist tracking**: See the current step and completed count above the input or in the sidebar. When there's room, a duck moves right along a blue river in proportion to the completed steps and stays at the far edge at 100%. Expand the full checklist with `/todo`.

### Context & Sessions

- **Smart token management**: Prunes superseded file reads, obsolete tool outputs, and failed attempts before performing compaction.
- **Lossless compaction**: Compaction preserves the original user prompt and files changed to date.
- **Context analytics**: `/context` visualizes token distribution across prompts, tools, and history; `/stats` graphs cumulative token usage, latency, and session cost.
- **Session persistence & retention**: Every session is written incrementally and survives crashes. Continue seamlessly with `quack --continue` or `/resume`, prune older transcripts with `quack sessions prune --older-than 30d [--all] [--dry-run]`, or enable automatic startup retention cleanup with `[sessions] retention_days`.
- **Time travel and branches**: `/undo` rolls back file changes (including mutating shell commands via private shadow git snapshots); `/rewind` reverts files and conversation turns to any previous point; `/fork` branches a session into a new thread, and `/fork --worktree [name]` (or `quack --worktree <name>`) isolates the branch in a git worktree under `.quack/worktrees/<name>`.
- **Persistent memory**: Maintains project and global memory notes (`/memory`), auto-detecting corrections and preferences across sessions.
- **Instruction hierarchy**: Automatically loads instructions from `AGENTS.md` (and imports existing `CLAUDE.md`, `.cursorrules`, or Copilot files).

### MCP & Extensibility

- **Model Context Protocol (MCP)**: Supports stdio and streamable HTTP MCP servers — including **OAuth 2.1 + PKCE** sign-in with RFC 9728/8414 discovery and RFC 7591 Dynamic Client Registration (`quack mcp login <id>`, `/mcp login <id>`), bundled disabled examples (`context7`, `roost`, `github`, `fetch`, `playwright`, `filesystem`, `time`, `everything`), guided setup (`quack mcp add`, `/mcp`), per-server `defer = "auto" | "always" | "never"` (`tool_search`), full-output spill files on truncation, live progress streaming, prompts as `/<server>:<prompt>`, resources, and elicitation/sampling (with optional `sampling_model = "small"`).
- **Deferred discovery**: Large tool surfaces are indexed and searched dynamically (`tool_search`), avoiding context bloat.
- **Event hooks**: Run custom scripts on session events (`session_start`, `user_prompt`, `pre_tool`, `post_tool`, `turn_end`) configured in `hooks.d/`. `/hooks` configures linked Jira issues and GitHub, GitLab, Gitea/Forgejo or Bitbucket reviews, including private repositories and multiple self-hosted instances. Enter credentials in a masked field, then **Save and enable**; the next prompt uses them. [Setup and examples](defaults/hooks.d/README.md).
- **Standard Agent Skills (`SKILL.md`)**: Reads generic, standard Agent Skills (`SKILL.md` with YAML frontmatter) automatically from `.agents/skills/` (project) and `~/.agents/skills/` (user), alongside `skills/` in your config layers and bundles—plus extra folders such as `.claude/skills` via `[skills] compat_paths`. Only names and descriptions sit in the prompt index; the model loads full instructions on demand with the `skill` tool, or you can browse and invoke them via `/skills` and `/<name>`.
- **Custom slash commands**: Place markdown prompt templates in `commands/` (e.g. `commands/git/pr.md` becomes `/git:pr`).
- **Import from other coding agents**: If Quack finds folders such as `.claude/skills` or `.claude/commands`, the terminal startup panel points to `/compat`. Review a folder, then press **Enter** to load it for this run or **Ctrl-S** to load it and save its path for future runs. Workspace paths are saved in `.quack/config.toml` when project settings are allowed; home paths are saved in your user config. Nothing is imported automatically or copied. Only load prompt files you trust.
- **Bundles**: Group skills, commands, MCP servers, and hooks into shareable directory bundles.
- **Scoped live hot-reload**: Edits to `skills/`, `commands/`, `hooks.d/`, `profiles.d/`, `agents.d/`, `themes/`, and `grammars/` hot-reload automatically without dropping session approvals; edits to individual `mcp.d/` or `lsp.d/` files restart or stop only the affected server(s).

### Interface

- **Full-screen Ratatui TUI**: Markdown formatting, syntax-highlighted code blocks, tables, and optional ASCII mode (`--ascii`). The welcome duck takes one short paddle at launch, then rests above notices and messages. It scrolls away as the conversation grows; **Ctrl-Home** brings it back.
- **Wide-terminal sidebar**: At 140 columns × 24 rows and above, the right sidebar shows Session, Working folder (with Git branch), Model (with profile), Context, Plan, Sub-agents, LSP, and Skills, in that order. It lists all available skills, marks the ones loaded in the session, and lists all running and finished sub-agent runs. Scroll over the sidebar or use **Alt-PageUp / Alt-PageDown** when it overflows. It grows from 36 up to 48 columns on wider terminals; set `ui.sidebar = false` to disable it, or toggle it in `/config`.
- **Window title**: The terminal's title shows the duck and what the session is about (`_(•)> fix the login bug`), and marks when a turn runs or quack needs you. If `[model] small` is configured, quack makes one background request to title the session (3–6 words, at most 50 characters) from the first message; thinking overhead is minimized while still leaving reasoning models room to think, broken `model.small` setups warn, and generation hiccups quietly keep the first-line fallback. The title also appears in the sidebar, `/resume` and `quack-gui` (its window title, header and job list). The first line of the message serves as the fallback, and `/title` sets it manually. `ui.terminal_title = false` turns it off.
- **Smooth scrolling and selection**: Mouse wheel scrolling, click-to-position cursor, drag-to-select copying (`Ctrl-Shift-C` or `Ctrl-C` with a selection copies without interrupting a running turn; `Shift`-drag uses the terminal's own selection), and native system clipboard (`wl-copy`, `xclip`/`xsel`, `pbcopy`, Windows clipboard) alongside OSC 52. When scrolled up, a width-aware hint shows that **Ctrl-End** jumps to the latest output.
- **Interactive pickers**: Fast, fuzzy-filtered modals for `/models`, `/providers`, `/config`, `/context`, `/stats`, `/doctor`, `/probe` (synthetic tool-calling probe), `/gui` (hand off session to `quack-gui`), and `/help`. In `/models` (and `quack-gui`'s model selector), set any model as the **Large** default (`[model] default`), **Small** helper (`[model] small`, used for session titles and `explore` sub-agents), or **Both** (`Ctrl-S` in `/models`, or `/models default [large|small|both] <provider>/<model>`). In `/config` → Layers, inspect each layer’s status and file path, then press Enter to copy the selected path. In `/config`, `/models`, `/profiles`, `/theme`, `/bundles`, `/hooks`, `/mcp` (including MCP add prompts), and approval prompts (`always`), press `Ctrl-L` (or `/scope [user|project]` / `--project` / `--user`) to choose whether writes go to user config or the project's `.quack/` folder.
- **Status bar**: Separate work mode and approval indicators, context window utilization, model generation speed, and background tasks.

### Editor Integration (ACP)

`quack acp` implements the [Agent Client Protocol](https://agentclientprotocol.com) over stdio, allowing editors like **Zed, JetBrains, Neovim, and VS Code** to drive quack inside their native agent panels.

Example configuration for Zed (`settings.json`):
```json
{
  "agent_servers": {
    "quack": { "type": "custom", "command": "quack", "args": ["acp"] }
  }
}
```

The editor panel receives streaming replies, reasoning steps, tool diffs, sub-agent tool calls, slash command lists, session titles, image attachments, `session/list`, permission approval prompts, and checklist updates directly from quack — plus optional editor-routed `fs/*` and `terminal/*` delegation (`[tools] acp_client_fs` / `acp_client_terminal`).

---

## Installation Options

### Prebuilt Packages

Prebuilt archives for all supported targets are published on the [Releases](https://github.com/ClackStack/quack/releases) page:
- **Linux**: Static binaries (musl) for x64 and ARM64, `.deb` and `.rpm` packages.
- **macOS**: Universal archives (x64 and Apple Silicon) and Homebrew tap formulas.
- **Windows**: x64 and ARM64 WiX v5 `.msi` installers (per-machine `ALLUSERS=2` for GPO/MDM/Intune or per-user `MSIINSTALLPERUSER=1`, MST-ready `DefaultsFeature`), zip archives, and PowerShell installer.
- **Desktop GUI (`quack-gui`)**: Unsigned desktop installers and packages sharing the same on-disk `defaults/` directory as `quack`: `.dmg` for macOS (x64 and Apple Silicon), `-setup.exe` NSIS installers for Windows (x64 and ARM64), and `.deb`, `.rpm`, and `.AppImage` for glibc Linux (x64 and ARM64; static musl targets ship the CLI only). Installing the GUI alone does not install `quack` (and vice versa), though they share one `defaults/`, config, sessions, memory, and history; `install.sh` asks what to install (both by default, or whatever is already installed when updating) unless `--tui`/`-Tui` or `--gui`/`-Gui` is passed. On Linux it prefers the `.deb`/`.rpm` packages — which register the desktop entry and the shared `defaults/` system-wide — when the machine can install one (root or sudo), falling back to the self-contained `~/.local` archive; `--pkg deb|rpm|archive` forces a format.
- **Supply chain & verification**: Every release publishes `SHA256SUMS`, CycloneDX 1.5 SBOMs (`quack.cdx.json`, `quack-gui.cdx.json`), and keyless GitHub SLSA build provenance attestations (`gh attestation verify <file> --repo ClackStack/quack`).

### Minimal / Air-Gapped Installs (`--plucked`)

For air-gapped environments or private clusters, `--plucked` (`-Plucked` on Windows) installs the same `quack` and/or `quack-gui` binaries — whether via `.deb`/`.rpm` system packages or self-contained archives, with profiles, sub-agents, MCP, ACP, language servers, hooks, and local model support — while omitting all hosted-service provider files and `defaults/config.toml` from the installed `defaults/` folder. Updates to a plucked installation keep hosted providers omitted automatically (pass `--full` / `-Full` to restore them):

```sh
curl -fsSL https://quack.clackstack.dev/install.sh | sh -s -- --plucked
```

See [docs/enterprise.md](docs/enterprise.md) for enterprise deployment policies and packaging guides.

---

<a id="configuration-reference"></a>

## Configuration

quack contacts only the providers and MCP servers you explicitly configure. Everything is declared in human-readable TOML files across layered directories.

### Configuration Layers

Settings merge in the following order of precedence (higher layers override lower layers):

| Layer | Location | Purpose |
|---|---|---|
| **Defaults** | `<prefix>/share/quack/defaults/` or `QUACK_DEFAULTS_DIR` | Bundled base configurations and profiles |
| **System** | `/etc/quack/` (Linux), `/Library/Application Support/quack/` (macOS), `%ProgramData%\quack\` + `HKLM\SOFTWARE\Policies\quack` (Windows) | Administrative policies and enforced locks |
| **User** | `~/.config/quack/` (`$XDG_CONFIG_HOME`) or `%APPDATA%\quack\` | User preferences and private configurations |
| **Project** | `.quack/` at the repository root | Project-specific tools, commands, and rules (active once trusted) |
| **Environment** | `QUACK_MODEL`, `QUACK_PROMPT_CACHE_KEY`, `QUACK_CONFIG_DIR`, etc. | Runtime environment variables |
| **CLI Flags** | `-m`, `--profile`, `--mode`, `--approvals`, `--worktree`, `--cache-key`, `-c key=value` | Command-line overrides for the active run |

Run `quack config show --origin` to view your effective configuration and see which layer defined each setting.

### Directory Structure

Every layer shares an identical layout:

```text
<layer>/
├── config.toml          # Global engine, UI, and permission settings
├── providers.d/*.toml   # Model servers and endpoints
├── mcp.d/*.toml         # Model Context Protocol servers
├── lsp.d/*.toml         # Language server configurations
├── hooks.d/*.toml       # Event lifecycle scripts
├── agents.d/*.toml      # Sub-agent configurations
├── profiles.d/<id>/     # Model family prompt templates and tool mappings
├── commands/*.md        # Custom slash commands
├── skills/<name>/       # Standard Agent Skills (SKILL.md; also read from .agents/skills/ and ~/.agents/skills/)
├── bundles/<name>/      # Modular packages grouping any of the above
└── AGENTS.md            # Project and workspace instructions
```

### Enterprise & Private Server Example

To connect quack exclusively to an internal model server (e.g. vLLM or llama.cpp) and lock down egress:

**1. Define the provider:**
```toml
# /etc/quack/providers.d/corp.toml
name = "Corp LLM"
base_url = "https://llm.internal.example:8000/v1"
api_key = { env = "CORP_LLM_KEY" }
models = [
  { id = "qwen3-coder-480b", alias = "coder", context_window = 262144 },
  { id = "qwen3-coder-30b", alias = "fast", context_window = 131072 },
]
```

**2. Enforce system policy:**
```toml
# /etc/quack/config.toml
[model]
default = "corp/coder"
small = "corp/fast"

[network]
offline = true                 # Remote MCP servers allowed only on local host or egress_allow

[sandbox]
mode = "workspace-write"       # Sandboxed execution; no network egress

[permissions]
deny = ["shell:curl *", "shell:wget *"]

[audit]
path = "/var/log/quack/audit.jsonl"  # append-only metadata log (no args/content by default)

[policy]
locked = ["model.default", "network", "sandbox.mode", "permissions.deny", "audit"]
max_permission_mode = "allow"  # allow automatic actions, but prevent yolo
project_config = "trusted-only"

[policy.providers]
allowed = ["corp"]
hosts = ["llm.internal.example"]

[policy.mcp]
hosts = ["*.internal.example"]
```

**3. Verify with `quack doctor`:**
```text
ok    defaults      /usr/share/quack/defaults
ok    providers     1 usable
ok      corp        2 declared models
ok    model         corp/coder
ok    agents        2 agents: explore (read-only, on corp/fast), general
ok    sandbox       workspace-write · Landlock + seccomp
ok    policy        5 locked · max approvals: allow · provider allowlist · mcp allowlist · offline
ok    audit         /var/log/quack/audit.jsonl (metadata only)
info  network       llm.internal.example · policy allows only llm.internal.example, *.internal.example
```

---

## Configuration Reference

### `config.toml`

**`[model]`**
| Key | Default | Description |
|---|---|---|
| `default` | `""` | Default model in format `<provider>/<model>` or `<provider>/<alias>` |
| `small` | `""` | Fast model for summaries, titles, and `explore` sub-agent |
| `planner` | `""` | Dedicated model for plan mode (falls back to active model) |
| `remember_last` | `true` | Resume with the most recently used model |
| `profile` | `""` | Force a specific prompt profile across all models |

**`[permissions]`**
| Key | Default | Description |
|---|---|---|
| `work_mode` | `"plan"` | `plan` (read/search and propose tests) or `edit` (implementation) |
| `mode` | `"allow"` | Approval policy: `ask` or `allow` |
| `allow` | `[]` | Rule patterns executed without prompting (`shell:cargo test*`, `edit:src/**`) |
| `ask` | `[]` | Rule patterns that always prompt, even in `allow` (yolo bypasses asks) |
| `deny` | `[]` | Rule patterns strictly forbidden across all modes |

Older `auto-edit` and `read-only` settings retain their behavior for compatibility but are
omitted from the approval choices. Use Plan for read/search-only work, or Ask with
`allow = ["edit:src/**"]` to approve selected edits automatically. `full-auto` remains an
alias for Allow; existing flags and saved sessions still load without increasing access.

**`[sandbox]`**
| Key | Default | Description |
|---|---|---|
| `mode` | `"auto"` | Sandbox mode: `auto`, `workspace-write`, `workspace-write-net`, `read-only`, or `off` |
| `writable` | `[]` | Additional writable directories outside the workspace (`["~/.cargo", "~/.cache"]`) |

**`[network]`**
| Key | Default | Description |
|---|---|---|
| `offline` | `false` | When true, restricts network access to local interfaces or `egress_allow` hosts |
| `egress_allow` | `[]` | Allowed remote host globs when `offline = true` |
| `proxy` | `""` | Proxy URL. When empty, each scheme uses `HTTP_PROXY` or `HTTPS_PROXY`, with `ALL_PROXY` as a fallback. System settings apply when no environment proxy is set. Hosts in `NO_PROXY` go direct |
| `ca_bundle` | `""` | Path to custom PEM certificate bundle |
| `http1_only` | `false` | Force HTTP/1.1. Try it if HTTP/2 requests stall behind a VPN or proxy |
| `secret_guard` | `"ask"` | Pre-egress Secret Shield policy (`ask`, `redact`, or `off`) scanning prompts and tool outputs locally with `redact-secret` + resolved credentials before HTTP send (non-interactive clients fall back from `ask` to `redact`) |
| `secret_guard_local` | `false` | When `true`, also applies Secret Shield to loopback destinations (`localhost`, `127.0.0.1`) |
| `secret_allowlist` | `[]` | SHA-256 secret fingerprints explicitly allowlisted across sessions |

In `ask` mode, Secret Shield shows the destination provider and host, where it found
each possible secret, and what the match could expose if valid. The preview is masked,
and line numbers refer to the scanned text, not necessarily a file. Choose **Redact &
send** to replace detected matches, or withhold the flagged text. If you send it
unredacted, later requests may include it again as part of the conversation.

For a single provider, select it in TUI `/providers` and press `Ctrl-S` (or `Enter`
when it has no API key), or open **Advanced connection settings** on the desktop
**Providers** screen. Choose an HTTP version and whether to use proxies. **Direct
connection** sets `no_proxy = true` under `[http]` in the provider file. **HTTP/1.1
only** sets `http1_only = true`, **Negotiate** sets it to `false`, and **Inherit**
removes that file's override.

For a vLLM server reached through a Windows VPN from WSL, run
`getent ahosts HOST` and `curl --noproxy '*' -v http://HOST:5000/v1/models` inside WSL
with the same hostname and API prefix as the provider URL. If direct HTTP reaches the
server, try **Direct connection** for that provider or put the hostname in `NO_PROXY`.
An address lookup error happens before HTTP version negotiation. If lookup fails inside
WSL, try the full hostname and check [Microsoft's WSL VPN and DNS tunneling guidance](https://learn.microsoft.com/en-us/windows/wsl/troubleshooting#wsl-connectivity-issues-with-vpns-when-mirrored-networking-mode-is-on).

**`[catalog]`**
| Key | Default | Description |
|---|---|---|
| `refresh` | `false` | Refresh model prices and context windows in the background, at most once a day |
| `url` | models.dev (from the defaults bundle) | Where the catalog comes from, in the models.dev format. Empty: nowhere |

**`[updates]`**
| Key | Default | Description |
|---|---|---|
| `check` | `false` | Look for a new version at most once a day and say so in the TUI. quack never updates itself |
| `url` | this repository's latest release (from the defaults bundle) | Answers with a redirect ending in the version, or the version as text. Empty: nowhere |

**`[policy]`** *(System administrative layer only)*
| Key | Default | Description |
|---|---|---|
| `locked` | `[]` | Configuration keys that lower layers cannot modify |
| `max_permission_mode` | unset | Ceiling on approvals; `"allow"` prevents yolo. Lock `permissions.work_mode` separately to enforce Plan |
| `project_config` | `"trusted-only"` | Project `.quack/` loading policy: `trusted-only`, `never`, or `always` |
| `providers.allowed` | `[]` | Allowed provider ID globs |
| `providers.hosts` | `[]` | Allowed provider host globs |
| `mcp.allowed` | `[]` | Allowed MCP server ID globs |
| `mcp.hosts` | `[]` | Allowed remote MCP host globs |

**`[context]`**
| Key | Default | Description |
|---|---|---|
| `window` | unset | Maximum token ceiling (overrides server limit) |
| `budget_ratio` | `0.85` | Fraction of window reserved for context vs response buffer |
| `compaction_threshold` | `0.80` | Threshold of budget at which automatic pruning and compaction trigger |
| `cache_strategy` | `"auto"` | KV cache optimization: `auto`, `preserve`, or `aggressive` |
| `cache_key` | unset | Shared prompt-cache routing key (overrides per-session UUID for parallel `quack exec` / CI runs; also `--cache-key` or `QUACK_PROMPT_CACHE_KEY`) |

**`[tools]`**
| Key | Default | Description |
|---|---|---|
| `edit_format` | `"auto"` | Edit format: `auto`, `str_replace`, `hashline`, `apply_patch`, `search_replace_blocks`, or `whole_file` |
| `shell` | `"auto"` | Shell interpreter: `auto`, `bash`, `zsh`, `pwsh`, `powershell`, `cmd`, or path |
| `default_timeout_secs` | `120` | Maximum execution time before process tree termination |
| `read_outline_tokens` | `2000` | Token threshold above which large file reads return Tree-sitter outlines |
| `web_fetch.enabled` | `false` | Opt-in SSRF-protected `web_fetch` tool; always unregistered when `[network] offline = true` |
| `web_fetch.allow_private` | `false` | Allow fetching loopback, RFC1918 private, link-local, and cloud-metadata IPs |
| `web_fetch.max_bytes` | `2097152` | Maximum response body size in bytes (2 MiB) |
| `web_fetch.timeout_secs` | `20` | Per-request timeout in seconds |
| `acp_client_fs` | `false` | Under `quack acp`, route `read`/`write` through the editor (`fs/*`) when advertised |
| `acp_client_terminal` | `false` | Under `quack acp`, run foreground `shell` commands in the editor's terminal (`terminal/*`) when advertised |

**`[audit]`**
| Key | Default | Description |
|---|---|---|
| `path` | `""` | Mode-`0600` JSONL file appending one structured record per tool execution (empty disables auditing) |
| `include_content` | `false` | Include tool arguments and result summaries in audit entries (omitted by default) |

**`[agent]`**
| Key | Default | Description |
|---|---|---|
| `max_steps` | `100` | Model calls per turn before it pauses and asks whether to keep going (headless runs stop instead). Sub-agents have their own limits in `agents.d` |

**`[ui]`**
| Key | Default | Description |
|---|---|---|
| `theme` | `"auto"` | Theme name (`dark` or `light`) |
| `show_reasoning` | `"collapsed"` | Initial display state for reasoning: `collapsed`, `expanded`, or `hidden` |
| `startup_panel` | `"auto"` | Startup diagnostics: in the sidebar until the first turn; on narrow terminals only on problems. `always` or `never` |
| `ascii` | `false` | Enable pure ASCII rendering mode |
| `mouse` | `true` | Enable mouse wheel scrolling, cursor placement, and drag selection |
| `reduced_motion` | `false` | Keep the welcome duck and spinner indicators still |
| `terminal_title` | `true` | Set the terminal window title to the duck and the session title (`_(•)> fix the login bug`); `false` leaves the title alone |
| `sidebar` | `true` | Show session details on the right at 140 columns × 24 rows and above, growing from 36 up to 48 columns on wider terminals; `false` uses the full width for the conversation |
| `gui_restore_tabs` | `true` | Reopen previously open workspace tabs (`session-tabs.json`) when starting `quack-gui` unless `--no-restore` is passed |

**`[instructions]`, `[skills]`, `[commands]`, `[memory]`, `[sessions]`**
| Key | Default | Description |
|---|---|---|
| `instructions.files` | `["AGENTS.md"]` | Instruction filenames checked in workspace root |
| `instructions.other_agents` | `true` | Read existing `CLAUDE.md`, `GEMINI.md`, or `.cursorrules` if `AGENTS.md` is absent |
| `instructions.max_bytes` | `32768` | Maximum byte limit per instruction file |
| `skills.compat_paths` | `[]` | Extra `SKILL.md` directories beyond `.agents/skills/`, `~/.agents/skills/`, and `skills/` (e.g. `.claude/skills`) |
| `skills.disabled` | `[]` | Skill names to skip |
| `skills.index_max_tokens` | `1500` | Maximum token budget for the skill index in the prompt |
| `commands.compat_paths` | `[]` | Extra Markdown command folders (e.g. `.claude/commands`); relative paths are from the workspace root |
| `memory.path` | `""` | Directory for persistent memory files (defaults to user data directory) |
| `memory.project_scope` | `"data-dir"` | Memory storage scope: `data-dir` or `repo` (`.quack/memory`) |
| `memory.writes` | `"auto"` | Approval policy for new memory notes: `auto` or `ask` |
| `memory.index_max_tokens` | `1500` | Maximum token budget for the memory index in the prompt |
| `memory.nudge` | `true` | Remind the model to save corrections and lasting preferences |
| `sessions.path` | `""` | Directory for session transcripts |
| `sessions.retention_days` | `0` | Automatically prune sessions older than N days on startup (`0` keeps forever) |

---

### `providers.d/<id>.toml`

| Key | Default | Description |
|---|---|---|
| `base_url` | required (or `base_urls`) | Primary server API endpoint URL |
| `base_urls` | `[]` | Additional endpoint URLs for round-robin balancing and exponential health-cooldown failover across nodes |
| `api` | `"openai-chat"` | Protocol: `openai-chat`, `openai-responses`, `anthropic-messages`, or `ollama` |
| `api_key` | none | Secret definition (`"sk-..."`, `{ env = "VAR" }`, `{ keyring = "id" }`, `{ file = "path" }`, `{ cmd = [...] }`) |
| `auth` | none | Browser sign-in instead of a key (e.g. `{ type = "chatgpt", issuer = "https://auth.openai.com" }`) |
| `models` | `[]` | Declared models or server discovery options |
| `name`, `group` | ID, `""` | Display labels in setup and picker modals |
| `enabled` | `true` | Toggle provider availability |
| `auth_header`, `auth_scheme` | `Authorization`, `Bearer` | HTTP authorization header configuration |
| `headers` | `{}` | Custom HTTP headers sent with requests |
| `extra_body` | `{}` | JSON fields merged into every request body under each model's `extra_body` |
| `native_compaction` | `false` | Use native OpenAI Responses compaction (`true` on bundled `openai` and `chatgpt`) |
| `responses_websocket` | `false` | Use persistent Responses WebSocket continuation with Ping/Pong keepalive (`true` on bundled `openai` and `chatgpt`) |
| `prompt_cache` | `{}` | Prompt-cache wire options: `developer_instructions`, `explicit_breakpoints`, `ttl` (or Ollama `keep_alive`), `diagnostics`, `cache_key`, `affinity_header`, `cache_control` (`auto`\|`breakpoints`), `cache_control_models` |
| `tls.ca_file`, `client_cert`, `client_key` | none | Custom root CA and mutual TLS client certificate |
| `http.timeout_connect_secs` | `30` | Connection timeout, covering TCP, any proxy and TLS |
| `http.timeout_idle_secs` | `300` | Stream idle timeout |
| `http.timeout_first_byte_secs` | `0` (the idle timeout) | Wait for response headers before retrying; doubles on each retry |
| `http.max_retries` | `3` | Maximum automatic retries on transient errors |
| `http.proxy`, `http.no_proxy`, `http.http1_only` | `""`, `false`, unset | Per-provider proxy URL, direct-connection bypass, and HTTP/1.1 override |
| `discovery.enabled`, `discovery.include` | `false`, `[]` | Query `/models` at startup and filter by model ID globs |
| `catalog` | provider ID | ID in the model catalog, for prices and windows nothing else reports. Servers on this machine aren't looked up unless named; `""` turns it off |

**Per-model options in `models`:**
| Key | Default | Description |
|---|---|---|
| `id` | required | Model identifier recognized by provider |
| `alias` | none | Short name for CLI and picker selection (`<provider>/<alias>`) |
| `context_window`, `max_output` | from server, then catalog (window only) | Token boundary definitions |
| `compact_at` | unset | Soft prompt-token ceiling for automatic compaction (e.g. `250000` below a 272k cliff) |
| `profile` | `"auto"` | Model profile ID or `auto` matching |
| `vision` | `false` | Enable image input and `view_image` tool |
| `tool_calling.mode` | `"native"` | `native`, `text` (markup in reply), or `grammar` (guided decoding) |
| `tool_calling.text_format` | `"auto"` | Format for text tool calls: `hermes`, `qwen3-xml`, `glm`, `json-block`, `deepseek`, or `kimi` |
| `tool_calling.parallel` | `true` | Allow parallel tool calls on the wire (`false` sends `parallel_tool_calls: false`) |
| `reasoning.format` | `"auto"` | Format of thinking tokens: `reasoning_content`, `reasoning`, `think-tags`, or `none` |
| `reasoning.preserve_in_history` | `"all_turns"` | Replay earlier reasoning: `"never"` (`false`), `"current_turn"`, or `"all_turns"` (`true`) |
| `reasoning.wire` | `"effort"` | How effort serializes on `openai-chat`: `effort`, `enable-thinking`, or `none` |
| `reasoning.effort` | unset (server default) | Default thinking effort; explicit values take precedence over effort in `extra_body` |
| `reasoning.efforts` | `[]` | Supported effort values in picker order; enables `/thinking` choices without assuming model capabilities |
| `prompt_cache` | from provider | Per-model prompt-cache override (replaces provider `[prompt_cache]` when set) |
| `sampling` | from profile | Model parameters (`temperature`, `top_p`, `top_k`, `min_p`, `repetition_penalty`) |
| `serving.prefix_cache`, `prefill_tps` | `true`, unset | KV prefix-cache reuse and prefill throughput hints for `cache_strategy = "auto"` |
| `cost` | from server, then catalog | Cost per million tokens: `input`, `output`, `cached_input`, `cache_write` |

For example, add `reasoning = { effort = "medium", efforts = ["low", "medium", "high"] }`
to a model entry when the server supports those levels. The bundled OpenAI API and
ChatGPT models declare their supported levels. Selections apply between turns;
`/new` starts with the configured defaults.

---

### `mcp.d/<id>.toml`

MCP servers live as individual `<id>.toml` files in your user config folder (`mcp.d/<id>.toml`, printed by `quack config paths`) or, per repository, in `.quack/mcp.d/<id>.toml` (requires `quack trust`). Bundled examples ship in `defaults/mcp.d/` with `enabled = false` (`context7`, `roost`, `github`, `fetch`, `playwright`, `filesystem`, `time`, `everything`).

**Three ways to add or enable an MCP server:**

1. **In the TUI (`/mcp`)**: Open `/mcp` to browse live servers alongside bundled disabled examples and `+ Add remote MCP server (HTTP)` / `+ Add local MCP server (stdio)`. Pressing `Enter` on an example copies, enables, and connects it right away without restarting quack; `/mcp add`, `/mcp enable <id>`, `/mcp disable <id>`, and `/mcp restart <id>` also work directly, and `Ctrl-L` (or `--project` / `--user`) chooses between user `mcp.d/` and project `.quack/mcp.d/`.
2. **From the CLI (`quack mcp`)**:
   ```sh
   quack mcp list                                       # show configured servers & bundled examples
   quack mcp add                                        # interactive guided setup
   quack mcp enable context7                            # enable a bundled example
   quack mcp add context7 --url https://mcp.context7.com/mcp
   quack mcp add roost --url https://roost.internal:8484/mcp \
     --header "Authorization=Bearer {env:ROOST_TOKEN}" --auto-approve "list_*,search_*,get_*,read_*"
   quack mcp add playwright -- npx -y @playwright/mcp@latest
   quack mcp login linear                               # OAuth 2.1 + PKCE sign-in for [oauth] server
   quack mcp logout linear                              # revoke and delete saved OAuth tokens
   ```
3. **By editing TOML directly**:

   Remote HTTP example (`~/.config/quack/mcp.d/context7.toml`):
   ```toml
   description = "Up-to-date library documentation"
   url = "https://mcp.context7.com/mcp"
   # Optional higher rate limits:
   # headers = { CONTEXT7_API_KEY = { env = "CONTEXT7_API_KEY" } }

   [tools]
   auto_approve = ["*"]
   ```

   OAuth 2.1 + PKCE HTTP example (`~/.config/quack/mcp.d/linear.toml`, then run `quack mcp login linear` or `/mcp login linear`):
   ```toml
   description = "Linear issues and projects"
   url = "https://mcp.linear.app/mcp"

   [oauth]
   # Uses RFC 9728/8414 discovery + RFC 7591 Dynamic Client Registration automatically;
   # optionally set client_id, client_secret, scopes, authorization_server, redirect_port.
   ```

   Self-hosted HTTP example (`~/.config/quack/mcp.d/roost.toml`, or run `quack mcp enable roost`):
   ```toml
   description = "Self-hosted internal document search (Roost)"
   url = "https://roost.internal:8484/mcp"
   headers = { Authorization = { env = "ROOST_TOKEN", format = "Bearer {}" } }

   [tools]
   auto_approve = ["list_*", "search_*", "get_*", "read_*"]
   ```

   Values in `headers` and `env` support `{ env = "VAR", format = "Bearer {}" }`, `{ keyring = "quack/name" }`, `{ file = "~/token.txt" }`, `{ cmd = ["pass", "show", "..."] }`, or a plain string.

| Key | Default | Description |
|---|---|---|
| `enabled` | `true` | Set `false` to keep the config without connecting |
| `description` | `""` | Shown in `quack mcp list`, `/mcp`, and `tool_search` |
| `id` | file stem | Override server id used in tool/command names |
| `command`, `args` | | Executable path and arguments for stdio MCP servers |
| `url` | | Endpoint URL for remote streamable HTTP MCP servers |
| `headers` | `{}` | HTTP headers for remote servers (`Secret` values supported) |
| `oauth` | none | OAuth 2.1 + PKCE table (`enabled`, `client_id`, `client_secret`, `scopes`, `authorization_server`, `redirect_port`) for remote HTTP servers |
| `env` | `{}` | Environment variables passed to stdio server process (`Secret` values supported) |
| `cwd` | workspace root | Process working directory (`~/` expanded) |
| `inherit_env` | OS safe defaults | Explicit list of host env var names to pass through |
| `sandbox` | `false` | Run local stdio server inside the OS sandbox |
| `defer` | `"auto"` | Defer tools behind `tool_search`: `auto` (above 10% budget), `always`, or `never` |
| `sampling_model` | `"current"` | Model serving `sampling/createMessage`: `current` or `small` (`[model] small`) |
| `tools.allow`, `tools.deny` | `[]` | Globs filtering exposed tool definitions |
| `tools.auto_approve` | `[]` | Globs of tool names that execute without asking approval |
| `startup_timeout_secs` | `20` | Server initialization timeout |
| `call_timeout_secs` | `120` | Per-tool-call timeout (`0` disables) |
| `max_output_tokens` | `8000` | Token cap applied to MCP tool and resource outputs (full output spills to a temp file when trimmed) |

---

### `lsp.d/<id>.toml`

| Key | Default | Description |
|---|---|---|
| `command`, `args` | required | Language server binary and startup arguments |
| `extensions` | required | Covered file extensions without leading dot (`["rs"]`, `["py"]`) |
| `root_markers` | `[]` | Project root boundary files (`Cargo.toml`, `go.mod`, `pyproject.toml`) |
| `language_ids` | `{}` | Explicit LSP `languageId` overrides per extension |
| `env` | `{}` | Extra environment variables (`Secret` values supported) |
| `initialization_options` | `{}` | LSP initialization parameters |
| `settings` | `{}` | Answers to `workspace/configuration` requests |
| `sandbox` | `false` | Run language server inside the OS sandbox |
| `diagnostics.settle_ms` | `800` | Settle delay before querying diagnostics after edits |
| `diagnostics.max_wait_ms` | `5000` | Maximum wait for diagnostics after edits |
| `diagnostics.min_severity` | `"warning"` | Minimum diagnostic level to report: `error`, `warning`, `info`, or `hint` |
| `diagnostics.max_items` | `20` | Maximum diagnostics per file appended to edit results |

---

### `hooks.d/<id>.toml`

A hook ships disabled (`enabled = false`). `/hooks` opens its setup form; Ctrl-S saves
without changing its enabled state, Ctrl-E saves and enables, and Ctrl-T checks a
sample URL against the current draft without saving or calling a model. The form
supports masked tokens, OS credential storage with private-file fallback, external
credential sources, multiple instances, rotation, and per-connection enable/disable.
Changes apply after any running turn, without restarting MCP or LSP services.
`/hooks configure <id>`, `/hooks test <id>`, `/hooks toggle <id>`, keyring commands and
`/reload` remain available. Python 3 is required by the shipped link handlers.

Enabling a shipped hook copies its file
and the folder named after it (e.g. `jira/` next to `jira.toml`) into your
config folder's `hooks.d/` with `enabled = true`. If the payload folder already
exists, choose whether to keep it or replace it with the bundled folder. Enabling
clears the hook's disabled entries in active user and project settings. Quack reloads
and confirms the hook is active. Turning a hook off lists its id in `[hooks] disabled`.

| Key | Default | Description |
|---|---|---|
| `enabled` | `true` | `false` disables the hook in place (the shipped examples set this) |
| `description` | `""` | Shown in `/hooks` |
| `event` | required | Target event: `session_start`, `user_prompt`, `pre_tool`, `post_tool`, `turn_end`, `session_end`, `pre_compact` |
| `command` | required | Executable and arguments (`{dir}` is hook directory, `{python}` is system Python) |
| `match.tool`, `match.pattern` | `""` | Tool name glob and argument pattern matchers |
| `match.prompt` | `""` | Regular expression for `user_prompt` events |
| `timeout_secs` | `10` | Maximum hook execution duration |
| `full_context` | `false` | Preserve full stdout and added context, warning above 32,000 estimated tokens (other hooks truncate at 8,000) |
| `setup` | none | Versioned, declarative TUI fields and service presets; optional for custom hooks |
| `connections` | none | Connection settings and credential references, passed to the script on stdin with credentials in its environment; an explicit empty list disables all connections |
| `on_error` | `"warn"` | Failure behavior: `warn`, `allow` (silent continue), or `deny` |

---

### `agents.d/<id>.toml`

| Key | Default | Description |
|---|---|---|
| `description` | required | Sub-agent role description used by caller model |
| `prompt` | `""` | Supplementary system instructions for the sub-agent |
| `tools` | all | Allowed tool names (e.g. `["read", "grep", "index"]`) |
| `read_only` | `false` | Restrict agent to read-only tools; permits parallel runs |
| `model` | caller model | Model override (`small` or `<provider>/<model>`) |
| `max_steps` | `250` | Maximum model calls before stopping (runaway guard; shipped `explore` = 250, `general` = 500) |
| `reasoning_effort` | inherit | Optional reasoning effort override (e.g. `"low"` on shipped `explore`) |

---

## Development

If you have [`task`](https://taskfile.dev) (`go-task`) installed, [Taskfile.yml](Taskfile.yml) (plus [crates/quack-gui/Taskfile.yml](crates/quack-gui/Taskfile.yml) and [website/Taskfile.yml](website/Taskfile.yml)) wraps the common TUI, desktop GUI, and website workflows:

```sh
# List all tasks
task

# Build TUI (`quack`), desktop GUI (`quack-gui`), or both
task build-tui
task build-gui         # builds crates/quack-gui/ui + quack-gui (--features desktop)
task build             # builds both (add -release suffix for release builds)

# Run from checkout (automatically points QUACK_DEFAULTS_DIR at defaults/)
task run-tui
task run-gui
task gui-dev           # Vite HMR dev server + live-reloading Tauri desktop window
task gui-ui-dev        # GUI frontend Vite dev server only (browser scripted mode)
task website-dev       # marketing website Vite dev server

# Build from source & install into ~/.local (wraps scripts/install.sh --from-source)
task install           # installs both TUI and GUI
task install-tui       # TUI only
task install-gui       # GUI only

# Checks & tests
task check             # fmt, clippy, workspace tests, and GUI checks
task gui:protocol-ts   # regenerate ui/src/lib/protocol.ts from quack-protocol
```

On glibc Linux, building or running the desktop GUI requires `libwebkit2gtk-4.1-dev` and `libgtk-3-dev` (plus Node.js and `pnpm` or `npm`).

Direct Cargo commands (without `task`):

```sh
# Build CLI/TUI binary (default-members is quack-cli)
cargo build

# Build desktop GUI (requires built SPA in crates/quack-gui/ui/build)
(cd crates/quack-gui/ui && pnpm install --frozen-lockfile && pnpm run build)
cargo build -p quack-gui --features desktop

# Run interactive TUI with bundled defaults from checkout
QUACK_DEFAULTS_DIR=defaults cargo run

# Run desktop GUI with bundled defaults from checkout
QUACK_DEFAULTS_DIR=defaults target/debug/quack-gui

# Run headless turn
cargo run -- exec "explain src/main.rs"

# Run tests, linter, and formatter
QUACK_DEFAULTS_DIR=none cargo test --workspace --locked
RUSTFLAGS="-D warnings" cargo clippy --workspace --all-targets --locked
cargo fmt --all --check
```

### Repository Layout

| Directory | Contents |
|---|---|
| `crates/` | Modular Rust crates (`quack-core`, `quack-tui`, `quack-tools`, `quack-providers`, etc.) |
| `defaults/` | Default configurations, model profiles, MCP servers, and JSON schemas |
| `scripts/` | Installation scripts, CI helpers, and packaging tools |
| `evals/` | Task evaluation suites (`evals/tasks`, `evals/hard`) and benchmarking runners |
| `docs/plan/` | Architecture specs and technical design documents |

---

## License

Apache-2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).
