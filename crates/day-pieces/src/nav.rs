// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Navigation. Imperative helpers (`navigate`, `nav_back`, `current_route`, `nav_link`); typed
//! routes (the `Route` trait, the `routes!` macro, `RoutePath`); and the host pieces that project
//! an app-owned `Signal` into native navigation — `selector` (tabs/sidebar), `stack` (push/pop),
//! and `cover` (modal) — including nested-stack merging.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal, bind, bind_seeded};
use day_spec::{Event, Size, kinds};

use crate::*;

// ---------------------------------------------------------------------------
// Navigation & tabs (docs/navigation.md, docs/tabs.md) — selector + stack, each a
// projection of an app-owned Signal.
// ---------------------------------------------------------------------------

/// Navigate to a route (docs/navigation.md).
///
/// * A single key (`navigate("inbox")`) is RELATIVE — the innermost route surface is tried
///   first, falling through outward; `""` pops the innermost stack to its root.
/// * A `/`-separated path (`navigate("mail/inbox/msg-42")`) is ABSOLUTE — anchored at the
///   outermost surface that knows the first segment, everything inside reset, the remaining
///   segments consumed inward (surfaces mounting during the cascade take theirs as they appear).
/// * A trailing `?name=value&…` carries [`route_params`] to the destination builders.
///
/// False = no surface recognized the (first) segment.
pub fn navigate(path: &str) -> bool {
    day_core::navigate(path)
}

/// Pop one navigation level. False = nothing to pop.
pub fn nav_back() -> bool {
    day_core::nav_back()
}

/// The FULL current route — every mounted surface's contribution, outermost to innermost,
/// `/`-joined. Round-trips through [`navigate`]: persist it on exit, `navigate(&saved)` on
/// launch (docs/navigation.md).
pub fn current_route() -> Option<String> {
    day_core::current_route()
}

/// The query params of the most recent [`navigate`] (`?name=value&…`) — read inside a
/// destination builder. See docs/navigation.md for when params apply.
pub fn route_params() -> std::rc::Rc<Vec<(String, String)>> {
    day_core::route_params()
}

/// One query param of the most recent [`navigate`] (`None` = not present).
pub fn route_param(name: &str) -> Option<String> {
    day_core::route_param(name)
}

/// A tappable link that navigates to `path` when pressed.
pub fn nav_link<M>(label: impl IntoText<M>, path: &str) -> Button {
    let path = path.to_string();
    button(label).action(move || {
        let _ = day_core::navigate(&path);
    })
}

// ---------------------------------------------------------------------------
// Typed routes (docs/navigation.md) — routes as data instead of string encoding.
// ---------------------------------------------------------------------------

/// A typed route key — the compile-checked alternative to raw string keys.
///
/// Implement on an enum (one variant per destination) and use it everywhere a key goes:
/// `selector(Signal<Option<Section>>)` + `.item(Section::Controls, …)`,
/// `stack(Signal<Vec<Drill>>, …)` + `.destination(|d: &Drill| …)`, [`navigate_to`], [`route`].
/// The string layer stays the wire format — deep links, dayscript, and [`current_route`]
/// still speak [`Route::key`] strings — but app code never assembles or splits them.
///
/// Variants can carry data (`Item { id: u32 }` ↔ `"item-42"`): encode it in [`Route::key`],
/// parse it back in [`Route::from_key`], and destination builders receive the typed value.
/// For plain data-free enums the [`routes!`] macro writes both sides.
pub trait Route: Clone + PartialEq + 'static {
    /// The path segment this value occupies in a route string. Must round-trip through
    /// [`Route::from_key`] and must not be empty — `""` means "no selection" (see the
    /// `Option<R>` impl).
    fn key(&self) -> String;
    /// Parse a path segment back into the typed value; `None` = not one of this type's routes.
    fn from_key(key: &str) -> Option<Self>;
    /// The human-readable title shown in the native navigation bar when this route is the top of
    /// a [`stack`]. Defaults to [`key`](Route::key); override it to show a display name (e.g. an
    /// app's name) instead of the wire key.
    fn title(&self) -> String {
        self.key()
    }
}

/// Raw string keys — the untyped baseline. Every segment parses.
impl Route for String {
    fn key(&self) -> String {
        self.clone()
    }
    fn from_key(key: &str) -> Option<Self> {
        Some(key.to_string())
    }
}

