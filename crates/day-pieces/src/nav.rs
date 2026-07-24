//! Navigation. Imperative helpers (`navigate`, `nav_back`, `current_route`, `nav_link`); typed
//! routes (the `Route` trait, the `routes!` macro, `RoutePath`); and the host pieces that project
//! an app-owned `Signal` into native navigation — `selector` (tabs/sidebar), `stack` (push/pop),
//! and `cover` (modal) — including nested-stack merging.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, bind, bind_seeded};
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

// ===========================================================================
// Selector — one-of-N, bound to a Signal<String> of the active key.
// ===========================================================================

/// How a [`selector`] presents its one-of-N choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectorStyle {
    /// A native tab widget: NSTabView / UITabBarController / GtkNotebook / QTabWidget /
    /// Android tab strip. All pages resident; each keeps its state.
    Tabs,
    /// A NavigationSplitView: a sidebar list + a detail. Desktop shows both panes (on GTK an
    /// `AdwNavigationSplitView`); mobile collapses to a list that pushes the detail.
    Sidebar,
}

struct SelItem<K> {
    key: K,
    title: TextSource,
    /// Optional bundled-image name for the item's native icon (docs/navigation.md).
    icon: Option<String>,
    build: Box<dyn Fn() -> AnyPiece>,
}

/// A sidebar item resolved for the detail switcher: (encoded key, resolved title, lazy builder).
type ResolvedItems = Rc<Vec<(String, String, Box<dyn Fn() -> AnyPiece>)>>;

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
    items: Vec<SelItem<K>>,
}

