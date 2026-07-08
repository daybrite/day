---
title: "Tutorial: A native piece (per-toolkit)"
description: "Build a piece backed by a native control on each platform: a front-end in Rust plus per-toolkit backends (objc2 for AppKit/UIKit, gtk-rs, a Qt C++ shim, an Android Java factory, WinUI), registered without touching any core Day crate."
order: 32
---

Some Pieces cannot be composed from `label`, `button`, and `stack`. A map, an embedded web view, or
a native search field with its magnifier and clear button is a platform control with input
handling, assistive-technology behavior, and pixels that only the toolkit can produce. To ship
one as a Day Piece you write a small cross-platform front-end in Rust, then a native backend for
each toolkit you want to support.

This is the highest-effort tier of extension in Day. You are going to write the same widget five or
six times: once in Objective-C (through `objc2`), once in gtk-rs, once as a Qt C++ shim, once as an
Android Java factory, and once as a WinUI C++/WinRT shim. That is more work than a composite
piece. The payoff is a control that works like a built-in `text_field` on every platform, is
installable by any app with a single line in `Cargo.toml`, and requires no changes to any core Day
crate.

We will build one throughout: a **native search field** bound two-way to a `Signal<String>`. It
already exists in the tree as [`day-piece-searchfield`](https://github.com/daybrite/day/tree/main/pieces/day-piece-searchfield),
so you can read the finished crate alongside this tutorial. Every snippet below is lifted from it.

> If your control can instead be *assembled* from existing Pieces (an HStack of a label and a
> stepper, a card built from a `column` and a `divider`), you want the much lighter
> [composite-piece tutorial](/docs/tutorial-composite-piece) instead. Reach for a native piece only when
> there is an actual platform control underneath.

## 1. When you need a native piece

Ask one question: am I wrapping a native control, or am I arranging Pieces?

| You are… | Tier | What you write |
|---|---|---|
| Arranging existing Pieces into a reusable unit | **Composite piece** | A `Piece` whose `build` returns `column((...))`; no backend, no `KIND`. |
| Wrapping one native widget per platform | **Native piece** (this tutorial) | A front-end plus a backend per toolkit. |
| Adding a headless capability (battery, clipboard) | **Part** | A `day-part-*` crate, no `RENDERERS`, selected by `#[cfg(target_os)]`. |

A native piece is warranted when the thing you want does not exist as a composition: it has native
text input, native scrolling physics, a system popover, camera/map/media surfaces, or platform
accessibility semantics you cannot fake. The search field qualifies: `NSSearchField`,
`UISearchTextField`, `GtkSearchEntry`, and the rest each bring a magnifier, a clear button, and IME
behavior that a hand-rolled `text_field` would not.

To set expectations: you will implement `make`/`update`/`measure` once per toolkit, and each
one speaks that toolkit's native API. The rest of this tutorial is about making that as mechanical as
possible, and [step 6](#6-the-llm-workflow) is about handing most of the typing to an LLM.

## 2. The architecture

Start with the scaffolder. `day new piece --toolkits <list>` generates every file described below:
the front-end `src/lib.rs` (builder + `KIND` + `Props`/`Patch` + `#[cfg]`/`#[path]` backend index),
one `src/lib-<backend>.rs` renderer per toolkit, and the `[features]` table. Where a backend needs
native glue, it also emits the C++ shim + `build.rs` (Qt/WinUI), the Java shim +
`[package.metadata.day.android]` (Android/`widget`), and the `[package.metadata.day.ios]` block
(iOS/`uikit`):

```bash
day new piece day-piece-searchfield --toolkits appkit,gtk,qt,uikit,widget,winui
```

Pick any subset of `appkit,gtk,qt,uikit,widget,winui`; passing `--toolkits` at all makes it a
native piece rather than a composite one. The crate builds against a remote Day release out of
the box (add `--local <path>` for a local Day checkout). Everything from here down describes what the
scaffolder produces and how the halves fit together.

One crate carries both halves of the piece:

- **The front-end**: cross-platform Rust. A builder function, a config struct, and an
  `impl Piece` whose `build` emits a *native leaf* of a well-known `KIND` string and wires up
  reactivity. This is the only code an app author ever calls.
- **A backend per toolkit**: one file each (`lib-appkit.rs`, `lib-gtk.rs`, `lib-qt.rs`,
  `lib-uikit.rs`, `lib-android.rs`, `lib-winui.rs`), compiled only for its feature+target, that
  turns the `KIND` leaf into a native widget.

The two halves never call each other directly. They communicate through a tiny typed protocol:

- **`KIND`**: a stable string (`"day.piece.searchfield"`) naming this piece to every backend.
- **`Props`**: the full "realize" payload, handed to a backend's `make` when the widget is first
  created (initial text, placeholder, …).
- **`Patch`**: a *sparse* enum of imperative changes, handed to `update` when a bound value later
  changes (here just `SetText(String)`).

Backends register themselves into each toolkit's `RENDERERS` slice at link time, via `linkme`.
When a backend's `AppKit::new()` starts up it walks `RENDERERS` and builds a registry:

```rust
// toolkits/day-appkit/src/lib.rs
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<AppKit>];

let mut registry = Registry::default();
for f in RENDERERS { registry.register(f()); }   // your piece is now known to AppKit
```

Because registration is link-time, adding a piece requires no edit to any Day crate: no central
enum, no match arm, no registry table. Just linking your crate into the AppKit build inserts its
renderer.

The glue that makes the backend bodies pleasant is the `renderer!` macro. `Props` and `Patch` cross
the backend boundary type-erased as `&dyn Any`; the macro inserts the downcast for you, so your
`make`/`update` see fully typed `&SearchProps` / `&SearchPatch`:

```rust
// crates/day-pieces/src/render.rs (what the macro expands to, abridged)
make: |backend, props, id| {
    let p = props.downcast_ref::<SearchProps>()
        .expect("day renderer: props are not SearchProps");
    make_fn(backend, p, id)            // ← your typed fn
},
update: |backend, handle, patch| {
    if let Some(p) = patch.downcast_ref::<SearchPatch>() {
        update_fn(backend, handle, p)  // ← your typed fn
    }
},
```

You never write a downcast. You write `fn make(backend: &mut AppKit, p: &SearchProps, id: NodeId)`
and one `renderer!` line.

## 3. The front-end

The front-end is the only part every backend shares, and the only part an app author touches. It has
four moving parts: the config struct + builder, the `Props`/`Patch` types, and the `impl Piece`.

### The `KIND`, `Props`, and `Patch`

```rust
// pieces/day-piece-searchfield/src/lib.rs
pub const KIND: &str = "day.piece.searchfield";

/// Full props (realize). `text` seeds the control; `placeholder` is the empty-state prompt.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SearchProps {
    pub text: String,
    pub placeholder: String,
}

/// The single imperative update: replace the control's text (programmatic sync from the signal).
#[derive(Clone, Debug, PartialEq)]
pub enum SearchPatch {
    SetText(String),
}
```

`Props` is everything a backend needs to build the widget from scratch. `Patch` is the *sparse* set
of changes that can happen afterward. Keep it minimal: each variant is one thing a backend must know
how to apply. A search field only ever needs its text re-synced, so `Patch` has exactly one variant.

### The builder and config struct

```rust
pub struct SearchField {
    query: Signal<String>,
    placeholder: Option<TextSource>,
}

/// `search_field(query)`: a native search input whose text mirrors `query` in both directions.
pub fn search_field(query: Signal<String>) -> SearchField {
    SearchField { query, placeholder: None }
}

impl SearchField {
    /// The empty-state prompt (evaluated once; the placeholder is not reactive after build).
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }
}
```

This is the ordinary Day builder idiom: a free function returns a config struct, chained `.method()`
calls fill it in. Because `SearchField` will `impl Piece`, it also gets `.id()`, `.a11y()`, and
`.frame()` for free from the blanket `Decorate` impl. You do not write those.

### `impl Piece`: emit the leaf, bind reactivity, handle events

`build` is where the front-end meets the protocol. Three things happen: emit the native leaf, bind
the signal *into* the control, and route native events *out of* it.

```rust
impl Piece for SearchField {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let SearchField { query, placeholder } = self;
        let initial = query.get_untracked();
        let ph = placeholder.map(|p| p.initial()).unwrap_or_default();

        // (1) Emit a native leaf of KIND. `cx.leaf` hands `&SearchProps` to the backend's `make`.
        let node = cx.leaf(
            KIND,
            &SearchProps { text: initial.clone(), placeholder: ph },
            // A search field fills the available width and keeps its natural (single-line) height.
            Flex { grow_w: true, ..Default::default() },
        );

        // (2) Bind the signal INTO the control: when `query` changes, patch the widget's text.
        //     `bind_seeded` runs the setter once now, then on every change to a read signal.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let g = guard.clone();
        bind_seeded(
            initial,
            move || query.get(),
            move |t: &String| {
                // Echo guard (see below): skip the value that just came from the native widget.
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                if !from_native {
                    with_tree(|tr| tr.patch(node, Box::new(SearchPatch::SetText(t.clone())), false));
                }
            },
        );

        // (3) Route native edits OUT of the control back into the signal.
        cx.on(node, move |ev| {
            if let Event::TextChanged(t) = ev {
                *guard.borrow_mut() = Some(t.clone());   // remember: this value came from native
                query.set(t.clone());
            }
        });
        node
    }
}
```

Three details are load-bearing:

- **`cx.leaf(KIND, &props, flex)`** creates a native-leaf node of `KIND`. `Flex { grow_w: true, .. }`
  makes it a width-growing leaf (it fills its row, keeps its natural single-line height), the same
  shape as the built-in `text_field`.
- **`bind_seeded(seed, read, apply)`** is the reactive setter. It applies `apply` once with the seed,
  then re-runs it whenever the read closure's signal changes, emitting a sparse `Patch`. Only this
  binding re-runs; there is no view re-execution.
- **The echo guard** is the one subtlety of any *two-way* (controlled) input. The user types → the
  native widget fires `Event::TextChanged` → `cx.on` writes the signal → `bind_seeded` sees the signal
  change and would immediately patch the control with the value it *just produced*, which some
  toolkits re-emit as another change → a feedback loop. The `guard` cell remembers the last value that
  arrived *from* native so `bind_seeded` skips patching that one value straight back. (One-way pieces
  do not need this.)

That is the entire front-end. Everything else is native.

## 4. The backends, one per toolkit

Each backend lives in its own file and is wired into `lib.rs` with a `#[cfg]/#[path]` module index,
so a file is compiled only for its feature+target and the whole native surface for a toolkit sits in
one place:

```rust
// pieces/day-piece-searchfield/src/lib.rs
#[cfg(all(feature = "appkit", target_os = "macos"))] #[path = "lib-appkit.rs"] mod appkit_impl;
#[cfg(feature = "gtk")]                               #[path = "lib-gtk.rs"]    mod gtk_impl;
#[cfg(feature = "qt")]                                #[path = "lib-qt.rs"]     mod qt_impl;
#[cfg(all(feature = "uikit", target_os = "ios"))]     #[path = "lib-uikit.rs"]  mod uikit_impl;
#[cfg(all(feature = "widget", target_os = "android"))]#[path = "lib-android.rs"]mod android_impl;
#[cfg(all(feature = "winui", windows))]               #[path = "lib-winui.rs"]  mod winui_impl;
```

Every backend implements the same three functions and ends with one `renderer!` line:

- **`make(backend, &Props, NodeId) -> Handle`**: build the native widget, wire its change callback to
  `day_<backend>::emit(node, Event::TextChanged(..))`, return the toolkit's handle type.
- **`update(backend, &Handle, &Patch)`**: apply a sparse patch (here, set the text) *guarded on
  equality* so a programmatic sync is a no-op when unchanged.
- **`measure(backend, &Handle, Proposal) -> Size`**: report the widget's size given a proposal. All
  five of our backends grow to the proposed width and keep their natural height.

### AppKit: `NSSearchField` via objc2 (the fully worked backend)

The Apple backends use the `objc2` crates: real Objective-C objects, driven from Rust. An
`NSSearchField` *is* an `NSTextField`, so a per-node delegate implementing
`controlTextDidChange:` delivers each keystroke. Programmatic `setStringValue:` does not fire that
delegate, so this backend needs no echo guard of its own; `update` simply writes when the value
differs.

```rust
// pieces/day-piece-searchfield/src/lib-appkit.rs (abridged)
use super::*;
use day_appkit::AppKit;
use day_spec::{NodeId, Proposal, Size};
use objc2_app_kit::{NSSearchField, NSTextField, NSView, NSTextFieldDelegate, NSControlTextEditingDelegate};

// A DefinedClass delegate carrying the NodeId; controlTextDidChange: → Event::TextChanged.
// (define_class! + an ivar holding `node`, omitted for length; see the file.)

fn make(backend: &mut AppKit, p: &SearchProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    let field = NSSearchField::new(mtm);
    if !p.placeholder.is_empty() {
        field.setPlaceholderString(Some(&NSString::from_str(&p.placeholder)));
    }
    field.setStringValue(&NSString::from_str(&p.text));
    let target = SearchTarget::new(mtm, id);     // per-node delegate, kept alive in a TLS map
    let tf: &NSTextField = field.as_ref();
    unsafe { tf.setDelegate(Some(ProtocolObject::from_ref(&*target))) };
    Retained::from(<NSSearchField as AsRef<NSView>>::as_ref(&field))
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &SearchPatch) {
    let SearchPatch::SetText(t) = patch;
    if let Some(field) = h.downcast_ref::<NSSearchField>() {
        if field.stringValue().to_string() != *t {         // equality-guarded
            field.setStringValue(&NSString::from_str(t));
        }
    }
}

fn measure(_backend: &mut AppKit, h: &Retained<NSView>, p: Proposal) -> Size {
    let fit = h.fittingSize();
    Size::new(p.width.unwrap_or(fit.width).max(120.0), fit.height.ceil().max(22.0))
}

// The one line that registers this backend link-time. The macro inserts the downcast.
day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: SearchProps, patch: SearchPatch,
    make: make, update: update, measure: measure);
```

The other backends are the *same three functions* against a different native API. Here is the shape
of each.

### UIKit: `UISearchTextField` via objc2

A separate file from AppKit (different `objc2-ui-kit` types), but the same pattern: build a
`UISearchTextField`, then attach a per-node target on `UIControlEvents::EditingChanged` that emits
`Event::TextChanged`. Since programmatic `setText:` does not fire `EditingChanged`, there is no echo
guard. `measure` uses `sizeThatFits:`. The handle type is `Retained<UIView>`, and the `renderer!`
targets `day_uikit::RENDERERS, Uikit`.

### GTK: `GtkSearchEntry` via gtk4-rs

Pure Rust through `gtk4`. `GtkSearchEntry::new()` gives the magnifier + clear icon. One wrinkle: its
`search-changed` signal fires on both user input and programmatic `set_text`, so this backend
carries a per-node `suppress: Rc<Cell<bool>>`. `update` raises it around `set_text` so the resulting
signal does not echo back as an `Event::TextChanged`:

```rust
// pieces/day-piece-searchfield/src/lib-gtk.rs (the guard)
entry.connect_search_changed(move |e| {
    if sup.get() { return; }                                  // suppressed = programmatic
    day_gtk::emit(id, Event::TextChanged(e.text().to_string()));
});
// …in update:
st.suppress.set(true);  st.entry.set_text(t);  st.suppress.set(false);
```

### Qt: a hand-written C++ shim behind a flat `extern "C"` ABI

Qt has no Rust bindings in Day, so the piece carries its own C++ shim, `src/lib-qt-shim.cpp`, and
`build.rs` compiles it. The shim wraps a `QLineEdit` dressed as a search box (clear button + a leading
magnifier action) and exposes a flat C ABI: `day_search_new` takes a callback pointer and a `u64`
node id; `textChanged` calls back with a UTF-8 string; and programmatic `setText` is wrapped in
`blockSignals` so it never echoes:

```cpp
// pieces/day-piece-searchfield/src/lib-qt-shim.cpp (abridged)
class DaySearch : public QLineEdit {
public:
    void setTextGuarded(const QString &t) {
        if (text() != t) { blockSignals(true); setText(t); blockSignals(false); }
    }
};
extern "C" void *day_search_new(const char *placeholder, const char *initial, uint64_t id,
                                void (*cb)(uint64_t, const char *)) {
    auto *w = new DaySearch();
    w->setPlaceholderText(QString::fromUtf8(placeholder));
    w->setClearButtonEnabled(true);
    w->addAction(QIcon::fromTheme(QStringLiteral("edit-find")), QLineEdit::LeadingPosition);
    if (initial && *initial) w->setText(QString::fromUtf8(initial));
    QObject::connect(w, &QLineEdit::textChanged,
                     [id, cb](const QString &t) { cb(id, t.toUtf8().constData()); });
    return w;
}
```

The Rust side (`lib-qt.rs`) declares those `extern "C"` functions, calls them from `make`/`update`,
and reuses `day_qt_size_hint` (already linked by `day-qt-sys`) for `measure`. The shim is compiled by
`build.rs` via `cc` + `pkg-config`, gated on the feature:

```rust
// pieces/day-piece-searchfield/build.rs (Qt branch, abridged)
if std::env::var("CARGO_FEATURE_QT").is_ok() {
    let cflags = Command::new("pkg-config").args(["--cflags", "Qt6Widgets"]).output().unwrap();
    let mut build = cc::Build::new();
    build.cpp(true).std("c++17").file("src/lib-qt-shim.cpp");
    for tok in String::from_utf8_lossy(&cflags.stdout).split_whitespace() { build.flag(tok); }
    build.compile("daysearchqtshim");   // Qt libs themselves are already linked by day-qt-sys
}
```

### Android: a Java factory staged through Gradle, called over JNI

The Android widget is created by a Java factory the piece ships under
`android/java/dev/daybrite/day/piece/searchfield/DaySearch.java`. It uses only `day-android`'s public
Java surface: `DayBridge.ctx` (the `Context`) and `DayBridge.nativeOnEvent(id, kind, num, str)` (the
event trampoline; `kind 1` = TextChanged):

```java
// android/java/…/DaySearch.java (abridged)
public static View makeSearch(final long id, String placeholder, String initial) {
    EditText e = new EditText(DayBridge.ctx);
    e.setSingleLine(true);
    e.setImeOptions(EditorInfo.IME_ACTION_SEARCH);
    e.setHint(placeholder);
    if (initial != null && !initial.isEmpty()) { e.setText(initial); e.setSelection(initial.length()); }
    e.addTextChangedListener(new TextWatcher() {
        public void afterTextChanged(Editable s) { DayBridge.nativeOnEvent(id, 1, 0, s.toString()); }
        public void beforeTextChanged(CharSequence s,int a,int b,int c){}
        public void onTextChanged(CharSequence s,int a,int b,int c){}
    });
    return e;
}
```

`lib-android.rs` calls that class through the re-exported `jni` (`with_env` + `call_static_method`),
returning a global ref as the handle. The factory reaches the app's Gradle build automatically:
the piece declares its Java dir in `Cargo.toml`, and `day build` folds it in with no edit to
`day-android` and no per-piece Gradle edit:

```toml
# pieces/day-piece-searchfield/Cargo.toml
[package.metadata.day.android]
java = ["android/java"]        # → Gradle java.srcDirs
gradle-dependencies = []       # EditText is a framework widget; nothing to pull
gradle-repositories = []
# permissions = ["android.permission.INTERNET"]   # add if your control needs one (e.g. a web view)
```

### WinUI: a C++/WinRT shim, boxed through `day-winui-sys`

Symmetric to Qt: the piece carries `src/lib-winui-shim.cpp`, a C++/WinRT shim wrapping an
`AutoSuggestBox` (the WinUI search control). Because WinUI handles are a private boxed type owned by
`day-winui-sys`, the shim boxes its XAML element through that crate's exported
`day_winui_box`/`day_winui_unbox` seam, and reuses `day_winui_measure`. `build.rs` compiles it with
`cc` (MSVC) + the Windows SDK cppwinrt projection. It is Windows-only and built in CI; you will
likely not verify it locally.

### The pattern, generalized

For any native piece, each backend is the same three functions:

1. **`make`**: construct the native widget from `Props`; wire its change callback to
   `day_<backend>::emit(node, event)`; return the toolkit handle.
2. **`update`**: apply a sparse `Patch`, equality-guarded, suppressing the change-callback if the
   toolkit re-emits programmatic edits.
3. **`measure`**: answer a `Proposal` with a `Size` (grow, natural, or fixed).

Toolkits that are hard to drive from Rust (Qt, WinUI, non-`@objc` iOS classes, anything on the Android
`Context`) get a *native asset*: a C++ shim compiled in `build.rs`, an Android Java factory staged via
`[package.metadata.day.android]`, or an iOS Swift shim + SwiftPM package via
`[package.metadata.day.ios]`. The last is how a piece links a framework it drives. The media piece,
which hand-rolls `AVPlayerViewController`, declares:

```toml
# pieces/day-piece-media/Cargo.toml
[package.metadata.day.ios]
frameworks = ["AVKit", "AVFoundation", "CoreMedia"]   # linked via the generated DayPieces SwiftPM pkg
```

## 5. Register and wire the features

Two pieces of wiring make the whole thing hang together.

**Per-backend `renderer!` line.** Each backend file ends with one macro call registering it into that
toolkit's slice:

```rust
day_pieces::renderer!(day_gtk::RENDERERS,   Gtk,   kind: KIND, props: SearchProps, patch: SearchPatch, make: make, update: update, measure: measure);
day_pieces::renderer!(day_qt::RENDERERS,    Qt,    kind: KIND, props: SearchProps, patch: SearchPatch, make: make, update: update, measure: measure);
day_pieces::renderer!(day_uikit::RENDERERS, Uikit, kind: KIND, props: SearchProps, patch: SearchPatch, make: make, update: update, measure: measure);
// …and day_android::RENDERERS / day_winui::RENDERERS likewise.
```

(For a piece configured once with no later updates, drop `patch:`/`update:`; the macro has a
patchless form. Add `measure: day_pieces::fill_measure` for a growing leaf that fills its proposal.)

**The crate's `[features]`.** One feature per backend, each pulling in that toolkit crate (and any
objc2/gtk4 crates it needs). A no-op `mock` feature exists so apps can enable the piece uniformly on a
toolkit with no real backend:

```toml
# pieces/day-piece-searchfield/Cargo.toml
[features]
appkit = ["dep:day-appkit", "dep:objc2", "dep:objc2-app-kit", "dep:objc2-foundation"]
gtk    = ["dep:day-gtk", "dep:gtk4"]
qt     = ["dep:day-qt"]              # + build.rs compiles src/lib-qt-shim.cpp
uikit  = ["dep:day-uikit", "dep:objc2", "dep:objc2-ui-kit", "dep:objc2-foundation", "dep:objc2-core-foundation"]
widget = ["dep:day-android"]         # + [package.metadata.day.android] carries the DaySearch Java
winui  = ["dep:day-winui", "dep:day-winui-sys"]   # + build.rs compiles src/lib-winui-shim.cpp
mock   = []                          # no renderer; falls back to Day's placeholder leaf
```

**The `backends` marker (Tier A).** Historically an app had to re-list every piece's per-backend
feature in its own `Cargo.toml`. It no longer does. A piece declares the backends it carries a
renderer feature for:

```toml
# pieces/day-piece-searchfield/Cargo.toml
[package.metadata.day.piece]
backends = ["appkit", "gtk", "qt", "uikit", "widget", "winui", "mock"]
```

…and `day build` reads that from `cargo metadata`, walks the app's dependency closure, and derives
`day-piece-searchfield/<backend>` for whichever toolkit it is building, unioning it into the
`--features` automatically (`crates/day-cli/src/pieces.rs::feature_union`).

The result: an app adds the piece with a single plain dependency and wires only Day's own backend,
with no per-piece feature fan-out:

```toml
# apps/showcase/Cargo.toml
[features]
appkit = ["day/appkit"]     # the app wires the core backend; the CLI derives the piece's
gtk    = ["day/gtk"]        # day-piece-searchfield/<backend> from its [package.metadata.day.piece]
# …

[dependencies]
day-piece-searchfield = { workspace = true }   # that's it
```

Then use it like a built-in:

```rust
use day_piece_searchfield::search_field;

let query = Signal::new(String::new());
search_field(query).placeholder("Search fruit…").id("fruit-search")
```

If a backend is not registered (say you enabled the piece on a toolkit you have not written yet),
Day does not fail silently. The backend logs a once-per-kind warning and renders a visible
placeholder in its place (`warn_missing_renderer` in each toolkit crate); in debug builds the §8.2
registration check panics first. A half-finished piece degrades noisily instead of vanishing.

## 6. The LLM workflow

Writing the same widget in Objective-C, C++, Java, and C++/WinRT is the tedious core of a native
piece. It is also the kind of narrow, well-specified translation that LLMs are good at, so lean
into that.

Make the boundary crisp first, then delegate each backend body:

1. **Design the typed protocol in Rust before any native code.** Write `KIND`, the `Props` struct, and
   the sparse `Patch` enum. These are small, and getting them right up front means every backend has
   the same fixed contract to satisfy.
2. **Write the front-end and validate it against the `mock` backend.** The front-end is
   platform-independent; with `day-piece-searchfield/mock` you can exercise `build`, the `bind_seeded`
   binding, the echo guard, and the `cx.on` event routing with no device and no native toolkit at
   all. The typed `renderer!` macro and the mock feature let you get the Rust side completely right
   before you touch Objective-C.
3. **Freeze the flat FFI/JNI signatures.** For the toolkits that need a native asset, decide the C ABI
   (`day_search_new(placeholder, initial, id, cb)` / `day_search_set_text(w, text)`) or the JNI method
   signatures (`makeSearch(long, String, String) -> View` / `setSearchText(View, String)`) up front.
   They are the exact spec you hand the model.
4. **Ask the LLM for each backend body from a one-paragraph description of the native control.** For
   example: *"Write the AppKit backend as an objc2 `make`/`update`/`measure` for a `NSSearchField`
   bound to this `SearchProps`/`SearchPatch`; per-node delegate on `controlTextDidChange:` emitting
   `Event::TextChanged`; no echo guard because programmatic `setStringValue:` doesn't fire the
   delegate."* Then repeat for the Qt C++ shim, the Android Java factory, and the WinUI C++/WinRT shim
   against the frozen signatures. Give the model this crate's finished file as the template; it is a
   ready-made few-shot example.
5. **Wire each behind `renderer!` and enable its feature.** Add the module line to `lib.rs`, the
   `renderer!` call at the bottom of the backend file, and the feature to `Cargo.toml`.

Two properties of the design make this workflow safe. First, the typed macro means a body that
compiles has type-checked `Props`/`Patch` handling; the model cannot silently mismatch
the protocol. Second, an unwritten backend degrades to the placeholder rather than breaking
the app, so you can ship AppKit + Android today and add Qt or WinUI later without either half of the
codebase blocking the other. Write the toolkits you can verify, generate the rest, and let the
registration check tell you which ones are still stubs.

---

You now have the full picture: one crate, a shared Rust front-end over a typed `KIND`/`Props`/`Patch`
protocol, a native backend per toolkit registered link-time with no core changes, and a build that
derives its own features. The reference crate to read end-to-end is
[`pieces/day-piece-searchfield`](https://github.com/daybrite/day/tree/main/pieces/day-piece-searchfield);
[`day-piece-picker`](https://github.com/daybrite/day/tree/main/pieces/day-piece-picker) shows three
native stylings of one piece, and [`day-piece-media`](https://github.com/daybrite/day/tree/main/pieces/day-piece-media)
shows framework linking. The mechanism is documented in full in
[`docs/extending.md`](https://github.com/daybrite/day/blob/main/docs/extending.md).
