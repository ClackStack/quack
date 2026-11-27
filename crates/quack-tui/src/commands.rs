//! Built-in slash command catalogue (docs/plan/06-tui.md#command-catalogue).

#[derive(Debug, Clone, Copy)]
pub(crate) struct Command {
    pub name: &'static str,
    pub aliases: &'static [&'static str],
    pub args: &'static str,
    pub description: &'static str,
    /// `None` when implemented; otherwise the milestone it lands in.
    pub pending: Option<&'static str>,
}

impl Command {
    pub fn matches(&self, name: &str) -> bool {
        self.name == name || self.aliases.contains(&name)
    }
}

const fn c(
    name: &'static str,
    aliases: &'static [&'static str],
    args: &'static str,
    description: &'static str,
    pending: Option<&'static str>,
) -> Command {
    Command {
        name,
        aliases,
        args,
        description,
        pending,
    }
}

pub(crate) const COMMANDS: &[Command] = &[
    c(
        "models",
        &["model"],
        "[provider/model]",
        "Pick a model for this session; with an argument, switch directly",
        None,
    ),
    c(
        "thinking",
        &["think"],
        "[level|default|show|hide]",
        "Choose how much the current model thinks; default restores its configured level",
        None,
    ),
    c(
        "providers",
        &[],
        "",
        "Each provider: usable or why not, and its models",
        None,
    ),
    c(
        "key",
        &[],
        "[provider]",
        "Replace a provider's API key, for example after rotating it",
        None,
    ),
    c(
        "setup",
        &[],
        "",
        "Add a provider: a server on this machine, a hosted service, or any URL",
        None,
    ),
    c(
        "profiles",
        &["profile"],
        "[id]",
        "Model profiles: preview each one's prompt and tools; switch, or `auto`",
        None,
    ),
    c(
        "resume",
        &["sessions", "session"],
        "[id]",
        "Browse and resume past sessions; with an id, resume directly",
        None,
    ),
    c(
        "new",
        &["clear"],
        "",
        "Start a fresh session (the current one stays saved)",
        None,
    ),
    c(
        "rewind",
        &[],
        "[n]",
        "Go back to an earlier message: conversation, files or both (Esc Esc)",
        None,
    ),
    c(
        "undo",
        &[],
        "[--force]",
        "Revert the files changed by the last edit (not shell commands)",
        None,
    ),
    c(
        "fork",
        &[],
        "",
        "Go on in a copy of this session; the original stays as it was (/resume)",
        None,
    ),
    c(
        "plan",
        &[],
        "[task|accept [fresh]|off]",
        "Read/search and propose changes plus tests for review; accept (or accept fresh) or keep planning",
        None,
    ),
    c("title", &[], "<text>", "Rename the session", None),
    c(
        "export",
        &[],
        "[md|json] [path]",
        "Write the transcript to a file in the session's folder",
        None,
    ),
    c(
        "compact",
        &["summarize", "summary"],
        "[focus]",
        "Summarize earlier conversation now, keeping the current turn",
        None,
    ),
    c(
        "context",
        &[],
        "",
        "What is using the context, segment by segment",
        None,
    ),
    c(
        "config",
        &["settings"],
        "[set [--project|--user] <key> <value>]",
        "Settings with their origins; ^l toggles user config or .quack/",
        None,
    ),
    c(
        "scope",
        &[],
        "[user|project]",
        "Choose where settings and new MCP servers save (user config or .quack/)",
        None,
    ),
    c(
        "bundles",
        &[],
        "",
        "Installed bundles of skills, commands and servers; turn each on or off",
        None,
    ),
    c(
        "edit",
        &[],
        "[task]",
        "Switch to Edit, keeping approvals; optionally send a task",
        None,
    ),
    c(
        "permissions",
        &[],
        "[ask|allow]",
        "Set ask/allow approvals without changing Plan/Edit; see permission rules",
        None,
    ),
    c(
        "yolo",
        &[],
        "[on|off]",
        "Toggle yolo: nothing asks and no sandbox (the built-in deny list still applies)",
        None,
    ),
    c(
        "sandbox",
        &[],
        "[on|off]",
        "Toggle the OS sandbox for shell commands, keeping the permission mode",
        None,
    ),
    c(
        "mcp",
        &[],
        "[add | enable <id> | disable <id> | restart <id>]",
        "MCP servers: status, examples, enable/disable, add; ^s restarts one",
        None,
    ),
    c(
        "lsp",
        &[],
        "",
        "Language servers: state, indexing, diagnostics",
        None,
    ),
    c(
        "skills",
        &[],
        "",
        "Installed skills; run one with /<name> [request]",
        None,
    ),
    c(
        "commands",
        &[],
        "",
        "Custom commands from the commands/ folders",
        None,
    ),
    c(
        "compat",
        &[],
        "",
        "Review skills and commands found in other agents’ folders",
        None,
    ),
    c(
        "memory",
        &[],
        "",
        "Memory notes: read them, delete with ^s",
        None,
    ),
    c(
        "hooks",
        &[],
        "[configure|test|toggle <id>]",
        "Configure hook connections and credentials",
        None,
    ),
    c(
        "instructions",
        &[],
        "",
        "The AGENTS.md files the model has, with their sizes",
        None,
    ),
    c(
        "init",
        &[],
        "",
        "Write AGENTS.md, merging CLAUDE.md and other agents' files into it",
        None,
    ),
    c(
        "todo",
        &[],
        "",
        "Show the model's whole plan above the input, or just its summary",
        None,
    ),
    c(
        "tasks",
        &[],
        "",
        "Sub-agents started this session: what each did and its report",
        None,
    ),
    c(
        "theme",
        &[],
        "[dark|light|auto]",
        "Switch the color theme (`dark`, `light`, or `auto`)",
        None,
    ),
    c("keys", &[], "", "Key bindings", None),
    c(
        "doctor",
        &[],
        "",
        "Environment checks: config, providers, shell, git, storage",
        None,
    ),
    c(
        "stats",
        &[],
        "",
        "This session: tokens, cost, cache hits, where the time went, tools used",
        None,
    ),
    c(
        "reload",
        &[],
        "",
        "Read config, providers, MCP, language servers and skills again",
        None,
    ),
    c(
        "trust",
        &[],
        "[trusted|settings-only|ignored]",
        "Decide whether this repo's .quack/ loads",
        None,
    ),
    c(
        "copy",
        &[],
        "[code]",
        "Copy the last response, or its last code block, to the clipboard",
        None,
    ),
    c(
        "probe",
        &[],
        "",
        "Probe whether the active model returns structured tool calls cleanly",
        None,
    ),
    c(
        "gui",
        &[],
        "",
        "Hand off this workspace and session to the desktop GUI",
        None,
    ),
    c("help", &["?"], "", "Commands and keys", None),
    c("quit", &["exit"], "", "Quit", None),
];

