#!/usr/bin/env bash
# Regenerate docs/coverage-matrix.md — WHICH PIECE KINDS each backend actually renders, and what
# each answers for every `Cap`. Companion to duty-matrix.sh: that one proves the Toolkit trait is
# implemented, this one proves the vocabulary is. CI runs both and fails on drift.
#
# Why generate it: a backend with no renderer for a kind draws a `⟨kind⟩` placeholder, which no
# screenshot and no other assertion can see. Prose tables tracking that went stale repeatedly
# (docs/harmonyos.md claimed images were placeholders long after they were real nodes), so the
# table is derived from the code instead — and dayscript's `assert_no_placeholders` allow-lists in
# apps/showcase/dayscript/walkthrough.yaml are the runtime half of the same fact.
#
#     scripts/ci/coverage-matrix.sh
set -euo pipefail
cd "$(dirname "$0")/../.."

python3 - <<'EOF'
import re
from pathlib import Path

BACKENDS = [
    ("appkit", "toolkits/day-appkit/src/lib.rs", "lib-appkit.rs"),
    ("uikit", "toolkits/day-uikit/src/lib.rs", "lib-uikit.rs"),
    ("gtk", "toolkits/day-gtk/src/lib.rs", "lib-gtk.rs"),
    ("qt", "toolkits/day-qt/src/lib.rs", "lib-qt.rs"),
    ("xaml", "toolkits/day-xaml/src/lib.rs", "lib-xaml.rs"),
    ("android", "toolkits/day-android/src/lib.rs", "lib-android.rs"),
    ("arkui", "toolkits/day-arkui/src/lib.rs", "lib-arkui.rs"),
    ("dom", "toolkits/day-dom/src/lib.rs", "lib-dom.rs"),
]
spec = Path("crates/day-spec/src/lib.rs").read_text()
sources = {name: Path(path).read_text() for name, path, _ in BACKENDS}


def body_of(src: str, header: re.Pattern) -> str:
    """The brace-balanced body of the first fn matching `header` (so a mention of a kind in a
    comment or in capability() is never mistaken for a realize arm)."""
    m = header.search(src)
    if not m:
        return ""
    i = src.index("{", m.end() - 1)
    depth, j = 0, i
    while j < len(src):
        if src[j] == "{":
            depth += 1
        elif src[j] == "}":
            depth -= 1
            if depth == 0:
                return src[i : j + 1]
        j += 1
    return src[i:]


REALIZE = re.compile(r"\bfn realize\s*\(")
CAPABILITY = re.compile(r"\bfn capability\s*\(")
realize_bodies = {n: body_of(sources[n], REALIZE) for n, _, _ in BACKENDS}
cap_bodies = {n: body_of(sources[n], CAPABILITY) for n, _, _ in BACKENDS}

# ---- built-in kinds ------------------------------------------------------------------------
# Parsed from the `builtin_kinds!` table in day-spec — the single source the Builtin enum, the
# wire keys, and the `kinds::*` constants are all generated from.
kinds = re.findall(
    r'^\s*(\w+) = (\w+) => "([^"]+)",', body_of(spec, re.compile(r"\bbuiltin_kinds!\s*")), re.M
)
assert kinds, "no builtin_kinds! entries found in day-spec — did the table move?"
# ListCell is never realized: day-core adopts the recycled native cell as the handle.
kinds = [(variant, key) for variant, _const, key in kinds if variant != "ListCell"]

# ---- external pieces ----------------------------------------------------------------------
# A piece implements a backend when it ships that backend's arm file. An empty feature (e.g.
# day-piece-map's `qt = []`) exists so an app can enable it uniformly, but registers no renderer.
pieces = []
for crate in sorted(Path("pieces").iterdir()):
    src = crate / "src"
    if not (crate / "Cargo.toml").exists() or not src.is_dir():
        continue
    # Kind constants are conventionally `KIND`, but a multi-kind piece names them per kind
    # (day-piece-datetime: DATE_KIND / TIME_KIND), so match any const whose value is a kind.
    kind_consts = re.findall(
        r'pub const \w*KIND\w*: &str = "([^"]+)"', (src / "lib.rs").read_text()
    )
    if not kind_consts:
        continue  # a pure-composition piece (rating/badge) has no native kind
    arms = {n for n, _, arm in BACKENDS if (src / arm).exists()}
    pieces.append((crate.name, kind_consts, arms))

