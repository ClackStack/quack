<script lang="ts">
	import { Cog, TriangleAlert, Check } from '@lucide/svelte';
	import Plate from '#lib/components/Plate.svelte';
	import PlateHeading from '#lib/components/PlateHeading.svelte';

	const modes = [
		{
			id: 'missed_tool_call',
			label: 'Tool call printed as text',
			short: 'tool calls',
			symptom: 'The model writes a tool call as Hermes or Qwen3-XML markup inside its reply instead of invoking a real call.',
			generic: 'The harness shows the raw markup as prose. Nothing runs, and the turn stalls or loops.',
			quack: 'Tool calls emitted as text (Hermes, Qwen3-XML, GLM, DeepSeek or Kimi formats) are converted into real tool calls and stripped from the reply. Malformed arguments are repaired and duplicate call IDs are fixed.',
			before: `<tool_call>\n{"name": "read", "arguments": {"path": "src/main.rs"}}\n</tool_call>`,
			after: `✓ read src/main.rs · 212 lines`
		},
		{
			id: 'cache_miss',
			label: 'Cold prompt cache',
			short: 'cache',
			symptom: 'A timestamp, working folder, or reordered tool list sits in the system prompt and changes on every turn or session.',
			generic: 'The server cannot reuse its KV cache, so every step re-reads the entire prompt: slower on your GPU and costlier on an API.',
			quack: 'The system prompt and tool definitions stay byte-identical across turns and projects. Per-session context opens the first user message, @file#symbol and @file:Lstart-Lend attach bounded slices to the turn, and /context lets you trim stale tail items without breaking the prompt cache. Automated tests check prefix stability for every profile.',
			before: `{"role": "system", "content": "… Today is 2026-10-24 15:04, cwd ~/src/app …"}`,
			after: `{"role": "system", "content": "…"}   // immutable KV prefix\n{"role": "user", "content": "<context>cwd, date, AGENTS.md</context> … @src/auth.rs#verify"}`
		},
		{
			id: 'invalid_parameter',
			label: 'Broken JSON arguments',
			short: 'JSON',
			symptom: 'A smaller model leaves a string unclosed, adds a trailing comma, or drops a required field.',
			generic: 'The call fails to parse and the turn errors out, or the model keeps repeating the same mistake.',
			quack: 'Malformed JSON arguments are repaired automatically when possible. On local engines (Ollama, vLLM, SGLang, LM Studio), grammar-guided decoding can constrain sampling directly to the tool schema.',
			before: `{"path": "src/lib.rs", "content": "fn main() {…`,
			after: `tool_calling.mode = "grammar"   # schema enforced by the server`
		},
		{
			id: 'syntax_break',
			label: 'Edit that breaks the file',
			short: 'syntax',
			symptom: 'An edit drops a closing brace or introduces a type error.',
			generic: 'The broken file is written to disk, and the model finds out three steps later when tests fail.',
			quack: 'Tree-sitter checks syntax right after an edit and reports the broken line immediately. Installed language servers also report any new type or compiler errors across the workspace in the same tool result.',
			before: `fn compute() {\n    return x;\n// missing '}'`,
			after: `✓ edit src/calc.rs · +3 -1\n  syntax error at line 3: unclosed delimiter\n  rust-analyzer: 1 new error in src/main.rs:14`
		},
		{
			id: 'context_overflow',
			label: 'Test output floods the window',
			short: 'context',
			symptom: 'A test run dumps 1,500 lines of passing output into a 16k or 32k context window.',
			generic: 'The context window fills up, the harness compacts early, and the model loses track of what it was doing.',
			quack: 'Output from cargo test, pytest, unittest, go test, jest and compiler builds is folded automatically: passing lines collapse, while every failure and stack trace stays visible. Old tool outputs are pruned before compaction is needed.',
			before: `test parse::a ... ok\ntest parse::b ... ok\n… 1,400 more lines`,
			after: `[... 42 passed tests omitted ...]\ntest auth::login ... FAILED`
		},
		{
			id: 'silent_egress',
			label: 'Calls you never configured',
			short: 'egress',
			symptom: 'The agent sends analytics or update checks to an unconfigured host, or follows an HTTP redirect off-host.',
			generic: 'Prompts, filenames, and code snippets can leave your network without warning.',
			quack: 'quack contacts only the providers and MCP servers you configure. HTTP redirects stay on the starting host unless your egress policy permits the hop, and the Session Egress Ledger logs every outbound request and block. Proxy tests and loopback-only CI runs enforce this guarantee.',
			before: `POST https://telemetry.example.com/v1/events`,
			after: `connect() → only the base_url in your provider file  (redirects gated · ledger logged)`
		},
		{
			id: 'workspace_upload',
			label: 'Whole workspace uploaded',
			short: 'uploads',
			symptom: 'To build a code index or embeddings, the agent uploads your whole folder remotely — including .env files, keys, and private code.',
			generic: 'Your entire repository ends up on a server you never chose.',
			quack: 'No hidden indexing, embeddings, or background workspace sync. The model sees only top-level filenames, your AGENTS.md, and the files or @file#symbol slices explicitly opened during the session.',
			before: `PUT https://index.example.com/v1/repos/acme-internal   (4,812 files)`,
			after: `✓ read src/auth.rs · 140 lines   // the only file the model saw`
		},
		{
			id: 'secret_leak',
			label: 'Secret in a prompt or .env',
			short: 'secrets',
			symptom: 'You paste a token or .env snippet, or a tool reads one back, and the next turn ships it raw to the model host.',
			generic: 'Most harnesses stream prompts and tool output straight over HTTPS as soon as you hit Send.',
			quack: 'Pre-Egress Secret Shield scans outbound prompts, @-mentions and tool output locally — combining built-in detectors with active session credentials — and pauses before sending so you can redact and send, allowlist a SHA-256 fingerprint, or deny the request.',
			before: `POST https://api.provider.example/v1/chat\n{"content": "GITHUB_TOKEN=ghp_…"}`,
			after: `⚑ Pre-Egress Secret Shield · Redact & send / Allowlist\n// scanned locally before HTTP send; headless auto-redacts`
		}
	];

	const models = ['Qwen', 'DeepSeek', 'GLM', 'Kimi', 'MiniMax', 'gpt-oss', 'Mistral'];
	const profiles = ['qwen-code', 'dsh', 'zcode', 'kimi-code', 'cc-style', 'codex', 'vibe'];

	let selected = $state(modes[0]);

	$effect(() => {
		const pick = () => {
			const id = location.hash.replace('#grinder-', '');
			const m = modes.find((x) => x.id === id);
			if (m) selected = m;
		};
		pick();
		window.addEventListener('hashchange', pick);
		return () => window.removeEventListener('hashchange', pick);
	});
