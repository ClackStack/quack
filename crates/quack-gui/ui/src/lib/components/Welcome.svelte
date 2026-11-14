<script lang="ts">
	import type { RecentFolder, Transport, ResumeRequest } from '#lib/transport';
	import { collapseHomeDisplay, dedupeRecentFolders } from '#lib/workspaces.svelte';
	import logoUrl from '#lib/assets/logo.png';
	import ThemeToggle from '#lib/components/ThemeToggle.svelte';

	let {
		transport,
		opening,
		openError,
		homePrefix = '',
		onOpen
	}: {
		transport: Transport;
		opening: boolean;
		openError: string | null;
		homePrefix?: string;
		onOpen: (cwd: string, resume: ResumeRequest) => void;
	} = $props();

	let rawRecents = $state<RecentFolder[]>([]);
	let recents = $derived(dedupeRecentFolders(rawRecents, homePrefix).slice(0, 8));
	let manualPath = $state('');

	$effect(() => {
		void transport.recentFolders(16).then((folders) => (rawRecents = folders ?? []));
	});

	async function pick() {
		const picked = await transport.pickFolder();
		if (picked) onOpen(picked, { resume: 'latest' });
	}

	function short(path: string): string {
		return collapseHomeDisplay(path, homePrefix);
	}

	function name(path: string): string {
		const parts = path.split(/[\\/]/).filter(Boolean);
		return parts[parts.length - 1] || path;
	}
</script>

