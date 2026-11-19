---
navTitle: "Combo boxes"
title: "Combo box"
description: "An editable text field with a native completion popup, in the core piece library."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Combo box

A combo box offers suggested values while allowing people to enter a value of their own. It suits
fields where common choices save typing but the list cannot cover every valid answer.

`day-pieces` binds the text and suggestion list to app state. For a choice restricted to listed
values, use a [picker](/docs/reference/picker).

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.
The selected toolkit supplies its native implementation; [`day::combobox::support()`](https://docs.rs/day-pieces/latest/day_pieces/combobox/fn.support.html) reports
its availability after the Day runtime starts.

| Platforms / toolkits | Availability |
| --- | --- |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit"), [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk"), [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt"), [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material"), [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | Native editable choice control. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | Browser input with a datalist. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | Composed text input and suggestions menu. |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | No renderer; use a [picker](/docs/reference/picker) or [text field](/docs/reference/textfield). |

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
use day::combobox::combo_box;

// Synthetic suggestions stand in for user-supplied data.
let flavors = Signal::new(vec!["vanilla".into(), "chocolate".into()]);
let flavor = Signal::new(String::new());
combo_box(flavors, flavor)
    .placeholder(res::str::type_or_pick_a_flavor())
    .id(id!(flavor))
```

[`combo_box(items, text)`](https://docs.rs/day-pieces/latest/day_pieces/combobox/fn.combo_box.html) binds `text` two-way: typing updates the signal, picking a
dropdown item writes the item's string, and setting the signal updates the control. `items` is reactive: a change patches the native dropdown live,
and the typed text survives the swap. [`.placeholder(impl IntoText)`](https://docs.rs/day-pieces/latest/day_pieces/combobox/trait.ComboBoxBuilder.html#tymethod.placeholder) sets the empty-state prompt
(read once at build). `ComboBox` implements `Piece`, so [`.id()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id)/[`.a11y()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.a11y)/[`.frame()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.frame) chain via
`Decorate`. Like [`text_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_field.html) it is a **width-growing leaf** (`grow_w = true`, natural
single-line height).

## Toolkit implementations

Platform guidance: [Apple: pickers](https://developer.apple.com/design/human-interface-guidelines/pickers); [Android: spinners](https://developer.android.com/develop/ui/views/components/spinner); [Windows: combo boxes](https://learn.microsoft.com/en-us/windows/apps/design/controls/combo-box).

| Platform / toolkit | Implementation |
| --- | --- |
| [AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") | [`NSComboBox`](https://developer.apple.com/documentation/appkit/nscombobox) |
| [GTK](https://daybrite.dev/docs/reference/platforms/linux-gtk "toolkit:linux-gtk") | [`GtkComboBoxText`](https://docs.gtk.org/gtk4/class.ComboBoxText.html) with entry |
| [Qt](https://daybrite.dev/docs/reference/platforms/linux-qt "toolkit:linux-qt") | Editable [`QComboBox`](https://doc.qt.io/qt-6/qcombobox.html) |
| [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") | [`AutoCompleteTextView`](https://developer.android.com/reference/android/widget/AutoCompleteTextView) |
| [WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") | Editable [`ComboBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.combobox) |
| [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit") | No renderer; placeholder only. |
| [Web](https://daybrite.dev/docs/reference/platforms/web-dom "toolkit:web-dom") | Browser input with a datalist. |
| [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") | ArkTS `TextInput` + suggestions menu. |

On [UIKit](https://daybrite.dev/docs/reference/platforms/ios-uikit "toolkit:ios-uikit"), use a [picker](/docs/reference/picker) or
[text field](/docs/reference/textfield). [ArkUI](https://daybrite.dev/docs/reference/platforms/harmony-arkui "toolkit:harmony-arkui") composes a text input and suggestions menu.
[AppKit](https://daybrite.dev/docs/reference/platforms/macos-appkit "toolkit:macos-appkit") completes text inline; [Android](https://daybrite.dev/docs/reference/platforms/android-material "toolkit:android-material") prefix-filters suggestions while typing.

[WinUI](https://daybrite.dev/docs/reference/platforms/windows-winui "toolkit:windows-winui") commits free-form text on Enter or focus loss; choosing a suggestion updates the binding
immediately. Other implementations report text edits as they occur.

## Limits

The placeholder is read when the control is built. The builder does not provide a separate
submit callback, enabled binding, or maximum-visible-items setting.
