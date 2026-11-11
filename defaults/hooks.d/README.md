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
| `forge-review.toml` + `forge-review/review.py` | `user_prompt` | Adds linked GitHub, GitLab, Gitea/Forgejo and Bitbucket reviews with REST metadata and a unified diff (Python 3, standard library only) |

## Linked code reviews

Enable `forge-review` through `/hooks`. Public GitHub, GitLab, Gitea, Codeberg
and Bitbucket Cloud links then work anonymously where the server permits it.
Add credentials in your copy's `[env]` for private repositories or authenticated
rate limits. Each `--host` entry names the credential variable used exclusively
for that API. Variables whose names end in `_USER` enable Bitbucket Cloud Basic
authentication with the corresponding token. Other Bitbucket tokens use Bearer
authentication. GitLab uses `PRIVATE-TOKEN`, Gitea/Forgejo uses `token`, and GitHub
uses Bearer authentication.

Add a `--host` entry for each self-hosted instance, including installations under
a URL prefix. The four arguments are the forge type, web base URL, API base URL,
and credential variable name (`"-"` means anonymous). Use `gitea` for Forgejo too.
For example, a GitLab installed at `https://git.example/team` uses that web base
and `https://git.example/team/api/v4`. GitHub Enterprise uses `github` with its
web base and `/api/v3` API base. Bitbucket Data Center uses `bitbucket-server`
with its web base and `/rest/api/latest` API base. Copy the commented entries
in the TOML and add the matching secret in `[env]`.

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

Requests use a five-second timeout and the hook has a 25-second overall timeout.
Tokens require HTTPS. Redirects stay within the API origin, including Bitbucket
Cloud's redirect from a pull request diff to a repository diff. System TLS
roots and standard proxy environment variables apply.

Enabling the hook authorizes its child process to contact the configured API
bases. Hook subprocess networking is subject to operating-system and network
controls. Quack's in-process offline/egress gate does not govern these requests.

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
