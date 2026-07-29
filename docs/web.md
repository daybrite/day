# Web: the web-dom backend (§9)

Day's ninth backend renders in a browser. The DOM is the toolkit: a Day `button` is a real
`<button>`, a slider is `<input type="range">`, a dialog is `<dialog>` — semantic HTML plus ARIA,
never widgets painted onto a canvas. The backend is `toolkits/day-dom`, the target name is
`web-dom`, and the feature is `dom`. Status: **experimental** — the capability subset below is
real, and gaps are listed, not hidden.

```bash
rustup target add wasm32-unknown-unknown
day build  -p web-dom     # wasm cdylib + host page → build/day/cargo/web-dom/<profile>/dist/
day launch -p web-dom     # build, serve dist/ on 127.0.0.1, open the default browser
day launch -p web-dom --locale ar    # locale rides as ?locale= on the URL
```

## Architecture

The same trampoline shape as the Android and HarmonyOS backends, with JavaScript in place of
Java/C. A plain ES-module shim owns every real DOM call; Rust holds numeric element ids and
crosses the boundary with `extern "C"` imports. No wasm-bindgen, no bundler, no npm.

```
index.html                        app.wasm (Rust cdylib)
  #day-root                        day-dom   (Toolkit/Platform over the shim imports)
  shim.js  ──instantiate──▶        day-core / day-pieces / day-fluent
     │  env.day_dom_create/insert/set_frame/…   (imports: the DOM verbs)
     └── wasm.day_dom_main/event/posted/frame/… (exports: entry + callbacks)
```

- **`toolkits/day-dom/src/lib.rs`** — the `Toolkit`/`Platform` impl. Pieces map to elements
  (`<div>`, `<button>`, `<input>`, `<select>`, `<progress>`, `<img>`, `<canvas>`, `<dialog>`…);
  layout stays day-core-owned `position:absolute` frames, with two exceptions below.
- **`crates/day-cli/resources/web/`** — `shim.js` (the DOM half: element table, event dispatch,
  canvas replay, dialogs, text measurement), `day.css` (control styling, light + dark via CSS
  custom properties), `index.html` (fetches and instantiates the wasm). The trio lives in the CLI
  rather than beside the toolkit because `day build` embeds it with `include_str!` — so an
  installed CLI needs no source checkout — and `include_str!` may not reach outside its own
  package. **Editing shim.js means rebuilding the CLI** before the change reaches a served page.
- **`day::web_main!(root)`** — exports `day_dom_main`, which `shim.js` calls once the module is
  instantiated. Emits nothing off wasm32, like the other entry macros (§17.4).

Two places where the browser, not day-core, owns geometry:

