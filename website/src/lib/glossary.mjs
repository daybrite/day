// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// The glossary: one entry per term, read by two consumers. The Glossary page
// (src/content/docs/glossary.mdx) lists every entry; the rehype plugin
// (plugins/glossary-links.mjs) attaches an entry's definition to any docs link into
// `/docs/glossary#<id>`, which DocsLayout shows as a popover.
//
// A definition is one to three plain sentences, in the sense the docs give the word (where the
// word has an everyday meaning too, the entry says which applies). Backticks mark code
// (`Signal<T>`) and are the only markup: the popover builds its text as DOM nodes, never as HTML.
// `see` names the page where the concept is introduced, which the popover offers as "Defined in".

/**
 * @typedef {object} Term
 * @property {string} id         The anchor on the glossary page and the `#id` docs link to.
 * @property {string} term       How the word is written, capitalized as in running prose.
 * @property {string} definition One to three sentences; backticks for code.
 * @property {{ title: string, href: string }} see The page that introduces the concept.
 * @property {string[]} [also]   Other spellings the glossary page lists beside the term.
 */

/** @type {Term[]} */
export const glossary = [
  {
    id: 'piece',
    term: 'Piece',
    definition:
      'A description of one part of the UI, built once into a native widget. `label`, `button`, and `column` are pieces; so is any function of yours that returns `impl Piece`. The word also names a UI extension crate (`day-piece-*`).',
    see: { title: 'Pieces', href: '/docs/pieces' },
  },
  {
    id: 'part',
    term: 'Part',
    definition:
      'A headless platform capability: a set of functions with no UI whose implementation differs per operating system, shipped as an ordinary crate (`day-part-*`). Battery, clipboard, and location are parts.',
    see: { title: 'Device capabilities (parts)', href: '/docs/parts' },
  },
  {
    id: 'tweak',
    term: 'Tweak',
    definition:
      'A supported way to reach the real native widget behind a built-in piece and configure it, while Day keeps owning layout and lifecycle. A piece with a tweak applied keeps the same widget, with a little more configured.',
    see: { title: 'Tweaks', href: '/docs/tweaks' },
  },
  {
    id: 'page',
    term: 'Page',
    definition:
      "One screen's worth of UI: a function that returns a piece, registered under a route and shown by a `selector` or a `stack`. Pages live under `src/pages/` by convention. A window can show more than one at a time: a phone shows a single page, while the same app on a tablet or desktop may lay out a sidebar, a list, a detail page, and an inspector across several navigation levels.",
    see: { title: 'Navigation', href: '/docs/navigation' },
  },
  {
    id: 'toolkit',
    term: 'Toolkit',
    definition:
      "A native widget system: UIKit, Android's Material Components, AppKit, GTK 4, Qt 6 Widgets, Windows XAML, ArkUI, or the browser DOM. Day builds each piece with the widgets of one toolkit per binary.",
    see: { title: 'Overview', href: '/docs/overview#the-targets' },
  },
  {
    id: 'platform',
    term: 'Platform',
    definition:
      'An operating system a Day app is built for and deployed to: macOS, iOS, Android, Linux, Windows, HarmonyOS, or the web. A platform can host more than one toolkit; the pair is a target.',
    see: { title: 'Platform support', href: '/docs/platforms' },
  },
  {
    id: 'target',
    term: 'Target',
    definition:
      "An `(OS, toolkit)` pair, written `macos-appkit`, `ios-uikit`, `android-mdc`. One binary is compiled per target, containing only that toolkit's backend.",
    see: { title: 'Overview', href: '/docs/overview#the-targets' },
  },
  {
    id: 'backend',
    term: 'Backend',
    definition:
      "The Rust crate that implements Day's toolkit interface for one toolkit, such as `day-appkit` or `day-gtk`. A Day binary links exactly one, chosen by a Cargo feature when it is built.",
    see: { title: 'Architecture', href: '/docs/architecture' },
  },
  {
    id: 'native',
    term: 'Native',
    definition:
      "Made of the platform's own widgets. A Day `button()` is an `NSButton` on macOS and a `MaterialButton` on Android, so text input, scrolling, dark mode, and screen readers are the platform's too.",
    see: { title: 'Overview', href: '/docs/overview' },
  },
  {
    id: 'declarative',
    term: 'Declarative',
    definition:
      'Describing what the UI is rather than the steps that build it. A Day app is a declarative tree of pieces; Day turns the tree into widgets and keeps them in sync with your state.',
    see: { title: 'Overview', href: '/docs/overview' },
  },
  {
    id: 'imperative',
    term: 'Imperative',
    definition:
      'Step-by-step instructions to a widget, as opposed to a description of the result. Day keeps imperative work behind the reactive system, in the patches that update a native widget.',
    see: { title: 'Tweaks', href: '/docs/tweaks#reaching-a-widget-later' },
  },
  {
    id: 'reactive',
    term: 'Reactive',
    definition:
      "Updating by itself when something it reads changes. Day's reactive system is a fine-grained signal graph: state lives in signals, and a closure that reads one re-runs when it changes, ending in one native widget update.",
    see: { title: 'Reactivity', href: '/docs/reactivity' },
  },
  {
    id: 'signal',
    term: 'Signal',
    definition:
      '`Signal<T>`, a reactive cell holding one value. The handle is `Copy`, so you move it into as many closures as you like; reading it inside a reactive closure subscribes that closure to changes.',
    see: { title: 'Reactivity', href: '/docs/reactivity#signals' },
  },
  {
    id: 'binding',
    term: 'Binding',
    definition:
      'The link between a reactive closure and one native widget attribute: the closure re-runs when its reads change, and its result is applied to the widget. `label(move || …)` creates one. Also the trait behind two-way controls such as `slider(signal)`.',
    see: { title: 'Reactivity', href: '/docs/reactivity#effects-and-bindings' },
  },
  {
    id: 'decorator',
    term: 'Decorator',
    definition:
      "A modifier such as `.padding()`, `.frame()`, or `.id()` that wraps a piece and returns `Decorated<P>`, keeping the piece's own type. Decorators live only in Day's tree and create no native widget.",
    see: { title: 'Pieces', href: '/docs/pieces#composing-trees' },
    also: ['modifier'],
  },
  {
    id: 'selector',
    term: 'Selector',
    definition:
      "The navigation piece for one of several top-level sections, bound to a `Signal<String>` holding the active item's key. It becomes a sidebar on the desktop and tabs where that is the platform's idiom.",
    see: { title: 'Navigation', href: '/docs/navigation#sections-selector' },
  },
  {
    id: 'sidebar',
    term: 'Sidebar',
    definition:
      "A list of sections beside the content. Day builds the selected page on demand and disposes it when the selection changes, so a sidebar's page state lives in your signals.",
    see: { title: 'Navigation', href: '/docs/navigation#sections-selector' },
  },
  {
    id: 'split-view',
    term: 'Split view',
    definition:
      'A window divided into a section list and a detail pane. It is how the platform draws a sidebar selector when there is room; on narrow screens the same selector pushes pages instead.',
    see: { title: 'API tour', href: '/docs/api-tour#navigation' },
  },
  {
    id: 'route',
    term: 'Route',
    definition:
      'A typed navigation destination declared with the `routes!` macro: what `selector`, `stack`, deep links, and dayscript `navigate` speak. Written as `segments/joined/by/slashes`; a single key is relative and a multi-segment path is absolute.',
    see: { title: 'Navigation', href: '/docs/navigation#routes-and-deep-links' },
  },
  {
    id: 'size-class',
    term: 'Size class',
    definition:
      "A bucket for a window's width or height in points, using Android's window size classes on every platform: `Compact`, `Medium`, `Expanded`, `Large`, and `ExtraLarge` for width. The class can change while the app runs, when a phone rotates or a desktop, tablet, or browser window is resized, and navigation re-resolves its shape each time, so one selector fits a phone, a tablet, and a desktop window.",
    see: { title: 'Size classes', href: '/docs/internal/size-classes' },
  },
  {
    id: 'capability',
    term: 'Capability',
    definition:
      'What a backend can do, asked at run time with `capability(Cap::…)`: `Native`, `Emulated`, or `Unsupported`. An app checks before offering a toolbar or a menu bar. A device capability, such as the battery or location, is a different thing: that is a part.',
    see: { title: 'Menus, toolbars, and windows', href: '/docs/guide-desktop' },
    also: ['Cap'],
  },
  {
    id: 'placeholder',
    term: 'Placeholder',
    definition:
      'What you see where a piece has no native implementation on this toolkit: its kind, in angle brackets. Coverage grows toolkit by toolkit, and the placeholder marks what is left.',
    see: { title: 'The extension model', href: '/docs/extending' },
  },
  {
    id: 'kind',
    term: 'Kind',
    definition:
      'The stable string that names a piece to every backend, such as `day.label` or `day.piece.lottie`. A backend dispatches a kind to its renderer; a kind it does not know draws a placeholder.',
    see: { title: 'How rendering works', href: '/docs/rendering#the-realized-tree' },
  },
  {
    id: 'dayscript',
    term: 'dayscript',
    definition:
      "Day's automation language: a YAML file of steps that drives and asserts a running app. The engine is compiled into your app and executes steps as real Day events, so one script runs on every target and waits are deterministic.",
    see: { title: 'Testing with dayscript', href: '/docs/dayscript' },
  },
  {
    id: 'walkthrough',
    term: 'Walkthrough',
    definition:
      "The main script in a project's `dayscript/`, conventionally `walkthrough.yaml`. CI runs it on every target, and the gallery shows its captures.",
    see: { title: 'Testing with dayscript', href: '/docs/dayscript' },
  },
  {
    id: 'resource',
    term: 'Resource',
    definition:
      "A file under the project's `resource/` directory, such as a locale, an image, a font, an icon, or an asset, each staged through the platform's native resource system. Code reaches one through a generated constant under `res::` rather than a bare string. Each platform's own tooling processes, compresses, and packages them, so a resource ends up where that platform expects it, in the form it expects.",
    see: { title: 'Resources, images, fonts & icons', href: '/docs/resources' },
  },
  {
    id: 'locale',
    term: 'Locale',
    definition:
      "Which translation and formatting rules are in force. A locale is chosen from a CLI override, then the OS preference, then the app's default, and it can change while the app runs.",
    see: { title: 'Localization', href: '/docs/localization#switching-locale-at-runtime' },
  },
  {
    id: 'fluent',
    term: 'Fluent',
    definition:
      'Mozilla Fluent, the message format Day localizes with. Translations live in `resource/locales/<lang>/app.ftl`, one file per language, and `build.rs` turns each message into a typed function under `res::str`.',
    see: { title: 'Localization', href: '/docs/localization' },
  },
  {
    id: 'day-toml',
    term: 'Day.toml',
    definition:
      'The project manifest, and the marker that makes a Cargo package a Day project. It holds everything Day-specific, such as the app id and its targets; name and version come from `Cargo.toml` and are never restated.',
    see: { title: 'CLI & projects', href: '/docs/cli#the-conventional-project' },
  },
  {
    id: 'day-cli',
    term: 'day (the CLI)',
    definition:
      'The command-line tool that creates, builds, launches, packs, lints, and scripts Day projects, the same by hand, from CI, or from an IDE. Lowercase `day` is the binary and the Rust crate; capitalized Day is the framework.',
    see: { title: 'CLI & projects', href: '/docs/cli' },
    also: ['day-cli'],
  },
];

/** Entries in alphabetical order, for the page. */
export const sorted = [...glossary].sort((a, b) =>
  a.term.localeCompare(b.term, 'en', { sensitivity: 'base' }),
);
