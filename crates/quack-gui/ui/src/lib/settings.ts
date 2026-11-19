import type { AppInfo, PathsSnapshot } from './transport/types';

export type SettingRow = AppInfo['settings'][number];

export type SettingsSectionId =
	| 'paths'
	| 'setup'
	| 'context'
	| 'network'
	| 'permissions'
	| 'extensions'
	| 'interface';

export type SettingKind =
	| 'path'
	| 'path-list'
	| 'list'
	| 'select'
	| 'bool'
	| 'int'
	| 'nullable-int'
	| 'float'
	| 'text'
	| 'model'
	| 'profile'
	| 'theme';

export interface SelectOption {
	value: string;
	label: string;
	detail?: string;
}

export interface SettingFieldDef {
	key: string;
	section: SettingsSectionId;
	group: string;
	label: string;
	kind: SettingKind;
	defaultValue: string;
	fallbackDescription: string;
	options?: readonly SelectOption[];
	placeholder?: string;
	emptyMeaning?: string;
	hint?: string;
	min?: number;
	max?: number;
	step?: number;
	tuiOnly?: boolean;
	supportsNullReset?: boolean;
}

export interface SettingsSectionMeta {
	id: SettingsSectionId;
	title: string;
	shortLabel: string;
	kicker: string;
	summary: string;
}

export interface ParsedOrigin {
	layer: 'default' | 'system' | 'user' | 'project' | 'env' | 'cli' | 'other';
	label: string;
	detail: string;
}

export interface ResolvedPathRow {
	id: keyof PathsSnapshot | 'cwd';
	label: string;
	path: string;
	hint: string;
}

export const SETTINGS_SECTIONS: readonly SettingsSectionMeta[] = [
	{
		id: 'paths',
		title: 'Paths & Storage',
		shortLabel: 'Paths',
		kicker: 'Files & directories',
		summary:
			'Configure where memories, session logs, instructions, skills, and defaults bundles are read and stored.'
	},
	{
		id: 'setup',
		title: 'Setup & Models',
		shortLabel: 'Setup',
		kicker: 'quack setup & defaults',
		summary:
			'Default model, planner split, prompt profile, and opt-in background checks offered by `quack setup`.'
	},
	{
		id: 'context',
		title: 'Agent, Context & Tools',
		shortLabel: 'Agent & Tools',
		kicker: 'Execution & prompt budget',
		summary:
			'Turn limits, context window caps, cache compaction strategy, shell binary, and edit tool format.'
	},
	{
		id: 'network',
		title: 'Network & Telemetry',
		shortLabel: 'Network',
		kicker: 'Egress & observability',
		summary:
			'Offline mode, outbound host allowlists, proxy / custom CA certificates, and OpenTelemetry export.'
	},
	{
		id: 'permissions',
		title: 'Permissions, Sandbox & Bundles',
		shortLabel: 'Permissions',
		kicker: 'Guardrails & toggles',
		summary:
			'Default approval policy, OS sandbox scope, permission rule lists, bundles, and disabled extensions.'
	},
	{
		id: 'extensions',
		title: 'Hooks & MCP Servers',
		shortLabel: 'Hooks & MCP',
		kicker: 'Tools, integrations & lifecycles',
		summary:
			'Inspect, enable, restart, and configure MCP servers (`mcp.d/*.toml`) and lifecycle hooks (`hooks.d/*.toml`).'
	},
	{
		id: 'interface',
		title: 'Interface & All TOML',
		shortLabel: 'Interface & TOML',
		kicker: 'Display & raw config',
		summary:
			'Desktop & terminal UI preferences plus every effective TOML key from config layers.'
	}
];

