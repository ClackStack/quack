<script lang="ts">
	import type { CheckStatus } from '#lib/protocol';
	import type { Session } from '#lib/session.svelte';

	let { session, onClose }: { session: Session; onClose: () => void } = $props();

	let dialogEl: HTMLElement | undefined = $state();

	const checks = $derived(session.doctorChecks ?? []);
	const counts = $derived.by(() => {
		let ok = 0;
		let warn = 0;
		let fail = 0;
		let info = 0;
		for (const c of checks) {
			if (c.status === 'ok') ok += 1;
			else if (c.status === 'warn') warn += 1;
			else if (c.status === 'fail') fail += 1;
			else info += 1;
		}
		return { ok, warn, fail, info };
	});

	function statusLabel(status: CheckStatus): string {
		if (status === 'ok') return 'ok';
		if (status === 'warn') return 'warn';
		if (status === 'fail') return 'fail';
		return 'info';
	}

	$effect(() => {
		dialogEl?.focus();
	});

	$effect(() => {
		if (!session.doctorChecks && !session.doctorLoading && !session.closed) {
			void session.runDoctor();
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
		aria-labelledby="doctor-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div>
				<h2 id="doctor-modal-title">Environment doctor</h2>
				<p class="sub">Config layers, defaults bundle, storage, tools, and provider readiness</p>
			</div>
			<button type="button" class="close-btn" aria-label="Close doctor report" onclick={onClose}>
				×
			</button>
		</div>

		<div class="modal-body">
			{#if checks.length > 0}
				<div class="summary-bar">
					<span class="pill pill-ok">{counts.ok} ok</span>
					{#if counts.warn > 0}
						<span class="pill pill-warn">{counts.warn} warn</span>
					{/if}
					{#if counts.fail > 0}
						<span class="pill pill-fail">{counts.fail} fail</span>
					{/if}
					{#if counts.info > 0}
						<span class="pill pill-info">{counts.info} info</span>
					{/if}
				</div>

				<div class="checks">
					{#each checks as check (check.name)}
						<div class="check-row">
							<span class={`status-pill status-${check.status}`}>{statusLabel(check.status)}</span>
							<div class="check-info">
								<div class="check-name">{check.name}</div>
								<div class="check-detail">{check.detail}</div>
							</div>
						</div>
					{/each}
				</div>
			{:else if session.doctorLoading}
				<p class="empty">Running environment checks…</p>
			{:else}
				<p class="empty">No health checks reported yet. Click “Run checks” to inspect this workspace.</p>
			{/if}
		</div>

		<div class="modal-foot">
			<button
				type="button"
				class="btn"
				disabled={session.closed || session.doctorLoading}
				onclick={() => void session.runDoctor()}
			>
				{session.doctorLoading ? 'Checking…' : 'Run checks'}
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
		width: min(600px, 100%);
		max-height: min(660px, calc(100vh - 32px));
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

	.summary-bar {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
	}

	.checks {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}

	.check-row {
		display: flex;
		align-items: flex-start;
		gap: 12px;
		padding: 10px 12px;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
	}

	.status-pill {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.06em;
		padding: 2px 7px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		flex: none;
	}

	.status-ok {
		background: var(--ok-soft);
		color: var(--ok);
	}

	.status-warn {
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.status-fail {
		background: var(--fail-soft);
		color: var(--fail);
	}

	.status-info {
		background: var(--sky-soft);
		color: var(--sky-ink);
	}

	.check-info {
		min-width: 0;
		flex: 1;
	}

	.check-name {
		font-size: 0.84375rem;
		font-weight: 600;
		color: var(--ink);
	}

	.check-detail {
		margin-top: 2px;
		font-size: 0.78125rem;
		color: var(--ink-2);
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
		margin: 18px 0;
		text-align: center;
		font-size: 0.84375rem;
		color: var(--ink-3);
	}
</style>
