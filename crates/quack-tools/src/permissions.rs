//! Permission modes, allow/ask/deny rules and the compiled-in deny floor.
//!
//! Rules are `kind:pattern` strings: `shell:git status*` (`*` glob over the whole command),
//! `edit:src/**` and `read:docs/**` (path globs relative to the workspace root).
//! Order: work-mode restriction → deny floor → deny → ask → session/allow → approval default.
//! Yolo bypasses asks after the work-mode restriction, floor and denies.
//!
//! A shell command is split into its simple commands first (`git status && rm -rf x` is
//! `git status` and `rm -rf x`, and `$(...)` counts too). Deny and ask rules, and the deny
//! floor, apply when the whole line or any one command matches. An allow rule applies only
//! when every command is allowed (by a rule, or because it only reads, with at least one
//! rule match) and nothing is redirected to a file. A line the splitter can't read
//! (backticks, subshells, heredocs, `if`/`for`) gets no allow rule at all.

use std::path::{Component, Path, PathBuf};

use quack_config::glob_match;
use quack_config::settings::{PermissionMode, PermissionSettings, WorkMode};

use crate::paths;
use crate::registry::Subject;
use crate::shell_split::{self, Split};

/// Commands refused in every mode, even `full-auto`. Matched after collapsing whitespace.
const DENY_FLOOR: &[&str] = &[
    "rm -rf /",
    "rm -rf /*",
    "rm -rf ~",
    "rm -rf ~/*",
    "rm -rf $HOME*",
    "sudo rm -rf /*",
    ":(){ :|:& };:",
    "mkfs*",
    "* > /dev/sda*",
    "dd * of=/dev/sd*",
    "dd * of=/dev/nvme*",
    "format c:*",
    "Remove-Item -Recurse -Force C:\\*",
];

/// Commands that only look, allowed while planning. Never matched with control operators.
const READ_ONLY_COMMANDS: &[&str] = &[
    "ls",
    "dir",
    "cat",
    "type",
    "head",
    "tail",
    "wc",
    "nl",
    "tac",
    "rev",
    "cut",
    "tr",
    "fold",
    "expand",
    "unexpand",
    "column",
    "paste",
    "join",
    "comm",
    "cmp",
    "diff",
    "diff3",
    "sdiff",
    "grep",
    "egrep",
    "fgrep",
    "zgrep",
    "zegrep",
    "zfgrep",
    "bzgrep",
    "xzgrep",
    "ag",
    "ack",
    "ugrep",
    "ug",
    "rg",
    "find",
    "fd",
    "fdfind",
    "tree",
    "file",
    "stat",
    "du",
    "df",
    "free",
    "uptime",
    "pwd",
    "which",
    "where",
    "basename",
    "dirname",
    "realpath",
    "readlink",
    "pathchk",
    "uname",
    "arch",
    "nproc",
    "lscpu",
    "id",
    "whoami",
    "groups",
    "hostname",
    "date",
    "cal",
    "locale",
    "printenv",
    "zcat",
    "bzcat",
    "xzcat",
    "zless",
    "less",
    "more",
    "od",
    "hexdump",
    "xxd",
    "strings",
    "readelf",
    "objdump",
    "nm",
    "ldd",
    "otool",
    "size",
    "md5sum",
    "sha1sum",
    "sha224sum",
    "sha256sum",
    "sha384sum",
    "sha512sum",
    "b2sum",
    "cksum",
    "shasum",
    "md5",
    "echo",
    "printf",
    "cd",
    "pushd",
    "popd",
    "dirs",
    "expr",
    "seq",
    "read",
    ":",
    // Tests, which the bash parser now finds inside `if` and `while` conditions.
    "[",
    "[[",
    "test",
    "true",
    "false",
    "Get-ChildItem",
    "Get-Content",
    "Select-String",
    // PowerShell's own, matched ignoring case. Not `ForEach-Object`, which can call a
    // method on every input (`gci | % Delete`).
    "gci",
    "gc",
    "gi",
    "gl",
    "gcm",
    "sls",
    "Get-Item",
    "Get-ItemProperty",
    "Get-Location",
    "Get-Command",
    "Get-FileHash",
    "Get-Date",
    "Test-Path",
    "Resolve-Path",
    "Split-Path",
    "Join-Path",
    "Select-Object",
    "Sort-Object",
    "Where-Object",
    "Measure-Object",
    "Format-Table",
    "Format-List",
    "Out-String",
    "Write-Output",
];
const READ_ONLY_GIT: &[&str] = &[
    "status",
    "log",
    "diff",
    "show",
    "branch",
    "blame",
    "grep",
    "ls-files",
    "rev-parse",
    "ls-tree",
    "cat-file",
    "rev-list",
    "for-each-ref",
    "describe",
    "shortlog",
    "merge-base",
    "name-rev",
    "check-ignore",
    "check-attr",
    "var",
    "whatchanged",
    "count-objects",
    "verify-commit",
    "verify-tag",
    "diff-tree",
    "diff-files",
    "diff-index",
    "tag",
    "remote",
    "stash",
    "worktree",
    "reflog",
    "config",
];

/// Environment variables that alter command resolution, dynamic linking, or shell startup
/// hooks and therefore must not be overridden in read-only plan mode.
const DANGEROUS_ENV_VARS: &[&str] = &[
    "PATH",
    "CDPATH",
    "IFS",
    "LD_PRELOAD",
    "LD_LIBRARY_PATH",
    "LD_AUDIT",
    "LD_DEBUG",
    "LD_PROFILE",
    "DYLD_INSERT_LIBRARIES",
    "DYLD_LIBRARY_PATH",
    "DYLD_FRAMEWORK_PATH",
    "DYLD_FALLBACK_LIBRARY_PATH",
    "BASH_ENV",
    "ENV",
    "PROMPT_COMMAND",
    "SHELLOPTS",
    "BASHOPTS",
    "PS4",
    "FPATH",
    "GLOBIGNORE",
    "EXECIGNORE",
    "GIT_SSH",
    "GIT_SSH_COMMAND",
    "GIT_ASKPASS",
    "SSH_ASKPASS",
    "GIT_EXEC_PATH",
    "GIT_EXTERNAL_DIFF",
    "GIT_EDITOR",
    "EDITOR",
    "VISUAL",
    "FCEDIT",
    "SUDO_ASKPASS",
    "LESSOPEN",
    "LESSCLOSE",
    "PYTHONPATH",
    "PYTHONSTARTUP",
    "PYTHONHOME",
    "NODE_OPTIONS",
    "RUBYOPT",
    "RUBYLIB",
    "PERL5OPT",
    "PERL5LIB",
    "JAVA_TOOL_OPTIONS",
    "RUSTC_WRAPPER",
    "CARGO_BUILD_RUSTC_WRAPPER",
];

/// Whether a shell command line is purely read-only (inspecting files/git/etc. without writing).
pub fn is_read_only_shell(cmd: &str) -> bool {
    ShellLine::new(cmd, false).read_only()
}

/// Split a simple command into words, respecting single and double quotes and backslash
/// escapes, and dropping harmless stream/null redirection tokens (`2>/dev/null`, `2>&1`)
/// that `shell_split` already verified do not write to files.
fn shell_words(cmd: &str) -> Option<Vec<String>> {
    let mut words = Vec::new();
    let mut cur = String::new();
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                let mut closed = false;
                for ch in chars.by_ref() {
                    if ch == '\'' {
                        closed = true;
                        break;
                    }
                    cur.push(ch);
                }
                if !closed {
                    return None;
                }
            }
            '"' => {
                in_word = true;
                let mut closed = false;
                while let Some(ch) = chars.next() {
                    if ch == '"' {
                        closed = true;
                        break;
                    }
                    if ch == '\\'
                        && let Some(&next) = chars.peek()
                        && matches!(next, '"' | '\\' | '$' | '`')
                    {
                        cur.push(next);
                        chars.next();
                        continue;
                    }
                    cur.push(ch);
                }
                if !closed {
                    return None;
                }
            }
            '\\' => {
                in_word = true;
                if let Some(next) = chars.next() {
                    cur.push(next);
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut cur));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                cur.push(c);
            }
        }
    }
    if in_word {
        words.push(cur);
    }
    let mut filtered = Vec::with_capacity(words.len());
    let mut i = 0;
    while i < words.len() {
        let w = words[i].as_str();
        if is_harmless_redirect_token(w) {
            i += 1;
            continue;
        }
        // Separate redirect operator followed by `/dev/null`, `NUL`, or `$null`, e.g. `2> /dev/null`.
        if matches!(
            w,
            ">" | ">>" | "1>" | "1>>" | "2>" | "2>>" | "&>" | "&>>" | "<" | "0<"
        ) && words
            .get(i + 1)
            .is_some_and(|t| matches!(t.as_str(), "/dev/null" | "NUL" | "nul" | "$null"))
        {
            i += 2;
            continue;
        }
        filtered.push(words[i].clone());
        i += 1;
    }
    Some(filtered)
}

