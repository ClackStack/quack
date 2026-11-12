"""Offline adapter/collector regressions. Run with unittest discovery."""

import importlib.util
import json
import os
import shlex
import subprocess
import sys
import tempfile
import types
import unittest
from pathlib import Path
from unittest.mock import patch

import quack_adapter as adapter
import quack_usage as usage


class AdapterTests(unittest.TestCase):
    def test_benchmark_routes_map_to_quack_providers(self):
        for supplied, expected in [
            (None, "fireworks/accounts/fireworks/models/kimi-k3"),
            ("kimi-k3", "fireworks/accounts/fireworks/models/kimi-k3"),
            ("fireworks_ai/accounts/fireworks/models/kimi-k3", "fireworks/accounts/fireworks/models/kimi-k3"),
            ("together_ai/moonshotai/Kimi-K3", "together/moonshotai/Kimi-K3"),
            ("charm-hyper/kimi-k3", "charm-hyper/kimi-k3"),
        ]:
            with self.subTest(route=supplied):
                self.assertEqual(adapter.quack_model(supplied), expected)

    def test_only_the_selected_providers_credential_is_forwarded(self):
        with patch.dict(os.environ, {"FIREWORKS_API_KEY": "test-fireworks", "CHARM_API_KEY": "test-charm"}, clear=True):
            env = adapter.quack_env("kimi-k3")
        self.assertEqual(env["FIREWORKS_API_KEY"], "test-fireworks")
        self.assertNotIn("CHARM_API_KEY", env)
        self.assertTrue(env["QUACK_DATA_DIR"].startswith("/logs/agent/"))

    def test_runner_injected_credentials_are_kept(self):
        with patch.dict(os.environ, {}, clear=True):
            env = adapter.quack_env("kimi-k3", {"FIREWORKS_API_KEY": "runta-secret-stub"})
        self.assertEqual(env["FIREWORKS_API_KEY"], "runta-secret-stub")

    def test_missing_key_fails_before_model_execution(self):
        with patch.dict(os.environ, {}, clear=True), patch("quack_adapter.os.path.exists", return_value=False):
            with self.assertRaisesRegex(RuntimeError, "FIREWORKS_API_KEY"):
                adapter.quack_env("kimi-k3")

    def test_provider_allowlist_matches_the_model(self):
        provider = adapter.quack_model("fireworks_ai/accounts/fireworks/models/kimi-k3").split("/", 1)[0]
        self.assertEqual(adapter.PROVIDERS[provider][1], "api.fireworks.ai")

    def test_command_quoting_and_exit_status(self):
        if os.name == "nt":
            self.skipTest("the benchmark wrapper runs in Linux containers")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "fake quack"
            args_file = root / "args.json"
            binary.write_text("#!/bin/sh\nprintf '%s\\n' \"$@\" > " + shlex.quote(str(args_file)) + "\nexit 7\n")
            binary.chmod(0o755)
            prompt = root / "prompt"
            prompt.write_text("synthetic task")
            with patch.multiple(adapter, BIN=str(binary), AGENT_LOGS=root / "logs", PROMPT=str(prompt)), patch.dict(os.environ, {}, clear=True):
                result = subprocess.run(["bash", "-c", adapter.quack_command("kimi-k3")], capture_output=True)
            self.assertEqual(result.returncode, 7)
            args = args_file.read_text().splitlines()
            self.assertIn('model.small=""', args)
            self.assertIn("agent.max_steps=500", args)
            self.assertIn("generic", args)
            self.assertIn("quack exit code: 7", (root / "logs/quack.log").read_text())

    def test_nonpositive_limit_is_rejected(self):
        with patch.dict(os.environ, {"QUACK_EVAL_MAX_STEPS": "0"}):
            with self.assertRaises(ValueError):
                adapter.quack_command("kimi-k3")


class UsageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.log = self.root / "quack-data/sessions/project/session/events.jsonl"
        self.log.parent.mkdir(parents=True)

    def write(self, records):
        self.log.write_text("\n".join(json.dumps(r) for r in records) + "\n")

    def call(self, kind="usage", **extra):
        return {"t": kind, "usage": {"prompt_tokens": 100, "cached_prompt_tokens": 40,
                "completion_tokens": 20}, "cost": 0.1, **extra}

    def test_agents_and_summaries_count_exactly_once(self):
        self.write([
            {"t": "request_start"}, self.call(),
            {"t": "agent_request_start", "agent": "explore-1"},
            self.call("agent_usage", agent="explore-1"),
            {"t": "request_start"}, self.call(), {"t": "compaction"},
        ])
        child = self.log.parent / "agents/explore-1.jsonl"
        child.parent.mkdir()
        child.write_text(json.dumps(self.call()))
        result = usage.session_usage(self.root)
        self.assertEqual((result["n_input_tokens"], result["n_cache_tokens"], result["n_agent_steps"]), (300, 120, 3))
        self.assertAlmostEqual(result["cost_usd"], 0.3)
        self.assertEqual(result["summarization_count"], 1)
        self.assertEqual(usage.details(self.root), {"calls": 3, "first_input": 100, "first_cached": 40})

    def test_killed_request_keeps_complete_totals_unknown(self):
        self.write([{"t": "request_start"}, self.call(), {"t": "request_start"}])
        result = usage.session_usage(self.root)
        self.assertIsNone(result["n_input_tokens"])
        self.assertIsNone(result["cost_usd"])
        self.assertEqual(result["metadata"]["unreported_calls"], 1)
        self.assertEqual(result["metadata"]["observed_tokens"]["input"], 100)

    def test_missing_first_call_usage_is_not_replaced_by_a_later_call(self):
        self.write([{"t": "usage", "usage": None}, self.call()])
        self.assertEqual(usage.details(self.root), {"calls": 2, "first_input": None, "first_cached": None})

    def test_unknown_cache_is_not_zero(self):
        call = self.call()
        del call["usage"]["cached_prompt_tokens"]
        self.write([call])
        self.assertIsNone(usage.session_usage(self.root)["n_cache_tokens"])
        self.assertIsNone(usage.details(self.root)["first_cached"])

    def test_partial_final_line_is_ignored(self):
        self.write([self.call()])
        with self.log.open("a") as handle:
            handle.write('{"t":"usage"')
        self.assertEqual(usage.details(self.root)["calls"], 1)

    def test_collector_extension_preserves_other_harnesses_and_is_idempotent(self):
        self.write([self.call()])
        collector = types.SimpleNamespace(
            extract_usage_details=lambda *_: "original details",
            extract_usage_totals=lambda *_: "original totals",
        )
        usage.register(collector)
        usage.register(collector)
        self.assertEqual(collector.extract_usage_details(self.root, "quack")["first_cached"], 40)
        self.assertEqual(collector.extract_usage_totals(self.root, "quack"), (100, 40, 0, 20))
        self.assertEqual(collector.extract_usage_details(self.root, "codex"), "original details")
        self.assertEqual(collector.extract_usage_totals(self.root, "codex"), "original totals")

    def test_preparation_script_installs_a_working_extension_once(self):
        scripts = self.root / "scripts"
        scripts.mkdir()
        collector = scripts / "usage_details.py"
        collector.write_text('def extract_usage_details(*args): return "original"\ndef extract_usage_totals(*args): return None\n')
        prepare = Path(__file__).with_name("prepare-frontierharness.py")
        for _ in range(2):
            subprocess.run([sys.executable, str(prepare), "--scripts", str(scripts)], check=True)
        self.assertEqual(collector.read_text().count("# quack usage extension"), 1)
        self.assertTrue(collector.with_suffix(".py.quack-original").exists())
        spec = importlib.util.spec_from_file_location("test_collector", collector)
        module = importlib.util.module_from_spec(spec)
        with patch.dict(sys.modules, {spec.name: module}):
            spec.loader.exec_module(module)
        self.write([self.call()])
        self.assertEqual(module.extract_usage_details(self.root, "quack")["first_cached"], 40)


if __name__ == "__main__":
    unittest.main()
