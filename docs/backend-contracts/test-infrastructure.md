<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Framework test infrastructure

## Organize framework cases


A piece's cases go in a `mod conformance` at the end of the file that defines it, the way unit
tests go in `mod tests`, behind the crate's `conformance` feature so a shipping app links none
of them:

```rust
// crates/day-pieces/src/leaves.rs, after the constructors
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::conformance::{Case, Drive};
    use crate::*;

    #[day_macros::test(day_core)]
    fn button_press() -> Case { /* … */ }

}
```

- **Names** are `<piece>_<aspect>` (`button_press`, `text_field_secure`), so
  `day test 'button-*'` selects a piece. A modifier's cases use the modifier's name
  (`padding_insets`).
- **One aspect per case**, named for it, so a failure reads as a sentence.
- **Ids** only need to be unique within the case: the test host shows one case at a time.
- **Declare each test once**, with the attribute. There is no per-module roster or startup
  registration call. `crates/day-pieces/src/conformance.rs` holds the cases that belong to no
  piece and the conformance app's browsing page.
- **What a case proves** is declared with [`.proves(kinds::X)`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.proves), [`.proves_modifier("padding")`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.proves_modifier),
  [`.proves_cap(Cap::X)`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.proves_cap) and [`.proves_duty("x")`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.proves_duty). `scripts/ci/conformance-coverage.py` reads
  those and reports, per family, what is proven and what is not. Its `lint.sh` leg gates the
  families already complete (`--require kinds,pieces`: every built-in kind and every piece crate
  has a case, so a new one without one fails lint) and reports the rest; `--check` gates them all once every family is
  covered.

**The pieces in this repository** (`pieces/*`) carry their cases the same way, in a
`pub mod conformance` behind each crate's own `conformance` feature. The conformance app depends
on every one of them with that feature on. A dependency used only for its tests must enter
rustc's dependency graph with `use provider_crate as _;` on every target. A piece from another repository is that repository's to test; the conformance app
takes none. The coverage script counts a piece crate as proven when it has cases, and `lint.sh`
gates that family with the kinds (`--require kinds,pieces`).

An app's own tests take the same shape in its own crate, with `#[day::test]`, behind a feature
its test build turns on.

### Registration

