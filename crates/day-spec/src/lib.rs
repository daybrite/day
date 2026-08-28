// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-spec — the toolkit specification (DESIGN.md §8).
//!
//! Backends depend ONLY on this crate (never on day-core). One backend is linked per binary;
//! `day-core` is monomorphized over the concrete [`Toolkit`].

use std::any::Any;
use std::collections::HashMap;

pub use day_geometry::*;

/// Inline markdown → styled runs (docs/markdown.md). Lives here rather than in day-pieces
/// because it produces `TextRun`s, and every format codec beside it reads the same model.
pub mod markdown;
/// Bundled-resource random-access API + the per-backend opener seam (§18.3).
pub mod resource;
/// Styled text: the document model `label().runs(…)` renders and `day-piece-texteditor` edits,
/// with the Markdown / HTML / RTF codecs over it (docs/texteditor.md).
pub mod styled;
/// Import and export for [`StyledText`]: Markdown, HTML and RTF (docs/texteditor.md).
pub mod styled_codec;

pub use resource::{
    AssetDir, AssetName, FontFamily, ImageName, Resource, ResourceOpener, VectorName,
    resolve_asset_dir, resolve_image_file, resource, set_resource_opener,
};
pub use styled::{
    ListStyle, ParagraphAlign, ParagraphRun, ParagraphStyle, RunStyle, StyledText, Underline,
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
/// query included — `notes://mail/inbox?hint=x` ⇒ `mail/inbox?hint=x`. A string with no
/// scheme passes through unchanged (it is already a route). One definition, shared by every
/// platform's intake and the dayscript `deep_link` step, so a URL maps to the same route
/// bytes everywhere; percent-decoding stays with the route parser (docs/navigation.md).
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

/// Declare the built-in piece vocabulary ONCE: the [`Builtin`] enum, its string keys, and the
/// `kinds::*` constants are all generated from this list, so they can never drift apart.
///
/// Adding a variant here is deliberately a breaking change for backends: every `Toolkit` must
/// decide how to realize the new kind, and the exhaustive `match Builtin` in each backend's
/// `realize` turns that decision into a compile error rather than a runtime placeholder.
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
            /// This kind's wire key — the same string as the matching `kinds::*` constant.
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

        /// The built-in piece keys as plain strings, for the registry and the `PieceKind` seam.
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
    /// Native navigation item list (docs/navigation.md): NSOutlineView source list /
    /// GtkListBox navigation-sidebar / QListWidget / UITableView rows with chevrons.
    NavMenu = NAV_MENU => "day.nav_menu",
    /// Native recycling list (docs/list.md): NSTableView / UITableView / RecyclerView /
    /// GtkListView / QListView. Owns scrolling + cell reuse; Day binds row content on demand.
    List = LIST => "day.list",
    /// A recycled row's content anchor inside a `LIST`; Day adopts the native cell as its handle.
    /// ADOPTED, never realized — `Toolkit::adopt` wraps the native cell, so backends' `realize`
    /// never sees this kind.
    ListCell = LIST_CELL => "day.list_cell",
    /// Native hierarchical tree (docs/tree.md): NSOutlineView / sidebar collection list /
    /// GtkListView+TreeListModel / WinUI TreeView / ArkTS TreeView. Rows nest, disclose, and
    /// drag-to-reparent; Day binds row content on demand through `Toolkit::attach_tree`.
    /// Row cells are ADOPTED (the same `LIST_CELL` anchor path as `List`).
    Tree = TREE => "day.tree",
    /// A fullscreen cover (docs/cover.md): a modal surface presented over the whole window,
    /// edge-to-edge, driven by `CoverPatch::{Present,Dismiss}`. The handle is the cover's
    /// CONTENT container; its frame is native-owned while presented (the backend sizes it to
    /// the safe area and reports it via `Event::FrameChanged`).
    Cover = COVER => "day.cover",
    /// Content beside a TRAILING inspector pane (docs/inspector.md): the native split whose
    /// second pane is a show/hidable properties panel. Exactly two `INSPECTOR_PANE` children —
    /// the content, then the panel (`InspectorPaneProps::panel`). Only realized where
    /// `Cap::Inspector` answers `Native`; everywhere else the `inspector` piece composes the
    /// pane from plain containers and this kind never reaches the backend.
    Inspector = INSPECTOR => "day.inspector",
    /// One pane's container inside an `INSPECTOR` host — the NAV_PAGE of the inspector split.
    /// Its frame is native-owned (the splitter/dock sizes it), reported via
    /// `Event::FrameChanged` on this node; Day lays the pane's content out inside it.
    InspectorPane = INSPECTOR_PANE => "day.inspector_pane",
}

/// Whether a kind is worth asking [`Toolkit::first_baseline`] about (docs/baseline.md).
///
/// A BLOCKLIST, not an allowlist: the structural and graphic kinds below have no text of their
/// own, and everything else — including extension pieces like the date/time pickers, whose kinds
/// day-spec has never heard of — is assumed to be worth asking. An extension piece that turns
/// out to have no baseline simply answers `None`, which costs one query; an allowlist would
/// instead silently leave every extension piece centered, which is the bug this shape avoids.
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
/// failing — the right runtime behavior, but it means a missing renderer LOOKS like a rendered
/// app: the walkthrough passes, and the gallery's validator (which counts distinct colors) sees
/// nothing wrong. That is how a stale `skip_on:` survived in the showcase walkthrough after the
/// renderer it skipped had actually landed.
///
/// So every backend reports here instead of keeping its own warn-once set. The table is process-
/// wide and append-only, which gives dayscript's `assert_no_placeholders` something to assert on
/// (`crates/day-script`) — a placeholder becomes a test failure instead of a silent gap.
pub mod placeholder {
    use super::PieceKind;
    use std::collections::BTreeSet;
    use std::sync::{Mutex, OnceLock};

    fn table() -> &'static Mutex<BTreeSet<PieceKind>> {
        static TABLE: OnceLock<Mutex<BTreeSet<PieceKind>>> = OnceLock::new();
        TABLE.get_or_init(|| Mutex::new(BTreeSet::new()))
    }

    /// Record that `toolkit` had no renderer for `kind` and warn — once per kind, however many
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
    pub fn seen() -> Vec<PieceKind> {
        table()
            .lock()
            .map(|s| s.iter().copied().collect())
            .unwrap_or_default()
    }
}

/// Downcast a realize/update payload to the props/patch type a backend arm expects,
/// warning (once per kind, via [`placeholder::report`]) instead of panicking on a
/// mismatch. A mismatch means a piece registered under a builtin key or a day-core
/// regression — degrade to the placeholder path, never abort: most backend arms run
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

/// Pointer-keyed per-view side tables with an AUTOMATIC release sweep.
///
/// Backends keep auxiliary per-view state in thread-local maps keyed by the native
/// handle's address. Historically each map needed its own line in `release()` — a
/// checklist that drifted repeatedly (missed entries have produced both leaks and
/// stale-pointer CI segfaults, since a freed address gets recycled). A [`SideTable`]
/// registers itself with a per-thread sweeper list at construction, so a backend's
/// `release()` calls [`sweep`] ONCE and every table — present and future — drops its
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

    /// A pointer-keyed map swept automatically by [`sweep`]. Construct inside a
    /// `thread_local!` initializer; the optional teardown hook runs for each removed
    /// value (release a retained native object, dispose an owned child, …) — on sweep
    /// AND on plain [`SideTable::remove`], so teardown cannot be bypassed by accident.
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
                    // Remove under the borrow, tear down OUTSIDE it: hooks may re-enter
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
        /// exists — the value is consumed by it; use [`SideTable::take`] to skip it).
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

        /// Remove WITHOUT running the teardown hook (the caller takes ownership).
        pub fn take(&self, key: usize) -> Option<V> {
            self.map.borrow_mut().remove(&key)
        }

        pub fn contains(&self, key: usize) -> bool {
            self.map.borrow().contains_key(&key)
        }

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

    /// Remove `key` from EVERY table registered on this thread, running teardowns.
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

/// Containment for FFI entry points — native callbacks, JNI up-calls, posted-closure
/// trampolines. A Rust panic unwinding out of an `extern "C"` frame is undefined
/// behavior (in practice an abort), so every backend trampoline runs its body through
/// [`ffi_guard::contain`]: a caught panic is reported, the installed recovery hook
/// restores runtime coherence (day-core installs the reactive runtime's
/// `recover_from_panic` at boot), and the given default is returned.
pub mod ffi_guard {
    use std::sync::OnceLock;

    static RECOVERY: OnceLock<fn()> = OnceLock::new();

    /// Install the post-panic recovery hook (idempotent; first install wins). day-core
    /// registers `day_reactive::recover_from_panic` here so a contained panic cannot
    /// leave the reactive observer stack stranded.
    pub fn set_recovery(f: fn()) {
        let _ = RECOVERY.set(f);
    }

    /// Run `f`, containing any panic; on panic, report, run the recovery hook, and
    /// return `default`.
    pub fn contain<R>(default: R, f: impl FnOnce() -> R) -> R {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
            Ok(r) => r,
            Err(payload) => {
                let msg = payload
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| payload.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| "<non-string panic>".into());
                log::error!("panic contained at an FFI boundary: {msg}");
                if let Some(recover) = RECOVERY.get() {
                    recover();
                }
                default
            }
        }
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
    /// A form `section` card: the platform's grouped-content background — AppKit quaternary
    /// system fill, libadwaita `.card`, Qt `palette(alternate-base)`, UIKit tertiary system
    /// fill, Material surface-container, XAML card background brush.
    SectionCard,
}

pub const NAV_SIDEBAR_WIDTH: f64 = 240.0;

/// Default preferred width of a navigation host's content-list pane
/// (`props::NavProps::list_width`, docs/navigation.md) — Mail's message-list proportion.
pub const NAV_LIST_WIDTH: f64 = 300.0;

/// Reserved id for window-level events (resize, lifecycle): day-core routes it to the root.
pub const WINDOW_NODE: NodeId = NodeId(u64::MAX);

/// Which nodes handle the non-text keys (docs/menus.md). Keys follow FOCUS — the focused piece
/// hears them and nobody else — so a backend whose focused view would have to CLAIM a key from
/// the platform's own dispatch (AppKit's responder chain, the DOM's default action) has to know
/// whether the app wants it before swallowing it. An unclaimed arrow has to keep traveling, or
/// a canvas inside a scroll view would silently eat the keys that scroll it.
///
/// It lives HERE, at the seam, rather than in day-core: `day-dom` deliberately depends on
/// day-spec alone, and this is a fact about a node that both sides need.
pub mod keys {
    use super::NodeId;
    use std::cell::RefCell;
    use std::collections::HashSet;

    thread_local! {
        static HANDLED: RefCell<HashSet<NodeId>> = RefCell::new(HashSet::new());
    }

    /// Declare that `node` has a key handler — what `Decorate::on_key` records at build.
    pub fn mark(node: NodeId) {
        HANDLED.with(|h| h.borrow_mut().insert(node));
    }

    /// Whether `node` has one.
    pub fn handled(node: NodeId) -> bool {
        HANDLED.with(|h| h.borrow().contains(&node))
    }
}

/// Raw foreign native handle for polyglot adoption (§15.3).
pub type RawHandle = *mut std::ffi::c_void;

// ---------------------------------------------------------------------------
// Events (§8.3)
// ---------------------------------------------------------------------------

/// The wire table for backends whose native side reaches Rust through ONE numeric-kind
/// trampoline (Android's JNI `nativeOnEvent`, ArkUI's `day_arkui_on_event`). This enum is the
/// single source of truth for those kind numbers; the Java and C++ sides carry mirrored
/// constants that parity tests check against these discriminants (so a collision or drift
/// fails `cargo test` on the host instead of silently mis-decoding events on a device).
/// AppKit/UIKit/GTK/Qt emit `Event` values directly and never use these numbers; XAML uses
/// per-event callbacks with its own small local codes.
///
/// Payload conventions ride `(num: f64, text: String)` per kind — documented on each variant.
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
        /// Gesture; `num` = phase (0 tap, 1 began, 2 changed, 3 ended), `text` = `"x,y,tx,ty"` px.
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
        /// `num` = the settled value — the drag ended (see `Event::ValueCommitted`). A backend
        /// that cannot tell a settled value from a moving one sends only `ValueChanged`.
        ValueCommitted = 22,
        /// Inline search on a `.searchable()` navigation surface (docs/search.md): the field's
        /// new text, against the nav host's node.
        SearchChanged = 23,
        /// The toolkit's own adaptive nav container changed presentation and is reporting it
        /// (docs/size-classes.md). `num` = 1.0 for split, 0.0 for stacked.
        NavPresentation = 24,
        /// The system switched between light and dark appearance. No payload; node id ignored
        /// and no `Event` is emitted — Rust calls `day_core::note_appearance_changed`, the same
        /// thing AppKit's and GTK's own scheme observers do.
        AppearanceChanged = 25,
        /// A fullscreen cover's hide transition finished (docs/cover.md); no payload — the
        /// node id is the cover's. Decodes to [`crate::Event::CoverHidden`].
        CoverHidden = 26,
        /// A styled text run's link was tapped (docs/text-runs.md); `text` = the run's target.
        /// Decodes to [`crate::Event::LinkActivated`].
        LinkActivated = 27,
        /// The platform's own undo affordance fired (⌘Z, a three-finger swipe, the Edit menu
        /// through a native front); `num` != 0 ⇒ redo.
        UndoInvoked = 28,
        /// A non-text key reached the FOCUSED node (docs/menus.md); `text` = the day key name
        /// (`"ArrowLeft"`, …), `num` = the [`crate::KeyEvent`] modifier mask. Decodes to
        /// [`crate::Event::Key`].
        Key = 29,
    }

    impl BridgeKind {
        /// Every variant, for uniqueness/parity tests and exhaustive dispatch.
        pub const ALL: [BridgeKind; 30] = [
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
        /// nothing checks — which is how `SearchChanged` and `NavPresentation` sat outside the
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
    /// A continuous control's value SETTLED: the drag ended (mouse/touch up), or the value moved
    /// by a discrete action (a keyboard arrow, a click on the track). Always preceded by the
    /// `ValueChanged` carrying the same value.
    ///
    /// The pair exists because one gesture produces two different facts. `ValueChanged` is the
    /// live one — bindings follow it so the UI tracks the thumb — and it fires continuously, which
    /// is why nothing durable should key off it: a drag from 1 to 100 and back to 50 emits every
    /// value in between. `ValueCommitted` fires once, with the value the user actually chose, and
    /// is what a recording, a log, or a refetch should use.
    ///
    /// A backend that cannot tell the two apart emits only `ValueChanged`; its sliders then do not
    /// record (docs/coverage-matrix.md).
    ValueCommitted(f64),
    SelectionChanged(i64),
    /// The platform's own undo affordance fired through a native front (⌘Z on the Edit menu,
    /// a three-finger swipe, shake-to-undo) — `redo` distinguishes the pair. Only backends
    /// with a native undo system emit it; everywhere else the app's own controls call the
    /// stack directly and this event never exists (docs/model.md).
    Undo {
        redo: bool,
    },
    /// The platform's own standard edit command fired through its native route (Edit ▸
    /// Cut/Copy/Paste, ⌘X/⌘C/⌘V, the browser's `copy`/`paste` DOM events) with no text
    /// widget claiming it first — the app's edit bridge answers (docs/menus.md). Transport
    /// is the system clipboard (day-part-clipboard), not the event.
    Edit(EditOp),
    /// A multi-select list's selection changed: the FULL set of selected row indices,
    /// ascending (empty = nothing selected). Emitted instead of `SelectionChanged` where
    /// `ListProps::multi_select` is honored (docs/list.md).
    SelectionSet(Vec<i64>),
    FocusChanged(bool),
    Tap(Point),
    /// A link run inside a label was activated; the payload is [`TextRun::link`] verbatim.
    LinkActivated(String),
    LongPress(Point),
    /// A context-menu summon reported by a toolkit that presents no native menu of its own
    /// (web-dom): the pieces layer shows the COMPOSED menu (docs/menus.md). `local` is the
    /// point in the node's own coordinates (what the app's provider receives); `window` the
    /// same point in window coordinates (where the composed panel goes). Toolkits that serve
    /// context menus natively never emit this.
    ContextMenu {
        local: Point,
        window: Point,
    },
    /// A drag/pan gesture (docs/shapes.md). `location` is in the node's local coordinates;
    /// `translation` is the cumulative movement since `Began`.
    Drag {
        phase: DragPhase,
        location: Point,
        translation: Point,
    },
    /// A pinch/magnify gesture over the node (docs/shapes.md): `scale` is CUMULATIVE since
    /// `Began` (1.0 = unchanged), `location` the gesture's centroid in local coordinates —
    /// the anchor a zoom keeps stationary. Only nodes that enabled
    /// [`GestureKind::Pinch`] receive it; a backend with no recognizer emits nothing.
    Pinch {
        phase: DragPhase,
        scale: f64,
        location: Point,
    },
    /// A viewport pan over the node (docs/shapes.md): `delta` is the movement SINCE THE
    /// PREVIOUS event — incremental, not cumulative, because desktop wheels have no gesture
    /// session to accumulate over (a discrete wheel tick arrives as a lone `Changed`).
    /// Two-finger touch pans bracket theirs with `Began`/`Ended`. Distinct from
    /// [`Event::Drag`], the primary pointer's press-drag.
    Pan {
        phase: DragPhase,
        delta: Point,
        location: Point,
    },
    ScrollChanged(Point),
    /// A canvas node was re-framed by layout; re-record (§11). Nav pane/page containers
    /// also report their allocated size with this (docs/navigation.md).
    FrameChanged(Size),
    /// Native back navigation (iOS back button/swipe, Android system back or toolbar up).
    /// `already_popped` = the toolkit already performed the pop natively (iOS); the nav
    /// host then syncs its stack WITHOUT re-issuing `NavPatch::Popped`.
    NavBack {
        already_popped: bool,
    },
    /// A nav host's own adaptive container changed presentation, and Day is being told about it
    /// after the fact (docs/size-classes.md).
    ///
    /// The counterpart to [`props::NavPatch::Presentation`], for the toolkits that answer
    /// `Cap::NavRepresent = Emulated`: `UISplitViewController` collapsing as a Pro Max iPhone
    /// rotates into portrait, `SlidingPaneLayout` deciding at measure time that two panes no
    /// longer fit. Day did not ask for it and must not fight it — the pieces layer only
    /// reconciles what follows, which is the selection rule (a split presentation cannot draw an
    /// empty detail pane, so expanding with nothing selected picks the first item).
    ///
    /// Emitted on the NAV host node, and only when the presentation actually changed.
    NavPresentationChanged(props::NavPresentation),
    Key(KeyEvent),
    Pointer(PointerEvent),
    WindowResized(Size),
    /// The platform asks the app to show a route (docs/navigation.md): a runtime deep link —
    /// on web-dom, the URL hash changing under the app (hand-edited, or browser back/forward);
    /// on mobile, a warm scheme link (iOS `application:openURL:`, Android `onNewIntent` via
    /// `BridgeKind::Deeplink`). Emitted on [`WINDOW_NODE`] (day-core answers with `navigate()`
    /// when the route differs from the current one) or on the active NAV HOST's node (the nav
    /// piece answers with `navigate()` itself, docs/deep-links.md). Launch-time deep links use
    /// `DAY_DEEPLINK`/`set_launch_deeplink` instead — this variant is for changes while the
    /// app runs.
    RouteRequested(String),
    /// A native modal answered request `req` (docs/dialogs.md).
    PresentResult {
        req: u64,
        result: present::PresentResult,
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
    /// A toolbar item produced a value — a search field's text, a toggle's new state
    /// (docs/toolbars.md). `action` is the item's dispatch id, from the same registry
    /// [`Event::MenuAction`] uses, and day-core routes it to the closure registered for it.
    /// A plain toolbar button has no value and emits `MenuAction` instead.
    ToolbarChanged {
        action: u64,
        value: ToolbarValue,
    },
    /// A searchable navigation surface's field changed (docs/search.md). Emitted against the NAV
    /// HOST's node, not through the toolbar's dispatch registry: search belongs to the surface,
    /// so the field can move between the toolbar and the navigation list without the app
    /// re-registering anything.
    SearchChanged(String),
    /// The user picked a different scope. Carries the index into `SearchProps::scopes`.
    SearchScopeChanged(usize),
    /// The user chose one of `SearchProps::suggestions`, by index. The toolkit has ALREADY put
    /// that completion in the field and emitted [`Event::SearchChanged`] for it; this says which
    /// one, for an app that wants to act on the choice itself.
    SearchSuggestionChosen(usize),
    /// The app moved through a lifecycle phase (docs/lifecycle.md). Backends emit this from the
    /// native app/activity delegate; day-core routes it to the app's `on_lifecycle` handlers.
    Lifecycle(Lifecycle),
    /// A secondary native window closed — the title-bar close, a platform gesture (an
    /// app-switcher swipe, an activity finish), or `Toolkit::close_window` — emitted on the
    /// WINDOW'S ROOT node after the platform committed the close. day-core disposes the
    /// window's subtree on receipt, so native and programmatic closes share one teardown
    /// path (docs/windows.md). Never emitted for the primary window (its close terminates
    /// the app).
    WindowClosed,
    /// A secondary window's key/active state changed — emitted on the window's root node.
    /// day-core tracks the focused window with it (dialog parenting, dayscript targeting).
    WindowFocused(bool),
    /// A native list committed a drag-reorder (docs/list.md): row `from` landed at row `to`.
    /// Emitted on the LIST node by the list piece's own commit hook — the reorder driver
    /// rotates its snapshot synchronously inside the platform's drop callback, then reports
    /// through the event queue so the app's `on_reorder` never runs inside a native animation
    /// callstack.
    ListReorder {
        from: usize,
        to: usize,
    },
    /// A native list committed a swipe-delete (docs/list.md): the row at this index is gone.
    /// Deferred through the event queue exactly as [`Event::ListReorder`] is.
    ListDelete(usize),
    /// A native list activated a swipe action (docs/list.md): the user pressed button `action`
    /// (an index into the row's [`ListSwipe::actions_at`] offer for `edge`) on row `index`, or
    /// swiped clear across for the edge's first action. Emitted on the LIST node by the list
    /// piece's own commit hook and deferred through the event queue exactly as
    /// [`Event::ListReorder`] is, so the app's handler never runs inside a native animation
    /// callstack.
    ListSwipe {
        index: usize,
        edge: SwipeEdge,
        action: usize,
    },
    /// A fullscreen cover's hide transition finished (docs/cover.md): the content can now be
    /// disposed. Answers [`props::CoverPatch::Dismiss`]; delivery is a hard guarantee, and
    /// backends over-report rather than under-report — consumers gate on their own closing
    /// flag, so duplicates and belated reports are no-ops.
    CoverHidden,
    /// The user showed or hid an inspector pane through a NATIVE affordance — a dock widget's
    /// close button, a divider dragged shut — rather than through Day (docs/inspector.md).
    /// Emitted on the `kinds::INSPECTOR` node, and only when the visibility actually changed;
    /// the piece writes it back to the bound signal. Applying
    /// [`props::InspectorPatch::Visible`] must never re-emit it (the from-native echo rule).
    InspectorChanged(bool),
    /// The user disclosed or collapsed a tree row through the NATIVE affordance (docs/tree.md).
    /// Emitted on the `kinds::TREE` node; the piece writes it back to the app's expansion
    /// signal. Applying [`props::TreePatch::Expand`] must never re-emit it (the from-native
    /// echo rule).
    TreeExpanded {
        token: u64,
        expanded: bool,
    },
    /// A native tree committed a drag move (docs/tree.md): `token` now sits under `parent`
    /// (`None` = the root) at `index` (`None` = dropped ONTO the parent — append). Deferred
    /// through the event queue exactly as [`Event::ListReorder`] is; the app's `on_move`
    /// writes its own data, whose refresh reloads the tree.
    TreeMove {
        token: u64,
        parent: Option<u64>,
        index: Option<usize>,
    },
    /// A tree's selection changed: the FULL set of selected row TOKENS (empty = cleared).
    /// Token-addressed — a tree cannot speak row indices, because expanding a row renumbers
    /// everything below it (docs/tree.md).
    TreeSelection(Vec<u64>),
}

impl Event {
    /// Build a text-carrying [`Event::Custom`] (with `num` defaulted to 0) — the common case for an
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
/// activity delegate. Some phases only exist on some platforms — a mobile app truly enters the
/// background and can be low on memory, a desktop app essentially cannot — so [`Lifecycle::is_universal`]
/// marks the ones every backend delivers, and [`Toolkit::supports_lifecycle`] reports per-backend truth.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lifecycle {
    /// Before the window and UI are built — the first thing to run. Set up global state here.
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
    /// The app is about to terminate — the last chance to save. Triggered by the Quit command,
    /// the platform's quit shortcut, or the OS reclaiming the app.
    WillTerminate,
}

impl Lifecycle {
    /// Every phase, in delivery order (launch → run → quit). Handy for logging/registration sweeps.
    pub const ALL: [Lifecycle; 8] = [
        Lifecycle::WillLaunch,
        Lifecycle::DidLaunch,
        Lifecycle::DidBecomeActive,
        Lifecycle::WillResignActive,
        Lifecycle::WillEnterForeground,
        Lifecycle::DidEnterBackground,
        Lifecycle::DidReceiveMemoryWarning,
        Lifecycle::WillTerminate,
    ];

    /// True for phases EVERY backend delivers (launch, activation, termination). The remaining
    /// phases (`WillEnterForeground`, `DidEnterBackground`, `DidReceiveMemoryWarning`) are genuine
    /// mobile concepts and are only delivered by the mobile backends. `const` so it composes into a
    /// backend's `const fn lifecycle_supported` and thus into compile-time guards.
    pub const fn is_universal(self) -> bool {
        matches!(
            self,
            Lifecycle::WillLaunch
                | Lifecycle::DidLaunch
                | Lifecycle::DidBecomeActive
                | Lifecycle::WillResignActive
                | Lifecycle::WillTerminate
        )
    }

    /// A stable, human-readable name (for logs/warnings).
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

/// A gesture a node wants delivered. Backends attach the matching native recognizer when day-core
/// calls [`Toolkit::enable_gesture`]; the default is no gesture (recognizers cost, so opt-in).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GestureKind {
    Tap,
    LongPress,
    Drag,
    /// Pinch/magnify (docs/shapes.md): trackpad magnification, two-finger touch pinch.
    Pinch,
    /// Viewport pan (docs/shapes.md): trackpad two-finger scroll, two-finger touch pan —
    /// distinct from [`GestureKind::Drag`], the primary pointer's press-drag.
    Pan,
}

// ---------------------------------------------------------------------------
// Menus (app menu bar + context menus). The MODEL is a toolkit-neutral tree; each backend renders it
// with its OWN native affordance (NSMenu / GMenu+GtkPopoverMenu / QMenu / UIMenu / Android PopupMenu /
// XAML MenuFlyout) and its own conventions, so day imposes no menu manager of its own.
// ---------------------------------------------------------------------------

/// A keyboard shortcut for a menu item. `primary` is the platform's command modifier — ⌘ on Apple,
/// Ctrl on GTK/Qt/XAML — so one declaration reads correctly everywhere. `key` is a single character
/// (`"s"`, `"."`) or a named key (`"Return"`, `"Delete"`, `"Left"`, `"F1"`).
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Shortcut {
    pub key: String,
    /// ⌘ (Apple) / Ctrl (elsewhere). The conventional command modifier.
    pub primary: bool,
    pub shift: bool,
    /// ⌥ / Alt.
    pub alt: bool,
    /// Literal Control (⌃ on Apple). Rare — prefer `primary` for the command modifier.
    pub control: bool,
}

impl Shortcut {
    /// `primary`+`key` (⌘S / Ctrl+S) — the common case.
    pub fn new(key: impl Into<String>) -> Shortcut {
        Shortcut {
            key: key.into(),
            primary: true,
            ..Default::default()
        }
    }
    /// `key` with NO modifiers (e.g. `F1`, plain `Delete`).
    pub fn plain(key: impl Into<String>) -> Shortcut {
        Shortcut {
            key: key.into(),
            ..Default::default()
        }
    }
    pub fn shift(mut self) -> Shortcut {
        self.shift = true;
        self
    }
    pub fn alt(mut self) -> Shortcut {
        self.alt = true;
        self
    }
    pub fn control(mut self) -> Shortcut {
        self.control = true;
        self
    }
}

/// A standard/system command. The backend supplies the NATIVE item — selector on AppKit/UIKit
/// (`cut:`/`copy:`/`paste:`…), a stock action on GTK/Qt/XAML — so it targets the focused control,
/// gets the platform's default label + shortcut, and enables/disables itself automatically. This is
/// how default items (Edit ▸ Cut/Copy/Paste, the app's Quit/About) are accommodated without the app
/// re-implementing them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuRole {
    Cut,
    Copy,
    Paste,
    SelectAll,
    Undo,
    Redo,
    Delete,
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
}

/// The keyboard modifiers held at an interaction, queryable ambiently
/// ([`Toolkit::modifiers`]): `primary` is the platform's command key (⌘ on Apple platforms,
/// Ctrl elsewhere — the same convention [`Shortcut::primary`] uses). Touch-only backends
/// answer all-false.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub shift: bool,
    pub primary: bool,
    pub alt: bool,
}

