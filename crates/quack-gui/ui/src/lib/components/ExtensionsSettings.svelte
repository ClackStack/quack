<script lang="ts">
	import type {
		HookConnectionDraft,
		HookSetupInfo,
		WriteScope
	} from '#lib/protocol';
	import {
		type McpBindingEntry,
		addBlankConnection,
		applyPresetToDraft,
		buildMcpSecretMap,
		connectionCheckUrl,
		createDraftsFromSetup,
		deriveConnectionFields,
		fieldChoices,
		formatKeySource,
		isFieldRequired,
		isFieldVisible,
		mergeMcpServers,
		parseArgsString,
		sortHooks
	} from '../extensions';
	import type { Session } from '../session.svelte';

	let {
		session,
		writeScope,
		busy
	}: {
		session: Session;
		writeScope: WriteScope;
		busy: boolean;
	} = $props();

	let showAddMcp = $state(false);
	let mcpId = $state('');
	let mcpDescription = $state('');
	let mcpTransport = $state<'stdio' | 'http'>('stdio');
	let mcpCommand = $state('');
	let mcpArgs = $state('');
	let mcpCwd = $state('');
	let mcpUrl = $state('');
	let mcpSandbox = $state<'inherit' | 'on' | 'off'>('inherit');
	let nextBindingSeq = 1;
	let mcpEnvEntries = $state<McpBindingEntry[]>([]);
	let mcpHeaderEntries = $state<McpBindingEntry[]>([]);

	interface HookDraftState {
		key: string;
		drafts: HookConnectionDraft[];
		enableHook: boolean;
		showAdvanced: boolean;
	}

	let hookDraftState = $state<HookDraftState | null>(null);

	const info = $derived(session.info);
	const mcpServers = $derived(mergeMcpServers(info?.mcp ?? [], session.mcpServers));
	const hooks = $derived(sortHooks(info?.hooks ?? []));

	const mcpBusy = $derived(busy || session.mcpSaving);
	const hookBusy = $derived(busy || session.hookSaving);

	const activeSetup = $derived(session.hookSetup);
	const setupKey = $derived(activeSetup ? `${activeSetup.id}:${activeSetup.revision}` : '');
	const activeDrafts = $derived.by(() => {
		if (!activeSetup) return [];
		if (hookDraftState && hookDraftState.key === setupKey) return hookDraftState.drafts;
		return createDraftsFromSetup(activeSetup);
	});
	const enableAfterSetup = $derived.by(() => {
		if (!activeSetup) return false;
		if (hookDraftState && hookDraftState.key === setupKey) return hookDraftState.enableHook;
		return true;
	});
	const showAdvancedFields = $derived(
		hookDraftState?.key === setupKey ? hookDraftState.showAdvanced : false
	);
	const hasAnyAdvancedField = $derived(
		activeSetup?.descriptor.fields.some((f) => f.advanced) ?? false
	);

	function ensureHookDraftState(setup: HookSetupInfo): HookDraftState {
		const key = `${setup.id}:${setup.revision}`;
		if (hookDraftState && hookDraftState.key === key) {
			return hookDraftState;
		}
		const fresh: HookDraftState = {
			key,
			drafts: createDraftsFromSetup(setup),
			enableHook: true,
			showAdvanced: false
		};
		hookDraftState = fresh;
		return fresh;
	}

	function updateDraftAt(
		index: number,
		updater: (draft: HookConnectionDraft, setup: HookSetupInfo) => HookConnectionDraft
	) {
		if (!activeSetup) return;
		const st = ensureHookDraftState(activeSetup);
		const next = [...st.drafts];
		next[index] = updater(next[index], activeSetup);
		hookDraftState = { ...st, drafts: next };
	}

	function setEnableAfterSetup(checked: boolean) {
		if (!activeSetup) return;
		const st = ensureHookDraftState(activeSetup);
		hookDraftState = { ...st, enableHook: checked };
	}

	function setShowAdvancedFields(checked: boolean) {
		if (!activeSetup) return;
		const st = ensureHookDraftState(activeSetup);
		hookDraftState = { ...st, showAdvanced: checked };
	}

	function addHookConnection() {
		if (!activeSetup) return;
		const st = ensureHookDraftState(activeSetup);
		hookDraftState = {
			...st,
			drafts: [...st.drafts, addBlankConnection(activeSetup.descriptor, st.drafts)]
		};
	}

	function removeHookConnection(index: number) {
		if (!activeSetup) return;
		const st = ensureHookDraftState(activeSetup);
		hookDraftState = {
			...st,
			drafts: st.drafts.filter((_, i) => i !== index)
		};
	}

	function updateHookFieldValue(index: number, fieldId: string, nextValue: string) {
		updateDraftAt(index, (draft, setup) => {
			const updated: HookConnectionDraft = {
				...draft,
				connection: {
					...draft.connection,
					values: { ...draft.connection.values, [fieldId]: nextValue }
				}
			};
			const fieldDef = setup.descriptor.fields.find((f) => f.id === fieldId);
			if (fieldDef?.credential_scope) {
				const orig = setup.connections.find((c) => c.connection.id === draft.connection.id);
				const prevVal = orig?.connection.values[fieldId] ?? '';
				if (prevVal && prevVal !== nextValue) {
					const creds = { ...updated.credentials };
					for (const sf of setup.descriptor.fields) {
						if (sf.kind === 'secret' && creds[sf.id]?.action === 'keep') {
							creds[sf.id] = { action: 'remove' };
						}
					}
					return deriveConnectionFields({ ...updated, credentials: creds }, setup.descriptor);
				}
			}
			return deriveConnectionFields(updated, setup.descriptor);
		});
	}

	function updateHookSecretValue(index: number, fieldId: string, secretText: string) {
		updateDraftAt(index, (draft, setup) => {
			const creds = { ...draft.credentials };
			const origHasCred = Boolean(
				setup.connections.find((c) => c.connection.id === draft.connection.id)?.credentials[fieldId]
			);
			if (secretText.trim()) {
				creds[fieldId] = { action: 'replace', value: secretText };
			} else if (origHasCred) {
				creds[fieldId] = { action: 'keep' };
			} else {
				delete creds[fieldId];
			}
			return { ...draft, credentials: creds };
		});
	}

	function setHookSecretAction(index: number, fieldId: string, action: 'keep' | 'remove') {
		updateDraftAt(index, (draft) => ({
			...draft,
			credentials: {
				...draft.credentials,
				[fieldId]: { action }
			}
		}));
	}

	async function submitHookSetup() {
		if (!activeSetup || !activeSetup.writable) return;
		await session.saveHookSetup({
			id: activeSetup.id,
			revision: activeSetup.revision,
			connections: activeDrafts,
			enable: enableAfterSetup
		});
	}

	function openAddMcpForm() {
		showAddMcp = true;
		mcpId = '';
		mcpDescription = '';
		mcpTransport = 'stdio';
		mcpCommand = '';
		mcpArgs = '';
		mcpCwd = '';
		mcpUrl = '';
		mcpSandbox = 'inherit';
		mcpEnvEntries = [];
		mcpHeaderEntries = [
			{ id: nextBindingSeq++, key: 'Authorization', mode: 'env', value: '', format: 'Bearer {}' }
		];
	}

	function addBindingRow(kind: 'env' | 'header') {
		const row: McpBindingEntry =
			kind === 'env'
				? { id: nextBindingSeq++, key: '', mode: 'env', value: '', format: '' }
				: { id: nextBindingSeq++, key: '', mode: 'env', value: '', format: 'Bearer {}' };
		if (kind === 'env') mcpEnvEntries = [...mcpEnvEntries, row];
		else mcpHeaderEntries = [...mcpHeaderEntries, row];
	}

	function removeBindingRow(kind: 'env' | 'header', id: number) {
		if (kind === 'env') mcpEnvEntries = mcpEnvEntries.filter((r) => r.id !== id);
		else mcpHeaderEntries = mcpHeaderEntries.filter((r) => r.id !== id);
	}

	const canSubmitMcp = $derived.by(() => {
		if (mcpBusy || !mcpId.trim()) return false;
		if (mcpTransport === 'stdio') return mcpCommand.trim().length > 0;
		return mcpUrl.trim().length > 0;
	});

	async function submitAddMcp(e: SubmitEvent) {
		e.preventDefault();
		if (!canSubmitMcp) return;
		const ok = await session.saveMcpServer({
			id: mcpId.trim(),
			scope: writeScope,
			description: mcpDescription.trim(),
			command: mcpTransport === 'stdio' ? mcpCommand.trim() : '',
			args: mcpTransport === 'stdio' ? parseArgsString(mcpArgs) : [],
			cwd: mcpTransport === 'stdio' ? mcpCwd.trim() : '',
			url: mcpTransport === 'http' ? mcpUrl.trim() : '',
			...(mcpTransport === 'stdio' && mcpSandbox !== 'inherit'
				? { sandbox: mcpSandbox === 'on' }
				: {}),
			env: mcpTransport === 'stdio' ? buildMcpSecretMap(mcpEnvEntries) : {},
			headers: mcpTransport === 'http' ? buildMcpSecretMap(mcpHeaderEntries) : {}
		});
		if (ok) {
			showAddMcp = false;
			mcpId = '';
			mcpDescription = '';
			mcpCommand = '';
			mcpArgs = '';
			mcpCwd = '';
			mcpUrl = '';
		}
	}
