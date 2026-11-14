<script lang="ts">
	import {
		sortJobsForOverview,
		summarizeJobs,
		type JobActionTarget,
		type JobCategory,
		type JobSummary
	} from '#lib/jobs';
	import type { Session } from '#lib/session.svelte';

	type Filter = 'all' | JobCategory;

	let {
		sessions,
		activeTab,
		opening = false,
		onSelectJob,
		onInterruptJob,
		onDismissIssue,
		onReopenJob,
		onCloseJob,
		onNewProject
	}: {
		sessions: Session[];
		activeTab: number;
		opening?: boolean;
		onSelectJob: (tab: number, target?: JobActionTarget) => void;
		onInterruptJob: (tab: number) => void;
		onDismissIssue: (tab: number) => void;
		onReopenJob: (tab: number) => void;
		onCloseJob: (tab: number) => void;
		onNewProject: () => void;
	} = $props();

	let filter = $state<Filter>('all');

	const counts = $derived(summarizeJobs(sessions));
	const activeFilter = $derived<Filter>(
		filter !== 'all' && counts[filter] === 0 ? 'all' : filter
	);
	const ordered = $derived(sortJobsForOverview(sessions));
	const visible = $derived(
		activeFilter === 'all'
			? ordered
			: ordered.filter(({ summary }) => summary.category === activeFilter)
	);

	const filterOptions = $derived.by(() => {
		const opts: { id: Filter; label: string; count: number }[] = [
			{ id: 'all', label: 'All', count: counts.total }
		];
		if (counts.input > 0) opts.push({ id: 'input', label: 'Input', count: counts.input });
		if (counts.error > 0) opts.push({ id: 'error', label: 'Issues', count: counts.error });
		if (counts.working > 0) opts.push({ id: 'working', label: 'Live', count: counts.working });
		if (counts.ready > 0) opts.push({ id: 'ready', label: 'Ready', count: counts.ready });
		return opts;
	});

	function modelLabel(model: string | null): string | null {
		if (!model) return null;
		const slash = model.indexOf('/');
		return slash >= 0 ? model.slice(slash + 1) : model;
	}

	function formatCost(usd: number): string {
		return usd < 0.01 ? `$${usd.toFixed(4)}` : `$${usd.toFixed(2)}`;
	}

	function showPrimaryAction(summary: JobSummary, isCurrent: boolean): boolean {
		if (summary.canReopen) return false;
		if (!isCurrent) return true;
		if (summary.actionTarget === 'providers') return true;
		return summary.category === 'input';
	}
</script>

