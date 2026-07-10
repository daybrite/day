# day-fonts

Font-file conventions shared by Day's build tool and its runtime backends.

A ~100-line, bounds-checked sfnt `name`-table reader (`parse_font_names`) extracts a font's
family and PostScript names from `.ttf`/`.otf` bytes; `font_ident` derives the
resource-safe identifier both sides agree on ("Special Elite" → `special_elite`); and the
runtime helpers locate an app's bundled font files. This single vocabulary is why
`Font::Custom("Pacifico", 24.0)` resolves by family name on every platform Day targets —
the CLI stages files under names the backends can re-derive, with no side table.

Pure std, no dependencies. Usable anywhere you need a font's family name without pulling
in a shaping stack.

## Part of Day

[Day](https://daybrite.dev) builds cross-platform apps from each platform's *real* native
widgets — AppKit, UIKit, Android, GTK 4, Qt 6, WinUI, and ArkUI — from a single Rust
codebase. No web view, no bundled rendering engine: a `button("Save")` is an `NSButton` on
macOS and a Material button on Android.

Start at [daybrite.dev](https://daybrite.dev), or browse the
[source repository](https://github.com/daybrite/day).
