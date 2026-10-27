---
title: "Focus"
description: "Keyboard focus as a reactive signal: focus order, programmatic focus, and per-toolkit behavior."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Focus

Keyboard focus identifies the control that receives typing and other keyboard input. A visible focus
indicator and a predictable order help people move through a screen without a pointer.

Bind keyboard focus to a signal to observe the active control or move focus from your app. Text
inputs support this across toolkits. Focusable containers and non-text controls have platform limits
described below.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Authoring

Focus binds to a signal with [`.focused(…)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.focused), exactly like every other two-way control binding.
Use a boolean for one control, or an optional key to coordinate a group:

```rust
// One field: a Bool signal. Native focus writes it; writing it moves focus.
let editing = Signal::new(false);
text_field(query).focused(editing)

// A form: one Option<K> signal for the whole group (the recommended shape).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field { User, Pass }
let focus = Signal::new(Some(Field::User));
column((
    text_field(user)
        .on_submit(move || focus.set(Some(Field::Pass)))
        .focused((focus, Field::User)),
    text_field(pass).focused((focus, Field::Pass)),
    button(res::str::log_in()).action(move || {
        if user.with(|s| s.is_empty()) { focus.set(Some(Field::User)); return; }
        // Continue with the app's sign-in action.
    }),
))
```

- **Native → signal:** when the control gains focus the signal becomes `true` / `Some(K)`; when
  it loses focus, `false` / `None`, unless another bound control gained it in the same turn, in
  which case the group signal moves straight to that control's value (no `None` in between).
- **Signal → native:** writing `Some(K)` / `true` requests native focus for the bound control.
  Writing `None` / `false` resigns it; on iOS and Android that dismisses the soft keyboard,
  matching the platform convention SwiftUI set.
- **Read the binding.** [`focus.get() == Some(Field::User)`](https://docs.rs/day-reactive/latest/day_reactive/struct.Signal.html#method.get) is a tracked read like any other,
  with nothing else to wire.
- **Field chaining** is a write in [`on_submit(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.on_submit), to move to the next field after submission.

## Focus requests and lifetime

Writing a focus binding updates the signal immediately and requests a native focus change.
The platform may refuse the request, for example when the control cannot take focus. A signal
read immediately after the write does not confirm that the native control has gained focus;
subsequent native focus events reconcile the binding with the actual state.

When a bound control mounts and its binding already selects it, it requests focus. This lets
an app select a field before presenting a sheet. An initial `false` or `None` does not resign
another control's focus.

Moving between controls in the same group updates the selected key. Losing focus without a
successor clears it. Tab and Shift-Tab traversal follows the platform's focus order; Day does
not expose custom focus-order or dialog focus-scope builders.

## Which controls take focus

Text fields take focus on every toolkit. Desktop buttons, toggles, and sliders follow the
platform's keyboard-access settings; macOS button traversal depends on Full Keyboard Access.
On touch platforms, non-text controls generally do not take keyboard focus. A canvas can
receive focus and key events; on Android, press-to-focus applies when it has an [`.on_key`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_key) handler.

[`.focusable()`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.focusable) makes composed containers keyboard targets on AppKit, UIKit, GTK, Qt, Android,
and DOM. On WinUI and ArkUI, bind focus to a native control or canvas inside the container.

## Mobile keyboards

Clearing an editor's focus binding dismisses the keyboard. UIKit and Android also end editing
when navigation, cover dismissal, or subtree removal takes its editing surface away. Moving
between text fields keeps the keyboard open. Button actions that leave a form visible can
clear its focus binding explicitly after successful submission.

Place long forms in a scroll view. Android and UIKit resize the available content area when
the keyboard appears and reveal the focused field, in a cover or sheet as well as in the
window, so a button row at the bottom of a sheet stays above the keyboard. HarmonyOS uses ArkUI's resize avoidance
mode. Test field visibility at the bottom of the form and after changing keyboard height.
