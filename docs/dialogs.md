---
navTitle: "Dialogs"
title: "Dialogs and prompts"
description: "Imperative presentation via present(): alerts, confirmations, sheets, and how dismissal returns a value."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Dialogs and prompts

A dialog asks for a decision or a short piece of information before an app continues a task. It is
useful when the answer affects what happens next, such as confirming a deletion or naming a new
item.

An action opens the dialog and awaits the result; the app can then continue or cancel its work.

```rust
button(res::str::delete()).action(move || {
    day::task(async move {
        let choice = Alert::new(res::str::delete_title())
            .message(res::str::delete_body())
            .destructive(res::str::delete(), Choice::Delete) // a button carrying a payload
            .cancel(res::str::cancel()) // dismissal → None
            .present()
            .await; // -> Option<Choice>
        if choice == Some(Choice::Delete) {
            store.delete(id);
        }
    });
});
```

Use [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) to await a dialog from a synchronous action.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Alerts, confirmations, and prompts

```rust
alert(res::str::saved()).present();                    // fire-and-forget notice (1 button)
let ok: bool         = confirm(res::str::quit()).await;
let name: Option<String> = prompt(res::str::name()).await; // -> Option<String>

// full builder: buttons carry a typed payload; `.cancel()` and dismissal → None
let picked: Option<Flavor> = Alert::new(res::str::pick())
    .button(res::str::vanilla(), Flavor::Vanilla)
    .button(res::str::pistachio(), Flavor::Pistachio)
    .sheet()                                       // bottom action sheet on mobile
    .cancel(res::str::cancel())
    .present().await;
```

Every text field is an `IntoText`, so titles and buttons accept generated `res::str` messages.

## Toolkit implementations

Platform guidance: [Apple: alerts](https://developer.apple.com/design/human-interface-guidelines/alerts); [Android: dialogs](https://developer.android.com/develop/ui/views/components/dialogs); [Windows: dialogs](https://learn.microsoft.com/en-us/windows/apps/design/controls/dialogs-and-flyouts/dialogs).

| Toolkit | Dialog / sheet | Prompt |
|---|---|---|
| appkit | [`NSAlert`](https://developer.apple.com/documentation/appkit/nsalert) + `beginSheetModalForWindow:` (async) | [`NSAlert`](https://developer.apple.com/documentation/appkit/nsalert) with an [`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) accessory |
| uikit | [`UIAlertController`](https://developer.apple.com/documentation/uikit/uialertcontroller) (`.alert` / `.actionSheet`) on the root VC | [`UIAlertController`](https://developer.apple.com/documentation/uikit/uialertcontroller) + `addTextField` |
| gtk | [`AdwAlertDialog`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.AlertDialog.html) (libadwaita 1.5; `response` signal) | [`AdwAlertDialog`](https://gnome.pages.gitlab.gnome.org/libadwaita/doc/main/class.AlertDialog.html) with a [`GtkEntry`](https://docs.gtk.org/gtk4/class.Entry.html) extra-child |
| qt | [`QMessageBox.open()`](https://doc.qt.io/qt-6/qmessagebox.html) + `finished` (shim) | [`QInputDialog.getText`](https://doc.qt.io/qt-6/qinputdialog.html) (shim) |
| android | `MaterialAlertDialogBuilder` (buttons / `setItems` for sheets) | M3 dialog + [`TextInputLayout`](https://developer.android.com/reference/com/google/android/material/textfield/TextInputLayout) |
| arkui | ArkUI `PromptAction.openCustomDialog` modal with themed native `Text` / `Button` content; sheets align to the bottom | Same modal host with native `TextInput`, initial value, placeholder and IME submit |
| mock | records the spec; resolved programmatically | same |
| WinUI | [`ContentDialog`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.contentdialog) | [`ContentDialog`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.contentdialog) + [`TextBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.textbox) |

The main loop keeps running while a dialog is open.

### ArkUI

Dialogs use the primary window. Alerts support multiple buttons, destructive choices,
scrolling content, and Back or outside dismissal. Action-sheet choices are vertical.
A prompt returns `Some("")` for an accepted empty value and `None` for cancellation.
The answer is delivered after the native dialog closes, so the continuation can open another.

## Lifetime

Dialogs attach to the active window where supported. On ArkUI they use the primary window.
Do not assume that disposing the initiating UI subtree cancels an outstanding dialog task.
Check the app state again before applying its answer. See [Async work](/docs/explanation/async)
for task ownership and cancellation.

Use [file pickers](/docs/reference/files) for open/save operations and
[windows](/docs/reference/windows) for additional app windows.