</script>

<section class="plate" aria-label="MCP servers">
	<div class="plate-head">
		<div>
			<h4 class="plate-title">MCP servers ({mcpServers.length})</h4>
			<p class="plate-sub">
				Configured in <code>mcp.d/*.toml</code> under
				{#if writeScope === 'project' && info?.paths?.project}
					<code>{info.paths.project}/mcp.d</code>.
				{:else if info?.paths?.user_config}
					<code>{info.paths.user_config}/mcp.d</code>.
				{:else}
					user or project <code>mcp.d/</code>.
				{/if}
			</p>
		</div>
		<div class="head-actions">
			<button
				type="button"
				class="btn btn-sm"
				disabled={session.closed}
				onclick={() => void session.refreshMcp()}
			>
				Refresh status
			</button>
			<button
				type="button"
				class="btn btn-sm btn-primary"
				disabled={mcpBusy}
				onclick={() => (showAddMcp ? (showAddMcp = false) : openAddMcpForm())}
			>
				{showAddMcp ? 'Cancel new server' : '+ Add MCP server'}
			</button>
		</div>
	</div>

	{#if session.mcpError}
		<div class="inline-banner err" role="alert">{session.mcpError}</div>
	{:else if session.mcpFeedback}
		<div class="inline-banner ok" role="status">{session.mcpFeedback}</div>
	{/if}

	{#if showAddMcp}
		<form class="subform" onsubmit={submitAddMcp} aria-label="Add MCP server">
			<div class="subform-head">
				<strong>New MCP server (<code>{writeScope}</code> layer)</strong>
				<div class="segmented" role="group" aria-label="MCP transport">
					<button
						type="button"
						class={['seg-btn', { active: mcpTransport === 'stdio' }]}
						onclick={() => (mcpTransport = 'stdio')}
					>
						Local (stdio)
					</button>
					<button
						type="button"
						class={['seg-btn', { active: mcpTransport === 'http' }]}
						onclick={() => (mcpTransport = 'http')}
					>
						Remote (HTTP)
					</button>
				</div>
			</div>

			<div class="form-grid">
				<label class="field">
					<span>Server ID</span>
					<input
						type="text"
						class="ctrl"
						required
						placeholder="e.g. github or local-tools"
						bind:value={mcpId}
						disabled={mcpBusy}
					/>
				</label>
				<label class="field">
					<span>Description</span>
					<input
						type="text"
						class="ctrl"
						placeholder="Short description of provided tools"
						bind:value={mcpDescription}
						disabled={mcpBusy}
					/>
				</label>

				{#if mcpTransport === 'stdio'}
					<label class="field">
						<span>Command</span>
						<input
							type="text"
							class="ctrl mono"
							required
							placeholder="npx"
							bind:value={mcpCommand}
							disabled={mcpBusy}
						/>
					</label>
					<label class="field">
						<span>Arguments</span>
						<input
							type="text"
							class="ctrl mono"
							placeholder="-y @modelcontextprotocol/server-github"
							bind:value={mcpArgs}
							disabled={mcpBusy}
						/>
					</label>
					<label class="field">
						<span>Working directory (optional)</span>
						<input
							type="text"
							class="ctrl mono"
							placeholder="Leave blank for workspace root"
							bind:value={mcpCwd}
							disabled={mcpBusy}
						/>
					</label>
					<label class="field">
						<span>Sandbox</span>
						<select class="ctrl" bind:value={mcpSandbox} disabled={mcpBusy}>
							<option value="inherit">inherit (default)</option>
							<option value="on">enabled</option>
							<option value="off">disabled</option>
						</select>
					</label>
				{:else}
					<label class="field field-wide">
						<span>Endpoint URL</span>
						<input
							type="url"
							class="ctrl mono"
							required
							placeholder="https://api.example.com/mcp"
							bind:value={mcpUrl}
							disabled={mcpBusy}
						/>
					</label>
				{/if}
			</div>

			{#if mcpTransport === 'stdio'}
				<div class="bindings-block">
					<div class="bindings-head">
						<span>Environment variables (<code>env</code>)</span>
						<button type="button" class="btn btn-sm" onclick={() => addBindingRow('env')}>
							+ Add env var
						</button>
					</div>
					{#each mcpEnvEntries as entry (entry.id)}
						<div class="binding-row">
							<input
								type="text"
								class="ctrl mono"
								placeholder="ENV_VAR_KEY"
								aria-label="Env key"
								bind:value={entry.key}
							/>
							<select class="ctrl" aria-label="Binding type" bind:value={entry.mode}>
								<option value="env">from env var</option>
								<option value="keyring">from keyring</option>
								<option value="plain">literal text</option>
							</select>
							<input
								type="text"
								class="ctrl mono"
								placeholder={entry.mode === 'env' ? 'SOURCE_ENV_NAME' : entry.mode === 'keyring' ? 'keyring-key' : 'value'}
								aria-label="Env source"
								bind:value={entry.value}
							/>
							<button
								type="button"
								class="btn btn-sm"
								onclick={() => removeBindingRow('env', entry.id)}
							>
								Remove
							</button>
						</div>
					{/each}
				</div>
			{:else}
				<div class="bindings-block">
					<div class="bindings-head">
						<span>HTTP headers (<code>headers</code>)</span>
						<button type="button" class="btn btn-sm" onclick={() => addBindingRow('header')}>
							+ Add header
						</button>
					</div>
					{#each mcpHeaderEntries as entry (entry.id)}
						<div class="binding-row">
							<input
								type="text"
								class="ctrl mono"
								placeholder="Authorization"
								aria-label="Header name"
								bind:value={entry.key}
							/>
							<select class="ctrl" aria-label="Header binding type" bind:value={entry.mode}>
								<option value="env">from env var</option>
								<option value="keyring">from keyring</option>
								<option value="plain">literal text</option>
							</select>
							<input
								type="text"
								class="ctrl mono"
								placeholder={entry.mode === 'env' ? 'API_TOKEN_ENV' : 'secret-id'}
								aria-label="Header source"
								bind:value={entry.value}
							/>
							{#if entry.mode === 'env'}
								<input
									type="text"
									class="ctrl mono"
									placeholder="Bearer &#123;&#125;"
									aria-label="Format template"
									bind:value={entry.format}
								/>
							{/if}
							<button
								type="button"
								class="btn btn-sm"
								onclick={() => removeBindingRow('header', entry.id)}
							>
								Remove
							</button>
						</div>
					{/each}
				</div>
			{/if}

			<div class="subform-foot">
				<button type="submit" class="btn btn-sm btn-primary" disabled={!canSubmitMcp}>
					{session.mcpSaving ? 'Saving…' : `Save mcp.d/${mcpId.trim() || 'server'}.toml`}
				</button>
			</div>
		</form>
	{/if}

	{#if mcpServers.length === 0}
		<p class="empty-note">
			No MCP servers found in loaded layers. Add a TOML server above or place definitions in
			<code>mcp.d/</code>.
		</p>
	{:else}
		<div class="ext-list">
			{#each mcpServers as srv (srv.id)}
				<article class="ext-card">
					<div class="ext-top">
						<div class="ext-title-wrap">
							<code class="ext-id">{srv.id}</code>
							<span class="tag">{srv.transport}</span>
							<span class="tag">{srv.layer}</span>
							<span
								class={[
									'tag',
									'state-tag',
									{
										'is-ok': srv.state === 'ready' || srv.state === 'enabled',
										'is-err': srv.state === 'failed' || srv.blockedByPolicy,
										'is-muted': srv.state === 'disabled' || srv.state === 'stopped'
									}
								]}
							>
								{srv.blockedByPolicy ? 'blocked by policy' : srv.state}
							</span>
							{#if srv.deferred}
								<span class="tag">deferred</span>
							{/if}
							{#if srv.toolTokens > 0}
								<span class="tag">~{srv.toolTokens} tok</span>
							{/if}
						</div>

						<div class="ext-actions">
							{#if srv.enabled}
								<button
									type="button"
									class="btn btn-sm"
									disabled={session.closed}
									onclick={() => void session.restartMcp(srv.id)}
								>
									Restart
								</button>
								<button
									type="button"
									class="btn btn-sm"
									disabled={mcpBusy || !srv.canToggle}
									onclick={() => void session.disableMcp(srv.id, writeScope)}
								>
									Disable
								</button>
							{:else}
								<button
									type="button"
									class="btn btn-sm btn-primary"
									disabled={mcpBusy || !srv.canToggle}
									title={srv.blockedByPolicy
										? 'Blocked by enterprise/system policy'
										: srv.canToggle
											? `Enable ${srv.id} and reload`
											: 'Managed by system or invalid file'}
									onclick={() => void session.enableMcp(srv.id)}
								>
									Enable
								</button>
							{/if}
						</div>
					</div>

					{#if srv.description}
						<p class="ext-desc">{srv.description}</p>
					{/if}
					<div class="ext-meta">
						{#if srv.target}
							<code class="ext-target">{srv.target}</code>
						{/if}
						{#if srv.detail && srv.detail !== 'enabled'}
							<span class={['ext-detail', { 'is-err': srv.state === 'failed' || srv.blockedByPolicy }]}>
								{srv.detail}
							</span>
						{/if}
					</div>
					{#if srv.file || srv.envKeys.length > 0 || srv.headerKeys.length > 0 || srv.autoApprove.length > 0}
						<div class="ext-chips">
							{#if srv.file}
								<code class="ext-file">{srv.file}</code>
							{/if}
							{#if srv.envKeys.length > 0}
								<span class="chip">env: {srv.envKeys.join(', ')}</span>
							{/if}
							{#if srv.headerKeys.length > 0}
								<span class="chip">headers: {srv.headerKeys.join(', ')}</span>
							{/if}
							{#if srv.autoApprove.length > 0}
								<span class="chip">auto-approve: {srv.autoApprove.join(', ')}</span>
							{/if}
							{#if srv.sandbox !== null}
								<span class="chip">sandbox: {srv.sandbox ? 'on' : 'off'}</span>
							{/if}
						</div>
					{/if}

					{#if srv.tools.length > 0 || srv.prompts.length > 0 || srv.resources.length > 0 || srv.log.length > 0}
						<details class="ext-details">
							<summary>
								Runtime capabilities: {srv.tools.length} tool{srv.tools.length === 1 ? '' : 's'}
								{#if srv.prompts.length > 0} · {srv.prompts.length} prompt(s){/if}
								{#if srv.resources.length > 0} · {srv.resources.length} resource(s){/if}
							</summary>
							<div class="details-body">
								{#if srv.tools.length > 0}
									<div>
										<strong>Tools:</strong>
										{#each srv.tools as tool (tool)}
											<code class="cap-tag">{tool}</code>
										{/each}
									</div>
								{/if}
								{#if srv.prompts.length > 0}
									<div>
										<strong>Prompts:</strong>
										{#each srv.prompts as p (p)}
											<code class="cap-tag">/{srv.id}:{p}</code>
										{/each}
									</div>
								{/if}
								{#if srv.resources.length > 0}
									<div>
										<strong>Resources:</strong>
										{#each srv.resources as res (res)}
											<code class="cap-tag">{res}</code>
										{/each}
									</div>
								{/if}
								{#if srv.log.length > 0}
									<pre class="ext-log">{srv.log.join('\n')}</pre>
								{/if}
							</div>
						</details>
					{/if}
				</article>
			{/each}
		</div>
	{/if}
</section>

<section class="plate" aria-label="Lifecycle hooks">
	<div class="plate-head">
		<div>
			<h4 class="plate-title">Lifecycle hooks ({hooks.length})</h4>
			<p class="plate-sub">
				Run scripts at lifecycle events (<code>hooks.d/*.toml</code>). System hooks stay
				enforced; user &amp; defaults hooks can be enabled or customized.
			</p>
		</div>
	</div>

	{#if session.hookError}
		<div class="inline-banner err" role="alert">{session.hookError}</div>
	{:else if session.hookFeedback}
		<div class="inline-banner ok" role="status">{session.hookFeedback}</div>
	{/if}

	{#if session.hookPayloadChoice}
		{@const choice = session.hookPayloadChoice}
		<div class="payload-prompt" role="dialog" aria-label="Existing hook script">
			<p>
				Hook <code>{choice.id}</code> already has a local companion script at
				<code>{choice.path}</code>. Keep your copy or reset it to the bundled template?
			</p>
			<div class="head-actions">
				<button
					type="button"
					class="btn btn-sm btn-primary"
					disabled={hookBusy}
					onclick={() => void session.enableHook(choice.id, false)}
				>
					Keep my file
				</button>
				<button
					type="button"
					class="btn btn-sm"
					disabled={hookBusy}
					onclick={() => void session.enableHook(choice.id, true)}
				>
					Replace with bundled
				</button>
				<button
					type="button"
					class="btn btn-sm"
					onclick={() => session.dismissHookPayloadChoice()}
				>
					Cancel
				</button>
			</div>
		</div>
	{/if}

	{#if hooks.length === 0}
		<p class="empty-note">No hooks configured in <code>hooks.d/</code> across active layers.</p>
	{:else}
		<div class="ext-list">
			{#each hooks as hook (hook.id)}
				<article class="ext-card">
					<div class="ext-top">
						<div class="ext-title-wrap">
							<code class="ext-id">{hook.id}</code>
							{#if hook.event}
								<span class="tag">{hook.event}</span>
							{/if}
							<span class="tag">{hook.layer}</span>
							<span
								class={[
									'tag',
									'state-tag',
									{
										'is-ok': hook.enabled,
										'is-err': hook.state === 'failed',
										'is-muted': !hook.enabled && hook.state !== 'failed'
									}
								]}
							>
								{hook.state}
							</span>
							{#if hook.connections_count > 0}
								<span class="tag">
									{hook.connections_count} connection{hook.connections_count === 1 ? '' : 's'}
								</span>
							{/if}
						</div>
						<div class="ext-actions">
							{#if hook.has_setup}
								<button
									type="button"
									class="btn btn-sm"
									disabled={session.closed || session.hookSetupLoading}
									onclick={() => void session.inspectHookSetup(hook.id)}
								>
									{session.hookSetupLoading && activeSetup?.id === hook.id
										? 'Loading…'
										: 'Configure…'}
								</button>
							{/if}
							{#if hook.layer === 'system'}
								<span class="chip">Enforced</span>
							{:else if hook.enabled}
								<button
									type="button"
									class="btn btn-sm"
									disabled={hookBusy || !hook.can_toggle}
									onclick={() => void session.disableHook(hook.id, writeScope)}
								>
									Disable
								</button>
							{:else}
								<button
									type="button"
									class="btn btn-sm btn-primary"
									disabled={hookBusy || !hook.can_toggle}
									onclick={() => void session.enableHook(hook.id)}
								>
									Enable
								</button>
							{/if}
						</div>
					</div>

					{#if hook.description}
						<p class="ext-desc">{hook.description}</p>
					{/if}
					<div class="ext-meta">
						{#if hook.command.length > 0}
							<code class="ext-target">{hook.command.join(' ')}</code>
						{/if}
						{#if hook.detail && hook.detail !== 'on'}
							<span class={['ext-detail', { 'is-err': hook.state === 'failed' }]}>
								{hook.detail}
							</span>
						{/if}
					</div>
					<div class="ext-chips">
						<code class="ext-file">{hook.file}</code>
						{#if hook.matcher_tool}
							<span class="chip">tool: {hook.matcher_tool}</span>
						{/if}
						{#if hook.matcher_pattern}
							<span class="chip">pattern: {hook.matcher_pattern}</span>
						{/if}
						{#if hook.matcher_prompt}
							<span class="chip">prompt: {hook.matcher_prompt}</span>
						{/if}
						{#if hook.on_error}
							<span class="chip">on_error: {hook.on_error}</span>
						{/if}
						{#if hook.timeout_secs !== null}
							<span class="chip">timeout: {hook.timeout_secs}s</span>
						{/if}
						{#if hook.full_context}
							<span class="chip">full_context</span>
						{/if}
					</div>
				</article>
			{/each}
		</div>
	{/if}

	{#if session.hookSetupLoading && !activeSetup}
		<div class="setup-drawer">
			<p class="plate-sub">Loading hook connection schema…</p>
		</div>
	{/if}

	{#if activeSetup}
		<div class="setup-drawer" aria-label="Hook connection setup">
			<div class="subform-head">
				<div>
					<strong>Configure hook: <code>{activeSetup.id}</code></strong>
					<span class="tag">{activeSetup.source}</span>
					{#if !activeSetup.writable}
						<span class="tag state-tag is-err">read-only</span>
					{/if}
				</div>
				<div class="head-actions">
					{#if activeSetup.writable}
						<button type="button" class="btn btn-sm" onclick={addHookConnection}>
							+ Add connection
						</button>
					{/if}
					<button type="button" class="btn btn-sm" onclick={() => session.closeHookSetup()}>
						Close
					</button>
				</div>
			</div>

			{#if activeSetup.problem}
				<div class="inline-banner err" role="alert">{activeSetup.problem}</div>
			{/if}
			{#if session.hookSetupError}
				<div class="inline-banner err" role="alert">{session.hookSetupError}</div>
			{:else if session.hookSetupStatus}
				<div
					class={[
						'inline-banner',
						{
							ok: session.hookSetupStatus.kind === 'ok',
							err: session.hookSetupStatus.kind === 'fail',
							notice: session.hookSetupStatus.kind === 'notice'
						}
					]}
					role="status"
				>
					{session.hookSetupStatus.text}
				</div>
			{/if}

			{#if hasAnyAdvancedField}
				<label class="inline-check">
					<input
						type="checkbox"
						checked={showAdvancedFields}
						onchange={(e) => setShowAdvancedFields(e.currentTarget.checked)}
					/>
					<span>Show advanced fields</span>
				</label>
			{/if}

			{#if activeDrafts.length === 0}
				<p class="empty-note">
					No connections configured yet. Click <strong>+ Add connection</strong> to create one.
				</p>
			{:else}
				<div class="conn-list">
					{#each activeDrafts as draft, i (i)}
						{@const origInfo = activeSetup.connections.find(
							(c) => c.connection.id === draft.connection.id
						)}
						{@const activePreset = activeSetup.descriptor.presets.find(
							(p) => p.id === draft.connection.preset
						)}
						<div class="conn-card">
							<div class="conn-head">
								<label class="conn-id-field">
									<span>Connection ID</span>
									<input
										type="text"
										class="ctrl mono"
										value={draft.connection.id}
										disabled={!activeSetup.writable || session.hookSetupSaving}
										oninput={(e) => {
											const val = e.currentTarget.value;
											updateDraftAt(i, (d) => ({
												...d,
												connection: { ...d.connection, id: val }
											}));
										}}
									/>
								</label>

								{#if activeSetup.descriptor.presets.length > 0}
									<label class="conn-id-field">
										<span>Preset</span>
										<select
											class="ctrl"
											value={draft.connection.preset}
											disabled={!activeSetup.writable || session.hookSetupSaving}
											onchange={(e) => {
												const presetId = e.currentTarget.value;
												updateDraftAt(i, (d, s) =>
													applyPresetToDraft(d, s.descriptor, presetId)
												);
											}}
										>
											{#if !draft.connection.preset}
												<option value="">Custom</option>
											{/if}
											{#each activeSetup.descriptor.presets as preset (preset.id)}
												<option value={preset.id}>{preset.label}</option>
											{/each}
										</select>
									</label>
								{/if}

								<label class="inline-check">
									<input
										type="checkbox"
										checked={draft.connection.enabled}
										disabled={!activeSetup.writable || session.hookSetupSaving}
										onchange={(e) => {
											const checked = e.currentTarget.checked;
											updateDraftAt(i, (d) => ({
												...d,
												connection: { ...d.connection, enabled: checked }
											}));
										}}
									/>
									<span>Enabled</span>
								</label>

								{#if activeSetup.writable}
									<button
										type="button"
										class="btn btn-sm"
										disabled={session.hookSetupSaving}
										onclick={() => removeHookConnection(i)}
									>
										Remove
									</button>
								{/if}
							</div>

							{#if activePreset?.help}
								<p class="preset-help">{activePreset.help}</p>
							{/if}

							<div class="form-grid">
								{#each activeSetup.descriptor.fields as field (field.id)}
									{#if isFieldVisible(field, draft.connection.values, showAdvancedFields)}
										{@const required = isFieldRequired(field, draft.connection.values)}
										{@const currentVal = draft.connection.values[field.id] ?? field.default ?? ''}
										<div class="field">
											<label for={`hf-${i}-${field.id}`}>
												{field.label}
												{#if required}<span class="req">*</span>{/if}
											</label>

											{#if field.kind === 'boolean'}
												<label class="inline-check">
													<input
														id={`hf-${i}-${field.id}`}
														type="checkbox"
														checked={currentVal === 'true'}
														disabled={!activeSetup.writable || session.hookSetupSaving}
														onchange={(e) =>
															updateHookFieldValue(
																i,
																field.id,
																e.currentTarget.checked ? 'true' : 'false'
															)}
													/>
													<span>{field.help || 'Enable'}</span>
												</label>
											{:else if field.kind === 'choice'}
												{@const choices = fieldChoices(field, activePreset)}
												<select
													id={`hf-${i}-${field.id}`}
													class="ctrl"
													value={currentVal}
													disabled={!activeSetup.writable || session.hookSetupSaving}
													onchange={(e) => updateHookFieldValue(i, field.id, e.currentTarget.value)}
												>
													{#if !choices.includes(currentVal)}
														<option value={currentVal}>{currentVal || '(select)'}</option>
													{/if}
													{#each choices as ch (ch)}
														<option value={ch}>{ch}</option>
													{/each}
												</select>
											{:else if field.kind === 'secret'}
												{@const credChange = draft.credentials[field.id]}
												{@const existingSource = origInfo?.credentials[field.id]}
												<div class="secret-col">
													<input
														id={`hf-${i}-${field.id}`}
														type="password"
														autocomplete="off"
														class="ctrl"
														placeholder={existingSource
															? `Saved via ${formatKeySource(existingSource)} — type to replace`
															: 'Enter secret value'}
														value={credChange?.action === 'replace' ? credChange.value : ''}
														disabled={!activeSetup.writable || session.hookSetupSaving}
														oninput={(e) => updateHookSecretValue(i, field.id, e.currentTarget.value)}
													/>
													{#if existingSource || credChange?.action === 'remove'}
														<div class="secret-actions">
															<span class="field-help">
																{#if credChange?.action === 'remove'}
																	Will be cleared on save.
																{:else if credChange?.action === 'replace'}
																	Replacing {formatKeySource(existingSource)}.
																{:else}
																	Current: {formatKeySource(existingSource)}.
																{/if}
															</span>
															{#if existingSource && credChange?.action !== 'remove'}
																<button
																	type="button"
																	class="btn-link"
																	onclick={() => setHookSecretAction(i, field.id, 'remove')}
																>
																	Clear saved secret
																</button>
															{:else if existingSource && credChange?.action === 'remove'}
																<button
																	type="button"
																	class="btn-link"
																	onclick={() => setHookSecretAction(i, field.id, 'keep')}
																>
																	Keep existing
																</button>
															{/if}
														</div>
													{/if}
												</div>
											{:else}
												<input
													id={`hf-${i}-${field.id}`}
													type={field.kind === 'url' ? 'url' : field.kind === 'integer' ? 'number' : 'text'}
													class={['ctrl', { mono: field.kind === 'url' }]}
													placeholder={field.default || ''}
													value={currentVal}
													disabled={!activeSetup.writable || session.hookSetupSaving}
													oninput={(e) => updateHookFieldValue(i, field.id, e.currentTarget.value)}
												/>
											{/if}

											{#if field.help && field.kind !== 'boolean'}
												<span class="field-help">{field.help}</span>
											{/if}
										</div>
									{/if}
								{/each}
							</div>

							<div class="conn-foot">
								{#if session.hookSetupChecking}
									<button
										type="button"
										class="btn btn-sm"
										onclick={() => void session.cancelHookCheck()}
									>
										Cancel check
									</button>
								{:else}
									<button
										type="button"
										class="btn btn-sm"
										disabled={session.hookSetupSaving}
										onclick={() =>
											void session.checkHookSetup({
												id: activeSetup.id,
												revision: activeSetup.revision,
												connection: draft,
												url: connectionCheckUrl(draft, activeSetup.descriptor)
											})}
									>
										Test connection
									</button>
								{/if}
							</div>
						</div>
					{/each}
				</div>
			{/if}

			{#if activeSetup.writable}
				<div class="subform-foot between">
					<label class="inline-check">
						<input
							type="checkbox"
							checked={enableAfterSetup}
							disabled={session.hookSetupSaving}
							onchange={(e) => setEnableAfterSetup(e.currentTarget.checked)}
						/>
						<span>Enable hook after saving</span>
					</label>
					<button
						type="button"
						class="btn btn-sm btn-primary"
						disabled={session.hookSetupSaving}
						onclick={() => void submitHookSetup()}
					>
						{session.hookSetupSaving ? 'Saving…' : 'Save connections'}
					</button>
				</div>
			{/if}
		</div>
	{/if}

	{#if session.hookRuns.length > 0}
		<div class="runs-block">
			<h5>Recent hook runs</h5>
			<ul class="runs-list">
				{#each session.hookRuns as run, i (`${run.id}-${run.at}-${i}`)}
					<li>
						<code>{run.id}</code>
						<span class="tag">{run.on}</span>
						<span
							class={[
								'tag',
								{
									'is-ok': run.outcome === 'ok' || run.outcome === 'allowed' || run.outcome === 'context',
									'is-err': run.outcome === 'failed' || run.outcome === 'denied'
								}
							]}
						>
							{run.outcome}
						</span>
						<span class="muted">{run.ms}ms</span>
						{#if run.contextTokens > 0}
							<span class="muted">+{run.contextTokens} tok</span>
						{/if}
						{#if run.message}
							<span class="run-msg">{run.message}</span>
						{/if}
					</li>
				{/each}
			</ul>
		</div>
	{/if}
</section>

<style>
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
	.plate-title {
		margin: 0 0 2px;
		font-family: var(--font-serif);
		font-size: 0.9375rem;
		font-weight: 600;
		color: var(--ink);
	}
	.plate-sub,
	.empty-note {
		margin: 0;
		font-size: 0.78125rem;
		color: var(--ink-3);
		line-height: 1.45;
	}
	.head-actions {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}

	.inline-banner {
		padding: 7px 10px;
		border-radius: var(--r-md);
		font-size: 0.78125rem;
		margin-bottom: 10px;
		border: 1px solid var(--rule);
	}
	.inline-banner.ok {
		background: var(--ok-soft, color-mix(in srgb, var(--ok) 12%, transparent));
		border-color: var(--ok);
		color: var(--ink);
	}
	.inline-banner.err {
		background: var(--danger-soft, color-mix(in srgb, var(--danger) 12%, transparent));
		border-color: var(--danger);
		color: var(--danger);
	}
	.inline-banner.notice {
		background: var(--bg-soft);
		color: var(--ink-2);
	}

	.ext-list,
	.conn-list {
		display: flex;
		flex-direction: column;
		gap: 8px;
	}
	.ext-card,
	.conn-card,
	.subform,
	.setup-drawer,
	.payload-prompt {
		padding: 10px 12px;
		border: 1px solid var(--rule-soft);
		border-radius: var(--r-lg);
		background: var(--bg-soft);
	}
	.subform,
	.setup-drawer,
	.payload-prompt {
		margin: 10px 0 12px;
		border-color: var(--rule);
		background: var(--bg-panel);
	}
	.ext-top,
	.subform-head,
	.conn-head,
	.bindings-head {
		display: flex;
		align-items: center;
		justify-content: space-between;
		flex-wrap: wrap;
		gap: 8px;
	}
	.subform-head {
		margin-bottom: 10px;
	}
	.ext-title-wrap {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px;
		min-width: 0;
	}
	.ext-id {
		font-weight: 700;
		font-size: 0.8125rem;
		color: var(--ink);
	}
	.ext-actions {
		display: flex;
		align-items: center;
		gap: 6px;
		flex-wrap: wrap;
	}
	.ext-desc {
		margin: 5px 0 0;
		font-size: 0.78125rem;
		color: var(--ink-2);
		line-height: 1.4;
	}
	.ext-meta {
		margin-top: 5px;
		display: flex;
		flex-wrap: wrap;
		gap: 6px 10px;
		align-items: center;
	}
	.ext-target,
	.ext-file {
		font-size: 0.71875rem;
		color: var(--ink-3);
		overflow-wrap: anywhere;
	}
	.ext-detail {
		font-size: 0.75rem;
		color: var(--ink-2);
	}
	.ext-detail.is-err {
		color: var(--danger);
	}
	.ext-chips {
		margin-top: 6px;
		display: flex;
		flex-wrap: wrap;
		gap: 5px 8px;
		align-items: center;
	}
	.chip,
	.tag {
		font-size: 0.6875rem;
		padding: 1px 6px;
		border-radius: var(--r-sm);
		background: var(--bg-raised);
		border: 1px solid var(--rule-soft);
		color: var(--ink-2);
	}
	.tag.is-ok {
		border-color: var(--ok);
		color: var(--ok);
	}
	.tag.is-err {
		border-color: var(--danger);
		color: var(--danger);
	}
	.tag.is-muted {
		color: var(--ink-3);
	}

	.ext-details {
		margin-top: 8px;
		font-size: 0.75rem;
		color: var(--ink-2);
	}
	.ext-details summary {
		cursor: pointer;
		color: var(--sky-ink);
		font-weight: 600;
	}
	.details-body {
		margin-top: 6px;
		display: flex;
		flex-direction: column;
		gap: 5px;
	}
	.cap-tag {
		display: inline-block;
		margin: 2px 4px 2px 0;
		padding: 1px 5px;
		font-size: 0.6875rem;
		border-radius: var(--r-sm);
		background: var(--bg-raised);
		border: 1px solid var(--rule-soft);
	}
	.ext-log {
		margin: 4px 0 0;
		padding: 6px 8px;
		max-height: 140px;
		overflow: auto;
		font-size: 0.6875rem;
		border-radius: var(--r-sm);
		background: var(--bg-raised);
		border: 1px solid var(--rule-soft);
	}

	.segmented {
		display: inline-flex;
		padding: 2px;
		border-radius: var(--r-lg);
		border: 1px solid var(--rule);
		background: var(--bg-soft);
	}
	.seg-btn {
		padding: 3px 9px;
		border-radius: var(--r-md);
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--ink-2);
	}
	.seg-btn.active {
		background: var(--bg-raised);
		color: var(--ink);
		box-shadow: 0 1px 2px rgba(0, 0, 0, 0.08);
	}

	.form-grid {
		display: grid;
		grid-template-columns: repeat(auto-fit, minmax(220px, 1fr));
		gap: 10px 12px;
	}
	.field {
		display: flex;
		flex-direction: column;
		gap: 4px;
		font-size: 0.75rem;
		font-weight: 600;
		color: var(--ink-2);
	}
	.field-wide {
		grid-column: 1 / -1;
	}
	.ctrl {
		width: 100%;
		min-width: 0;
		min-height: 1.875rem;
		padding: 4px 8px;
		border-radius: var(--r-md);
		border: 1px solid var(--rule);
		background-color: var(--bg-raised);
		color: var(--ink);
		font-size: 0.78125rem;
		line-height: 1.4;
	}
	select.ctrl {
		padding-right: 28px;
		background-position: right 8px center;
	}
	.ctrl.mono {
		font-family: var(--font-mono);
		font-size: 0.75rem;
	}
	.req {
		color: var(--danger);
	}
	.field-help,
	.preset-help {
		font-size: 0.71875rem;
		font-weight: 400;
		color: var(--ink-3);
		margin: 0;
	}
	.preset-help {
		margin: 4px 0 8px;
	}

	.bindings-block {
		margin-top: 10px;
		display: flex;
		flex-direction: column;
		gap: 6px;
		font-size: 0.75rem;
		color: var(--ink-2);
	}
	.binding-row {
		display: grid;
		grid-template-columns: 1fr 132px 1fr auto;
		gap: 6px;
		align-items: center;
	}
	.subform-foot,
	.conn-foot {
		margin-top: 10px;
		display: flex;
		justify-content: flex-end;
		align-items: center;
		gap: 8px;
		flex-wrap: wrap;
	}
	.subform-foot.between {
		justify-content: space-between;
	}
	.conn-id-field {
		display: flex;
		align-items: center;
		gap: 6px;
		font-size: 0.75rem;
		color: var(--ink-2);
		font-weight: 600;
	}
	.inline-check {
		display: inline-flex;
		align-items: center;
		gap: 6px;
		font-size: 0.75rem;
		color: var(--ink-2);
		font-weight: 500;
	}
	.secret-col {
		display: flex;
		flex-direction: column;
		gap: 4px;
	}
	.secret-actions {
		display: flex;
		align-items: center;
		justify-content: space-between;
		gap: 8px;
		flex-wrap: wrap;
	}
	.btn-link {
		font-size: 0.71875rem;
		color: var(--sky-ink);
		text-decoration: underline;
	}
	.btn-sm {
		padding: 4px 10px;
		font-size: 0.75rem;
		min-height: 1.875rem;
		white-space: nowrap;
	}

	.runs-block {
		margin-top: 14px;
		padding-top: 10px;
		border-top: 1px solid var(--rule-soft);
	}
	.runs-block h5 {
		margin: 0 0 6px;
		font-size: 0.78125rem;
		color: var(--ink-2);
	}
	.runs-list {
		list-style: none;
		padding: 0;
		margin: 0;
		display: flex;
		flex-direction: column;
		gap: 5px;
		font-size: 0.75rem;
	}
	.runs-list li {
		display: flex;
		align-items: center;
		flex-wrap: wrap;
		gap: 6px;
	}
	.muted {
		color: var(--ink-3);
	}
	.run-msg {
		color: var(--ink-2);
		overflow-wrap: anywhere;
	}
</style>
