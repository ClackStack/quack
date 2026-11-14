import type {
	HookConnectionDraft,
	HookCredentialChange,
	HookDescriptor,
	HookField,
	HookOutcome,
	HookPreset,
	HookSetupInfo,
	KeySource,
	McpSecretValue,
	McpServerInfo
} from '#lib/protocol';
import type { HookRow, McpRow } from './transport/types';

export interface MergedMcpServer {
	id: string;
	config: McpRow | null;
	live: McpServerInfo | null;
	layer: string;
	file: string;
	transport: string;
	target: string;
	description: string;
	enabled: boolean;
	canToggle: boolean;
	blockedByPolicy: boolean;
	state: 'enabled' | 'starting' | 'ready' | 'failed' | 'stopped' | 'disabled';
	detail: string;
	tools: string[];
	prompts: string[];
	resources: string[];
	log: string[];
	deferred: boolean;
	toolTokens: number;
	sandbox: boolean | null;
	autoApprove: string[];
	envKeys: string[];
	headerKeys: string[];
}

export interface HookRunEntry {
	id: string;
	on: string;
	outcome: HookOutcome;
	ms: number;
	message?: string;
	contextTokens: number;
	at: number;
}

export interface McpBindingEntry {
	id: number;
	key: string;
	mode: 'env' | 'plain' | 'keyring';
	value: string;
	format: string;
}

/** Combine static config rows (`info.mcp`) with live runtime state (`session.mcpServers`). */
export function mergeMcpServers(configRows: McpRow[], liveRows: McpServerInfo[]): MergedMcpServer[] {
	const byId = new Map<string, { config: McpRow | null; live: McpServerInfo | null }>();
	for (const row of configRows) {
		byId.set(row.id, { config: row, live: null });
	}
	for (const live of liveRows) {
		const prev = byId.get(live.id);
		if (prev) prev.live = live;
		else byId.set(live.id, { config: null, live });
	}

	const merged: MergedMcpServer[] = [];
	for (const [id, { config, live }] of byId) {
		const blockedByPolicy = config?.blocked_by_policy ?? false;
		const configFailed = config?.state === 'failed';
		const enabled = config ? config.enabled : true;

		let state: MergedMcpServer['state'];
		if (configFailed) state = 'failed';
		else if (!enabled) state = 'disabled';
		else if (live) state = live.state;
		else state = 'stopped';

		const detail =
			live?.error ||
			(configFailed || !enabled ? config?.detail : '') ||
			(live ? '' : config?.detail || '');

		merged.push({
			id,
			config,
			live,
			layer: config?.layer ?? 'runtime',
			file: config?.file ?? '',
			transport: live?.transport ?? config?.transport ?? 'unknown',
			target: live?.target || config?.target || '',
			description: live?.description || config?.description || '',
			enabled,
			canToggle: config?.can_toggle ?? false,
			blockedByPolicy,
			state,
			detail,
			tools: live?.tools ?? [],
			prompts: live?.prompts ?? [],
			resources: live?.resources ?? [],
			log: live?.log ?? [],
			deferred: live?.deferred ?? false,
			toolTokens: live?.tool_tokens ?? 0,
			sandbox: config?.sandbox ?? null,
			autoApprove: config?.auto_approve ?? [],
			envKeys: config?.env_keys ?? [],
			headerKeys: config?.header_keys ?? []
		});
	}

	const rank = (s: MergedMcpServer) => {
		if (s.state === 'failed') return 0;
		if (s.state === 'ready' || s.state === 'starting') return 1;
		if (s.enabled) return 2;
		return 3;
	};
	return merged.sort((a, b) => rank(a) - rank(b) || a.id.localeCompare(b.id));
}

export function sortHooks(rows: HookRow[]): HookRow[] {
	const rank = (h: HookRow) => (h.state === 'failed' ? 0 : h.enabled ? 1 : 2);
	return [...rows].sort((a, b) => rank(a) - rank(b) || a.id.localeCompare(b.id));
}

