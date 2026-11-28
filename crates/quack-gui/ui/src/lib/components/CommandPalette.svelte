<script lang="ts">
	import {
		BUILTIN_COMMANDS,
		executeBuiltinCommand,
		parseSlashCommand,
		type CommandUiHost,
		type ModalKind
	} from '#lib/commands';
	import type { Session } from '#lib/session.svelte';

	let {
		session,
		sessions,
		uiHost,
		onClose,
		onSelectJob
	}: {
		session: Session;
		sessions: Session[];
		uiHost: CommandUiHost;
		onClose: () => void;
		onSelectJob: (tab: number) => void;
	} = $props();

	interface PaletteEntry {
		id: string;
		label: string;
		category: string;
		description: string;
		badge?: string;
		shortcut?: string;
		searchText: string;
		run: () => void;
	}

	let query = $state('');
	let selectedIndex = $state(0);
	let inputEl: HTMLInputElement | undefined = $state();

	const quickUiHost: CommandUiHost = $derived({
		openModal(modal: ModalKind) {
			onClose();
			uiHost.openModal(modal);
		},
		openPalette() {
			// Already open inside CommandPalette.
		},
		openRewind() {
			onClose();
			uiHost.openRewind();
		},
		setTheme(mode?: string) {
			onClose();
			uiHost.setTheme(mode);
		}
	});

	function prefillDraft(text: string) {
		session.draft = text;
		onClose();
	}

	const entries = $derived.by((): PaletteEntry[] => {
		const trimmed = query.trim();
		const list: PaletteEntry[] = [];

		// If the user typed a built-in command with arguments (e.g. `/title Release prep`),
		// offer a direct top action to run it immediately with those arguments.
		if (trimmed.startsWith('/')) {
			const parsed = parseSlashCommand(trimmed);
			if (parsed && parsed.args) {
				list.push({
					id: `run-arg:${parsed.def.name}:${parsed.args}`,
					label: `/${parsed.def.name} ${parsed.args}`,
					category: 'Run with args',
					description: parsed.def.description,
					badge: 'execute',
					searchText: trimmed.toLowerCase(),
					run: () => {
						onClose();
						void executeBuiltinCommand(parsed, session, quickUiHost);
					}
				});
			}
		}

		for (const cmd of BUILTIN_COMMANDS) {
			if (cmd.name === 'help') continue;
			const hasRequiredArg = cmd.args.startsWith('<');
			list.push({
				id: `builtin:${cmd.name}`,
				label: `/${cmd.name}${cmd.args ? ` ${cmd.args}` : ''}`,
				category: cmd.category,
				description: cmd.description,
				shortcut: cmd.shortcut,
				searchText: `${cmd.name} ${cmd.aliases.join(' ')} ${cmd.description} ${cmd.category}`.toLowerCase(),
				run: () => {
					if (hasRequiredArg) {
						prefillDraft(`/${cmd.name} `);
						return;
					}
					onClose();
					void executeBuiltinCommand({ def: cmd, args: '' }, session, quickUiHost);
				}
			});
		}

		for (const cmd of session.commands) {
			list.push({
				id: `custom:${cmd.name}`,
				label: `/${cmd.name}${cmd.args ? ` ${cmd.args}` : ''}`,
				category: cmd.source || 'Command',
				description: cmd.description,
				badge: 'command',
				searchText: `${cmd.name} ${cmd.description} ${cmd.source}`.toLowerCase(),
				run: () => {
					if (cmd.args) prefillDraft(`/${cmd.name} `);
					else {
						onClose();
						void session.submit(`/${cmd.name}`);
					}
				}
			});
		}

		for (const skill of session.skills) {
			list.push({
				id: `skill:${skill.name}`,
				label: `/${skill.name}`,
				category: `Skill · ${skill.source}`,
				description: skill.description,
				badge: 'skill',
				searchText: `${skill.name} ${skill.description} ${skill.source}`.toLowerCase(),
				run: () => prefillDraft(`/${skill.name} `)
			});
		}

		for (const prompt of session.mcpPrompts) {
			list.push({
				id: `mcp:${prompt.name}`,
				label: `/${prompt.name}${prompt.args ? ` ${prompt.args}` : ''}`,
				category: prompt.source || 'MCP Prompt',
				description: prompt.description,
				badge: 'mcp',
				searchText: `${prompt.name} ${prompt.description} ${prompt.source}`.toLowerCase(),
				run: () => {
					if (prompt.args) prefillDraft(`/${prompt.name} `);
					else {
						onClose();
						void session.submit(`/${prompt.name}`);
					}
				}
			});
		}

		if (sessions.length > 1) {
			for (const s of sessions) {
				if (s.tab === session.tab) continue;
				const label = s.title || s.info?.cwd?.split(/[\\/]/).pop() || `Tab ${s.tab}`;
				list.push({
					id: `tab:${s.tab}`,
					label: `Switch to ${label}`,
					category: 'Jobs & Tabs',
					description: `${s.info?.cwd ?? 'Workspace'}${s.currentModel ? ` · ${s.currentModel}` : ''}`,
					badge: s.streaming ? 'working' : 'tab',
					searchText: `switch tab ${label} ${s.info?.cwd ?? ''} ${s.currentModel ?? ''}`.toLowerCase(),
					run: () => {
						onClose();
						onSelectJob(s.tab);
					}
				});
			}
		}

		const q = trimmed.replace(/^\/+/, '').toLowerCase();
		if (!q) return list;
		const scored: { entry: PaletteEntry; rank: number }[] = [];
		for (const entry of list) {
			const lowLabel = entry.label.toLowerCase();
			if (lowLabel.startsWith(`/${q}`) || lowLabel.startsWith(q)) {
				scored.push({ entry, rank: 0 });
			} else if (entry.searchText.includes(q)) {
				scored.push({ entry, rank: 1 });
			}
		}
		scored.sort((a, b) => a.rank - b.rank);
		return scored.map((s) => s.entry);
	});

	$effect(() => {
		inputEl?.focus();
	});

	$effect(() => {
		void entries.length;
		if (selectedIndex >= entries.length) selectedIndex = 0;
	});

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Escape') {
			event.preventDefault();
			onClose();
			return;
		}
		if (entries.length === 0) return;
		if (event.key === 'ArrowDown') {
			event.preventDefault();
			selectedIndex = (selectedIndex + 1) % entries.length;
		} else if (event.key === 'ArrowUp') {
			event.preventDefault();
			selectedIndex = (selectedIndex - 1 + entries.length) % entries.length;
		} else if (event.key === 'Enter' && !event.isComposing) {
			event.preventDefault();
			const picked = entries[Math.min(selectedIndex, entries.length - 1)];
			picked?.run();
		}
	}
