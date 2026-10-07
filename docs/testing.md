---
title: "Tests in the app"
description: "day test: #[day::test] cases that run inside a built Day app on a chosen toolkit, GUI and headless, the conformance app, the results they leave, and the mock harness that runs the same cases under cargo test."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Tests in the app

> **Status: shipped (2026-10).** `#[day::test]`, `Case`, `Drive`, the `tests` and `run_tests`
> engine steps, `day test`, the conformance app under `apps/conformance`, the mock harness in
> `crates/day-script/tests/conformance.rs`, and the `conformance` CI jobs. Cases cover the
> controls, layout and appearance, lists, trees, navigation stacks, sidebars, tabs, covers,
> context menus, toolbars, dialogs, the inspector, gestures, focus and announcements, and the
> pieces in this repository.

`day test` runs tests inside a built Day app on a chosen toolkit. A test is a plain function
marked `#[day::test]` that returns a [`Case`]: either a page plus a drive against it (a GUI
test), or a body with no page (a headless test, for logic that needs the app's own
environment). The same function runs on the mock toolkit under `cargo test`, so a broken
binding or id fails in milliseconds before a toolkit is ever involved.

A test is named after its function, with hyphens for underscores: `button_press` runs as
`button-press`. The name has one source, so the CLI, the report, `conformance.json` and an editor
reading the source all agree on it.

## Declaring a test

```rust
use day::prelude::*;
use day::{Case, Drive};

#[day::test]
fn button_press() -> Case {
    let presses = Signal::new(0i64);
    Case::new()
        .proves(kinds::BUTTON)
        .page(move || column((
            button("Press").action(move || presses.update(|n| *n += 1)).id("btn-press"),
            label(move || presses.get().to_string()).id("btn-count"),
        )))
        .shot("default")
        .drive(|d: Drive| async move {
            d.tap("btn-press").await?;
            d.assert_text("btn-count", "1").await
        })
}

#[day::test]
fn store_round_trip() -> Case {
    Case::headless().run(|t: Drive| async move {
        let store = Store::open()?;
        t.check_eq(store.count(), 0)
    })
}
```

- **`Case::new()`** is a GUI test; `.page(..)` is any piece, which the app's test host shows
  alone while the case runs. **`Case::headless()`** has no page.
- **`.drive(|d| async { .. })`** (or `.run`, which reads better for a headless body) is an
  async body over a [`Drive`]. Each op is awaited, so the app's main loop turns between ops
  the way it does between a script's steps: a transition settles, a capture paints.
- **`.proves(kind)`, `.proves_cap(cap)`, `.proves_duty(name)`** say what a pass marks in the
  coverage tables, in the matrices' own spelling (`kind:day.button`, `cap:Announce`,
  `duty:set_a11y`).
- **`.requires(cap)`** skips the case with a recorded reason where the toolkit answers
  `Unsupported`; **`.requires_support(name, f)`** does the same for a piece's own `support()`
  answer. A case never asks which toolkit it is on. A case is also skipped, not failed, when a
  kind it proves renders a placeholder: the piece has no renderer on that toolkit (a native
  stepper on iOS, a map off the Apple platforms).
- **`.shot(name)`** takes a capture once the page shows, before the drive; `d.shot(name)`
  takes one mid-drive.
- **`.timeout(secs)`** is how long the case may take, page and drive together; unset, the
  run's limit applies (30 s, or `day test --case-timeout`). A case past its limit fails as
  timed out and the run moves on.

`Drive` speaks dayscript's vocabulary: `tap`, `input`, `toggle`, `set_value`, `select`,
`focus`, `submit`, `navigate`, `wait_idle`, `pause`, `shot`, `assert_text`, `assert_visible`,
`assert_missing`, `assert_value`, `assert_on`, `assert_focused`, `assert_route`, `a11y_audit`,
`assert_native` (and `assert_enabled`, its `enabled` shorthand), `assert_frame` (and
`assert_size`), `sample_pixel`, `assert_opened_url`, and for lists, trees and navigation
`activate`, `reorder`, `delete_row`, `swipe_row`, `scroll_to`, `expand`, `tree_move`,
`nav_back`; for chrome and dialogs `context_menu`, `menu`, `toolbar_press`, `toolbar_toggle`,
`dialogs_scripted`, `assert_presented`, `respond`; for input `tap_at`, `drag`, `hover`,
`hover_leave`, `pan`, `pinch`, `key`; and `assert_hidden`, `assert_announced`.
Each is the dayscript step of that name, run in process with the step's own retry window, so a
drive and a script mean the same thing by the same words; a step dayscript lacks is added to the
engine, where a script gets it as well. For a headless body, `check(ok, what)` and
`check_eq(got, want)` are the assertions. A failed op or check ends the test with its message.

