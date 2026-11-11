# 16 — Hook setup in the TUI

Status is planned, as of 2026-11-10.

## Outcome

A developer can enable a shipped hook, configure its service, enter credentials,
check a private review URL and start using it in the same TUI session. Setup also
covers Jira and multiple self-hosted GitLab, Gitea/Forgejo and Bitbucket instances.
The existing CLI, slash commands, environment variables, keyring commands and
handwritten hook files continue to work.

The surface follows the existing ratatui pickers and masked prompts. It supports
keyboard operation, light/dark themes, ASCII mode and narrow terminals. The primary
flow is configuration, followed by one Save and enable action.

## User flows

### Private GitHub review

1. Open `/hooks`, select the code-review hook and choose Configure.
2. Select GitHub. Its public web and API addresses are prefilled from the bundle.
3. Choose authentication. Public access and Personal access token are available.
4. Paste the token into a masked field. Show a link or copyable address for the
   service's token instructions, including repository access requirements.
5. Choose Save and enable. Store the credential, write its reference, enable the
   hook and refresh the running engine. Return to the hook details with the saved
   connection and its status visible.
6. Paste a private PR URL in the normal composer. The next turn uses the saved key.

An existing connection opens directly to its editable fields. Adding another
connection always has an explicit Add connection action.

### Self-hosted forge

1. Choose Add connection, then the forge type and server address. Optionally paste
   a review URL to populate the address and show which instance it belongs to.
2. Preserve installation prefixes. Derive a proposed API base using bundle data,
   show it, and allow an override under advanced fields.
3. Enter a display name and credentials. Authentication choices follow the selected
   forge, including Bitbucket Cloud's email and API token combination.
4. Save and enable. The connection appears alongside the others, with its host and
   credential status. Add, edit, disable, remove and rotate credentials from here.

Support multiple instances of one forge and distinct installations on one host.
Each connection has a stable identifier and separate credential references.

### Jira and other hooks

Jira gets the same flow for server address, token and optional account email,
plus its existing size/comment options under advanced fields. A hook author can
supply setup fields as data to obtain this interface. Existing custom hooks can
still be listed, enabled and disabled when they have no setup descriptor.

### Check access and recover

Each connection offers Check access. A sample review or issue URL checks access to
that resource using the configured REST endpoints and authentication. A general
identity check, where the service supports it, reports only what it establishes.
The test never calls a model or adds its response to conversation context.

Show separate states for Saved, Checking, Verified for this URL, Authentication
failed, Resource unavailable, Rate limited, and Cannot reach server. A 404 may mean
missing access. Include the next useful action and retain the entered nonsecret
fields after a failure. Distinguish a local configuration check from a successful
remote request. Show when a previous verification is stale after an edit.

Saving requires local validation. Remote verification is an explicit action and
its failure can be retried or dismissed while keeping the saved configuration.
Cancel and Escape return to the previous screen. Escape from a dirty form offers
Keep editing or Discard changes. Secrets are cleared when the draft is discarded.

## Commands and navigation

| Entry | Behavior |
|---|---|
| `/hooks` | Existing list, with configuration and connection status in its details |
| Enter on a shipped hook requiring setup | Opens setup on first use |
| Configure action | Opens saved settings and connections |
| Enable/disable action | Explicit action available alongside Configure |
| `/hooks toggle <id>` | Existing direct toggle behavior |
| `/hooks configure <id>` | Direct entry to the same setup form |
| `/hooks test <id>` | Opens connection/resource selection for Check access |
| `quack keyring set <id>` | Existing credential storage command |
| `/reload` | Existing reload action, also useful after external edits |

Use the existing modal action and alternate-action mechanisms where they fit.
Display the available keys in the modal footer and `/help`. Tab and Shift-Tab move
between fields, Enter accepts the current step, and validation returns focus to
the affected field. Pasted tokens stay masked and never enter input history.
Long hostnames, errors and paths wrap. At narrow sizes, show the selected field
and its help in a single-column flow with a visible Back action.

## Data and backwards compatibility

Add optional, versioned setup metadata to hook definitions. Keep labels, service
presets, web/API base templates, authentication choices, help links and validation
hints in the defaults bundle. The Rust UI renders a small set of field types
including text, URL, choice, integer, boolean, secret and repeatable connections.
The metadata must describe every initially supported forge and Jira.

Store user-selected values as optional structured hook settings, alongside existing
fields in the user hook file. Keep secrets represented by the existing `Secret`
references and binding them through the hook's existing environment mechanism.
Connection identifiers determine stable, collision-free credential bindings.
The script receives nonsecret settings and credential variable names in its stdin
JSON event. Credential values are resolved by the engine and passed in the child
environment, as they are today.

Treat connection credential availability separately. An unavailable credential on
one instance must produce a connection-specific error while other configured
instances keep working. Setup metadata identifies optional connection bindings.
The engine reports a redacted unavailable-binding status to the script, which
reports it when a matching link is used. Preserve fail-on-resolution-error behavior
for existing required hook environment bindings.

Update the forge script to consume structured connections while retaining its
existing `--host` arguments. Preserve the Jira environment settings. Define
precedence explicitly so a configured connection is fetched once. The setup loader
imports recognizable existing `--host` and `[env]` entries into a draft, preserving
credential sources and options. Saving preserves unknown fields and custom command
arguments. If an existing command cannot be mapped safely, show its source and the
specific incompatible fields and keep the direct configuration route available.

