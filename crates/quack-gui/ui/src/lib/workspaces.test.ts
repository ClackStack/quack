import { describe, expect, it, vi } from 'vitest';
import { Workspaces } from './workspaces.svelte';
import { Session } from './session.svelte';
import type { AppInfo, TabMessage, Transport } from './transport/types';

const info: AppInfo = {
	version: 'test', cwd: '/workspace', branch: null, model: 'mock/org/model',
	permission_mode: 'ask', work_mode: 'edit', ui: { theme: 'auto', reduced_motion: false },
	settings: [], layers: [], rules: [], bundles: [], bundles_disabled: []
};

function fixture() {
	let receive: (message: TabMessage) => void = () => {};
	const unlisten = vi.fn();
	const transport: Transport = {
		kind: 'tauri', pickFolder: vi.fn(), recentFolders: vi.fn(),
		openFolder: vi.fn(), sendOp: vi.fn().mockResolvedValue(undefined),
		closeTab: vi.fn().mockResolvedValue(undefined), openUrl: vi.fn().mockResolvedValue(undefined),
		listen(listener) { receive = listener; return unlisten; }
	};
	const workspaces = new Workspaces(transport);
	return { transport, workspaces, unlisten, receive: (message: TabMessage) => receive(message) };
}

describe('workspace lifecycle', () => {
	it('retains replay and mode events sent before open resolves', async () => {
		const { transport, workspaces, receive } = fixture();
		vi.mocked(transport.openFolder).mockImplementation(async () => {
			receive({ tab: 3, info });
			receive({ tab: 3, event: {
				event: 'session_resumed',
				session: { id: 'saved', cwd: '/workspace', model: null, profile: 'generic' },
				items: [{ kind: 'assistant', text: 'Saved response' }]
			} });
			receive({ tab: 3, event: { event: 'mode_changed', work_mode: 'edit', approvals: 'read-only' } });
			return { tab: 3, info };
		});
		await workspaces.open('/workspace', { resume: 'latest' });
		expect(workspaces.sessions).toHaveLength(1);
		expect(workspaces.sessions[0].items[0]).toMatchObject({ text: 'Saved response' });
		expect(workspaces.sessions[0].approvals).toBe('read-only');
		expect(workspaces.sessions[0].workMode).toBe('edit');
		expect(workspaces.sessions[0].resumed).toBe(true);
	});

	it('focuses the existing workspace on a structured duplicate error', async () => {
		const { transport, workspaces } = fixture();
		workspaces.sessions.push(new Session(transport, 3, info));
		vi.mocked(transport.openFolder).mockRejectedValue({ kind: 'already_open', detail: { tab: 3 } });
		await workspaces.open('/workspace');
		expect(workspaces.activeIndex).toBe(0);
		expect(workspaces.error).toBeNull();
		expect(workspaces.sessions).toHaveLength(1);
	});

	it('marks a stopped tab disconnected and clears pending questions', () => {
		const { transport, workspaces, receive } = fixture();
		const session = new Session(transport, 3, info);
		workspaces.sessions.push(session);
		session.streaming = true;
		session.questions = { callId: 'q', questions: [] };
		receive({ tab: 3, closed: { reason: 'Lost events · reopen the folder' } });
		expect(session.closed).toBe(true);
		expect(session.streaming).toBe(false);
		expect(session.questions).toBeNull();
		expect(session.items[0]).toMatchObject({ tone: 'error' });
	});

	it('releases the page listener on disposal', () => {
		const { workspaces, unlisten } = fixture();
		workspaces.dispose();
		expect(unlisten).toHaveBeenCalledOnce();
	});

	it('keeps provider model ids containing slashes intact', async () => {
		const { transport } = fixture();
		await new Session(transport, 3, info).setModel('mock/org/model');
		expect(transport.sendOp).toHaveBeenCalledWith(3, {
			op: 'set_model', provider: 'mock', model: 'org/model'
		});
	});
	it('seals reasoning when assistant output starts and clears questions on interruption', () => {
		const { transport } = fixture();
		const session = new Session(transport, 3, info);
		session.onEvent({ event: 'reasoning_delta', text: 'Thinking' });
		session.onEvent({ event: 'assistant_delta', text: 'Answer' });
		session.questions = { callId: 'question', questions: [] };
		session.onEvent({ event: 'turn_aborted', turn: 1, reason: 'Interrupted' });
		expect(session.items.filter((item) => 'streaming' in item && item.streaming)).toHaveLength(0);
		expect(session.questions).toBeNull();
	});

});
