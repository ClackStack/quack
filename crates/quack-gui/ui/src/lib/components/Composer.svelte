<script lang="ts">
	import {
		APPROVALS,
		WORK_MODES,
		type ApprovalPolicy,
		type Session,
		type WorkMode
	} from '#lib/session.svelte';

	let {
		session,
		modalOpen = false,
		onOpenModels,
		onOpenContext
	}: {
		session: Session;
		modalOpen?: boolean;
		onOpenModels: () => void;
		onOpenContext: () => void;
	} = $props();

	let editor: HTMLTextAreaElement | undefined = $state();
	let showWorkMode = $state(false);
	let showApprovals = $state(false);

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

	function autogrow() {
		if (!editor) return;
		editor.style.height = 'auto';
		editor.style.height = `${Math.min(editor.scrollHeight, 240)}px`;
	}

	async function send() {
		const draft = session.draft;
		const body = draft.trim();
		if (!body || !session.canSubmit) return;
		if (await session.submit(body)) {
			if (session.draft === draft) session.draft = '';
			autogrow();
		}
	}

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Enter' && !event.shiftKey && !event.isComposing) {
			event.preventDefault();
			void send();
		}
	}

	function closeAllPopups() {
		showWorkMode = false;
		showApprovals = false;
		session.thinkingOpen = false;
	}

	function onWindowClick(event: MouseEvent) {
		if ((event.target as HTMLElement).closest('.ctrl-anchor')) return;
		closeAllPopups();
	}

	function onWindowKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape' && (showWorkMode || showApprovals || session.thinkingOpen)) {
			event.preventDefault();
			closeAllPopups();
		}
	}

	// Dismiss popups when an approval or question plate takes over the room.
	$effect(() => {
		if (session.approval || session.questions) closeAllPopups();
	});

	// Keep the composer focused while nothing else asks for the keyboard; approvals
	// answer with Enter from anywhere.
	$effect(() => {
		if (
			!session.approval &&
			!session.questions &&
			!session.switching &&
			!modalOpen &&
			!showWorkMode &&
			!showApprovals &&
			!session.thinkingOpen
		) {
			editor?.focus();
		}
	});
</script>

<svelte:window onclick={onWindowClick} onkeydown={onWindowKeydown} />

