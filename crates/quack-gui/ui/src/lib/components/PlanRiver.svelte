<script lang="ts">
	import { planRiverPosition } from '#lib/duck';

	let {
		done,
		total,
		percent,
		active = false,
		complete = false,
		compact = false
	}: {
		done: number;
		total: number;
		percent: number;
		/** Gently ripple wake and bob the duck while a step is actively running. */
		active?: boolean;
		complete?: boolean;
		/** Compact variant for dense job overview cards. */
		compact?: boolean;
	} = $props();

	const position = $derived(planRiverPosition(done, total));
	const paddling = $derived(active && !complete);
</script>

<div
	class={['plan-river', { compact, paddling, complete }]}
	style:--pos={position}
	role="progressbar"
	aria-label="Plan progress"
	aria-valuenow={done}
	aria-valuemin={0}
	aria-valuemax={total}
	aria-valuetext={`${done} of ${total} steps complete (${percent}%)`}
>
	<div class="channel" aria-hidden="true">
		<div class="water-ahead"></div>
		<div class="river-wake"></div>
		<span class="river-duck">
			<span class="duck-pose">
				<span class="duck-body">_(•)</span><span class="duck-beak">&gt;</span>
			</span>
		</span>
	</div>
</div>

<style>
	.plan-river {
		--duck-w: 3.1em;
		position: relative;
		height: 22px;
		padding: 0 6px;
		border: 1px solid rgba(127, 179, 201, 0.36);
		border-radius: 999px;
		background: rgba(8, 16, 24, 0.36);
		overflow: hidden;
		font-family: var(--font-mono);
		font-size: 0.71875rem;
		line-height: 1;
		user-select: none;
		transition: border-color 240ms ease-out;
	}

	.plan-river.compact {
		height: 18px;
		padding: 0 5px;
		font-size: 0.625rem;
	}

	.plan-river.complete {
		border-color: color-mix(in srgb, var(--cloth-ok) 58%, transparent);
	}

	.channel {
		position: relative;
		width: 100%;
		height: 100%;
	}

	/* Calm dotted pond water ahead of the duck, echoing the TUI's river. */
	.water-ahead {
		position: absolute;
		inset: 0;
		background-image: radial-gradient(circle, rgba(127, 179, 201, 0.42) 1px, transparent 1.25px);
		background-size: 7px 100%;
		background-position: left center;
		background-repeat: repeat-x;
	}

	.plan-river.compact .water-ahead {
		background-size: 6px 100%;
	}

	/* Rippling blue wake behind the duck; turns leaf green at 100% completion. */
	.river-wake {
		position: absolute;
		left: -6px;
		top: 0;
		bottom: 0;
		width: calc(6px + var(--pos, 0) * (100% - var(--duck-w)) + var(--duck-w) * 0.2);
		background-color: rgba(94, 139, 163, 0.24);
		background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='14' height='8' viewBox='0 0 14 8' fill='none'%3E%3Cpath d='M0 4.5 Q3.5 1.5 7 4.5 T14 4.5' stroke='%237fb3c9' stroke-opacity='0.82' stroke-width='1.35' stroke-linecap='round'/%3E%3C/svg%3E");
		background-size: 14px 8px;
		background-repeat: repeat-x;
		background-position: 0 64%;
		transition:
			width 420ms cubic-bezier(0.22, 1, 0.36, 1),
			background-color 300ms ease-out;
	}

	.plan-river.compact .river-wake {
		background-size: 12px 7px;
	}

	.plan-river.paddling .river-wake {
		animation: wake-flow 1.4s linear infinite;
	}

	.plan-river.complete .river-wake {
		background-color: rgba(169, 209, 134, 0.2);
		background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='14' height='8' viewBox='0 0 14 8' fill='none'%3E%3Cpath d='M0 4.5 Q3.5 1.5 7 4.5 T14 4.5' stroke='%23a9d186' stroke-opacity='0.85' stroke-width='1.35' stroke-linecap='round'/%3E%3C/svg%3E");
	}

	/* Duck glides strictly to the completed fraction, never on a timer. */
	.river-duck {
		position: absolute;
		top: 50%;
		left: calc(var(--pos, 0) * (100% - var(--duck-w)));
		width: var(--duck-w);
		transform: translateY(-50%);
		display: inline-flex;
		align-items: center;
		justify-content: center;
		white-space: nowrap;
		transition: left 420ms cubic-bezier(0.22, 1, 0.36, 1);
	}

	.duck-pose {
		display: inline-flex;
		align-items: center;
		transform-origin: center 70%;
	}

	.plan-river.paddling .duck-pose {
		animation: duck-bob 1.8s ease-in-out infinite;
	}

	.plan-river.complete .duck-pose {
		animation: duck-arrive 360ms cubic-bezier(0.22, 1, 0.36, 1) 1;
	}

	.duck-body {
		color: var(--duck);
		font-weight: 700;
	}

	.duck-beak {
		color: var(--beak);
		font-weight: 700;
	}

	@keyframes wake-flow {
		from {
			background-position-x: 0;
		}
		to {
			background-position-x: -14px;
		}
	}

	@keyframes duck-bob {
		0%,
		100% {
			transform: translateY(0) rotate(0deg);
		}
		50% {
			transform: translateY(-1.5px) rotate(-2deg);
		}
	}

	@keyframes duck-arrive {
		0% {
			transform: translateY(0) rotate(0deg);
		}
		45% {
			transform: translateY(-2px) rotate(3deg);
		}
		100% {
			transform: translateY(0) rotate(0deg);
		}
	}

	@media (prefers-reduced-motion: reduce) {
		.plan-river.paddling .river-wake,
		.plan-river.paddling .duck-pose,
		.plan-river.complete .duck-pose {
			animation: none;
		}
		.river-wake,
		.river-duck {
			transition: none;
		}
	}

	:global(:root[data-reduced-motion='true']) .plan-river.paddling .river-wake,
	:global(:root[data-reduced-motion='true']) .plan-river.paddling .duck-pose,
	:global(:root[data-reduced-motion='true']) .plan-river.complete .duck-pose {
		animation: none;
	}

	:global(:root[data-reduced-motion='true']) .river-wake,
	:global(:root[data-reduced-motion='true']) .river-duck {
		transition: none;
	}
</style>
