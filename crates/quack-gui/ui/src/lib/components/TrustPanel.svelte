<script lang="ts">
	import type { Session } from '#lib/session.svelte';
	let { session }: { session: Session } = $props();
</script>

<section class="trust" aria-labelledby="trust-heading">
	<h1 id="trust-heading">
		{session.trust?.changed
			? 'Project configuration changed'
			: 'How should quack use this project’s configuration?'}
	</h1>
	<p class="path">{session.trust?.path}</p>
	<p class="summary">{session.trust?.summary}</p>
	<p>Project configuration can start tools and servers. Choose what to allow for this folder.</p>
	<fieldset disabled={session.trustSaving || session.closed}>
		<legend>Project access</legend>
		<button class="choice" onclick={() => session.trustProject('trusted')}>
			<strong>Trust project</strong><span
				>Load its settings, providers, tools, hooks, and servers.</span
			>
		</button>
		<button class="choice" onclick={() => session.trustProject('settings-only')}>
			<strong>Use settings only</strong><span
				>Apply ordinary settings. Keep executable configuration disabled.</span
			>
		</button>
		<button class="choice" onclick={() => session.trustProject('ignored')}>
			<strong>Ignore project configuration</strong><span
				>Continue with your user and system configuration.</span
			>
		</button>
	</fieldset>
	{#if session.trustSaving}<p role="status">Saving your choice and reloading…</p>{/if}
</section>

<style>
	.trust {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: var(--sp-xl);
	}
	h1 {
		font-size: 22px;
		max-width: 36ch;
		margin: 0 0 16px;
	}
	p {
		color: var(--ink-2);
		max-width: 65ch;
	}
	.path {
		font-family: var(--font-mono);
		overflow-wrap: anywhere;
	}
	.summary {
		white-space: pre-wrap;
	}
	fieldset {
		border: 0;
		padding: 0;
		margin: var(--sp-lg) 0;
		max-width: 600px;
		display: grid;
		gap: var(--sp-sm);
	}
	legend {
		font-weight: 600;
		margin-bottom: var(--sp-sm);
	}
	.choice {
		display: grid;
		gap: 4px;
		text-align: left;
		padding: var(--sp-md);
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
	}
	.choice:hover:not(:disabled) {
		background: var(--bg-soft);
		border-color: var(--sky-ink);
	}
	.choice span {
		color: var(--ink-2);
	}
</style>
