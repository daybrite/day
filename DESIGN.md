# Day — Design Document

**An industry-strength Rust framework for cross-platform application development with native toolkits.**

> Status: **design for review** — nothing in this document is implemented yet in this repository.
> This document is written to be sufficient to drive implementation (by a human team or an LLM) without
> access to the authors. Open questions that need a human decision are collected in §22.

---

## Table of contents

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
- §15 Extensibility: Day Piece packages (polyglot)
- §16 The `day` CLI
- §17 The Conventional Day Project and `day.yaml`
- §18 Resources, icons, and theming
- §19 Repository layout, examples, and docs site
- §20 Continuous integration
- §21 MVP definition and milestone plan
- §22 Decision points for review
- §23 Risks
- §24 Adversarial review findings and resolutions
- Appendix A: The showcase app, end to end
- Appendix B: Worked Day Piece examples (ComboBox, Battery, WebView, Lottie, RichText)
- Appendix C: dayscript reference
- Appendix D: `day` CLI transcripts
- Appendix E: Implementation notes for the builder

---

## §0 Vision, lineage, and non-goals

### §0.1 What Day is

**Day** is a Rust framework for building applications that look, feel, and behave like native
applications on every platform — because they *are* native applications. UI is authored once, in
idiomatic Rust, as a declarative tree of **Pieces** (what SwiftUI calls a View and Flutter calls a
Widget). Each Piece is realized by **real native components** — `UILabel`, `android.widget.Button`,
`NSTextField`, `GtkEntry`, `QSlider`, WinUI `TextBox`, a DOM `<input>` — through a per-platform
**toolkit** backend. Day owns layout, reactivity, localization, accessibility policy, and scripting;
the platform owns pixels, text input, scrolling physics, and assistive technology.

Eight **primary targets** (OS–toolkit combinations):

| target | OS | toolkit | tier |
|---|---|---|---|
| `ios-uikit` | iOS | UIKit | **MVP** |
| `android-widget` | Android | android.widget / android.view | **MVP** |
| `macos-appkit` | macOS | AppKit | **MVP** |
| `linux-gtk` | Linux | GTK 4 | MVP-adjacent (CI) |
| `linux-qt` | Linux | Qt 6 Widgets | MVP-adjacent (CI) |
| `windows-winui` | Windows | WinUI 3 | post-MVP (CI build) |
| `web-html` | Web (wasm32) | HTML DOM | experimental |
| `ohos-arkui` | HarmonyOS | ArkUI (NDK C API) | speculative |

Because GTK and Qt are themselves portable, the **non-default combinations** `macos-gtk`,
`macos-qt`, `windows-qt`, and `windows-gtk` are also valid targets — a target is just an
(OS, toolkit) pair whose toolkit supports that OS. The MVP (§21) proves five targets **on a single
macOS host**: `macos-appkit`, `macos-gtk`, `macos-qt`, `ios-uikit` (Simulator), and
`android-widget` (emulator).

A `day` command-line tool — deliberately modeled on the architecture of `flutter_tools`, which we
have studied in depth (`flutter/packages/flutter_tools`) — creates, builds, signs, launches, packs,
lints, and scripts Day projects, and is designed from day one for use by humans, CI, IDEs, and AI
agents.

### §0.2 Lineage — what each ancestor contributes

Day is not a greenfield guess. It consolidates several years of prior art in this workspace:

