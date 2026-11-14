<script lang="ts">
	import type { Session } from '#lib/session.svelte';
	import { partitionExpertProviders, splitInlineCode } from '#lib/setup';

	let { session }: { session: Session } = $props();
	let serviceId = $state('');
	let serviceKey = $state('');
	let baseUrl = $state('');
	let serverKey = $state('');
	const available = $derived(session.providers.filter((p) => p.available && p.models.length > 0));
	const busy = $derived(session.setupWorking || session.closed || session.starting);
	const service = $derived(session.services.find((s) => s.id === serviceId) ?? null);
	const signingIn = $derived(session.signingIn !== null && session.signingIn.id === serviceId);
	const signInError = $derived(
		session.signInError !== null && session.signInError.id === serviceId
			? session.signInError.message
			: null
	);
	const expert = $derived(partitionExpertProviders(session.providers, session.services));
	const hasExpert = $derived(
		expert.envProviders.length > 0 || expert.otherUnavailable.length > 0
	);
	const expertSummary = $derived.by(() => {
		const parts: string[] = [];
		if (expert.envProviders.length > 0) {
			const n = expert.envProviders.length;
			parts.push(`${n} environment ${n === 1 ? 'variable' : 'variables'}`);
		}
		if (expert.otherUnavailable.length > 0) {
			parts.push(`${expert.otherUnavailable.length} other unavailable`);
		}
		return parts.join(' · ');
	});

	async function saveKey(event: SubmitEvent) {
		event.preventDefault();
		if (await session.saveProviderKey(serviceId, serviceKey)) serviceKey = '';
	}
	async function addServer(event: SubmitEvent) {
		event.preventDefault();
		if (await session.connectServer(baseUrl, serverKey)) serverKey = '';
	}
</script>

