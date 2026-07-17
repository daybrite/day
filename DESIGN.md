# Day — Design Document

**An industry-strength Rust framework for cross-platform application development with native toolkits.**

> Status: **implemented and shipping.** This document began as the pre-implementation design
> (adversarially reviewed 2026-07-01); the framework has since been built. Seven native targets
> plus the headless mock toolkit run today; the showcase app passes its 200+-step scripted
> walkthrough on macOS (AppKit, GTK, Qt), iOS, and Android; the `day` CLI builds, launches,
> scripts, and packs for every target; CI exercises all of it. The document is now the
> **architecture overview and rationale**:
>
> - **Part I (§0–§20)** describes the system as built. Where the shipped design differs from
>   the original text, a `> **Status:**` note at the top of the section says exactly how.
> - **Part II (§21–§24)** is the preserved historical record — the milestone plan, decision
>   points, and adversarial-review findings. It is complete; nothing in it is open. It stays
>   because it documents *why* the architecture is shaped this way.
>
> Subsystem detail lives in `docs/*.md` (normative — see the index below); this document is the
> map and the rationale. Section numbers are stable: hundreds of source comments cite them
> (`§4.4`, `§8.3`, …). Never renumber a section; add subsections or addenda instead.

## Reading this document

Each Part I section is annotated with one of:

- **Shipped as written** — the code matches this text.
- **Shipped differently** — the goal survived, the mechanism changed; the note says how and
  where the real design is documented.
- **Not implemented** — specified here but never built (kept as a recorded design).
- *(unannotated sections describe concepts and rationale that are accurate as written)*

Part I still names milestones (M0–M9) in places; those refer to §21.2's historical plan — all
of it is complete. Read "an M5 acceptance item" as "verified when that milestone landed".

### Subsystem index

The normative reference for each shipped subsystem is its `docs/` file; the section here gives
the architecture-level view and the rationale.

| subsystem | normative doc | overview here |
|---|---|---|
| navigation — `routes!`, `selector`, `stack`, deep links, predictive back | docs/navigation.md | §10.5 |
| native recycling lists | docs/list.md | §10 |
| tabs | docs/tabs.md | §10.5 |
| menus — app menu, context menus, roles, shortcuts | docs/menus.md | §8.1 |
| dialogs & presentation — alert/confirm/prompt/sheets, file pickers | docs/dialogs.md, docs/files.md | §8.1 |
| forms — `form`/`section`/`labeled` | docs/forms.md | §5.3 |
| keyboard focus — `.focused()`, `on_submit`, dayscript focus steps | docs/focus.md | §4.4, §8.3 |
| canvas, shapes, gradients, gestures | docs/shapes.md | §11 |
| text & typography | docs/text.md | §6.4 |
| localization — Fluent, `res::str` typed keys, locales | docs/localization.md | §12, §18.5 |
| resources — images, data assets, custom fonts, typed constants | docs/resources.md | §18 |
| accessibility & the a11y audit | docs/accessibility.md | §13 |
| app lifecycle | docs/lifecycle.md | §8.1, §9 |
| tweaks — per-toolkit configuration of built-ins | docs/tweaks.md | Addendum |
| extension packages — pieces, parts, `[package.metadata.day.*]` | docs/extending.md | §15 |
| scripting & agents — dayscript, `day drive`, MCP | docs/agent.md, website dayscript reference | §14 |
| platform services ("parts": battery, network, sensors, clipboard, prefs, haptics, deviceinfo) | docs/battery.md, docs/network.md, docs/sensors.md, docs/clipboard.md, docs/prefs.md, docs/haptics.md, docs/deviceinfo.md | §15 |
| bundled pieces (webview, media, map, lottie, picker, searchfield, …) | docs/webview.md, docs/media.md, docs/map.md, docs/lottie.md, docs/picker.md, docs/searchfield.md | §15 |
| HarmonyOS / OpenHarmony | docs/harmonyos.md | §9 |
| toolchain & environment discovery | docs/environment.md | §16 |
| API design conventions | docs/api-style.md | §5.1 |

**Maintenance rule (binding):** any change that alters what this document describes — day-spec
duties or events, the built-in piece vocabulary, CLI commands, dayscript steps, the extension
mechanisms, the crate set, or the repository layout — must update the affected section (or its
pointer table) in the same change, so this document always reflects the current reality. When a
section would restate something a `docs/*.md` file owns, point to it instead of duplicating it.

---

## Table of contents

**Part I — the architecture as built**

- §0 Vision, lineage, and non-goals
- §1 Glossary and naming
- §2 The four pillars
- §3 Architecture overview and crate graph
- §4 Reactive core (`day-reactive`)
- §5 The Piece model (`day-core`)
- §6 Styling and per-target variation
- §7 Layout
- §8 The Toolkit specification (`day-spec`)
- §9 The eight toolkits (and the extra combinations)
- §10 Native list integration
- §11 Canvas
- §12 Localization (Fluent)
- §13 Accessibility
- §14 Scripting (dayscript)
- §15 Extensibility: pieces, parts, and tweaks
- §16 The `day` CLI
- §17 The Conventional Day Project and `Day.toml`
- §18 Resources, icons, and theming
- §19 Repository layout, examples, and docs site
- §20 Continuous integration

**Part II — historical record (complete; kept for rationale)**

- §21 MVP definition and milestone plan
- §22 Decision points for review
- §23 Risks
- §24 Adversarial review findings and resolutions
- Addendum (2026-07-09): Tweaks

**Appendices**

- Appendix A: The showcase app (pointer to the live app)
- Appendix B: Worked extension examples — design-era sketches with shipped outcomes
- Appendix C: dayscript reference (v1)
- Appendix D: `day` CLI transcripts (illustrative)
- Appendix E: Implementation notes for the builder (historical)

---

## §0 Vision, lineage, and non-goals

### §0.1 What Day is

**Day** is a Rust framework for building applications that look, feel, and behave like native
applications on every platform — because they *are* native applications. UI is authored once, in
idiomatic Rust, as a declarative tree of **Pieces** (what SwiftUI calls a View and Flutter calls a
Widget). Each Piece is realized by **real native components** — `UILabel`, a Material `MaterialButton`,
`NSTextField`, `GtkEntry`, `QSlider`, WinUI `TextBox`, a DOM `<input>` — through a per-platform
**toolkit** backend. Day owns layout, reactivity, localization, accessibility policy, and scripting;
the platform owns pixels, text input, scrolling physics, and assistive technology.

Seven **primary targets** (OS–toolkit combinations), all shipped:

| target | OS | toolkit | status |
|---|---|---|---|
| `macos-appkit` | macOS | AppKit | shipped; walkthrough + pack (`.dmg`) in CI |
| `ios-uikit` | iOS | UIKit | shipped; Simulator walkthrough + pack (`.ipa`) in CI |
| `android-widget` | Android | Material Components (M3 Expressive) / android.view | shipped; emulator walkthrough + pack (`.apk`/`.aab`) in CI |
| `linux-gtk` | Linux | GTK 4 | shipped; headless walkthrough + pack (flatpak) in CI |
| `linux-qt` | Linux | Qt 6 Widgets | shipped; headless walkthrough + pack (flatpak) in CI |
| `windows-winui` | Windows | system XAML (XAML Islands in a Win32 host) | shipped; CI-verified (`.msix` + installer) |
| `ohos-arkui` | HarmonyOS | ArkUI (NDK C API) | shipped; cross-compile in CI, `.hap` pack, `day ohos` emulator helpers (docs/harmonyos.md) |

An eighth backend, **`day-mock`**, is headless: it records toolkit ops and answers deterministic
measurements, so the whole pipeline is unit-testable without a display (§3.2). A `web-html`
(wasm32 + DOM) backend was sketched in the original design and **was never built**; no `day-web`
crate exists. The sketch is preserved in §9 as a recorded design.

Because GTK and Qt are themselves portable, the **non-default combinations** `macos-gtk`,
`macos-qt`, `windows-qt`, and `windows-gtk` are also valid targets — a target is just an
(OS, toolkit) pair whose toolkit supports that OS. Day's own development loop runs six targets
on a single macOS host: `macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit` (Simulator),
`android-widget` (emulator), and `ohos-arkui` (cross-compile; emulator via `day ohos`).

A `day` command-line tool — deliberately modeled on the architecture of `flutter_tools`
(`flutter/packages/flutter_tools`) — creates, builds, signs, launches, packs, lints, scripts,
and drives Day projects, and is designed for use by humans, CI, IDEs, and AI agents alike
(`day drive` and `day mcp-server` are the agent surface — docs/agent.md, §16).

### §0.2 Lineage — what each ancestor contributes

Day is not a greenfield guess. It consolidates several years of prior art in this workspace:

