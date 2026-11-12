<script lang="ts">
	import {
		APPROVALS,
		WORK_MODES,
		type ApprovalPolicy,
		type Session,
		type WorkMode
	} from '#lib/session.svelte';
	import iconUrl from '#lib/assets/icon.png';
	import ThemeToggle from '#lib/components/ThemeToggle.svelte';

	let { session }: { session: Session } = $props();

	let showModels = $state(false);
	let showWorkMode = $state(false);
	let showApprovals = $state(false);
	let showContext = $state(false);

	const modelLabel = $derived(session.currentModel ?? 'no model');
	const workModeLabel = $derived(
		WORK_MODES.find((m) => m.id === session.workMode)?.label ?? session.workMode
	);
	const approvalsLabel = $derived(
		APPROVALS.find((a) => a.id === session.approvals)?.label ?? session.approvals
	);
	const thinkingLabel = $derived(session.reasoningEffort ?? 'default');
	// Legacy policies stay visible only while active (docs/plan/15-desktop-client.md).
	const approvalChoices = $derived(
		APPROVALS.filter((a) => !a.legacy || session.approvals === a.id)
	);

	const fill = $derived.by(() => {
		const c = session.context;
		if (!c) return 0;
		const base = c.budget ?? c.window;
		return base ? Math.min(1, c.used / base) : 0;
	});
	const nearing = $derived(
		session.context?.threshold !== undefined &&
			session.context.threshold !== null &&
			(session.context?.used ?? 0) >= session.context.threshold * 0.9
	);

	function closeMenus(event: MouseEvent) {
		if ((event.target as HTMLElement).closest('.menu-anchor')) return;
		showModels = false;
		showWorkMode = false;
		showApprovals = false;
		showContext = false;
		session.thinkingOpen = false;
	}

	function closeMenusExcept(thinking: boolean) {
		showModels = false;
		showWorkMode = false;
		showApprovals = false;
		showContext = false;
		if (!thinking) session.thinkingOpen = false;
	}

	const disabled = $derived(
		session.closed ||
			session.starting ||
			!!session.trust ||
			session.switching ||
			session.setupWorking ||
			session.streaming ||
			session.turnPending
	);

	const providers = $derived(session.providers.filter((p) => p.models.length > 0));
</script>

<svelte:window onclick={closeMenus} />

