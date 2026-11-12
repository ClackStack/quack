# defaults/profiles.d

Model profiles, one directory each: `profile.toml` plus a `system.md` prompt template
(minijinja), and optionally a `context.md`. `context.md` is the per-session part (workspace,
date, files, AGENTS.md, skills, memory), sent at the start of the first user message so the
system prompt and tools stay the same in every project and the server's prompt cache reuses
them. Without one, everything goes in `system.md`. Delete a directory to remove that profile. `generic` is also compiled into
the binary from these same files, so a binary without the bundle still has it; a `generic`
folder in any layer replaces it, and one whose `profile.toml` holds only `enabled = false`
turns it off, leaving the empty profile (no prompt beyond the instructions).

- `generic`: a short original prompt for any model, using the internal tool names.
- `qwen-code`: Qwen Code tool names (`read_file`, `edit`, `run_shell_command`, ...).
- `cc-style`: Claude Code-style tool names (`Read`, `Edit`, `Bash`, ...), for GLM, MiniMax,
  DeepSeek and MiMo.
- `zcode`: Z.ai's ZCode tool names (`Read`, `Edit`, `Bash`, `TodoWrite`, ...), for GLM.
- `kimi-code`: Kimi CLI tool names (`ReadFile`, `StrReplaceFile`, `Shell`, ...).
- `codex`: Codex-shaped tools for OpenAI coding models: `shell` with an argument list,
  `apply_patch`, `update_plan`, tracked `read` before patching existing files, `task`, and
  capability-gated `view_image`. Automatically selected only for `gpt-6.1-sol`,
  `gpt-6-astra`, `gpt-6-luna`, and `gpt-6.1-terra`; gpt-oss remains opt-in.
- `dsh`: DeepSeek Harness tool names (`read`, `write`, `edit`, `bash`, `todo_write`), for
  DeepSeek.
- `vibe`: Mistral Vibe tool names (`read_file`, `write_file`, `edit`, `bash`, `grep`,
  `todo`), for Mistral coding models (Large 4, Medium 3.5, Codestral, Devstral).

Most family profiles have empty `[match]` lists: they are used only when a model names
them (`profile = "qwen-code"`) until `quack eval` shows they beat `generic` for that
family; `zcode` (GLM-5) and `dsh` (DeepSeek-V4) passed that bar on 2026-10-20 and are
matched automatically. `vibe` (Mistral coding models) is matched automatically too, as
of 2026-11-09: it tied `generic`'s pass rate on a single hard pass with clearly fewer
steps and cost, and its `--repeat 3` confirmation is still owed. `codex` now matches
Sol, Astra, Luna and Terra by explicit user approval after a live Sol comparison and upstream
review (2026-11-11); Astra has one successful smoke task, while Luna/Terra remain unmeasured. Each file's comment
has the proposed globs for the profiles still waiting. Every prompt is
original text; only tool names and parameter shapes follow the family's own harness
(see each `source`).
