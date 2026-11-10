# quack-gui UI

The desktop SPA (SvelteKit 3, Svelte 5 runes, adapter-static SPA mode) that the Tauri
shell in `..` serves from `build/` — docs/plan/15-desktop-client.md.

```sh
npm ci
npm run dev        # vite dev server; runs against the scripted dev transport
npm run check      # svelte-check
npm test           # vitest
npm run build      # static bundle into build/ (what tauri.conf.json embeds)
```

Without the Tauri shell (`npm run dev` in a plain browser) the UI drives a scripted
demo session through the same `Transport` seam — nothing renders that the real
bridge does not send.

Layout:

- `src/lib/protocol.ts` — generated from `crates/quack-protocol`; regenerate with
  `cargo test -p quack-protocol --features ts export`, never edit by hand.
- `src/lib/transport/` — the one seam between UI and engines: `tauri.ts` (IPC),
  `dev.ts` (scripted demo), `types.ts` (the bridge envelope).
- `src/lib/session.svelte.ts` — one tab's view of one engine: the `Event` reducer and
  the `Op` senders.
- `src/lib/components/` — Welcome, workspace shell (header, sidebar, transcript,
  composer), approvals, questions, tool cards.
