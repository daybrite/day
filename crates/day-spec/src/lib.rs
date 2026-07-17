//! day-spec — the toolkit specification (DESIGN.md §8).
//!
//! Backends depend ONLY on this crate (never on day-core). One backend is linked per binary;
//! `day-core` is monomorphized over the concrete [`Toolkit`].

use std::any::Any;
use std::collections::HashMap;

pub use day_geometry::*;

/// Bundled-resource random-access API + the per-backend opener seam (§18.3).
pub mod resource;
pub use resource::{
    AssetName, FontFamily, ImageName, Resource, ResourceOpener, resolve_image_file, resource,
    set_resource_opener,
};

/// Bundled custom fonts: name-table parsing, runtime font directory, family → file resolution
/// (§18.4). Shared by the CLI stagers and the backends' startup registration. Lives in the leaf
/// `day-fonts` crate (pure std, no `day-geometry`), re-exported here so `day_spec::fonts::…` is
/// unchanged for the backends while the CLI can depend on `day-fonts` alone.
pub use day_fonts as fonts;

// ---------------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------------

/// Interned piece-kind key, e.g. `"day.label"` or `"acme.combobox"`.
pub type PieceKind = &'static str;

pub mod kinds {
    pub const CONTAINER: &str = "day.container"; // dumb native panel (column/row/stack backing)
    pub const LABEL: &str = "day.label";
    pub const BUTTON: &str = "day.button";
    pub const TOGGLE: &str = "day.toggle";
    pub const SLIDER: &str = "day.slider";
    pub const TEXT_FIELD: &str = "day.text_field";
    pub const DIVIDER: &str = "day.divider";
    pub const SCROLL: &str = "day.scroll";
    pub const IMAGE: &str = "day.image";
    /// Progress indicator: determinate bar (fraction) or indeterminate spinner.
    pub const PROGRESS: &str = "day.progress";
    pub const CANVAS: &str = "day.canvas";
    /// Navigation host (docs/navigation.md): stack on mobile, split panes on desktop.
    pub const NAV: &str = "day.nav";
    /// One destination's native container inside a NAV host.
    pub const NAV_PAGE: &str = "day.nav_page";
    /// Native navigation item list (docs/navigation.md): NSOutlineView source list /
    /// GtkListBox navigation-sidebar / QListWidget / UITableView rows with chevrons.
    pub const NAV_MENU: &str = "day.nav_menu";
    /// Native tabbed container (docs/tabs.md): NSTabView / GtkNotebook / QTabWidget /
    /// UITabBarController / Android tab strip. Holds `TABS_PAGE` children, one visible.
    pub const TABS: &str = "day.tabs";
    /// One tab's content container inside a `TABS` host; its frame is native-owned.
    pub const TABS_PAGE: &str = "day.tabs_page";
    /// Native recycling list (docs/list.md): NSTableView / UITableView / RecyclerView /
    /// GtkListView / QListView. Owns scrolling + cell reuse; Day binds row content on demand.
    pub const LIST: &str = "day.list";
    /// A recycled row's content anchor inside a `LIST`; Day adopts the native cell as its handle.
    pub const LIST_CELL: &str = "day.list_cell";
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
    /// fill, Material surface-container, WinUI card background brush.
    SectionCard,
}

pub const NAV_SIDEBAR_WIDTH: f64 = 240.0;

/// Reserved id for window-level events (resize, lifecycle): day-core routes it to the root.
pub const WINDOW_NODE: NodeId = NodeId(u64::MAX);

/// Raw foreign native handle for polyglot adoption (§15.3).
pub type RawHandle = *mut std::ffi::c_void;

