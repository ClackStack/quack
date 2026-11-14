<script lang="ts">
	import { onDestroy, onMount, tick } from 'svelte';
	import type { MemoryNote } from '#lib/protocol';
	import type { Session } from '#lib/session.svelte';
	import {
		MEMORY_KINDS,
		parseMemoryNote,
		slugifyMemoryName,
		sortMemoryNotes,
		validateMemoryDraft,
		type MemoryKind,
		type MemoryScope
	} from '#lib/memory';
	import AssistantMessage from './AssistantMessage.svelte';

	interface MemoryTarget {
		scope: string;
		name: string;
		create?: boolean;
	}

	let {
		session,
		initialTarget = null,
		onClose,
		onOpenSettings
	}: {
		session: Session;
		initialTarget?: MemoryTarget | null;
		onClose: () => void;
		onOpenSettings?: () => void;
	} = $props();

	let dialogEl: HTMLElement | undefined = $state();
	let nameInputEl: HTMLInputElement | undefined = $state();
	let previousFocus: HTMLElement | null = null;

	let query = $state('');
	let scopeFilter = $state<'all' | MemoryScope>('all');
	let selectedKey = $state<string | null>(null);
	let mode = $state<'view' | 'edit'>('view');
	let rawView = $state(false);
	let editorTab = $state<'write' | 'preview'>('write');

	let editOriginal = $state<{ scope: string; name: string } | null>(null);
	let baseline = $state({ scope: 'project', kind: 'project', name: '', description: '', body: '' });
	let draftScope = $state<MemoryScope>('project');
	let draftKind = $state<MemoryKind>('project');
	let draftName = $state('');
	let draftDescription = $state('');
	let draftBody = $state('');

	let formError = $state<string | null>(null);
	let confirmDelete = $state(false);
	let copiedField = $state<'path' | 'text' | null>(null);
	let pendingSave = $state<{ scope: string; name: string } | null>(null);
	let pendingDelete = $state(false);
	let pendingDiscard = $state<(() => void) | null>(null);

	function keyOf(note: Pick<MemoryNote, 'scope' | 'name'>): string {
		return `${note.scope}:${note.name}`;
	}

	const sortedNotes = $derived(sortMemoryNotes(session.memoryNotes));
	const projectCount = $derived(sortedNotes.filter((n) => n.scope === 'project').length);
	const globalCount = $derived(sortedNotes.filter((n) => n.scope === 'global').length);

	const filteredNotes = $derived.by(() => {
		const q = query.trim().toLowerCase();
		return sortedNotes.filter((note) => {
			if (scopeFilter !== 'all' && note.scope !== scopeFilter) return false;
			if (!q) return true;
			return (
				note.name.toLowerCase().includes(q) ||
				note.description.toLowerCase().includes(q) ||
				note.text.toLowerCase().includes(q)
			);
		});
	});

	const selectedNote = $derived(
		sortedNotes.find((n) => keyOf(n) === selectedKey) ?? filteredNotes[0] ?? sortedNotes[0] ?? null
	);
	const parsedSelected = $derived(selectedNote ? parseMemoryNote(selectedNote) : null);

	const isDirty = $derived(
		mode === 'edit' &&
			(draftScope !== baseline.scope ||
				draftKind !== baseline.kind ||
				draftName !== baseline.name ||
				draftDescription !== baseline.description ||
				draftBody !== baseline.body)
	);

	const targetPathPreview = $derived.by(() => {
		const slug = draftName.trim() || '<name>';
		return draftScope === 'project'
			? `.quack/memory/notes/${slug}.md`
			: `~/.local/share/quack/memory/notes/${slug}.md`;
	});

	const canSlugifyName = $derived.by(() => {
		const s = slugifyMemoryName(draftName);
		return draftName.length > 0 && s.length > 0 && s !== draftName;
	});

	const fillWarning = $derived(session.memoryIndexFill >= 80);

	onMount(() => {
		previousFocus = document.activeElement instanceof HTMLElement ? document.activeElement : null;
		session.clearMemoryError();
		void session.listMemory();
		if (initialTarget?.create) {
			startCreate(initialTarget.scope === 'global' ? 'global' : 'project');
		} else {
			if (initialTarget?.name) {
				selectedKey = `${initialTarget.scope}:${initialTarget.name}`;
			}
			dialogEl?.focus();
		}
	});

	onDestroy(() => {
		previousFocus?.focus?.();
	});

	$effect(() => {
		if (pendingSave && !session.memorySaving) {
			const saved = pendingSave;
			pendingSave = null;
			if (!session.memoryError) {
				selectedKey = `${saved.scope}:${saved.name}`;
				mode = 'view';
				rawView = false;
				formError = null;
			}
		}
		if (pendingDelete && !session.memorySaving) {
			pendingDelete = false;
			confirmDelete = false;
			if (!session.memoryError) {
				selectedKey = null;
				mode = 'view';
			}
		}
	});

	function guardUnsaved(action: () => void) {
		if (isDirty) {
			pendingDiscard = action;
		} else {
			action();
		}
	}

	function selectNote(note: MemoryNote) {
		const nextKey = keyOf(note);
		if (mode === 'view' && selectedKey === nextKey) return;
		guardUnsaved(() => {
			selectedKey = nextKey;
			mode = 'view';
			confirmDelete = false;
			formError = null;
			session.clearMemoryError();
		});
	}

	function startCreate(scope: MemoryScope = scopeFilter === 'global' ? 'global' : 'project') {
		guardUnsaved(() => {
			const kind: MemoryKind = scope === 'global' ? 'user' : 'project';
			editOriginal = null;
			draftScope = scope;
			draftKind = kind;
			draftName = '';
			draftDescription = '';
			draftBody = '';
			baseline = { scope, kind, name: '', description: '', body: '' };
			editorTab = 'write';
			formError = null;
			confirmDelete = false;
			pendingDiscard = null;
			session.clearMemoryError();
			mode = 'edit';
			void tick().then(() => nameInputEl?.focus());
		});
	}

	function startEdit(note: MemoryNote) {
		const parsed = parseMemoryNote(note);
		const scope: MemoryScope = parsed.scope === 'global' ? 'global' : 'project';
		const kind: MemoryKind = parsed.kind ?? (scope === 'global' ? 'user' : 'project');
		editOriginal = { scope: parsed.scope, name: parsed.name };
		draftScope = scope;
		draftKind = kind;
		draftName = parsed.name;
		draftDescription = parsed.description;
		draftBody = parsed.body;
		baseline = {
			scope,
			kind,
			name: parsed.name,
			description: parsed.description,
			body: parsed.body
		};
		editorTab = 'write';
		formError = null;
		confirmDelete = false;
		pendingDiscard = null;
		session.clearMemoryError();
		mode = 'edit';
		void tick().then(() => nameInputEl?.focus());
	}

	function cancelEdit() {
		guardUnsaved(() => {
			mode = 'view';
			formError = null;
			pendingDiscard = null;
			session.clearMemoryError();
		});
	}

	async function save() {
		if (session.memorySaving || session.closed) return;
		formError = null;
		session.clearMemoryError();
		const cleanName = draftName.trim();
		const cleanDesc = draftDescription.trim();
		const err = validateMemoryDraft(
			{
				scope: draftScope,
				name: cleanName,
				description: cleanDesc,
				kind: draftKind,
				body: draftBody
			},
			session.memoryNotes,
			editOriginal
		);
		if (err) {
			formError = err;
			return;
		}
		draftName = cleanName;
		draftDescription = cleanDesc;
		pendingSave = { scope: draftScope, name: cleanName };
		const sent = await session.saveMemory({
			scope: draftScope,
			name: cleanName,
			description: cleanDesc,
			kind: draftKind,
			body: draftBody.trim(),
			...(editOriginal ? { previousScope: editOriginal.scope, previousName: editOriginal.name } : {})
		});
		if (!sent) {
			pendingSave = null;
			formError = 'Could not send save request to the engine.';
		}
	}

	async function executeDelete() {
		if (!selectedNote || session.memorySaving || session.closed) return;
		formError = null;
		session.clearMemoryError();
		pendingDelete = true;
		const sent = await session.deleteMemory(selectedNote.scope, selectedNote.name);
		if (!sent) {
			pendingDelete = false;
			formError = 'Could not send delete request to the engine.';
		}
	}

	async function copyText(text: string, field: 'path' | 'text') {
		try {
			await navigator.clipboard?.writeText(text);
			copiedField = field;
			setTimeout(() => {
				if (copiedField === field) copiedField = null;
			}, 1600);
		} catch {
			/* clipboard optional in headless/browser */
		}
	}

	function closeWithGuard() {
		guardUnsaved(() => onClose());
	}

	function onWindowKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			event.preventDefault();
			if (pendingDiscard) {
				pendingDiscard = null;
				return;
			}
			if (confirmDelete) {
				confirmDelete = false;
				return;
			}
			if (mode === 'edit') {
				cancelEdit();
				return;
			}
			onClose();
			return;
		}
		if (mode === 'edit' && (event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 's') {
			event.preventDefault();
			void save();
		}
	}

	const activeError = $derived(formError ?? session.memoryError);
