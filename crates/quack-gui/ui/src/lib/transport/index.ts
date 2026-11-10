import type { Transport } from './types';
import { devTransport } from './dev';
import { tauriTransport } from './tauri';

export type { AppInfo, OpenedTab, RecentFolder, ResumeRequest, TabMessage, Transport } from './types';

let transport: Transport | undefined;

/**
 * Pick the transport once: the Tauri IPC one when running inside the desktop shell
 * (the `@tauri-apps/api` modules are import-safe in a plain browser; only the calls
 * need the shell), the scripted demo one otherwise (`npm run dev`).
 */
export function getTransport(): Transport {
	if (!transport) {
		transport = '__TAURI_INTERNALS__' in window ? tauriTransport() : devTransport();
	}
	return transport;
}
