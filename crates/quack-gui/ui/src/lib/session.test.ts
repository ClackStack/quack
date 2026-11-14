import { describe, expect, it, vi } from 'vitest';
import { Session } from './session.svelte';
import type { AppInfo, Transport } from './transport/types';

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
const sessionInfo = { id: 'original', cwd: '/workspace', model: info.model, profile: 'generic' };
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
	session.onEvent({ event: 'session_configured', session: { ...sessionInfo, model } });
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
		session.onEvent({ event: 'model_catalog', providers: [provider('first')], current: null });
		await session.saveProviderKey('second', 'test-key');
		expect(session.setupWorking).toBe(true);
		session.onEvent({ event: 'provider_added', id: 'second' });
		session.onEvent({ event: 'model_catalog', providers: [provider('second')], current: null });
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
		expect(send).toHaveBeenCalledWith(3, { op: 'set_reasoning_effort', effort: null });
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
		session.onEvent({ event: 'sign_in_result', id: 'charm', error: null });
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
		session.onEvent({ event: 'sign_in_result', id: 'other', error: null });
		expect(session.setupNotice).toBe('');
	});
	it('clears the pending sign-in locally on cancel', async () => {
		const { session, send } = fixture();
		await session.signIn('charm');
		session.onEvent({ event: 'sign_in_url', id: 'charm', url: 'https://charm.example' });
		await session.signIn(null);
		expect(send).toHaveBeenCalledWith(3, { op: 'sign_in', id: null });
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

		// Error stays more severe than subsequent turn_aborted and clears via dismissIssue().
		session.onEvent({ event: 'turn_started', turn: 2 });
		session.onEvent({ event: 'error', error: { kind: 'provider', message: 'Quota exceeded' } });
		session.onEvent({ event: 'turn_aborted', turn: 2, reason: 'Quota exceeded' });
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
});
