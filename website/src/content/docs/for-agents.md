---
navTitle: "Agent instructions"
title: Agent instructions
description: Required conventions for UI, state, resources, and target verification.
order: 60
section: Reference
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Coding agents need the same project conventions and API constraints as other contributors. This
reference gathers the Day-specific rules they need when generating or editing an app.

The examples are intended for direct reuse. A machine-readable site index is available at
[`/llms.txt`](/llms.txt).

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Terminology

- **Day** is the framework; `day` is its CLI and Rust library. The Cargo package is `day-app`.
- `Day.toml` is the app manifest.
- A **Piece** describes a UI control or container.
- A **Signal** stores reactive state.
- A **target** pairs an OS and toolkit, such as `macos-appkit` or `ios-uikit`.

## Architecture

Day implements desktop and mobile UIs in Rust using native platform controls. Day manages
layout, reactivity, localization, accessibility policy, and scripting. Platform toolkits provide
rendering, text input, scrolling, and assistive technology integration.

Day builds the root UI once. Bindings update control properties when signals change.
Conditional and collection pieces create and dispose child subtrees as their inputs change.

## Required conventions

1. Select exactly one toolkit per binary through the target's Cargo feature.
2. Bind dynamic content to a Signal or a closure that reads it. A value read outside a binding
   is a snapshot. Do not manually rebuild or diff Piece trees on state changes.
