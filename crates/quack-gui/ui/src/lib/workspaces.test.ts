import { describe, expect, it, vi } from 'vitest';
import {
	Workspaces,
	PROJECTS_STORAGE_KEY,
	collapseHomeDisplay,
	dedupeRecentFolders,
	type ProjectStorage
} from './workspaces.svelte';
import { Session } from './session.svelte';
import type { AppInfo, TabMessage, Transport } from './transport/types';

function makeInfo(cwd = '/workspace'): AppInfo {
	return {
		version: 'test', cwd, branch: null, model: 'mock/org/model',
		permission_mode: 'ask', work_mode: 'edit', ui: { theme: 'auto', reduced_motion: false },
		settings: [], layers: [], rules: [], bundles: [], bundles_disabled: []
	};
}

const info = makeInfo();

function memStorage(initial: Record<string, string> = {}): ProjectStorage {
	const map = { ...initial };
	return {
		getItem: (k) => map[k] ?? null,
		setItem: (k, v) => {
			map[k] = v;
		}
	};
}

function fixture(storage: ProjectStorage | null = null) {
	let receive: (message: TabMessage) => void = () => {};
	const unlisten = vi.fn();
	const transport: Transport = {
		kind: 'tauri', pickFolder: vi.fn(), recentFolders: vi.fn(),
		openFolder: vi.fn(), sendOp: vi.fn().mockResolvedValue(undefined),
		closeTab: vi.fn().mockResolvedValue(undefined), openUrl: vi.fn().mockResolvedValue(undefined),
		listen(listener) { receive = listener; return unlisten; }
	};
	const workspaces = new Workspaces(transport, storage);
	return { transport, workspaces, unlisten, receive: (message: TabMessage) => receive(message) };
}

