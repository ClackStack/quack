import { describe, expect, it } from 'vitest';
import { inlineSpans, renderMarkdown } from './markdown';

describe('inlineSpans', () => {
	it('escapes html before anything else', () => {
		const blocks = renderMarkdown('a <b> & c');
		expect(blocks[0]).toEqual({ type: 'p', spans: 'a &lt;b&gt; &amp; c' });
	});

	it('renders bold, italic, code and links', () => {
		const out = inlineSpans('**bold** and *soft* and `x=1` and [pond](https://example.com)');
		expect(out).toContain('<strong>bold</strong>');
		expect(out).toContain('<em>soft</em>');
		expect(out).toContain('<code>x=1</code>');
		expect(out).toContain('<a href="https://example.com"');
	});

	it('keeps javascript: links out', () => {
		expect(inlineSpans('[click](javascript:alert(1))')).not.toContain('<a ');
	});

	it('protects code spans from other transforms', () => {
		expect(inlineSpans('`a *b* c`')).toBe('<code>a *b* c</code>');
	});
});

describe('renderMarkdown', () => {
	it('splits fenced code with its language', () => {
		const blocks = renderMarkdown('```ts\nconst a = 1;\n```');
		expect(blocks).toEqual([{ type: 'code', lang: 'ts', text: 'const a = 1;' }]);
	});

	it('groups list items and keeps inline spans', () => {
		const blocks = renderMarkdown('- one\n- **two**\n3. three');
		expect(blocks[0]).toEqual({ type: 'ul', items: ['one', '<strong>two</strong>'] });
		expect(blocks[1]).toEqual({ type: 'ol', items: ['three'] });
	});

	it('joins paragraph lines and escapes them', () => {
		const blocks = renderMarkdown('hello\nworld <tag>');
		expect(blocks[0]).toEqual({ type: 'p', spans: 'hello\nworld &lt;tag&gt;' });
	});
});
