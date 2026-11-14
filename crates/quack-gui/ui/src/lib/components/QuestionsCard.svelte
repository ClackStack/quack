<script lang="ts">
	import type { Question } from '#lib/protocol';

	let {
		questions,
		onAnswer,
		busy = false
	}: {
		questions: Question[];
		busy?: boolean;
		onAnswer: (answers: { picked?: number; text?: string }[]) => void;
	} = $props();

	let picks = $state<Record<number, number>>({});
	let texts = $state<Record<number, string>>({});

	const options = (q: Question) => q.options ?? [];

	const ready = $derived(
		questions.every((q, i) => (options(q).length > 0 ? picks[i] !== undefined : true))
	);

	function submit() {
		if (busy || !ready) return;
		onAnswer(
			questions.map((q, i) =>
				options(q).length > 0 ? { picked: picks[i] } : { text: texts[i] ?? '' }
			)
		);
	}
</script>

<section class="questions" aria-label="The model asks">
	{#each questions as question, i (i)}
		<div class="q">
			{#if question.header}<span class="pill">{question.header}</span>{/if}
			<p>{question.question}</p>
			{#if options(question).length > 0}
				<div class="options" role="radiogroup" aria-label={question.question}>
					{#each options(question) as option, j (j)}
						<label class="option" class:picked={picks[i] === j}>
							<input
								type="radio"
								name={`q${i}`}
								disabled={busy}
								checked={picks[i] === j}
								onchange={() => (picks[i] = j)}
							/>
							<span class="label">{option.label}</span>
							{#if option.description}<span class="desc">{option.description}</span>{/if}
						</label>
					{/each}
				</div>
			{:else}
				<input
					class="text"
					disabled={busy}
					type="text"
					aria-label={question.question}
					placeholder="Your answer…"
					bind:value={texts[i]}
					onkeydown={(e) => {
						if (e.key === 'Enter' && questions.length === 1) submit();
					}}
				/>
			{/if}
		</div>
	{/each}
	<div class="actions">
		<button class="btn btn-primary" disabled={!ready || busy} onclick={submit}>Answer</button>
	</div>
</section>

<style>
	.questions {
		margin: 0 var(--sp-lg) var(--sp-sm);
		border: 1px solid var(--sky);
		border-radius: var(--r-lg);
		background: var(--sky-soft);
		padding: 12px 14px;
		display: grid;
		gap: var(--sp-md);
	}

	.q {
		display: grid;
		gap: var(--sp-sm);
		justify-items: start;
	}

	.q > .pill {
		justify-self: start;
	}

	.q p {
		margin: 0;
		font-family: var(--font-serif);
		font-size: 14.5px;
	}

	.options {
		display: grid;
		gap: var(--sp-xs);
		width: 100%;
	}

	.option {
		display: grid;
		grid-template-columns: auto auto 1fr;
		align-items: baseline;
		gap: 8px;
		padding: 7px 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		background: var(--bg-panel);
		cursor: pointer;
	}

	.option:hover {
		border-color: var(--sky-ink);
	}

	.option.picked {
		border-color: var(--sky-ink);
		background: var(--bg-raised);
		box-shadow: var(--shadow-card);
	}

	.label {
		font-weight: 600;
	}

	.desc {
		color: var(--ink-3);
		font-size: 12.5px;
	}

	.text {
		width: 100%;
		padding: 7px 10px;
		border: 1px solid var(--rule);
		border-radius: var(--r-md);
		background: var(--bg-panel);
	}

	.actions {
		display: flex;
		justify-content: flex-end;
	}
</style>
