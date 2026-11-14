<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session, onClose }: { session: Session; onClose: () => void } = $props();

	let query = $state('');
	let dialogEl: HTMLElement | undefined = $state();
	let filterEl: HTMLInputElement | undefined = $state();

	const disabled = $derived(
		session.closed ||
			session.starting ||
			!!session.trust ||
			session.switching ||
			session.setupWorking ||
			session.streaming ||
			session.turnPending
	);

	const providers = $derived(session.providers.filter((p) => p.models.length > 0));
	const totalModels = $derived(providers.reduce((sum, p) => sum + p.models.length, 0));

	const filteredProviders = $derived.by(() => {
		const needle = query.trim().toLowerCase();
		if (!needle) return providers;
		return providers
			.map((p) => ({
				...p,
				models: p.models.filter((m) =>
					`${p.id}/${m.id}`.toLowerCase().includes(needle)
				)
			}))
			.filter((p) => p.models.length > 0);
	});

	function fmtWindow(tokens: number | undefined): string | null {
		if (!tokens) return null;
		return tokens >= 1000 ? `${Math.round(tokens / 1000)}k ctx` : `${tokens} ctx`;
	}

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			event.preventDefault();
			onClose();
		}
	}

	$effect(() => {
		if (totalModels > 5 && filterEl) filterEl.focus();
		else dialogEl?.focus();
	});
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
		aria-labelledby="model-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div>
				<h2 id="model-modal-title">Choose a model</h2>
				<p class="sub">
					{#if session.currentModel}
						Active: <span class="mono">{session.currentModel}</span>
					{:else}
						No model selected yet
					{/if}
				</p>
			</div>
			<button class="close-btn" aria-label="Close model picker" onclick={onClose}>×</button>
		</div>

		{#if totalModels > 5}
			<div class="search-row">
				<input
					type="search"
					class="search"
					placeholder="Filter models or providers…"
					aria-label="Filter models or providers"
					bind:value={query}
					bind:this={filterEl}
				/>
			</div>
		{/if}

		<div class="modal-body">
			{#each filteredProviders as provider (provider.id)}
				<section class="provider" class:unavailable={!provider.available}>
					<div class="provider-head">
						<h3>{provider.id}</h3>
						{#if !provider.available}
							<span class="provider-note">{provider.note}</span>
						{/if}
					</div>
					<div class="list">
						{#each provider.models as model (model.id)}
							{@const fullId = `${provider.id}/${model.id}`}
							{@const active = session.currentModel === fullId}
							{@const meta = [
								fmtWindow(model.context_window),
								model.price,
								model.reasoning_efforts?.length
									? `thinking: ${model.reasoning_efforts.join(' · ')}`
									: null
							]
								.filter(Boolean)
								.join(' · ')}
							<button
								class="model-row"
								class:active
								disabled={!provider.available || disabled}
								onclick={() => {
									void session.setModel(fullId);
									onClose();
								}}
							>
								<span class="row-top">
									<span class="model-id">{fullId}</span>
									{#if active}<span class="badge">Current</span>{/if}
								</span>
								<span class="note">{meta || 'Select model'}</span>
							</button>
						{/each}
					</div>
				</section>
			{:else}
				<p class="empty">
					{query.trim() ? `No models match “${query.trim()}”.` : 'No providers ready.'}
				</p>
			{/each}
		</div>

		<div class="modal-foot">
			<button
				class="btn"
				{disabled}
				onclick={() => {
					onClose();
					void session.openSetup();
				}}
			>
				Connect a provider…
			</button>
			<div class="grow"></div>
			<button class="btn" onclick={onClose}>
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
		width: min(540px, 100%);
		max-height: min(600px, calc(100vh - 32px));
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
		padding: var(--sp-md) var(--sp-md) 12px;
		border-bottom: 1px solid var(--rule-soft);
		background: var(--bg-panel);
	}

	.modal-head h2 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 19px;
		font-weight: 600;
		color: var(--ink);
	}

	.sub {
		margin: 3px 0 0;
		font-size: 12px;
		color: var(--ink-3);
	}

	.mono {
		font-family: var(--font-mono);
		color: var(--ink-2);
	}

	.close-btn {
		width: 28px;
		height: 28px;
		border-radius: var(--r-md);
		color: var(--ink-3);
		font-size: 18px;
		line-height: 1;
		display: grid;
		place-items: center;
		flex: none;
	}

	.close-btn:hover {
		background: var(--bg-soft);
		color: var(--ink);
	}

	.search-row {
		padding: 10px var(--sp-md);
		border-bottom: 1px solid var(--rule-soft);
		background: var(--bg-panel);
	}

	.search {
		width: 100%;
		padding: 7px 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		background: var(--bg-raised);
		color: var(--ink);
		font-size: 13px;
	}

	.search:focus {
		outline: none;
		border-color: var(--sky-ink);
	}

	.modal-body {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		overscroll-behavior: contain;
		padding: 12px var(--sp-md);
		display: grid;
		gap: var(--sp-md);
		align-content: start;
	}

	.provider {
		display: grid;
		gap: 6px;
	}

	.provider + .provider {
		padding-top: 12px;
		border-top: 1px solid var(--rule-soft);
	}

	.provider-head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px;
	}

	.provider-head h3 {
		margin: 0;
		font-size: 11px;
		font-weight: 700;
		letter-spacing: 0.12em;
		text-transform: uppercase;
		color: var(--ink-2);
	}

	.provider.unavailable .provider-head h3 {
		color: var(--ink-3);
	}

	.provider-note {
		font-size: 12px;
		color: var(--ink-3);
	}

	.list {
		display: grid;
		gap: 4px;
	}

	.model-row {
		width: 100%;
		display: grid;
		gap: 2px;
		text-align: left;
		padding: 8px 10px;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		transition:
			background-color 120ms ease-out,
			border-color 120ms ease-out;
	}

	.model-row:hover:not(:disabled) {
		background: var(--bg-soft);
		border-color: var(--rule);
	}

	.model-row.active {
		background: var(--sky-soft);
		border-color: var(--sky-ink);
	}

	.row-top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.model-id {
		font-family: var(--font-mono);
		font-size: 13px;
		font-weight: 500;
		color: var(--ink);
		overflow-wrap: anywhere;
	}

	.badge {
		font-size: 10.5px;
		font-weight: 600;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--sky-ink);
		background: var(--bg-raised);
		border: 1px solid var(--sky);
		border-radius: 999px;
		padding: 1px 8px;
		flex: none;
	}

	.note {
		font-size: 12px;
		color: var(--ink-3);
	}

	.empty {
		margin: 8px 0;
		color: var(--ink-3);
	}

	.modal-foot {
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
		padding: 12px var(--sp-md);
		border-top: 1px solid var(--rule-soft);
		background: var(--bg-panel);
	}

	.grow {
		flex: 1;
	}
</style>