<header>
	<div class="wordmark" title={`_(•)> quack ${session.info?.version ?? ''}`}>
		<img src={iconUrl} class="app-icon" alt="" width="22" height="22" />
		<span>quack</span>
	</div>

	<div class="menu-anchor">
		<button
			class="pill model-picker"
			class:pill-live={session.streaming}
			onclick={() => {
				showModels = !showModels;
				closeMenusExcept(false);
			}}
			aria-expanded={showModels}
		>
			{modelLabel}{#if session.reasoningEfforts.length > 0}<span class="note"> · {thinkingLabel}</span>{/if}
			{#if session.streaming}<span class="live-dot" aria-label="streaming"></span>{/if}
		</button>
		{#if session.reasoningEfforts.length > 0}
			<button
				class="pill chip"
				class:chip-open={session.thinkingOpen}
				onclick={() => {
					session.thinkingOpen = !session.thinkingOpen;
				closeMenusExcept(true);
			}}
				aria-expanded={session.thinkingOpen}
				title="Thinking level for {modelLabel}"
			>
				thinking: {thinkingLabel}
			</button>
		{/if}
		{#if showModels}
			<div class="menu" role="menu">
				{#each providers as provider (provider.id)}
					<div class="provider" class:unavailable={!provider.available}>
						<div class="provider-name">
							{provider.id}
							{#if !provider.available}<span class="note">{provider.note}</span>{/if}
						</div>
						{#each provider.models as model (model.id)}
							<button
								class="model"
								role="menuitem"
								disabled={!provider.available || disabled}
								onclick={() => {
									void session.setModel(`${provider.id}/${model.id}`);
									showModels = false;
								}}
							>
								<span class="model-id">{provider.id}/{model.id}</span>
								<span class="note">{[
									model.price,
									model.reasoning_efforts?.length
										? `thinking: ${model.reasoning_efforts.join(' · ')}`
										: null
									].filter(Boolean).join(' · ') || 'Select model'}</span>
							</button>
						{/each}
					</div>
				{/each}
				<button
					class="model"
					role="menuitem"
					{disabled}
					onclick={() => {
						showModels = false;
						void session.openSetup();
					}}>Connect a provider…</button
				>
				{#if providers.length === 0}
					<div class="empty">No providers ready.</div>
				{/if}
			</div>
		{/if}
		{#if session.thinkingOpen}
			<div class="menu" role="menu" aria-label="Thinking level">
				<div class="menu-heading">Thinking · {modelLabel}</div>
				<button
					class="model"
					role="menuitemradio"
					aria-checked={session.reasoningEffort === null}
					onclick={() => void session.setReasoningEffort(null)}
				>
					<span class="model-id">Default</span>
					<span class="note">The level configured for this model</span>
				</button>
				{#each session.reasoningEfforts as level (level)}
					<button
						class="model"
						role="menuitemradio"
						aria-checked={session.reasoningEffort === level}
						onclick={() => void session.setReasoningEffort(level)}
					>
						<span class="model-id">{level}</span>
					</button>
				{/each}
				<div class="menu-foot">
					Lower levels favor speed and cost; higher ones allow more time for hard tasks.
					Applies between turns and is saved with the session.
				</div>
			</div>
		{/if}
	</div>

	<div class="menu-anchor">
		<button
			class="pill"
			class:pill-live={session.workMode === 'plan'}
			onclick={() => {
				showWorkMode = !showWorkMode;
				closeMenusExcept(false);
			}}
			aria-expanded={showWorkMode}
		>
			{workModeLabel}
		</button>
		{#if showWorkMode}
			<div class="menu" role="menu" aria-label="Work mode">
				{#each WORK_MODES as mode (mode.id)}
					<button
						class="model"
						role="menuitemradio"
						aria-checked={session.workMode === mode.id}
						onclick={() => {
							void session.setWorkMode(mode.id as WorkMode);
							showWorkMode = false;
						}}
					>
						<span class="model-id">{mode.label}</span>
						<span class="note">{mode.about}</span>
					</button>
				{/each}
				<div class="menu-foot">What the agent may do; Plan proposes changes for review.</div>
			</div>
		{/if}
	</div>

	<div class="menu-anchor">
		<button
			class="pill"
			class:pill-live={session.approvals === 'yolo'}
			onclick={() => {
				showApprovals = !showApprovals;
				closeMenusExcept(false);
			}}
			aria-expanded={showApprovals}
		>
			{approvalsLabel}{session.sandboxOff ? ' · sandbox off' : ''}
		</button>
		{#if showApprovals}
			<div class="menu" role="menu" aria-label="Approvals">
				{#each approvalChoices as approval (approval.id)}
					<button
						class="model"
						role="menuitemradio"
						aria-checked={session.approvals === approval.id}
						onclick={() => {
							void session.setApprovals(approval.id as ApprovalPolicy);
							showApprovals = false;
						}}
					>
						<span class="model-id">{approval.label}</span>
						<span class="note">{approval.about}</span>
					</button>
				{/each}
				<div class="menu-foot">
					Approvals apply within the work mode; deny rules and the sandbox still hold.
				</div>
			</div>
		{/if}
	</div>

	<button class="btn" {disabled} onclick={() => session.openSetup()}>Providers</button>
	<div class="grow"></div>

	<div class="menu-anchor">
		<button
			class="pill"
			class:nearing
			onclick={() => {
				void session.refreshContext();
				showContext = !showContext;
				closeMenusExcept(false);
			}}
			aria-expanded={showContext}
		>
			context {session.context ? `${Math.round(fill * 100)}%` : '—'}
		</button>
		{#if showContext}
			<div class="menu right">
				{#if session.contextReport}
					{#each session.contextReport.segments as segment (segment.name)}
						<div class="segment">
							<span class="model-id">{segment.name}</span>
							<span class="note">{segment.tokens} tok · {segment.detail}</span>
						</div>
					{/each}
					{#if session.contextReport.note}
						<div class="empty">{session.contextReport.note}</div>
					{/if}
					<button
						class="model"
						role="menuitem"
						onclick={() => {
							void session.compact();
							showContext = false;
						}}
					>
						<span class="model-id">Compact now</span>
						<span class="note">summarize older history</span>
					</button>
				{:else}
					<div class="empty">
						{session.context ? `${session.context.used} tokens in use` : 'no context report yet'}
					</div>
				{/if}
			</div>
		{/if}
	</div>

	<ThemeToggle />
</header>

<!-- The signature move: the website's water line, carrying data. It fills toward the
     session's budget; when pruning or compaction is near it turns duck yellow. -->
<div class="waterline" aria-hidden="true">
	<div class="water" class:nearing style:--fill={fill}></div>
</div>

<style>
	header {
		height: var(--header-h);
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
		padding: 0 var(--sp-md);
		border-bottom: 1px solid var(--rule);
		background: var(--bg-panel);
	}

	.model-picker {
		max-width: min(38vw, 360px);
		overflow: hidden;
		text-overflow: ellipsis;
		display: block;
	}

	.chip {
		font-size: 12px;
		padding: 2px 8px;
	}

	.chip-open {
		border-color: var(--sky-ink);
	}

	.wordmark {
		font-family: var(--font-mono);
		font-weight: 700;
		font-size: 15px;
		letter-spacing: -0.02em;
		display: inline-flex;
		align-items: center;
		gap: 8px;
		margin-right: var(--sp-sm);
		user-select: none;
	}

	.wordmark .app-icon {
		width: 22px;
		height: 22px;
		object-fit: contain;
		display: block;
		border-radius: var(--r-sm);
	}

	.grow {
		flex: 1;
	}

	.live-dot {
		width: 7px;
		height: 7px;
		border-radius: 999px;
		background: var(--beak);
		animation: pulse 1.1s ease-out infinite alternate;
	}

	@keyframes pulse {
		from {
			opacity: 1;
		}
		to {
			opacity: 0.35;
		}
	}

	.menu-anchor {
		position: relative;
	}

	.menu {
		position: absolute;
		top: calc(100% + 6px);
		left: 0;
		min-width: 280px;
		max-height: 380px;
		overflow: auto;
		background: var(--bg-raised);
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		box-shadow: var(--shadow-pop);
		padding: 6px;
		z-index: 30;
	}

	.menu.right {
		left: auto;
		right: 0;
	}

	.menu-heading {
		font-weight: 700;
		font-size: 12px;
		color: var(--ink-2);
		padding: 6px 8px 2px;
	}

	.menu-foot {
		font-size: 12px;
		color: var(--ink-3);
		padding: 6px 8px;
		border-top: 1px solid var(--rule-soft);
		margin-top: 4px;
	}

	.provider {
		padding: 4px 6px;
	}

	.provider + .provider {
		border-top: 1px solid var(--rule-soft);
	}

	.provider-name {
		font-weight: 700;
		font-size: 12px;
		color: var(--ink-2);
		padding: 4px 8px 2px;
	}

	.provider.unavailable .provider-name {
		color: var(--ink-3);
	}

	.model {
		width: 100%;
		display: flex;
		flex-direction: column;
		align-items: stretch;
		gap: 1px;
		text-align: left;
		padding: 7px 8px;
		border-radius: var(--r-md);
	}

	.model:hover:not(:disabled) {
		background: var(--bg-soft);
	}

	.model-id {
		font-family: var(--font-mono);
		font-size: 13px;
	}

	.note {
		font-size: 12px;
		color: var(--ink-3);
	}

	.empty {
		padding: 10px;
		color: var(--ink-3);
	}

	.pill.nearing {
		border-color: var(--duck-line);
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.waterline {
		height: 3px;
		background: var(--rule-soft);
	}

	.water {
		height: 100%;
		width: calc(var(--fill, 0) * 100%);
		background: var(--sky);
		transition:
			width 400ms ease-out,
			background-color 400ms ease-out;
	}

	.water.nearing {
		background: var(--duck);
	}
</style>
