import type { EgressSnapshot } from './transport/types';

export interface ModelSpeedSample {
	ttftMs: number;
	tokensPerSec: number | null;
	samples: number;
}

const STORAGE_KEY = 'quack.modelSpeeds.v1';

const memoryCache = new Map<string, ModelSpeedSample>();

function loadMap(): Map<string, ModelSpeedSample> {
	if (typeof localStorage !== 'undefined') {
		try {
			const raw = localStorage.getItem(STORAGE_KEY);
			if (raw) {
				const parsed = JSON.parse(raw) as Record<string, ModelSpeedSample>;
				return new Map(Object.entries(parsed));
			}
		} catch {
			// Ignore malformed storage entries or unavailable storage.
		}
	}
	return new Map(memoryCache);
}

function saveMap(map: Map<string, ModelSpeedSample>) {
	memoryCache.clear();
	for (const [k, v] of map) memoryCache.set(k, v);
	if (typeof localStorage !== 'undefined') {
		try {
			localStorage.setItem(STORAGE_KEY, JSON.stringify(Object.fromEntries(map)));
		} catch {
			// Quota/privacy mode; keep in-memory cache only.
		}
	}
}

export function recordModelSpeed(
	model: string,
	ttftMs: number,
	tokensPerSec: number | null
): ModelSpeedSample {
	const map = loadMap();
	const prev = map.get(model);
	const samples = (prev?.samples ?? 0) + 1;
	const blend = (a: number, b: number) => Math.round((a * 0.6 + b * 0.4) * 10) / 10;
	const next: ModelSpeedSample = {
		ttftMs: prev ? Math.round(blend(prev.ttftMs, ttftMs)) : Math.round(ttftMs),
		tokensPerSec:
			tokensPerSec !== null
				? prev?.tokensPerSec != null
					? blend(prev.tokensPerSec, tokensPerSec)
					: Math.round(tokensPerSec * 10) / 10
				: (prev?.tokensPerSec ?? null),
		samples
	};
	map.set(model, next);
	saveMap(map);
	return next;
}

export function getModelSpeed(model: string): ModelSpeedSample | null {
	return loadMap().get(model) ?? null;
}

export function isLocalProvider(
	egress: EgressSnapshot | null | undefined,
	providerId: string
): boolean {
	return egress?.providers.some((p) => p.id === providerId && p.local) ?? false;
}