| ancestor | what Day inherits | what Day changes |
|---|---|---|
| **pane/** (Rust, 6 native backends running) | The `Backend`-trait shape with an associated `Handle`; one-toolkit-per-binary monomorphization; the open, link-time component registry (`linkme`); descriptor-carried value bindings (signal + `on_change` closure, per-widget callback tables keyed by id); the C++ shim pattern for Qt and WinUI; the JNI + Java-shim pattern for Android; the objc2 patterns for AppKit/UIKit; the headless mock toolkit for unit testing the whole pipeline | pane re-renders observing views and reconciles; Day builds the tree **once** and binds attributes reactively (§4) — no tree diffing on state change |
| **hop/** (Swift, 4 desktop toolkits) | The parent-proposes/child-chooses layout engine and its hard-won lessons (text height-for-width measurement, GTK window shrink, scroll/split interactions); AX-tree diff validation; the CI screenshot pipeline (content-validated captures, `GITHUB_STEP_SUMMARY` galleries); `hoppack`'s per-OS packaging Stage pipeline | Day's layout engine is a from-scratch Rust design informed by hop's, with an open `Layout` trait |
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
| **Piece** | Day's unit of UI composition (SwiftUI "View", Flutter "Widget"). Also the brand for extension packages: "a Day Piece". |
| **Toolkit** | A native widget system: UIKit, android.widget, AppKit, GTK 4, Qt 6 Widgets, WinUI 3, HTML DOM, ArkUI. |
| **Target** | An (OS, toolkit) pair, written `<os>-<toolkit>`: `macos-appkit`, `macos-gtk`, `ios-uikit`, … One binary is built per target. |
| **Backend crate** | The Rust crate implementing `day-spec` for one toolkit (`day-appkit`, `day-gtk`, …). One backend is linked per binary. |
| **Realized tree** | The runtime tree of mounted pieces: each node owns a native handle (or is layout-only), a reactive scope, and layout state. |
| **Signal / Memo / Effect / Scope** | The reactive primitives (§4). |
| **Day Piece package** | An external crate (plus optional per-platform native code) adding pieces or services with zero edits to Day itself (§15). |
| **dayffi** | The stable C ABI over which polyglot native code implements pieces and services (§15.3). |
| **dayscript** | The Maestro-inspired YAML UI-scripting language and its embedded engine (§14). |
| **day.yaml** | The project manifest (§17.3). |
| **Porcelain / plumbing** | User-facing CLI commands vs. stable internal commands invoked by build systems (`day xcode-backend build`, `day gradle-backend build`) (§16, §17.4). |

**Crate naming.** All crates are prefixed `day-` (`day-core`, `day-reactive`, `day-appkit`, …); the
umbrella facade crate that apps depend on is `day` with the binary tool in `day-cli` producing a
binary named `day`. crates.io naming and reservation are **DP-24** (§22; reservation explicitly
deferred by owner directive); nothing in the MVP requires publishing (workspace + git
dependencies), and the CLI binary name is independent of crate names.

**Target strings** are the canonical identifiers everywhere: `day.yaml` `targets:`, `day launch
--platform`, CI job names, screenshot directory names, `PerTarget` style values. The toolkit half
also exists alone (`uikit`, `widget`, `appkit`, `gtk`, `qt`, `winui`, `html`, `arkui`) for cases
where OS doesn't matter (styling varies by toolkit far more often than by OS).

---

## §2 The four pillars

Every Day app must be **1. localizable, 2. accessible, 3. scriptable, and 4. extensible** — and the
pillars deliberately build on each other:

1. **Localizable (§12).** Mozilla Fluent throughout. Every user-facing string in a Piece is a
   Fluent key by convention; `day lint` warns on bare user-facing literals. The current locale is a
   *signal*, so locale switches are just another fine-grained update.
2. **Accessible (§13).** Accessibility rides the platform's native accessibility tree — Day uses
   native widgets, so baseline accessibility is inherited rather than reimplemented. Day adds a
   uniform annotation API and, critically, **stable identifiers**.
3. **Scriptable (§14).** dayscript targets elements by those same accessibility identifiers — the
   accessibility pillar is the scripting pillar's addressing scheme. Scripts run against localized
   builds (`day launch --locale fr-FR --script …`), so pillar 1 × pillar 3 = automated per-locale
   screenshots and e2e tests in CI.
4. **Extensible (§15).** Everything above — pieces, services, toolkit renderers, lint rules,
   dayscript steps — is registered through open registries, so external crates (with polyglot
   native halves) participate as equals of the built-ins, including in localization (they ship
   their own `.ftl`), accessibility (they annotate through the same API), and scripting (their
   root elements and exposed props are addressable like any other; sub-element addressability
   *inside* adopted native content is DP-22).

---

## §3 Architecture overview and crate graph

### §3.1 Layers

```
┌────────────────────────────────────────────────────────────────────┐
│ app crate (user code: pieces as plain Rust functions)              │
├────────────────────────────────────────────────────────────────────┤
│ day (umbrella: prelude, launch(), re-exports)                      │
├──────────────┬──────────────┬───────────────┬──────────────────────┤
│ day-pieces   │ day-canvas   │ 3rd-party Day │ day-fluent  day-script│
│ (built-ins)  │ (Draw API)   │ Piece crates  │ (l10n)      (engine)  │
├──────────────┴──────────────┴───────────────┴──────────────────────┤
│ day-core: Piece model · realized tree · mounter · layout · events  │
├────────────────────────────────────────────────────────────────────┤
│ day-reactive (signals/memos/effects/scopes)   day-geometry (values)│
├────────────────────────────────────────────────────────────────────┤
│ day-spec: Toolkit trait · renderer registry · a11y types · dayffi  │
├───────┬───────┬───────┬───────┬───────┬───────┬───────┬────────────┤
│appkit │ uikit │android│  gtk  │  qt   │ winui │  web  │ arkui  mock│
└───────┴───────┴───────┴───────┴───────┴───────┴───────┴────────────┘
          each backend crate drives ONE native toolkit
```

### §3.2 Crates

| crate | contents | depends on |
|---|---|---|
| `day-reactive` | `Signal<T>`, `Memo<T>`, `Effect`, `Trigger`, `Scope`, batching, scheduler hook | — |
| `day-geometry` | `Point`, `Size`, `Rect`, `Insets`, `Color`, `Path`, `Transform` — plain `Copy` value types shared by layout, canvas, and the spec | — |
| `day-spec` | `Toolkit` trait, `Handle`, renderer registry, event trampoline types, `A11y` types, `DrawOp`, `DayValue` + dayffi C ABI, `TargetId`/`ToolkitId` | day-geometry |
| `day-core` | `Piece` trait + `AnyPiece`, `BuildCx`, the realized tree, the mounter, the layout engine + `Layout` trait, event routing, focus, window plumbing | day-reactive, day-geometry, day-spec |
| `day-pieces` | built-in pieces: `label`, `button`, `toggle`, `slider`, `text_field`, `column`, `row`, `stack_z`, `spacer`, `scroll`, `each`, `when`, `image`, `divider`, `list` (post-MVP), `grid`/`tabs` (post-MVP) | day-core |
| `day-canvas` | `Draw` recording context, `DrawOp` display list, `canvas()` piece | day-core, day-geometry |
| `day-fluent` | Fluent runtime: `LocaleMap`, locale signal, `tr()`, negotiation, pseudolocalization | day-reactive |
| `day-script` | the embedded dayscript engine: step executor, element index, transport server | day-core, day-fluent (for `key:` assertions) |
| `day-script-proto` | wire types shared by engine and CLI (serde) | — |
| `day-meta` | the shared metadata engine: day.yaml parsing, asset/locale scanning, conveyance-file generation — used by BOTH `day-cli` and the `day` crate's build script so `cargo build` works standalone (§17.5) | — |
| `day-mock` | headless toolkit for tests (records ops, deterministic measurement, synthetic events, in-memory "screenshots") | day-spec |
| `day` | umbrella: `prelude`, `day::launch`, feature-gated re-export of the selected backend | all of the above |
| `day-appkit`, `day-uikit`, `day-gtk`, `day-qt` (+`day-qt-sys`), `day-android`, `day-winui` (+`day-winui-sys`), `day-web`, `day-arkui` | backend crates | day-spec (NOT day-core) |
| `day-cli` | the `day` binary (§16) | day-meta, day-script-proto (+ clap, serde, YAML, fluent-syntax — §16.2) |

Two structural rules carried over from pane, both load-bearing:

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
- Background work uses `day::task::spawn(async … )` (a small executor per backend or a tokio
  runtime on desktop — backend-provided); results re-enter via `Setter` or
  `day::task::on_main(f)` where `f: FnOnce() + Send` (so it cannot capture a `Signal`; capture a
  `Setter`). Backends implement `on_main` over `dispatch_async` / `Handler.post` / `g_idle_add` /
  `QMetaObject::invokeMethod` / `DispatcherQueue.TryEnqueue` — pane already implements all five.
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
signal to an explicit scope, and `Scope::detached()` creates a manually-disposed scope. For keyed
collection state, `day-reactive` ships **`Store<K, T>`** — per-key child scopes owning item
signals, disposed on key removal, driven by the same keyed diff as `each` (the Solid `createStore`
analogue; §5.4 shows it). Debug builds diagnose the inverse leak: scope disposal warns about
ancestor-owned signals that were only ever read from the disposed subtree.

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
  in place with no extra node — and `column_vec(Vec<AnyPiece>)`/`row_vec` cover the
  runtime-heterogeneous case. `Decorate` provides `fn any(self) -> AnyPiece` for build-time
  heterogeneous branches (`if compact { a.any() } else { b.any() }`).
- **Closure capture rules**: the builder closures of `when`/`each`/`piece_dyn` are `Fn` (they may
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

```rust
// leaves — two-way controls take `impl SignalRw<T>` (implemented by Signal<T> and by
// ItemSlot::rw projections — §5.4/§10), shown here with the common Signal case:
label(text)                       // text: impl IntoText (see §12 for IntoText and tr())
button(text).action(f)            // also button_with(child_piece)
toggle(on: impl SignalRw<bool>)   // two-way
slider(value: impl SignalRw<f64>).range(0.0..=100.0).step(1.0)
text_field(text: impl SignalRw<String>).placeholder(text2).on_submit(f)
image(ImageSource::asset("logo")) // §18
divider()
spacer()                          // greedy space in a row/column
canvas(draw_fn)                   // §11

// containers
column(children).spacing(8.0).align(HAlign::Leading)
row(children).spacing(8.0).align(VAlign::Center)
stack_z(children)                 // overlay
scroll(child).axis(Axis::Vertical)            // §7.6

// structure
when(cond_fn, build_fn).or_else(build_fn)     // reactive conditional subtree
each(items_fn, key_fn, build_fn)              // reactive keyed collection (§5.4)
piece_dyn(build_fn)                           // arbitrary reactive swap (escape hatch)

// gestures (v1 surface; drag/magnify/rotate are post-MVP): mapped to native recognizers
// (UITap/UILongPress + UIContextMenuInteraction, setOnClickListener/setOnLongClickListener,
// GtkGestureClick/LongPress, Qt event filter + customContextMenuRequested, DOM events) and
// delivered as Event variants through the standard trampoline (§8.3)
.on_tap(f)  .on_long_press(f)  .on_context_menu(f)
```

Post-MVP built-ins: `list` and navigation have reserved designs in this document (§10, §10.5);
`grid`, `tabs`, `picker`, `date_picker`, `progress`, `menu`, `alert`/`sheet` are named roadmap
items whose spec-level needs are covered by the §10.5 presentation seam and the §8.1 evolution
policy — their piece-level designs are deliberately *not* claimed here.

Example — everything composed together (this is the heart of the showcase app; complete version in
Appendix A):

```rust
pub fn controls_panel() -> impl Piece {
    let name = Signal::new(String::new());
    let volume = Signal::new(40.0f64);
    let subscribed = Signal::new(false);

    column((
        label(tr("controls-title")).style(|s| s.font(Font::title())),
        text_field(name)
            .placeholder(tr("name-placeholder"))
            .id("name-field"),
        when(move || !name.with(String::is_empty),
             move || label(tr("greeting").arg("name", name)).id("greeting-label")),
        row((
            label(tr("volume-label")),
            slider(volume).range(0.0..=100.0).id("volume-slider"),
            label(move || format!("{:.0}", volume.get())).id("volume-value"),
        )).spacing(8.0),
        toggle(subscribed).id("subscribe-toggle")
            .a11y(|a| a.label(tr("subscribe-a11y"))),
    ))
    .spacing(12.0)
    .padding(16.0)
}
```

### §5.4 Keyed collections: `each`

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

```rust
label(tr("title")).style(|s| s
    .font(Font::title())
    .color(theme::TEXT)
    .padding(Insets::all(12.0))
    .background(theme::CARD)
    .corner_radius(6.0))
```

`Style` is a plain struct of `Option<T>` fields — constructible, storable, mergeable
(`base.or(overrides)`), diffable. `.style(|s| …)` is sugar over `.style_value(Style::new()…)`;
named styles are consts or fns returning `Style`. Style properties bind reactively like any other
attribute: `s.color_with(move || if error.get() { theme::DANGER } else { theme::TEXT })`.

Style properties are **honest about native limits**: each property documents its per-toolkit
mapping (e.g. `corner_radius` → CALayer / GTK CSS provider / QSS / drawable / DOM style), and
properties a toolkit cannot honor are logged once per property per run in debug builds rather than
silently dropped. The style system is *not* a CSS engine; it is a curated set of properties every
backend can implement or explicitly decline.

### §6.2 Per-target variation: `PerTarget<T>` values (no macros)

Because the toolkit is a process constant (§3.2), per-target styling is a *value* that resolves at
build time with zero runtime overhead beyond one comparison chain:

```rust
// any style parameter position accepts impl Resolve<T>:
.style(|s| s.padding(per_toolkit(12.0).uikit(16.0).qt(8.0).gtk(8.0)))
//              default ^          overrides ^

// coarser: whole-style overlays
.style(|s| s.padding(12.0))
.style_on(Toolkit::QT, |s| s.padding(8.0).font_size(13.0))

// and it is just Rust — plain control flow always works:
let pad = match Toolkit::current() {
    Toolkit::UIKIT => 16.0,
    Toolkit::QT | Toolkit::GTK => 8.0,
    _ => 12.0,
};
```

`per_target(…)` exists too (keyed by full `macos-gtk`-style targets) but `per_toolkit` is the
common case. This design descends from the `platform!{}` exploration in `pane/DESIGN.md` §4b and
`pane/DESIGN2.md`, reduced to macro-free form.

### §6.3 Semantic theme tokens

Hard-coded colors break native fidelity (dark mode, high contrast, platform accent). The `theme`
module exposes **semantic tokens** — `theme::TEXT`, `theme::SECONDARY_TEXT`, `theme::CARD`,
`theme::ACCENT`, `theme::DANGER`, … — which resolve *in the backend* to native dynamic colors
(`UIColor.label`, `?attr/colorOnSurface`, `NSColor.labelColor`, GTK/Adwaita named colors, QPalette
roles, WinUI theme resources). Concrete `Color` values remain available for brand colors. The
system appearance (dark/light) is surfaced as `theme::scheme(): Signal<ColorScheme>` for the rare
manual branch. Apps that want full custom theming set tokens app-wide via context.

Tokens are **late-bound**: backends resolve token→concrete color at every apply, and day-core
re-runs token-consuming bindings when `theme::scheme()`, density, or locale signals fire — the
same mechanism as a live locale switch. This is what makes Android dark-mode changes (delivered
as configuration changes, §9) and desktop appearance toggles work without rebuilding the tree.

### §6.4 Typography

`Font` is **semantic-first**: `Font::title()`, `Font::headline()`, `Font::body()`, … resolve to
the platform's text-style system (`UIFont.preferredFont(forTextStyle:)`, Android
textAppearance-class scaled sizes, `NSFont.preferredFont`, and documented fixed ramps on
gtk/qt/web) so **Dynamic Type / system font scaling works by default**; raw point sizes remain the
escape hatch (`Font::system(13.0)`, with a lint nudge toward semantic styles).
`env::font_scale(): Signal<f64>` exposes the user's scale; resolve-at-build is acceptable for MVP,
live tracking rides the Android configuration plumbing (§9) and Apple trait-collection
notifications post-M6. A points-first API would make Dynamic Type unfixable later; this one is
semantic-first from M2.

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

Android 15 (target-sdk 35, which `day.yaml` defaults to) makes edge-to-edge mandatory, and iOS
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

Day owns absolute placement, so **no native mirroring applies automatically** — RTL is the
engine's job and is threaded through from M1 (items 1–3), with native attributes and pseudolocale
at M6 (items 4–5):

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
   `day lint` flags physical left/right styling when day.yaml declares an RTL locale.

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

Evolution of pane's `Backend` (proven across six toolkits), extended for Day's pillars. This
listing is the **v1 surface** — everything the MVP and the reserved designs need is present, and
the evolution policy below makes later additions non-breaking:

```rust
pub trait Toolkit: Sized + 'static {
    type Handle: Clone;

    // capabilities — feature detection for pieces (§10, B.4)
    fn capability(&self, cap: Cap) -> Support { Support::Unsupported }
    // pub enum Support { Native, Emulated, Unsupported }

    // node lifecycle
    fn realize(&mut self, kind: PieceKind, props: &dyn Props, id: NodeId) -> Self::Handle;
    fn update(&mut self, h: &Self::Handle, kind: PieceKind, patch: &dyn Props, anim: Option<&AnimSpec>);
    fn release(&mut self, h: Self::Handle);   // called from the turn-boundary release queue;
                                              // backends may defer further (Qt deleteLater)

    // tree
    fn insert(&mut self, parent: &Self::Handle, child: &Self::Handle, index: usize);
    fn remove(&mut self, parent: &Self::Handle, child: &Self::Handle);
    fn move_child(&mut self, parent: &Self::Handle, child: &Self::Handle, to: usize);

    // geometry (§7)
    fn measure(&mut self, h: &Self::Handle, p: Proposal) -> Size;
    fn set_frame(&mut self, h: &Self::Handle, frame: Rect, anim: Option<&AnimSpec>);

    // scroll (§7.6)
    fn set_scroll_content(&mut self, h: &Self::Handle, content: Size);
    fn scroll_to(&mut self, h: &Self::Handle, target: Rect, animated: bool);
    fn scroll_offset(&mut self, h: &Self::Handle) -> Point;

    // events: one trampoline, node-id keyed (pane's design). CONTRACT: the sink may be invoked
    // re-entrantly from inside any Toolkit method (Qt/GTK/Android setters fire synchronously)
    // and MUST only enqueue — day-core drains queued events at safe points as fresh batches.
    fn set_event_sink(&mut self, sink: Box<dyn Fn(NodeId, Event)>);

    // pillars
    fn set_a11y(&mut self, h: &Self::Handle, a11y: &A11yProps);          // §13
    fn replay(&mut self, h: &Self::Handle, ops: &[DrawOp], size: Size);  // canvas §11
    fn snapshot_window(&mut self) -> Result<Png, SnapshotError>;         // dayscript §14

    // native list hosting (§10) — defaulted; backends return None until M9
    fn list_host(&mut self) -> Option<&mut dyn ListHost> { None }

    // app lifecycle (mobile; desktop backends no-op)
    fn on_suspend(&mut self) {}
    fn on_resume(&mut self) {}
    fn on_memory_warning(&mut self) {}

    // adoption of foreign native handles (polyglot pieces, §15; ownership table in §15.3)
    fn adopt(&mut self, raw: RawHandle) -> Self::Handle;
}

pub trait Platform: Toolkit {
    const TARGET: TargetId;                 // e.g. "macos-gtk" — a process constant
    fn run(self, app: impl FnOnce(&mut AppCx<Self>));   // owns the main loop
    fn on_main(&self, f: Box<dyn FnOnce() + Send>);
    fn locale_hints(&self) -> Vec<LanguageIdentifier>;  // ORDERED OS preference list (fluent-langneg needs the list)
}

// windows are created through AppCx, not baked into run() — alerts/sheets/menus/multi-window
// all flow through window/scene creation, so the seam exists from v1 even though v1 backends
// may support exactly one live window (clear error otherwise). day::launch(root) remains the
// one-line wrapper. On mobile, "window" maps to the scene/activity content (§9).
impl<P: Platform> AppCx<P> {
    pub fn create_window(&mut self, options: WindowOptions, root: impl Piece) -> WindowId { … }
}
```

**Evolution policy:** every duty added after M2 ships with a default no-op/`Unsupported` body, so
post-freeze additions are non-breaking; `day-spec` is additive-only from M2 onward.

`Props` is `&dyn Any` downcast to the piece's typed descriptor (e.g. `LabelProps`) — **zero
serialization between Rust and Rust-implemented backends**; patches are sparse (only changed
fields). Only the polyglot boundary (§15) encodes, and then into `DayValue`, not JSON.

### §8.2 The open renderer registry

Registration is **layered** so that `linkme` is a convenience, not a correctness mechanism (the
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
    Pressed,                       // button
    TextChanged(String), Submitted,
    ToggleChanged(bool),
    ValueChanged(f64),             // slider et al.
    FocusChanged(bool),
    Tap(Point), LongPress(Point), ContextMenu(Point),   // §5.3 gesture decorators
    ScrollChanged(Point),          // §7.6
    Key(KeyEvent), Pointer(PointerEvent),   // canvas + custom pieces
    Custom(PieceKind, DayValue),   // polyglot piece events
}
```

The single sink keeps the backend ignorant of closures/lifetimes (day-core owns the `NodeId →
handlers` table) — this is the shape that made pane's six backends small. The sink contract is
enqueue-only (§8.1); handlers run under their registration scope (§4.3).

### §8.4 Animation (reserved hooks, v1; implementation post-MVP)

Native-widget frameworks that bolt animation on later end up breaking their backend ABI — so the
seam ships now even though MVP backends ignore it. Day commits to **backend-executed animation**:
Day passes *intent*, the platform animates (consistent with §0.3 — Day never ticks pixel frames
for native widgets). `AnimSpec { duration, curve, spring }` parameters already sit on `set_frame`
and `update` (§8.1), no-op in MVP backends. The post-MVP surface (design sketch, not v1 API):
`.transition(anim)` on `when`/`each` enter/exit, animated frame changes
(`with_animation(anim, || …)`), and a day-driven frame-clock ticker **for canvas only**.

### §8.5 Panics and crashes

A panic unwinding out of an `extern "C"` / ObjC / JNI frame aborts the process with no useful
report, so this policy is v1:

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

Shared mechanics come from pane's working code (FFI choices are *proven*, not aspirational):

| backend | FFI mechanism | container | status of precedent |
|---|---|---|---|
| `day-appkit` | `objc2` (`objc2-app-kit`) | `NSView` (flipped) | pane runs today |
| `day-uikit` | `objc2` (`objc2-ui-kit`) | `UIView` | pane runs today (Simulator) |
| `day-gtk` | `gtk4-rs` | fixed-pos container w/ custom `GtkLayoutManager` | pane + hop run today (incl. macOS host) |
| `day-qt` | `cc`-built C++ shim (`day-qt-sys`) | bare `QWidget` | pane + hop run today (incl. macOS host) |
| `day-android` | `jni` + a small Java/Kotlin shim (`DayBridge`) | absolute-layout `ViewGroup` | pane runs today (emulator) |
| `day-winui` | C++/WinRT shim (`day-winui-sys`, cppwinrt-staged headers) | `Canvas` panel | pane builds in CI today |
| `day-web` | `wasm-bindgen`/`web-sys` DOM | absolutely-positioned `<div>` | new (experimental) |
| `day-arkui` | ArkUI **NDK C API** (`arkui/native_node.h`, OHOS Rust targets exist: `aarch64-unknown-linux-ohos`) | ArkUI custom container node | new (speculative) |

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
  backend routes `onConfigurationChanged` into Day's signals — dark mode → `theme::scheme()`
  (tokens are late-bound, §6.3), locale → the locale signal (§12), density → measure-cache epoch
  bump + frame re-multiplication (§7.9), font scale → `env::font_scale()`. The
  suspend/resume/memory hooks (§8.1) map to the Activity callbacks. Process-death state
  restoration (`onSaveInstanceState`) is **DP-25** — v1 documents cold restart.
- **Windows App SDK runtime.** An unpackaged WinUI 3 app fails at process start without the
  runtime: `day-winui-sys` calls `MddBootstrapInitialize2` at startup with the WinAppSDK version
  pinned in `day.yaml` (`windows.app-sdk`); `day doctor` checks runtime presence; `day pack`'s
  msix flavor declares the framework-package dependency, while the unpackaged/msi flavor chooses
  between chaining `WindowsAppRuntimeInstall.exe` and `WindowsAppSDKSelfContained=true` (size
  trade-off documented). pane's shim already does the bootstrap call.

On mobile, the §8.1 "window" maps to the scene / activity content view: `AppCx::create_window` is
the seam through which scene-based multi-window arrives later without reshaping the trait.

**Extra combinations** (`macos-gtk`, `macos-qt`, `windows-qt`, `windows-gtk`) need no extra code in
the backend crates — GTK/Qt are portable; the *target* differs only in build/packaging (§16, §17:
where the toolkit libraries come from and whether `day pack` can bundle them; bundling GTK/Qt into
a redistributable macOS/Windows app is real work and is explicitly **post-MVP**, DP-7). The
`day.yaml` `targets:` list and `day doctor` gate which combinations a project claims.

**web-html (experimental) sketch:** wasm32 binary; pieces map to semantic elements
(`<button>`, `<input>`, `<label>`); Day layout emits `position:absolute; transform:translate(…)`
placements; text measurement via a hidden measurement element or `canvas.measureText` (cached);
events via `wasm-bindgen` closures; scripting transport is a `WebSocket` (§14.5). The open
question — whether absolute placement forfeits too much of the browser (text selection across
elements, native scrolling) — is recorded as DP-8 with a proposed hybrid (Day layout, but `scroll`
maps to overflow scrolling).

**ohos-arkui (speculative) sketch:** ArkUI exposes a C node API in the NDK
(`OH_ArkUI_…`, `ArkUI_NativeNodeAPI_1`: create node, set attributes, register event receivers,
custom layout/measure hooks) — the same shape as day-spec, which suggests a thin backend. Rust
`*-ohos` targets are tier 2. No local device/emulator commitment in this plan; the section exists
to keep the spec honest about an eighth shape of toolkit.

---

## §10 Native list integration

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

This is the single hardest backend feature; it is **post-MVP by design** (M9), but the spec-level
hooks (`ListHost`, `bind_row`, the data source, `row_size_invalidated`) are part of `day-spec` v1
so backends don't need breaking changes later. Until then, `scroll(column(each(…)))` covers small
collections honestly.

### §10.5 Navigation and presentation (reserved design)

Navigation is where native-widget frameworks live or die (React Native spent a decade converging
on react-native-screens because a JS-composed stack never felt native). Day's **resolved**
position (**DP-23**: native containers) is: `nav_stack` maps to `UINavigationController` (one
child view controller per page: back-swipe, large titles, transitions for free) and a
predictive-back-compatible fragment/back-dispatcher host on Android; desktop composes a
day-driven stack with native-style transitions. Two things happen *now* even though navigation
itself ships post-MVP:

1. `day-spec` v1 reserves the **presentation seam**: `push/pop/set_stack` +
   `present_sheet/present_alert/popup_menu` duties (defaulted `Unsupported` per §8.1's evolution
   policy) — alerts, sheets, menus, and navigation all flow through it later.
2. The M5 iOS/Android scaffolds host Day's root **inside a view controller / fragment** (not a
   bare view), so native nav containers are possible without a scaffold migration.

---

## §11 Canvas

```rust
pub fn gauge(value: Signal<f64>) -> impl Piece {
    canvas(move |d: &mut Draw, size: Size| {
        let r = Rect::from(size).inset(4.0);
        d.stroke(Path::arc(r, 135.0, 270.0), theme::SEPARATOR, 6.0);
        d.stroke(Path::arc(r, 135.0, 270.0 * value.get() / 100.0), theme::ACCENT, 6.0);
        d.text(&format!("{:.0}", value.get()), r.center(), TextStyle::title().centered());
    })
    .frame(120.0, 120.0)
    .a11y(|a| a.role(Role::Meter).value_with(move || value.get().to_string()))
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
locales/
  en/app.ftl        # default locale (day.yaml localization.default)
  fr/app.ftl
  de/app.ftl
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

```rust
label(tr("greeting").arg("name", name))       // name: Signal<String> — live
button(tr("increment"))
label(tr("app-title"))
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
- `day lint` warns on literal user-facing text, `tr()` keys missing from the default locale,
  unused keys, and locales missing keys (`--strict` for CI).
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
- Fluent bundles load from packaged resources (§18, staged into M5) at startup, with per-message
  fallback to the default bundle. Fluent's `use_isolating` stays **on** (FSI/PDI isolation marks
  around placeables); dayscript text comparison normalizes U+2068/U+2069 (§14, Appendix C).
- **Native-side metadata** also localizes: `day build` generates `InfoPlist.strings` /
  `strings.xml` entries for the app display name and OS-facing strings (permission prompts) from
  reserved Fluent keys (`app-title`, `permission-camera`, …) — conveyance mechanics in §17.5. A
  lint rule requires reserved keys to be placeable-free plain text.
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
  (§5.5). Focus order follows layout order; `.focus_group` and `.a11y_sort_priority` planned
  post-MVP.
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

```yaml
# scripts/walkthrough.yaml
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

Full step catalog (tap, long_press, input, clear, set_value, toggle, scroll_to, swipe, key, back,
wait_for, wait_idle, assert_*, screenshot, pause, repeat, run_flow for composition, launch/terminate
app control) is specified in Appendix C.

### §14.2 The embedded engine

`day-script` compiles **into the app** (cargo feature `dayscript`, on by default in debug profiles;
in release only if `day.yaml` sets `scripting.release: true` — and `day pack` verifies that
release artifacts without the opt-in contain no engine). It:

- maintains the **element index**: id → NodeId (from §5.5), plus role/text/value accessors that
  read day-core's cached last-applied props (not platform a11y trees — one implementation, all
  toolkits; the `a11y_audit` step below is the deliberate exception that reads the native tree);
- executes steps **as synthesized Day events** (tap = the button's action path; input = the
  controlled-text path), on the main thread, between flushes (`flush_sync`, §3.3) — deterministic
  and toolkit-uniform. (Driving *native* input synthesis instead is deliberately rejected for v1:
  per-toolkit event forgery is flaky and permission-gated. DP-13.)
- **enforces actionability**: before any acting step, the target must be enabled ∧ visible ∧
  within all ancestor scroll viewports ∧ topmost at its center per Day's z-order hit-test —
  failures name the blocker ("occluded by #settings-sheet"); acting steps auto-`scroll_to` the
  target first (the showcase's toggle sits below the fold on phones).
- is honest about **what it cannot verify**: the native keyboard and IME, native hit-testing,
  native animations, and out-of-process UI. Manual smokes in M2/M5/M6 acceptance carry that load.
- serves the **transport** (§14.5), implements `screenshot` via `Toolkit::snapshot_window`, and
  implements **`a11y_audit`**: walk the *native* accessibility tree in-process
  (NSAccessibility/UIAccessibility — hop's proven recipe; `AccessibilityNodeInfo` on Android;
  GtkAccessible/QAccessibleInterface where present), diff role/label/identifier against day-core's
  expectations for every node with an `.id()`, and report through the normal step-result path.
  Required in M6 acceptance and the CI walkthrough on apple targets.

### §14.3 Waits and flakiness

Every locator step has an implicit bounded wait (default 5s, configurable) that polls after
`wait_idle`. **`wait_idle` is precisely defined**: no pending reactive drain ∧ no dirty layout ∧
zero in-flight `Resource`s (auto-registered) ∧ no open `day::script::busy_scope()` (the app-side
escape hatch for custom async). Backends best-effort disable native animations in script mode.
No sleeps in well-written scripts; `pause` exists for demos. Text assertions normalize Fluent's
FSI/PDI isolation marks (§12.2).

### §14.4 Results

The runner (`day script`, or `--script` on launch) reports per-step pass/fail with timings,
produces `--format json` NDJSON events, **JUnit XML** (`--junit out.xml`; one
`<testsuite name="<target>/<locale>/<script>">` per combination, steps as timed `<testcase>`
entries) for CI, and writes screenshots to `build/day/screenshots/<target>/<locale>/<name>.png`.

### §14.5 Transport and rendezvous

`day-script-proto` defines a versioned, length-prefixed JSON message protocol (hello/capabilities,
run_step, step_result, screenshot chunks as binary frames).

**Rendezvous** (five parallel targets share the host loopback — fixed ports are a design bug):
the engine binds **only when invited** — `DAYSCRIPT_PORT`/`DAYSCRIPT_TOKEN` present in the
environment (`SIMCTL_CHILD_*` for the Simulator) or intent extras (Android) — never otherwise,
debug or release. It binds **port 0** (OS-assigned), then writes a handshake file
`build/day/run/<target>.json` `{port, token, pid, target}` (pulled over adb on Android). The
launcher generates the one-time token; wrong/missing token → connection refused.
`day script --attach <target>` reads the same session registry.

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

## §15 Extensibility: Day Piece packages (polyglot)

### §15.1 The promise

Anyone can publish a **Day Piece** — a crate exposing a unified Rust API whose implementation on
each toolkit may be written in the *platform's own language with its own conventional build
structure*: Swift (SwiftPM) for ios/macos, Kotlin/Java (Gradle module) for Android, C++ (CMake) for
Qt/Windows, C for GTK — with **zero edits to Day or to the app's platform scaffolds**.

Three implementation tiers, cheapest first (a single package may mix tiers per toolkit):

- **Tier 0 — composition:** pure Day pieces (a `Gauge` from `canvas`). No native code.
- **Tier 1 — Rust-native:** per-toolkit renderers written in Rust against the backend's own FFI
  (objc2/gtk4-rs/jni/…), registered via the §8.2 registry. This is pane's `pane-combobox` pattern,
  running today.
- **Tier 2 — polyglot:** native-language implementation behind **dayffi** (§15.3). This is the tier
  that makes "wrap Lottie-iOS / a RecyclerView library / NSTextView" a normal package.

### §15.2 Package layout (tier 2)

```
day-piece-lottie/
  Cargo.toml            # the Rust API crate (features per toolkit)
  src/lib.rs            # pub fn lottie(source) -> impl Piece  + registration
  piece.yaml            # native build + link metadata consumed by `day build`
  platform/
    apple/              # SwiftPM package (ios + macos): wraps lottie-ios (SPM dep)
      Package.swift
      Sources/DayLottie/DayLottie.swift
    android/            # Gradle library module: wraps lottie-android (maven dep)
      build.gradle.kts
      src/main/kotlin/dev/daybrite/day/lottie/DayLottie.kt
    qt/                 # optional: CMake + C++ (rlottie), used by *-qt targets
      CMakeLists.txt
  locales/en/lottie.ftl # packages localize their own strings
```

`piece.yaml` declares, **keyed by target selector** with §6.2's fallback precedence (exact target
> OS family > toolkit > default — "per toolkit" alone cannot express the mandated extra combos:
Battery on `macos-gtk` needs the IOKit *apple* half, not a upower-DBus *gtk* half, while widget
renderers select by toolkit): how to build (`swiftpm | gradle | cmake | none`; a `js` kind is
reserved in the schema and rejected with a clear error until web tier-2 exists), what to link,
transitive native deps (SPM/Maven/pkg-config coordinates), **license/notices metadata** (Maven and
SPM obligations are invisible to `cargo metadata` — §16.5 pack's licenses stage needs this), and
the unique per-piece registration entry points (§15.3). OS-API service halves select by OS;
widget renderers select by toolkit; piece-ci exercises one extra-combo case (battery on
`macos-gtk`) to keep the precedence honest.

**Aggregation never mutates the scaffolds.** `day build` resolves the app's piece graph via
`cargo metadata --filter-platform <triple>` restricted to the active target's resolved feature set
(bare `cargo metadata` would drag in inactive, target-gated pieces), then regenerates
**generated, gitignored files that the immutable checked-in scaffolds reference exactly once**:

- apple: a single local SwiftPM package **`DayGeneratedPieces`** — the template `.xcodeproj`
  references it from day one (Flutter's `FlutterGeneratedPluginSwiftPackage`, verbatim); its
  regenerated `Package.swift` lists the aggregated piece dependencies;
- android: `settings.gradle.kts` applies a generated **`day-pieces.gradle.kts`** using `include` +
  `projectDir` substitution (no `includeBuild` — composite builds fight AGP);
- qt/cmake: built by `day build` and linked via the `-sys` build-script conventions;
- **resources**: piece `locales/` and declared assets are copied into per-package subtrees of the
  generated resource index — each package resolves `tr()` against its own bundle set with
  app-bundle override (namespacing without key prefixes), and `day lint` attributes keys to their
  owning packages.

Publishing hygiene: the piece template's `Cargo.toml` `include` covers
`piece.yaml`/`platform/`/`locales/`, with a `cargo package --list` check in piece-ci (a crate that
forgets to package its native halves fails CI, not its first user). Tier-2 dayffi does **not**
apply to `web-html` in v1 (DOM handles aren't C pointers; native halves there are JS modules) —
web pieces are tier 0/1 until the reserved `js` kind is designed.

This mirrors — deliberately — how Flutter plugins carry an `android/` and `ios/` folder that the
tool weaves into host projects, and how skip.yml declares per-module Gradle deps. It is the piece
of Skip/Flutter that pane's hand-assembled MVP packaging could not express, and it is the reason
Day's scaffolds are real Xcode/Gradle projects (§17).

### §15.3 dayffi: the C ABI

A deliberately small, versioned, stable C ABI. Sketch (the full header, with the ownership
contract as normative comments, ships at `crates/day-spec/include/dayffi.h` in the same milestone
as the first tier-2 piece):

```c
typedef struct DayValue DayValue;      // OPAQUE tagged value tree: null/bool/i64/f64/str/bytes/list/map
// ALL construction & inspection through day-exported day_value_* functions (single allocator —
// also the fix for Windows cross-CRT frees): day_value_new_map()/…, day_value_get()/…,
// day_value_clone(), day_value_free().

typedef struct DayPieceVTable {
  uint32_t abi_version;                // the version this piece was COMPILED against
  void* (*make)(const DayValue* props, DayHost* host, uint64_t node_id);  // returns native view/widget
  void  (*update)(void* self, const DayValue* patch);
  void  (*measure)(void* self, double w, double h, int w_set, int h_set, double out[2]); // OPTIONAL:
        // NULL ⇒ the backend's generic native measurement of the adopted handle; non-NULL wins
  void  (*command)(void* self, const char* name, const DayValue* args, DayValue** out); // callee
        // allocates *out via day_value_*; host frees with day_value_free
  void  (*command_async)(void* self, const char* name, const DayValue* args, uint64_t completion_id);
  void  (*destroy)(void* self);
} DayPieceVTable;

// host services (DayHost* is valid for the process lifetime):
void day_host_emit(DayHost*, uint64_t node_id, const char* event, const DayValue* payload);
void day_host_complete(DayHost*, uint64_t completion_id, DayValue* result, const char* error);
void day_host_post(DayHost*, void (*fn)(void*), void* ctx);        // schedule on the main thread
void* day_host_get_proc(DayHost*, const char* name);               // capability-queried extensions
```

**Ownership contract (normative):** inputs (`props`, `patch`, `args`) are **borrowed for the call
duration** — callees deep-copy via `day_value_clone` to retain; `command`'s output is
callee-allocated, host-freed; `day_host_emit`/`day_host_complete` payloads are consumed/copied by
the host before returning. An ASAN/leak ownership-round-trip suite runs in piece-ci.

**Threading contract (normative):** all `DayPieceVTable` calls occur **only on the toolkit main
thread**. `day_host_emit` and `day_host_complete` are callable from **any** thread — the host
deep-copies synchronously, then enqueues via `on_main`. Emit against a disposed `node_id` is a
defined silent drop (debug-logged), which makes scope teardown race-free. `day_host_post` lets
native halves schedule main-thread work. (B.2's Battery gets OS notifications on arbitrary queues —
this contract is exercised by the first real piece.)

**Async commands are v1**, not an afterthought: B.3's `evaluate_js(…).await` cannot be built on a
synchronous `command` (WKWebView/WebView2/android.webkit are completion-handler-only; blocking the
main thread deadlocks). Rust surfaces `command_async` as a future; sync `command` remains for
cheap getters.

- `make` returns the **raw native handle**, which the active backend **adopts**
  (`Toolkit::adopt`, §8.1) — thereafter it is placed, framed, measured, snapshotted, and
  a11y-annotated like any built-in (pane's `WidgetComponent.makeNative` / hop's
  `HopRepresentable` pattern, generalized). **Adoption ownership table (normative):** ObjC objects
  arrive +1-retained; a `jobject` arrives as a *local* ref that the DayBridge JNI adapter promotes
  to `NewGlobalRef` **before** Rust sees the `RawHandle` (release = `DeleteGlobalRef` — pane's
  exact pattern); `GtkWidget` is `g_object_ref_sink`'d; `QWidget` is parentless and day-owned
  until inserted (parent-ownership transfer documented). Teardown ordering: layout detach →
  toolkit deparent → `vtable.destroy` → backend drops its ref.
- `DayValue` crosses by pointer on C-ABI platforms — no text serialization anywhere. **JNI is the
  honest exception**: per-field JNI downcalls would be worse than one serialization pass, so
  DayBridge transports DayValue trees as **one packed binary frame** (direct `ByteBuffer`, the
  same encoding family as the DrawOp buffer) with a small pure-Kotlin codec producing idiomatic
  types. The claim is precisely: *no serialization except where the platform boundary demands it
  (JNI, wasm), and never text-based.* Patches are sparse (changed keys only), so fine-grained
  updates stay fine-grained across the boundary.
- **Rich logic on either side**: `command`/`command_async` (Rust→native) and `day_host_emit`
  (native→Rust, arriving as `Event::Custom` and dispatched to the piece's typed event closures)
  form the bidirectional channel. The Rust API crate wraps both in typed methods; app code never
  sees `DayValue`.
- **Registration is generated, not discovered.** A fixed exported symbol
  (`day_register_pieces`) would be a guaranteed duplicate-symbol link failure under iOS's
  mandatory static linking, and "linker section / JNI static block" discovery is
  dead-strip-fragile. Instead — Flutter's generated-registrant pattern: `piece.yaml` declares
  per-platform **unique** entry points (a C symbol per piece for apple/C/C++; a factory-class FQN
  for Kotlin); `day build` generates per-target registrants (a Swift/C extern-decl+call file
  inside `DayGeneratedPieces`; a `DayGeneratedRegistrant` class loaded by DayBridge; a Rust
  extern-C call list for cargo-driven desktop targets) invoked once during Toolkit init on the
  main thread. Deterministic, dead-strip-proof, and failure is a *link error*, not a runtime
  surprise.
- **ABI evolution policy** (embedded in the v1 helpers): registration passes the piece's
  compiled-against version; the host supports a declared `[min, max]` and rejects out-of-range
  registrations at startup with a diagnostic naming the package and the required Day version
  (surfaced through the §8.2 missing-renderer report, never a crash). VTables grow append-only
  (size implied by version; the host never reads past the registrant's declared version). Unknown
  `DayValue` tags are a hard per-value error, checked by the helpers from v1. piece-ci runs a
  v1-pinned piece against the current host as a permanent matrix cell.
- Per-language conformance is **hand-written against documented conventions in v1** (a tiny Swift
  package `DayFFI`, the Kotlin `DayBridge` helper, C++ headers ship alongside the header); a
  `day bindgen` generator (Rust trait → Swift protocol/Kotlin interface stubs) is roadmapped
  (DP-4) — the ABI is designed so the generator is an ergonomic layer, not a semantic one.

Worked examples in Appendix B: **ComboBox** (simple: one native control each on
appkit/uikit/gtk/qt/android), **Battery** (a *service*, no UI: the same ABI minus view adoption —
`DayServiceVTable` with `call`/`subscribe`), **WebView** (complex: commands, events, delegates),
**Lottie** (bridging famous third-party native libraries), **RichText** (deep native control:
NSTextView/UITextView with a Rust-side document model).

---

## §16 The `day` CLI

### §16.1 Design goals

For humans: colorful, animated, cancellable, self-explanatory. For machines (CI, IDEs, AI agents):
deterministic, non-interactive on demand, JSON-structured, stable exit codes, discoverable
(`day --help` is complete; every command supports `--help`, and `day help --json` dumps the whole
command tree with flags and descriptions for agent consumption).

### §16.2 Crate choices

| concern | crate | notes |
|---|---|---|
| argument parsing | `clap` v4 (derive) + `clap_complete` | shell completions ship in `day create`'s CI template |
| progress | `indicatif` (MultiProgress) | auto-disabled when not a TTY or `--format != auto` |
| colors/terminal | `anstream`/`owo-colors` | honors `NO_COLOR`, `--color {auto,always,never}` |
| diagnostics | `miette` | every error has a code (`day::build::gradle_jdk_mismatch`), a message, and a help footer; `gradle_errors.dart`-style translation tables for xcodebuild/gradle failures |
| logging | `tracing` + `tracing-subscriber` | `-v/-vv`, `--log-file`, `DAY_LOG=…` env filter |
| async + processes | `tokio` (process, signal) | per-OS cancellation spec below |
| interactive prompts | `inquire` | only in `create` and confirmation points; disabled by `--no-input` |
| YAML | `serde` + **`serde_norway`** (maintained `serde_yaml` fork; `serde_yml` is a distrusted automated fork, `serde_yaml_ng` dormant) | wrapped in one `day_yaml` module in a shared crate — day-cli, day-script, and the piece.yaml aggregator all parse YAML; manifests parse to a `Value` first for closed-schema walks with miette source locations; YAML 1.2 core-schema semantics required (DP-14: resolved) |

**Cancellation, per OS** (SIGINT and process groups don't exist on Windows; SIGKILLing the Gradle
client doesn't stop the daemon's build): POSIX = `setsid` process groups, SIGINT to the group,
SIGKILL after a grace period or on second Ctrl-C. Windows = one Job Object per pipeline with
kill-on-close + `CTRL_BREAK_EVENT` for graceful. Gradle = client-disconnect cancellation (kill
only the client; the daemon cancels asynchronously — documented; `--no-daemon` exposed for CI).
Deliberate survivors are enumerated: the emulator, the Simulator, the adb server, the Gradle
daemon. Every exit path — including cancellation — emits the terminal NDJSON `result` event with
code 130.

### §16.3 Global contract (every subcommand)

```
--project <dir>          # default: nearest ancestor with day.yaml
--format {auto,plain,json}   # json = NDJSON events on stdout, logs on stderr
--no-input               # never prompt; missing required input = error with code
--yes                    # assume-yes for confirmations
--color {auto,always,never}
-v, -q, --log-file <path>
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

- **Service context, not globals:** a `CliContext` bundling `FileSystem`, `ProcessRunner`, `Env`,
  `Clock`, `Terminal`, `Console` traits — injected into commands, faked in tests
  (flutter's Zone-DI, done Rust-idiomatically as a struct of `Arc<dyn Trait>`).
- **Command envelope:** each subcommand is a struct implementing
  `DayCommand { fn validate(&self, cx) -> Result<()>; async fn run(&self, cx) -> Result<Outcome> }`
  with shared pre-flight (project discovery, day.yaml parse, doctor-lite checks relevant to the
  command).
- **Workflows/doctor:** per-target `Workflow` objects (`applicable? functional? missing?`) power
  both `day doctor` and actionable failures ("`android-widget` needs: ANDROID_HOME, JDK 17/21 —
  found JDK 26 (known-broken with AGP; see day doctor)"). This bakes in the workspace's hard-won
  toolchain knowledge (JDK-26/Robolectric-class problems, rustup-vs-homebrew Rust for cross-std,
  cargo-ndk, `aarch64-apple-ios-sim` on Apple Silicon).
- **Plumbing tier:** stable, documented, hidden-from-default-help subcommands invoked by build
  systems: the arg-less `day xcode-backend build` / `day gradle-backend build` entrypoints
  (called by the Xcode Run-Script phase and the Gradle task, reading their parameters from the
  build system's environment — §17.4). Porcelain may change UX; plumbing changes are
  semver-relevant.
- **`day daemon --machine`** (roadmap, post-MVP): long-lived JSON-RPC for IDEs, mirroring
  flutter's daemon; the NDJSON event schema of §16.3 is designed to be reused by it.

### §16.5 Subcommands

The seven mandated commands, plus three additions proposed for approval (**DP-10**): `day doctor`
(indispensable given five toolchains; flutter-proven), `day clean`, and `day config` (the
machine-local settings store doctor's fix-suggestions target).

#### `day create` — interactive project initialization

```
$ day create
✔ Project name · fieldnotes
✔ Bundle / application id · dev.example.fieldnotes
✔ Targets · macos-appkit, ios-uikit, android-widget   (multi-select)
✔ Locales · en, fr                                    (first = default)
✔ Include · dayscript walkthrough, GitHub Actions CI, sample tests

  created fieldnotes/  (24 files)

  next:
    cd fieldnotes
    day doctor          # verify toolchains for your targets
    day launch          # build & run on the default desktop target
```

Non-interactive (CI/agents):
`day create fieldnotes --id dev.example.fieldnotes --targets macos-appkit,ios-uikit,android-widget
--locales en,fr --with ci,script --no-input`. Templates live in `templates/` as `.tmpl` trees with
token substitution (flutter's mechanism; no mustache engine needed for v1). `day create --list`
enumerates templates (`app` default; `piece` scaffolds a Day Piece package with polyglot stubs).

#### `day build`

`day build [-p <target>]… [--profile debug|release]`

Per target: (1) preflight (workflow check), (2) generate conveyance files from `day.yaml` (§17.5),
(3) invoke the build pipeline for the target — **`xcodebuild` for ios only**;
`gradle assembleDebug|Release` (android); **cargo + bundle assembly for ALL cargo-driven desktop
targets including `macos-appkit`** (their "scaffold" is a packaging recipe, not an IDE project;
SwiftPM piece halves build via `swift build` on the generated `DayGeneratedPieces` package, whose
static libs and flags — `-L`/`-l`/`-framework`/Swift-runtime rpath — feed the cargo link through a
generated linker-args file; this path is required for `macos-gtk`/`macos-qt` regardless, DP-20);
MSBuild/cargo for winui. The Xcode/Gradle projects **call back** into the arg-less plumbing
entrypoints (§17.4) for the Rust staticlib/dylib, so builds started from Xcode/Android
Studio/Gradle are first-class and never stale. Multiple `-p` build in parallel with multiplexed
progress and **per-(target, profile) `CARGO_TARGET_DIR`** (`build/day/cargo/<target>/<profile>` —
concurrent cargo invocations otherwise serialize on the build-dir lock and feature-flips thrash
the shared fingerprint cache; disk trade-off documented, covered by `day clean`). Results land in
`build/day/<target>/…` and are printed (and emitted as `result` JSON).

#### `day sign`

`day sign -p macos-appkit [--identity "Developer ID Application: …"] [--notarize] [--no-wait]`

Platform-specific signing of already-built artifacts, designed for CI from the start:

- **Per-format truth**: `.app`/`.dmg` = `codesign` + `notarytool` + `stapler`; `.apk` =
  `apksigner`; `.aab` = Gradle signingConfig (**apksigner cannot sign an .aab**); `.msix` =
  `signtool`; ios = Xcode export signing.
- **Config**: `day.yaml [signing]` with env-var interpolation for secrets —
  `signing.macos { identity, notarize: { key-id, issuer, key-path, wait: 30m } }` (notarytool
  API-key auth, not interactive Apple-ID), `signing.android { keystore, key-alias, store-pass,
  key-pass }`. `Day sign --check` validates presence without logging secrets; `--no-wait` +
  `day sign --notarize-status <id>` support async CI (notarization latency is real).
- **Windows is a provider enum**, decided before the manifest freezes (post-June-2023 CA/B rules
  mean no software `.pfx` exists; MSIX won't install unsigned at all):
  `signing.windows.provider: trusted-signing | azure-key-vault | signtool-cert-store |
  self-signed-dev` with provider-specific config; `day pack -p windows-winui --profile debug`
  auto-generates a self-signed cert with trust-installation instructions for the sideloading dev
  loop (constraints noted in DP-6).

MVP implements: macOS ad-hoc + Developer-ID paths, Android debug/release keystores, iOS Simulator
(no-op) — real-device iOS profiles post-MVP.

#### `day launch`

`day launch [-p <target>]… [--locale <bcp47>] [--script <file>]… [--device <id>] [--profile …]
[--env K=V]…`

Build (+ sign if the destination requires) + install + run + stream logs, per target, in parallel.
`--device <target>=<id>` is repeatable (the bare `--device <id>` form is legal only with exactly
one `-p`, else exit 2):

- `macos-appkit|gtk|qt`: run the binary (bundle for appkit), capture stdout/stderr.
- `ios-uikit`: `simctl install/launch` on the booted (or `--device`) simulator; logs via
  `log stream` with the predicate `subsystem == <app-id> OR processImagePath CONTAINS <name>`.
- `android-widget`: `adb install` / `am start -W`, then a bounded `pidof` retry →
  `logcat --pid`, with an app-id-filtered time-window logcat as the pre-pid fallback (a fast
  startup crash must not lose its logs).

`--locale` injects the locale override (launch env/intent extra). Each `--script` runs in order
via the embedded engine (§14) after connect; with scripts, `day launch` exits when the last script
completes (code 5 on assertion failure) — this is the CI entry point. Without scripts it stays
attached (`q` quits, `r` relaunches, `s` screenshot — no hot reload, honest about it, §16.9).

#### `day pack`

`day pack -p <target> [--profile release]` = build → sign → **installable artifact**. Per target:
`.dmg` (macos-appkit; the stage order is normative, ported from hoppack: sign `.app` → `hdiutil`
→ sign dmg → notarize → staple), **zipped Simulator `.app`** for ios in the MVP
(`Showcase-sim.app.zip`, installable via `simctl` — there is **no** "simulator .ipa";
`xcodebuild -exportArchive` cannot export Simulator builds, so real `.ipa` arrives with the
post-MVP device milestone), `.apk`/`.aab` (android), `.msix` primary / `.msi` via WiX optional
(windows — DP-6), flatpak (linux, post-MVP; hoppack precedent). GTK/Qt bundling on non-native
OSes is post-MVP (DP-7). Emits `result` JSON with artifact paths + checksums.

Distribution-compliance guard rails are part of `pack`, not an afterthought: for qt targets,
store-channel packaging and any static-Qt configuration **hard-fail** unless
`qt.license: commercial` is attested (LGPL-3); qt pack recipes pin dynamic linking +
`macdeployqt`/`windeployqt` + bundled LGPL texts + a source-offer URL (GTK's LGPL-2.1+ is
documented separately in DP-7). A `licenses` stage (schema v1 now, implementation post-MVP)
harvests Rust deps cargo-about-style **plus** the `license:`/`notices:` fields from every
aggregated `piece.yaml` (Maven/SPM obligations are invisible to cargo metadata) into
THIRD-PARTY-NOTICES inside each artifact; lint warns on piece.yaml entries missing license
metadata.

#### `day lint`

Rule framework — **built-in rules only in v1** (a registry in app crates cannot reach into the
prebuilt CLI binary; third-party rules via dylint/WASM plugins are recorded post-MVP). Day ships:
fluent-coverage, bare-user-facing-literals, missing-a11y-labels, ids-in-a11y-labels,
duplicate/missing ids (incl. `id_keyed` prefix uniqueness), day.yaml schema validation, asset
references, the *advisory* signal-read-outside-binding heuristic (§4.1 — the sound check is the
runtime debug diagnostic), scroll-nesting restrictions (§7.6), physical-LTR-styling-with-RTL-locale
(§7.8), reserved-fluent-keys-plain-text (§12), piece-license-metadata. `--strict` (exit 10),
`--format json`, per-rule `allow` in day.yaml. Fast (no full build: operates on sources +
day.yaml + locales + assets).

#### `day script`

`day script <file>… [-p <target> | --attach <host:port>] [--locale …] [--junit out.xml]
[--screenshots-dir …]`

Runs dayscript against a fresh launch (default) or an already-running instance (`--attach`).
`day script --check <file>` validates YAML + step schema + referenced ids against the project's
declared id set without launching.

#### `day doctor` (proposed addition)

Per-target toolchain diagnosis with fixes; `day doctor --json` for agents.

#### `day clean` (proposed addition)

Removes `build/day` (including the per-target cargo dirs) + per-scaffold outputs.

#### `day config` (proposed addition)

The per-machine configuration store the doctor's fix-suggestions write to (`Day config set
android.java-home …`): user-level config in the platform config dir plus an optional gitignored
`day.local.yaml` (the `local.properties` analogue — where the android scaffold reads `sdk.dir`).
Precedence: CLI flag > env > `day.local.yaml` > user config > detection.

### §16.6–16.8 (reserved: command reference details live in Appendix D)

### §16.9 The inner loop (no hot reload — the honest story)

Rust has no VM; Day v1 does not pretend. The inner loop is: incremental `cargo` rebuild of the app
dylib + relaunch + optional `--script` replay to restore UI state (a dayscript that navigates back
to where you were — a genuinely good use of pillar 3). Desktop relaunch is ~seconds (pane measured
this). Roadmap (explicitly out of v1): dylib hot-swapping of the app crate behind a stable
`day-core` boundary (the build-once model actually *helps* here — rebuilt constructors, preserved
signals), recorded as a research item, not promised.

---

## §17 The Conventional Day Project and `day.yaml`

### §17.1 Project layout (`day create` output)

```
fieldnotes/
  day.yaml
  Cargo.toml                 # normal cargo project; `cargo build`/`test`/`clippy` work standalone
  src/
    lib.rs                   # pub fn root() -> impl Piece  (the app)
    main.rs                  # desktop entry: fn main() { day::launch(fieldnotes::root) }
  locales/
    en/app.ftl  fr/app.ftl
  assets/                    # data resources (§18)
  icons/app.svg              # source icon (§18)
  scripts/
    walkthrough.yaml
  tests/
    pieces.rs                # unit tests against day-mock
  day.local.yaml             # OPTIONAL, gitignored: machine-local settings (sdk paths — §16.5 config)
  platform/
    ios/                     # real Xcode project (from template): DayApp.xcodeproj, Runner sources
                             #   (day root hosted in a view controller — §10.5), a Run-Script phase
                             #   `"$DAY_BIN" xcode-backend build` (§17.4), and a checked-in REFERENCE
                             #   to the generated local SwiftPM package DayGeneratedPieces (§15.2)
    android/                 # real Gradle project: settings.gradle.kts (applies the committed
                             #   day.gradle.kts, guards the generated day-pieces.gradle.kts apply with
                             #   an existence check → "run day build once"), app/ (fragment-hosted root)
    macos/                   # bundle recipe (Info.plist template, entitlements) — appkit is
                             #   cargo-driven, no IDE project (§16.5 build, DP-20)
    linux/                   # .desktop file, flatpak manifest (post-MVP)
    windows/                 # winui shim project + packaging manifest (post-MVP)
  .github/workflows/ci.yml   # optional, from `--with ci`
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

### §17.3 `day.yaml`

```yaml
day: 1                              # manifest schema version
scaffold: 1                         # platform-scaffold version stamped by `day create`; `day build`/
                                    #   `doctor` verify it against the CLI's supported range and fail
                                    #   with instructions on mismatch (Flutter needed 30+ migrators for
                                    #   exactly this — the handshake ships in v1; an idempotent
                                    #   `day upgrade` running per-file migrators is committed for M9;
                                    #   "delete platform/ and re-create" is explicitly rejected)
app:
  name: fieldnotes                  # crate/artifact name
  id: dev.example.fieldnotes        # bundle id / application id / app id
  title: app-title                  # Fluent key → localized display name (falls back to name)
  version: 0.3.1                    # CFBundleShortVersionString / versionName / msix version
  build: 42                         # CFBundleVersion / versionCode (int, monotonic)
targets: [macos-appkit, macos-gtk, macos-qt, ios-uikit, android-widget]
localization:
  default: en
  locales: [en, fr]
  dir: locales
assets:
  - assets/                         # recursively packaged (§18)
icons:
  source: icons/app.svg
scripting:
  release: false                    # embed dayscript engine in release builds?
lint:
  allow: [bare-text]                # per-rule opt-outs (discouraged)
ios:
  deployment-target: "15.0"
  capabilities: []                  # entitlements toggles understood by the generator
android:
  min-sdk: 24
  target-sdk: 35                    # edge-to-edge is mandatory at 35 — see §7.7 inset policy
windows:
  app-sdk: "1.6"                    # WinAppSDK runtime pin (§9)
qt:
  license: lgpl-dynamic             # or `commercial` — gates `day pack` static/store configurations (§16.5)
signing:
  macos:
    identity: "${DAY_SIGN_MACOS_IDENTITY}"
    notarize: { key-id: "${DAY_NOTARY_KEY_ID}", issuer: "${DAY_NOTARY_ISSUER}", key-path: "${DAY_NOTARY_KEY}", wait: 30m }
  android: { keystore: "${DAY_ANDROID_KEYSTORE}", key-alias: release, store-pass: "${DAY_KS_PASS}", key-pass: "${DAY_KEY_PASS}" }
  windows: { provider: trusted-signing }   # §16.5 sign — provider enum
dependencies:                       # Day Piece packages needing native aggregation (§15.2)
  # (cargo deps remain in Cargo.toml; this section only exists for overrides/pins of piece.yaml data)
```

Principles: **single source of truth for identity/version/targets**; anything expressible in
`Cargo.toml` stays in `Cargo.toml` (no duplication — `app.name` defaults from the cargo package);
per-platform sections are small and closed-schema (unknown keys = lint error, catching typos).

### §17.4 The build callback (flutter's pattern, exactly — including its hard-won details)

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
- **Freshness and fresh clones**: both callback entrypoints regenerate conveyance from `day.yaml`
  first (content-hashed, §17.5); because Xcode reads xcconfig *before* the phase runs, drift is
  detected and that build fails with "metadata changed — build again". `settings.gradle.kts`
  guards the generated `day-pieces.gradle.kts` apply with an existence check throwing "run `Day
  build` once". Committed-vs-generated is explicit: `day.gradle.kts` and a bootstrap xcconfig stub
  are **create-time committed** files; only value-bearing generated files are gitignored; the
  pbxproj references generated `.lproj` outputs via a folder reference so it never names
  gitignored files.
- Recursion guard: the plumbing entrypoints never re-enter the native build; `DAY_BUILD_PARENT`
  marks provenance for diagnostics.

### §17.5 Metadata conveyance (day.yaml → each build system)

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

**`cargo build` works standalone — really.** Metadata generation is implemented once in the small
`day-meta` library used by **both** the CLI and the `day` crate's build script: when
`DAY_META_PATH` is unset, the build script walks up from `CARGO_MANIFEST_DIR` to `day.yaml` and
synthesizes identical metadata (id/version/asset scan/locale list); dev-profile `Asset::named`
resolves against the project directory (`DAY_ASSET_ROOT` override); absent `day.yaml`
(library/day-mock consumers) → empty metadata with a compile-time note. This also fixes milestone
ordering — the M2 showcase runs before the CLI exists at M4.

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

### §18.2 Icons and images

- **Vector icons, first-class — with an honest pipeline:** android.graphics and Qt Widgets cannot
  render SVG at runtime, so `day build` **pre-renders** bundled SVG icon sets to per-density
  PNGs via `resvg`/`usvg` (plus `.icns`/`.ico` encoders); runtime uses the plain toolkit image
  decode path. Native symbol systems (SF Symbols on apple targets, themed icons on gtk) remain
  available as `per_toolkit` overrides — Skip's learned lesson (prefer bundled symbol assets over
  `systemName` for cross-platform consistency) is the default stance.
- `icons.source` (SVG) → `day build` renders the full per-platform icon matrix
  (AppIcon.appiconset, mipmap/adaptive icons, `.icns`, `.ico`, favicon) — Skip's `skip icon` is
  the direct precedent including gradient-background/inset conveniences.
- **Theming**: §6.3 semantic tokens; dark/light tracked natively per toolkit and surfaced as a
  signal.

---

## §19 Repository layout, examples, and docs site

```
day/                                # THIS repository
  Cargo.toml                        # workspace; default-members = host-portable crates
  DESIGN.md                         # this document
  crates/                           # day, day-core, day-reactive, day-geometry, day-spec,
                                    # day-pieces, day-canvas, day-fluent, day-script,
                                    # day-script-proto, day-mock, day-cli
  toolkits/                         # day-appkit, day-uikit, day-gtk, day-qt(+sys),
                                    # day-android, day-winui(+sys), day-web, day-arkui
  pieces/                           # example/blessed external Day Pieces (separate crates,
                                    #   structured EXACTLY like third-party ones would be):
    day-piece-combobox/             #   UI piece, tier-1 Rust renderers — IN the MVP acceptance (DP-21)
    day-piece-battery/              #   platform service, tier-2 polyglot (M9 — first dayffi proof; DP-21)
    day-piece-webview/              #   complex piece (post-MVP)
    day-piece-lottie/               #   third-party native lib bridge (post-MVP)
  apps/
    counter/                        # minimal app, all 6 primary buildable targets
    showcase/                       # THE demo: every implemented piece; MVP acceptance app
    fieldnotes/                     # mobile-only sample (ios-uikit, android-widget; uses list when it lands)
    deskclock/                      # desktop-only sample (appkit/gtk/qt; canvas-heavy, menus later)
  templates/                        # `day create` scaffolds (.tmpl trees)
  site/                             # Astro Starlight docs site (skip.dev precedent)
  docs/                             # design docs, per-toolkit conventions, dayffi spec
  scripts/                          # repo dev scripts (setup-winui.ps1 analogue, emulator helpers)
  .github/workflows/
```

Apps and pieces live in the workspace but depend on Day **by path exactly as external users would
by version** — the pieces/ crates are the continuous proof of the zero-edit extensibility claim
(pane's `pane-combobox` discipline). The site and individual pieces are expected to migrate to
separate repositories eventually; nothing may depend on their in-repo location (site pulls docs via
a sync script, pieces use only public APIs).

Docs site: Astro Starlight (as skip.dev), sections: Guide (tutorial: counter → showcase), Concepts
(pieces/reactivity/layout/styling), Platforms (per-target setup + capability matrices), Pieces
(catalog, incl. external), CLI reference (generated from clap definitions — single source of
truth), dayffi spec, dayscript reference.

---

## §20 Continuous integration

Workflows (patterns lifted from pane's per-OS matrix and hop's screenshot pipeline):

1. **ci.yml** — per-OS jobs:
   - linux: host-portable core tests (`cargo test` over default-members incl. day-mock e2e), gtk +
     qt backend builds, **headless e2e**: `xvfb-run day launch -p linux-gtk --script
     walkthrough` (+ `linux-qt` via the offscreen platform) with screenshot upload — cheap
     (in-process `snapshot_window` + `GSK_RENDERER=cairo`), and it converts the "MVP-adjacent
     (CI)" tier label from aspiration to evidence; android cross-build (cargo-ndk) + gradle
     assemble **with `--configuration-cache`**; lint self-check.
   - macos: core tests, appkit + uikit builds, **launch showcase on macos-appkit + gtk + qt and on
     the iOS Simulator, run `scripts/walkthrough.yaml` per locale (en, fr, en-XA; ar-XB leg
     post-M6), upload content-validated screenshots** (hop's recipe: ≥N distinct colors check,
     retries, `GITHUB_STEP_SUMMARY` gallery), `a11y_audit` on the apple targets (§14.2), and a
     **release+LTO ios-uikit leg of showcase + day-piece-combobox** asserting the
     externally-registered piece rendered (§8.2).
   - linux-android-e2e: emulator via KVM runner (`reactivecircus/android-emulator-runner`),
     launch + walkthrough + screenshots.
   - windows: core tests, winui shim build (`setup-winui.ps1` staging, pane's exact pipeline).
   - msrv: build at the declared MSRV (§20.5).
2. **pack.yml** — `day pack` per releasable target; artifacts: `.dmg`, zipped Simulator `.app`
   (no "sim .ipa" — §16.5), `.apk`; checksums; uploaded per run, promoted on tags. **Split
   signing tiers**: PR/branch runs use ad-hoc signing + `--no-notarize` (fork PRs have no
   secrets); tag/protected runs use real identities + notarize + staple, degrading loudly
   ("unsigned artifact" in the result JSON) when secrets are unresolvable.
3. **site.yml** — build Astro site (`npm run build`, link check) → GitHub Pages.
4. **piece-ci.yml** — builds `pieces/*` against Day as external consumers (per-toolkit matrix),
   plus: the dayffi ASAN/leak ownership-round-trip suite, the v1-pinned-piece ABI cell (§15.3),
   the extra-combo battery-on-`macos-gtk` selector case (§15.2, once battery lands in M9), and
   `cargo package --list` packaging checks.

CI knowledge already banked in this workspace and encoded into the workflows from day one: JDK
pinning (17/21, never 26), rustup toolchain for cross-std, `GSK_RENDERER=cairo` for headless GTK,
`--locked` everywhere, emulator boot polling.

### §20.5 Toolchain and dependency governance

Declared **MSRV** (latest-stable-minus-2 at each release) tested as a CI job; `rust-toolchain.toml`
pins repo development; edition 2024; **cargo-deny** in ci.yml (advisories, license allowlist,
duplicate bans); documented per-backend floors where FFI crates force higher MSRV (gtk4-rs tracks
GNOME cycles) surfaced by `day doctor`; `Cargo.lock` committed for day-cli + example apps, ranges
for libraries.

---

## §21 MVP definition and milestone plan

### §21.1 MVP acceptance (verbatim goal)

On the current macOS host: `day launch -p macos-appkit -p macos-gtk -p macos-qt -p ios-uikit -p
android-widget` builds and launches the **showcase** app on all five targets; `day launch -p
ios-uikit --locale fr-FR --script scripts/walkthrough.yaml` runs the localized walkthrough,
passes its assertions, and produces screenshots; `day create` scaffolds a working project;
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
| M4 | CLI v0: `create`/`build`/`launch` (desktop targets), day.yaml + day-meta, templates, doctor-lite, JSON events (hello/log/result), cancellation | `day create t && cd t && day launch` works |
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

Each has a recommendation. **DP-16 (row contract) and DP-23 (navigation) are resolved**
(owner-ratified 2026-07-01; resolutions folded into §5.4/§10) — the M2 `day-spec` freeze is
unblocked. DP-24's crates.io reservation is **explicitly deferred by owner directive** (no
registry action; the naming decision stays open). The remaining DPs do not block M0–M2.

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
| DP-10 | extra subcommands `doctor`/`clean`/`config` | approve / reject | approve (doctor is load-bearing for 5-toolchain UX; config is where doctor's fixes land) |
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

# Appendix A — The showcase app, end to end

### A.1 `apps/showcase/src/lib.rs` (complete MVP surface)

```rust
use day::prelude::*;
use day_piece_combobox::combo_box;      // external Day Piece — zero edits to day (§8.2, §15)

pub fn root() -> impl Piece {
    let count = Signal::new(0);
    let name = Signal::new(String::new());
    let volume = Signal::new(40.0f64);
    let subscribed = Signal::new(false);
    let flavors = Signal::new(vec!["vanilla".into(), "chocolate".into(), "pistachio".into()]);
    let flavor = Signal::new(Some(0usize));

    scroll(
        column((
            row((
                image(ImageSource::asset("day-logo"))
                    .frame(32.0, 32.0)
                    .a11y(|a| a.decorative()),
                label(tr("app-title"))
                    .style(|s| s.font(Font::title()))
                    .id("controls-title")
                    .a11y(|a| a.role(Role::Heading(1))),
                spacer(),
            )).spacing(8.0).align(VAlign::Center),

            // — state: counter —
            row((
                button(tr("decrement")).action(move || count.update(|c| *c -= 1)).id("decrement-button"),
                label(tr("counter-value").arg("count", count)).id("counter-label"),
                button(tr("increment")).action(move || count.update(|c| *c += 1)).id("increment-button"),
            )).spacing(8.0).align(VAlign::Center),

            divider(),

            // — text input + conditional —
            text_field(name).placeholder(tr("name-placeholder")).id("name-field"),
            when(move || !name.with(String::is_empty),
                 move || label(tr("greeting").arg("name", name)).id("greeting-label")),

            // — slider with live readout —
            row((
                label(tr("volume-label")),
                slider(volume).range(0.0..=100.0).id("volume-slider")
                    .a11y(|a| a.label(tr("volume-label"))),
                label(move || format!("{:.0}", volume.get())).id("volume-value"),
            )).spacing(8.0),

            toggle(subscribed).id("subscribe-toggle")
                .a11y(|a| a.label(tr("subscribe-label"))),

            // — an EXTERNAL Day Piece, registered like any built-in (MVP acceptance, DP-21) —
            row((
                label(tr("flavor-label")),
                combo_box(flavors, flavor).id("flavor-combo"),
            )).spacing(8.0),

            divider(),

            // — canvas gauge bound to the slider (lands in M8a; walkthrough gains its
            //   screenshot step then — §21.2) —
            gauge(volume),

            // — keyed collection —
            history(count),
        ))
        .spacing(12.0)
        .padding(per_toolkit(16.0).qt(12.0).gtk(12.0)),
    )
}

fn gauge(value: Signal<f64>) -> impl Piece { /* §11 verbatim */ }

