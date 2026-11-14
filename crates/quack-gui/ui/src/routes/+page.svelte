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

	// The dev demo can open its scripted folder by itself (`?demo=turn`), so a
	// headless capture reaches the workspace without a click.
	let demoStarted = false;
	$effect(() => {
		if (demoStarted || transport.kind !== 'dev' || active) return;
		if (new URLSearchParams(window.location.search).get('demo') !== 'turn') return;
		demoStarted = true;
		void workspaces.open('/home/you/Development/demo-pond', { resume: 'latest' });
	});

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
		projects={workspaces.sidebarProjects}
		opening={workspaces.opening}
		openingCwd={workspaces.openingCwd}
		openError={workspaces.error}
		onDismissOpenError={() => (workspaces.error = null)}
		onSelectProject={(cwd) => void workspaces.selectProject(cwd)}
		onCloseProject={(cwd) => void workspaces.closeProject(cwd)}
		onNewTab={pickFolder}
	/>
{:else}
	<Welcome
		{transport}
		opening={workspaces.opening}
		openError={workspaces.error}
		homePrefix={workspaces.homePrefix}
		onOpen={(cwd, resume) => void workspaces.open(cwd, resume)}
	/>
{/if}
