import type { Event, Op } from '#lib/protocol';

/**
 * The bridge envelope (docs/plan/15-desktop-client.md#the-bridge): every message the
 * desktop bridge pushes across the IPC carries the tab it belongs to plus an untagged
 * payload, matching `quack_gui::TabMessage`.
 */
export interface AppInfo {
	version: string;
	cwd: string;
	branch: string | null;
	model: string | null;
	permission_mode: string;
	ui: { theme: string; reduced_motion: boolean };
	settings: {
		key: string;
		value: string;
		origin: string;
		description: string;
		is_bool: boolean;
		locked: boolean;
	}[];
	layers: { layer: string; status: string; file: string | null }[];
	rules: { list: string; rule: string }[];
	bundles: { id: string; layer: string; enabled: boolean; contents: string }[];
	bundles_disabled: string[];
}

export type TabMessage =
	| { tab: number; event: Event }
	| { tab: number; info: AppInfo }
	| { tab: number; closed: { reason: string | null } };

/** What a tab opens with; matches the shell's `ResumeDto`. */
export type ResumeRequest = { resume: 'new' } | { resume: 'latest' } | { resume: 'id'; id: string };

export interface RecentFolder {
	path: string;
	opened: number;
}

export interface OpenedTab {
	tab: number;
	info: AppInfo;
}

/**
 * The one seam between the UI and the engines. The desktop transport wraps Tauri
 * IPC (`invoke` + one channel per tab); the dev transport scripts a fake session so
 * `npm run dev` works in a plain browser. A future browser-over-socket client
 * (plan open question) re-enters here without moving the UI.
 */
export interface Transport {
	readonly kind: 'tauri' | 'dev';
	pickFolder(): Promise<string | null>;
	openFolder(cwd: string, resume: ResumeRequest): Promise<OpenedTab>;
	sendOp(tab: number, op: Op): Promise<void>;
	closeTab(tab: number): Promise<void>;
	recentFolders(limit?: number): Promise<RecentFolder[]>;
	listen(listener: (message: TabMessage) => void): () => void;
}
