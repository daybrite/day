#!/usr/bin/env python3
"""render-installers.py — render the installer templates for one CLI release.

Fills the placeholders in scripts/release/templates/ (shell + PowerShell installers and the
Homebrew formula, modeled on cargo-dist's installers) with the release's version, download
URLs, archive sizes, and sha256 checksums, and writes a checksums manifest:

    render-installers.py --version 0.2.0 --dist dist --out dist \
        [--templates scripts/release/templates] \
        [--base-url https://github.com/daybrite/day/releases/download/v0.2.0]

Inputs are the archives produced by package-cli.sh (day-<triple>.tar.gz / .zip) in --dist.
Every expected archive must exist, and every placeholder must resolve — a partial render is a
hard error, never a silently broken installer. Outputs into --out:

    day-installer.sh   day-installer.ps1   day.rb   day-SHA256SUMS.txt
"""

import argparse
import hashlib
import pathlib
import re
import sys

REPO_URL = "https://github.com/daybrite/day"

# (triple, archive extension) — must match package-cli.sh exactly.
TARGETS = [
    ("x86_64-apple-darwin", "tar.gz"),
    ("aarch64-apple-darwin", "tar.gz"),
    ("x86_64-unknown-linux-gnu", "tar.gz"),
    ("aarch64-unknown-linux-gnu", "tar.gz"),
    ("x86_64-pc-windows-msvc", "zip"),
    ("aarch64-pc-windows-msvc", "zip"),
]

TEMPLATES = {
    "installer.sh": "day-installer.sh",
    "installer.ps1": "day-installer.ps1",
    "day.rb": "day.rb",
}


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--version", required=True, help="release version, no leading v (e.g. 0.2.0)")
    ap.add_argument("--dist", required=True, type=pathlib.Path, help="dir with day-<triple> archives")
    ap.add_argument("--out", required=True, type=pathlib.Path)
    ap.add_argument(
        "--templates",
        type=pathlib.Path,
        default=pathlib.Path(__file__).parent / "templates",
    )
    ap.add_argument(
        "--base-url",
        default=None,
        help="download base for the archives (default: the GitHub release for v<version>; "
        "override for testing, e.g. file:///tmp/dist)",
    )
    args = ap.parse_args()

    base_url = args.base_url or f"{REPO_URL}/releases/download/v{args.version}"
    subs = {
        "__DAY_VERSION__": args.version,
        "__DAY_BASE_URL__": base_url,
        # Where the installer scripts themselves are fetched from (usage comments): the
        # evergreen `latest` alias, so docs copy-paste stays correct across releases.
        "__DAY_INSTALLER_BASE__": f"{REPO_URL}/releases/latest/download",
    }

    manifest = []
    for triple, ext in TARGETS:
        archive = args.dist / f"day-{triple}.{ext}"
        if not archive.is_file():
            print(f"render-installers: missing archive {archive}", file=sys.stderr)
            return 1
        data = archive.read_bytes()
        digest = hashlib.sha256(data).hexdigest()
        key = triple.replace("-", "_")
        subs[f"__SHA256_{key}__"] = digest
        subs[f"__SIZE_{key}__"] = str(len(data))
        manifest.append(f"{digest}  {archive.name}")

    args.out.mkdir(parents=True, exist_ok=True)
    for template, out_name in TEMPLATES.items():
        text = (args.templates / template).read_text()
        for k, v in subs.items():
            text = text.replace(k, v)
        leftover = sorted(set(re.findall(r"__DAY_[A-Z_]+__|__SHA256_\w+__|__SIZE_\w+__", text)))
        if leftover:
            print(f"render-installers: unresolved placeholders in {template}: {leftover}", file=sys.stderr)
            return 1
        out_path = args.out / out_name
        out_path.write_text(text)
        if out_name.endswith(".sh"):
            out_path.chmod(0o755)
        print(f"  rendered {out_path}")

    checksums = args.out / "day-SHA256SUMS.txt"
    checksums.write_text("\n".join(manifest) + "\n")
    print(f"  wrote {checksums}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
