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
}

/// Realized-node identity as seen by backends (day-core's slotmap key, FFI-encoded).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct NodeId(pub u64);

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
    /// A canvas node was re-framed by layout; re-record (§11).
    FrameChanged(Size),
    Key(KeyEvent),
    Pointer(PointerEvent),
    WindowResized(Size),
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

#[derive(Clone, Debug, PartialEq)]
pub enum DrawOp {
    Fill(Shape, Color),
    Stroke(Shape, Color, f64),
    Text {
        text: String,
        at: Point,
        size: f64,
        color: Color,
        centered: bool,
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
/// 7 text(a,b=pos, e=size, f=centered).
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
                centered,
            } => {
                push(
                    7.0,
                    at.x,
                    at.y,
                    0.0,
                    0.0,
                    *size,
                    if *centered { 1.0 } else { 0.0 },
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
