import { describe, expect, it } from 'vitest';
import {
	initialHistoryCursor,
	isOnFirstLine,
	isOnLastLine,
	stepHistoryDown,
	stepHistoryUp
} from './inputHistory';

describe('input history navigation', () => {
	const history = ['first prompt', 'second\nprompt', 'third prompt'];

	it('walks backward, stops at oldest, and restores the unsent draft walking forward', () => {
		let cursor = initialHistoryCursor();

		const s1 = stepHistoryUp(history, cursor, 'unsent work');
		expect(s1).not.toBeNull();
		expect(s1?.draft).toBe('third prompt');
		expect(s1?.cursor.index).toBe(2);
		cursor = s1!.cursor;

		const s2 = stepHistoryUp(history, cursor, s1!.draft);
		expect(s2?.draft).toBe('second\nprompt');
		expect(s2?.cursor.index).toBe(1);
		cursor = s2!.cursor;

		const s3 = stepHistoryUp(history, cursor, s2!.draft);
		expect(s3?.draft).toBe('first prompt');
		expect(s3?.cursor.index).toBe(0);
		cursor = s3!.cursor;

		// At the oldest entry, pressing Up again does nothing.
		expect(stepHistoryUp(history, cursor, s3!.draft)).toBeNull();

		const d1 = stepHistoryDown(history, cursor);
		expect(d1?.draft).toBe('second\nprompt');
		expect(d1?.cursor.index).toBe(1);
		cursor = d1!.cursor;

		const d2 = stepHistoryDown(history, cursor);
		expect(d2?.draft).toBe('third prompt');
		expect(d2?.cursor.index).toBe(2);
		cursor = d2!.cursor;

		const d3 = stepHistoryDown(history, cursor);
		expect(d3?.draft).toBe('unsent work');
		expect(d3?.cursor.index).toBeNull();

		// Once back at the live draft, Down does nothing.
		expect(stepHistoryDown(history, d3!.cursor)).toBeNull();
	});

	it('handles empty history gracefully', () => {
		expect(stepHistoryUp([], initialHistoryCursor(), 'draft')).toBeNull();
		expect(stepHistoryDown([], initialHistoryCursor())).toBeNull();
	});

	it('detects first and last lines accurately for multi-line drafts', () => {
		const multi = 'line 1\nline 2\nline 3';
		expect(isOnFirstLine(multi, 0, 0)).toBe(true);
		expect(isOnFirstLine(multi, 6, 6)).toBe(true);
		expect(isOnFirstLine(multi, 7, 7)).toBe(false);
		expect(isOnFirstLine(multi, 0, 3)).toBe(false);

		expect(isOnLastLine(multi, multi.length, multi.length)).toBe(true);
		expect(isOnLastLine(multi, 14, 14)).toBe(true);
		expect(isOnLastLine(multi, 13, 13)).toBe(false);
		expect(isOnLastLine(multi, 14, 16)).toBe(false);
	});
});
