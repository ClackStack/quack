<script lang="ts">
	import type { Session } from '#lib/session.svelte';
	import { theme } from '#lib/theme.svelte';
	import Header from './Header.svelte';
	import SetupPanel from './SetupPanel.svelte';
	import TrustPanel from './TrustPanel.svelte';
	import Sidebar from './Sidebar.svelte';
	import Transcript from './Transcript.svelte';
	import ApprovalCard from './ApprovalCard.svelte';
	import QuestionsCard from './QuestionsCard.svelte';
	import Composer from './Composer.svelte';

	let {
		session,
		sessions,
		activeIndex,
		onSelectTab,
		onCloseTab,
		onNewTab
	}: {
		session: Session;
		sessions: Session[];
		activeIndex: number;
		onSelectTab: (index: number) => void;
		onCloseTab: (index: number) => void;
		onNewTab: () => void;
	} = $props();

	let sidebarOpen = $state(false);

	const planReady = $derived(
		session.mode === 'plan' && session.plan.trim().length > 0 && !session.streaming
	);

	$effect(() => {
		if (session.info?.ui.theme) {
			theme.initFromSession(session.info.ui.theme);
		}
	});
</script>

<div class="workspace">
	<Header {session} />
	<div class="tabs" aria-label="Workspaces">
		{#each sessions as tab, i (tab.tab)}
			<button
				class="tab"
				aria-current={i === activeIndex ? 'page' : undefined}
				onclick={() => onSelectTab(i)}>{tab.info?.cwd ?? 'Workspace'}</button
			>
			<button
				class="tab ghost"
				aria-label={`Close ${tab.info?.cwd ?? 'workspace'}`}
				onclick={() => onCloseTab(i)}>×</button
			>
		{/each}
		<button class="tab ghost" aria-label="Open folder" onclick={onNewTab}>+</button>
		<button
			class="tab sidebar-toggle"
			aria-expanded={sidebarOpen}
			aria-controls="workspace-sidebar"
			onclick={() => (sidebarOpen = !sidebarOpen)}>Sessions</button
		>
	</div>

	<div class="body">
		<main>
			{#if session.actionError}
				<div class="action-error" role="alert">
					<span>{session.actionError}</span><button
						class="btn"
						onclick={() => (session.actionError = null)}>Dismiss</button
					>
				</div>
			{/if}
			{#if session.trust}
				<TrustPanel {session} />
			{:else if session.setupOpen}
				{#key session.tab}<SetupPanel {session} />{/key}
			{:else}
				<Transcript {session} />
				{#if planReady}
					<div class="plan-bar">
						<span class="pill pill-live">Plan ready</span>
						<p>{session.plan}</p>
						<div class="grow"></div>
						<button class="btn btn-primary" onclick={() => session.acceptPlan()}>
							Carry out the plan
						</button>
						<button class="btn" onclick={() => session.setMode('ask')}>Back to Ask</button>
					</div>
				{/if}
				{#if session.approval}
					<ApprovalCard
						approval={session.approval}
						busy={session.sending}
						onAnswer={(d) => void session.answerApproval(d)}
					/>
				{/if}
				{#if session.questions}
					{#key session.questions.callId}
						<QuestionsCard
							questions={session.questions.questions}
							busy={session.sending}
							onAnswer={(a) => void session.answerQuestions(a)}
						/>
					{/key}
				{/if}
				<Composer {session} />
			{/if}
		</main>
		<div id="workspace-sidebar" class="sidebar" class:open={sidebarOpen}><Sidebar {session} /></div>
	</div>
</div>

<style>
	.workspace {
		height: 100vh;
		display: grid;
		grid-template-rows: auto auto auto minmax(0, 1fr);
		overflow: hidden;
	}

	.action-error {
		display: flex;
		align-items: center;
		gap: 16px;
		padding: 12px 24px;
		color: var(--fail);
		background: var(--fail-soft);
		overflow-wrap: anywhere;
	}
	.action-error span {
		flex: 1;
	}

	.tabs {
		overflow-x: auto;
		display: flex;
		align-items: center;
		gap: var(--sp-xs);
		padding: 4px 12px;
		border-bottom: 1px solid var(--rule);
		background: var(--bg-soft);
	}

	.tab {
		font-family: var(--font-mono);
		font-size: 12px;
		padding: 3px 10px;
		border-radius: var(--r-md);
		background: var(--bg-panel);
		border: 1px solid var(--rule);
	}

	.tab.ghost {
		background: none;
		border-style: dashed;
		color: var(--ink-2);
	}

	.sidebar {
		min-height: 0;
		display: grid;
	}
	.sidebar-toggle {
		display: none;
	}

	.body {
		display: grid;
		grid-template-columns: 1fr var(--sidebar-w);
		min-height: 0;
	}

	main {
		display: flex;
		flex-direction: column;
		min-height: 0;
		min-width: 0;
	}

	.plan-bar {
		display: flex;
		align-items: center;
		gap: var(--sp-md);
		padding: 10px 16px;
		border-top: 1px solid var(--duck-line);
		background: var(--duck-soft);
	}

	.plan-bar p {
		margin: 0;
		font-family: var(--font-mono);
		font-size: 12px;
		white-space: pre-wrap;
		color: var(--duck-ink);
		display: -webkit-box;
		-webkit-line-clamp: 3;
		line-clamp: 3;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.grow {
		flex: 1;
	}

	@media (max-width: 900px) {
		.body {
			position: relative;
			grid-template-columns: minmax(0, 1fr);
		}
		.sidebar-toggle {
			display: block;
			margin-left: auto;
		}
		.sidebar {
			display: none;
		}
		.sidebar.open {
			display: grid;
			position: absolute;
			inset: 0 0 0 auto;
			width: var(--sidebar-w);
			z-index: 20;
		}
	}
</style>
