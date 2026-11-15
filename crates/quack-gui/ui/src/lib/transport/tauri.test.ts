import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { AppInfo, TabMessage } from './types';

const { invoke } = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock('@tauri-apps/api/core', () => ({
	invoke,
	Channel: class { onmessage: (message: TabMessage) => void = () => {}; }
}));
import { tauriTransport } from './tauri';

const info = { cwd: '/workspace' } as AppInfo;

beforeEach(() => { invoke.mockReset(); });

describe('desktop connection and open handshake', () => {
	it('connects once before opening and preserves events before the command resolves', async () => {
		invoke.mockImplementation(async (command, args) => {
			if (command === 'connect') return 42;
			if (command === 'open_folder') {
				args.events.onmessage({ tab: 7, info });
				args.events.onmessage({ tab: 7, event: { event: 'startup_complete' } });
				return 7;
			}
			return [];
		});
		const transport = tauriTransport();
		const seen: TabMessage[] = [];
		transport.listen((message) => seen.push(message));
		const opening = transport.openFolder('/workspace', { resume: 'latest' });
		expect(invoke).toHaveBeenCalledWith('connect');
		await expect(opening).resolves.toEqual({ tab: 7, info });
		expect(seen).toHaveLength(2);
		await transport.sendOp(7, { op: 'interrupt' });
		await transport.recentFolders();
		await transport.closeTab(7);
		expect(invoke.mock.calls.filter(([command]) => command === 'connect')).toHaveLength(1);
		expect(invoke).toHaveBeenCalledWith('open_folder', expect.objectContaining({ client: 42 }));
		expect(invoke).toHaveBeenCalledWith('send_op', { client: 42, tab: 7, op: { op: 'interrupt' } });
		expect(invoke).toHaveBeenCalledWith('recent_folders', { client: 42, limit: 10 });
		expect(invoke).toHaveBeenCalledWith('close_tab', { client: 42, tab: 7 });
	});

	it('shares an in-flight connection and waits for old engines to stop before opening', async () => {
		let connected!: (client: number) => void;
		invoke.mockImplementation((command) => command === 'connect'
			? new Promise<number>((resolve) => connected = resolve)
			: Promise.resolve([]));
		const transport = tauriTransport();
		const first = transport.recentFolders();
		const second = transport.closeTab(7);
		expect(invoke).toHaveBeenCalledOnce();
		connected(43);
		await Promise.all([first, second]);
		expect(invoke).toHaveBeenCalledWith('recent_folders', { client: 43, limit: 10 });
		expect(invoke).toHaveBeenCalledWith('close_tab', { client: 43, tab: 7 });
	});

	it('propagates an open failure without waiting for a snapshot that will not arrive', async () => {
		invoke.mockImplementation((command) => command === 'connect'
			? Promise.resolve(42) : Promise.reject('cannot open folder'));
		await expect(tauriTransport().openFolder('/missing', { resume: 'new' }))
			.rejects.toBe('cannot open folder');
	});

	it('routes desktop helper commands cleanly through Tauri IPC', async () => {
		invoke.mockImplementation(async (command) => {
			if (command === 'connect') return 9;
			if (command === 'list_workspace_files') return ['src/lib.rs'];
			if (command === 'highlight_code') return [{ text: 'fn', kind: 'keyword' }];
			if (command === 'stage_pasted_image') return '/tmp/quack-gui-paste/paste.png';
			return undefined;
		});
		const transport = tauriTransport();
		await expect(transport.listFiles?.(4)).resolves.toEqual(['src/lib.rs']);
		await transport.revealPath?.(4, 'src/lib.rs', true);
		await expect(transport.highlightCode?.('rust', 'fn')).resolves.toEqual([{ text: 'fn', kind: 'keyword' }]);
		await expect(transport.stagePastedImage?.(4, new Uint8Array([1, 2, 3]))).resolves.toBe(
			'/tmp/quack-gui-paste/paste.png'
		);
		await transport.notifyAttention?.('quack', 'Approval needed');
		expect(invoke).toHaveBeenCalledWith('reveal_path', {
			client: 9,
			tab: 4,
			path: 'src/lib.rs',
			openFile: true
		});
		expect(invoke).toHaveBeenCalledWith('notify_attention', {
			title: 'quack',
			body: 'Approval needed'
		});
	});

	it('does not send commands after connection failure', async () => {
		invoke.mockRejectedValue('connection was replaced');
		const transport = tauriTransport();
		await expect(transport.openFolder('/workspace', { resume: 'new' })).rejects.toBe('connection was replaced');
		await expect(transport.sendOp(7, { op: 'interrupt' })).rejects.toBe('connection was replaced');
		expect(invoke).toHaveBeenCalledOnce();
	});

	it('reads shared input history via IPC', async () => {
		invoke.mockImplementation(async (cmd: string) => {
			if (cmd === 'connect') return 11;
			if (cmd === 'input_history') return ['first', 'second'];
			return undefined;
		});
		const transport = tauriTransport();
		await expect(transport.inputHistory?.(5)).resolves.toEqual(['first', 'second']);
		expect(invoke).toHaveBeenCalledWith('input_history', { client: 11, tab: 5 });
	});
});
