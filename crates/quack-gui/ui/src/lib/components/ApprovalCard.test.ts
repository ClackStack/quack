import { render } from 'svelte/server';
import { expect, it, vi } from 'vitest';
import type { SecretFinding } from '#lib/protocol';
import ApprovalCard from './ApprovalCard.svelte';

const legacy: SecretFinding = {
	id: 'sec-1',
	kind: 'github_token',
	location: 'tool:read',
	preview: 'ghp_••••9876',
	fingerprint: 'sha256:abcdef1234567890'
};

function renderWarning(...findings: SecretFinding[]) {
	return render(ApprovalCard, {
		props: {
			approval: {
				callId: 'egress-1',
				request: { kind: 'egress_secrets', provider: 'work-model', host: 'api.example.test', findings },
				always: []
			},
			onAnswer: vi.fn()
		}
	}).body;
}

it('explains the source, destination and consequences of a flagged send', () => {
	const html = renderWarning({
		...legacy,
		line: 12,
		context: {
			label: 'Possible GitHub token',
			source: 'Tool output: read',
			impact: 'If valid, it could grant access to GitHub resources.'
		}
	});
	for (const text of [
		'work-model',
		'api.example.test',
		'Possible GitHub token',
		'Tool output: read, scanned line 12',
		'If valid, it could grant access to GitHub resources.',
		'ghp_••••9876',
		'later requests may include it again',
		'Redact &amp; send',
		'Send unredacted',
		'Withhold flagged text'
	]) {
		expect(html).toContain(text);
	}
	expect(html).not.toContain(legacy.fingerprint);
});

it('shows a safe explanation for findings without context', () => {
	const html = renderWarning(legacy);
	expect(html).toContain('github_token');
	expect(html).toContain('tool:read');
	expect(html).toContain('If this is a real credential');
	expect(html).not.toContain('scanned line');
});
