<script lang="ts">
	import type { Session } from '#lib/session.svelte';
	import AssistantMessage from './AssistantMessage.svelte';
	import ToolCard from './ToolCard.svelte';

	let { session }: { session: Session } = $props();

	let scroller: HTMLElement | undefined = $state();
	let pinned = $state(true);

	function onScroll() {
		if (!scroller) return;
		pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
	}

	$effect(() => {
		void session.items.length;
		void session.streaming;
		if (pinned && scroller) scroller.scrollTop = scroller.scrollHeight;
	});
</script>

<div class="transcript" bind:this={scroller} onscroll={onScroll}>
	{#if session.starting}
		<div class="starting">
			<pre aria-hidden="true">   __
&lt;(o )___
 ( ._&gt; /
  `---'
~~~~~~~~~~</pre>
			<p>starting the engine…</p>
		</div>
	{:else if session.items.length === 0}
		<div class="empty">
			<pre aria-hidden="true">   __
&lt;(o )___
 ( ._&gt; /
  `---'
~~~~~~~~~~

ask quack to do something in this folder</pre>
		</div>
	{:else}
		{#each session.items as item (item.id)}
			{#if item.kind === 'user'}
				<div class="user-row">
					<p>{item.text}</p>
					{#if item.files.length > 0}
						<div class="chips">
							{#each item.files as file (file)}<span class="chip mono">{file}</span>{/each}
						</div>
					{/if}
					{#if item.images.length > 0}
						<div class="chips">
							{#each item.images as image (image)}<span class="chip mono">🖼 {image}</span>{/each}
						</div>
					{/if}
				</div>
			{:else if item.kind === 'assistant'}
				<article class="assistant">
					<AssistantMessage text={item.text} streaming={item.streaming} />
				</article>
			{:else if item.kind === 'reasoning'}
				<details class="reasoning">
					<summary>thinking</summary>
					<p>{item.text}</p>
				</details>
			{:else if item.kind === 'tool'}
				<ToolCard item={item} />
			{:else if item.kind === 'compacted'}
				<div class="compacted">older history summarized</div>
			{:else}
				<div class="system {item.tone}" role={item.tone === 'error' ? 'alert' : 'status'}>
					{item.text}
				</div>
			{/if}
		{/each}
		{#if session.streaming && !session.approval}
			<div class="status">
				streaming<span class="cursor" aria-hidden="true"></span>
				{#if session.speed}<span class="measure"> · first token {session.speed.ttftMs} ms{session.speed.tokensPerSec ? ` · ${session.speed.tokensPerSec.toFixed(0)} tok/s` : ''}</span>{/if}
				<button class="btn stop" onclick={() => session.interrupt()}>Interrupt</button>
			</div>
		{/if}
	{/if}
</div>

<style>
	.transcript {
		flex: 1;
		overflow-y: auto;
		padding: var(--sp-md) var(--sp-lg) var(--sp-lg);
		display: flex;
		flex-direction: column;
		gap: var(--sp-md);
		scroll-behavior: smooth;
	}

	.starting,
	.empty {
		color: var(--ink-3);
		font-family: var(--font-mono);
		padding-top: var(--sp-xl);
		text-align: center;
	}

	.starting pre,
	.empty pre {
		display: inline-block;
		text-align: left;
		color: var(--beak-ink);
		line-height: 1.25;
		margin: 0;
	}

	.starting p {
		margin: var(--sp-xs) 0 0;
		color: var(--ink-3);
	}

	.user-row {
		background: var(--user-row);
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-lg);
		padding: 10px 14px;
	}

	.user-row p {
		margin: 0;
		font-size: var(--text-prose);
		line-height: var(--leading-prose);
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.chips {
		display: flex;
		flex-wrap: wrap;
		gap: var(--sp-xs);
		margin-top: var(--sp-sm);
	}

	.chip {
		font-size: 11.5px;
		background: var(--bg-soft);
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		padding: 2px 8px;
		color: var(--ink-2);
	}

	.mono {
		font-family: var(--font-mono);
	}

	.assistant {
		font-size: var(--text-prose);
		line-height: var(--leading-prose);
		max-width: 76ch;
	}

	.reasoning {
		color: var(--ink-3);
		font-size: 13px;
		border-left: 2px solid var(--rule);
		padding-left: 12px;
	}

	.reasoning summary {
		cursor: pointer;
		color: var(--ink-2);
		user-select: none;
	}

	.reasoning p {
		white-space: pre-wrap;
		margin: 6px 0 0;
	}

	.compacted {
		align-self: center;
		color: var(--ink-3);
		font-size: 12px;
		border: 1px dashed var(--rule);
		border-radius: 999px;
		padding: 3px 12px;
	}

	.system {
		font-size: 13px;
		padding: 8px 12px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		color: var(--ink-2);
	}

	.system.warning {
		border-color: var(--duck-line);
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.system.error {
		border-color: var(--fail);
		background: var(--fail-soft);
		color: var(--fail);
	}

	.status {
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
		color: var(--ink-2);
		font-size: 12.5px;
	}

	.measure {
		font-family: var(--font-mono);
		color: var(--ink-3);
	}

	.cursor {
		width: 8px;
		height: 15px;
		background: var(--duck);
		display: inline-block;
		animation: blink 900ms steps(1) infinite;
	}

	@keyframes blink {
		50% {
			opacity: 0;
		}
	}

	.stop {
		border-color: var(--fail);
		color: var(--fail);
		padding: 2px 10px;
	}

	.stop:hover:not(:disabled) {
		background: var(--fail-soft);
	}
</style>
