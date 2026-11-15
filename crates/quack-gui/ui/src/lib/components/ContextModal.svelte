<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session, onClose }: { session: Session; onClose: () => void } = $props();

	let dialogEl: HTMLElement | undefined = $state();

	const disabled = $derived(
		session.closed ||
			session.starting ||
			!!session.trust ||
			session.switching ||
			session.setupWorking ||
			session.streaming ||
			session.turnPending
	);

	const gauge = $derived.by(() => {
		const c = session.context;
		if (!c) return null;
		const used = c.used;
		const budget = c.budget ?? null;
		const windowSize = c.window ?? null;
		const threshold = c.threshold ?? null;
		const base = budget ?? windowSize;
		if (!base) return null;
		return {
			used,
			base,
			budget,
			windowSize,
			threshold,
			fill: Math.min(1, used / base),
			thresholdPos: threshold ? Math.min(1, threshold / base) : null,
			near: threshold !== null && used >= threshold * 0.9
		};
	});

	const fmtTokens = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`);

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			event.preventDefault();
			onClose();
		}
	}

	$effect(() => {
		dialogEl?.focus();
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
		aria-labelledby="context-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div>
				<h2 id="context-modal-title">Context window</h2>
				<p class="sub">
					<span class="mono">{session.contextReport?.model ?? session.currentModel ?? 'No model'}</span>
				</p>
			</div>
			<button class="close-btn" aria-label="Close context details" onclick={onClose}>×</button>
		</div>

		<div class="modal-body">
			{#if gauge}
				<div class="gauge-card">
					<div class="gauge-head">
						<span class="gauge-nums">
							{fmtTokens(gauge.used)} of {fmtTokens(gauge.base)} tokens
						</span>
						<span class="pct" class:near={gauge.near}>{Math.round(gauge.fill * 100)}%</span>
					</div>
					<div
						class="gauge"
						role="img"
						aria-label={`Context ${fmtTokens(gauge.used)} of ${fmtTokens(gauge.base)} tokens`}
					>
						<div class="g-fill" class:near={gauge.near} style:--fill={gauge.fill}></div>
						{#if gauge.thresholdPos !== null}
							<div
								class="g-thresh"
								style:--t={gauge.thresholdPos}
								title={`Pruning starts at ${fmtTokens(gauge.threshold ?? 0)} tokens`}
							></div>
						{/if}
					</div>
					{#if gauge.threshold !== null}
						<p class="thresh-note">
							Pruning starts around {fmtTokens(gauge.threshold)} tokens
							{#if gauge.windowSize && gauge.budget && gauge.windowSize !== gauge.budget}
								· window {fmtTokens(gauge.windowSize)}
							{/if}
						</p>
					{/if}
				</div>
			{/if}

			{#if session.contextReport}
				{#if session.contextReport.strategy}
					<div class="strategy-strip">
						<span>Strategy: <code>{session.contextReport.strategy}</code></span>
						{#if session.contextReport.tail_items?.some((t) => t.cached)}
							<span class="zone-pill prefix">KV prefix cached</span>
						{/if}
					</div>
				{/if}

				<div class="segments">
					{#each session.contextReport.segments as segment (`${segment.zone ?? 'all'}:${segment.name}`)}
						<div class="segment" class:cached-zone={segment.zone === 'cached_prefix'}>
							<div class="seg-top">
								<span class="seg-title-wrap">
									<span class="seg-name">{segment.name}</span>
									{#if segment.zone === 'cached_prefix'}
										<span class="zone-badge prefix">Immutable KV prefix</span>
									{:else if segment.zone === 'session_context'}
										<span class="zone-badge session">Session context</span>
									{:else if segment.zone === 'uncached_tail'}
										<span class="zone-badge tail">Prunable tail</span>
									{/if}
								</span>
								<span class="seg-tokens">{segment.tokens.toLocaleString()} tok</span>
							</div>
							<span class="seg-detail">{segment.detail}</span>
						</div>
					{/each}
				</div>

				{#if session.contextReport.tail_items && session.contextReport.tail_items.length > 0}
					<div class="tail-section">
						<div class="tail-head">
							<span class="tail-title">Turn items ({session.contextReport.tail_items.length})</span>
							<span class="tail-sub">Click Trim on older uncached tool/message outputs to free space without breaking prefix caching.</span>
						</div>
						<div class="tail-list">
							{#each session.contextReport.tail_items as item (item.index)}
								<div class="tail-item" class:pruned={item.pruned}>
									<div class="tail-meta">
										<span class="tail-role">{item.role}</span>
										<span class="tail-label">{item.label}</span>
									</div>
									<div class="tail-actions">
										{#if item.cached}
											<span class="item-badge cached">cached</span>
										{/if}
										{#if item.pruned}
											<span class="item-badge pruned">pruned</span>
										{/if}
										<span class="tail-tok">{item.tokens.toLocaleString()} tok</span>
										{#if item.prunable && !item.pruned}
											<button
												type="button"
												class="trim-btn"
												{disabled}
												onclick={() => void session.pruneContext([item.index])}
											>
												Trim
											</button>
										{/if}
									</div>
								</div>
							{/each}
						</div>
					</div>
				{/if}
				{#if session.contextReport.note}
					<p class="report-note">{session.contextReport.note}</p>
				{/if}
			{:else}
				<p class="empty">
					{session.context
						? `${session.context.used.toLocaleString()} tokens in use · reading breakdown…`
						: 'No context used yet — start a turn to inspect the breakdown.'}
				</p>
			{/if}
		</div>

		<div class="modal-foot">
			<button
				type="button"
				class="btn"
				disabled={disabled || (!session.context && !session.contextReport)}
				onclick={() => void session.pruneContext()}
				title="Prune stale tool outputs while keeping the cached KV prefix byte-identical"
			>
				Prune stale
			</button>
			<button
				type="button"
				class="btn"
				disabled={disabled || (!session.context && !session.contextReport)}
				onclick={() => {
					void session.compact();
					onClose();
				}}
				title="Summarize older history into a shorter carry"
			>
				Compact now
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
		width: min(480px, 100%);
		max-height: min(560px, calc(100vh - 32px));
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

	.modal-body {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		overscroll-behavior: contain;
		padding: var(--sp-md);
		display: grid;
		gap: var(--sp-md);
		align-content: start;
	}

	.gauge-card {
		display: grid;
		gap: 8px;
		padding: 12px 14px;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
	}

	.gauge-head {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px;
	}

	.gauge-nums {
		font-family: var(--font-mono);
		font-size: 0.78125rem;
		color: var(--ink);
	}

	.pct {
		font-family: var(--font-mono);
		font-size: 0.8125rem;
		font-weight: 600;
		color: var(--sky-ink);
	}

	.pct.near {
		color: var(--duck-ink);
	}

	.gauge {
		position: relative;
		height: 12px;
		border: 1px solid var(--rule);
		border-radius: 999px;
		background: var(--bg-soft);
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
		border-left: 2px dashed var(--fail);
	}

	.thresh-note {
		margin: 0;
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.segments {
		display: grid;
		gap: 6px;
	}

	.segment {
		display: grid;
		gap: 2px;
		padding: 9px 12px;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
	}

	.seg-top {
		display: flex;
		align-items: baseline;
		justify-content: space-between;
		gap: 8px;
	}

	.seg-name {
		font-weight: 600;
		color: var(--ink);
	}

	.seg-tokens {
		font-family: var(--font-mono);
		font-size: 0.78125rem;
		color: var(--ink-2);
	}

	.seg-detail {
		font-size: 0.75rem;
		color: var(--ink-3);
	}

	.report-note,
	.empty {
		margin: 0;
		font-size: 0.78125rem;
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

	.strategy-strip {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		font-size: 0.75rem;
		color: var(--ink-2);
	}

	.seg-title-wrap {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}

	.segment.cached-zone {
		border-color: color-mix(in srgb, var(--ok) 35%, var(--rule));
	}

	.zone-pill,
	.zone-badge,
	.item-badge {
		font-family: var(--font-mono);
		font-size: 0.625rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 1px 6px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink-2);
	}

	.zone-pill.prefix,
	.zone-badge.prefix,
	.item-badge.cached {
		background: var(--ok-soft);
		color: var(--ok);
	}

	.zone-badge.tail {
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.item-badge.pruned {
		background: var(--sky-soft);
		color: var(--sky-ink);
	}

	.tail-section {
		display: flex;
		flex-direction: column;
		gap: 8px;
		padding-top: 6px;
		border-top: 1px solid var(--rule-soft);
	}

	.tail-head {
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.tail-title {
		font-size: 0.78125rem;
		font-weight: 600;
		color: var(--ink);
	}

	.tail-sub {
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.tail-list {
		display: flex;
		flex-direction: column;
		gap: 4px;
		max-height: 220px;
		overflow-y: auto;
	}

	.tail-item {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		padding: 6px 10px;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-md);
		background: var(--bg-panel);
		font-size: 0.75rem;
	}

	.tail-item.pruned {
		opacity: 0.65;
	}

	.tail-meta {
		display: flex;
		align-items: center;
		gap: 7px;
		min-width: 0;
		flex: 1;
	}

	.tail-role {
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		font-weight: 600;
		color: var(--ink-2);
		flex: none;
	}

	.tail-label {
		color: var(--ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.tail-actions {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		flex: none;
	}

	.tail-tok {
		font-family: var(--font-mono);
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.trim-btn {
		padding: 2px 7px;
		font-size: 0.6875rem;
		font-weight: 600;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink);
	}

	.trim-btn:hover:not(:disabled) {
		background: var(--duck-soft);
		color: var(--duck-ink);
	}
</style>
