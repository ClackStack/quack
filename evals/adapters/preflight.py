#!/usr/bin/env python3
"""Exercise the real benchmark command and native tools against a local mock model."""

import argparse
import base64
import json
import os
import subprocess
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest.mock import patch

import quack_adapter as adapter
from quack_usage import session_usage


def preflight(binary: Path, defaults: Path) -> dict:
    requests = []

    class Model(BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            requests.append(body)
            child = any(m.get("role") == "user" and isinstance(m.get("content"), str)
                        and "QUACK_PREFLIGHT_CHILD" in m["content"] for m in body["messages"])
            has_results = any(m.get("role") == "tool" for m in body["messages"])
            calls = []
            if not has_results:
                specs = [("read", {"path": "input.txt"})]
                if not child:
                    specs += [
                        ("write", {"path": "output.txt", "content": "verified\n"}),
                        ("shell", {"command": "printf 'shell verified\\n'"}),
                        ("view_image", {"path": "image.png"}),
                        ("task", {"agent": "explore", "description": "Verify delegation",
                                  "prompt": "QUACK_PREFLIGHT_CHILD: read input.txt"}),
                    ]
                calls = [{"index": i, "id": f"call-{i}", "function": {
                    "name": name, "arguments": json.dumps(args),
                }} for i, (name, args) in enumerate(specs)]
            delta = {"tool_calls": calls} if calls else {"content": "Preflight complete."}
            chunk = {"choices": [{"delta": delta, "finish_reason": "tool_calls" if calls else "stop"}],
                     "usage": {"prompt_tokens": 100, "completion_tokens": 20,
                               "prompt_tokens_details": {"cached_tokens": 40}}}
            response = f"data: {json.dumps(chunk)}\n\ndata: [DONE]\n\n".encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(response)))
            self.end_headers()
            self.wfile.write(response)

    with tempfile.TemporaryDirectory(prefix="quack-preflight-") as directory:
        root = Path(directory)
        work = root / "work"
        work.mkdir()
        (work / "input.txt").write_text("read verified\n")
        (work / "image.png").write_bytes(base64.b64decode(
            "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aU/8AAAAASUVORK5CYII="))
        prompt = root / "prompt"
        prompt.write_text("QUACK_PREFLIGHT_MAIN: verify the supplied tools.")
        server = ThreadingHTTPServer(("127.0.0.1", 0), Model)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            with patch.multiple(adapter, BIN=str(binary.resolve()), DEFAULTS=str(defaults.resolve()),
                                AGENT_LOGS=root / "logs", PROMPT=str(prompt)):
                env = dict(os.environ)
                for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "QUACK_MODEL", "QUACK_OFFLINE"):
                    env.pop(key, None)
                env.update(adapter.quack_env("kimi-k3", {"FIREWORKS_API_KEY": "synthetic-key"}))
                env["QUACK_SYSTEM_DIR"] = str(root / "system")
                provider_dir = Path(env["QUACK_CONFIG_DIR"]) / "providers.d"
                provider_dir.mkdir(parents=True)
                (provider_dir / "fireworks.toml").write_text(
                    f'base_url="http://127.0.0.1:{server.server_port}/v1"\n'
                    'api_key={env="FIREWORKS_API_KEY"}\n'
                    'models=[{id="accounts/fireworks/models/kimi-k3",vision=true,context_window=1000000,'
                    'cost={input=1.0,output=2.0,cached_input=0.5}}]\n')
                result = subprocess.run(["bash", "-c", adapter.quack_command("kimi-k3")],
                                        cwd=work, env=env, capture_output=True, text=True, timeout=60)
            if result.returncode:
                raise RuntimeError(f"Preflight command exited {result.returncode}:\n{result.stdout}\n{result.stderr}")
            if (work / "output.txt").read_text() != "verified\n":
                raise RuntimeError("Native write did not produce the expected file")
            serialized = json.dumps(requests)
            for evidence in ("read verified", "shell verified", "data:image/png;base64,", "Report from the explore agent"):
                if evidence not in serialized:
                    raise RuntimeError(f"Missing native tool evidence: {evidence}")
            usage = session_usage(root / "logs")
            if len(requests) != 4 or usage is None or usage["n_agent_steps"] != 4 or usage["n_input_tokens"] != 400:
                raise RuntimeError(f"Expected four accounted main/child calls: {usage}")
            return {"model_calls": len(requests), "tools": ["read", "write", "shell", "view_image", "task"],
                    "input_tokens": usage["n_input_tokens"], "cached_tokens": usage["n_cache_tokens"]}
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--defaults", type=Path, default=Path(__file__).resolve().parents[2] / "defaults")
    args = parser.parse_args()
    print(json.dumps(preflight(args.binary, args.defaults), indent=2))
