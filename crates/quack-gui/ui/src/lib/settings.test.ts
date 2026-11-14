import { describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import SettingsFieldRow from './components/SettingsFieldRow.svelte';
import SettingsPanel from './components/SettingsPanel.svelte';
import SetupPanel from './components/SetupPanel.svelte';
import { Session } from './session.svelte';
import {
	SETTINGS_SECTIONS,
	SETTING_FIELDS,
	SETTING_FIELD_MAP,
	buildAllSettingRows,
	buildSettingRowMap,
	extractUrlHost,
	formatTomlString,
	formatTomlStringArray,
	matchesSettingSearch,
	parseOrigin,
	parseTomlNumber,
	parseTomlString,
	parseTomlStringArray,
	prepareFieldTomlValue,
	resolvedPathRows
} from './settings';
import type { AppInfo, Transport } from './transport/types';

function makeInfo(overrides: Partial<AppInfo> = {}): AppInfo {
	return {
		version: 'test',
		cwd: '/workspace/demo',
		branch: 'main',
		model: 'openai/gpt-5',
		planner_model: null,
		planner_locked: false,
		permission_mode: 'ask',
		work_mode: 'edit',
		ui: { theme: 'auto', reduced_motion: false },
		egress: {
			offline: false,
			offline_locked: false,
			egress_allow: ['https://api.example.com'],
			providers: [{ id: 'openai', host: 'api.openai.com', local: false, enabled: true }],
			mcp_remote: [],
			http1_only: false,
			has_proxy: false,
			updates_check: true,
			catalog_refresh: false,
			telemetry_enabled: false
		},
		sandbox: {
			mode: 'auto',
			os_supported: true,
			status_detail: 'active (Landlock + seccomp)',
			writable: ['/tmp/shared'],
			locked: false
		},
		project_status: 'Trusted (hash matches)',
		paths: {
			user_config: '/home/paul/.config/quack',
			user_data: '/home/paul/.local/share/quack',
			user_cache: '/home/paul/.cache/quack',
			project: '/workspace/demo/.quack',
			system: '/etc/quack',
			defaults: '/workspace/demo/defaults',
			memory_global: '/custom/memories',
			memory_project: '/custom/memories/projects/demo-123',
			sessions_root: '/home/paul/.local/share/quack/sessions',
			sessions_project: '/home/paul/.local/share/quack/sessions/demo-123'
		},
		settings: [
			{
				key: 'memory.path',
				value: '"/custom/memories"',
				origin: 'user:~/.config/quack/config.toml',
				description: 'Where memory notes live.',
				is_bool: false,
				locked: false
			},
			{
				key: 'model.default',
				value: '"openai/gpt-5"',
				origin: 'user:~/.config/quack/config.toml',
				description: 'Default provider/model.',
				is_bool: false,
				locked: false
			},
			{
				key: 'model.profile',
				value: '"generic"',
				origin: 'default',
				description: 'Prompt profile id.',
				is_bool: false,
				locked: false
			},
			{
				key: 'catalog.refresh',
				value: 'true',
				origin: 'user:~/.config/quack/config.toml',
				description: 'Opt-in background catalog refresh.',
				is_bool: true,
				locked: false
			},
			{
				key: 'updates.check',
				value: 'false',
				origin: 'default',
				description: 'Opt-in version check.',
				is_bool: true,
				locked: false
			},
			{
				key: 'sandbox.mode',
				value: '"auto"',
				origin: 'default',
				description: 'Sandbox enforcement mode.',
				is_bool: false,
				locked: false
			},
			{
				key: 'permissions.mode',
				value: '"ask"',
				origin: 'default',
				description: 'Default approval mode.',
				is_bool: false,
				locked: false
			}
		],
		layers: [
			{ layer: 'defaults', status: 'loaded', file: '/workspace/demo/defaults/config.toml' },
			{ layer: 'user', status: 'loaded', file: '/home/paul/.config/quack/config.toml' },
			{ layer: 'project', status: 'no config.toml', file: null }
		],
		rules: [{ list: 'allow', rule: 'Bash(cargo test *)' }],
		bundles: [
			{
				id: 'quack-skills',
				layer: 'user',
				enabled: true,
				contents: '1 skill'
			}
		],
		bundles_disabled: [],
		hooks: [
			{
				id: 'jira',
				layer: 'defaults',
				file: '/usr/share/quack/defaults/hooks.d/jira.toml',
				state: 'disabled',
				detail: 'disabled in file',
				enabled: false,
				can_toggle: true,
				has_setup: true,
				description: 'Fetch Jira tickets',
				event: 'user_prompt',
				command: [],
				matcher_tool: '',
				matcher_pattern: '',
				matcher_prompt: '',
				timeout_secs: 15,
				on_error: 'warn',
				full_context: false,
				connections_count: 0
			}
		],
		hooks_disabled: [],
		mcp: [
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
				description: 'GitHub MCP server',
				sandbox: null,
				startup_timeout_secs: 20,
				call_timeout_secs: 120,
				max_output_tokens: 8000,
				auto_approve: [],
				env_keys: ['GITHUB_PERSONAL_ACCESS_TOKEN'],
				header_keys: []
			}
		],
		mcp_disabled: [],
		history: [],
		...overrides
	};
}