/** Split an argument string supporting single/double quotes into argv elements. */
export function parseArgsString(raw: string): string[] {
	const out: string[] = [];
	let current = '';
	let quote: '"' | "'" | null = null;
	let hasToken = false;
	for (let i = 0; i < raw.length; i++) {
		const ch = raw[i];
		if (quote) {
			if (ch === '\\' && quote === '"' && i + 1 < raw.length) {
				current += raw[++i];
			} else if (ch === quote) {
				quote = null;
			} else {
				current += ch;
			}
		} else if (ch === '"' || ch === "'") {
			quote = ch;
			hasToken = true;
		} else if (/\s/.test(ch)) {
			if (hasToken) {
				out.push(current);
				current = '';
				hasToken = false;
			}
		} else {
			current += ch;
			hasToken = true;
		}
	}
	if (hasToken) out.push(current);
	return out;
}

/** Convert UI key/secret binding entries into `McpSecretValue` records for `Op::SaveMcpServer`. */
export function buildMcpSecretMap(entries: McpBindingEntry[]): Record<string, McpSecretValue> {
	const out: Record<string, McpSecretValue> = {};
	for (const entry of entries) {
		const key = entry.key.trim();
		const value = entry.value.trim();
		if (!key || !value) continue;
		if (entry.mode === 'env') {
			const fmt = entry.format.trim();
			out[key] = fmt ? { kind: 'env', env: value, format: fmt } : { kind: 'env', env: value };
		} else if (entry.mode === 'keyring') {
			out[key] = { kind: 'keyring', keyring: value };
		} else {
			out[key] = { kind: 'plain', value };
		}
	}
	return out;
}

/** Build editable connection drafts from the engine's `HookSetupInfo`. */
export function createDraftsFromSetup(info: HookSetupInfo): HookConnectionDraft[] {
	return info.connections.map((entry) => {
		const credentials: Record<string, HookCredentialChange> = {};
		for (const field of info.descriptor.fields) {
			if (field.kind !== 'secret') continue;
			if (entry.credentials[field.id]) {
				credentials[field.id] = { action: 'keep' };
			}
		}
		return {
			connection: {
				id: entry.connection.id,
				preset: entry.connection.preset,
				enabled: entry.connection.enabled,
				values: { ...entry.connection.values }
			},
			credentials
		};
	});
}

function uniqueConnectionId(existing: HookConnectionDraft[]): string {
	const used = new Set(existing.map((c) => c.connection.id));
	if (!used.has('default')) return 'default';
	for (let n = 2; n < 1000; n++) {
		const candidate = `connection-${n}`;
		if (!used.has(candidate)) return candidate;
	}
	return `connection-${Date.now()}`;
}

/** Create a new blank connection draft seeded with defaults and the first preset (if any). */
export function addBlankConnection(
	descriptor: HookDescriptor,
	existing: HookConnectionDraft[]
): HookConnectionDraft {
	const preset = descriptor.presets[0];
	const values: Record<string, string> = {};
	for (const field of descriptor.fields) {
		if (field.kind === 'secret') continue;
		if (preset && field.id in preset.values) {
			values[field.id] = preset.values[field.id];
		} else if (field.default) {
			values[field.id] = field.default;
		} else {
			values[field.id] = '';
		}
	}
	const draft: HookConnectionDraft = {
		connection: {
			id: uniqueConnectionId(existing),
			preset: preset?.id ?? '',
			enabled: true,
			values
		},
		credentials: {}
	};
	return deriveConnectionFields(draft, descriptor);
}

