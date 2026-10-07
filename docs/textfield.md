---
title: "Text field"
description: "The single-line text input: password entry, read-only, keyboard and autofill purpose, the action key, and a length limit."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Text field (built-in)

> **Status: implemented** as a built-in piece (`kinds::TEXT_FIELD`). A native single-line input
> bound two-way to a string. `text_field`, `secure_field`, `InputPurpose` and `SubmitLabel` are in
> `day::prelude::*`.

## Authoring

```rust
use day::prelude::*;

let email = Signal::new(String::new());
let password = Signal::new(String::new());

form((
    labeled("Email", text_field(email)
        .input_purpose(InputPurpose::Email)
        .submit_label(SubmitLabel::Next)),
    labeled("Password", secure_field(password)
        .submit_label(SubmitLabel::Go)
        .on_submit(sign_in)),
))
```

`text_field(text)` binds a `Signal<String>` (or any `Binding<String>`) two-way: typing writes the
signal, and setting the signal replaces the field's text. `.placeholder(_)` sets the empty-state
prompt and `.on_submit(_)` fires on Return or the keyboard's action key. Focus is the
`.focused(_)` decorator ([docs/focus.md](focus.md)). `.enabled(_)`, a constant or a reactive
`bool`, turns input off altogether: a disabled field neither edits nor takes focus, where
`.read_only(_)` keeps it focusable and selectable.

## Password fields

`secure_field(text)` is a `text_field` that hides its characters and tells the platform it holds
a password, so the password manager offers to fill it. It is the same piece, and every builder
below applies to it.

`.secure(_)` takes a constant, a signal or a closure. Bound to a signal it makes a show-password
switch:

```rust
let password = Signal::new(String::new());
let shown = Signal::new(false);

section((
    labeled("Password", secure_field(password).secure(move || !shown.get())),
    labeled("Show Password", toggle(shown)),
))
```

The text, the placeholder, the enabled state and the focus carry across the change on every
platform. A sign-up form marks its field `.input_purpose(InputPurpose::NewPassword)` so the
password manager offers to make and save one.

## Entry traits

| builder | what it sets | reactive |
|---|---|---|
| `.secure(v)` | hide the characters | yes |
| `.read_only(v)` | show and select the text, take no edits; the field keeps its ordinary look and still takes focus | yes |
| `.input_purpose(p)` | what the field collects: the on-screen keyboard, capitalization and correction, and what the system offers to fill in | no |
| `.submit_label(l)` | what the on-screen keyboard's action key says: `Return`, `Done`, `Go`, `Next`, `Search`, `Send` | no |
| `.max_length(n)` | the most characters the field takes | no |

`InputPurpose` is one of `Text` (the default), `Name`, `Email`, `Url`, `Phone`, `Number`,
`Decimal`, `Username`, `Password`, `NewPassword` and `OneTimeCode`. Purpose and `secure` are
independent, so a numeric PIN is both:

```rust
secure_field(pin).input_purpose(InputPurpose::Number).max_length(4)
```

`OneTimeCode` keeps the text keyboard, since a code can carry letters; the keyboard offers the
arriving code either way.

`.max_length(n)` counts characters (Unicode scalar values). Typing or pasting past the bound is
cut to fit before the bound value sees it, and the cut text is written back to the field. Day
holds the bound itself so it counts the same characters on every platform; GTK's native limit
counts the same unit and is applied as well.

## Per-backend native realization

| | AppKit | UIKit | GTK | Qt | Android | XAML | ArkUI | web |
|---|---|---|---|---|---|---|---|---|
| field | `NSTextField` | `UITextField` | `GtkEntry` | `QLineEdit` | `TextInputEditText` | `TextBox` | `ARKUI_NODE_TEXT_INPUT` | `<input>` |
| secure | `NSSecureTextField` | `secureTextEntry` | `visibility` | `EchoMode::Password` | password input type and transformation | `PasswordBox` | password input type | `type="password"` |
| read-only | `editable` off | delegate refuses edits, empty input view | `editable` off | `readOnly` | no key listener, text selectable | `IsReadOnly` | refuses inserts and deletes | `readonly` |
| purpose | content type | keyboard, content type, capitalization, correction | input purpose and hints | input method hints | input type, autofill hints | `InputScope`, spell check, prediction | input type, content type | `inputmode`, `autocomplete`, `autocapitalize`, `spellcheck` |
| action key | – | `returnKeyType` | – | – | IME action | – | enter key type | `enterkeyhint` |

A dash means the toolkit has no such notion: a desktop keyboard's Return key carries no label.

On AppKit and WinUI the secure field is a different native class from the plain one, so a change
of `secure` rebuilds the widget. `Toolkit::set_input_traits` returns the replacement and day-core
points the node at it, the same contract `set_selectable` uses for a UIKit label that becomes
selectable ([docs/text.md](text.md)). The backend builds the replacement through the code that
`realize` runs, hands it the text, placeholder, enabled state, frame, accessibility identity and
focus, and puts it in the old widget's place among its siblings. A `.tweak` applied to the field
before the change does not carry over, and day-core logs a warning when that happens
([docs/tweaks.md](tweaks.md)).

Details that differ by platform:

- **AppKit** resolves the content types at run time, because most arrived in macOS 14. On
  macOS 13 the username, password and one-time-code types apply and the others are skipped.
- **UIKit** re-enters the text when `secure` changes on a focused field, which keeps the caret
  and stops the next keystroke from clearing a field that has just turned secure.
- **Android** masks with an explicit transformation method as well as the input type, so a phone
  or decimal field can be secure too.
- **WinUI**'s `PasswordBox` has no read-only property. A read-only secure field leaves hit
  testing and the tab order instead: it keeps its look and takes no edits.
- **ArkUI** (API 18) has no URL input type, so `Url` takes the text keyboard. The built-in
  show-password icon is turned off, so `secure` is the one switch.
- **web** keeps `type="text"` for every purpose and picks the keyboard with `inputmode`. The typed
  inputs (`email`, `url`, `number`) trim or coerce what the user types, and a controlled input
  has to hold exactly the text it reports.

## The duty

```rust
fn set_input_traits(&mut self, h: &Self::Handle, traits: &InputTraits) -> Option<Self::Handle>;
```

`InputTraits` carries all five members. day-pieces calls the duty once after `realize` when a
field asks for anything but the defaults, and again with the whole set whenever `secure` or
`read_only` changes, so a backend applies every member on every call. A field that asks for
nothing never calls it. The duty is required: a backend has to say how it hides a password.

## Verification

- `crates/day-pieces/tests/mock_e2e.rs`: `plain_text_field_sends_no_input_traits`,
  `secure_field_carries_its_traits_and_follows_a_reactive_flip`,
  `secure_flip_on_a_class_swap_toolkit_repoints_the_node` (the mock rebuilds the widget under a
  new handle, as AppKit and WinUI do) and
  `max_length_cuts_over_long_input_and_paints_the_cut_back`.
- Day-Showcase's Text fields page (`src/pages/text_fields.rs`) and `dayscript/textfields.yaml`:
  a password with its Show Password switch, one field per purpose, a secure four-digit PIN, a
  read-only field and a length limit. The script types after every change of `secure` and
  asserts the field still holds focus.