function makeSession(infoOverrides: Partial<AppInfo> = {}): Session {
	const info = makeInfo(infoOverrides);
	const transport: Transport = {
		kind: 'tauri',
		pickFolder: vi.fn().mockResolvedValue('/picked/dir'),
		recentFolders: vi.fn(),
		openFolder: vi.fn(),
		sendOp: vi.fn().mockResolvedValue(undefined),
		closeTab: vi.fn(),
		openUrl: vi.fn().mockResolvedValue(undefined),
		listen: () => () => {}
	};
	const session = new Session(transport, 1, info);
	session.providers = [
		{
			id: 'openai',
			available: true,
			note: 'ready',
			bundled: true,
			models: [
				{
					id: 'gpt-5',
					context_window: 128000
				}
			]
		}
	];
	session.profiles = {
		active: 'generic',
		forced: false,
		profiles: [
			{
				id: 'generic',
				description: 'Balanced default profile',
				source: 'builtin',
				matches: true,
				edit_format: 'str_replace',
				tools: ['read', 'edit', 'shell'],
				system: 'You are quack.'
			},
			{
				id: 'cc-style',
				description: 'CC Style',
				source: 'defaults',
				matches: false,
				edit_format: 'str_replace',
				tools: ['read', 'edit', 'shell'],
				system: 'CC prompt'
			}
		]
	};
	return session;
}

