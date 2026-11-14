import { Session } from '#lib/session.svelte';
import type { RecentFolder, ResumeRequest, TabMessage, Transport } from '#lib/transport/types';

export const PROJECTS_STORAGE_KEY = 'quack-projects';

export interface ProjectStorage {
	getItem(key: string): string | null;
	setItem(key: string, value: string): void;
}

export interface SidebarProject {
	cwd: string;
	session: Session | null;
	active: boolean;
}

const HOME_DIR_RE = /^(\/home\/[^/]+|\/Users\/[^/]+|[A-Za-z]:[\\/]Users[\\/][^\\/]+)(?=[\\/]|$)/;

function normalizePath(path: string): string {
	const trimmed = path.trim();
	if (!trimmed) return '';
	if (trimmed === '/' || trimmed === '~' || /^[A-Za-z]:[\\/]?$/.test(trimmed)) {
		return trimmed.length === 2 && trimmed !== '~' ? `${trimmed}\\` : trimmed;
	}
	return trimmed.replace(/[\\/]+$/, '');
}

function pathKey(path: string): string {
	const norm = normalizePath(path);
	if (!norm) return '';
	if (/^[A-Za-z]:/.test(norm)) {
		return norm.replace(/\\/g, '/').toLowerCase();
	}
	if (isTildePath(norm)) {
		return norm.replace(/\\/g, '/');
	}
	return norm;
}

function isTildePath(path: string): boolean {
	return path === '~' || path.startsWith('~/') || path.startsWith('~\\');
}

function inferHomePrefix(paths: readonly string[], explicitHome = ''): string {
	const explicit = normalizePath(explicitHome);
	if (explicit) return explicit;
	for (const p of paths) {
		const norm = normalizePath(p);
		if (!norm || isTildePath(norm)) continue;
		const match = norm.match(HOME_DIR_RE);
		if (match?.[1]) return normalizePath(match[1]);
	}
	return '';
}

function resolveWithHome(path: string, homePrefix: string): string {
	const norm = normalizePath(path);
	if (!norm || !homePrefix || !isTildePath(norm)) return norm;
	if (norm === '~') return homePrefix;
	const rest = norm.slice(1).replace(/\\/g, '/');
	return homePrefix.includes('\\') ? `${homePrefix}${rest.replace(/\//g, '\\')}` : `${homePrefix}${rest}`;
}

export function samePath(a: string | undefined | null, b: string | undefined | null): boolean {
	if (!a || !b) return false;
	return pathKey(a) === pathKey(b);
}

/** Format a filesystem path with leading `$HOME` collapsed to `~` for display. */
export function collapseHomeDisplay(path: string, homePrefix = ''): string {
	const norm = normalizePath(path);
	if (!norm) return '';
	const prefix = homePrefix ? normalizePath(homePrefix) : '';
	if (prefix && !isTildePath(norm)) {
		if (pathKey(norm) === pathKey(prefix)) return '~';
		if (pathKey(norm).startsWith(`${pathKey(prefix)}/`)) {
			return `~${norm.slice(prefix.length)}`;
		}
	}
	const match = norm.match(HOME_DIR_RE);
	if (match?.[1]) {
		return norm.length === match[1].length ? '~' : `~${norm.slice(match[1].length)}`;
	}
	return norm;
}

/** Deduplicate recent folders by normalized path (resolving any leftover `~` entries). */
export function dedupeRecentFolders(folders: readonly RecentFolder[], homePrefix = ''): RecentFolder[] {
	const home = inferHomePrefix(folders.map((f) => f.path), homePrefix);
	const out: RecentFolder[] = [];
	const byKey = new Map<string, number>();
	for (const folder of folders) {
		const resolved = resolveWithHome(folder.path, home);
		const key = pathKey(resolved);
		if (!key) continue;
		const existingIdx = byKey.get(key);
		if (existingIdx !== undefined) {
			out[existingIdx] = {
				path: out[existingIdx].path,
				opened: Math.max(out[existingIdx].opened, folder.opened)
			};
			continue;
		}
		byKey.set(key, out.length);
		out.push({ path: resolved, opened: folder.opened });
	}
	return out;
}

