<script lang="ts">
	import { onMount } from 'svelte';
	import { AppWindow, Check, Copy, Download, Layers, Terminal } from '@lucide/svelte';

	const repo = 'https://github.com/ClackStack/quack/releases';
	const base = `${repo}/latest/download`;
	const site = 'https://quack.clackstack.dev';

	type Target = 'both' | 'tui' | 'gui';
	type Way = 'sh' | 'ps' | 'src';
	type Os = 'linux' | 'mac' | 'windows';
	type Arch = 'x86_64' | 'aarch64';

	const ways: { id: Way; label: string }[] = [
		{ id: 'sh', label: 'Linux & macOS' },
		{ id: 'ps', label: 'Windows' },
		{ id: 'src', label: 'From source' }
	];

	function command(way: Way, target: Target, plucked: boolean, detectedOs: Os | null): string {
		if (way === 'ps') {
			const flags = [
				target === 'tui' ? '-Tui' : target === 'gui' ? '-Gui' : '',
				plucked ? '-Plucked' : ''
			].filter(Boolean);
			if (flags.length === 0) return `irm ${site}/install.ps1 | iex`;
			return `& ([scriptblock]::Create((irm ${site}/install.ps1))) ${flags.join(' ')}`;
		}
		if (way === 'src') {
			if (detectedOs === 'windows') {
				const flags = [
					'-FromSource .',
					target === 'tui' ? '-Tui' : target === 'gui' ? '-Gui' : '',
					plucked ? '-Plucked' : ''
				].filter(Boolean);
				return `.\\scripts\\install.ps1 ${flags.join(' ')}`;
			}
			const flags = [
				'--from-source',
				target === 'tui' ? '--tui' : target === 'gui' ? '--gui' : '',
				plucked ? '--plucked' : ''
			].filter(Boolean);
			return `scripts/install.sh ${flags.join(' ')}`;
		}
		const flags = [
			target === 'tui' ? '--tui' : target === 'gui' ? '--gui' : '',
			plucked ? '--plucked' : ''
		].filter(Boolean);
		if (flags.length === 0) return `curl -fsSL ${site}/install.sh | sh`;
		return `curl -fsSL ${site}/install.sh | sh -s -- ${flags.join(' ')}`;
	}

	const tuiPlatforms: Record<Os, { name: string; triple: string; ext: string; arch: Record<Arch, string> }> = {
		linux: { name: 'Linux', triple: 'unknown-linux-musl', ext: 'tar.gz', arch: { x86_64: 'x64', aarch64: 'ARM64' } },
		mac: { name: 'macOS', triple: 'apple-darwin', ext: 'tar.gz', arch: { x86_64: 'Intel', aarch64: 'Apple silicon' } },
		windows: { name: 'Windows', triple: 'pc-windows-msvc', ext: 'zip', arch: { x86_64: 'x64', aarch64: 'ARM64' } }
	};

	const guiPlatforms: Record<Os, { name: string; triple: string; suffix: string; extLabel: string; arch: Record<Arch, string> }> = {
		linux: { name: 'Linux', triple: 'unknown-linux-gnu', suffix: '.AppImage', extLabel: 'AppImage', arch: { x86_64: 'x64', aarch64: 'ARM64' } },
		mac: { name: 'macOS', triple: 'apple-darwin', suffix: '.dmg', extLabel: 'dmg', arch: { x86_64: 'Intel', aarch64: 'Apple silicon' } },
		windows: { name: 'Windows', triple: 'pc-windows-msvc', suffix: '-setup.exe', extLabel: 'setup.exe', arch: { x86_64: 'x64', aarch64: 'ARM64' } }
	};

	let { size = 'lg' }: { size?: 'lg' | 'md' } = $props();
	let target = $state<Target>('both');
	let way = $state<Way>('sh');
	let plucked = $state(false);
	let os = $state<Os | null>(null);
	// null when the browser won't say; then both architectures are offered.
	let arch = $state<Arch | null>(null);
	let copied = $state(false);
	let timer: ReturnType<typeof setTimeout> | undefined;

	const cmd = $derived(command(way, target, plucked, os));

	const archives = $derived.by(() => {
		if (!os || way === 'src') return [];
		const arches: Arch[] = arch ? [arch] : ['aarch64', 'x86_64'];
		const items: { kind: string; file: string; href: string; label: string }[] = [];

		if (target === 'both' || target === 'gui') {
			const gp = guiPlatforms[os];
			for (const a of arches) {
				const file = `quack-gui-${a}-${gp.triple}${gp.suffix}`;
				items.push({
					kind: 'GUI',
					file,
					href: `${base}/${file}`,
					label: arch ? `GUI (.${gp.extLabel})` : `GUI ${gp.arch[a]}`
				});
			}
		}

		if (target === 'both' || target === 'tui') {
			const tp = tuiPlatforms[os];
			for (const a of arches) {
				const file = `quack-${a}-${tp.triple}.${tp.ext}`;
				items.push({
					kind: 'TUI',
					file,
					href: `${base}/${file}`,
					label: arch ? `TUI (.${tp.ext})` : `TUI ${tp.arch[a]}`
				});
			}
		}

		return items;
	});

	// Pick the visitor's platform: client hints where the browser has them, else the user
	// agent. Phones and anything unrecognised keep the default tab and get no archive link.
	onMount(() => {
		type Hints = { platform: string; getHighEntropyValues(h: string[]): Promise<{ architecture?: string }> };
		const hints = (navigator as Navigator & { userAgentData?: Hints }).userAgentData;
		const ua = navigator.userAgent;
		const name = hints?.platform || ua;
		if (/android|iphone|ipad|cros/i.test(name) || /android|iphone|ipad/i.test(ua)) return;
		const found: Os | null = /win/i.test(name) ? 'windows' : /mac/i.test(name) ? 'mac' : /linux/i.test(name) ? 'linux' : null;
		if (!found) return;
		os = found;
		way = found === 'windows' ? 'ps' : 'sh';
		// Windows on ARM still reports x64 in its user agent, and Macs always say Intel.
		if (found === 'linux') arch = /aarch64|arm64/i.test(ua) ? 'aarch64' : /x86_64|amd64/i.test(ua) ? 'x86_64' : null;
		hints
			?.getHighEntropyValues(['architecture'])
			.then((v) => {
				if (v.architecture === 'arm') arch = 'aarch64';
				else if (v.architecture === 'x86') arch = 'x86_64';
			})
			.catch(() => {});
	});

	async function copy() {
		try {
			await navigator.clipboard.writeText(cmd);
			copied = true;
			clearTimeout(timer);
			timer = setTimeout(() => (copied = false), 2000);
		} catch {
			copied = false;
		}
	}
