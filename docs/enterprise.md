# Rolling quack out in a company

quack has no built-in providers, servers or telemetry. Everything it talks to comes from
TOML files, so a company decides what it can reach by deciding which files it ships and
locking the rest with a system policy. This page covers the parts an administrator needs:
the packages, the system policy, your own defaults bundle, and building from source without
network access.

## Packages

Each release ships a single CLI package (`quack`) and a single desktop GUI package (`quack-gui`)
for every supported target. TUI and GUI are separate apps sharing one core, shared
`defaults/` directory, config, sessions, and memory. Filtering out hosted-service providers
and `defaults/config.toml` (`--plucked`) happens during installation rather than building
separate `-plucked` binaries or packages:

- **Installer flags (`install.sh --plucked` / `install.ps1 -Plucked`)**: installs the same
  `quack` and/or `quack-gui` binaries (including Linux `.deb` and `.rpm` packages as well as
  archives), then strips `defaults/config.toml` and every `providers.d/*.toml` whose `base_url`
  is not loopback (`127.*`, `localhost`, `[::1]`) before leaving `defaults/` on disk. Profiles,
  `explore` and `general` sub-agents, language servers, hooks, MCP examples (off until
  enabled), and local server configs stay. Subsequent updates keep an existing install
  plucked automatically unless `--full` (`-Full`) is passed.
- **Archives & desktop installers**: `quack-<target>.tar.gz` (Linux, macOS) and `.zip`
  (Windows) carry the CLI binary and the `defaults/` folder. `quack-gui-<target>.dmg`
  (macOS), `.AppImage` (glibc Linux), and `-setup.exe` (Windows) carry the desktop app
  without embedding `defaults/` inside the app bundle. When `--gui` is run without
  `--binary-only`, `install.sh` / `install.ps1` pull the shared `defaults/` folder from the
  matching `quack` archive (without installing `quack`), applying `--plucked` if requested.
  `--archive FILE` installs from a local file without network access, `--binary-only`
  (`-BinaryOnly`) leaves an existing `defaults/` folder untouched, and `--prefix` picks the
  folder.
- **Linux**: `quack_<version>_<amd64|arm64>.deb` and `.rpm` install `/usr/bin/quack` and
  `/usr/share/quack/defaults` (static musl, zero runtime dependencies), and
  `quack-gui_<version>_<amd64|arm64>.deb` and `.rpm` install `/usr/bin/quack-gui` for
  glibc desktops. `install.sh` uses these packages by default when the machine has
  apt/dpkg or dnf/yum/zypper tooling plus root or sudo (`--pkg archive` keeps the
  self-contained `~/.local` install instead). Administrators deploying `.deb`/`.rpm` without
  hosted providers can remove
  or replace `/usr/share/quack/defaults` or override it via the system policy/bundle below.
