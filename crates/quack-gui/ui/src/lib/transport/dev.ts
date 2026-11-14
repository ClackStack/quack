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
	work_mode: 'edit',
	ui: { theme: 'auto', reduced_motion: false },
	settings: [],
	layers: [{ layer: 'user', status: 'no config.toml', file: null }],
	rules: [],
	bundles: [],
	bundles_disabled: []
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
	const send = (tab: number, event: Event) => fan({ tab, event });
	let nextTab = 0;
	let workMode: import('#lib/protocol').WorkMode = 'edit';
	let approvals: import('#lib/protocol').ApprovalPolicy = 'ask';
	let effort: string | null = null;
	let resolveApproval: (() => void) | null = null;
	// `?demo=turn` plays the demo turn on open; `&auto=1` also answers its approval,
	// so a headless capture can reach the settled state without a keyboard.
	const params =
		typeof window !== 'undefined' ? new URLSearchParams(window.location.search) : null;
	const autoDemo = params?.get('demo') === 'turn';
	const autoApprove = params?.get('auto') === '1';

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
			resolveApproval = resolve;
			if (autoApprove)
				setTimeout(() => {
					resolveApproval?.();
					resolveApproval = null;
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
			send(tab, { event: 'mode_changed', work_mode: workMode, approvals });
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
			send(tab, { event: 'session_list', sessions: savedSessions() });
			send(tab, { event: 'startup_complete' });
			if (autoDemo) setTimeout(() => void runDemoTurn(tab), 500);
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
				case 'set_work_mode':
					workMode = op.mode;
					send(tab, { event: 'mode_changed', work_mode: workMode, approvals });
					if (workMode === 'plan') {
						send(tab, {
							event: 'plan',
							checklist: '[x] read the pond\n[ ] count the ducks\n[ ] export the census'
						});
					}
					break;
				case 'set_approval_policy':
					approvals = op.policy;
					send(tab, { event: 'mode_changed', work_mode: workMode, approvals });
					break;
				case 'set_reasoning_effort':
					effort = op.effort ?? null;
					send(tab, {
						event: 'session_configured',
						session: {
							...SESSION,
						model: 'dev-local/thinker',
						reasoning_effort: effort,
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
							model,
							reasoning_effort: effort,
							reasoning_efforts: ['low', 'medium', 'high']
						}
						});
					else send(tab, { event: 'session_configured', session: { ...SESSION, model } });
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
				default:
					send(tab, { event: 'notice', message: `the dev transport ignores ${op.op}` });
			}
		},

		async closeTab(tab: number): Promise<void> {
			fan({ tab, closed: { reason: null } });
		},

		async openUrl(url: string): Promise<void> {
			// The demo runs in a plain browser; a popup is the honest stand-in for the
			// desktop shell's system browser.
			window.open(url, '_blank', 'noopener');
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
