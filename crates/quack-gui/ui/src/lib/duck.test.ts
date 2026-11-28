import { describe, expect, it } from 'vitest';
import { DUCK_ANIM_STEPS, duckFrame, planRiverPosition, transcriptActivity } from './duck';
import type { Item } from './session.svelte';

describe('duckFrame', () => {
	it('returns the resting pose at step 0 and out-of-range steps', () => {
		const rest = duckFrame(0);
		expect(rest.shift).toBe(0);
		expect(rest.quacking).toBe(false);
		expect(rest.blinking).toBe(false);
		expect(rest.water.trim()).toBe('~~~~~~~~~~');
		expect(duckFrame(-1)).toEqual(rest);
		expect(duckFrame(DUCK_ANIM_STEPS)).toEqual(rest);
	});

	it('glides forward, quacks, blinks, and keeps constant line widths across all steps', () => {
		const widths = new Set<number>();
		for (let s = 0; s < DUCK_ANIM_STEPS; s++) {
			const f = duckFrame(s);
			widths.add(f.top.length);
			widths.add((f.headLead + f.beakTop + f.headRest).length);
			widths.add((f.bodyLead + f.beakLower + f.tail).length);
			widths.add(f.bottom.length);
			widths.add(f.water.length);
		}
		expect([...widths]).toEqual([11]);
		expect(duckFrame(4).shift).toBe(1);
		expect(duckFrame(7).quacking).toBe(true);
		expect(duckFrame(8).quacking).toBe(true);
		expect(duckFrame(13).blinking).toBe(true);
		expect(duckFrame(13).shift).toBe(0);
	});
});

describe('planRiverPosition', () => {
	it('returns 0 when empty or untouched and 1 only on full completion', () => {
		expect(planRiverPosition(0, 0)).toBe(0);
		expect(planRiverPosition(0, 4)).toBe(0);
		expect(planRiverPosition(2, 4)).toBe(0.5);
		expect(planRiverPosition(99, 100)).toBeLessThan(1);
		expect(planRiverPosition(99, 100)).toBe(0.94);
		expect(planRiverPosition(100, 100)).toBe(1);
		expect(planRiverPosition(5, 4)).toBe(1);
	});
});

describe('transcriptActivity', () => {
	it('mirrors the TUI activity states from the latest transcript item', () => {
		expect(transcriptActivity([])).toBe('working');

		const thinking: Item[] = [{ kind: 'reasoning', id: 1, text: 'hmm', streaming: true }];
		expect(transcriptActivity(thinking)).toBe('thinking');

		const writing: Item[] = [{ kind: 'assistant', id: 2, text: 'Hello', streaming: true }];
		expect(transcriptActivity(writing)).toBe('writing');

		const emptyAssistant: Item[] = [{ kind: 'assistant', id: 3, text: '   ', streaming: true }];
		expect(transcriptActivity(emptyAssistant)).toBe('working');

		const toolItem: Extract<Item, { kind: 'tool' }> = {
			kind: 'tool',
			id: 4,
			callId: 'c1',
			name: 'grep',
			summary: 'grep foo',
			ok: null,
			output: '',
			detail: null,
			open: false,
			subjects: []
		};
		expect(transcriptActivity([toolItem])).toBe('running grep');
		expect(transcriptActivity([{ ...toolItem, ok: true }])).toBe('working');
	});
});
