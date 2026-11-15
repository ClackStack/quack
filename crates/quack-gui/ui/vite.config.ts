import adapter from '@sveltejs/adapter-static';
import { sveltekit } from '@sveltejs/kit/vite';
import { defineConfig } from 'vitest/config';

// The desktop SPA (docs/plan/15-desktop-client.md): built as a static bundle that the
// Tauri shell serves from `build/`, and runnable alone with `npm run dev` against the
// scripted dev transport. The same stack as `website/`, minus Tailwind: the UI's
// design tokens are hand-rolled CSS in `src/app.css`.
export default defineConfig({
	plugins: [
		sveltekit({
			compilerOptions: {
				// Force runes mode for the project, except for libraries. Can be removed in svelte 6.
				runes: ({ filename }) =>
					filename.split(/[/\\]/).includes('node_modules') ? undefined : true
			},
			adapter: adapter({
				fallback: 'index.html',
				pages: 'build',
				assets: 'build'
			})
		})
	],
	test: {
		environment: 'node',
		include: ['src/**/*.test.ts']
	}
});
