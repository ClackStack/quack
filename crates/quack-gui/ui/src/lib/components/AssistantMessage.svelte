<script lang="ts">
	import { renderMarkdown } from '#lib/markdown';
	import { getTransport, type HighlightSpan } from '#lib/transport';

	let { text, streaming = false }: { text: string; streaming?: boolean } = $props();

	const blocks = $derived(renderMarkdown(text));
	const spanCache = new Map<string, HighlightSpan[] | null>();
	let highlighted = $state<Record<string, HighlightSpan[] | null>>({});
	let copiedIndex = $state<number | null>(null);

	$effect(() => {
		const list = blocks;
		const isStreaming = streaming;
		const hl = getTransport().highlightCode;
		if (!hl) return;

		for (let i = 0; i < list.length; i++) {
			const block = list[i];
			if (block.type !== 'code' || !block.lang) continue;
			// Wait until the fence stabilizes or streaming ends before invoking tree-sitter.
			if (isStreaming && i === list.length - 1) continue;
			const key = `${block.lang}\0${block.text}`;
			if (spanCache.has(key)) continue;
			spanCache.set(key, null);
			void hl(block.lang, block.text)
				.then((spans) => {
					spanCache.set(key, spans);
					if (spans) highlighted = { ...highlighted, [key]: spans };
				})
				.catch(() => {
					spanCache.set(key, null);
				});
		}
	});

	function spansFor(lang: string, code: string): HighlightSpan[] | null {
		if (!lang) return null;
		return highlighted[`${lang}\0${code}`] ?? spanCache.get(`${lang}\0${code}`) ?? null;
	}

	async function copyCode(code: string, index: number) {
		try {
			await navigator.clipboard.writeText(code);
			copiedIndex = index;
			setTimeout(() => {
				if (copiedIndex === index) copiedIndex = null;
			}, 1600);
		} catch {
			// Ignore clipboard failures in restricted environments.
		}
	}
</script>

<div class="prose" class:streaming>
	{#each blocks as block, i (i)}
		{#if block.type === 'h'}
			<svelte:element this={`h${block.level}`}>{@html block.spans}</svelte:element>
		{:else if block.type === 'code'}
			{@const spans = spansFor(block.lang, block.text)}
			<div class="code-wrap">
				<div class="code-head">
					<span class="code-lang">{block.lang || 'text'}</span>
					<button
						type="button"
						class="code-copy"
						onclick={() => void copyCode(block.text, i)}
					>
						{copiedIndex === i ? 'Copied' : 'Copy'}
					</button>
				</div>
				<pre class="code" lang={block.lang}><code>{#if spans}{#each spans as span, j (j)}{#if span.kind}<span class={`tok-${span.kind.replace(/\./g, '-')}`}>{span.text}</span>{:else}{span.text}{/if}{/each}{:else}{block.text}{/if}</code></pre>
			</div>
		{:else if block.type === 'ul'}
			<ul>{#each block.items as item (item)}<li>{@html item}</li>{/each}</ul>
		{:else if block.type === 'ol'}
			<ol>{#each block.items as item (item)}<li>{@html item}</li>{/each}</ol>
		{:else if block.type === 'quote'}
			<blockquote>{@html block.spans}</blockquote>
		{:else if block.type === 'hr'}
			<hr />
		{:else}
			<p>{@html block.spans}</p>
		{/if}
	{/each}
	{#if streaming}<span class="tail" aria-hidden="true"></span>{/if}
</div>

<style>
	.prose > :first-child {
		margin-top: 0;
	}
	.prose > :last-child {
		margin-bottom: 0;
	}

	.prose p {
		margin: 0 0 0.8em;
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	h1,
	h2,
	h3,
	h4,
	h5,
	h6 {
		margin: 1.2em 0 0.4em;
		line-height: 1.25;
		letter-spacing: -0.015em;
	}

	h1 {
		font-size: 1.3em;
	}
	h2 {
		font-size: 1.2em;
	}
	h3 {
		font-size: 1.08em;
	}
	h4,
	h5,
	h6 {
		font-size: 1em;
	}

	ul,
	ol {
		margin: 0 0 0.8em;
		padding-left: 1.4em;
	}

	li {
		margin: 0.2em 0;
	}

	blockquote {
		margin: 0 0 0.8em;
		padding: 2px 0 2px 12px;
		border-left: 2px solid var(--rule);
		color: var(--ink-2);
	}

	.code-wrap {
		background: var(--term);
		color: var(--term-text);
		border-radius: var(--r-lg);
		margin: 0 0 0.8em;
		overflow: hidden;
		border: 1px solid color-mix(in srgb, var(--plate-line) 55%, transparent);
	}

	.code-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		padding: 4px 10px 4px 12px;
		background: color-mix(in srgb, var(--term) 78%, #000 22%);
		border-bottom: 1px solid rgba(255, 255, 255, 0.08);
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--term-dim);
	}

	.code-copy {
		color: var(--term-dim);
		font-size: 11px;
		padding: 1px 6px;
		border-radius: var(--r-sm);
	}

	.code-copy:hover {
		color: var(--term-text);
		background: rgba(255, 255, 255, 0.08);
	}

	.code {
		background: transparent;
		color: var(--term-text);
		padding: 11px 14px;
		overflow-x: auto;
		margin: 0;
		line-height: 1.55;
	}

	.code code {
		background: none;
		padding: 0;
	}

	:not(pre) > code {
		background: var(--bg-soft);
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-sm);
		padding: 1px 5px;
	}

	a {
		color: var(--sky-ink);
	}

	hr {
		border: none;
		border-top: 1px solid var(--rule);
		margin: 1em 0;
	}

	.tail {
		display: inline-block;
		width: 9px;
		height: 1em;
		vertical-align: text-bottom;
		background: var(--duck);
		animation: blink 900ms steps(1) infinite;
	}

	@keyframes blink {
		50% {
			opacity: 0;
		}
	}
</style>