<div class="welcome">
	<div class="welcome-top">
		<ThemeToggle showLabels />
	</div>
	<header>
		<div class="logo-pond">
			<span class="pond-ripple ripple-1" aria-hidden="true"></span>
			<span class="pond-ripple ripple-2" aria-hidden="true"></span>
			<img src={logoUrl} alt="quack" class="welcome-logo" />
		</div>
		<h1>A terminal coding agent,<br />in a window.</h1>
		<p class="lede">
			Open a folder to start a session. The engine runs locally; quack only talks to
			providers you configured.
		</p>
		<div class="actions">
			<button class="btn btn-primary" onclick={pick} disabled={opening}>
				{opening ? 'Opening…' : 'Open Folder…'}
			</button>
			{#if transport.kind === 'dev'}
				<form
					class="manual"
					onsubmit={(e) => {
						e.preventDefault();
						if (manualPath.trim()) onOpen(manualPath.trim(), { resume: 'latest' });
					}}
				>
					<input
						type="text"
						placeholder="…or type a path (dev transport)"
						bind:value={manualPath}
						aria-label="Workspace path"
					/>
					<button class="btn" type="submit">Open</button>
				</form>
			{/if}
		</div>
		{#if openError}
			<p class="error" role="alert">{openError}</p>
		{/if}
	</header>

	{#if recents.length > 0}
		<section>
			<h2>Recent folders</h2>
			<ul>
				{#each recents as folder (folder.path)}
					<li>
						<button class="recent" onclick={() => onOpen(folder.path, { resume: 'latest' })}>
							<span class="name">{name(folder.path)}</span>
							<span class="path">{short(folder.path)}</span>
						</button>
					</li>
				{/each}
			</ul>
			<p class="hint">Reopening a folder resumes its most recent session.</p>
		</section>
	{/if}
</div>

<style>
	.welcome {
		position: relative;
		min-height: 100vh;
		display: grid;
		place-content: center;
		gap: var(--sp-xl);
		background:
			radial-gradient(1200px 420px at 50% -120px, var(--bg-soft), transparent),
			var(--bg);
		padding: var(--sp-lg);
	}

	.welcome-top {
		position: absolute;
		top: var(--sp-md);
		right: var(--sp-md);
		z-index: 10;
	}

	header {
		text-align: center;
		display: grid;
		justify-items: center;
		gap: var(--sp-md);
	}

	.logo-pond {
		position: relative;
		display: inline-flex;
		align-items: center;
		justify-content: center;
		padding: 16px 24px;
		border-radius: 24px;
		background: radial-gradient(
			ellipse 85% 75% at 40% 35%,
			#82e4fa 0%,
			#4ec4e9 45%,
			#1d9ac5 100%
		);
		border: 1px solid rgba(255, 255, 255, 0.6);
		box-shadow:
			0 12px 32px -4px rgba(8, 106, 134, 0.28),
			0 4px 10px -2px rgba(8, 106, 134, 0.16),
			inset 0 1px 1px rgba(255, 255, 255, 0.8),
			inset 0 -2px 6px rgba(8, 106, 134, 0.2);
		overflow: hidden;
		user-select: none;
	}

	:root[data-theme='dark'] .logo-pond {
		background: radial-gradient(
			ellipse 85% 75% at 40% 35%,
			#2a83a5 0%,
			#1b617c 50%,
			#103f53 100%
		);
		border: 1px solid rgba(122, 212, 236, 0.28);
		box-shadow:
			0 12px 32px -4px rgba(0, 0, 0, 0.5),
			0 4px 10px -2px rgba(0, 0, 0, 0.35),
			inset 0 1px 1px rgba(255, 255, 255, 0.18),
			inset 0 -2px 6px rgba(0, 0, 0, 0.4);
	}

	.pond-ripple {
		position: absolute;
		border-radius: 50%;
		border: 1.5px solid rgba(255, 255, 255, 0.3);
		pointer-events: none;
		transform: translate(-50%, -50%);
	}

	.ripple-1 {
		left: 28%;
		top: 62%;
		width: 120px;
		height: 48px;
	}

	.ripple-2 {
		left: 28%;
		top: 62%;
		width: 190px;
		height: 76px;
		border-color: rgba(255, 255, 255, 0.18);
	}

	:root[data-theme='dark'] .ripple-1 {
		border-color: rgba(255, 255, 255, 0.18);
	}

	:root[data-theme='dark'] .ripple-2 {
		border-color: rgba(255, 255, 255, 0.1);
	}

	.welcome-logo {
		width: min(340px, 75vw);
		height: auto;
		display: block;
		margin: 0 auto;
		position: relative;
		z-index: 1;
		user-select: none;
		filter: drop-shadow(0 2px 6px rgba(8, 106, 134, 0.35));
	}

	:root[data-theme='dark'] .welcome-logo {
		filter: drop-shadow(0 2px 6px rgba(0, 0, 0, 0.5));
	}

	h1 {
		font-family: var(--font-serif);
		font-size: 27px;
		font-weight: 600;
		line-height: 1.15;
		letter-spacing: -0.01em;
		margin: 0;
	}

	.lede {
		color: var(--ink-2);
		max-width: 46ch;
		margin: 0;
		line-height: 1.55;
		font-family: var(--font-serif);
		font-size: 15.5px;
	}

	.actions {
		display: grid;
		gap: var(--sp-sm);
		justify-items: center;
		margin-top: var(--sp-xs);
	}

	.manual {
		display: flex;
		gap: var(--sp-sm);
	}

	.manual input {
		width: 280px;
		padding: 5px 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		font-family: var(--font-mono);
		font-size: var(--text-mono);
	}

	.error {
		color: var(--fail);
		margin: 0;
	}

	section {
		width: min(480px, 90vw);
		display: grid;
		gap: var(--sp-sm);
	}

	h2 {
		font-size: 12px;
		font-weight: 700;
		letter-spacing: 0.12em;
		text-transform: uppercase;
		color: var(--ink-3);
		margin: 0;
	}

	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		overflow: hidden;
	}

	.recent {
		width: 100%;
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: var(--sp-md);
		padding: 9px 14px;
		text-align: left;
		border-bottom: 1px solid var(--rule-soft);
	}

	li:last-child .recent {
		border-bottom: none;
	}

	.recent:hover {
		background: var(--bg-soft);
	}

	.name {
		font-weight: 600;
	}

	.path {
		font-family: var(--font-mono);
		font-size: 12px;
		color: var(--ink-3);
	}

	.hint {
		color: var(--ink-3);
		font-size: 12.5px;
		margin: 0;
	}
</style>
