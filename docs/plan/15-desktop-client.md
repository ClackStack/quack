# 15 — Desktop client (Tauri + SvelteKit)

Decision (2026-11-08): the desktop GUI is **Tauri 2 + SvelteKit (Svelte 5)**, and the app *is* the client: Tauri's own IPC carries the protocol, so there is no `quack serve`, no port, no token and no browser mode. Launching `quack-gui` opens the window and goes; `quack-cli` is untouched. The engine does not change: a GUI is a third client of the Op/Event protocol ([00](00-architecture.md#engine-protocol)), like the TUI, `exec` and ACP ([12](12-headless-and-acp.md)). **No new `Op` or `Event` is required** — turns and streaming deltas, approvals and questions, project trust, plan mode and plan accept, rewind, stats and context reports, sub-agents, and sessions (`Op::ListSessions` → `Event::SessionList`, resume replay through `Event::SessionResumed`) all exist.

A browser target was considered first (a loopback `quack serve` hosting the same SPA, for headless servers over SSH) and dropped the same day: it added a server, an auth surface and the axum dependency tree for a path nobody asked for. The SPA keeps its transport behind one small module, so a browser client could be added later without moving the UI.

Why this stack (investigated 2026-11-08): what a GUI must rebuild is the TUI's roughly 8,000 lines of rendering — markdown, diffs, highlighting, charts, 23 modals — and web tech replaces most of that with mature parts, while desktop integration (menus, tray, dialogs, notifications, clipboard, file associations) is Tauri plugins instead of per-platform code. The Rust side stays substantial: the bridge, engines and packaging. The webview holds only views.

