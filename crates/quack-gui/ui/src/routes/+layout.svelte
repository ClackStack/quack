<script lang="ts">
	import '../app.css';
	import type { Snippet } from 'svelte';
	import { theme } from '#lib/theme.svelte';
	import { textScale } from '#lib/textScale.svelte';

	let { children }: { children: Snippet } = $props();

	$effect(() => {
		theme.apply();
		textScale.apply();
	});
</script>

{@render children()}
