#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""reserve-names.py — reserve every workspace crate name on crates.io with a 0.0.1 placeholder.

Trusted Publishing can only be configured for a crate that already exists, and crates.io lets an
account create 5 new crates at once and then one every 10 minutes. Reserving the names ahead of a
release leaves the release itself as new versions only. See the crates.io publishing plan.

    reserve-names.py [--dry-run] [--only NAME ...] [--work DIR]

Each placeholder is an empty library with a README that points at the crate's directory in
daybrite/day. The package named `day` is reserved as `day-app` (library name `day`), first.
Names follow in dependency order, the order a real release publishes them.

The run is idempotent: a name crates.io already has is skipped, so a re-run finishes what an
interrupted run started. On a rate-limit refusal the script sleeps until the time crates.io names
and tries again; any other error stops the run. Uploads use the token `cargo login` stored.
"""

import argparse
import email.utils
import json
import os
import re
import subprocess
import sys
import time
import urllib.error
import urllib.request
from datetime import datetime, timezone
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
VERSION = "0.0.1"
USER_AGENT = "daybrite-reserve-names (https://github.com/daybrite/day)"
# The umbrella crate's package name; its library keeps the name `day`.
RENAMES = {"day": "day-app"}


def log(msg):
    stamp = datetime.now().strftime("%Y-%m-%d %H:%M:%S")
    print(f"[{stamp}] {msg}", flush=True)


def workspace_order():
    """Return (package name, crate dir relative to ROOT) in dependency order, umbrella first."""
    out = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=ROOT, check=True, capture_output=True, text=True,
    ).stdout
    packages = json.loads(out)["packages"]
    names = {p["name"] for p in packages}
    deps = {
        p["name"]: {d["name"] for d in p["dependencies"]
                    if d["name"] in names and d["kind"] in (None, "build")}
        for p in packages
    }
    dirs = {p["name"]: Path(p["manifest_path"]).parent.relative_to(ROOT) for p in packages}
    level = {}

    def depth(name):
        if name not in level:
            level[name] = 0  # cycle guard; normal dependencies cannot cycle
            level[name] = 1 + max((depth(d) for d in deps[name]), default=-1)
        return level[name]

    order = sorted(names, key=lambda n: (n not in RENAMES, depth(n), n))
    return [(RENAMES.get(n, n), dirs[n]) for n in order]


def exists_on_crates_io(name):
    req = urllib.request.Request(f"https://crates.io/api/v1/crates/{name}",
                                 headers={"User-Agent": USER_AGENT})
    try:
        with urllib.request.urlopen(req, timeout=30) as resp:
            return resp.status == 200
    except urllib.error.HTTPError as e:
        if e.code == 404:
            return False
        raise


def write_placeholder(work, name, crate_dir):
    pkg = work / name
    (pkg / "src").mkdir(parents=True, exist_ok=True)
    source = f"https://github.com/daybrite/day/tree/main/{crate_dir}"
    lib = '\n[lib]\nname = "day"\n' if name == "day-app" else ""
    (pkg / "Cargo.toml").write_text(f"""[package]
name = "{name}"
version = "{VERSION}"
edition = "2024"
license = "MPL-2.0"
description = "Part of Day, which builds native apps from one Rust codebase. This placeholder holds the name until the 0.7.0 release."
repository = "https://github.com/daybrite/day"
homepage = "https://daybrite.dev"
readme = "README.md"
{lib}""")
    (pkg / "README.md").write_text(f"""# {name}

Part of [Day](https://daybrite.dev), which builds native apps from one Rust codebase.

This 0.0.1 release is a placeholder that holds the name. The code is developed in
[`{crate_dir}`]({source}) in the daybrite/day repository and ships here from 0.7.0.
""")
    (pkg / "src" / "lib.rs").write_text(
        f"//! Placeholder for `{name}`. The code ships from 0.7.0; see <{source}>.\n")
    return pkg


def retry_after(output):
    """The Unix time a rate-limit refusal says to try again, or None if this is not one."""
    if "too many" not in output and "429" not in output:
        return None
    m = re.search(r"try again after (.+? GMT)", output)
    if m:
        try:
            return email.utils.parsedate_to_datetime(m.group(1)).timestamp()
        except (TypeError, ValueError):
            pass
    return datetime.now(timezone.utc).timestamp() + 600


def publish(pkg, dry_run, target):
    cmd = ["cargo", "publish", "--allow-dirty"] + (["--dry-run"] if dry_run else [])
    env = dict(os.environ, CARGO_TARGET_DIR=str(target))
    while True:
        res = subprocess.run(cmd, cwd=pkg, env=env, capture_output=True, text=True)
        output = res.stdout + res.stderr
        if res.returncode == 0:
            return
        when = retry_after(output)
        if when is None:
            sys.stderr.write(output)
            raise SystemExit(f"cargo publish failed in {pkg}; stopping. A re-run resumes here.")
        wait = max(30, when - time.time() + 15)
        log(f"rate limited; sleeping {wait / 60:.1f} min")
        time.sleep(wait)


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--dry-run", action="store_true", help="package and verify, upload nothing")
    ap.add_argument("--only", nargs="+", metavar="NAME", help="reserve only these names")
    ap.add_argument("--work", type=Path, help="directory for the generated placeholders")
    args = ap.parse_args()

    # Placeholders are built outside the checkout, so no .cargo/config.toml patch table applies.
    work = (args.work or Path(os.environ.get("TMPDIR", "/tmp")) / "day-reserve-names").resolve()
    if work.is_relative_to(ROOT):
        raise SystemExit("--work must be outside the day checkout")
    work.mkdir(parents=True, exist_ok=True)

    plan = workspace_order()
    if args.only:
        unknown = set(args.only) - {n for n, _ in plan}
        if unknown:
            raise SystemExit(f"not workspace names: {', '.join(sorted(unknown))}")
        plan = [(n, d) for n, d in plan if n in args.only]

    reserved = skipped = 0
    for i, (name, crate_dir) in enumerate(plan, 1):
        if exists_on_crates_io(name):
            log(f"{i}/{len(plan)} {name}: already on crates.io, skipped")
            skipped += 1
            continue
        pkg = write_placeholder(work, name, crate_dir)
        publish(pkg, args.dry_run, work / "target")
        reserved += 1
        log(f"{i}/{len(plan)} {name}: {'verified' if args.dry_run else 'published'} {VERSION}")
    log(f"done: {reserved} {'verified' if args.dry_run else 'published'}, {skipped} skipped")


if __name__ == "__main__":
    main()