</script>

<Plate id="grinder" color="var(--color-beak)">
		<PlateHeading title="Same model, two harnesses">
			Open-weight models are good at coding, yet most harnesses trip them up anyway: tool calls go missing,
			prompt caches reset, and broken edits slip through. Pick any issue below to compare what happens in a
			generic harness versus quack.
		</PlateHeading>

		<div class="grid grid-cols-1 lg:grid-cols-2 gap-6">
			<!-- The generic grinder -->
			<div class="rounded-2xl border-2 border-ink/80 bg-paper p-6 sm:p-8">
				<div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
					<h3 class="text-2xl font-extrabold">The generic grinder</h3>
					<span class="text-sm font-semibold text-pink-ink">one harness for every model</span>
				</div>
				<div class="mt-6 flex flex-wrap gap-2">
					{#each models as m}
						<span class="px-2.5 py-1 rounded-md bg-wash border border-rule font-mono text-sm">{m}</span>
					{/each}
				</div>
				<div class="jitter mt-6 rounded-xl border-2 border-dashed border-pink-ink/60 bg-pink-soft py-6 flex items-center justify-center gap-6">
					<Cog class="spin-fast w-14 h-14 text-pink-ink" aria-hidden="true" />
					<span class="font-display font-bold text-pink-ink text-center leading-tight">same prompt,<br />same tools,<br />for everyone</span>
					<Cog class="spin-fast-rev w-14 h-14 text-pink-ink" aria-hidden="true" />
				</div>
				<p class="mt-6 mb-3 text-sm font-semibold text-ink-2">What goes wrong · pick one</p>
				<div class="flex flex-wrap gap-2" role="tablist" aria-label="Failure modes">
					{#each modes as m (m.id)}
						<button
							type="button"
							role="tab"
							aria-selected={selected.id === m.id}
							aria-controls="grinder-detail"
							onclick={() => (selected = m)}
							class="cursor-pointer flex items-center gap-1.5 px-3 py-1.5 rounded-full border-2 text-sm font-semibold transition-colors {selected.id === m.id
								? 'bg-pink-ink border-pink-ink text-paper'
								: 'border-pink-ink/40 text-pink-ink hover:border-pink-ink'}"
						>
							<TriangleAlert class="w-4 h-4" aria-hidden="true" />
							{m.label}
						</button>
					{/each}
				</div>
			</div>

			<!-- quack -->
			<div class="rounded-2xl border-2 border-ink bg-paper p-6 sm:p-8 shadow-[0_10px_30px_-12px_rgba(29,42,58,0.3)]">
				<div class="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
					<h3 class="text-2xl font-extrabold">
						<span class="font-mono"><span class="text-duck-ink">_(o)</span><span class="text-beak-ink">&gt;</span></span> quack
					</h3>
					<span class="text-sm font-semibold text-leaf-ink">a profile per model family</span>
				</div>
				<div class="mt-6 flex flex-wrap gap-2">
					{#each profiles as p}
						<span class="px-2.5 py-1 rounded-md bg-duck-soft border border-duck-line font-mono text-sm">{p}</span>
					{/each}
				</div>
				<div class="mt-6 rounded-xl border-2 border-sky bg-sky-soft/60 py-6 flex items-center justify-center gap-6">
					<Cog class="spin-slow w-14 h-14 text-sky-ink" aria-hidden="true" />
					<span class="font-display font-bold text-sky-ink text-center leading-tight">tool names,<br />prompts and edit format<br />that suit the model</span>
					<Cog class="spin-slow-rev w-14 h-14 text-sky-ink" aria-hidden="true" />
				</div>
				<p class="mt-6 mb-3 text-sm font-semibold text-ink-2">How quack handles it · pick one</p>
				<div class="flex flex-wrap gap-2" role="tablist" aria-label="Handled modes">
					{#each modes as m (m.id)}
						<button
							type="button"
							role="tab"
							aria-selected={selected.id === m.id}
							aria-controls="grinder-detail"
							onclick={() => (selected = m)}
							class="cursor-pointer flex items-center gap-1.5 px-3 py-1.5 rounded-full border-2 text-sm font-semibold transition-colors {selected.id === m.id
								? 'bg-leaf-ink border-leaf-ink text-paper'
								: 'border-leaf-ink/30 text-leaf-ink hover:border-leaf-ink'}"
						>
							<Check class="w-4 h-4" aria-hidden="true" />
							{m.short} handled
						</button>
					{/each}
				</div>
			</div>
		</div>

		<!-- The account of the chosen failure -->
		<div id="grinder-detail" role="tabpanel" class="mt-6 rounded-2xl border-2 border-ink bg-paper p-6 sm:p-8">
			{#each modes as m}<span id="grinder-{m.id}" class="block -mt-24 pt-24" aria-hidden="true"></span>{/each}
			<h3 class="text-2xl sm:text-3xl font-extrabold">{selected.label}</h3>
			<p class="mt-2 text-lg text-ink-2 max-w-[70ch]">{selected.symptom}</p>
			<div class="mt-6 grid grid-cols-1 md:grid-cols-2 gap-6">
				<div>
					<p class="font-display font-bold text-pink-ink">In a generic harness</p>
					<p class="mt-2 leading-relaxed">{selected.generic}</p>
					<pre class="mt-4 rounded-lg bg-term text-[#ffb3c5] p-4 text-sm overflow-x-auto whitespace-pre-wrap">{selected.before}</pre>
				</div>
				<div>
					<p class="font-display font-bold text-leaf-ink">In quack</p>
					<p class="mt-2 leading-relaxed">{selected.quack}</p>
					<pre class="mt-4 rounded-lg bg-term text-[#a8e6b0] p-4 text-sm overflow-x-auto whitespace-pre-wrap">{selected.after}</pre>
				</div>
			</div>
		</div>
	</Plate>
