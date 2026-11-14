export const TEXT_SCALE_STORAGE_KEY = 'quack-text-scale';
export const DEFAULT_TEXT_SCALE = 1;
export const MIN_TEXT_SCALE = 0.8;
export const MAX_TEXT_SCALE = 1.5;
export const TEXT_SCALE_STEP = 0.05;
export const TEXT_SCALE_PRESETS = [90, 100, 110, 125, 140] as const;

export function normalizeTextScale(raw: number | string | null | undefined): number {
	if (raw == null || raw === '') return DEFAULT_TEXT_SCALE;
	const parsed = typeof raw === 'number' ? raw : Number.parseFloat(raw);
	if (!Number.isFinite(parsed) || parsed <= 0) return DEFAULT_TEXT_SCALE;
	const clamped = Math.min(MAX_TEXT_SCALE, Math.max(MIN_TEXT_SCALE, parsed));
	const stepped = Math.round(clamped / TEXT_SCALE_STEP) * TEXT_SCALE_STEP;
	return Number(stepped.toFixed(2));
}

export class TextScaleStore {
	scale = $state<number>(DEFAULT_TEXT_SCALE);

	percent = $derived(Math.round(this.scale * 100));
	isDefault = $derived(this.scale === DEFAULT_TEXT_SCALE);

	constructor() {
		this.initBrowser();
	}

	initBrowser(): void {
		if (typeof window === 'undefined' || typeof document === 'undefined') return;
		try {
			const saved = localStorage.getItem(TEXT_SCALE_STORAGE_KEY);
			this.scale = normalizeTextScale(saved);
		} catch {
			this.scale = DEFAULT_TEXT_SCALE;
		}
		this.apply();
	}

	set(scale: number): void {
		const next = normalizeTextScale(scale);
		this.scale = next;
		try {
			if (next === DEFAULT_TEXT_SCALE) {
				localStorage.removeItem(TEXT_SCALE_STORAGE_KEY);
			} else {
				localStorage.setItem(TEXT_SCALE_STORAGE_KEY, String(next));
			}
		} catch {
			// Ignore storage errors in restricted environments.
		}
		this.apply();
	}

	setPercent(percent: number): void {
		this.set(percent / 100);
	}

	step(deltaSteps: number): void {
		this.set(this.scale + deltaSteps * TEXT_SCALE_STEP);
	}

	reset(): void {
		this.set(DEFAULT_TEXT_SCALE);
	}

	apply(): void {
		if (typeof document === 'undefined') return;
		const root = document.documentElement;
		if (!root?.style?.setProperty) return;
		root.style.setProperty('--text-scale', String(this.scale));
	}
}

export const textScale = new TextScaleStore();
