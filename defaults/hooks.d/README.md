# defaults/hooks.d

Example hooks, one per file, all `enabled = false`. To use one, run `/hooks` in quack and
press ⏎ on it: the hook's file (and the folder its `command` names with `{dir}`, if any)
is copied into `hooks.d` in your config folder (`quack config paths` shows where) with
`enabled = true`, and `[hooks] disabled` stops listing it. The shipped copy stays as it
is, so an upgrade replaces it freely. Doing it by hand works the same: copy the file and
its folder, and set `enabled = true`. `/doctor` checks a hook without running it.

A hook keeps scripts and data next to its file in a folder named after it (`jira.toml`
→ `jira/`); its `command` reaches them with `{dir}`. When you toggle a hook off, quack
lists it in `[hooks] disabled` in your config.toml instead of touching the file.

A hook needs an `event` and a `command`. The command gets the event as JSON on stdin and
may answer on stdout; see docs/plan/11-security.md#hooks, or `../schemas/hook.json` for
every option.

| File | Event | What it does |
|---|---|---|
| `jira.toml` + `jira/jira.py` | `user_prompt` | Adds the Jira issues a prompt links to, fetched over the REST API (Python 3, standard library only) |
