// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// The glossary: one entry per term, read by two consumers. The Glossary page
// (src/content/docs/glossary.mdx) lists every entry; the rehype plugin
// (plugins/glossary-links.mjs) attaches an entry's definition to any docs link into
// `/docs/reference/glossary#<id>`, which DocsLayout shows as a popover.
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
    id: 'window',
    term: 'Window',
    definition:
      "A top-level host for UI content, with options and lifecycle distinct from the pieces it contains. Day exposes `WindowOptions` and `WindowHandle`; secondary content can fall back to a fullscreen cover when the toolkit cannot open another native window.",
    see: { title: 'Windows', href: '/docs/reference/windows' },
  },
  {
    id: 'control',
    term: 'Control',
    definition:
      "A user-interface element for presenting information or accepting interaction, such as a label, button, or slider. This is descriptive UI vocabulary, not a shared `Control` trait or a synonym for every Piece, Part, or window.",
    see: { title: 'Pieces', href: '/docs/explanation/pieces' },
  },
  {
    id: 'widget',
    term: 'Widget',
    definition:
      "A concrete UI object managed by a toolkit, such as an AppKit control or a DOM element. A Piece describes UI that Day realizes; composed pieces and logical layout nodes do not imply a one-to-one mapping to widgets.",
    see: { title: 'How rendering works', href: '/contributing/rendering' },
  },
  {
    id: 'piece',
    term: 'Piece',
    definition:
      "A UI description implementing `Piece`, consumed by `build` to produce a realized node. Constructors such as `label`, `button`, and `column` return pieces; a function returning `impl Piece` is a piece builder, and composition need not correspond to one native widget. The word also names a UI extension crate (`day-piece-*`).",
    see: { title: 'Pieces', href: '/docs/explanation/pieces' },
  },
  {
    id: 'part',
    term: 'Part',
    definition:
      "A headless capability shipped as an ordinary crate (`day-part-*`), such as battery status or location. Part names an ecosystem category, not a shared UI trait or a synonym for Piece; implementations and availability vary by platform.",
    see: { title: 'Device capabilities (parts)', href: '/docs/reference/parts' },
  },
  {
    id: 'tweak',
    term: 'Tweak',
    definition:
      'A supported way to reach the real native widget behind a built-in piece and configure it, while Day keeps owning layout and lifecycle. A piece with a tweak applied keeps the same widget, with a little more configured.',
    see: { title: 'Tweaks', href: '/docs/how-to/tweaks' },
  },
  {
    id: 'page',
    term: 'Page',
    definition:
      "One screen's worth of UI: a function that returns a piece, registered under a route and shown by a `nav` or a `nav_stack`. Pages live under `src/pages/` by convention. A window can show more than one at a time: a phone shows a single page, while the same app on a tablet or desktop may lay out a sidebar, a list, a detail page, and an inspector across several navigation levels.",
    see: { title: 'Navigation', href: '/docs/how-to/navigation' },
  },
  {
    id: 'toolkit',
    term: 'Toolkit',
    definition:
      "A UI system, such as AppKit, GTK, or the browser DOM, used by Day to realize UI. In Rust, the `Toolkit` trait in `day-spec` defines the implementation contract, including node operations and capability queries; it is distinct from an OS or build target.",
    see: { title: 'Overview', href: '/docs/explanation/overview#the-targets' },
  },
  {
    id: 'platform',
    term: 'Platform',
    definition:
      'An operating system a Day app is built for and deployed to: macOS, iOS, Android, Linux, Windows, HarmonyOS, or the web. A platform can host more than one toolkit; the pair is a target.',
    see: { title: 'Platform support', href: '/docs/reference/platforms' },
  },
  {
    id: 'target',
    term: 'Target',
    definition:
      "A Day build selection identified by an OS/toolkit pair, such as `macos-appkit` or `android-mdc`, in the CLI target registry. It is distinct from a Rust compiler target triple; registry entries also record host constraints, experimental status, and deprecation.",
    see: { title: 'Overview', href: '/docs/explanation/overview#the-targets' },
  },
  {
    id: 'backend',
    term: 'Toolkit implementation',
    definition:
      "The implementation of the `Toolkit` contract for a particular UI system, typically a crate such as `day-appkit` or `day-gtk`. The toolkit implements realization, updates, events, and capabilities; its presence does not establish complete feature support or a maintenance tier.",
    see: { title: 'Architecture', href: '/contributing/architecture' },
  },
  {
    id: 'native',
    term: 'Native',
    definition:
      "Made of the platform's own widgets. A Day `button()` is an `NSButton` on macOS and a `MaterialButton` on Android, so text input, scrolling, dark mode, and screen readers are the platform's too.",
    see: { title: 'Overview', href: '/docs/explanation/overview' },
  },
  {
    id: 'declarative',
    term: 'Declarative',
    definition:
      'Describing what the UI is rather than the steps that build it. A Day app is a declarative tree of pieces; Day turns the tree into widgets and keeps them in sync with your state.',
    see: { title: 'Overview', href: '/docs/explanation/overview' },
  },
  {
    id: 'imperative',
    term: 'Imperative',
    definition:
      'Step-by-step instructions to a widget, as opposed to a description of the result. Day keeps imperative work behind the reactive system, in the patches that update a native widget.',
    see: { title: 'Tweaks', href: '/docs/how-to/tweaks#reaching-a-widget-later' },
  },
  {
    id: 'reactive',
    term: 'Reactive',
    definition:
      "Updating by itself when something it reads changes. Day's reactive system is a fine-grained signal graph: state lives in signals, and a closure that reads one re-runs when it changes, ending in one native widget update.",
    see: { title: 'Reactivity', href: '/docs/explanation/reactivity' },
  },
  {
    id: 'signal',
    term: 'Signal',
    definition:
      "A `Signal<T>` is a reactive value handle that is `Copy` and not `Send`. Tracked reads inside a reactive computation subscribe it to changes; `Signal::new` allocates in the current scope. Use the write-only `Setter<T>` handle for cross-thread delivery rather than moving a signal to a worker.",
    see: { title: 'Reactivity', href: '/docs/explanation/reactivity#signal-and-memo-contracts' },
  },
  {
    id: 'binding',
    term: 'Binding',
    definition:
      'The link between a reactive closure and one native widget attribute: the closure re-runs when its reads change, and its result is applied to the widget. `label(move || …)` creates one. Also the trait behind two-way controls such as `slider(signal)`.',
    see: { title: 'Reactivity', href: '/docs/explanation/reactivity#effects-bindings-and-watches' },
  },
  {
    id: 'decorator',
    term: 'Decorator',
    definition:
      "A modifier such as `.padding()`, `.frame()`, or `.id()` that wraps a piece and returns `Decorated<P>`, keeping the piece's own type. Decorators live only in Day's tree and create no native widget.",
    see: { title: 'Pieces', href: '/docs/explanation/pieces#composing-trees' },
    also: ['modifier'],
  },
  {
    id: 'nav',
    term: 'Nav',
    definition:
      "The navigation piece for selecting among sections with a `Binding<K>` where `K: Route`; strings and app-defined route types are supported. Its presentation depends on style, size, and toolkit support, and the selection value is separate from page-local state.",
    see: { title: 'Navigation', href: '/docs/how-to/navigation#sections-nav' },
  },
  {
    id: 'sidebar',
    term: 'Sidebar',
    definition:
      "A list of sections beside the content. Day builds the selected page on demand and disposes it when the selection changes, so a sidebar's page state lives in your signals.",
    see: { title: 'Navigation', href: '/docs/how-to/navigation#sections-nav' },
  },
  {
    id: 'split-view',
    term: 'Split view',
    definition:
      'A window divided into a section list and a detail pane. It is how the platform draws a sidebar nav host when there is room; on narrow screens the same nav host pushes pages instead.',
    see: { title: 'API tour', href: '/docs/reference/api-tour#navigation' },
  },
  {
    id: 'route',
    term: 'Route',
    definition:
      'A typed navigation destination declared with the `routes!` macro: what `nav`, `nav_stack`, deep links, and dayscript `navigate` speak. Written as `segments/joined/by/slashes`; a single key is relative and a multi-segment path is absolute.',
    see: { title: 'Navigation', href: '/docs/how-to/navigation#routes-and-deep-links' },
  },
  {
    id: 'size-class',
    term: 'Size class',
    definition:
      "A bucket for a window's width or height in points, using Android's window size classes on every platform: `Compact`, `Medium`, `Expanded`, `Large`, and `ExtraLarge` for width. The class can change while the app runs, when a phone rotates or a desktop, tablet, or browser window is resized, and navigation re-resolves its shape each time, so one nav host fits a phone, a tablet, and a desktop window.",
    see: { title: 'Size classes', href: '/docs/reference/size-classes' },
  },
  {
    id: 'capability',
    term: 'Capability',
    definition:
      'What a toolkit can do, asked at run time with `capability(Cap::…)`: `Native`, `Emulated`, or `Unsupported`. An app checks before offering a toolbar or a menu bar. A device capability, such as the battery or location, is a different thing: that is a part.',
    see: { title: 'Menus, toolbars, and windows', href: '/docs/how-to/guide-desktop' },
    also: ['Cap'],
  },
  {
    id: 'placeholder',
    term: 'Placeholder',
    definition:
      'What you see where a piece has no native implementation on this toolkit: its kind, in angle brackets. Coverage grows toolkit by toolkit, and the placeholder marks what is left.',
    see: { title: 'The extension model', href: '/contributing/extending' },
  },
  {
    id: 'kind',
    term: 'Kind',
    definition:
      'The stable string that names a piece to every toolkit, such as `day.label` or `day.piece.lottie`. A toolkit dispatches a kind to its renderer; a kind it does not know draws a placeholder.',
    see: { title: 'How rendering works', href: '/contributing/rendering#the-realized-tree' },
  },
  {
    id: 'dayscript',
    term: 'dayscript',
    definition:
      "Day's automation language: a YAML file of steps that drives and asserts a running app. The engine is compiled into your app and executes steps as real Day events, so one script runs on every target and waits are deterministic.",
    see: { title: 'Testing with dayscript', href: '/docs/how-to/dayscript' },
  },
  {
    id: 'walkthrough',
    term: 'Walkthrough',
    definition:
      "The main script in a project's `dayscript/`, conventionally `walkthrough.yaml`. CI runs it on every target, and the gallery shows its captures.",
    see: { title: 'Testing with dayscript', href: '/docs/how-to/dayscript' },
  },
  {
    id: 'resource',
    term: 'Resource',
    definition:
      "A file under the project's `resource/` directory, such as a locale, an image, a font, an icon, or an asset, each staged through the platform's native resource system. Code reaches one through a generated constant under `res::` rather than a bare string. Each platform's own tooling processes, compresses, and packages them, so a resource ends up where that platform expects it, in the form it expects.",
    see: { title: 'Resources, images, fonts & icons', href: '/docs/how-to/resources' },
  },
  {
    id: 'locale',
    term: 'Locale',
    definition:
      "Which translation and formatting rules are in force. A locale is chosen from a CLI override, then the OS preference, then the app's default, and it can change while the app runs.",
    see: { title: 'Localization', href: '/docs/how-to/localization#switching-locale-at-runtime' },
  },
  {
    id: 'fluent',
    term: 'Fluent',
    definition:
      'Mozilla Fluent, the message format Day localizes with. Translations live in `resource/locales/<lang>/app.ftl`, one file per language, and `build.rs` turns each message into a typed function under `res::str`.',
    see: { title: 'Localization', href: '/docs/how-to/localization' },
  },
  {
    id: 'day-toml',
    term: 'Day.toml',
    definition:
      'The project manifest, and the marker that makes a Cargo package a Day project. It holds everything Day-specific, such as the app id and its targets; name and version come from `Cargo.toml` and are never restated.',
    see: { title: 'CLI & projects', href: '/docs/reference/cli#the-conventional-project' },
  },
  {
    id: 'day-cli',
    term: 'day (the CLI)',
    definition:
      'The command-line tool that creates, builds, launches, packs, lints, and scripts Day projects, the same by hand, from CI, or from an IDE. Lowercase `day` is the binary and the Rust crate; capitalized Day is the framework.',
    see: { title: 'CLI & projects', href: '/docs/reference/cli' },
    also: ['day-cli'],
  },
];

/** Entries in alphabetical order, for the page. */
export const sorted = [...glossary].sort((a, b) =>
  a.term.localeCompare(b.term, 'en', { sensitivity: 'base' }),
);
