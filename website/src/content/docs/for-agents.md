---
title: For AI Agents
description: A terse, rule-based reference for LLMs and coding agents writing Day apps. Follow the invariants verbatim; prefer the patterns shown.
order: 5
---

This page is written for coding agents rather than humans, so it is terse and imperative. Prefer
the patterns here verbatim, obey the invariants, and cross-check against the failure modes before you
finish. A machine-readable index of the whole site lives at [`/llms.txt`](/llms.txt).

## Naming (disambiguate before writing)

- **Day**: the framework (proper noun; always capitalized in prose).
- `day`: the CLI binary. You type `day build`, `day launch`, etc. Always lowercase.
- `day`: the Rust crate. `use day::prelude::*;` brings in the whole API. Always lowercase.
- `day.yaml`: the project manifest. **Piece**: a UI node (SwiftUI View / Flutter Widget). **Signal**:
  a reactive state cell. **target**: an `(OS, toolkit)` pair, e.g. `macos-appkit`, `ios-uikit`.

## What Day is (facts)

Day builds cross-platform desktop + mobile apps in Rust. You write one declarative UI as a tree of
**Pieces**; each Piece is realized by a native widget (`NSTextField`, `UILabel`, `GtkEntry`,
`QSlider`, WinUI `TextBox`, `android.widget.*`) through a per-platform toolkit backend. Day owns layout,
reactivity, localization, accessibility policy, and scripting; the OS owns pixels, text input, scrolling,
and assistive tech. There is no virtual DOM and no diffing: the native tree is built once and Signals
bind straight to native attributes.

## Invariants (MUST; violating these is a bug)

1. **One toolkit backend per binary.** A binary compiles exactly one backend (selected by a Cargo
   feature via the target). Never enable two. The build enforces this with a `compile_error!`.
2. **Build once, bind forever.** Never rebuild the view on state change. To make UI reactive, pass a
   closure that reads a Signal (`label(move || …count.get()…)`), or pass a Signal to a control
   (`slider(volume)`). Do not diff, re-run, or recreate Piece trees yourself.
3. **`Signal<T>` is `Copy`.** Clone/move it into as many closures as you need; do not wrap it in `Rc`.
4. **Give every interactive/asserted Piece a stable `.id("…")`.** Tests, dayscript, and deep links
   address Pieces by id. No id ⇒ not scriptable.
5. **Localize user-facing text with `tr("key")`** and Fluent files; don't hard-code display strings in
   shipped apps (the showcase uses literals only for its own demo labels).
6. **Edit `day.yaml` + Rust; never hand-edit the generated Xcode/Gradle scaffolds.** `day` regenerates
   them.
7. **Verify on a real target.** `cargo build` does not prove a target works. Use `day launch -p <target>`
   and, for assertions, `day launch -p <target> --script <dayscript.yaml>`.

## Setup (canonical)

```bash
day new app my-app --toolkit macos-appkit,ios-uikit,android-widget
cd my-app
day launch -p macos-appkit                 # build + run
day launch -p macos-appkit --script scripts/walkthrough.yaml   # build + run + assert
```

`day.yaml`:

```yaml
day: 1
app: { name: my-app, id: dev.example.my-app, title: My App, version: 0.1.0 }
targets: [macos-appkit, ios-uikit, android-widget]
window: { width: 480, height: 640 }
```

## Core model (precise)

- A **Piece** is a value produced by a function call (`label(...)`, `button(...)`, `column((...))`).
  Containers take a **tuple** of children. Builder methods (`.padding`, `.spacing`, `.id`, `.font`, …)
  return the Piece. End a heterogeneous Piece with `.any()` to get `AnyPiece`.
- `Signal<T>`: `get()` (tracked read), `set(v)`, `update(|v| …)`, `with(|v| …)` (borrow),
  `get_untracked()`. Reading a Signal inside a binding closure makes that binding re-run when the
  Signal changes; nothing else re-runs.
- **Reactivity rule:** static content → pass a value; dynamic content → pass a closure. `label("Hi")`
  is static; `label(move || format!("{}", n.get()))` is reactive.
- A **target** is `(OS, toolkit)`: `macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit`,
  `android-widget`, `linux-gtk`, `linux-qt`, `windows-winui`, `windows-gtk`, `windows-qt`.

## Canonical patterns (copy these)

**App skeleton**

```rust
use day::prelude::*;

fn main() {
    day::launch(
        WindowOptions { title: "My App".into(), size: Size::new(480.0, 640.0), min_size: None },
        root,
    );
}

fn root() -> AnyPiece {
    let count = Signal::new(0i64);
    column((
        label(move || format!("{} clicks", count.get())).font(Font::Title).id("counter"),
        row((
            button("−").action(move || count.update(|c| *c -= 1)).id("dec"),
            button("+").action(move || count.update(|c| *c += 1)).id("inc"),
        ))
        .spacing(8.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
```

**Inputs (two-way; edits flow back into the Signal)**

```rust
let name = Signal::new(String::new());
let volume = Signal::new(40.0);
let on = Signal::new(false);
column((
    text_field(name).placeholder("Your name").id("name"),
    slider(volume).range(0.0..=100.0).step(1.0).id("vol"),
    toggle(on).id("on"),
    progress(move || volume.get() / 100.0),   // tracks the slider live
))
```

