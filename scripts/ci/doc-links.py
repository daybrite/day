#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0

# Doc cross-reference lint (scripts/ci/lint.sh "doc links" leg).
#
# Two failure modes, both of which shipped for months before this gate existed:
#   1. A bare `docs/foo.md` mention of a doc that exists — unclickable on the published site
#      and a 404 on GitHub. Write it as a link: [docs/foo.md](foo.md) from inside docs/,
#      [docs/foo.md](docs/foo.md) from DESIGN.md.
#   2. A relative .md link whose target does not exist (a typo, or a doc that moved).
#
# Bare mentions of docs that DON'T exist are ignored: they are planned-file references
# (annotate them as planned in prose) and website-page mentions, which should link to the
# site URL instead. Code fences are exempt — a path in sample output is not a reference.

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
DOCS = os.path.join(ROOT, "docs")

existing = {f for f in os.listdir(DOCS) if f.endswith(".md")}
BARE = re.compile(r"docs/([a-z0-9-]+\.md)")
REL_LINK = re.compile(r"\]\((?:docs/)?([a-z0-9-]+\.md)(?:#[a-zA-Z0-9_-]+)?\)")

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
            if name not in existing:
                problems.append(f"{label}:{lineno}: link target {name} does not exist in docs/")


for f in sorted(existing):
    check(os.path.join(DOCS, f), f"docs/{f}")
check(os.path.join(ROOT, "DESIGN.md"), "DESIGN.md")

# Every doc must be placed in the shared curation (the /docs/internal index and the reference
# index both derive from it) — this is what stops a new doc from silently joining no index.
groups_file = os.path.join(ROOT, "website", "src", "lib", "internal-groups.mjs")
curated = re.findall(r"\['([a-z0-9-]+)',", open(groups_file).read())
doc_ids = {f[:-3] for f in existing}
for missing in sorted(doc_ids - set(curated)):
    problems.append(f"docs/{missing}.md is not placed in website/src/lib/internal-groups.mjs")
for ghost in sorted(set(curated) - doc_ids):
    problems.append(f"internal-groups.mjs lists {ghost!r}, which has no docs/{ghost}.md")
seen = set()
for c in curated:
    if c in seen:
        problems.append(f"internal-groups.mjs lists {c!r} twice")
    seen.add(c)

if problems:
    for p in problems:
        print(p)
    print(f"\n{len(problems)} doc-link problem(s)")
    sys.exit(1)
print("doc links OK")
