<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session }: { session: Session } = $props();
	let serviceId = $state('');
	let serviceKey = $state('');
	let baseUrl = $state('');
	let serverKey = $state('');
	const available = $derived(session.providers.filter((p) => p.available && p.models.length > 0));
	const busy = $derived(session.setupWorking || session.closed || session.starting);

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
							<span>{model.id}</span><span class="note">{model.price ?? 'Select model'}</span>
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
			<form onsubmit={saveKey}>
				<label
					>Provider<select bind:value={serviceId} disabled={busy} required>
						<option value="" disabled>Choose a provider</option>
						{#each session.services as service (service.id)}<option value={service.id}
								>{service.name}{service.ready ? ' · connected' : ''}</option
							>{/each}
					</select></label
				>
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
	{#each session.providers.filter((p) => !p.available) as provider (provider.id)}
		<p class="status"><strong>{provider.id}</strong>: {provider.note}</p>
	{/each}
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
	.heading {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: var(--sp-md);
	}
	h1 {
		font-size: 22px;
		margin: 0 0 8px;
	}
	h2 {
		font-size: 16px;
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
</style>
