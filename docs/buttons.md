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

```rust
button("Save").action(save)
button("Save").icon(Symbol::Save).prominent().action(save)
button("Play").icon(Symbol::Play).icon_only().action(play)
button("About").image(res::vectors::app_mark).action(about)
button("Send").enabled(move || !busy.get()).action(send)
```

For an operation shared across surfaces, use a [reusable `Command`](commands.md) to define its
title, availability, optional check state, icon, shortcut, and handler once.

## Icons and labels

`.icon(symbol)` adds a platform symbol beside the title. It accepts a `Symbol`, signal, or
closure. `.image(name)` uses a bundled image or vector; pass a generated `res::images` or
`res::vectors` constant. The last `.icon()` or `.image()` call determines the image.
Use a small template glyph with a transparent background, as you would for a toolbar icon.
Resource staging uses the existing [image and vector pipeline](vectors.md).

`.icon_only()` hides the visible title when an icon is available. Keep a meaningful title:
it remains the accessible name and, on desktop and web, the automatic tooltip. Buttons
with a visible title do not receive an automatic tooltip. With no icon, the title stays
visible. Native loaders that cannot resolve an icon also fall back to the title; web image
loading is asynchronous, so a missing asset can leave the icon blank.

Keep the title and symbol in sync for actions that change:

```rust
button(move || if playing.get() { "Pause" } else { "Play" })
    .icon(move || if playing.get() { Symbol::Pause } else { Symbol::Play })
    .icon_only()
    .action(move || playing.set(!playing.get()))
```

These updates preserve the button instance. The icon builders also work after decorations,
for example `button("Play").padding(4.0).icon(Symbol::Play)`.

## Styles

| Modifier | Behavior |
| --- | --- |
| *(none)* | The platform's ordinary button |
| `.bordered()` | A contained button where the stock style is borderless, notably UIKit |
| `.prominent()` | The platform's primary or default-action treatment |
| `.tint(color)` | A filled button with a contrasting foreground |
| `.compact()` | Removes extra minimum width and horizontal padding on Android and web |
| `.enabled(value)` | Enables or disables interaction; accepts reactive values |

`.tint()` takes precedence over the other styles and can follow a signal. Day chooses black
or white foreground according to which has the higher WCAG contrast ratio against the fill.
Disabled buttons reject actions even when a dayscript injects a press.

```rust
button("Record")
    .icon(Symbol::Play)
    .tint(move || if recording.get() { RUST } else { SLATE })
```

## Platform implementation

| Toolkit | Button and icon implementation |
| --- | --- |
| AppKit | `NSButton` image and image-position properties; SF Symbols or bundled template images |
| UIKit | `UIButtonConfiguration` image, title, and spacing; SF Symbols or bundled template images sized to 20 points |
| Android | `MaterialButton` icon, gravity, padding, and content description; packaged drawables |
| GTK | `GtkButton` containing an image and optional label; icon-theme symbols with Day outlines as fallback |
| Qt | `QPushButton` with a `QIcon`; theme icons, standard icons, Day outlines, or bundled files |
| XAML | `Button` content containing an icon and optional text; symbol glyphs, vector paths, or bundled images |
| ArkUI | A native button containing an image and optional text; Day symbol outlines or packaged SVG/PNG assets |
| web-dom | A native `<button>` with a CSS image mask using the current text color |

The button owns interaction in every case; the icon is content, not a separate action.
Symbols may look different between platforms. Bundled template images use the existing
[toolbar icon loaders](toolbars.md), including their platform-specific tint behavior.

AppKit's `.prominent()` establishes the Return-key default action. GTK uses
`suggested-action`, UIKit uses its bordered-prominent configuration, and XAML requests
`AccentButtonStyle` where available. Qt asks the current style to draw a default button;
Android and ArkUI retain their stock filled treatment.

Tinting preserves native controls but can affect their feedback. Qt uses explicit hover,
pressed, and disabled stylesheet rules. XAML's local background brush reduces the template's
usual pressed-color change. AppKit applies label contrast through an attributed title and
icon contrast through `contentTintColor`; UIKit uses configuration foreground and background
colors.

## Backend contract

`ButtonProps` seeds the title, optional `Icon`, `icon_only`, enabled state, and style. Plain
buttons continue to send `ButtonPatch::Title`. Icon buttons send `ButtonPatch::Content`,
which updates their title and icon together and invalidates their measured size. The
reactive binding uses `bind_seeded`, avoiding a duplicate update immediately after creation.

`Enabled` and `Style` patches remain independent of content. A backend must preserve the
icon and accessible title when a style changes. UIKit retains content in a side table because
replacing a configuration also replaces its image. ArkUI releases its internal image and
text nodes when content changes or the button is released.

External toolkit implementations can keep their existing plain-button title handling.
To support icons, read the new props and handle `Content`, retaining its title as the
accessible name even when it is not drawn.
