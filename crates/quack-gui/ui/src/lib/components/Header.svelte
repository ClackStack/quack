<script lang="ts">
	import type { Session } from '#lib/session.svelte';
	import ThemeToggle from '#lib/components/ThemeToggle.svelte';

	let {
		session,
		sidebarOpen = false,
		onToggleSidebar = () => {}
	}: { session: Session; sidebarOpen?: boolean; onToggleSidebar?: () => void } = $props();

	const disabled = $derived(
		session.closed ||
			session.starting ||
			!!session.trust ||
			session.switching ||
			session.setupWorking ||
			session.streaming ||
			session.turnPending
	);

	const fill = $derived.by(() => {
		const c = session.context;
		if (!c) return 0;
		const base = c.budget ?? c.window;
		return base ? Math.min(1, c.used / base) : 0;
	});
	const nearing = $derived(
		session.context?.threshold !== undefined &&
			session.context.threshold !== null &&
			(session.context?.used ?? 0) >= session.context.threshold * 0.9
	);

	function projectName(cwd: string | undefined): string {
		if (!cwd) return 'Workspace';
		const parts = cwd.split('/').filter(Boolean);
		return parts[parts.length - 1] || cwd;
	}
</script>

<header>
	<button
		class="menu-btn"
		aria-expanded={sidebarOpen}
		aria-controls="workspace-sidebar"
		onclick={onToggleSidebar}
	>
		<svg viewBox="0 0 24 24" aria-hidden="true"
			><path
				d="M4.5 7h15M4.5 12h15M4.5 17h15"
				fill="none"
				stroke="currentColor"
				stroke-width="1.8"
				stroke-linecap="round"
			/></svg
		>
		<span class="menu-label">Projects</span>
	</button>

	<div class="identity">
		<span class="job-name">{session.title || 'New job'}</span>
		{#if session.info?.cwd}
			<span class="sep" aria-hidden="true">·</span>
			<span class="folder" title={session.info.cwd}>{projectName(session.info.cwd)}</span>
		{/if}
		{#if session.info?.branch}
			<span class="branch" title={`Git branch: ${session.info.branch}`}>{session.info.branch}</span>
		{/if}
	</div>

	<div class="grow"></div>

	<button class="btn providers-btn" {disabled} onclick={() => session.openSetup()}>Providers</button>
	<ThemeToggle />
</header>

<!-- The signature move: the website's water line, carrying data. It fills toward the
     session's budget; when pruning or compaction is near it turns duck yellow. -->
<div class="waterline" aria-hidden="true">
	<div class="water" class:nearing style:--fill={fill}></div>
</div>

<style>
	header {
		height: var(--header-h);
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
		padding: 0 var(--sp-md);
		border-bottom: 3px double var(--rule);
		background: var(--bg-panel);
	}

	.menu-btn {
		display: none;
		align-items: center;
		gap: 7px;
		height: 30px;
		padding: 0 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		color: var(--ink-2);
		font-size: 12.5px;
		font-weight: 500;
		flex: none;
	}

	.menu-btn svg {
		width: 15px;
		height: 15px;
	}

	@media (max-width: 900px) {
		.menu-btn {
			display: inline-flex;
		}
	}

	.identity {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
		overflow: hidden;
	}

	.job-name {
		font-family: var(--font-serif);
		font-size: 14.5px;
		font-weight: 600;
		color: var(--ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.sep {
		color: var(--ink-3);
		flex: none;
	}

	.folder {
		font-family: var(--font-mono);
		font-size: 12px;
		color: var(--ink-3);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.branch {
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--sky-ink);
		background: var(--sky-soft);
		border: 1px solid color-mix(in srgb, var(--sky) 35%, var(--rule));
		border-radius: 999px;
		padding: 1px 8px;
		flex: none;
	}

	@media (max-width: 600px) {
		.sep,
		.folder,
		.branch {
			display: none;
		}
	}

	.grow {
		flex: 1;
	}

	.providers-btn {
		padding: 4px 11px;
		font-size: 12.5px;
	}

	.waterline {
		height: 3px;
		background: var(--rule-soft);
	}

	.water {
		height: 100%;
		width: calc(var(--fill, 0) * 100%);
		background: var(--sky);
		transition:
			width 400ms ease-out,
			background-color 400ms ease-out;
	}

	.water.nearing {
		background: var(--duck);
	}
</style>
