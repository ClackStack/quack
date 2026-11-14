import { describe, expect, it, vi } from 'vitest';
import { describeJob, parsePlanSteps, sortJobsForOverview, summarizeJobs } from './jobs';
import { Session } from './session.svelte';
import type { AppInfo, Transport } from './transport/types';

function makeSession(
	tab: number,
	overrides: { cwd?: string; model?: string | null; branch?: string | null } = {}
) {
	const cwd = overrides.cwd ?? `/workspace/proj-${tab}`;
	const model = overrides.model === undefined ? 'mock/model' : overrides.model;
	const info: AppInfo = {
		version: 'test',
		cwd,
		branch: overrides.branch ?? 'main',
		model,
		permission_mode: 'ask',
		work_mode: 'edit',
		ui: { theme: 'auto', reduced_motion: false },
		settings: [],
		layers: [],
		rules: [],
		bundles: [],
		bundles_disabled: []
	};
	const transport: Transport = {
		kind: 'tauri',
		pickFolder: vi.fn(),
		recentFolders: vi.fn(),
		openFolder: vi.fn(),
		sendOp: vi.fn().mockResolvedValue(undefined),
		closeTab: vi.fn(),
		openUrl: vi.fn(),
		listen: () => () => {}
	};
	const session = new Session(transport, tab, info, cwd);
	session.onEvent({ event: 'session_configured', session: { id: `s-${tab}`, cwd, model, profile: 'generic' } });
	session.onEvent({ event: 'startup_complete' });
	return session;
}

describe('parsePlanSteps', () => {
	it('parses done, current, and pending plan lines', () => {
		expect(parsePlanSteps('[x] read files\n[~] edit lib\n[ ] run tests')).toEqual([
			{ state: 'x', text: 'read files' },
			{ state: '~', text: 'edit lib' },
			{ state: ' ', text: 'run tests' }
		]);
		expect(parsePlanSteps('   ')).toEqual([]);
	});
});