# ---- caps ---------------------------------------------------------------------------------
caps = re.findall(r"\n    (\w+),", body_of(spec, re.compile(r"\bpub enum Cap\s*\{")))


def cap_answer(backend: str, cap: str) -> str:
    """Parse one backend's `capability()` match. Handles the three shapes in the tree: a grouped
    arm (which may carry `//` comments BETWEEN variants), a braced arm body, and a runtime
    conditional (reported `?` — a static table cannot resolve it; day-dom's NavSplit depends on
    the viewport width)."""
    body = re.sub(r"//[^\n]*", "", cap_bodies[backend])
    for m in re.finditer(
        r"((?:Cap::\w+\s*\|\s*)*Cap::\w+)\s*=>\s*\{?\s*(Support::(\w+)|if\b)", body
    ):
        if cap in re.findall(r"Cap::(\w+)", m.group(1)):
            if m.group(2) == "if":
                return "?"
            return {"Native": "N", "Emulated": "E"}.get(m.group(3), "\u2013")
    m = re.search(r"_\s*=>\s*\{?\s*Support::(\w+)", body)
    if m:
        return {"Native": "N", "Emulated": "E"}.get(m.group(1), "\u2013")
    return "?"


names = [n for n, _, _ in BACKENDS]
out = [
    "# Coverage matrix",
    "",
    "<!-- GENERATED by scripts/ci/coverage-matrix.sh — do not edit by hand. CI diffs this file",
    "     against a fresh run, so regenerate after adding a kind, a renderer, or a capability. -->",
    "",
    "What each backend actually renders. A kind with no renderer draws a visible `⟨kind⟩`",
    "placeholder instead of failing, so these gaps are invisible in a screenshot — the showcase",
    "walkthrough asserts the same facts at runtime via `assert_no_placeholders`.",
    "",
    "## Built-in kinds",
    "",
    "`✓` = the backend's `realize` handles it; `·` = it falls through to the placeholder.",
    "",
    "| kind | " + " | ".join(names) + " |",
    "|---|" + "---|" * len(names),
]
for variant, kind in kinds:
    row = [f"`{kind}`"]
    # Word-boundary match, NOT a substring test: `Builtin::List` is a prefix of
    # `Builtin::ListCell`, which every realize body mentions in its fallback arm.
    arm = re.compile(r"\bBuiltin::" + re.escape(variant) + r"\b")
    for n in names:
        row.append("✓" if arm.search(realize_bodies[n]) else "·")
    out.append("| " + " | ".join(row) + " |")

out += [
    "",
    "## External pieces",
    "",
    "`✓` = the crate ships that backend's renderer arm; `·` = no arm, so the kind renders the",
    "placeholder. A piece may still be absent at runtime if the app does not enable its feature,",
    "and one arm (`day-piece-webview` on GTK) is further limited to Linux hosts.",
    "",
    "| piece | kind(s) | " + " | ".join(names) + " |",
    "|---|---|" + "---|" * len(names),
]
for crate, kind_consts, arms in pieces:
    row = [f"`{crate}`", ", ".join(f"`{k}`" for k in kind_consts)]
    for n in names:
        row.append("✓" if n in arms else "·")
    out.append("| " + " | ".join(row) + " |")

out += [
    "",
    "## Capabilities",
    "",
    "What each backend answers for `Cap` (`capability()`): `N` native, `E` emulated, `–`",
    "unsupported, `?` decided at runtime (day-dom answers `NavSplit` from the viewport width).",
    "An app branches on this rather than on the target name.",
    "",
    "| cap | " + " | ".join(names) + " |",
    "|---|" + "---|" * len(names),
]
for cap in caps:
    out.append(
        "| `" + cap + "` | " + " | ".join(cap_answer(n, cap) for n in names) + " |"
    )
out.append("")

Path("docs/coverage-matrix.md").write_text("\n".join(out))
print(
    f"docs/coverage-matrix.md: {len(kinds)} kinds, {len(pieces)} pieces, {len(caps)} caps "
    f"x {len(BACKENDS)} backends"
)
EOF
