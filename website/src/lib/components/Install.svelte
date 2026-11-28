<script lang="ts">
	import Duck from '#lib/components/Duck.svelte';
	import InstallBox from '#lib/components/InstallBox.svelte';
	import Plate from '#lib/components/Plate.svelte';
	import PlateHeading from '#lib/components/PlateHeading.svelte';
</script>

<Plate id="install" color="var(--color-beak)">
		<PlateHeading title="Where to find it">
			The default installer sets up both the desktop app (<code class="text-ink">quack-gui</code>) and
			the terminal UI (<code class="text-ink">quack</code>) together — or pick TUI or GUI alone, with
			full defaults or <code class="text-ink">--plucked</code>.
		</PlateHeading>

		<InstallBox />

		<div class="mt-8 grid grid-cols-1 lg:grid-cols-[minmax(0,3fr)_minmax(0,2fr)] gap-12 items-center">
			<div class="max-w-2xl">
				<ul class="space-y-3 text-ink-2 leading-relaxed">
					<li><strong class="text-ink">Two apps, one core:</strong> installing the TUI does not install the GUI, and installing the GUI does not install the <code class="text-ink">quack</code> CLI command. Because they share one core, <code class="text-ink">defaults/</code> folder, config, sessions, memory notes, and prompt history, the default installer sets up both together — or you can add the other at any time.</li>
					<li><strong class="text-ink">TUI (<code class="text-ink">quack</code>):</strong> the terminal agent with profiles, sub-agents, MCP, ACP, and language servers. Passing <code class="text-ink">--plucked</code> (<code class="text-ink">-Plucked</code>) omits hosted-service providers and leaves only local servers and your own config.</li>
					<li><strong class="text-ink">Desktop GUI (<code class="text-ink">quack-gui</code>):</strong> the native desktop app for macOS, Windows, and glibc Linux, sharing the same on-disk <code class="text-ink">defaults/</code> directory alongside the TUI.</li>
					<li><strong class="text-ink">Options &amp; packages:</strong> pass <code class="text-ink whitespace-nowrap">--tui</code>, <code class="text-ink whitespace-nowrap">--gui</code>, <code class="text-ink whitespace-nowrap">--plucked</code>, <code class="text-ink whitespace-nowrap">--binary-only</code>, <code class="text-ink whitespace-nowrap">--prefix</code>, <code class="text-ink whitespace-nowrap">--version</code>, <code class="text-ink whitespace-nowrap">--archive FILE</code>, or <code class="text-ink whitespace-nowrap">--uninstall</code> (<code class="text-ink whitespace-nowrap">-Tui</code>, <code class="text-ink whitespace-nowrap">-Gui</code>, <code class="text-ink whitespace-nowrap">-Plucked</code> on Windows). <code class="text-ink">.deb</code>, <code class="text-ink">.rpm</code>, Windows <code class="text-ink">.msi</code>, <code class="text-ink">.AppImage</code>, <code class="text-ink">.dmg</code>, <code class="text-ink">-setup.exe</code>, CycloneDX SBOMs, and keyless GitHub SLSA build provenance attestations (<code class="text-ink">gh attestation verify</code>) are on the <a href="https://github.com/ClackStack/quack/releases" class="text-sky-ink font-semibold" rel="noreferrer">releases page</a>.</li>
				</ul>
				<p class="mt-8 text-sm text-ink-2">
					Actively developed and ready for daily use. Still on the roadmap: macOS notarization and Windows Authenticode code signing.
				</p>
			</div>
			<div class="flex justify-center">
				<Duck class="text-[clamp(1.5rem,3.4vw,2.6rem)]" />
			</div>
		</div>
	</Plate>
