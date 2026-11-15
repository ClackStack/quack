import { describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import ExtensionsSettings from './components/ExtensionsSettings.svelte';
import {
	addBlankConnection,
	applyPresetToDraft,
	buildMcpSecretMap,
	connectionCheckUrl,
	createDraftsFromSetup,
	deriveConnectionFields,
	fieldChoices,
	formatKeySource,
	isFieldRequired,
	isFieldVisible,
	mergeMcpServers,
	parseArgsString,
	sortHooks
} from './extensions';
import type { HookSetupInfo } from './protocol';
import { Session } from './session.svelte';
import type { AppInfo, Transport } from './transport/types';

function makeInfo(): AppInfo {
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
			egress_allow: [],
			providers: [{ id: 'openai', host: 'api.openai.com', local: false, enabled: true }],
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
			status_detail: 'active',
			writable: [],
			locked: false
		},
		settings: [],
		layers: [],
		rules: [],
		bundles: [],
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
				description: 'Fetch Jira tickets referenced in prompts',
				event: 'user_prompt',
				command: ['node', 'jira.mjs'],
				matcher_tool: '',
				matcher_pattern: '',
				matcher_prompt: '[A-Z]+-\\d+',
				timeout_secs: 15,
				on_error: 'warn',
				full_context: false,
				connections_count: 1
			},
			{
				id: 'org-policy',
				layer: 'system',
				file: '/etc/quack/hooks.d/org-policy.toml',
				state: 'enabled',
				detail: 'on',
				enabled: true,
				can_toggle: false,
				has_setup: false,
				description: 'Enforced pre-tool audit hook',
				event: 'pre_tool',
				command: ['/usr/local/bin/org-audit'],
				matcher_tool: 'shell',
				matcher_pattern: '',
				matcher_prompt: '',
				timeout_secs: 5,
				on_error: 'deny',
				full_context: false,
				connections_count: 0
			}
		],
		hooks_disabled: [],
		mcp: [
			{
				id: 'filesystem-demo',
				layer: 'user',
				file: '/home/paul/.config/quack/mcp.d/filesystem-demo.toml',
				state: 'enabled',
				detail: 'enabled',
				enabled: true,
				can_toggle: true,
				blocked_by_policy: false,
				transport: 'stdio',
				target: 'npx -y @modelcontextprotocol/server-filesystem .',
				description: 'Read project files over MCP',
				sandbox: true,
				startup_timeout_secs: 20,
				call_timeout_secs: 120,
				max_output_tokens: 8000,
				auto_approve: ['read_file'],
				env_keys: ['ROOT_DIR'],
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
				description: 'GitHub repositories & issues',
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
		history: []
	};
}

function makeSetupInfo(): HookSetupInfo {
	return {
		id: 'jira',
		revision: 'rev-42',
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
					help: 'Base Jira site URL',
					choices: [],
					advanced: false,
					credential_scope: true,
					required_when: {}
				},
				{
					id: 'auth_mode',
					label: 'Auth mode',
					kind: 'choice',
					required: true,
					default: 'api_token',
					help: 'Authentication mode',
					choices: ['api_token', 'bearer'],
					advanced: false,
					credential_scope: false,
					required_when: {}
				},
				{
					id: 'email',
					label: 'Email',
					kind: 'text',
					required: false,
					default: '',
					help: 'Required for Cloud API token',
					choices: [],
					advanced: false,
					credential_scope: false,
					required_when: { auth_mode: ['api_token'] }
				},
				{
					id: 'token',
					label: 'API token',
					kind: 'secret',
					required: true,
					default: '',
					help: 'Stored in keyring',
					choices: [],
					advanced: false,
					credential_scope: false,
					required_when: {}
				},
				{
					id: 'api_url',
					label: 'API URL',
					kind: 'url',
					required: false,
					default: '',
					help: 'Derived endpoint URL',
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
					help: 'Atlassian Cloud',
					values: { web_url: 'https://company.atlassian.net', auth_mode: 'api_token' },
					choices: { auth_mode: ['api_token'] },
					derived: { api_url: '{web_url}/rest/api/3' }
				},
				{
					id: 'datacenter',
					label: 'Data Center',
					help: 'Self-hosted Jira',
					values: { web_url: 'https://jira.internal', auth_mode: 'bearer' },
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
						web_url: 'https://acme.atlassian.net',
						auth_mode: 'api_token',
						email: 'dev@acme.example',
						api_url: 'https://acme.atlassian.net/rest/api/3'
					}
				},
				credentials: {
					token: { from: 'keyring', name: 'quack:jira:default' }
				}
			}
		],
		enabled: false,
		source: 'defaults',
		writable: true,
		problem: null
	};
}

