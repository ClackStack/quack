# Terra edit-format comparison, 2026-11-18

Keep `str_replace` as the generic default and `apply_patch` in Terra's Codex profile.
`apply_patch` took the fewest requests in this pilot. Hashline passed all the same
checks and used 23% more prompt tokens than `str_replace`. A wider, repeated comparison
would be needed to justify a generic default change.

## Setup

`chatgpt/terra` (`gpt-6.1-terra`) used the ChatGPT plan at `medium` reasoning effort.
Each format used `--profile generic` with `-c tools.edit_format=<format>` and one
fresh `quack eval` session per task. The comparison includes the checkout's batched
exact `str_replace` implementation and the Responses `strict: false` fix. The three
formats ran concurrently. Both smoke runs and the comparison consumed 104 successful
inference requests in total, including requests from the evals' explore agents.
Dollar cost is unavailable on the plan route.

```sh
QUACK_DEFAULTS_DIR="$PWD/defaults" target/debug/quack eval \
  -m chatgpt/terra --profile generic -c tools.edit_format=str_replace \
  --suite evals/hard --tasks csv-export,log-parser,cancel-restock
```

Repeat with `--suite evals/tasks --tasks big-file-edit` and with `hashline` and
`apply_patch`. The original invocation ran `cancel-restock` separately from the
other hard tasks.

## Results

| Format | Passed | Requests | Prompt tokens | Cached prompt | Completion tokens | Rejected edits |
|---|---:|---:|---:|---:|---:|---:|
| `str_replace` | 4/4 | 28 | 163,233 | 29,568 | 3,937 | 0 |
| `hashline` | 4/4 | 32 | 200,859 | 39,296 | 3,727 | 0 |
| `apply_patch` | 4/4 | 26 | 144,204 | 10,624 | 3,603 | 0 |

All runs had zero argument parse failures and passed the fixture checks, including
hidden tests where supplied. Figures count all model requests. Cache hit differences
left `apply_patch` and `str_replace` at almost identical uncached prompt totals
(133,580 and 133,665). Hashline used 161,563 uncached prompt tokens. Four Python
fixtures with one run per format provide a directional result. The models chose
their own reads, plan updates, and verification commands, so request and token
totals describe the whole turn.

| Task | `str_replace` requests | `hashline` requests | `apply_patch` requests | What the trajectory showed |
|---|---:|---:|---:|---|
| `csv-export` | 10 | 11 | 9 | Hashline read more inputs and edited an extra test file. |
| `log-parser` | 4 | 4 | 5 | `str_replace` and hashline each sent seven changes in one edit call. The patch trajectory read `sample.log` too. |
| `cancel-restock` | 10 | 12 | 8 | `apply_patch` updated all three files in one call. Each other format took three edit calls. |
| `big-file-edit` | 4 | 5 | 4 | All applied on the first edit. `str_replace` and patch used `read(symbol=...)`. Hashline used a search and range read. |

Raw results for `str_replace` ([hard](str_replace-hard.json), [multi-file](str_replace-multi.json), [large file](str_replace-large.json)),
`hashline` ([hard](hashline-hard.json), [multi-file](hashline-multi.json), [large file](hashline-large.json)),
and `apply_patch` ([hard](apply_patch-hard.json), [multi-file](apply_patch-multi.json), [large file](apply_patch-large.json)).

## Responses argument regression

On `fix-typo`, Terra sent `edits` along with `old_string: ""`, `new_string: ""`,
and `replace_all: false` on nine attempts. Quack rejected the mixed forms every
time. The task passed in 13 requests after a shell rewrite. Responses attempts to
normalize function schemas into strict mode when `strict` is omitted. The adapter
now explicitly sets `strict: false` on both Responses routes. In the follow-up Terra
sent just `path` and `edits`, applied both replacements in one call, and passed in
five requests with zero edit failures. The provider tests cover the wire field on
both routes. [Before](smoke-before.json), [after](smoke-after.json),
[OpenAI function-calling docs](https://developers.openai.com/api/docs/guides/function-calling#strict-mode).

## Targeting defects reproduced locally

The examples below exercised the actual tools with temporary files and no model calls.

- A valid hashline tag proves the snapshot version. A line number one off can still
  land on a different, visible line. Replacing line 1 of `low = 10\nhigh = 20\n`
  with `high = 90` succeeded and left two assignments to `high`. The syntax check
  accepts both. The original published hashline experiment paired a hash with
  each cited line, and its biggest reported gain was against its patch baseline.
  Quack's current mode uses one tag per file. [Original hashline experiment](https://stencil.so/blog/the-harness-problem).
- An outline-only `read` records every file line as shown and omits the tag.
  A hashline replacement with no tag changed the unseen body of a Python function
  immediately after its outline was shown. The same guard also excludes insertions.
  An insertion after line 1 of an unread file succeeded with an unknown tag.
  See `read.rs` and the coverage check in `hashline.rs`.
- A patch hunk with a missing `@@` anchor can fall back to the first matching
  context. With two functions containing `return 20`, a misspelling in the second
  function's anchor changed the first function. See `seek` and `apply_hunks` in
  `patch.rs`. Requiring unique hunk context when an anchor fails would close this
  case while allowing uniquely matched patches to recover.

## Candidate for the next comparison

Add optional symbol scope to `edit` using the Tree-sitter outline already used by
`read(symbol=...)`. A call naming `Order::cancel` could match a short `old_string`
once inside that method even when the same text appears elsewhere in the file.
Resolve the symbol uniquely, keep the read and stale checks, and validate all batch
matches before writing. This keeps a content check and saves the context that models
currently copy to disambiguate common snippets. For larger changes, a related
candidate replaces the body of a uniquely identified function by name. A published
29-task Python experiment found strong results for that approach. The proposed
scoped snippet variant still needs evaluation in Quack. [AST editing benchmark](https://geometricagi.github.io/2026/04/02/ast-edits.html).

Compare first-edit correctness, unintended changed regions, edit retries, requests,
and uncached prompt tokens. Include repeated snippets in different symbols, long
single-function files, and multi-file changes. Fail the task if the intended edit
lands in the wrong symbol even when the file still parses.
