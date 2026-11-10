import type {
	ApprovalRequest,
	ContextSegment,
	Event,
	ProviderModels,
	Question,
	ReplayItem,
	SessionSummary
} from '#lib/protocol';
import type { AppInfo, Transport } from '#lib/transport/types';

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
	  }
	| { kind: 'system'; id: number; tone: 'notice' | 'warning' | 'error'; text: string }
	| { kind: 'compacted'; id: number };

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

export type PermissionMode = 'read-only' | 'ask' | 'auto-edit' | 'full-auto' | 'plan' | 'yolo';

export const MODES: { id: PermissionMode; label: string; about: string }[] = [
	{ id: 'ask', label: 'Ask', about: 'Ask before edits and commands' },
	{ id: 'auto-edit', label: 'Auto-edit', about: 'Edit files without asking; ask before commands' },
	{ id: 'full-auto', label: 'Full auto', about: 'Run without asking, inside the sandbox' },
	{ id: 'read-only', label: 'Read-only', about: 'Look around; change nothing' },
	{ id: 'plan', label: 'Plan', about: 'Look around and propose a plan; change nothing' }
];

let itemSeq = 0;
const nextId = () => ++itemSeq;

export class Session {
	tab = $state(0);
	info = $state<AppInfo | null>(null);
	title = $state('');
	items = $state<Item[]>([]);
	streaming = $state(false);
	speed = $state<SpeedState | null>(null);
	lastUsage = $state<{ prompt: number; completion: number; cached: number | null } | null>(null);
	approval = $state<{ callId: string; request: ApprovalRequest; always: string[] } | null>(null);
	questions = $state<{ callId: string; questions: Question[] } | null>(null);
	providers = $state<ProviderModels[]>([]);
	currentModel = $state<string | null>(null);
	mode = $state<PermissionMode>('ask');
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

	#transport: Transport;

	constructor(transport: Transport, tab: number, info: AppInfo) {
		this.#transport = transport;
		this.tab = tab;
		this.info = info;
	}

