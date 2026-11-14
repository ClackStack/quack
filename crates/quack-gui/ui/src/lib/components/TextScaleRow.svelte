<script lang="ts">
	import {
		MAX_TEXT_SCALE,
		MIN_TEXT_SCALE,
		TEXT_SCALE_PRESETS,
		TEXT_SCALE_STEP,
		textScale
	} from '#lib/textScale.svelte';

	const minPercent = Math.round(MIN_TEXT_SCALE * 100);
	const maxPercent = Math.round(MAX_TEXT_SCALE * 100);
	const stepPercent = Math.round(TEXT_SCALE_STEP * 100);

	const uiPx = $derived(Number((13.5 * textScale.scale).toFixed(1)));
	const prosePx = $derived(Number((15 * textScale.scale).toFixed(1)));

	function onRangeInput(e: Event) {
		const target = e.currentTarget as HTMLInputElement;
		const parsed = Number.parseFloat(target.value);
		if (Number.isFinite(parsed)) {
			textScale.setPercent(parsed);
		}
	}
</script>

<div class="field-row" data-setting-key="desktop.text_scale">
	<div class="field-info">
		<div class="field-head">
			<span class="field-label">Text size</span>
			<code class="field-key">desktop.text_scale</code>
			<span class="badge desktop" title="Applies only to the desktop app and saves locally">
				Desktop
			</span>
		</div>
		<p class="field-desc">
			Resize interface, sidebar, transcript prose, and code text across the desktop window. Saved
			locally on this device.
		</p>
		<div class="field-meta">
			<span>Effective: <code>{uiPx}px</code> UI · <code>{prosePx}px</code> prose</span>
			<span aria-hidden="true">·</span>
			<span>Storage: <code>browser local</code></span>
		</div>
	</div>

	<div class="field-control">
		<div class="scale-bar">
			<button
				type="button"
				class="step-btn"
				onclick={() => textScale.step(-1)}
				disabled={textScale.percent <= minPercent}
				aria-label="Decrease text size"
				title="Decrease text size"
			>
				A−
			</button>
			<input
				type="range"
				class="scale-range"
				min={minPercent}
				max={maxPercent}
				step={stepPercent}
				value={textScale.percent}
				oninput={onRangeInput}
				aria-label="Desktop text size percentage"
			/>
			<button
				type="button"
				class="step-btn"
				onclick={() => textScale.step(1)}
				disabled={textScale.percent >= maxPercent}
				aria-label="Increase text size"
				title="Increase text size"
			>
				A+
			</button>
			<output class="scale-readout">{textScale.percent}%</output>
		</div>

		<div class="preset-row">
			<div class="presets" role="group" aria-label="Text size presets">
				{#each TEXT_SCALE_PRESETS as preset (preset)}
					<button
						type="button"
						class="preset-btn"
						class:active={textScale.percent === preset}
						aria-pressed={textScale.percent === preset}
						onclick={() => textScale.setPercent(preset)}
					>
						{preset}%
					</button>
				{/each}
			</div>
			<button
				type="button"
				class="reset-btn"
				onclick={() => textScale.reset()}
				disabled={textScale.isDefault}
			>
				Reset
			</button>
		</div>
	</div>
</div>

<style>
	.field-row {
		display: grid;
		grid-template-columns: minmax(0, 1.3fr) minmax(240px, 1fr);
		gap: 14px;
		align-items: start;
		padding: 12px 14px;
		background: var(--bg-raised);
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-lg);
	}

	@media (max-width: 780px) {
		.field-row {
			grid-template-columns: 1fr;
			gap: 10px;
		}
	}

	.field-info {
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.field-head {
		display: flex;
		align-items: center;
		gap: 7px;
		flex-wrap: wrap;
	}

	.field-label {
		font-weight: 600;
		font-size: 0.84375rem;
		color: var(--ink);
	}

	.field-key {
		font-size: 0.6875rem;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		background: var(--bg-soft);
		color: var(--ink-3);
	}

	.badge {
		display: inline-flex;
		align-items: center;
		padding: 1px 6px;
		border-radius: 999px;
		font-size: 0.625rem;
		font-weight: 700;
		letter-spacing: 0.04em;
		text-transform: uppercase;
	}

	.badge.desktop {
		background: var(--sky-soft);
		color: var(--sky-ink);
	}

	.field-desc {
		margin: 0;
		font-size: 0.78125rem;
		line-height: 1.45;
		color: var(--ink-2);
	}

	.field-meta {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px;
		margin-top: 2px;
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.field-meta code {
		font-size: 0.6875rem;
		padding: 0 4px;
		border-radius: var(--r-sm);
		background: var(--bg-soft);
		color: var(--ink-2);
	}

	.field-control {
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.scale-bar {
		display: flex;
		align-items: center;
		gap: 6px;
	}

	.step-btn {
		flex: none;
		min-height: 1.875rem;
		padding: 0 9px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-panel);
		color: var(--ink);
		font-size: 0.75rem;
		font-weight: 600;
	}

	.step-btn:hover:not(:disabled) {
		border-color: var(--sky);
		background: var(--bg-soft);
	}

	.step-btn:disabled {
		opacity: 0.45;
		cursor: default;
	}

	.scale-range {
		flex: 1;
		min-width: 72px;
		accent-color: var(--sky-ink);
		cursor: pointer;
	}

	.scale-readout {
		flex: none;
		min-width: 3.5rem;
		text-align: center;
		font-family: var(--font-mono);
		font-size: 0.75rem;
		font-variant-numeric: tabular-nums;
		padding: 3px 6px;
		border-radius: var(--r-sm);
		background: var(--bg-soft);
		color: var(--ink);
		border: 1px solid var(--rule-soft);
	}

	.preset-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		flex-wrap: wrap;
	}

	.presets {
		display: inline-flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 4px;
	}

	.preset-btn,
	.reset-btn {
		min-height: 1.5rem;
		padding: 2px 8px;
		border-radius: var(--r-sm);
		border: 1px solid var(--rule-soft);
		background: var(--bg-panel);
		color: var(--ink-2);
		font-family: var(--font-mono);
		font-size: 0.71875rem;
		font-variant-numeric: tabular-nums;
	}

	.preset-btn:hover:not(:disabled),
	.reset-btn:hover:not(:disabled) {
		border-color: var(--rule);
		color: var(--ink);
		background: var(--bg-soft);
	}

	.preset-btn.active {
		background: var(--sky-soft);
		border-color: var(--sky);
		color: var(--sky-ink);
		font-weight: 600;
	}

	.reset-btn {
		font-family: var(--font-ui);
		font-size: 0.71875rem;
	}

	.reset-btn:disabled {
		opacity: 0.45;
		cursor: default;
	}
</style>
