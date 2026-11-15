<script lang="ts">
	import Duck from '#lib/components/Duck.svelte';
	import InstallBox from '#lib/components/InstallBox.svelte';
	import Plate from '#lib/components/Plate.svelte';
	import PlateHeading from '#lib/components/PlateHeading.svelte';
</script>

<Plate id="install" color="var(--color-beak)">
		<PlateHeading title="Where to find it">
			The default installer sets up both the desktop app (<code class="text-ink">quack-gui</code>) and
			the terminal UI (<code class="text-ink">quack</code>) together — or pick TUI or GUI alone, standard or
			plucked.
		</PlateHeading>

		<div class="grid grid-cols-1 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)] gap-12 items-center">
			<div class="max-w-2xl">
				<InstallBox />
				<ul class="mt-8 space-y-3 text-ink-2 leading-relaxed">
					<li><strong class="text-ink">Two apps, one core:</strong> installing the TUI does not install the GUI, and installing the GUI does not install the <code class="text-ink">quack</code> CLI command. Because they share one core, config, sessions, memory notes, and prompt history, the default installer sets up both in harmony — or add the other anytime.</li>
					<li><strong class="text-ink">TUI &amp; Plucked (<code class="text-ink">quack</code> / <code class="text-ink">quack-plucked</code>):</strong> the terminal agent with profiles, sub-agents, MCP, ACP, and language servers. Plucked drops the hosted-service providers and leaves only local servers and your own config.</li>
					<li><strong class="text-ink">GUI &amp; Plucked (<code class="text-ink">quack-gui</code> / <code class="text-ink">quack-gui-plucked</code>):</strong> the native desktop app for macOS, Windows, and glibc Linux, shipped in both standard and plucked bundles alongside the TUI.</li>
					<li><strong class="text-ink">Options &amp; packages:</strong> pass <code class="text-ink whitespace-nowrap">--tui</code>, <code class="text-ink whitespace-nowrap">--gui</code>, <code class="text-ink whitespace-nowrap">--plucked</code>, <code class="text-ink whitespace-nowrap">--binary-only</code>, <code class="text-ink whitespace-nowrap">--prefix</code>, <code class="text-ink whitespace-nowrap">--version</code>, <code class="text-ink whitespace-nowrap">--archive FILE</code>, or <code class="text-ink whitespace-nowrap">--uninstall</code> (<code class="text-ink whitespace-nowrap">-Tui</code>, <code class="text-ink whitespace-nowrap">-Gui</code>, <code class="text-ink whitespace-nowrap">-Plucked</code> on Windows). <code class="text-ink">.deb</code>, <code class="text-ink">.rpm</code>, <code class="text-ink">.AppImage</code>, <code class="text-ink">.dmg</code>, and <code class="text-ink">-setup.exe</code> packages for both flavours are on the <a href="https://github.com/ClackStack/quack/releases" class="text-sky-ink font-semibold" rel="noreferrer">releases page</a>.</li>
				</ul>
				<p class="mt-8 text-sm text-ink-2">
					In development and usable day to day. Not there yet: sandboxing on Windows, <code class="whitespace-nowrap">/fork --worktree</code>, config auto-reload and MCP
					OAuth.
				</p>
			</div>
			<div class="flex justify-center">
				<Duck class="text-[clamp(1.5rem,3.4vw,2.6rem)]" />
			</div>
		</div>
	</Plate>