fn is_harmless_redirect_token(w: &str) -> bool {
    matches!(
        w,
        "2>&1"
            | "1>&2"
            | ">&2"
            | ">&1"
            | ">&-"
            | "1>&-"
            | "2>&-"
            | ">/dev/null"
            | ">>"
            | "1>/dev/null"
            | "2>/dev/null"
            | "2>>/dev/null"
            | "&>/dev/null"
            | "</dev/null"
            | "0</dev/null"
            | ">NUL"
            | "2>NUL"
            | ">$null"
            | "2>$null"
    )
}

fn is_safe_env_assignment(token: &str) -> bool {
    let Some((name, val)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_')
        || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return false;
    }
    if DANGEROUS_ENV_VARS
        .iter()
        .any(|d| d.eq_ignore_ascii_case(name))
    {
        return false;
    }
    if matches!(
        name.to_ascii_uppercase().as_str(),
        "PAGER" | "GIT_PAGER" | "MANPAGER"
    ) && !matches!(val, "" | "cat" | "less" | "more")
    {
        return false;
    }
    true
}

fn resolve_command_name(raw: &str) -> Option<&str> {
    let name = if let Some(rest) = raw
        .strip_prefix("/usr/bin/")
        .or_else(|| raw.strip_prefix("/bin/"))
        .or_else(|| raw.strip_prefix("/usr/local/bin/"))
        .or_else(|| raw.strip_prefix("/opt/homebrew/bin/"))
    {
        rest
    } else {
        raw
    };
    if name.contains(['/', '\\']) {
        return None;
    }
    Some(name.strip_suffix(".exe").unwrap_or(name))
}

fn read_only_command(cmd: &str) -> bool {
    let Some(words) = shell_words(&normalize(cmd)) else {
        return false;
    };
    read_only_tokens(&words, 0)
}

fn read_only_tokens(words: &[String], depth: usize) -> bool {
    if depth > 6 || words.is_empty() {
        return false;
    }
    let mut idx = 0;
    while idx < words.len() && words[idx].contains('=') && !words[idx].starts_with('-') {
        if !is_safe_env_assignment(&words[idx]) {
            return false;
        }
        idx += 1;
    }
    // A bare variable assignment (`dir=src` or `files=$(git ls-files)`, where any inner
    // command substitution was already extracted and checked as its own part).
    if idx == words.len() {
        return idx > 0;
    }
    let rest = &words[idx..];
    let Some(first) = resolve_command_name(&rest[0]) else {
        return false;
    };
    let args = &rest[1..];
    match first {
        "env" => read_only_env(args, depth),
        "command" => read_only_command_builtin(args, depth),
        "xargs" => read_only_xargs(args, depth),
        "git" => read_only_git(args),
        "find" => read_only_find(args, depth),
        "rg" | "ripgrep" => !args.iter().any(|w| {
            w == "--pre"
                || w.starts_with("--pre=")
                || w == "--hostname-bin"
                || w.starts_with("--hostname-bin=")
        }),
        "fd" | "fdfind" => read_only_fd(args),
        "file" => !args.iter().any(|w| {
            w == "--compile"
                || w == "-C"
                || (w.starts_with('-') && !w.starts_with("--") && w[1..].contains('C'))
        }),
        "sed" => read_only_sed(args),
        "awk" | "gawk" | "mawk" | "nawk" => read_only_awk(args),
        "sort" => read_only_sort(args),
        "uniq" => read_only_uniq(args),
        "jq" => !args.iter().any(|w| {
            matches!(w.as_str(), "-f" | "--from-file" | "-L") || w.starts_with("--from-file=")
        }),
        "yq" => !args.iter().any(|w| {
            matches!(
                w.as_str(),
                "-i" | "--inplace" | "--in-place" | "-s" | "--split-exp"
            ) || w.starts_with("--inplace=")
                || w.starts_with("--in-place=")
                || w.starts_with("--split-exp=")
                || (w.starts_with('-')
                    && !w.starts_with("--")
                    && (w[1..].contains('i') || w[1..].contains('s')))
        }),
        _ => READ_ONLY_COMMANDS
            .iter()
            .any(|c| c.eq_ignore_ascii_case(first)),
    }
}

fn read_only_env(args: &[String], depth: usize) -> bool {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if matches!(a, "-i" | "--ignore-environment" | "-0" | "--null") {
            i += 1;
        } else if a == "-u" {
            i += 2;
        } else if a.starts_with("--unset=") || a.starts_with("-u") {
            i += 1;
        } else if a.starts_with('-') {
            return false;
        } else if a.contains('=') {
            if !is_safe_env_assignment(a) {
                return false;
            }
            i += 1;
        } else {
            return read_only_tokens(&args[i..], depth + 1);
        }
    }
    true
}

fn read_only_command_builtin(args: &[String], depth: usize) -> bool {
    let mut lookup_only = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            i += 1;
            break;
        }
        if a.starts_with('-') && a.len() > 1 && a[1..].chars().all(|c| matches!(c, 'v' | 'V' | 'p'))
        {
            if a.contains('v') || a.contains('V') {
                lookup_only = true;
            }
            i += 1;
        } else {
            break;
        }
    }
    if lookup_only {
        return i < args.len();
    }
    i < args.len() && read_only_tokens(&args[i..], depth + 1)
}

fn read_only_xargs(args: &[String], depth: usize) -> bool {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            i += 1;
            break;
        }
        if matches!(
            a,
            "-0" | "--null"
                | "-r"
                | "--no-run-if-empty"
                | "-t"
                | "--verbose"
                | "-x"
                | "--exit"
                | "-p"
                | "--interactive"
                | "--show-limits"
        ) {
            i += 1;
        } else if matches!(
            a,
            "-n" | "--max-args"
                | "-L"
                | "--max-lines"
                | "-P"
                | "--max-procs"
                | "-I"
                | "-i"
                | "--replace"
                | "-d"
                | "--delimiter"
                | "-s"
                | "--max-chars"
                | "-a"
                | "--arg-file"
                | "-E"
                | "-e"
                | "--eof"
                | "-l"
        ) {
            i += 2;
        } else if a.starts_with("-n")
            || a.starts_with("-L")
            || a.starts_with("-P")
            || a.starts_with("-I")
            || a.starts_with("-d")
            || a.starts_with("-s")
            || a.starts_with("-a")
            || a.starts_with("-E")
            || a.starts_with("--max-args=")
            || a.starts_with("--max-lines=")
            || a.starts_with("--max-procs=")
            || a.starts_with("--replace=")
            || a.starts_with("--delimiter=")
            || a.starts_with("--max-chars=")
            || a.starts_with("--arg-file=")
            || a.starts_with("--eof=")
        {
            i += 1;
        } else if a.starts_with('-') {
            return false;
        } else {
            break;
        }
    }
    if i > args.len() {
        return false;
    }
    if i == args.len() {
        // `xargs` with no command defaults to `echo`.
        return true;
    }
    read_only_tokens(&args[i..], depth + 1)
}

fn is_safe_git_config_override(kv: &str) -> bool {
    let key = kv
        .split_once('=')
        .map_or(kv, |(k, _)| k)
        .trim()
        .to_ascii_lowercase();
    key.starts_with("color.")
        || key.starts_with("log.")
        || key.starts_with("status.")
        || key.starts_with("i18n.")
        || matches!(
            key.as_str(),
            "core.quotepath"
                | "core.abbrev"
                | "core.autocrlf"
                | "diff.algorithm"
                | "diff.mnemonicprefix"
                | "diff.noprefix"
                | "diff.renames"
                | "diff.colorMoved"
        )
}