// ---------------------------------------------------------------------------
// Events (§8.3)
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Pressed,
    TextChanged(String),
    Submitted,
    ToggleChanged(bool),
    ValueChanged(f64),
    SelectionChanged(i64),
    FocusChanged(bool),
    Tap(Point),
    LongPress(Point),
    ContextMenu(Point),
    /// A drag/pan gesture (docs/shapes.md). `location` is in the node's local coordinates;
    /// `translation` is the cumulative movement since `Began`.
    Drag {
        phase: DragPhase,
        location: Point,
        translation: Point,
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
    Key(KeyEvent),
    Pointer(PointerEvent),
    WindowResized(Size),
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
    /// The app moved through a lifecycle phase (docs/lifecycle.md). Backends emit this from the
    /// native app/activity delegate; day-core routes it to the app's `on_lifecycle` handlers.
    Lifecycle(Lifecycle),
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
/// | phase | AppKit | UIKit | GTK | Qt | Android | WinUI |
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GestureKind {
    Tap,
    LongPress,
    Drag,
}

// ---------------------------------------------------------------------------
// Menus (app menu bar + context menus). The MODEL is a toolkit-neutral tree; each backend renders it
// with its OWN native affordance (NSMenu / GMenu+GtkPopoverMenu / QMenu / UIMenu / Android PopupMenu /
// WinUI MenuFlyout) and its own conventions, so day imposes no menu manager of its own.
// ---------------------------------------------------------------------------

/// A keyboard shortcut for a menu item. `primary` is the platform's command modifier — ⌘ on Apple,
/// Ctrl on GTK/Qt/WinUI — so one declaration reads correctly everywhere. `key` is a single character
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
/// (`cut:`/`copy:`/`paste:`…), a stock action on GTK/Qt/WinUI — so it targets the focused control,
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
}

/// One entry in a menu (recursive — a `Submenu` nests).
#[derive(Clone, Debug, PartialEq)]
pub enum MenuItem {
    /// A command. `id` (nonzero) dispatches [`Event::MenuAction`] to the app; a `role`-only item uses
    /// the native standard command instead (id 0). `label`/`shortcut` override the role's defaults.
    Action {
        id: u64,
        label: String,
        shortcut: Option<Shortcut>,
        enabled: bool,
        role: Option<MenuRole>,
    },
    /// A nested submenu.
    Submenu { label: String, items: Vec<MenuItem> },
    /// A visual separator.
    Separator,
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyEvent {
    pub key: String,
    pub modifiers: u8,
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
}

// ---------------------------------------------------------------------------
// Capabilities, animation, a11y
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cap {
    ListRecycling,
    Lottie,
    NativeSymbols,
    Snapshot,
    /// The toolkit presents `nav()` as sidebar+detail split panes (desktop). Mobile
    /// stacks answer `Unsupported` and get push/pop presentation instead.
    NavSplit,
    /// The toolkit shows the current destination's title in a NATIVE header/bar (e.g. the Windows
    /// NavigationView header, the iOS/GTK nav bar) — so a page needn't repeat it in its own content.
    NavHeader,
    /// The toolkit can present native alert/confirm/sheet/prompt modals (docs/dialogs.md).
    Dialogs,
    /// The toolkit can present native open/save file pickers (docs/files.md).
    FileDialogs,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Support {
    Native,
    Emulated,
    Unsupported,
}

/// Reserved animation intent (§8.4) — MVP backends ignore it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AnimSpec {
    pub duration_ms: u32,
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
}

impl Shape {
    /// The shape's bounding rectangle — the box gradient [`UnitPoint`]s resolve against.
    pub fn bounds(&self) -> Rect {
        match self {
            Shape::Rect(r) | Shape::RoundedRect(r, _) | Shape::Ellipse(r) => *r,
            Shape::Arc { rect, .. } => *rect,
            Shape::Line(a, b) => points_bounds(&[*a, *b]),
            Shape::Polygon(pts) => points_bounds(pts),
        }
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
/// gradient stretches into an ELLIPSE when the bounds aren't square (the WinUI relative-brush
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
    Stroke(Shape, Color, f64),
    Text {
        text: String,
        at: Point,
        size: f64,
        color: Color,
        anchor: TextAnchor,
    },
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
/// `*TextBlockStyle` resources on WinUI — so a Day app matches the OS's own typography and inherits its
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
/// weight override and italic flag. Backends resolve this to one native font.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontSpec {
    pub style: Font,
    pub weight: Option<FontWeight>,
    pub italic: bool,
}

impl Default for FontSpec {
    fn default() -> Self {
        FontSpec {
            style: Font::Body,
            weight: None,
            italic: false,
        }
    }
}

impl From<Font> for FontSpec {
    fn from(style: Font) -> Self {
        FontSpec {
            style,
            weight: None,
            italic: false,
        }
    }
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
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum LabelPatch {
        Text(String),
        Color(Option<Color>),
        Font(FontSpec),
    }