describe('settings metadata and TOML helpers', () => {
	it('registers all unique editable fields under valid sections', () => {
		const sectionIds = new Set(SETTINGS_SECTIONS.map((s) => s.id));
		const seen = new Set<string>();
		for (const field of SETTING_FIELDS) {
			expect(sectionIds.has(field.section)).toBe(true);
			expect(seen.has(field.key)).toBe(false);
			seen.add(field.key);
		}
		expect(SETTING_FIELDS.length).toBeGreaterThanOrEqual(55);
		expect(SETTING_FIELD_MAP.get('memory.path')?.section).toBe('paths');
		expect(SETTING_FIELD_MAP.get('mcp.disabled')?.section).toBe('extensions');
		expect(SETTING_FIELD_MAP.get('hooks.disabled')?.section).toBe('extensions');
		expect(SETTING_FIELD_MAP.get('context.window')?.supportsNullReset).toBe(true);
	});

	it('parses and formats TOML strings cleanly including Windows backslashes and literals', () => {
		expect(parseTomlString('"/tmp/memories"')).toBe('/tmp/memories');
		expect(parseTomlString('"C:\\\\Users\\\\paul\\\\quack"')).toBe('C:\\Users\\paul\\quack');
		expect(parseTomlString("'C:\\raw\\path'")).toBe('C:\\raw\\path');
		expect(parseTomlString('null')).toBe('');
		expect(parseTomlString('plain-value')).toBe('plain-value');

		const escaped = formatTomlString('C:\\quack\\notes "v1"');
		expect(parseTomlString(escaped)).toBe('C:\\quack\\notes "v1"');
	});

	it('parses and formats TOML string arrays cleanly', () => {
		expect(parseTomlStringArray('[]')).toEqual([]);
		expect(
			parseTomlStringArray(`["~/.quack/skills", '~/.agents/skills', "dir with, comma\\"q\\""]`)
		).toEqual(['~/.quack/skills', '~/.agents/skills', 'dir with, comma"q"']);
		expect(formatTomlStringArray(['a', 'b\\c'])).toBe('["a", "b\\\\c"]');
	});

	it('parses TOML numbers and origins accurately', () => {
		expect(parseTomlNumber('16_384')).toBe(16384);
		expect(parseTomlNumber('0.85')).toBeCloseTo(0.85);
		expect(parseTomlNumber('null')).toBeNull();
		expect(parseTomlNumber('bad')).toBeNull();

		expect(parseOrigin('default')).toEqual({ layer: 'default', label: 'default', detail: '' });
		expect(parseOrigin('cli')).toEqual({
			layer: 'cli',
			label: 'cli',
			detail: 'command-line override'
		});
		expect(parseOrigin('user:~/.config/quack/config.toml')).toEqual({
			layer: 'user',
			label: 'user',
			detail: '~/.config/quack/config.toml'
		});
		expect(parseOrigin('project:.quack/config.toml').layer).toBe('project');
		expect(parseOrigin('system:/etc/quack/config.toml').layer).toBe('system');
		expect(parseOrigin('env:QUACK_OFFLINE').layer).toBe('env');
		expect(parseOrigin('defaults:bundled').layer).toBe('default');
	});

	it('extracts host from URLs cleanly', () => {
		expect(extractUrlHost('https://api.example.com:8443/v1')).toBe('api.example.com:8443');
		expect(extractUrlHost('not a url')).toBeNull();
		expect(extractUrlHost('')).toBeNull();
	});

	it('fills missing optional schema rows like context.window and searches across fields', () => {
		const partial = makeInfo().settings;
		const map = buildSettingRowMap(partial);
		expect(map.get('context.window')?.value).toBe('null');
		expect(map.get('context.window')?.origin).toBe('default');

		const all = buildAllSettingRows(partial);
		expect(all).toHaveLength(SETTING_FIELDS.length);
		expect(all.map((r) => r.key)).toEqual(
			[...all.map((r) => r.key)].sort((a, b) => a.localeCompare(b))
		);

		const memRow = map.get('memory.path')!;
		const memField = SETTING_FIELD_MAP.get('memory.path')!;
		expect(matchesSettingSearch('memory custom', memRow, memField)).toBe(true);
		expect(matchesSettingSearch('telemetry', memRow, memField)).toBe(false);
	});

	it('builds resolved path rows with fallback and defaults=null handling', () => {
		expect(resolvedPathRows('/repo')).toEqual([
			{ id: 'cwd', label: 'Workspace', path: '/repo', hint: 'Active repository root' }
		]);
		const rows = resolvedPathRows('/repo', makeInfo({ paths: { ...makeInfo().paths!, defaults: null } }).paths);
		expect(rows.find((r) => r.id === 'memory_project')?.path).toBe('/custom/memories/projects/demo-123');
		expect(rows.find((r) => r.id === 'defaults')?.path).toBe('(none)');
	});

	it('validates and formats TOML values across field kinds', () => {
		const maxSteps = SETTING_FIELD_MAP.get('agent.max_steps')!;
		expect(prepareFieldTomlValue(maxSteps, '40')).toEqual({ ok: true, value: '40' });
		expect(prepareFieldTomlValue(maxSteps, '0').ok).toBe(false);
		expect(prepareFieldTomlValue(maxSteps, 'abc').ok).toBe(false);

		const ctxWindow = SETTING_FIELD_MAP.get('context.window')!;
		expect(prepareFieldTomlValue(ctxWindow, '')).toEqual({ ok: true, value: 'null' });
		expect(prepareFieldTomlValue(ctxWindow, 'null')).toEqual({ ok: true, value: 'null' });
		expect(prepareFieldTomlValue(ctxWindow, '32768')).toEqual({ ok: true, value: '32768' });
		expect(prepareFieldTomlValue(ctxWindow, '0').ok).toBe(false);

		const budgetRatio = SETTING_FIELD_MAP.get('context.budget_ratio')!;
		expect(prepareFieldTomlValue(budgetRatio, '0.75')).toEqual({ ok: true, value: '0.75' });
		expect(prepareFieldTomlValue(budgetRatio, '1.5').ok).toBe(false);

		const memPath = SETTING_FIELD_MAP.get('memory.path')!;
		expect(prepareFieldTomlValue(memPath, ' /tmp/mem ')).toEqual({
			ok: true,
			value: '"/tmp/mem"'
		});
	});
});

