import type { Event, HookSetupInfo, McpServerInfo, MemoryNote } from '#lib/protocol';
import {
	SETTING_FIELDS,
	extractUrlHost,
	formatTomlStringArray,
	parseTomlString,
	parseTomlStringArray
} from '../settings';
import type {
	AppInfo,
	ClientEvent,
	ClientOp,
	HighlightSpan,
	HookRow,
	McpRow,
	OpenedTab,
	RecentFolder,
	ResumeRequest,
	TabMessage,
	Transport
} from './types';

/**
 * The dev transport: `npm run dev` in a plain browser has no engines behind it, so
 * this scripts a small, honest demo session through the same `Transport` seam the
 * desktop shell uses — streamed deltas, a write call with an approval, plan events.
 * It never claims to be a real engine; the tab header says so.
 */

const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

const DEV_INITIAL_OVERRIDES: Record<string, string> = {
	'model.default': '"dev-local/tiny"',
	'model.planner': '"dev-local/thinker"',
	'network.offline': 'true',
	'catalog.url': '"https://models.dev/api.json"',
	'updates.url': '"https://api.github.com/repos/charmbracelet/quack/releases/latest"'
};

const DEV_HOOKS: HookRow[] = [
	{
		id: 'jira',
		layer: 'defaults',
		file: '/usr/share/quack/defaults/hooks.d/jira.toml',
		state: 'disabled',
		detail: 'disabled in file',
		enabled: false,
		can_toggle: true,
		has_setup: true,
		description: 'Fetch or verify referenced Jira issues before editing',
		event: 'user_prompt',
		command: ['node', '/usr/share/quack/defaults/hooks.d/jira.mjs'],
		matcher_tool: '',
		matcher_pattern: '',
		matcher_prompt: '[A-Z]{2,}-\\d+',
		timeout_secs: 15,
		on_error: 'warn',
		full_context: false,
		connections_count: 1
	},
	{
		id: 'audit-edits',
		layer: 'user',
		file: '/home/you/.config/quack/hooks.d/audit-edits.toml',
		state: 'enabled',
		detail: 'on',
		enabled: true,
		can_toggle: true,
		has_setup: false,
		description: 'Record edited paths to workspace audit trail',
		event: 'post_tool',
		command: ['scripts/audit-edits.sh'],
		matcher_tool: 'edit|write',
		matcher_pattern: '',
		matcher_prompt: '',
		timeout_secs: 5,
		on_error: 'warn',
		full_context: false,
		connections_count: 0
	}
];

const DEV_MCP: McpRow[] = [
	{
		id: 'filesystem-demo',
		layer: 'user',
		file: '/home/you/.config/quack/mcp.d/filesystem-demo.toml',
		state: 'enabled',
		detail: 'enabled',
		enabled: true,
		can_toggle: true,
		blocked_by_policy: false,
		transport: 'stdio',
		target: 'npx -y @modelcontextprotocol/server-filesystem .',
		description: 'Read-only project file explorer (demo)',
		sandbox: true,
		startup_timeout_secs: 20,
		call_timeout_secs: 120,
		max_output_tokens: 8000,
		auto_approve: ['read_file', 'list_directory'],
		env_keys: [],
		header_keys: []
	},
	{
		id: 'github',
		layer: 'defaults',
		file: '/usr/share/quack/defaults/mcp.d/github.toml',
		state: 'disabled',
		detail: 'disabled in file',
		enabled: false,
		can_toggle: true,
		blocked_by_policy: false,
		transport: 'stdio',
		target: 'npx -y @modelcontextprotocol/server-github',
		description: 'GitHub repositories, issues and pull requests',
		sandbox: null,
		startup_timeout_secs: 20,
		call_timeout_secs: 120,
		max_output_tokens: 8000,
		auto_approve: [],
		env_keys: ['GITHUB_PERSONAL_ACCESS_TOKEN'],
		header_keys: []
	}
];

function createJiraSetupInfo(enabled = false): HookSetupInfo {
	return {
		id: 'jira',
		revision: 'rev-1',
		descriptor: {
			version: 1,
			scope_field: 'web_url',
			fields: [
				{
					id: 'web_url',
					label: 'Site URL',
					kind: 'url',
					required: true,
					default: 'https://company.atlassian.net',
					help: 'Base web URL of your Jira site',
					choices: [],
					advanced: false,
					credential_scope: true,
					required_when: {}
				},
				{
					id: 'email',
					label: 'Account email',
					kind: 'text',
					required: false,
					default: '',
					help: 'Required when authenticating with an Atlassian API token',
					choices: [],
					advanced: false,
					credential_scope: false,
					required_when: { auth_mode: ['api_token'] }
				},
				{
					id: 'auth_mode',
					label: 'Auth mode',
					kind: 'choice',
					required: true,
					default: 'api_token',
					help: 'Authentication scheme for Jira REST API requests',
					choices: ['api_token', 'bearer'],
					advanced: false,
					credential_scope: false,
					required_when: {}
				},
				{
					id: 'token',
					label: 'API token / PAT',
					kind: 'secret',
					required: true,
					default: '',
					help: 'Stored in your OS keyring when available',
					choices: [],
					advanced: false,
					credential_scope: false,
					required_when: {}
				},
				{
					id: 'api_url',
					label: 'REST API URL',
					kind: 'url',
					required: false,
					default: '',
					help: 'Derived automatically from Site URL unless overridden',
					choices: [],
					advanced: true,
					credential_scope: false,
					required_when: {}
				}
			],
			presets: [
				{
					id: 'cloud',
					label: 'Jira Cloud',
					help: 'Atlassian Cloud site (`*.atlassian.net`) with email + API token',
					values: { web_url: 'https://company.atlassian.net', auth_mode: 'api_token' },
					choices: { auth_mode: ['api_token'] },
					derived: { api_url: '{web_url}/rest/api/3' }
				},
				{
					id: 'datacenter',
					label: 'Jira Data Center',
					help: 'Self-hosted Jira Server / Data Center with Personal Access Token',
					values: { web_url: 'https://jira.internal.example', auth_mode: 'bearer' },
					choices: { auth_mode: ['bearer', 'api_token'] },
					derived: { api_url: '{web_url}/rest/api/2' }
				}
			]
		},
		connections: [
			{
				connection: {
					id: 'default',
					preset: 'cloud',
					enabled: true,
					values: {
						web_url: 'https://company.atlassian.net',
						email: 'you@example.com',
						auth_mode: 'api_token',
						api_url: 'https://company.atlassian.net/rest/api/3'
					}
				},
				credentials: {
					token: { from: 'env', var: 'JIRA_API_TOKEN' }
				}
			}
		],
		enabled,
		source: 'defaults',
		writable: true,
		problem: null
	};
}

