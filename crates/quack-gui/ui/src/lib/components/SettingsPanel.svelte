<script lang="ts">
	import type { WriteScope } from '#lib/protocol';
	import { isHttpUrl, type Session } from '../session.svelte';
	import {
		SETTING_FIELDS,
		SETTING_FIELD_MAP,
		SETTINGS_SECTIONS,
		buildAllSettingRows,
		buildSettingRowMap,
		extractUrlHost,
		formatTomlStringArray,
		matchesSettingSearch,
		parseOrigin,
		parseTomlString,
		parseTomlStringArray,
		resolvedPathRows,
		type SettingFieldDef,
		type SettingRow,
		type SettingsSectionId
	} from '../settings';
	import ExtensionsSettings from './ExtensionsSettings.svelte';
	import SettingsFieldRow from './SettingsFieldRow.svelte';
	import TextScaleRow from './TextScaleRow.svelte';

	interface Props {
		session: Session;
	}

	let { session }: Props = $props();

	let writeScope = $state<WriteScope>('user');
	let searchQuery = $state('');
	let copiedId = $state<string | null>(null);
	let editingRawKey = $state<string | null>(null);
	let rawDraft = $state('');

	const info = $derived(session.info);
	const rowMap = $derived(buildSettingRowMap(info?.settings ?? []));
	const allRows = $derived(buildAllSettingRows(info?.settings ?? []));
	const activeSectionMeta = $derived(
		SETTINGS_SECTIONS.find((s) => s.id === session.settingsTab) ?? SETTINGS_SECTIONS[0]
	);

	const busy = $derived(
		session.closed || session.starting || session.settingSaving || session.streaming || session.turnPending
	);

	const groupedFields = $derived.by(() => {
		const groups = new Map<string, SettingFieldDef[]>();
		for (const field of SETTING_FIELDS) {
			if (field.section !== session.settingsTab) continue;
			const bucket = groups.get(field.group) ?? [];
			bucket.push(field);
			groups.set(field.group, bucket);
		}
		return [...groups.entries()].map(([title, fields]) => ({ title, fields }));
	});

	const searchResults = $derived.by(() => {
		const q = searchQuery.trim();
		if (!q) return null;
		const needle = q.toLowerCase();
		const curated: { field: SettingFieldDef; row: SettingRow; sectionTitle: string }[] = [];
		const matchedKeys = new Set<string>();
		const sectionTitleById = new Map(SETTINGS_SECTIONS.map((s) => [s.id, s.shortLabel]));
		for (const field of SETTING_FIELDS) {
			const row = rowMap.get(field.key);
			if (!row) continue;
			if (matchesSettingSearch(q, row, field)) {
				curated.push({
					field,
					row,
					sectionTitle: sectionTitleById.get(field.section) ?? field.section
				});
				matchedKeys.add(field.key);
			}
		}
		const extra = allRows.filter(
			(row) => !matchedKeys.has(row.key) && matchesSettingSearch(q, row)
		);
		const mcpHits = (info?.mcp ?? []).filter(
			(m) =>
				m.id.toLowerCase().includes(needle) ||
				m.description.toLowerCase().includes(needle) ||
				m.target.toLowerCase().includes(needle)
		);
		const hookHits = (info?.hooks ?? []).filter(
			(h) =>
				h.id.toLowerCase().includes(needle) ||
				h.description.toLowerCase().includes(needle) ||
				(h.event ?? '').toLowerCase().includes(needle)
		);
		return { curated, extra, mcpHits, hookHits };
	});

	const pathRows = $derived(resolvedPathRows(info?.cwd ?? session.cwd, info?.paths));
	const catalogUrlVal = $derived(parseTomlString(rowMap.get('catalog.url')?.value ?? ''));
	const updatesUrlVal = $derived(parseTomlString(rowMap.get('updates.url')?.value ?? ''));
	const catalogHost = $derived(extractUrlHost(catalogUrlVal));
	const updatesHost = $derived(extractUrlHost(updatesUrlVal));
	const isOffline = $derived((rowMap.get('network.offline')?.value ?? 'false').trim() === 'true');
	const disabledBundles = $derived(
		parseTomlStringArray(rowMap.get('bundles.disabled')?.value ?? '[]')
	);

	const projectTrustNote = $derived.by(() => {
		const status = info?.project_status;
		if (!status || status === 'none') {
			return 'Creates `.quack/config.toml` in this repository. New project folders need trust approval once before taking effect.';
		}
		if (status === 'trusted' || status === 'settings-only') {
			return `Workspace config is active (${status}).`;
		}
		return `Workspace status is \`${status}\` — settings written here stay skipped until you trust the project.`;
	});

	async function saveSetting(key: string, tomlValue: string): Promise<boolean> {
		return session.saveSetting(key, tomlValue, writeScope, true);
	}

	async function toggleBundle(bundleId: string, currentlyEnabled: boolean) {
		if (busy) return;
		const next = currentlyEnabled
			? [...new Set([...disabledBundles, bundleId])]
			: disabledBundles.filter((id) => id !== bundleId);
		await saveSetting('bundles.disabled', formatTomlStringArray(next));
	}

	async function copyText(id: string, value: string) {
		try {
			await navigator.clipboard?.writeText(value);
			copiedId = id;
			setTimeout(() => {
				if (copiedId === id) copiedId = null;
			}, 1600);
		} catch {
			// clipboard unavailable in headless contexts
		}
	}

	function startRawEdit(row: SettingRow) {
		editingRawKey = row.key;
		rawDraft = row.value;
	}

	async function commitRawEdit(key: string) {
		const val = rawDraft.trim();
		if (!val) return;
		const ok = await saveSetting(key, val);
		if (ok) editingRawKey = null;
	}

	function handleWindowKeydown(e: KeyboardEvent) {
		if (e.key !== 'Escape') return;
		if (editingRawKey !== null) {
			editingRawKey = null;
			return;
		}
		const active = document.activeElement;
		if (active && (active.tagName === 'INPUT' || active.tagName === 'TEXTAREA')) {
			if (active === searchEl && searchQuery) {
				searchQuery = '';
				return;
			}
			(active as HTMLElement).blur();
			return;
		}
		session.closeSettings();
	}

	let searchEl: HTMLInputElement | undefined = $state();

	function sectionDisplayName(id: SettingsSectionId) {
		return SETTINGS_SECTIONS.find((s) => s.id === id)?.shortLabel ?? id;
	}
