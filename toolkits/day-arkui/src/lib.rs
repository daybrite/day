// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-arkui: the HarmonyOS Next **ArkUI** backend (target `harmony-arkui`; DESIGN.md §9).
//!
//! HarmonyOS has no AOSP layer; its UI framework is ArkUI. Day drives it through the **ArkUI
//! Native NodeAPI**, bound by openharmony-rs's `ohos-sys` (src/node.rs): every Piece becomes a
//! real `ArkUI_NodeHandle` (Text / Button / TextInput / Toggle / Slider / Stack), built natively
//! and mounted into an ArkTS `NodeContent` slot. The ArkTS host reaches the native side through
//! the NAPI module ohos-rs's `napi-ohos` registers (src/host_api.rs). Architecturally it mirrors
//! `day-android`: a managed UI runtime (ArkTS) hosts the window, native code (Rust) builds the
//! tree, and **day owns absolute layout**: containers are `ARKUI_NODE_STACK` and each child gets
//! an explicit position + size (in vp = day points).
//!
//! Off HarmonyOS the crate is empty (`cfg(target_env = "ohos")`), so the workspace still
//! type-checks on the host.

#![allow(clippy::missing_safety_doc)]

#[cfg(target_env = "ohos")]
pub use imp::*;

#[cfg(target_env = "ohos")]
mod anim;
#[cfg(target_env = "ohos")]
mod bridge;
#[cfg(target_env = "ohos")]
mod canvas;
#[cfg(target_env = "ohos")]
mod events;
#[cfg(target_env = "ohos")]
pub mod ext;
#[cfg(target_env = "ohos")]
mod fonts;
#[cfg(target_env = "ohos")]
mod gesture;
#[cfg(target_env = "ohos")]
mod hilog;
#[cfg(target_env = "ohos")]
mod host;
#[cfg(target_env = "ohos")]
mod host_api;
#[cfg(target_env = "ohos")]
mod images;
#[cfg(target_env = "ohos")]
mod list;
#[cfg(target_env = "ohos")]
mod main_thread;
/// The ArkUI node API for standalone pieces (docs/extending.md): create a node, set its
/// attributes, register its events, and the raw bindings behind them.
#[cfg(target_env = "ohos")]
pub mod node;
#[cfg(target_env = "ohos")]
mod resources;
#[cfg(target_env = "ohos")]
mod transfer;
#[cfg(target_env = "ohos")]
mod vsync;
#[cfg(target_env = "ohos")]
pub use ext::*;
/// The NDK's ArkUI bindings, for a piece that needs an attribute or event `node` does not wrap.
#[cfg(target_env = "ohos")]
pub use ohos_sys::arkui as arkui_sys;

#[cfg(target_env = "ohos")]
mod imp {
    use std::cell::{Cell, RefCell};
    use std::collections::HashMap;
    use std::rc::Rc;

    use linkme::distributed_slice;

    use day_spec::props::*;
    use day_spec::{
        A11yProps, AnimSpec, Builtin, Cap, DrawOp, Event, EventSink, Font, FontSpec, GestureKind,
        NodeId, PieceKind, Platform, Point, Proposal, Rect, Registry, Renderer, Size, Support,
        Toolkit, WindowOptions, kinds,
    };

    use crate::node::{self, Handle};

    /// An `ArkUI_NodeHandle`. day owns the tree, so the raw pointer is the identity.
    #[derive(Clone, Copy, PartialEq, Eq, Hash)]
    pub struct AHandle(pub Handle);

    type Sink = Rc<dyn Fn(NodeId, Event)>;

