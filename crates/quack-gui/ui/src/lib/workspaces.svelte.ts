import { Session } from '#lib/session.svelte';
import type { ResumeRequest, TabMessage, Transport } from '#lib/transport/types';

/** Own the listener for the lifetime of the page, including the open IPC handshake. */
export class Workspaces {
	sessions = $state<Session[]>([]);
	activeIndex = $state(-1);
	opening = $state(false);
	error = $state<string | null>(null);
	#transport: Transport;
	#unlisten: () => void;

	constructor(transport: Transport) {
		this.#transport = transport;
		this.#unlisten = transport.listen((msg) => this.#receive(msg));
	}

	#receive(msg: TabMessage) {
		let session = this.sessions.find((s) => s.tab === msg.tab);
		// Info is always the first message. Register the session immediately: replay,
		// trust, and model events can all precede the open_folder command response.
		if (!session && 'info' in msg && this.opening) {
			session = new Session(this.#transport, msg.tab, msg.info);
			this.sessions.push(session);
		}
		if (!session) return;
		if ('event' in msg) session.onEvent(msg.event);
		else if ('info' in msg) session.info = msg.info;
		else session.onClosed(msg.closed.reason);
	}

	async open(cwd: string, resume: ResumeRequest = { resume: 'new' }) {
		if (this.opening) return;
		this.opening = true;
		this.error = null;
		try {
			const { tab, info } = await this.#transport.openFolder(cwd, resume);
			if (!this.sessions.some((s) => s.tab === tab)) {
				this.sessions.push(new Session(this.#transport, tab, info));
			}
			this.activeIndex = this.sessions.findIndex((s) => s.tab === tab);
			await this.sessions[this.activeIndex].listModels();
			await this.sessions[this.activeIndex].listSessions();
		} catch (error) {
			if (isAlreadyOpen(error)) {
				this.activeIndex = this.sessions.findIndex((s) => s.tab === error.detail.tab);
			} else {
				this.error = errorMessage(error);
			}
		} finally {
			this.opening = false;
		}
	}

	async close(index: number) {
		const session = this.sessions[index];
		try {
			await this.#transport.closeTab(session.tab);
			const currentIndex = this.sessions.indexOf(session);
			this.sessions.splice(currentIndex, 1);
			if (this.activeIndex > currentIndex) this.activeIndex--;
			else if (this.activeIndex >= this.sessions.length) this.activeIndex = this.sessions.length - 1;
		} catch (error) {
			this.error = errorMessage(error);
		}
	}

	dispose() {
		this.#unlisten();
	}
}

function isAlreadyOpen(error: unknown): error is { kind: 'already_open'; detail: { tab: number } } {
	if (typeof error !== 'object' || error === null || !('kind' in error) || error.kind !== 'already_open') return false;
	return 'detail' in error && typeof error.detail === 'object' && error.detail !== null &&
		'tab' in error.detail && typeof error.detail.tab === 'number';
}

function errorMessage(error: unknown): string {
	if (typeof error === 'object' && error !== null && 'detail' in error && typeof error.detail === 'string') {
		return error.detail;
	}
	return error instanceof Error ? error.message : String(error);
}
