"""Read retained quack usage once, including agents, summaries and interrupted calls."""

import json
from pathlib import Path


def sessions(logs_dir: Path):
    sessions = Path(logs_dir) / "quack-data" / "sessions"
    for path in sorted(sessions.glob("*/*/events.jsonl")):
        events = []
        for line in path.read_text(errors="replace").splitlines():
            try:
                rec = json.loads(line)
            except ValueError:
                continue
            if not isinstance(rec, dict):
                continue
            events.append(rec)
        yield events


def records(logs_dir: Path) -> list[dict]:
    calls = []
    # Sub-agent usage is mirrored into events.jsonl; reading agents/*.jsonl doubles it.
    for events in sessions(logs_dir):
        pending = {}
        for rec in events:
            kind = rec.get("t")
            key = rec.get("agent") if kind in ("agent_request_start", "agent_usage") else None
            if kind in ("request_start", "agent_request_start"):
                pending[key] = len(calls)
                calls.append({"agent": key, "usage": None})
            elif kind in ("usage", "agent_usage"):
                index = pending.pop(key, None)
                if index is None:
                    calls.append(rec)
                else:
                    calls[index] = rec
    return calls


def details(logs_dir: Path) -> dict:
    calls = records(logs_dir)
    first = (calls[0].get("usage") or {}) if calls else {}
    return {
        "calls": len(calls),
        "first_input": first.get("prompt_tokens"),
        "first_cached": first.get("cached_prompt_tokens"),
    }


def session_usage(logs_dir: Path) -> dict | None:
    calls = records(logs_dir)
    if not calls:
        return None
    usages = [c["usage"] for c in calls if isinstance(c.get("usage"), dict)]
    complete = len(usages) == len(calls)
    cached = [u.get("cached_prompt_tokens") for u in usages]
    costs = [c.get("cost") for c in calls]
    priced = complete and all(isinstance(c, (int, float)) for c in costs)
    # Missing reports remain visible. Observed totals are diagnostic lower bounds.
    observed = {
        "input": sum(u.get("prompt_tokens", 0) for u in usages),
        "cached": sum(c for c in cached if isinstance(c, int)),
        "output": sum(u.get("completion_tokens", 0) for u in usages),
    }
    return {
        "n_input_tokens": observed["input"] if complete else None,
        "n_cache_tokens": observed["cached"] if complete and all(c is not None for c in cached) else None,
        "n_cache_write_tokens": sum(u.get("cache_write_tokens") or 0 for u in usages) if complete else None,
        "n_output_tokens": observed["output"] if complete else None,
        "cost_usd": sum(costs) if priced else None,
        "peak_context_tokens": max((u.get("prompt_tokens", 0) for u in usages), default=0),
        "n_agent_steps": len(calls),
        "summarization_count": sum(r.get("t") == "compaction" for events in sessions(logs_dir) for r in events),
        "metadata": {
            "model_calls": len(calls),
            "sub_agent_calls": sum(bool(c.get("agent")) for c in calls),
            "usage_reports": len(usages),
            "unreported_calls": len(calls) - len(usages),
            "unpriced_calls": sum(not isinstance(c, (int, float)) for c in costs),
            "observed_tokens": observed,
            **details(logs_dir),
        },
    }


def register(usage_details) -> None:
    """Extend the frozen FrontierHarness collector without changing other harnesses."""
    # The collector normally passes file text to a registered parser. This extension
    # needs the directory to include every session and request-start marker.
    original = usage_details.extract_usage_details
    if getattr(original, "_quack", False):
        return

    def extract(agent_dir, harness):
        return details(agent_dir) if harness == "quack" else original(agent_dir, harness)

    extract._quack = True
    usage_details.extract_usage_details = extract
    original_totals = usage_details.extract_usage_totals

    def totals(agent_dir, harness):
        if harness != "quack":
            return original_totals(agent_dir, harness)
        usage = session_usage(agent_dir)
        if usage is None or usage["n_input_tokens"] is None:
            return None
        return tuple(usage[k] for k in (
            "n_input_tokens", "n_cache_tokens", "n_cache_write_tokens", "n_output_tokens",
        ))

    usage_details.extract_usage_totals = totals
