<script lang="ts">
	import { tick } from 'svelte';
	import {
		BUILTIN_COMMANDS,
		executeBuiltinCommand,
		parseSlashCommand,
		type CommandUiHost
	} from '#lib/commands';
	import {
		initialHistoryCursor,
		isOnFirstLine,
		isOnLastLine,
		stepHistoryDown,
		stepHistoryUp,
		type HistoryCursor
	} from '#lib/inputHistory';
	import {
		APPROVALS,
		WORK_MODES,
		type ApprovalPolicy,
		type Session,
		type WorkMode
	} from '#lib/session.svelte';
	import { getTransport, type WorkspaceSymbol } from '#lib/transport';

	let {
		session,
		modalOpen = false,
		commandUiHost,
		onOpenModels,
		onOpenContext
	}: {
		session: Session;
		modalOpen?: boolean;
		commandUiHost?: CommandUiHost;
		onOpenModels: () => void;
		onOpenContext: () => void;
	} = $props();

	interface AutocompleteItem {
		id: string;
		label: string;
		badge: string;
		detail: string;
		args?: string;
		insert: string;
	}

	const transport = getTransport();
	const IMAGE_EXT = /\.(?:png|jpe?g|gif|webp)$/i;

	let editor: HTMLTextAreaElement | undefined = $state();
	let showWorkMode = $state(false);
	let showApprovals = $state(false);
	let caret = $state(0);
	let acIndex = $state(0);
	let dismissedAnchor = $state<string | null>(null);
	let attachedFiles = $state<string[]>([]);
	let attachedImages = $state<string[]>([]);
	let attachmentError = $state<string | null>(null);
	let dragOver = $state(false);
	let symbolCache = $state<Record<string, WorkspaceSymbol[]>>({});
	let symbolLoadingPath = $state<string | null>(null);
	let historyState = $state<{ tab: number; cursor: HistoryCursor }>({
		tab: -1,
		cursor: initialHistoryCursor()
	});
	const historyCursor = $derived(
		historyState.tab === session.tab ? historyState.cursor : initialHistoryCursor()
	);

	function setHistoryCursor(cursor: HistoryCursor) {
		historyState = { tab: session.tab, cursor };
	}

	const modelLabel = $derived(session.currentModel ?? 'no model');
	const workModeLabel = $derived(
		WORK_MODES.find((m) => m.id === session.workMode)?.label ?? session.workMode
	);
	const approvalsLabel = $derived(
		APPROVALS.find((a) => a.id === session.approvals)?.label ?? session.approvals
	);
	const thinkingLabel = $derived(session.reasoningEffort ?? 'default');
	const hasAttachImages = $derived(Boolean(transport.pickImages));
	const canSend = $derived(
		(session.draft.trim().length > 0 || attachedFiles.length > 0 || attachedImages.length > 0) &&
			session.canSubmit
	);
	// Legacy policies stay visible only while active (docs/plan/15-desktop-client.md).
	const approvalChoices = $derived(
		APPROVALS.filter((a) => !a.legacy || session.approvals === a.id)
	);

	const trigger = $derived.by(() => {
		const pos = Math.max(0, Math.min(caret, session.draft.length));
		const before = session.draft.slice(0, pos);
		const slash = /^\/([^\s]*)$/.exec(before);
		if (slash) {
			return {
				kind: 'slash' as const,
				query: slash[1].toLowerCase(),
				start: 0,
				end: pos,
				anchor: 'slash:0'
			};
		}
		const file = /(?:^|[\s(])@([^\s@]*)$/.exec(before);
		if (file) {
			const atIdx = before.lastIndexOf('@');
			const raw = file[1];
			const hashIdx = raw.indexOf('#');
			if (hashIdx > 0) {
				const filePath = raw.slice(0, hashIdx);
				return {
					kind: 'symbol' as const,
					path: filePath,
					query: raw.slice(hashIdx + 1).toLowerCase(),
					start: atIdx,
					end: pos,
					anchor: `sym:${atIdx}:${filePath}`
				};
			}
			return {
				kind: 'file' as const,
				query: raw.toLowerCase(),
				start: atIdx,
				end: pos,
				anchor: `file:${atIdx}`
			};
		}
		return null;
	});

	const suggestions = $derived.by((): AutocompleteItem[] => {
		if (!trigger || dismissedAnchor === trigger.anchor) return [];
		if (trigger.kind === 'slash') {
			const q = trigger.query;
			const seen: Record<string, true> = {};
			const all: AutocompleteItem[] = [];
			for (const b of BUILTIN_COMMANDS) {
				seen[b.name] = true;
				all.push({
					id: `builtin:${b.name}`,
					label: `/${b.name}`,
					badge: b.category.toLowerCase(),
					detail: b.description,
					args: b.args || undefined,
					insert: `/${b.name}${b.args ? ' ' : ''}`
				});
			}
			for (const c of session.commands) {
				if (seen[c.name]) continue;
				all.push({
					id: `cmd:${c.name}`,
					label: `/${c.name}`,
					badge: c.source || 'command',
					detail: c.description,
					args: c.args || undefined,
					insert: `/${c.name} `
				});
			}
			for (const s of session.skills) {
				all.push({
					id: `skill:${s.name}`,
					label: `/${s.name}`,
					badge: `skill · ${s.source}`,
					detail: s.description,
					insert: `/${s.name} `
				});
			}
			for (const p of session.mcpPrompts) {
				all.push({
					id: `mcp:${p.name}`,
					label: `/${p.name}`,
					badge: p.source || 'mcp',
					detail: p.description,
					args: p.args || undefined,
					insert: `/${p.name} `
				});
			}
			return all
				.filter(
					(it) =>
						!q ||
						it.label.toLowerCase().includes(q) ||
						it.detail.toLowerCase().includes(q)
				)
				.slice(0, 12);
		}

		if (trigger.kind === 'symbol') {
			const list = symbolCache[`${session.tab}:${trigger.path}`] ?? [];
			const q = trigger.query;
			const ranked: { sym: WorkspaceSymbol; rank: number }[] = [];
			for (const sym of list) {
				const low = sym.name.toLowerCase();
				if (!q) ranked.push({ sym, rank: 0 });
				else if (low.startsWith(q)) ranked.push({ sym, rank: 0 });
				else if (low.includes(q)) ranked.push({ sym, rank: 1 });
			}
			ranked.sort((a, b) => a.rank - b.rank || a.sym.start - b.sym.start);
			return ranked.slice(0, 14).map(({ sym }) => ({
				id: `sym:${trigger.path}#${sym.name}:${sym.start}`,
				label: `@${trigger.path}#${sym.name}`,
				badge: sym.kind,
				detail: `L${sym.start}-${sym.end}`,
				insert: `@${trigger.path}#${sym.name} `
			}));
		}

		const files = session.workspaceFiles ?? [];
		const q = trigger.query;
		const scored: { path: string; rank: number }[] = [];
		for (const path of files) {
			const low = path.toLowerCase();
			if (!q) {
				scored.push({ path, rank: 0 });
			} else {
				const base = low.slice(low.lastIndexOf('/') + 1);
				if (base.startsWith(q)) scored.push({ path, rank: 0 });
				else if (low.startsWith(q)) scored.push({ path, rank: 1 });
				else if (low.includes(q)) scored.push({ path, rank: 2 });
			}
		}
		scored.sort((a, b) => a.rank - b.rank || a.path.localeCompare(b.path));
		return scored.slice(0, 12).map(({ path }) => ({
			id: `file:${path}`,
			label: `@${path}`,
			badge: 'file',
			detail: 'tip: add #symbol or :L1-L40 for a slice',
			insert: `@${path} `
		}));
	});

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

	function syncCaret() {
		caret = editor?.selectionStart ?? session.draft.length;
	}

	function autogrow() {
		if (!editor) return;
		editor.style.height = 'auto';
		editor.style.height = `${Math.min(editor.scrollHeight, 240)}px`;
	}

	function onEditorInput() {
		setHistoryCursor(initialHistoryCursor());
		syncCaret();
		autogrow();
	}

	async function applySuggestion(item: AutocompleteItem) {
		if (!trigger) return;
		const before = session.draft.slice(0, trigger.start);
		const after = session.draft.slice(trigger.end);
		session.draft = `${before}${item.insert}${after}`;
		const nextPos = before.length + item.insert.length;
		caret = nextPos;
		dismissedAnchor = null;
		await tick();
		if (editor) {
			editor.focus();
			editor.setSelectionRange(nextPos, nextPos);
			autogrow();
		}
	}

	async function triggerAtMention() {
		const pos = editor?.selectionStart ?? session.draft.length;
		const before = session.draft.slice(0, pos);
		const after = session.draft.slice(pos);
		const needsSpace = before.length > 0 && !/[\s(]$/.test(before);
		const prefix = needsSpace ? ' @' : '@';
		session.draft = `${before}${prefix}${after}`;
		const nextPos = before.length + prefix.length;
		caret = nextPos;
		dismissedAnchor = null;
		void session.refreshWorkspaceFiles();
		await tick();
		if (editor) {
			editor.focus();
			editor.setSelectionRange(nextPos, nextPos);
			autogrow();
		}
	}

	function addUniqueFiles(paths: string[]) {
		const next = [...attachedFiles];
		for (const p of paths) {
			if (p && !next.includes(p)) next.push(p);
		}
		attachedFiles = next;
	}

	function addUniqueImages(paths: string[]) {
		const next = [...attachedImages];
		for (const p of paths) {
			if (p && !next.includes(p)) next.push(p);
		}
		attachedImages = next;
	}

	async function pickImages() {
		if (!transport.pickImages) return;
		attachmentError = null;
		try {
			const picked = await transport.pickImages();
			addUniqueImages(picked);
		} catch (e) {
			attachmentError = String(e);
		}
	}

	async function stageImageFile(file: File) {
		if (!transport.stagePastedImage) {
			attachmentError = 'Image paste/drop is only supported on the desktop app.';
			return;
		}
		attachmentError = null;
		try {
			const bytes = new Uint8Array(await file.arrayBuffer());
			const staged = await transport.stagePastedImage(session.tab, bytes);
			addUniqueImages([staged]);
		} catch (e) {
			attachmentError = String(e);
		}
	}

	function handleDroppedPaths(paths: string[]) {
		const cwd = session.info?.cwd?.replace(/[\\/]+$/, '');
		const filesToAdd: string[] = [];
		const imagesToAdd: string[] = [];
		for (const raw of paths) {
			if (IMAGE_EXT.test(raw)) {
				imagesToAdd.push(raw);
				continue;
			}
			if (cwd) {
				const normalizedRaw = raw.replace(/\\/g, '/');
				const normalizedCwd = cwd.replace(/\\/g, '/');
				if (normalizedRaw.startsWith(`${normalizedCwd}/`)) {
					filesToAdd.push(normalizedRaw.slice(normalizedCwd.length + 1));
					continue;
				}
			}
			attachmentError = 'Dropped non-image files must be inside the workspace folder.';
		}
		addUniqueImages(imagesToAdd);
		addUniqueFiles(filesToAdd);
	}

	async function onPaste(event: ClipboardEvent) {
		const items = event.clipboardData?.items;
		if (!items) return;
		const imageFiles: File[] = [];
		for (const item of items) {
			if (item.type.startsWith('image/')) {
				const file = item.getAsFile();
				if (file) imageFiles.push(file);
			}
		}
		if (imageFiles.length === 0) return;
		event.preventDefault();
		for (const file of imageFiles) {
			await stageImageFile(file);
		}
	}

	async function onDrop(event: DragEvent) {
		dragOver = false;
		const files = event.dataTransfer?.files;
		if (!files || files.length === 0) return;
		const images = [...files].filter((f) => f.type.startsWith('image/') || IMAGE_EXT.test(f.name));
		if (images.length === 0) return;
		event.preventDefault();
		for (const img of images) {
			await stageImageFile(img);
		}
	}

	async function send() {
		const draft = session.draft;
		const body = draft.trim();
		if (!canSend) return;
		const files = [...attachedFiles];
		const images = [...attachedImages];
		const parsedCmd = files.length === 0 && images.length === 0 ? parseSlashCommand(body) : null;
		if (parsedCmd && commandUiHost) {
			if (session.draft === draft) session.draft = '';
			setHistoryCursor(initialHistoryCursor());
			attachedFiles = [];
			attachedImages = [];
			attachmentError = null;
			caret = 0;
			autogrow();
			await executeBuiltinCommand(parsedCmd, session, commandUiHost);
			return;
		}
		if (await session.submit(body, files, images)) {
			if (session.draft === draft) session.draft = '';
			setHistoryCursor(initialHistoryCursor());
			attachedFiles = [];
			attachedImages = [];
			attachmentError = null;
			caret = 0;
			autogrow();
		}
	}

	async function applyHistoryStep(step: { draft: string; cursor: HistoryCursor; caret: number }) {
		setHistoryCursor(step.cursor);
		session.draft = step.draft;
		caret = step.caret;
		await tick();
		if (editor) {
			editor.setSelectionRange(step.caret, step.caret);
			autogrow();
		}
	}

	function onKeydown(event: KeyboardEvent) {
		if (suggestions.length > 0) {
			if (event.key === 'ArrowDown') {
				event.preventDefault();
				acIndex = (acIndex + 1) % suggestions.length;
				return;
			}
			if (event.key === 'ArrowUp') {
				event.preventDefault();
				acIndex = (acIndex - 1 + suggestions.length) % suggestions.length;
				return;
			}
			if (event.key === 'Tab' || (event.key === 'Enter' && !event.shiftKey && !event.isComposing)) {
				event.preventDefault();
				const chosen = suggestions[Math.min(acIndex, suggestions.length - 1)];
				if (chosen) void applySuggestion(chosen);
				return;
			}
			if (event.key === 'Escape' && trigger) {
				event.preventDefault();
				dismissedAnchor = trigger.anchor;
				return;
			}
		}
		if (
			(event.key === 'ArrowUp' || event.key === 'ArrowDown') &&
			!event.shiftKey &&
			!event.ctrlKey &&
			!event.metaKey &&
			!event.isComposing
		) {
			const start = editor?.selectionStart ?? session.draft.length;
			const end = editor?.selectionEnd ?? session.draft.length;
			if (event.key === 'ArrowUp' && (event.altKey || isOnFirstLine(session.draft, start, end))) {
				const step = stepHistoryUp(session.inputHistory, historyCursor, session.draft);
				if (step) {
					event.preventDefault();
					void applyHistoryStep(step);
					return;
				}
			} else if (
				event.key === 'ArrowDown' &&
				historyCursor.index !== null &&
				(event.altKey || isOnLastLine(session.draft, start, end))
			) {
				const step = stepHistoryDown(session.inputHistory, historyCursor);
				if (step) {
					event.preventDefault();
					void applyHistoryStep(step);
					return;
				}
			}
		}
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

	$effect(() => {
		if (!trigger) dismissedAnchor = null;
		else if (trigger.kind === 'file' && session.workspaceFiles === null && !session.loadingFiles && !session.closed) {
			void session.refreshWorkspaceFiles();
		} else if (trigger.kind === 'symbol' && !session.closed) {
			const path = trigger.path;
			const key = `${session.tab}:${path}`;
			if (!symbolCache[key] && symbolLoadingPath !== key) {
				symbolLoadingPath = key;
				void session
					.listSymbols(path, '')
					.then((syms) => {
						symbolCache = { ...symbolCache, [key]: syms };
					})
					.finally(() => {
						if (symbolLoadingPath === key) symbolLoadingPath = null;
					});
			}
		}
	});

	$effect(() => {
		void trigger?.anchor;
		void suggestions.length;
		if (acIndex >= suggestions.length) acIndex = 0;
	});

	// Dismiss popups when an approval or question plate takes over the room.
	$effect(() => {
		if (session.approval || session.questions || session.stepLimit) closeAllPopups();
	});

	$effect(() => {
		if (typeof window === 'undefined' || !('__TAURI_INTERNALS__' in window)) return;
		let active = true;
		let unlisten: (() => void) | undefined;
		void import('@tauri-apps/api/webview')
			.then(({ getCurrentWebview }) =>
				getCurrentWebview().onDragDropEvent((event) => {
					if (!active || modalOpen || session.closed) return;
					if (event.payload.type === 'enter' || event.payload.type === 'over') {
						dragOver = true;
					} else if (event.payload.type === 'leave') {
						dragOver = false;
					} else if (event.payload.type === 'drop') {
						dragOver = false;
						handleDroppedPaths(event.payload.paths);
					}
				})
			)
			.then((fn) => {
				if (active) unlisten = fn;
				else fn();
			})
			.catch(() => {});
		return () => {
			active = false;
			unlisten?.();
		};
	});

	// Keep the composer focused while nothing else asks for the keyboard; approvals
	// answer with Enter from anywhere.
	$effect(() => {
		if (
			!session.approval &&
			!session.questions &&
			!session.stepLimit &&
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

<svelte:window
	onclick={onWindowClick}
	onkeydown={onWindowKeydown}
	onfocus={() => void session.refreshInputHistory()}
/>

<div class="composer">
	<div
		class="box"
		role="region"
		aria-label="Message composer"
		class:drag-over={dragOver}
		ondragover={(e) => {
			if (e.dataTransfer?.types.includes('Files')) {
				e.preventDefault();
				dragOver = true;
			}
		}}
		ondragleave={() => (dragOver = false)}
		ondrop={(e) => void onDrop(e)}
	>
		{#if suggestions.length > 0 || (trigger?.kind === 'file' && session.loadingFiles) || (trigger?.kind === 'symbol' && symbolLoadingPath === `${session.tab}:${trigger.path}`)}
			<div class="ac-popup" role="listbox" aria-label="Suggestions">
				{#if trigger?.kind === 'file' && session.loadingFiles && suggestions.length === 0}
					<div class="ac-empty">Scanning workspace files…</div>
				{:else if trigger?.kind === 'symbol' && symbolLoadingPath === `${session.tab}:${trigger.path}` && suggestions.length === 0}
					<div class="ac-empty">Indexing symbols in {trigger.path}…</div>
				{:else}
					{#each suggestions as item, idx (item.id)}
						<button
							type="button"
							role="option"
							aria-selected={idx === acIndex}
							class="ac-item"
							class:active={idx === acIndex}
							onmousedown={(e) => {
								e.preventDefault();
								void applySuggestion(item);
							}}
						>
							<span class="ac-label">{item.label}{#if item.args} <span class="ac-args">{item.args}</span>{/if}</span>
							<span class="ac-badge">{item.badge}</span>
							{#if item.detail}<span class="ac-detail">{item.detail}</span>{/if}
						</button>
					{/each}
				{/if}
			</div>
		{/if}

		{#if attachedFiles.length > 0 || attachedImages.length > 0}
			<div class="attach-chips">
				{#each attachedFiles as file (file)}
					<span class="attach-chip">
						<span>@{file}</span>
						<button
							type="button"
							aria-label={`Remove ${file}`}
							onclick={() => (attachedFiles = attachedFiles.filter((f) => f !== file))}
						>
							×
						</button>
					</span>
				{/each}
				{#each attachedImages as image (image)}
					<span class="attach-chip" title={image}>
						<span>🖼 {image.split(/[\\/]/).pop() || image}</span>
						<button
							type="button"
							aria-label={`Remove image ${image}`}
							onclick={() => (attachedImages = attachedImages.filter((i) => i !== image))}
						>
							×
						</button>
					</span>
				{/each}
			</div>
		{/if}

		{#if attachmentError}
			<div class="attach-err" role="status">{attachmentError}</div>
		{/if}

		<textarea
			bind:this={editor}
			bind:value={session.draft}
			rows={1}
			placeholder={session.streaming || session.turnPending
				? 'quack is working — interrupt or wait…'
				: 'Ask quack… (@ for files, / for commands & skills, paste or drop images)'}
			onkeydown={onKeydown}
			oninput={onEditorInput}
			onclick={syncCaret}
			onkeyup={syncCaret}
			onpaste={(e) => void onPaste(e)}
			aria-label="Message"
			disabled={session.closed || session.sending || session.switching || session.starting}
		></textarea>
		<div class="row">
			<div class="attach-btns">
				<button
					type="button"
					class="mini-btn"
					disabled={session.closed || session.starting}
					title="Reference a workspace file with @"
					onclick={() => void triggerAtMention()}
				>
					@ File
				</button>
				{#if hasAttachImages}
					<button
						type="button"
						class="mini-btn"
						disabled={session.closed || session.starting}
						title="Attach an image file"
						onclick={() => void pickImages()}
					>
						+ Image
					</button>
				{/if}
			</div>
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
				disabled={!canSend}
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

		{#if session.info?.small_model}
			<button
				type="button"
				class="ctrl model-ctrl"
				aria-haspopup="dialog"
				title={`Small helper model: ${session.info.small_model}`}
				onclick={() => {
					closeAllPopups();
					onOpenModels();
				}}
			>
				<span class="ctrl-k">small</span>
				<span class="model-name">{session.info.small_model}</span>
			</button>
		{/if}

		{#if session.info?.planner_model}
			<button
				type="button"
				class="ctrl model-ctrl"
				class:pill-live={session.workMode === 'plan'}
				aria-haspopup="dialog"
				title={`Planner model used in Plan mode: ${session.info.planner_model}`}
				onclick={() => {
					closeAllPopups();
					onOpenModels();
				}}
			>
				<span class="ctrl-k">planner</span>
				<span class="model-name">{session.info.planner_model}</span>
			</button>
		{/if}

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
		position: relative;
		border: 1px solid var(--rule);
		border-radius: var(--r-xl);
		background: var(--bg-raised);
		box-shadow: var(--shadow-card);
		padding: 10px 12px 8px;
		display: grid;
		gap: 6px;
	}

	.box.drag-over {
		border-color: var(--sky);
		background: color-mix(in srgb, var(--sky-soft) 45%, var(--bg-raised));
	}

	.ac-popup {
		position: absolute;
		bottom: calc(100% + 8px);
		left: 0;
		right: 0;
		max-height: min(260px, 44vh);
		overflow-y: auto;
		overscroll-behavior: contain;
		background: var(--bg-raised);
		border: 1px solid var(--plate-line);
		border-radius: var(--r-lg);
		box-shadow: var(--shadow-pop);
		padding: 6px;
		z-index: 45;
		display: grid;
		gap: 3px;
	}

	.ac-empty {
		padding: 8px 10px;
		font-size: 0.75rem;
		color: var(--ink-3);
	}

	.ac-item {
		width: 100%;
		text-align: left;
		display: grid;
		grid-template-columns: minmax(0, 1fr) auto;
		gap: 2px 8px;
		padding: 6px 8px;
		border-radius: var(--r-md);
		border: 1px solid transparent;
	}

	.ac-item:hover,
	.ac-item.active {
		background: var(--sky-soft);
		border-color: color-mix(in srgb, var(--sky) 45%, var(--rule));
	}

	.ac-label {
		font-family: var(--font-mono);
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.ac-args {
		font-weight: 400;
		color: var(--ink-3);
	}

	.ac-badge {
		font-size: 0.65625rem;
		font-family: var(--font-mono);
		color: var(--sky-ink);
		align-self: center;
	}

	.ac-detail {
		grid-column: 1 / -1;
		font-size: 0.71875rem;
		color: var(--ink-3);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.attach-chips {
		display: flex;
		flex-wrap: wrap;
		gap: 5px;
	}

	.attach-chip {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		padding: 2px 7px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-panel);
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		color: var(--ink-2);
	}

	.attach-chip button {
		color: var(--ink-3);
		font-size: 0.8125rem;
		line-height: 1;
		padding: 0 2px;
	}

	.attach-chip button:hover {
		color: var(--fail);
	}

	.attach-err {
		font-size: 0.71875rem;
		color: var(--fail);
	}

	.attach-btns {
		display: inline-flex;
		align-items: center;
		gap: 5px;
		flex: none;
	}

	.mini-btn {
		min-height: 1.5rem;
		padding: 0 8px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-panel);
		color: var(--ink-2);
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		white-space: nowrap;
	}

	.mini-btn:hover:not(:disabled) {
		border-color: var(--sky);
		color: var(--ink);
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
		min-height: 1.5rem;
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
		font-size: 0.75rem;
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
		min-height: 1.625rem;
		padding: 0 10px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink-2);
		font-size: 0.75rem;
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
		font-size: 0.625rem;
		color: var(--ink-3);
		margin-left: 1px;
	}

	.model-ctrl {
		max-width: min(44vw, 320px);
		min-width: 0;
	}

	.model-name {
		font-family: var(--font-mono);
		font-size: 0.71875rem;
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
		font-size: 0.71875rem;
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
		font-size: 0.8125rem;
		font-weight: 600;
		color: var(--ink);
	}

	.check {
		color: var(--sky-ink);
		font-size: 0.75rem;
		font-weight: 700;
	}

	.choice-note {
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.popup-foot {
		font-size: 0.71875rem;
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
