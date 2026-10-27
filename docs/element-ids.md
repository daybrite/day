---
navTitle: "Element IDs"
title: "Element IDs"
description: "Name controls for UI tests, use stable keys for repeated rows, and measure DayScript element coverage."
---

<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Element IDs

Give controls stable names so [DayScript](/docs/how-to/dayscript) can find them in any UI language.
Declare each identity at the piece with `id!`.

```rust
button(res::str::save()).id(id!(account::save))
label(state).id(id!(camera_state))
row(content).id_of(move || id!(query_row, slot.item().key()))

fn readout(text: impl Fn() -> String + 'static, id: ElementId) -> impl Piece {
    label(text).tabular().id(id)
}
readout(value, id!(volume_value))
```

The macro is exported through `day`, `day-pieces`, `day-core`, and the usual preludes. Each
segment uses lowercase ASCII Rust-style names: no keywords, raw identifiers, leading or
trailing underscores, doubled underscores, or case conversion. Namespaces are written with
`::`. These tokens are declarations, not references to generated Rust symbols: IDE symbol
rename/completion does not manage a vocabulary. Rename source and script references together.

`ElementId` is Clone. A reusable `Fn` closure must clone a captured ID if it gives the value
to a builder on each call. Element setters and builders take
the typed value; arbitrary strings cannot be converted into IDs.

## Keys and composite controls

`IntoElementKey` accepts integer and string values, and explicit application implementations.
It does not accept arbitrary Display values. A custom key must have a stable,
unambiguous canonical text representation. The macro evaluates the key expression once.

```rust
.id(id!(query_row, 42_u64))         // query_row:42
.id(id!(account::row, ":a b"))    // account::row:%3Aa%20b
```

Only key bytes are escaped: UTF-8 outside `A-Z a-z 0-9 - . _ ~` uses uppercase percent encoding.
Integer 42 and text "42" are the same key. Empty keys are allowed (`row:`). Scripts retain
scalar locators; no tagged object schema is required:

```yaml
- tap: { id: 'query_row:42' }
- assert_text: { id: camera_state, key: camera_running }
```

Retain [`.id_of`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id_of) for recycled rows. A one-time [`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id) would keep identifying the old item after
rebinding. A tree's [`.row_id`](https://docs.rs/day-pieces/latest/day_pieces/trait.TreeBuilder.html#tymethod.row_id) also takes a closure returning ElementId; its canonical string
is used in `expand.row` and `tree_move.row/parent` references.

Composite controls declare their distinct targets explicitly:

```rust
rating(stars)
    .star_id(|n| id!(rating_star, n))
    .id(id!(rating_row))
legend(entries).entry_id(|key| id!(series_legend, key))
```

Framework-owned settings children use declared families such as `pref:<preference-key>`,
`pref_increment:<encoded-control-id>`, `prefs_group:<group-key>` and `about_link:<index>`.
Document strips accept [`.strip_id(id!(documents))`](https://docs.rs/day-pieces/latest/day_pieces/struct.DocumentTabs.html#method.strip_id); library-owned tab, close and new buttons
use `document_tab`, `document_close`, and `document_new` families with encoded host/row keys.
Route, model, and preference keys are separate from element IDs.

## Validation and accessibility

A family must have the same owner and static or keyed shape throughout the app. Use namespaces
to avoid collisions between packages. Conflicts fail before a walkthrough runs.
`day lint` checks source declarations, but does not expand custom macros or evaluate Cargo
features. The compiled app validates the declarations used on the selected target.

IDs identify controls in Day's tree, accessibility tree, and DOM. They do not replace
localized accessibility labels. See [accessibility](accessibility.md).
[`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id) names the piece
independently of wrapper order. Document tabs also offer
[`.wrapper_id`](https://docs.rs/day-pieces/latest/day_pieces/struct.DocumentTabs.html#method.wrapper_id)
to name the current wrapper.

## Reports and gates

A scripted launch validates every target-applicable element reference before its first
step. An unknown or wrongly shaped reference is an error even for an absence assertion;
remove obsolete tests for IDs which no longer exist rather than declaring them in a driver.

Enable family-level coverage in the app's Day.toml:

```toml
[dayscript.coverage]
enabled = true
# Add a minimum after reviewing a complete suite's baseline:
minimum = 80

[dayscript.coverage.targets.ios-uikit]
minimum = 75

[[dayscript.coverage.exclusions]]
id = "camera_title"
targets = ["ios-uikit"]
reason = "This target uses the native navigation heading."
```

Setting a minimum also enables reporting and the gate, even without `enabled = true`.

The example exclusion is only appropriate when that app actually has that declaration and
behavior. Unknown exclusions and empty reasons fail. Coverage reports are saved alongside
existing dayscript reports under `build/day/reports/dayscript`. They retain the compiled
catalog, operation outcomes, family-only evidence, cumulative mounted families, and uncovered
families, plus never-mounted and mounted-but-unexercised diagnostics when a mount snapshot is
available. Reports are saved as JSON; the CLI prints the suite score.

The primary score is `|interacted ∪ asserted| / eligible declared families`. Repeated keys,
actions and retries do not multiply credit. Waits are observed-only; negative assertions and
screenshots earn no exercised credit. Failed steps and optional absent actions earn none.
A named label can be exercised by a positive assertion. Unnamed controls, all possible key
values, visual fidelity, and business correctness are not measured. Treat this as element-ID
coverage, not total UI coverage. Some operations without an explicit locator (for example,
a menu command selected only by localized label) conservatively receive no family credit.

Launch multiple scripts together to gate their union. Reports from one target/profile and
one launch session can also be checked later with an explicit suite manifest:

```json
{
  "schema_version": 1,
  "target": "web-dom",
  "reports": {
    "dayscript/demo.yaml": "reports/demo-final.json",
    "dayscript/settings.yaml": "reports/settings-final.json"
  },
  "policy": { "minimum": 80 }
}
```

```sh
day coverage check --manifest suite.json
```

Paths are manifest-relative. The command fails on missing, failed, incomplete, duplicate,
retried, or mixed-session/target/profile reports. It never globs an output directory. Aggregation requires
one launch session; reports from independent launches cannot be combined. Separate targets always have separate gates.
A zero denominator is not 100%, and an enabled minimum fails for zero eligible declarations.
Percentages are compared using counts, without rounding to the displayed score. Test failure
always fails the invocation independently of coverage.
