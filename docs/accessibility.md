# Accessibility (§13)

> **Status: implemented (annotation pillar).** `A11yProps` (label/hint/value/role/hidden/
> decorative/identifier) now reaches native accessibility APIs on all five backends, and the
> dayscript `a11y_audit` step verifies it in-process on the apple targets.

Native-first: every interactive Piece is a native control, so screen readers, switch access, and
keyboard navigation work at the level the platform provides before Day adds anything. Day's job is
to (a) not break it, (b) provide one uniform annotation API, (c) verify it landed.

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
Annotations merge onto a node: a piece default, `.a11y()`, and `.id()` accumulate (day-core stores
the merged `A11yProps` on the node, re-applies the full picture on each change, and hands it to
`a11y_audit` as the expectation).

Put `.id`/`.a11y` before `.frame()`/`.padding()` on canvas/leaf pieces. Those wrap in a
handle-less layout node, so annotations placed after them wouldn't reach a native widget.

## Roles

`Role`: `None`, `Button`, `Toggle`, `Slider`, `TextInput`, `Heading(u8)`, `Image`, `Meter`, `Group`.

Day only applies an explicit role (the canvas/custom cases, e.g. a `Meter` gauge). Native controls
already report the right role, so Day records their kind-default (`Role::for_kind`) as the audit
expectation but never overrides the widget. `resolved_role(kind)` = explicit role, else the kind
default.

## Per-backend mapping (`set_a11y`)

| field | AppKit | UIKit | GTK | Qt | Android |
|---|---|---|---|---|---|
| label | `setAccessibilityLabel` | `accessibilityLabel` | `Property::Label` | `setAccessibleName` (+tooltip) | `contentDescription` |
| hint | `setAccessibilityHelp` | `accessibilityHint` | `Property::Description` | `setAccessibleDescription` | — (follow-up) |
| value | `setAccessibilityValue` | `accessibilityValue` | `Property::ValueText` | — (QAccessible subclass) | `stateDescription` (API 30+) |
| role (explicit) | `setAccessibilityRole` (AXButton/Slider/CheckBox/TextField/StaticText/Image/LevelIndicator/Group) | `accessibilityTraits` (Button/Adjustable/Header/Image) | construction-time (follow-up) | widget-derived | delegate (follow-up) |
| hidden/decorative | `setAccessibilityElement(false)` | `accessibilityElementsHidden` | `State::Hidden` | — | `importantForAccessibility=NO` |
| identifier | `setAccessibilityIdentifier` | `accessibilityIdentifier` | `set_widget_name` (Inspector) | `setObjectName` | — |

Current state (from §13's truth table): apple + Qt have full native a11y on all their OSes. GTK
has no AT bridge on macOS (it works via AT-SPI on Linux). Android role/hint need an
`AccessibilityDelegate` (deferred). This is why those are secondary combos.

## Verification: `a11y_audit` (§14.2)

The dayscript step `a11y_audit: { id? }` walks Day's id'd nodes, reads each widget's actual
native a11y (`Toolkit::read_a11y`), and diffs identifier + label + value + explicit-role against
Day's stored expectation. Backends that can't read their native tree (`found=false`) skip. Apple
targets implement `read_a11y` (NSAccessibility / UIAccessibility → Day `Role`); it is required in
the CI walkthrough on apple targets and passes there (the showcase gauge audits as
role=Meter/AXLevelIndicator + label + value + id). Role is diffed only for explicit roles Day
applied, since native controls own their roles, which vary per platform.

## Follow-ups

- Reactive a11y strings (`.value_with(|| …)` / `IntoText` on a11y; currently build-time snapshots).
- GTK/Android/canvas construction-time roles; Qt `QAccessibleInterface` value/role.
- `day lint` a11y rule: interactive piece without a derivable label → warning (`--strict` error).
- `read_a11y` for Qt/GTK so `a11y_audit` runs on the desktop-toolkit combos too.