</script>

<svelte:window onkeydown={handleWindowKeydown} />

<section class="settings-page" aria-label="Settings">
	<div class="settings-inner">
		<header class="page-head">
			<div class="head-copy">
				<span class="kicker">Configuration &amp; Setup</span>
				<h2>Settings</h2>
				<p class="subtitle">
					Everything configured by <code>quack setup</code> and <code>config.toml</code> —
					directory paths, model defaults, background checks, context limits, permissions, hooks,
					MCP servers, and all layered TOML keys.
				</p>
			</div>
			<div class="head-actions">
				<button
					type="button"
					class="btn"
					disabled={busy}
					title="Reload config.toml layers and restart MCP/LSP servers"
					onclick={() => void session.reloadConfig()}
				>
					Reload config
				</button>
				<button
					type="button"
					class="btn btn-primary"
					onclick={() => session.closeSettings()}
				>
					Done
				</button>
			</div>
		</header>

		<div class="toolbar">
			<div class="scope-group" role="group" aria-label="Config save layer">
				<span class="scope-label">Save to</span>
				<div class="segmented">
					<button
						type="button"
						class="seg-btn"
						class:active={writeScope === 'user'}
						aria-pressed={writeScope === 'user'}
						onclick={() => (writeScope = 'user')}
					>
						User config
					</button>
					<button
						type="button"
						class="seg-btn"
						class:active={writeScope === 'project'}
						aria-pressed={writeScope === 'project'}
						onclick={() => (writeScope = 'project')}
					>
						Project (.quack)
					</button>
				</div>
				{#if writeScope === 'project'}
					<span class="scope-note">{projectTrustNote}</span>
				{:else if info?.paths?.user_config}
					<code class="scope-path">{info.paths.user_config}/config.toml</code>
				{/if}
			</div>

			<div class="search-box">
				<input
					bind:this={searchEl}
					type="search"
					class="search-input"
					aria-label="Filter settings"
					placeholder="Filter settings, hooks &amp; MCP (e.g. memory.path, jira, github)…"
					bind:value={searchQuery}
				/>
				{#if searchQuery}
					<button
						type="button"
						class="clear-search"
						onclick={() => (searchQuery = '')}
					>
						Clear
					</button>
				{/if}
			</div>
		</div>

		{#if session.settingError}
			<div class="banner error" role="alert">
				<span class="banner-text">{session.settingError}</span>
				<button type="button" class="banner-close" onclick={() => session.clearSettingStatus()}>
					Dismiss
				</button>
			</div>
		{:else if session.settingFeedback}
			<div class="banner saved" role="status">
				<span class="banner-text">
					Saved <code>{session.settingFeedback.key} = {session.settingFeedback.value}</code>
					to <code>{session.settingFeedback.path}</code>
					{#if session.settingFeedback.note}
						— <strong>{session.settingFeedback.note}</strong>
					{/if}
				</span>
				<button type="button" class="banner-close" onclick={() => session.clearSettingStatus()}>
					Dismiss
				</button>
			</div>
		{/if}

		{#if searchResults}
			<div class="search-results">
				<div class="plate">
					<div class="plate-head">
						<div>
							<h3>Matching settings</h3>
							<p class="plate-sub">
								{searchResults.curated.length +
									searchResults.extra.length +
									searchResults.mcpHits.length +
									searchResults.hookHits.length} matches for “{searchQuery.trim()}”
							</p>
						</div>
					</div>
					{#if searchResults.curated.length === 0 && searchResults.extra.length === 0 && searchResults.mcpHits.length === 0 && searchResults.hookHits.length === 0}
						<p class="empty-note">
							No settings, hooks, or MCP servers match “{searchQuery.trim()}”. Try a key like
							<code>memory.path</code>, <code>catalog.refresh</code>, <code>mcp</code>, or
							<code>jira</code>.
						</p>
					{:else}
						{#if searchResults.mcpHits.length > 0 || searchResults.hookHits.length > 0}
							<div class="extra-toml-list">
								<div class="plate-head">
									<h4>Hooks &amp; MCP servers</h4>
									<button
										type="button"
										class="btn btn-sm"
										onclick={() => {
											searchQuery = '';
											session.selectSettingsTab('extensions');
										}}
									>
										Open in Hooks &amp; MCP
									</button>
								</div>
								{#each searchResults.mcpHits as srv (srv.id)}
									<div class="extra-row">
										<span class="tag">MCP</span>
										<code>{srv.id}</code>
										<code class="extra-val">{srv.state}</code>
										{#if srv.description}
											<span class="extra-origin">{srv.description}</span>
										{/if}
									</div>
								{/each}
								{#each searchResults.hookHits as hk (hk.id)}
									<div class="extra-row">
										<span class="tag">Hook</span>
										<code>{hk.id}</code>
										<code class="extra-val">{hk.state}</code>
										{#if hk.description}
											<span class="extra-origin">{hk.description}</span>
										{/if}
									</div>
								{/each}
							</div>
						{/if}
						{#each searchResults.curated as hit (hit.field.key)}
							<SettingsFieldRow
								field={hit.field}
								row={hit.row}
								{busy}
								providers={session.providers}
								profilesState={session.profiles}
								showSectionBadge={hit.sectionTitle}
								onSave={saveSetting}
								onBrowseFolder={() => session.pickDirectory()}
								onApplyProfileSession={(id) => session.setProfile(id)}
							/>
						{/each}
						{#if searchResults.extra.length > 0}
							<div class="extra-toml-list">
								<h4>Additional TOML entries</h4>
								{#each searchResults.extra as row (row.key)}
									<div class="extra-row">
										<code>{row.key}</code>
										<code class="extra-val">{row.value}</code>
										<span class="extra-origin">{row.origin}</span>
									</div>
								{/each}
							</div>
						{/if}
					{/if}
				</div>
			</div>
		{:else}
			<div class="settings-body">
				<nav class="section-rail" aria-label="Settings sections">
					{#each SETTINGS_SECTIONS as section (section.id)}
						<button
							type="button"
							class="rail-item"
							class:active={session.settingsTab === section.id}
							aria-current={session.settingsTab === section.id ? 'page' : undefined}
							onclick={() => session.selectSettingsTab(section.id)}
						>
							<span class="rail-title">{section.title}</span>
							<span class="rail-kicker">{section.kicker}</span>
						</button>
					{/each}
				</nav>

				<div class="section-content">
					<div class="section-intro">
						<span class="kicker">{activeSectionMeta.kicker}</span>
						<h3>{activeSectionMeta.title}</h3>
						<p>{activeSectionMeta.summary}</p>
					</div>

					{#if session.settingsTab === 'extensions'}
						<ExtensionsSettings {session} {writeScope} {busy} />
					{/if}

					{#each groupedFields as group (group.title)}
						<section class="plate" aria-label={group.title}>
							<h4 class="plate-title">{group.title}</h4>
							<div class="rows-stack">
								{#if group.title === 'Desktop & terminal appearance'}
									<TextScaleRow />
								{/if}
								{#each group.fields as field (field.key)}
									{@const row = rowMap.get(field.key)}
									{#if row}
										<SettingsFieldRow
											{field}
											{row}
											{busy}
											providers={session.providers}
											profilesState={session.profiles}
											onSave={saveSetting}
											onBrowseFolder={() => session.pickDirectory()}
											onApplyProfileSession={(id) => session.setProfile(id)}
										/>
									{/if}
								{/each}
							</div>
						</section>
					{/each}

					{#if session.settingsTab === 'paths'}
						<section class="plate" aria-label="Resolved directories">
							<div class="plate-head">
								<div>
									<h4 class="plate-title">Resolved storage &amp; config directories</h4>
									<p class="plate-sub">
										Effective paths for memory notes, transcripts, user config, cache, and project files
										(equivalent to <code>quack config paths</code>).
									</p>
								</div>
							</div>
							<div class="table-list">
								{#each pathRows as pr (pr.id)}
									<div class="kv-row">
										<div class="kv-label">
											<span class="kv-name">{pr.label}</span>
											<span class="kv-hint">{pr.hint}</span>
										</div>
										<div class="kv-val-wrap">
											<code class="kv-path">{pr.path}</code>
											{#if pr.path !== '(none)'}
												<button
													type="button"
													class="btn btn-sm"
													onclick={() => void copyText(pr.id, pr.path)}
												>
													{copiedId === pr.id ? 'Copied' : 'Copy'}
												</button>
											{/if}
										</div>
									</div>
								{/each}
							</div>
						</section>

						<section class="plate" aria-label="Config layers">
							<h4 class="plate-title">Config layers (lowest to highest precedence)</h4>
							<div class="table-list">
								{#each info?.layers ?? [] as layer (layer.layer)}
									<div class="kv-row">
										<div class="kv-label">
											<span class="kv-name">{layer.layer}</span>
											<span class="kv-hint">{layer.status}</span>
										</div>
										<div class="kv-val-wrap">
											<code class="kv-path">{layer.file ?? '(no file)'}</code>
											{#if layer.file}
												<button
													type="button"
													class="btn btn-sm"
													onclick={() => void copyText(`layer-${layer.layer}`, layer.file ?? '')}
												>
													{copiedId === `layer-${layer.layer}` ? 'Copied' : 'Copy'}
												</button>
											{/if}
										</div>
									</div>
								{/each}
							</div>
						</section>
					{/if}

					{#if session.settingsTab === 'setup'}
						<section class="plate" aria-label="Software updates">
							<div class="plate-head">
								<div>
									<h4 class="plate-title">Software updates</h4>
									<p class="plate-sub">
										Running quack <code>{info?.version ?? 'unknown'}</code> ·
										{info?.egress.updates_check
											? 'Daily auto-check is enabled'
											: 'Auto-check is off (opt-in)'}
									</p>
								</div>
								<button
									type="button"
									class="btn"
									disabled={session.closed || session.starting || session.updateChecking}
									onclick={() => void session.checkForUpdates()}
								>
									{session.updateChecking ? 'Checking…' : 'Check for updates'}
								</button>
							</div>
							{#if session.updateChecking || session.updateState}
								<div class="callouts">
									{#if session.updateChecking}
										<p class="callout" role="status">
											Checking <code>{updatesHost ?? 'release source'}</code> for a newer release…
										</p>
									{:else if session.updateState?.error}
										<p class="callout fail" role="status">
											<strong>Update check failed:</strong>
											{session.updateState.error}
										</p>
									{:else if session.updateState?.updateAvailable}
										<div class="callout info" role="status">
											<div class="update-callout-row">
												<span>
													<strong>quack {session.updateState.latest} is available</strong>
													(you have <code>{session.updateState.current}</code>). quack does not
													self-update; install the new release from your package manager, release
													page, or run <code>quack update</code>.
												</span>
												{#if isHttpUrl(session.updateState.url)}
													<button
														type="button"
														class="btn btn-sm"
														onclick={() => void session.openReleasePage()}
													>
														Open release page
													</button>
												{/if}
											</div>
										</div>
									{:else if session.updateState}
										<p class="callout ok" role="status">
											{#if session.updateState.latest}
												<strong>quack {session.updateState.current} is up to date</strong>
												(latest published: <code>{session.updateState.latest}</code>).
											{:else}
												<strong>quack {session.updateState.current}</strong> — no release has been
												published at this source yet.
											{/if}
										</p>
									{/if}
								</div>
							{/if}
						</section>

						<section class="plate" aria-label="Provider and check details">
							<div class="plate-head">
								<div>
									<h4 class="plate-title">Provider connections &amp; check policy</h4>
									<p class="plate-sub">
										Interactive provider detection, API keys, and OAuth sign-ins live in Providers.
									</p>
								</div>
								<button
									type="button"
									class="btn"
									onclick={() => void session.openSetup()}
								>
									Open Providers…
								</button>
							</div>
							<div class="callouts">
								{#if isOffline}
									<p class="callout warn">
										<strong>Offline mode is enabled</strong> (<code>network.offline = true</code>).
										Daily catalog and update checks remain paused while offline even if turned on above.
									</p>
								{/if}
								{#if !catalogUrlVal && !updatesUrlVal}
									<p class="callout">
										Both <code>catalog.url</code> and <code>updates.url</code> are empty (typical for a
										<code>--plucked</code> install with hosted endpoints stripped). Configure custom URLs above if needed.
									</p>
								{:else}
									<p class="callout">
										Configured check hosts:
										<code>{catalogHost ?? 'catalog: unset'}</code> ·
										<code>{updatesHost ?? 'updates: unset'}</code>. Checks run at most once a day and only
										when explicitly enabled.
									</p>
								{/if}
							</div>
						</section>
					{/if}

					{#if session.settingsTab === 'network'}
						<section class="plate" aria-label="Configured network endpoints">
							<h4 class="plate-title">Configured providers &amp; remote MCP endpoints</h4>
							<div class="endpoints-grid">
								<div>
									<h5>Providers</h5>
									{#if (info?.egress.providers ?? []).length === 0}
										<p class="plate-sub">No provider files loaded.</p>
									{:else}
										<ul class="plain-list">
											{#each info?.egress.providers ?? [] as p (p.id)}
												<li>
													<code>{p.id}</code>
													<span class="muted">→ {p.host}</span>
													<span class="tag">{p.local ? 'local' : 'remote'}{p.enabled ? '' : ' · off'}</span>
												</li>
											{/each}
										</ul>
									{/if}
								</div>
								<div>
									<h5>
										Remote MCP servers
										<button
											type="button"
											class="jump-link"
											onclick={() => session.selectSettingsTab('extensions')}
										>
											Manage Hooks &amp; MCP →
										</button>
									</h5>
									{#if (info?.egress.mcp_remote ?? []).length === 0}
										<p class="plate-sub">No remote MCP servers configured.</p>
									{:else}
										<ul class="plain-list">
											{#each info?.egress.mcp_remote ?? [] as m (m.id)}
												<li>
													<code>{m.id}</code>
													<span class="muted">→ {m.host}</span>
													<span class="tag">{m.enabled ? 'enabled' : 'disabled'}</span>
												</li>
											{/each}
										</ul>
									{/if}
								</div>
							</div>
						</section>
					{/if}

					{#if session.settingsTab === 'permissions'}
						<section class="plate" aria-label="Installed bundles">
							<div class="plate-head">
								<div>
									<h4 class="plate-title">Installed extension bundles</h4>
									<p class="plate-sub">
										Toggle discovered bundles cleanly (updates <code>bundles.disabled</code> and reloads).
									</p>
								</div>
							</div>
							{#if (info?.bundles ?? []).length === 0}
								<p class="empty-note">No extension bundles installed in loaded layers.</p>
							{:else}
								<div class="table-list">
									{#each info?.bundles ?? [] as bundle (`${bundle.layer}:${bundle.id}`)}
										<div class="kv-row">
											<div class="kv-label">
												<span class="kv-name">
													<code>{bundle.id}</code>
													<span class="tag">{bundle.layer}</span>
												</span>
												<span class="kv-hint">{bundle.contents || 'No items declared'}</span>
											</div>
											<button
												type="button"
												class="btn btn-sm"
												class:btn-primary={!bundle.enabled}
												disabled={busy}
												onclick={() => void toggleBundle(bundle.id, bundle.enabled)}
											>
												{bundle.enabled ? 'Disable' : 'Enable'}
											</button>
										</div>
									{/each}
								</div>
							{/if}
						</section>

						<section class="plate" aria-label="Merged permission rules">
							<h4 class="plate-title">Effective merged permission rules</h4>
							{#if (info?.rules ?? []).length === 0}
								<p class="empty-note">No explicit allow / ask / deny rules configured across layers.</p>
							{:else}
								<ul class="plain-list">
									{#each info?.rules ?? [] as rule (`${rule.list}:${rule.rule}`)}
										<li>
											<span class="tag rule-{rule.list}">{rule.list}</span>
											<code>{rule.rule}</code>
										</li>
									{/each}
								</ul>
							{/if}
						</section>
					{/if}

					{#if session.settingsTab === 'interface'}
						<section class="plate" aria-label="All TOML settings">
							<div class="plate-head">
								<div>
									<h4 class="plate-title">All TOML settings ({allRows.length})</h4>
									<p class="plate-sub">
										Every effective key in <code>config.toml</code>. Edit any value as a TOML literal
										(for example <code>"text"</code>, <code>120</code>, <code>true</code>, or
										<code>["a", "b"]</code>).
									</p>
								</div>
							</div>
							<div class="toml-table">
								{#each allRows as row (row.key)}
									{@const origin = parseOrigin(row.origin)}
									{@const fieldDef = SETTING_FIELD_MAP.get(row.key)}
									<div class="toml-row">
										<div class="toml-key-col">
											<div class="toml-key-top">
												<code class="toml-key">{row.key}</code>
												{#if fieldDef}
													<button
														type="button"
														class="jump-link"
														onclick={() => {
															session.selectSettingsTab(fieldDef.section);
														}}
													>
														{sectionDisplayName(fieldDef.section)}
													</button>
												{/if}
											</div>
											{#if row.description}
												<p class="toml-desc">{row.description}</p>
											{/if}
										</div>
										<div class="toml-val-col">
											{#if editingRawKey === row.key}
												<div class="ctrl-inline">
													<input
														type="text"
														class="raw-input"
														aria-label="TOML value for {row.key}"
														bind:value={rawDraft}
														disabled={busy || row.locked}
														onkeydown={(e) => {
															if (e.key === 'Enter') {
																e.preventDefault();
																void commitRawEdit(row.key);
															} else if (e.key === 'Escape') {
																editingRawKey = null;
															}
														}}
													/>
													<button
														type="button"
														class="btn btn-primary btn-sm"
														disabled={busy || row.locked || !rawDraft.trim()}
														onclick={() => void commitRawEdit(row.key)}
													>
														Save
													</button>
													<button
														type="button"
														class="btn btn-sm"
														onclick={() => (editingRawKey = null)}
													>
														Cancel
													</button>
												</div>
											{:else}
												<div class="toml-display">
													<code class="toml-val">{row.value}</code>
													<span class="toml-origin">{origin.label}</span>
													{#if !row.locked}
														<button
															type="button"
															class="btn btn-sm"
															disabled={busy}
															onclick={() => startRawEdit(row)}
														>
															Edit
														</button>
														{#if origin.layer === 'user' || origin.layer === 'project'}
															<button
																type="button"
																class="btn btn-sm"
																disabled={busy}
																title="Remove this key from the edited layer"
																onclick={() => void saveSetting(row.key, 'null')}
															>
																Reset
															</button>
														{/if}
													{/if}
												</div>
											{/if}
										</div>
									</div>
								{/each}
							</div>
						</section>
					{/if}
				</div>
			</div>
		{/if}
	</div>
</section>

<style>
	.settings-page {
		flex: 1 1 0;
		min-height: 0;
		overflow-y: auto;
		padding: var(--sp-lg) var(--sp-md) var(--sp-xl);
		background: var(--bg);
	}
	.settings-inner {
		max-width: 1020px;
		margin: 0 auto;
		display: flex;
		flex-direction: column;
		gap: var(--sp-md);
	}

	.page-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: var(--sp-md);
		padding-bottom: var(--sp-sm);
		border-bottom: 3px double var(--rule);
	}
	.head-copy {
		min-width: 0;
		flex: 1 1 320px;
	}
	.kicker {
		display: block;
		font-size: 0.6875rem;
		font-weight: 700;
		letter-spacing: 0.08em;
		text-transform: uppercase;
		color: var(--ink-3);
		margin-bottom: 2px;
	}
	.page-head h2 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 1.5rem;
		font-weight: 600;
		color: var(--ink);
	}
	.subtitle {
		margin: 6px 0 0;
		color: var(--ink-2);
		font-size: 0.84375rem;
		line-height: 1.5;
	}
	.head-actions {
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}

	.toolbar {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 10px var(--sp-md);
		padding: 10px 14px;
		border: 1px solid var(--rule);
		border-radius: var(--r-xl);
		background: var(--bg-panel);
	}
	.scope-group {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px 10px;
		min-width: 0;
	}
	.scope-label {
		font-size: 0.71875rem;
		font-weight: 700;
		letter-spacing: 0.05em;
		text-transform: uppercase;
		color: var(--ink-3);
	}
	.segmented {
		display: inline-flex;
		padding: 2px;
		border-radius: var(--r-lg);
		border: 1px solid var(--rule);
		background: var(--bg-soft);
	}
	.seg-btn {
		padding: 4px 10px;
		border-radius: var(--r-md);
		font-size: 0.78125rem;
		font-weight: 600;
		color: var(--ink-2);
	}
	.seg-btn.active {
		background: var(--bg-raised);
		color: var(--ink);
		box-shadow: var(--shadow-card);
	}
	.scope-path,
	.scope-note {
		font-size: 0.75rem;
		color: var(--ink-3);
		overflow-wrap: anywhere;
	}

	.search-box {
		display: flex;
		align-items: center;
		gap: 6px;
		flex: 1 1 240px;
		max-width: 360px;
	}
	.search-input {
		width: 100%;
		min-height: 2rem;
		padding: 4px 10px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		color: var(--ink);
		font-size: 0.8125rem;
	}
	.search-input:focus {
		outline: 2px solid var(--focus);
		outline-offset: 1px;
	}
	.clear-search {
		font-size: 0.75rem;
		color: var(--ink-2);
		padding: 4px 8px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-raised);
	}

	.banner {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--sp-sm);
		padding: 10px 14px;
		border-radius: var(--r-lg);
		border: 1px solid var(--rule);
		font-size: 0.8125rem;
	}
	.banner.saved {
		background: var(--ok-soft);
		border-color: color-mix(in srgb, var(--ok) 45%, var(--rule));
		color: var(--ok);
	}
	.banner.error {
		background: var(--fail-soft);
		border-color: color-mix(in srgb, var(--fail) 45%, var(--rule));
		color: var(--fail);
	}
	.banner-text {
		min-width: 0;
		overflow-wrap: anywhere;
	}
	.banner-close {
		font-size: 0.75rem;
		font-weight: 600;
		text-decoration: underline;
		flex: none;
	}

	.settings-body {
		display: grid;
		grid-template-columns: 215px minmax(0, 1fr);
		gap: var(--sp-md);
		align-items: start;
	}
	.section-rail {
		position: sticky;
		top: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
		padding: 6px;
		border: 1px solid var(--rule);
		border-radius: var(--r-xl);
		background: var(--bg-panel);
	}
	.rail-item {
		text-align: left;
		padding: 9px 11px;
		border-radius: var(--r-lg);
		border: 1px solid transparent;
		display: flex;
		flex-direction: column;
		gap: 2px;
	}
	.rail-item:hover {
		background: var(--bg-soft);
	}
	.rail-item.active {
		background: var(--bg-raised);
		border-color: var(--rule);
		box-shadow: var(--shadow-card);
	}
	.rail-title {
		font-size: 0.8125rem;
		font-weight: 600;
		color: var(--ink);
	}
	.rail-item.active .rail-title {
		color: var(--ink);
	}
	.rail-kicker {
		font-size: 0.6875rem;
		color: var(--ink-3);
	}

	@media (max-width: 860px) {
		.settings-body {
			grid-template-columns: 1fr;
		}
		.section-rail {
			position: static;
			flex-direction: row;
			flex-wrap: wrap;
		}
		.rail-item {
			flex: 1 1 auto;
			padding: 6px 10px;
		}
		.rail-kicker {
			display: none;
		}
	}

	.section-content,
	.search-results {
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: var(--sp-md);
	}
	.section-intro {
		padding: 2px 2px 4px;
	}
	.section-intro h3 {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 1.125rem;
		font-weight: 600;
		color: var(--ink);
	}
	.section-intro p {
		margin: 4px 0 0;
		color: var(--ink-2);
		font-size: 0.8125rem;
	}

	.plate {
		background: var(--bg-panel);
		border: 1px solid var(--rule);
		border-radius: var(--r-xl);
		padding: var(--sp-md);
	}
	.plate-head {
		display: flex;
		align-items: flex-start;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: var(--sp-sm);
		margin-bottom: 10px;
	}
	.plate-head h3,
	.plate-head h4,
	.plate-title {
		margin: 0 0 8px;
		font-family: var(--font-serif);
		font-size: 0.9375rem;
		font-weight: 600;
		color: var(--ink);
	}
	.plate-head .plate-title {
		margin-bottom: 2px;
	}
	.plate-sub,
	.empty-note {
		margin: 0;
		font-size: 0.78125rem;
		color: var(--ink-3);
		line-height: 1.45;
	}

	.table-list {
		display: flex;
		flex-direction: column;
	}
	.kv-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 8px 14px;
		padding: 9px 0;
		border-top: 1px solid var(--rule-soft);
	}
	.kv-row:first-child {
		border-top: none;
	}
	.kv-label {
		display: flex;
		flex-direction: column;
		gap: 2px;
		min-width: 180px;
	}
	.kv-name {
		font-weight: 600;
		font-size: 0.8125rem;
		color: var(--ink);
		display: inline-flex;
		align-items: center;
		gap: 6px;
	}
	.kv-hint {
		font-size: 0.71875rem;
		color: var(--ink-3);
	}
	.kv-val-wrap {
		display: flex;
		align-items: center;
		gap: 8px;
		min-width: 0;
		flex: 1 1 260px;
		justify-content: flex-end;
	}
	.kv-path {
		font-size: 0.75rem;
		color: var(--ink-2);
		overflow-wrap: anywhere;
		text-align: right;
	}

	.callouts {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.callout {
		margin: 0;
		padding: 9px 12px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule-soft);
		background: var(--bg-soft);
		font-size: 0.78125rem;
		color: var(--ink-2);
		line-height: 1.45;
	}
	.callout.warn {
		background: var(--beak-soft);
		border-color: color-mix(in srgb, var(--beak) 45%, var(--rule));
		color: var(--beak-ink);
	}
	.callout.info {
		background: var(--sky-soft);
		border-color: color-mix(in srgb, var(--sky) 55%, var(--rule));
		color: var(--sky-ink);
	}
	.callout.ok {
		background: var(--ok-soft);
		border-color: color-mix(in srgb, var(--ok) 45%, var(--rule));
		color: var(--ok);
	}
	.callout.fail {
		background: var(--fail-soft);
		border-color: color-mix(in srgb, var(--fail) 45%, var(--rule));
		color: var(--fail);
	}
	.update-callout-row {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: var(--sp-sm);
		flex-wrap: wrap;
	}
	.update-callout-row > span {
		flex: 1 1 240px;
	}

	.endpoints-grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
		gap: var(--sp-md);
	}
	.endpoints-grid h5 {
		margin: 0 0 6px;
		font-size: 0.75rem;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		color: var(--ink-3);
	}
	.plain-list {
		list-style: none;
		padding: 0;
		margin: 0;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.plain-list li {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px;
		font-size: 0.78125rem;
	}
	.muted {
		color: var(--ink-3);
		overflow-wrap: anywhere;
	}
	.tag {
		font-size: 0.65625rem;
		font-weight: 700;
		text-transform: uppercase;
		letter-spacing: 0.05em;
		padding: 1px 6px;
		border-radius: 999px;
		border: 1px solid var(--rule-soft);
		background: var(--bg-soft);
		color: var(--ink-3);
	}
	.tag.rule-deny {
		color: var(--fail);
		background: var(--fail-soft);
	}
	.tag.rule-ask {
		color: var(--beak-ink);
		background: var(--beak-soft);
	}
	.tag.rule-allow {
		color: var(--ok);
		background: var(--ok-soft);
	}

	.toml-table {
		display: flex;
		flex-direction: column;
	}
	.toml-row {
		display: grid;
		grid-template-columns: minmax(0, 1.1fr) minmax(220px, 1fr);
		gap: 8px 14px;
		padding: 9px 0;
		border-top: 1px solid var(--rule-soft);
		align-items: start;
	}
	.toml-row:first-child {
		border-top: none;
	}
	@media (max-width: 780px) {
		.toml-row {
			grid-template-columns: 1fr;
		}
		.kv-val-wrap {
			justify-content: flex-start;
		}
		.kv-path {
			text-align: left;
		}
	}
	.toml-key-top {
		display: flex;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}
	.toml-key {
		font-size: 0.78125rem;
		font-weight: 600;
		color: var(--ink);
	}
	.jump-link {
		font-size: 0.6875rem;
		color: var(--sky-ink);
		text-decoration: underline;
		text-underline-offset: 2px;
	}
	.toml-desc {
		margin: 3px 0 0;
		font-size: 0.75rem;
		color: var(--ink-3);
		line-height: 1.4;
		overflow-wrap: anywhere;
	}
	.toml-display {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px;
		justify-content: flex-end;
	}
	.toml-val {
		font-size: 0.75rem;
		padding: 2px 7px;
		border-radius: var(--r-sm);
		background: var(--bg-soft);
		border: 1px solid var(--rule-soft);
		overflow-wrap: anywhere;
	}
	.toml-origin {
		font-size: 0.6875rem;
		color: var(--ink-3);
	}
	.ctrl-inline {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		align-items: center;
	}
	.raw-input {
		flex: 1 1 150px;
		min-width: 0;
		min-height: 1.875rem;
		padding: 3px 8px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-raised);
		font-family: var(--font-mono);
		font-size: 0.75rem;
	}
	.extra-toml-list {
		margin-top: var(--sp-md);
		padding-top: var(--sp-sm);
		border-top: 1px solid var(--rule);
	}
	.extra-toml-list h4 {
		margin: 0 0 8px;
		font-size: 0.8125rem;
		color: var(--ink-2);
	}
	.extra-row {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 8px;
		padding: 5px 0;
		border-top: 1px solid var(--rule-soft);
	}
	.extra-val {
		padding: 1px 6px;
		background: var(--bg-soft);
		border-radius: var(--r-sm);
	}
	.extra-origin {
		font-size: 0.71875rem;
		color: var(--ink-3);
	}
	.btn-sm {
		padding: 4px 10px;
		font-size: 0.75rem;
		min-height: 1.875rem;
		white-space: nowrap;
	}
</style>
