#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Reject retired Day package/import names without relying on staged changes."""

import argparse
import json
from pathlib import Path
import re
import subprocess
import sys


def check(root, manifest):
    names = manifest["packages"]
    tokens = list(names) + [name.replace("-", "_") for name in names]
    pattern = re.compile(r"(?<![\w-])(?:" + "|".join(map(re.escape, tokens)) + r")(?![\w-])")
    paths = manifest["directories"]
    path_pattern = re.compile(r"(?<![\w-])(?:" + "|".join(map(re.escape, paths)) + r")(?![\w-])")
    external = "|".join(map(re.escape, manifest["external_repository_names_retained"]))
    # Package identity changes do not change an existing remote repository's URL.
    repository_url = re.compile(r"(?:https?://|git@)(?:github\.com|raw\.githubusercontent\.com)[:/]daybrite/(?:" + external + r")(?=[/.\s\"')#]|$)")
    local_checkout = re.compile(r"(?:\.\./|/opt/src/github/daybrite/)(?:" + external + r")(?=[/\s\"')]|$)")
    files = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=root).decode().split("\0")
    errors = []
    for name in sorted(set(filter(None, files))):
        path = root / name
        if not path.is_file() or path.is_symlink() or path.name == "crate-renames.json":
            continue
        if path.stat().st_size > 3_000_000:
            continue
        try:
            content = path.read_text()
        except (UnicodeError, OSError):
            continue
        if "\0" in content:
            continue
        for number, line in enumerate(content.splitlines(), 1):
            line = local_checkout.sub("<repository>", repository_url.sub("<repository>", line))
            for match in path_pattern.finditer(line.replace("\\", "/")):
                errors.append(f"{name}:{number}: retired Day crate path {match[0]!r}")
            for match in pattern.finditer(line):
                errors.append(f"{name}:{number}: retired Day name {match[0]!r}")
    return errors


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path(__file__).resolve().parents[2])
    args = parser.parse_args()
    manifest = json.loads(Path(__file__).with_name("crate-renames.json").read_text())
    errors = check(args.root, manifest)
    if errors:
        sys.exit("\n".join(errors))
    print("Day package naming: no retired crate or import names")


if __name__ == "__main__":
    main()
