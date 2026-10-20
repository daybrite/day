---
title: Accessibility
description: "Accessible labels and roles, platform limitations, and checks for the native accessibility tree."
order: 22
section: Guides
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
[example messages](/docs/internal/snippet-catalog) and [catalog setup](/docs/localization).

## Annotating pieces

> [!TIP]
> **An ID is not an accessible name.** Use `id!(close_button)` for automation and
> `res::str::close()` for the title a person reads or hears. Raw text is accepted, but
> accessibility labels, hints, and spoken values need translations just as visible text does.
> Keep those messages in the [Fluent catalog](/docs/localization).

```rust
// An icon-only button: its accessible name can't be derived, so provide one.
button(res::str::close()).icon(Symbol::Close).icon_only()
    .a11y(|a| a.label(res::str::close()))
    .id(id!(close_button))

// A custom-drawn gauge: no native control underneath, so declare role and value.
canvas(move |d, size| draw_gauge(d, size, value.get()))
    .a11y(|a| a.role(Role::Meter).label(res::str::cpu_usage()))
    .frame(120.0, 120.0)

// Decorative artwork: remove it from the accessibility tree entirely.
image(res::images::hero_banner).a11y(|a| a.decorative())
```

`.id` names the piece's own node wherever it sits in the chain. Put `.a11y` before `.frame()`
and `.padding()` on canvas and leaf [pieces](/docs/glossary#piece): those modifiers wrap the
piece in a layout-only node, and `.a11y` annotates the node built so far, so an annotation
placed after them lands on the wrapper instead of the native widget.

The builder covers `label`, `hint`, `value`, `role`, `hidden`, and `decorative`. The three
strings take the same sources as a label (generated `res::str` messages, strings, signals, or closures) and are
live: a localized label follows a [locale](/docs/glossary#locale) switch, and a `value` closure
re-sends whenever what it reads changes. Native controls report their roles on
their own, but no built-in invents a *label* for you; a `toggle` has no title parameter, so a
titled switch is `labeled(res::str::subscribe(), toggle(subscribed))`. Your annotations merge onto the
node's defaults.

Hold yourself to these rules (`day lint` has no a11y rules yet; a missing-label warning for
interactive pieces is a listed follow-up):

- give every interactive Piece whose purpose isn't its text an accessible name;
- never let an element id double as a label; a screen reader would read `"save-button"` aloud.

## Identifiers

`.id(id!(save_button))` sets a stable identifier used by three consumers: [dayscript](/docs/dayscript)
element targeting, external automation tools, and Day's diagnostics. For external tools the
platform mapping is uneven:

| Platform | Identifier surface |
|---|---|
| iOS / macOS | `accessibilityIdentifier` — full support (XCUITest etc.) |
| Windows (XAML, Qt) | UIA `AutomationId` — full support |
| Android | — (the backend maps label, value, and hidden; it sets no automation id today) |
| GTK | no public settable AT-SPI id today — inspector-visible only |
| Web | DOM `id` |

[dayscript](/docs/glossary#dayscript) is unaffected by this table; it resolves ids inside the app, uniformly everywhere.
The table matters only when pointing external tooling (Appium, UIA scrapers) at a Day app.

## Auditing the native tree

Setting an attribute and the platform *exposing* it are different things, so Day includes an
audit step that reads the native accessibility tree back and diffs it against what your code
declared:

```yaml
# in a dayscript flow
- a11y_audit:
```

The audit walks id'd nodes, asks the [toolkit](/docs/glossary#toolkit) for the realized role, label,
and value (via each [backend](/docs/glossary#backend)'s read-back hooks), and fails the script on
mismatch. Run in CI, it fails the build when a declared label stops reaching the native tree.
Read-back is implemented on the Apple backends (AppKit and UIKit); backends that can't yet read
their native tree skip the audit (read-back for Qt and GTK is a listed follow-up).

## Current limits

- **GTK off Linux has no accessibility tree.** GTK's AT-SPI bridge is Linux-only, so the
  `macos-gtk` and `windows-gtk` development combos ([Tier 4](/docs/platforms#support-tiers))
  are invisible to screen readers. Ship the platform-native [target](/docs/glossary#target) for real users.
- **Android annotations are partial:** labels and values map today (`contentDescription`, state
  description); role and hint refinement is still open, and audit read-back isn't implemented.
- **Qt's `QAccessible` layer bridges to the native accessibility API on every OS** (no other
  Day backend does that today), which makes `linux-qt` a reasonable choice when Linux
  accessibility is a hard requirement.
- **Reactive values**: an a11y `value` set at build time is a snapshot; live values (a slider
  announcing as it moves) work through the control's native behavior, but custom [reactive](/docs/glossary#reactive) a11y
  values on canvas pieces are still a designed-not-built refinement.
- **Focus order** follows layout order; explicit focus groups and custom sort priority aren't
  exposed yet.

`day doctor` and the audit step report what each target supports. The
[accessibility reference](/docs/internal/accessibility) has the full per-backend mapping tables.