describe('hooks and MCP pure helpers', () => {
	it('merges static MCP config rows with live runtime capabilities', () => {
		const info = makeInfo();
		const merged = mergeMcpServers(info.mcp ?? [], [
			{
				id: 'filesystem-demo',
				transport: 'stdio',
				target: 'npx -y @modelcontextprotocol/server-filesystem .',
				description: 'Read project files over MCP',
				state: 'ready',
				tools: ['read_file', 'list_directory'],
				prompts: ['summarize'],
				resources: ['file://README.md'],
				deferred: false,
				tool_tokens: 380,
				log: ['server ready']
			}
		]);
		expect(merged.map((s) => s.id)).toEqual(['filesystem-demo', 'github']);
		expect(merged[0].state).toBe('ready');
		expect(merged[0].tools).toEqual(['read_file', 'list_directory']);
		expect(merged[0].toolTokens).toBe(380);
		expect(merged[1].state).toBe('disabled');
		expect(merged[1].canToggle).toBe(true);
	});

	it('splits quoted args and builds McpSecretValue bindings cleanly', () => {
		expect(parseArgsString(`-y @scope/pkg --root "/tmp/my dir" 'plain val'`)).toEqual([
			'-y',
			'@scope/pkg',
			'--root',
			'/tmp/my dir',
			'plain val'
		]);
		expect(
			buildMcpSecretMap([
				{ id: 1, key: 'Authorization', mode: 'env', value: 'API_KEY', format: 'Bearer {}' },
				{ id: 2, key: 'TOKEN', mode: 'keyring', value: 'my-secret', format: '' },
				{ id: 3, key: 'MODE', mode: 'plain', value: 'strict', format: '' },
				{ id: 4, key: '', mode: 'env', value: 'IGNORED', format: '' }
			])
		).toEqual({
			Authorization: { kind: 'env', env: 'API_KEY', format: 'Bearer {}' },
			TOKEN: { kind: 'keyring', keyring: 'my-secret' },
			MODE: { kind: 'plain', value: 'strict' }
		});
	});

	it('manages declarative hook drafts, presets, derived URLs, required_when, and visibility', () => {
		const setup = makeSetupInfo();
		const drafts = createDraftsFromSetup(setup);
		expect(drafts).toHaveLength(1);
		expect(drafts[0].credentials.token).toEqual({ action: 'keep' });

		const switched = applyPresetToDraft(drafts[0], setup.descriptor, 'datacenter');
		expect(switched.connection.preset).toBe('datacenter');
		expect(switched.connection.values.auth_mode).toBe('bearer');
		expect(switched.connection.values.api_url).toBe('https://acme.atlassian.net/rest/api/2');

		const updated = deriveConnectionFields(
			{
				...switched,
				connection: {
					...switched.connection,
					values: { ...switched.connection.values, web_url: 'https://jira.example.com/' }
				}
			},
			setup.descriptor
		);
		expect(updated.connection.values.api_url).toBe('https://jira.example.com/rest/api/2');
		expect(connectionCheckUrl(updated, setup.descriptor)).toBe('https://jira.example.com/');

		const blank = addBlankConnection(setup.descriptor, drafts);
		expect(blank.connection.id).toBe('default' === drafts[0].connection.id ? 'connection-2' : 'default');
		expect(blank.connection.preset).toBe('cloud');

		const emailField = setup.descriptor.fields.find((f) => f.id === 'email')!;
		const apiUrlField = setup.descriptor.fields.find((f) => f.id === 'api_url')!;
		const authField = setup.descriptor.fields.find((f) => f.id === 'auth_mode')!;
		expect(isFieldRequired(emailField, { auth_mode: 'api_token' })).toBe(true);
		expect(isFieldRequired(emailField, { auth_mode: 'bearer' })).toBe(false);
		expect(isFieldVisible(apiUrlField, { api_url: '' }, false)).toBe(false);
		expect(isFieldVisible(apiUrlField, { api_url: '' }, true)).toBe(true);
		expect(fieldChoices(authField, setup.descriptor.presets[0])).toEqual(['api_token']);
		expect(formatKeySource({ from: 'keyring', name: 'quack:jira:default' })).toBe(
			'OS keyring (quack:jira:default)'
		);
	});

	it('sorts enabled hooks ahead of disabled hooks', () => {
		const sorted = sortHooks(makeInfo().hooks ?? []);
		expect(sorted.map((h) => h.id)).toEqual(['org-policy', 'jira']);
	});
});

describe('ExtensionsSettings SSR rendering', () => {
	it('renders MCP tools, Restart/Enable buttons, Hook list, declarative hook setup, and recent runs', () => {
		const transport: Transport = {
			kind: 'tauri',
			pickFolder: vi.fn().mockResolvedValue(null),
			recentFolders: vi.fn(),
			openFolder: vi.fn(),
			sendOp: vi.fn().mockResolvedValue(undefined),
			closeTab: vi.fn(),
			openUrl: vi.fn(),
			listen: () => () => {}
		};
		const session = new Session(transport, 1, makeInfo());
		session.mcpServers = [
			{
				id: 'filesystem-demo',
				transport: 'stdio',
				target: 'npx -y @modelcontextprotocol/server-filesystem .',
				description: 'Read project files over MCP',
				state: 'ready',
				tools: ['read_file', 'list_directory'],
				prompts: [],
				resources: [],
				deferred: false,
				tool_tokens: 380,
				log: []
			}
		];
		session.hookSetup = makeSetupInfo();
		session.hookRuns = [
			{
				id: 'org-policy',
				on: 'pre_tool',
				outcome: 'allowed',
				ms: 14,
				message: 'command allowed',
				contextTokens: 0,
				at: 1000
			}
		];

		const { body } = render(ExtensionsSettings, {
			props: { session, writeScope: 'user', busy: false }
		});
		expect(body).toContain('MCP servers (2)');
		expect(body).toContain('filesystem-demo');
		expect(body).toContain('read_file');
		expect(body).toContain('Restart');
		expect(body).toContain('github');
		expect(body).toContain('Enable');
		expect(body).toContain('Lifecycle hooks (2)');
		expect(body).toContain('Enforced');
		expect(body).toContain('Configure hook:');
		expect(body).toContain('Jira Cloud');
		expect(body).toContain('https://acme.atlassian.net');
		expect(body).toContain('Recent hook runs');
		expect(body).toContain('command allowed');
	});
});