function defaultStorage(): ProjectStorage | null {
	try {
		if (typeof window !== 'undefined' && typeof localStorage !== 'undefined') {
			return localStorage;
		}
	} catch {
		// ignore unavailable storage
	}
	return null;
}

function loadProjects(storage: ProjectStorage | null): string[] {
	if (!storage) return [];
	try {
		const raw = storage.getItem(PROJECTS_STORAGE_KEY);
		if (!raw) return [];
		const parsed: unknown = JSON.parse(raw);
		if (!Array.isArray(parsed)) return [];
		const rawStrings = parsed
			.filter((item): item is string => typeof item === 'string' && Boolean(item.trim()))
			.map((item) => normalizePath(item))
			.filter(Boolean);
		const inferredHome = inferHomePrefix(rawStrings);
		const seen: string[] = [];
		for (const item of rawStrings) {
			let resolved = item;
			if (isTildePath(item) && inferredHome) {
				const candidate = resolveWithHome(item, inferredHome);
				if (rawStrings.some((other) => !isTildePath(other) && samePath(other, candidate))) {
					resolved = candidate;
				}
			}
			if (!seen.some((p) => samePath(p, resolved))) seen.push(resolved);
		}
		return seen;
	} catch {
		return [];
	}
}

/** Own the listener for the lifetime of the page, including the open IPC handshake. */
export class Workspaces {
	sessions = $state<Session[]>([]);
	projects = $state<string[]>([]);
	activeIndex = $state(-1);
	opening = $state(false);
	openingCwd = $state<string | null>(null);
	error = $state<string | null>(null);
	#transport: Transport;
	#storage: ProjectStorage | null;
	#homePrefix = '';
	#unlisten: () => void;

	constructor(transport: Transport, storage: ProjectStorage | null = defaultStorage()) {
		this.#transport = transport;
		this.#storage = storage;
		this.projects = loadProjects(storage);
		this.#unlisten = transport.listen((msg) => this.#receive(msg));
		if (this.projects.some(isTildePath)) {
			void this.#repairTildeProjects();
		}
	}

	get homePrefix(): string {
		return this.#homePrefix;
	}

	/** Ordered sidebar projects: saved projects in their prior slots, plus any live tabs. */
	get sidebarProjects(): SidebarProject[] {
		const list: SidebarProject[] = [];
		for (const cwd of this.projects) {
			if (!cwd || list.some((p) => samePath(p.cwd, cwd))) continue;
			const tabIndex = this.sessions.findIndex((s) => samePath(s.cwd, cwd));
			const session = tabIndex >= 0 ? this.sessions[tabIndex] : null;
			list.push({
				cwd,
				session,
				active: tabIndex >= 0 && tabIndex === this.activeIndex
			});
		}
		for (let i = 0; i < this.sessions.length; i++) {
			const session = this.sessions[i];
			const cwd = session.cwd;
			if (!cwd || !list.some((p) => samePath(p.cwd, cwd))) {
				list.push({
					cwd,
					session,
					active: i === this.activeIndex
				});
			}
		}
		return list;
	}

	#expandHome(path: string): string {
		if (!this.#homePrefix || !isTildePath(path)) return path;
		return path === '~' ? this.#homePrefix : `${this.#homePrefix}${path.slice(1)}`;
	}