/** Switch preset on a draft connection and apply its preset defaults and derived templates. */
export function applyPresetToDraft(
	draft: HookConnectionDraft,
	descriptor: HookDescriptor,
	presetId: string
): HookConnectionDraft {
	const previousPreset = descriptor.presets.find((p) => p.id === draft.connection.preset);
	const nextPreset = descriptor.presets.find((p) => p.id === presetId);
	const values = { ...draft.connection.values };

	if (nextPreset) {
		for (const [key, presetVal] of Object.entries(nextPreset.values)) {
			const currentVal = (values[key] ?? '').trim();
			const wasPrevDefault =
				previousPreset !== undefined &&
				previousPreset.values[key] !== undefined &&
				currentVal === previousPreset.values[key];
			const fieldDefault = descriptor.fields.find((f) => f.id === key)?.default ?? '';
			if (!currentVal || wasPrevDefault || currentVal === fieldDefault) {
				values[key] = presetVal;
			}
		}
		for (const field of descriptor.fields) {
			if (field.kind !== 'choice') continue;
			const allowed = nextPreset.choices[field.id];
			if (allowed && allowed.length > 0 && !allowed.includes(values[field.id] ?? '')) {
				values[field.id] = allowed[0];
			}
		}
	}

	return deriveConnectionFields(
		{
			...draft,
			connection: {
				...draft.connection,
				preset: presetId,
				values
			}
		},
		descriptor
	);
}

/** Expand `preset.derived` templates such as `{web_url}/api/v4` when placeholders are known. */
export function deriveConnectionFields(
	draft: HookConnectionDraft,
	descriptor: HookDescriptor
): HookConnectionDraft {
	const preset = descriptor.presets.find((p) => p.id === draft.connection.preset);
	if (!preset || Object.keys(preset.derived).length === 0) return draft;

	const values = { ...draft.connection.values };
	let changed = false;
	for (const [targetKey, template] of Object.entries(preset.derived)) {
		let missingInput = false;
		const expanded = template.replace(/\{([a-zA-Z0-9_]+)\}/g, (_, ref: string) => {
			const raw = (values[ref] ?? '').trim().replace(/\/+$/, '');
			if (!raw) missingInput = true;
			return raw;
		});
		if (!missingInput && values[targetKey] !== expanded) {
			values[targetKey] = expanded;
			changed = true;
		}
	}
	if (!changed) return draft;
	return {
		...draft,
		connection: {
			...draft.connection,
			values
		}
	};
}

/** Choices offered for a `choice` hook field, preferring preset-scoped choices when defined. */
export function fieldChoices(field: HookField, preset?: HookPreset): string[] {
	const override = preset?.choices[field.id];
	if (override && override.length > 0) return override;
	return field.choices;
}

/** Whether a field is currently required given `required` and `required_when`. */
export function isFieldRequired(field: HookField, values: Record<string, string>): boolean {
	if (field.required) return true;
	for (const [depKey, accepted] of Object.entries(field.required_when)) {
		if (accepted.includes(values[depKey] ?? '')) return true;
	}
	return false;
}

/** Whether a field should be rendered in the connection editor. */
export function isFieldVisible(
	field: HookField,
	values: Record<string, string>,
	showAdvanced: boolean
): boolean {
	if (!field.advanced || showAdvanced) return true;
	if (isFieldRequired(field, values)) return true;
	return (values[field.id] ?? '').trim().length > 0;
}

/** Human summary for an existing `KeySource` returned in `HookConnectionInfo.credentials`. */
export function formatKeySource(source: KeySource | undefined): string {
	if (!source) return 'not configured';
	switch (source.from) {
		case 'keyring':
			return `OS keyring (${source.name})`;
		case 'env':
			return `env var ${source.var}`;
		case 'file':
			return `file (${source.path})`;
		case 'cmd':
			return 'command';
		case 'value':
			return 'saved in hook file';
	}
}

/** Target connection URL passed to `Op::CheckHookSetup` for reachability/auth verification. */
export function connectionCheckUrl(
	draft: HookConnectionDraft,
	descriptor: HookDescriptor
): string {
	if (descriptor.scope_field && draft.connection.values[descriptor.scope_field]) {
		return draft.connection.values[descriptor.scope_field].trim();
	}
	const api = draft.connection.values.api_url?.trim();
	if (api) return api;
	const web = draft.connection.values.web_url?.trim();
	return web ?? '';
}
