#!/usr/bin/env python3
"""Install quack accounting into a pinned FrontierHarness v1 scripts directory."""

import argparse
import ast
import shutil
from pathlib import Path

MARKER = "# quack usage extension (managed by prepare-frontierharness.py)"
EXTENSION = f'''
{MARKER}
import sys as _quack_sys
from quack_usage import register as _quack_register
_quack_register(_quack_sys.modules[__name__])
'''


def prepare(scripts: Path) -> None:
    target = scripts / "usage_details.py"
    text = target.read_text()
    tree = ast.parse(text)
    functions = {node.name for node in tree.body if isinstance(node, ast.FunctionDef)}
    if not {"extract_usage_details", "extract_usage_totals"} <= functions:
        raise ValueError("Unsupported FrontierHarness collector · use the pinned v1 workflow")
    if MARKER not in text:
        backup = target.with_suffix(".py.quack-original")
        if not backup.exists():
            shutil.copyfile(target, backup)
        target.write_text(text.rstrip() + "\n" + EXTENSION)
    shutil.copyfile(Path(__file__).with_name("quack_usage.py"), scripts / "quack_usage.py")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--scripts", required=True, type=Path)
    args = parser.parse_args()
    prepare(args.scripts)