pub fn selector<K: Route, S: SignalRw<K>>(selection: S) -> Selector<S, K> {
    Selector {
        selection,
        style: SelectorStyle::Sidebar,
        title: TextSource::Static(String::new()),
        header: None,
        items: Vec::new(),
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
        self.items.push(SelItem {
            key: key.into(),
            title: title.into_text(),
            icon: None,
            build: Box::new(move || AnyPiece::new(build())),
        });
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
        self.items.push(SelItem {
            key: key.into(),
            title: title.into_text(),
            icon: Some(icon.into().as_str().to_owned()),
            build: Box::new(move || AnyPiece::new(build())),
        });
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

fn build_tabs<K: Route, S: SignalRw<K>>(sel: Selector<S, K>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{TabsPageProps, TabsPatch, TabsProps};
    let selection = sel.selection;
    let metas: Vec<(String, String)> = sel
        .items
        .iter()
        .map(|it| (it.key.key(), it.title.initial()))
        .collect();
    let titles: Vec<String> = metas.iter().map(|(_, t)| t.clone()).collect();
    let icons: Vec<Option<String>> = sel.items.iter().map(|it| it.icon.clone()).collect();
    let keys: Rc<Vec<String>> = Rc::new(metas.iter().map(|(k, _)| k.clone()).collect());
    let typed: Rc<Vec<K>> = Rc::new(sel.items.iter().map(|it| it.key.clone()).collect());
    let initial = selection.get_untracked_rw().key();
    let initial_idx = keys.iter().position(|k| *k == initial).unwrap_or(0);

    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let host = cx.native(
        kinds::TABS,
        &TabsProps {
            titles,
            icons,
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
    for (i, it) in sel.items.into_iter().enumerate() {
        let page = tabs_page(
            host,
            &TabsPageProps {
                title: metas[i].1.clone(),
                icon: it.icon.clone(),
            },
            &sizes,
        );
        let content = (it.build)();
        // Barrier: tabs are resident, not a push stack, so a stack inside a tab must not merge
        // through this container into an outer nav host — it keeps its own (docs/navigation.md).
        with_nav_host(None, || {
            let mut pcx = BuildCx::new(page);
            let _ = content.build(&mut pcx);
        });
    }

    // Two-way: signal → native selection (skip the echo of a native tap).
    let echo: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    {
        let (keys, echo, s) = (keys.clone(), echo.clone(), selection.clone());
        bind_seeded(
            initial_idx,
            move || {
                let cur = s.get_rw().key();
                keys.iter().position(|k| *k == cur).unwrap_or(0)
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
        let (typed, echo, s) = (typed.clone(), echo.clone(), selection.clone());
        cx.on(host, move |ev| match ev {
            Event::SelectionChanged(i) if *i >= 0 => {
                let idx = *i as usize;
                if let Some(k) = typed.get(idx) {
                    echo.set(Some(idx));
                    s.set_rw(k.clone());
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
    // string-route adapter (the typed key decodes at this boundary; app code stays typed)
    let (ks_push, ts_push, s_push) = (keys.clone(), typed.clone(), selection.clone());
    let s_cur = selection.clone();
    let (ks_enter, ts_enter, s_enter) = (keys.clone(), typed.clone(), selection.clone());
    let s_seg = selection.clone();
    register_route_surface(
        move |k| {
            if let Some(i) = ks_push.iter().position(|x| x == k) {
                s_push.set_rw(ts_push[i].clone());
                true
            } else {
                false
            }
        },
        |_| false,
        move || s_cur.get_untracked_rw().key(),
        // Absolute-path segment: same as push — a tab key is a declared key.
        move |k| {
            if let Some(i) = ks_enter.iter().position(|x| x == k) {
                s_enter.set_rw(ts_enter[i].clone());
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
    host
}

fn build_sidebar<K: Route, S: SignalRw<K>>(sel: Selector<S, K>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{NavMenuPatch, NavMenuProps, NavPageProps, NavPatch, NavProps};
    let split = with_tree(|t| t.capability(day_spec::Cap::NavSplit)) == day_spec::Support::Native;
    let selection = sel.selection;
    let title_s = sel.title.initial();
    let metas: Vec<(String, String)> = sel
        .items
        .iter()
        .map(|it| (it.key.key(), it.title.initial()))
        .collect();
    let keys: Rc<Vec<String>> = Rc::new(metas.iter().map(|(k, _)| k.clone()).collect());
    let typed: Rc<Vec<K>> = Rc::new(sel.items.iter().map(|it| it.key.clone()).collect());
    let titles: Vec<String> = metas.iter().map(|(_, t)| t.clone()).collect();
    let icons: Vec<Option<String>> = sel.items.iter().map(|it| it.icon.clone()).collect();
    let builders: ResolvedItems = Rc::new(
        sel.items
            .into_iter()
            .enumerate()
            .map(|(i, it)| (metas[i].0.clone(), metas[i].1.clone(), it.build))
            .collect(),
    );

    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let host = cx.native(
        kinds::NAV,
        &NavProps {
            title: title_s.clone(),
            split,
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
        let (mh, ks, s, titles2, icons2) = (
            menu_holder.clone(),
            typed.clone(),
            selection.clone(),
            titles.clone(),
            icons.clone(),
        );
        let menu_piece = piece_fn(move |mcx| {
            let node = mcx.native(
                kinds::NAV_MENU,
                &NavMenuProps {
                    items: titles2,
                    icons: icons2,
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
                    && let Some(k) = ks.get(*i as usize)
                {
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
    let show = {
        let (builders, current, sizes, keys, sync_menu, owners, host_cx, selection) = (
            builders.clone(),
            current.clone(),
            sizes.clone(),
            keys.clone(),
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
            let Some((_, page_title, build)) = builders.iter().find(|(k, _, _)| k == key) else {
                sync_menu(None);
                return;
            };
            let page = nav_page(
                host,
                &NavPageProps {
                    title: page_title.clone(),
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
            let content = build();
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
                        title: page_title.clone(),
                    }),
                    false,
                );
                t.mark_layout_dirty();
                t.layout_if_needed();
            });
            *current.borrow_mut() = Some((key.to_string(), scope, page));
            sync_menu(keys.iter().position(|k| k == key));
        }
    };

    // Desktop split never shows an empty detail: default to the first item.
    if split
        && selection.get_untracked_rw().key().is_empty()
        && let Some(k) = typed.first()
    {
        selection.set_rw(k.clone());
    }
    {
        let s = selection.clone();
        bind(move || s.get_rw().key(), move |key: &String| show(key));
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

    // string-route adapter over `selection` (typed keys decode at this boundary)
    let (ks_push, ts_push, s_push) = (keys.clone(), typed.clone(), selection.clone());
    let s_pop = selection.clone();
    let s_cur = selection.clone();
    let (ks_enter, ts_enter, s_enter) = (keys.clone(), typed.clone(), selection.clone());
    let s_seg = selection.clone();
    register_route_surface(
        move |k| {
            if k.is_empty() {
                if let Some(root) = K::from_key("") {
                    s_push.set_rw(root);
                    true
                } else {
                    false // no empty state (bare-enum key) — let the parent handle ""
                }
            } else if let Some(i) = ks_push.iter().position(|x| x == k) {
                s_push.set_rw(ts_push[i].clone());
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
            if let Some(i) = ks_enter.iter().position(|x| x == k) {
                s_enter.set_rw(ts_enter[i].clone());
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
}

pub fn stack<K: Route, S: SignalRw<Vec<K>>>(path: S, root: impl Piece) -> Stack<S, K> {
    Stack {
        path,
        title: TextSource::Static(String::new()),
        root: AnyPiece::new(root),
        destination: Rc::new(|_| {
            piece_fn(|cx| cx.layout_only(Rc::new(PassThrough), Flex::default(), Boundary::No))
        }),
    }
}

impl<K: Route, S: SignalRw<Vec<K>>> Stack<S, K> {
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text();
        self
    }
    /// Build the view for a pushed key (`&String` for raw keys, the typed value otherwise).
    pub fn destination<P: Piece>(mut self, build: impl Fn(&K) -> P + 'static) -> Self {
        self.destination = Rc::new(move |k| AnyPiece::new(build(k)));
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
        } = self;
        let title_s = title.initial();

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

        // This stack's back owner (one Rc shared by all its pages): bump the native-pop absorb
        // counter when the toolkit already popped, then pop the path.
        let stack_owner: PopOwner = {
            let (p, native_popped) = (path.clone(), native_popped.clone());
            Rc::new(move |already_popped: bool| {
                if already_popped {
                    native_popped.set(native_popped.get() + 1);
                }
                let mut v = p.get_untracked_rw();
                if v.pop().is_some() {
                    p.set_rw(v);
                }
            })
        };

        // Reconcile the native stack to `want`: keep the common prefix, pop the rest, push
        // the new suffix. A pop the native already performed (iOS back) is not re-issued. Pages
        // and owners land on `host` (our own, or the enclosing one when merged).
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
                    with_tree(|t| t.patch(host, Box::new(NavPatch::Pushed { title }), false));
                    owners.borrow_mut().push(stack_owner.clone());
                    entries.borrow_mut().push(StackEntry {
                        key: key.clone(),
                        scope,
                        page,
                    });
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
        let p_pop = path.clone();
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
            move |_| {
                let mut v = p_pop.get_untracked_rw();
                if v.pop().is_some() {
                    p_pop.set_rw(v);
                    true
                } else {
                    false
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