</script>

<div class="@container w-full rounded-2xl border-2 border-ink bg-paper p-3.5 sm:p-4 shadow-[0_8px_22px_-12px_rgba(29,42,58,0.28)]">
	<!-- 1. What to install -->
	<div class="grid grid-cols-1 @md:grid-cols-3 gap-2 mb-3" role="group" aria-label="What to install">
		<button
			type="button"
			aria-pressed={target === 'both'}
			onclick={() => (target = 'both')}
			class="min-w-0 flex flex-col items-start gap-1 rounded-xl border-2 px-3 py-2 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink {target === 'both'
				? 'border-ink bg-ink text-paper'
				: 'border-rule bg-wash text-ink hover:border-ink/45'}"
		>
			<span class="flex w-full flex-wrap items-center justify-between gap-1.5">
				<span class="inline-flex items-center gap-1.5 font-display font-bold text-sm">
					<Layers class="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
					Both
				</span>
				<span
					class="rounded-full px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wide {target === 'both'
						? 'bg-duck text-on-tint'
						: 'bg-beak text-on-tint'}"
				>
					Default
				</span>
			</span>
			<span class="text-xs leading-tight {target === 'both' ? 'text-paper/80' : 'text-ink-2'}">
				{plucked ? 'GUI + TUI · plucked defaults' : 'GUI + TUI together'}
			</span>
		</button>

		<button
			type="button"
			aria-pressed={target === 'tui'}
			onclick={() => (target = 'tui')}
			class="min-w-0 flex flex-col items-start gap-1 rounded-xl border-2 px-3 py-2 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink {target === 'tui'
				? 'border-ink bg-ink text-paper'
				: 'border-rule bg-wash text-ink hover:border-ink/45'}"
		>
			<span class="inline-flex items-center gap-1.5 font-display font-bold text-sm">
				<Terminal class="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
				TUI only
			</span>
			<span class="font-mono text-[11px] leading-tight {target === 'tui' ? 'text-paper/80' : 'text-ink-2'}">
				{plucked ? 'quack · plucked defaults' : 'quack'}
			</span>
		</button>

		<button
			type="button"
			aria-pressed={target === 'gui'}
			onclick={() => (target = 'gui')}
			class="min-w-0 flex flex-col items-start gap-1 rounded-xl border-2 px-3 py-2 text-left transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink {target === 'gui'
				? 'border-ink bg-ink text-paper'
				: 'border-rule bg-wash text-ink hover:border-ink/45'}"
		>
			<span class="inline-flex items-center gap-1.5 font-display font-bold text-sm">
				<AppWindow class="w-3.5 h-3.5 shrink-0" aria-hidden="true" />
				GUI only
			</span>
			<span class="font-mono text-[11px] leading-tight {target === 'gui' ? 'text-paper/80' : 'text-ink-2'}">
				{plucked ? 'quack-gui · plucked defaults' : 'quack-gui'}
			</span>
		</button>
	</div>

	<!-- Context note: separate binaries, one shared core -->
	<p class="mb-3 text-xs sm:text-[13px] text-ink-2 leading-snug">
		{#if target === 'both'}
			Installs both <code class="font-semibold text-ink">quack-gui</code> (desktop) and
			<code class="font-semibold text-ink">quack</code> (terminal) — separate apps sharing one core,
			<code class="font-semibold text-ink">defaults/</code>, config, sessions, memory, and prompt
			history{plucked ? ' (with hosted providers omitted via --plucked)' : ''}.
		{:else if target === 'tui'}
			Installs only the <code class="font-semibold text-ink">quack</code> terminal binary (no desktop
			GUI){plucked ? ' with hosted providers omitted via --plucked' : ''}. They share one core and
			config, so pick <strong class="text-ink">Both</strong> anytime to add the GUI.
		{:else}
			Installs only the <code class="font-semibold text-ink">quack-gui</code> desktop app (no
			<code class="font-semibold text-ink">quack</code> CLI command){plucked
				? ' with hosted providers omitted via --plucked'
				: ''}. They share one core and config, so pick <strong class="text-ink">Both</strong> anytime to
			add the TUI.
		{/if}
	</p>

	<!-- 2. Method tabs + Plucked toggle -->
	<div class="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 mb-2.5 pt-2.5 border-t border-rule">
		<div class="flex flex-wrap gap-1" role="group" aria-label="Install method">
			{#each ways as w (w.id)}
				<button
					type="button"
					aria-pressed={way === w.id}
					onclick={() => (way = w.id)}
					class="px-2 min-[360px]:px-2.5 sm:px-3 py-1 rounded-full text-[13px] min-[360px]:text-sm font-semibold transition-colors focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink {way === w.id
						? 'bg-ink text-paper'
						: 'text-ink-2 hover:bg-wash hover:text-ink'}"
				>
					{w.label}
				</button>
			{/each}
		</div>
		<button
			type="button"
			role="switch"
			aria-checked={plucked}
			onclick={() => (plucked = !plucked)}
			class="group flex items-center gap-2 text-sm font-semibold text-ink focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-ink rounded-full"
		>
			<span
				class="relative w-9 h-5 rounded-full border-2 border-ink transition-colors {plucked ? 'bg-beak' : 'bg-paper'}"
				aria-hidden="true"
			>
				<span
					class="absolute top-0.5 left-0.5 w-3 h-3 rounded-full bg-ink transition-transform {plucked ? 'translate-x-4' : ''}"
				></span>
			</span>
			Plucked: no hosted services
		</button>
	</div>

	<!-- 3. Command bar -->
	<div
		class="flex items-stretch rounded-xl border-2 border-ink bg-duck-soft overflow-hidden shadow-[0_6px_18px_-8px_rgba(29,42,58,0.35)]"
	>
		<code
			class="flex-1 min-w-0 whitespace-pre-wrap [overflow-wrap:break-word] leading-relaxed px-4 {size === 'lg'
				? 'py-3 text-[14px] sm:text-[15px]'
				: 'py-3 text-sm'} font-semibold text-ink"
			><span class="select-none text-beak-ink">{way === 'ps' ? '>' : '$'}&nbsp;</span>{#each cmd.split(' ') as word, w (`${w}:${word}`)}{w ? ' ' : ''}{#if word.includes('/')}{#each word.split('/') as part, i (`${i}:${part}`)}{#if i}/<wbr />{/if}{part}{/each}{:else}<span class="whitespace-nowrap">{word}</span>{/if}{/each}</code
		>
		<button
			type="button"
			onclick={copy}
			class="shrink-0 flex items-center gap-1.5 px-3.5 sm:px-4 bg-duck border-l-2 border-ink font-semibold text-on-tint hover:brightness-95 active:translate-y-px transition-all focus-visible:outline-2 focus-visible:outline-offset-[-3px] focus-visible:outline-ink"
		>
			{#if copied}
				<Check class="w-4 h-4" aria-hidden="true" /> <span class="max-sm:sr-only">Copied</span>
			{:else}
				<Copy class="w-4 h-4" aria-hidden="true" /> <span class="max-sm:sr-only">Copy</span>
			{/if}
		</button>
	</div>

	<!-- 4. Direct downloads -->
	{#if archives.length}
		<p class="mt-2.5 flex flex-wrap items-center gap-x-3.5 gap-y-1 text-sm text-ink-2">
			<span>Direct downloads{plucked ? ' (same binaries; installer applies --plucked)' : ''}:</span>
			{#each archives as a (a.file)}
				<a href={a.href} class="inline-flex items-center gap-1 font-semibold text-sky-ink hover:underline underline-offset-2" title={a.file}>
					<Download class="w-3.5 h-3.5" aria-hidden="true" />{a.label}
				</a>
			{/each}
			<a href={repo} class="font-semibold text-sky-ink hover:underline underline-offset-2" rel="noreferrer">
				.deb, .rpm &amp; all platforms
			</a>
		</p>
	{:else if way !== 'src'}
		<p class="mt-2.5 text-sm text-ink-2">
			Or download packages (<code class="text-ink">.deb</code>, <code class="text-ink">.rpm</code>,
			<code class="text-ink">.AppImage</code>, <code class="text-ink">.dmg</code>,
			<code class="text-ink">.exe</code>, <code class="text-ink">.tar.gz</code>) on
			<a href={repo} class="font-semibold text-sky-ink hover:underline underline-offset-2" rel="noreferrer">GitHub Releases</a>.
		</p>
	{/if}
	<p class="sr-only" aria-live="polite">{copied ? 'Install command copied' : ''}</p>
</div>
