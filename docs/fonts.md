---
title: Fonts
---
<!-- Copyright © The Daybrite Project -->
<!-- SPDX-License-Identifier: CC-BY-SA-4.0 -->

# Fonts: the platform list, canvas fonts, and measurement

Labels pick their type through [`FontSpec`](text.md) — semantic styles the reader's text-size
setting scales. A drawing has different needs: it names a family from the machine's own list,
sets an absolute size, and has to know how big the result is before it draws a frame around it.
This is that surface: `day::font_families()`, `CanvasFont` on canvas text, and
`day::measure_text()`.

## The font list

```rust
let families = day::font_families();           // Rc<[FontFamilyInfo]>, sorted by family
for f in families.iter() {
    println!("{} — bold {} italic {}", f.family, f.has_bold(), f.has_italic());
    for face in &f.faces {
        println!("  {} {:?} italic={}", face.name, face.weight, face.italic);
    }
}
```

`FontFamilyInfo { family, faces }` names one family and every face it ships; `FontFace { name,
weight, italic }` is one face, with its display name ("Bold Italic") and its `FontWeight` rung.
`has_bold()` / `has_italic()` answer the style-picker question, and `face_for(weight, italic)`
picks the face that will draw for a request: an exact slant first, then the nearest weight,
ties going heavier (the CSS rule).

The list is enumerated **once per process** and cached: it is stable for the process (bundled
fonts register before the tree exists, and nothing tracks a system font install under a running
app — the OS font panels cache too), enumeration is slow on every platform, and the query is
synchronous on the UI thread. The bundled fonts the app ships under `resource/fonts/` are always
in it, even where the platform's own database does not report them.

Ask `capability(Cap::FontList)` before offering a font menu:

| Backend | `Cap::FontList` | Source |
| --- | --- | --- |
| macOS (AppKit) | Native | `NSFontManager.availableFontFamilies` + `availableMembersOfFontFamily:` |
| iOS (UIKit) | Native | `UIFont.familyNames` + `fontNamesForFamilyName:`, faces described through `UIFontDescriptor` |
| GTK | Native | the PangoCairo font map: `list_families()` → `list_faces()` |
| Qt | Native | `QFontDatabase::families()` + `styles()` |
| Android | Native | `/system/etc/fonts.xml` (the families `Typeface.create` resolves) plus the bundled `res/font/` families |
| HarmonyOS | Native | `OH_Drawing_FontMgr` families and style sets, plus the bundled manifest |
| Windows (XAML) | Native | DirectWrite's system font collection (a bundled family is addressed as `ms-appx:///fonts/<file>#<family>` on the way to the `TextBlock`, the one form unpackaged XAML loads; the app keeps naming it by family) |
| web-dom | **Emulated** | the CSS generic families (`system-ui`, `sans-serif`, `serif`, `monospace`, `cursive`, `fantasy`) with the four faces a browser synthesizes for any family, plus every bundled `FontFace` |

The web answer is composed rather than read on purpose: a browser exposes the machine's fonts
only through `queryLocalFonts()`, which is Chromium-only, asynchronous, and behind a permission
prompt. `Unsupported` means the list is empty, and an app should offer only the default face —
the `CanvasFont` default draws everywhere regardless.

## Canvas fonts

```rust
d.text("Aa Bb", at, TextStyle {
    size: 24.0,
    color: ink,
    font: CanvasFont { family: Some(family.clone()), weight: Some(FontWeight::Bold), italic: true },
    ..Default::default()
});
```

`CanvasFont` is a family (`None` = the platform's own UI face), a weight (`None` = Regular) and a
slant. The family is a name as `font_families()` lists it, or a bundled family's name; an unknown
name draws in the default face with one warning in the log. Size stays absolute — see
[docs/canvas.md](canvas.md) for why canvas text ignores the reader's text-size setting.

A weight or slant the family does not ship is synthesized where the platform does that (Skia on
Android and HarmonyOS, DirectWrite, CSS) and rounded to the nearest face elsewhere (Pango,
CoreText); `face_for` says which.

## Measurement

```rust
let m = day::measure_text("Aa Bb", 24.0, &font);   // TextMetrics { width, height, ascent }
let frame = Rect::new(at.x, at.y, m.width, m.height);
let baseline = at.y + m.ascent;
```

`measure_text` measures one line in the same engine `replay` draws it with, so the box it
returns is the box the anchors position: `width` is the advance width, `height` the line box
(ascent + descent, plus the line gap where the engine reports one), and `ascent` the baseline's
offset from the top. A toolkit that cannot measure answers `TextMetrics::approximate` — 0.6 ×
size per character, 1.2 × size tall, the baseline at 0.9 × size — so a caller always gets a
usable box; there is no capability to probe for measurement.

| Backend | Engine |
| --- | --- |
| AppKit / UIKit | `sizeWithAttributes:` + the font's `ascender` |
| GTK | a Pango layout's logical extents and `baseline()` |
| Qt | `QFontMetricsF` |
| Android | `Paint.measureText` + `FontMetrics` |
| HarmonyOS | `OH_Drawing_FontMeasureText` + `OH_Drawing_FontGetMetrics` |
| Windows (XAML) | a `TextBlock`'s desired size and `BaselineOffset` |
| web-dom | `measureText` with `fontBoundingBoxAscent` / `Descent` |

## For backend authors

Two defaulted `Toolkit` duties — `font_families()` (default: none) and `measure_text()`
(default: `None`) — plus `Cap::FontList`. Measure with the engine `replay` draws with, and put
a `Leading` anchor at the top-leading corner of the line box: baseline-origin APIs draw at
`at.y + ascent`.

On the wire (`day_spec::encode_ops`), a non-default font precedes its text record as
`OpCode::SetFont = 19`: `a` = the CSS weight (100 … 900, 0 = default), `b` = italic (0/1), the
family name on the texts channel (`""` = default). Like `SetGradient` and `StrokeStyle` it
applies to the next record and is then cleared; a decoder that does not know it must still
consume its texts entry. A default font emits no record, so a drawing without fonts encodes
exactly as before.

A shim that enumerates in C++, Java or JavaScript hands the list back as one text in the format
`day_spec::parse_font_list` decodes: families separated by U+001E; inside a family,
U+001F-separated fields — the family name, then one (face name, CSS weight, italic `0`/`1`)
triple per face; an empty face name is synthesized from the weight and slant.
