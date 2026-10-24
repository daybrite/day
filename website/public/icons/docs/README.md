<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Documentation sidebar icons

These SVGs are Google's **Material Symbols Outlined**, at optical size 24, from
[Google Fonts Icons](https://fonts.google.com/icons). Each file is an unchanged upstream
asset from the [Material Design Icons repository](https://github.com/google/material-design-icons).
The SVGs are licensed under **Apache-2.0**; the full license is in `LICENSE` beside them.
`sources.json` records the pinned upstream revision, exact download URL, and SHA-256 for each file.

`website/src/data/sidebar-icons.json` maps canonical page routes, reference group IDs, and the
mobile browse control to distinct symbols. Section headings and their index links share a
symbol because they describe the same destination. Decorative icons inherit the sidebar text
color through CSS masks and are hidden from assistive technology; the visible link text supplies
the accessible name. The browser loads SVGs locally, without a Google font or external icon request.

When adding a sidebar page, choose an unused symbol, download its outlined 24px SVG from the
pinned revision, and update both JSON files. Keep the upstream bytes intact and retain the license.
Run `npm --prefix website run docs:architecture` to check coverage, distinct geometry, local files,
and source hashes. Review new icons at sidebar size in light and dark themes.
