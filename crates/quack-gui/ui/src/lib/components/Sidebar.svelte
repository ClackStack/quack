<script lang="ts">
	import iconUrl from '#lib/assets/icon.png';
	import type { Session } from '#lib/session.svelte';
	import { samePath, type SidebarProject } from '#lib/workspaces.svelte';

	let {
		session,
		projects,
		opening = false,
		openingCwd = null,
		onSelectProject,
		onCloseProject,
		onNewTab
	}: {
		session: Session;
		projects: SidebarProject[];
		opening?: boolean;
		openingCwd?: string | null;
		onSelectProject: (cwd: string) => void;
		onCloseProject: (cwd: string) => void;
		onNewTab: () => void;
	} = $props();

	let contextOpen = $state(false);

	const planItems = $derived.by(() => {
		const text = session.plan.trim();
		if (!text) return [];
		return text.split('\n').map((line) => {
			const match = /^\[([x~ ])\]\s*(.*)$/.exec(line.trim());
			return {
				state: (match?.[1] ?? ' ') as 'x' | '~' | ' ',
				text: match?.[2] ?? line
			};
		});
	});
	const doneCount = $derived(planItems.filter((p) => p.state === 'x').length);

	const busy = $derived(session.streaming || session.turnPending);
	const jobStatus = $derived.by(() => {
		if (session.closed) return 'closed';
		if (session.starting) return 'starting…';
		if (session.switching) return 'switching…';
		if (session.approval || session.questions) return 'waiting on you';
		if (busy) return 'working…';
		if (session.trust) return 'needs your trust';
		if (!session.currentModel) return 'choose a model';
		return 'ready';
	});

	const disabled = $derived(
		session.closed ||
			session.starting ||
			session.switching ||
			session.setupWorking ||
			busy ||
			!!session.trust ||
			!session.currentModel
	);

	// ---- context ----
	const gauge = $derived.by(() => {
		const c = session.context;
		if (!c) return null;
		const base = c.budget ?? c.window;
		if (!base) return null;
		const used = Math.min(1, c.used / base);
		const threshold = c.threshold ? Math.min(1, c.threshold / base) : null;
		const near = c.threshold !== null && c.used >= c.threshold * 0.9;
		return { used, threshold, near, usedTokens: c.used, base };
	});

	const fmtTokens = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`);

	// ---- usage ----
	const stats = $derived(session.stats);
	const spark = $derived.by(() => {
		const history = stats?.history ?? [];
		if (history.length < 2) return [];
		const max = Math.max(...history.map((r) => r.prompt + r.completion));
		return history.map((r) => ({
			cached: r.cached / max,
			fresh: (r.prompt - r.cached + r.completion) / max
		}));
	});
	const cachedShare = $derived.by(() => {
		if (!stats || stats.prompt_tokens === 0 || stats.cache_reported === 0) return null;
		return stats.cached_tokens / stats.prompt_tokens;
	});

	// ---- files ----
	const files = $derived(session.editedFiles);

	function projectName(cwd: string | undefined): string {
		if (!cwd) return 'Workspace';
		const parts = cwd.split(/[\\/]/).filter(Boolean);
		return parts[parts.length - 1] || cwd;
	}
	function shorten(path: string): string {
		return path.replace(/^\/home\/[^/]+/, '~');
	}
</script>

<aside class="rail cloth" aria-label="Projects">
	<div class="brand" title={`_(•)> quack ${session.info?.version ?? ''}`}>
		<img class="brand-duck" src={iconUrl} alt="" />
		<span class="brand-name">quack</span>
	</div>

	<section>
		<h2>Projects</h2>
		<button class="open" disabled={opening} onclick={onNewTab}>
			<svg viewBox="0 0 24 24" aria-hidden="true"
				><path
					d="M3.5 7.2c0-1 .8-1.7 1.7-1.7h4.2l2 2.2h7.4c1 0 1.7.8 1.7 1.7v8.4c0 1-.8 1.7-1.7 1.7H5.2c-1 0-1.7-.8-1.7-1.7z"
					fill="none"
					stroke="currentColor"
					stroke-width="1.7"
					stroke-linejoin="round"
				/><path
					d="M12.5 11.5v5M10 14h5"
					fill="none"
					stroke="currentColor"
					stroke-width="1.7"
					stroke-linecap="round"
				/></svg
			>
			Open a project…
		</button>
		<ul class="projects">
			{#each projects as item (item.session ? `live-${item.session.tab}` : item.cwd)}
				{@const tab = item.session}
				{@const isOpening = opening && samePath(openingCwd, item.cwd)}
				<li>
					{#if item.active}
						<!-- The open project is a card that contains its jobs: jobs are the
						     conversations that run inside a project. -->
						<div class="p-card">
							<div class="p-row p-head-row">
								<button class="project active" aria-current="true" onclick={() => onSelectProject(item.cwd)}>
									<span class="p-head">
										<span class="p-dot" aria-hidden="true"></span>
										<span class="p-name">{projectName(item.cwd)}</span>
									</span>
									<span class="p-note">
										{shorten(item.cwd)}
										{tab?.streaming ? ' · working' : ''}
									</span>
								</button>
								<button
									class="p-close"
									aria-label={`Close ${projectName(item.cwd)}`}
									onclick={() => onCloseProject(item.cwd)}>×</button
								>
							</div>
							<div class="jobs">
								<div class="jobs-head">
									<h3>Jobs</h3>
									<span class="tools">
										<button class="mini" {disabled} onclick={() => void session.newSession()}>New job</button>
										<button
											class="mini"
											disabled={session.closed}
											onclick={() => void session.listSessions()}
											title="Refresh the job list">Refresh</button
										>
									</span>
								</div>
								<p class="s-note jobs-hint">each job is one conversation in this project</p>
								<div class="job live" class:idle={!busy && !session.approval && !session.questions}>
									<div class="j-title">{session.title || 'New job'}</div>
									<div class="j-note">
										{#if busy || session.approval || session.questions}<span class="dot" aria-hidden="true"></span
											>{/if}
										{jobStatus}
									</div>
								</div>
								<ul class="saved">
									{#each session.sessions as saved (saved.id)}
										<li>
											<button
												class:current={saved.current}
												disabled={disabled || saved.current}
												onclick={() => void session.resumeSession(saved.id)}
												title={saved.current ? 'This is the open job' : `Resume “${saved.title || 'Untitled'}”`}
											>
												<span class="j-title">{saved.title || 'Untitled'}</span>
												<span class="j-note">{saved.model} · {saved.turns} turns · {saved.age}</span>
											</button>
										</li>
									{:else}
										<li class="j-note plain">no other jobs yet</li>
									{/each}
								</ul>
							</div>
						</div>
					{:else}
						<div class="p-row idle">
							<button
								class="project"
								disabled={opening}
								onclick={() => onSelectProject(item.cwd)}
								title={tab ? `Switch to ${projectName(item.cwd)}` : `Open ${projectName(item.cwd)}`}
							>
								<span class="p-main">
									<span class="p-head">
										<svg class="p-ico" viewBox="0 0 24 24" aria-hidden="true"
											><path
												d="M3.5 7.2c0-1 .8-1.7 1.7-1.7h4.2l2 2.2h7.4c1 0 1.7.8 1.7 1.7v8.4c0 1-.8 1.7-1.7 1.7H5.2c-1 0-1.7-.8-1.7-1.7z"
												fill="none"
												stroke="currentColor"
												stroke-width="1.7"
												stroke-linejoin="round"
											/></svg
										>
										<span class="p-name">{projectName(item.cwd)}</span>
									</span>
									<span class="p-note">
										{shorten(item.cwd)}
										{#if isOpening} · opening…{:else if tab?.streaming} · working{/if}
									</span>
								</span>
								<span class="p-go" aria-hidden="true">›</span>
							</button>
							<button
								class="p-close"
								disabled={isOpening}
								aria-label={`Close ${projectName(item.cwd)}`}
								onclick={() => onCloseProject(item.cwd)}>×</button
							>
						</div>
					{/if}
				</li>
			{/each}
		</ul>
	</section>

	<section>
		<h2>
			Context
			{#if gauge}<span class="pct" class:near={gauge.near}
					>{Math.round(gauge.used * 100)}%</span
				>{/if}
		</h2>
		{#if gauge}
			<button
				class="gauge-wrap"
				aria-expanded={contextOpen}
				onclick={() => {
					if (!contextOpen && !session.contextReport) void session.refreshContext();
					contextOpen = !contextOpen;
				}}
			>
				<div
					class="gauge"
					role="img"
					aria-label={`Context ${fmtTokens(gauge.usedTokens)} of ${fmtTokens(gauge.base)} tokens`}
				>
					<div class="g-fill" class:near={gauge.near} style:--fill={gauge.used}></div>
					{#if gauge.threshold !== null}
						<div class="g-thresh" style:--t={gauge.threshold} title="Pruning starts here"></div>
					{/if}
				</div>
				<div class="g-note">{fmtTokens(gauge.usedTokens)} of {fmtTokens(gauge.base)} tokens</div>
			</button>
			{#if contextOpen}
				<div class="segments">
					{#if session.contextReport}
						{#each session.contextReport.segments as segment (segment.name)}
							<div class="segment">
								<span class="s-name">{segment.name}</span>
								<span class="s-tokens">{fmtTokens(segment.tokens)}</span>
								<span class="s-detail">{segment.detail}</span>
							</div>
						{/each}
						{#if session.contextReport.note}
							<p class="s-note">{session.contextReport.note}</p>
						{/if}
					{:else}
						<p class="s-note">reading the context…</p>
					{/if}
					<button
						class="mini wide"
						disabled={disabled}
						onclick={() => void session.compact()}
						title="Summarize older history into a shorter carry">Compact now</button
					>
				</div>
			{/if}
		{:else}
			<p class="s-note">no context yet — start a job</p>
		{/if}
	</section>

	{#if stats}
		<section>
			<h2>Usage</h2>
			<dl class="log">
				<div><dt>cost</dt><dd>{stats.cost !== undefined ? `$${stats.cost.toFixed(2)}` : '—'}</dd></div>
				<div><dt>turns</dt><dd>{stats.turns}</dd></div>
				<div><dt>sent</dt><dd>{fmtTokens(stats.prompt_tokens + stats.completion_tokens)}</dd></div>
				<div>
					<dt>from cache</dt>
					<dd>{cachedShare !== null ? `${Math.round(cachedShare * 100)}%` : '—'}</dd>
				</div>
				{#if stats.tokens_per_sec}
					<div><dt>speed</dt><dd>{stats.tokens_per_sec.toFixed(0)} tok/s</dd></div>
				{/if}
				{#if session.speed}
					<div><dt>last call</dt><dd>{session.speed.ttftMs} ms to first token</dd></div>
				{/if}
			</dl>
			{#if spark.length > 1}
				<svg class="spark" viewBox="0 0 {spark.length * 8} 26" preserveAspectRatio="none" aria-hidden="true">
					{#each spark as bar, i (i)}
						<rect x={i * 8 + 1} y={26 - bar.cached * 24} width="6" height={bar.cached * 24} class="sp-cached" />
						<rect x={i * 8 + 1} y={26 - (bar.cached + bar.fresh) * 24} width="6" height={bar.fresh * 24} class="sp-fresh" />
					{/each}
				</svg>
				<p class="s-note">each bar one model call · lighter share served from cache</p>
			{/if}
		</section>
	{/if}

	{#if files.length > 0}
		<section>
			<h2>
				Files
				<span class="count">{files.length}</span>
				<button
					class="mini"
					disabled={disabled}
					onclick={() => void session.undo()}
					title="Undo the last edit call">Undo last</button
				>
			</h2>
			<p class="s-note">files this job has changed</p>
			<ul class="files">
				{#each files as file (file.path)}
					<li>
						<details>
							<summary>
								<span class="f-path">{file.path}</span>
								<span class="f-counts">
									<span class="f-add">+{file.adds}</span>
									<span class="f-del">−{file.dels}</span>
								</span>
							</summary>
							{#if file.diff}
								<pre class="f-diff">{file.diff}</pre>
							{:else}
								<p class="s-note">{file.ops} change{file.ops === 1 ? '' : 's'}</p>
							{/if}
						</details>
					</li>
				{/each}
			</ul>
		</section>
	{/if}

	{#if session.skills.length > 0}
		<section>
			<details class="kit">
				<summary><h2>Skills<span class="count">{session.skills.length}</span></h2></summary>
				<ul class="skills">
					{#each session.skills as skill (skill.name)}
						<li>
							<span class="k-name">{skill.name}</span>
							<span class="k-desc">{skill.description}</span>
						</li>
					{/each}
				</ul>
			</details>
		</section>
	{/if}

	{#if planItems.length > 0}
		<section>
			<h2>Plan <span class="count">{doneCount}/{planItems.length}</span></h2>
			<ol class="plan">
				{#each planItems as item, i (i)}
					<li class:done={item.state === 'x'} class:doing={item.state === '~'}>
						<span class="mark" aria-hidden="true">{i + 1}</span>
						<span class="p-text">{item.text}</span>
						{#if item.state === '~'}<span class="tag">now</span>{/if}
					</li>
				{/each}
			</ol>
		</section>
	{/if}

	{#if session.agents.length > 0}
		<section>
			<h2>Sub-agents</h2>
			<ul class="agents">
				{#each session.agents as run (run.id)}
					<li>
						<div class="a-head">
							<span class="dot" class:still={run.ok !== null} aria-hidden="true"></span>
							<span class="a-name">{run.agent}</span>
						</div>
						<p class="a-desc">{run.description}</p>
						{#if run.text}<p class="a-text">{run.text.slice(-200)}</p>{/if}
						{#if run.summary}<p class="a-sum">{run.ok === false ? 'returned with findings' : ''} {run.summary}</p>{/if}
					</li>
				{/each}
			</ul>
		</section>
	{/if}
</aside>

<style>
	.rail {
		min-height: 0;
		overflow-y: auto;
		color: var(--cloth-ink);
		padding: 16px 14px 28px;
		display: grid;
		gap: 22px;
		align-content: start;
		border-right: 1px solid var(--cloth-deep);
	}

	.brand {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 2px 2px 0;
		user-select: none;
	}

	.brand-duck {
		width: 27px;
		height: 27px;
		flex: none;
		display: block;
	}

	.brand-name {
		font-family: var(--font-serif);
		font-weight: 700;
		font-size: 19px;
		letter-spacing: -0.01em;
		color: #f2f6f8;
	}

	section {
		display: grid;
		gap: 8px;
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
		transition: border-color 120ms ease-out, background-color 120ms ease-out;
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

	/* projects and the jobs inside them */
	.projects {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 6px;
	}

	.projects li {
		display: grid;
		gap: 4px;
	}

	.p-row {
		display: flex;
		align-items: stretch;
		gap: 4px;
	}

	.p-row.idle {
		border: 1px solid rgba(219, 228, 233, 0.28);
		border-radius: 9px;
		background: rgba(240, 248, 252, 0.06);
		transition:
			border-color 120ms ease-out,
			background-color 120ms ease-out;
	}

	.p-row.idle:has(> .project:hover:not(:disabled)),
	.p-row.idle:has(> .project:focus-visible) {
		background: rgba(240, 248, 252, 0.13);
		border-color: rgba(219, 228, 233, 0.62);
	}

	.p-row.idle .project:disabled {
		opacity: 0.72;
		cursor: wait;
	}

	/* The open project is the card that holds its jobs. */
	.p-card {
		display: grid;
		gap: 10px;
		padding: 9px 10px 10px;
		border: 1px solid rgba(127, 179, 201, 0.45);
		border-radius: 10px;
		background: var(--cloth-card);
	}

	.p-head-row {
		padding-bottom: 8px;
		border-bottom: 1px solid rgba(219, 228, 233, 0.14);
	}

	.p-card .project {
		padding: 2px 3px;
		cursor: default;
	}

	.p-card .project.active {
		background: transparent;
		border-color: transparent;
	}

	.project {
		flex: 1;
		min-width: 0;
		display: grid;
		gap: 2px;
		text-align: left;
		padding: 8px 4px 8px 10px;
		border: 1px solid transparent;
		border-radius: 9px;
	}

	.p-row.idle .project {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 8px 4px 8px 10px;
	}

	.p-main {
		flex: 1;
		min-width: 0;
		display: grid;
		gap: 2px;
	}

	.p-head {
		display: flex;
		align-items: center;
		gap: 7px;
		min-width: 0;
	}

	.p-dot {
		width: 7px;
		height: 7px;
		border-radius: 50%;
		background: var(--duck);
		flex: none;
	}

	.p-ico {
		width: 14px;
		height: 14px;
		flex: none;
		color: var(--cloth-dim);
	}

	.p-go {
		flex: none;
		color: var(--cloth-dim);
		font-size: 15px;
		line-height: 1;
		transition:
			color 120ms ease-out,
			transform 120ms ease-out;
	}

	.p-row.idle:has(> .project:hover) .p-go,
	.p-row.idle:has(> .project:hover) .p-ico {
		color: var(--cloth-ink);
	}

	.p-row.idle:has(> .project:hover) .p-go {
		transform: translateX(1px);
	}

	.p-name {
		font-family: var(--font-serif);
		font-size: 14px;
		font-weight: 600;
		line-height: 1.3;
		color: var(--cloth-ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.p-note {
		font-size: 11px;
		color: var(--cloth-mut);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.project.active .p-note {
		color: var(--cloth-dim);
	}

	.p-close {
		align-self: center;
		width: 22px;
		height: 22px;
		margin-right: 4px;
		border-radius: 6px;
		color: var(--cloth-mut);
		font-size: 14px;
		line-height: 1;
		opacity: 0.55;
		transition:
			opacity 120ms ease-out,
			background-color 120ms ease-out,
			color 120ms ease-out;
	}

	.p-card .p-close {
		margin-right: 0;
		opacity: 0.45;
	}

	.projects li:hover .p-close,
	.p-close:focus-visible {
		opacity: 1;
	}

	.p-close:hover {
		color: var(--cloth-ink);
		background: rgba(240, 248, 252, 0.1);
	}

	.open {
		display: flex;
		align-items: center;
		gap: 9px;
		padding: 9px 11px;
		border: 1px dashed rgba(219, 228, 233, 0.45);
		border-radius: 9px;
		background: rgba(240, 248, 252, 0.04);
		font-size: 12.5px;
		font-weight: 500;
		color: var(--cloth-ink);
		width: 100%;
		transition:
			border-color 120ms ease-out,
			background-color 120ms ease-out;
	}

	.open:hover:not(:disabled) {
		background: rgba(240, 248, 252, 0.1);
		border-color: rgba(219, 228, 233, 0.78);
	}

	.open:disabled {
		opacity: 0.65;
		cursor: wait;
	}

	.open svg {
		width: 15px;
		height: 15px;
		flex: none;
		color: var(--cloth-dim);
	}

	/* jobs: one connector line down from the project card */
	.jobs {
		display: grid;
		gap: 8px;
		margin-left: 5px;
		padding-left: 10px;
		border-left: 1px solid rgba(219, 228, 233, 0.24);
	}

	.jobs-head {
		display: flex;
		align-items: center;
		gap: 8px;
		min-height: 20px;
	}

	.jobs-head h3 {
		margin: 0;
		font-size: 11px;
		font-weight: 600;
		letter-spacing: 0.14em;
		text-transform: uppercase;
		color: var(--cloth-dim);
	}

	.jobs-hint {
		margin: -4px 0 0;
	}

	.job,
	.saved button {
		display: grid;
		gap: 3px;
		text-align: left;
	}

	.job {
		padding: 9px 11px;
		border: 1px solid rgba(232, 178, 26, 0.45);
		border-radius: 8px;
		background: var(--cloth-job);
	}

	.job.idle {
		border-color: rgba(219, 228, 233, 0.2);
		background: var(--cloth-job);
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

	.job.idle .j-title,
	.saved .j-title {
		color: var(--cloth-ink);
	}

	.j-note {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 11px;
		color: var(--cloth-dim);
	}

	.job.idle .j-note {
		color: var(--cloth-mut);
	}

	.j-note.plain {
		padding: 2px 4px;
	}

	.dot {
		width: 6px;
		height: 6px;
		border-radius: 50%;
		background: var(--duck);
		animation: pulse 1.1s ease-out infinite alternate;
	}

	.dot.still {
		animation: none;
	}

	@keyframes pulse {
		from {
			opacity: 1;
		}
		to {
			opacity: 0.35;
		}
	}

	.saved {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 4px;
	}

	.saved button {
		width: 100%;
		padding: 8px 10px;
		border: 1px solid rgba(219, 228, 233, 0.16);
		border-radius: 8px;
		background: rgba(240, 248, 252, 0.04);
		transition:
			border-color 120ms ease-out,
			background-color 120ms ease-out;
	}

	.saved button:hover:not(:disabled) {
		background: rgba(240, 248, 252, 0.1);
		border-color: rgba(219, 228, 233, 0.48);
	}

	.saved button.current {
		background: rgba(240, 248, 252, 0.08);
		border-color: rgba(219, 228, 233, 0.45);
	}

	.saved .j-note {
		color: var(--cloth-mut);
	}

	/* context */
	.pct {
		margin-left: auto;
		font-family: var(--font-mono);
		font-size: 12px;
		letter-spacing: 0;
		color: var(--cloth-ink);
	}

	.pct.near {
		color: var(--duck);
	}

	.gauge-wrap {
		display: grid;
		gap: 5px;
		text-align: left;
	}

	.gauge {
		position: relative;
		height: 12px;
		border: 1px solid rgba(219, 228, 233, 0.4);
		border-radius: 999px;
		background: rgba(8, 16, 24, 0.3);
		overflow: hidden;
	}

	.g-fill {
		position: absolute;
		inset: 1px auto 1px 1px;
		width: calc((100% - 2px) * var(--fill));
		border-radius: 999px;
		background: var(--sky);
		transition: width 300ms ease-out;
	}

	.g-fill.near {
		background: var(--duck);
	}

	.g-thresh {
		position: absolute;
		top: -1px;
		bottom: -1px;
		left: calc(100% * var(--t));
		width: 0;
		border-left: 2px dashed var(--cloth-fail);
		opacity: 0.9;
	}

	.g-note {
		font-size: 11px;
		color: var(--cloth-mut);
	}

	.segments {
		display: grid;
		gap: 6px;
		padding-top: 2px;
	}

	.segment {
		display: grid;
		grid-template-columns: 1fr auto;
		gap: 0 8px;
		font-size: 11.5px;
	}

	.s-name {
		color: var(--cloth-ink);
	}

	.s-tokens {
		font-family: var(--font-mono);
		color: var(--cloth-dim);
	}

	.s-detail {
		grid-column: 1 / -1;
		color: var(--cloth-mut);
		font-size: 10.5px;
	}

	.s-note {
		margin: 0;
		font-size: 11px;
		color: var(--cloth-mut);
	}

	/* usage */
	.log {
		margin: 0;
		display: grid;
		grid-template-columns: 1fr 1fr;
		gap: 6px 12px;
	}

	.log div {
		display: grid;
		gap: 1px;
	}

	.log dt {
		font-size: 10.5px;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--cloth-mut);
	}

	.log dd {
		margin: 0;
		font-family: var(--font-mono);
		font-size: 12.5px;
		color: var(--cloth-ink);
	}

	.spark {
		width: 100%;
		height: 26px;
		display: block;
	}

	.sp-cached {
		fill: rgba(219, 228, 233, 0.4);
	}

	.sp-fresh {
		fill: var(--duck);
	}

	/* files */
	.files {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 4px;
	}

	.files details {
		border: 1px solid rgba(219, 228, 233, 0.25);
		border-radius: 8px;
		background: rgba(240, 248, 252, 0.04);
		overflow: hidden;
	}

	.files summary {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 7px 10px;
		cursor: pointer;
		list-style: none;
	}

	.files summary::-webkit-details-marker {
		display: none;
	}

	.f-path {
		flex: 1;
		min-width: 0;
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--cloth-ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.f-counts {
		display: inline-flex;
		gap: 6px;
		font-family: var(--font-mono);
		font-size: 11px;
	}

	.f-add {
		color: var(--cloth-ok);
	}

	.f-del {
		color: var(--cloth-fail);
	}

	.f-diff {
		margin: 0;
		max-height: 220px;
		overflow: auto;
		padding: 8px 10px;
		font-size: 11px;
		line-height: 1.5;
		background: var(--term);
		color: var(--term-text);
	}

	/* skills */
	.kit summary {
		display: flex;
		align-items: center;
		gap: 8px;
		min-height: 22px;
		cursor: pointer;
		list-style: none;
		user-select: none;
	}

	.kit summary::-webkit-details-marker {
		display: none;
	}

	.kit h2 {
		flex: 1;
		display: flex;
		gap: 8px;
		align-items: center;
		transition: color 120ms ease-out;
	}

	.kit summary::after {
		content: '▸';
		color: var(--cloth-dim);
		font-size: 11px;
		line-height: 1;
		transition: color 120ms ease-out;
	}

	.kit[open] summary::after {
		content: '▾';
	}

	.kit summary:hover h2,
	.kit summary:hover::after {
		color: var(--cloth-ink);
	}

	.skills {
		list-style: none;
		margin: 8px 0 0;
		padding: 0;
		display: grid;
		gap: 8px;
	}

	.skills li {
		display: grid;
		gap: 1px;
	}

	.k-name {
		font-family: var(--font-mono);
		font-size: 12px;
		color: var(--cloth-ink);
	}

	.k-desc {
		font-size: 11px;
		color: var(--cloth-mut);
	}

	/* plan */
	.plan {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 7px;
		counter-reset: step;
	}

	.plan li {
		display: flex;
		align-items: baseline;
		gap: 9px;
		font-size: 12.5px;
		line-height: 1.35;
		color: var(--cloth-mut);
	}

	.plan .mark {
		width: 16px;
		height: 16px;
		transform: translateY(2px);
		color: var(--cloth-dim);
	}

	.plan li.doing {
		color: var(--cloth-ink);
	}

	.plan li.doing .mark {
		color: var(--duck);
		border-color: var(--duck);
	}

	.plan li.done {
		color: var(--cloth-dim);
	}

	.plan li.done .p-text {
		text-decoration: line-through;
		text-decoration-color: rgba(219, 228, 233, 0.4);
	}

	.plan li.done .mark {
		background: var(--cloth-ok);
		border-color: var(--cloth-ok);
		color: #10140a;
	}

	.tag {
		margin-left: auto;
		background: var(--duck);
		color: #2b240f;
		font-size: 9.5px;
		font-weight: 700;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		border-radius: 999px;
		padding: 1.5px 7px;
		transform: translateY(-1px);
	}

	/* agents */
	.agents {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 8px;
	}

	.agents li {
		display: grid;
		gap: 3px;
		border: 1px solid rgba(219, 228, 233, 0.25);
		border-radius: 8px;
		background: rgba(240, 248, 252, 0.04);
		padding: 8px 10px;
	}

	.a-head {
		display: flex;
		align-items: center;
		gap: 7px;
	}

	.a-name {
		font-family: var(--font-serif);
		font-weight: 600;
		font-size: 13px;
		color: var(--cloth-ink);
	}

	.a-desc {
		margin: 0;
		font-size: 11px;
		color: var(--cloth-mut);
	}

	.a-text {
		margin: 2px 0 0;
		color: var(--cloth-dim);
		font-size: 11.5px;
		display: -webkit-box;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.a-sum {
		margin: 2px 0 0;
		font-family: var(--font-mono);
		font-size: 10.5px;
		color: var(--cloth-mut);
	}
</style>
