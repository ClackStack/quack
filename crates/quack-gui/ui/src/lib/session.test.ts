import { describe, expect, it, vi } from 'vitest';
import { Session } from './session.svelte';
import { devTransport } from './transport/dev';
import type { AppInfo, TabMessage, Transport } from './transport/types';

const info: AppInfo = {
	version: 'test',
	cwd: '/workspace',
	branch: null,
	model: 'mock/org/model',
	planner_model: null,
	planner_locked: false,
	permission_mode: 'ask',
	work_mode: 'edit',
	ui: { theme: 'auto', reduced_motion: false },
	egress: {
		offline: false,
		offline_locked: false,
		egress_allow: [],
		providers: [],
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
	settings: [],
	layers: [],
	rules: [],
	bundles: [],
	bundles_disabled: [],
	history: []
};
const sessionInfo = {
	id: 'original',
	cwd: '/workspace',
	model: info.model ?? undefined,
	profile: 'generic'
};
function fixture(model: string | null = info.model) {
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
	const session = new Session(transport, 3, { ...info, model });
	session.onEvent({ event: 'session_configured', session: { ...sessionInfo, model: model ?? undefined } });
	return {
		session,
		send: vi.mocked(transport.sendOp),
		openUrl: vi.mocked(transport.openUrl)
	};
}

describe('setup and project trust', () => {
	it('opens setup only after startup confirms no model is ready', () => {
		const { session, send } = fixture(null);
		expect(session.setupOpen).toBe(false);
		session.onEvent({ event: 'startup_complete' });
		expect(session.setupOpen).toBe(true);
		expect(send).toHaveBeenCalledWith(3, { op: 'list_services' });
		expect(send).not.toHaveBeenCalledWith(3, { op: 'detect_servers' });
		expect(session.canSubmit).toBe(false);
	});
	it('blocks turns during trust and retains the gate after a failed decision', async () => {
		const { session, send } = fixture();
		session.onEvent({ event: 'startup_complete' });
		session.onEvent({
			event: 'project_trust',
			path: '/workspace/.quack',
			summary: '1 server',
			changed: true
		});
		expect(await session.submit('hello')).toBe(false);
		send.mockRejectedValueOnce(new Error('Could not write trust file'));
		expect(await session.trustProject('settings-only')).toBe(false);
		expect(session.trust?.changed).toBe(true);
		expect(session.trustSaving).toBe(false);
		expect(session.actionError).toContain('try again');
		await session.trustProject('settings-only');
		expect(session.trust).not.toBeNull();
		session.onEvent({ event: 'config_reloaded' });
		expect(session.trust).toBeNull();
		expect(session.canSubmit).toBe(false);
		session.onEvent({ event: 'startup_complete' });
		expect(session.canSubmit).toBe(true);
	});
	it('waits for model acknowledgement and merges provider-only catalogs', async () => {
		const { session } = fixture(null);
		session.onEvent({ event: 'startup_complete' });
		const provider = (id: string) => ({
			id,
			available: true,
			note: '',
			bundled: false,
			models: [{ id: 'org/model' }]
		});
		session.onEvent({ event: 'model_catalog', providers: [provider('first')], current: undefined });
		await session.saveProviderKey('second', 'test-key');
		expect(session.setupWorking).toBe(true);
		session.onEvent({ event: 'provider_added', id: 'second' });
		session.onEvent({ event: 'model_catalog', providers: [provider('second')], current: undefined });
		expect(session.providers.map((p) => p.id)).toEqual(['first', 'second']);
		await session.setModel('second/org/model');
		expect(session.setupOpen).toBe(true);
		expect(session.currentModel).toBeNull();
		session.onEvent({
			event: 'session_configured',
			session: { ...sessionInfo, model: 'second/org/model' }
		});
		expect(session.setupOpen).toBe(false);
		expect(session.setupWorking).toBe(false);
		expect(session.currentModel).toBe('second/org/model');
	});
	it('keeps the acknowledged model when an older catalog request finishes later', () => {
		const { session } = fixture();
		session.onEvent({
			event: 'session_configured',
			session: { ...sessionInfo, model: 'new/model' }
		});
		session.onEvent({ event: 'model_catalog', providers: [], current: 'old/model' });
		expect(session.currentModel).toBe('new/model');
	});

	it('forwards server settings without hardcoded providers and recovers from engine errors', async () => {
		const { session, send } = fixture(null);
		await session.connectServer(' https://example.test/v1 ', 'test-key');
		expect(send).toHaveBeenCalledWith(3, {
			op: 'add_server',
			base_url: 'https://example.test/v1',
			key: 'test-key'
		});
		session.onEvent({
			event: 'error',
			error: { kind: 'config', message: 'Endpoint is unavailable' }
		});
		expect(session.setupWorking).toBe(false);
		expect(session.actionError).toBe('Endpoint is unavailable');
	});
});

describe('action recovery', () => {
	it('retains the draft and removes an unsent turn on IPC failure', async () => {
		const { session, send } = fixture();
		session.onEvent({ event: 'startup_complete' });
		session.draft = 'Please fix this';
		send.mockRejectedValueOnce(new Error('Channel closed'));
		expect(await session.submit(session.draft)).toBe(false);
		expect(session.draft).toBe('Please fix this');
		expect(session.items).toEqual([]);
		expect(session.canSubmit).toBe(true);
		expect(session.actionError).toContain('Channel closed');
	});
	it('retains approvals and questions on failure so they can be retried', async () => {
		const { session, send } = fixture();
		session.approval = {
			callId: 'a',
			request: { kind: 'shell', cwd: '/workspace', command: 'pwd', outside_sandbox: false },
			always: []
		};
		send.mockRejectedValueOnce(new Error('Offline'));
		expect(await session.answerApproval('allow_once')).toBe(false);
		expect(session.approval?.callId).toBe('a');
		await session.answerApproval('deny');
		expect(session.approval).toBeNull();
		session.questions = { callId: 'q', questions: [] };
		send.mockRejectedValueOnce(new Error('Offline'));
		expect(await session.answerQuestions([{ text: 'Keep it' }])).toBe(false);
		expect(session.questions?.callId).toBe('q');
		await session.answerQuestions([{ text: 'Keep it' }]);
		expect(session.questions).toBeNull();
	});
	it('does not clear a newer approval delivered before the previous send completes', async () => {
		const { session, send } = fixture();
		session.approval = {
			callId: 'a',
			request: { kind: 'shell', cwd: '/workspace', command: 'pwd', outside_sandbox: false },
			always: []
		};
		send.mockImplementationOnce(async () => {
			session.onEvent({
				event: 'approval_request',
				call_id: 'b',
				request: { kind: 'shell', cwd: '/workspace', command: 'ls', outside_sandbox: false }
			});
		});
		await session.answerApproval('allow_once');
		expect(session.approval?.callId).toBe('b');
	});
	it('blocks duplicate submissions while awaiting IPC and while a turn is running', async () => {
		const { session, send } = fixture();
		session.onEvent({ event: 'startup_complete' });
		let resolve!: () => void;
		send.mockImplementationOnce(
			() =>
				new Promise<void>((done) => {
					resolve = done;
				})
		);
		const pending = session.submit('First');
		expect(await session.submit('Second')).toBe(false);
		resolve();
		await pending;
		session.onEvent({ event: 'turn_started', turn: 1 });
		expect(await session.submit('Third')).toBe(false);
		expect(send).toHaveBeenCalledTimes(1);
	});
	it('keeps the transcript until New is acknowledged with a new session id', async () => {
		const { session } = fixture();
		session.onEvent({ event: 'startup_complete' });
		session.onEvent({ event: 'assistant_delta', text: 'Existing response' });
		await session.newSession();
		expect(session.items[0]).toMatchObject({ text: 'Existing response' });
		expect(session.switching).toBe(true);
		session.onEvent({ event: 'session_configured', session: { ...sessionInfo, id: 'new' } });
		expect(session.items).toEqual([]);
		expect(session.switching).toBe(false);
	});
	it('retains the transcript for failed resume and replaces it on replay', async () => {
		const { session } = fixture();
		session.onEvent({ event: 'startup_complete' });
		session.onEvent({ event: 'assistant_delta', text: 'Existing response' });
		await session.resumeSession('missing');
		session.onEvent({ event: 'warning', message: 'Cannot find that session' });
		expect(session.items[0]).toMatchObject({ text: 'Existing response' });
		expect(session.switching).toBe(false);
		await session.resumeSession('saved');
		session.onEvent({
			event: 'session_resumed',
			session: { ...sessionInfo, id: 'saved' },
			items: [{ kind: 'assistant', text: 'Saved response' }]
		});
		session.onEvent({ event: 'session_configured', session: { ...sessionInfo, id: 'saved' } });
		expect(session.items).toHaveLength(1);
		expect(session.items[0]).toMatchObject({ text: 'Saved response' });
	});
});

describe('work mode and approvals', () => {
	it('tracks both controls from the authoritative mode_changed event', () => {
		const { session } = fixture();
		expect(session.workMode).toBe('edit');
		expect(session.approvals).toBe('ask');
		session.onEvent({ event: 'mode_changed', work_mode: 'plan', approvals: 'ask' });
		expect(session.workMode).toBe('plan');
		expect(session.approvals).toBe('ask');
		session.onEvent({ event: 'mode_changed', work_mode: 'plan', approvals: 'allow' });
		expect(session.workMode).toBe('plan');
		expect(session.approvals).toBe('allow');
	});
	it('sends the typed ops for each control and keeps plan on accept', async () => {
		const { session, send } = fixture();
		await session.setWorkMode('plan');
		expect(send).toHaveBeenCalledWith(3, { op: 'set_work_mode', mode: 'plan' });
		await session.setApprovals('auto-edit');
		expect(send).toHaveBeenCalledWith(3, { op: 'set_approval_policy', policy: 'auto-edit' });
		await session.acceptPlan();
		expect(send).toHaveBeenCalledWith(3, { op: 'accept_plan', mode: '' });
	});
	it('maps the legacy permission_mode notification onto both controls', () => {
		const { session } = fixture();
		session.onEvent({ event: 'permission_mode', mode: 'plan' });
		expect(session.workMode).toBe('plan');
		session.onEvent({ event: 'permission_mode', mode: 'read-only' });
		expect(session.workMode).toBe('edit');
		expect(session.approvals).toBe('read-only');
	});
	it('only marks planReviewReady for unfinished follow-up plans, not finished or executed ones', async () => {
		const { session } = fixture();
		const usage = { prompt_tokens: 10, completion_tokens: 5 };
		session.onEvent({ event: 'mode_changed', work_mode: 'plan', approvals: 'ask' });

		// Initial planning turn with pending steps opens review.
		session.onEvent({ event: 'turn_started', turn: 1 });
		session.onEvent({ event: 'plan', checklist: '[ ] step 1\n[ ] step 2' });
		session.onEvent({ event: 'turn_complete', turn: 1, usage });
		expect(session.planReviewReady).toBe(true);

		// Accepting and executing the plan to completion clears review.
		await session.acceptPlan();
		expect(session.planReviewReady).toBe(false);
		session.onEvent({ event: 'mode_changed', work_mode: 'edit', approvals: 'ask' });
		session.onEvent({ event: 'turn_started', turn: 2 });
		session.onEvent({ event: 'plan', checklist: '[x] step 1\n[x] step 2' });
		session.onEvent({ event: 'turn_complete', turn: 2, usage });
		expect(session.planReviewReady).toBe(false);

		// Switching back to Plan mode with a finished plan does not show review.
		session.onEvent({ event: 'mode_changed', work_mode: 'plan', approvals: 'ask' });
		expect(session.planReviewReady).toBe(false);

		// Follow-up check turn with no new pending steps does not show review.
		session.onEvent({ event: 'turn_started', turn: 3 });
		session.onEvent({ event: 'turn_complete', turn: 3, usage });
		expect(session.planReviewReady).toBe(false);

		// Explicitly clearing the plan in a planning turn does not show review.
		session.onEvent({ event: 'turn_started', turn: 4 });
		session.onEvent({ event: 'plan', checklist: '' });
		session.onEvent({ event: 'turn_complete', turn: 4, usage });
		expect(session.planReviewReady).toBe(false);

		// Creating a follow-up plan in Plan mode opens review again.
		session.onEvent({ event: 'turn_started', turn: 5 });
		session.onEvent({ event: 'plan', checklist: '[ ] follow-up step' });
		session.onEvent({ event: 'turn_complete', turn: 5, usage });
		expect(session.planReviewReady).toBe(true);

		// Manual /compact in Plan mode does not reopen review after submitting/dismissing.
		session.onEvent({ event: 'turn_started', turn: 6 });
		session.onEvent({ event: 'compacting', reason: '/compact' });
		session.onEvent({ event: 'turn_complete', turn: 6, usage });
		expect(session.planReviewReady).toBe(false);
	});
	it('does not reopen planReviewReady when switching to plan mid-edit turn with stale steps', async () => {
		const { session } = fixture();
		const usage = { prompt_tokens: 10, completion_tokens: 5 };
		session.onEvent({ event: 'mode_changed', work_mode: 'plan', approvals: 'allow' });
		session.onEvent({ event: 'turn_started', turn: 1 });
		session.onEvent({ event: 'plan', checklist: '[ ] edit file\n[ ] run tests' });
		session.onEvent({ event: 'turn_complete', turn: 1, usage });
		expect(session.planReviewReady).toBe(true);

		await session.acceptPlan();
		session.onEvent({ event: 'mode_changed', work_mode: 'edit', approvals: 'allow' });
		session.onEvent({ event: 'turn_started', turn: 2 });
		session.onEvent({ event: 'plan', checklist: '[x] edit file\n[~] run tests' });
		session.onEvent({ event: 'mode_changed', work_mode: 'plan', approvals: 'allow' });
		session.onEvent({ event: 'turn_complete', turn: 2, usage });
		expect(session.planReviewReady).toBe(false);

		// Even a subsequent Plan turn that leaves the stale checklist unchanged stays false.
		session.onEvent({ event: 'turn_started', turn: 3 });
		session.onEvent({ event: 'turn_complete', turn: 3, usage });
		expect(session.planReviewReady).toBe(false);
	});
});

describe('model thinking levels', () => {
	it('asks for a level when the switched-to model declares them', async () => {
		const { session, send } = fixture();
		session.onEvent({
			event: 'model_catalog',
			providers: [
				{
					id: 'mock',
					available: true,
					note: '',
					bundled: false,
					models: [
						{ id: 'org/model' },
						{ id: 'org/thinker', reasoning_efforts: ['low', 'high'] }
					]
				}
			],
			current: 'mock/org/model'
		});
		await session.setModel('mock/org/thinker');
		expect(send).toHaveBeenCalledWith(3, { op: 'set_model', provider: 'mock', model: 'org/thinker' });
		expect(session.thinkingOpen).toBe(false);
		session.onEvent({
			event: 'session_configured',
			session: {
				...sessionInfo,
				model: 'mock/org/thinker',
				reasoning_effort: null,
				reasoning_efforts: ['low', 'high']
			}
		});
		expect(session.thinkingOpen).toBe(true);
		expect(session.reasoningEfforts).toEqual(['low', 'high']);
		await session.setReasoningEffort('high');
		expect(send).toHaveBeenCalledWith(3, { op: 'set_reasoning_effort', effort: 'high' });
		expect(session.thinkingOpen).toBe(false);
		await session.setReasoningEffort(null);
		expect(send).toHaveBeenCalledWith(3, { op: 'set_reasoning_effort', effort: undefined });
	});
	it('does not ask for a level when the model declares none', () => {
		const { session } = fixture();
		session.onEvent({
			event: 'session_configured',
			session: { ...sessionInfo, model: 'mock/org/model' }
		});
		expect(session.thinkingOpen).toBe(false);
		expect(session.reasoningEfforts).toEqual([]);
	});
});

describe('subscription sign-in', () => {
	it('opens the browser address, then refreshes services on success', async () => {
		const { session, send, openUrl } = fixture();
		await session.signIn('charm');
		expect(send).toHaveBeenCalledWith(3, { op: 'sign_in', id: 'charm' });
		session.onEvent({
			event: 'sign_in_url',
			id: 'charm',
			url: 'https://charm.example/oauth'
		});
		expect(session.signingIn).toEqual({ id: 'charm', url: 'https://charm.example/oauth' });
		expect(openUrl).toHaveBeenCalledWith('https://charm.example/oauth');
		session.onEvent({ event: 'sign_in_result', id: 'charm', error: undefined });
		expect(session.signingIn).toBeNull();
		expect(session.setupNotice).toContain('Signed in to charm');
		expect(send).toHaveBeenCalledWith(3, { op: 'list_services' });
		expect(send).toHaveBeenCalledWith(3, { op: 'list_models' });
	});
	it('keeps the error beside the service and can retry', async () => {
		const { session } = fixture();
		await session.signIn('charm');
		session.onEvent({
			event: 'sign_in_result',
			id: 'charm',
			error: 'browser timed out'
		});
		expect(session.signInError).toEqual({ id: 'charm', message: 'browser timed out' });
		expect(session.signingIn).toBeNull();
		await session.signIn('charm');
		expect(session.signInError).toBeNull();
	});
	it('ignores results for services it is no longer signing in to', () => {
		const { session } = fixture();
		session.onEvent({ event: 'sign_in_result', id: 'other', error: undefined });
		expect(session.setupNotice).toBe('');
	});
	it('clears the pending sign-in locally on cancel', async () => {
		const { session, send } = fixture();
		await session.signIn('charm');
		session.onEvent({ event: 'sign_in_url', id: 'charm', url: 'https://charm.example' });
		await session.signIn(null);
		expect(send).toHaveBeenCalledWith(3, { op: 'sign_in', id: undefined });
		expect(session.signingIn).toBeNull();
	});
});

describe('field notes: skills, stats and specimens', () => {
	it('keeps the skills the engine reports', () => {
		const { session } = fixture();
		session.onEvent({
			event: 'skills',
			skills: [{ name: 'pond-census', description: 'Count residents.', source: 'demo', dir: '/d' }]
		});
		expect(session.skills).toHaveLength(1);
		expect(session.skills[0].name).toBe('pond-census');
	});

	it('collects edited files from edit and write calls, keeping the opening subject', () => {
		const { session } = fixture();
		session.onEvent({
			event: 'tool_call_begin',
			call: { call_id: 'a', name: 'edit', summary: 'edit src/one.ts' }
		});
		session.onEvent({
			event: 'tool_call_end',
			call_id: 'a',
			result: {
				ok: true,
				summary: 'edited 1 place',
				detail: '--- a/src/one.ts\n+++ b/src/one.ts\n@@\n-old\n+new\n+newer'
			}
		});
		session.onEvent({
			event: 'tool_call_begin',
			call: { call_id: 'b', name: 'edit', summary: 'edit 2 files' }
		});
		session.onEvent({
			event: 'tool_call_end',
			call_id: 'b',
			result: {
				ok: true,
				summary: 'edit 2 files',
				detail: '--- a/src/two.ts\n+++ b/src/two.ts\n@@\n+a\n--- a/src/three.rs\n+++ b/src/three.rs\n@@\n-b'
			}
		});
		const files = session.editedFiles;
		expect(files.map((f) => f.path)).toEqual(['src/one.ts', 'src/two.ts', 'src/three.rs']);
		expect(files[0]).toMatchObject({ adds: 2, dels: 1, ops: 1 });
		expect(files[1]).toMatchObject({ adds: 1, dels: 0 });
		expect(files[2]).toMatchObject({ adds: 0, dels: 1 });
	});

	it('re-reads the usage and context after a turn completes', () => {
		const { session, send } = fixture();
		session.onEvent({ event: 'turn_started', turn: 1 });
		session.onEvent({
			event: 'turn_complete',
			turn: 1,
			usage: { prompt_tokens: 100, completion_tokens: 20 }
		});
		expect(send).toHaveBeenCalledWith(3, { op: 'get_stats' });
		expect(send).toHaveBeenCalledWith(3, { op: 'get_context' });
	});

	it('holds the stats report the engine answers with', () => {
		const { session } = fixture();
		session.onEvent({
			event: 'stats',
			report: {
				model: 'mock/org/model',
				turns: 1,
				requests: 2,
				unreported: 0,
				prompt_tokens: 400,
				completion_tokens: 50,
				cached_tokens: 300,
				cache_reported: 2,
				unpriced: 2,
				open_ms: 1000,
				model_ms: 500,
				tools_ms: 100,
				approval_ms: 0,
				tools: [],
				compactions: 0,
				pruned: 0,
				resumed: false
			}
		});
		expect(session.stats?.turns).toBe(1);
	});

	it('tracks startup progress, transient retry warnings, persistent errors, and close reasons', () => {
		const { session } = fixture();
		session.onEvent({
			event: 'startup_progress',
			subsystem: 'mcp',
			state: { state: 'running', detail: 'connecting filesystem' }
		});
		expect(session.startupDetail).toBe('connecting filesystem');
		session.onEvent({ event: 'startup_complete' });
		expect(session.startupDetail).toBeNull();

		// Retry warning followed by progress clears automatically when turn finishes.
		session.onEvent({ event: 'turn_started', turn: 1 });
		session.onEvent({ event: 'warning', message: 'Transient 503, retrying…' });
		expect(session.issue).toEqual({ kind: 'warning', message: 'Transient 503, retrying…' });
		session.onEvent({ event: 'assistant_delta', text: 'Recovered!' });
		session.onEvent({
			event: 'turn_complete',
			turn: 1,
			usage: { prompt_tokens: 50, completion_tokens: 10 }
		});
		expect(session.issue).toBeNull();

		// A background warning mid-stream keeps the open assistant message unsplit and streaming.
		session.onEvent({ event: 'turn_started', turn: 2 });
		session.onEvent({ event: 'assistant_delta', text: 'Hello ' });
		session.onEvent({ event: 'warning', message: 'Background note' });
		session.onEvent({ event: 'assistant_delta', text: 'world!' });
		const assistantItems = session.items.filter((i) => i.kind === 'assistant');
		expect(assistantItems.at(-1)).toMatchObject({ text: 'Hello world!', streaming: true });
		session.onEvent({
			event: 'turn_complete',
			turn: 2,
			usage: { prompt_tokens: 60, completion_tokens: 12 }
		});
		expect(session.items.filter((i) => i.kind === 'assistant').at(-1)).toMatchObject({
			text: 'Hello world!',
			streaming: false
		});

		// Error stays more severe than subsequent turn_aborted and clears via dismissIssue().
		session.onEvent({ event: 'turn_started', turn: 3 });
		session.onEvent({ event: 'error', error: { kind: 'provider', message: 'Quota exceeded' } });
		session.onEvent({ event: 'turn_aborted', turn: 3, reason: 'Quota exceeded' });
		expect(session.issue).toEqual({ kind: 'error', message: 'Quota exceeded' });
		session.dismissIssue();
		expect(session.issue).toBeNull();
		expect(session.actionError).toBeNull();

		session.onClosed('Process terminated with exit code 1');
		expect(session.closed).toBe(true);
		expect(session.closedReason).toBe('Process terminated with exit code 1');
	});

	it('tracks memory notes, handles save/delete errors, and persists input history across sessions', async () => {
		const sent: unknown[] = [];
		const externalHistory = ['from tui 1', 'from tui 2'];
		const initialSession = new Session(
			{
				kind: 'tauri',
				pickFolder: vi.fn(),
				openFolder: vi.fn(),
				sendOp: vi.fn(async (_tab, op) => {
					sent.push(op);
				}),
				closeTab: vi.fn(),
				recentFolders: vi.fn(),
				openUrl: vi.fn(),
				listen: () => () => {},
				inputHistory: vi.fn(async () => [...externalHistory, 'later tui'])
			},
			2,
			{ ...info, history: externalHistory }
		);
		expect(initialSession.inputHistory).toEqual(externalHistory);

		initialSession.onEvent({ event: 'startup_complete' });
		await initialSession.listMemory();
		expect(sent.at(-1)).toEqual({ op: 'list_memory' });

		await initialSession.saveMemory({
			scope: 'project',
			name: 'style',
			description: 'Style notes',
			kind: 'project',
			body: 'Use snake_case'
		});
		expect(initialSession.memorySaving).toBe(true);
		expect(sent.at(-1)).toEqual({
			op: 'save_memory',
			scope: 'project',
			name: 'style',
			description: 'Style notes',
			kind: 'project',
			body: 'Use snake_case'
		});

		// A warning while saving clears memorySaving and surfaces memoryError.
		initialSession.onEvent({ event: 'warning', message: 'cannot save note: duplicate' });
		expect(initialSession.memorySaving).toBe(false);
		expect(initialSession.memoryError).toBe('cannot save note: duplicate');

		// Receiving memory_notes populates notes and clears errors.
		const sample = {
			scope: 'project',
			name: 'style',
			description: 'Style notes',
			path: '/workspace/.quack/memory/notes/style.md',
			text: '---\nname: style\ndescription: Style notes\ntype: project\n---\n\nUse snake_case\n'
		};
		initialSession.onEvent({ event: 'memory_notes', notes: [sample], index_fill: 12 });
		expect(initialSession.memoryLoaded).toBe(true);
		expect(initialSession.memoryIndexFill).toBe(12);
		expect(initialSession.memoryNotes).toEqual([sample]);
		expect(initialSession.memoryError).toBeNull();

		// Submit adds prompt to inputHistory and keeps it across new_session.
		expect(await initialSession.submit('from gui')).toBe(true);
		expect(initialSession.inputHistory).toEqual(['from tui 1', 'from tui 2', 'from gui']);

		initialSession.onEvent({
			event: 'session_configured',
			session: { ...sessionInfo, id: 'fresh-session' }
		});
		expect(initialSession.inputHistory).toEqual(['from tui 1', 'from tui 2', 'from gui']);
		expect(initialSession.memoryNotes).toEqual([sample]);

		expect(await initialSession.refreshInputHistory()).toBe(true);
		expect(initialSession.inputHistory).toEqual([...externalHistory, 'later tui']);
	});

	it('handles step limits, feedback denials, scoped always-allow, rewind conflicts, and fork', async () => {
		const { session, send } = fixture();
		session.onEvent({ event: 'startup_complete' });

		// Submit injects unmentioned attached files into InputItem::Text without duplicating `@path` already present.
		expect(
			await session.submit('Review @src/main.rs please', ['src/main.rs', 'src/lib.rs'], ['/tmp/img.png'])
		).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'user_turn',
			items: [
				{ type: 'text', text: 'Review @src/main.rs please\n\n@src/lib.rs' },
				{ type: 'image', path: '/tmp/img.png' }
			],
			overrides: {}
		});

		// Approval: feedback denial + project-scoped AlwaysAllow.
		session.onEvent({
			event: 'approval_request',
			call_id: 'call-1',
			request: { kind: 'shell', command: 'cargo test', cwd: '/workspace', outside_sandbox: false },
			always: ['cargo test']
		});
		expect(await session.answerApproval('deny', 'Run clippy first')).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'approval_response',
			call_id: 'call-1',
			decision: { decision: 'deny_with_feedback', feedback: 'Run clippy first' }
		});

		session.onEvent({
			event: 'approval_request',
			call_id: 'call-2',
			request: { kind: 'shell', command: 'cargo clippy', cwd: '/workspace', outside_sandbox: false },
			always: ['cargo clippy']
		});
		expect(await session.answerApproval('always_allow', undefined, 'project')).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'approval_response',
			call_id: 'call-2',
			decision: { decision: 'always_allow', scope: 'project' }
		});

		// Step limit pause & continueTurn.
		session.onEvent({ event: 'step_limit_reached', turn: 1, limit: 15 });
		expect(session.stepLimit).toEqual({ turn: 1, limit: 15 });
		expect(await session.continueTurn(true)).toBe(true);
		expect(session.stepLimit).toBeNull();
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'continue_turn',
			turn: 1,
			proceed: true
		});

		// Pruning, diagnostics, and turn completion settle state.
		session.onEvent({ event: 'diagnostics', errors: 2, warnings: 1 });
		expect(session.diagnostics).toEqual({ errors: 2, warnings: 1 });
		session.onEvent({
			event: 'context_pruned',
			report: {
				removed_tokens: 1200,
				items: ['older shell output']
			}
		});
		expect(session.items.some((it) => it.kind === 'pruned' && it.removedTokens === 1200)).toBe(true);
		session.onEvent({ event: 'turn_complete', turn: 1, usage: { prompt_tokens: 100, completion_tokens: 20 } });

		// Fork preserves transcript items and flags modelPickerAfterFork when requested.
		const itemCountBeforeFork = session.items.length;
		expect(await session.forkSession(true)).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'fork_session' });
		session.onEvent({
			event: 'session_configured',
			session: { ...sessionInfo, id: 'forked-session' }
		});
		expect(session.items.length).toBe(itemCountBeforeFork);
		expect(session.modelPickerAfterFork).toBe(true);

		// Rewind points, file conflict detection, and forced overwrite.
		expect(await session.listRewindPoints()).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'list_rewind_points' });
		session.onEvent({
			event: 'rewind_points',
			points: [{ back: 1, text: 'Review @src/main.rs please', files: ['src/lib.rs'] }]
		});
		expect(session.rewindPoints).toHaveLength(1);

		expect(await session.rewind(1, true, true)).toBe(true);
		session.onEvent({ event: 'files_restored', restored: [], conflicts: ['src/lib.rs'] });
		expect(session.rewindConflict).toEqual({
			back: 1,
			conversation: true,
			files: true,
			conflicts: ['src/lib.rs']
		});

		expect(await session.rewind(1, true, true, true)).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'rewind',
			back: 1,
			conversation: true,
			files: true,
			force: true
		});
		session.onEvent({ event: 'rewound', back: 1, text: 'Review @src/main.rs please' });
		expect(session.draft).toBe('Review @src/main.rs please');
		expect(session.items.filter((i) => i.kind === 'user')).toHaveLength(0);

		// saveSetting triggers reload_config when setting_saved arrives.
		expect(await session.setPlannerModel('anthropic/claude-opus')).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'save_setting',
			key: 'model.planner',
			value: '"anthropic/claude-opus"',
			scope: 'user'
		});
		session.onEvent({
			event: 'setting_saved',
			key: 'model.planner',
			value: '"anthropic/claude-opus"',
			path: '/home/user/.config/quack/config.toml'
		});
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'reload_config' });
	});

	it('manages settings panel state, config feedback/errors, profiles, and folder picker', async () => {
		const { session, send } = fixture();

		expect(session.settingsOpen).toBe(false);
		session.openSettings('paths');
		expect(session.settingsOpen).toBe(true);
		expect(session.settingsTab).toBe('paths');
		expect(session.setupOpen).toBe(false);

		expect(await session.saveSetting('memory.path', '"/tmp/mem"', 'user')).toBe(true);
		expect(session.settingSaving).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'save_setting',
			key: 'memory.path',
			value: '"/tmp/mem"',
			scope: 'user'
		});

		session.onEvent({
			event: 'setting_saved',
			key: 'memory.path',
			value: '"/tmp/mem"',
			path: '/home/user/.config/quack/config.toml'
		});
		expect(session.settingSaving).toBe(false);
		expect(session.settingError).toBeNull();
		expect(session.settingFeedback).toEqual({
			key: 'memory.path',
			value: '"/tmp/mem"',
			path: '/home/user/.config/quack/config.toml'
		});
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'reload_config' });

		session.onEvent({ event: 'error', error: { kind: 'config', message: 'setting is locked' } });
		expect(session.settingSaving).toBe(false);
		expect(session.settingError).toBe('setting is locked');

		expect(await session.listProfiles()).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'list_profiles' });
		session.onEvent({
			event: 'profiles',
			active: 'generic',
			forced: false,
			profiles: [
				{
					id: 'cc-style',
					description: 'CC Style',
					source: 'defaults',
					matches: false,
					edit_format: 'str_replace',
					tools: ['read', 'edit', 'shell'],
					system: 'Prompt'
				}
			]
		});
		expect(session.profiles?.profiles).toHaveLength(1);

		expect(await session.setProfile('cc-style')).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'set_profile', id: 'cc-style' });

		expect(await session.reloadConfig()).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'reload_config' });

		session.closeSettings();
		expect(session.settingsOpen).toBe(false);
		session.openSetup();
		expect(session.settingsOpen).toBe(false);
		expect(session.setupOpen).toBe(true);
	});

	it('manages MCP servers, hooks, hook payload choices, and declarative hook setups', async () => {
		const { session, send } = fixture();

		session.openSettings('extensions');
		expect(session.settingsTab).toBe('extensions');
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'list_mcp' });

		expect(await session.enableMcp('github')).toBe(true);
		expect(session.mcpSaving).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'enable_mcp', id: 'github' });
		session.onEvent({ event: 'mcp_enabled', id: 'github', path: '~/.config/quack/mcp.d/github.toml' });
		expect(session.mcpSaving).toBe(false);
		expect(session.mcpFeedback).toContain('github');

		expect(
			await session.saveMcpServer({
				id: 'custom-http',
				scope: 'user',
				description: 'Custom MCP',
				command: '',
				args: [],
				cwd: '',
				url: 'https://mcp.example.com/mcp',
				env: {},
				headers: { Authorization: { kind: 'env', env: 'MCP_TOKEN', format: 'Bearer {}' } }
			})
		).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'save_mcp_server',
			id: 'custom-http',
			scope: 'user',
			description: 'Custom MCP',
			command: '',
			args: [],
			cwd: '',
			url: 'https://mcp.example.com/mcp',
			env: {},
			headers: { Authorization: { kind: 'env', env: 'MCP_TOKEN', format: 'Bearer {}' } }
		});
		session.onEvent({
			event: 'mcp_server_saved',
			id: 'custom-http',
			path: '~/.config/quack/mcp.d/custom-http.toml'
		});
		expect(session.mcpSaving).toBe(false);
		expect(session.mcpFeedback).toContain('custom-http');

		expect(await session.restartMcp('custom-http')).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'restart_mcp', id: 'custom-http' });

		expect(await session.disableMcp('github', 'user')).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'save_setting',
			key: 'mcp.disabled',
			value: '["github"]',
			scope: 'user'
		});

		expect(await session.enableHook('jira')).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({ op: 'enable_hook', id: 'jira' });
		session.onEvent({ event: 'hook_payload_choice', id: 'jira', path: '~/.config/quack/hooks.d/jira.mjs' });
		expect(session.hookPayloadChoice).toEqual({
			id: 'jira',
			path: '~/.config/quack/hooks.d/jira.mjs'
		});
		expect(await session.enableHook('jira', false)).toBe(true);
		expect(send.mock.calls.at(-1)?.[1]).toEqual({
			op: 'enable_hook',
			id: 'jira',
			replace_payload: false
		});
		session.onEvent({
			event: 'hook_enabled',
			id: 'jira',
			path: '~/.config/quack/hooks.d/jira.toml',
			note: null
		});
		expect(session.hookPayloadChoice).toBeNull();
		expect(session.hookFeedback).toContain('jira');

		session.onEvent({
			event: 'hook_ran',
			id: 'jira',
			on: 'user_prompt',
			outcome: 'context',
			ms: 28,
			message: 'Fetched ENG-12',
			context_tokens: 140
		});
		expect(session.hookRuns).toHaveLength(1);
		expect(session.hookRuns[0].id).toBe('jira');
		expect(session.hookRuns[0].contextTokens).toBe(140);

		expect(await session.inspectHookSetup('jira')).toBe(true);
		const inspectReq = (send.mock.calls.at(-1)?.[1] as { request: number }).request;
		expect(session.hookSetupLoading).toBe(true);
		session.onEvent({
			event: 'hook_setup',
			request: inspectReq,
			info: {
				id: 'jira',
				revision: 'r1',
				descriptor: { version: 1, scope_field: 'web_url', fields: [], presets: [] },
				connections: [],
				enabled: false,
				source: 'defaults',
				writable: true,
				problem: null
			}
		});
		expect(session.hookSetupLoading).toBe(false);
		expect(session.hookSetup?.id).toBe('jira');

		const draftConn = {
			connection: { id: 'default', preset: 'cloud', enabled: true, values: { web_url: 'https://co.atlassian.net' } },
			credentials: { token: { action: 'replace' as const, value: 'secret' } }
		};
		expect(
			await session.checkHookSetup({
				id: 'jira',
				revision: 'r1',
				connection: draftConn,
				url: 'https://co.atlassian.net'
			})
		).toBe(true);
		const checkReq = (send.mock.calls.at(-1)?.[1] as { request: number }).request;
		session.onEvent({
			event: 'hook_check_result',
			request: checkReq,
			status: 'ok',
			message: 'Connected!'
		});
		expect(session.hookSetupChecking).toBe(false);
		expect(session.hookSetupStatus).toEqual({ kind: 'ok', text: 'Connected!' });

		expect(
			await session.saveHookSetup({
				id: 'jira',
				revision: 'r1',
				connections: [draftConn],
				enable: true
			})
		).toBe(true);
		const saveReq = (send.mock.calls.at(-1)?.[1] as { request: number }).request;
		session.onEvent({
			event: 'hook_setup_saved',
			request: saveReq,
			info: {
				id: 'jira',
				revision: 'r2',
				descriptor: { version: 1, scope_field: 'web_url', fields: [], presets: [] },
				connections: [],
				enabled: true,
				source: 'user',
				writable: true,
				problem: null
			},
			note: 'Saved'
		});
		expect(session.hookSetupSaving).toBe(false);
		expect(session.hookSetup?.enabled).toBe(true);
		expect(session.hookSetupStatus).toEqual({ kind: 'ok', text: 'Saved' });
	});
});

