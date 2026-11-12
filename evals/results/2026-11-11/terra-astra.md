# Terra addition and Astra smoke — 2026-11-11

## Terra

Added `gpt-6.1-terra` with alias `terra` to both `chatgpt.toml` and `openai.toml`,
and to the Codex profile's exact priority-50 model matches. It supports image input
and uses a 500,000-token default context budget, based on the upstream Codex catalog
at commit `e58bf8f5233b51e7cd1679d947e3028c1ed91b06`
(`codex-rs/models-manager/models.json`). That catalog distinguishes the recent
`gpt-6.1-terra` from older `gpt-5.6-terra`; the older ID is not auto-matched.
The catalog lists 872,000 as the optional maximum, not the default.

Terra API pricing was not verified and is deliberately not hard-coded. No Terra
inference was run. Availability on the user's account and actual route limits are
not proven by the upstream catalog.

The provider budget/vision, exact-match, explicit generic override, and profile
removal fallback tests now include Terra. Existing profile and prefix tests pass.

## Astra: one small live task only

The user requested a minimal smoke because Astra is expensive. Ran exactly one
`fix-typo` task on the ChatGPT-plan route, with no repeats or hard tasks:

```sh
QUACK_DEFAULTS_DIR=defaults target/debug/quack eval \
  -m chatgpt/astra --profile auto --tasks fix-typo \
  --out evals/results/2026-11-11/chatgpt-astra-codex-smoke.json
```

Automatic profile selection resolved to **`codex`**. The task passed, changing only
the fixture's `README.md`.

| Metric | Result |
|---|---:|
| Passed | 1/1 |
| Model requests | 5 |
| Tool calls | 4 |
| Tool errors | 1 |
| Argument parse failures | 0 |
| Edit failures | 0 |
| Prompt tokens | 19,595 |
| Cached prompt tokens | 2,944 |
| Completion tokens | 190 |
| Time | 14.96 s |
| Compactions / pruned messages | 0 / 0 |

[Raw result](chatgpt-astra-codex-smoke.json). The route uses plan allowance;
dollar cost is unavailable, **not zero**. No separately billed API calls were made.
This validates a short tool/edit turn with the expanded Codex profile and explicit
encrypted-reasoning include, not Astra's comparative performance, long-window,
image, subagent or resume behavior. Luna and Terra remain untested live.
