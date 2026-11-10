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

A text field collects a short value such as a name, address, or search term. Its label and input
behavior help people understand what to enter and when the entry is complete.

[`text_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.text_field.html) and
[`secure_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.secure_field.html) collect a single
line of text, bind edits to app state, and let the app choose the keyboard purpose and submit
action.

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

## Selection

[`.select(trigger, range)`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.select) selects part of the field's text, or places the caret, each time
`trigger` fires. `range` is a closure Day calls once the turn settles, and it returns a range of
characters (Unicode scalar values, as [`.max_length`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.max_length) counts them): `..` selects everything,
`3..` from the fourth character to the end, and `3..3` puts the caret after the third. Ends
past the text are held to it. Each backend converts the range to the unit its field counts, so
an emoji or an accented letter is one position on every platform.

An address bar uses it twice. Inline completion writes the completed text and selects the part
the user did not type, so the next keystroke replaces it and Backspace removes it:

```rust
let address = Signal::new(String::new());
let typed = Signal::new(0usize);
let complete = Trigger::new();

watch(
    move || address.get(),
    move |now, before| {
        // Complete only while the text grows, so Backspace can remove the completion.
        let grew = now.chars().count() > before.map_or(0, |b| b.chars().count());
        if let Some(host) = grew.then(|| history.complete(now)).flatten() {
            typed.set(now.chars().count());
            address.set(host);
            complete.notify();
        }
    },
);

text_field(address)
    .input_purpose(InputPurpose::Url)
    .select(complete, move || typed.get_untracked()..)
```

Select-all on focus fires the trigger when the focus binding turns on. A field takes one
[`.select`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.select),
so a page that needs both behaviors keeps the range in a signal and fires one
trigger for both:

```rust
let editing = Signal::new(false);
let range = Signal::new(0..usize::MAX);
let select = Trigger::new();
watch(move || editing.get(), move |&on, _| if on {
    range.set(0..usize::MAX);
    select.notify();
});

text_field(address)
    .focused(editing)
    .select(select, move || range.get_untracked())
```

Writing the bound text while the user types replaces the typing session. Return or focus loss
then commits nothing further, so the typed "git" never lands back over the completed
"github.com", and an Escape handler that puts the page's URL back keeps it as the field loses
focus. The binding's preview session ends there too ([`Binding::end_preview`](https://docs.rs/day-reactive/latest/day_reactive/trait.Binding.html#method.end_preview)): the app's write
recorded its own change, so with a [day-model](model.md) field the next edit undoes back to the
app's text, not to the text from before the typing.

Day reads the range once the turn settles and applies it after the turn's other updates, so
text written in the same handler or effect reaches the field first, whichever the app wrote
first, and so does a focus request. Each firing is a new request, and the same range applies
again on the next one.

A selection is meant for a focused field. AppKit and UIKit keep a field's selection in the
editor they attach while it edits, so a request for an unfocused field does nothing there.
GTK, Qt, Android, WinUI, ArkUI and the web set it on the unfocused field, where the platform's
own focus behavior may then replace it: a keyboard focus-in selects everything on GTK, and a
click places the caret on every desktop. Focus the field first, fire the trigger from the
focus change as above, or write the focus binding and fire the trigger in the same handler, as
a browser's ⌘L does.

WinUI's `PasswordBox` can only select everything, so a [`secure_field`](https://docs.rs/day-pieces/latest/day_pieces/fn.secure_field.html) takes a request for the
whole text and ignores any other. An input method's composition in progress belongs to the
input method; a request that arrives during one may end it or be overridden, depending on the
platform.

## Keys

