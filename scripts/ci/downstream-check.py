#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Resolve the repositories that depend on this checkout against it, the way their CI will.

Every external part, piece and app takes the framework from `https://github.com/daybrite/day.git`
with no revision pinned, so a push to this repository's `main` changes what every one of them
resolves on its next CI run. A rename or removal here (a crate, a feature, a Java class) is
complete only when each of those repositories still resolves against it, and the developer's
machine cannot show that: `day patch --local` points every project at every sibling checkout, so
a tree that is green here can be broken as pushed. This script asks the question CI asks, before
the push:

    for each downstream repository, does its tree AS PUSHED (its upstream ref) resolve against
    this checkout's HEAD?

**Siblings mode** (the default, a developer's machine): the downstream repositories are the sibling
checkouts of this one (`../*`) whose manifests depend on daybrite/day from git. Each is exported at
`@{upstream}` and checked with `day patch --local <this checkout> --check`, which rewrites the
export's patch table and asks cargo to resolve the graph for every platform. A repository that
fails is checked again from its HEAD and its working tree, so the report says whether the fix
exists and where:

    ok                   resolves as pushed
    FIXED, unpushed      broken as pushed; its HEAD resolves — push it right after day
    FIXED, uncommitted   broken as pushed; only its working tree resolves — commit it first
    BROKEN               broken as pushed, in HEAD and in the working tree: the change is incomplete

**Clone mode** (`--clone`, CI): the repositories listed in `scripts/ci/downstream-repos.txt` are
cloned shallow and checked against this checkout. There is no working tree to consult, so a
failure is a failure.

Exit status is 0 when every downstream resolves as pushed or has a committed, unpushed fix, and 1
otherwise. `--strict` also fails on the committed-but-unpushed case. Nothing here touches the
sibling checkouts: every check runs on an export under a temporary directory, so no `.cargo/config.toml`
or `Cargo.lock` of a real checkout is rewritten.

    python3 -B scripts/ci/downstream-check.py              # siblings, day at HEAD
    python3 -B scripts/ci/downstream-check.py --worktree   # siblings, day as it is on disk
    python3 -B scripts/ci/downstream-check.py --clone      # CI: the listed repositories

The `day` binary is built from this checkout (`cargo build -q -p day-cli`) unless `--day-bin`
names one; an installed `day` is never used.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import io
import os
import re
import shutil
import subprocess
import sys
import tarfile
import tempfile
from dataclasses import dataclass, field
from pathlib import Path

DAY_ROOT = Path(__file__).resolve().parents[2]
REPO_LIST = DAY_ROOT / "scripts" / "ci" / "downstream-repos.txt"
# A manifest line that takes a crate from this repository, as the apps and pieces write it. The
# optional `.git` and the closing quote keep `daybrite/day-piece-lottie.git` from matching.
DAY_DEP = re.compile(r'github\.com/daybrite/day(\.git)?"')
# Directories whose manifests are never roots worth checking: build output, host projects'
# node trees, and anything git itself keeps.
SKIP_DIRS = {"target", "node_modules", ".git", "build", "dist"}


def run(args: list[str], cwd: Path | None = None, env: dict | None = None,
        check: bool = False, timeout: int | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(args, cwd=cwd, env=env, check=check, timeout=timeout,
                          capture_output=True, text=True, errors="replace")


def git(repo: Path, *args: str) -> str:
    out = run(["git", "-C", str(repo), *args])
    if out.returncode != 0:
        raise RuntimeError(f"git {' '.join(args)} in {repo}: {out.stderr.strip()}")
    return out.stdout


def git_ok(repo: Path, *args: str) -> bool:
    return run(["git", "-C", str(repo), *args]).returncode == 0


# --- Exports ----------------------------------------------------------------------------------

def export_ref(repo: Path, ref: str, dst: Path) -> None:
    """Extract the tree at `ref` into `dst` (an empty directory): the repository as pushed."""
    out = subprocess.run(["git", "-C", str(repo), "archive", "--format=tar", ref],
                         capture_output=True, check=False)
    if out.returncode != 0:
        raise RuntimeError(f"git archive {ref} in {repo}: {out.stderr.decode(errors='replace').strip()}")
    dst.mkdir(parents=True, exist_ok=True)
    with tarfile.open(fileobj=io.BytesIO(out.stdout)) as tar:
        tar.extractall(dst, filter="data")


def export_worktree(repo: Path, dst: Path) -> None:
    """Copy the working tree into `dst`: tracked files plus untracked ones git does not ignore.

    Ignored files stay behind, which is the point: the machine-local `.cargo/config.toml` that
    `day patch --local` writes must not come along, or the export would resolve against the
    sibling checkouts instead of against this one.
    """
    listing = run(["git", "-C", str(repo), "ls-files", "-z", "--cached", "--others",
                   "--exclude-standard"])
    if listing.returncode != 0:
        raise RuntimeError(f"git ls-files in {repo}: {listing.stderr.strip()}")
    dst.mkdir(parents=True, exist_ok=True)
    for rel in listing.stdout.split("\0"):
        if not rel:
            continue
        src = repo / rel
        if not src.is_file():  # deleted in the working tree, or a submodule entry
            continue
        target = dst / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, target)


# --- What to check ----------------------------------------------------------------------------

@dataclass
class Repo:
    name: str
    path: Path            # the checkout (siblings mode) or the clone (clone mode)
    upstream: str | None  # the ref CI resolves: `@{upstream}` when it exists, else HEAD
    ahead: int            # commits in HEAD not yet pushed
    dirty: bool           # uncommitted changes (untracked, unignored files count)


def manifests_depend_on_day(repo: Path, ref: str | None) -> bool:
    """Whether any Cargo.toml at `ref` (or in the working tree when `ref` is None) takes a crate
    from daybrite/day over git."""
    args = ["grep", "-l", "-E", "-e", DAY_DEP.pattern]
    if ref:
        args.append(ref)
    args += ["--", "*Cargo.toml", "**/Cargo.toml"]
    out = run(["git", "-C", str(repo), *args])
    return out.returncode == 0 and bool(out.stdout.strip())


def sibling_repos(parent: Path, day: Path) -> list[Repo]:
    repos = []
    for entry in sorted(parent.iterdir(), key=lambda p: p.name.lower()):
        if not entry.is_dir() or entry.resolve() == day.resolve() or not (entry / ".git").exists():
            continue
        if not git_ok(entry, "rev-parse", "--verify", "--quiet", "HEAD"):
            continue  # a repository with no commits yet has nothing pushed to resolve
        upstream = None
        if git_ok(entry, "rev-parse", "--verify", "--quiet", "@{upstream}"):
            upstream = git(entry, "rev-parse", "--abbrev-ref", "@{upstream}").strip()
        if not (manifests_depend_on_day(entry, upstream or "HEAD")
                or manifests_depend_on_day(entry, "HEAD")
                or manifests_depend_on_day(entry, None)):
            continue
        ahead = int(git(entry, "rev-list", "--count", f"{upstream}..HEAD").strip()) if upstream else 0
        dirty = bool(git(entry, "status", "--porcelain").strip())
        repos.append(Repo(entry.name, entry, upstream, ahead, dirty))
    return repos


def listed_repos(list_file: Path) -> list[str]:
    slugs = []
    for line in list_file.read_text().splitlines():
        line = line.split("#", 1)[0].strip()
        if line:
            slugs.append(line)
    return slugs


def cargo_roots(tree: Path) -> list[Path]:
    """The workspace or package roots under an export whose graph depends on daybrite/day.

    A piece repository is usually two of them: the crate at the top and its `demo/` app, and
    each resolves on its own. `cargo locate-project --workspace` maps every manifest to the root
    cargo would build it from, so nested members collapse onto their workspace.
    """
    manifests = []
    for dirpath, dirnames, filenames in os.walk(tree):
        dirnames[:] = [d for d in dirnames if d not in SKIP_DIRS]
        if "Cargo.toml" in filenames:
            manifests.append(Path(dirpath) / "Cargo.toml")
    roots: dict[Path, bool] = {}
    for manifest in manifests:
        out = run(["cargo", "locate-project", "--workspace", "--message-format", "plain",
                   "--manifest-path", str(manifest)], cwd=tree)
        if out.returncode != 0:
            continue  # a template or fixture manifest cargo cannot load is not a root
        root = Path(out.stdout.strip()).parent
        depends = bool(DAY_DEP.search(manifest.read_text(errors="replace")))
        roots[root] = roots.get(root, False) or depends
    return sorted(root for root, depends in roots.items() if depends)


# --- The check --------------------------------------------------------------------------------

@dataclass
class Result:
    repo: str
    root: str            # root relative to the repository, "." for the top
    tree: str            # "pushed", "HEAD" or "worktree"
    ok: bool
    detail: str = ""     # the cargo error on failure, trimmed


def check_root(day_bin: Path, day_tree: Path, root: Path, env: dict) -> tuple[bool, str]:
    """`day patch --local <day_tree> --check` at `root`; on failure, cargo's own error text."""
    try:
        out = run([str(day_bin), "--project", str(root), "patch", "--local", str(day_tree),
                   "--check"], cwd=root, env=env, timeout=900)
    except subprocess.TimeoutExpired:
        return False, "day patch --check: timed out after 900 s"
    if out.returncode == 0:
        return True, ""
    # `day patch` reports resolution failure as one line; the reason is cargo's, so ask it.
    meta = run(["cargo", "metadata", "--format-version", "1"], cwd=root, env=env, timeout=600)
    text = meta.stderr if meta.returncode != 0 else (out.stderr + out.stdout)
    lines = [ln.rstrip() for ln in text.splitlines()
             if ln.strip() and not ln.lstrip().startswith(("Updating", "Blocking", "Note", "Patched",
                                                             "Relocked"))]
    return False, "\n".join(lines[-8:])


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n", 1)[0],
                                 formatter_class=argparse.RawDescriptionHelpFormatter,
                                 epilog=__doc__.split("\n\n", 1)[1])
    ap.add_argument("--day", type=Path, default=DAY_ROOT, help="the day checkout (default: this one)")
    ap.add_argument("--day-bin", type=Path, help="a `day` built from that checkout (default: build it)")
    ap.add_argument("--worktree", action="store_true",
                    help="check day's working tree rather than its HEAD")
    mode = ap.add_mutually_exclusive_group()
    mode.add_argument("--siblings", type=Path, help="directory holding the sibling checkouts "
                                                    "(default: the day checkout's parent)")
    mode.add_argument("--clone", nargs="?", const=REPO_LIST, type=Path, metavar="LIST",
                      help=f"clone the repositories listed in LIST (default {REPO_LIST.name}) "
                           "instead of using sibling checkouts")
    ap.add_argument("--repo", action="append", default=[], metavar="NAME",
                    help="only this repository (directory name or owner/name); repeatable")
    ap.add_argument("--skip", action="append", default=[], metavar="NAME",
                    help="leave this repository out; repeatable")
    ap.add_argument("--strict", action="store_true",
                    help="also fail when a downstream fix is committed but not pushed")
    ap.add_argument("--jobs", type=int, default=4, help="checks to run at once (default 4)")
    ap.add_argument("--keep", action="store_true", help="keep the temporary exports and say where")
    args = ap.parse_args()

    day = args.day.resolve()
    if not (day / "Cargo.toml").is_file() or not (day / "crates" / "day").is_dir():
        print(f"error: {day} is not a day checkout", file=sys.stderr)
        return 2

    # The CLI from this checkout, never an installed one: a stale binary checks with stale rules.
    if args.day_bin:
        day_bin = args.day_bin.resolve()
    else:
        build = run(["cargo", "build", "-q", "-p", "day-cli"], cwd=day)
        if build.returncode != 0:
            print(build.stderr, file=sys.stderr)
            print("error: could not build day-cli from the checkout", file=sys.stderr)
            return 2
        target = Path(os.environ.get("CARGO_TARGET_DIR", day / "target"))
        day_bin = target / "debug" / ("day.exe" if os.name == "nt" else "day")
    if not day_bin.is_file():
        print(f"error: no day binary at {day_bin}", file=sys.stderr)
        return 2

    # The checks must see only the export's own manifests: a DAY_LOCAL in the environment would
    # point `day patch` at a checkout the developer happens to have set, not at the one under test.
    env = {k: v for k, v in os.environ.items() if k != "DAY_LOCAL"}

    tmp = Path(tempfile.mkdtemp(prefix="day-downstream-"))
    try:
        day_tree = tmp / "day"
        if args.worktree:
            export_worktree(day, day_tree)
            day_label = "working tree"
        else:
            export_ref(day, "HEAD", day_tree)
            day_label = f"HEAD {git(day, 'rev-parse', '--short', 'HEAD').strip()}"
            if git(day, "status", "--porcelain").strip():
                print("note: day's working tree has uncommitted changes; checking HEAD as it would "
                      "be pushed (--worktree checks the tree on disk)")

        # The downstream trees as pushed.
        work: list[tuple[Repo, Path, str]] = []  # (repo, export, tree label)
        if args.clone is not None:
            for slug in listed_repos(args.clone):
                name = slug.rsplit("/", 1)[-1]
                if args.repo and name not in args.repo and slug not in args.repo:
                    continue
                if name in args.skip or slug in args.skip:
                    continue
                dst = tmp / name / "pushed"
                url = slug if "://" in slug else f"https://github.com/{slug}.git"
                clone = run(["git", "clone", "--quiet", "--depth", "1", url, str(dst)])
                if clone.returncode != 0:
                    print(f"error: clone {url}: {clone.stderr.strip()}", file=sys.stderr)
                    return 1
                work.append((Repo(name, dst, None, 0, False), dst, "pushed"))
        else:
            parent = (args.siblings or day.parent).resolve()
            repos = sibling_repos(parent, day)
            if args.repo:
                repos = [r for r in repos if r.name in args.repo]
            repos = [r for r in repos if r.name not in args.skip]
            if not repos:
                print(f"no sibling checkouts under {parent} depend on daybrite/day: nothing to check "
                      "here (CI's `downstream` job covers the published repositories)")
                return 0
            for repo in repos:
                dst = tmp / repo.name / "pushed"
                export_ref(repo.path, repo.upstream or "HEAD", dst)
                work.append((repo, dst, "pushed"))

        print(f"checking {len(work)} downstream repositories against day {day_label} "
              f"with {day_bin}")

        def check_tree(repo: Repo, tree: Path, label: str) -> list[Result]:
            results = []
            roots = cargo_roots(tree)
            if not roots:
                return [Result(repo.name, ".", label, False, "no cargo root depends on daybrite/day")]
            for root in roots:
                ok, detail = check_root(day_bin, day_tree, root, env)
                rel = os.path.relpath(root, tree)
                results.append(Result(repo.name, rel, label, ok, detail))
            return results

        with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
            futures = [pool.submit(check_tree, repo, tree, label) for repo, tree, label in work]
            pushed: list[Result] = [r for f in futures for r in f.result()]

        # A failure as pushed: is the fix in the sibling's HEAD, or only in its working tree?
        fixes: dict[str, str] = {}  # repo -> "HEAD" | "worktree"
        failed = sorted({r.repo for r in pushed if not r.ok})
        if failed and args.clone is None:
            by_name = {repo.name: repo for repo, _, _ in work}
            for name in failed:
                repo = by_name[name]
                if repo.ahead:
                    dst = tmp / name / "head"
                    export_ref(repo.path, "HEAD", dst)
                    if all(r.ok for r in check_tree(repo, dst, "HEAD")):
                        fixes[name] = "HEAD"
                        continue
                if repo.dirty:
                    dst = tmp / name / "worktree"
                    export_worktree(repo.path, dst)
                    if all(r.ok for r in check_tree(repo, dst, "worktree")):
                        fixes[name] = "worktree"

        # --- Report
        width = max(len(f"{r.repo}/{r.root}") for r in pushed)
        broken: list[Result] = []
        unpushed: list[str] = []
        uncommitted: list[str] = []
        for r in pushed:
            where = r.repo if r.root == "." else f"{r.repo}/{r.root}"
            if r.ok:
                verdict = "ok"
            elif fixes.get(r.repo) == "HEAD":
                verdict = "FIXED, unpushed"
                if r.repo not in unpushed:
                    unpushed.append(r.repo)
            elif fixes.get(r.repo) == "worktree":
                verdict = "FIXED, uncommitted"
                if r.repo not in uncommitted:
                    uncommitted.append(r.repo)
            else:
                verdict = "BROKEN"
                broken.append(r)
            print(f"  {where:<{width}}  {verdict}")
            if not r.ok and r.detail:
                for line in r.detail.splitlines():
                    print(f"  {'':<{width}}    {line}")

        print()
        status = 0
        if broken:
            names = ", ".join(sorted({r.repo for r in broken}))
            print(f"BROKEN as pushed with no fix on this machine: {names}")
            print("  the change to day is incomplete until these repositories resolve against it")
            status = 1
        if uncommitted:
            print(f"fixed only in the working tree: {', '.join(uncommitted)}")
            print("  commit the fix, so it can be pushed right after day")
            status = 1
        if unpushed:
            print(f"fixed in HEAD, not yet pushed: {', '.join(unpushed)}")
            print("  push these right after day; their CI is red until then")
            if args.strict:
                status = 1
        if status == 0 and not unpushed:
            print(f"every downstream repository resolves against day {day_label}")
        return status
    finally:
        if args.keep:
            print(f"exports kept at {tmp}")
        else:
            shutil.rmtree(tmp, ignore_errors=True)


if __name__ == "__main__":
    sys.exit(main())