const INFO: AppInfo = {
	version: '0.1.0-dev',
	cwd: '~/Development/demo-pond',
	branch: 'main',
	model: 'dev-local/tiny',
	planner_model: 'dev-local/thinker',
	planner_locked: false,
	permission_mode: 'ask',
	work_mode: 'edit',
	ui: { theme: 'auto', reduced_motion: false },
	egress: {
		offline: true,
		offline_locked: false,
		egress_allow: [],
		providers: [{ id: 'dev-local', host: 'localhost:11434', local: true, enabled: true }],
		mcp_remote: [],
		http1_only: false,
		has_proxy: false,
		updates_check: false,
		catalog_refresh: false,
		telemetry_enabled: false
	},
	sandbox: {
		mode: 'auto',
		os_supported: true,
		status_detail: 'active (Landlock + seccomp)',
		writable: [],
		locked: false
	},
	settings: SETTING_FIELDS.filter((f) => f.key !== 'context.window').map((f) => ({
		key: f.key,
		value: DEV_INITIAL_OVERRIDES[f.key] ?? f.defaultValue,
		origin: f.key in DEV_INITIAL_OVERRIDES ? 'user: ~/.config/quack/config.toml' : 'default',
		description: f.fallbackDescription,
		is_bool: f.kind === 'bool',
		locked: false
	})),
	layers: [
		{ layer: 'defaults', status: 'loaded', file: '/usr/share/quack/defaults/config.toml' },
		{ layer: 'system', status: 'no config.toml', file: '/etc/quack/config.toml' },
		{ layer: 'user', status: 'loaded', file: '/home/you/.config/quack/config.toml' },
		{ layer: 'project', status: 'no config.toml', file: null }
	],
	rules: [],
	bundles: [
		{
			id: 'rust-reviewer',
			layer: 'user',
			enabled: true,
			contents: '1 skill, 1 agent'
		}
	],
	bundles_disabled: [],
	hooks: DEV_HOOKS,
	hooks_disabled: [],
	mcp: DEV_MCP,
	mcp_disabled: [],
	project_status: 'none',
	paths: {
		defaults: '/usr/share/quack/defaults',
		system: '/etc/quack',
		user_config: '/home/you/.config/quack',
		user_data: '/home/you/.local/share/quack',
		user_cache: '/home/you/.cache/quack',
		project: '/home/you/Development/demo-pond/.quack',
		memory_global: '/home/you/.local/share/quack/memory',
		memory_project: '/home/you/.local/share/quack/memory/projects/demo-pond',
		sessions_root: '/home/you/.local/share/quack/sessions',
		sessions_project: '/home/you/.local/share/quack/sessions/demo-pond'
	},
	history: ['Check test failures in the pond module', 'Explain how session memory works']
};

const SESSION = { id: 'dev-session-0000', cwd: INFO.cwd, model: INFO.model, profile: 'generic' };

function savedSessions() {
	return [
		{
			id: `${SESSION.id}-older`,
			title: 'Untangle the migration test',
			model: 'dev-local/tiny',
			turns: 6,
			age: '3h ago',
			started: '2026-11-08',
			current: false
		}
	];
}

