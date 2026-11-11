# 16 — Hook setup in the TUI

Implemented 2026-11-10. This is the initial pre-launch format. There is no legacy
installation migration framework. Existing commands and custom script hooks remain.

## Outcome and user flow

A developer configures private reviews and Jira issues entirely inside `/hooks`.
The engine and client are Rust. Link handlers remain removable Python files in the
defaults bundle; all committed regression tests are Rust.

1. Open `/hooks`, select the hook and press Enter to configure it.
2. Edit an existing connection, or add one from the bundle's service presets.
3. Enter the server base URL, including an installation prefix when needed. Public
   service addresses are prefilled. API bases derived from the web address remain
   visible under advanced settings and can be overridden.
4. Choose public, token or Basic authentication as offered by the preset. Paste a
   token into a masked field; Basic authentication also needs an account email.
   Contextual help supplies token instructions and access requirements.
5. Ctrl-S saves while preserving the hook's enabled state. Ctrl-E saves and enables.
   Credentials use the OS credential store, with private-file fallback when needed;
   the result identifies the storage used.
6. Paste a review/issue URL into the normal composer. The next turn uses the saved
   connection, without a separate reload or restarting MCP/LSP services.

There are presets for GitHub, GitHub Enterprise, GitLab, Gitea, Forgejo, Bitbucket
Cloud/Data Center and Jira Cloud/Server/Data Center. Multiple instances of a service
can coexist, including distinct installations on one host. Server bases are explicit;
a pasted review URL is used for access checks rather than guessing ambiguous GitLab
subgroup/installation prefixes.

A connection has a stable ID. It can be edited, disabled, removed, or have its
credential replaced or removed. Removing it does not enable a legacy argument entry.
An explicit `connections = []` means no connections. The most specific matching web
prefix wins; a disabled specific connection suppresses a broader match. Duplicate
enabled web bases are rejected.

## Forms and commands

Forms use the existing terminal styles, themes, borders and ASCII glyph choices.
They show one selected field with its help, so the same flow works at 80x24 and
60x20. Tab/Shift-Tab changes fields, Enter edits, and left/right cycles choices.
V includes advanced fields. ? opens scrollable help, including token documentation
and the configuration source. Escape backs out; leaving a dirty draft offers keep,
discard, or reload. Tokens never enter composer history.

Ctrl-T opens a sample URL prompt using unsaved settings. A check does not persist
credentials, enable hooks, call a model, or add conversation context. A review check
requires readable metadata and diff; metadata alone reports partial access. Statuses
also distinguish authentication failure, missing/inaccessible resources, rate limits,
connection failures and invalid settings. A 404 does not claim a token is invalid.
Remote verification is optional; local validation is required for saving.

Checks run as background engine tasks. Escape/Ctrl-C cancels a pending check and its
child process tree. External credential commands use the cancellable runner too.
Keyring resolution stays off the async runtime and is covered by the check deadline.
An edit invalidates prior verification. Closing a form or cancelling a check prevents
late results from updating a different draft. Verification is session-local; after a
restart the UI reports configured settings with access not yet checked.

| Entry | Behavior |
|---|---|
| `/hooks` | Hook list, connection setup and recent runs |
| Enter on a configurable hook | Open setup |
| Ctrl-S in the hook list | Existing enable/disable action |
| `/hooks configure <id>` | Direct setup entry |
| `/hooks test <id>` | Select a connection/resource for Check access |
| `/hooks toggle <id>` | Existing direct toggle |
| `quack keyring set <id>` | Existing credential command |
| `/reload` | Existing full reload after external changes |

Inside a form Ctrl-S saves, Ctrl-E saves and enables, Ctrl-T checks, A adds, Ctrl-D
toggles the selected connection, and Delete removes it from the draft. Secret fields
offer Keep (empty input), Replace (masked input), Remove (Ctrl-R), and an external
source (Ctrl-X: env, file, keyring or command). R reloads saved values after offering
to discard edits; Shift-R reloads and reapplies the draft for review. Keys are shown
in the form footer or contextual help and the command catalogue.

## Data contract

`HookFile.setup` holds optional declarative metadata. It is parsed separately from
the executable definition: an invalid or unsupported descriptor does not prevent
normal hook execution. Version 1 describes fields, service presets and a web-scope
field. Field types are text, URL, integer, boolean, choice and secret, with required
conditions, help, advanced grouping and credential-destination markers. Presets carry
service labels, initial values, allowed choices, derived fields and help links.
Service addresses and Python commands remain TOML data, never compiled constants.

