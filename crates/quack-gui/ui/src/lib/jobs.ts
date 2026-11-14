import type { ApprovalRequest } from '#lib/protocol';
import type { AgentRun, Session } from '#lib/session.svelte';

export type JobCategory = 'input' | 'error' | 'working' | 'ready';
export type JobActionTarget = 'workspace' | 'providers';

export interface PlanStep {
	state: 'x' | '~' | ' ';
	text: string;
}

export interface JobSummary {
	tab: number;
	cwd: string;
	project: string;
	title: string;
	branch: string | null;
	model: string | null;
	category: JobCategory;
	/** Short status pill label for the overview card. */
	statusLabel: string;
	/** Short phrase used in tight rail spaces like project rows. */
	compactStatus: string;
	/** What needs input, what went wrong, or what is running now. */
	detail: string | null;
	actionLabel: string;
	actionTarget: JobActionTarget;
	activeTool: string | null;
	activeAgents: AgentRun[];
	flaggedAgents: number;
	totalAgents: number;
	planDone: number;
	planTotal: number;
	planCurrent: string | null;
	turns: number;
	filesCount: number;
	adds: number;
	dels: number;
	contextPct: number | null;
	costUsd: number | null;
	canInterrupt: boolean;
	canDismissIssue: boolean;
	canReopen: boolean;
}

export interface JobsCounts {
	total: number;
	input: number;
	error: number;
	working: number;
	ready: number;
	attention: number;
}

const CATEGORY_RANK: Record<JobCategory, number> = {
	input: 0,
	error: 1,
	working: 2,
	ready: 3
};

export function projectName(cwd: string | undefined): string {
	if (!cwd) return 'Workspace';
	const parts = cwd.split(/[\\/]/).filter(Boolean);
	return parts[parts.length - 1] || cwd;
}

export function shortenPath(path: string): string {
	return path.replace(/^\/(?:home|Users)\/[^/]+/, '~');
}

