# quack-gui UI

The desktop SPA (SvelteKit 3, Svelte 5 runes, adapter-static SPA mode) that the Tauri
shell in `..` serves from `build/` — docs/plan/15-desktop-client.md.

```sh
pnpm install
pnpm dev          # vite dev server; runs against the scripted dev transport
pnpm check        # svelte-check
pnpm test         # vitest
pnpm build        # static bundle into build/ (what tauri.conf.json embeds)
```

Without the Tauri shell (`pnpm dev` in a plain browser) the UI drives a scripted
demo session through the same `Transport` seam — nothing renders that the real
bridge does not send. URL parameters steer it: `?theme=dark|light` previews one
theme without touching the saved choice, and `?demo=turn[&auto=1]` opens the
demo folder and plays a full turn (`auto=1` also answers its approval, so a
headless capture can reach the settled state).

Design: the field-guide system — tokens and shared control styles live in
`src/app.css`; `crates/quack-gui/DESIGN.md` records the durable decisions.
Variable fonts (Bitter, Archivo, Spline Sans Mono) are bundled under
`static/fonts/` and loaded with `@font-face`; the UI loads no external hosts.

Layout:

- `src/lib/protocol.ts` — generated from `crates/quack-protocol`; regenerate with
  `cargo test -p quack-protocol --features ts export`, never edit by hand.
- `src/lib/transport/` — the one seam between UI and engines: `tauri.ts` (IPC),
  `dev.ts` (scripted demo), `types.ts` (the bridge envelope).
- `src/lib/session.svelte.ts` — one tab's view of one engine: the `Event` reducer and
  the `Op` senders. `editedFiles` derives the sidebar's specimen case from the
  transcript's edit and write calls.
- `src/lib/components/` — Welcome, workspace shell (header, rail sidebar with
  projects/jobs/context/usage/files/skills/plan/sub-agents, transcript, composer with
  session control strip), `ModelModal`, `ContextModal`, approvals, questions, tool cards.
