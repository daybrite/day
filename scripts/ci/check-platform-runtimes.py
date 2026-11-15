#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Keep platform services, shared runtimes and backends inside their layers (DESIGN.md §3.2)."""

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
runtimes = ("day-platform-android", "day-platform-windows")
roots = [*runtimes, *(name for name in packages if name.startswith("day-part-"))]
for root in roots:
    pending = [(root, [root])]
    seen = set()
    while pending:
        name, chain = pending.pop()
        if name in seen or name not in packages:
            continue
        seen.add(name)
        path, data = packages[name]
        if name.startswith("day-toolkit-") or name in ("day-pieces", "day") or (root in runtimes and name == "day-core"):
            errors.append("UI dependency: " + " -> ".join(chain))
        pending.extend((dep, chain + [dep]) for dep in dependencies(data))
    path, _ = packages[root]
    for source in path.rglob("*"):
        if source.suffix not in (".java", ".kt", ".rs"):
            continue
        # Includes Java embedded in bridge! Rust declarations.
        if re.search(r"\bDayBridge\b|\bday_mdc\b|\bday_winui\b|\bday_xaml_sys\b|com\.google\.android\.material", source.read_text()):
            errors.append(f"UI bridge reference: {source.relative_to(ROOT)}")



def reach(root):
    """Every workspace package `root` depends on, transitively, with the chain that reaches it."""
    found = {}
    pending = [(dep, [root, dep]) for dep in dependencies(packages[root][1])]
    while pending:
        name, chain = pending.pop()
        if name in found or name not in packages:
            continue
        found[name] = chain
        pending.extend((dep, chain + [dep]) for dep in dependencies(packages[name][1]))
    return found


# The layers below the user interface (DESIGN.md §3.2). Each rule names what a crate may not
# reach through its normal and build dependencies; dev-dependencies are a test's business.
UI = ("day-core", "day-spec", "day-reactive", "day-pieces")
# A part that reads bundled assets needs the resource contract; one that offers a reactive
# binding needs the signal crate. Nothing else below the UI needs either.
PART_EXCEPTIONS = {"day-part-sound": {"day-spec"}, "day-part-preferences": {"day-reactive"}}
below = {"day-process": tuple(packages), "day-bridge": UI, "day-async": UI, "day-cli": ("day-core", "day-pieces", "day-spec")}
for name in packages:
    if name.startswith("day-part-"):
        below[name] = tuple(layer for layer in UI if layer not in PART_EXCEPTIONS.get(name, ()))
# A backend implements the contract in day-spec; the widget builders sit above it. The whole
# graph is followed, so an edge that arrives through another crate is caught as well.
for name in packages:
    if name.startswith("day-toolkit-"):
        below[name] = ("day-pieces",)
for root, forbidden in below.items():
    for name, chain in reach(root).items():
        if name in forbidden:
            errors.append("layer dependency: " + " -> ".join(chain))

if errors:
    sys.exit("\n".join(errors))
print(f"Platform runtime boundaries: {len(roots)} runtime/service crates are independent of UI backends")
