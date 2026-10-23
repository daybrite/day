#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Keep Android services and their shared runtime independent of widget backends."""

from pathlib import Path
import re
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
workspace = tomllib.loads((ROOT / "Cargo.toml").read_text())
packages = {}
for member in workspace["workspace"]["members"]:
    manifest = ROOT / member / "Cargo.toml"
    data = tomllib.loads(manifest.read_text())
    packages[data["package"]["name"]] = (manifest.parent, data)


def dependencies(data):
    for key in ("dependencies", "build-dependencies"):
        for alias, spec in data.get(key, {}).items():
            if isinstance(spec, dict) and spec.get("workspace"):
                spec = workspace["workspace"]["dependencies"][alias]
            yield spec.get("package", alias) if isinstance(spec, dict) else alias
    for target in data.get("target", {}).values():
        yield from dependencies(target)


errors = []
roots = ["day-android-runtime", *(name for name in packages if name.startswith("day-part-"))]
for root in roots:
    pending = [(root, [root])]
    seen = set()
    while pending:
        name, chain = pending.pop()
        if name in seen or name not in packages:
            continue
        seen.add(name)
        path, data = packages[name]
        if path.parent.name == "toolkits" or name in ("day-pieces", "day") or (root == "day-android-runtime" and name == "day-core"):
            errors.append("UI dependency: " + " -> ".join(chain))
        pending.extend((dep, chain + [dep]) for dep in dependencies(data))
    path, _ = packages[root]
    for source in path.rglob("*"):
        if source.suffix not in (".java", ".kt", ".rs"):
            continue
        # Includes Java embedded in bridge! Rust declarations.
        if re.search(r"\bDayBridge\b|\bday_mdc\b|com\.google\.android\.material", source.read_text()):
            errors.append(f"UI bridge reference: {source.relative_to(ROOT)}")

if errors:
    sys.exit("\n".join(errors))
print(f"Android runtime boundary: {len(roots)} runtime/service crates are independent of UI backends")
