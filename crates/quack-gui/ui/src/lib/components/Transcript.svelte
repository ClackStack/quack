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
					<div class="cap">You</div>
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
					<div class="who">
						<svg viewBox="0 0 24 24" aria-hidden="true"
							><circle cx="8.7" cy="6" r="2.7" fill="none" stroke="currentColor" stroke-width="1.7"/>
							<path d="M6.3 4.9 3.4 6l2.9 1.1" fill="none" stroke="currentColor" stroke-width="1.7"
								stroke-linecap="round" stroke-linejoin="round"/>
							<path d="M3.4 14.4c0-3 2.4-5.4 5.4-5.4h2.3c3.5 0 6.3 2.3 6.3 5.1 0 2.4-2.1 4.3-5.3 4.3H7.1c-2 0-3.7-1.7-3.7-4z" fill="none"
								stroke="currentColor" stroke-width="1.7" stroke-linejoin="round"/>
							<path d="M12 15.2a3.2 3.2 0 0 0-3.2-3.2" fill="none" stroke="currentColor" stroke-width="1.7"/>
							<path d="M19.6 18.8h1.9" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg
						>
						quack
					</div>
					<AssistantMessage text={item.text} streaming={item.streaming} />
				</article>
			{:else if item.kind === 'reasoning'}
				<details class="reasoning">
					<summary>field thoughts</summary>
					<p>{item.text}</p>
				</details>
			{:else if item.kind === 'tool'}
				<ToolCard item={item} />
			{:else if item.kind === 'compacted'}
				<div class="compacted">older notes summarized for the trail ahead</div>
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
		padding: var(--sp-lg) var(--sp-lg) var(--sp-md);
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
		color: var(--sky-ink);
		line-height: 1.25;
		margin: 0;
	}

	.starting p {
		margin: var(--sp-xs) 0 0;
		color: var(--ink-3);
	}

	/* Your notes: the vermilion edge of a field entry. */
	.user-row {
		background: var(--user-row);
		border: 1px solid var(--rule-soft);
		border-left: 1px solid var(--ver);
		border-radius: var(--r-lg);
		padding: 10px 14px;
		max-width: 76ch;
	}

	.cap {
		font-size: 10.5px;
		font-weight: 700;
		letter-spacing: 0.1em;
		text-transform: uppercase;
		color: var(--ver);
	}

	.user-row p {
		margin: 4px 0 0;
		font-family: var(--font-serif);
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
		background: var(--bg-panel);
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		padding: 2px 8px;
		color: var(--ink-2);
	}

	.mono {
		font-family: var(--font-mono);
	}

	/* quack's notes: the blue-ink entries, serif like the guide's prose. */
	.assistant {
		max-width: 76ch;
	}

	.who {
		display: flex;
		align-items: center;
		gap: 7px;
		font-size: 10.5px;
		font-weight: 700;
		letter-spacing: 0.1em;
		text-transform: uppercase;
		color: var(--sky-ink);
		margin-bottom: 2px;
	}

	.who svg {
		width: 16px;
		height: 16px;
		flex: none;
	}

	.assistant :global(.prose) {
		font-family: var(--font-serif);
		font-size: var(--text-prose);
		line-height: var(--leading-prose);
	}

	.reasoning {
		color: var(--ink-3);
		font-size: 13px;
		border-left: 1px solid var(--rule);
		padding-left: 12px;
	}

	.reasoning summary {
		cursor: pointer;
		color: var(--ink-2);
		user-select: none;
		font-style: italic;
		font-family: var(--font-serif);
	}

	.reasoning p {
		white-space: pre-wrap;
		margin: 6px 0 0;
	}

	.compacted {
		align-self: center;
		color: var(--ink-3);
		font-size: 12px;
		font-family: var(--font-serif);
		font-style: italic;
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
		gap: 8px;
		color: var(--ink-3);
		font-size: 12.5px;
	}

	.measure {
		font-family: var(--font-mono);
		font-size: 11.5px;
	}

	.cursor {
		display: inline-block;
		width: 8px;
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

	.stop {
		border-color: var(--fail);
		color: var(--fail);
		padding: 2px 10px;
	}

	.stop:hover:not(:disabled) {
		background: var(--fail-soft);
	}
</style>