A field decorated with [`.on_key(f)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_key) hands three keys to `f` while it has focus, instead of
acting on them itself: `ArrowUp`, `ArrowDown` and `Escape`, pressed with no modifier. That is
what an address bar needs to walk a suggestion list drawn below it and to cancel an edit:

```rust
text_field(address)
    .focused(editing)
    .on_submit(go)
    .on_key(move |k| match k.key.as_str() {
        "ArrowDown" => highlight.update(|i| *i = (*i + 1).min(last)),
        "ArrowUp" => highlight.update(|i| *i = i.saturating_sub(1)),
        "Escape" => {
            address.set(current_url.get_untracked());
            editing.set(false);
        }
        _ => {}
    })
```

The field claims the three keys whenever it has a handler, so the native field never sees
them: AppKit's caret stays put on Up, GTK's focus does not move to the next widget on Down, and
an `Escape` neither opens AppKit's completion list nor closes a dialog or a sheet around the
field. Return stays [`on_submit`](https://docs.rs/day-pieces/latest/day_pieces/trait.TextFieldBuilder.html#tymethod.on_submit), Tab moves focus, every other key edits, and a modified arrow
(Shift+Up extends the selection) stays the field's. A field without a handler behaves as it
always did. On AppKit the keys arrive as the field editor's `moveUp:`, `moveDown:` and
`cancelOperation:` commands, so Control-P, Control-N and ⌘. arrive as `ArrowUp`, `ArrowDown`
and `Escape` too, as they do in Safari's address bar.

The desktop toolkits (AppKit, GTK, Qt, WinUI) and the web deliver these keys. UIKit, Android
and ArkUI do not deliver them from a text field.

## Toolkit implementations

Platform guidance: [Apple: text fields](https://developer.apple.com/design/human-interface-guidelines/text-fields); [Android: input methods](https://developer.android.com/develop/ui/views/touch-and-input/creating-input-method); [Windows: text controls](https://learn.microsoft.com/en-us/windows/apps/design/controls/text-controls).

| | AppKit | UIKit | GTK | Qt | Android | WinUI | ArkUI | web |
|---|---|---|---|---|---|---|---|---|
| field | [`NSTextField`](https://developer.apple.com/documentation/appkit/nstextfield) | [`UITextField`](https://developer.apple.com/documentation/uikit/uitextfield) | [`GtkEntry`](https://docs.gtk.org/gtk4/class.Entry.html) | [`QLineEdit`](https://doc.qt.io/qt-6/qlineedit.html) | [`TextInputEditText`](https://developer.android.com/reference/com/google/android/material/textfield/TextInputEditText) | [`TextBox`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.textbox) | [`ARKUI_NODE_TEXT_INPUT`](https://github.com/openharmony/docs/blob/master/en/application-dev/reference/apis-arkui/capi-native-node-h.md) | `<input>` |
| secure | [`NSSecureTextField`](https://developer.apple.com/documentation/appkit/nssecuretextfield) | `secureTextEntry` | `visibility` | `EchoMode::Password` | password input type and transformation | `PasswordBox` | password input type | `type="password"` |
| read-only | `editable` off | delegate refuses edits, empty input view | `editable` off | `readOnly` | no key listener, text selectable | `IsReadOnly` | refuses inserts and deletes | `readonly` |
| purpose | content type | keyboard, content type, capitalization, correction | input purpose and hints | input method hints | input type, autofill hints | `InputScope`, spell check, prediction | input type, content type | `inputmode`, `autocomplete`, `autocapitalize`, `spellcheck` |
| action key | – | `returnKeyType` | – | – | IME action | – | enter key type | `enterkeyhint` |
| selection | field editor `selectedRange` | `selectedTextRange` | `select_region` | `setSelection` | `setSelection` | `Select` | `NODE_TEXT_INPUT_TEXT_SELECTION` | `setSelectionRange` |
| ↑ ↓ Esc | field editor delegate | – | capture-phase key controller | event filter | – | `PreviewKeyDown` | – | `keydown` |

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

## Numeric input

[`number_field(value)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/fn.number_field.html) binds a native
text field to a numeric signal or other `Binding<f64>`. It uses decimal formatting with two
fraction digits by default:

```rust
let value = Signal::new(0.4);
number_field(value)
    .range(0.0..=1.0)
    .format(NumberFormat::percent())
```

Typing preserves incomplete input until Return or focus loss. Committing a valid number writes
the binding once and clamps to the range; rejected input restores the previous formatted value.
Locale and bound-value changes refresh the displayed text. The control supports generic
modifiers such as [`.focused(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.focused), [`.a11y(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.a11y), and [`.min_width(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.min_width).

[`NumberFieldBuilder`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.NumberFieldBuilder.html)
also provides a localized [`.placeholder(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.NumberFieldBuilder.html#tymethod.placeholder),
reactive [`.enabled(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.NumberFieldBuilder.html#tymethod.enabled)
and [`.read_only(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.NumberFieldBuilder.html#tymethod.read_only),
[`.input_purpose(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.NumberFieldBuilder.html#tymethod.input_purpose),
[`.submit_label(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.NumberFieldBuilder.html#tymethod.submit_label),
and [`.on_submit(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.NumberFieldBuilder.html#tymethod.on_submit). The default input purpose is
`InputPurpose::Decimal`. Submission commits the number and restores formatted text before
calling [`.on_submit(...)`](https://docs.rs/day-pieces/latest/day_pieces/stepper/trait.NumberFieldBuilder.html#tymethod.on_submit); invalid input leaves the bound value unchanged. Focus loss commits
without calling the submit callback. Numeric builders work before or after layout modifiers.

See [editable numbers](/docs/reference/localization#editable-numbers) for parsing rules and
[steppers](/docs/reference/stepper#formatted-values) to add increment/decrement buttons.
