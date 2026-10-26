<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Element ID registration notes

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

## Validation and limits

Regression coverage lives in `day-core/src/element_id.rs` (exact names, key escaping,
exactly-once evaluation, unused generic retention, framing), `day-script-protocol/src/ids.rs`
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