	/** Feed one protocol event; the only place transcript state is written. */
	onEvent(ev: Event) {
		switch (ev.event) {
			case 'session_configured':
				this.starting = false;
				break;
			case 'session_resumed':
				this.starting = false;
				this.resumed = true;
				for (const item of ev.items) this.#pushReplay(item);
				break;
			case 'session_title':
				this.title = ev.title ?? '';
				break;
			case 'assistant_delta':
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
					open: false
				});
				break;
			case 'tool_call_output_delta': {
				const tool = this.items.findLast(
					(i) => i.kind === 'tool' && i.callId === ev.call_id
				) as Extract<Item, { kind: 'tool' }> | undefined;
				if (tool) tool.output += ev.chunk;
				break;
			}
			case 'tool_call_end': {
				const tool = this.items.findLast(
					(i) => i.kind === 'tool' && i.callId === ev.call_id
				) as Extract<Item, { kind: 'tool' }> | undefined;
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
				this.streaming = true;
				break;
			case 'turn_complete':
				this.streaming = false;
				this.#sealStreams();
				this.approval = null;
				this.lastUsage = {
					prompt: ev.usage.prompt_tokens,
					completion: ev.usage.completion_tokens,
					cached: ev.usage.cached_prompt_tokens ?? null
				};
				break;
			case 'turn_aborted':
				this.streaming = false;
				this.#sealStreams();
				this.approval = null;
				this.items.push({ kind: 'system', id: nextId(), tone: 'warning', text: `Interrupted: ${ev.reason}` });
				break;
			case 'speed':
				this.speed = { ttftMs: ev.ttft_ms, tokensPerSec: ev.tokens_per_sec ?? null };
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
			case 'permission_mode':
				this.mode = ev.mode as PermissionMode;
				break;
			case 'sandbox':
				this.sandboxOff = ev.off;
				break;
			case 'model_catalog':
				this.providers = ev.providers;
				this.currentModel = ev.current ?? null;
				break;
			case 'session_list':
				this.sessions = ev.sessions;
				break;
			case 'agent_started':
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
			case 'rewound':
				// The transcript above the rewind point stays visible; the next turn
				// continues from the shorter history (G2 adds item surgery).
				this.items.push({
					kind: 'system',
					id: nextId(),
					tone: 'notice',
					text: `Rewound ${ev.back} turn${ev.back === 1 ? '' : 's'} — the conversation continues from “${ev.text.slice(0, 80)}”`
				});
				break;
			case 'notice':
				this.items.push({ kind: 'system', id: nextId(), tone: 'notice', text: ev.message });
				break;
			case 'warning':
				this.items.push({ kind: 'system', id: nextId(), tone: 'warning', text: ev.message });
				break;
			case 'error':
				this.items.push({
					kind: 'system',
					id: nextId(),
					tone: 'error',
					text: `${ev.error.message} · check the provider's api_key if this keeps happening`
				});
				this.streaming = false;
				break;
			default:
				// Facts the header does not show yet (skills, hooks, LSP, doctor, stats…)
				// arrive here until their surfaces land (G2/G3).
				break;
		}
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
				open: false
			});
		} else {
			this.items.push({ kind: 'compacted', id: nextId() });
		}
	}

	#appendStream(kind: 'assistant' | 'reasoning', text: string) {
		const open = this.#openStreaming(kind);
		if (open) open.text += text;
		else this.items.push({ kind, id: nextId(), text, streaming: true });
	}

	#openStreaming(kind: 'assistant' | 'reasoning'): (Extract<Item, { kind: 'assistant' }> | Extract<Item, { kind: 'reasoning' }>) | undefined {
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

	async submit(text: string, files: string[] = [], images: string[] = []) {
		this.#sealStreams();
		this.items.push({ kind: 'user', id: nextId(), text, files, images });
		await this.#transport.sendOp(this.tab, {
			op: 'user_turn',
			items: [
				...(text ? [{ type: 'text' as const, text }] : []),
				...files.map((path) => ({ type: 'file' as const, path })),
				...images.map((path) => ({ type: 'image' as const, path }))
			],
			overrides: {}
		});
	}

	async interrupt() {
		await this.#transport.sendOp(this.tab, { op: 'interrupt' });
	}

	async answerApproval(decision: 'allow_once' | 'allow_session' | 'deny', feedback?: string) {
		const callId = this.approval?.callId;
		this.approval = null;
		if (!callId) return;
		await this.#transport.sendOp(this.tab, {
			op: 'approval_response',
			call_id: callId,
			decision:
				decision === 'deny' && feedback
					? { decision: 'deny_with_feedback', feedback }
					: { decision }
		});
	}

	async answerQuestions(answers: { picked?: number; text?: string; skipped?: boolean }[]) {
		const callId = this.questions?.callId;
		this.questions = null;
		if (!callId) return;
		await this.#transport.sendOp(this.tab, {
			op: 'answer_questions',
			call_id: callId,
			answers: answers.map((a) =>
				a.skipped ? { answer: 'skipped' } : a.picked !== undefined ? { answer: 'picked', index: a.picked } : { answer: 'text', text: a.text ?? '' }
			),
			stop_asking: false
		});
	}

	async setModel(providerModel: string) {
		const [provider, model] = providerModel.split('/');
		if (!provider || !model) return;
		await this.#transport.sendOp(this.tab, { op: 'set_model', provider, model });
	}

	async setMode(mode: PermissionMode) {
		await this.#transport.sendOp(this.tab, { op: 'set_permission_mode', mode });
	}

	async acceptPlan() {
		await this.#transport.sendOp(this.tab, { op: 'accept_plan', mode: 'auto-edit' });
		this.mode = 'auto-edit';
		this.plan = '';
	}

	async refreshContext() {
		await this.#transport.sendOp(this.tab, { op: 'get_context' });
	}

	async compact(focus?: string) {
		await this.#transport.sendOp(this.tab, { op: 'compact', ...(focus ? { focus } : {}) });
	}

	async listSessions() {
		await this.#transport.sendOp(this.tab, { op: 'list_sessions' });
	}

	async newSession() {
		this.items = [];
		this.title = '';
		this.plan = '';
		this.agents = [];
		this.resumed = false;
		this.starting = true;
		await this.#transport.sendOp(this.tab, { op: 'new_session' });
		await this.listSessions();
	}

	async resumeSession(id: string) {
		this.items = [];
		this.title = '';
		this.plan = '';
		this.agents = [];
		this.resumed = false;
		this.starting = true;
		await this.#transport.sendOp(this.tab, { op: 'resume_session', id });
	}

	async undo() {
		await this.#transport.sendOp(this.tab, { op: 'undo', checkpoint: null, force: false });
	}
}
