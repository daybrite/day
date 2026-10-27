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

A combo box lets people type a value or choose a suggestion. `day-pieces` binds the text
and suggestion list to app state. For a choice restricted to listed values, use a
[picker](/docs/reference/picker).

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.
The selected toolkit supplies its native implementation; [`day::combobox::support()`](https://daybrite.dev/api/day_pieces/combobox/fn.support.html) reports
its availability after the Day runtime starts.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, GTK, Qt, Android, WinUI | Native editable choice control. |
| Web | Browser input with a datalist. |
| ArkUI | Composed text input and suggestions menu. |
| UIKit | No renderer; use a [picker](/docs/reference/picker) or [text field](/docs/reference/textfield). |

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

[`combo_box(items, text)`](https://daybrite.dev/api/day_pieces/combobox/fn.combo_box.html) binds `text` two-way: typing updates the signal, picking a
dropdown item writes the item's string, and setting the signal updates the control. `items` is reactive: a change patches the native dropdown live,
and the typed text survives the swap. [`.placeholder(impl IntoText)`](https://daybrite.dev/api/day_pieces/combobox/trait.ComboBoxBuilder.html#tymethod.placeholder) sets the empty-state prompt
(read once at build). `ComboBox` implements `Piece`, so [`.id()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)/[`.a11y()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y)/[`.frame()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) chain via
`Decorate`. Like [`text_field`](https://daybrite.dev/api/day_pieces/fn.text_field.html) it is a **width-growing leaf** (`grow_w = true`, natural
single-line height).

## Toolkit implementations

Platform guidance: [Apple: pickers](https://developer.apple.com/design/human-interface-guidelines/pickers); [Android: spinners](https://developer.android.com/develop/ui/views/components/spinner); [Windows: combo boxes](https://learn.microsoft.com/en-us/windows/apps/design/controls/combo-box).

| Platform / toolkit | Implementation |
| --- | --- |
| AppKit | [`NSComboBox`](https://developer.apple.com/documentation/appkit/nscombobox) |
| GTK | [`GtkComboBoxText`](https://docs.gtk.org/gtk4/class.ComboBoxText.html) with entry |
| Qt | Editable [`QComboBox`](https://doc.qt.io/qt-6/qcombobox.html) |
| Android | [`AutoCompleteTextView`](https://developer.android.com/reference/android/widget/AutoCompleteTextView) |
| WinUI | Editable [`ComboBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.combobox) |
| UIKit | No renderer; placeholder only. |
| Web | Browser input with a datalist. |
| ArkUI | ArkTS `TextInput` + suggestions menu. |

On UIKit, use a [picker](/docs/reference/picker) or
[text field](/docs/reference/textfield). ArkUI composes a text input and suggestions menu.
AppKit completes text inline; Android prefix-filters suggestions while typing.

WinUI commits free-form text on Enter or focus loss; choosing a suggestion updates the binding
immediately. Other implementations report text edits as they occur.

## Limits

The placeholder is read when the control is built. The builder does not provide a separate
submit callback, enabled binding, or maximum-visible-items setting.
