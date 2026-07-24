# day-lite: dynamic miniapps on day (normative)

`day-lite` runs **miniapps** — apps written in JavaScript or TypeScript, distributed as plain
git repositories, updated remotely — on top of day's native pieces. A host app (a **superapp**)
embeds the `day-lite` crate, chooses which native capabilities to expose, and presents dynamic
apps that were never compiled: the JS layer drives the same pieces, signals, and parts a
compiled day app uses. The model generally follows the
[W3C MiniApp white paper](https://www.w3.org/TR/mini-app-white-paper/): a manifest-described
package of pages with app/page lifecycles, declared permissions, and platform-provided storage,
network, and sensor services.

`day/apps/daylite` is the reference superapp: a catalog browser that installs, updates, and
runs miniapps (see §12). Everything it does goes through the public embedding API, so other
superapps can ship different piece/part/tweak sets and different permission policies (§10).

Initial platform support is the three mobile targets (ios-uikit, android-widget, ohos-arkui);
the design has no mobile-specific dependencies, so desktop toolkits can follow later.

## 1. Architecture

```
miniapp repo (manifest.json + *.ts)      catalog.json (anywhere on the web)
        │  fetched + cached                        │
        ▼                                          ▼
┌──────────────────────────── superapp (compiled day app) ───────────────────────────┐
│  day-lite                                                                          │
│  ┌───────────┐  ┌──────────────┐  ┌───────────────────────────────────────────┐    │
│  │ loader    │→│ QuickJS       │→│ bridges                                    │    │
│  │ (fetch,   │  │ (rquickjs;    │  │  pieces DSL ─→ day-pieces dyn registry    │    │
│  │  cache,   │  │  one context  │  │  signals    ─→ day-reactive Signal        │    │
│  │  TS strip │  │  per miniapp) │  │  nav/pages  ─→ routes + stack             │    │
│  │  via oxc) │  │               │  │  parts      ─→ http / sensors / …         │    │
│  └───────────┘  └──────────────┘  │  storage    ─→ sqlite + sandboxed fs      │    │
│        ▲                          └───────────────────────────────────────────┘    │
│        │                                   ▲ every call permission-gated (§9)      │
│  package store (per-app cache + db + fs sandbox)                                   │
└────────────────────────────────────────────────────────────────────────────────────┘
```

- **Engine**: QuickJS via `rquickjs` (bytecode interpreter — no JIT, which iOS forbids
  anyway). One `Runtime` + `Context` per running miniapp, created on the main thread and only
  touched there — the same single-threaded discipline day-reactive already imposes. Async work
  (http, timers) lands back on the main thread via `Platform::post` and resolves JS promises.
- **TypeScript**: modules are type-stripped at load time with the `oxc` parser/transformer
  (`oxc_parser` + `oxc_semantic` + `oxc_transformer` + `oxc_codegen`). `.ts` files run
  directly; no build step, no decorators/JSX in v1.
- **No WebView, no HTML**: unlike WeChat-style hosts (and skip-miniapp), the UI layer *is*
  day. JS builds real pieces through a dynamic registry (§4), and reactive text/bindings run
  through real `Signal`s (§5). There is no template language, no virtual DOM, and no
  whole-state `setData` push.

## 2. Miniapp package

A miniapp is **any directory shape reachable over HTTP or the local filesystem** — by design,
a git repository checked out or served raw (GitHub's `raw.githubusercontent.com/<owner>/<repo>/<branch>/`
prefix works as-is, as does any static host or a local path during development):

```
my-weather/
├── manifest.json        # §3 — identity, pages, permissions, files
├── app.ts               # entry: App({...}) registration + page() definitions
├── pages/…              # more modules, imported relatively from app.ts
├── icon.svg             # app icon (svg or png)
├── i18n/en.ftl          # Fluent catalogs, one per locale (§7.2)
├── dayscript/smoke.yaml # scripted drive of this miniapp inside a host (§11)
└── tests/app.test.ts    # optional headless tests (§11)
```

Fetching is manifest-driven: `install(origin)` downloads `<origin>/manifest.json`, then every
file in its `files` list, into the package store. There is no zip step (the W3C packaging spec
allows a container format; day-lite treats the *repo itself* as the container). Local origins
(`/path/to/dir`) skip the cache and re-read from disk on every launch — the dev loop is
"edit, relaunch the miniapp".

## 3. Manifest

`manifest.json`, W3C MiniApp Manifest field names (snake_case), plus day extensions:

```json
{
  "app_id": "dev.daybrite.weather",
  "name": "Weather",
  "description": "Current conditions via open-meteo",
  "icons": [{ "src": "icon.svg", "sizes": "any" }],
  "version": { "code": 3, "name": "1.2.0" },
  "platform_version": { "min_code": 1 },
  "pages": ["home", "detail"],
  "req_permissions": [
    { "name": "day.permission.NETWORK", "reason": "Fetches forecasts from open-meteo.com" }
  ],
  "window": { "background_color": "#101024", "orientation": "portrait" },
  "day": {
    "entry": "app.ts",
    "files": ["app.ts", "pages/detail.ts", "icon.svg", "i18n/en.ftl"],
    "net_origins": ["https://api.open-meteo.com"]
  }
}
```

- `pages` — route ids the app must register with `page(id, builder)`; `pages[0]` is the
  launch page.
- `req_permissions` — every capability the app may use, with a human reason. The superapp
  MUST show this list before install (§9); calls to undeclared or ungranted capabilities
  reject at runtime.
- `day.entry` — the module evaluated at launch (default `app.ts`, falling back to `app.js`).
- `day.files` — the complete fetch list (everything the app needs offline). The manifest and
  entry are always fetched even if unlisted.
- `day.net_origins` — URL prefixes `day.net.fetch` may touch (W3C "domain validation").
  Empty/absent means the NETWORK permission grants nothing concrete and fetch always rejects.
- `version.code` — monotonically increasing int; the update check (§8) compares it.

## 4. Driving pieces dynamically: the dyn registry

The piece layer's API contract gains a machine-readable surface. `day-pieces` (feature
`dyn-registry`) registers every built-in piece constructor and every `Decorate` modifier in a
runtime registry keyed by name:

