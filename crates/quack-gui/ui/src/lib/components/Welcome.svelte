<script lang="ts">
	import type { Transport, ResumeRequest } from '#lib/transport';

	let {
		transport,
		opening,
		openError,
		onOpen
	}: {
		transport: Transport;
		opening: boolean;
		openError: string | null;
		onOpen: (cwd: string, resume: ResumeRequest) => void;
	} = $props();

	let recents = $state<{ path: string; opened: number }[]>([]);
	let manualPath = $state('');

	$effect(() => {
		void transport.recentFolders(8).then((folders) => (recents = folders));
	});

	async function pick() {
		const picked = await transport.pickFolder();
		if (picked) onOpen(picked, { resume: 'latest' });
	}

	function short(path: string): string {
		const home = path.match(/^\/home\/[^/]+(\/.*)?$/);
		return home?.[1] !== undefined ? `~${home[1]}` : path;
	}

	function name(path: string): string {
		const parts = path.split('/');
		return parts[parts.length - 1] || path;
	}
</script>

<div class="welcome">
	<header>
		<pre class="duck" aria-hidden="true">   __
  ( o&lt;   quack
  /|_|\
  _/ \_</pre>
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
		min-height: 100vh;
		display: grid;
		place-content: center;
		gap: var(--sp-xl);
		background:
			radial-gradient(1200px 420px at 50% -120px, var(--bg-soft), transparent),
			var(--bg);
		padding: var(--sp-lg);
	}

	header {
		text-align: center;
		display: grid;
		justify-items: center;
		gap: var(--sp-md);
	}

	.duck {
		margin: 0;
		font-family: var(--font-mono);
		font-size: 15px;
		line-height: 1.25;
		color: var(--beak-ink);
	}

	h1 {
		font-size: 26px;
		line-height: 1.15;
		letter-spacing: -0.02em;
		font-weight: 800;
		margin: 0;
	}

	.lede {
		color: var(--ink-2);
		max-width: 46ch;
		margin: 0;
		line-height: 1.55;
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
		font-size: 13px;
		font-weight: 700;
		color: var(--ink-2);
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
