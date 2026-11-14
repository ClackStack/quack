import type { MemoryNote } from '#lib/protocol';

export type MemoryScope = 'project' | 'global';
export type MemoryKind = 'user' | 'project' | 'feedback' | 'reference';

export interface MemoryKindMeta {
	id: MemoryKind;
	label: string;
	summary: string;
}

export const MEMORY_KINDS: readonly MemoryKindMeta[] = [
	{
		id: 'project',
		label: 'Project',
		summary: 'Repository architecture, build commands, conventions, and gotchas'
	},
	{
		id: 'user',
		label: 'User',
		summary: 'Personal workflow, editor, and communication preferences'
	},
	{
		id: 'feedback',
		label: 'Feedback',
		summary: 'Corrections and rules on how the agent should work'
	},
	{
		id: 'reference',
		label: 'Reference',
		summary: 'Pointers to external docs, dashboards, issues, or services'
	}
];

export interface ParsedMemoryNote {
	scope: string;
	name: string;
	description: string;
	path: string;
	kind: MemoryKind | null;
	updated: string | null;
	body: string;
	raw: string;
}

export interface MemoryDraft {
	scope: string;
	name: string;
	description: string;
	kind: string;
	body: string;
}

const VALID_NAME_RE = /^[a-z0-9][a-z0-9-]{0,63}$/;
const VALID_KINDS = new Set<string>(MEMORY_KINDS.map((k) => k.id));

/**
 * Parse YAML front matter (`---\n...\n---`) produced by `quack_store::memory`
 * so the GUI renders and edits clean Markdown bodies while retaining metadata.
 */
export function parseMemoryNote(note: MemoryNote): ParsedMemoryNote {
	const normalized = note.text.replace(/\r\n/g, '\n');
	let kind: MemoryKind | null = null;
	let updated: string | null = null;
	let body = normalized.trim();

	if (normalized.startsWith('---\n')) {
		const close = normalized.indexOf('\n---', 4);
		if (close !== -1) {
			const header = normalized.slice(4, close);
			for (const line of header.split('\n')) {
				const sep = line.indexOf(':');
				if (sep === -1) continue;
				const key = line.slice(0, sep).trim();
				const val = line.slice(sep + 1).trim();
				if (key === 'type' && VALID_KINDS.has(val)) {
					kind = val as MemoryKind;
				} else if (key === 'updated' && val) {
					updated = val;
				}
			}
			const after = normalized.slice(close + 4);
			body = after.replace(/^\n+/, '').trimEnd();
		}
	}

	return {
		scope: note.scope,
		name: note.name,
		description: note.description,
		path: note.path,
		kind,
		updated,
		body,
		raw: note.text
	};
}

/** Normalize a human title into a valid kebab-case note filename stem. */
export function slugifyMemoryName(input: string): string {
	return input
		.toLowerCase()
		.replace(/[^a-z0-9]+/g, '-')
		.replace(/^-+|-+$/g, '')
		.slice(0, 64)
		.replace(/-+$/g, '');
}

/** Validate note fields client-side using the same constraints as `quack-store`. */
export function validateMemoryDraft(
	draft: MemoryDraft,
	existingNotes: readonly MemoryNote[],
	previous?: { scope: string; name: string } | null
): string | null {
	if (draft.scope !== 'project' && draft.scope !== 'global') {
		return 'Scope must be project or global.';
	}
	if (!VALID_KINDS.has(draft.kind)) {
		return 'Select a valid note type (project, user, feedback, or reference).';
	}
	if (!VALID_NAME_RE.test(draft.name)) {
		return 'Name must be 1–64 lowercase letters, digits, or hyphens (cannot start with `-`).';
	}
	const desc = draft.description.trim();
	if (!desc) {
		return 'Description is required for the memory index.';
	}
	if (/[\r\n]/.test(desc)) {
		return 'Description must be a single line.';
	}
	if (!draft.body.trim()) {
		return 'Note body cannot be empty.';
	}
	const isRenamingOrCreating =
		!previous || previous.scope !== draft.scope || previous.name !== draft.name;
	if (
		isRenamingOrCreating &&
		existingNotes.some((n) => n.scope === draft.scope && n.name === draft.name)
	) {
		return `A ${draft.scope} note named “${draft.name}” already exists.`;
	}
	return null;
}

/** Keep project notes ahead of global notes, sorted alphabetically by name. */
export function sortMemoryNotes(notes: readonly MemoryNote[]): MemoryNote[] {
	return [...notes].sort((a, b) => {
		if (a.scope !== b.scope) return a.scope === 'project' ? -1 : 1;
		return a.name.localeCompare(b.name);
	});
}