### Native checks

Day's own assertions read Day's tree, so on their own they prove the binding and the event
path but not that the platform's widget shows what Day thinks it shows. In a conformance run,
`assert_text`, `assert_value` and `assert_on` are therefore each followed by `assert_native` of
the same fact, read back from the widget through `Toolkit::read_native`: its displayed text,
its value, its checked state. `d.assert_native(id, NativeExpect { .. })` checks what those do
not cover (`enabled`, `visible`), and `d.assert_enabled(id, on)` checks enabled in Day and
natively at once.

`assert_frame` checks an element's size and its origin relative to another element (or the
window content) in Day's layout and in the native frame. `sample_pixel` reads one point of an
in-process capture, given as fractions of the element's frame, and compares it with a
`#rrggbb` within a tolerance that absorbs color management (a Display P3 capture reads sRGB
red as `#ea3323`) but not a different color; the engine decodes the capture with its own small
PNG reader rather than linking a decoder into every app. While a run goes on, `open_url` (a
link, `day::open_url`) records the URL instead of opening it, so `assert_opened_url` can check
it and no browser starts on the machine running the tests.

A field a toolkit cannot read (a secure field's masked text, a toolkit with no capture,
anything a backend has no getter for) does not fail the case: the report lists it as `native_unread` (`"<id> <field>"`), so the
`conformance.json` shows exactly what was proven natively and what only in Day's tree.

### The test host

A GUI case's page needs somewhere to show. An app's test build roots its content in
`day::test_host(..)`:

```rust
pub fn root() -> impl Piece {
    day::test_host(|| app_root())
}
```

The host shows the app's own content until a run starts; then it shows each driven case's page
alone, built fresh for the case and disposed when the next one starts, so one case's ids,
signals and native views never meet another's. When the run ends the app's content returns. A
GUI case in an app with no test host fails with that said.

### When a case panics

A panic in a case's drive, its headless body, or while its page is built fails that case with
the panic's message, and the run goes on to the next case. Where the target aborts on panic
instead of unwinding (the web), the app still goes down and `day test` reports the lost run.

