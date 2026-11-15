<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session, onClose }: { session: Session; onClose: () => void } = $props();

	let dialogEl: HTMLElement | undefined = $state();
	let severityFilter = $state<'all' | 'error' | 'warning'>('all');

	const diagnostics = $derived(session.lspDiagnostics ?? []);
	const filtered = $derived.by(() => {
		if (severityFilter === 'all') return diagnostics;
		return diagnostics.filter((d) => d.severity.toLowerCase() === severityFilter);
	});
	const errorCount = $derived(
		diagnostics.filter((d) => d.severity.toLowerCase() === 'error').length
	);
	const warningCount = $derived(
		diagnostics.filter((d) => d.severity.toLowerCase() === 'warning').length
	);

	$effect(() => {
		dialogEl?.focus();
	});

	$effect(() => {
		if (session.lspDiagnostics === null && !session.lspDiagnosticsLoading && !session.closed) {
			void session.refreshLsp();
			void session.refreshLspDiagnostics();
		}
	});

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			event.preventDefault();
			onClose();
		}
	}
</script>

<svelte:window onkeydown={onKeydown} />

<div
	class="backdrop"
	role="presentation"
	onclick={(e) => {
		if (e.target === e.currentTarget) onClose();
	}}
>
	<div
		class="modal"
		role="dialog"
		aria-modal="true"
		aria-labelledby="diagnostics-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div>
				<h2 id="diagnostics-modal-title">LSP diagnostics</h2>
				<p class="sub">
					{session.diagnostics.errors} errors · {session.diagnostics.warnings} warnings across connected language servers
				</p>
			</div>
			<button
				type="button"
				class="close-btn"
				aria-label="Close LSP diagnostics"
				onclick={onClose}
			>
				×
			</button>
		</div>

		<div class="modal-body">
			{#if session.lspServers.length > 0}
				<div class="servers">
					{#each session.lspServers as srv (srv.id)}
						<div class="server-chip" title={srv.detail || srv.command}>
							<span class="srv-id">{srv.id}</span>
							<span class={`srv-state srv-${srv.state}`}>{srv.state}</span>
							{#if srv.errors > 0 || srv.warnings > 0}
								<span class="srv-counts">{srv.errors}e/{srv.warnings}w</span>
							{/if}
						</div>
					{/each}
				</div>
			{/if}

			<div class="filter-bar" role="tablist" aria-label="Filter diagnostics by severity">
				<button
					type="button"
					class="tab-btn"
					class:active={severityFilter === 'all'}
					onclick={() => (severityFilter = 'all')}
				>
					All ({diagnostics.length})
				</button>
				<button
					type="button"
					class="tab-btn"
					class:active={severityFilter === 'error'}
					onclick={() => (severityFilter = 'error')}
				>
					Errors ({errorCount})
				</button>
				<button
					type="button"
					class="tab-btn"
					class:active={severityFilter === 'warning'}
					onclick={() => (severityFilter = 'warning')}
				>
					Warnings ({warningCount})
				</button>
			</div>

			{#if filtered.length > 0}
				<div class="diag-list">
					{#each filtered as item, idx (`${item.server}:${item.path}:${item.line}:${item.column}:${idx}`)}
						<div class="diag-row">
							<div class="diag-top">
								<button
									type="button"
									class="diag-path"
									title="Reveal file in editor / file manager"
									onclick={() => void session.revealFile(item.path, true)}
								>
									{item.path}:{item.line}:{item.column}
								</button>
								<div class="diag-badges">
									{#if item.code}
										<span class="code-badge">{item.code}</span>
									{/if}
									<span class="srv-badge">{item.server}</span>
									<span class={`sev-badge sev-${item.severity.toLowerCase()}`}>
										{item.severity}
									</span>
								</div>
							</div>
							<p class="diag-msg">{item.message}</p>
						</div>
					{/each}
				</div>
			{:else if session.lspDiagnosticsLoading}
				<p class="empty">Loading language server diagnostics…</p>
			{:else}
				<p class="empty">
					{diagnostics.length === 0
						? 'Workspace is clean — no active diagnostics reported.'
						: `No ${severityFilter} diagnostics reported.`}
				</p>
			{/if}
		</div>

		<div class="modal-foot">
			<button
				type="button"
				class="btn"
				disabled={session.closed || session.lspDiagnosticsLoading}
				onclick={() => {
					void session.refreshLsp();
					void session.refreshLspDiagnostics();
				}}
			>
				{session.lspDiagnosticsLoading ? 'Refreshing…' : 'Refresh'}
			</button>
			<div class="grow"></div>
			<button type="button" class="btn" onclick={onClose}>
				Close <span class="kbd">esc</span>
			</button>
		</div>
	</div>
</div>

<style>
	.backdrop {
		position: fixed;
		inset: 0;
		z-index: 60;
		background: var(--scrim);
		display: grid;
		place-items: center;
		padding: var(--sp-md);
	}

	.modal {
		width: min(680px, 100%);
		max-height: min(680px, calc(100vh - 32px));
		display: flex;
		flex-direction: column;
		background: var(--bg-raised);
		border: 1px solid var(--plate-line);
		border-radius: var(--r-xl);
		box-shadow: var(--shadow-pop);
		overflow: hidden;
		outline: none;
	}

	.modal-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--sp-md);
		padding: 16px 20px;
		background: var(--bg-panel);
		border-bottom: 1px solid var(--rule);
	}

	.modal-head h2 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 1.125rem;
		color: var(--ink);
	}

	.sub {
		margin: 4px 0 0;
		font-size: 0.78125rem;
		color: var(--ink-2);
	}

	.close-btn {
		font-size: 1.25rem;
		line-height: 1;
		padding: 2px 8px;
		border-radius: var(--r-md);
		color: var(--ink-2);
	}

	.close-btn:hover {
		background: var(--bg-soft);
		color: var(--ink);
	}

	.modal-body {
		padding: var(--sp-md) 20px;
		overflow-y: auto;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.servers {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}

	.server-chip {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		padding: 4px 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		font-size: 0.75rem;
	}

	.srv-id {
		font-family: var(--font-mono);
		font-weight: 600;
		color: var(--ink);
	}

	.srv-state {
		color: var(--ink-2);
	}

	.srv-state.srv-ready {
		color: var(--ok);
	}

	.srv-state.srv-failed {
		color: var(--fail);
	}

	.srv-counts {
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		color: var(--ink-3);
	}

	.filter-bar {
		display: flex;
		gap: 6px;
	}

	.tab-btn {
		padding: 5px 10px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-panel);
		font-size: 0.75rem;
		color: var(--ink-2);
	}

	.tab-btn.active {
		background: var(--duck-soft);
		color: var(--duck-ink);
		font-weight: 600;
	}

	.diag-list {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.diag-row {
		padding: 10px 12px;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
	}

	.diag-top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
		flex-wrap: wrap;
	}

	.diag-path {
		font-family: var(--font-mono);
		font-size: 0.78125rem;
		font-weight: 600;
		color: var(--sky-ink);
		text-align: left;
		overflow-wrap: anywhere;
	}

	.diag-path:hover {
		text-decoration: underline;
	}

	.diag-badges {
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}

	.code-badge,
	.srv-badge,
	.sev-badge {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		font-weight: 600;
		padding: 1px 6px;
		border-radius: 999px;
		border: 1px solid var(--rule);
	}

	.code-badge,
	.srv-badge {
		background: var(--bg-raised);
		color: var(--ink-2);
	}

	.sev-error {
		background: var(--fail-soft);
		color: var(--fail);
	}

	.sev-warning {
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.sev-info,
	.sev-hint {
		background: var(--sky-soft);
		color: var(--sky-ink);
	}

	.diag-msg {
		margin: 6px 0 0;
		font-size: 0.8125rem;
		color: var(--ink);
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.modal-foot {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 12px 20px;
		background: var(--bg-panel);
		border-top: 1px solid var(--rule-soft);
	}

	.grow {
		flex: 1;
	}

	.empty {
		margin: 20px 0;
		text-align: center;
		font-size: 0.84375rem;
		color: var(--ink-3);
	}
</style>
