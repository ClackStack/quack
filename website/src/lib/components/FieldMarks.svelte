<script lang="ts">
	import Plate from '#lib/components/Plate.svelte';
	import PlateHeading from '#lib/components/PlateHeading.svelte';

	// Everything from the README's feature list that the grinder and the teams plate don't
	// already cover, each said once.
	const groups = [
		{
			name: 'Models and providers',
			color: 'var(--color-duck)',
			items: [
				['Local servers', 'Connect to llama.cpp, Ollama, vLLM, SGLang, LM Studio, or Docker Model Runner; `/setup` detects running local servers automatically.'],
				['Hosted services', 'Includes 36 hosted provider presets (35 API-key services plus ChatGPT plan — 42 bundled provider files total), or any OpenAI-compatible URL.'],
				['Multi-endpoint failover', 'List `base_urls = [...]` to round-robin across internal GPU nodes with automatic health-cooldown failover, every host validated against policy.'],
				['Frontier & plan access', 'Use Anthropic, OpenAI, Google Gemini (preserving `thought_signature`), xAI Grok or Cohere with an API key, or sign in to a ChatGPT plan right inside `/setup`.'],
				['WebSockets & cache tuning', 'Persistent Responses WebSockets with keepalive, native compaction, `cache_control` breakpoints, sticky routing keys, Ollama `keep_alive`, and shared `--cache-key` for CI.'],
				['Reasoning & thinking', 'Thinking blocks survive tool calls and `/resume`, formatted as each API expects; switch effort levels any time with `/thinking`.'],
				['Switch & probe', 'Switch models mid-session with `/models` without losing history or approvals, and run `/probe` to check native tool calling before relying on it.'],
				['Dedicated planner model', 'Use a larger model in Plan mode, then hand execution off to your everyday model while keeping its prompt cache warm.']
			]
		},
		{
			name: 'Editing code',
			color: 'var(--color-beak)',
			items: [
				['Five edit formats', 'Supports `hashline`, `str_replace` (with batched `edits` and `symbol` scoping), `apply_patch`, search/replace blocks, or whole-file writes, selected per model.'],
				['Forgiving edit matching', 'Handles stray whitespace, copied line numbers and smart quotes when matching edits, while preserving CRLF line endings and BOMs.'],
				['Atomic multi-file edits', 'If a multi-file patch fails partway through, every modified file rolls back cleanly.'],
				['Undo, rewind & shell rollback', 'Roll back file edits and mutating shell commands (via a private shadow git repo) with `/undo`, or rewind conversation history, files, or both with `/rewind`.'],
				['Code intelligence', 'Integrates with language servers across 11 languages, tracks Cargo workspaces and shell file changes, and offers `lsp` for definitions, references, hover, project-wide renames, and `code_action` quickfixes.'],
				['Symbol outlines', 'Use `index` (cached per file snapshot) to summarize a file or folder with tree-sitter, or `read` to inspect a specific symbol by name.']
			]
		},
		{
			name: 'Long tasks',
			color: 'var(--color-sky)',
			items: [
				['Live todo list', 'The model maintains a working checklist that stays visible across steps.'],
				['Sub-agents', 'Built-in `explore` and `general` sub-agents scout or implement in isolated contexts and return concise reports, with optional per-agent `reasoning_effort`.'],
				['Managed background jobs', 'Run dev servers and watchers via `shell` (`background: true`), blocking `wait_secs`, delta polls, and automatic process-tree cleanup.'],
				['Git worktree forks', 'Isolate parallel branches in `.quack/worktrees/` with `/fork --worktree` or `quack --worktree <name>` without touching your main checkout.'],
				['Fresh-context plan handoff', 'Choose “Implement plan (fresh context)” or `/plan accept fresh` to condense planning exploration into one brief before editing.'],
				['Loop guards & step pauses', 'Detects repeated calls, edit-test-edit thrashing, and empty replies; turns pause at `agent.max_steps` so you can steer or continue.'],
				['Changed-file reminders', 'Keeps track of files touched in the session without rewriting earlier turns.'],
				['Precision @ slices', 'Attach specific `@file#symbol` definitions or `@file:Lstart-Lend` line ranges via tree-sitter instead of loading whole files.'],
				['KV zoning & small windows', 'Inspect the immutable KV prefix separately from the prunable tail in `/context` (`Trim` in `quack-gui`). Compaction keeps your original request verbatim.'],
				['Crash-proof sessions & retention', 'Sessions save incrementally and resume with identical prompt context; prune old runs with `quack sessions prune` or `[sessions] retention_days`.']
			]
		},
		{
			name: 'Safety',
			color: 'var(--color-leaf)',
			items: [
				['Sandboxed shell', 'Shell commands write only inside the workspace: Landlock + seccomp with no network on Linux, Seatbelt with no network on macOS, and Restricted Token + Low Integrity + Job Objects on Windows.'],
				['Plan or Edit', 'Plan mode only reads and searches, then proposes edits and test commands for review. Edit mode can modify files and run approved commands.'],
				['Ask or Allow', 'Approval rules are separate from mode: Allow skips prompts while keeping your sandbox and deny rules active, while `/yolo` disables both.'],
				['Parsed shell commands', 'Tree-sitter parses Bash and PowerShell ASTs so destructive commands like `rm` in `git status && rm -rf x` still require approval inside subshells, heredocs, and aliases.'],
				['Opt-in web fetch', '`web_fetch` is off by default and unregistered while offline; when enabled it blocks loopback, private, and metadata IPs with DNS pinning and validates every redirect.'],
				['Pre-Egress Secret Shield', 'Scans outbound prompts, `@` slices and tool output locally with built-in detectors and active session keys (`ask`, `redact`, or `off`) before sending over HTTP.'],
				['Gated redirects & ledger', 'Cross-host HTTP redirects must pass your egress policy, and every outbound call or block is recorded in the Session Egress Ledger.'],
				['Enterprise audit log', 'Lockable `[audit]` appends mode-`0600` JSONL records of every tool decision (`include_content = false` by default), checked by `quack doctor`.'],
				['Project trust & scope', 'A cloned repository’s `.quack/` folder never loads until you trust it; press `Ctrl-L` or run `/scope` to choose user vs `.quack/` when saving settings or rules.']
			]
		},
		{
			name: 'Making it yours',
			color: 'var(--color-pink)',
			items: [
				['MCP & OAuth 2.1', 'Connect stdio and streamable HTTP MCP servers via `quack mcp` or `/mcp` — with OAuth 2.1 + PKCE sign-in (`quack mcp login`, RFC 9728/8414/7591 discovery), per-server `defer`, spill files, live progress, prompts, resources, forms, and `sampling_model = "small"`.'],
				['Scoped live hot-reload', 'Edits to skills, commands, hooks, profiles, agents, themes, grammars, and individual `mcp.d`/`lsp.d` files reload automatically without dropping session approvals.'],
				['Hooks', 'Run custom programs in any language on seven lifecycle events; `/hooks` provides guided setup and live checks for bundled `forge-review` (GitHub, GitLab, Gitea/Forgejo, Bitbucket) and `jira` hooks with masked credentials.'],
				['Agent skills', 'Loads standard `SKILL.md` Agent Skills from `.agents/skills/`, `~/.agents/skills/` and `skills/` (plus `compat_paths`), triggered on demand with `skill` or `/<name>`.'],
				['Slash commands', 'Turn any Markdown file in `commands/` into a slash command (for example, `commands/git/pr.md` becomes `/git:pr`).'],
				['Project instruction files', 'Reads `AGENTS.md`, `CLAUDE.md`, `GEMINI.md`, `.cursorrules`, and Copilot instruction files automatically.'],
				['Shared memory notes', 'Keep project and global Markdown notes with configurable write approvals (never modified in Plan mode).'],
				['Custom tree-sitter queries', 'Drop in `tags.scm` or `highlights.scm` files to customize symbol outlining and syntax highlighting for a framework.']
			]
		},
		{
			name: 'Everywhere',
			color: 'var(--color-duck)',
			items: [
				['Cross-platform', 'Runs natively on Linux, macOS and Windows across x64 and ARM64, with `.deb`/`.rpm`, `.msi`, `.dmg`, `-setup.exe`, CycloneDX SBOMs, and keyless SLSA build attestations.'],
				['Editor integration (ACP)', '`quack acp` connects to Zed, JetBrains, Neovim, and VS Code over stdio with streaming replies, diffs, sub-agent tool calls, slash commands, inline images, `session/list`, and opt-in editor `fs/*` & `terminal/*`.'],
				['Fast startup', 'Draws the first terminal frame in under 10 ms on Linux.'],
				['Headless & GUI handoff', 'Run one-off turns in scripts with `quack exec "…"`, open repositories directly via `quack gui [PATH...]`, or jump from TUI to GUI using `/gui`.'],
				['Built-in diagnostics', 'Inspect token usage, cache hits, cost, and speed in `/context` and `/stats`, and verify providers, policy locks, audit log, or runtime health with `/doctor` and `/diagnostics`.'],
				['SSH & wide terminals', 'Mouse selection and `/copy` work over SSH via OSC 52, while terminals `140×24` and wider show a live sidebar with Git branch, context fill, speed, plan steps, and loaded MCP/LSP servers.'],
				['Desktop app', '`quack-gui` wraps the same engine in Tauri with a `Cmd/Ctrl-K` command palette, restored workspace tabs, parallel jobs, and an egress ledger.']
			]
		}
	];

	// `code` spans in the item text.
	const parts = (t: string) => t.split(/(`[^`]+`)/).filter(Boolean);
</script>

<Plate id="field-marks" color="var(--color-leaf)">
		<PlateHeading title="Field marks">
			A quick overview of everything else quack supports today. For the complete list, see the
			<a href="https://github.com/ClackStack/quack#features" class="text-sky-ink font-semibold" rel="noreferrer">README</a>.
		</PlateHeading>

		<section aria-labelledby="work-modes" class="mb-14">
			<h3 id="work-modes" class="font-display text-2xl sm:text-3xl font-extrabold">Plan or edit. Ask or allow.</h3>
			<p class="mt-3 text-ink-2 leading-relaxed max-w-[65ch]">
				Decide what the agent is allowed to do separately from when it must ask for approval. New sessions
				start in <strong class="text-ink">Plan + Allow</strong>: the agent can investigate without prompting
				for each read, then show you the proposed edits and tests before changing files. Set either default
				in Settings or <code>config.toml</code>.
			</p>
			<div class="mt-5 rounded-xl border border-rule overflow-x-auto">
				<table class="w-full text-left text-sm sm:text-base">
					<caption class="sr-only">Work mode and approval combinations</caption>
					<thead class="bg-sky-soft">
						<tr>
							<th scope="col" class="px-3 sm:px-5 py-3">Work mode</th>
							<th scope="col" class="px-3 sm:px-5 py-3">Ask</th>
							<th scope="col" class="px-3 sm:px-5 py-3">Allow</th>
						</tr>
					</thead>
					<tbody class="text-ink-2">
						<tr class="border-t border-rule">
							<th scope="row" class="px-3 sm:px-5 py-4 text-ink align-top">Plan</th>
							<td class="px-3 sm:px-5 py-4 align-top">Read and search; unapproved tools ask</td>
							<td class="px-3 sm:px-5 py-4 align-top">Read and search automatically</td>
						</tr>
						<tr class="border-t border-rule">
							<th scope="row" class="px-3 sm:px-5 py-4 text-ink align-top">Edit</th>
							<td class="px-3 sm:px-5 py-4 align-top">Read and search; unapproved edits and commands ask</td>
							<td class="px-3 sm:px-5 py-4 align-top">Read, search, edit and run commands automatically</td>
						</tr>
					</tbody>
				</table>
			</div>
			<p class="mt-4 text-ink-2 leading-relaxed max-w-[75ch]">
				In Plan mode, quack never runs tests, builds, or scripts, and never changes files or memory. Allow
				skips routine confirmation prompts while keeping explicit ask rules, deny rules, and sandbox boundaries
				active. Press <code class="text-ink">Shift-Tab</code> to toggle Plan/Edit, <code class="text-ink">Ctrl-Y</code>
				or <code class="text-ink">/permissions</code> to switch Ask/Allow (or click the status-bar pills).
				Approving a plan carries straight into implementation — or pick <code class="text-ink">/plan accept fresh</code>
				to condense planning exploration into one brief first.
			</p>
			<p class="mt-3 text-sm text-ink-2 leading-relaxed max-w-[75ch]">
				Legacy Auto-edit and Read-only settings remain supported for compatibility.
				<a href="https://github.com/ClackStack/quack/blob/main/docs/modes.md" class="text-sky-ink font-semibold" rel="noreferrer">Commands, configuration, and migration guide</a>.
			</p>
		</section>

		<div class="columns-1 md:columns-2 xl:columns-3 gap-x-12">
			{#each groups as g (g.name)}
				<div class="break-inside-avoid mb-12">
					<h3 class="flex items-center gap-2.5 text-2xl font-extrabold mb-4">
						<span class="w-6 h-2.5 rounded-full" style="background:{g.color}" aria-hidden="true"></span>
						{g.name}
					</h3>
					<dl class="space-y-3">
						{#each g.items as [term, text] (term)}
							<div>
								<dt class="inline font-bold">{term}.</dt>
								<dd class="inline text-ink-2">
									{#each parts(text) as p, i (i)}{#if p.startsWith('`')}<code class="text-[0.9em] text-ink bg-paper px-1 rounded border border-rule">{p.slice(1, -1)}</code>{:else}{p}{/if}{/each}
								</dd>
							</div>
						{/each}
					</dl>
				</div>
			{/each}
		</div>
	</Plate>