describe('workspace lifecycle', () => {
	it('retains replay and mode events sent before open resolves', async () => {
		const { transport, workspaces, receive } = fixture();
		vi.mocked(transport.openFolder).mockImplementation(async () => {
			receive({ tab: 3, info });
			receive({ tab: 3, event: {
				event: 'session_resumed',
				session: { id: 'saved', cwd: '/workspace', model: null, profile: 'generic' },
				items: [{ kind: 'assistant', text: 'Saved response' }]
			} });
			receive({ tab: 3, event: { event: 'mode_changed', work_mode: 'edit', approvals: 'read-only' } });
			return { tab: 3, info };
		});
		await workspaces.open('/workspace', { resume: 'latest' });
		expect(workspaces.sessions).toHaveLength(1);
		expect(workspaces.sessions[0].items[0]).toMatchObject({ text: 'Saved response' });
		expect(workspaces.sessions[0].approvals).toBe('read-only');
		expect(workspaces.sessions[0].workMode).toBe('edit');
		expect(workspaces.sessions[0].resumed).toBe(true);
	});

	it('focuses the existing workspace on a structured duplicate error', async () => {
		const { transport, workspaces } = fixture();
		workspaces.sessions.push(new Session(transport, 3, info));
		vi.mocked(transport.openFolder).mockRejectedValue({ kind: 'already_open', detail: { tab: 3 } });
		await workspaces.open('/workspace');
		expect(workspaces.activeIndex).toBe(0);
		expect(workspaces.error).toBeNull();
		expect(workspaces.sessions).toHaveLength(1);
	});

	it('marks a stopped tab disconnected and clears pending questions', () => {
		const { transport, workspaces, receive } = fixture();
		const session = new Session(transport, 3, info);
		workspaces.sessions.push(session);
		session.streaming = true;
		session.questions = { callId: 'q', questions: [] };
		receive({ tab: 3, closed: { reason: 'Lost events · reopen the folder' } });
		expect(session.closed).toBe(true);
		expect(session.streaming).toBe(false);
		expect(session.questions).toBeNull();
		expect(session.items[0]).toMatchObject({ tone: 'error' });
	});

	it('releases the page listener on disposal', () => {
		const { workspaces, unlisten } = fixture();
		workspaces.dispose();
		expect(unlisten).toHaveBeenCalledOnce();
	});

	it('keeps provider model ids containing slashes intact', async () => {
		const { transport } = fixture();
		await new Session(transport, 3, info).setModel('mock/org/model');
		expect(transport.sendOp).toHaveBeenCalledWith(3, {
			op: 'set_model', provider: 'mock', model: 'org/model'
		});
	});
	it('seals reasoning when assistant output starts and clears questions on interruption', () => {
		const { transport } = fixture();
		const session = new Session(transport, 3, info);
		session.onEvent({ event: 'reasoning_delta', text: 'Thinking' });
		session.onEvent({ event: 'assistant_delta', text: 'Answer' });
		session.questions = { callId: 'question', questions: [] };
		session.onEvent({ event: 'turn_aborted', turn: 1, reason: 'Interrupted' });
		expect(session.items.filter((item) => 'streaming' in item && item.streaming)).toHaveLength(0);
		expect(session.questions).toBeNull();
	});

	it('keeps previously open projects in sidebar order when reopening another project on restart', async () => {
		const storage = memStorage();
		let nextTab = 1;
		const first = fixture(storage);
		vi.mocked(first.transport.openFolder).mockImplementation(async (cwd) => {
			const tab = nextTab++;
			const tabInfo = makeInfo(cwd);
			first.receive({ tab, info: tabInfo });
			return { tab, info: tabInfo };
		});

		await first.workspaces.open('/projects/a');
		await first.workspaces.open('/projects/b');
		expect(JSON.parse(storage.getItem(PROJECTS_STORAGE_KEY)!)).toEqual(['/projects/a', '/projects/b']);
		first.workspaces.dispose();

		// Simulate restarting the app and picking Project B from the Welcome screen.
		const restarted = fixture(storage);
		let nextRestartTab = 10;
		vi.mocked(restarted.transport.openFolder).mockImplementation(async (cwd) => {
			const tab = nextRestartTab++;
			const tabInfo = makeInfo(cwd);
			restarted.receive({ tab, info: tabInfo });
			return { tab, info: tabInfo };
		});

		await restarted.workspaces.open('/projects/b', { resume: 'latest' });
		expect(restarted.workspaces.sidebarProjects.map((p) => ({
			cwd: p.cwd,
			loaded: p.session !== null,
			active: p.active
		}))).toEqual([
			{ cwd: '/projects/a', loaded: false, active: false },
			{ cwd: '/projects/b', loaded: true, active: true }
		]);

		// Clicking dormant Project A activates it in slot 0 without moving Project B out of slot 1.
		await restarted.workspaces.selectProject('/projects/a');
		expect(restarted.workspaces.sidebarProjects.map((p) => ({
			cwd: p.cwd,
			loaded: p.session !== null,
			active: p.active
		}))).toEqual([
			{ cwd: '/projects/a', loaded: true, active: true },
			{ cwd: '/projects/b', loaded: true, active: false }
		]);

		// Clicking an already-open project switches without spawning another engine.
		await restarted.workspaces.selectProject('/projects/b');
		expect(restarted.transport.openFolder).toHaveBeenCalledTimes(2);
		expect(restarted.workspaces.sidebarProjects[1].active).toBe(true);
	});

	it('appends new projects after remembered ones and removes projects only on close', async () => {
		const storage = memStorage({
			[PROJECTS_STORAGE_KEY]: JSON.stringify(['/projects/a', '/projects/b'])
		});
		const { transport, workspaces, receive } = fixture(storage);
		vi.mocked(transport.openFolder).mockImplementation(async (cwd) => {
			const tabInfo = makeInfo(cwd);
			receive({ tab: 1, info: tabInfo });
			return { tab: 1, info: tabInfo };
		});

		await workspaces.open('/projects/c', { resume: 'latest' });
		expect(workspaces.sidebarProjects.map((p) => p.cwd)).toEqual([
			'/projects/a',
			'/projects/b',
			'/projects/c'
		]);

		// Closing a dormant project removes it from the sidebar list without calling closeTab.
		await workspaces.closeProject('/projects/a');
		expect(transport.closeTab).not.toHaveBeenCalled();
		expect(workspaces.sidebarProjects.map((p) => p.cwd)).toEqual(['/projects/b', '/projects/c']);
		expect(JSON.parse(storage.getItem(PROJECTS_STORAGE_KEY)!)).toEqual(['/projects/b', '/projects/c']);

		// Closing the live project calls closeTab and keeps remaining dormant projects remembered.
		await workspaces.closeProject('/projects/c');
		expect(transport.closeTab).toHaveBeenCalledWith(1);
		expect(workspaces.sessions).toHaveLength(0);
		expect(workspaces.activeIndex).toBe(-1);
		expect(JSON.parse(storage.getItem(PROJECTS_STORAGE_KEY)!)).toEqual(['/projects/b']);
	});

	it('stores real paths and repairs stale tilde paths when AppInfo collapses HOME', async () => {
		const storage = memStorage({
			[PROJECTS_STORAGE_KEY]: JSON.stringify(['~/Development/proj-a', '~/Development/proj-b'])
		});
		const { transport, workspaces, receive } = fixture(storage);
		let nextTab = 1;
		vi.mocked(transport.openFolder).mockImplementation(async (cwd) => {
			const tab = nextTab++;
			const displayCwd = cwd.replace('/home/paul', '~');
			const tabInfo = makeInfo(displayCwd);
			receive({ tab, info: tabInfo });
			return { tab, info: tabInfo };
		});

		await workspaces.open('/home/paul/Development/proj-b', { resume: 'latest' });

		// Must heal stale tilde entries in place instead of creating duplicate or Workspace tabs.
		expect(workspaces.sidebarProjects.map((p) => ({
			cwd: p.cwd,
			loaded: p.session !== null,
			active: p.active
		}))).toEqual([
			{ cwd: '/home/paul/Development/proj-a', loaded: false, active: false },
			{ cwd: '/home/paul/Development/proj-b', loaded: true, active: true }
		]);
		expect(JSON.parse(storage.getItem(PROJECTS_STORAGE_KEY)!)).toEqual([
			'/home/paul/Development/proj-a',
			'/home/paul/Development/proj-b'
		]);

		// Clicking dormant proj-a now opens its real filesystem path in slot 0.
		await workspaces.selectProject(workspaces.sidebarProjects[0].cwd);
		expect(transport.openFolder).toHaveBeenLastCalledWith('/home/paul/Development/proj-a', {
			resume: 'latest'
		});
		expect(workspaces.sidebarProjects.map((p) => ({
			cwd: p.cwd,
			active: p.active
		}))).toEqual([
			{ cwd: '/home/paul/Development/proj-a', active: true },
			{ cwd: '/home/paul/Development/proj-b', active: false }
		]);
	});

	it('deduplicates out-of-sync recent folders across tilde, trailing slashes, and empty entries', () => {
		const deduped = dedupeRecentFolders([
			{ path: '', opened: 99 },
			{ path: '~/Development/quack/', opened: 20 },
			{ path: '/home/paul/Development/quack', opened: 10 },
			{ path: '/home/paul/Development/quack/', opened: 15 },
			{ path: '/home/paul/Development/other/', opened: 12 },
			{ path: 'c:\\Users\\paul\\dev\\', opened: 5 },
			{ path: 'C:/Users/paul/dev', opened: 8 }
		]);
		expect(deduped).toEqual([
			{ path: '/home/paul/Development/quack', opened: 20 },
			{ path: '/home/paul/Development/other', opened: 12 },
			{ path: 'c:\\Users\\paul\\dev', opened: 8 }
		]);
		expect(collapseHomeDisplay('/home/paul/Development/quack', '/home/paul')).toBe('~/Development/quack');
		expect(collapseHomeDisplay('C:\\Users\\paul\\dev')).toBe('~\\dev');
	});

	it('deduplicates out-of-sync sidebar projects stored with tilde, slashes, and full paths', () => {
		const storage = memStorage({
			[PROJECTS_STORAGE_KEY]: JSON.stringify([
				'',
				'~/Development/quack/',
				'/home/paul/Development/quack/',
				'/home/paul/Development/quack',
				'/home/paul/Development/other/'
			])
		});
		const { workspaces } = fixture(storage);
		expect(workspaces.sidebarProjects.map((p) => p.cwd)).toEqual([
			'/home/paul/Development/quack',
			'/home/paul/Development/other'
		]);
	});
});