/// `None` ↔ `""` (no selection) — the key type for a sidebar [`selector`], whose collapsed
/// mobile state IS "nothing selected". `.item(Section::X, …)` still takes the bare value
/// (`Section: Into<Option<Section>>`).
impl<R: Route> Route for Option<R> {
    fn key(&self) -> String {
        match self {
            Some(r) => r.key(),
            None => String::new(),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        if key.is_empty() {
            Some(None)
        } else {
            R::from_key(key).map(Some)
        }
    }
}

/// Define a plain routes enum and its [`Route`] impl in one shot:
///
/// ```ignore
/// day::routes! {
///     pub enum Section { Controls => "controls", Text => "text" }
/// }
/// selector(section).item(Section::Controls, tr("controls"), controls_page)
/// ```
///
/// Variants that carry data (`Item { id: u32 }` ↔ `"item-42"`) implement [`Route`] by hand.
#[macro_export]
macro_rules! routes {
    ($(#[$meta:meta])* $vis:vis enum $name:ident {
        $($(#[$vmeta:meta])* $variant:ident => $key:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        $vis enum $name { $($(#[$vmeta])* $variant),+ }
        impl $crate::Route for $name {
            fn key(&self) -> String {
                match self { $(Self::$variant => ($key).to_string()),+ }
            }
            fn from_key(key: &str) -> Option<Self> {
                match key { $($key => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

/// A typed absolute route: segments built from [`Route`] values plus query params.
/// `route(&Section::Stack).then(&Drill::Item { id: 42 }).param("hint", "linked")` encodes to
/// `"stack/item-42?hint=linked"` — [`RoutePath::navigate`] it, or hand it to [`nav_link_to`].
#[derive(Clone, Debug, Default)]
pub struct RoutePath {
    segments: Vec<String>,
    params: Vec<(String, String)>,
}

/// Start a typed [`RoutePath`] at the outermost segment.
pub fn route(first: &impl Route) -> RoutePath {
    RoutePath {
        segments: vec![first.key()],
        params: Vec::new(),
    }
}

impl RoutePath {
    /// Append the next-inner segment.
    pub fn then(mut self, next: &impl Route) -> Self {
        self.segments.push(next.key());
        self
    }
    /// Append a query param (the destination reads it via [`route_param`]).
    pub fn param(mut self, name: &str, value: impl std::fmt::Display) -> Self {
        self.params.push((name.to_string(), value.to_string()));
        self
    }
    /// The encoded route string (percent-escaped where needed) — what [`navigate`] accepts.
    pub fn to_route(&self) -> String {
        day_core::encode_route(&self.segments, &self.params)
    }
    /// Navigate to this path. False = no surface recognized the first segment.
    pub fn navigate(&self) -> bool {
        day_core::navigate(&self.to_route())
    }
}

impl std::fmt::Display for RoutePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_route())
    }
}

/// Navigate to a single typed key, RELATIVE (innermost surface first) — the typed
/// `navigate(&r.key())`, percent-escaped. For absolute paths chain a [`route`].
pub fn navigate_to(r: &impl Route) -> bool {
    day_core::navigate(&day_core::encode_route(std::slice::from_ref(&r.key()), &[]))
}

/// A tappable link that navigates to a typed [`RoutePath`] when pressed.
pub fn nav_link_to<M>(label: impl IntoText<M>, path: RoutePath) -> Button {
    let path = path.to_route();
    button(label).action(move || {
        let _ = day_core::navigate(&path);
    })
}

// ---------------------------------------------------------------------------
// Nested-nav merge (docs/navigation.md): a `stack()` built inside a page of an enclosing NAV
// host that presents as a push stack (mobile, `split == false`) pushes its pages onto THAT host
// instead of minting a second native container — one native nav chain, one back button. The
// enclosing host is threaded to nested pieces at build time via a thread-local context stack;
// `owners` is the per-host ordered stack of "what a back on the topmost page does".
// ---------------------------------------------------------------------------

/// Performs the topmost page's back action. Arg = the toolkit already popped natively (iOS/Android
/// system back), so the owner must not re-issue a pop.
type PopOwner = Rc<dyn Fn(bool)>;

#[derive(Clone)]
struct NavHostCx {
    host: RNode,
    sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>>,
    /// One entry per page pushed above the root, in native order; the host's single `NavBack`
    /// handler invokes the last.
    owners: Rc<RefCell<Vec<PopOwner>>>,
    /// The enclosing host presents as split panes (desktop). A nested stack does NOT merge into a
    /// split host — it keeps its own detail-pane stack.
    split: bool,
}

thread_local! {
    /// Build-time stack of enclosing nav hosts. `None` is a barrier (a resident container such as
    /// tabs) that a nested stack must not merge through.
    static NAV_HOST_CX: RefCell<Vec<Option<NavHostCx>>> = const { RefCell::new(Vec::new()) };
}

/// Run `f` with `cx` as the innermost nav-host context (a barrier when `None`), restoring after.
fn with_nav_host<R>(cx: Option<NavHostCx>, f: impl FnOnce() -> R) -> R {
    NAV_HOST_CX.with(|s| s.borrow_mut().push(cx));
    let r = f();
    NAV_HOST_CX.with(|s| {
        s.borrow_mut().pop();
    });
    r
}

/// The innermost mergeable nav host, if any (a barrier or an empty stack yields `None`).
fn current_nav_host() -> Option<NavHostCx> {
    NAV_HOST_CX.with(|s| s.borrow().last().cloned().flatten())
}

/// Create a NAV_PAGE under `host` and wire its FrameChanged size reports into `sizes`
/// (the native container owns each page's frame; Day lays content out at the reported size).
fn nav_page(
    host: RNode,
    props: &day_spec::props::NavPageProps,
    sizes: &Rc<RefCell<std::collections::HashMap<RNode, Size>>>,
) -> RNode {
    let mut cx = BuildCx::new(host);
    let page = cx.native(
        kinds::NAV_PAGE,
        props,
        Rc::new(PassThrough),
        Flex::default(),
        Boundary::Yes,
    );
    let sizes = sizes.clone();
    cx.on(page, move |ev| {
        if let Event::FrameChanged(sz) = ev {
            let changed = sizes.borrow().get(&page) != Some(sz);
            if changed {
                sizes.borrow_mut().insert(page, *sz);
                with_tree(|t| {
                    t.mark_needs_measure(page);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        }
    });
    page
}

/// Create a TABS_PAGE under `host`, wiring its FrameChanged reports into `sizes`.
fn tabs_page(
    host: RNode,
    props: &day_spec::props::TabsPageProps,
    sizes: &Rc<RefCell<std::collections::HashMap<RNode, Size>>>,
) -> RNode {
    let mut cx = BuildCx::new(host);
    let page = cx.native(
        kinds::TABS_PAGE,
        props,
        Rc::new(PassThrough),
        Flex::default(),
        Boundary::Yes,
    );
    let sizes = sizes.clone();
    cx.on(page, move |ev| {
        if let Event::FrameChanged(sz) = ev {
            let changed = sizes.borrow().get(&page) != Some(sz);
            if changed {
                sizes.borrow_mut().insert(page, *sz);
                with_tree(|t| {
                    t.mark_needs_measure(page);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        }
    });
    page
}

/// Register a string-route adapter over a route surface's own signal, so `navigate()` /
/// deep links / dayscript keep working by key. This is a *convenience layer* — the surface
/// itself is driven by the signal, not by this registry (docs/navigation.md).
///
/// `enter` consumes one segment of an ABSOLUTE path (`navigate("a/b/c")`); `segments` is the
/// surface's contribution to the full [`current_route`].
fn register_route_surface(
    push: impl Fn(&str) -> bool + 'static,
    pop: impl Fn(bool) -> bool + 'static,
    current: impl Fn() -> String + 'static,
    enter: impl Fn(&str) -> bool + 'static,
    segments: impl Fn() -> Vec<String> + 'static,
) {
    let token = day_core::register_nav(day_core::NavController {
        push: Box::new(push),
        pop: Box::new(pop),
        current: Box::new(current),
        enter: Box::new(enter),
        segments: Box::new(segments),
    });
    Scope::current().on_cleanup(move || day_core::unregister_nav(token));
}

thread_local! {
    /// How many routed one-of-N surfaces (`selector`/tabs) are live at each nesting depth. Two at
    /// the same depth are siblings whose keys both flow into `current_route()` — the case that
    /// wants `.local()` (docs/navigation.md). Used only to warn; never changes behavior.
    static ROUTED_ONE_OF_N: RefCell<std::collections::HashMap<usize, usize>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Note a routed selector/tabs at the current nav depth and, in debug builds, warn if it is a
/// sibling of another routed one-of-N surface — the `.local()` footgun (docs/navigation.md). The
/// count is decremented when the surface's scope disposes, so switching sections doesn't leak.
/// A stack is exempt (its whole path is one surface's contribution; sibling stacks are a
/// deliberate, documented layout), as is a cover (its segment is empty unless presented).
fn note_routed_one_of_n(kind: &str) {
    let depth = NAV_HOST_CX.with(|s| s.borrow().len());
    let count = ROUTED_ONE_OF_N.with(|m| {
        let mut m = m.borrow_mut();
        let c = m.entry(depth).or_insert(0);
        *c += 1;
        *c
    });
    if count > 1 {
        warn_sibling_selectors(kind);
    }
    Scope::current().on_cleanup(move || {
        ROUTED_ONE_OF_N.with(|m| {
            if let Some(c) = m.borrow_mut().get_mut(&depth) {
                *c = c.saturating_sub(1);
            }
        });
    });
}

#[cfg(debug_assertions)]
fn warn_sibling_selectors(kind: &str) {
    eprintln!(
        "day: two routed one-of-N surfaces ({kind}) are mounted at the same navigation level. \
         Their keys both flow into current_route(), so you'll see `section/childA/childB` and \
         `navigate(\"child\")` is ambiguous. Mark all but the primary one `.local()` \
         (docs/navigation.md)."
    );
}

#[cfg(not(debug_assertions))]
fn warn_sibling_selectors(_kind: &str) {}

// ===========================================================================
// Selector — one-of-N, bound to a Signal<String> of the active key.
// ===========================================================================

/// How a [`selector`] presents its one-of-N choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectorStyle {
    /// A native tab widget: NSTabView / UITabBarController / AdwViewStack / QTabWidget /
    /// BottomNavigationView. All pages resident; each keeps its state.
    Tabs,
    /// A NavigationSplitView: a sidebar list + a detail. Desktop shows both panes (on GTK an
    /// `AdwNavigationSplitView`); mobile collapses to a list that pushes the detail.
    Sidebar,
}

/// Builds the page for a data-driven key (`&K` → piece) — a selector's `.destination` fallback
/// or a stack's `.destination`.
type DestFn<K> = Rc<dyn Fn(&K) -> AnyPiece>;

/// Completions for a search field's current text (`Selector::search_suggestions`).
type SuggestFn = Rc<dyn Fn(&str) -> Vec<String>>;

/// The flattened live rows a selector's dynamic blocks derive to: per-row key strings, typed
/// keys, titles, and icons (index-aligned). Carried through the reconcile `bind`.
/// One tracked derive of a selector's rows: (key strings, typed keys, titles, icons, badges,
/// section headers). Compared by equality to gate the re-patch, so every decoration a backend
/// renders has to ride along — a badge that changed but was not carried here would not repaint.
type DerivedRows<K> = (
    Vec<String>,
    Vec<K>,
    Vec<String>,
    Vec<Option<String>>,
    Vec<Option<String>>,
    Vec<Option<String>>,
    Vec<Option<day_spec::Color>>,
    Vec<Vec<day_spec::MenuItem>>,
);

struct SelItem<K> {
    key: K,
    title: TextSource,
    /// Optional bundled-image name for the item's native icon (docs/navigation.md).
    icon: Option<String>,
    /// Optional per-row icon tint (docs/vectors.md).
    tint: Option<day_spec::Color>,
    /// Per-row context menu (docs/menus.md), already lowered — items carry registered
    /// action ids. Empty = no menu.
    menu: Vec<day_spec::MenuItem>,
    /// Trailing accessory (an unread count). A `TextSource` so a live count retitles on its
    /// own signal, and so a localized badge follows `set_locale`.
    badge: Option<TextSource>,
    /// Header introducing the group this item opens (docs/navigation.md).
    section: Option<TextSource>,
    /// Immersive-chrome page (docs/navigation.md): keeps the floating transparent bar on
    /// backends with an immersive nav mode; standard opaque bar otherwise.
    immersive: bool,
    /// A static item carries its own page builder; a dynamic item (from `.items`) leaves this
    /// `None` and its page is built by the selector's `.destination` fallback.
    build: Option<Box<dyn Fn() -> AnyPiece>>,
}

/// One data-driven selector item, returned by [`item`] inside a [`Selector::items`] mapper.
pub struct NavItem<K = String> {
    key: K,
    title: TextSource,
    icon: Option<String>,
    icon_tint: Option<day_spec::Color>,
    menu: Vec<day_spec::MenuItem>,
    badge: Option<TextSource>,
    section: Option<TextSource>,
    immersive: bool,
}

/// A selector item for a data-driven list: `item(room.id, room.name).icon(res::images::room)`
/// (docs/navigation.md). Used inside the `.items(signal, |t| …)` mapper; the page it selects is
/// built by the selector's [`Selector::destination`].
pub fn item<M, K, I: Into<K>>(key: I, title: impl IntoText<M>) -> NavItem<K> {
    NavItem {
        key: key.into(),
        icon_tint: None,
        menu: Vec::new(),
        title: title.into_text(),
        icon: None,
        badge: None,
        section: None,
        immersive: false,
    }
}

impl<K> NavItem<K> {
    /// A bundled-image name for the item's native icon (same convention as
    /// [`Selector::item_icon`]).
    pub fn icon(mut self, icon: impl Into<day_spec::ImageName>) -> Self {
        self.icon = Some(icon.into().as_str().to_owned());
        self
    }
    /// Tint this row's icon (docs/vectors.md): the glyph recolors to `color` instead of the
    /// backend's neutral template tint. Backends without per-row tinting keep the neutral look.
    pub fn icon_tint(mut self, color: day_spec::Color) -> Self {
        self.icon_tint = Some(color);
        self
    }
    /// Attach a context menu to this row (docs/menus.md): shown on secondary-click /
    /// long-press, the same `menu_item`/`sub_menu` builders as everywhere else. A chosen
    /// entry runs its action exactly like a piece context menu. Backends without per-row
    /// menus drop it (see the matrix in docs/menus.md).
    pub fn context_menu(mut self, entries: Vec<crate::menus::MenuEntry>) -> Self {
        self.menu = crate::menus::lower_menu(entries);
        self
    }
    /// A trailing accessory for this row — an unread count, a status. Rendered right-aligned
    /// and de-emphasized where the toolkit has an affordance for it, and dropped where it does
    /// not (see docs/coverage-matrix.md).
    pub fn badge<M>(mut self, badge: impl IntoText<M>) -> Self {
        self.badge = Some(badge.into_text());
        self
    }
    /// Open a new section with this header, immediately before this row.
    pub fn section<M>(mut self, title: impl IntoText<M>) -> Self {
        self.section = Some(title.into_text());
        self
    }
    /// Mark this item's pushed page immersive-chrome (docs/navigation.md) — the data-driven
    /// counterpart of [`Selector::immersive`].
    pub fn immersive(mut self) -> Self {
        self.immersive = true;
        self
    }
}

/// A source of selector items: a fixed item, or a signal-driven block that re-derives its items
/// when the signal changes (docs/navigation.md).
enum ItemSource<K> {
    Static(SelItem<K>),
    /// Reads a signal (tracked) and maps its elements to items. Called to (re)derive the block.
    Dynamic(Box<dyn Fn() -> Vec<SelItem<K>>>),
}

// A selector's item sources reduced to what both `build_sidebar` and `build_tabs` need: a
// per-key page builder for STATIC items, the ordered metadata sources (statics + dynamic
// blocks) that `derive` walks to produce the live row list, and the `.destination` fallback
// for data-driven keys. `derive` is called untracked for the first build and tracked inside a
// reactive effect that re-patches the native rows when a dynamic block's signal changes.
enum MetaSource<K> {
    Static(K, TextSource, RowMeta, bool),
    Dynamic(Box<dyn Fn() -> Vec<SelItem<K>>>),
}

/// The non-title decorations of one selector row.
#[derive(Clone, Default)]
struct RowMeta {
    icon: Option<String>,
    tint: Option<day_spec::Color>,
    menu: Vec<day_spec::MenuItem>,
    badge: Option<TextSource>,
    section: Option<TextSource>,
}

/// One selector's live rows, flattened across its static items and dynamic blocks.
struct NavRows<K> {
    keys: Vec<K>,
    titles: Vec<String>,
    icons: Vec<Option<String>>,
    badges: Vec<Option<String>>,
    sections: Vec<Option<String>>,
    tints: Vec<Option<day_spec::Color>>,
    menus: Vec<Vec<day_spec::MenuItem>>,
}

struct SelItems<K> {
    static_builders: std::collections::HashMap<String, Box<dyn Fn() -> AnyPiece>>,
    meta: Rc<Vec<MetaSource<K>>>,
    destination: Option<DestFn<K>>,
}

impl<K: Route> SelItems<K> {
    fn from_sources(sources: Vec<ItemSource<K>>, destination: Option<DestFn<K>>) -> Self {
        let mut static_builders = std::collections::HashMap::new();
        let mut meta = Vec::new();
        for src in sources {
            match src {
                ItemSource::Static(it) => {
                    if let Some(b) = it.build {
                        static_builders.insert(it.key.key(), b);
                    }
                    meta.push(MetaSource::Static(
                        it.key,
                        it.title,
                        RowMeta {
                            icon: it.icon,
                            tint: it.tint,
                            menu: it.menu,
                            badge: it.badge,
                            section: it.section,
                        },
                        it.immersive,
                    ));
                }
                ItemSource::Dynamic(f) => {
                    meta.push(MetaSource::Dynamic(f));
                }
            }
        }
        SelItems {
            static_builders,
            meta: Rc::new(meta),
            destination,
        }
    }

    /// The flat live rows: (typed keys, resolved titles, icons). TRACKED on purpose: reading
    /// a `Dynamic` block's signal subscribes the caller (the derive effect) to row changes,
    /// and resolving each title through [`TextSource::resolve`] subscribes it to the locale —
    /// `set_locale` re-runs the effect and the native rows retitle (docs/navigation.md).
    fn derive(&self) -> NavRows<K> {
        let mut r = NavRows {
            keys: Vec::new(),
            titles: Vec::new(),
            icons: Vec::new(),
            badges: Vec::new(),
            sections: Vec::new(),
            tints: Vec::new(),
            menus: Vec::new(),
        };
        let mut push = |k: K, title: String, m: &RowMeta| {
            r.keys.push(k);
            r.titles.push(title);
            r.icons.push(m.icon.clone());
            r.badges.push(m.badge.as_ref().map(|b| b.resolve()));
            r.sections.push(m.section.as_ref().map(|s| s.resolve()));
            r.tints.push(m.tint);
            r.menus.push(m.menu.clone());
        };
        for ms in self.meta.iter() {
            match ms {
                MetaSource::Static(k, t, m, _) => push(k.clone(), t.resolve(), m),
                MetaSource::Dynamic(f) => {
                    for it in f() {
                        let m = RowMeta {
                            icon: it.icon,
                            tint: it.tint,
                            menu: it.menu,
                            badge: it.badge,
                            section: it.section,
                        };
                        let title = it.title.resolve();
                        push(it.key, title, &m);
                    }
                }
            }
        }
        r
    }

    /// An item's immersive-chrome flag (docs/navigation.md). A data-driven key checks its
    /// block's current rows UNTRACKED — chrome is resolved at push time, not a dependency of
    /// the push (a tracked read here would re-run the selection effect on every list change).
    fn immersive_of(&self, key: &str) -> bool {
        self.meta.iter().any(|ms| match ms {
            MetaSource::Static(k, _, _, imm) => *imm && k.key() == key,
            MetaSource::Dynamic(f) => {
                day_reactive::untrack(|| f().iter().any(|it| it.immersive && it.key.key() == key))
            }
        })
    }

    /// A static item's live title source (locale-reactive retitle); `None` for a data-driven
    /// key, whose title is a resolved snapshot from the derived list.
    fn static_title(&self, key: &str) -> Option<TextSource> {
        self.meta.iter().find_map(|ms| match ms {
            MetaSource::Static(k, t, _, _) if k.key() == key => Some(t.clone()),
            _ => None,
        })
    }

    /// Build a key's page: a static item's own builder, else the `.destination` fallback (data-
    /// driven key), else a blank leaf (misconfigured — a dynamic item with no `.destination`).
    fn build_page(&self, key: &K) -> AnyPiece {
        if let Some(b) = self.static_builders.get(&key.key()) {
            b()
        } else if let Some(d) = &self.destination {
            d(key)
        } else {
            piece_fn(|cx| cx.layout_only(Rc::new(PassThrough), Flex::default(), Boundary::No)).any()
        }
    }
}

/// A one-of-N selector whose active key is an app-owned signal (two-way, exactly like
/// `Picker`/`Toggle`). Deep links and dayscript address items by key (docs/navigation.md).
///
/// The key type is any [`Route`]: `String` for raw keys, or a typed enum — use
/// `Signal<Option<Section>>` for a sidebar (`None` = the collapsed mobile list) and
/// `Signal<Tab>` for tabs (always selected).
///
/// ```ignore
/// let section = Signal::new("home".to_string());   // or Signal::new(None::<Section>)
/// selector(section).style(SelectorStyle::Sidebar)
///     .item("home", tr("home"), home_page)         // or .item(Section::Home, …)
///     .item("settings", tr("settings"), settings_page)
/// ```
pub struct Selector<S: SignalRw<K>, K: Route = String> {
    selection: S,
    style: SelectorStyle,
    title: TextSource,
    header: Option<Box<dyn FnOnce() -> AnyPiece>>,
    sources: Vec<ItemSource<K>>,
    /// Builds the page for a key with no static item (a data-driven item from `.items`).
    destination: Option<DestFn<K>>,
    /// Whether this selector contributes to the app route (deep links / dayscript). `false`
    /// (`.local()`) for a selector used as a self-contained widget, so it does not add a segment
    /// to `current_route` or intercept `navigate` (docs/navigation.md).
    routed: bool,
    /// The persistence key set by [`Selector::restore`]: the selected item's key is saved here on
    /// every change and restored at build (unless a launch deep link is pending). `None` = not
    /// persisted.
    restore: Option<String>,
    /// A header from [`Selector::section`] waiting to be attached to the next item added.
    pending_section: Option<TextSource>,
    /// An optional trailing nav-bar action ([`Selector::bar_action`]) — the mobile stand-in for a
    /// desktop toolbar button. `None` unless set.
    bar_action: Option<BarActionSpec>,
    /// Search over this surface ([`Selector::searchable`]). `None` unless set.
    search: Option<SearchSpec>,
}

/// A pending search declaration ([`Selector::searchable`] and its modifiers). The query and the
/// scope are app-owned signals, which is what lets the FIELD move between the toolbar and the
/// navigation list without the STATE moving with it (docs/search.md).
struct SearchSpec {
    query: Signal<String>,
    prompt: Option<TextSource>,
    placement: day_spec::props::SearchPlacement,
    /// Scope titles and the signal holding the chosen index. Empty titles = no scope bar.
    scopes: Vec<TextSource>,
    scope: Option<Signal<usize>>,
    /// Completions for the current text, re-derived whenever its reactive reads change.
    suggestions: Option<SuggestFn>,
}

impl SearchSpec {
    /// The props the host is realized with: current values only. Everything live rides the
    /// bindings in [`SearchSpec::bind`].
    fn lower(&self) -> day_spec::props::SearchProps {
        let text = self.query.get_untracked();
        day_spec::props::SearchProps {
            suggestions: self
                .suggestions
                .as_ref()
                .map(|f| day_reactive::untrack(|| f(&text)))
                .unwrap_or_default(),
            text,
            prompt: self
                .prompt
                .as_ref()
                .map(TextSource::initial)
                .unwrap_or_default(),
            placement: self.placement,
            scopes: self.scopes.iter().map(TextSource::initial).collect(),
            scope: self.scope.map(|s| s.get_untracked()).unwrap_or(0),
            active: false,
        }
    }

    /// Wire the two-way bindings once the host exists: the app's writes patch the live field,
    /// and the user's edits (arriving as `Event::SearchChanged` / `SearchScopeChanged`) write the
    /// app's signals. Both directions go through the SIGNAL, never through the widget, which is
    /// what lets a later placement change move the field without moving the state.
    fn bind(&self, host: RNode, seed: &day_spec::props::SearchProps) {
        use day_spec::props::SearchPatch;
        let query = self.query;
        // Text: app → field. Seeded, because `lower` already put the current value in the realize
        // props — re-applying it here would be the duplicate op §5.2 forbids.
        bind_seeded(
            seed.text.clone(),
            move || query.get(),
            move |text| {
                let p = SearchPatch::Text(text.clone());
                with_tree(|t| t.patch(host, Box::new(p), false));
            },
        );
        if let Some(sig) = self.scope {
            bind_seeded(
                seed.scope,
                move || sig.get(),
                move |i| {
                    let p = SearchPatch::Scope(*i);
                    with_tree(|t| t.patch(host, Box::new(p), false));
                },
            );
        }
        // Completions re-derive on every keystroke AND on whatever else the closure reads.
        if let Some(f) = self.suggestions.clone() {
            bind_seeded(
                seed.suggestions.clone(),
                move || f(&query.get()),
                move |list| {
                    let p = SearchPatch::Suggestions(list.clone());
                    with_tree(|t| t.patch(host, Box::new(p), false));
                },
            );
        }
    }
}

/// A pending nav-bar action ([`Selector::bar_action`] / [`Stack::bar_action`]): the bundled icon
/// name, the label source, and the closure to run. Lowered at build into [`NavProps::bar_action`]
/// (docs/navigation.md) — the closure is registered with day-core for a dispatch id the backend
/// emits as `Event::MenuAction` on tap.
struct BarActionSpec {
    icon: Option<String>,
    label: TextSource,
    action: Rc<dyn Fn()>,
}

impl BarActionSpec {
    /// Register the closure (getting a dispatch id) and resolve the label, producing the spec
    /// value the NAV host carries. Called once, at build.
    fn lower(self) -> day_spec::props::NavBarAction {
        day_spec::props::NavBarAction {
            action: day_core::register_menu_action(self.action),
            label: self.label.initial(),
            icon: self.icon,
        }
    }
}

pub fn selector<K: Route, S: SignalRw<K>>(selection: S) -> Selector<S, K> {
    Selector {
        selection,
        style: SelectorStyle::Sidebar,
        pending_section: None,
        title: TextSource::Static(String::new()),
        header: None,
        sources: Vec::new(),
        destination: None,
        routed: true,
        restore: None,
        bar_action: None,
        search: None,
    }
}

impl<K: Route, S: SignalRw<K>> Selector<S, K> {
    pub fn style(mut self, style: SelectorStyle) -> Self {
        self.style = style;
        self
    }
    /// The sidebar / window title (Sidebar style).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text();
        self
    }
    /// Open a section: the NEXT item added (static or the first row of the next `.items` block)
    /// carries this header. Backends without grouped rows ignore it and show one flat list, so
    /// a section is a grouping hint, never a source of items the user cannot otherwise reach.
    ///
    /// ```ignore
    /// selector(sel)
    ///     .section(res::str::smart_feeds())
    ///     .item_icon("today", res::str::today(), res::images::today, today_page)
    ///     .section(res::str::feeds())
    ///     .items(move || st.feeds.get(), |f| item(f.id, f.name))
    /// ```
    pub fn section<M>(mut self, title: impl IntoText<M>) -> Self {
        self.pending_section = Some(title.into_text());
        self
    }
    /// Attach a trailing badge — an unread count, a status — to the item just added:
    /// `.item(…).badge(move || n.get().to_string())`. Reactive, so a live count repaints on
    /// its own signal. An empty string draws nothing, which is the natural "zero" case.
    ///
    /// Ignored (in release) when no static item precedes it; data-driven rows carry their own
    /// badge via [`NavItem::badge`].
    pub fn badge<M>(mut self, badge: impl IntoText<M>) -> Self {
        match self.sources.last_mut() {
            Some(ItemSource::Static(it)) => it.badge = Some(badge.into_text()),
            _ => debug_assert!(false, "day: `.badge(…)` needs a preceding `.item…(…)`"),
        }
        self
    }
    /// An optional piece shown above the sidebar list (a logo, app name…).
    pub fn header<P: Piece>(mut self, build: impl FnOnce() -> P + 'static) -> Self {
        self.header = Some(Box::new(move || AnyPiece::new(build())));
        self
    }
    /// Add a destination. `key` addresses it (navigate / deep link / dayscript); `title` is
    /// its label; `build` runs when the item is first shown. For a typed selector over
    /// `Option<Section>` pass the bare `Section::X`.
    pub fn item<M, P: Piece>(
        mut self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        let section = self.pending_section.take();
        self.sources.push(ItemSource::Static(SelItem {
            key: key.into(),
            title: title.into_text(),
            icon: None,
            tint: None,
            menu: Vec::new(),
            badge: None,
            section,
            immersive: false,
            build: Some(Box::new(move || AnyPiece::new(build()))),
        }));
        self
    }
    /// Like [`item`](Self::item) but with a native icon: `icon` is a bundled-image name (typed
    /// [`ImageName`](day_spec::ImageName), resolved like [`image`], e.g. `res::images::nav_home`)
    /// shown beside the label where the backend's nav supports it (e.g. the Windows
    /// NavigationView, the iOS/macOS source list). Backends that can't decorate rows ignore it.
    pub fn item_icon<M, P: Piece>(
        mut self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        icon: impl Into<day_spec::ImageName>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        let section = self.pending_section.take();
        self.sources.push(ItemSource::Static(SelItem {
            key: key.into(),
            title: title.into_text(),
            icon: Some(icon.into().as_str().to_owned()),
            tint: None,
            menu: Vec::new(),
            badge: None,
            section,
            immersive: false,
            build: Some(Box::new(move || AnyPiece::new(build()))),
        }));
        self
    }
    /// Mark the LAST-added `.item`/`.item_icon` destination as an immersive-chrome page
    /// (docs/navigation.md): on backends with an immersive nav mode (android edge-to-edge
    /// today) its pushed page keeps the floating transparent bar over full-bleed content;
    /// unmarked pages get the standard opaque bar. A no-op on every other backend. For a
    /// data-driven `.items` block, mark individual rows with [`NavItem::immersive`] instead.
    pub fn immersive(mut self) -> Self {
        if let Some(ItemSource::Static(item)) = self.sources.last_mut() {
            item.immersive = true;
        }
        self
    }
    /// A data-driven item block: `.items(rooms_signal, |r| item(r.id, r.name).icon(…))`
    /// (docs/navigation.md). The block re-derives whenever the signal changes — rows are added
    /// and removed on the native sidebar/tab widget, and if the selected key disappears the
    /// selection resets (to `None` for an `Option` key). Static `.item`s and dynamic blocks may
    /// be mixed; the final list is their declaration order. Pair with [`destination`] to build
    /// the page for a data-driven key.
    ///
    /// [`destination`]: Self::destination
    pub fn items<T: Clone + 'static>(
        mut self,
        items: impl Fn() -> Vec<T> + 'static,
        map: impl Fn(&T) -> NavItem<K> + 'static,
    ) -> Self {
        // `items` is a TRACKED reader (a `Signal<Vec<T>>` via its `Fn()` deref, or a closure) —
        // reading it inside the derive effect subscribes the selector to changes.
        // A pending `.section(…)` heads the block's FIRST row, wherever the data starts.
        let section = self.pending_section.take();
        self.sources.push(ItemSource::Dynamic(Box::new(move || {
            items()
                .iter()
                .enumerate()
                .map(|(i, t)| {
                    let mut ni = map(t);
                    if i == 0 && ni.section.is_none() {
                        ni.section = section.clone();
                    }
                    SelItem {
                        key: ni.key,
                        title: ni.title,
                        icon: ni.icon,
                        tint: ni.icon_tint,
                        menu: ni.menu,
                        badge: ni.badge,
                        section: ni.section,
                        immersive: ni.immersive,
                        build: None,
                    }
                })
                .collect()
        })));
        self
    }
    /// Build the page for a data-driven key (one added by [`items`](Self::items) with no static
    /// item). Mirrors [`Stack::destination`]; unused for a purely static selector.
    pub fn destination<P: Piece>(mut self, build: impl Fn(&K) -> P + 'static) -> Self {
        self.destination = Some(Rc::new(move |k| AnyPiece::new(build(k))));
        self
    }
    /// Use this selector as a LOCAL widget: its selection is not part of the app route, so it
    /// neither adds a segment to `current_route` nor intercepts `navigate`/deep links.
    ///
    /// Reach for this when a page **already routes** and you embed a *second* one-of-N control in
    /// it (a filter tab strip, a secondary sidebar). Two routing selectors at the same level both
    /// feed `current_route()`, so you'd get `section/childA/childB` and `navigate("childB")` would
    /// be ambiguous — mark all but the primary one `.local()`. A selector nested one level *deeper*
    /// (a `Tabs` inside a `Sidebar` section) is a different case and should stay routed: that
    /// cascade is the point. In debug builds, two routed one-of-N surfaces at the same level log a
    /// warning naming this fix (docs/navigation.md).
    pub fn local(mut self) -> Self {
        self.routed = false;
        self
    }
    /// Remember the selected item across launches (docs/navigation.md). The selected key is saved
    /// under `key` on every change and restored at build — so the app reopens on the tab/section
    /// the user last had — unless a launch deep link is pending, which wins. Restore is a no-op
    /// until the app installs a store (e.g. `day_part_prefs::install_nav_store`); a stale saved
    /// key (its item no longer exists) is ignored. Works whether or not the selector is
    /// [`routed`](Self::local).
    pub fn restore(mut self, key: impl Into<String>) -> Self {
        self.restore = Some(key.into());
        self
    }
    /// Add a trailing action button to the navigation bar, for the toolkits that have no window
    /// toolbar (the phones and HarmonyOS — `Cap::Toolbar` is `Unsupported`): an upper-right bar
    /// button drawn with the bundled `icon` that runs `action` when tapped (docs/navigation.md).
    /// `icon` is a bundled-image name (typed [`ImageName`](day_spec::ImageName), like
    /// [`item_icon`](Self::item_icon)'s); `label` is the button's accessible name and tooltip.
    ///
    /// Desktop split presentations ignore it — they have a real toolbar, so put the same command
    /// there (docs/toolbars.md). The action is app-wide: it rides the current top page's bar, so
    /// the same handler serves every section (read [`current_route`] inside it to act on whatever
    /// is showing).
    pub fn bar_action<M>(
        mut self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.bar_action = Some(BarActionSpec {
            icon: Some(icon.into().as_str().to_owned()),
            label: label.into_text(),
            action: Rc::new(action),
        });
        self
    }

    /// Make this surface searchable, bound two-way to `query` (docs/search.md).
    ///
    /// Search is declared on the SURFACE, not on the toolbar — the same move SwiftUI made with
    /// `.searchable()`. That is what lets the platform choose where to draw the field: the window
    /// toolbar on a wide window, attached to the navigation list on a narrow one, without the app
    /// branching on either. `query` stays app-owned, so the field moving between placements never
    /// moves the state.
    ///
    /// ```ignore
    /// selector(section)
    ///     .style(SelectorStyle::Sidebar)
    ///     .searchable(query)
    ///     .items(move || destinations().filter(|d| matches(d, &query.get())), …)
    /// ```
    pub fn searchable(mut self, query: Signal<String>) -> Self {
        self.search = Some(SearchSpec {
            query,
            prompt: None,
            placement: day_spec::props::SearchPlacement::Automatic,
            scopes: Vec::new(),
            scope: None,
            suggestions: None,
        });
        self
    }

    /// The search field's empty-state prompt. No-op unless [`Selector::searchable`] was called.
    pub fn search_prompt<M>(mut self, prompt: impl IntoText<M>) -> Self {
        if let Some(s) = self.search.as_mut() {
            s.prompt = Some(prompt.into_text());
        }
        self
    }

    /// Ask for a particular placement. A PREFERENCE: a backend that cannot honour it falls back
    /// to its platform's own convention, so `Automatic` (the default) is almost always right.
    pub fn search_placement(mut self, placement: day_spec::props::SearchPlacement) -> Self {
        if let Some(s) = self.search.as_mut() {
            s.placement = placement;
        }
        self
    }

    /// A one-of-N scope bar under the field, bound to `scope` (an index into `titles`).
    ///
    /// Native on UIKit alone; elsewhere it is a real native component doing the same job (a
    /// Material `ChipGroup` of single-selection filter chips, an ArkUI `SegmentButtonV2`, an
    /// `NSSegmentedControl`) or, on web and system XAML, one composed from primitives. Probe
    /// [`day_spec::Cap::SearchScopes`] if the difference matters to the app.
    pub fn search_scopes<M>(mut self, scope: Signal<usize>, titles: Vec<impl IntoText<M>>) -> Self {
        if let Some(s) = self.search.as_mut() {
            s.scopes = titles.into_iter().map(IntoText::into_text).collect();
            s.scope = Some(scope);
        }
        self
    }

    /// Completions for the current text, re-derived whenever a reactive read inside `f` changes.
    ///
    /// On a navigation surface these COMPLETE THE FIELD rather than replacing the list: the list
    /// is already the filtered result set, so an overlay of results would cover the very thing it
    /// is narrowing. Backends whose search widget does completions natively use it
    /// (`AutoSuggestBox`, `QCompleter`, `<datalist>`, `UISearchResultsUpdating`).
    pub fn search_suggestions(mut self, f: impl Fn(&str) -> Vec<String> + 'static) -> Self {
        if let Some(s) = self.search.as_mut() {
            s.suggestions = Some(Rc::new(f));
        }
        self
    }
}

impl<K: Route, S: SignalRw<K>> Piece for Selector<S, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        match self.style {
            SelectorStyle::Tabs => build_tabs(self, cx),
            SelectorStyle::Sidebar => build_sidebar(self, cx),
        }
    }
}

/// Apply a selector's `.restore` at build: seed `selection` from the key saved under `restore`,
/// so the app reopens on the section/tab the user last chose. A pending launch deep link wins
/// (skip). Only a saved key that parses AND is a current item is honored — plus the empty
/// "deselected" key, for a sidebar's collapsed state — so a stale key left by an older build is
/// ignored. A no-op when `restore` is unset or no [`NavStore`](day_core::NavStore) is installed.
fn restore_selection<K: Route, S: SignalRw<K>>(
    restore: &Option<String>,
    selection: &S,
    items: &[K],
) {
    if let Some(key) = restore.as_deref()
        && !day_core::has_launch_deeplink()
        && let Some(saved) = day_core::nav_store_load(key)
        && let Some(k) = K::from_key(&saved)
        && (saved.is_empty() || items.iter().any(|x| x.key() == saved))
    {
        selection.set_rw(k);
    }
}

/// Persist a selector's selection under its `.restore` key on every change (docs/navigation.md).
/// The binding lives in the current scope, so it stops with the surface. Consumes `restore`.
fn persist_selection<K: Route, S: SignalRw<K>>(restore: Option<String>, selection: &S) {
    if let Some(key) = restore {
        let s = selection.clone();
        bind(
            move || s.get_rw().key(),
            move |k: &String| day_core::nav_store_save(&key, k),
        );
    }
}

fn build_tabs<K: Route, S: SignalRw<K>>(sel: Selector<S, K>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{TabsPageProps, TabsPatch, TabsProps};
    let selection = sel.selection;
    let routed = sel.routed;
    let restore = sel.restore;
    let items = Rc::new(SelItems::from_sources(sel.sources, sel.destination));
    let rows0 = day_reactive::untrack(|| items.derive());
    let (typed0, titles0, icons0) = (rows0.keys, rows0.titles, rows0.icons);
    // Restore the last-selected tab (before the initial native index is read).
    restore_selection(&restore, &selection, &typed0);
    let typed: Rc<RefCell<Vec<K>>> = Rc::new(RefCell::new(typed0.clone()));
    let initial = selection.get_untracked_rw().key();
    let initial_idx = typed0.iter().position(|k| k.key() == initial).unwrap_or(0);

    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let host = cx.native(
        kinds::TABS,
        &TabsProps {
            titles: titles0.clone(),
            icons: icons0.clone(),
            selected: initial_idx,
        },
        Rc::new(NavLayout {
            sizes: sizes.clone(),
            split: false,
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        Boundary::Yes,
    );
    // Resident pages (tabs keep every page alive): (key string, page node, scope). A dynamic
    // block reconciles this list against the derived keys.
    let pages: Rc<RefCell<Vec<(String, RNode, Scope)>>> = Rc::default();
    let tab_scope = Scope::current();
    let build_tab_page = {
        let (items_c, pages_c, sizes_c, tab_scope) =
            (items.clone(), pages.clone(), sizes.clone(), tab_scope);
        move |key: &K, title: String, icon: Option<String>| {
            let page = tabs_page(host, &TabsPageProps { title, icon }, &sizes_c);
            let scope = tab_scope.enter(Scope::child);
            let content = items_c.build_page(key);
            scope.enter(|| {
                // Barrier: tabs are resident, not a push stack, so a stack inside a tab keeps
                // its own container (docs/navigation.md).
                with_nav_host(None, || {
                    let mut pcx = BuildCx::new(page);
                    let _ = content.build(&mut pcx);
                });
            });
            pages_c.borrow_mut().push((key.key(), page, scope));
        }
    };
    for (k, t, i) in typed0
        .iter()
        .zip(titles0.iter())
        .zip(icons0.iter())
        .map(|((a, b), c)| (a, b, c))
    {
        build_tab_page(k, t.clone(), i.clone());
    }

    // Two-way: signal → native selection (skip the echo of a native tap).
    let echo: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    {
        let (typed_b, echo, s) = (typed.clone(), echo.clone(), selection.clone());
        bind_seeded(
            initial_idx,
            move || {
                let cur = s.get_rw().key();
                typed_b
                    .borrow()
                    .iter()
                    .position(|k| k.key() == cur)
                    .unwrap_or(0)
            },
            move |idx: &usize| {
                if echo.replace(None) == Some(*idx) {
                    return;
                }
                with_tree(|t| t.patch(host, Box::new(TabsPatch::Selected(*idx)), false));
            },
        );
    }
    // native selection → signal
    {
        let (typed_n, echo, s) = (typed.clone(), echo.clone(), selection.clone());
        cx.on(host, move |ev| match ev {
            Event::SelectionChanged(i) if *i >= 0 => {
                let idx = *i as usize;
                if let Some(k) = typed_n.borrow().get(idx).cloned() {
                    echo.set(Some(idx));
                    // Announce the navigation from its source (docs/navigation.md §14.6): the route
                    // this selection produces changes `NAV_STACK` only after the surface remounts a
                    // frame later, so a route-recording observer would otherwise miss a sidebar move
                    // between two pages. Fire synchronously with the intended key.
                    day_core::note_navigation(&k.key(), None);
                    s.set_rw(k);
                }
            }
            Event::Custom {
                tag: "deeplink",
                text: route,
                ..
            } => {
                let _ = day_core::navigate(route);
            }
            _ => {}
        });
    }
    // Reconcile the resident pages + native tab set when a dynamic block's signal changes
    // (docs/navigation.md) — new keys get a page, gone keys are disposed — and when the
    // locale changes (tracked title resolution): same keys, new titles, one Items re-patch.
    // Installed unconditionally: a fully static selector's derive subscribes to nothing and
    // this effect simply never re-fires.
    {
        let (items_e, typed_e, pages_e, sel_e, build_tab_page) = (
            items.clone(),
            typed.clone(),
            pages.clone(),
            selection.clone(),
            build_tab_page.clone(),
        );
        bind(
            move || {
                let NavRows {
                    keys: k,
                    titles: t,
                    icons: i,
                    badges: b,
                    sections: sc,
                    tints: tn,
                    menus: mn,
                } = items_e.derive();
                (
                    k.iter().map(|x| x.key()).collect::<Vec<_>>(),
                    k,
                    t,
                    i,
                    b,
                    sc,
                    tn,
                    mn,
                )
            },
            // Tabs render neither badges, sections, nor row menus; the derive carries them
            // for the selector's sake, so they are deliberately unused here.
            move |(key_strs, keys, ts, ics, _badges, _sections, _tints, _menus): &DerivedRows<
                K,
            >| {
                // Drop pages whose key vanished (dispose scope + remove subtree).
                pages_e.borrow_mut().retain(|(k, page, scope)| {
                    if key_strs.contains(k) {
                        true
                    } else {
                        scope.dispose();
                        with_tree(|t| t.remove_subtree(*page));
                        false
                    }
                });
                // Append pages for new keys (in derived order — the host reorders via Items).
                let have: std::collections::HashSet<String> =
                    pages_e.borrow().iter().map(|(k, _, _)| k.clone()).collect();
                for ((k, ks), (t, i)) in keys.iter().zip(key_strs).zip(ts.iter().zip(ics)) {
                    if !have.contains(ks) {
                        build_tab_page(k, t.clone(), i.clone());
                    }
                }
                *typed_e.borrow_mut() = keys.clone();
                let cur = sel_e.get_untracked_rw().key();
                let selected = key_strs.iter().position(|k| k == &cur).unwrap_or(0);
                with_tree(|t| {
                    t.patch(
                        host,
                        Box::new(day_spec::props::TabsPatch::Items {
                            titles: ts.clone(),
                            icons: ics.clone(),
                            selected,
                        }),
                        false,
                    );
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            },
        );
    }

    // string-route adapter (the typed key decodes at this boundary; app code stays typed)
    let (tp_push, s_push) = (typed.clone(), selection.clone());
    let s_cur = selection.clone();
    let (tp_enter, s_enter) = (typed.clone(), selection.clone());
    let s_seg = selection.clone();
    let tpick =
        |tp: &Rc<RefCell<Vec<K>>>, k: &str| tp.borrow().iter().find(|x| x.key() == k).cloned();
    if routed {
        note_routed_one_of_n("tabs");
        register_route_surface(
            move |k| {
                if let Some(key) = tpick(&tp_push, k) {
                    s_push.set_rw(key);
                    true
                } else {
                    false
                }
            },
            |_| false,
            move || s_cur.get_untracked_rw().key(),
            // Absolute-path segment: same as push — a tab key is a declared key.
            move |k| {
                if let Some(key) = tpick(&tp_enter, k) {
                    s_enter.set_rw(key);
                    true
                } else {
                    false
                }
            },
            move || {
                let k = s_seg.get_untracked_rw().key();
                if k.is_empty() { Vec::new() } else { vec![k] }
            },
        );
    }
    persist_selection(restore, &selection);
    host
}

fn build_sidebar<K: Route, S: SignalRw<K>>(sel: Selector<S, K>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{NavMenuPatch, NavMenuProps, NavPageProps, NavPatch, NavProps};
    let split = with_tree(|t| t.capability(day_spec::Cap::NavSplit)) == day_spec::Support::Native;
    let selection = sel.selection;
    let routed = sel.routed;
    let restore = sel.restore;
    let title_s = sel.title.initial();
    // Register the optional nav-bar action once (getting its dispatch id) and lower it into the
    // host props — the mobile backends draw it as an upper-right bar button (docs/navigation.md).
    let bar_action = sel.bar_action.map(BarActionSpec::lower);
    // Search over this surface (docs/search.md). Lowered to the host's props with its CURRENT
    // values; the live bindings below keep the field in step through targeted patches, so the
    // app writing the query never rebuilds (and refocuses) the field mid-word.
    let search_spec = sel.search;
    // Lowered ONCE: the bindings below need these same values as their seeds, and `lower` runs
    // the app's suggestion closure, which should not run twice per build.
    let search_seed = search_spec.as_ref().map(SearchSpec::lower);
    let search = search_seed.clone();
    let items = Rc::new(SelItems::from_sources(sel.sources, sel.destination));
    // The live row set is reactive: `typed` (index → key) and `titles` are shared mutable state
    // the derive effect updates; the initial derive is untracked (the effect below owns the
    // subscription).
    let rows0 = day_reactive::untrack(|| items.derive());
    let (typed0, titles0, icons0) = (rows0.keys, rows0.titles, rows0.icons);
    // Restore the last-selected section (before the detail's initial `show` runs).
    restore_selection(&restore, &selection, &typed0);
    let typed: Rc<RefCell<Vec<K>>> = Rc::new(RefCell::new(typed0));
    let titles: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(titles0));

    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let host = cx.native(
        kinds::NAV,
        &NavProps {
            title: title_s.clone(),
            split,
            bar_action,
            search,
        },
        Rc::new(NavLayout {
            sizes: sizes.clone(),
            split,
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        Boundary::Yes,
    );

    // Search, both directions (docs/search.md). The app's writes patch the live field; the user's
    // edits arrive as events against this host and write the app's own signals. Nothing here
    // touches the widget directly, which is what lets a later placement change relocate the field
    // without disturbing the query.
    if let Some((spec, seed)) = search_spec.as_ref().zip(search_seed.as_ref()) {
        spec.bind(host, seed);
        let query = spec.query;
        let scope_sig = spec.scope;
        cx.on(host, move |ev| match ev {
            Event::SearchChanged(text) => query.set(text.clone()),
            Event::SearchScopeChanged(i) => {
                if let Some(s) = scope_sig {
                    s.set(*i);
                }
            }
            // Activation feeds `day::is_searching()` and the suggestion choice already arrived as
            // a SearchChanged, so neither writes an app signal here.
            Event::SearchActiveChanged(on) => day_core::set_searching(*on),
            _ => {}
        });
    }

    // The per-host back-owner stack (docs/navigation.md): the detail page pushes its "deselect"
    // owner, and a nested stack that merges into this host pushes its page owners on top. The
    // context is threaded to nested pieces built under our pages.
    let owners: Rc<RefCell<Vec<PopOwner>>> = Rc::default();
    let host_cx = NavHostCx {
        host,
        sizes: sizes.clone(),
        owners: owners.clone(),
        split,
    };

    // Sidebar / root page: optional header + native item list.
    let root_page = nav_page(
        host,
        &NavPageProps {
            title: title_s.clone(),
            sidebar: split,
        },
        &sizes,
    );
    let menu_holder: Rc<Cell<Option<RNode>>> = Rc::new(Cell::new(None));
    {
        let (mh, ks, s, ts) = (
            menu_holder.clone(),
            typed.clone(),
            selection.clone(),
            titles.clone(),
        );
        let (titles_init, icons_init) = (titles.borrow().clone(), icons0.clone());
        let (badges_init, sections_init) = (rows0.badges.clone(), rows0.sections.clone());
        let tints_init = rows0.tints.clone();
        let menus_init = rows0.menus.clone();
        let menu_piece = piece_fn(move |mcx| {
            let node = mcx.native(
                kinds::NAV_MENU,
                &NavMenuProps {
                    items: titles_init,
                    icons: icons_init,
                    badges: badges_init,
                    sections: sections_init,
                    tints: tints_init,
                    menus: menus_init,
                    selected: None,
                },
                Rc::new(LeafLayout),
                Flex {
                    grow_w: true,
                    grow_h: true,
                    ..Default::default()
                },
                Boundary::No,
            );
            mh.set(Some(node));
            mcx.on(node, move |ev| {
                if let Event::SelectionChanged(i) = ev
                    && let Some(k) = ks.borrow().get(*i as usize)
                {
                    // Announce the navigation from its source (§14.6) with the row's own title —
                    // the sidebar changes the route only after a remount, so a route observer would
                    // otherwise miss the move AND have no label for it. Index the LIVE titles.
                    let label = ts.borrow().get(*i as usize).cloned();
                    day_core::note_navigation(&k.key(), label.as_deref());
                    s.set_rw(k.clone());
                }
            });
            node
        });
        let content: AnyPiece = match sel.header {
            Some(h) => column((h(), menu_piece))
                .spacing(4.0)
                .align(HAlign::Leading)
                .any(),
            None => column((menu_piece,))
                .spacing(4.0)
                .align(HAlign::Leading)
                .any(),
        };
        with_nav_host(Some(host_cx.clone()), || {
            let mut pcx = BuildCx::new(root_page);
            let _ = content.build(&mut pcx);
        });
    }

    let sync_menu = {
        let mh = menu_holder.clone();
        move |idx: Option<usize>| {
            if let Some(m) = mh.get() {
                with_tree(|t| t.patch(m, Box::new(NavMenuPatch::Selected(idx)), false));
            }
        }
    };

    // Detail: `selection` drives which item's page is shown (reset-to; depth ≤ 1).
    let current: Rc<RefCell<Option<(String, Scope, RNode)>>> = Rc::default();
    let nav_scope = Scope::current();
    // Shared: BOTH the selection bind and the row-derive effect drive the detail (see the
    // derive effect for why the selection bind alone is not enough).
    let show = std::rc::Rc::new({
        let (items, current, sizes, typed_s, titles_s, sync_menu, owners, host_cx, selection) = (
            items.clone(),
            current.clone(),
            sizes.clone(),
            typed.clone(),
            titles.clone(),
            sync_menu.clone(),
            owners.clone(),
            host_cx.clone(),
            selection.clone(),
        );
        move |key: &str| {
            if current.borrow().as_ref().map(|(k, _, _)| k.as_str()) == Some(key) {
                return;
            }
            if let Some((_, scope, page)) = current.borrow_mut().take() {
                // Dispose the detail scope FIRST: a merged inner stack's cleanup pops its pages
                // (which sit on top natively) before we pop the detail itself, so the native pop
                // order stays top-down (iOS pops the topmost VC; Android's INCLUSIVE pop unwinds
                // everything above an entry).
                scope.dispose();
                with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                owners.borrow_mut().pop();
                sizes.borrow_mut().remove(&page);
                with_tree(|t| {
                    t.remove_subtree(page);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
            if key.is_empty() {
                sync_menu(None);
                return;
            }
            let idx = typed_s.borrow().iter().position(|k| k.key() == key);
            let Some(idx) = idx else {
                sync_menu(None);
                return;
            };
            let typed_key = typed_s.borrow()[idx].clone();
            let title_now = titles_s.borrow()[idx].clone();
            // A static item retitles on locale change (its TextSource); a data-driven key uses
            // the resolved snapshot (its title tracks the items signal, not the locale).
            let retitle = items.static_title(key);
            let page = nav_page(
                host,
                &NavPageProps {
                    title: title_now.clone(),
                    sidebar: false,
                },
                &sizes,
            );
            // The detail page's back action = deselect (return to the list). Pushed BEFORE the
            // content builds, so a merged inner stack's page owners stack on top of it.
            let owner: PopOwner = {
                let s = selection.clone();
                Rc::new(move |_already_popped| {
                    if let Some(root) = K::from_key("") {
                        s.set_rw(root);
                    }
                })
            };
            owners.borrow_mut().push(owner);
            let scope = nav_scope.enter(Scope::child);
            let content = items.build_page(&typed_key);
            scope.enter(|| {
                with_nav_host(Some(host_cx.clone()), || {
                    let mut c = BuildCx::new(page);
                    let _ = content.build(&mut c);
                });
            });
            with_tree(|t| {
                t.patch(
                    host,
                    Box::new(NavPatch::Pushed {
                        title: title_now,
                        immersive: items.immersive_of(&typed_key.key()),
                    }),
                    false,
                );
                t.mark_layout_dirty();
                t.layout_if_needed();
            });
            // Live retitle for a static item: its title SOURCE re-resolves on locale change and
            // the host's native bar follows via `NavPatch::Title`. Scope-owned, dies with the page.
            if let Some(rt) = retitle {
                scope.enter(|| {
                    rt.bind_to(host, |t| Box::new(NavPatch::Title(t)), false);
                });
            }
            *current.borrow_mut() = Some((key.to_string(), scope, page));
            sync_menu(typed_s.borrow().iter().position(|k| k.key() == key));
        }
    });

    // Desktop split never shows an empty detail: default to the first item.
    if split
        && selection.get_untracked_rw().key().is_empty()
        && let Some(k) = typed.borrow().first().cloned()
    {
        selection.set_rw(k);
    }
    {
        let (s, show) = (selection.clone(), show.clone());
        bind(move || s.get_rw().key(), move |key: &String| show(key));
    }

    // Re-derive the row set when a dynamic block's signal changes (re-patch the native menu,
    // reset the selection if its item vanished) and when the locale changes (tracked title
    // resolution — same keys, new titles). Installed unconditionally: a fully static
    // selector's derive subscribes to nothing and this effect simply never re-fires.
    {
        let (items_e, typed_e, titles_e, mh_e, sel_e) = (
            items.clone(),
            typed.clone(),
            titles.clone(),
            menu_holder.clone(),
            selection.clone(),
        );
        let (show_e, split_e) = (show.clone(), split);
        bind(
            move || {
                // TRACKED derive: subscribes to every dynamic block's signal.
                let NavRows {
                    keys: k,
                    titles: t,
                    icons: i,
                    badges: b,
                    sections: sc,
                    tints: tn,
                    menus: mn,
                } = items_e.derive();
                (
                    k.iter().map(|x| x.key()).collect::<Vec<_>>(),
                    k,
                    t,
                    i,
                    b,
                    sc,
                    tn,
                    mn,
                )
            },
            move |(key_strs, keys, ts, ics, bs, scs, tns, mns): &DerivedRows<K>| {
                *typed_e.borrow_mut() = keys.clone();
                *titles_e.borrow_mut() = ts.clone();
                // If the selected key is gone, reset (Option key → None); else keep it selected.
                let cur = sel_e.get_untracked_rw().key();
                let still = cur.is_empty() || key_strs.iter().any(|k| k == &cur);
                if !still && let Some(root) = K::from_key("") {
                    sel_e.set_rw(root);
                }
                // A split view must never show an empty detail. The build-time fallback ran
                // once, before any filtering, so re-apply it here: when the selected row is
                // gone, move to the first row that survived rather than blanking the pane.
                let mut cur2 = sel_e.get_untracked_rw().key();
                if split_e
                    && cur2.is_empty()
                    && let Some(k) = keys.first().cloned()
                {
                    sel_e.set_rw(k.clone());
                    cur2 = k.key();
                }
                let selected = key_strs.iter().position(|k| k == &cur2);
                if let Some(m) = mh_e.get() {
                    with_tree(|t| {
                        t.patch(
                            m,
                            Box::new(day_spec::props::NavMenuPatch::Items {
                                items: ts.clone(),
                                icons: ics.clone(),
                                badges: bs.clone(),
                                sections: scs.clone(),
                                tints: tns.clone(),
                                menus: mns.clone(),
                                selected,
                            }),
                            false,
                        );
                    });
                }
                // Drive the detail from HERE as well as from the selection bind. That bind is
                // created FIRST, so when a query signal and the selection are written in one
                // batch it runs while `typed` still holds the pre-filter rows, finds no index
                // for the key, and gives up — leaving a highlighted row over an empty pane with
                // nothing left to re-trigger it. The fallback just above has the same problem in
                // reverse: it changes the selection after that bind has already run. `show` is
                // idempotent (it returns at once when the detail already shows this key), so
                // calling it on every derive costs nothing and closes both holes.
                show_e(&cur2);
            },
        );
    }

    // Native back (mobile up-arrow / system back) → the topmost page's owner. With only this
    // sidebar on the host, that's always the detail's deselect owner (returns to the list); when
    // a nested stack has merged its pages on top, its owners run first (docs/navigation.md). A
    // typed key deselects via its "" decoding (`Option<Section>` → `None`); a bare enum has no
    // list-only state so its owner's deselect is a no-op — back is effectively ignored.
    {
        let owners = owners.clone();
        cx.on(host, move |ev| match ev {
            Event::NavBack { already_popped } => {
                let top = owners.borrow().last().cloned();
                if let Some(f) = top {
                    f(*already_popped);
                }
            }
            Event::Custom {
                tag: "deeplink",
                text: route,
                ..
            } => {
                let _ = day_core::navigate(route);
            }
            _ => {}
        });
    }

    // string-route adapter over `selection` (typed keys decode at this boundary). The live
    // `typed` set is consulted per call, so a data-driven key routes as soon as its item exists.
    let (tp_push, s_push) = (typed.clone(), selection.clone());
    let s_pop = selection.clone();
    let s_cur = selection.clone();
    let (tp_enter, s_enter) = (typed.clone(), selection.clone());
    let s_seg = selection.clone();
    let pick =
        |tp: &Rc<RefCell<Vec<K>>>, k: &str| tp.borrow().iter().find(|x| x.key() == k).cloned();
    let pick_push = pick;
    let pick_enter = pick;
    if routed {
        note_routed_one_of_n("sidebar");
        register_route_surface(
            move |k| {
                if k.is_empty() {
                    if let Some(root) = K::from_key("") {
                        s_push.set_rw(root);
                        true
                    } else {
                        false // no empty state (bare-enum key) — let the parent handle ""
                    }
                } else if let Some(key) = pick_push(&tp_push, k) {
                    s_push.set_rw(key);
                    true
                } else {
                    false
                }
            },
            move |_| {
                if s_pop.get_untracked_rw().key().is_empty() {
                    false
                } else if let Some(root) = K::from_key("") {
                    s_pop.set_rw(root);
                    true
                } else {
                    false
                }
            },
            move || s_cur.get_untracked_rw().key(),
            // Absolute-path segment: a declared item key selects it (no "" — segments are non-empty).
            move |k| {
                if let Some(key) = pick_enter(&tp_enter, k) {
                    s_enter.set_rw(key);
                    true
                } else {
                    false
                }
            },
            move || {
                let k = s_seg.get_untracked_rw().key();
                if k.is_empty() { Vec::new() } else { vec![k] }
            },
        );
    }
    persist_selection(restore, &selection);
    host
}

// ===========================================================================
// Stack — a genuine push/pop navigation stack bound to a Signal<Vec<String>>.
// The native UINavigationController / AdwNavigationView / back-stack is reconciled
// to the path; the back button writes the pop back into the path.
// ===========================================================================

struct StackEntry<K> {
    key: K,
    scope: Scope,
    page: RNode,
}

/// What a [`Stack::on_back`] guard returns for one back-like event (a native back gesture/button,
/// or [`nav_back`]). Programmatic path writes are NOT guarded — the guard is a policy on the
/// user's back affordance, matching Jetpack Compose's `BackHandler` (docs/navigation.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BackResponse {
    /// Let the pop happen now.
    Proceed,
    /// Consume the back — the pop does NOT happen. Stash the [`BackRequest`] and call
    /// [`BackRequest::proceed`] later (e.g. after a confirmation dialog) to perform the pop.
    Handled,
}

/// The deferred pop handed to a [`Stack::on_back`] guard. Hold it, then call [`proceed`] to
/// perform the back the guard consumed (the unsaved-changes → confirm → leave flow).
///
/// [`proceed`]: BackRequest::proceed
#[derive(Clone)]
pub struct BackRequest {
    pop: Rc<dyn Fn()>,
}

impl BackRequest {
    /// Perform the pop this back requested. Runs the same path pop a `Proceed` would have; safe
    /// to call once (a second call after the page is gone is a no-op).
    pub fn proceed(&self) {
        (self.pop)();
    }
}

/// A push/pop navigation stack whose contents are an app-owned `Signal<Vec<K>>` (the path
/// above the root). Day reconciles the native stack to the path; the native back button
/// writes the pop back into it (docs/navigation.md).
///
/// The key type is any [`Route`]: `String` for raw keys, or a typed enum whose variants can
/// carry data — the destination builder then receives the typed value, and an absolute
/// `navigate("…/item-42")` parses each segment via [`Route::from_key`] (rejecting segments
/// that don't parse; `String` accepts everything).
///
/// ```ignore
/// let path = Signal::new(Vec::<Drill>::new());
/// stack(path.clone(), home_view).destination(|d: &Drill| detail_view(d))
/// // push:  path.update(|p| p.push(Drill::Item { id: 42 }));
/// ```
pub struct Stack<S: SignalRw<Vec<K>>, K: Route = String> {
    path: S,
    title: TextSource,
    root: AnyPiece,
    destination: Rc<dyn Fn(&K) -> AnyPiece>,
    on_back: Option<Rc<dyn Fn(BackRequest) -> BackResponse>>,
    /// The persistence key set by [`Stack::restore`]: the path is saved here (its keys `/`-joined)
    /// on every change and restored at build. `None` = not persisted.
    restore: Option<String>,
    /// An optional trailing nav-bar action ([`Stack::bar_action`]) — the mobile stand-in for a
    /// desktop toolbar button. `None` unless set.
    bar_action: Option<BarActionSpec>,
}

pub fn stack<K: Route, S: SignalRw<Vec<K>>>(path: S, root: impl Piece) -> Stack<S, K> {
    Stack {
        path,
        title: TextSource::Static(String::new()),
        root: AnyPiece::new(root),
        destination: Rc::new(|_| {
            piece_fn(|cx| cx.layout_only(Rc::new(PassThrough), Flex::default(), Boundary::No))
        }),
        on_back: None,
        restore: None,
        bar_action: None,
    }
}

impl<K: Route, S: SignalRw<Vec<K>>> Stack<S, K> {
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text();
        self
    }
    /// Add a trailing action button to the navigation bar, for the toolkits with no window toolbar
    /// (the phones and HarmonyOS): an upper-right bar button drawn with the bundled `icon` that
    /// runs `action` (docs/navigation.md). Mirrors [`Selector::bar_action`]; ignored on desktop.
    pub fn bar_action<M>(
        mut self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.bar_action = Some(BarActionSpec {
            icon: Some(icon.into().as_str().to_owned()),
            label: label.into_text(),
            action: Rc::new(action),
        });
        self
    }
    /// Build the view for a pushed key (`&String` for raw keys, the typed value otherwise).
    pub fn destination<P: Piece>(mut self, build: impl Fn(&K) -> P + 'static) -> Self {
        self.destination = Rc::new(move |k| AnyPiece::new(build(k)));
        self
    }
    /// Intercept the back affordance (docs/navigation.md). The guard runs for every native back
    /// gesture/button and [`nav_back`] while the stack is above its root; it returns
    /// [`BackResponse::Proceed`] to pop now or [`BackResponse::Handled`] to consume the back
    /// (stash the [`BackRequest`] and call [`BackRequest::proceed`] later). Programmatic path
    /// writes are never guarded. While a guard is armed the toolkit stops auto-popping on a
    /// native gesture and routes the back through Day instead (`NavPatch::GuardTop`).
    pub fn on_back(mut self, guard: impl Fn(BackRequest) -> BackResponse + 'static) -> Self {
        self.on_back = Some(Rc::new(guard));
        self
    }
    /// Remember the pushed path across launches (docs/navigation.md). On every change the path's
    /// keys are `/`-joined and saved under `key`; at build the saved path is parsed back (each
    /// segment via [`Route::from_key`]) and restored — so the app reopens exactly where the user
    /// left off, including after an Android process death — unless a launch deep link is pending,
    /// which wins. Restore is a no-op until the app installs a store (e.g.
    /// `day_part_prefs::install_nav_store`); a saved path with a segment that no longer parses is
    /// ignored whole.
    pub fn restore(mut self, key: impl Into<String>) -> Self {
        self.restore = Some(key.into());
        self
    }
}

impl<K: Route, S: SignalRw<Vec<K>>> Piece for Stack<S, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::props::{NavPageProps, NavPatch, NavProps};
        let Stack {
            path,
            title,
            root,
            destination: dest,
            on_back,
            restore,
            bar_action,
        } = self;
        let title_s = title.initial();
        // Lower the optional nav-bar action for the standalone host below (a merged stack rides
        // the enclosing host's bar instead). Registered once; the mobile backends draw it.
        let bar_action = bar_action.map(BarActionSpec::lower);

        // Restore the saved path before the reconcile binding runs, so its pages build on first
        // pass. A launch deep link wins (skip). The path is decoded from the SAME percent-encoded
        // wire format the rest of nav uses (`parse_route`), so a key that itself contains `/` (or
        // `?`, `%`) round-trips instead of splitting into two. The whole path is parsed via
        // `Route::from_key`; any segment that no longer parses discards the restore rather than
        // building a partial stack (docs/navigation.md).
        if let Some(key) = restore.as_deref()
            && !day_core::has_launch_deeplink()
            && let Some(saved) = day_core::nav_store_load(key)
        {
            let (segments, _) = day_core::parse_route(&saved);
            let parsed: Option<Vec<K>> = segments.iter().map(|s| K::from_key(s)).collect();
            if let Some(v) = parsed {
                path.set_rw(v);
            }
        }

        // If we're built inside a page of an enclosing NAV host that presents as a push stack
        // (mobile, `split == false`), MERGE: push our pages onto that host instead of minting a
        // second native container — one native nav chain, one back button (docs/navigation.md).
        // A split host (desktop) is not merged into; a stack keeps its own detail-pane stack.
        let merge = current_nav_host().filter(|c| !c.split);

        let entries: Rc<RefCell<Vec<StackEntry<K>>>> = Rc::default();
        let native_popped: Rc<Cell<usize>> = Rc::new(Cell::new(0));

        let host: RNode;
        let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>>;
        let owners: Rc<RefCell<Vec<PopOwner>>>;
        let host_cx: NavHostCx;
        let ret_node: RNode;
        let merged: bool;
        if let Some(ctx) = merge {
            // MERGED: reuse the enclosing host; our root renders inline in the current page (which
            // is already a NAV_PAGE), and only our pushed destinations become new pages.
            host = ctx.host;
            sizes = ctx.sizes.clone();
            owners = ctx.owners.clone();
            host_cx = ctx;
            let hc = host_cx.clone();
            ret_node = with_nav_host(Some(hc), || root.build(cx));
            merged = true;
        } else {
            // STANDALONE: create the native host + root page (an app-root stack, or a nested stack
            // under a split/desktop host).
            sizes = Rc::default();
            host = cx.native(
                kinds::NAV,
                &NavProps {
                    title: title_s.clone(),
                    split: false, // a stack is a stack (no sidebar)
                    bar_action,
                    // Stacks are not searchable yet — `.searchable()` is on `Selector` only
                    // (docs/search.md); a stack gains the same surface when the placement
                    // resolver lands, since it is the same lowering.
                    search: None,
                },
                Rc::new(NavLayout {
                    sizes: sizes.clone(),
                    split: false,
                }),
                Flex {
                    grow_w: true,
                    grow_h: true,
                    ..Default::default()
                },
                Boundary::Yes,
            );
            owners = Rc::default();
            host_cx = NavHostCx {
                host,
                sizes: sizes.clone(),
                owners: owners.clone(),
                split: false,
            };
            let root_page = nav_page(
                host,
                &NavPageProps {
                    title: title_s,
                    sidebar: false,
                },
                &sizes,
            );
            let hc = host_cx.clone();
            with_nav_host(Some(hc), || {
                let mut pcx = BuildCx::new(root_page);
                let _ = root.build(&mut pcx);
            });
            ret_node = host;
            merged = false;
        }

        let nav_scope = Scope::current();

        // The RAW pop: drop the top path segment (reconcile then pops the native page). The
        // guard's `BackRequest::proceed` runs exactly this.
        let raw_pop: Rc<dyn Fn()> = {
            let p = path.clone();
            Rc::new(move || {
                let mut v = p.get_untracked_rw();
                if v.pop().is_some() {
                    p.set_rw(v);
                }
            })
        };
        let depth = {
            let p = path.clone();
            move || p.get_untracked_rw().len()
        };
        // One back-like event (native gesture that Day owns, or `nav_back()`): consult the guard
        // if one is armed and we're above the root, else pop. `Proceed` pops now; `Handled`
        // consumes it (the app holds the `BackRequest`). Programmatic `path.set` never lands here.
        let run_back: Rc<dyn Fn()> = {
            let (raw_pop, depth, guard) = (raw_pop.clone(), depth.clone(), on_back.clone());
            Rc::new(move || {
                if depth() == 0 {
                    return; // at root — nothing of ours to pop
                }
                match &guard {
                    Some(g) => {
                        let req = BackRequest {
                            pop: raw_pop.clone(),
                        };
                        if g(req) == BackResponse::Proceed {
                            raw_pop();
                        }
                    }
                    None => raw_pop(),
                }
            })
        };

        // This stack's back owner (one Rc shared by all its pages): a native pop the toolkit
        // ALREADY performed (an unguarded iOS swipe) is absorbed + synced without the guard; a
        // back Day owns runs `run_back` so the guard decides.
        let stack_owner: PopOwner = {
            let (native_popped, raw_pop, run_back) =
                (native_popped.clone(), raw_pop.clone(), run_back.clone());
            Rc::new(move |already_popped: bool| {
                if already_popped {
                    native_popped.set(native_popped.get() + 1);
                    raw_pop();
                } else {
                    run_back();
                }
            })
        };

        // Reconcile the native stack to `want`: keep the common prefix, pop the rest, push
        // the new suffix. A pop the native already performed (iOS back) is not re-issued. Pages
        // and owners land on `host` (our own, or the enclosing one when merged).
        // `true` once GuardTop(true) has been sent for the current depth, so we only re-emit on
        // a real transition (arming/disarming native gesture handling is not free on every pop).
        let guard_armed_sent = Rc::new(Cell::new(false));
        let has_guard = on_back.is_some();
        let reconcile = {
            let (entries, sizes, dest, native_popped, owners, host_cx, stack_owner) = (
                entries.clone(),
                sizes.clone(),
                dest.clone(),
                native_popped.clone(),
                owners.clone(),
                host_cx.clone(),
                stack_owner.clone(),
            );
            let guard_armed_sent = guard_armed_sent.clone();
            move |want: &Vec<K>| {
                let common = {
                    let ents = entries.borrow();
                    let mut i = 0;
                    while i < ents.len() && i < want.len() && ents[i].key == want[i] {
                        i += 1;
                    }
                    i
                };
                while entries.borrow().len() > common {
                    let e = entries.borrow_mut().pop().unwrap();
                    if native_popped.get() > 0 {
                        native_popped.set(native_popped.get() - 1);
                    } else {
                        with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                    }
                    e.scope.dispose();
                    sizes.borrow_mut().remove(&e.page);
                    with_tree(|t| t.remove_subtree(e.page));
                    owners.borrow_mut().pop();
                }
                for key in want.iter().skip(common) {
                    let title = key.title();
                    let page = nav_page(
                        host,
                        &NavPageProps {
                            title: title.clone(),
                            sidebar: false,
                        },
                        &sizes,
                    );
                    let scope = nav_scope.enter(Scope::child);
                    let content = (dest)(key);
                    let hc = host_cx.clone();
                    scope.enter(|| {
                        with_nav_host(Some(hc), || {
                            let mut c = BuildCx::new(page);
                            let _ = content.build(&mut c);
                        });
                    });
                    with_tree(|t| {
                        // Stack destinations are standard chrome in v1; the immersive flag is a
                        // selector-item concept today (docs/navigation.md).
                        t.patch(
                            host,
                            Box::new(NavPatch::Pushed {
                                title,
                                immersive: false,
                            }),
                            false,
                        )
                    });
                    owners.borrow_mut().push(stack_owner.clone());
                    entries.borrow_mut().push(StackEntry {
                        key: key.clone(),
                        scope,
                        page,
                    });
                }
                // Arm/disarm native gesture handling when the guarded-above-root state changes
                // (docs/navigation.md). The host is our own container, or the enclosing one when
                // merged — either way the native nav that owns the back gesture.
                if has_guard {
                    let armed = !entries.borrow().is_empty();
                    if armed != guard_armed_sent.get() {
                        guard_armed_sent.set(armed);
                        with_tree(|t| t.patch(host, Box::new(NavPatch::GuardTop(armed)), false));
                    }
                }
                with_tree(|t| {
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        };
        {
            let p = path.clone();
            bind(move || p.get_rw(), move |want: &Vec<K>| reconcile(want));
        }

        // Persist the path across launches when `.restore` is set: save the keys in the same
        // percent-encoded wire format `parse_route` reads back (`encode_route`), so a key
        // containing `/` survives the round-trip (docs/navigation.md). Scope-owned, so it stops
        // with the stack.
        if let Some(key) = restore {
            let p = path.clone();
            bind(
                move || {
                    let keys: Vec<String> = p.get_rw().iter().map(|k| k.key()).collect();
                    day_core::encode_route(&keys, &[])
                },
                move |s: &String| day_core::nav_store_save(&key, s),
            );
        }

        // Standalone: own the host's single NavBack dispatcher (→ topmost page's owner) and the
        // deeplink handler. Merged: the enclosing host's creator already owns both.
        if !merged {
            let owners_h = owners.clone();
            cx.on(host, move |ev| match ev {
                Event::NavBack { already_popped } => {
                    let top = owners_h.borrow().last().cloned();
                    if let Some(f) = top {
                        f(*already_popped);
                    }
                }
                Event::Custom {
                    tag: "deeplink",
                    text: route,
                    ..
                } => {
                    let _ = day_core::navigate(route);
                }
                _ => {}
            });
        }

        // Merged: our pages live on the enclosing host, so the enclosing detail's
        // `remove_subtree` won't reach them — pop every remaining page (top-down) off that host
        // when our scope disposes (e.g. the section switches). Guarded for app teardown.
        if merged {
            let (entries_c, sizes_c, owners_c, native_popped_c) = (
                entries.clone(),
                sizes.clone(),
                owners.clone(),
                native_popped.clone(),
            );
            nav_scope.on_cleanup(move || {
                let alive = with_tree(|t| t.node_kind(host).is_some());
                loop {
                    let e = entries_c.borrow_mut().pop();
                    let Some(e) = e else { break };
                    if alive {
                        if native_popped_c.get() > 0 {
                            native_popped_c.set(native_popped_c.get() - 1);
                        } else {
                            with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                        }
                        sizes_c.borrow_mut().remove(&e.page);
                        with_tree(|t| t.remove_subtree(e.page));
                        owners_c.borrow_mut().pop();
                    }
                }
                if alive {
                    with_tree(|t| {
                        t.mark_layout_dirty();
                        t.layout_if_needed();
                    });
                }
            });
        }

        // string-route adapter. A stack is driven by its `path` (app state / buttons), not by
        // magic navigate-strings: a RELATIVE `navigate("<key>")` claims only "" (pop to root),
        // so sibling keys fall through to the enclosing surface — but an ABSOLUTE path's
        // segments (`enter`) push any segment the key type parses: a `String` stack is
        // open-ended, a typed stack validates via `Route::from_key`, and an explicit `a/b/c`
        // path IS the stack's state. `pop` falls through once empty.
        let p_push = path.clone();
        let p_cur = path.clone();
        let p_enter = path.clone();
        let p_seg = path.clone();
        register_route_surface(
            move |k| {
                if k.is_empty() {
                    let mut v = p_push.get_untracked_rw();
                    if v.is_empty() {
                        return false; // already at root — let the parent handle ""
                    }
                    v.clear();
                    p_push.set_rw(v);
                    true
                } else {
                    false
                }
            },
            {
                // `nav_back()` is a back-like event, so it is GUARDED too (docs/navigation.md):
                // run_back consults the guard. We "own" the back (return true, no fall-through)
                // whenever we're above the root, whether the guard pops or consumes.
                let (run_back, depth) = (run_back.clone(), depth.clone());
                move |_| {
                    if depth() == 0 {
                        return false; // at root — let the parent handle back
                    }
                    run_back();
                    true
                }
            },
            move || {
                p_cur
                    .get_untracked_rw()
                    .last()
                    .map(|k| k.key())
                    .unwrap_or_default()
            },
            move |k| {
                let Some(parsed) = K::from_key(k) else {
                    return false; // not one of this stack's routes — leave it queued
                };
                let mut v = p_enter.get_untracked_rw();
                v.push(parsed);
                p_enter.set_rw(v);
                true
            },
            move || p_seg.get_untracked_rw().iter().map(|k| k.key()).collect(),
        );
        ret_node
    }
}

// ===========================================================================
// Cover — a fullscreen modal surface bound to a Signal<Option<Route>> (docs/cover.md).
// ===========================================================================

/// A fullscreen cover: the modal counterpart of [`stack`], bound to a `Signal<Option<R>>`.
/// `Some(r)` presents the built content over the whole window (edge-to-edge, slide-up where
/// the platform animates modals); `None` dismisses it. The SwiftUI analogue is
/// `fullScreenCover(item:)`. Build one with [`cover`].
///
/// The open value is app state, exactly like a stack's path: set it and the cover presents;
/// a native dismissal (Android system back) writes `None` back — unless an
/// [`interactive_dismiss_disabled`](Decorate::interactive_dismiss_disabled) subtree is
/// mounted inside the content, in which case only programmatic writes close it.
/// A cover's per-route surface color (see [`Cover::background`]).
type CoverBackground<R> = Rc<dyn Fn(&R) -> day_spec::Color>;

pub struct Cover<S, R: Route> {
    open: S,
    build: Rc<dyn Fn(&R) -> AnyPiece>,
    background: Option<CoverBackground<R>>,
    _marker: std::marker::PhantomData<R>,
}

/// A fullscreen cover over `open`: `Some(r)` presents `build(&r)`, `None` dismisses
/// (docs/cover.md). Registers a string-route adapter, so `navigate("<key>")` opens it and
/// `nav_back()` closes it, and `current_route()` reports the presented key.
pub fn cover<R: Route, S: SignalRw<Option<R>>>(
    open: S,
    build: impl Fn(&R) -> AnyPiece + 'static,
) -> Cover<S, R> {
    Cover {
        open,
        build: Rc::new(build),
        background: None,
        _marker: std::marker::PhantomData,
    }
}

impl<S: SignalRw<Option<R>>, R: Route> Cover<S, R> {
    /// The surface color painted edge-to-edge behind the content (under the status bar and
    /// home indicator) while `r` is presented. Without it the platform's default surface
    /// color shows in the unsafe areas.
    pub fn background(mut self, f: impl Fn(&R) -> day_spec::Color + 'static) -> Self {
        self.background = Some(Rc::new(f));
        self
    }
}

impl<S: SignalRw<Option<R>>, R: Route> Piece for Cover<S, R> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::props::{CoverPatch, CoverProps};
        let Cover {
            open,
            build,
            background,
            ..
        } = self;

        let size: Rc<RefCell<Option<Size>>> = Rc::default();
        let node = cx.native(
            kinds::COVER,
            &CoverProps::default(),
            Rc::new(day_core::CoverLayout { size: size.clone() }),
            Flex::default(),
            Boundary::Yes,
        );

        // The presented content's scope, and whether a dismiss transition is in flight
        // (content stays mounted until the backend reports "cover-hidden", so the surface
        // isn't blank while it slides out).
        struct Presented<R> {
            key: R,
            scope: Scope,
        }
        let current: Rc<RefCell<Option<Presented<R>>>> = Rc::default();
        let closing: Rc<Cell<bool>> = Rc::default();
        let owner_scope = Scope::current();

        let dispose_content = {
            let current = current.clone();
            move || {
                if let Some(p) = current.borrow_mut().take() {
                    p.scope.dispose();
                }
                while with_tree(|t| t.child_count(node)) > 0 {
                    match with_tree(|t| t.first_child(node)) {
                        Some(c) => with_tree(|t| t.remove_subtree(c)),
                        None => break,
                    }
                }
            }
        };

        // Reconcile the presented surface to the signal.
        let reconcile = {
            let (current, closing, dispose_content) =
                (current.clone(), closing.clone(), dispose_content.clone());
            move |want: &Option<R>| match want {
                Some(r) => {
                    let already =
                        !closing.get() && current.borrow().as_ref().is_some_and(|p| p.key == *r);
                    if already {
                        return;
                    }
                    dispose_content();
                    closing.set(false);
                    let scope = owner_scope.enter(Scope::child);
                    // Run the app's builder INSIDE the presentation scope: side effects it
                    // performs eagerly (state restore, autosave/cleanup registration, signals)
                    // must belong to the presented content's lifetime, not the cover's.
                    scope.enter(|| {
                        let content = (build)(r);
                        let mut c = BuildCx::new(node);
                        let _ = content.build(&mut c);
                    });
                    *current.borrow_mut() = Some(Presented {
                        key: r.clone(),
                        scope,
                    });
                    // Content is mounted, so any `interactive_dismiss_disabled` inside it has
                    // registered — the present patch carries the resolved flag.
                    let bg = background.as_ref().map(|f| f(r));
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(CoverPatch::Present {
                                background: bg,
                                dismiss_disabled: day_core::shield::dismiss_disabled(),
                            }),
                            false,
                        );
                        t.mark_needs_measure(node);
                        t.mark_layout_dirty();
                        t.layout_if_needed();
                    });
                }
                None => {
                    if current.borrow().is_some() && !closing.get() {
                        closing.set(true);
                        with_tree(|t| t.patch(node, Box::new(CoverPatch::Dismiss), false));
                    }
                }
            }
        };
        {
            let o = open.clone();
            bind(move || o.get_rw(), move |want: &Option<R>| reconcile(want));
        }

        // While presented, keep the backend's dismiss-disabled flag in sync with the
        // mounted `interactive_dismiss_disabled` modifiers (the shield's change counter
        // makes this binding re-run as they mount/unmount).
        {
            let current = current.clone();
            bind(
                day_core::shield::dismiss_disabled,
                move |disabled: &bool| {
                    if current.borrow().is_some() {
                        with_tree(|t| {
                            t.patch(
                                node,
                                Box::new(CoverPatch::DismissDisabled(*disabled)),
                                false,
                            )
                        });
                    }
                },
            );
        }

        {
            let (o, size, closing, dispose_content) = (
                open.clone(),
                size.clone(),
                closing.clone(),
                dispose_content.clone(),
            );
            cx.on(node, move |ev| match ev {
                // The backend sized the presented content container (safe-area bounds).
                Event::FrameChanged(sz) => {
                    if *size.borrow() != Some(*sz) {
                        *size.borrow_mut() = Some(*sz);
                        with_tree(|t| {
                            t.mark_needs_measure(node);
                            t.mark_layout_dirty();
                            t.layout_if_needed();
                        });
                    }
                }
                // Native dismissal request (Android system back). Honored unless an
                // `interactive_dismiss_disabled` subtree is mounted.
                Event::NavBack { .. } => {
                    if !day_core::shield::dismiss_disabled() && o.get_untracked_rw().is_some() {
                        o.set_rw(None);
                    }
                }
                // The hide transition finished — now the content can go.
                // Idempotent + orderable (docs/cover.md): duplicates and belated reports
                // from a previous dismissal are no-ops via the closing gate.
                Event::Custom { tag, text, .. }
                    if (*tag == "cover-hidden" || text.as_str() == "cover-hidden")
                        && closing.get() =>
                {
                    closing.set(false);
                    dispose_content();
                }
                _ => {}
            });
        }

        // String-route adapter (docs/navigation.md): `navigate("<key>")` presents, `nav_back()`
        // dismisses, and the presented key is this surface's `current_route()` contribution.
        let o_push = open.clone();
        let o_pop = open.clone();
        let o_cur = open.clone();
        let o_enter = open.clone();
        let o_seg = open;
        let push = move |k: &str, sig: &S| match R::from_key(k) {
            Some(r) => {
                sig.set_rw(Some(r));
                true
            }
            None => false,
        };
        let push2 = push;
        register_route_surface(
            move |k| push(k, &o_push),
            move |_| {
                if o_pop.get_untracked_rw().is_some() {
                    o_pop.set_rw(None);
                    true
                } else {
                    false
                }
            },
            move || {
                o_cur
                    .get_untracked_rw()
                    .map(|r| r.key())
                    .unwrap_or_default()
            },
            move |k| push2(k, &o_enter),
            move || {
                o_seg
                    .get_untracked_rw()
                    .map(|r| vec![r.key()])
                    .unwrap_or_default()
            },
        );

        node
    }
}
