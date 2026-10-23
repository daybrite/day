# Declared element IDs and dayscript coverage

Declare an element identity at the piece. No resource catalog or build script is involved.

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
to a builder on each call. There is no arbitrary-string conversion. The dynamic registry accepts `DynValue::ElementId` supplied by the compiled host; strings
are rejected there too. Every element-bearing
setter and builder takes the typed value; the old keyed modifier has been removed.

## Keys and composite controls

`IntoElementKey` accepts integer and string values, and explicit application implementations.
It deliberately does not accept arbitrary Display values. A custom key must have a stable,
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

Retain [`.id_of`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id_of) for recycled rows. A one-time [`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) would keep identifying the old item after
rebinding. A tree's [`.row_id`](https://daybrite.dev/api/day_pieces/trait.TreeBuilder.html#tymethod.row_id) also takes a closure returning ElementId; its canonical string
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
Document strips accept [`.strip_id(id!(documents))`](https://daybrite.dev/api/day_pieces/struct.DocumentTabs.html#method.strip_id); library-owned tab, close and new buttons
use `document_tab`, `document_close`, and `document_new` families with encoded host/row keys.
These changes break old recordings. Route/model/preference keys remain their own data and
are not mechanically renamed just because they resemble an element ID.

## Registration and platform behavior

Native `id!` expansions emit linkme records; wasm32 expansions emit framed byte records into
the `day_ids` custom section. A small hidden procedural attribute also emits a unique WASM
export per declaration to retain its archive member, including unused generic helpers.
`#[used]` alone retains an object-file record but does not force the linker to include that
archive member. The exports are internal linker anchors; the loader never invokes them. The DOM loader reads every custom section from the final module,
then calls `day_dom_install_ids` before application startup. There is no mount-time insertion
into the declaration catalog. The custom section includes a version sentinel, even for an
empty catalog. Missing/truncated/unsupported metadata is a script-session error.

The normalized registry includes exact family, static/keyed shape, and declaring package
name/version. Repeated declarations of one family within a package count once. Different
owners or static/keyed shapes conflict and fail before a walkthrough runs. Native debug
records also carry source locations; release/web records omit those paths. Ordinary unused
helpers in linked crates contribute declarations. Entirely unlinked dependencies do not.
Dynamically loaded libraries do not automatically merge into the executable's registry.

Rust controls cfg/features, so offline lint does not reproduce Cargo's target evaluation.
`day lint` tokenizes macro calls to provide advisory shape-conflict diagnostics; it does not
expand custom macros or evaluate cfg. The runtime catalog is authoritative for scripts.

The full canonical locator remains the tree/accessibility/DOM identifier. [`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) still names
the semantic piece node independent of wrapper order; [`.wrapper_id`](https://daybrite.dev/api/day_pieces/struct.DocumentTabs.html#method.wrapper_id) explicitly names the
current wrapper. IDs do not replace localized accessibility labels.

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
available. Checkpoint writes are buffered before flushing and atomic replacement. JSON is the persisted report; CLI prints the suite score. No new HTML report viewer
or editor completion service is required to use the gate.

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
retried, or mixed-session/target/profile reports. It never globs an output directory. Session
identity is a hash, not the authentication token. This first release intentionally requires
one launch session for aggregation; independent launches/shards need an artifact identity
protocol before they can safely be combined. Separate targets always have separate gates.
A zero denominator is not 100%, and an enabled minimum fails for zero eligible declarations.
Percentages are compared using counts, without rounding to the displayed score. Test failure
always fails the invocation independently of coverage.

## Validation and limits

Regression coverage lives in `day-core/src/element_id.rs` (exact names, key escaping,
exactly-once evaluation, unused generic retention, framing), `day-script-proto/src/ids.rs`
(namespace/shape/canonical locator validation), and the CLI lint/coverage tests (tokenization,
set unions, thresholds, exclusions, missing/mixed evidence). The app walkthroughs exercise
real native and web registration. Release retention must be checked on final packaged modules
and binaries, since a downstream optimizer can discard custom metadata.
`python3 scripts/ci/element-ids.py` checks native and WASM debug/stripped-LTO output, unused
dependency modules and generic helpers, repeated caller-macro expansions, and cfg exclusion.
The same regression runs in CI.

The macro frontend invokes linkme's procedural attribute on native platforms and the small
`day-macros` retention attribute on WASM;
“no build scanner” does not mean “zero compile/editor overhead.” Measure no-op and edited-ID
builds, stripped artifact size, and editor diagnostics on the actual app. The registry is
normalized once per process. Reusable helpers may need Clone adjustments. Cross-package
family collisions require renaming/namespacing; there is no implicit owner-based name rewrite.
