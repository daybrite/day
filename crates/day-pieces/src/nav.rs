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
// Navigation (docs/navigation.md) — selector + stack, each a
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
    /// The enclosing host presents as split panes. A nested stack does NOT merge into a split
    /// host — it keeps its own detail-pane stack. Shared and mutable because the host can
    /// re-present under us: a page built while the window is narrow should merge, and one built
    /// after it widens should not.
    split: Rc<Cell<bool>>,
}

day_reactive::tls_group! {
    /// Build-time stack of enclosing nav hosts. `None` is a barrier (a resident container such as
    /// tabs) that a nested stack must not merge through.
    static NAV_HOST_CX: RefCell<Vec<Option<NavHostCx>>> = const { RefCell::new(Vec::new()) };
    /// Build-time stack of "is the page I am being built into the one its host shows?" — one entry
    /// per enclosing RESIDENT page. Only a chrome host (a tab bar, a rail) pushes one: everywhere
    /// else the outgoing page is torn down, so a surface that still exists is by definition the
    /// one on screen. A surface captures the whole stack at registration and is on screen only
    /// when every gate above it answers true.
    static NAV_PAGE_ACTIVE: RefCell<Vec<Rc<dyn Fn() -> bool>>> = const { RefCell::new(Vec::new()) };

    /// How many routed one-of-N surfaces (`selector`/tabs) are live at each nesting depth. Two at
    /// the same depth are siblings whose keys both flow into `current_route()` — the case that
    /// wants `.local()` (docs/navigation.md). Used only to warn; never changes behavior.
    static ROUTED_ONE_OF_N: RefCell<std::collections::HashMap<usize, usize>> =
        RefCell::new(std::collections::HashMap::new());
}