/// A standard edit command a platform route delivered ([`Event::Edit`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditOp {
    Cut,
    Copy,
    Paste,
    SelectAll,
}

/// What the app's edit bridge can currently do — mirrored into the toolkit
/// ([`Toolkit::set_edit_state`]) so native menu validation enables the stock items.
/// `can_paste` is the APP's half ("a paste handler is installed"); the toolkit combines it
/// with its own clipboard-has-text check where one exists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EditState {
    pub can_cut: bool,
    pub can_copy: bool,
    pub can_paste: bool,
    pub can_select_all: bool,
}

/// The undo stack's face, as a native front mirrors it ([`Toolkit::set_undo_state`]): what is
/// possible and what the menu titles should say. Labels arrive ALREADY LOCALIZED — a toolkit
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

/// One entry in a menu (recursive — a `Submenu` nests).
#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    /// A command. `id` (nonzero) dispatches [`Event::MenuAction`] to the app; a `role`-only item uses
    /// the native standard command instead (id 0). `label`/`shortcut` override the role's defaults.
    /// `icon` is the platform's own glyph beside the title where menus carry one (macOS,
    /// Windows, GNOME, KDE, Android); a backend whose menus are text-only ignores it.
    Action {
        id: u64,
        label: String,
        shortcut: Option<Shortcut>,
        enabled: bool,
        role: Option<MenuRole>,
        icon: Option<Icon>,
    },
    /// A nested submenu. At the TOP level of an app menu a `role` claims one of the platform's
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
/// not claim with its own stock menu, so an app never has to restate the platform's furniture
/// — and can still replace any of it by tagging its own submenu with the matching role.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MenuBarRole {
    File,
    Edit,
    View,
    /// KDE/Plasma's Settings menu (Configure <App>, Configure Shortcuts). No counterpart on
    /// macOS, where preferences live in the app menu, or on GNOME/Windows.
    Settings,
    /// macOS's Window menu. Windows and Linux desktops do not have one.
    Window,
    Help,
}

// ---------------------------------------------------------------------------
// Window toolbars (docs/toolbars.md)
// ---------------------------------------------------------------------------

/// A standard icon, named by what it MEANS rather than by how it looks, so each backend can
/// draw the platform's own glyph for it — an SF Symbol on macOS, a freedesktop icon name on
/// GTK and Qt, a Fluent glyph on Windows. This is the only way an icon looks native on every
/// desktop at once; a bundled PNG cannot, because it is one artist's take on all four.
///
/// The set is deliberately small: the commands that recur across toolbars. Anything
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
    /// Show/hide the sidebar — the leading item of most desktop toolbars.
    Sidebar,
    Filter,
    Sort,
    /// An overflow affordance (macOS `ellipsis`, GNOME's hamburger, Fluent `More`).
    More,
    Play,
    Pause,
    Stop,
    /// A photo capture — a screenshot command, a camera control.
    Camera,
    /// Source code.
    Code,
    /// Light appearance (a sun). The three below are the vocabulary of a theme chooser, which
    /// every platform draws with the same three ideas.
    Light,
    /// Dark appearance (a moon).
    Dark,
    /// Follow the system — the half-filled circle every platform uses for "automatic".
    Auto,
    ZoomIn,
    ZoomOut,
    /// Back to actual size — the 100% zoom command between [`Symbol::ZoomIn`] and
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
    /// A rectangle — the shape vocabulary a drawing surface needs (docs/shapes.md), and the
    /// glyphs every drawing, diagram and annotation tool puts in the same place.
    Rectangle,
    /// An ellipse.
    Oval,
    /// A straight line segment.
    Line,
    /// Combine the selection into a group — the drawing-tool command (two stacked squares).
    Group,
    /// Dissolve a group back into its members (two separated squares).
    Ungroup,
}

/// The SF Symbol each standard symbol draws as — the system's own glyphs, so they match the
/// user's device: weight, optical size, accent color and all. Shared by both Apple backends
/// (toolbars on macOS, menus on both).
///
/// Exhaustive on purpose: a new [`Symbol`] must name its glyph here rather than silently
/// drawing nothing on Apple. A name this OS release does not know still resolves to no image,
/// and the item falls back to its label.
pub fn sf_symbol_name(s: Symbol) -> &'static str {
    match s {
        Symbol::Add => "plus",
        Symbol::Remove => "minus",
        Symbol::Delete => "trash",
        Symbol::Edit => "pencil",
        Symbol::New => "square.and.pencil",
        Symbol::Open => "folder",
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
        Symbol::Folder => "folder",
        Symbol::Document => "doc",
        Symbol::Check => "checkmark",
        Symbol::Close => "xmark",
        Symbol::Warning => "exclamationmark.triangle",
        Symbol::Rectangle => "rectangle",
        Symbol::Oval => "oval",
        Symbol::Line => "line.diagonal",
        Symbol::Group => "square.on.square",
        Symbol::Ungroup => "square.on.square.dashed",
    }
}

/// A toolbar item's picture: a standard [`Symbol`] (drawn with the platform's own icon set) or
/// a bundled image from `resource/images` for something only this app has.
#[derive(Clone, Debug, PartialEq)]
pub enum Icon {
    Symbol(Symbol),
    /// A bundled image name, resolved the same way [`ImageName`] is elsewhere.
    Image(String),
}

/// What a toolbar item IS. The variants are the vocabulary every desktop toolbar shares; each
/// backend realizes one with its native control, never with a drawn imitation.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarItemKind {
    /// A push button running the item's `action`.
    Button,
    /// A two-state button. `on` seeds it; the user flipping it emits
    /// [`ToolbarValue::On`], and the app's own writes arrive as [`ToolbarPatch::On`].
    Toggle { on: bool },
    /// Show/hide this window's sidebar. The item carries NO `action`: the toolkit binds it to
    /// whatever `selector(Sidebar)` host the window contains and drives that host's own
    /// collapse, so the app declares the affordance and the platform supplies the behavior —
    /// `NSToolbarToggleSidebarItemIdentifier` on AppKit, the split view's collapse on GTK,
    /// `NavigationView.IsPaneOpen` on XAML. Place it where the platform expects it (leading,
    /// before the first [`ToolbarItemKind::FlexibleSpace`]); a window with no sidebar renders
    /// it disabled rather than dropping it, so the toolbar's shape does not change with the
    /// route. docs/toolbars.md.
    SidebarToggle,
    /// A button that drops a menu — the same [`MenuItem`] model the menu bar uses, so a
    /// toolbar menu and its menu-bar twin are one list of commands.
    Menu { items: Vec<MenuItem> },
    /// A search field. Edits emit [`ToolbarValue::Text`]; the app's own writes arrive as
    /// [`ToolbarPatch::Text`].
    ///
    /// Day creates this item itself, from a `.searchable()` surface whose placement resolved to
    /// the toolbar (docs/search.md) — apps declare search on the surface, never here.
    Search {
        text: String,
        placeholder: String,
        /// Completions for the current text, drawn by the field's own completion affordance
        /// where it has one (`AutoSuggestBox`, `QCompleter`, `<datalist>`). Empty = none.
        suggestions: Vec<String>,
    },
    /// A row of mutually exclusive choices drawn as ONE control — the native segmented control
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
    /// A divider, where the platform draws one (macOS toolbars have no separator, so AppKit
    /// renders it as a fixed space — docs/toolbars.md).
    Separator,
    /// A fixed gap.
    Space,
    /// A gap that absorbs the leftover width. This is how the model expresses each platform's
    /// packing: items before the first flexible space are leading, items after it trailing —
    /// GTK packs them start/end, XAML splits them across `Content`/`PrimaryCommands`, and
    /// AppKit and Qt place a real expanding spacer.
    FlexibleSpace,
}

/// One choice inside a [`ToolbarItemKind::Segmented`] control.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarSegment {
    /// The segment's name — its label where the platform shows text, its accessible name always.
    pub title: String,
    pub icon: Option<Icon>,
}

/// One item in a window's toolbar (docs/toolbars.md).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolbarItem {
    /// Stable identity: the native item identifier, the dayscript target, and the key a
    /// [`ToolbarPatch`] addresses. Unique within a toolbar.
    pub id: String,
    pub kind: ToolbarItemKind,
    /// The item's name. Shown beside or below the icon where the platform does that, and used
    /// verbatim in the overflow and customization menus — so it is never optional, even on a
    /// toolbar that only shows icons. It is also the item's accessible name.
    pub label: String,
    /// Hover help. Defaults to `label` where the platform expects a tooltip and none is given.
    pub tooltip: Option<String>,
    pub icon: Option<Icon>,
    pub enabled: bool,
    /// The item's command, as a dispatch id from the same registry [`Event::MenuAction`] uses
    /// (0 = no command), so a toolbar button and its menu twin can share one closure.
    pub action: u64,
}

/// A value a toolbar item produced.
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarValue {
    /// A search item's full new text.
    Text(String),
    /// A toggle item's new state.
    On(bool),
    /// A segmented item's newly chosen index.
    Selected(usize),
}

/// A targeted update to one live toolbar item — the path that keeps a bound signal in sync
/// without rebuilding the bar (which would drop the search field's focus mid-keystroke).
#[derive(Clone, Debug, PartialEq)]
pub enum ToolbarPatch {
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
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    /// The key's name, in the web `KeyboardEvent.key` vocabulary every platform can map onto:
    /// the four arrows ("ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown") everywhere, plus
    /// "Delete" and "Backspace" on backends with no menu bar ([`Cap::AppMenu`] unsupported),
    /// where no accelerator can own those keys — a focused piece claims every key the route
    /// carries, so offering them on a menu-bar platform would let a canvas swallow the Delete
    /// its own Edit menu was about to act on (docs/menus.md). The two delete keys keep the
    /// names the platform gives the PHYSICAL keys — a Mac's ⌫ is "Backspace" and its ⌦ is
    /// "Delete" — so a handler that means "remove this" takes both. Backends emit `Event::Key`
    /// only while no text widget has focus; a field's own editing keys never surface here.
    pub key: String,
    /// A [`KeyEvent::SHIFT`]/[`KeyEvent::PRIMARY`]/[`KeyEvent::ALT`] mask.
    pub modifiers: u8,
}

impl KeyEvent {
    pub const SHIFT: u8 = 1;
    /// The platform's command key: ⌘ on Apple platforms, Ctrl elsewhere (the
    /// [`Shortcut::primary`] convention).
    pub const PRIMARY: u8 = 2;
    pub const ALT: u8 = 4;