fn read_only_git(args: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if matches!(
            a,
            "--no-pager"
                | "-p"
                | "--paginate"
                | "--no-replace-objects"
                | "--bare"
                | "--literal-pathspecs"
                | "--glob-pathspecs"
                | "--noglob-pathspecs"
                | "--icase-pathspecs"
                | "--no-optional-locks"
        ) || a.starts_with("--git-dir=")
            || a.starts_with("--work-tree=")
            || a.starts_with("--namespace=")
        {
            i += 1;
        } else if matches!(a, "-C" | "--git-dir" | "--work-tree" | "--namespace") {
            if i + 1 >= args.len() {
                return false;
            }
            i += 2;
        } else if a.starts_with("-C") && a.len() > 2 {
            i += 1;
        } else if a == "-c" {
            let Some(kv) = args.get(i + 1) else {
                return false;
            };
            if !is_safe_git_config_override(kv) {
                return false;
            }
            i += 2;
        } else if let Some(kv) = a.strip_prefix("-c")
            && !kv.is_empty()
        {
            if !is_safe_git_config_override(kv) {
                return false;
            }
            i += 1;
        } else if a.starts_with('-') {
            return false;
        } else {
            break;
        }
    }
    let Some(subcommand) = args
        .get(i)
        .map(String::as_str)
        .filter(|w| READ_ONLY_GIT.contains(w))
    else {
        return false;
    };
    let sub_args = &args[i + 1..];
    if sub_args.iter().any(|a| {
        a.starts_with("--output")
            || a.starts_with("--ext-diff")
            || a.starts_with("--textconv")
            || a.starts_with("--open-files-in-pager")
            || (subcommand == "grep" && a.starts_with("-O"))
    }) {
        return false;
    }
    match subcommand {
        "branch" => {
            let has_list = sub_args
                .iter()
                .any(|a| matches!(a.as_str(), "-l" | "--list"));
            sub_args.iter().all(|a| {
                matches!(
                    a.as_str(),
                    "-a" | "--all"
                        | "-r"
                        | "--remotes"
                        | "-v"
                        | "-vv"
                        | "--verbose"
                        | "-l"
                        | "--list"
                        | "--show-current"
                        | "--no-color"
                        | "--color=never"
                        | "--color=always"
                        | "--color=auto"
                        | "--abbrev"
                        | "--no-abbrev"
                        | "--contains"
                        | "--no-contains"
                        | "--merged"
                        | "--no-merged"
                        | "--points-at"
                ) || a.starts_with("--sort=")
                    || a.starts_with("--format=")
                    || a.starts_with("--color=")
                    || a.starts_with("--abbrev=")
                    || (has_list && !a.starts_with('-'))
            })
        }
        "tag" => {
            if sub_args.is_empty() {
                return true;
            }
            let mutating = sub_args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "-d" | "--delete"
                        | "-a"
                        | "--annotate"
                        | "-s"
                        | "--sign"
                        | "-u"
                        | "-f"
                        | "--force"
                        | "-m"
                        | "-F"
                )
            });
            let listing = sub_args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "-l" | "--list"
                        | "--contains"
                        | "--no-contains"
                        | "--merged"
                        | "--no-merged"
                        | "--points-at"
                        | "-v"
                        | "--verify"
                ) || a.starts_with("--sort=")
                    || a.starts_with("--format=")
            });
            !mutating && listing
        }
        "remote" => match sub_args.first().map(String::as_str) {
            None | Some("-v" | "--verbose") => sub_args.len() <= 1,
            Some("show" | "get-url") => true,
            _ => false,
        },
        "stash" => matches!(sub_args.first().map(String::as_str), Some("list" | "show")),
        "worktree" => matches!(sub_args.first().map(String::as_str), Some("list")),
        "reflog" => !sub_args
            .iter()
            .any(|a| matches!(a.as_str(), "expire" | "delete" | "exists")),
        "config" => {
            let mutating = sub_args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "--unset"
                        | "--unset-all"
                        | "--replace-all"
                        | "--add"
                        | "--remove-section"
                        | "--rename-section"
                        | "--edit"
                        | "-e"
                )
            });
            let reading = sub_args.iter().any(|a| {
                matches!(
                    a.as_str(),
                    "--get" | "--get-all" | "--get-regexp" | "--get-urlmatch" | "--list" | "-l"
                )
            });
            !mutating && reading
        }
        _ => true,
    }
}

fn read_only_find(args: &[String], depth: usize) -> bool {
    let mut i = 0;
    while i < args.len() {
        let w = args[i].as_str();
        if matches!(
            w,
            "-delete" | "-ok" | "-okdir" | "-fprint" | "-fprint0" | "-fprintf" | "-fls"
        ) {
            return false;
        }
        if matches!(w, "-exec" | "-execdir") {
            let start = i + 1;
            let Some(end_rel) = args[start..]
                .iter()
                .position(|t| matches!(t.as_str(), ";" | "\\;" | "+"))
            else {
                return false;
            };
            let inner: Vec<String> = args[start..start + end_rel]
                .iter()
                .map(|t| {
                    if t == "{}" {
                        "file".to_owned()
                    } else {
                        t.clone()
                    }
                })
                .collect();
            if inner.is_empty() || !read_only_tokens(&inner, depth + 1) {
                return false;
            }
            i = start + end_rel + 1;
            continue;
        }
        i += 1;
    }
    true
}

fn read_only_fd(args: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            break;
        }
        if matches!(a, "-x" | "--exec" | "-X" | "--exec-batch")
            || a.starts_with("--exec=")
            || a.starts_with("--exec-batch=")
        {
            return false;
        }
        if matches!(
            a,
            "-e" | "--extension"
                | "-E"
                | "--exclude"
                | "-t"
                | "--type"
                | "-d"
                | "--max-depth"
                | "--min-depth"
                | "--exact-depth"
                | "-S"
                | "--size"
                | "--changed-within"
                | "--changed-before"
                | "-o"
                | "--owner"
                | "--base-directory"
                | "--path-separator"
                | "--search-path"
                | "--color"
                | "--hyperlink"
        ) {
            i += 2;
            continue;
        }
        if a.starts_with('-')
            && !a.starts_with("--")
            && (a[1..].contains('x') || a[1..].contains('X'))
        {
            return false;
        }
        i += 1;
    }
    true
}

fn read_only_sed(args: &[String]) -> bool {
    let mut scripts: Vec<&str> = Vec::new();
    let mut saw_explicit_script = false;
    let mut after_double_dash = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if after_double_dash {
            if !saw_explicit_script && scripts.is_empty() {
                scripts.push(a);
            }
            i += 1;
            continue;
        }
        if a == "--" {
            after_double_dash = true;
            i += 1;
            continue;
        }
        if a == "-e" || a == "--expression" {
            let Some(s) = args.get(i + 1) else {
                return false;
            };
            saw_explicit_script = true;
            scripts.push(s.as_str());
            i += 2;
        } else if let Some(s) = a
            .strip_prefix("--expression=")
            .or_else(|| a.strip_prefix("-e"))
            .filter(|s| !s.is_empty() || a.starts_with("--expression="))
        {
            saw_explicit_script = true;
            scripts.push(s);
            i += 1;
        } else if a == "-l" || a == "--line-length" {
            i += 2;
        } else if a.starts_with("--line-length=")
            || matches!(
                a,
                "--quiet"
                    | "--silent"
                    | "--regexp-extended"
                    | "--null-data"
                    | "--unbuffered"
                    | "--posix"
            )
        {
            i += 1;
        } else if a.starts_with("--") {
            return false;
        } else if a.starts_with('-') && a.len() > 1 {
            // Short flag cluster: only n, E, r, z, u, s are read-only flags; i and f are forbidden.
            let mut chars = a[1..].chars();
            while let Some(c) = chars.next() {
                match c {
                    'n' | 'E' | 'r' | 'z' | 'u' | 's' => {}
                    'e' => {
                        let rest = chars.as_str();
                        saw_explicit_script = true;
                        if !rest.is_empty() {
                            scripts.push(rest);
                        } else if let Some(s) = args.get(i + 1) {
                            scripts.push(s.as_str());
                            i += 1;
                        } else {
                            return false;
                        }
                        break;
                    }
                    _ => return false,
                }
            }
            i += 1;
        } else {
            if !saw_explicit_script && scripts.is_empty() {
                scripts.push(a);
            }
            i += 1;
        }
    }
    !scripts.is_empty() && scripts.into_iter().all(is_read_only_sed_script)
}

