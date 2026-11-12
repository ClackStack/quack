# ChatGPT Sol profile evaluation — 2026-11-11

## Scope and decision

Live calls used the user's saved ChatGPT-plan sign-in and `chatgpt/sol` (the shipped
alias for `gpt-6.1-sol`), not the separately billed `openai` provider. Sampling stayed
at server defaults. Calls consume plan allowance; no monetary cost or remaining
allowance is reported by this route. Missing cost must **not** be read as $0.

**Initial recommendation: keep `generic` as the automatic fallback.** The repaired `codex` profile works and
is a reasonable opt-in, but this targeted sample is not enough to enable matching
under the usual promotion rule. **Follow-up:** the user subsequently approved Codex
defaults for Sol, Astra and Luna after an [upstream review](codex-comparison.md);
that decision is an explicit exception, not additional evaluation evidence.
Only CSV export has three completed repeats per profile in the saved comparison;
the other three tasks have one each. Other OpenAI models were not tested.

## Harness bug found and fixed

The exclusive `codex` profile offered shell reads and `apply_patch`, but no `read`.
The patch tool requires a tracked read, which `cat` and `sed` do not register. Two
completed CSV attempts failed after repeated "has not been read yet" errors; a
third was interrupted. This was a harness/profile bug, not failure to identify the
required fix.

The profile now exposes `read` and explicitly requires it before patching existing
files. Exclusive tools, read-before-write and stale-read protection remain intact.
A mock engine regression verifies refusal before `read` and success afterward.

## Saved post-fix comparison

| Task | Generic pass | Codex pass | Generic mean steps | Codex mean steps |
|---|---:|---:|---:|---:|
| csv-export | 3/3 | 3/3 | 10 | 13 |
| log-parser | 1/1 | 1/1 | 12 | 5 |
| migrate-fetch | 1/1 | 1/1 | 17 | 11 |
| partial-shipment | 1/1 | 1/1 | 27 | 15 |

| Metric (all six runs per profile) | Generic | Codex |
|---|---:|---:|
| Passed | 6/6 | 6/6 |
| Model requests | 86 | 70 |
| Tool calls | 121 | 109 |
| Tool errors | 4 | 9 |
| Argument parse failures | 0 | 0 |
| Edit failures | 2 | 0 |
| Prompt tokens | 757,234 | 504,207 |
| Cached prompt tokens | 151,168 | 24,704 |
| Completion tokens | 12,750 | 14,895 |
| Total task time | 551 s | 493 s |

`eval-compare.py` weights tasks equally: steps 16.5 vs 11.0, difference -5.5
(95% interval -10.5 to +0.5; not clear). Prompt tokens 153.28k vs 80.85k,
difference -72.42k (95% interval -162.88k to -1.06k). Four tasks and mostly single
repeats limit generalization. Tool errors include missing `rg` (the model can fall
back to `grep`) and nonzero shell checks; they are not necessarily malformed calls.
The generic migration run recovered from its two edit failures and passed.

Reproduce the comparison without new inference:

```sh
python3 scripts/eval-compare.py \
  evals/results/2026-11-11/hard-chatgpt-sol-generic-csv.json,evals/results/2026-11-11/hard-chatgpt-sol-generic-other.json \
  evals/results/2026-11-11/hard-chatgpt-sol-codex-tracked-read-csv.json,evals/results/2026-11-11/hard-chatgpt-sol-codex-tracked-read-other.json
```

Caution: the current comparison script prints unknown costs as zero; the directory
table also does not aggregate split files for the same setup. Use the explicit
file lists above and treat dollar figures as unavailable.

## Run accounting and interrupted batches

24 task attempts total, within the approved maximum of 25:

- Initial `fix-typo` smoke: 1/1 passed, 5 requests, no parse/edit failures.
- Initial long generic batch: seven completed passes (CSV three times, log parser
  three times, migration once), then one interrupted migration attempt.
- Initial long codex batch: two completed CSV failures, then one interrupted CSV
  attempt. Do not interpret the interrupted attempts as model timeouts or failures.
- Saved comparison after the profile fix: 12/12 passed as detailed above.

The execution tool killed both initial long batches at its 600-second command
limit, before eval wrote aggregate JSON. Their partial stdout is in the ignored
`hard-chatgpt-sol-*-targeted.log` files; session artifacts were inspected to diagnose
the bug. No incomplete batch is included in the statistical comparison.

Kept scratch roots (local, not portable): `/home/paul/.cache/quack/eval/7860`
(smoke), `10566` (initial generic), `11050` (initial codex), `13381` (fixed codex
CSV), `13493` (generic CSV), `13802` (fixed codex other), `14292` (generic other).
Credentials and encrypted reasoning are not copied into this report.

## Compatibility evidence and limits

Successful multi-step live turns exercised plan authentication, Responses streaming,
namespaced tool calls/results, tracked reads, edits/patches, shell commands, planning,
and hidden-test verification. Post-fix session logs contain sealed encrypted reasoning
and both `commentary` and `final_answer` phases. Subsequent requests succeeded;
wire-level replay details remain covered by mock tests, not captured live requests.
No live auth/protocol errors appeared in the saved comparison.

Mock checks passed: provider login (5), Responses unit tests (6), provider mock-server
tests (6), eval integration (3), shipped-profile engine tests (5, including the new
tracked-read regression), and prefix stability (3).

Not established by these runs: other OpenAI model IDs, API-key billing route, fresh
browser consent or revocation against the live issuer, resume fidelity on the live
plan route, image input, long-window behavior, pruning/compaction, maximum output,
or sustained rate-limit behavior. Neither profile compacted or pruned in the saved
comparison. Do not treat this as blanket certification of all current OpenAI models.