	#learnHome(fullPath: string, tildePath: string) {
		const full = normalizePath(fullPath);
		const tilde = normalizePath(tildePath);
		if (!full || isTildePath(full) || !isTildePath(tilde)) return;
		const suffix = tilde.slice(1);
		if (!full.endsWith(suffix)) return;
		const home = normalizePath(full.slice(0, full.length - suffix.length));
		if (!home || home === this.#homePrefix) return;
		this.#homePrefix = home;
		let changed = false;
		const next: string[] = [];
		for (const p of this.projects) {
			const expanded = this.#expandHome(p);
			if (expanded !== p) changed = true;
			if (!next.some((existing) => samePath(existing, expanded))) {
				next.push(expanded);
			} else {
				changed = true;
			}
		}
		if (changed) {
			this.projects = next;
			this.#saveProjects();
		}
	}

	async #repairTildeProjects() {
		try {
			const recents = await this.#transport.recentFolders?.(20);
			if (!Array.isArray(recents) || recents.length === 0) return;
			let changed = false;
			const next: string[] = [];
			for (const p of this.projects) {
				let resolved = this.#expandHome(p);
				if (isTildePath(resolved)) {
					const suffix = normalizePath(resolved).slice(1);
					const match = recents.find((r) => {
						const full = normalizePath(r.path);
						return !isTildePath(full) && (suffix ? full.endsWith(suffix) : false);
					});
					if (match) resolved = match.path;
				}
				if (resolved !== p) changed = true;
				if (!next.some((existing) => samePath(existing, resolved))) {
					next.push(resolved);
				} else {
					changed = true;
				}
			}
			if (changed) {
				this.projects = next;
				this.#saveProjects();
			}
		} catch {
			// best-effort repair of stale tilde entries
		}
	}

	#resolveRealCwd(requestedCwd: string | null, infoCwd: string): string {
		if (requestedCwd) this.#learnHome(requestedCwd, infoCwd);
		if (requestedCwd && !isTildePath(requestedCwd)) return requestedCwd;
		if (infoCwd && !isTildePath(infoCwd)) return infoCwd;
		return this.#expandHome(requestedCwd || infoCwd);
	}

	#saveProjects() {
		if (!this.#storage) return;
		try {
			this.#storage.setItem(PROJECTS_STORAGE_KEY, JSON.stringify(this.projects));
		} catch {
			// ignore quota / storage access errors
		}
	}

	#ensureProject(cwd: string) {
		if (!cwd) return;
		const resolved = this.#expandHome(cwd);
		const idx = this.projects.findIndex((p) => samePath(p, resolved));
		if (idx >= 0) {
			if (this.projects[idx] !== resolved) {
				this.projects[idx] = resolved;
				this.#saveProjects();
			}
			return;
		}
		this.projects.push(resolved);
		this.#saveProjects();
	}

	#removeProject(cwd: string) {
		const idx = this.projects.findIndex((p) => samePath(p, cwd));
		if (idx < 0) return;
		this.projects.splice(idx, 1);
		this.#saveProjects();
	}

	#receive(msg: TabMessage) {
		let session = this.sessions.find((s) => s.tab === msg.tab);
		// Info is always the first message. Register the session immediately: replay,
		// trust, and model events can all precede the open_folder command response.
		if (!session && 'info' in msg && this.opening) {
			const realCwd = this.#resolveRealCwd(this.openingCwd, msg.info.cwd);
			session = new Session(this.#transport, msg.tab, msg.info, realCwd);
			this.sessions.push(session);
			this.#ensureProject(realCwd);
		}
		if (!session) return;
		if ('event' in msg) session.onEvent(msg.event);
		else if ('info' in msg) session.info = msg.info;
		else session.onClosed(msg.closed.reason);
	}

	async open(cwd: string, resume: ResumeRequest = { resume: 'new' }) {
		if (this.opening) return;
		const targetCwd = this.#expandHome(cwd);
		this.opening = true;
		this.openingCwd = targetCwd;
		this.error = null;
		try {
			const { tab, info } = await this.#transport.openFolder(targetCwd, resume);
			const realCwd = this.#resolveRealCwd(targetCwd, info.cwd);
			const existing = this.sessions.find((s) => s.tab === tab);
			if (!existing) {
				this.sessions.push(new Session(this.#transport, tab, info, realCwd));
			} else if (!existing.cwd || isTildePath(existing.cwd)) {
				existing.cwd = realCwd;
			}
			this.#ensureProject(realCwd);
			this.activeIndex = this.sessions.findIndex((s) => s.tab === tab);
			await this.sessions[this.activeIndex].listModels();
			await this.sessions[this.activeIndex].listSessions();
		} catch (error) {
			if (isAlreadyOpen(error)) {
				this.activeIndex = this.sessions.findIndex((s) => s.tab === error.detail.tab);
			} else {
				this.error = errorMessage(error);
			}
		} finally {
			this.opening = false;
			this.openingCwd = null;
		}
	}

	/** Focus a live (or closed) workspace tab by its engine tab id. */
	selectTab(tab: number) {
		const idx = this.sessions.findIndex((s) => s.tab === tab);
		if (idx < 0) return false;
		this.activeIndex = idx;
		this.error = null;
		return true;
	}

	/** Restart a stopped workspace tab without briefly dropping to the welcome screen. */
	async reopenTab(tab: number) {
		const stale = this.sessions.find((s) => s.tab === tab);
		if (!stale || this.opening) return;
		const cwd = stale.cwd;
		try {
			await this.#transport.closeTab(stale.tab);
		} catch (error) {
			this.error = errorMessage(error);
			return;
		}
		await this.open(cwd, { resume: 'latest' });
		const staleIndex = this.sessions.indexOf(stale);
		const hasReplacement = this.sessions.some((s) => s !== stale && samePath(s.cwd, cwd));
		if (staleIndex >= 0 && hasReplacement) {
			this.sessions.splice(staleIndex, 1);
			if (this.activeIndex > staleIndex) this.activeIndex--;
			else if (this.activeIndex >= this.sessions.length) this.activeIndex = this.sessions.length - 1;
		}
	}

	/** Focus an already-open sidebar project, or start/resume a remembered dormant one. */
	async selectProject(cwd: string) {
		const idx = this.sessions.findIndex((s) => samePath(s.cwd, cwd));
		if (idx >= 0) {
			this.activeIndex = idx;
			this.error = null;
			return;
		}
		await this.open(cwd, { resume: 'latest' });
	}

	/** Close a live session tab if open and remove the project from the sidebar list. */
	async closeProject(cwd: string) {
		const idx = this.sessions.findIndex((s) => samePath(s.cwd, cwd));
		if (idx >= 0) {
			await this.close(idx);
			return;
		}
		this.#removeProject(cwd);
	}

	async close(index: number) {
		const session = this.sessions[index];
		if (!session) return;
		try {
			await this.#transport.closeTab(session.tab);
			const currentIndex = this.sessions.indexOf(session);
			if (currentIndex >= 0) {
				this.sessions.splice(currentIndex, 1);
				if (this.activeIndex > currentIndex) this.activeIndex--;
				else if (this.activeIndex >= this.sessions.length) this.activeIndex = this.sessions.length - 1;
			}
			const cwd = session.cwd;
			if (cwd && !this.sessions.some((s) => samePath(s.cwd, cwd))) {
				this.#removeProject(cwd);
			}
		} catch (error) {
			this.error = errorMessage(error);
		}
	}

	dispose() {
		this.#unlisten();
	}
}

function isAlreadyOpen(error: unknown): error is { kind: 'already_open'; detail: { tab: number } } {
	if (typeof error !== 'object' || error === null || !('kind' in error) || error.kind !== 'already_open') return false;
	return 'detail' in error && typeof error.detail === 'object' && error.detail !== null &&
		'tab' in error.detail && typeof error.detail.tab === 'number';
}

function errorMessage(error: unknown): string {
	if (typeof error === 'object' && error !== null && 'detail' in error && typeof error.detail === 'string') {
		return error.detail;
	}
	return error instanceof Error ? error.message : String(error);
}
