---
title: "Dialogs & modals"
description: "Imperative presentation via present(): alerts, confirmations, sheets, and how dismissal returns a value."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Dialogs, modals & imperative presentation (`present`)

Alerts, confirmations, action sheets, text prompts (and later native pickers) are
*imperative request→response* interactions: an action opens one and needs the answer
back. SwiftUI models this as a detached binding
(`showAlert = true` … `.alert($showAlert)`) because `body` re-runs. In Day a
`button().action(|| …)` is a closure on the persistent main thread, so the request and
its response sit together with async/await:

```rust
button(tr("delete")).action(|| day::task(async move {
    let choice = Alert::new(tr("delete-title"))
        .message(tr("delete-body"))
        .destructive(tr("delete"), Choice::Delete)   // a button carrying a payload
        .cancel(tr("cancel"))                          // dismissal → None
        .present().await;                              // -> Option<Choice>
    if choice == Some(Choice::Delete) {
        store.delete(id);
    }
}));
```

The request and its answer live in one closure. `day::task` is the one
explicit opt-in ("this action starts an async flow").

## Layers

**Layer 0: the primitive (plumbing).** Mirrors the `nav` controller pattern: an
imperative call routes a request through the tree to the backend, and the answer flows
back through the enqueue-only `Event` sink.

- `day_spec::present::PresentSpec`: `Dialog { title, message, buttons, sheet }` or
  `Prompt { title, message, placeholder, initial, ok, cancel }`. `PresentButton { label,
  role }`, `ButtonRole { Default, Cancel, Destructive }`.
- `PresentResult`: `Button(i64)` / `Text(String)` / `Dismissed`. Tagged so it crosses
  the C ABI (Qt/Android) as a flat payload (tag + index + string), the same style as
  canvas `encode_ops`.
- `Toolkit::present(req: u64, spec: &PresentSpec)` and `dismiss(req)` (default no-ops);
  `Event::PresentResult { req, result }`; `Cap::Dialogs`.
- `day-core` keeps a thread-local `PENDING: HashMap<u64, …>`. `present(spec)` mints a
  `req`, presents through `with_tree(|t| t.present(req, spec))`, and parks a waker. When
  the backend answers, `pump_events` routes `Event::PresentResult` to `resolve(req,
  result)`, which wakes the future. Native modals dismiss themselves; a *programmatic*
  resolve (dayscript) also calls `dismiss(req)`.

**Layer 1: async (the surface).** A tiny single-threaded executor (`day::task`, ~60
lines, `std`-only): tasks are `Pin<Box<dyn Future>>` in a thread-local map, polled on the
main loop; the presentation future registers its `Waker` and is re-polled through the
existing `Platform::post`/`on_main`. Futures are `!Send`, since there is one UI
thread, and there is no async-runtime dependency.

## API (`day-pieces::present`, re-exported in the prelude)

```rust
alert(tr("saved")).present();                    // fire-and-forget notice (1 button)
let ok: bool         = confirm(tr("quit?")).await;
let name: Option<..> = prompt(tr("name")).await; // -> Option<String>

// full builder: buttons carry a typed payload; `.cancel()` and dismissal → None
let picked: Option<Flavor> = Alert::new(tr("pick"))
    .button(tr("vanilla"), Flavor::Vanilla)
    .button(tr("pistachio"), Flavor::Pistachio)
    .sheet()                                       // bottom action sheet on mobile
    .cancel(tr("cancel"))
    .present().await;
```

Every text field is an `IntoText`, so titles/buttons localize through `tr()` (Fluent).

## Per-toolkit native mapping

| Toolkit | Dialog / sheet | Prompt |
|---|---|---|
| appkit | `NSAlert` + `beginSheetModalForWindow:` (async) | `NSAlert` with an `NSTextField` accessory |
| uikit | `UIAlertController` (`.alert` / `.actionSheet`) on the root VC | `UIAlertController` + `addTextField` |
| gtk | `AdwAlertDialog` (libadwaita 1.5; `response` signal) | `AdwAlertDialog` with a `GtkEntry` extra-child |
| qt | `QMessageBox.open()` + `finished` (shim) | `QInputDialog.getText` (shim) |
| android | `MaterialAlertDialogBuilder` (buttons / `setItems` for sheets) | M3 dialog + `TextInputLayout` |
| mock | records the spec; resolved programmatically | same |
| xaml | `ContentDialog` (unverified, no local Windows) | `ContentDialog` + `TextBox` |

