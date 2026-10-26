---
title: "5. Navigate between pages"
description: Add counter and summary sections while keeping shared state above both pages.
order: 5
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Add a summary page that displays the same count. Complete
[4. Arrange the controls](/docs/tutorials/layout) first; keep the existing project files and catalog.

## 1. Name the sections

Append to `resource/locales/en/app.ftl`:

<!-- tutorial-step: navigation/messages -->

## 2. Share state across pages

Replace all of `src/lib.rs`:

<!-- tutorial-step: navigation/ui -->

[`nav(section)`](https://daybrite.dev/api/day_pieces/fn.nav.html) binds navigation to the selected route key. The strings `counter` and `summary`
are protocol identifiers, not visible labels; the Fluent accessors supply the labels.
[`.item`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.item) associates each key with a page-building closure. `NavStyle::Sidebar` selects a sidebar
presentation on this desktop target.

Create `count` in `root`, above the page builders. Sidebar pages can be disposed when you switch
sections; state created inside a page would belong to that page's lifetime. Passing the root's
signal to both pages keeps the count alive across switches. This does not save it across app launches.

## 3. Follow the count

<!-- tutorial-step: navigation/launch -->

<!-- tutorial-step: navigation/quit -->

In **Counter**, press **Add one** three times. Select **Summary** and expect **Sessions completed: 3**.
Return to **Counter** and expect **Count: 3**. Press **Reset**, then revisit **Summary**; it
should show zero. Quit the app.

## Troubleshooting

If switching pages resets the count, ensure [`Signal::new(0i64)`](https://daybrite.dev/api/day_reactive/struct.Signal.html#method.new) appears only in `root`, not in a
page function. If a section is blank, compare the initial selection with the [`.item`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.item) keys.
Both keys and the selection must use the same spelling.

## Next step

Continue to [6. Finish and test the app](/docs/tutorials/complete-app).
