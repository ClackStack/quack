<script lang="ts">
	import { formatTomlStringArray } from '#lib/settings';
	import { isHttpUrl, type Session } from '#lib/session.svelte';

	let {
		session,
		onClose,
		onOpenSettings
	}: { session: Session; onClose: () => void; onOpenSettings?: () => void } = $props();

	let dialogEl: HTMLElement | undefined = $state();
	let activeTab = $state<'policy' | 'ledger'>('policy');
	const egress = $derived(session.info?.egress);
	const sandbox = $derived(session.info?.sandbox);
	const shieldMode = $derived(egress?.secret_guard ?? 'ask');
	const shieldLocal = $derived(egress?.secret_guard_local ?? false);
	const shieldAllowlist = $derived(egress?.secret_allowlist ?? []);
	const entries = $derived([...session.egressEntries].reverse());

	const busy = $derived(
		session.closed ||
			session.starting ||
			session.switching ||
			session.streaming ||
			session.turnPending ||
			session.settingSaving
	);

	const activeProviderId = $derived(session.currentModel?.split('/')[0] ?? null);

	function formatBytes(bytes: number): string {
		if (bytes < 1024) return `${bytes} B`;
		if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
		return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
	}

	function formatTime(ms: number): string {
		try {
			return new Date(ms).toLocaleTimeString([], {
				hour: '2-digit',
				minute: '2-digit',
				second: '2-digit'
			});
		} catch {
			return '';
		}
	}

	async function revokeAllowlistEntry(fingerprint: string) {
		const next = shieldAllowlist.filter((fp) => fp !== fingerprint);
		await session.saveSetting('network.secret_allowlist', formatTomlStringArray(next), 'user', true);
	}

	$effect(() => {
		dialogEl?.focus();
	});

	$effect(() => {
		if (activeTab === 'ledger' && !session.closed) {
			void session.refreshEgress();
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
		aria-labelledby="egress-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div>
				<h2 id="egress-modal-title">Network &amp; Sandbox Boundary</h2>
				<p class="sub">
					quack only connects to endpoints you configure, scans outbound payloads for secrets, and records every network hop in this session ledger.
				</p>
			</div>
			<button type="button" class="close-btn" aria-label="Close boundary passport" onclick={onClose}>×</button>
		</div>

		<div class="modal-tabs" role="tablist" aria-label="Boundary sections">
			<button
				type="button"
				role="tab"
				aria-selected={activeTab === 'policy'}
				class="tab-btn"
				class:active={activeTab === 'policy'}
				onclick={() => (activeTab = 'policy')}
			>
				Policy &amp; Shield
			</button>
			<button
				type="button"
				role="tab"
				aria-selected={activeTab === 'ledger'}
				class="tab-btn"
				class:active={activeTab === 'ledger'}
				onclick={() => (activeTab = 'ledger')}
			>
				Session Ledger
				{#if session.egressEntries.length > 0}
					<span class="tab-count">{session.egressEntries.length}</span>
				{/if}
			</button>
		</div>

		<div class="modal-body">
			{#if activeTab === 'policy'}
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

				<div class="update-row">
					<div class="update-row-info">
						<span class="label">Version {session.info?.version ?? 'unknown'}</span>
						{#if session.updateChecking}
							<span class="update-row-status">Checking for updates…</span>
						{:else if session.updateState?.error}
							<span class="update-row-status fail">{session.updateState.error}</span>
						{:else if session.updateState?.updateAvailable}
							<span class="update-row-status info">
								Update available: <strong>quack {session.updateState.latest}</strong>
							</span>
						{:else if session.updateState}
							<span class="update-row-status ok">
								{session.updateState.latest
									? `Up to date (${session.updateState.current})`
									: 'No release published yet'}
							</span>
						{:else}
							<span class="update-row-status">
								{egress?.updates_check ? 'Auto-checks daily' : 'Auto-check is off'}
							</span>
						{/if}
					</div>
					<div class="update-row-actions">
						{#if session.updateState?.updateAvailable && isHttpUrl(session.updateState.url)}
							<button
								type="button"
								class="btn"
								onclick={() => void session.openReleasePage()}
							>
								Open release
							</button>
						{/if}
						<button
							type="button"
							class="btn"
							disabled={session.closed || session.starting || session.updateChecking}
							onclick={() => void session.checkForUpdates()}
						>
							{session.updateChecking ? 'Checking…' : 'Check for updates'}
						</button>
					</div>
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

			<section class="card">
				<div class="card-head">
					<div>
						<h3>Pre-Egress Secret Shield</h3>
						<p class="desc">
							Scans prompts and tool outputs locally with <code>redact-secret</code> + resolved provider credentials before an outbound HTTP request leaves this machine.
						</p>
					</div>
				</div>

				<div class="block">
					<span class="label">Shield policy (<code>network.secret_guard</code>)</span>
					<div class="seg-row" role="group" aria-label="Secret Shield mode">
						{#each [['ask', 'Ask'], ['redact', 'Auto-redact'], ['block', 'Block'], ['off', 'Off']] as [mode, label] (mode)}
							<button
								type="button"
								class="seg-btn"
								class:active={shieldMode === mode}
								disabled={busy}
								onclick={() =>
									void session.saveSetting(
										'network.secret_guard',
										JSON.stringify(mode),
										'user',
										true
									)}
							>
								{label}
							</button>
						{/each}
					</div>
				</div>

				<div class="block inline-row">
					<div>
						<span class="label">Scan loopback destinations (<code>network.secret_guard_local</code>)</span>
						<p class="desc">
							{shieldLocal
								? 'Loopback servers (localhost / 127.0.0.1) are also scanned before egress.'
								: 'Loopback servers skip secret prompts so local weights stay fast.'}
						</p>
					</div>
					<button
						type="button"
						class="btn"
						disabled={busy}
						onclick={() =>
							void session.saveSetting(
								'network.secret_guard_local',
								shieldLocal ? 'false' : 'true',
								'user',
								true
							)}
					>
						{shieldLocal ? 'Scanning local' : 'Local bypass'}
					</button>
				</div>

				<div class="block">
					<span class="label">Allowlisted Secret Fingerprints ({shieldAllowlist.length})</span>
					{#if shieldAllowlist.length > 0}
						<ul class="dest-list">
							{#each shieldAllowlist as fp (fp)}
								<li>
									<span class="dest-host">{fp}</span>
									<button
										type="button"
										class="revoke-btn"
										disabled={busy}
										onclick={() => void revokeAllowlistEntry(fp)}
									>
										Revoke
									</button>
								</li>
							{/each}
						</ul>
					{:else}
						<p class="empty">No allowlisted secrets saved yet.</p>
					{/if}
				</div>
			</section>
			{:else}
				<div class="ledger-head">
					<div class="stats-grid">
						<div class="stat-tile">
							<span class="stat-k">Requests</span>
							<strong class="stat-v">{session.egressStats?.total_requests ?? entries.length}</strong>
						</div>
						<div class="stat-tile">
							<span class="stat-k">Sent</span>
							<strong class="stat-v">{formatBytes(session.egressStats?.total_bytes_sent ?? 0)}</strong>
						</div>
						<div class="stat-tile">
							<span class="stat-k">Received</span>
							<strong class="stat-v">{formatBytes(session.egressStats?.total_bytes_received ?? 0)}</strong>
						</div>
						<div class="stat-tile">
							<span class="stat-k">Redacted</span>
							<strong class="stat-v">{session.egressStats?.redacted_count ?? 0}</strong>
						</div>
						<div class="stat-tile">
							<span class="stat-k">Blocked</span>
							<strong class="stat-v">{session.egressStats?.blocked_count ?? 0}</strong>
						</div>
					</div>
					<button
						type="button"
						class="btn"
						disabled={session.closed || session.egressLoading}
						onclick={() => void session.refreshEgress()}
					>
						{session.egressLoading ? 'Refreshing…' : 'Refresh ledger'}
					</button>
				</div>

				{#if entries.length > 0}
					<div class="ledger-list">
						{#each entries as entry (entry.seq)}
							<div class="ledger-row" class:blocked={Boolean(entry.blocked_reason)}>
								<div class="ledger-top">
									<span class="ledger-seq">#{entry.seq}</span>
									<span class="tag">{entry.kind}</span>
									<span class="ledger-dest">{entry.host}{entry.path}</span>
									<span class="tag" class:tag-local={entry.local}>{entry.local ? 'local' : 'remote'}</span>
								</div>
								<div class="ledger-meta">
									<span>{formatTime(entry.timestamp_ms)}</span>
									<span>{entry.protocol}</span>
									<span>↑ {formatBytes(entry.bytes_sent)} · ↓ {formatBytes(entry.bytes_received)}</span>
									{#if entry.status !== undefined && entry.status !== null}
										<span>HTTP {entry.status}</span>
									{/if}
									{#if entry.redactions > 0}
										<span class="badge-warn">{entry.redactions} redacted</span>
									{/if}
									{#if entry.blocked_reason}
										<span class="badge-fail">{entry.blocked_reason}</span>
									{/if}
								</div>
							</div>
						{/each}
					</div>
				{:else if session.egressLoading}
					<p class="empty">Reading session egress ledger…</p>
				{:else}
					<p class="empty">
						No outbound HTTP requests recorded in this session yet — zero implicit traffic.
					</p>
				{/if}
			{/if}
		</div>

		<div class="modal-foot">
			<span class="foot-note">Defaults are local TOML layers; no endpoints are hardcoded in the binary.</span>
			<div class="foot-actions">
				{#if onOpenSettings}
					<button type="button" class="btn" onclick={onOpenSettings}>
						Edit in Settings
					</button>
				{/if}
				<button class="btn" onclick={onClose}>
					Close <span class="kbd">esc</span>
				</button>
			</div>
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
		font-size: 0.875rem;
		font-weight: 700;
		color: var(--ink);
	}

	.desc {
		margin: 3px 0 0;
		font-size: 0.78125rem;
		color: var(--ink-2);
		line-height: 1.45;
	}

	.block {
		display: flex;
		flex-direction: column;
		gap: 5px;
	}

	.label {
		font-size: 0.65625rem;
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
		font-size: 0.78125rem;
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
		font-size: 0.71875rem;
		color: var(--ink-2);
		margin-right: auto;
	}

	.tag {
		font-size: 0.65625rem;
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
		font-size: 0.75rem;
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
		font-size: 0.75rem;
	}

	.check-k {
		color: var(--ink-2);
	}

	.check-v {
		font-family: var(--font-mono);
		font-size: 0.71875rem;
		color: var(--ok);
	}

	.check-v.on {
		color: var(--beak-ink);
	}

	.empty {
		margin: 0;
		font-size: 0.78125rem;
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
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.foot-actions {
		display: flex;
		align-items: center;
		gap: 8px;
		flex: none;
	}

	.update-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
		flex-wrap: wrap;
		padding-top: 8px;
		border-top: 1px solid var(--rule-soft);
	}

	.update-row-info {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 0;
		flex: 1 1 200px;
	}

	.update-row-status {
		font-size: 0.78125rem;
		color: var(--ink-2);
		overflow-wrap: anywhere;
	}

	.update-row-status.ok {
		color: var(--ok);
	}

	.update-row-status.info {
		color: var(--sky-ink);
	}

	.update-row-status.fail {
		color: var(--fail);
	}

	.update-row-actions {
		display: flex;
		align-items: center;
		gap: 6px;
		flex: none;
	}

	.modal-tabs {
		display: flex;
		gap: 6px;
		padding: 10px 20px 0;
		background: var(--bg-panel);
		border-bottom: 1px solid var(--rule);
	}

	.tab-btn {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		padding: 7px 12px;
		font-size: 0.78125rem;
		font-weight: 500;
		color: var(--ink-2);
		border-bottom: 2px solid transparent;
	}

	.tab-btn.active {
		color: var(--ink);
		font-weight: 600;
		border-bottom-color: var(--duck);
	}

	.tab-count {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		padding: 1px 6px;
		border-radius: 999px;
		background: var(--bg-soft);
		color: var(--ink-2);
	}

	.seg-row {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.seg-btn {
		padding: 5px 10px;
		font-size: 0.75rem;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-panel);
		color: var(--ink-2);
	}

	.seg-btn.active {
		background: var(--duck-soft);
		color: var(--duck-ink);
		border-color: var(--duck);
		font-weight: 600;
	}

	.inline-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
	}

	.revoke-btn {
		font-size: 0.71875rem;
		padding: 2px 8px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		color: var(--fail);
		background: var(--bg-raised);
	}

	.ledger-head {
		display: flex;
		flex-wrap: wrap;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
	}

	.stats-grid {
		display: flex;
		flex-wrap: wrap;
		gap: 8px;
		flex: 1;
	}

	.stat-tile {
		display: flex;
		flex-direction: column;
		gap: 1px;
		padding: 6px 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		background: var(--bg-panel);
		min-width: 72px;
	}

	.stat-k {
		font-size: 0.65625rem;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--ink-3);
	}

	.stat-v {
		font-family: var(--font-mono);
		font-size: 0.8125rem;
		color: var(--ink);
	}

	.ledger-list {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}

	.ledger-row {
		padding: 8px 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
		display: flex;
		flex-direction: column;
		gap: 4px;
	}

	.ledger-row.blocked {
		border-color: color-mix(in srgb, var(--fail) 45%, var(--rule));
		background: color-mix(in srgb, var(--fail-soft) 35%, var(--bg-panel));
	}

	.ledger-top {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}

	.ledger-seq {
		font-family: var(--font-mono);
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.ledger-dest {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		color: var(--ink);
		overflow-wrap: anywhere;
		flex: 1;
		min-width: 0;
	}

	.ledger-meta {
		display: flex;
		flex-wrap: wrap;
		gap: 10px;
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		color: var(--ink-3);
	}

	.badge-warn {
		color: var(--duck-ink);
		font-weight: 600;
	}

	.badge-fail {
		color: var(--fail);
		font-weight: 600;
	}
</style>
