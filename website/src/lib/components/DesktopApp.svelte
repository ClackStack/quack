<script lang="ts">
	import Plate from '#lib/components/Plate.svelte';
	import PlateHeading from '#lib/components/PlateHeading.svelte';
	import { Check, ChevronLeft, ChevronRight, Maximize2, Monitor, Moon, Sun, X } from '@lucide/svelte';

	type ScreenId = 'approval' | 'jobs' | 'workspace' | 'models';
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
			tab: 'Specimen approvals',
			short: 'Numbered diff plate',
			title: 'Approve file changes on a specimen plate',
			summary:
				'Writes and patches pause for review before touching disk. Circled field marks number every added or removed line alongside the files already edited in this job.',
			points: [
				['One-keystroke decisions', 'Approve a single write, allow it for this session, or reject cleanly with esc.'],
				['Session control strip', 'Switch Plan or Edit mode, permissions, and model right under the composer.'],
				['Live file ledger', 'Changed paths stack in the pond rail with an Undo last action ready beside them.']
			],
			alt: 'quack desktop app paused for approval on a specimen-plate diff of src/pond/ducks.ts, with numbered additions, Approve change, Allow for this session, Not this, and the session control strip below the composer'
		},
		{
			id: 'jobs',
			tab: 'Parallel jobs',
			short: 'Urgency queue across repos',
			title: 'Triage parallel jobs across repositories',
			summary:
				'Keep turns running across multiple repositories at once and switch from Projects to the live Jobs queue when a session needs input or hits an error.',
			points: [
				['Sorted by urgency', 'Waiting approvals and failures rise above running and ready conversations.'],
				['Sub-agent & plan status', 'Track active explore sub-agents, tool counts, and plan steps directly on each card.'],
				['Header waterline', 'A thin progress line under the workspace header traces streaming output while you read elsewhere.']
			],
			alt: 'quack desktop app Jobs tab triaging four jobs across demo-pond, quack, and orderly, showing an outside-sandbox approval, a provider timeout error, and an active turn with an explore sub-agent and plan progress bar'
		},
		{
			id: 'workspace',
			tab: 'Context & cache',
			short: 'Token meter & sparkline',
			title: 'Watch context pressure and prompt cache hits',
			summary:
				'Settled turns collapse thinking traces into an expandable toggle and plot engine metrics in the pond rail as they happen.',
			points: [
				['Pruning threshold tick', 'The context bar marks where automatic pruning kicks in so you know when to compact.'],
				['Per-call cache chart', 'Compare sent tokens, % served from cache, throughput, and time to first token.'],
				['Collapsible thoughts & tools', 'Inspect field thoughts and written lines inline without burying the conversation.']
			],
			alt: 'quack desktop app settled conversation showing collapsed field thoughts, written lines in src/pond/ducks.ts, 16% context meter with pruning tick, 80% cache hit usage sparkline at 43 tok/s, and Undo last'
		},
		{
			id: 'models',
			tab: 'Model picker',
			short: 'Local servers & providers',
			title: 'Switch local or API models mid-project',
			summary:
				'Keep multiple workspaces pinned in the pond rail and pick from discovered local servers or configured providers with full context sizes and reasoning levels.',
			points: [
				['Providers & limits', 'Groups every model by provider with context window size and supported thinking effort.'],
				['Per-job model choice', 'Run a deep reasoning model in one repo while another job stays on a fast local server.'],
				['Same TOML layers as CLI', 'Reads your existing provider files and local discovery with nothing hardcoded inside.']
			],
			alt: 'quack desktop app Choose a model dialog open over four pinned projects, listing dev-local/tiny with 33k ctx and dev-local/thinker with 131k ctx and thinking effort options'
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
		{ value: 'light', shortLabel: 'Day', restLabel: ' survey', label: 'Day survey' },
		{ value: 'dark', shortLabel: 'Night', restLabel: ' survey', label: 'Night survey' }
	];
</script>

{#snippet surveyImage(screen: Screen, eager: boolean, cls: string)}
	{#if previewTheme === null}
		<img
			src="/assets/gui/{screen.id}-light.webp"
			alt={screen.alt}
			width="2560"
			height="1720"
			loading={eager ? 'eager' : 'lazy'}
			decoding="async"
			class="{cls} dark:hidden"
		/>
		<img
			src="/assets/gui/{screen.id}-dark.webp"
			alt={screen.alt}
			width="2560"
			height="1720"
			loading={eager ? 'eager' : 'lazy'}
			decoding="async"
			class="{cls} hidden dark:block"
		/>
	{:else}
		<img
			src="/assets/gui/{screen.id}-{previewTheme}.webp"
			alt={screen.alt}
			width="2560"
			height="1720"
			loading={eager ? 'eager' : 'lazy'}
			decoding="async"
			class={cls}
		/>
	{/if}
{/snippet}

<Plate id="desktop" color="var(--color-duck)">
	<PlateHeading title="The same engine, in a window">
		Prefer a native window for reviewing diffs and juggling repos? <code class="text-[0.95em]">quack-gui</code>
		wraps the exact same <code class="text-[0.95em]">quack-core</code> engine with Tauri 2 and SvelteKit over
		local IPC — no background server port, no telemetry, on Linux, macOS and Windows.
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
				{@render surveyImage(active, true, 'block w-full h-auto aspect-[1280/860] object-cover')}
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
				<div class="aspect-[1280/860] w-full overflow-hidden rounded-lg border border-rule bg-term">
					{@render surveyImage(screen, false, 'block w-full h-full object-cover')}
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
		<strong class="font-semibold text-ink">Source preview:</strong> the release archives in
		<a href="#install" class="text-sky-ink underline underline-offset-2 font-semibold">Install</a> currently
		package the CLI. To run the desktop window today, build the UI bundle in
		<code>crates/quack-gui/ui</code> and run <code>cargo build -p quack-gui --features desktop</code>.
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
						<span class="hidden sm:inline">{opt.label}</span>
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
		{@render surveyImage(
			active,
			true,
			'block max-w-full max-h-[74vh] w-auto h-auto rounded-xl border border-rule bg-term'
		)}
	</div>

	<p class="mt-3 text-sm text-ink-2 leading-relaxed">
		{active.summary}
	</p>
</dialog>
