# Daylite

The reference [day-lite](../../docs/lite.md) **superapp**: browse a catalog of JS/TS
miniapps, install them with permission disclosure, keep them updated, and run them in a
fullscreen cover — every miniapp drives real native day pieces through QuickJS, with no
WebView and no build step (TypeScript is stripped at load).

## Run it

```sh
day launch -p ios-uikit          # or android-mdc / ohos-arkui
```

The three bundled samples (Weather, Todos, Tic-Tac-Toe) appear in the catalog and install
through the normal disclosure flow, offline. "Add from URL" installs any miniapp served
from a static host — a raw git branch URL like
`https://raw.githubusercontent.com/<owner>/<repo>/main` works directly, which is also the
remote-update channel (version bumps in `manifest.json` surface as updates).

## The samples (`miniapps/`)

Each is a self-contained miniapp repo — `manifest.json`, TypeScript sources, Fluent
catalogs in `i18n/{en,fr,ar,zh-CN}.ftl`, headless tests in `tests/`, and a scripted drive
in `dayscript/`:

- **weather** — open-meteo over `day.net.fetch` (NETWORK permission + `net_origins`).
- **todo** — sqlite persistence + search (`day.db` with append-only migrations).
- **tictactoe** — grid UI, one string signal for the whole board, score in `day.prefs`.

```sh
day lite test miniapps/todo                      # the miniapp's own tests, headless
day launch -p ios-uikit --env DAYLITE_RESET=1 \
  --script miniapps/tictactoe/dayscript/smoke.yaml   # scripted drive + screenshot
day launch -p ios-uikit --locale fr --variant fr --env DAYLITE_RESET=1 \
  --script dayscript/fr.yaml                          # localized-run check
```

`DAYLITE_RESET=1` starts from an empty store so install flows are reproducible; without it
installed apps and their data persist across launches.

## Known issues

- **ohos-arkui**: builds (the whole rquickjs/oxc/rusqlite stack cross-compiles — see
  docs/lite.md §13); an on-emulator run is still pending local-emulator time.

Two earlier issues in this list are FIXED in day itself (see docs/cover.md's delivery
contract): re-opening a miniapp after closing one no longer breaks — the process-global
locale signal no longer dies with the first cover's scope (`Signal::global`), and Android's
cover shell is no longer re-parented/clobbered by day-tree layout while presented.