describe('describeJob', () => {
	it('flags stopped tabs with their reason and allows reopen', () => {
		const session = makeSession(1);
		session.onClosed('Engine panicked cleanly');
		const summary = describeJob(session);
		expect(summary).toMatchObject({
			category: 'error',
			statusLabel: 'Stopped',
			compactStatus: 'stopped',
			detail: 'Engine panicked cleanly',
			canReopen: true
		});
	});

	it('classifies approval requests, questions, trust, and sign-in as input', () => {
		const shellJob = makeSession(1);
		shellJob.onEvent({
			event: 'approval_request',
			call_id: 'c1',
			request: { kind: 'shell', command: 'cargo publish', cwd: '/workspace/proj-1', outside_sandbox: true },
			always: []
		});
		expect(describeJob(shellJob)).toMatchObject({
			category: 'input',
			statusLabel: 'Needs approval',
			actionLabel: 'Review & approve',
			actionTarget: 'workspace'
		});
		expect(describeJob(shellJob).detail).toContain('Outside sandbox: cargo publish');

		const questionJob = makeSession(2);
		questionJob.onEvent({
			event: 'questions',
			call_id: 'q1',
			questions: [{ header: 'DB', question: 'Use sqlite or postgres?', options: [] }]
		});
		expect(describeJob(questionJob)).toMatchObject({
			category: 'input',
			statusLabel: 'Question',
			detail: 'Use sqlite or postgres?',
			actionLabel: 'Answer'
		});

		const trustJob = makeSession(3);
		trustJob.onEvent({
			event: 'project_trust',
			path: '/workspace/.quack',
			summary: '1 local MCP command',
			changed: false
		});
		expect(describeJob(trustJob)).toMatchObject({
			category: 'input',
			statusLabel: 'Needs trust',
			detail: '1 local MCP command'
		});

		const authJob = makeSession(4, { model: null });
		authJob.signingIn = { id: 'openai', url: 'https://example.test' };
		expect(describeJob(authJob)).toMatchObject({
			category: 'input',
			statusLabel: 'Signing in',
			actionTarget: 'providers'
		});
	});

	it('prioritizes errors ahead of approvals and tracks active tools, sub-agents, plan, and stats', () => {
		const working = makeSession(1);
		working.onEvent({ event: 'session_title', title: 'Fix stream bug' });
		working.onEvent({ event: 'turn_started', turn: 1 });
		working.onEvent({ event: 'plan', checklist: '[x] inspect\n[~] patch\n[ ] test' });
		working.onEvent({
			event: 'tool_call_begin',
			call: { call_id: 't1', name: 'shell', summary: 'cargo test -p quack-core' }
		});
		working.onEvent({
			event: 'agent_started',
			id: 'ag1',
			agent: 'explore',
			description: 'find callers',
			call_id: 't2',
			model: 'mock/model'
		});
		working.onEvent({
			event: 'context_update',
			used: 500,
			budget: 1000,
			window: 2000,
			cache_hit_tokens: null
		});

		const w = describeJob(working);
		expect(w).toMatchObject({
			category: 'working',
			statusLabel: 'Working',
			detail: 'Running cargo test -p quack-core',
			activeTool: 'cargo test -p quack-core',
			planDone: 1,
			planTotal: 3,
			planCurrent: 'patch',
			contextPct: 50,
			canInterrupt: true
		});
		expect(w.activeAgents).toHaveLength(1);

		working.onEvent({ event: 'error', error: { kind: 'provider', message: 'Rate limited (429)' } });
		expect(describeJob(working)).toMatchObject({
			category: 'error',
			statusLabel: 'Error',
			detail: 'Rate limited (429)',
			canDismissIssue: true,
			canInterrupt: false
		});
	});

	it('identifies plan-ready and missing-model jobs cleanly', () => {
		const noModel = makeSession(1, { model: null });
		expect(describeJob(noModel)).toMatchObject({
			category: 'input',
			statusLabel: 'Choose model',
			actionTarget: 'providers'
		});

		const planned = makeSession(2);
		planned.onEvent({ event: 'mode_changed', work_mode: 'plan', approvals: 'ask' });
		planned.onEvent({ event: 'plan', checklist: '[ ] step 1\n[ ] step 2' });
		expect(describeJob(planned)).toMatchObject({
			category: 'input',
			statusLabel: 'Plan ready',
			detail: '2 steps waiting for review.'
		});
	});

	it('reads cost and edit stats for settled jobs', () => {
		const done = makeSession(5);
		done.onEvent({ event: 'turn_started', turn: 1 });
		done.onEvent({
			event: 'tool_call_begin',
			call: { call_id: 'e1', name: 'edit', summary: 'edit src/lib.rs' }
		});
		done.onEvent({
			event: 'tool_call_end',
			call_id: 'e1',
			result: {
				ok: true,
				summary: 'edited',
				detail: '--- a/src/lib.rs\n+++ b/src/lib.rs\n@@\n+one\n+two\n-old'
			}
		});
		done.onEvent({ event: 'turn_complete', turn: 1, usage: { prompt_tokens: 100, completion_tokens: 20 } });
		done.onEvent({
			event: 'stats',
			report: {
				model: 'mock/model',
				turns: 3,
				requests: 4,
				unreported: 0,
				prompt_tokens: 1200,
				completion_tokens: 300,
				cached_tokens: 600,
				cache_reported: 4,
				cost: 0.12,
				unpriced: 0,
				open_ms: 1000,
				model_ms: 400,
				tools_ms: 200,
				approval_ms: 0,
				tools: [],
				compactions: 0,
				pruned: 0,
				resumed: false
			}
		});

		expect(describeJob(done)).toMatchObject({
			category: 'ready',
			turns: 3,
			filesCount: 1,
			adds: 2,
			dels: 1,
			costUsd: 0.12
		});
	});
});

describe('sortJobsForOverview and summarizeJobs', () => {
	it('orders input -> error -> working -> ready and counts attention accurately', () => {
		const readyJob = makeSession(1);
		const workingJob = makeSession(2);
		workingJob.onEvent({ event: 'turn_started', turn: 1 });
		const errorJob = makeSession(3);
		errorJob.onEvent({ event: 'error', error: { kind: 'config', message: 'Bad key' } });
		const inputJob = makeSession(4);
		inputJob.onEvent({
			event: 'approval_request',
			call_id: 'a1',
			request: { kind: 'edit', path: 'foo.ts', diff: '+x' },
			always: []
		});

		const list = [readyJob, workingJob, errorJob, inputJob];
		const ordered = sortJobsForOverview(list);
		expect(ordered.map((o) => o.summary.tab)).toEqual([4, 3, 2, 1]);

		expect(summarizeJobs(list)).toEqual({
			total: 4,
			input: 1,
			error: 1,
			working: 1,
			ready: 1,
			attention: 2
		});
	});
});
