<script lang="ts">
	import { DUCK_ANIM_STEPS, DUCK_FRAME_MS, duckFrame } from '#lib/duck';

	let {
		loop = true,
		reducedMotion = false,
		caption = ''
	}: {
		/** Continuously paddle while true; false takes one lap then rests. */
		loop?: boolean;
		/** Honor quack's `ui.reduced_motion` setting alongside OS prefers-reduced-motion. */
		reducedMotion?: boolean;
		caption?: string;
	} = $props();

	let step = $state(0);
	let osReduced = $state(false);

	const quiet = $derived(reducedMotion || osReduced);
	const frame = $derived(duckFrame(quiet ? 0 : step));

	$effect(() => {
		if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return;
		const mq = window.matchMedia('(prefers-reduced-motion: reduce)');
		osReduced = mq.matches;
		const onChange = (event: MediaQueryListEvent) => {
			osReduced = event.matches;
		};
		mq.addEventListener?.('change', onChange);
		return () => mq.removeEventListener?.('change', onChange);
	});

	$effect(() => {
		const shouldLoop = loop;
		if (quiet) {
			step = 0;
			return;
		}
		step = 0;
		let current = 0;
		const id = window.setInterval(() => {
			if (typeof document !== 'undefined' && document.hidden) return;
			current += 1;
			if (current >= DUCK_ANIM_STEPS) {
				if (shouldLoop) {
					current = 0;
					step = 0;
				} else {
					step = 0;
					window.clearInterval(id);
				}
				return;
			}
			step = current;
		}, DUCK_FRAME_MS);
		return () => window.clearInterval(id);
	});
</script>

<div class="welcome-duck">
	<pre class="duck-art" aria-hidden="true"><span class="duck-body">{frame.top}</span>
<span class="duck-body">{frame.headLead}</span><span class="duck-beak">{frame.beakTop}</span><span class="duck-body">{frame.headRest}</span>
<span class="duck-body">{frame.bodyLead}</span><span class="duck-beak">{frame.beakLower}</span><span class="duck-body">{frame.tail}</span>
<span class="duck-body">{frame.bottom}</span>
<span class="duck-water">{frame.water}</span></pre>
	{#if caption}
		<p class="duck-caption">{caption}</p>
	{/if}
</div>

<style>
	.welcome-duck {
		display: inline-grid;
		justify-items: center;
		gap: var(--sp-xs);
		font-family: var(--font-mono);
	}

	.duck-art {
		display: inline-block;
		text-align: left;
		line-height: 1.25;
		margin: 0;
		font-family: var(--font-mono);
		font-size: var(--text-mono);
		user-select: none;
	}

	.duck-body {
		color: var(--duck-ink);
		font-weight: 600;
	}

	.duck-beak {
		color: var(--beak);
		font-weight: 700;
	}

	.duck-water {
		color: var(--sky-ink);
	}

	:global(:root[data-theme='dark']) .duck-body {
		color: var(--duck);
	}

	:global(:root[data-theme='dark']) .duck-water {
		color: var(--sky);
	}

	.duck-caption {
		margin: var(--sp-xs) 0 0;
		color: var(--ink-3);
		font-family: var(--font-mono);
		font-size: var(--text-mono);
	}
</style>