/// Verify that a `sed` script contains only read-only addresses and commands (never `w`,
/// `W`, `e`, `r`, `R`, or `s///w` / `s///e`).
fn is_read_only_sed_script(script: &str) -> bool {
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    let skip_delimited = |chars: &[char], mut j: usize, delim: char| -> Option<usize> {
        while j < chars.len() {
            if chars[j] == '\\' {
                j += 2;
                continue;
            }
            if chars[j] == delim {
                return Some(j + 1);
            }
            j += 1;
        }
        None
    };
    while i < chars.len() {
        // Skip whitespace, separators, braces, and address tokens.
        while i < chars.len() {
            match chars[i] {
                c if c.is_whitespace()
                    || c.is_ascii_digit()
                    || matches!(c, '$' | ',' | '~' | '+' | '!' | ';' | '{' | '}') =>
                {
                    i += 1;
                }
                '/' => {
                    let Some(next_i) = skip_delimited(&chars, i + 1, '/') else {
                        return false;
                    };
                    i = next_i;
                }
                '\\' => {
                    let Some(&delim) = chars.get(i + 1) else {
                        return false;
                    };
                    let Some(next_i) = skip_delimited(&chars, i + 2, delim) else {
                        return false;
                    };
                    i = next_i;
                }
                _ => break,
            }
        }
        if i >= chars.len() {
            break;
        }
        let cmd = chars[i];
        i += 1;
        match cmd {
            's' => {
                let Some(&delim) = chars.get(i) else {
                    return false;
                };
                if delim == '\n' {
                    return false;
                }
                let Some(after_pat) = skip_delimited(&chars, i + 1, delim) else {
                    return false;
                };
                let Some(after_rep) = skip_delimited(&chars, after_pat, delim) else {
                    return false;
                };
                i = after_rep;
                while i < chars.len() && !matches!(chars[i], ';' | '\n' | '}') {
                    let f = chars[i];
                    if !f.is_whitespace()
                        && !f.is_ascii_digit()
                        && !matches!(f, 'g' | 'p' | 'i' | 'I' | 'm' | 'M')
                    {
                        return false;
                    }
                    i += 1;
                }
            }
            'y' => {
                let Some(&delim) = chars.get(i) else {
                    return false;
                };
                if delim == '\n' {
                    return false;
                }
                let Some(after_src) = skip_delimited(&chars, i + 1, delim) else {
                    return false;
                };
                let Some(after_dst) = skip_delimited(&chars, after_src, delim) else {
                    return false;
                };
                i = after_dst;
            }
            'p' | 'P' | 'd' | 'D' | '=' | 'n' | 'N' | 'h' | 'H' | 'g' | 'G' | 'x' | 'z' => {}
            'q' | 'Q' | 'l' => {
                while i < chars.len() && (chars[i].is_whitespace() && chars[i] != '\n') {
                    i += 1;
                }
                while i < chars.len() && chars[i].is_ascii_digit() {
                    i += 1;
                }
            }
            'b' | 't' | 'T' | ':' => {
                while i < chars.len() && !matches!(chars[i], ';' | '\n' | '}') {
                    if !chars[i].is_whitespace()
                        && !chars[i].is_ascii_alphanumeric()
                        && !matches!(chars[i], '_' | '-' | '.')
                    {
                        return false;
                    }
                    i += 1;
                }
            }
            _ => return false,
        }
    }
    true
}

fn read_only_awk(args: &[String]) -> bool {
    let mut script: Option<&str> = None;
    let mut after_double_dash = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if after_double_dash {
            if script.is_none() {
                script = Some(a);
            }
            i += 1;
            continue;
        }
        if a == "--" {
            after_double_dash = true;
            i += 1;
            continue;
        }
        if matches!(a, "-F" | "--field-separator" | "-v" | "--assign") {
            if i + 1 >= args.len() {
                return false;
            }
            i += 2;
        } else if a.starts_with("-F")
            || a.starts_with("-v")
            || a.starts_with("--field-separator=")
            || a.starts_with("--assign=")
            || matches!(
                a,
                "--characters-as-bytes"
                    | "-b"
                    | "--traditional"
                    | "-c"
                    | "--copyright"
                    | "-C"
                    | "--gen-pot"
                    | "-g"
                    | "--help"
                    | "-h"
                    | "--non-decimal-data"
                    | "-n"
                    | "--use-lc-numeric"
                    | "-N"
                    | "--optimize"
                    | "-O"
                    | "--posix"
                    | "-P"
                    | "--re-interval"
                    | "-r"
                    | "--no-optimize"
                    | "-s"
                    | "--sandbox"
                    | "-S"
                    | "--version"
                    | "-V"
            )
        {
            i += 1;
        } else if a.starts_with('-') {
            // Reject `-f` (external script file), `-i` (include), `-l` (load extension), `-o` (pretty-print to file), etc.
            return false;
        } else {
            if script.is_none() {
                script = Some(a);
            }
            i += 1;
        }
    }
    script.is_some_and(is_read_only_awk_script)
}

/// Verify that an inline `awk` script does not call `system(...)`, use `getline`, pipe to/from
/// commands (`|` or `|&`), or redirect `print` / `printf` output (`>` or `>>`).
fn is_read_only_awk_script(script: &str) -> bool {
    let chars: Vec<char> = script.chars().collect();
    let mut i = 0;
    let mut paren_depth = 0usize;
    let mut in_print = false;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '#' => {
                while i < chars.len() && chars[i] != '\n' {
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                while i < chars.len() {
                    if chars[i] == '\\' {
                        i += 2;
                        continue;
                    }
                    if chars[i] == '"' {
                        i += 1;
                        break;
                    }
                    i += 1;
                }
            }
            '(' => {
                paren_depth += 1;
                i += 1;
            }
            ')' => {
                paren_depth = paren_depth.saturating_sub(1);
                i += 1;
            }
            ';' | '\n' | '{' | '}' => {
                if paren_depth == 0 {
                    in_print = false;
                }
                i += 1;
            }
            '|' => {
                // `||` is logical OR; single `|` or `|&` is a command pipe.
                if chars.get(i + 1) == Some(&'|') {
                    i += 2;
                } else {
                    return false;
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'>') {
                    return false;
                }
                if chars.get(i + 1) == Some(&'=') {
                    i += 2;
                    continue;
                }
                if in_print && paren_depth == 0 {
                    return false;
                }
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_alphanumeric() || chars[i] == '_') {
                    i += 1;
                }
                let ident: String = chars[start..i].iter().collect();
                if matches!(ident.as_str(), "system" | "getline") {
                    return false;
                }
                if matches!(ident.as_str(), "print" | "printf") {
                    in_print = true;
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    true
}

fn read_only_sort(args: &[String]) -> bool {
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if a == "--" {
            break;
        }
        if matches!(
            a,
            "-o" | "--output" | "--compress-program" | "--files0-from"
        ) || a.starts_with("-o")
            || a.starts_with("--output=")
            || a.starts_with("--compress-program=")
            || a.starts_with("--files0-from=")
        {
            return false;
        }
        if matches!(
            a,
            "-k" | "--key"
                | "-t"
                | "--field-separator"
                | "-S"
                | "--buffer-size"
                | "-T"
                | "--temporary-directory"
        ) {
            i += 2;
            continue;
        }
        if a.starts_with("-k") || a.starts_with("-t") || a.starts_with("-S") || a.starts_with("-T")
        {
            i += 1;
            continue;
        }
        if a.starts_with('-') && !a.starts_with("--") && a[1..].contains('o') {
            return false;
        }
        i += 1;
    }
    true
}

fn read_only_uniq(args: &[String]) -> bool {
    let mut positional = 0usize;
    let mut after_double_dash = false;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        if after_double_dash {
            positional += 1;
            i += 1;
            continue;
        }
        if a == "--" {
            after_double_dash = true;
            i += 1;
            continue;
        }
        if matches!(
            a,
            "-f" | "--skip-fields" | "-s" | "--skip-chars" | "-w" | "--check-chars"
        ) {
            i += 2;
            continue;
        }
        if a.starts_with('-') {
            i += 1;
            continue;
        }
        positional += 1;
        i += 1;
    }
    // `uniq [OPTION]... [INPUT [OUTPUT]]`: a 2nd positional argument writes to `OUTPUT`.
    positional <= 1
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Ask,
    Deny(String),
}

#[derive(Debug, Clone)]
enum Rule {
    Shell(String),
    Edit(globset::GlobMatcher),
    Read(globset::GlobMatcher),
    /// `mcp:<server>/<tool>` globs, e.g. `mcp:github/get_*`.
    Mcp(String),
}

impl Rule {
    fn parse(s: &str) -> Result<Self, String> {
        let (kind, pattern) = s.split_once(':').ok_or_else(|| {
            format!("rule `{s}` needs a kind prefix: shell:, edit:, read: or mcp:")
        })?;
        let glob = |p: &str| {
            globset::GlobBuilder::new(p.trim())
                .literal_separator(true)
                .case_insensitive(cfg!(any(windows, target_os = "macos")))
                .build()
                .map(|g| g.compile_matcher())
                .map_err(|e| format!("rule `{s}`: {e}"))
        };
        match kind.trim() {
            "shell" | "bash" => Ok(Self::Shell(normalize(pattern))),
            "edit" | "write" => Ok(Self::Edit(glob(pattern)?)),
            "read" => Ok(Self::Read(glob(pattern)?)),
            "mcp" => Ok(Self::Mcp(pattern.trim().to_owned())),
            other => Err(format!("rule `{s}`: unknown kind `{other}`")),
        }
    }

    fn matches(&self, subject: &Subject, root: &Path) -> bool {
        let rel = |p: &Path| p.strip_prefix(root).unwrap_or(p).to_path_buf();
        match (self, subject) {
            (Self::Shell(pat), Subject::Shell(cmd)) => glob_match(pat, &normalize(cmd)),
            (Self::Edit(g), Subject::Edit(p)) => g.is_match(rel(p)),
            (Self::Read(g), Subject::Read(p)) => g.is_match(rel(p)),
            (Self::Mcp(pat), Subject::Mcp { server, tool, .. }) => {
                glob_match(pat, &format!("{server}/{tool}"))
            }
            _ => false,
        }
    }
}

