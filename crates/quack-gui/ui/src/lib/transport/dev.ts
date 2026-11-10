import type { Event, Op } from '#lib/protocol';
import type {
	AppInfo,
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

const INFO: AppInfo = {
	version: '0.1.0-dev',
	cwd: '~/Development/demo-pond',
	branch: 'main',
	model: 'dev-local/tiny',
	permission_mode: 'ask',
	ui: { theme: 'auto', reduced_motion: false },
	settings: [],
	layers: [{ layer: 'user', status: 'no config.toml', file: null }],
	rules: [],
	bundles: [],
	bundles_disabled: []
};

const SESSION = { id: 'dev-session-0000', cwd: INFO.cwd, model: INFO.model, profile: 'generic' };

export function devTransport(): Transport {
	const listeners = new Set<(message: TabMessage) => void>();
	const fan = (message: TabMessage) => {
		for (const listener of listeners) listener(message);
	};
	const send = (tab: number, event: Event) => fan({ tab, event });
	let nextTab = 0;
	let mode = 'ask';
	let resolveApproval: (() => void) | null = null;

	const catalog = () => ({
		providers: [
			{
				id: 'dev-local',
				available: true,
				note: 'scripted demo provider',
				bundled: false,
				models: [
					{ id: 'tiny', context_window: 32768, price: 'free (scripted)' },
					{ id: 'large', context_window: 131072, price: 'free (scripted)' }
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
			resolveApproval = resolve;
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

	return {
		kind: 'dev',

		async pickFolder(): Promise<string | null> {
			await sleep(150);
			return '/home/you/Development/demo-pond';
		},

		async openFolder(cwd: string, resume: ResumeRequest): Promise<OpenedTab> {
			const tab = nextTab++;
			fan({ tab, info: { ...INFO, cwd } });
			fan({
				tab,
				event: {
					event: 'session_configured',
					session: { ...SESSION, id: `${SESSION.id}-${tab}` }
				}
			});
			send(tab, { event: 'model_catalog', ...catalog() });
			send(tab, { event: 'permission_mode', mode });
			send(tab, {
				event: 'session_list',
				sessions: [
					{
						id: `${SESSION.id}-older`,
						title: 'Untangle the migration test',
						model: 'dev-local/tiny',
						turns: 6,
						age: '3h ago',
						started: '2026-11-08',
						current: false
					}
				]
			});
			send(tab, { event: 'startup_complete' });
			return { tab, info: { ...INFO, cwd } };
		},

		async sendOp(tab: number, op: Op): Promise<void> {
			switch (op.op) {
				case 'user_turn':
					void runDemoTurn(tab);
					break;
				case 'approval_response':
					resolveApproval?.();
					resolveApproval = null;
					break;
				case 'set_permission_mode':
					mode = op.mode;
					send(tab, { event: 'permission_mode', mode });
					if (mode === 'plan') {
						send(tab, {
							event: 'plan',
							checklist: '[x] read the pond\n[ ] count the ducks\n[ ] export the census'
						});
					}
					break;
				case 'set_model':
					send(tab, {
						event: 'model_catalog',
						...catalog(),
						current: `${op.provider}/${op.model}`
					});
					break;
				case 'list_sessions':
					send(tab, { event: 'session_list', sessions: [] });
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
				default:
					send(tab, { event: 'notice', message: `the dev transport ignores ${op.op}` });
			}
		},

		async closeTab(tab: number): Promise<void> {
			fan({ tab, closed: { reason: null } });
		},

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