// derive-state idiom: watch() (tracked source, UNTRACKED callback — §4.2), monotonic keys
// (never e.len(): removal would recycle keys and corrupt the each diff)
fn history(count: Signal<i32>) -> impl Piece {
    let entries = Signal::new(Vec::<(u64, i32)>::new());
    let next_id = Signal::new(0u64);
    watch(move || count.get(), move |new, _old| {
        let id = next_id.get_untracked();
        next_id.set(id + 1);
        entries.update(|e| e.push((id, *new)));
    });
    column((
        label(tr("history-title")).style(|s| s.font(Font::headline())),
        each(move || entries.get(), |e| e.0,
             move |(_, v)| label(tr("history-entry").arg("value", v))),
    )).spacing(4.0)
}
```

### A.2 Locales

`locales/en/app.ftl` — §12.1 plus `history-title = History`, `history-entry = count became { $value }`,
`subscribe-label = Subscribe to updates`, `flavor-label = Flavor`, `decrement = −`, `increment = +`.
`locales/fr/app.ftl` — full French mirror (`greeting = Bonjour, { $name } !`, …).

### A.3 `scripts/walkthrough.yaml` — §14.1 verbatim; the gauge screenshot step joins in M8a (§21.2).

### A.4 Run it

```
$ day launch -p macos-appkit -p macos-gtk -p macos-qt -p ios-uikit -p android-widget
$ day launch -p ios-uikit --locale fr-FR --script scripts/walkthrough.yaml
$ day launch -p android-widget --locale en-XA --script scripts/walkthrough.yaml   # pseudolocale layout stress
```

---

# Appendix B — Worked Day Piece examples

### B.1 ComboBox (tier 1 — Rust renderers, the pane-combobox pattern)

```rust
// day-piece-combobox/src/lib.rs
pub fn combo_box(items: Signal<Vec<String>>, selected: Signal<Option<usize>>) -> impl Piece {
    ForeignPiece::new("acme.combobox")
        .prop_with("items", move || items.get())
        .prop_with("selected", move || selected.get())
        .on_event("changed", move |v: i64| selected.set(Some(v as usize)))
}
// day-piece-combobox/src/appkit.rs (cfg(feature = "appkit")): NSComboBox renderer registered
// into day-appkit's slice; gtk.rs → GtkDropDown; qt.rs → QComboBox (via a 40-line C shim);
// uikit.rs → UIButton+UIMenu; android.rs → Spinner via DayBridge factory.
```

App usage: add the crate with the matching toolkit features; `use day_piece_combobox as _;` anchors
registration. Zero edits to day.

### B.2 Battery (tier 2 — a *service*, polyglot, no UI)

```rust
// day-piece-battery/src/lib.rs
pub fn battery() -> BatteryHandle;             // BatteryHandle { pub level: Signal<f32>, pub charging: Signal<bool> }
```
`platform/apple/…/DayBattery.swift` (SwiftPM): conforms to `DayServiceVTable` via the `DayFFI`
helper — `call("read") -> {level, charging}` + `subscribe` pushing on
`UIDevice.batteryLevelDidChangeNotification` (delivered on arbitrary queues — exercising §15.3's
any-thread `day_host_emit` contract). `platform/android/…` (Gradle lib): `BatteryManager` +
broadcast receiver → `day_host_emit`. Other targets: Rust tier-1 impls keyed by **OS selector**,
not toolkit — `windows` = `GetSystemPowerStatus` (used by windows-winui *and* windows-qt),
`linux` = upower DBus (linux-gtk and linux-qt), and on `macos-gtk`/`macos-qt` the **apple** half
applies (IOKit) — the §15.2 selector-precedence design in action, exercised by piece-ci.
`piece.yaml` wires the SwiftPM/Gradle halves into the app scaffolds via `day build` aggregation
(§15.2). Battery is the first tier-2 piece (M9; DP-21 stretch pulls it to M8).

### B.3 WebView (tier 2 — complex: commands + events)

`web_view(url).on_navigate(f)` returning `(impl Piece, WebViewController)`;
`controller.evaluate_js("…").await -> DayValue` via **`command_async` + `day_host_complete`**
(§15.3 — WKWebView/WebView2/android.webkit are completion-handler-only; a synchronous `command`
would deadlock the main thread, which is why async commands are v1 ABI); native halves: WKWebView
(Swift), android.webkit.WebView (Kotlin), WebKitGTK, QWebEngineView, WebView2 — each ~200 lines
against dayffi. skip-web is the API-shape precedent.

### B.4 Lottie (tier 2 — bridging famous native libraries)

`lottie(Asset::named("hello.json")).looping(true).playing(sig)`; `platform/apple` wraps lottie-ios
via SPM; `platform/android` wraps lottie-android via Maven — the package's `piece.yaml` carries
those third-party coordinates (and their notices metadata for the pack notices stage);
`day build` threads them into the host projects. Desktop: **ThorVG / dotlottie-rs** (maintained,
with a Rust-native option — not rlottie, which is unmaintained with 2025 CVEs), or the canvas
fallback renderer reported as `capability(Cap::Lottie) == Support::Emulated`.

### B.5 RichText (tier 2 — deep native control)

`rich_text(doc: RichDoc)` over NSTextView/UITextView/EditText/GtkTextView/QTextEdit; a Rust-side
document model (spans + attributes) diffed into native attributed strings; selection/undo stay
native; `command("apply_format", …)`; the worked example that proves `measure`+`command`+event
density at scale. Post-MVP, design-complete here so the ABI is sized for it.

---

# Appendix C — dayscript reference (v1)

Locators: `id:` primary (incl. keyed `id: "todo-remove:42"`), `text:`/`key:` secondary; `index:` /
`all:` qualifiers define semantics over non-unique matches (`assert_count` counts all matches).
All locator steps take a `timeout: <secs>` override; every *acting* step first checks
**actionability** (§14.2: enabled ∧ visible ∧ within ancestor scroll viewports ∧ topmost at
center) and auto-scrolls the target into view. `repeat: <n>` is a modifier available on any
acting step (as used in §14.1); `repeat: {times, steps}` is the block form.

**Step tiers** (three, explicit — `day script --check` reports tier and support):

1. **Engine steps** (synthesized Day events; uniform on every toolkit): `tap`, `long_press`
   (targets with an `.on_long_press` handler), `input`, `clear`, `set_value {value}` (typed per
   piece kind: toggle=boolean, slider=number, text_field=string — YAML 1.1 `on/off` coercion is
   exactly why values are schema-typed), `toggle`, `key {chord}` (scoped to `.on_key` nodes),
   `scroll_to`, `wait_for`, `wait_idle`, `pause {secs}`, `assert_visible` (realized ∧ resolved
   frame intersects window bounds and every ancestor scroll viewport ∧ no hidden ancestor),
   `assert_not_visible`, `assert_text {text|key+args}` (FSI/PDI-normalized, §12.2),
   `assert_value`, `assert_enabled`, `assert_count {n}`, `screenshot {name}`, `a11y_audit`
   (§14.2), `repeat`, `run_flow {file}`.
2. **Runner steps** (executed by the CLI, not the embedded engine — an app cannot terminate its
   own process and report the result): `launch {clear_state}`, `terminate`.
3. **Reserved native-injection steps** (post-v1; structured "unsupported step" errors with
   capability metadata until then): `swipe {dir}`, `back`, `native_tap`.

Env interpolation `${VAR}`; per-script `config: {timeout, screenshot_dir}`. JSON Schema published
at `docs/dayscript.schema.json` (editor completion; `day script --check` validates steps, types,
and referenced ids against the project's declared id set).

---

# Appendix D — `day` CLI transcripts

```
$ day doctor
day 0.1.0 · project fieldnotes · targets: macos-appkit, ios-uikit, android-widget
✓ rust        1.89 (rustup) + targets aarch64-apple-ios-sim, aarch64-linux-android
✓ xcode       16.3 · simulators: iPhone 16 (booted)
✗ android     JDK 26 found — AGP requires ≤21    → brew install openjdk@21; day config set android.java-home …
✓ gtk4        4.16 (homebrew) · pkg-config OK
! qt6         not found — target macos-qt disabled  → brew install qt@6