```rust
// day-pieces/src/dynreg.rs (feature dyn-registry)
pub enum DynValue { Null, Bool(bool), Num(f64), Str(String), List(Vec<DynValue>),
                    Map(Vec<(String, DynValue)>), Fn(DynCallback) } // DynCallback: host closure
pub struct DynPiece(/* erased piece + modifier dispatch */);
pub fn construct(name: &str, args: &[DynValue]) -> Result<DynPiece, DynError>;
impl DynPiece {
    pub fn modify(&mut self, name: &str, args: &[DynValue]) -> Result<(), DynError>;
    pub fn into_any(self) -> AnyPiece;
}
pub fn catalog() -> &'static [PieceSpec];   // introspection: names, arities, modifier lists
```

- Constructors cover the built-in vocabulary (`text`, `button`, `column`, `row`, `grid`,
  `grid_row`, `scroll`, `list`, `image`, `canvas`, `toggle`, `slider`, `text_input`,
  `spacer`, `divider`, `progress`, `when`, …). Container constructors take child `DynPiece`s.
- Modifiers cover the `Decorate` chain (`frame`, `padding`, `spacing`, `background`,
  `corner_radius`, `font`, `foreground`, `align`, `grow`, `id`, `on_tap`, `action`,
  `overlay_aligned`, `defers_system_gestures`, …). `DynValue::Fn` carries JS callbacks for
  `action`/`on_tap`/canvas draw.
- Names are **snake_case throughout** — constructors, modifiers, and string enum values
  (`label(...).font("large_title")`, `grid_align("top_leading")`) — mirroring day's Rust
  API exactly, so nothing needs re-casing between the two languages.
- `catalog()` is the introspection surface: JS generates its API (and `day lite test` its
  typings) from the same registry the bridge dispatches through, so the two cannot drift.
  Superapps that compile in extra piece crates extend the registry through the same
  `register_piece!` / `register_modifier!` macros the built-ins use — their pieces become
  scriptable with no day-lite changes (§10).

