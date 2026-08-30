<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Day — Design Document

**An industry-strength Rust framework for cross-platform application development with native toolkits.**

> [!IMPORTANT]
> Status: **implemented and shipping.** This document began as the pre-implementation design
> (adversarially reviewed 2026-07-01); the framework has since been built. Seven native targets
> plus the headless mock toolkit run today; the showcase app passes its 200+-step scripted
> walkthrough on macOS (AppKit, GTK, Qt), iOS, and Android; the `day` CLI builds, launches,
> scripts, and packs for every target; CI exercises all of it. The document is now the
> **architecture overview and rationale**:
>
> - **Part I ([§0](#0-vision-lineage-and-non-goals)–[§20](#20-continuous-integration))** describes the system as built. Where the shipped design differs from
>   the original text, a status alert at the top of the section says exactly how.
> - **[Part II](#part-ii--historical-record) ([§21](#21-mvp-definition-and-milestone-plan)–[§24](#24-adversarial-review-findings-and-resolutions))** is the preserved historical record — the milestone plan, decision
>   points, and adversarial-review findings. It is complete; nothing in it is open. It stays
>   because it documents *why* the architecture is shaped this way.
>
> Subsystem detail lives in `docs/*.md` (normative — see the index below); this document is the
> map and the rationale. Section numbers are stable: hundreds of source comments cite them
> (`§4.4`, `§8.3`, …). Never renumber a section; add subsections or addenda instead.

## Reading this document

Sections carry a status alert whose type says how far to trust the text below it:

- A **Note** alert — the section matches the code (or is an outcome/context note). This
  includes "shipped as written" stamps and sections whose body was rewritten to the current
  design.
- An **Important** alert — the goal survived but the mechanism changed, or only part shipped;
  the alert says exactly how, and where the real design is documented.
- A **Warning** alert — the design below was never built or was superseded. It is kept as a
  recorded design; don't build against it.
- Unannotated sections describe concepts and rationale that are accurate as written.

Part I still names milestones (M0–M9) in places; those refer to [§21.2](#212-milestones-each-lands-green-ci--tests-forward-dependencies-eliminated)'s historical plan — all
of it is complete. Read "an M5 acceptance item" as "verified when that milestone landed".

### Subsystem index

The normative reference for each shipped subsystem is its `docs/` file; the section here gives
the architecture-level view and the rationale.

| subsystem | normative doc | overview here |
|---|---|---|
| navigation — `routes!`, `selector`, `stack`, deep links, predictive back | [docs/navigation.md](docs/navigation.md) | [§10.5](#105-navigation-and-presentation) |
| native recycling lists | [docs/list.md](docs/list.md) | [§10](#10-native-list-integration) |
| scrolling — the scroll piece, programmatic `ScrollTarget`, dayscript `scroll_to` | [docs/scroll.md](docs/scroll.md) | [§7.6](#76-scroll) |
| baseline alignment — form rows and `VAlign::FirstBaseline` sitting text on one line | [docs/baseline.md](docs/baseline.md) | [§7.10](#710-baseline-alignment) |
| Toolkit duty conformance — which backend implements which duty (generated, CI-gated) | [docs/duty-matrix.md](docs/duty-matrix.md) | [§8.1](#81-the-toolkit-trait) |
| Piece-vocabulary coverage — which kinds each backend renders, which piece ships which arm, every `Cap` answer (generated, CI-gated) | [docs/coverage-matrix.md](docs/coverage-matrix.md) | [§8.2](#82-the-open-renderer-registry) |
| Dayscript recorder coverage — the step the recorder writes for every `Event` (generated, CI-gated) | [docs/recorder-matrix.md](docs/recorder-matrix.md) | [§14.6](#146-recording) |
| menus — app menu, context menus, roles, shortcuts | [docs/menus.md](docs/menus.md) | [§8.1](#81-the-toolkit-trait) |
| deep links — scheme registration, cold/warm delivery, per-platform intake, `[[shortcuts]]` launcher shortcuts (spec; ios/android/web/harmony shipped) | [docs/deep-links.md](docs/deep-links.md) | [§10.5](#105-navigation-and-presentation) |
| window toolbars — `toolbar`, the item vocabulary, `Symbol` icons, per-desktop realization | [docs/toolbars.md](docs/toolbars.md) | [§8.1](#81-the-toolkit-trait) |
| search — `.searchable()` on a navigation surface, placement as a preference, scopes and completions | [docs/search.md](docs/search.md) | [§8.1](#81-the-toolkit-trait) |
| size classes — window width/height buckets, per-window signal, re-presenting a nav host on a breakpoint; the `RowFit` row fit policies and the debug overflow diagnostic | [docs/size-classes.md](docs/size-classes.md) | [§5.3](#53-built-in-pieces-mvp-set), [§10.5](#105-navigation-and-presentation) |
| app icons — `day icon`, the layered master, per-platform exports + drift gate | [docs/icons.md](docs/icons.md) | [§16.5](#165-subcommands) |
| vector images — `resource/vectors/`, the `vector` piece, per-backend staging + tint | [docs/vectors.md](docs/vectors.md) | [§18.3](#183-images-and-data) |
| window image — `day::window_image()`, content vs `.chrome()`, per-backend capture, dayscript precedence | [docs/window-image.md](docs/window-image.md) | [§8.1](#81-the-toolkit-trait), [§14](#14-scripting-dayscript) |
| dialogs & presentation — alert/confirm/prompt/sheets, file pickers | [docs/dialogs.md](docs/dialogs.md), [docs/files.md](docs/files.md) | [§8.1](#81-the-toolkit-trait) |
| fullscreen cover — `cover`, `defers_system_gestures`, `interactive_dismiss_disabled` | [docs/cover.md](docs/cover.md) | [§10.5](#105-navigation-and-presentation) |
| inspector — `inspector(visible, content, panel)`, native trailing pane vs composed pane + compact sheet, `Cap::Inspector`; `.edge(PaneEdge::Leading)` for a leading utility pane | [docs/inspector.md](docs/inspector.md) | [§5.3](#53-built-in-pieces-mvp-set), [§8.1](#81-the-toolkit-trait) |
| tree — `tree(source, row)` hierarchical rows: native tree views where `Cap::Tree` is Native, the composed list-backed tree elsewhere; token identity, app-owned expansion, drag-to-reparent | [docs/tree.md](docs/tree.md) | [§5.3](#53-built-in-pieces-mvp-set), [§8.1](#81-the-toolkit-trait) |
| forms — `form`/`section`/`labeled` | [docs/forms.md](docs/forms.md) | [§5.3](#53-built-in-pieces-mvp-set) |
| grid — `grid`/`grid_row` eager grid, `.grid_span`/`.grid_align` | [docs/grid.md](docs/grid.md) | [§5.3](#53-built-in-pieces-mvp-set), [§7.2](#72-the-protocol-parent-proposes-child-chooses) |
| keyboard focus — `.focused()`, `on_submit`, dayscript focus steps | [docs/focus.md](docs/focus.md) | [§4.4](#44-events-and-controlled-inputs), [§8.3](#83-events) |
| canvas, shapes, gradients, gestures | [docs/shapes.md](docs/shapes.md) | [§11](#11-canvas) |
| text & typography | [docs/text.md](docs/text.md), [docs/text-runs.md](docs/text-runs.md), [docs/markdown.md](docs/markdown.md) | [§6.4](#64-typography) |
| localization — Fluent, `res::str` typed keys, locales | [docs/localization.md](docs/localization.md) | [§12](#12-localization-fluent), [§18.5](#185-typed-resource-constants-docsresourcesmd) |
| resources — images, data assets, custom fonts, typed constants | [docs/resources.md](docs/resources.md) | [§18](#18-resources-icons-and-theming) |
| accessibility & the a11y audit | [docs/accessibility.md](docs/accessibility.md) | [§13](#13-accessibility) |
| app lifecycle | [docs/lifecycle.md](docs/lifecycle.md) | [§8.1](#81-the-toolkit-trait), [§9](#9-the-eight-toolkits-and-the-extra-combinations) |
| async — `day::task`/`TaskHandle`, `Resource`/`Load`, the runtime-quarantine policy | [docs/async.md](docs/async.md) | [§4.5](#45-async) |
| observable model — per-property `Store`/`Keyed`/`Elem`/`Field`, `#[derive(Observable)]`, the change log | [docs/model.md](docs/model.md) | [Addendum](#addendum-2026-08-22--day-model-per-property-observation) |
| persistence — `ModelContainer`, `#[derive(Model)]`, SQLite drivers/engines, migrations, codecs | [docs/persistence.md](docs/persistence.md) | [Addendum](#addendum-2026-08-22--day-persistence-sqlite-storage-for-the-model) |
| tweaks — per-toolkit configuration of built-ins | [docs/tweaks.md](docs/tweaks.md) | [Addendum](#addendum-2026-07-09--tweaks-per-toolkit-configuration-of-built-in-pieces) |
| extension packages — pieces, parts, `[package.metadata.day.*]` | [docs/extending.md](docs/extending.md) | [§15](#15-extensibility-pieces-parts-and-tweaks) |
| daybridge — foreign-language implementations of a Rust API (Swift/Kotlin/Java/ArkTS/JS/C/C++) | [docs/bridge.md](docs/bridge.md) | [§15.6](#156-daybridge-foreign-language-implementations-of-a-rust-api) |
| scripting & agents — dayscript, recording (`day::record`, `--record`), `day drive`, MCP | [docs/agent.md](docs/agent.md), website dayscript reference | [§14](#14-scripting-dayscript) |
| platform services ("parts": battery, network, sensors, clipboard, prefs, haptics, deviceinfo, http, permissions, location, fs) | [docs/battery.md](docs/battery.md), [docs/network.md](docs/network.md), [docs/sensors.md](docs/sensors.md), [docs/clipboard.md](docs/clipboard.md), [docs/prefs.md](docs/prefs.md), [docs/haptics.md](docs/haptics.md), [docs/deviceinfo.md](docs/deviceinfo.md), [docs/http.md](docs/http.md), [docs/permissions.md](docs/permissions.md), [docs/location.md](docs/location.md), [docs/fs.md](docs/fs.md) | [§15](#15-extensibility-pieces-parts-and-tweaks) |
| bundled pieces (webview, media, map, lottie, searchfield, combobox, color picker, …) | [docs/webview.md](docs/webview.md), [docs/media.md](docs/media.md), [docs/map.md](docs/map.md), [docs/lottie.md](docs/lottie.md), [docs/searchfield.md](docs/searchfield.md), [docs/combobox.md](docs/combobox.md), [docs/colorpicker.md](docs/colorpicker.md) | [§15](#15-extensibility-pieces-parts-and-tweaks) |
| color — the `Color`/`Paint` currency, what a native picker can hand back, and a proposal to widen it | [docs/color.md](docs/color.md) | [§6.3](#63-semantic-theme-tokens), [§11](#11-canvas) |
| SwiftUI embedding — local SwiftPM packages, generated `crate::swiftui::*` bindings + hosting glue, the macOS Swift build leg | [docs/swiftui.md](docs/swiftui.md) | [§15.2](#152-package-layout-and-aggregation) |
| built-in controls — picker, text area | [docs/picker.md](docs/picker.md), [docs/textarea.md](docs/textarea.md) | [§5.3](#53-built-in-pieces-mvp-set) |
| styled text editing — `StyledText`, its Markdown/HTML/RTF codecs, and the editor piece over them | [docs/texteditor.md](docs/texteditor.md) | [B.5](#b5-richtext-tier-2--deep-native-control) |
| HarmonyOS / OpenHarmony | [docs/harmonyos.md](docs/harmonyos.md) | [§9](#9-the-eight-toolkits-and-the-extra-combinations) |
| web — the `web-dom` backend (wasm32 + DOM) | [docs/web.md](docs/web.md) | [§9](#9-the-eight-toolkits-and-the-extra-combinations) |
| day-lite — JS/TS miniapps, the dyn piece registry, superapp embedding, a headless miniapp test runner | [docs/lite.md](docs/lite.md) | [§15](#15-extensibility-pieces-parts-and-tweaks) |
| logging — the `log` facade every day crate emits through, the auto-installed default logger, per-platform sinks (stderr / logcat / the browser console), `DAY_LOG` | [docs/logging.md](docs/logging.md) | [§8.5](#85-panics-and-crashes) |
| day-break — consent-first crash reporting (panic hook + signal handlers, next-launch report, pluggable upload) | [docs/break.md](docs/break.md) | [§8.5](#85-panics-and-crashes) |
| secondary windows — `open_window`, the Preferences window + auto menu item, `WindowKind`, the cover fallback, the debug title tag | [docs/windows.md](docs/windows.md) | [§8.1](#81-the-toolkit-trait) |
| toolchain & environment discovery | [docs/environment.md](docs/environment.md) | [§16](#16-the-day-cli) |
| API design conventions | [docs/api-style.md](docs/api-style.md) | [§5.1](#51-authoring-surface-functions-and-builders-no-macros) |

**Maintenance rule (binding):** any change that alters what this document describes — day-spec
duties or events, the built-in piece vocabulary, CLI commands, dayscript steps, the extension
mechanisms, the crate set, or the repository layout — must update the affected section (or its
pointer table) in the same change, so this document always reflects the current reality. When a
section would restate something a `docs/*.md` file owns, point to it instead of duplicating it.

---

## Table of contents

**Part I — the architecture as built**

- [§0 Vision, lineage, and non-goals](#0-vision-lineage-and-non-goals)
- [§1 Glossary and naming](#1-glossary-and-naming)
- [§2 The four pillars](#2-the-four-pillars)
- [§3 Architecture overview and crate graph](#3-architecture-overview-and-crate-graph)
- [§4 Reactive core (`day-reactive`)](#4-reactive-core-day-reactive)
- [§5 The Piece model (`day-core`)](#5-the-piece-model-day-core)
- [§6 Styling and per-target variation](#6-styling-and-per-target-variation)
- [§7 Layout](#7-layout)
- [§8 The Toolkit specification (`day-spec`)](#8-the-toolkit-specification-day-spec)
- [§9 The eight toolkits (and the extra combinations)](#9-the-eight-toolkits-and-the-extra-combinations)
- [§10 Native list integration](#10-native-list-integration)
- [§11 Canvas](#11-canvas)
- [§12 Localization (Fluent)](#12-localization-fluent)
- [§13 Accessibility](#13-accessibility)
- [§14 Scripting (dayscript)](#14-scripting-dayscript)
- [§15 Extensibility: pieces, parts, and tweaks](#15-extensibility-pieces-parts-and-tweaks)
- [§16 The `day` CLI](#16-the-day-cli)
- [§17 The Conventional Day Project and `Day.toml`](#17-the-conventional-day-project-and-daytoml)
- [§18 Resources, icons, and theming](#18-resources-icons-and-theming)
- [§19 Repository layout, examples, and docs site](#19-repository-layout-examples-and-docs-site)
- [§20 Continuous integration](#20-continuous-integration)

**[Part II — historical record (complete; kept for rationale)](#part-ii--historical-record)**

- [§21 MVP definition and milestone plan](#21-mvp-definition-and-milestone-plan)
- [§22 Decision points for review](#22-decision-points-for-review)
- [§23 Risks](#23-risks)
- [§24 Adversarial review findings and resolutions](#24-adversarial-review-findings-and-resolutions)
- [Addendum (2026-07-09): Tweaks](#addendum-2026-07-09--tweaks-per-toolkit-configuration-of-built-in-pieces)
- [Addendum (2026-08-22): day-model — per-property observation](#addendum-2026-08-22--day-model-per-property-observation)
- [Addendum (2026-08-22): day-persistence — SQLite storage for the model](#addendum-2026-08-22--day-persistence-sqlite-storage-for-the-model)

**Appendices**

- [Appendix A: The showcase app (pointer to the live app)](#appendix-a--the-showcase-app-end-to-end)
- [Appendix B: Extension examples — design-era sketches with shipped outcomes](#appendix-b--extension-examples)
- [Appendix C: dayscript reference (v1)](#appendix-c--dayscript-reference-v1)
- [Appendix D: `day` CLI transcripts (illustrative)](#appendix-d--day-cli-transcripts)
- [Appendix E: Implementation notes for the builder (historical)](#appendix-e--implementation-notes-for-the-builder-historical)

---

## §0 Vision, lineage, and non-goals

### §0.1 What Day is

**Day** is a Rust framework for building applications that look, feel, and behave like native
applications on every platform — because they *are* native applications. UI is authored once, in
idiomatic Rust, as a declarative tree of **Pieces** (what SwiftUI calls a View and Flutter calls a
Widget). Each Piece is realized by **real native components** — `UILabel`, a Material `MaterialButton`,
`NSTextField`, `GtkEntry`, `QSlider`, XAML `TextBox`, a DOM `<input>` — through a per-platform
**toolkit** backend. Day owns layout, reactivity, localization, accessibility policy, and scripting;
the platform owns pixels, text input, scrolling physics, and assistive technology.

Seven **primary targets** (OS–toolkit combinations), all shipped:

| target | OS | toolkit | status |
|---|---|---|---|
| `macos-appkit` | macOS | AppKit | shipped; walkthrough + pack (`.dmg`) in CI |
| `ios-uikit` | iOS | UIKit | shipped; Simulator walkthrough + pack (`.ipa`) in CI |
| `android-mdc` | Android | Material Components (M3 Expressive) / android.view | shipped; emulator walkthrough + pack (`.apk`/`.aab`) in CI |
| `linux-gtk` | Linux | GTK 4 | shipped; headless walkthrough + pack (flatpak + appimage) in CI |
| `linux-qt` | Linux | Qt 6 Widgets | shipped; headless walkthrough + pack (flatpak + appimage) in CI |
| `windows-xaml` | Windows | system XAML (XAML Islands in a Win32 host) | shipped; CI-verified (`.msix` + installer) |
| `harmony-arkui` | HarmonyOS | ArkUI (NDK C API) | shipped; cross-compile in CI, `.hap` pack, `day ohos` emulator helpers ([docs/harmonyos.md](docs/harmonyos.md)) |
| `web-dom` | any modern browser | the DOM (semantic HTML + ARIA) | experimental (2026-07); wasm32 cdylib + JS shim, `day launch` dev server ([docs/web.md](docs/web.md)) |

An eighth backend, **`day-mock`**, is headless: it records toolkit ops and answers deterministic
measurements, so the whole pipeline is unit-testable without a display ([§3.2](#32-crates)). A ninth,
**`web-dom`** (`toolkits/day-dom`: wasm32 + the browser DOM as the toolkit), landed 2026-07 as an
**experimental** target — build/serve via `day build|launch -p web-dom`, subset capabilities;
[docs/web.md](docs/web.md) is the reference. It descends from the original `web-html` sketch, whose record is
preserved in [§9](#9-the-eight-toolkits-and-the-extra-combinations).

Because GTK and Qt are themselves portable, the **non-default combinations** `macos-gtk`,
`macos-qt`, `windows-qt`, and `windows-gtk` are also valid targets — a target is just an
(OS, toolkit) pair whose toolkit supports that OS. Day's own development loop runs six targets
on a single macOS host: `macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit` (Simulator),
`android-mdc` (emulator), and `harmony-arkui` (cross-compile; emulator via `day ohos`).

A `day` command-line tool — deliberately modeled on the architecture of `flutter_tools`
(`flutter/packages/flutter_tools`) — creates, builds, signs, launches, packs, lints, scripts,
and drives Day projects, and is designed for use by humans, CI, IDEs, and AI agents alike
(`day drive` and `day mcp-server` are the agent surface — [docs/agent.md](docs/agent.md), [§16](#16-the-day-cli)).

### §0.2 Lineage — what each ancestor contributes

Day is not a greenfield guess. It consolidates several years of prior art in this workspace:

| ancestor | what Day inherits | what Day changes |
|---|---|---|
| **pane/** (Rust, 6 native backends running) | The `Backend`-trait shape with an associated `Handle`; one-toolkit-per-binary monomorphization; the open, link-time component registry (`linkme`); descriptor-carried value bindings (signal + `on_change` closure, per-widget callback tables keyed by id); the C++ shim pattern for Qt and XAML; the JNI + Java-shim pattern for Android; the objc2 patterns for AppKit/UIKit; the headless mock toolkit for unit testing the whole pipeline | pane re-renders observing views and reconciles; Day builds the tree **once** and binds attributes reactively ([§4](#4-reactive-core-day-reactive)) — no tree diffing on state change |
| **hop/** (Swift, 4 desktop toolkits) | The parent-proposes/child-chooses layout engine and the lessons it banked (text height-for-width measurement, GTK window shrink, scroll/split interactions); AX-tree diff validation; the CI screenshot pipeline (content-validated captures, `GITHUB_STEP_SUMMARY` galleries); `hoppack`'s per-OS packaging Stage pipeline | Day's layout engine is a from-scratch Rust design informed by hop's, with an open `Layout` trait |
| **skip/ + skipstone/** (Swift↔Kotlin app tooling) | The Conventional Project shape (a normal language-native project plus per-platform scaffolds); metadata conveyance via generated files (`Skip.env` → xcconfig); the discipline of gradle/xcodebuild orchestration; emulator/simulator management; polyglot bridging scar tissue (skip-bridge) | Day's polyglot boundary is a small stable C ABI ([§15](#15-extensibility-pieces-parts-and-tweaks)), not transpilation or generated JNI bridging |
| **floem/** (Rust, GPU-rendered) | The authoring surface: plain functions and builder methods, **no required macros**; `Copy` signals in a scope-owned arena; `create_updater`-style bind-to-setter effects; keyed `dyn_stack` and virtualized `virtual_stack` decomposition; `canvas(|cx, size| …)`; Fluent-based localization proven in this exact API style | floem renders its own pixels (vello/vger + taffy); Day drives native widgets and owns a native-measurement-aware layout engine |
| **flutter/** (Dart; tool studied at `flutter/packages/flutter_tools`) | CLI architecture: DI'd services behind a context for testability; the `Command` envelope (`validate → run`); `doctor` + per-platform workflows; templates for `create`; **the platform-shell callback build pattern** (the Xcode/Gradle project calls back into the tool for the framework part, so native IDE builds are never stale); `gradle_errors`-style failure translation; the machine/daemon protocol for IDEs | Day has no VM: no hot reload in v1 (fast recompile + relaunch + dayscript replay instead, [§16.9](#169-the-inner-loop-no-hot-reload--the-honest-story)); Day's platform shells host native widgets, not a rendering engine |

### §0.3 Non-goals

- **Not a renderer.** Day never rasterizes text or widgets itself (the Canvas piece delegates to the
  platform's native 2D API). No skia, no vello, no embedded web view for core UI.
- **Not pixel-identical across platforms.** A Day app looks like a Mac app on macOS and a Material
  app on Android. Cross-platform *consistency of behavior and information architecture*, native
  *look and feel*.
- **Not a Dart/JS-style VM platform.** Rust compiles ahead of time. No hot reload in v1 ([§16.9](#169-the-inner-loop-no-hot-reload--the-honest-story)
  explains the mitigation and the roadmap position).
- **Not a widget-toolkit abstraction of lowest common denominator.** Where platforms diverge, the
  Piece API exposes capability flags and per-target styling rather than pretending divergence away;
  where a platform lacks a control, the toolkit composes one from primitives (as hop did for GTK's
  missing date picker).

---

## §1 Glossary and naming

| term | meaning |
|---|---|
| **Piece** | Day's unit of UI composition (SwiftUI "View", Flutter "Widget"). Also the brand for UI extension packages: "a Day Piece" (`pieces/day-piece-*`). |
| **Part** | A headless platform-service package — battery, network, clipboard, sensors, prefs, haptics, device info, HTTP, OS permissions, location, app-local files, local notifications, wall clock & time zones — exposing signals/functions with per-OS native halves (`parts/day-part-*`, [§15](#15-extensibility-pieces-parts-and-tweaks)). |
| **Tweak** | A per-toolkit configuration of the native widget behind an existing built-in piece (`Decorate::tweak`, `tweaks/day-tweak-*`; [Addendum](#addendum-2026-07-09--tweaks-per-toolkit-configuration-of-built-in-pieces), [docs/tweaks.md](docs/tweaks.md)). |
| **Toolkit** | A native widget system: UIKit, Android Material, AppKit, GTK 4, Qt 6 Widgets, Windows XAML, ArkUI (+ the headless mock). |
| **Target** | An (OS, toolkit) pair, written `<os>-<toolkit>`: `macos-appkit`, `macos-gtk`, `ios-uikit`, … One binary is built per target. |
| **Backend crate** | The Rust crate implementing `day-spec` for one toolkit (`toolkits/day-appkit`, `toolkits/day-gtk`, …). One backend is linked per binary. |
| **Realized tree** | The runtime tree of mounted pieces: each node owns a native handle (or is layout-only), a reactive scope, and layout state. |
| **Signal / Memo / Effect / Scope** | The reactive primitives ([§4](#4-reactive-core-day-reactive)). |
| **Route** | A typed navigation destination declared with the `routes!` macro; what `selector`/`stack`, deep links, and dayscript `navigate` speak ([docs/navigation.md](docs/navigation.md)). |
| **dayffi** | *(superseded)* The C ABI designed for polyglot extensions; never shipped. The shipped mechanism is `[package.metadata.day.<platform>]` ([§15](#15-extensibility-pieces-parts-and-tweaks)). |
| **dayscript** | The Maestro-inspired YAML UI-scripting language and its embedded engine ([§14](#14-scripting-dayscript)); a project's scripts live in `dayscript/` and the showcase's main script is "the walkthrough". |
| **Day.toml** | The project manifest ([§17.3](#173-daytoml)). |
| **Porcelain / plumbing** | User-facing CLI commands vs. stable hidden commands invoked by build systems (`day xcode-backend build`, `day gradle-backend build`) ([§16](#16-the-day-cli), [§17.4](#174-the-build-callback-flutters-pattern-exactly--including-the-details-flutter-learned-the-slow-way)). |

**Crate naming.** All crates are prefixed `day-` (`day-core`, `day-reactive`, `day-appkit`, …); the
umbrella facade crate that apps depend on is `day` with the binary tool in `day-cli` producing a
binary named `day`. DP-24 ([§22](#22-decision-points-for-review)) deferred crates.io reservation during the design phase; the
release lane is since **wired for crates.io** (publishability verified per PR; Trusted
Publishing on semver tags, [§20](#20-continuous-integration)) but the crates are **not yet published** — scaffolds default to
git dependencies (`day new --git`), with `--registry` ready for the day they are.

**Target strings** are the canonical identifiers everywhere: `Day.toml` `targets:`, `day launch
--platform`, CI job names, screenshot directory names, `PerTarget` style values. The toolkit half
also exists alone (`uikit`, `mdc`, `appkit`, `gtk`, `qt`, `xaml`, `arkui`, `mock`) for cases
where OS doesn't matter (styling varies by toolkit far more often than by OS).

---

## §2 The four pillars

Every Day app must be **1. localizable, 2. accessible, 3. scriptable, and 4. extensible** — and the
pillars deliberately build on each other:

1. **Localizable ([§12](#12-localization-fluent)).** Mozilla Fluent throughout. Every user-facing string in a Piece is a
   Fluent key by convention — enforced in practice by the `res::str` typed keys ([§18.5](#185-typed-resource-constants-docsresourcesmd)), which
   make a missing key a compile error, with `day lint` covering cross-locale coverage. The
   current locale is a *signal*, so locale switches are just another fine-grained update.
2. **Accessible ([§13](#13-accessibility)).** Accessibility rides the platform's native accessibility tree — Day uses
   native widgets, so baseline accessibility is inherited rather than reimplemented. Day adds a
   uniform annotation API and, critically, **stable identifiers**.
3. **Scriptable ([§14](#14-scripting-dayscript)).** dayscript targets elements by those same accessibility identifiers — the
   accessibility pillar is the scripting pillar's addressing scheme. Scripts run against localized
   builds (`day launch --locale fr-FR --script …`), so pillar 1 × pillar 3 = automated per-locale
   screenshots and e2e tests in CI.
4. **Extensible ([§15](#15-extensibility-pieces-parts-and-tweaks)).** Pieces, parts, and toolkit renderers are registered through open
   registries, so external crates (with native halves where needed) participate as equals of
   the built-ins — including in accessibility (they annotate through the same API) and
   scripting (their elements are addressable like any other). Lint rules and dayscript steps
   are *not* extension points in the shipped system (built-in sets only).

---

## §3 Architecture overview and crate graph

### §3.1 Layers

```
┌─────────────────────────────────────────────────────────────────────┐
│ app crate (user code: pieces as plain Rust functions)               │
├─────────────────────────────────────────────────────────────────────┤
│ day (umbrella: prelude, launch(), re-exports)                       │
├───────────────┬─────────────────────┬───────────────────────────────┤
│ day-pieces    │ pieces/ · parts/ ·  │ day-fluent → day-l10n         │
│ (built-ins,   │ tweaks/ (external   │ (localization)    day-script  │
│  canvas, nav) │ extension crates)   │                   (engine)    │
├───────────────┴─────────────────────┴───────────────────────────────┤
│ day-core: Piece model · realized tree · mounter · layout · events · │
│           focus · navigation · lists · menus · presentation         │
├─────────────────────────────────────────────────────────────────────┤
│ day-reactive (signals/memos/effects/scopes)   day-geometry (values) │
├─────────────────────────────────────────────────────────────────────┤
│ day-spec: Toolkit trait · renderer registry · events · a11y · DrawOp│
├────────┬───────┬────────┬───────┬───────┬───────┬────────┬──────────┤
│ appkit │ uikit │ android│  gtk  │  qt   │ xaml │ arkui  │   mock   │
└────────┴───────┴────────┴───────┴───────┴───────┴────────┴──────────┘
           each backend crate drives ONE native toolkit
```

Beside the runtime graph sit the build-time crates: `day-build` (an app's `build.rs` dependency —
typed resource constants, [§18.5](#185-typed-resource-constants-docsresourcesmd)), `day-fonts` (font name-table parsing shared by the CLI and the
runtimes), `day-toolchain` (host SDK/toolchain discovery shared by the CLI and the `-sys` build
scripts), and `day-cli` (the `day` binary).

### §3.2 Crates

> [!IMPORTANT]
> **Status: shipped differently.** This table reflects the crates as they exist. Relative to the
> original design: `day-canvas` was folded into `day-pieces`/`day-spec` (the `DrawOp` types live
> in the spec, the `canvas()`/shape pieces in day-pieces); `day-script-proto` was dropped (the
> wire protocol is newline-delimited JSON inside `day-script`); `day-meta` became a `day-cli`
> module plus the published `day-build` crate; `day-web` was never built; and `day-l10n`,
> `day-build`, `day-fonts`, `day-toolchain` were added.

| crate | contents | depends on |
|---|---|---|
| `day-reactive` | `Signal<T>`, `Memo<T>`, `Effect`, `Trigger`, `Scope`, `bind`/`watch`, batching, `Setter`, `Binding` (the two-way binding trait: `read`/`write`/`peek`; re-exported by day-pieces), `on_main` scheduler hook | — |
| `day-model` | the per-property observable store ([docs/model.md](docs/model.md)): `Store`/`Keyed`/`Elem`/`Field`, path interning + trigger reclamation, integer/`Uuid`/`String` keys behind `ModelId<M>`, the change log, background transactions; opt-in via the facade's `model` feature | day-reactive, uuid |
| `day-persistence` | SQLite storage for the model ([docs/persistence.md](docs/persistence.md)): `ModelContainer`, the `Model` derive's trait half, the change-log→SQL fold, typed live queries (`Query`/`LiveSet`, FTS5 + R*Tree through the derive), `SqliteDriver` with the rusqlite `Sqlite` and `Recorder` built-ins (engine features `bundled`/`system`/`cipher`), migrations, codecs, maintenance, external-change detection (`check_external` over `PRAGMA data_version`), relations (`One`/`Many`, delete rules, ordered to-many, generated join tables), `container.undo(levels)`; opt-in via the facade's `persistence` feature | day-model, day-reactive (+ day-pieces under `pieces`) |
| `day-sqlite-worker` | The web engine behind day-persistence ([docs/persistence.md](docs/persistence.md)): the vendored SQLite amalgamation compiled to wasm with no libc and no wasm-bindgen, a VFS over the day-sql worker page's synchronous OPFS access-handle imports (plus an in-RAM default VFS for `:memory:`), and the statement protocol the main thread speaks over the SharedArrayBuffer channel; unit-tests natively against an in-memory OPFS fake | (vendored C only) |
| `day-geometry` | `Point`, `Size`, `Rect`, `Insets`, `Color`, `Affine` — plain `Copy` value types shared by layout, canvas, and the spec | — |
| `day-spec` | `Toolkit` + `Platform` traits, renderer `Registry`, `Event`, typed props/patches, `A11yProps`, `DrawOp` + `Paint`/gradients, `MenuItem`, presentation types, `Cap`/`Support`, `Lifecycle`, `WindowOptions`, piece `kinds` | day-geometry |
| `day-core` | `Piece` trait + `AnyPiece`, `BuildCx`, the realized tree, the mounter, the layout engine (+ measure cache) and `Layout` trait, the event pump, focus, navigation host, list plumbing, menus, presentation, lifecycle, the `resource()` runtime | day-reactive, day-geometry, day-spec |
| `day-pieces` | the built-in vocabulary ([§5.3](#53-built-in-pieces-mvp-set)), the `Decorate` modifier set, `routes!`, forms, `selector`/`stack` navigation, dialogs, canvas + shape pieces, the prelude | day-core |
| `day-fluent` | the app-facing Fluent API: `install`, `tr()`, `set_locale`, `LocalizedText` | day-l10n |
| `day-l10n` | the core localization engine — low in the graph so day-pieces' own strings (dialog buttons, menu roles) localize too; also the `res::str` typing rules ([§18.5](#185-typed-resource-constants-docsresourcesmd)) | — |
| `day-script` | the embedded dayscript engine: step executor, element index, localhost-TCP transport (token-gated, newline-delimited JSON) | day-core, day-fluent |
| `day-vector` | the vector-graphics engine ([docs/icons.md](docs/icons.md), [docs/vectors.md](docs/vectors.md)): SVG parse/raster (resvg, text shaping off), SF Symbol template handling, VectorDrawable/.ico/.icns/.symbolset writers, the seeded icon generator (`icongen`) — consumed by day-cli (`day icon`, `resource/vectors/` staging) | resvg, tiny-skia, roxmltree |
| `day-mock` | headless toolkit for tests (records ops, deterministic measurement, synthetic events) | day-spec |
| `day-build` | `build.rs` codegen for apps: typed resource constants `res::{images,assets,fonts,str}` plus the `res::locales` catalog ([§18.5](#185-typed-resource-constants-docsresourcesmd)); the single source of the name-sanitization and Fluent-parsing rules the CLI stagers share | day-fonts, day-l10n |
| `day-fonts` | sfnt name-table parsing ([§18.4](#184-bundled-custom-fonts-docsresourcesmd)), shared by the CLI stagers and the runtimes | — |
| `day-toolchain` | one place that knows where host toolchains/SDKs live — used by the CLI, the `-sys` build scripts, and generated scaffolds | — |
| `day-lite` | dynamic miniapps ([docs/lite.md](docs/lite.md)): QuickJS runtime (`rquickjs`), oxc TypeScript stripping, the JS `day.*` API over the day-pieces dyn registry, package store (install/update/permissions), sqlite (over day-persistence's driver, so a superapp compiles one engine) + sandboxed fs, the headless test-runner core (`day_lite::run_tests`) | day-core, day-pieces (`dyn-registry`), day-part-http, day-persistence (driver only) |
| `day-break` | OPTIONAL consent-first crash reporting ([docs/break.md](docs/break.md), [§8.5](#85-panics-and-crashes)): chained panic hook + POSIX signal handlers + Android UEH, session sentinel, next-launch reconcile into a schema-versioned JSON report, pluggable `Reporter` upload (never automatic) | day-core, day-pieces (`ui`), day-part-http, day-part-deviceinfo |
| `day` | umbrella: `prelude`, `day::launch`, feature-gated re-export of the selected backend, plus `day::prefs` (day-part-prefs, default-on `prefs` feature — [docs/prefs.md](docs/prefs.md)) | all of the above |
| `toolkits/day-appkit`, `day-uikit`, `day-gtk`, `day-qt` (+`day-qt-sys`), `day-android`, `day-xaml` (+`day-xaml-sys`), `day-arkui` (+`day-arkui-sys`), `day-dom` (whose JS shim ships in `crates/day-cli/resources/web/`) | backend crates | day-spec (NOT day-core) |
| `day-cli` | the `day` binary ([§16](#16-the-day-cli)) | day-build, day-toolchain, day-fonts (+ clap, serde, `serde_norway` YAML, fluent-syntax) |

Two structural rules carried over from pane, both still enforced:

1. **Backends depend only on `day-spec`.** They never see the Piece model or the reactive graph.
   This keeps the spec surface small, keeps backends implementable in ~2–4k lines each, and makes
   the mock toolkit a true stand-in.
2. **One backend per binary.** The active toolkit is selected by cargo feature at app link time
   (`day launch -p macos-gtk` builds with `--features day/gtk`). The running `ToolkitId` is a
   process constant, which [§6](#6-styling-and-per-target-variation) exploits for zero-cost per-target styling. Cross-toolkit code paths
   (e.g. a Day Piece with per-toolkit renderers) select at link time via the registry, not at
   runtime via dynamic dispatch across toolkits. The `day` umbrella crate emits a
   `compile_error!` when more than one backend feature is enabled, and CI enumerates backend
   features explicitly (never `--all-features`).

### §3.3 Threading model and the turn state machine

- The **UI thread** is the toolkit's main thread. The reactive arena, the realized tree, and all
  `Signal` handles are **`!Send`** and live there — enforced by the type system (a compile-fail
  test in M0 asserts `Signal: !Send`), not convention.
- **Crossing threads is done with `Setter<T>`**, a `Send` (for `T: Send`) *write-only* handle
  obtained via `sig.setter()`. It holds only the generational arena key; `Setter::set(v)`
  re-enters through the backend's main-loop scheduling, checks generation liveness, and silently
  no-ops (with a once-per-callsite debug log) if the signal's scope has been disposed — async
  results racing disposal are an expected, defined event. `Signal` itself never crosses threads.
- Background work is plain threads (`std::thread::spawn`, or whatever executor the app brings);
  results re-enter via `Setter` or `day_reactive::on_main(f)` where `f: FnOnce() + Send` (so it
  cannot capture a `Signal`; capture a `Setter`). Backends implement the main-loop post
  (`Platform::post`) over `dispatch_async` / `Handler.post` / `g_idle_add` /
  `QMetaObject::invokeMethod` / `DispatcherQueue.TryEnqueue` / `uv_async_send`. (The designed
  `day::task::spawn` async executor was **not implemented** — threads + `Setter` cover the real
  apps, including the network parts.)
- **One turn state machine**, referenced by every other section (ratification: DP-17):

  1. A native callback (event, timer, `on_main` delivery) opens a **batch**; handler closures run;
     signal writes coalesce.
  2. At batch close, the **reactive drain** runs *synchronously*: memos are pull-based and
     glitch-free; effects and bindings drain from the pending queue **to fixpoint** — writes made
     during the drain extend the current drain. Queue order is (priority class: structural
     bindings first, then plain effects; scope depth ascending, so owners run before descendants;
     creation sequence). A per-drain re-run cap (~100 re-runs of one effect) panics in debug with
     the effect's `#[track_caller]` creation site and warns-and-defers in release.
  3. Size-affecting applies only *mark* layout dirty. **Layout, paint, and the release-queue drain
     run in one coalesced posted main-loop callback** — the *turn boundary*. `Setter` deliveries
     arriving outside any batch open one and schedule the posted drain.
  4. There is no per-frame tick in v1 (no portable frame clock across AppKit < 14 / Qt Widgets);
     aligning turn boundaries to CVDisplayLink / Choreographer / GdkFrameClock is post-MVP.

  `day_reactive::flush_sync()` runs steps 2–3 immediately — used by day-mock tests and dayscript's
  `wait_idle`; its scoped form `Scope::flush_now(scope)` serves the RowHost's sanctioned
  `bind_row` exception ([§10.2](#102-realization-the-rowhost-protocol)).
- **Native events are never dispatched re-entrantly.** The backend event sink may be *invoked*
  re-entrantly (Qt/GTK/Android text setters fire change notifications synchronously) but its
  contract is enqueue-only ([§8.3](#83-events)); day-core drains queued events at safe points, each as a fresh
  batch.

---

## §4 Reactive core (`day-reactive`)

### §4.1 The model: build once, bind forever

> [!NOTE]
> **Status: shipped as written**, with three deltas: the `piece_dyn` escape hatch was never
> needed and does not exist — reactive structure is `when`/`each` (plus the navigation
> containers); the advisory `day lint` heuristic for signal-reads-outside-bindings was not
> built (the shipped lint rule set is smaller, [§16.5](#165-subcommands)); and the debug-build
> runtime warning for a tracked read during `Piece::build`, described below, was not built
> either — day-core keeps no build-phase flag and day-reactive emits no such diagnostic, so a
> signal read in a component body that can never re-run is today caught by review, not by the
> runtime.

This is Day's central architectural decision and its largest departure from pane.

**Pieces are built exactly once.** A component function runs one time, creating realized nodes and
native handles. It never "re-renders". Reactivity lives in the *bindings*: every dynamic attribute
(a label's text, a toggle's state, a style property, a canvas draw closure) is an **updater
effect** — a closure that reads signals, computes a value, and writes it directly to the native
handle through the toolkit. When a signal changes:

```
signal write → (batch) → the ONE updater effect that read it re-runs
             → one native setter call (e.g. setText)
             → if the attribute affects size: mark node needs-measure, bubble dirty (§7.4)
             → incremental relayout of the smallest affected subtree
```

There is **no tree diffing** for ordinary state changes. Structural change happens only at explicit
dynamic points — `when` (conditional subtree), `each` (keyed collection), `piece_dyn` (arbitrary
swap) — and reconciliation there is local to that node and keyed. This is the SolidJS/floem model,
and it is the strongest possible answer to the requirement that *"a dependent piece of data that
changes should invalidate as little of the realized view tree as possible"*: the invalidation unit
is a single attribute of a single node.

Consequences worth internalizing (they answer most "but how does…" questions):

- Component functions run once, so they are *constructors*, not render functions. Passing data to a
  child means passing a value (static forever) or a `Signal`/`impl Fn() -> T` (live). There is no
  "props changed, child re-renders" — there are only bindings.
- There is no `@State`-by-structural-identity machinery (pane needed it because it re-rendered;
  Day does not). State is just signals created where you need them; dynamic pieces own their
  state's `Scope`, so removal disposes it ([§4.3](#43-scopes-and-disposal)).
- `if`/`for` in plain Rust run once at build time — correct for static structure. *Reactive*
  structure must use `when`/`each`/`piece_dyn`. Day catches the classic SolidJS footgun — a signal
  read in a component body that can never re-run — **at runtime in debug builds**: a tracked read
  during `Piece::build` with no live observer emits a once-per-callsite `#[track_caller]` warning
  ("this read at src/lib.rs:41 will never re-run — wrap it in a binding or use `get_untracked`"),
  asserted by day-mock tests from M1. `day lint` additionally ships a lexical heuristic for the
  same pattern (direct `.get()` in `fn … -> impl Piece` bodies), explicitly labeled *advisory* — a
  fast source-level lint cannot be sound without type information ([§16.5](#165-subcommands)).

### §4.2 Primitives

Evolved from `pane-graph` (Copy generational handles over a thread-local slotmap arena, push-pull
Clean/Check/Dirty invalidation, `set_if_changed`) with floem/leptos-style **scope ownership** added:

```rust
// all handles are Copy + !Send; all creation is attributed to the current Scope
let count: Signal<i32> = Signal::new(0);
count.get();                    // tracked read (inside a binding/effect/memo)
count.get_untracked();
count.set(5); count.update(|c| *c += 1); count.set_if_changed(5);
count.try_get();                // Option<i32> — the blessed form in closures that can outlive their scope
let tx = count.setter();        // Setter<i32>: Send write-only handle (§3.3)

let doubled: Memo<i32> = Memo::new(move || count.get() * 2);   // cached; T: PartialEq
                                                               // (Memo::new_with_eq for float-tolerance etc.)

Effect::new(move || log::info!("count is {}", count.get()));   // re-runs on change

// derive-state without effect-write loops: source is TRACKED, the callback is UNTRACKED
watch(move || count.get(), move |new, old| history.update(|h| h.push((*new, old.copied()))));

let ping: Trigger = Trigger::new();  // data-less invalidation source

// the binding primitive used by day-core (floem's create_updater):
// compute (tracked) + apply (untracked, side-effecting) — apply receives the new value.
// bind requires V: PartialEq (all day-spec attribute types implement it; DrawOp's PartialEq
// doubles as §11's skip-replay check); bind_always exists for incomparable payloads.
bind(move || count.get().to_string(),
     move |text| node.patch(|p: &mut LabelProps| p.text = text));   // sparse typed patch → Toolkit::update
```

- **Batching and ordering:** exactly the [§3.3](#33-threading-model-and-the-turn-state-machine) turn state machine — synchronous fixpoint drain,
  (priority, scope-depth, creation-seq) queue order, re-run cap with `#[track_caller]`
  attribution, one posted layout turn. Applies are equality-gated so no-op recomputes never touch
  the toolkit. (No `PartialEq`-"where available" specialization — that isn't stable Rust; the
  bound is explicit, with `bind_always`/`Memo::new_with_eq` as the escape hatches.)
- **Scheduler hook:** `install_scheduler(fn)` — each backend installs "post a callback on the main
  loop". Identical to pane's proven design.
- **Sync signals** (cross-thread reads, floem's `SyncStorage` analogue) are deliberately **out of
  scope for v1**; `Setter` and `day::task::on_main` are the only cross-thread doors. Revisit if
  real apps demand it (recorded as DP-12).

### §4.3 Scopes and disposal

Every signal/memo/effect/binding is owned by the `Scope` current at its creation; **event handlers
run under the scope current at handler registration**. day-core enters a child scope for each
dynamic region:

- `each(items, key, build)` — one child scope **per key**; when a key disappears, its scope is
  disposed: effects unsubscribed, signals dropped, native handles released.
- `when(cond, build)` — child scope per active arm.
- App teardown disposes the root scope.

Escape hatches for state that must outlive its creation site: `Signal::new_in(scope)` attributes a
signal to an explicit scope, and `Scope::detached()` creates a manually-disposed scope (the
idiom real apps use for page-outliving state — e.g. a settings page whose signals feed a
long-lived fetcher). The designed **`Store<K, T>`** keyed-state container was **not
implemented** — `each`'s `ItemSlot` projections plus plain signals have covered every real
collection so far.

**Disposal semantics (all M0 unit/property tests):**

- *Disposal during a drain* is legal: disposing a scope removes its pending effects from the queue
  (generational liveness check at pop — pane's mechanism, promoted to a documented invariant); the
  (priority, scope-depth, seq) order guarantees owners run before descendants.
- *Native release is deferred*: day-core queues all `toolkit.release` calls and drains them at the
  turn boundary; the [§8.1](#81-the-toolkit-trait) contract lets backends defer further (Qt `deleteLater`) and requires
  them to tolerate release at any main-loop-safe point.
- *Disposed-handle access*: **writes are silent no-ops** with a once-per-callsite debug warning
  (`Setter` inherits this — async deliveries racing disposal are expected); **reads panic in every
  build** — DP-18 answer (A), shipped — naming the handle's `#[track_caller]` creation location;
  `try_get`/`try_with` are the blessed forms in any closure that can outlive its scope. Event
  handlers on nodes disposed in the current drain are unregistered before their scope's signals
  drop.

`Scope::provide::<T>(value)` / `Scope::use_context::<T>()` give dependency injection down the
*build* tree (theme, locale handle, navigation), resolved at build time — again, no re-render
semantics needed.

### §4.4 Events and controlled inputs

Native events (button press, text change, slider drag) enter through the backend's **event
trampoline** (per-widget callback table keyed by node id — pane's proven design). The sink is
enqueue-only ([§3.3](#33-threading-model-and-the-turn-state-machine), [§8.3](#83-events)); day-core dispatches each queued event as a fresh batch.

Two-way controls are **controlled**, with an IME-safe protocol (pane's value-equality guard is
proven for ASCII only — it breaks CJK composition and autocorrect):

- The **native widget is the source of truth while it has focus**. Signal→native writes apply only
  when (a) the write did not originate from this widget's own change event (**origin-tagged
  writes**, not value comparison) and (b) **composition is not active** (`markedTextRange` /
  composing spans / `GtkIMContext` preedit / `QInputMethodEvent` / DOM
  `compositionstart`–`compositionend`). Programmatic writes during composition are queued until
  composition ends; mutating the buffer mid-composition is documented as unsupported.
- Echo suppression is additionally a backend duty (compare the native value before applying, with
  a per-control post-roundtrip `f64` tolerance rule for sliders), with `set_if_changed` as the
  second layer so divergent echoes survive as real events.
- Manual Japanese-IME checks are acceptance items in M2 (AppKit) and M5 (iOS Simulator); the mock
  toolkit's reentrancy test (apply triggers a synchronous synthetic echo; assert no double-borrow,
  no lost divergent value) lands in M0–M1.

High-frequency events (slider drag) apply value writes per event; layout coalesces to the turn
boundary ([§3.3](#33-threading-model-and-the-turn-state-machine)).

### §4.5 Async

> [!NOTE]
> **Status: shipped 2026-07** ([docs/async.md](docs/async.md) is the normative reference) with three divergences
> from the recorded design below. (1) The executor is day-core's existing main-loop `day::task`
> (§3.3's `present().await` executor, now with `TaskHandle`/abort), reached from day-reactive
> through an installed `install_spawner` hook returning an abort closure — not a generic
> `spawn(F: Future + MaybeSend)`; the `MaybeSend` cfg-alias collapsed because the futures run
> on the UI thread, where `S` needs only `Clone + PartialEq` and the fetcher may touch signals
> after its awaits. (2) Results are therefore plain signal writes guarded by the generation
> check — no `Setter` in the delivery path. (3) The fetcher's output is
> `Result<T, E: Error + Send + Sync>` (`Infallible` for infallible fetchers). Superseded and
> disposed fetches ARE aborted — dropping the task's future cancels any
> `day_part_http::FetchFuture` inside (the cancel matrix in [docs/http.md](docs/http.md)). The pre-`Resource`
> idiom (thread + `Setter`) remains valid for callback-style parts; [docs/async.md](docs/async.md) carries the
> full policy, including the app-private tokio-quarantine rule the Matrix client models.

```rust
// two-closure shape (leptos-style), as shipped in day-reactive (`day::reactive::Resource` —
// namespaced: the prelude's `Resource` is the asset handle, §18.3):
let stations = Resource::new(
    move || region.get(),                       // S: Clone + PartialEq — tracked; refetch on change
    |region| async move { fetch_stations(region).await },
);
// stations.signal(): Signal<Load<Vec<Station>>>; Load: Clone
// Load::Loading | Load::Ready(T) | Load::Failed(Arc<dyn Error + Send + Sync>)
when(move || stations.ready(), move || station_list(stations));
stations.refetch(); stations.loading();
```

Latest wins by fetch generation: a source change (or `refetch()`) supersedes the in-flight
fetch — its task is aborted, and a completion that slips through writes nothing; scope disposal
aborts the same way. The source's value moves into the future, so no `!Send` `Signal` ever
crosses a thread boundary *by construction* — exactly as designed, just with the thread
boundary gone.

---

## §5 The Piece model (`day-core`)

### §5.1 Authoring surface: functions and builders, no macros

Per the project mandate (and floem's demonstration that it works at scale), the API is **plain Rust
functions returning piece values, configured by builder methods**. There is no required macro
anywhere in the framework. (No `view!{}`, no `#[component]`. Optional future sugar must lower to
this API.)

```rust
use day::prelude::*;

pub fn counter() -> impl Piece {
    let count = Signal::new(0);

    column((
        label(tr("counter-value").arg("count", count)),
        row((
            button(tr("decrement")).action(move || count.update(|c| *c -= 1)),
            button(tr("increment")).action(move || count.update(|c| *c += 1)),
        ))
        .spacing(8.0),
    ))
    .spacing(12.0)
    .padding(16.0)
}
```

Components are **plain functions** (any `fn(…) -> impl Piece`). Refactoring is ordinary Rust
refactoring. Children are **tuples** (`PieceSeq` implemented for tuples up to arity 16, plus
`column_iter`/`row_iter` for static iterators, plus `each` for reactive collections).

Authoring-surface edges, specified now so they don't accrete ad hoc:

- **`PieceSeq` flattens recursively** — a tuple containing a `PieceSeq` contributes its children
  in place with no extra node — and `PieceVec(Vec<AnyPiece>)` covers the runtime-heterogeneous
  case (`row(PieceVec(stars))`). A build-time heterogeneous branch takes `Either<A, B>`
  (`if compact { Either::Left(a) } else { Either::Right(b) }`), which keeps both arms concrete;
  `Decorate::any` erases when a single `AnyPiece` is what's actually needed. `AnyPiece::any` is
  inherent and returns `self`, so re-erasing an erased piece costs nothing.
- **Deferring is not erasing.** A piece whose body must run at BUILD time (it reads an ambient
  `environment`, a scope, or the laid-out size) defers into `piece_fn`, which returns the
  concrete `PieceFn<F>` — so `canvas`, `frame_clock`, `shape_group`, `shape_group_fn`, `each`
  and `with_environment` all return `impl Piece` and stay unboxed in the caller's type. The two
  form constructors are the deliberate exception: `form` and `labeled` erase, because their
  results are collected far more often than consumed inline, and a flat row type keeps a
  form — the densest surface an app has — from nesting one closure type per row.
- **Closure capture rules**: the builder closures of `when`/`each` are `Fn` (they may
  run more than once); non-`Copy` captures must be cloned per activation
  (`let items = items.clone();` inside the closure, or capture a `Signal` — signals are `Copy`,
  which is why the idiomatic Day style keeps shared state in signals). The M2 template and
  showcase demonstrate one non-`Copy` capture deliberately.

### §5.2 The `Piece` trait

> [!NOTE]
> **Status: shipped as written**, with one revision (2026-08-24): `form` and `labeled` were the
> last built-in constructors returning `AnyPiece`. They now return `Form<C>` and `Labeled<P>`, so
> no constructor in day-pieces erases and `.any()` is always the caller's explicit choice. That
> erasure had been justified as bounding monomorphization; measured on Day-Showcase (116 `labeled`
> rows) for a macos-appkit release build, removing it cost +90 KB of machine code (+0.81% of
> `__text`, +0.40% of the stripped executable) and +10% on that crate's compile time.

A Piece value is a *description consumed once*:

```rust
pub trait Piece: 'static {
    fn build(self, cx: &mut BuildCx) -> NodeId;   // realize into the tree, return the root node
}
pub struct AnyPiece(Box<dyn FnOnce(&mut BuildCx) -> NodeId>);  // for heterogeneous/dynamic cases
pub trait IntoPiece { fn into_piece(self) -> …; }              // &str → label, etc. (sparingly)
```

`BuildCx` provides: the current parent node, the current `Scope`, the toolkit (via `day-spec`),
context lookup, and locale/theme handles. `build` for a leaf: create native handle through the
renderer registry, create updater-effect bindings for each dynamic attribute, insert into parent.
`build` for a container: create the container node (native container view), enter it, build
children. Concrete piece structs (`Label`, `Button`, `Column`…) are public so builder methods are
inherent methods (good rustdoc, good autocomplete) — the modifiers that apply to ANY piece
(`padding`, `id`, `a11y`, `background`, `on_tap`…) come from a blanket `Decorate` extension trait,
while a modifier only some pieces can honor stays an inherent method on those pieces: `style` is
typed per piece (`Picker::style` takes a `PickerStyle`), and `enabled` needs a control to gray out
(`Button::enabled`, `Toggle::enabled`).

**Modifiers do not erase.** Each `Decorate` method returns `Decorated<Self>` — the piece plus an
ordered op list — so the concrete piece type survives a chain; inherent methods on `Decorated<P>`
shadow the trait's, keeping the chain flat rather than nesting. A piece's own builders are also
declared in a `*Builder` trait (`LabelBuilder`, `ButtonBuilder`, `ColumnBuilder`, `RowBuilder`)
that `Decorated<P>` forwards through `map_inner`, so `label(…).padding(8.0).font(…)` resolves and
there is no "typed modifiers must come first" ordering rule. Toolkit and `day-tweak-*` extension
traits follow the same signature ([docs/tweaks.md](docs/tweaks.md), [docs/api-style.md](docs/api-style.md) "Typed builders and erasure").
`Decorate::modifier` is the one erasing modifier, because `Modifier` is defined over `AnyPiece`.
Annotating ops (`id`, `selectable`, `grid_span`) still target the node built so far, which is why
grid facts are documented as applying LAST.

**Constructors do not erase either.** A constructor returns its own concrete piece — `label()` →
`Label`, `column()` → `Column<C>`, `labeled()` → `Labeled<P>` — or, where the body must wait for
the build to read an ambient `environment` or a laid-out size, an `impl Piece` over `PieceFn<F>`
(`canvas`, `each`, `with_environment`). A deferred piece worth naming defers inside its own
`build` instead: `Labeled` reads the enclosing form's shared label column there and stays a plain
struct. `AnyPiece` appears only where a boundary needs ONE type — a stored `Rc<dyn Fn() ->
AnyPiece>` (nav destinations, `when` arms, window and preferences builders), a `PieceVec`, or a
build-time branch between two piece types that does not use `Either<A, B>`. Erasure is the
caller's `.any()`, never the constructor's.

### §5.3 Built-in pieces (MVP set)

> [!NOTE]
> **Status: shipped and outgrown.** The design-era "MVP set" grew into the full vocabulary
> below, which reflects the prelude as it exists in day-pieces. Deltas from the original text:
> `stack_z` shipped as `zstack`; `piece_dyn` was never needed (structure is `when`/`each` plus
> the navigation containers); the gesture decorators shipped as `.on_tap`/`.on_drag` (context
> menus are declarative — `.context_menu(items)`, [docs/menus.md](docs/menus.md)). Three modifiers the design-era
> §5.2 text listed as `Decorate` members never shipped there: `disabled` is spelled `enabled` and
> is per-piece (`Button`, `Toggle`) because it needs a native control to gray out, and `visible`
> and `on_key` do not exist at all — hide a subtree with `when`, and while `Event::Key` rides the
> event sink, no piece-level API consumes it yet. `Decorate` did instead grow the
> transform family (`.opacity()`, `.rotation()`, `.scale()`, `.translation()`, `.transform()`) and
> `.animation()`. Per-subsystem detail lives in the docs/ files named in the subsystem index.

```rust
// text & controls — two-way controls take `impl Binding<T>` (Signal<T>, or a projection):
label(text)                        // text: impl IntoText — value, Signal<String>, closure, or
                                   //   LocalizedText; styled via .font(Font::Headline) / .color(c)
    .monospace().runs(runs)        //   .runs()/.runs_from(TextBuilder): styled runs in ONE label,
                                   //   so it wraps, selects and reads as one (docs/text-runs.md)
    .markdown().on_link(f)         //   inline markdown parsed at RUN TIME — the case a macro
                                   //   cannot serve, since the string is a translation or typed
                                   //   (docs/markdown.md); a link run reports through on_link
link(text, url)                    // tappable accent text → opens url in the system browser /
                                   //   default handler (§8.1 open_url); .font() / .color() / .bold()
button(text).action(f)             // .bordered() / .prominent() / .tint(color) (docs/buttons.md)
toggle(on)                         // two-way bool
slider(value).range(0.0..=100.0)   // two-way f64; .step(…)
text_field(text).placeholder(p).on_submit(f)   // two-way String; focus via .focused(…) (docs/focus.md)
text_area(text).min_lines(3).max_lines(8)      // two-way String, multi-line (docs/textarea.md)
    .editable(e).selectable(s).spellcheck(sc)  // reactive attrs; Cap::Text{Editable,Selectable,SpellCheck}
picker(opts, idx).segmented()      // one-of-N: .menu()/.segmented()/.inline() (docs/picker.md)
progress(fraction)   spinner()     // docs/progress.md
image(res::images::logo)           // typed resource constants (§18.5)
vector(res::vectors::home)         // resource/vectors/ glyph + .tint(color) (docs/vectors.md)
divider()   spacer()

// layout containers
column(children).spacing(8.0).align(HAlign::Leading)
row(children).spacing(8.0).align(VAlign::Center)
    .fit(RowFit::Wrap { run_spacing })   // what happens when the row outgrows its width
    .fit(RowFit::WrapColumns { run_spacing })     //   (docs/size-classes.md "Row fit
    .fit(RowFit::ColumnAt(WidthClass::Compact))   //   policies"); Wrap is ragged, WrapColumns
    .fit(RowFit::Scroll)                 //   uniform; default Clip logs overflow in debug
zstack(children)                   // overlay
grid((grid_row((…)), …)).spacing(8.0)   // SwiftUI-style eager grid (docs/grid.md): columns
                                   //   infer from cells; .grid_span(n)/.grid_align(a) per cell
scroll(child)
form((section((…)).title(t), …))   // grouped platform forms (docs/forms.md)
labeled(caption, control)

// structure
when(cond_fn, build_fn)            // reactive conditional subtree
    .otherwise(build_fn)           //   optional else arm; without it, false builds nothing
each(items_fn, key_fn, build_fn)   // reactive keyed collection (§5.4)
list(items_fn, key_fn, row_fn)     // NATIVE recycling list (§10, docs/list.md)
tree(source, row_fn)               // hierarchical tree (docs/tree.md): token-addressed rows,
                                   //   app-owned expansion, drag-to-reparent; sources are
                                   //   branches(items, key, parent) or store.tree(children_of);
                                   //   NATIVE where Cap::Tree says so (appkit/gtk/uikit),
                                   //   COMPOSED onto list() everywhere else (web-dom, qt)

// navigation & presentation (docs/navigation.md, docs/cover.md, docs/dialogs.md, docs/menus.md, docs/files.md)
selector(section)                  // sidebar / tabs / segmented, per SelectorStyle
    .content_list(build)           //   the Mail shape's middle column (2026-08): a resident
                                   //   Pane::List page — a real contentList split item on
                                   //   appkit, the uikit triple-column supplementary column,
                                   //   composed beside the detail elsewhere
                                   //   (Cap::NavContentList); .content_list_for(pred) collapses
                                   //   it per destination, .detail_visible(sig) gates the
                                   //   compact push flow two-way — a nested nav host inside a
                                   //   tab, a merged push while stacked — and
                                   //   .detail_title(text) names the detail layer's bar, live
stack(path, root)                  // push/pop navigation bound to a Vec<Route> signal
cover(open, build)                 // fullscreen modal surface bound to a Signal<Option<Route>>
inspector(visible, content, panel) // trailing properties pane bound to a Binding<bool>; native
                                   //   split where Cap::Inspector is Native, composed pane +
                                   //   compact-width sheet elsewhere (docs/inspector.md);
                                   //   .edge(PaneEdge::Leading) makes it a leading utility
                                   //   pane (a layer panel, docs/tree.md)
nav_link(…)   navigate_to(…)   current_route()   route_param(…)
alert(…)   confirm(…)   prompt(…)   open_file(…)   save_file(…)
app_menu(…)   menu_item(…)   sub_menu(…)   menu_role(…)   menu_separator()

// drawing (§11, docs/shapes.md)
canvas(draw_fn)
rectangle()  rounded_rectangle(r)  circle()  capsule()  ellipse()  arc(start, sweep)
line(a, b)  polygon(points)        // unit-point kinds over the existing Line/Polygon ops
    .fill(color) / .fill_linear(g) / .fill_radial(g) / .stroke(color, w)
    .rotate(deg) / .inset(v) / .offset(x, y)      // reactive: any of these takes a closure
    .at(fx, fy, fw, fh)            // fractional sub-rect placement (glyph composition)
shape_group(shapes)  shape_group_fn(size_fn)      // many shapes, ONE canvas leaf (§3.6 there)

// ambient environment
with_environment(value, build_fn)   environment::<T>()
```

The **`Decorate`** extension trait carries the universal modifiers: `.id()` / `.id_keyed()`,
`.padding()`, `.frame()` / `.width()` / `.height()`, `.grow()` variants, `.background()`,
`.corner_radius()`, `.overlay()` / `.overlay_aligned()`, `.grid_span()` / `.grid_align()`
([docs/grid.md](docs/grid.md); inert outside a grid), `.a11y()`, `.on_tap()` / `.on_drag()` /
`.on_pinch()` / `.on_pan()` (the continuous zoom/scroll pair, [docs/canvas.md](docs/canvas.md)), `.focused()`,
`.on_key()` (the non-text keys, delivered only while THIS piece has focus — [docs/menus.md](docs/menus.md)),
`.focusable()` (opt a composed container into the focus system — the canvas contract behind
`Toolkit::set_focusable`, [docs/focus.md](docs/focus.md); appkit today, a no-op elsewhere),
`.selectable()` (make text user-selectable — routed to `Toolkit::set_selectable`, [docs/text.md](docs/text.md)),
`.context_menu()`, `.defers_system_gestures()` / `.interactive_dismiss_disabled()`
([docs/cover.md](docs/cover.md)), `.tweak()` / `.native_ref()` ([docs/tweaks.md](docs/tweaks.md)), `.modifier(impl Modifier)`,
and `.any()`.

Beyond the built-ins, optional widgets ship as ordinary crates under `pieces/` (`combo_box`,
`search_field`, `rating`, `activity`, `web_view`, `media`, `map`, `lottie`, `remote_image`,
`color_picker` ([docs/colorpicker.md](docs/colorpicker.md)), `stepper` — a numeric field with
increment/decrement arrows, [docs/stepper.md](docs/stepper.md) —
`swiftui` — hosted SwiftUI views, [docs/swiftui.md](docs/swiftui.md)) and headless services under
`parts/` (battery, network, sensors,
clipboard, prefs, haptics, deviceinfo, http, fs) — [§15](#15-extensibility-pieces-parts-and-tweaks) has the extension model.

Example — the shipped composition idiom (from the showcase's Controls page; the live app is the
complete reference, [Appendix A](#appendix-a--the-showcase-app-end-to-end)):

```rust
fn basics_section() -> impl Piece {
    let name = Signal::new(String::new());
    let volume = Signal::new(40.0f64);
    let subscribed = Signal::new(false);

    section((
        text_field(name)
            .placeholder(res::str::name_placeholder())
            .id("name-field"),
        when(
            move || !name.with(|s| s.is_empty()),
            move || label(res::str::greeting(name)).id("greeting-label"),
        ),
        labeled(
            res::str::volume_label(),
            row((
                slider(volume).range(0.0..=100.0).id("volume-slider"),
                label(move || format!("{:.0}", volume.get())).id("volume-value"),
            ))
            .spacing(8.0),
        ),
        labeled(res::str::subscribe_label(), toggle(subscribed).id("subscribe-toggle")),
    ))
    .title(res::str::controls_basics())
}
```

### §5.4 Keyed collections: `each`

> [!IMPORTANT]
> **Status: shipped with deltas.** The unified `ItemSlot` contract is real (`ItemSlot<T, K>`:
> tracked `get()`/`with()`, `field()` projections, `key()`; keyed diff with per-key scopes,
> slot writes for surviving keys, debug key-uniqueness assertion), and `each` and `list` share
> it as designed. The `slot.rw(get, set)` two-way projection and the `.on_edit` write-back hook
> were **not implemented**, and day-model superseded them (2026-08, [docs/model.md](docs/model.md)):
> `each`/`list` take a **`RowSource`** — plain data wrapped as `items(closure, key_of)`, or a
> day-model store passed directly (`store.rows(projection)` for display order) — and a store
> source's rows receive a **`ModelSlot`**, itself a day-model `Source`, so `slot.done()` binds
> two-way and follows the row across recycling. The sample below is the shipped shape; plain
> collections keep `ItemSlot` with one-way `item.field()` projections.

**Resolved (DP-16: unified).** `each` and the native-recycling `list` ([§10](#10-native-list-integration)) share **one item
contract**: the builder receives an **`ItemSlot<T>`**, never the item by value. The same row
function serves both, so moving a collection from `each` to `list` is a one-word change.

```rust
#[derive(Observable, Clone, Default, PartialEq)]
struct Todo { #[obs(key)] id: u64, title: String, done: bool }

let todos: Store<Keyed<Todo>> = Store::new(Keyed::default());   // per-property (docs/model.md)

column((
    each(todos, move |item: ModelSlot<Todo>| {
        row((
            toggle(item.done()),                       // two-way: a day-model Field IS a Binding (§5.3)
            label(move || item.title().read()),        // wakes only for THIS row's `title`
            spacer(),
            button(icon("close"))
                .action(move || todos.restructure("remove", Op::Delete, item.key(), |v| {
                    v.remove(item.key());
                }))
                .a11y(|a| a.label(tr("todo-remove")))
                .id_keyed("todo-remove", item.key()),  // stable per-item id (§5.5)
        )).spacing(6.0)
    }),
))
// The store's SHAPE is the tracked row set: a field edit re-runs no items closure and rebuilds
// no rows — only the one control bound to the edited field patches. Plain data reads
// `each(items(closure, key_of), row)` instead.
```

Semantics (identical for `list`):

- `each` re-runs only its *items* closure when the source changes, then performs a **keyed diff**
  (order + set; longest-increasing-subsequence move minimization, as floem's `dyn_stack` does).
  Only inserted/removed/moved keys touch native children; **surviving keys are not rebuilt** —
  their slot receives the new value in place.
- `ItemSlot<T>` is `Copy`. `slot.get()` is a tracked read of the whole item; `slot.field(f)` is a
  per-field memoized projection (`V: PartialEq` — its bindings re-run only when *that field's*
  value actually changed); `slot.key()` is the key. Slot writes on surviving keys are
  unconditional; the field projections are the equality gate (no `T: PartialEq` bound on items,
  no specialization).
- Value changes therefore **propagate automatically**: mutate the source
  (`todos.update(…)`) and every affected row updates fine-grained — the silent-staleness hole of
  a captured-by-value item cannot exist.
- Two-way controls bind through a day-model field accessor
  (`toggle(todos.elem(slot.key()).done())`, [docs/model.md](docs/model.md)): the write lands in the store,
  the change log carries it, and every reader — this row's control included — follows from the
  store's own notification. The designed `slot.rw(get, set)` + `.on_edit(key, &T)` write-back
  protocol was never needed and did not ship.
- Debug builds **assert key uniqueness** per diff, panicking with the duplicate key and `each`'s
  creation site (floem's `dyn_stack` corrupts silently on duplicates).
- Reactive *structure* inside a row still uses `when`/`piece_dyn` — deriving structure from
  `slot.get()` in plain Rust freezes at first bind (the [§10.1](#101-api--the-shared-itemslot-contract-unified-with-each--dp-16-resolved) trap; same rule here, same lint).
- The keyed model-layer container that shipped is day-model's `Store<Keyed<T>>`
  ([docs/model.md](docs/model.md)) — the design-era `Store<K, T>`/`each_store` adapter pair never did
  ([§4.3](#43-scopes-and-disposal)). Items whose `T` carries `Signal` handles remain legal, and plain data + slots
  stays the blessed default for view-local collections.

### §5.5 Node identity, ids, and the element index

Every realized node has a `NodeId` (slotmap key). Separately, `.id("volume-slider")` assigns a
**stable string identifier**, and `.id_keyed("todo-remove", key)` its keyed form for collection
items (rendered as `todo-remove:<key>`; `day lint` enforces prefix uniqueness). Three consumers:
the platform automation/accessibility identifier where one truly exists (the verified per-toolkit
matrix is in [§13](#13-accessibility) — notably Android has **no** external automation-id channel below API 33, and GTK
has none at all today; the doc does not pretend otherwise), the dayscript element index ([§14](#14-scripting-dayscript),
which reads day-core directly and therefore works uniformly regardless of platform id support),
and `day lint` uniqueness checks. Ids are the contract between the app and its tests; a lint rule
forbids leaking them into `contentDescription`/a11y labels (screen readers would speak them).

---

## §6 Styling and per-target variation

### §6.1 Style as a value, applied through a builder closure

> [!IMPORTANT]
> **Status: shipped differently.** The designed `Style` struct + `.style(|s| …)` closure never
> shipped. Styling is **direct builder methods** on the piece and on `Decorate`, reactive like
> every other attribute:

```rust
label(res::str::title())
    .font(Font::Title)                        // semantic text style (§6.4)
    .color(Color::hex(0x2E6FB8))              // or a closure: .color(move || if err.get() { … } else { … })
column((…))
    .padding(12.0)
    .background(Color::hex(0xF4F4F6))
    .corner_radius(6.0)
```

The named-`Style`-value layer can be added later as sugar over these methods without breaking
anything; nothing has needed it. `ButtonStyle` (`.bordered()`/`.prominent()`/custom impls) and
`SelectorStyle` (sidebar/tabs/segmented) are the two piece-specific style enums that did ship.

Style properties remain **honest about native limits**: each documents its per-toolkit mapping
(e.g. `corner_radius` → CALayer / GTK CSS provider / QSS / drawable), and the surface is a
curated set every backend implements or explicitly declines — not a CSS engine. Grouped-surface
styling (the [§5.3](#53-built-in-pieces-mvp-set) `form`/`section` cards) travels as a semantic `SurfaceRole`, which each backend
resolves to its platform's own material (e.g. `quaternarySystemFill` on macOS 14+).

### §6.2 Per-target variation: `PerTarget<T>` values (no macros)

> [!IMPORTANT]
> **Status: shipped differently.** The `per_toolkit()`/`PerTarget` value combinators and
> `style_on` never shipped. Per-target variation in practice is **plain Rust over compile-time
> constants** — one backend per binary means `cfg` and feature flags resolve everything
> statically:

```rust
// OS-level branches: ordinary cfg (the map page exists only on Apple targets)
#[cfg(any(target_os = "macos", target_os = "ios"))]
let nav = nav.item_icon(Section::Map, …);

// toolkit-level branches: the backend cargo feature (one per binary, §3.2)
let pad = if cfg!(feature = "qt") { 8.0 } else { 12.0 };
```

In practice per-target styling has barely been needed: semantic fonts ([§6.4](#64-typography)), semantic surface
roles ([§6.1](#61-style-as-a-value-applied-through-a-builder-closure)), and native controls absorb most platform variation by construction. The value-
combinator design (from the `platform!{}` exploration in `pane/DESIGN.md` [§4](#4-reactive-core-day-reactive)b) is kept here as
the recorded shape sugar could take if branching ever becomes common.

### §6.3 Semantic theme tokens

> [!IMPORTANT]
> **Status: shipped differently.** There is no `theme::` token module. Native fidelity comes
> from a different split: **default appearance is native by construction** — text, controls,
> separators, form cards, and window grounds take the platform's own dynamic colors inside each
> backend (`NSColor.labelColor`, `?attr/colorOnSurface`, QPalette roles, XAML theme resources),
> so dark/light tracking needs no app-side tokens at all. Apps state only *deliberate* colors
> (`Color::hex(…)` brand values, shape fills, gradients). Semantic *roles* that must cross the
> spec do so as typed values: `SurfaceRole` for grouped-card surfaces, `Font` for typography.
> Forced schemes for screenshots/CI ride the `DAY_THEME=light|dark` launch environment, which
> every backend honors (per-element on XAML islands, palette on Qt ≤6.7, color-scheme
> elsewhere). An app-wide token module remains possible later; no real app has needed one.

### §6.4 Typography

> [!NOTE]
> **Status: shipped as written** (as an enum rather than constructor fns; no `env::font_scale`
> signal — scaling is applied inside the backends).

`Font` is **semantic-first**: an enum of the platform text styles — `LargeTitle`, `Title`,
`Title2`, `Title3`, `Headline`, `Subheadline`, `Body` (default), `Callout`, `Footnote`,
`Caption`, `Caption2` — resolving to the platform's text-style system
(`UIFont.preferredFont(forTextStyle:)`, Android textAppearance-class scaled sizes,
`NSFont.preferredFont`, documented ramps on gtk/qt) so **Dynamic Type / system font scaling
works by default**. `Font::System(pt)` is the raw-size escape hatch, still scaled by the
platform's accessibility text factor (UIFontMetrics / `sp` / GTK text-scaling-factor);
`Font::Custom(family, pt)` selects a bundled font by family name ([§18.4](#184-bundled-custom-fonts-docsresourcesmd)). `FontWeight` and
italic ride the same spec ([docs/text.md](docs/text.md)). A points-first API would have made Dynamic Type
unfixable later; this one has been semantic-first from the start.

---

## §7 Layout

### §7.1 Day owns layout

Native components are *placed* by day. Every backend exposes two core geometry duties:
`measure(handle, proposal) -> Size` (native intrinsic measurement — text, control chrome) and
`set_frame(handle, rect, anim)` (absolute placement, in points; the backend multiplies by
scale/density). Containers are dumb native panels (`NSView`/`PaneFixed`-style absolute
`ViewGroup`/`GtkFixed`+custom layout manager/bare `QWidget`/`Canvas` panel/absolutely-positioned
`<div>`) — all six proven in pane/hop, including the GTK shrink fix (custom `GtkLayoutManager`
reporting min 0) and Qt child-clipping caveats.

**Coordinate spaces, precisely:** `set_frame` rects are expressed in the **nearest realized
*native* ancestor's** coordinate space. Layout-only wrapper nodes ([§7.3](#73-alignment-frames-and-modifiers) decorators, alignment
wrappers) have no native handle; day-core accumulates their offsets when emitting frames. This
rule is what permits a later optimization — flattening pure-layout containers out of the native
tree entirely — as a non-breaking day-core change.

Exceptions where the native container drives: `scroll` ([§7.6](#76-scroll) — Day measures content, native owns
the viewport) and `list` ([§10](#10-native-list-integration) — native recycling owns the viewport).

### §7.2 The protocol: parent proposes, child chooses

SwiftUI's model, as implemented twice in this lineage (hop's engine for the four desktop toolkits;
pane's re-implementation):

```rust
pub struct Proposal { pub width: Option<f64>, pub height: Option<f64> }  // None = unconstrained

pub trait Layout: 'static {
    fn measure(&self, cx: &mut MeasureCx, children: &[ChildRef], p: Proposal) -> Size;
    fn place(&self, cx: &mut PlaceCx, children: &[ChildRef], bounds: Rect);
}
```

- Leaves answer `measure` by asking the toolkit; **text is height-for-width** (measure(width=W)
  returns wrapped height). Desktop incantations are hop-proven (`cellSize(forBounds:)`,
  GTK/Qt height-for-width shims). The mobile incantations are specified here and validated in M5
  (hop has no mobile backends): Android width-bounded probes use
  `View.measure(AT_MOST(w·density), UNSPECIFIED)` — **not** `EXACTLY`, which would force the child
  to report width=w and break child-chooses; UIKit uses `sizeThatFits(CGSize(w, .greatestFiniteMagnitude))`
  / `systemLayoutSizeFitting`. M5 acceptance includes a wrapping-label reflow test on both
  Simulator and emulator.
- `column`/`row` implement the SwiftUI-style flexible-space negotiation (rigid children first,
  remaining space divided among flexibles by priority; `spacer()` is a maximally-flexible child).
- **Child layout facts:** a parent cannot see into a child's wrappers, so `ChildRef` exposes a
  read-only facts surface — `priority()`, `is_spacer()`, `flexibility(axis)` — populated by
  decorator wrappers and leaves and forwarded through wrappers unless overridden (hop's
  `greedyAlong` precedent). (An earlier draft put `priority(child)` on the `Layout` trait itself;
  that is dead API — the parent's impl has no way to know a child's `layout_priority` wrapper.)
  *Shipped form:* the facts surface is the `Flex` struct (`grow_w`/`grow_h`/`is_spacer`/
  `is_group`), read via `LayoutOps::flex_of`; it grew a `grid: GridFacts` field (row marker,
  span, cell alignment) for `GridLayout` — the grid's cell metadata rides the same channel
  ([docs/grid.md](docs/grid.md)). Numeric `priority()` remains unimplemented.
- **The `Layout` trait is public and open** — a custom container (flow layout, masonry) is a piece
  whose node carries a user `Layout` impl. Built-ins use the same trait (no private privileges).
  This satisfies "flexible and extensible" without adopting Taffy; the web-flexbox model fights
  native height-for-width measurement and proposal negotiation (DP-11 records the Taffy
  alternative and why we recommend against it).

### §7.3 Alignment, frames, and modifiers

`frame(width, height, min_*, max_*, align)`, `padding`, `offset`, `fixed_size()`, `layout_priority(n)`
are layout-affecting decorators implemented as wrapper nodes with trivial `Layout` impls — no
special cases in the engine. Alignment and insets are **logical by default** (`HAlign::Leading`/
`Trailing`, `Insets::leading/trailing`), resolved against the layout direction at place time
([§7.8](#78-rtl-and-bidi)).

### §7.4 Incremental relayout and the measurement cache

Proposal negotiation multiplies measure probes down the tree, and on Android/UIKit every leaf
measure is an FFI round-trip — so the cache is not an optimization, it is part of the design
(floem gets this from taffy; neither hop nor pane solved it, and both simply re-ran full layout):

- **Per-node measure cache** keyed by quantized `Proposal` (+ layout direction + density epoch),
  invalidated by the node's `needs_measure` generation. `MeasureCx` answers child measures from
  cache before delegating. Probes are bounded (≤3 distinct proposals per child per pass —
  SwiftUI's own ceiling). Leaf text measurement additionally caches on
  (text, font, resolved width) for android/uikit.
- **Measure-call counts are part of the M1 day-mock golden tests** — the fine-grained claim is a
  regression test for layout too.

When a binding changes a size-affecting attribute:

1. The node is marked `needs_measure`; the dirt bubbles to the nearest **layout boundary** — a
   node whose size is externally fixed **on both axes** (explicit two-axis `frame`, the window
   root, a scroll node, a `RowHeight::Uniform` list cell). One-axis frames are *not* boundaries
   under height-for-width. `RowHeight::Automatic` list cells are boundaries **with notification**
   ([§10.2](#102-realization-the-rowhost-protocol)).
2. At the turn boundary, relayout **re-enters at each dirty subtree's boundary** and runs a normal
   measure+place pass from there: clean descendants answer from the proposal-keyed cache, and
   place-recursion prunes subtrees whose (proposal, size, origin) are all unchanged. A scroll
   boundary re-runs its *content* layout and emits a content-size update ([§7.6](#76-scroll)).
3. `set_frame` is diffed with a half-device-pixel epsilon ([§7.9](#79-pixel-snapping-and-density)), so a text change that moves
   nothing results in exactly one native `set_text` and zero frame calls.

Note the soundness subtlety the naive version misses (and which is an M1 mock test): "unchanged
size stops propagation" is only valid **because the pass re-entered at a boundary whose own
proposal is unchanged** — a dirty child's size change alters its *siblings'* proposals inside a
negotiated stack, so propagation stops at negotiation scopes, not at arbitrary nodes.

### §7.5 Window sizing

- **Minimum size** comes from measuring the root under `Proposal { width: Some(0), height: Some(0) }`
  (what is the smallest you can be?), clamped up to a small platform default, overridable via
  `WindowOptions::min_size`. `measure(unconstrained)` provides only the *initial/ideal* size.
  (Deriving min from the unconstrained ideal produces unshrinkable windows — the exact hop lesson
  [§7.1](#71-day-owns-layout) cites, reintroduced at the window level.)
- Relayout runs with the actual size on every native resize, so text reflows.
- **Locale switches** recompute the minimum; the window grows if it is below the new minimum and
  never auto-shrinks. A locale-switch relayout benchmark is an M6 acceptance item.

### §7.6 Scroll

Scroll is in day-spec **v1** (it is M2 and the showcase root; pane has zero scroll precedent and
hop needed a dedicated protocol — this cannot be retrofitted after the spec freeze):

- Day measures the content subtree (unconstrained on the scroll axis), calls
  `set_scroll_content(handle, content_size)`, and lays out content children inside the native
  content coordinate space. Per-toolkit mapping: `NSScrollView.documentView` frame /
  `UIScrollView.contentSize` / `GtkScrolledWindow` child min-size / `QScrollArea` widget resize /
  Android content-`ViewGroup` that stores the size and reports it from `onMeasure` under
  `UNSPECIFIED` / DOM overflow element.
- **Axis.** Scroll defaults to the vertical axis; `scroll(child).horizontal()` (or
  `.axis(Axis::Horizontal)`) flips it. The axis rides `realize` as
  `day_spec::props::ScrollProps { horizontal }`, and each backend maps it to its native scroller:
  `NSScrollView` horizontal/vertical scrollers, `GtkScrolledWindow` per-axis policy, `QScrollArea`
  bar policy, Android `HorizontalScrollView` vs `ScrollView`, XAML `ScrollViewer` scroll modes,
  ArkUI `Scroll` direction. Content is measured unconstrained on the chosen axis.
- The native side owns the viewport, physics, indicators, and emits `Event::ScrollChanged(Point)`.
  `Toolkit::scroll_to(handle, target_rect, animated)` and `scroll_offset(handle)` complete the
  surface. Shipped riders (2026-07, [docs/scroll.md](docs/scroll.md)): the app-side
  `scroll(child).scroll_target(signal)` builder (a `Signal<Option<ScrollTarget>>` of
  Top/Bottom/Leading/Trailing/Offset/Id), `TreeOps::{scroll_to_target, scroll_reveal}` composing
  reveal-rects in core, and the dayscript `scroll_to` step — one rail, every backend.
- On content relayout the offset is preserved, clamped to the new extent.
- **v1 restrictions, linted:** same-axis nested scrolls and `list`-inside-`scroll` are
  unsupported (`day lint` rule); cross-axis gesture arbitration is documented post-MVP work.

### §7.7 Safe areas, insets, and the keyboard

> [!IMPORTANT]
> **Status: partially shipped.** Safe-area insets are applied at the window root by the mobile
> backends (UIKit pins the root inside the window's `safeAreaInsets`; Android is edge-to-edge
> with the root held in a margin-inset wrapper), and on every backend the root's size changes —
> late inset passes, rotation, bar changes — flow to Day as `Event::WindowResized`, the same rail
> AppKit uses, so layout follows the safe area instead of a launch-time snapshot. UIKit's rail is
> the holder view's layout pass (`DayHolderView.layoutSubviews`, fixed 2026-07: rotation used to
> leave the root at its launch frame). **Keyboard avoidance shipped (2026-07,
> [docs/focus.md](docs/focus.md)):** each mobile backend consumes the keyboard natively and resizes the Day root
> through the `WindowResized` rail — Android folds `WindowInsetsCompat.ime()` into the root
> margins, UIKit observes `UIKeyboardWillChangeFrame` (clamping the root to the keyboard top and
> revealing the focused field via `scrollRectToVisible`), ArkUI uses `KeyboardAvoidMode.RESIZE`
> plus the host's `onAreaChange` → `resized()` NAPI. The
> soft keyboard is raised/dismissed through the focus system ([docs/focus.md](docs/focus.md)). The
> `env::safe_area()` / `env::keyboard_insets()` *signals* and `.ignore_safe_area(edges)` are
> **not implemented** — no app has needed to read the values directly yet. The policy below
> remains the design of record for when one does.

Android 15 (target-sdk 35, which `Day.toml` defaults to) makes edge-to-edge mandatory, and iOS
adjusts scroll insets behind frameworks' backs — so inset policy is v1, not polish:

- The **window root applies safe-area insets as padding by default**; a root-level `scroll`
  instead converts them to native content insets so content underflows the bars;
  `.ignore_safe_area(edges)` opts out per subtree. `env::safe_area(): Signal<Insets>` exposes the
  raw values.
- Backends **neutralize native auto-adjustment** so Day's layout is the only inset authority
  (`contentInsetAdjustmentBehavior = .never` on iOS; `setDecorFitsSystemWindows(false)` + a
  `ViewCompat` inset listener on Android).
- `env::keyboard_insets(): Signal<Insets>` (from `keyboardLayoutGuide`/willShow-notifications and
  `WindowInsetsCompat.ime()`; zero on desktop). `scroll` applies it as bottom inset and reveals
  the focused field via `scroll_to`. Scoped into M5; a manual keyboard check is an M5 acceptance
  item (dayscript cannot see the native keyboard — [§14.2](#142-the-embedded-engine)).

### §7.8 RTL and BiDi

> [!NOTE]
> **Status: shipped**, with one delta: the `ar-XB` RTL *pseudolocale* was not built — the
> showcase ships a real Arabic locale instead, and the walkthrough + an `rtl-check` dayscript
> run against it (`en-XA` expansion pseudolocalization did ship, [§12.2](#122-api)). `layout_direction()` /
> `set_layout_direction` live in day-core; backends set per-widget native direction at realize.

Day owns absolute placement, so **no native mirroring applies automatically** — RTL is the
engine's job:

1. `env::layout_direction(): Signal<LayoutDirection>` derives from the active locale, overridable
   per subtree.
2. Leading/Trailing alignment and logical insets resolve at **place** time.
3. Mirroring is a single x-flip applied by `PlaceCx` within the parent's bounds when RTL —
   **`Layout` impls stay direction-naive** (they always compute in LTR logical space); `MeasureCx`/
   `PlaceCx` carry the direction so direction-aware customs remain possible.
4. Backends set per-view native direction at realize (`semanticContentAttribute` /
   `setLayoutDirection` / `gtk_widget_set_direction` / `Qt::RightToLeft` / `dir=rtl`) so native
   text alignment, cursors, and a11y agree with Day's mirroring.
5. An **`ar-XB`** RTL pseudolocale ships beside `en-XA`, with one RTL screenshot CI leg post-M6.
   `day lint` flags physical left/right styling when Day.toml declares an RTL locale.

### §7.9 Pixel snapping and density

- Backends convert rects to device pixels by **rounding edges** (`round(x·s)`, `round((x+w)·s)`)
  so adjacent frames tile without hairline gaps on fractional densities (2.625, 1.25, …).
- Measure results are ceiled to the device grid, then converted back to points.
- The `set_frame` diff uses a half-device-pixel epsilon.
- Density is part of the measure-cache epoch: a monitor change / density configuration change
  bumps the epoch and marks the tree `needs_measure` (Android delivery via [§9](#9-the-eight-toolkits-and-the-extra-combinations)'s configuration
  plumbing; frames are re-multiplied on the new scale).

### §7.10 Baseline alignment

> [!NOTE]
> Shipped 2026-08. Normative detail: **[docs/baseline.md](docs/baseline.md)**.

Rows align text on its **baseline**, not on the middle of its box. The two agree only when both
children put their text at the same height inside their own boxes, which real controls do not: a
bordered field insets its text, an `NSDatePicker` is taller than the text it shows, and a
Title-size number has a taller ascent than the Caption beside it.

- One new duty, `Toolkit::first_baseline(handle, kind, size) -> Option<f64>` ([§8.1](#81-the-toolkit-trait)), reporting
  the distance from the top of the widget's frame to its first text baseline. Defaulted to
  `None`, which means "no baseline" and falls back to box alignment — so a backend that never
  implements it renders exactly as it did before.
- A **measurement**, not a layout mode. Day places every frame itself ([§7.1](#71-day-owns-layout)) and never hands a
  row to a native baseline-aligning container, so what it needs is where the text sits inside the
  box. That is also why nearly every toolkit can answer: AppKit, GTK and Android publish a
  baseline directly; UIKit, Qt, XAML, ArkUI and the DOM derive one from the widget's font.
- `Layout::baseline` lets a container answer for its content, and every single-child wrapper
  forwards its child's. Without that, `.width(90)` on a label would silently remove it from the
  alignment — decorators are invisible at the call site.
- `labeled()` rows are baseline-aligned by default; `row(..).align(VAlign::FirstBaseline)` is the
  explicit opt-in. `Cap::BaselineAlignment` reports where it is real.
- Deliberately not done: baseline alignment between grid cells ([docs/grid.md](docs/grid.md)), and last-baseline
  alignment — `labeled` uses the FIRST baseline, matching AppKit and CSS.

---

## §8 The Toolkit specification (`day-spec`)

### §8.1 The `Toolkit` trait

> [!NOTE]
> **Status: shipped and grown, exactly as the evolution policy intended.** The original v1
> surface froze, and every later subsystem arrived as a defaulted duty. The listing below is
> the **current** surface (crates/day-spec/src/lib.rs is normative — read the trait there for
> exact signatures and doc comments).

Evolution of pane's `Backend` (proven across six toolkits), extended for Day's pillars:

```rust
pub trait Toolkit: Sized + 'static {
    type Handle: Clone + 'static;

    // capabilities — feature detection for pieces (§10; Cap: ListRecycling, Lottie,
    // NativeSymbols, Snapshot, NavSplit, NavRepresent, NavContentList, NavHeader, Appearance,
    // Dialogs, FileDialogs, Animation, Cover, TextEditable, TextSelectable, TextSpellCheck)
    fn capability(&self, cap: Cap) -> Support { Support::Unsupported }

    // node lifecycle — typed props in, sparse typed patches on update
    fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Self::Handle;
    fn update(&mut self, h, kind, patch: &dyn Any, anim: Option<&AnimSpec>);
    fn release(&mut self, h: Self::Handle);   // turn-boundary release queue; Qt defers further

    // tree
    fn insert(&mut self, parent, child, index);
    fn remove(&mut self, parent, child);
    fn move_child(&mut self, parent, child, to);

    // geometry (§7)
    fn measure(&mut self, h, kind: PieceKind, p: Proposal) -> Size;
    fn first_baseline(&mut self, h, kind: PieceKind, size: Size) -> Option<f64> { None } // §7.10
    fn set_frame(&mut self, h, frame: Rect, anim: Option<&AnimSpec>);

    // scroll (§7.6)
    fn set_scroll_content(&mut self, h, content: Size) {}
    fn scroll_to(&mut self, h, target: Rect, animated: bool) {}
    fn scroll_offset(&mut self, h) -> Point { … }

    // events: one enqueue-only trampoline, node-id keyed (contract below)
    fn set_event_sink(&mut self, sink: EventSink);

    // gestures + focus (docs/shapes.md, docs/focus.md)
    fn enable_gesture(&mut self, h, node: NodeId, kind: GestureKind) {}
    fn focus(&mut self, h, node: NodeId, focused: bool) {}
    fn set_focusable(&mut self, h, node: NodeId, focusable: bool) {}  // Decorate::focusable —
                                   // the canvas contract for composed containers (2026-08)

    // native recycling lists (§10, docs/list.md)
    fn attach_list(&mut self, host, source: ListSource) {}

    // native hierarchical trees (docs/tree.md): the same row-pull seam, token-addressed
    fn attach_tree(&mut self, host, source: TreeSource) {}

    // routes (docs/navigation.md): the current route, mirrored to a backend with a native
    // notion of location (web-dom: the URL hash); the reverse direction arrives as
    // Event::RouteRequested
    fn set_route(&mut self, route: &str) {}

    // undo bridge (2026-08, docs/persistence.md): mirror the app's one undo stack into the
    // platform's own undo object where one exists (Cap::UndoBridge — NSUndoManager fronts on
    // appkit/uikit, so the stock Edit menu retitles/enables itself and ⌘Z / the three-finger
    // gestures land); the user's invocation returns as Event::Undo { redo }. Everywhere else
    // the app's own affordances call the stack and this duty stays a no-op.
    fn set_undo_state(&mut self, state: &UndoState) {}

    // edit bridge (2026-08, docs/menus.md): mirror what the app's Cut/Copy/Paste handlers
    // can do (Cap::EditBridge — the responder chain's cut:/copy:/paste: on appkit/uikit, the
    // browser's clipboard events on web-dom, standing menu-item dispatch elsewhere); the
    // invocation returns as Event::Edit(EditOp). Transport is the system clipboard
    // (day-part-clipboard), so validation greys Paste until it holds text.
    fn set_edit_state(&mut self, state: &EditState) {}

    // ambient modifiers + non-text keys (2026-08, docs/menus.md): `modifiers()` answers the
    // keys held right now (shift-click multi-select; pull-based — NSEvent.modifierFlags, the
    // web shim's tracked mask, Qt's queryKeyboardModifiers; a backend with no live query keeps
    // the all-false default, which is right for touch and wrong for a desktop toolkit — see
    // the `modifiers` row of docs/duty-matrix.md for who answers), and `Event::Key`
    // (the dormant variant, now live) carries the arrows — plus Delete/Backspace where no menu
    // bar owns them — to the FOCUSED node's
    // `Decorate::on_key`. Keys follow focus, so there is no window-level route and nothing runs
    // ahead of the platform's dispatch: appkit's canvas answers `acceptsFirstResponder` and
    // reports from its own `keyDown:`, web-dom's carries a tabindex and its own keydown, and a
    // key nobody claimed (`day_spec::keys::handled`) keeps walking the chain. This shipped as a
    // global NSEvent monitor first, which took the arrows away from every focused list and
    // sidebar in the process. `EditOp::SelectAll` joined the edit bridge with the same
    // responder-first routing as the clipboard trio.
    fn modifiers(&mut self) -> Modifiers { Modifiers::default() }

    // menus (docs/menus.md)
    fn set_app_menu(&mut self, items: &[MenuItem]) {}
    fn set_context_menu(&mut self, h, node: NodeId, items: &[MenuItem]) {}
    // Summon-time context menu (docs/menus.md "Dynamic context menus"): the provider
    // is called when the click lands, with the local point, and its result is shown —
    // natively (appkit/gtk/uikit/qt), or via `Event::ContextMenu` + the composed
    // presentation on a backend with no native menu (web-dom).
    fn set_context_menu_fn(&mut self, h, node: NodeId, f: ContextMenuFn) {}

    // window toolbars (docs/toolbars.md): `h` is the window root's handle, so the backend
    // walks from it to the window. A full install replaces the bar; `update_toolbar` is the
    // targeted path a bound signal writes through, so syncing a search field does not rebuild
    // (and refocus) the bar. Defaulted no-ops — a toolkit with no toolbar shows nothing and
    // answers Cap::Toolbar = Unsupported rather than drawing an imitation.
    fn set_toolbar(&mut self, h, items: &[ToolbarItem]) {}
    fn update_toolbar(&mut self, h, patch: &ToolbarPatch) {}
    // Show/hide the window's `selector(Sidebar)` pane — what a `ToolbarItemKind::SidebarToggle`
    // item drives. A DUTY rather than a dispatch id, because that item carries no app closure:
    // the native button and dayscript's `toolbar:` step both land here, so a walkthrough
    // exercises the path a click takes. `false` = no sidebar in this window, and the item
    // renders disabled. Defaulted, so a backend without one needs no code.
    fn toggle_sidebar(&mut self) -> bool { false }

    // presentation (docs/dialogs.md, docs/files.md): alerts/confirm/prompt/sheets/pickers
    fn present(&mut self, req: u64, spec: &present::PresentSpec) {}
    fn dismiss(&mut self, req: u64) {}
    fn open_url(&mut self, url: &str) {}   // system browser/handler for the `link` piece (§5.3)
    fn defer_system_gestures(&mut self, edges: Edges) {}   // the shield union (docs/cover.md)
    fn dark_mode(&mut self) -> bool {}     // current appearance, for app-painted opaque surfaces
    fn set_appearance(&mut self, dark: Option<bool>) {}  // runtime light/dark/system override (Cap::Appearance)

    // pillars
    fn set_a11y(&mut self, h, a11y: &A11yProps) {}                    // §13
    fn read_a11y(&self, h) -> A11ySnapshot { … }                      // the a11y_audit's native read
    fn replay(&mut self, h, ops: &[DrawOp], size: Size) {}            // canvas §11
    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> { … }    // dayscript §14, docs/window-image.md
    fn snapshot_window_chrome(&mut self) -> Result<Vec<u8>, String> { … } // + titlebar/status bar
    fn ui_idle(&mut self) -> bool { true }                            // transitions settled? (screenshots)

    // app lifecycle (docs/lifecycle.md)
    fn supports_lifecycle(&self, phase: Lifecycle) -> bool { … }
    fn on_suspend(&mut self) {}  fn on_resume(&mut self) {}  fn on_memory_warning(&mut self) {}

    // adoption of foreign native handles (external piece renderers, §15)
    fn adopt(&mut self, raw: RawHandle) -> Self::Handle { … }
}

pub trait Platform: Toolkit {
    const TARGET: &'static str;    // "macos-appkit" — a process constant
    const TOOLKIT: &'static str;   // "appkit"
    fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Self::Handle, Size)>);
    fn post(f: Box<dyn FnOnce() + Send>);          // the one cross-thread door (§3.3)
    fn post_delayed(ms: u32, f: Box<dyn FnOnce() + Send>) { … } // timers; default = thread +
                                                   // sleep + post; single-threaded hosts (web)
                                                   // override with a native timer. Backs
                                                   // `day::sleep(ms)` (docs/async.md).
    fn locale_hints(&self) -> Vec<String> { … }    // ORDERED OS preference list (fluent-langneg)
}
```

One deliberate simplification against the original design: the `AppCx`/`create_window`
multi-window seam was **not** built for v1 — `day::launch(root)` + `WindowOptions` stayed the
whole windowing surface, and dialogs/menus arrived as their own duties instead of flowing
through window creation.

> [!NOTE]
> Revised 2026-08: **secondary windows shipped** ([docs/windows.md](docs/windows.md)) as the evolution policy
> below prescribes — five defaulted `Toolkit` duties (`open_window` answering
> `WindowOpenReply::{Open, Pending, Unsupported}`, `close_window`, `focus_window`,
> `set_window_title`, `snapshot_window_of`), `Cap::MultiWindow`, and
> `Event::{WindowClosed, WindowFocused}` on the window's root node. The tree stayed
> SINGULAR: a second window's content container is adopted as an additional boundary root
> (multi-ROOT, not the once-sketched tree-per-window), so `with_tree`, `find_by_id`, and
> dayscript work across windows unchanged. `day::open_window(key, …)` is the app API
> (open-or-focus singletons), `day::register_preferences*`/`open_preferences` the settings
> paradigm behind the auto Settings…/⌘, menu item, `register_new_window` the File ▸ New
> Window / macOS tab-bar "+" builder. Native on the desktop backends AND the
> mobile ones — iPad UIScenes (day-uikit runs the scene lifecycle), Android document-style
> activity instances, OHOS multiton ability instances; iPhone, the Preferences kind on
> mobile, and web present the content as a fullscreen cover in the primary window.

**Evolution policy (held in practice):** every duty added after the freeze ships with a default
no-op/`Unsupported` body — gestures, focus, lists, menus, presentation, lifecycle, `read_a11y`,
and `ui_idle` all arrived that way, and no backend broke.

`Props` is `&dyn Any` downcast to the piece's typed descriptor (e.g. `LabelProps`) — **zero
serialization between Rust and Rust-implemented backends**; patches are sparse (only changed
fields). The native boundaries that must encode (JNI, the C++ shims) use small packed frames
and primitives, never text formats.

### §8.2 The open renderer registry

> [!IMPORTANT]
> **Status: shipped as the linkme layer.** Each backend exposes a `RENDERERS` distributed
> slice (`#[distributed_slice(day_appkit::RENDERERS)] …`) that external piece crates populate;
> the `day-spec` `Registry` folds them in at toolkit init. The layered hardening below — the
> generated Rust registrant and the required-kinds startup completeness check — was **not
> built**; release builds (including the packed iOS app) had not hit the dead-strip problem —
> until the windows-gnu dev combos did (2026-07): MinGW ld drops a piece's registration static
> when its codegen unit exports nothing else referenced, so on `windows-qt`/`windows-gtk` several
> external pieces render placeholder leaves (which crates survive is link-order luck; MSVC keeps
> `#[used]` statics via `/INCLUDE`, so `windows-xaml` is unaffected). The showcase walkthrough's
> `assert_no_placeholders` ledger records the affected kinds per target; the layered hardening
> below is the designed fix and is now motivated by a real failure, not a hypothetical.
>
> **web-dom is the exception (2026-07):** `linkme`'s `#[distributed_slice]` refuses to compile for
> `wasm32-unknown-unknown` ("distributed_slice is not implemented for this platform", 0.3.36 and
> 0.3.37), so `day-dom` carries a runtime registry instead — a `thread_local` `Registry<Dom>` plus
> `day_dom::register_renderer(fn() -> Renderer<Dom>)`, idempotent per kind and consulted at all
> three dispatch points (realize, update, measure) before the placeholder leaf. Pieces call it from
> their own constructor, which necessarily runs before the node they return is realized. That is
> layer 1 above (the idempotent `register()`) arriving on the one backend that had no choice, and it
> needs no link-time trick because a wasm module has a single deterministic init path. First
> consumer: `day-piece-media` ([docs/media.md](docs/media.md)).

> [!NOTE]
> **Built-in kinds are an enum, extension kinds stay strings (2026-08).** `PieceKind` is still the
> interned `&'static str` the registry keys on, but the kinds Day itself defines are now generated —
> together with their string keys and the `kinds::*` constants — from one `builtin_kinds!` table in
> day-spec, which emits a plain (deliberately NOT `#[non_exhaustive]`) `Builtin` enum. Every backend
> dispatches `match Builtin::from_key(kind)`, so the built-in arms are checked for exhaustiveness and
> the `None` arm is exactly the extension path described above. Adding a built-in kind is therefore a
> compile error in all eight backends until each one decides how to realize it, instead of silently
> degrading to a placeholder leaf at runtime. `Builtin::ListCell` shares the `None` arm: a recycled
> row's anchor is ADOPTED from the native list (`Toolkit::adopt`), never realized. The enum is plain
> because the guarantee is the point — a new built-in kind genuinely is a new duty for every backend,
> so it should break the build rather than pass semver-quietly.

Registration was designed **layered** so that `linkme` is a convenience, not a correctness mechanism (the
bare `use crate as _;` anchor is a link-time gamble under iOS `-dead_strip` + LTO, and a
startup-time completeness check is impossible if the registry itself is the only source of truth):

1. Every piece API crate exposes an idempotent `pub fn register()` and contributes a **required
   kinds** manifest entry, making the startup check — required kinds minus available renderers —
   implementable in **all** profiles: debug panics listing the missing (kind, toolkit) pairs;
   release logs loudly and shows an error surface. Never a mid-session surprise.
2. For app targets built by `day build`, tier-1 registration calls are folded into the
   **generated Rust registrant** (the same generated-registrant pattern as dayffi, [§15.3](#153-dayffi-the-c-abi-superseded--never-built)) — fully
   deterministic, dead-strip-proof.
3. The `linkme` distributed slice remains for zero-setup unit tests and pure-cargo development
   (pane's proven mechanism, kept as the ergonomic layer).

CI includes a release+LTO ios-uikit build of showcase + day-piece-searchfield that asserts via
dayscript that the externally-registered piece actually rendered ([§20](#20-continuous-integration)).

### §8.3 Events

> [!NOTE]
> The numeric event-kind wire table for the trampoline backends (Android JNI, ArkUI C-FFI)
> lives in `day_spec::bridge::BridgeKind`; the Java/C++ constants mirror it and parity tests
> hold them together (2026-07 — after a kind collision silently swallowed the resize rail).
> Additions since: `BridgeKind::SafeArea = 19` (2026-07) feeds `day_core::set_safe_area` from an
> edge-to-edge backend — px insets in `text`, no `Event` emitted — and `NavPatch::Pushed` gained
> an `immersive: bool` (the selector item's `.immersive()` flag; day-android flips the pushed
> page between the floating-scrim and opaque bars, other backends ignore it). docs/layout.md and
> [docs/navigation.md](docs/navigation.md) are normative. The built-in facts that rode `Event::Custom` tags became
> typed variants (2026-08): `ListReorder`/`ListDelete` (the list piece's deferred commit echoes,
> [docs/list.md](docs/list.md)), `Event::Undo { redo }` with `BridgeKind::UndoInvoked = 28` (the undo bridge's
> up direction, 2026-08 — emitted only by native fronts), `Event::Edit(EditOp)` (the edit
> bridge's up direction, 2026-08 — the platform's Cut/Copy/Paste route, [docs/menus.md](docs/menus.md)) and `CoverHidden` ([docs/cover.md](docs/cover.md); `BridgeKind::CoverHidden = 26` on the
> trampoline wire), while warm deep links now arrive as the existing `RouteRequested` — leaving
> `Custom` purely piece-defined. `LinkActivated(String)` joined them for styled text runs (2026-08,
[docs/text-runs.md](docs/text-runs.md)): `Cap::TextRuns` is Native on all eight backends, `Cap::TextLinks` on six —
AppKit needs an NSTextField→NSTextView swap it does not do yet, and ArkUI is unwired — so a
`.link()` run always draws, and taps report on the six.

```rust
pub enum Event {
    Pressed,                                  // button
    TextChanged(String), Submitted,
    ToggleChanged(bool),
    ValueChanged(f64),                        // slider et al.
    SelectionChanged(i64),                    // pickers, tabs, nav lists
    SelectionSet(Vec<i64>),                   // multi-select lists (docs/list.md)
    FocusChanged(bool),                       // docs/focus.md
    Tap(Point), LongPress(Point),
    ContextMenu { local, window },            // a reported summon — the composed menu
                                              // presents at `window` (docs/menus.md)
    Drag { phase, location, translation },    // docs/shapes.md gestures
    Pinch { phase, scale, location },         // trackpad/touch zoom (docs/canvas.md)
    Pan { phase, delta, location },           // two-finger scroll/pan (docs/canvas.md)
    ScrollChanged(Point),                     // §7.6
    FrameChanged(Size),                       // canvas re-record; nav pane size reports
    NavBack { already_popped: bool },         // native back (docs/navigation.md)
    Key(KeyEvent), Pointer(PointerEvent),
    WindowResized(Size),
    PresentResult { req, result },            // modal answers (docs/dialogs.md)
    MenuAction(u64),                          // docs/menus.md
    Lifecycle(Lifecycle),                     // docs/lifecycle.md
    ListReorder { from: usize, to: usize },   // committed native row drag (docs/list.md)
    ListDelete(usize),                        // committed swipe-delete (docs/list.md)
    ListSwipe { index, edge, action },        // activated swipe action (docs/list.md)
    TreeExpanded { token, expanded },         // native disclosure (docs/tree.md)
    TreeMove { token, parent, index },        // committed native tree drag (docs/tree.md)
    TreeSelection(Vec<u64>),                  // tree selection, full token set (docs/tree.md)
    CoverHidden,                              // cover hide transition finished (docs/cover.md)
    LinkActivated(String),                    // a styled run's link (docs/text-runs.md)
    InspectorChanged(bool),                   // native pane show/hide (docs/inspector.md)
    Custom { tag: &'static str, num: f64, text: String },  // open piece-defined channel (§8.2)
}
```

(`Custom` shipped with a primitive `num`/`text` payload rather than the designed
`DayValue` tree — [§15](#15-extensibility-pieces-parts-and-tweaks) explains; `tag` is empty for events crossing a native boundary.)

The single sink keeps the backend ignorant of closures/lifetimes (day-core owns the `NodeId →
handlers` table) — this is the shape that made pane's six backends small. The sink contract is
enqueue-only ([§8.1](#81-the-toolkit-trait)); handlers run under their registration scope ([§4.3](#43-scopes-and-disposal)).

### §8.4 Animation (reserved hooks — still unimplemented)

> [!NOTE]
> **Status: partly shipped (2026-07; XAML 2026-08).** `with_animation(spec, || …)` exists
> (day-core `anim.rs`) and threads `AnimSpec` through
> `set_frame`/`set_opacity`/`set_transform`/`update`; the backends execute opacity and
> transform changes natively (the showcase Animation page drives every channel at once).
> An animated background-color `update` interpolates on UIKit and XAML: `UIView.backgroundColor`
> is a CALayer property that UIKit's own animator tweens on the render server, and XAML's
> fill is a `SolidColorBrush` Day owns, which a `ColorAnimation` tweens given
> `EnableDependentAnimation` (brush color is not GPU-composited, unlike opacity and the
> `CompositeTransform` channels). The AppKit backend paints its fill in `drawRect` (so
> dynamic system colors re-resolve per appearance), which Core Animation cannot interpolate
> — and per §0.3 Day does not tick its own animations for native widgets — so there, and on
> the remaining backends, the color applies at commit. The `.transition` enter/exit surface
> remains unimplemented.
>
> **windows-xaml (2026-08).** `set_opacity`/`set_transform` were the trait's defaulted
> no-ops until now, so scale, rotation, offset and opacity did nothing at all on Windows
> (the color still applied, which made the page look half-alive). Both are implemented as
> XAML `Storyboard`s: opacity on `UIElement.Opacity` and the transform channels on a
> `CompositeTransform` about the element's center — the same anchor AppKit's layer and Qt's
> painter transform use. Storyboards are kept per (element, property) and stopped before
> re-animating, since two live storyboards on one property fight and a stopped one snaps its
> property back; `FillBehavior::HoldEnd` keeps the settled value.

Native-widget frameworks that bolt animation on later end up breaking their backend ABI — so the
seam ships now even though MVP backends ignore it. Day commits to **backend-executed animation**:
Day passes *intent*, the platform animates (consistent with [§0.3](#03-non-goals) — Day never ticks pixel frames
for native widgets). `AnimSpec { duration, curve, spring }` parameters already sit on `set_frame`
and `update` ([§8.1](#81-the-toolkit-trait)), no-op in MVP backends. The post-MVP surface (design sketch, not v1 API):
`.transition(anim)` on `when`/`each` enter/exit, animated frame changes
(`with_animation(anim, || …)`), and a day-driven frame-clock ticker **for canvas only**.

### §8.5 Panics and crashes

> [!IMPORTANT]
> **Status: partially shipped.** The event pump runs handler dispatch under `catch_unwind`
> (day-core), which covers the main native-callback surface. The release panic hook, native
> signal handlers, and the crash-reporter hook now ship in the **optional** `day-break` crate
> ([docs/break.md](docs/break.md)) — the hook is `day_break::on_crash` (the `day` umbrella crate can't depend on an
> optional reporter), and day-core notifies it of contained panics via
> `set_contained_panic_observer`. day-core now contains panics at three backend-agnostic
> trampoline boundaries — the event pump, posted main-thread tasks, and lifecycle dispatch (a
> panicking lifecycle handler, e.g. an `eprintln!` on a stderr pipe the parent has closed during
> teardown, no longer aborts the process); framework diagnostics on those paths use a
> non-panicking stderr writer. **Backend trampolines are guarded (2026-08):** day-spec ships
> `ffi_guard::contain` (a `catch_unwind` wrapper with a recovery hook day-core points at
> `day_reactive::recover_from_panic` at boot), and every backend wraps its `extern "C"` / ObjC
> method / JNI entry points in it — event callbacks, list-source trampolines, posted-closure
> deliveries, window/lifecycle callbacks. Two gaps remain: ObjC **block**-based callbacks on
> day-uikit (UIAction handlers, some completion blocks) still dispatch unguarded, and the in-app
> debug error surface is unimplemented — those remain the design of record. (On wasm, panics trap
> the instance rather than unwind, so day-dom's guards are defense-in-depth.)

> [!NOTE]
> **The reactive runtime is unwind-safe (2026-08).** Containment only pays off if what survives is
> coherent, and day-reactive used to restore its state *after* the user callback returned — a line
> an unwind skips. Four sites now restore through an RAII guard instead, so a contained panic can no
> longer strand runtime state: `Scope::enter` (a stranded `current_scope` left every later
> `Signal::new` parented to a disposed scope, so its first read failed as "read of disposed
> Signal" — a wrong diagnosis of a corrupt runtime), `untrack` (a stranded `None` observer silently
> disabled dependency tracking process-wide), `batch` (a stranded depth stopped writes from ever
> scheduling a drain), and the signal/memo read path. `recover_from_panic` additionally re-roots
> `current_scope`, since a panic raised between scopes never reaches a guard. A second pass
> (2026-08) extended the same RAII rule to the reaction and memo compute paths (an effect's own
> panic pops its observer frame during unwind) and to `flush_now`'s batch-depth restore, made the
> batch guard's decrement saturating, and fixed the release re-run cap to re-arm the capped
> effect on its next source write instead of silently disabling it for the process lifetime.
>
> A node's value is now held in a shared cell (`Rc<RefCell<…>>`) rather than being moved out for the
> duration of a `with`/`try_with` closure. That removes the last way a panic could destroy state — an
> unwind used to lose the value permanently, killing that signal for the rest of the process — and it
> makes **re-entrant reads work**: reading a signal inside its own closure returns the value instead
> of finding the hole. Writing a signal while a read of it is in flight was previously silent data
> loss (the read's restore clobbered the write) and is now a panic that names the cause. Measured at
> ~4.6 ns per `with()` read, so the shared cell is not a hot-path regression.

A panic unwinding out of an `extern "C"` / ObjC / JNI frame aborts the process with no useful
report, so this policy was specified up front:

- Every trampoline entry (events, timers, `on_main` deliveries, dayffi callbacks) wraps user
  closures in `catch_unwind`. day-core closures carry the `UnwindSafe` bounds from M0 —
  retrofitting bounds later is a breaking change.
- Debug: a caught panic renders a Day error surface (message + location) and keeps the app alive
  where sane (the offending subtree is quarantined).
- Release: a panic hook writes message + backtrace to the platform log (os_log / logcat /
  journald / Windows Event Log) and then aborts. Per-platform symbolication is documented, and a
  crash-reporter hook (`day::on_crash(fn)`) exists for integrating external reporters.

---

## §9 The eight toolkits (and the extra combinations)

> [!IMPORTANT]
> **Status: all eight shipped** (seven native + mock), and a ninth — **`day-dom`**, the
> `web-dom` backend — landed 2026-07 as experimental ([docs/web.md](docs/web.md); it grew out of the
> `web-html` sketch recorded below). One material change from the design: the Windows backend
> hosts **system XAML** (`Windows.UI.Xaml` controls in a `DesktopWindowXamlSource` island
> inside a Win32 window), not WinUI 3 / Windows App SDK — no runtime bootstrap, no
> framework-package dependency, and the `windows-xaml` target name stayed.

Shared mechanics came from pane's working code; every FFI choice below now runs in this repo:

| backend | FFI mechanism | container | status |
|---|---|---|---|
| `day-appkit` | `objc2` (`objc2-app-kit`) | `NSView` (flipped `DayFlipped`) | shipped; CI walkthrough + pack |
| `day-uikit` | `objc2` (`objc2-ui-kit`) | `UIView` | shipped; Simulator walkthrough + pack in CI |
| `day-gtk` | `gtk4-rs` | `gtk4::Fixed` | shipped (Linux + macOS host); headless CI walkthrough |
| `day-qt` | `cc`-built C++ shim (`day-qt-sys`) | bare `QWidget` | shipped (Linux + macOS host); headless CI walkthrough |
| `day-android` | `jni` + a Java shim (`DayBridge`/`DayFixed`/`DayActivity`) | absolute-layout `ViewGroup` (`DayFixed`) | shipped; emulator walkthrough + pack in CI |
| `day-xaml` | C++/WinRT shim (`day-xaml-sys`, cppwinrt-staged headers) | XAML `Canvas` in a `DesktopWindowXamlSource` island | shipped; CI-verified build/walkthrough/pack |
| `day-arkui` | ArkUI **NDK C API** via a C++ shim (`day-arkui-sys`; `aarch64-unknown-linux-ohos`) | ArkUI stack node | shipped; cross-compile in CI, emulator via `day ohos` ([docs/harmonyos.md](docs/harmonyos.md)) |
| `day-dom` | plain `extern "C"` imports to an ES-module JS shim (`crates/day-cli/resources/web/shim.js`, embedded in the CLI; `wasm32-unknown-unknown`, no wasm-bindgen) | `<div id="day-root">` | experimental ([docs/web.md](docs/web.md)); `day build\|launch -p web-dom` |
| `day-mock` | — | — | shipped; the headless test double ([§3.2](#32-crates)) |

Per-toolkit notes beyond pane's baseline (the day-new duties):

- **a11y ([§13](#13-accessibility)):** UIKit/AppKit: `NSAccessibility`/`UIAccessibility` protocols (mostly free on
  native controls; Day sets labels/identifiers/traits). Android: `contentDescription`,
  `AccessibilityNodeInfo`, `importantForAccessibility`. GTK 4: `GtkAccessible` roles/properties
  (AT-SPI on Linux; off-Linux, GTK 4.18's **AccessKit backend** is the forward path but default
  and Homebrew builds don't enable it — `macos-gtk` currently exposes **no a11y tree at all**,
  which is exactly why it is a *secondary* combination; `day doctor` probes the installed GTK for
  AccessKit and the build/env recipe is documented, not hidden). Qt: `QAccessible` (bridges to
  NSAccessibility/UIA/AT-SPI on all three OSes — Qt is the strongest cross-OS a11y story of the
  portable toolkits). XAML: UIA, mostly free. Web: ARIA attributes.
- **canvas ([§11](#11-canvas)):** CGContext in `drawRect:`/`draw(_:)`; `android.graphics.Canvas` in `onDraw`
  (display list crosses JNI once per redraw as a packed buffer); `GtkDrawingArea` + cairo;
  `QPainter` in `paintEvent`; Win2D or Direct2D via the shim; DOM `<canvas>` 2D.
- **snapshot ([§14](#14-scripting-dayscript)):** `CALayer`/`NSView` bitmap render; `UIGraphicsImageRenderer`; `PixelCopy` /
  `View.draw(Canvas)`; `gtk_widget_snapshot` → cairo surface; `QWidget::grab`;
  `RenderTargetBitmap`; `<canvas>` composite (web: best-effort).
- **list hosts ([§10](#10-native-list-integration)):** `UICollectionView` / `RecyclerView` / `NSTableView` / `GtkListView` /
  `ItemsRepeater` / virtualized DOM. **Qt is the honest exception**: `QListView` recycles
  *delegate paintings*, not live `QWidget` rows (`setIndexWidget` is unvirtualized) — Qt's list
  host is day-side emulated recycling behind the same RowHost protocol, reported as
  `Support::Emulated` (DP-19).

Two lifecycle realities that shape backends beyond pane's baseline:

- **Android configuration changes.** By default, rotation / dark mode / locale / density changes
  **recreate the Activity** — fatal to a build-once tree holding `jobject` handles. Day takes
  Flutter's stance: the scaffold manifest declares
  `android:configChanges="orientation|screenSize|uiMode|locale|density|fontScale"`, and the
  backend routes `onConfigurationChanged` into Day's signals and re-applies — dark mode natively
  (backends resolve dynamic colors, [§6.3](#63-semantic-theme-tokens)), locale → the locale signal ([§12](#12-localization-fluent)), density →
  measure-cache epoch bump + frame re-multiplication ([§7.9](#79-pixel-snapping-and-density)). The
  suspend/resume/memory hooks ([§8.1](#81-the-toolkit-trait)) map to the Activity callbacks. Process-death state
  restoration (`onSaveInstanceState`) is **DP-25** — v1 documents cold restart.
- **Windows runtime choice.** The designed WinUI 3 / Windows App SDK backend (with its
  `MddBootstrapInitialize2` bootstrap and runtime-installer story) was **replaced by system
  XAML Islands**: `Windows.UI.Xaml` ships in Windows itself, so an unpackaged Day app starts
  with no runtime dependency at all, and `day pack` produces `.msix` plus an NSIS installer
  with nothing to chain. The cost is system-XAML's older control set and per-element theming
  (the shim forces `DAY_THEME` per-element on the root). Moving to WinUI 3 later is a backend
  swap behind the same day-spec surface.

On mobile, the [§8.1](#81-the-toolkit-trait) "window" maps to the scene / activity content view; multi-window remains
future, additive work ([§8.1](#81-the-toolkit-trait)'s status note).

**Extra combinations** (`macos-gtk`, `macos-qt`, `windows-qt`, `windows-gtk`) need no extra code in
the backend crates — GTK/Qt are portable; the *target* differs only in build/packaging ([§16](#16-the-day-cli), [§17](#17-the-conventional-day-project-and-daytoml):
where the toolkit libraries come from and whether `day pack` can bundle them; bundling GTK/Qt into
a redistributable macOS/Windows app is real work and is explicitly **post-MVP**, DP-7). The
`Day.toml` `targets:` list and `day doctor` gate which combinations a project claims.

**Support tiers.** Every target carries a tier saying how much testing and maintenance it gets:
**Tier 1 — Supported** (`ios-uikit`, `android-mdc`, `macos-appkit`), **Tier 2 — Demi-supported**
(`linux-gtk`, `linux-qt`, `windows-xaml`), **Tier 3 — Experimental** (`harmony-arkui`,
`web-dom`), **Tier 4 — Development** (`macos-gtk`, `macos-qt`, `windows-gtk`, `windows-qt`). The
tier is independent of backend completeness — a Tier 4 target runs the same backend crate as its
Tier 2 sibling on another OS, and differs in the attention it gets, not in what it renders. The
definitions are normative on the website's Platform support page, "Support tiers"
(`website/src/content/docs/platforms.md`); the per-target assignment lives once in
`website/src/lib/platforms.mjs`, and `website/plugins/tier-badge.mjs` renders it as a badge
wherever the docs name a target's support level. A target moves up a tier when contributors and
maintainers commit to keeping it there (CONTRIBUTING.md).

**web-html sketch → `web-dom`, shipped experimental (2026-07):** the sketch read: wasm32 binary;
pieces map to semantic elements (`<button>`, `<input>`, `<label>`); Day layout emits
`position:absolute` placements; text measurement via a hidden measurement element or
`canvas.measureText` (cached); events via `wasm-bindgen` closures; scripting transport is a
`WebSocket` ([§14.5](#145-transport-and-rendezvous)). The open question — whether absolute placement forfeits too much of
the browser — was recorded as DP-8 with a proposed hybrid (Day layout, but `scroll` maps to
overflow scrolling). The shipped `day-dom` follows the sketch with two changes: no wasm-bindgen
(a hand-written ES-module shim owns the DOM, the day-arkui trampoline shape with JS in place of
C), and DP-8 resolved to exactly the proposed hybrid — absolute placement inside
`overflow:auto` scroll containers, with nav/tab panes CSS-framed and reporting size back via
ResizeObserver. The WebSocket dayscript transport shipped as sketched ([§14.5](#145-transport-and-rendezvous)):
the page speaks WebSocket to the dev server, which bridges to the runner's TCP protocol — CI
drives the full walkthrough this way, including the HTTP demo against the dev server's
`/day-http-ok` echo (day-part-http's browser arm rides the shim, [docs/http.md](docs/http.md)). [docs/web.md](docs/web.md)
is the reference. Two hardenings for dependency graphs (2026-08): `day build` compiles every
web app with `--cfg getrandom_backend="custom"` and day-dom answers getrandom v0.3's
custom-backend hook from the shim's `crypto.getRandomValues` import (entropy for uuid/rand
without wasm-bindgen), and the shim + day-sql worker satisfy import modules besides `env`
(a dependency's UNREACHED wasm-bindgen placeholders — chrono's default `wasmbind`, dragged in
by feed parsers) with throw-on-call stubs, so instantiation survives and only an actual call
into the missing runtime fails, diagnosably. One known limit: a DEBUG persistence app can
exhaust WebKit's machine stack opening its store (unoptimized SQLite frames; Chromium copes,
release fits everywhere) — scripted WebKit runs build `--profile release`, as CI always has.

**harmony-arkui — shipped.** The "speculative sketch" bet paid off: ArkUI's C node API
(`ArkUI_NativeNodeAPI_1`) matched day-spec's shape and the backend is now first-class — full
walkthrough support, native drawing, focus, dialogs, rawfile resources, `.hap` packing, and
`day ohos` emulator helpers. [docs/harmonyos.md](docs/harmonyos.md) is the reference.

---

## §10 Native list integration

> [!NOTE]
> **Status: shipped** ([docs/list.md](docs/list.md) is normative). The duty landed as
> `Toolkit::attach_list(host, ListSource)` rather than the sketched `ListHost` object — the
> host pulls `len`/`bind_row` through the `ListSource`, and the mock/walkthrough tests assert
> recycled cells rebind with a slot write, not a rebuild. Qt's emulated recycling shipped as
> designed (DP-19). The `rw`/`.on_edit` two-way projections did not ship ([§5.4](#54-keyed-collections-each));
> day-model field accessors are the shipped two-way path ([docs/model.md](docs/model.md)).
> **Drag-to-reorder shipped** (2026-08): `ListProps::reorderable` + the `ListSource::reorder`
> sync seam (`can_move` guard verdict / `move_row` commit), the piece API
> (`.reorderable/.on_reorder/.reorder_guard`), `Cap::ListReorder`, and the dayscript `reorder`
> step — native mechanisms on AppKit (`.gap` drop style), UIKit (drag delegates), Android
> (ItemTouchHelper on the list, which **migrated from framework ListView to RecyclerView** in
> the same change), GTK (DragSource/DropTarget), Qt (QDrag), ArkUI (SetNodeDraggable +
> NODE_ON_DROP), the WinRT drag pipeline over XAML's still-emulated list (a real-ListView
> migration via ContainerContentChanging remains a candidate follow-up), and a pointer-tracked
> emulation on web-dom. Recycled-row ids gained the reactive `.id_of` decorator (a build-time
> keyed id goes stale when a cell rebinds). **Programmatic row scrolling** followed
> (`ListPatch::ScrollToRow` + `.scroll_to_row(Signal<Option<usize>>)`, all backends), and a
> same-set/new-order Reload animates as native row moves on AppKit ([docs/list.md](docs/list.md)).
> **Swipe actions shipped** (2026-08): app-declared reveal-as-you-swipe buttons on either
> semantic edge, offers pulled per gesture so labels track row state — the pieces API
> (`swipe_action(label).destructive/.tint/.symbol/.action` + `.swipe_leading/.swipe_trailing`), the
> `ListSource::swipe` seam (`actions_at` offer / `perform` commit, handlers held app-side and
> re-resolved at the event drain), `Event::ListSwipe`, `Cap::ListSwipeActions` (Native on
> AppKit via `tableView:rowActionsForRow:edge:` and UIKit via `UISwipeActionsConfiguration`,
> sharing the delete pipeline; affordance simply absent elsewhere), and the dayscript
> `swipe_row` step, whose `label:`/`key:` pins which button a state-dependent offer held
> ([docs/list.md](docs/list.md)). The AppKit table then moved to `NSTableViewStyle::Plain`
> (2026-08): the FullWidth style's ~6pt cell inset had been countered by a custom
> `NSTableRowView` pinning cells to the row bounds, and that pin also clobbered the cell-frame
> slide AppKit performs for row-action swipes — actions revealed behind a motionless row.
> Plain has no inset, so the custom row view is gone and the swipe slides whole rows natively.
> **Host-drawn separators** joined in the same change (`ListProps::separators`,
> `.separators(bool)`, tri-state over each platform's default): the row boundary line belongs
> to the host, where it aligns with the native selection and holds still under a swipe —
> lowered on AppKit (grid mask), UIKit (`separatorStyle`), GTK (row CSS) and web (cell
> border); the docs matrix tracks the rest.

The requirement: Day's `list` must use the platform's recycling list (`UICollectionView`,
`RecyclerView`, `NSTableView`, `GtkListView`, `QListView`) so large collections get native
virtualization, scroll physics, and platform behaviors.

### §10.1 API — the shared `ItemSlot` contract (unified with `each` — DP-16 resolved)

Because cells are **recycled**, the row builder cannot receive the item by value (a moved value
can never be swapped later — recycling would be a rebuild). The builder receives the same
**`ItemSlot<T>`** as `each` ([§5.4](#54-keyed-collections-each) — one contract, one row function serves both; migrating a
collection from `scroll(column(each(…)))` to `list` is a one-word change):

```rust
// `messages` is a day-model store (docs/model.md); `ordered_keys` its display projection.
list(messages.rows(ordered_keys), move |row: ModelSlot<Message>| {
    column((
        label(move || row.sender().read()),    // wakes only for THIS row's `sender`
        label(move || row.preview().read()),
        toggle(row.starred()),                 // two-way; follows the row across recycles (§5.3)
    ))
})
.row_height(RowHeight::Uniform(56.0))          // or ::Automatic (self-sizing, slower)
.on_select(move |it: Elem<Message>| open(it.key()))
```

Slot semantics are as specified in [§5.4](#54-keyed-collections-each): plain-data rows get `ItemSlot` (Copy handle,
tracked `get()`, equality-gated `field()` projections, the structure-from-`get()` trap and its
lint); store rows get `ModelSlot` (a day-model `Source` whose accessors bind two-way and follow
the recycle). Key uniqueness is asserted per diff in debug builds. A designed `.row_kind`
(native reuse-identifier pools) has not shipped; every list runs one pool.

### §10.2 Realization: the RowHost protocol

The backend's list host owns scrolling and recycling; Day owns row *content*:

1. Day gives the host a **data source**: `len()`, `key_at(index)`, `kind_at(index)`, and change
   notifications derived from the same keyed diff as `each`. Hosts declare their change-batch
   capabilities and Day **normalizes**: moves are lowered to remove+insert where unsupported
   (`GListModel` has no move), illegal same-index combinations are split (`UICollectionView`
   batch-update constraints), and diffs above a size threshold collapse to reload-all.
2. When the host needs a cell it calls `bind_row(cell_container_handle, key, kind)`. Day either
   **builds** the row piece into that container (first use per pool) or **rebinds** a recycled
   row: one slot write. Because hosts measure cells synchronously after binding, `bind_row` runs
   `Scope::flush_now(row_scope)` and row layout **before returning** — the sanctioned exception to
   turn batching ([§3.3](#33-threading-model-and-the-turn-state-machine)); without it, recycled cells would display stale content and `Automatic`
   mode would cache wrong heights.
3. Row layout runs Day's engine inside the cell bounds. `RowHeight::Uniform` cells are true layout
   boundaries ([§7.4](#74-incremental-relayout-and-the-measurement-cache)); `::Automatic` cells are boundaries **with notification** — when a row's
   content size changes, Day calls `host.row_size_invalidated(key)`, mapping to
   `reconfigureItems`/preferred-attributes (UICollectionView), `noteHeightOfRows` (NSTableView),
   `requestLayout` (RecyclerView), `InvalidateMeasure` (ItemsRepeater).
4. Selection, separators, swipe actions, section headers are host-native features exposed as list
   options gated on `Toolkit::capability` ([§8.1](#81-the-toolkit-trait)); Qt reports `Emulated` recycling (DP-19).

This was the single hardest backend feature, deferred past the MVP by design — and the
pre-reserved spec hooks did their job: it landed later as a defaulted duty with no breaking
change. `scroll(column(each(…)))` remains the honest choice for small collections.

### §10.5 Navigation and presentation

> [!NOTE]
> **Status: shipped** ([docs/navigation.md](docs/navigation.md), [docs/dialogs.md](docs/dialogs.md), and [docs/menus.md](docs/menus.md) are normative).
> The DP-23 "native containers" resolution held, delivered through a richer surface than the
> sketch below:
>
> - **Typed routes.** `day::routes! { enum Section { Controls => "controls", … } }` declares
>   the destinations; deep links, dayscript `navigate`, and `current_route()` all speak the
>   same keys, compile-checked.
> - **`selector(signal)`** — one signal of the active destination, presented per platform and
>   `SelectorStyle` (desktop sidebar + detail split, mobile list-push, tabs, segmented);
>   `Cap::NavSplit`/`Cap::NavHeader` let pages adapt to what the toolkit provides.
> - **Adaptive navigation** *(2026-08)* — `SelectorStyle::Automatic` is now the DEFAULT, and
>   `NavPresentation` gained `Tabs` and `Rail` beside `Split`/`Stack`. One host wears all four:
>   `build_tabs` is gone and `selector(sel).style(Tabs)` lowers to `kinds::NAV`, so a tab bar is a
>   presentation rather than a second host kind ([docs/navigation.md](docs/navigation.md) records the
>   retirement). The ladder is `Split` ≥ 840pt, `Rail` at 600–839, and when compact either `Tabs`
>   or `Stack` — `Cap::NavTabsAdaptive` decides, separately from `Cap::NavTabs` ("can draw one"),
>   because every desktop can draw a tab bar and none should GROW one from a narrowed window.
>   Pages are RESIDENT while the rows are chrome and single while they are not, switched by
>   `NavPatch::Select`, so a morph only ever disposes pages that are off screen or lazily builds
>   ones not built yet. `NavProps::adaptive` carries the app's intent to the `Emulated` backends,
>   which four presentations can no longer encode in a lowered `Split`. Drawn today by
>   macos-appkit and web-dom; the rest answer `Cap::NavTabs = Unsupported` and take the
>   pre-adaptive sidebar ladder ([docs/navigation.md](docs/navigation.md) is normative).
> - **The content list** *(2026-08)* — `selector(…).content_list(build)` adds the Mail shape's
>   middle column as a third pane role: `Pane::List`, one resident page whose content follows
>   the app's signals. `Cap::NavContentList` carries where it lands — `Native` on macos-appkit
>   (a real `contentList` `NSSplitViewItem` that persists through every presentation),
>   `Emulated` on ios-uikit (`UISplitViewController` triple-column, merged into the stack at
>   compact width, interposed by `NavPatch::ListInStack` and gated by the app's
>   `.detail_visible(sig)` binding), composed by the selector everywhere else.
>   `.content_list_for(pred)` collapses the pane per destination (`NavPatch::ListVisible`).
>   The keyboard half rides `Decorate::focusable` — the canvas focus contract generalized to
>   containers through the new `Toolkit::set_focusable` duty (appkit today).
>   [docs/navigation.md](docs/navigation.md) and [docs/focus.md](docs/focus.md) are normative.
> - **The composed gated detail is real push navigation** *(2026-08)* — where the pane is
>   composed (`Cap::NavContentList` Unsupported, and every tabs presentation), a list-backed
>   destination with `.detail_visible` no longer swaps list and detail in place. In a chrome
>   presentation the destination's page is a NESTED nav host — a `UINavigationController`
>   inside the tab, a Material toolbar over the fragment back stack — carrying the selector's
>   bar actions (a tabs chrome draws none of its own); stacked, the detail pushes onto the
>   enclosing host, the same merge a nested `stack()` performs. The native back writes the
>   signal `false`; a pop-only route surface lets `nav_back()` close the layer first. The new
>   `.detail_title(text)` names the detail layer's bar, reactively, on the native pane shapes
>   too. To keep a mid-build inner push ordered, a stacked destination page is now PRESENTED
>   (`NavPatch::Pushed`) before its content builds, not after.
> - **Nav bar actions, plural and scoped** *(2026-08)* — `NavProps::bar_action: Option<_>` became
>   `bar_actions: Vec<NavBarAction>`, and each action carries a `NavBarScope`. `bar_action` appends
>   an `EveryPage` action (the old behavior, unchanged for existing callers); `list_action` appends
>   a `RootPage` one, for commands that act on the LIST rather than on whatever page is open — on a
>   phone the detail covers the list, so an "add to the list" button pushed over the detail acts on
>   something the user cannot see. iOS draws them through `setRightBarButtonItems` (reversed, since
>   that API fills from the trailing edge inward), Android as `MaterialToolbar` menu actions tinted
>   from the bar's OWN background luminance rather than a fixed white, HarmonyOS as `.menus()`
>   items with a root-scoped action bringing out the otherwise-hidden root title bar. A MERGED
>   `stack()` has no bar of its own and warns rather than dropping them silently.
>   [docs/navigation.md](docs/navigation.md) is normative.
> - **Size-class presentation** *(2026-08)* — the split-vs-stack choice moved off `Cap::NavSplit`
>   alone and onto the WINDOW: `SizeClass` (Android's breakpoints, one table for every backend)
>   rides a per-window reactive signal that day-core derives from `Event::WindowResized`, and
>   `NavProps::presentation` is re-resolved on every change. A `selector` re-presents in place
>   through `NavPatch::Presentation` — the toolkit rebuilds its chrome and RE-HOMES the pages it
>   already has, keyed by `NavPageProps::pane` (`Sidebar`/`Detail`, a model fact rather than a
>   drawing one), so scroll offsets, focus, and the search query survive a morph that a rebuild
>   would lose. `Cap::NavRepresent` gates it per backend and expresses the split policy: desktop
>   and web are TOLD their presentation, while the mobile containers that already morph natively
>   (`UISplitViewController`, `SlidingPaneLayout`, `Navigation.mode(Auto)`) are meant to be
>   OBSERVED instead — `Cap::NavRepresent = Emulated`, with the toolkit reporting through
>   `Event::NavPresentationChanged`. Shipped: web-dom, macos-appkit and Qt (told); ios-uikit
>   (`UISplitViewController`) and android-mdc (`SlidingPaneLayout`) (observed). GTK and XAML keep
>   the pre-size-class behavior; ArkUI is untouched. `safe_area` moved to the same per-window
>   signal in the same change. [docs/size-classes.md](docs/size-classes.md) is normative.
> - **`stack(path, root)`** — push/pop navigation bound to a `Vec<Route>` signal; native back
>   (iOS swipe/button, Android system + predictive back) arrives as
>   `Event::NavBack { already_popped }` so the path signal reconciles without double-popping. A
>   `stack` nested inside a page of an enclosing push-stack host (mobile) **merges** into that
>   host — pushing its pages onto the one native container for a single back button — instead of
>   nesting a second controller; under a SPLIT host it stays standalone in the detail pane, and
>   its host is lowered `presentation: Stack` — literal *(2026-08)*: the backend realizes it as a
>   plain navigation container (a bare `UINavigationController` on iOS, a single-pane host on
>   Android) because nesting an adaptive split container inside a pane re-runs the tiling
>   decision at pane width and breaks ([docs/navigation.md](docs/navigation.md), [docs/size-classes.md](docs/size-classes.md)).
> - **Presentation** shipped as the `present`/`dismiss` duties (`PresentSpec` →
>   `PresentResult`): alert/confirm/prompt/sheets and the open/save file pickers, all native,
>   all scriptable (`assert_presented` / `respond`).
> - **`cover(open, build)`** *(2026-07)* — a fullscreen modal Day subtree bound to a
>   `Signal<Option<Route>>` (the SwiftUI `fullScreenCover(item:)` shape): `kinds::COVER` +
>   `CoverPatch`, native modal VC on iOS, window overlay on Android, topmost root child on
>   ArkUI; registers a route adapter so `navigate`/`nav_back` present and dismiss it. Ships
>   with the system-gesture shield modifiers `defers_system_gestures(edges)` (the
>   `defer_system_gestures` duty + `Edges`) and `interactive_dismiss_disabled()`.
>   [docs/cover.md](docs/cover.md) is normative.
>
> The paragraphs below are the design-era rationale, kept because the trade-offs still explain
> the shape.

Navigation is where native-widget frameworks live or die (React Native spent a decade converging
on react-native-screens because a JS-composed stack never felt native). Day's resolved
position (**DP-23**: native containers): the stack maps to native navigation hosts (back-swipe,
titles, transitions for free) with a predictive-back-compatible host on Android; desktop
composes split-pane or day-driven stacks with native-style transitions. The iOS/Android
scaffolds host Day's root inside a view controller / fragment (not a bare view), which is what
made native nav containers possible without a scaffold migration.

---

## §11 Canvas

> [!NOTE]
> **Status: shipped** ([docs/shapes.md](docs/shapes.md) is normative), and extended beyond the sketch: the
> unified **shape pieces** (`rectangle()`, `rounded_rectangle(r)`, `circle()`, `capsule()`,
> `ellipse()`, `arc(start, sweep)`) record through the same display list with path-precise
> hit-testing for gestures, and fills take a **`Paint`** — solid color, `LinearGradient`
> (unit-space start/end points), or `RadialGradient` (unit-space center + radius, stretched
> elliptically to non-square bounds) — replayed as native gradient primitives on every backend
> (NSGradient / CGGradient / cairo / QGradient / android Shader / XAML brushes / ArkUI shader
> effects). Live transforms (`.rotate`/`.inset`/`.offset` taking closures) re-record just the
> node. Later additions, still with zero backend work: `line(a, b)` / `polygon(points)` shape
> kinds (unit-point geometry over the already-replayed `Shape::Line`/`Shape::Polygon` ops),
> fractional placement via `.at(fx, fy, fw, fh)`, and `shape_group` / `shape_group_fn` — many
> shape descriptions flattened into ONE canvas leaf ([docs/shapes.md](docs/shapes.md) §3.6; Day Skies' weather
> glyphs and range bars are the reference consumers).

```rust
pub fn gauge(value: Signal<f64>) -> impl Piece {
    canvas(move |d, size| {
        let r = Rect::from_size(size).inset(8.0);
        d.stroke(arc_path(r, 135.0, 270.0), Color::rgba(0.5, 0.5, 0.55, 0.35), 6.0);
        d.stroke(arc_path(r, 135.0, 270.0 * value.get() / 100.0), Color::hex(0x2F6FDE), 6.0);
        d.text(&format!("{:.0}", value.get()), r.center(), TextAnchor::Center, Font::Title);
    })
    .frame(120.0, 120.0)
    .a11y(|a| a.role(Role::Meter))
}
```

- The closure is a **binding**: reads are tracked; any signal change re-records and re-replays just
  this node.
- `Draw` **records** into a `Vec<DrawOp>` (fill/stroke path, rect, rounded-rect, ellipse, line,
  text run, image, clip, transform, save/restore — types from `day-geometry`); the backend
  **replays natively** (`replay()` in [§8.1](#81-the-toolkit-trait)) — CoreGraphics, android Canvas, cairo, QPainter,
  Direct2D, `<canvas>`. One FFI hop per redraw (the op buffer is a packed, pod-friendly encoding),
  not one per op — this matters on Android/JNI.
- Display lists make canvas **unit-testable on `day-mock`** (assert ops) and diffable —
  `DrawOp: PartialEq` is the [§4.2](#42-primitives) binding equality gate, so an unchanged recording skips the
  replay entirely.
- Text on canvas uses the toolkit's text engine via `DrawOp::Text` (native fonts, shaping, BiDi) —
  Day never rasterizes text. Per-toolkit shaping engines are pinned in the design because the
  defaults are traps: **PangoCairo** on GTK (cairo's "toy" text API has no shaping or BiDi),
  CoreText on apple targets, `QPainter::drawText` (harfbuzz underneath) on Qt,
  `android.graphics.Canvas.drawText` (minikin), DirectWrite via the XAML shim.
- Pointer/key events opt in: `.on_pointer(f)`. Accessibility of canvas content: MVP = the canvas
  node is one a11y element (label/value/role as above); **virtual child elements**
  (`UIAccessibilityElement` / `AccessibilityNodeProvider` / … ) are specified as a post-MVP
  extension of `A11yProps` so drawing-heavy pieces are not a11y holes forever.

---

## §12 Localization (Fluent)

### §12.1 Files and keys

```
resource/locales/           # under the project's resource/ tree (§18.3)
  en/app.ftl                # default locale
  fr/app.ftl
  ar/app.ftl
  zh-CN/app.ftl
```

```ftl
# locales/en/app.ftl
app-title = Showcase
controls-title = Controls
name-placeholder = Your name
greeting = Hello, { $name }!
volume-label = Volume
counter-value = { $count ->
    [one] { $count } click
   *[other] { $count } clicks
}
increment = Increment
decrement = Decrement
```

### §12.2 API

> [!IMPORTANT]
> **Status: shipped with deltas** ([docs/localization.md](docs/localization.md) is normative). The engine is
> `day-l10n` with `day-fluent` as the app-facing API (`install_locales(default, &[(locale,
> ftl_source)])` compiles the bundles in via `include_str!` — normally through the generated
> `res::locales::install()`, [§18.5](#185-typed-resource-constants-docsresourcesmd); `set_locale`
> switches live). **Registration moved into `launch` (2026-08)**: `WindowOptions::locales`
> carries `(DEFAULT, CATALOG)` and `day::start` installs it immediately after the backend's
> `locale_hints` reach day-l10n — the only ordering that resolves against the device's
> languages, and early enough for `WindowOptions::title_fn` to take a window title from the
> catalog. An app installing its own catalog before `launch` resolved against an empty hint
> list and opened in `DEFAULT`. `day::resources!()` surfaces the generated module in the same
> spirit: one line where the app used to write the `include!` itself. The
> **preferred authoring surface is now the generated `res::str::key(args…)` functions**
> ([§18.5](#185-typed-resource-constants-docsresourcesmd)) — typed, autocompleted, compile-checked keys — with `tr("…")` remaining for dynamic
> keys. Keys are therefore **snake_case** (they must be Rust identifiers), not kebab-case as
> sketched below. The ICU4X-backed `NUMBER`/`DATETIME` Fluent functions **shipped** (2026-07):
> `day-l10n` registers them — plus a bundle-wide number formatter, so plain `{ $n }`
> interpolations localize too — on every bundle via icu4x 2.x (in-tree, not fluent-datetime),
> with locale-aware collation (`compare`/`sort_localized`) alongside ([docs/localization.md](docs/localization.md)
> "Formatted values"/"Sorting"/"Locale data" are normative). Apps embed icu4x's all-locale
> `compiled_data`: the CLI's per-app thinning was removed in 2026-08, having cost more in the
> CLI's own graph than it saved in an app's binary ([docs/localization.md](docs/localization.md) "Locale data").
> Plural/`select` rules work (exercised by every locale in CI), and the `res::str` typing
> forces numeric arguments where CLDR plural selection needs them. `en-XA` pseudolocalization
> shipped; `ar-XB` did not (a real `ar` locale covers RTL, [§7.8](#78-rtl-and-bidi)).

```rust
label(res::str::greeting(name))               // generated, typed (name: Signal<String> — live)
button(res::str::increment())
label(tr("app-title"))                        // dynamic-key escape hatch
```

- `tr(key) -> LocalizedText` implements `IntoText`. `.arg(k, v)` accepts values, signals, and
  closures; Fluent handles plurals/genders/selection.
- **Number/date formatting is NOT free**: fluent-rs registers no default `NUMBER`/`DATETIME`
  functions and does no locale-aware number rendering. `day-l10n` registers **ICU4X-backed
  functions** (`icu_decimal`, `icu_datetime` — in-tree, `crates/day-l10n/src/intl.rs`) into every
  bundle, plus a `set_formatter` hook so plain number interpolations localize; fr/de
  digit-grouping and plural-rules conformance is pinned by `crates/day-l10n/tests/intl.rs`, and
  `day lint` flags `.ftl` references to unregistered functions and invalid format options.
- `IntoText` is a two-level design (a naive flat impl set is uncompilable — two closure blankets
  distinguished only by `Fn::Output` overlap): a sealed `TextValue` (String, `&'static str`, Cow,
  `LocalizedText`) plus exactly one closure blanket `impl<F: Fn() -> T, T: TextValue> IntoText for F`,
  plus concrete impls for `Signal<String>`/`LocalizedText`/`String`/`&str` (bare literals
  discouraged for user-facing text). The same pattern serves Fluent `.arg` values; compile-pass
  tests for all call shapes land in M1.
- `day lint` covers fluent coverage — keys missing from locales, unused keys, unknown key
  references (strict mode for CI); the bare-literal warning was not built (`res::str` makes
  keyed strings the path of least resistance instead).
- The **current locale is a `Signal<LanguageIdentifier>`** in `day-fluent`, initialized from
  (1) `--locale` launch override → (2) OS preference list (`Platform::locale_hints`, negotiated
  via fluent-langneg) → (3) default. Every `tr` binding reads it, so a locale change updates every
  visible string fine-grained, then one incremental relayout ([§7.5](#75-window-sizing)'s grow-never-shrink window
  policy; German is long). Each binding captures its resolved message reference once per locale —
  the per-locale parsed-bundle cache is the only cache (no (key, args) memo: Fluent args include
  `f64`, and applies are already equality-gated).
- **Per-target locale plumbing** (`--locale` must move the *whole app*, not just Day's strings):
  iOS Simulator launches pass `-AppleLanguages` via simctl; Android applies the intent-extra
  locale via `Locale.setDefault` + `createConfigurationContext` (per-app locale API on 33+) and
  routes `onConfigurationChanged` → locale signal; apple backends set `accessibilityLanguage`
  from the locale signal. Residual mixed-locale surfaces (out-of-process dialogs) are documented
  honestly.
- Fluent sources compile into the binary (the `.ftl` files under `resource/locales/` are the
  source of truth for the codegen, the lint, and the runtime alike), with per-message fallback
  to the default bundle. The `include_str!` list is generated too since 2026-07:
  `res::locales::install()` ([§18.5](#185-typed-resource-constants-docsresourcesmd)) registers
  every locale directory, so adding a language is adding a directory; `install_locales(default,
  &[(locale, ftl_source)])` remains public for app-assembled lists. Fluent's `use_isolating` stays
  **on** (FSI/PDI isolation marks around placeables); dayscript text comparison normalizes
  U+2068/U+2069 ([§14](#14-scripting-dayscript), [Appendix C](#appendix-c--dayscript-reference-v1)).
- **Native-side metadata** localization (generated `InfoPlist.strings` / `strings.xml` display
  names from reserved Fluent keys) was **not built** — the display title comes from
  `Day.toml [app] title` un-localized. The design stands for when a store submission needs it.
- Pseudolocales ship built-in: **`en-XA`** (expansion + accents) and **`ar-XB`** (RTL, [§7.8](#78-rtl-and-bidi)).
  Pseudolocalization parses messages with `fluent-syntax` and transforms only `TextElement`s
  (naive string transforms corrupt placeables and selectors), and pseudolocales bypass negotiation
  (an explicit pre-negotiation check — otherwise `en-XA` silently negotiates to `en`):
  `day launch --locale en-XA`.

---

## §13 Accessibility

**Native-first**: because every interactive Piece is a real native control, screen readers, switch
access, and keyboard navigation work at the level the platform provides *before Day adds anything*.
Day's job is to (a) not break it, (b) provide the uniform annotation API, (c) enforce policy.

```rust
button(icon("trash"))
    .a11y(|a| a.label(tr("delete-item")).hint(tr("delete-item-hint")))
    .id("delete-button")

image(ImageSource::asset("chart"))
    .a11y(|a| a.label(tr("q3-chart-summary")))     // or .decorative()

canvas(…).a11y(|a| a.role(Role::Meter).value_with(move || …))
```

- `A11yProps { label, hint, value, role, live, hidden, identifier }` — all text fields are
  `IntoText` (a11y strings are localized like any other, and they update reactively).
- Roles map to native: `Role::Button/Toggle/Slider/TextInput/Heading(level)/Image/Meter/Group/…` —
  most built-ins set their role automatically; `role` matters for canvas and custom pieces.
- **Identifiers** ([§5.5](#55-node-identity-ids-and-the-element-index)): the verified per-toolkit truth table — no pretending:

  | toolkit | native automation-id channel |
  |---|---|
  | UIKit / AppKit | `accessibilityIdentifier` ✓ |
  | XAML | `AutomationId` ✓ |
  | Qt | `QObject::setObjectName` (surfaces as UIA AutomationId on Windows) ✓ |
  | Android | `uniqueId` via `AccessibilityDelegate` on **API 33+**, plus `setTag` for in-process use — **no external automation id below 33** (`setTag` is invisible to UiAutomator/Appium; abusing `contentDescription` for ids is forbidden by lint because TalkBack reads it aloud) |
  | GTK | widget *name* is GtkInspector-only — **no public settable AT-SPI accessible-id today** (tracked upstream) |
  | web | DOM `id` ✓ |

  dayscript is unaffected by the gaps (its element index reads day-core, [§14.2](#142-the-embedded-engine)); the table
  matters for *external* tools (Appium, UIA scrapers) and is documented per target.
- Policy: `day lint` a11y rules — interactive piece without a derivable label (icon-only button,
  unlabeled image) is a warning, `--strict` error; ids leaking into a11y labels is an error
  ([§5.5](#55-node-identity-ids-and-the-element-index)). Focus order follows layout order; programmatic keyboard focus is its own shipped
  subsystem (`.focused()`, [docs/focus.md](docs/focus.md)); `.focus_group` and `.a11y_sort_priority` remain
  unimplemented.
- **Verification is automated**: the dayscript `a11y_audit` step ([§14](#14-scripting-dayscript), [Appendix C](#appendix-c--dayscript-reference-v1)) walks the
  *native* accessibility tree in-process and diffs it against day-core's expectations — nothing in
  CI trusts `set_a11y` blindly.
- Reality check per toolkit lives in [§9](#9-the-eight-toolkits-and-the-extra-combinations); the honest summary: primary combinations have first-class
  native a11y; `macos-gtk`/`windows-gtk` currently have none (GTK's AccessKit backend exists as of
  4.18 but isn't in default builds), which is precisely why they're secondary. Qt is solid on all
  three desktop OSes.

---

## §14 Scripting (dayscript)

### §14.1 A script

> [!NOTE]
> **Status: shipped** — the showcase's real `dayscript/walkthrough.yaml` runs 200+ steps on
> every scripted target; [Appendix C](#appendix-c--dayscript-reference-v1) lists the shipped step catalog exactly.

```yaml
# dayscript/walkthrough.yaml
name: showcase-walkthrough
description: Exercise every control and take localized screenshots.
flow:
  - wait_for: { id: controls-title }
  - screenshot: home
  - input: { id: name-field, text: "Ada" }
  - assert_visible: { id: greeting-label }
  - assert_text: { id: greeting-label, key: greeting, args: { name: "Ada" } }
  - set_value: { id: volume-slider, value: 80 }
  - assert_text: { id: volume-value, text: "80" }
  - tap: { id: subscribe-toggle }
  - assert_value: { id: subscribe-toggle, value: true }   # typed per piece kind (§C): toggle=bool
  - tap: { id: increment-button, repeat: 3 }
  - assert_text: { id: counter-label, key: counter-value, args: { count: 3 } }
  - screenshot: after-actions
```

Note `assert_text` with `key:` — assertions can reference **Fluent keys**, so one script passes in
every locale (the engine resolves the key in the app's active locale). This is what makes
`day launch --locale fr-FR --script walkthrough.yaml` a per-locale test *and* a per-locale
screenshot generator with zero per-locale script maintenance.

The shipped step catalog — waiting (`wait_for`, `wait_idle`, `pause`), acting (`tap`, `input`,
`set_value`, `toggle`, `select`, `focus`), navigation (`navigate`, `deep_link`, `nav_back`, `assert_route`),
asserting (`assert_visible`, `assert_missing`, `assert_text`, `assert_value`, `assert_focused`), dialogs
(`assert_presented`, `respond`), evidence (`screenshot`, `a11y_audit`), and termination
(`expect_exit` — the one step that tolerates the app dying, for crash-reporting flows,
[docs/break.md](docs/break.md)) — is specified in
[Appendix C](#appendix-c--dayscript-reference-v1), with `day drive` exposing the same vocabulary to agents ([docs/agent.md](docs/agent.md)).

### §14.2 The embedded engine

`day-script` compiles **into the app** (cargo feature `dayscript`, on by default in debug profiles;
in release only if `Day.toml` sets `scripting.release: true` — and `day pack` verifies that
release artifacts without the opt-in contain no engine). It:

- maintains the **element index**: id → NodeId (from [§5.5](#55-node-identity-ids-and-the-element-index)), plus role/text/value accessors that
  read day-core's cached last-applied props (not platform a11y trees — one implementation, all
  toolkits; the `a11y_audit` step below is the deliberate exception that reads the native tree);
- executes steps **as synthesized Day events** (tap = the button's action path; input = the
  controlled-text path), on the main thread, between flushes (`flush_sync`, [§3.3](#33-threading-model-and-the-turn-state-machine)) — deterministic
  and toolkit-uniform. (Driving *native* input synthesis instead is deliberately rejected for v1:
  per-toolkit event forgery is flaky and permission-gated. DP-13.)
- does **not** enforce the designed actionability preconditions (enabled/occlusion checks,
  auto-scroll-into-view) — that gating was never built; scripts scroll explicitly where needed
  and target ids they know to be interactive ([Appendix C](#appendix-c--dayscript-reference-v1) notes this per step).
- is honest about **what it cannot verify**: the native keyboard and IME, native hit-testing,
  native animations, and out-of-process UI. Manual checks in M2/M5/M6 acceptance carry that load.
- serves the **transport** ([§14.5](#145-transport-and-rendezvous)), implements `screenshot` via `Toolkit::snapshot_window` (on a
  device or simulator the runner prefers the platform's own screen capture, so it asks the
  engine to SKIP its render — `in_process: false` — and re-asks only if that capture fails;
  rendering one per shot and discarding it cost 33.6s of a single iOS walkthrough variant —
  [docs/window-image.md](docs/window-image.md)), and
  implements **`a11y_audit`**: walk the *native* accessibility tree in-process
  (NSAccessibility/UIAccessibility — hop's proven recipe; `AccessibilityNodeInfo` on Android;
  GtkAccessible/QAccessibleInterface where present), diff role/label/identifier against day-core's
  expectations for every node with an `.id()`, and report through the normal step-result path.
  Required in M6 acceptance and the CI walkthrough on apple targets.

### §14.3 Waits and flakiness

Every retryable step has an implicit bounded wait (5 s default) — element not found yet and
pending assertions poll rather than fail instantly. `wait_idle` flushes the reactive drain;
`screenshot` additionally waits on `Toolkit::ui_idle` (native transitions settled), which is
what keeps captures from showing half-dismissed dialogs. (The designed richer idle definition —
in-flight `Resource`s, `busy_scope()` — remains unbuilt even now that `Resource` shipped
([§4.5](#45-async)): the bounded-retry asserts absorb async gaps, as the showcase's Resource
walkthrough steps show.) No sleeps in
well-written scripts; `pause` exists for demos. Text assertions normalize Fluent's FSI/PDI
isolation marks ([§12.2](#122-api)).

### §14.4 Results

> [!IMPORTANT]
> **Status: shipped differently.** The runner is `--script` on `day launch` (exit code 5 on an
> assertion failure — the CI entry point) and `day drive` for step-at-a-time agent sessions;
> a standalone `day script` command and JUnit XML output were not built. Screenshots land in
> `build/day/screenshots/<target>/<locale-or-variant>/<name>.png` (`--variant` names themed
> sets, e.g. `--variant dark --env DAY_THEME=dark`); JSON results ride the global
> `--format json` NDJSON stream.

### §14.5 Transport and rendezvous

> [!IMPORTANT]
> **Status: shipped simpler.** The protocol is **newline-delimited JSON over localhost TCP**,
> defined by serde types inside `day-script` itself (`Request { token, step }` → `Reply { ok,
> error, retryable, png_base64, … }`); the separate `day-script-proto` crate and length-prefixed
> framing were dropped. Screenshots return as base64 within the reply.

**Rendezvous** (parallel targets share the host loopback — fixed ports are a design bug): the
engine binds **only when invited** — `DAYSCRIPT_PORT` + `DAYSCRIPT_TOKEN` present in the
environment (`SIMCTL_CHILD_*` for the Simulator, intent extras on Android) — never otherwise,
debug or release. The launcher picks the port and generates the one-time token; every request
carries it, and a wrong/missing token is refused. `day drive` attaches to the same session
registry (`day stop` tears sessions down).

| environment | transport | handshake |
|---|---|---|
| desktop (macOS/Linux/Windows) | localhost TCP (UNIX socket optional alt) | handshake file |
| iOS Simulator | localhost TCP (simulator shares host loopback) | handshake file via `simctl` container path |
| Android emulator/device | abstract UNIX socket `localabstract:dayscript.<app-id>` + `adb forward tcp:0` (adb assigns the host port; no on-device TCP port) | forwarded port + on-device handshake file |
| iOS device | post-MVP (usbmux tunnel) | — |
| web | WebSocket — shipped 2026-07 as sketched: the page opens `ws://<dev-server>/dayscript`, the `day launch` server bridges it to the SAME TCP protocol on `DAYSCRIPT_PORT`, so the runner is unchanged ([docs/web.md](docs/web.md)) | token in the page's `?dayscript=` query parameter |

The engine binds `127.0.0.1` only and is **not** a general remote-control surface: the protocol
allows only the step catalog.

### §14.6 Recording

> [!NOTE]
> **Status: shipped.** `day::record` (in `day-script`) plus `day launch --record <file>`; the
> showcase's **Scripting** page records and replays in-process. A recorded script is an ordinary
> dayscript.

Recording is playback run backwards: instead of turning a script into events, it turns the events
an app receives back into a script. It hangs off **one seam** — `day_core::set_event_observer`, an
optional observer that `enqueue_events` ([§8.3](#83-events)) calls for every `(NodeId, Event)` in
queue order, *before* dispatch, so it sees exactly what the app is about to receive. That is the
single point every backend funnels native events through, so the recorder needs **no per-toolkit
code** and no changes to the eight backends; a `None` observer costs nothing on the event path.
`day_core::id_of(NodeId)` (the inverse of the element index's `find_by_id`,
[§5.5](#55-node-identity-ids-and-the-element-index)) turns the dispatched node back into the
app-authored id a step would target.

Scope is **actions only, and only where the step is portable**:

- `Pressed` **and** `Tap(Point)` → `tap`, `TextChanged` → `input` (coalesced per field),
  `SelectionChanged`/`ToggleChanged` → `select`, `RouteRequested` → `navigate` (coalesced),
  `NavBack` → `nav_back`.
- `ValueCommitted` → `set_value`: a slider records the value it SETTLED on, once.
- **Dropped, deliberately:** gestures other than tap, `ValueChanged` (the live value a drag
  streams — the settled one arrives separately), multi-select, and every lifecycle/menu/toolbar/
  window event. An **id-less** action has no portable step, so it is dropped too — including a
  bare positional tap.

A continuous control produces two different facts and needs both: `ValueChanged` fires on every
motion so bindings track the thumb, and nothing durable can key off it (a drag from 1 to 100 and
back to 50 emits every value between). `ValueCommitted` fires once, with the value the user chose.
Every toolkit can tell them apart, though none the same way — Qt has `sliderReleased` and
`isSliderDown`, ArkUI hands over the `SliderChangeMode` directly, Android has
`onStopTrackingTouch`, UIKit the touch-up control events, the DOM separates `input` from `change`,
and AppKit and XAML have neither signal so they read the interaction that provoked the callback
(the current `NSEvent`'s type; the thumb's pointer capture being lost). `Step::SetValue` synthesizes
both, so a replayed `set_value` looks like a user who dragged and let go.

Both tap shapes record because a control's shape decides which one it gets. A native `button` leaf
delivers `Pressed`; a `Button::style(…)` is not a leaf at all but a piece COMPOSED from
`Decorate::on_tap` ([§5.3](#53-the-piece-vocabulary)), and delivers only `Tap` — as does every
tappable shape or card. `Step::Tap` has always synthesized both for exactly that reason, so
recognizing one of them made every styled button replayable and unrecordable at once: it recorded
as nothing, silently. A node that delivers both in one pump records once.

That defect is the reason for two guards, because each half was self-consistent and only the
comparison finds the gap. `playback_and_recording_agree` (day-script) pins the executor's emitted
events against `event_to_step`: **every** event a recordable step emits must map back to that step,
since one mapping is not enough when different piece kinds receive different events. And
[`docs/recorder-matrix.md`](docs/recorder-matrix.md) is generated from `day-spec`'s `Event` enum and `event_to_step`
([§20](#20-continuous-integration)), so a new variant lands in a diff as *dropped* rather than
falling into the catch-all unremarked.

Navigation is captured off a **second seam**, not the event observer: a sidebar row, a `nav_link`,
and a stack push change the route by calling `navigate`/`pop` from an event handler, none of which
pass back through `enqueue_events`. So the route is watched instead — the nav hosts call
`day_core::note_navigation(route, label)` synchronously from their selection handlers, and the
event pump re-checks `current_route()` at each boundary as a fallback for the signal-bound hosts
whose route settles a frame late. Each is recorded as one **absolute** `navigate`, which replays a
multi-level stack (`items/item-1`) in a single step, folding in the tap/select that triggered it.

Every recorded step is **annotated** with the control it came from — a trailing `# "label"` comment
on the `id:`/`route:` line (`route: focus # "Focus"`, `id: focus-next-button # "Focus next"`),
naming the element's accessibility label, or its visible text when it has no a11y label, in the
current locale. Comments are ordinary YAML, so an annotated script parses and replays unchanged;
`annotate_yaml` renders this form, `steps_to_yaml` the bare one. `day_core::label_of(NodeId)`
resolves the label.

**Action logging** is the same machinery with the capture removed: `day::record::log_actions(true)`
(or `DAY_LOG_ACTIONS=1`, read in `day_script::init` beside `DAY_RECORD`) installs the same two
observers and echoes each action in the same vocabulary, keeping no steps and writing no file — so
it costs what the observer costs and never grows, and an app can leave it on for its whole life.
The prefix names the mode (`dayscript ▸` logging, `day record ▸` recording) and `exclude_prefix`
applies to both. The two are independent: logging survives a recording starting and stopping under
it, and a live recording emits one line per action, not two. daybrite/Day-Showcase turns it on in
`root()`, so its console reads as the script a recording would have produced.

Known gaps follow from the scope: an element the app never gave an `.id()` cannot be recorded;
slider values and native OS chrome (the file picker, the IME, permission dialogs) are outside what
Day observes — the same blind spots playback has ([§14.2](#142-the-embedded-engine)). A recording
is therefore a **starting point to edit**, not a pixel-exact replay.

The API is small: `record::{start, start_into, start_to_file, stop, is_recording,
recording_signal, script, steps, save, clear, exclude_prefix}`, with `exclude_prefix` keeping a
UI's own record/stop controls out of its own recording. `start_into(Signal<String>)` streams the
script into an editable buffer (the showcase binds a `text_area` to it); `start_to_file` flushes
continuously, so `DAY_RECORD` / `day launch --record` capture headlessly and survive a kill. The
on-disk form is the ordinary `flow:` document (`steps_to_yaml`/`steps_from_yaml` are the exact
inverse of day-cli's `parse_flow`), so a recorded script replays cross-toolkit through
`day::play_script(yaml)` in-process **or** `day launch -p <other-target> --script <file>` — record
on one backend, replay on any.

### §14.7 Screenshot metadata and the gallery index

A `screenshot:` step may carry gallery metadata beside its capture keys: `title:` and
`caption:` (a plain string, or a locale-keyed map — `title: { en: "Home", fr: "Accueil" }`)
and `source:` (the path of the code the screen renders from, relative to the app repository).
The metadata lives on the step because that is where the capture is declared; it is
**runner-side only** — day-cli strips the keys before the step reaches the engine, so apps and
the day-script protocol are untouched (Appendix C lists the keys).

The runner folds every capture it saves into `build/day/screenshots/<target>/gallery.json`
(upserted across runs and variants; entries whose files are gone are pruned), carrying the
step's metadata plus the file's facts — pixel dimensions, byte size, sha-256, and the run's
actual locale. `day screenshot index` (§16.5) merges those per-target files into one
`gallery.json`: shot order is the dayscript's declaration order, titles/captions ship as
locale maps in `shots[]` and resolved per-capture (each entry's own locale, falling back by
primary language then English), and `website/site.toml`'s `host` turns paths into published
URLs. App sites serve the result at `<host>/gallery/gallery.json` — the machine-readable
index other sites and tools reference (crates/day-cli/src/screenshot.rs).

A shot **with** a `title:` is gallery-curated: the daysite gallery shows the curated set when
one exists, and untitled captures stay machine-readable in the index. `day lint`
cross-references each title/caption map's locale keys against the app's translation locales
(missing = that page silently falls back to English; unknown = usually a typo).

---

## §15 Extensibility: pieces, parts, and tweaks

> [!IMPORTANT]
> **Status: shipped differently — and simpler.** The promise held: external crates add UI and
> platform services without touching Day or the app's scaffolds. The mechanism did not need a
> C ABI. [docs/extending.md](docs/extending.md) is the normative reference; [docs/tweaks.md](docs/tweaks.md) covers tweaks. The
> section title changed from "Day Piece packages (polyglot)" to match the shipped taxonomy.

### §15.1 The promise

Anyone can publish an extension crate exposing a unified Rust API whose native halves, where
needed, are written in the *platform's own language with its own conventional dependencies* —
Swift (+ SwiftPM packages) for ios/macos, Java (+ Gradle/Maven deps) for Android, C++ shims for
Qt/XAML/ArkUI — without touching Day or the app's platform scaffolds.

The shipped ladder, cheapest first (a single package may mix rungs per toolkit):

- **Tweaks** (below composition; [Addendum](#addendum-2026-07-09--tweaks-per-toolkit-configuration-of-built-in-pieces), [docs/tweaks.md](docs/tweaks.md)): configure the native widget behind
  an existing built-in — `Decorate::tweak`/`native_ref`, packaged as `tweaks/day-tweak-*`.
- **Tier 0 — composition:** pure Day pieces (a gauge from `canvas`, `day-piece-rating`). No
  native code.
- **Tier 1 — Rust renderers:** per-toolkit renderers written in Rust against the backend's own
  FFI (objc2 / gtk4-rs / jni / the C++ shims), registered link-time into each backend's
  `RENDERERS` slice with the `renderer!` macro ([§8.2](#82-the-open-renderer-registry)). Most `pieces/day-piece-*` crates are
  this tier.
- **Native halves:** where a piece or part needs platform-language code or third-party native
  libraries, its `Cargo.toml` declares them under **`[package.metadata.day.<platform>]`** and
  `day build` folds them into the app's native build ([§15.2](#152-package-layout-and-aggregation)). Events come back through the
  standard sink (`Event::Custom { tag, num, text }` for open piece-defined events); foreign
  views enter the tree via `Toolkit::adopt`.

Two package kinds share the mechanism:

- **Pieces** (`pieces/day-piece-*`): UI — combobox, search field, rating, activity,
  datetime, color picker, styled-text editor, pull-refresh, webview, media, map, lottie,
  remote-image.
- **Parts** (`parts/day-part-*`): headless platform services exposing signals/functions —
  battery, network, sensors (streaming, [docs/sensors.md](docs/sensors.md)), clipboard, prefs, haptics, deviceinfo,
  http (requests through the platform HTTP stack, [docs/http.md](docs/http.md)), permissions (the OS consent system
  plus the build-time declarations each platform requires, [docs/permissions.md](docs/permissions.md)), location
  ([docs/location.md](docs/location.md)), fs (app-local file storage, [docs/fs.md](docs/fs.md)), local-notify (local notifications:
  post or schedule, channels, tap-to-route, [docs/notify.md](docs/notify.md)), and timezone (the wall clock — also on
  wasm, where std has none — plus IANA zone facts from a bundled tzdb, [docs/timezone.md](docs/timezone.md)). Same
  registration and metadata machinery, no widget.

### §15.2 Package layout and aggregation

The shipped layout — everything rides `Cargo.toml`, no side manifest:

```
day-piece-lottie/
  Cargo.toml            # the Rust API crate (one feature per toolkit) + [package.metadata.day.*]
  src/lib.rs            # pub fn lottie(source) -> impl Piece  + per-backend renderer! modules
  android/java/…        # Java shim sources, staged into the app's Gradle build
  ios/…                 # Swift shim sources, compiled into the generated DayPieces package
```

```toml
[package.metadata.day.android]
java = ["android/java"]                    # dirs → Gradle java srcDirs
res = []                                   # dirs → Gradle res srcDirs (piece-shipped styles/drawables)
gradle-dependencies = ["com.airbnb.android:lottie:6.x"]
gradle-repositories = []                   # extra Maven repos if needed
permissions = []                           # <uses-permission> entries merged into the manifest
proguard = []                              # R8 keep rules for classes native code reaches by name

[package.metadata.day.ios]
swift = ["ios/swift"]                      # Swift shim source dirs
swift-packages = [{ url = "https://github.com/airbnb/lottie-ios", from = "4.0.0", products = ["Lottie"] }]
frameworks = ["CoreLocation"]              # system frameworks the app must link (xcodebuild ignores Rust #[link])
platform = "16.0"                          # optional: minimum OS this contribution needs (max across crates wins)

[package.metadata.day.macos]               # the macos-appkit leg (docs/swiftui.md): same shape as .ios,
swift = ["apple/swift"]                    # compiled by `swift build` and statically linked into the cargo binary
swift-packages = [{ path = "swiftui", products = ["MyViews"] }]  # local packages allowed on both Apple legs;
                                           # public SwiftUI views in them are scanned and exported (docs/swiftui.md)

[package.metadata.day.ohos]
ets = ["ohos/ets"]                         # ArkTS source dirs, staged into the hvigor project

[package.metadata.day.permissions]
uses = ["camera"]                          # PORTABLE permissions this crate needs (docs/permissions.md)
```

`[package.metadata.day.permissions]` is machine-facing only: a library declares WHICH permissions it
needs, never the user-facing reason, which is app copy and lives in the app's `[permissions]` table
in Day.toml. A contribution the app has given no reason for is a hard build error on iOS and
HarmonyOS — the alternative is an app that builds and then terminates on a device.

Qt/XAML/ArkUI native halves are C++ compiled by the crate's own `build.rs` (the `-sys`
convention, with `day-toolchain` locating SDKs) — no metadata needed. The exception is a HarmonyOS
component that exists only in **ArkTS** (the C node API cannot construct a `Web` or a `Map` at all):
those pieces declare `ets` dirs above, and day-arkui's generic piece bridge (`registerPiece` /
`pieceEvent`) mounts the ArkTS-built FrameNode in the Day tree — one seam for every such piece, not
one shim entry point per piece ([docs/extending.md](docs/extending.md)). OS-API *parts* select
their half by OS (`cfg(target_os)`), so battery on `macos-gtk` gets the IOKit half, exactly the
extra-combo case the design worried about.

> [!NOTE]
> **Status: one deliberate exception (2026-07).** Permission declarations ([docs/permissions.md](docs/permissions.md)) are
> written into two CHECKED-IN scaffold files: iOS/macOS `Info.plist` usage-description keys, and a
> marker region in HarmonyOS's `module.json5`. `sync_uiappfonts` already set that precedent for
> `UIAppFonts`. Two alternatives were evaluated and rejected: `INFOPLIST_KEY_*` build settings are
> consumed only when `GENERATE_INFOPLIST_FILE = YES`, which the scaffold pbxproj sets to `NO`
> (flipping it is a full scaffold rewrite plus a migration); and pointing `INFOPLIST_FILE` at a
> generated merged plist is architecturally right but breaks the IDE escape hatch — ⌘R in Xcode
> would produce an app that crashes on first camera use with no signal that `day build` was
> required. What keeps the exception honest: Day writes and removes ONLY keys inside a closed
> managed set derived from the declaration table, every other byte is preserved, and two consecutive
> builds produce a byte-identical file.

**Aggregation never mutates the scaffolds** — this principle shipped intact for everything else. `day build` reads
the resolved dependency graph via `cargo metadata`, collects every crate's
`[package.metadata.day.<platform>]`, and regenerates gitignored files the checked-in scaffolds
reference generically, exactly once:

- **android**: contributions land in `build/day/android/day-pieces.json`; the app's committed
  `build.gradle.kts` loops over its lists (srcDirs, dependencies, repositories) — no per-piece
  Gradle edits, ever. Permissions merge through a generated manifest overlay. Release builds minify
  with R8: since Day reaches Java by name (JNI FindClass, `dcall_static`, reflection), `day build`
  also folds in keep rules — day-android's own (the whole `dev.daybrite.day.**` namespace) plus each
  app/piece's declared `proguard` file — so minification never renames a JNI-reached class out from
  under native code ([docs/extending.md](docs/extending.md)).
- **apple**: the CLI generates a LOCAL SwiftPM package at `build/day/ios/DayPieces` whose
  `Package.swift` lists every piece's `swift-packages` and compiles every piece's staged Swift
  shims; the checked-in `.xcodeproj` depends on that one package — adding an iOS piece is pure
  `Cargo.toml` data, no `.xcodeproj` edits. (Flutter's generated-plugin-package pattern,
  as designed — under the shipped name `DayPieces`.) `swift-packages` entries may also be **local**
  (`{ path = "…" }`, relative to the declaring crate): the package's transitive SwiftPM
  dependencies come along, and its public SwiftUI views are scanned and exported as typed Rust
  constructors plus generated hosting glue ([docs/swiftui.md](docs/swiftui.md)). A `platform` key raises the
  generated package's floor and the leg's deployment target (conveyed as an xcodebuild
  command-line setting — the pbxproj is never edited).
- **macos** (2026-08, [docs/swiftui.md](docs/swiftui.md)): the same aggregation for `[package.metadata.day.macos]`,
  scaffold-free because macos-appkit has no scaffold to reference it — the CLI generates
  `build/day/macos/DayPieces` (static library product), builds it with `swift build`, and
  statically links the archives into the cargo binary via `cargo rustc -- <link args>`
  (`-force_load` on the DayPieces archive keeps the by-name-resolved provider classes). Apps with
  no macOS Swift contributions keep the exact prior cargo build, with no Swift toolchain
  requirement.

- **harmonyos**: piece `.ets` stages into the hvigor project at `entry/src/main/ets/daypieces/<crate>/`
  beside a generated `DayPieces.ets` aggregator, whose `registerDayPieces(uiContext)` the checked-in
  host page calls once. Hvigor compiles ArkTS only from inside the module, so unlike the android/apple
  legs these land in the project rather than `build/day/` — the scaffold gitignores the directory, and
  the generated pair is rewritten from scratch every build so a removed piece leaves nothing behind.

This mirrors how Flutter plugins carry `android/`/`ios/` folders the tool weaves into host
projects. It is the reason Day's scaffolds are real Xcode/Gradle projects ([§17](#17-the-conventional-day-project-and-daytoml)).

**One module per build system, not one per crate (decided 2026-08).** Every contributing crate's
native sources land in a *single* generated unit per platform: one `DayPieces` SwiftPM target, one
Gradle `:app` module (sources referenced in place through `srcDirs`, never copied), one `daypieces`
ArkTS directory. The alternative — a module per crate, `day-x` → `DayX` — was evaluated and
declined for now, because the price of a module differs by an order of magnitude across these build
systems while the benefit does not. Gradle and hvigor modules cost configuration time, a manifest,
a namespace, an R class, and a resource-merge pass each, and buy nothing: Java/Kotlin packages
already namespace, and Android never stages a copy at all. Swift is the exception in kind: a
SwiftPM target IS the module, and **subdirectories inside it are not namespaces**, so two crates
declaring the same module-scope type collide with an error the app author cannot fix in either
crate.

The chosen answer for that is detection rather than partition: day-build already parses contributed
Swift for the SwiftUI bindings, so it collects top-level type names per crate and fails with both
crate names when two collide. Two enhancements stay on the shelf, each with a stated trigger:
(1) **per-crate SwiftPM targets** with `DayPieces` as an umbrella — worth doing at three or more
contributing crates, or at the first collision a rename cannot resolve, and cheap because the graph
is flat (contributing crates' native halves never call each other, so there are no inter-module
edges to compute); (2) **a generated Gradle module for one crate** — only when a crate needs
settings that cannot merge into `:app` (its own `minSdk`, a compiler plugin, consumer ProGuard
rules, a different JVM toolchain), and never as a wholesale conversion. Module splitting would not
fix `@objc` class-name collisions, which live in a process-global namespace; that stays a naming
rule, and is why [§15.6](#156-daybridge-foreign-language-implementations-of-a-rust-api) derives every bridge name mechanically.

### §15.3 dayffi: the C ABI (superseded — never built)

> [!WARNING]
> **Status: superseded.** The design specified a versioned C ABI (`DayValue` tagged trees, a
> `DayPieceVTable` with sync/async commands, `day_host_emit`, generated per-platform
> registrants) so native-language halves could implement pieces behind a stable boundary. None
> of it was needed: the shipped extension crates pair **Rust renderers** (tier 1, `adopt`ing
> native views created by their own shims) with **staged native sources**
> (`[package.metadata.day.*]`, [§15.2](#152-package-layout-and-aggregation)), and the open event channel shipped as the primitive
> `Event::Custom { tag, num, text }` — one string and one number cover every real piece so far
> (webview URLs, picked dates, media positions), with no cross-language value-tree management.
>
> What survives of the design in practice: `Toolkit::adopt` (foreign native handles enter the
> tree and are framed/measured/snapshotted like built-ins, with the ownership rules the design
> spelled out — retained ObjC objects, JNI globals promoted before Rust sees them, ref-sunk
> GObjects, parentless QWidgets), and the threading rule that native callbacks re-enter through
> the main-loop post. If a future piece genuinely needs rich structured payloads or
> out-of-process native logic, the dayffi design remains in this file's git history
> (pre-2026-07 revisions) as the starting point.

Reference examples of the shipped mechanisms are in [Appendix B](#appendix-b--extension-examples): **ComboBox** (tier 1 — one native
control per toolkit), **Battery** (a part: headless, per-OS halves), **WebView** (commands +
events over the shipped channel), **Lottie** (bridging famous native libraries via
`[package.metadata.day.*]`).

### §15.4 day-lite: the dynamic-language extension surface

> [!NOTE]
> **Status: new (2026-07).** Normative doc: [docs/lite.md](docs/lite.md).

Where §15.1–§15.2 extend day with *compiled* Rust crates, `day-lite` extends it with
*interpreted* apps: JS/TS **miniapps** (W3C MiniApp-shaped packages served from any git
repo/static host) run inside a compiled **superapp** and drive real pieces through a
`dyn-registry` feature in day-pieces — a runtime registry of piece constructors and
`Decorate` modifiers keyed by name, which any compiled-in extension crate joins via the same
registration macros (so a superapp's custom pieces are scriptable automatically). JS signals
are day-reactive `Signal`s (one reactive system across both languages), parts are exposed as
permission-gated bridge modules, and sqlite + a sandboxed filesystem are built in. day-lite's runner runs
a miniapp's own headless tests against day-mock. The reference superapp embedding (catalog,
install/update, permission disclosure) lived in `apps/daylite` and was removed from this
repository in 2026-08; the runtime crate remains, with no in-repo app building against it.

### §15.5 External toolkits (Stage 0 — experimental)

> [!NOTE]
> **Status: Stage 0 shipped (2026-08).** The registration seam only: a crate OUTSIDE this
> repository declares a platform-toolkit pair in `[package.metadata.day.toolkit]`, and the CLI
> resolves `-p <name>` against builtin ∪ declared ([docs/extending.md](docs/extending.md) "External toolkits"). A
> declared target inherits the desktop pipeline — build, launch, log streaming, dayscript,
> sessions, a `day doctor` probe, the `day metadata` catalog entry (`external: true`) — and is
> refused by `day pack` and `day new` with explicit errors. The app-side entry is
> `day::launch_external`, the cfg-free launcher that starts the dayscript engine exactly as the
> feature-gated launchers do. The toolkit SPI itself (day-spec's `Toolkit`/`Platform`, `Event`,
> `Cap`, the props structs) stays UNSTABLE and unpublished: an external toolkit pins the day
> crates to a git revision. Publishing the SPI crates, a conformance kit, and pack hooks are
> Stage 1, deferred to SPI stabilization — the dayffi record (§15.3) is the cautionary precedent
> for freezing an extension boundary early.

### §15.6 daybridge: foreign-language implementations of a Rust API

> [!NOTE]
> **Status: v1 in the tree through phase 7 (2026-08).** Every arm language ships — Swift, Kotlin,
> Java, ArkTS, JavaScript, C, C++ — with `parts/day-part-speech` as the reference crate and a
> Showcase demo driven by a dayscript walkthrough on each target. [docs/bridge.md](docs/bridge.md) is the normative
> contract (type table, ownership rule, threading rule, name derivation) and remains the place to
> read before writing an arm; this section is the architecture-level view. What is left is
> migrating the remaining synchronous parts and the CI gates (phases 8–9). **v1 bridges
> synchronous functions only**: callbacks, futures, and streams are sketched in [docs/bridge.md](docs/bridge.md)'s
> "After v1" but deliberately unbuilt, and line-number remapping is best-effort per language
> (Swift, C/C++, JS, and ArkTS have it; Kotlin and Java do not, so long arms there stay in their
> own files).

**The problem, measured.** Eleven parts (battery, clipboard, deviceinfo, haptics, http,
local-notify, location, network, permissions, prefs, sensors) each carry an Android shim, and every
one of them is Java-only — because Apple, Windows, Linux, and HarmonyOS have been reachable through
`objc2` and `#[link]`, while Android's platform APIs are not reachable at all. Their *web* halves
exist too, but not in the crates: `day_dom_sensor_*`, `day_dom_fs_start`, `day_http_start`, and
`day_location_fix` all live in the CLI's centralized `resources/web/shim.js`, because there is no
per-crate staging for JavaScript. And where a value has to cross, each crate invents its own wire
format — day-part-battery packs a level and a state into one `i64` as `(state << 8) | levelByte`,
written twice, in two languages, agreed by comment.

**The design.** A bridge makes the **Rust signature the contract** and each foreign arm an
*implementation* of it, selected per target at build time. Calling code sees an ordinary function
with no platform conditionals. Arms are written inline in the crate's `.rs` (raw strings, because
foreign code must still lex as Rust tokens inside a macro body and idiomatic JavaScript and ArkTS
do not) or in their own files past about 25 lines, staged the way `[package.metadata.day.*]`
directories are staged today.

**Why this is not dayffi.** §15.3 designed a general C ABI — `DayValue` tagged trees, a
`DayPieceVTable`, generated registrants — and it was never needed, because "one string and one
number" covered every real case. daybridge starts from that evidence: it bridges *functions*, not
objects; its v1 type table is scalars, `&str`, owned `String`/`Vec<u8>`, and POD structs of those;
`Option` does not cross; calls are synchronous, so an `Ok` means the platform accepted the request
rather than finished it; and there is no cross-language value tree and no out-of-process host. What
it inherits from dayffi is the rule that survived: **every foreign→Rust re-entry goes through the
main-loop post** (`day_reactive::on_main`), because Day's UI is single-threaded and signals are not
`Send` — a rule v1 needs only for the callback tier that follows it, and states now so that tier
cannot be built without it.

**Lowering, and why [§5.1](#51-authoring-surface-functions-and-builders-no-macros) still holds.** A crate's bridge sections sit inside one
`day_bridge::bridge!` block — a `macro_rules!` that **discards its body** and expands to an
`include!` of generated code in `OUT_DIR`. There is no procedural macro: the real parser is
day-build reading the crate's own source text, exactly as `day-build/src/swiftui.rs` already parses
`.swift` to generate `crate::swiftui::*`. So the framework still requires no macro anywhere, and a
crate that uses no bridge sees nothing.

**Two generators, no new host machinery.** day-build (in `build.rs`, on any host, with no foreign
toolchain) emits the Rust externs, wrappers, and the C/C++ translation units cargo compiles, so
plain `cargo check` and day-mock keep working. `day build` emits each arm's adapter into the
project that target already builds from — parsing the crate's own sources with the same parser, not
reading build output, since a prepass has to finish before cargo runs: the generated `DayPieces` SwiftPM module, a Gradle `srcDirs` entry, an
hvigor ArkTS module with its `Index.ets`, a per-crate ES module the day-dom shim imports, or a `cc`
translation unit. Contributions ride the existing `day-pieces.json` aggregation rather than a
second manifest. Build-graph facts — Gradle coordinates, permissions, frameworks, `pkg-config`
names, deployment floors — stay in `[package.metadata.day.*]`, where the CLI already reads them.

**Android takes Java or Kotlin.** Both generate the same `Day<CrateCamel>Bridge` class and the
same JNI binding; the difference is that AGP compiles `.java` from any source directory, while a
`.kt` needs the project to have a Kotlin plugin. A `.kt` arm in a project without one is skipped
silently and dies at the first call with `ClassNotFoundException`, so `day build` and `day lint`
refuse that combination and name the fix. The shipped parts use Java, which cannot make an
assumption about someone else's Gradle build.

**Reporting.** Each bridged function answers a `Support` per target, and `docs/bridge-matrix.md` is
generated from the declarations and CI-gated for drift alongside the duty, coverage, and recorder
matrices ([§8.1](#81-the-toolkit-trait), [§8.2](#82-the-open-renderer-registry)). A crate with no `other` arm fails `day lint`, because it could not
compile under the mock toolkit.

`parts/day-part-speech` is the reference crate: one file, six arms (Swift, Java, ArkTS,
JavaScript, C++, C) over each platform's text-to-speech API, plus the Rust fallback.

### §16.1 Design goals

For humans: colorful, animated, cancellable, self-explanatory. For machines (CI, IDEs, AI agents):
deterministic, non-interactive on demand, JSON-structured, stable exit codes, discoverable
(`day --help` is complete; every command supports `--help`, and `day help --json` dumps the whole
command tree with flags and descriptions for agent consumption).

### §16.2 Crate choices

> [!IMPORTANT]
> **Status: shipped leaner.** The CLI kept the small set — `clap` v4 (derive), `anstream` for
> terminal color, `inquire` for the interactive `day new`, `serde` + **`serde_norway`** for
> YAML (DP-14's resolution held) — and skipped the rest of the designed stack: no `indicatif`,
> `miette`, `tracing`, or `tokio`; progress is plain line output, errors are typed enums with
> exit codes (usage 2, build 4, script/assertion 5, signing 6), and processes are
> `std::process` with a signal module that tears down launched sessions and their log pipes
> (`day stop`; Ctrl-C kills the process group). The designed per-OS Job-Object/process-group
> cancellation spec and error-code/diagnostic framework are kept in this file's history as the
> shape to grow into if the CLI's surface demands it.

### §16.3 Global contract (every subcommand)

> [!IMPORTANT]
> **Status: shipped smaller.** The global flags are `--project <dir>` (nearest-ancestor
> `Day.toml` default), `--format {plain,json}` (NDJSON result events), and `--verbose` (forward
> every sub-command's raw stdout/stderr to the terminal instead of capturing it — cargo/gradle/
> xcodebuild/hvigor/adb/codesign/…, so a build/launch/pack shows the full underlying log;
> `DAY_VERBOSE=1` in the environment is the same switch, which is how CI turns a whole job
> verbose — [docs/environment.md](docs/environment.md));
> `--no-input` exists where prompting exists (`day new`, `day app`). `--yes`/`--color`/`-v`
> (the short alias)/`--log-file` and the full event vocabulary below were not built — the `result`
> event and stable exit codes were, and `day metadata --json` / `day help` cover machine
> discovery. The design below remains the target shape for a future `day daemon`.

```
--project <dir>          # default: nearest ancestor with Day.toml
--format {plain,json}    # json = NDJSON result events on stdout
--verbose                # forward every sub-command's raw output to the terminal (unfiltered);
                         # DAY_VERBOSE=1 in the environment is the same switch
--no-input               # never prompt (new/app); missing required input = error
```

JSON event stream (machine mode). The protocol is versioned and hardened: the first event is
always `hello` (flutter daemon's `daemon.connected` precedent), `proto` bumps only on breaking
changes; raw subprocess output is **wrapped** as bounded `log` events (raw xcodebuild/gradle bytes
on stdout would corrupt the stream; full raw output goes to `--log-file`); a terminal `result`
event is **guaranteed on every exit path**, including cancellation; multi-target commands carry
per-target entries and the process exit code is the highest-severity per-target code:

```json
{"event":"hello","proto":1,"day":"0.1.0","pid":48231}
{"event":"task.start","id":"t3","target":"android-mdc","label":"gradle :app:assembleDebug","parent":"t1"}
{"event":"log","task":"t3","stream":"stdout","line":"> Task :app:compileDebugKotlin"}
{"event":"task.progress","id":"t3","detail":"compileDebugKotlin","fraction":0.61}
{"event":"task.done","id":"t3","ok":true,"seconds":24.1}
{"event":"diagnostic","severity":"warning","code":"day::lint::missing_translation","message":"…","path":"locales/fr/app.ftl"}
{"event":"result","command":"build","ok":true,"targets":[{"target":"ios-uikit","ok":true,"code":0,"artifacts":[{"path":"build/day/ios-uikit/Showcase.app"}]}]}
```

A `build` result entry for a **desktop** target also carries a `launch` object — `program`, `args`,
`cwd`, `env`, and a `wrapper` argv when the host needs one (`xvfb-run`) — describing exactly how
`day launch` would start that binary:

```json
{"target":"linux-gtk","ok":true,"code":0,"artifacts":[{"path":"build/day/cargo/linux-gtk/debug/debug/showcase"}],
 "launch":{"program":"build/day/cargo/linux-gtk/debug/debug/showcase","args":[],"cwd":".",
           "env":{"DAY_IMAGE_ROOT":"resource/images","GSK_RENDERER":"cairo"},"wrapper":null}}
```

This is what lets a caller that starts the binary **itself** get the app Day would have started —
the VS Code extension hands it to lldb for a real debug session ([day-vscode](https://github.com/daybrite/day-vscode)).
`ops::desktop_launch_plan` is the single producer: `day launch` spawns the plan and this event
reports it, so the two cannot drift. A `.app` bundle reports the binary **inside** it (a debugger
needs a Mach-O), and device or browser runtimes carry no `launch` at all.

Exit codes: `0` ok · `1` failure · `2` usage · `3` environment/toolchain (doctor-able) · `4` build
failure · `5` script/assertion failure · `6` signing failure · `10` lint findings (with
`--strict`) · `130` cancelled.

### §16.4 Architecture (from flutter_tools, translated to Rust)

> [!IMPORTANT]
> **Status: the ideas shipped; the framework didn't.** The CLI is a plain clap command tree
> with per-target modules — no `CliContext` DI, no `DayCommand` envelope, no daemon. What
> survived from flutter_tools is what mattered: the **doctor workflows**, the **plumbing
> tier** (`xcode-backend`/`gradle-backend` callbacks, [§17.4](#174-the-build-callback-flutters-pattern-exactly--including-the-details-flutter-learned-the-slow-way)), and failure translation where it
> counts (gradle/xcodebuild error surfacing). The designed structure below is kept as the shape
> to grow into if the CLI's complexity ever demands it.

- **Service context, not globals:** a `CliContext` bundling `FileSystem`, `ProcessRunner`, `Env`,
  `Clock`, `Terminal`, `Console` traits — injected into commands, faked in tests
  (flutter's Zone-DI, done Rust-idiomatically as a struct of `Arc<dyn Trait>`).
- **Command envelope:** each subcommand is a struct implementing
  `DayCommand { fn validate(&self, cx) -> Result<()>; async fn run(&self, cx) -> Result<Outcome> }`
  with shared pre-flight (project discovery, Day.toml parse, doctor-lite checks relevant to the
  command).
- **Workflows/doctor:** per-target `Workflow` objects (`applicable? functional? missing?`) power
  both `day doctor` and actionable failures ("`android-mdc` needs: ANDROID_HOME, JDK 17+ —
  none found; see day doctor"). This bakes in the toolchain knowledge
  this workspace accumulated (JDK-26/Robolectric-class problems, rustup-vs-homebrew Rust for cross-std,
  cargo-ndk, `aarch64-apple-ios-sim` on Apple Silicon).
- **Plumbing tier:** stable, documented, hidden-from-default-help subcommands invoked by build
  systems: the arg-less `day xcode-backend build` / `day gradle-backend build` entrypoints
  (called by the Xcode Run-Script phase and the Gradle task, reading their parameters from the
  build system's environment — [§17.4](#174-the-build-callback-flutters-pattern-exactly--including-the-details-flutter-learned-the-slow-way)). Porcelain may change UX; plumbing changes are
  semver-relevant.
- **`day daemon --machine`** (roadmap, post-MVP): long-lived JSON-RPC for IDEs, mirroring
  flutter's daemon; the NDJSON event schema of [§16.3](#163-global-contract-every-subcommand) is designed to be reused by it.

### §16.5 Subcommands

> [!IMPORTANT]
> **Status: shipped, with a different final roster.** Of the designed set, `new`, `build`,
> `sign`, `launch`, `pack`, `lint`, and `doctor` shipped; `day script` became `--script` on
> launch plus **`day drive`**; `day clean` and `day config` were not built (machine-local
> settings ride `day doctor`'s guidance + environment variables, [docs/environment.md](docs/environment.md)). The
> shipped roster (`day --help` is the authority):

| command | what it does |
|---|---|
| `day version` | version, build profile, git ref — the tag or branch when there is one, and **always the commit** (`0.3.0 (release, branch main, bd026ff7)`), so a build can be told from another build of the same branch. Omitted entirely off a git checkout, which is what a crates.io build looks like |
| `day new` | scaffold an app, a **piece**, or a **part** (interactive when bare; `--no-input` for CI; `--describe` prints the question set as JSON for a GUI to render). An app scaffold includes `website/` (site.toml + theme.css — the daysite/GitHub Pages config); `--no-website` omits it; `--locales "en fr …"` scaffolds the app pre-localized, applying each tag beyond `en` through the same code path as `day localize add`; `--day-version <main\|x.y.z\|latest\|branch\|commit>` pins the scaffold's `day` dependencies to that version (a git tag/branch/rev, or the crates.io version with `--registry`) instead of the remote's default branch |
| `day build -p <target>…` | build for one or more targets, in parallel |
| `day launch -p <target>… [--locale …] [--env K=V]… [--script <file>]… [--variant name] [--themes t,…] [--locales l,…] [--keep-alive] [--detach] [--skip-build] [--ios-device <name\|udid>] [--ios-simulator <name\|udid>] [--android-device <serial>] [--ohos-device <key>]` | build + install + run + stream logs; scripts imply detach and exit 5 on assertion failure; `--skip-build` reuses the previous build's artifact (recorded per target×profile) — CI's capture loops build once and launch per variant; device selection is one flag per runtime, so a single launch can name a different one for each `-p`: `--ios-device` a physical iPhone/iPad, `--ios-simulator` (alias `--device`) one booted simulator instead of every booted one; `--detach` (alias `--detached`) exits after launch and leaves the apps running, so nothing of `day`'s is left to Ctrl-C and `day stop` is what ends them, `--android-device` an adb serial, `--ohos-device` an hdc connect key. A named device is also what the run's dayscript port forward and screenshots address, rather than whichever device enumerated first. `--ios-device` also changes the BUILD — the `iphoneos` SDK, and signing against the provisioning profile installed for that app id, with the identity and entitlements taken from the profile itself; installer chatter from adb/devicectl is captured rather than streamed so every target narrates through the same `Installing`/`Launching` lines and the app's own output carries the same `[target]` prefix; `-p` resolves builtin targets first, then pairs declared by dependency crates' `[package.metadata.day.toolkit]` ([§15.5](#155-external-toolkits-stage-0--experimental)); `--themes`/`--locales` expand a scripted launch into the capture matrix (build once, one run per theme×locale, the gallery/app variant-naming conventions, the iOS app-death retry, and linux headless plumbing all internal) — the loops both CI workflows used to carry |
| `day pack -p <target> [--profile release] [--formats <list>] [--no-version-in-name] [--artifact-name <stem>]` | build → sign → installable artifact (formats and naming below) |
| `day rebuild <artifact> [--strict] [--keep] [--force-tool <name>] [--from-dir <dir>]` | rebuild a shipped artifact from its own provenance (the SBOM + `.buildinfo` sidecars) and report the payload/container verdicts ([§20.3](#203-reproducible-build-verification)); `--from-dir <dir>` rebuilds from that project directory instead of cloning the recorded commit — for artifacts whose source is not in git, e.g. CI's freshly scaffolded project — with tool gating still applied from the sidecar |
| `day sign` | signing utilities; `--check` validates `Day.toml [signing]` without printing secrets; `--notarize-status <id>` |
| `day doctor` | per-toolkit environment diagnosis with fixes |
| `day checkup [-p <target>…] [--day-version <spec>] [--profile …] [--no-pack] [--strict] [--dir <d>] [--keep]` | end-to-end check of THIS machine: `day doctor` (fail-fast), then per combo scaffold a throwaway app, build it, and pack it — reporting each combo's build time and packaged artifact size. No `-p` checks every combo this host can build with what is installed (a missing prerequisite is a reported SKIP); naming combos asserts they work here, so a missing prerequisite is an error. `--strict` fails on any combo this host could have checked but is not set up for. `--day-version <main\|x.y.z\|latest\|branch\|commit>` names the day under test: checkup installs THAT day-cli and pins the app it scaffolds to the same one — what the scheduled `checkup.yml` crosses with its combo matrix ([§20](#20-continuous-integration)) |
| `day app` | grow an existing app's platform support: `add-toolkit <target>…` appends new targets to Day.toml and materializes their host projects (`platform/…`, plus the `store/` listing skeleton when the first store target arrives); on an already-declared target it materializes whatever scaffold files are missing, never overwriting — how an older app adopts a host project the template gained later (e.g. `platform/macos/`). `split-xcconfig` migrates pre-split Xcode projects to the `DayApp.xcconfig` layout (§17.4) without building — `day build` runs the same migration automatically |
| `day metadata [--json]` | machine-readable project metadata (versioned, grow-only envelope — IDE tooling consumes this, never Day.toml directly) |
| `day lint` | fluent coverage (missing/unused/unknown keys), duplicate element ids, unknown navigation routes (including `[[shortcuts]]` routes), shortcut-label coverage, permission declaration/manifest drift ([docs/permissions.md](docs/permissions.md)), store-listing rules ([docs/store.md](docs/store.md)), Day.toml schema — fast, source-level  Findings carry `file:line:column` and a severity; `--json` emits them as a versioned envelope with the fix a rule proposes, and `--fix` applies those fixes  Under GitHub Actions (`GITHUB_ACTIONS=true`) findings also emit `::warning::`/`::error::` annotations on stdout, anchored to their line, and a markdown table into `$GITHUB_STEP_SUMMARY` |
| `day patch [--local <checkout>] [--check]` | build a standalone app against a LOCAL day checkout: writes the machine-local `.cargo/config.toml` `[patch]` table, and `--check` fails when any day crate still resolves from git — the guard against a stale table silently mixing a local framework with a published one |
| `day store <init\|stage>` | the App Store / Google Play listing: `init` writes `store/<locale>/` skeletons for every locale the app ships, `stage` generates the fastlane trees a release uploads ([docs/store.md](docs/store.md)) |
| `day localize <list\|add\|remove>` | the project's locale surfaces — `resource/locales/`, `store/`, the iOS `knownRegions`, `website/site.toml`'s `locales` array — surveyed (`list`, with drift warnings; `day lint` reports the same findings) or edited together (`add`/`remove` a Day BCP-47 tag on every surface the project has; per-store and Xcode spellings remain a generation-time concern) |
| `day screenshot index` | merge capture trees (`--screenshot-paths`, default `build/day/screenshots`) into `gallery.json` — the published machine-readable screenshot index: URL, localized title/caption from the dayscript metadata (§14.7), theme, locale, platform, dimensions, byte size, sha-256. App sites serve it at `/gallery/gallery.json`; `--out` places it |
| `day web driver` | print the path of the bundled `DAY_WEB_DRIVER` page-driver script (headless Playwright; materialized to a temp location) — `DAY_WEB_DRIVER="node $(day web driver)"` is how CI drives scripted web-dom runs with a driver that always matches the CLI's protocol ([docs/web.md](docs/web.md)) |
| `day stop` / `day relaunch` | stop running launches / stop-rebuild-relaunch ("apply my code changes") |
| `day drive` | execute dayscript steps against a RUNNING app, step-at-a-time ([docs/agent.md](docs/agent.md) — the agent inner loop) |
| `day mcp-server` | serve Day tools to coding agents over the Model Context Protocol (stdio) |
| `day devices list [-p <target>] [--format json]` | what each mobile target can be launched onto right now: booted simulators and attached iPhones, adb devices and emulators, reachable hdc targets — plus shut-down simulators and defined AVDs under `bootable`. Every device names the FLAG that selects it (`--ios-simulator` and `--ios-device` differ per device), so an editor fills a picker without hard-coding that mapping; a target whose toolchain is missing reports `available: false` with a `note` rather than an empty list, so one absent SDK never blanks out the other two. Needs no project; the JSON envelope is schema-versioned and grow-only like `day metadata`. `day devices boot -p <target> <id>` starts one of the `bootable` entries — `simctl boot` + Simulator.app, a detached `emulator -avd`, or the Oniro emulator — which is what makes a picker's "nothing running" one action from a device rather than a dead end (iOS cannot install onto a shut-down simulator) |
| `day ohos` | HarmonyOS helpers (emulator management, …; [docs/harmonyos.md](docs/harmonyos.md)) |
| `day xcode-backend build` / `day gradle-backend build` | hidden plumbing the scaffolds call back into ([§17.4](#174-the-build-callback-flutters-pattern-exactly--including-the-details-flutter-learned-the-slow-way)); the Xcode scaffolds also call `stage-resources` (macOS bundle resources) and `stage-strings` (iOS `[[shortcuts]]` label localizations) |

> [!NOTE]
> `day lite test` ([docs/lite.md](docs/lite.md) §11) is **not** built into the published `day` CLI yet: day-cli must
> not depend on `day-lite`, which stays `publish = false` until it ships on crates.io. The runner core
> lives in `day-lite` (`day_lite::run_tests`); re-add the `Lite` subcommand + the `day-lite` dependency
> to expose it once day-lite is publishable.

#### `day new`

Interactive when run bare (`inquire` prompts: name, id, targets, locales); non-interactive with
flags + `--no-input` for CI/agents. Templates are embedded in the CLI binary; `app`, `piece`,
and `part` scaffolds exist — the latter two produce the [§15](#15-extensibility-pieces-parts-and-tweaks) package shapes with per-toolkit
feature wiring. An app scaffold gets a unique generated icon ([docs/icons.md#generate](docs/icons.md#generate)), seeded
by the app id so the same id always scaffolds the same icon; `--icon-seed` overrides.

Which `day` a scaffold depends on is `--day-version` (2026-08): a release pins the matching
`vX.Y.Z` git tag, `main` or any other branch name pins `branch`, a 7–40 character hex string pins
`rev`, and `latest` asks crates.io for the newest published day-cli first. With `--registry` a
release becomes the crates.io version instead; a branch has no version to ask for, so that pair is
refused rather than silently ignored, as is `--day-version` alongside `--local`. Without the flag
the scaffold takes the remote's default branch, exactly as before. This is what lets one CLI check
several days — `day checkup --day-version` drives both halves through it.

> [!NOTE]
> **`day new --describe` added 2026-08.** The prompts are a terminal conversation, and an editor
> cannot join one — so day-vscode hand-copied the question set and came to offer a
> `windows-winui` target, which does not exist. `--describe` prints that set instead: a
> versioned, grow-only JSON document of every kind's fields — id, label, help, type, options,
> default, validation pattern, and **the flag each one fills** — plus the host's own
> `default_target`. Output is JSON by definition, so it takes no `--format`, the same way
> `day metadata --schema` does.
>
> A caller renders it however it likes and then runs an ordinary
> `day new <kind> <name> --flag …`. A field left blank is omitted rather than passed empty, so
> the CLI applies the default it would have applied anyway — which is what keeps
> `dev.example.<name>` and the title-cased name from being recomputed by every GUI. Branching is
> declarative: the piece's toolkit field carries
> `"visible_when": {"field": "native", "equals": "native"}`.
>
> The fields are generated from the same constants the prompts read and the same target catalog
> `day metadata` publishes — which cannot serve here, since it needs a `Day.toml` and this is the
> one moment before there is one. A test checks every described flag against clap's own
> definition of that subcommand, so a typo fails the build rather than someone's editor.

#### `day build`

Per target: (1) preflight, (2) conveyance generation from `Day.toml` ([§17.5](#175-metadata-conveyance-daytoml--each-build-system)), (3) the target's
pipeline — `xcodebuild` for ios; `gradle` for android; hvigor for ohos; cargo + bundle
assembly for the cargo-driven desktop targets; MSBuild-free cargo + C++/WinRT shim for windows.
**`macos-appkit` builds through the `platform/macos/` Xcode host project, always** (dual-mode
2026-08, single-mode later that month — the bare cargo + bundle-assembly path and its
`DAY_MACOS_XCODE=0` escape hatch were retired, along with the `swift build` prepass that
statically linked macOS Swift contributions into the cargo binary; those now build inside the
same xcodebuild run via the generated DayPieces package,
[§15.2](#152-package-layout-and-aggregation), [docs/swiftui.md](docs/swiftui.md)). The scaffold
ships by default and an app that predates it adopts it with `day app add-toolkit macos-appkit`
— without it, `day build -p macos-appkit` fails with that instruction. The build is a real
`.app`: bundle identity, compiled appiconset, resources staged into `Contents/Resources` by the
`day xcode-backend stage-resources` script phase (host-arch by default; `DAY_MACOS_UNIVERSAL=1`
builds arm64 + x86_64, which needs both Rust stdlibs installed), and `day pack` takes that
bundle as-is — copy, codesign, dmg, notarize — assembling nothing. The
Xcode/Gradle projects **call back** into the arg-less plumbing entrypoints ([§17.4](#174-the-build-callback-flutters-pattern-exactly--including-the-details-flutter-learned-the-slow-way)) for the Rust
staticlib/dylib, so builds started from Xcode/Android Studio are first-class and never stale.
Both Xcode scaffolds keep their user-adjustable build settings (signing, deployment target,
device family) in a committed `DayApp.xcconfig` rather than the pbxproj (2026-08): every build
configuration's `baseConfigurationReference` points at it, and it `#include?`s — LAST, so
Day.toml stays authoritative — a generated `build/day/xcconfig/<platform>.xcconfig` carrying
the Day.toml-derived bundle id, version, and build number. Command-line settings still win, a
fresh checkout builds in the IDE from the committed fallback lines, and `day build` migrates a
pre-split scaffold in place (also available standalone as `day app split-xcconfig`; an
unrecognized pbxproj degrades to a warning, never a half-edit).
Multiple `-p` build in parallel. Results land in `build/day/<target>/…`.

#### `day icon`

`day icon [master] [--check] [-p <target>]` renders every platform's app-icon set from one
master (`resource/icons/icon.svg` layered via `day:` group ids, or a plain svg/png) into the
`resource/icons/` exports plus the committed `platform/` copies, writing `icons.lock.json`.
`--check` re-renders in memory and exits 5 on drift — the CI gate. Engine: `day-vector`
(resvg with text shaping off; `<text>` masters are refused with an outline hint).
`day icon --generate [--seed <int|string>] [--overwrite] [--out <file.svg>]` writes a seeded
pseudo-random layered master instead (`day-vector`'s `icongen`) and regenerates the outputs
from it; `--out` is the project-less preview form (SVG + 512 px PNG at the given path).
`day new app --icon-seed <seed>` overrides the scaffold's default (the app id). Normative:
[docs/icons.md](docs/icons.md).

#### `day sign`

Per-format truth as designed: `.app`/`.dmg` = `codesign` + `notarytool` + `stapler`; `.apk` =
`apksigner`; `.aab` = Gradle signingConfig; ios = App Store Connect API-key signing; windows =
self-signed dev flow. Config in `Day.toml [signing]` with env-var interpolation — an unset
variable degrades that section to the dev tier LOUDLY (ad-hoc / debug keystore / self-signed),
it never fails the pack; `day sign --check` reports readiness without printing any secret.

#### `day launch`

Build (+ sign where the destination requires) + install + run + stream logs, per target, in
parallel: desktop runs the binary/bundle — for a `platform/macos/` app the xcodebuild-built
`.app`'s own executable, exec'd directly so stdio (log streaming, dayscript) stays attached
while macOS resolves the bundle's real identity, Dock icon, and `Contents/Resources` (none of
the bare-binary `DAY_*`/`DAY_APP_ICON` environment applies); ios via `simctl` with
`log stream`; android via `adb install` / `am start` with pid-scoped logcat; ohos via `hdc`. `--locale` moves the whole
app's locale; `--env` passes app environment (on web-dom as page query parameters, read back
through `day::env` — a browser sandbox has no process environment, [docs/web.md](docs/web.md)); each
`--script` runs via the embedded engine
([§14](#14-scripting-dayscript)) — with scripts the command exits when the last one finishes (the CI entry point), and
`--keep-alive` keeps the session drivable via `day drive` afterwards.

#### `day clean`

`day clean [--dry-run]` (2026-08) removes every build artifact a project accumulates and
reports the space reclaimed: `build/` (all `day build`/`launch`/`pack` outputs), cargo's
bare-invocation `target/`, and the platform scaffolds' generated outputs (gradle's `.gradle` +
module `build` dirs, hvigor's caches/modules and the CLI's in-scaffold ArkTS/resource staging,
SwiftPM's local `swiftui/.build`). The list is the scaffold `.gitignore`s made executable —
source, IDE state, and machine-local config (`local.properties`, `.cargo/config.toml`) stay.
Recorded sessions are stopped first, the `day stop --all` teardown. `--dry-run` lists what
would go, with sizes. The day-vscode extension's "Clean Project" action calls this.

#### `day pack`

`day pack -p <target> [--profile release]` = build → sign → **installable artifact**, per
target: `.dmg` (macos-appkit: sign `.app` → `hdiutil` → sign dmg → notarize → staple), `.ipa`
(ios; degrades to an UNSIGNED device `.ipa` — `-unsigned.ipa`, for sideloading via
AltStore/SideStore or the developer's own signing — without App Store Connect signing config;
changed 2026-07 from the original zipped-Simulator-`.app` fallback),
`.apk` + `.aab` (android), **flatpak + AppImage** (linux-gtk/qt), **`.msix` + an NSIS
`setup.exe`** (windows), **`.hap`** (ohos via hvigor).

The two Linux formats are siblings, and the split is where the toolkit comes from. The
**`.flatpak`** takes GTK/Qt from a runtime the user's flatpak installation resolves at install
time (`org.gnome.Platform` / `org.kde.Platform`), which keeps the bundle app-only and Qt's LGPL
obligations satisfied by the runtime's relinkable shared libs; icons are generated at the
freedesktop policy sizes, and the Qt WebEngine BaseApp — which a `base:` copies INTO the bundle at
~87 MB — is named only when the packed binary's `DT_NEEDED` list actually links WebEngine
(2026-07; it was previously added to every Qt bundle). The **`.appimage`** carries its toolkit
inside, so it runs on a machine with nothing installed, which is what a one-line
`curl … | bash` launcher needs (daybrite/actions ships one per release). Day stages the AppDir
and delegates the bundling to `linuxdeploy` plus its `gtk`/`qt` plugin: the parts a naive `ldd`
closure misses — GdkPixbuf loaders, GIO modules, GSettings schemas, Qt's platform plugins — are
exactly where a hand-rolled bundler goes wrong. Without the plugin the AppImage still builds and
still runs on a machine that already has the toolkit, and says so loudly (§20). The payload tree
inside both is staged once (`pack/linux.rs`), so one recorded digest set verifies either (§20.3).
GTK/Qt bundling on non-native OSes remains unsupported (the extra combos are dev targets), and
the designed LGPL/licenses-stage guard rails remain future work.

Every format lands on one filename pattern (`pack/naming.rs`):

```text
<stem>[-<version>]-<platform>-<toolkit>[-<extra>].<ext>
  day-showcase-macos-appkit.dmg          day-showcase-windows-xaml-setup.exe
  day-showcase-android-mdc.aab           day-showcase-linux-gtk-x86_64.flatpak
  day-showcase-linux-gtk-x86_64.appimage
```

`<stem>` is `day pack --artifact-name`, else `Day.toml` `[app] artifact` (overridable per target
like any `[app]` property, [§17.3](#173-daytoml)), else a slug of `title` — always slugged to
lowercase `[a-z0-9-]`, because GitHub rewrites a space in an uploaded asset name to a dot.
`<extra>` distinguishes artifacts that would otherwise collide: `setup` for the NSIS installer,
the CPU arch for a flatpak or an AppImage, `unsigned` for an `.ipa` packed without signing
material.
`--no-version-in-name` drops the version infix so a `releases/latest/download/<name>` URL stays
stable across releases; daybrite/actions' release job packs with that flag
([§20](#20-continuous-integration)).

The target combo is written by the CLI rather than spliced in by release CI. That is what lets
`day rebuild <downloaded-asset>` find its own rebuild (it looks for a file of the same name), and
what lets the provenance sidecars be named after the artifact they describe
([§20.4](#204-provenance-sbom--buildinfo)).

#### `day lint`

Built-in rules only, source-level and fast: fluent coverage (missing/unused/unknown keys across
all locales), duplicate element ids, unknown navigation routes, permission declarations (a
`Permission::X` the app's `[permissions]` table doesn't declare, a missing reason, and drift between
that table and the checked-in iOS `Info.plist` — [docs/permissions.md](docs/permissions.md)), `Day.toml` schema validation.
`day lint` exits nonzero (10) on findings under `--strict`. `--allow <CODE>` lets one finding code
stand: repeatable, `day::lint::` prefix optional, and the finding still reports (as one summary
line per code, with a count and a sample) so a stale `--allow` stays visible. It exists for CI
gates that have to hold a tree to every rule except one known-outstanding class, as
[§20](#20-continuous-integration)'s scaffold check does with `store-placeholder`. The wider designed rule set (a11y labels,
bare literals, scroll nesting, RTL styling) has not been built; `res::str` ([§18.5](#185-typed-resource-constants-docsresourcesmd)) made the
missing-key class a compile error instead.

> [!NOTE]
> **Extended 2026-08** with the three things an editor needs: a **place**, a **severity**, and a
> **repair**.
>
> Every finding that is about something in a file now carries `file:line:column`. Fluent findings
> get theirs from real spans — `fluent-syntax` attaches none, so day-build recovers each one by
> comparing the parsed fragment's address against the source it borrows from
> (`day_build::offset_in` / `ftl_key_offsets` / `FtlCall.offset`). Source-literal rules carry the
> position of the `tr("…")` or `.id("…")` that produced them; manifest rules find their own value
> in `Day.toml`'s text, because the parsed manifest keeps no spans. A finding about something
> ABSENT — a locale with no listing, a key no catalog defines — has no place and reports none.
>
> `severity_of(code)` is the one place the error/warning split lives. A finding is an **error**
> when it names something that does not exist or that will misbehave at runtime (an unknown
> route, target, override, or Fluent function; an undeclared permission; a duplicate id).
> Everything else stays a warning. `unknown-key` is the deliberate exception: it meets the test
> but its evidence is a scan for `tr("`, a two-character name that occurs inside other
> identifiers (`push_str("` reported every SVG tag a file writes), so it reports as a warning —
> an error is held to the standard of the evidence for it. Severity is presentational: `--strict` still fails on ANY
> active finding, so no CI gate changes meaning. GitHub annotations follow it, and now carry the
> file and line, so a finding lands on its own line in the PR diff.
>
> `--json` emits a versioned, grow-only envelope: every finding with its place, its severity, its
> waived flag, and its fix when it has one, plus `counts`. Waived findings are included and
> flagged rather than dropped, so a stale `--allow` is visible to a tool too. The global
> `--format json` selects the same output.
>
> `--fix` applies the repairs and says what it did to each file. A rule proposes one only when the
> repair is **safe** (reversible, inventing no content) and **unambiguous** (exactly one right
> answer) — today `store-whitespace` and `store-bad-keywords`, both whole-file rewrites. A waived
> code is never rewritten: `--allow` says the finding may stand. Because two rules can propose a
> repair for the same file from the same original text, `--fix` applies one per file, re-checks,
> and repeats until nothing is left.

#### `day drive` (replaces the designed `day script`)

Executes dayscript steps against a running app — one step or a JSON list per call, results as
JSON on stdout — which is the shape agents need (act, observe, decide, repeat). See
[docs/agent.md](docs/agent.md); `day launch --script` covers the batch/CI case.

#### `day doctor`

Shipped as designed: per-toolkit workflows (`applicable? functional? missing?`) power both the
report and actionable failures; `day doctor --json` for agents. The toolchain knowledge lives
in `day-toolchain`, shared with the build scripts. Each probe declares what its absence blocks —
build, packaging, or launch — and only a BUILD miss is ever an error; `day checkup` reads the same
classification to decide what it can build and what it can package.

#### `day checkup`

> [!NOTE]
> **Added 2026-08.** `day checkup` moved the scheduled install workflow's YAML — focused doctor,
> `day new`, `day build` — into the CLI, and added the packaging step and `--day-version`.
> `checkup.yml` is now one step per cell of a combo × day-version matrix
> ([§20](#20-continuous-integration)).

`day checkup [-p <target>…]` answers "can this machine take a user from `day new` to a shippable
artifact?" for each platform-toolkit combo. It runs the doctor probes first and stops if they fail,
then per combo: scaffold a throwaway app into a temporary directory, `day build` it, and `day pack`
it — reporting the build time and the packaged artifact's size for each, on the console, in the
`--format json` result event, and (under `GITHUB_ACTIONS`) as a table in the job summary. Needs no
project: it makes the projects it checks, and removes them unless `--keep`.

The three steps run as sub-processes of a day CLI — this binary, or the one `--day-version`
installed — the way `day rebuild` re-invokes `day pack`: what is under test is the user-facing
commands, so that is what runs. One scaffold per combo, not one shared
multi-target project — the single-target scaffold path (`template::filter_for_targets`) is the one
that broke silently when `harmony-arkui` was renamed.

With no `-p`, checkup takes every combo this host can build whose BUILD prerequisites are present
and reports the rest as skips carrying doctor's own fix lines (experimental targets stay out unless
named). With `-p`, the caller asserts the combos work here: their toolkits are checked in FOCUSED
doctor mode, so a missing prerequisite is an error before anything is scaffolded. A missing
PACKAGING tool is never a hard error — doctor treats those as warnings — so the pack stage is
skipped with its reason instead, and `--strict` is what fails on it.

**Which day is under test** is `--day-version`: `main` (or any branch), `0.2.0` (or any release),
`latest` (the newest day-cli on crates.io, resolved once at the start so a release landing mid-run
cannot split it), or a commit. It decides BOTH halves — the day-cli that runs new/build/pack, and
the `day` dependency the scaffold carries — because testing one against the other measures nothing.
The CLI is `cargo install`ed into the run's scratch directory (skipped when the running binary is
already that version, which is what makes the `latest` cells cheap); the scaffold is pinned through
`day new --day-version`, as a git tag for a release and a branch/rev otherwise, since the framework
crates are not on crates.io yet. A CLI that predates `day new --day-version` cannot pin its
scaffold, so checkup refuses it by name rather than building against the remote's default branch
and reporting the result as that release. Omitting the flag checks the running binary with
`day new`'s own defaults.

One asymmetry to keep in mind when reading a `--day-version` run: the *driver* is the CLI you
invoked, so the doctor probes and the target catalog are ITS — a combo or a prerequisite that only
the day under test knows about is not part of the selection. What the day under test supplies is
the scaffold, the build, and the pack. For the scheduled `main` cells that means a brand-new target
is checked once the driver (the published release) knows it too.

`--strict` fails on the skips this machine could have prevented — a missing build prerequisite, a
missing packaging tool — and reports the combo ones before building rather than after, since no
amount of building changes the verdict. A combo that builds on another OS, or an experimental one
nobody named, is out by definition and never turns a strict run red; counting those would make
`--strict` impossible to pass anywhere. A scheduled job needs the flag: a prerequisite that stopped
installing would otherwise shorten the check and still report success. Selecting nothing at all is
an error with or without it. Exit codes: 2 usage, 3 environment, 4 a build or pack failed.

### §16.6–16.8 (reserved: command reference details live in Appendix D and `day help`)

### §16.9 The inner loop (no hot reload — the honest story)

Rust has no VM; Day does not pretend. The inner loop is `day relaunch` — stop, incremental
cargo rebuild, relaunch — plus optional `--script` replay to restore UI state (a dayscript that
navigates back to where you were: pillar 3 earning its keep), and `day drive` for
state-preserving pokes at a `--keep-alive` session. Desktop relaunch is seconds. Roadmap
(still out): dylib hot-swapping of the app crate behind a stable `day-core` boundary (the
build-once model helps — rebuilt constructors, preserved signals); a research item, not a
promise.

---

## §17 The Conventional Day Project and `Day.toml`

### §17.1 Project layout (`day new` output)

> [!IMPORTANT]
> **Status: shipped differently.** The real scaffold (below) differs from the design sketch:
> resources live under one `resource/` tree ([§18.3](#183-processed-images--random-access-data-resources-docsresourcesmd)), scripts under `dayscript/`, and the
> starter is a small multi-page app rather than a bare root. `AGENTS.md` ships in every
> scaffold — agent-readable project instructions are a first-class output. Platform scaffolds
> appear only for the toolkits the app declares.

```
fieldnotes/
  Day.toml
  Cargo.toml                 # normal cargo project; `cargo build`/`test`/`clippy` work standalone
  build.rs                   # day_build::generate_resources() → typed res:: constants (§18.5)
  README.md
  AGENTS.md                  # instructions for coding agents (day drive, day mcp-server, conventions)
  .gitignore
  .vscode/extensions.json    # recommends the Day VS Code extension (docs/vscode.md)
  src/
    lib.rs                   # routes! + root() (the app)
    main.rs                  # desktop entry: day::launch
    model.rs                 # the domain object + its prefs-backed store
    pages/                   # starter pages: welcome, navigate, settings
  resource/
    locales/en/app.ftl       # + one dir per locale
    vectors/                 # tab + row glyphs, and app_mark.svg (a copy of the generated icon)
    images/app_logo.png      # processed images (§18.3); assets/ and fonts/ join as needed
  dayscript/
    demo.yaml                # starter walkthrough; real apps grow it further
  platform/                  # only for toolkits with a native host project:
    ios/                     #   DayApp.xcodeproj + Runner (day root in a view controller),
                             #   Run-Script phase calling `day xcode-backend build` (§17.4);
                             #   DayApp.xcconfig holds the user-adjustable build settings
    macos/                   #   DayApp.xcodeproj + Runner (thin main.swift → day_main),
                             #   same callback phase + `day xcode-backend stage-resources`;
                             #   builds a real .app (debugger/Instruments-ready; §16.5 day build);
                             #   DayApp.xcconfig, as on ios
    android/                 #   Gradle project; committed build files read the generated
                             #   build/day/android/*.json|properties generically (§17.5)
    harmony/                 #   hvigor project (docs/harmonyos.md; pre-rename scaffolds'
                             #   platform/ohos/ is still read, with a rename hint)
```

Rust code layout: the app is a **lib crate** (`fieldnotes`) so mobile targets (which need
`cdylib`/`staticlib` + platform entry glue) and the desktop `main.rs` share everything. The mobile
entry glue (`#[no_mangle] JNI_OnLoad`-adjacent start fn for android; the UIKit `main` shim for ios)
is generated into the scaffolds, calling `day::launch_with(Options::from_env(), fieldnotes::root)`.

### §17.2 Why real platform projects (and not pane's hand-assembly)

pane proved the hand-assembled path (`aapt2`+`d8`+`zip` APKs, hand-written `Info.plist` bundles) —
excellent as a fast CI signal, structurally incapable of: native transitive dependencies (a
Lottie AAR, an SPM package — [§15](#15-extensibility-pieces-parts-and-tweaks)'s whole point), store submission (entitlements, provisioning,
Play/App Store toolchains), and IDE escape hatches. Day therefore adopts the Flutter/Skip position
from day one: **checked-in, template-generated, thin platform projects that remain buildable by
their native tools**, with the callback hook keeping Rust fresh. The framework repo keeps a
pane-style hand-assembly harness *only* as an internal CI signal for backend development (it's cheap
and hermetic), never as the product path — this is the "no cheating" resolution of the two models.

### §17.3 `Day.toml`

> [!IMPORTANT]
> **Status: shipped with a smaller schema.** The shipped manifest keeps the principles below;
> the concrete sections in real projects are `schema`, `[app]` (id, title, artifact, build,
> targets — any property overridable per platform/toolkit/target), `[window]` (width/height/min sizes),
> `[signing.*]` (env-var interpolated, degrade-loudly), and — added 2026-07 —
> `[permissions]`, which declares the OS permissions the app uses and the reason each prompt shows;
> `day build` turns it into every platform's manifest entry ([docs/permissions.md](docs/permissions.md)); and — added
> 2026-08 — `[[shortcuts]]`, launcher shortcuts as saved deep links (a route plus a Fluent label
> id, resolved per locale at build and conveyed into each platform's native declaration —
> [docs/deep-links.md](docs/deep-links.md)). Locales, images,
> assets, and fonts
> are **convention, not configuration** — the `resource/` tree is scanned ([§18](#18-resources-icons-and-theming)). The extended
> schema sketched below (`[localization]`, `[assets]`, `[icons]`, `[scripting]`, `[lint]`,
> per-OS tables) was not needed; `day metadata --json` is the tooling contract either way.

The manifest is TOML (the Tauri / Dioxus model): a dedicated file that doubles as the project
marker. `name` and `version` are DERIVED from `Cargo.toml`'s `[package]` — never restated.
Any `[app]` property can be overridden per platform (`[app.ios]`), per toolkit (`[app.qt]`),
or per target (`[app.macos-appkit]`); the most specific table wins when the build derives
platform metadata (Info.plist, AndroidManifest label/applicationId, …).

```toml
schema = 1                          # manifest schema version
# scaffold = 1                      # platform-scaffold version stamped by `day new`; `day build`/
                                    #   `doctor` verify it against the CLI's supported range and fail
                                    #   with instructions on mismatch (Flutter needed 30+ migrators for
                                    #   exactly this; an idempotent `day upgrade` running per-file
                                    #   migrators is committed for M9; "delete platform/ and re-create"
                                    #   is explicitly rejected)

[app]
id = "dev.example.fieldnotes"       # bundle id / application id / app id
title = "app-title"                 # Fluent key → localized display name (falls back to name)
artifact = "fieldnotes"             # filename stem for packages: fieldnotes-macos-appkit.dmg
                                    #   (default: a slug of title; see §16.5's `day pack`)
scheme = "fieldnotes"               # deep-link URI scheme: fieldnotes://<route> (docs/deep-links.md)
                                    #   (default: the id's last segment. Declared only where an
                                    #   app must keep a scheme it already published — a scheme is
                                    #   a contract, so it is never re-derived under a shipped app)
build = 42                          # CFBundleVersion / versionCode (int, monotonic)
targets = ["macos-appkit", "macos-gtk", "macos-qt", "ios-uikit", "android-mdc"]

[app.ios]                           # per-platform/toolkit/target overrides of any [app] property
title = "Fieldnotes Mobile"

[localization]
default = "en"
locales = ["en", "fr"]
dir = "locales"

[assets]
dirs = ["assets/"]                  # recursively packaged (§18)

[icons]
source = "icons/app.svg"

[scripting]
release = false                     # embed dayscript engine in release builds?

[lint]
allow = ["bare-text"]               # per-rule opt-outs (discouraged)

[ios]
deployment-target = "15.0"
capabilities = []                   # entitlements toggles understood by the generator

[android]
min-sdk = 24
target-sdk = 35                     # edge-to-edge is mandatory at 35 — see §7.7 inset policy

[windows]
app-sdk = "1.6"                     # WinAppSDK runtime pin (§9)

[qt]
license = "lgpl-dynamic"            # or "commercial" — gates `day pack` static/store configurations (§16.5)

[signing.macos]
identity = "${DAY_SIGN_MACOS_IDENTITY}"
notarize = { key-id = "${DAY_NOTARY_KEY_ID}", issuer = "${DAY_NOTARY_ISSUER}", key-path = "${DAY_NOTARY_KEY}", wait = "30m" }

[signing.android]
keystore = "${DAY_ANDROID_KEYSTORE}"
key-alias = "release"
store-pass = "${DAY_KS_PASS}"
key-pass = "${DAY_KEY_PASS}"

[signing.windows]
provider = "trusted-signing"        # §16.5 sign — provider enum

[dependencies]                      # Day Piece packages needing native aggregation (§15.2)
# (cargo deps remain in Cargo.toml; this section only exists for overrides/pins of piece metadata)
```

Principles: **derive, don't restate** — anything expressible in `Cargo.toml` stays in
`Cargo.toml` (`name`/`version` come from `[package]`); **base + overrides** — per-platform
sections are small and closed-schema (unknown keys = lint error, catching typos), and any
`[app]` property may be specialized per platform / toolkit / target; tooling reads the
manifest through `day metadata --json` (a versioned envelope), never by parsing the file.

### §17.4 The build callback (flutter's pattern, exactly — including the details flutter learned the slow way)

- **ios/**: the Runner target's Run-Script phase resolves the CLI, then calls the arg-less
  **`"$DAY_BIN" xcode-backend build`** — plumbing that reads
  `CONFIGURATION`/`ARCHS`/`BUILT_PRODUCTS_DIR`/`PLATFORM_NAME` from
  Xcode's environment (flutter's `xcode_backend.sh` pattern; a fully-parameterized checked-in
  invocation would fossilize flags into user projects). The resolution (2026-08) exists for
  builds started from the Xcode GUI, which run on Xcode's own minimal PATH with no shell
  profile: `DAY_BIN` (exported by `day build`) wins, then `command -v day`, then the standard
  install locations (`~/.cargo/bin`, Homebrew, `/usr/local/bin`), with a named error rather
  than sh's bare `day: command not found`. The scaffolded projects also carry a plain folder
  reference to the app root (never in any build phase), so the Rust sources are browsable and
  editable from inside Xcode. Inside: configuration→cargo-profile
  mapping by case-insensitive substring with a `DAY_BUILD_MODE` override (miette error listing
  accepted names); the space-separated `ARCHS` list is split, **one cargo build per (arch, sdk),
  `lipo`'d together** (a single `--arch "$ARCHS"` is wrong for universal builds); output is the
  linked staticlib (iOS requires one), staged into `$(BUILT_PRODUCTS_DIR)/day/` under the FIXED
  name `libdayapp.a` — Day owns that directory, so the name is Day's, and the app crate's name
  never reaches the Xcode project (renaming an app touches no build setting). The crate-named
  `lib<app>.a` is hard-linked beside it for projects generated before that, which link `-l<app>`.
  **The template pbxproj sets `ENABLE_USER_SCRIPT_SANDBOXING=NO`** on every configuration (Xcode
  15+ defaults it to YES, which blocks the phase from writing `$BUILT_PRODUCTS_DIR` — Flutter's
  templates set exactly this), marks the Day phase `alwaysOutOfDate=1` (cargo's own incrementality
  is the freshness authority), and declares the staged `$(BUILT_PRODUCTS_DIR)/day/libdayapp.a` as
  an `outputPath`. That declaration is what tells Xcode this phase PRODUCES the archive its
  `-force_load` names: without it a clean tree fails while planning the link — "Build input file
  cannot be found" — because the check runs before the phase does, and an incremental tree hides
  it behind last build's copy. The plumbing detects sandboxing at runtime and fails with
  `day::build::xcode_script_sandboxed` + fix instructions; `day doctor` checks it too.
- **android/**: `settings.gradle.kts` applies the **committed** `day.gradle.kts`, which registers
  a proper task class (`DayRustBuildTask`) — **configuration-cache compatible** (Gradle 9 enables
  it by default): declared inputs (target/profile/ABI list + the conveyance properties file),
  output `layout.buildDirectory.dir("day/jniLibs")` registered via `sourceSets jniLibs.srcDir`
  (**never** writing into `src/main/jniLibs` — source-tree pollution and broken up-to-date
  checks), `outputs.upToDateWhen { false }`, `ExecOperations` only inside `@TaskAction`, invoking
  the arg-less `"$DAY_BIN" gradle-backend build`. A tested Gradle/AGP version matrix is published;
  CI builds the scaffold with `--configuration-cache` ([§20](#20-continuous-integration)).
  The scaffold also commits `gradle/wrapper/gradle-wrapper.properties`, pinning the Gradle version
  the app builds with; `day build` runs the app's own `./gradlew` when it has one and falls back to
  `gradle` on PATH otherwise, so the CLI and an IDE build with the same Gradle. Only the properties
  are committed, not `gradlew` and its jar: an IDE resolves the distribution from the properties
  alone. Without that file an IDE writes its own, pinned to AGP's declared minimum — a milestone
  build newer Android Studio then refuses to sync against.
- **Freshness and fresh clones**: both callback entrypoints regenerate conveyance from `Day.toml`
  first (content-hashed, [§17.5](#175-metadata-conveyance-daytoml--each-build-system)); because Xcode reads xcconfig *before* the phase runs, drift is
  detected and that build fails with "metadata changed — build again". `settings.gradle.kts`
  guards the generated `day-pieces.gradle.kts` apply with an existence check throwing "run `Day
  build` once". Committed-vs-generated is explicit: `day.gradle.kts` and a bootstrap xcconfig stub
  are **create-time committed** files; only value-bearing generated files are gitignored; the
  pbxproj references generated `.lproj` outputs via a folder reference so it never names
  gitignored files.
- Recursion guard: the plumbing entrypoints never re-enter the native build; `DAY_BUILD_PARENT`
  marks provenance for diagnostics.

### §17.5 Metadata conveyance (Day.toml → each build system)

> [!IMPORTANT]
> **Status: shipped; concrete filenames evolved.** The mechanism is exactly as designed —
> generated, gitignored, content-hashed files that committed scaffolds reference generically.
> The real names: Android reads `build/day/android/day-app.properties`, `day-signing.properties`,
> and `day-pieces.json` ([§15.2](#152-package-layout-and-aggregation)); iOS and macOS convey
> identity through `build/day/xcconfig/<platform>.xcconfig` (2026-08), `#include?`d LAST by the
> committed `platform/<p>/DayApp.xcconfig` holding the user-adjustable settings ([§16.5](#165-the-command-surface) day build),
> with the callback phase failing on mid-build drift as designed below — plus the `DayPieces`
> SwiftPM package; the Rust side's "generated metadata" became the `day-build` resource
> constants ([§18.5](#185-typed-resource-constants-docsresourcesmd)). The `day-meta` shared library was folded into `day-cli` (its `meta`
> module) + `day-build`. The table below records the designed shape:

Generated at build time into ignored-by-git locations (like flutter's `Generated.xcconfig` +
`local.properties`):

| consumer | generated file | contents |
|---|---|---|
| Xcode | `platform/ios/Day/Day-Generated.xcconfig` | `DAY_APP_ID`, `MARKETING_VERSION`, `CURRENT_PROJECT_VERSION`, `DAY_BIN`, deployment target |
| Xcode (l10n + plist) | `build/day/gen/ios-l10n/<locale>.lproj/InfoPlist.strings`, copied into `${TARGET_BUILD_DIR}/${UNLOCALIZED_RESOURCES_FOLDER_PATH}` by a "Day L10n" build phase before signing; `Info.plist` is itself a conveyance template into which Day build injects `CFBundleLocalizations` + `CFBundleDevelopmentRegion` | localized `CFBundleDisplayName` etc. from reserved Fluent keys (a static template `.xcodeproj` cannot pre-reference user-defined `.lproj` variant groups — the copy phase is the correct mechanism) |
| Gradle | `platform/android/day-generated.properties` + `res.srcDir("build/day/gen/android-res")` registered by `day.gradle.kts`, with the BCP-47→qualifier mapping (`fr-FR`→`values-fr-rFR`, `sr-Latn`→`values-b+sr+Latn`, `en-XA`→`values-en-rXA`) | applicationId, versionCode/Name, localized `app_name` |
| Rust | `build/day/gen/day_meta.rs` via `DAY_META_PATH` env consumed by the `day` crate's build script | `pub const APP_ID/VERSION/BUILD/DEFAULT_LOCALE` + packaged-asset index |
| CMake/MSBuild | `build/day/gen/day.cmake` / props file | equivalents |

Regeneration is idempotent and content-hashed (touch only when changed — keeps native incremental
builds warm).

**Renaming a project (2026-08).** The scaffold reproduced the project's name in six places and its
bundle id in eight, so renaming an app meant a hunt through five build systems — and a missed site
failed at link time, at `System.loadLibrary`, or not until a deep link went unanswered. The name
now appears ONCE, in `Cargo.toml [package] name`; the id once, in `Day.toml [app] id`. What
removed each copy:

| was | now |
|---|---|
| `[[bin]] name` | dropped — cargo auto-names the binary after the package |
| `src/main.rs` importing `<crate>::root` | `[lib] name = "dayapp"`, a CONSTANT, so the import never moves |
| iOS/macOS staticlib `lib<crate>.a` | falls out of the same `[lib] name` — already the `libdayapp.a` the pbxproj links |
| Android `day.lib` meta-data | dropped; the cdylib is `libdayapp.so` and `DayActivity` defaults to it |
| `rootProject.name` | a constant (Gradle shows it in the IDE; nothing reads it) |
| Android `namespace`, deep-link scheme | the generated `day-app.properties` + a `${dayScheme}` manifest placeholder |
| Apple `CFBundleURLName`/`CFBundleURLSchemes` | `$(PRODUCT_BUNDLE_IDENTIFIER)` and a generated `DAY_URL_SCHEME`, the indirection `CFBundleIdentifier` already used |
| `store/app.toml bundle-id` | omitted — `day store` falls back to `Day.toml [app] id` |
| HarmonyOS `bundleName`, `uris` scheme | rewritten in place on every build (OHOS has no include/properties channel), the way permissions and shortcuts already are |

The scheme DEFAULTS to the id's last segment, so a new app never states it twice, and
`Day.toml [app] scheme` overrides that where an app needs a different one. It is declarable
rather than purely derived because a scheme is a PUBLISHED contract — links already in the world
stop resolving if it moves. Apps scaffolded before the default existed derived theirs from the
CRATE name (`Day-Showcase` ⇒ `dayshowcase`, not `showcase`); making the derivation authoritative
would have rewritten their committed manifests on the next build and changed `dayshowcase://`
links on every platform at once, which is what the harmony pristine check caught.

Two literals survive on purpose, both labeled as such: the Gradle `?:` fallbacks and the
committed xcconfig identity block, which let Android Studio and Xcode open a fresh checkout
before `day build` has run.

An app has TWO names, and they are spelled differently on purpose. `day new app Day-Rise`
scaffolds a **`Day-Rise/` directory** holding a **`day-rise` package**: the repository keeps the
case that was typed, the Cargo package is lowered (`kebab_name`, so no app is born needing a
crate-level `allow(non_snake_case)`). Only one file uses the repository spelling —
`website/site.toml`'s Pages host, whose repository segment is case-sensitive, so
`daybrite.github.io/Day-Rise` and `.../day-rise` are different sites and lowering it would point
a canonical URL at a 404. Everything else derives from the package name.

Permission declarations ([docs/permissions.md](docs/permissions.md), added 2026-07) follow the same touch-only-when-changed
rule but two of their three destinations are CHECKED-IN scaffold files rather than generated ones —
see the exception note in [§15.2](#152-package-layout-and-aggregation), which also records why the
`Day-Generated.xcconfig` + `INFOPLIST_KEY_*` route above was evaluated for them and rejected (the
scaffold pbxproj sets `GENERATE_INFOPLIST_FILE = NO`, and changing that would break running the app
straight from Xcode).

**`cargo build` works standalone — really.** The shipped mechanism: the app's own `build.rs`
calls `day_build::generate_resources()` (scanning `resource/` relative to the manifest — no CLI
required), and the `mock` backend is the default cargo feature, so bare `cargo build`, `cargo
test`, `cargo clippy`, and rust-analyzer work in any checkout. `day build` adds what only the
CLI can: backend feature selection, conveyance files, native pipelines, and the
resource/locale environment for `day launch`.

---

## §18 Resources, icons, and theming

### §18.1 Data resources (lands in **M5**, with the scaffolds — Fluent (M6) and the walkthrough (M7) depend on it)

Assets ship platform-idiomatically, with the per-target mechanics specified now:

- **apple**: the template project's resources phase copies `build/day/gen/resources/` into the
  bundle `Resources/` (same folder-reference rule as [§17.4](#174-the-build-callback-flutters-pattern-exactly--including-the-details-flutter-learned-the-slow-way)'s l10n).
- **android**: `day.gradle.kts` registers the generated tree as an `assets` sourceSet dir;
  lookup via `AssetManager`.
- **cargo desktop targets**: a staging dir beside the binary (bundled into `Resources/` by the
  macOS bundle recipe, `share/` on Linux, packaged content on Windows); dev runs resolve via
  `DAY_ASSET_ROOT` ([§17.5](#175-metadata-conveyance-daytoml--each-build-system)).
- Uniform API: `Asset::named("stations.json").bytes() / .string() / .url()`; locale-qualified
  variants (`assets/fr/…`) resolve like Fluent fallback. The asset index is generated at build
  (into `day_meta.rs`), so `Asset::named` typos are lint-able and `day lint` cross-checks
  references. Piece-package resources aggregate per [§15.2](#152-package-layout-and-aggregation).

> [!WARNING]
> **Superseded:** the shipped data API is `resource("name")` ([§18.3](#183-processed-images--random-access-data-resources-docsresourcesmd)), not `Asset::named`; the
> "generated index, lint-able typos" goal is realized as the **typed resource constants of [§18.5](#185-typed-resource-constants-docsresourcesmd)**.

### §18.2 Icons and images

> [!IMPORTANT]
> **Status: shipped differently.** There is no SVG render pipeline (`resvg` was not adopted).
> In-app images are pre-exported PNGs under `resource/images/` ([§18.3](#183-processed-images--random-access-data-resources-docsresourcesmd)) — the Skip lesson
> (bundle the glyphs; don't rely on platform symbol names) is the working practice, with
> Material Symbols exports as the common source. The **app icon** comes from
> `resource/icons/{macos,linux,windows,png}/` PNG export sets (falling back to any root icon):
> `day pack` assembles `.icns` via `sips` + `iconutil` on macOS, `.ico` on Windows, and the
> freedesktop policy sizes (48/64/128) for flatpak — with embedded defaults so a bare project
> still packs. Dark/light theming is native per toolkit ([§6.3](#63-semantic-theme-tokens)), forced only by `DAY_THEME`.

### §18.3 Processed images + random-access data resources ([docs/resources.md](docs/resources.md))

Two declared buckets — `images/` (processed images for `image("name")`) and `assets/` (arbitrary
data for `resource("name")`) — are routed through each platform's **native** resource machinery so
they inherit its optimizations and by-name load paths. Day never processes pixels itself; it hands
raw files to the native build system, which *optionally* optimizes (actool/aapt2/…). Data is stored
uncompressed where possible so `resource("name")` returns an efficient **zero-copy random-access**
view (`as_slice`/`read_at`/`len`), backed by the platform-native data API — mmap of a bundle file on
Apple, `AAssetManager` on Android, `g_resources_lookup_data` on GTK, `QResource` on Qt, rawfile fd on
ArkUI. Images map to SwiftPM `.process`→`Assets.car` (iOS), `res/drawable`→`R` (Android), GResource
(GTK), `.qrc` (Qt), MRT (XAML), rawfile (ArkUI). `assets/` is staged as a TREE (names are
`/`-relative paths — §18.5's nested modules), recreated in every store: recursive bundle/exe/dist
copies, `assets/`-rooted APK srcDir, path-carrying gresource/qrc aliases, rawfile subdirs. Core
API in `day-core::resource`; build-time staging in `crates/day-cli/src/resources/`. Full design +
per-platform detail: **[docs/resources.md](docs/resources.md)**.

### §18.4 Bundled custom fonts ([docs/resources.md](docs/resources.md))

A third declared bucket — `fonts/` (`.ttf`/`.otf`) — makes `Font::Custom("Family", pt)` resolve by
the font's **family name** on every target. The invariant that makes the name "just work" with no
side table: `day build` parses each file's sfnt `name` table (`day_spec::fonts`, shared by the CLI
and the runtimes) and derives every staged name from the family, so runtimes can re-derive it.
Staging per platform: Android `res/font/<ident>.<ext>` (aapt2 → `R.font`; `DayBridge` re-derives
`<ident>` from the requested family), iOS the DayPieces bundle (`.copy("fonts")`) **plus** a
`UIAppFonts` array synced into the app Info.plist, ArkUI rawfile `day/fonts/` + a `fonts.json`
manifest the scaffold's EntryAbility feeds to ArkTS `font.registerFont`, desktops loose files
(`DAY_FONT_ROOT` under `day launch`; `Resources/fonts` / next-to-exe when packed). Backends
register at startup: CoreText (AppKit/UIKit), fontconfig + CoreText (GTK, per-OS), `QFontDatabase`
(Qt), XAML `path#family` (XAML — unpackaged apps have no registration API). Validation is
build-time and hard: only ttf/otf, a parseable name table, no family-ident collisions. An unknown
family at runtime falls back to the system font with a log line, never a crash.

### §18.5 Typed resource constants ([docs/resources.md](docs/resources.md))

Every bundled resource is also surfaced to app code as a **typed constant**, so a reference is
checked at compile time instead of failing at runtime on whichever backend can't find the name. An
app's `build.rs` calls `day_build::generate_resources()`, which scans `resource/{images,assets,fonts}`
and emits (into `$OUT_DIR`, surfaced by the scaffold's one-line `pub mod res { include!(…) }`):
`res::images::<stem>: ImageName`, `res::assets::<file>: AssetName`, `res::fonts::<family>: FontFamily`.
`resource/assets/` is a TREE: subdirectories generate nested modules, each directory doubling as
a typed `AssetDir` const of the same name (`res::assets::web::minisite` beside
`res::assets::web::minisite::index_html`; values are `/`-relative paths, staged verbatim by every
§18.3 stager) — the handle `web_view_inline` serves bundled sites from ([docs/webview.md](docs/webview.md)).
`image`, `resource`, and `Font::custom` take those newtypes, so `image(res::images::nav_home)` is a
build error if the file is missing and the available names autocomplete; `cargo:rerun-if-changed`
regenerates when a file is added or removed. A name known only at runtime uses the explicit
`ImageName::dynamic(…)` / `AssetName::dynamic(…)` escape hatch (a bare string literal deliberately does
**not** coerce — that is what turns "present" from convention into guarantee); the untyped
`Font::Custom(&'static str, pt)` variant remains the font escape hatch.

`day-build` (a published leaf, `day-fonts` + std only, so an app can take it as a `[build-dependencies]`)
is the **single source of truth** for the name→identifier rule (`sanitize_ident`) — the CLI stagers of
[§18.3](#183-processed-images--random-access-data-resources-docsresourcesmd)/[§18.4](#184-bundled-custom-fonts-docsresourcesmd) re-export it, so the string baked into a constant is exactly the name staged into each
backend's native store. It rejects at build time any image stem that is not portable across toolkits
(differs after sanitization — verbatim on Apple/GTK/Qt but re-sanitized on Android/ArkUI) and any two
files that collide on one symbol, each with a rename hint. This realizes [§18.1](#181-data-resources-lands-in-m5-with-the-scaffolds--fluent-m6-and-the-walkthrough-m7-depend-on-it)'s "generated,
lint-able asset index" intent for the shipped `image()` / `resource()` / `Font` APIs.

The same `build.rs` also emits a **`res::str`** bucket for localization ([§12](#12-localization-fluent)): one function per Fluent
message key under `resource/locales/`, so `res::str::greeting(name)` is a checked, autocompleting stand-in
for `tr("greeting").arg("name", name)`. `day-build` parses each `.ftl` with `fluent-syntax` and shapes
each function's signature from the message's `$variables` (`res::str::hello_world()`,
`res::str::counter_value(count)`, `res::str::deviceinfo_system(name, version)`), so a missing key or wrong
argument count is a compile error, not a runtime `⟨key⟩`. A variable used as a **plural / `select`
selector** (`{ $count -> … }`) is typed `impl IntoNumberFArg` rather than `impl IntoFArg`, so a string can't
be passed where CLDR plural rules need a number (a string select like `$gender ->` is left un-numeric); and
each function's **doc comment carries the reference-locale value** (`/// \`greeting\` — \`Hello, { $name }!\``)
so hover shows the actual text. Two build-time rules apply: every key must be a valid Rust identifier (so
keys are **snake_case**, not the Fluent-legal kebab-case), and **all locales must agree on a key's parameter
names** (`en {name}` vs `fr {nom}` → error; numeric-ness is OR-ed across locales). `tr("…")` stays for dynamic
keys, and using the generated functions is optional (`day lint` counts a `res::str::key` reference as a use).
The `fluent-syntax` parse is the single source of Fluent handling — the codegen, `day lint`'s coverage
checks (`day_build::message_keys`), and the runtime resolver (`fluent-bundle`) all share it, so what the
tooling accepts matches what resolves.

Since the same scan already knows every locale, it also emits a **`res::locales`** bucket (2026-07):
`CATALOG` (one `(tag, ftl_source)` pair per directory under `resource/locales/`, `include_str!`-embedded
and `concat!`-joined when a locale is split across several `.ftl` files), `DEFAULT` (the fallback locale
— `en` when present, else the first tag alphabetically), and `install()`, which registers them. An app's
`root()` therefore says `res::locales::install()` instead of a hand-maintained `install_locales("en",
&[("en", include_str!("../resource/locales/en/app.ftl")), …])`, and **adding a language is adding a
directory** — the list can no longer drift from what ships. The path stays explicit (the generated file
is `include!`d from `$OUT_DIR`, so the embedded paths are absolute), `install_locales` stays public for
lists an app assembles itself, and an app wanting a different fallback keeps the generated catalog:
`install_locales("fr", res::locales::CATALOG)`.

---

## §19 Repository layout, examples, and docs site

> [!IMPORTANT]
> **Status: shipped differently.** The real tree:

```
day/                                # THIS repository
  Cargo.toml                        # workspace
  DESIGN.md                         # this document
  crates/                           # day, day-core, day-reactive, day-geometry, day-spec,
                                    #   day-pieces, day-fluent, day-l10n, day-script, day-mock,
                                    #   day-build, day-fonts, day-toolchain, day-cli
  toolkits/                         # day-appkit, day-uikit, day-gtk, day-qt(+sys),
                                    #   day-android, day-xaml(+sys), day-arkui(+sys)
  pieces/                           # external-style UI pieces (day-piece-combobox, -searchfield,
                                    #   -picker, -rating, -activity, -webview, -media, -map,
                                    #   -lottie, -remote-image, -colorpicker, -texteditor)
  parts/                            # headless platform services (day-part-battery, -network,
                                    #   -sensors, -clipboard, -prefs, -haptics, -deviceinfo,
                                    #   -http, -permissions, -location)
  tweaks/                           # packaged tweaks (day-tweak-button-bezel, -tooltip,
                                    #   -slider-tickmarks) — Addendum, docs/tweaks.md
                                    # (the apps live in their own repositories: daybrite/Day-Showcase
                                    #  is THE demo — every subsystem, 4 locales, the walkthrough —
                                    #  and daybrite/Day-Matrix is the scale proof, a full Matrix
                                    #  client with its own DESIGN.md. CI checks the showcase out
                                    #  to build the framework against the app it documents.)
  docs/                             # the normative subsystem docs (see the index at the top)
  website/                          # Astro site: curated guides + docs/ symlinked as the
                                    #   internal reference (scripts/website.sh builds it)
  scripts/                          # CI + release helpers (screenshot validation, duty matrix,
                                    #   installer packaging, API-docs build, website.sh)
  .github/workflows/                # ci.yml (build/test/e2e/pack/release), checkup.yml
```

Scaffold templates are embedded in `day-cli` (no `templates/` tree); the sample apps the design
imagined (`counter`, `fieldnotes`, `deskclock`) were folded into the showcase's pages and the
scaffold's starter pages. Apps and pieces still depend on Day exactly as external users would —
the `pieces/`, `parts/`, and `tweaks/` crates are the continuous proof that extensions never
need core edits.

Docs are two-layer by design: `docs/*.md` in this repo is normative per subsystem (and heavily
cited from source comments); `website/` is the curated public site (Astro) — guides (overview,
api-tour, reactivity, layout, dayscript, packaging, …) plus the internal reference, which
**symlinks** `docs/*.md` under `/docs/internal/…` so it can never drift. A companion repo,
`daybrite/actions`, publishes the reusable GitHub workflows external Day apps build and deploy with.

---

## §20 Continuous integration

> [!IMPORTANT]
> **Status: shipped, consolidated.** Instead of the designed four workflows, one `ci.yml`
> carries the whole pipeline, plus `checkup.yml` (scheduled end-user install checks — one
> `day checkup -p <combo> --day-version <v> --strict` per cell of an 11-combo × 2-version matrix,
> `main` and `latest`, [§16.5](#165-subcommands); it was `install.yml` until 2026-08, when the
> doctor/new/build steps moved into the CLI and packaging and the version axis joined them) in
> this repo.
> External Day apps are served by the **`daybrite/actions`** companion repo: one reusable
> `build-day-app.yml` matrix workflow that builds, packs, attaches release assets on a `vX.Y.Z`
> tag — including two generated launcher scripts, `launch.sh` (macOS `.dmg`, Linux `.appimage`)
> and `launch.ps1` (the Windows per-user installer), which are release ASSETS rather than hosted
> files so the URL chooses the version and every Day app gets a one-line try-it path without
> hosting anything — and — with `deploy-web: true` and web-dom among its targets — also deploys that build to the
> app repo's own GitHub Pages (reusing the dist it already built; relative-path so a project-Pages
> subpath works; a `web-deploy-tag-pattern` input gates publish-on-tag vs publish-on-main;
> [docs/web.md](docs/web.md)), plus a scaffold-validation workflow. The Gradle/AGP legs use the runner's DEFAULT
> preinstalled JDK (Day accepts 17+; the old "pin 21" was an AGP-8-era quirk).

`ci.yml`, in order:

1. **Fast checks** — rustfmt, MSRV build, and the toolkit-independent clippy (host-portable
   crates + CLI/dayscript + mock-backend showcase, all `--all-targets`; the android
   cross-*check* lint; the drift checks for all three generated tables — duty, piece-vocabulary
   coverage, and recorder coverage); plus **`spelling`** (2026-08), which runs `typos` over the
   whole tree. It gates two things at once: misspellings, and STYLE_GUIDE.md's American-English
   rule, which `typos.toml`'s `locale = "en-us"` turns from a convention into a check. The gate
   arrived with the cleanup it enforces — 355 British spellings had accumulated across 116 files,
   90% of them in doc and code comments rather than in the prose the style guide names, because a
   dialect is not something anyone decides one comment at a time. Exceptions live in `typos.toml`,
   each with its reason: `cancelled` (pinned by `HttpError::Cancelled` and by GitHub Actions'
   own `cancelled()`), the deliberate misspellings that ARE test fixtures (`--profile relaese`,
   the `stlye:` Fluent lint case), and the starter-app translations, which are not English.
   Clippy is a required status but NOT in
   the combos' `needs:`, so a lint error blocks merge without suppressing the platform
   matrix's build/test signal (it once rode the linux-day artifact job, where a pure lint
   failure killed the CLI artifact and with it every Linux-descended combo). `spelling` and
   `deny` sit the same way.
2. **CLI builds** — the `day` binary in release for 3 OSes × 2 arches; artifacts feed every
   later job (and the release lane). The Linux and Windows legs each run on hardware matching
   their arch (`ubuntu-24.04-arm` / `windows-11-arm` for aarch64, 2026-08); macOS x86_64 is
   the one remaining cross build, GitHub having retired its Intel runners. Every arch-matching
   leg runs the host-portable test suite first (`scripts/ci/host-test.sh`: the whole workspace
   minus the toolkit crates) — so a failing host test fails that leg
   before the release build spends anything, and every combo downstream of that binary skips.
   The script exists because a bare `cargo test` covers only `default-members`, the small
   quick-iteration set — which until 2026-08 silently left most of the workspace's tests
   (day-cli's and day-persistence's entire suites among them) out of CI.
   The tag lane skips the test step (the tagged commit was already validated on main). The leg whose arch matches its runner also runs
   `scripts/ci/scaffold-check.sh`, the only place CI exercises `day new app` end to end: it
   scaffolds a 21-locale project and lints it with `--strict --allow store-placeholder`, so every
   rule but the listing TODOs a human still has to write holds against a fresh project. It runs on
   all three OSes because the four locale surfaces (`resource/locales/`, `store/`, Xcode's
   `knownRegions`, `website/site.toml`) are written through platform path handling.
3. **Framework checks** — `toolkit (<backend>)` lints the showcase against one backend crate's
   feature and scaffolds a piece/part/app for it. Feature unification is why it exists — a
   `--workspace` clippy would link several backends into one binary and trip the
   one-backend-per-binary guard ([§3](#3-crate-architecture)). It used to run INSIDE the
   per-combo jobs, which made every build job framework-shaped and unusable as an app pipeline;
   split out, it runs beside the build pipeline. The other framework check, the host-portable
   `cargo test`, rides the CLI builds' native-arch legs (phase 2). Two
   backends still lint from their own combo job, because that job already sets up the cross-target
   toolchain and a `toolkit` row would mean a second copy of it: arkui needs the OpenHarmony SDK,
   dom the wasm32 target plus a wasm-capable clang for persistence's bundled SQLite
   ([docs/web.md](docs/web.md)).
4. **Per-combo jobs** (macOS: appkit/gtk/qt; Linux: gtk/qt headless; Windows: xaml; plus a
   dedicated `ios-uikit` Simulator job and an Android emulator job): each checks out
   daybrite/Day-Showcase, points its day dependencies at this commit (`day patch --check`,
   [§16.5](#165-subcommands)), and runs `day doctor`, the **showcase walkthrough ×
   light/dark/fr** with content-validated screenshot uploads, service round-trip scripts (e.g.
   clipboard), and `day pack` — the generic app pipeline, nothing framework-shaped. Every leg packs at the dev tier: releasing and signing the showcase is its own
   repository's business ([§20.2](#202-release-signing-isolation)), and these jobs exist for the
   build/walkthrough/screenshot signal. A `web-dom` job builds the showcase's wasm
   dist (`day build -p web-dom --profile release`) and runs the SAME walkthrough ×
   light/dark × en/fr/ar/zh-CN in headless WebKit
   (`DAY_WEB_DRIVER` = the CLI's bundled page-driver, `day web driver`; the dayscript WebSocket bridge, §14.5),
   uploading `screenshots-web-dom` for the gallery's "Web DOM" column ([docs/web.md](docs/web.md)). It does not
   publish the dist — the app deploys its own web build to daybrite.github.io/Day-Showcase from its
   own repository, and daybrite.dev links there.
5. **Release lane** (semver tags) — publishability check (`cargo publish --workspace
   --dry-run`), tag-vs-version check, GitHub release with the six CLI binaries and the installers,
   and crates.io Trusted Publishing (wired; crates not yet published —
   [§1](#1-glossary-and-naming)). It ships the CLI and nothing else: signing, notarizing and
   store distribution of an app all belong to that app's repository, through daybrite/actions
   ([§20.2](#202-release-signing-isolation), [docs/store.md](docs/store.md)). The website's `/showcase/` page
   therefore links the release assets of `daybrite/Day-Showcase` over the API
   (`website/scripts/assemble-downloads.mjs`) rather than serving anything this run built.

CI knowledge banked in the workflows from day one: JDK pinning, rustup toolchains for
cross-std, `--locked` everywhere, emulator boot polling, screenshot content validation
(`scripts/ci/validate-screenshots.sh`), and the freedesktop icon-size rules flatpak's
`appstreamcli` enforces.

### §20.2 Release signing isolation

> [!NOTE]
> **Moved 2026-08.** This model now lives in daybrite/actions' `build-day-app.yml` as the
> `sign-macos` job, so every Day app gets it rather than only the showcase — which is itself an app
> repository now (daybrite/Day-Showcase). day's own release publishes the CLI: six binaries, the
> installers and the Homebrew formula, none of which touch a Developer ID. The five properties
> below are the contract that job keeps; they were written here first and are unchanged.

> [!IMPORTANT]
> **Status: shipped.** Supersedes the original arrangement, in which the macOS combo job held the
> Developer ID certificate and the notary key as repository secrets gated by an inline
> `startsWith(github.ref, 'refs/tags/')` test.

The macOS Developer ID identity and the App Store Connect notary key are reachable from exactly
one job, `notarize`, and from nothing else in any workflow. The rule it enforces: **the code that
builds the app and the credentials that sign it never occupy the same runner.**

`macos` builds and packs at the dev tier on every ref, tags included, then uploads the unsigned
`.app` as a `ditto -c -k` zip (`upload-artifact` preserves neither the executable bit nor symlinks,
and a bundle needs both to survive `codesign`; `ditto` keeps them, a plain `zip` drops the
symlinks). `notarize` expands it with `ditto -x -k` and re-runs the signing half of §16.5's macOS
lane — sign inside-out → `hdiutil`
→ sign the dmg → `notarytool` → `stapler`. Duplicating those stages in YAML is the cost of the
isolation, so a change to either copy updates the other.

What holds the boundary:

| Control | Effect |
| --- | --- |
| `environment: release-signing` | The six secrets are environment-scoped, so no other job in the run can read them, and the environment's deployment rule admits only `v*` tags. A required reviewer is available and deliberately NOT used: a tagged release signs and notarizes without waiting on a human, so the gate is the tag itself. |
| `github.event_name == 'push' && startsWith(github.ref, 'refs/tags/v')` | `workflow_dispatch` cannot aim the job at a chosen ref, and a bare `refs/tags/` prefix would have matched any tag. |
| No `actions/checkout` | The tagged commit's source never reaches the runner holding the keys. |
| Apple's tools only | Cargo and the `day` CLI never run here, so a compromised dependency gets no execution beside the credentials — its `build.rs` ran in a job with no secrets. |
| Ephemeral keychain | Created for the job, auto-locking, deleted in `always()`. |
| `permissions: contents: read` | The job cannot write to the repository or mint an OIDC token. |
| Identity pinned by SHA-1 | `codesign -s` takes the fingerprint read back out of the keychain, not a name. The team holds two Developer ID certificates with identical common names, so a name would be ambiguous — the job also asserts the keychain holds exactly one identity and that it is the expected one. |

The `release` job checks `needs.notarize.result` before publishing, because its `!cancelled()`
guard would otherwise let a failed notarization ship a release whose macOS package is simply
absent from the artifact glob.

Residual exposure, recorded rather than fixed here: `ios-uikit`, `android-mdc`, and
`harmony-arkui` still import their signing material into jobs that run repo code, and every
third-party action in the workflow floats on a tag rather than a commit SHA.

### §20.3 Reproducible-build verification

> [!NOTE]
> **Status: shipped, partial by design.** The payload tier is enforced; the container tier reports
> but does not fail. Nothing in the tree sets `SOURCE_DATE_EPOCH` yet. The six `<combo>-validate`
> follow-up jobs this section describes were replaced (2026-08) by a scaffold rebuild check inside
> each packing platform job: `scripts/ci/scaffold-check.sh` scaffolds a fresh 21-locale app, packs
> it, and verifies it with `day rebuild --from-dir --strict` on the same runner — the desktop
> combos then launch the rebuilt copy. Stage 1's install-and-launch of the shipped showcase
> artifact retired with those jobs — except on Linux, where the packing job still installs the
> `.flatpak` and RUNS the `.appimage` under xvfb (2026-08). The AppImage's claim is that it works
> on a machine with nothing installed, and the only check for that is executing it: a GTK or Qt
> module the bundling failed to carry crashes on launch and nowhere earlier. A rebuild check
> compares normalized bytes; it cannot notice a missing loader.

The user-facing version of this section is `website/src/content/docs/reproducible-builds.md`,
which carries the per-platform caveats and the manual verification recipe; keep the two in step.

**What is guaranteed.** A rebuild is not promised to be byte-identical to the artifact it verifies.
It is promised to be identical **after normalization** — once the parts that describe the machine
and the moment, rather than the compiled program, are removed. `normalize()` in `rebuild.rs` is
where that line is drawn, and for Mach-O it removes three things: the code signature (computed over
the bytes below it, and carrying the packager's identity and clock), the `LC_UUID` build id (unique
per link by design), and the debug map — the `N_OSO` stabs, in which the linker records an absolute
path to every object file it consumed. Everything that decides what the program does is left alone:
text and data, the symbol table proper, the load commands, the linked libraries. A change in any of
those still fails the check. Both external steps `normalize()` drives (`codesign
--remove-signature`, then `strip -S`) are checked: a tool that refuses ends the run with the reason
it gave, since a silent refusal leaves in place the exact bytes normalization exists to remove and
then reports them as a difference in the code.

**When it fails.** The verdict names the first difference, not only the file holding it. A text
member quotes the differing line. A compiled member reports how many bytes differ, the offset of
the first, and the Mach-O region that offset falls in (`__TEXT,__text`, `__LINKEDIT symbol table`,
a named load command, a slice of a fat binary), and answers a length mismatch as such. The runner
is gone by the time anyone reads its log, so the verdict has to carry the evidence with it.

The debug map is the reason this has to be a normalized comparison rather than a byte one. Those
paths reach into `SYMROOT`, into cargo's output, and into the build directory of any SwiftPM package
the app links, so the same commit built from two directories differs in both content and *length*.
`day build` passes `-oso_prefix` to shrink them to project-relative paths at link time (§16.5,
`mobile.rs`), which removes most of them, but it cannot reach the objects a SwiftPM package prelinks
with `ld -r` — Xcode gives that step neither `OTHER_LDFLAGS` nor `PRELINK_FLAGS`. Requiring byte
equality would therefore mean pinning the build directory, which is a weaker claim than the one
made here, not a stronger one.

Every platform-toolkit job that runs `day pack` has a follow-up `<combo>-validate` job, and it runs
two stages in order. **Stage 1 installs the shipped artifact and launches it** on a runner with
nothing else installed — the `.dmg` is mounted and dragged to `/Applications`, the `.flatpak` is
installed from the bundle, the `.apk` goes onto an emulator, the NSIS installer runs silently — and
the job stops there if the app does not survive ten seconds. An artifact that only runs on a machine
which already has the toolchain is broken for whoever downloads it. Two targets cannot be installed
on a runner at all: an `-sdk iphoneos` `.ipa` needs provisioned hardware, and a `.hap` needs the
Oniro emulator that makes the parent job the flakiest leg in the workflow. Both get structural
validation instead, and say so rather than implying a launch. The Android leg's install-and-launch
check is `scripts/ci/validate-apk.sh`: the emulator action runs each line of its `script` input as
a separate `sh -c`, so a check that needs shell state has to be a file it invokes rather than a
block it inlines.

**Stage 2 checks reproducibility**, and only runs once stage 1 passes. The whole stage is one
command, `day rebuild --strict <artifact>` (§20.4): it reads the SBOM and `.buildinfo` shipped
beside the artifact, gates on the recorded tool versions, clones the recorded commit into a scratch
directory, packs it again, and compares. The SBOM records WHICH project in that repository was
packed (`day:project`, from `git rev-parse --show-prefix`) — this one holds three apps plus the
scaffold templates, and a rebuild that searched for a `Day.toml` would pack whichever the directory
walk reached first. The scratch directory is at a different absolute path than
the parent job built in, which is the point — `day pack` hardcodes its output under
`<project-root>/build/day/dist` and `find_project` canonicalizes that root, so the second build
genuinely drives every downstream tool from a different prefix, and a source path baked into a
binary surfaces as a mismatch rather than hiding.

Both stages rest on the packing job having built from a **pristine checkout**: `day rebuild` refuses
an artifact whose SBOM records a dirty tree, since a commit cannot describe a tree that has extra
files in it — and on GitHub Actions the workspace IS the checkout, so anything a job downloads into
it (the `day` CLI artifact, once) counts. Downloads go to `$RUNNER_TEMP`, and
`scripts/ci/assert-pristine.sh` runs immediately before every `day pack` so a stray path names
itself there rather than surfacing a job later as an artifact nothing can rebuild.

Stage 2 needs no checkout of its own; only `android-mdc-validate` still checks the repo out, because
its stage 1 runs `scripts/ci/validate-apk.sh` on the emulator. Each job names the container it
verifies (`*.dmg`, `*.ipa`, `*.flatpak`, `*.appimage`, `*.apk`, `*.msix`, `*.hap`) rather than globbing the dist
directory: `windows-xaml` also ships a self-extracting `-setup.exe` that nothing here can open, and
`android-mdc` also ships an `.aab`.

Two build inputs are recorded and replayed, because their defaults depend on the machine rather
than the commit: `DAY_ANDROID_ABI` and `DAY_OHOS_ARCH` fall back to "whatever device is attached,
else a fixed default", so a rebuild on a runner with nothing plugged in packs one ABI where the
shipping job packed two, and the two artifacts differ structurally for a reason no verdict could
explain. The buildinfo records what the build resolved; `day rebuild` re-applies it.

The payload tier has a second route for containers the verifying host cannot open — an
`.appimage` is an ELF with a squashfs appended, a `.flatpak` is
an OSTree bundle whose import wants privileges a CI runner does not have, and a `.msix` needs a
working `unzip`. For those, `day pack` records the sha256 of every staged payload file (the compiled
code as the build wrote it, before packaging) and `day rebuild` hashes what it staged and compares.
Debian's `.buildinfo` has always done exactly this, and it is what turns "not checked" into a
verdict for those two targets.

`day rebuild` grades the result in two tiers:

| Tier | What it compares | On mismatch |
| --- | --- | --- |
| payload | the compiled code — Mach-O / ELF / PE / `.so` — extracted from whatever container ships it | **fails the job** |
| container | the shipped file itself (`.dmg`, `.ipa`, `.apk`, `.aab`, `.hap`, `.msix`, `.flatpak`, `.appimage`, `-setup.exe`) | warns |

The payload tier excludes signature material, matched by path component: `AppxSignature.p7x`, the
`AppxMetadata/CodeIntegrity.cat` catalog, anything under `_CodeSignature/`,
`embedded.mobileprovision`, and jar signature files under `META-INF/`. CI signs Windows packages
with a certificate generated per run, so those bytes differ on every pack while every compiled byte
is identical — counting them as payload differences held windows-xaml red for the one cause this
section already classes as advisory. Excluded files are named in the output, never dropped
silently.

It also excludes the CONTAINER INDEX — `AppxBlockMap.xml` and `[Content_Types].xml`, which
`makeappx` writes from its own walk of the staging directory and which record member sizes,
local-file-header sizes, per-block hashes and the extensions present. They describe the ZIP, not
the app, and two packs of a byte-identical payload have produced differing ones. Excluding them
cannot mask a payload change: every file they index is compared directly, and a member appearing or
disappearing is caught first as "different sets of files". The output names the category
(`signature` / `container index`) beside each excluded file, and a member that still differs is
reported with its first differing line, since the reader usually cannot open the package.

`--strict` adds a third outcome: a payload that could not be compared at all — a container this
host has no extractor for — fails rather than reporting "not checked". CI passes it because a green
run
that never opened the artifact is a false pass, and this check has produced two of those before.

The split reflects what was measured, not a preference. On `macos-appkit` the compiled executable
is byte-identical across build directories once two things are normalized away: the Mach-O
`LC_UUID`, which Apple's linker derives from the object-file paths, and the ad-hoc signature that
covers it (`zero_macho_uuid` in `rebuild.rs`, ported from and validated against the shell checker
it replaced, `scripts/ci/macho-normalize.py`, last present at 07dc6ac). The `.dmg` around it
differs on *every* build, even in the same directory, because `hdiutil` stamps mtimes. That
was originally true of every container
— `ditto -c -k`, Gradle's zip writer, `flatpak-builder`'s ostree commit, `makeappx`, `makensis` —
and failing on it would have made the check permanently red. Those clocks are normalized now (see
the table below), so a container mismatch today means a linker build-id or a signature, neither of
which a build controls. It stays advisory for that reason rather than the original one.

`ios-uikit` needed a real fix to reach that bar, and it is the reason `REPRODUCIBLE_BUILD_SETTINGS`
exists in `pack/ios.rs`. Xcode is the only linker in the matrix that writes a **debug map** into the
product: one `N_OSO` stab per object file, each holding that `.o`'s absolute path under `SYMROOT`,
which derives from the project root. The showcase app carried 267 of them, so the same commit built
in two directories produced two different binaries — the first failure this check caught. Passing
`DEPLOYMENT_POSTPROCESSING=YES STRIP_INSTALLED_PRODUCT=YES STRIP_STYLE=debugging` strips the debug
map; Xcode runs `dsymutil` before `strip`, so the `.dSYM` still appears and symbolication is
unaffected, `STRIP_STYLE=debugging` keeps the symbol table so in-process backtraces still resolve,
and the binary loses ~17% of its size. What remains is the same 16-byte `LC_UUID` as on macOS.
`day build` (the simulator/dev lane in `mobile.rs`) is deliberately untouched — stripping a
debug build would be a poor trade.

Exit codes: `0` both tiers match, `10` code matches and packaging does not, `1` the code itself
differs, `2` the two builds produced different file lists. On any mismatch the job installs
diffoscope and attaches its HTML and text reports as `repro-report-<combo>`.

These run on pushes to `main` only. Tags are excluded because release signing embeds a wall-clock
TSA timestamp, which makes a signed artifact non-reproducible by construction; giving the repro
jobs signing secrets so they could match would reopen exactly the exposure
[§20.2](#202-release-signing-isolation) closes.

**A check that cannot answer must not report success.** Two false passes shipped in the first
version of this and are worth recording, because both made the job look green while verifying
nothing:

- The payload verdict came from `diff -rq`. `harmony-arkui` puts the OpenHarmony SDK toolchains on
  `PATH`, and they ship a `diff` that rejects GNU options *and exits 0* — so every payload
  comparison passed unconditionally. Nothing decides a verdict with `diff(1)` now; `cmp` does, and
  the script prepends `/usr/bin:/bin` so a vendored SDK cannot shadow coreutils again.
- A format with no payload extractor was reported as "code reproducible" on the strength of a
  comparison that never ran. Unopenable now means **UNVERIFIED, and unverified fails.**

That second rule is why the `linux` job uploads `stage/bin/` as `showcase-payload-<combo>`. A
`.flatpak` is an OSTree bundle no ordinary archiver can open, so `flatpak.rs`'s pre-bundle ELF is
what `linux-validate` compares — the same shape as macOS handing over its `.app`. The bundle itself is
not byte-compared, and the job says so rather than implying coverage it doesn't have.

It is also why `windows-xaml` blocked on its NSIS `-setup.exe` until an extractor existed: the
`.msix` beside it opened as a zip and verified, but the installer did not, and one unopenable
artifact is enough to make the whole combo UNVERIFIED. 7-Zip reads the NSIS format (and is
preinstalled on the Windows runners), so `extract_payload` shells out to it — skipping
`$PLUGINSDIR` and the generated `uninst.exe`, which are NSIS's own furniture rather than day's
output and are rebuilt per pack. Comparing those would have graded makensis's determinism instead
of day's, and turned an advisory container diff into a spurious hard failure.

**What the first enforcing run found.** Five combos failed, and only two were about compiled code:

- `windows-xaml` — 24 bytes, all of them the PE `TimeDateStamp` and its copy in the debug
  directory, differing by exactly the gap between the two jobs. `ops.rs` now passes
  `-Clink-arg=/Brepro`, which substitutes a hash of the input for the wall clock; the `.exe`
  inside the `.msix` stopped differing on the next run. What remained was the NSIS `-setup.exe`,
  a self-extracting installer no comparator here can open — so the job uploads
  `build/day/pack/windows-payload/` (staged before either container is built, and the input to
  both) and compares that, the same shape as macOS's `.app` and Linux's `stage/bin`.
- `harmony-arkui` — not a codegen difference at all: the two `.hap`s carried *different
  architectures*. `ohos_build_arches()` probed connected devices before consulting
  `DAY_OHOS_ARCH`, so a pack run alongside the x86_64 emulator shipped x86_64 while the same
  commit packed elsewhere shipped arm64 — and `entry/libs/<abi>/` was never cleared, so a stale
  arch could ride along. The override now wins, and the libs tree is cleared before staging. A
  distribution pack must not change shape because a device was attached.
- `ios-uikit` — three causes stacked, and each was only visible once the one above it was gone.
  The `LC_UUID`/debug-map strip was necessary but not sufficient; underneath it, every one of the
  201 `__objc_stubs` entries pointed at a different `__got` slot than in the other build. The
  binary carries TWO slots for `_objc_msgSend` — one for the classic `__stubs` path, one for the
  `__objc_stubs` section Xcode 14 added — with byte-identical GOT contents, and which consumer
  gets which slot is not stable. The two builds were equivalent; the linker flipped a coin.
  `-fno-objc-msgsend-selector-stubs` leaves one slot, so there is no coin. Note this also means
  the same-runner comparison passing for iOS was a ~50/50 result, not evidence.
- `linux-gtk` / `linux-qt` — two symbols differ only in their `.llvm.<moduleId>` suffix, which
  cascades into `NT_GNU_BUILD_ID` and a two-byte `.strtab` change.

**The Linux cause was ThinLTO symbol promotion**, and finding it took a diagnostic rather than a
guess. A first round of reasoning rejected path-dependence outright — the real showcase built
byte-identical across two directories on this machine (same sha256), identical across `-j1` and
`-j8`, and Android's `.so` and macOS's Mach-O both passed in CI with the same rustc, runner image,
and system libraries. That reasoning was wrong, and only a second build *at a third path on the same
runner* showed it: two builds one machine apart still differed.

What differed was narrow. The machine code was identical — same addresses, same sizes — and so were
the crate disambiguators and CGU names. The whole delta was the `.llvm.<hash>` suffix on two
promoted symbols, plus what that drags along: the GNU build-id, and four bytes of `.strtab`.

ThinLTO makes an internal symbol external so it can be inlined across modules, and renames it with
that suffix to avoid collisions. **What feeds the hash is not established here** — upstream reports
attribute it to different inputs in different cases (rust-lang/rust#129080 is the standing list, and
the rustdoc case traced to PGO rather than to paths) — so what follows is a fix for a measured
symptom, not for a diagnosed root cause. `[profile.release] lto = "fat"` merges everything into one
module, so no cross-module promotion happens and the suffix is never emitted: measured on the real
app, 53 occurrences down to zero, and `linux-gtk`/`linux-qt` went green on the next CI run. Cargo's
`trim-paths`, the more targeted candidate, is **still unstable as of Cargo 1.97** and unusable from a
stable toolchain. `codegen-units = 1`, already set, is the documented half of this. Fat LTO costs
roughly 40% more link time and produced a binary ~9% smaller.

A temporary second build at a third path on the same runner carried `linux-validate` and
`ios-uikit-validate` through this. It paid for itself twice — proving Linux really was path-dependent
when a first round of reasoning said otherwise, and then showing iOS was not — and has been removed
now that every combo is green. Worth rebuilding if a platform ever regresses: varying the directory
alone is what separates "depends on its path" from "the two runners disagreed".

`notarize` and `macos-appkit-validate` both `needs: [macos]`, which waits on the WHOLE matrix — so an
unrelated leg failing (macos-gtk's walkthrough, in the run that surfaced this) silently SKIPPED
them. Both now carry `!cancelled()`, so a sibling's failure cannot quietly cancel a release
signing; if the appkit leg itself fails they fail at download-artifact instead, which is honest.

`ZERO_AR_DATE=1` rides on every cargo and xcodebuild invocation (`ops::apply_determinism`).
`libtool` and `ld64` otherwise write file modification times into static archives and into the debug
map's `OSO` entries. It is set in the CLI rather than in CI so local packs are deterministic too —
reproducibility that only holds on the build farm is not worth much. It is preventive here: the
`OSO` entries are already gone via `STRIP_STYLE=debugging`, so what it still protects is the
intermediate `.a` archives and any future config that keeps a debug map.

**The container tier is closed for `ios-uikit`.** `ditto -c -k` copies each entry's modification
time into the ZIP — both the DOS field and the `UX` extra field — and has no flag to suppress it, so
two packs differed by exactly the wall-clock gap between them. `pack/ios.rs::normalize_mtimes` walks
the staging tree and stamps every entry before archiving, deepest first so a directory is stamped
after its contents. The timestamp is `SOURCE_DATE_EPOCH` when set, else 2020-01-01T00:00:00Z — not
the Unix epoch, because ZIP's DOS field cannot encode anything before 1980 and an out-of-range value
would be clamped back into variance. Measured: two clean packs now produce a byte-identical `.ipa`.

The walk is plain `read_dir` rather than `walkdir` — this is a tree Day just created, so a general
walker's loop detection and ordering guarantees would be unused weight. The stamping is `filetime`,
because std genuinely cannot do it: `File::set_times` follows symlinks, and on Windows a directory
cannot be opened at all without `FILE_FLAG_BACKUP_SEMANTICS`. `filetime` was already in the tree via
`tar`, so naming it added one dependency edge and zero compiled crates. Keeping this in Rust rather
than shelling out to `find`/`touch` also means it ports to the Windows and Android containers when
their turn comes.

The same normalization is applied wherever a container tool reads the clock, via two shared helpers
in `pack/mod.rs`. Which one applies depends on whether Day stages the tree the archiver reads:

| Container | Lever |
| --- | --- |
| `.ipa`, `.msix`, `-setup.exe` | `normalize_mtimes` on the staging tree before archiving. The Windows two share one payload dir, so it is stamped once in `msix::stage_payload`. |
| `.hap` | `normalize_zip_mtimes` — hvigor emits the zip itself, so the finished archive is patched instead. |
| `.apk` / `.aab` | Gradle's own `isPreserveFileTimestamps = false` + `isReproducibleFileOrder = true`, in the app template and the showcase project. |
| `-setup.exe` | `SetDateSave off` in the generated `.nsi`; the `/SOLID lzma` compressor was already deterministic. |
| `.flatpak` | `SOURCE_DATE_EPOCH`, honored by flatpak-builder 1.3.1+. `ops::apply_determinism` now EXPORTS the resolved epoch to every child, so one clock governs the whole pack rather than each tool inventing its own. |

`normalize_zip_mtimes` rewrites the DOS date/time words in both the local headers and the central
directory, and the Unix times inside the `0x5455` extended-timestamp and `0x000a` NTFS extra fields
— the DOS words alone are not enough, because `unzip` and diffoscope both prefer the extra field and
will keep reporting the old date. Entry offsets and compressed data are untouched, so the rewrite
cannot invalidate an archive.

**Ordering matters: normalize before signing, never after.** A `.hap` signature covers the local
headers, so patching timestamps afterwards would invalidate it — the release path normalizes the
unsigned hap and then signs, and the dev path (already hvigor-signed) is deliberately left alone.

**What signing costs.** Two containers cannot be byte-reproducible however much is normalized, and
it is worth being explicit about why rather than chasing them. A signed `.hap` carries an
`SHA256withECDSA` signature, and ECDSA picks a random `k` per signature — measured, the residual
13405-byte difference between two haps of byte-identical content is entirely the signing block. The
released `.dmg` is stapled, and `xcrun stapler staple` writes an Apple-issued notarization ticket
into it. For both, the payload tier is the real guarantee.

`.dmg` is left advisory by choice. Measured: two DMGs from identical, mtime-normalized input differ
by 628 bytes uncompressed under APFS — UUIDs plus a Fletcher-64 checksum on every block — or 151
bytes under `-fs HFS+`, split across GPT GUIDs and their CRC32s, the volume header's dates and
`finderInfo` UUID, and per-file creation dates in the catalog B-tree (`hdiutil` stamps copy time as
birthtime, so normalizing the source does not reach them). UDZO then amplifies any of that to the
whole file, because a ten-byte shift in compressed length moves every subsequent chunk. Closing it
means forcing HFS+ and rewriting three classes of field in the uncompressed image before
`hdiutil convert` — a few hundred lines of undocumented-format surgery, for an artifact that gets a
notarization ticket stapled into it anyway.

On Windows diffoscope also installs via pip without most of its comparators and degrades to a
binary diff.

### §20.4 Provenance: SBOM + buildinfo

> [!NOTE]
> **Status: shipped.** Both documents are generated on every `day pack` and `day rebuild` reads
> them. The artifact-prefixed sidecar naming below replaced a fixed-name scheme in 2026-08.

Two documents travel with every packaged artifact, and they answer different questions. The
**SBOM** describes what the app was built FROM — the resolved dependency graph, the repository,
the commit, and which project inside it (`day:project`, from `git rev-parse --show-prefix`).
It derives only from source, so it is identical on every machine, which is why `day pack` writes
it before the build and can stage it into the bundle. The **buildinfo** describes what the app
was built WITH — compiler, SDK and packaging-tool versions, the environment inputs that shaped
the package ([§20.3](#203-reproducible-build-verification)), and the sha256 of every staged
payload file. It is machine-specific by nature, so it is never embedded: doing so would make the
artifact differ whenever a tool version differed.

`[sbom]` in `Day.toml` ([§17.3](#173-daytoml)) picks the mode (`sidecar`, the default; `embed`,
for an app that shows its own license screen; `none`) and the formats (CycloneDX 1.5 and SPDX 2.3
JSON, both by default). The buildinfo is always a sidecar.

**Sidecars are named after the artifact they describe**, whole file name including the extension:

```text
day-showcase-macos-appkit.dmg
day-showcase-macos-appkit.dmg.buildinfo.json
day-showcase-macos-appkit.dmg.sbom-cdx.json
day-showcase-macos-appkit.dmg.sbom-spdx.json
```

A release directory merges every target's dist, so a fixed `day-sbom.cdx.json` there says nothing
about which download it belongs to — and a pack that emits both an `.apk` and an `.aab` needs one
set per artifact, not one per target. `day rebuild` resolves the sidecar by that exact name rather
than scanning the directory, which is what makes verifying a downloaded asset among six others
work. The EMBEDDED spelling stays fixed at `day-sbom.cdx.json` / `day-sbom.spdx.json`, since an
app looks its own document up by name at runtime.

The Linux targets additionally emit `<artifact>.buildinfo.deb822` — the same facts in Debian's
deb-buildinfo(5) format, which is what Debian's reproducibility tooling consumes. It does not take
Debian's `${source}_${version}_${arch}.buildinfo` filename: that convention means something inside
a Debian archive, and here the file ships as a release asset beside artifacts from six other
platforms.

### §20.5 Toolchain and dependency governance

> [!IMPORTANT]
> **Status: partially shipped.** The MSRV CI job and edition 2024 are real; `Cargo.lock` is
> committed. `rust-toolchain.toml` and `cargo-deny` were not adopted (kept here as intended
> future hardening).

---

# Part II — Historical record

> [!IMPORTANT]
> Everything from here to the appendices is the **completed plan**: the MVP definition, the
> milestones, the decision points, the risk register, and the adversarial-review record. It is
> kept verbatim (plus outcome notes) because it documents *why* the architecture is shaped the
> way it is — nothing in it is open work. For current status, Part I's section stamps are the
> truth.

## §21 MVP definition and milestone plan

> [!NOTE]
> **Outcome: achieved and exceeded.** Every acceptance item in [§21.1](#211-mvp-acceptance-verbatim-goal) passes today, and the
> M9+ roadmap items shipped too — lists, tabs, navigation, XAML launch parity, plus systems
> the plan never named (parts, tweaks, menus, dialogs, focus, gradients, OHOS, the agent
> tooling). The walkthrough grew from the planned 13 steps to 200+. The [§21.3](#213-performance-budget-asserted-in-ci-from-m5) performance
> budget was **not** wired into CI (no frame-time assertions exist); it remains an aspiration.

### §21.1 MVP acceptance (verbatim goal)

On the current macOS host: `day launch -p macos-appkit -p macos-gtk -p macos-qt -p ios-uikit -p
android-mdc` builds and launches the **showcase** app on all five targets; `day launch -p
ios-uikit --locale fr-FR --script scripts/walkthrough.yaml` runs the localized walkthrough,
passes its assertions, and produces screenshots; `day new` scaffolds a working project;
`day lint` reports fluent/a11y findings; `day pack -p macos-appkit` emits a `.dmg` and
`-p android-mdc` an `.apk`; canvas renders the gauge demo natively on all five; **the showcase
includes an externally-registered tier-1 piece (`day-piece-combobox`) on all five targets**
(pillar 4 is demonstrated, not deferred — DP-21). Showcase pieces: `column`, `row`, `label`,
`button`, `toggle`, `text_field`, `slider`, `canvas`, `when`, `each`, `scroll`, `spacer`,
`divider`, `image`, `combo_box` — with state, localization (en/fr + en-XA/ar-XB), a11y
annotations, and ids throughout.

### §21.2 Milestones (each lands green CI + tests; forward dependencies eliminated)

| # | scope | acceptance |
|---|---|---|
| M0 | workspace bootstrap; `day-reactive` (scoped signals/memos/effects/bind/watch/Setter, fixpoint drain, batching); `day-geometry`; `day-spec` v0; `day-mock` | unit/property tests: graph semantics, disposal-during-drain, disposed-handle rules, `Signal: !Send` compile-fail, setter-after-dispose, reentrancy (synthetic echo), batching |
| M1 | `day-core`: build-once mounter, realized tree, layout engine (`column`/`row`/leaf protocol, measurement cache, RTL flip, boundary re-entry), event routing; `label`+`button`+`column`+`row`+`divider` on mock; `IntoText` compile-pass suite | e2e-on-mock: counter updates exactly-one-op per click **and bounded measure-call counts** (op-log golden tests — the fine-grained guarantee is a *test*); sibling-re-proposal relayout test |
| M2 | `day-appkit` + desktop `launch()` + **default main menu** (Cmd+C/V/X/A via responder-chain selectors, Cmd+Q — NSTextField editing is broken without it); pieces: toggle/slider/text_field/spacer/scroll/when/each; styling core + `PerTarget`; `snapshot_window`; showcase v0 | manual + screenshot verification on host; **manual Japanese-IME smoke** |
| M3 | `day-gtk`, `day-qt` (host macOS; C++ shim per pane) incl. `snapshot_window`; **`day-piece-combobox` tier-1 renderers (appkit/gtk/qt)**; showcase parity across 3 desktop toolkits | side-by-side screenshots; external piece renders on all 3 |
| M4 | CLI v0: `new`/`build`/`launch` (desktop targets), Day.toml + day-meta, templates, doctor-lite, JSON events (hello/log/result), cancellation | `day new app t && cd t && day launch` works |
| M5 | mobile: `day-uikit` + ios scaffold (VC-hosted root, xcode-backend callback, sandboxing off) + simctl pipeline; `day-android` + gradle scaffold (fragment-hosted root, DayRustBuildTask, configChanges) + adb pipeline; **assets/locales resource conveyance ([§18.1](#181-data-resources-lands-in-m5-with-the-scaffolds--fluent-m6-and-the-walkthrough-m7-depend-on-it))**; safe-area/keyboard insets ([§7.7](#77-safe-areas-insets-and-the-keyboard)); combobox uikit/android renderers | showcase on Simulator + emulator via `day launch`; wrapping-label reflow test; **manual keyboard + iOS IME smokes** |
| M6 | `day-fluent` (tr/locale signal/negotiation/ICU4X functions/en-XA/ar-XB), a11y props + lint v0, ids, per-target locale plumbing | live locale switch + relayout benchmark; VoiceOver smoke on appkit/uikit; **`a11y_audit` green on apple targets**; fr/de number-format conformance |
| M7 | dayscript: engine, rendezvous/transport, `day script`, screenshots, JUnit; `--locale`/`--script` on launch | walkthrough (showcase-v1, no gauge step yet) passes on all 5 targets locally |
| M8a | canvas: `Draw`/DrawOp/replay on all 5 (PangoCairo/CoreText/QPainter/minikin/DirectWrite text); gauge joins showcase + walkthrough screenshot step | gauge renders natively on all 5; mock display-list tests |
| M8b | `image` piece + [§18.2](#182-icons-and-images) icons pipeline (resvg pre-render, per-platform icon matrix) | image/icons on all 5 |
| M8c | `sign` v0 + `pack` (dmg / apk / zipped sim-.app); lint v1; site + CI complete | **MVP acceptance [§21.1](#211-mvp-acceptance-verbatim-goal)** |
| M9+ | list (native recycling), battery (first dayffi tier-2 proof), xaml launch parity, `day upgrade`, webview→lottie→richtext, grid/tabs/nav (native containers per resolved DP-23), `day daemon`, real-device iOS, web-html experiment | — |

Sequencing rationale: mock-first (M0–M1) makes the fine-grained-invalidation claim a regression
test before any native code exists; AppKit before GTK/Qt because objc2 is the fastest loop on the
host; CLI before mobile because mobile *is* orchestration; `snapshot_window` lands with each
backend's milestone (M2/M3/M5), never as a retrofit; assets land with the scaffolds (M5) because
Fluent (M6) and the walkthrough (M7) read packaged resources.

### §21.3 Performance budget (asserted in CI from M5)

- Cold start to first frame: **< 400 ms** on a mid-range Android emulator profile.
- 60 fps slider-drag and typing on all five MVP targets (manual verification + frame-time logging
  in debug HUD).
- Layout pass wall-time budget for the ~500-node showcase on day-mock (regression-tracked).
- Release binary size delta over a toolkit baseline app: tracked per target with a budget set at
  M5 (Rust dylib + Day is expected to be single-digit MB; the number is measured, not promised).

---

## §22 Decision points for review

> [!NOTE]
> **Outcome notes (2026-07):** every DP is closed by implementation. Where reality diverged
> from the recommendation: **DP-3/DP-4** — dayffi and its bindgen never happened; the C ABI
> was superseded by `[package.metadata.day.*]` + `Event::Custom{tag,num,text}` ([§15](#15-extensibility-pieces-parts-and-tweaks)). **DP-8**
> — the web experiment never ran; no web backend exists. **DP-9** — lists later shipped ([§10](#10-native-list-integration)).
> **DP-10** — doctor shipped; `clean`/`config` did not. **DP-22** — piece-internal
> scriptability was never needed; dayscript drives everything through Day ids. **DP-24** —
> crates.io publishing is wired but not yet executed ([§1](#1-glossary-and-naming)). **DP-25** — Android process-death
> restoration remains cold-restart, as accepted. The table is preserved as written.

Each had a recommendation. **DP-16 (row contract) and DP-23 (navigation) were resolved**
(owner-ratified 2026-07-01; resolutions folded into [§5.4](#54-keyed-collections-each)/[§10](#10-native-list-integration)). The rest resolved through
implementation as noted above.

| # | question | options | recommendation |
|---|---|---|---|
| DP-1 | ~~crates.io naming~~ | — | **superseded by DP-24** |
| DP-2 | style variation surface | `per_toolkit()` values + `style_on` (as specced) vs. only plain `match` | as specced ([§6.2](#62-per-target-variation-pertargett-values-no-macros)) |
| DP-3 | dayffi payloads | `DayValue` tagged union vs. serialized (JSON/postcard) | `DayValue`, with the JNI packed-frame exception ([§15.3](#153-dayffi-the-c-abi-superseded--never-built)) |
| DP-4 | `day bindgen` codegen for polyglot stubs | v1 hand-written conventions vs. generator in MVP | hand-written v1; generator M9+ |
| DP-5 | iOS/macOS project generation | checked-in template `.xcodeproj` (flutter-style) vs. xcodegen/tuist dependency | template (no extra toolchain; scaffold-version handshake [§17.3](#173-daytoml) covers evolution); revisit if pbxproj churn hurts |
| DP-6 | Windows installer | `.msix` primary + `.msi` (WiX) optional vs. msi-only | msix primary, msi optional; note Azure Trusted Signing onboarding constraints (individual/org verification, subscription) affect who can sign — [§16.5](#165-subcommands)'s provider enum keeps alternatives open |
| DP-7 | bundling GTK/Qt into macOS/Windows apps for `pack` | support post-MVP vs. never (dev-only combos) | post-MVP support for qt (windeployqt/macdeployqt exist; **LGPL-3** obligations enforced by pack, [§16.5](#165-subcommands)); gtk (**LGPL-2.1+**, different obligations) stays dev-only until demand |
| DP-8 | web-html layout strategy | day-absolute-positioning (as specced) vs. hybrid with browser flow | start absolute + native `scroll`; evaluate hybrid in the experiment — *outcome (2026-07): `day-dom` shipped exactly this hybrid ([§9](#9-the-eight-toolkits-and-the-extra-combinations), [docs/web.md](docs/web.md))* |
| DP-9 | `list` excluded from MVP | confirm | confirm (spec hooks reserved, [§10](#10-native-list-integration)) |
| DP-10 | extra subcommands `doctor`/`clean`/`config` | approve / reject | approve (five toolchains make doctor indispensable; config is where doctor's fixes land) |
| DP-11 | layout engine | own SwiftUI-model engine (as specced, now with measurement cache [§7.4](#74-incremental-relayout-and-the-measurement-cache)) vs. Taffy | own engine (native height-for-width measurement + proposal negotiation don't fit Taffy; hop/pane heritage de-risks it) |
| DP-12 | cross-thread signals | main-thread-only + `Setter`/`on_main` (as specced) vs. floem-style sync storage | main-thread-only v1 |
| DP-13 | dayscript event injection level | day-event synthesis (as specced) vs. native input synthesis | day-event v1; native injection as later additive step tier ([Appendix C](#appendix-c--dayscript-reference-v1)) |
| DP-14 | ~~YAML crate~~ | — | **resolved: `serde_norway`** wrapped in a shared `day_yaml` module ([§16.2](#162-crate-choices)) |
| DP-16 | ~~row contract unification~~ | — | **resolved (owner, 2026-07-01): unified.** `each` and `list` share the `ItemSlot<T>` contract ([§5.4](#54-keyed-collections-each), [§10.1](#101-api--the-shared-itemslot-contract-unified-with-each--dp-16-resolved)) — one row function serves both, same-key value changes propagate automatically (`each_diff` dropped as subsumed); validated on day-mock in M1–M2 |
| DP-17 | flush scheduling: when does the reactive drain run? | (A) synchronous fixpoint drain at batch end; layout in a coalesced posted callback (as specced [§3.3](#33-threading-model-and-the-turn-state-machine)); (B) always-posted flush (pane's literal model — simpler reentrancy, +1 turn latency on every event, fuzzier `wait_idle`) | **A** (already specced; this DP records the ratification) |
| DP-18 | reading a disposed signal in **release** builds | (A) panic (floem/pane precedent, fail-fast); (B) log-once + default via try-path (leptos's panic-on-read is a notorious production footgun) | **A**, paired with the `try_*` doctrine of [§4.3](#43-scopes-and-disposal) — silent defaults hide real bugs and the no-op-write rule already covers legitimate async races |
| DP-19 | Qt list recycling (QListView recycles delegate *paintings*, not live QWidget rows; `setIndexWidget` is unvirtualized) | (A) day-side emulated recycling: QAbstractScrollArea host + pooled cell QWidgets behind the same RowHost protocol, reported `Support::Emulated`; (B) painted `QStyledItemDelegate` fast path, rows restricted to text/icon/accessory | **A** default (preserves "any piece is a row"), B as a later optimization |
| DP-20 | SwiftPM piece halves on cargo-driven targets (macos-appkit/gtk/qt have no Xcode project) | (A) `swift build` on `DayGeneratedPieces` + generated linker-args file into the cargo link (xcodebuild becomes ios-only); (B) promote macOS to a real Xcode project (kills the seconds-fast desktop loop) | **A** (already folded into [§16.5](#165-subcommands); needed for macos-gtk/qt regardless). **Outcome (2026-08):** both, in the end — macos-appkit gained the `platform/macos/` Xcode host project and is now dual-mode ([§16.5](#165-subcommands)), while A survives as the bare-cargo path (`DAY_MACOS_XCODE=0`, or no scaffold) and is still the only answer for macos-gtk/qt. The feared cost of B was real and is why the escape hatch exists: CI capture loops take the cargo path for speed. **Outcome (2026-08, later):** B alone — the bare-cargo macos-appkit path, its escape hatch, and the prepass were retired once every checkout carried the scaffold and CI proved the xcodebuild loop fast enough in practice; macos-appkit is single-mode ([§16.5](#165-subcommands)). macos-gtk/qt never used the prepass (the Swift halves are appkit-only), so nothing remains of A |
| DP-21 | extensibility (pillar 4) in the MVP | (A) tier-1 combobox joins MVP acceptance; battery/dayffi defer to M9; scaffolds ship the (empty) generated-aggregator attachment points from M5; (B) A + one thin tier-2 slice (battery, apple+android) in M8 to force dayffi real before templates ossify (~1 milestone-week); (C) confirm full M9+ deferral and say pillar 4 ships unproven | **A** (folded into [§21](#21-mvp-definition-and-milestone-plan)), with **B as stretch** if schedule allows |
| DP-22 | scriptability of tier-2/adopted-native piece *internals* (a ComboBox popup or WebView content is one opaque handle to the element index) | (A) optional `script_query`/`script_act` dayffi vtable entries + sub-element locator syntax (`stations-combo#item:3`) — additive, keeps pillar composition true; (B) scope the claim to root nodes + exposed props, with capability-flagged structured errors | **A** for MVP-adjacent pieces (ComboBox); at minimum [§2](#2-the-four-pillars)'s claim stays scoped as now written |
| DP-23 | ~~navigation architecture~~ | — | **resolved (owner, 2026-07-01): native containers.** `nav_stack` = UINavigationController / fragment+predictive-back hosts, desktop day-composed with native-style transitions ([§10.5](#105-navigation-and-presentation)); prerequisites already in place — VC/fragment-hosted roots ([§17.1](#171-project-layout-day-new-output)) + reserved push/pop/present hooks ([§10.5](#105-navigation-and-presentation)) |
| DP-24 | crates.io namespace (no namespacing on crates.io — RFC 3243 unshipped; `day-*` names are squattable once public; "Day" fights day.js for SEO) | (A) umbrella `day` + `day-*` crates; (B) umbrella `dayui` (SEO hedge), binary/brand stay `day`. Either way, reservation timing is its own call | **deferred by owner directive (2026-07-01): no crates.io reservation now.** Nothing in the MVP requires publishing (workspace + git deps); revisit naming + reservation together before anything is published or the design circulates publicly |
| DP-25 | Android process-death restoration (`onSaveInstanceState`) | (A) v1: documented cold restart; post-MVP opt-in persisted signals (`Signal::persist("key")`) into the state bundle; (B) design persistence into day-spec v1 now | **A** — the [§9](#9-the-eight-toolkits-and-the-extra-combinations) configChanges opt-out covers the common recreation triggers; cold restart is honest for v1 and `Signal::persist` is additive later |

---

## §23 Risks

> [!NOTE]
> **Outcome notes:** the two engineering risks the review weighted most — incremental relayout
> with no ancestor implementation, and native list integration — both landed (the measure
> cache and boundary re-entry live in day-core/src/layout.rs; lists in [§10](#10-native-list-integration)). The linkme/LTO
> gamble has not bitten in release builds. The M8c-density worry proved right in spirit —
> packaging absorbed the most iteration of any subsystem (flatpak icon policy, XAML installer,
> OHOS hvigor). "No hot reload" stands, mitigated exactly as described ([§16.9](#169-the-inner-loop-no-hot-reload--the-honest-story)).

| risk | mitigation |
|---|---|
| **Scope breadth** (5 targets × 4 pillars × CLI) | milestone gating ([§21.2](#212-milestones-each-lands-green-ci--tests-forward-dependencies-eliminated)); mock-first regression tests; MVP-adjacent tiers explicitly deferred |
| **Incremental relayout is spec-sound but ancestor-unproven** (hop re-runs its whole engine; pane relayouts the whole tree — nobody in the lineage has implemented boundary re-entry + measure cache) | M1's measure-call-count and sibling-re-proposal mock tests are the gate; [§21.3](#213-performance-budget-asserted-in-ci-from-m5) wall-time budget regression-tracked |
| Native list integration proves toolkit-hostile | deferred to M9 with spec hooks pre-reserved ([§10.2](#102-realization-the-rowhost-protocol) completed protocol); `scroll`+`each` is the honest fallback; Qt emulation is DP-19 |
| GTK a11y off-Linux, GTK bundling | secondary-combination framing ([§9](#9-the-eight-toolkits-and-the-extra-combinations), [§13](#13-accessibility)); doctor probes AccessKit; DP-7 |
| Qt licensing (LGPL-3) | pack-enforced guard rails ([§16.5](#165-subcommands)): dynamic linking pinned, store/static configs require commercial attestation, LGPL texts + source offer bundled |
| `build-once` model surprises (signal-read-outside-binding) | **runtime debug diagnostic** (once-per-callsite, `#[track_caller]`) is the sound mechanism; lint heuristic is advisory ([§4.1](#41-the-model-build-once-bind-forever)); mock tests encode idioms |
| linkme dead-strip under iOS release+LTO | layered registration ([§8.2](#82-the-open-renderer-registry): generated registrant + all-profile required-kinds check) + a dedicated release+LTO CI leg — mitigated, but inherently a link-time gamble until that leg exists |
| Toolchain drift (JDK/AGP/Xcode/NDK/Gradle config-cache) | `day doctor` with pinned known-good matrices; CI encodes them ([§20](#20-continuous-integration), [§20.5](#205-toolchain-and-dependency-governance)); scaffold-version handshake ([§17.3](#173-daytoml)) |
| dayffi ABI lock-in | versioned vtables + written evolution policy ([§15.3](#153-dayffi-the-c-abi-superseded--never-built)): [min,max] negotiation at registration, append-only growth, v1-pinned piece-ci cell |
| Fluent runtime cost per binding | per-locale parsed-bundle cache + per-binding resolved-message capture (no (key,args) memo — args contain `f64`); ICU4X function cost measured in M6 |
| **M8c density** (sign+pack+lint+site+CI in one gate) | already split from canvas/image (M8a/M8b); if it slips, the MVP claim slips with it — watch this milestone first |
| No hot reload disappoints Flutter refugees | honest positioning + script-replay inner loop ([§16.9](#169-the-inner-loop-no-hot-reload--the-honest-story)); research item tracked |
| dayscript blind spots (keyboard, IME, native hit-testing, animations) | [§14.2](#142-the-embedded-engine) says so explicitly; manual smokes in M2/M5/M6 acceptance carry that load |

---

## §24 Adversarial review findings and resolutions

> [!NOTE]
> Historical record of the pre-implementation review. The "accepted resolutions" below were
> folded into Part I's sections before implementation began; where implementation later
> diverged (dayffi, day-script-proto, the CLI's error framework), Part I's status stamps are
> the record.

**Round 1 (2026-07-01):** 8 parallel reviewers (reactivity, layout-lists, polyglot-ffi, cli-build,
pillars, mvp-audit, ecosystem, architecture) produced **119 findings**: 12 blockers, 74 majors,
33 minors. After cross-lens dedupe (~15 merges — list-row contract ×3, RTL ×3, lint-soundness ×4,
dayscript transport ×2, scaffold handshake ×2, cargo-standalone ×2, registrant/aggregator ×2,
keyboard/safe-area ×2, .ipa ×2, target-dir ×2), 1 finding was dropped (naming bikeshed) and the
rest were accepted as ~75 edits — **all folded into the sections above** — plus **10 new decision
points (DP-16–DP-25)**.

### Blockers and accepted resolutions

- **[§10](#10-native-list-integration) list contradiction** (by-value row builder vs recycling — 3 lenses converged): rows build
  against a Copy slot handle with per-field projections; `SignalRw` for two-way controls;
  `row_kind` for reuse identifiers. `each`-unification → DP-16 (since resolved: unified as
  `ItemSlot<T>`, [§5.4](#54-keyed-collections-each)).
- **!Send hole in the async story** (`on_main(move || sig.set(v))` could never compile): `Send`
  `Setter<T>` write-handle with liveness check; `Resource` rebuilt on it (leptos two-closure
  shape, tracked source, latest-wins).
- **Flush semantics**: "once per turn in dependency order" replaced by a fixpoint drain + re-run
  cap + (priority, scope-depth, seq) ordering + `watch()`; one scheduling state machine (sync
  drain at batch end, posted layout turn — DP-17); enqueue-only event-sink reentrancy contract;
  disposal/release-queue and disposed-handle rules (DP-18).
- **Scroll had no protocol** despite being M2 and the showcase root:
  `set_scroll_content`/`ScrollChanged`/`scroll_to`/`scroll_offset` added to day-spec v1 with the
  per-toolkit mapping; v1 nesting restrictions linted.
- **Android Activity recreation** (rotation/dark-mode/locale destroys a build-once tree's
  jobjects): configChanges opt-out + late-bound theme tokens + lifecycle hooks in spec v1;
  process-death → DP-25.
- **dayffi**: full ownership/borrowing contract (opaque DayValue, `day_value_*` single-allocator
  API, fixed `command` out-param); Flutter-style generated registrants (a fixed
  `day_register_pieces` symbol was a guaranteed duplicate-symbol link failure on static iOS);
  generated aggregator packages (`DayGeneratedPieces` / `day-pieces.gradle.kts`) instead of
  pbxproj mutation and `includeBuild`.
- **CLI/scaffold**: `ENABLE_USER_SCRIPT_SANDBOXING=NO` + `alwaysOutOfDate` in the pbxproj template
  (Xcode 15+ blocks the callback otherwise); dayscript port-0 handshake files +
  bind-only-when-invited (five parallel targets collide on loopback; the engine never listens
  uninvited).
- **Localization plumbing**: iOS `.lproj` conveyance via a copy-into-bundle phase + injected
  `CFBundleLocalizations` (a static template cannot reference user-defined lproj groups);
  assets/locales packaging pulled from M8 to M5 (M6 Fluent and the M7 walkthrough depended on it —
  the plan's "no forward references" claim was false as written).

### Notable majors (accepted; details in their sections)

Runtime debug diagnostic replaces the unsoundly-promised static signal-read lint ([§4.1](#41-the-model-build-once-bind-forever));
measurement cache + corrected boundary/sibling-re-proposal relayout rules ([§7.4](#74-incremental-relayout-and-the-measurement-cache)); min-size from
`Proposal(0,0)` not unconstrained — the hop shrink lesson at window level ([§7.5](#75-window-sizing)); safe-area/
keyboard-inset policy — API-35 edge-to-edge bites at M5 ([§7.7](#77-safe-areas-insets-and-the-keyboard)); RTL threaded through the M1 engine
with `ar-XB` ([§7.8](#78-rtl-and-bidi)); RowHost completion — `flush_now` on bind, `row_size_invalidated`,
move-lowering ([§10.2](#102-realization-the-rowhost-protocol)); IME-safe controlled inputs — origin-tagged writes + composition gating;
pane never proved CJK ([§4.4](#44-events-and-controlled-inputs)); AppKit default menu bar — Cmd+C/V/Q were broken in the flagship demo
(M2); navigation section + reserved presentation hooks ([§10.5](#105-navigation-and-presentation), DP-23); `AppCx::create_window`
reshape before the spec freeze ([§8.1](#81-the-toolkit-trait)); animation `AnimSpec` parameter reserved ([§8.4](#84-animation-reserved-hooks--still-unimplemented));
panic/catch_unwind policy ([§8.5](#85-panics-and-crashes)); per-toolkit a11y-id truth table — Android `setTag` is invisible
to automation, `uniqueId` is API 33+ ([§13](#13-accessibility)); native-tree `a11y_audit` step — nothing previously
verified `set_a11y` landed ([§14.2](#142-the-embedded-engine)); dayscript step tiers + actionability preconditions — no more
green taps on disabled/occluded elements ([Appendix C](#appendix-c--dayscript-reference-v1)); dayffi threading/async-command/
ABI-negotiation/JNI-packed-frame ([§15.3](#153-dayffi-the-c-abi-superseded--never-built)); piece.yaml re-keyed by target selectors ([§15.2](#152-package-layout-and-aggregation));
arg-less `day xcode-backend` + configuration-cache-safe Gradle task + conveyance-drift detection +
per-target `CARGO_TARGET_DIR` + scaffold-version handshake ([§16](#16-the-day-cli)–[§17](#17-the-conventional-day-project-and-daytoml)); NDJSON hello/protocol
version ([§16.3](#163-global-contract-every-subcommand)); CI-realistic signing — notarytool API-key auth, Windows HSM provider enum,
WinAppSDK bootstrap, fork-PR no-notarize split ([§16.5](#165-subcommands), [§20](#20-continuous-integration)); Fluent `NUMBER`/`DATETIME` via ICU4X —
fluent-rs registers none by default, so French numbers rendered wrong as originally specced
([§12.2](#122-api)); MSRV/cargo-deny governance ([§20.5](#205-toolchain-and-dependency-governance)); Qt-LGPL pack guards + THIRD-PARTY-NOTICES stage
([§16.5](#165-subcommands)).

### Dropped

- Naming/ergonomics polish (`piece_dyn`→`dyn_piece`, crate renames): bikeshed against an already-
  coherent convention with real churn cost; the one substantive item (`.any()` on `Decorate`) was
  folded into [§5.1](#51-authoring-surface-functions-and-builders-no-macros) instead.

### Remaining risks (carried into §23)

The incremental-relayout algorithm now has a sound spec but no ancestor implemented it — M1's
op-count and wall-time mock tests are the gate. Emulated Qt list recycling (DP-19) and
piece-internal scriptability (DP-22) are accepted scope, not proven designs. linkme-under-LTO is
mitigated but remains a link-time gamble until the release+LTO CI leg exists. dayscript still
cannot see keyboards, IME, native hit-testing, or native animations — [§14.2](#142-the-embedded-engine) says so, and manual
smokes carry that load. M8c remains the densest single gate even after the M8 split.

---

## Addendum (2026-07-09) — Tweaks: per-toolkit configuration of built-in pieces

Adopted post-review (owner-ratified): **tweaks** amend [§15](#15-extensibility-pieces-parts-and-tweaks)'s tier ladder with a rung BELOW
composition — configuring the native widget behind an existing built-in piece, case by case,
without a new piece kind. A piece with a tweak applied is a **Tweaked Piece**. This supersedes
the earlier composition-only stance for built-ins: "call two extra methods on the real NSButton /
XAML Button" is a legitimate, supported need that a full tier-1 renderer over-serves.

Mechanism (implemented; [docs/tweaks.md](docs/tweaks.md) is normative):
- `Toolkit::Handle: Clone + 'static`; the object-safe tree seam gains
  `node_handle_any(node) -> Option<Box<dyn Any>>` (a handle CLONE — retain / gobject ref /
  GlobalRef clone / Copy pointer). Toolkit `ext` modules downcast to their concrete handle.
- Portable surface: `Decorate::tweak(FnOnce(RNode))` (runs once at mount, post-realize — the
  [§17.4](#174-the-build-callback-flutters-pattern-exactly--including-the-details-flutter-learned-the-slow-way)/[§5.2](#52-the-piece-trait) synchronous-realize guarantee makes this sound), `Decorate::native_ref(&NativeRef)`
  (retained, liveness-checked, reactive on mount/clear transitions), and
  `day_core::invalidate_size(node)` for native mutations that change intrinsic size ([§7.4](#74-incremental-relayout-and-the-measurement-cache)'s
  measure cache cannot see mutations Day didn't make).
- Per-toolkit sugar: `.appkit(…)/.uikit(…)/.gtk(…)/.android(…)` typed ext traits;
  `.qt_raw(…)/.xaml_raw(…)/.arkui_raw(…)` raw tiers (the `windows` crate ships no
  Windows.UI.Xaml bindings, so XAML hands out the borrowed ABI pointer via the existing
  `day_xaml_unbox` seam; C++/WinRT recipes are the supported path).
- Native-class metadata (Level 1): every accessor also hands the closure the realized widget's
  concrete class name (`&str`), with no new trait method. Typed tiers read the live object's
  runtime class (objc `object_getClass`, GTK GType name), so a *conditional backing* — e.g. a
  plain `label` as `UILabel` vs a link-bearing one as `UITextView` — is reported accurately and a
  tweak branches instead of guessing a downcast. Raw tiers can't introspect the opaque pointer, so
  Day reads the node's kind off the same `node_kind` seam and maps it to the class it realized —
  the metadata a C++ tweak crosses the FFI with to guard its cast rather than blind-`static_cast`.
- Packaged tweaks: `tweaks/day-tweak-*` crates mirror piece crates' Cargo shape and reuse
  `[package.metadata.day.piece] backends` for [§15.2](#152-package-layout-and-aggregation)'s feature union. Three in-tree examples
  (button-bezel / tooltip / slider-tickmarks) span single-toolkit trivial to
  six-toolkit with crate-owned Qt/WinRT/ArkUI native code; the showcase Tweaks page exercises
  them in CI.
- Boundaries: main-thread only; never destroy/reparent; managed properties (title, value,
  enabled, frame, a11y) may be re-applied by Day and are NOT tweak-stable; unmanaged properties
  are. Packaged tweaks must document per-toolkit coverage and no-op silently elsewhere.

---

## Addendum (2026-08-22) — day-model: per-property observation

Adopted as phase 1 of the observation-and-persistence plan (owner-ratified in dialog, 2026-08-19
– 2026-08-22): a store whose writes wake only the readers of the field that changed, closing the
compute waste a coarse `Signal<Vec<T>>` pays on every keystroke. [docs/model.md](docs/model.md) is normative.

What shipped, and where:

- **`crates/day-model`** — `Store`/`Keyed`/`Elem`/`Field` over interned paths and lazily created
  `Trigger`s. Reads track the most specific path touched; writes notify that path and its
  ancestors. Triggers are refcounted by observing scope; interner slots by their triggers and
  children; both reclaim, and a stale `Copy` handle re-interns through its own chain on next use.
  The change log announces `(components, label, op)` per write, with prior/new values captured
  when a consumer asks — the persistence layer's input later, a headless test seam today.
- **`#[derive(Observable)]`** joined `build_path!` in day-macros (same no-syn construction):
  typed accessors on every `Source` of the struct, `Identified` from the always-explicit
  `#[obs(key)]`, `#[obs(skip)]` opt-out.
- **The two-way binding trait moved down** from day-pieces to day-reactive and took its shipped
  name in the same pass: `Binding`, with `read`/`write`/`peek` replacing the historical
  `get_rw`/`set_rw`/`get_untracked_rw` surface (no deprecated aliases — one sweep). A re-export
  stays in day-pieces; day-model implements it for `Field`, and every dependency points downward.
- **Six constructors widened** from `Signal<T>` to `impl Binding<T>`: `picker`, `text_area`,
  and the four external pieces (`date_picker`, `rating`, `color_picker`, `search_field`) —
  additive, so existing `Signal` call sites compile unchanged.
- **The facade** grew an off-by-default `model` feature; the prelude re-exports the API and the
  `day_model` crate name the derive's generated code resolves against.
- **The scaffold's editor** ([§17](#17-the-conventional-day-project-and-daytoml)) now binds each form control to a field accessor
  directly — the three-names-per-property plumbing (a `Signal`, a `watch` write-back, the
  control) collapsed to one — and the model file persists through one coarse `watch`.

Decisions recorded with their rationale in [docs/model.md](docs/model.md): deleted-row reads return `Default`
beside a tracked `exists()`; `Store::new` leaks and the handle stays `Copy` (the
`Signal::global` precedent — a scoped owner arrives with the persistence container);
announcement of background transactions stays an explicit `pump()` until that container exists.
`Signal<T>` is unchanged throughout, and an app can hold both. Part II of the plan
(day-persistence) shipped as its own addendum below; nothing here presumes it.

**Consolidation (2026-08-20).** Three follow-ons landed as one pass:

- **Claims mirror the reactive graph.** A day-model claim made inside a computation now belongs
  to that computation's RUN — released on its re-track or death via day-reactive's new
  `active_run`/`on_run_retrack` seam, exactly like `sources` bookkeeping — and a tracked read
  OUTSIDE any computation creates nothing at all (nothing could wake through it). This is what
  lets a recycled list cell rotate across a million rows and leave the observation tables where
  they began; it also closed two latent holes (re-run claims mis-attributed to the flusher's
  scope; build-time initial-value reads pinning triggers to long-lived scopes).
- **`each`/`list` take a `RowSource`** ([§5.4](#54-keyed-collections-each), [§10.1](#101-api--the-shared-itemslot-contract-unified-with-each--dp-16-resolved), [docs/list.md](docs/list.md)): plain data wrapped as
  `items(closure, key_of)`, or — feature `model` on day-pieces — a day-model store directly
  (`store.rows(projection)` for display order). Store rows receive a **`ModelSlot`**, itself a
  day-model `Source` (`DYNAMIC`, re-resolving its row per operation), so derive accessors bind
  two-way and follow the recycle; selection callbacks hand the row's `Elem`. An unchanged row
  set skips the native reload, so a field edit costs the one control it patched.
  `day-pieces/tests/model_rows.rs` measures the claims (one label patch per edit, zero reloads,
  zero residue across a full-collection scroll).
- **day-appkit's list borrows narrowed** (`list_entry` clone-out): six sites held the
  `LIST_STATE` map across table calls that can synchronously re-enter `viewForRow` → a flush →
  `release`, the contained "RefCell already borrowed" panic every list walkthrough logged, 24 a
  run. Now zero.

Adopters: the scaffold's list is store-driven end to end; Day-Time's alarm card converted (the
independent generalization check — including a hand-written `Binding` for one bit of a mask);
Day-Showcase gained a Model page with walkthrough coverage.

## Addendum (2026-08-22) — day-persistence: SQLite storage for the model

Adopted as phase 2 of the same plan: the change log, folded into SQL.
[docs/persistence.md](docs/persistence.md) is normative. The API vocabulary is deliberately
SwiftData's (`ModelContainer`, `@Model`-style declarations, delete rules); the STORAGE
strategy as first shipped was not — a container opened a database and loaded each model's
table into an ordinary `Store<Keyed<M>>`, the document pattern, where SwiftData faults rows
in on demand. (This paragraph originally attributed the load-everything shape to SwiftData;
corrected 2026-08-27 — SwiftData is built on Core Data's faulting and never loads a store
whole. The lazy-engine note below replaced the strategy itself.) The schema is visible
instead of hidden, and the engine a cargo feature instead of a linked fate.

What shipped, and where:

- **`crates/day-persistence`** — `ModelContainer` (open → migrate → load → watch): a standing
  change sink (day-model grew `install_change_sink`/`store_id` for it) marks rows dirty as
  changes announce; a turn's end flushes the fold in one transaction. Merge rules: same-row
  changes coalesce onto one `UPDATE`, an insert absorbs the edits that fill it, a delete absorbs
  everything, a wholesale `Store::update` resyncs its table. Rows then merge across each other
  where one statement carries them: same-table deletes become one `… WHERE id IN (?, …)`, and
  updates join when they set the same columns to the same values (the multi-selection edit).
  Different values, and multi-column keys like a join row's, keep their own statements; a batch
  past the bound-parameter limit chunks inside the same transaction. Row values are read from
  the store at flush time; the change log never carries contents. `record_sql` returns one
  flush's SQL — the headless assert ("twenty keystrokes, one `UPDATE`" is a test, not a slogan).
- **Drivers** — the object-safe `SqliteConnection` seam under a `SqliteDriver` trait. Built-ins:
  `Sqlite` (rusqlite on native targets; on web-dom the same type proxies statements to the
  day-sql worker's OPFS-backed engine, `crates/day-sqlite-worker` — [docs/persistence.md](docs/persistence.md) §The
  web; `bundled` default / `system` / `cipher` as engine features;
  `at`/`memory`/`app_data` — the last resolving day-part-fs' data-root rules under a `day-db/`
  leaf) and `Recorder` (fixture-answering, statement-logging, always compiled). `capabilities()`
  reports what is real per build.
- **Schema** — derived DDL is `STRICT` where the engine allows, readable by any tool, and
  bookkept in one `_day_schema` table (fingerprint + version); every other table in the file is
  left alone, so hand-made tables coexist. Fingerprint drift runs lightweight migration
  (add + backfill with the field's `Default`, drop); renames and recodes are staged
  `MigrationPlan` stages, ascending, transactional, with newer-than-this-build files refused.
- **`#[derive(Model)]`** in day-macros (same no-syn construction): implies `Observable`, adds
  the trait half — `TABLE`/`KEY`/`COLUMNS` (each column carrying its field name, which is what
  lets `#[model(column = …)]` renames meet the change log's field labels), row↔struct mappers,
  `NULL`-reads-as-`Default` leniency. `Observable` also accepts `#[model(…)]`, so a web build
  swaps only the derive line. Codecs: `ColumnValue` (one impl per type) and named
  `ValueCodec`s (`#[model(with = …)]`, `#[model(json)]`); day-piece-datetime ships
  `DayDate`/`DayTime` canonical `INTEGER` forms plus `Iso8601`/`EpochSeconds`/`EpochMillis`
  behind its `persistence` feature.
- **Cipher** — `.key(Secret)` (zeroed on drop, never stored by Day), `BadKey` at open,
  `rekey`/`encrypt_to`/`decrypt_to`; maintenance is `backup_to` (`VACUUM INTO`),
  `integrity_check`, `checkpoint`, `vacuum`, `size_bytes`.
- **The facade** — `persistence` implies `model`; `sqlite-system`/`sqlite-cipher` select
  engines; the prelude re-exports the API and the `day_persistence` crate name.

Adopter: Day-Showcase's Model page
runs on a container on every native target (web stays in memory), with insert/edit/delete and
the storage readout in the walkthrough.

**Phases 3–5 (2026-08-21).** Queries, the extensions, and undo landed as designed, one pass:

- **Queries** — predicates are DATA (`Pred`/`Fetch`, compiled to SQL once, evaluated in memory
  after), maintained by the ported `LiveSet`: a column the query never mentions costs zero
  evaluations, a predicate/sort column evaluates one row and emits `Insert`/`Remove`/`Move`
  deltas, windows re-derive. The derive emits `Trip::name()` column refs beside the binding
  accessors (no receiver, so the namespaces never collide); `container.query::<M>()` builds,
  `query_fn` re-derives from signals, `query_raw` re-runs per named-table flush,
  `with_connection` + `rescan` close the escape hatch. The 600-edit agreement test — and its
  undo-interleaved variant — pin the property that makes skipping the database safe.
- **Row deltas reached the toolkit line** — one new `ListPatch::Splice(Vec<RowDelta>)` with a
  reload fallback on every backend and true animated splices on appkit (insert/remove/moveRow)
  and uikit (performBatchUpdates-family); `RowConn::take_row_events` feeds it, so a
  query-backed `list` animates a row out instead of reloading (mock-asserted headlessly).
- **FTS5 + R*Tree** — struct-level `fts("a", "b")` / `spatial(lat, lon)` generate the
  external-content shadow table, the R*Tree, their `AFTER` triggers and first-create
  backfills; `matches`/`within`/`rank()` are typed predicates; capability checks refuse at
  open naming the missing module. Consequence worth recording: the fold's upsert became a true
  `ON CONFLICT DO UPDATE` — `INSERT OR REPLACE`'s implicit delete skips delete triggers unless
  `recursive_triggers` is on, and the FTS index would silently rot.
- **Sessions** — `Binding` grew `write_preview`/`write_commit` (defaulted to `write`); a
  day-model field implements them as a preview overlay: store updated, field triggers wake,
  nothing durable until commit seals ONE record whose prior predates the gesture. Sliders wire
  the pair from `ValueChanged`/`ValueCommitted`; text fields preview per keystroke and commit
  on Return/blur (the typing coalescer). Sixty previews = one unit, one UPDATE — asserted.
- **Undo** — `UndoStack` in day-model (persistence optional): units are turns, inverted from
  captured prior values (`restructure` now captures the deleted/inserted ROW when a values
  consumer stands); replay is author-tagged (`Change.author`) and applies through the
  derive-generated `ApplyField` seam; `container.undo(levels)` watches every store. The one
  day-spec touch shipped as designed: `UndoState` duty + `Event::Undo`/`BridgeKind::UndoInvoked
  = 28` + `Cap::UndoBridge` (§8 amendment; all three matrices regenerated). Native fronts: an
  `NSUndoManager` subclass answering canUndo/titles from mirrored state and forwarding
  invocations — the appkit window's `windowWillReturnUndoManager` and the uikit root VC's
  `undoManager` both yield it, so a focused text field's own manager keeps precedence (the
  typing rule, by construction). Transient UI state (a selection) rides the units via
  `set_transient_context`: captured as each unit seals, restored at the landing point of
  every undo/redo, never persisted ([docs/model.md](docs/model.md) "Transient UI state") —
  Day-Sketch wires its canvas selection through it.
- Also fixed en route: `Mapped` (`field.map(...)`) now carries the mapped field's LABEL into
  the change log — the literal "mapped" it wrote before was invisible to the SQL fold, so a
  converted binding over a container store silently never persisted.

Still deferred then: lazy faulting (landed 2026-08-27 — the lazy-engine note below),
external storage, session-suspend auto-commit and cross-window undo focus routing.

**Outcome (2026-08-22).** Cross-connection watching landed — not via the preupdate hook the
plan named (it reports only its own connection's writes and cannot see another process's), but
as `ModelContainer::check_external`: a `PRAGMA data_version` poll (the counter moves only when
another connection commits) followed by a per-table diff fed through a new
`Store::merge_row` seam — per-column announcements, authored `"database"`, dispatched to live
queries, declined by the autosave fold and by undo. The wasm driver leg had already landed via
day-sqlite-worker when the list above was written. day-lite's storage moved onto the shared
driver in the same change (`SqliteConnection` grew `execute_batch` and `query_named`), so a
superapp compiles one SQLite and the app's engine features govern miniapp storage too.

**Predicate vocabulary (2026-08-23).** `is_in`/`not_in` (sets sorted once at construction, so
membership is a binary search — the shape relation traversal will compile into), `IdIn` (the
row's own key, no column read at all), `starts_with`/`starts_with_ci` (deliberately not `LIKE`,
whose SQLite default is case-INsensitive for ASCII), and `is_null`/`is_not_null`/`is_set`/
`is_unset`/`is`/`is_one_of` as constructors over the existing `Eq(col, Null)` variant rather
than new arms. Two contracts came with them, both closing latent divergences between the
in-memory and SQL paths:

- **NULL follows SQL's three-valued logic in both paths.** A comparison against a NULL column is
  UNKNOWN, not false, and UNKNOWN propagates through `&`/`|`/`!` by Kleene's rules — so
  `ne`/`lt`/`between`/`contains` no longer select rows the SQL form would exclude.
  `compare_values` is untouched: NULL still sorts below numbers, because that is `ORDER BY`'s
  rule and ordering is a different question from comparison.
- **`Pred::sql_exact()`** says whether a predicate's SQL form would select the same rows its
  in-memory evaluation does; `to_sql` may only be used when it holds. Case-insensitive
  predicates answer false, because Rust folds case over all of Unicode while SQLite's `lower()`
  folds ASCII only — the `ÉCOLE` divergence recorded in the Room comparison, now named and
  guarded rather than latent.

`tests/predicates.rs` (26) covers the vocabulary, the three-valued table, the exactness flag and
the Unicode fold. (`sql_exact` itself was retired 2026-08-27: the lazy engine registers
`day_fold` — Rust's fold as a SQL function — so the case-insensitive forms compile exactly and
SQL became the one evaluation path; the three-valued contract carried over unchanged.)

**Predicates across relations (2026-08-25).** A query can now ask about a row's relatives:
`Trip::lodging().any(…)`, `.none(…)`, `.all(…)`, `.is_empty()`, `.count_ge(n)` — one quantifier
vocabulary over to-one, to-many, self-referential and many-to-many alike, because a to-one is a
to-many of at most one. The derive emits a receiver-less `Trip::lodging()` beside the instance
accessor, the same way `Trip::name()` sits beside `trip.name()`.

The point is that crossing a relation does not leave the incremental tier. `Deps` split into
`local` and `related` halves, so a related column the predicate never reads costs ZERO
evaluations exactly as a local one does; a column it does read resolves back through the
relation index — `parent_of` for a to-many, `holders_of` for a join, both O(1) — and
re-evaluates only the local rows that change could move, emitting row deltas rather than
re-deriving the set. `is_empty`/`count_ge` read no related row at all.

Maintenance is TWO phases around relation upkeep, because each half needs a different view of
the index: which local rows a related change can move is answered before it (a deleted child is
still filed under its parent), and whether they still match is answered after (a reparented
child must be under its new parent to be found there). Getting this wrong was the bug the
membership tests caught — single-phase dispatch evaluated against a stale index.

Two limits are declared rather than hidden: a relation inside a relation evaluates to any depth
but back-resolves only one hop (`Deps::deep` reports it, and such a fetch re-derives), and a
relation predicate is not `sql_exact` because its faithful SQL is a correlated `EXISTS` needing
the wiring's column names. `EvalCtx` replaced `RowsView` to carry the relation half of what an
evaluation can reach. `tests/relation_query.rs` (19) covers every shape and, more importantly,
the counting claims. (Superseded 2026-08-27: the lazy engine compiles exactly that correlated
`EXISTS`, nesting included, so the one-hop back-resolution limit and `Deps::deep` are gone —
the dependency gating survives as staleness marking.)

**Keys and relations (2026-08-23).** The two schema-shaping decisions, owner-ratified in
dialog and landed together:

- **Wide keys.** `#[obs(key)]`/`#[model(id)]` now take an integer, a `Uuid` (16-byte `BLOB`)
  or a `String` (`TEXT`), through a new `AsKey` trait. Integer keys are still their own path
  handle — no interner, no lock, nothing added to the hot path; wide keys intern
  process-globally (NOT thread-locally: a background transaction's reindex must mint the
  handles the main thread resolves) to a handle above a reserved top bit, so paths stay 12
  bytes and every collection index stays `u64`-keyed. `ModelId<M>` is the typed surface —
  `Copy`, opaque, 8 bytes — and `elem`/`ids`/query results/list slots all speak it, with
  `From` conversions so integer literals, `Uuid`s and `&str` keys all just work.
  `day_model::Uuid` re-exports `uuid::Uuid` and **v7 is the taught default** (time-ordered
  inserts, cross-device uniqueness — the sync groundwork); generation stays native-target
  only until the web pipeline's entropy import lands. Refusals rather than mis-service:
  `fts(…)`/`spatial(…)` need an integer key (both address rows by ROWID), and a key field
  takes no codec.
- **Relations, the full SwiftData-style vocabulary.** `One<M>` is the child's foreign-key
  column — the single source of truth — and `Many<M>` a marker field whose accessor reads an
  index the container maintains from the change log, which is how maintained inverses come out of the
  existing pipeline rather than parallel bookkeeping: writing either side wakes both, and
  `add` goes through the child's FK so it announces, captures for undo, folds to one
  statement, and animates live queries. Delete rules default to `nullify` (refused over a
  required reference, naming the fix), with `cascade` recursing through the same pipeline (one
  undo unit restores a whole subtree) and `deny` refusing through `container.delete`, the
  checked door. Generated DDL carries the matching `REFERENCES … ON DELETE …`, deferred, so
  another process honors the same rules. Ordered to-many keys a visible `f64` child field
  fractionally — a drag is ONE row, with an O(n) rebalance when a gap bisects away.
  Many-to-many generates the join table and keys its memberships by the PAIR
  (`Key::Pair`), so they fold, undo and merge through the same machinery every other row
  uses; declaring it on both models yields one relation with two views, and a join cascade
  takes only the rows no other row still holds. `Model::RELATIONS` exposes it all as data.

## Addendum (2026-08-27) — day-persistence: the lazy engine

The load-everything container was replaced wholesale; [docs/persistence.md](docs/persistence.md)
remains normative and now describes only this engine. The trigger was the Day-News redesign
surfacing that `ModelContainer::open` read every row of every table (`SELECT {cols} FROM {t}`
per model) and that queries evaluated in memory over the loaded rows — fine for a sketch
document, wrong for any store that grows. Breaking changes were accepted deliberately; the
in-repo adopters migrated in the same change.

What changed, and what it replaced:

- **Open reads no rows.** `attach` creates/migrates the table and registers hooks; every
  `Store<Keyed<M>>` starts empty and is renamed at the API — `container.cache::<M>()`, because
  `store()` implied "the table" and its `keys()` no longer are. Rows FAULT in: `get(id)`,
  `ensure_resident(&keys)` (one chunked `SELECT`), or a list materializing the window it binds
  (`Query::materialize(range)`; the `list(query, …)` glue faults around the bound row). The
  cache is bounded (`set_cache_limit`, default 8192/model): eviction spares dirty and observed
  rows (day-model grew `populate`/`depopulate`/`is_observed` — silent cache traffic, no
  announcements), and a row deleted this turn never resurrects through a fault.
- **Queries compile whole.** Predicate → WHERE (relation crossings as correlated `EXISTS` at
  any depth, joins through the join table, FTS as a shadow-table subquery, `rank()` as a join,
  `within` narrowed through the R*Tree then re-checked exactly), sorts → ORDER BY + key
  tie-break, limit → LIMIT. Case-insensitive predicates stay EXACT in SQL: the native driver
  registers `day_fold` (Rust's full-Unicode `to_lowercase`) at open —
  `Capabilities::unicode_fold`; a driver without it (the web engine, for now) takes a fallback
  that SQL-filters the exact conjuncts and re-checks the fold over the selected dependency
  columns. `sql_exact`, `evaluable`, `EvalCtx`, `OneRow` and `LiveSet` are gone.
- **Liveness = staleness + one requery + a verified diff.** The change sink marks a query
  stale only when a change touches its dependency set (own columns, relation fk/order fields,
  related columns, join-store membership); ONE requery after the turn's flush re-derives the
  ids, and `ResultSet::adopt` diffs old→new into `Insert`/`Remove`/`Move` deltas, verified by
  simulation before delivery (an un-narratable change reloads honestly). Every read
  (`ids`, `count`, `take_events`, the untracked forms) settles staleness first, so read-your-
  writes holds; with autosave off, queries answer from the last save, documented. The 600-edit
  agreement tests were rewritten to mirror the delta feed and still pin id-for-id agreement.
- **Relations became lazy views.** The eager per-relation indexes (seeded O(n) at open) are
  per-parent memos over one indexed `SELECT`, overlaid with the turn's unflushed dirty rows so
  mid-turn reads are coherent without flushing; join membership stores start empty. Writes
  materialize their rows first; cascades walk children from the same indexed SELECT, fold to
  chunked deletes, materialize only when an undo stack is installed, and the engine's
  `ON DELETE` clauses backstop rows this process never faulted. A wholesale `Store::update`
  now upserts the RESIDENT rows only — a working set cannot infer deletions from absence, so
  emptying a table became an explicit act (the one deliberate behavior break).
- **External checks are O(working set).** `check_external`/`rescan` re-select only the
  resident keys, diff per column under the `"database"` author, and re-run every query;
  arrivals surface through the queries and fault like any row.
- Fixed en route: raw queries never re-ran on inserts (`statement_touches` predated the
  upsert form), and empty `NOT IN ()` compiles to `IS NOT NULL` so both paths keep the
  three-valued reading.

Adopters in the same change: Day-Showcase's Query page now caps its cache at 2,048 under the
10,000-row table and shows a residency readout where the evaluation counter was; its Model
page clears the demo file explicitly before reseeding; Day-Sketch (a document app) lifts the
cache bound and `warm`s its tables at open — the load-at-open shape, by choice. `tests/lazy.rs`
pins the contract (no row SELECT at open, batch faulting, bounded cache, eviction exemptions,
no zombie faults); the full suite runs 19 green targets.

One cliff found by benchmarking and fixed in the same change: eviction cost O(cache) PER
EVICTED ROW (`Keyed::remove` rebuilds its key map per call), which made a 50k-row relation
traversal over a 500k-row table take 8.3s at the default cache bound. day-model grew
`depopulate_many` (one retain, one reindex) and the enforcement pass gained hysteresis
(slack before it runs, then one batched pass to the limit); the same traversal is 68ms —
level with an unbounded cache. Day-Bench gained the measuring apparatus: `persist/` +
`versus/swiftdata/`, one schema and phase set over both engines
(`scripts/compare-persistence.sh`).

---

# Appendix A — The showcase app, end to end

> [!WARNING]
> **Status: superseded by the live app.** The design-era single-page sketch this appendix
> carried is long outgrown — **daybrite/Day-Showcase is the reference**, and it is deliberately
> self-documenting: every page's source comments name the docs/ file and DESIGN section it
> demonstrates. It moved out of this repository in 2026-08 (§20); CI checks it out to keep testing
> the framework against it.

What the shipped showcase covers, per navigation destination (a `selector` sidebar on desktop,
a list-push on mobile — [docs/navigation.md](docs/navigation.md)): **Controls** (every two-way binding, pickers,
search, progress/activity), **Focus** (the [§4.4](#44-events-and-controlled-inputs)/[docs/focus.md](docs/focus.md) permutations), **Text**
(semantic styles, weights, custom fonts), **Canvas & shapes** (shape kinds, gradients, live
transforms + gestures, the gauge, composition-tier widgets), **List** (native recycling),
**Tabs**, **Stack** (push/pop bound to a path signal), **Media**, **Web View**, **Menus &
dialogs** (app menu, context menus, alert/confirm/prompt/sheet), **Device & sensors** and
**Platform services** (the `parts/`), **Resources** (bundled images/data, content modes),
**Tweaks**, **Map** (Apple targets), and **About** (live lifecycle readout).

Four locales ship (`en`, `fr`, `ar` — RTL, `zh-CN`); every user-facing string flows through
`res::str` typed keys. `dayscript/walkthrough.yaml` (200+ steps) navigates every destination,
exercises every control, and screenshots each page — it runs per locale and per theme in CI on
macOS (AppKit/GTK/Qt), iOS, and Android, and is the acceptance gate for backend changes.

### Run it

```
$ day launch -p macos-appkit -p macos-gtk -p macos-qt -p ios-uikit -p android-mdc
$ day launch -p ios-uikit --locale fr --script dayscript/walkthrough.yaml
$ day launch -p android-mdc --locale ar --script dayscript/walkthrough.yaml   # RTL pass
$ day launch -p macos-appkit --variant dark --env DAY_THEME=dark --script dayscript/walkthrough.yaml
```

---

# Appendix B — Extension examples

> [!NOTE]
> **Status: design-era sketches with shipped outcomes.** Each example below now exists in the
> repo; the outcome lines say what changed. [docs/extending.md](docs/extending.md) is the how-to.

### B.1 ComboBox (tier 1 — Rust renderers, the pane-combobox pattern)

> [!NOTE]
> **Shipped** as `pieces/day-piece-combobox`. The `ForeignPiece` prop-bag sketch became
> **typed props + the `renderer!` macro**. Reworked 2026-07 ([docs/combobox.md](docs/combobox.md)) from a
> selection-only dropdown into a real combo box — free-form text plus a dropdown, the text
> being the value — on every toolkit that has such a control (iOS and ArkUI do not; the piece
> carries no renderer there and day renders its placeholder leaf).

```rust
// pieces/day-piece-combobox/src/lib.rs (as shipped)
pub fn combo_box(items: Signal<Vec<String>>, text: Signal<String>) -> ComboBox { … }

// per-backend module, e.g. cfg(feature = "appkit"):
day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
// appkit → NSComboBox; gtk → GtkComboBoxText with entry; qt → editable QComboBox (own C++
// shim); android → AutoCompleteTextView (own Java factory); xaml → editable ComboBox (own
// C++/WinRT shim).
```

App usage: add the crate with the matching toolkit features. No edits to day.

### B.2 Battery (tier 2 — a *service*, polyglot, no UI)

```rust
// day-piece-battery/src/lib.rs
pub fn battery() -> BatteryHandle;             // BatteryHandle { pub level: Signal<f32>, pub charging: Signal<bool> }
```
> [!NOTE]
> **Shipped** as `parts/day-part-battery` — the first **part** ([docs/battery.md](docs/battery.md)). Per-OS Rust
> halves selected by `cfg(target_os)` (IOKit on Apple targets — including `macos-gtk`/`-qt`,
> exactly the selector case the design worried about; upower on Linux; `GetSystemPowerStatus`
> on Windows) plus a small Java shim staged via `[package.metadata.day.android]`. No dayffi:
> events re-enter through `Setter`/`on_main`, values are signals.

### B.3 WebView (tier 2 — complex: commands + events)

> [!NOTE]
> **Shipped** as `pieces/day-piece-webview` ([docs/webview.md](docs/webview.md)): WKWebView / android.webkit /
> WebKitGTK / QWebEngineView / WebView2 / ArkUI web, driven by tier-1 Rust renderers with C++
> shims where the toolkit needs one. Navigation events ride `Event::Custom`; the
> `evaluate_js(…).await`-over-dayffi design was not needed.

### B.4 Lottie (tier 2 — bridging famous native libraries)

> [!NOTE]
> **Shipped** as `pieces/day-piece-lottie` ([docs/lottie.md](docs/lottie.md)): lottie-ios via
> `[package.metadata.day.ios]` `swift-packages`, lottie-android via
> `[package.metadata.day.android]` `gradle-dependencies` — the exact third-party-coordinate
> flow this example was designed to prove, minus `piece.yaml` ([§15.2](#152-package-layout-and-aggregation)). `Cap::Lottie` gates
> support per toolkit.

### B.5 RichText (tier 2 — deep native control)

> [!NOTE]
> **Shipped** as `pieces/day-piece-texteditor` ([docs/texteditor.md](docs/texteditor.md)):
> `text_editor(Signal<StyledText>)` edits the same document a label renders and `.markdown()`
> produces, in each platform's own rich-text view. ALL EIGHT toolkits ship one — `NSTextView`,
> `UITextView`, `GtkTextView` over a tag table, `QTextEdit`, an `EditText` over its live span
> buffer, a `RichEditBox` driven through its TOM, the ArkTS `RichEditor`, and a `contenteditable`
> element — so the piece has no composed tier and must not have one: a hand-rolled editor loses
> IME, bidi, undo, dictation and the accessibility tree, invisibly.
>
> The design rule the arms are built on: **Day owns the attributes, the native view owns the
> characters.** An edit arrives as plain text, is diffed against the text the piece last knew, and
> the runs reflow over the delta — so the toolbar, the mixed-state read, the paragraph attributes
> and the import/export are pure Rust on all nine targets. Each platform's own formatting UI is
> turned off rather than read back, which [docs/texteditor.md](docs/texteditor.md) §3 states as
> the piece's cost.
>
> `TextRun` grew `background` and `underline`, `FontSpec` grew `scale` (a relative size, so a run
> still tracks the reader's text-size setting), and both reached all eight label paths. The
> document type, its paragraph runs and the Markdown / HTML / RTF codecs live in **day-spec**
> ([§8](#8-day-spec-the-contract)), not in the piece.

### B.6 PullRefresh (the reference CONTAINER piece — native/emulated hybrid)

> [!NOTE]
> **Shipped** as `pieces/day-piece-pullrefresh` ([docs/pullrefresh.md](docs/pullrefresh.md)): pull-to-refresh for any
> scrollable, and the first external piece whose native view **hosts a Day child** — proving the
> container seam needs no framework hook (day-core mounts children by handle; the piece supplies
> a fill layout via `cx.native` + `cx.under` — [docs/extending.md](docs/extending.md) §5). Per toolkit it is a hybrid:
> NATIVE wrappers where the platform has them (`UIRefreshControl` attached on subview-add,
> `SwipeRefreshLayout` via `[package.metadata.day.android]`, `ARKUI_NODE_REFRESH` via its own NDK
> shim — the first external ArkUI renderer), EMULATED elsewhere (a composed spinner overlay plus
> overscroll observation: AppKit elastic-scroll bounds notifications, GTK `edge-overshot`). The
> two-way `refreshing: Signal<bool>` contract mirrors SwiftUI's `refreshable`; dayscript drives it
> through the existing `toggle:` step (`Event::ToggleChanged` as synthetic begin/end).

### B.7 Date & time pickers (the first all-seven-toolkits external piece)

> [!NOTE]
> **Shipped** as `pieces/day-piece-datetime` ([docs/datepicker.md](docs/datepicker.md)): `date_picker(Signal<DayDate>)`
> and `time_picker(Signal<DayTime>)` — TWO pieces, because a combined date-time control exists on
> only 3 of the 7 toolkits while separate controls are native on all of them (combined = row
> composition). Two style intents (`Compact` = field → transient chooser, `Inline` = embedded
> calendar/wheels) map to `NSDatePicker`, `UIDatePicker`, Material dialogs launched through
> `DayActivity`'s FragmentManager, `QDateEdit`/`QCalendarWidget` (own shim), `CalendarDatePicker`/
> `CalendarView`/`TimePicker` (own shim), the ArkUI NDK picker nodes (own shim), and a
> GTK-composed `GtkMenuButton`+`GtkCalendar`/spin-button build (GTK4 has no stock picker —
> `support()` reports Emulated there). The first external piece covering EVERY backend's renderer
> slice. Values are civil/zoneless; controls are pinned to a Gregorian-UTC calendar with the
> user's locale, so platforms localize month names while the value never shifts by zone; dayscript
> drives every picker via the existing `input:` step (`Event::TextChanged` with ISO text).

### B.9 Color picker (the piece that is native OR composed, per target and per call)

> [!NOTE]
> **Shipped** as `pieces/day-piece-colorpicker` ([docs/colorpicker.md](docs/colorpicker.md)):
> `color_picker(Signal<Color>)`, a color well bound two-way, and the first piece to ship TWO idioms
> behind one API. `PickerIdiom::Native` realizes a leaf that six toolkits render with the system
> chooser (`NSColorWell` → the shared `NSColorPanel`, `UIColorWell` → the iOS picker,
> `GtkColorDialogButton`, a `QColorDialog` shim, the XAML `ColorPicker` in a flyout,
> `<input type="color">`); `PickerIdiom::Composed` builds the whole picker out of ordinary Day
> pieces and a canvas — a saturation/brightness field, a hue strip, an opacity strip and a preset
> palette in a [`cover`](docs/cover.md) — so it needs no renderer arm and behaves identically on all
> nine targets. `Automatic` (the default) picks native where the toolkit has a chooser and composed
> where it does not, which is android-mdc and harmony-arkui: neither platform ships a color picker
> in its framework, its design library, or its NDK. Writing one twice in two foreign languages
> would have produced two dialogs that were neither the platform's nor each other's; one panel
> written once in Rust also gives every other target the option. This is the first piece whose
> "emulated" tier is a COMPOSITION rather than a per-backend hand-roll, and the pattern generalizes
> to any control the platforms disagree about.
>
> Two framework changes came with it, both small and both general: `Decorate::on_tap_at` reports
> the tap's location in the piece's own space (`Event::Tap` always carried the point; the
> decorator threw it away), which is what lets a canvas turn a press into a value; and
> `Dom::listen` gives a web piece the shim's event wiring that the built-in kinds already had.
> [docs/color.md](docs/color.md) records what a native pick can carry that `Color` cannot — wide-gamut
> spaces, the authoring model, dynamic system colors — and proposes what to do about it.

### B.8 SwiftUI embedding (user views inside a Day app)

> [!NOTE]
> **Shipped (2026-08)** as `pieces/day-piece-swiftui` + build support ([docs/swiftui.md](docs/swiftui.md)). An app
> points `[package.metadata.day.ios/macos].swift-packages` at a local SwiftPM package; day-build
> scans its public `View` structs (text parse of a documented subset, validated by the Swift
> compile of the generated glue) and emits typed constructors (`crate::swiftui::MyView(…)`,
> arguments `IntoReactive` — the res::str model applied to views), while `day build` generates
> `@objc(DayView_<Module>_<View>)` provider classes wrapping each view in `NSHostingView` /
> `UIHostingController`. The providers subclass a hand-written escape hatch
> (`DaySwiftUIProvider` + `swiftui("name")`) that remains public for views the subset can't
> express. Params ride one JSON string; reactive changes swap `rootView`, and SwiftUI diffing
> preserves `@State`; an opt-in `.state_key(…)` retains the hosting view across unmount/remount,
> so `@State` also survives tab switches and page navigation. This shipped the
> `[package.metadata.day.macos]` leg (§15.2) — the first
> Swift compilation on a cargo-driven target — and the provider naming deliberately carries no
> Apple structure so an android-mdc Compose leg (`Class.forName` + `AbstractComposeView`) stays
> possible. First consumer: Day-Showcase's Benchmark page, hosting the SwiftUI twin of the
> Day-Bench Grids benchmark beside the Day-native one under a segmented picker.

---

# Appendix C — dayscript reference (v1)

> [!NOTE]
> **Status: rewritten to the shipped catalog** (`day-script`'s `Step` enum is normative; the
> website's dayscript page is the tutorial form). The designed catalog was larger in some
> directions (locator qualifiers, `clear`/`key`/`scroll_to`/`repeat` blocks/`run_flow`,
> runner-executed `launch`/`terminate`, native-injection tiers) and smaller in others — the
> shipped one gained navigation, dialogs, and focus steps the design predates. Unshipped
> designed steps return "unknown step" errors, exactly as the step-tier plan intended.

Scripts are YAML: `name`, `description`, and a `flow:` list of steps. Any string in a step may
carry `${project}`, which the CLI expands to the absolute path of the project root before the
step reaches the engine — the way a script names a fixture that lives in the REPOSITORY
(`respond: { path: "${project}/tests/data/sample.opml" }`), so a run works on any machine and
on CI. Test resources belong in the repository for exactly this reason; a path into a
developer's home directory compiles and runs nowhere else. Every element reference
is a Day `.id()` ([§5.5](#55-node-identity-ids-and-the-element-index)). Steps whose failure may resolve with time (element not found yet,
assertion pending) retry within a bounded implicit wait (5 s default) — no sleeps in
well-written scripts; `pause` exists for demos and settle-time.

| step | fields | notes |
|---|---|---|
| `wait_for` | `id`, `timeout_secs?` | until the element has a visible frame; `timeout_secs` raises the implicit wait for elements gated on slow work (a login round-trip, a first sync) |
| `wait_idle` | — | flush the reactive drain |
| `tap` | `id`, `repeat?`, `at?`, `modifiers?` | delivers `Pressed` AND a gesture `Tap` at `at` (default the node's center); `modifiers: [shift]`/`[primary]`/`[alt]` stand held keys in through `day::modifiers()` while dispatching |
| `drag` | `id`, `from`, `to`, `steps?`, `modifiers?` | a whole gesture in the element's own coordinates: `Began` at `from`, `steps` (default 4) `Changed` samples along the way, `Ended` at `to`. `modifiers` are held for every phase — a drag reads them once, when it starts |
| `key` | `key`, `id?`, `modifiers?` | a non-text key (`Event::Key`, web `KeyboardEvent.key` names — `ArrowRight`, …) delivered the way the platform delivers one: to the named piece, or to whatever holds FOCUS. Pair it with `focus:` to drive the whole route; with nothing focused and no `id` the step fails rather than dropping the key — [docs/menus.md](docs/menus.md) |
| `input` | `id`, `text?` \| `key?` + `args?` | `key:` resolves a Fluent key in the run's locale — locale-portable typing |
| `submit` | `id` | delivers `Event::Submitted` — the scripted stand-in for Enter in a `text_area` `.on_submit` (or a field's return key) |
| `set_value` | `id`, `value` | sliders et al. |
| `toggle` | `id`, `value?` | omitted value = flip |
| `select` | `id`, `index` | pickers/tabs |
| `reorder` | `id`, `from`, `to` | drag-reorder a list row through the guard → commit seam ([docs/list.md](docs/list.md)); a guard denial fails the step, non-retryably |
| `delete_row` | `id`, `row` | delete a list row through the same guard → commit path a native swipe takes ([docs/list.md](docs/list.md)); a guard refusal fails the step, non-retryably |
| `swipe_row` | `id`, `row`, `edge?`, `action?`, `label?`/`key?` | activate a row's swipe action through the offer → commit seam ([docs/list.md](docs/list.md)): `edge` defaults to `trailing`, `action` to 0 (the full-swipe button); `label:`/`key:` (Fluent, run-locale) pins which button may be pressed — a mismatched offer refuses the press, leaving state untouched |
| `web_eval` | `id`, `script`, `contains?`, `text?` | evaluate JavaScript in a web-view node and assert on the result ([docs/webview-eval.md](docs/webview-eval.md)) — proves a page RENDERED where `assert_visible` only proves the native view exists; retryable while pending/throwing/mismatching, so a page mid-load settles within the wait; gate with `only_on:` to the backends whose eval arm exists |
| `expand` | `id`, `row`, `expanded?` | disclose/collapse a tree row by its `.row_id` string — emits the same `TreeExpanded` a native disclosure does ([docs/tree.md](docs/tree.md)); omitted `expanded` = true |
| `tree_move` | `id`, `row`, `parent?`, `index?` | move a tree row through the guard → commit seam ([docs/tree.md](docs/tree.md)): absent `parent` = the root, absent `index` = dropped ONTO the parent; a guard denial fails the step, non-retryably |
| `menu` | `item` \| `key`, `path?` | invoke an app-menu action by label or Fluent key (locale-portable; the auto Preferences/New Window items resolve by `day-preferences`/`day-new-window` even with no app menu). `path` narrows by ancestor submenu, each entry matching a literal label or a Fluent key — [docs/menus.md](docs/menus.md) |
| `toolbar` | `item`, `text?` \| `key?` + `args?` \| `on?` | drive a window-toolbar item by its id: bare = run a button's command, `text` types into a search item (`key` types a Fluent key resolved in the run's locale instead — locale-portable, as `input` takes one), `on` sets a toggle. Goes through the same dispatch the native control fires, so it exercises the app's wiring but does NOT prove the widget drew — [docs/toolbars.md](docs/toolbars.md) |
| `close_window` | `window` | close the secondary window opened under this key through the async confirm → teardown path ([docs/windows.md](docs/windows.md)); already-closed is a success |
| `focus` | `id`, `focused?` | drives the REAL `Toolkit::focus` duty (keyboards engage); `focused: false` resigns ([docs/focus.md](docs/focus.md)) |
| `scroll_to` | `id`, `edge?` \| `x?`+`y?` | `edge: top\|bottom\|leading\|trailing` or an offset drives a `scroll` piece; bare `id` reveals that element in its nearest scroll ([docs/scroll.md](docs/scroll.md)); unanimated |
| `navigate` | `route` | reset-to semantics; `""` = root ([docs/navigation.md](docs/navigation.md)) |
| `deep_link` | `url` | deliver a deep-link URL in-process: the URL maps to its route through the same `day_spec::route_of_url` every platform intake uses, then navigates — a warm OS delivery minus the OS (which is the launch runner's tier; [docs/deep-links.md](docs/deep-links.md)) |
| `nav_back` | — | pop one level, the native back path |
| `assert_route` | `route` | current path |
| `assert_visible` | `id` | realized with a nonzero frame |
| `assert_missing` | `id` | the id is NOT in the tree — the assertion for a subtree a `when` has not mounted (a property row that does not apply). `assert_visible` cannot express it: a missing id is an error there |
| `assert_text` | `id`, `text?` \| `key?` + `args?` | FSI/PDI-normalized ([§12.2](#122-api)) |
| `assert_value` | `id`, `value` | typed per piece kind: toggle = bool, slider = number, field = string |
| `assert_focused` | `id`, `focused?` | reads the probe's focus mirror; retryable |
| `assert_presented` | `title?` | a native modal is up ([docs/dialogs.md](docs/dialogs.md)) |
| `respond` | `button?` \| `text?` \| `path?` \| `dismiss` | answer the open modal / file picker |
| `a11y_audit` | `id?` | diff the NATIVE accessibility tree against Day's expectations ([§13](#13-accessibility), [§14.2](#142-the-embedded-engine)) |
| `assert_no_placeholders` | `allow?` | fails if any kind rendered a `⟨kind⟩` placeholder — the one gap no screenshot or other assertion can see. `allow` is the per-target ledger; the generated [docs/coverage-matrix.md](docs/coverage-matrix.md) is its static twin |
| `screenshot` | name, `window?`, `title?`, `caption?`, `source?` | waits for `ui_idle`; `window` captures the secondary window opened under that key ([docs/windows.md](docs/windows.md)). Desktop captures in-process; a device or simulator uses the platform's screen capture, falling back to the in-process one ([docs/window-image.md](docs/window-image.md)). `title`/`caption` (plain string or locale-keyed map) and `source` are runner-side gallery metadata (§14.7) — stripped before the engine, folded into the target's gallery.json |
| `pause` | `secs` | demos only |
| `expect_exit` | `within?` | MUST be last: tolerates the app terminating — a dropped connection within `within` s (default 15) is success, surviving is failure. Runner-side; drives crash-reporting tests ([docs/break.md](docs/break.md)) |

Acting steps synthesize Day events (`tap` = the action path, `input` = the controlled-text
path) on the main thread between flushes — deterministic and toolkit-uniform, per DP-13. The
`focus` step is the deliberate exception that drives a real toolkit duty. The designed
actionability preconditions (enabled/occlusion checks, auto-scroll-into-view) are **not
implemented** — scripts scroll explicitly and the walkthrough is written accordingly.

Any step may carry `skip_on: [<target-or-toolkit>, …]` (2026-07): the RUNNER drops it on the
named targets before sending, so one script drives every platform while staying honest about
genuinely absent capabilities (the showcase walkthrough skips its file-picker and
loopback-HTTP steps on `web-dom` — [docs/web.md](docs/web.md)). Its mirror `only_on: [...]` (2026-07) runs a step
ONLY on the named targets, for a step whose expectation is per-target — the walkthrough's
`assert_no_placeholders` allow-lists differ sharply between, say, `macos-appkit` (none) and
`web-dom` (six).

---

# Appendix D — `day` CLI transcripts

> [!NOTE]
> Illustrative (paths/versions/timings are representative, not verbatim); `day --help` and
> docs/cli.md (website) are authoritative.

```
$ day doctor
day 0.0.9 · project fieldnotes · targets: macos-appkit, ios-uikit, android-mdc
✓ rust        1.89 (rustup) + targets aarch64-apple-ios-sim, aarch64-linux-android
✓ xcode       16.3 · simulators: iPhone 16 (booted)
✗ android     JDK 26 found — AGP requires ≤21    → brew install openjdk@21
✓ gtk4        4.16 (homebrew) · pkg-config OK
! qt6         not found — target macos-qt disabled  → brew install qt@6

$ day launch -p macos-appkit -p ios-uikit -p android-mdc
  macos-appkit    cargo build … ✓ · launched
  ios-uikit       xcodebuild … ✓ · installed → launched on iPhone 16
  android-mdc  gradle :app:assembleDebug … ✓ · adb install → launched on emulator-5554

$ day launch -p ios-uikit --locale fr --script dayscript/walkthrough.yaml
  … ✓ 208/208 steps · 20 screenshots → build/day/screenshots/ios-uikit/fr/

$ day drive -p macos-appkit --steps-json \
  '[{"navigate":{"route":"controls"}},{"tap":{"id":"increment-button","repeat":2}},
    {"assert_text":{"id":"counter-label","key":"counter_value","args":{"count":2}}}]'

$ day lint
✓ no lint findings

$ day pack -p macos-appkit --profile release
✓ build → sign (Developer ID …) → notarize → build/day/dist/Fieldnotes.dmg
```

---

# Appendix E — Implementation notes for the builder (historical)

> [!NOTE]
> The build happened; these notes guided it. Kept because they explain the port order and the
> "copy pane's FFI verbatim" strategy that made eight backends tractable. Items that changed
> in flight: `day-meta` became day-cli's `meta` module + `day-build` ([§17.5](#175-metadata-conveyance-daytoml--each-build-system)); the
> registrant/aggregator codegen is the [§15.2](#152-package-layout-and-aggregation) metadata mechanism; the dayffi/piece-ci items were
> superseded ([§15.3](#153-dayffi-the-c-abi-superseded--never-built)).

1. Port order for `day-reactive`: start from `pane-graph` (arena, Copy handles, push-pull,
   batching) and add scopes/ownership, `Setter`, `watch`, and `bind` (floem `create_updater`
   semantics). Property tests for diamond deps, disposal-during-drain, write-during-drain,
   setter-after-dispose, the fixpoint re-run cap, and the reentrancy echo case ([§4.4](#44-events-and-controlled-inputs)).
2. `day-mock` op-log format is the contract for M1's "exactly one op per state change" **and
   "bounded measure calls"** tests — design it for golden-file diffs from day one.
3. Backends: copy pane's working FFI verbatim where possible (objc2 class registration pitfalls,
   `MainThreadOnly` app delegates, GTK layout-manager shrink fix, Qt/WinRT shim build scripts,
   Android absolute-layout ViewGroup + single JNI trampoline); the Day deltas are:
   measure-with-proposal, scroll protocol, a11y props, DrawOp replay, snapshot, adopt, lifecycle
   hooks, and the enqueue-only sink contract.
4. Keep `day-spec` additive-only from M2 onward (every new duty defaults to
   no-op/`Unsupported`); backends live in-tree but must compile against the spec's published
   semver. DP-16 (unified `ItemSlot`) and DP-23 (native navigation containers) are resolved — the
   freeze is unblocked; validate the `ItemSlot` contract on day-mock in M1–M2 as specified.
5. Implement `day-meta` before the CLI (the `day` crate's build script needs it at M2; the CLI
   reuses it at M4). Registrant/aggregator codegen ([§8.2](#82-the-open-renderer-registry), [§15.2](#152-package-layout-and-aggregation), [§15.3](#153-dayffi-the-c-abi-superseded--never-built)) is one subsystem — build
   it once with three emitters (Swift/C, Kotlin, Rust).
6. Every CI-critical toolchain fact (JDK version, `GSK_RENDERER=cairo`, rustup-not-homebrew,
   `aarch64-apple-ios-sim`, Gradle configuration-cache, `ENABLE_USER_SCRIPT_SANDBOXING`) is
   encoded in `day doctor` checks *and* asserted in CI, never tribal.
7. piece-ci runs the dayffi ASAN ownership-round-trip suite and the v1-pinned ABI cell from the
   first tier-2 piece onward ([§15.3](#153-dayffi-the-c-abi-superseded--never-built), [§20](#20-continuous-integration)).
