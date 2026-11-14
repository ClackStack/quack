export type RiskLevel = 'low' | 'medium' | 'high';

export interface ShellRiskSummary {
	level: RiskLevel;
	summary: string;
	reasons: string[];
}

const RULES: { test: RegExp; level: RiskLevel; reason: string }[] = [
	{
		test: /\b(?:curl|wget|iwr|Invoke-WebRequest)\b[^|;\n]*\|\s*(?:sh|bash|zsh|dash|python3?|node)\b/i,
		level: 'high',
		reason: 'Downloads remote code and executes it right away.'
	},
	{
		test: /(?:^|[;&|(\s])(?:sudo|doas|su)\s+/i,
		level: 'high',
		reason: 'Requests elevated administrator privileges.'
	},
	{
		test: /(?:^|[;&|(\s])rm\s+[^\n]*(?:-[^\s]*r[^\s]*f|-[^\s]*f[^\s]*r|--recursive\s+--force|--force\s+--recursive|-rf|-fr)\b|\b(?:mkfs(?:\.\w+)?|shred|wipefs|dd\s+if=|git\s+reset\s+--hard|git\s+clean\s+-[^\s]*[fdx])/i,
		level: 'high',
		reason: 'Deletes or overwrites files and changes irreversibly.'
	},
	{
		test: /(?:^|[;&|(\s])chmod\s+[^\n]*(?:777|a\+rwx)\b|\bchown\s+-R\b/i,
		level: 'high',
		reason: 'Widens filesystem permissions recursively or globally.'
	},
	{
		test: /\b(?:curl|wget|ssh|scp|rsync|nc|ncat|nmap|git\s+push|npm\s+publish|cargo\s+publish|pnpm\s+publish|gh\s+(?:pr|issue|release)\s+(?:create|merge))\b/i,
		level: 'medium',
		reason: 'Contacts a remote host, transfers data, or publishes changes.'
	},
	{
		test: /\b(?:npm\s+(?:i|install)|pnpm\s+(?:add|install)|yarn\s+add|pip3?\s+install|cargo\s+install|brew\s+install|apt(?:-get)?\s+install|dnf\s+install)\b/i,
		level: 'medium',
		reason: 'Installs packages or modifies your tool environment.'
	},
	{
		test: /(?:^|[^-])>>?\s*(?:\/(?:etc|usr|bin|sbin|var|System)|~\/)/,
		level: 'high',
		reason: 'Redirects file output outside the current project folder.'
	}
];

const LEVEL_RANK: Record<RiskLevel, number> = { low: 0, medium: 1, high: 2 };

/**
 * Translates a raw shell command and sandbox state into a plain-English explanation for
 * confirmation prompts.
 */
export function summarizeShellCommand(command: string, outsideSandbox: boolean): ShellRiskSummary {
	const trimmed = command.trim();
	const reasons: string[] = [];
	let level: RiskLevel = 'low';

	const pushReason = (text: string) => {
		if (!reasons.includes(text)) reasons.push(text);
	};

	if (outsideSandbox) {
		pushReason(
			'Runs outside the workspace sandbox — can read and write anywhere your account can.'
		);
		level = 'high';
	}

	for (const rule of RULES) {
		if (rule.test.test(trimmed)) {
			pushReason(rule.reason);
			if (LEVEL_RANK[rule.level] > LEVEL_RANK[level]) level = rule.level;
		}
	}

	if (/[;&|]/.test(trimmed) && trimmed.split(/\s*(?:&&|\|\||;|\|)\s*/).filter(Boolean).length > 1) {
		if (reasons.length === 0) {
			pushReason('Chains multiple commands together in one shell invocation.');
		}
	}

	if (reasons.length === 0) {
		reasons.push('Runs inside the workspace directory within the configured sandbox.');
	}

	const summaryByLevel: Record<RiskLevel, string> = {
		low: 'Routine workspace command',
		medium: 'Moderate impact — modifies packages or uses network',
		high: 'High impact — review before running'
	};
	const summary = summaryByLevel[level];

	return { level, summary, reasons };
}
