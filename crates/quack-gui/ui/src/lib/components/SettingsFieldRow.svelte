<script lang="ts">
	import { untrack } from 'svelte';
	import type { ProfileInfo, ProviderModels } from '#lib/protocol';
	import {
		parseOrigin,
		parseTomlNumber,
		parseTomlString,
		parseTomlStringArray,
		formatTomlStringArray,
		prepareFieldTomlValue,
		extractUrlHost,
		type SettingFieldDef,
		type SettingRow
	} from '../settings';

	interface Props {
		field: SettingFieldDef;
		row: SettingRow;
		busy: boolean;
		providers?: ProviderModels[];
		profilesState?: { active: string; forced: boolean; profiles: ProfileInfo[] } | null;
		showSectionBadge?: string;
		onSave: (key: string, tomlValue: string) => Promise<boolean>;
		onBrowseFolder?: () => Promise<string | null>;
		onApplyProfileSession?: (id: string) => Promise<boolean>;
	}

	let {
		field,
		row,
		busy,
		providers = [],
		profilesState = null,
		showSectionBadge,
		onSave,
		onBrowseFolder,
		onApplyProfileSession
	}: Props = $props();

	function toDisplayValue(f: SettingFieldDef, rawToml: string): string {
		switch (f.kind) {
			case 'bool':
				return rawToml.trim() === 'true' ? 'true' : 'false';
			case 'int':
			case 'float': {
				const num = parseTomlNumber(rawToml);
				return num !== null ? String(num) : '';
			}
			case 'nullable-int': {
				const num = parseTomlNumber(rawToml);
				return num !== null ? String(num) : '';
			}
			case 'list':
			case 'path-list':
				return rawToml;
			default:
				return parseTomlString(rawToml);
		}
	}

	const committedValue = $derived(toDisplayValue(field, row.value));
	const origin = $derived(parseOrigin(row.origin));
	const listItems = $derived(
		field.kind === 'list' || field.kind === 'path-list' ? parseTomlStringArray(row.value) : []
	);
	const boolEnabled = $derived(row.value.trim() === 'true');
	const isUrlField = $derived(
		field.key.endsWith('.url') || field.key === 'network.proxy' || field.key === 'telemetry.otel_endpoint'
	);
	const urlHost = $derived(isUrlField ? extractUrlHost(committedValue) : null);

	let draft = $state<{ base: string; text: string } | null>(null);
	let newListItem = $state('');
	let customThemeEditing = $state(false);
	let localError = $state<string | null>(null);

	const inputValue = $derived(
		draft && draft.base === committedValue ? draft.text : committedValue
	);
	const dirty = $derived(inputValue !== committedValue);
	const disabled = $derived(busy || row.locked);

	const modelSuggestions = $derived.by(() => {
		if (field.kind !== 'model') return [];
		const out: string[] = [];
		for (const p of providers) {
			if (!p.available) continue;
			for (const m of p.models) {
				out.push(`${p.id}/${m.alias ?? m.id}`);
			}
		}
		return out;
	});

	const selectDetail = $derived.by(() => {
		if (!field.options) return undefined;
		return field.options.find((opt) => opt.value === committedValue)?.detail;
	});

	const isKnownThemeOption = $derived(
		field.kind === 'theme' &&
			(field.options?.some((o) => o.value === committedValue) ?? false)
	);

	const datalistId = $derived(`datalist-${field.key.replace(/[^a-z0-9]/gi, '-')}`);
	const inputId = $derived(`setting-${field.key.replace(/[^a-z0-9]/gi, '-')}`);

	async function commitRawText(candidate: string) {
		if (disabled) return;
		localError = null;
		const prepared = prepareFieldTomlValue(field, candidate);
		if (!prepared.ok) {
			localError = prepared.error;
			return;
		}
		const ok = await onSave(field.key, prepared.value);
		if (ok) {
			draft = null;
			customThemeEditing = false;
		}
	}

	async function commitList(next: string[]) {
		if (disabled) return;
		localError = null;
		await onSave(field.key, formatTomlStringArray(next));
	}

	async function handleAddListItem() {
		const trimmed = newListItem.trim();
		if (!trimmed) return;
		if (listItems.includes(trimmed)) {
			newListItem = '';
			return;
		}
		const ok = await onSave(field.key, formatTomlStringArray([...listItems, trimmed]));
		if (ok) newListItem = '';
	}

	async function handleRemoveListItem(idx: number) {
		const next = listItems.filter((_, i) => i !== idx);
		await commitList(next);
	}

	async function handleBrowseSingle() {
		if (!onBrowseFolder || disabled) return;
		const picked = await onBrowseFolder();
		if (!picked) return;
		const base = untrack(() => committedValue);
		draft = { base, text: picked };
		localError = null;
	}

	async function handleBrowseAndAppend() {
		if (!onBrowseFolder || disabled) return;
		const picked = await onBrowseFolder();
		if (!picked) return;
		if (listItems.includes(picked)) return;
		await commitList([...listItems, picked]);
	}

	async function handleResetNull() {
		if (disabled) return;
		localError = null;
		const ok = await onSave(field.key, 'null');
		if (ok) draft = null;
	}

	function handleTextKeydown(e: KeyboardEvent) {
		if (e.key === 'Enter' && dirty) {
			e.preventDefault();
			void commitRawText(inputValue);
		} else if (e.key === 'Escape' && dirty) {
			e.preventDefault();
			draft = null;
			localError = null;
		}
	}
