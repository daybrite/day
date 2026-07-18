---
title: Why Day (and why not)
description: An honest comparison with web-view shells, custom renderers, and per-platform native — including the cases where Day is the wrong choice.
order: 2
section: Start here
---

Choosing a cross-platform stack means choosing what to give up. This page lays out what Day
trades away and what it gets back, and names the situations where you should pick something
else. We'd rather lose you on this page than after three months of investment.

## The landscape

Four established ways to ship one app on many platforms:

| Approach | Examples | Keeps | Gives up |
|---|---|---|---|
| Web view shell | Electron, Tauri | Web skills, one DOM UI | Native behavior and feel; memory; platform integration depth |
| Custom renderer | Flutter, egui, Slint | Pixel-identical UI, hot reload (Flutter) | Native look/behavior; must reimplement text, scrolling, a11y |
| Shared logic, native UI | Kotlin Multiplatform, Skip | Fully native UI | The single UI codebase — you still write each UI |
| Native widgets, one codebase | **Day**, React Native* | Native widgets and one UI codebase | Pixel-identical branding; some framework-mediated control |

\* React Native shares the "real native widgets" premise for mobile; it differs in language (JS +
a bridge), in update model (re-render + reconcile), and in desktop coverage.

Day's position: write the UI once in Rust; realize it with the platform's own widgets; keep the
framework's surface area small enough that the platform, not the framework, defines how your app
feels.

## What you get

**Native fidelity without per-platform UI code.** Text rendering, input methods, spellcheck,
scrolling physics, selection, drag, focus behavior, dark-mode chrome, screen readers — these come
from the platform's widgets, which means they're correct in ways a reimplementation struggles to
match, and they improve with OS updates you never ship. The practical consequence: a Day app
doesn't have a "slightly off" feel to platform-native users, because the parts users touch are
the platform's own.

**A runtime profile you can reason about.** Day builds the widget tree once and binds state to
native attributes. A state change re-runs the closures that read that state — typically ending in
one native setter call — with no re-render, no virtual tree, and no diffing on the hot path
([how this works](/docs/reactivity)). The compiler monomorphizes your app against exactly one
toolkit backend per binary, so there's no runtime abstraction layer either. Binaries are ordinary
Rust binaries linking system libraries — no bundled engine, no bundled browser.

**One language for everything.** UI, state, logic, tests, and build tooling are Rust. There's no
FFI seam between your view layer and your data layer, no separate template language, and the
borrow checker applies to your UI code the same way it applies to everything else. Whether that's
a benefit depends entirely on your team — see the costs below.

**Four pillars designed in, not bolted on.** These compose: localized strings are reactive, so
locale switches update a running app; accessibility identifiers double as automation ids; one
dayscript walkthrough, run per-locale, is simultaneously an end-to-end test, an accessibility
audit, and a screenshot generator. This composition is the part of Day that's hard to retrofit
onto other stacks.

1. **Localizable** — Mozilla Fluent throughout, with ICU-correct plurals, number and date
   formatting, and collation-aware sorting — locale data thinned to the locales you ship.
   The current locale is a signal. ([guide](/docs/localization))
2. **Accessible** — real native widgets give a real native accessibility tree as the baseline;
   Day adds uniform annotations and stable identifiers, and CI can diff the native tree against
   your declarations. ([guide](/docs/accessibility))
3. **Scriptable** — a YAML automation language drives the running app over a socket, identically
   on every platform. ([guide](/docs/dayscript))
4. **Extensible** — new widgets plug in as ordinary crates, from pure composition down to
   per-toolkit native code, without forking Day. ([how](/docs/extending))

**Tooling built for CI and agents as much as humans.** `day doctor` diagnoses five toolchains
with fix-it text; `day launch` runs any subset of eleven targets; `day pack`
[produces signed installable artifacts](/docs/packaging); everything speaks JSON when asked.

## What you give up

**Hot reload.** Rust compiles ahead of time. The edit loop is an incremental compile plus
relaunch — seconds on desktop, longer for mobile targets — with dayscript replay to restore UI
state. Flutter's sub-second stateful hot reload is genuinely better for exploratory UI work, and
nothing in Day currently matches it. (Hot-swapping the app dylib is a researched possibility, not
a promise.)

**Pixel-level brand control.** Your app looks like a Mac app on macOS and a Material app on
Android. If the design brief is a bespoke design system rendered identically everywhere — custom
controls, custom motion, brand color on every surface — Day's native-widget premise works against
you, and a renderer (Flutter, or Rust-native options like Slint or egui) will fight you less.
[Styling](/docs/styling) is explicit about where the line sits.

**Ecosystem maturity.** Flutter has years of production hardening, thousands of packages, and an
enormous community. Day is young: the widget vocabulary is deliberately small, some designed
features aren't implemented yet (semantic color tokens, an animation scheduler, multi-window,
form validation — [Platform support](/docs/platforms) keeps the honest list), and you will hit
edges. The mitigations are real but partial: the architecture descends from several generations
of working systems, a nontrivial Matrix chat client runs on five targets, and every target is
exercised in CI with screenshot validation on every push. Judge the risk for your project
accordingly.

**Rust, with a single-threaded UI.** If your team doesn't know Rust, the learning curve is the
project's learning curve. UI state is main-thread-only by construction (`Signal` isn't `Send`);
background work returns through explicit `Setter`/`on_main` doors. The compiler enforcing this
prevents a whole bug class, and it also means there's no casual shared-state shortcut when you
want one.

**Platform variance is still yours to test.** One codebase does not mean one behavior. Native
widgets differ — focus order, dialog conventions, text metrics — and while dayscript makes
cross-platform testing cheap, it doesn't make it unnecessary. Day also can't script what it
doesn't own: native keyboards, IME composition, and OS dialogs still need occasional manual
checks per platform.

**Framework-mediated platform access.** When you need a platform API Day doesn't surface, you
write it yourself — the [parts](/docs/parts) pattern makes this a normal, contained thing to do
(a few `cfg`-gated functions per platform), but it's work a single-platform app wouldn't have.

## Choosing

Pick **Electron or Tauri** when your product *is* a web UI, your team is a web team, and desktop
integration depth matters less than shipping this quarter. Pick **Flutter** when design-system
uniformity across platforms is a requirement, or when hot-reload-driven iteration speed dominates
everything else. Pick **per-platform native** when you're on one platform, or when each platform
app has its own team and roadmap. Pick **Day** when you want one Rust codebase, you want the
result to feel native on each platform because it is, and you can live with a young framework's
edges in exchange for a runtime model with very little between your code and the platform.

---

Convinced enough to try it? [Getting started](/docs/getting-started) takes about ten minutes.
