import type {
	ApprovalRequest,
	CommandInfo,
	ContextSegment,
	DetectedServer,
	HookConnectionDraft,
	HookSetupInfo,
	LspServerInfo,
	McpSecretValue,
	McpServerInfo,
	MemoryNote,
	Op,
	ProfileInfo,
	ServiceInfo,
	ProviderModels,
	Question,
	ReplayItem,
	RewindPoint,
	SessionSummary,
	SkillInfo,
	StatsReport,
	WriteScope
} from '#lib/protocol';
import type { AppInfo, ClientEvent, ClientOp, Transport } from '#lib/transport/types';
import type { HookRunEntry } from './extensions';
import { recordModelSpeed } from './modelStats';
import { formatTomlStringArray, type SettingsSectionId } from './settings';

/**
 * One tab's view of one engine (docs/plan/15-desktop-client.md#the-ui): a reducer from
 * protocol `Event`s to transcript items and header facts, plus the `Op` senders the UI
 * acts through. All engine traffic goes through the `Transport`; nothing here talks to
 * Tauri directly.
 */

export type Item =
	| { kind: 'user'; id: number; text: string; files: string[]; images: string[] }
	| { kind: 'assistant'; id: number; text: string; streaming: boolean }
	| { kind: 'reasoning'; id: number; text: string; streaming: boolean }
	| {
			kind: 'tool';
			id: number;
			callId: string;
			name: string;
			summary: string;
			ok: boolean | null;
			output: string;
			detail: string | null;
			open: boolean;
			/** Files the call's opening summary named; the end summary may reword it. */
			subjects: string[];
	  }
	| { kind: 'system'; id: number; tone: 'notice' | 'warning' | 'error'; text: string }
	| { kind: 'compacted'; id: number }
	| { kind: 'pruned'; id: number; removedTokens: number; items: string[] };

export interface AgentRun {
	id: string;
	agent: string;
	description: string;
	model: string;
	/** The sub-agent's last assistant text, for the pinned card. */
	text: string;
	toolCalls: number;
	ok: boolean | null;
	summary: string;
}

export interface ContextState {
	used: number;
	budget: number | null;
	window: number | null;
	threshold: number | null;
}

export interface SpeedState {
	ttftMs: number;
	tokensPerSec: number | null;
}

/** One file the session has written into, for the specimen case. */
export interface EditedFile {
	path: string;
	adds: number;
	dels: number;
	/** Write and edit calls that touched it. */
	ops: number;
	/** The latest diff, for the expanded view. */
	diff: string | null;
}

export type WorkMode = 'plan' | 'edit';
export type ApprovalPolicy = 'read-only' | 'ask' | 'auto-edit' | 'allow' | 'yolo';
export type JobIssue = { kind: 'error' | 'warning' | 'aborted'; message: string };

/** What the agent may do; independent of the approval policy. */
export const WORK_MODES: { id: WorkMode; label: string; about: string }[] = [
	{ id: 'edit', label: 'Edit', about: 'Edit files and run commands, within approvals' },
	{ id: 'plan', label: 'Plan', about: 'Read and search only; propose a plan for review' }
];

/** What runs without asking; independent of the work mode. Ask and Allow are the
 * offered choices; the rest are legacy policies that stay reachable for existing
 * configurations and show only while active. */
export const APPROVALS: {
	id: ApprovalPolicy;
	label: string;
	about: string;
	legacy?: boolean;
}[] = [
	{ id: 'ask', label: 'Ask', about: 'Ask before unapproved edits and commands' },
	{
		id: 'allow',
		label: 'Allow',
		about: 'Run without asking, inside the sandbox; Plan still only reads'
	},
	{
		id: 'yolo',
		label: 'Yolo',
		about: 'Run without asking and without the sandbox',
		legacy: true
	},
	{
		id: 'auto-edit',
		label: 'Auto-edit',
		about: 'Edit without asking; ask before commands (legacy)',
		legacy: true
	},
	{
		id: 'read-only',
		label: 'Read-only',
		about: 'Look around; change nothing (legacy)',
		legacy: true
	}
];

let itemSeq = 0;
const nextId = () => ++itemSeq;

/** File paths named by a unified diff's `+++`/`---` headers. */
/** A unified diff split into per-file sections, each with its own change counts.
 * Header pairs are read together so `--- a/x` + `+++ /dev/null` and the reverse
 * still name the file that changed. */
function diffSections(
	detail: string
): { path: string; adds: number; dels: number }[] {
	const sections: { path: string; adds: number; dels: number }[] = [];
	let current: { path: string; adds: number; dels: number } | null = null;
	const lines = detail.split('\n');
	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];
		const plus = /^\+\+\+ (?:b\/)?(.+)$/.exec(line);
		if (plus) {
			if (plus[1] !== '/dev/null') {
				current = { path: plus[1], adds: 0, dels: 0 };
				sections.push(current);
			} else current = null;
			continue;
		}
		const minus = /^--- (?:a\/)?(.+)$/.exec(line);
		if (minus) {
			const next = /^\+\+\+ (?:b\/)?(.+)$/.exec(lines[i + 1] ?? '');
			const path =
				next && next[1] !== '/dev/null'
					? next[1]
					: minus[1] !== '/dev/null'
						? minus[1]
						: null;
			if (path) {
				current = { path, adds: 0, dels: 0 };
				sections.push(current);
			} else current = null;
			if (next) i++;
			continue;
		}
		if (current) {
			if (line.startsWith('+')) current.adds += 1;
			else if (line.startsWith('-')) current.dels += 1;
		}
	}
	return sections;
}

/** The files an `edit src/x.rs` / `write docs.md` opening summary names. */
function summarySubjects(summary: string): string[] {
	const named = /^(?:edit|write|patch) (.+)$/.exec(summary.trim());
	return named && !named[1].endsWith(' files') ? [named[1]] : [];
}

