<script lang="ts">
	import { renderMarkdown } from '#lib/markdown';

	let { text, streaming = false }: { text: string; streaming?: boolean } = $props();

	const blocks = $derived(renderMarkdown(text));
</script>

<div class="prose" class:streaming>
	{#each blocks as block, i (i)}
		{#if block.type === 'h'}
			<svelte:element this={`h${block.level}`}>{block.spans}</svelte:element>
		{:else if block.type === 'code'}
			<pre class="code" lang={block.lang}><code>{block.text}</code></pre>
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

	.code {
		background: var(--term);
		color: var(--term-text);
		border-radius: var(--r-lg);
		padding: 12px 14px;
		overflow-x: auto;
		margin: 0 0 0.8em;
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