<section class="setup" aria-labelledby="setup-heading">
	<div class="heading">
		<div>
			<h1 id="setup-heading">Choose a model</h1>
			<p>Connect a local server or a hosted provider to start working in this folder.</p>
		</div>
		{#if session.currentModel}<button class="btn" onclick={() => (session.setupOpen = false)}
				>Back to conversation</button
			>{/if}
	</div>
	{#if session.setupNotice}<p class="notice" role="status">{session.setupNotice}</p>{/if}
	{#if session.setupWorking}<p role="status">Connecting…</p>{/if}

	{#if available.length > 0}
		<section aria-labelledby="ready-heading">
			<h2 id="ready-heading">Available models</h2>
			{#each available as provider (provider.id)}
				<div class="provider">
					<h3>{provider.id}</h3>
					{#each provider.models as model (model.id)}
						<button
							class="model"
							disabled={busy}
							onclick={() => session.setModel(`${provider.id}/${model.id}`)}
						>
							<span>{model.id}</span
							><span class="note">{[
									model.price,
									model.reasoning_efforts?.length
										? `thinking: ${model.reasoning_efforts.join(' · ')}`
										: null
									].filter(Boolean).join(' · ') || 'Select model'}</span
							>
						</button>
					{/each}
				</div>
			{/each}
		</section>
	{/if}

	<section aria-labelledby="local-heading">
		<div class="heading">
			<h2 id="local-heading">Local server</h2>
			<button
				class="btn"
				disabled={busy || session.detecting}
				onclick={() => session.detectServers()}
			>
				{session.detecting ? 'Looking for servers…' : 'Find local servers'}
			</button>
		</div>
		<p>Looks for model servers on this computer. No model requests are sent.</p>
		{#if session.detectedServers?.length === 0}<p role="status">
				No servers found. Start your model server, then try again, or enter its URL below.
			</p>{/if}
		{#each session.detectedServers ?? [] as server (server.base_url)}
			<div class="provider">
				<h3>{server.name}</h3>
				<p class="path">{server.base_url}</p>
				{#if server.note}<p>{server.note}</p>{/if}
				{#each server.models as model (model.id)}
					<button
						class="model"
						disabled={busy}
						onclick={() => session.useDetectedServer(server, model.id)}>Use {model.id}</button
					>
				{:else}
					<p>No models loaded. Load a model in the server, then scan again.</p>
				{/each}
			</div>
		{/each}
	</section>

	<section aria-labelledby="hosted-heading">
		<h2 id="hosted-heading">Hosted provider</h2>
		{#if session.services.length > 0}
			<label
				>Provider<select bind:value={serviceId} disabled={busy} required>
					<option value="" disabled>Choose a provider</option>
					{#each session.services as service (service.id)}<option value={service.id}
						>{service.name}{service.ready
							? service.subscription
								? ' · signed in'
								: ' · connected'
							: ''}</option
					>{/each}
				</select></label
			>
			{#if service?.subscription}
				<p>
					{service.name} uses your browser to sign in; no API key is needed. Your saved sign-in
					allows plan usage.
				</p>
				{#if signInError}
					<p class="status" role="alert">Sign-in failed: {signInError}</p>
				{/if}
				{#if signingIn}
					<p role="status">Waiting for the sign-in to finish in your browser…</p>
					{#if session.signingIn?.url}
						<button class="btn" onclick={() => session.reopenSignIn()}
							>Open the sign-in page again</button
						>
					{/if}
					<button class="btn" disabled={busy} onclick={() => session.signIn(null)}
						>Cancel sign-in</button
					>
				{:else}
					<button class="btn" disabled={busy || !serviceId} onclick={() => session.signIn(serviceId)}
						>{service.ready ? 'Sign in again' : `Sign in to ${service.name}`}</button
					>
				{/if}
			{:else}
				<form onsubmit={saveKey}>
					<label
						>API key<input
							type="password"
							bind:value={serviceKey}
							disabled={busy}
							required
							autocomplete="off"
							spellcheck="false"
						/></label
					>
					<p>
						Keys are stored in your OS credential store, or a private key file when the store is
						unavailable.
					</p>
					<button class="btn" type="submit" disabled={busy || !serviceId || !serviceKey.trim()}
						>Save key and list models</button
					>
				</form>
			{/if}
		{:else}
			<p>No hosted providers are configured. Add an OpenAI-compatible endpoint below.</p>
		{/if}
	</section>

	<section aria-labelledby="custom-heading">
		<h2 id="custom-heading">Connect by URL</h2>
		<p>Use the base URL of an OpenAI-compatible server, including its API prefix.</p>
		<form onsubmit={addServer}>
			<label
				>Server URL<input
					type="url"
					bind:value={baseUrl}
					disabled={busy}
					required
					placeholder="http://localhost:8000/v1"
				/></label
			>
			<label
				>API key (optional)<input
					type="password"
					bind:value={serverKey}
					disabled={busy}
					autocomplete="off"
					spellcheck="false"
				/></label
			>
			<button class="btn" type="submit" disabled={busy || !baseUrl.trim()}
				>Connect and list models</button
			>
		</form>
	</section>

	{#if hasExpert}
		<section class="expert" aria-labelledby="expert-heading">
			<details class="accordion">
				<summary>
					<span class="accordion-head">
						<span id="expert-heading" class="accordion-title"
							>Expert Setup: Environment Variables</span
						>
						{#if expertSummary}
							<span class="accordion-sub">{expertSummary}</span>
						{/if}
					</span>
					<span class="accordion-chevron" aria-hidden="true">
						<svg width="14" height="14" viewBox="0 0 16 16" fill="none">
							<path
								d="M6 3.5 10.5 8 6 12.5"
								stroke="currentColor"
								stroke-width="1.6"
								stroke-linecap="round"
								stroke-linejoin="round"
							/>
						</svg>
					</span>
				</summary>
				<div class="accordion-body">
					{#if expert.envProviders.length > 0}
						<p class="expert-intro">
							Export these variables in your shell before launching quack instead of saving API
							keys above.
						</p>
						<ul class="expert-list">
							{#each expert.envProviders as item (item.id)}
								<li class="expert-row">
									<div class="expert-main">
										<span class="expert-provider">
											{#if item.name}
												<span class="provider-name">{item.name}</span>
												<span class="provider-id">{item.id}</span>
											{:else}
												<span class="provider-id solo">{item.id}</span>
											{/if}
										</span>
										<span class="expert-meta">
											<code class="env-var">{item.envVar}</code>
											{#if item.available}
												<span class="state-badge ready">Set</span>
											{:else if item.extraNote}
												<span class="state-badge warn">Unavailable</span>
											{:else}
												<span class="state-badge">Not set</span>
											{/if}
										</span>
									</div>
									{#if item.extraNote}
										<p class="expert-note">
											{#each splitInlineCode(item.extraNote) as seg, i (i)}{#if seg.code}<code>{seg.text}</code>{:else}{seg.text}{/if}{/each}
										</p>
									{/if}
								</li>
							{/each}
						</ul>
					{/if}

					{#if expert.otherUnavailable.length > 0}
						<div class="other-group">
							<h3 class="subhead">Other unavailable providers</h3>
							<ul class="expert-list">
								{#each expert.otherUnavailable as item (item.id)}
									<li class="expert-row">
										<div class="expert-main">
											<span class="expert-provider">
												{#if item.name}
													<span class="provider-name">{item.name}</span>
													<span class="provider-id">{item.id}</span>
												{:else}
													<span class="provider-id solo">{item.id}</span>
												{/if}
											</span>
											<span class="state-badge">Unavailable</span>
										</div>
										<p class="expert-note">
											{#each splitInlineCode(item.note) as seg, i (i)}{#if seg.code}<code>{seg.text}</code>{:else}{seg.text}{/if}{/each}
										</p>
									</li>
								{/each}
							</ul>
						</div>
					{/if}
				</div>
			</details>
		</section>
	{/if}
</section>

<style>
	.setup {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: var(--sp-lg);
	}
	.setup > section {
		max-width: 720px;
		padding: var(--sp-lg) 0;
		border-bottom: 1px solid var(--rule);
	}
	.setup > section:last-child {
		border-bottom: none;
	}
	.heading {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: var(--sp-md);
	}
	h1 {
		font-family: var(--font-serif);
		font-size: 24px;
		font-weight: 600;
		margin: 0 0 8px;
	}
	h2 {
		font-family: var(--font-serif);
		font-size: 17px;
		font-weight: 600;
		margin: 0 0 8px;
	}
	h3 {
		font-size: 14px;
		margin: 0 0 4px;
	}
	p {
		color: var(--ink-2);
		max-width: 70ch;
		margin: 8px 0;
		overflow-wrap: anywhere;
	}
	form {
		display: grid;
		gap: var(--sp-sm);
		max-width: 480px;
	}
	label {
		display: grid;
		gap: 4px;
	}
	input,
	select {
		width: 100%;
		padding: 8px;
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		background: var(--bg-panel);
		color: var(--ink);
	}
	form .btn {
		justify-self: start;
	}
	.provider {
		margin-top: var(--sp-md);
	}
	.model {
		display: flex;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 4px 16px;
		padding: 10px 8px;
		width: 100%;
		text-align: left;
		border-radius: var(--r-sm);
		overflow-wrap: anywhere;
	}
	.model:hover:not(:disabled) {
		background: var(--bg-soft);
	}
	.note {
		color: var(--ink-2);
		font-size: 12px;
	}
	.path {
		font-family: var(--font-mono);
	}
	.notice {
		color: var(--ok);
	}
	.status {
		font-size: 12px;
	}
	.accordion {
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
	}
	.accordion summary {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--sp-md);
		padding: 12px 14px;
		cursor: pointer;
		list-style: none;
		user-select: none;
		border-radius: calc(var(--r-lg) - 1px);
		transition: background-color 120ms ease-out;
	}
	.accordion summary::-webkit-details-marker {
		display: none;
	}
	.accordion summary:hover {
		background: var(--bg-soft);
	}
	.accordion[open] summary {
		border-bottom-left-radius: 0;
		border-bottom-right-radius: 0;
		border-bottom: 1px solid var(--rule-soft);
	}
	.accordion-head {
		display: grid;
		gap: 2px;
		min-width: 0;
	}
	.accordion-title {
		font-family: var(--font-serif);
		font-size: 15.5px;
		font-weight: 600;
		color: var(--ink);
	}
	.accordion-sub {
		font-size: 12px;
		color: var(--ink-3);
	}
	.accordion-chevron {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		width: 20px;
		height: 20px;
		color: var(--ink-3);
		flex: none;
		transition:
			transform 150ms ease-out,
			color 120ms ease-out;
	}
	.accordion-chevron svg {
		width: 14px;
		height: 14px;
	}
	.accordion summary:hover .accordion-chevron {
		color: var(--ink);
	}
	.accordion[open] .accordion-chevron {
		transform: rotate(90deg);
	}
	.accordion-body {
		padding: 14px;
		display: grid;
		gap: var(--sp-md);
	}
	.expert-intro {
		margin: 0;
		font-size: 12.5px;
		color: var(--ink-2);
	}
	.other-group {
		display: grid;
		gap: 8px;
	}
	.subhead {
		margin: 0;
		font-family: var(--font-ui);
		font-size: 11px;
		font-weight: 600;
		letter-spacing: 0.1em;
		text-transform: uppercase;
		color: var(--ink-3);
	}
	.expert-list {
		list-style: none;
		margin: 0;
		padding: 0;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-md);
		background: var(--bg-raised);
	}
	.expert-row {
		padding: 9px 12px;
		display: grid;
		gap: 4px;
	}
	.expert-row + .expert-row {
		border-top: 1px solid var(--rule-soft);
	}
	.expert-main {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 6px 12px;
	}
	.expert-provider {
		display: inline-flex;
		align-items: baseline;
		flex-wrap: wrap;
		gap: 2px 8px;
		min-width: 0;
	}
	.provider-name {
		font-family: var(--font-serif);
		font-size: 14px;
		font-weight: 600;
		color: var(--ink);
	}
	.provider-id {
		font-family: var(--font-mono);
		font-size: 12px;
		color: var(--ink-3);
		overflow-wrap: anywhere;
	}
	.provider-id.solo {
		font-size: 13px;
		font-weight: 500;
		color: var(--ink);
	}
	.expert-meta {
		display: inline-flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
	}
	.env-var {
		font-family: var(--font-mono);
		font-size: 12px;
		color: var(--ink);
		background: var(--bg-soft);
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-sm);
		padding: 2px 7px;
		overflow-wrap: anywhere;
	}
	.state-badge {
		font-size: 10.5px;
		font-weight: 600;
		letter-spacing: 0.05em;
		text-transform: uppercase;
		padding: 2px 7px;
		border-radius: 999px;
		color: var(--ink-3);
		background: var(--bg-panel);
		border: 1px solid var(--rule-soft);
		white-space: nowrap;
	}
	.state-badge.ready {
		color: var(--ok);
		background: var(--ok-soft);
		border-color: color-mix(in srgb, var(--ok) 35%, var(--rule-soft));
	}
	.state-badge.warn {
		color: var(--fail);
		background: var(--fail-soft);
		border-color: color-mix(in srgb, var(--fail) 35%, var(--rule-soft));
	}
	.expert-note {
		margin: 0;
		font-size: 12px;
		line-height: 1.45;
		color: var(--ink-2);
		overflow-wrap: anywhere;
	}
	.expert-note code {
		font-size: 11.5px;
		color: var(--ink);
		background: var(--bg-soft);
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-sm);
		padding: 1px 5px;
	}
</style>
