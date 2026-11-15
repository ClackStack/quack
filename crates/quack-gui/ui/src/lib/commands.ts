export type ModalKind =
	| 'model'
	| 'context'
	| 'egress'
	| 'memory'
	| 'doctor'
	| 'diagnostics'
	| 'instructions'
	| 'profiles';

export interface BuiltinSlashCommand {
	name: string;
	aliases: string[];
	args: string;
	description: string;
	category: 'Session' | 'Inspect' | 'Security' | 'Config';
	shortcut?: string;
}

export const BUILTIN_COMMANDS: BuiltinSlashCommand[] = [
	{
		name: 'models',
		aliases: ['model'],
		args: '',
		description: 'Switch active or planner model and run tool-calling probe',
		category: 'Config'
	},
	{
		name: 'context',
		aliases: ['ctx'],
		args: '',
		description: 'Inspect KV-cache prefix vs uncached tail & selectively trim context',
		category: 'Inspect'
	},
	{
		name: 'compact',
		aliases: ['summarize', 'summary'],
		args: '[focus]',
		description: 'Compact older conversation turns into a summary',
		category: 'Session'
	},
	{
		name: 'prune',
		aliases: [],
		args: '',
		description: 'Prune older tool outputs without rewriting the cached system prefix',
		category: 'Inspect'
	},
	{
		name: 'probe',
		aliases: [],
		args: '',
		description: 'Probe whether the active model returns structured tool calls cleanly',
		category: 'Inspect'
	},
	{
		name: 'egress',
		aliases: ['shield', 'network'],
		args: '',
		description: 'View boundary policy, Secret Shield settings, and live egress ledger',
		category: 'Security'
	},
	{
		name: 'diagnostics',
		aliases: ['diag', 'lsp'],
		args: '',
		description: 'Inspect per-file diagnostics from connected language servers',
		category: 'Inspect'
	},
	{
		name: 'doctor',
		aliases: [],
		args: '',
		description: 'Run environment, storage, binary, and provider health checks',
		category: 'Inspect'
	},
	{
		name: 'instructions',
		aliases: ['agents'],
		args: '',
		description: 'Inspect loaded AGENTS.md and instruction files with token sizes',
		category: 'Inspect'
	},
	{
		name: 'profiles',
		aliases: ['profile'],
		args: '[id]',
		description: 'Inspect or switch model prompt profile',
		category: 'Config'
	},
	{
		name: 'settings',
		aliases: ['config'],
		args: '[section]',
		description: 'Open settings & TOML configuration',
		category: 'Config'
	},
	{
		name: 'memory',
		aliases: [],
		args: '',
		description: 'Browse, edit, or delete project and global memory notes',
		category: 'Config'
	},
	{
		name: 'providers',
		aliases: ['setup'],
		args: '',
		description: 'Manage configured model providers, keys, and local servers',
		category: 'Config'
	},
	{
		name: 'new',
		aliases: [],
		args: '',
		description: 'Start a fresh conversation in this workspace',
		category: 'Session'
	},
	{
		name: 'fork',
		aliases: [],
		args: '[title]',
		description: 'Fork the conversation into a clean branch session',
		category: 'Session'
	},
	{
		name: 'rewind',
		aliases: [],
		args: '',
		description: 'Open conversation & file checkpoint rewind picker',
		category: 'Session'
	},
	{
		name: 'undo',
		aliases: [],
		args: '',
		description: 'Undo the last workspace edit checkpoint',
		category: 'Session'
	},
	{
		name: 'title',
		aliases: [],
		args: '<text>',
		description: 'Rename the active session',
		category: 'Session'
	},
	{
		name: 'theme',
		aliases: [],
		args: '[auto|light|dark]',
		description: 'Switch or cycle the desktop UI theme',
		category: 'Config'
	},
	{
		name: 'help',
		aliases: ['?'],
		args: '',
		description: 'Open Command Palette & shortcuts',
		category: 'Session',
		shortcut: '⌘K'
	}
];

export interface ParsedSlashCommand {
	def: BuiltinSlashCommand;
	args: string;
}

const BY_NAME = new Map<string, BuiltinSlashCommand>();
for (const cmd of BUILTIN_COMMANDS) {
	BY_NAME.set(cmd.name, cmd);
	for (const alias of cmd.aliases) BY_NAME.set(alias, cmd);
}

