<script lang="ts">
	import type { Session } from '#lib/session.svelte';

	let { session }: { session: Session } = $props();

	let text = $state('');
	let editor: HTMLTextAreaElement | undefined = $state();

	function autogrow() {
		if (!editor) return;
		editor.style.height = 'auto';
		editor.style.height = `${Math.min(editor.scrollHeight, 240)}px`;
	}

	async function send() {
		const body = text.trim();
		if (!body || session.approval) return;
		text = '';
		autogrow();
		await session.submit(body);
	}

	function onKeydown(event: KeyboardEvent) {
		if (event.key === 'Enter' && !event.shiftKey) {
			event.preventDefault();
			void send();
		}
	}

	// Keep the composer focused while nothing else asks for the keyboard; approvals
	// answer with Enter from anywhere.
	$effect(() => {
		if (!session.approval && !session.questions) editor?.focus();
	});
</script>

<div class="composer">
	<div class="box">
		<textarea
			bind:this={editor}
			bind:value={text}
			rows={1}
			placeholder={session.streaming ? 'quack is working — interrupt or wait…' : 'Ask quack to do something in this folder…'}
			onkeydown={onKeydown}
			oninput={autogrow}
			aria-label="Message"
		></textarea>
		<div class="row">
			<span class="hint">
				{#if session.streaming}
					<button class="btn stop" onclick={() => session.interrupt()}>Interrupt</button>
				{:else}
					<span><span class="kbd">⏎</span> send · <span class="kbd">⇧⏎</span> newline</span>
				{/if}
			</span>
			<button class="btn btn-primary" onclick={() => void send()} disabled={!text.trim() || session.approval !== null}>
				Send
			</button>
		</div>
	</div>
</div>

<style>
	.composer {
		padding: var(--sp-sm) var(--sp-lg) var(--sp-md);
	}

	.box {
		border: 1px solid var(--rule);
		border-radius: var(--r-xl);
		background: var(--bg-panel);
		box-shadow: var(--shadow-card);
		padding: 10px 12px 8px;
		display: grid;
		gap: 6px;
	}

	.box:focus-within {
		border-color: var(--sky-ink);
	}

	textarea {
		resize: none;
		border: none;
		background: none;
		outline: none;
		font-size: var(--text-prose);
		line-height: 1.5;
		min-height: 24px;
		max-height: 240px;
		color: var(--ink);
	}

	textarea::placeholder {
		color: var(--ink-3);
	}

	.row {
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
	}

	.hint {
		flex: 1;
		color: var(--ink-3);
		font-size: 12px;
		display: flex;
		align-items: center;
		gap: var(--sp-sm);
	}

	.stop {
		border-color: var(--fail);
		color: var(--fail);
		padding: 2px 10px;
	}

	.stop:hover:not(:disabled) {
		background: var(--fail-soft);
	}
</style>