    pub fn shift(&self) -> bool {
        self.modifiers & Self::SHIFT != 0
    }
    pub fn primary(&self) -> bool {
        self.modifiers & Self::PRIMARY != 0
    }
    pub fn alt(&self) -> bool {
        self.modifiers & Self::ALT != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerEvent {
    pub position: Point,
    pub down: bool,
}

/// The event sink: enqueue-only — may be invoked re-entrantly from inside any Toolkit method;
/// day-core drains queued events at safe points, each as a fresh batch (§3.3).
pub type EventSink = Box<dyn Fn(NodeId, Event)>;

/// The synchronous row-pull seam for recycling lists (docs/list.md, §10). day-core injects one
/// per `LIST` host via [`Toolkit::attach_list`]; a recycling backend stores it and calls it from
/// its native data-source (on the UI thread, outside any day-core borrow). Each closure re-enters
/// day-core, so — unlike [`EventSink`] — these run to completion synchronously (`bind_row` even
/// flushes + lays out the row before returning, so the host can measure the cell immediately).
#[derive(Clone)]
pub struct ListSource {
    /// Current row count.
    pub len: std::rc::Rc<dyn Fn() -> usize>,
    /// Stable identity token for the row at `index` (for native diffing / animation).
    pub token_at: std::rc::Rc<dyn Fn(usize) -> u64>,
    /// Build (first use of this cell) or rebind (recycled cell) row `index` into the native cell.
    pub bind_row: std::rc::Rc<dyn Fn(usize, RawHandle)>,
    /// The native cell left the viewport — Day may drop per-cell bookkeeping (optional).
    pub recycle: std::rc::Rc<dyn Fn(RawHandle)>,
    /// Drag-to-reorder seam, present when `ListProps::reorderable` (docs/list.md). `None` on
    /// non-reorderable lists — backends must not enable their drag machinery without it.
    pub reorder: Option<ListReorder>,
    /// Swipe-to-delete seam, present when `ListProps::deletable` (docs/list.md). `None` on
    /// non-deletable lists — backends must not offer their delete affordance without it.
    pub delete: Option<ListDelete>,
    /// Swipe-ACTIONS seam, present when `ListProps::swipe_actions` (docs/list.md). `None` on
    /// lists that declared none — backends must not install their row-action machinery
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
    /// Returns the ACCEPTED target index — `to` itself to allow, another index to retarget the
    /// drop, or `-1` to deny.
    pub can_move: std::rc::Rc<dyn Fn(usize, usize) -> i64>,
    /// Commit: row `from` dropped at row `to` (an index `can_move` accepted). Rotates Day's row
    /// snapshot BEFORE returning — so `len`/`token_at`/`bind_row` reflect the new order while the
    /// native move animates — and defers the app's own callback to the next event drain.
    pub move_row: std::rc::Rc<dyn Fn(usize, usize)>,
}

/// The synchronous swipe-to-delete half of [`ListSource`] (docs/list.md), shaped exactly like
/// [`ListReorder`]: both closures are called on the UI thread from inside native swipe callbacks,
/// outside any day-core borrow, and run to completion synchronously.
///
/// Each platform spells the gesture its own way — a trailing swipe action on iOS, an
/// `ItemTouchHelper` swipe on Android, a `ListItem` swipe action on ArkUI — but all three ask the
/// same two questions, so the seam is one pair of closures rather than three shapes.
#[derive(Clone)]
pub struct ListDelete {
    /// May row `index` be deleted? Called before the affordance is offered, so a row the app
    /// protects shows no delete action at all rather than one that fails on use.
    pub can_delete: std::rc::Rc<dyn Fn(usize) -> bool>,
    /// Commit: delete row `index`. Removes it from Day's row snapshot BEFORE returning — so
    /// `len`/`token_at`/`bind_row` already reflect the shorter list while the native row-removal
    /// animates — and defers the app's own callback to the next event drain.
    pub delete_row: std::rc::Rc<dyn Fn(usize)>,
}

/// Which edge of a row a swipe action rides (docs/list.md). Semantic, not geometric:
/// `Leading` follows the reading direction — the left edge in LTR, the right in RTL — the
/// mapping every platform's own swipe API already makes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwipeEdge {
    Leading,
    Trailing,
}

/// One offered swipe action (docs/list.md) — pure data. The pieces layer keeps the handlers;
/// the backend answers a tap through [`ListSwipe::perform`] by index into the offer.
#[derive(Clone, Debug, PartialEq)]
pub struct ListSwipeAction {
    /// The word on the button, already localized (a toolkit cannot translate).
    pub label: String,
    /// Destructive styling — the platform's red.
    pub destructive: bool,
    /// Explicit button color; `None` = the platform's default for the style.
    pub tint: Option<Color>,
}

/// The synchronous swipe-ACTIONS half of [`ListSource`] (docs/list.md) — the generalized
/// sibling of [`ListDelete`]: reveal-as-you-swipe buttons on either row edge. The offer is
/// pulled per row AT GESTURE TIME, so labels follow row state ("Mark Read" on an unread row,
/// "Mark Unread" on a read one); the commit defers to the event drain like every seam here.
#[derive(Clone)]
pub struct ListSwipe {
    /// The OFFER: the actions for row `index` on `edge`, called inside the platform's swipe
    /// callback — keep it pure. Empty = no affordance for that row and edge. The FIRST action
    /// is the full-swipe action where the platform activates one at the far edge.
    pub actions_at: std::rc::Rc<dyn Fn(usize, SwipeEdge) -> Vec<ListSwipeAction>>,
    /// The COMMIT: the user activated action `action` (an index into the offer) on row
    /// `index`, `edge`. Unlike a delete this never reshapes the row snapshot; the app's
    /// handler is deferred to the next event drain, never run inside the native callback.
    pub perform: std::rc::Rc<dyn Fn(usize, SwipeEdge, usize)>,
}

/// The synchronous row-pull seam for hierarchical trees (docs/tree.md) — [`ListSource`]'s
/// shape, addressed by TOKEN instead of row index, because expanding a row renumbers every
/// index below it while tokens hold still. day-core injects one per `TREE` host via
/// [`Toolkit::attach_tree`]; the backend answers its native data-source from it. The same
/// synchronous discipline as [`ListSource`]: UI thread, outside any day-core borrow;
/// `bind_row` flushes and lays the row out before returning.
#[derive(Clone)]
pub struct TreeSource {
    /// How many children `parent` has (`None` = the root level).
    pub children_len: std::rc::Rc<dyn Fn(Option<u64>) -> usize>,
    /// The i-th child of `parent` — the stable token every backend keys its rows by.
    pub child_token: std::rc::Rc<dyn Fn(Option<u64>, usize) -> u64>,
    /// Whether this token can hold children at all — what draws (or omits) the disclosure.
    pub expandable: std::rc::Rc<dyn Fn(u64) -> bool>,
    /// Build (first use of this cell) or rebind (recycled cell) the row for `token` into the
    /// native cell.
    pub bind_row: std::rc::Rc<dyn Fn(u64, RawHandle)>,
    /// The native cell left the viewport — Day may drop per-cell bookkeeping (optional).
    pub recycle: std::rc::Rc<dyn Fn(RawHandle)>,
    /// Re-lay the row bound to this cell at `width` — called from the cell's own native
    /// layout pass. Trees need this where lists don't: indentation makes every cell's
    /// content width per-row, so the host-width layout `bind_row` did is only a first
    /// approximation. Skips quietly when called inside a day-core borrow (a snapshot pass).
    pub layout_cell: std::rc::Rc<dyn Fn(RawHandle, f64)>,
    /// The row's type-ahead string (docs/tree.md) — what native type-select matches against.
    pub type_select_text: std::rc::Rc<dyn Fn(u64) -> String>,
    /// The row's context menu, built AT SUMMON TIME (docs/menus.md "Dynamic context
    /// menus") — called on the UI thread from the platform's own menu callback, outside any
    /// day-core borrow, like every closure here. `None` = rows carry no menu; an empty
    /// result suppresses the menu for that row.
    pub row_menu: Option<std::rc::Rc<dyn Fn(u64) -> Vec<MenuItem>>>,
    /// Drag-to-move seam, present when `TreeProps::movable` (docs/tree.md). `None` on
    /// immovable trees — backends must not enable their drag machinery without it.
    pub moves: Option<TreeMoves>,
}

/// The synchronous drag half of [`TreeSource`] (docs/tree.md). Both closures follow
/// `bind_row`'s discipline: called on the UI thread from inside native drag callbacks.
#[derive(Clone)]
pub struct TreeMoves {
    /// The live verdict, consulted while the drag is over a target: may `token` land under
    /// `parent` at `index`? `index: None` means "dropped ONTO the parent" (append). Pure —
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

/// A tree move guard's verdict (docs/tree.md). No `Retarget` yet — the drop vocabulary is
/// already positional, so a guard that wants a different target denies and the user drops
/// where it is allowed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MoveVerdict {
    Allow,
    Deny,
}

/// One addressable widget within a COMPOSITE native backing (docs/tree.md,
/// docs/tweaks.md) — Qt's own name for the concept (`QStyle::SubControl`). A `list`'s or
/// `tree`'s node handle is its scroller; `Subcontrol::Content` is the widget inside it.
/// Backends resolve an unknown subcontrol to `None`, never to the host.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Subcontrol {
    /// The node's own handle — what every tweak accessor reached before subcontrols existed.
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
/// Distinct from `SelectorItem::badge`, which annotates a sidebar ROW inside the window. This one
/// decorates the application itself and is drawn by the shell, not by Day.
///
/// What each payload needs is asked separately — `Cap::AppBadgeCount`, `AppBadgeText`,
/// `AppBadgeDot` — because the platforms differ in WHAT they accept more than in whether they have
/// a badge at all: macOS takes arbitrary text, iOS and the web take a number, Android takes nothing.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum AppBadge {
    /// Clear the badge.
    #[default]
    None,
    /// A count. Zero clears it, matching every platform's own convention.
    Count(u32),
    /// Short arbitrary text (`"99+"`, `"beta"`). Only macOS renders it — probe `Cap::AppBadgeText`.
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
    /// `< 600dp` — a phone in portrait. One pane at a time.
    #[default]
    Compact,
    /// `600–839dp` — a tablet in portrait, a narrow desktop window.
    Medium,
    /// `840–1199dp` — a tablet in landscape, a typical desktop window.
    Expanded,
    /// `1200–1599dp` — a large desktop window.
    Large,
    /// `≥ 1600dp` — a maximized window on a big display.
    ExtraLarge,
}

/// How tall a window is (docs/size-classes.md). Consulted far less than [`WidthClass`]: it is
/// what tells a phone in LANDSCAPE — wide enough for two panes, too short for a tall list — that
/// it should not grow vertical chrome.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub enum HeightClass {
    /// `< 480dp` — a phone in landscape.
    #[default]
    Compact,
    /// `480–899dp` — a phone in portrait, a tablet in landscape.
    Medium,
    /// `≥ 900dp` — a tablet in portrait, most desktop windows.
    Expanded,
}

/// A window's size class (docs/size-classes.md), reported by the backend and read by apps
/// through `day::size_class()`.
///
/// PER-WINDOW, not per-app: two windows of one process can sit in different classes at the same
/// time (a narrow and a wide window side by side, iPadOS Stage Manager, Android split-screen), and
/// an app that keyed off a single global would lay the second window out for the first one's size.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct SizeClass {
    pub width: WidthClass,
    pub height: HeightClass,
}

impl SizeClass {
    /// The `Compact`/`Medium` width boundary in dp — the narrowest window where a second
    /// pane starts to fit. Backends that hand an adaptive container its pane widths derive
    /// them from this (Android's list + minimum-detail widths sum to it), so the platform's
    /// measure-time decision and this table agree by construction (docs/size-classes.md).
    pub const SPLIT_MIN_WIDTH: f64 = 600.0;

