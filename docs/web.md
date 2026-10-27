---
navTitle: "Web"
title: "Run and publish a web app"
description: "Build a Day app for the browser, check browser limitations, and publish the generated static files."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Run and publish a web app

Use the `web-dom` target to run a Day app in a browser. Rust compiles to WebAssembly, and
Day presents controls as HTML elements. The target is
[Tier 3 · Experimental](/docs/reference/platforms#support-tiers); check the capabilities your
app needs before publishing it.

## Run locally

From the app's project directory:

```sh
day doctor --toolkit dom
day launch -t web-dom
```

Day builds the app, starts a local HTTP server, and opens the default browser. It installs
the `wasm32-unknown-unknown` Rust target when needed. To build without launching:

```sh
day build -t web-dom
```

The output is in `build/day/cargo/web-dom/<profile>/dist/`. Serve that directory over HTTP
rather than opening `index.html` as a file. Use the browser's developer console for logs and
startup errors; Rust's `println!` does not provide browser console output. See
[Logging](/docs/how-to/logging).

Apps using [Persistence](/docs/reference/persistence) also need a C compiler with WebAssembly
support. On macOS, `brew install llvm` supplies one; Xcode's clang does not. Day probes available
compilers, and `day doctor --toolkit dom` reports the selection. Set
`CC_wasm32_unknown_unknown` explicitly when you need a particular compiler.

## Adapt the app to the browser

The browser controls permissions, network access, storage, and window management. Check
capabilities at runtime and provide an alternative when a feature is unavailable.

| Feature | Browser behavior |
| --- | --- |
| Controls and drawing | HTML controls, DOM layout, and canvas drawing. See the [toolkit reference](/docs/reference/platforms/web-dom) and each component's support table. |
| HTTP | Async requests use the browser's fetch API and are subject to CORS. Blocking HTTP calls are unsupported. |
| Preferences | Stored in `localStorage` for the current origin. |
| Files | Open uses a browser file picker; save initiates a download. Folder dialogs are unsupported. |
| Persistent databases | Require the storage and hosting capabilities described below. |
| Clipboard, location, and sensors | Depend on browser support, permissions, a secure context, and sometimes a live user gesture. |
| Windows | The app runs within its page. Desktop window placement and additional OS windows are unavailable. |

Use [`day::task`](https://docs.rs/day-app/latest/day/fn.task.html) and
[`day::sleep`](https://docs.rs/day-app/latest/day/fn.sleep.html) for asynchronous work.
[`day::spawn_blocking`](https://docs.rs/day-app/latest/day/fn.spawn_blocking.html) runs inline on this
target, so a long job still blocks the page. Prefer browser async APIs for long operations;
see [Async work](/docs/explanation/async).

Images and fonts are staged with the build. Synchronous
[`resource()`](https://docs.rs/day-core/latest/day_core/fn.resource.html) access to bundled data bytes
is unavailable in the browser. Check [Resources](/docs/reference/resources) when sharing data
loading code with native targets.

Browser storage belongs to an origin. A local server that starts on a different port has a
different store. Private browsing, storage policy, or clearing site data can also affect
persistence. Do not use a successful local session as proof that a hosted database will persist.

## Navigation, language, and appearance

App routes appear in the URL hash. Opening `/#mail/inbox` navigates to that route, and browser
Back and Forward follow the app's navigation history. Define routes as described in
[Navigation](/docs/how-to/navigation).

The app uses the browser's languages and appearance by default. To inspect another locale:

```sh
day launch -t web-dom --locale ar
```

The host also accepts `?locale=<language-tag>` and `?theme=light` or `?theme=dark`.
`day launch --env KEY=VALUE` passes app configuration through query parameters, read with
[`day::env`](https://docs.rs/day-app/latest/day/fn.env.html). These values are visible in the URL;
use them for non-secret configuration. Avoid the reserved keys `theme`, `locale`, `dayscript`,
`vw`, `vh`, `dpr`, `dark`, `locales`, `route`, `tz`, and `tzoffset`.

## Test browser behavior

Use [DayScript](/docs/how-to/dayscript) for the app's interactions:

```sh
day launch -t web-dom --script dayscript/walkthrough.yaml
```

Screenshots require a browser driver. Install Playwright and a browser once:

```sh
mkdir -p ~/.cache/day-webdriver && cd ~/.cache/day-webdriver
npm init -y && npm install playwright
npx playwright install chromium
```

Then return to the app directory and configure the bundled driver:

```sh
export DAY_WEB_DRIVER="node $(day web driver)"
export DAY_WEB_DRIVER_PLAYWRIGHT=~/.cache/day-webdriver
export DAY_WEB_DRIVER_BROWSER=chromium
day launch -t web-dom --script dayscript/walkthrough.yaml
```

`DAY_WEB_DRIVER_BROWSER` selects `webkit`, `chromium`, or `firefox`. Interactive launches
do not need a driver.

The bundled driver uses a temporary browser profile that is removed after the run. If one
script seeds data for another, pass both with repeated `--script` options in the same launch.
For storage tests on Linux, use Chromium; the bundled Linux WebKit engine lacks OPFS.

Also check the app interactively at narrow and wide viewport sizes, with keyboard navigation,
and with the browsers you intend to support. Scripted actions alone do not establish that
browser permission prompts or user-gesture restrictions work for a person.

## Publish the build

Upload the contents of `dist/` to a static web host. Keep the generated files together and
serve over HTTPS. Asset URLs are relative, so the app can live at a domain root or under a
subdirectory. Configure the host to serve `.wasm` as `application/wasm` for streaming startup.

For durable file databases, the host must provide cross-origin isolation:

```text
Cross-Origin-Opener-Policy: same-origin
Cross-Origin-Embedder-Policy: require-corp
```

Day's local server sends these headers. On hosts that cannot provide them, such as GitHub
Pages, the persistence extension reports `durable: false` and refuses file-database opens.
Check [Persistence](/docs/reference/persistence#the-web) before choosing a host.

The reusable Day app workflow can deploy the web build to GitHub Pages. Include `web-dom` in
its targets and set `deploy-web: true`; follow the complete
[GitHub Actions instructions](/docs/how-to/github-actions#deploy-to-github-pages) for permissions
and repository settings. The local `/dayscript` bridge and `/day-http-ok` demonstration endpoint
are not part of a static deployment.

## Installation and offline use

The build includes a web app manifest, generated icons, and a service worker. Supported
browsers can offer installation or Add to Home Screen. Once cached, the app shell can load
offline; remote APIs and uncached content still need a network connection.

The manifest uses the app's store metadata and icon resources. Optional `Day.toml` settings
control browser appearance:

```toml
[web]
theme-color = "#ffffff"
theme-color-dark = "#000000"
background-color = "#123246"
display = "standalone"
```

The service worker registers on HTTPS and loopback hosts. A changed build creates a new cache.
When checking an update, inspect the active service worker and clear site data if an old
installation is interfering with the test.

For custom JavaScript supplied by an extension, see the
[web module contract](https://github.com/daybrite/day/blob/main/docs/extending.md#web-scripts-packagemetadatadayweb).
