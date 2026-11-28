<script lang="ts">
	import type { Session } from '#lib/session.svelte';
	import ThemeToggle from '#lib/components/ThemeToggle.svelte';

	let {
		session,
		sidebarOpen = false,
		attentionCount = 0,
		onToggleSidebar = () => {},
		onOpenEgress = () => {},
		onOpenMemory = () => {},
		onOpenDiagnostics = () => {},
		onOpenPalette = () => {}
	}: {
		session: Session;
		sidebarOpen?: boolean;
		attentionCount?: number;
		onToggleSidebar?: () => void;
		onOpenEgress?: () => void;
		onOpenMemory?: () => void;
		onOpenDiagnostics?: () => void;
		onOpenPalette?: () => void;
	} = $props();

	const egress = $derived(session.info?.egress);
	const sandbox = $derived(session.info?.sandbox);
	const shieldMode = $derived(egress?.secret_guard ?? 'ask');
	const shieldEvents = $derived(
		(session.egressStats?.redacted_count ?? 0) + (session.egressStats?.blocked_count ?? 0)
	);
	const activeProviderId = $derived(session.currentModel?.split('/')[0] ?? null);
	const activeEndpoint = $derived(
		activeProviderId ? (egress?.providers.find((p) => p.id === activeProviderId) ?? null) : null
	);
	const remoteMcpCount = $derived(egress?.mcp_remote.filter((m) => m.enabled).length ?? 0);
	const sandboxWeak = $derived(session.sandboxOff || (sandbox ? !sandbox.os_supported : false));

	const egressPill = $derived.by(() => {
		if (!egress) return null;
		const sandboxNote = session.sandboxOff
			? 'Sandbox: off'
			: sandbox && !sandbox.os_supported
				? 'Sandbox: unsupported'
				: sandbox
					? `Sandbox: ${sandbox.mode}`
					: 'Sandbox: unknown';
		if (egress.offline) {
			return {
				label: 'Offline',
				tone: 'local' as const,
				title: `Strict offline mode · ${sandboxNote}. Click to inspect Network & Sandbox Boundary.`
			};
		}
		if (activeEndpoint?.local && remoteMcpCount === 0) {
			return {
				label: 'Local',
				tone: 'local' as const,
				title: `Local endpoint (${activeEndpoint.host}) with zero remote MCP · ${sandboxNote}. Click to inspect.`
			};
		}
		const target = activeEndpoint
			? `${activeEndpoint.host}${remoteMcpCount > 0 ? ` + ${remoteMcpCount} remote MCP` : ''}`
			: 'Configured endpoints only';
		return {
			label: 'Bounded',
			tone: 'bounded' as const,
			title: `No implicit egress (${target}) · ${sandboxNote}. Click to inspect Network & Sandbox Boundary.`
		};
	});

	const lspTitle = $derived(
		session.lspServers.length > 0
			? `LSP servers: ${session.lspServers.map((s) => `${s.id} (${s.state})`).join(', ')}. Click to refresh diagnostics.`
			: 'Workspace diagnostics reported by LSP. Click to refresh.'
	);

	const disabled = $derived(
		session.closed ||
			session.starting ||
			!!session.trust ||
			session.switching ||
			session.setupWorking ||
			session.streaming ||
			session.turnPending
	);

	const live = $derived(session.streaming || session.turnPending);
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
		{#if attentionCount > 0}
			<span class="menu-badge" title={`${attentionCount} job${attentionCount === 1 ? '' : 's'} needing attention`}>
				{attentionCount}
			</span>
		{/if}
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

	{#if session.diagnostics.errors > 0 || session.diagnostics.warnings > 0}
		<button
			type="button"
			class="diag-pill"
			class:has-errors={session.diagnostics.errors > 0}
			disabled={session.closed}
			title={`${lspTitle}. Click to inspect LSP diagnostics across files.`}
			onclick={onOpenDiagnostics}
		>
			{#if session.diagnostics.errors > 0}
				<span>{session.diagnostics.errors} err</span>
			{/if}
			{#if session.diagnostics.warnings > 0}
				<span>{session.diagnostics.warnings} warn</span>
			{/if}
		</button>
	{/if}

	{#if egressPill}
		<button
			type="button"
			class="egress-pill"
			class:local={egressPill.tone === 'local'}
			class:warn={sandboxWeak}
			title={`${egressPill.title} · Secret Shield: ${shieldMode}`}
			onclick={onOpenEgress}
		>
			<span class="egress-dot" aria-hidden="true"></span>
			<span>{egressPill.label}</span>
			<span class="shield-state">
				{shieldMode === 'off'
					? '· Shield off'
					: shieldEvents > 0
						? `· Shield (${shieldEvents})`
						: '· Shield'}
			</span>
			{#if sandboxWeak}
				<span class="sandbox-state">· No sandbox</span>
			{/if}
		</button>
	{/if}

	<button
		type="button"
		class="btn memory-btn"
		class:warn={session.memoryIndexFill >= 80}
		disabled={session.closed || session.starting}
		title={session.memoryIndexFill >= 80
			? `Memory (${session.memoryNotes.length} notes · index ${session.memoryIndexFill}% full)`
			: `View and edit shared project & global memory notes (${session.memoryNotes.length})`}
		onclick={onOpenMemory}
	>
		<span>Memory</span>
		{#if session.memoryNotes.length > 0}
			<span class="mem-count">{session.memoryNotes.length}</span>
		{/if}
	</button>
	<button
		type="button"
		class="btn palette-btn"
		disabled={session.closed || session.starting}
		title="Open Command Palette (⌘K / Ctrl+K)"
		onclick={onOpenPalette}
	>
		<span>Commands</span>
		<span class="kbd">⌘K</span>
	</button>
	<button class="btn providers-btn" {disabled} onclick={() => session.openSetup()}>Providers</button>
	<button
		type="button"
		class="btn settings-btn"
		class:active={session.settingsOpen}
		aria-pressed={session.settingsOpen}
		disabled={session.closed || session.starting}
		title="Configure paths, setup defaults, permissions, and TOML config"
		onclick={() => (session.settingsOpen ? session.closeSettings() : session.openSettings())}
	>
		Settings
	</button>
	<ThemeToggle />
</header>

<!-- The signature move: the website's water line, carrying data. It fills toward the
     session's budget; when pruning or compaction is near it turns duck yellow. -->
<div class={['waterline', { live }]} aria-hidden="true">
	<div class="water" class:nearing style:--fill={fill}></div>
</div>

<style>
	header {
		min-height: var(--header-h);
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
		min-height: 1.875rem;
		padding: 0 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		color: var(--ink-2);
		font-size: 0.78125rem;
		font-weight: 500;
		flex: none;
	}

	.menu-btn svg {
		width: 15px;
		height: 15px;
	}

	.menu-badge {
		font-family: var(--font-mono);
		font-size: 0.625rem;
		font-weight: 700;
		line-height: 1;
		padding: 2px 6px;
		border-radius: 999px;
		background: var(--duck);
		color: #2b240f;
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
		font-size: 0.90625rem;
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
		font-size: 0.75rem;
		color: var(--ink-3);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.branch {
		font-family: var(--font-mono);
		font-size: 0.6875rem;
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

	.egress-pill,
	.diag-pill {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		padding: 3px 9px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink-2);
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		font-weight: 500;
		flex: none;
		white-space: nowrap;
	}

	.egress-pill:hover,
	.diag-pill:hover:not(:disabled) {
		border-color: var(--sky);
		color: var(--ink);
	}

	.egress-dot {
		width: 7px;
		height: 7px;
		border-radius: 999px;
		background: var(--sky);
	}

	.egress-pill.local {
		color: var(--ok);
		border-color: color-mix(in srgb, var(--ok) 40%, var(--rule));
		background: var(--ok-soft);
	}

	.egress-pill.local .egress-dot {
		background: var(--ok);
	}

	.egress-pill.warn .sandbox-state {
		color: var(--beak-ink);
	}

	.diag-pill {
		color: var(--beak-ink);
		background: var(--duck-soft);
		border-color: var(--duck-line);
	}

	.diag-pill.has-errors {
		color: var(--fail);
		background: var(--fail-soft);
		border-color: color-mix(in srgb, var(--fail) 45%, var(--rule));
	}

	.sandbox-state,
	.shield-state {
		color: var(--ink-3);
	}

	@media (max-width: 680px) {
		.sandbox-state,
		.shield-state {
			display: none;
		}
	}

	.providers-btn,
	.settings-btn,
	.memory-btn,
	.palette-btn {
		padding: 4px 11px;
		font-size: 0.78125rem;
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}

	.settings-btn.active {
		border-color: var(--beak-line);
		background: var(--beak-soft);
		color: var(--beak-ink);
	}

	.memory-btn.warn {
		border-color: var(--duck-line);
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.mem-count {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		line-height: 1;
		padding: 2px 5px;
		border-radius: 999px;
		background: var(--bg-soft);
		color: var(--ink-2);
	}

	.waterline {
		position: relative;
		height: 3px;
		background: var(--rule-soft);
		overflow: hidden;
	}

	.waterline.live::after {
		content: '';
		position: absolute;
		inset: 0;
		width: 28%;
		background: linear-gradient(
			90deg,
			transparent 0%,
			color-mix(in srgb, var(--duck) 78%, transparent) 50%,
			transparent 100%
		);
		animation: water-current 1.8s ease-in-out infinite;
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

	@keyframes water-current {
		from {
			transform: translateX(-110%);
		}
		to {
			transform: translateX(420%);
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.waterline.live::after {
			animation: none;
			opacity: 0;
		}
	}

	:global(:root[data-reduced-motion='true']) .waterline.live::after {
		animation: none;
		opacity: 0;
	}
</style>
