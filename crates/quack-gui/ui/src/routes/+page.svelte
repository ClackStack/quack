<script lang="ts">
	import { onDestroy } from 'svelte';
	import type { JobActionTarget } from '#lib/jobs';
	import { getTransport } from '#lib/transport';
	import { Workspaces } from '#lib/workspaces.svelte';
	import Welcome from '#lib/components/Welcome.svelte';
	import Workspace from '#lib/components/Workspace.svelte';

	const transport = getTransport();
	const workspaces = new Workspaces(transport);
	const active = $derived(workspaces.sessions[workspaces.activeIndex]);
	const initialDemo =
		typeof window !== 'undefined' && transport.kind === 'dev'
			? new URLSearchParams(window.location.search).get('demo')
			: null;
	let railTab = $state<'projects' | 'jobs'>(initialDemo === 'jobs' ? 'jobs' : 'projects');
	onDestroy(() => workspaces.dispose());

	// The dev demo can open its scripted folder by itself (`?demo=turn` or `?demo=jobs`),
	// so a headless capture reaches the workspace without a click.
	let demoStarted = false;
	$effect(() => {
		if (demoStarted || transport.kind !== 'dev' || active) return;
		if (initialDemo === 'turn') {
			demoStarted = true;
			void workspaces.open('/home/you/Development/demo-pond', { resume: 'latest' });
		} else if (initialDemo === 'jobs') {
			demoStarted = true;
			void (async () => {
				for (const cwd of [
					'/home/you/Development/demo-pond',
					'/home/you/Development/quack',
					'/home/you/Development/orderly',
					'/home/you/Development/field-notes'
				]) {
					await workspaces.open(cwd, { resume: 'latest' });
				}
				workspaces.selectTab(0);
			})();
		}
	});

	async function pickFolder() {
		const cwd = await transport.pickFolder();
		if (cwd) await workspaces.open(cwd, { resume: 'latest' });
	}

	function selectJob(tab: number, target?: JobActionTarget) {
		if (!workspaces.selectTab(tab)) return;
		const targetSession = workspaces.sessions.find((s) => s.tab === tab);
		if (!targetSession) return;
		if (target === 'providers') void targetSession.openSetup();
		else targetSession.setupOpen = false;
	}

	function closeJob(tab: number) {
		const idx = workspaces.sessions.findIndex((s) => s.tab === tab);
		if (idx >= 0) void workspaces.close(idx);
	}
</script>

<svelte:head>
	<title>_(•)&gt; quack</title>
</svelte:head>

{#if active}
	<Workspace
		session={active}
		sessions={workspaces.sessions}
		projects={workspaces.sidebarProjects}
		opening={workspaces.opening}
		openingCwd={workspaces.openingCwd}
		openError={workspaces.error}
		bind:railTab
		onDismissOpenError={() => (workspaces.error = null)}
		onSelectProject={(cwd) => void workspaces.selectProject(cwd)}
		onCloseProject={(cwd) => void workspaces.closeProject(cwd)}
		onSelectJob={selectJob}
		onReopenJob={(tab) => void workspaces.reopenTab(tab)}
		onCloseJob={closeJob}
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