/// Build `f` with `gate` deciding whether what it builds counts as on screen (`NAV_PAGE_ACTIVE`).
fn with_page_active<R>(gate: Rc<dyn Fn() -> bool>, f: impl FnOnce() -> R) -> R {
    NAV_PAGE_ACTIVE.with(|s| s.borrow_mut().push(gate));
    let r = f();
    NAV_PAGE_ACTIVE.with(|s| {
        s.borrow_mut().pop();
    });
    r
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
    // The nesting depth day-core descends by. `NAV_HOST_CX` is the stack of hosts this build is
    // inside, so its length IS how deep this surface sits — and unlike registration order it does
    // not depend on whether a host registers before or after building its pages.
    let depth = NAV_HOST_CX.with(|s| s.borrow().len());
    // The resident pages this surface is built inside, captured now because the stack unwinds as
    // soon as the build returns. Empty for a surface at the window root, which is then always on
    // screen — `all` over nothing is true.
    let gates: Vec<Rc<dyn Fn() -> bool>> = NAV_PAGE_ACTIVE.with(|s| s.borrow().clone());
    let token = day_core::register_nav(day_core::NavController {
        depth,
        push: Box::new(push),
        pop: Box::new(pop),
        current: Box::new(current),
        enter: Box::new(enter),
        segments: Box::new(segments),
        active: Box::new(move || gates.iter().all(|g| g())),
    });
    Scope::current().on_cleanup(move || day_core::unregister_nav(token));
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

/// A merged stack has no host of its own to hang bar actions on, and the enclosing host's were
/// settled when IT was built. Without this the buttons simply never appear, which reads as a
/// backend bug rather than as a shape the app can change.
#[cfg(debug_assertions)]
fn warn_merged_bar_actions(n: usize) {
    log::warn!(
        "this stack's {n} bar action(s) are not drawn — it MERGED into the enclosing \
         navigation host, which owns the bar (docs/navigation.md). Declare them on that host \
         instead, or keep this stack standalone."
    );
}

#[cfg(debug_assertions)]
fn warn_sibling_selectors(kind: &str) {
    log::warn!(
        "two routed one-of-N surfaces ({kind}) are mounted at the same navigation level. \
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

/// How a [`selector`] presents its one-of-N choice (docs/navigation.md).
///
/// The default is [`Self::Automatic`]: unless you say otherwise, a selector wears whatever the
/// platform wears at the window's current width, and changes as the window does.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum SelectorStyle {
    /// **The default.** The platform's own answer at this size: a tab bar on a phone-shaped
    /// window, a rail in the middle, a sidebar beside the detail when there is room — and it
    /// re-presents live as the window crosses a breakpoint (docs/size-classes.md).
    ///
    /// This is `.tabViewStyle(.sidebarAdaptable)`'s job, and the native containers it maps to
    /// are the ones the platforms built for it: `UITabBarController` in `.tabSidebar` mode, a
    /// `NavigationView` driving its own `PaneDisplayMode`, a Material navigation bar / rail /
    /// drawer. A toolkit that cannot draw a tab bar (`Cap::NavTabs`) degrades to [`Self::Sidebar`]
    /// rather than to a hole.
    #[default]
    Automatic,
    /// Pinned: a tab bar at EVERY size, however wide the window gets. Reach for it when the
    /// content is genuinely peer sections that should never become a sidebar.
    Tabs,
    /// Pinned: a NavigationSplitView — a sidebar list + a detail. Wide windows show both panes
    /// (on GTK an `AdwNavigationSplitView`); narrow ones collapse to a list that PUSHES the
    /// detail, which is the one shape [`Self::Automatic`] never produces on its own.
    Sidebar,
}

/// A built detail page of a selector, and what it takes to tear it down. Exactly one of these
/// exists per shown page in a split or stacked presentation; one per VISITED page in a
/// presentation whose rows are chrome, where pages are resident (docs/navigation.md).
struct ResidentPage {
    /// The item key this page was built for — the identity a re-selection matches on.
    key: String,
    /// The page's own reactive scope; disposing it runs the content's cleanup.
    scope: Scope,
    /// The `NAV_PAGE` node, in the host's detail-child attach order. Its index in the owning
    /// `Vec` is what `NavPatch::Select` addresses.
    node: RNode,
}

/// Builds the page for a data-driven key (`&K` → piece) — a selector's `.destination` fallback
/// or a stack's `.destination`.
type DestFn<K> = Rc<dyn Fn(&K) -> AnyPiece>;

/// Which destinations show the content-list pane (`Selector::content_list_for`).
type ListPred<K> = Rc<dyn Fn(&K) -> bool>;

/// Completions for a search field's current text (`Selector::search_suggestions`).
type SuggestFn = Rc<dyn Fn(&str) -> Vec<String>>;

/// The toolbar item id a `.searchable()` surface's field carries when its placement resolves to
/// the window toolbar (docs/search.md). Reserved and stable: dayscript's `toolbar:` step addresses
/// the field by this id, and it is Day's own rather than the app's, since the app never declares
/// the item.
pub const SEARCH_ITEM_ID: &str = "day.search";

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
    Vec<Option<String>>,
    Vec<Option<day_spec::Color>>,
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
    /// Trailing accessory GLYPH, in the same slot as `badge` (docs/navigation.md).
    badge_icon: Option<String>,
    /// Tint for `badge_icon`; `None` leaves it at the backend's neutral template tint.
    badge_tint: Option<day_spec::Color>,
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
    badge_icon: Option<String>,
    badge_tint: Option<day_spec::Color>,
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
        badge_icon: None,
        badge_tint: None,
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
        // Scoped: row menus are re-lowered on every derive; the previous build's closures
        // are reclaimed when the registering scope (the selector's surface) is disposed.
        self.menu = crate::menus::lower_menu_scoped(entries);
        self
    }
    /// A trailing accessory for this row — an unread count, a status. Rendered right-aligned
    /// and de-emphasized where the toolkit has an affordance for it, and dropped where it does
    /// not (see docs/coverage-matrix.md).
    pub fn badge<M>(mut self, badge: impl IntoText<M>) -> Self {
        self.badge = Some(badge.into_text());
        self
    }
    /// A trailing accessory GLYPH for this row, in the same slot as [`Self::badge`] and drawn
    /// after it. Takes a bundled image name exactly as [`Self::icon`] does, so a symbol
    /// (`Symbol::Star`) or an app image both work.
    ///
    /// This is what a row-level STATUS gets drawn with — a starred page's star — where `badge`
    /// carries a count or a word. Pair it with [`Self::badge_tint`] when the glyph's color is
    /// part of its meaning; left untinted it takes the backend's neutral template tint and reads
    /// as another piece of chrome.
    pub fn badge_icon(mut self, icon: impl Into<day_spec::ImageName>) -> Self {
        self.badge_icon = Some(icon.into().to_string());
        self
    }
    /// The color for [`Self::badge_icon`]. `None` (the default) keeps the neutral template tint.
    pub fn badge_tint(mut self, color: day_spec::Color) -> Self {
        self.badge_tint = Some(color);
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
// Boxing `Static` would trade a real allocation PER ROW for a saving that is nominal here: these
// enums live in one `Vec` built once when a selector is constructed, a few dozen entries at most,
// and the static case is the overwhelmingly common one. The gap is only this wide because a row
// carries its decorations inline (icon, tint, badge text, badge glyph, section, menu).
#[allow(clippy::large_enum_variant)]
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
// Boxing `Static` would trade a real allocation PER ROW for a saving that is nominal here: these
// enums live in one `Vec` built once when a selector is constructed, a few dozen entries at most,
// and the static case is the overwhelmingly common one. The gap is only this wide because a row
// carries its decorations inline (icon, tint, badge text, badge glyph, section, menu).
#[allow(clippy::large_enum_variant)]
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
    badge_icon: Option<String>,
    badge_tint: Option<day_spec::Color>,
    section: Option<TextSource>,
}

/// One selector's live rows, flattened across its static items and dynamic blocks.
struct NavRows<K> {
    keys: Vec<K>,
    titles: Vec<String>,
    icons: Vec<Option<String>>,
    badges: Vec<Option<String>>,
    badge_icons: Vec<Option<String>>,
    badge_tints: Vec<Option<day_spec::Color>>,
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
                            badge_icon: it.badge_icon,
                            badge_tint: it.badge_tint,
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
            badge_icons: Vec::new(),
            badge_tints: Vec::new(),
            sections: Vec::new(),
            tints: Vec::new(),
            menus: Vec::new(),
        };
        let mut push = |k: K, title: String, m: &RowMeta| {
            r.keys.push(k);
            r.titles.push(title);
            r.icons.push(m.icon.clone());
            r.badges.push(m.badge.as_ref().map(|b| b.resolve()));
            r.badge_icons.push(m.badge_icon.clone());
            r.badge_tints.push(m.badge_tint);
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
                            badge_icon: it.badge_icon,
                            badge_tint: it.badge_tint,
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
pub struct Selector<S: Binding<K>, K: Route = String> {
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
    bar_actions: Vec<BarActionSpec>,
    /// Search over this surface ([`Selector::searchable`]). `None` unless set.
    search: Option<SearchSpec>,
    /// The presentation pinned by [`Selector::presentation`]; `None` = automatic, resolved from
    /// the window's size class and re-resolved whenever it changes.
    presentation: Option<day_spec::props::NavPresentation>,
    /// The content-list pane builder ([`Selector::content_list`], docs/navigation.md) — the
    /// Mail shape's middle column, built ONCE and resident for the host's life. `None` = the
    /// classic two-pane selector.
    content_list: Option<Rc<dyn Fn() -> AnyPiece>>,
    /// Preferred width of the content-list pane in points.
    content_list_width: f64,
    /// Which destinations show the pane ([`Selector::content_list_for`]); `None` = all of them.
    content_list_pred: Option<ListPred<K>>,
    /// Whether the DETAIL is showing, two-way ([`Selector::detail_visible`]) — what gates the
    /// detail push in a stacked presentation with a content list, and what native back writes
    /// `false` into. `None` = the detail behaves classically (pushed on selection).
    detail_visible: Option<Signal<bool>>,
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
    /// Turn a requested placement into the one this toolkit will actually use (docs/search.md).
    ///
    /// `Automatic` asks the platform. Today the answer is the window toolbar wherever the toolkit
    /// has one, and inline — attached to the navigation surface — where it does not, which is the
    /// phones. That second case needs no size class: "this toolkit has no toolbar at all" is a
    /// static fact about the backend, not a question about the window's width. Resolving a narrow
    /// window on a toolkit that DOES have a toolbar is what waits on the size-class work.
    ///
    /// Never returns `Automatic`: the props carry the decision, so a backend reads a placement it
    /// can act on rather than re-deriving the policy itself.
    fn resolve(requested: day_spec::props::SearchPlacement) -> day_spec::props::SearchPlacement {
        use day_spec::props::SearchPlacement as P;
        match requested {
            P::Toolbar | P::Inline => requested,
            P::Automatic => {
                if with_tree(|t| t.capability(day_spec::Cap::Toolbar))
                    == day_spec::Support::Unsupported
                {
                    P::Inline
                } else {
                    P::Toolbar
                }
            }
        }
    }

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
            // The RESOLVED placement, so a backend reads a decision rather than a request.
            placement: Self::resolve(self.placement),
            scopes: self.scopes.iter().map(TextSource::initial).collect(),
            scope: self.scope.map(|s| s.get_untracked()).unwrap_or(0),
        }
    }

    /// Install the field and wire it, both directions (docs/search.md).
    ///
    /// ONE model, ONE writer. `SearchProps` on the nav host is the source of truth for every
    /// placement; the toolbar item a desktop backend draws is a RENDERING of it, not a second
    /// representation with its own state. Both inbound transports — a toolbar value callback and
    /// `Event::SearchChanged` from an inline field — land on the same `apply` closure, and the
    /// single outbound binding patches whichever target the resolved placement renders into.
    ///
    /// That is what makes a future placement change tractable: the state does not live in the
    /// widget, so re-rendering into the other target is a patch rather than a rebuild. The
    /// remaining step for the size-class work is a `SearchPatch::Placement` that swaps the render
    /// target on a live host — see docs/search.md.
    fn install(&self, host: RNode, seed: &day_spec::props::SearchProps) {
        use day_spec::props::{SearchPatch, SearchPlacement as P};
        let placement = Self::resolve(self.placement);
        let query = self.query;
        // Controlled input (§4.4), tracked by VALUE rather than by origin.
        //
        // This used to be a one-shot origin guard: the inbound handler armed it with the text the
        // field reported, and the outbound binding consumed it to avoid patching that same text
        // back. One-shot is the wrong shape for a two-way sync. Whether it is armed at the moment
        // the binding runs depends on how many changes arrived first and whether the binding ran
        // at all, so it swallowed patches it should have sent (a cleared query that never reached
        // the field) and let through patches it should have skipped (rewriting the field mid-type,
        // which resets the caret and drops focus on AppKit).
        //
        // What the field HOLDS is the fact that matters, and it is knowable: the field reports
        // every value it takes, and Day knows every value it pushes. Comparing against it is
        // idempotent and order-independent — no arming, nothing to consume, no way to get out of
        // step. Equal means the field already shows it, so there is nothing to push and no caret
        // to disturb.
        let shown: Rc<RefCell<String>> = Rc::new(RefCell::new(seed.text.clone()));

        if placement == P::Toolbar {
            let g = shown.clone();
            let action =
                day_core::register_toolbar_value(Rc::new(move |v: &day_spec::ToolbarValue| {
                    if let day_spec::ToolbarValue::Text(t) = v {
                        *g.borrow_mut() = t.clone();
                        query.set(t.clone());
                    }
                }));
            day_core::set_window_search(
                day_core::toolbar::current_window(),
                Some(day_spec::ToolbarItem {
                    id: SEARCH_ITEM_ID.to_string(),
                    kind: day_spec::ToolbarItemKind::Search {
                        text: seed.text.clone(),
                        placeholder: seed.prompt.clone(),
                        suggestions: seed.suggestions.clone(),
                    },
                    label: seed.prompt.clone(),
                    tooltip: None,
                    icon: None,
                    enabled: true,
                    action,
                }),
            );
        }

        // The one outbound binding: the app writing its query reaches whichever target this
        // placement renders into. Seeded, because `lower` already put the value in the realize
        // props — re-applying it here would be the duplicate op §5.2 forbids.
        let window = day_core::toolbar::current_window();
        bind_seeded(
            seed.text.clone(),
            move || query.get(),
            move |t: &String| {
                // Before the comparison, because this has to happen for BOTH directions: the
                // stored item is what a toolbar REBUILD re-seeds the field from, and a rebuild can
                // be triggered by anything else on the bar. Skipping it on the field-originated
                // path is what left the box empty while the query kept filtering.
                day_core::toolbar::set_window_search_state(window, Some(t.as_str()), None);
                if *shown.borrow() == *t {
                    return; // the field already shows this; patching it back fights the caret
                }
                *shown.borrow_mut() = t.clone();
                match placement {
                    P::Toolbar => day_core::patch_window_toolbar(
                        window,
                        day_spec::ToolbarPatch::Text {
                            item: SEARCH_ITEM_ID.to_string(),
                            text: t.clone(),
                        },
                    ),
                    _ => {
                        let p = SearchPatch::Text(t.clone());
                        with_tree(|tr| tr.patch(host, Box::new(p), false));
                    }
                }
            },
        );

        if let Some(sig) = self.scope {
            let seed_scope = seed.scope;
            bind_seeded(
                seed_scope,
                move || sig.get(),
                move |i| {
                    let p = SearchPatch::Scope(*i);
                    with_tree(|tr| tr.patch(host, Box::new(p), false));
                },
            );
        }
        // Completions re-derive on every keystroke AND on whatever else the closure reads.
        if let Some(f) = self.suggestions.clone() {
            bind_seeded(
                seed.suggestions.clone(),
                move || f(&query.get()),
                move |list| match placement {
                    P::Toolbar => {
                        // Same staleness as the text: a rebuild re-seeds completions from the
                        // stored item, so it has to carry the current list too.
                        day_core::toolbar::set_window_search_state(window, None, Some(list));
                        day_core::patch_window_toolbar(
                            window,
                            day_spec::ToolbarPatch::Suggestions {
                                item: SEARCH_ITEM_ID.to_string(),
                                list: list.clone(),
                            },
                        )
                    }
                    _ => {
                        let p = SearchPatch::Suggestions(list.clone());
                        with_tree(|t| t.patch(host, Box::new(p), false));
                    }
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
    scope: day_spec::props::NavBarScope,
}

impl BarActionSpec {
    /// Register the closure (getting a dispatch id) and resolve the label, producing the spec
    /// value the NAV host carries. Called once, at build.
    fn lower(self) -> day_spec::props::NavBarAction {
        day_spec::props::NavBarAction {
            // Scoped: the id dies with the nav build's scope — bar actions are re-lowered
            // on every rebuild and were previously never reclaimed.
            action: day_core::register_scoped_menu_action(self.action),
            label: self.label.initial(),
            icon: self.icon,
            scope: self.scope,
        }
    }
}

pub fn selector<K: Route, S: Binding<K>>(selection: S) -> Selector<S, K> {
    Selector {
        selection,
        style: SelectorStyle::default(),
        pending_section: None,
        title: TextSource::Static(String::new()),
        header: None,
        sources: Vec::new(),
        destination: None,
        routed: true,
        restore: None,
        bar_actions: Vec::new(),
        search: None,
        presentation: None,
        content_list: None,
        content_list_width: day_spec::NAV_LIST_WIDTH,
        content_list_pred: None,
        detail_visible: None,
    }
}

impl<K: Route, S: Binding<K>> Selector<S, K> {
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
    /// The static-item counterpart of [`NavItem::badge_icon`]: a trailing glyph on the item just
    /// added. `.item(…).badge_icon(Symbol::Star).badge_tint(STAR)`.
    ///
    /// Ignored (in release) when no static item precedes it.
    pub fn badge_icon(mut self, icon: impl Into<day_spec::ImageName>) -> Self {
        let name = icon.into().to_string();
        match self.sources.last_mut() {
            Some(ItemSource::Static(it)) => it.badge_icon = Some(name),
            _ => debug_assert!(false, "day: `.badge_icon(…)` needs a preceding `.item…(…)`"),
        }
        self
    }
    /// The color for [`Self::badge_icon`] on the item just added.
    pub fn badge_tint(mut self, color: day_spec::Color) -> Self {
        match self.sources.last_mut() {
            Some(ItemSource::Static(it)) => it.badge_tint = Some(color),
            _ => debug_assert!(false, "day: `.badge_tint(…)` needs a preceding `.item…(…)`"),
        }
        self
    }
    /// An optional piece shown above the sidebar list (a logo, app name…).
    pub fn header<P: Piece>(mut self, build: impl FnOnce() -> P + 'static) -> Self {
        self.header = Some(Box::new(move || AnyPiece::new(build())));
        self
    }
    /// Pin the presentation instead of letting the window's size decide (docs/size-classes.md).
    ///
    /// Leave this unset — the default — and the selector shows sidebar+detail on a window wide
    /// enough for both and stacks on one that is not, re-presenting live as the window is resized
    /// or the device rotated. Pin it when the content only works one way: a settings sidebar whose
    /// detail is meaningless alone, a wizard that must stay a stack.
    ///
    /// A pin is still a preference. A toolkit with no split container (the phones today) stacks
    /// whatever is asked for, exactly as it does for the automatic case.
    pub fn presentation(mut self, presentation: day_spec::props::NavPresentation) -> Self {
        self.presentation = Some(presentation);
        self
    }
    /// A CONTENT-LIST pane between the sidebar and the detail — the Mail shape: mailboxes,
    /// message list, message (docs/navigation.md). Built ONCE and resident for the host's
    /// life; its content follows the app's own signals (the selection scoping it, the row
    /// chosen from it), never a rebuild.
    ///
    /// Where the toolkit has a native pane (`Cap::NavContentList`) the list gets its own
    /// column — an AppKit `contentList` split item, a UIKit supplementary column — and
    /// otherwise the selector composes it beside (split) or in place of (stacked) each
    /// destination. Pair with [`Self::detail_visible`] so compact widths get the
    /// list-then-detail push flow, and [`Self::content_list_for`] to give full-width
    /// destinations (a settings page) the whole detail area.
    pub fn content_list<P: Piece>(mut self, build: impl Fn() -> P + 'static) -> Self {
        self.content_list = Some(Rc::new(move || AnyPiece::new(build())));
        self
    }
    /// Preferred width of the content-list pane in points (default
    /// [`day_spec::NAV_LIST_WIDTH`]). Where the divider is user-draggable this is the initial
    /// width, not a limit.
    pub fn content_list_width(mut self, width: f64) -> Self {
        self.content_list_width = width;
        self
    }
    /// Which destinations show the content-list pane (default: all of them). A destination
    /// answering `false` collapses the pane and takes the whole detail area — the settings
    /// page beside a mail-shaped app.
    pub fn content_list_for(mut self, pred: impl Fn(&K) -> bool + 'static) -> Self {
        self.content_list_pred = Some(Rc::new(pred));
        self
    }
    /// Whether the DETAIL is showing, as a two-way signal (docs/navigation.md). Split-family
    /// presentations ignore it — the detail pane is always there, showing the app's empty
    /// state until a row is chosen. A STACKED presentation (a phone, a collapsed split) uses
    /// it as the push gate: the content list is the top of the stack until the app writes
    /// `true` (a row was opened), the detail pushes then, and the platform's back writes
    /// `false` on the way out.
    pub fn detail_visible(mut self, visible: Signal<bool>) -> Self {
        self.detail_visible = Some(visible);
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
            badge_icon: None,
            badge_tint: None,
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
            badge_icon: None,
            badge_tint: None,
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
    /// Recolor the LAST-added `.item_icon` destination's glyph (docs/vectors.md), instead of
    /// letting it take the sidebar's neutral template tint.
    ///
    /// The static counterpart to [`NavItem::icon_tint`], and the same last-added-item shape as
    /// [`immersive`](Self::immersive) — a tint belongs to one row, but `.item_icon(…)` returns
    /// the selector rather than the item, so the row is named by position rather than by handle.
    ///
    /// Untinted glyphs follow the theme, which is usually what a navigation list wants; reach
    /// for this where the color CARRIES something (a per-section identity, a status).
    pub fn icon_tint(mut self, color: day_spec::Color) -> Self {
        if let Some(ItemSource::Static(item)) = self.sources.last_mut() {
            item.tint = Some(color);
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
                        badge_icon: ni.badge_icon,
                        badge_tint: ni.badge_tint,
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
    ///
    /// Call it more than once for more than one button; they draw left to right in declaration
    /// order, trailing-aligned. Use [`list_action`](Self::list_action) for a command that acts on
    /// the LIST rather than on whatever page is open.
    pub fn bar_action<M>(
        mut self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.bar_actions.push(BarActionSpec {
            icon: Some(icon.into().as_str().to_owned()),
            label: label.into_text(),
            action: Rc::new(action),
            scope: day_spec::props::NavBarScope::EveryPage,
        });
        self
    }

    /// Like [`bar_action`](Self::bar_action), but the button rides the LIST only — it is gone from
    /// the detail pages the list pushes (docs/navigation.md).
    ///
    /// Which one to reach for is decided by what the command acts on, not by how it looks. "Add an
    /// item" and "sort" act on the list, and on a detail page the thing they act on is not even on
    /// screen — a narrow phone has pushed it away — so a button for them there is at best inert
    /// and at worst acts on something the user cannot see. "Show this page's source" is the other
    /// kind: it follows the user down.
    ///
    /// Where the presentation keeps the list in its own pane, its bar is the list's bar and the
    /// button simply stays there while details come and go.
    pub fn list_action<M>(
        mut self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.bar_actions.push(BarActionSpec {
            icon: Some(icon.into().as_str().to_owned()),
            label: label.into_text(),
            action: Rc::new(action),
            scope: day_spec::props::NavBarScope::RootPage,
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

    /// Ask for a particular placement. A PREFERENCE: a backend that cannot honor it falls back
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
    /// `NSSegmentedControl`) or, on web and system XAML, one composed from primitives.
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

impl<K: Route, S: Binding<K>> Piece for Selector<S, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        // ONE builder for all three styles. The styles differ only in which presentations the
        // resolver may produce, and a presentation differs only in chrome and page residency —
        // so a tab bar is a `kinds::NAV` host wearing a different hat, not a second host kind
        // with its own props, patches, and nine backend implementations (docs/navigation.md).
        build_selector(self, cx)
    }
}

/// Apply a selector's `.restore` at build: seed `selection` from the key saved under `restore`,
/// so the app reopens on the section/tab the user last chose. A pending launch deep link wins
/// (skip). Only a saved key that parses AND is a current item is honored — plus the empty
/// "deselected" key, for a sidebar's collapsed state — so a stale key left by an older build is
/// ignored. A no-op when `restore` is unset or no [`NavStore`](day_core::NavStore) is installed.
fn restore_selection<K: Route, S: Binding<K>>(
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
        selection.write(k);
    }
}

/// Persist a selector's selection under its `.restore` key on every change (docs/navigation.md).
/// The binding lives in the current scope, so it stops with the surface. Consumes `restore`.
fn persist_selection<K: Route, S: Binding<K>>(restore: Option<String>, selection: &S) {
    if let Some(key) = restore {
        let s = selection.clone();
        bind(
            move || s.read().key(),
            move |k: &String| day_core::nav_store_save(&key, k),
        );
    }
}

fn build_selector<K: Route, S: Binding<K>>(sel: Selector<S, K>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{
        NavMenuPatch, NavMenuProps, NavPageProps, NavPatch, NavPresentation, NavProps, Pane,
    };
    // Presentation (docs/size-classes.md). Resolved from the window's size class on every change,
    // not fixed at build time — `NavPatch::Presentation` re-presents the live host. The window
    // root is captured HERE: the effect below re-runs long after this build, when
    // `current_window` would answer the primary window instead of ours.
    let window = day_core::current_window();
    let can_split =
        with_tree(|t| t.capability(day_spec::Cap::NavSplit)) == day_spec::Support::Native;
    // WHO DECIDES the presentation (docs/size-classes.md), which is what `Cap::NavRepresent`'s
    // three answers distinguish:
    //   Native      — we do, and we patch the host when the class changes.
    //   Emulated    — the toolkit's own adaptive container does, and reports back.
    //   Unsupported — nobody; it is fixed at build time from `Cap::NavSplit` alone, because a
    //                 toolkit that cannot change presentation must not have it decided by
    //                 something that can — a window launched narrow would be stuck stacked.
    // Can the toolkit draw the rows as its own chrome — a tab bar, and a rail where it has one?
    // `Emulated` counts: Qt and web-dom compose theirs, and a composed tab bar is still a tab bar
    // to the app. Only `Unsupported` sends `Automatic` down the sidebar path.
    let can_tabs =
        with_tree(|t| t.capability(day_spec::Cap::NavTabs)) != day_spec::Support::Unsupported;
    // A separate question from `can_tabs`: every desktop CAN draw a tab bar (an app may pin one)
    // but none of them should GROW one as its window narrows. That is a statement about the
    // platform's idiom rather than its widget set, so it is the backend's to make.
    let adaptive_tabs = with_tree(|t| t.capability(day_spec::Cap::NavTabsAdaptive))
        != day_spec::Support::Unsupported;
    let represent = with_tree(|t| t.capability(day_spec::Cap::NavRepresent));
    let we_drive = represent == day_spec::Support::Native;
    let toolkit_drives = represent == day_spec::Support::Emulated;
    // The content-list pane (docs/navigation.md). `Native`/`Emulated` = the toolkit places the
    // `Pane::List` page itself; `Unsupported` = the wrapper below composes it around each
    // destination and the backend never hears of it.
    let list_cap = with_tree(|t| t.capability(day_spec::Cap::NavContentList));
    let native_list = sel.content_list.is_some() && list_cap != day_spec::Support::Unsupported;
    // A merged-pane backend (uikit) folds the list into the stack when it collapses; that is
    // the only shape where the detail push is gated on `detail_visible`.
    let merged_list = native_list && list_cap == day_spec::Support::Emulated;
    // Either way the window's size decides the INITIAL value: a backend that morphs itself still
    // starts wherever its container will land, so seeding from the class avoids a first frame in
    // the wrong presentation followed by a correcting report.
    let size_decides = we_drive || toolkit_drives;
    let requested = sel.presentation;
    // `Automatic` on a toolkit with no tab bar becomes `Sidebar` — the behavior every backend had
    // before adaptive navigation existed. Degrading to the OLD shape rather than to a hole is what
    // lets this land one backend at a time (docs/navigation.md).
    let style = match sel.style {
        SelectorStyle::Automatic if !can_tabs => SelectorStyle::Sidebar,
        s => s,
    };
    // The resolver owns every "can this toolkit do it" question, so `NavProps` always carries a
    // presentation the backend can actually draw.
    let resolve = move |class: Option<day_spec::SizeClass>| -> NavPresentation {
        // A pin is still a PREFERENCE (docs/size-classes.md): a toolkit with no split container
        // stacks whatever it is asked for, and one with no tab bar cannot wear a pinned `Tabs`.
        if let Some(p) = requested {
            return match p {
                NavPresentation::Split if !can_split => NavPresentation::Stack,
                NavPresentation::Tabs | NavPresentation::Rail if !can_tabs => {
                    if can_split {
                        NavPresentation::Split
                    } else {
                        NavPresentation::Stack
                    }
                }
                p => p,
            };
        }
        match style {
            // Pinned tabs: the same bar at every size, so the window never enters into it.
            SelectorStyle::Tabs => NavPresentation::Tabs,
            // Today's resolution, unchanged: split where there is room, stacked where there isn't.
            SelectorStyle::Sidebar => match class {
                _ if !can_split => NavPresentation::Stack,
                Some(c) if size_decides && !c.prefers_split() => NavPresentation::Stack,
                _ => NavPresentation::Split,
            },
            // The adaptive ladder: a tab bar at the narrow end, a rail in the middle, a sidebar
            // beside the detail when there is room.
            //
            // Only the BOTTOM rung is platform-specific. Where a tab bar is the idiomatic compact
            // answer (the phones, the web) that is what a narrow window gets; on a desktop it
            // collapses to a stack instead, which is what `Sidebar` has always done and what a
            // Mac or GNOME app does when you drag it narrow. The rail and the split are the same
            // everywhere.
            SelectorStyle::Automatic => {
                let compact = if adaptive_tabs {
                    NavPresentation::Tabs
                } else {
                    NavPresentation::Stack
                };
                if !size_decides {
                    // Nobody re-presents, so this is decided once and must not be decided by
                    // something that changes underneath (docs/size-classes.md). Prefer the
                    // roomier container the toolkit has.
                    return if can_split {
                        NavPresentation::Split
                    } else {
                        compact
                    };
                }
                if !can_split {
                    return compact;
                }
                match class.map(|c| c.width) {
                    Some(day_spec::WidthClass::Compact) => compact,
                    Some(day_spec::WidthClass::Medium) => NavPresentation::Rail,
                    _ => NavPresentation::Split,
                }
            }
        }
    };
    let presentation = resolve(day_core::window_size_class_untracked(window));
    // What the HOST PROPS carry is a different question from what this build currently shows.
    // An Emulated toolkit's adaptive container collapses and expands ITSELF, so its host is
    // lowered as `Split` — "build the adaptive container" — even when the window is compact
    // right now. `Stack` in props is thereby reserved for hosts that are stacks at EVERY size
    // (a pinned request, a toolkit that cannot split, the `stack()` piece), which a backend
    // may take literally and realize as a plain navigation container: nesting an adaptive
    // split container inside a pane is exactly what breaks (docs/size-classes.md).
    // An Emulated toolkit's adaptive container collapses and expands ITSELF, so it is lowered the
    // ROOMIEST presentation the app's style admits — "build the adaptive container" — even when
    // the window is compact right now. `Stack` in props is thereby reserved for hosts that are
    // stacks at EVERY size (a pinned request, a toolkit that cannot split, the `stack()` piece),
    // which a backend may take literally: nesting an adaptive container inside a pane is exactly
    // what breaks (docs/size-classes.md). `adaptive` carries whether it may morph at all.
    let adaptive = requested.is_none();
    let lowered = if toolkit_drives
        && adaptive
        && can_tabs
        && adaptive_tabs
        && style != SelectorStyle::Sidebar
    {
        // The toolkit has an adaptive container that wears BOTH chromes itself — iOS 18's
        // `UITabBarController` in `.tabSidebar` mode is the archetype: one controller that draws
        // a tab bar when compact and a sidebar when not, with its own animation and its own
        // user-facing toggle. Lowering `Tabs` says "build that", and the toolkit reports what it
        // settled on. Its pages are resident at every size, which is why such a host stays in the
        // chrome-rows model rather than flipping to push/pop as it widens.
        //
        // Only for a style that ADMITS a tab bar. `Sidebar` is a app's explicit "this is a list
        // beside a detail", and its compact answer has always been the stack — one navigation
        // controller the list pushes onto, which is what a phone app with more sections than fit
        // a tab bar looks like. Handing that app a tab container instead puts a bar under every
        // page and (below iOS 18, where there is no sidebar mode to switch to) leaves it there.
        NavPresentation::Tabs
    } else if toolkit_drives && adaptive && can_split && style != SelectorStyle::Tabs {
        NavPresentation::Split
    } else {
        presentation
    };
    // A `.tabSidebar`-style host is a TABS host at every width — its pages are resident whether
    // it is drawing a bar or a sidebar — so the resolved presentation has to agree with what the
    // backend was told to build. Left disagreeing, day-core would model push/pop while the
    // toolkit modeled resident tabs: the first page built would merge into a host that has no
    // stack to merge into, and land zero-sized.
    let presentation = if lowered == NavPresentation::Tabs && toolkit_drives && adaptive {
        NavPresentation::Tabs
    } else {
        presentation
    };
    let split = presentation.is_split();
    let presentation_cell = Rc::new(Cell::new(presentation));
    // The same fact as a SIGNAL, for content the composed list wrapper builds inside the
    // pages: a re-present re-runs its `when` where a Cell could only be re-read on rebuild.
    let presentation_sig = Signal::new(presentation);
    let split_cell = Rc::new(Cell::new(split));
    let sidebar_cell: Rc<Cell<Option<RNode>>> = Rc::new(Cell::new(None));
    let list_cell: Rc<Cell<Option<RNode>>> = Rc::new(Cell::new(None));
    // ListVisible / ListInStack bookkeeping (both start where the backend starts: the pane
    // visible, the stack without the list). `list_shown` is shared with NavLayout so a
    // collapsed pane stops narrowing the detail's fallback at once.
    let list_shown = Rc::new(Cell::new(true));
    let list_in_stack = Rc::new(Cell::new(false));
    let selection = sel.selection;
    let routed = sel.routed;
    let restore = sel.restore;
    let title_s = sel.title.initial();
    // Register the optional nav-bar action once (getting its dispatch id) and lower it into the
    // host props — the mobile backends draw it as an upper-right bar button (docs/navigation.md).
    let bar_actions: Vec<_> = sel
        .bar_actions
        .into_iter()
        .map(BarActionSpec::lower)
        .collect();
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
            presentation: lowered,
            adaptive,
            bar_actions,
            search,
            list_width: native_list.then_some(sel.content_list_width),
        },
        Rc::new(NavLayout {
            sizes: sizes.clone(),
            presentation: presentation_cell.clone(),
            sidebar: sidebar_cell.clone(),
            list: list_cell.clone(),
            list_width: sel.content_list_width,
            list_visible: list_shown.clone(),
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
        spec.install(host, seed);
        let query = spec.query;
        let scope_sig = spec.scope;
        cx.on(host, move |ev| match ev {
            Event::SearchChanged(text) => query.set(text.clone()),
            Event::SearchScopeChanged(i) => {
                if let Some(s) = scope_sig {
                    s.set(*i);
                }
            }
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
        split: split_cell.clone(),
    };

    // Sidebar / root page. `Pane::Sidebar` is unconditional: it says what this page IS in the
    // model, not how the host happens to draw it today, which is what lets a re-present re-home
    // it between the sidebar pane and the root of the stack (docs/size-classes.md).
    let root_page = nav_page(
        host,
        &NavPageProps {
            title: title_s.clone(),
            pane: Pane::Sidebar,
        },
        &sizes,
    );
    sidebar_cell.set(Some(root_page));
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
        let (badge_icons_init, badge_tints_init) =
            (rows0.badge_icons.clone(), rows0.badge_tints.clone());
        let tints_init = rows0.tints.clone();
        let menus_init = rows0.menus.clone();
        let menu_piece = piece_fn(move |mcx| {
            let node = mcx.native(
                kinds::NAV_MENU,
                &NavMenuProps {
                    items: titles_init,
                    icons: icons_init,
                    badges: badges_init,
                    badge_icons: badge_icons_init,
                    badge_tints: badge_tints_init,
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
                    s.write(k.clone());
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

    // The content-list page (docs/navigation.md): built ONCE, resident for the host's life —
    // its content follows the app's signals, so a selection change re-scopes it without a
    // rebuild. `with_nav_host(None)`: the pane is not a merge target — a stack inside the
    // list keeps its own container rather than pushing onto this host.
    let list_build = sel.content_list.clone();
    let list_pred = sel.content_list_pred.clone();
    let detail_visible = sel.detail_visible;
    // Whether the content-list pane belongs to THIS destination (`content_list_for`), applied
    // wherever the shown destination can change. It lives here, shared, because three paths
    // reach it: a fresh `show`, a `show` that finds its page already resident (a chrome
    // presentation has built them all), and a presentation change that rebuilds the shape
    // around an unchanged selection. Missing any one of them strands the pane — collapsed
    // because some other destination was the last to speak, with nothing left to reopen it.
    let apply_list_visible: Rc<dyn Fn(&K)> = {
        let (shown, pred) = (list_shown.clone(), list_pred.clone());
        Rc::new(move |key: &K| {
            if !native_list {
                return;
            }
            let want = pred.as_ref().is_none_or(|p| p(key));
            if want != shown.get() {
                shown.set(want);
                with_tree(|t| t.patch(host, Box::new(NavPatch::ListVisible(want)), false));
            }
        })
    };
    if native_list && let Some(build_list) = list_build.clone() {
        let page = nav_page(
            host,
            &NavPageProps {
                title: title_s.clone(),
                pane: Pane::List,
            },
            &sizes,
        );
        list_cell.set(Some(page));
        with_nav_host(None, || {
            let mut pcx = BuildCx::new(page);
            let _ = build_list().build(&mut pcx);
        });
    }
    // The COMPOSED content list (`Cap::NavContentList` Unsupported): every list-backed
    // destination page carries the pane itself — beside the content while split, in place of
    // it while stacked until `detail_visible` opens a row. The presentation is read through
    // the SIGNAL, so a live morph re-arranges the page without the host's involvement.
    let compose: Option<DestFn<K>> = if let (Some(list), true) = (
        list_build.clone(),
        list_cap == day_spec::Support::Unsupported,
    ) {
        let pred = list_pred.clone();
        let width = sel.content_list_width;
        let dv = detail_visible;
        let items_c = items.clone();
        let pres = presentation_sig;
        Some(Rc::new(move |key: &K| {
            if pred.as_ref().is_some_and(|p| !p(key)) {
                return items_c.build_page(key);
            }
            let (l1, l2) = (list.clone(), list.clone());
            let (i1, i2) = (items_c.clone(), items_c.clone());
            let (k1, k2) = (key.clone(), key.clone());
            AnyPiece::new(
                when(
                    move || {
                        let p = pres.get();
                        p.is_split() || p.rows_are_chrome()
                    },
                    move || {
                        let (l, i, k) = (l1.clone(), i1.clone(), k1.clone());
                        row((l().width(width).grow_h(), i.build_page(&k).grow())).grow()
                    },
                )
                .otherwise(move || {
                    let (l, i, k) = (l2.clone(), i2.clone(), k2.clone());
                    match dv {
                        Some(d) => AnyPiece::new(
                            when(move || d.get(), {
                                let (i, k) = (i.clone(), k.clone());
                                move || i.build_page(&k).grow()
                            })
                            .otherwise(move || l().grow()),
                        ),
                        None => i.build_page(&k),
                    }
                }),
            )
        }))
    } else {
        None
    };

    let sync_menu = {
        let mh = menu_holder.clone();
        move |idx: Option<usize>| {
            if let Some(m) = mh.get() {
                with_tree(|t| t.patch(m, Box::new(NavMenuPatch::Selected(idx)), false));
            }
            // And onto the host, which is the node `.id()` names — a script asserting on a
            // selector addresses the selector, not the row list nested inside it. Every path that
            // changes the selection lands here, so this is the one place that has to record it.
            with_tree(|t| t.set_probe_selected(host, idx));
        }
    };

    // Detail pages, and how long they live.
    //
    // A presentation whose rows are the CHROME (a tab bar, a rail) keeps every visited page
    // RESIDENT: switching tabs is `NavPatch::Select`, nothing is torn down, and each tab keeps
    // its scroll offset, its focused field, and its animations — which is what every native tab
    // container does. A split or stacked presentation keeps only the shown page and switches by
    // pop-then-push, which is what it did before adaptive navigation existed.
    //
    // Residency follows the PRESENTATION rather than the host, deliberately. Making every nav
    // page resident would keep effects running for pages nobody is looking at and change the
    // disposal contract the docs make ("leaving the piece's branch disposes"); making none
    // resident would rebuild a tab's content on every tap. Splitting it this way means a morph
    // only ever disposes pages that are NOT on screen, or lazily builds ones that were not built
    // yet — the VISIBLE page is never rebuilt, which is the invariant a morph has to keep.
    let resident: Rc<RefCell<Vec<ResidentPage>>> = Rc::default();
    let current: Rc<RefCell<Option<String>>> = Rc::default();
    let nav_scope = Scope::current();
    // Shared: BOTH the selection bind and the row-derive effect drive the detail (see the
    // derive effect for why the selection bind alone is not enough).
    let show = std::rc::Rc::new({
        let (items, resident, current, sizes, typed_s, titles_s) = (
            items.clone(),
            resident.clone(),
            current.clone(),
            sizes.clone(),
            typed.clone(),
            titles.clone(),
        );
        let (sync_menu, owners, host_cx, selection, pres) = (
            sync_menu.clone(),
            owners.clone(),
            host_cx.clone(),
            selection.clone(),
            presentation_cell.clone(),
        );
        let (list_pred_s, list_in_stack_s) = (list_pred.clone(), list_in_stack.clone());
        let apply_list_visible_s = apply_list_visible.clone();
        let compose_s = compose.clone();
        move |key: &str| {
            if current.borrow().as_deref() == Some(key) {
                return;
            }
            let chrome = pres.get().rows_are_chrome();
            // The list interposes between the sidebar root and the detail exactly while a
            // merged-pane backend is STACKED and the app gates the detail
            // (docs/navigation.md). Split-family presentations show the detail beside the
            // list, empty state and all.
            let gated =
                merged_list && pres.get() == NavPresentation::Stack && detail_visible.is_some();
            if !chrome {
                // Stacked or split: the outgoing page goes away. Dispose its scope FIRST — a
                // merged inner stack's cleanup pops its pages (which sit on top natively) before
                // we pop the detail itself, so the native pop order stays top-down (iOS pops the
                // topmost VC; Android's INCLUSIVE pop unwinds everything above an entry).
                if let Some(p) = resident.borrow_mut().pop() {
                    p.scope.dispose();
                    with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                    owners.borrow_mut().pop();
                    sizes.borrow_mut().remove(&p.node);
                    with_tree(|t| {
                        t.remove_subtree(p.node);
                        t.mark_layout_dirty();
                        t.layout_if_needed();
                    });
                }
            }
            *current.borrow_mut() = None;
            if key.is_empty() {
                // Deselected: the list layer (if interposed) leaves the stack with the detail.
                if list_in_stack_s.get() {
                    list_in_stack_s.set(false);
                    with_tree(|t| t.patch(host, Box::new(NavPatch::ListInStack(false)), false));
                    owners.borrow_mut().pop();
                }
                sync_menu(None);
                return;
            }
            let idx = typed_s.borrow().iter().position(|k| k.key() == key);
            let Some(idx) = idx else {
                sync_menu(None);
                return;
            };
            let typed_key_now = typed_s.borrow()[idx].clone();
            // The pane's visibility is settled for EVERY destination change, including the
            // resident case below — a chrome presentation builds every destination, so the
            // last one built would otherwise be the one that had the final word.
            apply_list_visible_s(&typed_key_now);
            // Already built and still alive — the resident case. Nothing to build, nothing to
            // push: tell the host which of its pages to show and we are done.
            if let Some(i) = resident.borrow().iter().position(|p| p.key == key) {
                with_tree(|t| {
                    t.patch(host, Box::new(NavPatch::Select(i)), false);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
                *current.borrow_mut() = Some(key.to_string());
                sync_menu(Some(idx));
                return;
            }
            let typed_key = typed_key_now;
            let title_now = titles_s.borrow()[idx].clone();
            // Per-destination pane visibility (`content_list_for`): a full-width destination
            // collapses the pane, a list-backed one brings it back. `interposed` = this
            // destination puts the LIST between the sidebar root and its detail (a stacked
            // merged-pane backend, docs/navigation.md).
            let want = native_list && list_pred_s.as_ref().is_none_or(|p| p(&typed_key));
            let interposed = gated && want;
            if native_list {
                if interposed && !list_in_stack_s.get() {
                    // Interpose the list above the sidebar root, with its own back owner
                    // (back from the list = deselect).
                    list_in_stack_s.set(true);
                    with_tree(|t| t.patch(host, Box::new(NavPatch::ListInStack(true)), false));
                    let s = selection.clone();
                    owners.borrow_mut().push(Rc::new(move |_already_popped| {
                        if let Some(root) = K::from_key("") {
                            s.write(root);
                        }
                    }) as PopOwner);
                } else if gated && !want && list_in_stack_s.get() {
                    // A full-width destination while stacked: the list leaves the stack so its
                    // page sits directly on the sidebar root and back deselects.
                    list_in_stack_s.set(false);
                    with_tree(|t| t.patch(host, Box::new(NavPatch::ListInStack(false)), false));
                    owners.borrow_mut().pop();
                }
                // The detail waits for `detail_visible`: the list is the top of the stack
                // until the app opens a row. The selection is recorded as current — the
                // `detail_visible` bind re-enters here to perform the deferred push.
                if interposed && !detail_visible.expect("gated implies Some").peek() {
                    *current.borrow_mut() = Some(key.to_string());
                    sync_menu(Some(idx));
                    return;
                }
            }
            // A static item retitles on locale change (its TextSource); a data-driven key uses
            // the resolved snapshot (its title tracks the items signal, not the locale).
            let retitle = items.static_title(key);
            let page = nav_page(
                host,
                &NavPageProps {
                    title: title_now.clone(),
                    pane: Pane::Detail,
                },
                &sizes,
            );
            if !chrome {
                // The detail page's back action: with the list interposed, back from the
                // detail returns TO THE LIST (`detail_visible` := false); classically it
                // deselects (returns to the sidebar rows). Pushed BEFORE the content builds,
                // so a merged inner stack's page owners stack on top of it. A chrome
                // presentation has no back stack to own — a tab bar never pops.
                let owner: PopOwner = if interposed && let Some(dv) = detail_visible {
                    Rc::new(move |_already_popped| {
                        dv.set(false);
                    })
                } else {
                    let s = selection.clone();
                    Rc::new(move |_already_popped| {
                        if let Some(root) = K::from_key("") {
                            s.write(root);
                        }
                    })
                };
                owners.borrow_mut().push(owner);
            }
            let scope = nav_scope.enter(Scope::child);
            let content = match &compose_s {
                Some(c) => c(&typed_key),
                None => items.build_page(&typed_key),
            };
            scope.enter(|| {
                // A resident page is a merge BARRIER: a `stack` inside a tab keeps its own native
                // container rather than pushing onto the enclosing host, because the enclosing
                // host is not a stack (docs/navigation.md).
                let inner = if chrome { None } else { Some(host_cx.clone()) };
                let build = || {
                    with_nav_host(inner, || {
                        let mut c = BuildCx::new(page);
                        let _ = content.build(&mut c);
                    });
                };
                if chrome {
                    // Resident, so anything this page builds outlives the switch away from it and
                    // has to say whether it is the page on screen. Reads `current` LIVE: the build
                    // itself runs before this page becomes current, and it changes on every switch.
                    let (cur, mine) = (current.clone(), key.to_string());
                    with_page_active(
                        Rc::new(move || cur.borrow().as_deref() == Some(mine.as_str())),
                        build,
                    );
                } else {
                    build();
                }
            });
            resident.borrow_mut().push(ResidentPage {
                key: key.to_string(),
                scope,
                node: page,
            });
            let at = resident.borrow().len() - 1;
            // Built BEFORE the borrow, deliberately. `immersive_of` scans the item sources, and a
            // data-driven `.items(…)` block is APP code — running it inside `with_tree` re-enters
            // the tree borrow the moment that closure reads anything ambient (`day::size_class()`
            // is the obvious one) and aborts the process. `with_tree`'s contract is that tree
            // methods never run user code; constructing the patch out here is what keeps it true.
            let patch: Box<dyn std::any::Any> = if chrome {
                Box::new(NavPatch::Select(at))
            } else {
                Box::new(NavPatch::Pushed {
                    title: title_now,
                    immersive: items.immersive_of(&typed_key.key()),
                })
            };
            with_tree(|t| {
                t.patch(host, patch, false);
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
            *current.borrow_mut() = Some(key.to_string());
            sync_menu(Some(idx));
        }
    });

    // Neither a split nor a tab bar can draw "nothing selected": a split has no way to fill the
    // detail pane, and a tab bar always has one tab active. Both default to the first item. Only
    // a STACK has an empty state, and there it is the whole point — the collapsed list the user
    // has not chosen from yet. A host with a native content list joins the default-selection
    // rule at every presentation: the pane needs a selection to scope itself to, and a
    // collapsed merged-pane host opens on the list rather than on bare rows.
    if (split || presentation.rows_are_chrome() || native_list)
        && selection.peek().key().is_empty()
        && let Some(k) = typed.borrow().first().cloned()
    {
        selection.write(k);
    }
    {
        let (s, show) = (selection.clone(), show.clone());
        bind(move || s.read().key(), move |key: &String| show(key));
    }

    // `detail_visible`, both directions (docs/navigation.md). Only the stacked merged-pane
    // shape reacts here: `true` performs the deferred detail push for the current selection,
    // `false` pops back to the interposed list. Split-family presentations ignore the signal —
    // their detail pane is always on screen (native back never runs a detail owner there).
    if let Some(dv) = detail_visible {
        let (show_dv, current_dv, resident_dv, sizes_dv, owners_dv) = (
            show.clone(),
            current.clone(),
            resident.clone(),
            sizes.clone(),
            owners.clone(),
        );
        let (pres_dv, list_in_stack_dv) = (presentation_cell.clone(), list_in_stack.clone());
        bind(
            move || dv.get(),
            move |v: &bool| {
                let gated_now = merged_list
                    && pres_dv.get() == NavPresentation::Stack
                    && list_in_stack_dv.get();
                if !gated_now {
                    return;
                }
                if *v {
                    let key = current_dv.borrow().clone();
                    if resident_dv.borrow().is_empty()
                        && let Some(k) = key
                    {
                        // Deferred push: `show` re-runs for the recorded selection, sees the
                        // signal true, and performs the build it withheld.
                        *current_dv.borrow_mut() = None;
                        show_dv(&k);
                    }
                } else if let Some(p) = resident_dv.borrow_mut().pop() {
                    p.scope.dispose();
                    with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                    owners_dv.borrow_mut().pop();
                    sizes_dv.borrow_mut().remove(&p.node);
                    with_tree(|t| {
                        t.remove_subtree(p.node);
                        t.mark_layout_dirty();
                        t.layout_if_needed();
                    });
                }
            },
        );
    }

    // Build every destination, then re-select the current one.
    //
    // A tab bar needs an ITEM PER DESTINATION up front — `UITabBarController` and Material's
    // navigation bar both build their chrome from the full set, so a page nobody has visited yet
    // is a tab that simply is not there. Lazy building is right for a split or a stack, where
    // only the shown page is drawn; where the rows ARE the chrome, the rows have to be complete.
    let build_all = {
        let (show, typed_a, sel_a) = (show.clone(), typed.clone(), selection.clone());
        Rc::new(move || {
            let keys: Vec<String> = typed_a.borrow().iter().map(|k| k.key()).collect();
            for k in keys {
                show(&k);
            }
            show(&sel_a.peek().key());
        })
    };
    if presentation.rows_are_chrome() {
        build_all();
    }

    // What a presentation change means for the MODEL, whoever caused it. Widening with nothing
    // selected would leave the detail pane empty, the one state a split presentation has no way
    // to draw — adopt the same first-item rule the build uses. Narrowing keeps the selection
    // instead: the detail simply becomes the top of the stack, which is where the user already
    // was.
    let reconcile = {
        let (pc, sel_r, typed_r, split_r) = (
            presentation_cell.clone(),
            selection.clone(),
            typed.clone(),
            split_cell.clone(),
        );
        let build_all_r = build_all.clone();
        let (resident_r, current_r, sizes_r, owners_r) = (
            resident.clone(),
            current.clone(),
            sizes.clone(),
            owners.clone(),
        );
        let (list_pred_r, list_in_stack_r, show_r) =
            (list_pred.clone(), list_in_stack.clone(), show.clone());
        let apply_list_visible_r = apply_list_visible.clone();
        Rc::new(move |next: NavPresentation| {
            let was_chrome = pc.get().rows_are_chrome();
            pc.set(next);
            split_r.set(next.is_split());
            presentation_sig.set(next);
            // LEAVING a chrome presentation: the resident pages nobody can see any more go away,
            // because a split or stacked host draws exactly one detail page. The SHOWN page is
            // kept — rebuilding what the user is looking at is the one thing a morph must never
            // do — and it is left as the sole entry, so it reads as the top of the new stack.
            //
            // Entering a chrome presentation needs no counterpart: the shown page is already
            // resident, and the others build lazily when they are first selected.
            if was_chrome && !next.rows_are_chrome() {
                let shown = current_r.borrow().clone();
                let gone: Vec<ResidentPage> = {
                    let mut r = resident_r.borrow_mut();
                    let (keep, drop): (Vec<_>, Vec<_>) =
                        r.drain(..).partition(|p| Some(&p.key) == shown.as_ref());
                    *r = keep;
                    drop
                };
                for p in gone {
                    p.scope.dispose();
                    sizes_r.borrow_mut().remove(&p.node);
                    with_tree(|t| t.remove_subtree(p.node));
                }
                // The kept page now owns the one back affordance the stack presentation offers.
                owners_r.borrow_mut().clear();
                if let Some(k) = shown {
                    let s = sel_r.clone();
                    let owner: PopOwner = Rc::new(move |_already_popped| {
                        if let Some(root) = K::from_key("") {
                            s.write(root);
                        }
                    });
                    owners_r.borrow_mut().push(owner);
                    let _ = k;
                }
                with_tree(|t| {
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
            if !was_chrome && next.rows_are_chrome() {
                // Entering a chrome presentation: the rows become the chrome, so every
                // destination needs its page for the bar to be complete.
                build_all_r();
            }
            if (next.is_split() || next.rows_are_chrome() || native_list)
                && sel_r.peek().key().is_empty()
                && let Some(k) = typed_r.borrow().first().cloned()
            {
                sel_r.write(k);
            }
            // Content-list settlement (docs/navigation.md). The pane's own visibility comes
            // first and applies to EVERY backend that places the pane itself: entering a chrome
            // presentation builds every destination, and a destination without a list collapses
            // the pane on its way past, so the shape that comes next has to state the selected
            // destination's answer again.
            if native_list
                && let Some(k) = typed_r
                    .borrow()
                    .iter()
                    .find(|x| Some(&x.key()) == current_r.borrow().as_ref())
                    .cloned()
            {
                apply_list_visible_r(&k);
            }
            // Narrowing into the gated stack interposes the list for the current selection and
            // retracts a detail the app is not showing; widening back restores the always-on
            // detail beside the pane. The owner stack is REBUILT for the new shape — the old
            // owners named the old one.
            if merged_list && let Some(dv) = detail_visible {
                if next == NavPresentation::Stack {
                    let key = current_r.borrow().clone();
                    let want = key.as_ref().is_some_and(|k| {
                        typed_r
                            .borrow()
                            .iter()
                            .find(|x| &x.key() == k)
                            .is_some_and(|tk| list_pred_r.as_ref().is_none_or(|p| p(tk)))
                    });
                    if want {
                        if !list_in_stack_r.get() {
                            list_in_stack_r.set(true);
                            with_tree(|t| {
                                t.patch(host, Box::new(NavPatch::ListInStack(true)), false)
                            });
                        }
                        owners_r.borrow_mut().clear();
                        let s = sel_r.clone();
                        owners_r.borrow_mut().push(Rc::new(move |_already_popped| {
                            if let Some(root) = K::from_key("") {
                                s.write(root);
                            }
                        }) as PopOwner);
                        if dv.peek() {
                            // The detail stays on top of the interposed list; its back now
                            // returns to the list, not to the rows.
                            owners_r.borrow_mut().push(Rc::new(move |_already_popped| {
                                dv.set(false);
                            }) as PopOwner);
                        } else if let Some(p) = resident_r.borrow_mut().pop() {
                            // The app is not showing a detail: the list is the top.
                            p.scope.dispose();
                            with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                            sizes_r.borrow_mut().remove(&p.node);
                            with_tree(|t| {
                                t.remove_subtree(p.node);
                                t.mark_layout_dirty();
                                t.layout_if_needed();
                            });
                        }
                    }
                } else if next.is_split() {
                    // The backend's expand lifted the list back into its own pane.
                    list_in_stack_r.set(false);
                    let key = current_r.borrow().clone();
                    if resident_r.borrow().is_empty()
                        && let Some(k) = key
                    {
                        // The detail was withheld while gated; a split shows it always.
                        *current_r.borrow_mut() = None;
                        show_r(&k);
                    } else if !resident_r.borrow().is_empty() {
                        owners_r.borrow_mut().clear();
                        let s = sel_r.clone();
                        owners_r.borrow_mut().push(Rc::new(move |_already_popped| {
                            if let Some(root) = K::from_key("") {
                                s.write(root);
                            }
                        }) as PopOwner);
                    }
                }
            }
        })
    };

    if requested.is_none() && can_split && we_drive {
        // WE drive (docs/size-classes.md). Seeded with what we just built, so the first run is a
        // no-op; after that a class change patches the LIVE host and the backend re-homes the
        // pages it already has. Nothing here rebuilds a page, which is what keeps scroll offsets,
        // field focus, and the search query across the morph.
        let rec = reconcile.clone();
        bind_seeded(
            presentation,
            move || resolve(day_core::window_size_class(window)),
            move |next: &NavPresentation| {
                with_tree(|t| {
                    t.patch(host, Box::new(NavPatch::Presentation(*next)), false);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
                rec(*next);
            },
        );
    } else if toolkit_drives {
        // The TOOLKIT drives: its own adaptive container already morphed and is telling us after
        // the fact, so there is nothing to patch — only the model to reconcile. Pushing a
        // presentation at it instead would be a second source of truth racing the platform's own
        // collapse animation.
        let rec = reconcile.clone();
        cx.on(host, move |ev| {
            if let Event::NavPresentationChanged(next) = ev {
                rec(*next);
                with_tree(|t| {
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        });
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
        // The LIVE presentation, not the build-time one: this effect outlives a morph, and what
        // "must never show an empty detail" means changes with it.
        let (show_e, pres_e) = (show.clone(), presentation_cell.clone());
        let (resident_e, current_e, sizes_e) = (resident.clone(), current.clone(), sizes.clone());
        bind(
            move || {
                // TRACKED derive: subscribes to every dynamic block's signal.
                let NavRows {
                    keys: k,
                    titles: t,
                    icons: i,
                    badges: b,
                    badge_icons: bi,
                    badge_tints: bt,
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
                    bi,
                    bt,
                )
            },
            move |(key_strs, keys, ts, ics, bs, scs, tns, mns, bis, bts): &DerivedRows<K>| {
                *typed_e.borrow_mut() = keys.clone();
                *titles_e.borrow_mut() = ts.clone();
                // A resident page whose row is gone has nothing left to select it, so it would
                // sit alive and invisible for the life of the surface — and shift every
                // `NavPatch::Select` index past it. Drop it here, where the new row set is known.
                // Only reachable in a chrome presentation; elsewhere at most one page is resident
                // and the selection reset below takes care of it.
                {
                    let stale: Vec<ResidentPage> = {
                        let mut r = resident_e.borrow_mut();
                        let (keep, drop): (Vec<_>, Vec<_>) =
                            r.drain(..).partition(|p| key_strs.contains(&p.key));
                        *r = keep;
                        drop
                    };
                    if !stale.is_empty() {
                        if current_e
                            .borrow()
                            .as_ref()
                            .is_some_and(|c| stale.iter().any(|p| &p.key == c))
                        {
                            *current_e.borrow_mut() = None;
                        }
                        for p in stale {
                            p.scope.dispose();
                            sizes_e.borrow_mut().remove(&p.node);
                            with_tree(|t| t.remove_subtree(p.node));
                        }
                        with_tree(|t| {
                            t.mark_layout_dirty();
                            t.layout_if_needed();
                        });
                    }
                }
                // If the selected key is gone, reset (Option key → None); else keep it selected.
                let cur = sel_e.peek().key();
                let still = cur.is_empty() || key_strs.iter().any(|k| k == &cur);
                if !still && let Some(root) = K::from_key("") {
                    sel_e.write(root);
                }
                // A split view must never show an empty detail. The build-time fallback ran
                // once, before any filtering, so re-apply it here: when the selected row is
                // gone, move to the first row that survived rather than blanking the pane.
                let mut cur2 = sel_e.peek().key();
                let pres_now = pres_e.get();
                if (pres_now.is_split() || pres_now.rows_are_chrome() || native_list)
                    && cur2.is_empty()
                    && let Some(k) = keys.first().cloned()
                {
                    sel_e.write(k.clone());
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
                                badge_icons: bis.clone(),
                                badge_tints: bts.clone(),
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
            Event::RouteRequested(route) => {
                let _ = day_core::navigate(route);
            }
            _ => {}
        });
    }

    // Registered AFTER the pages are built, deliberately: `register_nav` drains any PENDING
    // route as it registers, and a route arriving before this host has children attaches a
    // page to a parent that is not in the tree yet (an intermittent startup panic on AppKit).
    // Ordering here does not decide routing — `NavController::depth` does.
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
                        s_push.write(root);
                        true
                    } else {
                        false // no empty state (bare-enum key) — let the parent handle ""
                    }
                } else if let Some(key) = pick_push(&tp_push, k) {
                    s_push.write(key);
                    true
                } else {
                    false
                }
            },
            move |_| {
                if s_pop.peek().key().is_empty() {
                    false
                } else if let Some(root) = K::from_key("") {
                    s_pop.write(root);
                    true
                } else {
                    false
                }
            },
            move || s_cur.peek().key(),
            // Absolute-path segment: a declared item key selects it (no "" — segments are non-empty).
            move |k| {
                if let Some(key) = pick_enter(&tp_enter, k) {
                    s_enter.write(key);
                    true
                } else {
                    false
                }
            },
            move || {
                let k = s_seg.peek().key();
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
pub struct Stack<S: Binding<Vec<K>>, K: Route = String> {
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
    bar_actions: Vec<BarActionSpec>,
}

pub fn stack<K: Route, S: Binding<Vec<K>>>(path: S, root: impl Piece) -> Stack<S, K> {
    Stack {
        path,
        title: TextSource::Static(String::new()),
        root: AnyPiece::new(root),
        destination: Rc::new(|_| {
            piece_fn(|cx| cx.layout_only(Rc::new(PassThrough), Flex::default(), Boundary::No)).any()
        }),
        on_back: None,
        restore: None,
        bar_actions: Vec::new(),
    }
}

impl<K: Route, S: Binding<Vec<K>>> Stack<S, K> {
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text();
        self
    }
    /// Add a trailing action button to the navigation bar, for the toolkits with no window toolbar
    /// (the phones and HarmonyOS): an upper-right bar button drawn with the bundled `icon` that
    /// runs `action` (docs/navigation.md). Mirrors [`Selector::bar_action`]; ignored on desktop.
    ///
    /// Call it more than once for more than one button; they draw left to right in declaration
    /// order. Use [`list_action`](Self::list_action) for a command that acts on the stack's ROOT
    /// rather than on whatever page is on top of it.
    pub fn bar_action<M>(
        mut self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.bar_actions.push(BarActionSpec {
            icon: Some(icon.into().as_str().to_owned()),
            label: label.into_text(),
            action: Rc::new(action),
            scope: day_spec::props::NavBarScope::EveryPage,
        });
        self
    }

    /// Like [`bar_action`](Self::bar_action), but the button rides the stack's ROOT page only — it
    /// is gone from everything pushed on top of it. Mirrors [`Selector::list_action`], and the
    /// same rule decides between them: a command that acts on the root's content (adding to the
    /// list it shows, filtering it) has nothing to act on once a detail covers it.
    pub fn list_action<M>(
        mut self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.bar_actions.push(BarActionSpec {
            icon: Some(icon.into().as_str().to_owned()),
            label: label.into_text(),
            action: Rc::new(action),
            scope: day_spec::props::NavBarScope::RootPage,
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

impl<K: Route, S: Binding<Vec<K>>> Piece for Stack<S, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::props::{NavPageProps, NavPatch, NavPresentation, NavProps, Pane};
        let Stack {
            path,
            title,
            root,
            destination: dest,
            on_back,
            restore,
            bar_actions,
        } = self;
        let title_s = title.initial();
        // Lower the optional nav-bar action for the standalone host below (a merged stack rides
        // the enclosing host's bar instead). Registered once; the mobile backends draw it.
        let bar_actions: Vec<_> = bar_actions.into_iter().map(BarActionSpec::lower).collect();

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
                path.write(v);
            }
        }

        // If we're built inside a page of an enclosing NAV host that presents as a push stack
        // (mobile, `split == false`), MERGE: push our pages onto that host instead of minting a
        // second native container — one native nav chain, one back button (docs/navigation.md).
        // A split host (desktop) is not merged into; a stack keeps its own detail-pane stack.
        let merge = current_nav_host().filter(|c| !c.split.get());

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
            #[cfg(debug_assertions)]
            if !bar_actions.is_empty() {
                warn_merged_bar_actions(bar_actions.len());
            }
        } else {
            // STANDALONE: create the native host + root page (an app-root stack, or a nested stack
            // under a split/desktop host).
            sizes = Rc::default();
            host = cx.native(
                kinds::NAV,
                &NavProps {
                    // A stack is a stack at every size: it has no sidebar pane to re-home, so
                    // there is nothing for a size-class change to re-present.
                    title: title_s.clone(),
                    presentation: NavPresentation::Stack,
                    // Never adaptive: the `stack()` piece is a push/pop surface at every size, so
                    // an Emulated toolkit must build a PLAIN navigation container for it rather
                    // than its adaptive one (docs/size-classes.md).
                    adaptive: false,
                    bar_actions,
                    // Stacks are not searchable yet — `.searchable()` is on `Selector` only
                    // (docs/search.md); a stack gains the same surface when the placement
                    // resolver lands, since it is the same lowering.
                    search: None,
                    // A content list is a selector shape; a stack has no sidebar to sit beside.
                    list_width: None,
                },
                Rc::new(NavLayout::stack(sizes.clone())),
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
                split: Rc::new(Cell::new(false)),
            };
            let root_page = nav_page(
                host,
                &NavPageProps {
                    title: title_s,
                    pane: Pane::Detail,
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
                let mut v = p.peek();
                if v.pop().is_some() {
                    p.write(v);
                }
            })
        };
        let depth = {
            let p = path.clone();
            move || p.peek().len()
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
                            pane: Pane::Detail,
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
            bind(move || p.read(), move |want: &Vec<K>| reconcile(want));
        }

        // Persist the path across launches when `.restore` is set: save the keys in the same
        // percent-encoded wire format `parse_route` reads back (`encode_route`), so a key
        // containing `/` survives the round-trip (docs/navigation.md). Scope-owned, so it stops
        // with the stack.
        if let Some(key) = restore {
            let p = path.clone();
            bind(
                move || {
                    let keys: Vec<String> = p.read().iter().map(|k| k.key()).collect();
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
                Event::RouteRequested(route) => {
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
                    let mut v = p_push.peek();
                    if v.is_empty() {
                        return false; // already at root — let the parent handle ""
                    }
                    v.clear();
                    p_push.write(v);
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
            move || p_cur.peek().last().map(|k| k.key()).unwrap_or_default(),
            move |k| {
                let Some(parsed) = K::from_key(k) else {
                    return false; // not one of this stack's routes — leave it queued
                };
                let mut v = p_enter.peek();
                v.push(parsed);
                p_enter.write(v);
                true
            },
            move || p_seg.peek().iter().map(|k| k.key()).collect(),
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
    routed: bool,
    _marker: std::marker::PhantomData<R>,
}

/// A fullscreen cover over `open`: `Some(r)` presents `build(&r)`, `None` dismisses
/// (docs/cover.md). Registers a string-route adapter, so `navigate("<key>")` opens it and
/// `nav_back()` closes it, and `current_route()` reports the presented key.
pub fn cover<R: Route, S: Binding<Option<R>>, P: Piece>(
    open: S,
    build: impl Fn(&R) -> P + 'static,
) -> Cover<S, R> {
    Cover {
        open,
        // The stored builder is erased because a `Cover` holds one closure for every route it
        // presents; the PARAMETER stays generic so callers never write `.any()` for us.
        build: Rc::new(move |r| AnyPiece::new(build(r))),
        background: None,
        routed: true,
        _marker: std::marker::PhantomData,
    }
}

impl<S: Binding<Option<R>>, R: Route> Cover<S, R> {
    /// The surface color painted edge-to-edge behind the content (under the status bar and
    /// home indicator) while `r` is presented. Without it the platform's default surface
    /// color shows in the unsafe areas.
    pub fn background(mut self, f: impl Fn(&R) -> day_spec::Color + 'static) -> Self {
        self.background = Some(Rc::new(f));
        self
    }

    /// Keep this cover OUT of the app's route space: no `navigate("<key>")` to present it, no
    /// contribution to `current_route()`, and `nav_back()` walks past it.
    ///
    /// For a cover that is a **control's own panel** rather than an app destination — a color
    /// picker's chooser, a media scrubber's fullscreen mode — presented and dismissed by the
    /// control, never linked to. Two reasons that matters:
    ///
    /// - **A routed cover claims route segments**, and over the untyped `Route` (`String`,
    ///   whose `from_key` accepts anything) it claims *every* segment: mount one and the next
    ///   `navigate("settings")` presents the cover with the key `"settings"` instead of going to
    ///   settings. A piece that mounts a cover would be silently rewriting its host app's
    ///   navigation.
    /// - **It is not a place.** `current_route()` naming a transient chooser makes a restored
    ///   session reopen it, and a "share this screen" link point at a modal.
    ///
    /// Interactive dismissal is unaffected: the cover still answers Android's system back through
    /// [`Event::NavBack`](day_spec::Event::NavBack) on its own node, which never went through the
    /// route adapter.
    pub fn unrouted(mut self) -> Self {
        self.routed = false;
        self
    }
}

impl<S: Binding<Option<R>>, R: Route> Piece for Cover<S, R> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::props::{CoverPatch, CoverProps};
        let Cover {
            open,
            build,
            background,
            routed,
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
        // (content stays mounted until the backend reports `CoverHidden`, so the surface
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
            bind(move || o.read(), move |want: &Option<R>| reconcile(want));
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
                    if !day_core::shield::dismiss_disabled() && o.peek().is_some() {
                        o.write(None);
                    }
                }
                // The hide transition finished — now the content can go.
                // Idempotent + orderable (docs/cover.md): duplicates and belated reports
                // from a previous dismissal are no-ops via the closing gate.
                Event::CoverHidden if closing.get() => {
                    closing.set(false);
                    dispose_content();
                }
                _ => {}
            });
        }

        // String-route adapter (docs/navigation.md): `navigate("<key>")` presents, `nav_back()`
        // dismisses, and the presented key is this surface's `current_route()` contribution.
        // Skipped for an `unrouted()` cover, which is a control's own panel rather than a place
        // the app navigates to — see `Cover::unrouted` for why that distinction has teeth.
        if !routed {
            return node;
        }
        let o_push = open.clone();
        let o_pop = open.clone();
        let o_cur = open.clone();
        let o_enter = open.clone();
        let o_seg = open;
        let push = move |k: &str, sig: &S| match R::from_key(k) {
            Some(r) => {
                sig.write(Some(r));
                true
            }
            None => false,
        };
        let push2 = push;
        register_route_surface(
            move |k| push(k, &o_push),
            move |_| {
                if o_pop.peek().is_some() {
                    o_pop.write(None);
                    true
                } else {
                    false
                }
            },
            move || o_cur.peek().map(|r| r.key()).unwrap_or_default(),
            move |k| push2(k, &o_enter),
            move || o_seg.peek().map(|r| vec![r.key()]).unwrap_or_default(),
        );

        node
    }
}

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// [`Selector`]'s own builders, reachable THROUGH a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait SelectorBuilder<K: Route>: Sized {
    fn style(self, style: SelectorStyle) -> Self;
    fn title<M>(self, t: impl IntoText<M>) -> Self;
    fn section<M>(self, title: impl IntoText<M>) -> Self;
    fn badge<M>(self, badge: impl IntoText<M>) -> Self;
    fn badge_icon(self, icon: impl Into<day_spec::ImageName>) -> Self;
    fn badge_tint(self, color: day_spec::Color) -> Self;
    fn header<P: Piece>(self, build: impl FnOnce() -> P + 'static) -> Self;
    fn presentation(self, presentation: day_spec::props::NavPresentation) -> Self;
    fn item<M, P: Piece>(
        self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        build: impl Fn() -> P + 'static,
    ) -> Self;
    fn item_icon<M, P: Piece>(
        self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        icon: impl Into<day_spec::ImageName>,
        build: impl Fn() -> P + 'static,
    ) -> Self;
    fn immersive(self) -> Self;
    fn icon_tint(self, color: day_spec::Color) -> Self;
    fn items<T: Clone + 'static>(
        self,
        items: impl Fn() -> Vec<T> + 'static,
        map: impl Fn(&T) -> NavItem<K> + 'static,
    ) -> Self;
    fn destination<P: Piece>(self, build: impl Fn(&K) -> P + 'static) -> Self;
    fn local(self) -> Self;
    fn restore(self, key: impl Into<String>) -> Self;
    fn bar_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self;
    fn list_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self;
    fn searchable(self, query: Signal<String>) -> Self;
    fn search_prompt<M>(self, prompt: impl IntoText<M>) -> Self;
    fn search_placement(self, placement: day_spec::props::SearchPlacement) -> Self;
    fn search_scopes<M>(self, scope: Signal<usize>, titles: Vec<impl IntoText<M>>) -> Self;
    fn search_suggestions(self, f: impl Fn(&str) -> Vec<String> + 'static) -> Self;
}

impl<K: Route, S: Binding<K>> SelectorBuilder<K> for Selector<S, K> {
    fn style(self, style: SelectorStyle) -> Self {
        Selector::style(self, style)
    }
    fn title<M>(self, t: impl IntoText<M>) -> Self {
        Selector::title(self, t)
    }
    fn section<M>(self, title: impl IntoText<M>) -> Self {
        Selector::section(self, title)
    }
    fn badge<M>(self, badge: impl IntoText<M>) -> Self {
        Selector::badge(self, badge)
    }
    fn badge_icon(self, icon: impl Into<day_spec::ImageName>) -> Self {
        Selector::badge_icon(self, icon)
    }
    fn badge_tint(self, color: day_spec::Color) -> Self {
        Selector::badge_tint(self, color)
    }
    fn header<P: Piece>(self, build: impl FnOnce() -> P + 'static) -> Self {
        Selector::header(self, build)
    }
    fn presentation(self, presentation: day_spec::props::NavPresentation) -> Self {
        Selector::presentation(self, presentation)
    }
    fn item<M, P: Piece>(
        self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        Selector::item(self, key, title, build)
    }
    fn item_icon<M, P: Piece>(
        self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        icon: impl Into<day_spec::ImageName>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        Selector::item_icon(self, key, title, icon, build)
    }
    fn immersive(self) -> Self {
        Selector::immersive(self)
    }
    fn icon_tint(self, color: day_spec::Color) -> Self {
        Selector::icon_tint(self, color)
    }
    fn items<T: Clone + 'static>(
        self,
        items: impl Fn() -> Vec<T> + 'static,
        map: impl Fn(&T) -> NavItem<K> + 'static,
    ) -> Self {
        Selector::items(self, items, map)
    }
    fn destination<P: Piece>(self, build: impl Fn(&K) -> P + 'static) -> Self {
        Selector::destination(self, build)
    }
    fn local(self) -> Self {
        Selector::local(self)
    }
    fn restore(self, key: impl Into<String>) -> Self {
        Selector::restore(self, key)
    }
    fn bar_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        Selector::bar_action(self, icon, label, action)
    }
    fn list_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        Selector::list_action(self, icon, label, action)
    }
    fn searchable(self, query: Signal<String>) -> Self {
        Selector::searchable(self, query)
    }
    fn search_prompt<M>(self, prompt: impl IntoText<M>) -> Self {
        Selector::search_prompt(self, prompt)
    }
    fn search_placement(self, placement: day_spec::props::SearchPlacement) -> Self {
        Selector::search_placement(self, placement)
    }
    fn search_scopes<M>(self, scope: Signal<usize>, titles: Vec<impl IntoText<M>>) -> Self {
        Selector::search_scopes(self, scope, titles)
    }
    fn search_suggestions(self, f: impl Fn(&str) -> Vec<String> + 'static) -> Self {
        Selector::search_suggestions(self, f)
    }
}

impl<K: Route, Inner: SelectorBuilder<K> + Piece> SelectorBuilder<K> for Decorated<Inner> {
    fn style(self, style: SelectorStyle) -> Self {
        self.map_inner(|inner_piece| inner_piece.style(style))
    }
    fn title<M>(self, t: impl IntoText<M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.title(t))
    }
    fn section<M>(self, title: impl IntoText<M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.section(title))
    }
    fn badge<M>(self, badge: impl IntoText<M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.badge(badge))
    }
    fn badge_icon(self, icon: impl Into<day_spec::ImageName>) -> Self {
        self.map_inner(|inner_piece| inner_piece.badge_icon(icon))
    }
    fn badge_tint(self, color: day_spec::Color) -> Self {
        self.map_inner(|inner_piece| inner_piece.badge_tint(color))
    }
    fn header<P: Piece>(self, build: impl FnOnce() -> P + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.header(build))
    }
    fn presentation(self, presentation: day_spec::props::NavPresentation) -> Self {
        self.map_inner(|inner_piece| inner_piece.presentation(presentation))
    }
    fn item<M, P: Piece>(
        self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        self.map_inner(|inner_piece| inner_piece.item(key, title, build))
    }
    fn item_icon<M, P: Piece>(
        self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        icon: impl Into<day_spec::ImageName>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        self.map_inner(|inner_piece| inner_piece.item_icon(key, title, icon, build))
    }
    fn immersive(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.immersive())
    }
    fn icon_tint(self, color: day_spec::Color) -> Self {
        self.map_inner(|inner_piece| inner_piece.icon_tint(color))
    }
    fn items<T: Clone + 'static>(
        self,
        items: impl Fn() -> Vec<T> + 'static,
        map: impl Fn(&T) -> NavItem<K> + 'static,
    ) -> Self {
        self.map_inner(|inner_piece| inner_piece.items(items, map))
    }
    fn destination<P: Piece>(self, build: impl Fn(&K) -> P + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.destination(build))
    }
    fn local(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.local())
    }
    fn restore(self, key: impl Into<String>) -> Self {
        self.map_inner(|inner_piece| inner_piece.restore(key))
    }
    fn bar_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.map_inner(|inner_piece| inner_piece.bar_action(icon, label, action))
    }
    fn list_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.map_inner(|inner_piece| inner_piece.list_action(icon, label, action))
    }
    fn searchable(self, query: Signal<String>) -> Self {
        self.map_inner(|inner_piece| inner_piece.searchable(query))
    }
    fn search_prompt<M>(self, prompt: impl IntoText<M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.search_prompt(prompt))
    }
    fn search_placement(self, placement: day_spec::props::SearchPlacement) -> Self {
        self.map_inner(|inner_piece| inner_piece.search_placement(placement))
    }
    fn search_scopes<M>(self, scope: Signal<usize>, titles: Vec<impl IntoText<M>>) -> Self {
        self.map_inner(|inner_piece| inner_piece.search_scopes(scope, titles))
    }
    fn search_suggestions(self, f: impl Fn(&str) -> Vec<String> + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.search_suggestions(f))
    }
}

/// [`Stack`]'s own builders, reachable THROUGH a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait StackBuilder<K: Route>: Sized {
    fn title<M>(self, t: impl IntoText<M>) -> Self;
    fn bar_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self;
    fn list_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self;
    fn destination<P: Piece>(self, build: impl Fn(&K) -> P + 'static) -> Self;
    fn on_back(self, guard: impl Fn(BackRequest) -> BackResponse + 'static) -> Self;
    fn restore(self, key: impl Into<String>) -> Self;
}

impl<K: Route, S: Binding<Vec<K>>> StackBuilder<K> for Stack<S, K> {
    fn title<M>(self, t: impl IntoText<M>) -> Self {
        Stack::title(self, t)
    }
    fn bar_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        Stack::bar_action(self, icon, label, action)
    }
    fn list_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        Stack::list_action(self, icon, label, action)
    }
    fn destination<P: Piece>(self, build: impl Fn(&K) -> P + 'static) -> Self {
        Stack::destination(self, build)
    }
    fn on_back(self, guard: impl Fn(BackRequest) -> BackResponse + 'static) -> Self {
        Stack::on_back(self, guard)
    }
    fn restore(self, key: impl Into<String>) -> Self {
        Stack::restore(self, key)
    }
}

impl<K: Route, Inner: StackBuilder<K> + Piece> StackBuilder<K> for Decorated<Inner> {
    fn title<M>(self, t: impl IntoText<M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.title(t))
    }
    fn bar_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.map_inner(|inner_piece| inner_piece.bar_action(icon, label, action))
    }
    fn list_action<M>(
        self,
        icon: impl Into<day_spec::ImageName>,
        label: impl IntoText<M>,
        action: impl Fn() + 'static,
    ) -> Self {
        self.map_inner(|inner_piece| inner_piece.list_action(icon, label, action))
    }
    fn destination<P: Piece>(self, build: impl Fn(&K) -> P + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.destination(build))
    }
    fn on_back(self, guard: impl Fn(BackRequest) -> BackResponse + 'static) -> Self {
        self.map_inner(|inner_piece| inner_piece.on_back(guard))
    }
    fn restore(self, key: impl Into<String>) -> Self {
        self.map_inner(|inner_piece| inner_piece.restore(key))
    }
}

/// [`Cover`]'s own builders, reachable THROUGH a decoration (§5.2): `Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait CoverBuilder: Sized {
    fn unrouted(self) -> Self;
}

impl<S: Binding<Option<R>>, R: Route> CoverBuilder for Cover<S, R> {
    fn unrouted(self) -> Self {
        Cover::unrouted(self)
    }
}

impl<Inner: CoverBuilder + Piece> CoverBuilder for Decorated<Inner> {
    fn unrouted(self) -> Self {
        self.map_inner(|inner_piece| inner_piece.unrouted())
    }
}
