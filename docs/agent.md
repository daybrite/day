# Agentic development

How coding agents (VS Code agent mode, Claude Code, any MCP client) build, run, and — uniquely —
**drive and see** Day apps. The design rule throughout: every capability lives in the day CLI
behind stable commands, so all agents and editors share one implementation; editor extensions
only register it.

## The session registry

Every `day launch` records its app's dayscript-engine coordinates in
`build/day/sessions.json`:

```json
[{ "target": "macos-appkit", "appId": "dev.example.app", "profile": "debug",
   "enginePort": 34832, "engineToken": "…", "startedAt": 1783961597094 }]
```

The engine now rides **every** launch (loopback TCP, token-gated), not just `--script` runs —
so an app the developer opened an hour ago is still drivable. One session per target; entries
drop on `day stop` and are replaced by a new launch of the same target. A launch's engine env
(`DAYSCRIPT_PORT`/`DAYSCRIPT_TOKEN`) reaches the app the same way scripted runs always did
(process env on desktop, intent extras on Android, `--ps` want-params on OpenHarmony).

## `day drive` — dayscript against a running app

```sh
day drive -p macos-appkit --steps-json \
  '[{"navigate":{"route":"controls"}},{"wait_idle":null},
    {"tap":{"id":"increment-button","repeat":2}},
    {"assert_text":{"id":"counter-label","text":"2 clicks"}},
    {"screenshot":"after"}]'
```

Steps use the walkthrough vocabulary (single-key mapping form, or flattened `{"op": …}`):
`navigate`, `nav_back`, `tap`, `input`, `set_value`, `toggle`, `select`, `wait_for`,
`wait_idle`, `assert_visible`, `assert_text`, `assert_value`, `assert_route`,
`assert_presented`, `respond`, `a11y_audit`, `pause`, `screenshot`. Output: one JSON object
(`{target, steps: [{op, ok, error?, screenshot?}…], failed}`) on stdout; screenshots land in
`build/day/screenshots/_drive/` and are inlined as base64 for callers that want the pixels.
Device targets get their engine port forwarded automatically (adb / hdc), exactly like
scripted runs.

## `day stop` / `day relaunch`

- `day stop -p <target>… | --all` — terminate launches (per-platform: pkill / taskkill /
  `simctl terminate` / `am force-stop` / `aa force-stop`) and drop their sessions.
- `day relaunch -p <target>… | --all-running` — stop + rebuild + launch, recording fresh
  sessions. This is the agent's "apply my code changes" verb.

## `day mcp-server`

A Model Context Protocol server on stdio (newline-delimited JSON-RPC). Each tool call shells
back into the day CLI, so the server is transport, not logic. Tools:

| tool | wraps |
|---|---|
| `day_metadata` | `metadata --json` |
| `day_doctor` | `doctor [--toolkit …]` |
| `day_build` | `build -p … --profile …` |
| `day_launch` | `launch -p … --detach [--locale …] [--env K=V]` |
| `day_relaunch` | `relaunch` (no targets ⇒ all running sessions) |
| `day_stop` | `stop` (no targets ⇒ `--all`) |
| `day_running` | the session registry + reachability probe |
| `day_drive` | `drive -p … --steps-json …` — screenshots become MCP **image** content |
| `day_screenshot` | `drive` with `wait_idle` + `screenshot` |
| `day_lint` | `lint` |

VS Code: the Day extension registers the server automatically for Day workspaces
(`day.mcp.enabled`, default on) — agent mode then has all ten tools. Other MCP clients point
at `day --project <root> mcp-server`.

## The loop agents should follow

1. `day_metadata` → what targets/locales exist. Read the project's `AGENTS.md` (scaffolded by
   `day new`) for the page/localization/id conventions.
2. Edit code with normal file tools.
3. `day_relaunch` → compile errors come back in the result; fix; repeat.
4. `day_drive` → navigate to the changed screen, assert ids/text, `screenshot` — and **look**
   at it. On every affected target.
5. `day launch -p <target> --script scripts/walkthrough.yaml` when the change touches
   walkthrough-covered flows.

## Security posture

The engine binds loopback only and requires the per-launch token; sessions.json holds that
token, scoped to the project's own build directory. MCP clients surface tool calls for user
confirmation per their own policy (VS Code agent mode does). The VS Code extension declares
`untrustedWorkspaces: false` — none of this runs in Restricted Mode.
