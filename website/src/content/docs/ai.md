---
title: AI-assisted development
description: "Build a Day app with Claude Code from the terminal: scaffold, add a weather page by prompt, script it with dayscript, and put the whole loop in GitHub CI."
order: 24
section: Guides
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

Day treats an AI agent as a full developer. Every launch embeds the
[dayscript](/docs/dayscript) engine, and the `day` CLI exposes it as MCP tools an agent can
call to build, relaunch, tap, type, assert, and screenshot. The agent drives the running app and
checks the result on screen, on every platform you target.

This guide walks that loop end to end with [Claude Code](https://claude.com/claude-code) in a
plain terminal: scaffold an app, have the agent add a weather page, script the page with
dayscript, and put the script in GitHub CI. The [getting started](/docs/getting-started) page
covers the same ground with VS Code's agent mode; everything below needs only two CLIs.

## 0. Install the two CLIs

```bash
cargo install day-cli        # the day CLI (see getting-started for toolkit prerequisites)
npm install -g @anthropic-ai/claude-code
day doctor                   # confirms your desktop toolkit is ready
```

## 1. Scaffold the app

```bash
day new app skycheck --toolkit macos-appkit --no-input
cd skycheck
day launch -p macos-appkit
```

You get a running native app with adaptive navigation, an editable list, localized strings, a
dayscript walkthrough (`dayscript/demo.yaml`), and an `AGENTS.md` that teaches
any coding agent the project's conventions: where pages live, how routes register, that every
control gets a stable `.id()`, and that new strings go into *every* locale.

## 2. Give Claude Code the day tools

```bash
claude mcp add day -- day --project . mcp-server
claude
```

`day mcp-server` (docs: [agent surface](/docs/internal/agent)) exposes ten tools:
`day_metadata`, `day_build`, `day_launch`, `day_relaunch`, `day_drive`, `day_screenshot`, and
friends. The two this loop depends on are `day_relaunch`, which returns compile errors *inside
the tool result* so the agent fixes and retries on its own, and `day_drive`, whose screenshots
come back as images the agent can read.

## 3. Add a weather page, by prompt

In the Claude Code session:

> Add a "weather" page to the sidebar: a city picker (Lisbon, Nairobi, Osaka), a large
> temperature label, a one-line conditions label, and a Refresh button that simulates a reload
> with day::sleep. Use demo data, no network. Give every control a stable id
> (weather-city, weather-temp, weather-conditions, weather-refresh), localize every string in
> all locales, then relaunch and show me a screenshot of the page.

Watch the loop the scaffolded `AGENTS.md` prescribes: `day_metadata` first, then the edits, a
`day_relaunch` (fixing anything the compiler says), then a `day_drive` that navigates to the
page and hands back a screenshot. The page it lands on will be a normal Day page. Abridged, it
should look like this:

```rust
pub(crate) fn weather_page() -> impl Piece {
    let city = Signal::new(0usize);
    let cities = ["Lisbon", "Nairobi", "Osaka"]; // res::str keys in the real page
    let temps = ["18 °C", "24 °C", "11 °C"];
    column((
        label(crate::res::str::weather_title()).font(Font::Title).id("weather-title"),
        picker(cities.iter().cloned(), city).id("weather-city"),
        label(move || temps[city.get()].to_string())
            .font(Font::LargeTitle)
            .id("weather-temp"),
        // …conditions label, and a Refresh button whose action is
        // day::task(async move { day::sleep(600).await; /* set signals */ })
    ))
    .spacing(12.0)
    .padding(16.0)
}
```

If the result isn't right, say so in the same session ("the temperature should update when the
city changes") and the agent re-drives the app to show the fix. You never leave the terminal,
and every claim comes back with a screenshot.

## 4. Script it: dayscript

Now freeze that verification into a script anyone can rerun: human, agent, or CI. Ask the
agent to write it, or drop this in as `dayscript/weather.yaml`:

```yaml
flow:
  - wait_for: { id: nav }
  - navigate: { route: weather }
  - assert_route: { route: weather }
  - assert_visible: { id: weather-title }

  # The picker drives a signal; the labels read it; assert the round trip.
  - select: { id: weather-city, index: 1 }
  - assert_text: { id: weather-temp, text: "24 °C" }
  - tap: { id: weather-refresh }
  - assert_visible: { id: weather-conditions }

  - screenshot: weather
```

```bash
day launch -p macos-appkit --script dayscript/weather.yaml
day launch -p macos-appkit --script dayscript/weather.yaml --variant dark --env DAY_THEME=dark
day launch -p macos-appkit --script dayscript/weather.yaml --variant fr --locale fr
```

Each run drives the real app and writes content-checked captures under
`build/day/screenshots/<target>/<variant>/`, the same mechanism that produces the localized
[gallery](/gallery) on this site. For strings that vary by locale, assert by Fluent key
(`assert_text: { id: …, key: … }`) instead of literal text and one script passes in every
language.

You can also record the script instead of writing it:
`day launch -p macos-appkit --record dayscript/weather.yaml` captures your real taps, typing, and
navigation into a replayable dayscript while you use the app, rewriting the file continuously so
it survives a kill. Recording a manual walkthrough is the fastest way to get a first script; add
the `assert_*` steps by hand afterward.

## 5. Put it in CI

A minimal GitHub workflow that builds the app headlessly on Linux, the cheapest runner, and runs
the script on every push:

```yaml
name: ci
on: [push, pull_request]
jobs:
  walkthrough:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - run: |
          sudo apt-get update
          sudo apt-get install -y --no-install-recommends \
            libgtk-4-dev libadwaita-1-dev pkg-config xvfb
      - run: cargo install day-cli
      - name: Drive the app
        run: |
          xvfb-run -a -s "-screen 0 1000x720x24" \
            day launch -p linux-gtk --script dayscript/weather.yaml
      - uses: actions/upload-artifact@v4
        with:
          name: screenshots
          path: build/day/screenshots
```

A failed assertion is a red build; the uploaded captures show reviewers what the app
looked like. From here, add targets to the matrix as your app grows (`day new`'s scaffold works
unchanged on all twelve), or adopt the fuller multi-platform workflow Day itself publishes in
[`daybrite/actions`](https://github.com/daybrite/actions).

## Where to go next

- [Testing with dayscript](/docs/dayscript) — the full step catalog and how the engine works.
- [For AI agents](/docs/for-agents) — the terse rulebook agents should follow (link it from
  your own prompts).
- [Agent surface reference](/docs/internal/agent) — `day drive`, sessions, and the MCP tools in
  detail.
