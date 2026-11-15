/**
 * A small, dependency-free markdown renderer for transcript prose (docs/plan/15-desktop-client.md#the-ui):
 * headings, paragraphs, fenced code, lists, quotes, rules, and inline bold / italic /
 * code / links / strikethrough. Everything is HTML-escaped before inline transforms
 * run, so model output can never inject markup; links are restricted to known schemes.
 * Syntax highlighting arrives later (the plan's open question: quack's tree-sitter
 * grammars over IPC vs lean shiki).
 */

export type Block =
	| { type: 'p'; spans: string }
	| { type: 'h'; level: number; spans: string }
	| { type: 'code'; lang: string; text: string }
	| { type: 'ul'; items: string[] }
	| { type: 'ol'; items: string[] }
	| { type: 'quote'; spans: string }
	| { type: 'hr' };

const SAFE_URL = /^(https?:\/\/|mailto:|\/|#)/i;

function esc(text: string): string {
	return text
		.replaceAll('&', '&amp;')
		.replaceAll('<', '&lt;')
		.replaceAll('>', '&gt;')
		.replaceAll('"', '&quot;');
}

/** Inline markdown over already-escaped text; returns HTML for {@html}. */
export function inlineSpans(escaped: string): string {
	let out = escaped;
	// inline code first, so its content is protected from the other transforms
	const codes: string[] = [];
	out = out.replace(/`([^`]+)`/g, (_, code: string) => {
		codes.push(code);
		return `\u0000${codes.length - 1}\u0000`;
	});
	out = out.replace(/\[([^\]]+)\]\(([^)\s]+)\)/g, (m, label: string, url: string) => {
		if (!SAFE_URL.test(url)) return m;
		return `<a href="${url}" target="_blank" rel="noreferrer">${label}</a>`;
	});
	out = out.replace(/\*\*([^*]+)\*\*/g, '<strong>$1</strong>');
	out = out.replace(/__([^_]+)__/g, '<strong>$1</strong>');
	out = out.replace(/(^|[\s(])\*([^*\s][^*]*)\*/g, '$1<em>$2</em>');
	out = out.replace(/(^|[\s(])_([^_\s][^_]*)_/g, '$1<em>$2</em>');
	out = out.replace(/~~([^~]+)~~/g, '<del>$1</del>');
	out = out.replace(/\u0000(\d+)\u0000/g, (_, i: string) => `<code>${codes[Number(i)]}</code>`);
	return out;
}

/** Parse transcript markdown into blocks. Fenced code wins over everything. */
export function renderMarkdown(src: string): Block[] {
	const lines = src.split('\n');
	const blocks: Block[] = [];
	let i = 0;

	const isUl = (line: string) => /^\s*[-*+]\s+/.test(line);
	const isOl = (line: string) => /^\s*\d+[.)]\s+/.test(line);
	const stripMarker = (line: string, re: RegExp) => line.replace(re, '');

	while (i < lines.length) {
		const line = lines[i];

		if (line.trim() === '') {
			i += 1;
			continue;
		}

		const fence = /^```(\w*)\s*$/.exec(line);
		if (fence) {
			const lang = fence[1] ?? '';
			const body: string[] = [];
			i += 1;
			while (i < lines.length && !lines[i].startsWith('```')) {
				body.push(lines[i]);
				i += 1;
			}
			i += 1; // closing fence (or EOF)
			blocks.push({ type: 'code', lang, text: body.join('\n') });
			continue;
		}

		const heading = /^(#{1,6})\s+(.*)$/.exec(line);
		if (heading) {
			blocks.push({ type: 'h', level: heading[1].length, spans: inlineSpans(esc(heading[2])) });
			i += 1;
			continue;
		}

		if (/^\s*(---+|\*\*\*+)\s*$/.test(line)) {
			blocks.push({ type: 'hr' });
			i += 1;
			continue;
		}

		if (isUl(line)) {
			const items: string[] = [];
			while (i < lines.length && isUl(lines[i])) {
				items.push(inlineSpans(esc(stripMarker(lines[i], /^\s*[-*+]\s+/))));
				i += 1;
			}
			blocks.push({ type: 'ul', items });
			continue;
		}

		if (isOl(line)) {
			const items: string[] = [];
			while (i < lines.length && isOl(lines[i])) {
				items.push(inlineSpans(esc(stripMarker(lines[i], /^\s*\d+[.)]\s+/))));
				i += 1;
			}
			blocks.push({ type: 'ol', items });
			continue;
		}

		if (/^\s*>\s?/.test(line)) {
			const quoted: string[] = [];
			while (i < lines.length && /^\s*>\s?/.test(lines[i])) {
				quoted.push(inlineSpans(esc(lines[i].replace(/^\s*>\s?/, ''))));
				i += 1;
			}
			blocks.push({ type: 'quote', spans: quoted.join('<br>') });
			continue;
		}

		// paragraph: consecutive plain lines
		const para: string[] = [];
		while (
			i < lines.length &&
			lines[i].trim() !== '' &&
			!/^```/.test(lines[i]) &&
			!/^(#{1,6})\s+/.test(lines[i]) &&
			!isUl(lines[i]) &&
			!isOl(lines[i]) &&
			!/^\s*>\s?/.test(lines[i])
		) {
			para.push(lines[i]);
			i += 1;
		}
		blocks.push({ type: 'p', spans: inlineSpans(esc(para.join('\n'))) });
	}
	return blocks;
}
