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
[`Case`](https://daybrite.dev/api/day_core/conformance/struct.Case.html). A GUI case supplies a
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

Each awaited [`Drive`](https://daybrite.dev/api/day_core/conformance/struct.Drive.html) operation
lets the app process events. Use `tap`, `input`, and `toggle` to interact with controls, and
assertions such as `assert_text`, `assert_value`, and `assert_focused` to check the result.
Pass [`id!(...).as_str()`](https://daybrite.dev/api/day_core/element_id/struct.ElementId.html#method.as_str) where a drive operation accepts a string ID.

Keep each case focused on one behavior. Declare tests in a module included by the app's test
build. A Cargo feature such as `conformance` can exclude them from shipping builds; enable
that feature when building the test app. A disabled module contributes no tests.

## Host a test page

Wrap the app's root with [`day::test_host`](https://daybrite.dev/api/day/fn.test_host.html):

```rust
pub fn root() -> impl Piece {
    day::test_host(|| app_root())
}
```

The host shows the app until a test starts, then displays each case's page independently.
It disposes the page before the next case and restores the app when the run ends.
A GUI test fails if the app has no test host.

For logic that needs the app's environment without a page, use
[`Case::headless`](https://daybrite.dev/api/day_core/conformance/struct.Case.html#method.headless)
and [`.run`](https://daybrite.dev/api/day_core/conformance/struct.Case.html#method.run).
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
[`.timeout`](https://daybrite.dev/api/day_core/conformance/struct.Case.html#method.timeout).
A failed test fails the command. A timed-out case fails and the run continues.

For web tests, build through Day's CLI so the test registry is included in the app.
A bare Cargo build lacks the metadata needed to run those tests.

The [VS Code extension](vscode.md) also runs tests from the Test Explorer and provides
**Debug Test** on desktop targets.

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
[`.shot`](https://daybrite.dev/api/day_core/conformance/struct.Case.html#method.shot) on the case,
or call [`Drive::shot`](https://daybrite.dev/api/day_core/conformance/struct.Drive.html#method.shot)
during the test.

## Interpret a result

A passing test verifies the assertions it ran. Scripted input checks Day's event handling;
it does not verify that a physical tap or a native gesture recognizer produces that event.
Exercise native input separately when testing gestures, text composition, or platform menus.

Use [`assert_native`](https://daybrite.dev/api/day_core/conformance/struct.Drive.html#method.assert_native)
to check properties read from the native widget. Fields a toolkit cannot read appear as
`native_unread` in the report. Treat those as unverified native properties.
Visibility assertions check hidden flags and window membership; a clipped or transparent view
can still report visible.

A case can declare a required capability with
[`.requires`](https://daybrite.dev/api/day_core/conformance/struct.Case.html#method.requires).
Unsupported cases are skipped with a reason. Review skips alongside passes; a skip does not
verify the feature. Snapshot-dependent cases cannot capture on Web DOM.

On targets that unwind panics, a panic in the test body or page construction fails the case
and the run continues. Targets that abort on panic, including the web, terminate the app;
`day test` reports the interrupted run.
