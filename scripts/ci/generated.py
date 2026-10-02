# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Write generated text, or check it without changing the working tree."""

import argparse
import difflib
import sys
from pathlib import Path


def write_or_check(filename: str, content: str) -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    path = Path(filename)
    if not args.check:
        path.write_text(content, encoding="utf-8")
        return
    existing = path.read_text(encoding="utf-8") if path.exists() else ""
    if existing != content:
        sys.stderr.writelines(
            difflib.unified_diff(
                existing.splitlines(keepends=True),
                content.splitlines(keepends=True),
                fromfile=filename,
                tofile=f"{filename} (generated)",
            )
        )
        raise SystemExit(f"{filename} is stale; regenerate with its scripts/ci generator.")