    /// Bucket a window's size in points. The one place the breakpoint numbers appear.
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
    pub fn prefers_split(self) -> bool {
        self.width > WidthClass::Compact
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
    ListRecycling,
    /// The toolkit fronts the app's undo stack with the platform's own undo system
    /// ([`Toolkit::set_undo_state`] + `Event::Undo`) — `Native` where one exists
    /// (`NSUndoManager` on macOS/iOS, `QUndoStack`), `Unsupported` everywhere else, where
    /// Day's menu items and accelerators drive the stack directly and nothing platform-owned
    /// needs mirroring (docs/model.md).
    UndoBridge,
    /// The toolkit routes the platform's standard Cut/Copy/Paste to the app's edit bridge
    /// ([`Toolkit::set_edit_state`] + `Event::Edit`): the SAME menu items, shortcuts, and
    /// responder precedence the platform's text editing uses — a focused text widget keeps
    /// its own clipboard behavior, everything else reaches the app (docs/menus.md).
    /// `Native` where a system route exists (the responder chain, the browser's clipboard
    /// events), `Emulated` where Day's own menu items dispatch it, `Unsupported` elsewhere.
    EditBridge,
    /// The toolkit realizes `ListProps::reorderable` as drag-to-reorder rows — `Native` when the
    /// platform's own mechanism drives it (NSTableView drag/drop, UITableView drag delegates,
    /// ItemTouchHelper, …), `Emulated` for a pointer-tracked fake (web-dom). `Unsupported` ⇒ the
    /// list renders normally but rows cannot be dragged (docs/list.md).
    ListReorder,
    /// The toolkit realizes `ListProps::deletable` as the platform's own delete gesture —
    /// `Native` where the platform ships one (UIKit's trailing swipe actions, Android's
    /// `ItemTouchHelper`, ArkUI's `ListItem.swipeAction`), `Unsupported` on the desktop toolkits,
    /// whose lists have no swipe idiom and where deletion belongs to a menu or a button
    /// (docs/list.md).
    ListDelete,
    /// The toolkit realizes `ListProps::swipe_actions` as REAL edge swipe actions — buttons
    /// that reveal as the row tracks the swipe, with the far edge activating the first one
    /// (docs/list.md). `Native` where the platform ships the affordance: macos-appkit
    /// (`NSTableView` row actions — Mail's own machinery) and ios-uikit
    /// (`UISwipeActionsConfiguration`). `Unsupported` everywhere else: the affordance is
    /// simply absent, and the app's command lives in its explicit controls.
    ListSwipeActions,
    /// The toolkit realizes `kinds::TREE` as a hierarchical tree view (docs/tree.md) —
    /// `Native` where a platform tree widget hosts Day-built rows (NSOutlineView, the UIKit
    /// sidebar list, GtkListView+TreeListModel, WinUI TreeView, ArkTS TreeView), `Emulated`
    /// where Day flattens the tree onto its list machinery, `Unsupported` where neither
    /// exists yet — an app gates its tree UI on this answer rather than showing a placeholder.
    Tree,
    /// The toolkit realizes `TreeProps::movable` as drag-to-reparent/restack rows
    /// (docs/tree.md). A strictly smaller set than [`Cap::Tree`]: a backend can render a
    /// tree it cannot yet drag within.
    TreeMove,
    /// The toolkit answers [`Toolkit::first_baseline`], so rows can align text on its baseline
    /// rather than on the middle of its box (docs/baseline.md). `Native` where the platform
    /// reports the baseline itself (`NSView.firstBaselineOffsetFromTop`, `View.getBaseline`,
    /// `gtk_widget_measure`), `Emulated` where Day derives it from the widget's font metrics,
    /// `Unsupported` ⇒ baseline-aligned rows fall back to centering and look exactly as they do
    /// today.
    BaselineAlignment,
    /// The toolkit draws a label's [`TextRun`]s — bold, italic, color or a monospace face within
    /// one wrapping paragraph (docs/text-runs.md). `Unsupported` ⇒ the label renders its text
    /// uniformly, which reads correctly and loses only the emphasis.
    TextRuns,
    /// The toolkit makes a run carrying [`TextRun::link`] ACTIVATABLE, emitting
    /// [`Event::LinkActivated`]. A strictly smaller set than [`Cap::TextRuns`]: several toolkits
    /// draw a link run in link colors but have no way to hit-test it, and one (Android) can do
    /// links or selection but not both (docs/text-runs.md).
    TextLinks,
    Lottie,
    NativeSymbols,
    /// The toolkit can rasterize its OWN window and hand back the pixels
    /// (`Toolkit::snapshot_window`) — what the dayscript `screenshot` step captures in-process
    /// and what `day::window_image()` offers the app (docs/window-image.md). `Unsupported` where
    /// a backend cannot draw itself into a bitmap: web-dom, which would need a rasterizer
    /// shipped with it. Probe it before offering a "save a screenshot" affordance, rather than
    /// offering one that fails when pressed.
    Snapshot,
    /// The toolkit CAN present `nav()` as sidebar+detail split panes — a statement about the
    /// toolkit, not about the window it is currently drawing. Whether a given host is split right
    /// now follows from its [`SizeClass`]: the pieces layer resolves an automatic
    /// [`props::NavPresentation`] against both, and re-resolves on every class change. A backend
    /// with no split container answers `Unsupported` and stays stacked at every size.
    NavSplit,
    /// The toolkit CAN draw a navigation host's rows as its own chrome — a tab bar
    /// ([`props::NavPresentation::Tabs`]) and, where it has one, a rail
    /// ([`props::NavPresentation::Rail`]). Like [`Self::NavSplit`] this is a statement about the
    /// toolkit, not about the window it is drawing right now.
    ///
    /// This is what `SelectorStyle::Automatic` resolves against (docs/navigation.md): a backend
    /// answering `Unsupported` gets the sidebar resolver instead (`Split` ↔ `Stack`), which is
    /// what every backend did before adaptive navigation existed. So an unimplemented backend
    /// degrades to its previous behavior rather than to a hole.
    ///
    /// `Emulated` means the rows are composed from other widgets rather than drawn by a native
    /// tab container (Qt, web-dom). `Native` means a real one (`UITabBarController`, a Material
    /// `NavigationBarView`, `NavigationView`).
    NavTabs,
    /// A narrow window should BECOME a tab bar here — the separate question from [`Self::NavTabs`],
    /// which only says the toolkit *can* draw one.
    ///
    /// The two differ on every desktop. macOS, GNOME, Qt and Windows can all draw a tab bar, and
    /// must, because an app is free to pin `SelectorStyle::Tabs`; but none of them grows one when
    /// its window is dragged narrow. A narrow Mail.app hides its sidebar and pushes — it does not
    /// sprout a bottom tab bar, and an app that did would look like a port. The phones and the web
    /// are the opposite: those are the surfaces whose window size genuinely ranges from a phone to
    /// a desktop, and a tab bar is what their users expect at the narrow end.
    ///
    /// So this is a statement about the platform's IDIOM, not about its widget set:
    ///
    /// - `Native`/`Emulated` — `SelectorStyle::Automatic` may resolve to
    ///   [`props::NavPresentation::Tabs`] on a compact window (ios-uikit, android-mdc,
    ///   harmony-arkui, web-dom).
    /// - `Unsupported` — it may not; a compact window collapses to
    ///   [`props::NavPresentation::Stack`] instead, exactly as `SelectorStyle::Sidebar` has always
    ///   done (macos-appkit, linux-gtk, linux-qt, windows-xaml).
    ///
    /// [`props::NavPresentation::Rail`] is NOT gated by this. A narrow sidebar is an ordinary
    /// desktop shape — on Windows it is literally what `NavigationView` does at that width on its
    /// own — so the rail rung stays available everywhere `Cap::NavSplit` is.
    NavTabsAdaptive,
    /// How a navigation host's presentation follows the window (docs/size-classes.md). All three
    /// answers mean something different here, and the difference is WHO DECIDES:
    ///
    /// - `Native` — **Day tells the toolkit.** The pieces layer resolves the presentation from
    ///   the [`SizeClass`], re-resolves on every change, and sends
    ///   [`props::NavPatch::Presentation`]; the toolkit rebuilds its chrome and re-homes the
    ///   pages it already has. The desktops and web-dom.
    /// - `Emulated` — **the toolkit tells Day.** Its own adaptive container owns the decision
    ///   (`UISplitViewController` collapsing as a Pro Max iPhone rotates, `SlidingPaneLayout`
    ///   measuring whether both panes fit), so Day must NOT push a presentation into it — that
    ///   would be a second source of truth racing the platform's own animation. The toolkit
    ///   reports what it did with [`Event::NavPresentationChanged`] and Day reconciles.
    /// - `Unsupported` — nobody re-presents. The presentation is resolved once, from
    ///   [`Cap::NavSplit`] alone, and the window's size never enters into it. A toolkit that
    ///   cannot change presentation must not have it decided by something that can, or a window
    ///   launched narrow would be stuck stacked with no way back.
    NavRepresent,
    /// The toolkit places a navigation host's CONTENT-LIST page ([`props::Pane::List`]) in its
    /// own pane between the sidebar and the detail — the Mail shape: mailboxes, message list,
    /// message (docs/navigation.md). The three answers differ on what happens when the host
    /// leaves the split:
    ///
    /// - `Native` — the pane exists at EVERY presentation. A narrow window collapses the
    ///   sidebar but keeps the list beside the detail, exactly as a narrow Mail.app does
    ///   (macos-appkit: a `contentList` `NSSplitViewItem`).
    /// - `Emulated` — the pane exists while split and MERGES into the navigation stack when
    ///   the host collapses (ios-uikit: `UISplitViewController` triple-column). The pieces
    ///   layer then interposes the list between the sidebar root and the detail
    ///   ([`props::NavPatch::ListInStack`]) and gates the detail push on the app's
    ///   `detail_visible` binding.
    /// - `Unsupported` — the backend never sees [`props::Pane::List`] or
    ///   [`props::NavProps::list_width`]; the selector piece composes the pane beside (split)
    ///   or in place of (stacked) the destination content itself.
    NavContentList,
    /// The toolkit shows the current destination's title in a NATIVE header/bar — so a page
    /// needn't repeat it in its own content. `Native` on XAML (the NavigationView header),
    /// UIKit (`UINavigationBar`), Android (`MaterialToolbar`), and ArkUI (`NavDestination`
    /// title bars). The desktop custom back-headers (AppKit/Qt/web) show a title only in
    /// stack mode, so those backends stay `Unsupported` (docs/navigation.md).
    NavHeader,
    /// The toolkit applies a runtime appearance override (`set_appearance`): native widgets
    /// restyle in place and `dark_mode` answers the override. Probe before showing a theme
    /// picker — on `Unsupported` backends the call is ignored.
    Appearance,
    /// The toolkit renders an application MENU BAR (docs/menus.md) — so `app_menu` is visible to
    /// the user, and `register_preferences` gets its automatic Settings… item.
    ///
    /// The question an app asks before deciding where a command LIVES. Settings is the usual
    /// case: on a platform with a menu bar it belongs in the App menu and putting it in the
    /// navigation as well is clutter; on one without, the menu is a no-op and a Settings row is
    /// the only way in. Distinct from `Cap::Toolbar` — web-dom draws a window toolbar and has no
    /// menu bar at all, so keying off the toolbar strands Settings there.
    AppMenu,
    /// The toolkit can present native alert/confirm/sheet/prompt modals (docs/dialogs.md).
    Dialogs,
    /// The toolkit can present native open/save file pickers (docs/files.md).
    FileDialogs,
    /// The toolkit runs backend-executed animation for `AnimSpec` intents on
    /// `update`/`set_frame`/`set_opacity`/`set_transform` (§8.4). `Unsupported` ⇒ animated calls
    /// apply instantly (still correct, just not animated).
    Animation,
    /// The toolkit presents a `kinds::COVER` node as a native fullscreen modal surface
    /// (docs/cover.md). `Unsupported` ⇒ the `cover` piece's content never shows.
    Cover,
    /// The toolkit's `text_area` can be made read-only (`TextAreaProps::editable = false`).
    TextEditable,
    /// The toolkit's `text_area` selectability can be toggled (`TextAreaProps::selectable`).
    /// `Unsupported` where selection is always on (GTK) or the editor isn't wired (ArkUI).
    TextSelectable,
    /// The toolkit's `text_area` has built-in spell-check/autocorrect (`TextAreaProps::spellcheck`).
    /// `Unsupported` where the toolkit ships none (GTK, Qt, ArkUI).
    TextSpellCheck,
    /// The toolkit can open additional native OS windows (`Toolkit::open_window`,
    /// docs/windows.md). `Native` on the desktop backends and on mobile backends whose
    /// platform has a real secondary-window surface (iPad scenes, Android document
    /// activities, OHOS multiton abilities). On `Unsupported` backends
    /// `day_core::open_window` still works — the content presents as a fullscreen cover in
    /// the primary window instead. Probe it to adapt chrome: an `Unsupported` tier has no
    /// native title bar or close button, so window content should carry its own close
    /// affordance.
    MultiWindow,
    /// The toolkit can put a COUNT on the app icon (`Toolkit::set_app_badge`, docs/badge.md).
    /// `Emulated` where the call is made but the shell may ignore it — desktop Linux, where the
    /// Unity launcher protocol is honored by Plasma and Dash-to-Dock but not by stock GNOME.
    AppBadgeCount,
    /// The toolkit renders `AppBadge::Text` as written. macOS only: `NSDockTile.badgeLabel` is an
    /// arbitrary string, while every other platform's badge is a number or nothing.
    AppBadgeText,
    /// The toolkit can show a valueless indicator (`AppBadge::Dot`).
    AppBadgeDot,
    /// The toolkit gives a window a native toolbar (`Toolkit::set_toolbar`, docs/toolbars.md):
    /// `Native` on the desktop backends, `Unsupported` elsewhere — a phone has no toolbar, and
    /// day does not draw a fake one. Probe it to decide where a command lives: an app puts its
    /// refresh button on the toolbar where there is one and in the content where there is not.
    Toolbar,
    /// The toolkit realizes `kinds::INSPECTOR` as its own trailing-pane container
    /// (docs/inspector.md): an `NSSplitView` inspector pane, an `AdwOverlaySplitView` with the
    /// sidebar at the end, a `QDockWidget`, a XAML `SplitView` right pane. `Unsupported` ⇒ the
    /// `inspector` piece composes the pane from plain containers instead — same panel, same
    /// signal, no native divider — so an unimplemented backend degrades to a drawn pane, not
    /// to a hole. Apps normally have no reason to probe this; the piece does.
    Inspector,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Support {
    Native,
    Emulated,
    /// The default — what an unimplemented capability answers everywhere.
    #[default]
    Unsupported,
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

    pub fn contains(self, other: Edges) -> bool {
        self.0 & other.0 == other.0
    }
    pub fn union(self, other: Edges) -> Edges {
        Edges(self.0 | other.0)
    }
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

/// The timing curve of an animation (§8.4). Native backends map each variant onto their own
/// easing (`CAMediaTimingFunction`, `QEasingCurve`, ArkUI `ARKUI_CURVE_*`, spring animators); the
/// canvas/self-driven path samples it via [`Curve::fraction`]. `Spring` matches SwiftUI's
/// `.spring(response:dampingFraction:)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Curve {
    Linear,
    EaseIn,
    EaseOut,
    EaseInOut,
    /// `response` = approximate settling period (seconds); `damping` = damping ratio (1.0 =
    /// critically damped, `<1` = bouncy overshoot).
    Spring {
        response: f64,
        damping: f64,
    },
}

impl Curve {
    /// Fraction of the transition complete at `elapsed` seconds (0 at start, reaching — or, for an
    /// under-damped spring, overshooting — 1). Easing curves clamp to `duration`; springs evaluate
    /// their analytic unit-step response and use `duration` only as a settle cap. Drives the
    /// self-driven/canvas path; native backends interpolate on their own compositor instead.
    pub fn fraction(self, elapsed: f64, duration: f64) -> f64 {
        match self {
            Curve::Spring { response, damping } => spring_step(response, damping, elapsed),
            _ => {
                let t = if duration <= 0.0 {
                    1.0
                } else {
                    (elapsed / duration).clamp(0.0, 1.0)
                };
                self.ease(t)
            }
        }
    }

    /// Eased progress for normalized `t` in `0.0..=1.0` (springs pass through — use [`fraction`]).
    #[inline]
    pub fn ease(self, t: f64) -> f64 {
        match self {
            Curve::Linear | Curve::Spring { .. } => t,
            Curve::EaseIn => t * t,
            Curve::EaseOut => 1.0 - (1.0 - t) * (1.0 - t),
            Curve::EaseInOut => t * t * (3.0 - 2.0 * t), // smoothstep
        }
    }

    /// Whether the transition has settled, so the canvas frame clock can stop ticking it.
    pub fn is_settled(self, elapsed: f64, duration: f64) -> bool {
        match self {
            Curve::Spring { response, damping } => {
                let cap = response.max(0.0) * 4.0 + 0.2;
                if elapsed >= cap {
                    return true;
                }
                (spring_step(response, damping, elapsed) - 1.0).abs() < 0.001
                    && elapsed > response * 0.5
            }
            _ => elapsed >= duration.max(0.0),
        }
    }
}

/// Unit-step response of a second-order spring (`response` = period seconds, `damping` = ratio),
/// evaluated at `t` seconds. Under-damped rings and overshoots; critically/over-damped eases in.
fn spring_step(response: f64, damping: f64, t: f64) -> f64 {
    if response <= 0.0 || t <= 0.0 {
        return if t <= 0.0 { 0.0 } else { 1.0 };
    }
    let omega0 = std::f64::consts::TAU / response;
    let zeta = damping.max(0.0);
    if zeta < 1.0 {
        let omega_d = omega0 * (1.0 - zeta * zeta).sqrt();
        let e = (-zeta * omega0 * t).exp();
        1.0 - e * ((omega_d * t).cos() + (zeta * omega0 / omega_d) * (omega_d * t).sin())
    } else {
        let e = (-omega0 * t).exp();
        1.0 - e * (1.0 + omega0 * t)
    }
}

/// Animation intent (§8.4). Native-widget backends map it onto their own animator (Core Animation,
/// `ViewPropertyAnimator`, XAML Composition, `OH_ArkUI_AnimateTo`, …); the canvas/self-driven path
/// samples `curve` via [`Curve::fraction`]. Threaded through `Toolkit::update`/`set_frame`/
/// `set_opacity`/`set_transform`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimSpec {
    pub duration_ms: u32,
    pub delay_ms: u32,
    pub curve: Curve,
    /// Repeat count beyond the first play (`0` = play once); `u32::MAX` = repeat forever.
    pub repeat: u32,
    pub autoreverse: bool,
}

impl Default for AnimSpec {
    /// The default feel: a smooth spring (SwiftUI's modern default).
    fn default() -> Self {
        AnimSpec {
            duration_ms: 350,
            delay_ms: 0,
            curve: Curve::Spring {
                response: 0.4,
                damping: 0.8,
            },
            repeat: 0,
            autoreverse: false,
        }
    }
}

impl AnimSpec {
    /// A smooth spring: `response` = settling period (s), `damping` = ratio (1.0 = no bounce, `<1`
    /// bouncy). `duration_ms` is a nominal cap for backends that need a duration; spring backends
    /// use `response`/`damping` directly.
    pub fn spring(response: f64, damping: f64) -> Self {
        AnimSpec {
            // `response` is the animation's duration: every backend maps a spring to a
            // fixed-duration overshoot curve over exactly `duration_ms`, so timing is identical
            // across toolkits (a physics spring's settle time would vary per platform).
            duration_ms: (response * 1000.0).max(50.0) as u32,
            delay_ms: 0,
            curve: Curve::Spring { response, damping },
            repeat: 0,
            autoreverse: false,
        }
    }
    pub fn linear(duration_ms: u32) -> Self {
        Self::timed(duration_ms, Curve::Linear)
    }
    pub fn ease_in(duration_ms: u32) -> Self {
        Self::timed(duration_ms, Curve::EaseIn)
    }
    pub fn ease_out(duration_ms: u32) -> Self {
        Self::timed(duration_ms, Curve::EaseOut)
    }
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
    pub fn delay(mut self, ms: u32) -> Self {
        self.delay_ms = ms;
        self
    }
    /// Repeat `count` extra times (builder); `autoreverse` ping-pongs each cycle.
    pub fn repeat(mut self, count: u32, autoreverse: bool) -> Self {
        self.repeat = count;
        self.autoreverse = autoreverse;
        self
    }
    /// Repeat forever (builder) — e.g. a pulsing indicator.
    pub fn repeat_forever(mut self, autoreverse: bool) -> Self {
        self.repeat = u32::MAX;
        self.autoreverse = autoreverse;
        self
    }
    #[inline]
    pub fn duration_secs(&self) -> f64 {
        self.duration_ms as f64 / 1000.0
    }
    #[inline]
    pub fn delay_secs(&self) -> f64 {
        self.delay_ms as f64 / 1000.0
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
}

impl Role {
    /// The a11y role a built-in piece kind reports natively — the audit's *expectation* when the
    /// user hasn't set an explicit `.role()`. Native controls already expose these, so Day records
    /// them for `a11y_audit` (§14.2) rather than overriding the widget; only canvas/custom pieces
    /// need Day to apply a role. Returns `None` for kinds with no inherent control role.
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
}

impl A11yProps {
    /// Merge another set of annotations onto this one: any field `other` sets — a `Some`, a
    /// non-`None` role, or a `true` flag — overrides ours; unset fields are left intact. Lets a
    /// node accumulate its `.a11y()`, `.id()`, and piece defaults into one stored result, so
    /// each `set_a11y` re-applies the full picture and `a11y_audit` has the complete expectation.
    pub fn merge(&mut self, other: &A11yProps) {
        if other.label.is_some() {
            self.label = other.label.clone();
        }
        if other.hint.is_some() {
            self.hint = other.hint.clone();
        }
        if other.value.is_some() {
            self.value = other.value.clone();
        }
        if other.role != Role::None {
            self.role = other.role;
        }
        if other.identifier.is_some() {
            self.identifier = other.identifier.clone();
        }
        self.hidden |= other.hidden;
        self.decorative |= other.decorative;
    }

    /// The role to *expect* for a node of `kind` carrying these annotations: an explicit
    /// `.role()` wins, otherwise the kind's native default (`Role::for_kind`).
    pub fn resolved_role(&self, kind: PieceKind) -> Role {
        if self.role != Role::None {
            self.role
        } else {
            Role::for_kind(kind)
        }
    }
}

/// A widget's ACTUAL native accessibility properties, read back by `Toolkit::read_a11y` so
/// `a11y_audit` (§14.2) can diff the native tree against Day's expectation. `role` is the native
/// role mapped back to Day's `Role` (best-effort); `found = false` means the backend can't read
/// the native tree (audit skips the node).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct A11ySnapshot {
    pub found: bool,
    pub role: Role,
    pub label: Option<String>,
    pub value: Option<String>,
    pub identifier: Option<String>,
}

// ---------------------------------------------------------------------------
// Canvas display list (§11) — full op set lands with M8a; the types are v1.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Rect(Rect),
    RoundedRect(Rect, f64),
    Ellipse(Rect),
    /// Arc within `rect`'s inscribed ellipse; angles in degrees, 0 = +x axis, clockwise.
    Arc {
        rect: Rect,
        start_deg: f64,
        sweep_deg: f64,
    },
    Line(Point, Point),
    Polygon(Vec<Point>),
    /// An arbitrary path: curves, several contours, and a fill rule (docs/canvas.md).
    ///
    /// [`Shape::Polygon`] is the straight-line, single-contour special case, kept because it is
    /// what most drawing code actually wants and it encodes far more compactly.
    Path(Path),
}

impl Shape {
    /// The shape's bounding rectangle — the box gradient [`UnitPoint`]s resolve against.
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
/// is a property of the FILL, so stroking ignores it.
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
    /// A conservative bounding box: the hull of every point INCLUDING control points, which
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
/// [`StrokeStyle::hairline`] and `From<f64>` keep the common "just a width" case a single
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
    pub fn width(width: f64) -> Self {
        StrokeStyle {
            width,
            ..Default::default()
        }
    }
    /// A dashed stroke: `dash` is on/off run lengths, repeating.
    pub fn dashed(width: f64, dash: Vec<f64>) -> Self {
        StrokeStyle {
            width,
            dash,
            ..Default::default()
        }
    }
    /// Rounded ends AND joins — the shape a data line usually wants.
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
/// `(offset 0..=1, color)`, ascending.
#[derive(Clone, Debug, PartialEq)]
pub struct LinearGradient {
    pub start: UnitPoint,
    pub end: UnitPoint,
    pub stops: Vec<(f64, Color)>,
}

impl LinearGradient {
    pub fn new(start: UnitPoint, end: UnitPoint, stops: Vec<(f64, Color)>) -> Self {
        LinearGradient { start, end, stops }
    }
    /// Top-to-bottom between two colors — the everyday sky/backdrop case.
    pub fn vertical(top: Color, bottom: Color) -> Self {
        LinearGradient::new(
            UnitPoint::TOP,
            UnitPoint::BOTTOM,
            vec![(0.0, top), (1.0, bottom)],
        )
    }
    /// Leading-to-trailing between two colors.
    pub fn horizontal(leading: Color, trailing: Color) -> Self {
        LinearGradient::new(
            UnitPoint::LEADING,
            UnitPoint::TRAILING,
            vec![(0.0, leading), (1.0, trailing)],
        )
    }
}

/// A radial gradient (docs/shapes.md §3.2 / §7): color stops from `center` outward. Both the
/// center and the radius live in the unit space of the filled shape's bounding box, so the
/// gradient stretches into an ELLIPSE when the bounds aren't square (the XAML relative-brush
/// behavior; the other backends reproduce it with a local matrix on a circular gradient). A
/// `radius` of `0.5` from the default center touches the edge midpoints of the bounds.
#[derive(Clone, Debug, PartialEq)]
pub struct RadialGradient {
    pub center: UnitPoint,
    pub radius: f64,
    pub stops: Vec<(f64, Color)>,
}

impl RadialGradient {
    pub fn new(center: UnitPoint, radius: f64, stops: Vec<(f64, Color)>) -> Self {
        RadialGradient {
            center,
            radius,
            stops,
        }
    }
    /// Centered, edge-touching (radius 0.5) between two colors — the everyday glow case.
    pub fn centered(inner: Color, outer: Color) -> Self {
        RadialGradient::new(UnitPoint::CENTER, 0.5, vec![(0.0, inner), (1.0, outer)])
    }
}

/// A fill source: a solid color, or a linear/radial gradient (docs/shapes.md §3.2 — angular and
/// semantic tokens are later phases). `From<Color>` keeps every existing `fill(shape, color)`
/// call site compiling unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum Paint {
    Solid(Color),
    Linear(LinearGradient),
    Radial(RadialGradient),
}

