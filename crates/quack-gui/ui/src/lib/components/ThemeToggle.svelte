<script lang="ts">
	import { theme } from '#lib/theme.svelte';

	let { showLabels = false }: { showLabels?: boolean } = $props();
</script>

<div class="theme-toggle" role="radiogroup" aria-label="Color theme">
	<button
		type="button"
		class="toggle-btn"
		class:active={theme.preference === 'system'}
		onclick={() => theme.set('system')}
		title={`System theme (${theme.systemDark ? 'Dark' : 'Light'})`}
		aria-label={`System theme (${theme.systemDark ? 'Dark' : 'Light'})`}
		aria-checked={theme.preference === 'system'}
		role="radio"
	>
		<svg
			class="icon"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			stroke-width="2"
			stroke-linecap="round"
			stroke-linejoin="round"
			aria-hidden="true"
		>
			<rect x="2" y="3" width="20" height="14" rx="2" ry="2" />
			<line x1="8" y1="21" x2="16" y2="21" />
			<line x1="12" y1="17" x2="12" y2="21" />
		</svg>
		{#if showLabels}<span class="label">Auto</span>{/if}
	</button>

	<button
		type="button"
		class="toggle-btn"
		class:active={theme.preference === 'light'}
		onclick={() => theme.set('light')}
		title="Light theme"
		aria-label="Light theme"
		aria-checked={theme.preference === 'light'}
		role="radio"
	>
		<svg
			class="icon"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			stroke-width="2"
			stroke-linecap="round"
			stroke-linejoin="round"
			aria-hidden="true"
		>
			<circle cx="12" cy="12" r="4" />
			<path
				d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"
			/>
		</svg>
		{#if showLabels}<span class="label">Light</span>{/if}
	</button>

	<button
		type="button"
		class="toggle-btn"
		class:active={theme.preference === 'dark'}
		onclick={() => theme.set('dark')}
		title="Dark theme"
		aria-label="Dark theme"
		aria-checked={theme.preference === 'dark'}
		role="radio"
	>
		<svg
			class="icon"
			viewBox="0 0 24 24"
			fill="none"
			stroke="currentColor"
			stroke-width="2"
			stroke-linecap="round"
			stroke-linejoin="round"
			aria-hidden="true"
		>
			<path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
		</svg>
		{#if showLabels}<span class="label">Dark</span>{/if}
	</button>
</div>

<style>
	.theme-toggle {
		display: inline-flex;
		align-items: center;
		padding: 2px;
		gap: 2px;
		background: var(--bg-soft);
		border: 1px solid var(--rule);
		border-radius: var(--r-lg);
		user-select: none;
	}

	.toggle-btn {
		display: inline-flex;
		align-items: center;
		justify-content: center;
		gap: 5px;
		height: 24px;
		padding: 0 7px;
		border-radius: var(--r-md);
		border: 1px solid transparent;
		background: transparent;
		color: var(--ink-3);
		cursor: pointer;
		font-size: 11.5px;
		font-weight: 500;
		transition:
			color 120ms ease-out,
			background-color 120ms ease-out,
			border-color 120ms ease-out,
			box-shadow 120ms ease-out;
	}

	.toggle-btn:hover {
		color: var(--ink);
		background: var(--bg-raised);
	}

	.toggle-btn.active {
		color: var(--ink);
		background: var(--bg-panel);
		border-color: var(--rule-soft);
		box-shadow: var(--shadow-card);
		font-weight: 600;
	}

	.icon {
		width: 13px;
		height: 13px;
		flex-shrink: 0;
	}

	.label {
		line-height: 1;
	}

	@media (max-width: 480px) {
		.label {
			display: none;
		}
	}
</style>