export const SETTING_FIELDS: readonly SettingFieldDef[] = [
	// ── Paths & Storage ─────────────────────────────────────────────────
	{
		key: 'memory.path',
		section: 'paths',
		group: 'Memory storage',
		label: 'Memory directory',
		kind: 'path',
		defaultValue: '""',
		placeholder: '<user data>/memory',
		emptyMeaning: 'Uses <user data>/memory',
		fallbackDescription:
			'Where memory notes live. Empty stores global notes under `<user data>/memory`.'
	},
	{
		key: 'memory.project_scope',
		section: 'paths',
		group: 'Memory storage',
		label: 'Project memory location',
		kind: 'select',
		defaultValue: '"data-dir"',
		options: [
			{
				value: 'data-dir',
				label: 'data-dir',
				detail: 'Under <memory.path>/projects/<key> (private to this machine)'
			},
			{
				value: 'repo',
				label: 'repo',
				detail: 'Under <workspace>/.quack/memory (easy to share in git)'
			}
		],
		fallbackDescription:
			'Where project-scoped memory notes are stored relative to your workspace or data dir.'
	},
	{
		key: 'memory.writes',
		section: 'paths',
		group: 'Memory storage',
		label: 'Memory write approval',
		kind: 'select',
		defaultValue: '"auto"',
		options: [
			{ value: 'auto', label: 'auto', detail: 'Save notes immediately; easy to undo' },
			{ value: 'ask', label: 'ask', detail: 'Ask permission before writing a memory note' }
		],
		fallbackDescription: 'Whether the model may save memory notes without asking.'
	},
	{
		key: 'memory.index_max_tokens',
		section: 'paths',
		group: 'Memory storage',
		label: 'Memory index token budget',
		kind: 'int',
		defaultValue: '1500',
		min: 0,
		step: 100,
		fallbackDescription: 'Maximum tokens of MEMORY.md index injected into startup context.'
	},
	{
		key: 'memory.nudge',
		section: 'paths',
		group: 'Memory storage',
		label: 'Memory reminder nudges',
		kind: 'bool',
		defaultValue: 'true',
		fallbackDescription:
			'Remind the model to save a note when your message states a preference or correction.'
	},
	{
		key: 'sessions.path',
		section: 'paths',
		group: 'Session transcripts',
		label: 'Sessions directory',
		kind: 'path',
		defaultValue: '""',
		placeholder: '<user data>/sessions',
		emptyMeaning: 'Uses <user data>/sessions',
		fallbackDescription: 'Where JSONL transcripts and prompt history live.'
	},
	{
		key: 'sessions.retention_days',
		section: 'paths',
		group: 'Session transcripts',
		label: 'Retention days',
		kind: 'int',
		defaultValue: '0',
		min: 0,
		step: 1,
		hint: '0 keeps saved sessions forever.',
		fallbackDescription: 'How many days before old sessions expire (0 keeps forever).'
	},
	{
		key: 'instructions.files',
		section: 'paths',
		group: 'Instructions & skills',
		label: 'Instruction filenames',
		kind: 'list',
		defaultValue: '["AGENTS.md"]',
		placeholder: 'AGENTS.md',
		fallbackDescription:
			'Instruction filenames searched from workspace root down to files.'
	},
	{
		key: 'instructions.other_agents',
		section: 'paths',
		group: 'Instructions & skills',
		label: 'Fallback to other agent files',
		kind: 'bool',
		defaultValue: 'true',
		fallbackDescription:
			'Read CLAUDE.md, GEMINI.md, .cursorrules, etc. when primary instruction files are absent.'
	},
	{
		key: 'instructions.max_bytes',
		section: 'paths',
		group: 'Instructions & skills',
		label: 'Instruction size limit (bytes)',
		kind: 'int',
		defaultValue: '32768',
		min: 0,
		step: 1024,
		fallbackDescription: 'Maximum total bytes of project instruction files loaded.'
	},
	{
		key: 'skills.compat_paths',
		section: 'paths',
		group: 'Instructions & skills',
		label: 'Extra skill directories',
		kind: 'path-list',
		defaultValue: '[]',
		placeholder: '~/.claude/skills',
		fallbackDescription:
			'Extra folders searched for third-party skills after user and project folders.'
	},
	{
		key: 'skills.disabled',
		section: 'paths',
		group: 'Instructions & skills',
		label: 'Disabled skills',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'skill-name',
		fallbackDescription: 'Skill names omitted from discovery.'
	},
	{
		key: 'skills.index_max_tokens',
		section: 'paths',
		group: 'Instructions & skills',
		label: 'Skill index token budget',
		kind: 'int',
		defaultValue: '1500',
		min: 0,
		step: 100,
		fallbackDescription: 'Cap on the skills list loaded into the system context.'
	},
	{
		key: 'defaults.dir',
		section: 'paths',
		group: 'Bundle root',
		label: 'Defaults bundle override',
		kind: 'path',
		defaultValue: '""',
		placeholder: 'Use built-in / discovered defaults',
		emptyMeaning: 'Auto-discovers defaults/',
		hint: 'Matches QUACK_DEFAULTS_DIR. Use "none" to skip loading defaults.',
		fallbackDescription: 'Optional override directory for the shipped defaults bundle.'
	},

	// ── Setup & Models ──────────────────────────────────────────────────
	{
		key: 'model.default',
		section: 'setup',
		group: 'Default models',
		label: 'Default model',
		kind: 'model',
		defaultValue: '""',
		placeholder: 'provider/model',
		emptyMeaning: 'Uses last used model or opens Setup',
		hint: 'Chosen during `quack setup`. Leaves the session model unchanged until reload unless switched.',
		fallbackDescription: 'Default `<provider>/<model>` used when starting a new session.'
	},
	{
		key: 'model.small',
		section: 'setup',
		group: 'Default models',
		label: 'Small helper model',
		kind: 'model',
		defaultValue: '""',
		placeholder: 'Optional provider/model',
		emptyMeaning: 'Uses the main session model',
		hint: 'Can also be set as Small or Both directly in the model selector.',
		fallbackDescription: 'Faster/cheaper model used for lightweight tasks like session titles.'
	},
	{
		key: 'model.planner',
		section: 'setup',
		group: 'Default models',
		label: 'Planner model',
		kind: 'model',
		defaultValue: '""',
		placeholder: 'Optional provider/model',
		emptyMeaning: 'Plans with the main model',
		fallbackDescription: 'Dedicated model used when entering Plan mode.'
	},
	{
		key: 'model.remember_last',
		section: 'setup',
		group: 'Default models',
		label: 'Remember last used model',
		kind: 'bool',
		defaultValue: 'true',
		fallbackDescription:
			'Reopen the most recently chosen model across restarts when model.default is unset.'
	},
	{
		key: 'model.profile',
		section: 'setup',
		group: 'Default models',
		label: 'Prompt profile override',
		kind: 'profile',
		defaultValue: '""',
		placeholder: 'auto (pick by model family)',
		emptyMeaning: 'Automatically matches profile from model id',
		fallbackDescription: 'Prompt & tool-formatting profile; leave empty for automatic matching.'
	},
	{
		key: 'catalog.refresh',
		section: 'setup',
		group: 'Background checks (quack setup)',
		label: 'Refresh model catalog daily',
		kind: 'bool',
		defaultValue: 'false',
		fallbackDescription:
			'Opt-in daily refresh of model context windows and pricing from catalog.url.'
	},
	{
		key: 'catalog.url',
		section: 'setup',
		group: 'Background checks (quack setup)',
		label: 'Model catalog URL',
		kind: 'text',
		defaultValue: '""',
		placeholder: 'https://...',
		emptyMeaning: 'Empty (disabled, e.g. --plucked install)',
		fallbackDescription: 'Endpoint fetched when catalog.refresh is enabled.'
	},
	{
		key: 'updates.check',
		section: 'setup',
		group: 'Background checks (quack setup)',
		label: 'Check for new quack releases',
		kind: 'bool',
		defaultValue: 'false',
		fallbackDescription: 'Opt-in daily check against updates.url for newer versions.'
	},
	{
		key: 'updates.url',
		section: 'setup',
		group: 'Background checks (quack setup)',
		label: 'Release check URL',
		kind: 'text',
		defaultValue: '""',
		placeholder: 'https://...',
		emptyMeaning: 'Empty (disabled)',
		fallbackDescription: 'Endpoint queried when updates.check is enabled.'
	},

	// ── Agent, Context & Tools ──────────────────────────────────────────
	{
		key: 'agent.max_steps',
		section: 'context',
		group: 'Agent turn loop',
		label: 'Max model steps per turn',
		kind: 'int',
		defaultValue: '100',
		min: 1,
		step: 10,
		fallbackDescription: 'Pause and ask to continue after this many model steps in one turn.'
	},
	{
		key: 'context.window',
		section: 'context',
		group: 'Context window & caching',
		label: 'Context window cap (tokens)',
		kind: 'nullable-int',
		defaultValue: 'null',
		min: 256,
		step: 1024,
		supportsNullReset: true,
		emptyMeaning: 'Unset (uses the model’s full window)',
		fallbackDescription:
			'Optional token cap overriding or supplying the model context window (sent as num_ctx for Ollama).'
	},
	{
		key: 'context.budget_ratio',
		section: 'context',
		group: 'Context window & caching',
		label: 'Context budget ratio',
		kind: 'float',
		defaultValue: '0.85',
		min: 0.05,
		max: 1.0,
		step: 0.05,
		fallbackDescription: 'Fraction of the context window reserved for prompt history and input.'
	},
	{
		key: 'context.compaction_threshold',
		section: 'context',
		group: 'Context window & caching',
		label: 'Compaction threshold',
		kind: 'float',
		defaultValue: '0.80',
		min: 0.05,
		max: 1.0,
		step: 0.05,
		fallbackDescription: 'Fraction of the budget at which pruning and compaction trigger.'
	},
	{
		key: 'context.cache_strategy',
		section: 'context',
		group: 'Context window & caching',
		label: 'KV-cache strategy',
		kind: 'select',
		defaultValue: '"preserve"',
		options: [
			{
				value: 'preserve',
				label: 'preserve',
				detail: 'Prune/compact at thresholds to keep prompt cache prefixes intact'
			},
			{
				value: 'aggressive',
				label: 'aggressive',
				detail: 'Compact earlier to keep token usage lower'
			},
			{
				value: 'auto',
				label: 'auto',
				detail: 'Let the model setup choose'
			}
		],
		fallbackDescription: 'How context pruning and compaction trade off token cost vs cache hits.'
	},
	{
		key: 'tools.edit_format',
		section: 'context',
		group: 'Tools & editing',
		label: 'Edit tool format',
		kind: 'select',
		defaultValue: '"auto"',
		options: [
			{ value: 'auto', label: 'auto', detail: 'Use the profile default (or str_replace)' },
			{ value: 'str_replace', label: 'str_replace', detail: 'Exact snippet replacement (`edit`)' },
			{ value: 'hashline', label: 'hashline', detail: 'Line-hash anchored edits (`edit_hashline`)' },
			{ value: 'apply_patch', label: 'apply_patch', detail: 'Multi-file patch tool' },
			{ value: 'whole_file', label: 'whole_file', detail: 'Rewrite entire files with `write`' },
			{
				value: 'search_replace_blocks',
				label: 'search_replace_blocks',
				detail: 'SEARCH/REPLACE fenced blocks (`edit_blocks`)'
			}
		],
		fallbackDescription: 'Which file-editing tool format models receive.'
	},
	{
		key: 'tools.shell',
		section: 'context',
		group: 'Tools & editing',
		label: 'Shell binary override',
		kind: 'text',
		defaultValue: '""',
		placeholder: 'System default shell',
		emptyMeaning: 'Uses system default shell ($SHELL / cmd)',
		fallbackDescription: 'Custom executable used by the shell tool.'
	},
	{
		key: 'tools.default_timeout_secs',
		section: 'context',
		group: 'Tools & editing',
		label: 'Default command timeout (s)',
		kind: 'int',
		defaultValue: '120',
		min: 1,
		step: 10,
		fallbackDescription: 'Default shell command timeout in seconds.'
	},
	{
		key: 'tools.read_outline_tokens',
		section: 'context',
		group: 'Tools & editing',
		label: 'Read outline threshold (tokens)',
		kind: 'int',
		defaultValue: '2000',
		min: 0,
		step: 250,
		fallbackDescription: 'Files above this token count default to outlined `read` output.'
	},

	// ── Network & Telemetry ─────────────────────────────────────────────
	{
		key: 'network.offline',
		section: 'network',
		group: 'Outbound network policy',
		label: 'Offline mode (loopback only)',
		kind: 'bool',
		defaultValue: 'false',
		fallbackDescription: 'Block all outbound traffic except loopback hosts and local sockets.'
	},
	{
		key: 'network.egress_allow',
		section: 'network',
		group: 'Outbound network policy',
		label: 'Allowed egress hosts',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'api.example.com or *.internal',
		emptyMeaning: 'Configured providers and MCP servers determine destinations',
		fallbackDescription:
			'Additional hostnames or wildcard patterns permitted alongside configured providers.'
	},
	{
		key: 'network.proxy',
		section: 'network',
		group: 'Outbound network policy',
		label: 'HTTP/SOCKS proxy URL',
		kind: 'text',
		defaultValue: '""',
		placeholder: 'http://127.0.0.1:8080',
		emptyMeaning: 'Uses HTTP_PROXY, HTTPS_PROXY, ALL_PROXY, or system settings',
		fallbackDescription: 'Proxy URL used for outbound model and catalog requests.'
	},
	{
		key: 'network.ca_bundle',
		section: 'network',
		group: 'Outbound network policy',
		label: 'Custom CA bundle path',
		kind: 'text',
		defaultValue: '""',
		placeholder: '/etc/ssl/custom-ca.pem',
		emptyMeaning: 'Uses OS trust store',
		fallbackDescription: 'Additional PEM root CA certificate bundle trusted for TLS.'
	},
	{
		key: 'network.http1_only',
		section: 'network',
		group: 'Outbound network policy',
		label: 'Force HTTP/1.1',
		kind: 'bool',
		defaultValue: 'false',
		fallbackDescription: 'Default to HTTP/1.1. Individual providers can override this setting.'
	},
	{
		key: 'network.secret_guard',
		section: 'network',
		group: 'Pre-egress secret shield',
		label: 'Secret Shield mode',
		kind: 'select',
		defaultValue: '"ask"',
		options: [
			{ value: 'ask', label: 'ask', detail: 'Pause remote requests to redact or allow secrets' },
			{ value: 'redact', label: 'redact', detail: 'Automatically redact detected secrets before send' },
			{ value: 'off', label: 'off', detail: 'Disable pre-egress secret scanning' }
		],
		fallbackDescription: 'How pre-egress Secret Shield handles API keys and credentials before network calls.'
	},
	{
		key: 'network.secret_guard_local',
		section: 'network',
		group: 'Pre-egress secret shield',
		label: 'Scan loopback requests too',
		kind: 'bool',
		defaultValue: 'false',
		fallbackDescription: 'Also run Secret Shield on localhost / loopback providers instead of skipping local inference.'
	},
	{
		key: 'network.secret_allowlist',
		section: 'network',
		group: 'Pre-egress secret shield',
		label: 'Allowed secret fingerprints',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'sha256:a1b2c3d4e5f6',
		emptyMeaning: 'No allowlisted fingerprints',
		fallbackDescription: 'Short SHA-256 secret fingerprints (`sha256:...`) permitted outbound without prompting.'
	},
	{
		key: 'telemetry.otel_endpoint',
		section: 'network',
		group: 'Telemetry',
		label: 'OpenTelemetry OTLP endpoint',
		kind: 'text',
		defaultValue: '""',
		placeholder: 'http://127.0.0.1:4317',
		emptyMeaning: 'Telemetry disabled',
		fallbackDescription: 'OTLP/gRPC collector endpoint. Empty keeps telemetry completely off.'
	},

	// ── Permissions, Sandbox & Bundles ──────────────────────────────────
	{
		key: 'permissions.mode',
		section: 'permissions',
		group: 'Default approvals & work mode',
		label: 'Default approval policy',
		kind: 'select',
		defaultValue: '"ask"',
		options: [
			{ value: 'read-only', label: 'read-only', detail: 'Deny edits and shell writes' },
			{ value: 'ask', label: 'ask', detail: 'Prompt before edits and commands' },
			{ value: 'auto-edit', label: 'auto-edit', detail: 'Allow file edits, ask for shell' },
			{ value: 'allow', label: 'allow (full-auto)', detail: 'Run inside sandbox without prompts' },
			{ value: 'yolo', label: 'yolo', detail: 'Skip approval prompts' }
		],
		fallbackDescription: 'Startup approval policy when starting a new session.'
	},
	{
		key: 'permissions.work_mode',
		section: 'permissions',
		group: 'Default approvals & work mode',
		label: 'Default work mode',
		kind: 'select',
		defaultValue: '"edit"',
		options: [
			{ value: 'edit', label: 'edit', detail: 'Start in regular editing mode' },
			{ value: 'plan', label: 'plan', detail: 'Start in read-only planning mode' }
		],
		fallbackDescription: 'Initial work mode (`edit` or `plan`).'
	},
	{
		key: 'permissions.allow',
		section: 'permissions',
		group: 'Permission rule lists',
		label: 'Always allow rules',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'shell:cargo test*',
		fallbackDescription: 'Tool patterns permitted without prompting.'
	},
	{
		key: 'permissions.ask',
		section: 'permissions',
		group: 'Permission rule lists',
		label: 'Ask rules',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'shell:git push*',
		fallbackDescription: 'Tool patterns that always prompt (wins over allow rules).'
	},
	{
		key: 'permissions.deny',
		section: 'permissions',
		group: 'Permission rule lists',
		label: 'Deny rules',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'shell:rm -rf *',
		fallbackDescription: 'Tool patterns blocked unconditionally (wins over allow and ask).'
	},
	{
		key: 'sandbox.mode',
		section: 'permissions',
		group: 'OS sandbox',
		label: 'Sandbox mode',
		kind: 'select',
		defaultValue: '"auto"',
		options: [
			{ value: 'auto', label: 'auto', detail: 'Workspace-write on Landlock/Seatbelt OS' },
			{ value: 'off', label: 'off', detail: 'Run commands unsandboxed' },
			{
				value: 'workspace-write',
				label: 'workspace-write',
				detail: 'Write only workspace & temp, no network'
			},
			{
				value: 'workspace-write-net',
				label: 'workspace-write-net',
				detail: 'Write workspace & temp with network access'
			},
			{ value: 'read-only', label: 'read-only', detail: 'No writes and no network' }
		],
		fallbackDescription: 'OS filesystem and network isolation mode for shell commands.'
	},
	{
		key: 'sandbox.writable',
		section: 'permissions',
		group: 'OS sandbox',
		label: 'Extra writable folders',
		kind: 'path-list',
		defaultValue: '[]',
		placeholder: '~/.cargo',
		fallbackDescription: 'Additional directories shell commands may write outside the workspace.'
	},
	{
		key: 'bundles.disabled',
		section: 'permissions',
		group: 'Disabled extensions & bundles',
		label: 'Disabled bundles',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'bundle-id',
		fallbackDescription: 'Extension bundle IDs excluded from loading.'
	},
	{
		key: 'providers.disabled',
		section: 'permissions',
		group: 'Disabled extensions & bundles',
		label: 'Disabled providers',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'provider-id',
		fallbackDescription: 'Provider IDs disabled regardless of layer files.'
	},
	{
		key: 'mcp.disabled',
		section: 'extensions',
		group: 'Extension disable lists',
		label: 'Disabled MCP servers',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'server-id',
		fallbackDescription: 'MCP server IDs kept disabled.'
	},
	{
		key: 'lsp.disabled',
		section: 'permissions',
		group: 'Disabled extensions & bundles',
		label: 'Disabled language servers',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'lsp-id',
		fallbackDescription: 'LSP server IDs excluded from workspace startup.'
	},
	{
		key: 'hooks.disabled',
		section: 'extensions',
		group: 'Extension disable lists',
		label: 'Disabled hooks',
		kind: 'list',
		defaultValue: '[]',
		placeholder: 'hook-id',
		fallbackDescription: 'User/project hook IDs kept disabled.'
	},

	// ── Interface & All TOML ────────────────────────────────────────────
	{
		key: 'ui.theme',
		section: 'interface',
		group: 'Desktop & terminal appearance',
		label: 'Config theme preference',
		kind: 'theme',
		defaultValue: '"auto"',
		options: [
			{ value: 'auto', label: 'auto (follow OS)' },
			{ value: 'light', label: 'light' },
			{ value: 'dark', label: 'dark' }
		],
		hint: 'Desktop webview also tracks its local header toggle in browser storage when set explicitly.',
		fallbackDescription: 'Default theme (`auto`, `dark`, `light`, or TUI theme name).'
	},
	{
		key: 'ui.reduced_motion',
		section: 'interface',
		group: 'Desktop & terminal appearance',
		label: 'Reduced motion',
		kind: 'bool',
		defaultValue: 'false',
		fallbackDescription: 'Replace animated spinners and transitions with static indicators.'
	},
	{
		key: 'ui.gui_restore_tabs',
		section: 'interface',
		group: 'Desktop & terminal appearance',
		label: 'Restore open tabs on launch',
		kind: 'bool',
		defaultValue: 'true',
		fallbackDescription: 'Reopen previously open workspace tabs when starting the desktop GUI.'
	},
	{
		key: 'ui.show_reasoning',
		section: 'interface',
		group: 'Terminal UI defaults',
		label: 'Show model reasoning',
		kind: 'select',
		defaultValue: '"collapsed"',
		tuiOnly: true,
		options: [
			{ value: 'hidden', label: 'hidden' },
			{ value: 'collapsed', label: 'collapsed' },
			{ value: 'expanded', label: 'expanded' }
		],
		fallbackDescription: 'How reasoning blocks display by default (`hidden`, `collapsed`, `expanded`).'
	},
	{
		key: 'ui.startup_panel',
		section: 'interface',
		group: 'Terminal UI defaults',
		label: 'Startup diagnostics panel',
		kind: 'select',
		defaultValue: '"auto"',
		tuiOnly: true,
		options: [
			{ value: 'auto', label: 'auto' },
			{ value: 'always', label: 'always' },
			{ value: 'never', label: 'never' }
		],
		fallbackDescription: 'When to display startup subsystem diagnostics in the TUI.'
	},
	{
		key: 'ui.sidebar',
		section: 'interface',
		group: 'Terminal UI defaults',
		label: 'Show TUI sidebar',
		kind: 'bool',
		defaultValue: 'true',
		tuiOnly: true,
		fallbackDescription: 'Whether the terminal client opens with the sidebar shown.'
	},
	{
		key: 'ui.ascii',
		section: 'interface',
		group: 'Terminal UI defaults',
		label: 'ASCII-only TUI chrome',
		kind: 'bool',
		defaultValue: 'false',
		tuiOnly: true,
		fallbackDescription: 'Use ASCII borders and spinners instead of Unicode glyphs in the TUI.'
	},
	{
		key: 'ui.mouse',
		section: 'interface',
		group: 'Terminal UI defaults',
		label: 'TUI mouse capture',
		kind: 'bool',
		defaultValue: 'true',
		tuiOnly: true,
		fallbackDescription: 'Enable mouse scroll & click capture in the terminal client.'
	},
	{
		key: 'ui.terminal_title',
		section: 'interface',
		group: 'Terminal UI defaults',
		label: 'Update terminal window title',
		kind: 'bool',
		defaultValue: 'true',
		tuiOnly: true,
		fallbackDescription: 'Set the terminal window title to the active session title.'
	}
];