</script>

<article class="setting-row" class:locked={row.locked}>
	<div class="row-info">
		<div class="row-title-line">
			<label class="row-label" for={inputId}>{field.label}</label>
			{#if showSectionBadge}
				<span class="badge section">{showSectionBadge}</span>
			{/if}
			{#if field.tuiOnly}
				<span class="badge tui" title="Used by the terminal UI client">TUI</span>
			{/if}
			{#if row.locked}
				<span class="badge locked" title="Locked by system policy">Locked</span>
			{/if}
		</div>
		<div class="row-meta-line">
			<code class="key-code">{field.key}</code>
			<span
				class="origin-pill"
				class:user={origin.layer === 'user'}
				class:project={origin.layer === 'project'}
				class:system={origin.layer === 'system'}
				class:env={origin.layer === 'env'}
				class:cli={origin.layer === 'cli'}
				title={origin.detail || row.origin}
			>
				{origin.label}{#if origin.layer === 'env' && origin.detail}: {origin.detail}{/if}
			</span>
			{#if urlHost}
				<span class="host-pill">{urlHost}</span>
			{/if}
		</div>
		<p class="row-desc">{row.description || field.fallbackDescription}</p>
		{#if field.hint}
			<p class="row-hint">{field.hint}</p>
		{/if}
	</div>

	<div class="row-control">
		{#if field.kind === 'bool'}
			<div class="bool-row">
				<button
					id={inputId}
					type="button"
					role="switch"
					aria-checked={boolEnabled}
					class="switch-btn"
					class:on={boolEnabled}
					{disabled}
					onclick={() => void onSave(field.key, boolEnabled ? 'false' : 'true')}
				>
					<span class="switch-track" aria-hidden="true"><span class="switch-thumb"></span></span>
					<span>{boolEnabled ? 'Enabled' : 'Disabled'}</span>
				</button>
			</div>
		{:else if field.kind === 'select'}
			<select
				id={inputId}
				class="ctrl-select"
				value={committedValue}
				{disabled}
				onchange={(e) => void commitRawText(e.currentTarget.value)}
			>
				{#each field.options ?? [] as opt (opt.value)}
					<option value={opt.value}>{opt.label}</option>
				{/each}
			</select>
			{#if selectDetail}
				<p class="ctrl-note">{selectDetail}</p>
			{/if}
		{:else if field.kind === 'profile'}
			<div class="profile-ctrl">
				<select
					id={inputId}
					class="ctrl-select"
					value={committedValue}
					{disabled}
					onchange={(e) => void commitRawText(e.currentTarget.value)}
				>
					<option value="">auto (match by model family)</option>
					{#each profilesState?.profiles ?? [] as prof (prof.id)}
						<option value={prof.id}>
							{prof.id}{prof.matches ? ' · recommended' : ''}
						</option>
					{/each}
					{#if committedValue && !(profilesState?.profiles ?? []).some((p) => p.id === committedValue)}
						<option value={committedValue}>{committedValue}</option>
					{/if}
				</select>
				{#if onApplyProfileSession && profilesState}
					<div class="ctrl-subrow">
						<span class="ctrl-note">
							Session active: <code>{profilesState.active}</code>
							{profilesState.forced ? ' (forced)' : ' (auto)'}
						</span>
						<button
							type="button"
							class="btn btn-sm"
							{disabled}
							onclick={() => void onApplyProfileSession(committedValue || 'auto')}
						>
							Apply to session
						</button>
					</div>
				{/if}
			</div>
		{:else if field.kind === 'theme'}
			{#if !customThemeEditing && isKnownThemeOption}
				<div class="ctrl-inline">
					<select
						id={inputId}
						class="ctrl-select"
						value={committedValue}
						{disabled}
						onchange={(e) => void commitRawText(e.currentTarget.value)}
					>
						{#each field.options ?? [] as opt (opt.value)}
							<option value={opt.value}>{opt.label}</option>
						{/each}
					</select>
					<button
						type="button"
						class="btn btn-sm"
						{disabled}
						onclick={() => (customThemeEditing = true)}
					>
						Custom name
					</button>
				</div>
			{:else}
				<div class="ctrl-inline">
					<input
						id={inputId}
						class="ctrl-input mono"
						type="text"
						value={inputValue}
						placeholder="auto, dark, light, or theme name"
						{disabled}
						oninput={(e) => {
							draft = { base: committedValue, text: e.currentTarget.value };
							localError = null;
						}}
						onkeydown={handleTextKeydown}
					/>
					{#if dirty}
						<button
							type="button"
							class="btn btn-primary btn-sm"
							{disabled}
							onclick={() => void commitRawText(inputValue)}
						>
							Save
						</button>
					{/if}
					<button
						type="button"
						class="btn btn-sm"
						{disabled}
						onclick={() => {
							customThemeEditing = false;
							draft = null;
						}}
					>
						Presets
					</button>
				</div>
			{/if}
		{:else if field.kind === 'list' || field.kind === 'path-list'}
			<div class="list-ctrl">
				{#if listItems.length > 0}
					<ul class="chip-list">
						{#each listItems as item, idx (item)}
							<li class="chip">
								<code>{item}</code>
								<button
									type="button"
									class="chip-x"
									aria-label="Remove {item}"
									{disabled}
									onclick={() => void handleRemoveListItem(idx)}
								>
									×
								</button>
							</li>
						{/each}
					</ul>
				{:else}
					<p class="ctrl-note">{field.emptyMeaning ?? 'Empty list (`[]`)'}</p>
				{/if}
				<div class="ctrl-inline">
					<input
						id={inputId}
						class="ctrl-input mono"
						type="text"
						bind:value={newListItem}
						placeholder={field.placeholder ?? 'Add entry…'}
						{disabled}
						onkeydown={(e) => {
							if (e.key === 'Enter') {
								e.preventDefault();
								void handleAddListItem();
							}
						}}
					/>
					<button
						type="button"
						class="btn btn-sm"
						disabled={disabled || !newListItem.trim()}
						onclick={() => void handleAddListItem()}
					>
						Add
					</button>
					{#if field.kind === 'path-list' && onBrowseFolder}
						<button
							type="button"
							class="btn btn-sm"
							{disabled}
							onclick={() => void handleBrowseAndAppend()}
						>
							Browse…
						</button>
					{/if}
				</div>
			</div>
		{:else}
			<div class="text-ctrl">
				<div class="ctrl-inline">
					<input
						id={inputId}
						class="ctrl-input"
						class:mono={field.kind !== 'int' && field.kind !== 'nullable-int' && field.kind !== 'float'}
						type={field.kind === 'int' || field.kind === 'nullable-int' || field.kind === 'float' ? 'number' : 'text'}
						min={field.min}
						max={field.max}
						step={field.step}
						list={field.kind === 'model' && modelSuggestions.length > 0 ? datalistId : undefined}
						placeholder={field.placeholder ?? ''}
						value={inputValue}
						{disabled}
						oninput={(e) => {
							draft = { base: committedValue, text: e.currentTarget.value };
							localError = null;
						}}
						onkeydown={handleTextKeydown}
					/>
					{#if field.kind === 'model' && modelSuggestions.length > 0}
						<datalist id={datalistId}>
							{#each modelSuggestions as modelRef (modelRef)}
								<option value={modelRef}></option>
							{/each}
						</datalist>
					{/if}
					{#if field.kind === 'path' && onBrowseFolder}
						<button
							type="button"
							class="btn btn-sm"
							{disabled}
							onclick={() => void handleBrowseSingle()}
						>
							Browse…
						</button>
					{/if}
					{#if dirty}
						<button
							type="button"
							class="btn btn-primary btn-sm"
							{disabled}
							onclick={() => void commitRawText(inputValue)}
						>
							Save
						</button>
						<button
							type="button"
							class="btn btn-sm"
							{disabled}
							onclick={() => {
								draft = null;
								localError = null;
							}}
						>
							Cancel
						</button>
					{:else if field.supportsNullReset && committedValue !== ''}
						<button
							type="button"
							class="btn btn-sm"
							{disabled}
							onclick={() => void handleResetNull()}
						>
							Unset
						</button>
					{:else if (field.kind === 'path' || field.kind === 'text' || field.kind === 'model') && committedValue !== ''}
						<button
							type="button"
							class="btn btn-sm"
							{disabled}
							onclick={() => void commitRawText('')}
						>
							Clear
						</button>
					{/if}
				</div>
				{#if !inputValue && field.emptyMeaning}
					<p class="ctrl-note">{field.emptyMeaning}</p>
				{/if}
			</div>
		{/if}

		{#if origin.layer === 'user' || origin.layer === 'project'}
			<div class="layer-reset-row">
				<button
					type="button"
					class="reset-link"
					title="Remove this key from the edited config layer so lower defaults apply"
					{disabled}
					onclick={() => void handleResetNull()}
				>
					Remove override from {origin.layer} layer
				</button>
			</div>
		{/if}

		{#if localError}
			<p class="ctrl-error" role="alert">{localError}</p>
		{/if}
	</div>
</article>

<style>
	.setting-row {
		display: grid;
		grid-template-columns: minmax(0, 1.15fr) minmax(240px, 1fr);
		gap: var(--sp-md);
		align-items: start;
		padding: 13px 0;
		border-top: 1px solid var(--rule-soft);
	}
	.setting-row:first-child {
		border-top: none;
		padding-top: 4px;
	}
	.setting-row.locked {
		opacity: 0.82;
	}
	@media (max-width: 780px) {
		.setting-row {
			grid-template-columns: 1fr;
			gap: 10px;
		}
	}

	.row-info {
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.row-title-line {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px;
	}
	.row-label {
		font-weight: 600;
		font-size: 0.84375rem;
		color: var(--ink);
	}
	.badge {
		font-size: 0.625rem;
		font-weight: 700;
		letter-spacing: 0.06em;
		text-transform: uppercase;
		padding: 1px 6px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		color: var(--ink-3);
		background: var(--bg-soft);
	}
	.badge.tui {
		color: var(--sky-ink);
		background: var(--sky-soft);
		border-color: color-mix(in srgb, var(--sky) 45%, var(--rule));
	}
	.badge.locked {
		color: var(--beak-ink);
		background: var(--beak-soft);
		border-color: color-mix(in srgb, var(--beak) 45%, var(--rule));
	}
	.badge.section {
		color: var(--duck-ink);
		background: var(--duck-soft);
		border-color: var(--duck-line);
	}

	.row-meta-line {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px;
	}
	.key-code {
		font-size: 0.71875rem;
		color: var(--ink-2);
	}
	.origin-pill,
	.host-pill {
		font-family: var(--font-mono);
		font-size: 0.65625rem;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		border: 1px solid var(--rule-soft);
		background: var(--bg-soft);
		color: var(--ink-3);
		max-width: 100%;
		overflow: hidden;
		text-overflow: ellipsis;
		white-space: nowrap;
	}
	.origin-pill.user,
	.origin-pill.project {
		color: var(--sky-ink);
		background: var(--sky-soft);
		border-color: color-mix(in srgb, var(--sky) 40%, var(--rule-soft));
	}
	.origin-pill.system,
	.origin-pill.env,
	.origin-pill.cli {
		color: var(--beak-ink);
		background: var(--beak-soft);
		border-color: color-mix(in srgb, var(--beak) 40%, var(--rule-soft));
	}
	.host-pill {
		color: var(--ok);
		background: var(--ok-soft);
	}

	.row-desc,
	.row-hint {
		margin: 0;
		font-size: 0.78125rem;
		line-height: 1.45;
		color: var(--ink-2);
		overflow-wrap: anywhere;
	}
	.row-hint {
		font-size: 0.71875rem;
		color: var(--ink-3);
	}

	.row-control {
		min-width: 0;
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.ctrl-inline {
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
		align-items: center;
	}
	.ctrl-input,
	.ctrl-select {
		width: 100%;
		min-width: 0;
		min-height: 2rem;
		padding: 4px 10px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background-color: var(--bg-raised);
		color: var(--ink);
		font-size: 0.8125rem;
		line-height: 1.4;
	}
	.ctrl-select {
		padding-right: 30px;
	}
	.ctrl-inline > .ctrl-input,
	.ctrl-inline > .ctrl-select {
		flex: 1 1 160px;
		width: auto;
	}
	.ctrl-input.mono {
		font-family: var(--font-mono);
		font-size: 0.78125rem;
	}
	.ctrl-input:focus,
	.ctrl-select:focus {
		outline: 2px solid var(--focus);
		outline-offset: 1px;
	}
	.btn-sm {
		padding: 4px 10px;
		font-size: 0.75rem;
		min-height: 1.875rem;
		white-space: nowrap;
	}

	.switch-btn {
		display: inline-flex;
		align-items: center;
		gap: 9px;
		padding: 5px 11px 5px 7px;
		border-radius: 999px;
		border: 1px solid var(--rule);
		background: var(--bg-soft);
		color: var(--ink-2);
		font-size: 0.78125rem;
		font-weight: 600;
	}
	.switch-btn.on {
		background: var(--ok-soft);
		border-color: color-mix(in srgb, var(--ok) 45%, var(--rule));
		color: var(--ok);
	}
	.switch-track {
		width: 28px;
		height: 16px;
		border-radius: 999px;
		background: var(--rule);
		position: relative;
		transition: background-color 120ms ease-out;
	}
	.switch-btn.on .switch-track {
		background: var(--ok);
	}
	.switch-thumb {
		position: absolute;
		top: 2px;
		left: 2px;
		width: 12px;
		height: 12px;
		border-radius: 50%;
		background: var(--bg-raised);
		transition: transform 120ms ease-out;
	}
	.switch-btn.on .switch-thumb {
		transform: translateX(12px);
	}

	.list-ctrl,
	.text-ctrl,
	.profile-ctrl {
		display: flex;
		flex-direction: column;
		gap: 6px;
	}
	.ctrl-subrow {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 8px;
	}
	.chip-list {
		list-style: none;
		padding: 0;
		margin: 0;
		display: flex;
		flex-wrap: wrap;
		gap: 6px;
	}
	.chip {
		display: inline-flex;
		align-items: center;
		gap: 4px;
		padding: 2px 5px 2px 8px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background: var(--bg-soft);
		max-width: 100%;
	}
	.chip code {
		font-size: 0.71875rem;
		overflow-wrap: anywhere;
	}
	.chip-x {
		width: 18px;
		height: 18px;
		border-radius: var(--r-sm);
		display: inline-flex;
		align-items: center;
		justify-content: center;
		color: var(--ink-3);
		font-weight: 700;
		line-height: 1;
	}
	.chip-x:hover:not(:disabled) {
		background: var(--fail-soft);
		color: var(--fail);
	}

	.ctrl-note {
		margin: 0;
		font-size: 0.71875rem;
		color: var(--ink-3);
		line-height: 1.4;
	}
	.layer-reset-row {
		display: flex;
		justify-content: flex-start;
	}
	.reset-link {
		font-size: 0.71875rem;
		color: var(--ink-3);
		text-decoration: underline;
		text-decoration-color: var(--rule);
		text-underline-offset: 2px;
	}
	.reset-link:hover:not(:disabled) {
		color: var(--fail);
		text-decoration-color: var(--fail);
	}
	.ctrl-error {
		margin: 0;
		font-size: 0.75rem;
		color: var(--fail);
	}
</style>
