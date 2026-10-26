---
title: "6. Finish and test the app"
description: Complete the session counter and verify counting, reset, navigation, and shared state with a dayscript.
order: 6
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Finish the app with buttons that navigate between its two sections, then run an automated
walkthrough. Start from [5. Navigate between pages](/docs/tutorials/navigation). Keep
`Cargo.toml`, `Day.toml`, `build.rs`, and `src/main.rs` from lesson 1. The complete UI and catalog
below replace their earlier versions.

## 1. Complete the catalog

Replace `resource/locales/en/app.ftl` with this catalog. The earlier alert exercise's unused
messages are no longer needed:

<!-- tutorial-step: complete-app/messages -->

## 2. Complete the UI

Replace all of `src/lib.rs`:

<!-- tutorial-step: complete-app/ui -->

[`nav_link`](https://daybrite.dev/api/day_pieces/fn.nav_link.html) makes a button that navigates to a route key. These links and the sidebar update the
same navigation state. This app selects peer sections; it does not push a detail page onto a
back stack. Every visible message comes from the generated catalog accessors.

## 3. Run the finished app

<!-- tutorial-step: complete-app/launch -->

Add two sessions and press **Show summary**. Expect **Sessions completed: 2**. Press **Back to
counter**, reset, and open the summary again; expect zero.

<!-- tutorial-step: complete-app/quit -->

The app now combines event callbacks, reactive state, layout, and navigation. Press **Add one**
after completing a session. The count stays in memory and resets when you quit.

## 4. Automate the walkthrough

Create a script directory:

<!-- tutorial-step: complete-app/script-directory -->

Create `dayscript/session-counter.yaml`:

<!-- tutorial-step: complete-app/walkthrough -->

A **dayscript** is a YAML walkthrough executed by Day's automation engine. IDs locate controls;
message keys and arguments express expected localized text. `assert_route` checks the selected
route. `on_failure: stop` stops this walkthrough at its first failure.

Run from the app directory:

<!-- tutorial-step: complete-app/test -->

Expect all 15 steps to pass and the command to exit with status zero. Script assertion failures
exit with status 5. Scripted taps exercise Day's event dispatch inside the native app; also try
the controls by hand because this is not a physical mouse or keyboard test.

## Troubleshooting

| Symptom | Check and recovery |
| --- | --- |
| A script cannot find an ID | Match the YAML identifier to `id!(...)`, save both files, and rerun without `--skip-build`. |
| The first count is not zero | Start a fresh app run; the script assumes a new in-memory count. |
| Navigation does nothing | Match [`nav_link`](https://daybrite.dev/api/day_pieces/fn.nav_link.html) keys to the [`.item`](https://daybrite.dev/api/day_pieces/trait.NavBuilder.html#tymethod.item) keys; a visible translated label is not a route key. |
| A text assertion fails | Compare the expected argument with the actual count and keep the final Fluent catalog and Rust file together. |

## Next steps

You have completed the desktop learning path. Try the
[other-platform branch](/docs/tutorials/other-platforms) if you need another target.
For a larger app, continue with [localization](/docs/how-to/localization),
[state](/docs/explanation/state), or [navigation](/docs/how-to/navigation).