| Stack | Verdict |
|---|---|
| Tauri 2 + SvelteKit | **chosen.** Healthiest upstream (2.12 stable, Apache-2.0/MIT, MSRV below quack's), the user knows Svelte, Windows/macOS/Linux via the OS webview |
| gpui (+ gpui-component) | viable but costlier: crates.io packaging still pin-a-git-rev, no musl GUI builds upstream, the rendering rewrite stays, and Zed's own `ui`/`markdown` crates are GPL-3.0 and must not be used |
| egui, iced | weaker fallbacks if webviews are ever a hard no (immediate-mode ergonomics; one release in twelve months) |
| Slint | disqualified: GPL-3.0 or a paid licence, fails the `cargo deny` Apache-only gate |

## Layout

| Piece | Where | What |
|---|---|---|
| `quack-gui` (Rust side) | `crates/quack-gui` (workspace member, own binary) | The Tauri 2 app. An engine-per-tab registry; `#[tauri::command]` handlers (`open_folder`, `send_op`, `close_tab`, …); streams engine `Event`s to the webview over a per-tab `tauri::ipc::Channel`. Builds the engine-spawning closure itself, the way `quack-cli` builds ACP's (`quack_acp::Spawn`, `quack-acp/src/lib.rs:40`) |
| the SPA | `crates/quack-gui/ui/` | SvelteKit 3 + Svelte 5 runes app, built as a static bundle (`@sveltejs/adapter-static`, SPA mode: `fallback: 'index.html'`, output `build/`), embedded via `frontendDist`. All engine traffic goes through one `Transport` module over Tauri IPC (`invoke` + channel listener) |

No server component exists. `quack-gui` depends on `quack-core`/`quack-config` (it spawns engines) and never on another client crate: it mirrors the TUI, it does not import it. Since it depends on Tauri, the workspace-wide CI commands exclude it; nothing about the default `cargo build` or the `quack` binary changes.

## The bridge

One `Channel` per tab, created by the frontend and handed to `open_folder`; commands carry a tab id:

```
invoke("open_folder", { cwd, options, events })  → tab id
invoke("send_op",    { tab, op })                // op is the tagged JSON, parsed with serde_json into quack_protocol::Op
events.listen(…)                                 // {"tab":…,"event":{"event":"assistant_delta",…}} or {"tab":…,"serve":{"type":"tab_closed",…}}
```

- `send_op` accepts the `Op`'s serde JSON as-is (`#[serde(tag = "op")]`, `snake_case`); the TS side holds hand-written discriminated unions of the same shape. No translation layer beyond the tab envelope.
- Opening a tab names a folder; the Rust side loads that folder's config and spawns its engine (the ACP pattern). Tabs are independent engines; closing a tab drops the handle, and sessions are already crash-safe on disk. An engine that dies produces a `tab_closed` control message with the reason.
- Backpressure is the client's job, as with every client: the engine's event channel is bounded, and a slow webview gets `AssistantDelta`s coalesced (a transcript only needs the final text). `Channel` delivery stays ordered.
- Images come from native dialogs and drag-drop, so they carry real paths into `InputItem::Image` — no upload path is needed.
- Dev mode uses Tauri's `devUrl` against the vite dev server: hot reload, IPC working, standard workflow.
- TypeScript types are kept honest by fixtures: a Rust test serializes at least one of every `Op`/`Event` variant to JSON; a vitest test parses each fixture into the TS types. If the fixture burden grates, ts-rs behind a feature flag is the fallback.

### Security

- There is no socket: nothing listens, so no port, token, origin or DNS-rebinding story. Tauri IPC is reachable only from the app's own webviews.
- CSP stays minimal (Tauri injects what its IPC needs; everything else is `'self'`), assets are bundled — the UI loads nothing from the network. `withGlobalTauri: false`, least-privilege capabilities (dialog, clipboard-manager, notification, opener for revealing files, single-instance), no arbitrary shell.
- No egress change: the engines keep the egress gate, `network.offline` and `[policy.providers] hosts` exactly as every other client does.

## The UI

- One window, tabs inside it (a tab is a folder + session). A command palette (Ctrl-K) over the command catalogue covers what has no dedicated control yet.
- Transcript: markdown rendered client-side (Svelte component over the message text), code highlighted with Shiki (bundled locally, heavy highlighting in a worker), diffs rendered from the tool-call data the TUI renders. Themes: the defaults bundle's `themes/` files are data, so the SPA reads the active theme into CSS variables rather than growing a second theme system.
- Approvals render the diff the engine already sends; questions are dialogs; trust is a full-screen gate before anything in the folder starts (the engine owns this; the UI only presents it).
- Panels for `/context` and `/stats` draw SVG charts from the same data the TUI's `charts.rs` draws; no engine change. Sidebar lists sessions per folder (`Op::ListSessions`), the model picker rides `Op::ListModels`, and the profile picker rides `Op::ListProfiles`/`Op::SetProfile`.
- Settings and commands that mutate config (`/config set`, key replacement, permissions rules) use the same config-writing paths the TUI uses client-side (`quack-config`, `toml_edit`); where the TUI does it client-side today, the GUI does the same, and an `Op` is added only if a write turns out to need engine state.

## CI and release

- `ci.yml`: the workspace-wide steps gain `--exclude quack-gui` (clippy, test, and the MSRV `cargo check --workspace`, which otherwise needs the frontend dist present). `cargo deny` stays whole-lockfile on purpose: the Tauri tree is licence-gated from the day it enters `Cargo.lock`. A new `gui` job builds the SPA and runs `cargo clippy -p quack-gui` + its tests on all four OSes (Linux needs the webkit2gtk/gtk dev packages), with `svelte-check` and vitest on Linux only.
- `release.yml` and [14](14-packaging-and-distribution.md): **no GUI artifacts for musl, ever** (no webview on static Linux). GUI bundles (NSIS, DMG, AppImage/deb — glibc, built on the normal runners) join the release with the shell milestone, unsigned at first, and are named `quack-gui-*` so they don't disturb the existing archives, `.deb`/`.rpm` and formulas. Signing and notarization follow the same plan as the CLI's pending MSI work ([14](14-packaging-and-distribution.md#still-to-do)).
- No self-updater in the GUI, matching [14](14-packaging-and-distribution.md). The CLI's first-frame budget and the TUI are untouched: `quack` builds and behaves exactly as before.

Dependency budget, counted at the spike with `cargo tree -e normal` per the usual rule: tauri + the plugins above, roughly 120–180 crates, all MIT/Apache-2.0 dual-licensed (the spike confirms `cargo deny` passes), entering only the `gui` job's build. There is no server-side addition. npm dependencies stay inside `ui/` and lean (svelte, kit, vite, adapter-static, shiki, typescript).

## Milestones

- **G0 — spike.** Hello-world Tauri build and window on all four CI OSes (the unknown is `windows-11-arm`), `cargo deny` over the Tauri tree, crate counts, and the bridge core driven headless: the engine-per-tab registry against a mock webview sink and a wiremock model, including an approval round-trip, proving the command layer is testable without a window.
- **G1 — desktop MVP.** `quack-gui` opens, one folder, one session: streamed turns with markdown and highlighting, approvals with diff, questions, interrupt, resume-in-folder and the model picker. Exit: bridge-core tests green, and a real turn run by hand on each OS.
- **G2 — multi-folder and panels.** Tabs, trust gate, plan accept, `/context` and `/stats` panels, profile picker, images from dialogs and drag-drop, theme variables. Exit: a real task done against a local llama.cpp model.
- **G3 — native integration and bundles.** Menus, tray, native dialogs, notifications when unfocused, clipboard, drag-drop polish, single instance; NSIS, DMG and AppImage/deb bundles, unsigned at first; the `gui` CI job green on all four OSes.
- **G4 — parity and packaging.** The remaining commands become panels and palette entries (`/config`, `/permissions`, `/memory`, `/skills`, `/mcp`, `/lsp`, `/doctor`, `/export`, `/fork`, `/undo`, `/rewind`, `/key`, …), then signed bundles if the CLI's signing story lands first.

Estimated size: the MVP through G3 is of the order of 5–9k lines (about a quarter Rust, the rest TypeScript/Svelte); full parity is a quarter-sized tail, mostly panels.

## Invariants checklist

- **No built-ins:** nothing GUI-side names a provider, host or command; the GUI sees only the same bundle and layers.
- **No implicit egress:** no listener, bundled assets, CSP; engines keep the egress gate.
- **Prefix stability, defaults-as-data, MSRV:** untouched (clients only; `quack-gui` is excluded from the workspace-wide and MSRV jobs).
- **Client independence:** `quack-gui` shares nothing with `quack-tui` or `quack-acp` except the protocol.

## Open questions

- Hand-written TS types with fixtures versus ts-rs codegen; settle after the first schema pass.
- Multi-window (one window per tab) versus tabs-only; sharing one session between the TUI and the GUI at once is out of scope.
- A browser client over a loopback server was dropped on purpose; if it is ever wanted, it re-enters behind the SPA's `Transport` module as a small axum bridge, without moving the UI.
- The daemon question in [12](12-headless-and-acp.md#open-questions) (TUI attaching to a headless engine) is unrelated to this client and stays deferred.