    day_core::tls_group! {
        static BUTTON_INK: day_spec::sidetable::SideTable<u32> = day_spec::sidetable::SideTable::new();
        static BUTTON_CHILDREN: RefCell<HashMap<usize, Vec<AHandle>>> = RefCell::new(HashMap::new());
        /// Navigation state is keyed by host pointer; page keys are globally unique Day NodeIds.
        /// Each host owns its pending children and path. Creating another host must never
        /// clear a sibling tab's state or redirect its Back/search/title callbacks.
        static NAV_HOSTS: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        /// The primary window's toolbar (docs/toolbars.md) as Day's edits and patches left it:
        /// what the Navigation's title-bar actions are painted from.
        static WINDOW_BAR: RefCell<day_spec::ToolbarMirror> = RefCell::new(day_spec::ToolbarMirror::default());
        static NAV_ATTACHED: RefCell<HashMap<usize, Vec<(usize, u64)>>> = RefCell::new(HashMap::new());
        static NAV_OWNERS: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());
        static NAV_PUSHED: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        /// Keys whose NavDestination already disappeared (`nav_popped`) while the page is
        /// still mounted; its Remove must not touch the torn-down ArkTS content.
        static NAV_POPPED_KEYS: RefCell<std::collections::HashSet<u64>> =
            RefCell::new(std::collections::HashSet::new());
        /// Keys whose next `navPopped` acknowledges a Day-initiated pop (must not sync back
        /// as a native back). Keyed, not counted: a page pushed and popped within one frame
        /// never mounts, so ArkUI fires no disappear for it, and a counter would wait forever
        /// on an acknowledgment that never comes (the CI post-stack blank-screenshot wedge).
        static NAV_EXPECT_POP: RefCell<std::collections::HashSet<u64>> =
            RefCell::new(std::collections::HashSet::new());
        /// Rust's order of pushed page keys: what a `NavPatch::Popped` pops, so the pop
        /// handler knows which key it retired (ArkTS only reports keys on disappear).
        static NAV_STACK: RefCell<HashMap<usize, Vec<u64>>> = RefCell::new(HashMap::new());
        /// NAV_PAGE node ptr → day NodeId (recorded at realize; consumed by insert/push).
        static NAV_PAGE_IDS: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        /// The window root Stack + content size, set by [`init`] before `run`.
        static ROOT: RefCell<Option<(AHandle, Size)>> = const { RefCell::new(None) };
        /// Dark mode (docs/localization + theming): resolved once at init. DAY_THEME (the CI
        /// forced theme) wins, else DAY_ARKUI_DARK (the system color mode the ArkTS host reports
        /// via setEnv before start()). ArkUI's C-API nodes do not re-theme hardcoded colors, so
        /// every neutral day-arkui paint branches on this flag.
        static IS_DARK: Cell<bool> = const { Cell::new(false) };
        /// Slider NodeId → (min, max), so ArkUI's 0..100 maps back to day's range. Keyed by the
        /// id, not the node pointer, because the change event carries only the id: a
        /// pointer-keyed map missed there, and every drag landed in the fallback 0..1.
        static SLIDER_RANGE: RefCell<HashMap<u64, (f64, f64)>> = RefCell::new(HashMap::new());
        /// ArkTS-built piece nodes (docs/extending.md): wrapper Stack ptr → (day NodeId, the
        /// ArkTS FrameNode inside it). Release detaches the FrameNode, sends the ArkTS side its
        /// disposal (ArkTS owns that node), and disposes only the native wrapper.
        static PIECE_NODES: RefCell<HashMap<usize, (u64, usize)>> = RefCell::new(HashMap::new());
        /// ArkTS picker size by wrapper: Some(chrome) is a menu, None is a segmented row.
        /// Both reserve room for every option, so selecting another label never resizes them.
        static MENU_PICKER_SIZE: RefCell<HashMap<usize, (Option<f64>, Size)>> = RefCell::new(HashMap::new());
        /// The size each ArkTS piece was last told it has (see `set_frame`), by wrapper handle.
        static PIECE_FRAME: RefCell<HashMap<usize, Size>> = RefCell::new(HashMap::new());
        /// The neutral paints that follow the color mode (see [`themed`]), by the node whose
        /// lifetime bounds them: each owner's (node, paint, light ARGB, dark ARGB).
        static THEMED: RefCell<HashMap<usize, Vec<(usize, Paint, u32, u32)>>> =
            RefCell::new(HashMap::new());
        // Text-area (min_lines, max_lines) by handle, for the measure band (docs/textarea.md).
        static TEXTAREA_LINES: RefCell<HashMap<usize, (u32, u32)>> = RefCell::new(HashMap::new());
        /// A NAV_MENU row's synthetic click id → (menu node, row index). A tap on a menu row is a
        /// plain NODE_ON_CLICK, so we register it against a fresh synthetic id and translate the
        /// click back into `SelectionChanged(index)` against the menu host (day-android does the
        /// same with a per-row listener). See [`on_event`].
        static MENU_ROWS: RefCell<HashMap<u64, (NodeId, i64)>> = RefCell::new(HashMap::new());
        /// NAV_MENU scroll node ptr → its day NodeId, so `NavMenuPatch::Items` (which only gets
        /// the handle) can rebuild the rows against the right menu id, and `release` can purge
        /// the menu's synthetic-row entries from [`MENU_ROWS`].
        static NAV_MENU_IDS: RefCell<HashMap<usize, NodeId>> = RefCell::new(HashMap::new());
        /// Monotonic synthetic-id counter for menu rows (kept out of day's NodeId space by using the
        /// high bit, which day-core never allocates).
        static SYNTH: Cell<u64> = const { Cell::new(1u64 << 63) };
        /// List host node ptr → its day NodeId, so `attach_list` (which only gets the handle) can
        /// key the source by the id the native adapter callbacks report.
        static LIST_NODE: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        /// List host NodeId → its injected row-pull source (docs/list.md).
        /// Programmatic selection per list (docs/list.md `ListPatch::Selected`): the list
        /// module paints from this at bind and on a sync (`list::paint_selection`).
        static LIST_SELECTED: RefCell<HashMap<u64, std::collections::BTreeSet<usize>>> =
            RefCell::new(HashMap::new());
        /// Lists with a posted reload not yet run (see the Reload arm): one data change often
        /// fires several watches (shape + expansion + selection), and back-to-back
        /// ReloadAllItems bursts dropped adapter ADDs, so coalesce to one per drain.
        static LIST_RELOAD_PENDING: RefCell<std::collections::HashSet<usize>> =
            RefCell::new(std::collections::HashSet::new());
        /// Control handle → day node, for the echo cells below (a patch sees only the handle;
        /// the echoed event carries only the node).
        static CTRL_NODE: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        /// Programmatic-set echo cells (§4.4): ArkUI fires onChange for programmatic sets
        /// too, so a value day just wrote comes straight back as a change event, and a
        /// two-way binding then re-writes the app state (on Day-Sketch, phantom "style"
        /// undo units on every selection change). The cell holds the last programmatic
        /// value; a matching event is the echo and is swallowed, a differing one is the
        /// user and clears the cell.
        static TEXT_ECHO: RefCell<HashMap<u64, String>> = RefCell::new(HashMap::new());
        /// Text fields, by day node (docs/textfield.md). ArkUI has no read-only attribute for
        /// a TextInput, so the backend refuses the edits itself; see [`InputField`].
        static INPUT_FIELDS: RefCell<HashMap<u64, InputField>> = RefCell::new(HashMap::new());
        static SLIDER_ECHO: RefCell<HashMap<u64, f64>> = RefCell::new(HashMap::new());
        /// Toggle gates (§4.4), by day node. A value-match cell is not enough for a switch:
        /// ArkUI can report a programmatic set late, after day has already written the
        /// opposite value, and that stale event flipped the app state back. So a native
        /// change reaches the app only when the user touched or keyed the switch since
        /// day's last write; any other differing event is stale and the switch is repainted.
        static TOGGLE_GATE: RefCell<HashMap<u64, ToggleGate>> = RefCell::new(HashMap::new());
        static LIST_SOURCES: RefCell<HashMap<u64, day_spec::ListSource>> =
            RefCell::new(HashMap::new());
        /// Node ids with a Tap gesture (docs/shapes.md): a NODE_ON_CLICK on these emits `Event::Tap`
        /// (not `Event::Pressed`), which is how a canvas/shape `.on_tap` (e.g. day-piece-rating's
        /// stars) receives taps on ArkUI. See [`Toolkit::enable_gesture`] + [`on_event`].
        static TAP_NODES: RefCell<std::collections::HashSet<u64>> =
            RefCell::new(std::collections::HashSet::new());
        /// Tap-node handle ptr → its node id, so `release` (which only gets the handle) can drop the
        /// matching TAP_NODES entry (else a recycling list would grow the set unbounded).
        static TAP_HANDLES: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        /// Fullscreen covers (docs/cover.md): handle ptr → day NodeId. A cover's frame is
        /// native-owned (full window while presented), so `set_frame` skips these.
        static COVER_NODES: RefCell<HashMap<usize, u64>> = RefCell::new(HashMap::new());
        /// A cover's current native parent (its parked tree slot, or the dedicated cover
        /// layer while presented). Presenting re-homes it, so removals must target this.
        static COVER_PARENTS: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());
        /// Covers currently presented (topped on the cover layer). Separate from
        /// COVER_PARENTS, which records the parked tree slot; on the cover-fallback
        /// tier (docs/windows.md) that slot is the root itself, so parent-comparison cannot
        /// stand in for presented-ness.
        static COVER_PRESENTED: RefCell<std::collections::HashSet<usize>> =
            RefCell::new(std::collections::HashSet::new());
        /// The root-page Stack + its size, kept for the app's lifetime (unlike [`ROOT`],
        /// which `run` consumes).
        static ROOT_KEEP: Cell<Option<(usize, f64, f64)>> = const { Cell::new(None) };
        /// Separate host slot above ArkTS Navigation; the root page is below NavDestinations.
        static COVER_ROOT: Cell<Option<(usize, f64, f64)>> = const { Cell::new(None) };
        /// Nav transitions in flight, for [`Toolkit::ui_idle`] (dayscript screenshots wait on
        /// it): pushed page keys awaiting their destination's first area report, and popped
        /// page keys awaiting their `navPopped` acknowledgment. Both hold only keys whose
        /// native event is still coming: a pop retires its own pending-push entry (a
        /// never-mounted page reports neither), and only lands in the pending-pop set when
        /// the page had mounted.
        static NAV_PENDING_PUSH: RefCell<std::collections::HashSet<u64>> =
            RefCell::new(std::collections::HashSet::new());
        static NAV_PENDING_POP: RefCell<std::collections::HashSet<u64>> =
            RefCell::new(std::collections::HashSet::new());
        /// Each `scroll()`'s backend-owned content Stack (scroll ptr → stack ptr), sized by
        /// `set_scroll_content`. Day's content nodes are layout-only (no native child of
        /// their own), and an ArkUI Scroll whose children are absolutely-placed leaves
        /// measures a content extent of 0, so offsets clamp to nothing and neither touch nor
        /// programmatic scrolling moves. `insert`/`remove` re-route the scroll's day
        /// children into the container so the Scroll measures the real extent.
        static SCROLL_CONTENT: RefCell<HashMap<usize, usize>> = RefCell::new(HashMap::new());

        /// Each picker wheel's live selection, so a change of options can keep it: the
        /// range attribute is set whole, and the selected index goes with it. A
        /// [`SideTable`], so the backend's release sweep drops a dead picker's entry.
        static PICKER_SELECTED: day_spec::sidetable::SideTable<usize> =
            day_spec::sidetable::SideTable::new();

        /// Every live suite, by host node pointer. Suites nest (a tab whose page holds tabs of
        /// its own), so each host keeps its own pages, bar and layout.
        static NAV_SUITES: RefCell<HashMap<usize, NavSuite>> = RefCell::new(HashMap::new());
        /// The suite whose rows have not arrived yet: its host is realized before its sidebar
        /// page, so the next navigation menu realized is that suite's, and no later one is.
        static SUITE_AWAITING_MENU: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
        /// Each suite's navigation menu (menu node → suite host), for a data-driven rows patch.
        static MENU_SUITE: RefCell<HashMap<u64, usize>> = RefCell::new(HashMap::new());

        /// Secondary window roots (docs/windows.md): (day node, the window's Stack node
        /// pointer): the multiton DayWindowAbility instances' content.
        static SECONDARY: RefCell<Vec<(u64, usize)>> = const { RefCell::new(Vec::new()) };
    }

    /// Height of the composed bottom bar, in vp (HarmonyOS's tab-bar metric).
    const NAV_BAR_H: f64 = 56.0;
    /// The rail's width (Medium): an icon over a short label per destination.
    const NAV_RAIL_W: f64 = 80.0;
    /// The sidebar's width (Expanded): an icon beside a label per destination.
    const NAV_SIDEBAR_W: f64 = 240.0;

    /// Where a suite draws its rows, by the host's width (docs/size-classes.md): the bottom bar
    /// of a phone, a rail on a portrait tablet, a sidebar on a landscape one. The same
    /// resident-page model at every width (the pieces layer lowers `Tabs` and the chrome is
    /// this backend's own), which is what Material's navigation suite and iOS 18's tab sidebar
    /// do natively.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum SuitePlacement {
        Bottom,
        Rail,
        Sidebar,
    }

    fn suite_placement(width: f64) -> SuitePlacement {
        // The width classes' own boundaries (docs/size-classes.md): Compact < 600, Medium < 840.
        if width < 600.0 {
            SuitePlacement::Bottom
        } else if width < 840.0 {
            SuitePlacement::Rail
        } else {
            SuitePlacement::Sidebar
        }
    }

    /// The navigation suite (`NavPresentation::Tabs`): resident pages over a bottom bar.
    ///
    /// ArkUI's native node set has no tab container (`ARKUI_NODE_TABS` is an ArkTS-only
    /// component, and the NDK exposes a swiper at most), so the bar is composed from the same
    /// primitives every other Day piece is built from (docs/navigation.md). One implementation,
    /// the platform's own metrics, and the rows keep their meaning: a bar item reports through
    /// the same synthetic-click table a sidebar row uses, so a tap is one event either way.
    struct NavSuite {
        pages: AHandle,
        bar: AHandle,
        /// Destination pages in bar order; index i is the `Select(i)` index.
        items: Vec<(AHandle, NodeId)>,
        /// The bar's item nodes, so a rebuild can take the old ones out first.
        bar_items: Vec<AHandle>,
        bar_inks: Vec<(Handle, Paint, usize)>,
        selected: usize,
        /// The pages area, so a page joining later can be sized without waiting for a resize.
        page_size: Size,
        /// Where the rows are drawn now; a width crossing a class boundary rebuilds the bar.
        placement: SuitePlacement,
        /// The rows the bar was last filled from, so a placement change can refill it.
        menu: Option<NodeId>,
        labels: Vec<String>,
        icons: Vec<Option<String>>,
    }

    fn next_synth() -> u64 {
        SYNTH.with(|c| {
            let v = c.get();
            c.set(v + 1);
            v
        })
    }

    /// The staged image for a name (docs/vectors.md): a vector's SVG (recolorable), else the
    /// raster PNG, as a `resource://RAWFILE` URI.
    fn named_image(name: &str) -> (String, bool) {
        let svg = format!("day/{name}.svg");
        if crate::resources::rawfile_exists(&svg) {
            (format!("resource://RAWFILE/{svg}"), true)
        } else {
            (format!("resource://RAWFILE/day/{name}.png"), false)
        }
    }

    /// An icon node for a nav row or bar item: a vector tinted with `tint` or the themed
    /// secondary color, a raster drawn as authored.
    fn row_icon(owner: usize, name: &str, tint: Option<day_spec::Color>, size: f64) -> AHandle {
        let icon = new_node(node::IMAGE);
        let (src, is_vector) = named_image(name);
        node::set_image_src(icon.0, &src);
        if is_vector {
            match tint {
                Some(c) => node::set_image_fill(icon.0, argb(c)),
                None => themed(owner, icon.0, Paint::ImageFill, 0x9900_0000, 0x99FF_FFFF),
            }
        }
        node::set_image_fit(icon.0, 0);
        node::set_size(icon.0, size, size);
        icon
    }

    /// Build the bar's items from the host's rows: an icon over a label per destination, each
    /// registering a synthetic click that reports `SelectionChanged(i)` against the menu node.
    fn suite_fill_bar(
        host: usize,
        menu: NodeId,
        items: &[String],
        icons: &[Option<String>],
        selected: usize,
    ) {
        NAV_SUITES.with(|c| {
            let mut c = c.borrow_mut();
            let Some(suite) = c.get_mut(&host) else {
                return;
            };
            suite.menu = Some(menu);
            suite.labels = items.to_vec();
            suite.icons = icons.to_vec();
            let placement = suite.placement;
            for old in std::mem::take(&mut suite.bar_items) {
                node::remove_child(suite.bar.0, old.0);
            }
            suite.bar_inks.clear();
            forget_themed(host);
            // The bar's ground, kept across refills.
            themed(
                host,
                suite.bar.0,
                Paint::Background,
                0xFFF1_F3F5,
                0xFF1C_1C1E,
            );
            for (i, title) in items.iter().enumerate() {
                // A bottom bar's and a rail's cell stacks the icon over the label; a sidebar's
                // row puts them side by side, the way the rows list draws them.
                let cell = new_node(if placement == SuitePlacement::Sidebar {
                    node::ROW
                } else {
                    node::COLUMN
                });
                let synth = next_synth();
                MENU_ROWS.with(|m| m.borrow_mut().insert(synth, (menu, i as i64)));
                // The selected destination takes the accent; the rest the secondary label color,
                // which is how a HarmonyOS bottom bar reads.
                let on = i == selected;
                let (tint_light, tint_dark) = if on {
                    (0xFF00_7DFF, 0xFF3E_9BFF)
                } else {
                    (0x9900_0000, 0x99FF_FFFF)
                };
                let mut child = 0;
                if let Some(Some(name)) = icons.get(i) {
                    let icon = new_node(node::IMAGE);
                    let (src, is_vector) = named_image(name);
                    node::set_image_src(icon.0, &src);
                    if is_vector {
                        themed(host, icon.0, Paint::ImageFill, tint_light, tint_dark);
                        suite.bar_inks.push((icon.0, Paint::ImageFill, i));
                    }
                    node::set_image_fit(icon.0, 0);
                    node::set_size(icon.0, 24.0, 24.0);
                    if placement == SuitePlacement::Sidebar {
                        node::set_margin(icon.0, 8.0);
                    }
                    node::insert_child(cell.0, icon.0, child);
                    child += 1;
                }
                let label = new_node(node::TEXT);
                node::set_text(label.0, title);
                node::set_font_size(
                    label.0,
                    if placement == SuitePlacement::Sidebar {
                        15.0
                    } else {
                        10.0
                    },
                );
                themed(host, label.0, Paint::Font, tint_light, tint_dark);
                suite.bar_inks.push((label.0, Paint::Font, i));
                node::insert_child(cell.0, label.0, child);
                match placement {
                    // The bar's cells share its width.
                    SuitePlacement::Bottom => node::set_flex_grow(cell.0, 1.0),
                    SuitePlacement::Rail => node::set_size(cell.0, NAV_RAIL_W, 64.0),
                    SuitePlacement::Sidebar => node::set_size(cell.0, NAV_SIDEBAR_W, 48.0),
                }
                node::register_event(cell.0, node::EV_CLICK, synth);
                node::insert_child(suite.bar.0, cell.0, i as i32);
                suite.bar_items.push(cell);
            }
        });
    }

    /// Lay the suite out inside `size` and tell every page how much room it has.
    ///
    /// The pages area and the bar are sized here rather than by day-core, which sees one host
    /// node and gives it one frame: the same division of labor every other backend's native
    /// nav container performs for itself.
    fn suite_layout(host: usize, size: Size) {
        // The rows' placement follows the width. A change swaps the bar for one of the other
        // orientation (a Row for the bottom bar, a Column for a rail or sidebar) and refills
        // it from the rows it was last given; the pages never notice beyond their new frame.
        let placement = suite_placement(size.width);
        let refill = NAV_SUITES.with(|c| {
            let mut c = c.borrow_mut();
            let suite = c.get_mut(&host)?;
            if suite.placement == placement {
                return None;
            }
            suite.placement = placement;
            for old in std::mem::take(&mut suite.bar_items) {
                node::remove_child(suite.bar.0, old.0);
                node::dispose(old.0);
            }
            suite.bar_inks.clear();
            node::remove_child(host as Handle, suite.bar.0);
            node::dispose(suite.bar.0);
            let bar = new_node(if placement == SuitePlacement::Bottom {
                node::ROW
            } else {
                node::COLUMN
            });
            if placement == SuitePlacement::Sidebar {
                // Rows start at the leading edge, like the list they stand in for.
                node::set_i32(
                    bar.0,
                    ohos_sys::arkui::native_node::ArkUI_NodeAttributeType::NODE_COLUMN_ALIGN_ITEMS,
                    ohos_sys::arkui::native_type::ArkUI_HorizontalAlignment::ARKUI_HORIZONTAL_ALIGNMENT_START.0 as i32,
                );
            }
            node::insert_child(host as Handle, bar.0, 1);
            suite.bar = bar;
            suite
                .menu
                .map(|menu| (menu, suite.labels.clone(), suite.icons.clone(), suite.selected))
        });
        if let Some((menu, labels, icons, selected)) = refill {
            suite_fill_bar(host, menu, &labels, &icons, selected);
            suite_select(host, selected);
        }
        let reports: Vec<(NodeId, Size)> = NAV_SUITES.with(|c| {
            let mut c = c.borrow_mut();
            let Some(suite) = c.get_mut(&host) else {
                return Vec::new();
            };
            let (page_x, page, bar) = match placement {
                SuitePlacement::Bottom => {
                    let page = Size::new(size.width, (size.height - NAV_BAR_H).max(0.0));
                    (0.0, page, (0.0, page.height, size.width, NAV_BAR_H))
                }
                SuitePlacement::Rail => (
                    NAV_RAIL_W,
                    Size::new((size.width - NAV_RAIL_W).max(0.0), size.height),
                    (0.0, 0.0, NAV_RAIL_W, size.height),
                ),
                SuitePlacement::Sidebar => (
                    NAV_SIDEBAR_W,
                    Size::new((size.width - NAV_SIDEBAR_W).max(0.0), size.height),
                    (0.0, 0.0, NAV_SIDEBAR_W, size.height),
                ),
            };
            suite.page_size = page;
            node::set_frame(suite.pages.0, page_x, 0.0, page.width, page.height);
            node::set_frame(suite.bar.0, bar.0, bar.1, bar.2, bar.3);
            suite
                .items
                .iter()
                .map(|(h, id)| {
                    node::set_size(h.0, page.width, page.height);
                    (*id, page)
                })
                .collect()
        });
        for (id, size) in reports {
            emit(id, Event::FrameChanged(size));
        }
    }

    /// Show destination `i` and hide the rest (the resident-page switch, docs/navigation.md).
    fn suite_select(host: usize, i: usize) {
        NAV_SUITES.with(|c| {
            let mut c = c.borrow_mut();
            let Some(suite) = c.get_mut(&host) else {
                return;
            };
            suite.selected = i;
            forget_themed(host);
            themed(
                host,
                suite.bar.0,
                Paint::Background,
                0xFFF1_F3F5,
                0xFF1C_1C1E,
            );
            for &(node, paint, index) in &suite.bar_inks {
                let (light, dark) = if index == i {
                    (0xFF00_7DFF, 0xFF3E_9BFF)
                } else {
                    (0x9900_0000, 0x99FF_FFFF)
                };
                themed(host, node, paint, light, dark);
            }
            for (n, (h, _)) in suite.items.iter().enumerate() {
                node::set_visibility(h.0, n == i);
            }
        });
    }

    /// Build a NAV_MENU: a scrollable column of conventional navigation rows (an optional
    /// leading icon, leading-aligned label, trailing chevron, hairline separators, the
    /// HarmonyOS settings-list idiom), not buttons. Each row's tap becomes a synthetic click
    /// that [`on_event`] translates to `SelectionChanged(index)` against `menu`.
    ///
    /// Icons (docs/vectors.md): a vector name resolves to its staged rawfile SVG
    /// (`day/<name>.svg`), which ArkUI renders natively and `NODE_IMAGE_FILL_COLOR` recolors:
    /// the row's tint when given, else a secondary theme foreground. A raster name falls
    /// back to `day/<name>.png`, drawn as authored (fill color has no effect on rasters).
    fn build_nav_menu(
        menu: NodeId,
        items: &[String],
        icons: &[Option<String>],
        tints: &[Option<day_spec::Color>],
        badge_icons: &[Option<String>],
        badge_tints: &[Option<day_spec::Color>],
        sections: &[Option<String>],
    ) -> AHandle {
        let scroll = new_node(node::SCROLL);
        let col = build_nav_menu_rows(
            menu,
            items,
            icons,
            tints,
            badge_icons,
            badge_tints,
            sections,
        );
        node::insert_child(scroll.0, col.0, 0);
        // The rows column is owned content: registering it here lets `NavMenuPatch::Items`
        // swap it wholesale and `release` dispose it with the scroll.
        SCROLL_CONTENT.with(|m| m.borrow_mut().insert(scroll.0 as usize, col.0 as usize));
        NAV_MENU_IDS.with(|m| m.borrow_mut().insert(scroll.0 as usize, menu));
        scroll
    }

    /// The rows column for a NAV_MENU (see [`build_nav_menu`]): registers one synthetic click
    /// id per row in [`MENU_ROWS`]. Rebuilt wholesale on `NavMenuPatch::Items`.
    ///
    /// A section title (docs/navigation.md) is a plain text node ahead of its group's first row.
    /// It registers no click id, so [`MENU_ROWS`] keeps Day's own row indices however many titles
    /// sit above a row, and the hairline a title replaces is left out.
    fn build_nav_menu_rows(
        menu: NodeId,
        items: &[String],
        icons: &[Option<String>],
        tints: &[Option<day_spec::Color>],
        badge_icons: &[Option<String>],
        badge_tints: &[Option<day_spec::Color>],
        sections: &[Option<String>],
    ) -> AHandle {
        let col = new_node(node::COLUMN);
        let owner = col.0 as usize;
        let mut pos = 0;
        for (i, title) in items.iter().enumerate() {
            if let Some(Some(section)) = sections.get(i) {
                let heading = new_node(node::TEXT);
                node::set_text(heading.0, section);
                node::set_font_size(heading.0, 14.0);
                themed(owner, heading.0, Paint::Font, 0x9900_0000, 0x99FF_FFFF);
                node::style_nav_heading(heading.0, pos == 0);
                node::insert_child(col.0, heading.0, pos);
                pos += 1;
            }
            // A Row (vertically centered children) carries the whole-row click target.
            let row = new_node(node::ROW);
            let label = new_node(node::TEXT);
            let chevron = new_node(node::TEXT);
            let synth = next_synth();
            MENU_ROWS.with(|m| m.borrow_mut().insert(synth, (menu, i as i64)));
            let mut child = 0;
            if let Some(Some(name)) = icons.get(i) {
                let icon = row_icon(owner, name, tints.get(i).copied().flatten(), 20.0);
                node::set_margin(icon.0, 4.0);
                node::insert_child(row.0, icon.0, child);
                child += 1;
            }
            node::set_text(label.0, title);
            node::set_font_size(label.0, 16.0);
            themed(owner, label.0, Paint::Font, TEXT_LIGHT, TEXT_DARK);
            node::set_flex_grow(label.0, 1.0);
            node::set_text(chevron.0, "\u{203a}");
            node::set_font_size(chevron.0, 20.0);
            themed(owner, chevron.0, Paint::Font, 0x4D00_0000, 0x66FF_FFFF);
            node::insert_child(row.0, label.0, child);
            // The trailing status glyph (docs/navigation.md), between the growing label and the
            // chevron so it sits at the row's end without displacing the disclosure arrow.
            let mut after_label = child + 1;
            if let Some(Some(name)) = badge_icons.get(i) {
                let badge = row_icon(owner, name, badge_tints.get(i).copied().flatten(), 16.0);
                node::set_margin(badge.0, 4.0);
                node::insert_child(row.0, badge.0, after_label);
                after_label += 1;
            }
            node::insert_child(row.0, chevron.0, after_label);
            node::style_row(row.0, 52.0);
            node::register_event(row.0, node::EV_CLICK, synth);
            node::insert_child(col.0, row.0, pos);
            pos += 1;
            if i + 1 < items.len() && !matches!(sections.get(i + 1), Some(Some(_))) {
                let sep = new_node(node::STACK);
                themed(owner, sep.0, Paint::Separator, 0x1400_0000, 0x24FF_FFFF);
                node::insert_child(col.0, sep.0, pos);
                pos += 1;
            }
        }
        col
    }

    thread_local! {
        /// `BitmapId` → what the decode reported (docs/images.md). Only the METADATA lives here;
        /// the pixels are the image module's, in a pixelmap registry under the same id.
        static BITMAP_INFO: RefCell<HashMap<u64, day_spec::BitmapInfo>> =
            RefCell::new(HashMap::new());
    }

    /// Re-encode a decoded bitmap (docs/images.md).
    ///
    /// A free function rather than the duty's body so each refusal reads as an early return
    /// instead of another copy of the same emit.
    fn encode_bitmap(
        id: day_spec::BitmapId,
        spec: &day_spec::EncodeSpec,
    ) -> Result<Vec<u8>, day_spec::ImageError> {
        use day_spec::ImageFormat as F;
        let mime = match spec.format {
            F::Png => "image/png",
            F::Jpeg => "image/jpeg",
            // The image packer writes nothing else here; `encode_formats` says so up front.
            _ => return Err(day_spec::ImageError::Encode),
        };
        if !BITMAP_INFO.with(|m| m.borrow().contains_key(&id.0)) {
            return Err(day_spec::ImageError::Gone);
        }
        // OpenHarmony takes quality as 0..=100; none means the format's own default.
        let quality = spec
            .quality
            .map(|q| (q.clamp(0.0, 1.0) * 100.0).round() as u32);
        crate::images::encode(id.0, mime, quality).ok_or(day_spec::ImageError::Encode)
    }

    /// Point an image node at an [`day_spec::ImageSource`] (docs/images.md): shared by realize
    /// and the `Source` patch, so a swap loads exactly what a fresh realize would have.
    ///
    /// Named is the staged-rawfile path: a vector's SVG first (docs/vectors.md), then the PNG,
    /// and `tint` recolors an SVG's paths. Bytes and Decoded need no file at all, and take no
    /// tint: there is no SVG to repaint.
    fn arkui_apply_image_source(
        n: Handle,
        source: &day_spec::ImageSource,
        tint: Option<day_spec::Color>,
    ) {
        match source {
            day_spec::ImageSource::Named(named) => {
                let svg = format!("day/{named}.svg");
                if crate::resources::rawfile_exists(&svg) {
                    node::set_image_src(n, &format!("resource://RAWFILE/{svg}"));
                    if let Some(t) = tint {
                        node::set_image_fill(n, argb(t));
                    }
                } else if !named.is_empty() {
                    node::set_image_src(n, &format!("resource://RAWFILE/day/{named}.png"));
                }
            }
            day_spec::ImageSource::Bytes(bytes) => crate::images::node_set_bytes(n, bytes),
            day_spec::ImageSource::Decoded(id) => crate::images::node_set_bitmap(n, id.0),
        }
    }

    pub fn emit(id: NodeId, ev: Event) {
        let sink = SINK.with(|s| s.borrow().clone());
        if let Some(sink) = sink {
            sink(id, ev);
        }
    }

    /// Emit `ev` on the next loop turn: for events produced inside a toolkit duty (which runs
    /// under the tree borrow, so a synchronous emit would re-enter it).
    fn post_emit(id: NodeId, ev: Event) {
        crate::main_thread::post_local(Box::new(move || emit(id, ev)));
    }

    /// Pieces whose component exists only in ArkTS (docs/extending.md).
    ///
    /// The ArkUI C node API has no node kind for the declarative `Web` or `Map` components, so a
    /// piece wrapping one ships an `.ets` (staged into the hvigor project by `day build`) that
    /// builds it in a `BuilderNode`; this module mounts that FrameNode in a native Stack and hands
    /// the Stack back as an [`AHandle`] Day mounts like any other node.
    ///
    /// The wrapper is what makes Day's layout reach the component. The C API refuses to set
    /// attributes on a BuilderNode-generated node (`ARKUI_ERROR_CODE_NOT_SUPPROTED_FOR_ARKTS_NODE`),
    /// so `set_frame` on the FrameNode itself was silently dropped: the component kept ArkUI's
    /// default layout, filling its parent from (0, 0) and covering the siblings Day had laid out
    /// around it (the web view over its URL bar). Day positions and sizes the wrapper, which it
    /// created, and the component fills it. `props`, `cmd`, and `arg` are opaque strings the piece
    /// defines; the bridge stays generic, so a new piece needs no host change.
    pub mod piece {
        use super::{AHandle, PIECE_NODES, node};
        use day_spec::NodeId;

        /// Build the ArkTS component registered for `kind`. A null handle means no ArkTS piece
        /// factory is registered (or it declined `kind`); the caller should fall back to Day's
        /// placeholder leaf, exactly as an unregistered renderer does.
        pub fn make(kind: day_spec::PieceKind, id: NodeId, props: &str) -> AHandle {
            try_make(kind, id, props).unwrap_or_else(|| {
                // No ArkTS module claimed the kind (or building it threw). Hand back a real empty
                // node, never a null handle: the tree mounts this like any leaf, and a null would
                // take its whole parent's layout down instead of leaving one blank rectangle.
                // Reported so `assert_no_placeholders` sees it, exactly like a missing renderer.
                day_spec::placeholder::report(kind, "arkui");
                super::new_node(node::STACK)
            })
        }

        /// [`make`], answering `None` where no ArkTS module claims `kind`, for a caller with a
        /// native fallback of its own (the menu-style picker keeps the wheel on a host too old
        /// to register its `Select`).
        pub fn try_make(kind: &str, id: NodeId, props: &str) -> Option<AHandle> {
            let h = crate::host_api::piece_make(kind, id.0, props);
            if h.is_null() {
                return None;
            }
            let wrapper = super::new_node(node::STACK);
            node::add_child(wrapper.0, h);
            // Remembered so `release` can send the ArkTS side its disposal, and so it knows
            // NOT to dispose the ArkTS-owned node itself.
            PIECE_NODES.with(|m| {
                m.borrow_mut()
                    .insert(wrapper.0 as usize, (id.0, h as usize))
            });
            Some(wrapper)
        }

        /// Whether `h` is an ArkTS-built piece node.
        pub(crate) fn is_piece(h: &AHandle) -> bool {
            PIECE_NODES.with(|m| m.borrow().contains_key(&(h.0 as usize)))
        }

        /// Send a command to a piece's ArkTS component. Takes the handle rather than the node id
        /// because that is what a `Renderer`'s `update` is handed; the id it was made with is
        /// remembered here. A handle that isn't an ArkTS piece node is a no-op.
        pub fn update(h: &AHandle, cmd: &str, arg: &str) {
            let Some((id, _)) = PIECE_NODES.with(|m| m.borrow().get(&(h.0 as usize)).copied())
            else {
                return;
            };
            crate::host_api::piece_update(id, cmd, arg);
        }
    }

    /// day `Color` (0..1 components) → ArkUI ARGB `u32`.
    fn argb(c: day_spec::Color) -> u32 {
        let f = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
        (f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b)
    }

    /// Semantic [`Font`] → a vp point size (ArkUI's default length unit is vp ≈ day points).
    /// Public for standalone pieces (docs/extending.md), which resolve the same scale.
    pub fn font_vp(f: FontSpec) -> f64 {
        match f.style {
            Font::LargeTitle => 34.0,
            Font::Title => 28.0,
            Font::Title2 => 22.0,
            Font::Title3 => 20.0,
            Font::Headline => 17.0,
            Font::Body => 17.0,
            Font::Callout => 16.0,
            Font::Subheadline => 15.0,
            Font::Footnote => 13.0,
            Font::Caption => 12.0,
            Font::Caption2 => 11.0,
            Font::System(pt) => pt,
            Font::Custom(_, pt) => pt,
        }
    }

    /// Apply a `Font::Custom` family (§18.4): the family was registered by the
    /// platform/harmony scaffold's EntryAbility (from rawfile `day/fonts.json`), so NODE_FONT_FAMILY resolves it
    /// by name; ArkUI falls back to the default family when it doesn't.
    fn apply_font_attrs(n: Handle, spec: FontSpec) {
        if let Font::Custom(family, _) = spec.style {
            node::set_font_family(n, family);
        }
        // Tabular figures. Set unconditionally (empty string clears it) so a label that stops
        // asking for them goes back to proportional on the next patch.
        node::set_font_feature(n, if spec.tabular { "tnum 1" } else { "" });
        // Weight and italic, also unconditional. An explicit weight wins; otherwise the style's
        // own (a headline is semibold, as on Android and Apple).
        let weight = spec.weight.unwrap_or(match spec.style {
            Font::Headline => day_spec::FontWeight::Semibold,
            _ => day_spec::FontWeight::Regular,
        });
        node::set_font_weight_style(n, i32::from(weight.css()), spec.italic);
    }

    /// Rebuild a label's SPAN children from its runs (docs/text-runs.md).
    ///
    /// ArkUI is the one backend where runs are child NODES rather than attributes on one widget: a
    /// styled Text is a small subtree. Day's own layout still treats the label as a leaf, because
    /// ArkUI measures the spans itself and reports the Text's size.
    fn set_label_runs(n: Handle, text: &str, runs: &[day_spec::TextRun]) {
        if runs.is_empty() {
            // Plain text goes back on the Text itself; `runs_begin` cleared it when runs arrived.
            node::set_text(n, text);
            return;
        }
        node::label_runs_begin(n);
        let add = |slice: &str, run: Option<&day_spec::TextRun>| {
            let mut style = node::RunStyle::default();
            if let Some(r) = run {
                // The span's size is absolute in ArkUI, so a relative scale multiplies against
                // the size this run's own style resolves to: the same `font_vp` ramp the label
                // itself uses.
                let base_fp = font_vp(r.font);
                let scale_permille = (r.font.scale * 1000.0).round() as i32;
                if scale_permille != 1000 && scale_permille > 0 && base_fp > 0.0 {
                    style.size_fp = Some(base_fp * f64::from(scale_permille) / 1000.0);
                }
                style.bold = r
                    .font
                    .weight
                    .is_some_and(|w| w >= day_spec::FontWeight::Semibold);
                style.italic = r.font.italic;
                style.monospace = r.font.monospace;
                style.strikethrough = r.strikethrough;
                style.underline = r.underline.is_on();
                style.color = r.color.map(argb);
                style.background = r.background.map(argb);
            }
            node::label_runs_add(n, slice, style);
        };
        let mut at = 0usize;
        for r in runs {
            let Some(styled) = text.get(r.range.clone()) else {
                continue;
            };
            if r.range.start > at
                && let Some(plain) = text.get(at..r.range.start)
            {
                add(plain, None);
            }
            add(styled, Some(r));
            at = r.range.end;
        }
        if let Some(tail) = text.get(at..) {
            add(tail, None);
        }
    }

    fn clear_button_content(n: AHandle) {
        if let Some(children) = BUTTON_CHILDREN.with(|m| m.borrow_mut().remove(&(n.0 as usize))) {
            if let Some(root) = children.first() {
                node::remove_child(n.0, root.0);
                for child in children.iter().skip(1) {
                    node::remove_child(root.0, child.0);
                }
            }
            for child in children.into_iter().rev() {
                node::dispose(child.0);
            }
        }
    }

    /// An icon as an image URI ArkUI loads: a standard symbol staged into the app's sandbox
    /// cache (a file URI, since NODE_IMAGE_SRC reads a bare path as the name of a bundled asset:
    /// "GetAsset failed"), or a bundled image from the app's rawfiles.
    fn icon_source(icon: &day_spec::Icon) -> Option<String> {
        match icon {
            day_spec::Icon::Symbol(s) => day_spec::resource::stage_symbol_svg(*s)
                .map(|p| format!("file://{}", p.to_string_lossy())),
            day_spec::Icon::Image(name) => {
                let vector = crate::resources::rawfile_exists(&format!("day/{name}.svg"));
                let raster = crate::resources::rawfile_exists(&format!("day/{name}.png"));
                (vector || raster).then(|| {
                    format!(
                        "resource://RAWFILE/day/{name}.{}",
                        if vector { "svg" } else { "png" }
                    )
                })
            }
        }
    }

    fn apply_button_content(
        n: AHandle,
        title: &str,
        icon: Option<&day_spec::Icon>,
        icon_only: bool,
    ) {
        clear_button_content(n);
        let source = icon.and_then(icon_source);
        node::set_a11y_text(n.0, title);
        node::set_button_label(n.0, if source.is_some() { "" } else { title });
        if let Some(source) = source {
            let ink = BUTTON_INK
                .with(|m| m.get(n.0 as usize))
                .unwrap_or(0xFFFF_FFFF);
            let row = new_node(node::ROW);
            let image = new_node(node::IMAGE);
            let mut children = vec![row, image];
            node::set_image_src(image.0, &source);
            node::set_image_fill(image.0, ink);
            node::set_size(image.0, 20.0, 20.0);
            node::insert_child(row.0, image.0, 0);
            if !icon_only {
                let label = new_node(node::TEXT);
                node::set_text(label.0, &format!("  {title}"));
                node::set_font_color(label.0, ink);
                node::insert_child(row.0, label.0, 1);
                children.push(label);
            }
            node::insert_child(n.0, row.0, 0);
            BUTTON_CHILDREN.with(|m| m.borrow_mut().insert(n.0 as usize, children));
        }
    }

    fn apply_button_style(n: Handle, style: day_spec::props::ButtonStyleSpec) {
        use day_spec::props::ButtonStyleSpec as S;
        let (fill, ink, border) = match style {
            S::Automatic | S::Compact => (0, 0xFF33_7DFF, 0.0),
            S::Bordered => (0, 0xFF33_7DFF, 1.0),
            S::Prominent => (0xFF33_7DFF, 0xFFFF_FFFF, 0.0),
            S::Tinted(c) => (argb(c), argb(S::on_tint(c)), 0.0),
        };
        BUTTON_INK.with(|m| {
            m.insert(n as usize, ink);
        });
        BUTTON_CHILDREN.with(|m| {
            if let Some(children) = m.borrow().get(&(n as usize)) {
                if let Some(image) = children.get(1) {
                    node::set_image_fill(image.0, ink);
                }
                if let Some(label) = children.get(2) {
                    node::set_font_color(label.0, ink);
                }
            }
        });
        // Reset all style attributes, including when changing away from a tint or border.
        // Keep the native button so press, focus, accessibility and disabled behavior survive.
        node::set_bg_color(n, fill);
        node::set_font_color(n, ink);
        node::set_button_border(n, border, ink);
    }

    fn new_node(kind: ohos_sys::arkui::native_node::ArkUI_NodeType) -> AHandle {
        AHandle(node::create(kind))
    }

    /// The ArkTS hosts for menu and segmented pickers (DaySelect.ets).
    const MENU_PICKER_KIND: &str = "day.picker.menu";
    const SEGMENTED_PICKER_KIND: &str = "day.picker.segmented";

    /// A menu picker's props and `options` command: the selected index (empty to keep the
    /// current one), then each option, 0x1F-separated (DaySelect.ets parses it). An option can't
    /// contain the separator, a control character no label carries; one that does loses it
    /// rather than splitting.
    fn select_props(selected: Option<usize>, options: &[String]) -> String {
        let mut out = selected.map(|i| i.to_string()).unwrap_or_default();
        for o in options {
            out.push('\u{1F}');
            out.extend(o.chars().filter(|c| *c != '\u{1F}'));
        }
        out
    }

    /// The application's color mode changed (docs/appearance.md): repaint every neutral paint
    /// for the new mode, then let `dark_mode()` readers recolor.
    fn appearance_changed(dark: bool) {
        repaint_themed(dark);
        day_core::note_appearance_changed();
    }

    /// The animation scale changed under the app (docs/accessibility.md): day-core re-reads
    /// `reduce_motion()` and re-gates its transitions.
    fn motion_changed() {
        day_core::note_motion_changed();
    }

    /// Move IS_DARK to `dark`, repainting every theme-following paint if it changed.
    fn repaint_themed(dark: bool) {
        if IS_DARK.with(|d| d.replace(dark)) == dark {
            return;
        }
        THEMED.with(|t| {
            for paints in t.borrow().values() {
                for (n, paint, light, dark_argb) in paints {
                    paint.apply(*n as Handle, if dark { *dark_argb } else { *light });
                }
            }
        });
    }

    /// Which attribute a theme-following paint sets.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Paint {
        Background,
        Font,
        ImageFill,
        Separator,
    }

    impl Paint {
        fn apply(self, n: Handle, argb: u32) {
            match self {
                Paint::Background => node::set_bg_color(n, argb),
                Paint::Font => node::set_font_color(n, argb),
                Paint::ImageFill => node::set_image_fill(n, argb),
                Paint::Separator => node::menu_separator(n, argb),
            }
        }
    }

    /// Paint `n` with the color mode's pick of `light`/`dark`, and repaint it whenever the
    /// mode changes (the C nodes don't re-theme an explicit color). `owner` is the node whose
    /// release or rebuild ends the paint ([`forget_themed`]): the node itself for a Day node, the
    /// rows column or suite host for the chrome this backend builds inside one.
    fn themed(owner: usize, n: Handle, paint: Paint, light: u32, dark: u32) {
        paint.apply(n, theme_color(light, dark));
        THEMED.with(|t| {
            let mut t = t.borrow_mut();
            let paints = t.entry(owner).or_default();
            paints.retain(|(p, k, _, _)| !(*p == n as usize && *k == paint));
            paints.push((n as usize, paint, light, dark));
        });
    }

    /// Stop repainting what `owner` holds: it is being released or rebuilt.
    fn forget_themed(owner: usize) {
        THEMED.with(|t| t.borrow_mut().remove(&owner));
    }

    /// Stop repainting one attribute of one node (an app color replaced the neutral one).
    fn forget_paint(owner: usize, n: Handle, paint: Paint) {
        THEMED.with(|t| {
            if let Some(paints) = t.borrow_mut().get_mut(&owner) {
                paints.retain(|(p, k, _, _)| !(*p == n as usize && *k == paint));
            }
        });
    }

    /// The primary text color a label without one of its own takes.
    const TEXT_LIGHT: u32 = 0xE500_0000;
    const TEXT_DARK: u32 = 0xE6FF_FFFF;

    /// The width of `text` in the Select's own face (16 vp, medium weight).
    fn select_label_width(text: &str) -> f64 {
        crate::fonts::measure_text(
            text,
            16.0,
            i32::from(day_spec::FontWeight::Medium.css()),
            false,
            "",
        )[0]
    }

    /// Size a freshly built menu picker for its widest option. The button's chrome (padding and
    /// arrow) is what its first measure reports beyond the label it shows; a Select sized to its
    /// current label would clip a longer choice until something else relaid the row.
    fn size_menu_picker(h: &AHandle, shown: Option<&str>, options: &[String]) {
        let (w, hh) = node::measure(h.0, 0.0, 0.0);
        let chrome = (w - shown.map_or(0.0, select_label_width)).max(40.0);
        let size = Size::new(chrome + widest_label(options), hh.max(40.0));
        MENU_PICKER_SIZE.with(|m| m.borrow_mut().insert(h.0 as usize, (Some(chrome), size)));
        piece::update(h, "width", &size.width.to_string());
    }

    /// Re-size a menu picker for new options.
    fn resize_menu_picker(h: &AHandle, options: &[String]) {
        let Some((chrome, size)) =
            MENU_PICKER_SIZE.with(|m| m.borrow().get(&(h.0 as usize)).copied())
        else {
            return;
        };
        let width = chrome.map_or_else(
            || segmented_width(options),
            |chrome| chrome + widest_label(options),
        );
        if (width - size.width).abs() >= 0.5 {
            let size = Size::new(width, size.height);
            MENU_PICKER_SIZE.with(|m| m.borrow_mut().insert(h.0 as usize, (chrome, size)));
            piece::update(h, "width", &width.to_string());
        }
    }

    fn segmented_width(options: &[String]) -> f64 {
        (widest_label(options) + 24.0).max(48.0) * options.len() as f64 + 8.0
    }

    fn size_segmented_picker(h: &AHandle, options: &[String]) {
        let size = Size::new(segmented_width(options), 40.0);
        MENU_PICKER_SIZE.with(|m| m.borrow_mut().insert(h.0 as usize, (None, size)));
        piece::update(h, "width", &size.width.to_string());
    }

    /// Enable or disable a menu or segmented picker. The C API cannot set attributes on the
    /// ArkTS-built Select or buttons, so DaySelect.ets applies `.enabled()` to each of them; the
    /// wrapper Day created carries NODE_ENABLED too, which is what `read_native` reports.
    fn set_picker_piece_enabled(h: &AHandle, enabled: bool) {
        node::set_enabled(h.0, enabled);
        piece::update(h, "enabled", if enabled { "1" } else { "0" });
    }

    fn widest_label(options: &[String]) -> f64 {
        options
            .iter()
            .map(|o| select_label_width(o))
            .fold(0.0, f64::max)
            .ceil()
    }

    /// The theme-adaptive pick: `light` under the light theme, `dark` under dark.
    fn theme_color(light: u32, dark: u32) -> u32 {
        if IS_DARK.with(|d| d.get()) {
            dark
        } else {
            light
        }
    }

    /// Set up the window root and density from the ArkTS host, before `launch_with`. Called by
    /// `day::arkui::start` (via the `day::day_start_arkui!` entry macro) with the `NodeContent`
    /// handle the host's `start` export received.
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `content` is a trusted NodeContent handle
    pub fn init(content: *mut std::ffi::c_void, w_vp: f64, h_vp: f64, density: f64) {
        // Reached from the ArkTS host's NAPI start call: contained like every FFI entry.
        day_spec::ffi_guard::contain((), || {
            node::set_density(density);
            let dark = match std::env::var("DAY_THEME").ok().as_deref() {
                Some("dark") => true,
                Some("light") => false,
                _ => std::env::var("DAY_ARKUI_DARK").ok().as_deref() == Some("1"),
            };
            IS_DARK.with(|d| d.set(dark));
            // Follow the color mode live (a system switch, or the app's own override coming
            // back): neutral paints branch on IS_DARK, and `dark_mode()` closures recolor.
            crate::host::watch_appearance(appearance_changed);
            // And the user's animation scale, the reduce-motion setting here; `launch_with`
            // does the first read once the tree stands.
            crate::host::watch_reduce_motion(motion_changed);
            // Serve bundled data resources (§18.3) from the app's rawfile store. Registered once
            // here; the opener is a no-op until the ArkTS host hands us its resourceManager.
            day_spec::resource::set_resource_opener(open_resource);
            // A Stack fills the window; day mounts its tree under it and positions children
            // absolutely.
            let root = new_node(node::STACK);
            node::set_frame(root.0, 0.0, 0.0, w_vp, h_vp);
            node::content_add(content.cast(), root.0);
            ROOT.with(|r| *r.borrow_mut() = Some((root, Size::new(w_vp, h_vp))));
            ROOT_KEEP.with(|r| r.set(Some((root.0 as usize, w_vp, h_vp))));
        });
    }

    pub(crate) fn cover_layer_init(content: ohos_sys::arkui::native_type::ArkUI_NodeContentHandle) {
        if content.is_null() || COVER_ROOT.with(|r| r.get().is_some()) {
            return;
        }
        let root = new_node(node::STACK);
        node::content_add(content, root.0);
        COVER_ROOT.with(|r| r.set(Some((root.0 as usize, 0.0, 0.0))));
    }

    pub(crate) fn cover_layer_resized(w: f64, h: f64) {
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        COVER_ROOT.with(|r| {
            if let Some((root, _, _)) = r.get() {
                r.set(Some((root, w, h)));
                node::set_frame(root as Handle, 0.0, 0.0, w, h);
            }
        });
        let covers: Vec<usize> = COVER_PRESENTED.with(|s| s.borrow().iter().copied().collect());
        for cover in covers {
            node::set_frame(cover as Handle, 0.0, 0.0, w, h);
            if let Some(id) = COVER_NODES.with(|m| m.borrow().get(&cover).copied()) {
                post_emit(NodeId(id), Event::FrameChanged(Size::new(w, h)));
            }
        }
    }

    /// Native back targets the top cover, leaving the underlying navigation path intact.
    pub(crate) fn cover_back_requested() -> bool {
        let Some((root, _, _)) = COVER_ROOT.with(|r| r.get()) else {
            return false;
        };
        let count = node::child_count(root as Handle);
        if count == 0 {
            return false;
        }
        let top = node::child_at(root as Handle, count as i32 - 1) as usize;
        let Some(id) = COVER_NODES.with(|m| m.borrow().get(&top).copied()) else {
            return false;
        };
        emit(
            NodeId(id),
            Event::NavBack {
                already_popped: false,
            },
        );
        true
    }

    /// A secondary DayWindowAbility's page connected (the host's `windowStart` export): mount a
    /// Stack into its NodeContent and complete the pending open (docs/windows.md). False = closed
    /// before connecting; the ability terminates itself.
    pub fn window_start(
        node_id: u64,
        content: ohos_sys::arkui::native_type::ArkUI_NodeContentHandle,
        w_vp: f64,
        h_vp: f64,
    ) -> bool {
        // Every host entry runs contained (day_spec::ffi_guard): a panic unwinding into the
        // runtime's frame is an abort, so a caught panic reports, runs the recovery hook, and
        // returns the arm's safe default instead.
        day_spec::ffi_guard::contain(false, || {
            let root = new_node(node::STACK);
            node::set_frame(root.0, 0.0, 0.0, w_vp, h_vp);
            node::content_add(content, root.0);
            SECONDARY.with(|s| s.borrow_mut().push((node_id, root.0 as usize)));
            let ok = day_core::finish_window_open(
                day_spec::NodeId(node_id),
                root.0 as day_spec::RawHandle,
                Size::new(w_vp, h_vp),
            );
            if !ok {
                SECONDARY.with(|s| s.borrow_mut().retain(|(n, _)| *n != node_id));
                node::dispose(root.0);
            }
            ok
        })
    }

    /// The secondary window's content area changed (freeform resize, rotation), in vp.
    pub fn window_resized(node_id: u64, w_vp: f64, h_vp: f64) {
        day_spec::ffi_guard::contain((), || {
            emit(
                day_spec::NodeId(node_id),
                Event::WindowResized(Size::new(w_vp, h_vp)),
            );
        });
    }

    /// The ability instance is going away (back, recents swipe, terminateSelf): confirm to
    /// day-core, which tears the window's subtree down.
    pub fn window_closed(node_id: u64) {
        day_spec::ffi_guard::contain((), || {
            SECONDARY.with(|s| s.borrow_mut().retain(|(n, _)| *n != node_id));
            emit(day_spec::NodeId(node_id), Event::WindowClosed);
        });
    }

    /// Foreground/background transitions of a secondary ability instance.
    pub fn window_focused(node_id: u64, active: bool) {
        day_spec::ffi_guard::contain((), || {
            emit(day_spec::NodeId(node_id), Event::WindowFocused(active));
        });
    }

    /// An app lifecycle phase from the entry ability (docs/lifecycle.md), coded in
    /// `day_spec::Lifecycle` order like the Android bridge: 2 DidBecomeActive, 3
    /// WillResignActive, 4 WillEnterForeground, 5 DidEnterBackground, 6 DidReceiveMemoryWarning,
    /// 7 WillTerminate, 8 DidExit. The launch phases are day-core's own. Delivered on the primary window
    /// node, where the tree turns it into the app's `on_lifecycle` handlers and pauses the
    /// frame clock across the background.
    pub fn lifecycle(code: i32) {
        use day_spec::Lifecycle::*;
        let phase = match code {
            2 => DidBecomeActive,
            3 => WillResignActive,
            4 => WillEnterForeground,
            5 => DidEnterBackground,
            6 => DidReceiveMemoryWarning,
            7 => WillTerminate,
            8 => DidExit,
            _ => return,
        };
        day_spec::ffi_guard::contain((), || {
            emit(day_spec::WINDOW_NODE, Event::Lifecycle(phase));
        });
    }

    /// The entry ability reports every phase (docs/lifecycle.md). `const` for
    /// `day::require_lifecycle!` compile-time guards.
    pub const fn lifecycle_supported(_phase: day_spec::Lifecycle) -> bool {
        true
    }

    /// The hilog sink for Day's logger (docs/logging.md): std's stderr goes nowhere in an
    /// OHOS ability, so the facade installs this at start, one already-formatted line per call.
    pub fn hilog_sink(level: log::Level, line: &str) {
        crate::hilog::print(level, line);
    }

    /// One switch's gate (see `TOGGLE_GATE`).
    struct ToggleGate {
        /// The switch's node handle, to repaint it after a stale event.
        handle: usize,
        /// The value day last wrote, or the user's last accepted change.
        written: bool,
        /// Whether the user touched or keyed the switch since `written` was set.
        armed: bool,
    }

    /// A text field's read-only state (see `INPUT_FIELDS`). ArkUI asks before each insertion
    /// and deletion once a field is `watched`, and the answer is no while `held` is set. A
    /// paste, a cut or an input method's composition can change the text without asking, so
    /// `held` also carries the text to put back when a change slips through.
    #[derive(Default)]
    struct InputField {
        /// The will-insert and will-delete events are registered. They stay for the node's
        /// life: a field that leaves read-only answers yes.
        watched: bool,
        /// The text a read-only field shows; `None` while the field takes edits.
        held: Option<String>,
    }

    /// Whether node `id` is a text field that refuses edits now.
    pub(crate) fn is_read_only(id: u64) -> bool {
        INPUT_FIELDS.with(|m| m.borrow().get(&id).is_some_and(|f| f.held.is_some()))
    }

    /// A change event from text input `n`: when node `id` is read-only, put its text back if
    /// the change got past the will-edit events, and say the event is not the app's to see.
    pub(crate) fn hold_read_only(n: Handle, id: u64, text: &str) -> bool {
        let held = INPUT_FIELDS.with(|m| m.borrow().get(&id).and_then(|f| f.held.clone()));
        match held {
            Some(held) => {
                if held != text {
                    node::set_input_text(n, &held);
                }
                true
            }
            None => false,
        }
    }

    /// User input reached node `id`: if it is a switch, its next change is the user's.
    pub(crate) fn arm_toggle(id: u64) {
        TOGGLE_GATE.with(|m| {
            if let Some(gate) = m.borrow_mut().get_mut(&id) {
                gate.armed = true;
            }
        });
    }

    /// Record a programmatic write to switch `id`, which disarms its gate.
    fn toggle_written(id: u64, on: bool) {
        TOGGLE_GATE.with(|m| {
            if let Some(gate) = m.borrow_mut().get_mut(&id) {
                gate.written = on;
                gate.armed = false;
            }
        });
    }

    /// The native event trampoline: `kind` is `day_spec::bridge::BridgeKind`, the same wire
    /// table as the Android bridge; `id` is the day NodeId the event was registered against.
    pub fn on_event(id: u64, kind: i32, num: f64, text: &str) {
        day_spec::ffi_guard::contain((), || on_event_inner(id, kind, num, text));
    }

    /// [`on_event`]'s C form, for pieces built against it (day-piece-camera reports its ArkTS
    /// surface through it): `text` is a C string or null.
    #[unsafe(no_mangle)]
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `text` is a valid C string or null, per the caller
    pub extern "C" fn day_arkui_on_event(
        id: u64,
        kind: std::ffi::c_int,
        num: f64,
        text: *const std::ffi::c_char,
    ) {
        on_event(id, kind, num, &node::text_of(text));
    }

    fn on_event_inner(id: u64, kind: i32, num: f64, text: &str) {
        use day_spec::bridge::BridgeKind as K;
        if kind == K::Custom as i32 && text == "day-list-first-visible" {
            let source = LIST_SOURCES.with(|map| map.borrow().get(&id).cloned());
            if let Some(source) = source
                && let Some(report) = &source.first_visible
            {
                report((num.max(0.0) as usize).min((source.len)().saturating_sub(1)));
            }
            return;
        }
        // A NAV_MENU row click arrives with a synthetic id: translate it to a SelectionChanged
        // against the menu host before the normal per-node dispatch.
        if kind == K::Pressed as i32
            && let Some((menu, index)) = MENU_ROWS.with(|m| m.borrow().get(&id).copied())
        {
            emit(menu, Event::SelectionChanged(index));
            return;
        }
        // A node with a registered Tap gesture emits `Event::Tap`, not `Event::Pressed`.
        if kind == K::Pressed as i32 && TAP_NODES.with(|s| s.borrow().contains(&id)) {
            emit(NodeId(id), Event::Tap(Point::ZERO));
            return;
        }
        let node_id = NodeId(id);
        let ev = match kind {
            k if k == K::Pressed as i32 => Event::Pressed,
            // SelectionChanged (swiper tab / menu row), carried as the index in `num`.
            k if k == K::SelectionChanged as i32 => Event::SelectionChanged(num as i64),
            k if k == K::ListActivated as i32 => Event::ListActivated(num as usize),
            k if k == K::TextChanged as i32 => {
                // The programmatic-set echo (see TEXT_ECHO): a change carrying exactly what
                // day just wrote is ArkUI reporting the set back, not the user typing.
                let is_echo =
                    TEXT_ECHO.with(|m| m.borrow().get(&id).is_some_and(|last| *last == text));
                if is_echo {
                    return;
                }
                TEXT_ECHO.with(|m| m.borrow_mut().remove(&id));
                Event::TextChanged(text.to_owned())
            }
            k if k == K::ToggleChanged as i32 => {
                let on = num != 0.0;
                // The gate (see TOGGLE_GATE): `Some(handle)` is a stale event to repaint over.
                let stale = TOGGLE_GATE.with(|m| {
                    let mut m = m.borrow_mut();
                    let Some(gate) = m.get_mut(&id) else {
                        return Some(None);
                    };
                    if gate.written == on {
                        // The echo of day's own write, or no change at all.
                        return None;
                    }
                    if gate.armed {
                        gate.written = on;
                        gate.armed = false;
                        return Some(None);
                    }
                    Some(Some((gate.handle, gate.written)))
                });
                match stale {
                    None => return,
                    Some(Some((handle, written))) => {
                        // Out of the borrow: the set below may report back synchronously.
                        node::set_toggle(handle as node::Handle, written);
                        return;
                    }
                    Some(None) => Event::ToggleChanged(on),
                }
            }
            // Focus pair + text-input submit (docs/focus.md).
            k if k == K::FocusChanged as i32 => Event::FocusChanged(num != 0.0),
            k if k == K::Submitted as i32 => Event::Submitted,
            // A non-text key from a focused node (docs/menus.md): `text` is the day key name,
            // `num` the modifier mask. The receiver already asked whether this node claims keys.
            k if k == K::Key as i32 => Event::Key(day_spec::KeyEvent {
                key: text.to_owned(),
                modifiers: num as u8,
            }),
            // Pan/drag gesture (docs/shapes.md): `num` = phase (1 began, 2 changed, 3 ended),
            // `text` = "x,y,tx,ty" in px, converted to vp like the Android bridge.
            k if k == K::Gesture as i32 => {
                let p: Vec<f64> = text.split(',').filter_map(|s| s.parse().ok()).collect();
                if p.len() < 4 {
                    return;
                }
                let d = node::density();
                let at = Point::new(p[0] / d, p[1] / d);
                let tr = Point::new(p[2] / d, p[3] / d);
                match num as i32 {
                    1 => Event::Drag {
                        phase: day_spec::DragPhase::Began,
                        location: at,
                        translation: Point::ZERO,
                    },
                    3 => Event::Drag {
                        phase: day_spec::DragPhase::Ended,
                        location: at,
                        translation: tr,
                    },
                    _ => Event::Drag {
                        phase: day_spec::DragPhase::Changed,
                        location: at,
                        translation: tr,
                    },
                }
            }
            k if k == K::ValueChanged as i32 || k == K::ValueCommitted as i32 => {
                // ArkUI slider reports 0..100; map back to the node's day range. ValueCommitted
                // is the same value once the interaction settled.
                let (min, max) = SLIDER_RANGE
                    .with(|m| m.borrow().get(&id).copied())
                    .unwrap_or((0.0, 1.0));
                let value = min + (num / 100.0) * (max - min);
                // The programmatic-set echo (see SLIDER_ECHO), compared with the slack the
                // percent round-trip costs.
                let eps = ((max - min).abs()).max(1e-9) * 1e-4;
                let is_echo = SLIDER_ECHO
                    .with(|m| m.borrow().get(&id).copied())
                    .is_some_and(|last| (last - value).abs() <= eps);
                if is_echo {
                    return;
                }
                SLIDER_ECHO.with(|m| m.borrow_mut().remove(&id));
                if kind == K::ValueCommitted as i32 {
                    Event::ValueCommitted(value)
                } else {
                    Event::ValueChanged(value)
                }
            }
            // An ArkTS-built piece component reporting back (docs/extending.md), through the
            // host's `pieceEvent`. Like the Android bridge's Custom, the payload IS the event:
            // a cross-boundary Custom carries no tag, and the piece owns the whole channel.
            k if k == K::Custom as i32 => Event::Custom {
                tag: "",
                num,
                text: text.to_owned(),
            },
            // File-picker answer (docs/files.md): `id` is the request id, `text` the chosen local
            // path (a cache copy for open, a docs URI for save); empty means the user cancelled.
            k if k == K::PresentFile as i32 => {
                let result = day_spec::present::PresentResult::decode(3, 0, text.to_owned());
                emit(node_id, Event::PresentResult { req: id, result });
                return;
            }
            _ => return,
        };
        emit(node_id, ev);
    }

    /// Recycling-list row count, asked by the NodeAdapter (docs/list.md).
    pub(crate) fn list_count(host_id: u64) -> u32 {
        day_spec::ffi_guard::contain(0, || {
            LIST_SOURCES.with(|m| {
                m.borrow()
                    .get(&host_id)
                    .map(|s| (s.len)() as u32)
                    .unwrap_or(0)
            })
        })
    }

    /// Build (or rebind) row `index`'s content into the native cell `cell` (an inner Stack). The
    /// adapter reuses cells, so a repeat `cell` pointer is a rebind (day-core keys its cell cache
    /// by the raw handle). Called on the JS/main thread from the adapter's add callback.
    pub(crate) fn list_bind(host_id: u64, index: u32, cell: Handle) {
        day_spec::ffi_guard::contain((), || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            if let Some(source) = source {
                (source.bind_row)(index as usize, cell as day_spec::RawHandle);
            }
        });
    }

    /// A pooled cell left the adapter's visible set: clear the cell subtree's dayscript ids so
    /// hidden rows stop answering lookups (day-core's `list_recycle_cell`), keyed by the same
    /// inner-Stack pointer `list_bind` binds with.
    pub(crate) fn list_recycle(host_id: u64, cell: Handle) {
        day_spec::ffi_guard::contain((), || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            if let Some(source) = source {
                (source.recycle)(cell as day_spec::RawHandle);
            }
        });
    }

    /// Whether row `index` is in the list's programmatic selection: newly bound cells are
    /// painted from this (docs/list.md `ListPatch::Selected`).
    pub(crate) fn list_is_selected(host_id: u64, index: u32) -> bool {
        day_spec::ffi_guard::contain(false, || {
            LIST_SELECTED.with(|m| {
                m.borrow()
                    .get(&host_id)
                    .is_some_and(|set| set.contains(&(index as usize)))
            })
        })
    }

    /// The reorder guard's verdict for a hovered drop (docs/list.md): the accepted target index,
    /// or -1. Called synchronously from the list's drop handler; the source is cloned out before
    /// the app's guard runs, so no thread-local borrow is held.
    pub(crate) fn list_can_move(host_id: u64, from: u32, to: u32) -> i32 {
        day_spec::ffi_guard::contain(-1, || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(source) = source else { return -1 };
            let Some(r) = source.reorder.as_ref() else {
                return -1;
            };
            let len = (source.len)();
            let (from, to) = (from as usize, to as usize);
            if from >= len || to >= len {
                return -1;
            }
            ((r.can_move)(from, to) as i32).min(len.saturating_sub(1) as i32)
        })
    }

    /// Commit an accepted drop through the sync seam (rotates day's snapshot, defers the app
    /// callback); the list reloads the adapter afterwards.
    pub(crate) fn list_move(host_id: u64, from: u32, to: u32) {
        day_spec::ffi_guard::contain((), || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(r) = source.and_then(|s| s.reorder) else {
                return;
            };
            if from != to {
                (r.move_row)(from as usize, to as usize);
            }
        });
    }

    /// May this row be swiped away? A guarded row is refused at delete time (docs/list.md).
    pub(crate) fn list_can_delete(host_id: u64, index: u32) -> bool {
        day_spec::ffi_guard::contain(false, || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(source) = source else { return false };
            let Some(d) = source.delete.as_ref() else {
                return false;
            };
            let index = index as usize;
            index < (source.len)() && (d.can_delete)(index)
        })
    }

    /// Commit a swipe-to-delete through the sync seam (shortens day's snapshot, defers the app
    /// callback); the list reloads the adapter afterwards. True on commit.
    pub(crate) fn list_delete(host_id: u64, index: u32) -> bool {
        day_spec::ffi_guard::contain(false, || {
            if !list_can_delete(host_id, index) {
                return false;
            }
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(d) = source.and_then(|s| s.delete) else {
                return false;
            };
            (d.delete_row)(index as usize);
            true
        })
    }

    /// A NavDestination disappeared on the ArkTS side (docs/navigation.md). For a pop DAY
    /// initiated (NavPatch::Popped) this is just the acknowledgment; for a NATIVE back
    /// (system gesture / title-bar back button) sync the route state: the toolkit already
    /// popped, so the host receives `NavBack { already_popped: true }`.
    pub fn nav_popped(owner: u64, key: u64) {
        day_spec::ffi_guard::contain((), || nav_popped_inner(owner, key));
    }

    /// An adaptive host's Navigation decided its presentation (docs/size-classes.md): the
    /// toolkit tells Day (`Cap::NavRepresent = Emulated`), and the pieces layer reconciles its
    /// model to what ArkUI already drew, the way it does for `SlidingPaneLayout` on Android.
    pub fn nav_presented(owner: u64, split: bool) {
        use day_spec::props::NavPresentation;
        day_spec::ffi_guard::contain((), || {
            let next = if split {
                NavPresentation::Split
            } else {
                NavPresentation::Stack
            };
            emit(day_spec::NodeId(owner), Event::NavPresentationChanged(next));
        });
    }

    fn nav_popped_inner(owner: u64, key: u64) {
        let host = NAV_OWNERS.with(|m| m.borrow().get(&key).copied());
        let Some(host) =
            host.filter(|h| NAV_HOSTS.with(|m| m.borrow().get(h).copied()) == Some(owner))
        else {
            return;
        };
        // The destination's content tree is gone: if the page is still mounted (its Remove
        // patch hasn't landed yet), mark the key so that Remove skips the dead slot.
        if NAV_PUSHED.with(|m| m.borrow().values().any(|k| *k == key)) {
            NAV_POPPED_KEYS.with(|s| s.borrow_mut().insert(key));
        }
        // A destination that never landed can no longer be waited on.
        NAV_PENDING_PUSH.with(|s| {
            s.borrow_mut().remove(&key);
        });
        // Day-initiated pops retired their key from NAV_STACK already; a NATIVE back is the
        // toolkit popping on its own, so drop the key here (keeping Rust's order in sync
        // before the NavBack sync writes the pop into the route state).
        NAV_STACK.with(|s| {
            if let Some(stack) = s.borrow_mut().get_mut(&host) {
                stack.retain(|k| *k != key);
            }
        });
        let expected = NAV_EXPECT_POP.with(|e| e.borrow_mut().remove(&key));
        if expected {
            // The acknowledgment of a Day-initiated pop (`ui_idle`'s pending-pop signal).
            NAV_PENDING_POP.with(|p| {
                p.borrow_mut().remove(&key);
            });
        }
        if !NAV_PAGE_IDS.with(|m| m.borrow().values().any(|id| *id == key)) {
            NAV_OWNERS.with(|m| m.borrow_mut().remove(&key));
        }
        if !expected {
            emit(
                NodeId(owner),
                Event::NavBack {
                    already_popped: true,
                },
            );
        }
    }

    /// A guarded NavDestination consumed its back (ArkTS onBackPressed) and asks Day's guard to
    /// decide: emit `NavBack { already_popped: false }` (the native stack did NOT pop, unlike an
    /// unguarded back's [`nav_popped`]).
    pub fn nav_back_requested(owner: u64) {
        day_spec::ffi_guard::contain((), || {
            if NAV_HOSTS.with(|m| m.borrow().values().any(|id| *id == owner)) {
                emit(
                    NodeId(owner),
                    Event::NavBack {
                        already_popped: false,
                    },
                );
            }
        });
    }

    /// A destination's content area changed (vp): relayout that page in its real bounds. The
    /// first report for a key is also the push-landed signal `ui_idle` waits on.
    pub fn nav_area(key: u64, w: f64, h: f64) {
        day_spec::ffi_guard::contain((), || {
            if w > 0.0 && h > 0.0 {
                NAV_PENDING_PUSH.with(|s| {
                    s.borrow_mut().remove(&key);
                });
                if let Some(ptr) = NAV_PAGE_IDS.with(|m| {
                    m.borrow()
                        .iter()
                        .find(|(_, id)| **id == key)
                        .map(|(p, _)| *p)
                }) {
                    node::set_frame(ptr as Handle, 0.0, 0.0, w, h);
                }
                emit(NodeId(key), Event::FrameChanged(Size::new(w, h)));
            }
        });
    }

    /// A title-bar action was tapped (docs/toolbars.md). The bar's menu items carry only an
    /// action id and an optional segment index. A toggle flips; an explicit choice selects
    /// that segment (the legacy action-only path cycles); other items run their command.
    /// Recheck enablement and index bounds because an open popup may outlive a model patch.
    pub fn nav_menu_action(action: u64, selection: i32) {
        day_spec::ffi_guard::contain((), || {
            use day_spec::{ToolbarItemKind as K, ToolbarValue as V};
            let item = WINDOW_BAR.with(|b| b.borrow().by_action(action).cloned());
            if item.as_ref().is_some_and(|i| !i.enabled) {
                return;
            }
            if selection >= 0 {
                if let Some(day_spec::ToolbarItem {
                    kind: K::Segmented { segments, .. },
                    ..
                }) = item
                    && (selection as usize) < segments.len()
                {
                    emit(
                        day_spec::WINDOW_NODE,
                        Event::ToolbarChanged {
                            action,
                            value: V::Selected(selection as usize),
                        },
                    );
                }
                return;
            }
            let ev = match item.map(|i| i.kind) {
                Some(K::Toggle { on }) => Event::ToolbarChanged {
                    action,
                    value: V::On(!on),
                },
                Some(K::Segmented { segments, selected }) if !segments.is_empty() => {
                    Event::ToolbarChanged {
                        action,
                        value: V::Selected((selected + 1) % segments.len()),
                    }
                }
                _ => Event::MenuAction(action),
            };
            emit(day_spec::WINDOW_NODE, ev);
        });
    }

    /// The user edited the navigation surface's search field (docs/search.md): reported against
    /// the nav host, where the `.searchable()` surface listens, whatever its placement asked for.
    pub fn nav_search_changed(owner: u64, text: &str) {
        day_spec::ffi_guard::contain((), || {
            if NAV_HOSTS.with(|m| m.borrow().values().any(|id| *id == owner)) {
                emit(NodeId(owner), Event::SearchChanged(text.to_owned()));
            }
        });
    }

    /// Paint the window toolbar onto the Navigation's title bars (docs/toolbars.md): one
    /// `.menus()` item per action, shown on the root page and every pushed page alike. The bar
    /// uses native buttons and popup menus through Navigation's custom builder. Page actions
    /// precede window actions. The host tints template icons using system colors, preserves
    /// toggle state, and exposes segment choices rather than silently cycling on a tap.
    /// A pull-down's entries remain flattened; separators, labels and search draw nothing here.
    /// Wire format: five newline-separated parallel fields. Segment addresses are `id:index`
    /// and their label field is `current title` followed by U+001F-separated choices. Enabled
    /// carries bit 0 = enabled, bit 1 = checked. User text is stripped of wire separators.
    fn paint_window_bar() {
        use day_spec::ToolbarItemKind as K;
        let (mut icons, mut labels, mut actions, mut enabled) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut scopes = Vec::new();
        WINDOW_BAR.with(|b| {
            // No sidebar toggle here: Navigation's own nav bar shows and hides itself.
            let bar = b.borrow();
            let (page, window): (Vec<_>, Vec<_>) = bar
                .items()
                .iter()
                .filter(|i| i.id != day_spec::SIDEBAR_TOGGLE_ID)
                .partition(|i| {
                    matches!(
                        i.column,
                        day_spec::ToolbarColumn::Detail | day_spec::ToolbarColumn::List
                    )
                });
            for item in page.into_iter().chain(window) {
                // Which title bars carry the item (docs/toolbars.md): a detail or list
                // column's over its page (1), the sidebar column's over the root page (0),
                // the window's over both (2). Stacked, only one bar shows at a time, so this
                // changes nothing there; tiled, both show, and each column's commands sit
                // over that column, the way a desktop toolbar spans its panes.
                let scope = match item.column {
                    day_spec::ToolbarColumn::Detail | day_spec::ToolbarColumn::List => "1",
                    day_spec::ToolbarColumn::Sidebar => "0",
                    day_spec::ToolbarColumn::Window => "2",
                };
                let mut add =
                    |icon: Option<&day_spec::Icon>, label: &str, action: u64, on: bool| {
                        if action == 0 {
                            return;
                        }
                        icons.push(icon.and_then(icon_source).unwrap_or_default());
                        let clean = |text: &str| text.replace(['\n', '\u{1f}'], " ");
                        if let K::Segmented { segments, selected } = &item.kind {
                            let mut titles = vec![clean(label)];
                            titles.extend(segments.iter().map(|s| clean(&s.title)));
                            labels.push(titles.join("\u{1f}"));
                            actions.push(format!("{action}:{selected}"));
                        } else {
                            labels.push(clean(label));
                            actions.push(action.to_string());
                        }
                        // Bit 0: enabled; bit 1: checked. Preserve disabled checked state too.
                        let checked = matches!(item.kind, K::Toggle { on: true });
                        enabled.push((u8::from(on) | (u8::from(checked) << 1)).to_string());
                        scopes.push(scope);
                    };
                match &item.kind {
                    K::Button | K::Toggle { .. } => {
                        add(item.icon.as_ref(), &item.label, item.action, item.enabled)
                    }
                    // The choice in force names the control and lends it its glyph, the way a
                    // Material bar folds one (day-android).
                    K::Segmented { segments, selected } => {
                        let seg = segments.get(*selected);
                        add(
                            seg.and_then(|s| s.icon.as_ref()).or(item.icon.as_ref()),
                            seg.map(|s| s.title.as_str()).unwrap_or(&item.label),
                            item.action,
                            item.enabled,
                        )
                    }
                    K::Menu { items } => {
                        for entry in items {
                            if let day_spec::MenuItem::Action {
                                label,
                                action,
                                enabled: on,
                                ..
                            } = entry
                            {
                                add(None, label, *action, *on && item.enabled);
                            }
                        }
                    }
                    K::Search { .. } | K::Label | K::Separator => {}
                }
            }
        });
        crate::host_api::nav_set_menu(
            &icons.join("\n"),
            &labels.join("\n"),
            &actions.join("\n"),
            &scopes.join("\n"),
            &enabled.join("\n"),
        );
    }

    /// Whether `h` is a secondary window's root (a DayWindowAbility page). The title-bar actions
    /// belong to the primary window's `Navigation`; a secondary page has no such bar, so its
    /// toolbar is not drawn, and its edits must not land in the primary's model (docs/toolbars.md).
    fn is_secondary_root(h: &AHandle) -> bool {
        let ptr = h.0 as usize;
        SECONDARY.with(|s| s.borrow().iter().any(|(_, stack)| *stack == ptr))
    }

    /// What a node shows, read back from ArkUI's own attributes (`Toolkit::read_native`). Only
    /// what is read is reported; the accessibility group stays unread (`found: false`), since
    /// ArkUI reports a component's default announcement nowhere a getter reaches.
    fn read_native(n: Handle) -> day_spec::NativeSnapshot {
        use ohos_sys::arkui::native_node::ArkUI_NodeAttributeType as Attr;
        use ohos_sys::arkui::native_type::{ArkUI_TextInputType as Type, ArkUI_Visibility};
        let mut snap = day_spec::NativeSnapshot::default();
        if n.is_null() {
            return snap;
        }
        // An item without a string is an unread attribute, not an empty one.
        let string = |n: Handle, attr: Attr| {
            node::get_item(n, attr)
                .filter(|it| !it.string.is_null())
                .map(|it| node::text_of(it.string))
        };
        match node::node_type(n) {
            Some(node::TEXT) => {
                // A label with runs keeps its text in SPAN children (`set_label_runs`), the
                // Text's own content cleared.
                let own = string(n, Attr::NODE_TEXT_CONTENT);
                let count = node::child_count(n);
                snap.text = match own {
                    Some(own) if own.is_empty() && count > 0 => (0..count as i32)
                        .map(|i| string(node::child_at(n, i), Attr::NODE_SPAN_CONTENT))
                        .collect::<Option<String>>(),
                    own => own,
                };
            }
            Some(node::BUTTON) => {
                // An icon button's title is a Text beside the image (`apply_button_content`),
                // padded with two spaces; an icon-only one shows no text.
                let content = BUTTON_CHILDREN.with(|m| {
                    m.borrow()
                        .get(&(n as usize))
                        .map(|children| children.get(2).map(|label| label.0))
                });
                snap.text = match content {
                    Some(Some(label)) => string(label, Attr::NODE_TEXT_CONTENT)
                        .map(|s| s.strip_prefix("  ").map(str::to_owned).unwrap_or(s)),
                    Some(None) => None,
                    None => string(n, Attr::NODE_BUTTON_LABEL),
                };
            }
            Some(node::TEXT_INPUT) => {
                // A password type masks the text: nothing is reported, nor for an unread type.
                let masked = node::get_i32(n, Attr::NODE_TEXT_INPUT_TYPE, 0).is_none_or(|t| {
                    [
                        Type::ARKUI_TEXTINPUT_TYPE_PASSWORD,
                        Type::ARKUI_TEXTINPUT_TYPE_NUMBER_PASSWORD,
                        Type::ARKUI_TEXTINPUT_TYPE_NEW_PASSWORD,
                    ]
                    .iter()
                    .any(|ty| ty.0 as i32 == t)
                });
                if !masked {
                    snap.text = string(n, Attr::NODE_TEXT_INPUT_TEXT);
                }
            }
            Some(node::TEXT_AREA) => snap.text = string(n, Attr::NODE_TEXT_AREA_TEXT),
            Some(node::TOGGLE) => {
                snap.checked = node::get_i32(n, Attr::NODE_TOGGLE_VALUE, 0).map(|v| v != 0);
            }
            Some(node::SLIDER) => {
                // ArkUI's 0..100, mapped back onto the range Day gave (`normalize`).
                let range = CTRL_NODE
                    .with(|m| m.borrow().get(&(n as usize)).copied())
                    .and_then(|id| SLIDER_RANGE.with(|m| m.borrow().get(&id).copied()));
                snap.number = node::get_f32(n, Attr::NODE_SLIDER_VALUE, 0)
                    .zip(range)
                    .map(|(v, (min, max))| min + f64::from(v) / 100.0 * (max - min));
            }
            Some(node::TEXT_PICKER) => {
                // The wheel's option at its selected index, split the way `set_picker` joined
                // the range. A menu or segmented picker is an ArkTS piece, out of a getter's reach.
                // The index slot is a u32, read as i32: the same bits for any real index.
                let selected = node::get_i32(n, Attr::NODE_TEXT_PICKER_OPTION_SELECTED, 0)
                    .and_then(|i| usize::try_from(i).ok());
                snap.text = string(n, Attr::NODE_TEXT_PICKER_OPTION_RANGE)
                    .zip(selected)
                    .and_then(|(range, i)| range.split(';').nth(i).map(str::to_owned));
            }
            Some(node::PROGRESS) => {
                // The fraction over the total `set_progress` wrote; a spinner has no value.
                let total = node::get_f32(n, Attr::NODE_PROGRESS_TOTAL, 0).filter(|t| *t > 0.0);
                snap.number = node::get_f32(n, Attr::NODE_PROGRESS_VALUE, 0)
                    .zip(total)
                    .map(|(v, total)| f64::from(v) / f64::from(total));
            }
            _ => {}
        }
        snap.enabled = node::get_i32(n, Attr::NODE_ENABLED, 0).map(|v| v != 0);

        // Walk the C-API ancestors up to a window root: the primary root, the cover layer above
        // it, or a secondary window's. The walk stops short of one where a node sits in an
        // ArkTS host (a Navigation page, a piece), whose own visibility is out of reach.
        let primary = ROOT.with(|r| r.borrow().as_ref().map(|(h, _)| h.0 as usize));
        let cover = COVER_ROOT.with(|r| r.get()).map(|(p, _, _)| p);
        let secondaries: Vec<usize> =
            SECONDARY.with(|s| s.borrow().iter().map(|(_, p)| *p).collect());
        let visible = ArkUI_Visibility::ARKUI_VISIBILITY_VISIBLE.0 as i32;
        let (mut hidden, mut unread, mut window) = (false, false, None);
        let mut at = n;
        // Bounded: a tree is never this deep, and a cycle must not hang the reader.
        for _ in 0..4096 {
            if at.is_null() {
                break;
            }
            match node::get_i32(at, Attr::NODE_VISIBILITY, 0) {
                Some(v) if v != visible => hidden = true,
                Some(_) => {}
                None => unread = true,
            }
            let ptr = at as usize;
            if Some(ptr) == primary || Some(ptr) == cover {
                window = primary;
                break;
            }
            if secondaries.contains(&ptr) {
                window = Some(ptr);
                break;
            }
            at = node::parent(at);
        }
        snap.visible = if hidden {
            Some(false)
        } else if window.is_some() && !unread {
            Some(true)
        } else {
            None
        };

        // The frame against its window's root, Day's content area there. A node the walk did
        // not place still has one window to be in while no secondary is open.
        let base = window.or_else(|| primary.filter(|_| secondaries.is_empty()));
        if let Some(base) = base
            && let Some((x, y, w, h)) = node::layout_frame_px(n)
            && let Some((bx, by, _, _)) = node::layout_frame_px(base as Handle)
        {
            let d = node::density();
            snap.frame = Some(Rect::new((x - bx) / d, (y - by) / d, w / d, h / d));
        }
        snap
    }

    /// The ArkTS host reports a ROOT area change after start (keyboard RESIZE avoidance,
    /// rotation, window resize), routed to Day as a window resize, the shared rail
    /// (docs/focus.md; same shape as Android's kind-15 event).
    pub fn resized(w: f64, h: f64) {
        day_spec::ffi_guard::contain((), || {
            if w > 0.0 && h > 0.0 {
                ROOT_KEEP.with(|r| {
                    if let Some((ptr, _, _)) = r.get() {
                        r.set(Some((ptr, w, h)));
                    }
                });
                emit(day_spec::WINDOW_NODE, Event::WindowResized(Size::new(w, h)));
            }
        });
    }

    /// The permission seam for `day-part-permissions` (docs/permissions.md): the part reaches this
    /// by `dlsym` rather than a link-time dependency on this toolkit, and it forwards to the
    /// host's ArkTS-registered prompter. 1 when the request went out (`cb` then runs on the JS
    /// thread with the request id and a bit mask of the grants), 0 when no prompter is
    /// registered, in which case `cb` is never called.
    #[unsafe(no_mangle)]
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `names` is a valid C string from the part
    pub extern "C" fn day_arkui_request_permissions(
        req: u64,
        names: *const std::ffi::c_char,
        cb: extern "C" fn(u64, u64),
    ) -> std::ffi::c_int {
        day_spec::ffi_guard::contain(0, || {
            let names = node::text_of(names);
            std::ffi::c_int::from(crate::host_api::request_permissions(req, &names, cb))
        })
    }

    /// daybridge's door to an ArkTS arm (docs/bridge.md "Callbacks"): `day_bridge::arkts::invoke`
    /// finds this by `dlsym` rather than a link-time dependency on this toolkit, and it runs the
    /// registered function on the JS thread.
    #[unsafe(no_mangle)]
    #[allow(clippy::not_unsafe_ptr_arg_deref)] // `symbol`/`args` are valid for the call, per the bridge
    pub extern "C" fn day_arkui_bridge_invoke(
        symbol: *const std::ffi::c_char,
        args: *const std::ffi::c_void,
        n: usize,
        done: u64,
        ret: *mut std::ffi::c_void,
    ) -> std::ffi::c_int {
        day_spec::ffi_guard::contain(2, || {
            // SAFETY: the bridge passes `n` `Arg`s of the layout `bridge::Arg` mirrors.
            unsafe { crate::bridge::invoke(symbol, args.cast(), n, done, ret.cast()) }
        })
    }

    /// Whether the caller is the JS thread, the UI thread of a Day app here, where a bridged
    /// crate's blocking call cannot wait for an ArkTS answer.
    #[unsafe(no_mangle)]
    pub extern "C" fn day_arkui_bridge_on_js_thread() -> std::ffi::c_int {
        std::ffi::c_int::from(crate::main_thread::on_js_thread())
    }

    /// The ArkTS host reports the app cache dir here (docs/files.md); it's the app-writable staging
    /// area for `save_file(..)`, since HarmonyOS's OS temp dir isn't writable by the app.
    pub fn set_cache_dir(path: &str) {
        day_spec::ffi_guard::contain((), || {
            if !path.is_empty() {
                day_spec::present::set_app_temp_dir(path.to_owned());
            }
        });
    }

    /// The ArkUI backend. `new` collects any externally-registered renderers (§8.2), like the others.
    pub struct ArkUi {
        registry: Registry<ArkUi>,
    }

    #[distributed_slice]
    pub static RENDERERS: [fn() -> Renderer<ArkUi>];

    impl ArkUi {
        pub fn new() -> Self {
            let mut registry = Registry::default();
            for f in RENDERERS {
                registry.register(f());
            }
            ArkUi { registry }
        }
    }

    impl Default for ArkUi {
        fn default() -> Self {
            Self::new()
        }
    }

    /// Warn once per kind that this backend has no registered renderer for `kind`, before falling
    /// back to a placeholder (an empty stack node). A missing renderer usually means the piece's
    /// `arkui` feature wasn't enabled. Deduped per kind so it doesn't spam the log.
    fn warn_missing_renderer(kind: PieceKind) {
        day_spec::placeholder::report(kind, "arkui");
    }

    impl Toolkit for ArkUi {
        type Handle = AHandle;

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> AHandle {
            match Builtin::from_key(kind) {
                Some(Builtin::Container) => {
                    let n = new_node(node::STACK);
                    if let Some(p) = props.downcast_ref::<ContainerProps>() {
                        if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                            // A translucent neutral fill reads as a subtle card on both the
                            // light and dark ArkUI themes (no public semantic-fill API).
                            themed(
                                n.0 as usize,
                                n.0,
                                Paint::Background,
                                0x1480_8080,
                                0x2EFF_FFFF,
                            );
                        } else if let Some(c) = p.background {
                            node::set_bg_color(n.0, argb(c));
                        }
                        if p.corner_radius > 0.0 {
                            // NODE_BORDER_RADIUS in vp rounds this node's own background.
                            node::set_corner_radius(n.0, p.corner_radius);
                        }
                        if p.clips {
                            // `.corner_radius` wraps the piece, whose fill is an inner
                            // node, so rounding shows only when this node clips it.
                            node::set_clip(n.0, true);
                        }
                    }
                    n
                }
                Some(Builtin::Scroll) => {
                    let n = new_node(node::SCROLL);
                    let horizontal = props
                        .downcast_ref::<day_spec::props::ScrollProps>()
                        .map(|p| p.horizontal)
                        .unwrap_or(false);
                    node::scroll_direction(n.0, horizontal);
                    // The one real child ArkUI's Scroll measures its extent from (see
                    // [`SCROLL_CONTENT`]); day children land inside it via `insert`.
                    let content = new_node(node::STACK);
                    node::insert_child(n.0, content.0, 0);
                    SCROLL_CONTENT
                        .with(|m| m.borrow_mut().insert(n.0 as usize, content.0 as usize));
                    n
                }
                Some(Builtin::Image) => {
                    // Here and in every arm below: a props-type mismatch degrades to the same
                    // empty-stack placeholder a missing renderer gets (`props_of` reported it):
                    // realize runs inside native up-calls, where a panic is a process kill.
                    let Some(p) = day_spec::props_of::<ImageProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    let n = new_node(node::IMAGE);
                    // Resolve `image("name")` through the app's rawfile store, the only resource
                    // root the OpenHarmony NDK can address from native code (app.media is ArkTS-only,
                    // §18.3). The CLI stages each image uncompressed to resources/rawfile/day/<name>
                    // normalized to PNG, so a bare `source` (no extension) maps to `day/<source>.png`.
                    // A vector name resolves to its staged SVG instead (docs/vectors.md): ArkUI
                    // renders it natively at display size, and `.tint(…)` recolors it via
                    // NODE_IMAGE_FILL_COLOR (untinted = as authored, matching every backend).
                    // Bytes and Decoded arrive from `day::decode_image` (docs/images.md), so an
                    // `image()` piece can show a download or a pasted PNG with no staged resource
                    // behind it. Only a named source takes a tint: the recolor repaints an SVG's
                    // paths, and bytes have no SVG to repaint.
                    arkui_apply_image_source(n.0, &p.source, p.tint);
                    // Scaling (§18.3): ArkUI_ObjectFit CONTAIN=0 (fit) / COVER=1 (fill) / FILL=3.
                    let fit = match p.content_mode {
                        ContentMode::Fit => 0,
                        ContentMode::Fill => 1,
                        ContentMode::Stretch => 3,
                    };
                    node::set_image_fit(n.0, fit);
                    n
                }
                Some(Builtin::Label) => {
                    let Some(p) = day_spec::props_of::<LabelProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    let n = new_node(node::TEXT);
                    node::set_text(n.0, &p.text);
                    if !p.wraps {
                        node::label_single_line(n.0);
                    }
                    node::set_font_size(n.0, font_vp(p.font));
                    if let Some(c) = p.color {
                        node::set_font_color(n.0, argb(c));
                    } else {
                        // Text defaults don't re-theme through the C API: an un-colored
                        // label takes the mode's primary text color, repainted on a switch.
                        themed(n.0 as usize, n.0, Paint::Font, TEXT_LIGHT, TEXT_DARK);
                    }
                    apply_font_attrs(n.0, p.font);
                    if !p.runs.is_empty() {
                        set_label_runs(n.0, &p.text, &p.runs);
                    }
                    n
                }
                Some(Builtin::Button) => {
                    let Some(p) = day_spec::props_of::<ButtonProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    let n = new_node(node::BUTTON);
                    node::set_button_label(n.0, &p.title);
                    node::register_event(n.0, node::EV_CLICK, id.0);
                    node::enable_focus(n.0, id.0, false);
                    apply_button_style(n.0, p.style);
                    if p.icon.is_some() {
                        apply_button_content(n, &p.title, p.icon.as_ref(), p.icon_only);
                    }
                    node::set_enabled(n.0, p.enabled);
                    n
                }
                Some(Builtin::TextField) => {
                    let Some(p) = day_spec::props_of::<TextFieldProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    let n = new_node(node::TEXT_INPUT);
                    CTRL_NODE.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    TEXT_ECHO.with(|m| m.borrow_mut().insert(id.0, p.text.clone()));
                    INPUT_FIELDS.with(|m| m.borrow_mut().insert(id.0, InputField::default()));
                    node::set_input_text(n.0, &p.text);
                    node::set_placeholder(n.0, &p.placeholder);
                    node::register_event(n.0, node::EV_TEXT_INPUT_CHANGE, id.0);
                    node::enable_focus(n.0, id.0, true);
                    node::set_enabled(n.0, p.enabled);
                    n
                }
                // Multi-line editor (docs/textarea.md): ARKUI_NODE_TEXT_AREA. min/max-lines
                // aren't a native attribute here: the node grows with content and the measure
                // arm bounds it.
                Some(Builtin::TextArea) => {
                    let Some(p) = day_spec::props_of::<TextAreaProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    let n = new_node(node::TEXT_AREA);
                    CTRL_NODE.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    TEXTAREA_LINES.with(|m| {
                        m.borrow_mut()
                            .insert(n.0 as usize, (p.min_lines, p.max_lines))
                    });
                    node::set_textarea_text(n.0, &p.text);
                    node::set_textarea_placeholder(n.0, &p.placeholder);
                    node::register_event(n.0, node::EV_TEXT_AREA_CHANGE, id.0);
                    node::enable_focus(n.0, id.0, true);
                    n
                }
                // Option picker (docs/picker.md). A menu picker is HarmonyOS's own dropdown, the
                // ArkTS `Select` the host registers (DaySelect.ets; the C node API has no select
                // kind). The same host builds a compact button row for segmented pickers.
                // Inline pickers and hosts without the built-in piece use the TEXT_PICKER wheel.
                Some(Builtin::Picker) => {
                    let Some(p) = day_spec::props_of::<PickerProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    if matches!(p.style, PickerStyle::Menu | PickerStyle::Segmented)
                        && let Some(n) = piece::try_make(
                            if p.style == PickerStyle::Segmented {
                                SEGMENTED_PICKER_KIND
                            } else {
                                MENU_PICKER_KIND
                            },
                            id,
                            &select_props(Some(p.selected), &p.options),
                        )
                    {
                        if !p.enabled {
                            set_picker_piece_enabled(&n, false);
                        }
                        if p.style == PickerStyle::Segmented {
                            size_segmented_picker(&n, &p.options);
                        } else {
                            size_menu_picker(
                                &n,
                                p.options.get(p.selected).map(String::as_str),
                                &p.options,
                            );
                        }
                        return n;
                    }
                    let n = new_node(node::TEXT_PICKER);
                    let joined = p.options.join(";");
                    PICKER_SELECTED.with(|m| m.insert(n.0 as usize, p.selected));
                    node::set_picker(n.0, &joined, p.selected as u32);
                    node::register_event(n.0, node::EV_TEXT_PICKER_CHANGE, id.0);
                    node::enable_focus(n.0, id.0, false);
                    node::set_enabled(n.0, p.enabled);
                    n
                }
                Some(Builtin::Toggle) => {
                    let Some(p) = day_spec::props_of::<ToggleProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    let n = new_node(node::TOGGLE);
                    CTRL_NODE.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    TOGGLE_GATE.with(|m| {
                        m.borrow_mut().insert(
                            id.0,
                            ToggleGate {
                                handle: n.0 as usize,
                                written: p.on,
                                armed: false,
                            },
                        )
                    });
                    node::set_toggle(n.0, p.on);
                    node::register_event(n.0, node::EV_TOGGLE_CHANGE, id.0);
                    // What arms the gate: the user's touch, or a key on the focused switch.
                    node::register_event(n.0, node::EV_TOUCH, id.0);
                    node::register_event(n.0, node::EV_KEY, id.0);
                    node::enable_focus(n.0, id.0, false);
                    node::set_enabled(n.0, p.enabled);
                    n
                }
                Some(Builtin::Slider) => {
                    let Some(p) = day_spec::props_of::<SliderProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    let n = new_node(node::SLIDER);
                    CTRL_NODE.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    SLIDER_ECHO.with(|m| m.borrow_mut().insert(id.0, p.value));
                    SLIDER_RANGE.with(|m| m.borrow_mut().insert(id.0, (p.min, p.max)));
                    node::set_slider(n.0, normalize(p.value, p.min, p.max));
                    node::register_event(n.0, node::EV_SLIDER_CHANGE, id.0);
                    node::enable_focus(n.0, id.0, false);
                    node::set_enabled(n.0, p.enabled);
                    n
                }
                // A 1-vp hairline: a thin Stack tinted with a faint separator color.
                Some(Builtin::Divider) => {
                    let n = new_node(node::STACK);
                    themed(
                        n.0 as usize,
                        n.0,
                        Paint::Background,
                        0x3300_0000,
                        0x33FF_FFFF,
                    );
                    n
                }
                // Determinate bar (ARKUI_NODE_PROGRESS) vs indeterminate spinner (LOADING_PROGRESS).
                Some(Builtin::Progress) => {
                    let Some(p) = day_spec::props_of::<ProgressProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    match p.value {
                        Some(v) => {
                            let n = new_node(node::PROGRESS);
                            node::set_progress(n.0, v);
                            n
                        }
                        None => new_node(node::LOADING),
                    }
                }
                // Each stack is its own ArkTS Navigation, mounted through the piece bridge.
                // Root and pushed pages use that host's content slots, with native title bars,
                // transitions and Back. Tabs remain resident-page ownership boundaries.
                Some(Builtin::Nav) => {
                    let Some(p) = day_spec::props_of::<NavProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    // Rows as CHROME: a composed bottom bar over resident pages (see NavSuite).
                    // A compact window gets here through `Automatic`.
                    if p.presentation.rows_are_chrome() {
                        let host = new_node(node::COLUMN);
                        let pages = new_node(node::STACK);
                        let bar = new_node(node::ROW);
                        node::insert_child(host.0, pages.0, 0);
                        node::insert_child(host.0, bar.0, 1);
                        themed(
                            host.0 as usize,
                            bar.0,
                            Paint::Background,
                            0xFFF1_F3F5,
                            0xFF1C_1C1E,
                        );
                        SUITE_AWAITING_MENU.with(|c| c.set(Some(host.0 as usize)));
                        NAV_SUITES.with(|c| {
                            c.borrow_mut().insert(
                                host.0 as usize,
                                NavSuite {
                                    pages,
                                    bar,
                                    items: Vec::new(),
                                    bar_items: Vec::new(),
                                    bar_inks: Vec::new(),
                                    selected: 0,
                                    page_size: Size::ZERO,
                                    placement: SuitePlacement::Bottom,
                                    menu: None,
                                    labels: Vec::new(),
                                    icons: Vec::new(),
                                },
                            )
                        });
                        // Told once and never revised: the chrome is the same at every width.
                        emit(
                            id,
                            Event::NavPresentationChanged(day_spec::props::NavPresentation::Tabs),
                        );
                        return host;
                    }
                    let n = piece::make("day.navigation.stack", id, &p.title);
                    NAV_HOSTS.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    // An adaptive host follows the window: ArkUI's Navigation runs in Auto
                    // mode, tiling the rows beside the detail where both fit (a tablet, a wide
                    // window) and stacking them where they do not, and reports which through
                    // `navModeChanged` (docs/size-classes.md). `Stack` in props is literal, a
                    // host that stacks at every width (a nested `nav_stack`), so it is not.
                    if p.adaptive && p.presentation != day_spec::props::NavPresentation::Stack {
                        piece::update(&n, "day.adaptive", "1");
                    }
                    match p
                        .search
                        .as_ref()
                        .filter(|sp| sp.placement == SearchPlacement::Inline)
                    {
                        Some(sp) => crate::host_api::nav_set_search(id.0, 1, &sp.prompt, &sp.text),
                        None => crate::host_api::nav_set_search(id.0, 0, "", ""),
                    }
                    n
                }
                Some(Builtin::NavPage) => {
                    let n = new_node(node::STACK);
                    themed(
                        n.0 as usize,
                        n.0,
                        Paint::Background,
                        0xFFFF_FFFF,
                        0xFF1A_1A1C,
                    );
                    NAV_PAGE_IDS.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    n
                }
                // Fullscreen cover (docs/cover.md): a Stack that CoverPatch::Present re-homes
                // onto the host's cover layer above Navigation, at full-window bounds.
                Some(Builtin::Cover) => {
                    let n = new_node(node::STACK);
                    COVER_NODES.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    n
                }
                // A scrollable column of tappable rows; each row's tap becomes SelectionChanged(index)
                // against this menu host (via a synthetic click id, see on_event).
                Some(Builtin::NavMenu) => {
                    let Some(p) = day_spec::props_of::<NavMenuProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    // Inside a suite the rows are the bar. The list is still built (it lives in
                    // the sidebar page, which the suite keeps but never shows) so nothing else
                    // has to know which presentation it is in.
                    if let Some(host) = SUITE_AWAITING_MENU.with(|c| c.take()) {
                        MENU_SUITE.with(|m| m.borrow_mut().insert(id.0, host));
                        suite_fill_bar(host, id, &p.items, &p.icons, 0);
                    }
                    build_nav_menu(
                        id,
                        &p.items,
                        &p.icons,
                        &p.tints,
                        &p.badge_icons,
                        &p.badge_tints,
                        &p.sections,
                    )
                }
                // Canvas: a focus-holding host around the custom node whose on-draw callback
                // replays the encoded display list (src/canvas.rs).
                Some(Builtin::Canvas) => AHandle(crate::canvas::create(id.0)),
                // Recycling list: an ARKUI_NODE_LIST driven by a NodeAdapter (attach_list injects the
                // row source; the adapter binds cells on demand). See attach_list / src/list.rs.
                Some(Builtin::List) => {
                    let Some(p) = day_spec::props_of::<ListProps>(kind, "arkui", props) else {
                        return new_node(node::STACK);
                    };
                    let row_h = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 0.0,
                    };
                    let n = new_node(node::LIST);
                    LIST_NODE.with(|m| m.borrow_mut().insert(n.0 as usize, id.0));
                    crate::list::init(
                        n.0,
                        id.0,
                        row_h,
                        p.selectable,
                        p.reorderable,
                        p.deletable,
                        &p.delete_label,
                    );
                    n
                }
                // A recycled list cell is ADOPTED from the native list, never realized
                // through this path; anything else is an extension piece.
                Some(Builtin::ListCell)
                | Some(Builtin::Tree)
                | Some(Builtin::Inspector)
                | Some(Builtin::InspectorPane)
                | None => {
                    if let Some(r) = self.registry.get(kind) {
                        let make = r.make;
                        return make(self, props, id);
                    }
                    warn_missing_renderer(kind);
                    new_node(node::STACK)
                }
            }
        }

        fn update(
            &mut self,
            h: &AHandle,
            kind: PieceKind,
            patch: &dyn Any,
            anim: Option<&AnimSpec>,
        ) {
            match kind {
                // Navigation (docs/navigation.md): drive the ArkTS Navigation/NavPathStack.
                kinds::NAV => {
                    let owner = NAV_HOSTS.with(|m| m.borrow().get(&(h.0 as usize)).copied());
                    // Inline search (docs/search.md): the app writing its query fills the field.
                    if let Some(owner) = owner {
                        match patch.downcast_ref::<day_spec::props::SearchPatch>() {
                            Some(day_spec::props::SearchPatch::Focus) => {
                                crate::host_api::nav_set_search(owner, -2, "", "");
                            }
                            Some(day_spec::props::SearchPatch::Text(t)) => {
                                crate::host_api::nav_set_search(owner, -1, "", t);
                            }
                            _ => {}
                        }
                    }
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        match p {
                            NavPatch::Pushed { title, .. } => {
                                // Consume this host's pending child and mount it in a native
                                // destination. It is deliberately not attached to the wrapper:
                                // doing so would paint it on top of the native Navigation.
                                let last = NAV_ATTACHED.with(|v| {
                                    v.borrow_mut()
                                        .get_mut(&(h.0 as usize))
                                        .and_then(|v| v.pop())
                                });
                                if let Some((page, key)) = last {
                                    let page = page as Handle;
                                    if owner.is_some_and(|owner| {
                                        crate::host_api::nav_push(owner, page, key, title, true)
                                            == 0
                                    }) {
                                        NAV_PUSHED
                                            .with(|m| m.borrow_mut().insert(page as usize, key));
                                        NAV_STACK.with(|s| {
                                            s.borrow_mut()
                                                .entry(h.0 as usize)
                                                .or_default()
                                                .push(key)
                                        });
                                        NAV_PENDING_PUSH.with(|s| s.borrow_mut().insert(key));
                                    } else {
                                        // No ArkTS bridge (old host page): fall back to the
                                        // stacked-children presentation.
                                        node::add_child(h.0, page);
                                    }
                                }
                            }
                            NavPatch::Popped => {
                                // Pop natively only if a destination is actually up and not
                                // already popped by a native back (the NavBack sync path:
                                // `nav_popped` removed its key from NAV_STACK).
                                let popped = NAV_STACK.with(|s| {
                                    s.borrow_mut()
                                        .get_mut(&(h.0 as usize))
                                        .and_then(|s| s.pop())
                                });
                                if let Some(key) = popped {
                                    NAV_EXPECT_POP.with(|e| e.borrow_mut().insert(key));
                                    // A page popped before it ever landed (pushed and popped
                                    // within one frame) mounts nothing: ArkUI will fire
                                    // neither its area report nor its disappear. Retire the
                                    // pending push and wait on no acknowledgment; only a
                                    // LANDED page's pop blocks `ui_idle`.
                                    let landed =
                                        NAV_PENDING_PUSH.with(|s| !s.borrow_mut().remove(&key));
                                    if landed {
                                        NAV_PENDING_POP.with(|p| p.borrow_mut().insert(key));
                                    }
                                    if let Some(owner) = owner {
                                        crate::host_api::nav_pop(owner);
                                    }
                                }
                            }
                            NavPatch::Title(t) => {
                                if let Some(owner) = owner {
                                    crate::host_api::nav_set_title(owner, t);
                                }
                            }
                            NavPatch::GuardTop(on) => {
                                if let Some(owner) = owner {
                                    crate::host_api::nav_set_guard(owner, *on);
                                }
                            }
                            // Never arrives: this backend answers `Cap::NavRepresent =
                            // Emulated`. `Navigation.mode(Auto)` decides at its own threshold
                            // and is OBSERVED through `onNavigationModeChange`
                            // (`nav_presented`), never told (docs/size-classes.md).
                            NavPatch::Presentation(_) => {}
                            // The resident-page switch (docs/navigation.md): show that
                            // destination and move the bar's accent to it.
                            NavPatch::Select(i) => suite_select(h.0 as usize, *i),
                            // Never arrives: this backend answers `Cap::NavContentList`
                            // Unsupported, so the pieces layer composes the pane itself
                            // (docs/navigation.md).
                            NavPatch::ListVisible(_) | NavPatch::ListInStack(_) => {}
                        }
                    }
                }
                kinds::CONTAINER => {
                    if let Some(ContainerPatch::Background(Some(c))) =
                        patch.downcast_ref::<ContainerPatch>()
                    {
                        // Under `with_animation` the fill fades to the new color (§8.4).
                        let (n, c) = (h.0, argb(*c));
                        crate::anim::animate(n, anim, move || node::set_bg_color(n, c));
                    }
                }
                // Data-driven sidebar rebuild (docs/navigation.md): swap the rows column for a
                // freshly built one. Without this arm the patch was silently dropped: stale
                // rows kept rendering and their synthetic ids kept firing old indices (the same
                // bug the Android path documents fixing). Old synthetic ids are retired first so
                // a late tap on a recycled row cannot emit a wrong SelectionChanged.
                kinds::NAV_MENU => {
                    if let Some(NavMenuPatch::Items {
                        items,
                        icons,
                        tints,
                        badge_icons,
                        badge_tints,
                        sections,
                        ..
                    }) = patch.downcast_ref::<NavMenuPatch>()
                    {
                        let key = h.0 as usize;
                        let Some(menu) = NAV_MENU_IDS.with(|m| m.borrow().get(&key).copied())
                        else {
                            return;
                        };
                        MENU_ROWS.with(|m| m.borrow_mut().retain(|_, v| v.0 != menu));
                        // Data-driven rows: a suite's bar is those rows, so it is rebuilt from
                        // the same set rather than left showing the old destinations.
                        if let Some(host) = MENU_SUITE.with(|m| m.borrow().get(&menu.0).copied()) {
                            let selected = NAV_SUITES
                                .with(|m| m.borrow().get(&host).map_or(0, |s| s.selected));
                            suite_fill_bar(host, menu, items, icons, selected);
                        }
                        if let Some(old) = SCROLL_CONTENT.with(|m| m.borrow_mut().remove(&key)) {
                            forget_themed(old);
                            node::remove_child(h.0, old as Handle);
                            node::dispose(old as Handle);
                        }
                        let col = build_nav_menu_rows(
                            menu,
                            items,
                            icons,
                            tints,
                            badge_icons,
                            badge_tints,
                            sections,
                        );
                        node::insert_child(h.0, col.0, 0);
                        SCROLL_CONTENT.with(|m| m.borrow_mut().insert(key, col.0 as usize));
                    }
                    // NavMenuPatch::Selected: no native highlight on the conventional-rows
                    // menu (realize renders no selected state either).
                }
                kinds::COVER => {
                    if let Some(p) = patch.downcast_ref::<CoverPatch>() {
                        let node_id = COVER_NODES
                            .with(|m| m.borrow().get(&(h.0 as usize)).copied())
                            .map(NodeId);
                        let Some(node_id) = node_id else { return };
                        match p {
                            CoverPatch::Present { background, .. } => {
                                let bg = background
                                    .map(argb)
                                    .unwrap_or_else(|| theme_color(0xFFFF_FFFF, 0xFF1A_1A1C));
                                let Some((root, w, hgt)) = COVER_ROOT.with(|r| r.get()) else {
                                    return;
                                };
                                let key = h.0 as usize;
                                if COVER_PRESENTED.with(|s| s.borrow().contains(&key)) {
                                    return; // already presented
                                }
                                let prev = COVER_PARENTS.with(|m| m.borrow().get(&key).copied());
                                node::set_bg_color(h.0, bg);
                                // Detach from the tree slot it was parked in, then top the
                                // cover layer at full bounds. Navigation's root page is NOT
                                // the window root: its pushed destinations occlude that slot.
                                match prev {
                                    Some(p) => node::remove_child(p as Handle, h.0),
                                    None => node::remove_child(root as Handle, h.0),
                                }
                                node::add_child(root as Handle, h.0);
                                node::set_frame(h.0, 0.0, 0.0, w, hgt);
                                COVER_PARENTS.with(|m| m.borrow_mut().insert(key, root));
                                COVER_PRESENTED.with(|s| s.borrow_mut().insert(key));
                                crate::host_api::show_cover_layer(true);
                                // Report the content size outside this tree borrow.
                                post_emit(node_id, Event::FrameChanged(Size::new(w, hgt)));
                            }
                            // No interactive dismissal on this backend: nothing to disable.
                            CoverPatch::DismissDisabled(_) => {}
                            CoverPatch::Dismiss => {
                                let key = h.0 as usize;
                                if !COVER_PRESENTED.with(|s| s.borrow_mut().remove(&key)) {
                                    // Never presented (or already dismissed): still answer
                                    // the hide confirmation so the piece can dispose.
                                    post_emit(node_id, Event::CoverHidden);
                                    return;
                                }
                                let cur = COVER_PARENTS.with(|m| m.borrow_mut().remove(&key));
                                if let Some(p) = cur {
                                    node::remove_child(p as Handle, h.0);
                                }
                                crate::host_api::show_cover_layer(
                                    COVER_PRESENTED.with(|s| !s.borrow().is_empty()),
                                );
                                // No hide transition: the content can go immediately.
                                post_emit(node_id, Event::CoverHidden);
                            }
                        }
                    }
                }
                kinds::IMAGE => {
                    if let Some(p) = patch.downcast_ref::<day_spec::props::ImagePatch>() {
                        match p {
                            // SVG-only recolor, as at realize (docs/vectors.md). Only a tint to
                            // apply: ArkUI keeps no "authored" fill to go back to, so a `None`
                            // leaves the last recolor in place.
                            day_spec::props::ImagePatch::Tint(Some(c)) => {
                                node::set_image_fill(h.0, argb(*c))
                            }
                            day_spec::props::ImagePatch::Tint(None) => {}
                            // A source swap repaints the same node (docs/images.md).
                            day_spec::props::ImagePatch::Source(source) => {
                                arkui_apply_image_source(h.0, source, None);
                            }
                        }
                    }
                }
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Text(t) => node::set_text(h.0, t),
                            LabelPatch::Color(c) => match c {
                                Some(c) => {
                                    forget_paint(h.0 as usize, h.0, Paint::Font);
                                    node::set_font_color(h.0, argb(*c));
                                }
                                None => {
                                    themed(h.0 as usize, h.0, Paint::Font, TEXT_LIGHT, TEXT_DARK)
                                }
                            },
                            LabelPatch::Font(f) => {
                                node::set_font_size(h.0, font_vp(*f));
                                apply_font_attrs(h.0, *f);
                            }
                            LabelPatch::Runs(text, runs) => set_label_runs(h.0, text, runs),
                        }
                    }
                }
                kinds::BUTTON => match patch.downcast_ref::<ButtonPatch>() {
                    Some(ButtonPatch::Content(c)) => {
                        apply_button_content(*h, &c.title, c.icon.as_ref(), c.icon_only)
                    }
                    Some(ButtonPatch::Enabled(on)) => node::set_enabled(h.0, *on),
                    Some(ButtonPatch::Title(t)) => node::set_button_label(h.0, t),
                    Some(ButtonPatch::Style(s)) => apply_button_style(h.0, *s),
                    _ => {}
                },
                kinds::TOGGLE => match patch.downcast_ref::<TogglePatch>() {
                    Some(TogglePatch::On(on)) => {
                        // The gate (see TOGGLE_GATE): this write's echo is not the user's.
                        if let Some(nid) =
                            CTRL_NODE.with(|m| m.borrow().get(&(h.0 as usize)).copied())
                        {
                            toggle_written(nid, *on);
                        }
                        node::set_toggle(h.0, *on);
                    }
                    Some(TogglePatch::Enabled(on)) => node::set_enabled(h.0, *on),
                    None => {}
                },
                kinds::SLIDER => match patch.downcast_ref::<SliderPatch>() {
                    Some(SliderPatch::Value(v)) => {
                        let nid = CTRL_NODE.with(|m| m.borrow().get(&(h.0 as usize)).copied());
                        let (min, max) = nid
                            .and_then(|nid| SLIDER_RANGE.with(|m| m.borrow().get(&nid).copied()))
                            .unwrap_or((0.0, 1.0));
                        // The echo cell (see SLIDER_ECHO): the set below comes back as an
                        // onChange, which must not reach the app as the user's change.
                        if let Some(nid) = nid {
                            SLIDER_ECHO.with(|m| m.borrow_mut().insert(nid, *v));
                        }
                        node::set_slider(h.0, normalize(*v, min, max));
                    }
                    Some(SliderPatch::Enabled(on)) => node::set_enabled(h.0, *on),
                    None => {}
                },
                kinds::TEXT_FIELD => match patch.downcast_ref::<TextFieldPatch>() {
                    // A from_native echo would fight the user's caret: skip it (§4.4).
                    Some(TextFieldPatch::Text { text, from_native }) if !from_native => {
                        if let Some(nid) =
                            CTRL_NODE.with(|m| m.borrow().get(&(h.0 as usize)).copied())
                        {
                            TEXT_ECHO.with(|m| m.borrow_mut().insert(nid, text.clone()));
                            // The app's own write is the text a read-only field holds.
                            INPUT_FIELDS.with(|m| {
                                if let Some(f) = m.borrow_mut().get_mut(&nid)
                                    && f.held.is_some()
                                {
                                    f.held = Some(text.clone());
                                }
                            });
                        }
                        node::set_input_text(h.0, text);
                    }
                    Some(TextFieldPatch::Enabled(on)) => node::set_enabled(h.0, *on),
                    _ => {}
                },
                kinds::TEXT_AREA => {
                    if let Some(TextAreaPatch::SetText(text)) =
                        patch.downcast_ref::<TextAreaPatch>()
                    {
                        if let Some(nid) =
                            CTRL_NODE.with(|m| m.borrow().get(&(h.0 as usize)).copied())
                        {
                            TEXT_ECHO.with(|m| m.borrow_mut().insert(nid, text.clone()));
                        }
                        node::set_textarea_text(h.0, text);
                    }
                }
                kinds::PICKER if piece::is_piece(h) => match patch.downcast_ref::<PickerPatch>() {
                    Some(PickerPatch::Selected(i)) => piece::update(h, "selected", &i.to_string()),
                    // The Select keeps its own live choice across new options, clamped to the list;
                    // its width follows the new widest option (an options patch re-measures).
                    Some(PickerPatch::Options(opts)) => {
                        piece::update(h, "options", &select_props(None, opts));
                        resize_menu_picker(h, opts);
                    }
                    Some(PickerPatch::Enabled(on)) => set_picker_piece_enabled(h, *on),
                    None => {}
                },
                kinds::PICKER => match patch.downcast_ref::<PickerPatch>() {
                    Some(PickerPatch::Selected(i)) => {
                        PICKER_SELECTED.with(|m| m.insert(h.0 as usize, *i));
                        node::set_picker_selected(h.0, *i as u32);
                    }
                    // The wheel's whole option RANGE, re-set: the same attribute realize
                    // seeds. The selection rides along, clamped to the new list.
                    Some(PickerPatch::Options(opts)) => {
                        let joined = opts.join(";");
                        let selected = PICKER_SELECTED
                            .with(|m| m.get(h.0 as usize))
                            .unwrap_or(0)
                            .min(opts.len().saturating_sub(1));
                        node::set_picker(h.0, &joined, selected as u32);
                    }
                    Some(PickerPatch::Enabled(on)) => node::set_enabled(h.0, *on),
                    None => {}
                },
                kinds::PROGRESS => {
                    if let Some(ProgressPatch::Value(Some(v))) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        node::set_progress(h.0, *v);
                    }
                }
                kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                    Some(ListPatch::Reload) | Some(ListPatch::Splice(_)) => {
                        // Deferred out of the day-core borrow: ReloadAllItems fires the
                        // adapter's ADD/REMOVE synchronously, and a bind pulled while the
                        // borrow is held SKIPS (try_with_tree) and never retries, the
                        // deferred-native-mutation rule (docs/tree.md M1). Coalesced: one
                        // change fires several watches, and adapter reload bursts drop ADDs.
                        let n = h.0 as usize;
                        let fresh = LIST_RELOAD_PENDING.with(|p| p.borrow_mut().insert(n));
                        if fresh {
                            crate::main_thread::post_local(Box::new(move || {
                                LIST_RELOAD_PENDING.with(|p| p.borrow_mut().remove(&n));
                                crate::list::reload(n as Handle);
                            }));
                        }
                    }
                    Some(ListPatch::ScrollToEnd) => crate::list::scroll_to_end(h.0),
                    Some(ListPatch::ScrollToRow(row)) => {
                        crate::list::scroll_to_row(h.0, *row as u32)
                    }
                    // RowSizeInvalidated: the node adapter re-measures rows itself.
                    Some(ListPatch::Selected(rows)) => {
                        // Record, then repaint the live cells; newly bound cells pick the
                        // state up in the adapter's add path. Paint only, no echo.
                        if let Some(nid) =
                            LIST_NODE.with(|m| m.borrow().get(&(h.0 as usize)).copied())
                        {
                            LIST_SELECTED.with(|m| {
                                m.borrow_mut().insert(nid, rows.iter().copied().collect());
                            });
                            crate::list::paint_selection(h.0);
                        }
                    }
                    Some(ListPatch::RowSizeInvalidated(_)) | None => {}
                },
                // An external piece's own arkui renderer, if one registered for this kind. Without
                // this, every registered piece realized correctly and then ignored every patch:
                // realize and measure consulted the registry but update did not.
                _ => {
                    if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                        update(self, h, patch);
                    }
                }
            }
        }

        /// Offer a satellite piece its teardown hook before `release` frees the handle (§15.2).
        fn release_piece(&mut self, kind: day_spec::PieceKind, h: &Self::Handle) {
            // Copy the fn pointer out first: the registry lookup borrows `self` immutably and
            // the hook needs it mutably.
            let f = self.registry.get(kind).and_then(|r| r.release);
            if let Some(f) = f {
                f(self, h);
            }
        }

        fn release(&mut self, h: AHandle) {
            clear_button_content(h);
            let key = h.0 as usize;
            // One sweep drops this node's entry from every registered `SideTable`, present
            // and future, before the manual purges below (day_spec::sidetable; the existing
            // maps predate it and keep their explicit lines).
            day_spec::sidetable::sweep(key);
            forget_themed(key);
            // The control's echo cells go with it (a recycled address must not alias them).
            if let Some(nid) = CTRL_NODE.with(|m| m.borrow_mut().remove(&key)) {
                TEXT_ECHO.with(|m| m.borrow_mut().remove(&nid));
                INPUT_FIELDS.with(|m| m.borrow_mut().remove(&nid));
                SLIDER_ECHO.with(|m| m.borrow_mut().remove(&nid));
                SLIDER_RANGE.with(|m| m.borrow_mut().remove(&nid));
                TOGGLE_GATE.with(|m| m.borrow_mut().remove(&nid));
            }
            // A pushed page released without a Remove patch (whole-host teardown) must not
            // leave its re-home bookkeeping behind: a recycled node address would alias it.
            // The ArkTS side still holds the destination slot's keep-alive ref; drop that
            // too (nav_forget touches only the bookkeeping, never the content tree).
            if let Some(nav_key) = NAV_PUSHED.with(|m| m.borrow_mut().remove(&key)) {
                crate::host_api::nav_forget(nav_key);
            }
            NAV_ATTACHED.with(|v| {
                for pages in v.borrow_mut().values_mut() {
                    pages.retain(|(p, _)| *p != key);
                }
            });
            if NAV_HOSTS.with(|m| m.borrow_mut().remove(&key)).is_some() {
                NAV_ATTACHED.with(|m| m.borrow_mut().remove(&key));
                NAV_STACK.with(|m| m.borrow_mut().remove(&key));
                let pages: Vec<u64> = NAV_OWNERS.with(|m| {
                    let mut m = m.borrow_mut();
                    let pages = m
                        .iter()
                        .filter(|(_, host)| **host == key)
                        .map(|(page, _)| *page)
                        .collect();
                    m.retain(|_, host| *host != key);
                    pages
                });
                for page in pages {
                    NAV_PENDING_PUSH.with(|m| m.borrow_mut().remove(&page));
                    NAV_PENDING_POP.with(|m| m.borrow_mut().remove(&page));
                    NAV_EXPECT_POP.with(|m| m.borrow_mut().remove(&page));
                    NAV_POPPED_KEYS.with(|m| m.borrow_mut().remove(&page));
                }
            }
            // A cover released while presented, and a secondary window root released after
            // its ability went away, drop their records too (same aliasing hazard).
            COVER_PRESENTED.with(|s| {
                s.borrow_mut().remove(&key);
            });
            if COVER_NODES.with(|m| m.borrow().contains_key(&key)) {
                crate::host_api::show_cover_layer(COVER_PRESENTED.with(|s| !s.borrow().is_empty()));
            }
            SECONDARY.with(|s| s.borrow_mut().retain(|(_, ptr)| *ptr != key));
            if let Some(page) = NAV_PAGE_IDS.with(|m| m.borrow_mut().remove(&key)) {
                // A landed pop retains its owner until ArkTS acknowledges the transition.
                // Unmounted/same-frame pages will never produce that acknowledgment.
                if !NAV_PENDING_POP.with(|m| m.borrow().contains(&page)) {
                    NAV_OWNERS.with(|m| m.borrow_mut().remove(&page));
                    NAV_EXPECT_POP.with(|m| m.borrow_mut().remove(&page));
                    NAV_POPPED_KEYS.with(|m| m.borrow_mut().remove(&page));
                    NAV_PENDING_PUSH.with(|m| m.borrow_mut().remove(&page));
                }
            }
            COVER_NODES.with(|m| {
                m.borrow_mut().remove(&key);
            });
            COVER_PARENTS.with(|m| {
                m.borrow_mut().remove(&key);
            });
            TEXTAREA_LINES.with(|m| {
                m.borrow_mut().remove(&key);
            });
            // A host that is gone takes its suite with it: a stale suite would route the next
            // host at that address's children into freed nodes.
            if NAV_SUITES.with(|c| c.borrow_mut().remove(&key)).is_some() {
                MENU_SUITE.with(|m| m.borrow_mut().retain(|_, host| *host != key));
                SUITE_AWAITING_MENU.with(|c| {
                    if c.get() == Some(key) {
                        c.set(None);
                    }
                });
            }
            if let Some(nid) = TAP_HANDLES.with(|m| m.borrow_mut().remove(&key)) {
                TAP_NODES.with(|s| {
                    s.borrow_mut().remove(&nid);
                });
            }
            if let Some(nid) = LIST_NODE.with(|m| m.borrow_mut().remove(&key)) {
                LIST_SELECTED.with(|m| {
                    m.borrow_mut().remove(&nid);
                });
                LIST_SOURCES.with(|m| {
                    m.borrow_mut().remove(&nid);
                });
            }
            // A released NAV_MENU retires its rows' synthetic click ids: without this every
            // menu rebuild leaked its row entries for the process lifetime.
            if let Some(menu) = NAV_MENU_IDS.with(|m| m.borrow_mut().remove(&key)) {
                MENU_ROWS.with(|m| m.borrow_mut().retain(|_, v| v.0 != menu));
            }
            // A scroll owns its content container (realize): dispose it with the scroll.
            if let Some(stack) = SCROLL_CONTENT.with(|m| m.borrow_mut().remove(&key)) {
                forget_themed(stack);
                node::dispose(stack as Handle);
            }
            // An ArkTS-built piece node belongs to its BuilderNode: detach it from the native
            // wrapper, ask ArkTS to release it, and dispose only the wrapper; a native dispose
            // of the FrameNode would free a node ArkTS still holds.
            if let Some((id, inner)) = PIECE_NODES.with(|m| m.borrow_mut().remove(&key)) {
                MENU_PICKER_SIZE.with(|m| m.borrow_mut().remove(&key));
                PIECE_FRAME.with(|m| m.borrow_mut().remove(&key));
                node::remove_child(h.0, inner as Handle);
                crate::host_api::piece_dispose(id);
            }
            node::dispose(h.0);
        }

        fn insert(&mut self, parent: &AHandle, child: &AHandle, index: usize) {
            // A suite's own pages. The one at index 0 is the SIDEBAR page, whose rows became the
            // bar: it is kept so nothing downstream has to special-case a missing page, but never
            // shown; drawing the rows again as a list would be the same navigation twice.
            let mut report_page = None;
            let into_suite = NAV_SUITES.with(|c| {
                let mut c = c.borrow_mut();
                let Some(suite) = c.get_mut(&(parent.0 as usize)) else {
                    return false;
                };
                let page = suite.page_size;
                node::insert_child(suite.pages.0, child.0, index as i32);
                node::set_size(child.0, page.width, page.height);
                if index == 0 {
                    node::set_visibility(child.0, false);
                } else {
                    let id = NodeId(
                        NAV_PAGE_IDS
                            .with(|m| m.borrow().get(&(child.0 as usize)).copied())
                            .unwrap_or(0),
                    );
                    let position = (index - 1).min(suite.items.len());
                    suite.items.insert(position, (*child, id));
                    node::set_visibility(child.0, position == suite.selected);
                    report_page = Some(id);
                }
                true
            });
            if into_suite {
                if let Some(id) = report_page {
                    let host = parent.0 as usize;
                    // The host can be laid out before any pages join it. Report a new
                    // page's usable bounds after its FrameChanged listener is installed;
                    // otherwise NavLayout keeps the full host-height fallback forever.
                    // Read the current size when delivered: a resize/removal may intervene.
                    crate::main_thread::post_local(Box::new(move || {
                        let size = NAV_SUITES.with(|m| {
                            let m = m.borrow();
                            m.get(&host)
                                .filter(|s| s.items.iter().any(|(_, page)| *page == id))
                                .map(|s| s.page_size)
                        });
                        if let Some(size) = size.filter(|s| s.width > 0.0 && s.height > 0.0) {
                            emit(id, Event::FrameChanged(size));
                        }
                    }));
                }
                return;
            }
            // Root and pushed pages belong to this host's slots, never to a window singleton.
            let owner = NAV_HOSTS.with(|m| m.borrow().get(&(parent.0 as usize)).copied());
            if let Some(owner) = owner
                && let Some(id) =
                    NAV_PAGE_IDS.with(|m| m.borrow().get(&(child.0 as usize)).copied())
            {
                NAV_OWNERS.with(|m| m.borrow_mut().insert(id, parent.0 as usize));
                if index == 0 {
                    if crate::host_api::nav_push(owner, child.0, id, "", false) == 0 {
                        NAV_PUSHED.with(|m| m.borrow_mut().insert(child.0 as usize, id));
                    }
                } else {
                    NAV_ATTACHED.with(|v| {
                        v.borrow_mut()
                            .entry(parent.0 as usize)
                            .or_default()
                            .push((child.0 as usize, id))
                    });
                }
                return;
            }
            // A scroll's day children live in its content container (see [`SCROLL_CONTENT`]).
            let native_parent = SCROLL_CONTENT
                .with(|m| m.borrow().get(&(parent.0 as usize)).copied())
                .unwrap_or(parent.0 as usize);
            // A cover's current parent starts as its tree slot (Present re-homes it).
            if COVER_NODES.with(|m| m.borrow().contains_key(&(child.0 as usize))) {
                COVER_PARENTS.with(|m| m.borrow_mut().insert(child.0 as usize, native_parent));
            }
            node::insert_child(native_parent as Handle, child.0, index as i32);
        }

        fn remove(&mut self, parent: &AHandle, child: &AHandle) {
            let cp = child.0 as usize;
            if NAV_SUITES.with(|m| {
                let mut m = m.borrow_mut();
                let Some(suite) = m.get_mut(&(parent.0 as usize)) else {
                    return false;
                };
                node::remove_child(suite.pages.0, child.0);
                suite.items.retain(|(page, _)| page.0 != child.0);
                true
            }) {
                return;
            }
            NAV_ATTACHED.with(|v| {
                for pages in v.borrow_mut().values_mut() {
                    pages.retain(|(p, _)| *p != cp);
                }
            });
            // A presented cover lives under the window root, not its tree parent.
            if let Some(cur) = COVER_PARENTS.with(|m| m.borrow_mut().remove(&cp)) {
                node::remove_child(cur as Handle, child.0);
                return;
            }
            if let Some(key) = NAV_PUSHED.with(|m| m.borrow_mut().remove(&cp)) {
                // The page lives in an ArkTS NodeContent (NavDestination), not under the host.
                // Detach it only while that destination is still alive (a Day-initiated pop:
                // the Remove patch lands before the pop transition finishes). Once the ArkTS
                // side reported the disappearance (native back: the destination and its
                // content tree are already torn down), touching the slot would walk freed
                // FrameNodes: drop the bookkeeping instead.
                if NAV_POPPED_KEYS.with(|s| s.borrow_mut().remove(&key)) {
                    crate::host_api::nav_forget(key);
                } else {
                    crate::host_api::nav_remove(key, child.0);
                }
                return;
            }
            // Mirror `insert`'s re-routing for scroll children (see [`SCROLL_CONTENT`]).
            let native_parent = SCROLL_CONTENT
                .with(|m| m.borrow().get(&(parent.0 as usize)).copied())
                .unwrap_or(parent.0 as usize);
            node::remove_child(native_parent as Handle, child.0);
        }

        fn move_child(&mut self, parent: &AHandle, child: &AHandle, to: usize) {
            self.remove(parent, child);
            self.insert(parent, child, to);
        }

        fn measure(&mut self, h: &AHandle, kind: PieceKind, p: Proposal) -> Size {
            match kind {
                kinds::LABEL => {
                    // A label measures on a fresh copy: ArkUI answers a Text whose content
                    // changed with the old content's size (see node::measure_label).
                    let (w, hh) =
                        node::measure_label(h.0, p.width.unwrap_or(-1.0), p.height.unwrap_or(-1.0));
                    Size::new(w, hh)
                }
                kinds::BUTTON => {
                    // An icon button is sized from its content Row (see `apply_button_content`),
                    // a title button from a fresh copy (see node::measure_button).
                    let content = BUTTON_CHILDREN
                        .with(|m| {
                            m.borrow()
                                .get(&(h.0 as usize))
                                .and_then(|c| c.first().copied())
                        })
                        .map_or(std::ptr::null_mut(), |row| row.0);
                    let (w, hh) = node::measure_button(
                        h.0,
                        content,
                        p.width.unwrap_or(-1.0),
                        p.height.unwrap_or(-1.0),
                    );
                    Size::new(w, hh)
                }
                kinds::TEXT_FIELD => Size::new(p.width.unwrap_or(200.0), 40.0),
                kinds::TEXT_AREA => {
                    // Grow with content between the min/max line band (line ≈ 24 vp + padding).
                    let (min_lines, max_lines) = TEXTAREA_LINES
                        .with(|m| m.borrow().get(&(h.0 as usize)).copied())
                        .unwrap_or((1, 0));
                    let line = 24.0;
                    let min_h = min_lines as f64 * line + 16.0;
                    let nat = node::measure(h.0, p.width.unwrap_or(0.0), 0.0).1;
                    let capped = if max_lines == 0 {
                        nat.max(min_h)
                    } else {
                        nat.clamp(min_h, max_lines as f64 * line + 16.0)
                    };
                    Size::new(p.width.unwrap_or(200.0), capped)
                }
                // The Select button's own size (a wrapper measures its ArkTS child), with a floor
                // for a first measure before the component has laid out.
                kinds::PICKER if piece::is_piece(h) => MENU_PICKER_SIZE
                    .with(|m| m.borrow().get(&(h.0 as usize)).map(|(_, size)| *size))
                    .unwrap_or(Size::new(120.0, 40.0)),
                kinds::PICKER => Size::new(p.width.unwrap_or(200.0), 200.0),
                kinds::TOGGLE => Size::new(50.0, 30.0),
                kinds::SLIDER => Size::new(p.width.unwrap_or(200.0), 40.0),
                kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
                kinds::PROGRESS => Size::new(p.width.unwrap_or(40.0), p.height.unwrap_or(20.0)),
                // These fill their container (host owns scroll/paging; content is laid out inside).
                kinds::NAV_MENU => Size::new(p.width.unwrap_or(240.0), p.height.unwrap_or(400.0)),
                kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
                _ => {
                    if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                        return measure(self, h, p);
                    }
                    Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
                }
            }
        }

        fn set_selectable(&mut self, h: &AHandle, selectable: bool) -> Option<AHandle> {
            // NODE_TEXT_COPY_OPTION on the Text node; a non-text node ignores it (docs/text.md).
            node::label_set_selectable(h.0, selectable);
            None
        }

        fn set_input_traits(
            &mut self,
            h: &AHandle,
            traits: &day_spec::InputTraits,
        ) -> Option<AHandle> {
            // Attribute flips on the one TextInput node, so there is never a replacement.
            let nid = CTRL_NODE.with(|m| m.borrow().get(&(h.0 as usize)).copied())?;
            let watched = INPUT_FIELDS.with(|m| m.borrow().get(&nid).map(|f| f.watched))?;
            node::set_input_traits(h.0, traits);
            // Read-only (see `InputField`): the first time a field asks for it, start
            // answering its edits; from then on the flag alone decides the answer.
            if traits.read_only && !watched {
                node::watch_input_edits(h.0, nid);
            }
            let held = traits.read_only.then(|| node::input_text(h.0));
            INPUT_FIELDS.with(|m| {
                if let Some(f) = m.borrow_mut().get_mut(&nid) {
                    f.watched |= traits.read_only;
                    f.held = held;
                }
            });
            None
        }

        /// Derived from the node's font size (docs/baseline.md): the ArkUI C API publishes no
        /// baseline, so `Cap::BaselineAlignment` is `Emulated` here.
        fn first_baseline(&mut self, h: &AHandle, kind: PieceKind, size: Size) -> Option<f64> {
            if !day_spec::kind_has_baseline(kind) {
                return None;
            }
            node::baseline(h.0, size.height)
        }

        fn set_frame(&mut self, h: &AHandle, frame: Rect, _anim: Option<&AnimSpec>) {
            // The suite divides its own frame between the pages area and the bar, then tells each
            // page how much room it has: day-core sees one host node and gives it one frame.
            if NAV_SUITES.with(|c| c.borrow().contains_key(&(h.0 as usize))) {
                node::set_frame(
                    h.0,
                    frame.origin.x,
                    frame.origin.y,
                    frame.size.width,
                    frame.size.height,
                );
                suite_layout(h.0 as usize, frame.size);
                return;
            }
            // All navigation pages have native-owned frames, including resident tab
            // pages. Day's initial fallback must not overwrite the suite's allocation
            // and extend a scroll viewport underneath the tab bar.
            if NAV_PAGE_IDS.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
                return;
            }
            // A cover's frame is native-owned: full window while presented, parked otherwise.
            if COVER_NODES.with(|m| m.borrow().contains_key(&(h.0 as usize))) {
                return;
            }
            node::set_frame(
                h.0,
                frame.origin.x,
                frame.origin.y,
                frame.size.width,
                frame.size.height,
            );
            // An ArkTS piece is built detached, so a percentage size inside it resolves against
            // the window rather than this wrapper: tell it the size it was laid out at, when that
            // changes, as `day.frame` "<w>,<h>" (vp). A component that fills its frame sizes
            // itself from that; one that doesn't ignores the command.
            if piece::is_piece(h) {
                let key = h.0 as usize;
                let changed = PIECE_FRAME.with(|m| m.borrow_mut().insert(key, frame.size))
                    != Some(frame.size);
                if changed {
                    let size = format!("{},{}", frame.size.width, frame.size.height);
                    piece::update(h, "day.frame", &size);
                }
            }
        }

        fn set_opacity(&mut self, h: &AHandle, opacity: f64, anim: Option<&AnimSpec>) {
            let n = h.0;
            crate::anim::animate(n, anim, move || node::set_opacity(n, opacity));
        }

        fn set_transform(
            &mut self,
            h: &AHandle,
            t: day_spec::Transform,
            _size: Size,
            anim: Option<&AnimSpec>,
        ) {
            // ArkUI takes the pivot as a fraction of the node's own size, so the laid-out size
            // isn't needed. The pivot is not animated: only where the node moves to is.
            let n = h.0;
            node::set_transform_center(n, t.anchor_x, t.anchor_y);
            crate::anim::animate(n, anim, move || {
                node::set_transform(n, t.tx, t.ty, t.sx, t.sy, t.rotate_deg)
            });
        }

        fn set_scroll_content(&mut self, h: &AHandle, content: Size) {
            // Size the backend-owned container (see [`SCROLL_CONTENT`]) so ArkUI's Scroll
            // measures the real extent; that extent is what makes touch and programmatic
            // offsets take effect. Size without position: `NODE_POSITION` removes a child
            // from layout flow, and the Scroll's measure ignores positioned children.
            if let Some(stack) = SCROLL_CONTENT.with(|m| m.borrow().get(&(h.0 as usize)).copied()) {
                node::set_size(stack as Handle, content.width, content.height);
            }
        }

        fn scroll_to(&mut self, h: &AHandle, target: Rect, animated: bool) {
            // The node module owns the minimal-reveal math (it can read the offset + size).
            node::scroll_to_rect(
                h.0,
                target.origin.x as f32,
                target.origin.y as f32,
                target.size.width as f32,
                target.size.height as f32,
                animated,
            );
        }

        fn focus(&mut self, h: &AHandle, _node: NodeId, focused: bool) {
            // The UI context's focus is cleared only while this node still owns it, and typed
            // non-focusable errors are swallowed: no event, and the signal snaps back.
            node::focus(h.0, focused);
        }

        fn set_event_sink(&mut self, sink: EventSink) {
            SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
        }

        fn set_a11y(&mut self, h: &AHandle, a11y: &A11yProps) {
            // Every member is an ArkUI accessibility attribute (node.rs says which).
            node::set_a11y(h.0, a11y);
        }

        fn read_native(&self, h: &AHandle) -> day_spec::NativeSnapshot {
            read_native(h.0)
        }

        fn announce(&mut self, text: &str, urgent: bool) {
            // The accessibility kit's announcement event (src/host.rs); nothing when the ArkTS
            // arm is not staged, which is also what `Cap::Announce` reports.
            crate::host::announce(text, urgent);
        }

        fn enable_gesture(&mut self, h: &AHandle, node_id: NodeId, kind: GestureKind) {
            // Tap is a NODE_ON_CLICK that emits `Event::Tap` (tracked in TAP_NODES so the shared
            // click receiver knows to send Tap, not Pressed). Drag is a native pan recognizer
            // (docs/shapes.md) whose phases arrive on the shared gesture wire.
            // Long-press isn't wired on ArkUI yet: a piece that needs it degrades to no gesture.
            match kind {
                GestureKind::Tap => {
                    TAP_NODES.with(|s| s.borrow_mut().insert(node_id.0));
                    TAP_HANDLES.with(|m| m.borrow_mut().insert(h.0 as usize, node_id.0));
                    node::register_event(h.0, node::EV_CLICK, node_id.0);
                }
                GestureKind::Drag => crate::gesture::enable_pan(h.0, node_id.0),
                // Hover is deliberately unwired here (docs/canvas.md "Interaction"): the C node
                // API's `NODE_ON_HOVER` reports only entered/exited with no coordinates, and the
                // contract is a POINT. `NODE_ON_MOUSE` carries one, but this is the one target
                // nothing here can run, and a HarmonyOS phone has no pointer to hover with, so
                // it degrades to no gesture, like long-press, rather than to a guessed position.
                _ => {}
            }
        }

        fn supports_lifecycle(&self, phase: day_spec::Lifecycle) -> bool {
            lifecycle_supported(phase)
        }

        fn request_frame(
            &mut self,
            host: &AHandle,
            cb: day_spec::FrameCallback,
        ) -> day_spec::CancelFrame {
            fn fire(token: u64, timestamp: f64) {
                day_core::frame::native::deliver(token, day_spec::FrameStamp::new(timestamp));
            }
            let token = day_core::frame::native::register(cb);
            if !crate::vsync::request_frame(host.0 as usize, token, fire) {
                day_core::frame::native::cancel(token);
                log::error!("NativeVSync frame request failed");
            }
            Box::new(move || {
                day_core::frame::native::cancel(token);
                crate::vsync::cancel_frame(token);
            })
        }

        fn replay(&mut self, h: &AHandle, ops: &[DrawOp], _size: Size) {
            ensure_canvas_fonts();
            // Encode the display list the shared way (day-android uses the same encoder) and hand
            // it to the custom node; its on-draw callback replays it with OH_Drawing (§11).
            let (nums, texts) = day_spec::encode_ops(ops);
            crate::canvas::set_ops(h.0, &nums, &texts);
        }

        fn adopt(&mut self, raw: day_spec::RawHandle) -> AHandle {
            // A recycling LIST cell's inner Stack, created natively and handed back through the
            // adapter's bind callback: day mounts + rebinds the row's content into it.
            AHandle(raw.cast())
        }

        fn attach_list(&mut self, host: &AHandle, source: day_spec::ListSource) {
            if let Some(report) = &source.first_visible {
                report(0);
            }
            if let Some(nid) = LIST_NODE.with(|m| m.borrow().get(&(host.0 as usize)).copied()) {
                LIST_SOURCES.with(|m| m.borrow_mut().insert(nid, source));
            }
            crate::list::reload(host.0);
        }

        /// `OH_Drawing_FontMgr` families and faces (docs/fonts.md).
        fn font_families(&mut self) -> Vec<day_spec::FontFamilyInfo> {
            let Some(text) = crate::fonts::families_text() else {
                return Vec::new();
            };
            let mut list = day_spec::parse_font_list(&text);
            // The bundled families the ability registered from `day/fonts.json`
            // (`[{"family": …, "file": …}]`): one Regular face each, appended when the
            // manager's own list did not report them.
            if let Some(res) = open_resource("fonts.json") {
                let json = String::from_utf8_lossy(res.as_slice()).into_owned();
                for family in manifest_families(&json) {
                    if !list.iter().any(|f| f.family.eq_ignore_ascii_case(&family)) {
                        list.push(day_spec::FontFamilyInfo {
                            family,
                            faces: vec![day_spec::FontFace {
                                name: "Regular".to_string(),
                                weight: day_spec::FontWeight::Regular,
                                italic: false,
                            }],
                        });
                    }
                }
            }
            list
        }

        /// `OH_Drawing_FontMeasureText` + the font metrics of the face the canvas draws with
        /// (docs/fonts.md).
        fn measure_text(
            &mut self,
            text: &str,
            size: f64,
            font: &day_spec::CanvasFont,
        ) -> Option<day_spec::TextMetrics> {
            ensure_canvas_fonts();
            let out = crate::fonts::measure_text(
                text,
                size,
                i32::from(font.css_weight()),
                font.italic,
                font.family_str(),
            );
            Some(day_spec::TextMetrics::from_slots(&out))
        }

        /// Decode bytes with `OH_ImageSourceNative` (docs/images.md), which reads every container
        /// the platform's image framework knows.
        fn decode_image(&mut self, req: u64, id: day_spec::BitmapId, bytes: &[u8]) {
            let event = match crate::images::decode(id.0, bytes) {
                Some((w, h, has_alpha)) => {
                    let info = day_spec::BitmapInfo {
                        pixels: Size::new(w, h),
                        // Decoded bytes carry no density: a PNG is simply its pixels.
                        scale: 1.0,
                        format: day_spec::ImageFormat::sniff(bytes),
                        // Read from the pixelmap's alpha type, not inferred from the container.
                        has_alpha,
                    };
                    BITMAP_INFO.with(|m| m.borrow_mut().insert(id.0, info));
                    Event::ImageDecoded {
                        req,
                        result: Ok(info),
                    }
                }
                None => Event::ImageDecoded {
                    req,
                    result: Err(day_spec::ImageError::Decode),
                },
            };
            emit(day_spec::WINDOW_NODE, event);
        }

        fn image_info(&mut self, id: day_spec::BitmapId) -> Option<day_spec::BitmapInfo> {
            BITMAP_INFO.with(|m| m.borrow().get(&id.0).copied())
        }

        fn encode_image(&mut self, req: u64, id: day_spec::BitmapId, spec: &day_spec::EncodeSpec) {
            let result = encode_bitmap(id, spec);
            emit(day_spec::WINDOW_NODE, Event::ImageEncoded { req, result });
        }

        /// What the image packer WRITES. The platform READS more, and the asymmetry is its own,
        /// which is why this duty exists rather than letting `Cap::ImageEncode` imply symmetry.
        fn encode_formats(&mut self) -> Vec<day_spec::ImageFormat> {
            use day_spec::ImageFormat::{Jpeg, Png};
            vec![Png, Jpeg]
        }

        fn release_image(&mut self, id: day_spec::BitmapId) {
            BITMAP_INFO.with(|m| {
                m.borrow_mut().remove(&id.0);
            });
            crate::images::release(id.0);
        }

        /// In-process capture of the window root (docs/window-image.md). `hdc shell
        /// snapshot_display` remains what a dayscript screenshot uses on a device (it is the
        /// whole display, including the system status bar this cannot see), but the app itself
        /// needs an answer that does not shell out, and this is it.
        fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
            let (root, _, _) = ROOT_KEEP
                .with(|r| r.get())
                .ok_or("no window root to capture")?;
            crate::images::snapshot_png(root as Handle)
                .ok_or_else(|| "the node has no snapshot".into())
        }

        /// The color mode resolved at startup (DAY_THEME override, else the host-reported
        /// system mode): the same flag every neutral day-arkui paint branches on.
        fn dark_mode(&mut self) -> bool {
            IS_DARK.with(|d| d.get())
        }

        /// The settings data's animation duration scale at zero (docs/accessibility.md), read
        /// through the ArkTS arm in src/host.rs; false where the arm is not staged, which is
        /// also what `Cap::ReduceMotion` reports.
        fn reduce_motion(&mut self) -> bool {
            crate::host::reduce_motion()
        }

        /// The application context's color mode (docs/appearance.md). ArkTS components and
        /// the C nodes' theme colors restyle in place, and this backend repaints its own neutral
        /// paints (see [`themed`]). A return to the system mode has no answer until the
        /// environment callback `init` subscribed reports the mode it resolved to.
        fn set_appearance(&mut self, dark: Option<bool>) {
            crate::host::set_color_mode(dark);
            // An override repaints now (day-core reads `dark_mode()` right after this returns);
            // the environment callback that follows then finds nothing changed.
            if let Some(dark) = dark {
                repaint_themed(dark);
            }
        }

        /// NotificationKit's `setBadgeNumber` (docs/badge.md): a count, which the launcher
        /// draws on the icon; text and a bare dot have no HarmonyOS form.
        fn set_app_badge(&mut self, badge: &day_spec::AppBadge) {
            match badge {
                day_spec::AppBadge::None => crate::host::set_badge(0),
                day_spec::AppBadge::Count(n) => crate::host::set_badge(*n),
                day_spec::AppBadge::Text(_) | day_spec::AppBadge::Dot => {}
            }
        }

        /// Whether nav transitions have settled: dayscript screenshots poll this, so a shot
        /// taken right after a section switch waits for the pushed destination's first area
        /// report (content laid out) and for Day-initiated pops to be acknowledged.
        fn prepare_snapshot(
            &mut self,
            host: Option<&Self::Handle>,
            revision: u32,
        ) -> Result<day_spec::capture::Readiness, String> {
            let window = match host {
                None => 0,
                Some(host) => SECONDARY
                    .with(|s| {
                        s.borrow()
                            .iter()
                            .find(|(_, ptr)| *ptr == host.0 as usize)
                            .map(|(n, _)| *n)
                    })
                    .ok_or("capture window is gone")?,
            };
            crate::host_api::prepare_capture(revision, window)
        }

        fn ui_idle(&mut self) -> bool {
            NAV_PENDING_PUSH.with(|s| s.borrow().is_empty())
                && NAV_PENDING_POP.with(|p| p.borrow().is_empty())
        }

        /// Native file open/save via the ArkTS `@kit.CoreFileKit` DocumentViewPicker (docs/files.md).
        /// Alerts/prompts aren't wired on ArkUI yet, so those specs are ignored (like XAML).
        fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
            use day_spec::present::PresentSpec;
            match spec {
                PresentSpec::OpenFile { .. } => {
                    crate::host_api::present_file(req, 0, "", "", &spec.filters_joined());
                }
                PresentSpec::SaveFile {
                    suggested_name,
                    src_path,
                    ..
                } => {
                    crate::host_api::present_file(
                        req,
                        1,
                        suggested_name,
                        src_path,
                        &spec.filters_joined(),
                    );
                }
                // Dialog / Prompt aren't implemented on ArkUI (a follow-up); ignore.
                _ => {}
            }
        }

        fn open_url(&mut self, url: &str) {
            crate::host_api::open_url(url);
        }

        fn set_status_bar_hidden(&mut self, hidden: bool) {
            // The ArkTS host hides it with `setSpecificSystemBarEnabled('status', …)`; the page's
            // onAreaChange then reports the taller content area (docs/cover.md).
            crate::host_api::set_status_bar_hidden(hidden);
        }

        fn set_drag_source(&mut self, h: &AHandle, source: day_spec::transfer::Source) {
            crate::transfer::source(h, source);
        }
        fn set_drop_target(&mut self, h: &AHandle, target: day_spec::transfer::Target) {
            crate::transfer::target(h, target);
        }
        fn edit_toolbar(&mut self, h: &AHandle, ops: &[day_spec::ToolbarOp]) -> bool {
            if is_secondary_root(h) {
                return true;
            }
            WINDOW_BAR.with(|b| {
                let mut b = b.borrow_mut();
                for op in ops {
                    b.apply(op);
                }
            });
            paint_window_bar();
            true
        }

        fn update_toolbar(&mut self, h: &AHandle, patch: &day_spec::ToolbarPatch) {
            use day_spec::ToolbarPatch as P;
            if is_secondary_root(h) {
                return;
            }
            match patch {
                // Search is never on this bar (`Cap::ToolbarSearch`, docs/search.md).
                P::Text { .. } | P::Suggestions { .. } | P::Focus { .. } => {}
                _ => {
                    if WINDOW_BAR.with(|b| b.borrow_mut().patch(patch)) {
                        paint_window_bar();
                    }
                }
            }
        }

        fn capability(&self, cap: Cap) -> Support {
            match cap {
                Cap::DragDrop
                | Cap::DragExternalImport
                | Cap::DragExternalExport
                | Cap::DragMultipleItems => Support::Native,
                Cap::FileDialogs => Support::Native,
                // `animateTo` interpolates opacity, transform and background color on ArkUI's
                // compositor; frames move instantly, as on Android (§8.4).
                Cap::Animation => Support::Native,
                // `OH_Drawing_FontMgr` lists every family and style set (docs/fonts.md).
                Cap::FontList => Support::Native,
                // `OH_ImageSourceNative` decodes every container the platform reads, and the
                // image packer writes back PNG and JPEG (docs/images.md). `Cap::ImageProperties`
                // is deliberately NOT here: the platform does expose
                // `OH_ImageSourceNative_GetImageProperty`, but nothing reads it yet, and an empty
                // struct would read as "this file records nothing" rather than "nobody looked".
                Cap::ImageDecode | Cap::ImageEncode => Support::Native,
                // OH_ArkUI_GetNodeSnapshot + the native image packer, both synchronous
                // (docs/window-image.md).
                Cap::Snapshot => Support::Native,
                // Every pushed page is an ArkTS NavDestination with a native title bar
                // (DayNavigation.ets); content needn't repeat the title (docs/navigation.md).
                Cap::NavHeader => Support::Native,
                // `Navigation` in `NavigationMode.Auto` tiles the nav bar (the sidebar rows)
                // beside the content where both fit: a native split (docs/size-classes.md).
                Cap::NavSplit => Support::Native,
                // `Emulated`: Auto mode decides at layout, so the platform owns the presentation
                // and Day observes it through `Event::NavPresentationChanged` rather than
                // pushing one in, the Android/UIKit policy.
                Cap::NavRepresent => Support::Emulated,
                // A `Navigation`'s title bar carries `.menus()` items, which is where a page's
                // toolbar commands go here (docs/toolbars.md). Emulated rather than Native: the
                // bar belongs to the navigation destination, not to the window, so an app that
                // asks whether there is persistent window chrome gets the honest answer.
                Cap::Toolbar => Support::Emulated,
                // The composed bottom bar (see NavSuite): ArkUI's native node set has no tab
                // container, so this one is built from Day's own primitives; Emulated says so.
                Cap::NavTabs => Support::Emulated,
                // And HarmonyOS should grow one as it narrows: a bottom bar is the phone idiom
                // here as it is on iOS and Android (docs/navigation.md).
                Cap::NavTabsAdaptive => Support::Emulated,
                // ArkUI's own drag pipeline (SetNodeDraggable + NODE_ON_DROP): long-press lift
                // with the system preview; a denied drop springs back natively (docs/list.md).
                Cap::NavReorder | Cap::ListReorder => Support::Native,
                // `NODE_LIST_ITEM_SWIPE_ACTION`: the row slides to reveal the app's delete
                // button, ArkUI's own idiom for the gesture (docs/list.md).
                Cap::ListDelete => Support::Native,
                // Emulated: a topmost full-window child of the root, not a system modal.
                Cap::Cover => Support::Emulated,
                // The window's `setSpecificSystemBarEnabled('status', …)` (docs/cover.md).
                Cap::StatusBarHidden => Support::Native,
                // The COMPOSED tree (docs/tree.md M2/M4): the piece flattens onto this
                // backend's NodeAdapter list; disclosure, indentation and row content are
                // day pieces. No native drag wiring, so `Cap::TreeMove` stays Unsupported
                // (`tree_move:` drives the seam synthetically).
                Cap::Tree => Support::Emulated,
                // Derived from NODE_FONT_SIZE: ArkUI publishes no baseline (docs/baseline.md).
                Cap::BaselineAlignment => Support::Emulated,
                Cap::TextRuns => Support::Native,
                // ArkTS arms (src/host.rs): present wherever `day build` staged them.
                Cap::Appearance => crate::host::color_mode_support(),
                Cap::AppBadgeCount => crate::host::badge_support(),
                // The ArkTS host decides: Native once its announce arm is staged.
                Cap::Announce => {
                    if crate::host::announce_support() == Support::Native {
                        Support::Native
                    } else {
                        Support::Unsupported
                    }
                }
                // Likewise: the settings data's animation scale, read and watched from ArkTS.
                Cap::ReduceMotion => {
                    if crate::host::reduce_motion_support() == Support::Native {
                        Support::Native
                    } else {
                        Support::Unsupported
                    }
                }
                // Multiton DayWindowAbility instances (docs/windows.md): Native only when
                // the ArkTS host registered the launchers; an older host degrades to the
                // cover fallback.
                Cap::MultiWindow => {
                    if crate::host_api::has_windows() {
                        Support::Native
                    } else {
                        Support::Unsupported
                    }
                }
                _ => Support::Unsupported,
            }
        }

        fn open_window(
            &mut self,
            id: NodeId,
            options: &day_spec::WindowOptions,
            kind: day_spec::WindowKind,
        ) -> day_spec::WindowOpenReply<AHandle> {
            // Preferences stay modal on mobile (docs/windows.md); Normal windows become
            // multiton ability instances (their own task cards; freeform on tablets).
            if kind == day_spec::WindowKind::Preferences {
                return day_spec::WindowOpenReply::Unsupported;
            }
            if crate::host_api::open_window(id.0, &options.title) {
                day_spec::WindowOpenReply::Pending
            } else {
                day_spec::WindowOpenReply::Unsupported
            }
        }

        fn close_window(&mut self, host: &AHandle) {
            let node_id = SECONDARY.with(|s| {
                s.borrow()
                    .iter()
                    .find(|(_, ptr)| *ptr == host.0 as usize)
                    .map(|(n, _)| *n)
            });
            if let Some(node_id) = node_id {
                crate::host_api::close_window(node_id);
            }
        }
    }

    impl Platform for ArkUi {
        const TARGET: &'static str = "harmony-arkui";
        const TOOLKIT: &'static str = "arkui";
        // The host owns the loop: `run` hands back at once, and DidExit is its last callback.
        const RUN_ENDS_APP: bool = false;

        fn run(self, _options: WindowOptions, ready: Box<dyn FnOnce(Self, AHandle, Size)>) {
            // The ArkTS ability owns the loop; init() already created + mounted the root.
            let (root, size) = ROOT
                .with(|r| r.borrow_mut().take())
                .expect("day-arkui: init() not called before run()");
            ready(self, root, size);
        }

        fn post(f: Box<dyn FnOnce() + Send>) {
            crate::main_thread::post(f);
        }
    }

    /// Map a day slider value into ArkUI's default 0..100 range.
    fn normalize(v: f64, min: f64, max: f64) -> f64 {
        if max <= min {
            0.0
        } else {
            ((v - min) / (max - min) * 100.0).clamp(0.0, 100.0)
        }
    }

    /// The `"family"` values of the staged font manifest, by a scan rather than a JSON parser:
    /// the CLI writes the file (plain strings, no escapes beyond `\"`), and this backend takes no
    /// JSON dependency for one key.
    fn manifest_families(json: &str) -> Vec<String> {
        manifest_entries(json).into_iter().map(|(f, _)| f).collect()
    }

    /// The manifest's `(family, file)` pairs, in order: each staged font's family name and the
    /// rawfile it was staged as under `day/fonts/`. The same key scan, one object at a time.
    fn manifest_entries(json: &str) -> Vec<(String, String)> {
        fn value_after(rest: &str, key: &str) -> Option<(String, usize)> {
            let i = rest.find(key)?;
            let after = &rest[i + key.len()..];
            let q = after.find('"')?;
            let value = &after[q + 1..];
            let end = value.find('"')?;
            let text = value[..end].replace("\\\"", "\"").replace("\\\\", "\\");
            Some((text, i + key.len() + q + 1 + end + 1))
        }
        let mut out = Vec::new();
        let mut rest = json;
        while let Some(open) = rest.find('{') {
            let Some(close) = rest[open..].find('}') else {
                break;
            };
            let object = &rest[open..open + close];
            let family = value_after(object, "\"family\"").map(|(v, _)| v);
            let file = value_after(object, "\"file\"").map(|(v, _)| v);
            if let (Some(family), Some(file)) = (family, file)
                && !family.is_empty()
                && !file.is_empty()
            {
                out.push((family, file));
            }
            rest = &rest[open + close + 1..];
        }
        out
    }

    /// Hand every bundled font's bytes to the drawing layer, once per process, so canvas text
    /// can draw in it (docs/fonts.md). The ability's ArkTS `font.registerFont` reaches the
    /// text engine that labels use and not `OH_Drawing`'s font manager, which is why the
    /// showcase's canvas came out in the system face while its labels were right.
    fn ensure_canvas_fonts() {
        thread_local! {
            static DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
        }
        if DONE.with(|d| d.replace(true)) {
            return;
        }
        let Some(manifest) = open_resource("fonts.json") else {
            return;
        };
        let json = String::from_utf8_lossy(manifest.as_slice()).into_owned();
        for (family, file) in manifest_entries(&json) {
            let Some(res) = open_resource(&format!("fonts/{file}")) else {
                log::warn!("bundled font {file:?} ({family:?}) is not in the rawfile store");
                continue;
            };
            if !crate::fonts::register_canvas_font(&family, res.as_slice()) {
                log::warn!("bundled font {file:?} ({family:?}) did not parse as a font");
            }
        }
    }

    /// The rawfile-backed data-resource opener (§18.3), registered once in [`init`]. Serves
    /// `resource("numbers.bin")` from the app's `resources/rawfile/day/<name>` store via the
    /// OpenHarmony `OH_ResourceManager_*` API: zero-copy where the entry is mmap-able, else a copy.
    ///
    /// Returns `None` until the ArkTS entry ability has handed the native side its `resourceManager`
    /// (the host's `registerResourceManager`); without it there is no `NativeResourceManager` to
    /// read through, so no data resources are available.
    fn open_resource(name: &str) -> Option<day_spec::resource::Resource> {
        if !crate::resources::available() {
            return None;
        }
        // OpenRawFile addresses entries relative to the rawfile root, so the lookup key for a staged
        // resource is `day/<name>` (the CLI stages data uncompressed under resources/rawfile/day/).
        let mapped = crate::resources::open(&format!("day/{name}"))?;
        let (data, len) = mapped.as_ptr_len();
        if data.is_null() {
            return None;
        }
        // Safety: `data`/`len` describe a valid immutable region owned by `mapped`, which the
        // Resource keeps until it drops, unmapping or freeing it then.
        Some(unsafe { day_spec::resource::Resource::from_raw(data, len, Box::new(mapped)) })
    }

    use std::any::Any;
}
