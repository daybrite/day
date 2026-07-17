---
title: Testing with dayscript
description: "The YAML automation language that drives a running Day app: steps, assertions, screenshots, and how it works."
order: 23
section: Guides
---

**dayscript** is Day's automation language: a YAML file of steps that drives and asserts a
*running* app. One script taps buttons, types text, navigates, asserts what's on screen, and
captures screenshots — identically on macOS, iOS, Android, Linux, Windows, and OpenHarmony,
because it addresses your UI by the stable ids you gave your Pieces, not by pixels or platform
selectors.

If you've used Maestro for mobile testing, the shape is familiar. The difference is that the
engine is compiled into your app and executes steps as real Day events, which makes the same
script portable across all targets and makes waits deterministic instead of sleep-based.

## A script

```yaml
name: walkthrough
flow:
  - wait_for:      { id: home-title }
  - screenshot:    home
  - navigate:      { route: controls }
  - assert_route:  { route: controls }
  - input:         { id: name-field, text: "Ada" }
  - tap:           { id: increment-button }
  - assert_value:  { id: counter-label, value: "1 click" }
  - tap:           { id: btn-alert }
  - assert_presented:
  - respond:       { button: 0 }
  - a11y_audit:
  - screenshot:    controls
```

Run it against any target:

```bash
day launch -p macos-appkit --script dayscript/walkthrough.yaml
day launch -p android-widget --script dayscript/walkthrough.yaml --locale fr
```

`day launch` builds, starts the app with the scripting engine invited, executes the steps, and
exits nonzero if any assertion fails (exit code 5). Screenshots land under
`build/day/screenshots/<target>/<locale>/`. Several `--script` flags run in sequence, and
`--locale` makes the run a localization test at the same time — assertions can reference Fluent
keys instead of literal strings, so the same script passes in every language.

## The step vocabulary

| Group | Steps |
|---|---|
| Waiting | `wait_for` (an id appears), `pause` |
| Acting | `tap`, `input`, `set_value`, `toggle`, `select`, `focus` |
| Navigation | `navigate`, `nav_back`, `assert_route` |
| Asserting | `assert_visible`, `assert_text`, `assert_value`, `assert_focused` |
| Dialogs | `assert_presented`, `respond` |
| Evidence | `screenshot`, `a11y_audit` |

Every locating step waits (bounded, five seconds by default) rather than failing instantly, which
removes the sleep-tuning that makes UI tests flaky. Acting steps synthesize Day events on the
main thread between flushes, so they are deterministic and behave identically on every toolkit —
target elements by ids you know to be interactive, and scroll explicitly when a step needs an
element brought into view.

## How it works, briefly

The engine lives in `day-script`, compiled into your app. It activates only when invited — the
launcher passes a localhost port and a one-time token through the environment; without them the
engine never binds a socket, in debug or release. Steps arrive as JSON over that socket and
execute on the main thread between reactive flushes:

```text
day launch --script …          your app process
┌───────────────┐   localhost  ┌────────────────────────────────┐
│ script runner │ ───────────► │ day-script engine              │
│ (in the CLI)  │  step + token│  id → node (day-core index)    │
└───────────────┘ ◄─────────── │  synthesize Day event / assert │
        reply: ok / error / png└────────────────────────────────┘
```

A `tap` runs the same action path a user's tap would; `input` goes through the controlled-text
machinery; `screenshot` asks the toolkit for a native window snapshot. Because steps interleave
with the reactive turn, "wait until idle" has a real definition — no pending reactive work, no
dirty layout — rather than a timeout heuristic.

## What it's for beyond tests

The same scripts serve several jobs:

- **CI walkthroughs.** Every push builds the showcase on all targets and runs the walkthrough;
  the [gallery](/gallery) is those screenshots. A content-validation step catches blank captures.
- **Iteration.** With no hot reload, `--script goto-settings.yaml` after each relaunch puts you
  back on the screen you're editing. Cheap and surprisingly effective.
- **Accessibility audits.** The `a11y_audit` step diffs the native accessibility tree against
  your declarations ([details](/docs/accessibility#verifying-not-trusting)).
- **Agent verification.** AI coding agents use dayscript to check their own work — write a
  change, run a script, read the assertions ([for agents](/docs/for-agents)).

## Limits

dayscript can only see what Day owns. It cannot type through the native IME, verify the software
keyboard, drive OS permission prompts or file dialogs, or assert native animations. Those blind
spots are real; the project's practice is scripted coverage for everything Day-side plus a short
manual smoke per platform for the native seams. Unit-level testing below the UI has a separate
tool — the [mock toolkit](/docs/rendering#the-mock-toolkit) runs your Pieces headlessly in
`cargo test`.
