---
title: "Agentic development"
description: "How AI agents build, launch, drive, and screenshot a running Day app: the session registry, the MCP tools, and the crash post-mortem loop."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Agentic development

How coding agents (VS Code agent mode, Claude Code, any MCP client) build, run, drive, and
see Day apps. Every capability lives in the day CLI behind stable commands, so all agents and
editors share one implementation; editor extensions only register it.

## The session registry

Every `day launch` records its app's dayscript-engine coordinates in
`build/day/sessions.json`:

```json
[{ "target": "macos-appkit", "appId": "dev.example.app", "profile": "debug",
   "enginePort": 34832, "engineToken": "…", "startedAt": 1783961597094 }]
```

The engine now rides **every** launch (loopback TCP, token-gated), not just `--script` runs,
so an app the developer opened an hour ago is still drivable. One session per target; entries
drop on `day stop` and are replaced by a new launch of the same target. A launch's engine env
(`DAYSCRIPT_PORT`/`DAYSCRIPT_TOKEN`) reaches the app the same way scripted runs always did
(process env on desktop, intent extras on Android, `--ps` want-params on OpenHarmony).

## `day drive`: dayscript against a running app

```sh
day drive -p macos-appkit --steps-json \
  '[{"navigate":{"route":"controls"}},{"wait_idle":null},
    {"tap":{"id":"increment-button","repeat":2}},
    {"assert_text":{"id":"counter-label","text":"2 clicks"}},
    {"screenshot":"after"}]'
```

Steps use the walkthrough vocabulary (single-key mapping form, or flattened `{"op": …}`);
the step catalog lives in the dayscript reference
([website](https://daybrite.dev/docs/dayscript); the shipped list is DESIGN.md Appendix C)
rather than being copied here, where it has drifted before. The output is one JSON object
(`{target, steps: [{op, ok, error?, screenshot?}…], failed}`) on stdout; screenshots land in
`build/day/screenshots/_drive/` and are inlined as base64 for callers that want the pixels.
Device targets get their engine port forwarded automatically (adb / hdc), like
scripted runs.

## When the app dies mid-script

A scripted run whose app crashes ends in `engine connection lost`, which says only that the app is
gone. The runner then prints a post-mortem (`crates/day-cli/src/diagnose.rs`) from whatever this
host can produce:

- **day-break's own artifacts** ([docs/break.md](break.md)), when the app arms it: the kind of death, the
  panic message and location, the signal, how long the app lived, and the backtrace it captured.
  Reports are finalized on the app's next launch, so a fresh crash shows its raw session artifacts
  instead; either way, only the ones whose backend and session start match the run that just
  failed, because the store is keyed by app id and every target shares it.
- **The OS crash report**: on macOS and the iOS simulator, the `.ips` from
  `~/Library/Logs/DiagnosticReports`, rendered as the exception, the termination reason, and the
  faulting thread's frames rather than its several hundred lines of loaded-image addresses. It is
  matched by pid on a desktop launch, and the runner waits up to 30 s for it, because ReportCrash
  writes it well after the process dies, so looking once finds the previous run's or nothing.
- **Android**: the emulator's crash buffer (`adb logcat -b crash`).

Scripted launches also default `RUST_BACKTRACE=1`, so a panic's stack is in the streamed log the
first time; nobody is watching an unattended run to re-run it with the variable set. An explicit
`--env RUST_BACKTRACE=…` wins.

The same post-mortem runs on a plain `day launch` (no script): an attached launch whose app dies on
a fatal signal prints it before returning, and the command's own exit code carries the signal
(`128 + signo`; SIGABRT is 134), where it used to report 0 and look like a clean quit.

Under GitHub Actions the headline also becomes an `::error::` annotation, so the job page names the
crash without anyone opening the log.

> [!NOTE]
> **Desktop targets only, for now.** The diagnosis triggers where day can observe the app's death:
> the desktop launches, whose app is day's own child, and any target whose engine connection drops
> mid-script. On `ios-uikit` and `android-mdc` the app runs on a simulator/device and the process
> day waits on is a log pump that outlives it, so an interactive crash there still passes
> unnoticed. Closing that needs a liveness poll (`simctl spawn booted launchctl list <bundle-id>`,
> `adb shell pidof <app-id>`) while a launch is attached.

## `day stop` / `day relaunch`

- `day stop -p <target>… | --all` — terminate launches (per-platform: pkill / taskkill /
  `simctl terminate` / `am force-stop` / `aa force-stop`) and drop their sessions.
- `day relaunch -p <target>… | --all-running` — stop + rebuild + launch, recording fresh
  sessions. This is the agent's "apply my code changes" verb.

## `day mcp-server`

A Model Context Protocol server on stdio (newline-delimited JSON-RPC). Each tool call shells
back into the day CLI, so the server only forwards calls. Tools:

| tool | wraps |
|---|---|
| `day_metadata` | `metadata --json` |
| `day_doctor` | `doctor [--toolkit …]` |
| `day_build` | `build -p … --profile …` |
| `day_launch` | `launch -p … --detach [--locale …] [--env K=V]` |
| `day_relaunch` | `relaunch` (no targets ⇒ all running sessions) |
| `day_stop` | `stop` (no targets ⇒ `--all`) |
| `day_running` | the session registry + reachability probe |
| `day_drive` | `drive -p … --steps-json …`; screenshots become MCP **image** content |
| `day_screenshot` | `drive` with `wait_idle` + `screenshot` |
| `day_lint` | `lint` |

A server serves exactly one project (the `--project` it was spawned with), and no tool takes a
project argument, so every result opens with a line naming that project. An agent working in a
window of several apps reads it and knows which server it is holding.

A tool call shells back into the server's own executable, or into whatever `DAY_SELF_COMMAND`
names ([docs/environment.md](environment.md)). That matters when day itself is under development:
with `day.cliSource` set, the editor runs the CLI as `cargo run` against the open checkout, and
the extension passes the same invocation here, so a day-cli edit is compiled into the next tool
call instead of the agent running the binary that was on disk when the server started.
Changes to the server's own dispatch (this file's `mcp.rs`) still need **MCP: Restart Server**,
which recompiles it on the way back up.

VS Code: the Day extension registers one server per Day project in the window, labeled
`Day: <app title>` (`day.mcp.enabled`, default on). Agent mode then has all ten tools for each.
Other MCP clients point at `day --project <root> mcp-server`.

## The loop agents should follow

1. `day_metadata` → what targets/locales exist. Read the project's `AGENTS.md` (scaffolded by
   `day new`) for the page/localization/id conventions.
2. Edit code with normal file tools.
3. `day_relaunch` → compile errors come back in the result; fix; repeat.
4. `day_drive` → navigate to the changed screen, assert ids/text, `screenshot`, and **look**
   at it. On every affected target.
5. `day launch -p <target> --script dayscript/walkthrough.yaml` when the change touches
   walkthrough-covered flows.

## Security posture

The engine binds loopback only and requires the per-launch token; sessions.json holds that
token, scoped to the project's own build directory. MCP clients surface tool calls for user
confirmation per their own policy (VS Code agent mode does). The VS Code extension declares
`untrustedWorkspaces: false`; none of this runs in Restricted Mode.