export const SETTING_FIELD_MAP: ReadonlyMap<string, SettingFieldDef> = new Map(
	SETTING_FIELDS.map((field) => [field.key, field])
);

/** Unescape a TOML string literal (`"..."` or `'...'`), falling back cleanly to trimmed text. */
export function parseTomlString(raw: string): string {
	const trimmed = raw.trim();
	if (!trimmed || trimmed === 'null') return '';
	if (trimmed.startsWith('"') && trimmed.endsWith('"') && trimmed.length >= 2) {
		try {
			return JSON.parse(trimmed) as string;
		} catch {
			return trimmed.slice(1, -1).replace(/\\(.)/g, '$1');
		}
	}
	if (trimmed.startsWith("'") && trimmed.endsWith("'") && trimmed.length >= 2) {
		return trimmed.slice(1, -1);
	}
	return trimmed;
}

/** Encode a JS string as a valid TOML basic string (`"..."`). */
export function formatTomlString(value: string): string {
	return JSON.stringify(value);
}

/** Parse a TOML array of strings (`["a", "b"]` or `['a']`) cleanly without eval. */
export function parseTomlStringArray(raw: string): string[] {
	const trimmed = raw.trim();
	if (!trimmed.startsWith('[') || !trimmed.endsWith(']')) return [];
	const inner = trimmed.slice(1, -1).trim();
	if (!inner) return [];

	const items: string[] = [];
	let i = 0;
	while (i < inner.length) {
		while (i < inner.length && /[\s,]/.test(inner[i])) i++;
		if (i >= inner.length) break;
		const quote = inner[i];
		if (quote === '"' || quote === "'") {
			let j = i + 1;
			let escaped = false;
			while (j < inner.length) {
				const ch = inner[j];
				if (escaped) {
					escaped = false;
				} else if (quote === '"' && ch === '\\') {
					escaped = true;
				} else if (ch === quote) {
					break;
				}
				j++;
			}
			const token = inner.slice(i, Math.min(j + 1, inner.length));
			items.push(parseTomlString(token));
			i = j + 1;
		} else {
			let j = i;
			while (j < inner.length && inner[j] !== ',') j++;
			const word = inner.slice(i, j).trim();
			if (word) items.push(parseTomlString(word));
			i = j + 1;
		}
	}
	return items;
}

