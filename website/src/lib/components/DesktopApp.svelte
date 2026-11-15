<script lang="ts">
	import Plate from '#lib/components/Plate.svelte';
	import PlateHeading from '#lib/components/PlateHeading.svelte';
	import { Check, ChevronLeft, ChevronRight, Maximize2, Monitor, Moon, Sun, X } from '@lucide/svelte';

	type ScreenId =
		| 'approval'
		| 'jobs'
		| 'workspace'
		| 'models'
		| 'providers'
		| 'egress'
		| 'memory'
		| 'settings';
	type PreviewTheme = null | 'light' | 'dark';

	type Screen = {
		id: ScreenId;
		tab: string;
		short: string;
		title: string;
		summary: string;
		points: [string, string][];
		alt: string;
	};

	const screens: Screen[] = [
		{
			id: 'approval',
			tab: 'Edit approvals',
			short: 'Numbered diff & rules',
			title: 'Approve file edits and secret guards with context',
			summary:
				'Writes, patches, and Pre-Egress Secret Shield detections pause for review before touching disk or leaving your machine over HTTP. Circled line markers highlight diffs alongside persistent rules and rejection feedback.',
			points: [
				['Decision, rule & secret actions', 'Approve writes, save scoped rules, explain why not, or choose Redact & send / Allowlist on Pre-Egress Secret Shield alerts.'],
				['Precision @ slices & planner strip', 'Attach @file#symbol or @file:Lstart-Lend slices via tree-sitter and images, and switch mode, permissions, main model, and planner model below.'],
				['Exports, undo & memory', 'Branch jobs, try another model from a turn, export Markdown or JSON, Undo last writes, and open shared Memory notes.']
			],
			alt: 'quack desktop app paused for approval on a diff preview of src/pond/ducks.ts, with numbered additions, Approve change, Allow for this session, Save rule, Not this — tell quack why, header boundary pills, and model/planner controls below the composer'
		},
		{
			id: 'jobs',
			tab: 'Parallel jobs',
			short: 'Urgency queue across repos',
			title: 'Triage parallel jobs across repositories',
			summary:
				'Keep turns running across multiple repositories at once, switch from Projects to the live Jobs queue on approvals or step-limit pauses, and reopen tabs where you left off.',
			points: [
				['Sorted by urgency', 'Waiting approvals, secret guards, step-limit pauses, and failures rise above running and ready conversations.'],
				['Sub-agent & plan status', 'Track active explore sub-agents, tool counts, and plan steps directly on each card, with persistent workspace tab restore.'],
				['Header waterline & Cmd/Ctrl-K', 'Inspect LSP diagnostics, Offline/Shield egress status, streaming progress, and jump to any modal via the Cmd/Ctrl-K palette or slash commands.']
			],
			alt: 'quack desktop app Jobs tab triaging four jobs across quack, orderly, demo-pond, and field-notes, showing an outside-sandbox approval, a provider timeout error, and an active turn with an explore sub-agent and plan progress bar'
		},
		{
			id: 'workspace',
			tab: 'Context & cache',
			short: 'KV zones, meter & trim',
			title: 'Inspect KV zones and selectively trim stale tail items',
			summary:
				'Settled turns fold reasoning into collapsible traces while the sidebar tracks cache reuse and throughput, and /context splits the immutable KV prefix from the prunable tail.',
			points: [
				['KV prefix vs prunable tail', 'Inspect immutable KV prefix, session context, and prunable tail zones in /context and manually Trim stale items without busting the cache.'],
				['Per-call cache chart', 'Compare sent tokens, % reused from cache, throughput (tok/s), time to first token, and automatic pruning thresholds.'],
				['Modal parity & Cmd/Ctrl-K', 'Open /doctor, /diagnostics, /instructions, /profiles, or transcript exports directly from the palette or / slash menu.']
			],
			alt: 'quack desktop app settled conversation showing collapsed thinking toggle, written lines in src/pond/ducks.ts, 16% context meter with pruning tick, 80% cache hit usage sparkline at 43 tok/s, and attachment buttons below the composer'
		},
		{
			id: 'models',
			tab: 'Model & planner',
			short: 'Main, planner & probe',
			title: 'Switch models, tune thinking, and probe tool calling',
			summary:
				'Pin a dedicated Plan-mode architect beside your main model across workspaces, switch declared thinking efforts, and probe endpoints live for native tool-calling reliability.',
			points: [
				['Synthetic tool-calling probe', 'Run a fast live /probe right from the model picker to test native tool_calls support against text fallback markup.'],
				['Planner split & effort levels', 'Pair a reasoning planner with a fast local editor and inspect Local / Free tags, context windows, ~tok/s speed, and thinking efforts.'],
				['Same TOML layers as CLI', 'Reads 42 bundled provider definitions and local server discovery with nothing hardcoded inside.']
			],
			alt: 'quack desktop app Choose a model dialog open over four pinned projects, showing active dev-local/tiny at ~43 tok/s alongside Plan mode architect dev-local/thinker'
		},
		{
			id: 'providers',
			tab: 'Provider setup',
			short: 'Local servers & API keys',
			title: 'Discover local servers or wire up providers',
			summary:
				'Scan this machine for running local servers without sending inference requests, store API keys in the OS credential store, sign in to subscriptions, or add any compatible URL.',
			points: [
				['6 local server backends', 'Finds llama.cpp, Ollama, vLLM, SGLang, LM Studio, and Docker Model Runner on loopback without model requests.'],
				['OS credential storage', 'Hosted keys live in the OS credential store or a private key file; ChatGPT plans authenticate via browser OAuth in setup.'],
				['Connect by URL', 'Add any OpenAI-compatible, Responses, Anthropic, Gemini, or Cohere gateway endpoint directly from setup.']
			],
			alt: 'quack desktop app Providers setup page showing available dev-local models, Find local servers button, Hosted provider API key form with OS credential store note, and Connect by URL fields'
		},
		{
			id: 'egress',
			tab: 'Network & sandbox',
			short: 'Shield, ledger & sandbox',
			title: 'Audit every HTTP hop, secret guard, and sandbox bound',
			summary:
				'Click the header Shield / Offline pill for the two-tab Boundary Passport: configure Policy & Secret Shield, audit the live Session Egress Ledger, and check OS command sandboxing.',
			points: [
				['Pre-Egress Secret Shield', 'Choose ask, redact, or off to scan outbound prompts, @ slices, and tool results before they leave your machine, manage SHA-256 allowlists, and choose whether loopback is scanned.'],
				['Live Session Egress Ledger', 'Inspect every outbound HTTP call, redirect check, byte count, and blocked hop across inference, MCP, OAuth, catalog, and update checks.'],
				['Strict offline & OS sandbox', 'Restrict hosts to loopback and egress_allow, keep background checks Off by default, and inspect active OS command isolation bounds.']
			],
			alt: 'quack desktop app Network & Sandbox Boundary dialog showing Strict offline mode ON, dev-local localhost:11434 Local / Loopback destination, catalog/update/OpenTelemetry Off, and active Command Sandbox status'
		},
		{
			id: 'memory',
			tab: 'Memory notes',
			short: 'Shared TUI & GUI Markdown',
			title: 'Browse and edit shared Markdown memory notes',
			summary:
				'Review, filter, copy, or edit project and global Markdown memory notes stored in the same paths the terminal CLI reads.',
			points: [
				['Project & global scopes', 'Filter across All, Project, or Global notes alongside the live memory index budget meter.'],
				['Rendered or raw Markdown', 'Switch between rendered Markdown and raw text, create new notes, or copy paths directly.'],
				['Shared with the terminal', 'Reads and saves the exact same local Markdown note directories across TUI and GUI sessions.']
			],
			alt: 'quack desktop app Memory modal showing 1% index fill, project and global scope filters, rust-conventions note rendered as Markdown, and Edit/Copy path actions'
		},
		{
			id: 'settings',
			tab: 'Settings',
			short: 'Layered TOML & extensions',
			title: 'Configure layered TOML, guided hooks, and MCP servers',
			summary:
				'Edit paths, model defaults, context, network, sandbox, guided Hooks & MCP servers, and tab restoration in one UI, saving cleanly to user or project config.',
			points: [
				['Explicit save layer', 'Choose whether changes write to user config.toml or workspace .quack/config.toml, with layer badges and tab-restore controls.'],
				['Guided Hook setup & test', 'Configure bundled forge-review and jira hooks with masked credentials and run live draft connection checks from the form.'],
				['Live MCP lifecycle', 'Enable, add, inspect capabilities, or toggle stdio/HTTP MCP servers live without restarting active sessions.']
			],
			alt: 'quack desktop app Settings page with Save to User config or Project (.quack) toggle, filter box, seven-section rail, Setup & Models controls with layer badges, Reload config, and Done'
		}
	];

	let selectedId = $state<ScreenId>('approval');
	let previewTheme = $state<PreviewTheme>(null);
	let dialogEl: HTMLDialogElement | undefined = $state();
	let closeBtnEl: HTMLButtonElement | undefined = $state();
	let tabRefs: (HTMLButtonElement | undefined)[] = $state([]);

	const activeIndex = $derived(screens.findIndex((s) => s.id === selectedId));
	const active = $derived(screens[activeIndex] ?? screens[0]);

	function stepScreen(delta: number) {
		const next = (activeIndex + delta + screens.length) % screens.length;
		selectedId = screens[next].id;
	}

	function selectTab(index: number) {
		selectedId = screens[index].id;
		tabRefs[index]?.focus();
	}

	function onTabKeydown(e: KeyboardEvent, index: number) {
		if (e.key === 'ArrowRight') {
			e.preventDefault();
			selectTab((index + 1) % screens.length);
		} else if (e.key === 'ArrowLeft') {
			e.preventDefault();
			selectTab((index - 1 + screens.length) % screens.length);
		} else if (e.key === 'Home') {
			e.preventDefault();
			selectTab(0);
		} else if (e.key === 'End') {
			e.preventDefault();
			selectTab(screens.length - 1);
		}
	}

	function openLightbox() {
		dialogEl?.showModal();
		closeBtnEl?.focus();
	}

	function closeLightbox() {
		dialogEl?.close();
	}

	function onDialogKeydown(e: KeyboardEvent) {
		if (e.key === 'ArrowLeft') {
			e.preventDefault();
			stepScreen(-1);
		} else if (e.key === 'ArrowRight') {
			e.preventDefault();
			stepScreen(1);
		}
	}

	function onDialogClick(e: MouseEvent) {
		if (e.target === dialogEl) closeLightbox();
	}

	const themeOptions: { value: PreviewTheme; shortLabel: string; restLabel: string; label: string }[] = [
		{ value: null, shortLabel: 'Match', restLabel: ' site', label: 'Match site' },
		{ value: 'light', shortLabel: 'Light', restLabel: '', label: 'Light' },
		{ value: 'dark', shortLabel: 'Dark', restLabel: '', label: 'Dark' }
	];
