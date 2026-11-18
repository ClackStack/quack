<script lang="ts">
	import './layout.css';
	import { ModeWatcher } from 'mode-watcher';
	import type { LayoutProps } from './$types';

	let { children }: LayoutProps = $props();
</script>

<svelte:head>
	<link rel="icon" type="image/png" sizes="32x32" href="/favicon-32.png" />
	<link rel="apple-touch-icon" sizes="180x180" href="/apple-touch-icon.png" />
	<meta property="og:site_name" content="quack" />
	<meta property="og:image" content="https://quack.clackstack.dev/og.png" />
	<meta property="og:image:width" content="1200" />
	<meta property="og:image:height" content="630" />
	<meta property="og:image:alt" content="The quack duck and wordmark on a dark terminal background." />
	<meta property="og:locale" content="en_US" />
	<meta name="twitter:card" content="summary_large_image" />
	<script type="application/ld+json">
		{
			"@context": "https://schema.org",
			"@type": "Organization",
			"name": "quack",
			"url": "https://quack.clackstack.dev/",
			"logo": "https://quack.clackstack.dev/duck-icon.png",
			"sameAs": ["https://github.com/ClackStack/quack"]
		}
	</script>
</svelte:head>

<ModeWatcher themeColors={{ light: '#ffffff', dark: '#17212d' }} />
{@render children()}