**Conditionals + keyed collections**

```rust
when(move || !name.with(|s| s.is_empty()),
     move || label(move || format!("Hi, {}", name.get())))

// `each` builds one child per item and reconciles by key (each row keeps its own state):
each(move || items.get(), |it| it.id.clone(), |it| label(it.title).id_keyed("row", it.id))
```

**Navigation (a projection of an app-owned Signal; you own the state)**

```rust
// one-of-N (Sidebar → split view; Tabs → native tabs):
let section = Signal::new(String::new());
selector(section)
    .style(SelectorStyle::Sidebar)
    .title("My App")
    .item("home", "Home", home_page)
    .item("settings", "Settings", settings_page)
    .id("nav")

// push/pop stack bound to a path Signal:
let path = Signal::new(Vec::<String>::new());
stack(path, home_view).destination(|key| detail_view(key));
// push: path.update(|p| p.push("item-42".into()));  pop is written back by the native back button.

navigate("settings");  nav_back();  current_route();   // string-route adapter (also deep links + dayscript)
```

**Text, fonts, color, accessibility**

```rust
label("Chapter").font(Font::Title).bold()               // semantic style + weight
label("caption").font(Font::Footnote).italic()
label("18pt").font(Font::System(18.0))                  // custom size, still accessibility-scaled
label(tr("greeting").arg("name", name))                 // localized + interpolated Signal
progress(move || v.get() / 100.0).a11y(|a| a.role(Role::Meter).label("Volume"))
```

Semantic `Font` styles (largest→smallest): `LargeTitle, Title, Title2, Title3, Headline, Subheadline,
Body, Callout, Footnote, Caption, Caption2`, plus `System(pt)`. They map to the platform's native text
styles and scale with the OS accessibility text size.

**External Piece (native widget from a crate; no core edits)**

```rust
use day_piece_combobox::combo_box;
let items = Signal::new(vec!["a".into(), "b".into()]);
let sel = Signal::new(Some(0usize));
combo_box(items, sel).id("combo")
```

## API quick reference

| Need | Use |
|---|---|
| static / reactive text | `label("x")` / `label(move || …)` |
| button | `button("x").action(\|\| …)` |
| text input | `text_field(sig)` · secure: `secure_field(sig)` |
| number input | `slider(sig).range(a..=b)` · `stepper(sig)` |
| boolean | `toggle(sig)` |
| choice | `picker(opts, sig)` · external `combo_box(opts, sig)` |
| vertical / horizontal / z-stack | `column((…))` / `row((…))` / `stack_z((…))` |
| scroll · spacer · divider | `scroll(child)` · `spacer()` · `divider()` |
| conditional · list | `when(cond, view)` · `each(items, key, row)` |
| progress · busy | `progress(frac)` · `spinner()` |
| custom drawing | `canvas(\|d, size\| …)` (native 2D; Day never rasterizes) |
| nav (one-of-N / stack) | `selector(sig)` / `stack(path, root)` |
| localize | `tr("key").arg("n", val)` |
| accessibility | `.a11y(\|a\| a.role(Role::…).label("…"))` |
| identify for tests | `.id("stable-id")` / `.id_keyed("row", key)` |

## CLI reference

```bash
day new app <name> --toolkit <t1,t2>  # scaffold an app (bare `day new` = interactive)
day build   -p <target>               # compile
day launch  -p <target>               # build + run (streams stdout/stderr)
day launch  -p <target> --script s.yaml   # build + run + drive/assert
day pack    -p <target>               # installable artifact (.app.zip / .apk / .dmg)
day lint                              # ids, Fluent coverage, project shape
day doctor                            # toolchains per target
```

## Verifying your work (dayscript)

Assert a *running* app with a cross-platform YAML script; Pieces are addressed by their `.id`, routes by
`selector`/`stack` keys.

```yaml
name: check
flow:
  - wait_for: { id: counter }
  - tap: { id: inc }
  - assert_text: { id: counter, text: "1 clicks" }
  - navigate: { route: settings }
  - assert_route: { route: settings }
  - screenshot: settings
```

## Failure modes (do NOT do these)

- ❌ Enabling two toolkit features in one binary → `compile_error!`. Enable exactly one via the target.
- ❌ Rebuilding the view tree to reflect state. ✅ Bind a Signal (closure read or pass the Signal).
- ❌ Passing a `String`/value where dynamic content is wanted. ✅ Pass `move || …sig.get()…`.
- ❌ Wrapping `Signal` in `Rc`/`Arc`. ✅ It is `Copy`; move it directly.
- ❌ Omitting `.id(...)` on Pieces you need to test/script/deep-link.
- ❌ Hand-editing generated `platform/ios/*.xcodeproj` or `platform/android`. ✅ Edit `day.yaml`/Rust.
- ❌ Concluding a target works from `cargo build`. ✅ `day launch -p <target>` (and `--script` to assert).
- ❌ Hard-coded pixel font sizes for shipping text. ✅ Semantic `Font` styles (accessibility-scaled).

## Deeper references

Human-oriented pages with the same facts in narrative form: [Overview](/docs/overview) ·
[Why Day](/docs/benefits) · [API tour](/docs/api-tour) · [CLI & projects](/docs/cli). Machine index:
[`/llms.txt`](/llms.txt).
