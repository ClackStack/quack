<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session, onClose }: { session: Session; onClose: () => void } = $props();

	let dialogEl: HTMLElement | undefined = $state();

	const disabled = $derived(
		session.closed ||
			session.starting ||
			!!session.trust ||
			session.switching ||
			session.streaming ||
			session.turnPending
	);
	const profileState = $derived(session.profiles);
	const list = $derived(profileState?.profiles ?? []);

	$effect(() => {
		dialogEl?.focus();
	});

	$effect(() => {
		if (!session.profiles && !session.closed) {
			void session.listProfiles();
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
		aria-labelledby="profile-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div>
				<h2 id="profile-modal-title">Prompt profiles</h2>
				<p class="sub">
					Active: <code>{profileState?.active ?? 'auto'}</code>
					{#if profileState?.forced}
						· pinned
					{:else}
						· auto-matched to {session.currentModel ?? 'model'}
					{/if}
				</p>
			</div>
			<button type="button" class="close-btn" aria-label="Close prompt profiles" onclick={onClose}>
				×
			</button>
		</div>

		<div class="modal-body">
			{#if list.length > 0}
				{#each list as p (p.id)}
					{@const isActive = profileState?.active === p.id}
					<section class="profile-card" class:active={isActive}>
						<div class="card-head">
							<div>
								<div class="title-row">
									<h3>{p.id}</h3>
									{#if isActive}
										<span class="badge active">Active</span>
									{/if}
									{#if p.matches}
										<span class="badge match">Matches model</span>
									{/if}
									<span class="badge">{p.edit_format}</span>
								</div>
								<p class="desc">{p.description}</p>
							</div>
							<button
								type="button"
								class="btn"
								class:btn-primary={!isActive}
								disabled={disabled || isActive}
								onclick={() => void session.setProfile(p.id)}
							>
								{isActive ? 'Selected' : 'Use profile'}
							</button>
						</div>

						<div class="meta-line">
							<span>Source: <code>{p.source}</code></span>
							<span>Tools ({p.tools.length}): <code>{p.tools.join(', ')}</code></span>
						</div>

						<details class="sys-preview">
							<summary>Inspect rendered system prompt</summary>
							<pre>{p.system}</pre>
						</details>
					</section>
				{/each}
			{:else}
				<p class="empty">Loading prompt profiles…</p>
			{/if}
		</div>

		<div class="modal-foot">
			<button
				type="button"
				class="btn"
				disabled={session.closed}
				onclick={() => void session.listProfiles()}
			>
				Refresh
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
		gap: 12px;
	}

	.profile-card {
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		padding: 12px 14px;
		background: var(--bg-panel);
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.profile-card.active {
		border-color: var(--duck);
		background: var(--bg-raised);
	}

	.card-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 12px;
	}

	.title-row {
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}

	h3 {
		margin: 0;
		font-family: var(--font-mono);
		font-size: 0.90625rem;
		color: var(--ink);
	}

	.badge {
		font-size: 0.65625rem;
		font-weight: 600;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink-2);
	}

	.badge.active {
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.badge.match {
		background: var(--ok-soft);
		color: var(--ok);
	}

	.desc {
		margin: 4px 0 0;
		font-size: 0.8125rem;
		color: var(--ink-2);
	}

	.meta-line {
		display: flex;
		flex-direction: column;
		gap: 3px;
		font-size: 0.75rem;
		color: var(--ink-3);
	}

	.sys-preview summary {
		font-size: 0.75rem;
		color: var(--ink-2);
		cursor: pointer;
	}

	.sys-preview pre {
		margin: 8px 0 0;
		padding: 10px;
		border-radius: var(--r-md);
		background: var(--bg-soft);
		font-family: var(--font-mono);
		font-size: 0.71875rem;
		line-height: 1.45;
		max-height: 220px;
		overflow: auto;
		white-space: pre-wrap;
		overflow-wrap: anywhere;
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

	.empty {
		margin: 20px 0;
		text-align: center;
		font-size: 0.84375rem;
		color: var(--ink-3);
	}
</style>