</script>

<svelte:window onkeydown={onWindowKeydown} />

<div
	class="backdrop"
	role="presentation"
	onclick={(e) => {
		if (e.target === e.currentTarget) closeWithGuard();
	}}
>
	<div
		class="modal"
		role="dialog"
		aria-modal="true"
		aria-labelledby="memory-modal-title"
		tabindex="-1"
		bind:this={dialogEl}
	>
		<div class="modal-head">
			<div class="head-title">
				<div class="title-row">
					<h2 id="memory-modal-title">Memory</h2>
					<span class="fill-chip" class:warn={fillWarning} title="Prompt index capacity used by the fuller scope">
						Index {session.memoryIndexFill}%
					</span>
				</div>
				<p class="sub">
					Shared Markdown notes read across TUI and GUI sessions for this project and global config.
				</p>
			</div>
			<div class="head-actions">
				{#if onOpenSettings}
					<button
						type="button"
						class="btn"
						disabled={session.closed || session.memorySaving}
						onclick={onOpenSettings}
						title="Configure memory.path, extraction settings, and storage directories"
					>
						Paths…
					</button>
				{/if}
				<button
					type="button"
					class="btn"
					disabled={session.closed || session.memorySaving}
					onclick={() => void session.listMemory()}
					title="Reload notes from disk"
				>
					Refresh
				</button>
				<button
					type="button"
					class="btn btn-primary"
					disabled={session.closed || session.memorySaving}
					onclick={() => startCreate()}
				>
					+ New note
				</button>
				<button
					type="button"
					class="close-btn"
					aria-label="Close memory modal"
					onclick={closeWithGuard}
				>
					×
				</button>
			</div>
		</div>

		{#if session.info?.paths}
			<div class="paths-strip">
				<span class="paths-item" title={session.info.paths.memory_project}>
					<span class="paths-k">Project</span>
					<span class="mono">{session.info.paths.memory_project}</span>
				</span>
				<span class="paths-item" title={session.info.paths.memory_global}>
					<span class="paths-k">Global</span>
					<span class="mono">{session.info.paths.memory_global}</span>
				</span>
				{#if onOpenSettings}
					<button type="button" class="link-btn" onclick={onOpenSettings}>
						Change directory…
					</button>
				{/if}
			</div>
		{/if}

		{#if fillWarning}
			<div class="fill-banner" role="status">
				<span>Memory index is {session.memoryIndexFill}% full — concise descriptions help keep every note indexed.</span>
			</div>
		{/if}

		{#if pendingDiscard}
			<div class="discard-bar" role="alert">
				<span>You have unsaved changes to this note.</span>
				<div class="discard-actions">
					<button type="button" class="btn" onclick={() => (pendingDiscard = null)}>
						Keep editing
					</button>
					<button
						type="button"
						class="btn btn-danger"
						onclick={() => {
							const fn = pendingDiscard;
							pendingDiscard = null;
							baseline = {
								scope: draftScope,
								kind: draftKind,
								name: draftName,
								description: draftDescription,
								body: draftBody
							};
							fn?.();
						}}
					>
						Discard changes
					</button>
				</div>
			</div>
		{/if}

		<div class="split">
			<aside class="nav" aria-label="Memory notes">
				<div class="nav-controls">
					<input
						type="search"
						class="search"
						placeholder="Filter notes…"
						bind:value={query}
						aria-label="Filter memory notes"
					/>
					<div class="scope-tabs" role="tablist" aria-label="Filter by memory scope">
						<button
							type="button"
							role="tab"
							aria-selected={scopeFilter === 'all'}
							class:active={scopeFilter === 'all'}
							onclick={() => (scopeFilter = 'all')}
						>
							All <span class="tab-n">{sortedNotes.length}</span>
						</button>
						<button
							type="button"
							role="tab"
							aria-selected={scopeFilter === 'project'}
							class:active={scopeFilter === 'project'}
							onclick={() => (scopeFilter = 'project')}
						>
							Project <span class="tab-n">{projectCount}</span>
						</button>
						<button
							type="button"
							role="tab"
							aria-selected={scopeFilter === 'global'}
							class:active={scopeFilter === 'global'}
							onclick={() => (scopeFilter = 'global')}
						>
							Global <span class="tab-n">{globalCount}</span>
						</button>
					</div>
				</div>

				<div class="note-list">
					{#if !session.memoryLoaded && sortedNotes.length === 0}
						<p class="list-empty">Loading notes…</p>
					{:else if filteredNotes.length === 0}
						<p class="list-empty">
							{sortedNotes.length === 0
								? 'No memory notes saved yet.'
								: 'No notes match this filter.'}
						</p>
					{:else}
						{#each filteredNotes as note (keyOf(note))}
							{@const parsed = parseMemoryNote(note)}
							{@const active = mode === 'view' && !!selectedNote && keyOf(note) === keyOf(selectedNote)}
							<button
								type="button"
								class="note-item"
								class:active
								onclick={() => selectNote(note)}
							>
								<div class="item-top">
									<span class="item-name">{note.name}</span>
									<span class="scope-tag" class:project={note.scope === 'project'}>
										{note.scope}
									</span>
								</div>
								{#if note.description}
									<span class="item-desc">{note.description}</span>
								{/if}
								{#if parsed.kind}
									<span class="item-kind">{parsed.kind}</span>
								{/if}
							</button>
						{/each}
					{/if}
				</div>
			</aside>

			<section class="detail">
				{#if activeError}
					<div class="error-bar" role="alert">
						<span>{activeError}</span>
						<button
							type="button"
							class="btn"
							onclick={() => {
								formError = null;
								session.clearMemoryError();
							}}
						>
							Dismiss
						</button>
					</div>
				{/if}

				{#if mode === 'edit'}
					<div class="editor">
						<div class="detail-head">
							<div>
								<h3 class="detail-title">
									{editOriginal ? `Edit ${editOriginal.name}` : 'New memory note'}
								</h3>
								<p class="detail-path mono">{targetPathPreview}</p>
							</div>
							<div class="editor-mode-tabs" role="tablist" aria-label="Markdown editor view">
								<button
									type="button"
									role="tab"
									aria-selected={editorTab === 'write'}
									class:active={editorTab === 'write'}
									onclick={() => (editorTab = 'write')}
								>
									Write
								</button>
								<button
									type="button"
									role="tab"
									aria-selected={editorTab === 'preview'}
									class:active={editorTab === 'preview'}
									onclick={() => (editorTab = 'preview')}
								>
									Preview
								</button>
							</div>
						</div>

						<div class="field-row">
							<label class="field">
								<span class="f-label">Scope</span>
								<select bind:value={draftScope} disabled={session.memorySaving}>
									<option value="project">Project (this workspace)</option>
									<option value="global">Global (all workspaces)</option>
								</select>
							</label>
							<label class="field">
								<span class="f-label">Type</span>
								<select bind:value={draftKind} disabled={session.memorySaving}>
									{#each MEMORY_KINDS as kind (kind.id)}
										<option value={kind.id}>{kind.label}</option>
									{/each}
								</select>
							</label>
						</div>
						<p class="kind-hint">
							{MEMORY_KINDS.find((k) => k.id === draftKind)?.summary}
						</p>

						<label class="field">
							<span class="f-label-row">
								<span class="f-label">Name <span class="hint-mono">(kebab-case)</span></span>
								{#if canSlugifyName}
									<button
										type="button"
										class="slug-btn"
										onclick={() => (draftName = slugifyMemoryName(draftName))}
									>
										Slugify -> {slugifyMemoryName(draftName)}
									</button>
								{/if}
							</span>
							<input
								bind:this={nameInputEl}
								type="text"
								class="mono-input"
								placeholder="e.g. rust-conventions"
								bind:value={draftName}
								disabled={session.memorySaving}
							/>
						</label>

						<label class="field">
							<span class="f-label">Description <span class="hint-mono">(shown in prompt index)</span></span>
							<input
								type="text"
								placeholder="One-line summary of when or why this note matters"
								bind:value={draftDescription}
								disabled={session.memorySaving}
							/>
						</label>

						<div class="body-field">
							<span class="f-label">Body (Markdown)</span>
							{#if editorTab === 'write'}
								<textarea
									class="body-editor"
									placeholder="Write durable facts, conventions, or pointers in Markdown…"
									bind:value={draftBody}
									disabled={session.memorySaving}
								></textarea>
							{:else}
								<div class="preview-box">
									{#if draftBody.trim()}
										<AssistantMessage text={draftBody} streaming={false} />
									{:else}
										<p class="preview-empty">Nothing to preview yet.</p>
									{/if}
								</div>
							{/if}
						</div>

						<div class="detail-foot">
							<span class="kbd-hint"><span class="kbd">Ctrl/⌘ S</span> save · <span class="kbd">esc</span> cancel</span>
							<div class="grow"></div>
							<button
								type="button"
								class="btn"
								disabled={session.memorySaving}
								onclick={cancelEdit}
							>
								Cancel
							</button>
							<button
								type="button"
								class="btn btn-primary"
								disabled={session.closed || session.memorySaving}
								onclick={() => void save()}
							>
								{session.memorySaving ? 'Saving…' : 'Save note'}
							</button>
						</div>
					</div>
				{:else if parsedSelected && selectedNote}
					<div class="viewer">
						<div class="detail-head">
							<div class="v-meta">
								<div class="v-badges">
									<span class="scope-tag" class:project={parsedSelected.scope === 'project'}>
										{parsedSelected.scope}
									</span>
									{#if parsedSelected.kind}
										<span class="kind-tag">{parsedSelected.kind}</span>
									{/if}
									{#if parsedSelected.updated}
										<span class="date-tag">updated {parsedSelected.updated}</span>
									{/if}
								</div>
								<h3 class="detail-title">{parsedSelected.name}</h3>
								<p class="detail-desc">{parsedSelected.description}</p>
								<div class="path-row">
									<span class="detail-path mono" title={parsedSelected.path}>{parsedSelected.path}</span>
									<button
										type="button"
										class="mini-copy"
										onclick={() => void copyText(parsedSelected.path, 'path')}
									>
										{copiedField === 'path' ? 'Copied' : 'Copy path'}
									</button>
								</div>
							</div>
						</div>

						<div class="viewer-toolbar">
							<div class="view-tabs" role="tablist" aria-label="Note display format">
								<button
									type="button"
									role="tab"
									aria-selected={!rawView}
									class:active={!rawView}
									onclick={() => (rawView = false)}
								>
									Rendered
								</button>
								<button
									type="button"
									role="tab"
									aria-selected={rawView}
									class:active={rawView}
									onclick={() => (rawView = true)}
								>
									Raw
								</button>
							</div>
							<button
								type="button"
								class="btn"
								onclick={() => void copyText(rawView ? parsedSelected.raw : parsedSelected.body, 'text')}
							>
								{copiedField === 'text' ? 'Copied' : 'Copy'}
							</button>
							<div class="grow"></div>
							{#if confirmDelete}
								<span class="del-confirm">
									Delete this note?
									<button
										type="button"
										class="btn"
										disabled={session.memorySaving}
										onclick={() => (confirmDelete = false)}
									>
										Cancel
									</button>
									<button
										type="button"
										class="btn btn-danger"
										disabled={session.memorySaving || session.closed}
										onclick={() => void executeDelete()}
									>
										{session.memorySaving ? 'Deleting…' : 'Confirm delete'}
									</button>
								</span>
							{:else}
								<button
									type="button"
									class="btn"
									disabled={session.closed || session.memorySaving}
									onclick={() => (confirmDelete = true)}
								>
									Delete
								</button>
								<button
									type="button"
									class="btn btn-primary"
									disabled={session.closed || session.memorySaving}
									onclick={() => startEdit(selectedNote)}
								>
									Edit note
								</button>
							{/if}
						</div>

						<div class="note-content">
							{#if rawView}
								<pre class="raw-note">{parsedSelected.raw}</pre>
							{:else if parsedSelected.body}
								<div class="note-prose">
									<AssistantMessage text={parsedSelected.body} streaming={false} />
								</div>
							{:else}
								<p class="preview-empty">This note has no Markdown body yet.</p>
							{/if}
						</div>
					</div>
				{:else}
					<div class="empty-stage">
						<p class="empty-title">No memory note selected</p>
						<p class="empty-copy">
							Memory notes live in Markdown files under <span class="mono">.quack/memory/notes/</span> (project)
							and <span class="mono">quack/memory/notes/</span> (global), shared automatically between the TUI and GUI.
						</p>
						<button
							type="button"
							class="btn btn-primary"
							disabled={session.closed}
							onclick={() => startCreate()}
						>
							+ Create first note
						</button>
					</div>
				{/if}
			</section>
		</div>
	</div>
</div>

<style>
	.backdrop {
		position: fixed;
		inset: 0;
		z-index: 60;
		background: var(--scrim);
		display: grid;
		place-items: center;
		padding: var(--sp-md);
	}

	.modal {
		width: min(920px, 100%);
		height: min(680px, calc(100vh - 32px));
		display: flex;
		flex-direction: column;
		background: var(--bg-raised);
		border: 1px solid var(--plate-line);
		border-radius: var(--r-xl);
		box-shadow: var(--shadow-pop);
		overflow: hidden;
		outline: none;
	}

	.modal-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: var(--sp-md);
		padding: var(--sp-md) var(--sp-md) 12px;
		border-bottom: 1px solid var(--rule-soft);
		background: var(--bg-panel);
	}

	.title-row {
		display: flex;
		align-items: center;
		gap: 10px;
	}

	.modal-head h2 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 1.1875rem;
		font-weight: 600;
		color: var(--ink);
	}

	.fill-chip {
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		padding: 2px 8px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		background: var(--bg-soft);
		color: var(--ink-2);
	}

	.fill-chip.warn {
		border-color: var(--duck-line);
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.sub {
		margin: 4px 0 0;
		font-size: 0.75rem;
		color: var(--ink-3);
	}

	.paths-strip {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px 14px;
		padding: 6px var(--sp-md);
		background: var(--bg-soft);
		border-bottom: 1px solid var(--rule-soft);
		font-size: 0.71875rem;
		color: var(--ink-2);
	}

	.paths-item {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		min-width: 0;
		max-width: 100%;
	}

	.paths-k {
		font-size: 0.65625rem;
		font-weight: 600;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--ink-3);
		flex: none;
	}

	.paths-item .mono {
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		color: var(--ink);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.link-btn {
		margin-left: auto;
		font-size: 0.71875rem;
		font-weight: 500;
		color: var(--beak-ink);
		text-decoration: underline;
		text-underline-offset: 2px;
	}

	.link-btn:hover {
		color: var(--ink);
	}

	.head-actions {
		display: flex;
		align-items: center;
		gap: 8px;
		flex: none;
	}

	.close-btn {
		width: 1.75rem;
		height: 1.75rem;
		border-radius: var(--r-md);
		color: var(--ink-3);
		font-size: 1.125rem;
		line-height: 1;
		display: grid;
		place-items: center;
	}

	.close-btn:hover {
		background: var(--bg-soft);
		color: var(--ink);
	}

	.fill-banner,
	.discard-bar,
	.error-bar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 12px;
		padding: 8px var(--sp-md);
		font-size: 0.78125rem;
		border-bottom: 1px solid var(--rule-soft);
	}

	.fill-banner {
		background: var(--duck-soft);
		color: var(--duck-ink);
	}

	.discard-bar {
		background: var(--duck-soft);
		color: var(--ink);
	}

	.discard-actions {
		display: flex;
		gap: 8px;
		flex: none;
	}

	.error-bar {
		background: var(--fail-soft);
		color: var(--fail);
		border-radius: var(--r-md);
		border: 1px solid color-mix(in srgb, var(--fail) 30%, transparent);
		padding: 8px 12px;
	}

	.split {
		flex: 1;
		min-height: 0;
		display: grid;
		grid-template-columns: minmax(230px, 275px) minmax(0, 1fr);
	}

	.nav {
		min-height: 0;
		display: flex;
		flex-direction: column;
		border-right: 1px solid var(--rule-soft);
		background: var(--bg-panel);
	}

	.nav-controls {
		padding: 10px 12px;
		display: grid;
		gap: 8px;
		border-bottom: 1px solid var(--rule-soft);
	}

	.search,
	.field input,
	.field select,
	.body-editor {
		width: 100%;
		padding: 7px 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		background-color: var(--bg-raised);
		color: var(--ink);
		font: inherit;
		font-size: 0.8125rem;
	}

	.search,
	.field input,
	.field select {
		min-height: 2.125rem;
		padding: 6px 10px;
		line-height: 1.4;
	}

	.field select {
		padding-right: 30px;
	}

	.search:focus,
	.field input:focus,
	.field select:focus,
	.body-editor:focus {
		outline: 2px solid color-mix(in srgb, var(--sky) 40%, transparent);
		border-color: var(--sky-ink);
	}

	.scope-tabs,
	.editor-mode-tabs,
	.view-tabs {
		display: inline-flex;
		padding: 2px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule-soft);
		background: var(--bg-soft);
		gap: 2px;
	}

	.scope-tabs {
		display: grid;
		grid-template-columns: repeat(3, 1fr);
	}

	.scope-tabs button,
	.editor-mode-tabs button,
	.view-tabs button {
		padding: 4px 9px;
		border-radius: var(--r-sm);
		font-size: 0.75rem;
		font-weight: 500;
		color: var(--ink-2);
	}

	.scope-tabs button.active,
	.editor-mode-tabs button.active,
	.view-tabs button.active {
		background: var(--bg-raised);
		color: var(--ink);
		box-shadow: 0 1px 2px rgba(0, 0, 0, 0.06);
	}

	.tab-n {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		color: var(--ink-3);
	}

	.note-list {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 8px;
		display: grid;
		gap: 4px;
		align-content: start;
	}

	.list-empty {
		margin: 16px 8px;
		font-size: 0.78125rem;
		color: var(--ink-3);
		text-align: center;
	}

	.note-item {
		width: 100%;
		text-align: left;
		padding: 9px 10px;
		border-radius: var(--r-md);
		border: 1px solid transparent;
		display: grid;
		gap: 4px;
		color: var(--ink);
	}

	.note-item:hover {
		background: var(--bg-soft);
	}

	.note-item.active {
		background: var(--bg-raised);
		border-color: var(--sky-ink);
		box-shadow: 0 1px 3px rgba(0, 0, 0, 0.05);
	}

	.item-top {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.item-name {
		font-family: var(--font-mono);
		font-size: 0.78125rem;
		font-weight: 600;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.item-desc {
		font-size: 0.75rem;
		color: var(--ink-2);
		display: -webkit-box;
		-webkit-line-clamp: 2;
		line-clamp: 2;
		-webkit-box-orient: vertical;
		overflow: hidden;
	}

	.item-kind {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		color: var(--ink-3);
	}

	.scope-tag,
	.kind-tag,
	.date-tag {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		padding: 1px 6px;
		border-radius: 999px;
		border: 1px solid var(--rule-soft);
		background: var(--bg-soft);
		color: var(--ink-2);
		flex: none;
	}

	.scope-tag.project {
		border-color: color-mix(in srgb, var(--sky) 45%, transparent);
		background: color-mix(in srgb, var(--sky) 14%, transparent);
		color: var(--sky-ink);
	}

	.detail {
		min-width: 0;
		min-height: 0;
		padding: var(--sp-md);
		display: flex;
		flex-direction: column;
		gap: 12px;
		overflow: hidden;
	}

	.viewer,
	.editor {
		flex: 1;
		min-height: 0;
		display: flex;
		flex-direction: column;
		gap: 12px;
	}

	.detail-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		gap: 12px;
	}

	.v-meta {
		min-width: 0;
		display: grid;
		gap: 4px;
	}

	.v-badges {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}

	.detail-title {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 1.125rem;
		font-weight: 600;
		color: var(--ink);
		overflow-wrap: anywhere;
	}

	.detail-desc {
		margin: 0;
		font-size: 0.8125rem;
		color: var(--ink-2);
	}

	.path-row {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
	}

	.detail-path {
		margin: 0;
		font-size: 0.71875rem;
		color: var(--ink-3);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.mono,
	.mono-input {
		font-family: var(--font-mono);
	}

	.mini-copy,
	.slug-btn {
		font-size: 0.6875rem;
		font-family: var(--font-mono);
		color: var(--sky-ink);
		padding: 1px 6px;
		border-radius: var(--r-sm);
		border: 1px solid var(--rule-soft);
		background: var(--bg-soft);
		flex: none;
	}

	.mini-copy:hover,
	.slug-btn:hover {
		background: var(--bg-panel);
	}

	.viewer-toolbar {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
		padding-bottom: 10px;
		border-bottom: 1px solid var(--rule-soft);
	}

	.del-confirm {
		display: inline-flex;
		align-items: center;
		gap: 8px;
		font-size: 0.78125rem;
		color: var(--fail);
	}

	.grow {
		flex: 1;
	}

	.note-content,
	.preview-box {
		flex: 1;
		min-height: 0;
		overflow-y: auto;
		padding: 14px 16px;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-lg);
		background: var(--bg-panel);
	}

	.note-prose {
		font-size: 0.84375rem;
		line-height: 1.6;
		color: var(--ink);
	}

	.raw-note {
		margin: 0;
		font-family: var(--font-mono);
		font-size: 0.75rem;
		line-height: 1.55;
		color: var(--ink-2);
		white-space: pre-wrap;
		overflow-wrap: anywhere;
	}

	.field-row {
		display: grid;
		grid-template-columns: repeat(2, minmax(0, 1fr));
		gap: 10px;
	}

	.field,
	.body-field {
		display: grid;
		gap: 4px;
	}

	.body-field {
		flex: 1;
		min-height: 0;
		display: flex;
		flex-direction: column;
	}

	.f-label {
		font-size: 0.75rem;
		font-weight: 500;
		color: var(--ink-2);
	}

	.f-label-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
	}

	.hint-mono {
		font-family: var(--font-mono);
		font-size: 0.6875rem;
		font-weight: 400;
		color: var(--ink-3);
	}

	.kind-hint {
		margin: -6px 0 0;
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.body-editor {
		flex: 1;
		min-height: 160px;
		resize: none;
		font-family: var(--font-mono);
		font-size: 0.78125rem;
		line-height: 1.55;
	}

	.detail-foot {
		display: flex;
		align-items: center;
		gap: 8px;
		padding-top: 8px;
		border-top: 1px solid var(--rule-soft);
	}

	.kbd-hint {
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.btn-danger {
		background: var(--fail);
		color: #fff;
		border-color: var(--fail);
	}

	.btn-danger:hover:not(:disabled) {
		filter: brightness(0.92);
	}

	.empty-stage {
		flex: 1;
		display: grid;
		place-content: center;
		justify-items: center;
		text-align: center;
		gap: 10px;
		padding: var(--sp-md);
	}

	.empty-title {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 1.0625rem;
		font-weight: 600;
		color: var(--ink);
	}

	.empty-copy,
	.preview-empty {
		margin: 0;
		max-width: 44ch;
		font-size: 0.8125rem;
		color: var(--ink-3);
	}

	@media (max-width: 720px) {
		.modal {
			height: calc(100vh - 20px);
		}
		.split {
			grid-template-columns: 1fr;
			grid-template-rows: 220px minmax(0, 1fr);
		}
		.nav {
			border-right: none;
			border-bottom: 1px solid var(--rule-soft);
		}
	}
</style>