    /// A button's NATIVE styling tier. `Automatic` is the toolkit's stock look; `Bordered`
    /// asks for a visually contained button where the stock look is borderless (iOS's plain
    /// system button reads as a link); `Prominent` asks for the platform's accent-filled /
    /// default-action affordance. Toolkits whose stock buttons are already contained treat
    /// `Bordered` as `Automatic`.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
    pub enum ButtonStyleSpec {
        #[default]
        Automatic,
        Bordered,
        Prominent,
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

    /// Navigation host (docs/navigation.md). `split` = sidebar+detail presentation
    /// (chosen by the pieces layer from `Cap::NavSplit`); false = stack presentation.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavProps {
        pub title: String,
        pub split: bool,
    }
    /// Applied to the NAV HOST after a page child is attached / before it is removed;
    /// the toolkit animates its native presentation accordingly.
    #[derive(Clone, Debug, PartialEq)]
    pub enum NavPatch {
        /// The just-attached last page child became the top of the stack.
        Pushed { title: String },
        /// The top page is about to be removed; present its predecessor.
        Popped,
        /// Current top-of-stack title changed.
        Title(String),
    }

    /// One destination's native container. `sidebar` marks the split-mode sidebar pane.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavPageProps {
        pub title: String,
        pub sidebar: bool,
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
        pub selected: Option<usize>,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum NavMenuPatch {
        /// Programmatic highlight sync — toolkits apply WITHOUT re-emitting
        /// SelectionChanged (the TextField from_native echo rule).
        Selected(Option<usize>),
    }

    /// Native tabbed container (docs/tabs.md). `titles` are the tab labels in page order;
    /// `selected` is the active tab index. Toolkits present a native tab widget and show the
    /// selected page.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct TabsProps {
        pub titles: Vec<String>,
        /// Optional bundled-image name per tab (docs/tabs.md), same convention as
        /// [`NavMenuProps::icons`]. Rendered where the backend's tab widget shows icons (the iOS
        /// `UITabBar`, the Android tab strip); ignored by backends whose tabs are text-only.
        pub icons: Vec<Option<String>>,
        pub selected: usize,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum TabsPatch {
        /// Programmatic selection sync — toolkits apply WITHOUT re-emitting SelectionChanged
        /// (the TextField from_native echo rule).
        Selected(usize),
    }

    /// One tab's content container. `title` is its tab label (read by the host on insert);
    /// `icon` is its optional bundled-image name, set on the tab item where the backend shows
    /// tab icons (iOS `UITabBarItem`), ignored otherwise.
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct TabsPageProps {
        pub title: String,
        pub icon: Option<String>,
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
    }

