# 15 — Desktop client (Tauri + SvelteKit)

Decision (2026-11-08): the desktop GUI is **Tauri 2 + SvelteKit (Svelte 5)**, and the app *is* the client: Tauri's own IPC carries the protocol, so there is no `quack serve`, no port, no token and no browser mode. Launching `quack-gui` opens the window and goes; `quack-cli` is untouched. The engine does not change: a GUI is a fourth client of the Op/Event protocol ([00](00-architecture.md#engine-protocol)), alongside the TUI, `exec` and ACP ([12](12-headless-and-acp.md)). **No new `Op` or `Event` is required** — turns and streaming deltas, approvals and questions, first-run setup (`Op::DetectServers`, `Op::ListServices`, `Op::SetProviderKey`, `Op::AddServer`), project trust, plan mode and plan accept, rewind, stats and context reports, sub-agents, config writes (`Op::SaveSetting`) and sessions (`Op::ListSessions` → `Event::SessionList`, resume replay through `Event::SessionResumed`) all exist. Only [`ReplayItem`](../../crates/quack-protocol/src/lib.rs) grows optional `detail` (diffs/output) and `Reasoning` so resumed sessions keep collapsible tool output and thinking blocks.

A browser target was considered first (a loopback `quack serve` hosting the same SPA, for headless servers over SSH) and dropped the same day: it added a server, an auth surface and the axum dependency tree for a path nobody asked for. The SPA keeps its transport behind one small module, so a browser client could be added later without moving the UI.

Why this stack (investigated 2026-11-08): what a GUI must rebuild is the TUI's roughly 8,000 lines of rendering — markdown, diffs, highlighting, charts, modals — and web tech replaces most of that with mature parts (using the same Svelte 5 + SvelteKit + Vite stack as `website/`), while desktop integration (menus, window state, dialogs, notifications, clipboard, file opener) is Tauri plugins instead of per-platform code. The Rust side stays substantial: the bridge, engines and packaging. The webview holds only views.

