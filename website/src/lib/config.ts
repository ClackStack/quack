// The Config plate's content. It mirrors the README's Configuration section, which in turn
// follows crates/quack-config (settings.rs, provider.rs, mcp.rs, lsp.rs, hooks.rs, agent.rs).
// Text in backticks renders as code.

export const providerFile = `# /etc/quack/providers.d/corp.toml
name = "Corp LLM"
base_url = "https://llm.internal.example:8000/v1"
api_key = { env = "CORP_LLM_KEY" }   # leave out if no key
models = [
  { id = "qwen3-coder-480b", alias = "coder" },
  { id = "qwen3-coder-30b", alias = "fast" },
]`;

export const policyFile = `# /etc/quack/config.toml
[model]
default = "corp/coder"
small = "corp/fast"

[network]
offline = true

[sandbox]
mode = "workspace-write"     # commands: no network

[permissions]
deny = ["shell:curl *", "shell:wget *"]

[audit]
path = "/var/log/quack/audit.jsonl"

[policy]
locked = [
  "model.default", "network",
  "sandbox.mode", "permissions.deny", "audit",
]
max_permission_mode = "allow"       # no yolo
project_config = "trusted-only"

[policy.providers]
allowed = ["corp"]
hosts = ["llm.internal.example"]

[policy.mcp]
hosts = ["*.internal.example"]`;

export const installCmd = `curl -fsSL https://quack.clackstack.dev/install.sh | sh -s -- --plucked`;

export const doctorOutput = `$ quack doctor
ok    defaults   /usr/share/quack/defaults
warn  providers  1 of 2 usable
ok      corp     2 declared models
warn    rogue    not allowed by policy
ok    model      corp/coder
ok    agents     explore (on corp/fast), general
ok    sandbox    workspace-write · Landlock + seccomp
ok    policy     5 locked · max approvals: allow · offline
ok    audit      /var/log/quack/audit.jsonl (metadata only)
info  network    llm.internal.example · policy allows only
                 llm.internal.example, *.internal.example`;

export const guarantees: [string, string][] = [
	['Only what you configure', 'No implicit telemetry, background update checks, or silent model downloads. A recording-proxy test and a loopback-only network namespace check every build, and the Session Egress Ledger logs every outbound request and block live.'],
	['Hosts and redirects stay in policy', 'When `[policy.providers] hosts` is set, every `base_url` / `base_urls` endpoint pointing anywhere else is rejected regardless of layer — and cross-host HTTP redirects (plus OAuth discovery and opt-in `web_fetch`) must pass the egress gate.'],
	['Locks hold everywhere', 'User files, trusted repo `.quack/` writes (`Ctrl-L` / `/scope`), environment variables, `-c` flags, and `/config` cannot change a locked key such as `[network] secret_guard` or `[audit]`. `QUACK_SYSTEM_DIR` is ignored under an active policy, and Windows reads `HKLM\\SOFTWARE\\Policies\\quack` directly.'],
	['Sandboxed commands & screened prompts', 'Shell commands run inside an OS sandbox: Landlock + seccomp (no network) on Linux, Seatbelt (no network) on macOS, and Restricted Token + Low Integrity + Job Objects (workspace writes) on Windows — while Pre-Egress Secret Shield screens outbound prompts, `@` slices, and tool output before sending.']
];

export const layers: [string, string, string][] = [
	['Defaults bundle', '`share/quack/defaults/` beside the binary, or `QUACK_DEFAULTS_DIR`', '`--plucked` leaves out hosted services'],
	['System', '`/etc/quack/` · `/Library/Application Support/quack/` · `%ProgramData%\\quack\\` + `HKLM\\SOFTWARE\\Policies\\quack`', 'The only layer whose `[policy]` counts'],
	['User', '`~/.config/quack/` · `%APPDATA%\\quack\\`', 'Your own files'],
	['Project', '`.quack/` at the repository root', 'Loads only once you trust the repo'],
	['Environment', '`QUACK_MODEL`, `QUACK_PROMPT_CACHE_KEY`, `QUACK_CONFIG_DIR`', 'Runtime environment overrides'],
	['Command line', '`-m`, `--profile`, `--mode`, `--approvals`, `--worktree`, `--cache-key`, `-c key=value`', 'This run only']
];

