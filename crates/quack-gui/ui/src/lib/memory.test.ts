import { describe, expect, it } from 'vitest';
import type { MemoryNote } from '#lib/protocol';
import {
	parseMemoryNote,
	slugifyMemoryName,
	sortMemoryNotes,
	validateMemoryDraft
} from './memory';

const sampleNote: MemoryNote = {
	scope: 'project',
	name: 'rust-style',
	description: 'Workspace lints and error conventions',
	path: '/repo/.quack/memory/notes/rust-style.md',
	text: '---\nname: rust-style\ndescription: Workspace lints and error conventions\ntype: project\nupdated: 2026-11-12\n---\n\n# Rust rules\n\n- Never use `.unwrap()`.\n'
};

describe('memory helpers', () => {
	it('parses front matter metadata and separates markdown body', () => {
		const parsed = parseMemoryNote(sampleNote);
		expect(parsed.kind).toBe('project');
		expect(parsed.updated).toBe('2026-11-12');
		expect(parsed.body).toBe('# Rust rules\n\n- Never use `.unwrap()`.');
		expect(parsed.raw).toBe(sampleNote.text);
	});

	it('falls back gracefully when front matter is absent', () => {
		const parsed = parseMemoryNote({
			...sampleNote,
			text: 'Plain markdown without header\n\nSecond paragraph.\n'
		});
		expect(parsed.kind).toBeNull();
		expect(parsed.updated).toBeNull();
		expect(parsed.body).toBe('Plain markdown without header\n\nSecond paragraph.');
	});

	it('slugifies note names to valid 64-char kebab-case stems', () => {
		expect(slugifyMemoryName('Use pnpm & Vitest!')).toBe('use-pnpm-vitest');
		expect(slugifyMemoryName('--Leading and Trailing--')).toBe('leading-and-trailing');
		expect(slugifyMemoryName('a'.repeat(80)).length).toBeLessThanOrEqual(64);
	});

	it('validates fields and guards duplicate target names on create or move', () => {
		const existing: MemoryNote[] = [
			sampleNote,
			{
				scope: 'global',
				name: 'editor-prefs',
				description: 'Prefs',
				path: '/home/u/.local/share/quack/memory/notes/editor-prefs.md',
				text: ''
			}
		];

		expect(
			validateMemoryDraft(
				{
					scope: 'project',
					name: 'new-note',
					description: 'Summary line',
					kind: 'feedback',
					body: 'Always run tests.'
				},
				existing
			)
		).toBeNull();

		expect(
			validateMemoryDraft(
				{
					scope: 'project',
					name: '-bad-name',
					description: 'Summary',
					kind: 'project',
					body: 'Body'
				},
				existing
			)
		).toMatch(/lowercase letters/i);

		expect(
			validateMemoryDraft(
				{
					scope: 'project',
					name: 'new-note',
					description: 'line 1\nline 2',
					kind: 'project',
					body: 'Body'
				},
				existing
			)
		).toMatch(/single line/i);

		// Creating over an existing (scope, name) fails.
		expect(
			validateMemoryDraft(
				{
					scope: 'project',
					name: 'rust-style',
					description: 'Summary',
					kind: 'project',
					body: 'Body'
				},
				existing
			)
		).toMatch(/already exists/i);

		// Editing the same (scope, name) in place succeeds.
		expect(
			validateMemoryDraft(
				{
					scope: 'project',
					name: 'rust-style',
					description: 'Summary',
					kind: 'project',
					body: 'Updated body'
				},
				existing,
				{ scope: 'project', name: 'rust-style' }
			)
		).toBeNull();

		// Renaming/moving onto another existing (scope, name) fails.
		expect(
			validateMemoryDraft(
				{
					scope: 'global',
					name: 'editor-prefs',
					description: 'Summary',
					kind: 'user',
					body: 'Body'
				},
				existing,
				{ scope: 'project', name: 'rust-style' }
			)
		).toMatch(/already exists/i);
	});

	it('sorts project notes ahead of global notes alphabetically', () => {
		const sorted = sortMemoryNotes([
			{ ...sampleNote, scope: 'global', name: 'zebra' },
			{ ...sampleNote, scope: 'project', name: 'beta' },
			{ ...sampleNote, scope: 'global', name: 'alpha' },
			{ ...sampleNote, scope: 'project', name: 'alpha' }
		]);
		expect(sorted.map((n) => `${n.scope}:${n.name}`)).toEqual([
			'project:alpha',
			'project:beta',
			'global:alpha',
			'global:zebra'
		]);
	});
});