pub(crate) fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.matches(name))
}

/// Default key bindings, for the help modal.
pub(crate) const KEYS: &[(&str, &str)] = &[
    ("Enter", "Send the message"),
    ("Shift-Enter / Alt-Enter / Ctrl-J", "New line"),
    ("Esc", "Close modal or popup; otherwise interrupt the turn"),
    (
        "Ctrl-C",
        "Copy selection, clear input, interrupt, or press twice to quit",
    ),
    (
        "Ctrl-Shift-C",
        "Copy selection without interrupting a running turn",
    ),
    (
        "Drag text",
        "Select and copy from transcript or input (Shift-drag for terminal)",
    ),
    ("Up / Down", "Move between input rows, then through history"),
    ("PageUp / PageDown", "Scroll the transcript a page"),
    ("Shift-Up / Shift-Down", "Scroll the transcript a line"),
    (
        "Ctrl-Home / Ctrl-End",
        "Jump to the top or bottom of the transcript",
    ),
    ("Ctrl-T", "Show or hide the model's thinking"),
    ("/", "Slash commands; Tab completes"),
    ("@", "Mention a file in the workspace; Tab completes"),
    ("Ctrl-P", "Command palette (/help)"),
    ("Shift-Tab", "Toggle Plan/Edit, keeping approvals"),
    ("Ctrl-Y", "Toggle Ask/Allow approvals, keeping Plan/Edit"),
    (
        "Click mode pills",
        "Toggle Plan/Edit or Ask/Allow in the status bar",
    ),
    ("Tab (in modal)", "Switch tab"),
    ("Tab / Shift-Tab (hook setup)", "Move between fields"),
    ("Ctrl-S / Ctrl-E (hook setup)", "Save / Save and enable"),
    ("Ctrl-T (hook setup)", "Check access using unsaved settings"),
    (
        "Ctrl-R / Ctrl-X (hook secret)",
        "Remove credential / choose external source",
    ),
    (
        "Ctrl-S (in modal)",
        "Secondary action, e.g. save as default in /models",
    ),
    (
        "Ctrl-L (modal / approval)",
        "Toggle save scope between user config and project .quack/",
    ),
];
