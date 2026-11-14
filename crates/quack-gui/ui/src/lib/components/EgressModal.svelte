<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session, onClose }: { session: Session; onClose: () => void } = $props();

	let dialogEl: HTMLElement | undefined = $state();
	const egress = $derived(session.info?.egress);
	const sandbox = $derived(session.info?.sandbox);

	const busy = $derived(
		session.closed ||
			session.starting ||
			session.switching ||
			session.streaming ||
			session.turnPending ||
			session.settingSaving
	);

	const activeProviderId = $derived(session.currentModel?.split('/')[0] ?? null);

	$effect(() => {
		dialogEl?.focus();
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
		aria-labelledby="egress-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div>
				<h2 id="egress-modal-title">Network &amp; Sandbox Boundary</h2>
				<p class="sub">
					quack only connects to endpoints you configure. Background telemetry and update checks stay off unless enabled.
				</p>
			</div>
			<button class="close-btn" aria-label="Close boundary passport" onclick={onClose}>×</button>
		</div>

		<div class="modal-body">
			<section class="card">
				<div class="card-head">
					<div>
						<h3>Network Egress</h3>
						<p class="desc">
							{#if egress?.offline}
								<strong>Strict offline mode:</strong> only loopback servers, configured model providers, and explicit allow-list hosts can be reached.
							{:else}
								<strong>Bounded by config:</strong> only your configured model providers, remote MCP servers, and user-approved commands connect out.
							{/if}
						</p>
					</div>
					<button
						type="button"
						class="btn"
						class:btn-primary={egress?.offline}
						disabled={busy || !!egress?.offline_locked}
						title={egress?.offline_locked ? 'Locked by system policy' : 'Save network.offline and reload'}
						onclick={() => void session.saveSetting('network.offline', egress?.offline ? 'false' : 'true', 'user', true)}
					>
						{egress?.offline ? 'Offline: ON' : 'Go strict offline'}
					</button>
				</div>

				<div class="block">
					<span class="label">Configured Model Destinations</span>
					{#if egress && egress.providers.length > 0}
						<ul class="dest-list">
							{#each egress.providers as ep (ep.id)}
								<li class:active={ep.id === activeProviderId}>
									<span class="dest-id">{ep.id}</span>
									<span class="dest-host">{ep.host}</span>
									<span class="tag" class:tag-local={ep.local}>{ep.local ? 'Local / Loopback' : 'Hosted'}</span>
									{#if ep.id === activeProviderId}
										<span class="tag tag-active">In use</span>
									{:else if !ep.enabled}
										<span class="tag">Disabled</span>
									{/if}
								</li>
							{/each}
						</ul>
					{:else}
						<p class="empty">No model providers configured yet.</p>
					{/if}
				</div>

				{#if egress && egress.mcp_remote.length > 0}
					<div class="block">
						<span class="label">Remote HTTP MCP Servers</span>
						<ul class="dest-list">
							{#each egress.mcp_remote as ep (ep.id)}
								<li>
									<span class="dest-id">{ep.id}</span>
									<span class="dest-host">{ep.host}</span>
									<span class="tag" class:tag-local={ep.local}>{ep.local ? 'Local' : 'Remote'}</span>
									{#if !ep.enabled}<span class="tag">Disabled</span>{/if}
								</li>
							{/each}
						</ul>
					</div>
				{/if}

				{#if egress && egress.egress_allow.length > 0}
					<div class="block">
						<span class="label">Extra Allowed Hosts (`network.egress_allow`)</span>
						<p class="mono-line">{egress.egress_allow.join(', ')}</p>
					</div>
				{/if}

				<div class="checks-grid">
					<div class="check-item">
						<span class="check-k">Model catalog refresh</span>
						<span class="check-v" class:on={egress?.catalog_refresh}>{egress?.catalog_refresh ? 'Enabled' : 'Off'}</span>
					</div>
					<div class="check-item">
						<span class="check-k">Update check</span>
						<span class="check-v" class:on={egress?.updates_check}>{egress?.updates_check ? 'Enabled' : 'Off'}</span>
					</div>
					<div class="check-item">
						<span class="check-k">OpenTelemetry export</span>
						<span class="check-v" class:on={egress?.telemetry_enabled}>{egress?.telemetry_enabled ? 'Configured' : 'Off'}</span>
					</div>
					{#if egress?.has_proxy || egress?.http1_only}
						<div class="check-item">
							<span class="check-k">HTTP transport</span>
							<span class="check-v">
								{[egress.has_proxy ? 'Proxy set' : null, egress.http1_only ? 'HTTP/1.1 only' : null].filter(Boolean).join(' · ')}
							</span>
						</div>
					{/if}
				</div>
			</section>

			<section class="card">
				<div class="card-head">
					<div>
						<h3>Command Sandbox</h3>
						<p class="desc">
							Mode: <code>{sandbox?.mode ?? 'auto'}</code> · {session.sandboxOff ? 'Paused for this session' : (sandbox?.status_detail ?? 'Unknown')}
						</p>
					</div>
					{#if sandbox?.os_supported}
						<button
							type="button"
							class="btn"
							disabled={busy || sandbox.locked}
							title={sandbox.locked ? 'Locked by system policy' : 'Toggle sandbox for this session'}
							onclick={() => void session.setSandbox(!session.sandboxOff)}
						>
							{session.sandboxOff ? 'Enable sandbox' : 'Pause for session'}
						</button>
					{/if}
				</div>
				<div class="block">
					<span class="label">Write boundary</span>
					<p class="desc">
						{#if session.sandboxOff || !sandbox?.os_supported}
							Commands run with your normal user permissions and always ask before destructive or external work.
						{:else if sandbox?.writable.length}
							Workspace folder, system temp directories, and configured extras: <code>{sandbox.writable.join(', ')}</code>
						{:else}
							Workspace folder and system temp directories only; network sockets inside commands are blocked unless approved.
						{/if}
					</p>
				</div>
			</section>
		</div>

		<div class="modal-foot">
			<span class="foot-note">Defaults are local TOML layers; no endpoints are hardcoded in the binary.</span>
			<button class="btn" onclick={onClose}>
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
		width: min(620px, 100%);
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
		gap: 12px;
		padding: 16px 20px;
		background: var(--bg-panel);
		border-bottom: 1px solid var(--rule);
	}

	.modal-head h2 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 18px;
		color: var(--ink);
	}

	.sub {
		margin: 4px 0 0;
		font-size: 12.5px;
		color: var(--ink-2);
	}

	.close-btn {
		font-size: 20px;
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
		padding: 16px 20px;
		overflow: auto;
		display: flex;
		flex-direction: column;
		gap: 14px;
	}

	.card {
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		padding: 14px 16px;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.card-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 12px;
		flex-wrap: wrap;
	}

	.card-head h3 {
		margin: 0;
		font-size: 14px;
		font-weight: 700;
		color: var(--ink);
	}

	.desc {
		margin: 3px 0 0;
		font-size: 12.5px;
		color: var(--ink-2);
		line-height: 1.45;
	}

	.block {
		display: flex;
		flex-direction: column;
		gap: 5px;
	}

	.label {
		font-size: 10.5px;
		font-weight: 700;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--ink-3);
	}

	.dest-list {
		margin: 0;
		padding: 0;
		list-style: none;
		display: flex;
		flex-direction: column;
		gap: 5px;
	}

	.dest-list li {
		display: flex;
		align-items: center;
		gap: 8px;
		padding: 5px 8px;
		border-radius: var(--r-md);
		background: var(--bg-raised);
		border: 1px solid var(--rule-soft);
		font-size: 12.5px;
		flex-wrap: wrap;
	}

	.dest-list li.active {
		border-color: var(--sky);
	}

	.dest-id {
		font-weight: 600;
		color: var(--ink);
	}

	.dest-host {
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--ink-2);
		margin-right: auto;
	}

	.tag {
		font-size: 10.5px;
		font-weight: 600;
		padding: 1px 7px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		color: var(--ink-3);
	}

	.tag-local {
		color: var(--ok);
		border-color: color-mix(in srgb, var(--ok) 45%, var(--rule));
		background: var(--ok-soft);
	}

	.tag-active {
		color: var(--sky-ink);
		border-color: var(--sky);
		background: var(--sky-soft);
	}

	.mono-line {
		margin: 0;
		font-family: var(--font-mono);
		font-size: 12px;
		color: var(--ink);
	}

	.checks-grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
		gap: 8px;
		padding-top: 4px;
		border-top: 1px solid var(--rule-soft);
	}

	.check-item {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		font-size: 12px;
	}

	.check-k {
		color: var(--ink-2);
	}

	.check-v {
		font-family: var(--font-mono);
		font-size: 11.5px;
		color: var(--ok);
	}

	.check-v.on {
		color: var(--beak-ink);
	}

	.empty {
		margin: 0;
		font-size: 12.5px;
		color: var(--ink-3);
	}

	.modal-foot {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 12px 20px;
		background: var(--bg-panel);
		border-top: 1px solid var(--rule);
	}

	.foot-note {
		font-size: 11.5px;
		color: var(--ink-3);
	}
</style>