`HookFile.connections` optionally stores `HookBinding` entries: a plain connection
(ID, preset, enabled flag, values) and a map of `Secret` credential references.
The script receives nonsecret connection values and generated credential variable
names on stdin. Resolved values travel only in its child environment. An unavailable
connection credential is reported without its value; other connections still run.
Required legacy `[env]` bindings retain their existing failure behavior. A runtime
preparation containing unavailable credentials is retried on the next use.

The forge handler still accepts handwritten `--host` entries when no structured
connection list is supplied; Jira still accepts handwritten environment settings.
An explicit list is authoritative, even when empty. No automatic import or migration
of older user installations is required before launch. Existing custom hooks without
descriptors remain executable and can be toggled using existing commands.

For checks the command receives `event = "check"`, a single connection, `url` and
`matches`. It returns a structured `status` and redacted `message`, without context.
The same parsing/authentication/REST routines serve checks and normal link handling.
The hook JSON schema documents the descriptor and connection shapes.

## Engine, storage and activation

Inspection, save, check and cancellation use typed Ops/Events and request IDs.
The client owns drafts; filesystem writes, subprocesses and credential access belong
to the engine. Inspection reads metadata and credential source information without
resolving a token, running a script or making a remote request. Stored tokens never
come back to the UI. `SecretText` protects user-entered credential values in Debug.
Prepared hook commands also redact their environment in Debug output.

Before storing a key, validate the draft, connection IDs, choices, required fields,
web/API addresses, duplicate bases, policy and current revision. Changing the preset
or a credential destination requires explicit replacement/removal/source selection;
Keep cannot silently send an old credential to a new server. Authenticated API bases
require HTTPS, except local loopback servers used for local development and mocks.
A sample URL must belong to the selected web base before its secret is resolved.

Saving stages fresh credential references, installs any required payload, and then
atomically writes the connection configuration while preserving TOML comments.
Existing payload collisions require Keep or Replace before credential storage.
Failed staging removes newly created entries which have no saved reference. Rotation
switches references before retiring owned obsolete entries, retaining any secret
still referenced by another connection/hook. External sources are never deleted.
Storage tests inject an isolated store and never access the developer's keyring.

The default write target is the user layer. Trusted project definitions are edited
in place and their trust fingerprint is refreshed; their secrets remain in user
credential storage. System enforcement, policy locks and environment/CLI disable
overrides are respected. Stale revisions reject the write before changing active
configuration. The client can reload and reapply its draft for review.

Saves wait for the running turn's boundary, then reread configuration and rebuild
only the hook list and prepared credentials. They publish the updated config and
report the effective saved state. They do not invoke the full `/reload` startup path,
restart MCP/LSP, or make remote requests. Opening/saving a form does not test access.

## Networking and privacy

Handlers retain exact-origin credential/redirect checks, prefix-aware matching,
query/fragment removal from added review context, and TLS certificate validation.
Advanced fields expose extra CA roots, proxy and request timeout. Hook subprocess
traffic follows the existing documented OS/network controls; it is outside the
engine's in-process HTTP egress gate. Enabling/configuring a hook authorizes its
configured APIs. No network discovery occurs on opening a form.

Check diagnostics omit response bodies, headers, tokens and secret-bearing URLs.
The engine redacts resolved secret values from returned messages and does not write
check requests/results into the conversation or session log. Token input stays out
of transcripts, composer history, tracing and Debug output. The initial sign-in path
uses tokens; browser OAuth is outside this implementation.

## Verification

- Rust engine tests configure a private review, save/enable, fetch on the next prompt,
  and restart successfully. Save-only keeps a disabled hook disabled. Opening/saving
  makes no service/model requests beyond normal startup.
- Rust subprocess tests exercise every forge REST route and authentication form,
  Jira, nested GitLab paths, longest-prefix routing, disabled/empty lists, deduplication,
  malformed URLs, query stripping, cross-origin redirects, unavailable credentials,
  partial checks and actionable HTTP errors. Full diffs larger than 2 MiB survive.
- Rust storage tests cover failed stores, conflicts during staging, private-file mode,
  fresh references on rotation, shared references and external secret preservation.
- TUI key tests cover masked paste, separate engine Ops, save versus enable, adding
  connections, cancellation, dirty drafts and late responses. Reviewed snapshots
  cover 80x24, 60x20 and masked input in light/ASCII mode.
- Existing generic hooks, CLI/slash commands, schema generation, workspace tests,
  warning-free clippy and minimal-feature builds remain required checks. No paid
  model calls or real service credentials are needed for verification.
