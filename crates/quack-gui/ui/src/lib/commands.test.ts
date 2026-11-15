import { describe, expect, it, vi } from 'vitest';
import {
	BUILTIN_COMMANDS,
	executeBuiltinCommand,
	parseSlashCommand,
	type CommandSessionHost,
	type CommandUiHost
} from './commands';

function hosts() {
	const session: CommandSessionHost = {
		openSetup: vi.fn().mockResolvedValue(true),
		openSettings: vi.fn(),
		refreshContext: vi.fn().mockResolvedValue(true),
		compact: vi.fn().mockResolvedValue(true),
		pruneContext: vi.fn().mockResolvedValue(true),
		probeToolCalling: vi.fn().mockResolvedValue(true),
		refreshEgress: vi.fn().mockResolvedValue(true),
		runDoctor: vi.fn().mockResolvedValue(true),
		refreshLsp: vi.fn().mockResolvedValue(true),
		refreshLspDiagnostics: vi.fn().mockResolvedValue(true),
		listInstructions: vi.fn().mockResolvedValue(true),
		listProfiles: vi.fn().mockResolvedValue(true),
		setProfile: vi.fn().mockResolvedValue(true),
		newSession: vi.fn().mockResolvedValue(true),
		forkSession: vi.fn().mockResolvedValue(true),
		listRewindPoints: vi.fn().mockResolvedValue(true),
		undo: vi.fn().mockResolvedValue(true),
		setSessionTitle: vi.fn().mockResolvedValue(true)
	};
	const ui: CommandUiHost = {
		openModal: vi.fn(),
		openPalette: vi.fn(),
		openRewind: vi.fn(),
		setTheme: vi.fn()
	};
	return { session, ui };
}

describe('builtin slash commands', () => {
	it('parses built-in names and aliases with optional arguments', () => {
		expect(parseSlashCommand('/ctx')?.def.name).toBe('context');
		expect(parseSlashCommand('/shield')?.def.name).toBe('egress');
		expect(parseSlashCommand('/lsp')?.def.name).toBe('diagnostics');
		expect(parseSlashCommand('/agents')?.def.name).toBe('instructions');
		const titled = parseSlashCommand('  /title   Investigate egress ledger  ');
		expect(titled?.def.name).toBe('title');
		expect(titled?.args).toBe('Investigate egress ledger');
	});

	it('leaves custom slash commands and multiline input to the engine', () => {
		expect(parseSlashCommand('/review src/lib.rs')).toBeNull();
		expect(parseSlashCommand('/models\nextra line')).toBeNull();
		expect(parseSlashCommand('not a slash command')).toBeNull();
	});

	it('includes every major parity command cleanly', () => {
		const names = new Set(BUILTIN_COMMANDS.map((c) => c.name));
		for (const expected of [
			'models',
			'context',
			'compact',
			'prune',
			'probe',
			'egress',
			'diagnostics',
			'doctor',
			'instructions',
			'profiles',
			'settings',
			'memory',
			'providers',
			'new',
			'fork',
			'rewind',
			'undo',
			'title',
			'theme',
			'help'
		]) {
			expect(names.has(expected)).toBe(true);
		}
	});

	it('dispatches built-in commands to the session and UI hosts', async () => {
		const { session, ui } = hosts();

		await executeBuiltinCommand(parseSlashCommand('/probe')!, session, ui);
		expect(ui.openModal).toHaveBeenCalledWith('model');
		expect(session.probeToolCalling).toHaveBeenCalledTimes(1);

		await executeBuiltinCommand(parseSlashCommand('/egress')!, session, ui);
		expect(session.refreshEgress).toHaveBeenCalledTimes(1);
		expect(ui.openModal).toHaveBeenCalledWith('egress');

		await executeBuiltinCommand(parseSlashCommand('/diagnostics')!, session, ui);
		expect(session.refreshLsp).toHaveBeenCalledTimes(1);
		expect(session.refreshLspDiagnostics).toHaveBeenCalledTimes(1);
		expect(ui.openModal).toHaveBeenCalledWith('diagnostics');

		await executeBuiltinCommand(parseSlashCommand('/doctor')!, session, ui);
		expect(session.runDoctor).toHaveBeenCalledTimes(1);
		expect(ui.openModal).toHaveBeenCalledWith('doctor');

		await executeBuiltinCommand(parseSlashCommand('/instructions')!, session, ui);
		expect(session.listInstructions).toHaveBeenCalledTimes(1);
		expect(ui.openModal).toHaveBeenCalledWith('instructions');

		await executeBuiltinCommand(parseSlashCommand('/profiles cc-style')!, session, ui);
		expect(session.setProfile).toHaveBeenCalledWith('cc-style');
		expect(session.listProfiles).toHaveBeenCalledTimes(1);
		expect(ui.openModal).toHaveBeenCalledWith('profiles');

		await executeBuiltinCommand(parseSlashCommand('/settings network')!, session, ui);
		expect(session.openSettings).toHaveBeenCalledWith('network');

		await executeBuiltinCommand(parseSlashCommand('/title My bug hunt')!, session, ui);
		expect(session.setSessionTitle).toHaveBeenCalledWith('My bug hunt');

		await executeBuiltinCommand(parseSlashCommand('/rewind')!, session, ui);
		expect(session.listRewindPoints).toHaveBeenCalledTimes(1);
		expect(ui.openRewind).toHaveBeenCalledTimes(1);

		await executeBuiltinCommand(parseSlashCommand('/theme dark')!, session, ui);
		expect(ui.setTheme).toHaveBeenCalledWith('dark');

		await executeBuiltinCommand(parseSlashCommand('/help')!, session, ui);
		expect(ui.openPalette).toHaveBeenCalledTimes(1);
	});
});
