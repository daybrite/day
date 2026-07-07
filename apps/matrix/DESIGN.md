# Day Matrix Client — DESIGN & PROGRESS

A full-featured Matrix chat client built on the `day` native-UI framework, using
[matrix-rust-sdk](https://github.com/matrix-org/matrix-rust-sdk) for all protocol + E2E encryption.
Targets every Day backend: macOS (AppKit), Linux (GTK, Qt), Windows (WinUI), iOS (UIKit),
Android (widget/JNI), HarmonyOS (ArkUI).

This file is the source of truth for the plan and survives context compaction. Update the
**Progress log** at the bottom as work lands.

## Architecture

```
┌─────────────────────────── main thread (Day) ───────────────────────────┐
│  Day reactive Signals (room list, timeline, session state)  ← UI reads   │
│         ▲ on_main(|| signal.set(..))          spawn(async ..) ▼           │
└─────────┼─────────────────────────────────────────────────────┼─────────┘
          │                                                       │
┌─────────┴───────────────── tokio multi-thread rt ──────────────┴─────────┐
│  matrix-sdk Client · SyncService · RoomListService · Timeline · media    │
└──────────────────────────────────────────────────────────────────────────┘
```

- **Bridge primitive:** `day_reactive::on_main(FnOnce + Send)` marshals SDK results onto the main
  thread where Day's (thread-local, `!Send`) Signals live. `install_main_poster` is wired by each
  toolkit's launch. Background→UI is `on_main`; UI→background is `runtime.spawn(...)`.
- **`matrix-core` crate** (headless, no UI): owns the tokio runtime + the `matrix_sdk::Client`,
  exposes an app-facing state API. UI actions call sync-looking methods that spawn async work;
  state changes arrive back as Signal updates via `on_main`. Keeps ALL matrix-sdk types out of the
  UI layer (the UI sees plain structs: `RoomSummary`, `TimelineRow`, etc.).
- **E2E:** automatic once the `sqlite` store + `e2e-encryption` feature are enabled and a store path
  is set. Device verification is auto-accepted / deferred for v1.
- **TLS:** `rustls-tls` (not native-tls) for clean cross-compilation to iOS/Android/OHOS.
- **Store:** `sqlite_store(path, passphrase)` under the app's data dir (per platform).

## Crate / module layout

```
apps/matrix/
  day.yaml            # all 7 targets
  Cargo.toml          # depends on day, matrix-core, and any new day-piece-* we build
  src/main.rs         # day::launch(root)
  src/lib.rs          # root() + app shell (nav)
  src/screens/        # login, room_list, timeline, composer, room_detail
crates/matrix-core/   # SDK + tokio runtime + Day signal bridge (headless)
pieces/day-piece-*    # any NEW UI primitives we must build (see below)
```

## Day API facts (verified — build the UI against THESE)

- `Decorate` trait methods (chainable on any piece → `AnyPiece`): `.id(s)`, `.id_keyed(prefix,key)`,
  `.padding(impl IntoInsets)`, `.frame(w,h)`, `.on_tap(fn)`, `.context_menu(items)`, `.on_drag(fn)`,
  `.any()`. `Insets::all(f64)`/`::symmetric?`(check)/`::ZERO`; `f64` coerces via `IntoInsets`.
- `Label`: `.font(Font::…)`, `.weight(FontWeight::…)`, `.bold()`, `.italic()`, `.color(Color)`.
  Text source: `&str`/`String`/`Signal<String>`/`Fn()->String`.
- `Color::hex(0xRRGGBB)`, `Color::rgb(f,f,f)`, `Color::rgba(..)`, consts `BLACK/WHITE/CLEAR`.
- Layout fill = per-node `Flex { grow_w, grow_h }`. `text_field`, `list`, `scroll`, `spacer` grow by
  default; **there is NO public `.grow()`** to make an arbitrary piece fill — MUST ADD (sets Flex;
  day-core/pieces-only, no backend work).
- Reactive switch = only `when(cond_fn, build_fn)` (shows arm when true). Stack `.any()` arms in a
  column for multi-way. `bind(src_fn, react_fn)` for effects. `stack(path_sig, root).destination(|id|…)`
  for mobile push/pop. `list(items_fn, key_of, |slot| row).row_height(RowHeight::Automatic).on_select(|k|…)`;
  `ItemSlot::get()` returns the item (Clone, tracked).
- **NO `.background(Color)` / `.corner_radius(f64)`** on pieces — CRITICAL gap for chat bubbles,
  avatars, badges, cards. MUST ADD (core Decorate + per-backend native view bg + rounded clip).

## Missing Day primitives (fill in from inventory, build in parallel)

CONFIRMED to build (the UI depends on them):
- **remote-image** piece (avatars/media bytes → native image) — WORKFLOW RUNNING.
- **textarea** piece (multiline composer) — WORKFLOW RUNNING.
- **list `.scroll_to_end(Trigger)`** (timeline stick-to-bottom) — WORKFLOW RUNNING.
- **`.background(Color)` + `.corner_radius(f64)`** Decorate additions (bubbles/avatars/badges/cards) —
  TODO, second core workflow (after the first finishes, to avoid core-file edit conflicts).
- **`.grow()` / `.grow_w()` / `.grow_h()`** Decorate (sets Flex; fill panes) — TODO with background.
- src/ui.rs is a DRAFT written against these not-yet-existing APIs; it compiles only once they land.


Candidates a chat client needs that Day may lack — confirm against the inventory:
- **Async/remote image** — display an image from a URL or mxc bytes fetched off-thread (avatars,
  media). CRITICAL. Likely a new `day-piece-remote-image` (or extend the core image with a bytes
  source + async loader).
- **Multi-line text editor** — the composer; a growing multi-line input if `text_field` is single-line.
- **Sectioned / variable-height list + scroll-to-bottom** — the timeline (day separators, mixed cell
  heights, stick-to-bottom on new message, back-pagination on scroll-to-top).
- **Form** — labeled field rows + sections (login, settings).
- **Grid / collection view** — media galleries, emoji/reaction pickers (nice-to-have).

## Screens

1. **Login** — homeserver URL, username, password; loading + error states; restores a saved session.
2. **Room list** — avatar, display name, last-message preview, timestamp, unread/notification badge;
   search; sorted by recency.
3. **Timeline** — message bubbles (own vs other), sender name + avatar, images/media, reactions,
   day separators, read state, back-pagination; sticky-to-bottom.
4. **Composer** — multi-line input, send button, (later: attachments, emoji).
5. **Room detail** — topic, members, settings.
6. **Shell** — sidebar(rooms)+detail(timeline) split on desktop; stack (list→room) on mobile.

## Platform matrix (locally buildable on this Mac)

| Target | Build | Run locally | SDK cross-compile risk |
|---|---|---|---|
| macos-appkit | ✓ | ✓ (primary dev) | none (host) |
| macos-gtk / macos-qt | ✓ | ✓ | none (host) |
| ios-uikit | via xcodebuild | ✓ (simulator) | **rustls/sqlite for aarch64-apple-ios-sim** |
| android-widget | via gradle | ✓ (emulator) | **rustls/sqlite for aarch64-linux-android (NDK)** |
| ohos-arkui | env-gated | ✗ (needs DevEco/Huawei) | **ohos target** |
| windows-winui | ✗ on Mac | ✗ | — |

"Locally-available platforms" = appkit, gtk, qt, ios-sim, android-emu. OHOS/WinUI are build-checked
where possible but not runnable here.

## Build config (REQUIRED — hard-won)

- `matrix-sdk = { version = "0.14", default-features = false, features = ["e2e-encryption", "sqlite", "rustls-tls"] }`
  + `matrix-sdk-ui = { version = "0.14", default-features = false, features = ["rustls-tls"] }`.
- **TLS = rustls (ring backend)** — cross-compiles to iOS/Android cleanly (NOT native-tls/aws-lc).
- **`libsqlite3-sys = { version = "0.35", features = ["bundled"] }`** as a DIRECT dep — forces static
  SQLite compiled from C source. WITHOUT this, Android fails to link (`ld.lld: unable to find -lsqlite3`;
  the NDK sysroot has no system sqlite — iOS's SDK does, so iOS links either way). Feature-unifies the
  `bundled` flag onto matrix-sdk-sqlite's sqlite.
- **Android NDK env for cargo cross-compile** (NDK 28.2.13676358, prebuilt `darwin-x86_64`):
  `CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER` / `CC_aarch64_linux_android` / `AR_aarch64_linux_android`
  = the `.../bin/aarch64-linux-android24-clang` (+ `llvm-ar`). day's gradle pipeline sets ANDROID_NDK_HOME.
- SDK is tokio-based; store = `Client::builder().sqlite_store(dir, Some(passphrase))`. E2E automatic
  with sqlite+e2e-encryption. Cheat-sheet at `docs/matrix-sdk-0.14-cheatsheet.md` (exact 0.14 API;
  note MediaRequestParameters, nested TimelineItemContent::MsgLike, restore_session two-arg form,
  RoomList needs filter+add_one_page).

## Bridge pattern (matrix-core, the on_main + Send constraint)

`on_main` requires `FnOnce + Send`, but Day Signals are `!Send`. So a tokio task must NOT capture a
Signal. Pattern: matrix-core exposes `state() -> MatrixState` (all-`Signal` struct, `Copy`, lazily
created in a MAIN-THREAD `thread_local`). A tokio task captures only Send DATA and calls
`on_main(move || matrix_core::state().rooms.set(data))` — `state()` is looked up *inside* the closure
on the main thread, so the closure stays Send. NEVER call `state()` off the main thread.

## Local test homeserver (Conduit in Docker) — for demoing the app

- Run: `docker run -d --name conduit -p 6167:6167 -e CONDUIT_CONFIG='' -e CONDUIT_SERVER_NAME=localhost
  -e CONDUIT_ALLOW_REGISTRATION=true -e CONDUIT_ALLOW_ENCRYPTION=true -e CONDUIT_DATABASE_PATH=/var/lib/matrix-conduit
  -e CONDUIT_DATABASE_BACKEND=rocksdb -e CONDUIT_PORT=6167 -e CONDUIT_ADDRESS=0.0.0.0 matrixconduit/matrix-conduit:latest`
  (CONDUIT_CONFIG='' is REQUIRED — else it panics wanting a toml.)
- **Homeserver URL: `http://localhost:6167`** (desktop/iOS-sim). Android emulator must use **`http://10.0.2.2:6167`**.
- Test accounts: **`@alice:localhost` / `alicepass123`**, **`@bob:localhost` / `bobpass123`**.
- Seeded: bob created "Day Test Room", invited+ joined alice, sent 2 messages. Log in as alice → the
  room + bob's messages appear. Re-seed via the curl register/createRoom/join/send calls if the
  container is recreated. SDK allows http for localhost.

## New pieces / primitives — landed APIs (build the UI against these)

- `day_piece_remote_image::remote_image(Signal<Option<Arc<Vec<u8>>>>)` `.circle()`/`.rounded(r)`
  /`.content_mode(ContentMode::{Fill,Fit})`/`.placeholder_color(Color)` — a grow leaf; constrain with
  `.frame(w,h)`. For avatars/media bytes (app fetches mxc bytes → set the Signal).
- `day_piece_textarea::text_area(Signal<String>)` `.placeholder(t)`/`.min_lines(u32)`/`.max_lines(u32)`
  — multiline composer, two-way.
- `list(...).scroll_to_end(Trigger)` + `.stick_to_bottom(bool)` — timeline stick-to-bottom.
- (LANDING) `Decorate::background(Color)` + `.corner_radius(f64)` + `.grow()/.grow_w()/.grow_h()`.

## Working-app learnings (AppKit, verified)

- **FUNCTIONAL CLIENT WORKS end-to-end on AppKit against Conduit**: login (@alice) → sync → room list
  ("Day Test Room", "localhost Admin Room (2)") → open room → back-paginate → render bob's + alice's
  messages (sender, time, date dividers) → compose + **send** ("Hello from the Day Matrix client!")
  appears live. Script `scripts/full-demo.yaml` = 13/13.
- **`.grow()` is REQUIRED down the whole container chain** or `list`/`scroll` collapse to 0 height
  (header shows, list invisible). Grow root column → when-arm containers → the list itself.
- **List rows need `.on_tap(handler)` for taps** — dayscript `tap`/synthetic taps hit the row's inner
  content, NOT the native table row-selection, so `list.on_select` doesn't fire from a scripted tap.
  Put `.on_tap(move || open_room(id))` on the row (also better for real users).
- **Back-pagination is needed to show history**: `matrix_sdk_ui::Timeline::subscribe()` returns only
  the live window (post-join); older messages (e.g. sent before you joined) require
  `timeline.paginate_backwards(50)`. matrix-core now auto-paginates once on room open.
- **day-cli does NOT forward the launched app's stderr** — diagnose via a file
  (`matrix_core::diag()` → `~/.daybrite-matrix/diag.log`), NOT eprintln/tracing-to-stderr.
- **day launch uses its OWN target dir** (`build/day/cargo/<combo>/`), separate from `cargo build`'s.
  A plain `cargo build` won't update what `day launch` runs; edit source and let `day launch` rebuild.
- The reactive bridge is SOUND: diag confirmed `on_main FIRED` sets signals on the main thread and
  status/list both re-render. (Detached-scope signals in matrix-core::state() are fully reactive.)
- **TODO before shipping**: remove the temporary `diag()` calls + the `[ui]` items-closure log.

## Remaining (polish + platforms)

- POLISH (primitives now exist): message bubbles (`.background().corner_radius()`, own-vs-other
  alignment), avatars via `remote_image` (matrix-core must fetch mxc avatar bytes into RoomSummary/
  TimelineRow), multiline composer via `text_area`, `list.scroll_to_end` on new message, unread
  badges, desktop sidebar+detail split (row: rooms 320w + divider + timeline.grow), empty/loading
  states, dark-mode colors. The polished draft lives in `src/ui.rs` (needs API reconciliation:
  `.frame_width` doesn't exist → use `.frame`/grow; verify `Insets::symmetric`).
- CROSS-PLATFORM: gtk, qt (desktop — should mirror appkit), then ios-uikit (sim; homeserver
  `http://localhost:6167` reachable) + android-widget (emulator; homeserver `http://10.0.2.2:6167` +
  store_dir must use the app sandbox, NOT $HOME — fix `store_dir()` for ios/android). ohos/winui
  build-check only.

## Platform status (functional flow via scripts/full-demo.yaml)

- **macos-appkit: 13/13 ✓** (login→rooms→open→history→send, real messages rendered).
- **macos-gtk: 13/13 ✓** (same, messages render; window top strip is GTK chrome).
- **ios-uikit: 13/13 ✓** (simulator) — login (@alice) → sync → room list with REAL names
  ("Day Test Room", "localhost Admin Room") → tap `room-day-test-room` → timeline (message bubbles,
  date dividers, sender grouping, composer) → **send** ("Hello from the Day Matrix client!",
  confirmed server-side, fresh timestamp). Fixes applied for the mobile bring-up:
  - **Mobile entry points**: `src/lib.rs` was missing `day::ios_main!("Matrix", root);` +
    `day::android_main!(root);` (matrix is hand-authored, not `day create`d) — added at crate root.
  - **iOS platform scaffold**: matrix had no `platform/` dir. Created `platform/ios/{Runner/main.swift,
    Runner/Info.plist, DayApp.xcodeproj/project.pbxproj}` modeled on showcase (PRODUCT_NAME=Matrix,
    bundle id dev.daybrite.matrix, `-lmatrix`, showcase-only `hello.json` removed; keeps the always-
    generated DayPieces local SwiftPM ref — the two pieces are pure-objc2 Rust with no Swift shims).
  - **day-cli fix (general)**: `mobile.rs::build_ios` hardcoded the product as `Showcase.app`; now
    globs the single `.app` in the products dir so any app name works (showcase still finds it).
  - **ATS / cleartext**: `Info.plist` adds `NSAppTransportSecurity → NSAllowsLocalNetworking` so the
    sim can reach `http://localhost:6167`.
  - **Store dir**: iOS `$HOME/Library/Application Support/daybrite-matrix` (the sandbox container root
    isn't writable; Application Support is). Homeserver default = `http://localhost:6167` (sim shares
    the host loopback).
  - **Room name race FIXED (all platforms)**: the room-list stream yields only on *structural*
    changes, so a room surfaces before its `m.room.name`/heroes state syncs (shows "Empty Room"/id).
    `matrix-core` now (a) `resolved_name()` returns `Some` only for a genuine Named/Aliased/Calculated
    name (never the `RoomDisplayName::Empty` "Empty Room" literal), (b) `run_room_list` re-summarizes
    on a 1s interval AND **defers the first render** until every room resolves (≤6s grace, then id-
    localpart fallback). Essential because the native `list` builds a row once per stable key (room id)
    and does NOT rebuild it when only the name changes — so the row must be born with its final name +
    name-derived script id. (Keying the list by id+name to force a rebuild was tried and REVERTED: the
    native iOS list left a stale duplicate row on key change.) The colon-less `!hash` room ids this
    Conduit uses are handled by `name_fallback`.
- **android-widget: 13/13 ✓** (emulator) — same flow, real room names → open → timeline → send
  (confirmed server-side, fresh timestamp). Mobile bring-up fixes on top of the shared ones above:
  - **Store dir (sandbox)**: there is no usable `$HOME` on Android. `matrix-core` reaches the app's
    files dir (`/data/data/dev.daybrite.matrix/files/daybrite-matrix`) via day-android's JNI bridge —
    added `DayBridge.filesDirPath()` (Context.getFilesDir) mirroring the existing `cacheDirPath()`, and
    `matrix-core` gained a `[target.'cfg(target_os="android")'.dependencies] day-android` dep to call
    `with_env`. **Resolved ONCE on the main/UI thread in `init()` and cached** (`ANDROID_STORE`): a JNI
    `FindClass` from a spawned tokio thread uses the *system* classloader (no app classes) →
    `ClassNotFoundException`. `store_dir()` (run on tokio threads) reads the cache, never the JNI.
  - **Homeserver default**: `#[cfg(target_os="android")]` → `http://10.0.2.2:6167` (the emulator's own
    `localhost` is the device; 10.0.2.2 is the host loopback). Others → `http://localhost:6167`.
  - **Cleartext**: `AndroidManifest.xml` sets `android:usesCleartextTraffic="true"` (Android blocks
    cleartext by default on API 28+).
  - **Android platform scaffold**: created `platform/android/` (settings/build.gradle.kts,
    gradle.properties, app/build.gradle.kts, AndroidManifest.xml) modeled on showcase — namespace/
    applicationId `dev.daybrite.matrix`, `<meta-data day.lib = "matrix">` (loads `libmatrix.so`),
    launcher `dev.daybrite.day.bridge.DayActivity`. The generic day-pieces.json/manifest-overlay
    plumbing stages the two pieces' Android Java factories with no scaffold edits.
- **macos-qt: 13/13 ✓** — the earlier 10/13 "Empty Room" failure was the room-name race; the
  matrix-core fix above resolves it (real names → `tap room-day-test-room` succeeds → send).
- **macos-appkit re-verified 13/13 ✓** after the matrix-core changes (no regression); macos-gtk
  unaffected (matrix-core changes are cross-platform).
- **ohos-arkui / windows-winui**: not attempted here — winui needs Windows, and ohos would need its
  own `platform/` scaffold + an ohos entry macro + an OHOS matrix-sdk cross-compile (beyond the
  "don't spend long" build-check).

## Progress log

- (init) Design written. matrix-sdk 0.14 dep graph resolves (398 crates). Bridge = `on_main`.
- **DE-RISK PASSED**: matrix-sdk 0.14 builds on host + iOS-sim + Android (bundled sqlite fix). App
  skeleton (own workspace, `exclude`d from day) builds+launches placeholder on AppKit. Inventory done
  (missing: async image, multiline editor, list scroll-to-end — being built by a 3-agent workflow).
  SDK cheat-sheet saved. NEXT: matrix-core bridge + login/room-list/timeline screens.
- **POLISH LANDED (macos-appkit, 13/13)**: rewrote `src/lib.rs` into a polished native chat UI —
  centered login card; room list with colored initial-disc avatars + semibold name + unread badge
  pill; message-bubble timeline (own = right blue/white, other = left gray, grouped by consecutive
  sender with avatar + colored sender name + timestamp on the group head); date dividers; `text_area`
  multi-line composer (ids `composer-input`/`composer-send` preserved) at the bottom; auto
  scroll-to-bottom via a `Trigger` driven by `bind` on timeline length + `stick_to_bottom`; desktop
  sidebar(320w)+detail split with a vertical rule and a "Select a chat" empty state (mobile keeps the
  single-pane when()-toggle with a back button). Deleted the stale `src/ui.rs` draft.
  - **matrix-core additions** (public API preserved): `TimelineRow::Message.head: bool` (sender-run
    grouping, computed in `map_timeline`); `RoomSummary.avatar` now populated via a new cached
    `room_avatar()` (`room.avatar(MediaFormat::File)`, only when `avatar_url()` is set) — UI renders
    it with `remote_image(...).circle()`, falling back to the initial disc (the Conduit test rooms
    have no avatar, so the disc shows). `diag()` is now gated behind the `DAY_MATRIX_DIAG` env var
    (no-op by default); the `[ui]` items-closure logs are gone.
  - **Framework enablers**: added single-axis `Decorate::width(f64)`/`height(f64)` (layout-only,
    `FrameLayout` with one axis `None`) for the fixed-width sidebar; added an env-gated window
    appearance override in day-appkit (`DAY_APPEARANCE=light|dark` → `NSWindow.setAppearance`; unset =
    follow system). matrix's `main.rs` sets `DAY_APPEARANCE=light` so native controls (list, fields,
    composer) match the light palette instead of the host's dark mode.
  - **Layout gotcha (recorded)**: `.grow()` must be the OUTERMOST wrapper of any pane placed in a
    flex parent — `.background()`/`.corner_radius()` wrap in a `Flex::default` container, so
    `.grow().background()` strips the grow flex and the parent hugs it (timeline list collapses to 0h,
    composer jumps under the header). Use `.background(c).grow()`. Centering a fixed-width child needs
    real spacers (`row((spacer(), card, spacer()))`), not `column(...).align(Center)` — `background`'s
    PassThrough places the column at its intrinsic (hugged) width, so `align` has no slack.
