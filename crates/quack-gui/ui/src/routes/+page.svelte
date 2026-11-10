<script lang="ts">
	import { getTransport, type ResumeRequest } from '#lib/transport';
	import { Session } from '#lib/session.svelte';
	import Welcome from '#lib/components/Welcome.svelte';
	import Workspace from '#lib/components/Workspace.svelte';

	const transport = getTransport();

	let sessions = $state<Session[]>([]);
	let activeIndex = $state(-1);
	let opening = $state(false);
	let openError = $state<string | null>(null);

	const active = $derived(sessions[activeIndex]);

	// One listener fans every bridge message to the session that owns its tab; the
	// info messages refresh the AppInfo snapshot (tab start and config reloads).
	transport.listen((msg) => {
		const session = sessions.find((s) => s.tab === msg.tab);
		if (!session) return;
		if ('event' in msg) session.onEvent(msg.event);
		else if ('info' in msg) session.info = msg.info;
	});

	async function openFolder(cwd: string, resume: ResumeRequest = { resume: 'new' }) {
		opening = true;
		openError = null;
		try {
			const { tab, info } = await transport.openFolder(cwd, resume);
			sessions.push(new Session(transport, tab, info));
			activeIndex = sessions.length - 1;
			const session = sessions[activeIndex];
			await session.listSessions();
			session.starting = false;
		} catch (e) {
			openError = e instanceof Error ? e.message : String(e);
		} finally {
			opening = false;
		}
	}

	function closeTab(index: number) {
		const session = sessions[index];
		void transport.closeTab(session.tab);
		sessions.splice(index, 1);
		if (activeIndex >= sessions.length) activeIndex = sessions.length - 1;
	}
</script>

<svelte:head>
	<title>quack</title>
</svelte:head>

{#if active}
	<Workspace
		session={active}
		tabCount={sessions.length}
		onSelectTab={(i) => (activeIndex = i)}
		onCloseTab={closeTab}
		onNewTab={() => openFolder(active.info?.cwd ?? '')}
	/>
{:else}
	<Welcome {transport} {opening} {openError} onOpen={(cwd, resume) => openFolder(cwd, resume)} />
{/if}
