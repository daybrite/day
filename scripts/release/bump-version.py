#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""bump-version.py — move the workspace to a new release version, manifests and locks together.

    scripts/release/bump-version.py 0.6.3     # rewrite, then refresh every committed lock
    scripts/release/bump-version.py --check   # report anything not at the workspace version

A release changes the version in three kinds of place, and a tag whose commit misses one fails
CI on `--locked` (v0.6.2's tag run: the conformance app and two day-core fixtures still locked
the day crates at 0.6.1):

  * `[workspace.package].version` in the root Cargo.toml, which every framework crate inherits;
  * every internal path dependency that carries a `version` (crates.io requires one): the
    `[workspace.dependencies]` table, and the few member manifests that name a sibling directly;
  * every committed Cargo.lock, whose entries for the workspace's own crates record their
    versions. `cargo update --workspace --offline`, run inside each lock's workspace, rewrites
    exactly those entries and nothing else.

Each lock is then held to `cargo metadata --locked --offline`, the check CI's jobs run. The
documentation names no release (`cargo add`, `--day-version latest`, docs.rs `latest`), so a
release changes nothing there. Run from anywhere: paths resolve against the repository this script
lives in.
"""

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
SEMVER = re.compile(r"^\d+\.\d+\.\d+$")


def tracked(pattern):
    out = subprocess.run(["git", "ls-files", pattern], cwd=ROOT, check=True,
                         capture_output=True, text=True).stdout
    return [ROOT / p for p in out.splitlines() if p]


def workspace_version(manifest):
    """The `version` line of `[workspace.package]`, as (index of that line, value)."""
    lines = manifest.read_text().splitlines(keepends=True)
    section = None
    for i, line in enumerate(lines):
        header = re.match(r"\s*\[([^\]]+)\]\s*$", line)
        if header:
            section = header.group(1).strip()
            continue
        found = re.match(r'\s*version\s*=\s*"([^"]+)"', line)
        if section == "workspace.package" and found:
            return i, found.group(1)
    sys.exit(f"bump-version: no [workspace.package] version in {manifest}")


# A path dependency's inline table that pins a version: `{ path = "…", version = "x.y.z", … }`,
# either key first. Third-party requirements carry no `path`, so they never match.
PATH_DEP = re.compile(r'\{[^}]*\bpath\s*=\s*"[^"]*"[^}]*\}')
VERSION_IN = re.compile(r'(\bversion\s*=\s*")(\d+\.\d+\.\d+)(")')


def path_dep_versions(text):
    """Every version string inside a path dependency's inline table, with its span in `text`."""
    for dep in PATH_DEP.finditer(text):
        for v in VERSION_IN.finditer(dep.group(0)):
            yield dep.start() + v.start(2), dep.start() + v.end(2), v.group(2)


def rewrite_manifest(path, old, new, is_root):
    """Replace `old` with `new` in path-dependency versions (and the root's package version)."""
    text = path.read_text()
    spans = [(s, e) for s, e, v in path_dep_versions(text) if v == old]
    for s, e in reversed(spans):
        text = text[:s] + new + text[e:]
    changed = len(spans)
    if is_root:
        lines = text.splitlines(keepends=True)
        i, _ = workspace_version(path)
        lines[i] = re.sub(r'"[^"]+"', f'"{new}"', lines[i], count=1)
        text = "".join(lines)
        changed += 1
    if changed:
        path.write_text(text)
    return changed


def stale_manifest_versions(path, version):
    return [(path, v) for _, _, v in path_dep_versions(path.read_text()) if v != version]


def lock_is_current(lock_dir):
    return subprocess.run(["cargo", "metadata", "--locked", "--offline", "--format-version", "1"],
                          cwd=lock_dir, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
                          text=True)


def framework_names():
    """Every workspace member's package name: the crates whose version is the release's."""
    out = subprocess.run(["cargo", "metadata", "--no-deps", "--offline", "--format-version", "1"],
                         cwd=ROOT, check=True, capture_output=True, text=True).stdout
    return {p["name"] for p in json.loads(out)["packages"]}


def lock_versions(lock, names):
    """{name: version} for the lock's entries of `names` that come from a path (no `source`)."""
    found = {}
    for block in lock.read_text().split("[[package]]")[1:]:
        name = re.search(r'^name = "([^"]+)"', block, re.M)
        version = re.search(r'^version = "([^"]+)"', block, re.M)
        if name and version and name.group(1) in names and "\nsource = " not in block:
            found[name.group(1)] = version.group(1)
    return found


def main():
    parser = argparse.ArgumentParser(description=__doc__,
                                     formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("version", nargs="?", help="the new version, x.y.z")
    parser.add_argument("--check", action="store_true",
                        help="change nothing; exit 1 if any manifest or lock is behind")
    args = parser.parse_args()
    if args.check == bool(args.version):
        parser.error("give a new version, or --check")

    root_manifest = ROOT / "Cargo.toml"
    _, current = workspace_version(root_manifest)
    manifests = tracked("*Cargo.toml")
    locks = tracked("*Cargo.lock")
    names = framework_names()

    if args.version:
        new = args.version.removeprefix("v")
        if not SEMVER.match(new):
            sys.exit(f"bump-version: {args.version!r} is not x.y.z")
        if new == current:
            print(f"bump-version: the workspace is already at {new}; refreshing the locks only")
        else:
            total = 0
            for manifest in manifests:
                n = rewrite_manifest(manifest, current, new, manifest == root_manifest)
                if n:
                    total += n
                    print(f"  {manifest.relative_to(ROOT)}: {n} version(s) {current} → {new}")
            print(f"bump-version: {total} manifest version(s) moved to {new}")
        for lock in locks:
            subprocess.run(["cargo", "update", "--workspace", "--offline", "--quiet"],
                           cwd=lock.parent, check=True)
            print(f"  {lock.relative_to(ROOT)}: refreshed")
        current = new

    failed = False
    for manifest in manifests:
        for path, v in stale_manifest_versions(manifest, current):
            print(f"behind: {path.relative_to(ROOT)} pins a path dependency at {v}, not {current}")
            failed = True
    for lock in locks:
        behind = {n: v for n, v in lock_versions(lock, names).items() if v != current}
        if behind:
            found = ", ".join(sorted(set(behind.values())))
            print(f"behind: {lock.relative_to(ROOT)} locks {len(behind)} day crate(s) at {found}, "
                  f"not {current}")
            failed = True
        result = lock_is_current(lock.parent)
        if result.returncode != 0:
            lines = result.stderr.strip().splitlines()
            error = next((l for l in lines if "error" in l), lines[0] if lines else "")
            print(f"stale: {lock.relative_to(ROOT)} fails `cargo metadata --locked`: "
                  f"{error.strip()}")
            failed = True
    if failed:
        print(f"bump-version: not every manifest and lock is at {current}"
              + ("; run scripts/release/bump-version.py " + current if args.check else ""))
        return 1
    print(f"bump-version: {len(manifests)} manifest(s) and {len(locks)} lock(s) at {current}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
