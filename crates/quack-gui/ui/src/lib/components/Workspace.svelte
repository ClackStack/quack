<script lang="ts">
	import { summarizeJobs, type JobActionTarget } from '#lib/jobs';
	import type { Session } from '#lib/session.svelte';
	import type { SidebarProject } from '#lib/workspaces.svelte';
	import { theme } from '#lib/theme.svelte';
	import Header from './Header.svelte';
	import SetupPanel from './SetupPanel.svelte';
	import TrustPanel from './TrustPanel.svelte';
	import Sidebar from './Sidebar.svelte';
	import Transcript from './Transcript.svelte';
	import ApprovalCard from './ApprovalCard.svelte';
	import QuestionsCard from './QuestionsCard.svelte';
	import Composer from './Composer.svelte';
	import ModelModal from './ModelModal.svelte';
	import ContextModal from './ContextModal.svelte';
	import EgressModal from './EgressModal.svelte';
	import MemoryModal from './MemoryModal.svelte';

	let {
		session,
		sessions,
		projects,
		opening = false,
		openingCwd = null,
		openError = null,
		railTab = $bindable<'projects' | 'jobs'>('projects'),
		onDismissOpenError,
		onSelectProject,
		onCloseProject,
		onSelectJob,
		onReopenJob,
		onCloseJob,
		onNewTab
	}: {
		session: Session;
		sessions: Session[];
		projects: SidebarProject[];
		opening?: boolean;
		openingCwd?: string | null;
		openError?: string | null;
		railTab?: 'projects' | 'jobs';
		onDismissOpenError?: () => void;
		onSelectProject: (cwd: string) => void;
		onCloseProject: (cwd: string) => void;
		onSelectJob: (tab: number, target?: JobActionTarget) => void;
		onReopenJob: (tab: number) => void;
		onCloseJob: (tab: number) => void;
		onNewTab: () => void;
	} = $props();

	let sidebarOpen = $state(false);
	let activeModal = $state<'model' | 'context' | 'egress' | 'memory' | null>(null);
	let memoryTarget = $state<{ scope: string; name: string; create?: boolean } | null>(null);

	function openMemory(target?: { scope: string; name: string; create?: boolean }) {
		memoryTarget = target ?? null;
		activeModal = 'memory';
	}
	const attentionCount = $derived(summarizeJobs(sessions).attention);

	const planReady = $derived(
		session.workMode === 'plan' && session.plan.trim().length > 0 && !session.streaming
	);
	// While a decision is on the table, the rest of the room steps back: the log,
	// the rail and the composer dim so the specimen plate holds the only full-strength ink.
	const awaiting = $derived(!!session.approval || !!session.questions || !!session.stepLimit);

	$effect(() => {
		if (awaiting || session.trust || session.setupOpen) activeModal = null;
	});

	$effect(() => {
		if (session.modelPickerAfterFork) {
			session.modelPickerAfterFork = false;
			if (!awaiting && !session.trust && !session.setupOpen) activeModal = 'model';
		}
	});

	$effect(() => {
		if (session.egressOpen) {
			session.egressOpen = false;
			if (!awaiting && !session.trust && !session.setupOpen) activeModal = 'egress';
		}
	});

	$effect(() => {
		void session.tab;
		activeModal = null;
		memoryTarget = null;
	});

	$effect(() => {
		if (session.info?.ui.theme) {
			theme.initFromSession(session.info.ui.theme);
		}
	});
</script>