export function compactCount(n: number): string {
	return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`;
}

export function parsePlanSteps(plan: string): PlanStep[] {
	const text = plan.trim();
	if (!text) return [];
	return text.split('\n').map((line) => {
		const match = /^\[([x~ ])\]\s*(.*)$/.exec(line.trim());
		return {
			state: (match?.[1] ?? ' ') as 'x' | '~' | ' ',
			text: match?.[2] ?? line
		};
	});
}

function compactSnippet(text: string, maxLen = 110): string {
	const flat = text.replace(/\s+/g, ' ').trim();
	if (!flat) return '';
	if (flat.length <= maxLen) return flat;
	return `${flat.slice(0, maxLen - 1).trimEnd()}…`;
}

function approvalDetail(request: ApprovalRequest): string {
	switch (request.kind) {
		case 'shell':
			return `${request.outside_sandbox ? 'Outside sandbox: ' : 'Run '}${compactSnippet(request.command, 88)}`;
		case 'edit':
			return `Edit ${request.path}`;
		case 'tool':
			return request.arguments
				? `Tool ${request.name}: ${compactSnippet(request.arguments, 72)}`
				: `Tool ${request.name}`;
	}
}

type Classification = Pick<
	JobSummary,
	| 'category'
	| 'statusLabel'
	| 'compactStatus'
	| 'detail'
	| 'actionLabel'
	| 'actionTarget'
	| 'canInterrupt'
	| 'canDismissIssue'
	| 'canReopen'
>;

function classifyJob(
	session: Session,
	ctx: {
		activeTool: string | null;
		activeAgents: AgentRun[];
		flaggedAgents: number;
		planDone: number;
		planTotal: number;
		planCurrent: string | null;
		latestAssistant: string;
	}
): Classification {
	const base = {
		canInterrupt: false,
		canDismissIssue: false,
		canReopen: false
	};

	if (session.closed) {
		return {
			...base,
			category: 'error',
			statusLabel: 'Stopped',
			compactStatus: 'stopped',
			detail: session.closedReason || 'Workspace process stopped unexpectedly.',
			actionLabel: 'Inspect',
			actionTarget: 'workspace',
			canReopen: true
		};
	}

	if (session.actionError || session.issue?.kind === 'error') {
		return {
			...base,
			category: 'error',
			statusLabel: 'Error',
			compactStatus: 'error',
			detail:
				session.issue?.kind === 'error'
					? session.issue.message
					: (session.actionError ?? 'Action failed.'),
			actionLabel: 'Inspect',
			actionTarget: 'workspace',
			canDismissIssue: true
		};
	}

	if (session.approval) {
		return {
			...base,
			category: 'input',
			statusLabel: 'Needs approval',
			compactStatus: 'needs approval',
			detail: approvalDetail(session.approval.request),
			actionLabel: 'Review & approve',
			actionTarget: 'workspace'
		};
	}

	if (session.questions) {
		const firstQuestion = session.questions.questions[0]?.question ?? 'Waiting for your answer.';
		return {
			...base,
			category: 'input',
			statusLabel: 'Question',
			compactStatus: 'question',
			detail: compactSnippet(firstQuestion, 110),
			actionLabel: 'Answer',
			actionTarget: 'workspace'
		};
	}

	if (session.trust) {
		return {
			...base,
			category: 'input',
			statusLabel: 'Needs trust',
			compactStatus: 'needs trust',
			detail: session.trust.summary,
			actionLabel: 'Review trust',
			actionTarget: 'workspace'
		};
	}

	if (session.signingIn) {
		return {
			...base,
			category: 'input',
			statusLabel: 'Signing in',
			compactStatus: 'signing in',
			detail: `Finish signing in to ${session.signingIn.id} in your browser.`,
			actionLabel: 'Open setup',
			actionTarget: 'providers'
		};
	}

	if (session.signInError) {
		return {
			...base,
			category: 'error',
			statusLabel: 'Sign-in failed',
			compactStatus: 'sign-in failed',
			detail: `${session.signInError.id}: ${session.signInError.message}`,
			actionLabel: 'Open setup',
			actionTarget: 'providers',
			canDismissIssue: true
		};
	}

	if (session.starting) {
		return {
			...base,
			category: 'working',
			statusLabel: 'Starting',
			compactStatus: 'starting…',
			detail: session.startupDetail
				? `Loading ${session.startupDetail}…`
				: 'Starting workspace engine…',
			actionLabel: 'Open',
			actionTarget: 'workspace'
		};
	}

	if (session.switching) {
		return {
			...base,
			category: 'working',
			statusLabel: 'Switching',
			compactStatus: 'switching…',
			detail: 'Loading selected conversation…',
			actionLabel: 'Open',
			actionTarget: 'workspace'
		};
	}

	if (session.setupWorking) {
		return {
			...base,
			category: 'working',
			statusLabel: 'Connecting',
			compactStatus: 'connecting…',
			detail: 'Applying provider or model changes…',
			actionLabel: 'Open setup',
			actionTarget: 'providers'
		};
	}

	if (session.streaming || session.turnPending) {
		let detail: string;
		if (ctx.activeTool) detail = `Running ${ctx.activeTool}`;
		else if (ctx.activeAgents.length === 1) {
			const agent = ctx.activeAgents[0];
			detail = `${agent.agent}: ${compactSnippet(agent.description || agent.text || 'working…', 90)}`;
		} else if (ctx.activeAgents.length > 1) {
			detail = `${ctx.activeAgents.length} sub-agents running`;
		} else if (session.turnPending) {
			detail = 'Waiting for model response…';
		} else if (ctx.latestAssistant) {
			detail = compactSnippet(ctx.latestAssistant, 110);
		} else if (ctx.planCurrent) {
			detail = `Step: ${ctx.planCurrent}`;
		} else {
			detail = 'Thinking…';
		}
		return {
			...base,
			category: 'working',
			statusLabel: 'Working',
			compactStatus: 'working…',
			detail,
			actionLabel: 'View live',
			actionTarget: 'workspace',
			canInterrupt: true
		};
	}

	if (session.issue?.kind === 'aborted') {
		return {
			...base,
			category: 'error',
			statusLabel: 'Interrupted',
			compactStatus: 'interrupted',
			detail: session.issue.message || 'Turn was interrupted.',
			actionLabel: 'Inspect',
			actionTarget: 'workspace',
			canDismissIssue: true
		};
	}

	if (session.issue?.kind === 'warning') {
		return {
			...base,
			category: 'error',
			statusLabel: 'Warning',
			compactStatus: 'warning',
			detail: session.issue.message,
			actionLabel: 'Inspect',
			actionTarget: 'workspace',
			canDismissIssue: true
		};
	}

	if (!session.currentModel) {
		return {
			...base,
			category: 'input',
			statusLabel: 'Choose model',
			compactStatus: 'choose a model',
			detail: 'Connect a provider or pick a model to start.',
			actionLabel: 'Choose model',
			actionTarget: 'providers'
		};
	}

	if (session.workMode === 'plan' && ctx.planTotal > 0) {
		return {
			...base,
			category: 'input',
			statusLabel: 'Plan ready',
			compactStatus: 'plan ready',
			detail: `${ctx.planTotal} step${ctx.planTotal === 1 ? '' : 's'} waiting for review.`,
			actionLabel: 'Review plan',
			actionTarget: 'workspace'
		};
	}

	let readyDetail = 'Ready for your next message.';
	if (ctx.flaggedAgents > 0) {
		readyDetail = `${ctx.flaggedAgents} sub-agent returned with findings.`;
	} else if (ctx.planTotal > 0 && ctx.planDone < ctx.planTotal) {
		readyDetail = `${ctx.planDone} of ${ctx.planTotal} plan steps complete.`;
	} else if (ctx.latestAssistant) {
		readyDetail = compactSnippet(ctx.latestAssistant, 110);
	}

	return {
		...base,
		category: 'ready',
		statusLabel: 'Ready',
		compactStatus: 'ready',
		detail: readyDetail,
		actionLabel: 'Open',
		actionTarget: 'workspace'
	};
}

/** Build the single-source-of-truth status and activity snapshot for one open job tab. */
export function describeJob(session: Session): JobSummary {
	const runningTool = session.items.findLast((item) => item.kind === 'tool' && item.ok === null);
	const activeTool =
		runningTool && runningTool.kind === 'tool'
			? runningTool.summary || runningTool.name
			: null;

	const assistantItem = session.items.findLast((item) => item.kind === 'assistant' && item.text.trim());
	const latestAssistant = assistantItem && assistantItem.kind === 'assistant' ? assistantItem.text : '';

	const activeAgents = session.agents.filter((a) => a.ok === null);
	const flaggedAgents = session.agents.filter((a) => a.ok === false).length;

	const planSteps = parsePlanSteps(session.plan);
	const planDone = planSteps.filter((step) => step.state === 'x').length;
	const planCurrent = planSteps.find((step) => step.state === '~')?.text ?? null;

	const files = session.editedFiles;
	let adds = 0;
	let dels = 0;
	for (const file of files) {
		adds += file.adds;
		dels += file.dels;
	}

	const contextBase = session.context ? (session.context.budget ?? session.context.window) : null;
	const contextPct =
		session.context && contextBase
			? Math.min(100, Math.round((session.context.used / contextBase) * 100))
			: null;

	const turns = session.stats?.turns ?? session.items.filter((item) => item.kind === 'user').length;
	const costUsd = session.stats?.cost ?? null;

	const classification = classifyJob(session, {
		activeTool,
		activeAgents,
		flaggedAgents,
		planDone,
		planTotal: planSteps.length,
		planCurrent,
		latestAssistant
	});

	const project = projectName(session.cwd);
	return {
		tab: session.tab,
		cwd: session.cwd,
		project,
		title: session.title || project,
		branch: session.info?.branch ?? null,
		model: session.currentModel,
		activeTool,
		activeAgents,
		flaggedAgents,
		totalAgents: session.agents.length,
		planDone,
		planTotal: planSteps.length,
		planCurrent,
		turns,
		filesCount: files.length,
		adds,
		dels,
		contextPct,
		costUsd,
		...classification
	};
}

export interface OrderedJob {
	session: Session;
	summary: JobSummary;
}

/** Order jobs with attention items first (needs input, then errors), then working, then ready. */
export function sortJobsForOverview(sessions: Session[]): OrderedJob[] {
	return sessions
		.map((session, index) => ({ session, summary: describeJob(session), index }))
		.sort((a, b) => {
			const rankDiff = CATEGORY_RANK[a.summary.category] - CATEGORY_RANK[b.summary.category];
			return rankDiff !== 0 ? rankDiff : a.index - b.index;
		})
		.map(({ session, summary }) => ({ session, summary }));
}

export function summarizeJobs(sessions: Session[]): JobsCounts {
	const counts: JobsCounts = {
		total: sessions.length,
		input: 0,
		error: 0,
		working: 0,
		ready: 0,
		attention: 0
	};
	for (const session of sessions) {
		const { category } = describeJob(session);
		counts[category] += 1;
	}
	counts.attention = counts.input + counts.error;
	return counts;
}
