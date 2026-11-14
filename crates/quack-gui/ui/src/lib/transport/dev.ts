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
	type TabState = {
		cwd: string;
		branch: string;
		workMode: import('#lib/protocol').WorkMode;
		approvals: import('#lib/protocol').ApprovalPolicy;
		effort: string | null;
		resolveApproval: (() => void) | null;
	};
	const tabs = new Map<number, TabState>();
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
				resolveApproval: null
			};
			tabs.set(tab, tabState);
			const info = { ...INFO, cwd, branch };
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
			else if (jobsDemo) setTimeout(() => seedJobsDemo(tab, cwd), 120);
			return { tab, info };
		},

		async sendOp(tab: number, op: Op): Promise<void> {
			const state = tabs.get(tab);
			switch (op.op) {
				case 'user_turn':
					void runDemoTurn(tab);
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
