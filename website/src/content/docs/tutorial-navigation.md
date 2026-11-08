---
title: "5. Navigate between pages"
description: Configure sidebar navigation and share state across pages.
order: 5
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Add a summary page to the app from [lesson 4](/docs/tutorials/layout).
Retain the project files and catalog.

## 1. Name the sections

Append to `resource/locales/en/app.ftl`:

<!-- tutorial-step: navigation/messages -->

## 2. Share state across pages

Replace `src/lib.rs`:

<!-- tutorial-step: navigation/ui -->

[`nav(section)`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav.html) binds navigation to a
selected route key. `counter` and `summary` are identifiers; Fluent accessors supply visible labels.
[`.item`](https://docs.rs/day-pieces/latest/day_pieces/trait.NavBuilder.html#tymethod.item)
associates each key with a page builder. `NavStyle::Sidebar` selects the desktop sidebar.

Create `count` in `root` and pass it to both page builders. Switching sections can dispose pages
and their local state. Root-scoped state survives page switches but not app restarts.

## 3. Follow the count

<!-- tutorial-step: navigation/launch -->

<!-- tutorial-step: navigation/quit -->

Press **Add one** three times in **Counter**. Open **Summary**; expect **Sessions completed: 3**.
Return to **Counter**; expect **Count: 3**. Press **Reset** and reopen **Summary**; expect zero.
Quit the app.

## Troubleshooting

If page switches reset the count, create the Signal only in `root`.
If a page is blank, match the selection value to its
[`.item`](https://docs.rs/day-pieces/latest/day_pieces/trait.NavBuilder.html#tymethod.item) key.

## Next step

Continue to [6. Finish and test the app](/docs/tutorials/complete-app).