export class Session {
	tab = $state(0);
	/** Real filesystem folder used to open/reopen this tab (never `~`-collapsed). */
	cwd = $state('');
	info = $state<AppInfo | null>(null);
	title = $state('');
	draft = $state('');
	items = $state<Item[]>([]);
	streaming = $state(false);
	turnPending = $state(false);
	speed = $state<SpeedState | null>(null);
	lastUsage = $state<{ prompt: number; completion: number; cached: number | null } | null>(null);
	approval = $state<{ callId: string; request: ApprovalRequest; always: string[] } | null>(null);
	questions = $state<{ callId: string; questions: Question[] } | null>(null);
	providers = $state<ProviderModels[]>([]);
	currentModel = $state<string | null>(null);
	/** Thinking level in effect for the current model, when it declares any. */
	reasoningEffort = $state<string | null>(null);
	/** Selectable thinking levels of the current model, in picker order. */
	reasoningEfforts = $state<string[]>([]);
	/** Open after a model switch lands: that switch asked for a level. */
	thinkingOpen = $state(false);
	/** The model a pending switch expects, so the picker opens only for it. */
	#pickThinkingFor = $state<string | null>(null);
	workMode = $state<WorkMode>('edit');
	approvals = $state<ApprovalPolicy>('ask');
	/** A subscription sign-in in flight: the service and the browser address. */
	signingIn = $state<{ id: string; url: string | null } | null>(null);
	signInError = $state<{ id: string; message: string } | null>(null);
	sandboxOff = $state(false);
	sandboxAvailable = $state(true);
	context = $state<ContextState | null>(null);
	contextReport = $state<{ model: string; segments: ContextSegment[]; note: string | null } | null>(
		null
	);
	plan = $state('');
	agents = $state<AgentRun[]>([]);
	sessions = $state<SessionSummary[]>([]);
	resumed = $state(false);
	starting = $state(true);
	startupDetail = $state<string | null>(null);
	closed = $state(false);
	closedReason = $state<string | null>(null);
	sessionId = $state<string | null>(null);
	switching = $state(false);
	sending = $state(false);
	actionError = $state<string | null>(null);
	issue = $state<JobIssue | null>(null);
	trust = $state<{ path: string; summary: string; changed: boolean } | null>(null);
	trustSaving = $state(false);
	setupOpen = $state(false);
	setupWorking = $state(false);
	setupNotice = $state('');
	services = $state<ServiceInfo[]>([]);
	detectedServers = $state<DetectedServer[] | null>(null);
	detecting = $state(false);
	/** Skills the engine loaded for this project; arrives at startup. */
	skills = $state<SkillInfo[]>([]);
	/** Custom slash commands loaded from command folders. */
	commands = $state<CommandInfo[]>([]);
	/** MCP prompt shortcuts (`/server:prompt`). */
	mcpPrompts = $state<CommandInfo[]>([]);
	mcpServers = $state<McpServerInfo[]>([]);
	lspServers = $state<LspServerInfo[]>([]);
	diagnostics = $state<{ errors: number; warnings: number }>({ errors: 0, warnings: 0 });
	/** Turn paused at `agent.max_steps`; awaiting `Op::ContinueTurn`. */
	stepLimit = $state<{ turn: number; limit: number } | null>(null);
	/** Turn-level rewind points fetched from the engine. */
	rewindPoints = $state<RewindPoint[] | null>(null);
	rewindLoading = $state(false);
	rewindConflict = $state<{ back: number; conversation: boolean; files: boolean; conflicts: string[] } | null>(null);
	forking = $state(false);
	/** Set when "Branch & try another model" asked to open the model picker after fork lands. */
	modelPickerAfterFork = $state(false);
	settingSaving = $state(false);
	settingsOpen = $state(false);
	settingsTab = $state<SettingsSectionId>('paths');
	settingFeedback = $state<{ key: string; value: string; path: string; note?: string } | null>(
		null
	);
	settingError = $state<string | null>(null);
	profiles = $state<{ active: string; forced: boolean; profiles: ProfileInfo[] } | null>(null);
	egressOpen = $state(false);
	workspaceFiles = $state<string[] | null>(null);
	loadingFiles = $state(false);
	/** The engine's `/stats` report, fetched after each turn. */
	stats = $state<StatsReport | null>(null);
	memoryNotes = $state<MemoryNote[]>([]);
	memoryLoaded = $state(false);
	memoryIndexFill = $state(0);
	memorySaving = $state(false);
	memoryError = $state<string | null>(null);
	hookRuns = $state<HookRunEntry[]>([]);
	hookPayloadChoice = $state<{ id: string; path: string } | null>(null);
	hookSaving = $state(false);
	hookFeedback = $state<string | null>(null);
	hookError = $state<string | null>(null);
	hookSetup = $state<HookSetupInfo | null>(null);
	hookSetupLoading = $state(false);
	hookSetupSaving = $state(false);
	hookSetupChecking = $state(false);
	hookSetupPendingRequest = $state<number | null>(null);
	hookSetupStatus = $state<{ kind: 'ok' | 'fail' | 'notice'; text: string } | null>(null);
	hookSetupError = $state<string | null>(null);
	mcpSaving = $state(false);
	mcpFeedback = $state<string | null>(null);
	mcpError = $state<string | null>(null);
	inputHistory = $state<string[]>([]);

	get canSubmit() {
		return (
			!this.closed &&
			!this.starting &&
			!this.switching &&
			!this.sending &&
			!this.setupWorking &&
			!this.streaming &&
			!this.turnPending &&
			!this.approval &&
			!this.questions &&
			!this.stepLimit &&
			!this.trust &&
			!!this.currentModel
		);
	}

	#transport: Transport;
	#progressSinceWarning = false;
	#pendingForkOpenPicker = false;
	#reloadAfterSettingSaved = false;
	#nextHookSetupRequest = 1;
	#activeHookInspectRequest: number | null = null;
	#activeHookSaveRequest: number | null = null;
	#activeHookCheckRequest: number | null = null;
	#lastRewindAttempt: { back: number; conversation: boolean; files: boolean } | null = null;

	constructor(transport: Transport, tab: number, info: AppInfo, cwd?: string) {
		this.#transport = transport;
		this.tab = tab;
		this.info = info;
		this.cwd = cwd || info.cwd;
		this.currentModel = info.model;
		this.approvals = info.permission_mode as ApprovalPolicy;
		this.workMode = info.work_mode === 'plan' ? 'plan' : 'edit';
		this.sandboxOff = info.sandbox.mode === 'off';
		this.sandboxAvailable = info.sandbox.os_supported;
		this.inputHistory = [...(info.history ?? [])];
	}

	onInfo(info: AppInfo) {
		this.info = info;
		this.sandboxOff = info.sandbox.mode === 'off';
		this.sandboxAvailable = info.sandbox.os_supported;
		if (info.history) this.inputHistory = [...info.history];
	}

