---
navTitle: "Search fields"
title: "Search field"
description: "A search field bound to app state, with native input on every supported toolkit."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Search field

Add a search input whose query stays in sync with app state. `day-pieces` uses
the platform’s search control; the app uses the query to filter or fetch its results.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.
The selected toolkit supplies its native implementation; [`day::searchfield::support()`](https://daybrite.dev/api/day_pieces/searchfield/fn.support.html) reports
its availability after the Day runtime starts.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, UIKit, GTK, Qt, Android, WinUI, ArkUI | Toolkit-specific search input. |
| Web | Browser search input. |

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Authoring

```rust
use day::searchfield::search_field;

let query = Signal::new(String::new());
search_field(query)
    .placeholder(res::str::search_fruit())
    .id(id!(search))
```

[`search_field(query)`](https://daybrite.dev/api/day_pieces/searchfield/fn.search_field.html) takes a `Signal<String>` bound two-way: every native edit writes the text back
to the signal, and setting the signal (e.g. a Clear button doing [`query.set(String::new())`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.set)) patches the
control. [`.placeholder(impl IntoText)`](https://daybrite.dev/api/day_pieces/searchfield/trait.SearchFieldBuilder.html#tymethod.placeholder) sets the empty-state prompt; it accepts a constant,
`Signal<String>`, or closure, read once for the initial value (the placeholder is fixed at build).
`SearchField` implements `Piece`, so [`.id()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id)/[`.a11y()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y)/[`.frame()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) chain via `Decorate`. Like day-core's
[`text_field`](https://daybrite.dev/api/day_pieces/fn.text_field.html) it is a **width-growing leaf** (`grow_w = true`, natural single-line height): a search
field fills its row, so constrain it with [`.frame(w, h)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.frame) if you need a fixed width.

## Toolkit implementations

Platform guidance: [Apple: search fields](https://developer.apple.com/design/human-interface-guidelines/search-fields); [Android: search interface](https://developer.android.com/develop/ui/views/search/search-dialog).

| AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI |
|---|---|---|---|---|---|---|
| [`NSSearchField`](https://developer.apple.com/documentation/appkit/nssearchfield) | [`UISearchTextField`](https://developer.apple.com/documentation/uikit/uisearchtextfield) (iOS 13+) | [`GtkSearchEntry`](https://docs.gtk.org/gtk4/class.SearchEntry.html) | [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html) search shim (clear button + leading magnifier) | [`EditText`](https://developer.android.com/reference/android/widget/EditText) (single-line, `IME_ACTION_SEARCH`) | [`AutoSuggestBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.autosuggestbox) (query magnifier) | ArkTS `Search` (the toolkit's `controls/searchfield/platform/harmony/ets`; the C node API has no search kind) |

## Limits

The placeholder is read when the control is built. The builder does not provide an enabled
binding, a submit callback, or scoped-search tokens. Use a [text field](/docs/reference/textfield)
for an explicit submit action, or [page search](/docs/reference/search) to integrate search
with navigation and toolbar placement.
