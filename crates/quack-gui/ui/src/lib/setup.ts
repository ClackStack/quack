import type { ProviderModels, ServiceInfo } from '#lib/protocol';

export interface EnvProviderEntry {
	id: string;
	name: string | null;
	envVar: string;
	available: boolean;
	/** Non-boilerplate status note (for example, auth or network failure when env var is set). */
	extraNote: string | null;
}

export interface OtherUnavailableEntry {
	id: string;
	name: string | null;
	note: string;
}

export interface NoteSegment {
	text: string;
	code: boolean;
}

/**
 * Extracts the environment variable a provider reads from its structured key source,
 * falling back to the engine's `"set <VAR> to use this provider"` note when omitted.
 */
export function envVarForProvider(provider: ProviderModels): string | null {
	if (provider.key?.from === 'env' && provider.key.var) {
		return provider.key.var;
	}
	const match = /^set ([A-Z0-9_]+) to use this provider$/i.exec(provider.note.trim());
	return match ? match[1] : null;
}

/**
 * Groups providers into environment-variable entries and any remaining unavailable
 * providers so SetupPanel can present expert setup details inside a collapsed accordion.
 */
export function partitionExpertProviders(
	providers: ProviderModels[],
	services: ServiceInfo[]
): {
	envProviders: EnvProviderEntry[];
	otherUnavailable: OtherUnavailableEntry[];
} {
	const serviceNames = new Map(services.map((s) => [s.id, s.name]));
	const envProviders: EnvProviderEntry[] = [];
	const otherUnavailable: OtherUnavailableEntry[] = [];

	for (const provider of providers) {
		const serviceName = serviceNames.get(provider.id);
		const name = serviceName && serviceName !== provider.id ? serviceName : null;
		const envVar = envVarForProvider(provider);

		if (envVar) {
			const trimmed = provider.note.trim();
			const defaultNote = `set ${envVar} to use this provider`;
			const extraNote =
				!provider.available &&
				trimmed.length > 0 &&
				trimmed.toLowerCase() !== defaultNote.toLowerCase()
					? trimmed
					: null;
			envProviders.push({
				id: provider.id,
				name,
				envVar,
				available: provider.available,
				extraNote
			});
		} else if (!provider.available) {
			otherUnavailable.push({
				id: provider.id,
				name,
				note: provider.note.trim() || 'Unavailable'
			});
		}
	}

	envProviders.sort((a, b) => a.id.localeCompare(b.id));
	otherUnavailable.sort((a, b) => a.id.localeCompare(b.id));

	return { envProviders, otherUnavailable };
}

/**
 * Splits backtick-delimited inline code spans so notes like
 * ``run `quack login chatgpt` to use this provider`` render safely without raw HTML.
 */
export function splitInlineCode(text: string): NoteSegment[] {
	const segments: NoteSegment[] = [];
	const pattern = /`([^`]+)`/g;
	let cursor = 0;
	for (const match of text.matchAll(pattern)) {
		const idx = match.index ?? 0;
		if (idx > cursor) {
			segments.push({ text: text.slice(cursor, idx), code: false });
		}
		segments.push({ text: match[1], code: true });
		cursor = idx + match[0].length;
	}
	if (cursor < text.length) {
		segments.push({ text: text.slice(cursor), code: false });
	}
	return segments;
}
