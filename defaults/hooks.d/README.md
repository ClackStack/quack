# defaults/hooks.d

Example hooks ship disabled. Open `/hooks`, select a hook and press Enter to
configure it. Choose a service or edit an existing connection, enter its server URL,
choose authentication and paste the token into the masked field. Ctrl-E **saves and
enables** the hook; the next prompt uses it after any running turn finishes. Ctrl-S
saves settings while preserving the enabled state. Ctrl-T checks a sample review or
issue URL using the current draft without saving, enabling, or calling a model.

Use A to add a connection, Ctrl-D to disable one, and Delete to remove one from the
draft. Tab/Shift-Tab moves between fields; Enter edits; left/right cycles choices.
V includes advanced fields (API base, CA certificates, proxy and timeout, plus Jira
size/comment limits). ? shows scrollable help and token instructions. Secrets stay
masked and out of composer history. An empty secret field keeps its saved source;
Ctrl-R removes it and Ctrl-X selects an env, file, keyring or command source.
Changing a credential destination requires replacing, removing or explicitly
selecting its credential source again.

Credentials use the OS credential store, with private-file fallback when it is not
available. The TUI reports the storage used. Rotation stages a fresh reference;
shared and external credentials are preserved when removing a connection. Errors
keep the draft. R reloads saved settings (asking before discarding edits); Shift-R
reloads and reapplies the draft for review.

The hook's TOML and matching script folder are copied into your user config on the
first save. Existing payloads require an explicit keep/replace choice. Updates to
connections activate without restarting MCP/LSP or calling a remote service.
`/hooks configure <id>`, `/hooks test <id>`, `/hooks toggle <id>`, Ctrl-S in the hook
list, `quack keyring set <id>` and `/reload` are also available. Hooks without forms
can still be toggled. System hooks are enforced, and project hooks require trust.
`/doctor` checks programs and payloads without running them.

A hook needs an `event` and a `command`. The command gets the event as JSON on stdin and
may answer on stdout; see docs/plan/11-security.md#hooks, or `../schemas/hook.json` for
every option.

| File | Event | What it does |
|---|---|---|
| `jira.toml` + `jira/jira.py` | `user_prompt` | Adds the Jira issues a prompt links to, fetched over the REST API (Python 3, standard library only) |
| `forge-review.toml` + `forge-review/review.py` | `user_prompt` | Adds linked GitHub, GitLab, Gitea/Forgejo and Bitbucket reviews with REST metadata and a unified diff (Python 3, standard library only) |

## Linked code reviews

Configure `forge-review` through `/hooks`. Public connections for GitHub, GitLab,
Gitea, Codeberg (Forgejo) and Bitbucket Cloud are prefilled. For private repositories,
select token authentication and enter the credential. Bitbucket Cloud supports
Atlassian email plus API token with basic authentication, and access tokens with
token authentication. GitHub Enterprise, GitLab, Gitea/Forgejo and Bitbucket Data
Center support explicit self-hosted web and API bases, including URL prefixes.
Confirm the API base under advanced fields before saving. Use the server base URL,
including its installation prefix, rather than a full review URL.

Add as many instances as needed, each with its own credential source. The most
specific matching installation prefix wins, including a disabled connection. An
explicit `connections = []` prevents fetching. A missing credential affects its
connection while others keep working. A review check verifies both metadata and
the diff; readable metadata with an unavailable diff reports partial access.

The hook reads up to two distinct reviews per prompt. Each includes title,
state, description and branch/commit references where available, followed by a
unified diff. Review subpage links work too. Duplicate links share a fetch, and
query strings and fragments are omitted from API requests and added context.
Descriptions are limited to 1,200 characters, with explicit cut markers. Diffs
are loaded in full. A diff above 9,000 bytes produces a warning in the hook's
user-visible message showing its size. This hook sets `full_context = true` to
preserve stdout and context in full through the engine. Above 8,000 estimated
context tokens the engine also emits a warning. API errors appear beside the
affected review or diff, so another review can still load. Server-side diff
limits still apply, including GitLab's raw-diff limits. These snapshots describe
the review at fetch time and consume the model's configured context window.

Requests default to a five-second timeout and the hook has a 25-second overall timeout.
Tokens require HTTPS, except for local loopback servers. Redirects stay within the API origin, including Bitbucket
Cloud's redirect from a pull request diff to a repository diff. System TLS
roots and standard proxy environment variables apply.

Enabling the hook authorizes its child process to contact the configured API
bases. Hook subprocess networking is subject to operating-system and network
controls. Quack's in-process offline/egress gate does not govern these requests.

### Hook author setup metadata

`[setup]` is optional, with `version = 1`, `scope_field`, `fields` and `presets`.
Fields support text, URL, integer, boolean, choice and secret values, plus required
conditions, advanced grouping, help text and credential-destination markers.
Presets supply service labels, initial values, choices and derived field templates.
See the shipped TOML files and `../schemas/hook.json` for examples.

`connections` stores connection IDs, preset IDs, enabled flags, plain settings and
`Secret` credential references. The engine passes connection settings on stdin;
`credentials` maps secret field IDs to generated environment-variable names.
`unavailable_credentials` lists fields that could not resolve, without their values.
The script owns URL parsing, API selection, request headers and response rendering.
An explicit list is authoritative over script arguments and environment defaults.

For Check access the same command receives `event = "check"`, a single connection,
`url` and `matches`. Return JSON with `status` and `message`, without added context.
Supported statuses are `verified`, `partial`, `authentication_failed`,
`resource_unavailable`, `rate_limited`, `unreachable` and `invalid`. Checks use the
normal subprocess runner, a deadline, cancellation and redacted diagnostics.
Unsupported setup descriptor versions leave the normal hook executable usable.

### API references

- GitHub uses `GET /repos/{owner}/{repo}/pulls/{number}`, first as JSON and then
  with the diff media type. [GitHub REST documentation](https://docs.github.com/en/rest/pulls/pulls)
- GitLab uses `GET /projects/{encoded-path}/merge_requests/{iid}` and its
  `/raw_diffs` endpoint. [GitLab REST documentation](https://docs.gitlab.com/api/merge_requests/)
- Gitea/Forgejo uses `GET /repos/{owner}/{repo}/pulls/{index}` and `{index}.diff`.
  [Gitea REST documentation](https://docs.gitea.com/api/operations/repo-get-pull-request/),
  [Forgejo API documentation](https://forgejo.codeberg.page/docs/v17.0/user/api/)
- Bitbucket Cloud uses `GET /repositories/{workspace}/{repo}/pullrequests/{id}`
  and its `/diff` endpoint. [Bitbucket Cloud REST documentation](https://developer.atlassian.com/cloud/bitbucket/rest/api-group-pullrequests/)
- Bitbucket Data Center uses `GET /projects/{key}/repos/{repo}/pull-requests/{id}`
  and `{id}.diff`. [Bitbucket Data Center REST documentation](https://developer.atlassian.com/server/bitbucket/rest/v1000/api-group-pull-requests/)
