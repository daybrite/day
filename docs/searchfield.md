# Search field (external piece)

> **Status: implemented** as `day-piece-searchfield` — an EXTERNAL Day Piece (like `day-piece-picker`),
> registered link-time into each backend's renderer slice with **zero edits** to day. One API: a native
> search input, bound two-way to a `Signal<String>`, realized as each toolkit's dedicated search control.

## Authoring

```rust
use day_piece_searchfield::search_field;

let query = Signal::new(String::new());
search_field(query).placeholder("Search fruit…").id("search")
```

`search_field(query)` takes a `Signal<String>` bound **two-way**: every native edit writes the text back
to the signal, and setting the signal (e.g. a Clear button doing `query.set(String::new())`) patches the
control. `.placeholder(impl IntoText)` sets the empty-state prompt (constant, `Signal<String>`, or
closure — read once for the initial value; the placeholder is fixed at build). `SearchField` implements
`Piece`, so `.id()`/`.a11y()`/`.frame()` chain via `Decorate`. Like day-core's `text_field` it is a
**width-growing leaf** (`grow_w = true`, natural single-line height): a search field fills its row —
constrain it with `.frame(w, h)` if you need a fixed width.

The signal is a controlled input (§4.4): a per-build **echo guard** remembers the last value that
arrived FROM the native control so `bind_seeded` does not patch that same value straight back (which
some toolkits would re-emit as a change → a feedback loop). The programmatic-sync side is additionally
guarded per backend (see the table).

## Per-backend native realization

| AppKit | UIKit | GTK | Qt | Android | WinUI |
|---|---|---|---|---|---|
| `NSSearchField` | `UISearchTextField` (iOS 13+) | `GtkSearchEntry` | `QLineEdit` search shim (clear button + leading magnifier) | `EditText` (single-line, `IME_ACTION_SEARCH`) | `AutoSuggestBox` (query magnifier) |

Each control reports edits through **`Event::TextChanged(String)`** — the same event a built-in text
field emits, so `dayscript`'s `input:` step drives the piece on every backend without touching native
code. The change plumbing per backend:

- **AppKit** — a per-node delegate implements `NSControlTextEditingDelegate::controlTextDidChange:`.
  Programmatic `setStringValue` does not fire that delegate, so no suppression is needed (update only
  writes when the value actually differs).
- **UIKit** — a per-node target on `UIControlEvents::EditingChanged`. Programmatic `setText` does not
  fire `EditingChanged`, so no suppression is needed.
- **GTK** — `GtkSearchEntry::"search-changed"`. That signal *does* fire on programmatic `set_text`, so a
  per-node `suppress` cell guards the sync in `update`.
- **Qt / WinUI** — each carries its **OWN C++ shim** inside this crate (`src/lib-qt-shim.cpp`,
  `src/lib-winui-shim.cpp`), compiled by the crate's `build.rs`. The Qt shim wraps a `QLineEdit`
  (`setClearButtonEnabled(true)` + a leading `edit-find` action) and wraps programmatic `setText` in
  `blockSignals`. The WinUI shim boxes its `AutoSuggestBox` into a Day handle through the
  **`day_winui_box` / `day_winui_unbox` seam** that `day-winui-sys` exports — the same mechanism the
  picker/media WinUI shims use, so a piece never touches day-winui's private handle wrapper.
- **Android** — carries its OWN Java factory
  (`android/java/dev/daybrite/day/piece/searchfield/DaySearch.java`), folded into the app's Gradle build
  automatically via `[package.metadata.day.android]` — **zero edits to day-android** (see
  [docs/extending.md](extending.md)). A `TextWatcher` calls `DayBridge.nativeOnEvent(id, 1, …)`
  (kind 1 = `TextChanged`); the programmatic setter guards on equality (a plain `EditText`, so no Gradle
  dependency or manifest permission).

## Verification

The showcase **Search** page (`search_page`) binds a `search_field` to a `query` signal that filters a
small fruit list (Apple, Banana, Cherry, Date, Elderberry) case-insensitively — each match is a
`when`-gated label in a column, and a `#search-result` label shows the first match. A **Clear** button
sets the signal to `""` to prove the reverse binding patches the native field. The walkthrough navigates
to `search`, types `"ch"` into `#search-input`, asserts `#search-result` reads `Cherry` (the two-way
binding round-tripping the signal), taps `#search-clear`, and screenshots. Rust is clippy-clean
(`-D warnings`) and `cargo fmt`-clean for every backend feature; host-verified for AppKit/GTK/Qt/mock,
cross-compiled for iOS-sim (`uikit`) and Android (`widget`).

## Follow-ups

- Reactive placeholder (currently fixed at build).
- A submit rail (`Event::Submitted` on the search-action key / return) for "search on enter" semantics.
- Disabled/enabled state; scoped-search tokens (macOS `NSSearchField` recent-searches menu).

WinUI is CI-only (built under `cfg(windows)` + the Windows SDK); it isn't buildable on the macOS/Linux
dev hosts.