The JS side exposes each constructor as a function and each modifier as a chainable method
(`column(...)`, `.padding(12)`), generated once at context startup from `catalog()`.

## 5. Signals: one reactive system, two languages

JS signals **are** `day_reactive::Signal`s. `signal(initial)` allocates a Rust
`Signal<DynValue>` scoped to the running page; `sig.get()` reads it *through the normal
tracking path*, so when a JS closure runs inside a Rust reactive computation, its `get()`s
register dependencies exactly like Rust code:

```ts
const count = signal(0)
page('home', () =>
  column(
    text(() => `Count: ${count.get()}`),      // closure re-runs when count changes
    button('Increment').action(() => count.set(count.get() + 1)),
  ).spacing(12))
```

A `DynValue::Fn` passed where a reactive value is accepted (`text`, `bind`-style modifier
args) is wrapped in the piece layer's usual bind/watch: day-reactive re-invokes the JS
closure when its dependencies change, and only the affected piece patches. There is no
diffing and no bulk state push — the granularity is identical to a compiled day app.
`watch(fn)` and `effect(fn)` are exposed for non-UI reactions; all signal APIs are
main-thread only (enforced; calls from async callbacks are re-posted).

## 6. App shape, lifecycle, navigation

W3C-style two-level lifecycle, WeChat-compatible names:

```ts
App({
  onLaunch(opts) {},   // once, before the first page builds
  onShow() {}, onHide() {},          // foreground/background (day lifecycle phases)
  onError(err) {},
})
page('home', () => column(...))                   // builder; re-invoked per presentation
page('detail', (params) => column(...), {
  onLoad(params) {}, onReady() {}, onShow() {}, onHide() {}, onUnload() {},
})
day.nav.navigateTo('detail', { id: 7 })           // push
day.nav.navigateBack()                             // pop
day.nav.reLaunch('home')                           // reset stack
```

Pages map to a day `stack` inside the miniapp's host surface (the daylite superapp presents
that surface in a fullscreen cover with the standard X-to-exit affordance). Each page
presentation runs its builder inside a fresh reactive `Scope`; `onUnload` coincides with
scope cleanup, so signals and watches created in a page die with it.

## 7. Built-in services

All namespaced under the global `day` object (the host may alias its own brand). Promise
returns throughout; errors are typed (`PermissionError`, `NetError`, `DbError`, `FsError`).

- **`day.net.fetch(url, opts?)`** → `Promise<{ok, status, headers, text(), json()}>` —
  bridged to `day-part-http`'s async fetch; gated by `NETWORK` + `day.net_origins`.
- **`day.db`** — sqlite, always available (STORAGE is a default-granted permission):
  `db.migrate([...ddl])` (§7.1), `db.exec(sql, params?)` → `{changes, lastInsertRowId}`,
  `db.query(sql, params?)` → `[{col: value}]`. One database per app id.
- **`day.fs`** — OPFS-shaped sandboxed filesystem (same contract skip-miniapp validated):
  `day.fs.root.getFileHandle(name, {create})` / `.getDirectoryHandle(...)` / `.entries()` /
  `.removeEntry(name, {recursive})`; file handles `read()`, `write(data)`, `remove()`,
  `size`. Paths are confined to the app's sandbox dir; `..`, absolute paths, and escapes
  reject with `SecurityError`.
- **`day.sensors`** — bridged to `day-part-sensors` where compiled in; gated by `SENSORS`.
- **`day.prefs`** — small KV (`get/set/remove`), bridged to `day-part-prefs`, app-scoped.
- **`day.sys.info()`** → `{platform, appId, version, locale, host}`.
- **`day.i18n.t(key, args?)`** (global shorthand `t(...)`) — Fluent localization (§7.2).
- **Timers/console**: `setTimeout/setInterval/clearTimeout/clearInterval` (Platform::post
  driven), `console.log/warn/error` → the host log (visible in `day launch` output).

### 7.2 Fluent localization

