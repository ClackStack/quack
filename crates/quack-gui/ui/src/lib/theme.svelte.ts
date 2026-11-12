export type ThemePreference = 'system' | 'light' | 'dark';
export type ResolvedTheme = 'light' | 'dark';

export const THEME_STORAGE_KEY = 'quack-theme';

export class ThemeStore {
	preference = $state<ThemePreference>('system');
	systemDark = $state(false);

	resolved = $derived<ResolvedTheme>(
		this.preference === 'system' ? (this.systemDark ? 'dark' : 'light') : this.preference
	);

	constructor() {
		if (typeof window !== 'undefined') {
			this.initBrowser();
		}
	}

	initBrowser() {
		try {
			const saved = localStorage.getItem(THEME_STORAGE_KEY);
			if (saved === 'dark' || saved === 'light' || saved === 'system') {
				this.preference = saved;
			}
		} catch {
			// ignore storage access errors
		}

		if (typeof window !== 'undefined' && typeof window.matchMedia === 'function') {
			const mq = window.matchMedia('(prefers-color-scheme: dark)');
			this.systemDark = mq.matches;

			mq.addEventListener?.('change', (e) => {
				this.systemDark = e.matches;
				this.apply();
			});
		}

		this.apply();
	}

	set(pref: ThemePreference) {
		this.preference = pref;
		if (typeof localStorage !== 'undefined') {
			try {
				if (pref === 'system') {
					localStorage.removeItem(THEME_STORAGE_KEY);
				} else {
					localStorage.setItem(THEME_STORAGE_KEY, pref);
				}
			} catch {
				// ignore storage access errors
			}
		}
		this.apply();
	}

	cycle() {
		if (this.preference === 'system') this.set('light');
		else if (this.preference === 'light') this.set('dark');
		else this.set('system');
	}

	initFromSession(sessionTheme?: string) {
		if (typeof localStorage !== 'undefined') {
			try {
				const saved = localStorage.getItem(THEME_STORAGE_KEY);
				if (!saved && (sessionTheme === 'dark' || sessionTheme === 'light')) {
					this.preference = sessionTheme;
					this.apply();
				}
			} catch {
				// ignore storage access errors
			}
		}
	}

	apply() {
		if (typeof document !== 'undefined' && document.documentElement) {
			document.documentElement.dataset.theme = this.resolved;
		}
	}
}

export const theme = new ThemeStore();