$ day build -p ios-uikit -p android-widget --format json | tail -1
{"event":"result","command":"build","ok":true,"targets":[
  {"target":"ios-uikit","ok":true,"code":0,"artifacts":[{"path":"build/day/ios-uikit/Showcase.app"}],"seconds":41.7},
  {"target":"android-widget","ok":true,"code":0,"artifacts":[{"path":"build/day/android-widget/showcase-debug.apk"}],"seconds":58.2}]}

$ day launch -p macos-appkit -p ios-uikit -p android-widget
⠸ macos-appkit    cargo build … 3.2s ✓ · launched (pid 48112)
⠸ ios-uikit       xcodebuild … 41s ✓ · installed → launched on iPhone 16
⠼ android-widget  gradle :app:assembleDebug … 58s ✓ · adb install → launched on emulator-5554
   [appkit] fieldnotes: started (locale en-US)
   [ios]    fieldnotes: started (locale en-US)
   press q quit · r relaunch · s screenshot

$ day launch -p ios-uikit --locale fr-FR --script scripts/walkthrough.yaml --junit build/junit.xml
… ✓ 13/13 steps · 3 screenshots → build/day/screenshots/ios-uikit/fr-FR/ · junit → build/junit.xml

$ day lint --strict
warning day::lint::bare-text   src/lib.rs:41  user-facing literal "Save" — use tr("…")
error   day::lint::missing-translation  locales/fr/app.ftl  missing: history-title, subscribe-label
2 findings (1 error) — exit 10

$ day pack -p macos-appkit --profile release
✓ build → sign (Developer ID …) → notarize (2m10s) → build/day/dist/Fieldnotes-0.3.1.dmg (sha256 …)
```

---

# Appendix E — Implementation notes for the builder

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
