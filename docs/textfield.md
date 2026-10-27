---
navTitle: "Text fields"
title: "Text field"
description: "The single-line text input: password entry, read-only, keyboard and autofill purpose, the action key, and a length limit."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Text field

Collect a single line of text, such as a name, email address, or password. [`text_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_field.html) and
[`secure_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.secure_field.html) bind edits to app state and let you choose the keyboard purpose and submit action.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

| Platforms / toolkits | Availability |
| --- | --- |
| AppKit, UIKit, GTK, Qt, Android, WinUI, ArkUI, Web | Built-in control; see [toolkit implementations](#toolkit-implementations) for presentation differences. |

## Authoring

```rust
use day::prelude::*;

let email = Signal::new(String::new());
let password = Signal::new(String::new());

form((
    labeled(
        res::str::email(),
        text_field(email)
            .input_purpose(InputPurpose::Email)
            .submit_label(SubmitLabel::Next),
    ),
    labeled(
        res::str::password(),
        secure_field(password)
            .submit_label(SubmitLabel::Go)
            .on_submit(sign_in),
    ),
))
```

[`text_field(text)`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_field.html) binds a `Signal<String>` (or any `Binding<String>`) two-way: typing writes the
signal, and setting the signal replaces the field's text. [`.placeholder(_)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.placeholder) sets the empty-state
prompt and [`.on_submit(_)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.on_submit) fires on Return or the keyboard's action key. Focus is the
[`.focused(_)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.focused) decorator ([docs/focus](focus.md)). [`.enabled(_)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.enabled), a constant or a reactive
`bool`, turns input off altogether: a disabled field neither edits nor takes focus, where
[`.read_only(_)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.read_only) keeps it focusable and selectable.

## Password fields

[`secure_field(text)`](https://docs.rs/day-pieces/latest/day_pieces/fn.secure_field.html) is a [`text_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_field.html) that hides its characters and tells the platform it holds
a password, so the password manager offers to fill it. It is the same piece, and every builder
below applies to it.

[`.secure(_)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.secure) takes a constant, a signal or a closure. Bound to a signal it makes a show-password
switch:

```rust
let password = Signal::new(String::new());
let shown = Signal::new(false);

section((
    labeled(
        res::str::password(),
        secure_field(password).secure(move || !shown.get()),
    ),
    labeled(res::str::show_password(), toggle(shown)),
))
```

The text, the placeholder, the enabled state and the focus carry across the change on every
platform. A sign-up form marks its field [`.input_purpose(InputPurpose::NewPassword)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.input_purpose) so the
password manager offers to make and save one.

## Entry traits

| builder | what it sets | reactive |
|---|---|---|
| [`.secure(v)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.secure) | hide the characters | yes |
| [`.read_only(v)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.read_only) | show and select the text, take no edits; the field keeps its ordinary look and still takes focus | yes |
| [`.input_purpose(p)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.input_purpose) | what the field collects: the on-screen keyboard, capitalization and correction, and what the system offers to fill in | no |
| [`.submit_label(l)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.submit_label) | what the on-screen keyboard's action key says: `Return`, `Done`, `Go`, `Next`, `Search`, `Send` | no |
| [`.max_length(n)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.max_length) | the most characters the field takes | no |

`InputPurpose` is one of `Text` (the default), `Name`, `Email`, `Url`, `Phone`, `Number`,
`Decimal`, `Username`, `Password`, `NewPassword` and `OneTimeCode`. Purpose and [`secure`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.secure) are
independent, so a numeric PIN is both:

```rust
secure_field(pin).input_purpose(InputPurpose::Number).max_length(4)
```

`OneTimeCode` keeps the text keyboard, since a code can carry letters; the keyboard offers the
arriving code either way.

[`.max_length(n)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.max_length) counts characters (Unicode scalar values). Typing or pasting past the bound is
cut to fit before the bound value sees it, and the cut text is written back to the field. Day
holds the bound itself so it counts the same characters on every platform; GTK's native limit
counts the same unit and is applied as well.

## Toolkit implementations

Platform guidance: [Apple: text fields](https://developer.apple.com/design/human-interface-guidelines/text-fields); [Android: input methods](https://developer.android.com/develop/ui/views/touch-and-input/creating-input-method); [Windows: text controls](https://learn.microsoft.com/en-us/windows/apps/design/controls/text-controls).

| | AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI | web |
|---|---|---|---|---|---|---|---|---|
| field | [`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) | [`UITextField`](https://developer.apple.com/documentation/uikit/uitextfield) | [`GtkEntry`](https://docs.gtk.org/gtk4/class.Entry.html) | [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html) | [`TextInputEditText`](https://developer.android.com/reference/com/google/android/material/textfield/TextInputEditText) | [`TextBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.textbox) | [`ARKUI_NODE_TEXT_INPUT`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) | `<input>` |
| secure | [`NSSecureTextField`](https://developer.apple.com/documentation/appkit/nssecuretextfield) | `secureTextEntry` | `visibility` | `EchoMode::Password` | password input type and transformation | `PasswordBox` | password input type | `type="password"` |
| read-only | `editable` off | delegate refuses edits, empty input view | `editable` off | `readOnly` | no key listener, text selectable | `IsReadOnly` | refuses inserts and deletes | `readonly` |
| purpose | content type | keyboard, content type, capitalization, correction | input purpose and hints | input method hints | input type, autofill hints | `InputScope`, spell check, prediction | input type, content type | `inputmode`, `autocomplete`, `autocapitalize`, `spellcheck` |
| action key | – | `returnKeyType` | – | – | IME action | – | enter key type | `enterkeyhint` |

A dash means the toolkit has no such notion: a desktop keyboard's Return key carries no label.

On AppKit and WinUI the secure field is a different native class from the plain one, so a change
of [`secure`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.secure) rebuilds the widget. [`Toolkit::set_input_traits`](https://docs.rs/day-spec/latest/day_spec/trait.Toolkit.html#tymethod.set_input_traits) returns the replacement and day-core
points the node at it, the same contract `set_selectable` uses for a UIKit label that becomes
selectable ([docs/text](text.md)). The toolkit builds the replacement through the code that
`realize` runs, hands it the text, placeholder, enabled state, frame, accessibility identity and
focus, and puts it in the old widget's place among its siblings. A [`.tweak`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.tweak) applied to the field
before the change does not carry over, and day-core logs a warning when that happens
([docs/tweaks](tweaks.md)).

Details that differ by platform:

- **AppKit** resolves the content types at run time, because most arrived in macOS 14. On
  macOS 13 the username, password and one-time-code types apply and the others are skipped.
- **UIKit** re-enters the text when [`secure`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.secure) changes on a focused field, which keeps the caret
  and stops the next keystroke from clearing a field that has just turned secure.
- **Android** masks with an explicit transformation method as well as the input type, so a phone
  or decimal field can be secure too.
- **WinUI**'s `PasswordBox` has no read-only property. A read-only secure field leaves hit
  testing and the tab order instead: it keeps its look and takes no edits.
- **ArkUI** (API 18) has no URL input type, so `Url` takes the text keyboard. The built-in
  show-password icon is turned off, so [`secure`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.secure) is the one switch.
- **web** keeps `type="text"` for every purpose and picks the keyboard with `inputmode`. The typed
  inputs (`email`, `url`, `number`) trim or coerce what the user types, and a controlled input
  has to hold exactly the text it reports.
