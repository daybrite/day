// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-spec: the toolkit specification (DESIGN.md §8).
//!
//! Backends depend only on this crate (never on day-core). One backend is linked per binary;
//! `day-core` is monomorphized over the concrete [`Toolkit`].

use std::any::Any;
use std::collections::HashMap;

pub use day_geometry::*;
// `Support` is in `Toolkit`'s signatures and every backend trampoline runs through the guard, so
// a backend reaches both from the contract crate it already depends on. day-process owns them.
pub use day_process::{Support, ffi_guard};
// The link-time registry crate `renderer!` expands to, so a renderer needs no dependency on it.
pub use day_process::linkme;

pub mod controls;
pub use controls::datetime::{DayDate, DayTime, now_epoch_millis, now_epoch_secs};

pub mod applications;
pub mod capture;
pub mod ink;
/// Markdown: inline text to styled runs (docs/text-runs.md) and documents to blocks
/// (docs/markdown-view.md). Lives here rather than in day-pieces because it produces
/// `TextRun`s, and every format codec beside it reads the same model.
pub mod markdown;
pub use ink::{Ink, Palette, SystemColor, resolved_stops};
pub mod render;
pub use render::fill_measure;
/// Bundled-resource random-access API + the per-backend `ResourceOpener` hook (§18.3).
pub mod resource;
/// Styled text: the document model `label().runs(…)` renders and `day-piece-text-editor` edits,
/// with the Markdown / HTML / RTF codecs over it (docs/texteditor.md).
pub mod styled;
/// Import and export for [`StyledText`]: Markdown, HTML and RTF (docs/texteditor.md).
pub mod styled_codec;
pub mod transfer;

pub use resource::{
    AssetDir, AssetName, FontFamily, ImageName, Resource, ResourceOpener, VectorName,
    resolve_asset_dir, resolve_image_file, resource, set_resource_opener,
};
pub use styled::{
    Decoration, DecorationStyle, ListStyle, ParagraphRun, ParagraphStyle, RunStyle, StyledText,
    coalesce_runs, paragraph_bounds, paragraphs_are_valid,
};
pub use styled_codec::{
    DocStyle, html_to_styled, markdown_to_styled, rtf_to_styled, styled_to_html,
    styled_to_markdown, styled_to_rtf,
};

/// Bundled custom fonts: name-table parsing, runtime font directory, family → file resolution
/// (§18.4). Shared by the CLI stagers and the backends' startup registration. Lives in the leaf
/// `day-fonts` crate (pure std, no `day-geometry`), re-exported here so `day_spec::fonts::…` is
/// unchanged for the backends while the CLI can depend on `day-fonts` alone.
pub use day_fonts as fonts;

/// The route inside a deep-link URL (docs/deep-links.md): everything after `scheme://`,
/// query included (`notes://mail/inbox?hint=x` ⇒ `mail/inbox?hint=x`). A string with no
/// scheme passes through unchanged (it is already a route). One definition, shared by every
/// platform's intake and the dayscript `deep_link` step, so a URL maps to the same route
/// bytes everywhere; percent-decoding stays with the route parser (docs/navigation.md).
#[must_use]
pub fn route_of_url(url: &str) -> String {
    match url.split_once("://") {
        Some((_, rest)) => rest.to_string(),
        None => url.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Interned piece-kind key, e.g. `"day.label"` or `"acme.combobox"`.
pub type PieceKind = &'static str;

/// Declare the built-in piece vocabulary once: the [`Builtin`] enum, its string keys, and the
/// `kinds::*` constants are all generated from this list, so they can never drift apart.
///
/// Adding a variant here is a breaking change for backends: every `Toolkit` must decide how
/// to realize the new kind, and the exhaustive `match Builtin` in each backend's `realize`
/// turns that decision into a compile error rather than a runtime placeholder.
macro_rules! builtin_kinds {
    ($( $(#[$meta:meta])* $variant:ident = $konst:ident => $key:literal ),+ $(,)?) => {
        /// Every piece kind Day itself defines (§5.3). Backends match on this exhaustively in
        /// `realize`; anything outside it is an extension piece resolved through the
        /// [`Registry`] by its string [`PieceKind`].
        #[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
        pub enum Builtin {
            $( $(#[$meta])* $variant, )+
        }

        impl Builtin {
            /// This kind's wire key: the same string as the matching `kinds::*` constant.
            pub const fn key(self) -> PieceKind {
                match self { $( Builtin::$variant => $key, )+ }
            }

            /// The built-in this key names, or `None` for an extension piece's kind.
            pub fn from_key(kind: PieceKind) -> Option<Builtin> {
                match kind {
                    $( $key => Some(Builtin::$variant), )+
                    _ => None,
                }
            }

            /// Every built-in, in declaration order (conformance tests iterate it).
            pub const ALL: &'static [Builtin] = &[ $( Builtin::$variant, )+ ];
        }

        /// The built-in piece keys as plain strings, for the registry and the `PieceKind` API.
        pub mod kinds {
            $( $(#[$meta])* pub const $konst: super::PieceKind = super::Builtin::$variant.key(); )+
        }
    };
}

builtin_kinds! {
    /// A dumb native panel (the column/row/stack backing).
    Container = CONTAINER => "day.container",
    Label = LABEL => "day.label",
    Button = BUTTON => "day.button",
    Toggle = TOGGLE => "day.toggle",
    Slider = SLIDER => "day.slider",
    TextField = TEXT_FIELD => "day.text_field",
    /// A native multi-line text editor (docs/textarea.md). Built-in since 2026-07 (previously
    /// the satellite `day-piece-textarea`).
    TextArea = TEXT_AREA => "day.text_area",
    /// A native option picker with menu/segmented/inline stylings (docs/picker.md). Built-in
    /// since 2026-07 (previously the satellite `day-piece-picker`).
    Picker = PICKER => "day.picker",
    Divider = DIVIDER => "day.divider",
    Scroll = SCROLL => "day.scroll",
    Image = IMAGE => "day.image",
    /// Progress indicator: determinate bar (fraction) or indeterminate spinner.
    Progress = PROGRESS => "day.progress",
    Canvas = CANVAS => "day.canvas",
    /// Navigation host (docs/navigation.md): stack on mobile, split panes on desktop.
    Nav = NAV => "day.nav",
    /// One destination's native container inside a NAV host.
    NavPage = NAV_PAGE => "day.nav_page",
    /// Native navigation item list (docs/navigation.md): `NSOutlineView` source list /
    /// `GtkListBox` navigation-sidebar / `QListWidget` / `UITableView` rows with chevrons.
    NavMenu = NAV_MENU => "day.nav_menu",
    /// Native recycling list (docs/list.md): `NSTableView` / `UITableView` / `RecyclerView` /
    /// `GtkListView` / `QListView`. Owns scrolling + cell reuse; Day binds row content on demand.
    List = LIST => "day.list",
    /// A recycled row's content anchor inside a `LIST`; Day adopts the native cell as its handle.
    /// Adopted, never realized: `Toolkit::adopt` wraps the native cell, so backends' `realize`
    /// never sees this kind.
    ListCell = LIST_CELL => "day.list_cell",
    /// Native hierarchical tree (docs/tree.md): `NSOutlineView` / sidebar collection list /
    /// `GtkListView`+`TreeListModel` / WinUI `TreeView` / ArkTS `TreeView`. Rows nest, disclose, and
    /// drag-to-reparent; Day binds row content on demand through `Toolkit::attach_tree`.
    /// Row cells are adopted (the same `LIST_CELL` anchor path as `List`).
    Tree = TREE => "day.tree",
    /// A fullscreen cover (docs/cover.md): a modal surface presented over the whole window,
    /// edge-to-edge, driven by `CoverPatch::{Present,Dismiss}`. The handle is the cover's
    /// content container; its frame is native-owned while presented (the backend sizes it to
    /// the safe area and reports it via `Event::FrameChanged`).
    Cover = COVER => "day.cover",
    /// Content beside a trailing inspector pane (docs/inspector.md): the native split whose
    /// second pane is a show/hidable properties panel. Exactly two `INSPECTOR_PANE` children:
    /// the content, then the panel (`InspectorPaneProps::panel`). Only realized where
    /// `Cap::Inspector` answers `Native`; everywhere else the `inspector` piece composes the
    /// pane from plain containers and this kind never reaches the backend.
    Inspector = INSPECTOR => "day.inspector",
    /// One pane's container inside an `INSPECTOR` host: the `NAV_PAGE` of the inspector split.
    /// Its frame is native-owned (the splitter/dock sizes it), reported via
    /// `Event::FrameChanged` on this node; Day lays the pane's content out inside it.
    InspectorPane = INSPECTOR_PANE => "day.inspector_pane",
    /// Two panes with a divider the user drags (docs/split.md): the toolkit's own splitter,
    /// side by side or stacked (`SplitProps::axis`), the first pane's share bound to the app
    /// (`SplitProps::fraction`, reported back as `Event::ValueChanged`). Exactly two
    /// `SPLIT_PANE` children, first then second. Only realized where `Cap::Split` answers
    /// `Native`; everywhere else the `split` piece composes the panes and draws the divider
    /// itself, and this kind never reaches the backend.
    Split = SPLIT => "day.split",
    /// One pane's container inside a `SPLIT` host: native-owned frame (the splitter sizes it),
    /// reported via `Event::FrameChanged` on this node, like an `INSPECTOR_PANE`.
    SplitPane = SPLIT_PANE => "day.split_pane",
    /// A list row's standard content (docs/list.md "List items"): a leading icon, a title, an
    /// optional subtitle and trailing detail, and a check mark, drawn the way the platform
    /// draws its own list rows: a `UIListContentView` with the cell's default content
    /// configuration, a Material list item, an `NSTableCellView`'s image and text, a GTK row
    /// box. Only realized where `Cap::ListItem` answers `Native`; everywhere else the
    /// `list_item` piece composes the same content and this kind never reaches the backend.
    ListItem = LIST_ITEM => "day.list_item",
}

/// Whether a kind is worth asking [`Toolkit::first_baseline`] about (docs/baseline.md).
///
/// A blocklist, not an allowlist: the structural and graphic kinds below have no text of their
/// own, and everything else (including extension pieces like the date/time pickers, whose kinds
/// day-spec has never heard of) is assumed to be worth asking. An extension piece that turns
/// out to have no baseline answers `None`, which costs one query; an allowlist would instead
/// silently leave every extension piece centered, which is the bug this shape avoids.
#[must_use]
pub fn kind_has_baseline(kind: PieceKind) -> bool {
    !matches!(
        Builtin::from_key(kind),
        Some(
            Builtin::Container
                | Builtin::Scroll
                | Builtin::Divider
                | Builtin::Image
                | Builtin::Canvas
                | Builtin::Slider
                | Builtin::Toggle
                | Builtin::Progress
                | Builtin::Nav
                | Builtin::NavPage
                | Builtin::NavMenu
                | Builtin::Split
                | Builtin::SplitPane
                | Builtin::List
                | Builtin::ListCell
                | Builtin::Cover
                | Builtin::Inspector
                | Builtin::InspectorPane
        )
    )
}

/// Placeholder leaves: the one hole in Day's rendering that is invisible to a screenshot.
///
/// When a backend has no renderer for a kind it realizes a visible `⟨kind⟩` label rather than
/// failing. That is the right runtime behavior, but it means a missing renderer looks like a
/// rendered app: the walkthrough passes, and the gallery's validator (which counts distinct
/// colors) sees nothing wrong. That is how a stale `skip_on:` survived in the showcase
/// walkthrough after the renderer it skipped had landed.
///
/// So every backend reports here instead of keeping its own warn-once set. The table is process-
/// wide and append-only, which gives dayscript's `assert_no_placeholders` something to assert on
/// (`crates/day-script`): a placeholder becomes a test failure instead of a silent gap.
pub mod placeholder {
    use super::PieceKind;
    use std::collections::BTreeSet;
    use std::sync::{Mutex, OnceLock};

    fn table() -> &'static Mutex<BTreeSet<PieceKind>> {
        static TABLE: OnceLock<Mutex<BTreeSet<PieceKind>>> = OnceLock::new();
        TABLE.get_or_init(|| Mutex::new(BTreeSet::new()))
    }

    /// Record that `toolkit` had no renderer for `kind` and warn, once per kind, however many
    /// nodes of it are realized. Call from a backend's `realize` fallback arm.
    pub fn report(kind: PieceKind, toolkit: &str) {
        let Ok(mut seen) = table().lock() else { return };
        if seen.insert(kind) {
            log::warn!(
                "no renderer for piece kind \"{kind}\" on {toolkit} — is the piece's \
                 {toolkit} feature enabled? (rendering a placeholder)"
            );
        }
    }

    /// Every kind that has rendered a placeholder in this process, sorted. Empty is the goal.
    #[must_use]
    pub fn seen() -> Vec<PieceKind> {
        table()
            .lock()
            .map_or_default(|s| s.iter().copied().collect())
    }
}

/// Downcast a realize/update payload to the props/patch type a backend arm expects,
/// warning (once per kind, via [`placeholder::report`]) instead of panicking on a
/// mismatch. A mismatch means a piece registered under a builtin key or a day-core
/// regression; degrade to the placeholder path, never abort: most backend arms run
/// inside native up-calls, where a panic is a process kill.
pub fn props_of<'a, T: 'static>(
    kind: PieceKind,
    toolkit: &str,
    any: &'a dyn std::any::Any,
) -> Option<&'a T> {
    let p = any.downcast_ref::<T>();
    if p.is_none() {
        placeholder::report(kind, toolkit);
    }
    p
}

/// Pointer-keyed per-view side tables with an automatic release sweep.
///
/// Backends keep auxiliary per-view state in thread-local maps keyed by the native
/// handle's address. Historically each map needed its own line in `release()`, a
/// checklist that drifted repeatedly (missed entries have produced both leaks and
/// stale-pointer CI segfaults, since a freed address gets recycled). A [`crate::sidetable::SideTable`]
/// registers itself with a per-thread sweeper list at construction, so a backend's
/// `release()` calls [`crate::sidetable::sweep`] once and every table, present and future, drops its
/// entry for that handle, running its teardown hook if one was given.
pub mod sidetable {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::rc::Rc;

    /// One table's "remove this key" hook, registered at construction.
    type Sweeper = Rc<dyn Fn(usize)>;

    thread_local! {
        static SWEEPERS: RefCell<Vec<Sweeper>> = const { RefCell::new(Vec::new()) };
    }

    /// A pointer-keyed map swept automatically by [`crate::sidetable::sweep`]. Construct inside a
    /// `thread_local!` initializer; the optional teardown hook runs for each removed
    /// value (release a retained native object, dispose an owned child, …), both on sweep
    /// and on plain [`SideTable::remove`], so teardown cannot be bypassed by accident.
    pub struct SideTable<V: 'static> {
        map: Rc<RefCell<HashMap<usize, V>>>,
        teardown: Option<Rc<dyn Fn(V)>>,
    }

    impl<V> Default for SideTable<V> {
        fn default() -> Self {
            Self::new()
        }
    }

    impl<V> SideTable<V> {
        #[must_use]
        pub fn new() -> Self {
            Self::build(None)
        }

        /// A table whose values need active teardown when their view goes away.
        pub fn with_teardown(teardown: impl Fn(V) + 'static) -> Self {
            Self::build(Some(Rc::new(teardown)))
        }

        fn build(teardown: Option<Rc<dyn Fn(V)>>) -> Self {
            let map: Rc<RefCell<HashMap<usize, V>>> = Rc::default();
            let (m, t) = (map.clone(), teardown.clone());
            SWEEPERS.with(|s| {
                s.borrow_mut().push(Rc::new(move |key| {
                    // Remove under the borrow, tear down outside it: hooks may re-enter
                    // toolkit code that reads other tables.
                    let v = m.borrow_mut().remove(&key);
                    if let (Some(v), Some(t)) = (v, &t) {
                        t(v);
                    }
                }));
            });
            SideTable { map, teardown }
        }

        pub fn insert(&self, key: usize, value: V) -> Option<V> {
            self.map.borrow_mut().insert(key, value)
        }

        /// Remove and tear down the entry (returns `None` always when a teardown hook
        /// exists, because the value is consumed by it; use [`SideTable::take`] to skip it).
        #[must_use]
        pub fn remove(&self, key: usize) -> Option<V> {
            let v = self.map.borrow_mut().remove(&key)?;
            match &self.teardown {
                Some(t) => {
                    t(v);
                    None
                }
                None => Some(v),
            }
        }

        /// Remove without running the teardown hook (the caller takes ownership).
        #[must_use]
        pub fn take(&self, key: usize) -> Option<V> {
            self.map.borrow_mut().remove(&key)
        }

        #[must_use]
        pub fn contains(&self, key: usize) -> bool {
            self.map.borrow().contains_key(&key)
        }

        #[must_use]
        pub fn get(&self, key: usize) -> Option<V>
        where
            V: Clone,
        {
            self.map.borrow().get(&key).cloned()
        }

        /// Run `f` on the value in place (returns `None` if absent).
        pub fn with<R>(&self, key: usize, f: impl FnOnce(&mut V) -> R) -> Option<R> {
            self.map.borrow_mut().get_mut(&key).map(f)
        }

        /// Run `f` over every `(key, value)` (immutable; do not re-enter this table).
        pub fn for_each(&self, mut f: impl FnMut(usize, &V)) {
            for (k, v) in self.map.borrow().iter() {
                f(*k, v);
            }
        }
    }

    /// Remove `key` from every table registered on this thread, running teardowns.
    /// Call once from a backend's `release()` (and once per auxiliary native object a
    /// release frees, e.g. an owned content container).
    pub fn sweep(key: usize) {
        // Clone the sweeper list first: a teardown may construct a new SideTable
        // (registering a sweeper) without deadlocking on the borrow.
        let sweepers: Vec<Sweeper> = SWEEPERS.with(|s| s.borrow().clone());
        for f in sweepers {
            f(key);
        }
    }
}

/// The painting threads behind the offscreen exports (docs/canvas.md "Exporting").
///
/// `Toolkit::render_bitmap` answers with an event, so a toolkit whose 2-D API can draw into a
/// memory surface from any thread paints there and posts the answer back to the UI thread. The
/// pool is shared by every toolkit in the process: a few threads, one short of the cores and
/// never more than four, so a screenful of requests cannot starve the UI thread or the app's
/// own work. Jobs run in the order they were queued.
///
/// Not built for the web: a browser's `createImageBitmap` is already asynchronous, and
/// `wasm32-unknown-unknown` has no threads to spawn.
#[cfg(not(target_arch = "wasm32"))]
pub mod offscreen {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex, OnceLock, mpsc};

    type Job = Box<dyn FnOnce() + Send + 'static>;

    static QUEUE: OnceLock<Mutex<mpsc::Sender<Job>>> = OnceLock::new();
    /// How many painting threads started; zero means every job runs where it was queued.
    static WORKERS: AtomicUsize = AtomicUsize::new(0);

    /// Run `job` on a painting thread, or inline when none could be started. A panic inside
    /// `job` is contained ([`crate::ffi_guard::contain`]) so one bad replay cannot take the
    /// thread, and with it every later export, down; the job itself should answer its request
    /// in that case, which is why toolkits wrap the paint and not the post.
    pub fn spawn(job: impl FnOnce() + Send + 'static) {
        let queue = QUEUE.get_or_init(start);
        if WORKERS.load(Ordering::Acquire) == 0 {
            crate::ffi_guard::contain((), job);
            return;
        }
        let job: Job = Box::new(job);
        // The receivers live as long as the process, so a refused send means every worker left
        // its loop; the job still runs, inline, rather than a request going unanswered.
        let refused = match queue.lock() {
            Ok(sender) => sender.send(job).err().map(|e| e.0),
            Err(_) => Some(job),
        };
        if let Some(job) = refused {
            crate::ffi_guard::contain((), job);
        }
    }

    fn start() -> Mutex<mpsc::Sender<Job>> {
        let (tx, rx) = mpsc::channel::<Job>();
        let rx = Arc::new(Mutex::new(rx));
        let want = std::thread::available_parallelism()
            .map_or(2, std::num::NonZero::get)
            .saturating_sub(1)
            .clamp(1, 4);
        let mut started = 0;
        for i in 0..want {
            let rx = Arc::clone(&rx);
            let spawned = std::thread::Builder::new()
                .name(format!("day-paint-{i}"))
                .spawn(move || {
                    loop {
                        // The lock is held only while waiting for the next job, never while
                        // running one, so the other workers queue behind it fairly.
                        let next = match rx.lock() {
                            Ok(guard) => guard.recv(),
                            Err(_) => return,
                        };
                        match next {
                            Ok(job) => crate::ffi_guard::contain((), job),
                            Err(_) => return,
                        }
                    }
                });
            match spawned {
                Ok(_) => started += 1,
                Err(e) => log::error!("day-spec: could not start a painting thread: {e}"),
            }
        }
        WORKERS.store(started, Ordering::Release);
        Mutex::new(tx)
    }
}

/// Realized-node identity as seen by backends (day-core's slotmap key, FFI-encoded).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub u64);

/// Default navigation sidebar width (split presentation) until the pane reports its size.
/// Semantic container surfaces (see `ContainerProps::role`): each backend maps a role to its
/// own theme-adaptive material so the fill tracks light/dark mode without app code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceRole {
    /// A form `section` card, the platform's grouped-content background: AppKit quaternary
    /// system fill, libadwaita `.card`, Qt `palette(alternate-base)`, UIKit tertiary system
    /// fill, Material surface-container, XAML card background brush.
    SectionCard,
    /// The window's own background, the surface a page sits on: what an opaque cover paints
    /// by default, and the card of a composed modal sheet (docs/cover.md "Modal sheet").
    Window,
    /// The layer between a composed modal sheet and the page under it, which says the page is
    /// out of reach until the sheet goes. The container's `background` is the plain dimming a
    /// backend draws when it has no material of its own; the web frosts the page instead.
    Scrim,
}

pub const NAV_SIDEBAR_WIDTH: f64 = 240.0;

/// Drag limits for a navigation host's sidebar pane, around [`NAV_SIDEBAR_WIDTH`]. Shared by the
/// desktops whose divider the user can drag (docs/navigation.md): AppKit enforces both through
/// its `NSSplitViewItem`; Qt honors the minimum through the pane's own minimum size and leaves
/// the maximum to the user.
pub const NAV_SIDEBAR_MIN_W: f64 = 160.0;
pub const NAV_SIDEBAR_MAX_W: f64 = 400.0;

/// Default preferred width of a navigation host's content-list pane
/// (`props::NavProps::list_width`, docs/navigation.md), Mail's message-list proportion.
pub const NAV_LIST_WIDTH: f64 = 300.0;

/// Drag limits for the content-list pane, around the app's preferred width; the same split of
/// duties as the sidebar's.
pub const NAV_LIST_MIN_W: f64 = 220.0;
pub const NAV_LIST_MAX_W: f64 = 560.0;

/// Reserved id for window-level events (resize, lifecycle): day-core routes it to the root.
pub const WINDOW_NODE: NodeId = NodeId(u64::MAX);

/// Which nodes handle the non-text keys (docs/menus.md). Keys follow focus (the focused piece
/// hears them and nobody else), so a backend whose focused view would have to claim a key from
/// the platform's own dispatch (AppKit's responder chain, the DOM's default action) has to know
/// whether the app wants it before swallowing it. An unclaimed arrow has to keep traveling, or
/// a canvas inside a scroll view would silently eat the keys that scroll it.
///
/// It lives here, in day-spec, rather than in day-core: `day-toolkit-dom` depends on day-spec alone,
/// and this is a fact about a node that both sides need.
pub mod keys {
    use super::NodeId;
    use std::cell::RefCell;
    use std::collections::HashSet;

    thread_local! {
        static HANDLED: RefCell<HashSet<NodeId>> = RefCell::new(HashSet::new());
    }

    /// Declare that `node` has a key handler (what `Decorate::on_key` records at build).
    pub fn mark(node: NodeId) {
        HANDLED.with(|h| h.borrow_mut().insert(node));
    }

    /// Whether `node` has one.
    #[must_use]
    pub fn handled(node: NodeId) -> bool {
        HANDLED.with(|h| h.borrow().contains(&node))
    }

    thread_local! {
        static TEXT: RefCell<HashSet<NodeId>> = RefCell::new(HashSet::new());
    }

    /// Declare that `node` takes the whole keyboard while focused (what `Decorate::keyboard`
    /// records at build): every key the platform delivers reaches its handler, named the way
    /// [`super::KeyEvent`] describes for the full route, and a touch platform raises its soft
    /// keyboard for it.
    pub fn mark_text(node: NodeId) {
        TEXT.with(|h| h.borrow_mut().insert(node));
    }

    /// Whether `node` takes the whole keyboard: it declared so AND it [`handled`]s keys. A
    /// declaration with no `.on_key` behind it claims nothing, so every backend asks this one
    /// question before it raises a soft keyboard, names a letter or claims a Tab.
    #[must_use]
    pub fn text(node: NodeId) -> bool {
        TEXT.with(|h| h.borrow().contains(&node)) && handled(node)
    }

    /// The keys a focused single-line text field hands to its key handler instead of acting
    /// on them itself (docs/textfield.md "Keys"): what an address bar needs to walk a
    /// suggestion list and to cancel an edit. Return stays the field's submit, Tab its focus
    /// walk, and every other key its editing.
    pub const FIELD_KEYS: [&str; 3] = ["ArrowUp", "ArrowDown", "Escape"];

    /// Whether a focused text field at `node` claims the key named `key` (web name, see
    /// [`super::Key::from_name`]) pressed with `modifiers`: one of [`FIELD_KEYS`], no modifier
    /// held, and a handler declared for the field. A backend that answers yes reports
    /// `Event::Key` for `node` and keeps the key from the native field.
    #[must_use]
    pub fn field_claims(node: NodeId, key: &str, modifiers: impl Into<super::Modifiers>) -> bool {
        modifiers.into().is_empty() && FIELD_KEYS.contains(&key) && handled(node)
    }

    /// Drop both marks for a node that left the tree (day-core calls it from its removal
    /// path). Node ids are generational, so a stale id would never match a new node; this
    /// keeps the registries from growing with every rebuilt canvas.
    pub fn forget(node: NodeId) {
        HANDLED.with(|h| h.borrow_mut().remove(&node));
        TEXT.with(|h| h.borrow_mut().remove(&node));
    }
}

/// Raw foreign native handle for polyglot adoption (§15.3).
pub type RawHandle = *mut std::ffi::c_void;

// ---------------------------------------------------------------------------
// Events (§8.3)
// ---------------------------------------------------------------------------

/// The wire table for backends whose native side reaches Rust through one numeric-kind
/// trampoline (Android's JNI `nativeOnEvent`, ArkUI's `day_arkui_on_event`). This enum
/// defines those kind numbers; the Java and C++ sides carry mirrored constants that parity
/// tests check against these discriminants (so a collision or drift fails `cargo test` on the
/// host instead of silently mis-decoding events on a device). AppKit/UIKit/GTK/Qt emit `Event`
/// values directly and never use these numbers; XAML uses per-event callbacks with its own
/// small local codes.
///
/// Payload conventions ride `(num: f64, text: String)` per kind, documented on each variant.
pub mod bridge {
    /// One numeric event kind. `as i32` is the wire value.
    #[repr(i32)]
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum BridgeKind {
        /// Click/press. No payload.
        Pressed = 0,
        /// `text` = the field's full new text.
        TextChanged = 1,
        /// `num` != 0 ⇒ on.
        ToggleChanged = 2,
        /// `num` = the new value.
        ValueChanged = 3,
        /// `num` = the selected index.
        SelectionChanged = 4,
        /// `num` == 1 ⇒ the native side already popped (predictive back / up arrow).
        NavBack = 5,
        /// Nav/tab page size report; `text` = `"w,h"` in px (Rust divides by density).
        FrameChanged = 6,
        /// Warm deep link; `text` = the route.
        Deeplink = 7,
        /// Modal answered with a button; `num` = the button index.
        PresentButton = 8,
        /// Modal answered with text; `text` = the entry.
        PresentText = 9,
        /// Modal dismissed.
        PresentDismissed = 10,
        /// Gesture; `num` = the code, `text` = the payload in px (Rust divides by density).
        /// Codes 0 to 3 are a tap and a drag's three phases (0 tap, 1 began, 2 changed,
        /// 3 ended) with `text` = `"x,y,tx,ty"`. The rest are the [`gesture`] constants:
        /// long press, double tap, press in and out (`"x,y"`), a drag's or a pan's release
        /// velocity (`"vx,vy"` in px per second), a touch list ([`gesture::TOUCHES`]) and the
        /// pointer kind (`text` = [`crate::PointerKind::name`]).
        Gesture = 11,
        /// Piece-defined open channel (`Event::Custom` with an empty tag): the piece reads the
        /// raw `num`/`text` payload.
        Custom = 12,
        /// Menu selection; the event's node id is the chosen action's dispatch id.
        MenuAction = 13,
        /// App lifecycle; `num` = the phase code (`day_spec::Lifecycle` order).
        Lifecycle = 14,
        /// File-picker answer; `text` = chosen locators joined by the unit separator.
        PresentFile = 15,
        /// `num` != 0 ⇒ gained keyboard focus.
        FocusChanged = 16,
        /// IME action / Return.
        Submitted = 17,
        /// Root size change; `text` = `"w,h"` in px. Routed to `WINDOW_NODE` as a window
        /// resize (the rail rotation, late inset passes, and the soft keyboard ride).
        WindowResized = 18,
        /// Safe-area report from an edge-to-edge backend; `text` =
        /// `"top,bottom,leading,trailing"` in px (Rust divides by density and feeds
        /// `day_core::set_safe_area`). Node id ignored; no `Event` is emitted.
        SafeArea = 19,
        /// A secondary window closed (docs/windows.md); the node id is the window's root.
        WindowClosed = 20,
        /// A secondary window's key/active state changed; `num` != 0 ⇒ focused.
        WindowFocused = 21,
        /// `num` = the settled value: the drag ended (see `Event::ValueCommitted`). A backend
        /// that cannot tell a settled value from a moving one sends only `ValueChanged`.
        ValueCommitted = 22,
        /// Inline search on a `.searchable()` navigation surface (docs/search.md): the field's
        /// new text, against the nav host's node. `num` = 1 ⇒ the text was submitted (Return or
        /// the keyboard's Search key), decoding to [`crate::Event::SearchSubmitted`].
        SearchChanged = 23,
        /// The toolkit's own adaptive nav container changed presentation and is reporting it
        /// (docs/size-classes.md). `num` = 1.0 for split, 0.0 for stacked.
        NavPresentation = 24,
        /// The system switched between light and dark appearance. No payload; node id ignored
        /// and no `Event` is emitted; Rust calls `day_core::note_appearance_changed`, the same
        /// thing AppKit's and GTK's own scheme observers do.
        AppearanceChanged = 25,
        /// A fullscreen cover's hide transition finished (docs/cover.md); no payload, and the
        /// node id is the cover's. Decodes to [`crate::Event::CoverHidden`].
        CoverHidden = 26,
        /// A styled text run's link was tapped (docs/text-runs.md); `text` = the run's target.
        /// Decodes to [`crate::Event::LinkActivated`].
        LinkActivated = 27,
        /// The platform's own undo affordance fired (⌘Z, a three-finger swipe, the Edit menu
        /// through a native front); `num` != 0 ⇒ redo.
        UndoInvoked = 28,
        /// A toolbar item produced a value (docs/toolbars.md): the node id is the item's
        /// dispatch action; `text` = `"on"` with `num` != 0 ⇒ a toggle turned on, `"on"` with
        /// 0 ⇒ off, `"sel"` with `num` = the chosen segment, `"text:<query>"` ⇒ a search
        /// item's new text, `"submit:<query>"` ⇒ that text submitted. Decodes to
        /// [`crate::Event::ToolbarChanged`]. A plain toolbar button sends `MenuAction`.
        ToolbarChanged = 30,
        /// An activated list row; `num` is its current index.
        ListActivated = 31,
        /// External document copied into app storage; `text` = its local locator.
        DocumentOpened = 32,
        /// The user's reduce-motion setting flipped (docs/accessibility.md). No payload; node
        /// id ignored and no `Event` is emitted; Rust calls `day_core::note_motion_changed`, as
        /// the desktop backends' own observers do.
        ReduceMotionChanged = 33,
        /// Text committed on the full key route (docs/menus.md "The full route"): `text` =
        /// what the input method or the key typed, `num` = the modifier mask. Decodes to an
        /// [`crate::Event::Key`] whose `text` is set; a key that typed nothing rides `Key`.
        Text = 36,
        /// A non-text key reached the focused node (docs/menus.md); `text` = the key's web name
        /// (`"ArrowLeft"`, `"5"`, …, read by [`crate::Key::from_name`]), `num` = the
        /// [`crate::Modifiers::bits`] mask in its low byte and the phase above it (bit 8 set ⇒
        /// a repeat, bit 9 set ⇒ a release; [`crate::KeyPhase::from_wire`]). Decodes to
        /// [`crate::Event::Key`].
        Key = 29,
        /// A window's display state changed (docs/windows.md): `num` = the
        /// [`crate::WindowState`] code (0 normal, 1 minimized, 2 maximized, 3 fullscreen), against
        /// the window's root node, or [`crate::WINDOW_NODE`] for the primary. Decodes to
        /// [`crate::Event::WindowStateChanged`] through [`crate::WindowState::from_code`].
        WindowStateChanged = 34,
        /// Native pull-to-refresh request; no payload.
        RefreshRequested = 35,
        /// Text shared to the app from the system share sheet (Day.toml `share_target`,
        /// docs/deep-links.md): `text` = the shared text, then the unit separator and the
        /// subject when one came with it. Node id ignored and no `Event` is emitted; Rust calls
        /// `day_core::request_shared_text`, which sends a web link in it to `on_open_url`.
        SharedText = 37,
    }

    /// The [`BridgeKind::Gesture`] codes above the tap and drag phases (0 to 3), mirrored by the
    /// Java and C++ sides like the kind numbers.
    pub mod gesture {
        /// `Event::LongPress`; `text` = `"x,y"`.
        pub const LONG_PRESS: i32 = 4;
        /// `Event::DoubleTap`; `text` = `"x,y"`.
        pub const DOUBLE_TAP: i32 = 5;
        /// `Event::Press` with `pressed: true`; `text` = `"x,y"`.
        pub const PRESS_IN: i32 = 6;
        /// `Event::Press` with `pressed: false`; `text` = `"x,y"`.
        pub const PRESS_OUT: i32 = 7;
        /// `Event::Touches`; `text` = one `"id,phase,x,y,pressure,kind"` per contact, joined
        /// by `;` (phase 0 began, 1 moved, 2 stationary, 3 ended, 4 cancelled; kind 0 touch,
        /// 1 mouse, 2 stylus; pressure 0 to 1).
        pub const TOUCHES: i32 = 8;
        /// `Event::GestureVelocity` for a drag; `text` = `"vx,vy"`.
        pub const DRAG_VELOCITY: i32 = 9;
        /// `Event::GestureVelocity` for a pan; `text` = `"vx,vy"`.
        pub const PAN_VELOCITY: i32 = 10;
        /// `Event::PointerKindChanged`; `text` = the kind's name. Node id ignored.
        pub const POINTER_KIND: i32 = 11;
    }

    impl BridgeKind {
        /// Every variant, for uniqueness/parity tests and exhaustive dispatch.
        pub const ALL: [BridgeKind; 38] = [
            BridgeKind::Pressed,
            BridgeKind::TextChanged,
            BridgeKind::ToggleChanged,
            BridgeKind::ValueChanged,
            BridgeKind::SelectionChanged,
            BridgeKind::NavBack,
            BridgeKind::FrameChanged,
            BridgeKind::Deeplink,
            BridgeKind::PresentButton,
            BridgeKind::PresentText,
            BridgeKind::PresentDismissed,
            BridgeKind::Gesture,
            BridgeKind::Custom,
            BridgeKind::MenuAction,
            BridgeKind::Lifecycle,
            BridgeKind::PresentFile,
            BridgeKind::FocusChanged,
            BridgeKind::Submitted,
            BridgeKind::WindowResized,
            BridgeKind::SafeArea,
            BridgeKind::WindowClosed,
            BridgeKind::WindowFocused,
            BridgeKind::ValueCommitted,
            BridgeKind::SearchChanged,
            BridgeKind::NavPresentation,
            BridgeKind::AppearanceChanged,
            BridgeKind::CoverHidden,
            BridgeKind::LinkActivated,
            BridgeKind::UndoInvoked,
            BridgeKind::Key,
            BridgeKind::ToolbarChanged,
            BridgeKind::ListActivated,
            BridgeKind::DocumentOpened,
            BridgeKind::ReduceMotionChanged,
            BridgeKind::WindowStateChanged,
            BridgeKind::RefreshRequested,
            BridgeKind::Text,
            BridgeKind::SharedText,
        ];
    }

    #[cfg(test)]
    mod tests {
        use super::BridgeKind;

        /// The kind-15 lesson: two meanings on one number decode silently as the first. Every
        /// discriminant must be unique, and the table must stay dense enough to spot gaps.
        #[test]
        fn discriminants_are_unique() {
            let mut seen = std::collections::BTreeSet::new();
            for k in BridgeKind::ALL {
                assert!(
                    seen.insert(k as i32),
                    "duplicate bridge kind number {} ({k:?})",
                    k as i32
                );
            }
            assert_eq!(seen.len(), BridgeKind::ALL.len());
        }

        /// `ALL` is what the uniqueness check above sees, so a variant left out of it is a variant
        /// nothing checks, which is how `SearchChanged` and `NavPresentation` sat outside the
        /// table. The wire numbers are assigned densely from 0, so demanding the set be exactly
        /// `0..len` catches both a gap and an omission.
        #[test]
        fn table_covers_every_wire_number() {
            let mut nums: Vec<i32> = BridgeKind::ALL.iter().map(|k| *k as i32).collect();
            nums.sort_unstable();
            let dense: Vec<i32> = (0..BridgeKind::ALL.len() as i32).collect();
            assert_eq!(
                nums, dense,
                "bridge kinds must be dense from 0 — a missing entry means a variant no test sees"
            );
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Pressed,
    TextChanged(String),
    Submitted,
    ToggleChanged(bool),
    ValueChanged(f64),
    /// A continuous control's value settled: the drag ended (mouse/touch up), or the value moved
    /// by a discrete action (a keyboard arrow, a click on the track). Always preceded by the
    /// `ValueChanged` carrying the same value.
    ///
    /// The pair exists because one gesture produces two different facts. `ValueChanged` is the
    /// live one (bindings follow it so the UI tracks the thumb) and it fires continuously, which
    /// is why nothing durable should key off it: a drag from 1 to 100 and back to 50 emits every
    /// value in between. `ValueCommitted` fires once, with the value the user chose, and is what
    /// a recording, a log, or a refetch should use.
    ///
    /// A backend that cannot tell the two apart emits only `ValueChanged`; its sliders then do not
    /// record (docs/coverage-matrix.md).
    ValueCommitted(f64),
    SelectionChanged(i64),
    /// A list row was activated: desktop double-click/Enter, or a touch tap.
    /// Separate from selection so keyboard browsing need not open or play the row.
    ListActivated(usize),
    /// The platform's own undo affordance fired through a native front (⌘Z on the Edit menu,
    /// a three-finger swipe, shake-to-undo); `redo` distinguishes the pair. Only backends
    /// with a native undo system emit it; everywhere else the app's own controls call the
    /// stack directly and this event never exists (docs/model.md).
    Undo {
        redo: bool,
    },
    /// The platform's own standard edit command fired through its native route (Edit ▸
    /// Cut/Copy/Paste, ⌘X/⌘C/⌘V, the browser's `copy`/`paste` DOM events) with no text
    /// widget claiming it first; the app's edit bridge answers (docs/menus.md). Transport
    /// is the system clipboard (day-part-clipboard), not the event.
    Edit(EditOp),
    /// A multi-select list's selection changed: the full set of selected row indices,
    /// ascending (empty = nothing selected). Emitted instead of `SelectionChanged` where
    /// `ListProps::multi_select` is honored (docs/list.md).
    SelectionSet(Vec<i64>),
    FocusChanged(bool),
    Tap(Point),
    /// A link run inside a label was activated; the payload is [`TextRun::link`] verbatim.
    LinkActivated(String),
    /// The reader's selection in a selectable label changed ([`Cap::LabelSelection`]): byte
    /// offsets into the text the label displays, `start <= end`, both on character
    /// boundaries. `start == end` means nothing is selected any more. Emitted only by labels
    /// realized with `LabelProps::reports_selection`.
    TextSelected {
        start: usize,
        end: usize,
    },
    /// A press held still on the node (docs/gestures.md "Long press"), at the press point in the
    /// node's coordinates. Fires once, while the pointer is still down; the release that
    /// follows is not a tap. Only nodes that enabled [`GestureKind::LongPress`] receive it.
    LongPress(Point),
    /// Two taps in quick succession on the node (docs/gestures.md "Double tap"), at the second
    /// tap's point. A node that enabled both [`GestureKind::Tap`] and
    /// [`GestureKind::DoubleTap`] receives either this or one `Tap`, never both: the single
    /// tap waits out the double-tap interval. With `DoubleTap` alone enabled nothing waits.
    DoubleTap(Point),
    /// The pointer went down on the node (`pressed: true`) or the press ended
    /// (`pressed: false`): the release, the pointer leaving the node's hit area, or the
    /// platform taking the gesture away (docs/gestures.md "Press"). Every `true` is followed by
    /// exactly one `false`. Reported at the pointer-down itself, before any tap or drag is
    /// recognized, and it never delays or cancels another gesture. Only nodes that enabled
    /// [`GestureKind::Press`] receive it.
    Press {
        pressed: bool,
        location: Point,
    },
    /// The contacts on the node changed (docs/gestures.md "Touches"): every contact that is
    /// down, plus the ones that ended or were cancelled in this event. Only nodes that
    /// enabled [`GestureKind::Touches`] receive it.
    Touches(Vec<Touch>),
    /// The release velocity of the node's drag or pan as the platform's own recognizer
    /// measured it, in points per second along the node's axes. Sent immediately before the
    /// gesture's `Ended` event, by a backend whose recognizer has one; day-core prefers it to
    /// its own estimate from the event samples (docs/gestures.md "Velocity"). `kind` is
    /// [`GestureKind::Drag`] or [`GestureKind::Pan`].
    GestureVelocity {
        kind: GestureKind,
        velocity: Point,
    },
    /// The kind of device behind the latest pointer input changed (docs/gestures.md "Input
    /// kind"). Emitted on [`WINDOW_NODE`], only on a change; day-core publishes it as
    /// `day::primary_pointer()`.
    PointerKindChanged(PointerKind),
    /// A context-menu summon reported by a toolkit that presents no native menu of its own
    /// (web-dom): the pieces layer shows the composed menu (docs/menus.md). `local` is the
    /// point in the node's own coordinates (what the app's provider receives); `window` the
    /// same point in window coordinates (where the composed panel goes). Toolkits that serve
    /// context menus natively never emit this.
    ContextMenu {
        local: Point,
        window: Point,
    },
    /// A drag/pan gesture (docs/shapes.md). `location` is in the node's local coordinates;
    /// `translation` is the cumulative movement since `Began`. Once `Began` is sent the
    /// gesture belongs to the node until `Ended`: an enclosing scroll view must not take it
    /// over (docs/gestures.md "Gestures inside a scroll view").
    Drag {
        phase: DragPhase,
        location: Point,
        translation: Point,
    },
    /// A pinch/magnify gesture over the node (docs/shapes.md): `scale` is cumulative since
    /// `Began` (1.0 = unchanged), `location` the gesture's centroid in local coordinates,
    /// the anchor a zoom keeps stationary. Only nodes that enabled
    /// [`GestureKind::Pinch`] receive it; a backend with no recognizer emits nothing.
    Pinch {
        phase: DragPhase,
        scale: f64,
        location: Point,
    },
    /// A two-finger rotation over the node (docs/canvas.md "Zoom and pan"): `rotation` is in
    /// radians, cumulative since `Began`, positive clockwise on screen; `location` the
    /// gesture's centroid in local coordinates ([`Point::ZERO`] where the platform gives
    /// none). A trackpad's rotate tick has no gesture session and arrives as a lone `Changed`
    /// carrying that tick's own angle. Only nodes that enabled [`GestureKind::Rotate`]
    /// receive it; a backend with no recognizer emits nothing.
    Rotate {
        phase: DragPhase,
        rotation: f64,
        location: Point,
    },
    /// A viewport pan over the node (docs/shapes.md): `delta` is the movement since the
    /// previous event; incremental, not cumulative, because desktop wheels have no gesture
    /// session to accumulate over (a discrete wheel tick arrives as a lone `Changed`).
    /// Two-finger touch pans bracket theirs with `Began`/`Ended`. Distinct from
    /// [`Event::Drag`], the primary pointer's press-drag.
    Pan {
        phase: DragPhase,
        delta: Point,
        location: Point,
    },
    /// The pointer moved over the node, or left it (docs/canvas.md "Interaction"). `location` is
    /// in the node's local coordinates. Phases follow the drag vocabulary: `Began` on entry,
    /// `Changed` for each move, `Ended` on exit, where `location` is the last point seen inside,
    /// so a handler always knows where the pointer went out.
    ///
    /// Only nodes that enabled [`GestureKind::Hover`] receive it, and only on a device with a
    /// pointer at all.
    Hover {
        phase: DragPhase,
        location: Point,
    },
    /// A scroll view's content offset changed, in points from the content's top-left corner
    /// (docs/scroll.md "Offset and phase"). Sent only by a scroll realized with
    /// [`props::ScrollProps::track_offset`], whatever moved it (a finger, a wheel, momentum, a
    /// programmatic scroll), at most once per display frame, and always once more with the
    /// offset the view came to rest at.
    ScrollChanged(Point),
    /// A scroll view's user-driven scrolling changed phase (docs/scroll.md "Offset and phase").
    /// Sent only by a scroll realized with [`props::ScrollProps::track_phase`], and only on a
    /// change. A programmatic scroll moves the offset and leaves the phase alone.
    ScrollPhase(ScrollPhase),
    /// A zooming scroll view's scale changed ([`props::ScrollProps::zoom`], docs/scroll.md
    /// "Zoom"): the user pinched, or an animated [`props::ScrollPatch::Zoom`] moved it. `1.0`
    /// is the content's natural size.
    ScrollZoomed(f64),
    /// What a recycling list shows changed (docs/list.md "Visible rows"): day-core raises this
    /// from the host's [`ListSource::viewport`] reports, deduplicated. Toolkits never send it
    /// themselves. `rows` is the visible items as a half-open range of item indices (empty
    /// when no item is in view); `end_distance` is [`ListViewport::end_distance`], or `None`
    /// from a host that reports only its first visible row.
    ListViewport {
        rows: std::ops::Range<usize>,
        end_distance: Option<f64>,
    },
    /// The reader started moving a list themselves (docs/list.md "Following the end"):
    /// day-core sends it from the host's [`ListSource::reader_scroll`] report. Toolkits never
    /// send it themselves.
    ListReaderScrolled,
    /// A canvas node was re-framed by layout; re-record (§11). Nav pane/page containers
    /// also report their allocated size with this (docs/navigation.md).
    FrameChanged(Size),
    /// Native back navigation (iOS back button/swipe, Android system back or toolbar up).
    /// `already_popped` = the toolkit already performed the pop natively (iOS); the nav
    /// host then syncs its stack without re-issuing `NavPatch::Popped`.
    NavBack {
        already_popped: bool,
    },
    /// A nav host's own adaptive container changed presentation, and Day is being told about it
    /// after the fact (docs/size-classes.md).
    ///
    /// The counterpart to [`props::NavPatch::Presentation`], for the toolkits that answer
    /// `Cap::NavRepresent = Emulated`: `UISplitViewController` collapsing as a Pro Max iPhone
    /// rotates into portrait, `SlidingPaneLayout` deciding at measure time that two panes no
    /// longer fit. Day did not ask for it and must not fight it; the pieces layer only
    /// reconciles what follows, which is the selection rule (a split presentation cannot draw an
    /// empty detail pane, so expanding with nothing selected picks the first item).
    ///
    /// Emitted on the NAV host node, and only when the presentation actually changed.
    NavPresentationChanged(props::NavPresentation),
    Key(KeyEvent),
    WindowResized(Size),
    /// The platform asks the app to show a route (docs/navigation.md), a runtime deep link:
    /// on web-dom, the URL hash changing under the app (hand-edited, or browser back/forward);
    /// on mobile, a warm scheme link (iOS `application:openURL:`, Android `onNewIntent` via
    /// `BridgeKind::Deeplink`). Emitted on [`WINDOW_NODE`] (day-core answers with `navigate()`
    /// when the route differs from the current one) or on the active nav host's node (the nav
    /// piece answers with `navigate()` itself, docs/deep-links.md). Launch-time deep links use
    /// `DAY_DEEPLINK`/`set_launch_deeplink` instead; this variant is for changes while the
    /// app runs.
    RouteRequested(String),
    /// A native modal answered request `req` (docs/dialogs.md).
    PresentResult {
        req: u64,
        result: present::PresentResult,
    },
    /// A decode started by [`Toolkit::decode_image`] finished (docs/images.md).
    ///
    /// Keyed by request id like [`Event::PresentResult`], and for the same reason: the answer
    /// belongs to a waiting caller, not to a tree node. It never crosses a native boundary
    /// (every backend raises it from its own Rust arm once the platform decoder answers), so it
    /// has no [`bridge::BridgeKind`], exactly like the completion events dialogs use in-process.
    ///
    /// No bitmap id rides here: day-core minted it and handed it to the backend with the bytes,
    /// so it already knows which image a request answers for. A backend that stored the image
    /// under that id has nothing more to say than the facts it read.
    ImageDecoded {
        req: u64,
        result: Result<BitmapInfo, ImageError>,
    },
    /// An encode started by [`Toolkit::encode_image`] finished (docs/images.md): the encoded
    /// bytes, or why they could not be produced.
    ImageEncoded {
        req: u64,
        result: Result<Vec<u8>, ImageError>,
    },
    /// A canvas drawing started by [`Toolkit::render_bitmap`] finished rendering
    /// (docs/canvas.md "Exporting"). Keyed by request id like [`Event::ImageDecoded`], and like
    /// it carries no bitmap id: day-core minted the id and handed it over with the ops, so the
    /// answer is only the facts of the image the backend stored under it.
    BitmapRendered {
        req: u64,
        result: Result<BitmapInfo, ExportError>,
    },
    /// A PDF started by [`Toolkit::render_pdf`] finished (docs/canvas.md "Exporting"): the
    /// document's bytes, or why they could not be produced.
    PdfRendered {
        req: u64,
        result: Result<Vec<u8>, ExportError>,
    },
    /// An open, piece-defined event (§8.2). `tag` names the event for in-process emitters (a static
    /// literal); it is empty for events that cross a native boundary (JNI/C-ABI), which carry only the
    /// primitive `num`/`text` payload. A piece's `cx.on` reads whichever fields it needs. This is the
    /// escape hatch for events the fixed variants above don't cover (a web view's URL, a picked date, …).
    Custom {
        tag: &'static str,
        num: f64,
        text: String,
    },
    /// A menu item (app menu or context menu) with this action id was activated (§ menus). day-core
    /// routes it to the app closure registered for the id. Standard-role items don't carry an id
    /// (`role` items are handled natively) so they never emit this.
    MenuAction(u64),
    /// A toolbar item produced a value: a search field's text, a toggle's new state
    /// (docs/toolbars.md). `action` is the item's dispatch id, from the same registry
    /// [`Event::MenuAction`] uses, and day-core routes it to the closure registered for it.
    /// A plain toolbar button has no value and emits `MenuAction` instead.
    ToolbarChanged {
        action: u64,
        value: ToolbarValue,
    },
    /// A searchable navigation surface's field changed (docs/search.md). Emitted against the nav
    /// host's node, not through the toolbar's dispatch registry: search belongs to the surface,
    /// so the field can move between the toolbar and the navigation list without the app
    /// re-registering anything.
    SearchChanged(String),
    /// The user picked a different scope. Carries the index into `SearchProps::scopes`.
    SearchScopeChanged(usize),
    /// The user chose one of `SearchProps::suggestions`, by index. The toolkit has already put
    /// that completion in the field and emitted [`Event::SearchChanged`] for it; this says which
    /// one, for an app that wants to act on the choice itself.
    SearchSuggestionChosen(usize),
    /// The user submitted the inline field's text with its Return or Search key. Emitted
    /// after the last [`Event::SearchChanged`], with the text as submitted, for a search that
    /// runs on demand (a server query) rather than narrowing as the text changes.
    SearchSubmitted(String),
    /// The app moved through a lifecycle phase (docs/lifecycle.md). Backends emit this from the
    /// native app/activity delegate; day-core routes it to the app's `on_lifecycle` handlers.
    Lifecycle(Lifecycle),
    /// A secondary native window closed (the title-bar close, a platform gesture such as an
    /// app-switcher swipe or an activity finish, or `Toolkit::close_window`); emitted on the
    /// window's root node after the platform committed the close. day-core disposes the
    /// window's subtree on receipt, so native and programmatic closes share one teardown
    /// path (docs/windows.md). Never emitted for the primary window (its close terminates
    /// the app).
    WindowClosed,
    /// A secondary window's key/active state changed; emitted on the window's root node.
    /// day-core tracks the focused window with it (dialog parenting, dayscript targeting).
    WindowFocused(bool),
    /// A window's display state changed (docs/windows.md): the user minimized, zoomed or took it
    /// fullscreen, or a [`WindowChange::State`] request landed. Emitted on the window's root node
    /// (the primary's may arrive on [`WINDOW_NODE`]); day-core feeds the window's `state()`
    /// signal with it, so the app reads what the platform did rather than what it asked for.
    WindowStateChanged(WindowState),
    /// A native list committed a drag-reorder (docs/list.md): row `from` landed at row `to`.
    /// Emitted on the `LIST` node by the list piece's commit hook: the reorder driver
    /// rotates its snapshot synchronously inside the platform's drop callback, then reports
    /// through the event queue so the app's `on_reorder` never runs inside a native animation
    /// callstack.
    ListReorder {
        from: usize,
        to: usize,
    },
    /// A regrouping sidebar committed a drop (`props::NavMenuProps::regroup`, docs/navigation.md
    /// "Moving rows between sections"), in item indices into the rows as they were before the
    /// move: row `from` now sits in the section whose first row is item `section` (item 0 for
    /// rows above the first header), before item `before`, or at that section's end for
    /// `None`. `before` is never `from`. Emitted on the `NAV_MENU` node by a toolkit whose
    /// `Cap::NavReorder` is `Native`, deferred through the event queue exactly as
    /// [`Event::ListReorder`] is; the nav validates it against the app's rules before reporting.
    NavRegroup {
        from: usize,
        section: usize,
        before: Option<usize>,
    },
    /// A native list committed a swipe-delete (docs/list.md): the row at this index is gone.
    /// Deferred through the event queue exactly as [`Event::ListReorder`] is.
    ListDelete(usize),
    /// The native pull-to-refresh control requested an update. The app owns its busy state.
    RefreshRequested,
    /// A native list activated a swipe action (docs/list.md): the user pressed button `action`
    /// (an index into the row's [`ListSwipe::actions_at`] offer for `edge`) on row `index`, or
    /// swiped clear across for the edge's first action. Emitted on the `LIST` node by the list
    /// piece's commit hook and deferred through the event queue exactly as
    /// [`Event::ListReorder`] is, so the app's handler never runs inside a native animation
    /// callstack.
    ListSwipe {
        index: usize,
        edge: SwipeEdge,
        action: usize,
    },
    /// The platform dismissed the popover whose content root received this event.
    PopoverDismissed,
    /// A fullscreen cover's hide transition finished (docs/cover.md): the content can now be
    /// disposed. Answers [`props::CoverPatch::Dismiss`]; delivery is a hard guarantee, and
    /// backends over-report rather than under-report; consumers gate on their own closing
    /// flag, so duplicates and belated reports are no-ops.
    CoverHidden,
    /// The user showed or hid an inspector pane through a native affordance (a dock widget's
    /// close button, a divider dragged shut) rather than through Day (docs/inspector.md).
    /// Emitted on the `kinds::INSPECTOR` node, and only when the visibility actually changed;
    /// the piece writes it back to the bound signal. Applying
    /// [`props::InspectorPatch::Visible`] must never re-emit it (the from-native echo rule).
    InspectorChanged(bool),
    /// The user disclosed or collapsed a tree row through the native affordance (docs/tree.md).
    /// Emitted on the `kinds::TREE` node; the piece writes it back to the app's expansion
    /// signal. Applying [`props::TreePatch::Expand`] must never re-emit it (the from-native
    /// echo rule).
    TreeExpanded {
        token: u64,
        expanded: bool,
    },
    /// A native tree committed a drag move (docs/tree.md): `token` now sits under `parent`
    /// (`None` = the root) at `index` (`None` = dropped onto the parent, an append). Deferred
    /// through the event queue exactly as [`Event::ListReorder`] is; the app's `on_move`
    /// writes its own data, whose refresh reloads the tree.
    TreeMove {
        token: u64,
        parent: Option<u64>,
        index: Option<usize>,
    },
    /// A tree's selection changed: the full set of selected row tokens (empty = cleared).
    /// Token-addressed, since a tree cannot speak row indices: expanding a row renumbers
    /// everything below it (docs/tree.md).
    TreeSelection(Vec<u64>),
}

impl Event {
    /// Build a text-carrying [`Event::Custom`] (with `num` defaulted to 0), the common case for an
    /// in-process piece reporting a value back: `emit(node, Event::custom("webview:url", url))`.
    pub fn custom(tag: &'static str, text: impl Into<String>) -> Event {
        Event::Custom {
            tag,
            num: 0.0,
            text: text.into(),
        }
    }
}

/// An app-lifecycle phase (docs/lifecycle.md). Each backend maps these onto its OS's native app /
/// activity delegate. Some phases only exist on some platforms (a mobile app enters the
/// background and can be low on memory, a desktop app cannot), so [`Lifecycle::is_universal`]
/// marks the ones every backend delivers, and [`Toolkit::supports_lifecycle`] reports per-backend
/// truth.
///
/// Rough native mapping:
///
/// | phase | AppKit | UIKit | GTK | Qt | Android | XAML |
/// |---|---|---|---|---|---|---|
/// | `WillLaunch` / `DidLaunch` | `applicationWill/DidFinishLaunching` | same | `startup`/mount | mount | `onCreate` | window create |
/// | `DidBecomeActive` | `didBecomeActive` | `didBecomeActive` | `notify::is-active` | `ApplicationActive` | `onResume` | `Activated` |
/// | `WillResignActive` | `willResignActive` | `willResignActive` | `notify::is-active` | `ApplicationInactive` | `onPause` | `Deactivated` |
/// | `WillEnterForeground` | — | `willEnterForeground` | — | — | `onStart` | — |
/// | `DidEnterBackground` | — | `didEnterBackground` | — | — | `onStop` | — |
/// | `DidReceiveMemoryWarning` | — | `didReceiveMemoryWarning` | — | — | `onTrimMemory` | — |
/// | `WillTerminate` | `willTerminate` | `willTerminate` | `shutdown` | `aboutToQuit` | `onDestroy` | window close |
/// | `DidExit` | after `willTerminate` | after `willTerminate` | `run` returned | `exec` returned | after `onDestroy` | loop returned |
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lifecycle {
    /// Before the window and UI are built: the first thing to run. Set up global state here.
    WillLaunch,
    /// The UI is mounted and the app is about to start running. Kick off startup work here.
    DidLaunch,
    /// The app came to the foreground and is the active, focused app receiving input.
    DidBecomeActive,
    /// The app is about to stop being active (an interruption, app switch, or losing focus).
    WillResignActive,
    /// The app is about to return to the foreground (mobile). Refresh what background invalidated.
    WillEnterForeground,
    /// The app left the foreground and is no longer visible (mobile). Persist state, release UI work.
    DidEnterBackground,
    /// The system is low on memory (mobile). Drop caches and non-essential memory now.
    DidReceiveMemoryWarning,
    /// The app is about to terminate: the last chance to save. Triggered by the Quit command,
    /// the platform's quit shortcut, or the OS reclaiming the app.
    WillTerminate,
    /// The run loop is over and nothing of the app will run after this: the last phase, after
    /// every `WillTerminate` handler. Day logs its exit line ("Clean exit for …") once the
    /// handlers have run, so a handler here is the app's last word before it.
    DidExit,
}

impl Lifecycle {
    /// Every phase, in delivery order (launch → run → quit). Handy for logging/registration sweeps.
    pub const ALL: [Lifecycle; 9] = [
        Lifecycle::WillLaunch,
        Lifecycle::DidLaunch,
        Lifecycle::DidBecomeActive,
        Lifecycle::WillResignActive,
        Lifecycle::WillEnterForeground,
        Lifecycle::DidEnterBackground,
        Lifecycle::DidReceiveMemoryWarning,
        Lifecycle::WillTerminate,
        Lifecycle::DidExit,
    ];

    /// True for phases every backend delivers (launch, activation, termination). The remaining
    /// phases (`WillEnterForeground`, `DidEnterBackground`, `DidReceiveMemoryWarning`) are mobile
    /// concepts and are only delivered by the mobile backends. `const` so it composes into a
    /// backend's `const fn lifecycle_supported` and thus into compile-time guards.
    #[must_use]
    pub const fn is_universal(self) -> bool {
        matches!(
            self,
            Lifecycle::WillLaunch
                | Lifecycle::DidLaunch
                | Lifecycle::DidBecomeActive
                | Lifecycle::WillResignActive
                | Lifecycle::WillTerminate
                | Lifecycle::DidExit
        )
    }

    /// A stable, human-readable name (for logs/warnings).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Lifecycle::WillLaunch => "WillLaunch",
            Lifecycle::DidLaunch => "DidLaunch",
            Lifecycle::DidBecomeActive => "DidBecomeActive",
            Lifecycle::WillResignActive => "WillResignActive",
            Lifecycle::WillEnterForeground => "WillEnterForeground",
            Lifecycle::DidEnterBackground => "DidEnterBackground",
            Lifecycle::DidReceiveMemoryWarning => "DidReceiveMemoryWarning",
            Lifecycle::WillTerminate => "WillTerminate",
            Lifecycle::DidExit => "DidExit",
        }
    }
}

/// The phase of a drag gesture (docs/shapes.md).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragPhase {
    Began,
    Changed,
    Ended,
}

/// The one axis a drag follows (`Decorate::on_drag_along`, [`Toolkit::set_drag_axis`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DragAxis {
    Horizontal,
    Vertical,
}

impl DragAxis {
    /// Whether a movement of `translation` runs along this axis more than across it.
    #[must_use]
    pub fn leads(self, translation: Point) -> bool {
        match self {
            DragAxis::Horizontal => translation.x.abs() >= translation.y.abs(),
            DragAxis::Vertical => translation.y.abs() >= translation.x.abs(),
        }
    }
}

/// A gesture a node wants delivered. Backends attach the matching native recognizer when day-core
/// calls [`Toolkit::enable_gesture`]; the default is no gesture (recognizers cost, so opt-in).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GestureKind {
    Tap,
    /// A press held still (`Event::LongPress`).
    LongPress,
    /// Two taps in quick succession (`Event::DoubleTap`).
    DoubleTap,
    /// Press-in and press-out (`Event::Press`), reported at the pointer-down itself.
    Press,
    /// The raw contacts on the node (`Event::Touches`).
    Touches,
    Drag,
    /// The pointer moving over the node without pressing (docs/canvas.md "Interaction"), and
    /// leaving it. Delivered only where a pointer exists: every desktop, an iPad with a trackpad
    /// or pencil, an Android device with a mouse or stylus. A touch-only device reports none,
    /// which is the right answer, not a gap, so anything reachable by hover must also be
    /// reachable by a tap.
    Hover,
    /// Pinch/magnify (docs/shapes.md): trackpad magnification, two-finger touch pinch.
    Pinch,
    /// Rotation (docs/canvas.md "Zoom and pan"): trackpad rotate, two-finger touch twist.
    Rotate,
    /// Viewport pan (docs/shapes.md): trackpad two-finger scroll, two-finger touch pan.
    /// Distinct from [`GestureKind::Drag`], the primary pointer's press-drag.
    Pan,
}

// ---------------------------------------------------------------------------
// Menus (app menu bar + context menus). The model is a toolkit-neutral tree; each backend renders
// it with its native affordance (NSMenu / GMenu+GtkPopoverMenu / QMenu / UIMenu / Android
// PopupMenu / XAML MenuFlyout) and its own conventions, so day imposes no menu manager of its own.
// ---------------------------------------------------------------------------

/// A keyboard shortcut for a menu item. `primary` is the platform's command modifier (⌘ on Apple,
/// Ctrl on GTK/Qt/XAML) so one declaration reads correctly everywhere. `key` is a single character
/// (`"s"`, `"."`) or a named key (`"Return"`, `"Delete"`, `"Left"`, `"F1"`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Shortcut {
    pub key: String,
    /// ⌘ (Apple) / Ctrl (elsewhere). The conventional command modifier.
    pub primary: bool,
    pub shift: bool,
    /// ⌥ / Alt.
    pub alt: bool,
    /// Literal Control (⌃ on Apple). Rare; prefer `primary` for the command modifier.
    pub control: bool,
}

/// The key of a [`Shortcut`], in the menu vocabulary its `key` field holds. Converts from a
/// typed [`Key`] (`Key::Enter` is `"Return"`), a `char`, or a name already in that vocabulary
/// (`"s"`, `"Return"`, `"Left"`, `"F1"`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortcutKey(pub String);

impl From<Key> for ShortcutKey {
    fn from(key: Key) -> ShortcutKey {
        ShortcutKey(key.shortcut_name())
    }
}
impl From<char> for ShortcutKey {
    fn from(c: char) -> ShortcutKey {
        ShortcutKey(c.to_string())
    }
}
impl From<&str> for ShortcutKey {
    fn from(name: &str) -> ShortcutKey {
        ShortcutKey(name.to_owned())
    }
}
impl From<String> for ShortcutKey {
    fn from(name: String) -> ShortcutKey {
        ShortcutKey(name)
    }
}

impl Shortcut {
    /// `primary`+`key` (⌘S / Ctrl+S), the common case: `Shortcut::primary('s').shift()`,
    /// `Shortcut::primary(Key::Enter)`.
    pub fn primary(key: impl Into<ShortcutKey>) -> Shortcut {
        Shortcut {
            key: key.into().0,
            primary: true,
            ..Default::default()
        }
    }
    /// `key` with no modifiers (`Shortcut::plain(Key::F(1))`, plain `Key::Delete`).
    pub fn plain(key: impl Into<ShortcutKey>) -> Shortcut {
        Shortcut {
            key: key.into().0,
            ..Default::default()
        }
    }
    /// The shortcut's modifiers as a set, for code that compares it with a [`KeyEvent`].
    #[must_use]
    pub fn modifiers(&self) -> Modifiers {
        Modifiers::NONE
            .with(Modifiers::PRIMARY, self.primary)
            .with(Modifiers::SHIFT, self.shift)
            .with(Modifiers::ALT, self.alt)
            .with(Modifiers::CONTROL, self.control)
    }
    /// Whether a key event is this shortcut: the same key (letters compare without case, as
    /// accelerators do) with exactly these modifiers held.
    #[must_use]
    pub fn matches(&self, event: &KeyEvent) -> bool {
        let mut mods = event.modifiers;
        // Off Apple platforms Control IS the primary key, and a backend sets both bits.
        if self.primary && !self.control && !cfg!(target_vendor = "apple") {
            mods = Modifiers::from_bits(mods.bits() & !Modifiers::CONTROL.bits());
        }
        mods == self.modifiers() && event.key.shortcut_name().eq_ignore_ascii_case(&self.key)
    }
    #[must_use]
    pub fn shift(mut self) -> Shortcut {
        self.shift = true;
        self
    }
    #[must_use]
    pub fn alt(mut self) -> Shortcut {
        self.alt = true;
        self
    }
    #[must_use]
    pub fn control(mut self) -> Shortcut {
        self.control = true;
        self
    }
}

/// A standard/system command. The backend supplies the native item, a selector on AppKit/UIKit
/// (`cut:`/`copy:`/`paste:`…) or a stock action on GTK/Qt/XAML, so it targets the focused
/// control, gets the platform's default label + shortcut, and enables/disables itself
/// automatically. This is how default items (Edit ▸ Cut/Copy/Paste, the app's Quit/About) are
/// accommodated without the app re-implementing them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuRole {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Undo,
    Redo,
    Delete,
    /// The app's About command (docs/settings.md). With an About registered through
    /// `day::register_about` the item lowers to that dispatch action (a Day-rendered About
    /// window, or the toolkit's own About dialog where one exists); without one AppKit shows
    /// its stock Info.plist panel and the other desktops show the label alone.
    About,
    Quit,
    Preferences,
    Minimize,
    CloseWindow,
    Fullscreen,
    /// File ▸ New Window (docs/windows.md): opens another window through the builder the
    /// app registered with `day::register_new_window`. No platform has a native selector
    /// for it, so the item lowers to the registered dispatch action (disabled when none).
    NewWindow,
    /// Check for Updates… (docs/settings.md): placed directly under About in the macOS App
    /// menu (the Sparkle convention) and in Help elsewhere. Lowers to the action an updater
    /// registered with `day::register_check_for_updates`; absent from the bar when none did.
    CheckForUpdates,
}

/// The keyboard modifiers held at an interaction: what a [`KeyEvent`] carries and what
/// `day::modifiers()` answers ambiently ([`Toolkit::modifiers`]). A small set with a method
/// per key: `event.modifiers.shift()`, `Modifiers::PRIMARY | Modifiers::SHIFT`.
/// Touch-only backends answer [`Modifiers::NONE`].
///
/// [`Modifiers::bits`] is the mask a backend's numeric wire carries
/// ([`bridge::BridgeKind::Key`]), and [`Modifiers::from_bits`] reads it back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    /// No modifier held.
    pub const NONE: Modifiers = Modifiers(0);
    /// Shift.
    pub const SHIFT: Modifiers = Modifiers(1);
    /// The platform's command key: ⌘ on Apple platforms, Ctrl elsewhere (the
    /// [`Shortcut::primary`] convention).
    pub const PRIMARY: Modifiers = Modifiers(2);
    /// Alt (Option on Apple platforms).
    pub const ALT: Modifiers = Modifiers(4);
    /// The Control key itself, wherever it sits: on Apple platforms it is a different key
    /// from [`Modifiers::PRIMARY`] (⌘), elsewhere the same one, so a Ctrl+C on Linux sets
    /// both. Carried by key events on the full route (`Decorate::keyboard`).
    pub const CONTROL: Modifiers = Modifiers(8);

    const ALL_BITS: u8 = 15;

    /// The wire mask: one bit per modifier, in declaration order from bit 0.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
    /// The set a wire mask spells; bits above the four modifiers are ignored.
    #[must_use]
    pub const fn from_bits(bits: u8) -> Modifiers {
        Modifiers(bits & Self::ALL_BITS)
    }
    /// Whether no modifier is held.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }
    /// Whether every modifier in `other` is held.
    #[must_use]
    pub const fn contains(self, other: Modifiers) -> bool {
        self.0 & other.0 == other.0
    }
    /// This set with `other` added when `held`: how a backend builds the set from the
    /// platform's own flags (`Modifiers::NONE.with(Modifiers::SHIFT, flags.shift)`).
    #[must_use]
    pub const fn with(self, other: Modifiers, held: bool) -> Modifiers {
        if held {
            Modifiers(self.0 | other.0)
        } else {
            self
        }
    }
    /// Whether Shift is held.
    #[must_use]
    pub const fn shift(self) -> bool {
        self.contains(Modifiers::SHIFT)
    }
    /// Whether the platform's command key is held.
    #[must_use]
    pub const fn primary(self) -> bool {
        self.contains(Modifiers::PRIMARY)
    }
    /// Whether Alt (Option) is held.
    #[must_use]
    pub const fn alt(self) -> bool {
        self.contains(Modifiers::ALT)
    }
    /// Whether the Control key itself is held.
    #[must_use]
    pub const fn control(self) -> bool {
        self.contains(Modifiers::CONTROL)
    }
}

impl std::ops::BitOr for Modifiers {
    type Output = Modifiers;
    fn bitor(self, rhs: Modifiers) -> Modifiers {
        Modifiers(self.0 | rhs.0)
    }
}

impl std::ops::BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Modifiers) {
        self.0 |= rhs.0;
    }
}

/// A wire mask ([`Modifiers::from_bits`]): what a backend's key trampoline holds.
impl From<u8> for Modifiers {
    fn from(bits: u8) -> Modifiers {
        Modifiers::from_bits(bits)
    }
}

/// A standard edit command a platform route delivered ([`Event::Edit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOp {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

/// What the app's edit bridge can currently do, mirrored into the toolkit
/// ([`Toolkit::set_edit_state`]) so native menu validation enables the stock items.
/// `can_paste` is the app's half ("a paste handler is installed"); the toolkit combines it
/// with its own clipboard-has-text check where one exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditState {
    pub can_cut: bool,
    pub can_copy: bool,
    pub can_paste: bool,
    pub can_select_all: bool,
}

/// The undo stack's face, as a native front mirrors it ([`Toolkit::set_undo_state`]): what is
/// possible and what the menu titles should say. Labels arrive already localized: a toolkit
/// cannot invent translated text, the same rule as every other label in this file.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct UndoState {
    pub can_undo: bool,
    pub can_redo: bool,
    /// The next undo unit's display label ("Rename"), empty when nothing is undoable. Fronts
    /// interpolate it into the platform's own title form ("Undo Rename").
    pub undo_label: String,
    pub redo_label: String,
}

/// One entry in a menu (recursive: a `Submenu` nests).
#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    /// A command. `action` (nonzero) dispatches [`Event::MenuAction`] to the app; a `role`-only item
    /// uses the native standard command instead (action 0). `label`/`shortcut` override the role's
    /// defaults. `icon` is the platform's own glyph beside the title where menus carry one (macOS,
    /// Windows, GNOME, KDE, Android); a backend whose menus are text-only ignores it.
    ///
    /// The `id`/`action` split is [`ToolbarItem`]'s: `id` names the item and `action` is what it
    /// runs. They are separate because a label cannot serve as a name (it is localized, and an
    /// item that shows its own state changes it), so anything addressing an item from outside
    /// (dayscript's `menu: { id: … }`) needs something that does not move.
    Action {
        /// The app's own name for this item, where it gave one. Stable across locales and across
        /// a change of `checked`, and unique within the menu it is installed in.
        id: Option<String>,
        /// The item's command, as a dispatch id from the same registry [`Event::MenuAction`] uses
        /// (0 = no command), so a toolbar button and its menu twin can share one closure.
        action: u64,
        label: String,
        shortcut: Option<Shortcut>,
        enabled: bool,
        /// On/off state where the item stands for a setting or one of several choices, drawn with
        /// the platform's own check mark. `None` is a plain command and reserves no mark; `Some`
        /// makes the item checkable and says which way it currently sits.
        ///
        /// The item still runs its `action` when chosen; a backend never flips this on its own.
        /// The app owns the state, and re-installing the menu (`app_menu`, which re-lowers on change) is what moves
        /// the mark, so what the menu shows can never disagree with what the app thinks.
        checked: Option<bool>,
        role: Option<MenuRole>,
        icon: Option<Icon>,
    },
    /// A nested submenu. At the top level of an app menu a `role` claims one of the platform's
    /// standard menu-bar slots: the backend then uses this menu instead of its stock one, and
    /// places it where the platform expects that menu to sit.
    Submenu {
        label: String,
        items: Vec<MenuItem>,
        role: Option<MenuBarRole>,
    },
    /// A visual separator.
    Separator,
}

/// A standard menu-bar slot (macOS's File/Edit/View/Window/Help, and their counterparts
/// elsewhere). A backend that has a house style for the menu bar fills every slot an app did
/// not claim with its own stock menu, so an app never has to restate the platform's furniture,
/// and can still replace any of it by tagging its own submenu with the matching role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuBarRole {
    File,
    Edit,
    View,
    /// KDE/Plasma's Settings menu (`Configure <App>`, Configure Shortcuts). No counterpart on
    /// macOS, where preferences live in the app menu, or on GNOME/Windows.
    Settings,
    /// macOS's Window menu. Windows and Linux desktops do not have one.
    Window,
    Help,
}

// ---------------------------------------------------------------------------
// Window toolbars (docs/toolbars.md)
// ---------------------------------------------------------------------------

/// A standard icon, named by what it means rather than by how it looks, so each backend can
/// draw the platform's own glyph for it: an SF Symbol on macOS, a freedesktop icon name on
/// GTK and Qt, a Fluent glyph on Windows. This is the only way an icon looks native on every
/// desktop at once; a bundled PNG cannot, because it is one artist's take on all four.
///
/// The set is kept small: the commands that recur across toolbars. Anything
/// app-specific is an [`Icon::Image`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Symbol {
    Add,
    Remove,
    Delete,
    Edit,
    New,
    Open,
    Save,
    Print,
    Refresh,
    Search,
    Share,
    Settings,
    Info,
    Star,
    Bookmark,
    Back,
    Forward,
    Up,
    Down,
    Home,
    /// Show/hide the sidebar, the leading item of most desktop toolbars.
    Sidebar,
    Filter,
    Sort,
    /// An overflow affordance (macOS `ellipsis`, Android's vertical `more_vert`, GNOME's hamburger,
    /// Fluent `More`).
    More,
    Play,
    Pause,
    Stop,
    /// A photo capture: a screenshot command, a camera control.
    Camera,
    /// Source code.
    Code,
    /// Light appearance (a sun). The three below are the vocabulary of a theme chooser, which
    /// every platform draws with the same three ideas.
    Light,
    /// Dark appearance (a moon).
    Dark,
    /// Follow the system: the half-filled circle every platform uses for "automatic".
    Auto,
    ZoomIn,
    ZoomOut,
    /// Back to actual size: the 100% zoom command between [`Symbol::ZoomIn`] and
    /// [`Symbol::ZoomOut`].
    ZoomReset,
    Undo,
    Redo,
    Copy,
    Cut,
    Paste,
    Mail,
    Folder,
    Document,
    Check,
    Close,
    Warning,
    /// A rectangle: the shape vocabulary a drawing surface needs (docs/shapes.md), and the
    /// glyphs every drawing, diagram and annotation tool puts in the same place.
    Rectangle,
    /// An ellipse.
    Oval,
    /// An outlined circle: the read/unread vocabulary (a read row, its dot removed), and the
    /// generic ring glyph.
    Circle,
    /// A filled circle: the unread dot itself.
    CircleFilled,
    /// A straight line segment.
    Line,
    /// Combine the selection into a group: the drawing-tool command (two stacked squares).
    Group,
    /// Dissolve a group back into its members (two separated squares).
    Ungroup,
    /// Text: a line of type as a drawing element (the drawing-tool "T").
    Text,
    /// The device's own position: a map's "show my location" button (Apple's arrow, the
    /// crosshair elsewhere).
    Location,
    /// The open tabs or windows: a browser's tab switcher (two overlapping squares).
    Tabs,
    /// A book: a library of bookmarks or saved reading (Safari's bookmarks button).
    Book,
    /// Past activity: a history list (a clock with a turning arrow).
    History,
}

/// The SF Symbol each standard symbol draws as. The system's own glyphs match the user's
/// device: weight, optical size, accent color and all. Shared by both Apple backends
/// (toolbars on macOS, menus on both).
///
/// The match is exhaustive so that a new [`Symbol`] must name its glyph here rather than
/// silently drawing nothing on Apple. A name this OS release does not know still resolves to
/// no image, and the item falls back to its label.
#[must_use]
pub fn sf_symbol_name(s: Symbol) -> &'static str {
    match s {
        Symbol::Add => "plus",
        Symbol::Remove => "minus",
        Symbol::Delete => "trash",
        Symbol::Edit => "pencil",
        Symbol::New => "square.and.pencil",
        Symbol::Open | Symbol::Folder => "folder",
        Symbol::Save => "square.and.arrow.down",
        Symbol::Print => "printer",
        Symbol::Refresh => "arrow.clockwise",
        Symbol::Search => "magnifyingglass",
        Symbol::Share => "square.and.arrow.up",
        Symbol::Settings => "gearshape",
        Symbol::Info => "info.circle",
        Symbol::Star => "star",
        Symbol::Bookmark => "bookmark",
        Symbol::Back => "chevron.backward",
        Symbol::Forward => "chevron.forward",
        Symbol::Up => "chevron.up",
        Symbol::Down => "chevron.down",
        Symbol::Home => "house",
        Symbol::Sidebar => "sidebar.leading",
        Symbol::Filter => "line.3.horizontal.decrease",
        Symbol::Sort => "arrow.up.arrow.down",
        Symbol::More => "ellipsis",
        Symbol::Play => "play.fill",
        Symbol::Pause => "pause.fill",
        Symbol::Stop => "stop.fill",
        Symbol::Camera => "camera",
        Symbol::Code => "chevron.left.forwardslash.chevron.right",
        Symbol::Light => "sun.max",
        Symbol::Dark => "moon",
        Symbol::Auto => "circle.lefthalf.filled",
        Symbol::ZoomIn => "plus.magnifyingglass",
        Symbol::ZoomReset => "1.magnifyingglass",
        Symbol::ZoomOut => "minus.magnifyingglass",
        Symbol::Undo => "arrow.uturn.backward",
        Symbol::Redo => "arrow.uturn.forward",
        Symbol::Copy => "doc.on.doc",
        Symbol::Cut => "scissors",
        Symbol::Paste => "doc.on.clipboard",
        Symbol::Mail => "envelope",
        Symbol::Document => "doc",
        Symbol::Check => "checkmark",
        Symbol::Close => "xmark",
        Symbol::Warning => "exclamationmark.triangle",
        Symbol::Rectangle => "rectangle",
        Symbol::Oval => "oval",
        Symbol::Circle => "circle",
        Symbol::CircleFilled => "circle.fill",
        Symbol::Line => "line.diagonal",
        Symbol::Group | Symbol::Tabs => "square.on.square",
        Symbol::Ungroup => "square.on.square.dashed",
        Symbol::Text => "textformat",
        Symbol::Location => "location",
        Symbol::Book => "book",
        Symbol::History => "clock.arrow.circlepath",
    }
}

/// A button or toolbar item's picture: a standard [`Symbol`] (drawn with the platform's own icon set) or
/// a bundled image from `resource/images` for something only this app has.
#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    Symbol(Symbol),
    /// A bundled image name, resolved the same way [`ImageName`] is elsewhere.
    Image(String),
}

/// What a toolbar item is. The variants are the vocabulary every desktop toolbar shares; each
/// backend realizes one with its native control, never with a drawn imitation.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarItemKind {
    /// A push button running the item's `action`.
    Button,
    /// A two-state button. `on` seeds it; the user flipping it emits
    /// [`ToolbarValue::On`], and the app's own writes arrive as [`ToolbarPatch::On`].
    Toggle { on: bool },
    /// A button that drops a menu: the same [`MenuItem`] model the menu bar uses, so a
    /// toolbar menu and its menu-bar twin are one list of commands.
    Menu { items: Vec<MenuItem> },
    /// A search field. Edits emit [`ToolbarValue::Text`]; the app's own writes arrive as
    /// [`ToolbarPatch::Text`].
    ///
    /// Day creates this item itself, from a `.searchable()` surface whose placement resolved to
    /// the toolbar (docs/search.md); apps declare search on the surface, never here.
    Search {
        text: String,
        placeholder: String,
        /// Completions for the current text, drawn by the field's own completion affordance
        /// where it has one (`AutoSuggestBox`, `QCompleter`, `<datalist>`). Empty = none.
        suggestions: Vec<String>,
    },
    /// A row of mutually exclusive choices drawn as one control: the native segmented control
    /// (`NSSegmentedControl`, a linked GTK/Qt button box, a XAML toggle strip, `.day-segmented`
    /// on the web).
    ///
    /// This is what a set of related toolbar toggles should be when exactly one of them is on at
    /// a time: three separate toggles say "three independent switches" to the eye and to a screen
    /// reader, and leave the app to keep them exclusive. Choosing a segment emits
    /// [`ToolbarValue::Selected`]; the app's own writes arrive as [`ToolbarPatch::Selected`].
    Segmented {
        /// One entry per segment. A segment with an icon draws it in place of its title where
        /// the platform does that, and keeps the title as its accessible name.
        segments: Vec<ToolbarSegment>,
        selected: usize,
    },
    /// Static text, for a status or a caption.
    Label,
    /// A divider between neighbors in the same placement bucket, where the platform draws one
    /// (macOS toolbars have no separator, so AppKit renders it as a fixed space,
    /// docs/toolbars.md). Grouping is all it does: alignment is [`ToolbarPlacement`]'s job.
    Separator,
}

/// One choice inside a [`ToolbarItemKind::Segmented`] control.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarSegment {
    /// The segment's name: its label where the platform shows text, its accessible name always.
    pub title: String,
    pub icon: Option<Icon>,
}

/// The reserved id of the sidebar affordance a `nav(Sidebar)` supplies for itself
/// (docs/toolbars.md). Backends with a system item for the job (AppKit's
/// `NSToolbarToggleSidebarItem`) recognize it by this id and draw theirs instead; dayscript
/// recognizes it to drive the toolkit's own duty rather than an app closure.
pub const SIDEBAR_TOGGLE_ID: &str = "day::sidebar_toggle";

/// The reserved id of a `.searchable()` surface's field (docs/search.md): the `ToolbarItem` a
/// desktop bar draws for it, and the name dayscript's `toolbar:` step drives it by wherever the
/// platform put it — on the bar, or inline on the navigation surface where the bar holds no
/// field, in which case day-core resolves the id to that host instead.
pub const SEARCH_ITEM_ID: &str = "day::search";

/// One item in a window's toolbar (docs/toolbars.md).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarItem {
    /// Stable identity: the native item identifier, the dayscript target, and the key a
    /// [`ToolbarPatch`] addresses. Unique within a toolbar.
    pub id: String,
    pub kind: ToolbarItemKind,
    /// The item's name. Shown beside or below the icon where the platform does that, and used
    /// verbatim in the overflow and customization menus, so it is never optional, even on a
    /// toolbar that only shows icons. It is also the item's accessible name.
    pub label: String,
    /// Hover help. Defaults to `label` where the platform expects a tooltip and none is given.
    pub tooltip: Option<String>,
    pub icon: Option<Icon>,
    pub enabled: bool,
    /// The item's command, as a dispatch id from the same registry [`Event::MenuAction`] uses
    /// (0 = no command), so a toolbar button and its menu twin can share one closure.
    pub action: u64,
    /// This item's role on the chrome carrying it (docs/toolbars.md). It never names a surface:
    /// Which chrome an item rides is decided by where the app declared it, so this says only
    /// where on that chrome it belongs.
    pub placement: ToolbarPlacement,
    /// Whether the item draws its title, its icon, or both, where the platform can do more than
    /// one. An item folded into an overflow menu shows its title whatever this asks for; a menu
    /// row with no words is not a menu row.
    pub label_style: LabelStyle,
    /// Draw this item in the platform's emphasized style (a tinted/filled bar button, a
    /// prominent `NSToolbarItem`). For the one action a chrome is really offering.
    pub prominent: bool,
    /// Which column of a multi-pane window this item came from (docs/toolbars.md), set by Day
    /// from the piece that declared it, never by an app.
    ///
    /// A desktop toolbar spans every column at once, and a three-pane app expects each column's
    /// commands to sit over that column: the sidebar's against the first divider, the list's over
    /// the list, the rest over the detail. That is what Mail, Notes and Finder do, and it is only
    /// expressible because the declaration site already said which pane the command belongs to.
    /// Backends that draw one undivided bar ignore it.
    pub column: ToolbarColumn,
}

/// Which pane of a window's navigation a toolbar item was declared in (docs/toolbars.md).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolbarColumn {
    /// The window itself, over no particular column. Draws with the detail's items, which is
    /// where a command that acts on the whole window belongs when the panes are side by side.
    #[default]
    Window,
    /// The sidebar / master column.
    Sidebar,
    /// The content-list column between the sidebar and the detail.
    List,
    /// The detail column.
    Detail,
}

/// Where an item sits on the chrome that carries it (docs/toolbars.md).
///
/// A placement is a role, never a surface. Which toolbar, navigation bar or column an item
/// rides follows from the piece that declared it, so an app never spells that out twice.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ToolbarPlacement {
    /// The natural spot on this chrome: after the leading group on a desktop toolbar, the
    /// trailing item group on a navigation bar, an `IF_ROOM` menu action on Android.
    #[default]
    Automatic,
    /// The leading edge: a pane toggle, a control that reveals what is to its left.
    Navigation,
    /// Centered: a title accessory, a transport, a status readout.
    Principal,
    /// Trailing, and the last thing a crowded chrome gives up.
    Primary,
    /// Trailing, and the first thing a crowded chrome folds into its overflow.
    Secondary,
    /// A bottom bar where the platform has one (iOS `toolbarItems`, an Android bottom app bar);
    /// [`ToolbarPlacement::Secondary`] everywhere else.
    Bottom,
}

/// Which halves of an item's label a platform draws, where it can draw more than one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LabelStyle {
    /// The platform's own convention for this chrome.
    #[default]
    Automatic,
    /// The icon alone (the title stays as the accessible name and the tooltip).
    IconOnly,
    /// The title alone, even where an icon was given.
    TitleOnly,
    /// Both, where the chrome has room for both.
    TitleAndIcon,
}

/// A value a toolbar item produced.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarValue {
    /// A search item's full new text.
    Text(String),
    /// A search item's text, submitted with the field's Return or Search key. Sent after the
    /// last [`ToolbarValue::Text`]; it changes no state of its own.
    Submit(String),
    /// A toggle item's new state.
    On(bool),
    /// A segmented item's newly chosen index.
    Selected(usize),
}

/// A targeted update to one live toolbar item: the path that keeps a bound signal in sync
/// without rebuilding the bar (which would drop the search field's focus mid-keystroke).
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarPatch {
    /// Focus a search control and select its text without changing its value.
    Focus { item: String },
    /// Replace a search item's text.
    Text { item: String, text: String },
    /// Set a toggle item's state.
    On { item: String, on: bool },
    /// Select a segmented item's index.
    Selected { item: String, index: usize },
    /// Enable or disable any item.
    Enabled { item: String, on: bool },
    /// Replace a search item's completions (docs/search.md). Targeted rather than a rebuild,
    /// because these change on every keystroke and rebuilding the bar would take the field's
    /// focus with it.
    Suggestions { item: String, list: Vec<String> },
    /// Replace an item's words (a language change): its label, which an icon-only item still
    /// carries for the overflow menu and assistive technology, and its tooltip. The item keeps
    /// its native widget, its action and its place.
    Label {
        item: String,
        label: String,
        tooltip: Option<String>,
    },
    /// Replace a search item's empty-state prompt (docs/search.md), in the live field.
    Placeholder { item: String, text: String },
}

impl ToolbarPatch {
    /// The id of the item this patch addresses.
    #[must_use]
    pub fn item(&self) -> &str {
        match self {
            ToolbarPatch::Focus { item }
            | ToolbarPatch::Text { item, .. }
            | ToolbarPatch::On { item, .. }
            | ToolbarPatch::Selected { item, .. }
            | ToolbarPatch::Enabled { item, .. }
            | ToolbarPatch::Suggestions { item, .. }
            | ToolbarPatch::Label { item, .. }
            | ToolbarPatch::Placeholder { item, .. } => item,
        }
    }

    /// Write this patch's value into `items`, the model of a bar. `false` when no item there
    /// takes it (an unknown id, or a kind the value does not apply to).
    pub fn apply_to(&self, items: &mut [ToolbarItem]) -> bool {
        let Some(it) = items.iter_mut().find(|i| i.id == self.item()) else {
            return false;
        };
        match (self, &mut it.kind) {
            (ToolbarPatch::Focus { .. }, ToolbarItemKind::Search { .. }) => (),
            (ToolbarPatch::Text { text, .. }, ToolbarItemKind::Search { text: t, .. }) => {
                t.clone_from(text);
            }
            (
                ToolbarPatch::Suggestions { list, .. },
                ToolbarItemKind::Search { suggestions, .. },
            ) => suggestions.clone_from(list),
            (ToolbarPatch::On { on, .. }, ToolbarItemKind::Toggle { on: o }) => *o = *on,
            (
                ToolbarPatch::Selected { index, .. },
                ToolbarItemKind::Segmented { segments, selected },
            ) if *index < segments.len() => *selected = *index,
            (ToolbarPatch::Enabled { on, .. }, _) => it.enabled = *on,
            (ToolbarPatch::Label { label, tooltip, .. }, _) => {
                it.label = label.clone();
                it.tooltip = tooltip.clone();
            }
            (
                ToolbarPatch::Placeholder { text, .. },
                ToolbarItemKind::Search { placeholder, .. },
            ) => placeholder.clone_from(text),
            _ => return false,
        }
        true
    }
}

/// One step of an edit to a window's toolbar (docs/toolbars.md).
///
/// Day hands a backend its bar as edits, never as a whole new model to rebuild, for the same
/// reason the view tree arrives as `insert`/`remove` rather than as a fresh tree: an item the
/// edit does not name keeps its native widget, and with it the keyboard focus, the caret, a
/// menu that is open. A value (search text, a toggle) never travels as an edit; it is a
/// [`ToolbarPatch`] on the live item.
///
/// An edit lists every [`ToolbarOp::Remove`] first, then every [`ToolbarOp::Insert`] in
/// ascending `index`, so applying them in order to the bar's current items (a
/// [`ToolbarMirror`] does exactly that) produces the new bar. Items keep their relative order
/// across an edit: an item that moves arrives as a remove and an insert, like one whose
/// presentation changed.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarOp {
    /// The item with this id leaves the bar.
    Remove { id: String },
    /// `item` joins the bar at `index`, its position once this edit is complete.
    Insert { index: usize, item: ToolbarItem },
}

impl ToolbarOp {
    /// Apply this op to `items`, a bar's model in order.
    pub fn apply_to(&self, items: &mut Vec<ToolbarItem>) {
        match self {
            ToolbarOp::Remove { id } => items.retain(|i| i.id != *id),
            ToolbarOp::Insert { index, item } => {
                items.insert((*index).min(items.len()), item.clone());
            }
        }
    }
}

/// A backend's copy of one window's toolbar, kept in step with the [`ToolbarOp`]s and
/// [`ToolbarPatch`]es that reach it, and the one place a backend works out where a new item
/// goes (docs/toolbars.md).
///
/// Every backend lays its bar out as a few groups (the leading and trailing ends of a header
/// bar, the columns of a split window, the primary and overflow commands of a command bar) and
/// keeps model order within each. So an inserted item always goes right after
/// [`ToolbarMirror::prev_where`] its own group, or first in the group when that is `None`, and
/// nothing already on the bar moves.
#[derive(Clone, Debug, Default)]
pub struct ToolbarMirror {
    items: Vec<ToolbarItem>,
}

impl ToolbarMirror {
    /// The bar's items, in model order.
    #[must_use]
    pub fn items(&self) -> &[ToolbarItem] {
        &self.items
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// The item with this id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&ToolbarItem> {
        self.items.iter().find(|i| i.id == id)
    }

    /// The item dispatching `action`: how a backend whose native bar reports only an action
    /// id (a menu tap) finds what was tapped.
    #[must_use]
    pub fn by_action(&self, action: u64) -> Option<&ToolbarItem> {
        self.items
            .iter()
            .find(|i| i.action == action && action != 0)
    }

    /// Take the item with `id` out, returning it.
    pub fn remove(&mut self, id: &str) -> Option<ToolbarItem> {
        let at = self.items.iter().position(|i| i.id == id)?;
        Some(self.items.remove(at))
    }

    /// Put `item` at `index` (clamped to the end).
    pub fn insert(&mut self, index: usize, item: ToolbarItem) {
        let at = index.min(self.items.len());
        self.items.insert(at, item);
    }

    /// Apply one op to the model: what a backend that repaints its whole native bar from the
    /// model (a menu of actions) does with each.
    pub fn apply(&mut self, op: &ToolbarOp) {
        op.apply_to(&mut self.items);
    }

    /// Write a value patch into the model. `false` when no item takes it.
    pub fn patch(&mut self, patch: &ToolbarPatch) -> bool {
        patch.apply_to(&mut self.items)
    }

    /// The nearest item before `index` that `same_group` accepts: the native item a new one at
    /// `index` goes right after, in that group. `None` = it goes first in the group.
    pub fn prev_where(
        &self,
        index: usize,
        same_group: impl Fn(&ToolbarItem) -> bool,
    ) -> Option<&ToolbarItem> {
        self.items[..index.min(self.items.len())]
            .iter()
            .rev()
            .find(|i| same_group(i))
    }
}

/// A key on the keyboard, as [`KeyEvent::key`] names it (docs/menus.md "Keys").
///
/// A key that types is [`Key::Character`] with its unmodified character (`'a'`, `'A'` under
/// Shift, `'8'`, `'ü'` on a layout that has one, `' '` for the space bar); a key that types
/// nothing is one of the named variants. [`Key::Unidentified`] stands for text with no single
/// key behind it (an input method's commit) and for a key outside this vocabulary.
///
/// At the boundaries that stay text (a backend's wire, a dayscript `key:` step, a menu
/// [`Shortcut`]) a key is spelled by its web `KeyboardEvent.key` name: [`Key::from_name`]
/// reads one and [`Key::name`] writes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Key {
    /// A key that types, named by its unmodified character.
    Character(char),
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Enter,
    Tab,
    Escape,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    /// A function key, `F(1)` through `F(12)`.
    F(u8),
    /// No single key: text an input method committed, or a key this vocabulary does not name.
    Unidentified,
}

impl Key {
    /// The key a web `KeyboardEvent.key` name spells: a named key, `"F1"` through `"F12"`, a
    /// single character, or `"Space"` for the space bar. Anything else is
    /// [`Key::Unidentified`].
    #[must_use]
    pub fn from_name(name: &str) -> Key {
        match name {
            "ArrowLeft" => Key::ArrowLeft,
            "ArrowRight" => Key::ArrowRight,
            "ArrowUp" => Key::ArrowUp,
            "ArrowDown" => Key::ArrowDown,
            "Enter" => Key::Enter,
            "Tab" => Key::Tab,
            "Escape" => Key::Escape,
            "Backspace" => Key::Backspace,
            "Delete" => Key::Delete,
            "Home" => Key::Home,
            "End" => Key::End,
            "PageUp" => Key::PageUp,
            "PageDown" => Key::PageDown,
            "Insert" => Key::Insert,
            "Space" => Key::Character(' '),
            _ => {
                if let Some(n) = name.strip_prefix('F').and_then(|n| n.parse::<u8>().ok())
                    && (1..=12).contains(&n)
                {
                    return Key::F(n);
                }
                let mut chars = name.chars();
                match (chars.next(), chars.next()) {
                    (Some(c), None) => Key::Character(c),
                    _ => Key::Unidentified,
                }
            }
        }
    }

    /// The web `KeyboardEvent.key` name of this key: `"ArrowLeft"`, `"F5"`, `"a"`, and the
    /// empty string for [`Key::Unidentified`]. The inverse of [`Key::from_name`].
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Key::Character(c) => c.to_string(),
            Key::F(n) => format!("F{n}"),
            Key::Unidentified => String::new(),
            Key::ArrowLeft => "ArrowLeft".into(),
            Key::ArrowRight => "ArrowRight".into(),
            Key::ArrowUp => "ArrowUp".into(),
            Key::ArrowDown => "ArrowDown".into(),
            Key::Enter => "Enter".into(),
            Key::Tab => "Tab".into(),
            Key::Escape => "Escape".into(),
            Key::Backspace => "Backspace".into(),
            Key::Delete => "Delete".into(),
            Key::Home => "Home".into(),
            Key::End => "End".into(),
            Key::PageUp => "PageUp".into(),
            Key::PageDown => "PageDown".into(),
            Key::Insert => "Insert".into(),
        }
    }

    /// The character of a key that types, or `None` for a named key.
    #[must_use]
    pub fn character(self) -> Option<char> {
        match self {
            Key::Character(c) => Some(c),
            _ => None,
        }
    }

    /// The value of a digit key (`'0'` through `'9'`), or `None` for every other key.
    #[must_use]
    pub fn digit(self) -> Option<u8> {
        match self {
            Key::Character(c @ '0'..='9') => Some(c as u8 - b'0'),
            _ => None,
        }
    }

    /// Whether this is one of the four arrow keys.
    #[must_use]
    pub fn is_arrow(self) -> bool {
        matches!(
            self,
            Key::ArrowLeft | Key::ArrowRight | Key::ArrowUp | Key::ArrowDown
        )
    }

    /// The name a menu [`Shortcut`] spells this key with (`"Return"`, `"Left"`, `"F1"`,
    /// `"s"`): the menu vocabulary every backend's accelerator table reads, which differs
    /// from the web names for Enter and the arrows.
    #[must_use]
    pub fn shortcut_name(self) -> String {
        match self {
            Key::Enter => "Return".into(),
            Key::ArrowLeft => "Left".into(),
            Key::ArrowRight => "Right".into(),
            Key::ArrowUp => "Up".into(),
            Key::ArrowDown => "Down".into(),
            other => other.name(),
        }
    }
}

impl std::fmt::Display for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name())
    }
}

impl From<char> for Key {
    fn from(c: char) -> Key {
        Key::Character(c)
    }
}

/// A key by its web name ([`Key::from_name`]): what a backend's key table and a scripted
/// step hold.
impl From<&str> for Key {
    fn from(name: &str) -> Key {
        Key::from_name(name)
    }
}

impl From<String> for Key {
    fn from(name: String) -> Key {
        Key::from_name(&name)
    }
}

impl From<&String> for Key {
    fn from(name: &String) -> Key {
        Key::from_name(name)
    }
}

impl From<std::borrow::Cow<'_, str>> for Key {
    fn from(name: std::borrow::Cow<'_, str>) -> Key {
        Key::from_name(&name)
    }
}

/// Where a key is in its press (docs/menus.md "Keys").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum KeyPhase {
    /// The key went down.
    #[default]
    Down,
    /// The key is held and the platform repeated it. A handler that acts once per press
    /// ignores this phase; one that moves while a key is held treats it like `Down`.
    Repeat,
    /// The key came up. Reported where [`Cap::KeyPhases`] is supported.
    Up,
}

impl KeyPhase {
    /// The phase a backend's numeric wire carries beside the modifier mask
    /// ([`bridge::BridgeKind::Key`]): bits 8 and 9 of `num` (0 down, 1 repeat, 2 up).
    #[must_use]
    pub fn from_wire(num: f64) -> KeyPhase {
        match (num as u32 >> 8) & 3 {
            1 => KeyPhase::Repeat,
            2 => KeyPhase::Up,
            _ => KeyPhase::Down,
        }
    }
}

/// One key event for the focused piece (`Decorate::on_key`, `Decorate::on_key_up`).
///
/// Build one with [`KeyEvent::new`], [`KeyEvent::named`], [`KeyEvent::typed_text`] or
/// [`KeyEvent::typed_by`], then [`KeyEvent::with_phase`] for a repeat or a release.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct KeyEvent {
    /// The key. On the short route (a focusable piece or a canvas) the four arrows and the ten
    /// digits (from the main row or the numeric keypad) arrive everywhere, plus
    /// [`Key::Delete`] and [`Key::Backspace`] on backends with no menu bar ([`Cap::AppMenu`]
    /// unsupported), where no accelerator can own those keys. A focused piece claims every key
    /// the route carries, so offering them on a menu-bar platform would let a canvas swallow
    /// the Delete its own Edit menu was about to act on (docs/menus.md). The two delete keys
    /// keep the names the platform gives the physical keys (a Mac's ⌫ is `Backspace` and its ⌦
    /// is `Delete`), so a handler that means "remove this" takes both. A digit arrives only
    /// while neither Primary nor Alt (nor a Mac's Control) is held: ⌘1 and Ctrl+1 are
    /// accelerators, and a canvas claiming them would starve the menu. Most backends name the
    /// digit the keypress types, so a layout whose number row needs Shift for digits reports
    /// `'1'` with Shift set; XAML and ArkUI read the physical key. Backends emit `Event::Key`
    /// only while no text widget has focus; a field's own editing keys never surface here.
    /// The one exception is a single-line text field with a key handler ([`keys::handled`] for
    /// its node): it hands [`Key::ArrowUp`], [`Key::ArrowDown`] and [`Key::Escape`], pressed
    /// with no modifier, to that handler instead of acting on them itself, so an address bar
    /// can walk a suggestion list (docs/textfield.md "Keys").
    ///
    /// A piece that declared `Decorate::keyboard()` takes the full route instead: every key
    /// the platform delivers while it is focused. A key that types nothing is a named variant
    /// ([`KeyEvent::NAMED`] lists their web names); a key that types is [`Key::Character`]
    /// with its unmodified character (`'a'`, `'A'` under Shift, `'8'`, `'ü'` on a layout that
    /// has one), and what it typed rides in [`KeyEvent::text`]. Under Control or Alt the key
    /// stays that character (`'c'` with Control set, never the control code it would type).
    /// Text an input method commits (a dead-key accent, a Japanese phrase, the soft
    /// keyboard's word) arrives with the text alone and [`Key::Unidentified`]: no single key
    /// is behind it. Only the platform's command chords stay out: ⌘ on Apple platforms and
    /// the browser's own shortcuts belong to the menu and the browser, so a terminal's ⌘C
    /// still copies. [`KeyEvent::typed`] is the question a text consumer asks.
    pub key: Key,
    /// The modifiers held with the key.
    pub modifiers: Modifiers,
    /// The text this press committed, on the full route: what the key typed ("a", "A", "é"
    /// after a dead key, the whole string an input method committed at once), or `None` for
    /// a key that typed nothing (a named key, a Control or Alt chord, a composition still in
    /// progress). Separate from [`KeyEvent::key`] so that a committed "Enter" is text and the
    /// Enter key is a key, whatever the input method sends. Always `None` on the short route
    /// and on a release.
    pub text: Option<String>,
    /// Whether the key went down, repeated while held, or came up. `Decorate::on_key`
    /// receives `Down` and `Repeat`; `Decorate::on_key_up` receives `Up`.
    pub phase: KeyPhase,
}

impl KeyEvent {
    /// The ten digit key names, indexed by value: a backend's key-code table.
    pub const DIGITS: [&'static str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];

    /// The web `KeyboardEvent.key` name of every key that types nothing, as a backend's wire
    /// and a dayscript `key:` step spell them. A name outside this list is the text it types.
    pub const NAMED: [&'static str; 26] = [
        "ArrowLeft",
        "ArrowRight",
        "ArrowUp",
        "ArrowDown",
        "Enter",
        "Tab",
        "Escape",
        "Backspace",
        "Delete",
        "Home",
        "End",
        "PageUp",
        "PageDown",
        "Insert",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
    ];

    /// A press of `key` with `modifiers` held that typed nothing.
    #[must_use]
    pub fn new(key: Key, modifiers: Modifiers) -> KeyEvent {
        KeyEvent {
            key,
            modifiers,
            text: None,
            phase: KeyPhase::Down,
        }
    }

    /// This event in another phase: `KeyEvent::new(Key::ArrowLeft, Modifiers::NONE)`
    /// `.with_phase(KeyPhase::Up)` is the release. A release carries no text.
    #[must_use]
    pub fn with_phase(mut self, phase: KeyPhase) -> KeyEvent {
        self.phase = phase;
        if phase == KeyPhase::Up {
            self.text = None;
        }
        self
    }

    /// The text this press typed ([`KeyEvent::text`]), or `None` for a key that typed nothing.
    #[must_use]
    pub fn typed(&self) -> Option<&str> {
        self.text.as_deref()
    }

    /// A key that types nothing (a named key, or a chord's character under Control or Alt),
    /// on either route. A backend passes the key's web name and its wire modifier mask; both
    /// convert ([`Key::from_name`], [`Modifiers::from_bits`]).
    pub fn named(key: impl Into<Key>, modifiers: impl Into<Modifiers>) -> KeyEvent {
        KeyEvent::new(key.into(), modifiers.into())
    }

    /// Text a press committed on the full route. The key is the text's character when the
    /// text is one character (the press typed it), and [`Key::Unidentified`] for a longer
    /// commit, which no single key is behind.
    pub fn typed_text(text: impl Into<String>, modifiers: impl Into<Modifiers>) -> KeyEvent {
        let text = text.into();
        let mut chars = text.chars();
        let key = match (chars.next(), chars.next()) {
            (Some(c), None) => Key::Character(c),
            _ => Key::Unidentified,
        };
        KeyEvent {
            key,
            modifiers: modifiers.into(),
            text: Some(text),
            phase: KeyPhase::Down,
        }
    }

    /// Text a press committed on the full route, with the key that produced it when the
    /// backend knows it.
    pub fn typed_by(
        key: impl Into<Key>,
        text: impl Into<String>,
        modifiers: impl Into<Modifiers>,
    ) -> KeyEvent {
        KeyEvent {
            key: key.into(),
            modifiers: modifiers.into(),
            text: Some(text.into()),
            phase: KeyPhase::Down,
        }
    }

    /// The web name for the function key `F<n>`, `1..=12`.
    #[must_use]
    pub fn function_name(n: u8) -> Option<&'static str> {
        match n {
            1..=12 => Some(Self::NAMED[13 + n as usize]),
            _ => None,
        }
    }

    /// The key name for a typed character when it is one of the ten digits: what a backend
    /// reports for a keypress that types `0`–`9`.
    #[must_use]
    pub fn digit_name(c: char) -> Option<&'static str> {
        c.to_digit(10).map(|d| Self::DIGITS[d as usize])
    }

    /// The value of a digit key, or `None` for every other key.
    #[must_use]
    pub fn digit(&self) -> Option<u8> {
        self.key.digit()
    }

    /// Whether Shift is held.
    #[must_use]
    pub fn shift(&self) -> bool {
        self.modifiers.shift()
    }
    /// Whether the platform's command key is held ([`Modifiers::PRIMARY`]).
    #[must_use]
    pub fn primary(&self) -> bool {
        self.modifiers.primary()
    }
    /// Whether Alt (Option) is held.
    #[must_use]
    pub fn alt(&self) -> bool {
        self.modifiers.alt()
    }
    /// Whether the Control key itself is held ([`Modifiers::CONTROL`]).
    #[must_use]
    pub fn control(&self) -> bool {
        self.modifiers.control()
    }
    /// Whether the key went down or repeated: what `Decorate::on_key` delivers.
    #[must_use]
    pub fn is_press(&self) -> bool {
        self.phase != KeyPhase::Up
    }
    /// Whether this is the platform repeating a held key.
    #[must_use]
    pub fn is_repeat(&self) -> bool {
        self.phase == KeyPhase::Repeat
    }
    /// Whether the key came up.
    #[must_use]
    pub fn is_release(&self) -> bool {
        self.phase == KeyPhase::Up
    }
}

#[cfg(test)]
mod key_event_tests {
    use super::{Key, KeyEvent, KeyPhase, Modifiers};

    #[test]
    fn digit_names_read_back_as_values() {
        for (value, c) in ('0'..='9').enumerate() {
            let name = KeyEvent::digit_name(c).expect("an ASCII digit names itself");
            let key = KeyEvent::named(name, 0);
            assert_eq!(key.digit(), Some(value as u8));
        }
        // Only the ASCII digits: a letter, an Arabic-Indic three, and a named key are not.
        assert_eq!(KeyEvent::digit_name('a'), None);
        assert_eq!(KeyEvent::digit_name('٣'), None);
        let arrow = KeyEvent::named("ArrowUp", 0);
        assert_eq!(arrow.digit(), None);
        assert_eq!(arrow.key, Key::ArrowUp);
    }

    #[test]
    fn typed_text_is_told_from_named_keys() {
        let a = KeyEvent::typed_text("a", 0);
        assert_eq!((a.key, a.typed()), (Key::Character('a'), Some("a")));
        let space = KeyEvent::typed_text(" ", 0);
        assert_eq!((space.key, space.typed()), (Key::Character(' '), Some(" ")));
        assert_eq!(KeyEvent::typed_text("7", 0).digit(), Some(7));
        // A longer commit has no key behind it.
        let phrase = KeyEvent::typed_text("こんにちは", 0);
        assert_eq!(
            (phrase.key, phrase.typed()),
            (Key::Unidentified, Some("こんにちは"))
        );
        // Text that spells a key name is still text, and the key is still a key.
        for name in KeyEvent::NAMED {
            let key = KeyEvent::named(name, 0);
            assert_eq!(key.typed(), None, "{name} types nothing");
            assert_ne!(key.key, Key::Unidentified, "{name} is in the vocabulary");
            assert_eq!(key.key.name(), name, "{name} round-trips");
            assert_eq!(KeyEvent::typed_text(name, 0).typed(), Some(name));
        }
        let accent = KeyEvent::typed_by("e", "é", 0);
        assert_eq!(
            (accent.key, accent.typed()),
            (Key::Character('e'), Some("é"))
        );
        assert_eq!(KeyEvent::function_name(1), Some("F1"));
        assert_eq!(KeyEvent::function_name(12), Some("F12"));
        assert_eq!(KeyEvent::function_name(13), None);
        let chord = KeyEvent::named("c", Modifiers::CONTROL | Modifiers::SHIFT);
        assert!(chord.control() && chord.shift() && !chord.primary());
        assert_eq!(chord.typed(), None);
    }

    #[test]
    fn names_modifiers_and_phases_convert_at_the_wire() {
        assert_eq!(Key::from_name("Space"), Key::Character(' '));
        assert_eq!(Key::from_name("F12"), Key::F(12));
        assert_eq!(Key::from_name("F13"), Key::Unidentified);
        assert_eq!(Key::from_name(""), Key::Unidentified);
        assert_eq!(Key::Enter.shortcut_name(), "Return");
        assert_eq!(Key::ArrowLeft.shortcut_name(), "Left");
        assert_eq!(Key::Character('s').shortcut_name(), "s");
        let mask = Modifiers::SHIFT.bits() | Modifiers::ALT.bits();
        let mods = Modifiers::from_bits(mask);
        assert!(mods.shift() && mods.alt() && !mods.primary() && !mods.control());
        assert!(mods.contains(Modifiers::SHIFT) && !mods.contains(Modifiers::PRIMARY));
        assert!(Modifiers::NONE.is_empty() && !mods.is_empty());
        // The phase rides above the mask on the numeric wire.
        assert_eq!(KeyPhase::from_wire(f64::from(mask)), KeyPhase::Down);
        assert_eq!(
            KeyPhase::from_wire(f64::from(u16::from(mask) | 1 << 8)),
            KeyPhase::Repeat
        );
        assert_eq!(
            KeyPhase::from_wire(f64::from(u16::from(mask) | 2 << 8)),
            KeyPhase::Up
        );
        let up = KeyEvent::typed_text("a", 0).with_phase(KeyPhase::Up);
        assert!(up.is_release() && !up.is_press() && up.typed().is_none());
        assert!(
            KeyEvent::new(Key::Tab, Modifiers::NONE)
                .with_phase(KeyPhase::Repeat)
                .is_repeat()
        );
    }

    #[test]
    fn a_declaration_claims_nothing_without_a_handler() {
        let node = super::NodeId(0xdead_0001);
        super::keys::mark_text(node);
        assert!(!super::keys::text(node), "no handler, no claim");
        super::keys::mark(node);
        assert!(super::keys::text(node) && super::keys::handled(node));
        super::keys::forget(node);
        assert!(!super::keys::text(node) && !super::keys::handled(node));
    }
}

/// The kind of device behind a pointer (docs/gestures.md "Input kind"): what
/// `day::primary_pointer()` tracks and a [`Touch`] carries.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum PointerKind {
    /// A finger on a touch screen.
    Touch,
    /// A mouse or a trackpad: a pointer that hovers.
    #[default]
    Mouse,
    /// A stylus or pencil.
    Stylus,
}

impl PointerKind {
    /// The kind a platform's users reach for first, before any input arrives: touch on a
    /// phone or tablet OS, a mouse elsewhere. [`Toolkit::primary_pointer`] starts from it.
    #[must_use]
    pub fn platform_default() -> PointerKind {
        if cfg!(any(
            target_os = "ios",
            target_os = "android",
            target_env = "ohos"
        )) {
            PointerKind::Touch
        } else {
            PointerKind::Mouse
        }
    }

    /// Whether this pointer hovers (a mouse, a trackpad, or a stylus that reports proximity).
    #[must_use]
    pub fn hovers(self) -> bool {
        self != PointerKind::Touch
    }

    /// The name a backend's wire and a dayscript step spell this kind with.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            PointerKind::Touch => "touch",
            PointerKind::Mouse => "mouse",
            PointerKind::Stylus => "stylus",
        }
    }

    /// The kind `name` spells ([`PointerKind::name`]), or `None`.
    #[must_use]
    pub fn from_name(name: &str) -> Option<PointerKind> {
        match name {
            "touch" => Some(PointerKind::Touch),
            "mouse" => Some(PointerKind::Mouse),
            "stylus" => Some(PointerKind::Stylus),
            _ => None,
        }
    }
}

/// Where one contact of a multi-touch sequence is ([`Touch::phase`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TouchPhase {
    /// The contact went down.
    Began,
    /// The contact moved.
    Moved,
    /// The contact is down and did not move in this event.
    Stationary,
    /// The contact lifted.
    Ended,
    /// The platform took the contact away (a system gesture, an enclosing scroll view).
    Cancelled,
}

/// One contact of a raw multi-touch event (`Decorate::on_touches`, docs/gestures.md "Touches").
///
/// Build one with [`Touch::new`], then [`Touch::with_pressure`] and [`Touch::with_kind`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Touch {
    /// Stable for the life of the contact, from `Began` to `Ended` or `Cancelled`; reused
    /// afterwards.
    pub id: u64,
    pub phase: TouchPhase,
    /// In the piece's own coordinates, in points.
    pub location: Point,
    /// Contact force from 0 to 1; 1 where the device reports none.
    pub pressure: f64,
    /// The device behind the contact.
    pub kind: PointerKind,
}

impl Touch {
    /// A finger contact at full pressure.
    #[must_use]
    pub fn new(id: u64, phase: TouchPhase, location: Point) -> Touch {
        Touch {
            id,
            phase,
            location,
            pressure: 1.0,
            kind: PointerKind::Touch,
        }
    }

    /// This contact with a reported force (clamped to 0 through 1).
    #[must_use]
    pub fn with_pressure(mut self, pressure: f64) -> Touch {
        self.pressure = pressure.clamp(0.0, 1.0);
        self
    }

    /// This contact from another kind of device.
    #[must_use]
    pub fn with_kind(mut self, kind: PointerKind) -> Touch {
        self.kind = kind;
        self
    }

    /// Whether the contact is on the surface after this event.
    #[must_use]
    pub fn is_down(&self) -> bool {
        !matches!(self.phase, TouchPhase::Ended | TouchPhase::Cancelled)
    }
}

/// Where user-driven scrolling stands ([`Event::ScrollPhase`], docs/scroll.md "Offset and
/// phase").
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ScrollPhase {
    /// Nothing the user did is moving the content.
    #[default]
    Idle,
    /// The user is moving the content: a finger is down and dragging, or a wheel or trackpad
    /// gesture is in progress.
    Dragging,
    /// The user let go and the content is still moving: momentum, a bounce settling, or a snap
    /// to a page. A toolkit that cannot tell this from [`ScrollPhase::Dragging`] never sends it.
    Coasting,
}

/// What a recycling list shows, as its host reports it through [`ListSource::viewport`]
/// (docs/list.md "Visible rows"); day-core passes it on as [`Event::ListViewport`].
///
/// The host counts the rows [`ListSource::len`] counts (headers and supplementary rows
/// included); day-core translates to the app's items before the piece sees the report.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ListViewport {
    /// The first row with any part inside the viewport, overscan excluded.
    pub first: usize,
    /// The last row with any part inside the viewport, overscan excluded. Never less than
    /// `first`.
    pub last: usize,
    /// How far the viewport's end is from the end of the content, in points: `0.0` when the
    /// list is scrolled all the way to its end or its rows fit the viewport, never negative
    /// (an overscroll bounce reports `0.0`). A host that estimates the heights of rows it has
    /// not laid out reports the estimate.
    pub end_distance: f64,
}

/// Where a row lands in the viewport when a list scrolls to it ([`ListScroll::align`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum RowAlign {
    /// The least movement that makes the whole row visible; a row already in view stays put.
    #[default]
    Nearest,
    /// The row's top edge at the top of the viewport.
    Start,
    /// The row centered in the viewport.
    Center,
    /// The row's bottom edge at the bottom of the viewport.
    End,
}

/// One programmatic scroll of a recycling list ([`Toolkit::list_scroll`], docs/list.md
/// "Programmatic scrolling").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ListScroll {
    /// The row to bring into view, counted as [`ListSource::len`] counts rows and already
    /// clamped to it; `None` scrolls to the very end of the content.
    pub row: Option<usize>,
    /// Where the row lands. Ignored for `row: None`.
    pub align: RowAlign,
    /// Scroll with the platform's standard scroll animation, or jump.
    pub animated: bool,
}

/// The event sink: enqueue-only. It may be invoked re-entrantly from inside any Toolkit method;
/// day-core drains queued events at safe points, each as a fresh batch (§3.3).
pub type EventSink = Box<dyn Fn(NodeId, Event)>;

/// The synchronous row-pull closures for recycling lists (docs/list.md, §10). day-core injects one
/// per `LIST` host via [`Toolkit::attach_list`]; a recycling backend stores it and calls it from
/// its native data-source (on the UI thread, outside any day-core borrow). Each closure re-enters
/// day-core, so, unlike [`EventSink`], these run to completion synchronously (`bind_row` even
/// flushes + lays out the row before returning, so the host can measure the cell immediately).
#[derive(Clone)]
pub struct ListSource {
    /// Report the first actually visible row (exclude overscan). Enqueue-only; optional,
    /// so hosts without a viewport binding incur no app event work.
    pub first_visible: Option<std::rc::Rc<dyn Fn(usize)>>,
    /// Report what the list shows (docs/list.md "Visible rows"): the first and last visible
    /// rows and the distance from the viewport's end to the content's end. Present when the
    /// piece observes the viewport (`visible_rows`, `follow_end`, `on_end_reached`); a host
    /// calls it whenever any of the three values may have changed: after a scroll (the
    /// user's or a programmatic one), after the list is resized, and after a reload or a
    /// splice has been laid out. Enqueue-only and deduplicated by day-core, so a host may
    /// call it on every scroll callback. It supersedes `first_visible`: a host that calls
    /// this one need not call the other.
    pub viewport: Option<std::rc::Rc<dyn Fn(ListViewport)>>,
    /// Report that the reader started moving the list themselves (docs/list.md "Following
    /// the end"): a drag, a wheel or trackpad scroll, the scroll bar, or a key that scrolls.
    /// Never a scroll the reader did not start: [`Toolkit::list_scroll`], a reload or splice,
    /// a resize, or the momentum of a scroll the app asked for. Present when the piece follows
    /// the end (`follow_end`); a host that answers [`Cap::ListReaderScroll`] calls it as each
    /// such scroll begins (more calls during one are harmless). Enqueue-only.
    pub reader_scroll: Option<std::rc::Rc<dyn Fn()>>,
    /// Present when the list has rows that are neither items nor section headers
    /// (docs/list.md "Header, footer and empty state"): whether row `index` is such a
    /// supplementary row (the list's header, footer, empty state, or content padding). The
    /// host sizes it to its content even in a `Uniform` list (`cell_height` answers it),
    /// draws no separator, selection or swipe affordance on it, never drags it, never pins
    /// it, and gives it no group-header styling: it is plain content that scrolls with the
    /// rows. `is_header` answers `false` for it.
    pub is_supplement: Option<std::rc::Rc<dyn Fn(usize) -> bool>>,
    /// Current row count.
    pub len: std::rc::Rc<dyn Fn() -> usize>,
    /// Present on a sectioned list (`ListProps::sections`, docs/list.md "Sections"): whether
    /// row `index` is a section header. Every row index the host sees counts headers, and
    /// day-core translates to the app's items. A header row is not selectable, activatable or
    /// swipeable; the host draws it in its platform's group-header style, sized to its content
    /// even in a `Uniform` list, and floats it where the platform does (macOS group rows,
    /// UIKit plain-table section headers).
    pub is_header: Option<std::rc::Rc<dyn Fn(usize) -> bool>>,
    /// Stable identity token for the row at `index` (for native diffing / animation).
    pub token_at: std::rc::Rc<dyn Fn(usize) -> u64>,
    /// Build (first use of this cell) or rebind (recycled cell) row `index` into the native cell.
    pub bind_row: std::rc::Rc<dyn Fn(usize, RawHandle)>,
    /// A host that emulates `Cap::ListSections` binds the header it pins over the top of the
    /// list with this instead of `bind_row` (docs/list.md "Sections"): the same build or
    /// rebind, but the copy carries no element ids, so a dayscript or accessibility lookup of a
    /// header finds the header row in the list, never its pinned copy.
    pub bind_pinned: std::rc::Rc<dyn Fn(usize, RawHandle)>,
    /// The native cell left the viewport; Day may drop per-cell bookkeeping (optional).
    pub recycle: std::rc::Rc<dyn Fn(RawHandle)>,
    /// The native host is done with this cell for good (a `GtkListItemFactory::teardown`, a
    /// `UITableViewCell` deallocating): Day disposes the row it built there, its scope, nodes
    /// and element ids. Without this a host that discards cells, as every pool does on a far
    /// jump, leaves one dead row behind in day-core per discarded cell. Safe inside a day-core
    /// borrow (the work is held for the borrow's end) and for a cell Day never saw.
    pub release: std::rc::Rc<dyn Fn(RawHandle)>,
    /// Re-lay the row bound to this cell at `width`, called from the cell's own native layout
    /// pass, the hook [`TreeSource::layout_cell`] already gives trees. day-core lays a row at the
    /// `LIST`'s width first, which is right wherever the cell is the row itself; a backend
    /// whose native row wraps the cell in chrome (`GtkListView`'s themed `row` node pads it)
    /// corrects it here, so the row fits the cell the platform gave it instead of holding the
    /// whole list wider than its frame. Skips quietly when called inside a day-core borrow (a
    /// snapshot pass).
    pub layout_cell: std::rc::Rc<dyn Fn(RawHandle, f64)>,
    /// The height day-core last laid the row bound to this cell at: the row's own height,
    /// padding included, as `RowHeight` resolves it. A backend whose cell measures itself
    /// (`GtkListView`'s `DayCell`) reports it as the cell's height, so a padded row keeps its
    /// bottom padding rather than the height of its last child's frame. `0.0` before the row's
    /// first layout, or inside a day-core borrow.
    pub cell_height: std::rc::Rc<dyn Fn(RawHandle) -> f64>,
    /// Drag-to-reorder closures, present when `ListProps::reorderable` (docs/list.md). `None` on
    /// non-reorderable lists; backends must not enable their drag machinery without it.
    pub reorder: Option<ListReorder>,
    /// Swipe-to-delete closures, present when `ListProps::deletable` (docs/list.md). `None` on
    /// non-deletable lists; backends must not offer their delete affordance without it.
    pub delete: Option<ListDelete>,
    /// Swipe-actions closures, present when `ListProps::swipe_actions` (docs/list.md). `None` on
    /// lists that declared none; backends must not install their row-action machinery
    /// without it.
    pub swipe: Option<ListSwipe>,
}

/// The synchronous drag-to-reorder half of [`ListSource`] (docs/list.md). Both closures follow
/// `bind_row`'s discipline: called on the UI thread from inside native drag callbacks, outside
/// any day-core borrow, and they run to completion synchronously.
#[derive(Clone)]
pub struct ListReorder {
    /// May row `from` drop at row `to`? Called from the native validate/hover hook so the
    /// affordance (gap, insertion mark, forbidden cursor) reflects the app's answer live.
    /// Returns the accepted target index: `to` itself to allow, another index to retarget the
    /// drop, or `-1` to deny.
    pub can_move: std::rc::Rc<dyn Fn(usize, usize) -> i64>,
    /// Commit: row `from` dropped at row `to` (an index `can_move` accepted). Rotates Day's row
    /// snapshot before returning (so `len`/`token_at`/`bind_row` reflect the new order while the
    /// native move animates) and defers the app's own callback to the next event drain.
    pub move_row: std::rc::Rc<dyn Fn(usize, usize)>,
}

/// The synchronous swipe-to-delete half of [`ListSource`] (docs/list.md), shaped exactly like
/// [`ListReorder`]: both closures are called on the UI thread from inside native swipe callbacks,
/// outside any day-core borrow, and run to completion synchronously.
///
/// Each platform spells the gesture its own way (a trailing swipe action on iOS, an
/// `ItemTouchHelper` swipe on Android, a `ListItem` swipe action on ArkUI), but all three ask the
/// same two questions, so this struct is one pair of closures rather than three shapes.
#[derive(Clone)]
pub struct ListDelete {
    /// May row `index` be deleted? Called before the affordance is offered, so a row the app
    /// protects shows no delete action at all rather than one that fails on use.
    pub can_delete: std::rc::Rc<dyn Fn(usize) -> bool>,
    /// Commit: delete row `index`. Removes it from Day's row snapshot before returning (so
    /// `len`/`token_at`/`bind_row` already reflect the shorter list while the native row-removal
    /// animates) and defers the app's own callback to the next event drain.
    pub delete_row: std::rc::Rc<dyn Fn(usize)>,
}

/// Which edge of a row a swipe action rides (docs/list.md). Semantic, not geometric:
/// `Leading` follows the reading direction (the left edge in LTR, the right in RTL), the
/// mapping every platform's swipe API already makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeEdge {
    Leading,
    Trailing,
}

/// One offered swipe action (docs/list.md): pure data. The pieces layer keeps the handlers;
/// the backend answers a tap through [`ListSwipe::perform`] by index into the offer.
#[derive(Clone, Debug, PartialEq)]
pub struct ListSwipeAction {
    /// The word on the button, already localized (a toolkit cannot translate).
    pub label: String,
    /// Destructive styling: the platform's red.
    pub destructive: bool,
    /// Explicit button color; `None` = the platform's default for the style.
    pub tint: Option<Ink>,
    /// A glyph for the button where the platform draws one: above the label on macOS, in
    /// place of it on iOS (docs/list.md).
    pub symbol: Option<Symbol>,
}

/// The synchronous swipe-actions half of [`ListSource`] (docs/list.md), the generalized
/// sibling of [`ListDelete`]: reveal-as-you-swipe buttons on either row edge. The offer is
/// pulled per row at gesture time, so labels follow row state ("Mark Read" on an unread row,
/// "Mark Unread" on a read one); the commit defers to the event drain like every closure here.
#[derive(Clone)]
pub struct ListSwipe {
    /// The offer: the actions for row `index` on `edge`, called inside the platform's swipe
    /// callback, so keep it pure. Empty = no affordance for that row and edge. The first action
    /// is the full-swipe action where the platform activates one at the far edge.
    pub actions_at: std::rc::Rc<dyn Fn(usize, SwipeEdge) -> Vec<ListSwipeAction>>,
    /// The commit: the user activated action `action` (an index into the offer) on row
    /// `index`, `edge`. Unlike a delete this never reshapes the row snapshot; the app's
    /// handler is deferred to the next event drain, never run inside the native callback.
    pub perform: std::rc::Rc<dyn Fn(usize, SwipeEdge, usize)>,
}

/// The synchronous row-pull closures for hierarchical trees (docs/tree.md): [`ListSource`]'s
/// shape, addressed by token instead of row index, because expanding a row renumbers every
/// index below it while tokens hold still. day-core injects one per `TREE` host via
/// [`Toolkit::attach_tree`]; the backend answers its native data-source from it. The same
/// synchronous discipline as [`ListSource`]: UI thread, outside any day-core borrow;
/// `bind_row` flushes and lays the row out before returning.
#[derive(Clone)]
pub struct TreeSource {
    /// How many children `parent` has (`None` = the root level).
    pub children_len: std::rc::Rc<dyn Fn(Option<u64>) -> usize>,
    /// The i-th child of `parent`: the stable token every backend keys its rows by.
    pub child_token: std::rc::Rc<dyn Fn(Option<u64>, usize) -> u64>,
    /// Whether this token can hold children at all: what draws (or omits) the disclosure.
    pub expandable: std::rc::Rc<dyn Fn(u64) -> bool>,
    /// Non-selectable section heading, separate from a navigation destination.
    pub section_header: std::rc::Rc<dyn Fn(u64) -> bool>,
    /// Current desired expansion, including restoration before the first native reload.
    pub expanded: std::rc::Rc<dyn Fn(u64) -> bool>,
    /// Build (first use of this cell) or rebind (recycled cell) the row for `token` into the
    /// native cell.
    pub bind_row: std::rc::Rc<dyn Fn(u64, RawHandle)>,
    /// The native cell left the viewport; Day may drop per-cell bookkeeping (optional).
    pub recycle: std::rc::Rc<dyn Fn(RawHandle)>,
    /// Re-lay the row bound to this cell at `width`, called from the cell's own native
    /// layout pass. Trees need this where lists don't: indentation makes every cell's
    /// content width per-row, so the host-width layout `bind_row` did is only a first
    /// approximation. Skips quietly when called inside a day-core borrow (a snapshot pass).
    pub layout_cell: std::rc::Rc<dyn Fn(RawHandle, f64)>,
    /// Native bounds for controls whose group headers have a different height from leaves.
    /// Overrides the initial row-height estimate and preserves the native height on rebind.
    pub layout_cell_size: std::rc::Rc<dyn Fn(RawHandle, Size)>,
    /// The height day-core last laid this cell's row at (see [`ListSource::cell_height`]).
    pub cell_height: std::rc::Rc<dyn Fn(RawHandle) -> f64>,
    /// The row's type-ahead string (docs/tree.md): what native type-select matches against.
    pub type_select_text: std::rc::Rc<dyn Fn(u64) -> String>,
    /// The row's context menu, built when the menu is summoned (docs/menus.md "Dynamic context
    /// menus"), called on the UI thread from the platform's own menu callback, outside any
    /// day-core borrow, like every closure here. `None` = rows carry no menu; an empty
    /// result suppresses the menu for that row.
    pub row_menu: Option<std::rc::Rc<dyn Fn(u64) -> Vec<MenuItem>>>,
    /// Drag-to-move closures, present when `TreeProps::movable` (docs/tree.md). `None` on
    /// immovable trees; backends must not enable their drag machinery without it.
    pub moves: Option<TreeMoves>,
}

/// The synchronous drag half of [`TreeSource`] (docs/tree.md). Both closures follow
/// `bind_row`'s discipline: called on the UI thread from inside native drag callbacks.
#[derive(Clone)]
pub struct TreeMoves {
    /// The live verdict, consulted while the drag is over a target: may `token` land under
    /// `parent` at `index`? `index: None` means "dropped onto the parent" (append). Pure, since
    /// it runs inside the platform's drag-validate callback.
    #[allow(clippy::type_complexity)]
    pub can_move: std::rc::Rc<dyn Fn(u64, Option<u64>, Option<usize>) -> MoveVerdict>,
    /// Commit an accepted drop. Defers the app's `on_move` through the event queue
    /// ([`Event::TreeMove`]); the app's own data write drives the reload.
    #[allow(clippy::type_complexity)]
    pub move_node: std::rc::Rc<dyn Fn(u64, Option<u64>, Option<usize>)>,
}

/// A summon-time context-menu provider (docs/menus.md): local-point in, menu out.
pub type ContextMenuFn = std::rc::Rc<dyn Fn(Point) -> Vec<MenuItem>>;

/// Ask a menu button's provider for its menu at press time ([`Toolkit::set_button_menu`]), and
/// answer the items to show, or `None` to show nothing: an empty menu, or a provider that
/// removed its own button. The provider is app code; a signal it writes can rebuild the tree
/// synchronously and release the very button being pressed. `still_registered` asks the
/// toolkit's own registry, AFTER the provider has returned, whether the button is still the one
/// registered for this press; a toolkit holding a raw native handle must not touch it otherwise.
pub fn resolve_button_menu(
    provider: &ContextMenuFn,
    still_registered: impl FnOnce() -> bool,
) -> Option<Vec<MenuItem>> {
    let items = provider(Point::ZERO);
    if items.is_empty() || !still_registered() {
        return None;
    }
    Some(items)
}

/// A tree move guard's verdict (docs/tree.md). No `Retarget` yet: the drop vocabulary is
/// already positional, so a guard that wants a different target denies and the user drops
/// where it is allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveVerdict {
    Allow,
    Deny,
}

/// One addressable widget within a composite native backing (docs/tree.md,
/// docs/tweaks.md), Qt's name for the concept (`QStyle::SubControl`). A `list`'s or
/// `tree`'s node handle is its scroller; `Subcontrol::Content` is the widget inside it.
/// Backends resolve an unknown subcontrol to `None`, never to the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subcontrol {
    /// The node's own handle: what every tweak accessor reached before subcontrols existed.
    Host,
    /// The content widget inside the scroller (`NSOutlineView`, `NSTableView`, …).
    Content,
    /// The header view, where the widget has one. Day's lists and trees ship headerless, so
    /// this answers `None` until a kind grows a header.
    Header,
}

// ---------------------------------------------------------------------------
// Capabilities, animation, a11y
// ---------------------------------------------------------------------------

/// What to show on the app's icon in the Dock, launcher, home screen, or taskbar (docs/badge.md).
///
/// Distinct from `NavItem::badge`, which annotates a sidebar row inside the window. This one
/// decorates the application itself and is drawn by the shell, not by Day.
///
/// What each payload needs is asked separately (`Cap::AppBadgeCount`, `AppBadgeText`,
/// `AppBadgeDot`) because the platforms differ in what they accept more than in whether they have
/// a badge at all: macOS takes arbitrary text, iOS and the web take a number, Android takes nothing.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum AppBadge {
    /// Clear the badge.
    #[default]
    None,
    /// A count. Zero clears it, matching every platform's own convention.
    Count(u32),
    /// Short arbitrary text (`"99+"`, `"beta"`). Only macOS renders it; probe `Cap::AppBadgeText`.
    Text(String),
    /// An indicator with no value.
    Dot,
}

/// How wide a window is, in coarse buckets (docs/size-classes.md).
///
/// The breakpoints are Android's window size classes, in density-independent points, and they are
/// used verbatim on every backend rather than per-platform. One table means one answer: a 700pt
/// window is [`WidthClass::Medium`] on a Mac, in a browser, and on a tablet, so an app that lays
/// out from the class gets the same layout at the same size everywhere. Apple publishes only two
/// buckets (compact/regular); those map onto this table rather than replacing it, with
/// [`WidthClass::Compact`] the compact one and everything above it regular.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum WidthClass {
    /// `< 600dp`: a phone in portrait. One pane at a time.
    #[default]
    Compact,
    /// `600–839dp`: a tablet in portrait, a narrow desktop window.
    Medium,
    /// `840–1199dp`: a tablet in landscape, a typical desktop window.
    Expanded,
    /// `1200–1599dp`: a large desktop window.
    Large,
    /// `≥ 1600dp`: a maximized window on a big display.
    ExtraLarge,
}

/// How tall a window is (docs/size-classes.md). Consulted far less than [`WidthClass`]: it is
/// what tells a phone in landscape (wide enough for two panes, too short for a tall list) that
/// it should not grow vertical chrome.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum HeightClass {
    /// `< 480dp`: a phone in landscape.
    #[default]
    Compact,
    /// `480–899dp`: a phone in portrait, a tablet in landscape.
    Medium,
    /// `≥ 900dp`: a tablet in portrait, most desktop windows.
    Expanded,
}

/// A window's size class (docs/size-classes.md), reported by the backend and read by apps
/// through `day::size_class()`.
///
/// Per-window, not per-app: two windows of one process can sit in different classes at the same
/// time (a narrow and a wide window side by side, iPadOS Stage Manager, Android split-screen), and
/// an app that keyed off a single global would lay the second window out for the first one's size.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SizeClass {
    pub width: WidthClass,
    pub height: HeightClass,
}

impl SizeClass {
    /// The `Compact`/`Medium` width boundary in dp: the narrowest window where a second
    /// pane starts to fit. Backends that hand an adaptive container its pane widths derive
    /// them from this (Android's list + minimum-detail widths sum to it), so the platform's
    /// measure-time decision and this table agree by construction (docs/size-classes.md).
    pub const SPLIT_MIN_WIDTH: f64 = 600.0;

    /// Bucket a window's size in points. The one place the breakpoint numbers appear.
    #[must_use]
    pub fn from_size(width: f64, height: f64) -> Self {
        SizeClass {
            width: match width {
                w if w >= 1600.0 => WidthClass::ExtraLarge,
                w if w >= 1200.0 => WidthClass::Large,
                w if w >= 840.0 => WidthClass::Expanded,
                w if w >= Self::SPLIT_MIN_WIDTH => WidthClass::Medium,
                _ => WidthClass::Compact,
            },
            height: match height {
                h if h >= 900.0 => HeightClass::Expanded,
                h if h >= 480.0 => HeightClass::Medium,
                _ => HeightClass::Compact,
            },
        }
    }

    /// Whether a two-pane presentation fits. The rule behind an automatic
    /// [`props::NavPresentation`]: anything wider than compact gets both panes.
    #[must_use]
    pub fn prefers_split(self) -> bool {
        self.width > WidthClass::Compact
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
    /// Standard searchfield control.
    SearchField,
    /// Standard combobox control.
    ComboBox,
    /// Standard stepper control.
    Stepper,
    /// Standard datetime control.
    DateTimePicker,
    /// Standard colorpicker control.
    ColorPicker,
    /// Standard pullrefresh control.
    PullRefresh,
    /// Standard pager control.
    Pager,
    /// User-driven pull recognition, separate from a refresh indicator.
    PullRefreshGesture,

    /// Native data-transfer source and destination callbacks.
    DragDrop,
    /// Transfers between independent native applications (inbound).
    DragExternalImport,
    /// Transfers to independent native applications, including arbitrary binary MIME data.
    DragExternalExport,
    /// Lossless ordered items and alternatives between Day applications.
    DragMultipleItems,
    /// Existing native file references, distinct from promised/generated files.
    DragFileReferences,
    /// Native result can wait for asynchronous data receipt.
    DragDeferredReceipt,
    /// Generated files supplied to a destination after dropping.
    DragFilePromises,
    /// External move acknowledgment safe for source deletion.
    DragExternalMove,
    ListRecycling,
    /// The toolkit fronts the app's undo stack with the platform's own undo system
    /// ([`Toolkit::set_undo_state`] + `Event::Undo`): `Native` where one exists
    /// (`NSUndoManager` on macOS/iOS, `QUndoStack`), `Unsupported` everywhere else, where
    /// Day's menu items and accelerators drive the stack directly and nothing platform-owned
    /// needs mirroring (docs/model.md).
    UndoBridge,
    /// The toolkit routes the platform's standard Cut/Copy/Paste to the app's edit bridge
    /// ([`Toolkit::set_edit_state`] + `Event::Edit`): the same menu items, shortcuts, and
    /// responder precedence the platform's text editing uses; a focused text widget keeps
    /// its own clipboard behavior, everything else reaches the app (docs/menus.md).
    /// `Native` where a system route exists (the responder chain, the browser's clipboard
    /// events), `Emulated` where Day's own menu items dispatch it, `Unsupported` elsewhere.
    EditBridge,
    /// The toolkit realizes `ListProps::reorderable` as drag-to-reorder rows: `Native` when the
    /// platform's own mechanism drives it (`NSTableView` drag/drop, `UITableView` drag delegates,
    /// `ItemTouchHelper`, …), `Emulated` for a pointer-tracked fake (web-dom). `Unsupported` ⇒ the
    /// list renders normally but rows cannot be dragged (docs/list.md).
    ListReorder,
    /// Sidebar destinations can be dragged to reorder (`NavBuilder::reorder_items`) and between
    /// sections (`NavBuilder::regroup_items`). `Native` when the toolkit's own sidebar drags them
    /// (`NavMenuProps::movable`, and across headers under `NavMenuProps::regroup`); `Emulated`
    /// when Day stands a reorderable list in for the sidebar. Independent of `ListReorder`: UIKit
    /// supports draggable content lists but retains its native navigation collection for
    /// sidebars.
    NavReorder,
    /// The toolkit realizes `ListProps::deletable` as the platform's delete gesture:
    /// `Native` where the platform ships one (UIKit's trailing swipe actions, Android's
    /// `ItemTouchHelper`, ArkUI's `ListItem.swipeAction`), `Unsupported` on the desktop toolkits,
    /// whose lists have no swipe idiom and where deletion belongs to a menu or a button
    /// (docs/list.md).
    ListDelete,
    /// The toolkit realizes `ListProps::swipe_actions` as edge swipe actions: buttons
    /// that reveal as the row tracks the swipe, with the far edge activating the first one
    /// (docs/list.md). `Native` where the platform ships the affordance: macos-appkit
    /// (`NSTableView` row actions, Mail's own machinery) and ios-uikit
    /// (`UISwipeActionsConfiguration`). `Unsupported` everywhere else: the affordance is
    /// absent, and the app's command lives in its explicit controls.
    ListSwipeActions,
    /// The toolkit realizes `ListProps::editing` as the platform's list editing mode: rows
    /// show their delete controls and reorder grips while the list is editing (docs/list.md).
    /// `Native` on ios-uikit (`UITableView` editing). `Unsupported` everywhere else, where a
    /// list has no separate editing mode: its rows already drag, swipe and open menus as they
    /// are, and the list renders as if `editing` were off.
    ListEditing,
    /// The toolkit realizes `kinds::LIST_ITEM` as its own list row content (docs/list.md "List
    /// items"): `Native` where the platform has a standard row it draws itself (UIKit's
    /// `UIListContentConfiguration`, a Material list item, AppKit's image-and-text table cell,
    /// a GTK row box), so the row takes the platform's fonts, icon size, spacing, secondary
    /// colors and check mark. `Unsupported` ⇒ the `list_item` piece composes the same content
    /// from plain pieces. Apps have no reason to probe this; the piece does.
    ListItem,
    /// The toolkit realizes `kinds::TREE` as a hierarchical tree view (docs/tree.md):
    /// `Native` where a platform tree widget hosts Day-built rows (`NSOutlineView`, the UIKit
    /// sidebar list, GtkListView+TreeListModel, WinUI `TreeView`, ArkTS `TreeView`), `Emulated`
    /// where Day flattens the tree onto its list machinery, `Unsupported` where neither
    /// exists yet. An app gates its tree UI on this answer rather than showing a placeholder.
    Tree,
    /// The toolkit realizes `TreeProps::movable` as drag-to-reparent/restack rows
    /// (docs/tree.md). A strictly smaller set than [`Cap::Tree`]: a backend can render a
    /// tree it cannot yet drag within.
    TreeMove,
    /// Native two-level section headers; general trees may still be composed.
    TreeSections,
    /// The toolkit answers [`Toolkit::first_baseline`], so rows can align text on its baseline
    /// rather than on the middle of its box (docs/baseline.md). `Native` where the platform
    /// reports the baseline itself (`NSView.firstBaselineOffsetFromTop`, `View.getBaseline`,
    /// `gtk_widget_measure`), `Emulated` where Day derives it from the widget's font metrics,
    /// `Unsupported` ⇒ baseline-aligned rows fall back to centering and look exactly as they do
    /// today.
    BaselineAlignment,
    /// The toolkit draws a label's [`TextRun`]s: bold, italic, color or a monospace face within
    /// one wrapping paragraph (docs/text-runs.md). `Unsupported` ⇒ the label renders its text
    /// uniformly, which reads correctly and loses only the emphasis.
    TextRuns,
    /// The toolkit makes a run carrying [`TextRun::link`] activatable, emitting
    /// [`Event::LinkActivated`]. A strictly smaller set than [`Cap::TextRuns`]: several toolkits
    /// draw a link run in link colors but have no way to hit-test it, and one (Android) can do
    /// links or selection but not both (docs/text-runs.md).
    TextLinks,
    Lottie,
    NativeSymbols,
    /// The toolkit can rasterize its window and hand back the pixels
    /// (`Toolkit::snapshot_window`): what the dayscript `screenshot` step captures in-process
    /// and what `day::window_image()` offers the app (docs/window-image.md). `Unsupported` where
    /// a backend cannot draw itself into a bitmap: web-dom, which would need a rasterizer
    /// shipped with it. Probe it before offering a "save a screenshot" affordance, rather than
    /// offering one that fails when pressed.
    Snapshot,
    /// The toolkit can present `nav()` as sidebar+detail split panes, a statement about the
    /// toolkit, not about the window it is currently drawing. Whether a given host is split right
    /// now follows from its [`SizeClass`]: the pieces layer resolves an automatic
    /// [`props::NavPresentation`] against both, and re-resolves on every class change. A backend
    /// with no split container answers `Unsupported` and stays stacked at every size.
    NavSplit,
    /// The toolkit can draw a navigation host's rows as its own chrome: a tab bar
    /// ([`props::NavPresentation::Tabs`]) and, where it has one, a rail
    /// ([`props::NavPresentation::Rail`]). Like [`Self::NavSplit`] this is a statement about the
    /// toolkit, not about the window it is drawing right now.
    ///
    /// This is what `NavStyle::Automatic` resolves against (docs/navigation.md): a backend
    /// answering `Unsupported` gets the sidebar resolver instead (`Split` ↔ `Stack`), which is
    /// what every backend did before adaptive navigation existed. So an unimplemented backend
    /// degrades to its previous behavior rather than to a hole.
    ///
    /// `Emulated` means the rows are composed from other widgets rather than drawn by a native
    /// tab container (Qt, web-dom). `Native` means a real one (`UITabBarController`, a Material
    /// `NavigationBarView`, `NavigationView`).
    NavTabs,
    /// A narrow window should become a tab bar here, a separate question from [`Self::NavTabs`],
    /// which only says the toolkit *can* draw one.
    ///
    /// The two differ on every desktop. macOS, GNOME, Qt and Windows can all draw a tab bar, and
    /// must, because an app is free to pin `NavStyle::Tabs`; but none of them grows one when
    /// its window is dragged narrow. A narrow Mail.app hides its sidebar and pushes; it does not
    /// sprout a bottom tab bar, and an app that did would look like a port. The phones and the web
    /// are the opposite: those are the surfaces whose window size ranges from a phone to a
    /// desktop, and a tab bar is what their users expect at the narrow end.
    ///
    /// So this is a statement about the platform's idiom, not about its widget set:
    ///
    /// - `Native`/`Emulated`: `NavStyle::Automatic` may resolve to
    ///   [`props::NavPresentation::Tabs`] on a compact window (ios-uikit, android-material,
    ///   harmony-arkui, web-dom).
    /// - `Unsupported`: it may not; a compact window collapses to
    ///   [`props::NavPresentation::Stack`] instead, exactly as `NavStyle::Sidebar` has always
    ///   done (macos-appkit, linux-gtk, linux-qt, windows-winui).
    ///
    /// [`props::NavPresentation::Rail`] is not gated by this. A narrow sidebar is an ordinary
    /// desktop shape (on Windows it is what `NavigationView` does at that width on its own), so
    /// the rail rung stays available everywhere `Cap::NavSplit` is.
    NavTabsAdaptive,
    /// How a navigation host's presentation follows the window (docs/size-classes.md). All three
    /// answers mean something different here, and the difference is who decides:
    ///
    /// - `Native`: **Day tells the toolkit.** The pieces layer resolves the presentation from
    ///   the [`SizeClass`], re-resolves on every change, and sends
    ///   [`props::NavPatch::Presentation`]; the toolkit rebuilds its chrome and re-homes the
    ///   pages it already has. The desktops and web-dom.
    /// - `Emulated`: **the toolkit tells Day.** Its own adaptive container owns the decision
    ///   (`UISplitViewController` collapsing as a Pro Max iPhone rotates, `SlidingPaneLayout`
    ///   measuring whether both panes fit), so Day must not push a presentation into it: that
    ///   would be a second decision racing the platform's own animation. The toolkit
    ///   reports what it did with [`Event::NavPresentationChanged`] and Day reconciles.
    /// - `Unsupported`: nobody re-presents. The presentation is resolved once, from
    ///   [`Cap::NavSplit`] alone, and the window's size never enters into it. A toolkit that
    ///   cannot change presentation must not have it decided by something that can, or a window
    ///   launched narrow would be stuck stacked with no way back.
    NavRepresent,
    /// The toolkit places a navigation host's content-list page ([`props::Pane::List`]) in its
    /// own pane between the sidebar and the detail, the Mail shape: mailboxes, message list,
    /// message (docs/navigation.md). The three answers differ on what happens when the host
    /// leaves the split:
    ///
    /// - `Native`: the pane exists at every presentation. A narrow window collapses the
    ///   sidebar but keeps the list beside the detail, exactly as a narrow Mail.app does
    ///   (macos-appkit: a `contentList` `NSSplitViewItem`).
    /// - `Emulated`: the pane exists while split and merges into the navigation stack when
    ///   the host collapses (ios-uikit: `UISplitViewController` triple-column). The pieces
    ///   layer then interposes the list between the sidebar root and the detail
    ///   ([`props::NavPatch::ListInStack`]) and gates the detail push on the app's
    ///   `detail_visible` binding.
    /// - `Unsupported`: the backend never sees [`props::Pane::List`] or
    ///   [`props::NavProps::list_width`]; the nav piece composes the pane beside (split)
    ///   or in place of (stacked) the destination content itself.
    NavContentList,
    /// The toolkit shows the current destination's title in a native header/bar, so a page
    /// needn't repeat it in its own content. `Native` on XAML (the `NavigationView` header),
    /// UIKit (`UINavigationBar`), Android (`MaterialToolbar`), and ArkUI (`NavDestination`
    /// title bars). The desktop custom back-headers (AppKit/Qt/web) show a title only in
    /// stack mode, so those backends stay `Unsupported` (docs/navigation.md).
    NavHeader,
    /// The toolkit applies a runtime appearance override (`set_appearance`): native widgets
    /// restyle in place and `dark_mode` answers the override. Probe before showing a theme
    /// picker; on `Unsupported` backends the call is ignored.
    Appearance,
    /// The toolkit renders an application menu bar (docs/menus.md), so `app_menu` is visible to
    /// the user, and `register_preferences` gets its automatic Settings… item.
    ///
    /// The question an app asks before deciding where a command lives. Settings is the usual
    /// case: on a platform with a menu bar it belongs in the App menu and putting it in the
    /// navigation as well is clutter; on one without, the menu is a no-op and a Settings row is
    /// the only way in. Distinct from `Cap::Toolbar`: web-dom draws a window toolbar and has no
    /// menu bar at all, so keying off the toolbar strands Settings there.
    AppMenu,
    /// The toolkit can present native alert/confirm/sheet/prompt modals (docs/dialogs.md).
    Dialogs,
    /// The toolkit can present native open/save file pickers (docs/files.md).
    FileDialogs,
    /// The toolkit can present a native folder chooser (`PresentSpec::OpenFolder`,
    /// docs/files.md). `Native` on the four desktop toolkits (AppKit, GTK, Qt, WinUI); the
    /// phone and web backends answer `Unsupported` and resolve the request as `Dismissed`, so
    /// an app hides its "Add Folder…" command where this is not `Native`.
    FolderDialog,
    /// The toolkit plays `AnimSpec` intents on `update`/`set_frame`/`set_opacity`/
    /// `set_transform` on its own animator, keeping the whole contract documented on
    /// [`AnimSpec`]: delay, repeat, autoreverse, physical springs, retargeting, and an animated
    /// container background (§8.4). `Unsupported` ⇒ animated calls apply instantly (still
    /// correct, just not animated).
    Animation,
    /// The toolkit presents a `kinds::COVER` node as a native fullscreen modal surface
    /// (docs/cover.md). `Unsupported` ⇒ the `cover` piece's content never shows.
    Cover,
    /// The toolkit presents a `.sheet()` cover as a native modal sheet (AppKit's window sheet,
    /// UIKit's page sheet) over the page it covers (docs/cover.md "Modal sheet"). `Unsupported`
    /// ⇒ the `cover` piece composes the sheet itself: on a window wider than compact, a card
    /// centered over the dimmed page; on a compact one, the full window.
    CoverSheet,
    /// The toolkit can hide the system status bar while a `status_bar_hidden` subtree is
    /// mounted (`Toolkit::set_status_bar_hidden`, docs/cover.md). `Native` on the phone and
    /// tablet backends; desktop and web have no status bar, so a setting that offers to hide
    /// it should be offered only where this is not `Unsupported`.
    StatusBarHidden,
    /// The toolkit's `text_area` can be made read-only (`TextAreaProps::editable = false`).
    TextEditable,
    /// The toolkit's `text_area` selectability can be toggled (`TextAreaProps::selectable`).
    /// `Unsupported` where selection is always on (GTK); `Emulated` where the backend
    /// collapses native selections itself (XAML, ArkUI).
    TextSelectable,
    /// The toolkit reports a selectable label's selection as it changes
    /// ([`Event::TextSelected`], `Label::on_selection`). `Unsupported` where the platform's
    /// label has no selection notification: the label is still selectable and the callback
    /// never runs.
    LabelSelection,
    /// The toolkit's `text_area` has built-in spell-check/autocorrect (`TextAreaProps::spellcheck`).
    /// `Unsupported` where the toolkit ships none (GTK, Qt, ArkUI).
    TextSpellCheck,
    /// The toolkit can open additional native OS windows (`Toolkit::open_window`,
    /// docs/windows.md). `Native` on the desktop backends and on mobile backends whose
    /// platform has a real secondary-window surface (iPad scenes, Android document
    /// activities, OHOS multiton abilities). On `Unsupported` backends
    /// `day_core::open_window` still works; the content presents as a fullscreen cover in
    /// the primary window instead. Probe it to adapt chrome: an `Unsupported` tier has no
    /// native title bar or close button, so window content should carry its own close
    /// affordance.
    MultiWindow,
    /// The toolkit can put a count on the app icon (`Toolkit::set_app_badge`, docs/badge.md).
    /// `Emulated` where the call is made but the shell may ignore it: desktop Linux, where the
    /// Unity launcher protocol is honored by Plasma and Dash-to-Dock but not by stock GNOME.
    AppBadgeCount,
    /// The toolkit renders `AppBadge::Text` as written. macOS only: `NSDockTile.badgeLabel` is an
    /// arbitrary string, while every other platform's badge is a number or nothing.
    AppBadgeText,
    /// The toolkit can show a valueless indicator (`AppBadge::Dot`).
    AppBadgeDot,
    /// The toolkit gives a window chrome that persists across pages
    /// (`Toolkit::edit_toolbar`, docs/toolbars.md): `Native` on the desktop backends and the
    /// phones, `Emulated` on web-dom, `Unsupported` where the only chrome belongs to a page.
    ///
    /// It no longer decides whether a command can be shown at all: a toolbar item declared on a
    /// piece rides that piece's own chrome everywhere. Probe it only for a layout decision that
    /// turns on a persistent bar existing.
    Toolbar,
    /// The window toolbar can hold a search field (docs/search.md): `Native` on the desktop
    /// backends, `Emulated` on web-dom. `Unsupported` on the phones, whose bar is a row of actions
    /// (an Android app-bar menu, iOS bar buttons, ArkUI title-bar menus): their field goes on the
    /// navigation surface instead, so `SearchPlacement::Toolbar` resolves to `Inline` there.
    ToolbarSearch,
    /// The toolkit realizes `kinds::INSPECTOR` as its own trailing-pane container
    /// (docs/inspector.md): an `NSSplitView` inspector pane, a `GtkPaned` with the panel as
    /// its end child, a `QDockWidget`, a XAML `SplitView` right pane. `Unsupported` ⇒ the
    /// `inspector` piece composes the pane from plain containers instead (same panel, same
    /// signal, no native divider), so an unimplemented backend degrades to a drawn pane, not
    /// to a hole. Apps normally have no reason to probe this; the piece does.
    Inspector,
    /// The toolkit realizes `kinds::SPLIT` as its own splitter (docs/split.md): an
    /// `NSSplitView`, a `GtkPaned`, a `QSplitter`, each side by side or stacked and with the
    /// divider the user drags. `Unsupported` ⇒ the `split` piece composes the two panes and
    /// draws and drags the divider itself (the web, the mobile toolkits, XAML), so the piece
    /// behaves the same everywhere and only the divider's look is the toolkit's. Apps have no
    /// reason to probe this; the piece does.
    Split,
    /// The toolkit shapes the pointer over a node from the `.cursor()` decorator
    /// ([`Toolkit::set_cursor`], docs/cursor.md). `Native` where the platform draws the requested
    /// shape from its own set (AppKit, GTK, the DOM, Android's `PointerIcon`), `Emulated` where
    /// some shapes map to a nearest neighbor (Qt, XAML) or the platform draws pointer effects
    /// rather than shapes (iPadOS), `Unsupported` where nothing changes: a touch-only toolkit,
    /// or one whose pointer API Day does not reach yet. Apps ask before offering an affordance
    /// that only a cursor reveals, such as a resize handle with no other visual.
    Cursor,
    /// The toolkit can enumerate the platform's font families with the faces each ships
    /// ([`Toolkit::font_families`], docs/fonts.md). `Native` where an OS font database answers
    /// (`NSFontManager`, `UIFont`, Pango, `QFontDatabase`, Android's `fonts.xml`,
    /// `OH_Drawing_FontMgr`, DirectWrite); `Emulated` where the list is composed rather than
    /// read: web-dom answers the CSS generic families plus the bundled `document.fonts`, since
    /// a browser exposes no local font list without a permission prompt; `Unsupported` means
    /// the list is empty and an app should offer only the default face. Measurement
    /// ([`Toolkit::measure_text`]) has no probe: a `None` answer degrades to
    /// [`TextMetrics::approximate`]. Probe this before offering a font menu, not before
    /// drawing; a [`CanvasFont`] draws everywhere.
    FontList,
    /// The toolkit reads the user's reduce-motion setting ([`Toolkit::reduce_motion`],
    /// docs/accessibility.md) and reports when it changes. `Native` where the platform has the
    /// switch and Day reaches it; `Unsupported` where Day reaches none (the app then animates as
    /// it would with the setting off, and `DAY_REDUCE_MOTION=1` still forces it on).
    ReduceMotion,
    /// The toolkit can speak a sentence through the screen reader without moving its focus
    /// ([`Toolkit::announce`], docs/accessibility.md): "Saved", "3 results", "Upload failed".
    /// `Native` where the platform has an announcement request (`NSAccessibility`'s
    /// announcement notification, `UIAccessibility`'s, Android's accessibility event, GTK
    /// 4.14's `gtk_accessible_announce`, a WinUI notification event); `Emulated` on the web,
    /// where an ARIA live region carries it; `Unsupported` where Day reaches no announcement
    /// API yet. An app with a visible status label needs no announcement: screen readers read
    /// what changes on screen only when told to, so this is for the change the user cannot
    /// otherwise learn about.
    Announce,
    /// The toolkit can turn encoded image bytes into a native image
    /// ([`Toolkit::decode_image`], docs/images.md): what `image(bytes)`, a pasted screenshot and
    /// a canvas image all rest on. `Native` wherever the platform decoder takes a buffer;
    /// `Unsupported` means only staged asset names draw, so an app should not offer "open an
    /// image" at all. Every backend that decodes also answers [`Toolkit::image_info`], so pixel
    /// size needs no probe of its own.
    ImageDecode,
    /// The toolkit can encode a decoded image back to bytes ([`Toolkit::encode_image`]).
    /// Which formats it encodes is [`Toolkit::encode_formats`], the same shape as
    /// [`Cap::FontList`]/[`Toolkit::font_families`], because "can encode" and "can encode WebP"
    /// are different questions and only the second one needs a list.
    ImageEncode,
    /// The toolkit can read an image's metadata beyond its pixels: EXIF orientation, DPI,
    /// capture time ([`Toolkit::image_properties`]). `Native` where a platform metadata reader
    /// is already linked (WinRT's `BitmapPropertiesView`, Android's `ExifInterface`);
    /// `Unsupported` where reading it would mean shipping a parser or a new framework, which is
    /// most places. An app that wants orientation must handle `None`.
    ImageProperties,
    /// The toolkit can replay a canvas drawing into an offscreen bitmap
    /// ([`Toolkit::render_bitmap`], docs/canvas.md "Exporting"): what `Drawing::to_bitmap` and
    /// `Drawing::to_png` rest on. `Native` where the toolkit's own canvas painter can target an
    /// image surface (a `CGBitmapContext`, a cairo image surface, a `QImage`, an android
    /// `Bitmap`, an `OH_Drawing_Bitmap`, a detached `<canvas>`); `Unsupported` where the
    /// on-screen canvas has no painter to retarget (WinUI builds XAML shapes instead).
    CanvasBitmap,
    /// The toolkit can replay canvas drawings into a PDF document ([`Toolkit::render_pdf`],
    /// docs/canvas.md "Exporting"), with vector shapes and the platform's own font embedding.
    /// `Native` only where the platform ships a PDF writer the canvas painter can draw into
    /// (Core Graphics on AppKit and UIKit, cairo on GTK, `QPdfWriter` on Qt, `PdfDocument` on
    /// Android); `Unsupported` on ArkUI, the web and WinUI, which have none.
    CanvasPdf,
    /// The toolkit minimizes, zooms, hides and restores a window on request
    /// ([`WindowChange::State`], [`WindowChange::Visible`], docs/windows.md) and reports what the
    /// user does through [`Event::WindowStateChanged`]. `Native` on the desktop backends;
    /// `Unsupported` on the phones and the web, whose windows the OS sizes.
    WindowStates,
    /// The toolkit takes a window fullscreen on request and reports when it enters and leaves
    /// ([`WindowState::Fullscreen`]). `Native` on the desktops, on Android (system bars hidden
    /// through `WindowInsetsController`), on HarmonyOS and on the web (the Fullscreen API, which
    /// browsers allow only during a user gesture); `Emulated` on iOS, which hides the status bar
    /// and lets the home indicator fade but always keeps the app full-screen anyway.
    WindowFullscreen,
    /// The toolkit keeps a window out of screenshots, recordings and screen sharing
    /// ([`WindowChange::ContentProtected`]). `Native` on macOS (`NSWindowSharingNone`), Windows
    /// (`WDA_EXCLUDEFROMCAPTURE`), Android (`FLAG_SECURE`) and HarmonyOS (privacy mode);
    /// `Emulated` on iOS, which has no switch for it: Day covers the content while the screen is
    /// being recorded or mirrored, but a screenshot still captures it. `Unsupported` on Linux
    /// and the web.
    ContentProtection,
    /// The toolkit honors [`WindowOptions::tabbing`]: macOS window tabs. `Unsupported` everywhere
    /// else, where no system window tabs exist.
    WindowTabbing,
    /// The toolkit shows a Dock menu ([`Toolkit::set_dock_menu`], docs/menus.md): the app's
    /// shortcuts and `dock_menu(..)` entries above the system's own items. macOS only.
    DockMenu,
    /// The toolkit resizes a window and bounds its size on request ([`WindowChange::Frame`]'s
    /// size, [`WindowChange::Limits`], [`WindowChange::Resizable`]).
    WindowGeometry,
    /// The toolkit moves a window to a requested position ([`WindowChange::Frame`]'s origin).
    /// `Unsupported` on GTK 4, which removed window positioning, and wherever the compositor owns
    /// placement (Wayland).
    WindowPosition,
    /// The toolkit changes how a window stacks ([`WindowChange::Level`],
    /// [`WindowChange::OnAllWorkspaces`], [`WindowChange::SkipTaskbar`]).
    WindowLevel,
    /// The toolkit gives one window its own light/dark appearance ([`WindowChange::Appearance`]).
    /// Where it cannot, the app-wide [`Toolkit::set_appearance`] still applies to every window.
    WindowAppearance,
    /// The toolkit asks for the user's attention on request ([`WindowChange::RequestAttention`]):
    /// a Dock bounce, a flashing taskbar button.
    RequestAttention,
    /// The toolkit opens a window with no title bar or frame ([`WindowChrome::Frameless`],
    /// docs/window-chrome.md). Desktop backends only.
    FramelessWindow,
    /// The toolkit runs a window's content under its title bar with the window controls kept
    /// ([`WindowChrome::Overlay`]) and reports the bar's height as the safe-area top inset.
    OverlayTitleBar,
    /// The toolkit opens a window whose unpainted pixels show the desktop
    /// ([`WindowBackground::Transparent`]).
    TransparentWindow,
    /// The toolkit puts a translucent system material behind a window
    /// ([`WindowBackground::Material`]): macOS's visual-effect materials, Windows 11's Mica and
    /// Acrylic. `Emulated` where an approximation stands in.
    WindowMaterial,
    /// The toolkit lets a piece move its window like a title bar does
    /// ([`Toolkit::set_drag_region`], `.window_drag_region()`). `Emulated` on the web, where
    /// only an installed app with a window-controls overlay honors it.
    DragRegion,
    /// The toolkit shows status items ([`Toolkit::set_status_items`], docs/status-item.md): the
    /// macOS menu bar, the Windows notification area, a Linux tray. On Linux the answer depends
    /// on the desktop: `Native` while a `StatusNotifierItem` host is running (KDE Plasma, most
    /// panels, GNOME with the AppIndicator extension), `Unsupported` without one (stock GNOME),
    /// so an app can keep a window instead of vanishing into a tray nobody shows.
    StatusItem,
    /// The toolkit shows progress on the app's Dock or taskbar icon
    /// ([`Toolkit::set_app_progress`]). `Emulated` on Linux, where the Unity launcher protocol is
    /// honored by KDE Plasma and Dash to Dock but not by every dock.
    AppProgress,
    /// The toolkit can hide the app's Dock icon while it runs ([`Toolkit::set_dock_visible`]).
    /// macOS only.
    DockVisibility,
    /// The toolkit lists the displays and their work areas ([`Toolkit::monitors`]). `Emulated`
    /// where the one screen the app runs on is reported (a phone, the web).
    Monitors,
    /// The toolkit shows launcher shortcuts the app sets while it runs
    /// ([`Toolkit::set_launcher_shortcuts`]).
    DynamicShortcuts,
    /// How the toolkit draws a sectioned list's headers (`ListProps::sections`, docs/list.md
    /// "Sections"). `Native` where the platform's list has group headers that pin to the top
    /// while their section scrolls under them: macos-appkit (`NSTableView` group rows with
    /// `floatsGroupRows`) and ios-uikit (plain-style `UITableView` section headers).
    /// `Emulated` where the host pins a copy of the current header over the top of the list
    /// itself. Headers are unselectable rows on every toolkit, so an app needs this only to
    /// describe the difference, never to decide whether to call `.sections`.
    ListSections,
    /// The toolkit's share chooser carries the app's own actions ([`share::ShareSpec::actions`],
    /// docs/sharing.md): Add Bookmark or Find on Page beside the system's targets, each with
    /// its title and glyph. `Native` on ios-uikit (a `UIActivity` per action), macos-appkit (an
    /// `NSSharingService` per action, offered by the picker's delegate) and android-material
    /// on Android 14 and later (`ChooserAction`). `Unsupported` everywhere else: the chooser
    /// shows the system's targets only, and the app offers its actions in its own menu.
    ShareActions,
    /// A `scroll` reports its content offset as it moves (`ScrollProps::track_offset`,
    /// [`Event::ScrollChanged`], docs/scroll.md "Offset and phase"). `Unsupported`: the bound
    /// offset never changes, so a collapsing header stays expanded.
    ScrollOffset,
    /// A `scroll` reports the phases of user-driven scrolling (`ScrollProps::track_phase`,
    /// [`Event::ScrollPhase`]). `Native` where the platform says when a drag begins, when the
    /// finger lifts and when momentum ends; anything else and day-core derives `Dragging` and
    /// `Idle` from the offset reports (never `Coasting`), which needs [`Cap::ScrollOffset`].
    ScrollPhases,
    /// A `scroll` comes to rest on page or interval boundaries (`ScrollProps::snap`).
    /// `Unsupported`: the view scrolls freely.
    ScrollSnap,
    /// A `scroll` zooms its content by pinch and by [`props::ScrollPatch::Zoom`]
    /// (`ScrollProps::zoom`, docs/scroll.md "Zoom"). `Unsupported`: the content stays at its
    /// natural size and the bound scale stays `1.0`.
    ScrollZoom,
    /// A `scroll` dismisses the software keyboard as its props ask
    /// (`ScrollProps::keyboard_dismiss`, `ScrollProps::keyboard_taps`). `Native` where the
    /// platform's scroll view does it; `Emulated` where day-core resigns focus when a drag
    /// begins (no interactive tracking, and taps are always delivered); `Unsupported` where
    /// there is no software keyboard to dismiss.
    ScrollKeyboardDismiss,
    /// A native `pager` reports a continuous position while a page is dragged (docs/pager.md
    /// "Progress"). Anything else and the bound progress steps from page to page.
    PagerProgress,
    /// The list host reports its viewport through [`ListSource::viewport`] (docs/list.md
    /// "Visible rows"): what `visible_rows`, `follow_end` and `on_end_reached` are derived
    /// from. `Unsupported`: `visible_rows` follows only `first_visible` reports, `follow_end`
    /// follows every change, and `on_end_reached` never fires.
    ListViewport,
    /// The list host tells a scroll the reader started from one the app asked for, through
    /// [`ListSource::reader_scroll`] (docs/list.md "Following the end"). `Unsupported`:
    /// `follow_end` learns that the reader left only from the viewport reports, so a reader who
    /// interrupts a followed scroll before it reaches the end can be taken back to the end by
    /// the next row.
    ListReaderScroll,
    /// The list host honors [`ListScroll::align`] and [`ListScroll::animated`]
    /// ([`Toolkit::list_scroll`]). `Unsupported`: a row scroll reveals the row where the
    /// platform's own "scroll to row" puts it.
    ListScrollAlign,
    /// `Decorate::focusable` is honored ([`Toolkit::set_focusable`], docs/focus.md): a
    /// composed piece joins the focus order, takes focus on a press, reports
    /// `Event::FocusChanged` and hears `Event::Key` while focused. `Unsupported` ⇒ the piece
    /// renders and never takes the keyboard; offer the same commands through buttons or a
    /// menu there.
    Focusable,
    /// Key events carry every phase (docs/menus.md "Keys"): `Native` when the backend reports
    /// the release (`KeyPhase::Up`) and marks auto-repeat (`KeyPhase::Repeat`). `Unsupported`
    /// ⇒ only presses arrive, each as `KeyPhase::Down`, so held-key movement has no end to
    /// wait for and should step per press instead.
    KeyPhases,
    /// `Decorate::hit_slop` grows the piece's input area ([`Toolkit::set_hit_slop`],
    /// docs/gestures.md "Hit slop"). `Unsupported` ⇒ the piece is hit inside its own bounds.
    HitSlop,
}

impl Cap {
    /// Every capability, in declaration order: what a conformance run asks the toolkit about,
    /// so the evidence can be held against the declared coverage matrix (docs/testing.md).
    pub const ALL: [Cap; 104] = [
        Cap::DragDrop,
        Cap::DragExternalImport,
        Cap::DragExternalExport,
        Cap::DragMultipleItems,
        Cap::DragFileReferences,
        Cap::DragDeferredReceipt,
        Cap::DragFilePromises,
        Cap::DragExternalMove,
        Cap::ListRecycling,
        Cap::UndoBridge,
        Cap::EditBridge,
        Cap::ListReorder,
        Cap::NavReorder,
        Cap::ListDelete,
        Cap::ListSwipeActions,
        Cap::Tree,
        Cap::TreeMove,
        Cap::BaselineAlignment,
        Cap::TextRuns,
        Cap::TextLinks,
        Cap::Lottie,
        Cap::NativeSymbols,
        Cap::Snapshot,
        Cap::NavSplit,
        Cap::NavTabs,
        Cap::NavTabsAdaptive,
        Cap::NavRepresent,
        Cap::NavContentList,
        Cap::NavHeader,
        Cap::Appearance,
        Cap::AppMenu,
        Cap::Dialogs,
        Cap::FileDialogs,
        Cap::Animation,
        Cap::Cover,
        Cap::StatusBarHidden,
        Cap::TextEditable,
        Cap::TextSelectable,
        Cap::TextSpellCheck,
        Cap::MultiWindow,
        Cap::AppBadgeCount,
        Cap::AppBadgeText,
        Cap::AppBadgeDot,
        Cap::Toolbar,
        Cap::ToolbarSearch,
        Cap::Inspector,
        Cap::Cursor,
        Cap::FontList,
        Cap::ReduceMotion,
        Cap::Announce,
        Cap::ImageDecode,
        Cap::ImageEncode,
        Cap::ImageProperties,
        Cap::WindowStates,
        Cap::WindowFullscreen,
        Cap::ContentProtection,
        Cap::WindowTabbing,
        Cap::DockMenu,
        Cap::WindowGeometry,
        Cap::WindowPosition,
        Cap::WindowLevel,
        Cap::WindowAppearance,
        Cap::RequestAttention,
        Cap::FramelessWindow,
        Cap::OverlayTitleBar,
        Cap::TransparentWindow,
        Cap::WindowMaterial,
        Cap::DragRegion,
        Cap::StatusItem,
        Cap::AppProgress,
        Cap::DockVisibility,
        Cap::Monitors,
        Cap::DynamicShortcuts,
        Cap::Split,
        Cap::TreeSections,
        Cap::CoverSheet,
        Cap::ListEditing,
        Cap::ListItem,
        Cap::CanvasBitmap,
        Cap::CanvasPdf,
        Cap::SearchField,
        Cap::ComboBox,
        Cap::Stepper,
        Cap::DateTimePicker,
        Cap::ColorPicker,
        Cap::PullRefresh,
        Cap::Pager,
        Cap::PullRefreshGesture,
        Cap::FolderDialog,
        Cap::ListSections,
        Cap::ShareActions,
        Cap::ScrollOffset,
        Cap::ScrollPhases,
        Cap::ScrollSnap,
        Cap::ScrollZoom,
        Cap::ScrollKeyboardDismiss,
        Cap::PagerProgress,
        Cap::ListViewport,
        Cap::ListScrollAlign,
        Cap::Focusable,
        Cap::KeyPhases,
        Cap::HitSlop,
        Cap::LabelSelection,
        Cap::ListReaderScroll,
    ];

    /// The capability's place in [`Cap::ALL`]. Exhaustive, so a new variant fails to compile
    /// here until it is given one, and the unit test below then holds `ALL` to it.
    #[must_use]
    pub const fn ordinal(self) -> usize {
        match self {
            Cap::DragDrop => 0,
            Cap::DragExternalImport => 1,
            Cap::DragExternalExport => 2,
            Cap::DragMultipleItems => 3,
            Cap::DragFileReferences => 4,
            Cap::DragDeferredReceipt => 5,
            Cap::DragFilePromises => 6,
            Cap::DragExternalMove => 7,
            Cap::ListRecycling => 8,
            Cap::UndoBridge => 9,
            Cap::EditBridge => 10,
            Cap::ListReorder => 11,
            Cap::NavReorder => 12,
            Cap::ListDelete => 13,
            Cap::ListSwipeActions => 14,
            Cap::Tree => 15,
            Cap::TreeMove => 16,
            Cap::BaselineAlignment => 17,
            Cap::TextRuns => 18,
            Cap::TextLinks => 19,
            Cap::Lottie => 20,
            Cap::NativeSymbols => 21,
            Cap::Snapshot => 22,
            Cap::NavSplit => 23,
            Cap::NavTabs => 24,
            Cap::NavTabsAdaptive => 25,
            Cap::NavRepresent => 26,
            Cap::NavContentList => 27,
            Cap::NavHeader => 28,
            Cap::Appearance => 29,
            Cap::AppMenu => 30,
            Cap::Dialogs => 31,
            Cap::FileDialogs => 32,
            Cap::Animation => 33,
            Cap::Cover => 34,
            Cap::StatusBarHidden => 35,
            Cap::TextEditable => 36,
            Cap::TextSelectable => 37,
            Cap::TextSpellCheck => 38,
            Cap::MultiWindow => 39,
            Cap::AppBadgeCount => 40,
            Cap::AppBadgeText => 41,
            Cap::AppBadgeDot => 42,
            Cap::Toolbar => 43,
            Cap::ToolbarSearch => 44,
            Cap::Inspector => 45,
            Cap::Cursor => 46,
            Cap::FontList => 47,
            Cap::ReduceMotion => 48,
            Cap::Announce => 49,
            Cap::ImageDecode => 50,
            Cap::ImageEncode => 51,
            Cap::ImageProperties => 52,
            Cap::WindowStates => 53,
            Cap::WindowFullscreen => 54,
            Cap::ContentProtection => 55,
            Cap::WindowTabbing => 56,
            Cap::DockMenu => 57,
            Cap::WindowGeometry => 58,
            Cap::WindowPosition => 59,
            Cap::WindowLevel => 60,
            Cap::WindowAppearance => 61,
            Cap::RequestAttention => 62,
            Cap::FramelessWindow => 63,
            Cap::OverlayTitleBar => 64,
            Cap::TransparentWindow => 65,
            Cap::WindowMaterial => 66,
            Cap::DragRegion => 67,
            Cap::StatusItem => 68,
            Cap::AppProgress => 69,
            Cap::DockVisibility => 70,
            Cap::Monitors => 71,
            Cap::DynamicShortcuts => 72,
            Cap::Split => 73,
            Cap::TreeSections => 74,
            Cap::CoverSheet => 75,
            Cap::ListEditing => 76,
            Cap::ListItem => 77,
            Cap::CanvasBitmap => 78,
            Cap::CanvasPdf => 79,
            Cap::SearchField => 80,
            Cap::ComboBox => 81,
            Cap::Stepper => 82,
            Cap::DateTimePicker => 83,
            Cap::ColorPicker => 84,
            Cap::PullRefresh => 85,
            Cap::Pager => 86,
            Cap::PullRefreshGesture => 87,
            Cap::FolderDialog => 88,
            Cap::ListSections => 89,
            Cap::ShareActions => 90,
            Cap::ScrollOffset => 91,
            Cap::ScrollPhases => 92,
            Cap::ScrollSnap => 93,
            Cap::ScrollZoom => 94,
            Cap::ScrollKeyboardDismiss => 95,
            Cap::PagerProgress => 96,
            Cap::ListViewport => 97,
            Cap::ListScrollAlign => 98,
            Cap::Focusable => 99,
            Cap::KeyPhases => 100,
            Cap::HitSlop => 101,
            Cap::LabelSelection => 102,
            Cap::ListReaderScroll => 103,
        }
    }
}

// ---------------------------------------------------------------------------
// Raster images (docs/images.md): the currency shared by the `image` piece, the canvas, and
// the decode/metadata/encode duties below.
// ---------------------------------------------------------------------------

/// A decoded image the toolkit holds, addressed by an id day-core mints.
///
/// Opaque, because what sits behind it is an `NSImage`, a `UIImage`, a `GdkTexture`, a
/// `QPixmap`, a `BitmapImage`, an android `Bitmap` or an `ImageBitmap`, and none of those cross
/// the boundary. The id is released through [`Toolkit::release_image`] when the app's handle
/// drops, so a backend's side table never outlives the app's interest in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct BitmapId(pub u64);

/// An encoded image format: what [`BitmapInfo::format`] reports and [`EncodeSpec::format`] asks
/// for. Not every backend encodes every one: ask [`Toolkit::encode_formats`] (the
/// [`Cap::ImageEncode`] pair of [`Toolkit::font_families`]/[`Cap::FontList`]) rather than
/// assuming.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub enum ImageFormat {
    #[default]
    Png,
    Jpeg,
    Webp,
    Gif,
    Bmp,
    Tiff,
    /// HEIF/HEIC, Apple's camera format; decodable on the Apple backends and Android, and
    /// encodable almost nowhere.
    Heif,
    /// Decoded from bytes whose magic numbers [`ImageFormat::sniff`] does not know. A backend may
    /// still decode it: the platform decoders read more formats than Day names.
    Unknown,
}

impl ImageFormat {
    /// The IANA media type, which is also the string the ArkUI packer, GDI+ and the browser's
    /// `toBlob` take. `Unknown` answers `application/octet-stream`.
    #[must_use]
    pub fn mime(self) -> &'static str {
        match self {
            ImageFormat::Png => "image/png",
            ImageFormat::Jpeg => "image/jpeg",
            ImageFormat::Webp => "image/webp",
            ImageFormat::Gif => "image/gif",
            ImageFormat::Bmp => "image/bmp",
            ImageFormat::Tiff => "image/tiff",
            ImageFormat::Heif => "image/heif",
            ImageFormat::Unknown => "application/octet-stream",
        }
    }

    /// The pixel size the encoded bytes declare, read from the container's own header
    /// (docs/images.md).
    ///
    /// Every backend that decodes natively reports its own dimensions; this exists for the one
    /// that cannot. The XAML backend's (`windows-winui`) `BitmapImage` learns its size
    /// asynchronously, only once the element has been shown, so a synchronous `image_info` there
    /// would have nothing to say.
    /// Reading the header instead gives the same answer the decoder would, before any decoding.
    ///
    /// `None` when the header is absent, truncated, or a container this does not parse.
    #[must_use]
    pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
        let be16 = |i: usize| -> Option<u32> {
            Some(u32::from(u16::from_be_bytes([
                *bytes.get(i)?,
                *bytes.get(i + 1)?,
            ])))
        };
        let le16 = |i: usize| -> Option<u32> {
            Some(u32::from(u16::from_le_bytes([
                *bytes.get(i)?,
                *bytes.get(i + 1)?,
            ])))
        };
        let be32 = |i: usize| -> Option<u32> {
            Some(u32::from_be_bytes([
                *bytes.get(i)?,
                *bytes.get(i + 1)?,
                *bytes.get(i + 2)?,
                *bytes.get(i + 3)?,
            ]))
        };
        let le32 = |i: usize| -> Option<u32> {
            Some(u32::from_le_bytes([
                *bytes.get(i)?,
                *bytes.get(i + 1)?,
                *bytes.get(i + 2)?,
                *bytes.get(i + 3)?,
            ]))
        };
        match Self::sniff(bytes)? {
            // IHDR is always the first chunk: 8-byte signature, 4-byte length, 4-byte type.
            ImageFormat::Png => Some((be32(16)?, be32(20)?)),
            // The logical screen descriptor follows the 6-byte version stamp, little-endian.
            ImageFormat::Gif => Some((le16(6)?, le16(8)?)),
            // The DIB header's width/height are signed; a negative height is a top-down bitmap.
            ImageFormat::Bmp => Some((
                le32(18)?.cast_signed().unsigned_abs(),
                le32(22)?.cast_signed().unsigned_abs(),
            )),
            // Walk the segment chain to the frame header. Every SOF marker carries the size at
            // the same offsets; the arithmetic-coded and progressive ones are SOFs too, which is
            // why this matches the whole range rather than SOF0 alone. DNL/DHP (0xC4, 0xC8,
            // 0xCC) are not frame headers and are skipped like any other segment.
            ImageFormat::Jpeg => {
                let mut i = 2usize;
                // `<=`, not `<`: a frame header that ends the buffer is still a frame header, and
                // the size sits in its last four bytes.
                while i + 9 <= bytes.len() {
                    if bytes[i] != 0xFF {
                        i += 1;
                        continue;
                    }
                    let marker = bytes[i + 1];
                    // Padding fill bytes and the standalone markers carry no length.
                    if marker == 0xFF || (0xD0..=0xD9).contains(&marker) || marker == 0x01 {
                        i += 1;
                        continue;
                    }
                    let is_sof = (0xC0..=0xCF).contains(&marker)
                        && marker != 0xC4
                        && marker != 0xC8
                        && marker != 0xCC;
                    if is_sof {
                        // [marker][len:2][precision:1][height:2][width:2]
                        return Some((be16(i + 7)?, be16(i + 5)?));
                    }
                    i += 2 + be16(i + 2)? as usize;
                }
                None
            }
            // RIFF: a 12-byte header, then a chunk id. VP8X states the size minus one in 24-bit
            // little-endian; lossy VP8 hides it in the 14 low bits after the start code; lossless
            // VP8L packs width-1 and height-1 into 14 bits each across four bytes.
            ImageFormat::Webp => match bytes.get(12..16)? {
                b"VP8X" => {
                    let w = u32::from(*bytes.get(24)?)
                        | u32::from(*bytes.get(25)?) << 8
                        | u32::from(*bytes.get(26)?) << 16;
                    let h = u32::from(*bytes.get(27)?)
                        | u32::from(*bytes.get(28)?) << 8
                        | u32::from(*bytes.get(29)?) << 16;
                    Some((w + 1, h + 1))
                }
                b"VP8 " => Some((le16(26)? & 0x3FFF, le16(28)? & 0x3FFF)),
                b"VP8L" => {
                    let bits = le32(21)?;
                    Some(((bits & 0x3FFF) + 1, ((bits >> 14) & 0x3FFF) + 1))
                }
                _ => None,
            },
            // TIFF's IFD is a pointer chase and HEIF's is a box tree; neither is worth parsing
            // here, because every backend that reads them decodes natively and answers for itself.
            ImageFormat::Tiff | ImageFormat::Heif | ImageFormat::Unknown => None,
        }
    }

    /// The format `bytes` begins with, by magic number: enough to fill [`BitmapInfo::format`]
    /// on every backend, including the ones whose decoder never says what it decoded.
    ///
    /// Hand-rolled rather than a crate: it is a dozen byte comparisons against fixed prefixes,
    /// and the alternative is an image-decoding dependency in the one crate that must stay
    /// dependency-free.
    #[must_use]
    pub fn sniff(bytes: &[u8]) -> Option<ImageFormat> {
        let starts = |sig: &[u8]| bytes.len() >= sig.len() && &bytes[..sig.len()] == sig;
        if starts(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
            return Some(ImageFormat::Png);
        }
        if starts(&[0xff, 0xd8, 0xff]) {
            return Some(ImageFormat::Jpeg);
        }
        if starts(b"GIF87a") || starts(b"GIF89a") {
            return Some(ImageFormat::Gif);
        }
        if starts(b"BM") {
            return Some(ImageFormat::Bmp);
        }
        if starts(&[0x49, 0x49, 0x2a, 0x00]) || starts(&[0x4d, 0x4d, 0x00, 0x2a]) {
            return Some(ImageFormat::Tiff);
        }
        // RIFF....WEBP and ....ftypheic/heix/mif1 both carry their tag past a 4-byte prefix.
        if starts(b"RIFF") && bytes.len() >= 12 && &bytes[8..12] == b"WEBP" {
            return Some(ImageFormat::Webp);
        }
        if bytes.len() >= 12 && &bytes[4..8] == b"ftyp" {
            let brand = &bytes[8..12];
            if brand == b"heic" || brand == b"heix" || brand == b"mif1" || brand == b"msf1" {
                return Some(ImageFormat::Heif);
            }
        }
        None
    }
}

/// What a decoded image is, in the terms every backend can answer
/// ([`Toolkit::image_info`], docs/images.md).
///
/// `pixels` is the image's own pixel size, not the points it draws at: an asset that stages at
/// 2× reports twice its point size and a `scale` of 2, so an app sizing a frame divides. Bytes
/// carry no density, so a decoded buffer always answers `scale: 1.0`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BitmapInfo {
    pub pixels: Size,
    /// Pixels per point (1.0 for decoded bytes; 2 or 3 for a catalog/drawable asset that staged
    /// a density variant).
    pub scale: f64,
    /// The encoded format it came from, where the backend knows or [`ImageFormat::sniff`] said.
    pub format: Option<ImageFormat>,
    pub has_alpha: bool,
}

/// What [`Toolkit::encode_image`] should produce (docs/images.md).
///
/// A struct rather than three positional arguments, per docs/api-style.md: the call site reads
/// `EncodeSpec { format: ImageFormat::Jpeg, quality: Some(0.8), ..Default::default() }`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EncodeSpec {
    pub format: ImageFormat,
    /// Lossy quality in `0.0..=1.0`. `None` takes the platform's default; a lossless format
    /// ignores it.
    pub quality: Option<f64>,
    /// Scale the longest side down to fit this box first (aspect preserved). `None` encodes at
    /// the image's own size.
    pub fit: Option<Size>,
}

impl Default for EncodeSpec {
    fn default() -> Self {
        EncodeSpec {
            format: ImageFormat::Png,
            quality: None,
            fit: None,
        }
    }
}

/// The metadata an image file carries beyond its pixels ([`Toolkit::image_properties`]).
///
/// Sparse by construction: every field is what the platform's own reader answered, and a
/// backend with no metadata reader at all answers `None` to the whole call rather than an empty
/// struct. Probe [`Cap::ImageProperties`] before offering an affordance that depends on it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ImageProperties {
    /// The EXIF orientation tag (1–8), where one was recorded.
    pub orientation: Option<u16>,
    /// Pixels per inch, where the file records a resolution.
    pub dpi: Option<f64>,
    /// The capture timestamp as the file spells it (no parsing: the formats disagree).
    pub created: Option<String>,
    /// Everything else the platform reader named, in its own vocabulary. Keys are the
    /// platform's, so read them defensively.
    pub extra: Vec<(String, String)>,
}

/// Why an image operation could not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageError {
    /// The bytes are not an image this platform decodes.
    Decode,
    /// The image decoded, but this backend cannot encode the format asked for. `encode_formats`
    /// says which it can.
    Encode,
    /// The backend has no path for this at all (probe [`Cap::ImageDecode`] /
    /// [`Cap::ImageEncode`]).
    Unsupported,
    /// The id named no live image: released already, or never decoded.
    Gone,
}

impl std::fmt::Display for ImageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageError::Decode => write!(f, "the bytes are not a decodable image"),
            ImageError::Encode => write!(f, "this backend cannot encode that format"),
            ImageError::Unsupported => write!(f, "image support is unavailable on this backend"),
            ImageError::Gone => write!(f, "the image was released"),
        }
    }
}

impl std::error::Error for ImageError {}

// ---------------------------------------------------------------------------
// Canvas export (docs/canvas.md "Exporting"): a recorded drawing replayed into an offscreen
// bitmap or a PDF by the same painter the on-screen canvas uses.
// ---------------------------------------------------------------------------

/// The largest side, in pixels, a rendered bitmap may have (docs/canvas.md "Exporting").
/// day-core answers [`ExportError::TooLarge`] above it before any toolkit allocates.
pub const EXPORT_MAX_SIDE: u32 = 16_384;

/// The most pixels a rendered bitmap may hold in total: 64 megapixels, 256 MB at four bytes a
/// pixel, which is already more than the tightest per-app heap (Android's) comfortably holds.
pub const EXPORT_MAX_PIXELS: u64 = 64 * 1024 * 1024;

/// The pixel size a drawing of `size` points renders to at `scale` pixels per point: each side
/// rounded up, so a drawing never loses its last partial pixel. Shared by day-core's limit
/// check and every backend, so the bitmap a backend allocates is the one day-core approved.
#[must_use]
pub fn export_pixels(size: Size, scale: f64) -> (u32, u32) {
    let side = |v: f64| (v * scale).ceil().clamp(0.0, f64::from(u32::MAX)) as u32;
    (side(size.width), side(size.height))
}

/// One page of a PDF [`Toolkit::render_pdf`] writes: the page size in PDF points (1/72 inch,
/// the same unit as canvas points) and the ops to replay onto it, origin top-left.
#[derive(Clone, Debug, PartialEq)]
pub struct PdfPage {
    pub size: Size,
    pub ops: Vec<DrawOp>,
}

/// The document information a PDF carries. A backend whose writer has no metadata API (Android's
/// `PdfDocument`) ignores it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PdfInfo {
    pub title: Option<String>,
    pub author: Option<String>,
}

/// Why a canvas export could not answer (docs/canvas.md "Exporting").
#[derive(Clone, Debug, PartialEq)]
pub enum ExportError {
    /// This toolkit has no path for the export (probe [`Cap::CanvasBitmap`] /
    /// [`Cap::CanvasPdf`]).
    Unsupported,
    /// The bitmap would exceed [`EXPORT_MAX_SIDE`] or [`EXPORT_MAX_PIXELS`]; checked in
    /// day-core before the toolkit runs. The size is the one that was refused, in pixels.
    TooLarge { width: u32, height: u32 },
    /// A zero, negative or non-finite size or scale, or a PDF with no pages.
    Invalid,
    /// The platform refused (out of memory, a writer error); the text is the platform's.
    Failed(String),
}

impl std::fmt::Display for ExportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportError::Unsupported => write!(f, "this export is unavailable on this backend"),
            ExportError::TooLarge { width, height } => {
                write!(
                    f,
                    "a {width} × {height} pixel bitmap is too large to render"
                )
            }
            ExportError::Invalid => write!(f, "the drawing has no size or the PDF has no pages"),
            ExportError::Failed(why) => write!(f, "rendering failed: {why}"),
        }
    }
}

impl std::error::Error for ExportError {}

/// Where a raster image's pixels come from (docs/images.md).
///
/// One currency for both halves of the same question: `Named` is the staged asset every
/// `image(res::images::…)` has always drawn, `Bytes` is an encoded PNG/JPEG the app already
/// holds (a download, a file the user picked, a clipboard paste), and `Decoded` names an image
/// the toolkit decoded once so several nodes (and the canvas) share it.
#[derive(Clone)]
pub enum ImageSource {
    /// A staged asset name, resolved through the backend's own pipeline (§18.2).
    Named(String),
    /// Encoded bytes (PNG/JPEG/…) the app owns, decoded by the platform's own decoder. Shared
    /// rather than copied: handing over a new buffer is a refcount bump.
    Bytes(std::sync::Arc<Vec<u8>>),
    /// An image already decoded through [`Toolkit::decode_image`].
    Decoded(BitmapId),
}

impl Default for ImageSource {
    fn default() -> Self {
        ImageSource::Named(String::new())
    }
}

/// Bytes compare by identity, not content.
///
/// This is the equality a binding gates on (§4.2), so it runs on every evaluation, and
/// comparing two multi-megabyte buffers byte by byte would cost far more than the redundant
/// decode it saves. Handing over a distinct buffer holding the same pixels therefore re-decodes,
/// which is the same trade `Arc::ptr_eq` makes wherever else Day carries shared buffers.
impl PartialEq for ImageSource {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ImageSource::Named(a), ImageSource::Named(b)) => a == b,
            (ImageSource::Bytes(a), ImageSource::Bytes(b)) => std::sync::Arc::ptr_eq(a, b),
            (ImageSource::Decoded(a), ImageSource::Decoded(b)) => a == b,
            _ => false,
        }
    }
}

/// Logs the buffer's length, never its contents: a `Debug` that dumped a megabyte of pixels
/// into a trace line would make the trace useless and slow.
impl std::fmt::Debug for ImageSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImageSource::Named(n) => f.debug_tuple("Named").field(n).finish(),
            ImageSource::Bytes(b) => f.debug_tuple("Bytes").field(&b.len()).finish(),
            ImageSource::Decoded(id) => f.debug_tuple("Decoded").field(&id.0).finish(),
        }
    }
}

impl From<BitmapId> for ImageSource {
    fn from(id: BitmapId) -> Self {
        ImageSource::Decoded(id)
    }
}

/// A staged asset name is a source, so `image(res::images::cover)` keeps compiling unchanged.
///
/// There is no `From<&str>` here, for the same reason [`resource::ImageName`] has none:
/// `image("typo")` must stay a compile error rather than a blank view at runtime (resource.rs).
/// A path or URL string is not a source either: read the bytes and hand those over.
impl From<resource::ImageName> for ImageSource {
    fn from(name: resource::ImageName) -> Self {
        ImageSource::Named(name.as_str().to_owned())
    }
}

impl From<resource::VectorName> for ImageSource {
    fn from(name: resource::VectorName) -> Self {
        ImageSource::Named(name.as_str().to_owned())
    }
}

impl From<std::sync::Arc<Vec<u8>>> for ImageSource {
    fn from(bytes: std::sync::Arc<Vec<u8>>) -> Self {
        ImageSource::Bytes(bytes)
    }
}

/// An owned string is a name, not bytes: the same conversion [`resource::ImageName`] offers for
/// a name computed at runtime, and the reason `image(some_string)` keeps compiling. A `&str`
/// still converts nowhere, so `image("typo")` stays a compile error (resource.rs).
impl From<String> for ImageSource {
    fn from(name: String) -> Self {
        ImageSource::Named(name)
    }
}

impl From<Vec<u8>> for ImageSource {
    fn from(bytes: Vec<u8>) -> Self {
        ImageSource::Bytes(std::sync::Arc::new(bytes))
    }
}

/// What a text field collects (`text_field(..).input_purpose(_)`, docs/textfield.md).
///
/// One word for everything a platform keys off the kind of text: the on-screen keyboard's
/// layout, capitalization and correction, and the credential or contact the system offers to
/// fill in. A backend applies the parts its toolkit has: a desktop toolkit with no on-screen
/// keyboard still takes the autofill and correction half.
///
/// Purpose is independent of [`InputTraits::secure`]: a numeric PIN is `Number` and secure.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum InputPurpose {
    /// Free text: the default keyboard, with the platform's own capitalization and correction.
    #[default]
    Text,
    /// A person's name: words capitalized, never corrected.
    Name,
    /// An email address: the `@` keyboard, never capitalized or corrected.
    Email,
    /// A web address: the URL keyboard, never capitalized or corrected.
    Url,
    /// A telephone number: the phone pad.
    Phone,
    /// A whole number: the digit pad.
    Number,
    /// A number with a fractional part: the digit pad with a decimal separator.
    Decimal,
    /// An account name: never capitalized or corrected, and offered to the password manager.
    Username,
    /// An existing password, offered to the password manager for filling.
    Password,
    /// A password being chosen, so the password manager offers to make and save one.
    NewPassword,
    /// A code sent by message or mail, which the keyboard offers as it arrives. The keyboard
    /// stays the text one, since a code can carry letters.
    OneTimeCode,
}

/// The on-screen keyboard's action key (`text_field(..).submit_label(_)`, docs/textfield.md).
///
/// The key always fires the field's `on_submit`; this names what it promises. A toolkit with
/// no on-screen keyboard ignores it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SubmitLabel {
    /// The platform's own key (Return).
    #[default]
    Return,
    Done,
    Go,
    Next,
    Search,
    Send,
}

/// How a text field takes its text (docs/textfield.md): the whole set travels together through
/// [`Toolkit::set_input_traits`], so a backend that has to rebuild the widget for one member
/// has every other member in hand to dress the replacement.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct InputTraits {
    /// Hide the characters (a password field).
    pub secure: bool,
    /// Show and select the text, take no edits.
    pub read_only: bool,
    pub purpose: InputPurpose,
    pub submit_label: SubmitLabel,
    /// The most characters (Unicode scalar values) the field takes; `None` is unbounded.
    /// day-pieces holds the bound on every backend. A toolkit applies its native limit as well
    /// only where that limit counts the same unit (GTK): one that counts UTF-16 units would
    /// refuse characters the bound allows.
    pub max_length: Option<u32>,
}

/// The pointer's shape over a piece (the `.cursor()` decorator, docs/cursor.md).
///
/// The named variants are the CSS cursor vocabulary: the largest set any toolkit accepts as it
/// is (GTK and the DOM take the names verbatim), and every other toolkit's set is a subset of it
/// or maps onto it. A backend draws its own shape for each name, or the nearest one it has, and
/// answers [`Cap::Cursor`] with how faithful that is. [`Cursor::Native`] reaches a shape only one
/// toolkit names; the `day::cursor::<toolkit>` modules carry those as constants, gated on that
/// toolkit's feature, and every other backend ignores a name it does not know.
///
/// A cursor applies to the node and its descendants until a descendant sets its own.
/// [`Cursor::Default`] hands the shape back to the platform rather than pinning an arrow, so a
/// reactive cursor can release what it took.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Default)]
pub enum Cursor {
    /// The platform default (an arrow), and the value that releases a node's cursor.
    #[default]
    Default,
    /// A pointing hand: links and other tap-to-activate content.
    Pointer,
    /// An I-beam over editable or selectable text.
    Text,
    /// An I-beam for vertical text.
    VerticalText,
    Crosshair,
    /// Four-way move.
    Move,
    /// An open hand: content that can be dragged.
    Grab,
    /// A closed hand: content being dragged.
    Grabbing,
    NotAllowed,
    /// The app is busy and cannot take input.
    Wait,
    /// The app is busy but still takes input.
    Progress,
    Help,
    ContextMenu,
    /// A drop will copy.
    Copy,
    /// A drop will link.
    Alias,
    /// A table cell.
    Cell,
    ZoomIn,
    ZoomOut,
    /// No pointer at all.
    None,
    /// Vertical (north–south) resize.
    NsResize,
    /// Horizontal (east–west) resize.
    EwResize,
    /// Diagonal resize, north-east to south-west.
    NeswResize,
    /// Diagonal resize, north-west to south-east.
    NwseResize,
    ColResize,
    RowResize,
    /// A toolkit's own named cursor, resolved by that backend's lookup and ignored by every
    /// other. Prefer the `day::cursor::<toolkit>` constants, which are gated on the toolkit.
    Native(std::borrow::Cow<'static, str>),
}

impl Cursor {
    /// A toolkit-specific cursor by its native name (see [`Cursor::Native`]).
    #[must_use]
    pub const fn native(name: &'static str) -> Self {
        Cursor::Native(std::borrow::Cow::Borrowed(name))
    }

    /// The CSS keyword for a named cursor, which GTK and the DOM take as it is. A native cursor
    /// answers its own name, so a backend keyed on CSS names may still recognize it.
    #[must_use]
    pub fn css_name(&self) -> &str {
        match self {
            Cursor::Default => "default",
            Cursor::Pointer => "pointer",
            Cursor::Text => "text",
            Cursor::VerticalText => "vertical-text",
            Cursor::Crosshair => "crosshair",
            Cursor::Move => "move",
            Cursor::Grab => "grab",
            Cursor::Grabbing => "grabbing",
            Cursor::NotAllowed => "not-allowed",
            Cursor::Wait => "wait",
            Cursor::Progress => "progress",
            Cursor::Help => "help",
            Cursor::ContextMenu => "context-menu",
            Cursor::Copy => "copy",
            Cursor::Alias => "alias",
            Cursor::Cell => "cell",
            Cursor::ZoomIn => "zoom-in",
            Cursor::ZoomOut => "zoom-out",
            Cursor::None => "none",
            Cursor::NsResize => "ns-resize",
            Cursor::EwResize => "ew-resize",
            Cursor::NeswResize => "nesw-resize",
            Cursor::NwseResize => "nwse-resize",
            Cursor::ColResize => "col-resize",
            Cursor::RowResize => "row-resize",
            Cursor::Native(name) => name,
        }
    }

    /// Every named cursor, in declaration order, what a gallery or a test walks.
    pub const NAMED: [Cursor; 25] = [
        Cursor::Default,
        Cursor::Pointer,
        Cursor::Text,
        Cursor::VerticalText,
        Cursor::Crosshair,
        Cursor::Move,
        Cursor::Grab,
        Cursor::Grabbing,
        Cursor::NotAllowed,
        Cursor::Wait,
        Cursor::Progress,
        Cursor::Help,
        Cursor::ContextMenu,
        Cursor::Copy,
        Cursor::Alias,
        Cursor::Cell,
        Cursor::ZoomIn,
        Cursor::ZoomOut,
        Cursor::None,
        Cursor::NsResize,
        Cursor::EwResize,
        Cursor::NeswResize,
        Cursor::NwseResize,
        Cursor::ColResize,
        Cursor::RowResize,
    ];
}

/// A set of screen edges, for the `defers_system_gestures` modifier (docs/cover.md). Mirrors
/// SwiftUI's `Edge.Set`: on iOS these map to `UIRectEdge` for
/// `preferredScreenEdgesDeferringSystemGestures`; on Android any non-empty set enters
/// swipe-to-reveal immersive mode (the closest platform analogue).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Edges(pub u8);

impl Edges {
    pub const NONE: Edges = Edges(0);
    pub const TOP: Edges = Edges(1);
    pub const BOTTOM: Edges = Edges(2);
    pub const LEADING: Edges = Edges(4);
    pub const TRAILING: Edges = Edges(8);
    pub const ALL: Edges = Edges(15);

    #[must_use]
    pub fn contains(self, other: Edges) -> bool {
        self.0 & other.0 == other.0
    }
    #[must_use]
    pub fn union(self, other: Edges) -> Edges {
        Edges(self.0 | other.0)
    }
    #[must_use]
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::ops::BitOr for Edges {
    type Output = Edges;
    fn bitor(self, rhs: Edges) -> Edges {
        self.union(rhs)
    }
}

/// The timing curve of one cycle of an animation (§8.4, docs/tween.md "The animation
/// contract"): progress as a function of the time since the cycle started.
///
/// The four easing curves are defined by [`Curve::ease`], so every toolkit and the frame-driven
/// path draw the same shape: `Linear` is `t`, `EaseIn` is `t²`, `EaseOut` is `1 - (1 - t)²` and
/// `EaseInOut` is the smoothstep `t²(3 - 2t)`. A toolkit whose native easing differs (Core
/// Animation's `easeInEaseOut` is a different cubic) may use its own curve of the same name:
/// the conformance cases allow that much difference and no more.
///
/// `Spring` is a physical spring, not an overshoot curve stretched over a duration. Its
/// position is the closed-form unit-step response [`Curve::fraction`] evaluates, the response
/// of SwiftUI's `.spring(response:dampingFraction:)`. A toolkit with a spring animator hands
/// it [`Curve::spring_constants`]; every other toolkit plays [`AnimSpec::keyframes`], which
/// samples the same function.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    /// Constant speed.
    Linear,
    /// Slow at the start: `t²`.
    EaseIn,
    /// Slow at the end: `1 - (1 - t)²`.
    EaseOut,
    /// Slow at both ends: `t²(3 - 2t)`.
    EaseInOut,
    /// A damped spring released at rest one unit from its target.
    Spring {
        /// The period of the undamped oscillation, in seconds: the natural frequency is
        /// `2π / response` radians per second. Lower is faster.
        response: f64,
        /// The damping ratio: `1.0` is critically damped (the fastest approach with no
        /// overshoot), below `1.0` overshoots and rings, above `1.0` creeps in.
        damping: f64,
    },
}

/// How close to its target a spring has to stay to count as settled: a thousandth of the
/// distance it travels.
const SPRING_REST: f64 = 0.001;

impl Curve {
    /// Fraction of the transition complete at `elapsed` seconds into a cycle of `duration`
    /// seconds (0 at the start, reaching 1, or overshooting it for an under-damped spring).
    /// Easing curves clamp to `duration`; a spring evaluates its closed-form unit-step response
    /// at `elapsed` and does not read `duration`.
    #[must_use]
    pub fn fraction(self, elapsed: f64, duration: f64) -> f64 {
        if let Curve::Spring { response, damping } = self {
            spring_step(response, damping, elapsed)
        } else {
            let t = if duration <= 0.0 {
                1.0
            } else {
                (elapsed / duration).clamp(0.0, 1.0)
            };
            self.ease(t)
        }
    }

    /// Eased progress for normalized `t` in `0.0..=1.0` (springs pass through; use [`Self::fraction`]).
    #[inline]
    #[must_use]
    pub fn ease(self, t: f64) -> f64 {
        match self {
            Curve::Linear | Curve::Spring { .. } => t,
            Curve::EaseIn => t * t,
            Curve::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Curve::EaseInOut => t * t * (3.0 - 2.0 * t), // smoothstep
        }
    }

    /// Whether the transition has settled, so the canvas frame clock can stop ticking it.
    #[must_use]
    pub fn is_settled(self, elapsed: f64, duration: f64) -> bool {
        match self {
            Curve::Spring { .. } => elapsed >= self.settle_secs().unwrap_or(0.0),
            _ => elapsed >= duration.max(0.0),
        }
    }

    /// How long a spring takes to settle, in seconds: the time after which it stays within a
    /// thousandth of its travel from the target. `None` for the easing curves, whose length is
    /// the spec's `duration_ms`. [`AnimSpec::spring`] uses it as the cycle's duration, so a
    /// spring's cycle ends where the eye can no longer see it move. Capped at thirty periods
    /// for a spring with next to no damping.
    #[must_use]
    pub fn settle_secs(self) -> Option<f64> {
        let Curve::Spring { response, damping } = self else {
            return None;
        };
        if response <= 0.0 {
            return Some(0.0);
        }
        let omega0 = std::f64::consts::TAU / response;
        let zeta = damping.max(0.0);
        let rest = (1.0 / SPRING_REST).ln();
        let secs = if (zeta - 1.0).abs() < 1e-3 {
            // (1 + ω₀t)·e^(-ω₀t) = 0.001 at ω₀t ≈ 9.233.
            9.233 / omega0
        } else if zeta < 1.0 {
            // The envelope e^(-ζω₀t) / √(1 - ζ²) falls below the rest distance.
            let root = (1.0 - zeta * zeta).sqrt();
            (rest - root.ln()) / (zeta.max(1e-3) * omega0)
        } else {
            // The slow root dominates: r₂/(r₂ - r₁)·e^(r₁t).
            let root = (zeta * zeta - 1.0).sqrt();
            let (slow, fast) = (omega0 * (zeta - root), omega0 * (zeta + root));
            (rest + (fast / (fast - slow)).ln()) / slow
        };
        Some(secs.min(response * 30.0))
    }

    /// A spring's physical constants for a mass of 1, as `(stiffness, damping)`: what
    /// `CASpringAnimation`, `SpringAnimation`/`SpringForce`, `SpringScalarNaturalMotionAnimation`
    /// and `day::motion::Spring` take. `stiffness = (2π / response)²` and the damping
    /// coefficient is `2 · ratio · 2π / response`. `None` for the easing curves.
    #[must_use]
    pub fn spring_constants(self) -> Option<(f64, f64)> {
        let Curve::Spring { response, damping } = self else {
            return None;
        };
        let omega0 = std::f64::consts::TAU / response.max(1e-6);
        Some((omega0 * omega0, 2.0 * damping.max(0.0) * omega0))
    }
}

/// Unit-step response of a second-order spring (`response` = period seconds, `damping` = ratio),
/// evaluated at `t` seconds. Under-damped rings and overshoots; critically damped and
/// over-damped approach without crossing.
fn spring_step(response: f64, damping: f64, t: f64) -> f64 {
    if response <= 0.0 || t <= 0.0 {
        return if t <= 0.0 { 0.0 } else { 1.0 };
    }
    let omega0 = std::f64::consts::TAU / response;
    let zeta = damping.max(0.0);
    if (zeta - 1.0).abs() < 1e-3 {
        let e = (-omega0 * t).exp();
        1.0 - e * (1.0 + omega0 * t)
    } else if zeta < 1.0 {
        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        let e = (-zeta * omega0 * t).exp();
        1.0 - e * ((omega_d * t).cos() + (zeta * omega0 / omega_d) * (omega_d * t).sin())
    } else {
        let root = (zeta * zeta - 1.0).sqrt();
        let (slow, fast) = (-omega0 * (zeta - root), -omega0 * (zeta + root));
        1.0 - (fast * (slow * t).exp() - slow * (fast * t).exp()) / (fast - slow)
    }
}

/// Where an [`AnimSpec`] is at one moment: what [`AnimSpec::sample`] answers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimSample {
    /// `0` at the start value, `1` at the target. A spring may pass `1` on the way. Once
    /// `done`, it is where the last cycle ended: `1`, or `0` when an autoreversed run ended on
    /// a backward cycle.
    pub progress: f64,
    /// Whether the run is over. Always `false` for a spec that repeats forever.
    pub done: bool,
}

/// One point of a cycle's curve: what [`AnimSpec::keyframes`] yields.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Keyframe {
    /// When, as a fraction of the cycle: `0.0` at its start, `1.0` at its end.
    pub offset: f64,
    /// The progress there: `0` is the start value, `1` the target. Outside `0..=1` where a
    /// spring overshoots.
    pub progress: f64,
}

/// Animation intent (§8.4): how a change to a native property is to be played. Threaded through
/// `Toolkit::update`/`set_frame`/`set_opacity`/`set_transform`; the toolkit plays it on its own
/// animator, and the frame-driven path (`day::motion`) samples the same function.
///
/// # The contract every toolkit keeps
///
/// A change from the value on screen (the *start*) to the new value (the *target*) plays as
/// follows, and [`AnimSpec::sample`] is that timeline as a function, so a toolkit with no
/// matching native feature drives the property from it and a test compares against it.
///
/// 1. **Delay.** For `delay_ms` the property keeps showing the start value. The delay is
///    served once, before the first cycle, never between repeats.
/// 2. **Cycle.** One cycle lasts `duration_ms` and follows `curve` from the start to the
///    target. A zero duration applies the target at once (after the delay).
/// 3. **Repeat.** The cycle plays `repeat + 1` times; `u32::MAX` repeats until the property
///    is next changed. Without `autoreverse` every cycle starts again from the start value.
/// 4. **Autoreverse.** With `autoreverse` every second cycle (the 2nd, 4th, …) plays the curve
///    backwards, from the target to the start.
/// 5. **Rest.** When a finite run ends the property rests on the target, whatever the last
///    cycle did. A run that ends on a backward cycle (autoreverse with an odd `repeat`) is
///    therefore followed by a jump to the target; use an even `repeat` to end on a forward one.
/// 6. **Spring.** A `Curve::Spring` cycle is the spring's closed-form response
///    ([`Curve::fraction`]) for `duration_ms`, which [`AnimSpec::spring`] sets to the spring's
///    settle time, and it ends exactly on the cycle's end value.
/// 7. **Retarget.** A new change to a property in flight starts from the value on screen. A
///    change with no animation cancels the run and applies at once.
///
/// What animates: frame, opacity, transform, and a container's background color
/// (`ContainerPatch::Background`), whose in-between colors lie between the two ends in every
/// channel.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimSpec {
    /// The length of one cycle, in milliseconds.
    pub duration_ms: u32,
    /// How long the start value is held before the first cycle, in milliseconds.
    pub delay_ms: u32,
    /// The shape of one cycle.
    pub curve: Curve,
    /// Cycles beyond the first (`0` = play once); `u32::MAX` = repeat forever.
    pub repeat: u32,
    /// Whether every second cycle plays backwards.
    pub autoreverse: bool,
}

impl Default for AnimSpec {
    /// The default feel: a smooth spring (SwiftUI's modern default).
    fn default() -> Self {
        AnimSpec::spring(0.4, 0.8)
    }
}

impl AnimSpec {
    /// How many keyframes [`AnimSpec::keyframes`] yields per second of cycle.
    pub const KEYFRAMES_PER_SECOND: f64 = 60.0;

    /// A spring: `response` is the period of its undamped oscillation in seconds, `damping`
    /// its damping ratio (`1.0` = no bounce, below `1.0` bouncy). `duration_ms` is the spring's
    /// settle time ([`Curve::settle_secs`]), so the cycle ends once the motion is too small to
    /// see and a repeat or an autoreverse has a definite length.
    #[must_use]
    pub fn spring(response: f64, damping: f64) -> Self {
        let curve = Curve::Spring { response, damping };
        AnimSpec {
            duration_ms: (curve.settle_secs().unwrap_or(0.0) * 1000.0)
                .ceil()
                .max(50.0) as u32,
            delay_ms: 0,
            curve,
            repeat: 0,
            autoreverse: false,
        }
    }
    /// Constant speed over `duration_ms`.
    #[must_use]
    pub fn linear(duration_ms: u32) -> Self {
        Self::timed(duration_ms, Curve::Linear)
    }
    /// Slow at the start, over `duration_ms`.
    #[must_use]
    pub fn ease_in(duration_ms: u32) -> Self {
        Self::timed(duration_ms, Curve::EaseIn)
    }
    /// Slow at the end, over `duration_ms`.
    #[must_use]
    pub fn ease_out(duration_ms: u32) -> Self {
        Self::timed(duration_ms, Curve::EaseOut)
    }
    /// Slow at both ends, over `duration_ms`.
    #[must_use]
    pub fn ease_in_out(duration_ms: u32) -> Self {
        Self::timed(duration_ms, Curve::EaseInOut)
    }
    fn timed(duration_ms: u32, curve: Curve) -> Self {
        AnimSpec {
            duration_ms,
            delay_ms: 0,
            curve,
            repeat: 0,
            autoreverse: false,
        }
    }
    /// Delay before the animation starts (builder).
    #[must_use]
    pub fn delay(mut self, ms: u32) -> Self {
        self.delay_ms = ms;
        self
    }
    /// Repeat `count` extra times (builder); `autoreverse` ping-pongs each cycle.
    #[must_use]
    pub fn repeat(mut self, count: u32, autoreverse: bool) -> Self {
        self.repeat = count;
        self.autoreverse = autoreverse;
        self
    }
    /// Repeat forever (builder), e.g. a pulsing indicator.
    #[must_use]
    pub fn repeat_forever(mut self, autoreverse: bool) -> Self {
        self.repeat = u32::MAX;
        self.autoreverse = autoreverse;
        self
    }
    /// One cycle, in seconds.
    #[inline]
    #[must_use]
    pub fn duration_secs(&self) -> f64 {
        f64::from(self.duration_ms) / 1000.0
    }
    /// The delay, in seconds.
    #[inline]
    #[must_use]
    pub fn delay_secs(&self) -> f64 {
        f64::from(self.delay_ms) / 1000.0
    }
    /// Whether the spec repeats until the property is next changed.
    #[inline]
    #[must_use]
    pub fn repeats_forever(&self) -> bool {
        self.repeat == u32::MAX
    }
    /// How many cycles play, or `None` for a spec that repeats forever.
    #[must_use]
    pub fn cycles(&self) -> Option<u32> {
        (!self.repeats_forever()).then(|| self.repeat + 1)
    }
    /// The whole run including the delay, in seconds, or `None` for a spec that repeats forever.
    #[must_use]
    pub fn total_secs(&self) -> Option<f64> {
        self.cycles()
            .map(|cycles| self.delay_secs() + self.duration_secs() * f64::from(cycles))
    }
    /// Whether a finite run's last cycle plays backwards, ending on the start value before the
    /// property rests on its target (contract rule 5).
    #[must_use]
    pub fn ends_reversed(&self) -> bool {
        self.autoreverse && self.cycles().is_some_and(|cycles| cycles.is_multiple_of(2))
    }

    /// Where the run is `elapsed` seconds after the change was made: the contract above as a
    /// function. During the delay the progress is `0`; inside a cycle it is the curve, read
    /// backwards on an autoreversed cycle; after the last cycle `done` is set and the progress
    /// is where that cycle ended. The property itself rests on its target once `done`.
    #[must_use]
    pub fn sample(&self, elapsed: f64) -> AnimSample {
        let t = elapsed - self.delay_secs();
        if t < 0.0 || !t.is_finite() {
            return AnimSample {
                progress: 0.0,
                done: false,
            };
        }
        let one = self.duration_secs();
        if one <= 0.0 {
            return AnimSample {
                progress: 1.0,
                done: true,
            };
        }
        let index = (t / one).floor();
        if let Some(cycles) = self.cycles()
            && index >= f64::from(cycles)
        {
            return AnimSample {
                progress: if self.ends_reversed() { 0.0 } else { 1.0 },
                done: true,
            };
        }
        let mut local = t - index * one;
        if self.autoreverse && (index as u64) % 2 == 1 {
            local = one - local;
        }
        AnimSample {
            progress: self.cycle_progress(local / one),
            done: false,
        }
    }

    /// The progress at `offset` (`0.0..=1.0`) of one forward cycle. The cycle's last point is
    /// exactly `1`, so a spring ends on its target instead of a residue away from it.
    #[must_use]
    pub fn cycle_progress(&self, offset: f64) -> f64 {
        if offset >= 1.0 {
            return 1.0;
        }
        let one = self.duration_secs();
        self.curve.fraction(offset.max(0.0) * one, one)
    }

    /// One forward cycle as evenly spaced keyframes, [`Self::KEYFRAMES_PER_SECOND`] of them per
    /// second of cycle and never fewer than two: the first at offset 0 and progress 0, the last
    /// at offset 1 and progress exactly 1. A toolkit without a spring animator plays these
    /// through its keyframe animation or its custom interpolator with linear interpolation
    /// between neighbors, which makes every toolkit draw the same spring. They serve the easing
    /// curves equally, for a toolkit whose native curve of that name has another shape.
    #[must_use]
    pub fn keyframes(&self) -> Vec<Keyframe> {
        let steps = ((self.duration_secs() * Self::KEYFRAMES_PER_SECOND).ceil() as usize).max(1);
        (0..=steps)
            .map(|i| {
                let offset = i as f64 / steps as f64;
                Keyframe {
                    offset,
                    progress: self.cycle_progress(offset),
                }
            })
            .collect()
    }
}

/// What a widget shows right now, an animation's in-between state included: what
/// [`Toolkit::read_displayed`] answers and the `assert_displayed` step checks (docs/testing.md).
///
/// [`NativeSnapshot`] reports the state a widget was given; this reports what is on screen
/// while the toolkit's animator is still on its way there (Core Animation's presentation
/// layer, a `View`'s animated properties, a computed style). Each field is read on its own and
/// `None` means "not read": the toolkit cannot observe that property mid-flight.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Displayed {
    /// The opacity shown, `0.0..=1.0`.
    pub opacity: Option<f64>,
    /// The frame shown, in points, in the space [`NativeSnapshot::frame`] uses, before any
    /// transform.
    pub frame: Option<Rect>,
    /// The translation shown, in points: the transform's `tx`/`ty`. A toolkit that animates a
    /// transform animates its translation, scale and rotation as one, so this one component
    /// stands for the channel.
    pub translation: Option<Point>,
    /// The background color shown, for a container with one.
    pub background: Option<Color>,
}

#[cfg(test)]
mod anim_tests {
    use super::*;

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

    #[test]
    fn a_linear_run_lasts_its_duration_then_is_done() {
        let spec = AnimSpec::linear(200);
        assert!(near(spec.sample(0.0).progress, 0.0));
        assert!(near(spec.sample(0.1).progress, 0.5));
        assert!(!spec.sample(0.199).done);
        let end = spec.sample(0.2);
        assert!(end.done && near(end.progress, 1.0));
        assert_eq!(spec.total_secs(), Some(0.2));
        assert_eq!(spec.cycles(), Some(1));
    }

    #[test]
    fn the_delay_is_served_once_and_cycles_repeat_or_reverse() {
        let restart = AnimSpec::linear(100).delay(100).repeat(2, false);
        assert!(near(restart.sample(0.05).progress, 0.0));
        assert!(near(restart.sample(0.15).progress, 0.5));
        // The second and third cycles start from the beginning again, with no second delay.
        assert!(near(restart.sample(0.225).progress, 0.25));
        assert!(near(restart.sample(0.375).progress, 0.75));
        assert!(restart.sample(0.4).done && !restart.ends_reversed());
        assert!(near(restart.total_secs().unwrap(), 0.4));

        let back = AnimSpec::linear(100).delay(100).repeat(1, true);
        assert!(near(back.sample(0.15).progress, 0.5)); // forward
        assert!(near(back.sample(0.275).progress, 0.25)); // back
        let end = back.sample(0.31);
        assert!(end.done && near(end.progress, 0.0) && back.ends_reversed());
        assert!(!AnimSpec::linear(100).repeat(2, true).ends_reversed());

        let forever = AnimSpec::linear(100).repeat_forever(true);
        assert!(!forever.sample(1000.0).done);
        assert_eq!(forever.total_secs(), None);
        assert_eq!(forever.cycles(), None);

        let instant = AnimSpec::linear(0).delay(50);
        assert!(!instant.sample(0.01).done);
        assert!(instant.sample(0.05).done && near(instant.sample(0.05).progress, 1.0));
    }

    #[test]
    fn a_spring_cycle_is_the_closed_form_and_ends_on_its_target() {
        let spec = AnimSpec::spring(0.5, 0.4);
        let Curve::Spring { response, damping } = spec.curve else {
            panic!("a spring spec has a spring curve");
        };
        assert!(near(response, 0.5) && near(damping, 0.4));
        // The cycle lasts the settle time, well past the response.
        let settle = spec.curve.settle_secs().unwrap();
        assert!(settle > 1.0 && settle < 2.0, "{settle}");
        assert!((spec.duration_secs() - settle).abs() < 0.002);
        let peak = (1..100)
            .map(|i| spec.sample(f64::from(i) * 0.01).progress)
            .fold(0.0, f64::max);
        assert!(peak > 1.2, "an under-damped spring overshoots: {peak}");
        // Within a thousandth from the settle time on, and exactly 1 at the end.
        let late = spec.curve.fraction(settle, 0.0);
        assert!((late - 1.0).abs() <= 0.001_5, "{late}");
        let end = spec.sample(spec.duration_secs());
        assert!(end.done && near(end.progress, 1.0));

        // Critically damped and over-damped springs never pass the target.
        for ratio in [1.0, 1.6] {
            let curve = Curve::Spring {
                response: 0.4,
                damping: ratio,
            };
            assert!((1..200).all(|i| curve.fraction(f64::from(i) * 0.01, 0.0) <= 1.000_001));
            let settle = curve.settle_secs().unwrap();
            assert!(
                (curve.fraction(settle, 0.0) - 1.0).abs() <= 0.001_5,
                "{ratio}"
            );
            assert!(curve.is_settled(settle, 0.0) && !curve.is_settled(0.0, 0.0));
        }
        // The physical constants of a mass of 1.
        let (stiffness, damping) = spec.curve.spring_constants().unwrap();
        let omega0 = std::f64::consts::TAU / 0.5;
        assert!(near(stiffness, omega0 * omega0) && near(damping, 2.0 * 0.4 * omega0));
        assert_eq!(Curve::Linear.spring_constants(), None);
        assert_eq!(Curve::EaseIn.settle_secs(), None);
    }

    #[test]
    fn keyframes_sample_one_cycle_end_to_end() {
        let spring = AnimSpec::spring(0.4, 0.5);
        let frames = spring.keyframes();
        assert!(frames.len() >= 30, "{}", frames.len());
        let (first, last) = (frames[0], frames[frames.len() - 1]);
        assert!(near(first.offset, 0.0) && near(first.progress, 0.0));
        assert!(near(last.offset, 1.0) && near(last.progress, 1.0));
        assert!(frames.windows(2).all(|w| w[1].offset > w[0].offset));
        assert!(
            frames.iter().any(|k| k.progress > 1.05),
            "the overshoot is in the samples"
        );
        // Each keyframe is the timeline's own value at that moment.
        for k in &frames[1..frames.len() - 1] {
            let at = spring.sample(k.offset * spring.duration_secs()).progress;
            assert!((at - k.progress).abs() < 1e-6);
        }
        // An easing curve samples the same way; an instant spec is its two ends.
        let ease = AnimSpec::ease_in(100).keyframes();
        assert_eq!(ease.len(), 7);
        assert!(near(ease[3].progress, 0.25));
        assert_eq!(AnimSpec::linear(0).keyframes().len(), 2);
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Role {
    #[default]
    None,
    Button,
    Toggle,
    Slider,
    TextInput,
    Heading(u8),
    Image,
    Meter,
    Group,
    /// A hierarchical tree container (docs/tree.md): `role="tree"`, `AXOutline`.
    Tree,
    /// One row of a tree: `role="treeitem"`, with level/expanded state where the platform
    /// carries them (docs/tree.md).
    TreeItem,
    /// One tab in a strip of peers (`role="tab"`, `AXTabButton`): the composed document
    /// strip's title buttons. Pair it with [`A11yProps::selected`] for the current one.
    Tab,
    /// The strip holding tabs (`role="tablist"`, `AXTabGroup`).
    TabList,
}

impl Role {
    /// The a11y role a built-in piece kind reports natively, the audit's *expectation* when the
    /// user hasn't set an explicit `.role()`. Native controls already expose these, so Day records
    /// them for `a11y_audit` (§14.2) rather than overriding the widget; only canvas/custom pieces
    /// need Day to apply a role. Returns `None` for kinds with no inherent control role.
    #[must_use]
    pub fn for_kind(kind: PieceKind) -> Role {
        match kind {
            kinds::BUTTON => Role::Button,
            kinds::TOGGLE => Role::Toggle,
            kinds::SLIDER => Role::Slider,
            kinds::TEXT_FIELD => Role::TextInput,
            kinds::IMAGE => Role::Image,
            kinds::PROGRESS => Role::Meter,
            kinds::TREE => Role::Tree,
            _ => Role::None,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct A11yProps {
    pub label: Option<String>,
    pub hint: Option<String>,
    pub value: Option<String>,
    pub role: Role,
    pub identifier: Option<String>,
    pub hidden: bool,
    pub decorative: bool,
    /// Whether the element is the selected one among its peers (a tab, a row): the
    /// platform's selected state, read aloud as such. `None` leaves the widget's own answer.
    pub selected: Option<bool>,
}

impl A11yProps {
    /// Merge another set of annotations onto this one: any field `other` sets (a `Some`, a
    /// non-`None` role, or a `true` flag) overrides ours; unset fields are left intact. Lets a
    /// node accumulate its `.a11y()`, `.id()`, and piece defaults into one stored result, so
    /// each `set_a11y` re-applies the full picture and `a11y_audit` has the complete expectation.
    pub fn merge(&mut self, other: &A11yProps) {
        if other.label.is_some() {
            self.label.clone_from(&other.label);
        }
        if other.hint.is_some() {
            self.hint.clone_from(&other.hint);
        }
        if other.value.is_some() {
            self.value.clone_from(&other.value);
        }
        if other.role != Role::None {
            self.role = other.role;
        }
        if other.identifier.is_some() {
            self.identifier.clone_from(&other.identifier);
        }
        self.hidden |= other.hidden;
        self.decorative |= other.decorative;
        if other.selected.is_some() {
            self.selected = other.selected;
        }
    }

    /// The role to *expect* for a node of `kind` carrying these annotations: an explicit
    /// `.role()` wins, otherwise the kind's native default (`Role::for_kind`).
    #[must_use]
    pub fn resolved_role(&self, kind: PieceKind) -> Role {
        if self.role == Role::None {
            Role::for_kind(kind)
        } else {
            self.role
        }
    }
}

/// A widget's actual native state, read back from the platform by `Toolkit::read_native`
/// (docs/testing.md): what the widget itself shows and reports, independently of Day's tree,
/// so `a11y_audit` and `assert_native` (§14.2) can diff it against Day's expectation.
///
/// Two groups. The accessibility fields (`role`, `label`, `value`, `identifier`) are read
/// together and `found` says whether they were: `found = false` means the backend cannot read
/// its accessibility tree, and the audit skips the node. Every other field is read on its own,
/// and `None` means "not read": the backend cannot read it, or it does not apply to this kind.
/// A reader never reports a field it did not take from the widget, so a `None` is a recorded
/// gap rather than a pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NativeSnapshot {
    /// The accessibility fields below were read.
    pub found: bool,
    /// The native role mapped back to Day's `Role` (best-effort).
    pub role: Role,
    /// The accessibility label the platform reports.
    pub label: Option<String>,
    /// The accessibility value the platform reports.
    pub value: Option<String>,
    /// The automation identifier, which Day sets from `.id()`.
    pub identifier: Option<String>,
    /// The text the widget displays: a label's string, a button's title, a text field's or
    /// text area's contents, without markup or mnemonic markers. `None` for a secure field
    /// that masks it.
    pub text: Option<String>,
    /// A slider's or progress indicator's value, in the range Day gave it.
    pub number: Option<f64>,
    /// A toggle's state, or a button's selected state (`ButtonProps::selected`).
    pub checked: Option<bool>,
    /// Whether the widget takes input.
    pub enabled: Option<bool>,
    /// Whether the widget is shown: not hidden itself, nor by an ancestor.
    pub visible: Option<bool>,
    /// The widget's frame in points, relative to the window's content area, origin top left.
    pub frame: Option<Rect>,
}

// ---------------------------------------------------------------------------
// Canvas display list (§11): full op set lands with M8a; the types are v1.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect(Rect),
    RoundedRect(Rect, f64),
    Ellipse(Rect),
    /// Arc within `rect`'s inscribed ellipse; `start` from the +x axis and `sweep`, both
    /// positive clockwise on screen.
    Arc {
        rect: Rect,
        start: Angle,
        sweep: Angle,
    },
    Line(Point, Point),
    Polygon(Vec<Point>),
    /// An arbitrary path: curves, several contours, and a fill rule (docs/canvas.md).
    ///
    /// [`Shape::Polygon`] is the straight-line, single-contour special case, kept because it is
    /// what most drawing code wants and it encodes far more compactly.
    Path(Path),
}

impl Shape {
    /// A rectangle rounded corner by corner (docs/canvas.md "Rounded corners"), as the simplest
    /// shape that draws it: a plain [`Shape::Rect`] when every corner is square, a
    /// [`Shape::RoundedRect`] (each toolkit's own rounded rectangle) when the corners agree, and a
    /// path ([`rounded_rect_segs`]) otherwise. The radii are [fitted](CornerRadii::fitted) first.
    ///
    /// ```
    /// use day_spec::{CornerRadii, Rect, Shape};
    ///
    /// // A bar standing on its axis: rounded on top, square where it meets the baseline.
    /// let bar = Shape::rounded_rect(Rect::new(0.0, 0.0, 20.0, 80.0), CornerRadii::top(6.0));
    /// std::assert_matches!(bar, Shape::Path(_));
    /// ```
    pub fn rounded_rect(rect: Rect, radii: CornerRadii) -> Shape {
        let r = radii.fitted(rect.size);
        if r.is_zero() || rect.size.width <= 0.0 || rect.size.height <= 0.0 {
            Shape::Rect(rect)
        } else if r.is_uniform() {
            Shape::RoundedRect(rect, r.top_left)
        } else {
            Shape::Path(Path {
                segs: rounded_rect_segs(rect, r),
                rule: FillRule::NonZero,
            })
        }
    }

    /// The same shape moved by `(dx, dy)`.
    ///
    /// What a [`Stamp`] means by "the template translated by one point", and the one definition of
    /// it: a backend builds each copy of a batch with this rather than reaching for its own
    /// platform transform, so every toolkit stamps to the same coordinates.
    #[must_use]
    pub fn translated(&self, dx: f64, dy: f64) -> Shape {
        let r = |r: &Rect| {
            Rect::new(
                r.origin.x + dx,
                r.origin.y + dy,
                r.size.width,
                r.size.height,
            )
        };
        let p = |p: &Point| Point::new(p.x + dx, p.y + dy);
        match self {
            Shape::Rect(rect) => Shape::Rect(r(rect)),
            Shape::RoundedRect(rect, rad) => Shape::RoundedRect(r(rect), *rad),
            Shape::Ellipse(rect) => Shape::Ellipse(r(rect)),
            Shape::Arc { rect, start, sweep } => Shape::Arc {
                rect: r(rect),
                start: *start,
                sweep: *sweep,
            },
            Shape::Line(a, b) => Shape::Line(p(a), p(b)),
            Shape::Polygon(pts) => Shape::Polygon(pts.iter().map(p).collect()),
            Shape::Path(path) => Shape::Path(Path {
                segs: path
                    .segs
                    .iter()
                    .map(|seg| match seg {
                        PathSeg::Move(a) => PathSeg::Move(p(a)),
                        PathSeg::Line(a) => PathSeg::Line(p(a)),
                        PathSeg::Quad(c, a) => PathSeg::Quad(p(c), p(a)),
                        PathSeg::Cubic(c1, c2, a) => PathSeg::Cubic(p(c1), p(c2), p(a)),
                        PathSeg::Close => PathSeg::Close,
                    })
                    .collect(),
                rule: path.rule,
            }),
        }
    }

    /// The shape's bounding rectangle: the box gradient [`UnitPoint`]s resolve against.
    pub fn bounds(&self) -> Rect {
        match self {
            Shape::Rect(r) | Shape::RoundedRect(r, _) | Shape::Ellipse(r) => *r,
            Shape::Arc { rect, .. } => *rect,
            Shape::Line(a, b) => points_bounds(&[*a, *b]),
            Shape::Polygon(pts) => points_bounds(pts),
            Shape::Path(p) => p.bounds(),
        }
    }
}

/// One step of a [`Path`]. Points are absolute, in the canvas's coordinate space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PathSeg {
    /// Start a new contour at this point.
    Move(Point),
    Line(Point),
    /// Quadratic bezier: one control point, then the end point.
    Quad(Point, Point),
    /// Cubic bezier: two control points, then the end point.
    Cubic(Point, Point, Point),
    /// Close the current contour back to its `Move`.
    Close,
}

/// Which points count as inside when a path's contours overlap (docs/canvas.md).
///
/// This matters the moment a shape has a hole: the counter of an "o", a washer, a ring chart. It
/// is a property of the fill, so stroking ignores it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum FillRule {
    /// Direction-sensitive: a hole needs its contour wound the opposite way. Every 2-D API's
    /// default, and what a font's glyph outlines assume.
    #[default]
    NonZero,
    /// Parity: any contour inside another cuts a hole regardless of winding. What PDF's `f*`
    /// operator and SVG's `fill-rule: evenodd` mean.
    EvenOdd,
}

/// An arbitrary 2-D path: any number of contours, straight or curved, with a fill rule.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Path {
    pub segs: Vec<PathSeg>,
    pub rule: FillRule,
}

impl Path {
    /// A conservative bounding box: the hull of every point including control points, which
    /// contains the true curve bounds (a bezier stays inside its control hull). Used only to
    /// resolve gradient unit points, where being slightly generous costs nothing and computing
    /// exact curve extrema would not.
    pub fn bounds(&self) -> Rect {
        let mut pts: Vec<Point> = Vec::with_capacity(self.segs.len() * 3);
        for seg in &self.segs {
            match seg {
                PathSeg::Move(p) | PathSeg::Line(p) => pts.push(*p),
                PathSeg::Quad(c, p) => pts.extend_from_slice(&[*c, *p]),
                PathSeg::Cubic(c1, c2, p) => pts.extend_from_slice(&[*c1, *c2, *p]),
                PathSeg::Close => {}
            }
        }
        points_bounds(&pts)
    }
}

/// A radius for each corner of a rectangle, clockwise from the top left (docs/canvas.md "Rounded
/// corners"). The shape a bar in a chart wants (rounded at its value end, square against its
/// axis), a tab (rounded on top), or a card joined to its neighbor.
///
/// Radii are in points and never negative. [`CornerRadii::fitted`] shrinks them the way CSS
/// does when two radii on one side would overlap, which is what a caller animating a radius or
/// drawing a short bar needs to not think about.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct CornerRadii {
    pub top_left: f64,
    pub top_right: f64,
    pub bottom_right: f64,
    pub bottom_left: f64,
}

impl CornerRadii {
    /// The same radius on every corner: what [`Shape::RoundedRect`] draws.
    pub fn uniform(r: f64) -> Self {
        CornerRadii {
            top_left: r,
            top_right: r,
            bottom_right: r,
            bottom_left: r,
        }
    }
    /// Rounded along the top edge, square along the bottom: a bar standing on its axis.
    pub fn top(r: f64) -> Self {
        CornerRadii {
            top_left: r,
            top_right: r,
            ..Default::default()
        }
    }
    /// Rounded along the bottom edge: a bar hanging below its axis.
    pub fn bottom(r: f64) -> Self {
        CornerRadii {
            bottom_right: r,
            bottom_left: r,
            ..Default::default()
        }
    }
    /// Rounded along the left edge: a bar growing leftward from its axis.
    pub fn left(r: f64) -> Self {
        CornerRadii {
            top_left: r,
            bottom_left: r,
            ..Default::default()
        }
    }
    /// Rounded along the right edge: a bar growing rightward from its axis.
    pub fn right(r: f64) -> Self {
        CornerRadii {
            top_right: r,
            bottom_right: r,
            ..Default::default()
        }
    }

    /// The four radii, clockwise from the top left.
    #[must_use]
    pub fn to_array(self) -> [f64; 4] {
        [
            self.top_left,
            self.top_right,
            self.bottom_right,
            self.bottom_left,
        ]
    }

    /// Whether every corner is square.
    #[must_use]
    pub fn is_zero(self) -> bool {
        self.to_array().iter().all(|r| *r <= 0.0)
    }

    /// Whether every corner has the same radius, so the rectangle is a [`Shape::RoundedRect`].
    #[must_use]
    #[expect(
        clippy::float_cmp,
        reason = "uniform means the four stored radii are exactly equal"
    )]
    pub fn is_uniform(self) -> bool {
        let [a, b, c, d] = self.to_array();
        a == b && b == c && c == d
    }

    /// These radii made to fit a `size` rectangle, the rule CSS uses for `border-radius`: a
    /// negative or non-finite radius is square, and when the two radii along any side add up to
    /// more than that side, every radius is scaled down by the same factor until none overlap.
    /// Scaling all four together keeps the corners in proportion, where clamping each one alone
    /// would turn a pill into a lopsided shape.
    #[must_use]
    pub fn fitted(self, size: Size) -> Self {
        let clean = |r: f64| if r.is_finite() && r > 0.0 { r } else { 0.0 };
        let [tl, tr, br, bl] = self.to_array().map(clean);
        let (w, h) = (size.width.max(0.0), size.height.max(0.0));
        let factor = [(w, tl + tr), (w, bl + br), (h, tl + bl), (h, tr + br)]
            .into_iter()
            .filter(|(_, sum)| *sum > 0.0)
            .map(|(side, sum)| side / sum)
            .fold(1.0f64, f64::min);
        CornerRadii {
            top_left: tl * factor,
            top_right: tr * factor,
            bottom_right: br * factor,
            bottom_left: bl * factor,
        }
    }
}

/// The point at `angle` round a circle: `0` = the +x axis, positive clockwise in device space
/// (y down), the convention [`Shape::Arc`] and every arc helper here use.
#[must_use]
pub fn arc_point(center: Point, radius: f64, angle: Angle) -> Point {
    let (sin, cos) = angle.sin_cos();
    Point::new(center.x + radius * cos, center.y + radius * sin)
}

/// A circular arc as cubic beziers: `sweep` of a circle of `radius` about `center`, from
/// `start` ([`arc_point`]'s convention). The segments start *from* the arc's first point, which
/// the caller reaches itself (a move or a line); nothing for a zero or non-finite radius or sweep.
///
/// The arc is split into quarter turns at most, where a cubic tracks a circle best: the radial
/// error peaks at 2.7 × 10⁻⁴ of the radius, a quarter of a pixel on a circle a thousand points
/// across (docs/canvas.md "Paths"). The same beziers on every backend mean the same pixels on
/// every backend, which an arc op interpreted by nine rasterizers would not.
#[must_use]
pub fn arc_cubics(center: Point, radius: f64, start: Angle, sweep: Angle) -> Vec<PathSeg> {
    let (start, sweep) = (start.radians(), sweep.radians());
    if !radius.is_finite() || radius <= 0.0 || !sweep.is_finite() || sweep == 0.0 {
        return Vec::new();
    }
    let at = |a: f64| Point::new(center.x + radius * a.cos(), center.y + radius * a.sin());
    let steps = (sweep.abs() / std::f64::consts::FRAC_PI_2).ceil() as usize;
    let delta = sweep / steps as f64;
    // The exact control-handle length for a circular arc of this sweep. Negative for a negative
    // sweep, which is what turns the handles around for a counter-clockwise arc.
    let k = 4.0 / 3.0 * (delta / 4.0).tan() * radius;
    (0..steps)
        .map(|i| {
            let a = start + delta * i as f64;
            let b = a + delta;
            let (p0, p1) = (at(a), at(b));
            PathSeg::Cubic(
                Point::new(p0.x - k * a.sin(), p0.y + k * a.cos()),
                Point::new(p1.x + k * b.sin(), p1.y - k * b.cos()),
                p1,
            )
        })
        .collect()
}

/// The closed contour of a rectangle rounded corner by corner, clockwise from the top left, with
/// the radii [fitted](CornerRadii::fitted) first. A square corner is a sharp point, not a
/// zero-length curve.
#[must_use]
pub fn rounded_rect_segs(rect: Rect, radii: CornerRadii) -> Vec<PathSeg> {
    let r = radii.fitted(rect.size);
    let (x0, y0) = (rect.origin.x, rect.origin.y);
    let (x1, y1) = (x0 + rect.size.width, y0 + rect.size.height);
    // Each corner: the point where its curve starts, then the quarter turn round its center.
    let corners = [
        (
            Point::new(x0 + r.top_left, y0 + r.top_left),
            r.top_left,
            Angle::HALF,
        ),
        (
            Point::new(x1 - r.top_right, y0 + r.top_right),
            r.top_right,
            Angle::deg(270.0),
        ),
        (
            Point::new(x1 - r.bottom_right, y1 - r.bottom_right),
            r.bottom_right,
            Angle::ZERO,
        ),
        (
            Point::new(x0 + r.bottom_left, y1 - r.bottom_left),
            r.bottom_left,
            Angle::QUARTER,
        ),
    ];
    let mut segs = Vec::with_capacity(14);
    for (i, (center, radius, start)) in corners.into_iter().enumerate() {
        let first = arc_point(center, radius, start);
        segs.push(if i == 0 {
            PathSeg::Move(first)
        } else {
            PathSeg::Line(first)
        });
        segs.extend(arc_cubics(center, radius, start, Angle::QUARTER));
    }
    segs.push(PathSeg::Close);
    segs
}

/// How a stroked line ends (docs/canvas.md). Names match PDF's `J`, SVG's `stroke-linecap`, and
/// every native 2-D API.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineCap {
    #[default]
    Butt,
    Round,
    Square,
}

/// How two stroked segments meet (docs/canvas.md). Matches PDF's `j` and SVG's
/// `stroke-linejoin`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum LineJoin {
    #[default]
    Miter,
    Round,
    Bevel,
}

/// Everything about a stroke except its paint.
///
/// [`StrokeStyle::width`] and `From<f64>` keep the common "just a width" case a single
/// argument, so plain `stroke(shape, color, 2.0)` call sites read the same as they always did.
#[derive(Clone, Debug, PartialEq)]
pub struct StrokeStyle {
    pub width: f64,
    pub cap: LineCap,
    pub join: LineJoin,
    /// Ignored unless `join` is [`LineJoin::Miter`]. The 2-D convention: past this ratio of miter
    /// length to line width the join falls back to a bevel.
    pub miter_limit: f64,
    /// On/off run lengths, repeating. Empty means solid.
    pub dash: Vec<f64>,
    /// How far into the dash pattern the line starts.
    pub dash_phase: f64,
}

impl Default for StrokeStyle {
    fn default() -> Self {
        // 10.0 is the miter limit PDF, PostScript, SVG and Cairo all default to.
        StrokeStyle {
            width: 1.0,
            cap: LineCap::Butt,
            join: LineJoin::Miter,
            miter_limit: 10.0,
            dash: Vec::new(),
            dash_phase: 0.0,
        }
    }
}

impl StrokeStyle {
    /// A solid stroke of `width`, everything else default.
    #[must_use]
    pub fn width(width: f64) -> Self {
        StrokeStyle {
            width,
            ..Default::default()
        }
    }
    /// A dashed stroke: `dash` is on/off run lengths, repeating.
    #[must_use]
    pub fn dashed(width: f64, dash: Vec<f64>) -> Self {
        StrokeStyle {
            width,
            dash,
            ..Default::default()
        }
    }
    /// Rounded ends and joins, the shape a data line usually wants.
    #[must_use]
    pub fn round(width: f64) -> Self {
        StrokeStyle {
            width,
            cap: LineCap::Round,
            join: LineJoin::Round,
            ..Default::default()
        }
    }
    /// Is this the plain width-only stroke every pre-existing call site asks for? Backends use
    /// it to skip setting dash/cap/join state they would only have to set back.
    #[must_use]
    #[expect(
        clippy::float_cmp,
        reason = "10.0 is the stored default miter limit, compared as a sentinel"
    )]
    pub fn is_plain(&self) -> bool {
        self.cap == LineCap::Butt
            && self.join == LineJoin::Miter
            && self.dash.is_empty()
            && self.miter_limit == 10.0
    }
}

impl From<f64> for StrokeStyle {
    fn from(width: f64) -> Self {
        StrokeStyle::width(width)
    }
}

fn points_bounds(pts: &[Point]) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
    for p in pts {
        x0 = x0.min(p.x);
        y0 = y0.min(p.y);
        x1 = x1.max(p.x);
        y1 = y1.max(p.y);
    }
    if pts.is_empty() {
        return Rect::ZERO;
    }
    Rect::new(x0, y0, x1 - x0, y1 - y0)
}

/// A point in the unit space of a shape's bounding box: (0,0) = top-leading, (1,1) =
/// bottom-trailing. Gradient geometry is expressed in unit points so one paint value works for
/// any shape size (docs/shapes.md §3.2).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UnitPoint {
    pub x: f64,
    pub y: f64,
}

impl UnitPoint {
    #[must_use]
    pub const fn new(x: f64, y: f64) -> Self {
        UnitPoint { x, y }
    }
    pub const TOP: UnitPoint = UnitPoint::new(0.5, 0.0);
    pub const BOTTOM: UnitPoint = UnitPoint::new(0.5, 1.0);
    pub const LEADING: UnitPoint = UnitPoint::new(0.0, 0.5);
    pub const TRAILING: UnitPoint = UnitPoint::new(1.0, 0.5);
    pub const TOP_LEADING: UnitPoint = UnitPoint::new(0.0, 0.0);
    pub const TOP_TRAILING: UnitPoint = UnitPoint::new(1.0, 0.0);
    pub const BOTTOM_LEADING: UnitPoint = UnitPoint::new(0.0, 1.0);
    pub const BOTTOM_TRAILING: UnitPoint = UnitPoint::new(1.0, 1.0);
    pub const CENTER: UnitPoint = UnitPoint::new(0.5, 0.5);

    /// Resolve to an absolute point within `rect`.
    pub fn resolve(&self, rect: Rect) -> Point {
        Point::new(
            rect.origin.x + self.x * rect.size.width,
            rect.origin.y + self.y * rect.size.height,
        )
    }
}

/// A linear gradient (docs/shapes.md §3.2 / §7): color stops along the line from `start` to
/// `end`, both in the unit space of the filled shape's bounding box. Stops are
/// `(offset 0..=1, ink)`, ascending; a stop takes any [`Ink`], so a gradient can follow the
/// appearance like a solid fill.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub start: UnitPoint,
    pub end: UnitPoint,
    pub stops: Vec<(f64, Ink)>,
}

/// Collect `(offset, color)` stops into the ink stops a gradient keeps.
fn ink_stops<C: Into<Ink>>(stops: impl IntoIterator<Item = (f64, C)>) -> Vec<(f64, Ink)> {
    stops.into_iter().map(|(at, c)| (at, c.into())).collect()
}

/// Every stop resolved against `palette`, as fixed inks.
fn resolve_stops(stops: &[(f64, Ink)], palette: &Palette) -> Vec<(f64, Ink)> {
    stops
        .iter()
        .map(|(at, ink)| (*at, Ink::Fixed(ink.resolve(palette))))
        .collect()
}

impl LinearGradient {
    #[must_use]
    pub fn new<C: Into<Ink>>(
        start: UnitPoint,
        end: UnitPoint,
        stops: impl IntoIterator<Item = (f64, C)>,
    ) -> Self {
        LinearGradient {
            start,
            end,
            stops: ink_stops(stops),
        }
    }
    /// Top-to-bottom between two colors: the everyday sky/backdrop case.
    #[must_use]
    pub fn vertical(top: impl Into<Ink>, bottom: impl Into<Ink>) -> Self {
        LinearGradient::new(
            UnitPoint::TOP,
            UnitPoint::BOTTOM,
            [(0.0, top.into()), (1.0, bottom.into())],
        )
    }
    /// Leading-to-trailing between two colors.
    #[must_use]
    pub fn horizontal(leading: impl Into<Ink>, trailing: impl Into<Ink>) -> Self {
        LinearGradient::new(
            UnitPoint::LEADING,
            UnitPoint::TRAILING,
            [(0.0, leading.into()), (1.0, trailing.into())],
        )
    }
}

/// A radial gradient (docs/shapes.md §3.2 / §7): color stops from `center` outward. Both the
/// center and the radius live in the unit space of the filled shape's bounding box, so the
/// gradient stretches into an ellipse when the bounds aren't square (the XAML relative-brush
/// behavior; the other backends reproduce it with a local matrix on a circular gradient). A
/// `radius` of `0.5` from the default center touches the edge midpoints of the bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    pub center: UnitPoint,
    pub radius: f64,
    pub stops: Vec<(f64, Ink)>,
}

impl RadialGradient {
    #[must_use]
    pub fn new<C: Into<Ink>>(
        center: UnitPoint,
        radius: f64,
        stops: impl IntoIterator<Item = (f64, C)>,
    ) -> Self {
        RadialGradient {
            center,
            radius,
            stops: ink_stops(stops),
        }
    }
    /// Centered, edge-touching (radius 0.5) between two colors: the everyday glow case.
    #[must_use]
    pub fn centered(inner: impl Into<Ink>, outer: impl Into<Ink>) -> Self {
        RadialGradient::new(
            UnitPoint::CENTER,
            0.5,
            [(0.0, inner.into()), (1.0, outer.into())],
        )
    }
}

/// A conic (sweep) gradient: color stops around `center`, the way a color wheel or a gauge's
/// ring is painted. The center is in the unit space of the filled shape's bounding box; the
/// angle is in canvas space, `0` = the +x axis and positive sweeping clockwise, the
/// convention [`Shape::Arc`] uses. Stops are `(offset 0..=1, ink)`, ascending, where the
/// offset is the fraction of a full turn from `start`.
///
/// The colors meet at `start`: for a seamless wheel make the first and last stops the same
/// color. Native on Core Graphics (where the OS has it), Android, Qt, `OH_Drawing` and the HTML
/// canvas; drawn as a fan of thin wedges on the backends with no such gradient, which reads the
/// same at any size a UI draws.
#[derive(Clone, Debug, PartialEq)]
pub struct ConicGradient {
    pub center: UnitPoint,
    pub start: Angle,
    pub stops: Vec<(f64, Ink)>,
}

impl ConicGradient {
    #[must_use]
    pub fn new<C: Into<Ink>>(
        center: UnitPoint,
        start: Angle,
        stops: impl IntoIterator<Item = (f64, C)>,
    ) -> Self {
        ConicGradient {
            center,
            start,
            stops: ink_stops(stops),
        }
    }
    /// Around the middle of the shape, starting at `start`.
    #[must_use]
    pub fn centered<C: Into<Ink>>(start: Angle, stops: impl IntoIterator<Item = (f64, C)>) -> Self {
        ConicGradient::new(UnitPoint::CENTER, start, stops)
    }
    /// The color at `angle` (canvas space, any value) in the current appearance: what a
    /// backend that draws the gradient as wedges fills each wedge with. Colors interpolate
    /// linearly between stops, channel by channel, and hold flat before the first stop and
    /// after the last.
    #[must_use]
    pub fn color_at(&self, angle: Angle) -> Color {
        let t = ((angle - self.start).degrees() / 360.0).rem_euclid(1.0);
        let Some(first) = self.stops.first() else {
            return Color::CLEAR;
        };
        if t <= first.0 {
            return first.1.resolved();
        }
        for pair in self.stops.windows(2) {
            let ((a_at, a), (b_at, b)) = (pair[0], pair[1]);
            let (a, b) = (a.resolved(), b.resolved());
            if t <= b_at {
                let k = if b_at > a_at {
                    (t - a_at) / (b_at - a_at)
                } else {
                    1.0
                };
                return Color::rgba(
                    a.r + (b.r - a.r) * k,
                    a.g + (b.g - a.g) * k,
                    a.b + (b.b - a.b) * k,
                    a.a + (b.a - a.a) * k,
                );
            }
        }
        self.stops.last().map_or(Color::CLEAR, |s| s.1.resolved())
    }
}

/// A drop shadow: the silhouette of what is drawn, blurred, tinted `color` and moved by
/// (`dx`, `dy`) points (docs/canvas.md "Shadows", docs/surfaces.md).
///
/// `blur` is the CSS `box-shadow` blur radius: the width of the soft edge, twice the standard
/// deviation of the Gaussian that makes it. `0` is a hard-edged copy. The offset and the blur
/// are in points and are not scaled or rotated by a canvas's current transform, the rule Core
/// Graphics and the HTML canvas share.
///
/// On a canvas ([`DrawOp::Shadow`]) every fill, stroke, text and image drawn while it is set
/// casts its own. On a surface ([`props::ContainerProps::shadow`]) it is the shadow of the
/// surface's rounded rectangle, drawn outside its frame without taking any room in layout.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shadow {
    pub color: Ink,
    pub blur: f64,
    pub dx: f64,
    pub dy: f64,
}

impl Shadow {
    /// A shadow directly under the shape: `color` feathered over `blur` points.
    #[must_use]
    pub fn new(color: impl Into<Ink>, blur: f64) -> Self {
        Shadow {
            color: color.into(),
            blur,
            dx: 0.0,
            dy: 0.0,
        }
    }
    /// Move the shadow by (`dx`, `dy`) points; positive `dy` is down.
    #[must_use]
    pub const fn offset(mut self, dx: f64, dy: f64) -> Self {
        self.dx = dx;
        self.dy = dy;
        self
    }
}

/// How what a canvas draws next combines with what it has drawn so far ([`DrawOp::Blend`],
/// docs/canvas.md "Blend modes").
///
/// The backdrop is the canvas's own drawing, never the views behind it: a canvas is its own
/// compositing group. Where nothing has been drawn yet the modes other than `Erase` leave the
/// source as it is.
///
/// The set is the one every 2-D context under Day has: the separable modes below are W3C
/// compositing's, and `Add` and `Erase` are Porter-Duff's "plus" and "destination-out".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BlendMode {
    /// Source over: the new drawing covers the old by its alpha.
    #[default]
    Normal,
    /// Multiplies the colors: always darker, white changes nothing. Ink on paper.
    Multiply,
    /// The inverse of multiply: always lighter, black changes nothing. Light on a screen.
    Screen,
    /// Multiplies dark backdrops and screens light ones: contrast.
    Overlay,
    /// The darker of the two, per channel.
    Darken,
    /// The lighter of the two, per channel.
    Lighten,
    /// Adds the colors, clamped: overlapping light sources.
    Add,
    /// Removes what has been drawn, by the new drawing's alpha, leaving the canvas transparent
    /// there so whatever is behind the canvas shows through: a hole punched in a ticket. The
    /// new drawing's color is ignored.
    Erase,
}

impl BlendMode {
    /// Every mode, in wire order.
    pub const ALL: [BlendMode; 8] = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Overlay,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Add,
        BlendMode::Erase,
    ];

    /// The mode as its wire number (slot `a` of [`OpCode::SetBlend`]): the position in
    /// [`BlendMode::ALL`].
    #[must_use]
    pub const fn wire(self) -> i32 {
        self as i32
    }
}

/// A line around a surface's edge ([`props::ContainerProps::border`]): `width` points of
/// `color`, drawn inside the frame, following its rounded corners, and over the surface's
/// content: a child that reaches the edge never covers the border (docs/surfaces.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Border {
    pub color: Ink,
    pub width: f64,
}

impl Border {
    #[must_use]
    pub fn new(color: impl Into<Ink>, width: f64) -> Self {
        Border {
            color: color.into(),
            width,
        }
    }
}

/// Which corners of a surface its corner radius rounds ([`props::ContainerProps::corners`]).
/// The others stay square: a sheet rounded on top, a tab, the first and last rows of a group.
///
/// Physical corners, like [`CornerRadii`]: `TOP_LEFT` is the top-left in both layout
/// directions. Combine with `|`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Corners(u8);

impl Corners {
    pub const NONE: Corners = Corners(0);
    pub const TOP_LEFT: Corners = Corners(1);
    pub const TOP_RIGHT: Corners = Corners(2);
    pub const BOTTOM_RIGHT: Corners = Corners(4);
    pub const BOTTOM_LEFT: Corners = Corners(8);
    pub const TOP: Corners = Corners(1 | 2);
    pub const BOTTOM: Corners = Corners(4 | 8);
    pub const LEFT: Corners = Corners(1 | 8);
    pub const RIGHT: Corners = Corners(2 | 4);
    pub const ALL: Corners = Corners(15);

    /// Whether every corner in `other` is in this set.
    #[must_use]
    pub const fn contains(self, other: Corners) -> bool {
        self.0 & other.0 == other.0
    }
    /// The set as four bits: 1 top-left, 2 top-right, 4 bottom-right, 8 bottom-left. What a
    /// backend hands a decoder in another language.
    #[must_use]
    pub const fn bits(self) -> u8 {
        self.0
    }
    /// `radius` on the corners in this set and `0` on the rest, as the per-corner radii a
    /// path-drawing backend wants.
    #[must_use]
    pub fn radii(self, radius: f64) -> CornerRadii {
        let on = |c: Corners| if self.contains(c) { radius } else { 0.0 };
        CornerRadii {
            top_left: on(Corners::TOP_LEFT),
            top_right: on(Corners::TOP_RIGHT),
            bottom_right: on(Corners::BOTTOM_RIGHT),
            bottom_left: on(Corners::BOTTOM_LEFT),
        }
    }
}

impl Default for Corners {
    /// Every corner: what a plain `corner_radius` rounds.
    fn default() -> Self {
        Corners::ALL
    }
}

impl std::ops::BitOr for Corners {
    type Output = Corners;
    fn bitor(self, rhs: Corners) -> Corners {
        Corners(self.0 | rhs.0)
    }
}

/// The payload of [`DrawOp::Stamp`]: one shape drawn at every position in `at`.
///
/// A scatter plot is the case this exists for. Drawn as individual ops, fifty thousand points are
/// fifty thousand `DrawOp`s, rebuilt, compared and cloned on every frame that re-records, before
/// a backend draws anything. As one `Stamp` they are one op and a flat array of coordinates, and
/// a backend can put every copy into one path and hand the rasterizer a single fill.
///
/// Every copy shares the shape, the size and the paint. Varying any of those means more than one
/// stamp: group by what varies, which for a chart is usually the series color.
#[derive(Clone, Debug, PartialEq)]
pub struct Stamp {
    /// The template, authored around the origin: each copy is this shape translated by one point.
    /// A 6-point dot is `Shape::Ellipse(Rect::new(-3.0, -3.0, 6.0, 6.0))`.
    pub shape: Shape,
    /// Where the copies go. Order is the drawing order, so later points paint over earlier ones.
    pub at: Vec<Point>,
    pub paint: Paint,
    /// `None` fills each copy; `Some` strokes it with this style.
    pub stroke: Option<StrokeStyle>,
}

/// What fills an area: a solid [`Ink`], or a linear, radial or conic gradient (docs/color.md).
/// One vocabulary for every fill: a shape's `.fill(..)`, a surface's `.fill(..)`,
/// `.background(..)` and a canvas's `d.fill(..)` all take `impl Into<Paint>`, so a
/// [`Color`], a [`SystemColor`], an [`Ink`] or a gradient goes into any of them.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Ink),
    Linear(LinearGradient),
    Radial(RadialGradient),
    Conic(ConicGradient),
}

impl Paint {
    /// The same paint with every ink resolved against `palette`: what a canvas records, so the
    /// display list a backend draws holds fixed colors only.
    #[must_use]
    pub fn resolve(&self, palette: &Palette) -> Paint {
        match self {
            Paint::Solid(ink) => Paint::Solid(Ink::Fixed(ink.resolve(palette))),
            Paint::Linear(g) => Paint::Linear(LinearGradient {
                stops: resolve_stops(&g.stops, palette),
                ..g.clone()
            }),
            Paint::Radial(g) => Paint::Radial(RadialGradient {
                stops: resolve_stops(&g.stops, palette),
                ..g.clone()
            }),
            Paint::Conic(g) => Paint::Conic(ConicGradient {
                stops: resolve_stops(&g.stops, palette),
                ..g.clone()
            }),
        }
    }

    /// Whether any ink in the paint depends on the appearance or the platform.
    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        let stops = |s: &[(f64, Ink)]| s.iter().any(|(_, ink)| ink.is_dynamic());
        match self {
            Paint::Solid(ink) => ink.is_dynamic(),
            Paint::Linear(g) => stops(&g.stops),
            Paint::Radial(g) => stops(&g.stops),
            Paint::Conic(g) => stops(&g.stops),
        }
    }
}

impl From<Ink> for Paint {
    fn from(ink: Ink) -> Self {
        Paint::Solid(ink)
    }
}

impl From<SystemColor> for Paint {
    fn from(role: SystemColor) -> Self {
        Paint::Solid(role.into())
    }
}

impl From<ConicGradient> for Paint {
    fn from(g: ConicGradient) -> Self {
        Paint::Conic(g)
    }
}

impl From<Color> for Paint {
    fn from(c: Color) -> Self {
        Paint::Solid(c.into())
    }
}

impl From<LinearGradient> for Paint {
    fn from(g: LinearGradient) -> Self {
        Paint::Linear(g)
    }
}

impl From<RadialGradient> for Paint {
    fn from(g: RadialGradient) -> Self {
        Paint::Radial(g)
    }
}

/// Where `at` sits vertically. `Top`, `Middle` and `Bottom` are edges of the line box (ascent +
/// descent at the font's size, the box [`Toolkit::measure_text`] reports); `Baseline` is the
/// typographic baseline itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextVAlign {
    /// `at` is the top of the line box.
    #[default]
    Top,
    /// `at` is the vertical middle of the line box.
    Middle,
    /// `at` is the baseline itself: what every native 2-D text call takes, and what a run of labels
    /// sharing a line should align on rather than their individual boxes.
    Baseline,
    /// `at` is the bottom of the line box.
    Bottom,
}

/// How canvas text hangs on its `at` point: one placement per axis (docs/canvas.md "Text").
///
/// Two axes rather than a list of named positions, because that is the shape of the problem and
/// of every native API underneath: `NSAttributedString` drawing, Pango extents,
/// `QFontMetricsF`, `Paint.Align`, `Canvas2D`'s `textAlign`/`textBaseline`. The alignment is done
/// by the backend, which is both cheaper and more accurate than doing it in app code: it already
/// has the laid-out metrics, where an app has to ask for them through
/// [`Toolkit::measure_text`] first.
///
/// [`TextAnchor::LEADING`] and [`TextAnchor::CENTERED`] name the two placements that used to be
/// this type's only variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct TextAnchor {
    /// The horizontal placement: the same three-way leading/center/trailing a wrapped label's
    /// lines take ([`props::TextAlign`]), because it is the same question with the same RTL rule.
    /// `Trailing` is what an axis label right-aligned against its axis wants, and the reason the
    /// horizontal axis is settable at all: emulating it in app code costs a
    /// [`Toolkit::measure_text`] call per label, where the backend already holds the width it is
    /// about to draw with.
    pub h: props::TextAlign,
    pub v: TextVAlign,
}

impl TextAnchor {
    /// The top-leading corner of the line box, the default.
    pub const LEADING: TextAnchor = TextAnchor {
        h: props::TextAlign::Leading,
        v: TextVAlign::Top,
    };
    /// The center of the line box, both ways.
    pub const CENTERED: TextAnchor = TextAnchor {
        h: props::TextAlign::Center,
        v: TextVAlign::Middle,
    };
    /// The top-trailing corner of the line box.
    pub const TRAILING: TextAnchor = TextAnchor {
        h: props::TextAlign::Trailing,
        v: TextVAlign::Top,
    };

    /// Where the text's top-leading corner goes, as an offset from `at`, given the line box this
    /// text measures to. The single definition of what an anchor means: every backend that draws
    /// in Rust calls this, and the C++/Java/JS shims repeat exactly this arithmetic.
    ///
    /// `height` is the line box (ascent + descent) and `ascent` the baseline's offset from its
    /// top, the two numbers [`TextMetrics`] carries.
    #[must_use]
    pub fn offset(self, width: f64, height: f64, ascent: f64) -> (f64, f64) {
        let dx = match self.h {
            props::TextAlign::Leading | props::TextAlign::Justified => 0.0,
            props::TextAlign::Center => -width / 2.0,
            props::TextAlign::Trailing => -width,
        };
        let dy = match self.v {
            TextVAlign::Top => 0.0,
            TextVAlign::Middle => -height / 2.0,
            TextVAlign::Baseline => -ascent,
            TextVAlign::Bottom => -height,
        };
        (dx, dy)
    }

    /// The anchor as one number, for the numeric op stream the serializing backends share:
    /// `h * 4 + v`, so it still occupies the single slot the two-variant enum did.
    #[must_use]
    pub fn pack(self) -> f64 {
        f64::from(self.h as u8 * 4 + self.v as u8)
    }

    /// Inverse of [`TextAnchor::pack`]. An unrecognized code answers the default rather than
    /// failing: a stream is only ever written by a matching encoder, and a drawing with text in
    /// the wrong corner beats one that panics inside a native draw callback.
    #[must_use]
    pub fn unpack(code: f64) -> TextAnchor {
        let c = code.round().clamp(0.0, 11.0) as u8;
        let h = match c / 4 {
            1 => props::TextAlign::Center,
            2 => props::TextAlign::Trailing,
            _ => props::TextAlign::Leading,
        };
        let v = match c % 4 {
            1 => TextVAlign::Middle,
            2 => TextVAlign::Baseline,
            3 => TextVAlign::Bottom,
            _ => TextVAlign::Top,
        };
        TextAnchor { h, v }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawOp {
    Fill(Shape, Paint),
    /// Stroke `shape`. The paint may be a gradient, and [`StrokeStyle`] carries width, dash, cap
    /// and join; `Draw::stroke` builds the plain width-only case.
    Stroke(Shape, Paint, StrokeStyle),
    /// One line of text. `size` is absolute canvas points (no accessibility scale: canvas
    /// text is a label inside a drawing, docs/canvas.md); `font` picks the family, weight and
    /// slant, and its default is the platform's own UI face.
    Text {
        text: String,
        at: Point,
        size: f64,
        color: Color,
        anchor: TextAnchor,
        font: CanvasFont,
    },
    /// Draw one shape at many positions, the batched form of [`DrawOp::Fill`]/[`DrawOp::Stroke`]
    /// (docs/canvas.md "Stamping").
    ///
    /// Boxed because the payload is wider than the largest other op, and a bigger `DrawOp` would
    /// cost every op in every drawing, the exact thing this variant exists to stop paying.
    Stamp(Box<Stamp>),
    /// Draw an image into `rect` (docs/images.md).
    ///
    /// The source is a [`BitmapId`] and never bytes: a canvas re-records on every tracked read,
    /// so carrying a buffer here would hand the backend a megabyte to compare, and re-decode, on
    /// every frame. The app decodes once with `day::decode_image` and draws the handle.
    ///
    /// `opacity` multiplies the image's own alpha. A backend that cannot draw a bitmap at all
    /// draws nothing rather than a placeholder rectangle, the same way an unknown op is skipped.
    ///
    /// `source` is the region of the bitmap to draw, in the bitmap's own PIXELS (not points:
    /// [`BitmapInfo::scale`] says how many pixels make a point), `None` meaning the whole
    /// image. It is what lets one decoded sprite sheet serve every icon on a map: each draw
    /// picks its cell and scales it into `rect`. A region reaching outside the bitmap is clamped
    /// to it, never an error: the part inside draws where the mapping into `rect` puts it, and
    /// an empty intersection draws nothing. A backend without a sub-rect draw clips to `rect`
    /// and draws the whole image scaled so the region lands there ([`clamp_image_source`] is
    /// the shared arithmetic).
    Image {
        image: BitmapId,
        rect: Rect,
        opacity: f64,
        source: Option<Rect>,
    },
    /// Intersect the clip with `shape`; everything drawn afterwards is confined to it.
    ///
    /// Scoped by [`DrawOp::Save`]/[`DrawOp::Restore`], which is the only way to widen a clip
    /// again; every native 2-D context works this way, so there is no "unclip".
    Clip(Shape),
    /// Set the shadow everything drawn afterwards casts, or `None` to stop casting one
    /// (docs/canvas.md "Shadows"). Graphics state like the clip: [`DrawOp::Restore`] puts back
    /// the shadow that was set at the matching [`DrawOp::Save`].
    ///
    /// A backend whose 2-D context has no shadow draws the shapes without one; it never draws
    /// a hard-edged stand-in.
    Shadow(Option<Shadow>),
    /// Set how everything drawn afterwards combines with what is already on the canvas
    /// (docs/canvas.md "Blend modes"). Graphics state, scoped by [`DrawOp::Save`] and
    /// [`DrawOp::Restore`] like the clip and the shadow.
    Blend(BlendMode),
    /// Push the current transform, clip, shadow and blend mode (§11, shapes). Backends map to
    /// save/restore of the native 2-D context; `Concat` multiplies an affine onto the CTM (shape
    /// rotate/scale/offset).
    Save,
    Restore,
    Concat(day_geometry::Affine),
}

// ---------------------------------------------------------------------------
// Built-in piece descriptors: full props (realize) + sparse patches (update).
// One binding = one attribute = one patch value: sparseness by construction (§8.1).
// ---------------------------------------------------------------------------

/// A semantic (logical) text style. Each maps to the platform's native text style where the
/// toolkit has one: `UIFont`/`NSFont.preferredFont(forTextStyle:)` on Apple (Dynamic Type), the
/// `*TextBlockStyle` resources on XAML. A Day app thus matches the OS's own typography and
/// inherits its accessibility text scaling with no extra work. Backends without semantic styles
/// (GTK/Qt/Android) approximate with sizes that still track the platform's text-scale /
/// font-scale accessibility setting.
///
/// The set mirrors SwiftUI `Font.TextStyle` (largest → smallest).
#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Font {
    LargeTitle,
    Title,
    Title2,
    Title3,
    Headline,
    Subheadline,
    #[default]
    Body,
    Callout,
    Footnote,
    Caption,
    Caption2,
    /// A custom point size. Backends scale it by the platform's accessibility text-scale (iOS via
    /// `UIFontMetrics`, Android via `sp`, GTK via text-scaling-factor) so it stays legible.
    System(f64),
    /// A bundled custom font by **family name**, at a point size (`Font::Custom("Pacifico",
    /// 24.0)`). The family must ship in the project's `fonts/` directory; `day build` stages the
    /// file into each platform's native font store and the backend registers it at startup
    /// (§18.4). The name is the family name baked into the font file (what Font Book /
    /// fontconfig report), not the file name. The size scales with the platform accessibility
    /// text-scale exactly like [`Font::System`]; weight/italic apply only where the family ships
    /// (or the platform synthesizes) such a face. An unknown family falls back to the system font
    /// of the same size, with a warning in the log.
    Custom(&'static str, f64),
}

impl Font {
    /// A bundled custom font by **typed family**, at a point size, the checked form of
    /// [`Font::Custom`]. Pass a generated `res::fonts::…` constant
    /// (`Font::custom(res::fonts::pacifico, 24.0)`), which exists only if the family ships in the
    /// project's `fonts/` directory, so the font is guaranteed bundled. For a family name known
    /// another way, the untyped [`Font::Custom`] variant is the escape hatch.
    #[must_use]
    pub const fn custom(family: FontFamily, size: f64) -> Font {
        Font::Custom(family.as_str(), size)
    }
}

/// Font weight, matching `UIFont.Weight` / SwiftUI `Font.Weight` (lightest → heaviest).
/// Ordered by heaviness, so backends can e.g. map `>= Semibold` to a synthesized bold face.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum FontWeight {
    UltraLight,
    Thin,
    Light,
    Regular,
    Medium,
    Semibold,
    Bold,
    Heavy,
    Black,
}

impl FontWeight {
    /// Every rung, lightest first.
    pub const ALL: [FontWeight; 9] = [
        FontWeight::UltraLight,
        FontWeight::Thin,
        FontWeight::Light,
        FontWeight::Regular,
        FontWeight::Medium,
        FontWeight::Semibold,
        FontWeight::Bold,
        FontWeight::Heavy,
        FontWeight::Black,
    ];

    /// The CSS / OpenType numeric weight (100 … 900), the one number every platform font API
    /// also speaks (`Typeface.create(base, weight, italic)`, `QFont::Weight`,
    /// `DWRITE_FONT_WEIGHT`, `OH_Drawing_FontWeight`), and what the canvas wire carries.
    #[must_use]
    pub const fn css(self) -> u16 {
        match self {
            FontWeight::UltraLight => 100,
            FontWeight::Thin => 200,
            FontWeight::Light => 300,
            FontWeight::Regular => 400,
            FontWeight::Medium => 500,
            FontWeight::Semibold => 600,
            FontWeight::Bold => 700,
            FontWeight::Heavy => 800,
            FontWeight::Black => 900,
        }
    }

    /// The nearest rung to a CSS weight (ties round heavier, like CSS font matching); anything
    /// outside 100 … 900 clamps.
    #[must_use]
    pub fn from_css(n: u16) -> FontWeight {
        let n = n.clamp(100, 900);
        let idx = (usize::from(n) + 50) / 100; // 100 → 1 … 900 → 9
        FontWeight::ALL[idx.clamp(1, 9) - 1]
    }

    /// The English face name for a rung ("Semibold"), for platforms whose enumeration names
    /// no face (see [`FontFace::synthesized_name`]).
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            FontWeight::UltraLight => "Ultra Light",
            FontWeight::Thin => "Thin",
            FontWeight::Light => "Light",
            FontWeight::Regular => "Regular",
            FontWeight::Medium => "Medium",
            FontWeight::Semibold => "Semibold",
            FontWeight::Bold => "Bold",
            FontWeight::Heavy => "Heavy",
            FontWeight::Black => "Black",
        }
    }
}

/// One face a font family ships (docs/fonts.md): its display name ("Bold Italic"), weight and
/// slant. What [`FontFamilyInfo::faces`] lists and what a font menu's style picker offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFace {
    pub name: String,
    pub weight: FontWeight,
    pub italic: bool,
}

impl FontFace {
    /// A display name from weight + slant ("Semibold Italic") for platforms whose enumeration
    /// API names no face (Android's `fonts.xml`, the CSS generic families).
    #[must_use]
    pub fn synthesized_name(weight: FontWeight, italic: bool) -> String {
        if italic {
            format!("{} Italic", weight.name())
        } else {
            weight.name().to_string()
        }
    }
}

/// A platform (or bundled) font family and the faces it can draw canvas text in
/// ([`Toolkit::font_families`], docs/fonts.md).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFamilyInfo {
    pub family: String,
    pub faces: Vec<FontFace>,
}

impl FontFamilyInfo {
    /// Whether any face is [`FontWeight::Bold`] or heavier.
    #[must_use]
    pub fn has_bold(&self) -> bool {
        self.faces.iter().any(|f| f.weight >= FontWeight::Bold)
    }
    /// Whether any face is italic.
    #[must_use]
    pub fn has_italic(&self) -> bool {
        self.faces.iter().any(|f| f.italic)
    }
    /// The nearest face: an exact slant match preferred, then the smallest weight distance
    /// (ties go heavier, like CSS font matching). `None` only when the family lists no faces.
    #[must_use]
    pub fn face_for(&self, weight: FontWeight, italic: bool) -> Option<&FontFace> {
        let want = i32::from(weight.css());
        self.faces.iter().min_by_key(|f| {
            let slant = i32::from(f.italic != italic);
            let dist = (i32::from(f.weight.css()) - want).abs();
            // Heavier wins a tie: subtracting the bool sorts the heavier face first.
            (slant, dist, i32::from(i32::from(f.weight.css()) < want))
        })
    }
}

/// Single-line text measurement in canvas points ([`Toolkit::measure_text`], docs/fonts.md).
///
/// Two boxes, and the difference between them matters. `width`/`height` is the **line box**: the
/// typographic slot the text occupies, the same for every string in a face at a size, and what
/// [`TextAnchor`] positions. [`ink`](TextMetrics::ink) is the **ink box**: where the glyphs
/// put marks. A line of digits leaves the descender space of its line box empty, so the
/// two disagree, and which one is wanted depends on the question: laying text out in a column
/// wants the line box, and centering it on a rule or keeping it clear of a mark wants the ink.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextMetrics {
    /// The advance width of the whole string.
    pub width: f64,
    /// The line box: ascent + descent at this size (plus the line gap where the engine reports
    /// one), which is the box [`TextAnchor`] positions.
    pub height: f64,
    /// The baseline's offset from the top of the line box.
    pub ascent: f64,
    /// The face's cap height at this size: baseline to the top of a capital.
    ///
    /// A property of the font, not of this string, reported here because it is what optical
    /// centering needs and asking for it separately would be a second measurement. Text made of
    /// capitals and digits looks centered on a rule when its cap box straddles it, not its line
    /// box: the line box reserves descender room that digits never use, so centering by it sits
    /// every label a little low. The cap middle is `ascent - cap_height / 2.0` below the line
    /// box's top.
    pub cap_height: f64,
    /// The ink box, the tight bounds of what is drawn, with its origin relative to the line
    /// box's top-leading corner, so `ink.origin` is where the marks start inside the slot.
    ///
    /// Empty (zero-size) for text that draws nothing, a space included. Where a toolkit cannot
    /// report ink it answers the whole line box, which is a superset: an overlap test against it
    /// can only be too cautious, never wrong.
    pub ink: Rect,
}

impl TextMetrics {
    /// The portable guess `day::measure_text` falls back to when a toolkit cannot measure:
    /// 0.6 × size per character, 1.2 × size tall, the baseline at 0.9 × size, caps at 0.7 × size.
    /// Its ink is the whole line box, the safe direction to be wrong in (see
    /// [`ink`](TextMetrics::ink)).
    #[must_use]
    pub fn approximate(text: &str, size: f64) -> TextMetrics {
        let (width, height) = (0.6 * size * text.chars().count() as f64, 1.2 * size);
        TextMetrics {
            width,
            height,
            ascent: 0.9 * size,
            cap_height: 0.7 * size,
            ink: Rect::new(0.0, 0.0, width, height),
        }
    }

    /// Read the eight-slot form the serializing backends (qt, xaml, arkui, web) fill across their
    /// C ABI: advance, line height, ascent, cap height, then the ink box relative to the line
    /// box's top-leading corner. One definition so four shims cannot drift from each other.
    #[must_use]
    pub fn from_slots(out: &[f64; 8]) -> TextMetrics {
        TextMetrics {
            width: out[0],
            height: out[1],
            ascent: out[2],
            cap_height: out[3],
            ink: Rect::new(out[4], out[5], out[6], out[7]),
        }
    }

    /// The line box's top-leading corner, given a baseline-relative measurement, the shape every
    /// backend's metrics come in. `ink_*` are relative to the drawing origin on the baseline
    /// (`ink_top` negative above it), the convention Skia, Core Text and Qt all report in.
    #[must_use]
    pub fn from_baseline(
        width: f64,
        ascent: f64,
        descent: f64,
        cap_height: f64,
        ink: Rect,
    ) -> TextMetrics {
        TextMetrics {
            width,
            height: ascent + descent,
            ascent,
            cap_height,
            // Baseline-relative to line-box-relative is one shift by the ascent.
            ink: Rect::new(
                ink.origin.x,
                ink.origin.y + ascent,
                ink.size.width,
                ink.size.height,
            ),
        }
    }
}

/// What canvas text is drawn with beyond its size (docs/canvas.md "Text"): a family, a weight and
/// a slant. The default is the platform's own UI face, regular, upright, which is what every
/// backend drew before the field existed, so a default font encodes to nothing on the wire.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct CanvasFont {
    /// A platform or bundled family name (as [`Toolkit::font_families`] lists it); `None` =
    /// the platform's default UI face.
    pub family: Option<String>,
    /// `None` = Regular.
    pub weight: Option<FontWeight>,
    pub italic: bool,
}

impl CanvasFont {
    /// The family name to send over the wire, empty for the default.
    #[must_use]
    pub fn family_str(&self) -> &str {
        self.family.as_deref().unwrap_or("")
    }
    /// The CSS weight to send over the wire, 0 for the default.
    pub fn css_weight(&self) -> u16 {
        self.weight.map_or(0, FontWeight::css)
    }
}

/// Field separator inside one family of the shim-side font list ([`parse_font_list`]).
pub const FONT_LIST_FIELD: char = '\u{1f}';
/// Family separator of the shim-side font list ([`parse_font_list`]).
pub const FONT_LIST_FAMILY: char = '\u{1e}';

/// Decode the text a non-Rust font enumerator hands back (the Qt, XAML and ArkUI shims, the
/// Android bridge, the web shim all produce this one format): families separated by U+001E;
/// inside a family, U+001F-separated fields: the family name, then one (name, CSS weight,
/// italic `0`/`1`) triple per face. Tolerant: a malformed face or family is skipped, an empty
/// face name is synthesized from its weight and slant, and a family with no faces is kept
/// with none (the caller decides what a face-less family means).
pub fn parse_font_list(s: &str) -> Vec<FontFamilyInfo> {
    let mut out = Vec::new();
    for fam in s.split(FONT_LIST_FAMILY) {
        let mut fields = fam.split(FONT_LIST_FIELD);
        let Some(family) = fields.next().map(str::trim).filter(|f| !f.is_empty()) else {
            continue;
        };
        let rest: Vec<&str> = fields.collect();
        let mut faces = Vec::new();
        for face in rest.chunks(3) {
            let [name, weight, italic] = face else {
                continue;
            };
            let Ok(weight) = weight.trim().parse::<u16>() else {
                continue;
            };
            let weight = FontWeight::from_css(weight);
            let italic = matches!(italic.trim(), "1" | "true");
            let name = name.trim();
            let name = if name.is_empty() {
                FontFace::synthesized_name(weight, italic)
            } else {
                name.to_string()
            };
            if !faces
                .iter()
                .any(|f: &FontFace| f.weight == weight && f.italic == italic)
            {
                faces.push(FontFace {
                    name,
                    weight,
                    italic,
                });
            }
        }
        out.push(FontFamilyInfo {
            family: family.to_string(),
            faces,
        });
    }
    out
}

/// The full font descriptor a label carries: a semantic (or custom) [`Font`] style plus an optional
/// weight override, an italic flag, and the tabular-figures request. Backends resolve this to one
/// native font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontSpec {
    pub style: Font,
    pub weight: Option<FontWeight>,
    pub italic: bool,
    /// Ask for tabular (monospaced) figures: every digit takes the same advance, so a number that
    /// changes stops changing width.
    ///
    /// In a proportional system font `1` is narrower than `8`, so a readout beside a slider
    /// wobbles as the value changes even when the digit count is fixed. Every platform ships this
    /// as a font feature, so Day asks for the platform's own rather than substituting a monospaced
    /// family: the text keeps the system face, and only the digits change metrics.
    ///
    /// Reserving space (`Decorate::reserving`) and tabular figures solve the two halves of the
    /// same problem (reservation stops the box resizing, tabular stops the glyphs shifting inside
    /// it), and a numeric readout usually wants both.
    ///
    /// This rides the existing label realize/patch path rather than adding a Toolkit method, so it
    /// is not a backend duty and has no duty-matrix row. Every in-tree backend honors it: AppKit
    /// and UIKit re-pick the system face (`monospacedDigitSystemFont`), GTK and Qt and ArkUI set
    /// the `tnum` OpenType feature, Android `setFontFeatureSettings`, XAML the `Typography`
    /// `NumeralAlignment` property, and web-dom `font-variant-numeric`. A font with no `tnum` table,
    /// or an SDK predating the attribute, renders the stock proportional figures.
    pub tabular: bool,
    /// Ask for the platform's monospaced face at this style's size, what `` `code` `` in a
    /// markdown run needs (docs/text-runs.md).
    ///
    /// A flag rather than a `Font::Monospace(pt)` variant so the run keeps its semantic style:
    /// code inside a `Footnote` paragraph must stay footnote-sized and keep tracking the
    /// reader's text-size setting, which a fixed point size would throw away.
    ///
    /// Like [`Self::tabular`], this rides the label realize/patch path and is not a backend duty:
    /// every toolkit has a system monospace face (`monospacedSystemFont`, Pango's `monospace`
    /// family, `QFontDatabase::FixedFont`, `Typeface.MONOSPACE`, CSS `monospace`).
    pub monospace: bool,
    /// Multiply the resolved size. `1.0` is the style's own size, `1.6` a heading against a body
    /// paragraph, `0.8` a caption inside one.
    ///
    /// Relative rather than an absolute point size, so that an editor's font-size control can
    /// move this while text a person reads for an hour still tracks the platform's
    /// accessibility text-scale. `Font::System(pt)` remains the absolute escape hatch, and it
    /// is where an imported document's `\fs28` or `font-size: 14px` lands (docs/texteditor.md):
    /// faithful to the file, and no longer responsive to the reader, which is the trade that
    /// form makes.
    ///
    /// Three toolkits take it directly (`GtkTextTag::scale`, Android's `RelativeSizeSpan`, CSS
    /// `em`); the rest multiply it into the size they already compute.
    pub scale: f64,
    /// Ask for small capitals: lowercase letters drawn as capitals at the x-height (the
    /// OpenType `smcp` feature). Capitals stay full size.
    ///
    /// Like [`Self::tabular`], this is a font feature that rides the label realize/patch path:
    /// a UIKit or AppKit font descriptor feature (`kLowerCaseType`/`kLowerCaseSmallCapsSelector`),
    /// Pango's `font-features` attribute, `QFont::setCapitalization(SmallCaps)`, Android's
    /// `setFontFeatureSettings("smcp")`, XAML's `Typography.Capitals`, ArkUI's font feature
    /// string, and CSS `font-variant-caps`. A face with no `smcp` table draws its ordinary
    /// lowercase; a toolkit never substitutes scaled-down capitals.
    pub small_caps: bool,
    /// Ask for oldstyle (lowercase, text) figures: digits with ascenders and descenders that sit
    /// in running text the way lowercase letters do (the OpenType `onum` feature). Combines
    /// with [`Self::tabular`]. A face with no `onum` table draws its ordinary lining figures.
    pub oldstyle_figures: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec {
            style: Font::Body,
            weight: None,
            italic: false,
            tabular: false,
            monospace: false,
            scale: 1.0,
            small_caps: false,
            oldstyle_figures: false,
        }
    }
}

impl From<Font> for FontSpec {
    fn from(style: Font) -> Self {
        FontSpec {
            style,
            ..FontSpec::default()
        }
    }
}

impl FontSpec {
    /// The plain descriptor for a style: no weight override, no italic, scale 1.0.
    pub fn new(style: Font) -> Self {
        FontSpec::from(style)
    }
    /// This descriptor at a different relative size (see [`FontSpec::scale`]).
    #[must_use]
    pub fn scaled(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }
    /// The point size this descriptor resolves to, given the size the platform assigns its
    /// semantic style. The one place `scale` is applied, so every backend's font resolver
    /// multiplies the same way.
    #[must_use]
    pub fn resolved_points(&self, style_points: f64) -> f64 {
        let base = match self.style {
            Font::System(pt) | Font::Custom(_, pt) => pt,
            _ => style_points,
        };
        (base * self.scale).max(1.0)
    }
    /// The OpenType feature tags this descriptor asks for, in a fixed order: `tnum` for
    /// [`Self::tabular`], `smcp` for [`Self::small_caps`], `onum` for
    /// [`Self::oldstyle_figures`]. One list, so every toolkit that takes a feature string
    /// (Pango, Android, ArkUI, CSS `font-feature-settings`) spells the same set.
    #[must_use]
    pub fn feature_tags(&self) -> Vec<&'static str> {
        let mut tags = Vec::new();
        if self.tabular {
            tags.push("tnum");
        }
        if self.small_caps {
            tags.push("smcp");
        }
        if self.oldstyle_figures {
            tags.push("onum");
        }
        tags
    }
}

/// One styled span of a label's text (docs/text-runs.md).
///
/// `range` is a byte range into the label's `text`. Bytes, not chars: every backend that takes a
/// structured attributed string wants UTF-16 or byte offsets, `str` slicing is byte-based, and a
/// char count would have to be converted at every boundary. Ranges must be non-overlapping and
/// ascending; a gap between them is unstyled text, drawn with the label's own font.
///
/// A run says only what differs. `font` is the whole descriptor rather than a delta because a
/// bold run inside a Body paragraph is `FontSpec { style: Body, weight: Some(Bold), .. }`. The
/// style has to travel with the weight or the run loses its semantic size.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub range: std::ops::Range<usize>,
    pub font: FontSpec,
    pub color: Option<Ink>,
    /// A highlight painted behind the glyphs. Every native text editor carries this attribute
    /// (docs/texteditor.md), and search hits and review comments are the two things an app is
    /// asked for the moment it has styled text at all.
    pub background: Option<Ink>,
    /// A line under the text: its style and, optionally, a color of its own.
    pub underline: Option<Decoration>,
    /// A line through the text. Separate from `FontSpec` because it is a decoration, not a face:
    /// no platform expresses it by picking a different font.
    pub strikethrough: Option<Decoration>,
    /// Makes this run a link to the given target. Rendering it is [`Cap::TextRuns`]; making it
    /// activatable is [`Cap::TextLinks`], which is a smaller set (see the docs).
    pub link: Option<String>,
}

impl Default for TextRun {
    fn default() -> Self {
        TextRun {
            range: 0..0,
            font: FontSpec::default(),
            color: None,
            background: None,
            underline: None,
            strikethrough: None,
            link: None,
        }
    }
}

impl TextRun {
    /// A run that only changes the font, the common case for emphasis.
    #[must_use]
    pub fn font(range: std::ops::Range<usize>, font: FontSpec) -> Self {
        TextRun {
            range,
            font,
            ..TextRun::default()
        }
    }

    /// This run's attributes without its range: what a selection's style is compared against
    /// and what an editor's toolbar toggles (docs/texteditor.md).
    #[must_use]
    pub fn style(&self) -> RunStyle {
        RunStyle {
            font: self.font,
            color: self.color.map(Ink::resolved),
            background: self.background.map(Ink::resolved),
            underline: self.underline,
            strikethrough: self.strikethrough,
            link: self.link.clone(),
        }
    }

    /// A run of `style` over `range`.
    #[must_use]
    pub fn styled(range: std::ops::Range<usize>, style: RunStyle) -> Self {
        TextRun {
            range,
            font: style.font,
            color: style.color.map(Ink::from),
            background: style.background.map(Ink::from),
            underline: style.underline,
            strikethrough: style.strikethrough,
            link: style.link,
        }
    }
}

/// Serialize a label's text + runs as markup, for the two backends whose text widgets take a
/// string in their own dialect rather than a structured attributed string: GTK (Pango markup)
/// and Qt (its HTML subset). Shared so the escaping is written and tested once: a translated
/// string containing `&` or `<` corrupts the whole label otherwise, and it will be a translated
/// string that finds the bug.
///
/// `dialect` decides the spelling: Pango wants `<span foreground="#rrggbb"
/// font_family="monospace">`, Qt wants `<span style="color:#rrggbb">` with `<code>`. Both accept
/// `<b>`, `<i>`, `<u>`, `<s>` and `<a href>`, which is why one function can serve them.
///
/// `base_points` is the size the label itself resolved to. Only a relative-size run
/// ([`FontSpec::scale`]) needs it, and only in the Qt dialect: Pango markup takes a percentage
/// directly, while Qt's CSS subset understands `pt` and `px` and ignores `%`.
#[must_use]
pub fn runs_to_markup(
    text: &str,
    runs: &[TextRun],
    dialect: MarkupDialect,
    base_points: f64,
) -> String {
    let mut out = String::with_capacity(text.len() + runs.len() * 24);
    let mut at = 0usize;
    for r in runs {
        // Skip a run whose range does not address this string, without advancing `at`: dropping
        // the run loses styling, but advancing past it would drop the rest of the sentence.
        // `runs_are_valid` rejects these upstream; this keeps the failure cheap if one slips by.
        let Some(styled) = text.get(r.range.clone()) else {
            continue;
        };
        if r.range.start > at
            && let Some(plain) = text.get(at..r.range.start)
        {
            escape_markup_in(plain, &mut out, dialect);
        }
        open_run(r, dialect, base_points, &mut out);
        escape_markup_in(styled, &mut out, dialect);
        close_run(r, dialect, &mut out);
        at = r.range.end;
    }
    if let Some(tail) = text.get(at..) {
        escape_markup_in(tail, &mut out, dialect);
    }
    out
}

impl Symbol {
    /// A 24×24 outline path for this symbol, Day's own drawing of it.
    ///
    /// The platform's icon comes first on every backend that has one (SF Symbols, the freedesktop
    /// icon theme, Fluent's glyph font). This is the fallback for when that lookup finds nothing,
    /// which is not an edge case: `view-filter-symbolic` exists on a GNOME desktop and nowhere
    /// else, so a GTK or Qt app run off that desktop drew a toolbar item with no icon at all.
    /// One table here means the same fallback shape everywhere instead of a gap per platform.
    ///
    /// `None` for a variant with no drawing yet: `Symbol` is `#[non_exhaustive]`, so a new one
    /// degrades to a label rather than failing to compile.
    #[must_use]
    pub fn outline_path(self) -> Option<&'static str> {
        use Symbol as S;
        // The wildcard is unreachable today (every variant is drawn below) and it is here for
        // the next one, which should degrade to a label rather than fail to compile.
        #[allow(unreachable_patterns)]
        Some(match self {
            S::Add => "M11 5h2v6h6v2h-6v6h-2v-6H5v-2h6z",
            S::Remove => "M5 11h14v2H5z",
            S::Delete => "M9 3h6l1 2h4v2H4V5h4zM6 8h12l-1 13H7z",
            S::Edit => {
                "M3 17.3V21h3.7L17.8 9.9l-3.7-3.7zM20.7 7a1 1 0 0 0 0-1.4l-2.3-2.3a1 1 0 0 0-1.4 0l-1.8 1.8 3.7 3.7z"
            }
            S::New => "M13 2H6v20h12V7h-5zm2 .5L19.5 7H15z",
            S::Open | S::Folder => "M2 5h7l2 2h11v12H2z",
            S::Save => "M3 3h13l5 5v13H3zm5 0h7v6H8zm-1 11h10v7H7z",
            S::Print => "M7 3h10v4H7zM4 8h16v8h-3v6H7v-6H4z",
            S::Refresh => "M12 4V1L8 5l4 4V6a6 6 0 1 1-6 6H4a8 8 0 1 0 8-8z",
            S::Search => {
                "M10 4a6 6 0 1 0 3.5 10.9l4.8 4.8 1.4-1.4-4.8-4.8A6 6 0 0 0 10 4m0 2a4 4 0 1 1 0 8 4 4 0 0 1 0-8"
            }
            S::Share => "M12 3l5 5h-3v7h-4V8H7zM5 17h14v4H5z",
            // An eight-tooth gear, round about the center (radius 10 at the teeth, 7.6 at the
            // roots) with a 3.2 hole: square in its box, so it sits in a row's 24-point icon
            // slot at the same size as its neighbors.
            S::Settings => {
                "M9.9 4.7L10.2 2.2L13.8 2.2L14.1 4.7A7.6 7.6 0 0 1 15.7 5.4L17.7 3.8L20.2 6.3L18.6 8.3A7.6 7.6 0 0 1 19.3 9.9L21.8 10.2L21.8 13.8L19.3 14.1A7.6 7.6 0 0 1 18.6 15.7L20.2 17.7L17.7 20.2L15.7 18.6A7.6 7.6 0 0 1 14.1 19.3L13.8 21.8L10.2 21.8L9.9 19.3A7.6 7.6 0 0 1 8.3 18.6L6.3 20.2L3.8 17.7L5.4 15.7A7.6 7.6 0 0 1 4.7 14.1L2.2 13.8L2.2 10.2L4.7 9.9A7.6 7.6 0 0 1 5.4 8.3L3.8 6.3L6.3 3.8L8.3 5.4A7.6 7.6 0 0 1 9.9 4.7zM12 8.8a3.2 3.2 0 1 0 0 6.4a3.2 3.2 0 1 0 0-6.4z"
            }
            S::Info => "M12 2a10 10 0 1 0 0 20 10 10 0 0 0 0-20m-1 5h2v2h-2zm0 4h2v6h-2z",
            S::Star => {
                "M12 2l2.9 6.3 6.9.8-5.1 4.7 1.4 6.8L12 17.3 5.9 20.6l1.4-6.8L2.2 9.1l6.9-.8z"
            }
            S::Bookmark => "M6 2h12v20l-6-4-6 4z",
            S::Back => "M15.4 5.4L14 4l-8 8 8 8 1.4-1.4L8.8 12z",
            S::Forward => "M8.6 5.4L10 4l8 8-8 8-1.4-1.4L15.2 12z",
            S::Up => "M5.4 15.4L4 14l8-8 8 8-1.4 1.4L12 8.8z",
            S::Down => "M5.4 8.6L4 10l8 8 8-8-1.4-1.4L12 15.2z",
            S::Home => "M12 3l9 8h-3v10h-5v-6h-2v6H6V11H3z",
            S::Sidebar => "M3 4h18v16H3zm2 2v12h4V6zm6 0v12h8V6z",
            S::Filter => "M3 5h18l-7 8v6l-4 2v-8z",
            S::Sort => "M8 4v12H4l4 4 4-4H10V4zM14 6h7v2h-7zm0 4h5v2h-5zm0 4h3v2h-3z",
            S::More => {
                "M6 10a2 2 0 1 0 0 4 2 2 0 0 0 0-4m6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4m6 0a2 2 0 1 0 0 4 2 2 0 0 0 0-4"
            }
            S::Play => "M8 5l11 7-11 7z",
            S::Pause => "M7 5h3v14H7zm7 0h3v14h-3z",
            S::Stop => "M6 6h12v12H6z",
            S::Camera => {
                "M9 3h6l1.5 2H20a2 2 0 0 1 2 2v12H2V7a2 2 0 0 1 2-2h3.5zm3 6a4.5 4.5 0 1 0 0 9 4.5 4.5 0 0 0 0-9m0 1.6a2.9 2.9 0 1 1 0 5.8 2.9 2.9 0 0 1 0-5.8"
            }
            S::Code => {
                "M9.4 16.6L4.8 12l4.6-4.6L8 6l-6 6 6 6zm5.2 0l4.6-4.6L14.6 7.4 16 6l6 6-6 6z"
            }
            S::Light => {
                "M12 7a5 5 0 1 0 0 10 5 5 0 0 0 0-10M11 1h2v3h-2zm0 19h2v3h-2zM1 11h3v2H1zm19 0h3v2h-3zM3.5 4.9l1.4-1.4 2.1 2.1-1.4 1.4zm13.5 13.5l1.4-1.4 2.1 2.1-1.4 1.4zM4.9 20.5l-1.4-1.4 2.1-2.1 1.4 1.4zm13.5-13.5l-1.4-1.4 2.1-2.1 1.4 1.4z"
            }
            S::Dark => "M12 3a9 9 0 1 0 9 9 7 7 0 0 1-9-9",
            // "Automatic": the half-filled circle every platform uses: the left half solid, the
            // right half an outline, which `evenodd` below turns into the ring it should be.
            S::Auto => "M12 2a10 10 0 0 0 0 20zM12 4a8 8 0 0 1 0 16v-1.6a6.4 6.4 0 0 0 0-12.8z",
            S::ZoomIn => {
                "M10 4a6 6 0 1 0 3.5 10.9l4.8 4.8 1.4-1.4-4.8-4.8A6 6 0 0 0 10 4M9 7h2v2h2v2h-2v2H9v-2H7V9h2z"
            }
            S::ZoomOut => {
                "M10 4a6 6 0 1 0 3.5 10.9l4.8 4.8 1.4-1.4-4.8-4.8A6 6 0 0 0 10 4M7 9h6v2H7z"
            }
            // The ZoomIn lens with a numeral 1 in it, "back to actual size".
            S::ZoomReset => {
                "M10 4a6 6 0 1 0 3.5 10.9l4.8 4.8 1.4-1.4-4.8-4.8A6 6 0 0 0 10 4M12 13h-1.7V9.3l-1.2.8-.9-1.3 2.4-1.8H12z"
            }
            S::Undo => "M12 5V2L7 7l5 5V9a5 5 0 1 1 0 10H8v2h4a7 7 0 0 0 0-14z",
            S::Redo => "M12 5V2l5 5-5 5V9a5 5 0 1 0 0 10h4v2h-4a7 7 0 0 1 0-14z",
            S::Copy => "M8 2h10v14H8zM4 6h2v14h12v2H4z",
            S::Cut => {
                "M9 4l5.5 9.5-1.2 2L7.8 6zM15 4L9.5 13.5l1.2 2L16.2 6zM6 16a3 3 0 1 0 0 6 3 3 0 0 0 0-6m12 0a3 3 0 1 0 0 6 3 3 0 0 0 0-6"
            }
            S::Paste => "M9 2h6v2h3v18H6V4h3zm0 2v2h6V4z",
            S::Mail => "M2 5h20v14H2zm2 3.2V17h16V8.2l-8 5z",
            S::Document => "M14 2H6v20h12V6zm.5.8L17.2 6H14.5z",
            S::Check => "M9 16.2L4.8 12l-1.4 1.4L9 19 21 7l-1.4-1.4z",
            S::Close => {
                "M19 6.4L17.6 5 12 10.6 6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12z"
            }
            S::Warning => "M12 2l10 19H2zm-1 7h2v6h-2zm0 8h2v2h-2z",
            // Shape outlines, stroked as a 2-unit band (an even-odd ring), so they read as
            // outlines at any size rather than filled blocks.
            S::Rectangle => "M3 5h18v14H3zm2 2v10h14V7z",
            S::Oval => {
                "M12 5c5 0 9 3.1 9 7s-4 7-9 7-9-3.1-9-7 4-7 9-7zm0 2c-3.9 0-7 2.2-7 5s3.1 5 7 5 7-2.2 7-5-3.1-5-7-5z"
            }
            // A circular ring (even-odd: outer disc minus inner) and its filled twin.
            S::Circle => "M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16zm0 2a6 6 0 1 1 0 12 6 6 0 0 1 0-12z",
            S::CircleFilled => "M12 4a8 8 0 1 0 0 16 8 8 0 0 0 0-16z",
            S::Line => "M4.7 17.9l13.2-13.2 1.4 1.4L6.1 19.3z",
            // Group: a solid square stacked on another (the visible L of the back one);
            // Ungroup: the same two squares pulled apart.
            S::Group => "M8 4h12v12h-4V8H8zM4 8h12v12H4z",
            S::Ungroup => "M3 3h8v8H3zM13 13h8v8h-8z",
            // A crosshair: a dot in a ring with four ticks, the even-odd fill leaving the gap
            // between dot and ring empty.
            S::Location => {
                "M12 8a4 4 0 1 0 0 8 4 4 0 0 0 0-8m8.9 3A9 9 0 0 0 13 3.1V1h-2v2.1A9 9 0 0 0 3.1 11H1v2h2.1a9 9 0 0 0 7.9 7.9V23h2v-2.1a9 9 0 0 0 7.9-7.9H23v-2zM12 19a7 7 0 1 1 0-14 7 7 0 0 1 0 14"
            }
            // `Symbol` is `#[non_exhaustive]`: a variant added upstream draws nothing here rather
            // than failing to compile, and its item keeps its label.
            _ => return None,
        })
    }

    /// [`Self::outline_path`] wrapped as a standalone SVG document.
    #[must_use]
    pub fn outline_svg(self) -> Option<String> {
        self.outline_path().map(|d| {
            format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">\
                 <path fill-rule=\"evenodd\" d=\"{d}\"/></svg>"
            )
        })
    }
}

/// Which markup dialect [`runs_to_markup`] should emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkupDialect {
    /// Pango markup (GTK).
    Pango,
    /// Qt's rich-text HTML subset.
    QtHtml,
}

/// XML-escape a run of plain text into `out`. `&` first, or the escapes escape each other.
///
/// Newlines are dialect-specific. Pango lays a literal `\n` out as a line break, but Qt's rich
/// text is HTML, where any run of whitespace collapses to one space, which silently ran a
/// two-paragraph body together into a single block. `<br>` is what says "break here" in HTML, so
/// the same string wraps the same way on both.
fn escape_markup_in(s: &str, out: &mut String, dialect: MarkupDialect) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            '\n' if dialect == MarkupDialect::QtHtml => out.push_str("<br>"),
            _ => out.push(c),
        }
    }
}

/// [`escape_markup_in`] for the URL slot, where a newline cannot appear.
fn escape_markup(s: &str, out: &mut String) {
    escape_markup_in(s, out, MarkupDialect::Pango);
}

fn hex(c: Color) -> String {
    format!(
        "#{:02x}{:02x}{:02x}",
        (c.r.clamp(0.0, 1.0) * 255.0) as u8,
        (c.g.clamp(0.0, 1.0) * 255.0) as u8,
        (c.b.clamp(0.0, 1.0) * 255.0) as u8
    )
}

/// Does this run need a `<span>` in `dialect`, or do the plain tags cover it?
#[expect(
    clippy::float_cmp,
    reason = "a scale of exactly 1.0 is the unscaled default, compared as a sentinel"
)]
fn needs_span(r: &TextRun, dialect: MarkupDialect) -> bool {
    let extras = r.color.is_some()
        || r.background.is_some()
        || r.font.scale != 1.0
        || !r.font.feature_tags().is_empty()
        || !plain_line(r.underline)
        || !plain_line(r.strikethrough);
    match dialect {
        // Pango expresses the monospace family as a span attribute; Qt cannot (see below).
        MarkupDialect::Pango => extras || r.font.monospace,
        MarkupDialect::QtHtml => extras,
    }
}

/// Is this decoration absent, or the one a bare `<u>`/`<s>` tag draws: a single line in the
/// text color? Anything else (a style, a color) has to travel as span attributes.
fn plain_line(d: Option<Decoration>) -> bool {
    d.is_none_or(|d| d == Decoration::SINGLE)
}

/// Pango's `underline` attribute for a style. Pango has no dotted line, so `Dotted` draws
/// single; its `error` underline is the squiggle.
fn pango_underline(style: DecorationStyle) -> &'static str {
    match style {
        DecorationStyle::Single | DecorationStyle::Dotted => "single",
        DecorationStyle::Double => "double",
        DecorationStyle::Wavy => "error",
    }
}

#[expect(
    clippy::float_cmp,
    reason = "a scale of exactly 1.0 is the unscaled default, compared as a sentinel"
)]
fn open_run(r: &TextRun, dialect: MarkupDialect, base_points: f64, out: &mut String) {
    use std::fmt::Write as _;

    if let Some(url) = r.link.as_deref() {
        out.push_str("<a href=\"");
        escape_markup(url, out);
        out.push_str("\">");
    }
    // The span carries the attributes that have no tag: colors, relative size, and (on Pango)
    // the monospace family. Bold/italic/strike/underline are tags in both dialects, which keeps
    // the attribute string short and the escaping trivial.
    let color = r.color.map(|c| hex(c.resolved()));
    let background = r.background.map(|c| hex(c.resolved()));
    let mono = r.font.monospace;
    if needs_span(r, dialect) {
        match dialect {
            MarkupDialect::Pango => {
                out.push_str("<span");
                if let Some(c) = &color {
                    let _ = write!(out, " foreground=\"{c}\"");
                }
                if let Some(c) = &background {
                    let _ = write!(out, " background=\"{c}\"");
                }
                if mono {
                    out.push_str(" font_family=\"monospace\"");
                }
                if r.font.scale != 1.0 {
                    // Absolute, in Pango units (1/1024 pt), not the `font_scale`/`size="N%"`
                    // pair: those are Pango 1.50 attributes, and a Pango that does not know an
                    // attribute fails the whole markup parse, which renders the label empty
                    // rather than merely unscaled. A point size still tracks the desktop's text
                    // scaling, since Pango resolves points through the Xft DPI.
                    let pts = r.font.resolved_points(base_points);
                    let _ = write!(out, " size=\"{}\"", (pts * 1024.0).round() as i64);
                }
                let features = r.font.feature_tags();
                if !features.is_empty() {
                    // The CSS `font-feature-settings` spelling Pango parses: `tnum 1, smcp 1`.
                    let list: Vec<String> = features.iter().map(|t| format!("{t} 1")).collect();
                    let _ = write!(out, " font_features=\"{}\"", list.join(", "));
                }
                // A styled or colored line travels as attributes; the plain one stays a tag.
                if let Some(d) = r.underline.filter(|_| !plain_line(r.underline)) {
                    let _ = write!(out, " underline=\"{}\"", pango_underline(d.style));
                    if let Some(c) = d.color {
                        let _ = write!(out, " underline_color=\"{}\"", hex(c.resolved()));
                    }
                }
                if let Some(d) = r.strikethrough.filter(|_| !plain_line(r.strikethrough)) {
                    out.push_str(" strikethrough=\"true\"");
                    if let Some(c) = d.color {
                        let _ = write!(out, " strikethrough_color=\"{}\"", hex(c.resolved()));
                    }
                }
                out.push('>');
            }
            MarkupDialect::QtHtml => {
                // Color only. Qt's rich text does not resolve the generic `monospace` family
                // from a style attribute (it rendered proportional), so the fixed face comes
                // from the `<code>` tag below, which Qt maps to its own fixed font.
                out.push_str("<span style=\"");
                if let Some(c) = &color {
                    let _ = write!(out, "color:{c};");
                }
                if let Some(c) = &background {
                    let _ = write!(out, "background-color:{c};");
                }
                if r.font.scale != 1.0 {
                    // Points, not a percentage: Qt's rich-text CSS subset understands `pt` and
                    // `px` and silently ignores `%`, which is why a scaled run rendered at the
                    // base size here while every other backend scaled it.
                    let pt = r.font.resolved_points(base_points);
                    let _ = write!(out, "font-size:{pt}pt;");
                }
                if r.font.small_caps {
                    out.push_str("font-variant:small-caps;");
                }
                // Qt's rich text draws one line style from markup; the color is a property
                // of the decoration as a whole. `Double`, `Dotted` and `Wavy` need a
                // `QTextCharFormat` underline style, which the Qt toolkit applies itself.
                if let Some(c) = r
                    .underline
                    .and_then(|d| d.color)
                    .or_else(|| r.strikethrough.and_then(|d| d.color))
                {
                    let _ = write!(out, "text-decoration-color:{};", hex(c.resolved()));
                }
                out.push_str("\">");
            }
        }
    }
    if dialect == MarkupDialect::QtHtml && mono {
        out.push_str("<code>");
    }
    if r.font.weight.is_some_and(|w| w >= FontWeight::Semibold) {
        out.push_str("<b>");
    }
    if r.font.italic {
        out.push_str("<i>");
    }
    let tags = dialect == MarkupDialect::QtHtml;
    if r.underline.is_some() && (tags || plain_line(r.underline)) {
        out.push_str("<u>");
    }
    if r.strikethrough.is_some() && (tags || plain_line(r.strikethrough)) {
        out.push_str("<s>");
    }
}

fn close_run(r: &TextRun, dialect: MarkupDialect, out: &mut String) {
    let tags = dialect == MarkupDialect::QtHtml;
    if r.strikethrough.is_some() && (tags || plain_line(r.strikethrough)) {
        out.push_str("</s>");
    }
    if r.underline.is_some() && (tags || plain_line(r.underline)) {
        out.push_str("</u>");
    }
    if r.font.italic {
        out.push_str("</i>");
    }
    if r.font.weight.is_some_and(|w| w >= FontWeight::Semibold) {
        out.push_str("</b>");
    }
    if dialect == MarkupDialect::QtHtml && r.font.monospace {
        out.push_str("</code>");
    }
    if needs_span(r, dialect) {
        out.push_str("</span>");
    }
    if r.link.is_some() {
        out.push_str("</a>");
    }
}

/// Are these runs well formed for `text`: ascending, non-overlapping, inside the string, and on
/// character boundaries?
///
/// Checked once in the pieces layer rather than in eight backends. A backend handed overlapping
/// ranges would produce a different wrong answer per platform, and a range splitting a multi-byte
/// character would panic on `str` slicing in some and render mojibake in others.
///
/// # Errors
///
/// Returns a message naming the first run that overlaps or precedes the run before it, ends
/// past the text, has an inverted range, or splits a multi-byte character.
pub fn runs_are_valid(text: &str, runs: &[TextRun]) -> Result<(), String> {
    let mut prev_end = 0usize;
    for (i, r) in runs.iter().enumerate() {
        if r.range.start < prev_end {
            return Err(format!(
                "run {i} starts at {} but the previous run ends at {prev_end} — runs must be \
                 ascending and non-overlapping",
                r.range.start
            ));
        }
        if r.range.end > text.len() {
            return Err(format!(
                "run {i} ends at {} but the text is {} bytes",
                r.range.end,
                text.len()
            ));
        }
        if r.range.start > r.range.end {
            return Err(format!("run {i} has an inverted range {:?}", r.range));
        }
        if !text.is_char_boundary(r.range.start) || !text.is_char_boundary(r.range.end) {
            return Err(format!(
                "run {i} range {:?} splits a multi-byte character",
                r.range
            ));
        }
        prev_end = r.range.end;
    }
    Ok(())
}

/// Shorten one line of `text` to fit `width`, putting an ellipsis where `mode` says, for a
/// toolkit whose label has no native head or middle truncation (docs/text.md "Truncation").
///
/// `measure` answers the width of a candidate string in the label's font. The search is a
/// bisection over how many characters to keep, so it measures about `log2(n)` candidates.
/// Text that already fits comes back unchanged; if not even the ellipsis fits, the ellipsis
/// alone is returned. Characters are Unicode scalar values, so a cut can separate a base
/// letter from a combining mark; a toolkit with a grapheme-aware native mode should use it.
///
/// ```
/// use day_spec::{elide, props::Truncation};
/// let width = |s: &str| s.chars().count() as f64;
/// assert_eq!(elide("/usr/local/share/day", Truncation::Middle, 9.0, width), "/usr…/day");
/// assert_eq!(elide("/usr/local/share/day", Truncation::Head, 9.0, width), "…hare/day");
/// assert_eq!(elide("/usr/local/share/day", Truncation::Tail, 9.0, width), "/usr/loc…");
/// ```
#[must_use]
pub fn elide(
    text: &str,
    mode: props::Truncation,
    width: f64,
    measure: impl Fn(&str) -> f64,
) -> String {
    const ELLIPSIS: char = '\u{2026}';
    if measure(text) <= width {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let build = |keep: usize| -> String {
        let mut out = String::with_capacity(text.len());
        match mode {
            props::Truncation::Tail => {
                out.extend(&chars[..keep]);
                out.push(ELLIPSIS);
            }
            props::Truncation::Head => {
                out.push(ELLIPSIS);
                out.extend(&chars[chars.len() - keep..]);
            }
            props::Truncation::Middle => {
                let head = keep.div_ceil(2);
                out.extend(&chars[..head]);
                out.push(ELLIPSIS);
                out.extend(&chars[chars.len() - (keep - head)..]);
            }
        }
        out
    };
    // The largest `keep` whose candidate fits; `keep` < chars.len() because the whole text
    // does not fit.
    let (mut lo, mut hi) = (0usize, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if measure(&build(mid)) <= width {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    build(lo)
}

pub mod props {
    use super::*;

    /// A container's native surface (docs/surfaces.md): what `.background(..)`,
    /// `.corner_radius(..)`, `.border(..)`, `.shadow(..)`, `.surface(..)` and `.card()` realize.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ContainerProps {
        /// What fills the surface: a solid [`Ink`] or a linear, radial or conic
        /// gradient. A gradient's unit points resolve against the surface's frame and follow it
        /// when it resizes. Every ink follows the appearance (docs/color.md "How a toolkit
        /// follows the appearance").
        pub fill: Option<super::Paint>,
        pub corner_radius: f64,
        /// Which corners [`Self::corner_radius`] rounds; the default is all four.
        pub corners: super::Corners,
        /// A line around the edge, inside the frame, following the rounded corners, drawn over
        /// the surface's content on every toolkit: a child that reaches the edge never covers
        /// it.
        pub border: Option<super::Border>,
        /// A shadow of the surface's rounded rectangle, drawn outside its frame. A surface
        /// with a shadow is built with `clips: false`: the shadow lies outside the bounds a
        /// clip would cut to, and on most toolkits the two cannot share a view.
        pub shadow: Option<super::Shadow>,
        pub clips: bool,
        /// Semantic, theme-adaptive surface, mapped by each backend to a native material that
        /// follows the platform's light/dark appearance automatically (it overrides
        /// [`Self::fill`] when set).
        pub role: Option<super::SurfaceRole>,
    }
    /// Post-realize changes to a container's surface. A constant surface sends none: these
    /// come only from a reactive modifier (`.background(signal)`, `.corner_radius(closure)`,
    /// `.surface(signal)`) or from Day emulating a frame-relative corner radius.
    #[derive(Clone, Debug, PartialEq)]
    pub enum ContainerPatch {
        /// The fill alone, the cheap and common change. With an animation, a solid fill
        /// changing to another solid fill animates (motion brief: the background color).
        Fill(Option<super::Paint>),
        /// Every styling field at once, an idempotent setter: fill, corner radius, corners,
        /// border, shadow and clipping. [`ContainerProps::role`] is realize-time and ignored
        /// here.
        Surface(Box<ContainerProps>),
    }

    /// Realize props for a `scroll` container: which axis it scrolls, and how the native scroll
    /// view behaves (docs/scroll.md). Backends create the matching native scroll view (vertical
    /// `UIScrollView`/`ScrollView`, horizontal `HorizontalScrollView`, etc.).
    ///
    /// Everything past `horizontal` is a preference: a backend applies what its scroll view
    /// offers and ignores the rest, and the `Cap` named on a field says whether it does. Every
    /// default is the platform's own behavior, so `ScrollProps { horizontal, ..Default::default() }`
    /// is the scroll view Day has always made.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ScrollProps {
        pub horizontal: bool,
        /// Whether the scroll indicators (scroll bars) show.
        pub indicators: ScrollIndicators,
        /// Whether the content rubber-bands or glows past its edges: `None` keeps the
        /// platform's default, `Some(false)` stops at the edge, `Some(true)` asks for the
        /// overscroll effect even when the content fits.
        pub bounce: Option<bool>,
        /// Where the content comes to rest ([`crate::Cap::ScrollSnap`]).
        pub snap: ScrollSnap,
        /// When scrolling dismisses the software keyboard
        /// ([`crate::Cap::ScrollKeyboardDismiss`]).
        pub keyboard_dismiss: KeyboardDismiss,
        /// What a tap inside the scroll view does while the software keyboard is up
        /// ([`crate::Cap::ScrollKeyboardDismiss`]; `Native` only).
        pub keyboard_taps: KeyboardTaps,
        /// The content zooms between these scales ([`crate::Cap::ScrollZoom`]); `None` is a
        /// scroll view that does not zoom.
        pub zoom: Option<ZoomRange>,
        /// Send [`crate::Event::ScrollChanged`] as the content moves
        /// ([`crate::Cap::ScrollOffset`]). Off, the view installs no scroll observer.
        pub track_offset: bool,
        /// Send [`crate::Event::ScrollPhase`] as user-driven scrolling starts and stops
        /// ([`crate::Cap::ScrollPhases`]).
        pub track_phase: bool,
    }

    /// Whether a scroll view shows its indicators ([`ScrollProps::indicators`]).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum ScrollIndicators {
        /// The platform's default: overlay bars that appear while scrolling, or classic bars.
        #[default]
        Automatic,
        /// No indicators. The content still scrolls.
        Hidden,
    }

    /// Where a scroll view's content comes to rest ([`ScrollProps::snap`]).
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub enum ScrollSnap {
        /// Anywhere.
        #[default]
        None,
        /// On multiples of the viewport's extent along the scroll axis, one page per swipe.
        Page,
        /// On multiples of this many points along the scroll axis.
        Interval(f64),
    }

    /// When scrolling dismisses the software keyboard ([`ScrollProps::keyboard_dismiss`]).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum KeyboardDismiss {
        /// Scrolling leaves the keyboard up.
        #[default]
        Never,
        /// The keyboard goes away when a drag begins.
        OnDrag,
        /// The keyboard follows the finger down once the drag reaches it, where the platform
        /// has that (UIKit); elsewhere the same as [`KeyboardDismiss::OnDrag`].
        Interactive,
    }

    /// What a tap inside a scroll view does while the software keyboard is up
    /// ([`ScrollProps::keyboard_taps`]).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum KeyboardTaps {
        /// The tap reaches the control under it and the keyboard stays up.
        #[default]
        Deliver,
        /// The tap dismisses the keyboard and goes no further; the next tap is delivered.
        Dismiss,
    }

    /// The scales a zooming scroll view moves between ([`ScrollProps::zoom`]); `1.0` is the
    /// content's natural size.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct ZoomRange {
        pub min: f64,
        pub max: f64,
    }

    /// Post-realize changes to a `scroll` container.
    #[derive(Clone, Debug, PartialEq)]
    pub enum ScrollPatch {
        /// Zoom the content to `scale`, clamped to [`ScrollProps::zoom`], keeping the
        /// viewport's center where it is. The view answers with
        /// [`crate::Event::ScrollZoomed`] as the scale moves. Ignored where
        /// [`crate::Cap::ScrollZoom`] is `Unsupported`.
        Zoom { scale: f64, animated: bool },
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LabelProps {
        pub text: String,
        pub font: FontSpec,
        pub color: Option<Ink>,
        /// What the text means, for the color the platform gives it (docs/text.md).
        ///
        /// Semantic the way [`crate::Font`] is semantic: an app says "this is secondary text"
        /// and each platform answers with its own answer: `secondaryLabelColor` on Apple,
        /// `?android:textColorSecondary`, GTK's `dim-label`. That is the only way a de-emphasized
        /// label stays correct in both appearances, since a literal grey that reads well on white
        /// is wrong on black. An explicit [`Self::color`] still wins; a backend that has no such
        /// color renders primary, which is legible and correct, just not dimmed.
        pub role: TextRole,
        pub wraps: bool,
        /// Maximum rendered lines; zero is unlimited. A duty on every toolkit: the label
        /// draws at most this many lines, ends the last one as [`Self::truncation`] says when
        /// text is left over, and measures no taller than that many lines. Changes arrive as
        /// [`LabelPatch::Paragraph`].
        pub max_lines: u32,
        /// Where the ellipsis goes when the text does not fit in one line (`wraps == false`)
        /// or in [`Self::max_lines`] lines.
        pub truncation: Truncation,
        /// Extra space after each character, in ems (multiples of the label's resolved font
        /// size), so tracking follows the reader's text size. `0.0` is the font's own
        /// spacing; a negative value tightens. A toolkit whose API takes points multiplies
        /// by the resolved size and repeats that when the font changes.
        pub letter_spacing: f64,
        /// The distance between baselines as a multiple of the label's resolved font size.
        /// `0.0` (and anything nonpositive or nonfinite) is the font's natural line height,
        /// about 1.2. The extra space is split above and below each line, so a one-line
        /// label grows too and its text stays centered in the line box.
        pub line_height: f64,
        /// Emit [`Event::TextSelected`] when the reader's
        /// selection in this label changes ([`Cap::LabelSelection`]).
        /// Set only on a selectable label; a toolkit that cannot report selections ignores it.
        pub reports_selection: bool,
        /// How the label's lines sit within its own width. Only observable on a label that
        /// wraps or carries explicit newlines: a single line fills its box, so its alignment is
        /// the container's business, not the label's.
        ///
        /// `Leading` is the default and is what running text wants; `Center` is for the short,
        /// centered block a welcome screen or an empty state uses. A backend that cannot set
        /// it renders leading-aligned, which is legible and correct, just not centered.
        pub align: TextAlign,
        /// Styled spans within `text` (docs/text-runs.md). Empty is the overwhelmingly common
        /// case and means exactly what a label has always meant: one font, one color. A backend
        /// that cannot draw runs ignores this and renders `text` uniformly, which is legible and
        /// correct, just unstyled.
        pub runs: Vec<crate::TextRun>,
    }
    /// What a label's text means, for the color the platform gives it (`LabelProps::role`).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum TextRole {
        /// Ordinary text, in the platform's primary label color.
        #[default]
        Primary,
        /// De-emphasized text: a caption, a hint, the "nothing selected" line an empty detail
        /// pane shows. Rendered in the platform's secondary label color.
        Secondary,
    }

    impl LabelProps {
        /// The paragraph attributes as one value, what [`LabelPatch::Paragraph`] carries, so a
        /// toolkit applies them through one function at realize and on every patch.
        #[must_use]
        pub fn paragraph(&self) -> LabelParagraph {
            LabelParagraph {
                max_lines: self.max_lines,
                truncation: self.truncation,
                letter_spacing: self.letter_spacing,
                line_height: self.line_height,
            }
        }
    }

    /// How a label's lines break, space and end: the paragraph attributes that can change
    /// after realize ([`LabelPatch::Paragraph`]). Each field means what the
    /// [`LabelProps`] field of the same name means.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub struct LabelParagraph {
        pub max_lines: u32,
        pub truncation: Truncation,
        pub letter_spacing: f64,
        pub line_height: f64,
    }

    /// Where a label that runs out of room puts its ellipsis (`LabelProps::truncation`).
    ///
    /// `Head` and `Middle` are guaranteed on a one-line label (`single_line()` or
    /// `max_lines(1)`). With more lines a toolkit applies them to the last line where the
    /// platform can and truncates the tail where it cannot.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum Truncation {
        /// Keep the start: `The quick brown…`. What running text wants.
        #[default]
        Tail,
        /// Keep the end: `…the lazy dog`. For text whose end identifies it.
        Head,
        /// Keep both ends: `The qui…azy dog`. For a file path or an identifier.
        Middle,
    }

    /// How a paragraph's lines sit in the width they are given: a label's `align`, a canvas
    /// text anchor's horizontal part, and a styled document's paragraph alignment
    /// ([`ParagraphStyle::align`](crate::ParagraphStyle)).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub enum TextAlign {
        /// The reading direction's start edge: left in a left-to-right layout, right in a
        /// right-to-left one (docs/localization.md).
        #[default]
        Leading,
        Center,
        /// The reading direction's end edge.
        Trailing,
        /// Every line but the last stretched to both edges; the last line sits at the leading
        /// edge. A toolkit that cannot justify draws `Leading`. A canvas text anchor reads it
        /// as `Leading`.
        Justified,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum LabelPatch {
        Text(String),
        Color(Option<Ink>),
        Font(FontSpec),
        /// New text and its runs together. One patch, not two: a run's byte range is only
        /// meaningful against a particular string, so applying them separately would leave the
        /// widget briefly holding ranges that point into the wrong text.
        Runs(String, Vec<crate::TextRun>),
        /// The paragraph attributes, whole: the line limit, the truncation mode, the letter
        /// spacing and the line height. Sent when any reactive one changes, so the setter is
        /// idempotent. The label is measured again afterward.
        Paragraph(LabelParagraph),
    }

    /// A button's native styling tier. `Automatic` is the toolkit's stock look; `Bordered`
    /// asks for a visually contained button where the stock look is borderless (iOS's plain
    /// system button reads as a link); `Prominent` asks for the platform's accent-filled /
    /// default-action affordance. Toolkits whose stock buttons are already contained treat
    /// `Bordered` as `Automatic`.
    // `Eq` is gone with the `Tinted` color: `Color` holds floats. `PartialEq` is what the
    // props diff uses, and that is unaffected.
    #[derive(Clone, Copy, Debug, Default, PartialEq)]
    pub enum ButtonStyleSpec {
        #[default]
        Automatic,
        Bordered,
        Prominent,
        /// A filled button in an app-chosen color, still drawn by the native control: the
        /// platform keeps its own pressed, hover, focus and disabled rendering, its button role,
        /// and its keyboard activation. The label color is not carried; each backend picks the
        /// readable one for the fill (see [`ButtonStyleSpec::on_tint`]), the same call the
        /// platform's own tinted buttons make.
        ///
        /// A backend with no way to recolor its button ignores this and draws its ordinary
        /// button. That is the trade: a plain button everywhere beats a colored
        /// rectangle that is no longer a button on the platforms that cannot.
        Tinted(Ink),
        /// A button no wider than its title asks for (a one-glyph "−" or "+" in a stepper,
        /// a "×" on a chip) on toolkits whose stock button carries a minimum width and wide
        /// insets (Material's 88 dp button turns a two-button stepper into 240 dp). The native
        /// control stays: only its minimum width and horizontal padding are reduced. A toolkit
        /// whose buttons already hug their title (AppKit, UIKit's plain button, Qt) draws its
        /// ordinary button.
        Compact,
        /// Borderless native action button; compact horizontal insets on Material.
        Plain,
    }

    impl ButtonStyleSpec {
        /// The label color to draw on a [`ButtonStyleSpec::Tinted`] fill: whichever of black or
        /// white contrasts better against it, by WCAG's contrast ratio.
        ///
        /// Comparing the two ratios rather than thresholding the luminance is the difference
        /// between right and nearly right. A mid amber (`#F0A64C`) has a relative luminance of
        /// 0.44; under a "brighter than half" test it reads as dark and takes white text, at a
        /// hopeless 2.2:1. Its contrast against black is 9.7:1. The two ratios cross at a
        /// luminance of 0.179, not 0.5, and everything between those two numbers is a button
        /// nobody can read.
        pub fn on_tint(fill: Color) -> Color {
            // WCAG relative luminance: sRGB channels linearized, then weighted by the eye's
            // sensitivity to each.
            let lin = |c: f64| {
                let c = c.clamp(0.0, 1.0);
                if c <= 0.04045 {
                    c / 12.92
                } else {
                    ((c + 0.055) / 1.055).powf(2.4)
                }
            };
            let l = 0.2126 * lin(fill.r) + 0.7152 * lin(fill.g) + 0.0722 * lin(fill.b);
            // Contrast ratio is (lighter + 0.05) / (darker + 0.05); white is 1.0, black is 0.0.
            let against_white = 1.05 / (l + 0.05);
            let against_black = (l + 0.05) / 0.05;
            if against_black >= against_white {
                Color::BLACK
            } else {
                Color::WHITE
            }
        }
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    /// The visible content and accessible title of a button with an icon.
    pub struct ButtonContent {
        /// Also serves as the accessible name when only the icon is visible.
        pub title: String,
        pub icon: Option<Icon>,
        pub icon_only: bool,
    }
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ButtonProps {
        pub title: String,
        pub icon: Option<Icon>,
        pub icon_only: bool,
        pub enabled: bool,
        pub style: ButtonStyleSpec,
        /// The button stands for the current choice among its siblings (a document tab, a
        /// view switcher): the platform's own checked or selected rendering, which keeps the
        /// button enabled, focusable and activatable, and tells assistive tech it is selected.
        /// A click never toggles it natively; only the app's next patch changes it.
        pub selected: bool,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum ButtonPatch {
        Title(String),
        /// Update an icon button's label and icon together. Plain buttons still use `Title`.
        Content(ButtonContent),
        Enabled(bool),
        /// The selected state (see [`ButtonProps::selected`]).
        Selected(bool),
        /// A live style change: what a reactive `.tint(…)` sends, so a button can recolor with
        /// app state without being torn down and realized again.
        Style(ButtonStyleSpec),
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ToggleProps {
        pub on: bool,
        pub enabled: bool,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum TogglePatch {
        On(bool),
        Enabled(bool),
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct SliderProps {
        pub value: f64,
        pub min: f64,
        pub max: f64,
        pub step: Option<f64>,
        pub enabled: bool,
    }
    impl Default for SliderProps {
        fn default() -> Self {
            SliderProps {
                value: 0.0,
                min: 0.0,
                max: 1.0,
                step: None,
                enabled: true,
            }
        }
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum SliderPatch {
        Value(f64),
        Enabled(bool),
    }

    /// SwiftUI's `pickerStyle` analogue (`kinds::PICKER`). Each maps to a distinct native
    /// control per toolkit (docs/picker.md).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum PickerStyle {
        /// A dropdown/pop-up menu (`NSPopUpButton` / `GtkDropDown` / `QComboBox` / UIButton+UIMenu / Spinner).
        #[default]
        Menu,
        /// A horizontal segmented control (`NSSegmentedControl` / `UISegmentedControl` / linked toggles / …).
        Segmented,
        /// A vertical radio-button group laid out inline (`NSButton` radios / `GtkCheckButton` group / …).
        Inline,
    }

    /// Full picker props (realize). `style` is set once at build; `options`, `selected` and
    /// `enabled` patch (via [`PickerPatch`]).
    #[derive(Clone, Debug, PartialEq)]
    pub struct PickerProps {
        /// Decorative boundaries before option indexes (not selectable options). Native menu
        /// grouping on AppKit/UIKit; ignored by other backends and non-menu styles.
        pub separators_before: Vec<usize>,
        pub options: Vec<String>,
        pub selected: usize,
        pub style: PickerStyle,
        /// Whether the picker takes input. Default `true`.
        pub enabled: bool,
    }
    impl Default for PickerProps {
        fn default() -> Self {
            PickerProps {
                separators_before: Vec::new(),
                options: Vec::new(),
                selected: 0,
                style: PickerStyle::default(),
                enabled: true,
            }
        }
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum PickerPatch {
        Selected(usize),
        /// New option labels, for a picker whose choices come from data (a document list, a
        /// live count). The backend rebuilds its items and keeps the selected index where it
        /// still exists, clamping to the last option otherwise; a fresh
        /// [`PickerPatch::Selected`] follows whenever the app's own binding disagrees.
        Options(Vec<String>),
        /// Enable or disable the whole control, every style alike.
        Enabled(bool),
    }

    /// Full text-area props (realize, `kinds::TEXT_AREA`; docs/textarea.md). `text` seeds the
    /// editor; `min_lines`/`max_lines` bound the auto-growing height in text lines
    /// (`max_lines == 0` = unbounded). `editable`/`selectable`/`spellcheck` control the native
    /// editor attributes (all default `true`); a backend that can't honor one answers
    /// `Cap::Text{Editable,Selectable,SpellCheck}` = `Unsupported`. `text` and the three attributes
    /// change after build (via [`TextAreaPatch`]); the rest are build-only.
    #[derive(Clone, Debug, PartialEq)]
    pub struct TextAreaProps {
        pub text: String,
        pub placeholder: String,
        pub min_lines: u32,
        pub max_lines: u32,
        /// Whether the user can edit the text (`false` = read-only). Default `true`.
        pub editable: bool,
        /// Whether the text can be selected (and copied). Default `true`.
        pub selectable: bool,
        /// Whether spell-check / autocorrect highlighting is on. Default `true`.
        pub spellcheck: bool,
        /// Plain Enter emits `Event::Submitted` instead of inserting a newline (Shift+Enter
        /// still inserts one where the platform can distinguish). Chat composers. Default
        /// `false`.
        pub submit_on_enter: bool,
    }

    impl Default for TextAreaProps {
        fn default() -> Self {
            TextAreaProps {
                text: String::new(),
                placeholder: String::new(),
                min_lines: 1,
                max_lines: 0,
                editable: true,
                selectable: true,
                spellcheck: true,
                submit_on_enter: false,
            }
        }
    }

    /// Imperative text-area updates: replace the text (programmatic sync), or flip one of the
    /// live attributes.
    #[derive(Clone, Debug, PartialEq)]
    pub enum TextAreaPatch {
        SetText(String),
        SetEditable(bool),
        SetSelectable(bool),
        SetSpellCheck(bool),
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct TextFieldProps {
        pub text: String,
        pub placeholder: String,
        pub enabled: bool,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum TextFieldPatch {
        /// Origin-tagged write (§4.4): `from_native` suppresses the echo back into the widget.
        Text {
            text: String,
            from_native: bool,
        },
        Placeholder(String),
        Enabled(bool),
        /// Select a range of the field's text, or place the caret when the range is empty
        /// (docs/textfield.md "Selection"). Sent after any [`TextFieldPatch::Text`] of the same
        /// turn, so the range always counts the text the field now holds. A backend clamps it
        /// against its own text with [`TextSelection::utf16`] or [`TextSelection::bytes`] and
        /// applies it to a focused field; whether an unfocused field keeps it is the platform's
        /// (docs/textfield.md).
        Select(TextSelection),
    }

    /// A range of a text field's text in Unicode scalar values (Rust `char`s), the unit
    /// [`TextFieldPatch::Select`] carries on every backend. `start..end` is half-open, and an
    /// empty range is a caret position. The helpers clamp to the text and convert to the unit a
    /// native field counts: UTF-16 code units (Apple, Android, Qt, WinUI, the web, ArkUI) or
    /// bytes. GTK counts characters, which is [`TextSelection::clamped`].
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub struct TextSelection {
        pub start: usize,
        pub end: usize,
    }

    impl TextSelection {
        /// The range from `start` to `end`, ordered so that `start <= end`.
        pub fn new(start: usize, end: usize) -> Self {
            TextSelection {
                start: start.min(end),
                end: start.max(end),
            }
        }

        /// The range held to `text`: both ends at most its character count, `start <= end`.
        #[must_use]
        pub fn clamped(self, text: &str) -> Self {
            let n = text.chars().count();
            TextSelection::new(self.start.min(n), self.end.min(n))
        }

        /// The clamped range as UTF-16 code-unit offsets into `text`: `(start, end)`.
        #[must_use]
        pub fn utf16(self, text: &str) -> (usize, usize) {
            let r = self.clamped(text);
            let at = |chars: usize| -> usize {
                text.chars().take(chars).map(char::len_utf16).sum::<usize>()
            };
            (at(r.start), at(r.end))
        }

        /// The clamped range as UTF-8 byte offsets into `text`: `(start, end)`, each on a
        /// character boundary.
        #[must_use]
        pub fn bytes(self, text: &str) -> (usize, usize) {
            let r = self.clamped(text);
            let at = |chars: usize| -> usize {
                text.char_indices()
                    .nth(chars)
                    .map_or(text.len(), |(i, _)| i)
            };
            (at(r.start), at(r.end))
        }
    }

    #[cfg(test)]
    mod text_selection_tests {
        use super::TextSelection;

        #[test]
        fn orders_and_clamps() {
            assert_eq!(TextSelection::new(5, 2), TextSelection { start: 2, end: 5 });
            let r = TextSelection::new(3, usize::MAX).clamped("github.com");
            assert_eq!(r, TextSelection { start: 3, end: 10 });
            let past = TextSelection::new(40, 50).clamped("git");
            assert_eq!(past, TextSelection { start: 3, end: 3 });
            assert_eq!(
                TextSelection::new(0, 9).clamped(""),
                TextSelection::default()
            );
        }

        #[test]
        fn converts_to_native_units() {
            // "é" is one char, one UTF-16 unit and two bytes; "😀" is one char, two UTF-16
            // units and four bytes.
            let text = "aé😀b";
            assert_eq!(TextSelection::new(1, 3).utf16(text), (1, 4));
            assert_eq!(TextSelection::new(1, 3).bytes(text), (1, 7));
            assert_eq!(TextSelection::new(0, usize::MAX).utf16(text), (0, 5));
            assert_eq!(TextSelection::new(0, usize::MAX).bytes(text), (0, 8));
            assert_eq!(TextSelection::new(4, 4).bytes(text), (8, 8));
        }
    }

    /// How an image is scaled to fill its frame (§18.3). Maps to each toolkit's native scaling.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum ContentMode {
        /// Scale to fit entirely inside the frame, preserving aspect ratio (letterboxed). The
        /// default: an image never stretches unless asked. SwiftUI's `.scaledToFit`.
        #[default]
        Fit,
        /// Scale to fill the frame, preserving aspect ratio and cropping the overflow. SwiftUI's
        /// `.scaledToFill`.
        Fill,
        /// Stretch to fill the frame exactly, ignoring aspect ratio.
        Stretch,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ImageProps {
        /// Which pixels to draw: a staged asset name, encoded bytes, or an already-decoded
        /// image ([`ImageSource`], docs/images.md). A name resolves through the backend's own
        /// pipeline (§18.2); bytes and decoded ids need [`Cap::ImageDecode`], and a backend
        /// without it draws nothing rather than guessing.
        pub source: ImageSource,
        pub decorative: bool,
        /// How the image scales within its frame (default [`ContentMode::Fit`], no stretching).
        pub content_mode: ContentMode,
        /// Optional width:height ratio the view is constrained to (e.g. `16.0/9.0`). `None` lets the
        /// image take its allocated frame.
        pub aspect_ratio: Option<f64>,
        /// Monochrome tint for a template/vector glyph (docs/vectors.md): backends that can,
        /// recolor natively (template rendering on Apple, drawable tint on Android, pixel
        /// recolor on GTK); backends that can't yet ignore it and draw the source colors.
        /// `None` (the default, and every raster `image(…)`) means "as authored".
        pub tint: Option<Ink>,
        /// A monochrome template glyph (docs/vectors.md "Template"): with no `tint`, it draws
        /// in the foreground of the surface it sits on — the theme's label color on the
        /// desktops, the cell's tint on UIKit, `currentColor` on the web — and follows a
        /// theme switch. What a sidebar row's untinted icon is: a tinted glyph recolors the
        /// same way with or without this, and `false` (the default) draws an untinted glyph as
        /// authored. [`ImagePatch::Tint`]`(None)` on a template returns to that foreground,
        /// not to the authored colors.
        pub template: bool,
    }

    /// A list row's standard content (`kinds::LIST_ITEM`, docs/list.md "List items"). Empty
    /// text means the part is absent.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ListItemProps {
        pub title: String,
        /// A second line under the title, in the platform's secondary style.
        pub subtitle: String,
        /// Trailing text (a count, a value), in the platform's secondary style.
        pub detail: String,
        /// The leading glyph: a symbol or a bundled image.
        pub icon: Option<crate::Icon>,
        /// The glyph's color; `None` takes the platform's own (template glyphs follow the
        /// foreground, the tint color on UIKit).
        pub icon_tint: Option<Ink>,
        /// A trailing check mark: the platform's selected-option mark (UIKit's checkmark
        /// accessory, a Material check icon), announced as checked.
        pub checked: bool,
    }

    /// A live change to a realized list item; every part changes in place.
    #[derive(Clone, Debug, PartialEq)]
    pub enum ListItemPatch {
        Title(String),
        Subtitle(String),
        Detail(String),
        Icon(Option<crate::Icon>),
        IconTint(Option<Ink>),
        Checked(bool),
    }

    /// A live change to a realized image or vector glyph.
    ///
    /// `content_mode` and the rest still describe which art this is and how it fills its frame,
    /// which a rebuild expresses better than a patch. Two things do want to follow a signal: the
    /// tint (a glyph that recolors with the selection or the theme should repaint, not be torn
    /// down and realized again, docs/vectors.md "Tint") and the source, now that it can be a
    /// buffer the app swaps (a download landing, a new photo picked, docs/images.md). Replacing
    /// the pixels of a view that is already on screen is what a patch is for; realizing a second
    /// view would flash.
    #[derive(Clone, Debug, PartialEq)]
    pub enum ImagePatch {
        /// Recolor the glyph, or `None` to draw the authored colors again.
        Tint(Option<Ink>),
        /// Draw different pixels in the same view. A backend that cannot decode the new source
        /// (bytes where [`Cap::ImageDecode`] is `Unsupported`) leaves what it was drawing.
        Source(ImageSource),
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct CanvasProps {
        pub ops: Vec<DrawOp>,
    }

    /// Progress indicator. `value` is the completed fraction in `0.0..=1.0`; `None` means
    /// indeterminate (an animated spinner / busy bar, no known extent). Backends map this to
    /// their native determinate/indeterminate widgets (docs/progress.md).
    #[derive(Clone, Debug, PartialEq)]
    pub struct ProgressProps {
        pub value: Option<f64>,
        /// Whether an indeterminate indicator animates; ignored for a determinate bar.
        pub animating: bool,
        /// The platform's large indicator size.
        pub large: bool,
    }
    impl Default for ProgressProps {
        fn default() -> Self {
            Self {
                value: None,
                animating: true,
                large: false,
            }
        }
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum ProgressPatch {
        Animating(bool),
        /// New completed fraction, or `None` to switch to indeterminate.
        Value(Option<f64>),
    }

    /// A rule between neighbors (`divider()`): a hairline across the width it is offered, or,
    /// with `vertical`, down the height it is offered (`divider().vertical()`, between a
    /// row's panes). A toolkit measures both the same way, as a horizontal rule, because the
    /// pieces layer swaps the axes of a vertical one's proposal and answer; realization is
    /// where the orientation matters, for the few widgets that draw a horizontal line
    /// regardless of their frame.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DividerProps {
        pub vertical: bool,
    }

    /// How a navigation host lays its panes out (docs/navigation.md).
    ///
    /// This is the resolved presentation, what the toolkit must draw right now. The app asks for
    /// one through `Nav::presentation`, where leaving it unset means "automatic": the pieces
    /// layer resolves it from the window's [`SizeClass`] and the toolkit's `Cap::NavSplit`, and
    /// re-resolves it whenever the class changes. There is no `Auto` variant here because a
    /// backend can only ever draw a concrete one, so the undecided state stays on the app's side
    /// of the boundary where it can still be decided.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum NavPresentation {
        /// Sidebar beside detail, both visible (`NSSplitViewController`, nested `GtkPaned`s,
        /// a `NavigationView`). The sidebar pane gets its own container.
        Split,
        /// One page at a time, back-navigable. The sidebar pane becomes the stack's root.
        #[default]
        Stack,
        /// The rows drawn as a tab bar, one page's content beside it, no back stack: what a
        /// phone-shaped window wants from a one-of-N surface (`UITabBarController`, a Material
        /// `NavigationBarView`, `NavigationView.PaneDisplayMode = Top`). The `Pane::Sidebar` page
        /// is not drawn at all: its rows are the chrome.
        Tabs,
        /// The rows drawn as a narrow icon strip beside the content, the `Medium`-width answer
        /// between [`Self::Tabs`] and [`Self::Split`] (a Material `NavigationRailView`,
        /// `PaneDisplayMode = LeftCompact`, an ArkUI vertical `Tabs`).
        ///
        /// A backend with no rail rounds it to a neighbor of its own choosing rather than
        /// failing: UIKit answers `Tabs` (which is what iPadOS does at that width), GTK the same.
        /// Rounding is expected and correct; this variant is a request, not a contract.
        Rail,
    }

    impl NavPresentation {
        /// `true` for [`NavPresentation::Split`], the shape most backend code wants.
        #[must_use]
        pub fn is_split(self) -> bool {
            self == NavPresentation::Split
        }

        /// `true` when the rows are the chrome rather than a drawn page: [`Self::Tabs`] and
        /// [`Self::Rail`]. Both hide the `Pane::Sidebar` page and render its rows themselves, so
        /// backend code that only cares about that distinction asks this instead of matching both.
        #[must_use]
        pub fn rows_are_chrome(self) -> bool {
            matches!(self, NavPresentation::Tabs | NavPresentation::Rail)
        }
    }

    /// Navigation host (docs/navigation.md).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavProps {
        /// Document-tab chrome, independent of navigation destinations.
        pub documents: Option<DocumentTabsConfig>,
        pub title: String,
        /// The presentation to draw now; re-presented in place by [`NavPatch::Presentation`].
        pub presentation: NavPresentation,
        /// The app left the presentation automatic (`NavStyle::Automatic`, or a `Sidebar`
        /// nav host with no `.presentation(…)` pin), so it is free to follow the window.
        ///
        /// Only a toolkit answering `Cap::NavRepresent = Emulated` needs this: its own adaptive
        /// container owns the morph, and it must know whether the app asked for one at all before
        /// building it. With two presentations that bit could be inferred from a lowered `Split`;
        /// with four it cannot, so it is carried explicitly rather than encoded.
        ///
        /// `Native` re-presenters ignore it (they are told each presentation as it is resolved),
        /// and so do toolkits that cannot re-present.
        pub adaptive: bool,
        /// Draw the platform's own show/hide-sidebar affordance on this host's chrome
        /// (docs/toolbars.md). `true` for every `nav(Sidebar)` unless the app opted out:
        /// the toolkit owns both the button and the behavior
        /// (`NSToolbarToggleSidebarItemIdentifier`, the split view's collapse on GTK,
        /// `NavigationView.IsPaneOpen` on XAML), so an app that once declared the item by hand
        /// now declares nothing. Hosts with no sidebar ignore it.
        pub sidebar_toggle: bool,
        /// Search over this navigation surface (`Nav::searchable`, docs/search.md).
        /// `None` = the surface is not searchable and no field is rendered anywhere.
        pub search: Option<SearchProps>,
        /// `Some(width)` = this host has a content-list pane ([`Pane::List`]) at this preferred
        /// width in points, between the sidebar and the detail (docs/navigation.md). Read at
        /// realize, because a backend that builds a different container for a three-pane host
        /// (UIKit's triple-column style is init-only) needs to know before it builds. `None` on
        /// every host without one, including all of them on backends answering
        /// [`crate::Cap::NavContentList`] `Unsupported`: the pieces layer composes there and
        /// the prop is never set.
        pub list_width: Option<f64>,
        /// Whether that pane is showing for the destination the host opens on
        /// (`Nav::content_list_for`). Read at realize, like `list_width`, and for a
        /// related reason: a backend whose pane is a real split item can only honor a
        /// collapsed state reliably by setting it before the item joins the split. Told
        /// afterwards (by `NavPatch::ListVisible`, on a window that has not been displayed
        /// yet), AppKit reports the item collapsed and then lays it back out from its holding
        /// priorities, and the app opens with a list beside a page that does not own one.
        /// `true` on every host whose first destination has a list, and on hosts with no pane
        /// at all, where it means nothing.
        pub list_visible: bool,
    }

    /// Switch only tab chrome; resident contents must retain their native identity.
    #[derive(Clone, Copy, Debug, PartialEq)]
    pub struct DocumentTabsPatch {
        pub native: bool,
    }

    /// Localized document-tab chrome. Native callbacks emit stable route keys.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct DocumentTabsConfig {
        pub new_label: String,
        pub close_label: String,
        pub can_add: bool,
    }

    /// Where a searchable surface's field should be drawn.
    ///
    /// A preference, not an instruction: a backend that cannot honor the request falls back to
    /// whatever its platform does, exactly as SwiftUI's `searchable(placement:)` does ("depending
    /// on the containing view hierarchy and platform, the requested placement may not be able to
    /// be fulfilled"). `Automatic` is the one to reach for: it is what lets the field live in the
    /// window toolbar on a wide window and move into the navigation list on a narrow one.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum SearchPlacement {
        /// The platform decides, from the window's size class and its own convention.
        #[default]
        Automatic,
        /// In the window's toolbar (`NSSearchToolbarItem`, an `AdwHeaderBar` entry, a
        /// `CommandBar` `AutoSuggestBox`). `Inline` where the toolbar cannot hold a search field
        /// (`Cap::ToolbarSearch`).
        Toolbar,
        /// Attached to the navigation surface itself: above the sidebar list, or in the
        /// navigation bar's search drawer on the phones.
        Inline,
    }

    /// Search over a navigation surface (docs/search.md). The query itself stays an app-owned
    /// signal; this carries only what the toolkit must draw, so moving the field between
    /// placements never moves the state.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct SearchProps {
        /// Current text. Two-way: the user typing emits [`Event::SearchChanged`], and the app's
        /// own writes arrive as [`SearchPatch::Text`].
        pub text: String,
        /// Empty-state prompt.
        pub prompt: String,
        pub placement: SearchPlacement,
        /// Opt into native scrolling search/title chrome on mobile. Default false keeps
        /// the field pinned. Desktop toolbar search is unchanged.
        pub hides_on_scroll: bool,
        /// Start the UIKit navigation search drawer collapsed, with a compact toolbar title.
        /// Implies scrolling search on UIKit; ignored by other backends. Default false.
        pub initially_hidden: bool,
        /// Scope titles, empty = no scope bar. One-of-N: a `UISearchBar` scope bar, a Material
        /// `ChipGroup` of single-selection filter chips, an ArkUI `SegmentButtonV2`, an
        /// `NSSegmentedControl` (docs/search.md has the per-backend table).
        pub scopes: Vec<String>,
        /// Index into `scopes`; meaningless when `scopes` is empty.
        pub scope: usize,
        /// Completions offered for the current text. On a navigation surface these complete the
        /// field rather than replacing the list: the list is already the result set, so an
        /// overlay of results would cover the thing it is filtering.
        pub suggestions: Vec<String>,
    }

    /// A targeted update to a live search field, the path a bound signal writes through, so
    /// syncing text never rebuilds (and refocuses) the field mid-word.
    #[derive(Clone, Debug, PartialEq)]
    pub enum SearchPatch {
        Focus,
        Text(String),
        Scope(usize),
        Suggestions(Vec<String>),
    }
    /// Applied to the nav host after a page child is attached / before it is removed;
    /// the toolkit animates its native presentation accordingly.
    #[derive(Clone, Debug, PartialEq)]
    pub enum NavPatch {
        /// The just-attached last page child became the top of the stack. `immersive` marks a
        /// page that keeps the floating transparent chrome on backends with an immersive nav
        /// mode (UIKit and edge-to-edge Material). Unmarked pages get the
        /// standard opaque bar.
        Pushed { title: String, immersive: bool },
        /// The top page is about to be removed; present its predecessor.
        Popped,
        /// Current top-of-stack title changed.
        Title(String),
        /// The top page has a back guard (`NavStack::on_back`, docs/navigation.md): while `true`,
        /// the toolkit must not auto-pop on a native back gesture/button but route it to
        /// Day as `Event::NavBack { already_popped: false }` so the app's guard decides. On
        /// backends whose back already routes through Day (AppKit/Qt/XAML/web custom headers)
        /// this is a no-op; iOS disables the swipe, Android holds its callback, GTK sets the
        /// page `can-pop=false`, ArkUI consumes `onBackPressed`.
        GuardTop(bool),
        /// Re-present the host: the window's size class crossed a breakpoint, or the app wrote a
        /// new `presentation`. The page children do not change: the toolkit rebuilds its
        /// chrome and re-homes the pages it already has, each one landing by its
        /// [`NavPageProps::pane`]. Rebuilding them instead would drop scroll offsets, field
        /// focus, and every native animation in flight, which is why this is a patch and not
        /// a rebuild.
        ///
        /// Backends with no split container ignore it and stay stacked.
        Presentation(NavPresentation),
        /// Show the resident [`Pane::Detail`] page at this index, counting the host's detail
        /// children in attach order. The counterpart to [`Self::Pushed`]/[`Self::Popped`] for the
        /// presentations where pages are resident rather than stacked
        /// ([`NavPresentation::rows_are_chrome`]): a tab bar switches between pages that all stay
        /// alive, so there is nothing to push and nothing to pop.
        ///
        /// Applied without re-emitting [`crate::Event::SelectionChanged`], per the from-native
        /// echo rule: this is the programmatic-sync direction.
        ///
        /// A backend drawing a stacked presentation never receives it; the pieces layer sends
        /// push/pop there instead.
        Select(usize),
        /// Reorder resident detail pages by their previous indices without rebuilding them.
        Reorder(Vec<usize>),
        /// Show or collapse the content-list pane ([`Pane::List`]), the per-destination
        /// visibility switch (`Nav::content_list_for`, docs/navigation.md): a nav host
        /// section that spans the whole detail area (a settings page) collapses the pane, and
        /// selecting a list-backed section brings it back. Only sent to hosts whose
        /// [`NavProps::list_width`] is set. Applied directly, not through an animator proxy:
        /// the screenshot step captures the instant the patch returns (the sidebar-toggle rule).
        ListVisible(bool),
        /// Stacked presentations on a merged-pane backend ([`crate::Cap::NavContentList`]
        /// `Emulated`) only: the resident [`Pane::List`] page joins (`true`) or leaves
        /// (`false`) the navigation stack directly above the sidebar root. The pieces layer
        /// sends it as the selection gains or loses a list-backed destination; the backend
        /// realizes the membership change through its platform's navigation APIs (UIKit:
        /// `showColumn(.supplementary)` / a pop to the root; never a manual splice, which
        /// destroys the split's collapse bookkeeping). Never sent to `Native` backends (their
        /// pane persists through every presentation) nor while split.
        ListInStack(bool),
    }

    /// Which pane of a navigation host a page belongs to (docs/navigation.md).
    ///
    /// A page's pane is a fact about the model, not about how the host currently draws it: a
    /// nav host's list page is [`Pane::Sidebar`] whether the toolkit is showing a sidebar beside
    /// a detail or stacking the two. What the presentation decides is where each pane lands:
    /// its own splitter pane in [`NavPresentation::Split`], the root of the stack in
    /// [`NavPresentation::Stack`]. Keeping the two separate is what lets a host re-present on a
    /// size-class change (`NavPatch::Presentation`) by re-homing the pages it already has,
    /// instead of tearing the tree down and rebuilding it.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Pane {
        /// The list / master pane of a nav host. At most one page per host.
        Sidebar,
        /// The content-list pane between the sidebar and the detail, Mail's message list
        /// (`Nav::content_list`, docs/navigation.md). At most one page per host, and only
        /// on hosts whose [`NavProps::list_width`] is set; a backend answering
        /// [`crate::Cap::NavContentList`] `Unsupported` never sees one.
        List,
        /// A destination page. Every page of a `nav_stack` presentation is one of these.
        #[default]
        Detail,
    }

    /// One destination's native container.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavPageProps {
        pub title: String,
        pub pane: Pane,
        /// Paint behind the top system/navigation bars on UIKit and edge-to-edge Android.
        /// Desktop toolkits retain their usual page chrome.
        pub immersive: bool,
    }

    /// The largest a modal sheet grows on a desktop, in points: the window less a margin up to
    /// this (docs/cover.md "Modal sheet"). AppKit's native sheet and the composed card share it.
    pub const SHEET_MAX: crate::Size = crate::Size::new(640.0, 620.0);
    /// The largest a [`CoverProps::large`] sheet grows (docs/cover.md "Large sheet").
    pub const LARGE_SHEET_MAX: crate::Size = crate::Size::new(1040.0, 880.0);

    /// A fullscreen cover's content container (docs/cover.md). Realized detached and hidden;
    /// `CoverPatch::Present` shows it over the whole window.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct CoverProps {
        /// Prefer a native sheet; toolkits without sheet presentation keep their modal cover.
        pub sheet: bool,
        /// A half-height sheet rising from the bottom edge, the window content above it
        /// staying on show and, where the platform allows, interactive: an inspector over the
        /// canvas it describes. Implies `sheet`. The toolkit reports the sheet's own content
        /// size through `FrameChanged`, as for any cover (docs/cover.md "Half sheet").
        pub half: bool,
        /// A large sheet: most of the window, less a margin, for a document or web page that
        /// needs the room (docs/cover.md "Large sheet"). Implies `sheet`.
        pub large: bool,
    }

    /// Applied to a `kinds::COVER` node as its bound signal opens and closes it.
    #[derive(Clone, Debug, PartialEq)]
    pub enum CoverPatch {
        /// Present the cover fullscreen (slide up where the platform animates modals).
        /// `background` paints the whole surface edge-to-edge (under the status bar / home
        /// indicator); the content container is inset to the safe area within it.
        /// `dismiss_disabled` = the user must not be able to dismiss it interactively
        /// (system back / sheet gestures); programmatic dismissal still works.
        Present {
            background: Option<Ink>,
            dismiss_disabled: bool,
        },
        /// The `interactive_dismiss_disabled` state changed while presented.
        DismissDisabled(bool),
        /// Dismiss the cover. When the hide transition finishes the backend emits
        /// [`crate::Event::CoverHidden`] on the node (trampoline backends send
        /// `BridgeKind::CoverHidden`), letting the piece dispose the content only after it
        /// left the screen.
        Dismiss,
    }

    /// A `kinds::INSPECTOR` split (docs/inspector.md): content beside a trailing inspector
    /// pane. The pane's visibility is Day-owned state (the piece's bound signal), so the
    /// props carry the value to draw and [`InspectorPatch::Visible`] keeps it current;
    /// a native affordance hiding the pane (a dock close button, a dragged-shut divider)
    /// reports back with [`crate::Event::InspectorChanged`].
    #[derive(Clone, Debug, PartialEq)]
    pub struct InspectorProps {
        /// Whether the pane is showing now.
        pub visible: bool,
        /// The pane's preferred width in points. A backend with a user-draggable divider
        /// treats it as the initial width; one without draws it at exactly this.
        pub width: f64,
        /// Which side of the content the pane sits on. `Trailing` is the classic inspector;
        /// `Leading` is a utility pane like a layer panel (docs/tree.md).
        pub edge: PaneEdge,
    }

    /// Which side an inspector pane occupies (docs/inspector.md).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum PaneEdge {
        #[default]
        Trailing,
        Leading,
    }

    /// Applied to a `kinds::INSPECTOR` node as its bound signal changes.
    #[derive(Clone, Debug, PartialEq)]
    pub enum InspectorPatch {
        /// Show or hide the pane (animated where the platform animates panes). The
        /// programmatic-sync direction: applying it must not re-emit
        /// [`crate::Event::InspectorChanged`], per the from-native echo rule.
        Visible(bool),
    }

    /// One pane of a `kinds::INSPECTOR` split (docs/inspector.md).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct InspectorPaneProps {
        /// `true` for the inspector panel itself, `false` for the content pane.
        pub panel: bool,
    }

    /// How a `kinds::SPLIT` lays its two panes (docs/split.md): beside each other, the divider
    /// standing, or stacked, the divider lying. Named by where the panes go, since that is
    /// what an app's "layout" toggle means; the divider runs the other way.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum SplitAxis {
        /// The panes side by side (first leading), the divider vertical.
        #[default]
        Horizontal,
        /// The panes stacked (first on top), the divider horizontal.
        Vertical,
    }

    /// A `kinds::SPLIT` (docs/split.md): two panes with a divider the user drags. The first
    /// pane's share is Day-owned state (the piece's bound signal): the props carry the value
    /// to draw, [`SplitPatch::Fraction`] keeps it current, and a divider the user moved
    /// reports back as [`crate::Event::ValueChanged`] with the new share, which applying the
    /// patch must never re-emit (the from-native echo rule).
    #[derive(Clone, Debug, PartialEq)]
    pub struct SplitProps {
        pub axis: SplitAxis,
        /// The first pane's share of the split's length along `axis`, `0.0..=1.0`.
        pub fraction: f64,
        /// The least length either pane may be dragged to, in points.
        pub min_pane: f64,
    }

    /// Applied to a `kinds::SPLIT` node as its bound signals change.
    #[derive(Clone, Debug, PartialEq)]
    pub enum SplitPatch {
        /// Turn the split the other way, keeping the share.
        Axis(SplitAxis),
        /// Move the divider so the first pane takes this share. The programmatic-sync
        /// direction: applying it must not re-emit [`crate::Event::ValueChanged`].
        Fraction(f64),
    }

    /// One pane of a `kinds::SPLIT` (docs/split.md).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct SplitPaneProps {
        /// `false` for the first pane, `true` for the second.
        pub second: bool,
    }

    /// Native navigation item list. `items` are display titles in route order;
    /// `selected` highlights the active route (split presentation; None on mobile roots).
    /// `icons` (parallel to `items`, `None` = no icon) are bundled image names resolved by each
    /// backend via `resource::resolve_image_file`; a backend that can't decorate its rows just
    /// ignores them.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavMenuProps {
        /// Stable document identities, parallel to items; empty for ordinary navigation.
        pub document_keys: Vec<String>,
        pub items: Vec<String>,
        pub icons: Vec<Option<String>>,
        /// A trailing accessory per row: an unread count, a status. Rendered right-aligned and
        /// de-emphasized, opposite the label. Parallel to `items`; `None` draws nothing.
        pub badges: Vec<Option<String>>,
        /// A trailing symbol accessory per row, in the same slot as [`Self::badges`] and drawn
        /// after it: a bundled image name resolved the way [`Self::icons`] is. This is what a
        /// row-level status glyph rides on (a starred page's star) where `badges` carries only
        /// text. Parallel to `items`; `None` draws nothing.
        pub badge_icons: Vec<Option<String>>,
        /// The tint for [`Self::badge_icons`], parallel to `items`. `None` leaves the glyph at
        /// the backend's neutral template tint; a status glyph that means something (a yellow
        /// star) names its own color here, exactly as [`Self::tints`] does for the leading icon.
        pub badge_tints: Vec<Option<Ink>>,
        /// A per-row icon tint (docs/vectors.md): the row's glyph recolored to this instead of
        /// the backend's neutral template tint. Parallel to `items`; `None` keeps the default.
        pub tints: Vec<Option<Ink>>,
        /// A section header introducing the row at the same index. `Some` opens a new group
        /// before that row; `None` continues the current one. Parallel to `items`, so adding a
        /// header never shifts the selection indices the rows are addressed by.
        pub sections: Vec<Option<String>>,
        /// A per-row context menu (docs/menus.md): shown on secondary-click / long-press on
        /// that row, the same [`crate::MenuItem`] model as everywhere else: items carry
        /// registered action ids, so a chosen entry dispatches [`crate::Event::MenuAction`]
        /// exactly like a piece context menu. Parallel to `items`; empty = no menu.
        pub menus: Vec<Vec<crate::MenuItem>>,
        /// Which rows drag to reorder (docs/navigation.md "Reordering sidebar destinations"),
        /// parallel to `items`; empty when the sidebar does not reorder. A row moves only
        /// within the run of movable rows around it, which a section header or a fixed row
        /// ends. A toolkit whose `Cap::NavReorder` is `Native` drags the rows in its own
        /// sidebar and reports a drop as [`crate::Event::ListReorder`] in item indices: `from`
        /// is the moved row, `to` the row whose position it takes, in the order before the move.
        pub movable: Vec<bool>,
        /// The sidebar regroups (docs/navigation.md "Moving rows between sections"): a movable
        /// row may also cross section headers, into any section whose rows are all movable. A
        /// toolkit whose `Cap::NavReorder` is `Native` offers those drops too and reports every
        /// drop as [`crate::Event::NavRegroup`] instead of [`crate::Event::ListReorder`]. `false`
        /// keeps [`Self::movable`]'s run rule.
        pub regroup: bool,
        /// Which movable rows keep the app's sorted order within their own section, parallel to
        /// `items`; empty when the sidebar does not regroup. Under [`Self::regroup`], a sorted
        /// row drops only into another section: no gap in its own section is offered.
        pub sorted: Vec<bool>,
        pub selected: Option<usize>,
    }
    #[derive(Clone, Debug, PartialEq)]
    // `Items` carries every row array; a patch travels boxed (`Box<dyn Any>`) and lives for one
    // update, so the variants' size difference costs nothing worth an indirection.
    #[allow(clippy::large_enum_variant)]
    pub enum NavMenuPatch {
        /// Active icon overlays by row index: None spins, Some(0..=1) fills a circle.
        /// Rows absent from this complete snapshot have no overlay. Does not relayout rows.
        /// Currently rendered by AppKit/UIKit; other backends may ignore this decoration.
        IconProgress(Vec<(usize, Option<f64>)>),
        /// Programmatic highlight sync: toolkits apply without re-emitting
        /// `SelectionChanged` (the `TextField` `from_native` echo rule).
        Selected(Option<usize>),
        /// The item set changed (data-driven `nav().items(signal, …)`): rebuild the rows
        /// from these labels/icons, then apply `selected` (docs/navigation.md). Applied without
        /// re-emitting `SelectionChanged`.
        ///
        /// `rows_changed` is `Some` when the rows themselves are the ones already shown (same
        /// keys, sections and row menus) and only these rows' title, icon, tint or badge moved:
        /// a count ticking, a language change. A backend updates those rows in place (a tab's
        /// badge, a menu item's title) rather than rebuilding, which on most platforms re-lays
        /// the bar and replays its selection animation; one that cannot rebuilds as for `None`.
        Items {
            rows_changed: Option<Box<[usize]>>,
            document_keys: Box<[String]>,
            items: Vec<String>,
            icons: Vec<Option<String>>,
            badges: Vec<Option<String>>,
            badge_icons: Vec<Option<String>>,
            badge_tints: Vec<Option<Ink>>,
            sections: Vec<Option<String>>,
            tints: Vec<Option<Ink>>,
            menus: Vec<Vec<crate::MenuItem>>,
            /// [`NavMenuProps::movable`] for the new rows.
            movable: Vec<bool>,
            /// [`NavMenuProps::sorted`] for the new rows ([`NavMenuProps::regroup`] holds for
            /// the sidebar's life).
            sorted: Vec<bool>,
            selected: Option<usize>,
        },
    }

    /// How a recycling list sizes its rows (docs/list.md).
    #[derive(Clone, Copy, Debug, PartialEq, Default)]
    pub enum RowHeight {
        /// Every row is this tall, a true layout boundary; the fastest path.
        Uniform(f64),
        /// Rows self-size from their content (host re-measures on change; slower).
        #[default]
        Automatic,
    }

    /// Configure a navigation sidebar's native pull-to-refresh control. Not a synthetic pan:
    /// supported toolkits own gesture arbitration and the progress indicator. A false state
    /// ends a pull, including a request the application declines. Never emits an event.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct RefreshPatch {
        pub refreshing: bool,
    }

    /// Native recycling list (docs/list.md). The host owns scrolling + cell reuse; Day supplies
    /// row content on demand through the injected `ListSource` (see `Toolkit::attach_list`).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ListProps {
        /// Whether the app handles row activation. Do not claim activation keys when false.
        pub activatable: bool,
        pub row_height: RowHeight,
        /// Whether the native list reports row selection (`Event::SelectionChanged` with the row).
        pub selectable: bool,
        /// Whether the native list allows selecting several rows at once. Where honored
        /// (docs/list.md has the matrix), every selection change reports the full set via
        /// `Event::SelectionSet`; single-selection backends keep reporting
        /// `Event::SelectionChanged` and treat this as `selectable`.
        pub multi_select: bool,
        /// Whether rows can be drag-reordered. The toolkit enables its native drag machinery and
        /// drives `ListSource::reorder` (validate via `can_move`, commit via `move_row`);
        /// probe `Cap::ListReorder` for per-backend support (docs/list.md).
        pub reorderable: bool,
        /// Whether rows can be deleted by the platform's own delete gesture: a trailing swipe on
        /// iOS and ArkUI, an `ItemTouchHelper` swipe on Android. The toolkit drives
        /// `ListSource::delete` (offer via `can_delete`, commit via `delete_row`); probe
        /// `Cap::ListDelete` for per-backend support (docs/list.md). Desktop toolkits have no such
        /// gesture and answer `Unsupported`; an app that must delete everywhere pairs this with an
        /// explicit control (a menu item, a button) rather than relying on the swipe.
        pub deletable: bool,
        /// The label on that affordance, already localized by the app. A toolkit cannot invent
        /// translated text (it has no access to the app's catalog), so the string arrives with
        /// the props. Empty means "no text": each backend falls back to its platform's own
        /// wordless idiom (a trash glyph), which works in every language.
        pub delete_label: String,
        /// The rows offer edge swipe actions (docs/list.md) through the injected
        /// `ListSource::swipe` (offer via `actions_at`, commit via `perform`); probe
        /// [`crate::Cap::ListSwipeActions`] for per-backend support. A backend without the
        /// affordance never shows one; an app that must offer the command everywhere
        /// pairs the swipe with an explicit control, exactly as `deletable` documents.
        pub swipe_actions: bool,
        /// Row separators, drawn by the host at the row boundary (docs/list.md), so they
        /// align with the native selection and ride the platform's own row animations (a
        /// macOS/iOS swipe slides the row content past a stationary separator, exactly as
        /// Mail's does). `None` keeps each platform's own default (iOS draws them, the
        /// desktops don't); `Some(true)`/`Some(false)` forces. A backend without a separator
        /// mechanism ignores a force (the matrix in docs/list.md says which); rows there
        /// separate by their pitch alone.
        pub separators: Option<bool>,
        /// The list starts in the platform's editing mode (docs/list.md): delete controls on
        /// the rows `deletable` offers, reorder grips on the rows `reorderable` moves, and a
        /// tap that still activates. Later changes arrive as [`ListPatch::Editing`]. Probe
        /// [`crate::Cap::ListEditing`]; a backend without an editing mode ignores it.
        pub editing: bool,
        /// The rows fall into sections, each opened by a header row (docs/list.md
        /// "Sections"): `ListSource::is_header` says which rows. Probe
        /// [`crate::Cap::ListSections`] for whether headers float natively.
        pub sections: bool,
        /// Draw the rows as the platform's inset grouped list (docs/list.md "Appearance"):
        /// rounded groups set in from the edges on a grouped background, as iOS Settings and
        /// Safari's History draw them. ios-uikit (`UITableViewStyleInsetGrouped`) and
        /// macos-appkit (`NSTableViewStyleInset`); the other toolkits draw their plain list.
        pub inset_grouped: bool,
    }

    /// One row-level result-set change. Indexes are sequential (each delta describes the
    /// set as the previous ones left it), so a host applies them one at a time (or falls back
    /// to a reload; the source's snapshot already holds the final state either way).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub enum RowDelta {
        Insert(usize),
        Remove(usize),
        Move(usize, usize),
    }

    /// Immutable-at-realize props of a `kinds::TREE` host (docs/tree.md). Everything that
    /// changes after realize flows through [`TreePatch`] or the injected `TreeSource`.
    #[derive(Clone, Debug, PartialEq)]
    pub struct TreeProps {
        /// Two-level sectioned list: root branches are non-selectable headings.
        pub section_headers: bool,
        pub row_height: RowHeight,
        /// Rows highlight and report selection ([`crate::Event::TreeSelection`]).
        pub selectable: bool,
        /// Several rows may be selected at once; every change reports the full token set.
        pub multi_select: bool,
        /// Rows drag to a new parent/position through the platform's own mechanism, driving
        /// `TreeSource::moves`. Probe [`crate::Cap::TreeMove`] for support.
        pub movable: bool,
        /// Indentation per depth level, in points. `None` = the platform's default step.
        pub indent: Option<f64>,
    }

    /// Post-realize changes to a `kinds::TREE` host (docs/tree.md). Token-addressed
    /// throughout; see [`crate::TreeSource`] for why indices cannot address tree rows.
    #[derive(Clone, Debug, PartialEq)]
    pub enum TreePatch {
        /// The node set changed (count/order/parentage/content): the host re-queries its
        /// `TreeSource`. Expansion and selection survive by token (docs/tree.md).
        Reload,
        /// Programmatically disclose (`true`) or collapse (`false`) one row. Applied without
        /// re-emitting [`crate::Event::TreeExpanded`] (the from-native echo rule).
        Expand(u64, bool),
        /// Programmatic selection sync (tokens; empty = clear), applied without re-emitting
        /// [`crate::Event::TreeSelection`].
        Selected(Vec<u64>),
        /// Scroll this row into view, realizing it if needed. The piece has already expanded
        /// the row's ancestors (through the app's expansion signal) before issuing this.
        Reveal(u64),
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum ListPatch {
        /// The row set changed (count/order/content): the host re-queries its `ListSource`.
        Reload,
        /// The row set changed by exactly these deltas: a host that can, animates each row
        /// in, out, or across; one that cannot treats this as [`ListPatch::Reload`]. The
        /// data source answers with the final state throughout.
        Splice(Vec<RowDelta>),
        /// An `Automatic`-height row was re-laid at a new height after its first layout (its
        /// text changed, a wrapped line appeared, the backend corrected its width): the host
        /// gives the row bound to physical cell `cell` (the handle `ListSource::bind_row` got,
        /// as `usize`) the new `height`, padding included, the same value
        /// `ListSource::cell_height` now answers for it.
        ///
        /// Sent from inside a day-core layout pass, with the tree borrowed: a host applies it
        /// after the pass (posted to its main loop), never by asking the source anything
        /// synchronously, which would read a `0.0` height (docs/list.md "Row heights").
        RowSizeInvalidated { cell: usize, height: f64 },
        /// Imperatively scroll the native list so its last row is fully visible (a chat timeline
        /// sticking to the newest message). No-op when the list is empty (docs/list.md).
        ScrollToEnd,
        /// Imperatively scroll the native list so this row is visible, realizing it if needed
        /// (docs/list.md). Clamped to the row count; no-op when the list is empty.
        ScrollToRow(usize),
        /// Programmatic selection sync (row indices; empty = clear): toolkits apply without
        /// re-emitting a selection event, like every other programmatic sync.
        Selected(Vec<usize>),
        /// Enter (`true`) or leave (`false`) the platform's list editing mode, animated
        /// (docs/list.md, [`ListProps::editing`]). Ignored where `Cap::ListEditing` is
        /// `Unsupported`.
        Editing(bool),
    }
}

// ---------------------------------------------------------------------------
// Imperative presentation (docs/dialogs.md)
// ---------------------------------------------------------------------------

/// Sharing (docs/sharing.md): what [`Toolkit::share`] presents.
pub mod share {
    use crate::Symbol;

    /// A share request: the page or link to share, and the app's own actions to offer beside
    /// the system's targets.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ShareSpec {
        /// The URL handed to the chosen target.
        pub url: String,
        /// Its title: the subject of a mail, the header of the chooser where it shows one.
        pub title: String,
        /// The app's actions, in order (`Cap::ShareActions`).
        pub actions: Vec<ShareAction>,
    }

    /// One of the app's actions in a share chooser.
    #[derive(Clone, Debug, PartialEq)]
    pub struct ShareAction {
        /// The dispatch id a choice reports, as `Event::MenuAction(id)`.
        pub id: u64,
        /// The action's name, already localized.
        pub title: String,
        /// Its glyph, where the chooser draws one.
        pub symbol: Option<Symbol>,
    }
}

pub mod present {
    /// A dialog button's semantic role: styling + default/cancel placement on each toolkit.
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
    pub enum ButtonRole {
        #[default]
        Default,
        Cancel,
        Destructive,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub struct PresentButton {
        pub label: String,
        pub role: ButtonRole,
    }

    /// A named group of file extensions for a file dialog (e.g. "Text" → `["txt", "md"]`).
    /// An empty `extensions` list means "all files".
    #[derive(Clone, Debug, PartialEq, Default)]
    pub struct FileFilter {
        pub name: String,
        pub extensions: Vec<String>,
    }

    /// What a backend should present for a `req`. Kept toolkit-agnostic; the pieces layer
    /// maps a chosen button index back to a typed payload.
    #[derive(Clone, Debug, PartialEq)]
    pub enum PresentSpec {
        /// Alert / confirmation / action sheet: title + optional message + ordered buttons.
        /// `sheet` = present from the bottom on mobile (desktop falls back to an alert).
        Dialog {
            title: String,
            message: Option<String>,
            buttons: Vec<PresentButton>,
            sheet: bool,
        },
        /// A dialog with a single text field.
        Prompt {
            title: String,
            message: Option<String>,
            placeholder: String,
            initial: String,
            ok: String,
            cancel: String,
        },
        /// Native "open file" picker (docs/files.md). The backend must answer with
        /// `PresentResult::Files` whose entries are **readable local paths**: desktop returns
        /// the chosen path directly; iOS/Android copy the selection into app storage first, so
        /// the pieces layer can read it with `std::fs` regardless of platform. `multiple` lets
        /// the user pick several files at once; the answer then carries every chosen path, in
        /// the picker's order. A backend that cannot select more than one file still answers
        /// with the single pick.
        OpenFile {
            title: String,
            filters: Vec<FileFilter>,
            multiple: bool,
        },
        /// Native "choose folder" picker (docs/files.md). The backend answers with
        /// `PresentResult::Files` holding the one chosen directory path, or `Dismissed`. Only
        /// backends answering [`Cap::FolderDialog`](crate::Cap::FolderDialog) as `Native` show a
        /// chooser; the others resolve the request as `Dismissed` at once, so the await never hangs.
        OpenFolder { title: String },
        /// Native "save file" picker (docs/files.md). `src_path` is a Day-written temp file
        /// holding the bytes to save; iOS/Android deliver it to the chosen destination natively,
        /// and the pieces layer best-effort copies it to a chosen local path otherwise.
        SaveFile {
            title: String,
            suggested_name: String,
            src_path: String,
            filters: Vec<FileFilter>,
        },
    }

    /// The user's answer to a presentation.
    #[derive(Clone, Debug, PartialEq)]
    pub enum PresentResult {
        /// A dialog button at `index` (in spec order) was chosen.
        Button(i64),
        /// A prompt was confirmed with `text`.
        Text(String),
        /// One or more file locators chosen from an open/save picker (docs/files.md). Each is a
        /// local filesystem path or, on Android save, a `content://` URI.
        Files(Vec<String>),
        /// Dismissed without choosing (tap-outside / Esc / cancel gesture).
        Dismissed,
    }

    /// The unit-separator that joins string lists across the C ABI (Qt shim / Android JNI), the
    /// same encoding the nav menu, combobox, and dialog-button shims use.
    pub const UNIT_SEP: char = '\u{1f}';

    std::thread_local! {
        /// An app-writable scratch directory. Backends whose OS temp dir isn't app-writable
        /// (Android → `getCacheDir()`) set this at startup; elsewhere it stays `None` and callers
        /// fall back to `std::env::temp_dir()`.
        // False positive, and only when checking an android target: the initializer already is
        // the `const` block the lint asks for. Android's TLS model makes the macro expand to a
        // form clippy reads as lazy, so the suggestion is unfollowable as well as unneeded.
        #[allow(clippy::missing_const_for_thread_local)]
        static APP_TEMP_DIR: std::cell::RefCell<Option<std::path::PathBuf>> =
            const { std::cell::RefCell::new(None) };
    }

    /// Record an app-writable scratch directory (see [`app_temp_dir`]). Called by a backend at
    /// startup when the OS temp dir isn't writable by the app (Android).
    pub fn set_app_temp_dir(dir: impl Into<std::path::PathBuf>) {
        APP_TEMP_DIR.with(|d| *d.borrow_mut() = Some(dir.into()));
    }

    /// An app-writable scratch directory: the backend-supplied one, else `std::env::temp_dir()`.
    /// Used by the file-save flow (docs/files.md) to stage bytes before the native save picker.
    #[must_use]
    pub fn app_temp_dir() -> std::path::PathBuf {
        APP_TEMP_DIR.with(|d| {
            d.borrow().clone().unwrap_or_else(|| {
                #[cfg(target_arch = "wasm32")]
                {
                    // A browser has no filesystem and std's `temp_dir()` panics on wasm. A
                    // nominal path keeps join/display plumbing alive; actual reads and writes
                    // fail with ordinary io errors the file flows already surface.
                    std::path::PathBuf::from("/day-tmp")
                }
                #[cfg(not(target_arch = "wasm32"))]
                std::env::temp_dir()
            })
        })
    }

    /// The web build's file bytes (wasm32-unknown-unknown only): a browser has no filesystem,
    /// so the open/save flows carry bytes through this per-page store instead of `std::fs`:
    /// day-toolkit-dom writes a picked file's bytes here and answers with its virtual path; the pieces
    /// layer stages save bytes here for the shim to download. Paths are plain strings under
    /// `/day-web/`; lifetimes are one picker round-trip (the flows remove what they stage).
    #[cfg(all(target_family = "wasm", target_os = "unknown"))]
    pub mod web_files {
        std::thread_local! {
            static FILES: std::cell::RefCell<std::collections::HashMap<String, Vec<u8>>> =
                std::cell::RefCell::new(std::collections::HashMap::new());
        }

        /// Store `bytes` under `path` (replacing any prior content).
        pub fn write(path: &str, bytes: Vec<u8>) {
            FILES.with(|f| f.borrow_mut().insert(path.to_string(), bytes));
        }

        /// The bytes at `path`, if present.
        #[must_use]
        pub fn read(path: &str) -> Option<Vec<u8>> {
            FILES.with(|f| f.borrow().get(path).cloned())
        }

        /// Drop `path`'s bytes.
        pub fn remove(path: &str) {
            FILES.with(|f| {
                f.borrow_mut().remove(path);
            });
        }
    }

    impl PresentResult {
        /// Flat wire tag for the C ABI (Qt shim / Android JNI): 0 dismissed, 1 button, 2 text,
        /// 3 files (`text` is the chosen locators joined by the unit separator).
        pub fn decode(tag: i32, index: i64, text: String) -> PresentResult {
            match tag {
                1 => PresentResult::Button(index),
                2 => PresentResult::Text(text),
                3 => PresentResult::Files(
                    text.split(UNIT_SEP)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                        .collect(),
                ),
                _ => PresentResult::Dismissed,
            }
        }
    }

    impl PresentSpec {
        /// Backend-facing flattening for the C ABI: `(title, message, button labels, button
        /// roles as ints, sheet-or-prompt fields)`. Pure-Rust backends read the enum directly.
        #[must_use]
        pub fn title(&self) -> &str {
            match self {
                PresentSpec::Dialog { title, .. }
                | PresentSpec::Prompt { title, .. }
                | PresentSpec::OpenFile { title, .. }
                | PresentSpec::OpenFolder { title }
                | PresentSpec::SaveFile { title, .. } => title,
            }
        }
        #[must_use]
        pub fn message(&self) -> Option<&str> {
            match self {
                PresentSpec::Dialog { message, .. } | PresentSpec::Prompt { message, .. } => {
                    message.as_deref()
                }
                _ => None,
            }
        }
        /// Button labels joined with [`UNIT_SEP`], the encoding the nav menu and combobox
        /// shims already use for string lists.
        #[must_use]
        pub fn buttons_joined(&self) -> String {
            match self {
                PresentSpec::Dialog { buttons, .. } => buttons
                    .iter()
                    .map(|b| b.label.as_str())
                    .collect::<Vec<_>>()
                    .join(&UNIT_SEP.to_string()),
                PresentSpec::Prompt { ok, cancel, .. } => format!("{ok}{UNIT_SEP}{cancel}"),
                _ => String::new(),
            }
        }
        /// Button roles as ints (0 default, 1 cancel, 2 destructive), joined with commas.
        #[must_use]
        pub fn roles_joined(&self) -> String {
            let roles: Vec<i32> = match self {
                PresentSpec::Dialog { buttons, .. } => {
                    buttons.iter().map(|b| b.role as i32).collect()
                }
                PresentSpec::Prompt { .. } => {
                    vec![ButtonRole::Default as i32, ButtonRole::Cancel as i32]
                }
                _ => vec![],
            };
            roles
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(",")
        }

        // --- file-dialog accessors (docs/files.md) ---

        /// The file filters, if this is a file dialog.
        #[must_use]
        pub fn filters(&self) -> &[FileFilter] {
            match self {
                PresentSpec::OpenFile { filters, .. } | PresentSpec::SaveFile { filters, .. } => {
                    filters
                }
                _ => &[],
            }
        }
        /// Whether an open-file dialog lets the user pick several files (`false` for every
        /// other spec).
        #[must_use]
        pub fn multiple(&self) -> bool {
            matches!(self, PresentSpec::OpenFile { multiple: true, .. })
        }
        /// The suggested file name for a save dialog (empty otherwise).
        #[must_use]
        pub fn suggested_name(&self) -> &str {
            match self {
                PresentSpec::SaveFile { suggested_name, .. } => suggested_name,
                _ => "",
            }
        }
        /// The Day-written temp source path for a save dialog (empty otherwise).
        #[must_use]
        pub fn src_path(&self) -> &str {
            match self {
                PresentSpec::SaveFile { src_path, .. } => src_path,
                _ => "",
            }
        }
        /// Filters flattened for the C ABI: each filter is `name|ext1,ext2`, joined by
        /// [`UNIT_SEP`]. A trailing `|` (no extensions) means "all files". Empty when unfiltered.
        #[must_use]
        pub fn filters_joined(&self) -> String {
            self.filters()
                .iter()
                .map(|f| format!("{}|{}", f.name, f.extensions.join(",")))
                .collect::<Vec<_>>()
                .join(&UNIT_SEP.to_string())
        }
    }
}

// ---------------------------------------------------------------------------
// The Toolkit trait (§8.1)
// ---------------------------------------------------------------------------

/// Timing supplied by a native display scheduler. Seconds on a monotonic clock; its epoch
/// is platform-specific. `target_timestamp` is absent when the platform cannot predict presentation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FrameStamp {
    pub timestamp: f64,
    pub target_timestamp: Option<f64>,
}
impl FrameStamp {
    #[must_use]
    pub fn new(timestamp: f64) -> Self {
        Self {
            timestamp,
            target_timestamp: None,
        }
    }
}
/// A one-shot native frame callback, always delivered asynchronously on the UI thread.
pub type FrameCallback = Box<dyn FnOnce(FrameStamp)>;
/// Cancels a pending callback and releases its native registration. UI-thread only, idempotent
/// in effect, and safe during/after delivery. Dropping without calling is allowed after delivery.
pub type CancelFrame = Box<dyn FnOnce()>;

/// The margins and minimum height a native list gives each row ([`Toolkit::list_row_metrics`],
/// docs/list.md "Row metrics"): a `UITableView` cell's layout margins, a Material list item's
/// 16dp sides and 56dp height.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RowMetrics {
    /// Space between the row's edges and its content.
    pub insets: Insets,
    /// The least height a row takes, margins included; content centers vertically in it.
    pub min_height: f64,
}

pub trait Toolkit: Sized + 'static {
    // `'static` so a handle clone can cross the object-safe TreeOps boundary boxed as `Any`
    // (`node_handle_any`, the tweaks door, docs/tweaks.md).
    type Handle: Clone + 'static;

    /// Request one frame for the window containing `host`. Never call inline. Each native source
    /// must stop after delivery unless requested again; no polling timer. Hidden/unmapped windows
    /// may defer delivery until visible. Native callbacks must reach the UI thread before calling
    /// Rust. The returned closure cancels safely, including when the window has closed.
    /// The mock intentionally stays pending: tests inject a manual scheduler in day-core.
    fn request_frame(&mut self, _host: &Self::Handle, _cb: FrameCallback) -> CancelFrame {
        Box::new(|| {})
    }

    fn capability(&self, _cap: Cap) -> Support {
        Support::Unsupported
    }

    // node lifecycle
    fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Self::Handle;
    fn update(
        &mut self,
        h: &Self::Handle,
        kind: PieceKind,
        patch: &dyn Any,
        anim: Option<&AnimSpec>,
    );
    /// Called from the turn-boundary release queue; backends may defer destruction further.
    fn release(&mut self, h: Self::Handle);
    /// Give a satellite piece a chance to drop its own per-view state, immediately before
    /// [`release`](Toolkit::release) frees `h` (§15.2).
    ///
    /// A backend that hosts third-party pieces implements this by looking `kind` up in its renderer
    /// registry and calling that piece's `release` hook. Defaulted to nothing: a backend with no
    /// registry has nothing to dispatch, and one that has not wired this yet leaves piece
    /// teardown unrun rather than failing to build.
    fn release_piece(&mut self, _kind: PieceKind, _h: &Self::Handle) {}

    // tree
    fn insert(&mut self, parent: &Self::Handle, child: &Self::Handle, index: usize);
    fn remove(&mut self, parent: &Self::Handle, child: &Self::Handle);
    /// Put `child` at index `to` among `parent`'s native children: day's z-order resync, which
    /// walks the whole sibling row and asks for every position in turn. A child already at `to`
    /// must therefore cost nothing: a backend that re-parents unconditionally pays for the
    /// untouched siblings too, and where re-parenting means detach-then-add (no `ViewGroup` move
    /// primitive on Android) the detach resigns the focus of everything inside the subtree,
    /// which is how a text field being typed into loses the keyboard (docs/focus.md).
    fn move_child(&mut self, parent: &Self::Handle, child: &Self::Handle, to: usize);

    // geometry (§7): frames are in the nearest realized native ancestor's space, in points.
    fn measure(&mut self, h: &Self::Handle, kind: PieceKind, p: Proposal) -> Size;
    /// Distance from the top of the widget's frame to its first text baseline, in points, when
    /// the widget is `size` (docs/baseline.md). `None` ⇒ it has no text baseline (an image, a
    /// slider, a bare container) and a baseline-aligned row falls back to box alignment for it.
    ///
    /// This is a measurement, not a layout mode: day places every frame itself (§7.1) and never
    /// hands a row to a native baseline-aligning container, so what it needs from the toolkit is
    /// where the text sits inside the box. Backends that cannot answer keep the default and rows
    /// stay centered, which is what every row did before this existed.
    fn first_baseline(&mut self, _h: &Self::Handle, _kind: PieceKind, _size: Size) -> Option<f64> {
        None
    }
    fn set_frame(&mut self, h: &Self::Handle, frame: Rect, anim: Option<&AnimSpec>);

    // animatable visual channels (§8.4): cheap per-node opacity + transform that don't relayout.
    // Defaulted no-ops so backends adopt them incrementally; `anim = Some` ⇒ animate to the value
    // on the toolkit's own compositor, `None` ⇒ set instantly.
    fn set_opacity(&mut self, _h: &Self::Handle, _opacity: f64, _anim: Option<&AnimSpec>) {}
    fn set_transform(
        &mut self,
        _h: &Self::Handle,
        _t: Transform,
        _size: Size,
        _anim: Option<&AnimSpec>,
    ) {
    }

    // text selection (docs/text.md): make this node's text user-selectable (copy/drag). Applied
    // once from the `.selectable()` modifier (day-pieces) to the widget it wraps (a `label`'s
    // native text view, most usefully). Unmanaged: Day sets it here and never patches it, so it
    // survives text updates. The default no-op means a backend without a selection affordance
    // silently leaves the text unselectable.
    //
    // Returns `Some(replacement)` when the toolkit had to rebuild the widget as a different
    // native class to gain a selection affordance (UIKit: `UILabel` has none, so the label
    // becomes a read-only `UITextView`); day-core re-points the node's handle at the
    // replacement, so later patches and layout reach the widget that is actually on screen.
    // Every property-flip backend returns `None`.
    fn set_selectable(&mut self, _h: &Self::Handle, _selectable: bool) -> Option<Self::Handle> {
        None
    }

    /// Give one widget its reading direction (the `.layout_direction()` modifier,
    /// docs/localization.md "Direction of a subtree"). day-core calls this for every realized
    /// node inside a subtree whose direction differs from the app's, and again with the app's
    /// direction when an override is lifted, so the setter is idempotent.
    ///
    /// Day mirrors the frames itself, so this is about what happens inside the widget: a
    /// label's or field's base writing direction and which edge `TextAlign::Leading` means, the
    /// side a slider fills from, which way a disclosure chevron points, where a list row puts
    /// its accessory. It must never move a child Day has placed. Where the platform's
    /// direction property also mirrors child coordinates (XAML `FlowDirection` on a panel,
    /// Android `layoutDirection` on a view group), apply it to leaf controls only. Do not rely
    /// on native inheritance: Day calls this per node.
    ///
    /// The default does nothing, which leaves the widget in the app's direction.
    fn set_layout_direction(
        &mut self,
        _h: &Self::Handle,
        _kind: PieceKind,
        _direction: LayoutDirection,
    ) {
    }

    // text entry (docs/textfield.md): how a `text_field` takes its text: hidden characters,
    // read-only, the keyboard and autofill purpose, the action key, the length bound. Called
    // once after `realize` when a field asks for anything but the defaults, and again with the
    // whole set whenever a reactive member changes, so the setter is idempotent.
    //
    // Returns `Some(replacement)` when hiding the characters takes a different native class
    // (AppKit: `NSSecureTextField`; WinUI: `PasswordBox`), the same contract as
    // `set_selectable`: the backend carries the text, the placeholder and its own event wiring
    // over, takes the old widget's place in the native tree, and day-core re-points the node's
    // handle. Required, with no default: a backend that left this out would show a password in
    // the clear. A member the toolkit has no notion of (an action key on a desktop) is ignored.
    fn set_input_traits(&mut self, h: &Self::Handle, traits: &InputTraits) -> Option<Self::Handle>;

    // pointer shape (docs/cursor.md): the cursor to show while the pointer is over this node
    // and its descendants, from the `.cursor()` modifier. Called again whenever a reactive
    // cursor changes, so the setter is idempotent and cheap; `Cursor::Default` releases the
    // node. The default no-op is a backend with no pointer API to reach (`Cap::Cursor`
    // answers `Unsupported` there).
    fn set_cursor(&mut self, _h: &Self::Handle, _cursor: Cursor) {}

    /// Show or hide a retained subtree, including its input and accessibility participation.
    /// The layout and reactive scope stay alive while hidden.
    fn set_presented(&mut self, _h: &Self::Handle, _visible: bool) {}

    /// Attach overscroll delivery to a refresh host; the caller owns the mounted scope.
    fn attach_refresh(&mut self, _h: &Self::Handle, _host: NodeId) {}

    // scroll (§7.6)
    fn set_scroll_content(&mut self, _h: &Self::Handle, _content: Size) {}
    // The room a classic (non-overlay) scroll bar takes out of a scroll view's content area
    // while the content overflows along `vertical`'s axis: its width for a vertical scroll, its
    // height for a horizontal one (docs/scroll.md). Day lays overflowing content out that much
    // narrower, so it fits beside the bar instead of reaching under it. 0 — the default — is
    // every backend whose bars overlay the content (AppKit, GTK, UIKit, Android, XAML, DOM).
    fn scroll_bar_inset(&mut self, _h: &Self::Handle, _vertical: bool) -> f64 {
        0.0
    }
    // The margins and minimum height the platform's own list gives a row (docs/list.md "Row
    // metrics"), asked of list `h` each time day-core lays a row. Day insets the row's content by
    // the margins and keeps it at least that tall, so a row reads at the platform's size without
    // the app padding it per platform. Zero — the default — leaves the row to its content.
    fn list_row_metrics(&mut self, _h: &Self::Handle) -> RowMetrics {
        RowMetrics::default()
    }
    fn scroll_to(&mut self, _h: &Self::Handle, _target: Rect, _animated: bool) {}
    /// Scroll a native navigation menu to its top or bottom without changing selection.
    /// The widget owns its content extent. Return false when the handle is unsupported.
    fn nav_scroll_to_edge(&mut self, _h: &Self::Handle, _bottom: bool) -> bool {
        false
    }
    fn scroll_offset(&mut self, _h: &Self::Handle) -> Point {
        Point::ZERO
    }
    /// Scroll list `h` to a row, or to its end, with the alignment and animation asked for
    /// (docs/list.md "Programmatic scrolling"). Answer `true` once handled. The default
    /// answers `false`, and day-core then sends the plain [`props::ListPatch::ScrollToRow`] or
    /// [`props::ListPatch::ScrollToEnd`] instead, which reveals the row wherever the platform
    /// puts it. A toolkit that answers `true` reports [`Cap::ListScrollAlign`] as `Native`.
    fn list_scroll(&mut self, _h: &Self::Handle, _to: ListScroll) -> bool {
        false
    }

    // events: one trampoline, node-id keyed; enqueue-only contract (§8.3).
    fn set_event_sink(&mut self, sink: EventSink);

    // gestures (docs/shapes.md): attach a native recognizer for `kind` to `h`, emitting
    // `Event::Tap/LongPress/Drag` for `node` (enqueue-only). Default no gesture; a piece opts in
    // when it has a handler. Idempotent per (handle, kind). A view no recognizer was attached
    // to takes no pointer input at all: a press on it reaches whatever is behind it
    // (docs/gestures.md "Pass-through").
    fn enable_gesture(&mut self, _h: &Self::Handle, _node: NodeId, _kind: GestureKind) {}

    // native recognizers (docs/gestures.md "Recognizers"): whether `enable_gesture(kind)` attaches
    // the platform's own recognizer. `Unsupported` for `LongPress`, `DoubleTap`, `Press` or
    // `Touches` makes day-core derive the gesture from the tap and drag events instead and
    // never ask for it; the kinds every backend has had from the start default to `Native`.
    fn gesture_support(&self, kind: GestureKind) -> Support {
        match kind {
            GestureKind::LongPress
            | GestureKind::DoubleTap
            | GestureKind::Press
            | GestureKind::Touches => Support::Unsupported,
            _ => Support::Native,
        }
    }

    // hit slop (docs/gestures.md "Hit slop"): grow the area in which `h` takes pointer input by
    // `slop` points on each edge, without moving or resizing it. Every recognizer on the view
    // (and a native control's own press) answers inside the grown area, clipped by nothing but
    // the window; a point two slop areas share goes to the view in front. Zero insets restore
    // the view's own bounds. Default no-op: the piece is hit inside its bounds only
    // (`Cap::HitSlop` answers `Unsupported`).
    fn set_hit_slop(&mut self, _h: &Self::Handle, _node: NodeId, _slop: Insets) {}

    // drag axis (docs/gestures.md "Gestures inside a scroll view"): the node's drag follows one
    // axis only. `Some(axis)`: a pointer sequence that starts out along the other axis is not
    // the node's drag, so an enclosing scroll view takes it and no `Event::Drag` is sent;
    // one that starts along `axis` is the node's from `Began` to `Ended` and the scroll view
    // stays still. `None` (the default state): the node claims a drag in any direction.
    // Default no-op: the backend claims every direction, and day-core still withholds the
    // off-axis drags from the handler.
    fn set_drag_axis(&mut self, _h: &Self::Handle, _node: NodeId, _axis: Option<DragAxis>) {}

    // subtree enablement (docs/forms.md "Disabling a group"): `enabled = false` disables every
    // native control under `h` (dimmed the platform's way, out of the focus order, reported
    // disabled to assistive technology) and stops pointer input to the subtree; `true`
    // restores each control to its own enabled state. day-core drops the Day events of a
    // disabled subtree on every backend, so the default no-op loses the native dimming and
    // the accessibility state only.
    fn set_subtree_enabled(&mut self, _h: &Self::Handle, _node: NodeId, _enabled: bool) {}

    // input kind (docs/gestures.md "Input kind"): the kind of device behind the latest pointer
    // input, asked once at startup; changes arrive as `Event::PointerKindChanged`.
    fn primary_pointer(&self) -> PointerKind {
        PointerKind::platform_default()
    }

    // focus (docs/focus.md): move native keyboard focus to (or away from) this control.
    // `focused = true` requests focus (on mobile this also raises the soft keyboard for text
    // inputs); `false` resigns it (dismissing the keyboard; platforms without a "focus nothing"
    // state resign to a focusable root). Backends report the resulting state, user- or
    // programmatic, with `Event::FocusChanged(bool)` through the sink; a request that cannot
    // be honored (unfocusable, unmounted) produces no event. The default no-op means a
    // backend without focus support neither moves nor reports focus.
    fn focus(&mut self, _h: &Self::Handle, _node: NodeId, _focused: bool) {}

    // focusability (docs/focus.md): opt this container into the platform's focus system, the
    // canvas contract generalized (`Decorate::focusable`). A focusable container accepts focus,
    // takes it on a press (before the press reaches its gesture recognizers), reports both
    // directions with `Event::FocusChanged`, and delivers the non-text keys it hears while
    // focused as `Event::Key` for `node` (gated on `keys::handled`, like a canvas). Keys carry
    // their phase: `KeyPhase::Repeat` for the platform's auto-repeat and, where
    // `Cap::KeyPhases` is supported, `KeyPhase::Up` for the release. `Cap::Focusable` says
    // whether the duty is implemented. The default
    // no-op means the piece renders normally but never joins the focus order: `.focused(…)`
    // bindings stay quiet and `.on_key(…)` never fires, the same graceful silence unfocusable
    // controls already have. Idempotent; only `true` is ever sent today (a piece opts in at
    // build, it does not opt back out).
    fn set_focusable(&mut self, _h: &Self::Handle, _node: NodeId, _focusable: bool) {}

    // recycling list (docs/list.md, §10): day-core hands the `LIST` host its row-pull `source`
    // once, right after realize. A recycling backend stores it and calls it from its native
    // data-source; the default no-op means a backend without list support renders nothing.
    fn attach_list(&mut self, _host: &Self::Handle, _source: ListSource) {}

    // hierarchical tree (docs/tree.md): day-core hands the `TREE` host its token-addressed
    // row-pull `source` once, right after realize. A backend with a native tree stores it and
    // answers its data-source from it; the default no-op means a backend without tree support
    // renders nothing (`Cap::Tree` answers `Unsupported`, and apps gate on that).
    fn attach_tree(&mut self, _host: &Self::Handle, _source: TreeSource) {}

    // routes (docs/navigation.md): day-core reports the app's current route path here whenever
    // it changes ("" = everything at its root), so a backend with a native notion of location
    // can mirror it: web-dom writes the URL hash (`#controls`), and browser back/forward or a
    // hand-edited hash comes back as `Event::RouteRequested`. Default no-op: most toolkits
    // have nowhere to put a route.
    fn set_route(&mut self, _route: &str) {}

    // undo bridge (docs/model.md): mirror the app's undo stack into the platform's own undo
    // objects, so the stock Edit menu retitles and enables itself and the platform's gestures
    // land: the native front of one Day-owned history. The state flows down whenever it
    // changes; the user's invocation comes back up as `Event::Undo`. Default no-op: a backend
    // without a native undo system answers `Cap::UndoBridge` `Unsupported`, and the app's own
    // affordances (menu items, buttons, accelerators) drive the stack instead.
    fn set_undo_state(&mut self, _state: &UndoState) {}

    // edit bridge (docs/menus.md): mirror what the app's standard-edit handlers can do, so
    // the platform's own menu validation enables Cut/Copy/Paste exactly as it does for text
    // widgets; invocations come back up as `Event::Edit`. Default no-op: a backend without
    // a native edit-command route answers `Cap::EditBridge` accordingly.
    fn set_edit_state(&mut self, _state: &EditState) {}

    // ambient modifiers (docs/menus.md): the keyboard modifiers held right now, for
    // interactions whose meaning they change (shift-click multi-select). Pull-based: the
    // platforms expose exactly this query (NSEvent.modifierFlags, the shim's tracked mask).
    // Touch-only backends keep the all-false default.
    fn modifiers(&mut self) -> Modifiers {
        Modifiers::default()
    }

    // Dynamic context menu (docs/menus.md): the menu is built when it is summoned. The provider
    // is called synchronously from the platform's own menu callback (UI thread, outside any
    // day-core borrow) with the location in the node's local coordinates, and its result is
    // shown. The static `set_context_menu` below stays the simple path; this one exists for
    // surfaces whose menu depends on what is under the pointer (a canvas selection,
    // docs/tree.md). Default no-op: a backend without the affordance shows nothing.
    fn set_drag_source(&mut self, _h: &Self::Handle, _source: transfer::Source) {}
    fn set_drop_target(&mut self, _h: &Self::Handle, _target: transfer::Target) {}

    fn set_context_menu_fn(&mut self, _h: &Self::Handle, _node: NodeId, _f: ContextMenuFn) {}

    // menus (§ menus): render `items` with the backend's native menu affordance, firing
    // `Event::MenuAction(id)` (enqueue-only) for each id'd item; `role` items use the native standard
    // command. Default no-op: a toolkit without a menu bar / context menu shows nothing.
    /// The application menu (macOS/Windows/Linux menu bar; the app-bar overflow on Android; the
    /// `UIMenuBuilder` main menu on iPadOS/Catalyst). Replaces any previous app menu.
    fn set_app_menu(&mut self, _items: &[MenuItem]) {}
    /// A context menu for `h`, shown on secondary-click (desktop) or long-press (mobile). Passing an
    /// empty slice removes it.
    fn set_context_menu(&mut self, _h: &Self::Handle, _node: NodeId, _items: &[MenuItem]) {}
    /// Make the button `h` (a [`kinds::BUTTON`] node) a pull-down menu button (docs/menus.md
    /// "Menu buttons"): its primary press (a click, a tap, the keyboard's Space or Return, the
    /// accessibility press) asks `f` for the menu at that moment and shows it under the button
    /// with the toolkit's own menu, instead of reporting [`Event::Pressed`]. `f` is called on
    /// the UI thread, outside any day-core borrow, with [`Point::ZERO`] (where on the button
    /// the press landed does not change its menu); an empty result shows nothing. Chosen items
    /// fire [`Event::MenuAction`] as a context menu's do.
    ///
    /// A toolkit with no native menu to show reports [`Event::ContextMenu`] for `node` instead,
    /// with `window` at the button's bottom-leading corner, and the pieces layer presents the
    /// composed menu there. Called once per realized button; the release sweep reclaims `f`.
    /// Resolve the press through [`resolve_button_menu`], which re-checks after `f` runs that
    /// the button was not released by it. Default no-op: the button stays an inert button.
    fn set_button_menu(&mut self, _h: &Self::Handle, _node: NodeId, _f: ContextMenuFn) {}

    // toolbars (docs/toolbars.md): a window's native toolbar: NSToolbar, AdwHeaderBar, QToolBar,
    // CommandBar. `h` is the window root's handle (the same handle `open_window` returned, or the
    // primary root's); the backend walks from it to the window it belongs to. Default no-op: a
    // toolkit with no toolbar shows nothing rather than a drawn imitation, and reports
    // `Cap::Toolbar` as `Unsupported` so an app can put the command somewhere else.
    /// Edit the window's toolbar (docs/toolbars.md): the [`ToolbarOp`]s, removals first, then
    /// insertions in ascending position. The bar starts empty, and becomes empty again when every
    /// item has been removed, at which point the backend takes it off the window.
    ///
    /// An item no op names must keep its native widget untouched, which is what keeps a search
    /// field focused while the page around it changes. Day sends an edit only when the bar
    /// changed, at most once per turn, and an item keeps its [`ToolbarItem::action`] for as long
    /// as its window shows an item with that id, so a native item's handler never needs
    /// repointing. Keep a [`ToolbarMirror`] and place each new item after
    /// [`ToolbarMirror::prev_where`] its group.
    ///
    /// Returns whether the edit was applied. `false` (the window has no bar to edit yet) makes
    /// Day send the whole bar again at the next change rather than assume this one landed.
    fn edit_toolbar(&mut self, _h: &Self::Handle, _ops: &[ToolbarOp]) -> bool {
        true
    }
    /// Apply a targeted change to one live toolbar item, the path a bound signal writes through,
    /// so syncing a search field does not rebuild the bar. No-op if the item is not present.
    fn update_toolbar(&mut self, _h: &Self::Handle, _patch: &ToolbarPatch) {}

    // lifecycle (docs/lifecycle.md): does this backend deliver `phase`? The default answers "yes" for
    // the universal phases (launch/activation/termination) and "no" for the mobile-only ones. Backends
    // that wire up more (the mobile ones) override this; it must agree with the crate's
    // `const fn lifecycle_supported`, which drives compile-time guards in `day::require_lifecycle!`.
    fn supports_lifecycle(&self, phase: Lifecycle) -> bool {
        phase.is_universal()
    }

    // pillars
    /// Attach visible help to a widget. Empty text removes it.
    fn set_tooltip(&mut self, _anchor: &Self::Handle, _text: &str) {}
    /// Whether Day should also show the help on a long press.
    fn needs_touch_tooltip(&self) -> bool {
        false
    }
    /// Host a detached Day container in an anchored popup. Send `PopoverDismissed` to
    /// `id` on light dismissal. The content and its children remain owned by Day.
    fn open_popover(
        &mut self,
        _anchor: &Self::Handle,
        _content: &Self::Handle,
        _size: Size,
        _id: NodeId,
    ) -> bool {
        false
    }
    /// Update the size of an already presented popover, keeping its content and focus.
    fn resize_popover(&mut self, _content: &Self::Handle, _size: Size) {}

    /// Detach the content before its Day tree is released.
    fn close_popover(&mut self, _content: &Self::Handle) {}
    /// Show brief feedback in the window containing `host`.
    fn show_toast(&mut self, _host: &Self::Handle, _text: &str) {}

    /// Apply a node's accessibility annotations (docs/accessibility.md): the label, hint and
    /// value the screen reader speaks, an explicit role, whether the node is hidden from
    /// assistive technology, and the automation identifier. Called with the node's whole
    /// accumulated set each time any of it changes, so every member is written on every call.
    /// Only an explicit role is applied: a native control reports its own.
    fn set_a11y(&mut self, _h: &Self::Handle, _a11y: &A11yProps) {}
    /// Speak `text` through the screen reader without moving its focus (docs/accessibility.md;
    /// probe [`Cap::Announce`]). `urgent` interrupts what is being read; otherwise the sentence
    /// waits its turn. Nothing happens when no screen reader is running. The default is a
    /// backend that reaches no announcement API.
    fn announce(&mut self, _text: &str, _urgent: bool) {}
    /// Read a widget's native state back from the platform for `a11y_audit` and
    /// `assert_native` (§14.2, docs/testing.md) to diff against Day's expectation. Default:
    /// nothing read; the audit skips the node and `assert_native` records each field as unread.
    fn read_native(&self, _h: &Self::Handle) -> NativeSnapshot {
        NativeSnapshot::default()
    }
    /// Read what a widget shows at this moment, mid-animation included, for the
    /// `assert_displayed` step (docs/testing.md, docs/tween.md "The animation contract"): the
    /// presentation layer's opacity, frame, transform and background color on Core Animation,
    /// the animated `View` properties on Android, the computed style on the web. A field the
    /// toolkit cannot observe while its animator runs stays `None` and the step records it as
    /// unread. Default: nothing read.
    fn read_displayed(&self, _h: &Self::Handle) -> Displayed {
        Displayed::default()
    }
    fn replay(&mut self, _h: &Self::Handle, _ops: &[DrawOp], _size: Size) {}

    /// Every family this toolkit can draw canvas text in, with the faces each ships
    /// (docs/fonts.md; probe [`Cap::FontList`]). Default: none. `day::font_families` calls
    /// this once per process and caches the answer: enumeration is slow on every platform
    /// (fontconfig, a DirectWrite collection, an XML parse on Android) and the set does not
    /// change under a running app.
    fn font_families(&mut self) -> Vec<FontFamilyInfo> {
        Vec::new()
    }

    /// Measure one line of canvas text at `size` points in `font`, in the same engine
    /// [`Toolkit::replay`] draws [`DrawOp::Text`] with, so a `TextAnchor::LEADING` anchor plus
    /// the returned `ascent` lands on the drawn baseline. `None` = cannot measure; the facade
    /// then answers [`TextMetrics::approximate`].
    fn measure_text(&mut self, _text: &str, _size: f64, _font: &CanvasFont) -> Option<TextMetrics> {
        None
    }
    // --- raster images (docs/images.md) -------------------------------------------------
    //
    // Request-id shaped rather than blocking, because one backend can never answer
    // synchronously: a browser decodes through `createImageBitmap`, which is a promise. Every
    // other backend decodes inline and raises the completion before returning, so the shape
    // costs them nothing but a line, and the app-facing API can be one thing everywhere
    // instead of two (docs/async.md rule 3: a callback and a future, never a runtime).

    /// Decode encoded image bytes (PNG/JPEG/…) into a native image, answering `req` with
    /// [`Event::ImageDecoded`].
    ///
    /// The id in that event is minted by the caller (day-core), not the backend: the backend
    /// keeps its own side table from that id to whatever it decoded.
    ///
    /// The default does nothing because a default body cannot reach the event sink
    /// (it is installed per backend by [`Toolkit::set_event_sink`], not reachable through this
    /// trait), so there is no way for it to answer. day-core asks [`Cap::ImageDecode`] first and
    /// resolves the request as [`ImageError::Unsupported`] itself, which is why a backend with
    /// no decoder needs no code here at all.
    fn decode_image(&mut self, _req: u64, _id: BitmapId, _bytes: &[u8]) {}

    /// What a decoded image is: pixel size, scale, format, alpha ([`BitmapInfo`]).
    ///
    /// Synchronous: every backend that decoded the image already holds these facts. `None` means
    /// the id names nothing live.
    fn image_info(&mut self, _id: BitmapId) -> Option<BitmapInfo> {
        None
    }

    /// The metadata beyond the pixels: EXIF orientation, DPI, capture time
    /// ([`ImageProperties`], probe [`Cap::ImageProperties`]). `None` where the backend has no
    /// metadata reader, which is most of them.
    fn image_properties(&mut self, _id: BitmapId) -> Option<ImageProperties> {
        None
    }

    /// Encode a decoded image back to bytes per `spec`, answering `req` with
    /// [`Event::ImageEncoded`]. A format outside [`Self::encode_formats`] answers
    /// [`ImageError::Encode`] rather than silently substituting another one.
    ///
    /// Defaulted to nothing for the same reason as [`Self::decode_image`]: day-core probes
    /// [`Cap::ImageEncode`] and answers `Unsupported` without ever calling this.
    fn encode_image(&mut self, _req: u64, _id: BitmapId, _spec: &EncodeSpec) {}

    /// Which formats [`Self::encode_image`] can write, the [`Cap::ImageEncode`]
    /// counterpart of [`Self::font_families`]/[`Cap::FontList`]. Empty by default.
    fn encode_formats(&mut self) -> Vec<ImageFormat> {
        Vec::new()
    }

    /// Drop a decoded image and everything the backend cached for it. Called when the app's
    /// last handle to it goes away; an id the backend never saw is a no-op.
    fn release_image(&mut self, _id: BitmapId) {}

    // --- canvas export (docs/canvas.md "Exporting") --------------------------------------
    //
    // Request-id shaped like the image duties above, for the same reason: the browser's
    // `createImageBitmap` is a promise, and a native toolkit may paint on another thread. A
    // backend may also answer inline, before returning. day-core validates sizes, enforces
    // the pixel limits and probes the caps first, so a backend sees only requests it can
    // attempt.

    /// Replay `ops` into a new offscreen bitmap of `size` points at `scale` pixels per point
    /// ([`export_pixels`] gives the pixel size), with a transparent background and the
    /// canvas's top-left origin. Store it under `id` in the same table [`Self::decode_image`]
    /// fills, so `image_info`, `encode_image`, `release_image`, the `image` piece and
    /// [`DrawOp::Image`] all take it, then answer `req` with [`Event::BitmapRendered`] (its
    /// [`BitmapInfo::scale`] is `scale`).
    ///
    /// The painter must be the one [`Self::replay`] uses, so text, gradients, clips and stamps
    /// match the screen. Where the platform's 2-D API draws into a memory surface from any
    /// thread, the paint runs on a thread of [`offscreen`] and the answer is posted back, so
    /// a long replay never stalls the UI; the toolkit resolves on the UI thread, before it
    /// hands the job over, everything the painter would otherwise read from UI-thread state
    /// (the decoded images the ops reference, the fonts their text runs need). The contract
    /// holds either way: one answer per request, carrying that request's `req`, raised on the
    /// UI thread, and the bitmap in the table before it. Defaulted to nothing: day-core probes
    /// [`Cap::CanvasBitmap`] and answers [`ExportError::Unsupported`] itself.
    fn render_bitmap(
        &mut self,
        _req: u64,
        _id: BitmapId,
        _ops: &[DrawOp],
        _size: Size,
        _scale: f64,
    ) {
    }

    /// Replay each page into one PDF document and answer `req` with [`Event::PdfRendered`].
    /// Units are PDF points, top-left origin; shapes and text stay vector, and images embed at
    /// their pixel size. `pages` is never empty. Defaulted to nothing: day-core probes
    /// [`Cap::CanvasPdf`] first.
    fn render_pdf(&mut self, _req: u64, _pages: &[PdfPage], _info: &PdfInfo) {}

    /// Capture the primary window's content as encoded image bytes (the dayscript `screenshot`
    /// step, docs/window-image.md).
    ///
    /// # Errors
    ///
    /// Returns a message when the backend cannot capture; the default always does.
    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        Err("snapshot unsupported".into())
    }
    /// Where `root`, the primary window's root container, sits in [`Self::snapshot_window`]'s
    /// image, in points from its top-left. A reader of the capture (dayscript `sample_pixel`)
    /// maps Day's frames into it with this; it matters where the capture spans more than Day's
    /// content on more than one side, as Android's edge-to-edge content does (status bar
    /// above, navigation bar below).
    ///
    /// Defaulted to `None`, which the reader takes as "any rows beyond Day's content are
    /// above it", true where the only extra area is a title bar.
    fn snapshot_origin(&mut self, _root: &Self::Handle) -> Option<Point> {
        None
    }
    /// Establish a fresh screenshot checkpoint after reactive work and native transitions.
    /// `revision` is stable across retries and unique to the request; `host` selects a
    /// secondary window, or None for the primary. Never block the UI thread waiting for
    /// a frame. Pending requests are retried within the dayscript deadline.
    ///
    /// Ready means the platform render checkpoint completed, not that a vsync callback
    /// proves pixels were displayed. `OnCapture` delegates freshness to `snapshot_window`.
    ///
    /// # Errors
    ///
    /// Returns a message when the checkpoint cannot be established, for example because the
    /// window closed.
    fn prepare_snapshot(
        &mut self,
        _host: Option<&Self::Handle>,
        _revision: u32,
    ) -> Result<capture::Readiness, String> {
        // Compatible with external toolkits predating this hook: their synchronous
        // snapshot owns freshness, and no external-capture acknowledgment is fabricated.
        Ok(capture::Readiness::OnCapture)
    }

    /// The same capture with the window's own chrome: title bar, toolbar, whatever the platform
    /// draws around the content (docs/window-image.md).
    ///
    /// Defaulted to [`Self::snapshot_window`]: a backend that cannot separate the two
    /// (or has no chrome to speak of, a phone) answers with the content shot rather than an
    /// error, so an app asking for chrome degrades to "what there was" instead of failing. The
    /// dayscript `screenshot` step never calls this, which is what keeps every captured gallery
    /// baseline byte-identical to before it existed.
    ///
    /// # Errors
    ///
    /// Returns a message when the backend cannot capture the window.
    fn snapshot_window_chrome(&mut self) -> Result<Vec<u8>, String> {
        self.snapshot_window()
    }
    /// Show/hide the sidebar pane of the navigation host `host`, which is what the sidebar
    /// affordance a `nav(Sidebar)` contributes for itself drives ([`SIDEBAR_TOGGLE_ID`]).
    /// `false` when `host` has no pane to toggle (a stack, a tab bar, no sidebar concept at all).
    ///
    /// Per host, not per process: a second window's button collapses that window's sidebar
    /// and not the first's. The item's own action makes the call, and dayscript's `toolbar:`
    /// step presses the item like any other, so a walkthrough drives exactly the path a click
    /// takes. Defaulted, so a backend with no sidebar needs no code. docs/toolbars.md,
    /// docs/navigation.md.
    fn toggle_sidebar(&mut self, _host: &Self::Handle) -> bool {
        false
    }
    /// Press the platform's own back affordance (the navigation bar's back button on iOS,
    /// the system back on Android) on the innermost navigation host that has something to
    /// pop. What `nav_back()` does not do: that is Day's rail, which pops the model and lets the
    /// backend follow, while this runs the native path first (the bar's `shouldPop`, the
    /// dispatcher's callbacks, the pop itself) and lets the backend report it to Day as a user
    /// back, guard and all. dayscript's `nav_back: { native: true }` drives it, so a walkthrough
    /// can cover the code that a real tap runs and Day's rail never reaches. `false` when this
    /// toolkit has no native back or nothing is popped by it. Defaulted, so a desktop needs no
    /// code. docs/navigation.md.
    fn native_back(&mut self) -> bool {
        false
    }
    /// Take (`true`) or release (`false`) the platform's system back for the app's own
    /// handlers ([`Decorate::on_system_back`](/api/day_pieces/trait.Decorate.html#method.on_system_back)):
    /// day-core arms it while at least one mounted handler is enabled. Armed, a system back that
    /// nothing native consumes first (a presented cover, a pushed page, an armed guard) reaches
    /// Day as [`Event::NavBack`] on [`WINDOW_NODE`] instead of leaving the app; released, the
    /// platform's default runs, predictive-back preview and all. Android registers an
    /// `OnBackPressedCallback` beneath its own; HarmonyOS consumes the page's `onBackPress`.
    /// Defaulted, so a toolkit with no system back (desktop, iOS, the web) needs no code.
    /// docs/navigation.md "Handling the system Back".
    fn set_system_back(&mut self, _armed: bool) {}
    /// Whether the UI has settled, with no native transition (modal present/dismiss, nav push)
    /// still animating. The dayscript `screenshot` step polls this before capturing so shots
    /// never catch a half-faded dialog or half-pushed page. Backends without async
    /// transitions (or without a way to know) report `true`.
    fn ui_idle(&mut self) -> bool {
        true
    }

    // imperative presentation (docs/dialogs.md): show a native modal for request `req`;
    // the backend answers by enqueuing `Event::PresentResult { req, .. }`. `dismiss` is
    // used only when Day resolves programmatically (dayscript) while the modal is still up.
    fn present(&mut self, _req: u64, _spec: &present::PresentSpec) {}
    fn dismiss(&mut self, _req: u64) {}

    /// Open `url` in the platform's default handler: the system browser for `http(s)`, the mail
    /// client for `mailto:`, etc. Backs the [`link`](/api/day_pieces/fn.link.html) piece. Fire and
    /// forget: there is no result, and an unopenable URL is ignored. The default no-ops so a
    /// backend that hasn't wired it up still compiles.
    fn open_url(&mut self, _url: &str) {}

    /// Whether a native share chooser is available. Does not probe installed share targets.
    fn share_support(&self) -> Support {
        Support::Unsupported
    }
    /// Present the system share chooser for `spec` (docs/sharing.md). True means presentation
    /// started, not that sharing completed. Never sends data without the user's choice of a
    /// target. Where the chooser can carry the app's own actions (`Cap::ShareActions`), each
    /// of `spec.actions` appears with its title and glyph, and choosing one enqueues
    /// `Event::MenuAction(action.id)`; elsewhere the actions are left out. `anchor` is the
    /// control the chooser points at where it is a popover (iPad, macOS); without one the
    /// chooser uses the window's top trailing corner.
    fn share(&mut self, _spec: &share::ShareSpec, _anchor: Option<&Self::Handle>) -> bool {
        false
    }

    /// Read current OS associations. Unsupported platforms must not fabricate a default.
    ///
    /// # Errors
    ///
    /// Returns [`applications::ApplicationError::Unsupported`] where the platform has no
    /// association query (the default), or the platform's own failure.
    fn application_handlers(
        &mut self,
        _query: &applications::HandlerQuery,
    ) -> Result<applications::ApplicationHandlers, applications::ApplicationError> {
        Err(applications::ApplicationError::Unsupported)
    }

    /// Open with an opaque application identifier returned by `application_handlers`.
    fn open_url_with_application(
        &mut self,
        _url: &str,
        _application: &str,
        completion: applications::OpenApplicationCompletion,
    ) {
        completion(Err(applications::ApplicationError::Unsupported));
    }

    /// Ask the OS to require a second swipe for its edge gestures on `edges` (docs/cover.md):
    /// the union requested by every mounted `defers_system_gestures` modifier, re-sent whenever
    /// that set changes (`Edges::NONE` when the last one unmounts). iOS defers the home
    /// indicator / notification edges; Android enters swipe-to-reveal immersive mode while
    /// non-empty. The default no-ops (desktop has no system edge gestures).
    fn defer_system_gestures(&mut self, _edges: Edges) {}

    /// Hide (`true`) or show the system status bar (docs/cover.md): whether any mounted
    /// `status_bar_hidden` modifier currently asks for it hidden, re-sent whenever that answer
    /// may have changed (`false` when the last request unmounts). iOS answers the root and
    /// cover view controllers' `prefersStatusBarHidden`; Android and HarmonyOS hide the status
    /// bar through their window inset controls. The default no-ops (desktop and web have no
    /// status bar); report `Cap::StatusBarHidden` alongside an implementation.
    fn set_status_bar_hidden(&mut self, _hidden: bool) {}

    /// Whether the platform is rendering in dark appearance right now. Apps painting
    /// custom surfaces (opaque overlay panels, scrims) branch on this so their fills track
    /// the theme the default text colors already follow; hardcoding either theme's
    /// surface produces dark-on-dark or light-on-light text on the other. Queried at
    /// build time (a piece rebuilt after a theme change re-queries). The default honors a
    /// `DAY_THEME` launch override and otherwise answers light.
    /// Resolve a semantic color for custom artwork. Backends with native palette access
    /// override this; the default follows light/dark appearance using Day's fixed palette.
    fn system_color(&mut self, role: SystemColor) -> Color {
        role.fallback(self.dark_mode())
    }

    fn dark_mode(&mut self) -> bool {
        std::env::var("DAY_THEME").ok().as_deref() == Some("dark")
    }
    /// Whether the user asked the system for less motion (docs/accessibility.md; probe
    /// [`Cap::ReduceMotion`]): macOS and iOS "Reduce Motion", Android "Remove animations" (an
    /// animator duration scale of zero), GTK's `gtk-enable-animations` off, Windows "Show
    /// animations" off, the web's `prefers-reduced-motion`. Read at launch; a backend that
    /// watches the setting calls `day_core::note_motion_changed` when it flips. Day then applies
    /// every finite animation at its destination and keeps only indeterminate motion. The default
    /// is a backend that reads no such setting.
    fn reduce_motion(&mut self) -> bool {
        false
    }

    /// The factor the system's text-size setting currently applies to `style`, against that
    /// style's size at the platform's default setting (docs/text.md "Text scale"): `1.0` at
    /// the default, `1.3` when the reader's setting draws this style 30% larger, below `1.0`
    /// for a smaller-than-default setting. Platforms that scale every style by one number
    /// (Android's `fontScale`, GNOME's `text-scaling-factor`, Windows' `TextScaleFactor`, the
    /// browser's default font size) ignore `style`; UIKit answers per style, because Dynamic
    /// Type grows small styles more than large ones.
    ///
    /// Read when a piece asks (`day::text_scale()`) and again after the toolkit calls
    /// `day_core::note_text_size_changed`, which every toolkit that can observe the setting
    /// calls when it changes. Day uses the answer to hold a label under its
    /// `max_text_scale` cap. The default is a platform with no text-size setting.
    fn text_scale(&mut self, _style: Font) -> f64 {
        1.0
    }

    /// App-level appearance override: `Some(true)` forces dark, `Some(false)` forces light,
    /// `None` returns to following the system. Backends restyle their native widgets in
    /// place and answer the override from [`dark_mode`](Self::dark_mode); app-painted
    /// surfaces pick it up on their next rebuild. `Cap::Appearance` reports whether the
    /// backend honors this; the default ignores the call.
    fn set_appearance(&mut self, _dark: Option<bool>) {}

    /// Put `badge` on the app's icon in the Dock / launcher / home screen (docs/badge.md).
    ///
    /// Fire-and-forget, like `set_appearance`: a payload this toolkit cannot render is ignored, and
    /// the app probes `Cap::AppBadge{Count,Text,Dot}` before choosing one. Never substitute: a
    /// backend that turned `Text("beta")` into `Count(1)` would put a wrong number on a user's
    /// icon, which is worse than showing nothing. The default ignores every call.
    fn set_app_badge(&mut self, _badge: &AppBadge) {}

    // app lifecycle (mobile; desktop backends no-op)
    fn on_suspend(&mut self) {}
    fn on_resume(&mut self) {}
    fn on_memory_warning(&mut self) {}

    // adoption of foreign native handles (polyglot pieces, §15.3)
    fn adopt(&mut self, _raw: RawHandle) -> Self::Handle {
        unimplemented!("this toolkit does not adopt foreign handles yet")
    }

    // secondary windows (docs/windows.md)

    /// Open a native OS window per `options` + `kind`: create and show the window, wire its
    /// events to `id` (`WindowResized` with the content size in points, `WindowClosed` after
    /// the native close, `WindowFocused` on key/active changes), and answer with its content
    /// container handle, the same contract as the `ready` root container. Backends whose
    /// window creation is asynchronous (a scene, an activity, an ability) answer
    /// [`WindowOpenReply::Pending`] and complete later through
    /// `day_core::finish_window_open(id, raw, size)`, which goes through [`Toolkit::adopt`].
    /// The default answers `Unsupported`; day-core then presents the content as a
    /// fullscreen cover in the primary window instead.
    fn open_window(
        &mut self,
        _id: NodeId,
        _options: &WindowOptions,
        _kind: WindowKind,
    ) -> WindowOpenReply<Self::Handle> {
        WindowOpenReply::Unsupported
    }

    /// Close the native window whose content container is `host` (a handle `open_window`
    /// produced). Asynchronous: the platform confirms with `Event::WindowClosed` on the
    /// window's root node, and only then does day-core tear the subtree down. Idempotent; unknown
    /// hosts are ignored. Never called with the primary window's container.
    fn close_window(&mut self, _host: &Self::Handle) {}

    /// End the application: the last window with [`WindowRole::Primary`] has closed
    /// (docs/windows.md). day-core decides when, since it owns the window registry and the role
    /// each window carries; the backend supplies only the platform's exit:
    /// `NSApplication.terminate`, `QCoreApplication::quit`, `GtkApplication.quit`,
    /// `PostQuitMessage`. Everything day-core wanted to happen first (secondary windows
    /// disposed, `WillTerminate` delivered) already has by the time this is called.
    ///
    /// The default does nothing, which is right for the backends whose process lifetime is
    /// not theirs to end: a browser tab, an Android activity, an iOS scene.
    fn quit_app(&mut self) {}

    /// Bring the window whose content container is `host` to front and make it key/active:
    /// the focus half of open-or-focus singleton windows. Default no-op.
    fn focus_window(&mut self, _host: &Self::Handle) {}

    /// Retitle the window whose content container is `host`. Default no-op.
    fn set_window_title(&mut self, _host: &Self::Handle, _title: &str) {}

    /// Resize the window whose content container is `host` to fit `size` of content, and pin it
    /// there: what `WindowOptions::size_to_fit` asks for (docs/windows.md).
    ///
    /// `size` is the content size Day measured, not the window's outer frame: the backend adds
    /// its chrome. A settings panel is the case this exists for, and every desktop platform
    /// sizes one to its content rather than to a number the app guessed; a fixed height either
    /// clips the last row or leaves a band of empty panel under it, and which one you get depends
    /// on the user's text size.
    ///
    /// Default no-op, so a toolkit that cannot resize a live window keeps the size it was
    /// opened at, which is what every backend did before this existed.
    fn fit_window(&mut self, _host: &Self::Handle, _size: Size) {}

    /// Snapshot the window whose content container is `host` (the dayscript `screenshot`
    /// step's `window:` target). The default answers the primary snapshot, so backends that
    /// never open windows stay correct without changes.
    ///
    /// # Errors
    ///
    /// Returns a message when the backend cannot capture that window.
    fn snapshot_window_of(&mut self, _host: &Self::Handle) -> Result<Vec<u8>, String> {
        self.snapshot_window()
    }

    /// Apply one property change to the window whose content container is `host`
    /// (docs/windows.md "Window properties"). The primary window arrives with its root container,
    /// like [`set_window_title`](Self::set_window_title).
    ///
    /// Fire-and-forget, like [`set_app_badge`](Self::set_app_badge): a change this toolkit cannot
    /// make is ignored, and the app probes the matching `Cap` (named on each [`WindowChange`]
    /// variant) before relying on it. A change the platform completes later (a fullscreen
    /// transition, a zoom animation) reports its outcome through
    /// [`Event::WindowStateChanged`], never by assuming the request took. The default ignores
    /// every change.
    fn apply_window(&mut self, _host: &Self::Handle, _change: &WindowChange) {}

    /// True OS window groups, as distinct from native tab widgets.
    fn native_window_tabs(&self) -> bool {
        false
    }
    /// Native in-window document/section tab control; OS grouping is reported separately.
    fn native_document_tabs(&self) -> bool {
        false
    }
    /// Join/reorder these windows in an OS tab group. Content remains window-owned.
    fn group_windows(&mut self, _hosts: &[Self::Handle]) {}
    /// Current native order of the group containing this window, including detached groups.
    fn window_tab_order(&self, _host: &Self::Handle) -> Vec<NodeId> {
        Vec::new()
    }

    /// Install the app's Dock menu (docs/menus.md "Dock menu"): the items macOS shows above
    /// its own when the user right-clicks the app's Dock icon, built from the app's
    /// `[[shortcuts]]` and its `dock_menu(..)` entries. Same item model and dispatch as
    /// [`set_app_menu`](Self::set_app_menu); an empty slice removes the menu. `Cap::DockMenu`
    /// reports support; the default ignores the call.
    fn set_dock_menu(&mut self, _items: &[MenuItem]) {}

    /// Make the node at `h` move its window when the user drags it, as a title bar does
    /// (`.window_drag_region()`, docs/window-chrome.md): the region's own background starts a
    /// native window drag (with the platform's snapping and edge tiling), a double click does
    /// what the user's title-bar double-click setting says, and controls inside the region keep
    /// receiving their own clicks. `false` releases it. `Cap::DragRegion`; the default ignores
    /// the call, so on a phone the region is an ordinary view.
    fn set_drag_region(&mut self, _h: &Self::Handle, _drag: bool) {}

    /// Show these status items, replacing the previous set (docs/status-item.md): add the
    /// ones whose `id` is new, update the ones that changed, remove the ones no longer listed.
    /// A click on an item runs its `activate` action or opens its menu; menu choices dispatch
    /// [`Event::MenuAction`] like the app menu's. `Cap::StatusItem`; the default ignores the
    /// call, which is what a phone and the web do.
    fn set_status_items(&mut self, _items: &[StatusItemSpec]) {}

    /// Show `progress` on the app's Dock or taskbar icon (docs/status-item.md).
    /// `Cap::AppProgress`; the default ignores it.
    fn set_app_progress(&mut self, _progress: AppProgress) {}

    /// Show or hide the app's Dock icon while it runs: a menu-bar app hides it and lives in its
    /// status item (macOS activation policy). `Cap::DockVisibility`; the default ignores it.
    fn set_dock_visible(&mut self, _visible: bool) {}

    /// Keep the process running when its last window closes (docs/windows.md "Keeping the app
    /// running"). When `true`, the backend must not end the app on its own as windows close: it
    /// reports every close, the first window's included, as `Event::WindowClosed`, and day-core
    /// decides whether to call [`quit_app`](Self::quit_app). When `false`, the backend keeps its
    /// platform's own rule. A backend whose process lifetime is not its own (a phone, a browser
    /// tab) ignores it, which is the default.
    fn set_keep_running(&mut self, _keep: bool) {}

    /// The displays attached now (docs/windows.md "Monitors"). `Cap::Monitors`; the default
    /// answers none, and day-core then describes the primary window's own size as one display.
    fn monitors(&self) -> Vec<Monitor> {
        Vec::new()
    }

    /// The outer frame of the window whose content container is `host`, in points on the
    /// desktop's coordinate space (top-left origin), or `None` where the platform does not say
    /// (a phone, Wayland's positions). Read when the app asks for it and when day-core saves a
    /// remembered frame (`WindowOptions::remember_frame`).
    fn window_frame(&self, _host: &Self::Handle) -> Option<Rect> {
        None
    }

    /// Replace the launcher shortcuts the app set while running (docs/deep-links.md "Launcher
    /// shortcuts"), shown beside the ones Day.toml declares: iOS Home Screen quick actions,
    /// Android dynamic shortcuts, the macOS Dock menu. Each opens its route through the deep-link
    /// intake. `Cap::DynamicShortcuts`; the default ignores the call.
    fn set_launcher_shortcuts(&mut self, _shortcuts: &[LauncherShortcut]) {}
}

/// What a secondary window is, so backends can apply platform conventions (docs/windows.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowKind {
    /// A regular document/tool window: resizable, miniaturizable, platform-default tabbing.
    Normal,
    /// A settings/preferences window: singleton by convention; macOS disallows window
    /// tabbing and drops the resize/minimize chrome, mobile presents it modally. Pairs with
    /// day-core's key-singleton reopen (`open_window` with a key).
    Preferences,
    /// The About window (docs/settings.md): the same fixed, centered, untabbed panel chrome
    /// as [`Self::Preferences`] (every desktop treats the two alike), presented modally on
    /// mobile. Singleton under day-core's `day.about` key.
    About,
}

impl WindowKind {
    /// Whether this is a panel (settings or About) rather than a document window: the kinds
    /// every desktop draws fixed-size, centered and out of the Window menu, and mobile presents
    /// as a cover.
    #[must_use]
    pub fn is_panel(self) -> bool {
        !matches!(self, WindowKind::Normal)
    }
}

/// Whether a window keeps the app alive (docs/windows.md close policy).
///
/// A separate axis from [`WindowKind`], which describes chrome. They correlate (a
/// preferences panel is the archetypal window that should not hold an app open), but they are
/// not the same question, and conflating them would leave no way to say "an ordinary-looking
/// tool window that shouldn't outlive the documents".
///
/// Note the word "secondary" is overloaded around windows: elsewhere in day it means "a window
/// other than the first one opened". Here it means "does not keep the app alive", which is
/// about lifetime, not about order of creation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum WindowRole {
    /// Closing the last one of these ends the app.
    #[default]
    Primary,
    /// Never keeps the app alive: when the last `Primary` closes, the app exits and takes
    /// these with it, however many are open.
    Secondary,
}

impl From<WindowKind> for WindowRole {
    /// The default pairing: an ordinary window holds the app open, a preferences panel does
    /// not. An app that needs a different answer sets the role itself.
    fn from(kind: WindowKind) -> WindowRole {
        match kind {
            WindowKind::Normal => WindowRole::Primary,
            WindowKind::Preferences | WindowKind::About => WindowRole::Secondary,
        }
    }
}

/// A backend's answer to [`Toolkit::open_window`] (docs/windows.md).
pub enum WindowOpenReply<H> {
    /// The window exists now; here is its content container (desktop backends).
    Open(H),
    /// Native creation started but completes asynchronously (a scene, an activity, an
    /// ability); the backend will call `day_core::finish_window_open` when the content
    /// container exists. day-core parks the window record until then.
    Pending,
    /// This toolkit cannot open windows (`Cap::MultiWindow` = `Unsupported`); day-core
    /// falls back to presenting the content as a fullscreen cover in the primary window.
    Unsupported,
}

/// How a window is shown (docs/windows.md "Window properties"): what the window's `state()`
/// signal holds and what [`WindowChange::State`] asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowState {
    /// On screen at its own size: neither minimized, maximized nor fullscreen.
    #[default]
    Normal,
    /// Minimized to the Dock, taskbar or shelf. `Cap::WindowStates`.
    Minimized,
    /// Zoomed to fill the screen's work area, keeping its chrome. `Cap::WindowStates`.
    Maximized,
    /// Covering the whole screen with no window chrome; on a phone, the system bars hidden.
    /// `Cap::WindowFullscreen`.
    Fullscreen,
}

impl WindowState {
    /// The wire code of [`bridge::BridgeKind::WindowStateChanged`].
    #[must_use]
    pub const fn code(self) -> i32 {
        match self {
            WindowState::Normal => 0,
            WindowState::Minimized => 1,
            WindowState::Maximized => 2,
            WindowState::Fullscreen => 3,
        }
    }

    /// The state a wire code names; an unknown code reads as `Normal`.
    #[must_use]
    pub const fn from_code(code: i32) -> WindowState {
        match code {
            1 => WindowState::Minimized,
            2 => WindowState::Maximized,
            3 => WindowState::Fullscreen,
            _ => WindowState::Normal,
        }
    }
}

/// How a window stacks against other apps' windows ([`WindowChange::Level`], `Cap::WindowLevel`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowLevel {
    /// Ordinary stacking: the focused app's windows come forward.
    #[default]
    Normal,
    /// Stays above other apps' ordinary windows: a palette, a timer, a video peek.
    Floating,
}

/// How strongly a window asks for the user's attention ([`WindowChange::RequestAttention`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Attention {
    /// Once: one Dock bounce, a taskbar button that flashes a few times.
    #[default]
    Informational,
    /// Until the user switches to the app.
    Critical,
}

/// One change to one window (docs/windows.md "Window properties"), carried by
/// [`Toolkit::apply_window`]. Each variant names the `Cap` that reports whether the running
/// toolkit makes it.
#[derive(Clone, Debug, PartialEq)]
pub enum WindowChange {
    /// Minimize, zoom, take fullscreen, or return to normal. `Cap::WindowStates` for the
    /// first two, `Cap::WindowFullscreen` for fullscreen. The outcome comes back as
    /// [`Event::WindowStateChanged`].
    State(WindowState),
    /// Keep the window's content out of screenshots, recordings and screen sharing.
    /// `Cap::ContentProtection`.
    ContentProtected(bool),
    /// Move and/or resize. `origin` is the outer frame's top-left corner in points on the
    /// desktop's coordinate space (top-left origin, `Cap::WindowPosition`); `size` is the size Day
    /// lays the content out at, in points, the same size `WindowResized` reports
    /// (`Cap::WindowGeometry`): the backend adds its own chrome around it.
    Frame {
        origin: Option<Point>,
        size: Option<Size>,
    },
    /// The content size the user can resize the window to. `None` lifts that bound.
    /// `Cap::WindowGeometry`.
    Limits {
        min: Option<Size>,
        max: Option<Size>,
    },
    /// Stacking against other apps' windows. `Cap::WindowLevel`.
    Level(WindowLevel),
    /// Show the window on every virtual desktop / Space. `Cap::WindowLevel`.
    OnAllWorkspaces(bool),
    /// Leave the window out of the taskbar and the window switcher. `Cap::WindowLevel`.
    SkipTaskbar(bool),
    /// Whether the user can resize the window. `Cap::WindowGeometry`.
    Resizable(bool),
    /// Whether the window offers a minimize control. `Cap::WindowStates`.
    Minimizable(bool),
    /// Whether the window offers a maximize / zoom control. `Cap::WindowStates`.
    Maximizable(bool),
    /// Whether the window offers a close control. `Cap::WindowStates`.
    Closable(bool),
    /// Hide the window without closing it, or show it again. `Cap::WindowStates`.
    Visible(bool),
    /// This window's light/dark override over the app-wide [`Toolkit::set_appearance`].
    /// `Cap::WindowAppearance`.
    Appearance(Option<bool>),
    /// Bounce the Dock icon, flash the taskbar button. `Cap::RequestAttention`.
    RequestAttention(Attention),
}

/// The frame a window wears (docs/window-chrome.md), set once when it opens.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowChrome {
    /// The platform's title bar and frame.
    #[default]
    Standard,
    /// The content runs to the window's top edge, under the title bar, and the platform's own
    /// window controls (close, minimize, zoom) stay where they are. For a custom title bar: the
    /// backend reports the bar's height as the window's safe-area top inset (`day::safe_area()`),
    /// so the app pads its content by it and paints its background under it. Mark the draggable
    /// part with `.window_drag_region()`. `Cap::OverlayTitleBar`; where unsupported the window
    /// keeps its standard bar.
    Overlay,
    /// No title bar and no frame: a splash screen, a heads-up panel, a borderless utility. The
    /// window still resizes from its edges when resizable; mark the part that moves it with
    /// `.window_drag_region()`. `Cap::FramelessWindow`; where unsupported the window keeps its
    /// standard frame.
    Frameless,
}

/// What shows behind a window's content (docs/window-chrome.md).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowBackground {
    /// The platform's window background.
    #[default]
    Opaque,
    /// Nothing: pixels the content leaves unpainted show the desktop through the window. For
    /// rounded or shaped splash screens and overlays. `Cap::TransparentWindow`.
    Transparent,
    /// A translucent system material that blurs what is behind the window.
    /// `Cap::WindowMaterial`; where unsupported the window stays opaque.
    Material(WindowMaterial),
}

/// The system materials a window background can use ([`WindowBackground::Material`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowMaterial {
    /// The main-window material: macOS's window background, Windows 11's Mica.
    Window,
    /// A sidebar or source list: macOS's sidebar material, Windows 11's Mica Alt (tabbed).
    Sidebar,
    /// A transient surface (a popover, a menu-like panel, a heads-up display): macOS's popover
    /// material, Windows 11's Acrylic.
    Transient,
}

/// Where a window opens ([`WindowOptions::placement`]).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum WindowPlacement {
    /// The platform's choice: macOS cascades, Windows and Linux let the window manager decide.
    #[default]
    Automatic,
    /// Centered on the screen with the active window.
    Centered,
    /// The outer frame's top-left corner at this point, in points on the desktop's coordinate
    /// space (top-left origin). Falls back to `Automatic` without `Cap::WindowPosition`.
    At(Point),
}

/// A small runtime RGBA image for a status item: charts, meters, or a grid of results.
///
/// Rows run top to bottom; samples are tightly packed, non-premultiplied sRGB RGBA8.
/// `scale` is pixels per logical point (2 for Retina). This owns its bytes, independent of
/// any window or decoded-image registry. Currently rendered by AppKit; other backends use
/// [`StatusItemSpec::icon`] as a fallback. See docs/status-item.md.
#[derive(Clone, PartialEq)]
pub struct StatusImage {
    width: u32,
    height: u32,
    scale: f64,
    rgba: std::sync::Arc<[u8]>,
}

impl StatusImage {
    /// Validate a runtime image. Rejects empty/mismatched pixels, invalid scales, and images
    /// exceeding 16 MiB or 4096 logical points on either axis. No platform allocation occurs.
    pub fn rgba(
        width: u32,
        height: u32,
        scale: f64,
        pixels: impl Into<std::sync::Arc<[u8]>>,
    ) -> Option<Self> {
        let rgba = pixels.into();
        let expected = (width as usize)
            .checked_mul(height as usize)?
            .checked_mul(4)?;
        if width == 0
            || height == 0
            || expected > 16 * 1024 * 1024
            || rgba.len() != expected
            || !scale.is_finite()
            || scale <= 0.0
            || f64::from(width) / scale > 4096.0
            || f64::from(height) / scale > 4096.0
        {
            return None;
        }
        Some(Self {
            width,
            height,
            scale,
            rgba,
        })
    }

    #[must_use]
    pub fn width(&self) -> u32 {
        self.width
    }
    #[must_use]
    pub fn height(&self) -> u32 {
        self.height
    }
    #[must_use]
    pub fn scale(&self) -> f64 {
        self.scale
    }
    #[must_use]
    pub fn pixels(&self) -> &[u8] {
        &self.rgba
    }
    pub fn size(&self) -> Size {
        Size::new(
            f64::from(self.width) / self.scale,
            f64::from(self.height) / self.scale,
        )
    }
}

impl std::fmt::Debug for StatusImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StatusImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("scale", &self.scale)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

#[cfg(test)]
mod status_image_tests {
    use super::*;

    #[test]
    fn validates_pixel_layout_and_logical_size() {
        let image = StatusImage::rgba(120, 36, 2.0, vec![255; 120 * 36 * 4]).unwrap();
        assert_eq!(image.size(), Size::new(60.0, 18.0));
        assert_eq!(image.clone(), image);
        assert!(StatusImage::rgba(0, 1, 1.0, vec![]).is_none());
        assert!(StatusImage::rgba(1, 1, 1.0, vec![0; 3]).is_none());
        assert!(StatusImage::rgba(u32::MAX, u32::MAX, 1.0, vec![]).is_none());
        for scale in [0.0, -1.0, f64::NAN, f64::INFINITY, 0.00001] {
            assert!(StatusImage::rgba(1, 1, scale, vec![0; 4]).is_none());
        }
    }
}

/// One status item: an icon in the menu bar (macOS), the notification area (Windows) or a
/// `StatusNotifierItem` host's tray (Linux), with a menu (docs/status-item.md).
#[derive(Clone, Debug, PartialEq)]
pub struct StatusItemSpec {
    /// The app's name for the item, stable across updates: what a backend diffs by.
    pub id: String,
    /// The glyph. `None` shows the title alone (macOS) or the app's icon (elsewhere).
    pub icon: Option<Icon>,
    /// A runtime image. AppKit prefers this over `icon`, preserving aspect ratio and point
    /// size up to 18pt high; other backends ignore it and show `icon`/`title` as before.
    pub raster: Option<StatusImage>,
    /// Draw the icon as a template: its shape only, in the menu bar's or panel's own color, so
    /// it reads in light and dark. Symbols are always templates.
    pub template: bool,
    /// Text beside the icon where the platform shows one (the macOS menu bar). Empty: icon only.
    pub title: String,
    /// The hover text.
    pub tooltip: String,
    /// The menu the item opens. Same model and dispatch as the app menu.
    pub menu: Vec<MenuItem>,
    /// A dispatch id (the [`Event::MenuAction`] registry) run when the user clicks the item
    /// itself, where the platform separates a click from opening the menu (a left click on macOS
    /// and Windows, the host's "activate" on Linux). 0: a click opens the menu.
    pub activate: u64,
}

/// One display (docs/windows.md "Monitors"), in points on the desktop's coordinate space:
/// top-left origin at the primary display's top-left corner.
#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    /// Stable for the display across launches where the platform offers it (a display UUID, a
    /// connector name); an index otherwise.
    pub id: String,
    /// The display's name, for a picker ("Built-in Retina Display", "DELL U2720Q").
    pub name: String,
    /// The display's whole area.
    pub frame: Rect,
    /// The part windows should use: the frame minus the menu bar, Dock, taskbar or panels.
    pub work_area: Rect,
    /// Device pixels per point.
    pub scale: f64,
    /// The display the desktop's menu bar or primary taskbar is on.
    pub primary: bool,
}

/// A launcher shortcut the app sets while it runs (docs/deep-links.md "Launcher shortcuts"): a
/// route to open and the label the launcher shows, already localized.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherShortcut {
    /// The route the shortcut opens, through the deep-link intake.
    pub route: String,
    /// What the launcher shows.
    pub label: String,
}

/// A progress indicator on the app's Dock or taskbar icon ([`Toolkit::set_app_progress`],
/// docs/status-item.md).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AppProgress {
    /// No indicator.
    #[default]
    None,
    /// Work of unknown length.
    Indeterminate,
    /// A fraction done, 0.0 to 1.0.
    Value(f64),
    /// A fraction done, paused. Platforms without a paused state show `Value`.
    Paused(f64),
    /// A fraction done when the work failed. Platforms without an error state show `Value`.
    Error(f64),
}

/// Whether and how a window joins macOS window tabs ([`WindowOptions::tabbing`],
/// `Cap::WindowTabbing`). Other platforms have no system window tabs and ignore it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum WindowTabbing {
    /// The platform default for the window's kind: an ordinary window tabs with the app's other
    /// ordinary windows when the user's "Prefer tabs" setting says so; preferences never tab.
    #[default]
    Automatic,
    /// Tab only with windows of the same group, following the user's setting. The tab bar's
    /// "+" opens the builder registered for this group (`register_new_window_for`).
    Group(String),
    /// Always open as a tab of an existing window of the same group, whatever the setting.
    Preferred(String),
    /// Never become a tab.
    Disallowed,
}

/// An app's generated locale catalog: `(DEFAULT, CATALOG)`, exactly what `day-build` emits as
/// `res::locales::{DEFAULT, CATALOG}` (§18.5). Carried in [`WindowOptions::locales`] so the
/// framework can install it at the one moment that is correct.
pub type AppLocales = (&'static str, &'static [(&'static str, &'static str)]);

#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub title: String,
    pub size: Size,
    pub min_size: Option<Size>,
    /// Create and mount the window without showing it or activating the app. Supported on
    /// AppKit; other backends currently ignore this option. Defaults to false.
    ///
    /// Menu-bar apps can keep their initial host hidden and open Preferences on demand.
    /// The window still exists and can be shown with `WindowHandle::set_visible(true)`;
    /// this does not change the app's keep-running policy.
    pub start_hidden: bool,
    /// Ask the backend to size this window to its content once it has been built and laid out,
    /// rather than keeping [`Self::size`] (docs/windows.md). `size` still decides the width and
    /// acts as the height ceiling, so a panel with more content than fits the screen scrolls
    /// instead of growing past it.
    ///
    /// What a preferences panel wants on every desktop. Ignored by backends that cannot resize a
    /// live window, and by the ones with no windows at all.
    pub size_to_fit: bool,
    /// The app's display name for the standard application menu / About (macOS). `None` falls back
    /// to `title`; set it when `title` carries extra decoration you don't want in `About <name>`
    /// (e.g. the showcase's window title is "Day Showcase (AppKit)" but its app name is "Showcase").
    pub app_name: Option<String>,
    /// The app's locale catalog, installed by `day::launch` (docs/localization.md).
    ///
    /// The order is why this is the framework's job: the OS's languages reach day-localization from the
    /// live backend, and the catalog has to be registered after that hint and before the first
    /// localized string is read. An app that installs it itself (before `launch`, where there
    /// is no backend yet) registers against an empty hint list and resolves to `DEFAULT`, so a
    /// French device opens an English window.
    pub locales: Option<AppLocales>,
    /// The title, computed once the catalog above is installed: how a window title comes from
    /// the catalog rather than a literal. Takes precedence over [`Self::title`], which stays the
    /// fallback for the moment before (and for apps with nothing to translate).
    ///
    /// A plain `fn` rather than a closure: this struct is `Clone`, and the callers are a
    /// non-capturing `|| res::str::app_title().format()`, which coerces.
    pub title_fn: Option<fn() -> String>,
    /// The app's version, for Day's exit line (`Clean exit for <app> v<version> …`,
    /// docs/lifecycle.md): `env!("CARGO_PKG_VERSION")` in the app crate, which the scaffold's
    /// `window()` sets. `None` falls back to the `DAY_APP_VERSION` the `day` CLI sets on a launch.
    pub version: Option<String>,
    /// macOS window tabbing for this window (`Cap::WindowTabbing`); ignored elsewhere.
    pub tabbing: WindowTabbing,
    /// The window's frame: standard, a title bar the content runs under, or none
    /// (docs/window-chrome.md). Desktop only; phones and the web ignore it.
    pub chrome: WindowChrome,
    /// What shows behind the content: the window background, nothing, or a system material.
    pub background: WindowBackground,
    /// Whether the window casts the platform's drop shadow. A transparent, shaped window often
    /// wants none.
    pub shadow: bool,
    /// Whether the user can resize the window by its edges.
    pub resizable: bool,
    /// Where the window opens.
    pub placement: WindowPlacement,
    /// The largest content size the user can resize the window to (`Cap::WindowGeometry`).
    pub max_size: Option<Size>,
    /// Remember this window's size, position and maximized state under this key, and reopen it
    /// that way next launch (docs/windows.md "Remembered frames"). A position that no attached
    /// display shows any more is dropped, so a window never reopens off-screen.
    pub remember_frame: Option<String>,
    /// The logical window this opens (docs/windows.md): `open_window` focuses an already-open
    /// window with the same key instead of opening a second (the preferences pattern). `None`
    /// always opens a new one.
    pub key: Option<String>,
    /// The kind of window: a regular one, a preferences panel, an About panel.
    pub kind: WindowKind,
}

impl WindowOptions {
    /// The window title. Replaces a [`title_fn`](Self::title_fn) set earlier (the generated
    /// `res::window_options()` sets one), so the last title named wins.
    #[must_use]
    pub fn title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self.title_fn = None;
        self
    }
    /// The content size the window opens at (desktop; phones fill the screen).
    #[must_use]
    pub fn size(mut self, width: f64, height: f64) -> Self {
        self.size = Size::new(width, height);
        self
    }
    /// The smallest content size the user can resize to.
    #[must_use]
    pub fn min_size(mut self, width: f64, height: f64) -> Self {
        self.min_size = Some(Size::new(width, height));
        self
    }
    /// The largest content size the user can resize to (`Cap::WindowGeometry`).
    #[must_use]
    pub fn max_size(mut self, width: f64, height: f64) -> Self {
        self.max_size = Some(Size::new(width, height));
        self
    }
    /// Create and mount the window without showing it; see [`Self::start_hidden`].
    #[must_use]
    pub fn start_hidden(mut self) -> Self {
        self.start_hidden = true;
        self
    }
    /// Size the window to its content once built; see [`Self::size_to_fit`].
    #[must_use]
    pub fn size_to_fit(mut self) -> Self {
        self.size_to_fit = true;
        self
    }
    /// The app's display name for the application menu and About; see [`Self::app_name`].
    #[must_use]
    pub fn app_name(mut self, name: impl Into<String>) -> Self {
        self.app_name = Some(name.into());
        self
    }
    /// The app's locale catalog, as `res::locales::{DEFAULT, CATALOG}`; see [`Self::locales`].
    #[must_use]
    pub fn locales(
        mut self,
        default: &'static str,
        catalog: &'static [(&'static str, &'static str)],
    ) -> Self {
        self.locales = Some((default, catalog));
        self
    }
    /// The title, computed once the catalog is installed; see [`Self::title_fn`].
    #[must_use]
    pub fn title_fn(mut self, f: fn() -> String) -> Self {
        self.title_fn = Some(f);
        self
    }
    /// The app's version, for Day's exit line; see [`Self::version`].
    #[must_use]
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }
    /// macOS window tabbing for this window; see [`Self::tabbing`].
    #[must_use]
    pub fn tabbing(mut self, tabbing: WindowTabbing) -> Self {
        self.tabbing = tabbing;
        self
    }
    /// The window's frame; see [`Self::chrome`].
    #[must_use]
    pub fn chrome(mut self, chrome: WindowChrome) -> Self {
        self.chrome = chrome;
        self
    }
    /// What shows behind the content; see [`Self::background`].
    #[must_use]
    pub fn background(mut self, background: WindowBackground) -> Self {
        self.background = background;
        self
    }
    /// No drop shadow, for a transparent or shaped window; see [`Self::shadow`].
    #[must_use]
    pub fn no_shadow(mut self) -> Self {
        self.shadow = false;
        self
    }
    /// A window the user cannot resize; see [`Self::resizable`].
    #[must_use]
    pub fn fixed_size(mut self) -> Self {
        self.resizable = false;
        self
    }
    /// Where the window opens; see [`Self::placement`].
    #[must_use]
    pub fn placement(mut self, placement: WindowPlacement) -> Self {
        self.placement = placement;
        self
    }
    /// Remember and restore this window's frame under `key`; see [`Self::remember_frame`].
    #[must_use]
    pub fn remember_frame(mut self, key: impl Into<String>) -> Self {
        self.remember_frame = Some(key.into());
        self
    }
    /// The logical window this opens; see [`Self::key`].
    #[must_use]
    pub fn key(mut self, key: impl Into<String>) -> Self {
        self.key = Some(key.into());
        self
    }
    /// The kind of window; see [`Self::kind`].
    #[must_use]
    pub fn kind(mut self, kind: WindowKind) -> Self {
        self.kind = kind;
        self
    }
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions {
            title: "Day".into(),
            size: Size::new(480.0, 640.0),
            min_size: None,
            start_hidden: false,
            size_to_fit: false,
            app_name: None,
            locales: None,
            title_fn: None,
            version: None,
            tabbing: WindowTabbing::Automatic,
            chrome: WindowChrome::Standard,
            background: WindowBackground::Opaque,
            shadow: true,
            resizable: true,
            placement: WindowPlacement::Automatic,
            max_size: None,
            remember_frame: None,
            key: None,
            kind: WindowKind::Normal,
        }
    }
}

/// A platform backend: owns the native main loop and exactly one window in v1 (§8.1).
///
/// `run` sets up the native app + window, installs the reactive scheduler + main poster,
/// then hands `(self, root_container, content_size)` to `ready` (which mounts the tree and
/// takes ownership of the backend), and finally runs the native main loop.
pub trait Platform: Toolkit {
    /// The process-constant target id, e.g. `"macos-appkit"`.
    const TARGET: &'static str;
    /// The toolkit half of the target, e.g. `"appkit"`.
    const TOOLKIT: &'static str;
    /// Whether [`Platform::run`] returning means the app is over. True for a backend that
    /// drives the native loop itself until the app quits (GTK, Qt, XAML; AppKit's never
    /// returns). False where the host owns the loop and `run` hands back at once, with the
    /// app still to come (ArkUI's ability, Android's activity, the web page, the mock's test):
    /// their `DidExit` is the host's last callback, not the return (docs/lifecycle.md).
    const RUN_ENDS_APP: bool = true;

    fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Self::Handle, Size)>);

    /// Post a closure onto the native main loop. Callable from any thread; this is the
    /// single door the reactive scheduler and `Setter` deliveries ride (§3.3).
    fn post(f: Box<dyn FnOnce() + Send>);

    /// Post `f` onto the native main loop after (at least) `ms` milliseconds: the timer
    /// door behind `day::sleep` (docs/async.md). Every shipped backend overrides it with the
    /// loop's own timer (`dispatch_after`, `Handler.postDelayed`, `g_timeout_add`,
    /// `QTimer::singleShot`, `SetTimer`, `setTimeout`), so a sleep costs no thread. The
    /// default is the fallback for a backend without one: a helper thread that sleeps the
    /// whole delay and rides [`Platform::post`] home. It is correct, but a thread per sleep
    /// adds up under a ticking loop, so the first use says so once in the log.
    fn post_delayed(ms: u32, f: Box<dyn FnOnce() + Send>) {
        static SAID: std::sync::Once = std::sync::Once::new();
        SAID.call_once(|| {
            log::warn!(
                "day: {} has no native timer door; `day::sleep` falls back to a helper thread \
                 per delay (implement `Platform::post_delayed`)",
                Self::TARGET
            );
        });
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(u64::from(ms)));
            Self::post(f);
        });
    }

    /// Ordered OS locale preference list (BCP-47), for fluent-langneg (§12.2).
    fn locale_hints(&self) -> Vec<String> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Open renderer registry (§8.2)
// ---------------------------------------------------------------------------

/// The scale a window snapshot is rendered at, when a scripted run states one
/// (`DAY_CAPTURE_SCALE=2`, set by `day launch --script` from Day.toml `[screenshots]`).
///
/// For the backends that render their own snapshot (GTK, Qt): the capture then measures
/// window points times this, whatever display the host has, which is what makes a CI capture
/// under a 1x xvfb the same 2560x1600 pixels a HiDPI laptop produces. A backend that reads the
/// compositor's pixels back (AppKit, XAML) captures at the display's own scale and ignores it.
/// `None` when unset or unparseable: capture at the backend's natural scale.
#[must_use]
pub fn capture_scale() -> Option<f64> {
    std::env::var("DAY_CAPTURE_SCALE")
        .ok()
        .and_then(|v| v.trim().parse::<f64>().ok())
        .filter(|s| s.is_finite() && (0.5..=4.0).contains(s))
}

/// Seconds on a monotonic clock since the first call: the frame timestamp for a backend that
/// does not expose a native timestamp (§8.4). The origin is arbitrary. Not for the web
/// build, where `Instant` is unavailable and `requestAnimationFrame` stamps the frame itself.
pub fn frame_timestamp() -> f64 {
    static ORIGIN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    ORIGIN
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_secs_f64()
}

/// The user's language preference from the POSIX environment, newest-first, as BCP-47 tags.
///
/// `LANGUAGE` is the GNU multi-language list (`fr:en`); `LC_ALL`, `LC_MESSAGES` and `LANG` each
/// carry one locale in POSIX form (`fr_FR.UTF-8`), which becomes `fr-FR`. Shared by the backends
/// whose platform has no richer API to ask (§12.2, docs/localization.md).
#[must_use]
pub fn posix_locale_hints() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |tag: &str| {
        // "C" and "POSIX" are the absence of a preference, not a language.
        let tag = tag.split('.').next().unwrap_or(tag).replace('_', "-");
        if tag.is_empty() || tag == "C" || tag == "POSIX" {
            return;
        }
        if !out.contains(&tag) {
            out.push(tag);
        }
    };
    if let Ok(list) = std::env::var("LANGUAGE") {
        for tag in list.split(':') {
            push(tag);
        }
    }
    for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(key) {
            push(&v);
        }
    }
    out
}

/// Optional custom measure for a third-party piece (§8.2).
pub type MeasureFn<B> = fn(&mut B, &<B as Toolkit>::Handle, Proposal) -> Size;

/// A third-party piece's per-toolkit implementation. `make` receives the concrete backend
/// (public helper surface) and returns a native handle the backend then owns like any built-in.
pub struct Renderer<B: Toolkit> {
    pub kind: PieceKind,
    pub make: fn(&mut B, &dyn Any, NodeId) -> B::Handle,
    pub update: fn(&mut B, &B::Handle, &dyn Any),
    pub measure: Option<MeasureFn<B>>,
    /// Teardown, run from the release queue just before [`Toolkit::release`] frees the handle.
    ///
    /// A piece that keeps per-view state (a retained delegate, a session map, a native
    /// observer) has no other place to drop it: `make`/`update` are the only other hooks, and a
    /// disposed node never calls them again. Without this a piece leaks one entry per realized
    /// view, and any map keyed by the handle's address can go on to answer for a later view that
    /// the allocator hands the same address.
    ///
    /// `None` for the pieces that own nothing (most of them).
    pub release: Option<fn(&mut B, &B::Handle)>,
}

pub struct Registry<B: Toolkit> {
    map: HashMap<PieceKind, Renderer<B>>,
}

impl<B: Toolkit> Default for Registry<B> {
    fn default() -> Self {
        Registry {
            map: HashMap::new(),
        }
    }
}

impl<B: Toolkit> Registry<B> {
    pub fn register(&mut self, r: Renderer<B>) {
        let kind = r.kind;
        if self.map.insert(kind, r).is_some() {
            // Two pieces claiming one kind is last-linked-wins in link order, effectively
            // nondeterministic. Fail loudly in debug; in release, say so once at boot rather
            // than render the wrong widget silently.
            debug_assert!(
                false,
                "duplicate renderer registered for piece kind {kind:?}"
            );
            log::warn!("duplicate renderer for piece kind {kind:?} — later registration wins");
        }
    }
    #[must_use]
    pub fn get(&self, kind: PieceKind) -> Option<&Renderer<B>> {
        self.map.get(kind)
    }
    pub fn kinds(&self) -> impl Iterator<Item = PieceKind> + '_ {
        self.map.keys().copied()
    }
}

/// The canvas wire op codes (§11, §15.3): slot 0 of every 9-number record in
/// [`encode_ops`]'s output. The numeric values are the frozen wire contract: the JNI, C++,
/// and JS decoders each hand-write these as constants, so a value here may never be reused
/// or renumbered; new ops append. `ALL` drives the density test that keeps this enum and
/// the encoder in step.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum OpCode {
    /// Fill a rect (a,b=origin, c,d=size).
    FillRect = 0,
    /// Stroke a rect (g=width).
    StrokeRect = 1,
    /// Fill a rounded rect (e=radius).
    FillRrect = 2,
    /// Fill an ellipse in a rect.
    FillEllipse = 3,
    /// Stroke an ellipse (g=width).
    StrokeEllipse = 4,
    /// Stroke an arc (e=start°, f=sweep°, g=width). `Fill(Shape::Arc)` also encodes as
    /// this with width 0, the known asymmetry; see [`encode_ops`].
    StrokeArc = 5,
    /// Line a,b → c,d (g=width).
    Line = 6,
    /// Text at a,b (e=size, f=anchor: 0 leading / 1 centered); the string rides the texts
    /// channel.
    Text = 7,
    /// Save the graphics state.
    Save = 8,
    /// Restore the graphics state.
    Restore = 9,
    /// Concat an affine transform (a..f).
    Concat = 10,
    /// Fill a polygon; points ride the texts channel as "x,y x,y …" (closed automatically),
    /// or the geometry channel of [`encode_ops_with_geometry`] (b = offset, c = count, and
    /// the texts entry is empty). `a` = the [`geometry_key`] of those points, or 0 for none.
    FillPolygon = 11,
    /// Stroke a polygon (g=width); points and key as [`OpCode::FillPolygon`].
    StrokePolygon = 12,
    /// Stroke a rounded rect (e=radius, g=width).
    StrokeRrect = 13,
    /// Set a gradient for the next shape record (f=type: 0 linear with a,b=start / c,d=end
    /// unit points; 1 radial with a,b=center unit point, c=unit radius; 2 conic with
    /// a,b=center unit point, c=start angle in canvas degrees, clockwise from +x, the stops
    /// running one full turn from it ([`ConicGradient`]); e=stop count).
    /// Stops ride the texts channel as "offset,aarrggbb offset,aarrggbb …". Unit geometry
    /// resolves against the shape's bounding box.
    SetGradient = 14,
    /// Fill a path (f=fill rule: 0 non-zero / 1 even-odd); segments ride the texts channel
    /// (see [`encode_path`]), or the geometry channel of [`encode_ops_with_geometry`]
    /// (b = offset, c = count, see [`path_numbers`]; the texts entry is empty). `a` = the
    /// [`geometry_key`] of those segments, or 0 for none.
    ///
    /// The key is a cache handle, not a payload: a decoder that recognizes it may keep the
    /// native geometry it built last time and skip re-parsing the segments, but it must still
    /// consume the texts entry either way, because that channel is positional and a skipped
    /// entry desynchronizes every record after it.
    FillPath = 15,
    /// Stroke a path (g=width); segments and key as [`OpCode::FillPath`].
    StrokePath = 16,
    /// Clip to a shape (f=shape kind: 0 rect / 1 rrect(e=radius) / 2 ellipse / 3 path
    /// (e=fill rule) / 4 polygon; a..d=geometry; path/polygon payloads ride the texts
    /// channel).
    ///
    /// For the two payload-bearing kinds (3 and 4) there is no geometry in the slots, so `a`
    /// carries the [`geometry_key`] instead, b and c the geometry channel's offset and count
    /// when there is one, and d stays 0. The other three kinds put real geometry in a..d and
    /// have nothing to cache.
    Clip = 17,
    /// Stroke style for the next stroke record (a=cap, b=join, c=miter limit, d=dash
    /// phase, e=dash count; the dash array rides the texts channel).
    StrokeStyle = 18,
    /// Font for the next text record (a=CSS weight 100 … 900, 0 = default; b=italic 0/1); the
    /// family name rides the texts channel, `""` = the platform's default face. Like
    /// [`OpCode::SetGradient`] and [`OpCode::StrokeStyle`] it is consumed by the record that
    /// follows and then cleared; a default [`CanvasFont`] emits no record at all, so a drawing
    /// without fonts encodes exactly as it did before this op existed.
    SetFont = 19,
    /// The next shape record is a template, drawn once per stamped position rather than once
    /// (docs/canvas.md "Stamping"). `a` = how many positions follow. Like [`OpCode::SetGradient`],
    /// [`OpCode::StrokeStyle`] and [`OpCode::SetFont`] it is a prefix, consumed by the record it
    /// applies to and then cleared.
    ///
    /// Between this and that template record come `ceil(a / 4)` [`OpCode::StampPoints`] records
    /// carrying the coordinates. A drawing with no stamps encodes exactly as it did before.
    Stamp = 20,
    /// Four stamped positions, filling the record edge to edge: `[op, x0, y0, x1, y1, x2, y2,
    /// x3, y3]`, the one record that uses its last slot for geometry instead of a color, because
    /// it carries no color of its own. The final record of a run is padded; the count in the
    /// [`OpCode::Stamp`] header says how many of the pairs are real.
    StampPoints = 21,
    /// Draw a decoded image: `a,b` = origin, `c,d` = size, `e` = the [`BitmapId`], `f` = opacity
    /// in `0.0..=1.0` (docs/images.md).
    ///
    /// The id rides the numeric channel because it is a number, and an `f64` carries a `u64` id
    /// exactly up to 2^53; ids are minted from 1 per process, so that ceiling is unreachable.
    /// Nothing rides the texts channel for this op.
    Image = 22,
    /// The source region for the next image record (`a,b` = origin, `c,d` = size, in the
    /// bitmap's own pixels): [`DrawOp::Image`]'s `source`. Like [`OpCode::SetFont`] it is a
    /// prefix, consumed by the record that follows and then cleared, and a whole-image draw
    /// emits none, so a drawing without regions encodes exactly as it did before this op
    /// existed. A decoder clamps the region to the bitmap and draws nothing for an empty
    /// intersection ([`clamp_image_source`]).
    ImageSource = 23,
    /// Set the shadow cast by every drawing record that follows: `a,b` = offset in points,
    /// `c` = blur radius (the CSS `box-shadow` blur, twice the Gaussian's standard deviation),
    /// the color slot = the shadow's color. A fully transparent color clears the shadow.
    ///
    /// Unlike the prefix records this is graphics state: it stays set until another
    /// [`OpCode::SetShadow`] or the [`OpCode::Restore`] that matches the [`OpCode::Save`] it was
    /// set under, so a decoder whose native context does not save it must keep its own stack.
    /// The offset and blur are not transformed by [`OpCode::Concat`].
    SetShadow = 24,
    /// Set the blend mode for every drawing record that follows: `a` = [`BlendMode::wire`]
    /// (0 normal, 1 multiply, 2 screen, 3 overlay, 4 darken, 5 lighten, 6 add, 7 erase).
    /// Graphics state, scoped by save and restore exactly as [`OpCode::SetShadow`] is. A decoder
    /// that meets a number it does not know draws with the normal mode.
    SetBlend = 25,
}

impl OpCode {
    /// Every code, in wire order. The density test iterates this so a new variant that
    /// forgets to join fails loudly.
    pub const ALL: [OpCode; 26] = [
        OpCode::FillRect,
        OpCode::StrokeRect,
        OpCode::FillRrect,
        OpCode::FillEllipse,
        OpCode::StrokeEllipse,
        OpCode::StrokeArc,
        OpCode::Line,
        OpCode::Text,
        OpCode::Save,
        OpCode::Restore,
        OpCode::Concat,
        OpCode::FillPolygon,
        OpCode::StrokePolygon,
        OpCode::StrokeRrect,
        OpCode::SetGradient,
        OpCode::FillPath,
        OpCode::StrokePath,
        OpCode::Clip,
        OpCode::StrokeStyle,
        OpCode::SetFont,
        OpCode::Stamp,
        OpCode::StampPoints,
        OpCode::Image,
        OpCode::ImageSource,
        OpCode::SetShadow,
        OpCode::SetBlend,
    ];

    /// The code back from a wire number (a decoder-side aid and the round-trip test's
    /// reference); `None` for anything outside the table.
    #[must_use]
    #[expect(
        clippy::float_cmp,
        reason = "wire codes are small integers, which f64 represents exactly"
    )]
    pub fn from_wire(n: f64) -> Option<OpCode> {
        OpCode::ALL.into_iter().find(|c| f64::from(*c as i32) == n)
    }
}

/// Flat numeric encoding of a display list for shim/JNI boundaries (§11, §15.3): per op
/// 9 numbers [kind, a, b, c, d, e, f, g, rgba-bits]; text payloads ride separately in order.
/// The kind slot carries an [`OpCode`]; its doc comments are the per-slot contract.
///
/// Transports join `texts` with the unit separator U+001F (one entry per text-carrying
/// record, in order), so text payloads must not contain U+001F. Known asymmetry:
/// `Fill(Shape::Arc)` encodes as [`OpCode::StrokeArc`] with width 0. Filled arcs render
/// only on the direct-replay backends (AppKit/UIKit); use a polygon fan if a filled arc
/// must be portable.
/// Serialize a path for the `texts` side-channel: one space-separated token per segment,
/// `M x y` / `L x y` / `Q cx cy x y` / `C c1x c1y c2x c2y x y` / `Z`.
///
/// Text rather than numbers because that is the channel the encoder already has for
/// variable-length payloads, and every decoder (JS, Java, C++) can already split a string. The
/// numeric channel is a flat `[f64; 9]`-per-record array with no length prefix, so a path could
/// not ride it without changing the record shape for every backend at once.
/// A content key for a shape's variable-length geometry: the handle a backend caches decoded
/// native geometry under (a `Path2D`, a `QPainterPath`, an `android.graphics.Path`) so an
/// unchanged path is rebuilt once instead of re-parsed out of the texts channel every frame.
///
/// It hashes the geometry, not the op's position or its identity, so that a draw closure that
/// rebuilds its `PathBuilder` chain from scratch on every frame (which is what canvas consumers
/// do) produces an equal path, hashes the same and hits the same entry. An identity would miss
/// every time.
///
/// Two different paths that collided would draw each other, so the width matters: FNV-1a over the
/// IEEE bits, masked to the 53 bits an `f64` wire slot carries exactly. `0.0` is reserved for "no
/// key": a decoder seeing it must decode the payload as it always did, which is also what every
/// decoder written before this key existed does with the slot.
#[must_use]
pub fn geometry_key(shape: &Shape) -> f64 {
    fn mix(h: &mut u64, v: f64) {
        for byte in v.to_bits().to_le_bytes() {
            *h ^= u64::from(byte);
            *h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    fn mix_point(h: &mut u64, p: Point) {
        mix(h, p.x);
        mix(h, p.y);
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    match shape {
        Shape::Polygon(pts) => {
            // The discriminant joins the hash so a polygon and a path of the same points can
            // never share an entry: they decode to different geometry.
            mix(&mut h, 11.0);
            for p in pts {
                mix_point(&mut h, *p);
            }
        }
        Shape::Path(path) => {
            mix(&mut h, 15.0);
            mix(&mut h, rule_bits(path.rule));
            for seg in &path.segs {
                match seg {
                    PathSeg::Move(a) => {
                        mix(&mut h, 0.0);
                        mix_point(&mut h, *a);
                    }
                    PathSeg::Line(a) => {
                        mix(&mut h, 1.0);
                        mix_point(&mut h, *a);
                    }
                    PathSeg::Quad(c, a) => {
                        mix(&mut h, 2.0);
                        mix_point(&mut h, *c);
                        mix_point(&mut h, *a);
                    }
                    PathSeg::Cubic(c1, c2, a) => {
                        mix(&mut h, 3.0);
                        mix_point(&mut h, *c1);
                        mix_point(&mut h, *c2);
                        mix_point(&mut h, *a);
                    }
                    PathSeg::Close => mix(&mut h, 4.0),
                }
            }
        }
        // Everything else puts its whole geometry in the record's own slots, so there is nothing
        // to parse and nothing worth caching.
        _ => return 0.0,
    }
    let masked = h & ((1u64 << 53) - 1);
    // 0 means "no key", so the one hash that lands there borrows its neighbor.
    if masked == 0 { 1.0 } else { masked as f64 }
}

/// Append a coordinate to the texts channel: a plain decimal with up to six fractional digits
/// and no trailing zeros (`12`, `-0.5`, `183.472938`), never an exponent.
///
/// Six decimals of a canvas point is a millionth of a point, far below anything a transform an
/// app would draw under can bring to a pixel, and the fixed form costs a tenth of the shortest
/// round-trip repr: one integer write instead of a float formatter and a temporary `String` per
/// coordinate, which was the larger half of a map tile's export on Android. The decoders parse
/// with their platform's general float reader, so a value the fixed form cannot carry (a
/// non-finite number, a magnitude past 2^53 millionths) falls back to `Display`.
fn push_wire_num(out: &mut String, v: f64) {
    use std::fmt::Write as _;
    const SCALE: f64 = 1_000_000.0;
    let scaled = (v * SCALE).round();
    if scaled.is_nan() || scaled.abs() >= 9.0e15 {
        let _ = write!(out, "{v}");
        return;
    }
    let mut n = scaled as i64;
    if n < 0 {
        out.push('-');
        n = -n;
    }
    let whole = n / 1_000_000;
    let mut frac = n % 1_000_000;
    let _ = write!(out, "{whole}");
    if frac != 0 {
        let mut digits = 6;
        while frac % 10 == 0 {
            frac /= 10;
            digits -= 1;
        }
        out.push('.');
        let _ = write!(out, "{frac:0digits$}");
    }
}

/// Polygon points for the texts channel: `"x,y x,y …"`, numbers as [`push_wire_num`] writes
/// them.
fn encode_points(pts: &[Point]) -> String {
    let mut out = String::with_capacity(pts.len() * 16);
    for p in pts {
        if !out.is_empty() {
            out.push(' ');
        }
        push_wire_num(&mut out, p.x);
        out.push(',');
        push_wire_num(&mut out, p.y);
    }
    out
}

/// Path segments for the texts channel: `"M x y L x y Q cx cy x y C … Z"`, one token per
/// command or number. Coordinates use up to six decimal places, without trailing zeros;
/// non-finite values and magnitudes outside the fixed-point range use their display form.
#[must_use]
pub fn encode_path(path: &Path) -> String {
    let mut out = String::with_capacity(path.segs.len() * 16);
    let points = |out: &mut String, cmd: char, pts: &[Point]| {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push(cmd);
        for p in pts {
            out.push(' ');
            push_wire_num(out, p.x);
            out.push(' ');
            push_wire_num(out, p.y);
        }
    };
    for seg in &path.segs {
        match seg {
            PathSeg::Move(p) => points(&mut out, 'M', &[*p]),
            PathSeg::Line(p) => points(&mut out, 'L', &[*p]),
            PathSeg::Quad(c, p) => points(&mut out, 'Q', &[*c, *p]),
            PathSeg::Cubic(a, b, p) => points(&mut out, 'C', &[*a, *b, *p]),
            PathSeg::Close => points(&mut out, 'Z', &[]),
        }
    }
    out
}

/// Clamp a [`DrawOp::Image`] `source` region to the bitmap it indexes (`pixels` wide and high),
/// keeping the region's mapping into `rect`: the answer is the part of the region inside the
/// bitmap and the part of `rect` it lands in.
///
/// The single definition of what an out-of-range region means. The mapping is kept rather than
/// the clamped part stretched over the whole of `rect`, because that is what every native
/// sub-rect draw does on its own (Qt, the HTML canvas) and what the clip-and-scale fallback
/// gives for free, so the backends agree. `None` (an empty region, an empty intersection, or a
/// bitmap with no pixels) draws nothing. Every backend that replays in Rust calls this; the
/// C++/Java/JS decoders repeat the same arithmetic.
#[must_use]
pub fn clamp_image_source(source: Rect, rect: Rect, pixels: Size) -> Option<(Rect, Rect)> {
    if !(source.size.width > 0.0 && source.size.height > 0.0) {
        return None;
    }
    let x0 = source.origin.x.max(0.0);
    let y0 = source.origin.y.max(0.0);
    let x1 = (source.origin.x + source.size.width).min(pixels.width);
    let y1 = (source.origin.y + source.size.height).min(pixels.height);
    if !(x1 > x0 && y1 > y0) {
        return None;
    }
    let kx = rect.size.width / source.size.width;
    let ky = rect.size.height / source.size.height;
    let dest = Rect::new(
        rect.origin.x + (x0 - source.origin.x) * kx,
        rect.origin.y + (y0 - source.origin.y) * ky,
        (x1 - x0) * kx,
        (y1 - y0) * ky,
    );
    Some((Rect::new(x0, y0, x1 - x0, y1 - y0), dest))
}

/// `FillRule` as a wire number: 0 non-zero, 1 even-odd.
fn rule_bits(rule: FillRule) -> f64 {
    match rule {
        FillRule::NonZero => 0.0,
        FillRule::EvenOdd => 1.0,
    }
}

/// Encode a display list for a decoder written in another language (Java, C++, JS): the
/// numbers channel, nine `f64` slots per record (`[kind, a, b, c, d, e, f, g, color]`, kinds
/// from [`OpCode`]), and the texts channel, one entry per record that carries text or
/// variable-length geometry, in record order.
#[must_use]
pub fn encode_ops(ops: &[DrawOp]) -> (Vec<f64>, Vec<String>) {
    encode_ops_into(ops, None)
}

/// [`encode_ops`] with a third, geometry channel: the points of every path and polygon (fills,
/// strokes, clips) as numbers instead of text. Such a record's slot `b` is its offset into the
/// channel and slot `c` its count, and its texts entry is empty, so that channel stays
/// positional for a decoder that ignores the third one. Paths are written by
/// [`path_numbers`], polygons by [`point_numbers`].
///
/// Formatting a coordinate and parsing it back were, together, half of what a map tile's export
/// cost on Android (docs/canvas.md "Exporting"); a decoder that can take an array of doubles
/// across its boundary skips both.
#[must_use]
pub fn encode_ops_with_geometry(ops: &[DrawOp]) -> (Vec<f64>, Vec<String>, Vec<f64>) {
    let mut geometry = Vec::new();
    let (nums, texts) = encode_ops_into(ops, Some(&mut geometry));
    (nums, texts, geometry)
}

/// A path's segments for the geometry channel: a command code, then its points — `0` move
/// (x y), `1` line (x y), `2` quad (cx cy x y), `3` cubic (x1 y1 x2 y2 x y), `4` close.
pub fn path_numbers(path: &Path, out: &mut Vec<f64>) {
    for seg in &path.segs {
        match seg {
            PathSeg::Move(p) => out.extend_from_slice(&[0.0, p.x, p.y]),
            PathSeg::Line(p) => out.extend_from_slice(&[1.0, p.x, p.y]),
            PathSeg::Quad(c, p) => out.extend_from_slice(&[2.0, c.x, c.y, p.x, p.y]),
            PathSeg::Cubic(a, b, p) => out.extend_from_slice(&[3.0, a.x, a.y, b.x, b.y, p.x, p.y]),
            PathSeg::Close => out.push(4.0),
        }
    }
}

/// A polygon's points for the geometry channel: x then y, per point.
pub fn point_numbers(pts: &[Point], out: &mut Vec<f64>) {
    for p in pts {
        out.push(p.x);
        out.push(p.y);
    }
}

/// Put a path's or polygon's points on the wire: on the geometry channel when the caller opened
/// one, as `numbers` writes them, answering (offset, count) for slots b and c and leaving an
/// empty texts entry; else as `text` on the texts channel, answering (0, 0).
fn geometry_payload(
    geometry: &mut Option<&mut Vec<f64>>,
    texts: &mut Vec<String>,
    text: impl FnOnce() -> String,
    numbers: impl FnOnce(&mut Vec<f64>),
) -> (f64, f64) {
    if let Some(g) = geometry {
        let offset = g.len();
        numbers(g);
        texts.push(String::new());
        (offset as f64, (g.len() - offset) as f64)
    } else {
        texts.push(text());
        (0.0, 0.0)
    }
}

fn encode_ops_into(ops: &[DrawOp], mut geometry: Option<&mut Vec<f64>>) -> (Vec<f64>, Vec<String>) {
    fn color_bits(c: Color) -> f64 {
        let r = (c.r.clamp(0.0, 1.0) * 255.0) as u32;
        let g = (c.g.clamp(0.0, 1.0) * 255.0) as u32;
        let b = (c.b.clamp(0.0, 1.0) * 255.0) as u32;
        let a = (c.a.clamp(0.0, 1.0) * 255.0) as u32;
        f64::from((a << 24) | (r << 16) | (g << 8) | b)
    }
    #[allow(clippy::too_many_arguments)]
    fn push(
        k: OpCode,
        a: f64,
        b: f64,
        c: f64,
        d: f64,
        e: f64,
        f: f64,
        g: f64,
        col: Color,
        nums: &mut Vec<f64>,
    ) {
        nums.extend_from_slice(&[f64::from(k as i32), a, b, c, d, e, f, g, color_bits(col)]);
    }
    /// The kind-18 prefix a non-plain stroke style emits, applying to the next stroke record,
    /// the same "modifier record" rule the gradient and font records follow. A plain style emits
    /// nothing, so a drawing without dashes or caps encodes exactly as it always did.
    fn stroke_style_record(style: &StrokeStyle, nums: &mut Vec<f64>, texts: &mut Vec<String>) {
        if style.is_plain() {
            return;
        }
        push(
            OpCode::StrokeStyle,
            match style.cap {
                LineCap::Butt => 0.0,
                LineCap::Round => 1.0,
                LineCap::Square => 2.0,
            },
            match style.join {
                LineJoin::Miter => 0.0,
                LineJoin::Round => 1.0,
                LineJoin::Bevel => 2.0,
            },
            style.miter_limit,
            style.dash_phase,
            style.dash.len() as f64,
            0.0,
            0.0,
            Color::CLEAR,
            nums,
        );
        texts.push(
            style
                .dash
                .iter()
                .map(std::string::ToString::to_string)
                .collect::<Vec<_>>()
                .join(" "),
        );
    }

    /// One shape record (the fill/stroke kinds shared by both ops).
    fn shape_record(
        stroke: bool,
        shape: &Shape,
        w: f64,
        col: Color,
        nums: &mut Vec<f64>,
        texts: &mut Vec<String>,
        geometry: &mut Option<&mut Vec<f64>>,
    ) {
        match shape {
            Shape::Rect(r) => push(
                if stroke {
                    OpCode::StrokeRect
                } else {
                    OpCode::FillRect
                },
                r.origin.x,
                r.origin.y,
                r.size.width,
                r.size.height,
                0.0,
                0.0,
                w,
                col,
                nums,
            ),
            Shape::RoundedRect(r, rad) => push(
                if stroke {
                    OpCode::StrokeRrect
                } else {
                    OpCode::FillRrect
                },
                r.origin.x,
                r.origin.y,
                r.size.width,
                r.size.height,
                *rad,
                0.0,
                w,
                col,
                nums,
            ),
            Shape::Ellipse(r) => push(
                if stroke {
                    OpCode::StrokeEllipse
                } else {
                    OpCode::FillEllipse
                },
                r.origin.x,
                r.origin.y,
                r.size.width,
                r.size.height,
                0.0,
                0.0,
                w,
                col,
                nums,
            ),
            Shape::Arc { rect, start, sweep } => push(
                OpCode::StrokeArc,
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
                start.degrees(),
                sweep.degrees(),
                w,
                col,
                nums,
            ),
            Shape::Line(p1, p2) => {
                push(OpCode::Line, p1.x, p1.y, p2.x, p2.y, 0.0, 0.0, w, col, nums);
            }
            Shape::Polygon(pts) => {
                // Variable-length points ride the texts side-channel ("x,y x,y …"),
                // consumed in record order exactly like text payloads, or the geometry
                // channel when there is one (slots b, c).
                let (offset, count) = geometry_payload(
                    geometry,
                    texts,
                    || encode_points(pts),
                    |g| point_numbers(pts, g),
                );
                push(
                    if stroke {
                        OpCode::StrokePolygon
                    } else {
                        OpCode::FillPolygon
                    },
                    geometry_key(shape),
                    offset,
                    count,
                    0.0,
                    0.0,
                    0.0,
                    w,
                    col,
                    nums,
                );
            }
            Shape::Path(path) => {
                // Same side-channels as Polygon, one token per segment (see `encode_path`).
                // The fill rule rides slot f; stroking ignores it.
                let (offset, count) = geometry_payload(
                    geometry,
                    texts,
                    || encode_path(path),
                    |g| path_numbers(path, g),
                );
                push(
                    if stroke {
                        OpCode::StrokePath
                    } else {
                        OpCode::FillPath
                    },
                    geometry_key(shape),
                    offset,
                    count,
                    0.0,
                    0.0,
                    rule_bits(path.rule),
                    w,
                    col,
                    nums,
                );
            }
        }
    }
    /// A gradient applies to the next shape record, fill or stroke. Geometry per type rides
    /// slots a..d and the type discriminant slot f: one record shape, so every decoder keeps a
    /// single gradient code path.
    fn gradient_record(
        geo: [f64; 4],
        kind: f64,
        stops: &[(f64, Ink)],
        nums: &mut Vec<f64>,
        texts: &mut Vec<String>,
    ) {
        push(
            OpCode::SetGradient,
            geo[0],
            geo[1],
            geo[2],
            geo[3],
            stops.len() as f64,
            kind,
            0.0,
            Color::CLEAR,
            nums,
        );
        texts.push(
            stops
                .iter()
                .map(|(o, c)| format!("{o},{:08x}", color_bits(c.resolved()) as u32))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    /// The record a paint puts before its shape record, and the color that shape record
    /// carries. A gradient emits one kind-14 record; a solid color emits nothing.
    fn paint_record(paint: &Paint, nums: &mut Vec<f64>, texts: &mut Vec<String>) -> Color {
        let (geo, kind, stops) = match paint {
            Paint::Solid(c) => return c.resolved(),
            Paint::Linear(g) => ([g.start.x, g.start.y, g.end.x, g.end.y], 0.0, &g.stops),
            Paint::Radial(g) => ([g.center.x, g.center.y, g.radius, 0.0], 1.0, &g.stops),
            Paint::Conic(g) => (
                [g.center.x, g.center.y, g.start.degrees(), 0.0],
                2.0,
                &g.stops,
            ),
        };
        gradient_record(geo, kind, stops, nums, texts);
        // The gradient replaces the shape record's color, but it must be opaque, not clear:
        // Skia-based decoders (Android Paint, OH_Drawing) modulate a shader by the paint
        // alpha, so a clear slot would render the whole gradient invisible.
        Color::WHITE
    }
    let mut nums = Vec::with_capacity(ops.len() * 9);
    let mut texts = Vec::new();
    for op in ops {
        match op {
            DrawOp::Fill(shape, paint) => {
                // A gradient emits one kind-14 set-gradient record before its shape record;
                // the stops ride the texts channel as "offset,aarrggbb offset,aarrggbb …".
                // Geometry per type rides slots a..d, the type discriminant slot f: one
                // record shape, so every decoder keeps a single gradient code path.
                let col = paint_record(paint, &mut nums, &mut texts);
                shape_record(false, shape, 0.0, col, &mut nums, &mut texts, &mut geometry);
            }
            DrawOp::Stamp(st) => {
                // No copies means no drawing. Some native decoders use an empty point
                // buffer to mean an ordinary (unstamped) template, so omit the entire op.
                if st.at.is_empty() {
                    continue;
                }
                // A prefix record, then the coordinates, then the template shape record: the
                // "applies to the next record" rule the gradient, stroke style and font records
                // already follow, so a decoder needs no new kind of state, only one more slot in
                // the state it keeps. The template goes last because a decoder has to hold the
                // points before it can draw with them.
                //
                // The points are numbers on the numeric channel, not the texts channel a
                // polygon's points ride: this op exists for tens of thousands of them, and
                // formatting that many floats into a string every frame would cost more than
                // the individual ops it replaces.
                let col = paint_record(&st.paint, &mut nums, &mut texts);
                push(
                    OpCode::Stamp,
                    st.at.len() as f64,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    Color::CLEAR,
                    &mut nums,
                );
                for chunk in st.at.chunks(4) {
                    // Written directly rather than through `push`: this is the one record whose
                    // last slot is a coordinate rather than a color, which is what lets four
                    // points fit with nothing wasted.
                    let mut rec = [
                        f64::from(OpCode::StampPoints as i32),
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                    ];
                    for (i, p) in chunk.iter().enumerate() {
                        rec[1 + i * 2] = p.x;
                        rec[2 + i * 2] = p.y;
                    }
                    nums.extend_from_slice(&rec);
                }
                match &st.stroke {
                    None => shape_record(
                        false,
                        &st.shape,
                        0.0,
                        col,
                        &mut nums,
                        &mut texts,
                        &mut geometry,
                    ),
                    Some(style) => {
                        stroke_style_record(style, &mut nums, &mut texts);
                        shape_record(
                            true,
                            &st.shape,
                            style.width,
                            col,
                            &mut nums,
                            &mut texts,
                            &mut geometry,
                        );
                    }
                }
            }
            DrawOp::Stroke(shape, paint, style) => {
                // A styled stroke emits one kind-18 record first, applying to the next stroke
                // only, the same "modifier record" shape the gradient uses, so decoders keep
                // one rule: consume, apply to the next shape record, reset.
                stroke_style_record(style, &mut nums, &mut texts);
                let col = paint_record(paint, &mut nums, &mut texts);
                shape_record(
                    true,
                    shape,
                    style.width,
                    col,
                    &mut nums,
                    &mut texts,
                    &mut geometry,
                );
            }
            DrawOp::Clip(shape) => {
                // One record for every clip shape, with the shape kind in slot f and the
                // geometry in a..e; a path or polygon puts its points on the texts channel.
                // Encoding clips as a variant of `shape_record` instead would need a third
                // kind code per shape, which is five more numbers every decoder must learn.
                let (kind, geo, extra, payload): (f64, [f64; 4], f64, Option<String>) = match shape
                {
                    Shape::Rect(r) => (
                        0.0,
                        [r.origin.x, r.origin.y, r.size.width, r.size.height],
                        0.0,
                        None,
                    ),
                    Shape::RoundedRect(r, rad) => (
                        1.0,
                        [r.origin.x, r.origin.y, r.size.width, r.size.height],
                        *rad,
                        None,
                    ),
                    Shape::Ellipse(r) => (
                        2.0,
                        [r.origin.x, r.origin.y, r.size.width, r.size.height],
                        0.0,
                        None,
                    ),
                    // Slot a carries the geometry key for these two sub-kinds only. The other
                    // three put real geometry there, which is why the key rides the same slot
                    // the payload-bearing kinds leave empty; b and c are the geometry
                    // channel's offset and count when there is one.
                    Shape::Path(p) => {
                        let (offset, count) = geometry_payload(
                            &mut geometry,
                            &mut texts,
                            || encode_path(p),
                            |g| path_numbers(p, g),
                        );
                        (
                            3.0,
                            [geometry_key(shape), offset, count, 0.0],
                            rule_bits(p.rule),
                            None,
                        )
                    }
                    Shape::Polygon(pts) => {
                        let (offset, count) = geometry_payload(
                            &mut geometry,
                            &mut texts,
                            || encode_points(pts),
                            |g| point_numbers(pts, g),
                        );
                        (4.0, [geometry_key(shape), offset, count, 0.0], 0.0, None)
                    }
                    // A clip to a line or an arc has no interior to clip to. Clipping to an
                    // empty region would blank everything after it, so clip to the shape's
                    // bounds instead: wrong in the same direction as no clip at all.
                    Shape::Line(..) | Shape::Arc { .. } => {
                        let r = shape.bounds();
                        (
                            0.0,
                            [r.origin.x, r.origin.y, r.size.width, r.size.height],
                            0.0,
                            None,
                        )
                    }
                };
                push(
                    OpCode::Clip,
                    geo[0],
                    geo[1],
                    geo[2],
                    geo[3],
                    extra,
                    kind,
                    0.0,
                    Color::CLEAR,
                    &mut nums,
                );
                if let Some(s) = payload {
                    texts.push(s);
                }
            }
            DrawOp::Text {
                text,
                at,
                size,
                color,
                anchor,
                font,
            } => {
                if *font != CanvasFont::default() {
                    push(
                        OpCode::SetFont,
                        f64::from(font.css_weight()),
                        f64::from(u8::from(font.italic)),
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        0.0,
                        Color::CLEAR,
                        &mut nums,
                    );
                    texts.push(
                        font.family_str()
                            .replace([FONT_LIST_FIELD, FONT_LIST_FAMILY], " "),
                    );
                }
                push(
                    OpCode::Text,
                    at.x,
                    at.y,
                    0.0,
                    0.0,
                    *size,
                    anchor.pack(),
                    0.0,
                    *color,
                    &mut nums,
                );
                texts.push(text.clone());
            }
            DrawOp::Image {
                image,
                rect,
                opacity,
                source,
            } => {
                if let Some(src) = source {
                    push(
                        OpCode::ImageSource,
                        src.origin.x,
                        src.origin.y,
                        src.size.width,
                        src.size.height,
                        0.0,
                        0.0,
                        0.0,
                        Color::CLEAR,
                        &mut nums,
                    );
                }
                push(
                    OpCode::Image,
                    rect.origin.x,
                    rect.origin.y,
                    rect.size.width,
                    rect.size.height,
                    image.0 as f64,
                    *opacity,
                    0.0,
                    Color::CLEAR,
                    &mut nums,
                );
            }
            DrawOp::Shadow(shadow) => {
                // No shadow is a clear color: one record shape either way, and a decoder
                // needs no second "is there one" slot.
                let s = shadow.unwrap_or(Shadow::new(Color::CLEAR, 0.0));
                push(
                    OpCode::SetShadow,
                    s.dx,
                    s.dy,
                    s.blur.max(0.0),
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    s.color.resolved(),
                    &mut nums,
                );
            }
            DrawOp::Blend(mode) => push(
                OpCode::SetBlend,
                f64::from(mode.wire()),
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                Color::CLEAR,
                &mut nums,
            ),
            DrawOp::Save => push(
                OpCode::Save,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                Color::CLEAR,
                &mut nums,
            ),
            DrawOp::Restore => push(
                OpCode::Restore,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                0.0,
                Color::CLEAR,
                &mut nums,
            ),
            DrawOp::Concat(m) => push(
                OpCode::Concat,
                m.a,
                m.b,
                m.c,
                m.d,
                m.tx,
                m.ty,
                0.0,
                Color::CLEAR,
                &mut nums,
            ),
        }
    }
    (nums, texts)
}

#[cfg(test)]
mod font_list_tests {
    use super::*;

    #[test]
    fn css_weights_round_trip_and_snap_to_the_nearest_rung() {
        for w in FontWeight::ALL {
            assert_eq!(FontWeight::from_css(w.css()), w);
        }
        assert_eq!(FontWeight::from_css(0), FontWeight::UltraLight);
        assert_eq!(FontWeight::from_css(350), FontWeight::Regular); // ties go heavier
        assert_eq!(FontWeight::from_css(649), FontWeight::Semibold);
        assert_eq!(FontWeight::from_css(1200), FontWeight::Black);
    }

    fn sans() -> FontFamilyInfo {
        FontFamilyInfo {
            family: "Day Sans".into(),
            faces: vec![
                FontFace {
                    name: "Regular".into(),
                    weight: FontWeight::Regular,
                    italic: false,
                },
                FontFace {
                    name: "Bold".into(),
                    weight: FontWeight::Bold,
                    italic: false,
                },
                FontFace {
                    name: "Light Italic".into(),
                    weight: FontWeight::Light,
                    italic: true,
                },
            ],
        }
    }

    #[test]
    fn face_for_prefers_the_slant_then_the_nearest_weight() {
        let f = sans();
        assert!(f.has_bold() && f.has_italic());
        assert_eq!(f.face_for(FontWeight::Bold, false).unwrap().name, "Bold");
        assert_eq!(
            f.face_for(FontWeight::Semibold, false).unwrap().name,
            "Bold"
        );
        assert_eq!(
            f.face_for(FontWeight::Medium, false).unwrap().name,
            "Regular"
        );
        let tie = FontFamilyInfo {
            family: "Tie".into(),
            faces: vec![
                FontFace {
                    name: "Light".into(),
                    weight: FontWeight::Light,
                    italic: false,
                },
                FontFace {
                    name: "Bold".into(),
                    weight: FontWeight::Bold,
                    italic: false,
                },
            ],
        };
        // Medium sits 200 from both; the heavier face wins the tie.
        assert_eq!(
            tie.face_for(FontWeight::Medium, false).unwrap().name,
            "Bold"
        );
        assert_eq!(
            f.face_for(FontWeight::Black, true).unwrap().name,
            "Light Italic"
        );
        let none = FontFamilyInfo {
            family: "Empty".into(),
            faces: vec![],
        };
        assert!(none.face_for(FontWeight::Regular, false).is_none());
        assert!(!none.has_bold());
    }

    #[test]
    fn the_shim_list_format_decodes_tolerantly() {
        let s = "Day Sans\u{1f}Regular\u{1f}400\u{1f}0\u{1f}\u{1f}700\u{1f}1\u{1f}Broken\u{1f}x\u{1f}0\
                 \u{1e}\u{1e}Day Mono\u{1f}Regular\u{1f}400\u{1f}0\u{1e}Faceless";
        let list = parse_font_list(s);
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].family, "Day Sans");
        assert_eq!(list[0].faces.len(), 2, "the bad weight is skipped");
        assert_eq!(list[0].faces[1].name, "Bold Italic"); // synthesized from 700 + italic
        assert_eq!(list[1].family, "Day Mono");
        assert_eq!(list[2].family, "Faceless");
        assert_eq!(list[2].faces, [] as [FontFace; 0]);
    }

    #[test]
    fn approximate_metrics_scale_with_size_and_length() {
        let m = TextMetrics::approximate("abc", 10.0);
        assert_eq!((m.width, m.height, m.ascent), (18.0, 12.0, 9.0));
        assert_eq!(TextMetrics::approximate("", 10.0).width, 0.0);
    }

    #[test]
    fn a_default_canvas_font_is_the_platform_face() {
        let f = CanvasFont::default();
        assert_eq!((f.family_str(), f.css_weight(), f.italic), ("", 0, false));
    }
}

#[cfg(test)]
mod image_header_tests {
    use super::*;

    /// A 4×2 PNG: signature, then an IHDR whose width and height are big-endian.
    fn png(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        v.extend(13u32.to_be_bytes());
        v.extend(b"IHDR");
        v.extend(w.to_be_bytes());
        v.extend(h.to_be_bytes());
        v
    }

    /// A JPEG whose frame header sits behind one other segment, so the walk has to skip it.
    fn jpeg(w: u16, h: u16) -> Vec<u8> {
        let mut v = vec![0xFF, 0xD8]; // SOI
        v.extend([0xFF, 0xE0, 0x00, 0x04, 0x00, 0x00]); // APP0, length 4
        v.extend([0xFF, 0xC0, 0x00, 0x11, 0x08]); // SOF0, length 17, precision 8
        v.extend(h.to_be_bytes());
        v.extend(w.to_be_bytes());
        v
    }

    #[test]
    fn a_header_states_the_size_before_anything_decodes() {
        assert_eq!(ImageFormat::dimensions(&png(512, 512)), Some((512, 512)));
        assert_eq!(ImageFormat::dimensions(&png(1, 65535)), Some((1, 65535)));
        // The JPEG walk must step over the APP0 segment to reach the frame header.
        assert_eq!(ImageFormat::dimensions(&jpeg(640, 480)), Some((640, 480)));

        let mut gif = b"GIF89a".to_vec();
        gif.extend(320u16.to_le_bytes());
        gif.extend(240u16.to_le_bytes());
        assert_eq!(ImageFormat::dimensions(&gif), Some((320, 240)));

        // A top-down BMP states its height as a negative number; the size is still positive.
        let mut bmp = b"BM".to_vec();
        bmp.resize(18, 0);
        bmp.extend(64i32.to_le_bytes());
        bmp.extend((-32i32).to_le_bytes());
        assert_eq!(ImageFormat::dimensions(&bmp), Some((64, 32)));
    }

    #[test]
    fn a_truncated_or_unknown_header_answers_nothing_rather_than_a_guess() {
        assert_eq!(ImageFormat::dimensions(&[]), None);
        // The signature is there but the IHDR is not.
        assert_eq!(ImageFormat::dimensions(&png(8, 8)[..12]), None);
        // Not an image at all.
        assert_eq!(ImageFormat::dimensions(b"not an image"), None);
        // A JPEG that never reaches a frame header must not loop or invent a size.
        assert_eq!(
            ImageFormat::dimensions(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0, 0]),
            None
        );
    }
}

#[cfg(test)]
mod encode_ops_tests {
    use super::*;

    /// The texts channel carries plain decimals: whole numbers without a point, fractions to
    /// six places with the trailing zeros gone, and the general form only past what the fixed
    /// one can hold.
    #[test]
    fn wire_numbers_are_short_plain_decimals() {
        let text = |v: f64| {
            let mut out = String::new();
            push_wire_num(&mut out, v);
            out
        };
        assert_eq!(text(12.0), "12");
        assert_eq!(text(-0.5), "-0.5");
        assert_eq!(text(183.472_938_472_938_4), "183.472938");
        assert_eq!(text(0.1 + 0.2), "0.3");
        assert_eq!(text(-0.000_000_1), "0");
        assert_eq!(text(2.000_05), "2.00005");
        assert_eq!(text(f64::INFINITY), "inf");
        assert_eq!(text(1.0e17), "100000000000000000");
        let path = Path {
            segs: vec![
                PathSeg::Move(Point::new(1.5, 2.0)),
                PathSeg::Line(Point::new(-3.25, 4.0)),
                PathSeg::Close,
            ],
            rule: FillRule::NonZero,
        };
        assert_eq!(encode_path(&path), "M 1.5 2 L -3.25 4 Z");
        assert_eq!(
            encode_points(&[Point::new(0.0, 0.0), Point::new(10.0, 0.125)]),
            "0,0 10,0.125"
        );
    }

    /// With the geometry channel open, a path's and a polygon's points leave the texts channel
    /// (an empty entry keeps it positional) for the numbers, with offset and count in slots b
    /// and c; without it the texts channel carries them exactly as before.
    #[test]
    fn geometry_channel_carries_points_as_numbers() {
        let path = Shape::Path(Path {
            segs: vec![
                PathSeg::Move(Point::new(1.0, 2.0)),
                PathSeg::Quad(Point::new(3.0, 4.0), Point::new(5.0, 6.0)),
                PathSeg::Close,
            ],
            rule: FillRule::EvenOdd,
        });
        let polygon = Shape::Polygon(vec![Point::new(0.0, 0.0), Point::new(7.0, 8.0)]);
        let ops = vec![
            DrawOp::Clip(path.clone()),
            DrawOp::Fill(path.clone(), Paint::Solid(Color::WHITE.into())),
            DrawOp::Stroke(
                polygon.clone(),
                Paint::Solid(Color::WHITE.into()),
                StrokeStyle::default(),
            ),
        ];
        let (nums, texts, geometry) = encode_ops_with_geometry(&ops);
        assert_eq!(texts, vec!["", "", ""]);
        assert_eq!(
            geometry,
            vec![
                0.0, 1.0, 2.0, 2.0, 3.0, 4.0, 5.0, 6.0, 4.0, 0.0, 1.0, 2.0, 2.0, 3.0, 4.0, 5.0,
                6.0, 4.0, 0.0, 0.0, 7.0, 8.0
            ]
        );
        // The clip (kind 17, sub-kind 3 in f) and the fill (kind 15) point at the two copies
        // of the path, the stroke (kind 12) at the polygon's four numbers.
        assert_eq!((nums[0], nums[2], nums[3], nums[6]), (17.0, 0.0, 9.0, 3.0));
        assert_eq!(
            (nums[9], nums[11], nums[12], nums[15]),
            (15.0, 9.0, 9.0, 1.0)
        );
        assert_eq!((nums[18], nums[20], nums[21]), (12.0, 18.0, 4.0));
        let (plain_nums, plain_texts) = encode_ops(&ops);
        assert_eq!(plain_texts[1], "M 1 2 Q 3 4 5 6 Z");
        assert_eq!(plain_texts[2], "0,0 7,8");
        assert_eq!((plain_nums[11], plain_nums[12]), (0.0, 0.0));
    }

    /// A region outside the bitmap is clamped to it with its mapping into the rect kept, and an
    /// empty intersection draws nothing.
    #[test]
    fn image_source_clamps_to_the_bitmap() {
        let px = Size::new(64.0, 32.0);
        let dest = Rect::new(100.0, 200.0, 32.0, 32.0); // 2 points per source pixel
        assert_eq!(
            clamp_image_source(Rect::new(-8.0, -8.0, 16.0, 16.0), dest, px),
            Some((
                Rect::new(0.0, 0.0, 8.0, 8.0),
                Rect::new(116.0, 216.0, 16.0, 16.0)
            ))
        );
        assert_eq!(
            clamp_image_source(Rect::new(60.0, 28.0, 16.0, 16.0), dest, px),
            Some((
                Rect::new(60.0, 28.0, 4.0, 4.0),
                Rect::new(100.0, 200.0, 8.0, 8.0)
            ))
        );
        assert_eq!(
            clamp_image_source(Rect::new(16.0, 8.0, 16.0, 16.0), dest, px),
            Some((Rect::new(16.0, 8.0, 16.0, 16.0), dest))
        );
        assert_eq!(
            clamp_image_source(Rect::new(64.0, 0.0, 8.0, 8.0), dest, px),
            None
        );
        assert_eq!(
            clamp_image_source(Rect::new(0.0, 0.0, 0.0, 8.0), dest, px),
            None
        );
        assert_eq!(
            clamp_image_source(Rect::new(0.0, 0.0, 8.0, 8.0), dest, Size::new(0.0, 0.0)),
            None
        );
    }

    /// The wire codes are a frozen dense table: any gap, duplicate, or variant missing from
    /// `ALL` breaks a decoder somewhere, so it must break here first.
    #[test]
    fn op_codes_are_dense_and_round_trip() {
        for (i, code) in OpCode::ALL.into_iter().enumerate() {
            assert_eq!(code as i32, i as i32, "{code:?} broke wire density");
            assert_eq!(OpCode::from_wire(i as f64), Some(code));
        }
        assert_eq!(OpCode::from_wire(OpCode::ALL.len() as f64), None);
        assert_eq!(OpCode::from_wire(-1.0), None);
        assert_eq!(OpCode::from_wire(0.5), None);
    }

    /// Reference decode of a display list covering every op arm: the record stream must be
    /// whole 9-slot records of known codes, and the texts channel must carry exactly one
    /// entry per text-carrying record, in order: the contract every shim decoder assumes.
    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "wire slots hold integer codes and exact test constants"
    )]
    fn encode_ops_emits_whole_records_and_matched_text_payloads() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        let red = Color::rgb(1.0, 0.0, 0.0);
        let ops = vec![
            DrawOp::Fill(Shape::Rect(r), Paint::Solid(red.into())),
            DrawOp::Stroke(
                Shape::Rect(r),
                Paint::Solid(red.into()),
                StrokeStyle::round(2.0),
            ),
            DrawOp::Fill(Shape::RoundedRect(r, 5.0), Paint::Solid(red.into())),
            DrawOp::Fill(Shape::Ellipse(r), Paint::Solid(red.into())),
            // The documented asymmetry: a filled arc encodes as StrokeArc with width 0.
            DrawOp::Fill(
                Shape::Arc {
                    rect: r,
                    start: Angle::ZERO,
                    sweep: Angle::QUARTER,
                },
                Paint::Solid(red.into()),
            ),
            DrawOp::Stroke(
                Shape::Line(Point::new(0.0, 0.0), Point::new(9.0, 9.0)),
                Paint::Solid(red.into()),
                StrokeStyle::width(1.0),
            ),
            DrawOp::Fill(
                Shape::Polygon(vec![
                    Point::new(0.0, 0.0),
                    Point::new(4.0, 0.0),
                    Point::new(2.0, 3.0),
                ]),
                Paint::Solid(red.into()),
            ),
            DrawOp::Fill(
                Shape::Path(Path {
                    segs: vec![
                        PathSeg::Move(Point::new(0.0, 0.0)),
                        PathSeg::Line(Point::new(5.0, 5.0)),
                        PathSeg::Close,
                    ],
                    rule: FillRule::EvenOdd,
                }),
                Paint::Linear(LinearGradient::new(
                    UnitPoint::new(0.0, 0.0),
                    UnitPoint::new(1.0, 1.0),
                    vec![(0.0, red), (1.0, Color::WHITE)],
                )),
            ),
            DrawOp::Clip(Shape::Rect(r)),
            DrawOp::Clip(Shape::Polygon(vec![
                Point::new(0.0, 0.0),
                Point::new(1.0, 0.0),
                Point::new(0.0, 1.0),
            ])),
            DrawOp::Text {
                text: "hi".into(),
                at: Point::new(7.0, 8.0),
                size: 12.0,
                color: red,
                anchor: TextAnchor::LEADING,
                font: CanvasFont::default(),
            },
            DrawOp::Text {
                text: "bold".into(),
                at: Point::new(7.0, 8.0),
                size: 12.0,
                color: red,
                anchor: TextAnchor::CENTERED,
                font: CanvasFont {
                    family: Some("Day Sans".into()),
                    weight: Some(FontWeight::Bold),
                    italic: true,
                },
            },
            DrawOp::Save,
            DrawOp::Concat(Affine::IDENTITY),
            DrawOp::Restore,
            DrawOp::Image {
                image: BitmapId(7),
                rect: r,
                opacity: 0.5,
                source: None,
            },
            DrawOp::Image {
                image: BitmapId(7),
                rect: r,
                opacity: 1.0,
                source: Some(Rect::new(16.0, 32.0, 8.0, 8.0)),
            },
        ];
        let (nums, texts) = encode_ops(&ops);
        assert_eq!(nums.len() % 9, 0, "partial record on the wire");

        let mut expected_texts = 0usize;
        let mut decoded = Vec::new();
        for rec in nums.chunks(9) {
            let code = OpCode::from_wire(rec[0])
                .unwrap_or_else(|| panic!("unknown op code {} on the wire", rec[0]));
            decoded.push(code);
            expected_texts += match code {
                OpCode::Text
                | OpCode::FillPolygon
                | OpCode::StrokePolygon
                | OpCode::SetGradient
                | OpCode::FillPath
                | OpCode::StrokePath
                | OpCode::StrokeStyle
                | OpCode::SetFont => 1,
                // Clip carries a payload only for its path (3) / polygon (4) sub-kinds.
                OpCode::Clip => usize::from(rec[6] == 3.0 || rec[6] == 4.0),
                _ => 0,
            };
        }
        assert_eq!(
            texts.len(),
            expected_texts,
            "texts channel out of step with the records that consume it"
        );
        // No payload may carry the transport separator.
        assert!(texts.iter().all(|t| !t.contains('\u{1f}')));

        let expected = {
            use OpCode::*;
            vec![
                FillRect,
                StrokeStyle, // ::round(2.0) is not plain, so the style modifier precedes
                StrokeRect,
                FillRrect,
                FillEllipse,
                StrokeArc, // the filled arc's stroke encoding…
                Line,
                FillPolygon,
                SetGradient, // gradient precedes its shape record
                FillPath,
                Clip,
                Clip,
                Text,
                SetFont, // a non-default font precedes its text record…
                Text,
                Save,
                Concat,
                Restore,
                Image,
                ImageSource, // a region precedes its image record; a whole-image draw has none
                Image,
            ]
        };
        assert_eq!(decoded, expected);
        // The bitmap id rides the numeric channel intact (an f64 holds a u64 id exactly well
        // past any id this process will mint), and nothing of an image reaches the texts channel.
        let img = nums
            .chunks(9)
            .find(|r| r[0] == f64::from(OpCode::Image as i32))
            .expect("Image record");
        assert_eq!(&img[1..7], &[1.0, 2.0, 3.0, 4.0, 7.0, 0.5]);
        // The region rides its own prefix record, in bitmap pixels, with nothing else in it.
        let src = nums
            .chunks(9)
            .find(|r| r[0] == f64::from(OpCode::ImageSource as i32))
            .expect("ImageSource record");
        assert_eq!(&src[1..8], &[16.0, 32.0, 8.0, 8.0, 0.0, 0.0, 0.0]);
        // …carrying the CSS weight and the slant, with the family on the texts channel.
        let font_rec = nums
            .chunks(9)
            .find(|r| r[0] == f64::from(OpCode::SetFont as i32))
            .expect("SetFont record");
        assert_eq!(&font_rec[1..3], &[700.0, 1.0]);
        assert_eq!(texts.iter().filter(|t| *t == "Day Sans").count(), 1);
        // …with width 0, per the documented asymmetry.
        let arc = nums
            .chunks(9)
            .find(|r| r[0] == f64::from(OpCode::StrokeArc as i32));
        assert_eq!(arc.map(|r| r[7]), Some(0.0));
        // A gradient-filled shape's color slot is opaque white, never clear (Skia decoders
        // modulate the shader by it).
        let path_rec = nums
            .chunks(9)
            .find(|r| r[0] == f64::from(OpCode::FillPath as i32));
        assert_eq!(
            path_rec.map(|r| r[8] as u32),
            Some(0xFFFF_FFFF),
            "gradient shape record must carry opaque white"
        );
    }

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "wire slots hold integer codes and exact test constants"
    )]
    fn variable_length_geometry_carries_a_content_key() {
        let path = |x: f64| {
            Shape::Path(Path {
                segs: vec![
                    PathSeg::Move(Point::new(x, 0.0)),
                    PathSeg::Cubic(
                        Point::new(1.0, 1.0),
                        Point::new(2.0, 2.0),
                        Point::new(3.0, 3.0),
                    ),
                    PathSeg::Close,
                ],
                rule: FillRule::NonZero,
            })
        };
        let poly = Shape::Polygon(vec![
            Point::new(0.0, 0.0),
            Point::new(1.0, 0.0),
            Point::new(0.0, 1.0),
        ]);

        // Equal geometry keys equal though the two were built separately. The key exists for
        // this: a draw closure rebuilds its paths from scratch on every frame.
        assert_eq!(geometry_key(&path(0.0)), geometry_key(&path(0.0)));
        assert_ne!(geometry_key(&path(0.0)), geometry_key(&path(1.0)));
        // The fill rule decides what the path draws, so it belongs to the key.
        let even_odd = Shape::Path(Path {
            segs: match path(0.0) {
                Shape::Path(p) => p.segs,
                _ => unreachable!(),
            },
            rule: FillRule::EvenOdd,
        });
        assert_ne!(geometry_key(&path(0.0)), geometry_key(&even_odd));
        // A polygon and a path decode to different geometry and never share an entry.
        assert_ne!(geometry_key(&poly), geometry_key(&path(0.0)));
        // Fixed-size shapes keep 0: their geometry rides the record's own slots.
        assert_eq!(
            geometry_key(&Shape::Rect(Rect::new(0.0, 0.0, 1.0, 1.0))),
            0.0
        );
        // And a key survives an f64 wire slot exactly, which is what the 53-bit mask buys.
        let k = geometry_key(&path(0.0));
        assert!(
            k > 0.0 && k < (1u64 << 53) as f64 && k.fract() == 0.0,
            "{k}"
        );

        // It reaches the wire in slot a, for exactly the ops whose payload rides the texts
        // channel, and never displaces geometry that is already there.
        let (nums, _) = encode_ops(&[
            DrawOp::Fill(path(0.0), Paint::Solid(Color::WHITE.into())),
            DrawOp::Fill(poly.clone(), Paint::Solid(Color::WHITE.into())),
            DrawOp::Clip(path(0.0)),
            DrawOp::Clip(Shape::Rect(Rect::new(1.0, 2.0, 3.0, 4.0))),
        ]);
        let slot_a = |code: OpCode| {
            nums.chunks(9)
                .find(|r| r[0] == f64::from(code as i32))
                .unwrap_or_else(|| panic!("no {code:?} record"))[1]
        };
        assert_eq!(slot_a(OpCode::FillPath), geometry_key(&path(0.0)));
        assert_eq!(slot_a(OpCode::FillPolygon), geometry_key(&poly));
        let clips: Vec<&[f64]> = nums
            .chunks(9)
            .filter(|r| r[0] == f64::from(OpCode::Clip as i32))
            .collect();
        assert_eq!(clips[0][1], geometry_key(&path(0.0)), "path clip keys");
        assert_eq!(
            &clips[1][1..5],
            &[1.0, 2.0, 3.0, 4.0],
            "a rect clip still puts its geometry in a..d"
        );
    }
}

#[cfg(test)]
mod markup_tests {
    use super::*;

    fn bold(range: std::ops::Range<usize>) -> TextRun {
        TextRun::font(
            range,
            FontSpec {
                style: Font::Body,
                weight: Some(FontWeight::Bold),
                ..Default::default()
            },
        )
    }

    #[test]
    fn plain_text_is_escaped_in_both_dialects() {
        // The case a translated string will find: markup metacharacters in the content.
        let text = "5 < 6 & \"quoted\" <b>not bold</b>";
        for d in [MarkupDialect::Pango, MarkupDialect::QtHtml] {
            let m = runs_to_markup(text, &[], d, 16.0);
            assert!(!m.contains("<b>"), "content tag survived into markup: {m}");
            assert!(m.contains("&lt;b&gt;"), "{m}");
            assert!(m.contains("&amp;"), "{m}");
            assert!(m.contains("5 &lt; 6"), "{m}");
        }
    }

    #[test]
    fn a_styled_run_is_wrapped_and_its_text_escaped() {
        let text = "a <b> c";
        let m = runs_to_markup(text, &[bold(2..5)], MarkupDialect::Pango, 16.0);
        assert_eq!(m, "a <b>&lt;b&gt;</b> c");
    }

    #[test]
    fn color_and_monospace_differ_per_dialect() {
        let text = "code";
        let run = TextRun {
            range: 0..4,
            font: FontSpec {
                style: Font::Body,
                monospace: true,
                ..Default::default()
            },
            color: Some(Color::rgb(1.0, 0.0, 0.0).into()),
            ..TextRun::default()
        };
        let pango = runs_to_markup(text, std::slice::from_ref(&run), MarkupDialect::Pango, 16.0);
        assert!(pango.contains("foreground=\"#ff0000\""), "{pango}");
        assert!(pango.contains("font_family=\"monospace\""), "{pango}");
        let qt = runs_to_markup(
            text,
            std::slice::from_ref(&run),
            MarkupDialect::QtHtml,
            16.0,
        );
        assert!(qt.contains("color:#ff0000"), "{qt}");
        // Qt takes its fixed face from <code>: a `font-family:monospace` style attribute
        // renders proportional (observed on Qt 6.11).
        assert!(qt.contains("<code>") && qt.contains("</code>"), "{qt}");
    }

    #[test]
    fn the_new_attributes_reach_both_dialects() {
        let run = TextRun {
            range: 0..2,
            font: FontSpec::default().scaled(1.5),
            background: Some(Color::rgb(0.0, 1.0, 0.0).into()),
            underline: Some(Decoration::SINGLE),
            ..TextRun::default()
        };
        let pango = runs_to_markup("hi", std::slice::from_ref(&run), MarkupDialect::Pango, 16.0);
        assert!(pango.contains("background=\"#00ff00\""), "{pango}");
        // Pango takes an absolute size in 1/1024 pt (16 pt base × 1.5 = 24 pt), because the
        // relative attributes are Pango 1.50's and an unknown attribute empties the whole label.
        assert!(pango.contains("size=\"24576\""), "{pango}");
        assert!(pango.contains("<u>hi</u>"), "{pango}");
        let qt = runs_to_markup(
            "hi",
            std::slice::from_ref(&run),
            MarkupDialect::QtHtml,
            16.0,
        );
        assert!(qt.contains("background-color:#00ff00"), "{qt}");
        // Qt takes points: 16 pt base × 1.5 (observed; its CSS subset ignores a percentage).
        assert!(qt.contains("font-size:24pt"), "{qt}");
        assert!(qt.contains("<u>hi</u>"), "{qt}");
    }

    #[test]
    fn a_link_target_is_escaped_too() {
        // A URL carrying `&` between query parameters is ordinary, and unescaped it truncates
        // the attribute and swallows the rest of the label.
        let text = "docs";
        let run = TextRun {
            range: 0..4,
            link: Some("https://x.dev/?a=1&b=2".into()),
            ..TextRun::default()
        };
        let m = runs_to_markup(text, &[run], MarkupDialect::Pango, 16.0);
        assert!(m.contains("a=1&amp;b=2"), "{m}");
    }

    #[test]
    fn tags_nest_and_close_in_reverse() {
        let run = TextRun {
            range: 0..2,
            font: FontSpec {
                style: Font::Body,
                weight: Some(FontWeight::Bold),
                italic: true,
                ..Default::default()
            },
            strikethrough: Some(Decoration::SINGLE),
            link: Some("u".into()),
            ..TextRun::default()
        };
        let m = runs_to_markup("hi", &[run], MarkupDialect::Pango, 16.0);
        assert_eq!(m, "<a href=\"u\"><b><i><s>hi</s></i></b></a>");
    }

    #[test]
    fn runs_out_of_bounds_are_skipped_not_panicked() {
        // `runs_are_valid` rejects these upstream; this is the belt-and-braces path.
        let m = runs_to_markup("hi", &[bold(0..99)], MarkupDialect::Pango, 16.0);
        assert_eq!(m, "hi");
    }

    #[test]
    fn validation_catches_the_four_ways_runs_go_wrong() {
        let text = "héllo";
        assert!(runs_are_valid(text, &[bold(0..1)]).is_ok());
        // Overlapping.
        assert!(runs_are_valid(text, &[bold(0..3), bold(2..4)]).is_err());
        // Past the end.
        assert!(runs_are_valid(text, &[bold(0..99)]).is_err());
        // Inverted. Built from parts, since a literal `3..1` is a clippy error in its own right.
        let (hi, lo) = (3, 1);
        assert!(runs_are_valid(text, &[bold(hi..lo)]).is_err());
        // Splitting the 2-byte `é`.
        assert!(runs_are_valid(text, &[bold(0..2)]).is_err());
    }
}

#[cfg(test)]
mod route_of_url_tests {
    use super::route_of_url;

    #[test]
    fn strips_the_scheme_and_keeps_route_and_params() {
        assert_eq!(
            route_of_url("notes://mail/inbox?hint=x"),
            "mail/inbox?hint=x"
        );
        assert_eq!(route_of_url("notes://home"), "home");
        // No scheme: already a route, unchanged.
        assert_eq!(route_of_url("mail/inbox"), "mail/inbox");
        assert_eq!(route_of_url(""), "");
        // Percent-encoding passes through; the route parser decodes, not this mapping.
        assert_eq!(route_of_url("n://a/b%2Fc?q=1%202"), "a/b%2Fc?q=1%202");
    }
}

#[cfg(test)]
mod builtin_kind_tests {
    use super::*;

    /// Every built-in round-trips through its wire key, and the keys are unique. Guards the
    /// `builtin_kinds!` expansion: a copy-paste slip in the table would alias two kinds.
    #[test]
    fn keys_round_trip_and_are_unique() {
        let mut seen = std::collections::HashSet::new();
        for &b in Builtin::ALL {
            assert_eq!(Builtin::from_key(b.key()), Some(b), "{b:?} key round-trip");
            assert!(seen.insert(b.key()), "duplicate key {:?}", b.key());
            assert!(
                b.key().starts_with("day."),
                "built-in {b:?} must use the reserved `day.` prefix"
            );
        }
    }

    /// The `kinds::*` constants are the enum's keys, so the two spellings cannot drift.
    #[test]
    fn kinds_constants_match_the_enum() {
        assert_eq!(kinds::LABEL, Builtin::Label.key());
        assert_eq!(kinds::LIST_CELL, Builtin::ListCell.key());
        assert_eq!(kinds::COVER, Builtin::Cover.key());
        assert_eq!(kinds::INSPECTOR, Builtin::Inspector.key());
        assert_eq!(kinds::TREE, Builtin::Tree.key());
        // 19 after `Tabs`/`TabsPage` retired (the tab bar is a NAV presentation, not a kind),
        // +2 for the inspector split and its panes (docs/inspector.md), +1 for the tree
        // (docs/tree.md).
        assert_eq!(Builtin::ALL.len(), 25);
    }

    /// An extension piece's kind is not a built-in.
    #[test]
    fn extension_kinds_are_not_builtin() {
        assert_eq!(Builtin::from_key("acme.combobox"), None);
        assert_eq!(Builtin::from_key("day.not_a_real_kind"), None);
    }
}

#[cfg(test)]
mod locale_tests {
    /// POSIX locale strings are not BCP-47 tags, and "C" is not a language; that difference is
    /// the job of `posix_locale_hints` (§12.2).
    #[test]
    fn posix_hints_parse_and_drop_the_c_locale() {
        // The parsing is exercised through the same normalization the function applies.
        let normalize = |v: &str| v.split('.').next().unwrap_or(v).replace('_', "-");
        assert_eq!(normalize("fr_FR.UTF-8"), "fr-FR");
        assert_eq!(normalize("en_US"), "en-US");
        assert_eq!(normalize("C"), "C");
        // A real read never panics whatever this host's environment holds.
        let hints = super::posix_locale_hints();
        assert!(
            hints
                .iter()
                .all(|h| !h.is_empty() && h != "C" && h != "POSIX")
        );
    }
}

#[cfg(test)]
mod sidetable_tests {
    use super::sidetable::{SideTable, sweep};
    use std::cell::Cell;
    use std::rc::Rc;

    /// The registry exists so that one `sweep(key)` clears every table without any
    /// table needing its own line in a backend's `release()`.
    #[test]
    fn sweep_clears_every_registered_table_and_runs_teardowns() {
        let torn = Rc::new(Cell::new(0u32));
        let t2 = torn.clone();
        let plain: SideTable<&'static str> = SideTable::new();
        let hooked: SideTable<u32> = SideTable::with_teardown(move |_| t2.set(t2.get() + 1));

        plain.insert(7, "a");
        hooked.insert(7, 1);
        hooked.insert(8, 2);
        sweep(7);
        assert!(!plain.contains(7));
        assert!(!hooked.contains(7), "swept from every table");
        assert!(hooked.contains(8), "other keys untouched");
        assert_eq!(torn.get(), 1, "teardown ran for the swept value");

        // remove() runs the teardown too; take() bypasses it for ownership transfer.
        let _ = hooked.remove(8);
        assert_eq!(torn.get(), 2);
        hooked.insert(9, 3);
        assert_eq!(hooked.take(9), Some(3));
        assert_eq!(torn.get(), 2, "take() skips the hook");
    }

    #[test]
    fn contain_returns_default_on_panic() {
        let ok = super::ffi_guard::contain(0i32, || 5);
        assert_eq!(ok, 5);
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let contained = super::ffi_guard::contain(-1i32, || panic!("boom"));
        std::panic::set_hook(prev);
        assert_eq!(contained, -1, "the panic must not escape the guard");
    }
}

#[cfg(test)]
mod cursor_tests {
    use super::Cursor;

    /// Every named cursor has a distinct CSS keyword, and `NAMED` lists each exactly once: the
    /// gallery and every backend's table key on the keyword.
    #[test]
    fn named_cursors_have_distinct_css_names() {
        let mut seen = std::collections::BTreeSet::new();
        for c in &Cursor::NAMED {
            assert!(
                seen.insert(c.css_name().to_string()),
                "duplicate: {}",
                c.css_name()
            );
            assert!(!matches!(c, Cursor::Native(_)));
        }
        assert_eq!(seen.len(), Cursor::NAMED.len());
        assert_eq!(Cursor::native("dragCopy").css_name(), "dragCopy");
        assert_eq!(Cursor::default(), Cursor::Default);
    }
}

#[cfg(test)]
mod text_anchor_tests {
    /// The anchor arithmetic every backend repeats. A drawing whose text lands in the wrong
    /// corner is a per-backend bug that only a screenshot catches, so the shared definition is
    /// pinned here and each shim is written to match it.
    #[test]
    fn text_anchor_offsets_place_the_line_box() {
        use crate::props::TextAlign;
        use crate::{TextAnchor, TextVAlign};
        // A 40-wide, 20-tall line box whose baseline sits 16 below its top.
        let (w, h, asc) = (40.0, 20.0, 16.0);
        let at = |ha, va| TextAnchor { h: ha, v: va }.offset(w, h, asc);

        // The default is the drawing origin itself, every backend's native "draw here".
        assert_eq!(at(TextAlign::Leading, TextVAlign::Top), (0.0, 0.0));
        assert_eq!(TextAnchor::default(), TextAnchor::LEADING);
        // Horizontal: the box's leading edge, its middle, its trailing edge.
        assert_eq!(at(TextAlign::Center, TextVAlign::Top).0, -20.0);
        assert_eq!(at(TextAlign::Trailing, TextVAlign::Top).0, -40.0);
        // Vertical: top, middle, the baseline itself, bottom.
        assert_eq!(at(TextAlign::Leading, TextVAlign::Middle).1, -10.0);
        assert_eq!(at(TextAlign::Leading, TextVAlign::Baseline).1, -16.0);
        assert_eq!(at(TextAlign::Leading, TextVAlign::Bottom).1, -20.0);
        // The two named placements are the ones the two-variant enum used to have.
        assert_eq!(TextAnchor::CENTERED.offset(w, h, asc), (-20.0, -10.0));
        assert_eq!(TextAnchor::TRAILING.offset(w, h, asc), (-40.0, 0.0));
    }

    /// The wire code the serializing backends (qt, xaml, arkui, android, web) decode by hand.
    #[test]
    fn text_anchor_packs_into_one_slot_and_round_trips() {
        use crate::props::TextAlign;
        use crate::{TextAnchor, TextVAlign};
        let mut seen = std::collections::BTreeSet::new();
        for h in [TextAlign::Leading, TextAlign::Center, TextAlign::Trailing] {
            for v in [
                TextVAlign::Top,
                TextVAlign::Middle,
                TextVAlign::Baseline,
                TextVAlign::Bottom,
            ] {
                let a = TextAnchor { h, v };
                let code = a.pack();
                assert!(
                    (0.0..=11.0).contains(&code),
                    "{code} outside the packed range"
                );
                assert!(seen.insert(code as u64), "two anchors share code {code}");
                assert_eq!(TextAnchor::unpack(code), a);
            }
        }
        assert_eq!(seen.len(), 12);
        // The shims decode as `h = code / 4`, `v = code % 4` in integer arithmetic.
        assert_eq!(TextAnchor::LEADING.pack(), 0.0);
        assert_eq!(TextAnchor::CENTERED.pack(), 5.0);
        // Junk answers the default rather than failing inside a native draw callback.
        assert_eq!(TextAnchor::unpack(-3.0), TextAnchor::LEADING);
        assert_eq!(TextAnchor::unpack(99.0), TextAnchor::unpack(11.0));
    }

    /// The two shapes a backend's own metrics arrive in. Both land on the same contract: the line
    /// box's top-leading corner is the origin, and `ink` is relative to it.
    #[test]
    fn metrics_convert_from_a_backends_own_shape() {
        use crate::{Rect, TextMetrics};
        // Baseline-relative, y up-negative; Core Text, Skia and Qt all report like this.
        let m = TextMetrics::from_baseline(
            40.0,
            16.0,
            4.0,
            11.0,
            // Ink starting 1 right of the origin, from 11 above the baseline to 2 below it.
            Rect::new(1.0, -11.0, 38.0, 13.0),
        );
        assert_eq!(
            (m.width, m.height, m.ascent, m.cap_height),
            (40.0, 20.0, 16.0, 11.0)
        );
        // Shifted down by the ascent: the ink now starts 5 below the line box's top, which is
        // where a cap begins in a slot whose baseline is at 16.
        assert_eq!(m.ink, Rect::new(1.0, 5.0, 38.0, 13.0));
        assert_eq!(m.ascent - m.cap_height, m.ink.origin.y);

        // The eight-slot wire form is already line-box-relative and passes straight through.
        let w = TextMetrics::from_slots(&[40.0, 20.0, 16.0, 11.0, 1.0, 5.0, 38.0, 13.0]);
        assert_eq!(w, m);
    }

    /// The fallback must be usable, not merely present: a caller frames text with it on a toolkit
    /// that cannot measure, and every relation it relies on has to hold.
    #[test]
    fn the_approximation_is_internally_consistent() {
        use crate::TextMetrics;
        let m = TextMetrics::approximate("Agy", 20.0);
        assert!(m.ascent < m.height, "the baseline sits inside the line box");
        assert!(
            m.cap_height < m.ascent,
            "caps do not reach above the ascent"
        );
        // The ink box is the whole line box: a superset, so an overlap test using it can only be
        // too cautious. Never a claim to be tighter than it knows.
        assert_eq!(m.ink.origin.x, 0.0);
        assert_eq!(m.ink.origin.y, 0.0);
        assert_eq!(m.ink.size.width, m.width);
        assert_eq!(m.ink.size.height, m.height);
        // Empty text still answers a usable (zero-width, full-height) slot.
        let e = TextMetrics::approximate("", 20.0);
        assert_eq!(e.width, 0.0);
        assert!(e.height > 0.0);
    }
}

#[cfg(test)]
mod cap_list_tests {
    use super::Cap;

    #[test]
    fn every_capability_is_listed_once_in_its_place() {
        for (i, cap) in Cap::ALL.iter().enumerate() {
            assert_eq!(cap.ordinal(), i, "{cap:?} is out of place in Cap::ALL");
        }
    }
}

#[cfg(test)]
mod button_menu_tests {
    use super::*;
    use std::cell::Cell;
    use std::rc::Rc;

    fn action(label: &str) -> MenuItem {
        MenuItem::Action {
            id: None,
            action: 1,
            label: label.into(),
            shortcut: None,
            enabled: true,
            checked: None,
            role: None,
            icon: None,
        }
    }

    #[test]
    fn a_provider_that_releases_its_button_shows_nothing() {
        let registered = Rc::new(Cell::new(true));
        let r = registered.clone();
        // The provider's app code removes the button (here: unregisters it), then still answers.
        let provider: ContextMenuFn = Rc::new(move |_| {
            r.set(false);
            vec![action("Open")]
        });
        assert!(resolve_button_menu(&provider, || registered.get()).is_none());
    }

    #[test]
    fn a_live_button_shows_its_items_and_an_empty_menu_shows_nothing() {
        let provider: ContextMenuFn = Rc::new(|_| vec![action("Open")]);
        assert_eq!(
            resolve_button_menu(&provider, || true).map(|i| i.len()),
            Some(1)
        );
        let empty: ContextMenuFn = Rc::new(|_| Vec::new());
        let mut asked = false;
        assert!(
            resolve_button_menu(&empty, || {
                asked = true;
                true
            })
            .is_none()
        );
        assert!(!asked, "an empty menu needs no registry check");
    }
}

#[cfg(test)]
mod elide_tests {
    use super::{elide, props::Truncation};

    fn cells(s: &str) -> f64 {
        s.chars().count() as f64
    }

    #[test]
    fn text_that_fits_is_untouched() {
        for mode in [Truncation::Tail, Truncation::Head, Truncation::Middle] {
            assert_eq!(elide("short", mode, 5.0, cells), "short");
        }
    }

    #[test]
    fn each_mode_keeps_its_end_and_fills_the_width() {
        let text = "abcdefghijklmnopqrstuvwxyz";
        assert_eq!(elide(text, Truncation::Tail, 8.0, cells), "abcdefg\u{2026}");
        assert_eq!(elide(text, Truncation::Head, 8.0, cells), "\u{2026}tuvwxyz");
        assert_eq!(
            elide(text, Truncation::Middle, 8.0, cells),
            "abcd\u{2026}xyz"
        );
    }

    #[test]
    fn nothing_fits_leaves_the_ellipsis() {
        for mode in [Truncation::Tail, Truncation::Head, Truncation::Middle] {
            assert_eq!(elide("abcdef", mode, 0.5, cells), "\u{2026}");
        }
    }

    #[test]
    fn multibyte_text_is_cut_on_character_boundaries() {
        let text = "日本語のファイル名.txt";
        let got = elide(text, Truncation::Middle, 7.0, cells);
        assert_eq!(got.chars().count(), 7);
        assert!(got.starts_with("日本語") && got.ends_with("txt"), "{got}");
    }
}
