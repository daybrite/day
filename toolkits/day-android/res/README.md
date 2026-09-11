# day-android resources

`drawable/day_symbol_*.xml` is the toolkit's glyph set for `day_spec::Symbol` (docs/toolbars.md):
one Android vector drawable per variant, named `day_symbol_<variant in snake case>`, folded
into every Day app's Gradle build through this crate's `[package.metadata.day.android] res`.

The art is [Material Symbols](https://fonts.google.com/icons) by Google — the outlined style,
24 px, weight 400 — fetched from `fonts.gstatic.com` and wrapped verbatim in the vector-drawable
envelope (the 960-unit viewport, translated so the font's negative y axis lands on screen).
Material Symbols are licensed under the
[Apache License 2.0](https://github.com/google/material-design-icons/blob/master/LICENSE).
`day_symbol_oval.xml`, `day_symbol_line.xml`, `day_symbol_group.xml` and `day_symbol_ungroup.xml` are drawn here, in the same viewport, since the set has no ellipse, free line, or grouping glyph. These files carry no Day
copyright header: they are vendored third-party art.

| Symbol | Material name |
|---|---|
| Add / Remove / Delete / Edit | add / remove / delete / edit |
| New / Open / Save / Print | note_add / folder_open / save / print |
| Refresh / Search / Share / Settings / Info | refresh / search / share / settings / info |
| Star / Bookmark / Home / Sidebar | star / bookmark / home / view_sidebar |
| Back / Forward / Up / Down | arrow_back / arrow_forward / arrow_upward / arrow_downward |
| Filter / Sort / More | filter_list / sort / more_horiz |
| Play / Pause / Stop / Camera / Code | play_arrow / pause / stop / photo_camera / code |
| Light / Dark / Auto | light_mode / dark_mode / contrast |
| ZoomIn / ZoomOut / ZoomReset | zoom_in / zoom_out / fit_screen |
| Undo / Redo / Copy / Cut / Paste | undo / redo / content_copy / content_cut / content_paste |
| Mail / Folder / Document | mail / folder / description |
| Check / Close / Warning | check / close / warning |
| Rectangle / Circle / CircleFilled | rectangle / radio_button_unchecked / circle |
| Text | text_fields |
| Oval / Line / Group / Ungroup | drawn here |