impl From<Color> for Paint {
    fn from(c: Color) -> Self {
        Paint::Solid(c)
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

/// How canvas text hangs on its `at` point (style rule: no bare bools in public APIs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum TextAnchor {
    /// `at` is the top-leading corner.
    #[default]
    Leading,
    /// `at` is the center.
    Centered,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DrawOp {
    Fill(Shape, Paint),
    /// Stroke `shape`. The paint may be a gradient, and [`StrokeStyle`] carries width, dash, cap
    /// and join — `Draw::stroke` builds the plain width-only case.
    Stroke(Shape, Paint, StrokeStyle),
    Text {
        text: String,
        at: Point,
        size: f64,
        color: Color,
        anchor: TextAnchor,
    },
    /// Intersect the clip with `shape`; everything drawn afterwards is confined to it.
    ///
    /// Scoped by [`DrawOp::Save`]/[`DrawOp::Restore`], which is the only way to widen a clip
    /// again — every native 2-D context works this way, so there is deliberately no "unclip".
    Clip(Shape),
    /// Push the current transform + clip (§11, shapes). Backends map to save/restore of the
    /// native 2-D context; `Concat` multiplies an affine onto the CTM (shape rotate/scale/offset).
    Save,
    Restore,
    Concat(day_geometry::Affine),
}

// ---------------------------------------------------------------------------
// Built-in piece descriptors: full props (realize) + sparse patches (update).
// One binding = one attribute = one patch value — sparseness by construction (§8.1).
// ---------------------------------------------------------------------------

/// A semantic (logical) text style. Each maps to the PLATFORM's native text style where the toolkit
/// has one — `UIFont`/`NSFont.preferredFont(forTextStyle:)` on Apple (Dynamic Type), the
/// `*TextBlockStyle` resources on XAML — so a Day app matches the OS's own typography and inherits its
/// accessibility text scaling for free. Backends without semantic styles (GTK/Qt/Android) approximate
/// with sizes that still track the platform's text-scale / font-scale accessibility setting.
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
    /// 24.0)`). The family must ship in the project's `fonts/` directory — `day build` stages the
    /// file into each platform's native font store and the backend registers it at startup
    /// (§18.4). The name is the family name baked into the font file (what Font Book /
    /// fontconfig report), not the file name. The size scales with the platform accessibility
    /// text-scale exactly like [`Font::System`]; weight/italic apply only where the family ships
    /// (or the platform synthesizes) such a face. An unknown family falls back to the system font
    /// of the same size, with a warning in the log.
    Custom(&'static str, f64),
}

impl Font {
    /// A bundled custom font by **typed family**, at a point size — the checked form of
    /// [`Font::Custom`]. Pass a generated `res::fonts::…` constant
    /// (`Font::custom(res::fonts::pacifico, 24.0)`), which exists only if the family ships in the
    /// project's `fonts/` directory, so the font is guaranteed bundled. For a family name known
    /// another way, the untyped [`Font::Custom`] variant is the escape hatch.
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

/// The full font descriptor a label carries: a semantic (or custom) [`Font`] style plus an optional
/// weight override, an italic flag, and the tabular-figures request. Backends resolve this to one
/// native font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontSpec {
    pub style: Font,
    pub weight: Option<FontWeight>,
    pub italic: bool,
    /// Ask for TABULAR (monospaced) figures: every digit takes the same advance, so a number that
    /// changes stops changing width.
    ///
    /// In a proportional system font `1` is narrower than `8`, so a readout beside a slider
    /// wobbles as the value changes even when the digit COUNT is fixed. Every platform ships this
    /// as a font feature, so Day asks for the platform's own rather than substituting a monospaced
    /// family: the text keeps the system face, and only the digits change metrics.
    ///
    /// Reserving space ([`Decorate::reserving`]) and tabular figures solve the two halves of the
    /// same problem — reservation stops the BOX resizing, tabular stops the GLYPHS shifting inside
    /// it — and a numeric readout usually wants both.
    ///
    /// This rides the existing label realize/patch path rather than adding a Toolkit method, so it
    /// is NOT a backend duty and has no duty-matrix row. Every in-tree backend honors it: AppKit
    /// and UIKit re-pick the system face (`monospacedDigitSystemFont`), GTK and Qt and ArkUI set
    /// the `tnum` OpenType feature, Android `setFontFeatureSettings`, XAML the `Typography`
    /// NumeralAlignment property, and web-dom `font-variant-numeric`. A font with no `tnum` table,
    /// or an SDK predating the attribute, renders the stock proportional figures.
    pub tabular: bool,
    /// Ask for the platform's MONOSPACED face at this style's size — what `` `code` `` in a
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
    /// RELATIVE rather than an absolute point size, and that is the whole point: an editor's
    /// font-size control moves this, so text a person reads for an hour still tracks the
    /// platform's accessibility text-scale. `Font::System(pt)` remains the absolute escape
    /// hatch, and it is where an imported document's `\fs28` or `font-size: 14px` lands
    /// (docs/texteditor.md) — faithful to the file, and no longer responsive to the reader,
    /// which is the trade that form makes.
    ///
    /// Three toolkits take it directly (`GtkTextTag::scale`, Android's `RelativeSizeSpan`, CSS
    /// `em`); the rest multiply it into the size they already compute.
    pub scale: f64,
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
    /// The plain descriptor for a style — no weight override, no italic, scale 1.0.
    pub fn new(style: Font) -> Self {
        FontSpec::from(style)
    }
    /// This descriptor at a different relative size (see [`FontSpec::scale`]).
    pub fn scaled(mut self, scale: f64) -> Self {
        self.scale = scale;
        self
    }
    /// The point size this descriptor resolves to, given the size the platform assigns its
    /// semantic style. The one place `scale` is applied, so every backend's font resolver
    /// multiplies the same way.
    pub fn resolved_points(&self, style_points: f64) -> f64 {
        let base = match self.style {
            Font::System(pt) | Font::Custom(_, pt) => pt,
            _ => style_points,
        };
        (base * self.scale).max(1.0)
    }
}

/// One styled span of a label's text (docs/text-runs.md).
///
/// `range` is a BYTE range into the label's `text`. Bytes, not chars: every backend that takes a
/// structured attributed string wants UTF-16 or byte offsets, `str` slicing is byte-based, and a
/// char count would have to be converted at every boundary. Ranges must be non-overlapping and
/// ascending; a gap between them is unstyled text, drawn with the label's own font.
///
/// A run says only what DIFFERS. `font` is the whole descriptor rather than a delta because a
/// bold run inside a Body paragraph is `FontSpec { style: Body, weight: Some(Bold), .. }` — the
/// style has to travel with the weight or the run loses its semantic size.
#[derive(Clone, Debug, PartialEq)]
pub struct TextRun {
    pub range: std::ops::Range<usize>,
    pub font: FontSpec,
    pub color: Option<Color>,
    /// A highlight painted behind the glyphs. Every native text editor carries this attribute
    /// (docs/texteditor.md), and search hits and review comments are the two things an app is
    /// asked for the moment it has styled text at all.
    pub background: Option<Color>,
    /// A line under the text.
    pub underline: Underline,
    /// A line through the text. Separate from `FontSpec` because it is a decoration, not a face:
    /// no platform expresses it by picking a different font.
    pub strikethrough: bool,
    /// Makes this run a link to the given target. Rendering it is [`Cap::TextRuns`]; making it
    /// ACTIVATABLE is [`Cap::TextLinks`], which is a smaller set — see the docs.
    pub link: Option<String>,
}

impl Default for TextRun {
    fn default() -> Self {
        TextRun {
            range: 0..0,
            font: FontSpec::default(),
            color: None,
            background: None,
            underline: Underline::None,
            strikethrough: false,
            link: None,
        }
    }
}

impl TextRun {
    /// A run that only changes the font — the common case for emphasis.
    pub fn font(range: std::ops::Range<usize>, font: FontSpec) -> Self {
        TextRun {
            range,
            font,
            ..TextRun::default()
        }
    }

    /// This run's attributes without its range — what a selection's style is compared against
    /// and what an editor's toolbar toggles (docs/texteditor.md).
    pub fn style(&self) -> RunStyle {
        RunStyle {
            font: self.font,
            color: self.color,
            background: self.background,
            underline: self.underline,
            strikethrough: self.strikethrough,
            link: self.link.clone(),
        }
    }

    /// A run of `style` over `range`.
    pub fn styled(range: std::ops::Range<usize>, style: RunStyle) -> Self {
        TextRun {
            range,
            font: style.font,
            color: style.color,
            background: style.background,
            underline: style.underline,
            strikethrough: style.strikethrough,
            link: style.link,
        }
    }
}

/// Serialize a label's text + runs as MARKUP, for the two backends whose text widgets take a
/// string in their own dialect rather than a structured attributed string: GTK (Pango markup)
/// and Qt (its HTML subset). Shared so the escaping is written and tested once — a translated
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
pub fn runs_to_markup(
    text: &str,
    runs: &[TextRun],
    dialect: MarkupDialect,
    base_points: f64,
) -> String {
    let mut out = String::with_capacity(text.len() + runs.len() * 24);
    let mut at = 0usize;
    for r in runs {
        // Skip a run whose range does not address this string, WITHOUT advancing `at`: dropping
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
    /// A 24×24 outline path for this symbol — Day's own drawing of it.
    ///
    /// The platform's icon comes FIRST on every backend that has one (SF Symbols, the freedesktop
    /// icon theme, Fluent's glyph font). This is the fallback for when that lookup finds nothing,
    /// which is not an edge case: `view-filter-symbolic` exists on a GNOME desktop and nowhere
    /// else, so a GTK or Qt app run off that desktop drew a toolbar item with no icon at all.
    /// One table here means the same fallback shape everywhere instead of a gap per platform.
    ///
    /// `None` for a variant with no drawing yet — `Symbol` is `#[non_exhaustive]`, so a new one
    /// degrades to a label rather than failing to compile.
    pub fn outline_path(self) -> Option<&'static str> {
        use Symbol as S;
        // The wildcard is unreachable TODAY — every variant is drawn below — and it is here for
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
            S::Open => "M2 5h7l2 2h11v12H2z",
            S::Save => "M3 3h13l5 5v13H3zm5 0h7v6H8zm-1 11h10v7H7z",
            S::Print => "M7 3h10v4H7zM4 8h16v8h-3v6H7v-6H4z",
            S::Refresh => "M12 4V1L8 5l4 4V6a6 6 0 1 1-6 6H4a8 8 0 1 0 8-8z",
            S::Search => {
                "M10 4a6 6 0 1 0 3.5 10.9l4.8 4.8 1.4-1.4-4.8-4.8A6 6 0 0 0 10 4m0 2a4 4 0 1 1 0 8 4 4 0 0 1 0-8"
            }
            S::Share => "M12 3l5 5h-3v7h-4V8H7zM5 17h14v4H5z",
            S::Settings => {
                "M12 2l2 3h3l1 3-2 4 2 4-1 3h-3l-2 3-2-3H7l-1-3 2-4-2-4 1-3h3zm0 6a4 4 0 1 0 0 8 4 4 0 0 0 0-8"
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
            // "Automatic": the half-filled circle every platform uses — the left half solid, the
            // right half an outline, which `evenodd` below turns into the ring it should be.
            S::Auto => "M12 2a10 10 0 0 0 0 20zM12 4a8 8 0 0 1 0 16v-1.6a6.4 6.4 0 0 0 0-12.8z",
            S::ZoomIn => {
                "M10 4a6 6 0 1 0 3.5 10.9l4.8 4.8 1.4-1.4-4.8-4.8A6 6 0 0 0 10 4M9 7h2v2h2v2h-2v2H9v-2H7V9h2z"
            }
            S::ZoomOut => {
                "M10 4a6 6 0 1 0 3.5 10.9l4.8 4.8 1.4-1.4-4.8-4.8A6 6 0 0 0 10 4M7 9h6v2H7z"
            }
            // The ZoomIn lens with a numeral 1 in it — "back to actual size".
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
            S::Folder => "M2 5h7l2 2h11v12H2z",
            S::Document => "M14 2H6v20h12V6zm.5.8L17.2 6H14.5z",
            S::Check => "M9 16.2L4.8 12l-1.4 1.4L9 19 21 7l-1.4-1.4z",
            S::Close => {
                "M19 6.4L17.6 5 12 10.6 6.4 5 5 6.4 10.6 12 5 17.6 6.4 19 12 13.4 17.6 19 19 17.6 13.4 12z"
            }
            S::Warning => "M12 2l10 19H2zm-1 7h2v6h-2zm0 8h2v2h-2z",
            // Shape outlines, stroked as a 2-unit band (an even-odd ring), so they read as
            // OUTLINES at any size rather than filled blocks.
            S::Rectangle => "M3 5h18v14H3zm2 2v10h14V7z",
            S::Oval => {
                "M12 5c5 0 9 3.1 9 7s-4 7-9 7-9-3.1-9-7 4-7 9-7zm0 2c-3.9 0-7 2.2-7 5s3.1 5 7 5 7-2.2 7-5-3.1-5-7-5z"
            }
            S::Line => "M4.7 17.9l13.2-13.2 1.4 1.4L6.1 19.3z",
            // Group: a solid square stacked on another (the visible L of the back one);
            // Ungroup: the same two squares pulled apart.
            S::Group => "M8 4h12v12h-4V8H8zM4 8h12v12H4z",
            S::Ungroup => "M3 3h8v8H3zM13 13h8v8h-8z",
            // `Symbol` is `#[non_exhaustive]`: a variant added upstream draws nothing here rather
            // than failing to compile, and its item keeps its label.
            _ => return None,
        })
    }

    /// [`Self::outline_path`] wrapped as a standalone SVG document.
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
/// text is HTML, where any run of whitespace collapses to one space — which silently ran a
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
fn needs_span(r: &TextRun, dialect: MarkupDialect) -> bool {
    let extras = r.color.is_some() || r.background.is_some() || r.font.scale != 1.0;
    match dialect {
        // Pango expresses the monospace family as a span attribute; Qt cannot (see below).
        MarkupDialect::Pango => extras || r.font.monospace,
        MarkupDialect::QtHtml => extras,
    }
}

fn open_run(r: &TextRun, dialect: MarkupDialect, base_points: f64, out: &mut String) {
    if let Some(url) = r.link.as_deref() {
        out.push_str("<a href=\"");
        escape_markup(url, out);
        out.push_str("\">");
    }
    // The span carries the attributes that have no tag — colors, relative size, and (on Pango)
    // the monospace family. Bold/italic/strike/underline are tags in both dialects, which keeps
    // the attribute string short and the escaping trivial.
    let color = r.color.map(hex);
    let background = r.background.map(hex);
    let mono = r.font.monospace;
    if needs_span(r, dialect) {
        match dialect {
            MarkupDialect::Pango => {
                out.push_str("<span");
                if let Some(c) = &color {
                    out.push_str(&format!(" foreground=\"{c}\""));
                }
                if let Some(c) = &background {
                    out.push_str(&format!(" background=\"{c}\""));
                }
                if mono {
                    out.push_str(" font_family=\"monospace\"");
                }
                if r.font.scale != 1.0 {
                    // ABSOLUTE, in Pango units (1/1024 pt), not the `font_scale`/`size="N%"`
                    // pair: those are Pango 1.50 attributes, and a Pango that does not know an
                    // attribute fails the whole markup parse — which renders the label EMPTY
                    // rather than merely unscaled. A point size still tracks the desktop's text
                    // scaling, since Pango resolves points through the Xft DPI.
                    let pts = r.font.resolved_points(base_points);
                    out.push_str(&format!(" size=\"{}\"", (pts * 1024.0).round() as i64));
                }
                out.push('>');
            }
            MarkupDialect::QtHtml => {
                // Color only. Qt's rich text does NOT resolve the generic `monospace` family
                // from a style attribute — it rendered proportional — so the fixed face comes
                // from the `<code>` tag below, which Qt maps to its own fixed font.
                out.push_str("<span style=\"");
                if let Some(c) = &color {
                    out.push_str(&format!("color:{c};"));
                }
                if let Some(c) = &background {
                    out.push_str(&format!("background-color:{c};"));
                }
                if r.font.scale != 1.0 {
                    // POINTS, not a percentage: Qt's rich-text CSS subset understands `pt` and
                    // `px` and silently ignores `%`, which is why a scaled run rendered at the
                    // base size here while every other backend scaled it.
                    let pt = r.font.resolved_points(base_points);
                    out.push_str(&format!("font-size:{pt}pt;"));
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
    if r.underline.is_on() {
        out.push_str("<u>");
    }
    if r.strikethrough {
        out.push_str("<s>");
    }
}

fn close_run(r: &TextRun, dialect: MarkupDialect, out: &mut String) {
    if r.strikethrough {
        out.push_str("</s>");
    }
    if r.underline.is_on() {
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

pub mod props {
    use super::*;

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ContainerProps {
        pub background: Option<Color>,
        pub corner_radius: f64,
        pub clips: bool,
        /// Semantic, THEME-ADAPTIVE surface — mapped by each backend to a native material that
        /// follows the platform's light/dark appearance automatically (unlike the fixed-RGBA
        /// `background`, which it overrides when set).
        pub role: Option<super::SurfaceRole>,
    }
    /// Reactive surface update for a `background(..)` decorator whose color is a signal/closure:
    /// the backend re-applies the fill on the container's native backing view. Corner radius and
    /// clipping are fixed at realize (the `corner_radius(r)` decorator takes a plain `f64`).
    #[derive(Clone, Debug, PartialEq)]
    pub enum ContainerPatch {
        Background(Option<Color>),
    }

    /// Realize props for a `scroll` container: which axis it scrolls. Backends create the matching
    /// native scroll view (vertical `UIScrollView`/`ScrollView`, horizontal
    /// `HorizontalScrollView`, etc.).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ScrollProps {
        pub horizontal: bool,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LabelProps {
        pub text: String,
        pub font: FontSpec,
        pub color: Option<Color>,
        pub wraps: bool,
        /// How the label's lines sit within its own width. Only observable on a label that
        /// WRAPS or carries explicit newlines — a single line fills its box, so its alignment is
        /// the container's business, not the label's.
        ///
        /// `Leading` is the default and is what running text wants; `Center` is for the short,
        /// deliberately-centered block a welcome screen or an empty state uses. A backend that
        /// cannot set it renders leading-aligned, which is legible and correct — just not
        /// centered.
        pub align: TextAlign,
        /// Styled spans within `text` (docs/text-runs.md). EMPTY is the overwhelmingly common
        /// case and means exactly what a label has always meant: one font, one color. A backend
        /// that cannot draw runs ignores this and renders `text` uniformly, which is legible and
        /// correct — just unstyled.
        pub runs: Vec<crate::TextRun>,
    }
    /// How a wrapped label's lines sit within its width (`LabelProps::align`).
    ///
    /// Deliberately NOT a `Justified` variant: no toolkit in Day's set agrees on how to justify
    /// the last line, and the ones that can do it at all need a paragraph style the others have
    /// no equivalent for.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum TextAlign {
        /// Left in a left-to-right locale, right in an RTL one (docs/localization.md).
        #[default]
        Leading,
        Center,
        Trailing,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum LabelPatch {
        Text(String),
        Color(Option<Color>),
        Font(FontSpec),
        /// New text AND its runs together. One patch, not two: a run's byte range is only
        /// meaningful against a particular string, so applying them separately would leave the
        /// widget briefly holding ranges that point into the wrong text.
        Runs(String, Vec<crate::TextRun>),
    }

    /// A button's NATIVE styling tier. `Automatic` is the toolkit's stock look; `Bordered`
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
        /// A filled button in an APP-CHOSEN color, still drawn by the native control: the
        /// platform keeps its own pressed, hover, focus and disabled rendering, its button role,
        /// and its keyboard activation. The label color is not carried — each backend picks the
        /// readable one for the fill (see [`ButtonStyleSpec::on_tint`]), the same call the
        /// platform's own tinted buttons make.
        ///
        /// A backend with no way to recolor its button ignores this and draws its ordinary
        /// button. That is the deliberate trade: a plain button everywhere beats a colored
        /// rectangle that is no longer a button on the platforms that cannot.
        Tinted(Color),
    }

    impl ButtonStyleSpec {
        /// The label color to draw on a [`ButtonStyleSpec::Tinted`] fill: whichever of black or
        /// white CONTRASTS BETTER against it, by WCAG's contrast ratio.
        ///
        /// Comparing the two ratios rather than thresholding the luminance is the difference
        /// between right and nearly right. A mid amber (`#F0A64C`) has a relative luminance of
        /// 0.44 — under a "brighter than half" test it reads as dark and takes white text, at a
        /// hopeless 2.2:1. Its contrast against BLACK is 9.7:1. The two ratios cross at a
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
    pub struct ButtonProps {
        pub title: String,
        pub enabled: bool,
        pub style: ButtonStyleSpec,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum ButtonPatch {
        Title(String),
        Enabled(bool),
        /// A live style change — what a reactive `.tint(…)` sends, so a button can recolor with
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

    /// SwiftUI's `pickerStyle` analogue (kinds::PICKER). Each maps to a distinct native
    /// control per toolkit (docs/picker.md).
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum PickerStyle {
        /// A dropdown/pop-up menu (NSPopUpButton / GtkDropDown / QComboBox / UIButton+UIMenu / Spinner).
        #[default]
        Menu,
        /// A horizontal segmented control (NSSegmentedControl / UISegmentedControl / linked toggles / …).
        Segmented,
        /// A vertical radio-button group laid out inline (NSButton radios / GtkCheckButton group / …).
        Inline,
    }

    /// Full picker props (realize). `style` is set once at build; `options` and `selected`
    /// patch (via [`PickerPatch`]).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct PickerProps {
        pub options: Vec<String>,
        pub selected: usize,
        pub style: PickerStyle,
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum PickerPatch {
        Selected(usize),
        /// New option labels — a picker whose choices come from data (a document list, a
        /// live count). The backend rebuilds its items and keeps the selected INDEX where it
        /// still exists, clamping to the last option otherwise; a fresh
        /// [`PickerPatch::Selected`] follows whenever the app's own binding disagrees.
        Options(Vec<String>),
    }

    /// Full text-area props (realize, kinds::TEXT_AREA — docs/textarea.md). `text` seeds the
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
    }

    /// How an image is scaled to fill its frame (§18.3). Maps to each toolkit's native scaling.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum ContentMode {
        /// Scale to fit entirely inside the frame, preserving aspect ratio (letterboxed). The
        /// default — an image never stretches unless asked. SwiftUI's `.scaledToFit`.
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
        /// Resolved asset path or name; backend loads through its image pipeline (§18.2).
        pub source: String,
        pub decorative: bool,
        /// How the image scales within its frame (default [`ContentMode::Fit`] — no stretching).
        pub content_mode: ContentMode,
        /// Optional width:height ratio the view is constrained to (e.g. `16.0/9.0`). `None` lets the
        /// image take its allocated frame.
        pub aspect_ratio: Option<f64>,
        /// Monochrome tint for a template/vector glyph (docs/vectors.md): backends that can,
        /// recolor natively (template rendering on Apple, drawable tint on Android, pixel
        /// recolor on GTK); backends that can't yet ignore it and draw the source colors.
        /// `None` (the default, and every raster `image(…)`) means "as authored".
        pub tint: Option<Color>,
    }

    /// A live change to a realized image or vector glyph.
    ///
    /// Only the tint: `source`, `content_mode` and the rest describe which art this is and how it
    /// fills its frame, which a rebuild expresses better than a patch. The tint is the one that
    /// wants to follow a signal — a glyph that recolors with the selection or the theme should
    /// repaint, not be torn down and realized again (docs/vectors.md "Tint").
    #[derive(Clone, Debug, PartialEq)]
    pub enum ImagePatch {
        /// Recolor the glyph, or `None` to draw the authored colors again.
        Tint(Option<Color>),
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct CanvasProps {
        pub ops: Vec<DrawOp>,
    }

    /// Progress indicator. `value` is the completed fraction in `0.0..=1.0`; `None` means
    /// indeterminate (an animated spinner / busy bar — no known extent). Backends map this to
    /// their native determinate/indeterminate widgets (docs/progress.md).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ProgressProps {
        pub value: Option<f64>,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum ProgressPatch {
        /// New completed fraction, or `None` to switch to indeterminate.
        Value(Option<f64>),
    }

    /// A trailing action button on the navigation bar (docs/navigation.md) — the phones' and
    /// HarmonyOS's stand-in for a desktop toolbar button, since those toolkits have no window
    /// toolbar (`Cap::Toolbar` is `Unsupported`). Rendered upper-right on the nav bar by the
    /// mobile backends (iOS `rightBarButtonItem`, Android/HarmonyOS a menu action); ignored by
    /// desktop split presentations, which carry their commands in a real toolbar instead.
    ///
    /// `action` is a menu-action dispatch id (`register_menu_action`): the backend emits
    /// `Event::MenuAction(action)` when the button is tapped, and the tree runs the registered
    /// closure. `icon` is a bundled image name (resolved via [`resolve_image_file`], the same
    /// convention as [`NavMenuProps::icons`]); `label` is its accessible name and tooltip.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavBarAction {
        pub action: u64,
        pub label: String,
        pub icon: Option<String>,
        /// Which of the host's pages carry this button.
        pub scope: NavBarScope,
    }

    /// Which pages a [`NavBarAction`] rides (docs/navigation.md).
    ///
    /// A nav bar is shared: the same bar draws the list and then every detail pushed on top of
    /// it. Whether a command belongs on all of them depends on what it acts ON, which only the
    /// app knows — "show this page's source" follows the user down, "add an item" does not,
    /// because on a detail page there is no list in front of them to add to.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum NavBarScope {
        /// Every page, root and pushed alike — a command that acts on WHATEVER IS SHOWING.
        #[default]
        EveryPage,
        /// The root page only — a command that acts on the LIST rather than on a detail opened
        /// from it. Where a split presentation keeps the list in its own pane, that pane's bar is
        /// the root page's bar, so the button stays put while details come and go. This is where
        /// inline search already lives (docs/search.md), for the same reason.
        RootPage,
    }

    /// How a navigation host lays its panes out (docs/navigation.md).
    ///
    /// This is the RESOLVED presentation — what the toolkit must draw right now. The app asks for
    /// one through `Selector::presentation`, where leaving it unset means "automatic": the pieces
    /// layer resolves it from the window's [`SizeClass`] and the toolkit's `Cap::NavSplit`, and
    /// re-resolves it whenever the class changes. There is deliberately no `Auto` variant here —
    /// a backend can only ever draw a concrete one, so the undecided state stays on the app's side
    /// of the boundary where it can still be decided.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum NavPresentation {
        /// Sidebar beside detail, both visible (`NSSplitViewController`, `AdwOverlaySplitView`,
        /// a `NavigationView`). The sidebar pane gets its own container.
        Split,
        /// One page at a time, back-navigable. The sidebar pane becomes the stack's root.
        #[default]
        Stack,
        /// The rows drawn as a TAB BAR, one page's content beside it, no back stack — what a
        /// phone-shaped window wants from a one-of-N surface (`UITabBarController`, a Material
        /// `NavigationBarView`, `NavigationView.PaneDisplayMode = Top`). The `Pane::Sidebar` page
        /// is not drawn at all: its rows ARE the chrome.
        Tabs,
        /// The rows drawn as a narrow icon strip beside the content — the `Medium`-width answer
        /// between [`Self::Tabs`] and [`Self::Split`] (a Material `NavigationRailView`,
        /// `PaneDisplayMode = LeftCompact`, an ArkUI vertical `Tabs`).
        ///
        /// **A backend with no rail ROUNDS IT to a neighbor of its own choosing** rather than
        /// failing — UIKit answers `Tabs` (which is what iPadOS does at that width), GTK the same.
        /// Rounding is expected and correct; this variant is a request, not a contract.
        Rail,
    }

    impl NavPresentation {
        /// `true` for [`NavPresentation::Split`] — the shape most backend code wants.
        pub fn is_split(self) -> bool {
            self == NavPresentation::Split
        }

        /// `true` when the rows are the CHROME rather than a drawn page — [`Self::Tabs`] and
        /// [`Self::Rail`]. Both hide the `Pane::Sidebar` page and render its rows themselves, so
        /// backend code that only cares about that distinction asks this instead of matching both.
        pub fn rows_are_chrome(self) -> bool {
            matches!(self, NavPresentation::Tabs | NavPresentation::Rail)
        }
    }

    /// Navigation host (docs/navigation.md).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavProps {
        pub title: String,
        /// The presentation to draw now; re-presented in place by [`NavPatch::Presentation`].
        pub presentation: NavPresentation,
        /// The app left the presentation AUTOMATIC (`SelectorStyle::Automatic`, or a `Sidebar`
        /// selector with no `.presentation(…)` pin), so it is free to follow the window.
        ///
        /// Only a toolkit answering `Cap::NavRepresent = Emulated` needs this: its own adaptive
        /// container owns the morph, and it must know whether the app asked for one at all before
        /// building it. With two presentations that bit could be inferred from a lowered `Split`;
        /// with four it cannot, so it is carried explicitly rather than encoded.
        ///
        /// `Native` re-presenters ignore it — they are told each presentation as it is resolved —
        /// and so do toolkits that cannot re-present.
        pub adaptive: bool,
        /// Trailing bar-button commands for the mobile nav bar (see [`NavBarAction`]), in the
        /// order the app declared them; empty on desktop, where the toolbar carries commands
        /// instead. A backend that can only draw one shows the first.
        pub bar_actions: Vec<NavBarAction>,
        /// Search over this navigation surface (`Selector::searchable`, docs/search.md).
        /// `None` = the surface is not searchable and no field is rendered anywhere.
        pub search: Option<SearchProps>,
        /// `Some(width)` = this host has a CONTENT-LIST pane ([`Pane::List`]) at this preferred
        /// width in points, between the sidebar and the detail (docs/navigation.md). Read at
        /// realize — a backend that builds a different container for a three-pane host (UIKit's
        /// triple-column style is init-only) needs to know before it builds. `None` on every
        /// host without one, including all of them on backends answering
        /// [`crate::Cap::NavContentList`] `Unsupported` — the pieces layer composes there and
        /// the prop is never set.
        pub list_width: Option<f64>,
    }

    /// Where a searchable surface's field should be drawn.
    ///
    /// A PREFERENCE, not an instruction: a backend that cannot honor the request falls back to
    /// whatever its platform does, exactly as SwiftUI's `searchable(placement:)` does ("depending
    /// on the containing view hierarchy and platform, the requested placement may not be able to
    /// be fulfilled"). `Automatic` is the one to reach for — it is what lets the field live in the
    /// window toolbar on a wide window and move into the navigation list on a narrow one.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum SearchPlacement {
        /// The platform decides, from the window's size class and its own convention.
        #[default]
        Automatic,
        /// In the window's toolbar (`NSSearchToolbarItem`, an `AdwHeaderBar` entry, a
        /// `CommandBar` `AutoSuggestBox`). Ignored where there is no toolbar.
        Toolbar,
        /// Attached to the navigation surface itself: above the sidebar list, or in the
        /// navigation bar's search drawer on the phones.
        Inline,
    }

    /// Search over a navigation surface (docs/search.md). The QUERY itself stays an app-owned
    /// signal — this carries only what the toolkit must draw, so moving the field between
    /// placements never moves the state.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct SearchProps {
        /// Current text. Two-way: the user typing emits [`Event::SearchChanged`], and the app's
        /// own writes arrive as [`SearchPatch::Text`].
        pub text: String,
        /// Empty-state prompt.
        pub prompt: String,
        pub placement: SearchPlacement,
        /// Scope titles, empty = no scope bar. One-of-N: a `UISearchBar` scope bar, a Material
        /// `ChipGroup` of single-selection filter chips, an ArkUI `SegmentButtonV2`, an
        /// `NSSegmentedControl` (docs/search.md has the per-backend table).
        pub scopes: Vec<String>,
        /// Index into `scopes`; meaningless when `scopes` is empty.
        pub scope: usize,
        /// Completions offered for the current text. On a navigation surface these COMPLETE THE
        /// FIELD rather than replacing the list: the list is already the result set, so an
        /// overlay of results would cover the thing it is filtering.
        pub suggestions: Vec<String>,
    }

    /// A targeted update to a live search field — the path a bound signal writes through, so
    /// syncing text never rebuilds (and refocuses) the field mid-word.
    #[derive(Clone, Debug, PartialEq)]
    pub enum SearchPatch {
        Text(String),
        Scope(usize),
        Suggestions(Vec<String>),
    }
    /// Applied to the NAV HOST after a page child is attached / before it is removed;
    /// the toolkit animates its native presentation accordingly.
    #[derive(Clone, Debug, PartialEq)]
    pub enum NavPatch {
        /// The just-attached last page child became the top of the stack. `immersive` marks a
        /// page that keeps the floating transparent chrome on backends with an immersive nav
        /// mode (day-android edge-to-edge today; ignored elsewhere) — unmarked pages get the
        /// standard opaque bar.
        Pushed { title: String, immersive: bool },
        /// The top page is about to be removed; present its predecessor.
        Popped,
        /// Current top-of-stack title changed.
        Title(String),
        /// The top page has a back guard (`Stack::on_back`, docs/navigation.md): while `true`,
        /// the toolkit must NOT auto-pop on a native back gesture/button — instead route it to
        /// Day as `Event::NavBack { already_popped: false }` so the app's guard decides. On
        /// backends whose back already routes through Day (AppKit/Qt/XAML/web custom headers)
        /// this is a no-op; iOS disables the swipe, Android holds its callback, GTK sets the
        /// page `can-pop=false`, ArkUI consumes `onBackPressed`.
        GuardTop(bool),
        /// Re-present the host: the window's size class crossed a breakpoint, or the app wrote a
        /// new `presentation`. The page children do NOT change — the toolkit rebuilds its own
        /// chrome and RE-HOMES the pages it already has, each one landing by its
        /// [`NavPageProps::pane`]. Rebuilding them instead would drop scroll offsets, field
        /// focus, and every native animation in flight, which is the whole reason this is a
        /// patch and not a rebuild.
        ///
        /// Backends with no split container ignore it and stay stacked.
        Presentation(NavPresentation),
        /// Show the resident [`Pane::Detail`] page at this index, counting the host's detail
        /// children in attach order. The counterpart to [`Self::Pushed`]/[`Self::Popped`] for the
        /// presentations where pages are RESIDENT rather than stacked
        /// ([`NavPresentation::rows_are_chrome`]): a tab bar switches between pages that all stay
        /// alive, so there is nothing to push and nothing to pop.
        ///
        /// Applied WITHOUT re-emitting [`crate::Event::SelectionChanged`], per the from-native
        /// echo rule — this is the programmatic-sync direction.
        ///
        /// A backend drawing a stacked presentation never receives it; the pieces layer sends
        /// push/pop there instead.
        Select(usize),
        /// Show or collapse the CONTENT-LIST pane ([`Pane::List`]) — the per-destination
        /// visibility switch (`Selector::content_list_for`, docs/navigation.md): a selector
        /// section that spans the whole detail area (a settings page) collapses the pane, and
        /// selecting a list-backed section brings it back. Only sent to hosts whose
        /// [`NavProps::list_width`] is set. Applied directly, not through an animator proxy —
        /// the screenshot seam captures the instant the patch returns (the sidebar-toggle rule).
        ListVisible(bool),
        /// STACKED presentations on a merged-pane backend ([`crate::Cap::NavContentList`]
        /// `Emulated`) only: the resident [`Pane::List`] page joins (`true`) or leaves
        /// (`false`) the navigation stack directly above the sidebar root. The pieces layer
        /// sends it as the selection gains or loses a list-backed destination; the backend
        /// realizes the membership change through its platform's OWN navigation APIs (UIKit:
        /// `showColumn(.supplementary)` / a pop to the root — never a manual splice, which
        /// destroys the split's collapse bookkeeping). Never sent to `Native` backends (their
        /// pane persists through every presentation) nor while split.
        ListInStack(bool),
    }

    /// Which pane of a navigation host a page belongs to (docs/navigation.md).
    ///
    /// A page's pane is a fact about the MODEL, not about how the host currently draws it: a
    /// selector's list page is [`Pane::Sidebar`] whether the toolkit is showing a sidebar beside
    /// a detail or stacking the two. What the presentation decides is where each pane lands —
    /// its own splitter pane in [`NavPresentation::Split`], the root of the stack in
    /// [`NavPresentation::Stack`]. Keeping the two separate is what lets a host RE-PRESENT on a
    /// size-class change (`NavPatch::Presentation`) by re-homing the pages it already has,
    /// instead of tearing the tree down and rebuilding it.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum Pane {
        /// The list / master pane of a selector. At most one page per host.
        Sidebar,
        /// The content-list pane between the sidebar and the detail — Mail's message list
        /// (`Selector::content_list`, docs/navigation.md). At most one page per host, and only
        /// on hosts whose [`NavProps::list_width`] is set; a backend answering
        /// [`crate::Cap::NavContentList`] `Unsupported` never sees one.
        List,
        /// A destination page. Every page of a `stack` presentation is one of these.
        #[default]
        Detail,
    }

    /// One destination's native container.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavPageProps {
        pub title: String,
        pub pane: Pane,
    }

    /// A fullscreen cover's content container (docs/cover.md). Realized detached and hidden;
    /// `CoverPatch::Present` shows it over the whole window.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct CoverProps {}

    /// Applied to a `kinds::COVER` node as its bound signal opens and closes it.
    #[derive(Clone, Debug, PartialEq)]
    pub enum CoverPatch {
        /// Present the cover fullscreen (slide up where the platform animates modals).
        /// `background` paints the whole surface edge-to-edge (under the status bar / home
        /// indicator); the content container is inset to the safe area within it.
        /// `dismiss_disabled` = the user must not be able to dismiss it interactively
        /// (system back / sheet gestures); programmatic dismissal still works.
        Present {
            background: Option<Color>,
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
    /// pane. The pane's visibility is Day-owned state — the piece's bound signal — so the
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
        /// programmatic-sync direction: applying it must NOT re-emit
        /// [`crate::Event::InspectorChanged`], per the from-native echo rule.
        Visible(bool),
    }

    /// One pane of a `kinds::INSPECTOR` split (docs/inspector.md).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct InspectorPaneProps {
        /// `true` for the inspector panel itself, `false` for the content pane.
        pub panel: bool,
    }

    /// Native navigation item list. `items` are display titles in route order;
    /// `selected` highlights the active route (split presentation; None on mobile roots).
    /// `icons` (parallel to `items`, `None` = no icon) are BUNDLED IMAGE NAMES resolved by each
    /// backend via `resource::resolve_image_file` — a backend that can't decorate its rows just
    /// ignores them.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavMenuProps {
        pub items: Vec<String>,
        pub icons: Vec<Option<String>>,
        /// A trailing accessory per row — an unread count, a status. Rendered right-aligned and
        /// de-emphasized, opposite the label. Parallel to `items`; `None` draws nothing.
        pub badges: Vec<Option<String>>,
        /// A trailing SYMBOL accessory per row, in the same slot as [`Self::badges`] and drawn
        /// after it: a BUNDLED IMAGE NAME resolved the way [`Self::icons`] is. This is what a
        /// row-level status GLYPH rides on — a starred page's star — where `badges` carries only
        /// text. Parallel to `items`; `None` draws nothing.
        pub badge_icons: Vec<Option<String>>,
        /// The tint for [`Self::badge_icons`], parallel to `items`. `None` leaves the glyph at
        /// the backend's neutral template tint; a status glyph that means something (a yellow
        /// star) names its own color here, exactly as [`Self::tints`] does for the leading icon.
        pub badge_tints: Vec<Option<Color>>,
        /// A per-row icon tint (docs/vectors.md): the row's glyph recolored to this instead of
        /// the backend's neutral template tint. Parallel to `items`; `None` keeps the default.
        pub tints: Vec<Option<Color>>,
        /// A section header introducing the row at the same index. `Some` opens a new group
        /// before that row; `None` continues the current one. Parallel to `items`, so adding a
        /// header never shifts the selection indices the rows are addressed by.
        pub sections: Vec<Option<String>>,
        /// A per-row context menu (docs/menus.md): shown on secondary-click / long-press on
        /// that row, the same [`crate::MenuItem`] model as everywhere else — items carry
        /// registered action ids, so a chosen entry dispatches [`crate::Event::MenuAction`]
        /// exactly like a piece context menu. Parallel to `items`; empty = no menu.
        pub menus: Vec<Vec<crate::MenuItem>>,
        pub selected: Option<usize>,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum NavMenuPatch {
        /// Programmatic highlight sync — toolkits apply WITHOUT re-emitting
        /// SelectionChanged (the TextField from_native echo rule).
        Selected(Option<usize>),
        /// The item set changed (data-driven `selector().items(signal, …)`): rebuild the rows
        /// from these labels/icons, then apply `selected` (docs/navigation.md). Applied WITHOUT
        /// re-emitting SelectionChanged.
        Items {
            items: Vec<String>,
            icons: Vec<Option<String>>,
            badges: Vec<Option<String>>,
            badge_icons: Vec<Option<String>>,
            badge_tints: Vec<Option<Color>>,
            sections: Vec<Option<String>>,
            tints: Vec<Option<Color>>,
            menus: Vec<Vec<crate::MenuItem>>,
            selected: Option<usize>,
        },
    }

    /// How a recycling list sizes its rows (docs/list.md).
    #[derive(Clone, Copy, Debug, PartialEq, Default)]
    pub enum RowHeight {
        /// Every row is this tall — a true layout boundary; the fastest path.
        Uniform(f64),
        /// Rows self-size from their content (host re-measures on change; slower).
        #[default]
        Automatic,
    }

    /// Native recycling list (docs/list.md). The host owns scrolling + cell reuse; Day supplies
    /// row content on demand through the injected `ListSource` (see `Toolkit::attach_list`).
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ListProps {
        pub row_height: RowHeight,
        /// Whether the native list reports row selection (`Event::SelectionChanged` with the row).
        pub selectable: bool,
        /// Whether the native list allows selecting several rows at once. Where honored
        /// (docs/list.md has the matrix), every selection change reports the FULL set via
        /// `Event::SelectionSet`; single-selection backends keep reporting
        /// `Event::SelectionChanged` and treat this as `selectable`.
        pub multi_select: bool,
        /// Whether rows can be drag-reordered. The toolkit enables its native drag machinery and
        /// drives the `ListSource::reorder` seam (validate via `can_move`, commit via `move_row`);
        /// probe `Cap::ListReorder` for per-backend support (docs/list.md).
        pub reorderable: bool,
        /// Whether rows can be deleted by the platform's own delete gesture — a trailing swipe on
        /// iOS and ArkUI, an `ItemTouchHelper` swipe on Android. The toolkit drives the
        /// `ListSource::delete` seam (offer via `can_delete`, commit via `delete_row`); probe
        /// `Cap::ListDelete` for per-backend support (docs/list.md). Desktop toolkits have no such
        /// gesture and answer `Unsupported`; an app that must delete everywhere pairs this with an
        /// explicit control (a menu item, a button) rather than relying on the swipe.
        pub deletable: bool,
        /// The label on that affordance, ALREADY LOCALIZED by the app. A toolkit cannot invent
        /// translated text — it has no access to the app's catalog — so the string arrives with
        /// the props. Empty means "no text": each backend falls back to its platform's own
        /// wordless idiom (a trash glyph), which is honest in every language.
        pub delete_label: String,
        /// The rows offer edge swipe ACTIONS (docs/list.md) through the injected
        /// `ListSource::swipe` seam (offer via `actions_at`, commit via `perform`); probe
        /// [`crate::Cap::ListSwipeActions`] for per-backend support. A backend without the
        /// affordance simply never shows one — an app that must offer the command everywhere
        /// pairs the swipe with an explicit control, exactly as `deletable` documents.
        pub swipe_actions: bool,
        /// Row separators, drawn by the HOST at the row boundary (docs/list.md) — so they
        /// align with the native selection and ride the platform's own row animations (a
        /// macOS/iOS swipe slides the row content past a stationary separator, exactly as
        /// Mail's does). `None` keeps each platform's own default (iOS draws them, the
        /// desktops don't); `Some(true)`/`Some(false)` forces. A backend without a separator
        /// mechanism ignores a force — the matrix in docs/list.md says which; rows there
        /// separate by their pitch alone.
        pub separators: Option<bool>,
    }

    /// One row-level result-set change. Indexes are SEQUENTIAL — each delta describes the
    /// set as the previous ones left it — so a host applies them one at a time (or falls back
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
        pub row_height: RowHeight,
        /// Rows highlight and report selection ([`crate::Event::TreeSelection`]).
        pub selectable: bool,
        /// Several rows may be selected at once; every change reports the FULL token set.
        pub multi_select: bool,
        /// Rows drag to a new parent/position through the platform's own mechanism, driving
        /// the `TreeSource::moves` seam. Probe [`crate::Cap::TreeMove`] for support.
        pub movable: bool,
        /// Indentation per depth level, in points. `None` = the platform's default step.
        pub indent: Option<f64>,
    }

