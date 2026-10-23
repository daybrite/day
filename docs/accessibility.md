---
title: "Accessibility"
description: "Uniform accessibility annotations over each platform's native tree: labels, values, focus order, stable identifiers, and the CI audit that diffs them."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Accessibility

Day starts with the accessibility behavior of its native controls. App code supplies the
meaning those controls cannot infer: a label for an icon-only button, a summary for a chart,
or the role and value of a custom drawing.

`A11yProps` carries those annotations to the native accessibility APIs. The dayscript
`a11y_audit` step checks the declared metadata against the native tree on Apple targets.
Test keyboard navigation and screen-reader behavior on the platforms you ship; annotations
alone do not establish that a screen is usable.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
button(res::str::delete_item()).icon(Symbol::Delete).icon_only()
    .a11y(|a| a.label(res::str::delete_item()).hint(res::str::delete_item_hint()))
    .id(id!(delete_button))

image(res::images::chart).a11y(|a| a.label(res::str::q3_chart_summary()))   // or .decorative()

gauge(level).a11y(|a| a.role(Role::Meter).label(res::str::volume()).value(res::str::percentage(level)))   // canvas → explicit role
```

`A11yBuilder`: [`.label`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.label), [`.hint`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.hint), [`.value`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.value), [`.role(Role)`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.role), [`.selected(bool)`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.selected), [`.hidden()`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.hidden),
[`.decorative()`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.decorative) (decorative ⇒ hidden + exempt from the "needs a label" lint). [`.id(_)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) sets the
identifier. [`.selected`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.selected) is the platform's selected state for one item among peers, a `Role::Tab`
or a row; it takes a `bool`, a `Signal<bool>` or a closure and follows it. A [`button`](https://daybrite.dev/api/day_pieces/fn.button.html)`(..)`[`.selected`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.selected)`(..)`
carries the same state through the control itself ([docs/buttons](buttons.md)), so the annotation is for
custom chrome and drawn pieces.
The three strings take what a [`label`](https://daybrite.dev/api/day_pieces/fn.label.html) takes: a literal, a `String`, a `Signal<String>` or a
closure. A closure is re-read whenever what it reads changes and the new string is re-sent on
its own, so a gauge's spoken value follows the gauge and a localized label follows the locale:

```rust
canvas(move |d, size| …)
    .a11y(move |a| a.role(Role::Meter).label(res::str::level()).value(res::str::percentage(level)))
```
Annotations merge onto a node: a piece default, [`.a11y()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y), and [`.id()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) accumulate (day-core stores
the merged `A11yProps` on the node, re-applies the full picture on each change, and hands it to
`a11y_audit` as the expectation).

[`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) names the piece's own node wherever it sits in the chain. [`.a11y`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y) is an ordered
modifier ([docs/api-style](api-style.md) "Modifier classes"): it annotates the node built so
far, so put it before [`.frame()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame)/[`.padding()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding) on canvas/leaf pieces, which wrap in a handle-less
layout node that annotations placed after them would land on.

## Roles

`Role`: `None`, `Button`, `Toggle`, `Slider`, `TextInput`, `Heading(u8)`, `Image`, `Meter`, `Group`,
`Tree`, `TreeItem`, `Tab`, `TabList`.

`Tab` and `TabList` are what the composed document strip applies to its title buttons and their
row ([docs/navigation](navigation.md) "Document tabs"): a tab button on AppKit, `role="tab"` on the web, GTK's
and Qt's tab roles, a button with the selected trait or state on UIKit and Android, where no
single-tab role exists.

Day only applies an explicit role (the canvas/custom cases, e.g. a `Meter` gauge). Native controls
already report the right role, so Day records their kind-default ([`Role::for_kind`](https://daybrite.dev/api/day_spec/enum.Role.html#method.for_kind)) as the audit
expectation but never overrides the widget. [`resolved_role(kind)`](https://daybrite.dev/api/day_spec/struct.A11yProps.html#method.resolved_role) = explicit role, else the kind
default.

## Per-toolkit mapping (`set_a11y`)

Platform guidance: [Apple: accessibility](https://developer.apple.com/design/human-interface-guidelines/accessibility); [Android: accessibility principles](https://developer.android.com/guide/topics/ui/accessibility/principles); [Windows: accessible apps](https://learn.microsoft.com/en-us/windows/apps/design/accessibility/accessibility).

| field | AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI | web |
|---|---|---|---|---|---|---|---|---|
| label | `accessibilityLabel` | `accessibilityLabel` | `Property::Label` | `accessibleName` (+ tooltip) | `contentDescription` | `AutomationProperties.Name` | `NODE_ACCESSIBILITY_TEXT` | `aria-label` |
| hint | `accessibilityHelp` | `accessibilityHint` | `Property::Description` | `accessibleDescription` | `hintText` through a delegate | `HelpText` | `NODE_ACCESSIBILITY_DESCRIPTION` | `aria-description` |
| value | `accessibilityValue` | `accessibilityValue` | `Property::ValueText` | [`QAccessibleInterface::text(Value)`](https://doc.qt.io/qt-6/qaccessibleinterface.html) | `stateDescription` (API 30+; appended to the label below) | `ItemStatus` | `NODE_ACCESSIBILITY_VALUE` | `aria-valuetext` (+ `aria-valuenow`) on meters and sliders, else in `aria-description` |
| role (explicit) | `accessibilityRole` (+ the tab-button subrole for `Tab`) | `accessibilityTraits` (button, adjustable, header, image, tab bar) | `accessible-role`, set before the widget meets an AT | [`QAccessibleInterface::role()`](https://doc.qt.io/qt-6/qaccessibleinterface.html) | `className` and `heading` through a delegate | `HeadingLevel`; `LocalizedControlType` for the rest | `NODE_ACCESSIBILITY_ROLE` | `role` (+ `aria-level`) |
| selected | `accessibilitySelected` | the selected trait | `State::Selected` | `selected` state through Day's interface | [`View.setSelected`](https://developer.android.com/reference/android/view/View) | not applied: no automation property short of a custom peer | not applied: no state attribute in the bound NDK | `aria-selected` |
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
- **Qt** realizes value, role and hidden through its own [`QAccessibleInterface`](https://doc.qt.io/qt-6/qaccessibleinterface.html) for a widget
  that asked for any of them; the heading level needs Qt 6.8.
- **Android** reaches hint, role and the identifier through one [`AccessibilityDelegate`](https://developer.android.com/reference/android/view/View.AccessibilityDelegate) per
  annotated view, wrapping whatever delegate the widget already had. The identifier is the
  node's `uniqueId`, which UiAutomator and Appium address from API 33; below that the compat
  call keeps it in the node's extras, where only an in-process reader finds it.
- **GTK**'s role is set only while the widget has no root, which is when day-core first applies
  annotations; a role changed later is ignored. A plain Day container on **WinUI** is a `Canvas`,
  which has no automation peer, so annotations on a bare container are stored but not exposed.

## Announcements

[`day::announce(text)`](https://daybrite.dev/api/day_core/fn.announce.html) speaks a sentence through the screen reader without moving its focus;
[`day::announce_urgent(text)`](https://daybrite.dev/api/day_core/fn.announce_urgent.html) interrupts what is being read. Use them for the change a user
cannot otherwise learn about ("Saved", "3 results", "Upload failed"); a visible status label
that the user can reach needs none. Nothing happens when no screen reader is running.

| | AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI | web |
|---|---|---|---|---|---|---|---|---|
| `Cap::Announce` | Native | Native | Native on GTK 4.14+ | Native on Qt 6.8+ | Native | Native | Native | Emulated |
| mechanism | [`NSAccessibilityAnnouncementRequested`](https://developer.apple.com/documentation/appkit/nsaccessibility-swift.struct/notification/announcementrequested) notification on the window | [`UIAccessibilityAnnouncementNotification`](https://developer.apple.com/documentation/uikit/uiaccessibility/notification/announcement); polite announcements queue | [`gtk_accessible_announce`](https://docs.gtk.org/gtk4/method.Accessible.announce.html), looked up at run time | [`QAccessibleAnnouncementEvent`](https://doc.qt.io/qt-6/qaccessibleannouncementevent.html) | `announceForAccessibility`; urgent interrupts first | `AutomationPeer.RaiseNotificationEvent` | [`accessibility.sendAccessibilityEvent`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-accessibility-kit/js-apis-accessibility.md) from the ArkTS host | two ARIA live regions |

GTK makes the call only through an AT-SPI context, which exists only once the accessibility
bus answered: GTK 4.14 reaches the announcement through its no-AT context otherwise and the
process ends. On Windows the GTK build cannot look the symbol up and answers `Unsupported`.

## Reduced motion

Every platform has a switch for less motion, and users who set it do so for vestibular reasons
or because animation distracts them. Day honors it in one place: a gate in day-core that every
animation Day drives consults.

- **What it covers.** [`with_animation`](https://daybrite.dev/api/day_core/fn.with_animation.html), implicit [`.animation()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.animation), enter and exit pairs,
  `set_frame`/`set_opacity`/`set_transform` transitions, canvas `Tweened` values, [`animate`](https://daybrite.dev/api/day_core/tween/fn.animate.html)
  and programmatic scrolls. Under the gate each finite transition lands at its destination,
  and a tween already in flight finishes on its next frame. Nothing cross-fades: the
  destination is applied as it is.
- **What keeps moving.** An endless tween (`repeat: u32::MAX`): a spinner or a pulse that is
  the only sign of work. Native transitions a toolkit owns (a navigation push) are the
  toolkit's, and the toolkits that animate them read the gate before doing so.
- **What is the app's.** Motion driven from `day::frame` (a simulation, a chart's easing) is
  the app's own, and Day cannot know which of it is decoration. [`day::reduce_motion()`](https://daybrite.dev/api/day/fn.reduce_motion.html) is the
  reactive read: a closure reading it re-runs when the setting flips, so the app settles or
  slows what it drives.

```rust
let frames = day::frame::subscribe(move |f| {
    if day::reduce_motion() { marbles.settle() } else { marbles.step(f) }
    ControlFlow::Continue(())
});
```

The gate is the user's system setting ([`Toolkit::reduce_motion`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.reduce_motion), re-read when the toolkit's
observer reports a flip) OR a launch that forced it. `day launch --fast` forces it: fast mode
is this setting and nothing more, so a scripted run shows what a user who asked for less motion
sees. `DAY_REDUCE_MOTION=1` is the same force under the name an app or a screenshot variant
would use. `Cap::ReduceMotion` says whether the toolkit reads a real setting; where it does
not, apps on that platform animate as with the setting off, and the force still applies.

| | AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI | web |
|---|---|---|---|---|---|---|---|---|
| setting | Reduce Motion | Reduce Motion | Animations off ([`gtk-enable-animations`](https://docs.gtk.org/gtk4/property.Settings.gtk-enable-animations.html)) | — | Remove animations (animator scale 0) | Show animations off | animator scale 0 | `prefers-reduced-motion` |
| read | `accessibilityDisplayShouldReduceMotion` | `isReduceMotionEnabled` | [`GtkSettings`](https://docs.gtk.org/gtk4/class.Settings.html) | — | `Settings.Global.ANIMATOR_DURATION_SCALE` | [`UISettings.AnimationsEnabled`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.viewmanagement.uisettings.animationsenabled) | `settings.display.ANIMATOR_DURATION_SCALE` (ArkTS host) | `matchMedia` |
| change | workspace options notification | status-did-change notification | `notify::gtk-enable-animations` | — | settings [`ContentObserver`](https://developer.android.com/reference/android/database/ContentObserver) | `AnimationsEnabledChanged` | `settings.registerKeyObserver` | media-query `change` |

Qt has no reduce-motion reading of its own ([`QStyleHints`](https://doc.qt.io/qt-6/qstylehints.html) carries none), so `linux-qt` answers
`Unsupported`; the forced gate still applies there. Android and ArkUI also route their native
transition skips (list diffs, a navigation push, a cover slide) through the same gate, so a user
who removed animations gets what a scripted run gets.

## Auditing accessibility

The dayscript step `a11y_audit: { id? }` walks Day's id'd nodes, reads each widget's actual
native state ([`Toolkit::read_native`](https://daybrite.dev/api/day_spec/trait.Toolkit.html#method.read_native), whose accessibility group this uses), and diffs identifier
+ label + value + explicit-role against Day's stored expectation. Toolkits that can't read their
native tree (`found=false`) skip. Nodes reported natively hidden (`visible=false`) also skip:
retained navigation panes can omit accessibility metadata while offscreen. Unknown visibility
does not exempt a node. Navigate to a pane before auditing its metadata; use `assert_native`
to check visibility itself. The Apple targets read it from NSAccessibility /
UIAccessibility (→ Day `Role`), the web from the ARIA attributes on the element (`role`,
`aria-label`, `aria-valuetext` or else `aria-description`, `id`), Qt from the widget's
[`QAccessibleInterface`](https://doc.qt.io/qt-6/qaccessibleinterface.html) and `objectName`, Windows from the automation peer and
[`AutomationProperties`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.automation.automationproperties), and Android (API 30+) from the view's `AccessibilityNodeInfo`. GTK and
ArkUI answer `found=false`: GTK 4 has no public getters for an accessible's label and value, and
ArkUI's accessibility text is empty unless Day set it, so neither can be read back faithfully. Role
is diffed only for explicit roles Day applied, since native controls own their roles, which vary
per platform.

## Follow-ups

- `day lint` a11y rule: interactive piece without a derivable label → warning (`--strict` error).
- The accessibility group of `read_native` for GTK and ArkUI, so `a11y_audit` runs there too.
