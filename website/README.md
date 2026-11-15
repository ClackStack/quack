# quack Marketing Website

Marketing website for `quack`: a terminal coding agent for any model and any provider, tuned for open-weight models, that contacts only what you configure.

Built with **SvelteKit 3** (Svelte 5 runes), **Tailwind CSS v4** and `@sveltejs/adapter-static`. Fonts are self-hosted from `@fontsource-variable` packages, so the page loads nothing from third-party hosts.

## The page

It is laid out as plates in a field guide, with the TUI's own ASCII duck (`_(o)>` and the animated welcome duck) and its colours (duck yellow, beak orange, water sky). Coloured thumb tabs down the right edge are the section nav. It follows the OS light or dark setting, with a toggle in the header (`mode-watcher`); colours are tokens in `src/routes/layout.css`, with the dark values under `:root.dark`.

1. **quack**: "Any model. Any provider. No leaks.", the interactive installer (defaulting to both GUI and TUI together, with standalone TUI/GUI toggles, Plucked switch for both, and direct downloads for the visitor's detected platform), and the duck with four field marks that link into the grinder.
2. **Grinder**: the same models through a generic harness and through quack; pick a failure to see what each does.
3. **In the wild**: three example sessions drawn the way the TUI draws them, including Plan + Allow proposing tests for review. Work mode and approvals appear separately in the status bar.
4. **Desktop app**: eight interactive screenshots of `quack-gui` (diff-preview edit approvals, multi-project Jobs queue, context and cache telemetry, model & planner picker, provider setup, network & sandbox boundary, shared Memory notes, and layered Settings) with Light/Dark theme switching and full-size zoom.
5. **Field marks**: the Plan/Edit × Ask/Allow matrix, then every other README feature, one line each, grouped. Mode descriptions follow [`docs/modes.md`](../docs/modes.md); keep the matrix, terminal example and configuration reference in step.
6. **Measured**: quack's own dated eval runs from `docs/plan/roadmap.md`, newest first, and the FrontierHarness Eval chart (quack has not been run on it).
7. **For teams**: what administrators need: nothing built in, host policy, locked settings, enforced hooks, `quack-plucked`.
8. **Config**: the administrator's path (`quack-plucked`, a provider file, a locked policy, `quack doctor`), what is guaranteed, where each layer lives, and every option, folded by file. Its content is in `src/lib/config.ts` and follows the repo README's Configuration section; keep the two in step.
9. **Install** and the footer.

Every claim should trace to the repo README, the roadmap or a test. Don't add figures that aren't there.

## Install scripts

The build copies `../scripts/install.sh` and `../scripts/install.ps1` into the output
(`vite.config.ts`), so `https://quack.clackstack.dev/install.sh` and `/install.ps1` serve
the same files the release publishes, and `scripts/` stays the single source.

## Development

```sh
cd website
pnpm install
pnpm dev
```

## Static Production Build

```sh
cd website
pnpm build
```

The static HTML, CSS, and JS bundle is output directly to `website/build/`.
To preview the static build:

```sh
pnpm preview
```
