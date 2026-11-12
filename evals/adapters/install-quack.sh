#!/usr/bin/env bash
# Build quack and register it as an agent in Harbor and Pier benchmark harnesses.
set -euo pipefail

script_dir="$(cd "$(dirname "$0")" && pwd)"
repo_root="$(cd "$script_dir/../.." && pwd)"
# FrontierHarness uploads this script alone to /work/install-harness.sh and runs it
# from the pinned checkout. Direct invocation uses the script's own checkout.
if [ ! -f "$repo_root/crates/quack-cli/Cargo.toml" ]; then
  repo_root="$PWD"
fi
[ -f "$repo_root/crates/quack-cli/Cargo.toml" ] || { echo "Run from a quack checkout" >&2; exit 1; }
script_dir="$repo_root/evals/adapters"

echo "=== Building / Installing quack ==="
cd "$repo_root"

# Always ask Cargo to rebuild the pinned source; cached artifacts are not proof of freshness.
if ! command -v musl-gcc >/dev/null || ! command -v cc >/dev/null; then
  apt-get update -qq
  apt-get install -y -qq build-essential musl-tools
fi
if ! command -v cargo >/dev/null; then
  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain 1.99.1 --profile minimal
  . "$HOME/.cargo/env"
fi
case "$(uname -m)" in
  x86_64) build_target=x86_64-unknown-linux-musl ;;
  aarch64) build_target=aarch64-unknown-linux-musl ;;
  *) echo "Unsupported benchmark architecture" >&2; exit 1 ;;
esac
rustup target add "$build_target"
bash scripts/install.sh --from-source "$repo_root" --target "$build_target" --prefix /usr/local

python3 - "$build_target" <<'PY'
import hashlib
import json
import subprocess
import sys
from pathlib import Path

root = Path.cwd()
installed = Path("/usr/local/share/quack")
bundle = installed / "defaults"
files = {p.relative_to(bundle).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
         for p in sorted(bundle.rglob("*")) if p.is_file()}
adapter_files = {p.name: hashlib.sha256(p.read_bytes()).hexdigest()
                 for p in sorted((root / "evals/adapters").glob("*.py"))}
manifest = {
    "commit": subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip(),
    "dirty": bool(subprocess.check_output(["git", "status", "--porcelain"], text=True).strip()),
    "version": subprocess.check_output(["/usr/local/bin/quack", "--version"], text=True).strip().split(maxsplit=1)[1],
    "target": sys.argv[1],
    "binary_sha256": hashlib.sha256(Path("/usr/local/bin/quack").read_bytes()).hexdigest(),
    "defaults_sha256": hashlib.sha256(json.dumps(files, sort_keys=True).encode()).hexdigest(),
    "defaults_files": files,
    "adapter_files": adapter_files,
    "profile": "generic",
    "edit_format": "str_replace",
    "max_steps": int(__import__("os").environ.get("QUACK_EVAL_MAX_STEPS", "500")),
}
(installed / "eval-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
PY

echo "=== Registering quack in Harbor and Pier ==="
cp "$script_dir/quack_adapter.py" /usr/local/share/quack/quack_adapter.py
cp "$script_dir/quack_usage.py" /usr/local/share/quack/quack_usage.py

python3 - << 'PYEOF'
import glob
import os
import shutil

adapter_src = "/usr/local/share/quack/quack_adapter.py"
dirs = set()

for p in glob.glob("/root/.local/share/uv/tools/*/lib/python*/site-packages"):
    dirs.add(p)
for p in glob.glob(os.path.expanduser("~/.local/share/uv/tools/*/lib/python*/site-packages")):
    dirs.add(p)

try:
    import site
    for s in site.getsitepackages():
        dirs.add(s)
except Exception:
    pass

for d in dirs:
    if os.path.isdir(d):
        dest = os.path.join(d, "quack_adapter.py")
        shutil.copyfile(adapter_src, dest)
        shutil.copyfile("/usr/local/share/quack/quack_usage.py", os.path.join(d, "quack_usage.py"))
        pth = os.path.join(d, "register_quack.pth")
        with open(pth, "w") as f:
            f.write("import quack_adapter; quack_adapter.register()\n")
        print(f"Registered quack in {d}")
PYEOF

echo "=== Registration complete ==="
