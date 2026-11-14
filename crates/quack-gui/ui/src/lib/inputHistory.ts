export interface HistoryCursor {
	/** `null` when editing the current unsent draft; otherwise an index into `history` (`0` = oldest). */
	index: number | null;
	/** The unsent draft saved when entering history browsing from `index === null`. */
	savedDraft: string;
}

export interface HistoryStepResult {
	draft: string;
	cursor: HistoryCursor;
	caret: number;
}

export function initialHistoryCursor(): HistoryCursor {
	return { index: null, savedDraft: '' };
}

/** True when selection is collapsed and the caret is on the first line (no `\n` before it). */
export function isOnFirstLine(
	value: string,
	selectionStart: number,
	selectionEnd: number
): boolean {
	return selectionStart === selectionEnd && !value.slice(0, selectionStart).includes('\n');
}

/** True when selection is collapsed and the caret is on the last line (no `\n` after it). */
export function isOnLastLine(
	value: string,
	selectionStart: number,
	selectionEnd: number
): boolean {
	return selectionStart === selectionEnd && !value.slice(selectionEnd).includes('\n');
}

/** Step backward to an older prompt in `history` (oldest first). */
export function stepHistoryUp(
	history: readonly string[],
	cursor: HistoryCursor,
	currentDraft: string
): HistoryStepResult | null {
	if (history.length === 0) return null;
	if (cursor.index === null) {
		const nextIndex = history.length - 1;
		const draft = history[nextIndex];
		return {
			draft,
			cursor: { index: nextIndex, savedDraft: currentDraft },
			caret: draft.length
		};
	}
	if (cursor.index <= 0) return null;
	const nextIndex = cursor.index - 1;
	const draft = history[nextIndex];
	return {
		draft,
		cursor: { index: nextIndex, savedDraft: cursor.savedDraft },
		caret: draft.length
	};
}

/** Step forward toward newer prompts, or restore `savedDraft` past the newest entry. */
export function stepHistoryDown(
	history: readonly string[],
	cursor: HistoryCursor
): HistoryStepResult | null {
	if (cursor.index === null) return null;
	if (cursor.index + 1 < history.length) {
		const nextIndex = cursor.index + 1;
		const draft = history[nextIndex];
		return {
			draft,
			cursor: { index: nextIndex, savedDraft: cursor.savedDraft },
			caret: draft.length
		};
	}
	return {
		draft: cursor.savedDraft,
		cursor: initialHistoryCursor(),
		caret: cursor.savedDraft.length
	};
}
