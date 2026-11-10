<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session }: { session: Session } = $props();

	const planItems = $derived.by(() => {
		const text = session.plan.trim();
		if (!text) return [];
		return text.split('\n').map((line) => {
			const match = /^\[([x~ ])\]\s*(.*)$/.exec(line.trim());
			return {
				state: (match?.[1] ?? ' ') as 'x' | '~' | ' ',
				text: match?.[2] ?? line
			};
		});
	});

	const doneCount = $derived(planItems.filter((p) => p.state === 'x').length);
</script>

<aside>
	<section>
		<h3>Session</h3>
		<p class="title">{session.title || 'New session'}</p>
		<div class="row">
			<button class="btn slim" onclick={() => void session.newSession()}>New</button>
			<button class="btn slim" onclick={() => void session.listSessions()}>Refresh</button>
			{#if session.items.length > 0}
				<button class="btn slim" onclick={() => void session.undo()} title="Undo the last edit call">Undo</button>
			{/if}
		</div>
		{#if session.speed}
			<p class="measure">
				last call: {session.speed.ttftMs} ms to first token{session.speed.tokensPerSec
					? ` · ${session.speed.tokensPerSec.toFixed(0)} tok/s`
					: ''}
			</p>
		{/if}
	</section>

	{#if session.agents.length > 0}
		<section>
			<h3>Sub-agents</h3>
			<ul>
				{#each session.agents as run (run.id)}
					<li class="agent">
						<div class="head">
							<span class="marker" class:running={run.ok === null} class:ok={run.ok === true} class:fail={run.ok === false} aria-hidden="true"></span>
							<span class="name">{run.id}</span>
							<span class="note">{run.description}</span>
						</div>
						{#if run.text}
							<p class="snippet">{run.text.slice(-200)}</p>
						{/if}
						{#if run.summary}
							<p class="note">{run.summary}</p>
						{/if}
					</li>
				{/each}
			</ul>
		</section>
	{/if}

	{#if planItems.length > 0}
		<section>
			<h3>Plan <span class="count">{doneCount}/{planItems.length}</span></h3>
			<ul class="plan">
				{#each planItems as item, i (i)}
					<li>
						<span class="box" class:done={item.state === 'x'} class:doing={item.state === '~'} aria-hidden="true"></span>
						<span>{item.text}</span>
					</li>
				{/each}
			</ul>
		</section>
	{/if}

	<section>
		<h3>Sessions</h3>
		<ul>
			{#each session.sessions as saved (saved.id)}
				<li>
					<button
						class="saved"
						class:current={saved.current}
						onclick={() => void session.resumeSession(saved.id)}
					>
						<span class="name">{saved.title || 'Untitled'}</span>
						<span class="note">{saved.model} · {saved.turns} turns · {saved.age}</span>
					</button>
				</li>
			{:else}
				<li class="empty">no saved sessions</li>
			{/each}
		</ul>
	</section>
</aside>

<style>
	aside {
		border-left: 1px solid var(--rule);
		background: var(--bg-soft);
		overflow-y: auto;
		padding: var(--sp-md) var(--sp-md) var(--sp-lg);
		display: grid;
		gap: var(--sp-lg);
		align-content: start;
		min-height: 0;
	}

	section {
		display: grid;
		gap: var(--sp-sm);
	}

	h3 {
		margin: 0;
		font-size: 12px;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		color: var(--ink-3);
		display: flex;
		gap: 8px;
		align-items: baseline;
	}

	.count {
		color: var(--ink-3);
		font-family: var(--font-mono);
		font-weight: 400;
		text-transform: none;
		letter-spacing: 0;
	}

	.title {
		margin: 0;
		font-size: 14px;
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.row {
		display: flex;
		gap: var(--sp-xs);
		flex-wrap: wrap;
	}

	.btn.slim {
		padding: 3px 10px;
		font-size: 12.5px;
	}

	.measure {
		margin: 0;
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--ink-3);
	}

	ul {
		list-style: none;
		margin: 0;
		padding: 0;
		display: grid;
		gap: 2px;
	}

	.saved,
	.agent {
		display: grid;
		gap: 2px;
	}

	.saved {
		width: 100%;
		text-align: left;
		padding: 7px 10px;
		border-radius: var(--r-md);
		border: 1px solid transparent;
	}

	.saved:hover {
		background: var(--bg-panel);
		border-color: var(--rule);
	}

	.saved.current {
		background: var(--bg-panel);
		border-color: var(--sky);
	}

	.name {
		font-weight: 600;
		font-size: 13px;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.note {
		color: var(--ink-3);
		font-size: 11.5px;
	}

	.empty {
		color: var(--ink-3);
		font-size: 12.5px;
		padding: 4px 10px;
	}

	.agent {
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		background: var(--bg-panel);
		padding: 8px 10px;
	}

	.agent .head {
		display: flex;
		align-items: center;
		gap: 8px;
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

	.agent .name {
		font-family: var(--font-mono);
		font-weight: 600;
	}

	.snippet {
		margin: 4px 0 0;
		color: var(--ink-2);
		font-size: 12px;
		display: -webkit-box;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.plan {
		gap: 6px;
	}

	.plan li {
		display: flex;
		gap: 10px;
		align-items: baseline;
		font-size: 13px;
	}

	.box {
		width: 11px;
		height: 11px;
		border: 1.5px solid var(--ink-3);
		border-radius: 3px;
		flex: none;
		transform: translateY(1px);
	}

	.box.done {
		background: var(--ok);
		border-color: var(--ok);
	}

	.box.doing {
		border-color: var(--duck-ink);
		background: var(--duck-soft);
	}
</style>
