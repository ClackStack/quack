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
				['Hosted services', 'Includes 35 hosted provider presets (OpenRouter, DeepSeek, Kimi, Z.ai, Mistral, Groq, Cerebras, Alibaba and more — 42 bundled provider files total), or any OpenAI-compatible URL.'],
				['Frontier & plan access', 'Use Anthropic, OpenAI, Google Gemini (preserving `thought_signature`), xAI Grok or Cohere with an API key, or sign in to a ChatGPT plan right inside `/setup`.'],
				['Reasoning & thinking', 'Thinking blocks survive tool calls and `/resume`, formatted as each API expects; switch effort levels any time with `/thinking`.'],
				['Switch & probe', 'Switch models mid-session with `/models` without losing history or approvals, and run `/probe` to check native tool calling before relying on it.'],
				['Dedicated planner model', 'Use a larger model in Plan mode, then hand execution off to your everyday model while keeping its prompt cache warm.']
			]
		},
		{
			name: 'Editing code',
			color: 'var(--color-beak)',
			items: [
				['Five edit formats', 'Supports `hashline`, `str_replace`, `apply_patch`, search/replace blocks, or whole-file writes, selected per model.'],
				['Forgiving edit matching', 'Handles stray whitespace, copied line numbers and smart quotes when matching edits, while preserving CRLF line endings and BOMs.'],
				['Atomic multi-file edits', 'If a multi-file patch fails partway through, every modified file rolls back cleanly.'],
				['Undo and rewind', 'Roll back the last call with `/undo`, or rewind conversation history, files, or both with `/rewind`.'],
				['Code intelligence', 'Integrates with language servers across 11 languages and provides an `lsp` tool for definitions, references, and hover info.'],
				['Symbol outlines', 'Use `index` to summarize a file or folder with tree-sitter, or `read` to inspect a specific symbol by name.']
			]
		},
		{
			name: 'Long tasks',
			color: 'var(--color-sky)',
			items: [
				['Live todo list', 'The model maintains a working checklist that stays visible across steps.'],
				['Sub-agents', 'Built-in `explore` sub-agents scout the repository in an isolated context and return relevant paths and line ranges.'],
				['Loop guards & step pauses', 'Detects repeated calls, edit-test-edit thrashing, and empty replies; turns pause at `agent.max_steps` so you can steer or continue.'],
				['Changed-file reminders', 'Keeps track of files touched in the session without rewriting earlier turns.'],
				['Precision @ slices', 'Attach specific `@file#symbol` definitions or `@file:Lstart-Lend` line ranges via tree-sitter instead of loading whole files.'],
				['KV zoning & small windows', 'Inspect the immutable KV prefix separately from the prunable tail in `/context` (`Trim` in `quack-gui`). Compaction keeps your original request verbatim.'],
				['Crash-proof sessions', 'Sessions save incrementally as you work and resume with identical prompt context.']
			]
		},
		{
			name: 'Safety',
			color: 'var(--color-leaf)',
			items: [
				['Sandboxed shell', 'Shell commands can write only inside the workspace and have no network access (Landlock + seccomp on Linux, Seatbelt on macOS).'],
				['Plan or Edit', 'Plan mode only reads and searches, then proposes edits and test commands for review. Edit mode can modify files and run approved commands.'],
				['Ask or Allow', 'Approval rules are separate from mode: Allow skips prompts while keeping your sandbox and deny rules active, while `/yolo` disables both.'],
				['Parsed shell commands', 'In Ask mode, destructive commands like `rm` in `git status && rm -rf x` still require approval, even inside subshells and heredocs.'],
				['Pre-Egress Secret Shield', 'Scans outbound prompts, `@` slices and tool output locally with built-in detectors and active session keys (`ask`, `redact`, or `off`) before sending over HTTP.'],
				['Gated redirects & ledger', 'Cross-host HTTP redirects must pass your egress policy, and every outbound call or block is recorded in the Session Egress Ledger.'],
				['Project trust & scope', 'A cloned repository’s `.quack/` folder never loads until you trust it; press `Ctrl-L` or run `/scope` to choose user vs `.quack/` when saving settings or rules.']
			]
		},
		{
			name: 'Making it yours',
			color: 'var(--color-pink)',
			items: [
				['MCP', 'Connect local and remote MCP servers via `quack mcp` or toggle them live in `/mcp` — with deferred tool search, prompts as slash commands, forms and sampling (8 bundled examples including `context7`, `roost`, `github`, `playwright`).'],
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
				['Cross-platform', 'Runs natively on Linux, macOS and Windows across x64 and ARM64, using PowerShell and Job Objects on Windows.'],
				['Fast startup', 'Draws the first terminal frame in under 10 ms on Linux.'],
				['Headless & GUI handoff', 'Run one-off turns in scripts with `quack exec "…"`, open repositories directly via `quack gui [PATH...]`, or jump from TUI to GUI using `/gui`.'],
				['Built-in diagnostics', 'Inspect token usage, cache hits, cost, and speed in `/context` and `/stats`, and verify providers or runtime health with `/doctor` and `/diagnostics`.'],
				['SSH & wide terminals', 'Mouse selection and `/copy` work over SSH via OSC 52, while terminals `140×24` and wider show a live sidebar with Git branch, context fill, and speed.'],
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
				start in <strong class="text-ink">Edit + Ask</strong>. Switch to
				<strong class="text-ink">Plan + Allow</strong> to investigate read-only and review proposed edits
				and test commands before any files change.
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
				active. Press <code class="text-ink">Shift-Tab</code> to toggle Plan/Edit, or run
				<code class="text-ink">/permissions</code> to switch between Ask and Allow. Approving a plan carries
				straight into implementation under those same approval settings.
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
