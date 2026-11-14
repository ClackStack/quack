---
name: quack desktop client
description: A terminal coding agent, in a window — field notes on paper, bound in pond water.
colors:
  paper: "#f7f3e8"
  paper-soft: "#f0ead9"
  paper-panel: "#fbf7ec"
  paper-raised: "#fffdf4"
  ink: "#2f2a20"
  ink-2: "#57503f"
  ink-3: "#6b6353"
  rule: "#b3a98e"
  rule-soft: "#d9d0ba"
  cloth: "#264457"
  cloth-deep: "#182e3d"
  cloth-card: "#1d3646"
  cloth-job: "#162a38"
  cloth-ink: "#ebf1f5"
  cloth-dim: "#c8d7e0"
  cloth-mut: "#a7becb"
  cloth-ok: "#a9d186"
  cloth-fail: "#f09c75"
  duck: "#e8b21a"
  duck-line: "#b98a06"
  duck-soft: "#f6ecd0"
  duck-ink: "#6b5410"
  duck-deep-ink: "#2b240f"
  duck-bright: "#f0c23a"
  sky: "#5e8ba3"
  sky-soft: "#e7eef1"
  sky-ink: "#35586b"
  beak: "#d07c24"
  growth-green: "#4a7031"
  growth-green-soft: "#eaf0dd"
  vermilion: "#b6401f"
  vermilion-soft: "#f7e8e0"
  plate-line: "#3c3526"
  plate-ring: "#e2d8bd"
  terminal: "#1b2431"
  terminal-text: "#d8d3c0"
  focus-ink: "#35586b"
  selection: "rgba(53, 88, 107, 0.24)"
typography:
  display:
    fontFamily: "Bitter, Georgia, 'Times New Roman', serif"
    fontSize: "27px"
    fontWeight: 600
    lineHeight: 1.15
    letterSpacing: "-0.01em"
  headline:
    fontFamily: "Bitter, Georgia, 'Times New Roman', serif"
    fontSize: "21px"
    fontWeight: 600
  title:
    fontFamily: "Bitter, Georgia, 'Times New Roman', serif"
    fontSize: "14px"
    fontWeight: 600
    lineHeight: 1.3
  prose:
    fontFamily: "Bitter, Georgia, 'Times New Roman', serif"
    fontSize: "15px"
    fontWeight: 400
    lineHeight: 1.6
  ui:
    fontFamily: "Archivo, system-ui, -apple-system, 'Segoe UI', sans-serif"
    fontSize: "13.5px"
    fontWeight: 400
    lineHeight: 1.45
  machine:
    fontFamily: "'Spline Sans Mono', ui-monospace, 'Cascadia Mono', Menlo, Consolas, monospace"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: 1.5
  label:
    fontFamily: "Archivo, system-ui, -apple-system, 'Segoe UI', sans-serif"
    fontSize: "11px"
    fontWeight: 600
    letterSpacing: "0.14em"
rounded:
  sm: "4px"
  md: "6px"
  lg: "9px"
  xl: "12px"
  full: "999px"
spacing:
  xs: "4px"
  sm: "8px"
  md: "16px"
  lg: "24px"
  xl: "32px"
components:
  button:
    backgroundColor: "{colors.paper-raised}"
    textColor: "{colors.ink}"
    rounded: "{rounded.lg}"
    padding: "5px 12px"
  button-hover:
    backgroundColor: "{colors.paper-soft}"
    textColor: "{colors.ink}"
    rounded: "{rounded.lg}"
  button-primary:
    backgroundColor: "{colors.duck}"
    textColor: "{colors.duck-deep-ink}"
    rounded: "{rounded.lg}"
    padding: "5px 12px"
  button-primary-hover:
    backgroundColor: "{colors.duck-bright}"
    textColor: "{colors.duck-deep-ink}"
    rounded: "{rounded.lg}"
  button-danger:
    backgroundColor: "{colors.paper-raised}"
    textColor: "{colors.vermilion}"
    rounded: "{rounded.lg}"
    padding: "5px 12px"
  pill:
    backgroundColor: "{colors.paper-panel}"
    textColor: "{colors.ink-2}"
    height: "28px"
    padding: "0 12px"
    rounded: "{rounded.full}"
  pill-live:
    backgroundColor: "{colors.duck-soft}"
    textColor: "{colors.duck-ink}"
    height: "28px"
    rounded: "{rounded.full}"
  specimen-plate:
    backgroundColor: "{colors.paper-raised}"
    rounded: "{rounded.xl}"
    padding: "18px 20px 14px"
  rail:
    backgroundColor: "{colors.cloth}"
    textColor: "{colors.cloth-ink}"
    width: "292px"
    padding: "16px 14px 28px"
  composer-send:
    backgroundColor: "{colors.duck}"
    textColor: "{colors.duck-deep-ink}"
    size: "34px"
    rounded: "50%"
  field-mark:
    size: "17px"
    rounded: "50%"
