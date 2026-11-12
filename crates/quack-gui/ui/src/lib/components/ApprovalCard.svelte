<script lang="ts">
	import type { ApprovalRequest } from '#lib/protocol';

	let {
		approval,
		onAnswer,
		busy = false
	}: {
		approval: { callId: string; request: ApprovalRequest; always: string[] };
		busy?: boolean;
		onAnswer: (decision: 'allow_once' | 'allow_session' | 'deny') => void;
	} = $props();

	const kindLabel = $derived(
		approval.request.kind === 'shell'
			? 'run a command'
			: approval.request.kind === 'edit'
				? 'edit a file'
				: `use ${approval.request.name}`
	);

	function onKeydown(event: KeyboardEvent) {
		if (
			busy ||
			event.defaultPrevented ||
			event.repeat ||
			(event.target instanceof HTMLElement &&
				event.target.closest('button, input, textarea, select'))
		)
			return;
		if (event.key === 'Enter') onAnswer('allow_once');
		else if (event.key === 'Escape') onAnswer('deny');
	}
</script>

<svelte:window onkeydown={onKeydown} />

<section class="approval" aria-label={`Approval: ${kindLabel}`}>
	<div class="head">
		<span class="pill pill-live">Approval</span>
		<span class="what">
			{#if approval.request.kind === 'shell'}
				run <code>{approval.request.command}</code>
				{#if approval.request.outside_sandbox}<span class="outside">
						· outside the sandbox</span
					>{/if}
			{:else if approval.request.kind === 'edit'}
				edit <code>{approval.request.path}</code>
			{:else}
				<code>{approval.request.name}</code>
			{/if}
		</span>
	</div>

	{#if approval.request.kind === 'edit'}
		<pre class="diff">{#each approval.request.diff.split('\n') as line, i (i)}<span
					class="line {line.startsWith('+')
						? 'add'
						: line.startsWith('-')
							? 'del'
							: line.startsWith('+++') || line.startsWith('---')
								? 'meta'
								: 'ctx'}"
					>{line}
</span>{/each}</pre>
	{:else if approval.request.kind === 'tool'}
		<pre class="args">{approval.request.arguments}</pre>
	{/if}

	<div class="actions">
		<button class="btn btn-primary" disabled={busy} onclick={() => onAnswer('allow_once')}>
			Allow <span class="kbd kbd-invert">⏎</span>
		</button>
		{#if approval.always.length > 0}
			<button
				class="btn"
				disabled={busy}
				onclick={() => onAnswer('allow_session')}
				title={approval.always.join('\n')}
			>
				Allow for this session
			</button>
		{/if}
		<button class="btn btn-danger" disabled={busy} onclick={() => onAnswer('deny')}>
			Deny <span class="kbd">esc</span>
		</button>
	</div>
</section>

<style>
	.approval {
		margin: 0 var(--sp-lg) var(--sp-sm);
		border: 1px solid var(--duck-line);
		border-radius: var(--r-lg);
		background: var(--duck-soft);
		overflow: hidden;
	}

	.head {
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
		padding: 8px 12px;
	}

	.what {
		font-size: 13px;
		color: var(--duck-ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.what code {
		color: var(--ink);
	}

	.outside {
		color: var(--beak-ink);
		font-weight: 700;
	}

	.diff,
	.args {
		margin: 0;
		padding: 8px 12px;
		font-family: var(--font-mono);
		font-size: 12px;
		line-height: 1.5;
		background: var(--term);
		color: var(--term-text);
		max-height: 200px;
		overflow: auto;
		white-space: pre;
	}

	.line {
		display: block;
	}

	.line.add {
		background: color-mix(in srgb, var(--ok) 16%, transparent);
		color: var(--ok);
	}

	.line.del {
		background: color-mix(in srgb, var(--fail) 14%, transparent);
		color: var(--fail);
	}

	.line.meta {
		color: var(--term-dim);
	}

	.actions {
		display: flex;
		gap: var(--sp-sm);
		padding: 8px 12px;
	}

	.kbd-invert {
		border-color: currentColor;
		background: transparent;
		color: inherit;
	}
</style>