export const folder = `config.toml          settings
providers.d/*.toml   model servers and services
mcp.d/*.toml         MCP servers
lsp.d/*.toml         language servers
hooks.d/*.toml       programs that run on agent events
agents.d/*.toml      sub-agents for the task tool
profiles.d/<id>/     prompts and tool shapes per model family
commands/*.md        your slash commands
skills/<name>/       standard Agent Skills (SKILL.md)
bundles/<name>/      groups of the above
AGENTS.md            instructions for every project`;

export type Group = { title: string; file: string; rows: [string, string, string][] };

export const reference: Group[] = [
	{
		title: 'Models',
		file: 'config.toml [model]',
		rows: [
			['default', '""', 'Model to start with: `<provider>/<model>` or `<provider>/<alias>`'],
			['small', '""', 'Titles, summaries and the explore sub-agent. Empty: `default`'],
			['planner', '""', 'Writes the plan in plan mode; your usual model carries it out'],
			['remember_last', 'true', 'Start with the model you last switched to'],
			['profile', '""', 'One profile for every model, e.g. `generic` or `qwen-code`']
		]
	},
	{
		title: 'Permissions',
		file: 'config.toml [permissions]',
		rows: [
			['work_mode', '"plan"', '`plan` reads/searches and proposes tests for review; `edit` may change files and run commands'],
			['mode', '"allow"', 'Approvals: `ask` before unapproved actions, or `allow` within the selected work mode, sandbox and rules'],
			['allow', '[]', 'Rules that run without asking: `shell:cargo test*`, `edit:src/**`, `mcp:github/get_*`'],
			['ask', '[]', 'Rules that always ask, even in `allow`; yolo bypasses asks'],
			['deny', '[]', 'Rules that never run, in any mode. A built-in deny list always applies too']
		]
	},
	{
		title: 'Sandbox',
		file: 'config.toml [sandbox]',
		rows: [
			['mode', '"auto"', '`auto`, `workspace-write` (no network), `workspace-write-net`, `read-only` or `off`. A mode you set fails closed'],
			['writable', '[]', 'More folders commands may write to, e.g. `["~/.cargo"]`']
		]
	},
	{
		title: 'Network',
		file: 'config.toml [network]',
		rows: [
			['offline', 'false', 'Block all egress except provider base URLs, loopback and `egress_allow`'],
			['egress_allow', '[]', 'Host globs allowed while offline (applies to cross-host HTTP redirects too)'],
			['http1_only', 'false', 'Speak HTTP/1.1 only, one connection per request; try when a VPN or proxy leaves HTTP/2 stale'],
			['proxy', '""', 'Proxy for model requests. Empty: `HTTPS_PROXY` or the system’s (`NO_PROXY` always direct)'],
			['ca_bundle', '""', 'PEM file of extra root certificates, for an internal CA'],
			['secret_guard', '"ask"', 'Pre-egress secret scan: `ask`, `redact`, or `off` (headless falls back from `ask` to `redact`)'],
			['secret_guard_local', 'false', 'Apply `secret_guard` even when talking to loopback servers (`127.0.0.1` / `localhost`)'],
			['secret_allowlist', '[]', 'SHA-256 fingerprints (`sha256:…`) trusted to pass without secret-guard prompts']
		]
	},
	{
		title: 'Policy',
		file: 'config.toml [policy], system layer only',
		rows: [
			['locked', '[]', 'Dotted keys no other layer can change; a table locks all of it'],
			['max_permission_mode', 'unset', 'Approval ceiling: `allow` prevents yolo. Lock `permissions.work_mode` separately to enforce Plan'],
			['project_config', '"trusted-only"', '`trusted-only`, `never` or `always`: whether a repo’s `.quack/` loads'],
			['providers.allowed', '[]', 'Provider id globs that may load. Empty: all'],
			['providers.hosts', '[]', 'Host globs a provider’s `base_url` must match. Empty: any'],
			['mcp.allowed', '[]', 'MCP server id globs that may load'],
			['mcp.hosts', '[]', 'Host globs a remote MCP server’s `url` must match']
		]
	},
	{
		title: 'Context',
		file: 'config.toml [context]',
		rows: [
			['window', 'unset', 'Use at most this many tokens of the model’s window'],
			['budget_ratio', '0.85', 'Share of the window quack plans to use'],
			['compaction_threshold', '0.80', 'Share of the budget at which pruning, then compaction, starts'],
			['cache_strategy', '"auto"', '`auto` or `preserve` keep the prompt cache warm; `aggressive` prunes every request'],
			['cache_key', 'unset', 'Shared prompt-cache routing key for parallel `quack exec` / CI runs (`--cache-key` / `QUACK_PROMPT_CACHE_KEY`)']
		]
	},
	{
		title: 'Agent',
		file: 'config.toml [agent]',
		rows: [
			['max_steps', '100', 'Model calls before a turn pauses and asks whether to continue (headless stops instead; sub-agents use their own limit)']
		]
	},
	{
		title: 'Tools',
		file: 'config.toml [tools]',
		rows: [
			['edit_format', '"auto"', '`str_replace`, `hashline`, `apply_patch`, `search_replace_blocks` or `whole_file`'],
			['shell', '"auto"', '`bash`, `zsh`, `pwsh`, `powershell`, `cmd`, or a path'],
			['default_timeout_secs', '120', 'When a shell command’s process tree is killed'],
			['read_outline_tokens', '2000', 'Larger source files are read as an outline first. `0`: off'],
			['web_fetch.enabled', 'false', 'Opt-in SSRF-protected `web_fetch` tool; always unregistered while `[network] offline = true`'],
			['web_fetch.allow_private', 'false', 'Allow loopback, RFC1918 private, link-local, and cloud-metadata IPs'],
			['web_fetch.max_bytes / timeout_secs', '2 MiB / 20', 'Response size limit and per-request timeout for `web_fetch`'],
			['acp_client_fs / acp_client_terminal', 'false', 'Under `quack acp`, route `read`/`write` or foreground `shell` through the editor when advertised']
		]
	},
	{
		title: 'Audit',
		file: 'config.toml [audit]',
		rows: [
			['path', '""', 'Mode-`0600` JSONL file appending one structured record per tool execution (empty disables)'],
			['include_content', 'false', 'Include tool arguments and result summaries in audit entries (omitted by default)']
		]
	},
	{
		title: 'Interface',
		file: 'config.toml [ui]',
		rows: [
			['theme', '"auto"', '`dark`, `light`, or a theme name from `themes/`'],
			['show_reasoning', '"collapsed"', '`collapsed`, `expanded` or `hidden`'],
			['startup_panel', '"auto"', 'Startup diagnostics: in the sidebar until the first turn; on narrow terminals only on problems. `always` or `never`'],
			['ascii', 'false', 'ASCII instead of box-drawing glyphs'],
			['mouse', 'true', '`false` leaves the mouse to the terminal'],
			['reduced_motion', 'false', 'Static indicators instead of spinners'],
			['terminal_title', 'true', 'Window title shows the duck and the session title. `false`: leave it alone'],
			['sidebar', 'true', 'Session details on the right at 140 columns × 24 rows and above, growing from 36 up to 48 columns on wider terminals. `false`: use the full width for the conversation'],
			['gui_restore_tabs', 'true', 'Restore previously opened workspace tabs when launching `quack-gui`']
		]
	},
	{
		title: 'Catalog & updates',
		file: 'config.toml [catalog] / [updates]',
		rows: [
			['catalog.refresh', 'false', 'Opt-in daily background refresh of the model catalog snapshot from `catalog.url` (`quack catalog refresh` runs on demand)'],
			['catalog.url', '""', 'models.dev snapshot URL (defaults bundle sets it; `--plucked` leaves it empty)'],
			['updates.check', 'false', 'Opt-in daily check against `updates.url` for new versions (`quack update` checks on demand)'],
			['updates.url', '""', 'Release redirect URL (defaults bundle sets GitHub releases; `--plucked` leaves it empty)']
		]
	},
	{
		title: 'Instructions, skills, commands, memory, sessions',
		file: 'config.toml',
		rows: [
			['instructions.files', '["AGENTS.md"]', 'Instruction file names, in order of preference'],
			['instructions.other_agents', 'true', 'Read CLAUDE.md, GEMINI.md, `.cursorrules` where there’s no AGENTS.md'],
			['instructions.max_bytes', '32768', 'Cap on each instruction file'],
			['skills.compat_paths', '[]', 'Extra `SKILL.md` folders beyond `.agents/skills/`, `~/.agents/skills/` and `skills/`, e.g. `.claude/skills`'],
			['skills.disabled', '[]', 'Skill names to skip'],
			['skills.index_max_tokens', '1500', 'Token cap for the skill index in the prompt'],
			['commands.compat_paths', '[]', 'Extra Markdown command folders (e.g. `.claude/commands`)'],
			['memory.path', '""', 'Where notes live, e.g. a synced folder'],
			['memory.project_scope', '"data-dir"', '`repo` keeps project notes in `.quack/memory`'],
			['memory.writes', '"auto"', '`ask` makes each note ask first'],
			['memory.index_max_tokens', '1500', 'Token cap for the memory index in the prompt'],
			['memory.nudge', 'true', 'Remind the model to save lasting preferences'],
			['sessions.path', '""', 'Where sessions are saved'],
			['sessions.retention_days', '0', 'Prune sessions older than N days automatically on startup (`0`: keep forever)'],
			['defaults.dir', '""', 'The defaults bundle’s location; `"none"` turns it off']
		]
	},
	{
		title: 'A provider',
		file: 'providers.d/<id>.toml',
		rows: [
			['base_url', 'required (or base_urls)', 'The API root, e.g. `https://llm.internal.example:8000/v1`'],
			['base_urls', '[]', 'Extra endpoint URLs for round-robin balancing and exponential health-cooldown failover'],
			['api', '"openai-chat"', '`openai-chat`, `openai-responses`, `anthropic-messages` or `ollama`'],
			['api_key', 'none', 'A secret: `{ env = … }`, `{ file = … }`, `{ cmd = […] }` or `{ keyring = … }`'],
			['auth', 'none', 'Browser sign-in instead of a key (e.g. `{ type = "chatgpt", issuer = "https://auth.openai.com" }`)'],
			['models', '[]', 'Ids or tables. Empty: listed from the server'],
			['headers', '{}', 'Extra headers; values may be secrets'],
			['extra_body', '{}', 'Merged into every request'],
			['native_compaction / responses_websocket', 'false', 'Native Responses compaction and persistent WebSocket continuation (on for bundled OpenAI/ChatGPT)'],
			['prompt_cache', '{}', '`developer_instructions`, `explicit_breakpoints`, `ttl` (or Ollama `keep_alive`), `cache_key`, `affinity_header`, `cache_control`'],
			['tls.ca_file', 'none', 'An extra CA for this server'],
			['tls.client_cert / client_key', 'none', 'Mutual TLS'],
			['http.timeout_connect_secs', '30', 'TCP connect, proxy handshake and TLS timeout across addresses'],
			['http.timeout_first_byte_secs', '0', 'Wait for response headers before retrying (`0`: use `timeout_idle_secs`)'],
			['http.timeout_idle_secs', '300', 'How long the stream may go quiet'],
			['http.max_retries', '3', 'Retries, honouring `Retry-After`'],
			['http.proxy / no_proxy / http1_only', '"" / false / unset', 'Per-provider proxy URL, direct-connection bypass, and HTTP/1.1 override'],
			['discovery.enabled / include', 'false / []', 'Query `/models` at startup and filter by model ID globs']
		]
	},
	{
		title: 'A model',
		file: 'models = [{ id = …, … }]',
		rows: [
			['alias', 'none', 'A short name: `corp/coder`'],
			['context_window / max_output', 'from the server', 'Token limits'],
			['compact_at', 'unset', 'Soft prompt-token ceiling for automatic compaction (e.g. `250000` below a 272k cliff)'],
			['profile', '"auto"', 'A profile id, or `auto` to pick by its `[match]` globs'],
			['vision', 'false', 'Offer images to the model'],
			['tool_calling.mode / parallel', '"native" / true', '`native`, `text` or `grammar` (schema-guided decoding); `parallel = false` disables parallel calls'],
			['tool_calling.text_format', '"auto"', '`hermes`, `qwen3-xml`, `glm`, `json-block`, `deepseek` or `kimi`'],
			['reasoning.format / wire', '"auto" / "effort"', 'Thinking parser and wire format (`effort`, `enable-thinking`, `none`)'],
			['reasoning.preserve_in_history', '"all_turns"', 'Replay earlier reasoning: `"never"`, `"current_turn"`, or `"all_turns"`'],
			['reasoning.effort / efforts', '"" / []', 'Default thinking effort and supported levels in `/thinking` picker order'],
			['prompt_cache', 'from provider', 'Per-model prompt-cache override'],
			['sampling', 'from the profile', '`temperature`, `top_p`, `top_k`, `min_p`, `repetition_penalty`'],
			['serving.prefix_cache / prefill_tps', 'true / unset', 'KV prefix-cache reuse and prefill throughput hints for `cache_strategy = "auto"`'],
			['cost', 'from the server', 'Dollars per million tokens (`input`, `output`, `cached_input`, `cache_write`), for `/stats`']
		]
	},
	{
		title: 'An MCP server',
		file: 'mcp.d/<id>.toml (or `quack mcp add` / `/mcp`)',
		rows: [
			['enabled', 'true', 'Set `false` to keep the file without connecting'],
			['description', '', 'Shown in `quack mcp list`, `/mcp`, and `tool_search`'],
			['command / args', '', 'A local server, over stdio'],
			['url', '', 'A remote streamable HTTP server, instead'],
			['headers / env', '{}', 'Values may be secrets (`env`, `keyring`, `file`, `cmd`)'],
			['oauth', 'none', 'OAuth 2.1 + PKCE table (`enabled`, `client_id`, `client_secret`, `scopes`, `authorization_server`, `redirect_port`); sign in with `quack mcp login <id>`'],
			['sandbox', 'false', 'Run a local server in the OS sandbox'],
			['defer', '"auto"', 'Defer tools behind `tool_search`: `auto`, `always`, or `never`'],
			['sampling_model', '"current"', 'Model serving `sampling/createMessage`: `current` or `small`'],
			['max_output_tokens', '8000', 'Token cap for MCP tool/resource output (full output spills to a temp file when trimmed)'],
			['tools.allow / deny', '[]', 'Tool name globs to offer or hide'],
			['tools.auto_approve', '[]', 'Tool name globs that run without asking']
		]
	},
	{
		title: 'A language server',
		file: 'lsp.d/<id>.toml',
		rows: [
			['command / args', 'required', 'Used only when it’s on `PATH`; nothing is installed'],
			['extensions', 'required', 'File extensions it covers, without the dot'],
			['root_markers', '[]', 'Files that mark a project root'],
			['initialization_options / settings', '{}', 'LSP initialization parameters and `workspace/configuration` answers'],
			['sandbox', 'false', 'Run the language server inside the OS sandbox'],
			['diagnostics.min_severity', '"warning"', '`error`, `warning`, `info` or `hint` (with `settle_ms` 800, `max_wait_ms` 5000)']
		]
	},
	{
		title: 'A hook',
		file: 'hooks.d/<id>.toml',
		rows: [
			['event', 'required', '`session_start`, `user_prompt`, `pre_tool`, `post_tool`, `turn_end`, `session_end`, `pre_compact`'],
			['command', 'required', 'Program and arguments; `{dir}` is the file’s folder, `{python}` the Python on `PATH`'],
			['match', '{}', '`tool` and `pattern` globs for tool events, a `prompt` regex for prompts'],
			['timeout_secs', '10', 'How long it may run'],
			['on_error', '"warn"', '`warn`, `allow` or `deny`'],
			['full_context', 'false', 'Keep stdout and added context in full (warns above 32,000 estimated tokens)'],
			['setup / connections', 'none', 'Declarative `/hooks` form fields and saved preset bindings with secret references']
		]
	},
	{
		title: 'A sub-agent',
		file: 'agents.d/<id>.toml',
		rows: [
			['description', 'required', 'What it’s for; the model reads it to choose'],
			['prompt', '""', 'Added to the system prompt for its run'],
			['tools', 'all', 'Tool names it may use'],
			['read_only', 'false', 'Change nothing; may run in plan mode and in parallel'],
			['model', 'the caller’s', '`small` or `<provider>/<model>`'],
			['max_steps', '250', 'Model calls before it stops and reports (runaway guard; `explore` 250, `general` 500)'],
			['reasoning_effort', 'inherit', 'Optional effort override (e.g. `"low"` on shipped `explore`)']
		]
	}
];
