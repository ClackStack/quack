<script lang="ts">
	import type { ApprovalRequest, WriteScope } from '#lib/protocol';
	import { summarizeShellCommand } from '#lib/shellRisk';

	let {
		approval,
		onAnswer,
		busy = false
	}: {
		approval: { callId: string; request: ApprovalRequest; always: string[] };
		busy?: boolean;
		onAnswer: (
			decision: 'allow_once' | 'allow_session' | 'always_allow' | 'deny',
			feedback?: string,
			scope?: WriteScope
		) => void;
	} = $props();

	let feedbackOpen = $state(false);
	let feedbackText = $state('');
	let saveOpen = $state(false);
	let saveScope = $state<'project' | 'user'>('project');
	let feedbackEl = $state<HTMLInputElement | null>(null);

	$effect(() => {
		void approval.callId;
		feedbackOpen = false;
		feedbackText = '';
		saveOpen = false;
	});

	$effect(() => {
		if (feedbackOpen) feedbackEl?.focus();
	});

	const title = $derived(
		approval.request.kind === 'shell'
			? 'quack asks to run a command'
			: approval.request.kind === 'edit'
				? 'quack asks to change a file'
				: `quack asks to use ${approval.request.name}`
	);

	const shellRisk = $derived(
		approval.request.kind === 'shell'
			? summarizeShellCommand(approval.request.command, approval.request.outside_sandbox ?? false)
			: null
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
		{#if shellRisk}
			<div class={`risk risk-${shellRisk.level}`}>
				<div class="risk-head">
					<span class="risk-pill">{shellRisk.level === 'high' ? 'High impact' : shellRisk.level === 'medium' ? 'Moderate impact' : 'Workspace command'}</span>
					<span class="risk-title">{shellRisk.summary}</span>
				</div>
				<ul class="risk-list">
					{#each shellRisk.reasons as reason (reason)}
						<li>{reason}</li>
					{/each}
				</ul>
			</div>
		{/if}
		<pre class="command">{approval.request.command}</pre>
		<p class="lead">
			Runs in <code>{approval.request.cwd}</code>. Approve if this step matches what you asked for.
		</p>
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
			<button
				class="btn"
				disabled={busy}
				aria-expanded={saveOpen}
				onclick={() => (saveOpen = !saveOpen)}
				title={approval.always.join('\n')}
			>
				Save rule…
			</button>
		{/if}
		<button
			class="btn"
			disabled={busy}
			aria-expanded={feedbackOpen}
			onclick={() => (feedbackOpen = !feedbackOpen)}
		>
			Not this — tell quack why…
		</button>
		<button class="btn btn-danger" disabled={busy} onclick={() => onAnswer('deny')}>
			Not this <span class="kbd">esc</span>
		</button>
	</div>

	{#if saveOpen && approval.always.length > 0}
		<div class="subpanel">
			<div class="subpanel-row">
				<span class="sub-label">Remember rule:</span>
				<code class="rule-code">{approval.always.join(', ')}</code>
			</div>
			<div class="subpanel-row">
				<label class="scope-opt">
					<input type="radio" name="rule-scope" value="project" bind:group={saveScope} />
					<span>This project only</span>
				</label>
				<label class="scope-opt">
					<input type="radio" name="rule-scope" value="user" bind:group={saveScope} />
					<span>All my projects (user settings)</span>
				</label>
				<div class="grow"></div>
				<button
					class="btn btn-primary"
					disabled={busy}
					onclick={() => onAnswer('always_allow', undefined, saveScope)}
				>
					Save &amp; allow
				</button>
			</div>
		</div>
	{/if}

	{#if feedbackOpen}
		<form
			class="subpanel"
			onsubmit={(e) => {
				e.preventDefault();
				if (feedbackText.trim()) onAnswer('deny', feedbackText.trim());
			}}
		>
			<label class="sub-label" for="approval-feedback">What should quack do instead?</label>
			<div class="feedback-row">
				<input
					id="approval-feedback"
					type="text"
					class="feedback-input"
					placeholder="e.g. Use cargo check instead of modifying that file…"
					bind:value={feedbackText}
					bind:this={feedbackEl}
					onkeydown={(e) => {
						if (e.key === 'Escape') {
							e.preventDefault();
							feedbackOpen = false;
						}
					}}
				/>
				<button type="submit" class="btn btn-primary" disabled={busy || !feedbackText.trim()}>
					Send guidance
				</button>
			</div>
		</form>
	{/if}
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
		font-size: 1.3125rem;
		font-weight: 600;
		color: var(--ink);
	}

	.lead {
		margin: 6px 0 12px;
		font-family: var(--font-serif);
		font-size: 0.875rem;
		color: var(--ink-2);
		max-width: 60ch;
	}

	.command {
		margin: 12px 0;
		padding: 12px 14px;
		font-size: 0.78125rem;
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
		min-height: 1.875rem;
		padding: 0 12px;
		background: var(--bg-panel);
		border-bottom: 1px solid var(--rule-soft);
		font-size: 0.65625rem;
		font-weight: 600;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--ink-3);
	}

	.dh b {
		font-family: var(--font-mono);
		font-size: 0.71875rem;
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
		font-size: 0.78125rem;
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
		font-size: 0.6875rem;
	}

	.line.meta + .line.meta {
		display: none;
	}

	.line.hunk {
		padding-left: 14px;
		color: var(--ink-3);
		font-size: 0.6875rem;
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
		width: 1.0625rem;
		height: 1.0625rem;
		border: 1.5px solid currentColor;
		border-radius: 50%;
		font-family: var(--font-ui);
		font-size: 0.625rem;
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
		font-size: 0.8125rem;
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
		font-size: 0.8125rem;
		color: var(--ink-3);
	}

	.waiting svg {
		width: 14px;
		height: 14px;
		flex: none;
	}

	.risk {
		margin: 10px 0 6px;
		padding: 10px 12px;
		border-radius: var(--r-lg);
		border: 1px solid var(--rule);
		background: var(--bg-panel);
	}

	.risk-high {
		border-color: var(--fail);
		background: var(--fail-soft);
	}

	.risk-medium {
		border-color: var(--duck-line);
		background: var(--duck-soft);
	}

	.risk-low {
		border-color: color-mix(in srgb, var(--sky) 45%, var(--rule));
		background: var(--sky-soft);
	}

	.risk-head {
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}

	.risk-pill {
		font-size: 0.65625rem;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid currentColor;
	}

	.risk-high .risk-pill,
	.risk-high .risk-title {
		color: var(--fail);
	}

	.risk-medium .risk-pill,
	.risk-medium .risk-title {
		color: var(--duck-ink);
	}

	.risk-low .risk-pill,
	.risk-low .risk-title {
		color: var(--sky-ink);
	}

	.risk-title {
		font-size: 0.8125rem;
		font-weight: 600;
	}

	.risk-list {
		margin: 6px 0 0;
		padding-left: 18px;
		font-size: 0.78125rem;
		color: var(--ink-2);
	}

	.subpanel {
		margin-top: 12px;
		padding: 10px 12px;
		border-radius: var(--r-lg);
		border: 1px solid var(--rule);
		background: var(--bg-panel);
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.subpanel-row {
		display: flex;
		align-items: center;
		gap: 10px;
		flex-wrap: wrap;
	}

	.sub-label {
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--ink-2);
	}

	.rule-code {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		color: var(--ink);
		background: var(--bg-soft);
		padding: 1px 6px;
		border-radius: var(--r-sm);
	}

	.scope-opt {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		font-size: 0.78125rem;
		color: var(--ink);
		cursor: pointer;
	}

	.grow {
		flex: 1;
	}

	.feedback-row {
		display: flex;
		gap: 8px;
		align-items: center;
		flex-wrap: wrap;
	}

	.feedback-input {
		flex: 1 1 240px;
		padding: 7px 10px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink);
		font-size: 0.8125rem;
	}

	.feedback-input:focus {
		outline: none;
		border-color: var(--focus);
	}
</style>
