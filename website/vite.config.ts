import { readFileSync } from 'node:fs';
import tailwindcss from '@tailwindcss/vite';
import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig, type Plugin } from 'vite';

// The installers are served beside the site, so `curl -fsSL https://quack.clackstack.dev/install.sh | sh`
// works. scripts/ stays the single source; the release workflow publishes the same files.
const installScripts = (): Plugin => ({
	name: 'install-scripts',
	buildStart() {
		if (this.environment?.config.build.ssr) return;
		for (const file of ['install.sh', 'install.ps1']) {
			this.emitFile({
				type: 'asset',
				fileName: file,
				source: readFileSync(new URL(`../scripts/${file}`, import.meta.url))
			});
		}
	}
});

export default defineConfig({
	plugins: [
		tailwindcss(),
		installScripts(),
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			adapter: adapter()
		})
	]
});
