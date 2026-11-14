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

	const title = $derived(
		approval.request.kind === 'shell'
			? 'quack asks to run a command'
			: approval.request.kind === 'edit'
				? 'quack asks to change a file'
				: `quack asks to use ${approval.request.name}`
	);

	const diffRows = $derived.by(() => {
		if (approval.request.kind !== 'edit') return [];
		return approval.request.diff.split('\n').map((line) => ({
			line,
			kind: line.startsWith('+++') || line.startsWith('---')
				? 'meta'
				: line.startsWith('@@')
					? 'hunk'
					: line.startsWith('+')
						? 'add'
						: line.startsWith('-')
							? 'del'
							: 'ctx'
		}));
	});
	const adds = $derived(diffRows.filter((r) => r.kind === 'add').length);
	const dels = $derived(diffRows.filter((r) => r.kind === 'del').length);
	const plateFile = $derived(
		approval.request.kind === 'edit' ? approval.request.path : null
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

<section class="plate" aria-label={`Approval: ${title}`}>
	<h2>{title}</h2>

	{#if approval.request.kind === 'shell'}
		<pre class="command">{approval.request.command}</pre>
		{#if approval.request.outside_sandbox}
			<p class="lead">This command runs <strong>outside the sandbox</strong>.</p>
		{/if}
	{:else if approval.request.kind === 'edit'}
		<p class="lead">
			Review the change below; the circled numbers mark each addition and deletion.
		</p>
		<div class="diff">
			<div class="dh">
				<b>{plateFile}</b>
				<span>specimen plate</span>
			</div>
			<pre class="dl">{#each diffRows as row, i (i)}<span class="line {row.kind}">{row.line}
</span>{/each}</pre>
		</div>
		<p class="marks">
			Field marks · {adds} addition{adds === 1 ? '' : 's'} · {dels} deletion{dels === 1 ? '' : 's'}
		</p>
	{:else}
		<pre class="command">{approval.request.arguments}</pre>
	{/if}

	<div class="actions">
		<button class="btn btn-primary" disabled={busy} onclick={() => onAnswer('allow_once')}>
			{approval.request.kind === 'edit' ? 'Approve change' : 'Allow'}
			<span class="kbd kbd-invert">⏎</span>
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
			Not this <span class="kbd">esc</span>
		</button>
	</div>
	<p class="waiting" role="status">
		<svg viewBox="0 0 24 24" aria-hidden="true"
			><circle cx="12" cy="12" r="8.5" fill="none" stroke="currentColor" stroke-width="1.7"/><path d="M12 7.5V12l3 2" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round"/></svg
		>
		quack is waiting for you
	</p>
</section>

<style>
	.plate {
		margin: 0 var(--sp-lg) var(--sp-md);
		border: 1px solid var(--plate-line);
		border-radius: var(--r-xl);
		background: var(--bg-raised);
		padding: 18px 20px 14px;
		position: relative;
		box-shadow: var(--shadow-pop);
	}

	/* The plate's inner ring, the way a guide book frames its figures. */
	.plate::after {
		content: '';
		position: absolute;
		inset: 5px;
		border: 1px solid var(--plate-ring);
		border-radius: calc(var(--r-xl) - 4px);
		pointer-events: none;
	}

	.plate > * {
		position: relative;
		z-index: 1;
	}

	h2 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 21px;
		font-weight: 600;
		color: var(--ink);
	}

	.lead {
		margin: 6px 0 12px;
		font-family: var(--font-serif);
		font-size: 14px;
		color: var(--ink-2);
		max-width: 60ch;
	}

	.command {
		margin: 12px 0;
		padding: 12px 14px;
		font-size: 12.5px;
		line-height: 1.5;
		background: var(--term);
		color: var(--term-text);
		border-radius: var(--r-lg);
		overflow-x: auto;
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.diff {
		border: 1px solid var(--plate-line);
		border-radius: var(--r-lg);
		background: var(--bg-raised);
		overflow: hidden;
		margin: 0 0 10px;
	}

	.dh {
		display: flex;
		justify-content: space-between;
		align-items: center;
		gap: 12px;
		height: 30px;
		padding: 0 12px;
		background: var(--bg-panel);
		border-bottom: 1px solid var(--rule-soft);
		font-size: 10.5px;
		font-weight: 600;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--ink-3);
	}

	.dh b {
		font-family: var(--font-mono);
		font-size: 11.5px;
		letter-spacing: 0;
		text-transform: none;
		color: var(--ink-2);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.dl {
		margin: 0;
		padding: 6px 0;
		font-family: var(--font-mono);
		font-size: 12.5px;
		line-height: 1.6;
		color: var(--ink);
		max-height: 260px;
		overflow: auto;
		counter-reset: mark;
	}

	.line {
		display: block;
		padding: 0 14px 0 46px;
		white-space: pre;
	}

	.line.meta {
		padding-left: 14px;
		color: var(--ink-3);
		font-size: 11px;
	}

	.line.meta + .line.meta {
		display: none;
	}

	.line.hunk {
		padding-left: 14px;
		color: var(--ink-3);
		font-size: 11px;
	}

	.line.ctx {
		color: var(--ink-2);
	}

	.line.add,
	.line.del {
		position: relative;
		background: color-mix(in srgb, var(--grn) 10%, transparent);
		font-weight: 600;
	}

	.line.del {
		background: color-mix(in srgb, var(--ver) 9%, transparent);
	}

	.line.add::before,
	.line.del::before {
		counter-increment: mark;
		content: counter(mark);
		position: absolute;
		left: 12px;
		top: 50%;
		transform: translateY(-50%);
		display: flex;
		align-items: center;
		justify-content: center;
		width: 17px;
		height: 17px;
		border: 1.5px solid currentColor;
		border-radius: 50%;
		font-family: var(--font-ui);
		font-size: 10px;
		font-weight: 700;
		line-height: 1;
	}

	.line.add {
		color: color-mix(in srgb, var(--grn) 80%, var(--ink));
	}

	.line.del {
		color: color-mix(in srgb, var(--ver) 85%, var(--ink));
	}

	.marks {
		margin: 0 0 12px;
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 13px;
		color: var(--ink-3);
	}

	.actions {
		display: flex;
		align-items: center;
		gap: 10px;
		flex-wrap: wrap;
	}

	.kbd-invert {
		border-color: color-mix(in srgb, #2b240f 55%, transparent);
		background: transparent;
		color: inherit;
	}

	.waiting {
		display: flex;
		align-items: center;
		gap: 8px;
		margin: 12px 0 0;
		font-family: var(--font-serif);
		font-style: italic;
		font-size: 13px;
		color: var(--ink-3);
	}

	.waiting svg {
		width: 14px;
		height: 14px;
		flex: none;
	}
</style>
