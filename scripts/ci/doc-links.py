#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0

# Doc cross-reference lint (scripts/ci/lint.sh "doc links" leg).
#
# Two failure modes, both of which shipped for months before this gate existed:
#   1. A bare `docs/foo.md` mention of a doc that exists, unclickable on the published site
#      and a 404 on GitHub. Write it as a link: [docs/foo.md](foo.md) from inside docs/,
#      [docs/foo.md](docs/foo.md) from DESIGN.md.
#   2. A relative .md link whose target does not exist (a typo, or a doc that moved).
#
# Bare mentions of docs that don't exist are ignored: they are planned-file references
# (annotate them as planned in prose) and website-page mentions, which should link to the
# site URL instead. Code fences are exempt: a path in sample output is not a reference.

import os
import json
import re
import sys
from pathlib import Path

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DOCS = os.path.join(ROOT, "docs")

existing = {str(p.relative_to(DOCS)) for p in Path(DOCS).rglob('*.md')}
BARE = re.compile(r"docs/([a-z0-9-]+\.md)")
REL_LINK = re.compile(r"\]\(([^\s)#]+\.md)(?:#[^\s)]*)?\)")

problems = []


def check(path, label):
    in_fence = False
    for lineno, line in enumerate(open(path).read().split("\n"), 1):
        stripped = line.lstrip()
        if stripped.startswith("```") or stripped.startswith("~~~"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        for m in BARE.finditer(line):
            name = m.group(1)
            if name not in existing:
                continue
            start = m.start()
            if any(link.start() <= start < link.end()
                   for link in re.finditer(r"\]\([^\n)]*\)", line)):
                continue  # includes paths inside links to an external repository
            before = line[max(0, start - 1) : start]
            two = line[max(0, start - 2) : start]
            after = line[m.end() : m.end() + 1]
            if before == "[" or two == "](" or after == "]":
                continue  # already link syntax
            if before == "`" and two == "[`":
                continue  # code-span link text: [`docs/foo.md`](foo.md)
            problems.append(f"{label}:{lineno}: bare reference to docs/{name} — make it a link")
        for m in REL_LINK.finditer(line):
            name = m.group(1)
            if name.startswith(('https:', 'http:', '/')):
                continue
            if not (Path(path).parent / name).is_file():
                problems.append(f"{label}:{lineno}: relative link target {name} does not exist")


for f in sorted(existing):
    check(os.path.join(DOCS, f), f"docs/{f}")
check(os.path.join(ROOT, "DESIGN.md"), "DESIGN.md")

# Canonical repository docs must have a published route. The website architecture check
# validates the corresponding symlink, audience, and sidebar placement.
catalog = json.loads((Path(ROOT) / "website/src/data/content-routes.json").read_text())
for name in sorted(existing):
    entry = catalog.get("internal/" + name[:-3])
    if not entry or entry.get("source") != "docs/" + name:
        problems.append(f"docs/{name} has no canonical content route")

if problems:
    for p in problems:
        print(p)
    print(f"\n{len(problems)} doc-link problem(s)")
    sys.exit(1)
print("doc links OK")
