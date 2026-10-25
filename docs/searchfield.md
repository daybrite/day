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
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

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

The signal is a controlled input: a per-build **echo guard** remembers the last value that
arrived from the native control so [`bind_seeded`](https://daybrite.dev/api/day_reactive/fn.bind_seeded.html) does not patch that same value straight back (which
some toolkits would re-emit as a change, causing a feedback loop). The programmatic-sync side is additionally
guarded per toolkit (see the table).

## Per-toolkit native realization

Platform guidance: [Apple: search fields](https://developer.apple.com/design/human-interface-guidelines/search-fields); [Android: search interface](https://developer.android.com/develop/ui/views/search/search-dialog).

| AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI |
|---|---|---|---|---|---|---|
| [`NSSearchField`](https://developer.apple.com/documentation/appkit/nssearchfield) | [`UISearchTextField`](https://developer.apple.com/documentation/uikit/uisearchtextfield) (iOS 13+) | [`GtkSearchEntry`](https://docs.gtk.org/gtk4/class.SearchEntry.html) | [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html) search shim (clear button + leading magnifier) | [`EditText`](https://developer.android.com/reference/android/widget/EditText) (single-line, `IME_ACTION_SEARCH`) | [`AutoSuggestBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.autosuggestbox) (query magnifier) | ArkTS `Search` (the toolkit's `controls/searchfield/platform/harmony/ets`; the C node API has no search kind) |

Each control reports edits through `Event::TextChanged(String)`, the same event a built-in text
field emits, so `dayscript`'s `input:` step drives the piece on every toolkit without touching native
code. The change plumbing per toolkit:

- **AppKit**: a per-node delegate implements [`NSControlTextEditingDelegate::controlTextDidChange:`](https://developer.apple.com/documentation/appkit/nscontroltexteditingdelegate).
  Programmatic `setStringValue` does not fire that delegate, so no suppression is needed (update only
  writes when the value actually differs).
- **UIKit**: a per-node target on [`UIControlEvents::EditingChanged`](https://developer.apple.com/documentation/uikit/uicontrol/event/editingchanged). Programmatic `setText` does not
  fire `EditingChanged`, so no suppression is needed.
- **GTK**: [`GtkSearchEntry::"search-changed"`](https://docs.gtk.org/gtk4/class.SearchEntry.html). That signal *does* fire on programmatic `set_text`, so a
  per-node `suppress` cell guards the sync in `update`.
- **Qt / WinUI**: the native C++ shims live in `day-toolkit-qt-sys` and `day-toolkit-xaml-sys`,
  compiled by those toolkits. The Qt shim wraps a [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html)
  (`setClearButtonEnabled(true)` + a leading `edit-find` action) and wraps programmatic `setText` in
  `blockSignals`. The WinUI shim boxes its [`AutoSuggestBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.autosuggestbox) into a Day handle through the
  `day_xaml_box` / `day_xaml_unbox` functions that `day-toolkit-xaml-sys` exports, the same mechanism the
  picker/media WinUI shims use, so a piece never touches day-toolkit-winui's private handle wrapper.
- **Android**: carries its own Java factory
  (`controls/searchfield/src/DaySearch.java` in `day-toolkit-material`), folded into the app's Gradle build automatically via `[package.metadata.day.android]` (see
  [docs/extending](extending.md)). A [`TextWatcher`](https://developer.android.com/reference/android/text/TextWatcher) calls `DayBridge.nativeOnEvent(id, 1, …)`
  (kind 1 = `TextChanged`); the programmatic setter guards on equality (a plain [`EditText`](https://developer.android.com/reference/android/widget/EditText), so no Gradle
  dependency or manifest permission).

## Follow-ups

- Reactive placeholder (currently fixed at build).
- A submit rail (`Event::Submitted` on the search-action key / return) for "search on enter" semantics.
- Disabled/enabled state; scoped-search tokens (macOS [`NSSearchField`](https://developer.apple.com/documentation/appkit/nssearchfield) recent-searches menu).

WinUI builds on Windows with the Windows SDK and Windows App SDK; it cannot be built on
macOS or Linux.