export function devTransport(): Transport {
	const listeners = new Set<(message: TabMessage) => void>();
	const fan = (message: TabMessage) => {
		for (const listener of listeners) listener(message);
	};
	const send = (tab: number, event: ClientEvent) => fan({ tab, event });
	let nextTab = 0;
	type TabState = {
		cwd: string;
		branch: string;
		workMode: import('#lib/protocol').WorkMode;
		approvals: import('#lib/protocol').ApprovalPolicy;
		effort: string | null;
		offline: boolean;
		sandboxOff: boolean;
		plannerModel: string | null;
		activeProfile: string;
		settings: AppInfo['settings'];
		resolveApproval: (() => void) | null;
		userTurns: string[];
		inputHistory: string[];
		memoryNotes: MemoryNote[];
		hooks: HookRow[];
		mcp: McpRow[];
		jiraSetup: HookSetupInfo;
	};
	const tabs = new Map<number, TabState>();

	function formatDemoNote(
		name: string,
		description: string,
		kind: string,
		body: string,
		updated = '2026-11-12'
	): string {
		return `---\nname: ${name}\ndescription: ${description}\ntype: ${kind}\nupdated: ${updated}\n---\n\n${body.trim()}\n`;
	}

	function seedMemoryNotes(cwd: string): MemoryNote[] {
		const base = cwd.replace(/^~\//, '/home/you/');
		return [
			{
				scope: 'project',
				name: 'rust-conventions',
				description: 'Workspace build flags and test conventions',
				path: `${base}/.quack/memory/notes/rust-conventions.md`,
				text: formatDemoNote(
					'rust-conventions',
					'Workspace build flags and test conventions',
					'project',
					'- Always run `cargo clippy --workspace --all-targets --locked` with `-D warnings`.\n- Never leave `.unwrap()` in library or bridge code; return errors or use `expect` with context.'
				)
			},
			{
				scope: 'global',
				name: 'editor-preferences',
				description: 'User preference for concise diffs and direct tests',
				path: '/home/you/.local/share/quack/memory/notes/editor-preferences.md',
				text: formatDemoNote(
					'editor-preferences',
					'User preference for concise diffs and direct tests',
					'user',
					'Keep helpers small and verify behavior with unit tests before claiming fixes.',
					'2026-11-10'
				)
			}
		];
	}

	function memoryIndexFill(notes: MemoryNote[]): number {
		let maxFill = 0;
		for (const scope of ['project', 'global'] as const) {
			const scoped = notes.filter((n) => n.scope === scope);
			if (scoped.length === 0) continue;
			const lines = scoped.length;
			const bytes = scoped.reduce((acc, n) => acc + `- [${n.name}](notes/${n.name}.md) — ${n.description}\n`.length, 0);
			const fill = Math.max(Math.round((lines / 200) * 100), Math.round((bytes / 25_000) * 100), 1);
			if (fill > maxFill) maxFill = fill;
		}
		return Math.min(100, maxFill);
	}
	const demoFolders = [
		'/home/you/Development/demo-pond',
		'/home/you/Development/quack',
		'/home/you/Development/orderly',
		'/home/you/Development/field-notes'
	];
	let nextPick = 0;

	// `?demo=turn` plays the demo turn on open; `&auto=1` also answers its approval,
	// so a headless capture can reach the settled state without a keyboard.
	// `?demo=jobs` seeds multiple parallel jobs across states (input, error, working, ready).
	const params =
		typeof window !== 'undefined' ? new URLSearchParams(window.location.search) : null;
	const demoMode = params?.get('demo');
	const autoDemo = demoMode === 'turn';
	const jobsDemo = demoMode === 'jobs';
	const autoApprove = params?.get('auto') === '1';

	function branchFor(cwd: string): string {
		if (cwd.endsWith('/quack')) return 'fix/eval-timeouts';
		if (cwd.endsWith('/orderly')) return 'feat/batch-export';
		if (cwd.endsWith('/field-notes')) return 'chore/deps';
		return 'main';
	}

	const catalog = () => ({
		providers: [
			{
				id: 'dev-local',
				available: true,
				note: 'scripted demo provider',
				bundled: false,
				models: [
					{ id: 'tiny', context_window: 32768, price: 'free (scripted)' },
					{
						id: 'thinker',
					context_window: 131072,
					price: 'free (scripted)',
					reasoning_efforts: ['low', 'medium', 'high']
					}
				]
			}
		],
		current: 'dev-local/tiny'
	});

	async function runDemoTurn(tab: number) {
		const push = (chunks: string[], kind: 'assistant' | 'reasoning') => {
			const ev = kind === 'assistant' ? 'assistant_delta' : 'reasoning_delta';
			return chunks.map((text) => send(tab, { event: ev, text } as Event));
		};
		await sleep(300);
		send(tab, { event: 'turn_started', turn: 1 });
		send(tab, { event: 'speed', ttft_ms: 240, tokens_per_sec: 42.7 });
		push(['Reading the pond first…'], 'reasoning');
		await sleep(200);
		push(
			['I’ll add the missing ', 'duck counter', ' to the pond module', ' and check it.'],
			'assistant'
		);
		await sleep(250);
		send(tab, {
			event: 'tool_call_begin',
			call: { call_id: 'dev-call-1', name: 'write', summary: 'write src/pond/ducks.ts' }
		});
		for (const chunk of ['const ducks = 5;\n', 'export default ducks;\n']) {
			send(tab, { event: 'tool_call_output_delta', call_id: 'dev-call-1', chunk });
			await sleep(120);
		}
		send(tab, {
			event: 'approval_request',
			call_id: 'dev-call-1',
			request: {
				kind: 'edit',
				path: 'src/pond/ducks.ts',
				diff:
					'--- src/pond/ducks.ts\n+++ src/pond/ducks.ts\n@@ -0,0 +1,2 @@\n+const ducks = 5;\n+export default ducks;\n'
			},
			always: ['write:src/pond/*']
		});

		// The demo pauses here until the UI answers the approval (an Op, via sendOp).
		await new Promise<void>((resolve) => {
			const state = tabs.get(tab);
			if (state) state.resolveApproval = resolve;
			if (autoApprove)
				setTimeout(() => {
					const current = tabs.get(tab);
					current?.resolveApproval?.();
					if (current) current.resolveApproval = null;
				}, 2200);
		});
		await sleep(200);
		send(tab, {
			event: 'agent_started',
			id: 'scout-1',
			agent: 'explore',
			description: 'check the pond edge for other writers',
			call_id: 'dev-call-1',
			model: 'dev-local/tiny'
		});
		send(tab, {
			event: 'agent',
			id: 'scout-1',
			inner: { event: 'assistant_delta', text: 'No other module writes ducks.ts; the census is safe to add. ' }
		});
		send(tab, {
			event: 'agent_finished',
			id: 'scout-1',
			ok: true,
			summary: '3 steps · 1.2k tokens'
		});
		send(tab, {
			event: 'tool_call_end',
			call_id: 'dev-call-1',
			result: {
				ok: true,
				summary: 'wrote 2 lines',
				detail: 'const ducks = 5;\nexport default ducks;\n'
			}
		});
		send(tab, {
			event: 'plan',
			checklist: '[x] read the pond module\n[x] add the duck counter\n[~] check the census test'
		});
		await sleep(200);
		push(['Done — ', '`src/pond/ducks.ts`', ' now exports the pond census.'], 'assistant');
		send(tab, {
			event: 'turn_complete',
			turn: 1,
			usage: { prompt_tokens: 1840, completion_tokens: 96, cached_prompt_tokens: 1024 }
		});
		send(tab, {
			event: 'context_update',
			used: 4132,
			budget: 26214,
			window: 32768,
			cache_hit_tokens: 1024,
			threshold: 24576
		});
	}

	function seedJobsDemo(tab: number, cwd: string) {
		if (cwd.endsWith('/quack')) {
			send(tab, { event: 'session_title', title: 'Clean stale eval cache' });
			send(tab, { event: 'turn_started', turn: 2 });
			send(tab, {
				event: 'assistant_delta',
				text: 'I found 4.2 GB of stale eval caches in target/tmp-cache. Removing them next.'
			});
			send(tab, {
				event: 'tool_call_begin',
				call: {
					call_id: `dev-job-${tab}-1`,
					name: 'shell',
					summary: 'rm -rf target/tmp-cache'
				}
			});
			send(tab, {
				event: 'approval_request',
				call_id: `dev-job-${tab}-1`,
				request: {
					kind: 'shell',
					command: 'rm -rf /var/tmp/quack-eval-cache && cargo check -p quack-exec',
					cwd,
					outside_sandbox: true
				},
				always: []
			});
			send(tab, {
				event: 'context_update',
				used: 11200,
				budget: 26214,
				window: 32768,
				cache_hit_tokens: 6100,
				threshold: 24576
			});
			return;
		}
		if (cwd.endsWith('/orderly')) {
			send(tab, { event: 'session_title', title: 'Refactor CSV streaming' });
			send(tab, { event: 'turn_started', turn: 3 });
			send(tab, {
				event: 'tool_call_begin',
				call: { call_id: `dev-job-${tab}-1`, name: 'edit', summary: 'edit src/export.rs' }
			});
			send(tab, {
				event: 'tool_call_end',
				call_id: `dev-job-${tab}-1`,
				result: {
					ok: true,
					summary: 'edited src/export.rs',
					detail: '--- src/export.rs\n+++ src/export.rs\n@@ -12,1 +12,4 @@\n-    write_all()\n+    flush_rows()\n+    stream_chunks()\n'
				}
			});
			send(tab, {
				event: 'error',
				error: { kind: 'provider', message: 'Upstream provider timed out after 30s (HTTP 504).' }
			});
			send(tab, {
				event: 'turn_aborted',
				turn: 3,
				reason: 'Upstream provider timed out after 30s (HTTP 504).'
			});
			send(tab, {
				event: 'context_update',
				used: 18900,
				budget: 26214,
				window: 32768,
				cache_hit_tokens: 9400,
				threshold: 24576
			});
			return;
		}
		if (cwd.endsWith('/field-notes')) {
			send(tab, { event: 'session_title', title: 'Update changelog & deps' });
			send(tab, { event: 'turn_started', turn: 1 });
			send(tab, {
				event: 'assistant_delta',
				text: 'Updated Cargo.lock and documented the 0.14 migration steps in CHANGELOG.md.'
			});
			send(tab, {
				event: 'tool_call_begin',
				call: { call_id: `dev-job-${tab}-1`, name: 'edit', summary: 'edit CHANGELOG.md' }
			});
			send(tab, {
				event: 'tool_call_end',
				call_id: `dev-job-${tab}-1`,
				result: {
					ok: true,
					summary: 'edited CHANGELOG.md',
					detail: '--- CHANGELOG.md\n+++ CHANGELOG.md\n@@ -1,0 +1,3 @@\n+## 0.14\n+- Added multi-job rail\n'
				}
			});
			send(tab, {
				event: 'turn_complete',
				turn: 1,
				usage: { prompt_tokens: 3400, completion_tokens: 180, cached_prompt_tokens: 2100 }
			});
			send(tab, {
				event: 'stats',
				report: {
					model: 'dev-local/tiny',
					turns: 4,
					requests: 7,
					unreported: 0,
					prompt_tokens: 21000,
					completion_tokens: 1200,
					cached_tokens: 14000,
					cache_reported: 7,
					cost: 0.08,
					unpriced: 0,
					ttft_ms: [210, 480],
					tokens_per_sec: 48.2,
					open_ms: 92000,
					model_ms: 21000,
					tools_ms: 6000,
					approval_ms: 0,
					tools: [{ name: 'edit', calls: 3, failed: 0, ms: 90 }],
					compactions: 0,
					pruned: 0,
					resumed: false,
					history: []
				}
			});
			send(tab, {
				event: 'context_update',
				used: 7200,
				budget: 26214,
				window: 32768,
				cache_hit_tokens: 4500,
				threshold: 24576
			});
			return;
		}
		// demo-pond: actively running job with sub-agent and plan progress
		send(tab, { event: 'session_title', title: 'Add pond duck counter' });
		send(tab, { event: 'turn_started', turn: 1 });
		send(tab, {
			event: 'assistant_delta',
			text: 'Running the census suite while explore checks other pond writers…'
		});
		send(tab, {
			event: 'plan',
			checklist: '[x] inspect pond module\n[~] update duck counter\n[ ] verify census tests'
		});
		send(tab, {
			event: 'tool_call_begin',
			call: { call_id: `dev-job-${tab}-1`, name: 'shell', summary: 'cargo test -p quack-core' }
		});
		send(tab, {
			event: 'agent_started',
			id: `scout-${tab}`,
			agent: 'explore',
			description: 'check pond edge for writers',
			call_id: `dev-job-${tab}-1`,
			model: 'dev-local/tiny'
		});
		for (let i = 0; i < 4; i++) {
			send(tab, {
				event: 'agent',
				id: `scout-${tab}`,
				inner: { event: 'tool_call_begin', call: { call_id: `sub-${i}`, name: 'read', summary: 'read file' } }
			});
		}
		send(tab, {
			event: 'context_update',
			used: 10900,
			budget: 26214,
			window: 32768,
			cache_hit_tokens: 6800,
			threshold: 24576
		});
	}

	function devSetting(state: TabState, key: string): string {
		return state.settings.find((r) => r.key === key)?.value ?? '';
	}

	function buildDevInfo(state: TabState): AppInfo {
		const workspace = state.cwd.replace(/^~\//, '/home/you/');
		const repoKey = workspace.split('/').filter(Boolean).pop() ?? 'workspace';
		const memoryPath = parseTomlString(devSetting(state, 'memory.path'));
		const memoryScope = parseTomlString(devSetting(state, 'memory.project_scope')) || 'data-dir';
		const sessionsPath = parseTomlString(devSetting(state, 'sessions.path'));
		const defaultsDir = parseTomlString(devSetting(state, 'defaults.dir'));
		const memoryGlobal = memoryPath || '/home/you/.local/share/quack/memory';
		const memoryProject =
			memoryScope === 'repo' ? `${workspace}/.quack/memory` : `${memoryGlobal}/projects/${repoKey}`;
		const sessionsRoot = sessionsPath || '/home/you/.local/share/quack/sessions';
		const disabledBundles = parseTomlStringArray(devSetting(state, 'bundles.disabled'));
		const disabledHooks = parseTomlStringArray(devSetting(state, 'hooks.disabled'));
		const disabledMcp = parseTomlStringArray(devSetting(state, 'mcp.disabled'));
		const sandboxMode = parseTomlString(devSetting(state, 'sandbox.mode')) || 'auto';
		const effectiveSandbox = state.sandboxOff ? 'off' : sandboxMode;
		const egressAllow = parseTomlStringArray(devSetting(state, 'network.egress_allow'));
		const proxyUrl = parseTomlString(devSetting(state, 'network.proxy'));
		const otelUrl = parseTomlString(devSetting(state, 'telemetry.otel_endpoint'));
		const catalogRefresh = devSetting(state, 'catalog.refresh') === 'true';
		const updatesCheck = devSetting(state, 'updates.check') === 'true';
		const writable = parseTomlStringArray(devSetting(state, 'sandbox.writable'));

		return {
			...INFO,
			cwd: state.cwd,
			branch: state.branch,
			model: parseTomlString(devSetting(state, 'model.default')) || INFO.model,
			planner_model: state.plannerModel,
			permission_mode: parseTomlString(devSetting(state, 'permissions.mode')) || 'ask',
			work_mode: parseTomlString(devSetting(state, 'permissions.work_mode')) || 'edit',
			ui: {
				theme: parseTomlString(devSetting(state, 'ui.theme')) || 'auto',
				reduced_motion: devSetting(state, 'ui.reduced_motion') === 'true'
			},
			egress: {
				...INFO.egress,
				offline: state.offline,
				egress_allow: egressAllow,
				http1_only: devSetting(state, 'network.http1_only') === 'true',
				has_proxy: proxyUrl.length > 0,
				catalog_refresh: catalogRefresh,
				updates_check: updatesCheck,
				telemetry_enabled: otelUrl.length > 0
			},
			sandbox: {
				...INFO.sandbox,
				mode: effectiveSandbox,
				writable,
				status_detail:
					effectiveSandbox === 'off'
						? 'disabled (sandbox.mode = off)'
						: `active (${effectiveSandbox})`
			},
			settings: state.settings.map((r) => ({ ...r })),
			bundles: INFO.bundles.map((b) => ({
				...b,
				enabled: !disabledBundles.includes(b.id)
			})),
			bundles_disabled: disabledBundles,
			hooks: state.hooks.map((h) => {
				const listed = disabledHooks.includes(h.id);
				const enabled = h.enabled && !listed;
				return {
					...h,
					enabled,
					state: enabled ? 'enabled' : 'disabled',
					detail: listed ? 'listed in `disabled`' : enabled ? 'on' : 'disabled in file'
				};
			}),
			hooks_disabled: disabledHooks,
			mcp: state.mcp.map((m) => {
				const listed = disabledMcp.includes(m.id);
				const enabled = m.enabled && !listed;
				return {
					...m,
					enabled,
					state: enabled ? 'enabled' : 'disabled',
					detail: listed ? 'listed in `disabled`' : enabled ? 'enabled' : 'disabled in file'
				};
			}),
			mcp_disabled: disabledMcp,
			paths: {
				defaults: defaultsDir === 'none' ? null : defaultsDir || '/usr/share/quack/defaults',
				system: '/etc/quack',
				user_config: '/home/you/.config/quack',
				user_data: '/home/you/.local/share/quack',
				user_cache: '/home/you/.cache/quack',
				project: `${workspace}/.quack`,
				memory_global: memoryGlobal,
				memory_project: memoryProject,
				sessions_root: sessionsRoot,
				sessions_project: `${sessionsRoot}/${repoKey}`
			},
			history: [...state.inputHistory]
		};
	}

	return {
		kind: 'dev',

		async pickFolder(): Promise<string | null> {
			await sleep(150);
			const openCwds = new Set([...tabs.values()].map((t) => t.cwd));
			const unused = demoFolders.find((dir) => !openCwds.has(dir));
			if (unused) return unused;
			const pick = demoFolders[nextPick % demoFolders.length];
			nextPick++;
			return pick;
		},

		async openFolder(cwd: string, _resume: ResumeRequest): Promise<OpenedTab> {
			const tab = nextTab++;
			const branch = branchFor(cwd);
			const tabState: TabState = {
				cwd,
				branch,
				workMode: 'edit',
				approvals: 'ask',
				effort: null,
				offline: true,
				sandboxOff: false,
				plannerModel: INFO.planner_model,
				activeProfile: 'generic',
				settings: INFO.settings.map((row) => ({ ...row })),
				resolveApproval: null,
				userTurns: [],
				inputHistory: [...INFO.history],
				memoryNotes: seedMemoryNotes(cwd),
				hooks: DEV_HOOKS.map((h) => ({ ...h, command: [...h.command] })),
				mcp: DEV_MCP.map((m) => ({
					...m,
					auto_approve: [...m.auto_approve],
					env_keys: [...m.env_keys],
					header_keys: [...m.header_keys]
				})),
				jiraSetup: createJiraSetupInfo(false)
			};
			tabs.set(tab, tabState);
			const info = buildDevInfo(tabState);
			fan({ tab, info });
			fan({
				tab,
				event: {
					event: 'session_configured',
					session: { ...SESSION, cwd, id: `${SESSION.id}-${tab}` }
				}
			});
			send(tab, { event: 'model_catalog', ...catalog() });
			send(tab, { event: 'mode_changed', work_mode: tabState.workMode, approvals: tabState.approvals });
			send(tab, {
				event: 'commands',
				commands: [
					{ name: 'review', description: 'Review current diff for regressions (demo).', args: '[scope]', source: 'demo', path: 'commands/review.md' },
					{ name: 'compact', description: 'Summarize history to reclaim context.', args: '[focus]', source: 'built-in', path: '' }
				]
			});
			send(tab, {
				event: 'skills',
				skills: [
					{
						name: 'pond-census',
						description: 'Count and report the pond\u2019s residents (demo).',
						source: 'demo',
						dir: '~/Development/demo-pond/.agents/skills/pond-census'
					},
					{
						name: 'migration-notes',
						description: 'Summarize schema changes for the flock (demo).',
						source: 'demo',
						dir: '~/Development/demo-pond/.agents/skills/migration-notes'
					}
				]
			});
			send(tab, {
				event: 'lsp_servers',
				servers: [
					{
						id: 'rust-analyzer',
						command: 'rust-analyzer',
						extensions: ['rs'],
						state: 'ready',
						detail: 'workspace indexed',
						errors: 0,
						warnings: 1,
						log: []
					}
				]
			});
			send(tab, { event: 'diagnostics', errors: 0, warnings: 1 });
			send(tab, { event: 'session_list', sessions: savedSessions() });
			send(tab, { event: 'startup_complete' });
			if (autoDemo) setTimeout(() => void runDemoTurn(tab), 500);
			else if (jobsDemo) setTimeout(() => seedJobsDemo(tab, cwd), 120);
			return { tab, info };
		},

		async sendOp(tab: number, op: ClientOp): Promise<void> {
			const state = tabs.get(tab);
			switch (op.op) {
				case 'user_turn': {
					const text = op.items
						.map((item) => (item.type === 'text' ? item.text : ''))
						.filter(Boolean)
						.join('\n');
					if (state && text.trim()) {
						state.userTurns.push(text);
						if (state.inputHistory.at(-1) !== text) {
							state.inputHistory = [...state.inputHistory, text].slice(-100);
						}
						fan({ tab, history: [...state.inputHistory] });
					}
					void runDemoTurn(tab);
					break;
				}
				case 'continue_turn':
					if (op.proceed) {
						send(tab, { event: 'notice', message: `Granted another batch of steps (turn ${op.turn}).` });
					} else {
						send(tab, { event: 'turn_aborted', turn: op.turn, reason: 'Stopped at step pause.' });
					}
					break;
				case 'set_sandbox':
					if (state) state.sandboxOff = op.off;
					send(tab, { event: 'sandbox', off: op.off });
					break;
				case 'list_rewind_points':
					send(tab, {
						event: 'rewind_points',
						points: (state?.userTurns ?? ['Check the pond module'])
							.slice()
							.reverse()
							.map((text, idx) => ({
								back: idx + 1,
								text,
								files: idx === 0 ? ['src/pond/ducks.ts'] : ['CHANGELOG.md']
							}))
					});
					break;
				case 'rewind':
					if (op.files) {
						send(tab, {
							event: 'files_restored',
							restored: ['src/pond/ducks.ts'],
							conflicts: []
						});
					}
					if (op.conversation) {
						const removed = state ? state.userTurns.splice(Math.max(0, state.userTurns.length - op.back)) : [];
						send(tab, { event: 'rewound', back: op.back, text: removed[0] ?? 'Previous turn' });
					}
					break;
				case 'fork_session':
					send(tab, {
						event: 'session_configured',
						session: {
							...SESSION,
							cwd: state?.cwd ?? INFO.cwd,
							id: `${SESSION.id}-fork-${Date.now()}`
						}
					});
					send(tab, {
						event: 'notice',
						message: 'Forked: working in a branched session; original saved.'
					});
					break;
				case 'export':
					send(tab, {
						event: 'notice',
						message: `Exported transcript (${op.format}) to ${op.path ?? 'field-notes.' + (op.format === 'json' ? 'json' : 'md')}`
					});
					break;
				case 'save_setting': {
					const layerPath =
						op.scope === 'project' ? `${state?.cwd ?? '~/workspace'}/.quack/config.toml` : '~/.config/quack/config.toml';
					const layerLabel = op.scope === 'project' ? 'project' : 'user';
					if (state) {
						if (op.value.trim() === 'null') {
							if (op.key === 'context.window') {
								state.settings = state.settings.filter((r) => r.key !== 'context.window');
							} else {
								const def = SETTING_FIELDS.find((f) => f.key === op.key);
								const existing = state.settings.find((r) => r.key === op.key);
								if (existing && def) {
									existing.value = def.defaultValue;
									existing.origin = 'default';
								}
							}
						} else {
							const existing = state.settings.find((r) => r.key === op.key);
							const def = SETTING_FIELDS.find((f) => f.key === op.key);
							if (existing) {
								existing.value = op.value;
								existing.origin = `${layerLabel}: ${layerPath}`;
							} else if (def) {
								state.settings = [
									...state.settings,
									{
										key: op.key,
										value: op.value,
										origin: `${layerLabel}: ${layerPath}`,
										description: def.fallbackDescription,
										is_bool: def.kind === 'bool',
										locked: false
									}
								];
							}
						}
						if (op.key === 'network.offline') state.offline = op.value === 'true';
						if (op.key === 'sandbox.mode') state.sandboxOff = parseTomlString(op.value) === 'off';
						if (op.key === 'model.planner') {
							state.plannerModel = op.value === '""' ? null : parseTomlString(op.value) || null;
						}
					}
					send(tab, {
						event: 'setting_saved',
						key: op.key,
						value: op.value,
						path: layerPath
					});
					break;
				}
				case 'reload_config':
					if (state) {
						fan({
							tab,
							info: buildDevInfo(state)
						});
					}
					send(tab, { event: 'config_reloaded' });
					send(tab, { event: 'startup_complete' });
					break;
				case 'list_profiles':
					send(tab, {
						event: 'profiles',
						active: state?.activeProfile ?? 'generic',
						forced: (state ? parseTomlString(devSetting(state, 'model.profile')) : '') !== '',
						profiles: [
							{
								id: 'generic',
								description: 'Balanced defaults across open-weight and hosted models',
								source: 'built-in',
								matches: true,
								edit_format: 'str_replace',
								tools: ['read', 'edit', 'write', 'shell', 'grep', 'glob'],
								system: 'You are quack, a careful coding agent.'
							},
							{
								id: 'cc-style',
								description: 'Compact tool prompts tuned for GLM / DeepSeek / MiniMax',
								source: 'defaults',
								matches: false,
								edit_format: 'str_replace',
								tools: ['read', 'edit', 'write', 'shell', 'grep', 'glob'],
								system: 'Compact cc-style prompt.'
							},
							{
								id: 'codex',
								description: 'Shell-first multi-file `apply_patch` workflow',
								source: 'defaults',
								matches: false,
								edit_format: 'apply_patch',
								tools: ['read', 'apply_patch', 'shell'],
								system: 'Shell-first apply_patch prompt.'
							}
						]
					});
					break;
				case 'set_profile':
					if (state) state.activeProfile = op.id === 'auto' ? 'generic' : op.id;
					send(tab, {
						event: 'notice',
						message: `Switched session profile to ${state?.activeProfile ?? op.id}.`
					});
					break;
				case 'list_mcp': {
					const disabledMcp = state
						? parseTomlStringArray(devSetting(state, 'mcp.disabled'))
						: [];
					const servers: McpServerInfo[] = (state?.mcp ?? DEV_MCP)
						.filter((m) => m.enabled && !disabledMcp.includes(m.id))
						.map((m) => ({
							id: m.id,
							transport: m.transport ?? 'stdio',
							target: m.target,
							description: m.description,
							state: 'ready',
							tools: m.id === 'filesystem-demo' ? ['read_file', 'list_directory'] : ['status'],
							prompts: m.id === 'filesystem-demo' ? ['summarize'] : [],
							resources: m.id === 'filesystem-demo' ? ['file://README.md'] : [],
							deferred: false,
							tool_tokens: 420,
							log: [`[${m.id}] ready (${m.transport ?? 'stdio'})`]
						}));
					send(tab, { event: 'mcp_servers', servers });
					break;
				}
				case 'restart_mcp':
					send(tab, { event: 'notice', message: `Restarted MCP server ${op.id} (demo).` });
					break;
				case 'enable_mcp': {
					if (!state) break;
					const target = state.mcp.find((m) => m.id === op.id);
					if (!target) {
						send(tab, {
							event: 'error',
							error: { kind: 'config', message: `no MCP server named \`${op.id}\`` }
						});
						break;
					}
					target.enabled = true;
					target.state = 'enabled';
					target.detail = 'enabled';
					if (target.layer === 'defaults') {
						target.layer = 'user';
						target.file = `/home/you/.config/quack/mcp.d/${op.id}.toml`;
					}
					const nextDisabled = parseTomlStringArray(devSetting(state, 'mcp.disabled')).filter(
						(x) => x !== op.id
					);
					const row = state.settings.find((r) => r.key === 'mcp.disabled');
					if (row) row.value = formatTomlStringArray(nextDisabled);
					send(tab, { event: 'mcp_enabled', id: op.id, path: target.file });
					fan({ tab, info: buildDevInfo(state) });
					send(tab, { event: 'config_reloaded' });
					send(tab, { event: 'startup_complete' });
					break;
				}
				case 'save_mcp_server': {
					if (!state) break;
					const id = op.id.trim();
					const path =
						op.scope === 'project'
							? `${state.cwd}/.quack/mcp.d/${id}.toml`
							: `/home/you/.config/quack/mcp.d/${id}.toml`;
					const transport = op.url.trim() ? 'http' : 'stdio';
					const target =
						transport === 'http'
							? op.url.trim()
							: op.args.length > 0
								? `${op.command.trim()} ${op.args.join(' ')}`
								: op.command.trim();
					const nextRow: McpRow = {
						id,
						layer: op.scope === 'project' ? 'project' : 'user',
						file: path,
						state: 'enabled',
						detail: 'enabled',
						enabled: true,
						can_toggle: true,
						blocked_by_policy: false,
						transport,
						target,
						description: op.description,
						sandbox: op.sandbox ?? null,
						startup_timeout_secs: 20,
						call_timeout_secs: 120,
						max_output_tokens: 8000,
						auto_approve: [],
						env_keys: Object.keys(op.env),
						header_keys: Object.keys(op.headers)
					};
					state.mcp = [...state.mcp.filter((m) => m.id !== id), nextRow];
					send(tab, { event: 'mcp_server_saved', id, path });
					fan({ tab, info: buildDevInfo(state) });
					send(tab, { event: 'config_reloaded' });
					send(tab, { event: 'startup_complete' });
					break;
				}
				case 'enable_hook': {
					if (!state) break;
					const target = state.hooks.find((h) => h.id === op.id);
					if (!target) {
						send(tab, {
							event: 'error',
							error: { kind: 'config', message: `no hook named \`${op.id}\`` }
						});
						break;
					}
					target.enabled = true;
					target.state = 'enabled';
					target.detail = 'on';
					if (target.layer === 'defaults') {
						target.layer = 'user';
						target.file = `/home/you/.config/quack/hooks.d/${op.id}.toml`;
					}
					if (op.id === 'jira') state.jiraSetup.enabled = true;
					const nextDisabled = parseTomlStringArray(devSetting(state, 'hooks.disabled')).filter(
						(x) => x !== op.id
					);
					const row = state.settings.find((r) => r.key === 'hooks.disabled');
					if (row) row.value = formatTomlStringArray(nextDisabled);
					send(tab, { event: 'hook_enabled', id: op.id, path: target.file, note: null });
					fan({ tab, info: buildDevInfo(state) });
					send(tab, { event: 'config_reloaded' });
					send(tab, { event: 'startup_complete' });
					break;
				}
				case 'inspect_hook_setup':
					if (!state || op.id !== 'jira') {
						send(tab, {
							event: 'hook_setup_failed',
							request: op.request,
							message: `No interactive setup form declared for hook \`${op.id}\`.`
						});
					} else {
						send(tab, {
							event: 'hook_setup',
							request: op.request,
							info: structuredClone(state.jiraSetup)
						});
					}
					break;
				case 'save_hook_setup':
					if (!state || op.id !== 'jira') {
						send(tab, {
							event: 'hook_setup_failed',
							request: op.request,
							message: `Cannot save setup for unknown hook \`${op.id}\`.`
						});
					} else {
						const enabled = op.enable ?? state.jiraSetup.enabled;
						state.jiraSetup = {
							...state.jiraSetup,
							revision: `rev-${Date.now()}`,
							enabled,
							source: 'user',
							connections: op.connections.map((draft) => ({
								connection: structuredClone(draft.connection),
								credentials: Object.fromEntries(
									Object.entries(draft.credentials).map(([key, change]) => [
										key,
										change.action === 'replace'
											? { from: 'keyring', name: `quack:${op.id}:${draft.connection.id}` }
											: { from: 'env', var: 'JIRA_API_TOKEN' }
									])
								)
							}))
						};
						const hookRow = state.hooks.find((h) => h.id === op.id);
						if (hookRow) {
							hookRow.enabled = enabled;
							hookRow.state = enabled ? 'enabled' : 'disabled';
							hookRow.detail = enabled ? 'on' : 'disabled in file';
							hookRow.connections_count = op.connections.length;
							hookRow.layer = 'user';
							hookRow.file = '/home/you/.config/quack/hooks.d/jira.toml';
						}
						send(tab, {
							event: 'hook_setup_saved',
							request: op.request,
							info: structuredClone(state.jiraSetup),
							note: `Saved ${op.connections.length} connection(s) in demo config.`
						});
						fan({ tab, info: buildDevInfo(state) });
						send(tab, { event: 'config_reloaded' });
						send(tab, { event: 'startup_complete' });
					}
					break;
				case 'check_hook_setup':
					send(tab, {
						event: 'hook_check_result',
						request: op.request,
						status: 'ok',
						message: `Reachable (${op.url || op.id}) — browser demo check passed.`
					});
					break;
				case 'cancel_hook_check':
					break;
				case 'list_lsp':
					send(tab, {
						event: 'lsp_servers',
						servers: [
							{
								id: 'rust-analyzer',
								command: 'rust-analyzer',
								extensions: ['rs'],
								state: 'ready',
								detail: 'workspace indexed',
								errors: 0,
								warnings: 1,
								log: []
							}
						]
					});
					break;
				case 'interrupt':
					send(tab, { event: 'turn_aborted', turn: 1, reason: 'Stopped by user' });
					break;
				case 'approval_response':
					state?.resolveApproval?.();
					if (state) state.resolveApproval = null;
					break;
				case 'set_work_mode':
					if (state) state.workMode = op.mode;
					send(tab, {
						event: 'mode_changed',
						work_mode: state?.workMode ?? op.mode,
						approvals: state?.approvals ?? 'ask'
					});
					if (op.mode === 'plan') {
						send(tab, {
							event: 'plan',
							checklist: '[x] read the pond\n[ ] count the ducks\n[ ] export the census'
						});
					}
					break;
				case 'set_approval_policy':
					if (state) state.approvals = op.policy;
					send(tab, {
						event: 'mode_changed',
						work_mode: state?.workMode ?? 'edit',
						approvals: state?.approvals ?? op.policy
					});
					break;
				case 'set_reasoning_effort':
					if (state) state.effort = op.effort ?? null;
					send(tab, {
						event: 'session_configured',
						session: {
							...SESSION,
							cwd: state?.cwd ?? INFO.cwd,
							model: 'dev-local/thinker',
							reasoning_effort: state?.effort ?? null,
							reasoning_efforts: ['low', 'medium', 'high']
						}
					});
					break;
				case 'set_model': {
					const model = `${op.provider}/${op.model}`;
					if (model === 'dev-local/thinker')
						send(tab, {
							event: 'session_configured',
							session: {
								...SESSION,
								cwd: state?.cwd ?? INFO.cwd,
								model,
								reasoning_effort: state?.effort ?? null,
								reasoning_efforts: ['low', 'medium', 'high']
							}
						});
					else
						send(tab, {
							event: 'session_configured',
							session: { ...SESSION, cwd: state?.cwd ?? INFO.cwd, model }
						});
					send(tab, {
						event: 'model_catalog',
						...catalog(),
						current: model
					});
					break;
				}
				case 'list_models':
					send(tab, { event: 'model_catalog', ...catalog() });
					break;
				case 'list_services':
					send(tab, {
						event: 'services',
						services: [
						{
							subscription: true,
							id: 'dev-plan',
							name: 'Dev Plan (scripted)',
							group: 'demo',
							base_url: '',
							ready: false
						}
						]
					});
					break;
				case 'sign_in':
					if (op.id) {
						const id = op.id;
						send(tab, {
							event: 'sign_in_url',
							id,
							url: 'https://sign-in.example.dev/authorize?demo=1'
							});
						setTimeout(() => {
							send(tab, { event: 'sign_in_result', id, error: null });
							send(tab, { event: 'services', services: [] });
						}, 4000);
					}
					break;
				case 'detect_servers':
					send(tab, { event: 'servers_detected', servers: [] });
					break;
				case 'add_server':
				case 'set_provider_key':
					send(tab, { event: 'error', error: { kind: 'config', message: 'The browser demo does not save providers. Open the desktop app to connect a provider.' } });
					break;
				case 'new_session':
					send(tab, { event: 'session_title', title: undefined });
					send(tab, { event: 'session_configured', session: { ...SESSION, id: `dev-session-${Date.now()}` } });
					break;
				case 'resume_session':
					send(tab, { event: 'warning', message: 'The browser demo has no saved sessions.' });
					break;
				case 'list_sessions':
					send(tab, { event: 'session_list', sessions: savedSessions() });
					break;
				case 'get_stats':
					send(tab, {
					event: 'stats',
					report: {
						model: 'dev-local/tiny',
						turns: 3,
						requests: 9,
						unreported: 0,
						prompt_tokens: 41200,
						completion_tokens: 1490,
						cached_tokens: 33100,
						cache_reported: 9,
						unpriced: 9,
						ttft_ms: [240, 610],
						tokens_per_sec: 42.7,
						open_ms: 187000,
						model_ms: 52000,
						tools_ms: 21000,
						approval_ms: 9000,
						tools: [
							{ name: 'read', calls: 14, failed: 0, ms: 210 },
							{ name: 'write', calls: 1, failed: 0, ms: 40 },
							{ name: 'task', calls: 1, failed: 0, ms: 8400 }
						],
						compactions: 0,
						pruned: 1,
						resumed: false,
						history: [
							{ prompt: 640, cached: 0, completion: 60 },
							{ prompt: 3900, cached: 2900, completion: 210 },
							{ prompt: 4400, cached: 3600, completion: 180 },
							{ prompt: 4600, cached: 4100, completion: 240 },
							{ prompt: 5100, cached: 4700, completion: 160 },
							{ prompt: 5300, cached: 4900, completion: 300 },
							{ prompt: 5600, cached: 5200, completion: 140 },
							{ prompt: 5900, cached: 5500, completion: 120 },
							{ prompt: 6360, cached: 6000, completion: 80 }
						]
					}
				});
					break;
				case 'get_context':
					send(tab, {
						event: 'context_report',
						report: {
							model: 'dev-local/tiny',
							window: 32768,
							budget: 26214,
							threshold: 24576,
							used: 4132,
							strategy: 'cache',
							segments: [
								{ name: 'System prompt', tokens: 910, detail: 'generic profile' },
								{ name: 'History', tokens: 2871, detail: '2 messages, 1 pruned' },
								{ name: 'Tools', tokens: 351, detail: '11 tools' }
							]
						}
					});
					break;
				case 'list_memory': {
					const notes = state?.memoryNotes ?? [];
					send(tab, { event: 'memory_notes', notes: [...notes], index_fill: memoryIndexFill(notes) });
					break;
				}
				case 'save_memory': {
					if (!state) break;
					const { scope, name, description, kind, body, previous_scope, previous_name } = op;
					if (scope !== 'project' && scope !== 'global') {
						send(tab, { event: 'warning', message: `cannot save note: unknown scope \`${scope}\`` });
						break;
					}
					if (!['user', 'project', 'feedback', 'reference'].includes(kind)) {
						send(tab, { event: 'warning', message: `cannot save note: unknown type \`${kind}\`` });
						break;
					}
					if (!/^[a-z0-9][a-z0-9-]{0,63}$/.test(name)) {
						send(tab, {
							event: 'warning',
							message: 'cannot save note: note name must be 1–64 lowercase letters, digits, or hyphens'
						});
						break;
					}
					if (!description.trim() || !body.trim()) {
						send(tab, { event: 'warning', message: 'cannot save note: description and body cannot be empty' });
						break;
					}
					const hasPrev = previous_scope !== undefined && previous_name !== undefined;
					if ((previous_scope !== undefined) !== (previous_name !== undefined)) {
						send(tab, {
							event: 'warning',
							message: 'cannot save note: previous_scope and previous_name must both be set when moving a note'
						});
						break;
					}
					const renaming = hasPrev && (previous_scope !== scope || previous_name !== name);
					if (renaming && state.memoryNotes.some((n) => n.scope === scope && n.name === name)) {
						send(tab, {
							event: 'warning',
							message: `cannot save note: a ${scope} note named \`${name}\` already exists`
						});
						break;
					}
					const base = state.cwd.replace(/^~\//, '/home/you/');
					const path =
						scope === 'project'
							? `${base}/.quack/memory/notes/${name}.md`
							: `/home/you/.local/share/quack/memory/notes/${name}.md`;
					const today = new Date().toISOString().slice(0, 10);
					const nextNote: MemoryNote = {
						scope,
						name,
						description: description.trim(),
						path,
						text: formatDemoNote(name, description.trim(), kind, body, today)
					};
					state.memoryNotes = state.memoryNotes.filter((n) => {
						if (n.scope === scope && n.name === name) return false;
						if (hasPrev && n.scope === previous_scope && n.name === previous_name) return false;
						return true;
					});
					state.memoryNotes.push(nextNote);
					send(tab, {
						event: 'memory_notes',
						notes: [...state.memoryNotes],
						index_fill: memoryIndexFill(state.memoryNotes)
					});
					break;
				}
				case 'delete_memory': {
					if (!state) break;
					const idx = state.memoryNotes.findIndex((n) => n.scope === op.scope && n.name === op.name);
					if (idx === -1) {
						send(tab, { event: 'warning', message: `no ${op.scope} note named \`${op.name}\`` });
						break;
					}
					state.memoryNotes.splice(idx, 1);
					send(tab, {
						event: 'memory_notes',
						notes: [...state.memoryNotes],
						index_fill: memoryIndexFill(state.memoryNotes)
					});
					break;
				}
				case 'check_updates': {
					const updatesUrl = state ? parseTomlString(devSetting(state, 'updates.url')).trim() : '';
					const offline = state ? devSetting(state, 'network.offline') === 'true' : false;
					if (!updatesUrl) {
						send(tab, {
							event: 'update_check_result',
							current: INFO.version,
							latest: null,
							update_available: false,
							url: null,
							error: 'no update source is set · set updates.url'
						});
						break;
					}
					const host = extractUrlHost(updatesUrl) ?? updatesUrl;
					const loopback =
						host.startsWith('localhost') || host.startsWith('127.') || host.startsWith('[::1]');
					if (offline && !loopback) {
						send(tab, {
							event: 'update_check_result',
							current: INFO.version,
							latest: null,
							update_available: false,
							url: updatesUrl,
							error: `blocked by network.offline: add \`${host}\` to network.egress_allow to use it`
						});
						break;
					}
					send(tab, {
						event: 'update_check_result',
						current: INFO.version,
						latest: INFO.version,
						update_available: false,
						url: updatesUrl,
						error: null
					});
					break;
				}
				default:
					send(tab, { event: 'notice', message: `the dev transport ignores ${op.op}` });
			}
		},

		async closeTab(tab: number): Promise<void> {
			tabs.delete(tab);
			fan({ tab, closed: { reason: null } });
		},

		async openUrl(url: string): Promise<void> {
			// The demo runs in a plain browser; a popup is the honest stand-in for the
			// desktop shell's system browser.
			window.open(url, '_blank', 'noopener');
		},

		async listFiles(): Promise<string[]> {
			return [
				'src/main.rs',
				'src/export.rs',
				'src/pond/ducks.ts',
				'CHANGELOG.md',
				'Cargo.toml',
				'README.md'
			];
		},

		async inputHistory(tab: number): Promise<string[]> {
			return [...(tabs.get(tab)?.inputHistory ?? INFO.history)];
		},

		async revealPath(tab: number, path: string): Promise<void> {
			send(tab, { event: 'notice', message: `Desktop app reveals "${path}" in your file manager.` });
		},

		async highlightCode(): Promise<HighlightSpan[] | null> {
			return null;
		},

		async pickImages(): Promise<string[]> {
			return [];
		},

		async pickSavePath(defaultName: string): Promise<string | null> {
			return defaultName;
		},

		async stagePastedImage(): Promise<string> {
			throw new Error('Pasting image bytes requires the desktop app.');
		},

		async notifyAttention(): Promise<void> {},

		async recentFolders(_limit = 10): Promise<RecentFolder[]> {
			return [
				{ path: '/home/you/Development/demo-pond', opened: 1 },
				{ path: '/home/you/Development/quack', opened: 2 }
			];
		},

		listen(listener: (message: TabMessage) => void): () => void {
			listeners.add(listener);
			return () => listeners.delete(listener);
		}
	};
}