/** Serialize string items as a TOML inline array (`["a", "b"]`). */
export function formatTomlStringArray(items: readonly string[]): string {
	return `[${items.map((item) => JSON.stringify(item)).join(', ')}]`;
}

/** Parse an integer or float from a TOML value string (`null` -> `null`). */
export function parseTomlNumber(raw: string): number | null {
	const trimmed = raw.trim();
	if (!trimmed || trimmed === 'null') return null;
	const cleaned = trimmed.replaceAll('_', '');
	const num = Number(cleaned);
	return Number.isFinite(num) ? num : null;
}

export function parseOrigin(origin: string): ParsedOrigin {
	const trimmed = origin.trim();
	if (!trimmed || trimmed === 'default') {
		return { layer: 'default', label: 'default', detail: '' };
	}
	if (trimmed === 'cli') {
		return { layer: 'cli', label: 'cli', detail: 'command-line override' };
	}
	const colon = trimmed.indexOf(':');
	if (colon !== -1) {
		const head = trimmed.slice(0, colon).trim().toLowerCase();
		const detail = trimmed.slice(colon + 1).trim();
		if (
			head === 'user' ||
			head === 'project' ||
			head === 'system' ||
			head === 'env'
		) {
			return { layer: head, label: head, detail };
		}
		if (head === 'defaults') {
			return { layer: 'default', label: 'defaults', detail };
		}
	}
	return { layer: 'other', label: trimmed, detail: '' };
}

