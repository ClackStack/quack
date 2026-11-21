import { invoke, Channel } from '@tauri-apps/api/core';
import type {
	AppInfo,
	ClientOp,
	HighlightSpan,
	LaunchTarget,
	OpenedTab,
	RecentFolder,
	ResumeRequest,
	SavedTab,
	StartupPayload,
	TabMessage,
	Transport,
	WorkspaceSymbol
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

		async sendOp(tab: number, op: ClientOp): Promise<void> {
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

		async listFiles(tab: number, limit = 5000): Promise<string[]> {
			return await invoke<string[]>('list_workspace_files', { client: await connect(), tab, limit });
		},

		async listSymbols(
			tab: number,
			path: string,
			query?: string,
			limit = 50
		): Promise<WorkspaceSymbol[]> {
			return await invoke<WorkspaceSymbol[]>('list_workspace_symbols', {
				client: await connect(),
				tab,
				path,
				query: query ?? null,
				limit
			});
		},

		async startupTargets(): Promise<StartupPayload> {
			return await invoke<StartupPayload>('startup_targets', { client: await connect() });
		},

		async saveOpenTabs(tabs: SavedTab[]): Promise<void> {
			await invoke('save_open_tabs', { client: await connect(), tabs });
		},

		onOpenRequest(handler: (targets: LaunchTarget[]) => void): () => void {
			let active = true;
			let unlisten: (() => void) | undefined;
			void import('@tauri-apps/api/event')
				.then(({ listen }) =>
					listen<LaunchTarget[]>('quack://open-request', (event) => {
						if (active) handler(event.payload);
					})
				)
				.then((fn) => {
					if (active) unlisten = fn;
					else fn();
				})
				.catch(() => {});
			return () => {
				active = false;
				unlisten?.();
			};
		},

		async inputHistory(tab: number): Promise<string[]> {
			return await invoke<string[]>('input_history', { client: await connect(), tab });
		},

		async revealPath(tab: number, path: string, openFile = false): Promise<void> {
			await invoke('reveal_path', { client: await connect(), tab, path, openFile });
		},

		async highlightCode(lang: string, code: string): Promise<HighlightSpan[] | null> {
			return await invoke<HighlightSpan[] | null>('highlight_code', { lang, code });
		},

		async pickImages(): Promise<string[]> {
			const { open } = await import('@tauri-apps/plugin-dialog');
			const picked = await open({
				directory: false,
				multiple: true,
				filters: [{ name: 'Images', extensions: ['png', 'jpg', 'jpeg', 'gif', 'webp'] }]
			});
			if (typeof picked === 'string') return [picked];
			return Array.isArray(picked) ? picked : [];
		},

		async pickSavePath(defaultName: string): Promise<string | null> {
			const { save } = await import('@tauri-apps/plugin-dialog');
			const picked = await save({ defaultPath: defaultName });
			return typeof picked === 'string' ? picked : null;
		},

		async stagePastedImage(tab: number, bytes: Uint8Array): Promise<string> {
			return await invoke<string>('stage_pasted_image', {
				client: await connect(),
				tab,
				bytes: Array.from(bytes)
			});
		},

		async notifyAttention(title: string, body: string): Promise<void> {
			await invoke('notify_attention', { title, body });
		},

		async setWindowTitle(title: string): Promise<void> {
			const { getCurrentWindow } = await import('@tauri-apps/api/window');
			await getCurrentWindow().setTitle(title);
		},

		listen(listener: (message: TabMessage) => void): () => void {
			listeners.add(listener);
			return () => listeners.delete(listener);
		}
	};
}