- **Scrolling** maps to `overflow:auto` containers (DP-8's hybrid, as proposed): day children
  are absolutely placed inside a sized content `<div>`, the browser scrolls it natively.
- **Nav and tab panes** are CSS-framed (flex split view, stacked pages); each pane reports its
  size back through a ResizeObserver as `Event::FrameChanged` — the DayNavPage contract
  (docs/navigation.md).

Synchronous text measurement — the one duty a browser makes hard — uses a hidden measurement
element (so wrapping matches real labels), cached per element and invalidated on text or font
patches.

## The main loop, timers, and `day::sleep`

The browser owns the loop; wasm has one thread and no `std::thread`, no `Instant`, no
`SystemTime`, no process environment. Three seams make Day code run unchanged:

- `Platform::post` queues a microtask; `Platform::request_frame` is `requestAnimationFrame`
  (animation clocks tick per frame, and CSS transitions carry opacity/transform/color).
- `Platform::post_delayed(ms, f)` (new with this backend, default = thread + sleep on native)
  is `setTimeout` here. It backs **`day::sleep(ms)`**, the awaitable timer for
  `day::task` flows — use it instead of `std::thread::sleep` for fake-work delays and it works
  on every backend (docs/async.md).
- The launch locale reaches localization through `set_launch_locale` (the `?locale=` query
  parameter, else the browser's language list) because there is no `DAY_LOCALE` environment
  variable to read.

## Capabilities

What the subset covers today: containers, labels, buttons, toggles, sliders, text fields and
areas, all three picker styles, progress and spinners, images, dividers, scrolling, canvas
(shapes, gradients, text, transforms — replayed onto `<canvas>` 2D), split + stack navigation
with back bar, tabs, the emulated recycling list with multi-selection, alert/confirm/prompt
dialogs (`<dialog>`), fonts bundled via `FontFace` with a generated `fonts.json`, localization
including RTL mirroring, dark mode, lifecycle (`DidBecomeActive`/`WillResignActive` from page
visibility), routes in the URL (below), day-part-prefs backed by `localStorage`
(docs/prefs.md) — so app state bound through `day_part_prefs::bind` survives a reload — and
day-part-http backed by the browser's `fetch()` (docs/http.md): `fetch_async` and
`fetch_future` work in full, with drop-cancel through an `AbortController`; the blocking
entry points return `Unsupported` (one thread, no blocking waits).

## Routes in the URL

The app's route and the URL hash stay in step, both ways (docs/navigation.md):

- **Loading `…/#controls`** opens on that section: the host page hands the hash to
  `set_launch_deeplink`, the web spelling of `DAY_DEEPLINK`.
- **Navigating in the app** updates the hash through the `Toolkit::set_route` duty — one
  history entry per step, so browser back/forward walk the app's navigation. The launch
  reflection replaces the current entry instead of pushing one.
- **A hash change the app didn't write** (back/forward, a hand-edited URL) arrives as
  `Event::RouteRequested` and navigates; echoes of the app's own updates are dropped on both
  sides of the boundary.

Routes are the same strings every platform speaks — `day::routes!` keys and `/`-separated
paths (`#mail/inbox/msg-42` works like `navigate("mail/inbox/msg-42")`).

## dayscript on the web

`day launch -p web-dom --script …` and `day drive` work like every other target. The engine
runs inside the wasm; the page opens a WebSocket to the dev server's `/dayscript`, and the
server bridges it to the plain TCP protocol the runner already speaks (§14.5) — one script,
every platform. Differences from native, all internal:

- The engine's implicit bounded wait reschedules through the delayed poster instead of
  sleeping (one thread, no `Instant`); replies arrive when the step settles.
- The in-page `screenshot` step reports unsupported (a DOM cannot rasterize itself), so the
  runner captures through the **`DAY_WEB_DRIVER`** browser instead: set it to a command line
  (e.g. `node scripts/ci/webdom-driver.mjs`, headless WebKit via Playwright) and `day` spawns
  it as `<cmd> <url> <control-port>`; the driver serves `GET /screenshot` (PNG) and
  `GET /quit` on the control port. Without a driver, scripted runs fail at the first
  screenshot; interactive `day launch` never needs one.
- Steps for capabilities the web genuinely lacks (the native file pickers) carry
  `skip_on: [web-dom]` in the walkthrough — the runner drops them for this target
  (DESIGN.md Appendix C). The HTTP demo runs unskipped: the dev server answers the
  same-origin `/day-http-ok` echo endpoint (below) with the same bodies the native demo's
  loopback server serves.
- Day element ids double as DOM ids (via the a11y identifier duty), so the page is
  inspectable with the same ids scripts use.

CI runs the full showcase walkthrough this way (light/dark × en/fr/ar/zh-CN) and publishes
the captures in the website gallery's "Web DOM" column.

Known gaps, in rough order of interest:

- **Bundled data assets** — `resource()` returns `None` (no synchronous file reads in a
  browser). Images and fonts work; raw asset bytes need an embedding mode that does not exist
  yet.
- **App menus and context menus** — no DOM equivalent of a native menu bar; unsupported.
- **Native pieces** — webview, map, lottie, combobox, searchfield and activity render their
  standard placeholder. `day-piece-media` now renders a real `<video>` (docs/media.md): the browser
  supplies the transport chrome, so a URL is required (a file path cannot load) and autoplay needs
  `.muted(true)`. The seam is open for the others — day-dom exposes a RUNTIME renderer registry
  (`day_dom::register_renderer`) rather than the `linkme` distributed slice the other eight backends
  use, because `#[distributed_slice]` does not compile for wasm32; a piece self-registers from its
  own constructor. Parts other than prefs, http, sensors and location (battery,
  clipboard, haptics…) answer their unavailable tier. `day-part-sensors` streams the accelerometer
  and gyroscope from `DeviceMotionEvent` (no cross-browser magnetometer exists — docs/sensors.md),
  `day-part-location` rides `navigator.geolocation`, and `day-part-permissions` answers from
  `navigator.permissions`. All three need a secure context, and iOS Safari's motion prompt must be
  requested from inside a button action while the user gesture is still live.
- **day-break** — no signal handlers on wasm; init succeeds and every API degrades to its
  documented stub.
- **Window control** — the page can set `document.title`; size, minimum size, and multi-window
  do not apply.

## Serving and static hosting

The dist directory is self-contained static files — no server component, no build tooling on
the host. The only reason `day launch` runs a server at all is that browsers refuse to
instantiate wasm from `file:` URLs; any static host works, including GitHub Pages. The shim
prefers `instantiateStreaming` (which requires the `application/wasm` MIME type) and falls
back to a buffered instantiate on hosts that serve wasm as something else, so a plain
directory listing on a dumb server still boots.

Beyond static files, the dev server answers two dynamic paths: `/dayscript` (the WebSocket
bridge above) and **`/day-http-ok`** — a method-echo endpoint for HTTP demos whose native
form would spin a loopback listener, which a browser tab cannot (GET answers `day-http-ok`;
any other method echoes `day-http-ok:<METHOD>`, matching the showcase's native one-shot
server byte for byte). On a static host that endpoint does not exist, so the showcase's demo
buttons report the host's honest answer (a 404 page, or a 405 for PATCH) — the URL checker
and every other HTTP call work anywhere.

The Day website publishes the showcase's web-dom build at
**`https://daybrite.dev/showcase/web-dom/`**: CI's `web-dom` job uploads the release dist as
an artifact, and the website job drops it into the built site (see `.github/workflows/ci.yml`
and [§20](../DESIGN.md#20-continuous-integration)). To preview that page locally, stage the
dist with `scripts/website.sh webdom`, then `scripts/website.sh dev`.

Query parameters the host page reads: `theme=light|dark` (else the OS preference),
`locale=<bcp47>` (else the browser languages).