    /// Post-realize changes to a `kinds::TREE` host (docs/tree.md). Token-addressed
    /// throughout — see [`crate::TreeSource`] for why indices cannot address tree rows.
    #[derive(Clone, Debug, PartialEq)]
    pub enum TreePatch {
        /// The node set changed (count/order/parentage/content): the host re-queries its
        /// `TreeSource`. Expansion and selection survive by token (docs/tree.md).
        Reload,
        /// Programmatically disclose (`true`) or collapse (`false`) one row. Applied WITHOUT
        /// re-emitting [`crate::Event::TreeExpanded`] (the from-native echo rule).
        Expand(u64, bool),
        /// Programmatic selection sync (tokens; empty = clear) — applied WITHOUT re-emitting
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
        /// The row set changed by exactly these deltas — a host that can, animates each row
        /// in, out, or across; one that cannot treats this as [`ListPatch::Reload`]. The
        /// data source answers with the FINAL state throughout.
        Splice(Vec<RowDelta>),
        /// An `Automatic`-height row's content size changed; the host re-measures just that row.
        RowSizeInvalidated(usize),
        /// Imperatively scroll the native list so its LAST row is fully visible (a chat timeline
        /// sticking to the newest message). No-op when the list is empty (docs/list.md).
        ScrollToEnd,
        /// Imperatively scroll the native list so this row is visible, realizing it if needed
        /// (docs/list.md). Clamped to the row count; no-op when the list is empty.
        ScrollToRow(usize),
        /// Programmatic selection sync (row indices; empty = clear) — toolkits apply WITHOUT
        /// re-emitting a selection event, like every other programmatic sync.
        Selected(Vec<usize>),
    }
}