export function extractUrlHost(rawUrl: string): string | null {
	const trimmed = rawUrl.trim();
	if (!trimmed) return null;
	try {
		return new URL(trimmed).host || null;
	} catch {
		return null;
	}
}

/** Build effective map including unset nullable keys like `context.window`. */
export function buildSettingRowMap(settings: readonly SettingRow[]): Map<string, SettingRow> {
	const map = new Map<string, SettingRow>();
	for (const row of settings) {
		map.set(row.key, row);
	}
	for (const def of SETTING_FIELDS) {
		if (!map.has(def.key)) {
			map.set(def.key, {
				key: def.key,
				value: def.defaultValue,
				origin: 'default',
				description: def.fallbackDescription,
				is_bool: def.kind === 'bool',
				locked: false
			});
		}
	}
	return map;
}

/** Sorted list of all effective TOML rows, filling missing optional schema fields cleanly. */
export function buildAllSettingRows(settings: readonly SettingRow[]): SettingRow[] {
	const rows = [...buildSettingRowMap(settings).values()];
	rows.sort((a, b) => a.key.localeCompare(b.key));
	return rows;
}

/** Check whether a query matches a setting key, label, value, or description. */
export function matchesSettingSearch(
	query: string,
	row: SettingRow,
	field?: SettingFieldDef
): boolean {
	const needle = query.trim().toLowerCase();
	if (!needle) return true;
	const haystack = [
		row.key,
		row.value,
		row.origin,
		row.description,
		field?.label ?? '',
		field?.group ?? '',
		field?.fallbackDescription ?? ''
	]
		.join('\n')
		.toLowerCase();
	return needle.split(/\s+/).every((part) => haystack.includes(part));
}