fn normalize(cmd: &str) -> String {
    cmd.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// PowerShell names for the same command, for deny and ask rules: `shell:rm *` also
/// stops `Remove-Item x` and `del x`, and `shell:curl *` stops `iwr`.
const PS_ALIASES: &[&[&str]] = &[
    &["remove-item", "rm", "del", "erase", "rd", "ri", "rmdir"],
    &["get-childitem", "ls", "dir", "gci"],
    &["get-content", "cat", "type", "gc"],
    &["set-content", "sc"],
    &["add-content", "ac"],
    &["copy-item", "cp", "copy", "cpi"],
    &["move-item", "mv", "move", "mi"],
    &["rename-item", "ren", "rni"],
    &["new-item", "ni"],
    &["invoke-webrequest", "iwr", "curl", "wget"],
    &["invoke-restmethod", "irm"],
    &["invoke-expression", "iex"],
    &["invoke-command", "icm"],
    &["invoke-item", "ii"],
    &["start-process", "start", "saps"],
    &["stop-process", "kill", "spps"],
    &["set-location", "cd", "chdir", "sl"],
    &["write-output", "echo", "write"],
];

/// A lowercased PowerShell command and the same command under each of its aliases.
fn ps_variants(cmd: &str) -> Vec<String> {
    let lower = cmd.to_lowercase();
    let (first, rest) = lower.split_once(' ').unwrap_or((lower.as_str(), ""));
    let program = first.rsplit(['/', '\\']).next().unwrap_or(first);
    let program = program.strip_suffix(".exe").unwrap_or(program);
    let mut out = vec![lower.clone()];
    if let Some(group) = PS_ALIASES.iter().find(|g| g.contains(&program)) {
        for name in group.iter().filter(|n| **n != first) {
            out.push(if rest.is_empty() {
                (*name).to_owned()
            } else {
                format!("{name} {rest}")
            });
        }
    }
    out
}

/// A shell command line as the rules see it.
struct ShellLine {
    whole: String,
    split: Option<Split>,
    /// Run by PowerShell: names match ignoring case, and deny and ask rules see aliases.
    powershell: bool,
}

impl ShellLine {
    fn new(cmd: &str, powershell: bool) -> Self {
        Self {
            whole: normalize(cmd),
            split: shell_split::split_for(cmd, powershell),
            powershell,
        }
    }

    /// The whole line and each simple command in it.
    fn pieces(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.whole.as_str()).chain(
            self.split
                .iter()
                .flat_map(|s| s.parts.iter().map(String::as_str)),
        )
    }

    /// A shell pattern matches one command; for PowerShell, ignoring case and, with
    /// `aliases`, under any of the command's names.
    fn matches(&self, pattern: &str, piece: &str, aliases: bool) -> bool {
        if !self.powershell {
            return glob_match(pattern, piece);
        }
        let pattern = pattern.to_lowercase();
        if aliases {
            ps_variants(piece).iter().any(|v| glob_match(&pattern, v))
        } else {
            glob_match(&pattern, &piece.to_lowercase())
        }
    }

    fn rule_matches(&self, rule: &Rule, piece: &str, aliases: bool) -> bool {
        match rule {
            Rule::Shell(pattern) => self.matches(pattern, piece, aliases),
            _ => false,
        }
    }

    /// The whole line or one of its commands is on the built-in deny list.
    fn on_floor(&self) -> bool {
        self.pieces()
            .any(|p| DENY_FLOOR.iter().any(|f| self.matches(f, p, true)))
    }

    /// Some rule matches the whole line or one of its commands (deny and ask rules).
    fn hit(&self, rules: &[Rule]) -> bool {
        self.pieces()
            .any(|p| rules.iter().any(|r| self.rule_matches(r, p, true)))
    }

    /// Every command is allowed by a rule or only reads, at least one by a rule, and
    /// nothing is written through a redirection.
    fn allowed(&self, rules: &[Rule]) -> bool {
        let Some(split) = self.split.as_ref().filter(|s| !s.writes) else {
            return false;
        };
        let mut matched = false;
        for p in &split.parts {
            if rules.iter().any(|r| self.rule_matches(r, p, false)) {
                matched = true;
            } else if !read_only_command(p) {
                return false;
            }
        }
        matched
    }

    /// Every command only reads and nothing is redirected to a file.
    fn read_only(&self) -> bool {
        self.read_only_violation().is_none()
    }

    /// Why this shell line cannot run in plan mode, if any part writes or is not a
    /// recognized read-only inspection command.
    fn read_only_violation(&self) -> Option<String> {
        let Some(split) = &self.split else {
            return Some("opaque or unparseable shell syntax".into());
        };
        if split.writes {
            return Some("file output redirection (`>` / `>>`)".into());
        }
        for p in &split.parts {
            if !read_only_command(p) {
                return Some(format!(
                    "command `{p}` is not a recognized read-only inspection command"
                ));
            }
        }
        None
    }
}

#[derive(Debug, Clone)]
pub struct Permissions {
    pub mode: PermissionMode,
    root: PathBuf,
    allow: Vec<Rule>,
    ask: Vec<Rule>,
    deny: Vec<Rule>,
    session: Vec<Subject>,
    /// `[memory] writes = "ask"`.
    pub memory_writes_ask: bool,
    /// Plan mode: look but don't change anything (docs/plan/04-agent-loop-and-tools.md#plan-mode).
    pub work_mode: WorkMode,
    /// `/sandbox off`: shell commands run without the OS sandbox this session.
    pub sandbox_off: bool,
    /// Commands run in PowerShell (`pwsh`, `powershell`), so they are read as PowerShell.
    pub powershell: bool,
}