---

# Design System: quack desktop client

Source of truth: the built world — `crates/quack-gui/ui/src/app.css` (tokens, both themes) and `crates/quack-gui/ui/src/lib/components/*.svelte`. Dark-theme values for every token live in `.impeccable/design.json` (`extensions.colorMeta.*.dark`); the frontmatter above carries the light (day-survey) canonicals.

## Overview

**Creative North Star: "The Field Guide, Evolved"**

The project is a pond quack surveys and the user reads the findings. The course of a job reads as curated field notes on paper — not a chat feed — and every requested change arrives as a full-strength specimen plate that earns the room's only chroma. Cream paper grounds the work; a deep pond-water rail with a ripple texture binds it — the duck works on its own element; double rules and hairlines frame every edge the way a guide book frames its plates.

Three voices carry the whole surface, all self-hosted variable fonts (Tauri webview, CSP `'self'`, no external hosts): Bitter serif speaks prose and names, Archivo speaks chrome, Spline Sans Mono speaks for the machine. Chroma does one job at a time — duck yellow marks the live thing, leaf green what grows into the project, vermilion what is struck out of it, pond blue is the water everything sits on. Dark mode is the night survey: the same pond after dark, paper gone dark, inks kept in rank.

The rail answers "what is known about this project" — projects (each open project a card that contains its jobs), context, usage, files (modified), skills, plan, sub-agents. When quack asks, the log dims, a plate rises, and one decision is obvious.

### Recorded adaptations (deliberate; do not "correct" them toward the sketches)

1. **Circled numerals on diff lines are inline CSS counters.** The decision sketch drew absolutely-positioned SVG circles; the build numbers each addition/deletion with a `::before` pseudo-element driven by `counter-increment: mark` (ApprovalCard.svelte). Same visual vocabulary — a 17px circle, 1.5px `currentColor` ring, Archivo numeral — but robust to line wrapping, scrolling, and both themes. This is a code-led adaptation, not drift.
2. **The Welcome pond is the one gradient-as-illustration exception.** `Welcome.svelte` renders the logo in a radial-gradient pool (cyan tones, light and dark variants) with two ripple ellipses and a soft drop shadow. Everywhere else the world is flat paper and pond water; the pond is a deliberate first-screen flourish, the only place this device is allowed.
3. **Left-edge hairlines are 1px everywhere, and no colored edge exceeds 1px.** User notes carry a 1px vermilion left edge; field thoughts (reasoning) carry a 1px neutral rule. Chroma never thickens an edge beyond 1px anywhere in the world.