/** Format resolved directory paths into display rows matching `quack config paths` + storage dirs. */
export function resolvedPathRows(
	cwd: string,
	paths?: PathsSnapshot
): ResolvedPathRow[] {
	if (!paths) {
		return [{ id: 'cwd', label: 'Workspace', path: cwd, hint: 'Active repository root' }];
	}
	return [
		{
			id: 'memory_global',
			label: 'Global memory notes',
			path: paths.memory_global,
			hint: 'Controlled by [memory] path (notes/<name>.md)'
		},
		{
			id: 'memory_project',
			label: 'Project memory notes',
			path: paths.memory_project,
			hint: 'Controlled by [memory] path + project_scope'
		},
		{
			id: 'sessions_root',
			label: 'Sessions directory',
			path: paths.sessions_root,
			hint: 'Controlled by [sessions] path'
		},
		{
			id: 'sessions_project',
			label: 'Workspace session logs',
			path: paths.sessions_project,
			hint: 'Session logs & prompt history for this repo'
		},
		{
			id: 'user_config',
			label: 'User config',
			path: paths.user_config,
			hint: 'config.toml, providers.d/, skills/, hooks.d/'
		},
		{
			id: 'user_data',
			label: 'User data',
			path: paths.user_data,
			hint: 'Default store for memory, sessions, auth & trust.toml'
		},
		{
			id: 'user_cache',
			label: 'Cache',
			path: paths.user_cache,
			hint: 'Downloaded catalog cache & eval scratch'
		},
		{
			id: 'project',
			label: 'Project config (.quack)',
			path: paths.project,
			hint: 'Repository overrides & trust-gated extensions'
		},
		{
			id: 'system',
			label: 'System config',
			path: paths.system,
			hint: 'Administrator policy & locked settings'
		},
		{
			id: 'defaults',
			label: 'Defaults bundle',
			path: paths.defaults ?? '(none)',
			hint: 'Shipped TOML defaults layer (override with defaults.dir)'
		}
	];
}

