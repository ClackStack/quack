import { render } from 'svelte/server';
import { describe, expect, it } from 'vitest';
import WelcomeDuck from './WelcomeDuck.svelte';

describe('WelcomeDuck', () => {
	it('renders the duck and caption without requiring a click trigger', () => {
		const html = render(WelcomeDuck, {
			props: { reducedMotion: false, caption: 'ask quack to do something' }
		}).body;

		expect(html).not.toContain('<button');
		expect(html).toContain('(o )___');
		expect(html).toContain('~~~~~~~~~~');
		expect(html).toContain('ask quack to do something');
	});

	it('renders cleanly under reduced motion', () => {
		const reduced = render(WelcomeDuck, {
			props: { reducedMotion: true, caption: 'calm duck' }
		}).body;

		expect(reduced).not.toContain('<button');
		expect(reduced).toContain('(o )___');
		expect(reduced).toContain('calm duck');
	});
});
