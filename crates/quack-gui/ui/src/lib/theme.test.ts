import { describe, it, expect, beforeEach, vi } from 'vitest';
import { ThemeStore, THEME_STORAGE_KEY } from './theme.svelte';

describe('ThemeStore', () => {
	let store: ThemeStore;
	let mockStorage: Record<string, string>;

	beforeEach(() => {
		mockStorage = {};
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
				dataset: {}
			}
		});
		vi.stubGlobal('window', {
			matchMedia: vi.fn().mockReturnValue({
				matches: true,
				addEventListener: vi.fn()
			})
		});

		store = new ThemeStore();
	});

	it('defaults to system preference when storage is empty', () => {
		expect(store.preference).toBe('system');
		expect(store.systemDark).toBe(true);
		expect(store.resolved).toBe('dark');
		expect((document.documentElement as any).dataset.theme).toBe('dark');
	});

	it('resolves to light when system preference is light', () => {
		store.systemDark = false;
		expect(store.resolved).toBe('light');
	});

	it('sets explicit light theme and stores it', () => {
		store.set('light');
		expect(store.preference).toBe('light');
		expect(store.resolved).toBe('light');
		expect(mockStorage[THEME_STORAGE_KEY]).toBe('light');
		expect((document.documentElement as any).dataset.theme).toBe('light');
	});

	it('sets explicit dark theme and stores it', () => {
		store.set('dark');
		expect(store.preference).toBe('dark');
		expect(store.resolved).toBe('dark');
		expect(mockStorage[THEME_STORAGE_KEY]).toBe('dark');
		expect((document.documentElement as any).dataset.theme).toBe('dark');
	});

	it('cycles through system -> light -> dark -> system', () => {
		expect(store.preference).toBe('system');
		store.cycle();
		expect(store.preference).toBe('light');
		store.cycle();
		expect(store.preference).toBe('dark');
		store.cycle();
		expect(store.preference).toBe('system');
	});

	it('initializes from session only when localStorage has no explicit choice', () => {
		store.initFromSession('light');
		expect(store.preference).toBe('light');

		// Now explicit choice is saved
		store.set('dark');
		// Trying to init from another session should not override user choice
		store.initFromSession('light');
		expect(store.preference).toBe('dark');
	});
});