/**
 * Parse a single-line composer submission into a built-in desktop slash command.
 * Unknown `/commands` (e.g. project skills or MCP prompts) return `null` so they
 * continue straight to the engine as a user turn.
 */
export function parseSlashCommand(raw: string): ParsedSlashCommand | null {
	const trimmed = raw.trim();
	if (!trimmed.startsWith('/') || trimmed.includes('\n')) return null;
	const body = trimmed.slice(1).trim();
	if (!body) return null;
	const space = body.search(/\s/);
	const head = (space < 0 ? body : body.slice(0, space)).toLowerCase();
	const args = space < 0 ? '' : body.slice(space + 1).trim();
	const def = BY_NAME.get(head);
	return def ? { def, args } : null;
}

export interface CommandSessionHost {
	openModels?(): void;
	openSetup(): Promise<unknown>;
	openSettings(tab?: 'paths' | 'setup' | 'context' | 'permissions' | 'extensions' | 'network' | 'interface'): void;
	refreshContext(): Promise<unknown>;
	compact(focus?: string): Promise<unknown>;
	pruneContext(indices?: number[]): Promise<unknown>;
	probeToolCalling(): Promise<unknown>;
	refreshEgress(): Promise<unknown>;
	runDoctor(): Promise<unknown>;
	refreshLsp(): Promise<unknown>;
	refreshLspDiagnostics(serverId?: string): Promise<unknown>;
	listInstructions(): Promise<unknown>;
	listProfiles(): Promise<unknown>;
	setProfile(id: string): Promise<unknown>;
	newSession(): Promise<unknown>;
	forkSession(openModelPicker?: boolean): Promise<unknown>;
	listRewindPoints(): Promise<unknown>;
	undo(): Promise<unknown>;
	setSessionTitle(title: string): Promise<unknown>;
}

export interface CommandUiHost {
	openModal(modal: ModalKind): void;
	openPalette(): void;
	openRewind(): void;
	setTheme(mode?: string): void;
}

export async function executeBuiltinCommand(
	parsed: ParsedSlashCommand,
	session: CommandSessionHost,
	ui: CommandUiHost
): Promise<void> {
	switch (parsed.def.name) {
		case 'models':
			ui.openModal('model');
			break;
		case 'context':
			void session.refreshContext();
			ui.openModal('context');
			break;
		case 'compact':
			await session.compact(parsed.args || undefined);
			break;
		case 'prune':
			await session.pruneContext();
			break;
		case 'probe':
			ui.openModal('model');
			await session.probeToolCalling();
			break;
		case 'egress':
			void session.refreshEgress();
			ui.openModal('egress');
			break;
		case 'diagnostics':
			void session.refreshLsp();
			void session.refreshLspDiagnostics();
			ui.openModal('diagnostics');
			break;
		case 'doctor':
			void session.runDoctor();
			ui.openModal('doctor');
			break;
		case 'instructions':
			void session.listInstructions();
			ui.openModal('instructions');
			break;
		case 'profiles':
			if (parsed.args) {
				await session.setProfile(parsed.args);
			}
			void session.listProfiles();
			ui.openModal('profiles');
			break;
		case 'settings':
			if (
				parsed.args === 'paths' ||
				parsed.args === 'setup' ||
				parsed.args === 'context' ||
				parsed.args === 'permissions' ||
				parsed.args === 'extensions' ||
				parsed.args === 'network' ||
				parsed.args === 'interface'
			) {
				session.openSettings(parsed.args);
			} else {
				session.openSettings();
			}
			break;
		case 'memory':
			ui.openModal('memory');
			break;
		case 'providers':
			await session.openSetup();
			break;
		case 'new':
			await session.newSession();
			break;
		case 'fork':
			await session.forkSession(false);
			break;
		case 'rewind':
			void session.listRewindPoints();
			ui.openRewind();
			break;
		case 'undo':
			await session.undo();
			break;
		case 'title':
			if (parsed.args) await session.setSessionTitle(parsed.args);
			break;
		case 'theme':
			ui.setTheme(parsed.args || undefined);
			break;
		case 'help':
			ui.openPalette();
			break;
	}
}
