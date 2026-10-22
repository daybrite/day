---
title: Accessibility
description: "Accessible labels and roles, platform limitations, and checks for the native accessibility tree."
order: 22
section: How-to guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Native controls expose roles and actions to assistive technology, but an app must still label
ambiguous controls and describe meaningful custom graphics. Day’s accessibility API supplies
these annotations and excludes decorative content from the accessibility tree. This guide covers
the annotations, testing, and [current platform limits](#current-limits).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## Annotating pieces

> [!TIP]
> **An ID is not an accessible name.** Use `id!(close_button)` for automation and
> `res::str::close()` for the title a person reads or hears. Raw text is accepted, but
> accessibility labels, hints, and spoken values need translations just as visible text does.
> Keep those messages in the [Fluent catalog](/docs/how-to/localization).

```rust
// Give an icon-only button a localized title and an explicit accessible name.
button(res::str::close()).icon(Symbol::Close).icon_only()
    .a11y(|a| a.label(res::str::close()))
    .id(id!(close_button));

// A custom-drawn gauge needs an explicit role, label, and spoken value.
canvas(move |d, size| draw_gauge(d, size, value.get()))
    .a11y(move |a| {
        a.role(Role::Meter)
            .label(res::str::cpu_usage())
            .value(res::str::percentage(value))
    })
    .frame(120.0, 120.0);

// Decorative artwork: remove it from the accessibility tree entirely.
image(res::images::hero_banner).a11y(|a| a.decorative())
```

`.id` names the piece's own node wherever it sits in the chain. Put `.a11y` before `.frame()`
and `.padding()` on canvas and leaf [pieces](/docs/reference/glossary#piece): those modifiers wrap the
piece in a layout-only node, and `.a11y` annotates the node built so far, so an annotation
placed after them lands on the wrapper instead of the native widget.

The builder covers `label`, `hint`, `value`, `role`, `hidden`, and `decorative`. The three
strings take the same sources as a label (generated `res::str` messages, strings, signals, or closures) and are
live: a localized label follows a [locale](/docs/reference/glossary#locale) switch, and a `value` closure
re-sends whenever what it reads changes. Native controls report their roles on
their own, but no built-in invents a *label* for you; a `toggle` has no title parameter, so a
titled switch is `labeled(res::str::subscribe(), toggle(subscribed))`. Your annotations merge onto the
node's defaults.

Hold yourself to these rules (`day lint` has no a11y rules yet; a missing-label warning for
interactive pieces is a listed follow-up):

- give every interactive Piece whose purpose isn't its text an accessible name;
- never let an element id double as a label; a screen reader would read `"save-button"` aloud.

## Identifiers

`.id(id!(save_button))` sets a stable identifier used by three consumers: [dayscript](/docs/how-to/dayscript)
element targeting, external automation tools, and Day's diagnostics. For external tools the
platform mapping is uneven:

| Platform | Identifier surface |
|---|---|
| iOS / macOS | `accessibilityIdentifier` — full support (XCUITest etc.) |
| Windows (XAML, Qt) | UIA `AutomationId` — full support |
| Android | `uniqueId` through the accessibility delegate on API 33+; compat extras below that |
| GTK | no public settable AT-SPI id today — inspector-visible only |
| Web | DOM `id` |

[dayscript](/docs/reference/glossary#dayscript) is unaffected by this table; it resolves ids inside the app, uniformly everywhere.
The table matters only when pointing external tooling (Appium, UIA scrapers) at a Day app.

## Auditing the native tree

Setting an attribute and the platform *exposing* it are different things, so Day includes an
audit step that reads the native accessibility tree back and diffs it against what your code
declared:

```yaml
# in a dayscript flow
- a11y_audit:
```

The audit walks id'd nodes, asks the [toolkit](/docs/reference/glossary#toolkit) for the realized role, label,
and value (via each [backend](/docs/reference/glossary#backend)'s read-back hooks), and fails the script on
mismatch. Run in CI, it fails the build when a declared label stops reaching the native tree.
All eight toolkit implementations provide a `read_native` hook in this checkout, with differing
field coverage. The audit ignores a node whose snapshot is not found or explicitly invisible;
it compares identifier and declared label/value for the remaining nodes, and explicit roles only
when the native role is known. A successful audit is therefore not a complete screen-reader test.
See [audit logic](https://github.com/daybrite/day/blob/main/crates/day-script/src/lib.rs).

## Current limits

- **GTK off Linux has no accessibility tree.** GTK's AT-SPI bridge is Linux-only, so the
  `macos-gtk` and `windows-gtk` development combos ([Tier 4](/docs/reference/platforms#support-tiers))
  are invisible to screen readers. Ship the platform-native [target](/docs/reference/glossary#target) for real users.
- **Android annotations:** the delegate carries hints, explicit role/class hints, heading state,
  and identifiers as well as labels and values. Identifier exposure differs below API 33.
  [DayBridge](https://github.com/daybrite/day/blob/main/toolkits/day-android/java/dev/daybrite/day/bridge/DayBridge.java)
  defines the mapping; TalkBack behavior still needs runtime review.
- **Qt uses `QAccessible` to expose native accessibility information.** Validate the OS bridge
  and the controls your app uses; an implementation hook is not proof of assistive-technology
  usability on every host.
- **Reactive values:** `A11yBuilder::value` accepts the same reactive text sources as labels.
  Core binds changing values to accessibility patches, including custom pieces. Native exposure
  still depends on backend support and must be checked with read-back or a screen reader.
- **Focus order** follows layout order; explicit focus groups and custom sort priority aren't
  exposed yet.

`day doctor` checks toolchain prerequisites; it does not certify accessibility support.
Use the audit step to inspect realized nodes and a screen reader to check usability. The
[accessibility reference](/docs/reference/accessibility) has the full per-backend mapping tables.

## Verify the declared contract

The Rust blocks are integration fragments with app-defined signals, drawing code, and generated
resources. Source review confirms reactive label/hint/value handling in
[decorators.rs](https://github.com/daybrite/day/blob/main/crates/day-pieces/src/decorators.rs).
Give icon-only controls a localized name, run `a11y_audit` on targets with read-back, then check
keyboard navigation and a screen reader on the intended OS. A skipped audit is not a pass.

## Expose a changing value

Give the icon-only reset button a localized name and the progress display a localized spoken
value. Annotate the native piece before wrapping it in layout modifiers. The caller supplies
an integer percentage; the drawing fraction is clamped to its valid range.

Complete module for `src/accessibility_recipe.rs` in the
[technical recipe harness](/docs/reference/examples/catalog#technical-recipes).
Add the matching Fluent messages below. This module is compiled as part of the harness;
it is not a standalone app.

<!-- technical-example: accessibility_recipe -->
```rust
use crate::res;
use day::prelude::*;

pub fn progress_controls(percent: Signal<i64>) -> impl Piece {
    column((
        progress(move || percent.get().clamp(0, 100) as f64 / 100.0)
            .a11y(move |a| {
                a.label(res::str::a11y_progress())
                    .value(res::str::a11y_percent(percent))
            })
            .id(id!(a11y_progress)),
        button(res::str::a11y_reset())
            .icon(Symbol::Close)
            .icon_only()
            .action(move || percent.set(0))
            .a11y(|a| a.label(res::str::a11y_reset()))
            .id(id!(a11y_reset)),
    ))
    .spacing(Space::M)
    .padding(Space::L)
}
```

<!-- technical-catalog: accessibility_recipe -->
```ftl
a11y_progress = Download progress
a11y_percent = { $percent } percent complete
a11y_reset = Reset progress
```

An element ID locates a node for a test; it does not supply its spoken name. Mock assertions
can check delivered labels and changing values. Screen-reader behavior still requires a
native check on each supported target.