impl Permissions {
    /// Build from settings. Invalid rules are returned as warnings and skipped.
    pub fn new(settings: &PermissionSettings, root: &Path) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut parse = |list: &[String]| {
            list.iter()
                .filter_map(|r| Rule::parse(r).map_err(|e| warnings.push(e)).ok())
                .collect::<Vec<_>>()
        };
        let perms = Self {
            mode: settings.mode,
            root: root.to_path_buf(),
            allow: parse(&settings.allow),
            ask: parse(&settings.ask),
            deny: parse(&settings.deny),
            session: Vec::new(),
            memory_writes_ask: false,
            work_mode: settings.work_mode,
            sandbox_off: false,
            powershell: false,
        };
        (perms, warnings)
    }

    /// Remember an "allow for this session" answer.
    pub fn allow_session(&mut self, subject: Subject) {
        match subject {
            Subject::Edits(paths) => self.session.extend(paths.into_iter().map(Subject::Edit)),
            other => self.session.push(other),
        }
    }

    /// The allow rules that "always allow" saves for `subject`: exact, so they allow this
    /// call and nothing broader. Empty when no rule can allow it (a line the splitter
    /// can't read, a redirection, a command outside the sandbox, a name with a wildcard).
    pub fn rules_for(&self, subject: &Subject) -> Vec<String> {
        let path = |kind: &str, p: &Path| {
            // Relative to the workspace when inside it, always with `/`.
            let rel = p.strip_prefix(&self.root).unwrap_or(p);
            let mut text = String::new();
            for c in rel.components() {
                match c {
                    Component::Prefix(pre) => text.push_str(&pre.as_os_str().to_string_lossy()),
                    Component::RootDir => text.push('/'),
                    other => {
                        if !text.is_empty() && !text.ends_with('/') {
                            text.push('/');
                        }
                        text.push_str(&other.as_os_str().to_string_lossy());
                    }
                }
            }
            let plain = !text.is_empty() && !text.contains(['*', '?', '[', ']', '{', '}', '\\']);
            plain.then(|| format!("{kind}:{text}"))
        };
        match subject {
            Subject::Shell(cmd) => {
                let Some(split) =
                    shell_split::split_for(cmd, self.powershell).filter(|s| !s.writes)
                else {
                    return Vec::new();
                };
                let parts: Vec<String> = split
                    .parts
                    .iter()
                    .map(|p| normalize(p))
                    .filter(|p| !read_only_command(p))
                    .collect();
                if parts.is_empty() {
                    // Only reading commands: one rule for the first lets the line through.
                    return split
                        .parts
                        .first()
                        .map(|p| normalize(p))
                        .filter(|p| !p.contains('*'))
                        .map(|p| vec![format!("shell:{p}")])
                        .unwrap_or_default();
                }
                if parts.iter().any(|p| p.contains('*')) {
                    return Vec::new();
                }
                let mut out: Vec<String> = Vec::new();
                for p in parts {
                    let rule = format!("shell:{p}");
                    if !out.contains(&rule) {
                        out.push(rule);
                    }
                }
                out
            }
            Subject::Edit(p) => path("edit", p).into_iter().collect(),
            Subject::Edits(ps) => {
                let rules: Vec<_> = ps.iter().map(|p| path("edit", p)).collect();
                if rules.iter().any(Option::is_none) {
                    return Vec::new();
                }
                rules.into_iter().flatten().collect()
            }
            Subject::Read(p) => path("read", p).into_iter().collect(),
            Subject::Mcp { server, tool, .. } if !format!("{server}{tool}").contains('*') => {
                vec![format!("mcp:{server}/{tool}")]
            }
            _ => Vec::new(),
        }
    }

    pub fn check(&self, subject: &Subject) -> Decision {
        self.check_with_tools(subject, None)
    }

    /// Like [`check`](Self::check), but when `read_only_tools` is provided, plan-mode shell
    /// denial messages list only the read-only tools actually exposed by the active profile.
    pub fn check_with_tools(
        &self,
        subject: &Subject,
        read_only_tools: Option<&[&str]>,
    ) -> Decision {
        // Several files: the strictest answer for any of them.
        if let Subject::Edits(paths) = subject {
            let mut out = Decision::Allow;
            for p in paths {
                match self.check_with_tools(&Subject::Edit(p.clone()), read_only_tools) {
                    deny @ Decision::Deny(_) => return deny,
                    Decision::Ask => out = Decision::Ask,
                    Decision::Allow => {}
                }
            }
            return out;
        }
        let shell = match subject {
            Subject::Shell(cmd) | Subject::ShellOutsideSandbox(cmd) => {
                Some(ShellLine::new(cmd, self.powershell))
            }
            _ => None,
        };
        // Work-mode restrictions run before rules, remembered approvals and yolo.
        if self.work_mode == WorkMode::Plan {
            let changes = match subject {
                Subject::Edit(_)
                | Subject::Memory { write: true }
                | Subject::Agent { read_only: false }
                | Subject::Mcp {
                    read_only: false, ..
                } => true,
                Subject::Shell(_) | Subject::ShellOutsideSandbox(_) => {
                    shell.as_ref().is_some_and(|line| !line.read_only())
                }
                _ => false,
            };
            if changes {
                let reason = if matches!(subject, Subject::Agent { .. }) {
                    "plan mode runs only read-only agents; accept the plan to use this one"
                        .to_owned()
                } else if let Some(why) = shell.as_ref().and_then(ShellLine::read_only_violation) {
                    let tools = match read_only_tools {
                        Some(list) if !list.is_empty() => {
                            format!("the dedicated read-only tools ({})", list.join(", "))
                        }
                        Some(_) => "read-only inspection".to_owned(),
                        None => {
                            "the dedicated read-only tools (read, grep, glob, list_dir, index, lsp)"
                                .to_owned()
                        }
                    };
                    format!(
                        "plan mode: no changes or test/build commands ({why}); prefer {tools} or read-only shell inspection commands (rg, grep, find, sed -n, awk, head, tail, cat, wc, sort, uniq, cut, jq, diff, git status/diff/log/show), and list any proposed test/build commands for the user to accept before running them"
                    )
                } else {
                    "plan mode: no changes or test/build commands; list the proposed tests for the user and accept the plan before running them"
                        .to_owned()
                };
                return Decision::Deny(reason);
            }
        }
        // Memory is not part of the workspace: reads are free, and writes ask only when
        // configured to (they show in the transcript and are easy to undo).
        // Installed skills are instructions the user put there.
        if matches!(
            subject,
            Subject::Agent { .. } | Subject::Skill(_) | Subject::ToolSearch | Subject::Plan
        ) {
            return Decision::Allow;
        }
        if let Subject::Memory { write } = subject {
            let ask = *write
                && self.memory_writes_ask
                && self.mode < PermissionMode::Allow
                && !self.session.contains(subject);
            return if ask { Decision::Ask } else { Decision::Allow };
        }
        if let Some(line) = &shell
            && line.on_floor()
        {
            return Decision::Deny("blocked by the built-in safety list".into());
        }
        let hit = |rules: &[Rule]| match &shell {
            Some(line) => line.hit(rules),
            None => rules.iter().any(|r| r.matches(subject, &self.root)),
        };
        let allowed = || match &shell {
            Some(line) => line.allowed(&self.allow),
            None => self.allow.iter().any(|r| r.matches(subject, &self.root)),
        };
        if hit(&self.deny) {
            return Decision::Deny("denied by a [permissions] deny rule".into());
        }
        // Yolo: nothing asks. Only the floor and deny rules above (and plan mode) stop a call.
        if self.mode == PermissionMode::Yolo {
            return Decision::Allow;
        }
        // Leaving the sandbox is the user's call in every other mode, allow rules included.
        if let (Subject::ShellOutsideSandbox(_), Some(line)) = (subject, &shell) {
            if self.mode == PermissionMode::ReadOnly && !line.read_only() {
                return Decision::Deny(
                    "only read-only commands run outside the sandbox in this mode".into(),
                );
            }
            return if self.session.contains(subject) {
                Decision::Allow
            } else {
                Decision::Ask
            };
        }
        // Only tools the server marks read-only run while planning or in read-only mode.
        if let Subject::Mcp {
            read_only: false, ..
        } = subject
            && self.mode == PermissionMode::ReadOnly
        {
            return Decision::Deny("read-only mode: this MCP tool may change things".into());
        }
        if self.mode == PermissionMode::ReadOnly && matches!(subject, Subject::Edit(_)) {
            return Decision::Deny("read-only mode: file changes are not allowed".into());
        }
        if hit(&self.ask) {
            return Decision::Ask;
        }
        if self.session.contains(subject) || allowed() {
            return Decision::Allow;
        }
        if self.work_mode == WorkMode::Plan && matches!(subject, Subject::Shell(_)) {
            return Decision::Allow;
        }
        let inside = |p: &Path| paths::inside(&self.root, p);
        match (self.mode, subject) {
            (PermissionMode::Allow, _) => Decision::Allow,
            (_, Subject::Read(p)) if inside(p) => Decision::Allow,
            (PermissionMode::ReadOnly, Subject::Shell(_)) => Decision::Deny(
                "read-only mode: only commands matching an allow rule can run".into(),
            ),
            (PermissionMode::AutoEdit, Subject::Edit(p)) if inside(p) => Decision::Allow,
            (
                _,
                Subject::Mcp {
                    auto_approve: true, ..
                },
            ) => Decision::Allow,
            _ => Decision::Ask,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn perms(mode: PermissionMode, allow: &[&str], deny: &[&str]) -> Permissions {
        let settings = PermissionSettings {
            mode,
            allow: allow.iter().map(|s| (*s).to_owned()).collect(),
            ask: Vec::new(),
            deny: deny.iter().map(|s| (*s).to_owned()).collect(),
            ..Default::default()
        };
        let (p, warnings) = Permissions::new(&settings, Path::new("/repo"));
        assert!(warnings.is_empty(), "{warnings:?}");
        p
    }

    fn shell(c: &str) -> Subject {
        Subject::Shell(c.into())
    }

    fn mcp(tool: &str, read_only: bool, auto_approve: bool) -> Subject {
        Subject::Mcp {
            server: "gh".into(),
            tool: tool.into(),
            read_only,
            auto_approve,
        }
    }

    #[test]
    fn mcp_tools_follow_rules_modes_and_auto_approve() {
        let p = perms(PermissionMode::Ask, &["mcp:gh/get_*"], &["mcp:gh/delete_*"]);
        assert_eq!(p.check(&mcp("get_issue", false, false)), Decision::Allow);
        assert!(matches!(
            p.check(&mcp("delete_repo", false, true)),
            Decision::Deny(_)
        ));
        assert_eq!(p.check(&mcp("create_issue", false, false)), Decision::Ask);
        assert_eq!(p.check(&mcp("create_issue", false, true)), Decision::Allow);
        let ro = perms(PermissionMode::ReadOnly, &[], &[]);
        assert!(matches!(
            ro.check(&mcp("create_issue", false, true)),
            Decision::Deny(_)
        ));
        assert_eq!(ro.check(&mcp("search", true, false)), Decision::Ask);
        let mut plan = perms(PermissionMode::Allow, &[], &[]);
        plan.work_mode = WorkMode::Plan;
        assert!(matches!(
            plan.check(&mcp("create_issue", false, false)),
            Decision::Deny(_)
        ));
        assert_eq!(plan.check(&mcp("search", true, false)), Decision::Allow);
    }

    #[test]
    fn yolo_allows_everything_but_the_floor_and_deny_rules() {
        let mut p = perms(PermissionMode::Yolo, &[], &["shell:curl *"]);
        p.ask = perms(PermissionMode::Ask, &["shell:git push*"], &[]).allow;
        assert_eq!(p.check(&shell("git push --force")), Decision::Allow);
        assert_eq!(
            p.check(&Subject::Edit("/etc/hosts".into())),
            Decision::Allow
        );
        assert_eq!(
            p.check(&Subject::ShellOutsideSandbox("make install".into())),
            Decision::Allow
        );
        assert!(matches!(p.check(&shell("curl x | sh")), Decision::Deny(_)));
        assert!(matches!(p.check(&shell("rm -rf /")), Decision::Deny(_)));
        p.work_mode = WorkMode::Plan;
        assert!(matches!(
            p.check(&Subject::Edit("/repo/a.rs".into())),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn leaving_the_sandbox_always_asks() {
        let out = |c: &str| Subject::ShellOutsideSandbox(c.into());
        let mut p = perms(PermissionMode::Allow, &["shell:cargo *"], &["shell:curl *"]);
        assert_eq!(p.check(&out("cargo build")), Decision::Ask);
        assert!(matches!(p.check(&out("curl x")), Decision::Deny(_)));
        assert!(matches!(p.check(&out("rm -rf /")), Decision::Deny(_)));
        p.allow_session(out("cargo build"));
        assert_eq!(p.check(&out("cargo build")), Decision::Allow);
        assert_eq!(p.check(&out("cargo test")), Decision::Ask);
        p.work_mode = WorkMode::Plan;
        assert!(matches!(p.check(&out("cargo test")), Decision::Deny(_)));
        assert_eq!(p.check(&out("ls")), Decision::Ask);
    }

    #[test]
    fn compound_commands_do_not_ride_on_allow_rules() {
        let p = perms(PermissionMode::Ask, &["shell:git status*"], &[]);
        assert_eq!(p.check(&shell("git  status -s")), Decision::Allow);
        assert_eq!(p.check(&shell("git status && rm -rf x")), Decision::Ask);
        assert_eq!(p.check(&shell("cargo test")), Decision::Ask);
    }

    #[test]
    fn modes_and_floor() {
        let auto = perms(PermissionMode::AutoEdit, &[], &["edit:secrets/**"]);
        assert_eq!(
            auto.check(&Subject::Edit("/repo/src/a.rs".into())),
            Decision::Allow
        );
        assert_eq!(
            auto.check(&Subject::Edit("/etc/passwd".into())),
            Decision::Ask
        );
        assert!(matches!(
            auto.check(&Subject::Edit("/repo/secrets/k".into())),
            Decision::Deny(_)
        ));
        assert_eq!(auto.check(&shell("ls")), Decision::Ask);

        let full = perms(PermissionMode::Allow, &[], &[]);
        assert_eq!(full.check(&shell("cargo build")), Decision::Allow);
        assert!(matches!(full.check(&shell("rm  -rf /")), Decision::Deny(_)));

        let ro = perms(PermissionMode::ReadOnly, &["shell:git log*"], &[]);
        assert!(matches!(
            ro.check(&Subject::Edit("/repo/a".into())),
            Decision::Deny(_)
        ));
        assert_eq!(ro.check(&shell("git log -3")), Decision::Allow);
        assert!(matches!(ro.check(&shell("touch a")), Decision::Deny(_)));
        assert_eq!(ro.check(&Subject::Read("/repo/a".into())), Decision::Allow);
    }

    #[test]
    fn compound_commands_are_checked_per_command() {
        let p = perms(
            PermissionMode::Ask,
            &["shell:git status*", "shell:cargo test*", "shell:echo *"],
            &[],
        );
        assert_eq!(p.check(&shell("git status && rm -rf x")), Decision::Ask);
        assert_eq!(p.check(&shell("git status && cargo test")), Decision::Allow);
        // Commands that only read may join an allowed one, but not stand alone.
        assert_eq!(
            p.check(&shell("cargo test 2>&1 | tail -50")),
            Decision::Allow
        );
        assert_eq!(p.check(&shell("cat a | head")), Decision::Ask);
        // Operators inside quotes are text.
        assert_eq!(p.check(&shell("echo 'a && rm b'")), Decision::Allow);
        // Substitutions run too; redirections write; unreadable lines get no allow.
        assert_eq!(p.check(&shell("echo $(rm x)")), Decision::Ask);
        assert_eq!(p.check(&shell("echo hi > notes.txt")), Decision::Ask);
        assert_eq!(p.check(&shell("echo `rm x`")), Decision::Ask);

        // Deny rules and the floor see every command, even in full-auto.
        let full = perms(PermissionMode::Allow, &[], &["shell:rm *"]);
        assert!(matches!(
            full.check(&shell("git status && rm -rf x")),
            Decision::Deny(_)
        ));
        assert!(matches!(
            full.check(&shell("echo \"$(rm x)\"")),
            Decision::Deny(_)
        ));
        let full = perms(PermissionMode::Allow, &[], &[]);
        assert!(matches!(
            full.check(&shell("cd /tmp; rm -rf /")),
            Decision::Deny(_)
        ));
        assert_eq!(
            full.check(&shell("cargo build && cargo test")),
            Decision::Allow
        );
    }

    #[test]
    fn session_approvals() {
        let mut p = perms(PermissionMode::Ask, &[], &[]);
        p.allow_session(shell("make"));
        assert_eq!(p.check(&shell("make")), Decision::Allow);
        assert_eq!(p.check(&shell("make install")), Decision::Ask);
    }

    #[test]
    fn always_rules_allow_exactly_the_call() {
        let p = perms(PermissionMode::Ask, &[], &[]);
        let rules = |s: &Subject| p.rules_for(s);
        assert_eq!(rules(&shell("cargo  test -q")), ["shell:cargo test -q"]);
        assert_eq!(
            rules(&shell("cargo fmt && cargo test | tail -5")),
            ["shell:cargo fmt", "shell:cargo test"]
        );
        assert_eq!(rules(&shell("cat a | head")), ["shell:cat a"]);
        assert!(rules(&shell("echo hi > notes.txt")).is_empty());
        assert!(rules(&shell("rm *.o")).is_empty());
        assert!(rules(&Subject::ShellOutsideSandbox("make".into())).is_empty());
        assert_eq!(
            rules(&Subject::Edit("/repo/src/a.rs".into())),
            ["edit:src/a.rs"]
        );
        assert!(rules(&Subject::Edit("/repo/src/[id].tsx".into())).is_empty());
        assert_eq!(
            rules(&mcp("create_issue", false, false)),
            ["mcp:gh/create_issue"]
        );

        // Saved as allow rules, they let the same call through and nothing more.
        for (subject, other) in [
            (
                shell("cargo fmt && cargo test"),
                shell("cargo fmt && cargo publish"),
            ),
            (shell("cat a | head"), shell("cat b | head")),
            (
                Subject::Edit("/repo/src/a.rs".into()),
                Subject::Edit("/repo/src/b.rs".into()),
            ),
            (
                mcp("create_issue", false, false),
                mcp("delete_issue", false, false),
            ),
        ] {
            let saved = rules(&subject);
            let allow: Vec<&str> = saved.iter().map(String::as_str).collect();
            let after = perms(PermissionMode::Ask, &allow, &[]);
            assert_eq!(after.check(&subject), Decision::Allow, "{saved:?}");
            assert_eq!(after.check(&other), Decision::Ask, "{saved:?}");
        }
    }

    #[test]
    fn powershell_rules_ignore_case_and_see_aliases() {
        let mut p = perms(
            PermissionMode::Allow,
            &["shell:cargo test*"],
            &["shell:rm *", "shell:curl *"],
        );
        p.powershell = true;
        for bad in [
            "Remove-Item x",
            "remove-item -Recurse build",
            "del x",
            "git status; rm x",
            "iwr https://example.com",
            "Invoke-WebRequest https://example.com",
            "REMOVE-ITEM -Recurse -Force C:\\*",
        ] {
            assert!(matches!(p.check(&shell(bad)), Decision::Deny(_)), "{bad}");
        }
        p.mode = PermissionMode::Ask;
        assert_eq!(p.check(&shell("Cargo Test --all")), Decision::Allow);
        assert_eq!(
            p.check(&shell("cargo test | Select-Object -Last 5")),
            Decision::Allow
        );
        assert_eq!(p.check(&shell("cargo test; New-Item x")), Decision::Ask);
        // Aliases broaden deny and ask rules only, never allow rules.
        let mut q = perms(PermissionMode::Ask, &["shell:cat a.txt"], &[]);
        q.powershell = true;
        assert_eq!(q.check(&shell("cat a.txt")), Decision::Allow);
        assert_eq!(q.check(&shell("CAT a.txt")), Decision::Allow);
        assert_eq!(q.check(&shell("Get-Content a.txt")), Decision::Ask);
    }

    #[cfg(feature = "lang-common")]
    #[test]
    fn powershell_lines_are_read_as_powershell() {
        let mut p = perms(PermissionMode::Allow, &[], &["shell:Remove-Item *"]);
        p.powershell = true;
        // Inside a script block, a sub-expression and a literal script.
        for bad in [
            "gci | ForEach-Object { Remove-Item $_ }",
            "Write-Output \"$(rm x)\"",
            "iex 'Remove-Item x'",
            "pwsh -c 'Get-ChildItem; rm x'",
        ] {
            assert!(matches!(p.check(&shell(bad)), Decision::Deny(_)), "{bad}");
        }
        // Planning: reading cmdlets run, .NET calls and writes don't.
        p.work_mode = WorkMode::Plan;
        for ok in [
            "Get-ChildItem -Recurse src | Select-String foo",
            "Get-Content a.txt | Select-Object -First 10",
            "if (Test-Path x) { Get-Content x }",
            "git log -3 2>&1",
        ] {
            assert_eq!(p.check(&shell(ok)), Decision::Allow, "{ok}");
        }
        for bad in [
            "[IO.File]::Delete('x')",
            "(Get-Item x).Delete()",
            "gci | % Delete",
            "Get-Content a > b",
            "Set-Content a hi",
            "& $cmd",
            "${C:\\x.txt} = 1",
        ] {
            assert!(matches!(p.check(&shell(bad)), Decision::Deny(_)), "{bad}");
        }
    }

    #[test]
    fn plan_mode_only_looks() {
        let (mut p, _) = Permissions::new(
            &PermissionSettings {
                mode: PermissionMode::Allow,
                allow: vec!["shell:cargo check*".into()],
                ..Default::default()
            },
            Path::new("/repo"),
        );
        p.work_mode = WorkMode::Plan;
        assert!(matches!(
            p.check(&Subject::Edit("/repo/a".into())),
            Decision::Deny(_)
        ));
        assert_eq!(p.check(&shell("git status")), Decision::Allow);
        assert_eq!(p.check(&shell("rg foo src")), Decision::Allow);
        assert!(matches!(
            p.check(&shell("cargo check -q")),
            Decision::Deny(_)
        ));
        assert_eq!(p.check(&shell("rg foo | head -5")), Decision::Allow);
        assert_eq!(p.check(&shell("git log 2>/dev/null")), Decision::Allow);
        for bad in [
            "git log | xargs rm",
            "ls $(touch x)",
            "git push",
            "find . -delete",
            "ls; rm x",
            "cat a > b",
            "npm install",
        ] {
            assert!(matches!(p.check(&shell(bad)), Decision::Deny(_)), "{bad}");
        }
        assert_eq!(p.check(&Subject::Read("/repo/a".into())), Decision::Allow);
    }

    #[test]
    fn work_mode_and_approval_matrix() {
        for (work_mode, policy, edit, mcp_read) in [
            (
                WorkMode::Plan,
                PermissionMode::Ask,
                Decision::Deny(String::new()),
                Decision::Ask,
            ),
            (
                WorkMode::Plan,
                PermissionMode::Allow,
                Decision::Deny(String::new()),
                Decision::Allow,
            ),
            (
                WorkMode::Edit,
                PermissionMode::Ask,
                Decision::Ask,
                Decision::Ask,
            ),
            (
                WorkMode::Edit,
                PermissionMode::Allow,
                Decision::Allow,
                Decision::Allow,
            ),
        ] {
            let mut p = perms(policy, &[], &[]);
            p.work_mode = work_mode;
            let actual = p.check(&Subject::Edit("/repo/a".into()));
            assert_eq!(
                std::mem::discriminant(&actual),
                std::mem::discriminant(&edit),
                "{work_mode:?}/{policy:?}: {actual:?}"
            );
            assert_eq!(
                p.check(&mcp("search", true, false)),
                mcp_read,
                "{work_mode:?}/{policy:?}"
            );
        }
    }

    #[test]
    fn planning_rules_and_grants_cannot_authorize_changes_or_tests() {
        for policy in [
            PermissionMode::Ask,
            PermissionMode::Allow,
            PermissionMode::Yolo,
        ] {
            let mut p = perms(policy, &["shell:*", "edit:**", "mcp:*"], &[]);
            for subject in [
                shell("cargo test"),
                shell("cargo check"),
                shell("npm test"),
                shell("python check.py"),
                shell("git branch new-branch"),
                shell("git log --output=log.txt"),
                shell("find . -fprintf result %p"),
                shell("rg --pre=script.py pattern"),
                shell("file -C"),
                Subject::ShellOutsideSandbox("cargo test".into()),
                Subject::Edit("/repo/a".into()),
                Subject::Memory { write: true },
                Subject::Agent { read_only: false },
                mcp("create_issue", false, true),
            ] {
                p.allow_session(subject.clone());
                p.work_mode = WorkMode::Plan;
                assert!(
                    matches!(p.check(&subject), Decision::Deny(_)),
                    "{policy:?}: {subject:?}"
                );
            }
        }
    }

    #[test]
    fn explicit_asks_and_denies_still_apply_to_plan_allow() {
        let mut p = perms(
            PermissionMode::Allow,
            &["shell:git status*"],
            &["mcp:gh/private*"],
        );
        p.work_mode = WorkMode::Plan;
        p.ask = vec![Rule::parse("shell:git status*").expect("rule")];
        p.allow_session(shell("git status"));
        assert_eq!(p.check(&shell("git status")), Decision::Ask);
        assert!(matches!(
            p.check(&mcp("private_search", true, true)),
            Decision::Deny(_)
        ));
    }

    #[test]
    fn plan_mode_allows_common_read_only_shell_pipelines_and_sanity_checks_arguments() {
        let mut p = perms(PermissionMode::Allow, &[], &[]);
        p.work_mode = WorkMode::Plan;

        for ok in [
            "grep -rn \"fn main\" src/ | cut -d: -f1 | sort -u | uniq -c",
            "sed -n '10,40p' src/main.rs",
            "sed -n '1,20p; 50,60p' src/main.rs",
            "sed -n '/fn main/,/^}/p' src/main.rs",
            "sed 's/foo/bar/g' src/main.rs",
            "nl -ba src/main.rs | sed -n '10,30p'",
            "awk -F: '{print $1, $2}' src/main.rs",
            "awk 'NR >= 10 && NR <= 30 { printf \"%4d: %s\\n\", NR, $0 }' src/main.rs",
            "find . -name '*.rs' | xargs grep -n 'PermissionMode'",
            "find . -name '*.rs' -print0 | xargs -0 -n 20 rg -n foo",
            "find . -name '*.rs' -exec grep -Hn 'WorkMode' {} +",
            "git -C crates/quack-tui --no-pager log -n 5 --oneline",
            "git -c color.ui=false diff --stat",
            "git branch --show-current 2>/dev/null",
            "git branch -l 'feat/*'",
            "git tag -l 'v*'",
            "git remote -v",
            "git stash list",
            "git worktree list",
            "git config --get user.name",
            "cd crates/quack-tools && rg -n read_only src",
            "LC_ALL=C grep -rn foo src || echo 'not found'",
            "command -v rg >/dev/null && rg -n foo src || grep -rn foo src",
            "jq '.package.name' Cargo.toml",
            "diff -u a.txt b.txt",
        ] {
            assert_eq!(p.check(&shell(ok)), Decision::Allow, "should allow: {ok}");
        }

        for bad in [
            "sed -i 's/foo/bar/g' src/main.rs",
            "sed -ni '10,20p' src/main.rs",
            "sed 's/foo/bar/w out.txt' src/main.rs",
            "sed '1,10w out.txt' src/main.rs",
            "sed '1e rm -rf x' src/main.rs",
            "sed -f script.sed src/main.rs",
            "awk '{system(\"rm -rf x\")}' src/main.rs",
            "awk '{print $0 > \"out.txt\"}' src/main.rs",
            "awk '{print $0 >> \"out.txt\"}' src/main.rs",
            "awk '{print $0 | \"sh\"}' src/main.rs",
            "awk -f script.awk src/main.rs",
            "sort -o out.txt in.txt",
            "sort --output=out.txt in.txt",
            "uniq in.txt out.txt",
            "find . -name '*.rs' -exec rm {} +",
            "find . -name '*.rs' | xargs rm -f",
            "fd -x rm {}",
            "yq -i '.a = 1' file.yaml",
            "git -c core.sshCommand=evil status",
            "git tag v1.0",
            "git remote add origin https://example.com",
            "git stash pop",
            "git worktree add ../wt",
            "git config user.name foo",
            "LD_PRELOAD=./evil.so grep foo a.txt",
            "./custom_script.sh",
        ] {
            let decision = p.check(&shell(bad));
            assert!(
                matches!(decision, Decision::Deny(_)),
                "should deny: {bad} (got {decision:?})"
            );
            if let Decision::Deny(reason) = decision {
                assert!(
                    reason.contains("prefer the dedicated read-only tools"),
                    "missing guidance in denial reason for `{bad}`: {reason}"
                );
            }
        }
    }
}