Setup descriptors are declarative. Opening a form reads configuration and metadata.
Secret resolution, scripts and remote checks run when their actions need them.
Invalid or newer descriptors produce an actionable configuration status while the
hook's existing runtime definition remains usable. New fields remain optional,
with existing files retaining their runtime behavior. Regenerate the hook schema.

## Engine and storage

All TUI operations go through typed Ops and Events. Extend the protocol with setup
inspection, save and explicit test operations, carrying request identifiers so a
late response cannot replace a different form or connection. Add each new Op to
`op_name`. Secret input uses `SecretText`, following provider setup.

The engine returns redacted descriptors, current values, credential source/state,
effective layer and write capabilities. It never sends a stored token back to the
UI. The TUI holds form drafts and submits intentional changes, including an
explicit Keep, Replace or Remove choice for a credential. An empty masked field
means Keep when a credential already exists.

Reuse or extract the credential-saving helper in `quack-core/src/setup.rs`.
Use the OS credential store when available and the existing private-file fallback
on headless Linux or a compiled-out keyring. Show which storage was used. Preserve
existing environment, file, command and keyring references when editing unrelated
fields. Offer the existing external secret sources in advanced settings.

Stage configuration and payload changes, validate them and commit the enabled
configuration only after credentials and payloads are ready. Reuse the payload
keep/replace flow in `quack-config/src/enable.rs` and comment-preserving atomic
writes in `quack-config/src/edit.rs`. Keep saved credentials reachable on partial
failure. For rotation, stage a new credential reference and switch the config only
after storage succeeds, then clean up owned obsolete entries. Retries are
idempotent. Removing a connection preserves shared or externally managed secrets.

Protect edits against a stale config revision. On a conflict, reload the changed
values and offer to reapply the draft. Respect system enforcement, policy locks
and project trust. Default setup writes to the user layer. Show when a higher
layer determines the effective value and provide an explanation before saving an
ineffective override. Credentials stay in user credential storage even when an
existing trusted project definition references them.

After saving, reload and verify the effective hook and connections before reporting
success. Rebuild cached prepared commands and secret bindings so the next prompt
uses the new values. Serialize changes with turn execution at the engine's safe
reload boundary. Show pending application if a running turn must finish first.
Persist connection configuration and safe status metadata across restart.

## Checks, privacy and networking

An explicit test runs the hook's declared check entry point through the engine's
existing process runner with cancellation and timeout. Its request and response
are structured, with a redacted diagnostic result. Reuse the script's URL parsing,
REST request and authentication code so setup verification matches normal use.
Validate that the sample URL belongs to the selected connection before resolving
its credential. Cancelling a test leaves the configuration and conversation usable.

Show the API destinations while configuring the service. Retain exact-origin
credential scoping, HTTPS for tokens, prefix-aware URL matching and redirect
checks. Hooks remain configured child processes with the documented networking
semantics. Saving or opening setup triggers no remote request. Any future browser
sign-in can use the same credential storage once a service supplies that flow.
Personal access tokens are the initial authentication path.

Secret input and values must stay out of transcript entries, command history,
tracing, serialized session records, Debug output, test diagnostics and setup
responses. Test errors omit headers and secret-bearing query strings. Logging
reports the action and result using connection IDs and safe host information.

## Delivery sequence

1. Define optional setup/settings data and redacted protocol types. Add descriptors
   for the shipped forge and Jira hooks and backwards-compatible script parsing.
2. Implement inspection, credential persistence, staged saving and live activation
   in config/core. Reuse provider credential storage and hook enable/reload logic.
3. Add `/hooks` configuration screens, repeatable connections, masked input,
   credential rotation and source-aware statuses. Ship the private GitHub flow here.
4. Add explicit connection/resource checks and recovery states. Complete Jira and
   every self-hosted adapter through the same forms before calling setup complete.
5. Update schemas, README, hook examples, slash-command help, plan docs and roadmap.

## Acceptance and verification

- A user configures a private GitHub PR entirely in the TUI and uses it immediately.
- A user adds two self-hosted GitLab instances, including a URL prefix, and each
  receives only its configured credential. Gitea/Forgejo and both Bitbucket APIs
  work through the same flow, including Bitbucket Cloud Basic authentication.
- Jira can be configured through the same field and credential primitives.
- Editing, disabling, removing, rotating and restarting preserve the expected state.
- Existing CLI/slash commands, environment setup, handwritten files and hooks
  without setup metadata continue to work.
- Tests cover failed secret storage, failed writes, partial saves, stale revisions,
  shared secrets, cached credential refresh, payload collisions, enforced hooks,
  project trust, external overrides and cancel/retry behavior.
- Mock API/model integration tests prove that saving enables private review fetches
  on the next prompt, failed tests remain recoverable, and secret values never
  appear in client events or session logs. Use isolated key storage fixtures.
- TUI behavior tests and reviewed snapshots cover first setup, saved connections,
  masked input, errors and narrow layouts at 80x24 and 60x20, plus ASCII mode and
  Windows Press/Repeat handling.
- Existing full-diff behavior and size warnings continue through configured hooks.
  Verification uses mock endpoints and requires no paid model calls.

The initial design assumes personal access tokens, explicit connection checks and
user-layer configuration. Service labels and presets remain bundle data so future
hooks can acquire a setup form without adding service-specific TUI screens.