/** Validate a candidate edit and return either formatted TOML value or an inline error. */
export function prepareFieldTomlValue(
	field: SettingFieldDef,
	rawInput: string
): { ok: true; value: string } | { ok: false; error: string } {
	const trimmed = rawInput.trim();
	switch (field.kind) {
		case 'bool':
			return { ok: true, value: trimmed === 'true' ? 'true' : 'false' };
		case 'int': {
			if (!/^-?\d+$/.test(trimmed)) {
				return { ok: false, error: 'Enter a whole number.' };
			}
			const parsed = Number(trimmed);
			const min = field.min ?? 0;
			if (!Number.isSafeInteger(parsed) || parsed < min) {
				return { ok: false, error: `Enter a whole number >= ${min}.` };
			}
			return { ok: true, value: String(parsed) };
		}
		case 'nullable-int': {
			if (!trimmed || trimmed === 'null') {
				return { ok: true, value: 'null' };
			}
			if (!/^-?\d+$/.test(trimmed)) {
				return { ok: false, error: 'Enter a whole number, or leave blank to unset.' };
			}
			const parsed = Number(trimmed);
			const min = field.min ?? 1;
			if (!Number.isSafeInteger(parsed) || parsed < min) {
				return { ok: false, error: `Enter a whole number >= ${min}, or leave blank to unset.` };
			}
			return { ok: true, value: String(parsed) };
		}
		case 'float': {
			const parsed = Number(trimmed);
			if (!trimmed || !Number.isFinite(parsed)) {
				return { ok: false, error: 'Enter a decimal number.' };
			}
			if (field.min !== undefined && parsed < field.min) {
				return { ok: false, error: `Must be at least ${field.min}.` };
			}
			if (field.max !== undefined && parsed > field.max) {
				return { ok: false, error: `Must be at most ${field.max}.` };
			}
			return { ok: true, value: String(parsed) };
		}
		case 'model': {
			const clean = trimmed.replace(/^"(.*)"$/, '$1').trim();
			if (!clean) {
				return { ok: true, value: formatTomlString('') };
			}
			const slash = clean.indexOf('/');
			if (slash < 1 || slash === clean.length - 1 || /\s/.test(clean)) {
				return { ok: false, error: 'Use <provider>/<model> (for example ollama/qwen3.5:35b).' };
			}
			return { ok: true, value: formatTomlString(clean) };
		}
		case 'path':
		case 'text':
		case 'profile':
		case 'select':
		case 'theme':
			return { ok: true, value: formatTomlString(rawInput.trim()) };
		case 'list':
		case 'path-list':
			return {
				ok: true,
				value: trimmed.startsWith('[')
					? formatTomlStringArray(parseTomlStringArray(trimmed))
					: formatTomlStringArray([])
			};
	}
}
