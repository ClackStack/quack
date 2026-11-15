import { describe, it, expect, beforeEach, vi } from 'vitest';
import {
	DEFAULT_TEXT_SCALE,
	MAX_TEXT_SCALE,
	MIN_TEXT_SCALE,
	TEXT_SCALE_STORAGE_KEY,
	TextScaleStore,
	normalizeTextScale
} from './textScale.svelte';

describe('normalizeTextScale', () => {
	it('returns default for null, empty, NaN, or non-positive values', () => {
		expect(normalizeTextScale(null)).toBe(DEFAULT_TEXT_SCALE);
		expect(normalizeTextScale(undefined)).toBe(DEFAULT_TEXT_SCALE);
		expect(normalizeTextScale('')).toBe(DEFAULT_TEXT_SCALE);
		expect(normalizeTextScale('abc')).toBe(DEFAULT_TEXT_SCALE);
		expect(normalizeTextScale(0)).toBe(DEFAULT_TEXT_SCALE);
		expect(normalizeTextScale(-0.5)).toBe(DEFAULT_TEXT_SCALE);
	});

	it('clamps to min/max bounds and rounds to 5% increments', () => {
		expect(normalizeTextScale(0.2)).toBe(MIN_TEXT_SCALE);
		expect(normalizeTextScale(3)).toBe(MAX_TEXT_SCALE);
		expect(normalizeTextScale(1.12)).toBe(1.1);
		expect(normalizeTextScale('1.18')).toBe(1.2);
		expect(normalizeTextScale(1.15)).toBe(1.15);
	});
});

describe('TextScaleStore', () => {
	let store: TextScaleStore;
	let mockStorage: Record<string, string>;
	let styleProps: Record<string, string>;

	beforeEach(() => {
		mockStorage = {};
		styleProps = {};
		vi.stubGlobal('localStorage', {
			getItem: vi.fn((key: string) => mockStorage[key] ?? null),
			setItem: vi.fn((key: string, val: string) => {
				mockStorage[key] = val;
			}),
			removeItem: vi.fn((key: string) => {
				delete mockStorage[key];
			})
		});
		vi.stubGlobal('document', {
			documentElement: {
				style: {
					setProperty: vi.fn((name: string, val: string) => {
						styleProps[name] = val;
					})
				}
			}
		});
		vi.stubGlobal('window', {});
		store = new TextScaleStore();
	});

	it('defaults to 100% when storage is empty', () => {
		expect(store.scale).toBe(1);
		expect(store.percent).toBe(100);
		expect(store.isDefault).toBe(true);
		expect(styleProps['--text-scale']).toBe('1');
	});

	it('restores saved text scale from localStorage', () => {
		mockStorage[TEXT_SCALE_STORAGE_KEY] = '1.25';
		const restored = new TextScaleStore();
		expect(restored.scale).toBe(1.25);
		expect(restored.percent).toBe(125);
		expect(restored.isDefault).toBe(false);
		expect(styleProps['--text-scale']).toBe('1.25');
	});

	it('sets percent, clamps bounds, and persists cleanly', () => {
		store.setPercent(120);
		expect(store.scale).toBe(1.2);
		expect(store.percent).toBe(120);
		expect(mockStorage[TEXT_SCALE_STORAGE_KEY]).toBe('1.2');
		expect(styleProps['--text-scale']).toBe('1.2');

		store.step(1);
		expect(store.percent).toBe(125);

		store.step(-2);
		expect(store.percent).toBe(115);

		store.setPercent(250);
		expect(store.scale).toBe(MAX_TEXT_SCALE);

		store.setPercent(40);
		expect(store.scale).toBe(MIN_TEXT_SCALE);
	});

	it('removes storage entry when reset to default 100%', () => {
		store.setPercent(130);
		expect(mockStorage[TEXT_SCALE_STORAGE_KEY]).toBe('1.3');
		store.reset();
		expect(store.scale).toBe(1);
		expect(store.isDefault).toBe(true);
		expect(mockStorage[TEXT_SCALE_STORAGE_KEY]).toBeUndefined();
		expect(styleProps['--text-scale']).toBe('1');
	});
});