Miniapps ship [Fluent](https://projectfluent.org) catalogs at the standardized location
`i18n/<locale>.ftl` (listed in `day.files` like any other package file). `t(key, args?)`
formats through a per-app `FluentBundle` for the RUN's locale — `day launch --locale` and
`day lite test` deliver it via `DAY_LOCALE`, else day's live locale signal applies, so a
`set_locale` in the host re-renders miniapp text reactively. Resolution falls back
`zh-CN → zh → en`, and a missing key returns the key itself (an unlocalized app keeps
working, visibly). Bidi isolates are stripped from output exactly as day-l10n does for
compiled apps, so scripted `assert_text` matches the authored string. The bundled samples
each carry `en`, `fr`, `ar`, and `zh-CN` catalogs.

### 7.1 sqlite migrations

`db.migrate(migrations: string[])` is the schema contract: an append-only array of DDL
scripts. Position *n* runs exactly once, tracked in sqlite's `user_version` pragma; on
launch the app calls `migrate` with its full history and day-lite applies the tail. Editing
history instead of appending is an error (a recorded hash per step catches it).

## 8. Install, update, catalog

The package store keeps, per app id: the manifest, fetched files, the sqlite db, the fs
sandbox, and install metadata (origin, granted permissions, installed `version.code`, file
hashes). Superapp-facing API (all async):

```rust
lite::store::install(origin: &str) -> InstallPlan   // fetched manifest + permission list, NOT yet installed
InstallPlan::confirm(granted: &[Permission]) -> Installed   // fetch files, persist
lite::store::check_update(app_id) -> Option<UpdatePlan>     // refetch manifest, version.code compare
UpdatePlan::apply()                                          // fetch changed files (hash-diff), atomic swap
lite::store::remove(app_id)                                  // db + fs + cache gone
```

The two-step install is what makes permission disclosure structural: the UI cannot install
without passing the granted set through `confirm`. A **catalog** is any JSON document listing
origins — the superapp renders it, but installing an entry goes through the same
`install(origin)`:

```json
{ "apps": [ { "app_id": "dev.daybrite.weather", "name": "Weather",
              "description": "…", "icon": "https://…/icon.svg",
              "origin": "https://raw.githubusercontent.com/daybrite/miniapp-weather/main",
              "version": { "code": 3, "name": "1.2.0" },
              "req_permissions": [ { "name": "day.permission.NETWORK", "reason": "…" } ] } ] }
```

Catalog entries duplicate the permission list so disclosure can render before any fetch; at
install time the fetched manifest is the source of truth and a mismatch surfaces in the UI.
Updates re-run disclosure only when the permission set grew.

## 9. Permissions

Permission ids are plain strings (`day.permission.NETWORK`, `.SENSORS`, `.FS`, `.STORAGE`,
`.PREFS`). The mapping is: **each bridge module declares the permission it requires**;
day-lite installs a bridge into a context only if the manifest declares it AND the user
granted it. Ungranted calls don't half-work: the namespace exists but every entry point
rejects with `PermissionError` (so feature detection is `day.can('NETWORK')`, not
try/catch-shaped guessing). `STORAGE` (sqlite) and `PREFS` are granted implicitly at install
— they touch only app-private data — but still must be declared to be visible in disclosure.
Hosts can reclassify (a kiosk superapp may implicit-grant nothing) and define new permission
ids for their own bridges (§10). Grants persist in the package store and are revocable from
the superapp's app-detail UI.

## 10. Embedding in other superapps

The whole system is a library; `apps/daylite` holds no privileged code:

```rust
let host = day_lite::Host::builder()
    .store(day_lite::Store::at(data_dir))            // package store root
    .bridge(day_lite::bridges::net())                 // day-part-http, wants NETWORK
    .bridge(day_lite::bridges::sensors())             // day-part-sensors, wants SENSORS
    .bridge(my_crate::payments_bridge())              // custom part, custom permission id
    .implicit(&["day.permission.STORAGE", "day.permission.PREFS"])
    .build();
let surface: AnyPiece = host.launch(app_id)?;         // the miniapp's UI, place it anywhere
```

A bridge is `{ namespace, permission, install(ctx, services) }` — the same seam the
built-ins use. Pieces compiled into the host (any crate using `register_piece!`) are
automatically scriptable (§4). The daylite superapp is this builder plus catalog UI.

## 11. Testing: `day lite test`

Headless unit tests ship inside the miniapp (`tests/*.test.ts`):

```ts
import { totalFor } from '../app.ts'
test('sums open todos', () => { expect(totalFor([{done:false},{done:true}])).toBe(1) })
test('db roundtrip', async () => {
  await day.db.exec('insert into todos(title) values (?)', ['x'])
  expect((await day.db.query('select count(*) n from todos'))[0].n).toBe(1)
})
```

Because miniapp pieces are real day pieces in the host's tree, **dayscript drives them
like any compiled UI**: element ids assigned in JS (`.id("ttt-cell-0")`) are tappable,
assertable, and screenshot-able through the ordinary engine. The convention is a
`dayscript/` directory in the miniapp repo whose flows run against the reference host:

```sh
cd apps/daylite
day launch -p ios-uikit --env DAYLITE_RESET=1 \
  --script miniapps/tictactoe/dayscript/smoke.yaml     # install → open → play → screenshot
day launch -p ios-uikit --locale fr --variant fr --env DAYLITE_RESET=1 \
  --script dayscript/fr.yaml                            # localized-run screenshots per locale
```

`DAYLITE_RESET=1` starts the host from an empty store so install flows are reproducible;
`--variant` files each locale's screenshots separately, mirroring the showcase galleries.

`day lite test <path>` (path = miniapp dir, default `.`) runs every test module in a fresh
context wired to the **day-mock toolkit**: pieces construct and patch for real (assertable
via ids), sqlite/fs run against a temp sandbox, `day.net.fetch` requires an explicit
`mock.net.route(url, reply)` (network is never live in tests). `test()`, `expect().toBe/
toEqual/toContain/toThrow`, async tests, and `beforeEach` are provided by the runner. Exit
code 5 on failure, mirroring dayscript. The same runner backs miniapp CI (a plain
`day lite test` in the repo's workflow).

## 12. The daylite superapp (`day/apps/daylite`)

Reference host, three surfaces:

1. **Catalog** — renders a catalog JSON (default: the daybrite samples catalog; overridable
   in settings). Each entry: icon, name, description, version, and the permission list with
   reasons. Install → disclosure sheet → `confirm`.
2. **My apps** — installed grid (tile per app), update-available badges (`check_update` on
   launch), open / update / revoke-permission / uninstall.
3. **Add by URL** — paste any origin (https prefix or local path) to install straight from a
   repo; this is also the dev loop for miniapp authors (point it at a working tree, edits
   apply on next launch).

Running apps present in a fullscreen cover (docs/cover.md) with the X affordance;
`window.background_color` seeds the cover background.

## 13. Build integration

`rquickjs` (bundled quickjs C) and rusqlite's bundled sqlite3 cross-compile through each
platform's NDK. The `day` CLI supplies, per target, what their build scripts need — the
same env it already curates for `cc`:

- iOS: the `bindgen` feature (rquickjs ships no prebuilt bindings for `aarch64-apple-ios-sim`).
- Android/OHOS additionally: `BINDGEN_EXTRA_CLANG_ARGS_<target>` = `--sysroot=<ndk sysroot>
  --target=<triple>` (bindgen's libclang doesn't inherit the CC sysroot) and
  `AR_<target>` = the NDK's `llvm-ar` (the host Darwin `ar` silently produces an empty
  archive → `JS_Call` undefined at link).

## 14. Security considerations

- Miniapp JS is untrusted: no filesystem outside the sandbox, no process/env access, no
  dynamic native loading. The bridge surface is the *entire* capability set.
- Network is doubly confined: permission + `net_origins` prefix allow-list.
- Remote code is cached then executed: installs record per-file hashes; the update path
  re-verifies before swap. Origins are https-or-local only.
- A miniapp's context is dropped on exit; runaway scripts are bounded by QuickJS's
  interrupt handler (a main-thread watchdog cancels evaluation after a budget).
- Superapps choose their exposure: an app compiled without a sensors bridge simply has no
  sensors capability to grant.
