---
title: "Accessibility"
description: "Uniform accessibility annotations over each platform's native tree: labels, values, focus order, stable identifiers, and the CI audit that diffs them."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Accessibility (§13)

Day starts with the accessibility behavior of its native controls. App code supplies the
meaning those controls cannot infer: a label for an icon-only button, a summary for a chart,
or the role and value of a custom drawing.

`A11yProps` carries those annotations to the native accessibility APIs. The dayscript
`a11y_audit` step checks the declared metadata against the native tree on Apple targets.
Test keyboard navigation and screen-reader behavior on the platforms you ship; annotations
alone do not establish that a screen is usable.

## Authoring

```rust
button(icon("trash"))
    .a11y(|a| a.label(tr("delete-item").format()).hint(tr("delete-item-hint").format()))
    .id("delete-button")

image("chart").a11y(|a| a.label(tr("q3-chart-summary").format()))   // or .decorative()

gauge(level).a11y(|a| a.role(Role::Meter).label("Volume").value("72"))   // canvas → explicit role
```

`A11yBuilder`: `.label`, `.hint`, `.value`, `.role(Role)`, `.hidden()`, `.decorative()`
(decorative ⇒ hidden + exempt from the "needs a label" lint). `.id(_)` sets the identifier.
The three strings take what a `label` takes: a literal, a `String`, a `Signal<String>` or a
closure. A closure is re-read whenever what it reads changes and the new string is re-sent on
its own, so a gauge's spoken value follows the gauge and a localized label follows the locale:

```rust
canvas(move |d, size| …)
    .a11y(move |a| a.role(Role::Meter).label("Level").value(move || format!("{:.0}%", level.get())))
```
Annotations merge onto a node: a piece default, `.a11y()`, and `.id()` accumulate (day-core stores
the merged `A11yProps` on the node, re-applies the full picture on each change, and hands it to
`a11y_audit` as the expectation).

Put `.id`/`.a11y` before `.frame()`/`.padding()` on canvas/leaf pieces. Those wrap in a
handle-less layout node, so annotations placed after them wouldn't reach a native widget.

## Roles

`Role`: `None`, `Button`, `Toggle`, `Slider`, `TextInput`, `Heading(u8)`, `Image`, `Meter`, `Group`,
`Tree`, `TreeItem`.

Day only applies an explicit role (the canvas/custom cases, e.g. a `Meter` gauge). Native controls
already report the right role, so Day records their kind-default (`Role::for_kind`) as the audit
expectation but never overrides the widget. `resolved_role(kind)` = explicit role, else the kind
default.

## Per-backend mapping (`set_a11y`)

| field | AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI | web |
|---|---|---|---|---|---|---|---|---|
| label | `accessibilityLabel` | `accessibilityLabel` | `Property::Label` | `accessibleName` (+ tooltip) | `contentDescription` | `AutomationProperties.Name` | `NODE_ACCESSIBILITY_TEXT` | `aria-label` |
| hint | `accessibilityHelp` | `accessibilityHint` | `Property::Description` | `accessibleDescription` | `hintText` through a delegate | `HelpText` | `NODE_ACCESSIBILITY_DESCRIPTION` | `aria-description` |
| value | `accessibilityValue` | `accessibilityValue` | `Property::ValueText` | `QAccessibleInterface::text(Value)` | `stateDescription` (API 30+; appended to the label below) | `ItemStatus` | `NODE_ACCESSIBILITY_VALUE` | `aria-valuetext` (+ `aria-valuenow`) on meters and sliders, else in `aria-description` |
| role (explicit) | `accessibilityRole` | `accessibilityTraits` (button, adjustable, header, image) | `accessible-role`, set before the widget meets an AT | `QAccessibleInterface::role()` | `className` and `heading` through a delegate | `HeadingLevel`; `LocalizedControlType` for the rest | `NODE_ACCESSIBILITY_ROLE` | `role` (+ `aria-level`) |
| hidden / decorative | `accessibilityElement = false`, no children | `isAccessibilityElement = false`, descendants hidden | `State::Hidden` | `invisible` + `offscreen` state, no children | `importantForAccessibility = NO_HIDE_DESCENDANTS` | `AccessibilityView.Raw` | `NODE_ACCESSIBILITY_MODE` disabled for descendants | `aria-hidden`, out of the tab order |
| identifier | `accessibilityIdentifier` | `accessibilityIdentifier` | `widget name` (Inspector only) | `objectName` | `uniqueId` through the delegate (API 33+; node extras below) | `AutomationId` | `NODE_ID` | `id` |

