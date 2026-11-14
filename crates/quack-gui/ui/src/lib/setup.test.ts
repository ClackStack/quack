import { describe, expect, it, vi } from 'vitest';
import { render } from 'svelte/server';
import SetupPanel from './components/SetupPanel.svelte';
import { Session } from './session.svelte';
import { envVarForProvider, partitionExpertProviders, splitInlineCode } from './setup';
import type { ProviderModels, ServiceInfo } from '#lib/protocol';
import type { AppInfo, Transport } from './transport/types';

const info: AppInfo = {
	version: 'test',
	cwd: '/workspace',
	branch: null,
	model: null,
	permission_mode: 'ask',
	work_mode: 'edit',
	ui: { theme: 'auto', reduced_motion: false },
	settings: [],
	layers: [],
	rules: [],
	bundles: [],
	bundles_disabled: []
};

function makeSession(providers: ProviderModels[], services: ServiceInfo[] = []): Session {
	const transport: Transport = {
		kind: 'tauri',
		pickFolder: vi.fn(),
		recentFolders: vi.fn(),
		openFolder: vi.fn(),
		sendOp: vi.fn().mockResolvedValue(undefined),
		closeTab: vi.fn(),
		openUrl: vi.fn().mockResolvedValue(undefined),
		listen: () => () => {}
	};
	const session = new Session(transport, 1, info);
	session.providers = providers;
	session.services = services;
	return session;
}

describe('expert provider helpers', () => {
	it('extracts env var from structured key source or fallback note', () => {
		expect(
			envVarForProvider({
				id: 'anthropic',
				available: false,
				note: 'set ANTHROPIC_API_KEY to use this provider',
				bundled: true,
				models: [],
				key: { from: 'env', var: 'ANTHROPIC_API_KEY' }
			})
		).toBe('ANTHROPIC_API_KEY');

		expect(
			envVarForProvider({
				id: 'novita',
				available: false,
				note: 'set NOVITA_API_KEY to use this provider',
				bundled: true,
				models: []
			})
		).toBe('NOVITA_API_KEY');

		expect(
			envVarForProvider({
				id: 'ollama',
				available: false,
				note: 'could not reach 127.0.0.1:11434: Connection refused',
				bundled: true,
				models: []
			})
		).toBeNull();
	});

	it('partitions env providers and other unavailable providers cleanly', () => {
		const providers: ProviderModels[] = [
			{
				id: 'openai',
				available: true,
				note: '3 models',
				bundled: true,
				models: [{ id: 'gpt-6.1-sol' }],
				key: { from: 'env', var: 'OPENAI_API_KEY' }
			},
			{
				id: 'anthropic',
				available: false,
				note: 'set ANTHROPIC_API_KEY to use this provider',
				bundled: true,
				models: [],
				key: { from: 'env', var: 'ANTHROPIC_API_KEY' }
			},
			{
				id: 'deepinfra',
				available: false,
				note: '401 Unauthorized',
				bundled: true,
				models: [],
				key: { from: 'env', var: 'DEEPINFRA_API_KEY' }
			},
			{
				id: 'ollama',
				available: false,
				note: 'could not reach 127.0.0.1:11434: Connection refused',
				bundled: true,
				models: []
			},
			{
				id: 'local-keyring',
				available: true,
				note: '1 model',
				bundled: false,
				models: [{ id: 'qwen' }],
				key: { from: 'keyring', name: 'local-keyring' }
			}
		];
		const services: ServiceInfo[] = [
			{
				id: 'anthropic',
				name: 'Anthropic API',
				group: 'Proprietary models',
				base_url: 'https://api.anthropic.com',
				env: 'ANTHROPIC_API_KEY',
				ready: false,
				subscription: false
			},
			{
				id: 'openai',
				name: 'OpenAI API',
				group: 'Proprietary models',
				base_url: 'https://api.openai.com/v1',
				env: 'OPENAI_API_KEY',
				ready: true,
				subscription: false
			}
		];

		const result = partitionExpertProviders(providers, services);
		expect(result.envProviders).toEqual([
			{
				id: 'anthropic',
				name: 'Anthropic API',
				envVar: 'ANTHROPIC_API_KEY',
				available: false,
				extraNote: null
			},
			{
				id: 'deepinfra',
				name: null,
				envVar: 'DEEPINFRA_API_KEY',
				available: false,
				extraNote: '401 Unauthorized'
			},
			{
				id: 'openai',
				name: 'OpenAI API',
				envVar: 'OPENAI_API_KEY',
				available: true,
				extraNote: null
			}
		]);
		expect(result.otherUnavailable).toEqual([
			{
				id: 'ollama',
				name: null,
				note: 'could not reach 127.0.0.1:11434: Connection refused'
			}
		]);
	});

	it('splits backticked inline code spans', () => {
		expect(splitInlineCode('run `quack login chatgpt` to use this provider')).toEqual([
			{ text: 'run ', code: false },
			{ text: 'quack login chatgpt', code: true },
			{ text: ' to use this provider', code: false }
		]);
	});
});

describe('SetupPanel expert accordion', () => {
	it('renders the provider status list collapsed inside an accordion', () => {
		const session = makeSession(
			[
				{
					id: 'anthropic',
					available: false,
					note: 'set ANTHROPIC_API_KEY to use this provider',
					bundled: true,
					models: [],
					key: { from: 'env', var: 'ANTHROPIC_API_KEY' }
				},
				{
					id: 'openai',
					available: true,
					note: '2 models',
					bundled: true,
					models: [{ id: 'gpt-6.1-sol' }],
					key: { from: 'env', var: 'OPENAI_API_KEY' }
				},
				{
					id: 'chatgpt',
					available: false,
					note: 'run `quack login chatgpt` to use this provider',
					bundled: true,
					models: []
				}
			],
			[
				{
					id: 'anthropic',
					name: 'Anthropic API',
					group: 'Proprietary models',
					base_url: 'https://api.anthropic.com',
					env: 'ANTHROPIC_API_KEY',
					ready: false,
					subscription: false
				}
			]
		);

		const { body } = render(SetupPanel, { props: { session } });
		expect(body).toMatch(/<details\b[^>]*class="[^"]*\baccordion\b/);
		expect(body).not.toMatch(/<details\b[^>]*\bopen\b/);
		expect(body).toContain('Expert Setup: Environment Variables');
		expect(body).toContain('2 environment variables · 1 other unavailable');
		expect(body).toContain('ANTHROPIC_API_KEY');
		expect(body).toContain('OPENAI_API_KEY');
		expect(body).toMatch(/<code\b[^>]*>quack login chatgpt<\/code>/);
		expect(body).not.toContain('set ANTHROPIC_API_KEY to use this provider');
	});

	it('omits the expert accordion when there are no env or unavailable providers', () => {
		const session = makeSession([
			{
				id: 'dev-local',
				available: true,
				note: '1 model',
				bundled: false,
				models: [{ id: 'tiny' }]
			}
		]);

		const { body } = render(SetupPanel, { props: { session } });
		expect(body).not.toContain('Expert Setup: Environment Variables');
		expect(body).not.toMatch(/<details\b[^>]*class="[^"]*\baccordion\b/);
	});
});