<div class="workspace" class:awaiting>
	<div id="workspace-sidebar" class="sidebar" class:open={sidebarOpen}>
		<Sidebar
			{session}
			{sessions}
			{projects}
			{opening}
			{openingCwd}
			bind:railTab
			onSelectProject={(cwd) => {
				sidebarOpen = false;
				onSelectProject(cwd);
			}}
			{onCloseProject}
			onSelectJob={(tab, target) => {
				sidebarOpen = false;
				onSelectJob(tab, target);
			}}
			onInterruptJob={(tab) => void sessions.find((s) => s.tab === tab)?.interrupt()}
			onDismissIssue={(tab) => sessions.find((s) => s.tab === tab)?.dismissIssue()}
			{onReopenJob}
			{onCloseJob}
			{onNewTab}
			onOpenMemory={(target) => {
				sidebarOpen = false;
				openMemory(target);
			}}
		/>
	</div>
	<div class="pane">
		<Header
			{session}
			{sidebarOpen}
			{attentionCount}
			onToggleSidebar={() => (sidebarOpen = !sidebarOpen)}
			onOpenEgress={() => (activeModal = 'egress')}
			onOpenMemory={() => openMemory()}
		/>
		<main>
			{#if session.actionError || openError}
				<div class="action-error" role="alert">
					<span>{session.actionError ?? openError}</span><button
						class="btn"
						onclick={() => {
							session.dismissIssue();
							onDismissOpenError?.();
						}}>Dismiss</button
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
						<button class="btn" onclick={() => session.setWorkMode('edit')}>Back to Edit</button>
					</div>
				{/if}
				{#if session.approval || session.questions || session.stepLimit}
					<div class="decision-slot">
						{#if session.approval}
							<ApprovalCard
								approval={session.approval}
								busy={session.sending}
								onAnswer={(d, feedback, scope) => void session.answerApproval(d, feedback, scope)}
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
						{#if !session.approval && !session.questions && session.stepLimit}
							<section class="step-plate" aria-label="Step limit pause">
								<div class="step-head">
									<span class="step-kicker">Step limit</span>
									<h3>Paused after {session.stepLimit.limit} model steps (turn {session.stepLimit.turn})</h3>
								</div>
								<p class="step-body">
									The agent hit its step budget for this turn. Review the transcript above before letting it continue.
								</p>
								<div class="step-actions">
									<button
										type="button"
										class="btn"
										disabled={session.sending}
										onclick={() => void session.continueTurn(false)}
									>
										Stop here
									</button>
									<button
										type="button"
										class="btn btn-primary"
										disabled={session.sending}
										onclick={() => void session.continueTurn(true)}
									>
										Keep going
									</button>
								</div>
							</section>
						{/if}
					</div>
				{/if}
				<Composer
					{session}
					modalOpen={activeModal !== null}
					onOpenModels={() => (activeModal = 'model')}
					onOpenContext={() => (activeModal = 'context')}
				/>
			{/if}
			</main>
		</div>
	</div>

{#if activeModal === 'model'}
	<ModelModal {session} onClose={() => (activeModal = null)} />
{:else if activeModal === 'context'}
	<ContextModal {session} onClose={() => (activeModal = null)} />
{:else if activeModal === 'egress'}
	<EgressModal {session} onClose={() => (activeModal = null)} />
{:else if activeModal === 'memory'}
	<MemoryModal
		{session}
		initialTarget={memoryTarget}
		onClose={() => {
			activeModal = null;
			memoryTarget = null;
		}}
	/>
{/if}

<style>
	.workspace {
		height: 100vh;
		display: grid;
		/* The rail runs the full height of the window; the header and the
		   waterline belong to the chat pane alone. */
		grid-template-columns: var(--sidebar-w) minmax(0, 1fr);
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

	.sidebar {
		min-height: 0;
		display: grid;
	}

	.pane {
		min-width: 0;
		min-height: 0;
		display: grid;
		grid-template-rows: auto auto minmax(0, 1fr);
	}

	main {
		display: flex;
		flex-direction: column;
		min-height: 0;
		min-width: 0;
	}

	.decision-slot {
		flex: 0 1 auto;
		min-height: 0;
		padding-top: 4px;
		overflow-y: auto;
		overscroll-behavior: contain;
	}

	/* Decision focus: everything but the plate on the table steps back. */
	.awaiting :global(.transcript),
	.awaiting :global(.composer),
	.awaiting :global(.rail),
	.awaiting :global(header),
	.plan-bar {
		transition:
			opacity 220ms ease-out,
			filter 220ms ease-out;
	}

	.awaiting :global(.transcript),
	.awaiting :global(.composer),
	.awaiting :global(.rail),
	.awaiting :global(header) {
		opacity: 0.55;
		filter: saturate(0.6);
	}

	.plan-bar {
		display: flex;
		align-items: center;
		gap: var(--sp-md);
		padding: 10px 16px;
		border-top: 1px solid var(--duck-line);
		background: var(--duck-soft);
	}

	.awaiting .plan-bar {
		opacity: 0.55;
		filter: saturate(0.6);
	}

	.plan-bar p {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 13px;
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

	.step-plate {
		margin: 0 auto 10px;
		max-width: min(760px, calc(100% - 32px));
		padding: 14px 18px;
		border: 1px solid var(--duck-line);
		border-left: 4px solid var(--duck);
		border-radius: var(--r-lg);
		background: var(--bg-raised);
		box-shadow: var(--shadow-card);
		display: flex;
		flex-direction: column;
		gap: 10px;
	}

	.step-head {
		display: flex;
		align-items: center;
		gap: 10px;
		flex-wrap: wrap;
	}

	.step-kicker {
		font-family: var(--font-mono);
		font-size: 10.5px;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.08em;
		padding: 2px 7px;
		border-radius: 999px;
		background: var(--duck-soft);
		color: var(--duck-ink);
		border: 1px solid var(--duck-line);
	}

	.step-head h3 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 15px;
		color: var(--ink);
	}

	.step-body {
		margin: 0;
		font-size: 13px;
		color: var(--ink-2);
	}

	.step-actions {
		display: flex;
		justify-content: flex-end;
		gap: 8px;
	}

	@media (max-width: 900px) {
		.workspace {
			position: relative;
			grid-template-columns: minmax(0, 1fr);
		}
		.sidebar {
			display: none;
		}
		.sidebar.open {
			display: grid;
			position: absolute;
			inset: 0 auto 0 0;
			width: var(--sidebar-w);
			max-width: 86vw;
			z-index: 20;
		}
	}
</style>