</script>

{#snippet screenImage(screen: Screen, eager: boolean, cls: string)}
	{#if previewTheme === null}
		<img
			src="/assets/gui/{screen.id}-light.webp"
			alt={screen.alt}
			width="2640"
			height="1880"
			loading={eager ? 'eager' : 'lazy'}
			decoding="async"
			class="{cls} dark:hidden"
		/>
		<img
			src="/assets/gui/{screen.id}-dark.webp"
			alt={screen.alt}
			width="2640"
			height="1880"
			loading={eager ? 'eager' : 'lazy'}
			decoding="async"
			class="{cls} hidden dark:block"
		/>
	{:else}
		<img
			src="/assets/gui/{screen.id}-{previewTheme}.webp"
			alt={screen.alt}
			width="2640"
			height="1880"
			loading={eager ? 'eager' : 'lazy'}
			decoding="async"
			class={cls}
		/>
	{/if}
{/snippet}

<Plate id="desktop" color="var(--color-duck)">
	<PlateHeading title="The same engine, in a window">
		Prefer a native window for reviewing diffs, secret alerts, and parallel repositories? <code class="text-[0.95em]">quack-gui</code>
		wraps the exact same <code class="text-[0.95em]">quack-core</code> engine in Tauri 2 over local IPC —
		complete with a <code class="text-[0.95em]">Cmd/Ctrl-K</code> command palette and live egress ledger, and
		no background server port or telemetry on Linux, macOS, and Windows.
	</PlateHeading>

	<div class="flex flex-wrap items-center justify-between gap-3 mb-5">
		<div class="flex flex-wrap items-center gap-2">
			<span class="text-xs font-semibold uppercase tracking-wider text-ink-2">GUI theme</span>
			<div
				class="inline-flex items-center gap-0.5 sm:gap-1 rounded-full border border-rule bg-wash p-1"
				role="group"
				aria-label="Desktop screenshot theme"
			>
				{#each themeOptions as opt (opt.label)}
					<button
						type="button"
						aria-label={opt.label}
						aria-pressed={previewTheme === opt.value}
						onclick={() => (previewTheme = opt.value)}
						class="inline-flex items-center gap-1 sm:gap-1.5 px-2.5 sm:px-3 py-1 rounded-full text-xs sm:text-sm font-semibold whitespace-nowrap transition-colors {previewTheme === opt.value
							? 'bg-ink text-paper'
							: 'text-ink-2 hover:text-ink hover:bg-paper/60'}"
					>
						{#if opt.value === null}
							<Monitor class="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
						{:else if opt.value === 'light'}
							<Sun class="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
						{:else}
							<Moon class="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
						{/if}
						<span>{opt.shortLabel}<span class="hidden sm:inline">{opt.restLabel}</span></span>
					</button>
				{/each}
			</div>
		</div>

		<button
			type="button"
			onclick={openLightbox}
			class="inline-flex items-center gap-1.5 rounded-full border border-rule bg-paper px-3.5 py-1.5 text-sm font-semibold text-ink hover:bg-wash hover:border-ink/40 transition-colors"
		>
			<Maximize2 class="w-4 h-4" aria-hidden="true" />
			Inspect full size
		</button>
	</div>

	<div class="grid grid-cols-1 lg:grid-cols-[minmax(0,1.75fr)_minmax(0,1fr)] gap-7 lg:gap-8 items-start">
		<figure>
			<button
				type="button"
				onclick={openLightbox}
				aria-label="Inspect full-size screenshot: {active.title}"
				class="block w-full text-left rounded-xl border-2 border-ink overflow-hidden bg-term shadow-[0_18px_40px_-18px_rgba(29,42,58,0.55)] cursor-zoom-in transition-transform hover:-translate-y-0.5"
			>
				{@render screenImage(active, true, 'block w-full h-auto aspect-[1320/940] object-cover')}
			</button>
			<figcaption class="mt-2.5 flex flex-wrap items-center justify-between gap-2 text-xs text-ink-2">
				<span>Captured from the built-in scripted demo in <code class="text-ink">quack-gui</code>.</span>
				<span>Click screenshot to zoom</span>
			</figcaption>
		</figure>

		<div
			id="desktop-panel-{active.id}"
			role="tabpanel"
			aria-labelledby="desktop-tab-{active.id}"
			class="flex flex-col"
		>
			<p class="text-xs font-semibold uppercase tracking-wider text-ink-2 tabular-nums">
				Screen {activeIndex + 1} of {screens.length}
			</p>
			<h3 class="mt-1 font-display text-2xl sm:text-[1.65rem] font-bold leading-snug text-ink">
				{active.title}
			</h3>
			<p class="mt-2.5 text-ink-2 leading-relaxed">
				{active.summary}
			</p>

			<ul class="mt-5 space-y-3 border-t border-rule pt-4">
				{#each active.points as [label, desc]}
					<li class="flex items-start gap-2.5 text-sm leading-relaxed">
						<Check class="w-4 h-4 mt-1 shrink-0 text-sky-ink" aria-hidden="true" />
						<span class="text-ink-2">
							<strong class="font-semibold text-ink">{label}:</strong>
							{desc}
						</span>
					</li>
				{/each}
			</ul>
		</div>
	</div>

	<div
		class="mt-6 grid grid-cols-2 lg:grid-cols-4 gap-3 sm:gap-4"
		role="tablist"
		aria-label="Desktop app screenshots"
	>
		{#each screens as screen, index (screen.id)}
			{@const selected = selectedId === screen.id}
			<button
				bind:this={tabRefs[index]}
				id="desktop-tab-{screen.id}"
				type="button"
				role="tab"
				tabindex={selected ? 0 : -1}
				aria-selected={selected}
				aria-controls="desktop-panel-{screen.id}"
				onclick={() => (selectedId = screen.id)}
				onkeydown={(e) => onTabKeydown(e, index)}
				class="group text-left rounded-xl p-2 sm:p-2.5 border transition-colors {selected
					? 'border-ink ring-1 ring-ink bg-sky-soft'
					: 'border-rule bg-paper hover:border-ink/40 hover:bg-wash'}"
			>
				<div class="aspect-[1320/940] w-full overflow-hidden rounded-lg border border-rule bg-term">
					{@render screenImage(screen, false, 'block w-full h-full object-cover')}
				</div>
				<span class="mt-2.5 block font-display font-bold text-sm sm:text-base leading-snug text-ink">
					{screen.tab}
				</span>
				<span class="mt-0.5 block text-xs sm:text-[13px] leading-snug text-ink-2">
					{screen.short}
				</span>
			</button>
		{/each}
	</div>

	<p class="mt-6 rounded-xl border border-rule bg-wash px-4 py-3 text-sm text-ink-2 leading-relaxed">
		<strong class="font-semibold text-ink">Two apps sharing one core:</strong> installing
		<code>quack-gui</code> does not install the <code>quack</code> terminal command (and installing the
		TUI does not install the GUI), though both share one config, sessions, memory notes, and prompt
		history. Open workspace folders directly with <code>quack gui [PATH...]</code> (with single-instance
		tab forwarding and persistent tab restore) or type <code>/gui</code> inside the TUI to hand off.
		The default installer in
		<a href="#install" class="text-sky-ink underline underline-offset-2 font-semibold">Install</a> sets up
		both, or install the GUI alone with <code>--gui</code> (add <code>--plucked</code> to leave out
		hosted-service providers). Tagged GitHub releases also publish unsigned <code>.dmg</code>,
		<code>-setup.exe</code>, <code>.deb</code>, <code>.rpm</code>, and <code>.AppImage</code> packages
		sharing the same on-disk <code>defaults/</code> folder.
	</p>
</Plate>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<dialog
	bind:this={dialogEl}
	aria-label="Full-size desktop screenshot viewer"
	onkeydown={onDialogKeydown}
	onclick={onDialogClick}
	class="m-auto max-w-6xl w-[calc(100vw-1rem)] sm:w-[calc(100vw-3rem)] max-h-[calc(100vh-1rem)] overflow-y-auto rounded-2xl border-2 border-ink bg-paper text-ink p-3 sm:p-5 shadow-2xl backdrop:bg-night/80 backdrop:backdrop-blur-xs"
>
	<div class="flex flex-wrap items-center justify-between gap-3 pb-3 mb-3 border-b border-rule">
		<div class="min-w-0">
			<span class="text-xs font-semibold uppercase tracking-wider text-ink-2 tabular-nums">
				{activeIndex + 1} / {screens.length} · {active.tab}
			</span>
			<h3 class="font-display font-bold text-lg sm:text-xl leading-snug text-ink truncate">
				{active.title}
			</h3>
		</div>

		<div class="flex flex-wrap items-center gap-2">
			<div
				class="inline-flex items-center gap-0.5 rounded-full border border-rule bg-wash p-0.5"
				role="group"
				aria-label="Lightbox screenshot theme"
			>
				{#each themeOptions as opt (opt.label)}
					<button
						type="button"
						aria-label={opt.label}
						aria-pressed={previewTheme === opt.value}
						onclick={() => (previewTheme = opt.value)}
						class="inline-flex items-center gap-1 px-2.5 py-1 rounded-full text-xs font-semibold transition-colors {previewTheme === opt.value
							? 'bg-ink text-paper'
							: 'text-ink-2 hover:text-ink'}"
					>
						{#if opt.value === null}
							<Monitor class="w-3 h-3" aria-hidden="true" />
						{:else if opt.value === 'light'}
							<Sun class="w-3 h-3" aria-hidden="true" />
						{:else}
							<Moon class="w-3 h-3" aria-hidden="true" />
						{/if}
						<span>{opt.shortLabel}<span class="hidden sm:inline">{opt.restLabel}</span></span>
					</button>
				{/each}
			</div>

			<div class="inline-flex items-center gap-1">
				<button
					type="button"
					onclick={() => stepScreen(-1)}
					aria-label="Previous screenshot"
					class="p-1.5 rounded-lg border border-rule text-ink hover:bg-wash transition-colors"
				>
					<ChevronLeft class="w-4 h-4" aria-hidden="true" />
				</button>
				<button
					type="button"
					onclick={() => stepScreen(1)}
					aria-label="Next screenshot"
					class="p-1.5 rounded-lg border border-rule text-ink hover:bg-wash transition-colors"
				>
					<ChevronRight class="w-4 h-4" aria-hidden="true" />
				</button>
			</div>

			<button
				bind:this={closeBtnEl}
				type="button"
				onclick={closeLightbox}
				class="inline-flex items-center gap-1 rounded-lg border border-rule px-2.5 py-1.5 text-xs sm:text-sm font-semibold text-ink hover:bg-wash transition-colors"
			>
				<X class="w-4 h-4" aria-hidden="true" />
				Close
			</button>
		</div>
	</div>

	<div class="flex justify-center">
		{@render screenImage(
			active,
			true,
			'block max-w-full max-h-[74vh] w-auto h-auto rounded-xl border border-rule bg-term'
		)}
	</div>

	<p class="mt-3 text-sm text-ink-2 leading-relaxed">
		{active.summary}
	</p>
</dialog>
