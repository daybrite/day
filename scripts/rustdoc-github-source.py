#!/usr/bin/env python3
"""Repoint rustdoc's "Source" links at GitHub instead of the local `src/*.html` pages.

Rustdoc renders every documented crate's source into `<doc-dir>/src/<crate>/<file>.rs.html` and links
each item's "Source" / `[src]` link there. This rewrites those links to the canonical source on GitHub,
e.g.

    ../src/day_part_battery/lib.rs.html#1-113
    -> https://github.com/daybrite/day/blob/main/parts/day-part-battery/src/lib.rs#L1-L113

then deletes the orphaned local source tree. The crate -> repo-path map comes from `cargo metadata`
(each crate's lib source dir, relative to the workspace root), so it needs no hand-maintained table.
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys

# ../src/<crate>/<path>.rs.html#<start>[-<end>]  — always relative (at least one ../) from an item page.
LINK = re.compile(r'(?:\.\./)+src/([A-Za-z0-9_]+)/(.+?\.rs)\.html#(\d+)(?:-(\d+))?')


def crate_src_dirs():
    """Map each workspace crate's rustdoc dir name (underscored) -> its lib source dir, repo-relative."""
    md = json.loads(
        subprocess.check_output(["cargo", "metadata", "--no-deps", "--format-version", "1"])
    )
    root = md["workspace_root"]
    out = {}
    lib_kinds = {"lib", "rlib", "dylib", "cdylib", "staticlib", "proc-macro"}
    for pkg in md["packages"]:
        for target in pkg["targets"]:
            if lib_kinds.intersection(target["kind"]):
                rel = os.path.relpath(os.path.dirname(target["src_path"]), root)
                out[pkg["name"].replace("-", "_")] = rel.replace(os.sep, "/")
                break
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--doc-dir", required=True, help="rustdoc output directory (e.g. target/doc)")
    ap.add_argument("--repo", required=True, help="repo URL, e.g. https://github.com/daybrite/day")
    ap.add_argument("--ref", default="main", help="branch, tag, or commit the links resolve against")
    args = ap.parse_args()

    src_dirs = crate_src_dirs()
    base = args.repo.rstrip("/") + "/blob/" + args.ref
    unknown = set()

    def repl(m):
        crate, path, start, end = m.group(1), m.group(2), m.group(3), m.group(4)
        src_dir = src_dirs.get(crate)
        if src_dir is None:
            unknown.add(crate)
            return m.group(0)  # leave untouched rather than emit a wrong link
        frag = f"#L{start}" + (f"-L{end}" if end else "")
        return f"{base}/{src_dir}/{path}{frag}"

    # Rewrite .html AND .js: rustdoc's trait.impl/ & type.impl/ .js carry pre-rendered impl blocks that
    # can hold source links too. Absolute std links (https://doc.rust-lang.org/.../src/…) lack the
    # leading ../ so the regex never touches them; only relative Day-crate links match.
    files, links = 0, 0
    for dirpath, _dirs, names in os.walk(args.doc_dir):
        rel = os.path.relpath(dirpath, args.doc_dir)
        if rel == "src" or rel.startswith("src" + os.sep):
            continue  # the local source tree is deleted below; don't rewrite within it
        for name in names:
            if not name.endswith((".html", ".js")):
                continue
            fp = os.path.join(dirpath, name)
            with open(fp, encoding="utf-8") as fh:
                text = fh.read()
            new, n = LINK.subn(repl, text)
            if n:
                with open(fp, "w", encoding="utf-8") as fh:
                    fh.write(new)
                files += 1
                links += n

    # The local source viewer is now unreferenced — drop it (and its file index) to keep the bundle lean.
    shutil.rmtree(os.path.join(args.doc_dir, "src"), ignore_errors=True)
    try:
        os.remove(os.path.join(args.doc_dir, "src-files.js"))
    except FileNotFoundError:
        pass

    print(f"rustdoc-github-source: rewrote {links} Source links across {files} files -> {base}")
    if unknown:
        print(f"  warning: unmapped crates left untouched: {', '.join(sorted(unknown))}", file=sys.stderr)
    # Fail loudly if any local source link survived outside the (now-deleted) src/ tree.
    leftover = 0
    for dirpath, _dirs, names in os.walk(args.doc_dir):
        for name in names:
            if name.endswith((".html", ".js")):
                with open(os.path.join(dirpath, name), encoding="utf-8") as fh:
                    leftover += len(LINK.findall(fh.read()))
    if leftover:
        print(f"  error: {leftover} local source link(s) remain after rewrite", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
