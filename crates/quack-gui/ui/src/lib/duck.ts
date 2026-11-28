import type { Item } from './session.svelte';

/** Frames in one welcome-duck paddle lap, matching the TUI's 16-step choreography. */
export const DUCK_ANIM_STEPS = 16;

/** Milliseconds per welcome-duck frame (TUI runs 8 × 16 ms = 128 ms per step). */
export const DUCK_FRAME_MS = 128;

export interface DuckFrame {
	step: number;
	shift: 0 | 1;
	top: string;
	headLead: string;
	beakTop: string;
	headRest: string;
	bodyLead: string;
	beakLower: string;
	tail: string;
	bottom: string;
	water: string;
	quacking: boolean;
	blinking: boolean;
}

const WAVES = ['~~~~~~~~~~', '~≈~≈~≈~≈~≈', '≈~≈~≈~≈~≈~', '~≈~≈~≈~≈~≈'] as const;

type RawPose = readonly [
	shift: 0 | 1,
	eye: string,
	beakLower: string,
	tail: string,
	waveIdx: 0 | 1 | 2 | 3
];

const POSES: readonly RawPose[] = [
	// 0: resting pose
	[0, '(o )', '_>', ' /  ', 0],
	[0, '(o )', '_>', ' /  ', 1],
	[0, '(o )', '_>', ' |  ', 2],
	[0, '(o )', '_>', ' /  ', 3],
	// 4..9: glide forward & quack
	[1, '(o )', '_>', ' / ', 0],
	[1, '(o )', '_>', ' | ', 1],
	[1, '(o )', '_>', ' / ', 2],
	[1, '(o )', ' >', ' / ', 3], // quack!
	[1, '(o )', ' >', ' / ', 0], // quack!
	[1, '(o )', '_>', ' / ', 1],
	// 10..15: glide back, blink & settle
	[0, '(o )', '_>', ' |  ', 2],
	[0, '(o )', '_>', ' /  ', 3],
	[0, '(o )', '_>', ' /  ', 0],
	[0, '(- )', '_>', ' /  ', 1], // blink!
	[0, '(o )', '_>', ' /  ', 2],
	[0, '(o )', '_>', ' /  ', 3]
];

/**
 * Compute one frame of the welcome duck's paddle lap.
 * Mirrors `crates/quack-tui/src/ui/chrome.rs::welcome` so both clients share
 * the exact same glide, tail wag, quack, blink, and shimmering pond water.
 */
export function duckFrame(rawStep: number): DuckFrame {
	const step =
		Number.isFinite(rawStep) && rawStep >= 0 && rawStep < POSES.length
			? Math.floor(rawStep)
			: 0;
	const [shift, eye, beakLower, tail, waveIdx] = POSES[step];
	const pad = shift === 0 ? '' : ' ';
	const trail = shift === 0 ? '  ' : ' ';
	return {
		step,
		shift,
		top: `${pad}   __     ${shift === 0 ? ' ' : ''}`,
		headLead: `${pad} `,
		beakTop: '<',
		headRest: `${eye}___${trail}`,
		bodyLead: `${pad}  ( .`,
		beakLower,
		tail,
		bottom: `${pad}   \`---'  ${shift === 0 ? ' ' : ''}`,
		water: `${WAVES[waveIdx]} `,
		quacking: beakLower === ' >',
		blinking: eye === '(- )'
	};
}

/**
 * Normalized 0..1 position for the plan-river duck along the track.
 * Follows completed steps (never a timer) and reserves the far bank strictly
 * for 100% completion, matching `crates/quack-tui/src/theme.rs::plan_river`.
 */
export function planRiverPosition(done: number, total: number): number {
	if (!Number.isFinite(done) || !Number.isFinite(total) || total <= 0 || done <= 0) {
		return 0;
	}
	if (done >= total) return 1;
	return Math.min(done / total, 0.94);
}

/**
 * Describe what the active turn is doing right now, mirroring the TUI's
 * status-bar activity label (`thinking`, `writing`, `running <tool>`, `working`).
 */
export function transcriptActivity(items: readonly Item[]): string {
	const last = items[items.length - 1];
	if (!last) return 'working';
	if (last.kind === 'tool' && last.ok === null) {
		return last.name ? `running ${last.name}` : 'running tool';
	}
	if (last.kind === 'reasoning' && last.streaming) return 'thinking';
	if (last.kind === 'assistant' && last.streaming && last.text.trim().length > 0) {
		return 'writing';
	}
	return 'working';
}
