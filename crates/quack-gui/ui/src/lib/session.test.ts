import { describe, expect, it, vi } from 'vitest';
import { Session } from './session.svelte';
import type { AppInfo, Transport } from './transport/types';

const info: AppInfo = {
	version: 'test',
	cwd: '/workspace',
	branch: null,
	model: 'mock/org/model',
	permission_mode: 'ask',
	work_mode: 'edit',
	ui: { theme: 'auto', reduced_motion: false },
	settings: [],
	layers: [],
	rules: [],
	bundles: [],
	bundles_disabled: []
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
});