	/** Feed one protocol event; the only place transcript state is written. */
	onEvent(ev: ClientEvent) {
		switch (ev.event) {
			case 'session_configured': {
				const wasForking = this.forking;
				if (this.sessionId !== null && this.sessionId !== ev.session.id && !wasForking) {
					this.#resetConversation();
				}
				this.forking = false;
				this.sessionId = ev.session.id;
				this.switching = false;
				this.currentModel = ev.session.model ?? null;
				this.reasoningEffort = ev.session.reasoning_effort ?? null;
				this.reasoningEfforts = ev.session.reasoning_efforts ?? [];
				if (this.currentModel) {
					this.setupWorking = false;
					this.setupOpen = false;
				}
				if (wasForking) {
					void this.listSessions();
					if (this.#pendingForkOpenPicker) {
						this.#pendingForkOpenPicker = false;
						this.modelPickerAfterFork = true;
					}
				}
				// Switching to a model that declares thinking levels asks for one: the level
				// changes both how the model works and what it costs.
				if (
					this.#pickThinkingFor !== null &&
					this.#pickThinkingFor === this.currentModel &&
					this.reasoningEfforts.length > 0
				) {
					this.thinkingOpen = true;
				}
				this.#pickThinkingFor = null;
				break;
			}
			case 'startup_progress':
				if (ev.state.state === 'running') {
					this.startupDetail = ev.state.detail || ev.subsystem;
				} else if (ev.state.state === 'failed') {
					this.startupDetail = `${ev.subsystem} failed`;
					this.issue = { kind: 'error', message: `${ev.subsystem}: ${ev.state.error}` };
				} else if (ev.state.state === 'needs_attention' && this.issue?.kind !== 'error') {
					this.issue = {
						kind: 'warning',
						message: ev.state.detail || `${ev.subsystem} needs attention`
					};
				}
				break;
			case 'startup_complete':
				this.starting = false;
				this.startupDetail = null;
				this.trustSaving = false;
				if (this.settingsOpen && this.settingsTab === 'extensions') void this.refreshMcp();
				if (!this.currentModel) void this.openSetup();
				break;
			case 'config_reloaded':
				this.trust = null;
				this.starting = true;
				this.startupDetail = null;
				this.issue = null;
				void this.listMemory();
				break;
			case 'project_trust':
				this.trust = { path: ev.path, summary: ev.summary, changed: ev.changed };
				break;
			case 'session_resumed':
				this.#resetConversation();
				this.sessionId = ev.session.id;
				this.switching = false;
				this.resumed = true;
				for (const item of ev.items) this.#pushReplay(item);
				break;
			case 'session_title':
				this.title = ev.title ?? '';
				break;
			case 'assistant_delta':
				this.#progressSinceWarning = true;
				this.#appendStream('assistant', ev.text);
				break;
			case 'assistant_rewrite': {
				const open = this.#openStreaming('assistant');
				if (open && open.kind === 'assistant') open.text = ev.text;
				break;
			}
			case 'reasoning_delta':
				this.#appendStream('reasoning', ev.text);
				break;
			case 'tool_call_begin':
				this.#progressSinceWarning = true;
				this.#sealStreams();
				this.items.push({
					kind: 'tool',
					id: nextId(),
					callId: ev.call.call_id,
					name: ev.call.name,
					summary: ev.call.summary,
					ok: null,
					output: '',
					detail: null,
					open: false,
					subjects: summarySubjects(ev.call.summary)
				});
				break;
			case 'tool_call_output_delta': {
				const tool = this.items.findLast((i) => i.kind === 'tool' && i.callId === ev.call_id) as
					Extract<Item, { kind: 'tool' }> | undefined;
				if (tool) tool.output += ev.chunk;
				break;
			}
			case 'tool_call_end': {
				const tool = this.items.findLast((i) => i.kind === 'tool' && i.callId === ev.call_id) as
					Extract<Item, { kind: 'tool' }> | undefined;
				if (tool) {
					tool.ok = ev.result.ok;
					tool.summary = ev.result.summary || tool.summary;
					tool.detail = ev.result.detail ?? null;
				}
				break;
			}
			case 'approval_request':
				this.approval = { callId: ev.call_id, request: ev.request, always: ev.always ?? [] };
				break;
			case 'questions':
				this.questions = { callId: ev.call_id, questions: ev.questions };
				break;
			case 'turn_started':
				this.turnPending = false;
				this.streaming = true;
				this.issue = null;
				this.actionError = null;
				this.#progressSinceWarning = false;
				break;
			case 'turn_complete':
				this.turnPending = false;
				this.streaming = false;
				this.#sealStreams();
				this.approval = null;
				this.questions = null;
				this.stepLimit = null;
				this.workspaceFiles = null;
				if (this.issue?.kind === 'warning' && this.#progressSinceWarning) this.issue = null;
				this.lastUsage = {
					prompt: ev.usage.prompt_tokens,
					completion: ev.usage.completion_tokens,
					cached: ev.usage.cached_prompt_tokens ?? null
				};
				// Usage and context re-read after every turn.
				void this.#send({ op: 'get_stats' });
				void this.refreshContext();
				void this.listMemory();
				break;
			case 'step_limit_reached':
				this.turnPending = false;
				this.streaming = false;
				this.#sealStreams();
				this.approval = null;
				this.questions = null;
				this.stepLimit = { turn: ev.turn, limit: ev.limit };
				break;
			case 'turn_aborted':
				this.turnPending = false;
				this.streaming = false;
				this.#sealStreams();
				this.approval = null;
				this.questions = null;
				this.stepLimit = null;
				if (this.issue?.kind !== 'error') this.issue = { kind: 'aborted', message: ev.reason };
				this.items.push({
					kind: 'system',
					id: nextId(),
					tone: 'warning',
					text: `Interrupted: ${ev.reason}`
				});
				break;
			case 'speed':
				this.speed = { ttftMs: ev.ttft_ms, tokensPerSec: ev.tokens_per_sec ?? null };
				if (this.currentModel) {
					recordModelSpeed(this.currentModel, ev.ttft_ms, ev.tokens_per_sec ?? null);
				}
				break;
			case 'context_update':
				this.context = {
					used: ev.used,
					budget: ev.budget ?? null,
					window: ev.window ?? null,
					threshold: ev.threshold ?? null
				};
				break;
			case 'context_report':
				this.contextReport = {
					model: ev.report.model,
					segments: ev.report.segments,
					note: ev.report.note ?? null
				};
				break;
			case 'plan':
				this.plan = ev.checklist;
				break;
			case 'mode_changed':
				// Authoritative for both controls; no need to remember a previous mode.
				this.workMode = ev.work_mode;
				this.approvals = ev.approvals;
				break;
			case 'permission_mode':
				// Legacy single-control notification: `plan` named the work mode.
				if (ev.mode === 'plan') this.workMode = 'plan';
				else {
					this.workMode = 'edit';
					this.approvals = ev.mode as ApprovalPolicy;
				}
				break;
			case 'sign_in_url':
				if (this.signingIn?.id === ev.id) {
					this.signingIn = { id: ev.id, url: ev.url };
					void this.#transport.openUrl(ev.url);
				}
				break;
			case 'sign_in_result':
				if (this.signingIn?.id !== ev.id) break;
				this.signingIn = null;
				if (ev.error) {
					this.signInError = { id: ev.id, message: ev.error };
					this.items.push({
						kind: 'system',
					id: nextId(),
					tone: 'warning',
					text: `Sign-in failed: ${ev.error}`
					});
				} else {
					this.signInError = null;
					this.setupNotice = `Signed in to ${ev.id}. Choose a model to continue.`;
					void this.#send({ op: 'list_services' });
					void this.#send({ op: 'list_models' });
				}
				break;
			case 'sandbox':
				this.sandboxOff = ev.off;
				break;
			case 'model_catalog':
				// Key/URL setup returns only that provider; retain the other entries.
				this.providers = [
					...this.providers.filter((p) => !ev.providers.some((next) => next.id === p.id)),
					...ev.providers
				];
				this.setupWorking = false;
				break;
			case 'services':
				this.services = ev.services;
				break;
			case 'servers_detected':
				this.detectedServers = ev.servers;
				this.detecting = false;
				break;
			case 'skills':
				this.skills = ev.skills;
				break;
			case 'commands':
				this.commands = ev.commands;
				break;
			case 'mcp_prompts':
				this.mcpPrompts = ev.prompts;
				break;
			case 'mcp_servers':
				this.mcpServers = ev.servers;
				break;
			case 'lsp_servers':
				this.lspServers = ev.servers;
				break;
			case 'diagnostics':
				this.diagnostics = { errors: ev.errors, warnings: ev.warnings };
				break;
			case 'context_pruned':
				this.#sealStreams();
				this.items.push({
					kind: 'pruned',
					id: nextId(),
					removedTokens: ev.report.removed_tokens,
					items: ev.report.items
				});
				break;
			case 'setting_saved':
				this.settingSaving = false;
				this.settingError = null;
				this.settingFeedback = {
					key: ev.key,
					value: ev.value,
					path: ev.path,
					...(ev.note ? { note: ev.note } : {})
				};
				if (this.#reloadAfterSettingSaved) {
					this.#reloadAfterSettingSaved = false;
					void this.#send({ op: 'reload_config' });
				}
				break;
			case 'profiles':
				this.profiles = {
					active: ev.active,
					forced: ev.forced,
					profiles: ev.profiles
				};
				break;
			case 'stats':
				this.stats = ev.report;
				break;
			case 'provider_added':
				this.setupNotice = ev.note ?? 'Provider saved. Choose a model to continue.';
				break;
			case 'session_list':
				this.sessions = ev.sessions;
				break;
			case 'agent_started':
				this.#progressSinceWarning = true;
				this.agents.push({
					id: ev.id,
					agent: ev.agent,
					description: ev.description,
					model: ev.model,
					text: '',
					toolCalls: 0,
					ok: null,
					summary: ''
				});
				break;
			case 'agent': {
				this.#progressSinceWarning = true;
				const run = this.agents.findLast((a) => a.id === ev.id);
				if (run) {
					if (ev.inner.event === 'assistant_delta') run.text += ev.inner.text;
					else if (ev.inner.event === 'tool_call_begin') run.toolCalls += 1;
				}
				break;
			}
			case 'agent_finished': {
				const run = this.agents.findLast((a) => a.id === ev.id);
				if (run) {
					run.ok = ev.ok;
					run.summary = ev.summary;
				}
				break;
			}
			case 'compacted':
				this.#sealStreams();
				this.items.push({ kind: 'compacted', id: nextId() });
				break;
			case 'rewind_points':
				this.rewindLoading = false;
				this.rewindPoints = ev.points;
				break;
			case 'files_restored':
				if (ev.conflicts.length > 0 && ev.restored.length === 0) {
					if (this.#lastRewindAttempt) {
						this.rewindConflict = { ...this.#lastRewindAttempt, conflicts: ev.conflicts };
					}
				} else if (ev.restored.length > 0) {
					this.rewindConflict = null;
					this.items.push({
						kind: 'system',
						id: nextId(),
						tone: 'notice',
						text: `Restored ${ev.restored.length} file${ev.restored.length === 1 ? '' : 's'}: ${ev.restored.join(', ')}`
					});
				}
				break;
			case 'rewound': {
				this.#sealStreams();
				this.rewindConflict = null;
				this.rewindPoints = null;
				this.rewindLoading = false;
				let seenUsers = 0;
				let cutIndex = -1;
				for (let i = this.items.length - 1; i >= 0; i--) {
					if (this.items[i].kind === 'user') {
						seenUsers += 1;
						if (seenUsers === ev.back) {
							cutIndex = i;
							break;
						}
					}
				}
				if (cutIndex >= 0) {
					this.items = this.items.slice(0, cutIndex);
				}
				if (!this.draft.trim() && ev.text) {
					this.draft = ev.text;
				}
				this.items.push({
					kind: 'system',
					id: nextId(),
					tone: 'notice',
					text: `Rewound ${ev.back} turn${ev.back === 1 ? '' : 's'} — edit the prompt below or branch cleanly.`
				});
				void this.refreshContext();
				void this.refreshStats();
				break;
			}
			case 'memory_notes':
				this.memoryNotes = ev.notes;
				this.memoryIndexFill = ev.index_fill;
				this.memoryLoaded = true;
				this.memorySaving = false;
				this.memoryError = null;
				break;
			case 'hook_ran':
				this.hookRuns = [
					{
						id: ev.id,
						on: ev.on,
						outcome: ev.outcome,
						ms: ev.ms,
						...(ev.message ? { message: ev.message } : {}),
						contextTokens: ev.context_tokens ?? 0,
						at: Date.now()
					},
					...this.hookRuns
				].slice(0, 25);
				break;
			case 'hook_payload_choice':
				this.hookSaving = false;
				this.hookPayloadChoice = { id: ev.id, path: ev.path };
				break;
			case 'hook_enabled':
				this.hookSaving = false;
				this.hookError = null;
				this.hookPayloadChoice = null;
				this.hookFeedback = ev.note
					? `Enabled ${ev.id} (${ev.path}): ${ev.note}`
					: `Enabled hook ${ev.id} (${ev.path}).`;
				break;
			case 'mcp_enabled':
				this.mcpSaving = false;
				this.mcpError = null;
				this.mcpFeedback = `Enabled MCP server ${ev.id} (${ev.path}).`;
				break;
			case 'mcp_server_saved':
				this.mcpSaving = false;
				this.mcpError = null;
				this.mcpFeedback = `Saved MCP server ${ev.id} (${ev.path}).`;
				break;
			case 'hook_setup':
				if (this.#activeHookInspectRequest === ev.request) {
					this.#activeHookInspectRequest = null;
					this.hookSetupLoading = false;
					this.hookSetupError = null;
					this.hookSetupStatus = null;
					this.hookSetup = ev.info;
				}
				break;
			case 'hook_setup_saved':
				if (this.#activeHookSaveRequest === ev.request) {
					this.#activeHookSaveRequest = null;
					this.hookSetupSaving = false;
					this.hookSetupPendingRequest = null;
					this.hookSetupError = null;
					this.hookSetup = ev.info;
					this.hookSetupStatus = {
						kind: 'ok',
						text: ev.note || `Saved connections for ${ev.info.id}.`
					};
				}
				break;
			case 'hook_setup_pending':
				if (this.#activeHookSaveRequest === ev.request) {
					this.hookSetupPendingRequest = ev.request;
					this.hookSetupStatus = {
						kind: 'notice',
						text: 'Saved to file; finishing turn before reloading hook configuration…'
					};
				}
				break;
			case 'hook_setup_failed':
				if (this.#activeHookInspectRequest === ev.request) {
					this.#activeHookInspectRequest = null;
					this.hookSetupLoading = false;
					this.hookSetupError = ev.message;
				} else if (this.#activeHookSaveRequest === ev.request) {
					this.#activeHookSaveRequest = null;
					this.hookSetupSaving = false;
					this.hookSetupPendingRequest = null;
					this.hookSetupError = ev.message;
				} else if (this.#activeHookCheckRequest === ev.request) {
					this.#activeHookCheckRequest = null;
					this.hookSetupChecking = false;
					this.hookSetupError = ev.message;
				}
				break;
			case 'hook_check_result':
				if (this.#activeHookCheckRequest === ev.request) {
					this.#activeHookCheckRequest = null;
					this.hookSetupChecking = false;
					this.hookSetupStatus = {
						kind: ev.status === 'ok' ? 'ok' : ev.status === 'cancelled' ? 'notice' : 'fail',
						text: ev.message
					};
				}
				break;
			case 'notice':
				this.#sealStreams();
				this.items.push({ kind: 'system', id: nextId(), tone: 'notice', text: ev.message });
				break;
			case 'warning':
				this.#sealStreams();
				this.turnPending = false;
				this.settingSaving = false;
				this.hookSaving = false;
				this.mcpSaving = false;
				this.hookSetupLoading = false;
				this.hookSetupSaving = false;
				this.hookSetupChecking = false;
				this.hookSetupPendingRequest = null;
				this.rewindLoading = false;
				this.forking = false;
				if (this.memorySaving) {
					this.memorySaving = false;
					this.memoryError = ev.message;
				}
				this.#reloadAfterSettingSaved = false;
				this.#progressSinceWarning = false;
				if (this.switching || this.setupWorking) this.actionError = ev.message;
				if (this.issue?.kind !== 'error') this.issue = { kind: 'warning', message: ev.message };
				this.switching = false;
				this.setupWorking = false;
				this.items.push({ kind: 'system', id: nextId(), tone: 'warning', text: ev.message });
				break;
			case 'error': {
				const wasMcpSaving = this.mcpSaving;
				const wasHookSaving = this.hookSaving;
				this.#sealStreams();
				this.turnPending = false;
				if (this.settingSaving || ev.error.kind === 'config') {
					this.settingError = ev.error.message;
				}
				if (wasMcpSaving) this.mcpError = ev.error.message;
				if (wasHookSaving) this.hookError = ev.error.message;
				this.settingSaving = false;
				this.hookSaving = false;
				this.mcpSaving = false;
				this.rewindLoading = false;
				this.forking = false;
				this.memorySaving = false;
				this.#reloadAfterSettingSaved = false;
				this.actionError = ev.error.message;
				this.issue = { kind: 'error', message: ev.error.message };
				this.switching = false;
				this.setupWorking = false;
				this.trustSaving = false;
				this.items.push({
					kind: 'system',
					id: nextId(),
					tone: 'error',
					text: ev.error.message
				});
				this.streaming = false;
				break;
			}
			default:
				// Facts the header does not show yet (skills, hooks, LSP, doctor, stats…)
				// arrive here until their surfaces land (G2/G3).
				break;
		}
	}

	onClosed(reason: string | null) {
		this.closed = true;
		this.closedReason = reason;
		this.turnPending = false;
		this.switching = false;
		this.setupWorking = false;
		this.trustSaving = false;
		this.detecting = false;
		this.starting = false;
		this.startupDetail = null;
		this.streaming = false;
		this.signingIn = null;
		this.stepLimit = null;
		this.rewindPoints = null;
		this.rewindLoading = false;
		this.rewindConflict = null;
		this.forking = false;
		this.settingSaving = false;
		this.hookSaving = false;
		this.mcpSaving = false;
		this.hookSetupLoading = false;
		this.hookSetupSaving = false;
		this.hookSetupChecking = false;
		this.hookSetupPendingRequest = null;
		this.memorySaving = false;
		this.#sealStreams();
		this.approval = null;
		this.questions = null;
		if (reason) this.items.push({ kind: 'system', id: nextId(), tone: 'error', text: reason });
	}

	dismissIssue() {
		this.actionError = null;
		this.issue = null;
		this.signInError = null;
	}

	#resetConversation() {
		this.items = [];
		this.draft = '';
		this.plan = '';
		this.agents = [];
		this.context = null;
		this.contextReport = null;
		this.speed = null;
		this.lastUsage = null;
		this.stats = null;
		this.approval = null;
		this.questions = null;
		this.stepLimit = null;
		this.rewindPoints = null;
		this.rewindLoading = false;
		this.rewindConflict = null;
		this.resumed = false;
		this.issue = null;
		this.actionError = null;
		this.signInError = null;
		this.#progressSinceWarning = false;
	}

	#pushReplay(item: ReplayItem) {
		if (item.kind === 'user') {
			this.items.push({ kind: 'user', id: nextId(), text: item.text, files: [], images: [] });
		} else if (item.kind === 'assistant') {
			this.items.push({ kind: 'assistant', id: nextId(), text: item.text, streaming: false });
		} else if (item.kind === 'reasoning') {
			this.items.push({ kind: 'reasoning', id: nextId(), text: item.text, streaming: false });
		} else if (item.kind === 'tool') {
			this.items.push({
				kind: 'tool',
				id: nextId(),
				callId: '',
				name: '',
				summary: item.summary,
				ok: item.ok,
				output: item.result,
				detail: item.detail ?? null,
				open: false,
				subjects: summarySubjects(item.summary)
			});
		} else {
			this.items.push({ kind: 'compacted', id: nextId() });
		}
	}

