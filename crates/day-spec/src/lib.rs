//! day-spec — the toolkit specification (DESIGN.md §8).
//!
//! Backends depend ONLY on this crate (never on day-core). One backend is linked per binary;
//! `day-core` is monomorphized over the concrete [`Toolkit`].

use std::any::Any;
use std::collections::HashMap;

pub use day_geometry::*;

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
    pub const CANVAS: &str = "day.canvas";
    /// Navigation host (docs/navigation.md): stack on mobile, split panes on desktop.
    pub const NAV: &str = "day.nav";
    /// One destination's native container inside a NAV host.
    pub const NAV_PAGE: &str = "day.nav_page";
    /// Native navigation item list (docs/navigation.md): NSOutlineView source list /
    /// GtkListBox navigation-sidebar / QListWidget / UITableView rows with chevrons.
    pub const NAV_MENU: &str = "day.nav_menu";
}

/// Realized-node identity as seen by backends (day-core's slotmap key, FFI-encoded).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub u64);

/// Default navigation sidebar width (split presentation) until the pane reports its size.
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
    Custom(&'static str, String),
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
    /// The toolkit can present native alert/confirm/sheet/prompt modals (docs/dialogs.md).
    Dialogs,
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
    Fill(Shape, Color),
    Stroke(Shape, Color, f64),
    Text {
        text: String,
        at: Point,
        size: f64,
        color: Color,
        anchor: TextAnchor,
    },
}

// ---------------------------------------------------------------------------
// Built-in piece descriptors: full props (realize) + sparse patches (update).
// One binding = one attribute = one patch value — sparseness by construction (§8.1).
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub enum Font {
    Title,
    Headline,
    #[default]
    Body,
    Caption,
    System(f64),
}

pub mod props {
    use super::*;

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ContainerProps {
        pub background: Option<Color>,
        pub corner_radius: f64,
        pub clips: bool,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct LabelProps {
        pub text: String,
        pub font: Font,
        pub color: Option<Color>,
        pub wraps: bool,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum LabelPatch {
        Text(String),
        Color(Option<Color>),
        Font(Font),
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ButtonProps {
        pub title: String,
        pub enabled: bool,
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

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct ImageProps {
        /// Resolved asset path or name; backend loads through its image pipeline (§18.2).
        pub source: String,
        pub decorative: bool,
    }

    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct CanvasProps {
        pub ops: Vec<DrawOp>,
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
    #[derive(Clone, Debug, Default, PartialEq)]
    pub struct NavMenuProps {
        pub items: Vec<String>,
        pub selected: Option<usize>,
    }
    #[derive(Clone, Debug, PartialEq)]
    pub enum NavMenuPatch {
        /// Programmatic highlight sync — toolkits apply WITHOUT re-emitting
        /// SelectionChanged (the TextField from_native echo rule).
        Selected(Option<usize>),
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
    }

    /// The user's answer to a presentation.
    #[derive(Clone, Debug, PartialEq)]
    pub enum PresentResult {
        /// A dialog button at `index` (in spec order) was chosen.
        Button(i64),
        /// A prompt was confirmed with `text`.
        Text(String),
        /// Dismissed without choosing (tap-outside / Esc / cancel gesture).
        Dismissed,
    }

    impl PresentResult {
        /// Flat wire tag for the C ABI (Qt shim / Android JNI): 0 dismissed, 1 button, 2 text.
        pub fn decode(tag: i32, index: i64, text: String) -> PresentResult {
            match tag {
                1 => PresentResult::Button(index),
                2 => PresentResult::Text(text),
                _ => PresentResult::Dismissed,
            }
        }
    }

    impl PresentSpec {
        /// Backend-facing flattening for the C ABI: `(title, message, button labels, button
        /// roles as ints, sheet-or-prompt fields)`. Pure-Rust backends read the enum directly.
        pub fn title(&self) -> &str {
            match self {
                PresentSpec::Dialog { title, .. } | PresentSpec::Prompt { title, .. } => title,
            }
        }
        pub fn message(&self) -> Option<&str> {
            match self {
                PresentSpec::Dialog { message, .. } | PresentSpec::Prompt { message, .. } => {
                    message.as_deref()
                }
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
            };
            roles
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(",")
        }
    }
}

// ---------------------------------------------------------------------------
// The Toolkit trait (§8.1)
// ---------------------------------------------------------------------------

pub trait Toolkit: Sized + 'static {
    type Handle: Clone;

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

    // pillars
    fn set_a11y(&mut self, _h: &Self::Handle, _a11y: &A11yProps) {}
    fn replay(&mut self, _h: &Self::Handle, _ops: &[DrawOp], _size: Size) {}
    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        Err("snapshot unsupported".into())
    }

    // imperative presentation (docs/dialogs.md): show a native modal for request `req`;
    // the backend answers by enqueuing `Event::PresentResult { req, .. }`. `dismiss` is
    // used only when day resolves programmatically (dayscript) while the modal is still up.
    fn present(&mut self, _req: u64, _spec: &present::PresentSpec) {}
    fn dismiss(&mut self, _req: u64) {}

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
}

impl Default for WindowOptions {
    fn default() -> Self {
        WindowOptions {
            title: "day".into(),
            size: Size::new(480.0, 640.0),
            min_size: None,
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
/// 7 text(a,b=pos, e=size, f=anchor: 0 leading / 1 centered).
pub fn encode_ops(ops: &[DrawOp]) -> (Vec<f64>, Vec<String>) {
    fn color_bits(c: Color) -> f64 {
        let r = (c.r.clamp(0.0, 1.0) * 255.0) as u32;
        let g = (c.g.clamp(0.0, 1.0) * 255.0) as u32;
        let b = (c.b.clamp(0.0, 1.0) * 255.0) as u32;
        let a = (c.a.clamp(0.0, 1.0) * 255.0) as u32;
        ((a << 24) | (r << 16) | (g << 8) | b) as f64
    }
    let mut nums = Vec::with_capacity(ops.len() * 9);
    let mut texts = Vec::new();
    let push = |k: f64,
                a: f64,
                b: f64,
                c: f64,
                d: f64,
                e: f64,
                f: f64,
                g: f64,
                col: Color,
                nums: &mut Vec<f64>| {
        nums.extend_from_slice(&[k, a, b, c, d, e, f, g, color_bits(col)]);
    };
    for op in ops {
        match op {
            DrawOp::Fill(shape, col) | DrawOp::Stroke(shape, col, _) => {
                let w = if let DrawOp::Stroke(_, _, w) = op {
                    *w
                } else {
                    0.0
                };
                let stroke = matches!(op, DrawOp::Stroke(..));
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
                        *col,
                        &mut nums,
                    ),
                    Shape::RoundedRect(r, rad) => push(
                        2.0,
                        r.origin.x,
                        r.origin.y,
                        r.size.width,
                        r.size.height,
                        *rad,
                        0.0,
                        w,
                        *col,
                        &mut nums,
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
                        *col,
                        &mut nums,
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
                        *col,
                        &mut nums,
                    ),
                    Shape::Line(p1, p2) => {
                        push(6.0, p1.x, p1.y, p2.x, p2.y, 0.0, 0.0, w, *col, &mut nums)
                    }
                    Shape::Polygon(_) => {} // post-MVP
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
        }
    }
    (nums, texts)
}