| Stack | Verdict |
|---|---|
| Tauri 2 + SvelteKit | **chosen.** Healthiest upstream (2.12 stable, Apache-2.0/MIT, MSRV below quack's), matches `website/`'s Svelte 5 stack, Windows/macOS/Linux via the OS webview |
| gpui (+ gpui-component) | viable but costlier: crates.io packaging still pin-a-git-rev, no musl GUI builds upstream, the rendering rewrite stays, and Zed's own `ui`/`markdown` crates are GPL-3.0 and must not be used |
| egui, iced | weaker fallbacks if webviews are ever a hard no (immediate-mode ergonomics; one release in twelve months) |
| Slint | disqualified: GPL-3.0 or a paid licence, fails the `cargo deny` Apache-only gate |

## Layout

| Piece | Where | What |
|---|---|---|
| `quack-gui` (library: pure-Rust bridge) | `crates/quack-gui/src/lib.rs` | Engine-per-tab registry, 16 ms delta coalescer, `AppInfo` snapshot builder, session-lock guard and recent-workspaces state. **No Tauri dependency by default**, so `cargo test --workspace`, `cargo clippy --workspace` and the MSRV check run on every OS without `webkit2gtk` or a built frontend |
| `quack-gui` (binary: Tauri shell) | `crates/quack-gui/src/main.rs` (feature `desktop`) | The Tauri 2 app. Thin `#[tauri::command]` wrappers (`open_folder`, `send_op`, `close_tab`, `recent_folders`, …) over the bridge; streams coalesced `Event`s to the webview over a per-tab `tauri::ipc::Channel` |
| the SPA | `crates/quack-gui/ui/` | SvelteKit 3 + Svelte 5 runes app, built as a static bundle (`@sveltejs/adapter-static`, SPA mode: `fallback: 'index.html'`, output `build/`), embedded via `frontendDist`. All engine traffic goes through one `Transport` module over Tauri IPC (`invoke` + channel listener) |

No server component exists. `quack-gui` depends on `quack-core`/`quack-config` (it spawns engines) and never on another client crate: it mirrors the TUI, it does not import it. Because the `tauri` dependency is gated behind `--features desktop`, nothing about the default `cargo build`, `cargo test --workspace` or the `quack` binary changes.

## As implemented (2026-11-09, G0 bridge core + the SPA skeleton)

The pure-Rust bridge (`crates/quack-gui`, no Tauri dependency by default), the SvelteKit SPA (`crates/quack-gui/ui`), and the Tauri shell (`src/main.rs` behind `--features desktop`) exist; CI gained a `gui` job (four OSes; Linux installs the webkit2gtk dev packages) that checks and builds the SPA, regenerates `protocol.ts` with `git diff --exit-code`, and compiles the shell. Notes and deviations from the sketches above:

- `open_folder` returns a numeric tab id; the `AppInfo` snapshot arrives as the channel's first message (the envelope sketch below is canonical). The SPA's Transport captures it so the header can render immediately.
- Opening returns after the initial `SessionConfigured`, including when no model is ready. Resume/replay continues asynchronously. One live engine owns each workspace, checked before spawning; a duplicate open returns a structured `already_open` error and the SPA focuses that tab. This prevents startup resume records and later session switches from introducing a second log writer.
- The login-shell `PATH` merge lives in `path_env`, behind `desktop`: `std::env::set_var` is `unsafe` in edition 2024 and the workspace denies `unsafe_code`, so the merge is scoped to the shell with an `#[allow]` and a SAFETY comment (single-threaded startup, before any engine spawns).
- Deltas coalesce per stream (`(agent id, kind, call id)`), merge in start order, and flush on a 16 ms tick or before any non-delta event; deltas inside `Event::Agent` are buffered under their own id.
- The SPA is hand-rolled CSS (the plan's lean dependency list) with system font stacks; the one npm addition beyond the listed set is `@tauri-apps/api` (+ `plugin-dialog`) because `withGlobalTauri` stays false. `pnpm dev` in a plain browser works against a scripted dev transport through the same `Transport` seam.
- The Tauri tree resolved into `Cargo.lock` at 322 crates (tauri + the six plugins + `ts-rs`), above the spike's 120–180 estimate; all MIT/Apache-2.0, gated by `cargo deny` in CI.
- `protocol.ts` is generated and checked in: `cargo test -p quack-protocol --features ts export` writes one file of `export type` declarations (`../quack-gui/ui/src/lib/protocol.ts`). ts-rs's `serde-compat` turns `#[serde(default)]`/`skip_serializing_if` into optional (`?`) fields and renders transparent ids and `SecretText` as `string`, `u64` ids as `number` (`large_int_as_number`). Two tests pin the agreement: `wire_shapes_agree_with_the_typescript_types` (src/lib.rs) checks the serde JSON of tagged variants, and the export test asserts the generated shapes (tags, `[number, number]` tuples, `Box` recursion through `Event::Agent`).
- Replay carries two new, backwards-compatible fields the desktop client renders: `ReplayItem::Tool.detail` (the full diff or output, written into the `tool` record by `turn.rs` when a tool returned one) and a `ReplayItem::Reasoning` variant (from an assistant message's unsealed reasoning). The TUI replays both (reasoning entries, expandable tool detail), ACP maps reasoning to `agent_thought_chunk`, and the SPA shows reasoning collapsed and the detail in the tool card.
- SvelteKit 3 removed the `$lib` alias: the SPA imports with `#lib/*`, resolved for vite by the package.json `imports` field and for TypeScript by an explicit `paths` entry in `tsconfig.json` (kit's generated tsconfig only mirrors the deprecated `alias` option). `pnpm check` (svelte-check) and `pnpm build` (adapter-static → `build/`, the Tauri `frontendDist`) are clean.
- The shell needs icons at compile time: even with no icon configured, `generate_context!` falls back to `icons/icon.png` (and Windows' resource embed and macOS dev look for `icon.ico`/`icon.icns`), so `icons/` holds the standard set (32x32, 128x128, 128x128@2x, icon.png/ico/icns), drawn by `icons/make_icons.py` (Pillow, on the app.css palette, with the title duck's solid `>` beak) and listed under `bundle.icon`. `git_branch`'s tests use `.git/HEAD` fixtures instead of asserting the checkout's real branch, which broke on any branch name and on CI's detached-HEAD clones.

## Foundation review (2026-11-10)

The shared engine, protocol boundary, feature-gated Tauri shell and generated protocol types remain the foundation. The review fixed:

- **Open handshake deadlock:** the transport now invokes `open_folder` before waiting for its first channel snapshot; command failures reject immediately.
- **Lost startup/replay:** the page registers a session on the first `AppInfo`, before the command resolves, so startup facts and saved transcript events have an owner. Page teardown removes the listener. Model catalog requests are explicit, and provider/model splitting preserves slashes inside model ids.
- **Session ownership:** the pre-spawn workspace guard prevents duplicate resume writes, including after New/Resume/Fork operations. Opening without a ready model no longer waits for a second session announcement that never arrives.
- **Event loss and shutdown:** broadcast lag shuts down the engine and reports a recoverable closed state instead of silently continuing with an incomplete transcript or missed approval. Closed is emitted after shutdown; the composer disables sends for a stopped tab.
- **Standalone builds:** `desktop` enables the CLI-equivalent provider protocols, ChatGPT auth, grammars, tokenizer and keyring features; it does not rely on workspace feature unification. `frontendDist` points at `ui/build` relative to the Tauri config, and CSP allows only self and the IPC endpoints for connections, following [Tauri's CSP guidance](https://v2.tauri.app/security/csp/).
- **Build/test hygiene:** the lockfile now matches the existing `rmcp = 3.9.0` workspace pin. The approval mock matches a tool-role message, and recent-folder tests use isolated temporary directories.

Regression tests exercise the IPC handshake, early replay, duplicate focusing, closed-state handling, model ids containing slashes, no-model startup, pre-spawn log integrity and workspace ownership across session changes. The browser demo alone is not an integration test.

Validated locally on Linux: `cargo test -p quack-gui -p quack-mcp --locked` (29 tests), `cargo test -p quack-gui --features desktop --locked` (17 tests), desktop Clippy with `RUSTFLAGS="-D warnings"` and `--all-targets --locked`, `cargo fmt --all --check`, Svelte analysis, `pnpm check`, `pnpm test` (15 tests), and `pnpm build`. Rust tests used `QUACK_DEFAULTS_DIR=none`; no live model calls were made. Full-workspace/MSRV checks and native-window tests on the four-platform matrix were not run in this review.

Remaining before calling the desktop MVP ready (updated 2026-11-11):

- Exercise defaults resolution from installed native bundles. Resources are now mapped and resolved through Tauri's resource directory, with isolated loader tests; installer/runtime smoke tests remain.
- Finish the remaining G1 user flows: launch-folder arguments/restoring open tabs, and the transcript code-highlighting choice.
- Run native-window smoke tests and one local-model task on each supported OS/architecture. Compile checks and mock-server tests do not prove webview runtime behavior, packaged assets, or platform shutdown behavior.

These remain G1–G3 work; this review does not mark the desktop release or cross-platform smoke criteria complete.

## MVP setup and recovery (2026-11-11)

First-run setup now opens after `StartupComplete` when no model is ready. The Providers button reopens it later. The surface lists configured models and hosted services, accepts provider keys and custom OpenAI-compatible URLs, and offers an explicit local-server scan. Each action uses the existing setup Ops; `SessionConfigured` confirms model selection. Provider-only catalog responses merge into the displayed list, and a slow older catalog response cannot override the acknowledged model. Keys stay out of transcript state and are cleared from forms after submission. The browser demo explicitly declines to store real provider credentials.

`ProjectTrust` now gates the conversation with Trust project, Use settings only, and Ignore project configuration. The gate stays until a successful config reload; failed writes remain visible and retryable. The engine continues to enforce what each decision loads.

New/Resume preserve the transcript until `SessionConfigured`/`SessionResumed` acknowledge the switch. A failed resume leaves the previous conversation visible. Failed IPC sends retain message drafts, approvals and question answers; action errors appear above the active surface. Per-workspace drafts survive tab changes, duplicate sends are blocked while queued/running, and IME composition does not submit a message. Approval shortcuts avoid focused form controls and repeated keys. The corrected workspace grid keeps the composer in the window; narrow windows expose the sidebar through Sessions.

Native resource mapping ships the repository's `defaults/` at `defaults/` under Tauri's resource directory, following [Tauri resource configuration](https://v2.tauri.app/develop/resources/). The desktop registry uses that directory as its defaults fallback without mutating process environment; explicit `QUACK_DEFAULTS_DIR`, including `none`, wins and reloads keep the same resource source. This handles platform-specific resource locations through Tauri instead of adding application-specific paths to the shared config loader. Installed packages still need runtime verification.

Validation on Linux: 20 GUI Rust tests, including first-run setup/model selection and project-trust reload over the bridge, plus resource loading/override tests; desktop Clippy with warnings denied; 26 frontend tests; Svelte checks/analysis and production build; protocol export and formatting. A Chromium smoke pass exercised trust, URL setup, model selection, failed-send draft recovery, failed approval retry, and 1200×800/720×480 light/dark layouts using a scripted transport. That browser pass is separate from the real-engine wiremock tests. No live inference calls were made. Native-window, installed-package and cross-platform task checks remain open, so G1 is not marked complete.

## Native startup and session lifecycle (2026-11-11)

Login-shell PATH recovery now stops after two seconds or 64 KiB of output. A POSIX child prints the exported PATH with NUL framing, so login-shell banners, trailing newlines and fish's list expansion cannot become path entries. The merge preserves non-UTF-8 entries, prefers login-shell order, keeps inherited tools and skips empty/duplicate components. Failed recovery keeps the inherited PATH. A nonblocking socket avoids reader threads before the startup environment change; the shell's process group is killed and reaped on completion or failure. The Unix-only desktop dependency reuses the already locked `nix` version and adds no transitive packages.

The registry now locks only its tab map for bookkeeping. Config loading, startup, operation backpressure and shutdown run outside that lock. Workspace reservations cover startup through completed shutdown; simultaneous opens cannot start competing engines, and reopening a closing workspace waits for its owner. Each tab task owns its engine independently of the opening IPC future. Cancelling an open before acknowledgement shuts its reserved engine down. Dropping the registry signals its tasks to stop; explicit close/quit paths still await cleanup.

A native [page-load hook](https://docs.rs/tauri/2.12.5/tauri/struct.Builder.html#method.on_page_load) invalidates discarded channels and signals their engines as soon as a full navigation starts. The desktop transport begins with `connect` and carries its returned connection id on every command. A new UI document invalidates the previous connection, signals all previous tab tasks and waits for their engines to finish before opening anything. Stale commands fail, and an old config load cannot publish an engine after reconnect. Reload returns to Welcome/recent folders; reopening a folder with Latest restores saved history. This is cleanup followed by resume, not live engine reattachment, and does not restore unsent drafts after a document reload. The current shell has one webview; independent multi-window ownership remains outside the MVP.

Validation on Linux: 30 desktop Rust tests (including six concurrency/cancellation/reconnect tests and four Unix PATH tests), 26 bridge-only tests, warnings-denied desktop Clippy, 28 frontend tests, Svelte checks and a production SPA build. Gate-controlled tests verify other-tab Interrupt/session controls during slow load/close, closing-workspace reservation, simultaneous opens, stale config completion, cancelled opens and engine retirement. All config, cache and recent-folder data in tests are isolated. No live model calls. Native-window reload, installed-bundle and other-platform runtime checks remain pending; G1 remains in progress.

## The bridge

One `Channel` per tab, created by the frontend and handed to `open_folder`; commands carry a connection id and (where applicable) a tab id:

```
invoke("connect") → client                      // retires engines from the discarded UI document
invoke("open_folder", { client, cwd, resume, events }) → tab (number); AppInfo arrives on events first
invoke("send_op", { client, tab, op })            // op is tagged JSON, parsed into quack_protocol::Op
invoke("close_tab", { client, tab })             // awaits engine.shutdown() so MCP/LSP and session_end hooks finish
events.listen(…)                                 // {"tab":…,"event":{…}}, {"tab":…,"info":{…}} or {"tab":…,"closed":{…}}
```

- `send_op` accepts the `Op`'s serde JSON as-is (`#[serde(tag = "op")]`, `snake_case`); no translation layer beyond the tab envelope.
- **TypeScript types are generated from `quack-protocol`:** `ts-rs` (behind a non-default feature/test on `quack-protocol`, adding zero crates to normal builds) exports `protocol.ts`, checked in CI with `git diff --exit-code` the same way `quack config schema --dir defaults/schemas` checks the config schemas.
- **`AppInfo` snapshot:** at tab start and after `Event::StartupComplete` (including reloads), the bridge reads `engine.config()` (`Arc<Loaded>`) and sends the same static facts the TUI's `AppInfo::from_config` gathers (`setting_rows()`, `layers`, permission `rules`, `bundles`, git `branch`, `ui` settings). All config and key mutations go through existing `Op`s (`Op::SaveSetting`, `Op::SetProviderKey`, `Op::AddServer`, `Op::AddProvider`, `ApprovalDecision::AlwaysAllow`) — the GUI never calls `toml_edit` directly.
- **Independent engines & clean shutdown:** opening a tab loads that folder's config and spawns its engine (the ACP pattern). The registry refuses a second live engine for the same canonical workspace before spawning it (focusing the existing tab instead). Session ids are tracked as they change. This is a deliberate MVP limit: use the session sidebar to switch sessions within a workspace; separate folders can have separate tabs. Closing a tab or quitting the app awaits `engine.shutdown()` so child processes, MCP/LSP servers and `session_end` hooks clean up.
- **Always-on 16 ms delta coalescing:** `Channel::send` posts asynchronously to the webview queue and does not block when JS rendering lags. Each tab's bridge task buffers `AssistantDelta`, `ReasoningDelta` and `ToolCallOutputDelta` (including inside `Event::Agent`) and flushes them on a 16 ms (60 fps) tick — or immediately before any non-delta event (`ToolCallBegin`, `ApprovalRequest`, `TurnComplete`) — matching `quack-tui`'s frame loop and cutting IPC/DOM churn by 10–20× during fast streaming.
- **Desktop environment parity:**
  - **Login-shell `PATH` (macOS/Linux):** when launched from Finder/Dock or a desktop launcher rather than a terminal, `quack-gui` resolves the user's login-shell `PATH` once at startup, capped at two seconds and 64 KiB of output, before spawning engines, so `quack-shell`, `quack-mcp` (`npx`/`uvx`), `quack-lsp` and `secret-tool` find the same tools as the TUI.
  - **Defaults bundle in native resources:** the shell passes Tauri's platform-specific resource directory to the registry. Its loader supplies `<resource dir>/defaults` as a fallback, beneath an explicit `QUACK_DEFAULTS_DIR`, so `.app`, Windows and Linux bundle layouts use the same path contract.
- **Files and images** come from native dialogs, `@`-mentions in the composer (`InputItem::File`) and drag-drop (`InputItem::Image`), carrying real local paths into `Op::UserTurn` — no upload path is needed.
- Dev mode uses Tauri's `devUrl` against the vite dev server: hot reload, IPC working, standard workflow.

### Security

- There is no socket: nothing listens, so no port, token, origin or DNS-rebinding story. Tauri IPC is reachable only from the app's own webviews.
- CSP stays minimal (Tauri injects what its IPC needs; everything else is `'self'`), assets are bundled — the UI loads nothing from the network. `withGlobalTauri: false`, least-privilege capabilities (dialog, clipboard-manager, notification, window-state, opener for revealing files, single-instance), no arbitrary shell plugin.
- No egress change: the engines keep the egress gate, `network.offline` and `[policy.providers] hosts` exactly as every other client does.

## The UI

Instead of cloning the TUI's 23 terminal modals one-for-one, the GUI groups everything into **four desktop-native surfaces** plus a command palette (`Ctrl-K` / `Cmd-K`):

1. **Welcome / Recent Folders & First-Run Setup (when no folder or model is ready):**
   - Launching from an icon reopens the last open workspace tabs (persisted in `user_data_dir`), or shows a Welcome view with *Open Folder…* and recent folders (`quack-gui [path]` opens that folder directly).
   - When startup finishes with no model ready, the setup flow opens automatically (matching the TUI's `/setup`: local server detection via `Op::DetectServers`, hosted services via `Op::ListServices` + `Op::SetProviderKey`, or custom URL via `Op::AddServer`).
2. **Header, Composer & Context Inspector:**
   - **Header / composer bar:** model selector dropdown (`Op::ListModels` + inline provider status and key entry), permission mode pill (`Ask` · `Auto-edit` · `Plan` · `Yolo` + sandbox indicator), and a live context meter pill.
   - **Composer:** multi-line editor, `@` workspace file autocomplete (`InputItem::File`), image/file drag-drop and picker (`InputItem::Image`), `/skill` and custom command autocomplete.
   - **Context & Stats popover:** clicking the context pill opens the `/context` segment breakdown (`Op::GetContext`), `/stats` SVG charts (`Op::GetStats`, from the same data as the TUI's `charts.rs`) and a *Compact now* action (`Op::Compact`).
3. **Sidebar (Workspaces, Sessions, Plan & Sub-agents):**
   - Open folders and their saved sessions (`Op::ListSessions`, `Op::NewSession`, `Op::ForkSession`, inline rename via `Op::SetTitle`, export via `Op::Export`).
   - Live `Event::Plan` checklist and `Event::AgentStarted`/`Agent`/`AgentFinished` sub-agent runs pinned cleanly without needing `/todo` or `/tasks` modals.
4. **Transcript with Inline Actions & One Unified Settings Dialog:**
   - **Transcript:** markdown rendered client-side, code highlighted either via `quack_core::grammar` (`tree-sitter-highlight`, preserving `grammars/<lang>/highlights.scm` overrides) or lean `shiki/core` (`shiki/engine/javascript`, `lang-common` grammars only, no WASM), collapsible tool cards with diffs/output, and built-in dark/light CSS themes matching `quack-tui/src/theme.rs` (ready for `defaults/themes/` when M3 lands).
   - **Inline actions:** hover *Rewind / Edit from here* on any user message (`Op::ListRewindPoints` + `Op::Rewind`), *Undo* (`Op::Undo`) and *Open / Reveal* (`tauri-plugin-opener`) on file-edit tool cards, and the *Plan Review Bar* (`Op::AcceptPlan`) directly above the composer when a plan turn finishes.
   - **Approvals, questions & trust:** approvals render the diff inline above the composer; `Event::Questions` renders interactive choice/text cards; `Event::ProjectTrust` gates the folder view until answered (`Op::TrustProject`).
   - **Unified Settings & Workspace dialog (tabbed):** *Providers & Profiles* (`/setup`, `/providers`, `/key`, `/profiles`), *Workspace* (`/instructions`, `/memory`, `/skills`, `/bundles`), *MCP & LSP* (`/mcp` with restart + logs, `/lsp` with diagnostics, `/permissions` rules), and *Settings & Doctor* (`/config` table with inline edits via `Op::SaveSetting`, `/doctor` via `Op::RunDoctor`).

## CI and release

- `ci.yml` and `.forgejo/workflows/ci.yml`: because `tauri` is behind `--features desktop` on `quack-gui`, the existing `cargo clippy --workspace`, `cargo test --workspace` and MSRV `cargo check --workspace` steps stay as-is and test `quack-gui`'s bridge core on all four OSes without needing `webkit2gtk` or `ui/build/`. `cargo deny` stays whole-lockfile on purpose: the Tauri tree is licence-gated from the day it enters `Cargo.lock`. A new `gui` job builds the SPA, runs `svelte-check` and vitest, and runs `cargo clippy -p quack-gui --features desktop --all-targets --locked` + `cargo test -p quack-gui --features desktop --locked` on all four OSes (Linux installs the `webkit2gtk-4.1`/`gtk3` dev packages).
- `release.yml` and [14](14-packaging-and-distribution.md): **no GUI artifacts for musl, ever** (no webview on static Linux). GUI bundles (NSIS, DMG, AppImage/deb — glibc, built on the normal runners) join the release with G3, unsigned at first, and are named `quack-gui-*` so they don't disturb the existing CLI archives, `.deb`/`.rpm` and formulas. Signing and notarization follow the same plan as the CLI's pending MSI work ([14](14-packaging-and-distribution.md#still-to-do)).
- No self-updater in the GUI, matching [14](14-packaging-and-distribution.md). The CLI's first-frame budget and the TUI are untouched: `quack` builds and behaves exactly as before.

Dependency budget, counted at the spike with `cargo tree -e normal` per the usual rule: tauri + the plugins above, roughly 120–180 crates, all MIT/Apache-2.0 dual-licensed (the spike confirms `cargo deny` passes), compiled only when `--features desktop` is enabled. There is no server-side addition. npm dependencies stay inside `ui/` and lean (svelte, kit, vite, adapter-static, typescript, and optional `shiki/core`).

## Milestones

- **G0 — spike.** Hello-world Tauri build and window (`quack-gui --features desktop`) on all four CI OSes (the unknown is `windows-11-arm`), `cargo deny` over the Tauri tree, crate counts, `ts-rs` export of `quack-protocol` into `protocol.ts`, and the pure-Rust bridge core driven headless: the engine-per-tab registry with 16 ms delta coalescing against a mock webview sink and a wiremock model, including an approval round-trip.
- **G1 — desktop MVP.** `quack-gui` opens to the Welcome / Recent Folders view (or reopens the last folder), resolves the login-shell `PATH` on macOS/Linux, and finds `defaults/` across layouts (including macOS `.app` `Resources/`): first-run provider setup (`/setup`), streamed turns with markdown and highlighting, approvals with diff, questions, interrupt, session sidebar with resume (`ReplayItem` carrying `detail` and `Reasoning`), model picker and mode pill. Exit: bridge-core tests green, and a real turn run by hand on each OS.
- **G2 — multi-folder, inline actions and inspector.** Multi-folder tabs (with duplicate-session guard and clean `engine.shutdown()`), project trust gate, plan checklist + review bar (`Op::AcceptPlan`) + sub-agent view, inline message rewind (`Op::Rewind`) and tool-card undo (`Op::Undo`), Context & Stats inspector popover, `@` file mentions and image drag-drop, dark/light themes. Exit: a real task done against a local llama.cpp model.
- **G3 — Unified settings, native integration and bundles.** The tabbed Settings & Workspace dialog (*Providers & Profiles*, *Workspace*, *MCP & LSP*, *Settings & Doctor*) and `Ctrl-K`/`Cmd-K` command palette; native menus, window-state persistence, native dialogs, notifications when unfocused, clipboard, file reveal (`opener`), single instance (no system tray); NSIS, DMG and AppImage/deb bundles, unsigned at first; the `gui` CI job green on all four OSes.
- **G4 — polish and signing.** Signed and notarized GUI bundles alongside the CLI's signing story ([14](14-packaging-and-distribution.md#still-to-do)), custom theme TOML files once M3 lands them, and any remaining UX polish from daily use.

Estimated size: G1 through G3 is of the order of 4–7k lines (about a quarter Rust, the rest TypeScript/Svelte), kept small by generating `protocol.ts` and consolidating the 23 TUI modals into the four desktop surfaces above.

## Invariants checklist

- **No built-ins:** nothing GUI-side names a provider, host or command; the GUI sees only the same bundle and layers.
- **No implicit egress:** no listener, bundled assets, CSP; engines keep the egress gate.
- **Prefix stability, defaults-as-data, MSRV:** untouched (clients only; the `desktop` feature is only enabled in the `gui` job).
- **Client independence:** `quack-gui` shares nothing with `quack-tui` or `quack-acp` except the protocol.

## Open questions

- Code highlighting in the transcript: `quack_core::grammar` (`tree-sitter-highlight` over IPC, reusing compiled-in grammars and user `highlights.scm` overrides with zero JS bundle weight) versus `shiki/core` (`shiki/engine/javascript` in a web worker); settle during G1.
- Multi-window (one window per workspace) versus tabs-only; sharing one live session between the TUI and the GUI at once is out of scope.
- A browser client over a loopback server was dropped on purpose; if it is ever wanted, it re-enters behind the SPA's `Transport` module as a small axum bridge, without moving the UI.
- The daemon question in [12](12-headless-and-acp.md#open-questions) (TUI attaching to a headless engine) is unrelated to this client and stays deferred.