// ---------------------------------------------------------------------------
// Imperative presentation (docs/dialogs.md)
// ---------------------------------------------------------------------------

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
        /// `PresentResult::Files` whose entries are **readable local paths** — desktop returns
        /// the chosen path directly; iOS/Android copy the selection into app storage first, so
        /// the pieces layer can read it with `std::fs` regardless of platform.
        OpenFile {
            title: String,
            filters: Vec<FileFilter>,
        },
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

    /// The unit-separator that joins string lists across the C ABI (Qt shim / Android JNI) — the
    /// same encoding the nav menu, combobox, and dialog-button shims use.
    pub const UNIT_SEP: char = '\u{1f}';

    std::thread_local! {
        /// An app-writable scratch directory. Backends whose OS temp dir isn't app-writable
        /// (Android → `getCacheDir()`) set this at startup; elsewhere it stays `None` and callers
        /// fall back to `std::env::temp_dir()`.
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
    pub fn app_temp_dir() -> std::path::PathBuf {
        APP_TEMP_DIR.with(|d| {
            d.borrow().clone().unwrap_or_else(|| {
                #[cfg(target_arch = "wasm32")]
                {
                    // A browser has no filesystem and std's `temp_dir()` PANICS on wasm. A
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
    /// so the open/save flows carry bytes through this per-page store instead of `std::fs` —
    /// day-dom writes a picked file's bytes here and answers with its virtual path; the pieces
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
        pub fn title(&self) -> &str {
            match self {
                PresentSpec::Dialog { title, .. }
                | PresentSpec::Prompt { title, .. }
                | PresentSpec::OpenFile { title, .. }
                | PresentSpec::SaveFile { title, .. } => title,
            }
        }
        pub fn message(&self) -> Option<&str> {
            match self {
                PresentSpec::Dialog { message, .. } | PresentSpec::Prompt { message, .. } => {
                    message.as_deref()
                }
                _ => None,
            }
        }
        /// Button labels joined with [`UNIT_SEP`] — the encoding the nav menu and combobox
        /// shims already use for string lists.
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
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(",")
        }

        // --- file-dialog accessors (docs/files.md) ---

        /// The file filters, if this is a file dialog.
        pub fn filters(&self) -> &[FileFilter] {
            match self {
                PresentSpec::OpenFile { filters, .. } | PresentSpec::SaveFile { filters, .. } => {
                    filters
                }
                _ => &[],
            }
        }
        /// The suggested file name for a save dialog (empty otherwise).
        pub fn suggested_name(&self) -> &str {
            match self {
                PresentSpec::SaveFile { suggested_name, .. } => suggested_name,
                _ => "",
            }
        }
        /// The Day-written temp source path for a save dialog (empty otherwise).
        pub fn src_path(&self) -> &str {
            match self {
                PresentSpec::SaveFile { src_path, .. } => src_path,
                _ => "",
            }
        }
        /// Filters flattened for the C ABI: each filter is `name|ext1,ext2`, joined by
        /// [`UNIT_SEP`]. A trailing `|` (no extensions) means "all files". Empty when unfiltered.
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

pub trait Toolkit: Sized + 'static {
    // `'static` so a handle CLONE can cross the object-safe TreeOps seam boxed as `Any`
    // (`node_handle_any` — the tweaks door, docs/tweaks.md).
    type Handle: Clone + 'static;

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
    /// registry has nothing to dispatch, and one that has not wired this yet simply leaves piece
    /// teardown unrun rather than failing to build.
    fn release_piece(&mut self, _kind: PieceKind, _h: &Self::Handle) {}

    // tree
    fn insert(&mut self, parent: &Self::Handle, child: &Self::Handle, index: usize);
    fn remove(&mut self, parent: &Self::Handle, child: &Self::Handle);
    fn move_child(&mut self, parent: &Self::Handle, child: &Self::Handle, to: usize);

    // geometry (§7): frames are in the nearest realized native ancestor's space, in points.
    fn measure(&mut self, h: &Self::Handle, kind: PieceKind, p: Proposal) -> Size;
    /// Distance from the top of the widget's frame to its FIRST text baseline, in points, when
    /// the widget is `size` (docs/baseline.md). `None` ⇒ it has no text baseline — an image, a
    /// slider, a bare container — and a baseline-aligned row falls back to box alignment for it.
    ///
    /// This is a MEASUREMENT, not a layout mode: day places every frame itself (§7.1) and never
    /// hands a row to a native baseline-aligning container, so what it needs from the toolkit is
    /// where the text sits inside the box. Backends that cannot answer keep the default and rows
    /// stay centered, which is what every row did before this existed.
    fn first_baseline(&mut self, _h: &Self::Handle, _kind: PieceKind, _size: Size) -> Option<f64> {
        None
    }
    fn set_frame(&mut self, h: &Self::Handle, frame: Rect, anim: Option<&AnimSpec>);

    // animatable visual channels (§8.4): cheap per-node opacity + transform that DON'T relayout.
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
    // once from the `.selectable()` modifier (day-pieces) to the widget it wraps — a `label`'s
    // native text view, most usefully. UNMANAGED: Day sets it here and never patches it, so it
    // survives text updates. The default no-op means a backend without a selection affordance
    // silently leaves the text unselectable.
    //
    // Returns `Some(replacement)` when the toolkit had to REBUILD the widget as a different
    // native class to gain a selection affordance (UIKit: `UILabel` has none, so the label
    // becomes a read-only `UITextView`); day-core re-points the node's handle at the
    // replacement, so later patches and layout reach the widget that is actually on screen.
    // Every property-flip backend returns `None`.
    fn set_selectable(&mut self, _h: &Self::Handle, _selectable: bool) -> Option<Self::Handle> {
        None
    }

    // scroll (§7.6)
    fn set_scroll_content(&mut self, _h: &Self::Handle, _content: Size) {}
    fn scroll_to(&mut self, _h: &Self::Handle, _target: Rect, _animated: bool) {}
    fn scroll_offset(&mut self, _h: &Self::Handle) -> Point {
        Point::ZERO
    }

    // events: one trampoline, node-id keyed; ENQUEUE-ONLY contract (§8.3).
    fn set_event_sink(&mut self, sink: EventSink);

    // gestures (docs/shapes.md): attach a native recognizer for `kind` to `h`, emitting
    // `Event::Tap/LongPress/Drag` for `node` (enqueue-only). Default no gesture; a piece opts in
    // when it has a handler. Idempotent per (handle, kind).
    fn enable_gesture(&mut self, _h: &Self::Handle, _node: NodeId, _kind: GestureKind) {}

    // focus (docs/focus.md): move native keyboard focus to (or away from) this control.
    // `focused = true` requests focus (on mobile this also raises the soft keyboard for text
    // inputs); `false` resigns it (dismissing the keyboard; platforms without a "focus nothing"
    // state resign to a focusable root). Backends report the RESULTING state — user- or
    // programmatic — with `Event::FocusChanged(bool)` through the sink; a request that cannot
    // be honored (unfocusable, unmounted) simply produces no event. The default no-op means a
    // backend without focus support neither moves nor reports focus.
    fn focus(&mut self, _h: &Self::Handle, _node: NodeId, _focused: bool) {}

    // focusability (docs/focus.md): opt this CONTAINER into the platform's focus system — the
    // canvas contract generalized (`Decorate::focusable`). A focusable container accepts focus,
    // takes it on a press (before the press reaches its gesture recognizers), reports both
    // directions with `Event::FocusChanged`, and delivers the non-text keys it hears while
    // focused as `Event::Key` for `node` (gated on `keys::handled`, like a canvas). The default
    // no-op means the piece renders normally but never joins the focus order — `.focused(…)`
    // bindings stay quiet and `.on_key(…)` never fires, the same graceful silence unfocusable
    // controls already have. Idempotent; only `true` is ever sent today (a piece opts in at
    // build, it does not opt back out).
    fn set_focusable(&mut self, _h: &Self::Handle, _node: NodeId, _focusable: bool) {}

    // recycling list (docs/list.md, §10): day-core hands the `LIST` host its row-pull `source`
    // once, right after realize. A recycling backend stores it and calls it from its native
    // data-source; the default no-op means a backend without list support simply renders nothing.
    fn attach_list(&mut self, _host: &Self::Handle, _source: ListSource) {}

    // hierarchical tree (docs/tree.md): day-core hands the `TREE` host its token-addressed
    // row-pull `source` once, right after realize. A backend with a native tree stores it and
    // answers its data-source from it; the default no-op means a backend without tree support
    // renders nothing (`Cap::Tree` answers `Unsupported`, and apps gate on that).
    fn attach_tree(&mut self, _host: &Self::Handle, _source: TreeSource) {}

    // routes (docs/navigation.md): day-core reports the app's CURRENT route path here whenever
    // it changes ("" = everything at its root), so a backend with a native notion of location
    // can mirror it — web-dom writes the URL hash (`#controls`), and browser back/forward or a
    // hand-edited hash comes back as `Event::RouteRequested`. Default no-op: most toolkits
    // have nowhere to put a route.
    fn set_route(&mut self, _route: &str) {}

    // undo bridge (docs/model.md): mirror the app's undo stack into the platform's own undo
    // objects, so the stock Edit menu retitles and enables itself and the platform's gestures
    // land — the NATIVE FRONT of one Day-owned history. The state flows down whenever it
    // changes; the user's invocation comes back up as `Event::Undo`. Default no-op: a backend
    // without a native undo system answers `Cap::UndoBridge` `Unsupported`, and the app's own
    // affordances (menu items, buttons, accelerators) drive the stack instead.
    fn set_undo_state(&mut self, _state: &UndoState) {}

    // edit bridge (docs/menus.md): mirror what the app's standard-edit handlers can do, so
    // the platform's own menu validation enables Cut/Copy/Paste exactly as it does for text
    // widgets; invocations come back up as `Event::Edit`. Default no-op — a backend without
    // a native edit-command route answers `Cap::EditBridge` accordingly.
    fn set_edit_state(&mut self, _state: &EditState) {}

    // ambient modifiers (docs/menus.md): the keyboard modifiers held RIGHT NOW, for
    // interactions whose meaning they change (shift-click multi-select). Pull-based — the
    // platforms expose exactly this query (NSEvent.modifierFlags, the shim's tracked mask).
    // Touch-only backends keep the all-false default.
    fn modifiers(&mut self) -> Modifiers {
        Modifiers::default()
    }

    // Dynamic context menu (docs/menus.md): the menu is built AT SUMMON TIME — the provider
    // is called synchronously from the platform's own menu callback (UI thread, outside any
    // day-core borrow) with the location in the node's LOCAL coordinates, and its result is
    // shown. The static `set_context_menu` below stays the simple path; this one exists for
    // surfaces whose menu depends on what is under the pointer (a canvas selection,
    // docs/tree.md). Default no-op: a backend without the affordance shows nothing.
    fn set_context_menu_fn(&mut self, _h: &Self::Handle, _node: NodeId, _f: ContextMenuFn) {}

    // menus (§ menus): render `items` with the backend's native menu affordance, firing
    // `Event::MenuAction(id)` (enqueue-only) for each id'd item; `role` items use the native standard
    // command. Default no-op — a toolkit without a menu bar / context menu simply shows nothing.
    /// The application menu (macOS/Windows/Linux menu bar; the app-bar overflow on Android; the
    /// UIMenuBuilder main menu on iPadOS/Catalyst). Replaces any previous app menu.
    fn set_app_menu(&mut self, _items: &[MenuItem]) {}
    /// A context menu for `h`, shown on secondary-click (desktop) or long-press (mobile). Passing an
    /// empty slice removes it.
    fn set_context_menu(&mut self, _h: &Self::Handle, _node: NodeId, _items: &[MenuItem]) {}

    // toolbars (docs/toolbars.md): a window's native toolbar — NSToolbar, AdwHeaderBar, QToolBar,
    // CommandBar. `h` is the window root's handle (the same handle `open_window` returned, or the
    // primary root's); the backend walks from it to the window it belongs to. Default no-op: a
    // toolkit with no toolbar shows nothing rather than a drawn imitation, and reports
    // `Cap::Toolbar` as `Unsupported` so an app can put the command somewhere else.
    /// Install `items` as the window's toolbar, replacing any previous one. An empty slice removes
    /// it. Items are identified by [`ToolbarItem::id`]; a backend that can reuse the native item
    /// already carrying an id should, so a replace does not drop the search field's focus.
    fn set_toolbar(&mut self, _h: &Self::Handle, _items: &[ToolbarItem]) {}
    /// Apply a targeted change to one live toolbar item — the path a bound signal writes through,
    /// so syncing a search field does not rebuild the bar. No-op if the item is not present.
    fn update_toolbar(&mut self, _h: &Self::Handle, _patch: &ToolbarPatch) {}

    // lifecycle (docs/lifecycle.md): does this backend deliver `phase`? The default answers "yes" for
    // the universal phases (launch/activation/termination) and "no" for the mobile-only ones. Backends
    // that wire up more (the mobile ones) override this; it MUST agree with the crate's
    // `const fn lifecycle_supported`, which drives compile-time guards in `day::require_lifecycle!`.
    fn supports_lifecycle(&self, phase: Lifecycle) -> bool {
        phase.is_universal()
    }

    // pillars
    fn set_a11y(&mut self, _h: &Self::Handle, _a11y: &A11yProps) {}
    /// Read a widget's ACTUAL native accessibility properties for `a11y_audit` (§14.2) to diff
    /// against Day's expectation. Default: unsupported (`found = false`) — the audit skips the node.
    fn read_a11y(&self, _h: &Self::Handle) -> A11ySnapshot {
        A11ySnapshot::default()
    }
    fn replay(&mut self, _h: &Self::Handle, _ops: &[DrawOp], _size: Size) {}
    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        Err("snapshot unsupported".into())
    }
    /// The same capture WITH the window's own chrome — title bar, toolbar, whatever the platform
    /// draws around the content (docs/window-image.md).
    ///
    /// Defaulted to [`Self::snapshot_window`] on purpose: a backend that cannot separate the two
    /// (or has no chrome to speak of — a phone) answers with the content shot rather than an
    /// error, so an app asking for chrome degrades to "what there was" instead of failing. The
    /// dayscript `screenshot` step never calls this, which is what keeps every captured gallery
    /// baseline byte-identical to before it existed.
    fn snapshot_window_chrome(&mut self) -> Result<Vec<u8>, String> {
        self.snapshot_window()
    }
    /// Show/hide this window's `selector(Sidebar)` pane — what a
    /// [`ToolbarItemKind::SidebarToggle`] item drives. `false` when there is no split host to
    /// toggle, which is how the caller knows to render the item disabled.
    ///
    /// A duty rather than an action id, because the item carries no app closure: the native
    /// toolbar button and dayscript's `toolbar:` step both land here, so a walkthrough drives
    /// exactly the path a click takes. Defaulted, so a backend with no sidebar needs no code.
    /// docs/toolbars.md, docs/navigation.md.
    fn toggle_sidebar(&mut self) -> bool {
        false
    }
    /// Whether the UI has settled — no native transition (modal present/dismiss, nav push)
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

    /// Open `url` in the platform's default handler — the system browser for `http(s)`, the mail
    /// client for `mailto:`, etc. Backs the [`link`](../day_pieces/fn.link.html) piece. Fire and
    /// forget: there is no result, and an unopenable URL is ignored. The default no-ops so a
    /// backend that hasn't wired it up still compiles.
    fn open_url(&mut self, _url: &str) {}

    /// Ask the OS to require a second swipe for its edge gestures on `edges` (docs/cover.md) —
    /// the union requested by every mounted `defers_system_gestures` modifier, re-sent whenever
    /// that set changes (`Edges::NONE` when the last one unmounts). iOS defers the home
    /// indicator / notification edges; Android enters swipe-to-reveal immersive mode while
    /// non-empty. The default no-ops (desktop has no system edge gestures).
    fn defer_system_gestures(&mut self, _edges: Edges) {}

    /// Whether the platform is rendering in DARK appearance right now. Apps painting
    /// custom surfaces (opaque overlay panels, scrims) branch on this so their fills track
    /// the theme the DEFAULT text colors already follow — hardcoding either theme's
    /// surface produces dark-on-dark or light-on-light text on the other. Queried at
    /// build time (a piece rebuilt after a theme change re-queries). The default honors a
    /// `DAY_THEME` launch override and otherwise answers light.
    fn dark_mode(&mut self) -> bool {
        std::env::var("DAY_THEME").ok().as_deref() == Some("dark")
    }

    /// App-level appearance override: `Some(true)` forces dark, `Some(false)` forces light,
    /// `None` returns to following the system. Backends restyle their native widgets in
    /// place and answer the override from [`dark_mode`](Self::dark_mode); app-painted
    /// surfaces pick it up on their next rebuild. `Cap::Appearance` reports whether the
    /// backend honors this; the default ignores the call.
    fn set_appearance(&mut self, _dark: Option<bool>) {}

    /// Put `badge` on the app's icon in the Dock / launcher / home screen (docs/badge.md).
    ///
    /// Fire-and-forget, like `set_appearance`: a payload this toolkit cannot render is IGNORED, and
    /// the app probes `Cap::AppBadge{Count,Text,Dot}` before choosing one. Never substitute — a
    /// backend that turned `Text("beta")` into `Count(1)` would put a wrong number on a user's icon,
    /// which is worse than showing nothing. The default ignores every call.
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

    /// Open a native OS window per `options` + `kind`: create and show the window, wire ITS
    /// events to `id` — `WindowResized` (content size, points), `WindowClosed` (after the
    /// native close), `WindowFocused` (key/active changes) — and answer with its CONTENT
    /// container handle, the same contract as the `ready` root container. Backends whose
    /// window creation is asynchronous (a scene, an activity, an ability) answer
    /// [`WindowOpenReply::Pending`] and complete later through
    /// `day_core::finish_window_open(id, raw, size)` (the [`Toolkit::adopt`] seam).
    /// The default answers `Unsupported` — day-core then presents the content as a
    /// fullscreen cover in the primary window instead.
    fn open_window(
        &mut self,
        _id: NodeId,
        _options: &WindowOptions,
        _kind: WindowKind,
    ) -> WindowOpenReply<Self::Handle> {
        WindowOpenReply::Unsupported
    }

    /// Close the native window whose CONTENT container is `host` (a handle `open_window`
    /// produced). Asynchronous: the platform confirms with `Event::WindowClosed` on the
    /// window's root node, and day-core tears the subtree down THEN. Idempotent; unknown
    /// hosts are ignored. Never called with the primary window's container.
    fn close_window(&mut self, _host: &Self::Handle) {}

    /// End the application: the last window with [`WindowRole::Primary`] has closed
    /// (docs/windows.md). day-core decides WHEN — it owns the window registry and the role
    /// each window carries — and the backend supplies only the platform's own exit:
    /// `NSApplication.terminate`, `QCoreApplication::quit`, `GtkApplication.quit`,
    /// `PostQuitMessage`. Everything day-core wanted to happen first (secondary windows
    /// disposed, `WillTerminate` delivered) already has by the time this is called.
    ///
    /// The default does nothing, which is right for the backends whose process lifetime is
    /// not theirs to end — a browser tab, an Android activity, an iOS scene.
    fn quit_app(&mut self) {}

    /// Bring the window whose CONTENT container is `host` to front and make it key/active —
    /// the focus half of open-or-focus singleton windows. Default no-op.
    fn focus_window(&mut self, _host: &Self::Handle) {}

    /// Retitle the window whose CONTENT container is `host`. Default no-op.
    fn set_window_title(&mut self, _host: &Self::Handle, _title: &str) {}

    /// Resize the window whose CONTENT container is `host` to fit `size` of content, and pin it
    /// there — what `WindowOptions::size_to_fit` asks for (docs/windows.md).
    ///
    /// `size` is the CONTENT size Day measured, not the window's outer frame: the backend adds
    /// its own chrome. A settings panel is the case this exists for, and every desktop platform
    /// sizes one to its content rather than to a number the app guessed — a fixed height either
    /// clips the last row or leaves a band of empty panel under it, and which one you get depends
    /// on the user's text size.
    ///
    /// Default no-op, so a toolkit that cannot resize a live window simply keeps the size it was
    /// opened at, which is what every backend did before this existed.
    fn fit_window(&mut self, _host: &Self::Handle, _size: Size) {}

    /// Snapshot the window whose CONTENT container is `host` (the dayscript `screenshot`
    /// step's `window:` target). The default answers the primary snapshot, so backends that
    /// never open windows stay correct without changes.
    fn snapshot_window_of(&mut self, _host: &Self::Handle) -> Result<Vec<u8>, String> {
        self.snapshot_window()
    }
}

/// What a secondary window IS, so backends can apply platform conventions (docs/windows.md).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WindowKind {
    /// A regular document/tool window: resizable, miniaturizable, platform-default tabbing.
    Normal,
    /// A settings/preferences window: singleton by convention; macOS disallows window
    /// tabbing and drops the resize/minimize chrome, mobile presents it modally. Pairs with
    /// day-core's key-singleton reopen (`open_window` with a key).
    Preferences,
}

/// Whether a window keeps the APP alive (docs/windows.md close policy).
///
/// A separate axis from [`WindowKind`], which describes chrome. They correlate — a
/// preferences panel is the archetypal window that should not hold an app open — but they are
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
            WindowKind::Preferences => WindowRole::Secondary,
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

#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub title: String,
    pub size: Size,
    pub min_size: Option<Size>,
    /// Ask the backend to size this window to its CONTENT once it has been built and laid out,
    /// rather than keeping [`Self::size`] (docs/windows.md). `size` still decides the width and
    /// acts as the height CEILING, so a panel with more content than fits the screen scrolls
    /// instead of growing past it.
    ///
    /// What a preferences panel wants on every desktop. Ignored by backends that cannot resize a
    /// live window, and by the ones with no windows at all.
    pub size_to_fit: bool,
    /// The app's display name for the standard application menu / About (macOS). `None` falls back
    /// to `title`; set it when `title` carries extra decoration you don't want in "About <name>"
    /// (e.g. the showcase's window title is "Day Showcase (AppKit)" but its app name is "Showcase").
    pub app_name: Option<String>,
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions {
            title: "Day".into(),
            size: Size::new(480.0, 640.0),
            min_size: None,
            size_to_fit: false,
            app_name: None,
        }
    }
}

/// A platform backend: owns the native main loop and exactly one window in v1 (§8.1).
///
/// `run` sets up the native app + window, installs the reactive scheduler + main poster,
/// then hands `(self, root_container, content_size)` to `ready` — which mounts the tree and
/// takes ownership of the backend — and finally runs the native main loop.
pub trait Platform: Toolkit {
    /// e.g. `"macos-appkit"` — the process-constant target id.
    const TARGET: &'static str;
    /// The toolkit half of the target, e.g. `"appkit"`.
    const TOOLKIT: &'static str;

    fn run(self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Self::Handle, Size)>);

    /// Post a closure onto the native main loop. Callable from ANY thread; this is the
    /// single door the reactive scheduler and `Setter` deliveries ride (§3.3).
    fn post(f: Box<dyn FnOnce() + Send>);

    /// Post `f` onto the native main loop after (at least) `ms` milliseconds — the timer
    /// door behind `day::sleep` (docs/async.md). The default spawns a helper thread that
    /// sleeps and rides [`Platform::post`] home, which is correct on every threaded
    /// platform with zero backend code; a single-threaded host (web) overrides it with
    /// the platform's own timer.
    fn post_delayed(ms: u32, f: Box<dyn FnOnce() + Send>) {
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(u64::from(ms)));
            Self::post(f);
        });
    }

    /// Request a single main-thread callback aligned to the next display refresh (vsync), carrying
    /// the frame timestamp in seconds. The day-core animation driver re-arms it each tick while
    /// animations / game frame-clocks are live and stops requesting when none remain (no idle
    /// wakeups → battery). Main-thread only. A backend without a display link may approximate with a
    /// ~16 ms timer. Defaulted no-op: the canvas/self-driven animation path is inert until a backend
    /// provides it (native-widget animation via `AnimSpec` is unaffected). (§8.4)
    fn request_frame(_cb: Box<dyn FnOnce(f64) + 'static>) {}

    /// Ordered OS locale preference list (BCP-47), for fluent-langneg (§12.2).
    fn locale_hints(&self) -> Vec<String> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Open renderer registry (§8.2)
// ---------------------------------------------------------------------------

/// The user's language preference from the POSIX environment, newest-first, as BCP-47 tags.
///
/// `LANGUAGE` is the GNU multi-language list (`fr:en`); `LC_ALL`, `LC_MESSAGES` and `LANG` each
/// carry one locale in POSIX form (`fr_FR.UTF-8`), which becomes `fr-FR`. Shared by the backends
/// whose platform has no richer API to ask (§12.2, docs/localization.md).
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
    /// A piece that keeps per-view state of its own — a retained delegate, a session map, a native
    /// observer — has no other place to drop it: `make`/`update` are the only other hooks, and a
    /// disposed node never calls them again. Without this a piece leaks one entry per realized
    /// view, and any map keyed by the handle's ADDRESS can go on to answer for a later view that
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
            // Two pieces claiming one kind is last-linked-wins in link order — effectively
            // nondeterministic. Fail loudly in debug; in release, say so once at boot rather
            // than render the wrong widget silently.
            debug_assert!(
                false,
                "duplicate renderer registered for piece kind {kind:?}"
            );
            log::warn!("duplicate renderer for piece kind {kind:?} — later registration wins");
        }
    }
    pub fn get(&self, kind: PieceKind) -> Option<&Renderer<B>> {
        self.map.get(kind)
    }
    pub fn kinds(&self) -> impl Iterator<Item = PieceKind> + '_ {
        self.map.keys().copied()
    }
}