- **macOS and Linux with Homebrew**: each release carries `quack.rb` for a tap.
- **Windows**: `quack-<target>.msi` (`x86_64-pc-windows-msvc` and `aarch64-pc-windows-msvc`, WiX v5) installs `quack.exe`, `LICENSE`, `NOTICE`, PATH registration, and the editable `defaults\` bundle into `%ProgramFiles%\quack\` per-machine by default (`ALLUSERS=2`, GPO/MDM/Intune ready) or per-user via `msiexec /i quack-<target>.msi MSIINSTALLPERUSER=1`. Enterprise transforms (`.mst`) or `REMOVE=DefaultsFeature` can customize or omit the defaults bundle. Builds are currently unsigned (Authenticode signing activates automatically in `scripts/package-msi.ps1` once a certificate is configured).
- **SBOMs & Attestations**: `quack.cdx.json` and `quack-gui.cdx.json` (CycloneDX 1.5) list every crate in the CLI and GUI binaries, and every release asset in `dist/*` carries a keyless GitHub SLSA build provenance attestation (`gh attestation verify <file> --repo ClackStack/quack`).
- `SHA256SUMS` covers every release asset.

The binary finds its shared defaults bundle relative to itself (`<exe>/../share/quack/defaults`,
`<exe dir>/defaults`), beside `$APPIMAGE` on Linux, in Homebrew/user share folders
(`/opt/homebrew/share/quack/defaults`, `~/.local/share/quack/defaults`,
`%LOCALAPPDATA%\Programs\quack\defaults`), in system share folders, or where `QUACK_DEFAULTS_DIR`
points. `quack doctor` shows which bundle it found.

## The system policy

The system layer sits under the user and project layers, but its `[policy]` table can lock
any setting so nothing above it changes it: not a user or project file, an environment
variable, a command-line flag, or `/config` in the UI. `quack config paths` prints where it
is:

| OS | System folder |
|---|---|
| Linux | `/etc/quack/` |
| macOS | `/Library/Application Support/quack/` |
| Windows | `%ProgramData%\quack\` |

Deploy `config.toml` there (plus `providers.d/`, `mcp.d/`, `hooks.d/` and so on, if you
ship items) with your usual tooling (MDM, Group Policy file deployment, Ansible). An example
that keeps everything on an internal model server:

```toml
[model]
default = "corp/qwen3.8-coder"

[network]
offline = true                     # nothing but the providers' base URLs and egress_allow

[permissions]
mode = "ask"
allow = ["edit:src/**"]             # selected edits run automatically; commands still ask

[sandbox]
mode = "workspace-write"

[policy]
locked = ["model.default", "network", "permissions", "sandbox.mode"]
max_permission_mode = "allow"      # caps approvals, including --yolo, in every layer
project_config = "trusted-only"    # or "never": ignore repositories' .quack/ entirely

[policy.providers]
allowed = ["corp"]                 # provider ids that may load
hosts = ["llm.corp.example"]       # a provider's base_url must be one of these

[policy.mcp]
allowed = ["github-enterprise", "jira"]
hosts = ["*.corp.example"]
```

- A lock covers everything under it: `locked = ["permissions"]` locks approvals, work
  mode and every rule list. A locked key that is unset stays unset.
- Work mode and approvals are independent: Plan reads/searches and proposes tests;
  Edit may implement. Ask prompts for unapproved actions; Allow keeps the configured
  sandbox and rules. To enforce planning only, set `permissions.work_mode = "plan"` and
  lock that key; this also prevents accepting a plan. See [work modes and approvals](modes.md).
- `[policy.providers] hosts` holds even when the defaults bundle is installed: a provider
  pointing anywhere else does not load. The same goes for remote MCP servers.
- Hooks in the system layer's `hooks.d/` always run; users cannot turn them off. Use them
  for logging or extra checks on tool calls.
- **Windows machine-level registry policy (`HKLM\SOFTWARE\Policies\quack`)**: on Windows, quack reads `HKLM\SOFTWARE\Policies\quack` directly from the registry (`RegGetValueW` with `RRF_NOEXPAND`):
  - `SystemDir` (`REG_SZ`, absolute path): pins the system configuration folder so neither `%ProgramData%` nor `QUACK_SYSTEM_DIR` can redirect policy.
  - `PolicyToml` (`REG_SZ` or `REG_MULTI_SZ`): TOML policy text merged into the system layer ahead of user/project config and locks.
  - `quack doctor` reports active system policy locks, registry policy presence, and audit log configuration.
- **Enterprise JSONL audit log (`[audit]`)**: set `[audit] path = "/var/log/quack/audit.jsonl"` (and lock `audit` in `[policy] locked`) to append structured JSONL records (`ts`, `session_id`, `agent`, `call_id`, `tool`, `subject`, `decision`, `decided_by`, `ok`, `duration_ms`) for every allowed, denied, hook-blocked, or interrupted tool execution. Set `include_content = true` only if you want tool arguments and summaries included; by default content is omitted.
- **Multi-endpoint failover (`base_urls`)**: internal clusters (vLLM, SGLang, llama.cpp) can list `base_urls = ["https://gpu-1.corp.example/v1", "https://gpu-2.corp.example/v1"]` in `providers.d/<id>.toml` for round-robin balancing and automatic health cooldown failover; `[policy.providers] hosts` validates every URL in `base_urls`.
- **Session retention**: configure `[sessions] retention_days = 30` (lockable via `[policy]`) for automatic background cleanup on startup, or run `quack sessions prune --older-than 30d [--all] [--dry-run]`.
- **Opt-in `web_fetch`**: disabled by default (`[tools.web_fetch] enabled = false`), unregistered whenever `[network] offline = true`, blocks loopback/private/link-local/CGNAT/metadata IPs with DNS-pinning against rebinding, validates every redirect hop against `[network]`/`[policy]`, and logs to the session egress ledger.
- Check the result on a client with `quack doctor` and `quack config show --origin`, which
  shows each setting's layer.

## Your own defaults bundle

The defaults bundle is plain files, so a company bundle is a folder you build and ship:

1. Start from `defaults/` in the source (or the bundle of a release archive).
2. Delete the providers, MCP servers and language servers you don't want, and add your own
   (`providers.d/corp.toml` with `base_url` and `models` is enough).
3. Ship it at `<prefix>/share/quack/defaults` with the binary (installed with
   `--binary-only`), or point `QUACK_DEFAULTS_DIR` at it.

Startup works with any file removed, so a smaller bundle needs no other changes. Teams can
also share skills, commands and server files as **bundles**: a folder with a `bundle.toml`
copied into `bundles/` in the system or user config folder. `/bundles` lists them.

## Building without network access

The recipe below was run with an empty `CARGO_HOME` and `--offline`, so every crate comes
from the vendored folder. It needs the pinned Rust toolchain (`rust-toolchain.toml`, with
your targets) installed on the build machine beforehand.

On a machine with network access:

```sh
git clone https://github.com/ClackStack/quack && cd quack
mkdir -p .cargo
cargo vendor --locked > .cargo/config.toml   # about 600 MB in vendor/
```

Copy the whole folder, `vendor/` and `.cargo/` included, to the build machine. Then, from
inside it (cargo reads `.cargo/config.toml` from the current folder, not the manifest's):

```sh
cargo build --release --locked --offline -p quack-cli \
  --no-default-features --features proto-openai-chat,lang-common,tokenizers-hf
scripts/package.sh x86_64-unknown-linux-gnu target/release/quack   # archives, if wanted
```

- The features keep what an internal OpenAI-compatible server (vLLM, llama.cpp, SGLang)
  needs. Add `proto-anthropic`, `proto-ollama`, `proto-openai-responses`, `acp` (editor
  integration) or `keyring` (OS credential stores) as required. A provider file that names
  a protocol left out of the build fails to load on its own, with a clear error.
- `--locked` and the pinned toolchain make builds repeatable. Set `SOURCE_DATE_EPOCH` for
  reproducible archive timestamps.

## Checking that nothing leaves the network

quack contacts only the providers and MCP servers in its files. Two opt-in checks, the
model catalog (`catalog.refresh`, `quack catalog refresh`) and the version check
(`updates.check`, `quack update`), are off by default and blocked by `[network] offline`
and by `[policy.providers] hosts` or `[policy.mcp] hosts`. To rule them out everywhere,
lock them: `locked = ["catalog", "updates"]`, with both URLs empty in the system layer. Or
point both URLs at an internal mirror. Installs with `--plucked` omit `defaults/config.toml`, so neither URL is shipped.

The test suite checks this
with a recording proxy (`no_egress_beyond_the_configured_provider`), and on Linux
`scripts/no-egress.sh` runs the startup and turn tests in a network namespace with only
loopback, tracing every `connect()`. To check a deployed setup, run a session with a
network capture and `[network] offline = true`. You should see connections only to your
model server's host and to the remote MCP servers you configured.
