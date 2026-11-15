import type { Event, Op } from '#lib/protocol';

export interface EgressEndpoint {
	id: string;
	host: string;
	local: boolean;
	enabled: boolean;
}

export interface EgressSnapshot {
	offline: boolean;
	offline_locked: boolean;
	egress_allow: string[];
	providers: EgressEndpoint[];
	mcp_remote: EgressEndpoint[];
	http1_only: boolean;
	has_proxy: boolean;
	updates_check: boolean;
	catalog_refresh: boolean;
	telemetry_enabled: boolean;
	secret_guard?: string;
	secret_guard_local?: boolean;
	secret_allowlist?: string[];
}

export interface WorkspaceSymbol {
	name: string;
	kind: string;
	start: number;
	end: number;
	signature: string;
}

export interface LaunchTarget {
	path: string;
	resume?: string | null;
	continue_latest: boolean;
	focus: boolean;
}

export interface SavedTab {
	path: string;
	session_id?: string | null;
	active: boolean;
}

export interface StartupPayload {
	targets: LaunchTarget[];
	active_index?: number | null;
}

export interface SandboxSnapshot {
	mode: string;
	os_supported: boolean;
	status_detail: string;
	writable: string[];
	locked: boolean;
}

export interface HighlightSpan {
	text: string;
	kind?: string;
}

/** Engine `Event::StepLimitReached` (not yet exported into `protocol.ts` by `ts-rs`). */
export interface StepLimitEvent {
	event: 'step_limit_reached';
	turn: number;
	limit: number;
}

/** Engine `Op::ContinueTurn` (not yet exported into `protocol.ts` by `ts-rs`). */
export interface ContinueTurnOp {
	op: 'continue_turn';
	turn: number;
	proceed: boolean;
}

export type ClientEvent = Event | StepLimitEvent;
export type ClientOp = Op | ContinueTurnOp;

/**
 * The bridge envelope (docs/plan/15-desktop-client.md#the-bridge): every message the
 * desktop bridge pushes across the IPC carries the tab it belongs to plus an untagged
 * payload, matching `quack_gui::TabMessage`.
 */
export interface PathsSnapshot {
	defaults: string | null;
	system: string;
	user_config: string;
	user_data: string;
	user_cache: string;
	project: string;
	memory_global: string;
	memory_project: string;
	sessions_root: string;
	sessions_project: string;
}

export interface HookRow {
	id: string;
	layer: string;
	file: string;
	state: string;
	detail: string;
	enabled: boolean;
	can_toggle: boolean;
	has_setup: boolean;
	description: string;
	event: string | null;
	command: string[];
	matcher_tool: string;
	matcher_pattern: string;
	matcher_prompt: string;
	timeout_secs: number | null;
	on_error: string | null;
	full_context: boolean;
	connections_count: number;
}

export interface McpRow {
	id: string;
	layer: string;
	file: string;
	state: string;
	detail: string;
	enabled: boolean;
	can_toggle: boolean;
	blocked_by_policy: boolean;
	transport: string | null;
	target: string;
	description: string;
	sandbox: boolean | null;
	startup_timeout_secs: number | null;
	call_timeout_secs: number | null;
	max_output_tokens: number | null;
	auto_approve: string[];
	env_keys: string[];
	header_keys: string[];
}

export interface AppInfo {
	version: string;
	cwd: string;
	branch: string | null;
	model: string | null;
	planner_model: string | null;
	planner_locked: boolean;
	/** The approval policy label: `read-only`, `ask`, `auto-edit`, `allow` or `yolo`. */
	permission_mode: string;
	/** The work mode label: `plan` or `edit`. */
	work_mode: string;
	ui: { theme: string; reduced_motion: boolean; gui_restore_tabs?: boolean };
	egress: EgressSnapshot;
	sandbox: SandboxSnapshot;
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
	hooks?: HookRow[];
	hooks_disabled?: string[];
	mcp?: McpRow[];
	mcp_disabled?: string[];
	project_status?: string;
	paths?: PathsSnapshot;
	history: string[];
}

export type TabMessage =
	| { tab: number; event: ClientEvent }
	| { tab: number; info: AppInfo }
	| { tab: number; history: string[] }
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
	sendOp(tab: number, op: ClientOp): Promise<void>;
	closeTab(tab: number): Promise<void>;
	recentFolders(limit?: number): Promise<RecentFolder[]>;
	/** Opens a URL in the user's browser (a subscription sign-in address). */
	openUrl(url: string): Promise<void>;
	listen(listener: (message: TabMessage) => void): () => void;
	/** List workspace-relative file paths for `@` mentions. */
	listFiles?(tab: number, limit?: number): Promise<string[]>;
	/** List syntax symbols inside a workspace file for `@file#symbol` mentions. */
	listSymbols?(tab: number, path: string, query?: string, limit?: number): Promise<WorkspaceSymbol[]>;
	/** Fetch merged startup tabs (`session-tabs.json` + CLI arguments). */
	startupTargets?(): Promise<StartupPayload>;
	/** Persist currently open tabs for desktop restore. */
	saveOpenTabs?(tabs: SavedTab[]): Promise<void>;
	/** Listen for single-instance folder open requests (`quack://open-request`). */
	onOpenRequest?(handler: (targets: LaunchTarget[]) => void): () => void;
	/** Read shared workspace prompt input history (`history.jsonl`), oldest first. */
	inputHistory?(tab: number): Promise<string[]>;
	/** Reveal or open a file inside the active tab's workspace. */
	revealPath?(tab: number, path: string, openFile?: boolean): Promise<void>;
	/** Highlight a fenced code block using tree-sitter grammars on the backend. */
	highlightCode?(lang: string, code: string): Promise<HighlightSpan[] | null>;
	/** Open native image picker and return chosen local image paths. */
	pickImages?(): Promise<string[]>;
	/** Open native save dialog for exporting field notes. */
	pickSavePath?(defaultName: string): Promise<string | null>;
	/** Stage pasted image bytes to a validated temporary image file path. */
	stagePastedImage?(tab: number, bytes: Uint8Array): Promise<string>;
	/** Show an OS attention notification when enabled and unfocused. */
	notifyAttention?(title: string, body: string): Promise<void>;
}
