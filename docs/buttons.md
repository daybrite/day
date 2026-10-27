---
title: "Buttons"
description: "Native buttons with labels, icons, reactive state, and platform styles."
---

<!-- Copyright © The Daybrite Project
     SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Buttons

A Day button uses the platform's button control, including its keyboard handling, focus
ring, accessibility role, and pressed and disabled states. Give it a title describing the
action, then add an icon or style when that helps the user recognize it.

## Add to your app

This control is included in [`day-pieces`](https://github.com/daybrite/day/tree/main/crates/day-pieces)
and re-exported through `day::prelude::*`. No additional crate or control feature is required.

## Authoring

```rust
button(res::str::save()).action(save)
button(res::str::save()).icon(Symbol::Save).prominent().action(save)
button(res::str::play()).icon(Symbol::Play).icon_only().action(play)
button(res::str::about()).image(res::vectors::app_mark).action(about)
button(res::str::send()).enabled(move || !busy.get()).action(send)
```

Use [`.action(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.action) for button activation, including after decorations such as [`.padding(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding).
The generic [`.on_tap(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap) decorator installs a separate gesture recognizer; it is not an alias
for the native button action. On AppKit that recognizer delays primary mouse events and can
delay or suppress the pressed highlight. UIKit also installs a separate tap recognizer.
Keeping activation on [`.action(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.action) preserves the control's normal pointer, keyboard,
accessibility, and disabled behavior across toolkits.
Day logs a runtime warning when a tap recognizer is attached to a native button, including
through layout decorators. This also covers [`.on_tap_at(...)`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.on_tap_at). Existing callbacks continue
to work, so the diagnostic does not silently change gesture semantics.

For an operation shared across surfaces, use a [reusable `Command`](commands.md) to define its
title, availability, optional check state, icon, shortcut, and handler once.

The UI snippets use generated `res::str` accessors from an app's Fluent catalog; see the
[example messages](/docs/reference/localization#reference-example-messages) and [catalog setup](/docs/how-to/localization).

## Icons and labels

[`.icon(symbol)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.icon) adds a platform symbol beside the title. It accepts a `Symbol`, signal, or
closure. [`.image(name)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.image) uses a bundled image or vector; pass a generated `res::images` or
`res::vectors` constant. The last [`.icon()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.icon) or [`.image()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.image) call determines the image.
Use a small template glyph with a transparent background, as you would for a toolbar icon.
Resource staging uses the existing [image and vector pipeline](vectors.md).

[`.icon_only()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.icon_only) hides the visible title when an icon is available. Keep a meaningful title:
it remains the accessible name and, on desktop and web, the automatic tooltip. Buttons
with a visible title do not receive an automatic tooltip. With no icon, the title stays
visible. Native loaders that cannot resolve an icon also fall back to the title; web image
loading is asynchronous, so a missing asset can leave the icon blank.

Keep the title and symbol in sync for actions that change:

```rust
button(move || {
    if playing.get() {
        res::str::pause()
    } else {
        res::str::play()
    }
})
.icon(move || {
    if playing.get() {
        Symbol::Pause
    } else {
        Symbol::Play
    }
})
.icon_only()
.action(move || playing.set(!playing.get()))
```

These updates preserve the button instance. The icon builders also work after decorations,
for example [`button`](https://docs.rs/day-pieces/latest/day_pieces/fn.button.html)`(res::str::play())`[`.padding`](https://docs.rs/day-pieces/latest/day_pieces/trait.Decorate.html#method.padding)`(4.0)`[`.icon`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.icon)`(Symbol::Play)`.

## Styles

| Modifier | Behavior |
| --- | --- |
| *(none)* | The platform's ordinary button |
| [`.bordered()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.bordered) | A contained button where the stock style is borderless, notably UIKit |
| [`.prominent()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.prominent) | The platform's primary or default-action treatment |
| [`.tint(color)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.tint) | A filled button with a contrasting foreground |
| [`.compact()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.compact) | Removes extra minimum width and horizontal padding on Android and web |
| [`.enabled(value)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.enabled) | Enables or disables interaction; accepts reactive values |
| [`.selected(value)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.selected) | Marks the button as the current choice among its peers; accepts reactive values |

[`.tint()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.tint) takes precedence over the other styles and can follow a signal. Day chooses black
or white foreground according to which has the higher WCAG contrast ratio against the fill.
Disabled buttons reject actions even when a dayscript injects a press.

```rust
button(res::str::record())
    .icon(Symbol::Play)
    .tint(move || if recording.get() { RUST } else { SLATE })
```

## Selected state

[`.selected(value)`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.selected) marks a button as the current choice among its peers: a document tab, one
view of a switcher. The platform draws its own checked or selected look and tells assistive
technology the button is selected, while the button stays enabled, focusable and activatable.
A disabled stand-in would do neither. A press never toggles the state natively: the app's
handler decides, and the next value of the closure is what the control shows.

```rust
button(res::str::drafts())
    .selected(move || current.get() == Section::Drafts)
    .action(move || current.set(Section::Drafts))
```

| Toolkit | Selected rendering |
| --- | --- |
| AppKit | The [`NSButton`](https://developer.apple.com/documentation/appkit/nsbutton) becomes push-on/push-off on its first selection; the on state is the bezel's highlighted fill |
| UIKit | `isSelected`, drawn with the system's tinted configuration; deselecting restores the button's style |
| Android | A checkable [`MaterialButton`](https://developer.android.com/reference/com/google/android/material/button/MaterialButton) in its checked state, plus the view's selected state for TalkBack |
| GTK | The `:checked` state flag on the plain [`GtkButton`](https://docs.gtk.org/gtk4/class.Button.html), which the theme draws as a toggle's checked look |
| Qt | A checkable [`QPushButton`](https://doc.qt.io/qt-6/qpushbutton.html) in its checked state |
| WinUI | The theme's accent fill on the [`Button`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.button); automation carries no selected state |
| ArkUI | The prominent fill over the button's own style; the bound NDK has no selected state attribute |
| Web | `aria-pressed` and the `selected` class, drawn as the accent fill |


## Toolkit implementations

Platform guidance: [Apple: buttons](https://developer.apple.com/design/human-interface-guidelines/buttons); [Android: buttons](https://developer.android.com/develop/ui/views/components/button).

| Toolkit | Button and icon implementation |
| --- | --- |
| AppKit | [`NSButton`](https://developer.apple.com/documentation/appkit/nsbutton) image and image-position properties; SF Symbols or bundled template images |
| UIKit | [`UIButtonConfiguration`](https://developer.apple.com/documentation/uikit/uibuttonconfiguration) image, title, and spacing; SF Symbols or bundled template images sized to 20 points |
| Android | [`MaterialButton`](https://developer.android.com/reference/com/google/android/material/button/MaterialButton) icon, gravity, padding, and content description; packaged drawables |
| GTK | [`GtkButton`](https://docs.gtk.org/gtk4/class.Button.html) containing an image and optional label; icon-theme symbols with Day outlines as fallback |
| Qt | [`QPushButton`](https://doc.qt.io/qt-6/qpushbutton.html) with a [`QIcon`](https://doc.qt.io/qt-6/qicon.html); theme icons, standard icons, Day outlines, or bundled files |
| WinUI | [`Button`](https://learn.microsoft.com/en-us/windows/windows-app-sdk/api/winrt/microsoft.ui.xaml.controls.button) content containing an icon and optional text; symbol glyphs, vector paths, or bundled images |
| ArkUI | A native button containing an image and optional text; Day symbol outlines or packaged SVG/PNG assets |
| Web | A native `<button>` with a CSS image mask using the current text color |

The button owns interaction in every case; the icon is content, not a separate action.
Symbols may look different between platforms. Bundled template images use the existing
[toolbar icon loaders](toolbars.md), including their platform-specific tint behavior.

AppKit's [`.prominent()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.prominent) establishes the Return-key default action. GTK uses
`suggested-action`, UIKit uses its bordered-prominent configuration, and WinUI requests
`AccentButtonStyle` where available. Qt asks the current style to draw a default button;
Android retains its stock filled treatment. ArkUI uses a transparent background for automatic
and compact buttons, an outline for bordered buttons, and a filled background for prominent
and tinted buttons. All remain native buttons with native focus, press, and disabled behavior.

Tinting preserves native controls but can affect their feedback. Qt uses explicit hover,
pressed, and disabled stylesheet rules. WinUI's local background brush reduces the template's
usual pressed-color change. AppKit applies label contrast through an attributed title and
icon contrast through `contentTintColor`; UIKit uses configuration foreground and background
colors.

## Plain buttons

[`.plain()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.plain) requests a borderless native action button for toolbar commands and list
accessories. AppKit removes the [NSButton](https://developer.apple.com/documentation/appkit/nsbutton) bezel; UIKit uses its system button; Android
uses Material's theme-resolved text-button colors with compact horizontal insets;
GTK applies `flat`. Native press, focus, accessibility and disabled behavior remain.
Qt, WinUI and web currently use their automatic style as a fallback; ArkUI's automatic
style is already transparent. [`.plain()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.plain) and [`.compact()`](https://docs.rs/day-pieces/latest/day_pieces/trait.ButtonBuilder.html#tymethod.compact) are mutually exclusive style
requests: the last one wins. Android plain already includes compact horizontal sizing.
Switching away from a tinted or plain style restores the original Material foreground
and icon state colors as well as its background.
