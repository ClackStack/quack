<script lang="ts">
	import type { Session } from '#lib/session.svelte';
	import AssistantMessage from './AssistantMessage.svelte';
	import ToolCard from './ToolCard.svelte';

	let { session }: { session: Session } = $props();

	let scroller: HTMLElement | undefined = $state();
	let pinned = $state(true);
	let rewindTargetBack = $state<number | null>(null);

	const idle = $derived(
		!session.closed &&
			!session.starting &&
			!session.switching &&
			!session.streaming &&
			!session.turnPending &&
			!session.approval &&
			!session.questions &&
			!session.stepLimit
	);

	const userBackById = $derived.by(() => {
		const map = new Map<number, number>();
		let back = 0;
		for (let i = session.items.length - 1; i >= 0; i--) {
			const it = session.items[i];
			if (it.kind === 'user') {
				back += 1;
				map.set(it.id, back);
			}
		}
		return map;
	});

	function toggleRewind(back: number) {
		if (!idle) return;
		if (rewindTargetBack === back) {
			rewindTargetBack = null;
			session.clearRewind();
			return;
		}
		rewindTargetBack = back;
		void session.listRewindPoints();
	}

	function cancelRewind() {
		rewindTargetBack = null;
		session.clearRewind();
	}

	function onScroll() {
		if (!scroller) return;
		pinned = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
	}

	$effect(() => {
		if (!idle && rewindTargetBack !== null) {
			rewindTargetBack = null;
		}
	});

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
				{@const back = userBackById.get(item.id) ?? 0}
				<div class="user-row" class:rewinding={rewindTargetBack === back}>
					<div class="user-head">
						<div class="cap">You</div>
						{#if idle && back > 0}
							<div class="turn-tools">
								<button
									type="button"
									class="turn-btn"
									onclick={() => toggleRewind(back)}
								>
									{rewindTargetBack === back ? 'Close restore' : 'Restore to here…'}
								</button>
								{#if back === 1}
									<button
										type="button"
										class="turn-btn"
										disabled={session.forking}
										title="Fork this conversation into a fresh copy and switch models"
										onclick={() => void session.forkSession(true)}
									>
										{session.forking ? 'Forking…' : 'Try another model'}
									</button>
								{/if}
							</div>
						{/if}
					</div>
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
					{#if rewindTargetBack === back}
						{@const point = session.rewindPoints?.find((p) => p.back === back) ?? null}
						{@const conflict = session.rewindConflict?.back === back ? session.rewindConflict : null}
						<div class="rewind-box" role="region" aria-label="Restore checkpoint">
							{#if session.rewindLoading && !session.rewindPoints}
								<p class="rewind-note">Inspecting checkpoint files…</p>
							{:else if point && point.files.length > 0}
								<p class="rewind-note">
									Modified after this note ({point.files.length}):
								</p>
								<div class="chips">
									{#each point.files as file (file)}<span class="chip mono">{file}</span>{/each}
								</div>
							{:else if point}
								<p class="rewind-note">No edited files tracked after this note.</p>
							{:else}
								<p class="rewind-note">No file diff snapshot for this earlier note; conversation history can still rewind.</p>
							{/if}

							{#if conflict}
								<div class="rewind-conflict" role="alert">
									<span>
										External edits detected in <code>{conflict.conflicts.join(', ')}</code>. Overwrite them or cancel?
									</span>
									<div class="rewind-actions">
										<button
											type="button"
											class="btn danger-btn"
											onclick={() => void session.rewind(conflict.back, conflict.conversation, conflict.files, true)}
										>
											Overwrite external changes
										</button>
										<button type="button" class="btn" onclick={cancelRewind}>Cancel</button>
									</div>
								</div>
							{:else}
								<div class="rewind-actions">
									{#if !point || point.files.length > 0}
										<button
											type="button"
											class="btn btn-primary"
											onclick={() => void session.rewind(back, true, true)}
										>
											Files + conversation
										</button>
									{/if}
									<button
										type="button"
										class="btn"
										onclick={() => void session.rewind(back, true, false)}
									>
										Conversation only
									</button>
									{#if point && point.files.length > 0}
										<button
											type="button"
											class="btn"
											onclick={() => void session.rewind(back, false, true)}
										>
											Files only
										</button>
									{/if}
									<button type="button" class="btn" onclick={cancelRewind}>Cancel</button>
								</div>
							{/if}
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
			{:else if item.kind === 'pruned'}
				<details class="pruned">
					<summary>
						trimmed ~{item.removedTokens.toLocaleString()} tokens from earlier context
						{#if item.items.length > 0}({item.items.length} items){/if}
					</summary>
					{#if item.items.length > 0}
						<ul class="pruned-list">
							{#each item.items as prunedItem, idx (`${item.id}-${idx}`)}
								<li>{prunedItem}</li>
							{/each}
						</ul>
					{/if}
				</details>
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

	.user-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
	}

	.turn-tools {
		display: flex;
		align-items: center;
		gap: 6px;
		opacity: 0;
		transition: opacity 140ms ease;
	}

	.user-row:hover .turn-tools,
	.user-row:focus-within .turn-tools,
	.user-row.rewinding .turn-tools {
		opacity: 1;
	}

	.turn-btn {
		font-size: 11px;
		color: var(--ink-3);
		padding: 1px 7px;
		border-radius: var(--r-md);
		border: 1px solid transparent;
	}

	.turn-btn:hover:not(:disabled) {
		color: var(--ink);
		background: var(--bg-panel);
		border-color: var(--rule);
	}

	.rewind-box {
		margin-top: 10px;
		padding-top: 10px;
		border-top: 1px solid var(--rule-soft);
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.rewind-note {
		margin: 0;
		font-family: var(--font-sans);
		font-size: 12px;
		color: var(--ink-2);
	}

	.rewind-actions {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.rewind-actions .btn {
		font-size: 12px;
		padding: 4px 10px;
	}

	.rewind-conflict {
		padding: 8px 10px;
		border-radius: var(--r-md);
		border: 1px solid color-mix(in srgb, var(--fail) 45%, var(--rule));
		background: var(--fail-soft);
		color: var(--fail);
		font-family: var(--font-sans);
		font-size: 12px;
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.danger-btn {
		border-color: var(--fail);
		color: var(--fail);
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

	.pruned {
		align-self: center;
		max-width: 72ch;
		border: 1px dashed var(--rule);
		border-radius: var(--r-lg);
		padding: 4px 12px;
		color: var(--ink-3);
		font-size: 12px;
	}

	.pruned summary {
		cursor: pointer;
		font-family: var(--font-serif);
		font-style: italic;
		user-select: none;
	}

	.pruned-list {
		margin: 6px 0 2px;
		padding-left: 18px;
		font-family: var(--font-mono);
		font-size: 11px;
		color: var(--ink-2);
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
