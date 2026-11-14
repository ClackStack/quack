import { describe, expect, it } from 'vitest';
import { getModelSpeed, isLocalProvider, recordModelSpeed } from './modelStats';
import type { EgressSnapshot } from './transport/types';

describe('modelStats', () => {
	it('records and blends TTFT and throughput samples per model', () => {
		const id = `test-provider/model-${Date.now()}`;
		expect(getModelSpeed(id)).toBeNull();

		const first = recordModelSpeed(id, 300, 50);
		expect(first).toEqual({ ttftMs: 300, tokensPerSec: 50, samples: 1 });

		const second = recordModelSpeed(id, 200, 75);
		expect(second.samples).toBe(2);
		expect(second.ttftMs).toBe(260);
		expect(second.tokensPerSec).toBe(60);
		expect(getModelSpeed(id)).toEqual(second);
	});

	it('identifies local vs hosted providers from egress snapshot', () => {
		const egress: EgressSnapshot = {
			offline: false,
			offline_locked: false,
			egress_allow: [],
			providers: [
				{ id: 'ollama', host: '127.0.0.1:11434', local: true, enabled: true },
				{ id: 'openrouter', host: 'openrouter.ai', local: false, enabled: true }
			],
			mcp_remote: [],
			http1_only: false,
			has_proxy: false,
			updates_check: false,
			catalog_refresh: false,
			telemetry_enabled: false
		};

		expect(isLocalProvider(egress, 'ollama')).toBe(true);
		expect(isLocalProvider(egress, 'openrouter')).toBe(false);
		expect(isLocalProvider(null, 'ollama')).toBe(false);
	});
});