	#appendStream(kind: 'assistant' | 'reasoning', text: string) {
		const open = this.#openStreaming(kind);
		if (open) open.text += text;
		else {
			this.#sealStreams();
			this.items.push({ kind, id: nextId(), text, streaming: true });
		}
	}

	#openStreaming(
		kind: 'assistant' | 'reasoning'
	): (Extract<Item, { kind: 'assistant' }> | Extract<Item, { kind: 'reasoning' }>) | undefined {
		const last = this.items[this.items.length - 1];
		return last && last.kind === kind && last.streaming ? last : undefined;
	}

	#sealStreams() {
		const last = this.items[this.items.length - 1];
		if (last && (last.kind === 'assistant' || last.kind === 'reasoning') && last.streaming) {
			last.streaming = false;
		}
	}

	// ---- actions ----

	/** IPC success means queued; engine events acknowledge the resulting state. */
	async #send(op: ClientOp): Promise<boolean> {
		if (this.closed) {
			this.actionError = 'This workspace has stopped · close and reopen the folder.';
			return false;
		}
		if (!op.op.startsWith('list_') && !op.op.startsWith('get_')) this.actionError = null;
		try {
			await this.#transport.sendOp(this.tab, op);
			return true;
		} catch (error) {
			this.actionError = `${error instanceof Error ? error.message : String(error)} · try again.`;
			return false;
		}
	}

	async submit(text: string, files: string[] = [], images: string[] = []) {
		if (!this.canSubmit) return false;
		const cleanText = text.trim();
		if (!cleanText && files.length === 0 && images.length === 0) return false;
		this.sending = true;
		this.#sealStreams();
		this.turnPending = true;
		const id = nextId();
		this.items.push({ kind: 'user', id, text: cleanText || text, files, images });
		// Core ignores InputItem::File; inject `@path` mentions into InputItem::Text cleanly.
		const extraRefs = files.filter((f) => !text.includes(`@${f}`)).map((f) => `@${f}`);
		const wireText = [cleanText, extraRefs.join(' ')].filter(Boolean).join('\n\n');
		const sent = await this.#send({
			op: 'user_turn',
			items: [
				...(wireText ? [{ type: 'text' as const, text: wireText }] : []),
				...images.map((path) => ({ type: 'image' as const, path }))
			],
			overrides: {}
		});
		if (!sent) {
			this.items = this.items.filter((item) => item.id !== id);
			this.turnPending = false;
		} else if (wireText.trim() && this.inputHistory.at(-1) !== wireText) {
			this.inputHistory = [...this.inputHistory, wireText].slice(-100);
		}
		this.sending = false;
		return sent;
	}

	async interrupt() {
		return this.#send({ op: 'interrupt' });
	}

	async continueTurn(proceed: boolean) {
		const pending = this.stepLimit;
		if (!pending || this.sending) return false;
		this.sending = true;
		const sent = await this.#send({ op: 'continue_turn', turn: pending.turn, proceed });
		if (sent && this.stepLimit?.turn === pending.turn) {
			this.stepLimit = null;
			if (proceed) this.streaming = true;
		}
		this.sending = false;
		return sent;
	}

	async answerApproval(
		decision: 'allow_once' | 'allow_session' | 'always_allow' | 'deny',
		feedback?: string,
		scope: WriteScope = 'project'
	) {
		const callId = this.approval?.callId;
		if (!callId || this.sending) return false;
		this.sending = true;
		const wireDecision: import('#lib/protocol').ApprovalDecision =
			decision === 'deny' && feedback?.trim()
				? { decision: 'deny_with_feedback', feedback: feedback.trim() }
				: decision === 'always_allow'
					? { decision: 'always_allow', scope }
					: { decision };
		const sent = await this.#send({
			op: 'approval_response',
			call_id: callId,
			decision: wireDecision
		});
		if (sent && this.approval?.callId === callId) this.approval = null;
		this.sending = false;
		return sent;
	}

	async answerQuestions(answers: { picked?: number; text?: string; skipped?: boolean }[]) {
		const callId = this.questions?.callId;
		if (!callId || this.sending) return false;
		this.sending = true;
		const sent = await this.#send({
			op: 'answer_questions',
			call_id: callId,
			answers: answers.map((a) =>
				a.skipped
					? { answer: 'skipped' }
					: a.picked !== undefined
						? { answer: 'picked', index: a.picked }
						: { answer: 'text', text: a.text ?? '' }
			),
			stop_asking: false
		});
		if (sent && this.questions?.callId === callId) this.questions = null;
		this.sending = false;
		return sent;
	}

	async setModel(providerModel: string) {
		if (this.setupWorking || this.switching || this.streaming || this.turnPending) return false;
		const slash = providerModel.indexOf('/');
		if (slash < 1 || slash === providerModel.length - 1) return false;
		this.setupWorking = true;
		// A model that declares thinking levels asks for one once the switch lands.
		const provider = this.providers.find((p) => p.id === providerModel.slice(0, slash));
		const model = provider?.models.find((m) => m.id === providerModel.slice(slash + 1));
		if (model?.reasoning_efforts?.length) this.#pickThinkingFor = providerModel;
		const sent = await this.#send({
			op: 'set_model',
			provider: providerModel.slice(0, slash),
			model: providerModel.slice(slash + 1)
		});
		if (!sent) {
			this.setupWorking = false;
			this.#pickThinkingFor = null;
		}
		return sent;
	}

	/** Picks a thinking level for the current model; `null` restores its default. */
	async setReasoningEffort(effort: string | null) {
		this.thinkingOpen = false;
		return this.#send({ op: 'set_reasoning_effort', effort });
	}

	async setWorkMode(mode: WorkMode) {
		return this.#send({ op: 'set_work_mode', mode });
	}
	async setApprovals(policy: ApprovalPolicy) {
		return this.#send({ op: 'set_approval_policy', policy });
	}
	async acceptPlan() {
		// An empty mode keeps the current approvals; the engine performs the handoff.
		return this.#send({ op: 'accept_plan', mode: '' });
	}
	/** Starts a browser subscription sign-in, or cancels the pending one with `null`. */
	async signIn(id: string | null) {
		const before = this.signingIn;
		if (id) {
			this.signInError = null;
			this.signingIn = { id, url: null };
		} else {
			this.signingIn = null; // the engine aborts silently on cancel
		}
		const sent = await this.#send({ op: 'sign_in', id });
		if (!sent) this.signingIn = before;
		return sent;
	}
	/** Reopens the browser page of a pending sign-in. */
	async reopenSignIn() {
		const url = this.signingIn?.url;
		if (url) await this.#transport.openUrl(url);
	}
	async refreshContext() {
		return this.#send({ op: 'get_context' });
	}
	async refreshStats() {
		return this.#send({ op: 'get_stats' });
	}
	async compact(focus?: string) {
		return this.#send({ op: 'compact', ...(focus ? { focus } : {}) });
	}
	async listModels() {
		return this.#send({ op: 'list_models' });
	}
	async listSessions() {
		return this.#send({ op: 'list_sessions' });
	}

	async newSession() {
		return this.#switchSession({ op: 'new_session' });
	}
	async resumeSession(id: string) {
		if (id === this.sessionId) return false;
		return this.#switchSession({ op: 'resume_session', id });
	}
	async #switchSession(op: Op) {
		if (
			this.switching ||
			this.setupWorking ||
			this.streaming ||
			this.turnPending ||
			this.starting ||
			this.trust ||
			!this.currentModel
		)
			return false;
		this.switching = true;
		if (!(await this.#send(op))) {
			this.switching = false;
			return false;
		}
		await this.listSessions();
		return true;
	}
	async undo() {
		return this.#send({ op: 'undo', checkpoint: null, force: false });
	}

	async listRewindPoints() {
		if (this.rewindLoading) return false;
		this.rewindLoading = true;
		this.rewindConflict = null;
		const sent = await this.#send({ op: 'list_rewind_points' });
		if (!sent) this.rewindLoading = false;
		return sent;
	}

	async rewind(back: number, conversation = true, files = true, force = false) {
		if (this.streaming || this.turnPending) return false;
		this.#lastRewindAttempt = { back, conversation, files };
		this.rewindConflict = null;
		return this.#send({ op: 'rewind', back, conversation, files, force });
	}

	clearRewind() {
		this.rewindPoints = null;
		this.rewindLoading = false;
		this.rewindConflict = null;
		this.#lastRewindAttempt = null;
	}

	async forkSession(openModelPicker = false) {
		if (this.forking || this.switching || this.streaming || this.turnPending || this.starting) {
			return false;
		}
		this.forking = true;
		this.#pendingForkOpenPicker = openModelPicker;
		const sent = await this.#send({ op: 'fork_session' });
		if (!sent) {
			this.forking = false;
			this.#pendingForkOpenPicker = false;
		}
		return sent;
	}

	async exportTranscript(format: 'markdown' | 'json' = 'markdown') {
		const ext = format === 'json' ? 'json' : 'md';
		const slug = (this.title || 'session').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '') || 'session';
		let path: string | undefined;
		if (this.#transport.pickSavePath) {
			const picked = await this.#transport.pickSavePath(`quack-${slug}.${ext}`);
			if (!picked) return false;
			path = picked;
		}
		return this.#send({ op: 'export', format, ...(path ? { path } : {}) });
	}

	async setSandbox(off: boolean) {
		return this.#send({ op: 'set_sandbox', off });
	}

	async saveSetting(
		key: string,
		value: string,
		scope: WriteScope = 'user',
		reload = true
	) {
		if (this.settingSaving || this.streaming || this.turnPending) return false;
		this.settingSaving = true;
		this.settingError = null;
		this.settingFeedback = null;
		this.#reloadAfterSettingSaved = reload;
		const sent = await this.#send({ op: 'save_setting', key, value, scope });
		if (!sent) {
			this.settingSaving = false;
			this.#reloadAfterSettingSaved = false;
		}
		return sent;
	}

	async setPlannerModel(providerModel: string | null, scope: WriteScope = 'user') {
		return this.saveSetting(
			'model.planner',
			providerModel ? JSON.stringify(providerModel) : '""',
			scope,
			true
		);
	}

	async reloadConfig() {
		if (this.closed || this.streaming || this.turnPending) return false;
		this.settingError = null;
		return this.#send({ op: 'reload_config' });
	}

	async listProfiles() {
		if (this.closed) return false;
		return this.#send({ op: 'list_profiles' });
	}

	async setProfile(id: string) {
		if (this.closed || this.streaming || this.turnPending) return false;
		return this.#send({ op: 'set_profile', id });
	}

	async pickDirectory(): Promise<string | null> {
		return this.#transport.pickFolder();
	}

	clearSettingStatus() {
		this.settingError = null;
		this.settingFeedback = null;
	}

	async refreshMcp() {
		return this.#send({ op: 'list_mcp' });
	}

	async refreshLsp() {
		return this.#send({ op: 'list_lsp' });
	}

	async refreshWorkspaceFiles(force = false) {
		if (!this.#transport.listFiles) return [];
		if (!force && this.workspaceFiles !== null) return this.workspaceFiles;
		if (this.loadingFiles) return this.workspaceFiles ?? [];
		this.loadingFiles = true;
		try {
			const files = await this.#transport.listFiles(this.tab, 5000);
			this.workspaceFiles = files;
			return files;
		} catch {
			return this.workspaceFiles ?? [];
		} finally {
			this.loadingFiles = false;
		}
	}

	async revealFile(path: string, openFile = false) {
		if (!this.#transport.revealPath) return;
		try {
			await this.#transport.revealPath(this.tab, path, openFile);
		} catch (error) {
			this.actionError = error instanceof Error ? error.message : String(error);
		}
	}

	/** Files this session has written into, newest evidence last. Derived from the
	 * transcript's edit and write calls, so it needs no engine round trip. */
	get editedFiles(): EditedFile[] {
		const map = new Map<string, EditedFile>();
		for (const item of this.items) {
			if (item.kind !== 'tool') continue;
			const detail = item.detail ?? '';
			// A real diff attributes each hunk to its own file; anything else falls
			// back to the file the opening summary named, with no counts claimed.
			const byPath = new Map<string, { path: string; adds: number; dels: number }>();
			for (const section of diffSections(detail)) {
				const seen = byPath.get(section.path);
				if (seen) {
					seen.adds += section.adds;
					seen.dels += section.dels;
				} else byPath.set(section.path, section);
			}
			const hits =
				byPath.size > 0
					? [...byPath.values()]
					: [...item.subjects, ...summarySubjects(item.summary)].map((path) => ({
							path,
							adds: 0,
							dels: 0
						}));
			for (const hit of hits) {
				const entry = map.get(hit.path) ?? { path: hit.path, adds: 0, dels: 0, ops: 0, diff: null };
				entry.adds += hit.adds;
				entry.dels += hit.dels;
				entry.ops += 1;
				if (detail) entry.diff = detail;
				map.set(hit.path, entry);
			}
		}
		return [...map.values()];
	}

	async trustProject(decision: 'trusted' | 'settings-only' | 'ignored') {
		if (this.trustSaving) return false;
		this.trustSaving = true;
		const sent = await this.#send({ op: 'trust_project', decision });
		if (!sent) this.trustSaving = false;
		return sent;
	}

	async openSetup() {
		this.settingsOpen = false;
		this.setupOpen = true;
		this.setupNotice = '';
		await this.#send({ op: 'list_services' });
	}

	openSettings(tab?: SettingsSectionId) {
		if (tab) this.settingsTab = tab;
		this.setupOpen = false;
		this.settingsOpen = true;
		this.settingError = null;
		if (!this.profiles) void this.listProfiles();
		if (this.settingsTab === 'extensions') void this.refreshMcp();
	}

	selectSettingsTab(tab: SettingsSectionId) {
		this.settingsTab = tab;
		if (tab === 'extensions') void this.refreshMcp();
	}

	closeSettings() {
		this.settingsOpen = false;
		this.settingError = null;
	}
	async detectServers() {
		if (this.detecting) return;
		this.detecting = true;
		if (!(await this.#send({ op: 'detect_servers' }))) this.detecting = false;
	}
	async connectServer(baseUrl: string, key: string) {
		return this.#setup({ op: 'add_server', base_url: baseUrl.trim(), ...(key ? { key } : {}) });
	}
	async saveProviderKey(id: string, key: string) {
		return this.#setup({ op: 'set_provider_key', id, key });
	}
	async useDetectedServer(server: DetectedServer, model: string) {
		return this.#setup({
			op: 'add_provider',
			id: server.id,
			base_url: server.base_url,
			api: server.api,
			model
		});
	}
	async #setup(op: Op) {
		if (this.setupWorking) return false;
		this.setupWorking = true;
		this.setupNotice = '';
		const sent = await this.#send(op);
		if (!sent) this.setupWorking = false;
		return sent;
	}

	setInputHistory(history: string[]) {
		this.inputHistory = [...history];
	}

	async refreshInputHistory() {
		if (this.closed || !this.#transport.inputHistory) return false;
		try {
			const history = await this.#transport.inputHistory(this.tab);
			if (!this.closed) this.inputHistory = history;
			return true;
		} catch {
			return false;
		}
	}

	async listMemory() {
		return this.#send({ op: 'list_memory' });
	}

	clearMemoryError() {
		this.memoryError = null;
	}

	async saveMemory(args: {
		scope: string;
		name: string;
		description: string;
		kind: string;
		body: string;
		previousScope?: string;
		previousName?: string;
	}) {
		if (this.closed || this.memorySaving) return false;
		this.memorySaving = true;
		this.memoryError = null;
		const sent = await this.#send({
			op: 'save_memory',
			scope: args.scope,
			name: args.name,
			description: args.description,
			kind: args.kind,
			body: args.body,
			...(args.previousScope && args.previousName
				? { previous_scope: args.previousScope, previous_name: args.previousName }
				: {})
		});
		if (!sent) this.memorySaving = false;
		return sent;
	}

	async deleteMemory(scope: string, name: string) {
		if (this.closed || this.memorySaving) return false;
		this.memorySaving = true;
		this.memoryError = null;
		const sent = await this.#send({ op: 'delete_memory', scope, name });
		if (!sent) this.memorySaving = false;
		return sent;
	}

	async restartMcp(id: string) {
		if (this.closed) return false;
		this.mcpError = null;
		const sent = await this.#send({ op: 'restart_mcp', id });
		if (sent) this.mcpFeedback = `Restart requested for ${id}…`;
		return sent;
	}

	async enableMcp(id: string) {
		if (this.closed || this.mcpSaving || this.streaming || this.turnPending) return false;
		this.mcpSaving = true;
		this.mcpError = null;
		this.mcpFeedback = null;
		const sent = await this.#send({ op: 'enable_mcp', id });
		if (!sent) this.mcpSaving = false;
		return sent;
	}

	async disableMcp(id: string, scope: WriteScope) {
		const current = this.info?.mcp_disabled ?? [];
		const next = current.includes(id) ? current : [...current, id];
		this.mcpError = null;
		const ok = await this.saveSetting('mcp.disabled', formatTomlStringArray(next), scope, true);
		if (ok) this.mcpFeedback = `Disabled MCP server ${id} in ${scope} config.`;
		return ok;
	}

	async saveMcpServer(args: {
		id: string;
		scope: WriteScope;
		description: string;
		command: string;
		args: string[];
		cwd: string;
		url: string;
		sandbox?: boolean;
		env: Record<string, McpSecretValue>;
		headers: Record<string, McpSecretValue>;
	}) {
		if (this.closed || this.mcpSaving || this.streaming || this.turnPending) return false;
		this.mcpSaving = true;
		this.mcpError = null;
		this.mcpFeedback = null;
		const sent = await this.#send({
			op: 'save_mcp_server',
			id: args.id,
			scope: args.scope,
			description: args.description,
			command: args.command,
			args: args.args,
			cwd: args.cwd,
			url: args.url,
			...(args.sandbox === undefined ? {} : { sandbox: args.sandbox }),
			env: args.env,
			headers: args.headers
		});
		if (!sent) this.mcpSaving = false;
		return sent;
	}

	async enableHook(id: string, replacePayload?: boolean | null) {
		if (this.closed || this.hookSaving || this.streaming || this.turnPending) return false;
		this.hookSaving = true;
		this.hookError = null;
		this.hookFeedback = null;
		const sent = await this.#send({
			op: 'enable_hook',
			id,
			...(replacePayload === undefined ? {} : { replace_payload: replacePayload })
		});
		if (!sent) this.hookSaving = false;
		return sent;
	}

	async disableHook(id: string, scope: WriteScope) {
		const current = this.info?.hooks_disabled ?? [];
		const next = current.includes(id) ? current : [...current, id];
		this.hookError = null;
		const ok = await this.saveSetting('hooks.disabled', formatTomlStringArray(next), scope, true);
		if (ok) this.hookFeedback = `Disabled hook ${id} in ${scope} config.`;
		return ok;
	}

	dismissHookPayloadChoice() {
		this.hookPayloadChoice = null;
	}

	async inspectHookSetup(id: string) {
		if (this.closed || this.hookSetupLoading) return false;
		await this.cancelHookCheck();
		const request = this.#nextHookSetupRequest++;
		this.#activeHookInspectRequest = request;
		this.hookSetupLoading = true;
		this.hookSetupError = null;
		this.hookSetupStatus = null;
		const sent = await this.#send({ op: 'inspect_hook_setup', id, request });
		if (!sent) {
			this.#activeHookInspectRequest = null;
			this.hookSetupLoading = false;
		}
		return sent;
	}

	async saveHookSetup(args: {
		id: string;
		revision: string;
		connections: HookConnectionDraft[];
		enable?: boolean | null;
		replacePayload?: boolean | null;
	}) {
		if (this.closed || this.hookSetupSaving) return false;
		const request = this.#nextHookSetupRequest++;
		this.#activeHookSaveRequest = request;
		this.hookSetupSaving = true;
		this.hookSetupError = null;
		this.hookSetupStatus = null;
		this.hookSetupPendingRequest = null;
		const sent = await this.#send({
			op: 'save_hook_setup',
			id: args.id,
			request,
			revision: args.revision,
			connections: args.connections,
			enable: args.enable ?? null,
			replace_payload: args.replacePayload ?? null
		});
		if (!sent) {
			this.#activeHookSaveRequest = null;
			this.hookSetupSaving = false;
		}
		return sent;
	}

	async checkHookSetup(args: {
		id: string;
		revision: string;
		connection: HookConnectionDraft;
		url: string;
	}) {
		if (this.closed || this.hookSetupChecking) return false;
		const request = this.#nextHookSetupRequest++;
		this.#activeHookCheckRequest = request;
		this.hookSetupChecking = true;
		this.hookSetupError = null;
		this.hookSetupStatus = { kind: 'notice', text: `Testing connection to ${args.url || args.id}…` };
		const sent = await this.#send({
			op: 'check_hook_setup',
			id: args.id,
			request,
			revision: args.revision,
			connection: args.connection,
			url: args.url
		});
		if (!sent) {
			this.#activeHookCheckRequest = null;
			this.hookSetupChecking = false;
		}
		return sent;
	}

	async cancelHookCheck() {
		const request = this.#activeHookCheckRequest;
		if (request === null) return false;
		this.#activeHookCheckRequest = null;
		this.hookSetupChecking = false;
		this.hookSetupStatus = { kind: 'notice', text: 'Connection check cancelled.' };
		return this.#send({ op: 'cancel_hook_check', request });
	}

	closeHookSetup() {
		void this.cancelHookCheck();
		this.#activeHookInspectRequest = null;
		this.#activeHookSaveRequest = null;
		this.hookSetup = null;
		this.hookSetupLoading = false;
		this.hookSetupSaving = false;
		this.hookSetupChecking = false;
		this.hookSetupPendingRequest = null;
		this.hookSetupStatus = null;
		this.hookSetupError = null;
	}
}
