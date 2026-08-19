#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0

# web-dom ABI lint (scripts/ci/lint.sh "web shim ABI" leg).
#
# The web backend and its host shim call each other across two SEPARATE name spaces, and nothing
# in the build checks that either side is holding up its end:
#
#   IMPORTS  Rust declares in an `extern "C"` block and the shim provides on the `env` object.
#            A missing one is a WebAssembly LinkError at instantiate — loud, but it surfaces as a
#            blank app whose cause reads like a backend bug (a stale installed CLI is the usual
#            reason: the app is new, the embedded shim is not).
#   EXPORTS  Rust marks `pub extern "C" fn` and the shim calls as `wasm.name(...)`.
#            A missing one is a plain `TypeError: undefined is not a function` inside a DOM event
#            handler, which the browser swallows: the button simply does nothing. The sidebar
#            toggle shipped that way — `wasm.day_dom_toolbar_sidebar()` names an IMPORT, so the
#            call was undefined and the handler died before toggling anything.
#
# So both directions are checked. The rule to remember when writing shim code: a verb the shim
# DEFINES is called through `env.`, and a verb Rust EXPORTS is called through `wasm.`.

import os
import re
import sys

ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
SHIM = os.path.join(ROOT, "crates", "day-cli", "resources", "web", "shim.js")

js = open(SHIM, encoding="utf-8").read()

# The `env` object literal: everything the shim hands the module as an import.
start = js.index("const env = {")
depth, i = 0, start + len("const env = ")
while True:
    if js[i] == "{":
        depth += 1
    elif js[i] == "}":
        depth -= 1
        if depth == 0:
            break
    i += 1
env_block = js[start : i + 1]
provided = set(re.findall(r"^\s+(day_\w+)\s*[:(]", env_block, re.M))

# `wasm.name(` — a call into the module, which must therefore be an export.
called = set(re.findall(r"wasm\.(\w+)\s*\(", js))

def rust_files(*roots):
    for root in roots:
        for dirpath, dirs, files in os.walk(os.path.join(ROOT, root)):
            dirs[:] = [d for d in dirs if d not in ("target", "node_modules")]
            for f in files:
                if f.endswith(".rs"):
                    yield os.path.join(dirpath, f)


# IMPORTS come only from code in the WEB build: the backend and each piece/part/tweak's `-dom`
# arm. A tweak's qt arm declares qt imports, which shim.js has no business providing.
import_sources = [os.path.join(ROOT, "toolkits", "day-dom", "src", "lib.rs")]
import_sources += [f for f in rust_files("pieces", "parts", "tweaks") if f.endswith("-dom.rs")]

# EXPORTS can live anywhere — `day_dom_main` is written by a macro in `crates/day`, and the parts
# export their own completion callbacks — so this half looks at every crate.
imports, exports = set(), set()
for path in rust_files("crates", "toolkits", "pieces", "parts", "tweaks"):
    try:
        rs = open(path, encoding="utf-8").read()
    except OSError:
        continue
    exports |= set(re.findall(r'pub\s+extern\s+"C"\s+fn\s+(\w+)', rs))
    exports |= set(re.findall(r'#\[export_name\s*=\s*"(\w+)"\]', rs))
for path in import_sources:
    try:
        rs = open(path, encoding="utf-8").read()
    except OSError:
        continue
    # Import blocks, which may or may not carry the wasm_import_module attribute.
    for blk in re.findall(r'extern\s+"C"\s*\{(.*?)\n\}', rs, re.S):
        imports |= {n for n in re.findall(r"\bfn\s+(\w+)\s*\(", blk) if n.startswith("day_")}

missing_imports = sorted(imports - provided)
missing_exports = sorted(n for n in called if n.startswith("day_") and n not in exports)

for name in missing_imports:
    print(f"shim.js provides no `{name}` — Rust imports it, so the module fails to instantiate")
for name in missing_exports:
    print(
        f"shim.js calls `wasm.{name}(...)` but no Rust code exports it "
        f"({'it is an env import — call it as `env.' + name + '(...)`' if name in provided else 'no such export'})"
    )

n = len(missing_imports) + len(missing_exports)
print(
    f"web shim ABI: {len(imports)} import(s), {len([c for c in called if c.startswith('day_')])} "
    f"export call(s) — {n} problem(s)"
)
sys.exit(1 if n else 0)