<section class="jobs-view" aria-label="Active jobs overview">
	<h2>
		Active jobs
		<span class="count">{counts.total}</span>
		<span class="tools">
			<button type="button" class="mini" disabled={opening} onclick={onNewProject}>+ Open</button>
		</span>
	</h2>

	{#if counts.attention > 0}
		<div class={['triage-banner', { 'has-error': counts.error > 0 }]} role="status">
			{#if counts.input > 0 && counts.error > 0}
				{counts.input} waiting on you · {counts.error} {counts.error === 1 ? 'issue' : 'issues'}
			{:else if counts.input > 0}
				{counts.input} {counts.input === 1 ? 'job needs' : 'jobs need'} your input
			{:else}
				{counts.error} {counts.error === 1 ? 'job hit' : 'jobs hit'} an issue
			{/if}
		</div>
	{/if}

	{#if filterOptions.length > 2}
		<div class="filters" role="group" aria-label="Filter jobs by status">
			{#each filterOptions as opt (opt.id)}
				<button
					type="button"
					class={['pill', { active: activeFilter === opt.id }]}
					aria-pressed={activeFilter === opt.id}
					onclick={() => (filter = opt.id)}
				>
					<span>{opt.label}</span>
					<span class="pill-n">{opt.count}</span>
				</button>
			{/each}
		</div>
	{/if}

	{#if visible.length === 0}
		<div class="empty-card">
			<p>No active jobs match this filter.</p>
			{#if activeFilter !== 'all'}
				<button type="button" class="mini" onclick={() => (filter = 'all')}>Show all jobs</button>
			{/if}
		</div>
	{:else}
		<ul class="cards">
			{#each visible as { summary } (summary.tab)}
				{@const isCurrent = summary.tab === activeTab}
				{@const modelShort = modelLabel(summary.model)}
				{@const hasActions =
					showPrimaryAction(summary, isCurrent) ||
					summary.canInterrupt ||
					summary.canReopen ||
					summary.canDismissIssue}
				<li>
					<article
						class={[`j-card ${summary.category}`, { current: isCurrent }]}
						aria-label={`${summary.title}: ${summary.statusLabel}`}
					>
						<div class="j-top">
							<button
								type="button"
								class="j-head"
								aria-current={isCurrent ? 'true' : undefined}
								onclick={() => onSelectJob(summary.tab, summary.actionTarget)}
							>
								<span class="j-status-row">
									<span class="j-status {summary.category}">
										<span
											class={[
												'j-dot',
												{ pulse: summary.category === 'working' || summary.category === 'input' }
											]}
											aria-hidden="true"
										></span>
										{summary.statusLabel}
									</span>
									{#if isCurrent}
										<span class="j-viewing">viewing</span>
									{/if}
								</span>
								<span class="j-title">{summary.title}</span>
								<span class="j-where">
									<span class="j-proj">{summary.project}</span>
									{#if summary.branch}
										<span class="j-branch">{summary.branch}</span>
									{/if}
								</span>
							</button>
							<button
								type="button"
								class="x"
								aria-label={`Close ${summary.title}`}
								title={`Close ${summary.title}`}
								onclick={() => onCloseJob(summary.tab)}
							>
								<svg viewBox="0 0 12 12" width="11" height="11" aria-hidden="true">
									<path
										d="M3 3l6 6M9 3l-6 6"
										fill="none"
										stroke="currentColor"
										stroke-width="1.4"
										stroke-linecap="round"
									/>
								</svg>
							</button>
						</div>

						{#if summary.detail}
							<p class="j-detail {summary.category}">{summary.detail}</p>
						{/if}

						{#if summary.activeAgents.length > 0}
							<ul class="j-agents" aria-label="Running sub-agents">
								{#each summary.activeAgents.slice(0, 2) as agent (agent.id)}
									<li>
										<span class="j-agent-name">{agent.agent}</span>
										<span class="j-agent-meta">
											{agent.toolCalls} tool{agent.toolCalls === 1 ? '' : 's'}
										</span>
									</li>
								{/each}
								{#if summary.activeAgents.length > 2}
									<li class="more">+{summary.activeAgents.length - 2} more running</li>
								{/if}
							</ul>
						{/if}

						{#if summary.planTotal > 0}
							<div class="j-plan">
								<div class="j-plan-row">
									<span>Plan</span>
									<span class="j-plan-n">{summary.planDone}/{summary.planTotal}</span>
								</div>
								<div class="j-plan-track" aria-hidden="true">
									<div
										class="j-plan-fill"
										style:width={`${Math.round((summary.planDone / summary.planTotal) * 100)}%`}
									></div>
								</div>
							</div>
						{/if}

						{#if modelShort || summary.turns > 0 || summary.filesCount > 0 || summary.contextPct !== null || (summary.costUsd !== null && summary.costUsd > 0)}
							<div class="j-meta">
								{#if modelShort}
									<span class="j-model" title={summary.model ?? undefined}>{modelShort}</span>
								{/if}
								{#if summary.turns > 0}
									<span>{summary.turns}t</span>
								{/if}
								{#if summary.filesCount > 0}
									<span>
										{summary.filesCount}f
										{#if summary.adds > 0 || summary.dels > 0}
											<em class="diff">+{summary.adds}/-{summary.dels}</em>
										{/if}
									</span>
								{/if}
								{#if summary.contextPct !== null}
									<span>{summary.contextPct}% ctx</span>
								{/if}
								{#if summary.costUsd !== null && summary.costUsd > 0}
									<span>{formatCost(summary.costUsd)}</span>
								{/if}
							</div>
						{/if}

						{#if hasActions}
							<div class="j-actions">
								{#if summary.canReopen}
									<button
										type="button"
										class="act primary"
										disabled={opening}
										onclick={() => onReopenJob(summary.tab)}
									>
										Reopen
									</button>
								{:else if showPrimaryAction(summary, isCurrent)}
									<button
										type="button"
										class={['act', { primary: summary.category === 'input' }]}
										onclick={() => onSelectJob(summary.tab, summary.actionTarget)}
									>
										{summary.actionLabel}
									</button>
								{/if}

								{#if summary.canInterrupt}
									<button
										type="button"
										class="act warn"
										onclick={() => onInterruptJob(summary.tab)}
									>
										Stop
									</button>
								{/if}

								{#if summary.canDismissIssue}
									<button
										type="button"
										class="act ghost"
										onclick={() => onDismissIssue(summary.tab)}
									>
										Dismiss
									</button>
								{/if}
							</div>
						{/if}
					</article>
				</li>
			{/each}
		</ul>
	{/if}

	{#if counts.total <= 1}
		<div class="multi-hint">
			<p>Open more project folders to monitor parallel runs, input requests, and errors in one place.</p>
			<button type="button" class="mini wide" disabled={opening} onclick={onNewProject}>
				Open another project…
			</button>
		</div>
	{/if}
</section>

<style>
	.jobs-view {
		display: grid;
		gap: 10px;
	}

	h2 {
		margin: 0;
		font-size: 11px;
		font-weight: 600;
		letter-spacing: 0.14em;
		text-transform: uppercase;
		color: var(--cloth-dim);
		display: flex;
		align-items: center;
		gap: 8px;
		min-height: 22px;
	}

	.count {
		font-family: var(--font-mono);
		font-weight: 400;
		letter-spacing: 0;
		color: var(--cloth-mut);
	}

	.tools {
		margin-left: auto;
		display: inline-flex;
		gap: 6px;
	}

	.mini {
		font-size: 11.5px;
		font-weight: 500;
		color: var(--cloth-ink);
		border: 1px solid rgba(219, 228, 233, 0.35);
		border-radius: 999px;
		padding: 2px 9px;
		letter-spacing: 0;
		text-transform: none;
		transition:
			border-color 120ms ease-out,
			background-color 120ms ease-out;
	}

	.mini:hover:not(:disabled) {
		border-color: rgba(219, 228, 233, 0.7);
		background: rgba(240, 248, 252, 0.06);
	}

	.mini.wide {
		width: 100%;
		text-align: center;
		padding: 5px 9px;
	}

	.triage-banner {
		font-size: 12px;
		font-weight: 500;
		color: #ffefc2;
		background: rgba(232, 178, 26, 0.18);
		border: 1px solid rgba(232, 178, 26, 0.48);
		border-radius: 8px;
		padding: 5px 9px;
		line-height: 1.35;
	}

	.triage-banner.has-error {
		color: #ffe0db;
		background: rgba(235, 131, 118, 0.2);
		border-color: rgba(235, 131, 118, 0.5);
	}

	.filters {
		display: flex;
		flex-wrap: wrap;
		gap: 4px;
	}

	.pill {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		padding: 2px 7px;
		border-radius: 999px;
		border: 1px solid rgba(219, 228, 233, 0.24);
		background: rgba(240, 248, 252, 0.04);
		color: var(--cloth-dim);
		font-size: 10.5px;
		font-weight: 500;
		transition:
			border-color 120ms ease-out,
			background-color 120ms ease-out,
			color 120ms ease-out;
	}

	.pill:hover {
		color: var(--cloth-ink);
		border-color: rgba(219, 228, 233, 0.5);
	}

	.pill.active {
		color: var(--cloth-ink);
		background: rgba(127, 179, 201, 0.18);
		border-color: rgba(127, 179, 201, 0.65);
	}

	.pill-n {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--cloth-mut);
	}

	.pill.active .pill-n {
		color: var(--cloth-ink);
	}

	.cards {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 7px;
	}

	.j-card {
		display: grid;
		gap: 6px;
		padding: 8px 9px 9px;
		border-radius: 10px;
		border: 1px solid rgba(219, 228, 233, 0.24);
		background: var(--cloth-card);
	}

	.j-card.current {
		border-color: rgba(127, 179, 201, 0.58);
	}

	.j-card.input {
		border-color: rgba(232, 178, 26, 0.62);
	}

	.j-card.error {
		border-color: rgba(235, 131, 118, 0.68);
	}

	.j-card.working {
		border-color: rgba(232, 178, 26, 0.42);
	}

	.j-top {
		display: flex;
		align-items: flex-start;
		gap: 4px;
	}

	.j-head {
		flex: 1;
		min-width: 0;
		display: grid;
		gap: 3px;
		text-align: left;
		padding: 1px 2px;
		border-radius: 6px;
	}

	.j-head:hover .j-title,
	.j-head:focus-visible .j-title {
		color: #ffffff;
		text-decoration: underline;
		text-decoration-color: rgba(219, 228, 233, 0.45);
		text-underline-offset: 2px;
	}

	.j-status-row {
		display: flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
	}

	.j-status {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-size: 10.5px;
		font-weight: 600;
		letter-spacing: 0.05em;
		text-transform: uppercase;
		color: var(--cloth-dim);
	}

	.j-status.input {
		color: var(--duck);
	}

	.j-status.error {
		color: var(--cloth-fail);
	}

	.j-status.working {
		color: #f2cf66;
	}

	.j-status.ready {
		color: var(--cloth-ok);
	}

	.j-dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		flex: none;
		background: currentColor;
	}

	.j-dot.pulse {
		animation: jpulse 1.1s ease-out infinite alternate;
	}

	.j-viewing {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: 9.5px;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--cloth-dim);
		border: 1px solid rgba(127, 179, 201, 0.45);
		background: rgba(127, 179, 201, 0.14);
		border-radius: 999px;
		padding: 0 6px;
		line-height: 1.5;
	}

	.j-title {
		font-family: var(--font-serif);
		font-size: 13.5px;
		font-weight: 600;
		line-height: 1.3;
		color: var(--cloth-ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.j-where {
		display: flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: var(--cloth-mut);
	}

	.j-proj {
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.j-branch {
		flex: none;
		max-width: 110px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		padding: 0 5px;
		border-radius: 4px;
		background: rgba(240, 248, 252, 0.06);
		border: 1px solid rgba(219, 228, 233, 0.18);
		color: var(--cloth-dim);
	}

	.x {
		flex: none;
		width: 24px;
		height: 24px;
		display: grid;
		place-items: center;
		border-radius: 6px;
		color: var(--cloth-mut);
		transition:
			color 120ms ease-out,
			background-color 120ms ease-out;
	}

	.x:hover {
		color: var(--cloth-ink);
		background: rgba(240, 248, 252, 0.1);
	}

	.j-detail {
		margin: 0;
		font-size: 11.5px;
		line-height: 1.4;
		color: var(--cloth-dim);
		background: var(--cloth-job);
		border: 1px solid rgba(219, 228, 233, 0.16);
		border-radius: 7px;
		padding: 6px 8px;
		word-break: break-word;
		display: -webkit-box;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.j-detail.input {
		color: #ffefc2;
		background: rgba(232, 178, 26, 0.14);
		border-color: rgba(232, 178, 26, 0.42);
	}

	.j-detail.error {
		color: #ffe0db;
		background: rgba(235, 131, 118, 0.16);
		border-color: rgba(235, 131, 118, 0.42);
	}

	.j-agents {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 4px;
	}

	.j-agents li {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		font-size: 11px;
		color: var(--cloth-dim);
		background: rgba(240, 248, 252, 0.04);
		border: 1px solid rgba(219, 228, 233, 0.14);
		border-radius: 6px;
		padding: 3px 7px;
	}

	.j-agents li.more {
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--cloth-mut);
	}

	.j-agent-name {
		font-family: var(--font-serif);
		font-weight: 600;
		color: var(--cloth-ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.j-agent-meta {
		flex: none;
		font-family: var(--font-mono);
		font-size: 10px;
		color: var(--cloth-mut);
	}

	.j-plan {
		display: grid;
		gap: 4px;
	}

	.j-plan-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		font-size: 10.5px;
		color: var(--cloth-mut);
	}

	.j-plan-n {
		font-family: var(--font-mono);
		color: var(--cloth-dim);
	}

	.j-plan-track {
		height: 4px;
		border-radius: 999px;
		background: rgba(219, 228, 233, 0.16);
		overflow: hidden;
	}

	.j-plan-fill {
		height: 100%;
		border-radius: 999px;
		background: var(--cloth-ok);
	}

	.j-meta {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		gap: 4px 8px;
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: var(--cloth-mut);
	}

	.j-model {
		max-width: 145px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--cloth-dim);
	}

	.diff {
		font-style: normal;
		color: var(--cloth-dim);
	}

	.j-actions {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.act {
		font-size: 11px;
		font-weight: 600;
		padding: 3.5px 9px;
		border-radius: 999px;
		border: 1px solid rgba(219, 228, 233, 0.34);
		color: var(--cloth-ink);
		background: rgba(240, 248, 252, 0.06);
	}

	.act:hover:not(:disabled) {
		border-color: rgba(219, 228, 233, 0.68);
		background: rgba(240, 248, 252, 0.13);
	}

	.act.primary {
		background: var(--duck);
		border-color: var(--duck);
		color: #2b240f;
	}

	.act.primary:hover:not(:disabled) {
		background: #f0be2d;
		border-color: #f0be2d;
	}

	.act.warn {
		color: var(--cloth-fail);
		border-color: rgba(235, 131, 118, 0.48);
		background: rgba(235, 131, 118, 0.12);
	}

	.act.warn:hover:not(:disabled) {
		border-color: var(--cloth-fail);
		background: rgba(235, 131, 118, 0.2);
	}

	.act.ghost {
		color: var(--cloth-dim);
		border-color: rgba(219, 228, 233, 0.22);
		background: transparent;
	}

	.empty-card,
	.multi-hint {
		display: grid;
		gap: 8px;
		padding: 10px;
		border: 1px dashed rgba(219, 228, 233, 0.24);
		border-radius: 9px;
		font-size: 12px;
		line-height: 1.45;
		color: var(--cloth-mut);
	}

	.empty-card p,
	.multi-hint p {
		margin: 0;
	}

	@keyframes jpulse {
		from {
			opacity: 1;
		}
		to {
			opacity: 0.35;
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.j-dot.pulse {
			animation: none;
		}
		.mini,
		.pill,
		.j-card,
		.x,
		.act {
			transition: none;
		}
	}
</style>