<div class="composer">
	<div class="box">
		<textarea
			bind:this={editor}
			bind:value={session.draft}
			rows={1}
			placeholder={session.streaming || session.turnPending
				? 'quack is working — interrupt or wait…'
				: 'Ask quack to do something in this folder…'}
			onkeydown={onKeydown}
			oninput={autogrow}
			aria-label="Message"
			disabled={session.closed || session.sending || session.switching || session.starting}
		></textarea>
		<div class="row">
			<span class="hint">
				{#if session.streaming || session.turnPending}
					<button class="btn stop" onclick={() => session.interrupt()}>Interrupt</button>
				{:else}
					<span><span class="kbd">⏎</span> send · <span class="kbd">⇧⏎</span> newline</span>
				{/if}
			</span>
			<button
				class="btn btn-primary send"
				onclick={() => void send()}
				disabled={!session.draft.trim() || !session.canSubmit}
				aria-label="Send"
			>
				<svg viewBox="0 0 24 24" aria-hidden="true"
					><path
						d="M12 19V5.5"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
					/><path
						d="m6.5 11 5.5-5.5L17.5 11"
						fill="none"
						stroke="currentColor"
						stroke-width="2"
						stroke-linecap="round"
						stroke-linejoin="round"
					/></svg
				>
			</button>
		</div>
	</div>

	<div class="controls" aria-label="Session controls">
		<div class="ctrl-anchor">
			<button
				type="button"
				class="ctrl"
				class:pill-live={session.workMode === 'plan'}
				class:ctrl-open={showWorkMode}
				aria-expanded={showWorkMode}
				aria-haspopup="menu"
				title="Change work mode"
				onclick={() => {
					const next = !showWorkMode;
					closeAllPopups();
					showWorkMode = next;
				}}
			>
				<span class="ctrl-k">mode</span>
				<span class="ctrl-v">{workModeLabel}</span>
				<span class="caret" aria-hidden="true">▾</span>
			</button>
			{#if showWorkMode}
				<div class="popup" role="menu" aria-label="Work mode">
					<div class="popup-head">Work mode</div>
					{#each WORK_MODES as mode (mode.id)}
						{@const active = session.workMode === mode.id}
						<button
							type="button"
							class="choice"
							class:active
							role="menuitemradio"
							aria-checked={active}
							onclick={() => {
								void session.setWorkMode(mode.id as WorkMode);
								showWorkMode = false;
							}}
						>
							<span class="choice-top">
								<span class="choice-label">{mode.label}</span>
								{#if active}<span class="check" aria-hidden="true">✓</span>{/if}
							</span>
							<span class="choice-note">{mode.about}</span>
						</button>
					{/each}
					<div class="popup-foot">What the agent may do; Plan proposes changes for review.</div>
				</div>
			{/if}
		</div>

		<div class="ctrl-anchor">
			<button
				type="button"
				class="ctrl"
				class:pill-live={session.approvals === 'yolo'}
				class:ctrl-open={showApprovals}
				aria-expanded={showApprovals}
				aria-haspopup="menu"
				title="Change approval permissions"
				onclick={() => {
					const next = !showApprovals;
					closeAllPopups();
					showApprovals = next;
				}}
			>
				<span class="ctrl-k">permissions</span>
				<span class="ctrl-v">{approvalsLabel}{session.sandboxOff ? ' · sandbox off' : ''}</span>
				<span class="caret" aria-hidden="true">▾</span>
			</button>
			{#if showApprovals}
				<div class="popup" role="menu" aria-label="Permissions">
					<div class="popup-head">Permissions</div>
					{#each approvalChoices as approval (approval.id)}
						{@const active = session.approvals === approval.id}
						<button
							type="button"
							class="choice"
							class:active
							role="menuitemradio"
							aria-checked={active}
							onclick={() => {
								void session.setApprovals(approval.id as ApprovalPolicy);
								showApprovals = false;
							}}
						>
							<span class="choice-top">
								<span class="choice-label">{approval.label}</span>
								{#if active}<span class="check" aria-hidden="true">✓</span>{/if}
							</span>
							<span class="choice-note">{approval.about}</span>
						</button>
					{/each}
					<div class="popup-foot">
						Approvals apply within the work mode; deny rules and the sandbox still hold.
					</div>
				</div>
			{/if}
		</div>

		<button
			type="button"
			class="ctrl model-ctrl"
			class:pill-live={session.streaming}
			aria-haspopup="dialog"
			title={`Change model (${modelLabel})`}
			onclick={() => {
				closeAllPopups();
				onOpenModels();
			}}
		>
			<span class="ctrl-k">model</span>
			<span class="model-name">{modelLabel}</span>
			{#if session.streaming}<span class="live-dot" aria-label="streaming"></span>{/if}
		</button>

		{#if session.reasoningEfforts.length > 0}
			<div class="ctrl-anchor">
				<button
					type="button"
					class="ctrl"
					class:ctrl-open={session.thinkingOpen}
					aria-expanded={session.thinkingOpen}
					aria-haspopup="menu"
					title={`Thinking level for ${modelLabel}`}
					onclick={() => {
						const next = !session.thinkingOpen;
						closeAllPopups();
						session.thinkingOpen = next;
					}}
				>
					<span class="ctrl-k">thinking</span>
					<span class="ctrl-v">{thinkingLabel}</span>
					<span class="caret" aria-hidden="true">▾</span>
				</button>
				{#if session.thinkingOpen}
					<div class="popup" role="menu" aria-label="Thinking level">
						<div class="popup-head">Thinking · <span class="mono">{modelLabel}</span></div>
						<button
							type="button"
							class="choice"
							class:active={session.reasoningEffort === null}
							role="menuitemradio"
							aria-checked={session.reasoningEffort === null}
							onclick={() => void session.setReasoningEffort(null)}
						>
							<span class="choice-top">
								<span class="choice-label">Default</span>
								{#if session.reasoningEffort === null}<span class="check" aria-hidden="true">✓</span
									>{/if}
							</span>
							<span class="choice-note">The level configured for this model</span>
						</button>
						{#each session.reasoningEfforts as level (level)}
							{@const active = session.reasoningEffort === level}
							<button
								type="button"
								class="choice"
								class:active
								role="menuitemradio"
								aria-checked={active}
								onclick={() => void session.setReasoningEffort(level)}
							>
								<span class="choice-top">
									<span class="choice-label mono">{level}</span>
									{#if active}<span class="check" aria-hidden="true">✓</span>{/if}
								</span>
							</button>
						{/each}
						<div class="popup-foot">
							Lower levels favor speed and cost; higher ones allow more time for hard tasks. Applies
							between turns and is saved with the session.
						</div>
					</div>
				{/if}
			</div>
		{/if}

		<button
			type="button"
			class="ctrl context-ctrl"
			class:nearing
			aria-haspopup="dialog"
			title="Inspect context window and compact history"
			onclick={() => {
				closeAllPopups();
				void session.refreshContext();
				onOpenContext();
			}}
		>
			<span class="ctrl-k">context</span>
			<span class="ctrl-v mono">{session.context ? `${Math.round(fill * 100)}%` : '—'}</span>
		</button>
	</div>
</div>

<style>
	.composer {
		flex: none;
		padding: var(--sp-sm) var(--sp-lg) 12px;
		border-top: 3px double var(--rule);
		background: var(--bg-panel);
	}

	.box {
		border: 1px solid var(--rule);
		border-radius: var(--r-xl);
		background: var(--bg-raised);
		box-shadow: var(--shadow-card);
		padding: 10px 12px 8px;
		display: grid;
		gap: 6px;
	}

	.box:focus-within {
		border-color: var(--sky-ink);
	}

	textarea {
		resize: none;
		border: none;
		background: none;
		outline: none;
		font-family: var(--font-serif);
		font-size: var(--text-prose);
		line-height: 1.5;
		min-height: 24px;
		max-height: 240px;
		color: var(--ink);
	}

	textarea::placeholder {
		color: var(--ink-3);
	}

	.row {
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
	}

	.hint {
		flex: 1;
		color: var(--ink-3);
		font-size: 12px;
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
	}

	.stop {
		border-color: var(--fail);
		color: var(--fail);
		padding: 2px 10px;
	}

	.stop:hover:not(:disabled) {
		background: var(--fail-soft);
	}

	.send {
		padding: 0;
		width: 34px;
		height: 34px;
		justify-content: center;
		border-radius: 50%;
	}

	.send svg {
		width: 16px;
		height: 16px;
	}

	/* Session controls directly beneath the input box */
	.controls {
		position: relative;
		margin-top: 8px;
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px;
	}

	.ctrl-anchor {
		position: relative;
	}

	.ctrl {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		height: 26px;
		padding: 0 10px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink-2);
		font-size: 12px;
		font-weight: 500;
		white-space: nowrap;
		transition:
			background-color 120ms ease-out,
			border-color 120ms ease-out,
			color 120ms ease-out;
	}

	.ctrl:hover:not(:disabled) {
		background: var(--bg-soft);
		border-color: var(--ink-3);
		color: var(--ink);
	}

	.ctrl-open {
		border-color: var(--sky-ink);
		background: var(--sky-soft);
		color: var(--ink);
	}

	.ctrl.pill-live {
		border-color: var(--duck-line);
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.ctrl.nearing {
		border-color: var(--duck-line);
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.ctrl-k {
		color: var(--ink-3);
		font-weight: 400;
	}

	.ctrl.pill-live .ctrl-k,
	.ctrl.nearing .ctrl-k {
		color: inherit;
		opacity: 0.8;
	}

	.ctrl-v {
		color: inherit;
		font-weight: 600;
	}

	.caret {
		font-size: 10px;
		color: var(--ink-3);
		margin-left: 1px;
	}

	.model-ctrl {
		max-width: min(44vw, 320px);
		min-width: 0;
	}

	.model-name {
		font-family: var(--font-mono);
		font-size: 11.5px;
		font-weight: 500;
		color: var(--ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.context-ctrl {
		margin-left: auto;
	}

	.mono {
		font-family: var(--font-mono);
	}

	.live-dot {
		width: 7px;
		height: 7px;
		border-radius: 999px;
		background: var(--beak);
		flex: none;
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

	/* Upward anchored popups */
	.popup {
		position: absolute;
		bottom: calc(100% + 8px);
		left: 0;
		min-width: 260px;
		max-width: min(320px, calc(100vw - 32px));
		max-height: min(340px, 55vh);
		overflow-y: auto;
		overscroll-behavior: contain;
		background: var(--bg-raised);
		border: 1px solid var(--plate-line);
		border-radius: var(--r-lg);
		box-shadow: var(--shadow-pop);
		padding: 6px;
		z-index: 40;
		display: grid;
		gap: 3px;
	}

	.popup-head {
		font-weight: 700;
		font-size: 11.5px;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		color: var(--ink-3);
		padding: 5px 8px 3px;
	}

	.popup-head .mono {
		text-transform: none;
		letter-spacing: 0;
		color: var(--ink-2);
	}

	.choice {
		width: 100%;
		display: grid;
		gap: 2px;
		text-align: left;
		padding: 7px 8px;
		border-radius: var(--r-md);
		border: 1px solid transparent;
		transition:
			background-color 120ms ease-out,
			border-color 120ms ease-out;
	}

	.choice:hover:not(:disabled) {
		background: var(--bg-soft);
	}

	.choice.active {
		background: var(--sky-soft);
		border-color: color-mix(in srgb, var(--sky) 45%, var(--rule));
	}

	.choice-top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.choice-label {
		font-size: 13px;
		font-weight: 600;
		color: var(--ink);
	}

	.check {
		color: var(--sky-ink);
		font-size: 12px;
		font-weight: 700;
	}

	.choice-note {
		font-size: 11.5px;
		color: var(--ink-3);
	}

	.popup-foot {
		font-size: 11.5px;
		color: var(--ink-3);
		padding: 6px 8px 4px;
		border-top: 1px solid var(--rule-soft);
		margin-top: 3px;
	}

	@media (max-width: 600px) {
		.composer {
			padding: var(--sp-sm) var(--sp-md) 10px;
		}
		.ctrl-anchor {
			position: static;
		}
		.popup {
			left: 0;
			right: 0;
			max-width: none;
			min-width: 0;
		}
	}
</style>
