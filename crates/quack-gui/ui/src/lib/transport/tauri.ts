import { invoke, Channel } from '@tauri-apps/api/core';
import type { Op } from '#lib/protocol';
import type {
	AppInfo,
	OpenedTab,
	RecentFolder,
	ResumeRequest,
	TabMessage,
	Transport
} from './types';

/**
 * The desktop transport: one `tauri::ipc::Channel` per tab, handed to `open_folder`;
 * the shell streams coalesced bridge messages through it (docs/plan/15-desktop-client.md#the-bridge).
 */
export function tauriTransport(): Transport {
	// One connection per UI document. Reconnecting retires discarded channels and
	// awaits their engines before any new tab can acquire the same workspace.
	let connection: Promise<number> | undefined;
	const connect = () => connection ??= invoke<number>('connect');
	const listeners = new Set<(message: TabMessage) => void>();
	const fan = (message: TabMessage) => {
		for (const listener of listeners) listener(message);
	};

	return {
		kind: 'tauri',

		async pickFolder(): Promise<string | null> {
			const { open } = await import('@tauri-apps/plugin-dialog');
			const picked = await open({ directory: true, multiple: false });
			return typeof picked === 'string' ? picked : null;
		},

		async openFolder(cwd: string, resume: ResumeRequest): Promise<OpenedTab> {
			const client = await connect();
			const channel = new Channel<TabMessage>();
			// The AppInfo snapshot is the bridge's first message on the channel; capture it
			// for the caller and fan everything out to the listeners from the start.
			const infoReady = new Promise<AppInfo>((resolve) => {
				channel.onmessage = (message: TabMessage) => {
					if ('info' in message) resolve(message.info);
					fan(message);
				};
			});
			const resumeArg =
				resume.resume === 'id' ? { resume: 'id', id: resume.id } : { resume: resume.resume };
			const tab = await invoke<number>('open_folder', {
				client,
				cwd,
				resume: resumeArg,
				events: channel
			});
			return { tab, info: await infoReady };
		},

		async sendOp(tab: number, op: Op): Promise<void> {
			await invoke('send_op', { client: await connect(), tab, op });
		},

		async closeTab(tab: number): Promise<void> {
			await invoke('close_tab', { client: await connect(), tab });
		},

		async recentFolders(limit = 10): Promise<RecentFolder[]> {
			return await invoke<RecentFolder[]>('recent_folders', { client: await connect(), limit });
		},

		async openUrl(url: string): Promise<void> {
			await invoke('open_url', { url });
		},

		listen(listener: (message: TabMessage) => void): () => void {
			listeners.add(listener);
			return () => listeners.delete(listener);
		}
	};
}
