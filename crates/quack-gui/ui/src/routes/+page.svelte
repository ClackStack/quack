<script lang="ts">
	import { onDestroy } from 'svelte';
	import { getTransport } from '#lib/transport';
	import { Workspaces } from '#lib/workspaces.svelte';
	import Welcome from '#lib/components/Welcome.svelte';
	import Workspace from '#lib/components/Workspace.svelte';

	const transport = getTransport();
	const workspaces = new Workspaces(transport);
	const active = $derived(workspaces.sessions[workspaces.activeIndex]);
	onDestroy(() => workspaces.dispose());

	async function pickFolder() {
		const cwd = await transport.pickFolder();
		if (cwd) await workspaces.open(cwd, { resume: 'latest' });
	}
</script>

<svelte:head>
	<title>_(•)&gt; quack</title>
</svelte:head>

{#if active}
	<Workspace
		session={active}
		sessions={workspaces.sessions}
		activeIndex={workspaces.activeIndex}
		onSelectTab={(i) => (workspaces.activeIndex = i)}
		onCloseTab={(i) => void workspaces.close(i)}
		onNewTab={pickFolder}
	/>
{:else}
	<Welcome {transport} opening={workspaces.opening} openError={workspaces.error} onOpen={(cwd, resume) => void workspaces.open(cwd, resume)} />
{/if}