| ancestor | what Day inherits | what Day changes |
|---|---|---|
| **pane/** (Rust, 6 native backends running) | The `Backend`-trait shape with an associated `Handle`; one-toolkit-per-binary monomorphization; the open, link-time component registry (`linkme`); descriptor-carried value bindings (signal + `on_change` closure, per-widget callback tables keyed by id); the C++ shim pattern for Qt and WinUI; the JNI + Java-shim pattern for Android; the objc2 patterns for AppKit/UIKit; the headless mock toolkit for unit testing the whole pipeline | pane re-renders observing views and reconciles; Day builds the tree **once** and binds attributes reactively (§4) — no tree diffing on state change |
| **hop/** (Swift, 4 desktop toolkits) | The parent-proposes/child-chooses layout engine and the lessons it banked (text height-for-width measurement, GTK window shrink, scroll/split interactions); AX-tree diff validation; the CI screenshot pipeline (content-validated captures, `GITHUB_STEP_SUMMARY` galleries); `hoppack`'s per-OS packaging Stage pipeline | Day's layout engine is a from-scratch Rust design informed by hop's, with an open `Layout` trait |
| **skip/ + skipstone/** (Swift↔Kotlin app tooling) | The Conventional Project shape (a normal language-native project plus per-platform scaffolds); metadata conveyance via generated files (`Skip.env` → xcconfig); the discipline of gradle/xcodebuild orchestration; emulator/simulator management; polyglot bridging scar tissue (skip-bridge) | Day's polyglot boundary is a small stable C ABI (§15), not transpilation or generated JNI bridging |
| **floem/** (Rust, GPU-rendered) | The authoring surface: plain functions and builder methods, **no required macros**; `Copy` signals in a scope-owned arena; `create_updater`-style bind-to-setter effects; keyed `dyn_stack` and virtualized `virtual_stack` decomposition; `canvas(|cx, size| …)`; Fluent-based localization proven in this exact API style | floem renders its own pixels (vello/vger + taffy); Day drives native widgets and owns a native-measurement-aware layout engine |
| **flutter/** (Dart; tool studied at `flutter/packages/flutter_tools`) | CLI architecture: DI'd services behind a context for testability; the `Command` envelope (`validate → run`); `doctor` + per-platform workflows; templates for `create`; **the platform-shell callback build pattern** (the Xcode/Gradle project calls back into the tool for the framework part, so native IDE builds are never stale); `gradle_errors`-style failure translation; the machine/daemon protocol for IDEs | Day has no VM: no hot reload in v1 (fast recompile + relaunch + dayscript replay instead, §16.9); Day's platform shells host native widgets, not a rendering engine |

### §0.3 Non-goals

- **Not a renderer.** Day never rasterizes text or widgets itself (the Canvas piece delegates to the
  platform's native 2D API). No skia, no vello, no embedded web view for core UI.
- **Not pixel-identical across platforms.** A Day app looks like a Mac app on macOS and a Material
  app on Android. Cross-platform *consistency of behavior and information architecture*, native
  *look and feel*.
- **Not a Dart/JS-style VM platform.** Rust compiles ahead of time. No hot reload in v1 (§16.9
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
| **Part** | A headless platform-service package — battery, network, clipboard, sensors, prefs, haptics, device info — exposing signals/functions with per-OS native halves (`parts/day-part-*`, §15). |
| **Tweak** | A per-toolkit configuration of the native widget behind an existing built-in piece (`Decorate::tweak`, `tweaks/day-tweak-*`; Addendum, docs/tweaks.md). |
| **Toolkit** | A native widget system: UIKit, android.widget, AppKit, GTK 4, Qt 6 Widgets, Windows XAML, ArkUI (+ the headless mock). |
| **Target** | An (OS, toolkit) pair, written `<os>-<toolkit>`: `macos-appkit`, `macos-gtk`, `ios-uikit`, … One binary is built per target. |
| **Backend crate** | The Rust crate implementing `day-spec` for one toolkit (`toolkits/day-appkit`, `toolkits/day-gtk`, …). One backend is linked per binary. |
| **Realized tree** | The runtime tree of mounted pieces: each node owns a native handle (or is layout-only), a reactive scope, and layout state. |
| **Signal / Memo / Effect / Scope** | The reactive primitives (§4). |
| **Route** | A typed navigation destination declared with the `routes!` macro; what `selector`/`stack`, deep links, and dayscript `navigate` speak (docs/navigation.md). |
| **dayffi** | *(superseded)* The C ABI designed for polyglot extensions; never shipped. The shipped mechanism is `[package.metadata.day.<platform>]` (§15). |
| **dayscript** | The Maestro-inspired YAML UI-scripting language and its embedded engine (§14); a project's scripts live in `dayscript/` and the showcase's main script is "the walkthrough". |
| **Day.toml** | The project manifest (§17.3). |
| **Porcelain / plumbing** | User-facing CLI commands vs. stable hidden commands invoked by build systems (`day xcode-backend build`, `day gradle-backend build`) (§16, §17.4). |

**Crate naming.** All crates are prefixed `day-` (`day-core`, `day-reactive`, `day-appkit`, …); the
umbrella facade crate that apps depend on is `day` with the binary tool in `day-cli` producing a
binary named `day`. DP-24 (§22) deferred crates.io reservation during the design phase; the
release lane is since **wired for crates.io** (publishability verified per PR; Trusted
Publishing on semver tags, §20) but the crates are **not yet published** — scaffolds default to
git dependencies (`day new --git`), with `--registry` ready for the day they are.

**Target strings** are the canonical identifiers everywhere: `Day.toml` `targets:`, `day launch
--platform`, CI job names, screenshot directory names, `PerTarget` style values. The toolkit half
also exists alone (`uikit`, `widget`, `appkit`, `gtk`, `qt`, `winui`, `arkui`, `mock`) for cases
where OS doesn't matter (styling varies by toolkit far more often than by OS).

---

## §2 The four pillars

Every Day app must be **1. localizable, 2. accessible, 3. scriptable, and 4. extensible** — and the
pillars deliberately build on each other:

1. **Localizable (§12).** Mozilla Fluent throughout. Every user-facing string in a Piece is a
   Fluent key by convention — enforced in practice by the `res::str` typed keys (§18.5), which
   make a missing key a compile error, with `day lint` covering cross-locale coverage. The
   current locale is a *signal*, so locale switches are just another fine-grained update.
2. **Accessible (§13).** Accessibility rides the platform's native accessibility tree — Day uses
   native widgets, so baseline accessibility is inherited rather than reimplemented. Day adds a
   uniform annotation API and, critically, **stable identifiers**.
3. **Scriptable (§14).** dayscript targets elements by those same accessibility identifiers — the
   accessibility pillar is the scripting pillar's addressing scheme. Scripts run against localized
   builds (`day launch --locale fr-FR --script …`), so pillar 1 × pillar 3 = automated per-locale
   screenshots and e2e tests in CI.
4. **Extensible (§15).** Pieces, parts, and toolkit renderers are registered through open
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
│ appkit │ uikit │ android│  gtk  │  qt   │ winui │ arkui  │   mock   │
└────────┴───────┴────────┴───────┴───────┴───────┴────────┴──────────┘
           each backend crate drives ONE native toolkit
```

Beside the runtime graph sit the build-time crates: `day-build` (an app's `build.rs` dependency —
typed resource constants, §18.5), `day-fonts` (font name-table parsing shared by the CLI and the
runtimes), `day-toolchain` (host SDK/toolchain discovery shared by the CLI and the `-sys` build
scripts), and `day-cli` (the `day` binary).

### §3.2 Crates

> **Status: shipped differently.** This table reflects the crates as they exist. Relative to the
> original design: `day-canvas` was folded into `day-pieces`/`day-spec` (the `DrawOp` types live
> in the spec, the `canvas()`/shape pieces in day-pieces); `day-script-proto` was dropped (the
> wire protocol is newline-delimited JSON inside `day-script`); `day-meta` became a `day-cli`
> module plus the published `day-build` crate; `day-web` was never built; and `day-l10n`,
> `day-build`, `day-fonts`, `day-toolchain` were added.

| crate | contents | depends on |
|---|---|---|
| `day-reactive` | `Signal<T>`, `Memo<T>`, `Effect`, `Trigger`, `Scope`, `bind`/`watch`, batching, `Setter`, `on_main` scheduler hook | — |
| `day-geometry` | `Point`, `Size`, `Rect`, `Insets`, `Color`, `Affine` — plain `Copy` value types shared by layout, canvas, and the spec | — |
| `day-spec` | `Toolkit` + `Platform` traits, renderer `Registry`, `Event`, typed props/patches, `A11yProps`, `DrawOp` + `Paint`/gradients, `MenuItem`, presentation types, `Cap`/`Support`, `Lifecycle`, `WindowOptions`, piece `kinds` | day-geometry |
| `day-core` | `Piece` trait + `AnyPiece`, `BuildCx`, the realized tree, the mounter, the layout engine (+ measure cache) and `Layout` trait, the event pump, focus, navigation host, list plumbing, menus, presentation, lifecycle, the `resource()` runtime | day-reactive, day-geometry, day-spec |
| `day-pieces` | the built-in vocabulary (§5.3), the `Decorate` modifier set, `routes!`, forms, `selector`/`stack` navigation, dialogs, canvas + shape pieces, the prelude | day-core |
| `day-fluent` | the app-facing Fluent API: `install`, `tr()`, `set_locale`, `LocalizedText` | day-l10n |
| `day-l10n` | the core localization engine — low in the graph so day-pieces' own strings (dialog buttons, menu roles) localize too; also the `res::str` typing rules (§18.5) | — |
| `day-script` | the embedded dayscript engine: step executor, element index, localhost-TCP transport (token-gated, newline-delimited JSON) | day-core, day-fluent |
| `day-mock` | headless toolkit for tests (records ops, deterministic measurement, synthetic events) | day-spec |
| `day-build` | `build.rs` codegen for apps: typed resource constants `res::{images,assets,fonts,str}` (§18.5); the single source of the name-sanitization and Fluent-parsing rules the CLI stagers share | day-fonts, day-l10n |
| `day-fonts` | sfnt name-table parsing (§18.4), shared by the CLI stagers and the runtimes | — |
| `day-toolchain` | one place that knows where host toolchains/SDKs live — used by the CLI, the `-sys` build scripts, and generated scaffolds | — |
| `day` | umbrella: `prelude`, `day::launch`, feature-gated re-export of the selected backend | all of the above |
| `toolkits/day-appkit`, `day-uikit`, `day-gtk`, `day-qt` (+`day-qt-sys`), `day-android`, `day-winui` (+`day-winui-sys`), `day-arkui` (+`day-arkui-sys`) | backend crates | day-spec (NOT day-core) |
| `day-cli` | the `day` binary (§16) | day-build, day-toolchain, day-fonts (+ clap, serde, `serde_norway` YAML, fluent-syntax) |

Two structural rules carried over from pane, both still enforced:

1. **Backends depend only on `day-spec`.** They never see the Piece model or the reactive graph.
   This keeps the spec surface small, keeps backends implementable in ~2–4k lines each, and makes
   the mock toolkit a true stand-in.
2. **One backend per binary.** The active toolkit is selected by cargo feature at app link time
   (`day launch -p macos-gtk` builds with `--features day/gtk`). The running `ToolkitId` is a
   process constant, which §6 exploits for zero-cost per-target styling. Cross-toolkit code paths
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
  `bind_row` exception (§10.2).
- **Native events are never dispatched re-entrantly.** The backend event sink may be *invoked*
  re-entrantly (Qt/GTK/Android text setters fire change notifications synchronously) but its
  contract is enqueue-only (§8.3); day-core drains queued events at safe points, each as a fresh
  batch.

---

## §4 Reactive core (`day-reactive`)

### §4.1 The model: build once, bind forever

> **Status: shipped as written**, with two deltas: the `piece_dyn` escape hatch was never
> needed and does not exist — reactive structure is `when`/`each` (plus the navigation
> containers); and the advisory `day lint` heuristic for signal-reads-outside-bindings was not
> built (the shipped lint rule set is smaller, §16.5).

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
  state's `Scope`, so removal disposes it (§4.3).
- `if`/`for` in plain Rust run once at build time — correct for static structure. *Reactive*
  structure must use `when`/`each`/`piece_dyn`. Day catches the classic SolidJS footgun — a signal
  read in a component body that can never re-run — **at runtime in debug builds**: a tracked read
  during `Piece::build` with no live observer emits a once-per-callsite `#[track_caller]` warning
  ("this read at src/lib.rs:41 will never re-run — wrap it in a binding or use `get_untracked`"),
  asserted by day-mock tests from M1. `day lint` additionally ships a lexical heuristic for the
  same pattern (direct `.get()` in `fn … -> impl Piece` bodies), explicitly labeled *advisory* — a
  fast source-level lint cannot be sound without type information (§16.5).

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

- **Batching and ordering:** exactly the §3.3 turn state machine — synchronous fixpoint drain,
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
  turn boundary; the §8.1 contract lets backends defer further (Qt `deleteLater`) and requires
  them to tolerate release at any main-loop-safe point.
- *Disposed-handle access*: **writes are silent no-ops** with a once-per-callsite debug warning
  (`Setter` inherits this — async deliveries racing disposal are expected); **reads panic in debug**
  with the handle's `#[track_caller]` creation location; `try_get`/`try_with` are the blessed
  forms in any closure that can outlive its scope. Event handlers on nodes disposed in the current
  drain are unregistered before their scope's signals drop. Release-build read behavior is DP-18.

`Scope::provide::<T>(value)` / `Scope::use_context::<T>()` give dependency injection down the
*build* tree (theme, locale handle, navigation), resolved at build time — again, no re-render
semantics needed.

### §4.4 Events and controlled inputs

Native events (button press, text change, slider drag) enter through the backend's **event
trampoline** (per-widget callback table keyed by node id — pane's proven design). The sink is
enqueue-only (§3.3, §8.3); day-core dispatches each queued event as a fresh batch.

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
- Manual Japanese-IME smokes are acceptance items in M2 (AppKit) and M5 (iOS Simulator); the mock
  toolkit's reentrancy test (apply triggers a synchronous synthetic echo; assert no double-borrow,
  no lost divergent value) lands in M0–M1.

High-frequency events (slider drag) apply value writes per event; layout coalesces to the turn
boundary (§3.3).

### §4.5 Async

> **Status: not implemented as designed.** `Resource`/`Load` never shipped. The shipped async
> story is the smaller §3.3 surface — spawn a thread (or bring your own executor), send results
> back through a `Setter` or `day_reactive::on_main`; the network parts (docs/network.md) and
> the real apps (Day Skies' weather fetch, the Matrix client) all use it. The design below is
> kept as the recorded shape a future `Resource` should take.

```rust
// two-closure shape (leptos-style): `source` is TRACKED on the main thread; its value moves
// into the future, so no !Send Signal ever crosses a thread boundary *by construction*.
let stations = Resource::new(
    move || region.get(),                       // S: Send + Clone + PartialEq — refetch on change
    |region| async move { fetch_stations(region).await },
);
// stations: Signal<Load<Vec<Station>>>; Load: Clone
// Load::Loading | Load::Ready(T) | Load::Failed(Arc<dyn Error + Send + Sync>)
when(move || stations.ready(), move || station_list(stations));
stations.refetch(); stations.loading();
```

Results return through a `Setter` stamped with a fetch generation — **latest wins**; superseded
futures are aborted where the executor supports it. The executor seam is
`spawn(F: Future + MaybeSend)` with a cfg-alias (`Send` bound on native targets, unbounded on
wasm). `Resource` ships in `day-reactive`; honest budget ~300 lines (not a "50-line convenience"),
and it is the only async primitive the MVP needs.

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
  case (`row(PieceVec(stars))`). `Decorate` provides `fn any(self) -> AnyPiece` for build-time
  heterogeneous branches (`if compact { a.any() } else { b.any() }`).
- **Closure capture rules**: the builder closures of `when`/`each` are `Fn` (they may
  run more than once); non-`Copy` captures must be cloned per activation
  (`let items = items.clone();` inside the closure, or capture a `Signal` — signals are `Copy`,
  which is why the idiomatic Day style keeps shared state in signals). The M2 template and
  showcase demonstrate one non-`Copy` capture deliberately.

### §5.2 The `Piece` trait

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
inherent methods (good rustdoc, good autocomplete) — the common modifier set (`padding`, `style`,
`id`, `a11y`, `disabled`, `visible`, `on_key`…) comes from a blanket `Decorate` extension trait.

### §5.3 Built-in pieces (MVP set)

> **Status: shipped and outgrown.** The design-era "MVP set" grew into the full vocabulary
> below, which reflects the prelude as it exists in day-pieces. Deltas from the original text:
> `stack_z` shipped as `zstack`; `piece_dyn` was never needed (structure is `when`/`each` plus
> the navigation containers); the gesture decorators shipped as `.on_tap`/`.on_drag` (context
> menus are declarative — `.context_menu(items)`, docs/menus.md). Per-subsystem detail lives in
> the docs/ files named in the subsystem index.

```rust
// text & controls — two-way controls take `impl SignalRw<T>` (Signal<T>, or a projection):
label(text)                        // text: impl IntoText — value, Signal<String>, closure, or
                                   //   LocalizedText; styled via .font(Font::Headline) / .color(c)
button(text).action(f)             // .bordered() / .prominent() / .style(impl ButtonStyle)
toggle(on)                         // two-way bool
slider(value).range(0.0..=100.0)   // two-way f64; .step(…)
text_field(text).placeholder(p).on_submit(f)   // two-way String; focus via .focused(…) (docs/focus.md)
progress(fraction)   spinner()     // docs/progress.md
image(res::images::logo)           // typed resource constants (§18.5)
divider()   spacer()

// layout containers
column(children).spacing(8.0).align(HAlign::Leading)
row(children).spacing(8.0).align(VAlign::Center)
zstack(children)                   // overlay
scroll(child)
form((section((…)).title(t), …))   // grouped platform forms (docs/forms.md)
labeled(caption, control)

// structure
when(cond_fn, build_fn)            // reactive conditional subtree
each(items_fn, key_fn, build_fn)   // reactive keyed collection (§5.4)
list(items_fn, key_fn, row_fn)     // NATIVE recycling list (§10, docs/list.md)

// navigation & presentation (docs/navigation.md, docs/dialogs.md, docs/menus.md, docs/files.md)
selector(section)                  // sidebar / tabs / segmented, per SelectorStyle
stack(path, root)                  // push/pop navigation bound to a Vec<Route> signal
nav_link(…)   navigate_to(…)   current_route()   route_param(…)
alert(…)   confirm(…)   prompt(…)   open_file(…)   save_file(…)
app_menu(…)   menu_item(…)   sub_menu(…)   menu_role(…)   menu_separator()

// drawing (§11, docs/shapes.md)
canvas(draw_fn)
rectangle()  rounded_rectangle(r)  circle()  capsule()  ellipse()  arc(start, sweep)
    .fill(color) / .fill_linear(g) / .fill_radial(g) / .stroke(color, w)
    .rotate(deg) / .inset(v) / .offset(x, y)      // reactive: any of these takes a closure

// ambient environment
with_environment(value, build_fn)   environment::<T>()
```

The **`Decorate`** extension trait carries the universal modifiers: `.id()` / `.id_keyed()`,
`.padding()`, `.frame()` / `.width()` / `.height()`, `.grow()` variants, `.background()`,
`.corner_radius()`, `.overlay()` / `.overlay_aligned()`, `.a11y()`, `.on_tap()` / `.on_drag()`,
`.focused()`, `.context_menu()`, `.tweak()` / `.native_ref()` (docs/tweaks.md),
`.modifier(impl Modifier)`, and `.any()`.

Beyond the built-ins, optional widgets ship as ordinary crates under `pieces/` (`combo_box`,
`search_field`, `picker`, `rating`, `activity`, `web_view`, `media`, `map`, `lottie`,
`remote_image`, `textarea`) and headless services under `parts/` (battery, network, sensors,
clipboard, prefs, haptics, deviceinfo) — §15 has the extension model.

Example — the shipped composition idiom (from the showcase's Controls page; the live app is the
complete reference, Appendix A):

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

> **Status: shipped with deltas.** The unified `ItemSlot` contract is real (`ItemSlot<T, K>`:
> tracked `get()`/`with()`, `field()` projections, `key()`; keyed diff with per-key scopes,
> slot writes for surviving keys, debug key-uniqueness assertion), and `each` and `list` share
> it as designed. The `slot.rw(get, set)` two-way projection and the `.on_edit` write-back hook
> were **not implemented** — rows that need two-way controls keep signals in app state (or in
> the items) instead; nothing has needed the projection yet.

**Resolved (DP-16: unified).** `each` and the native-recycling `list` (§10) share **one item
contract**: the builder receives an **`ItemSlot<T>`**, never the item by value. The same row
function serves both, so moving a collection from `each` to `list` is a one-word change.

```rust
let todos: Signal<Vec<Todo>> = Signal::new(vec![]);        // plain data; Todo: Clone

column((
    each(move || todos.get(), |t| t.id, move |item: ItemSlot<Todo>| {
        row((
            toggle(item.rw(|t| t.done, |t, v| t.done = v)),   // two-way via SignalRw (§5.3)
            label(move || item.field(|t| t.title.clone())),   // per-field memoized projection
            spacer(),
            button(icon("close"))
                .action(move || todos.update(|v| v.retain(|t| t.id != item.key())))
                .a11y(|a| a.label(tr("todo-remove")))
                .id_keyed("todo-remove", item.key()),         // stable per-item id (§5.5)
        )).spacing(6.0)
    })
    .on_edit(move |key, todo: &Todo| sync_to_model(key, todo)),   // optional write-back hook
))
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
- Two-way controls use `slot.rw(get, set)`: a write applies to the slot's value immediately (the
  collection row is a controlled component, §4.4) and fires the collection's `.on_edit(key, &T)`
  hook so the app writes it back to its source of truth; if the source later re-runs with a
  different value for that key, **the source wins**.
- Debug builds **assert key uniqueness** per diff, panicking with the duplicate key and `each`'s
  creation site (floem's `dyn_stack` corrupts silently on duplicates).
- Reactive *structure* inside a row still uses `when`/`piece_dyn` — deriving structure from
  `slot.get()` in plain Rust freezes at first bind (the §10.1 trap; same rule here, same lint).
- `Store<K, T>` (§4.3) remains the model-layer keyed state container; `each_store(store, build)`
  is a thin adapter (keys from the store, slots fed from its per-key state). Items whose `T`
  carries `Signal` handles remain legal, but plain data + slots is the blessed default.

### §5.5 Node identity, ids, and the element index

Every realized node has a `NodeId` (slotmap key). Separately, `.id("volume-slider")` assigns a
**stable string identifier**, and `.id_keyed("todo-remove", key)` its keyed form for collection
items (rendered as `todo-remove:<key>`; `day lint` enforces prefix uniqueness). Three consumers:
the platform automation/accessibility identifier where one truly exists (the verified per-toolkit
matrix is in §13 — notably Android has **no** external automation-id channel below API 33, and GTK
has none at all today; the doc does not pretend otherwise), the dayscript element index (§14,
which reads day-core directly and therefore works uniformly regardless of platform id support),
and `day lint` uniqueness checks. Ids are the contract between the app and its tests; a lint rule
forbids leaking them into `contentDescription`/a11y labels (screen readers would speak them).

---

## §6 Styling and per-target variation

### §6.1 Style as a value, applied through a builder closure

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
styling (the §5.3 `form`/`section` cards) travels as a semantic `SurfaceRole`, which each backend
resolves to its platform's own material (e.g. `quaternarySystemFill` on macOS 14+).

### §6.2 Per-target variation: `PerTarget<T>` values (no macros)

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

In practice per-target styling has barely been needed: semantic fonts (§6.4), semantic surface
roles (§6.1), and native controls absorb most platform variation by construction. The value-
combinator design (from the `platform!{}` exploration in `pane/DESIGN.md` §4b) is kept here as
the recorded shape sugar could take if branching ever becomes common.

### §6.3 Semantic theme tokens

> **Status: shipped differently.** There is no `theme::` token module. Native fidelity comes
> from a different split: **default appearance is native by construction** — text, controls,
> separators, form cards, and window grounds take the platform's own dynamic colors inside each
> backend (`NSColor.labelColor`, `?attr/colorOnSurface`, QPalette roles, WinUI theme resources),
> so dark/light tracking needs no app-side tokens at all. Apps state only *deliberate* colors
> (`Color::hex(…)` brand values, shape fills, gradients). Semantic *roles* that must cross the
> spec do so as typed values: `SurfaceRole` for grouped-card surfaces, `Font` for typography.
> Forced schemes for screenshots/CI ride the `DAY_THEME=light|dark` launch environment, which
> every backend honors (per-element on WinUI islands, palette on Qt ≤6.7, color-scheme
> elsewhere). An app-wide token module remains possible later; no real app has needed one.

### §6.4 Typography

> **Status: shipped as written** (as an enum rather than constructor fns; no `env::font_scale`
> signal — scaling is applied inside the backends).

`Font` is **semantic-first**: an enum of the platform text styles — `LargeTitle`, `Title`,
`Title2`, `Title3`, `Headline`, `Subheadline`, `Body` (default), `Callout`, `Footnote`,
`Caption`, `Caption2` — resolving to the platform's text-style system
(`UIFont.preferredFont(forTextStyle:)`, Android textAppearance-class scaled sizes,
`NSFont.preferredFont`, documented ramps on gtk/qt) so **Dynamic Type / system font scaling
works by default**. `Font::System(pt)` is the raw-size escape hatch, still scaled by the
platform's accessibility text factor (UIFontMetrics / `sp` / GTK text-scaling-factor);
`Font::Custom(family, pt)` selects a bundled font by family name (§18.4). `FontWeight` and
italic ride the same spec (docs/text.md). A points-first API would have made Dynamic Type
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
*native* ancestor's** coordinate space. Layout-only wrapper nodes (§7.3 decorators, alignment
wrappers) have no native handle; day-core accumulates their offsets when emitting frames. This
rule is what permits a later optimization — flattening pure-layout containers out of the native
tree entirely — as a non-breaking day-core change.

Exceptions where the native container drives: `scroll` (§7.6 — Day measures content, native owns
the viewport) and `list` (§10 — native recycling owns the viewport).

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
(§7.8).

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
   (§10.2).
2. At the turn boundary, relayout **re-enters at each dirty subtree's boundary** and runs a normal
   measure+place pass from there: clean descendants answer from the proposal-keyed cache, and
   place-recursion prunes subtrees whose (proposal, size, origin) are all unchanged. A scroll
   boundary re-runs its *content* layout and emits a content-size update (§7.6).
3. `set_frame` is diffed with a half-device-pixel epsilon (§7.9), so a text change that moves
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
  §7.1 cites, reintroduced at the window level.)
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
- The native side owns the viewport, physics, indicators, and emits `Event::ScrollChanged(Point)`.
  `Toolkit::scroll_to(handle, target_rect, animated)` and `scroll_offset(handle)` complete the
  surface (dayscript `scroll_to` and the keyboard focus-reveal ride these).
- On content relayout the offset is preserved, clamped to the new extent.
- **v1 restrictions, linted:** same-axis nested scrolls and `list`-inside-`scroll` are
  unsupported (`day lint` rule); cross-axis gesture arbitration is documented post-MVP work.

### §7.7 Safe areas, insets, and the keyboard

> **Status: partially shipped.** Safe-area insets are applied at the window root by the mobile
> backends (UIKit reads `safeAreaInsets`; Android is edge-to-edge with the root inset), and the
> soft keyboard is raised/dismissed through the focus system (docs/focus.md). The
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
  the focused field via `scroll_to`. Scoped into M5; a manual keyboard smoke is an M5 acceptance
  item (dayscript cannot see the native keyboard — §14.2).

### §7.8 RTL and BiDi

> **Status: shipped**, with one delta: the `ar-XB` RTL *pseudolocale* was not built — the
> showcase ships a real Arabic locale instead, and the walkthrough + an `rtl-check` dayscript
> run against it (`en-XA` expansion pseudolocalization did ship, §12.2). `layout_direction()` /
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
  bumps the epoch and marks the tree `needs_measure` (Android delivery via §9's configuration
  plumbing; frames are re-multiplied on the new scale).

---

## §8 The Toolkit specification (`day-spec`)

### §8.1 The `Toolkit` trait

> **Status: shipped and grown, exactly as the evolution policy intended.** The original v1
> surface froze, and every later subsystem arrived as a defaulted duty. The listing below is
> the **current** surface (crates/day-spec/src/lib.rs is normative — read the trait there for
> exact signatures and doc comments).

Evolution of pane's `Backend` (proven across six toolkits), extended for Day's pillars:

```rust
pub trait Toolkit: Sized + 'static {
    type Handle: Clone + 'static;

    // capabilities — feature detection for pieces (§10; Cap: ListRecycling, Lottie,
    // NativeSymbols, Snapshot, NavSplit, NavHeader, Dialogs, FileDialogs)
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

    // native recycling lists (§10, docs/list.md)
    fn attach_list(&mut self, host, source: ListSource) {}

    // menus (docs/menus.md)
    fn set_app_menu(&mut self, items: &[MenuItem]) {}
    fn set_context_menu(&mut self, h, node: NodeId, items: &[MenuItem]) {}

    // presentation (docs/dialogs.md, docs/files.md): alerts/confirm/prompt/sheets/pickers
    fn present(&mut self, req: u64, spec: &present::PresentSpec) {}
    fn dismiss(&mut self, req: u64) {}

    // pillars
    fn set_a11y(&mut self, h, a11y: &A11yProps) {}                    // §13
    fn read_a11y(&self, h) -> A11ySnapshot { … }                      // the a11y_audit's native read
    fn replay(&mut self, h, ops: &[DrawOp], size: Size) {}            // canvas §11
    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> { … }    // dayscript §14
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
    fn locale_hints(&self) -> Vec<String> { … }    // ORDERED OS preference list (fluent-langneg)
}
```

One deliberate simplification against the original design: the `AppCx`/`create_window`
multi-window seam was **not** built — `day::launch(root)` + `WindowOptions` (title, size,
min-size from `Day.toml [window]`) is the whole windowing surface, and dialogs/menus arrived as
their own duties instead of flowing through window creation. Multi-window remains future,
additive work.

**Evolution policy (held in practice):** every duty added after the freeze ships with a default
no-op/`Unsupported` body — gestures, focus, lists, menus, presentation, lifecycle, `read_a11y`,
and `ui_idle` all arrived that way, and no backend broke.

`Props` is `&dyn Any` downcast to the piece's typed descriptor (e.g. `LabelProps`) — **zero
serialization between Rust and Rust-implemented backends**; patches are sparse (only changed
fields). The native boundaries that must encode (JNI, the C++ shims) use small packed frames
and primitives, never text formats.

### §8.2 The open renderer registry

> **Status: shipped as the linkme layer.** Each backend exposes a `RENDERERS` distributed
> slice (`#[distributed_slice(day_appkit::RENDERERS)] …`) that external piece crates populate;
> the `day-spec` `Registry` folds them in at toolkit init. The layered hardening below — the
> generated Rust registrant and the required-kinds startup completeness check — was **not
> built**; release builds (including the packed iOS app) have not hit the dead-strip problem in
> practice, and the design is kept here in case it ever does.

Registration was designed **layered** so that `linkme` is a convenience, not a correctness mechanism (the
bare `use crate as _;` anchor is a link-time gamble under iOS `-dead_strip` + LTO, and a
startup-time completeness check is impossible if the registry itself is the only source of truth):

1. Every piece API crate exposes an idempotent `pub fn register()` and contributes a **required
   kinds** manifest entry, making the startup check — required kinds minus available renderers —
   implementable in **all** profiles: debug panics listing the missing (kind, toolkit) pairs;
   release logs loudly and shows an error surface. Never a mid-session surprise.
2. For app targets built by `day build`, tier-1 registration calls are folded into the
   **generated Rust registrant** (the same generated-registrant pattern as dayffi, §15.3) — fully
   deterministic, dead-strip-proof.
3. The `linkme` distributed slice remains for zero-setup unit tests and pure-cargo development
   (pane's proven mechanism, kept as the ergonomic layer).

CI includes a release+LTO ios-uikit build of showcase + day-piece-combobox that asserts via
dayscript that the externally-registered piece actually rendered (§20).

### §8.3 Events

```rust
pub enum Event {
    Pressed,                                  // button
    TextChanged(String), Submitted,
    ToggleChanged(bool),
    ValueChanged(f64),                        // slider et al.
    SelectionChanged(i64),                    // pickers, tabs, nav lists
    FocusChanged(bool),                       // docs/focus.md
    Tap(Point), LongPress(Point), ContextMenu(Point),
    Drag { phase, location, translation },    // docs/shapes.md gestures
    ScrollChanged(Point),                     // §7.6
    FrameChanged(Size),                       // canvas re-record; nav pane size reports
    NavBack { already_popped: bool },         // native back (docs/navigation.md)
    Key(KeyEvent), Pointer(PointerEvent),
    WindowResized(Size),
    PresentResult { req, result },            // modal answers (docs/dialogs.md)
    MenuAction(u64),                          // docs/menus.md
    Lifecycle(Lifecycle),                     // docs/lifecycle.md
    Custom { tag: &'static str, num: f64, text: String },  // open piece-defined channel (§8.2)
}
```

(`Custom` shipped with a primitive `num`/`text` payload rather than the designed
`DayValue` tree — §15 explains; `tag` is empty for events crossing a native boundary.)

The single sink keeps the backend ignorant of closures/lifetimes (day-core owns the `NodeId →
handlers` table) — this is the shape that made pane's six backends small. The sink contract is
enqueue-only (§8.1); handlers run under their registration scope (§4.3).

### §8.4 Animation (reserved hooks — still unimplemented)

> **Status: as designed, still reserved.** `AnimSpec` parameters sit on `set_frame`/`update`
> and every backend ignores them; no `.transition`/`with_animation` surface exists yet.

Native-widget frameworks that bolt animation on later end up breaking their backend ABI — so the
seam ships now even though MVP backends ignore it. Day commits to **backend-executed animation**:
Day passes *intent*, the platform animates (consistent with §0.3 — Day never ticks pixel frames
for native widgets). `AnimSpec { duration, curve, spring }` parameters already sit on `set_frame`
and `update` (§8.1), no-op in MVP backends. The post-MVP surface (design sketch, not v1 API):
`.transition(anim)` on `when`/`each` enter/exit, animated frame changes
(`with_animation(anim, || …)`), and a day-driven frame-clock ticker **for canvas only**.

### §8.5 Panics and crashes

> **Status: partially shipped.** The event pump runs handler dispatch under `catch_unwind`
> (day-core), which covers the main native-callback surface. The wider policy below — per-entry
> guards on every trampoline, the debug error surface, the release panic hook and
> `day::on_crash` — is **not implemented** and remains the design of record.

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

> **Status: all eight shipped** (seven native + mock); `day-web` was never built. One material
> change from the design: the Windows backend hosts **system XAML** (`Windows.UI.Xaml` controls
> in a `DesktopWindowXamlSource` island inside a Win32 window), not WinUI 3 / Windows App SDK —
> no runtime bootstrap, no framework-package dependency, and the `windows-winui` target name
> stayed.

Shared mechanics came from pane's working code; every FFI choice below now runs in this repo:

| backend | FFI mechanism | container | status |
|---|---|---|---|
| `day-appkit` | `objc2` (`objc2-app-kit`) | `NSView` (flipped `DayFlipped`) | shipped; CI walkthrough + pack |
| `day-uikit` | `objc2` (`objc2-ui-kit`) | `UIView` | shipped; Simulator walkthrough + pack in CI |
| `day-gtk` | `gtk4-rs` | `gtk4::Fixed` | shipped (Linux + macOS host); headless CI walkthrough |
| `day-qt` | `cc`-built C++ shim (`day-qt-sys`) | bare `QWidget` | shipped (Linux + macOS host); headless CI walkthrough |
| `day-android` | `jni` + a Java shim (`DayBridge`/`DayFixed`/`DayActivity`) | absolute-layout `ViewGroup` (`DayFixed`) | shipped; emulator walkthrough + pack in CI |
| `day-winui` | C++/WinRT shim (`day-winui-sys`, cppwinrt-staged headers) | XAML `Canvas` in a `DesktopWindowXamlSource` island | shipped; CI-verified build/walkthrough/pack |
| `day-arkui` | ArkUI **NDK C API** via a C++ shim (`day-arkui-sys`; `aarch64-unknown-linux-ohos`) | ArkUI stack node | shipped; cross-compile in CI, emulator via `day ohos` (docs/harmonyos.md) |
| `day-mock` | — | — | shipped; the headless test double (§3.2) |

Per-toolkit notes beyond pane's baseline (the day-new duties):

- **a11y (§13):** UIKit/AppKit: `NSAccessibility`/`UIAccessibility` protocols (mostly free on
  native controls; Day sets labels/identifiers/traits). Android: `contentDescription`,
  `AccessibilityNodeInfo`, `importantForAccessibility`. GTK 4: `GtkAccessible` roles/properties
  (AT-SPI on Linux; off-Linux, GTK 4.18's **AccessKit backend** is the forward path but default
  and Homebrew builds don't enable it — `macos-gtk` currently exposes **no a11y tree at all**,
  which is exactly why it is a *secondary* combination; `day doctor` probes the installed GTK for
  AccessKit and the build/env recipe is documented, not hidden). Qt: `QAccessible` (bridges to
  NSAccessibility/UIA/AT-SPI on all three OSes — Qt is the strongest cross-OS a11y story of the
  portable toolkits). WinUI: UIA, mostly free. Web: ARIA attributes.
- **canvas (§11):** CGContext in `drawRect:`/`draw(_:)`; `android.graphics.Canvas` in `onDraw`
  (display list crosses JNI once per redraw as a packed buffer); `GtkDrawingArea` + cairo;
  `QPainter` in `paintEvent`; Win2D or Direct2D via the shim; DOM `<canvas>` 2D.
- **snapshot (§14):** `CALayer`/`NSView` bitmap render; `UIGraphicsImageRenderer`; `PixelCopy` /
  `View.draw(Canvas)`; `gtk_widget_snapshot` → cairo surface; `QWidget::grab`;
  `RenderTargetBitmap`; `<canvas>` composite (web: best-effort).
- **list hosts (§10):** `UICollectionView` / `RecyclerView` / `NSTableView` / `GtkListView` /
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
  (backends resolve dynamic colors, §6.3), locale → the locale signal (§12), density →
  measure-cache epoch bump + frame re-multiplication (§7.9). The
  suspend/resume/memory hooks (§8.1) map to the Activity callbacks. Process-death state
  restoration (`onSaveInstanceState`) is **DP-25** — v1 documents cold restart.
- **Windows runtime choice.** The designed WinUI 3 / Windows App SDK backend (with its
  `MddBootstrapInitialize2` bootstrap and runtime-installer story) was **replaced by system
  XAML Islands**: `Windows.UI.Xaml` ships in Windows itself, so an unpackaged Day app starts
  with no runtime dependency at all, and `day pack` produces `.msix` plus an NSIS installer
  with nothing to chain. The cost is system-XAML's older control set and per-element theming
  (the shim forces `DAY_THEME` per-element on the root). Moving to WinUI 3 later is a backend
  swap behind the same day-spec surface.

On mobile, the §8.1 "window" maps to the scene / activity content view; multi-window remains
future, additive work (§8.1's status note).

**Extra combinations** (`macos-gtk`, `macos-qt`, `windows-qt`, `windows-gtk`) need no extra code in
the backend crates — GTK/Qt are portable; the *target* differs only in build/packaging (§16, §17:
where the toolkit libraries come from and whether `day pack` can bundle them; bundling GTK/Qt into
a redistributable macOS/Windows app is real work and is explicitly **post-MVP**, DP-7). The
`Day.toml` `targets:` list and `day doctor` gate which combinations a project claims.

**web-html sketch (never built):** wasm32 binary; pieces map to semantic elements
(`<button>`, `<input>`, `<label>`); Day layout emits `position:absolute; transform:translate(…)`
placements; text measurement via a hidden measurement element or `canvas.measureText` (cached);
events via `wasm-bindgen` closures; scripting transport is a `WebSocket` (§14.5). The open
question — whether absolute placement forfeits too much of the browser (text selection across
elements, native scrolling) — is recorded as DP-8 with a proposed hybrid (Day layout, but `scroll`
maps to overflow scrolling).

**ohos-arkui — shipped.** The "speculative sketch" bet paid off: ArkUI's C node API
(`ArkUI_NativeNodeAPI_1`) matched day-spec's shape and the backend is now first-class — full
walkthrough support, native drawing, focus, dialogs, rawfile resources, `.hap` packing, and
`day ohos` emulator helpers. docs/harmonyos.md is the reference.

---

## §10 Native list integration

> **Status: shipped** (docs/list.md is normative). The duty landed as
> `Toolkit::attach_list(host, ListSource)` rather than the sketched `ListHost` object — the
> host pulls `len`/`bind_row` through the `ListSource`, and the mock/walkthrough tests assert
> recycled cells rebind with a slot write, not a rebuild. Qt's emulated recycling shipped as
> designed (DP-19). The `rw`/`.on_edit` two-way projections did not ship (§5.4).

The requirement: Day's `list` must use the platform's recycling list (`UICollectionView`,
`RecyclerView`, `NSTableView`, `GtkListView`, `QListView`) so large collections get native
virtualization, scroll physics, and platform behaviors.

### §10.1 API — the shared `ItemSlot` contract (unified with `each` — DP-16 resolved)

Because cells are **recycled**, the row builder cannot receive the item by value (a moved value
can never be swapped later — recycling would be a rebuild). The builder receives the same
**`ItemSlot<T>`** as `each` (§5.4 — one contract, one row function serves both; migrating a
collection from `scroll(column(each(…)))` to `list` is a one-word change):

```rust
list(move || messages.get(), |m| m.id, move |row: ItemSlot<Message>| {
    column((
        label(move || row.field(|m| m.sender.clone())),   // per-field memoized projection
        label(move || row.field(|m| m.preview.clone())),
        toggle(row.rw(|m| m.starred, |m, v| m.starred = v)),  // SignalRw projection (§5.3, §5.4)
    ))
})
.row_height(RowHeight::Uniform(56.0))     // or ::Automatic (self-sizing, slower)
.row_kind(|m| if m.pinned { RowKind::named("pinned") } else { RowKind::default() })
.on_select(move |id| open(id))
```

All `ItemSlot` semantics are as specified in §5.4 (Copy handle, tracked `get()`,
equality-gated `field()` projections, `rw` + `.on_edit` write-back, key-uniqueness assert, the
structure-from-`get()` trap and its lint). `list` adds `.row_kind`, mapping to the host's native
reuse identifiers (one pool per kind; default single kind).

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
   turn batching (§3.3); without it, recycled cells would display stale content and `Automatic`
   mode would cache wrong heights.
3. Row layout runs Day's engine inside the cell bounds. `RowHeight::Uniform` cells are true layout
   boundaries (§7.4); `::Automatic` cells are boundaries **with notification** — when a row's
   content size changes, Day calls `host.row_size_invalidated(key)`, mapping to
   `reconfigureItems`/preferred-attributes (UICollectionView), `noteHeightOfRows` (NSTableView),
   `requestLayout` (RecyclerView), `InvalidateMeasure` (ItemsRepeater).
4. Selection, separators, swipe actions, section headers are host-native features exposed as list
   options gated on `Toolkit::capability` (§8.1); Qt reports `Emulated` recycling (DP-19).

This was the single hardest backend feature, deferred past the MVP by design — and the
pre-reserved spec hooks did their job: it landed later as a defaulted duty with no breaking
change. `scroll(column(each(…)))` remains the honest choice for small collections.

### §10.5 Navigation and presentation

> **Status: shipped** (docs/navigation.md, docs/dialogs.md, and docs/menus.md are normative).
> The DP-23 "native containers" resolution held, delivered through a richer surface than the
> sketch below:
>
> - **Typed routes.** `day::routes! { enum Section { Controls => "controls", … } }` declares
>   the destinations; deep links, dayscript `navigate`, and `current_route()` all speak the
>   same keys, compile-checked.
> - **`selector(signal)`** — one signal of the active destination, presented per platform and
>   `SelectorStyle` (desktop sidebar + detail split, mobile list-push, tabs, segmented);
>   `Cap::NavSplit`/`Cap::NavHeader` let pages adapt to what the toolkit provides.
> - **`stack(path, root)`** — push/pop navigation bound to a `Vec<Route>` signal; native back
>   (iOS swipe/button, Android system + predictive back) arrives as
>   `Event::NavBack { already_popped }` so the path signal reconciles without double-popping.
> - **Presentation** shipped as the `present`/`dismiss` duties (`PresentSpec` →
>   `PresentResult`): alert/confirm/prompt/sheets and the open/save file pickers, all native,
>   all scriptable (`assert_presented` / `respond`).
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

> **Status: shipped** (docs/shapes.md is normative), and extended beyond the sketch: the
> unified **shape pieces** (`rectangle()`, `rounded_rectangle(r)`, `circle()`, `capsule()`,
> `ellipse()`, `arc(start, sweep)`) record through the same display list with path-precise
> hit-testing for gestures, and fills take a **`Paint`** — solid color, `LinearGradient`
> (unit-space start/end points), or `RadialGradient` (unit-space center + radius, stretched
> elliptically to non-square bounds) — replayed as native gradient primitives on every backend
> (NSGradient / CGGradient / cairo / QGradient / android Shader / XAML brushes / ArkUI shader
> effects). Live transforms (`.rotate`/`.inset`/`.offset` taking closures) re-record just the
> node.

```rust
pub fn gauge(value: Signal<f64>) -> AnyPiece {
    canvas(move |d, size| {
        let r = Rect::from_size(size).inset(8.0);
        d.stroke(arc_path(r, 135.0, 270.0), Color::rgba(0.5, 0.5, 0.55, 0.35), 6.0);
        d.stroke(arc_path(r, 135.0, 270.0 * value.get() / 100.0), Color::hex(0x2F6FDE), 6.0);
        d.text(&format!("{:.0}", value.get()), r.center(), TextAnchor::Center, Font::Title);
    })
    .frame(120.0, 120.0)
    .a11y(|a| a.role(Role::Meter))
    .any()
}
```

- The closure is a **binding**: reads are tracked; any signal change re-records and re-replays just
  this node.
- `Draw` **records** into a `Vec<DrawOp>` (fill/stroke path, rect, rounded-rect, ellipse, line,
  text run, image, clip, transform, save/restore — types from `day-geometry`); the backend
  **replays natively** (`replay()` in §8.1) — CoreGraphics, android Canvas, cairo, QPainter,
  Direct2D, `<canvas>`. One FFI hop per redraw (the op buffer is a packed, pod-friendly encoding),
  not one per op — this matters on Android/JNI.
- Display lists make canvas **unit-testable on `day-mock`** (assert ops) and diffable —
  `DrawOp: PartialEq` is the §4.2 binding equality gate, so an unchanged recording skips the
  replay entirely.
- Text on canvas uses the toolkit's text engine via `DrawOp::Text` (native fonts, shaping, BiDi) —
  Day never rasterizes text. Per-toolkit shaping engines are pinned in the design because the
  defaults are traps: **PangoCairo** on GTK (cairo's "toy" text API has no shaping or BiDi),
  CoreText on apple targets, `QPainter::drawText` (harfbuzz underneath) on Qt,
  `android.graphics.Canvas.drawText` (minikin), DirectWrite via the WinUI shim.
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

> **Status: shipped with deltas** (docs/localization.md is normative). The engine is
> `day-l10n` with `day-fluent` as the app-facing API (`install_locales(default, &[(locale,
> ftl_source)])` compiles the bundles in via `include_str!`; `set_locale` switches live). The
> **preferred authoring surface is now the generated `res::str::key(args…)` functions**
> (§18.5) — typed, autocompleted, compile-checked keys — with `tr("…")` remaining for dynamic
> keys. Keys are therefore **snake_case** (they must be Rust identifiers), not kebab-case as
> sketched below. The ICU4X-backed `NUMBER`/`DATETIME` Fluent functions were **not** wired up;
> plural/`select` rules work (exercised by every locale in CI), and the `res::str` typing
> forces numeric arguments where CLDR plural selection needs them. `en-XA` pseudolocalization
> shipped; `ar-XB` did not (a real `ar` locale covers RTL, §7.8).

```rust
label(res::str::greeting(name))               // generated, typed (name: Signal<String> — live)
button(res::str::increment())
label(tr("app-title"))                        // dynamic-key escape hatch
```

- `tr(key) -> LocalizedText` implements `IntoText`. `.arg(k, v)` accepts values, signals, and
  closures; Fluent handles plurals/genders/selection.
- **Number/date formatting is NOT free**: fluent-rs registers no default `NUMBER`/`DATETIME`
  functions and does no locale-aware number rendering. `day-fluent` registers **ICU4X-backed
  functions** (`icu_decimal`, `icu_datetime` via fluent-datetime) into every bundle; M6 acceptance
  includes fr/de digit-grouping and plural-rules conformance tests, and `day lint` flags `.ftl`
  references to unregistered functions.
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
  visible string fine-grained, then one incremental relayout (§7.5's grow-never-shrink window
  policy; German is long). Each binding captures its resolved message reference once per locale —
  the per-locale parsed-bundle cache is the only cache (no (key, args) memo: Fluent args include
  `f64`, and applies are already equality-gated).
- **Per-target locale plumbing** (`--locale` must move the *whole app*, not just Day's strings):
  iOS Simulator launches pass `-AppleLanguages` via simctl; Android applies the intent-extra
  locale via `Locale.setDefault` + `createConfigurationContext` (per-app locale API on 33+) and
  routes `onConfigurationChanged` → locale signal; apple backends set `accessibilityLanguage`
  from the locale signal. Residual mixed-locale surfaces (out-of-process dialogs) are documented
  honestly.
- Fluent sources compile into the binary (`install_locales` + `include_str!`; the `.ftl` files
  under `resource/locales/` are the source of truth for the codegen, the lint, and the runtime
  alike), with per-message fallback to the default bundle. Fluent's `use_isolating` stays
  **on** (FSI/PDI isolation marks around placeables); dayscript text comparison normalizes
  U+2068/U+2069 (§14, Appendix C).
- **Native-side metadata** localization (generated `InfoPlist.strings` / `strings.xml` display
  names from reserved Fluent keys) was **not built** — the display title comes from
  `Day.toml [app] title` un-localized. The design stands for when a store submission needs it.
- Pseudolocales ship built-in: **`en-XA`** (expansion + accents) and **`ar-XB`** (RTL, §7.8).
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
- **Identifiers** (§5.5): the verified per-toolkit truth table — no pretending:

  | toolkit | native automation-id channel |
  |---|---|
  | UIKit / AppKit | `accessibilityIdentifier` ✓ |
  | WinUI | `AutomationId` ✓ |
  | Qt | `QObject::setObjectName` (surfaces as UIA AutomationId on Windows) ✓ |
  | Android | `uniqueId` via `AccessibilityDelegate` on **API 33+**, plus `setTag` for in-process use — **no external automation id below 33** (`setTag` is invisible to UiAutomator/Appium; abusing `contentDescription` for ids is forbidden by lint because TalkBack reads it aloud) |
  | GTK | widget *name* is GtkInspector-only — **no public settable AT-SPI accessible-id today** (tracked upstream) |
  | web | DOM `id` ✓ |

  dayscript is unaffected by the gaps (its element index reads day-core, §14.2); the table
  matters for *external* tools (Appium, UIA scrapers) and is documented per target.
- Policy: `day lint` a11y rules — interactive piece without a derivable label (icon-only button,
  unlabeled image) is a warning, `--strict` error; ids leaking into a11y labels is an error
  (§5.5). Focus order follows layout order; programmatic keyboard focus is its own shipped
  subsystem (`.focused()`, docs/focus.md); `.focus_group` and `.a11y_sort_priority` remain
  unimplemented.
- **Verification is automated**: the dayscript `a11y_audit` step (§14, Appendix C) walks the
  *native* accessibility tree in-process and diffs it against day-core's expectations — nothing in
  CI trusts `set_a11y` blindly.
- Reality check per toolkit lives in §9; the honest summary: primary combinations have first-class
  native a11y; `macos-gtk`/`windows-gtk` currently have none (GTK's AccessKit backend exists as of
  4.18 but isn't in default builds), which is precisely why they're secondary. Qt is solid on all
  three desktop OSes.

---

## §14 Scripting (dayscript)

### §14.1 A script

> **Status: shipped** — the showcase's real `dayscript/walkthrough.yaml` runs 200+ steps on
> every scripted target; Appendix C lists the shipped step catalog exactly.

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
`set_value`, `toggle`, `select`, `focus`), navigation (`navigate`, `nav_back`, `assert_route`),
asserting (`assert_visible`, `assert_text`, `assert_value`, `assert_focused`), dialogs
(`assert_presented`, `respond`), and evidence (`screenshot`, `a11y_audit`) — is specified in
Appendix C, with `day drive` exposing the same vocabulary to agents (docs/agent.md).

### §14.2 The embedded engine

`day-script` compiles **into the app** (cargo feature `dayscript`, on by default in debug profiles;
in release only if `Day.toml` sets `scripting.release: true` — and `day pack` verifies that
release artifacts without the opt-in contain no engine). It:

- maintains the **element index**: id → NodeId (from §5.5), plus role/text/value accessors that
  read day-core's cached last-applied props (not platform a11y trees — one implementation, all
  toolkits; the `a11y_audit` step below is the deliberate exception that reads the native tree);
- executes steps **as synthesized Day events** (tap = the button's action path; input = the
  controlled-text path), on the main thread, between flushes (`flush_sync`, §3.3) — deterministic
  and toolkit-uniform. (Driving *native* input synthesis instead is deliberately rejected for v1:
  per-toolkit event forgery is flaky and permission-gated. DP-13.)
- does **not** enforce the designed actionability preconditions (enabled/occlusion checks,
  auto-scroll-into-view) — that gating was never built; scripts scroll explicitly where needed
  and target ids they know to be interactive (Appendix C notes this per step).
- is honest about **what it cannot verify**: the native keyboard and IME, native hit-testing,
  native animations, and out-of-process UI. Manual smokes in M2/M5/M6 acceptance carry that load.
- serves the **transport** (§14.5), implements `screenshot` via `Toolkit::snapshot_window`, and
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
in-flight `Resource`s, `busy_scope()` — fell away with `Resource`, §4.5.) No sleeps in
well-written scripts; `pause` exists for demos. Text assertions normalize Fluent's FSI/PDI
isolation marks (§12.2).

### §14.4 Results

> **Status: shipped differently.** The runner is `--script` on `day launch` (exit code 5 on an
> assertion failure — the CI entry point) and `day drive` for step-at-a-time agent sessions;
> a standalone `day script` command and JUnit XML output were not built. Screenshots land in
> `build/day/screenshots/<target>/<locale-or-variant>/<name>.png` (`--variant` names themed
> sets, e.g. `--variant dark --env DAY_THEME=dark`); JSON results ride the global
> `--format json` NDJSON stream.

### §14.5 Transport and rendezvous

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
| web | WebSocket (engine in wasm connects *out* to the runner) | runner URL in query params |

The engine binds `127.0.0.1` only and is **not** a general remote-control surface: the protocol
allows only the step catalog.

---

## §15 Extensibility: pieces, parts, and tweaks

> **Status: shipped differently — and simpler.** The promise held: external crates add UI and
> platform services without touching Day or the app's scaffolds. The mechanism did not need a
> C ABI. docs/extending.md is the normative reference; docs/tweaks.md covers tweaks. The
> section title changed from "Day Piece packages (polyglot)" to match the shipped taxonomy.

### §15.1 The promise

Anyone can publish an extension crate exposing a unified Rust API whose native halves, where
needed, are written in the *platform's own language with its own conventional dependencies* —
Swift (+ SwiftPM packages) for ios/macos, Java (+ Gradle/Maven deps) for Android, C++ shims for
Qt/WinUI/ArkUI — without touching Day or the app's platform scaffolds.

The shipped ladder, cheapest first (a single package may mix rungs per toolkit):

- **Tweaks** (below composition; Addendum, docs/tweaks.md): configure the native widget behind
  an existing built-in — `Decorate::tweak`/`native_ref`, packaged as `tweaks/day-tweak-*`.
- **Tier 0 — composition:** pure Day pieces (a gauge from `canvas`, `day-piece-rating`). No
  native code.
- **Tier 1 — Rust renderers:** per-toolkit renderers written in Rust against the backend's own
  FFI (objc2 / gtk4-rs / jni / the C++ shims), registered link-time into each backend's
  `RENDERERS` slice with the `renderer!` macro (§8.2). Most `pieces/day-piece-*` crates are
  this tier.
- **Native halves:** where a piece or part needs platform-language code or third-party native
  libraries, its `Cargo.toml` declares them under **`[package.metadata.day.<platform>]`** and
  `day build` folds them into the app's native build (§15.2). Events come back through the
  standard sink (`Event::Custom { tag, num, text }` for open piece-defined events); foreign
  views enter the tree via `Toolkit::adopt`.

Two package kinds share the mechanism:

- **Pieces** (`pieces/day-piece-*`): UI — combobox, search field, picker, rating, activity,
  webview, media, map, lottie, remote-image, textarea.
- **Parts** (`parts/day-part-*`): headless platform services exposing signals/functions —
  battery, network, sensors, clipboard, prefs, haptics, deviceinfo. Same registration and
  metadata machinery, no widget.

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
gradle-dependencies = ["com.airbnb.android:lottie:6.x"]
gradle-repositories = []                   # extra Maven repos if needed
permissions = []                           # <uses-permission> entries merged into the manifest

[package.metadata.day.ios]
swift = ["ios/swift"]                      # Swift shim source dirs
swift-packages = [{ url = "https://github.com/airbnb/lottie-ios", from = "4.0.0", products = ["Lottie"] }]
```

Qt/WinUI/ArkUI native halves are C++ compiled by the crate's own `build.rs` (the `-sys`
convention, with `day-toolchain` locating SDKs) — no metadata needed. OS-API *parts* select
their half by OS (`cfg(target_os)`), so battery on `macos-gtk` gets the IOKit half, exactly the
extra-combo case the design worried about.

**Aggregation never mutates the scaffolds** — this principle shipped intact. `day build` reads
the resolved dependency graph via `cargo metadata`, collects every crate's
`[package.metadata.day.<platform>]`, and regenerates gitignored files the checked-in scaffolds
reference generically, exactly once:

- **android**: contributions land in `build/day/android/day-pieces.json`; the app's committed
  `build.gradle.kts` loops over its lists (srcDirs, dependencies, repositories) — no per-piece
  Gradle edits, ever. Permissions merge through a generated manifest overlay.
- **apple**: the CLI generates a LOCAL SwiftPM package at `build/day/ios/DayPieces` whose
  `Package.swift` lists every piece's `swift-packages` and compiles every piece's staged Swift
  shims; the checked-in `.xcodeproj` depends on that one package — adding an iOS piece is pure
  `Cargo.toml` data, no `.xcodeproj` edits. (Flutter's generated-plugin-package pattern,
  as designed — under the shipped name `DayPieces`.)

This mirrors how Flutter plugins carry `android/`/`ios/` folders the tool weaves into host
projects. It is the reason Day's scaffolds are real Xcode/Gradle projects (§17).

### §15.3 dayffi: the C ABI (superseded — never built)

> **Status: superseded.** The design specified a versioned C ABI (`DayValue` tagged trees, a
> `DayPieceVTable` with sync/async commands, `day_host_emit`, generated per-platform
> registrants) so native-language halves could implement pieces behind a stable boundary. None
> of it was needed: the shipped extension crates pair **Rust renderers** (tier 1, `adopt`ing
> native views created by their own shims) with **staged native sources**
> (`[package.metadata.day.*]`, §15.2), and the open event channel shipped as the primitive
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

Worked examples of the shipped mechanisms are in Appendix B: **ComboBox** (tier 1 — one native
control per toolkit), **Battery** (a part: headless, per-OS halves), **WebView** (commands +
events over the shipped channel), **Lottie** (bridging famous native libraries via
`[package.metadata.day.*]`).

---

## §16 The `day` CLI

### §16.1 Design goals

For humans: colorful, animated, cancellable, self-explanatory. For machines (CI, IDEs, AI agents):
deterministic, non-interactive on demand, JSON-structured, stable exit codes, discoverable
(`day --help` is complete; every command supports `--help`, and `day help --json` dumps the whole
command tree with flags and descriptions for agent consumption).

### §16.2 Crate choices

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

> **Status: shipped smaller.** The global flags are `--project <dir>` (nearest-ancestor
> `Day.toml` default) and `--format {plain,json}` (NDJSON result events); `--no-input` exists
> where prompting exists (`day new`, `day app`). `--yes`/`--color`/`-v`/`--log-file` and the
> full event vocabulary below were not built — the `result` event and stable exit codes were,
> and `day metadata --json` / `day help` cover machine discovery. The design below remains the
> target shape for a future `day daemon`.

```
--project <dir>          # default: nearest ancestor with Day.toml
--format {plain,json}    # json = NDJSON result events on stdout
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
{"event":"task.start","id":"t3","target":"android-widget","label":"gradle :app:assembleDebug","parent":"t1"}
{"event":"log","task":"t3","stream":"stdout","line":"> Task :app:compileDebugKotlin"}
{"event":"task.progress","id":"t3","detail":"compileDebugKotlin","fraction":0.61}
{"event":"task.done","id":"t3","ok":true,"seconds":24.1}
{"event":"diagnostic","severity":"warning","code":"day::lint::missing_translation","message":"…","path":"locales/fr/app.ftl"}
{"event":"result","command":"build","ok":true,"targets":[{"target":"ios-uikit","ok":true,"code":0,"artifacts":[{"path":"build/day/ios-uikit/Showcase.app"}]}]}
```

Exit codes: `0` ok · `1` failure · `2` usage · `3` environment/toolchain (doctor-able) · `4` build
failure · `5` script/assertion failure · `6` signing failure · `10` lint findings (with
`--strict`) · `130` cancelled.

### §16.4 Architecture (from flutter_tools, translated to Rust)

> **Status: the ideas shipped; the framework didn't.** The CLI is a plain clap command tree
> with per-target modules — no `CliContext` DI, no `DayCommand` envelope, no daemon. What
> survived from flutter_tools is what mattered: the **doctor workflows**, the **plumbing
> tier** (`xcode-backend`/`gradle-backend` callbacks, §17.4), and failure translation where it
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
  both `day doctor` and actionable failures ("`android-widget` needs: ANDROID_HOME, JDK 17/21 —
  found JDK 26 (known-broken with AGP; see day doctor)"). This bakes in the toolchain knowledge
  this workspace accumulated (JDK-26/Robolectric-class problems, rustup-vs-homebrew Rust for cross-std,
  cargo-ndk, `aarch64-apple-ios-sim` on Apple Silicon).
- **Plumbing tier:** stable, documented, hidden-from-default-help subcommands invoked by build
  systems: the arg-less `day xcode-backend build` / `day gradle-backend build` entrypoints
  (called by the Xcode Run-Script phase and the Gradle task, reading their parameters from the
  build system's environment — §17.4). Porcelain may change UX; plumbing changes are
  semver-relevant.
- **`day daemon --machine`** (roadmap, post-MVP): long-lived JSON-RPC for IDEs, mirroring
  flutter's daemon; the NDJSON event schema of §16.3 is designed to be reused by it.

### §16.5 Subcommands

> **Status: shipped, with a different final roster.** Of the designed set, `new`, `build`,
> `sign`, `launch`, `pack`, `lint`, and `doctor` shipped; `day script` became `--script` on
> launch plus **`day drive`**; `day clean` and `day config` were not built (machine-local
> settings ride `day doctor`'s guidance + environment variables, docs/environment.md). The
> shipped roster (`day --help` is the authority):

| command | what it does |
|---|---|
| `day version` | version, build profile, git ref |
| `day new` | scaffold an app, a **piece**, or a **part** (interactive when bare; `--no-input` for CI) |
| `day build -p <target>…` | build for one or more targets, in parallel |
| `day launch -p <target>… [--locale …] [--env K=V]… [--script <file>]… [--variant name] [--keep-alive] [--detach]` | build + install + run + stream logs; scripts imply detach and exit 5 on assertion failure |
| `day pack -p <target> [--profile release]` | build → sign → installable artifact (formats below) |
| `day sign` | signing utilities; `--check` validates `Day.toml [signing]` without printing secrets; `--notarize-status <id>` |
| `day doctor` | per-toolkit environment diagnosis with fixes |
| `day app` | add platforms/toolkits to an existing app |
| `day metadata [--json]` | machine-readable project metadata (versioned, grow-only envelope — IDE tooling consumes this, never Day.toml directly) |
| `day lint` | fluent coverage (missing/unused/unknown keys), duplicate element ids, unknown navigation routes, Day.toml schema — fast, source-level |
| `day stop` / `day relaunch` | stop running launches / stop-rebuild-relaunch ("apply my code changes") |
| `day drive` | execute dayscript steps against a RUNNING app, step-at-a-time (docs/agent.md — the agent inner loop) |
| `day mcp-server` | serve Day tools to coding agents over the Model Context Protocol (stdio) |
| `day ohos` | HarmonyOS helpers (emulator management, …; docs/harmonyos.md) |
| `day xcode-backend build` / `day gradle-backend build` | hidden plumbing the scaffolds call back into (§17.4) |

#### `day new`

Interactive when run bare (`inquire` prompts: name, id, targets, locales); non-interactive with
flags + `--no-input` for CI/agents. Templates are embedded in the CLI binary; `app`, `piece`,
and `part` scaffolds exist — the latter two produce the §15 package shapes with per-toolkit
feature wiring.

#### `day build`

Per target: (1) preflight, (2) conveyance generation from `Day.toml` (§17.5), (3) the target's
pipeline — `xcodebuild` for ios only; `gradle` for android; hvigor for ohos; **cargo + bundle
assembly for all cargo-driven desktop targets including `macos-appkit`** (their "scaffold" is a
packaging recipe, not an IDE project); MSBuild-free cargo + C++/WinRT shim for windows. The
Xcode/Gradle projects **call back** into the arg-less plumbing entrypoints (§17.4) for the Rust
staticlib/dylib, so builds started from Xcode/Android Studio are first-class and never stale.
Multiple `-p` build in parallel. Results land in `build/day/<target>/…`.

#### `day sign`

Per-format truth as designed: `.app`/`.dmg` = `codesign` + `notarytool` + `stapler`; `.apk` =
`apksigner`; `.aab` = Gradle signingConfig; ios = App Store Connect API-key signing; windows =
self-signed dev flow. Config in `Day.toml [signing]` with env-var interpolation — an unset
variable degrades that section to the dev tier LOUDLY (ad-hoc / debug keystore / self-signed),
it never fails the pack; `day sign --check` reports readiness without printing any secret.

#### `day launch`

Build (+ sign where the destination requires) + install + run + stream logs, per target, in
parallel: desktop runs the binary/bundle; ios via `simctl` with `log stream`; android via
`adb install` / `am start` with pid-scoped logcat; ohos via `hdc`. `--locale` moves the whole
app's locale; `--env` passes app environment; each `--script` runs via the embedded engine
(§14) — with scripts the command exits when the last one finishes (the CI entry point), and
`--keep-alive` keeps the session drivable via `day drive` afterwards.

#### `day pack`

`day pack -p <target> [--profile release]` = build → sign → **installable artifact**, per
target: `.dmg` (macos-appkit: sign `.app` → `hdiutil` → sign dmg → notarize → staple), `.ipa`
(ios; degrades to a zipped Simulator `.app` without App Store Connect signing config),
`.apk` + `.aab` (android), **flatpak** (linux-gtk/qt — with generated icons at the freedesktop
policy sizes), **`.msix` + an NSIS `setup.exe`** (windows), **`.hap`** (ohos via hvigor).
GTK/Qt bundling on non-native OSes remains unsupported (the extra combos are dev targets), and
the designed LGPL/licences-stage guard rails remain future work.

#### `day lint`

Built-in rules only, source-level and fast: fluent coverage (missing/unused/unknown keys across
all locales), duplicate element ids, unknown navigation routes, `Day.toml` schema validation.
`day lint` exits nonzero on findings in strict mode. The wider designed rule set (a11y labels,
bare literals, scroll nesting, RTL styling) has not been built; `res::str` (§18.5) made the
missing-key class a compile error instead.

#### `day drive` (replaces the designed `day script`)

Executes dayscript steps against a running app — one step or a JSON list per call, results as
JSON on stdout — which is the shape agents need (act, observe, decide, repeat). See
docs/agent.md; `day launch --script` covers the batch/CI case.

#### `day doctor`

Shipped as designed: per-toolkit workflows (`applicable? functional? missing?`) power both the
report and actionable failures; `day doctor --json` for agents. The toolchain knowledge lives
in `day-toolchain`, shared with the build scripts.
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

> **Status: shipped differently.** The real scaffold (below) differs from the design sketch:
> resources live under one `resource/` tree (§18.3), scripts under `dayscript/`, and the
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
    pages/                   # starter pages: home, controls, canvas, items
  resource/
    locales/en/app.ftl       # + one dir per locale
    images/app_logo.png      # processed images (§18.3); assets/ and fonts/ join as needed
  dayscript/
    smoke.yaml               # starter script; real apps grow a walkthrough
  platform/                  # only for declared mobile/ohos toolkits:
    ios/                     #   DayApp.xcodeproj + Runner (day root in a view controller),
                             #   Run-Script phase calling `day xcode-backend build` (§17.4)
    android/                 #   Gradle project; committed build files read the generated
                             #   build/day/android/*.json|properties generically (§17.5)
    ohos/                    #   hvigor project (docs/harmonyos.md)
```

Rust code layout: the app is a **lib crate** (`fieldnotes`) so mobile targets (which need
`cdylib`/`staticlib` + platform entry glue) and the desktop `main.rs` share everything. The mobile
entry glue (`#[no_mangle] JNI_OnLoad`-adjacent start fn for android; the UIKit `main` shim for ios)
is generated into the scaffolds, calling `day::launch_with(Options::from_env(), fieldnotes::root)`.

### §17.2 Why real platform projects (and not pane's hand-assembly)

pane proved the hand-assembled path (`aapt2`+`d8`+`zip` APKs, hand-written `Info.plist` bundles) —
excellent for framework CI smoke, structurally incapable of: native transitive dependencies (a
Lottie AAR, an SPM package — §15's whole point), store submission (entitlements, provisioning,
Play/App Store toolchains), and IDE escape hatches. Day therefore adopts the Flutter/Skip position
from day one: **checked-in, template-generated, thin platform projects that remain buildable by
their native tools**, with the callback hook keeping Rust fresh. The framework repo keeps a
pane-style hand-assembly harness *only* as internal CI smoke for backend development (it's cheap
and hermetic), never as the product path — this is the "no cheating" resolution of the two models.

### §17.3 `Day.toml`

> **Status: shipped with a smaller schema.** The shipped manifest keeps the principles below;
> the concrete sections in real projects are `schema`, `[app]` (id, title, build, targets —
> any property overridable per platform/toolkit/target), `[window]` (width/height/min sizes),
> and `[signing.*]` (env-var interpolated, degrade-loudly). Locales, images, assets, and fonts
> are **convention, not configuration** — the `resource/` tree is scanned (§18). The extended
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
build = 42                          # CFBundleVersion / versionCode (int, monotonic)
targets = ["macos-appkit", "macos-gtk", "macos-qt", "ios-uikit", "android-widget"]

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

- **ios/**: the Runner target's Run-Script phase is exactly **`"$DAY_BIN" xcode-backend build`** —
  arg-less plumbing that reads `CONFIGURATION`/`ARCHS`/`BUILT_PRODUCTS_DIR`/`PLATFORM_NAME` from
  Xcode's environment (flutter's `xcode_backend.sh` pattern; a fully-parameterized checked-in
  invocation would fossilize flags into user projects). Inside: configuration→cargo-profile
  mapping by case-insensitive substring with a `DAY_BUILD_MODE` override (miette error listing
  accepted names); the space-separated `ARCHS` list is split, **one cargo build per (arch, sdk),
  `lipo`'d together** (a single `--arch "$ARCHS"` is wrong for universal builds); output is the
  linked `libfieldnotes.a` (iOS requires the staticlib).
  **The template pbxproj sets `ENABLE_USER_SCRIPT_SANDBOXING=NO`** on every configuration (Xcode
  15+ defaults it to YES, which blocks the phase from writing `$BUILT_PRODUCTS_DIR` — Flutter's
  templates set exactly this), marks the Day phase `alwaysOutOfDate=1` (cargo's own incrementality
  is the freshness authority), and declares `$(BUILT_PRODUCTS_DIR)/lib<app>.a` as an `outputPath`
  for link ordering. The plumbing detects sandboxing at runtime and fails with
  `day::build::xcode_script_sandboxed` + fix instructions; `day doctor` checks it too.
- **android/**: `settings.gradle.kts` applies the **committed** `day.gradle.kts`, which registers
  a proper task class (`DayRustBuildTask`) — **configuration-cache compatible** (Gradle 9 enables
  it by default): declared inputs (target/profile/ABI list + the conveyance properties file),
  output `layout.buildDirectory.dir("day/jniLibs")` registered via `sourceSets jniLibs.srcDir`
  (**never** writing into `src/main/jniLibs` — source-tree pollution and broken up-to-date
  checks), `outputs.upToDateWhen { false }`, `ExecOperations` only inside `@TaskAction`, invoking
  the arg-less `"$DAY_BIN" gradle-backend build`. A tested Gradle/AGP version matrix is published;
  CI builds the scaffold with `--configuration-cache` (§20).
- **Freshness and fresh clones**: both callback entrypoints regenerate conveyance from `Day.toml`
  first (content-hashed, §17.5); because Xcode reads xcconfig *before* the phase runs, drift is
  detected and that build fails with "metadata changed — build again". `settings.gradle.kts`
  guards the generated `day-pieces.gradle.kts` apply with an existence check throwing "run `Day
  build` once". Committed-vs-generated is explicit: `day.gradle.kts` and a bootstrap xcconfig stub
  are **create-time committed** files; only value-bearing generated files are gitignored; the
  pbxproj references generated `.lproj` outputs via a folder reference so it never names
  gitignored files.
- Recursion guard: the plumbing entrypoints never re-enter the native build; `DAY_BUILD_PARENT`
  marks provenance for diagnostics.

### §17.5 Metadata conveyance (Day.toml → each build system)

> **Status: shipped; concrete filenames evolved.** The mechanism is exactly as designed —
> generated, gitignored, content-hashed files that committed scaffolds reference generically.
> The real names: Android reads `build/day/android/day-app.properties`, `day-signing.properties`,
> and `day-pieces.json` (§15.2); iOS conveys through the generated xcconfig + the `DayPieces`
> SwiftPM package; the Rust side's "generated metadata" became the `day-build` resource
> constants (§18.5). The `day-meta` shared library was folded into `day-cli` (its `meta`
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
  bundle `Resources/` (same folder-reference rule as §17.4's l10n).
- **android**: `day.gradle.kts` registers the generated tree as an `assets` sourceSet dir;
  lookup via `AssetManager`.
- **cargo desktop targets**: a staging dir beside the binary (bundled into `Resources/` by the
  macOS bundle recipe, `share/` on Linux, packaged content on Windows); dev runs resolve via
  `DAY_ASSET_ROOT` (§17.5).
- Uniform API: `Asset::named("stations.json").bytes() / .string() / .url()`; locale-qualified
  variants (`assets/fr/…`) resolve like Fluent fallback. The asset index is generated at build
  (into `day_meta.rs`), so `Asset::named` typos are lint-able and `day lint` cross-checks
  references. Piece-package resources aggregate per §15.2.

> **Superseded:** the shipped data API is `resource("name")` (§18.3), not `Asset::named`; the
> "generated index, lint-able typos" goal is realized as the **typed resource constants of §18.5**.

### §18.2 Icons and images

> **Status: shipped differently.** There is no SVG render pipeline (`resvg` was not adopted).
> In-app images are pre-exported PNGs under `resource/images/` (§18.3) — the Skip lesson
> (bundle the glyphs; don't rely on platform symbol names) is the working practice, with
> Material Symbols exports as the common source. The **app icon** comes from
> `resource/icons/{macos,linux,windows,png}/` PNG export sets (falling back to any root icon):
> `day pack` assembles `.icns` via `sips` + `iconutil` on macOS, `.ico` on Windows, and the
> freedesktop policy sizes (48/64/128) for flatpak — with embedded defaults so a bare project
> still packs. Dark/light theming is native per toolkit (§6.3), forced only by `DAY_THEME`.

### §18.3 Processed images + random-access data resources (docs/resources.md)

Two declared buckets — `images/` (processed images for `image("name")`) and `assets/` (arbitrary
data for `resource("name")`) — are routed through each platform's **native** resource machinery so
they inherit its optimizations and by-name load paths. Day never processes pixels itself; it hands
raw files to the native build system, which *optionally* optimizes (actool/aapt2/…). Data is stored
uncompressed where possible so `resource("name")` returns an efficient **zero-copy random-access**
view (`as_slice`/`read_at`/`len`), backed by the platform-native data API — mmap of a bundle file on
Apple, `AAssetManager` on Android, `g_resources_lookup_data` on GTK, `QResource` on Qt, rawfile fd on
ArkUI. Images map to SwiftPM `.process`→`Assets.car` (iOS), `res/drawable`→`R` (Android), GResource
(GTK), `.qrc` (Qt), MRT (WinUI), rawfile (ArkUI). Core API in `day-core::resource`; build-time
staging in `crates/day-cli/src/resources/`. Full design + per-platform detail: **docs/resources.md**.

### §18.4 Bundled custom fonts (docs/resources.md)

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
(Qt), XAML `path#family` (WinUI — unpackaged apps have no registration API). Validation is
build-time and hard: only ttf/otf, a parseable name table, no family-ident collisions. An unknown
family at runtime falls back to the system font with a log line, never a crash.

### §18.5 Typed resource constants (docs/resources.md)

Every bundled resource is also surfaced to app code as a **typed constant**, so a reference is
checked at compile time instead of failing at runtime on whichever backend can't find the name. An
app's `build.rs` calls `day_build::generate_resources()`, which scans `resource/{images,assets,fonts}`
and emits (into `$OUT_DIR`, surfaced by the scaffold's one-line `pub mod res { include!(…) }`):
`res::images::<stem>: ImageName`, `res::assets::<file>: AssetName`, `res::fonts::<family>: FontFamily`.
`image`, `resource`, and `Font::custom` take those newtypes, so `image(res::images::nav_home)` is a
build error if the file is missing and the available names autocomplete; `cargo:rerun-if-changed`
regenerates when a file is added or removed. A name known only at runtime uses the explicit
`ImageName::dynamic(…)` / `AssetName::dynamic(…)` escape hatch (a bare string literal deliberately does
**not** coerce — that is what turns "present" from convention into guarantee); the untyped
`Font::Custom(&'static str, pt)` variant remains the font escape hatch.

`day-build` (a published leaf, `day-fonts` + std only, so an app can take it as a `[build-dependencies]`)
is the **single source of truth** for the name→identifier rule (`sanitize_ident`) — the CLI stagers of
§18.3/§18.4 re-export it, so the string baked into a constant is exactly the name staged into each
backend's native store. It rejects at build time any image stem that is not portable across toolkits
(differs after sanitization — verbatim on Apple/GTK/Qt but re-sanitized on Android/ArkUI) and any two
files that collide on one symbol, each with a rename hint. This realizes §18.1's "generated,
lint-able asset index" intent for the shipped `image()` / `resource()` / `Font` APIs.

The same `build.rs` also emits a **`res::str`** bucket for localization (§12): one function per Fluent
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

---

## §19 Repository layout, examples, and docs site

> **Status: shipped differently.** The real tree:

```
day/                                # THIS repository
  Cargo.toml                        # workspace
  DESIGN.md                         # this document
  crates/                           # day, day-core, day-reactive, day-geometry, day-spec,
                                    #   day-pieces, day-fluent, day-l10n, day-script, day-mock,
                                    #   day-build, day-fonts, day-toolchain, day-cli
  toolkits/                         # day-appkit, day-uikit, day-gtk, day-qt(+sys),
                                    #   day-android, day-winui(+sys), day-arkui(+sys)
  pieces/                           # external-style UI pieces (day-piece-combobox, -searchfield,
                                    #   -picker, -rating, -activity, -webview, -media, -map,
                                    #   -lottie, -remote-image, -textarea)
  parts/                            # headless platform services (day-part-battery, -network,
                                    #   -sensors, -clipboard, -prefs, -haptics, -deviceinfo)
  tweaks/                           # packaged tweaks (day-tweak-button-bezel, -label-selectable,
                                    #   -slider-tickmarks) — Addendum, docs/tweaks.md
  apps/
    showcase/                       # THE demo: every subsystem, 4 locales, the walkthrough
    matrix/                         # a full Matrix chat client (matrix-rust-sdk bridge) — the
                                    #   scale proof; has its own DESIGN.md
    day-arkui-demo/                 # HarmonyOS host-app harness
  docs/                             # the normative subsystem docs (see the index at the top)
  website/                          # Astro site: curated guides + docs/ symlinked as the
                                    #   internal reference (scripts/website.sh builds it)
  scripts/                          # repo dev/CI helpers (axdump, screenshot validation, …)
  .github/workflows/                # ci.yml (build/test/e2e/pack/release), install.yml
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
`daybrite/actions`, publishes the reusable GitHub workflow external Day apps build with.

---

## §20 Continuous integration

> **Status: shipped, consolidated.** Instead of the designed four workflows, one `ci.yml`
> carries the whole pipeline, plus `install.yml` (scheduled end-user install checks) in this
> repo and the **`daybrite/actions`** companion repo (a reusable `build-day-app.yml` matrix
> workflow + a scaffold-validation workflow) for external Day apps.

`ci.yml`, in order:

1. **Fast checks** — rustfmt, MSRV build.
2. **CLI builds** — the `day` binary in release for 3 OSes × 2 arches; artifacts feed every
   later job (and the release lane).
3. **Per-combo jobs** (macOS: appkit/gtk/qt; Linux: gtk/qt headless; Windows: winui; plus a
   dedicated `ios-uikit` Simulator job and an Android emulator job): host-portable `cargo test`
   (incl. the day-mock e2e suite), per-backend clippy with warnings denied, `day doctor`, a
   `day new` scaffold smoke test, the **showcase walkthrough × light/dark/fr** with
   content-validated screenshot uploads, service round-trip scripts (e.g. clipboard), and
   `day pack` — with real Developer ID / notarization / ASC signing on protected runs,
   degrading loudly to dev signing on fork PRs.
4. **Release lane** (semver tags) — publishability check (`cargo publish --workspace
   --dry-run`), tag-vs-version check, GitHub release with the six CLI binaries, and crates.io
   Trusted Publishing (wired; crates not yet published — §1).

CI knowledge banked in the workflows from day one: JDK pinning, rustup toolchains for
cross-std, `--locked` everywhere, emulator boot polling, screenshot content validation
(`scripts/ci/validate-screenshots.sh`), and the freedesktop icon-size rules flatpak's
`appstreamcli` enforces.

### §20.5 Toolchain and dependency governance

> **Status: partially shipped.** The MSRV CI job and edition 2024 are real; `Cargo.lock` is
> committed. `rust-toolchain.toml` and `cargo-deny` were not adopted (kept here as intended
> future hardening).

---

# Part II — Historical record

> Everything from here to the appendices is the **completed plan**: the MVP definition, the
> milestones, the decision points, the risk register, and the adversarial-review record. It is
> kept verbatim (plus outcome notes) because it documents *why* the architecture is shaped the
> way it is — nothing in it is open work. For current status, Part I's section stamps are the
> truth.

## §21 MVP definition and milestone plan

> **Outcome: achieved and exceeded.** Every acceptance item in §21.1 passes today, and the
> M9+ roadmap items shipped too — lists, tabs, navigation, WinUI launch parity, plus systems
> the plan never named (parts, tweaks, menus, dialogs, focus, gradients, OHOS, the agent
> tooling). The walkthrough grew from the planned 13 steps to 200+. The §21.3 performance
> budget was **not** wired into CI (no frame-time assertions exist); it remains an aspiration.

### §21.1 MVP acceptance (verbatim goal)

On the current macOS host: `day launch -p macos-appkit -p macos-gtk -p macos-qt -p ios-uikit -p
android-widget` builds and launches the **showcase** app on all five targets; `day launch -p
ios-uikit --locale fr-FR --script scripts/walkthrough.yaml` runs the localized walkthrough,
passes its assertions, and produces screenshots; `day new` scaffolds a working project;
`day lint` reports fluent/a11y findings; `day pack -p macos-appkit` emits a `.dmg` and
`-p android-widget` an `.apk`; canvas renders the gauge demo natively on all five; **the showcase
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
| M5 | mobile: `day-uikit` + ios scaffold (VC-hosted root, xcode-backend callback, sandboxing off) + simctl pipeline; `day-android` + gradle scaffold (fragment-hosted root, DayRustBuildTask, configChanges) + adb pipeline; **assets/locales resource conveyance (§18.1)**; safe-area/keyboard insets (§7.7); combobox uikit/android renderers | showcase on Simulator + emulator via `day launch`; wrapping-label reflow test; **manual keyboard + iOS IME smokes** |
| M6 | `day-fluent` (tr/locale signal/negotiation/ICU4X functions/en-XA/ar-XB), a11y props + lint v0, ids, per-target locale plumbing | live locale switch + relayout benchmark; VoiceOver smoke on appkit/uikit; **`a11y_audit` green on apple targets**; fr/de number-format conformance |
| M7 | dayscript: engine, rendezvous/transport, `day script`, screenshots, JUnit; `--locale`/`--script` on launch | walkthrough (showcase-v1, no gauge step yet) passes on all 5 targets locally |
| M8a | canvas: `Draw`/DrawOp/replay on all 5 (PangoCairo/CoreText/QPainter/minikin/DirectWrite text); gauge joins showcase + walkthrough screenshot step | gauge renders natively on all 5; mock display-list tests |
| M8b | `image` piece + §18.2 icons pipeline (resvg pre-render, per-platform icon matrix) | image/icons on all 5 |
| M8c | `sign` v0 + `pack` (dmg / apk / zipped sim-.app); lint v1; site + CI complete | **MVP acceptance §21.1** |
| M9+ | list (native recycling), battery (first dayffi tier-2 proof), winui launch parity, `day upgrade`, webview→lottie→richtext, grid/tabs/nav (native containers per resolved DP-23), `day daemon`, real-device iOS, web-html experiment | — |

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

> **Outcome notes (2026-07):** every DP is closed by implementation. Where reality diverged
> from the recommendation: **DP-3/DP-4** — dayffi and its bindgen never happened; the C ABI
> was superseded by `[package.metadata.day.*]` + `Event::Custom{tag,num,text}` (§15). **DP-8**
> — the web experiment never ran; no web backend exists. **DP-9** — lists later shipped (§10).
> **DP-10** — doctor shipped; `clean`/`config` did not. **DP-22** — piece-internal
> scriptability was never needed; dayscript drives everything through Day ids. **DP-24** —
> crates.io publishing is wired but not yet executed (§1). **DP-25** — Android process-death
> restoration remains cold-restart, as accepted. The table is preserved as written.

Each had a recommendation. **DP-16 (row contract) and DP-23 (navigation) were resolved**
(owner-ratified 2026-07-01; resolutions folded into §5.4/§10). The rest resolved through
implementation as noted above.

| # | question | options | recommendation |
|---|---|---|---|
| DP-1 | ~~crates.io naming~~ | — | **superseded by DP-24** |
| DP-2 | style variation surface | `per_toolkit()` values + `style_on` (as specced) vs. only plain `match` | as specced (§6.2) |
| DP-3 | dayffi payloads | `DayValue` tagged union vs. serialized (JSON/postcard) | `DayValue`, with the JNI packed-frame exception (§15.3) |
| DP-4 | `day bindgen` codegen for polyglot stubs | v1 hand-written conventions vs. generator in MVP | hand-written v1; generator M9+ |
| DP-5 | iOS/macOS project generation | checked-in template `.xcodeproj` (flutter-style) vs. xcodegen/tuist dependency | template (no extra toolchain; scaffold-version handshake §17.3 covers evolution); revisit if pbxproj churn hurts |
| DP-6 | Windows installer | `.msix` primary + `.msi` (WiX) optional vs. msi-only | msix primary, msi optional; note Azure Trusted Signing onboarding constraints (individual/org verification, subscription) affect who can sign — §16.5's provider enum keeps alternatives open |
| DP-7 | bundling GTK/Qt into macOS/Windows apps for `pack` | support post-MVP vs. never (dev-only combos) | post-MVP support for qt (windeployqt/macdeployqt exist; **LGPL-3** obligations enforced by pack, §16.5); gtk (**LGPL-2.1+**, different obligations) stays dev-only until demand |
| DP-8 | web-html layout strategy | day-absolute-positioning (as specced) vs. hybrid with browser flow | start absolute + native `scroll`; evaluate hybrid in the experiment |
| DP-9 | `list` excluded from MVP | confirm | confirm (spec hooks reserved, §10) |
| DP-10 | extra subcommands `doctor`/`clean`/`config` | approve / reject | approve (five toolchains make doctor indispensable; config is where doctor's fixes land) |
| DP-11 | layout engine | own SwiftUI-model engine (as specced, now with measurement cache §7.4) vs. Taffy | own engine (native height-for-width measurement + proposal negotiation don't fit Taffy; hop/pane heritage de-risks it) |
| DP-12 | cross-thread signals | main-thread-only + `Setter`/`on_main` (as specced) vs. floem-style sync storage | main-thread-only v1 |
| DP-13 | dayscript event injection level | day-event synthesis (as specced) vs. native input synthesis | day-event v1; native injection as later additive step tier (Appendix C) |
| DP-14 | ~~YAML crate~~ | — | **resolved: `serde_norway`** wrapped in a shared `day_yaml` module (§16.2) |
| DP-16 | ~~row contract unification~~ | — | **resolved (owner, 2026-07-01): unified.** `each` and `list` share the `ItemSlot<T>` contract (§5.4, §10.1) — one row function serves both, same-key value changes propagate automatically (`each_diff` dropped as subsumed); validated on day-mock in M1–M2 |
| DP-17 | flush scheduling: when does the reactive drain run? | (A) synchronous fixpoint drain at batch end; layout in a coalesced posted callback (as specced §3.3); (B) always-posted flush (pane's literal model — simpler reentrancy, +1 turn latency on every event, fuzzier `wait_idle`) | **A** (already specced; this DP records the ratification) |
| DP-18 | reading a disposed signal in **release** builds | (A) panic (floem/pane precedent, fail-fast); (B) log-once + default via try-path (leptos's panic-on-read is a notorious production footgun) | **A**, paired with the `try_*` doctrine of §4.3 — silent defaults hide real bugs and the no-op-write rule already covers legitimate async races |
| DP-19 | Qt list recycling (QListView recycles delegate *paintings*, not live QWidget rows; `setIndexWidget` is unvirtualized) | (A) day-side emulated recycling: QAbstractScrollArea host + pooled cell QWidgets behind the same RowHost protocol, reported `Support::Emulated`; (B) painted `QStyledItemDelegate` fast path, rows restricted to text/icon/accessory | **A** default (preserves "any piece is a row"), B as a later optimization |
| DP-20 | SwiftPM piece halves on cargo-driven targets (macos-appkit/gtk/qt have no Xcode project) | (A) `swift build` on `DayGeneratedPieces` + generated linker-args file into the cargo link (xcodebuild becomes ios-only); (B) promote macOS to a real Xcode project (kills the seconds-fast desktop loop) | **A** (already folded into §16.5; needed for macos-gtk/qt regardless) |
| DP-21 | extensibility (pillar 4) in the MVP | (A) tier-1 combobox joins MVP acceptance; battery/dayffi defer to M9; scaffolds ship the (empty) generated-aggregator attachment points from M5; (B) A + one thin tier-2 slice (battery, apple+android) in M8 to force dayffi real before templates ossify (~1 milestone-week); (C) confirm full M9+ deferral and say pillar 4 ships unproven | **A** (folded into §21), with **B as stretch** if schedule allows |
| DP-22 | scriptability of tier-2/adopted-native piece *internals* (a ComboBox popup or WebView content is one opaque handle to the element index) | (A) optional `script_query`/`script_act` dayffi vtable entries + sub-element locator syntax (`stations-combo#item:3`) — additive, keeps pillar composition true; (B) scope the claim to root nodes + exposed props, with capability-flagged structured errors | **A** for MVP-adjacent pieces (ComboBox); at minimum §2's claim stays scoped as now written |
| DP-23 | ~~navigation architecture~~ | — | **resolved (owner, 2026-07-01): native containers.** `nav_stack` = UINavigationController / fragment+predictive-back hosts, desktop day-composed with native-style transitions (§10.5); prerequisites already in place — VC/fragment-hosted roots (§17.1) + reserved push/pop/present hooks (§10.5) |
| DP-24 | crates.io namespace (no namespacing on crates.io — RFC 3243 unshipped; `day-*` names are squattable once public; "Day" fights day.js for SEO) | (A) umbrella `day` + `day-*` crates; (B) umbrella `dayui` (SEO hedge), binary/brand stay `day`. Either way, reservation timing is its own call | **deferred by owner directive (2026-07-01): no crates.io reservation now.** Nothing in the MVP requires publishing (workspace + git deps); revisit naming + reservation together before anything is published or the design circulates publicly |
| DP-25 | Android process-death restoration (`onSaveInstanceState`) | (A) v1: documented cold restart; post-MVP opt-in persisted signals (`Signal::persist("key")`) into the state bundle; (B) design persistence into day-spec v1 now | **A** — the §9 configChanges opt-out covers the common recreation triggers; cold restart is honest for v1 and `Signal::persist` is additive later |

---

## §23 Risks

> **Outcome notes:** the two engineering risks the review weighted most — incremental relayout
> with no ancestor implementation, and native list integration — both landed (the measure
> cache and boundary re-entry live in day-core/src/layout.rs; lists in §10). The linkme/LTO
> gamble has not bitten in release builds. The M8c-density worry proved right in spirit —
> packaging absorbed the most iteration of any subsystem (flatpak icon policy, WinUI installer,
> OHOS hvigor). "No hot reload" stands, mitigated exactly as described (§16.9).

| risk | mitigation |
|---|---|
| **Scope breadth** (5 targets × 4 pillars × CLI) | milestone gating (§21.2); mock-first regression tests; MVP-adjacent tiers explicitly deferred |
| **Incremental relayout is spec-sound but ancestor-unproven** (hop re-runs its whole engine; pane relayouts the whole tree — nobody in the lineage has implemented boundary re-entry + measure cache) | M1's measure-call-count and sibling-re-proposal mock tests are the gate; §21.3 wall-time budget regression-tracked |
| Native list integration proves toolkit-hostile | deferred to M9 with spec hooks pre-reserved (§10.2 completed protocol); `scroll`+`each` is the honest fallback; Qt emulation is DP-19 |
| GTK a11y off-Linux, GTK bundling | secondary-combination framing (§9, §13); doctor probes AccessKit; DP-7 |
| Qt licensing (LGPL-3) | pack-enforced guard rails (§16.5): dynamic linking pinned, store/static configs require commercial attestation, LGPL texts + source offer bundled |
| `build-once` model surprises (signal-read-outside-binding) | **runtime debug diagnostic** (once-per-callsite, `#[track_caller]`) is the sound mechanism; lint heuristic is advisory (§4.1); mock tests encode idioms |
| linkme dead-strip under iOS release+LTO | layered registration (§8.2: generated registrant + all-profile required-kinds check) + a dedicated release+LTO CI leg — mitigated, but inherently a link-time gamble until that leg exists |
| Toolchain drift (JDK/AGP/Xcode/NDK/Gradle config-cache) | `day doctor` with pinned known-good matrices; CI encodes them (§20, §20.5); scaffold-version handshake (§17.3) |
| dayffi ABI lock-in | versioned vtables + written evolution policy (§15.3): [min,max] negotiation at registration, append-only growth, v1-pinned piece-ci cell |
| Fluent runtime cost per binding | per-locale parsed-bundle cache + per-binding resolved-message capture (no (key,args) memo — args contain `f64`); ICU4X function cost measured in M6 |
| **M8c density** (sign+pack+lint+site+CI in one gate) | already split from canvas/image (M8a/M8b); if it slips, the MVP claim slips with it — watch this milestone first |
| No hot reload disappoints Flutter refugees | honest positioning + script-replay inner loop (§16.9); research item tracked |
| dayscript blind spots (keyboard, IME, native hit-testing, animations) | §14.2 says so explicitly; manual smokes in M2/M5/M6 acceptance carry that load |

---

## §24 Adversarial review findings and resolutions

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

- **§10 list contradiction** (by-value row builder vs recycling — 3 lenses converged): rows build
  against a Copy slot handle with per-field projections; `SignalRw` for two-way controls;
  `row_kind` for reuse identifiers. `each`-unification → DP-16 (since resolved: unified as
  `ItemSlot<T>`, §5.4).
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

Runtime debug diagnostic replaces the unsoundly-promised static signal-read lint (§4.1);
measurement cache + corrected boundary/sibling-re-proposal relayout rules (§7.4); min-size from
`Proposal(0,0)` not unconstrained — the hop shrink lesson at window level (§7.5); safe-area/
keyboard-inset policy — API-35 edge-to-edge bites at M5 (§7.7); RTL threaded through the M1 engine
with `ar-XB` (§7.8); RowHost completion — `flush_now` on bind, `row_size_invalidated`,
move-lowering (§10.2); IME-safe controlled inputs — origin-tagged writes + composition gating;
pane never proved CJK (§4.4); AppKit default menu bar — Cmd+C/V/Q were broken in the flagship demo
(M2); navigation section + reserved presentation hooks (§10.5, DP-23); `AppCx::create_window`
reshape before the spec freeze (§8.1); animation `AnimSpec` parameter reserved (§8.4);
panic/catch_unwind policy (§8.5); per-toolkit a11y-id truth table — Android `setTag` is invisible
to automation, `uniqueId` is API 33+ (§13); native-tree `a11y_audit` step — nothing previously
verified `set_a11y` landed (§14.2); dayscript step tiers + actionability preconditions — no more
green taps on disabled/occluded elements (Appendix C); dayffi threading/async-command/
ABI-negotiation/JNI-packed-frame (§15.3); piece.yaml re-keyed by target selectors (§15.2);
arg-less `day xcode-backend` + configuration-cache-safe Gradle task + conveyance-drift detection +
per-target `CARGO_TARGET_DIR` + scaffold-version handshake (§16–§17); NDJSON hello/protocol
version (§16.3); CI-realistic signing — notarytool API-key auth, Windows HSM provider enum,
WinAppSDK bootstrap, fork-PR no-notarize split (§16.5, §20); Fluent `NUMBER`/`DATETIME` via ICU4X —
fluent-rs registers none by default, so French numbers rendered wrong as originally specced
(§12.2); MSRV/cargo-deny governance (§20.5); Qt-LGPL pack guards + THIRD-PARTY-NOTICES stage
(§16.5).

### Dropped

- Naming/ergonomics polish (`piece_dyn`→`dyn_piece`, crate renames): bikeshed against an already-
  coherent convention with real churn cost; the one substantive item (`.any()` on `Decorate`) was
  folded into §5.1 instead.

### Remaining risks (carried into §23)

The incremental-relayout algorithm now has a sound spec but no ancestor implemented it — M1's
op-count and wall-time mock tests are the gate. Emulated Qt list recycling (DP-19) and
piece-internal scriptability (DP-22) are accepted scope, not proven designs. linkme-under-LTO is
mitigated but remains a link-time gamble until the release+LTO CI leg exists. dayscript still
cannot see keyboards, IME, native hit-testing, or native animations — §14.2 says so, and manual
smokes carry that load. M8c remains the densest single gate even after the M8 split.

---

## Addendum (2026-07-09) — Tweaks: per-toolkit configuration of built-in pieces

Adopted post-review (owner-ratified): **tweaks** amend §15's tier ladder with a rung BELOW
composition — configuring the native widget behind an existing built-in piece, case by case,
without a new piece kind. A piece with a tweak applied is a **Tweaked Piece**. This supersedes
the earlier composition-only stance for built-ins: "call two extra methods on the real NSButton /
WinUI Button" is a legitimate, supported need that a full tier-1 renderer over-serves.

Mechanism (implemented; docs/tweaks.md is normative):
- `Toolkit::Handle: Clone + 'static`; the object-safe tree seam gains
  `node_handle_any(node) -> Option<Box<dyn Any>>` (a handle CLONE — retain / gobject ref /
  GlobalRef clone / Copy pointer). Toolkit `ext` modules downcast to their concrete handle.
- Portable surface: `Decorate::tweak(FnOnce(RNode))` (runs once at mount, post-realize — the
  §17.4/§5.2 synchronous-realize guarantee makes this sound), `Decorate::native_ref(&NativeRef)`
  (retained, liveness-checked, reactive on mount/clear transitions), and
  `day_core::invalidate_size(node)` for native mutations that change intrinsic size (§7.4's
  measure cache cannot see mutations Day didn't make).
- Per-toolkit sugar: `.appkit(…)/.uikit(…)/.gtk(…)/.android(…)` typed ext traits;
  `.qt_raw(…)/.winui_raw(…)/.arkui_raw(…)` raw tiers (the `windows` crate ships no
  Windows.UI.Xaml bindings, so WinUI hands out the borrowed ABI pointer via the existing
  `day_winui_unbox` seam; C++/WinRT recipes are the supported path).
- Native-class metadata (Level 1): every accessor also hands the closure the realized widget's
  concrete class name (`&str`), with no new trait method. Typed tiers read the live object's
  runtime class (objc `object_getClass`, GTK GType name), so a *conditional backing* — e.g. a
  plain `label` as `UILabel` vs a link-bearing one as `UITextView` — is reported accurately and a
  tweak branches instead of guessing a downcast. Raw tiers can't introspect the opaque pointer, so
  Day reads the node's kind off the same `node_kind` seam and maps it to the class it realized —
  the metadata a C++ tweak crosses the FFI with to guard its cast rather than blind-`static_cast`.
- Packaged tweaks: `tweaks/day-tweak-*` crates mirror piece crates' Cargo shape and reuse
  `[package.metadata.day.piece] backends` for §15.2's feature union. Three in-tree examples
  (button-bezel / label-selectable / slider-tickmarks) span single-toolkit trivial to
  six-toolkit with crate-owned Qt/WinRT/ArkUI native code; the showcase Tweaks page exercises
  them in CI.
- Boundaries: main-thread only; never destroy/reparent; managed properties (title, value,
  enabled, frame, a11y) may be re-applied by Day and are NOT tweak-stable; unmanaged properties
  are. Packaged tweaks must document per-toolkit coverage and no-op silently elsewhere.

---

# Appendix A — The showcase app, end to end

> **Status: superseded by the live app.** The design-era single-page sketch this appendix
> carried is long outgrown — **`apps/showcase/` is the reference**, and it is deliberately
> self-documenting: every page's source comments name the docs/ file and DESIGN section it
> demonstrates.

What the shipped showcase covers, per navigation destination (a `selector` sidebar on desktop,
a list-push on mobile — docs/navigation.md): **Controls** (every two-way binding, pickers,
search, progress/activity), **Focus** (the §4.4/docs/focus.md permutations), **Text**
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
$ day launch -p macos-appkit -p macos-gtk -p macos-qt -p ios-uikit -p android-widget
$ day launch -p ios-uikit --locale fr --script dayscript/walkthrough.yaml
$ day launch -p android-widget --locale ar --script dayscript/walkthrough.yaml   # RTL pass
$ day launch -p macos-appkit --variant dark --env DAY_THEME=dark --script dayscript/walkthrough.yaml
```

---

# Appendix B — Worked extension examples

> **Status: design-era sketches with shipped outcomes.** Each example below now exists in the
> repo; the outcome lines say what changed. docs/extending.md is the how-to.

### B.1 ComboBox (tier 1 — Rust renderers, the pane-combobox pattern)

> **Shipped** as `pieces/day-piece-combobox`, on every toolkit. The `ForeignPiece` prop-bag
> sketch became **typed props + the `renderer!` macro**:

```rust
// pieces/day-piece-combobox/src/lib.rs (as shipped)
pub fn combo_box(items: Signal<Vec<String>>, selected: Signal<Option<usize>>) -> AnyPiece { … }

// per-backend module, e.g. cfg(feature = "appkit"):
day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: ComboProps, patch: ComboPatch,
    make: make, update: update, measure: measure);
// gtk → GtkDropDown; qt → QComboBox (C++ shim); uikit → UIButton+UIMenu;
// android → MaterialAutoCompleteTextView; winui → ComboBox; arkui → select node.
```

App usage: add the crate with the matching toolkit features. No edits to day.

### B.2 Battery (tier 2 — a *service*, polyglot, no UI)

```rust
// day-piece-battery/src/lib.rs
pub fn battery() -> BatteryHandle;             // BatteryHandle { pub level: Signal<f32>, pub charging: Signal<bool> }
```
> **Shipped** as `parts/day-part-battery` — the first **part** (docs/battery.md). Per-OS Rust
> halves selected by `cfg(target_os)` (IOKit on Apple targets — including `macos-gtk`/`-qt`,
> exactly the selector case the design worried about; upower on Linux; `GetSystemPowerStatus`
> on Windows) plus a small Java shim staged via `[package.metadata.day.android]`. No dayffi:
> events re-enter through `Setter`/`on_main`, values are signals.

### B.3 WebView (tier 2 — complex: commands + events)

> **Shipped** as `pieces/day-piece-webview` (docs/webview.md): WKWebView / android.webkit /
> WebKitGTK / QWebEngineView / WebView2 / ArkUI web, driven by tier-1 Rust renderers with C++
> shims where the toolkit needs one. Navigation events ride `Event::Custom`; the
> `evaluate_js(…).await`-over-dayffi design was not needed.

### B.4 Lottie (tier 2 — bridging famous native libraries)

> **Shipped** as `pieces/day-piece-lottie` (docs/lottie.md): lottie-ios via
> `[package.metadata.day.ios]` `swift-packages`, lottie-android via
> `[package.metadata.day.android]` `gradle-dependencies` — the exact third-party-coordinate
> flow this example was designed to prove, minus `piece.yaml` (§15.2). `Cap::Lottie` gates
> support per toolkit.

### B.5 RichText (tier 2 — deep native control)

> **Not built.** The nearest shipped relative is `pieces/day-piece-textarea` (multi-line plain
> text). The rich-text design stays here as future work; nothing in the shipped extension
> mechanism blocks it.

---

# Appendix C — dayscript reference (v1)

> **Status: rewritten to the shipped catalog** (`day-script`'s `Step` enum is normative; the
> website's dayscript page is the tutorial form). The designed catalog was larger in some
> directions (locator qualifiers, `clear`/`key`/`scroll_to`/`repeat` blocks/`run_flow`,
> runner-executed `launch`/`terminate`, native-injection tiers) and smaller in others — the
> shipped one gained navigation, dialogs, and focus steps the design predates. Unshipped
> designed steps return "unknown step" errors, exactly as the step-tier plan intended.

Scripts are YAML: `name`, `description`, and a `flow:` list of steps. Every element reference
is a Day `.id()` (§5.5). Steps whose failure may resolve with time (element not found yet,
assertion pending) retry within a bounded implicit wait (5 s default) — no sleeps in
well-written scripts; `pause` exists for demos and settle-time.

| step | fields | notes |
|---|---|---|
| `wait_for` | `id` | until the element has a visible frame |
| `wait_idle` | — | flush the reactive drain |
| `tap` | `id`, `repeat?` | delivers `Pressed` AND a gesture `Tap` at the node's centre |
| `input` | `id`, `text?` \| `key?` + `args?` | `key:` resolves a Fluent key in the run's locale — locale-portable typing |
| `set_value` | `id`, `value` | sliders et al. |
| `toggle` | `id`, `value?` | omitted value = flip |
| `select` | `id`, `index` | pickers/tabs |
| `focus` | `id`, `focused?` | drives the REAL `Toolkit::focus` duty (keyboards engage); `focused: false` resigns (docs/focus.md) |
| `navigate` | `route` | reset-to semantics; `""` = root (docs/navigation.md) |
| `nav_back` | — | pop one level, the native back path |
| `assert_route` | `route` | current path |
| `assert_visible` | `id` | realized with a nonzero frame |
| `assert_text` | `id`, `text?` \| `key?` + `args?` | FSI/PDI-normalized (§12.2) |
| `assert_value` | `id`, `value` | typed per piece kind: toggle = bool, slider = number, field = string |
| `assert_focused` | `id`, `focused?` | reads the probe's focus mirror; retryable |
| `assert_presented` | `title?` | a native modal is up (docs/dialogs.md) |
| `respond` | `button?` \| `text?` \| `path?` \| `dismiss` | answer the open modal / file picker |
| `a11y_audit` | `id?` | diff the NATIVE accessibility tree against Day's expectations (§13, §14.2) |
| `screenshot` | name | waits for `ui_idle` (native transitions settled) |
| `pause` | `secs` | demos only |

Acting steps synthesize Day events (`tap` = the action path, `input` = the controlled-text
path) on the main thread between flushes — deterministic and toolkit-uniform, per DP-13. The
`focus` step is the deliberate exception that drives a real toolkit duty. The designed
actionability preconditions (enabled/occlusion checks, auto-scroll-into-view) are **not
implemented** — scripts scroll explicitly and the walkthrough is written accordingly.

---

# Appendix D — `day` CLI transcripts

> Illustrative (paths/versions/timings are representative, not verbatim); `day --help` and
> docs/cli.md (website) are authoritative.

```
$ day doctor
day 0.0.9 · project fieldnotes · targets: macos-appkit, ios-uikit, android-widget
✓ rust        1.89 (rustup) + targets aarch64-apple-ios-sim, aarch64-linux-android
✓ xcode       16.3 · simulators: iPhone 16 (booted)
✗ android     JDK 26 found — AGP requires ≤21    → brew install openjdk@21
✓ gtk4        4.16 (homebrew) · pkg-config OK
! qt6         not found — target macos-qt disabled  → brew install qt@6

$ day launch -p macos-appkit -p ios-uikit -p android-widget
  macos-appkit    cargo build … ✓ · launched
  ios-uikit       xcodebuild … ✓ · installed → launched on iPhone 16
  android-widget  gradle :app:assembleDebug … ✓ · adb install → launched on emulator-5554

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

> The build happened; these notes guided it. Kept because they explain the port order and the
> "copy pane's FFI verbatim" strategy that made eight backends tractable. Items that changed
> in flight: `day-meta` became day-cli's `meta` module + `day-build` (§17.5); the
> registrant/aggregator codegen is the §15.2 metadata mechanism; the dayffi/piece-ci items were
> superseded (§15.3).

1. Port order for `day-reactive`: start from `pane-graph` (arena, Copy handles, push-pull,
   batching) and add scopes/ownership, `Setter`, `watch`, and `bind` (floem `create_updater`
   semantics). Property tests for diamond deps, disposal-during-drain, write-during-drain,
   setter-after-dispose, the fixpoint re-run cap, and the reentrancy echo case (§4.4).
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
   reuses it at M4). Registrant/aggregator codegen (§8.2, §15.2, §15.3) is one subsystem — build
   it once with three emitters (Swift/C, Kotlin, Rust).
6. Every CI-critical toolchain fact (JDK version, `GSK_RENDERER=cairo`, rustup-not-homebrew,
   `aarch64-apple-ios-sim`, Gradle configuration-cache, `ENABLE_USER_SCRIPT_SANDBOXING`) is
   encoded in `day doctor` checks *and* asserted in CI, never tribal.
7. piece-ci runs the dayffi ASAN ownership-round-trip suite and the v1-pinned ABI cell from the
   first tier-2 piece onward (§15.3, §20).
