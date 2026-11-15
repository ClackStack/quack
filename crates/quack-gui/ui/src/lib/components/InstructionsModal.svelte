<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session, onClose }: { session: Session; onClose: () => void } = $props();

	let dialogEl: HTMLElement | undefined = $state();

	const files = $derived(session.instructions ?? []);
	const totalTokens = $derived(files.reduce((sum, f) => sum + f.tokens, 0));

	$effect(() => {
		dialogEl?.focus();
	});

	$effect(() => {
		if (session.instructions === null && !session.instructionsLoading && !session.closed) {
			void session.listInstructions();
		}
	});

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			event.preventDefault();
			onClose();
		}
	}
</script>

<svelte:window onkeydown={onKeydown} />

<div
	class="backdrop"
	role="presentation"
	onclick={(e) => {
		if (e.target === e.currentTarget) onClose();
	}}
>
	<div
		class="modal"
		role="dialog"
		aria-modal="true"
		aria-labelledby="instructions-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div>
				<h2 id="instructions-modal-title">Workspace instructions</h2>
				<p class="sub">
					{files.length} file{files.length === 1 ? '' : 's'} · {totalTokens.toLocaleString()} est. tokens loaded for this session
				</p>
			</div>
			<button
				type="button"
				class="close-btn"
				aria-label="Close workspace instructions"
				onclick={onClose}
			>
				×
			</button>
		</div>

		<div class="modal-body">
			{#if files.length > 0}
				{#each files as file (file.source)}
					<details class="inst-card" open={files.length === 1}>
						<summary class="inst-head">
							<span class="inst-path">{file.source}</span>
							<div class="inst-meta">
								<span class="badge" class:in-prompt={file.in_prompt}>
									{file.in_prompt ? 'session context' : 'attached on folder read'}
								</span>
								<span class="tok-count">{file.tokens.toLocaleString()} tok</span>
							</div>
						</summary>
						<pre class="inst-body">{file.text}</pre>
					</details>
				{/each}
			{:else if session.instructionsLoading}
				<p class="empty">Inspecting instruction files…</p>
			{:else}
				<div class="empty-card">
					<p class="empty">
						No <code>AGENTS.md</code> or project instruction files loaded in this workspace.
					</p>
					<button
						type="button"
						class="btn btn-primary"
						disabled={session.closed || session.streaming}
						onclick={() => void session.initInstructions()}
					>
						Create starter AGENTS.md
					</button>
				</div>
			{/if}
		</div>

		<div class="modal-foot">
			<button
				type="button"
				class="btn"
				disabled={session.closed || session.instructionsLoading}
				onclick={() => void session.listInstructions()}
			>
				{session.instructionsLoading ? 'Refreshing…' : 'Refresh'}
			</button>
			<div class="grow"></div>
			<button type="button" class="btn" onclick={onClose}>
				Close <span class="kbd">esc</span>
			</button>
		</div>
	</div>
</div>

<style>
	.backdrop {
		position: fixed;
		inset: 0;
		z-index: 60;
		background: var(--scrim);
		display: grid;
		place-items: center;
		padding: var(--sp-md);
	}

	.modal {
		width: min(640px, 100%);
		max-height: min(680px, calc(100vh - 32px));
		display: flex;
		flex-direction: column;
		background: var(--bg-raised);
		border: 1px solid var(--plate-line);
		border-radius: var(--r-xl);
		box-shadow: var(--shadow-pop);
		overflow: hidden;
		outline: none;
	}

	.modal-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--sp-md);
		padding: 16px 20px;
		background: var(--bg-panel);
		border-bottom: 1px solid var(--rule);
	}

	.modal-head h2 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 1.125rem;
		color: var(--ink);
	}

	.sub {
		margin: 4px 0 0;
		font-size: 0.78125rem;
		color: var(--ink-2);
	}

	.close-btn {
		font-size: 1.25rem;
		line-height: 1;
		padding: 2px 8px;
		border-radius: var(--r-md);
		color: var(--ink-2);
	}

	.close-btn:hover {
		background: var(--bg-soft);
		color: var(--ink);
	}

	.modal-body {
		padding: var(--sp-md) 20px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 10px;
	}

	.inst-card {
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		overflow: hidden;
	}

	.inst-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
		padding: 10px 12px;
		cursor: pointer;
		list-style: none;
	}

	.inst-path {
		font-family: var(--font-mono);
		font-size: 0.8125rem;
		font-weight: 600;
		color: var(--ink);
		overflow-wrap: anywhere;
	}

	.inst-meta {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		flex: none;
	}

	.badge {
		font-size: 0.65625rem;
		font-weight: 600;
		padding: 2px 7px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink-2);
	}

	.badge.in-prompt {
		background: var(--ok-soft);
		color: var(--ok);
	}

	.tok-count {
		font-family: var(--font-mono);
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.inst-body {
		margin: 0;
		padding: 12px;
		border-top: 1px solid var(--rule-soft);
		background: var(--bg-raised);
		font-family: var(--font-mono);
		font-size: 0.75rem;
		line-height: 1.5;
		color: var(--ink-2);
		max-height: 280px;
		overflow: auto;
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.empty-card {
		display: flex;
		flex-direction: column;
		align-items: center;
		gap: 12px;
		padding: 20px;
	}

	.empty {
		margin: 0;
		text-align: center;
		font-size: 0.84375rem;
		color: var(--ink-3);
	}

	.modal-foot {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 12px 20px;
		background: var(--bg-panel);
		border-top: 1px solid var(--rule-soft);
	}

	.grow {
		flex: 1;
	}
</style>