All backends use the non-blocking async APIs (sheets / `open()` / callbacks), so the
main loop keeps running and dayscript stays live while a modal is up.

## Dayscript presentation modes

`day launch --script flow.yaml` defaults to **scripted** Day presentations. Alerts,
confirmations, prompts, action sheets and open/save file pickers enter the same pending
request registry and await the script's explicit answer, but no native dialog is opened.
This avoids stranded system picker windows and exercises the app's real continuation
(including file reads/writes). It does not test the native dialog's appearance, focus,
permissions, filters or document-provider integration.

```yaml
flow:
  - tap: { id: btn-open-file }
  - assert_presented: {}
  - respond: { dismiss: true }
  - assert_text: { id: files-status, text: "open-cancel" }
  - assert_not_presented:
```

Use `respond: { path: "fixture.txt" }` to choose a file, `respond: { text: "Ada" }`
for a prompt, or `respond: { button: 1 }` for a dialog button. Relative paths resolve
under the app's writable temp directory. Requests are never automatically accepted or
cancelled: omitting `respond` leaves the app's future pending.

For native integration tests, launch with `--dialogs native`. The equivalent workflow
input is `launch-env: DAY_TEST_DIALOGS=native`. The CLI flag takes precedence over an
explicit environment value; otherwise script launches supply `DAY_TEST_DIALOGS=scripted`.
Ordinary interactive launches remain native, even though their dayscript engine is enabled.
This policy is independent of `--fast` and requires rebuilding the app with mode support;
older apps ignore the environment setting.

A script or `day drive` can change the policy before opening a dialog:

```yaml
  - dialog_mode: { mode: scripted }
  - tap: { id: btn-prompt }
  - assert_presented: {}
  - respond: { text: "Ada" }
  - assert_not_presented:
  - dialog_mode: { mode: native }
```

The step overrides the launch policy for subsequent presentations. Switching modes with
an unanswered request fails: respond to it first. The policy lasts for the app process or
until another `dialog_mode` step; finish with `native` if leaving the app interactive via
`--keep-alive`. Browser hosts initialize the same policy through their host environment.
`assert_presented` and `assert_not_presented` inspect Day's **request registry**, not OS
windows; they do not establish that a native dialog is visible or has physically closed.

Native dismissal remains toolkit-dependent. Android finishes its document-picker activity;
GTK deliberately avoids cancelling already-shown file pickers because that cancellation
reproduced a GTK crash, and Harmony's document-picker bridge currently has no dismissal
hook. Scripted mode prevents those native windows from opening at all. Native-mode tests
on these paths still need user or platform automation for dismissal.

OS permission prompts, authentication UI, external applications, and dialogs opened
directly by third-party code do not use Day's presentation registry and are unaffected.
Use explicit platform permission setup or platform UI automation for those tests.

## The four pillars

- **dayscript**: presentations flow through the registry as a `req`-tagged spec, so a
  script can inspect the pending modal (`assert_presented`) and answer it
  (`- respond: { button: 1 }` / `{ text: "Ada" }` / `{ dismiss: true }`), which
  resolves the future and, in native mode, asks the toolkit to dismiss the control. This makes modal flows
  headless-testable and screenshot-able.
- **a11y**: native controllers are accessible for free.
- **Fluent**: spec fields are `IntoText`.
- **polyglot**: `PresentSpec`/results are an open per-kind set; a third-party crate can
  add a platform picker (contacts, photos, files, share) the same way
  `day-piece-combobox` adds a widget: a new spec variant + `Cap` + backend arms, with no
  day-core edits.

## Deferred

- **Native integration pickers** (contacts / photos / share): same present→result model with
  richer result payloads and `Cap`-gated fallbacks; designed here, implemented after the dialog
  family lands. (File open/save pickers have landed; see [files.md](./files.md).)
- **New windows**: shipped since ([docs/windows.md](windows.md)), and the sketch above aged: the
  implementation kept one tree with multiple adopted roots (no tree-per-window refactor),
  and non-desktop backends degrade to a fullscreen cover instead of missing out. Dialogs
  now attach to the key window at present time.
- **Task/scope binding**: v1 tasks run at the root scope; cancelling an in-flight dialog
  when its owning subtree is disposed is a later refinement (signal writes to disposed
  scopes are already no-ops, so it's safe meanwhile).
