---
navTitle: "Agent instructions"
title: For AI Agents
description: A terse, rule-based reference for LLMs and coding agents writing Day apps. Follow the invariants verbatim; prefer the patterns shown.
order: 60
section: Reference
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

This reference lists conventions and constraints for coding agents working on Day apps.
The examples are intended for direct reuse. A machine-readable site index is available at
[`/llms.txt`](/llms.txt).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/examples/catalog) and [catalog setup](/docs/how-to/localization).

## Naming (disambiguate before writing)

- **Day**: the framework (proper noun; always capitalized in prose).
- `day`: the CLI binary. You type `day build`, `day launch`, etc. Always lowercase.
- `day`: the Rust crate. `use day::prelude::*;` imports common UI APIs. Always lowercase.
- `Day.toml`: the project manifest. **Piece**: a UI node (SwiftUI View / Flutter Widget). **Signal**:
  a reactive state cell. **target**: an `(OS, toolkit)` pair, e.g. `macos-appkit`, `ios-uikit`.

## What Day is (facts)

Day builds cross-platform desktop + mobile apps in Rust. You write one declarative UI as a tree of
**Pieces**; each Piece is realized by a native widget ([`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield), [`UILabel`](https://developer.apple.com/documentation/uikit/uilabel), [`GtkEntry`](https://docs.gtk.org/gtk4/class.Entry.html),
[`QSlider`](https://doc.qt.io/qt-6/qslider.html), XAML [`TextBox`](https://learn.microsoft.com/en-us/uwp/api/windows.ui.xaml.controls.textbox), `android.widget.*`) through a per-platform toolkit toolkit. Day owns layout,
reactivity, localization, accessibility policy, and scripting; the OS owns pixels, text input, scrolling,
and assistive tech. The native tree is built once and Signals bind straight to native attributes;
nothing diffs or rebuilds it.

## Invariants (MUST; violating these is a bug)

1. **One toolkit toolkit per binary.** A binary compiles exactly one toolkit (selected by a Cargo
   feature via the target). Never enable two. The build enforces this with a `compile_error!`.
2. **Views are built once, then kept live by bindings, never rebuilt.** Never rebuild the view on state change. To make UI reactive, pass a
   closure that reads a Signal ([`label`](https://daybrite.dev/api/day_pieces/fn.label.html)`(move || …count`[`.get`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.get)`()…)`), or pass a Signal to a control
   ([`slider(volume)`](https://daybrite.dev/api/day_pieces/fn.slider.html)). Do not diff, re-run, or recreate Piece trees yourself.
3. **`Signal<T>` is `Copy`.** Clone/move it into as many closures as you need; do not wrap it in `Rc`.
4. **Give every interactive/asserted Piece a stable [`.id(id!(name))`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id).** Tests and dayscript
   use its canonical spelling (`name` in YAML). Recycled rows need
   [`.id_of`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id_of)`(move || id!(row, slot`[`.key`](https://daybrite.dev/api/day_pieces/struct.ItemSlot.html#method.key)`()))`. Route and model keys remain separate from element IDs.
5. **Localize user-facing text** with Fluent files. Scaffolded apps generate a typed function
   per key (`res::str::my_key()`, parameters become typed arguments), so a missing key or
   wrong arity is a compile error. Always call those, never [`tr("key")`](https://daybrite.dev/api/day/fn.tr.html) with a literal (`day lint`
   reports it as `string-key`); Day's own strings are there too ([`res::str::day_cancel()`](https://daybrite.dev/api/day_fluent/res/str/fn.day_cancel.html)). Messages
   with multiple variables take generated named-field structs. Include accessibility labels,
   hints, errors, and units in the catalog. Provider data and synthetic test inputs stay verbatim.
6. **Edit `Day.toml` + Rust; never hand-edit the generated Xcode/Gradle scaffolds.** `day` regenerates
   them.
7. **Verify on a real target.** `cargo build` does not prove a target works. Use `day launch -t <target>`
   and, for assertions, `day launch -t <target> --script <dayscript.yaml>`.

## Setup (canonical)

```bash
day new app my-app --toolkit macos-appkit,ios-uikit,android-mdc
cd my-app
day launch -t macos-appkit                 # build + run
day launch -t macos-appkit --script dayscript/demo.yaml    # build + run + assert
```

`Day.toml` (`name`/`version` come from Cargo.toml's `[package]`):

```toml
schema = 1

[app]
id = "dev.example.my-app"
title = "My App"
targets = ["macos-appkit", "ios-uikit", "android-mdc"]

[window]
width = 480
height = 640
```

## Core model (precise)

- A **Piece** is a value produced by a function call ([`label(...)`](https://daybrite.dev/api/day_pieces/fn.label.html), [`button(...)`](https://daybrite.dev/api/day_pieces/fn.button.html), [`column((...))`](https://daybrite.dev/api/day_pieces/fn.column.html)).
  Containers take a **tuple** of children. Builder methods ([`.padding`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.padding), [`.spacing`](https://daybrite.dev/api/day_pieces/trait.ColumnBuilder.html#tymethod.spacing), [`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id), [`.font`](https://daybrite.dev/api/day_pieces/trait.LabelBuilder.html#tymethod.font), …)
  return the Piece. End a heterogeneous Piece with [`.any()`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.any) to get `AnyPiece`.
- `Signal<T>`: [`get()`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.get) (tracked read), [`set(v)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.set), [`update(|v| …)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.update), [`with(|v| …)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.with) (borrow),
  [`get_untracked()`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.get_untracked). Reading a Signal inside a binding closure makes that binding re-run when the
  Signal changes; nothing else re-runs.
- **Reactivity rule:** pass a signal to a generated message, as in
  [`label(res::str::counter_value(count))`](https://daybrite.dev/api/day_pieces/fn.label.html). For computed values, format the message inside
  a label's closure. [`label(res::str::hi())`](https://daybrite.dev/api/day_pieces/fn.label.html) also follows locale changes.
- A **target** is `(OS, toolkit)`: `macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit`,
  `android-mdc`, `harmony-arkui`, `linux-gtk`, `linux-qt`, `windows-winui`, `windows-gtk`,
  `windows-qt`, `web-dom` (wasm in a browser; no process
  environment, so pass runtime flags with `day launch --env K=V` and read them with [`day::env("K")`](https://daybrite.dev/api/day/fn.env.html), which is portable to every target).
- Each target has a **support tier** ([definitions](/docs/reference/platforms#support-tiers)), which tells
  you how much to trust it: `ios-uikit`, `android-mdc`, `macos-appkit` are Tier 1 (thoroughly
  tested); `linux-gtk`, `linux-qt`, `windows-winui` are Tier 2; `harmony-arkui` and `web-dom` are
  Tier 3 (experimental, with real gaps); `macos-gtk`, `macos-qt`, `windows-gtk`, `windows-qt` are
  Tier 4 development combos that no app ships on and `day pack` refuses. Verify a user-facing
  change on a Tier 1 target unless the change is that platform's.

## Canonical patterns (copy these)

### App skeleton

```rust
use day::prelude::*;

day::resources!();

fn main() {
    day::launch(res::window_options().size(480.0, 640.0), root);
}

fn root() -> impl Piece {
    let count = Signal::new(0i64);
    column((
        label(res::str::click_count(count))
            .font(Font::Title)
            .id(id!(counter)),
        row((
            button(res::str::decrement())
                .action(move || count.update(|c| *c -= 1))
                .id(id!(dec)),
            button(res::str::increment())
                .action(move || count.update(|c| *c += 1))
                .id(id!(inc)),
        ))
        .spacing(Space::S),
    ))
    .spacing(Space::M)
    .align(HAlign::Leading)
    .padding(Space::L)
}
```

### Inputs (two-way; edits flow back into the Signal)

```rust
let name = Signal::new(String::new());
let volume = Signal::new(40.0);
let on = Signal::new(false);
column((
    text_field(name)
        .placeholder(res::str::your_name())
        .id(id!(name)),
    slider(volume).range(0.0..=100.0).step(1.0).id(id!(vol)),
    toggle(on).id(id!(on)),
    progress(move || volume.get() / 100.0), // tracks the slider live
))
```

### Conditionals + keyed collections

```rust
when(move || !name.with(|s| s.is_empty()),
     move || label(res::str::greeting(name)))

// `each` builds one child per item and reconciles by key (each row keeps its own state).
// The row builder receives an ItemSlot (a Copy handle), not the item: read fields through it
// so recycled rows update when the backing item changes.
each(
    day::prelude::items(move || items.get(), |it| it.id.clone()),
    |slot| label(move || slot.field(|it| it.title.clone())).id_of(move || id!(row, slot.key())),
)
```

### Navigation (a projection of an app-owned Signal; you own the state)

```rust
// one-of-N (Sidebar → split view; Tabs → native tabs):
let section = Signal::new(String::new());
nav(section)
    .style(NavStyle::Sidebar)
    .title(res::str::my_app())
    .item("home", res::str::home(), home_page)
    .item("settings", res::str::settings(), settings_page)
    .id(id!(nav))

// push/pop stack bound to a path Signal:
let path = Signal::new(Vec::<String>::new());
nav_stack(path, home_view).destination(|key| detail_view(key));
// push: path.update(|p| p.push("item-42".into()));  pop is written back by the native back button.

navigate("settings");  nav_back();  current_route();   // string-route adapter (also deep links + dayscript)
nav_stack(path, root).on_back(|req| if dirty.get() { BackResponse::Handled } else { BackResponse::Proceed })  // intercept back
```

### Text, fonts, color, accessibility

```rust
label(res::str::chapter()).font(Font::Title).bold()               // semantic style + weight
label(res::str::caption()).font(Font::Footnote).italic()
label(res::str::custom_size_sample()).font(Font::System(18.0))                  // fixed point size; prefer semantic styles
label(res::str::greeting(name))                 // localized + interpolated Signal
progress(move || v.get() / 100.0).a11y(|a| a.role(Role::Meter).label(res::str::volume()))
```

Semantic `Font` styles (largest→smallest): `LargeTitle, Title, Title2, Title3, Headline, Subheadline,
Body, Callout, Footnote, Caption, Caption2`, plus `System(pt)`. They map to the platform's native text
styles and scale with the OS accessibility text size.

### External Piece (native widget from a crate; no core edits)

```rust
use day_pieces::combobox::combo_box;
let items = Signal::new(vec!["a".into(), "b".into()]);
let text = Signal::new(String::new());
combo_box(items, text).id(id!(combo))
```

SwiftUI views embed via [`day-piece-swiftui`](/docs/reference/swiftui) on macos-appkit / ios-uikit only:
[`swiftui("name")`](https://daybrite.dev/api/day_piece_swiftui/fn.swiftui.html) resolves a provider class, or a local SwiftPM package's views become typed
`crate::swiftui::MyView(…)` constructors. Gate the UI on the probe ([`support()`](https://daybrite.dev/api/day_piece_swiftui/fn.support.html) is fixed per
binary, so a plain `if` at build time is right):

```rust
if day_piece_swiftui::support() == Support::Native {
    swiftui("hello").frame(320.0, 240.0).any()
} else {
    day_native_fallback()
}
```

**Invariant:** [`support()`](https://daybrite.dev/api/day_piece_swiftui/fn.support.html) probes are the only correct gate for platform-limited pieces. Never
gate on `target_os` or toolkit-feature cfgs; `target_os = "macos"` also covers macos-gtk and
macos-qt, which have no AppKit view tree.

## API quick reference

| Need | Use |
|---|---|
| static / reactive text | [`label(res::str::sample_label())`](https://daybrite.dev/api/day_pieces/fn.label.html) / `label(move || …)` |
| button | [`button`](https://daybrite.dev/api/day_pieces/fn.button.html)`(res::str::sample_label())`[`.action`](https://daybrite.dev/api/day_pieces/trait.ButtonBuilder.html#tymethod.action)`(\|\| …)` |
| text input | [`text_field(sig)`](https://daybrite.dev/api/day_pieces/fn.text_field.html) · multiline: [`text_area(sig)`](https://daybrite.dev/api/day_pieces/fn.text_area.html) |
| number input | [`slider`](https://daybrite.dev/api/day_pieces/fn.slider.html)`(sig)`[`.range`](https://daybrite.dev/api/day_pieces/trait.SliderBuilder.html#tymethod.range)`(a..=b)` |
| boolean | [`toggle(sig)`](https://daybrite.dev/api/day_pieces/fn.toggle.html) |
| choice | [`picker(sig, [(value, title), …])`](https://daybrite.dev/api/day_pieces/fn.picker.html) · editable: external [`combo_box(opts, text_sig)`](https://daybrite.dev/api/day_pieces/combobox/fn.combo_box.html) |
| vertical / horizontal / z-stack | [`column((…))`](https://daybrite.dev/api/day_pieces/fn.column.html) / [`row((…))`](https://daybrite.dev/api/day_pieces/fn.row.html) / [`zstack((…))`](https://daybrite.dev/api/day_pieces/fn.zstack.html) |
| scroll · spacer · divider | [`scroll(child)`](https://daybrite.dev/api/day_pieces/fn.scroll.html) · [`spacer()`](https://daybrite.dev/api/day_pieces/fn.spacer.html) · [`divider()`](https://daybrite.dev/api/day_pieces/fn.divider.html) |
| conditional · list | [`when(cond, view)`](https://daybrite.dev/api/day_pieces/fn.when.html) · [`each`](https://daybrite.dev/api/day_pieces/fn.each.html)`(`[`items`](https://daybrite.dev/api/day_pieces/fn.items.html)`(data, key), row)` |
| progress · busy | [`progress(frac)`](https://daybrite.dev/api/day_pieces/fn.progress.html) · [`spinner()`](https://daybrite.dev/api/day_pieces/fn.spinner.html) |
| custom drawing | [`canvas(\|d, size\| …)`](https://daybrite.dev/api/day_pieces/fn.canvas.html) (native 2D; Day never rasterizes) |
| nav (one-of-N / stack) | [`nav(sig)`](https://daybrite.dev/api/day_pieces/fn.nav.html) / [`nav_stack(path, root)`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html) |
| localize | `res::str::counter_value(count)` |
| accessibility | [`.a11y`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.a11y)`(\|a\| a`[`.role`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.role)`(Role::Meter)`[`.label`](https://daybrite.dev/api/day_pieces/struct.A11yBuilder.html#method.label)`(res::str::volume()))` |
| identify for tests | [`.id(id!(stable_id))`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) / [`.id(id!(row, key))`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) |

## CLI reference

```bash
day new app <name> --toolkit <t1,t2>  # scaffold an app (bare `day new` = interactive)
day build   -t <target>               # compile
day launch  -t <target>               # build + run (streams stdout/stderr)
day launch  -t <target> --script s.yaml   # build + run + drive/assert
day launch  -t <target> --record s.yaml   # record your manual session as a replayable dayscript
#   scripted-launch flags: --variant <name>, --locales <l,…> / --themes <t,…>
#   (capture matrix: one run per theme×locale), --keep-alive, --skip-build,
#   --ios-device / --ios-simulator / --android-device (pick the device per runtime)
day pack    -t <target>               # installable artifact (.dmg / .ipa / .aab / .hap / flatpak / installer)
day lint                              # ids, Fluent coverage, project shape
day doctor                            # toolchains per target
day doctor verify [-t <target>,…]           # doctor, then scaffold + build + pack a throwaway app per target
#   --day-version <main|x.y.z|latest>  checks that Day: installs its CLI and pins the scaffold to it
day relaunch --all-running            # stop + rebuild + relaunch — "apply my changes"
day stop --all                        # stop every recorded session
day drive -t <target> --steps-json …  # drive a running app (see below)
day mcp-server                        # serve all of the above as MCP tools (stdio)
```

## Verifying your work (dayscript)

Assert a *running* app with a cross-platform YAML script; Pieces are addressed by their [`.id`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id), routes by
[`nav`](https://daybrite.dev/api/day_pieces/fn.nav.html)/[`nav_stack`](https://daybrite.dev/api/day_pieces/fn.nav_stack.html) keys.

```yaml
name: check
flow:
  - wait_for: { id: counter }
  - tap: { id: inc }
  - assert_text: { id: counter, text: "1 clicks" }
  - navigate: { route: settings }
  - assert_route: { route: settings }
  - pause: { secs: 1.0 }        # not `pause: 1s` — a mapping with float secs (or a bare int)
  - screenshot: settings
```

For apps with network data, ship a deterministic mock behind a `day launch --env MY_MOCK=1`
flag (read it with [`day::env`](https://daybrite.dev/api/day/fn.env.html), which also works on web-dom) and write the walkthrough against
the mock's exact values. Keep the mock's arithmetic integer-derived. Transcendentals can
round differently across platforms and break cross-target `assert_text` on formatted numbers.

## Driving a running app (`day drive` / MCP)

Every `day launch` embeds a loopback automation engine and records its coordinates in
`build/day/sessions.json`, so you can drive an app that is already running, without a script
file:

```bash
day drive -t macos-appkit --steps-json \
  '[{"navigate":{"route":"settings"}},{"wait_idle":null},
    {"tap":{"id":"save-button"}},{"assert_text":{"id":"status","text":"Saved"}},
    {"screenshot":"after-save"}]'
```

Output is JSON (per-step `ok`/`error`, screenshot paths + base64). The step vocabulary is
[the dayscript reference's table](/docs/how-to/dayscript#the-step-vocabulary), the same steps a
walkthrough file uses. `assert_no_placeholders` fails if any piece rendered the `⟨kind⟩`
placeholder; it is the step that catches a missing renderer.

If your host exposes MCP (VS Code agent mode does automatically in Day workspaces via the Day
extension), use the `day_*` tools instead: `day_metadata`, `day_build`, `day_launch`,
`day_relaunch`, `day_stop`, `day_running`, `day_drive`, `day_screenshot`, `day_doctor`,
`day_lint`. `day_drive`/`day_screenshot` return screenshots as images. Look at them to
verify UI changes on every target you touched. The canonical loop: edit → `day_relaunch`
(compile errors come back in the result) → `day_drive` (navigate + assert + screenshot).

## Failure modes (do not do these)

- ❌ Enabling two toolkit features in one binary → `compile_error!`. Enable exactly one via the target.
- ❌ Rebuilding the view tree to reflect state. ✅ Bind a Signal (closure read or pass the Signal).
- ❌ Passing a `String`/value where dynamic content is wanted. ✅ Pass [`move || …sig.get()…`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.get).
- ❌ Wrapping `Signal` in `Rc`/`Arc`. ✅ It is `Copy`; move it directly.
- ❌ Omitting [`.id(...)`](https://daybrite.dev/api/day_pieces/trait.Decorate.html#method.id) on Pieces you need to test/script/deep-link.
- ❌ Hand-editing generated projects under `build/day/xcode/`, `build/android/`, or `build/day/harmony/project/`. ✅ Edit `Day.toml`/Rust.
- ❌ Concluding a target works from `cargo build`. ✅ `day launch -t <target>` (and `--script` to assert).
- ❌ Declaring a UI change done without looking at it. ✅ `day_drive`/`day drive` → `screenshot` → inspect.
- ❌ Hard-coded pixel font sizes for shipping text. ✅ Semantic `Font` styles (accessibility-scaled).

## Deeper references

Human-oriented pages with the same facts in narrative form: [Overview](/docs/explanation/overview) ·
[Why Day](/features) · [API tour](/docs/reference/api-tour) · [CLI & projects](/docs/reference/cli). Machine index:
[`/llms.txt`](/llms.txt).