describe('SettingsPanel & SettingsFieldRow rendering', () => {
	it('renders resolved memory directories and path settings on the paths tab', () => {
		const session = makeSession();
		session.settingsTab = 'paths';
		const { body } = render(SettingsPanel, { props: { session } });
		expect(body).toContain('Paths &amp; Storage');
		expect(body).toContain('/custom/memories/projects/demo-123');
		expect(body).toContain('/home/paul/.local/share/quack/sessions');
		expect(body).toContain('Memory directory');
		expect(body).toContain('Config layers (lowest to highest precedence)');
		expect(body).toContain('/home/paul/.config/quack/config.toml');
	});

	it('renders quack setup options, background checks, and profiles on the setup tab', () => {
		const session = makeSession();
		session.settingsTab = 'setup';
		const { body } = render(SettingsPanel, { props: { session } });
		expect(body).toContain('Setup &amp; Models');
		expect(body).toContain('model.default');
		expect(body).toContain('catalog.refresh');
		expect(body).toContain('updates.check');
		expect(body).toContain('cc-style');
	});

	it('renders nullable context.window unset state on the context tab', () => {
		const session = makeSession();
		session.settingsTab = 'context';
		const { body } = render(SettingsPanel, { props: { session } });
		expect(body).toContain('Agent, Context &amp; Tools');
		expect(body).toContain('context.window');
		expect(body).toContain('Unset');
	});

	it('renders sandbox mode, permission rules, and bundles on the permissions tab', () => {
		const session = makeSession();
		session.settingsTab = 'permissions';
		const { body } = render(SettingsPanel, { props: { session } });
		expect(body).toContain('Permissions, Sandbox &amp; Bundles');
		expect(body).toContain('quack-skills');
		expect(body).toContain('Bash(cargo test *)');
		expect(body).toContain('sandbox.mode');
	});

	it('renders hooks and MCP servers on the extensions tab', () => {
		const session = makeSession();
		session.settingsTab = 'extensions';
		const { body } = render(SettingsPanel, { props: { session } });
		expect(body).toContain('Hooks &amp; MCP Servers');
		expect(body).toContain('MCP servers (1)');
		expect(body).toContain('github');
		expect(body).toContain('Lifecycle hooks (1)');
		expect(body).toContain('jira');
		expect(body).toContain('Configure…');
	});

	it('renders the raw effective TOML table on the interface tab', () => {
		const session = makeSession();
		session.settingsTab = 'interface';
		const { body } = render(SettingsPanel, { props: { session } });
		expect(body).toContain('All TOML settings');
		expect(body).toContain('telemetry.otel_endpoint');
	});

	it('renders desktop text resizing controls in Desktop &amp; terminal appearance', () => {
		const session = makeSession();
		session.settingsTab = 'interface';
		const { body } = render(SettingsPanel, { props: { session } });
		expect(body).toContain('Desktop &amp; terminal appearance');
		expect(body).toContain('desktop.text_scale');
		expect(body).toContain('Text size');
		expect(body).toContain('Desktop');
		expect(body).toContain('100%');
	});

	it('renders Defaults, profiles & background checks link on SetupPanel', () => {
		const session = makeSession();
		const { body } = render(SetupPanel, { props: { session } });
		expect(body).toContain('Defaults, profiles');
		expect(body).toContain('Open Settings');
	});

	it('renders Locked and TUI-only badges on SettingsFieldRow', () => {
		const row: AppInfo['settings'][number] = {
			key: 'ui.mouse',
			value: 'true',
			origin: 'system:/etc/quack/config.toml',
			description: 'Mouse capture',
			is_bool: true,
			locked: true
		};
		const { body } = render(SettingsFieldRow, {
			props: {
				field: SETTING_FIELD_MAP.get('ui.mouse')!,
				row,
				busy: false,
				onSave: () => Promise.resolve(true)
			}
		});
		expect(body).toContain('Locked');
		expect(body).toContain('Used by the terminal UI client');
		expect(body).toContain('disabled');
	});
});