### Where tests go

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

    day_core::tests! { button_press }
}
```

- **Names** are `<piece>_<aspect>` (`button_press`, `text_field_secure`), so
  `day test 'button-*'` selects a piece. A modifier's cases use the modifier's name
  (`padding_insets`).
- **One aspect per case**, named for it, so a failure reads as a sentence.
- **Ids** only need to be unique within the case: the test host shows one case at a time.
- **Each module ends with its roster** (`day_core::tests!`); `crates/day-pieces/src/conformance.rs`
  calls every module's `register_tests()` and concatenates their `roster()`s, and holds the cases
  that belong to no piece and the conformance app's browsing page.
- **What a case proves** is declared with `.proves(kinds::X)`, `.proves_modifier("padding")`,
  `.proves_cap(Cap::X)` and `.proves_duty("x")`. `scripts/ci/conformance-coverage.py` reads
  those and reports, per family, what is proven and what is not. Its `lint.sh` leg gates the
  families already complete (`--require kinds,pieces`: every built-in kind and every piece crate
  has a case, so a new one without one fails lint) and reports the rest; `--check` gates them all once every family is
  covered.

**The pieces in this repository** (`pieces/*`) carry their cases the same way, in a
`pub mod conformance` behind each crate's own `conformance` feature. The conformance app depends
on every one of them with that feature on, and its root calls each crate's `register_tests()`
for the web. A piece from another repository is that repository's to test; the conformance app
takes none. The coverage script counts a piece crate as proven when it has cases, and `lint.sh`
gates that family with the kinds (`--require kinds,pieces`).

An app's own tests take the same shape in its own crate, with `#[day::test]`, behind a feature
its test build turns on.

### Registration

`#[day::test]` adds the function, with its identifier, to a link-time registry (a `linkme`
slice, the mechanism the renderer registry uses). Two test functions with one name, in two
modules or two crates, both fail with that said. `linkme` does not compile for `wasm32-unknown-unknown`, so a crate
also lists its tests once in a `day::tests! { .. }` roster, which expands to
`register_tests()`: a no-op where the slice exists, the registration where it does not. A unit
test in day-pieces holds the roster equal to the slice, so the two cannot drift. A crate below
`day` names the registry's crate: `#[day_macros::test(day_core)]`.

## Running tests

### Under `cargo test`, on the mock

`cargo test -p day-script --test conformance` boots the mock toolkit with the conformance
app's content and runs every registered case through the real engine. A case's page builds,
its drive runs, its assertions read the same probe a script reads.

The harness turns on the mock's native behavior (`MockProbe::set_native_behavior`): what a
native toolkit does unasked, which a unit test otherwise drives by hand. Lists and trees bind a
window of rows (on attach, after a reload, around a scrolled-to row), a cover reports its
size when presented and that it is hidden once dismissed, and the mock claims `Cap::Toolbar` and
`Cap::Dialogs`, whose edits and presentations it records. Those binds and reports happen at
the next `sleep`, where Day's tree is free, so an op that looks for a row too early finds it on
its retry.

### `day test`, on a toolkit

```
day test -p macos-appkit                         # every test in the project
day test -p macos-gtk 'text-field-*'             # a glob over test names
day test -p ios-uikit --ios-simulator <udid>     # the device flags day launch takes
day test -p android-mdc --shots always           # keep every capture, not only a failure's
day test -p linux-gtk --case-timeout 0           # no per-case limit (a debugger attached)
day test -p web-dom --list                       # the registry, without running
```

`day test` is a launch with one generated script: it builds the project for the target
(`--skip-build` reuses the last build), launches it, sends the engine a `run_tests` step and
prints one line per test:

```
      button-press ............................ ok        (95 ms)
      slider-range ............................ skipped   Cap::Animation is Unsupported on this toolkit
      text-field-secure ....................... FAILED    assert_text tfs-len: "13" ≠ "12"
      8 tests: 6 passed, 1 skipped, 1 failed
      Results build/day/screenshots/macos-appkit/default/conformance.json
```

A failed test fails the run the way a failed script step does (exit 5). With no `--project`,
`day test` runs the project in the current directory; from the `day` checkout,
`cargo run -q -p day-cli -- --project apps/conformance test -p <target>` runs Day's own cases.
`--shots` is `on-failure` by default: a failing test's `failed.png` shows what the screen held
when the assertion failed.

### The engine steps

Two dayscript steps carry it, usable from any script:

| step | fields | what it does |
|---|---|---|
| `tests` | — | answers the registered tests (name, kind, what each proves and requires) in the reply's `data` |
| `run_tests` | `filter?` (globs), `shots?` (`never`, `on-failure`, `always`), `timeout_secs?`, `case_timeout_secs?` | runs the matching tests as one main-loop task; retryable while running, so the runner's wait loop polls it; `case_timeout_secs` is each case's limit where it sets none, `0` for none; the reply's `data` is the report |

`apps/conformance/dayscript/conformance.yaml` is the whole run as a script, which is what CI
drives.

### In VS Code

The [Day extension](vscode.md) lists every `#[day::test]` function under **Day Tests** in the
Test Explorer, with the play icon in the gutter beside it and one row per target ticked in the
Day view. Running the function runs `day test` for each of those targets and puts the verdicts on
the rows; a failure's message carries the assertion and links its `failed.png`. **Debug Test** on
a desktop target hands the app to the installed Rust debugger with its engine open, and runs the
tests inside it: a breakpoint in the app holds the run. The extension's own
[testing page](https://vscode.daybrite.dev/docs/testing) has the details.

## What a run leaves

Beside the run's screenshots, the same place a dayscript run writes:

```
build/day/screenshots/<target>/<variant>/conformance.json
build/day/screenshots/<target>/<variant>/tests/<test>/<shot>.png
build/day/screenshots/<target>/<variant>/tests/<test>/failed.png    # on a failure
```

`conformance.json` is the contract between the CLI and whatever reads a run; there is no
command to produce or merge it, only this layout (with a device profile, `<target>/<device>/<variant>/`):

```json
{
  "schema": 1,
  "target": "ios-uikit", "device": null, "variant": "default",
  "day": "0.5.0 (release, branch main, 3fd74cc2)", "commit": "3fd74cc2…", "run": "18522271", "at": 1791304466,
  "tests": {
    "button-press":      { "kind": "gui", "verdict": "pass", "ms": 260, "proves": ["kind:day.button"], "shots": ["default"] },
    "text-field-secure": { "kind": "gui", "verdict": "fail", "ms": 840, "proves": ["kind:day.text_field", "duty:set_input_traits"],
                           "message": "assert_text tfs-len: \"7\" ≠ \"6\"", "shots": ["masked", "shown", "failed"] },
    "slider-range":      { "kind": "gui", "verdict": "skip", "reason": "Cap::Animation is Unsupported on this toolkit", "proves": ["kind:day.slider"], "shots": [] }
  }
}
```

`commit` and `run` come from the CI environment (`GITHUB_SHA`, `GITHUB_RUN_ID`) and are absent
on a laptop rather than guessed; `at` is Unix seconds. `caps` holds what the toolkit answered for
every `Cap` during the run (`N`, `E`, `-`), which CI holds against the declared matrix.

## The conformance app

`apps/conformance` is the app `day test` runs Day's own cases in. It is almost empty: its
root is a test host around a `nav` over every registered GUI case, one route per case, so a
person can open any case on any toolkit and look at it, while a run shows each case alone. It
depends on `day` by path with the `conformance` feature, so it is built against the commit it
sits in and never against a published Day.

Only its own files are committed: `Cargo.toml`, `src/`, `dayscript/` and `generate.sh`. The
scaffold around them (`Day.toml`, `build.rs`, `resource/`, the `platform/` host projects) is
`day new app` output, which `generate.sh` writes from the checkout's own template, so the app
never runs on a host project an older template wrote. Run it once after a clone and again
after a template change; CI runs it on every leg.

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
[docs/coverage-matrix.md](coverage-matrix.md), which is generated from source: a disagreement
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
native reads are `assert_native`, `a11y_audit` (where the toolkit reads its accessibility tree
back), `assert_no_placeholders`, and the captures. Captures are illustration; a case asserts
effects, never looks.

## Known gaps

What a pass does not yet cover, so a skip or an unread field reads as recorded:

- **No capture on the web** (`Cap::Snapshot` is unsupported on DOM), so the pixel cases
  (shapes, backgrounds, opacity, transforms, overlays) skip there. Reading the computed styles
  through `read_native` instead is the planned answer.
- **No accessibility group on GTK and ArkUI**: GTK 4 has no public getters for an accessible's
  label and value, and ArkUI's accessibility text is empty unless Day set it, so `a11y_audit`
  skips their nodes.
- **ArkUI's menu and segmented pickers** are built in ArkTS and report no text.
- **Where Day's content sits in a capture**: `sample_pixel` maps a node's frame into the
  in-app capture through `Toolkit::snapshot_origin`. Android answers it (its content capture
  runs under the status and navigation bars); elsewhere, rows beyond Day's content are taken to
  be above it, true of AppKit's title bar and the mobile status bar.
- **Input is injected, by design.** Every input op delivers Day's own event, the stream a
  native recognizer would, so a gesture case proves the routing and the app's handling, not
  that the platform's recognizer fires on a real press, pan or pinch. Driving real platform
  input from a run (posted events, `adb input`, Playwright) was considered and set aside: each
  platform needs its own driver, and those drivers are the flakiest part of any UI test
  harness.
- **Dialogs** are answered through `respond`, which resolves the request and has the toolkit
  dismiss the dialog it showed: a pass proves `present` and `dismiss` ran on the toolkit and the
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
