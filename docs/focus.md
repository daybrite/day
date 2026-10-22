---
title: "Focus"
description: "Keyboard focus as a reactive signal: focus order, programmatic focus, and per-toolkit behavior."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Focus

Bind keyboard focus to a signal to observe the active control or move focus from your app.
Text inputs support this across toolkits. Focusable containers and non-text controls have
platform limits described below.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](snippet-catalog.md) and [catalog setup](/docs/how-to/localization).

## 1. The API

Focus binds to a signal with `.focused(…)`, exactly like every other two-way control binding.
One name covers both shapes (a marker-trait overload, the `IntoText` pattern):

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
        …
    }),
))
```

- **Native → signal:** when the control gains focus the signal becomes `true` / `Some(K)`; when
  it loses focus, `false` / `None`, unless another bound control gained it in the same turn, in
  which case the group signal moves straight to that control's value (no `None` in between).
- **Signal → native:** writing `Some(K)` / `true` requests native focus for the bound control.
  Writing `None` / `false` resigns it; on iOS and Android that dismisses the soft keyboard,
  matching the platform convention SwiftUI set.
- **Reading is free.** `focus.get() == Some(Field::User)` is a tracked read like any other,
  with nothing else to wire.
- **Field chaining** is a write in `on_submit(f)`, which ships with the same change: the native
  end-editing / return hooks focus needs are the ones `Event::Submitted` needs.

`.focusable()` opts a composed container into focus: the canvas contract (press-to-focus,
`FocusChanged` both ways, the keys through `.on_key` while focused) behind the
`Toolkit::set_focusable` duty. AppKit, UIKit, GTK, Qt, Android and DOM implement it for
containers. Editable descendants retain their editing keys; a container can request focus with
`.focused(...)` after navigation. XAML and ArkUI still require a canvas or native control as the
keyboard focus target; their container duty remains a no-op. Reserved with names but not implemented: `default_focus(…)`
on containers, `focus_order(n)`, and focus scopes for dialogs. Tab/Shift-Tab traversal stays
native, with focus order following layout order.

## 2. Semantics: the rules

1. **Writes are requests.** A write asks the toolkit on the next main-loop turn; the last write
   in a turn wins.
2. **The signal converges on reality.** Toolkits report the *resulting* state through
   `Event::FocusChanged`, and that report is what lands in the signal (through the echo guard).
   A request naming a target the platform will not focus produces no event: platform focus is
   unchanged, and the next real focus event corrects the signal.
3. **One-turn latency.** `focus.set(…)` then `focus.get()` in the same turn reads the old
   value, the same asynchrony every reference framework has, made explicit.
4. **Mount reconciliation.** When a piece bound to `K::V` mounts and the signal already reads
   `Some(K::V)`, it requests focus. Set the signal, then present the sheet: the field focuses
   when it appears. (The initial `false`/`None` is *not* applied: resigning focus a control
   never had would steal it from whoever has it.)
5. **Echo discipline.** A programmatic focus move fires native focus events; those must not
   re-request. The pieces layer keeps a per-binding echo cell of the native state and skips
   applies that already match it; toolkits additionally skip resigns when the control doesn't
   own focus (so a stale release can't blur a sibling).
6. **Loss and gain pair up.** When focus moves between two controls bound to the same group
   signal, the event pump dispatches the queued gain *before* the loss it arrived with, so the
   signal transitions `Some(A)` → `Some(B)` without an observable `None`. A loss with no queued
   successor (click on empty space, window resigned) writes `false` / `None`.

## Which controls take focus

Text fields take focus on every toolkit. Desktop buttons, toggles, and sliders follow the
platform's keyboard-access settings; macOS button traversal depends on Full Keyboard Access.
On touch platforms, non-text controls generally do not take keyboard focus. A canvas can
receive focus and key events; on Android, press-to-focus applies when it has an `.on_key` handler.

`.focusable()` makes composed containers keyboard targets on AppKit, UIKit, GTK, Qt, Android,
and DOM. On XAML and ArkUI, bind focus to a native control or canvas inside the container.

## Mobile keyboards

Clearing an editor's focus binding dismisses the keyboard. UIKit and Android also end editing
when navigation, cover dismissal, or subtree removal takes its editing surface away. Moving
between text fields keeps the keyboard open. Button actions that leave a form visible can
clear its focus binding explicitly after successful submission.

Place long forms in a scroll view. Android and UIKit resize the available content area when
the keyboard appears and reveal the focused field. HarmonyOS uses ArkUI's resize avoidance
mode. Test field visibility at the bottom of the form and after changing keyboard height.

## Implementation

See the [toolkit contract](backend-contracts/focus.md) for event delivery and keyboard handling.
