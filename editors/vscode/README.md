# Day for VS Code

Build and run [Day](https://daybrite.dev) apps — cross-platform native UI in Rust — across one or more
targets, straight from VS Code. It's a thin, conventional wrapper over the `day` CLI: each launch runs
as a VS Code **Task** in its own integrated terminal, so output is native (ANSI colors intact) and
filtered per target, and processes stop/restart through the standard task lifecycle.

## Features

- **Day sidebar** — pick a project, tick one or more targets, and Run / Build. Targets your host can't
  build (e.g. `windows-*` on macOS) are shown disabled.
- **Multiple simultaneous targets** — each selected target launches in its own terminal and can be
  **stopped / restarted independently** (inline buttons, or the status bar).
- **Build mode** (debug / release), **locale** (`--locale`), and an optional **dayscript**
  (`--script`) — all editable from the sidebar or command palette.
- **`day` task type** — auto-detected `day: build <target>` / `day: run <target>` tasks integrate with
  the Tasks system, `Ctrl+Shift+B`, and key bindings. Build errors surface via the `$rustc` matcher.
- **Doctor** — run `day doctor` to check your toolchains.

## Requirements

The `day` CLI. Set `day.cliPath` to your `day` binary. If it's not on `PATH` **and** the workspace is
the Day repository (a Cargo workspace with a `day-cli` member), the extension automatically falls back
to `cargo run -q -p day-cli --`, so it works in-repo with no installed binary.

## Settings

| Setting | Default | Description |
|---|---|---|
| `day.cliPath` | `day` | Path to the `day` CLI (falls back to `cargo run -p day-cli --` in the Day repo). |
| `day.defaultProfile` | `debug` | Default build mode. |
| `day.defaultLocale` | `""` | Default `--locale` (empty = app/system default). |
| `day.extraEnv` | `{}` | Extra `KEY=VALUE` env passed to every launch via `--env`. |

## Developing this extension

```bash
cd editors/vscode
npm install
npm run compile   # or: npm run watch
```

Press **F5** (Run → "Run Day Extension") to open an Extension Development Host with the Day repo loaded.
The **Day** sidebar lists the Showcase app and its targets — tick `macos-appkit`, click **Run**, and
the app launches in a terminal. Tick a second target to run both at once; use the inline
stop/restart buttons per target.