/// The canvas wire op codes (§11, §15.3): slot 0 of every 9-number record in
/// [`encode_ops`]'s output. The numeric values are the FROZEN wire contract — the JNI, C++,
/// and JS decoders each hand-write these as constants, so a value here may never be reused
/// or renumbered; new ops append. `ALL` drives the density test that keeps this enum and
/// the encoder honest.
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
    /// this with width 0 — the known asymmetry; see [`encode_ops`].
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
    /// Fill a polygon; points ride the texts channel as "x,y x,y …" (closed automatically).
    FillPolygon = 11,
    /// Stroke a polygon (g=width); points as [`OpCode::FillPolygon`].
    StrokePolygon = 12,
    /// Stroke a rounded rect (e=radius, g=width).
    StrokeRrect = 13,
    /// Set a gradient for the NEXT shape record (f=type: 0 linear with a,b=start / c,d=end
    /// unit points; 1 radial with a,b=center unit point, c=unit radius; e=stop count).
    /// Stops ride the texts channel as "offset,aarrggbb offset,aarrggbb …". Unit geometry
    /// resolves against the shape's bounding box.
    SetGradient = 14,
    /// Fill a path (f=fill rule: 0 non-zero / 1 even-odd); segments ride the texts channel
    /// (see [`encode_path`]).
    FillPath = 15,
    /// Stroke a path (g=width); segments as [`OpCode::FillPath`].
    StrokePath = 16,
    /// Clip to a shape (f=shape kind: 0 rect / 1 rrect(e=radius) / 2 ellipse / 3 path
    /// (e=fill rule) / 4 polygon; a..d=geometry; path/polygon payloads ride the texts
    /// channel).
    Clip = 17,
    /// Stroke style for the NEXT stroke record (a=cap, b=join, c=miter limit, d=dash
    /// phase, e=dash count; the dash array rides the texts channel).
    StrokeStyle = 18,
}

impl OpCode {
    /// Every code, in wire order — the density test iterates this so a new variant that
    /// forgets to join fails loudly.
    pub const ALL: [OpCode; 19] = [
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
    ];

    /// The code back from a wire number (a decoder-side aid and the round-trip test's
    /// reference); `None` for anything outside the table.
    pub fn from_wire(n: f64) -> Option<OpCode> {
        OpCode::ALL.into_iter().find(|c| *c as i32 as f64 == n)
    }
}

/// Flat numeric encoding of a display list for shim/JNI boundaries (§11, §15.3): per op
/// 9 numbers [kind, a, b, c, d, e, f, g, rgba-bits]; text payloads ride separately in order.
/// The kind slot carries an [`OpCode`] — its doc comments are the per-slot contract.
///
/// Transports join `texts` with the unit separator U+001F (one entry per text-carrying
/// record, in order), so text payloads must not contain U+001F. Known asymmetry:
/// `Fill(Shape::Arc)` encodes as [`OpCode::StrokeArc`] with width 0 — filled arcs render
/// only on the direct-replay backends (AppKit/UIKit); use a polygon fan if a filled arc
/// must be portable.
/// Serialize a path for the `texts` side-channel: one space-separated token per segment,
/// `M x y` / `L x y` / `Q cx cy x y` / `C c1x c1y c2x c2y x y` / `Z`.
///
/// Text rather than numbers because that is the channel the encoder already has for
/// variable-length payloads, and every decoder (JS, Java, C++) can already split a string. The
/// numeric channel is a flat `[f64; 9]`-per-record array with no length prefix, so a path could
/// not ride it without changing the record shape for every backend at once.
pub fn encode_path(path: &Path) -> String {
    let mut out = String::with_capacity(path.segs.len() * 16);
    for seg in &path.segs {
        if !out.is_empty() {
            out.push(' ');
        }
        match seg {
            PathSeg::Move(p) => out.push_str(&format!("M {} {}", p.x, p.y)),
            PathSeg::Line(p) => out.push_str(&format!("L {} {}", p.x, p.y)),
            PathSeg::Quad(c, p) => out.push_str(&format!("Q {} {} {} {}", c.x, c.y, p.x, p.y)),
            PathSeg::Cubic(a, b, p) => out.push_str(&format!(
                "C {} {} {} {} {} {}",
                a.x, a.y, b.x, b.y, p.x, p.y
            )),
            PathSeg::Close => out.push('Z'),
        }
    }
    out
}

/// `FillRule` as a wire number: 0 non-zero, 1 even-odd.
fn rule_bits(rule: FillRule) -> f64 {
    match rule {
        FillRule::NonZero => 0.0,
        FillRule::EvenOdd => 1.0,
    }
}

pub fn encode_ops(ops: &[DrawOp]) -> (Vec<f64>, Vec<String>) {
    fn color_bits(c: Color) -> f64 {
        let r = (c.r.clamp(0.0, 1.0) * 255.0) as u32;
        let g = (c.g.clamp(0.0, 1.0) * 255.0) as u32;
        let b = (c.b.clamp(0.0, 1.0) * 255.0) as u32;
        let a = (c.a.clamp(0.0, 1.0) * 255.0) as u32;
        ((a << 24) | (r << 16) | (g << 8) | b) as f64
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
        nums.extend_from_slice(&[k as i32 as f64, a, b, c, d, e, f, g, color_bits(col)]);
    }
    /// One shape record (the fill/stroke kinds shared by both ops).
    fn shape_record(
        stroke: bool,
        shape: &Shape,
        w: f64,
        col: Color,
        nums: &mut Vec<f64>,
        texts: &mut Vec<String>,
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
            Shape::Arc {
                rect,
                start_deg,
                sweep_deg,
            } => push(
                OpCode::StrokeArc,
                rect.origin.x,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
                *start_deg,
                *sweep_deg,
                w,
                col,
                nums,
            ),
            Shape::Line(p1, p2) => {
                push(OpCode::Line, p1.x, p1.y, p2.x, p2.y, 0.0, 0.0, w, col, nums)
            }
            Shape::Polygon(pts) => {
                // Variable-length points ride the texts side-channel ("x,y x,y …"),
                // consumed in record order exactly like text payloads.
                push(
                    if stroke {
                        OpCode::StrokePolygon
                    } else {
                        OpCode::FillPolygon
                    },
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    w,
                    col,
                    nums,
                );
                texts.push(
                    pts.iter()
                        .map(|p| format!("{},{}", p.x, p.y))
                        .collect::<Vec<_>>()
                        .join(" "),
                );
            }
            Shape::Path(path) => {
                // Same side-channel as Polygon, one token per segment (see `encode_path`).
                // The fill rule rides slot f; stroking ignores it.
                push(
                    if stroke {
                        OpCode::StrokePath
                    } else {
                        OpCode::FillPath
                    },
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    0.0,
                    rule_bits(path.rule),
                    w,
                    col,
                    nums,
                );
                texts.push(encode_path(path));
            }
        }
    }
    /// A gradient applies to the NEXT shape record, fill or stroke. Geometry per type rides
    /// slots a..d and the type discriminant slot f — ONE record shape, so every decoder keeps a
    /// single gradient code path.
    fn gradient_record(
        geo: [f64; 4],
        kind: f64,
        stops: &[(f64, Color)],
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
                .map(|(o, c)| format!("{o},{:08x}", color_bits(*c) as u32))
                .collect::<Vec<_>>()
                .join(" "),
        );
    }
    let mut nums = Vec::with_capacity(ops.len() * 9);
    let mut texts = Vec::new();
    for op in ops {
        match op {
            DrawOp::Fill(shape, paint) => {
                // A gradient emits one kind-14 set-gradient record before its shape record;
                // the stops ride the texts channel as "offset,aarrggbb offset,aarrggbb …".
                // Geometry per type rides slots a..d, the type discriminant slot f — ONE
                // record shape, so every decoder keeps a single gradient code path.
                let col = match paint {
                    Paint::Solid(c) => *c,
                    Paint::Linear(g) => {
                        gradient_record(
                            [g.start.x, g.start.y, g.end.x, g.end.y],
                            0.0,
                            &g.stops,
                            &mut nums,
                            &mut texts,
                        );
                        // The gradient replaces the shape record's color — but it must be
                        // OPAQUE, not clear: Skia-based decoders (Android Paint, OH_Drawing)
                        // modulate a shader by the paint alpha, so a clear slot would render
                        // the whole gradient invisible.
                        Color::WHITE
                    }
                    Paint::Radial(g) => {
                        gradient_record(
                            [g.center.x, g.center.y, g.radius, 0.0],
                            1.0,
                            &g.stops,
                            &mut nums,
                            &mut texts,
                        );
                        Color::WHITE
                    }
                };
                shape_record(false, shape, 0.0, col, &mut nums, &mut texts);
            }
            DrawOp::Stroke(shape, paint, style) => {
                // A styled stroke emits one kind-18 record first, applying to the NEXT stroke
                // only — the same "modifier record" shape the gradient uses, so decoders keep
                // one rule: consume, apply to the next shape record, reset.
                if !style.is_plain() {
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
                        &mut nums,
                    );
                    texts.push(
                        style
                            .dash
                            .iter()
                            .map(|d| d.to_string())
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                }
                let col = match paint {
                    Paint::Solid(c) => *c,
                    Paint::Linear(g) => {
                        gradient_record(
                            [g.start.x, g.start.y, g.end.x, g.end.y],
                            0.0,
                            &g.stops,
                            &mut nums,
                            &mut texts,
                        );
                        Color::WHITE
                    }
                    Paint::Radial(g) => {
                        gradient_record(
                            [g.center.x, g.center.y, g.radius, 0.0],
                            1.0,
                            &g.stops,
                            &mut nums,
                            &mut texts,
                        );
                        Color::WHITE
                    }
                };
                shape_record(true, shape, style.width, col, &mut nums, &mut texts);
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
                    Shape::Path(p) => (3.0, [0.0; 4], rule_bits(p.rule), Some(encode_path(p))),
                    Shape::Polygon(pts) => (
                        4.0,
                        [0.0; 4],
                        0.0,
                        Some(
                            pts.iter()
                                .map(|p| format!("{},{}", p.x, p.y))
                                .collect::<Vec<_>>()
                                .join(" "),
                        ),
                    ),
                    // A clip to a line or an arc has no interior to clip TO. Clipping to an
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
            } => {
                push(
                    OpCode::Text,
                    at.x,
                    at.y,
                    0.0,
                    0.0,
                    *size,
                    match anchor {
                        TextAnchor::Leading => 0.0,
                        TextAnchor::Centered => 1.0,
                    },
                    0.0,
                    *color,
                    &mut nums,
                );
                texts.push(text.clone());
            }
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
mod encode_ops_tests {
    use super::*;

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
    /// entry per text-carrying record, in order — the contract every shim decoder assumes.
    #[test]
    fn encode_ops_emits_whole_records_and_matched_text_payloads() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        let red = Color::rgb(1.0, 0.0, 0.0);
        let ops = vec![
            DrawOp::Fill(Shape::Rect(r), Paint::Solid(red)),
            DrawOp::Stroke(Shape::Rect(r), Paint::Solid(red), StrokeStyle::round(2.0)),
            DrawOp::Fill(Shape::RoundedRect(r, 5.0), Paint::Solid(red)),
            DrawOp::Fill(Shape::Ellipse(r), Paint::Solid(red)),
            // The documented asymmetry: a FILLED arc encodes as StrokeArc with width 0.
            DrawOp::Fill(
                Shape::Arc {
                    rect: r,
                    start_deg: 0.0,
                    sweep_deg: 90.0,
                },
                Paint::Solid(red),
            ),
            DrawOp::Stroke(
                Shape::Line(Point::new(0.0, 0.0), Point::new(9.0, 9.0)),
                Paint::Solid(red),
                StrokeStyle::width(1.0),
            ),
            DrawOp::Fill(
                Shape::Polygon(vec![
                    Point::new(0.0, 0.0),
                    Point::new(4.0, 0.0),
                    Point::new(2.0, 3.0),
                ]),
                Paint::Solid(red),
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
                anchor: TextAnchor::Leading,
            },
            DrawOp::Save,
            DrawOp::Concat(Affine::IDENTITY),
            DrawOp::Restore,
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
                | OpCode::StrokeStyle => 1,
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
                StrokeStyle, // ::round(2.0) is not plain — the style modifier precedes
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
                Save,
                Concat,
                Restore,
            ]
        };
        assert_eq!(decoded, expected);
        // …with width 0, per the documented asymmetry.
        let arc = nums
            .chunks(9)
            .find(|r| r[0] == OpCode::StrokeArc as i32 as f64);
        assert_eq!(arc.map(|r| r[7]), Some(0.0));
        // A gradient-filled shape's color slot is opaque white, never clear (Skia decoders
        // modulate the shader by it).
        let path_rec = nums
            .chunks(9)
            .find(|r| r[0] == OpCode::FillPath as i32 as f64);
        assert_eq!(
            path_rec.map(|r| r[8] as u32),
            Some(0xFFFF_FFFF),
            "gradient shape record must carry opaque white"
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
        // The case a translated string will find: markup metacharacters in the CONTENT.
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
            color: Some(Color::rgb(1.0, 0.0, 0.0)),
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
            background: Some(Color::rgb(0.0, 1.0, 0.0)),
            underline: Underline::Single,
            ..TextRun::default()
        };
        let pango = runs_to_markup("hi", std::slice::from_ref(&run), MarkupDialect::Pango, 16.0);
        assert!(pango.contains("background=\"#00ff00\""), "{pango}");
        // Pango takes an ABSOLUTE size in 1/1024 pt — 16 pt base × 1.5 = 24 pt — because the
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
        // Qt takes POINTS: 16 pt base × 1.5 (observed — its CSS subset ignores a percentage).
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
            strikethrough: true,
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
        // Percent-encoding passes through — the route parser decodes, not this mapping.
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

    /// The `kinds::*` constants are the enum's keys — the two spellings cannot drift.
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
        assert_eq!(Builtin::ALL.len(), 22);
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
    /// POSIX locale strings are not BCP-47 tags, and "C" is not a language — the difference is
    /// the whole job of `posix_locale_hints` (§12.2).
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

    /// The whole point of the registry: one `sweep(key)` clears EVERY table without any
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
        hooked.remove(8);
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
