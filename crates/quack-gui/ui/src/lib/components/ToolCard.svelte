<script lang="ts">
	import type { Item } from '#lib/session.svelte';

	let { item }: { item: Extract<Item, { kind: 'tool' }> } = $props();

	let open = $state(false);

	const running = $derived(item.ok === null);
	const showBody = $derived(open && (item.output.length > 0 || item.detail));
	const isDiff = $derived(
		item.detail !== null &&
			item.detail !== '' &&
			(item.name === 'write' || item.name === 'edit' || /^(\+\+\+|---|@@)/m.test(item.detail))
	);

	function diffLines(detail: string) {
		return detail.split('\n').map((line, i) => ({
			line,
			i,
			kind: line.startsWith('+++') || line.startsWith('---')
				? 'meta'
				: line.startsWith('+')
					? 'add'
					: line.startsWith('-')
						? 'del'
						: 'ctx'
		}));
	}
</script>

<div class="card" class:running class:failed={item.ok === false}>
	<button class="row" onclick={() => (open = !open)} aria-expanded={open}>
		<span class="marker" class:running class:ok={item.ok === true} class:fail={item.ok === false} aria-hidden="true"></span>
		<span class="summary">{item.summary}</span>
		{#if item.output.length > 0 || item.detail}
			<span class="hint">{open ? 'hide' : 'show'}</span>
		{/if}
	</button>

	{#if !open && !running && item.output.length > 0}
		<div class="preview">{item.output.slice(-160)}</div>
	{/if}

	{#if showBody}
		<div class="body">
			{#if isDiff && item.detail}
				<pre class="diff">{#each diffLines(item.detail) as d (d.i)}<span class="line {d.kind}">{d.line}
</span>{/each}</pre>
			{:else if item.detail}
				<pre class="out">{item.detail}</pre>
			{:else}
				<pre class="out">{item.output}</pre>
			{/if}
		</div>
	{/if}
</div>

<style>
	.card {
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		overflow: hidden;
	}

	.card.running {
		border-color: var(--duck-line);
	}

	.card.failed {
		border-color: var(--fail);
	}

	.row {
		width: 100%;
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 6px 12px;
		text-align: left;
		font-family: var(--font-mono);
		font-size: 12.5px;
	}

	.row:hover {
		background: var(--bg-soft);
	}

	.marker {
		width: 8px;
		height: 8px;
		border-radius: 999px;
		background: var(--rule);
		flex: none;
	}

	.marker.running {
		background: var(--duck);
		animation: pulse 1.1s ease-out infinite alternate;
	}

	.marker.ok {
		background: var(--ok);
	}

	.marker.fail {
		background: var(--fail);
	}

	@keyframes pulse {
		from {
			opacity: 1;
		}
		to {
			opacity: 0.35;
		}
	}

	.summary {
		flex: 1;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
		color: var(--ink-2);
	}

	.card.running .summary {
		color: var(--ink);
	}

	.hint {
		color: var(--ink-3);
		font-size: 11px;
		text-transform: uppercase;
		letter-spacing: 0.05em;
	}

	.preview {
		padding: 0 12px 8px 30px;
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--ink-3);
		white-space: pre-wrap;
		overflow: hidden;
		max-height: 3.2em;
	}

	.body {
		border-top: 1px solid var(--rule-soft);
	}

	.diff,
	.out {
		margin: 0;
		padding: 10px 12px;
		font-family: var(--font-mono);
		font-size: 12px;
		line-height: 1.5;
		overflow-x: auto;
		max-height: 320px;
		overflow-y: auto;
		white-space: pre;
	}

	/* Diffs are plates: paper, ink, and the guide's change colours. */
	.diff {
		background: var(--bg-panel);
		color: var(--ink);
	}

	.out {
		background: var(--term);
		color: var(--term-text);
	}

	.line {
		display: block;
	}

	.line.add {
		background: color-mix(in srgb, var(--grn) 12%, transparent);
		color: color-mix(in srgb, var(--grn) 75%, var(--ink));
	}

	.line.del {
		background: color-mix(in srgb, var(--ver) 10%, transparent);
		color: color-mix(in srgb, var(--ver) 80%, var(--ink));
	}

	.line.meta {
		color: var(--ink-3);
	}
</style>
