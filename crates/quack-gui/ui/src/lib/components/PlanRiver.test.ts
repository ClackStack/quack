import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import PlanRiver from './PlanRiver.svelte';

describe('PlanRiver', () => {
	it('renders an accessible progressbar with the duck positioned by completed steps', () => {
		const html = render(PlanRiver, {
			props: { done: 1, total: 4, percent: 25, active: true, complete: false }
		}).body;

		expect(html).toContain('role="progressbar"');
		expect(html).toContain('aria-valuenow="1"');
		expect(html).toContain('aria-valuemax="4"');
		expect(html).toContain('1 of 4 steps complete (25%)');
		expect(html).toContain('--pos: 0.25');
		expect(html).toContain('paddling');
		expect(html).toContain('_(•)');
	});

	it('reserves the far bank until completion and stops paddling at 100%', () => {
		const nearly = render(PlanRiver, {
			props: { done: 99, total: 100, percent: 99, active: true, complete: false }
		}).body;
		expect(nearly).toContain('--pos: 0.94');
		expect(nearly).toContain('paddling');

		const finished = render(PlanRiver, {
			props: { done: 4, total: 4, percent: 100, active: true, complete: true }
		}).body;
		expect(finished).toContain('--pos: 1');
		expect(finished).toContain('complete');
		expect(finished).not.toContain('paddling');
	});
});
