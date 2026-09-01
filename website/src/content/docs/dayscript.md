---
title: Testing with dayscript
description: "The YAML automation language that drives a running Day app: steps, assertions, screenshots, and how it works."
order: 23
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

**dayscript** is Day's automation language: a YAML file of steps that drives and asserts a
*running* app. One script taps buttons, types text, navigates, asserts what's on screen, and
captures screenshots, identically on macOS, iOS, Android, Linux, Windows, and OpenHarmony,
because it addresses your UI by the stable ids you gave your Pieces.

It resembles Maestro, but the engine is compiled into your app and executes steps as real Day
events, which makes the same script portable across all targets and makes waits deterministic
instead of sleep-based.

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
day launch -p android-mdc --script dayscript/walkthrough.yaml --locale fr
```

`day launch` builds, starts the app with the scripting engine invited, executes the steps, and
exits nonzero if any assertion fails (exit code 5). Screenshots land under
`build/day/screenshots/<target>/<subdir>/`, where the subdirectory is the `--variant` name when
given, else the locale, else `default`. Several `--script` flags run in sequence, and
`--locale` makes the run a localization test at the same time; assertions can reference Fluent
keys instead of literal strings, so the same script passes in every language.

## The step vocabulary

| Group | Steps |
|---|---|
| Waiting | `wait_for` (an id appears; `timeout_secs` raises its budget), `wait_idle`, `pause` |
| Acting | `tap` (`repeat`), `input`, `set_value`, `toggle`, `select`, `submit`, `focus`, `scroll_to` (to an `edge`, an `x`/`y` offset, or an element to reveal), `reorder` (list row `from` → `to`) |
| Navigation | `navigate`, `nav_back`, `assert_route` |
| Window chrome | `menu` (`item`/`key`/`path`), `toolbar` (`item`, plus `text`/`key` or `on`), `close_window` (`window`) |
| Asserting | `assert_visible`, `assert_text`, `assert_value`, `assert_focused`, `assert_no_placeholders` (`allow` lists expected gaps) |
| Dialogs | `assert_presented`, `respond` (a `button` index, prompt `text`, file `path`, or `dismiss`) |
| Evidence | `screenshot` (`window` captures a secondary window), `a11y_audit` |
| Exit | `expect_exit` (the app must terminate within `within` seconds; always the last step) |

`input`, `assert_text`, and `toolbar` accept a Fluent `key` (with `args`) in place of literal
text, resolved in the run's locale, so one script passes in every language.

`nav_back` reads the window's width class the same way the app does. A window wide enough to keep
the detail beside the list never pushed a page, so the step passes there without moving anything,
while a compact window still fails when it finds nothing to pop. One script can therefore drive a
phone and a tablet, since the same build stacks on one and splits on the other.

Every locating step waits (bounded, five seconds by default) rather than failing instantly, so
scripts need no hand-tuned sleeps. Acting steps synthesize Day events on the
main thread between flushes, so they are deterministic and behave identically on every toolkit.
Target elements by ids you know to be interactive, and scroll explicitly when a step needs an
element brought into view.

Any step can be gated per target: `skip_on:` drops it on the named targets or toolkits
(`skip_on: [web-dom]`), and `only_on:` is its mirror, for a step whose expectations differ per
target (an `assert_no_placeholders` allow list, say). One walkthrough then covers every backend
without forking per platform.

## How it works

The engine lives in `day-script`, compiled into your app. It activates only when invited: the
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
with the reactive turn, "wait until idle" has a real definition (the reactive queue is empty and
layout is clean) rather than a timeout heuristic.

## What it's for beyond tests

The same scripts serve several jobs:

- **CI walkthroughs:** every push builds the showcase on all targets and runs the walkthrough;
  the [gallery](/gallery) is those screenshots. A content-validation step catches blank captures.
  One launch covers the whole appearance matrix: `day launch --themes light,dark --locales
  en,fr,ar` builds once and runs the script per theme × locale, naming each variant's
  screenshot directory after it.
- **Iteration:** Day has no hot reload, so `--script goto-settings.yaml` after each relaunch puts
  you back on the screen you're editing.
- **Accessibility audits:** the `a11y_audit` step diffs the native accessibility tree against
  your declarations ([details](/docs/accessibility#auditing-the-native-tree)).
- **Agent verification:** AI coding agents use dayscript to check their own work: write a
  change, run a script, read the assertions ([for agents](/docs/for-agents)).

## Recording

You don't have to write a script from scratch. `day::record` captures the taps, edits, selections,
and navigation an app receives and turns them back into a dayscript. It observes the one point
every backend funnels its events through, so it needs no per-toolkit code.

Record headlessly from the CLI:

```bash
day launch -p macos-appkit --record recording.yaml
```

Drive the app by hand; `recording.yaml` is rewritten as you go and holds everything up to the last
action even if the app is killed. Because it's an ordinary dayscript, you replay it on any target:

```bash
day launch -p android-mdc --script recording.yaml
```

Or record and replay inside the app. `day::record::start_into(buffer)` streams the script into a
`Signal<String>` you can bind a `text_area` to; `day::play_script(&yaml)` replays one in-process
through the same engine `--script` drives. The showcase's **Scripting** page is a working example
(Record, move around, Stop, edit, Play). `exclude_prefix` keeps a UI's own record and stop controls
out of its recording.

The recorder skips the same gestures the engine cannot inject. It captures actions on elements
you gave ids; positional taps, slider drags, and native OS chrome are not recorded, so a recording
is a starting point you edit rather than a pixel-exact replay.

### Logging actions without recording

The same observer can log instead of capture. `day::record::log_actions(true)`, or
`DAY_LOG_ACTIONS=1` on any Day app without a rebuild, echoes every action to stdout in the same
vocabulary and keeps nothing:

```text
dayscript ▸ navigate → dates  "Date & time"
dayscript ▸ tap list-shuffle  "Shuffle"
dayscript ▸ select unit-picker = 1  "Units"
```

Nothing accumulates, so it is cheap to leave on for an app's whole life, and it reads as the script
a recording would have written. That is useful for watching what a walkthrough will capture before
you record it, and for making a bug report say what was actually pressed. The Showcase turns it on
at launch; `DAY_LOG_ACTIONS=0` silences it. Logging and recording are independent: start a recording
underneath a log and each action still prints once, with the prefix naming the mode
(`day record ▸`).

## Limits

dayscript can only see what Day owns. It cannot type through the native IME, verify the software
keyboard, drive OS permission prompts or file dialogs, or assert native animations. The project's
practice is scripted coverage for everything Day-side plus a short manual pass per platform for
those native surfaces: text input, the keyboard, OS dialogs, and animations. Unit-level testing
below the UI has a separate tool: the [mock toolkit](/docs/rendering#the-mock-toolkit) runs your
Pieces headlessly in `cargo test`.
