import { describe, expect, it, vi } from 'vitest';
import { Session } from './session.svelte';
import type { AppInfo, Transport } from './transport/types';

const info: AppInfo = {
	version: 'test',
	cwd: '/workspace',
	branch: null,
	model: 'mock/org/model',
	permission_mode: 'ask',
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
		listen: () => () => {}
	};
	const session = new Session(transport, 3, { ...info, model });
	session.onEvent({ event: 'session_configured', session: { ...sessionInfo, model } });
	return { session, send: vi.mocked(transport.sendOp) };
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