`#[day::test]` is the only test declaration. Native targets retain their static `linkme`
slice; WASM uses `inventory` constructors. A crate below `day` uses
`#[day_macros::test(day_core)]`. Public case names still replace underscores with hyphens;
discovery identities also include the package, version and module path. Duplicate compiled
case names fail before filtering. The runtime [`register`](https://docs.rs/day-core/latest/day_core/conformance/fn.register.html) API is for synthetic harness cases.

On WASM the attribute emits a descriptor export. Its address references the descriptor, so
rustc requires the object's inclusion. The Day CLI acts as the final linker driver, records
these required exports **before** invoking rust-lld, then appends a versioned
`day_test_expectations` custom section to the output. This handles target cfg and LTO without
source scanners, archive parsing, a second compilation, or handwritten test names. Rustc's
export contract and runtime inventory are independent: a retained descriptor does not hide
an omitted registration constructor.

The DOM host installs the manifest and invokes constructors once before app startup. [`tests`](https://docs.rs/day-core/latest/day_core/conformance/fn.tests.html)
and `run_tests` compare exact compiled identities before any filter or capability skip. Missing,
unexpected or duplicate entries fail nonretryably, with the missing names and no executed tests.
Absent/malformed expectations fail closed, including when a packaging tool strips the custom
section. Custom WASM hosts must install the section with
`day_core::conformance::install_web_registry`. Bare Cargo builds lack the audited manifest and
cannot run WASM conformance tests; use `day build`. Ordinary app functionality is unaffected.

An entire crate removed from the compiler graph cannot contribute an export. Test-suite apps
can require providers in Cargo.toml; `apps/conformance` does this for every contributing crate:

```toml
[package.metadata.day.conformance]
required-providers = ["my-test-app", "my-widget-crate"]
```

A required provider with no test exports fails the web build, including an accidentally disabled
feature or missing `use crate as _`. This is a suite-level dependency contract, not a catalog of
individual tests. Tests intentionally disabled by cfg are absent from expectations. Removing a
provider from both Cargo and this contract is an explicit source change to review.

No whole-archive linking, forced LTO or codegen-unit restriction is needed. Inventory adds static
nodes and constructor code, performs registration once, and traverses its list for discovery;
it does not allocate per-entry nodes or add work to the rendering loop. Tests behind a disabled
conformance feature contribute neither descriptors nor constructors. Other Day registries keep
their existing implementations.

`scripts/ci/conformance-registry.py` exercises real WASM artifacts in debug, one-codegen-unit
debug, ordinary release and stripped full-LTO release. It covers unused dependency modules,
registration inside an uncalled generic helper, cfg on both sides of the attribute, repeated
initialization, missing/corrupt expectations, and deliberate loss of a constructor and of an
entire provider. The loss cases must fail; a green run of the surviving tests is insufficient.


## Running framework tests on the mock


`cargo test -p day-script --test conformance` boots the mock toolkit with the conformance
app's content and runs every registered case through the real engine. A case's page builds,
its drive runs, its assertions read the same probe a script reads.

The harness turns on the mock's native behavior ([`MockProbe::set_native_behavior`](https://docs.rs/day-mock/latest/day_mock/struct.MockProbe.html#method.set_native_behavior)): what a
native toolkit does unasked, which a unit test otherwise drives by hand. Lists and trees bind a
window of rows (on attach, after a reload, around a scrolled-to row), a cover reports its
size when presented and that it is hidden once dismissed, and the mock claims `Cap::Toolbar` and
`Cap::Dialogs`, whose edits and presentations it records. Those binds and reports happen at
the next [`sleep`](https://docs.rs/day-core/latest/day_core/fn.sleep.html), where Day's tree is free, so an op that looks for a row too early finds it on
its retry.


## The conformance app

`apps/conformance` is the app `day test` runs Day's own cases in. It is almost empty: its
root is a test host around a [`nav`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html) over every registered GUI case, one route per case, so a
person can open any case on any toolkit and look at it, while a run shows each case alone. It
depends on `day` by path with the `conformance` feature, so it is built against the commit it
sits in and never against a published Day.

Only its own files are committed: `Cargo.toml`, `src/`, `dayscript/`, `scripts/` and `generate.sh`. The
scaffold around them (`Day.toml`, `build.rs`, `resource/`, the `platform/` host projects) is
`day new app` output, which `generate.sh` writes from the checkout's own template, so the app
never runs on a host project an older template wrote. Run it once after a clone and again
after a template change; CI runs it on every leg.

### Native ArkUI long-press tests

`apps/conformance/scripts/harmony-long-press.py` checks the existing
`GestureKind::LongPress` toolkit hook with real OS touch input. After building the conformance
app for `harmony-arkui`, run it with one connected, unlocked emulator/device and `hdc` on `PATH`:

```sh
target/debug/day --project apps/conformance build -t harmony-arkui
python3 apps/conformance/scripts/harmony-long-press.py
# Use another CLI binary with --day /path/to/day; --hdc selects the hdc executable.
```

The driver launches an opt-in root (`DAY_NATIVE_LONG_PRESS_TEST=1`), obtains coordinates from
the native UI hierarchy, and injects touches using the SDK's `uinput` tool. Day's engine only
controls fixture buttons and reads assertions; it never synthesizes a long-press event.
Checks cover short taps, one callback per hold despite repeated registration, local point
coordinates at the actual display density, scroll cancellation, disposal during a pending
hold, removal from the acceptance handler, repeated remounting, and native list drag arbitration
(the child hook owns its hit region; the rest of the row still drags). This is a separate
native-input suite, not a registered `day test` case or part of the shared CI conformance run; a synthetic
event test would not prove the recognizer works. The fixture does not introduce a public
`.on_long_press` API. Normal launches without the environment flag retain the usual test host.

Do not run this alongside another driver for the same app. It leaves the fixture visible for
inspection; a normal `day launch` or `day test` restores the regular conformance app.

### Native ArkUI dialog tests

After building the conformance app for `harmony-arkui`, run:

```sh
python3 apps/conformance/scripts/harmony-dialogs.py
```

This requires one connected, unlocked HarmonyOS target and `hdc` on `PATH`; `--day` and
`--hdc` can select other executables. It launches `DAY_NATIVE_DIALOG_TEST=1` and drives actual
ArkUI dialog buttons, native text editing, IME submit, Back and the modal mask. Checks cover
startup presentation, Unicode/quoted titles, prompt initial/empty/cancelled values,
action-sheet indices and safe-area placement, concurrent requests, queued dismissal,
closing during opening, immediate chained presentations, and light/dark surfaces. The fixture
remains visible afterwards.

The portable `confirm-native-*`, `prompt-native-*`, and `alert-sheet-preserves-button-indices`
cases run in the regular conformance suite on targets with `Cap::Dialogs`. They explicitly
select native presentation and answer through [`respond`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.respond) / [`respond_text`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.respond_text), exercising the
public future and toolkit dismissal contract. Those assertions inspect pending Day requests;
they do not prove a native button was drawn or clicked. The separate Python suite supplies
that evidence and is currently an explicit local/emulator check, not a shared CI step.

### Native ArkUI text-area tests

After building the conformance app for `harmony-arkui`, run:

```sh
python3 apps/conformance/scripts/harmony-textarea.py
```

Requires one unlocked target and `hdc` on `PATH` (`--day`/`--hdc` override executables). The
`DAY_NATIVE_TEXTAREA_TEST=1` fixture starts read-only. OS input checks typing/deletion/newlines,
copy/cut/paste, app writes, reactive editing/selection changes, long-press selection, keyboard
selection, and unmount/remount. Assertions read both the native TextArea and the bound model.
Only fixture buttons and assertions use Day's driver; no text events are injected.

Run this separately from other drivers for the same app. It leaves the fixture visible; normal
`day launch`/`day test` restores the test host. The portable `text-area-attributes-reactive` case
checks content preservation through policy changes on supporting backends. This native-input
suite supplies the OS-input evidence and is an explicit emulator check, not a shared CI step.

For TextArea [`.on_submit(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextAreaBuilder.html#tymethod.on_submit), run the companion suite:

```sh
python3 apps/conformance/scripts/harmony-textarea-submit.py
```

It launches `DAY_NATIVE_TEXTAREA_SUBMIT_TEST=1` and uses real Enter/Shift+Enter keys, native paste,
and the soft keyboard's **Send** button. It checks callback counts and current bound text,
Unicode selection/caret handling, native undo/redo, read-only suppression, callbacks that clear
or remove the editor, remounting, and ordinary multiline editors. The soft-keyboard check requires an IME
whose Send button is exposed as `Send` in the native accessibility tree (the test emulator's
English keyboard); it never substitutes a synthetic submit event for that check.

The portable `text-area-submit` case checks the public callback and clearing contract using
injected text/submit events on every backend. As with text-field submission, this portable
case alone cannot prove native keyboard support.

Run only one driver for the app at a time.

## Native ArkUI link tests

After building the conformance app for `harmony-arkui`, run:

```sh
python3 apps/conformance/scripts/harmony-links.py
```

The script launches `DAY_NATIVE_LINK_TEST=1` on the connected emulator. It uses OS taps in
native Text bounds for every link activation; Day's driver only operates fixture controls
and checks results. It verifies multiple targets in one label, Unicode targets, plain text
and trailing whitespace, first/last wrapped lines, selectable labels, `TextBuilder` links,
repeated target changes, linked/plain transitions, removal/remounting and default navigation.
The fixture uses fixed text and font sizes; run at the emulator's default font scale. These
tests do not claim keyboard traversal or screen-reader coverage. Run separately from other
drivers for the app. A normal `day launch` or `day test` restores the regular test host.

The portable `label-link-runs-reactive` conformance case checks native text preservation
through rebuilding links and switching to/from plain text on all supporting targets. Existing
mock tests cover the shared `LinkActivated` handler, custom-handler and route policies.

## In CI

`ci.yml`'s `conformance` job runs the app through `daybrite/actions`' `dayapp.yml` on all nine
targets, with the run's own CLI, one phone profile per mobile OS, and `dayscript/conformance.yaml`
as the script; its `setup-command` is `generate.sh`, run with that CLI. Each leg uploads its `screenshots/` tree (so `conformance.json` and the captures
ride along) as `conformance-screenshots-<target>[-<slug>]`; `conformance-results` merges the
`conformance.json` files into one object keyed by the path each was written at
(`<target>[/<device>]/<variant>`), copies the captures beside it under `shots/`, and uploads the
pair as the `conformance-results` artifact. A failed test fails its leg.

The same job then runs `scripts/ci/conformance-claims.py` over the merged results. Each leg's
`caps` (what its toolkit answered during the run) is held against
[docs/coverage-matrix](../coverage-matrix.md), which is generated from source: a disagreement
fails the job, naming the target and capability, because the published matrix would then be
wrong for that target (a `?` cell, decided at run time, accepts any answer and reports it). The
script also writes `verified-matrix.md` into the artifact: per built-in kind and target, `✓`
where every case proving the kind passed, `✗` where one failed, `·` where all were skipped. The screenshot
artifacts carry the caller's `artifact-prefix` since 2026-10, which is what lets this call of
the workflow sit beside the showcase's in one run.

## What a pass means

A pass says the toolkit realized the pieces, the drive's events reached the app's signals, the
assertions held on day-core's state, and the native widget reports the same text, value and
state, except for the fields `conformance.json` lists as `native_unread`. It does not say native
INPUT would have produced the same: the drive's events are injected, as a script's are. The
native reads are [`assert_native`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.assert_native), [`a11y_audit`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.a11y_audit) (where the toolkit reads its accessibility tree
back), `assert_no_placeholders`, and the captures. Captures are illustration; a case asserts
effects, never looks.

## Known gaps

What a pass does not yet cover, so a skip or an unread field reads as recorded:

- **No capture on the web** (`Cap::Snapshot` is unsupported on DOM), so the pixel cases
  (shapes, backgrounds, opacity, transforms, overlays) skip there. Reading the computed styles
  through `read_native` instead is the planned answer.
- **No accessibility group on GTK and ArkUI**: GTK 4 has no public getters for an accessible's
  label and value, and ArkUI's accessibility text is empty unless Day set it, so [`a11y_audit`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.a11y_audit)
  skips their nodes.
- **ArkUI's menu and segmented pickers** are built in ArkTS and report no text.
- **Where Day's content sits in a capture**: [`sample_pixel`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.sample_pixel) maps a node's frame into the
  in-app capture through [`Toolkit::snapshot_origin`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#method.snapshot_origin). Android answers it (its content capture
  runs under the status and navigation bars); elsewhere, rows beyond Day's content are taken to
  be above it, true of AppKit's title bar and the mobile status bar.
- **Input is injected, by design.** Every input op delivers Day's own event, the stream a
  native recognizer would, so a gesture case proves the routing and the app's handling, not
  that the platform's recognizer fires on a real press, pan or pinch. Driving real platform
  input from a run (posted events, `adb input`, Playwright) was considered and set aside: each
  platform needs its own driver, and those drivers are the flakiest part of any UI test
  harness.
- **Dialogs** are answered through [`respond`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.respond), which resolves the request and has the toolkit
  dismiss the dialog it showed: a pass proves [`present`](https://docs.rs/day-core/latest/day_core/fn.present.html) and `dismiss` ran on the toolkit and the
  answer came back, not that the dialog drew. A system dialog is its own window, out of reach of
  the in-app capture on several platforms.
- **Context menus and toolbars** are driven through Day's own model of them, the dispatch a
  native choice goes through; the native menu never opens, and on a phone a toolbar declared
  without a navigation host has no native bar to appear on.
- **Visibility** reads hidden flags and window membership; a view with opacity 0 or clipped
  away still reports visible.

## Follow-ups

- The website's coverage pages and per-piece galleries, read from the `conformance-results`
  artifact of the latest green run on `main`.
- `--watch` and a reusable detached app between runs; a `--baseline` comparison against the
  last published results.
- A scratch directory for headless tests, on the app's own data directory per platform.
- The remaining built-in pieces' cases.
