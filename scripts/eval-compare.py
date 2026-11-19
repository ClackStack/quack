"""Compare two `quack eval` setups task by task, with confidence intervals.

Usage: python3 scripts/eval-compare.py A.json[,A-2.json,...] B.json[,B-2.json,...]

Each side is one setup: one or more result files (a `--repeat` run, or reruns). Only
tasks both sides ran are compared, and each task counts once however many runs it had,
so a difference is never just one side having more tasks. The intervals come from
resampling tasks (a paired bootstrap): when one includes zero, this many tasks can't tell
the setups apart. Cost compares only common tasks where every non-skipped run
on both sides reported a price.
"""

import json
import math
import random
import sys
from collections import Counter, defaultdict
from pathlib import Path

ROUNDS = 10_000


def load(spec):
    files = spec.split(",")
    reports = [json.loads(Path(f).read_text(encoding="utf-8")) for f in files]
    first = reports[0]
    name = f"{first['model']} · {first['profile']}"
    if first.get("planner"):
        name += f" · planned by {first['planner']}"
    name += f" · {first.get('suite', 'tasks')}"
    def setup(r):
        return (r["model"], r["profile"], r.get("planner"), r.get("suite", "tasks"),
                r.get("window"), r.get("sampling", {}))

    if any(setup(r) != setup(first) for r in reports[1:]):
        sys.exit(f"{spec}: these files are different setups")
    revisions = {r["revision"] for r in reports if r.get("revision") is not None}
    if len(revisions) > 1:
        sys.exit(f"{spec}: these files record different revisions")
    runs = defaultdict(list)
    for r in reports:
        for x in r["results"]:
            if not x.get("skipped"):
                runs[x["task"]].append(x)
    return name, runs, reports


def has_price(run):
    cost = run.get("cost")
    return type(cost) in (int, float) and math.isfinite(cost)


def print_report_coverage(label, reports):
    planned = [r for r in reports if type(r.get("planned_trials")) is int and r["planned_trials"] >= 0]
    if planned:
        recorded = sum(len(r["results"]) for r in planned)
        total = sum(r["planned_trials"] for r in planned)
        print(f"{label} trial records: {recorded}/{total} planned "
              f"({len(planned)}/{len(reports)} reports with planned_trials)")
    results = [x for r in reports for x in r["results"]]
    errors = Counter(str(x["error_kind"]) for x in results if x.get("error_kind") is not None)
    if errors:
        print(f"{label} execution errors: " + ", ".join(f"{kind} {n}" for kind, n in errors.most_common()))


def mean(xs):
    return sum(xs) / len(xs) if xs else 0.0


def per_task(runs, key):
    return {t: mean([float(key(x)) for x in rs]) for t, rs in runs.items()}


def interval(diffs, rng):
    """The mean of `diffs` and its 95% bootstrap interval."""
    n = len(diffs)
    means = sorted(mean([diffs[rng.randrange(n)] for _ in range(n)]) for _ in range(ROUNDS))
    return mean(diffs), means[int(0.025 * ROUNDS)], means[int(0.975 * ROUNDS) - 1]


def main(args):
    if len(args) != 2:
        sys.exit(__doc__)
    (name_a, a, reports_a), (name_b, b, reports_b) = load(args[0]), load(args[1])
    revisions = {r["revision"] for r in reports_a + reports_b if r.get("revision") is not None}
    if len(revisions) > 1:
        print("Warning: the two sides record different revisions; this is not a profile-only comparison.")
    elif revisions and any(r.get("revision") is None for r in reports_a + reports_b):
        print("Warning: some reports do not record a revision; a matched build is not established.")
    tasks = sorted(set(a) & set(b))
    if not tasks:
        sys.exit("the two sides have no task in common")
    only = sorted(set(a) ^ set(b))
    rng = random.Random(0)

    cost_tasks = {t for t in tasks if all(has_price(x) for side in (a, b) for x in side[t])}
    metrics = {
        "pass rate": lambda x: x["passed"],
        "steps": lambda x: x["steps"],
        "cost $": lambda x: x["cost"],
        "prompt k": lambda x: x["prompt_tokens"] / 1000,
    }

    def values(side):
        paired_costs = {t: rs for t, rs in side.items() if t in cost_tasks}
        return {m: per_task(paired_costs if m == "cost $" else side, f) for m, f in metrics.items()}

    va, vb = values(a), values(b)
    common_a, common_b = ([x for t in tasks for x in side[t]] for side in (a, b))
    print(f"A: {name_a} ({len(common_a)} runs)")
    print(f"B: {name_b} ({len(common_b)} runs)")
    print(f"{len(tasks)} tasks in common" + (f"; left out: {', '.join(only)}" if only else ""))
    print(f"Price coverage: A {sum(map(has_price, common_a))}/{len(common_a)}, "
          f"B {sum(map(has_price, common_b))}/{len(common_b)} (non-skipped attempts on common tasks)")
    print(f"Cost comparison: {len(cost_tasks)}/{len(tasks)} common tasks fully priced on both sides")
    print_report_coverage("A", reports_a)
    print_report_coverage("B", reports_b)
    print()
    print("| task | A pass | B pass | A steps | B steps | A cost | B cost | A failures | B failures |")
    print("|---|---|---|---|---|---|---|---|---|")

    def labels(rs):
        c = Counter(x.get("failure") or "fail" for x in rs if not x["passed"])
        return ", ".join(f"{k} {v}" if v > 1 else k for k, v in sorted(c.items())) or "-"

    for t in tasks:
        pa = sum(x["passed"] for x in a[t])
        pb = sum(x["passed"] for x in b[t])
        costs = f"{va['cost $'][t]:.4f} | {vb['cost $'][t]:.4f}" if t in cost_tasks else "n/a | n/a"
        print(
            f"| {t} | {pa}/{len(a[t])} | {pb}/{len(b[t])} "
            f"| {va['steps'][t]:.1f} | {vb['steps'][t]:.1f} "
            f"| {costs} | {labels(a[t])} | {labels(b[t])} |"
        )
    print()
    print("B - A, per task (95% interval):")
    for m in metrics:
        diffs = [vb[m][t] - va[m][t] for t in tasks if m != "cost $" or t in cost_tasks]
        if not diffs:
            print(f"  {m:<10} n/a (no fully priced common tasks)")
            continue
        d, lo, hi = interval(diffs, rng)
        scale, unit = (100, " pts") if m == "pass rate" else (1, "")
        p = {"pass rate": 1, "cost $": 4}.get(m, 2)
        clear = "clear" if lo > 0 or hi < 0 else "not clear"
        print(
            f"  {m:<10} A {mean(list(va[m].values())) * scale:9.{p}f}  "
            f"B {mean(list(vb[m].values())) * scale:9.{p}f}  "
            f"diff {d * scale:+9.{p}f}{unit} ({lo * scale:+.{p}f} .. {hi * scale:+.{p}f}): {clear}"
        )
    print()
    for label, side in (("A", a), ("B", b)):
        c = Counter(x.get("failure") or "unlabelled" for t in tasks for x in side[t]
                    if not x["passed"])
        print(f"{label} failures: " + (", ".join(f"{k} {v}" for k, v in c.most_common()) or "none"))


if __name__ == "__main__":
    main(sys.argv[1:])