</script>

<div
	class="backdrop"
	role="presentation"
	onclick={(e) => {
		if (e.target === e.currentTarget) onClose();
	}}
>
	<div
		class="palette"
		role="dialog"
		aria-modal="true"
		aria-labelledby="palette-title"
		tabindex="-1"
		onkeydown={onKeydown}
	>
		<div class="palette-head">
			<span class="palette-sigil" aria-hidden="true">⌘K</span>
			<h2 id="palette-title" class="sr-only">Command palette</h2>
			<input
				bind:this={inputEl}
				bind:value={query}
				type="search"
				class="palette-input"
				placeholder="Type a command, slash action, skill, or tab…"
				aria-label="Filter commands and actions"
			/>
			<button type="button" class="close-btn" aria-label="Close command palette" onclick={onClose}>
				×
			</button>
		</div>

		<div class="palette-list" role="listbox" aria-label="Command actions">
			{#if entries.length === 0}
				<p class="empty">No commands match “{query.trim()}”.</p>
			{:else}
				{#each entries as entry, idx (entry.id)}
					<button
						type="button"
						role="option"
						aria-selected={idx === selectedIndex}
						class="entry"
						class:active={idx === selectedIndex}
						onmouseenter={() => (selectedIndex = idx)}
						onclick={entry.run}
					>
						<div class="entry-main">
							<span class="entry-label">{entry.label}</span>
							{#if entry.badge}
								<span class="entry-badge">{entry.badge}</span>
							{:else}
								<span class="entry-cat">{entry.category}</span>
							{/if}
						</div>
						<div class="entry-sub">
							<span class="entry-desc">{entry.description}</span>
							{#if entry.shortcut}
								<span class="kbd">{entry.shortcut}</span>
							{/if}
						</div>
					</button>
				{/each}
			{/if}
		</div>

		<div class="palette-foot">
			<span><span class="kbd">↑↓</span> move</span>
			<span><span class="kbd">⏎</span> run</span>
			<span><span class="kbd">esc</span> close</span>
		</div>
	</div>
</div>

<style>
	.backdrop {
		position: fixed;
		inset: 0;
		z-index: 70;
		background: var(--scrim);
		display: flex;
		justify-content: center;
		align-items: flex-start;
		padding: min(12vh, 88px) var(--sp-md) var(--sp-md);
		animation: scrim-in 140ms ease-out both;
	}

	.palette {
		width: min(580px, 100%);
		max-height: min(520px, calc(100vh - 120px));
		display: flex;
		flex-direction: column;
		background: var(--bg-raised);
		border: 1px solid var(--plate-line);
		border-radius: var(--r-xl);
		box-shadow: var(--shadow-pop);
		overflow: hidden;
		outline: none;
		animation: palette-in 160ms cubic-bezier(0.22, 1, 0.36, 1) both;
	}

	@keyframes scrim-in {
		from {
			opacity: 0;
		}
		to {
			opacity: 1;
		}
	}

	@keyframes palette-in {
		from {
			opacity: 0;
			transform: translateY(-6px) scale(0.985);
		}
		to {
			opacity: 1;
			transform: translateY(0) scale(1);
		}
	}

	.palette-head {
		display: flex;
		align-items: center;
		gap: 10px;
		padding: 12px 14px;
		background: var(--bg-panel);
		border-bottom: 1px solid var(--rule);
	}

	.palette-sigil {
		font-family: var(--font-mono);
		font-size: 0.71875rem;
		font-weight: 600;
		padding: 3px 7px;
		border-radius: var(--r-md);
		background: var(--duck-soft);
		color: var(--duck-ink);
		border: 1px solid var(--rule);
		flex: none;
	}

	.palette-input {
		flex: 1;
		min-width: 0;
		border: none;
		background: transparent;
		color: var(--ink);
		font-size: 0.90625rem;
		outline: none;
	}

	.close-btn {
		font-size: 1.25rem;
		line-height: 1;
		padding: 2px 8px;
		border-radius: var(--r-md);
		color: var(--ink-2);
	}

	.close-btn:hover {
		background: var(--bg-soft);
		color: var(--ink);
	}

	.palette-list {
		overflow-y: auto;
		padding: 8px;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}

	.entry {
		display: flex;
		flex-direction: column;
		gap: 2px;
		width: 100%;
		text-align: left;
		padding: 9px 12px;
		border-radius: var(--r-lg);
		border: 1px solid transparent;
	}

	.entry.active,
	.entry:hover {
		background: var(--bg-soft);
		border-color: var(--rule);
	}

	.entry.active {
		box-shadow: inset 3px 0 0 var(--duck);
	}

	.entry-main {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
	}

	.entry-label {
		font-family: var(--font-mono);
		font-size: 0.8125rem;
		font-weight: 600;
		color: var(--ink);
	}

	.entry-cat,
	.entry-badge {
		font-size: 0.65625rem;
		font-weight: 600;
		letter-spacing: 0.05em;
		text-transform: uppercase;
		color: var(--ink-3);
	}

	.entry-badge {
		padding: 1px 6px;
		border-radius: 999px;
		background: var(--sky-soft);
		color: var(--sky-ink);
	}

	.entry-sub {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 10px;
	}

	.entry-desc {
		font-size: 0.78125rem;
		color: var(--ink-2);
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}

	.palette-foot {
		display: flex;
		align-items: center;
		gap: 16px;
		padding: 9px 14px;
		border-top: 1px solid var(--rule-soft);
		background: var(--bg-panel);
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.empty {
		margin: 18px 12px;
		text-align: center;
		font-size: 0.84375rem;
		color: var(--ink-3);
	}

	.sr-only {
		position: absolute;
		width: 1px;
		height: 1px;
		padding: 0;
		margin: -1px;
		overflow: hidden;
		clip: rect(0, 0, 0, 0);
		border: 0;
	}
</style>