    #[derive(Clone, Debug, PartialEq)]
    pub enum ListPatch {
        /// The row set changed (count/order/content): the host re-queries its `ListSource`.
        Reload,
        /// An `Automatic`-height row's content size changed; the host re-measures just that row.
        RowSizeInvalidated(usize),
        /// Imperatively scroll the native list so its LAST row is fully visible (a chat timeline
        /// sticking to the newest message). No-op when the list is empty (docs/list.md).
        ScrollToEnd,
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
        APP_TEMP_DIR.with(|d| d.borrow().clone().unwrap_or_else(std::env::temp_dir))
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
        /// Button labels joined with the unit separator (0x1f) — the encoding the nav menu
        /// and combobox shims already use for string lists.
        pub fn buttons_joined(&self) -> String {
            match self {
                PresentSpec::Dialog { buttons, .. } => buttons
                    .iter()
                    .map(|b| b.label.as_str())
                    .collect::<Vec<_>>()
                    .join("\u{1f}"),
                PresentSpec::Prompt { ok, cancel, .. } => format!("{ok}\u{1f}{cancel}"),
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
        /// Filters flattened for the C ABI: each filter is `name|ext1,ext2`, joined by the unit
        /// separator. A trailing `|` (no extensions) means "all files". Empty when unfiltered.
        pub fn filters_joined(&self) -> String {
            self.filters()
                .iter()
                .map(|f| format!("{}|{}", f.name, f.extensions.join(",")))
                .collect::<Vec<_>>()
                .join("\u{1f}")
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

    // tree
    fn insert(&mut self, parent: &Self::Handle, child: &Self::Handle, index: usize);
    fn remove(&mut self, parent: &Self::Handle, child: &Self::Handle);
    fn move_child(&mut self, parent: &Self::Handle, child: &Self::Handle, to: usize);

    // geometry (§7): frames are in the nearest realized native ancestor's space, in points.
    fn measure(&mut self, h: &Self::Handle, kind: PieceKind, p: Proposal) -> Size;
    fn set_frame(&mut self, h: &Self::Handle, frame: Rect, anim: Option<&AnimSpec>);

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

    // recycling list (docs/list.md, §10): day-core hands the `LIST` host its row-pull `source`
    // once, right after realize. A recycling backend stores it and calls it from its native
    // data-source; the default no-op means a backend without list support simply renders nothing.
    fn attach_list(&mut self, _host: &Self::Handle, _source: ListSource) {}

    // menus (§ menus): render `items` with the backend's native menu affordance, firing
    // `Event::MenuAction(id)` (enqueue-only) for each id'd item; `role` items use the native standard
    // command. Default no-op — a toolkit without a menu bar / context menu simply shows nothing.
    /// The application menu (macOS/Windows/Linux menu bar; the app-bar overflow on Android; the
    /// UIMenuBuilder main menu on iPadOS/Catalyst). Replaces any previous app menu.
    fn set_app_menu(&mut self, _items: &[MenuItem]) {}
    /// A context menu for `h`, shown on secondary-click (desktop) or long-press (mobile). Passing an
    /// empty slice removes it.
    fn set_context_menu(&mut self, _h: &Self::Handle, _node: NodeId, _items: &[MenuItem]) {}

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

    // app lifecycle (mobile; desktop backends no-op)
    fn on_suspend(&mut self) {}
    fn on_resume(&mut self) {}
    fn on_memory_warning(&mut self) {}

    // adoption of foreign native handles (polyglot pieces, §15.3)
    fn adopt(&mut self, _raw: RawHandle) -> Self::Handle {
        unimplemented!("this toolkit does not adopt foreign handles yet")
    }
}

#[derive(Clone, Debug)]
pub struct WindowOptions {
    pub title: String,
    pub size: Size,
    pub min_size: Option<Size>,
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

    /// Ordered OS locale preference list (BCP-47), for fluent-langneg (§12.2).
    fn locale_hints(&self) -> Vec<String> {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Open renderer registry (§8.2)
// ---------------------------------------------------------------------------

/// Optional custom measure for a third-party piece (§8.2).
pub type MeasureFn<B> = fn(&mut B, &<B as Toolkit>::Handle, Proposal) -> Size;

/// A third-party piece's per-toolkit implementation. `make` receives the concrete backend
/// (public helper surface) and returns a native handle the backend then owns like any built-in.
pub struct Renderer<B: Toolkit> {
    pub kind: PieceKind,
    pub make: fn(&mut B, &dyn Any, NodeId) -> B::Handle,
    pub update: fn(&mut B, &B::Handle, &dyn Any),
    pub measure: Option<MeasureFn<B>>,
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
        self.map.insert(r.kind, r);
    }
    pub fn get(&self, kind: PieceKind) -> Option<&Renderer<B>> {
        self.map.get(kind)
    }
    pub fn kinds(&self) -> impl Iterator<Item = PieceKind> + '_ {
        self.map.keys().copied()
    }
}

/// Flat numeric encoding of a display list for shim/JNI boundaries (§11, §15.3): per op
/// 9 numbers [kind, a, b, c, d, e, f, g, rgba-bits]; text payloads ride separately in order.
/// Kinds: 0 fill-rect, 1 stroke-rect(g=w), 2 fill-rrect(e=r), 3 fill-ellipse,
/// 4 stroke-ellipse(g=w), 5 stroke-arc(e=start°, f=sweep°, g=w), 6 line(a,b→c,d, g=w),
/// 7 text(a,b=pos, e=size, f=anchor: 0 leading / 1 centered), 8 save, 9 restore,
/// 10 concat(a..f=affine), 11 fill-polygon / 12 stroke-polygon(g=w) — polygon points ride the
/// texts channel as "x,y x,y …" (closed automatically), 13 stroke-rrect(e=r, g=w),
/// 14 set-gradient(f=type: 0 linear with a,b=start / c,d=end unit points; 1 radial with
/// a,b=center unit point, c=unit radius; e=stop count) — the stops ride the texts channel as
/// "offset,aarrggbb offset,aarrggbb …"; the gradient applies to the NEXT fill-shape record
/// (whose color slot is then unused) and is cleared after it. Unit geometry resolves against
/// the filled shape's bounding box, so a radial stretches elliptically in non-square bounds.
///
/// Transports join `texts` with the unit separator U+001F (one entry per kind-7/11/12/14
/// record, in order), so text payloads must not contain U+001F. Known asymmetry:
/// `Fill(Shape::Arc)` encodes as kind 5 (stroke) with width 0 — filled arcs render only on the
/// direct-replay backends (AppKit/UIKit); use a polygon fan if a filled arc must be portable.
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
        k: f64,
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
        nums.extend_from_slice(&[k, a, b, c, d, e, f, g, color_bits(col)]);
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
                if stroke { 1.0 } else { 0.0 },
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
                if stroke { 13.0 } else { 2.0 },
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
                if stroke { 4.0 } else { 3.0 },
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
                5.0,
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
            Shape::Line(p1, p2) => push(6.0, p1.x, p1.y, p2.x, p2.y, 0.0, 0.0, w, col, nums),
            Shape::Polygon(pts) => {
                // Variable-length points ride the texts side-channel ("x,y x,y …"),
                // consumed in record order exactly like text payloads.
                push(
                    if stroke { 12.0 } else { 11.0 },
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
        }
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
                let mut gradient = |geo: [f64; 4], kind: f64, stops: &[(f64, Color)]| {
                    push(
                        14.0,
                        geo[0],
                        geo[1],
                        geo[2],
                        geo[3],
                        stops.len() as f64,
                        kind,
                        0.0,
                        Color::CLEAR,
                        &mut nums,
                    );
                    texts.push(
                        stops
                            .iter()
                            .map(|(o, c)| format!("{o},{:08x}", color_bits(*c) as u32))
                            .collect::<Vec<_>>()
                            .join(" "),
                    );
                };
                let col = match paint {
                    Paint::Solid(c) => *c,
                    Paint::Linear(g) => {
                        gradient([g.start.x, g.start.y, g.end.x, g.end.y], 0.0, &g.stops);
                        // The gradient replaces the shape record's color — but it must be
                        // OPAQUE, not clear: Skia-based decoders (Android Paint, OH_Drawing)
                        // modulate a shader by the paint alpha, so a clear slot would render
                        // the whole gradient invisible.
                        Color::WHITE
                    }
                    Paint::Radial(g) => {
                        gradient([g.center.x, g.center.y, g.radius, 0.0], 1.0, &g.stops);
                        Color::WHITE
                    }
                };
                shape_record(false, shape, 0.0, col, &mut nums, &mut texts);
            }
            DrawOp::Stroke(shape, col, w) => {
                shape_record(true, shape, *w, *col, &mut nums, &mut texts);
            }
            DrawOp::Text {
                text,
                at,
                size,
                color,
                anchor,
            } => {
                push(
                    7.0,
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
                8.0,
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
                9.0,
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
                10.0,
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
