---
title: "6. Finish and test the app"
description: Implement navigation buttons and verify the app with dayscript.
order: 6
section: Tutorials
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Add navigation buttons and automated tests to the app from
[lesson 5](/docs/tutorials/navigation). Retain `Cargo.toml`, `Day.toml`, `build.rs`, and `src/main.rs`.
Replace the UI and catalog with the versions below.

## 1. Complete the catalog

Replace `resource/locales/en/app.ftl`. The unused alert messages are removed:

<!-- tutorial-step: complete-app/messages -->

## 2. Complete the UI

Replace `src/lib.rs`:

<!-- tutorial-step: complete-app/ui -->

[`nav_link`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_link.html) creates a button that
selects a route key. These buttons and the sidebar update the same navigation state.
This app switches peer sections; it does not push pages onto a back stack.
All UI text uses generated Fluent accessors.

## 3. Run the finished app

<!-- tutorial-step: complete-app/launch -->

Add two sessions and press **Show summary**; expect **Sessions completed: 2**.
Press **Back to counter**, reset, and reopen the summary; expect zero.
The in-memory count resets when the app exits.

<!-- tutorial-step: complete-app/quit -->

## 4. Automate the walkthrough

Create the script directory:

<!-- tutorial-step: complete-app/script-directory -->

Create `dayscript/session-counter.yaml`:

<!-- tutorial-step: complete-app/walkthrough -->

Dayscript is Day's YAML UI automation format. IDs locate controls; message keys and arguments
specify expected localized text. `assert_route` checks navigation; `on_failure: stop` stops at
the first failed assertion.

Run from the app directory:

<!-- tutorial-step: complete-app/test -->

Expect 15 passing steps and exit status zero. Assertion failures return status 5.
Scripted taps test Day's event dispatch; test physical mouse and keyboard input separately.

## Troubleshooting

| Symptom | Check and recovery |
| --- | --- |
| Script cannot find an ID | Match YAML IDs to `id!(...)`, save both files, and rerun without `--skip-build`. |
| Initial count is not zero | Launch a new app instance. |
| Navigation fails | Match [`nav_link`](https://docs.rs/day-pieces/latest/day_pieces/fn.nav_link.html) keys to [`.item`](https://docs.rs/day-pieces/latest/day_pieces/trait.NavBuilder.html#tymethod.item) keys, not translated labels. |
| Text assertion fails | Check arguments against the count; use matching final catalog and Rust files. |

## Next steps

See [other platforms](/docs/tutorials/other-platforms),
[localization](/docs/how-to/localization), [state](/docs/explanation/state), and
[navigation](/docs/how-to/navigation).
