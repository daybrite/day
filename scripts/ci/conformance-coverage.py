#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
#
# conformance-coverage.py [--check | --require FAMILY,…]: how much of the built-in surface the `#[day::test]` cases
# prove (docs/testing.md). Lists every built-in kind, `Decorate` modifier, `Cap` and toolkit
# duty, reads the `.proves*` calls out of the cases, and prints proven/total per family with
# what is still unproven.
#
# Report mode (the default) always exits 0: it is how the coverage milestones measure
# progress while the backlog is open. `--check` fails on anything unproven and not in
# ALLOWED below; it becomes the gate once every family is covered. `--require kinds` gates the
# named families only (kinds, pieces, modifiers, caps, duties) and reports the rest: a family is gated
# from the milestone that completes it, so a new piece cannot arrive without its case.

import os
import re
import sys

ROOT = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))

# What cannot be proven in-app, each with its reason.
ALLOWED = {
    "cap:Lottie": "no toolkit supports it (docs/coverage-matrix.md)",
    "cap:DragFilePromises": "no toolkit supports it",
    "cap:DragExternalMove": "no toolkit supports it",
    "piece:day-piece-swiftui": "a case needs a SwiftUI view in the app's own Swift sources",
}


def read(*parts):
    with open(os.path.join(ROOT, *parts), encoding="utf-8") as f:
        return f.read()


def block(text, opener):
    """The body of the first `{ … }` block after `opener`."""
    start = text.index(opener)
    i = text.index("{", start)
    depth = 0
    for j in range(i, len(text)):
        if text[j] == "{":
            depth += 1
        elif text[j] == "}":
            depth -= 1
            if depth == 0:
                return text[i + 1 : j]
    return text[i + 1 :]


spec = read("crates", "day-spec", "src", "lib.rs")

# Kinds: `Variant = CONST => "day.kind"` in the `builtin_kinds!` invocation.
kinds_block = block(spec, "\nbuiltin_kinds! {")
kind_of_const = dict(re.findall(r"=\s*([A-Z_]+)\s*=>\s*\"(day\.[a-z_]+)\"", kinds_block))
kinds = sorted(kind_of_const.values())

# Caps: the variants of `pub enum Cap`.
caps = sorted(set(re.findall(r"^\s{4}([A-Z][A-Za-z0-9]*)\b", block(spec, "pub enum Cap"), re.M)))

# Modifiers: the methods of `trait Decorate`, less the type erasure (`any`).
decorate = block(read("crates", "day-pieces", "src", "decorators.rs"), "pub trait Decorate")
modifiers = sorted(set(re.findall(r"^\s{4}fn\s+([a-z_0-9]+)", decorate, re.M)) - {"any"})

# Duties: the first column of the generated duty matrix.
duties = sorted(set(re.findall(r"^\| `([a-z_0-9]+)` \|", read("docs", "duty-matrix.md"), re.M)))

# What the cases prove, read from every Rust file that can hold a case.
proven = set()
case_roots = [os.path.join(ROOT, "crates", "day-pieces", "src"), os.path.join(ROOT, "pieces")]
for root in case_roots:
    for dirpath, dirs, files in os.walk(root):
        dirs[:] = [d for d in dirs if d not in ("target", "build", "node_modules")]
        for name in files:
            if not name.endswith(".rs"):
                continue
            with open(os.path.join(dirpath, name), encoding="utf-8") as f:
                text = f.read()
            for const in re.findall(r"\.proves\(\s*kinds::([A-Z_]+)\s*\)", text):
                if const in kind_of_const:
                    proven.add("kind:" + kind_of_const[const])
            proven |= {"cap:" + c for c in re.findall(r"\.proves_cap\(\s*Cap::(\w+)\s*\)", text)}
            proven |= {"duty:" + d for d in re.findall(r"\.proves_duty\(\s*\"(\w+)\"\s*\)", text)}
            proven |= {"modifier:" + m for m in re.findall(r"\.proves_modifier\(\s*\"(\w+)\"\s*\)", text)}

# Pieces: the crates under pieces/, each proven by cases of its own (a `#[day::test]` in its
# sources, which the conformance app runs with the crate's `conformance` feature on).
pieces_dir = os.path.join(ROOT, "pieces")
pieces = sorted(d for d in os.listdir(pieces_dir) if os.path.isdir(os.path.join(pieces_dir, d)))
for piece in pieces:
    for dirpath, dirs, files in os.walk(os.path.join(pieces_dir, piece, "src")):
        for name in files:
            if name.endswith(".rs"):
                with open(os.path.join(dirpath, name), encoding="utf-8") as f:
                    if "#[day_macros::test" in f.read():
                        proven.add("piece:" + piece)

families = [
    ("kinds", "kind", kinds),
    ("pieces", "piece", pieces),
    ("modifiers", "modifier", modifiers),
    ("caps", "cap", caps),
    ("duties", "duty", duties),
]

args = sys.argv[1:]
check = "--check" in args
required = set()
if "--require" in args:
    required = set(args[args.index("--require") + 1].split(","))
unknown = required - {title for title, _, _ in families}
if unknown:
    print(f"conformance coverage: no family {', '.join(sorted(unknown))}")
    sys.exit(2)
missing_total = 0
missing_required = 0
for title, prefix, items in families:
    keys = [f"{prefix}:{i}" for i in items]
    done = [k for k in keys if k in proven]
    allowed = [k for k in keys if k not in proven and k in ALLOWED]
    missing = [k for k in keys if k not in proven and k not in ALLOWED]
    missing_total += len(missing)
    if title in required:
        missing_required += len(missing)
    gated = " (gated)" if title in required else ""
    print(f"{title}{gated}: {len(done)}/{len(keys)} proven" + (f", {len(allowed)} allowed" if allowed else ""))
    if missing:
        print("  unproven: " + ", ".join(k.split(":", 1)[1] for k in missing))

if check and missing_total:
    print(f"conformance coverage: {missing_total} item(s) have no proving case")
    sys.exit(1)
if missing_required:
    print(
        f"conformance coverage: {missing_required} gated item(s) have no proving case; add a "
        "`#[day::test]` case that `.proves` it (docs/testing.md)"
    )
    sys.exit(1)
print("conformance coverage: report" + (" — complete" if missing_total == 0 else ""))
