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

describe('desktop open handshake', () => {
	it('invokes open before waiting for the channel snapshot and preserves events', async () => {
		invoke.mockImplementation(async (_command, args) => {
			args.events.onmessage({ tab: 7, info });
			args.events.onmessage({ tab: 7, event: { event: 'startup_complete' } });
			return 7;
		});
		const transport = tauriTransport();
		const seen: TabMessage[] = [];
		transport.listen((message) => seen.push(message));
		const opening = transport.openFolder('/workspace', { resume: 'latest' });
		expect(invoke).toHaveBeenCalledOnce();
		await expect(opening).resolves.toEqual({ tab: 7, info });
		expect(seen).toHaveLength(2);
	});

	it('propagates an open failure without waiting for a snapshot that will not arrive', async () => {
		invoke.mockRejectedValue('cannot open folder');
		await expect(tauriTransport().openFolder('/missing', { resume: 'new' }))
			.rejects.toBe('cannot open folder');
	});
});
