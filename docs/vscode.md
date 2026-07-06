# VS Code extension

`editors/vscode/` is a VS Code extension that builds and runs Day apps across one or more targets from
the editor. It is a thin, conventional wrapper over the `day` CLI: the control surface is a sidebar +
status bar + command palette, and **execution goes through the VS Code Tasks API** — each launch is a
`day` Task in its own integrated terminal, so output is native (ANSI colors intact) and filtered per
target, and stop/restart ride the standard task lifecycle.

## What it does

- **Day sidebar** (activity bar): the current project, a *Configuration* section (build mode / locale /
  dayscript), and a *Targets* section with per-target checkboxes. Targets this host can't build are
  disabled.
- **Run / Build** the selected targets. Multiple targets launch **simultaneously**, each in its own
  terminal, each **stoppable / restartable independently** (inline buttons + status bar).
- **Build mode** (`--profile debug|release`), **locale** (`--locale`), and an optional **dayscript**
  (`--script`) — all editable from the sidebar or the command palette.
- **`day` task type**: auto-detected `day: build/run <target>` tasks integrate with `Ctrl+Shift+B`,
  `tasks.json`, and key bindings (see `apps/showcase/.vscode/tasks.json` for an example). Build errors
  surface through the `$rustc` problem matcher.
- **Doctor**: runs `day doctor` to check toolchains.

## How it maps to the CLI

| UI action | CLI invocation |
|---|---|
| Run target(s) | `day --project <root> launch -p <target> --profile <mode> [--locale …] [--script …] [--env …]` |
| Build target(s) | `day --project <root> build -p <target> --profile <mode>` |
| Stop | `TaskExecution.terminate()` → SIGTERM → `day` kills the app + simctl/adb watchers (`signals.rs`) |
| Restart | terminate + re-execute |

Stop is clean on every platform because `day` traps **both SIGINT and SIGTERM** and tears down the app
and its log watchers.

## The `day` binary

Set `day.cliPath` to your `day` binary. If it isn't on `PATH` **and** the workspace is the Day repo (a
Cargo workspace with a `day-cli` member), the extension falls back to `cargo run -q -p day-cli --`, so
it works in-repo with no installed binary.

## Developing / trying it

```bash
cd editors/vscode && npm install && npm run compile
```

Press **F5** (Run → "Run Day Extension") to open an Extension Development Host with the Day repo loaded.
The **Day** sidebar shows the Showcase app and its 5 targets — tick `macos-appkit`, click **Run**, and
the app launches in a terminal; tick `ios-uikit` too and both run at once; use the inline stop/restart
buttons per target. `npx @vscode/vsce package` produces an installable `.vsix`.

Scope of v1: multi-target run/stop/restart, mode/locale/dayscript selection, the `day` task provider,
project detection, and `day doctor`. Deferred: debugger/DAP, emulator management UI, and packaging/sign
flows.