3. Copy `Signal<T>` handles into closures; do not wrap them in `Rc` or `Arc`.
4. Give interactive and tested pieces stable [`.id(id!(name))`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id) identifiers.
   Use `name` in dayscript YAML. For recycled rows, use
   [`.id_of`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id_of) with
   [`id!(row, slot.key())`](https://docs.rs/day-pieces/latest/day_pieces/struct.ItemSlot.html#method.key)
   to identify the current item.
   Keep route and model keys separate from element IDs.
5. Put app-owned UI text in Fluent catalogs and use generated `res::str` accessors, including
   errors, units, and accessibility text. Generated signatures check keys and arguments;
   messages with multiple variables use named-field structs. Do not use literal keys with
   [`tr`](https://docs.rs/day-app/latest/day/fn.tr.html). Keep provider data and test inputs verbatim.
6. Edit `Day.toml` and Rust sources. Do not edit generated Xcode, Gradle, or HarmonyOS projects.
7. Verify each target with `day launch -t <target>` and
   `day launch -t <target> --script <dayscript.yaml>`. Compilation alone does not verify native behavior.
   Inspect screenshots after UI changes using `day drive` or `day_drive`.
8. Use semantic `Font` styles for text that must scale with accessibility settings.

## Setup (canonical)

```bash
day new app my-app --toolkit macos-appkit,ios-uikit,android-material
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
targets = ["macos-appkit", "ios-uikit", "android-material"]

[window]
width = 480
height = 640
```

## Core model (precise)

- A **Piece** is a value produced by a function call ([`label(...)`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html), [`button(...)`](https://docs.rs/day-pieces/latest/day_pieces/fn.button.html), [`column((...))`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html)).
  Containers take a **tuple** of children. Builder methods ([`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding), [`.spacing`](https://docs.rs/day-pieces/latest/day_pieces/trait.ColumnBuilder.html#tymethod.spacing), [`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id), [`.font`](https://docs.rs/day-pieces/latest/day_pieces/trait.LabelBuilder.html#tymethod.font), …)
  return the Piece. End a heterogeneous Piece with [`.any()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.any) to get `AnyPiece`.
- `Signal<T>`: [`get()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.get) (tracked read), [`set(v)`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.set), [`update(|v| …)`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.update), [`with(|v| …)`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.with) (borrow),
  [`get_untracked()`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.get_untracked). Reading a Signal inside a binding closure makes that binding re-run when the
  Signal changes; nothing else re-runs.
- **Reactivity rule:** pass a signal to a generated message, as in
  [`label(res::str::counter_value(count))`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html). For computed values, format the message inside
  a label's closure. [`label(res::str::hi())`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html) also follows locale changes.
- A **target** is `(OS, toolkit)`: `macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit`,
  `android-material`, `harmony-arkui`, `linux-gtk`, `linux-qt`, `windows-winui`, `web-dom` (wasm in a browser; no process
  environment, so pass runtime flags with `day launch --env K=V` and read them with [`day::env("K")`](https://docs.rs/day-app/latest/day/fn.env.html), which is portable to every target).
- Each target has a **support tier** ([definitions](/docs/reference/platforms#support-tiers)), which tells
  you how much to trust it: `ios-uikit`, `android-material`, `macos-appkit` are Tier 1 (thoroughly
  tested); `linux-gtk`, `linux-qt`, `windows-winui` are Tier 2; `harmony-arkui` and `web-dom` are
  Tier 3 (experimental, with real gaps); `macos-gtk` and `macos-qt` are
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
[`swiftui("name")`](https://docs.rs/day-piece-swiftui/latest/day_piece_swiftui/fn.swiftui.html) resolves a provider class, or a local SwiftPM package's views become typed
`crate::swiftui::MyView(…)` constructors. Gate the UI on the probe ([`support()`](https://docs.rs/day-piece-swiftui/latest/day_piece_swiftui/fn.support.html) is fixed per
binary, so a plain `if` at build time is right):

```rust
if day_piece_swiftui::support() == Support::Native {
    swiftui("hello").frame(320.0, 240.0).any()
} else {
    day_native_fallback()
}
```

**Invariant:** [`support()`](https://docs.rs/day-piece-swiftui/latest/day_piece_swiftui/fn.support.html) probes are the only correct gate for platform-limited pieces. Never
gate on `target_os` or toolkit-feature cfgs; `target_os = "macos"` also covers macos-gtk and
macos-qt, which have no AppKit view tree.

## API quick reference

| Need | Use |
|---|---|
| static / reactive text | [`label(res::str::sample_label())`](https://docs.rs/day-pieces/latest/day_pieces/fn.label.html) / `label(move || …)` |
| button | [`button`](https://docs.rs/day-pieces/latest/day_pieces/fn.button.html)`(res::str::sample_label())`[`.action`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.action)`(\|\| …)` |
| text input | [`text_field(sig)`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_field.html) · multiline: [`text_area(sig)`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_area.html) |
| number input | [`slider`](https://docs.rs/day-pieces/latest/day_pieces/fn.slider.html)`(sig)`[`.range`](https://docs.rs/day-pieces/latest/day_pieces/trait.SliderBuilder.html#tymethod.range)`(a..=b)` |
| boolean | [`toggle(sig)`](https://docs.rs/day-pieces/latest/day_pieces/fn.toggle.html) |
| choice | [`picker(sig, [(value, title), …])`](https://docs.rs/day-pieces/latest/day_pieces/fn.picker.html) · editable: external [`combo_box(opts, text_sig)`](https://docs.rs/day-pieces/latest/day_pieces/combobox/fn.combo_box.html) |
| vertical / horizontal / z-stack | [`column((…))`](https://docs.rs/day-pieces/latest/day_pieces/fn.column.html) / [`row((…))`](https://docs.rs/day-pieces/latest/day_pieces/fn.row.html) / [`zstack((…))`](https://docs.rs/day-pieces/latest/day_pieces/fn.zstack.html) |
| scroll · spacer · divider | [`scroll(child)`](https://docs.rs/day-pieces/latest/day_pieces/fn.scroll.html) · [`spacer()`](https://docs.rs/day-pieces/latest/day_pieces/fn.spacer.html) · [`divider()`](https://docs.rs/day-pieces/latest/day_pieces/fn.divider.html) |
| conditional · list | [`when(cond, view)`](https://docs.rs/day-pieces/latest/day_pieces/fn.when.html) · [`each`](https://docs.rs/day-pieces/latest/day_pieces/fn.each.html)`(`[`items`](https://docs.rs/day-pieces/latest/day_pieces/fn.items.html)`(data, key), row)` |
| progress · busy | [`progress(frac)`](https://docs.rs/day-pieces/latest/day_pieces/fn.progress.html) · [`spinner()`](https://docs.rs/day-pieces/latest/day_pieces/fn.spinner.html) |
| custom drawing | [`canvas(\|d, size\| …)`](https://docs.rs/day-pieces/latest/day_pieces/fn.canvas.html) (native 2D; Day never rasterizes) |
| nav (one-of-N / stack) | [`nav(sig)`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html) / [`nav_stack(path, root)`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_stack.html) |
| localize | `res::str::counter_value(count)` |
| accessibility | [`.a11y`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.a11y)`(\|a\| a`[`.role`](https://docs.rs/day-pieces/latest/day_pieces/struct.A11yBuilder.html#method.role)`(Role::Meter)`[`.label`](https://docs.rs/day-pieces/latest/day_pieces/struct.A11yBuilder.html#method.label)`(res::str::volume()))` |
| identify for tests | [`.id(id!(stable_id))`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id) / [`.id(id!(row, key))`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id) |

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

Assert a *running* app with a cross-platform YAML script; Pieces are addressed by their [`.id`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.id), routes by
[`nav`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html)/[`nav_stack`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_stack.html) keys.

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
flag (read it with [`day::env`](https://docs.rs/day-app/latest/day/fn.env.html), which also works on web-dom) and write the walkthrough against
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

## Deeper references

Human-oriented pages with the same facts in narrative form: [Overview](/docs/explanation/overview) ·
[Why Day](/features) · [API tour](/docs/reference/api-tour) · [CLI & projects](/docs/reference/cli). Machine index:
[`/llms.txt`](/llms.txt).
