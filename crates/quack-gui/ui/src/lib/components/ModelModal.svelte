<script lang="ts">
	import { getModelSpeed, isLocalProvider } from '#lib/modelStats';
	import type { Session } from '#lib/session.svelte';

	let { session, onClose }: { session: Session; onClose: () => void } = $props();

	let query = $state('');
	let dialogEl: HTMLElement | undefined = $state();
	let filterEl: HTMLInputElement | undefined = $state();

	const egress = $derived(session.info?.egress);
	const plannerModel = $derived(session.info?.planner_model ?? null);
	const plannerLocked = $derived(session.info?.planner_locked ?? false);

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
				<p class="sub">
					{#if plannerModel}
						Plan mode architect: <span class="mono">{plannerModel}</span>
						{#if !plannerLocked}
							<button
								type="button"
								class="inline-link"
								disabled={disabled || session.settingSaving}
								onclick={() => void session.setPlannerModel(null)}
							>
								Use active model
							</button>
						{:else}
							<span class="locked-note">(locked)</span>
						{/if}
					{:else}
						Plan mode uses the active model
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
				{@const local = isLocalProvider(egress, provider.id)}
				<section class="provider" class:unavailable={!provider.available}>
					<div class="provider-head">
						<div class="provider-left">
							<h3>{provider.id}</h3>
							<span class="p-tag" class:local>{local ? 'Local / Free' : 'Hosted'}</span>
						</div>
						{#if !provider.available}
							<span class="provider-note">{provider.note}</span>
						{/if}
					</div>
					<div class="list">
						{#each provider.models as model (model.id)}
							{@const fullId = `${provider.id}/${model.id}`}
							{@const active = session.currentModel === fullId}
							{@const isPlanner = plannerModel === fullId}
							{@const speed = getModelSpeed(fullId)}
							{@const speedText = speed?.tokensPerSec
								? `~${speed.tokensPerSec.toFixed(0)} tok/s`
								: speed?.ttftMs
									? `~${speed.ttftMs} ms first token`
									: null}
							{@const meta = [
								local ? 'local' : model.price,
								fmtWindow(model.context_window),
								speedText,
								model.reasoning_efforts?.length
									? `thinking: ${model.reasoning_efforts.join(' · ')}`
									: null
							]
								.filter(Boolean)
								.join(' · ')}
							<div class="model-item" class:active>
								<button
									type="button"
									class="model-row"
									disabled={!provider.available || disabled}
									onclick={() => {
										void session.setModel(fullId);
										onClose();
									}}
								>
									<span class="row-top">
										<span class="model-id">{fullId}</span>
										<span class="row-badges">
											{#if active}<span class="badge">Current</span>{/if}
											{#if isPlanner}<span class="badge planner">Planner</span>{/if}
										</span>
									</span>
									<span class="note">{meta || 'Select model'}</span>
								</button>
								<button
									type="button"
									class="planner-btn"
									class:selected={isPlanner}
									disabled={!provider.available || disabled || session.settingSaving || plannerLocked}
									title={plannerLocked
										? 'model.planner is locked by system policy'
										: isPlanner
											? 'Clear dedicated Plan-mode architect model'
											: 'Use this model in Plan mode (saved to user config)'}
									onclick={() => void session.setPlannerModel(isPlanner ? null : fullId)}
								>
									{isPlanner ? 'Planner ✓' : 'Planner'}
								</button>
							</div>
						{/each}
					</div>
				</section>
			{:else}
				<p class="empty">
					{query.trim() ? `No models match “${query.trim()}”.` : 'No providers ready.'}
				</p>
			{/each}

			{#if session.toolProbe || session.toolProbing}
				<section class="probe-card" aria-live="polite">
					<div class="probe-head">
						<div>
							<h3>Tool-calling capability probe</h3>
							{#if session.toolProbe}
								<p class="probe-sub">
									Configured: <code>{session.toolProbe.configured_mode}</code> · Recommended:
									<code>{session.toolProbe.recommended_mode}</code>
									{#if session.toolProbe.recommended_text_format}
										(<code>{session.toolProbe.recommended_text_format}</code>)
									{/if}
								</p>
							{:else}
								<p class="probe-sub">Running synthetic tool-calling &amp; JSON repair checks…</p>
							{/if}
						</div>
					</div>
					{#if session.toolProbe}
						<div class="probe-cases">
							{#each session.toolProbe.cases as c (c.name)}
								<div class="probe-case" class:fail={!c.passed}>
									<span class={`probe-pill ${c.passed ? 'ok' : 'fail'}`}>
										{c.passed ? 'pass' : 'fail'}
									</span>
									<div class="probe-case-body">
										<div class="probe-case-top">
											<span class="probe-name">{c.name}</span>
											<span class="probe-ms">{c.latency_ms} ms</span>
										</div>
										<p class="probe-detail">{c.detail}</p>
									</div>
								</div>
							{/each}
						</div>
						<p class="probe-summary">{session.toolProbe.summary}</p>
					{/if}
				</section>
			{/if}
		</div>

		<div class="modal-foot">
			<button
				type="button"
				class="btn"
				{disabled}
				onclick={() => {
					onClose();
					void session.openSetup();
				}}
			>
				Connect a provider…
			</button>
			<button
				type="button"
				class="btn"
				disabled={disabled || session.toolProbing || !session.currentModel}
				onclick={() => void session.probeToolCalling()}
				title="Test native tool calling vs text fallback on the active model"
			>
				{session.toolProbing ? 'Probing tools…' : 'Probe tool calling'}
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
		font-size: 1.1875rem;
		font-weight: 600;
		color: var(--ink);
	}

	.sub {
		margin: 3px 0 0;
		font-size: 0.75rem;
		color: var(--ink-3);
	}

	.mono {
		font-family: var(--font-mono);
		color: var(--ink-2);
	}

	.close-btn {
		width: 1.75rem;
		height: 1.75rem;
		border-radius: var(--r-md);
		color: var(--ink-3);
		font-size: 1.125rem;
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
		font-size: 0.8125rem;
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
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		flex-wrap: wrap;
	}

	.provider-left {
		display: inline-flex;
		align-items: center;
		gap: 8px;
	}

	.p-tag {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		color: var(--ink-3);
	}

	.p-tag.local {
		color: var(--ok);
		background: var(--ok-soft);
		border-color: color-mix(in srgb, var(--ok) 45%, var(--rule));
	}

	.inline-link {
		margin-left: 8px;
		color: var(--sky-ink);
		font-size: 0.71875rem;
		text-decoration: underline;
	}

	.locked-note {
		margin-left: 6px;
		color: var(--ink-3);
	}

	.provider-head h3 {
		margin: 0;
		font-size: 0.6875rem;
		font-weight: 700;
		letter-spacing: 0.12em;
		text-transform: uppercase;
		color: var(--ink-2);
	}

	.provider.unavailable .provider-head h3 {
		color: var(--ink-3);
	}

	.provider-note {
		font-size: 0.75rem;
		color: var(--ink-3);
	}

	.list {
		display: grid;
		gap: 4px;
	}

	.model-item {
		display: flex;
		align-items: stretch;
		gap: 4px;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		overflow: hidden;
		transition:
			background-color 120ms ease-out,
			border-color 120ms ease-out;
	}

	.model-item:hover {
		background: var(--bg-soft);
		border-color: var(--rule);
	}

	.model-item.active {
		background: var(--sky-soft);
		border-color: var(--sky-ink);
	}

	.model-row {
		flex: 1;
		min-width: 0;
		display: grid;
		gap: 2px;
		text-align: left;
		padding: 8px 10px;
	}

	.planner-btn {
		flex: none;
		padding: 0 10px;
		border-left: 1px solid var(--rule-soft);
		font-size: 0.6875rem;
		font-family: var(--font-mono);
		color: var(--ink-3);
	}

	.planner-btn:hover:not(:disabled) {
		color: var(--ink);
		background: var(--bg-raised);
	}

	.planner-btn.selected {
		color: var(--duck-ink);
		background: var(--duck-soft);
		font-weight: 600;
	}

	.row-badges {
		display: inline-flex;
		gap: 5px;
		flex: none;
	}

	.badge.planner {
		color: var(--duck-ink);
		border-color: var(--duck-line);
		background: var(--duck-soft);
	}

	.row-top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.model-id {
		font-family: var(--font-mono);
		font-size: 0.8125rem;
		font-weight: 500;
		color: var(--ink);
		overflow-wrap: anywhere;
	}

	.badge {
		font-size: 0.65625rem;
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
		font-size: 0.75rem;
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

	.probe-card {
		margin-top: 10px;
		padding: 12px 14px;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.probe-head h3 {
		margin: 0;
		font-size: 0.84375rem;
		color: var(--ink);
	}

	.probe-sub,
	.probe-summary {
		margin: 3px 0 0;
		font-size: 0.75rem;
		color: var(--ink-2);
	}

	.probe-cases {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.probe-case {
		display: flex;
		align-items: flex-start;
		gap: 8px;
		padding: 6px 8px;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-md);
		background: var(--bg-raised);
	}

	.probe-pill {
		font-family: var(--font-mono);
		font-size: 0.625rem;
		font-weight: 600;
		text-transform: uppercase;
		padding: 1px 6px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		flex: none;
	}

	.probe-pill.ok {
		background: var(--ok-soft);
		color: var(--ok);
	}

	.probe-pill.fail {
		background: var(--fail-soft);
		color: var(--fail);
	}

	.probe-case-body {
		min-width: 0;
		flex: 1;
	}

	.probe-case-top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.probe-name {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--ink);
	}

	.probe-ms {
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		color: var(--ink-3);
	}

	.probe-detail {
		margin: 2px 0 0;
		font-size: 0.71875rem;
		color: var(--ink-2);
		overflow-wrap: anywhere;
	}
</style>
