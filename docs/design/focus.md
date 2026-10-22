---
title: "Focus design decisions"
description: "Focus design decisions: retained contributor discussion and its implementation boundaries."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Focus design decisions

> [!NOTE]
> **Status: resolved design discussion.** This contributor record preserves earlier design and implementation
> discussion. Examples here are not stable API guidance. See the [app documentation](../focus.md)
> for the current entry point. Platform execution claims are historical and were not rerun in Phase 2.

## 6. Prior art: what Day adopted and rejected

- **SwiftUI** (`@FocusState`, `.focused(_:equals:)`). Adopted: the Bool + Optional-of-Hashable
  binding shape, `nil` clears focus and dismisses the keyboard, moved focus writes back on loss.
  Rejected: the unconstructible binding (state can't live outside the view); Day signals have
  no such restriction.
- **Flutter** (`FocusNode`/`FocusScope`). Rejected as an API (imperative node lifecycle inside a
  declarative tree); adopted as semantics: "focus changes apply after the build phase" is
  rule 3, and the scope-restore behavior informs the reserved focus-scope design.
- **floem** (the nearest cousin): its `request_focus(when)` showed that signal-driven focus
  writes work; its event-only *reads* are the asymmetry rule 2 closes.
- **iced / egui / Slint / GPUI**: converge on last-write-wins, next-frame application, and
  focus-nowhere being representable, all reflected in the rules.

## 7. Resolved design questions

1. **Group-signal loss semantics** — solved in the pump: a `FocusChanged(false)` scans the
   queue for a paired `FocusChanged(true)` from another node and dispatches the gain first, so
   group signals never blip through `None` (rule 6).
2. **Resign target on Android/XAML** — both resign to a focusable root: Android to the
   activity's focusable-in-touch-mode content root, XAML to a hidden focus-sink control.
3. **`focused_eq` naming** — one `.focused()` name via two marker-trait impls; the tuple form
   `(signal, key)` replaces a second method.
4. **Headless CI focus** — programmatic `grab_focus`/`setFocus` is window-local on every
   backend (Qt after the app-local activation above), so scripted runs don't depend on the
   window manager granting real OS focus.
