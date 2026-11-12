# OpenAI Codex source comparison — 2026-11-11

## Decision

Following the live [Sol evaluation](README.md), the user explicitly chose `codex` as
the automatic profile for **three model IDs initially**: `gpt-6.1-sol`,
`gpt-6-astra`, and `gpt-6-luna`. Matching is exact at priority 50, works independently
of provider name, and does not match arbitrary GPT models, bare aliases or gpt-oss.
The normal model resolution supplies the full ID when using `chatgpt/sol` or
`openai/sol`. Explicit `--profile generic` or a model's `profile = "generic"`
still overrides automatic selection. Removing the Codex profile restores the generic
fallback rather than leaving a mandatory missing-profile dependency.

This is an **explicit user-approved product decision**, not a claim that the usual
three-repeat promotion rule was satisfied. Only Sol on the ChatGPT-plan route has
live results. Astra, Luna and the API-key route remain unmeasured. The profile/tool
and request changes below have mock coverage but have not been re-evaluated live.
No inference calls were made for this source review.

**Subsequent follow-up:** [Terra was added and one Astra smoke task passed](terra-astra.md).
Terra uses its upstream 500k default budget and the same exact Codex matching.
The measurement limits below describe the state at this review; Astra now has a
short live pass, not a comparative evaluation. Luna/Terra remain untested live.

## Upstream source

Repository: https://github.com/openai/codex

Pinned commit: [`e58bf8f5233b51e7cd1679d947e3028c1ed91b06`](https://github.com/openai/codex/tree/e58bf8f5233b51e7cd1679d947e3028c1ed91b06),
committed 2026-11-11. Source was downloaded under ignored `scratch/upstream-codex/`
and inspected, not executed. References below are relative to upstream `codex-rs/`.
No upstream prompts were copied; quack's prompt text remains original.

## Changes made from the comparison

| Area | Upstream evidence | Quack change |
|---|---|---|
| Stateless reasoning | `core/src/client.rs:1258` requests `reasoning.encrypted_content` | Responses requests now explicitly include encrypted reasoning on both API and ChatGPT-plan routes; existing seals/phase replay remain intact. `extra_body` can override this for compatible servers. |
| Vision | `models-manager/models.json` current entries advertise image input; `core/src/tools/handlers/view_image_spec.rs` exposes viewing | Enable `vision = true` in both shipped provider files and expose the existing vision-gated `view_image` tool in the Codex profile. |
| Delegation | Sol selects multi-agent v2 in `models-manager/models.json`; `core/src/tools/handlers/multi_agents_spec.rs` defines agent tools | Expose quack's existing synchronous `task`, with honest semantics rather than aliases for upstream's asynchronous agent handles. |
| Context budgeting | All three catalog entries declare `context_window = 272000`, `max_context_window = 872000` (Sol at `models-manager/models.json:206–207`) | Replace the unsupported 1,050,000-token assumption with a conservative 272,000-token configured window in both provider files. This is an upstream Codex budget, not a live measurement of public API limits. Larger windows remain explicit overrides after route/account verification. |
| Shell contract | `core/src/tools/handlers/shell_spec.rs` distinguishes execution from session input | Clarify quack's one-shot argv shell, no stdin/persistent session, configured timeout and sandbox. Add fallback guidance when `rg` is absent. No fake session support. |
| Default selection | User chose all three, informed by Sol's 6/6 results and lower prompt tokens/requests | Exact ID matches only; gpt-oss stays opt-in. |

Reasoning effort and sampling remain at existing configured/server defaults rather
than copying upstream tuning without measurement. Provider URLs, sign-in and prices
were not changed. No config struct changed, so schema regeneration is unnecessary.

## Deliberate differences / remaining gaps

### Responses Lite and code execution

Upstream marks these models `use_responses_lite: true` and
`tool_mode: "code_mode_only"` in `models-manager/models.json`. Its client moves tools
into developer-role `additional_tools` input items, places instructions in input,
and sets Lite-specific request metadata and parallel-tool behavior
(`core/src/client.rs:1198–1224,1295–1305,2720–2725`).

Quack uses the ordinary Responses function-tool contract that worked in the live
Sol evaluation. It does **not** implement Lite or the upstream JavaScript code-mode
runtime. This is a substantial harness difference, not proof that quack's tested
route is broken. Enabling Lite without verifying third-party endpoint support or
implementing its execution contract would be an unsafe assumption. Treat a future
Lite/code-mode adapter as a separate, mock-tested and live-evaluated project.

### Authentication contract

Upstream's Codex client uses a registered Codex OAuth client, the Codex backend,
and optionally a `ChatGPT-Account-ID` header
(`login/src/auth/manager.rs`, `login/src/server.rs`,
`model-provider-info/src/lib.rs:472–490`,
`model-provider/src/bearer_auth_provider.rs:41–44`).

Quack uses its separate direct-plan third-party sign-in contract and configured
public Responses endpoint, which succeeded in our live runs. Upstream source does
not validate that contract, and its client identity/endpoints must not be blindly
borrowed. No auth migration was attempted.

### Freeform patches, persistent shells, transport and discovery

- Upstream uses a grammar-backed **custom/freeform** patch tool
  (`core/src/tools/handlers/apply_patch_spec.rs`). Quack uses the same patch family
  through an advertised JSON function tool and requires tracked reads. The existing
  Sol run parsed it correctly; no unsupported raw-patch instructions were added.
- Upstream has `exec_command` / `write_stdin` sessions; quack's shell has no stdin,
  background session handle or polling API. A real implementation needs lifecycle,
  cancellation, sandbox and resource-management work.
- Upstream prefers WebSockets; quack uses HTTP SSE. This is a latency/performance
  opportunity, not demonstrated correctness failure.
- Upstream can request encrypted output replay, cache TTL, verbosity and service
  tiers; quack does not expose all of these as dedicated controls. Existing cache
  keys and reasoning/phase replay are not missing.
- Upstream's catalog carries Lite/tool-mode metadata, context limits, supported
  efforts and plan availability. Quack's general model discovery does not consume
  that full catalog. The static provider entries now use conservative budgets;
  account/route-specific capability discovery is still future work.
- General parallel filesystem reads/searches and upstream's multi-agent v2 API are
  not reproduced. Quack's bounded read-only subagent execution remains available.

## Validation

Mock regression coverage verifies exact model matching, explicit generic override,
Codex removal fallback, the shipped provider budgets/vision flags, `task` exposure,
vision gating, argv-shell mapping, unread-patch refusal followed by tracked-read
success, encrypted reasoning inclusion/override, and sealed replay on both routes.
Prefix stability is checked for every shipped profile. All **74 relevant tests**
passed: core profiles (9), prefix (3), invariants (5), provider units (46), login
mocks (5), and provider mock-server tests (6). The CLI build, formatting, diff
whitespace check, and core/provider all-targets Clippy with warnings denied passed.
These checks do not replace live Astra/Luna, image, subagent, resume or long-window
validation.