describe('update checks', () => {
	it('ignores auto update_available when updates_check is disabled and honors it when enabled', async () => {
		const { session, openUrl } = fixture();
		session.onEvent({ event: 'startup_complete' });

		// When updates_check is false, auto-check events must not surface a banner.
		session.onEvent({ event: 'update_available', current: '0.1.0', latest: '0.2.0' });
		expect(session.updateState).toBeNull();
		expect(session.updateDismissed).toBe(false);

		// Turning updates.check on (and providing updates.url) surfaces the auto-check notice.
		session.onInfo({
			...info,
			egress: { ...info.egress, updates_check: true },
			settings: [
				{
					key: 'updates.url',
					value: '"https://github.com/ClackStack/quack/releases/latest"',
					origin: 'defaults',
					description: '',
					is_bool: false,
					locked: false
				}
			]
		});
		session.onEvent({ event: 'update_available', current: '0.1.0', latest: '0.2.0' });
		expect(session.updateState).toMatchObject({
			current: '0.1.0',
			latest: '0.2.0',
			updateAvailable: true,
			url: 'https://github.com/ClackStack/quack/releases/latest',
			source: 'auto',
			error: null
		});
		expect(session.updateDismissed).toBe(false);

		// Dismissing remembers the dismissal for the same release if reloaded, then clears if turned off.
		session.dismissUpdateBanner();
		session.onEvent({ event: 'update_available', current: '0.1.0', latest: '0.2.0' });
		expect(session.updateDismissed).toBe(true);

		expect(await session.openReleasePage()).toBe(true);
		expect(openUrl).toHaveBeenCalledWith('https://github.com/ClackStack/quack/releases/latest');

		session.onInfo({
			...info,
			egress: { ...info.egress, updates_check: false }
		});
		expect(session.updateState).toBeNull();
		expect(session.updateDismissed).toBe(false);
	});

	it('sends check_updates manually even with auto-check off and tracks results', async () => {
		const { session, send, openUrl } = fixture();
		session.onEvent({ event: 'startup_complete' });

		expect(await session.checkForUpdates()).toBe(true);
		expect(session.updateChecking).toBe(true);
		expect(send).toHaveBeenCalledWith(3, { op: 'check_updates' });
		// Guard against duplicate in-flight clicks.
		expect(await session.checkForUpdates()).toBe(false);

		session.onEvent({
			event: 'update_check_result',
			current: '0.1.0',
			latest: '0.1.0',
			update_available: false,
			url: 'https://releases.example/latest',
			error: undefined
		});
		expect(session.updateChecking).toBe(false);
		expect(session.updateState).toMatchObject({
			current: '0.1.0',
			latest: '0.1.0',
			updateAvailable: false,
			url: 'https://releases.example/latest',
			source: 'manual',
			error: null
		});

		// Manual check errors surface without leaving updateChecking stuck.
		expect(await session.checkForUpdates()).toBe(true);
		session.onEvent({
			event: 'update_check_result',
			current: '0.1.0',
			latest: undefined,
			update_available: false,
			url: 'javascript:alert(1)',
			error: 'blocked by network.offline'
		});
		expect(session.updateChecking).toBe(false);
		expect(session.updateState?.error).toBe('blocked by network.offline');
		expect(await session.openReleasePage()).toBe(false);
		expect(openUrl).not.toHaveBeenCalled();

		// Closing resets in-flight state cleanly.
		expect(await session.checkForUpdates()).toBe(true);
		expect(session.updateChecking).toBe(true);
		session.onClosed(null);
		expect(session.updateChecking).toBe(false);
	});

	it('handles manual check_updates in dev transport cleanly without auto-checking when off', async () => {
		const dev = devTransport();
		const events: unknown[] = [];
		const off = dev.listen((msg: TabMessage) => {
			if ('event' in msg) events.push(msg.event);
		});
		const opened = await dev.openFolder('~/demo', { resume: 'latest' });
		expect(opened.info.egress.updates_check).toBe(false);

		// Default dev state has offline = true, so check_updates reports the offline block.
		await dev.sendOp(opened.tab, { op: 'check_updates' });
		expect(events).toContainEqual(
			expect.objectContaining({
				event: 'update_check_result',
				update_available: false,
				error: expect.stringContaining('network.offline')
			})
		);

		// Turning offline off yields an up-to-date check_updates response.
		events.length = 0;
		await dev.sendOp(opened.tab, {
			op: 'save_setting',
			key: 'network.offline',
			value: 'false',
			scope: 'user'
		});
		await dev.sendOp(opened.tab, { op: 'check_updates' });
		expect(events).toContainEqual(
			expect.objectContaining({
				event: 'update_check_result',
				update_available: false,
				error: undefined
			})
		);
		off();
		await dev.closeTab(opened.tab);
	});

	it('tracks live egress ledger entries, Secret Shield approvals, LSP diagnostics, tool probe, and context pruning', async () => {
		const { session, send } = fixture();

		await session.refreshEgress();
		expect(send).toHaveBeenCalledWith(3, { op: 'list_egress' });
		session.onEvent({
			event: 'egress_ledger',
			entries: [
				{
					seq: 1,
					timestamp_ms: 1000,
					kind: 'inference',
					host: 'api.example.test',
					path: '/v1/chat/completions',
					protocol: 'https',
					local: false,
					bytes_sent: 512,
					bytes_received: 2048,
					status: 200,
					redactions: 0
				}
			],
			stats: {
				total_requests: 1,
				total_bytes_sent: 512,
				total_bytes_received: 2048,
				local_requests: 0,
				remote_requests: 1,
				blocked_count: 0,
				redacted_count: 0
			}
		});
		// Duplicate seq should be ignored while new seq increments totals.
		session.onEvent({
			event: 'egress_recorded',
			entry: {
				seq: 1,
				timestamp_ms: 1000,
				kind: 'inference',
				host: 'api.example.test',
				path: '/v1/chat/completions',
				protocol: 'https',
				local: false,
				bytes_sent: 512,
				bytes_received: 2048,
				status: 200,
				redactions: 0
			}
		});
		expect(session.egressEntries).toHaveLength(1);
		session.onEvent({
			event: 'egress_recorded',
			entry: {
				seq: 2,
				timestamp_ms: 2000,
				kind: 'inference',
				host: 'api.example.test',
				path: '/v1/chat/completions',
				protocol: 'https',
				local: false,
				bytes_sent: 256,
				bytes_received: 1024,
				status: 200,
				redactions: 1
			}
		});
		expect(session.egressEntries).toHaveLength(2);
		expect(session.egressStats?.total_requests).toBe(2);
		expect(session.egressStats?.redacted_count).toBe(1);

		// Secret Shield approval flow (`redact_and_send`).
		session.onEvent({
			event: 'approval_request',
			call_id: 'egress-1',
			request: {
				kind: 'egress_secrets',
				provider: 'mock',
				host: 'api.example.test',
				findings: [
					{
						id: 'f1',
						kind: 'aws_access_key',
						location: 'prompt',
						preview: 'AKIA••••••••1234',
						fingerprint: 'fp-1234567890abcdef'
					}
				]
			},
			always: []
		});
		expect(session.approval?.request.kind).toBe('egress_secrets');
		await session.answerApproval('redact_and_send');
		expect(send).toHaveBeenCalledWith(3, {
			op: 'approval_response',
			call_id: 'egress-1',
			decision: { decision: 'redact_and_send' }
		});
		expect(session.approval).toBeNull();

		// Tool-calling capability probe + LSP diagnostics + selective prune.
		await session.probeToolCalling();
		expect(send).toHaveBeenCalledWith(3, { op: 'probe_tool_calling' });
		expect(session.toolProbing).toBe(true);
		session.onEvent({
			event: 'tool_probe_result',
			report: {
				model: 'mock/org/model',
				configured_mode: 'auto',
				recommended_mode: 'native',
				cases: [{ name: 'native_echo', passed: true, latency_ms: 42, detail: 'ok' }],
				summary: 'Native tool calling verified.'
			}
		});
		expect(session.toolProbing).toBe(false);
		expect(session.toolProbe?.recommended_mode).toBe('native');

		await session.refreshLspDiagnostics('rust-analyzer');
		expect(send).toHaveBeenCalledWith(3, { op: 'list_lsp_diagnostics', server_id: 'rust-analyzer' });
		session.onEvent({
			event: 'lsp_diagnostics',
			items: [
				{
					server: 'rust-analyzer',
					path: 'src/main.rs',
					line: 12,
					column: 5,
					severity: 'error',
					message: 'mismatched types'
				}
			]
		});
		expect(session.lspDiagnostics).toHaveLength(1);

		await session.pruneContext([4]);
		expect(send).toHaveBeenCalledWith(3, { op: 'prune_context', indices: [4] });
	});

	it('saves large, small, or both default models and updates info live', async () => {
		const { session, send } = fixture();
		session.onEvent({ event: 'startup_complete' });

		expect(await session.setModelDefaults('small', 'charm/qwen-flash', 'user')).toBe(true);
		expect(send).toHaveBeenCalledWith(3, {
			op: 'save_setting',
			key: 'model.small',
			value: '"charm/qwen-flash"',
			scope: 'user'
		});
		session.onEvent({
			event: 'setting_saved',
			key: 'model.small',
			value: '"charm/qwen-flash"',
			path: '~/.config/quack/config.toml'
		});
		expect(session.settingSaving).toBe(false);
		expect(session.info?.small_model).toBe('charm/qwen-flash');

		expect(await session.setModelDefaults('both', 'charm/glm-5.3', 'project')).toBe(true);
		expect(send).toHaveBeenCalledWith(3, {
			op: 'save_setting',
			key: 'model.default',
			value: '"charm/glm-5.3"',
			scope: 'project'
		});
		expect(send).toHaveBeenCalledWith(3, {
			op: 'save_setting',
			key: 'model.small',
			value: '"charm/glm-5.3"',
			scope: 'project'
		});
		expect(session.settingSaving).toBe(true);
		session.onEvent({
			event: 'setting_saved',
			key: 'model.default',
			value: '"charm/glm-5.3"',
			path: '.quack/config.toml'
		});
		expect(session.settingSaving).toBe(true);
		session.onEvent({
			event: 'setting_saved',
			key: 'model.small',
			value: '"charm/glm-5.3"',
			path: '.quack/config.toml'
		});
		expect(session.settingSaving).toBe(false);
		expect(session.info?.default_model).toBe('charm/glm-5.3');
		expect(session.info?.small_model).toBe('charm/glm-5.3');
	});

	it('tracks live cache hit percent, loaded skills, and LSP servers across session lifecycle', () => {
		const { session } = fixture();
		session.onEvent({ event: 'startup_complete' });

		session.onEvent({
			event: 'context_update',
			used: 12000,
			budget: 24000,
			window: 32000,
			cache_hit_tokens: 9000,
			cache_hit_percent: 75,
			threshold: 20000
		});
		expect(session.cacheHitPercent).toBe(75);

		// A follow-up context estimate without cache_hit_percent preserves the last known ratio.
		session.onEvent({
			event: 'context_update',
			used: 12500,
			budget: 24000,
			window: 32000,
			threshold: 20000
		});
		expect(session.cacheHitPercent).toBe(75);

		// Stats report also updates cacheHitPercent (e.g. after resume).
		session.onEvent({
			event: 'stats',
			report: {
				model: 'mock/org/model',
				turns: 2,
				requests: 4,
				unreported: 0,
				prompt_tokens: 10000,
				completion_tokens: 500,
				cached_tokens: 8200,
				cache_write_tokens: 0,
				reasoning_tokens: 0,
				cache_reported: 3,
				unpriced: 0,
				open_ms: 1000,
				model_ms: 500,
				tools_ms: 100,
				approval_ms: 0,
				tools: [],
				compactions: 0,
				pruned: 0,
				resumed: false,
				sampling: [],
				history: []
			}
		});
		expect(session.cacheHitPercent).toBe(82);

		session.onEvent({ event: 'loaded_skills', skills: ['rust-best-practices', 'release'] });
		expect(session.loadedSkills).toEqual(['rust-best-practices', 'release']);

		session.onEvent({
			event: 'lsp_servers',
			servers: [
				{
					id: 'rust-analyzer',
					command: 'rust-analyzer',
					extensions: ['rs'],
					state: 'ready',
					detail: '',
					root: '.',
					errors: 0,
					warnings: 0,
					log: []
				}
			]
		});
		expect(session.lspServers).toHaveLength(1);

		// Switching to a new session resets conversation-scoped metrics & loaded skills, not workspace LSPs.
		session.onEvent({
			event: 'session_configured',
			session: { ...sessionInfo, id: 'fresh-session' }
		});
		expect(session.cacheHitPercent).toBeNull();
		expect(session.loadedSkills).toEqual([]);
		expect(session.lspServers).toHaveLength(1);
	});
});
