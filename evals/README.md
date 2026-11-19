# evals

The task suite for `quack eval` (docs/plan/13-testing-and-evals.md#quack-eval).

```sh
quack eval -m <provider>/<model>                     # every task, the model's own profile
quack eval -m <provider>/<model> --profile qwen-code # the same tasks with another profile
quack eval --tasks fix-parser,needle --repeat 3      # a few tasks, three runs each
quack eval --suite evals/hard --repeat 3             # the hard suite (hard/README.md)
```

There are two suites. `tasks/` is quick (2–6 steps a task) and every model tested so far
passes it, so it catches regressions and compares cost. `hard/` is meant to take 10–30
steps a task and to be failed by some models, so it can tell setups apart.

Each run gets a fresh copy of the task's `repo/` as its workspace root and its own data
directory. It does not inherit the enclosing checkout's Git root, even when the cache
is inside that checkout. Runs are unattended: the approval policy is `allow`, and
anything that still asks is denied. System and user settings still apply. **Every task
calls the model, so a run costs what that model costs.**

The table goes to stdout and the JSON to `evals/results/<date>/<model>-<profile>.json`
(`--out` to change); a rerun writes `<model>-<profile>-2.json` and so on rather than
overwriting, and another suite's results start with its name
(`hard-<model>-<profile>.json`). The JSON is replaced atomically after every recorded trial.
Compare `results.length` with `planned_trials` to spot a partial report. Pass `--revision`
to record the source revision of the binary and defaults being tested. Each result includes
the resolved model, profile, reasoning effort (when set), edit format, request and tool
counts, tokens, and changed files. Execution errors also get an `error_kind` (for example,
`provider` or `timeout`), separate from the failed trial's `failure` label:

| label | meaning |
|---|---|
| `timeout`, `error` | the time ran out; the provider or engine failed |
| `bad-calls` | half or more of its tool calls didn't parse |
| `edit-failed` | edits failed, and the solution's files never all changed |
| `no-change` | the task needed changes and it made none |
| `wrong-files` / `missed-files` | none / only some of the files the solution changes |
| `wrong-answer` | only the checks on its reply failed |
| `untested` | the right files changed, it never ran a command, and the result is wrong |
| `wrong-fix` | the right files changed and the result is still wrong |

`--keep` keeps each run's workspace and session log (`data/sessions/`) and prints where.
`--stop-on-incomplete` saves the result and exits nonzero on a skip, execution error,
step limit, unknown setup, or missing trial cost; an ordinary verifier miss is still a
valid result. Estimated costs with incomplete accounting are shown separately as
`observed_cost`, not used as full trial costs. The stop flag is **not a dollar cap** and
does not stop a trial already in progress.

## Comparing two setups

```sh
python3 scripts/eval-table.py evals/results/<date>     # one row per setup, with failure labels
python3 scripts/eval-compare.py A.json,A-2.json B.json # two setups, task by task
```

`eval-compare.py` counts each task once however many runs it had, and gives each
difference (pass rate, steps, cost, prompt tokens) a 95% interval from resampling the
tasks. Only a difference whose interval leaves out zero is "clear"; one run of 14 tasks
rarely gets there, which is why `--repeat 3` matters. Cost uses only common tasks with
all recorded, non-skipped attempts priced on both sides. Check the trial and price
coverage before interpreting a difference.

## Before a paid profile pilot

1. Run the adapter and reporting unit tests and the loopback preflight. Verify that
   `generic`, explicit `zcode`, and auto selection send the expected schemas with the
   `quack-exec` mock eval tests. The suite tests must reject every starting fixture and
   accept its reference solution.
2. Pin the binary, defaults, and task revision. For Fireworks GLM-5.3 Flash, use the
   exact model ID `accounts/fireworks/models/glm-5p3-flash`. Pin the same effort, edit
   format and context window in both arms. Use isolated config and cache directories,
   explicit output files, `--keep` and `--stop-on-incomplete`. Review provider prices
   and billing controls, and set a call limit and per-request output limit before
   asking for approval. Allow for retries and any child requests; the step limit is
   not a provider-side request or billing cap.
3. After explicit approval, run one task per profile serially on Fireworks. Check
   report metadata, verifier outcomes, and provider usage before expanding. Do not
   rank a Fireworks pilot against the published Baseten results or use it to change
   defaults. A later Harbor comparison needs a profile-aware adapter and a control
   on the same route.

## A task

```
tasks/<id>/
├── task.toml     # the prompt and the checks
├── repo/         # the files the model starts with
├── hidden/       # optional: files copied in after the model is done, for the checks
└── solution/     # a reference solution: files to put in place, ANSWER, DELETE
```

```toml
category = "fix-test"        # navigate, single-edit, multi-file, fix-test, shell, create, long-context
prompt = "The tests in test_window.py fail. Fix the bug in window.py."
requires = ["python"]        # skipped unless on PATH; `python` is python3 or python
timeout_secs = 600           # the default
base = "../_projects/orderly" # optional: a project copied in first; repo/ then holds
                              # only the files this task changes (it may be left out)

# Every check must hold. Each sets one condition.
[[check]]
command = "{python} -m unittest -q test_window"   # exits 0; sh on Unix, cmd on Windows
[[check]]
file = "window.py"           # with contains / not_contains / equals
contains = "def last_n"
[[check]]
answer_contains = "42"       # the final reply, case-insensitive
[[check]]
tool_called = "shell"        # an internal tool name
[[check]]
exists = "new.py"            # or missing = "old.py"
```

`cargo test -p quack-exec --test suite` checks every task in both suites: its checks must
fail on the files as given and pass once `solution/` is applied (with `hidden/` in place
both times).

The solution also tells failures apart: a run that changed none of the files
`solution/` changes is `wrong-files`, so keep a solution to the files a fix needs.