What each platform does not carry:

- **Heading levels** reach WinUI, the web and GTK (`Property::Level`). AppKit, UIKit, Android
  and ArkUI know a heading from other text and nothing more; AppKit's heading is `AXHeading`.
- **Roles are advisory where the platform cannot retype an element.** WinUI speaks a control
  type string (English) and keeps the native patterns; ArkUI's `Tree`/`TreeItem` become a list
  and its rows; Android's `Tree`/`TreeItem` and UIKit's `TextInput`/`Meter`/`Group`/`Tree` have
  no counterpart. A canvas with `Role::Button` on the web becomes keyboard-reachable
  (`tabindex`, Enter and Space tap it).
- **Qt** realizes value, role and hidden through its own `QAccessibleInterface` for a widget
  that asked for any of them; the heading level needs Qt 6.8.
- **Android** reaches hint, role and the identifier through one `AccessibilityDelegate` per
  annotated view, wrapping whatever delegate the widget already had. The identifier is the
  node's `uniqueId`, which UiAutomator and Appium address from API 33; below that the compat
  call keeps it in the node's extras, where only an in-process reader finds it (§13's table).
- **GTK**'s role is set only while the widget has no root, which is when day-core first applies
  annotations; a role changed later is ignored. A plain Day container on **WinUI** is a `Canvas`,
  which has no automation peer, so annotations on a bare container are stored but not exposed.

## Announcements

`day::announce(text)` speaks a sentence through the screen reader without moving its focus;
`day::announce_urgent(text)` interrupts what is being read. Use them for the change a user
cannot otherwise learn about ("Saved", "3 results", "Upload failed"); a visible status label
that the user can reach needs none. Nothing happens when no screen reader is running.

| | AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI | web |
|---|---|---|---|---|---|---|---|---|
| `Cap::Announce` | Native | Native | Native on GTK 4.14+ | Native on Qt 6.8+ | Native | Native | Native | Emulated |
| mechanism | `NSAccessibilityAnnouncementRequested` notification on the window | `UIAccessibilityAnnouncementNotification`; polite announcements queue | `gtk_accessible_announce`, looked up at run time | `QAccessibleAnnouncementEvent` | `announceForAccessibility`; urgent interrupts first | `AutomationPeer.RaiseNotificationEvent` | `accessibility.sendAccessibilityEvent` from the ArkTS host | two ARIA live regions |

GTK makes the call only through an AT-SPI context, which exists only once the accessibility
bus answered: GTK 4.14 reaches the announcement through its no-AT context otherwise and the
process ends. On Windows the GTK build cannot look the symbol up and answers `Unsupported`.

## Verification: `a11y_audit` (§14.2)

The dayscript step `a11y_audit: { id? }` walks Day's id'd nodes, reads each widget's actual
native a11y (`Toolkit::read_a11y`), and diffs identifier + label + value + explicit-role against
Day's stored expectation. Backends that can't read their native tree (`found=false`) skip. The
Apple targets implement `read_a11y` (NSAccessibility / UIAccessibility → Day `Role`), and so
does the web, where the ARIA attributes on the element are the tree (`role`, `aria-label`,
`aria-valuetext` or else `aria-description`, `id`). It is required in the CI walkthrough on those
targets and passes there (the showcase gauge audits as role=Meter + label + value + id, twice:
once more after its slider moves, which is what shows the reactive value landing natively). Role
is diffed only for explicit roles Day applied, since native controls own their roles, which vary
per platform.

## Follow-ups

- `day lint` a11y rule: interactive piece without a derivable label → warning (`--strict` error).
- `read_a11y` for GTK, Qt, Android, WinUI and ArkUI so `a11y_audit` runs there too.
