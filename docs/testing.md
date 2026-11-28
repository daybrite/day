---
navTitle: "Testing apps"
title: "Test a Day app"
description: "Write UI and headless tests, run them on a selected target, and inspect failures and screenshots."
---

<!-- Copyright © The Daybrite Project; SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Test a Day app

Use `day test` to run tests inside your app on a selected toolkit. Tests can exercise a page
or check logic that needs the app's environment. Run them on each target your app supports.
For walkthroughs written in YAML, see [DayScript](/docs/how-to/dayscript).

## Declare a test

Mark a function with `#[day::test]` and return a
[`Case`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html). A GUI case supplies a
page and an async body that drives it. Test names replace underscores with hyphens:
`button_press` runs as `button-press`. Names must be unique across linked test providers.

Use [element IDs](element-ids.md) to locate controls independently of their translated labels.
The example below uses generated `res::str` accessors; add the
[example messages](/docs/reference/localization#reference-example-messages) to your app's [catalog](/docs/how-to/localization).

```rust
use day::prelude::*;
use day::{Case, Drive};

#[day::test]
fn button_press() -> Case {
    let presses = Signal::new(0i64);
    Case::new()
        .page(move || {
            column((
                button(res::str::press())
                    .action(move || presses.update(|n| *n += 1))
                    .id(id!(btn_press)),
                label(res::str::counter_value(presses)).id(id!(btn_count)),
            ))
        })
        .shot("default")
        .drive(|d: Drive| async move {
            d.tap(id!(btn_press).as_str()).await?;
            d.assert_text(
                id!(btn_count).as_str(),
                &res::str::counter_value(1_i64).format(),
            )
            .await
        })
}
```

Each awaited [`Drive`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html) operation
lets the app process events. Use `tap`, `input`, and `toggle` to interact with controls, and
assertions such as `assert_text`, `assert_value`, and `assert_focused` to check the result.
Gestures have their own operations: `drag`, `fling` (a drag released at a velocity), `press`,
`long_press`, `double_tap`, `touches`, `hover`, `pan`, `pinch` and `rotate`; `key` and
`key_phase` send keys; `drag_enter`, `drag_leave`, `drop_text` and `drop_files` drive a drop
target; `pointer` changes the input kind ([gestures](gestures.md)). `scroll_to` scrolls a
scroll view or a list to an edge, and `reveal` scrolls an element into view, a list row
included ([scroll views](scroll.md)).
Pass [`id!(...).as_str()`](https://docs.rs/day-core/latest/day_core/element_id/struct.ElementId.html#method.as_str) where a drive operation accepts a string ID.

Keep each case focused on one behavior. Declare tests in a module included by the app's test
build. A Cargo feature such as `conformance` can exclude them from shipping builds; enable
that feature when building the test app. A disabled module contributes no tests.

## Host a test page

Wrap the app's root with [`day::test_host`](https://docs.rs/day-app/latest/day/fn.test_host.html):

```rust
pub fn root() -> impl Piece {
    day::test_host(|| app_root())
}
```

The host shows the app until a test starts, then displays each case's page independently.
It disposes the page before the next case and restores the app when the run ends.
A GUI test fails if the app has no test host.

For logic that needs the app's environment without a page, use
[`Case::headless`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.headless)
and [`.run`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.run).
The body receives a `Drive` with `check` and `check_eq` assertions.

## Run tests

Run these commands from the app project directory:

```sh
day test -t macos-appkit
day test -t macos-appkit 'button-*'
day test -t ios-uikit --ios-simulator <udid>
day test -t android-material --shots always
day test -t web-dom --list
```

`day test` builds and launches the app, then reports each selected test's result.
`--skip-build` reuses the last build. `--list` lists registered tests without running them.
The command accepts the same device-selection flags as `day launch`; see the
[CLI reference](/docs/reference/cli#tests-in-the-app).

The default case timeout is 30 seconds. Set `--case-timeout <seconds>` to change it, or use
`0` when debugging without a limit. A case can set its limit with
[`.timeout`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.timeout).
A failed test fails the command. A timed-out case fails and the run continues.

For web tests, build through Day's CLI so the test registry is included in the app.
A bare Cargo build lacks the metadata needed to run those tests.

The [VS Code extension](vscode.md) also runs tests from the Test Explorer and provides
**Debug Test** on desktop targets.

## Control time in a host test

Construct the mock with
[`MockToolkit::new_with_manual_clock()`](https://docs.rs/day-mock/latest/day_mock/struct.MockToolkit.html#method.new_with_manual_clock)
when a test needs to inspect a UI before and after a deadline. Launch with that toolkit, trigger
the action and call [`probe.advance(2999)`](https://docs.rs/day-mock/latest/day_mock/struct.MockProbe.html#method.advance)
to inspect a three-second tooltip just before dismissal. Advance one more millisecond to observe
the dismissal. For touch help, enable
[`set_touch_tooltips`](https://docs.rs/day-mock/latest/day_mock/struct.MockProbe.html#method.set_touch_tooltips) before mounting the piece.

The clock starts at zero. Each advance runs all due UI timers, including ones scheduled by
other callbacks, in deadline order. Equal deadlines run in registration order. An advance of
zero serves timers already due. Requested display frames arrive at 60 Hz on the same clock.
The default mock still completes sleeps immediately.

## Inspect results

Results and captures are written under:

```text
build/day/screenshots/<target>/<variant>/conformance.json
build/day/screenshots/<target>/<variant>/tests/<test>/<shot>.png
build/day/screenshots/<target>/<variant>/tests/<test>/failed.png
```

With a device profile, the path includes `<device>` between the target and variant.
The JSON report records passes, failures, skips, durations, and failure messages.
`--shots on-failure` is the default; use `always` to retain every capture or `never` to disable
captures. Declare a capture with
[`.shot`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.shot) on the case,
or call [`Drive::shot`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.shot)
during the test.

## Interpret a result

A passing test verifies the assertions it ran. Scripted input checks Day's event handling;
it does not verify that a physical tap or a native gesture recognizer produces that event.
Exercise native input separately when testing gestures, text composition, or platform menus.

Use [`assert_native`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.assert_native)
to check properties read from the native widget. Fields a toolkit cannot read appear as
`native_unread` in the report. Treat those as unverified native properties.
Visibility assertions check hidden flags and window membership; a clipped or transparent view
can still report visible.

Use [`assert_displayed`](https://docs.rs/day-core/latest/day_core/conformance/struct.Drive.html#method.assert_displayed)
to check what an element shows at this moment, an animation's in-between state included: its
opacity, translation, frame size and background color, as the toolkit's animator presents them.
The opacity is the element's own times its ancestors', and the translation the element's plus
its ancestors', because a modifier's layer sits above the piece it wraps. The step is read once
and never retried, since a value in flight is only right at the moment it is asked for. To test
an animation, measure how long it has run on the window's [frame clock](frames.md) and compare
with [`AnimSpec::sample`](https://docs.rs/day-spec/latest/day_spec/struct.AnimSpec.html#method.sample)
for that time, as the built-in `animation-*` cases do ([the animation contract](tween.md#the-animation-contract)).
Fields a toolkit cannot observe mid-flight are recorded as `native_unread`.

A case can declare a required capability with
[`.requires`](https://docs.rs/day-core/latest/day_core/conformance/struct.Case.html#method.requires).
Unsupported cases are skipped with a reason. Review skips alongside passes; a skip does not
verify the feature. Snapshot-dependent cases cannot capture on Web DOM.

On targets that unwind panics, a panic in the test body or page construction fails the case
and the run continues. Targets that abort on panic, including the web, terminate the app;
`day test` reports the interrupted run.