**Key Characteristics:**
- Paper field and cards on cream (#f7f3e8 family); pond-water rail with ripple texture (#264457 family)
- Double rules (3px `double`) at the header base and composer head; 1px hairlines elsewhere
- Bitter / Archivo / Spline Sans Mono — three voices, all self-hosted variable fonts
- Circled field marks number the evidence; specimen plates frame decisions with an inset double ring
- Green grows, vermilion strikes, duck yellow is only the live thing; pond blue is the water it all sits on
- Decision-focus: while a plate is up, everything else dims to opacity 0.55 / saturate 0.6
- Both themes ship: day survey and night survey; minimum viewport 720×480

## Colors

The palette is a field guide's: cream paper, deep pond water, and a handful of inks with assigned jobs. Light values below are canonical; the night-survey dark values are carried per-token in the sidecar.

### Primary
- **Duck Yellow** (#e8b21a): the live thing only — the primary action, the live job's glow, streaming cursors, pulsing status dots, the plan's "now" tag, the fresh-token bars of the usage sparkline. With its line (#b98a06), soft wash (#f6ecd0), and ink (#6b5410) for text on paper. The primary button pairs it with a fixed deep ink (#2b240f) and brightens to #f0c23a on hover.

### Secondary
- **Pond Water** (#2e4f63) with deep (#1e3a4b), ink (#dbe4e9), dim (#b2c4cd), and mut (#8fa9b5): the rail's binding and everything written on it — the duck's own element.
- **Survey Sky** (#5e8ba3) with soft (#e7eef1) and ink (#35586b): quack's own ink — the QUACK byline, waterline fill, context gauge fill, links, focus color (#35586b), selection wash, and the questions card.

### Tertiary
- **Leaf Green** (#4a7031, soft #eaf0dd): what grows into the project — added diff lines, `+` file counts, completed plan steps, tool-success markers.
- **Vermilion** (#b6401f, soft #f7e8e0): what is struck out of it — deleted diff lines, `−` counts, danger/deny/interrupt actions, the dashed pruning tick on the context gauge. It also edges the user's own notes (1px).
- **Beak Orange** (#d07c24): the streaming live-dot in the header model pill.

### Neutral
- **Cream Paper** (#f7f3e8), soft (#f0ead9), panel (#fbf7ec), raised (#fffdf4): the field and its cards.
- **Survey Inks** (#2f2a20 / #57503f / #6b6353): primary, secondary, and tertiary text on paper.
- **Rules** (#b3a98e strong, #d9d0ba soft): hairlines, borders, dividers, the styled scrollbar thumb.
- **Plate frame** (#3c3526 line, #e2d8bd ring): the specimen plate's dark outer border and pale inset ring.
- **Terminal** (#1b2431 ground, #d8d3c0 text): machine output — commands, tool stdout, file diff panes on the rail.

### Named Rules
**The Earned Chroma Rule.** Chroma does one job at a time: duck yellow is the live thing, leaf green what grows, vermilion what is struck, sky is quack's voice, pond water is the binding. Nothing decorative gets chroma; a surface earns its one accent.

**The Night Survey Rule.** Dark mode is the same pond after dark, not an inversion: paper goes to #151b23, pond water to #0d1722, and the chroma inks lighten to keep their rank (vermilion → #e08356, green → #a3c47e, duck stays #e8b21a with lightened ink #edc65a). Every token above has a recorded dark twin in the sidecar.

## Typography

**Display Font:** Bitter (with Georgia, Times New Roman fallbacks)
**Body Font:** Archivo for chrome (with system-ui fallbacks); Bitter for prose
**Label/Mono Font:** Spline Sans Mono (with ui-monospace, Cascadia Mono, Menlo, Consolas fallbacks)

**Character:** A field guide's pairing — a warm slab-ish serif for the human record, a workaday grotesque for the instrument panel, and a quiet mono for the machine's own voice. All three ship as self-hosted variable TTFs in `ui/static/fonts/` (Bitter 100–900 + italic, Archivo 100–900 at 62–125% width, Spline Sans Mono 300–700); the UI loads no external hosts (CSP `'self'`).

### Hierarchy
- **Display** (Bitter 600, 27px/1.15): the Welcome hero line.
- **Headline** (Bitter 600, 21px): the specimen plate title; full-page panel headings run 24px, section headings 17px.
- **Title** (Bitter 600, 14px/1.3): names — job titles, project names, agent names. The brand is the real duck icon (`icon.png`, 27px) beside a Bitter 700 19px wordmark at the rail's head.
- **Body** (Bitter 400, 15px/1.6): prose — assistant messages, user notes, the composer's textarea. Transcript entries cap at 76ch.
- **UI body** (Archivo 400–600, 13.5px/1.45): buttons, pills, menus, form chrome — the app's default body voice.
- **Machine** (Spline Sans Mono 400, 13px/1.5): paths, commands, diffs, model ids, token counts, keyboard hints (11px).
- **Label** (Archivo 600, 11px, +0.14em, uppercase): rail section kickers (PROJECTS, JOBS, CONTEXT, USAGE…), YOU/QUACK bylines (10.5px, +0.1em), diff plate header (10.5px, +0.08em).

### Named Rules
**The Three Voices Rule.** Bitter speaks prose and names; Archivo speaks chrome; Spline Sans Mono speaks for the machine. A voice never borrows another's job — a path never sets in Bitter, a paragraph never in Archivo.

**The Self-Contained Kit Rule.** All faces are self-hosted variable fonts loaded by `@font-face` from `/fonts/`. No webfont service, no external host, no system-display substitute.

## Layout

The workspace is a full-height two-column grid: a fixed 292px pond rail (`--sidebar-w`) running the full viewport height, and the paper main column. The main column stacks its own header (52px, `--header-h`, with its 3px waterline immediately below), transcript, decision cards, and composer — with the per-session controls (`mode`, `permissions`, `model`, `thinking`, `context`) anchored cleanly right beneath the composer box so they stay beside the input without crowding the top header. Under 900px the rail collapses behind a "Projects" menu button and overlays from the left (max-width 86vw). The minimum supported viewport is 720×480.

The main column stacks: header, transcript, decision cards, composer. Transcript entries center-lighten rather than stretch: user rows and assistant prose cap at 76ch. The composer is a paper card at the foot; the plan-ready bar and action-error strip occupy full width when present.

Spacing runs on the 4/8/16/24/32 rhythm (`--sp-xs…xl`). The rail pads 16px/14px and spaces its sections 22px apart; cards pad 10–20px. Line lengths: prose 60–76ch, panel paragraphs 65–70ch.

## Elevation & Depth

The world is flat paper. Depth comes from three devices, in order of strength: tonal steps between paper tones (field → panel → raised), hairline borders, and exactly two shadows. The strongest depth move is not a shadow at all — it is decision-focus dimming: while an approval or questions card is on the table, the transcript, composer, rail, and header drop to opacity 0.55 and saturate 0.6 over 220ms, so the plate holds the room's only full-strength ink.

### Shadow Vocabulary
- **Card weight** (`box-shadow: 0 1px 2px rgba(23, 42, 55, 0.1)`): the resting weight of paper cards — composer box, picked options, primary buttons. Night survey: `0 1px 2px rgba(0, 0, 0, 0.35)`.
- **Plate lift** (`box-shadow: 0 16px 34px -14px rgba(23, 42, 55, 0.45), 0 3px 9px rgba(23, 42, 55, 0.14)`): reserved for things that rise above the page — the specimen plate and header menus. Pond-tinted in light; near-black in dark. The Welcome pond carries its own illustrative pool shadow (the one exception, see Overview).

### Named Rules
**The Room Steps Back Rule.** When a decision is on the table, everything but the plate dims (opacity 0.55, saturate 0.6, 220ms ease-out). Never dim the plate; never light two plates at once.

**The Paper Weight Rule.** Surfaces are flat at rest. Only the two named shadows exist; a new surface either rests on a tonal step or earns the plate lift by rising over the page.

## Shapes

Corners are small and bookish: 4/6/9/12px (`--r-sm…xl`) for inputs, cards, buttons, and plates; pills, dots, and gauges close to full rounds (999px); nothing between 12px and a circle. The signature circles are the field marks — 17px numerals in a 1.5px `currentColor` ring (16px on the rail's plan) — and the composer's 34px round send button.

Edges do the framing. The header base and composer head carry a 3px `double` rule (two hairlines in one border); every other edge is a single 1px hairline in `--rule` or `--rule-soft`. The specimen plate frames itself twice: a 1px dark border (#3c3526) at 12px radius and, inset 5px, a second 1px pale ring (#e2d8bd) at 8px radius — the guide book's plate mount. The pond rail's surface is drawn as two passing ripples: 1px repeating gradients at 4px pitch, one light (0.03 alpha) and one dark (0.08 alpha), over the pond base.

### Named Rules
**The Double Rule Rule.** Double rules (3px `double`) mark the workspace's major horizontal boundaries — the header's base and the composer's head. Everywhere else, single 1px hairlines.

**The 1px Edge Rule.** Entry-edge hairlines are 1px, and no chroma-bearing edge ever exceeds 1px: the user note's vermilion left edge and the field-thoughts rule are both exactly 1px. Do not thicken a colored edge to make it louder — loudness is chroma's job, not weight's.

## Components

### Buttons
- **Shape:** bookish 9px corners (`--r-lg`), inline-flex, 5px 12px padding, 1px rule border.
- **Default:** raised paper ground (`--bg-raised`), ink text, Archivo 600; hover sinks to `--bg-soft` with an ink-3 border (120ms ease-out).
- **Primary:** duck yellow ground, deep ink text (#2b240f), duck-line border, card-weight shadow; hover brightens to #f0c23a. The guide's one bright mark.
- **Danger:** outline vermilion (text and border), wash of `--fail-soft` on hover. "Not this", "Interrupt", "Undo last" sit here.
- **Focus:** 2px solid `--focus` outline, 1px offset, 4px radius — everywhere in the app, not just buttons.

### Pills / chips
- **Style:** 26–28px full-round, raised/panel paper ground, rule border, ink-2 text, 12px Archivo 500. Used for the session control strip under the composer (`mode`, `permissions`, `model`, `thinking`, `context`) and the plan-ready chip.
- **State:** `.pill-live` (duck-soft ground, duck-line border, duck-ink text) marks the live condition — streaming model, plan mode active, context nearing. File chips in transcript rows are square-cornered (6px) mono chips.

### Cards / containers
- **Tool cards:** panel ground, rule border, 9px corners; mono summary row with a status marker dot — neutral idle, duck pulsing while running, green ok, vermilion failed. Diffs open as paper with green/vermilion `color-mix` washes (10–12%) on changed lines; plain output opens as terminal dark.
- **Questions card:** sky-soft panel with a sky border — the one blue card; option rows pick up a raised ground and card weight.
- **System notes:** hairline cards; warning notes take duck wash, errors vermilion.

### Inputs / fields
- **Style:** 1px rule border, panel ground, 6px corners (8px padding); the composer's textarea is borderless Bitter prose inside a raised paper card (12px corners, card weight) whose border shifts to sky-ink on focus-within.
- **Keyboard hints** (`⏎`, `esc`): mono 11px chips, rule border with a 2px bottom edge, soft ground; inverted (transparent, deep-ink border) when riding a duck button.

### Navigation
- **Header (chat pane):** panel ground over a 3px double rule carrying the active job title, project name, and git branch pill at the left, plus Providers and the theme toggle (a segmented paper control) at the right; below it sits the waterline (see Signature).
- **Composer controls & overlays:** directly underneath the composer's input box sits the per-session control strip (`mode`, `permissions`, `model`, `thinking` when supported, and `context` at the right) matching the TUI's status strip under the input box. Clicking `mode`, `permissions`, or `thinking` opens a compact upward-anchored popup above the pill; clicking `model` or `context` opens a centered modal dialog (`ModelModal`, `ContextModal`) over `--scrim`.
- **Rail (pond):** the water column, full viewport height, headed by the real duck icon and wordmark, followed by a two-way `[ Projects | Jobs ]` tab switcher. Uppercase Archivo kickers title each section; entries are Bitter names with mono and dim notes. The `Projects` tab opens with an `Open a project…` button at the top, followed by clickable card rows (folder icon + `›` chevron, plus concise attention status when active in the background) for inactive projects and a deep-pond card (`--cloth-card`, sky-pond border, duck active dot) for the open project that **contains its jobs** behind a 1px connector line — a Jobs kicker with New job/Refresh tools, the live job as an even deeper inset panel (`--cloth-job`, pulsing duck dot when working, `--cloth-fail` edge on errors), and saved jobs as quiet rows — plus Context, Usage, Files, Skills, Plan, and Sub-agents below. The `Jobs` tab surfaces a triage-ordered overview of every active workspace job (`input` and `error` ahead of `working` and `ready`), flagging approval/question/trust prompts and failures alongside live tool calls, sub-agents, plan progress, and inline actions (`Stop`, `Dismiss`, `Reopen`, `Close`), with a tab badge counting items that need attention. Pond-safe contrast tokens (`--cloth-ok`, `--cloth-fail`) carry additions, deletions, completed marks, issues, and pruning thresholds.

### Signature components
- **The specimen plate** (ApprovalCard): the decision surface. Raised paper, 12px corners, dark 1px border, inset pale ring (5px inset), plate lift. Bitter headline ("quack asks to change a file"), serif lead, the diff as a mounted figure with a mono path header reading "SPECIMEN PLATE". Additions and deletions are numbered by **circled field marks drawn as CSS counters** — `counter-increment: mark` on each `.line.add`/`.line.del`, rendered as a 17px `::before` circle (1.5px `currentColor` ring, Archivo numeral) at the line's left — a deliberate adaptation from the sketch's SVG circles, robust to wrapping and theme; do not "fix" it back. An italic serif tally follows ("Field marks · N additions · M deletions"), then the actions: duck primary "Approve change ⏎", optional "Allow for this session", vermilion "Not this esc", and a clock-glyph waiting line. Enter approves from anywhere; Escape denies.
- **Field marks:** the circled numerals that number evidence — plan steps (done: green-filled circle; doing: duck ring; pending: dim ring), plate diff lines, and the shared 17px `.mark` device.
- **Context gauge:** the rail's context instrument — a 12px pill track on pond, sky fill (300ms ease-out) with the used-token note beneath, and a 2px dashed vermilion tick at the pruning threshold ("Pruning starts here"). At ≥90% of threshold the fill and percentage turn duck. Expanding it reveals the segment breakdown and "Compact now".
- **Usage:** cost, turns, tokens sent, cache share, speed as a two-column mono `dl`, under it a per-request sparkline — one bar per model call, cached share in muted cloth-ink, fresh tokens in duck.
- **Files:** the job's modified files as pond-mounted entries — mono path, green `+`/vermilion `−` counts, expandable per-file diff in terminal dark, and an "Undo last" danger-outlined action.
- **The waterline:** a 3px bar under the header carrying context fill — sky normally, duck when nearing the pruning threshold (400ms ease-out), echoed by the context pill taking the live wash.
- **Decision focus:** while a plate or questions card waits, transcript, composer, rail, and header dim to 55% opacity / 0.6 saturation.

## Do's and Don'ts

### Do:
- **Do** keep every color decision inside the token sheet (`app.css` custom properties) and give any new token a dark twin in the same file — both themes ship, always.
- **Do** number diff evidence with circled field marks implemented as CSS counters on the line pseudo-element; the same vocabulary at plate, plan, and rail scale.
- **Do** keep the Welcome pond as the single gradient-as-illustration exception, with its own dark variant; gradient is illustration there and nowhere else.
- **Do** draw the pond surface with the two-ripple repeating-gradient pattern (1px ripples, 4px pitch) whenever pond water appears.
- **Do** tint diff washes with `color-mix` over transparent (≈10%) rather than solid pastels, so paper texture and themes survive.
- **Do** answer every focusable control with the 2px `--focus` outline at 1px offset.
- **Do** honor `prefers-reduced-motion` — the global override collapses animation to 0.01ms.

### Don't:
- **Don't** load fonts, images, or any asset from an external host; the webview is CSP `'self'` and the UI bundles everything (fonts under `ui/static/fonts/`, logo under `src/lib/assets/`).
- **Don't** use duck yellow as decoration or branding — it marks the live thing and the one primary action; rarity is the point.
- **Don't** let any colored edge exceed 1px, and don't thicken entry hairlines (user notes' vermilion, field thoughts' neutral rule) to raise emphasis.
- **Don't** add a third shadow, a glow, or a blur beyond the two named shadow tokens; depth is tonal steps and hairlines.
- **Don't** put machine text (paths, commands, diffs, token counts) in anything but Spline Sans Mono, or prose in anything but Bitter.
- **Don't** re-draw the duck: the brand mark is the bundled `icon.png` (`src/lib/assets/`), never an inline approximation.
- **Don't** let the rail grow past its 292px column (`--sidebar-w`) or the workspace lose its 720×480 floor.
- **Don't** rebuild the circled numerals as absolutely-positioned SVG overlays or per-line DOM nodes — the counter implementation is the recorded decision.
