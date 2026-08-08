//! day-appkit — the macos-appkit backend (DESIGN.md §9). objc2, pure Rust, no shim.
//!
//! `Handle = Retained<NSView>`. Containers are flipped `NSView`s (top-left origin, so Day's
//! frames apply directly and survive diffing). One custom target class (`DayTarget`) forwards
//! target/action + text-delegate callbacks into the Day event sink, node-id keyed (§8.3).

#![allow(unused_unsafe)]
#![cfg(target_os = "macos")]

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;

use linkme::distributed_slice;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{
    AllocAnyThread, DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel,
};
use objc2_app_kit::NSAccessibility as _;
use objc2_app_kit::NSAppearanceCustomization as _;
use objc2_app_kit::NSDraggingInfo as _;
use objc2_app_kit::NSUserInterfaceItemIdentification as _;
use objc2_app_kit::{
    NSAffineTransformNSAppKitAdditions, NSClickGestureRecognizer, NSGestureRecognizer,
    NSGestureRecognizerState, NSPanGestureRecognizer,
};
use objc2_app_kit::{
    NSAnimationContext, NSApplication, NSApplicationActivationPolicy, NSBackingStoreType,
    NSBitmapImageFileType, NSBox, NSBoxType, NSButton, NSColor, NSControl,
    NSControlTextEditingDelegate, NSEventType, NSFont, NSGraphicsContext, NSLineBreakMode, NSMenu,
    NSMenuItem, NSProgressIndicator, NSProgressIndicatorStyle, NSResponder, NSScrollView, NSSlider,
    NSSwitch, NSTabView, NSTabViewItem, NSText, NSTextField, NSTextFieldDelegate, NSTextMovement,
    NSTextMovementUserInfoKey, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_app_kit::{
    NSApplicationDidBecomeActiveNotification, NSApplicationWillResignActiveNotification,
    NSApplicationWillTerminateNotification,
};
use objc2_app_kit::{
    NSOutlineViewDataSource, NSOutlineViewDelegate, NSSplitViewDelegate, NSTabViewDelegate,
};
use objc2_app_kit::{NSTableColumn, NSTableView, NSTableViewDataSource, NSTableViewDelegate};
use objc2_core_foundation::CGAffineTransform;
use objc2_foundation::{
    NSAffineTransform, NSAffineTransformStruct, NSDictionary, NSNotification, NSNumber, NSObject,
    NSPoint, NSRect, NSSize, NSString,
};
use objc2_quartz_core::{
    CAMediaTimingFunction, kCAMediaTimingFunctionEaseIn, kCAMediaTimingFunctionEaseInEaseOut,
    kCAMediaTimingFunctionEaseOut, kCAMediaTimingFunctionLinear,
};

use day_spec::present;
use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Builtin, Cap, Curve, DrawOp, Event, EventSink, Font, ListSource, NodeId,
    PieceKind, Platform, Point, Proposal, RawHandle, Rect, Registry, Renderer, Size, Support,
    Toolkit, Transform, WINDOW_NODE, WindowOptions, kinds,
};

pub type Handle = Retained<NSView>;

// Built-in leaf pieces split into modules (moved in from their satellite crates 2026-07).
mod picker;
mod textarea;
mod toolbar;

pub mod ext;
pub use ext::*;

// ---------------------------------------------------------------------------
// Event plumbing: node-id keyed sink, thread-local (single UI thread)
// ---------------------------------------------------------------------------

/// The day-core event sink (node-id keyed).
type Sink = Rc<dyn Fn(NodeId, Event)>;

thread_local! {
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// Keeps each control's `DayTarget` alive (target/action holds it weakly).
    static TARGETS: RefCell<HashMap<usize, Retained<DayTarget>>> = RefCell::new(HashMap::new());
}

/// Emit an event into day-core's queue (public: external Day Piece renderers use this too).
pub fn emit(id: NodeId, ev: Event) {
    let sink = SINK.with(|s| s.borrow().clone());
    if let Some(sink) = sink {
        sink(id, ev);
    }
}

fn ptr_of(v: &NSView) -> usize {
    (v as *const NSView).cast::<()>() as usize
}

/// Day `Role` → the `NSAccessibilityRole` constant to apply (§13). `None` for `Role::None` —
/// Day leaves native controls' own roles untouched and only applies explicit canvas/custom roles.
fn ns_role(role: day_spec::Role) -> Option<&'static objc2_app_kit::NSAccessibilityRole> {
    use day_spec::Role;
    use objc2_app_kit::{
        NSAccessibilityButtonRole, NSAccessibilityCheckBoxRole, NSAccessibilityGroupRole,
        NSAccessibilityImageRole, NSAccessibilityLevelIndicatorRole, NSAccessibilitySliderRole,
        NSAccessibilityStaticTextRole, NSAccessibilityTextFieldRole,
    };
    unsafe {
        Some(match role {
            Role::Button => NSAccessibilityButtonRole,
            Role::Toggle => NSAccessibilityCheckBoxRole,
            Role::Slider => NSAccessibilitySliderRole,
            Role::TextInput => NSAccessibilityTextFieldRole,
            Role::Heading(_) => NSAccessibilityStaticTextRole, // macOS has no arbitrary-view heading role
            Role::Image => NSAccessibilityImageRole,
            Role::Meter => NSAccessibilityLevelIndicatorRole,
            Role::Group => NSAccessibilityGroupRole,
            Role::None => return None,
        })
    }
}

/// Native `AXRole` string → Day `Role` (best-effort, for `read_a11y`/`a11y_audit`).
fn day_role_from_ns(ax: &str) -> day_spec::Role {
    use day_spec::Role;
    match ax {
        "AXButton" => Role::Button,
        "AXCheckBox" => Role::Toggle,
        "AXSlider" => Role::Slider,
        "AXTextField" => Role::TextInput,
        "AXStaticText" => Role::Heading(0), // ambiguous with plain text; audit ignores heading level
        "AXImage" => Role::Image,
        "AXLevelIndicator" | "AXProgressIndicator" => Role::Meter,
        "AXGroup" => Role::Group,
        _ => Role::None,
    }
}

// ---------------------------------------------------------------------------
// DayTarget — target/action + text delegate trampoline
// ---------------------------------------------------------------------------

struct TargetIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayTarget"]
    #[ivars = TargetIvars]
    struct DayTarget;

    unsafe impl NSObjectProtocol for DayTarget {}
    unsafe impl NSTextFieldDelegate for DayTarget {}

    impl DayTarget {
        #[unsafe(method(action:))]
        fn action(&self, sender: &NSControl) {
            let node = self.ivars().node;
            if sender.downcast_ref::<NSSwitch>().is_some() {
                emit(node, Event::ToggleChanged(unsafe { sender.integerValue() } != 0));
            } else if sender.downcast_ref::<NSSlider>().is_some() {
                let value = unsafe { sender.doubleValue() };
                // The live value first: bindings follow this, so the UI tracks the thumb.
                emit(node, Event::ValueChanged(value));
                // Then, once, the value the user actually chose. The slider is `continuous`, so
                // this action fires on every tick of a drag; AppKit's own way to tell where in the
                // gesture you are is the event that provoked it. A mouse-up ends a drag; a key
                // press (arrow keys) moves the value by one discrete step and is already settled;
                // anything else — mouse-down, mouse-dragged — is mid-gesture and commits nothing.
                if slider_value_settled() {
                    emit(node, Event::ValueCommitted(value));
                }
            } else {
                emit(node, Event::Pressed);
            }
        }

        /// The stack-nav back header's button (docs/navigation.md): a day-initiated pop — the
        /// nav host's handler writes it into the path signal, which reconciles the pop.
        #[unsafe(method(navBack:))]
        fn nav_back(&self, _sender: &NSControl) {
            emit(
                self.ivars().node,
                Event::NavBack {
                    already_popped: false,
                },
            );
        }
    }

    unsafe impl NSControlTextEditingDelegate for DayTarget {
        #[unsafe(method(controlTextDidChange:))]
        fn control_text_did_change(&self, notification: &NSNotification) {
            let node = self.ivars().node;
            if let Some(obj) = unsafe { notification.object() }
                && let Ok(tf) = obj.downcast::<NSTextField>() {
                    emit(node, Event::TextChanged(unsafe { tf.stringValue() }.to_string()));
                }
        }

        /// End of an editing session (docs/focus.md). Return submits — AppKit keeps the field
        /// first responder and re-selects, so it is not a focus loss; every other movement
        /// (tab, click-away, cancel) reports `FocusChanged(false)`.
        #[unsafe(method(controlTextDidEndEditing:))]
        fn control_text_did_end_editing(&self, notification: &NSNotification) {
            let node = self.ivars().node;
            let movement = unsafe { notification.userInfo() }
                .and_then(|ui| ui.objectForKey(unsafe { NSTextMovementUserInfoKey }.as_ref()))
                .and_then(|n| n.downcast::<NSNumber>().ok())
                .map(|n| NSTextMovement(n.integerValue()))
                .unwrap_or(NSTextMovement::Other);
            if movement == NSTextMovement::Return {
                emit(node, Event::Submitted);
            } else {
                emit(node, Event::FocusChanged(false));
            }
        }
    }
);

impl DayTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

/// Whether the NSEvent currently being dispatched ends a slider's interaction — see the
/// `ValueCommitted` emission in `DayTarget::action:`. AppKit gives a continuous slider no
/// "drag ended" callback, so the event that provoked the action is what says where in the gesture
/// we are: a mouse-up ends a drag, an arrow key moves one discrete step and is already settled,
/// and mouse-down/mouse-dragged are mid-gesture. No current event at all — a programmatic
/// `setDoubleValue:` that fires the action — is not a user commit either.
fn slider_value_settled() -> bool {
    let Some(mtm) = MainThreadMarker::new() else {
        return false;
    };
    let Some(event) = NSApplication::sharedApplication(mtm).currentEvent() else {
        return false;
    };
    matches!(
        unsafe { event.r#type() },
        NSEventType::LeftMouseUp | NSEventType::KeyDown | NSEventType::KeyUp
    )
}

// ---------------------------------------------------------------------------
// DayTextField — NSTextField that reports focus gain (docs/focus.md)
// ---------------------------------------------------------------------------

struct FieldIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSTextField))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayTextField"]
    #[ivars = FieldIvars]
    struct DayTextField;

    impl DayTextField {
        /// Focus gain. Key events go to the shared field editor, but the field itself receives
        /// `becomeFirstResponder` first — the reliable gain hook (`controlTextDidBeginEditing:`
        /// waits for the first keystroke). Loss comes from `controlTextDidEndEditing:` on the
        /// delegate.
        #[unsafe(method(becomeFirstResponder))]
        fn become_first_responder(&self) -> bool {
            let ok: bool = unsafe { msg_send![super(self), becomeFirstResponder] };
            if ok {
                emit(self.ivars().node, Event::FocusChanged(true));
            }
            ok
        }
    }
);

impl DayTextField {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FieldIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// DayFlipped — top-left-origin container view
// ---------------------------------------------------------------------------

/// A `background(..)` fill: `(r, g, b, a, corner_radius)`, painted in `drawRect` with NSColor.
type Surface = (f64, f64, f64, f64, f64);

#[derive(Default)]
struct FlippedIvars {
    surface: Cell<Option<Surface>>,
    /// SurfaceRole::SectionCard: `(radius,)` — drawn with a DYNAMIC system fill resolved at
    /// draw time, so the card tracks light/dark appearance changes automatically.
    section_card: Cell<Option<f64>>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayFlipped"]
    #[ivars = FlippedIvars]
    struct DayFlipped;

    impl DayFlipped {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        /// Show the attached context menu (docs/menus.md) explicitly rather than relying on
        /// `NSResponder`'s default `.menu` display — popping it ourselves is deterministic
        /// regardless of how the click threads through Day's container hierarchy.
        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &objc2_app_kit::NSEvent) {
            if let Some(menu) = self.menu() {
                unsafe {
                    objc2_app_kit::NSMenu::popUpContextMenu_withEvent_forView(&menu, event, self)
                };
            } else {
                let _: () = unsafe { msg_send![super(self), rightMouseDown: event] };
            }
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            if let Some(radius) = self.ivars().section_card.get() {
                let bounds = self.bounds();
                unsafe {
                    // quaternarySystemFill (macOS 14+) is the grouped-card material System
                    // Settings uses; older systems fall back to the control background.
                    let cls = objc2::class!(NSColor);
                    let has: bool = msg_send![cls, respondsToSelector: objc2::sel!(quaternarySystemFillColor)];
                    let color: objc2::rc::Retained<NSColor> = if has {
                        msg_send![cls, quaternarySystemFillColor]
                    } else {
                        msg_send![cls, controlBackgroundColor]
                    };
                    color.setFill();
                    let path = objc2_app_kit::NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                        bounds, radius, radius,
                    );
                    path.fill();
                }
            }
            if let Some((r, g, b, a, radius)) = self.ivars().surface.get() {
                let bounds = self.bounds();
                unsafe {
                    NSColor::colorWithSRGBRed_green_blue_alpha(r, g, b, a).setFill();
                    let path = if radius > 0.0 {
                        objc2_app_kit::NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(
                            bounds, radius, radius,
                        )
                    } else {
                        objc2_app_kit::NSBezierPath::bezierPathWithRect(bounds)
                    };
                    path.fill();
                }
            }
        }
    }
);

impl DayFlipped {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FlippedIvars::default());
        unsafe { msg_send![super(this), init] }
    }

    /// Apply a `background`/`corner_radius` surface. The fill (rounded by `corner_radius`) is
    /// drawn in `drawRect` with NSColor — deliberately NOT via the layer's `backgroundColor`,
    /// whose CGColorRef argument objc2's `msg_send` cannot type-check. A rounded child clip does
    /// use the CALayer (`cornerRadius` + `masksToBounds` are a CGFloat + BOOL, which are fine).
    /// SurfaceRole::SectionCard — the fill resolves dynamically in drawRect (theme-adaptive).
    fn set_section_card(&self, corner_radius: f64) {
        self.ivars().section_card.set(Some(corner_radius));
        unsafe {
            let _: () = msg_send![self, setNeedsDisplay: true];
        }
    }

    fn set_surface(&self, bg: Option<day_spec::Color>, corner_radius: f64, clips: bool) {
        self.ivars()
            .surface
            .set(bg.map(|c| (c.r, c.g, c.b, c.a, corner_radius)));
        unsafe {
            let _: () = msg_send![self, setNeedsDisplay: true];
            if clips || corner_radius > 0.0 {
                let _: () = msg_send![self, setWantsLayer: true];
                let layer: *mut objc2::runtime::AnyObject = msg_send![self, layer];
                if !layer.is_null() {
                    let _: () = msg_send![layer, setCornerRadius: corner_radius];
                    let _: () = msg_send![layer, setMasksToBounds: true];
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// DayCanvas — a flipped view replaying the Day display list in drawRect (§11)
// ---------------------------------------------------------------------------

thread_local! {
    static OPS: RefCell<HashMap<usize, Vec<DrawOp>>> = RefCell::new(HashMap::new());
}

struct CanvasIvars;

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayCanvas"]
    #[ivars = CanvasIvars]
    struct DayCanvas;

    impl DayCanvas {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
            let ptr = (self as *const DayCanvas).cast::<NSView>() as usize;
            let ops = OPS.with(|m| m.borrow().get(&ptr).cloned()).unwrap_or_default();
            for op in &ops {
                draw_op(op);
            }
        }
    }
);

impl DayCanvas {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(CanvasIvars);
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// DayGesture — tap/drag recognizer target, node-id keyed (docs/shapes.md)
// ---------------------------------------------------------------------------

struct GestureIvars {
    node: NodeId,
    is_drag: bool,
}

thread_local! {
    /// Keeps each view's gesture targets alive + records which gestures are attached (idempotent).
    static GESTURES: RefCell<HashMap<usize, Vec<Retained<DayGesture>>>> =
        RefCell::new(HashMap::new());
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayGesture"]
    #[ivars = GestureIvars]
    struct DayGesture;

    unsafe impl NSObjectProtocol for DayGesture {}

    impl DayGesture {
        #[unsafe(method(fire:))]
        fn fire(&self, g: &NSGestureRecognizer) {
            let node = self.ivars().node;
            let view = g.view();
            let loc = g.locationInView(view.as_deref());
            let at = Point::new(loc.x, loc.y);
            if self.ivars().is_drag {
                let obj: &objc2::runtime::AnyObject = g.as_ref();
                let (translation, phase) = if let Some(pan) = obj.downcast_ref::<NSPanGestureRecognizer>() {
                    let t = unsafe { pan.translationInView(view.as_deref()) };
                    let phase = match g.state() {
                        NSGestureRecognizerState::Began => day_spec::DragPhase::Began,
                        NSGestureRecognizerState::Ended
                        | NSGestureRecognizerState::Cancelled
                        | NSGestureRecognizerState::Failed => day_spec::DragPhase::Ended,
                        _ => day_spec::DragPhase::Changed,
                    };
                    (Point::new(t.x, t.y), phase)
                } else {
                    (Point::ZERO, day_spec::DragPhase::Changed)
                };
                emit(node, Event::Drag { phase, location: at, translation });
            } else {
                emit(node, Event::Tap(at));
            }
        }
    }
);

impl DayGesture {
    fn new(mtm: MainThreadMarker, node: NodeId, is_drag: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(GestureIvars { node, is_drag });
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// Navigation (docs/navigation.md): NSSplitView host, sidebar + detail panes.
// Page FRAMES are pane-owned (autoresized); Day lays content inside the size each
// page reports from setFrameSize:. Day's set_frame on pages is skipped.
// ---------------------------------------------------------------------------

/// The stack presentation's back header (desktop has no system back affordance — this bar
/// gives a pushed page its way out, like AdwNavigationView's header on GTK). Hidden at root.
struct NavHeader {
    bar: Retained<NSView>,
    title: Retained<NSTextField>,
    /// Title per stack level (index 0 = root); the field shows the last.
    titles: Vec<String>,
    _target: Retained<DayTarget>,
}

/// Height of the stack-nav back header while a pushed page is showing.
const NAV_HEADER_H: f64 = 34.0;

struct NavState {
    sidebar_wrap: Retained<NSView>,
    detail_wrap: Retained<NSView>,
    /// Keeps the split's delegate (weakly referenced by AppKit) alive with the host.
    _split_delegate: Retained<DaySplitDelegate>,
    /// Detail pages in stack order (the sidebar page is not in here in split mode; in stack
    /// mode `split == false`, the root page is here too, so push/pop visibility covers it).
    pages: Vec<Retained<NSView>>,
    positioned: bool,
    /// Sidebar+detail split (a selector Sidebar) vs. a pure push/pop stack (a `stack`).
    split: bool,
    /// Back header (stack presentation only).
    header: Option<NavHeader>,
}

/// The frame a stack page occupies inside the detail wrap: full bounds at the root, inset
/// below the back header while a pushed page is showing.
fn nav_page_frame(wrap: &NSView, header_visible: bool) -> NSRect {
    let b = wrap.bounds();
    if header_visible {
        NSRect::new(
            NSPoint::new(0.0, NAV_HEADER_H),
            NSSize::new(b.size.width, (b.size.height - NAV_HEADER_H).max(0.0)),
        )
    } else {
        b
    }
}

/// Show/hide the stack-nav back header for the given page stack and re-frame the pages under
/// it. The header shows exactly while a pushed page (depth ≥ 1 above the root) is on top; each
/// page's `DayNavPage::setFrameSize` reports the new size so Day re-lays its content.
fn sync_nav_header(hdr: &NavHeader, wrap: &NSView, pages: &[Retained<NSView>]) {
    let visible = pages.len() >= 2;
    hdr.bar.setHidden(!visible);
    unsafe {
        hdr.title.setStringValue(&NSString::from_str(
            hdr.titles.last().map(String::as_str).unwrap_or(""),
        ));
    }
    let frame = nav_page_frame(wrap, visible);
    for page in pages {
        unsafe { page.setFrame(frame) };
    }
}

thread_local! {
    static NAV_STATE: RefCell<HashMap<usize, NavState>> = RefCell::new(HashMap::new());
    /// Handles whose frames are native-owned (nav pages): set_frame skips them.
    static NAV_PAGES: RefCell<std::collections::HashSet<usize>> =
        RefCell::new(std::collections::HashSet::new());
}

// ---------------------------------------------------------------------------
// DaySplitDelegate — the sidebar pane holds its width through NSSplitView's own
// layout passes (docs/navigation.md).
// ---------------------------------------------------------------------------

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DaySplitDelegate"]
    struct DaySplitDelegate;

    unsafe impl NSObjectProtocol for DaySplitDelegate {}

    unsafe impl NSSplitViewDelegate for DaySplitDelegate {
        /// Size changes go to the DETAIL pane; the sidebar (pane 0) holds its width. Without
        /// this, the split's own layout pass after a section switch (the swap of the detail
        /// page subview dirties the split) redistributed the panes and collapsed the sidebar
        /// to zero — and `set_frame`'s divider restore only runs on window resizes, so
        /// nothing ever brought it back. In stack mode the same rule pins the (deliberately
        /// zero-width) sidebar pane closed.
        #[unsafe(method(splitView:shouldAdjustSizeOfSubview:))]
        fn should_adjust(
            &self,
            sv: &objc2_app_kit::NSSplitView,
            subview: &NSView,
        ) -> objc2::runtime::Bool {
            let subs = sv.subviews();
            if subs.count() == 0 {
                return objc2::runtime::Bool::YES;
            }
            let first = subs.objectAtIndex(0);
            objc2::runtime::Bool::new(!std::ptr::eq(
                &*first as *const NSView,
                subview as *const NSView,
            ))
        }
    }
);

impl DaySplitDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        unsafe { msg_send![Self::alloc(mtm), init] }
    }
}

struct NavPageIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayNavPage"]
    #[ivars = NavPageIvars]
    struct DayNavPage;

    impl DayNavPage {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            // Pane-driven resize (splitter drag, window resize): report the usable size
            // so NavLayout re-lays this page's content (enqueue-only, §8.3).
            emit(
                self.ivars().node,
                Event::FrameChanged(Size::new(size.width, size.height)),
            );
        }
    }
);

impl DayNavPage {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavPageIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// Tabs (docs/tabs.md): NSTabView host + DayTabDelegate for selection.
// ---------------------------------------------------------------------------

struct TabDelegateIvars {
    node: NodeId,
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: std::cell::Cell<bool>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayTabDelegate"]
    #[ivars = TabDelegateIvars]
    struct DayTabDelegate;

    unsafe impl NSObjectProtocol for DayTabDelegate {}

    unsafe impl NSTabViewDelegate for DayTabDelegate {
        #[unsafe(method(tabView:didSelectTabViewItem:))]
        fn did_select(&self, tabview: &NSTabView, item: &NSTabViewItem) {
            if self.ivars().suppress.get() {
                return;
            }
            let idx = unsafe { tabview.indexOfTabViewItem(item) };
            emit(self.ivars().node, Event::SelectionChanged(idx as i64));
        }
    }
);

impl DayTabDelegate {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TabDelegateIvars {
            node,
            suppress: std::cell::Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }
}

struct TabState {
    delegate: Retained<DayTabDelegate>,
    /// Tab to select once its item has been inserted (NSTabView selects the first by default).
    initial: usize,
}

thread_local! {
    /// TABS host view ptr → its delegate + initial selection.
    static TAB_STATE: RefCell<HashMap<usize, TabState>> = RefCell::new(HashMap::new());
    /// TABS_PAGE view ptr → its tab label (read by the host on insert).
    static TAB_TITLES: RefCell<HashMap<usize, String>> = RefCell::new(HashMap::new());
}

// ---------------------------------------------------------------------------
// DayNavMenuData — flat NSOutlineView source list for nav_menu() (docs/navigation.md)
// ---------------------------------------------------------------------------

struct NavMenuIvars {
    node: NodeId,
    items: RefCell<Vec<Retained<NSString>>>,
    /// Pre-resolved template icons per row (docs/navigation.md), `None` where a row has no icon.
    /// Template images tint with the source-list's selection/appearance, so they read in light,
    /// dark, and while selected — the macOS-idiomatic sidebar look (Finder/Mail).
    icons: RefCell<Vec<Option<Retained<objc2_app_kit::NSImage>>>>,
    /// Per-row icon tint (docs/vectors.md): recolors the template glyph via contentTintColor;
    /// `None` keeps the source-list's neutral template tint.
    tints: RefCell<Vec<Option<day_spec::Color>>>,
    /// Trailing accessory per item row (an unread count), `None` where a row has none.
    badges: RefCell<Vec<Option<Retained<NSString>>>>,
    /// Per-row context menu (docs/menus.md), empty = none. Built into an NSMenu and attached
    /// to the row's cell view, so a secondary click pops it like any native sidebar menu.
    menus: RefCell<Vec<Vec<day_spec::MenuItem>>>,
    /// The outline's rows, in display order: `Some(i)` is item `i`, `None` is a section
    /// header. Day addresses rows by ITEM index, so every selection crossing the boundary
    /// goes through this map — a header must never shift what index 3 means.
    rows: RefCell<Vec<Option<usize>>>,
    /// One retained NSString per outline row, used as the outline's item identity.
    row_objects: RefCell<Vec<Retained<NSString>>>,
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: std::cell::Cell<bool>,
}

define_class!(
    #[unsafe(super(objc2_app_kit::NSTableRowView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayNavRowView"]
    struct DayNavRowView;

    unsafe impl NSObjectProtocol for DayNavRowView {}

    impl DayNavRowView {
        /// Always report the emphasized (key-window) state, so the selected sidebar row draws
        /// the accent pill with auto-whitened label/icon in BOTH themes. Without this, a run
        /// whose window never becomes key (scripted screenshot captures) falls back to the
        /// unemphasized selection fill, which under a forced appearance rendered as a
        /// near-black pill that swallowed the row's label.
        #[unsafe(method(isEmphasized))]
        fn is_emphasized(&self) -> bool {
            true
        }
    }
);

define_class!(
    #[unsafe(super(objc2_app_kit::NSOutlineView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayNavOutlineView"]
    struct DayNavOutlineView;

    unsafe impl NSObjectProtocol for DayNavOutlineView {}

    impl DayNavOutlineView {
        /// The clicked row's context menu (docs/menus.md): NSTableView-family views consume
        /// right-clicks themselves, so per-row menus are served HERE — resolve the row under
        /// the click, then build the same NSMenu the piece decorator would.
        #[unsafe(method_id(menuForEvent:))]
        fn menu_for_event(
            &self,
            event: &objc2_app_kit::NSEvent,
        ) -> Option<Retained<NSMenu>> {
            let point = self.convertPoint_fromView(unsafe { event.locationInWindow() }, None);
            let row = unsafe { self.rowAtPoint(point) };
            let data = NAV_OUTLINE_MENUS
                .with(|m| m.borrow().get(&(self as *const _ as usize)).cloned());
            // `define_class!` rewrites the return, so no early `return` — one expression.
            match data {
                None => None,
                Some(d) => {
                    let items = d
                        .item_of_row(row)
                        .and_then(|i| d.ivars().menus.borrow().get(i).cloned())
                        .unwrap_or_default();
                    if items.is_empty() {
                        None
                    } else {
                        Some(build_ns_menu(d.mtm(), "", &items))
                    }
                }
            }
        }
    }
);

thread_local! {
    /// Outline ptr → its data source, for [`DayNavOutlineView::menu_for_event`]'s row lookup.
    static NAV_OUTLINE_MENUS: RefCell<HashMap<usize, Retained<DayNavMenuData>>> =
        RefCell::new(HashMap::new());
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayNavMenuData"]
    #[ivars = NavMenuIvars]
    struct DayNavMenuData;

    unsafe impl NSObjectProtocol for DayNavMenuData {}

    unsafe impl NSOutlineViewDataSource for DayNavMenuData {
        #[unsafe(method(outlineView:numberOfChildrenOfItem:))]
        fn number_of_children(
            &self,
            _ov: &objc2_app_kit::NSOutlineView,
            item: Option<&objc2::runtime::AnyObject>,
        ) -> isize {
            if item.is_none() {
                self.ivars().row_objects.borrow().len() as isize
            } else {
                0
            }
        }

        #[unsafe(method_id(outlineView:child:ofItem:))]
        fn child_of_item(
            &self,
            _ov: &objc2_app_kit::NSOutlineView,
            index: isize,
            _item: Option<&objc2::runtime::AnyObject>,
        ) -> Retained<objc2::runtime::AnyObject> {
            let objects = self.ivars().row_objects.borrow();
            let ns = objects[index as usize].clone();
            unsafe { objc2::rc::Retained::cast_unchecked(ns) }
        }

        #[unsafe(method(outlineView:isItemExpandable:))]
        fn is_expandable(
            &self,
            _ov: &objc2_app_kit::NSOutlineView,
            _item: &objc2::runtime::AnyObject,
        ) -> bool {
            false
        }
    }

    unsafe impl NSControlTextEditingDelegate for DayNavMenuData {}

    unsafe impl NSOutlineViewDelegate for DayNavMenuData {
        /// Section headers are AppKit group rows: the source-list treatment (small, bold,
        /// secondary) comes free, and they are excluded from selection below.
        #[unsafe(method(outlineView:isGroupItem:))]
        fn is_group_item(
            &self,
            ov: &objc2_app_kit::NSOutlineView,
            item: &objc2::runtime::AnyObject,
        ) -> bool {
            let row = unsafe { ov.rowForItem(Some(item)) };
            row >= 0 && self.item_of_row(row).is_none()
        }

        #[unsafe(method(outlineView:shouldSelectItem:))]
        fn should_select(
            &self,
            ov: &objc2_app_kit::NSOutlineView,
            item: &objc2::runtime::AnyObject,
        ) -> bool {
            let row = unsafe { ov.rowForItem(Some(item)) };
            self.item_of_row(row).is_some()
        }

        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn view_for(
            &self,
            ov: &objc2_app_kit::NSOutlineView,
            _col: Option<&objc2_app_kit::NSTableColumn>,
            item: &objc2::runtime::AnyObject,
        ) -> Option<Retained<NSView>> {
            let mtm = self.mtm();
            // No early returns: the method_id macro owns the return conversion.
            item.downcast_ref::<NSString>().map(|text| {
                let row = unsafe { ov.rowForItem(Some(item)) };
                let Some(index) = self.item_of_row(row) else {
                    // A section header: a plain secondary label. AppKit gives group rows their
                    // own typography, so this only supplies the text.
                    let cell = unsafe { objc2_app_kit::NSTableCellView::new(mtm) };
                    let label = unsafe { NSTextField::labelWithString(text, mtm) };
                    unsafe {
                        label.setTranslatesAutoresizingMaskIntoConstraints(false);
                        cell.addSubview(&label);
                        cell.setTextField(Some(&label));
                        objc2_app_kit::NSLayoutConstraint::activateConstraints(
                            &objc2_foundation::NSArray::from_retained_slice(&[
                                label
                                    .leadingAnchor()
                                    .constraintEqualToAnchor_constant(&cell.leadingAnchor(), 0.0),
                                label
                                    .trailingAnchor()
                                    .constraintEqualToAnchor_constant(&cell.trailingAnchor(), -6.0),
                                label
                                    .centerYAnchor()
                                    .constraintEqualToAnchor(&cell.centerYAnchor()),
                            ]),
                        );
                    }
                    return objc2::rc::Retained::into_super(cell);
                };
                let icon = self
                    .ivars()
                    .icons
                    .borrow()
                    .get(index)
                    .and_then(|o| o.clone());
                let badge = self
                    .ivars()
                    .badges
                    .borrow()
                    .get(index)
                    .and_then(|o| o.clone());
                // Indent the label past the icon when there is one, so labels align icon-to-text.
                let label_x = if icon.is_some() { 26.0 } else { 0.0 };
                let cell = unsafe { objc2_app_kit::NSTableCellView::new(mtm) };
                let label = unsafe { NSTextField::labelWithString(text, mtm) };
                unsafe {
                    // Feed titles are arbitrarily long, so the label truncates with an
                    // ellipsis — but only within a frame that ENDS where the cell does.
                    // Autoresizing could not deliver that: the label was born 10pt wide in a
                    // zero-width cell, so a width-sizable mask grew it to (cell + 10) and the
                    // overhang was clipped by the sidebar's edge, cutting names mid-glyph
                    // with no ellipsis at all. Pin it to the cell instead.
                    label.cell().inspect(|c| {
                        c.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail)
                    });
                    label.setTranslatesAutoresizingMaskIntoConstraints(false);
                    cell.addSubview(&label);
                    cell.setTextField(Some(&label));
                    // The badge sits at the trailing edge and keeps its intrinsic width; the
                    // label truncates into whatever is left, so a long feed name never pushes
                    // its own unread count off the pane.
                    let trailing: Retained<NSView> = match &badge {
                        Some(text) => {
                            let b = NSTextField::labelWithString(text, mtm);
                            b.setTranslatesAutoresizingMaskIntoConstraints(false);
                            b.setTextColor(Some(&objc2_app_kit::NSColor::secondaryLabelColor()));
                            b.setAlignment(objc2_app_kit::NSTextAlignment::Right);
                            b.setFont(Some(
                                &objc2_app_kit::NSFont::monospacedDigitSystemFontOfSize_weight(
                                    objc2_app_kit::NSFont::smallSystemFontSize(),
                                    objc2_app_kit::NSFontWeightRegular,
                                ),
                            ));
                            b.setContentCompressionResistancePriority_forOrientation(
                                objc2_app_kit::NSLayoutPriorityRequired,
                                objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
                            );
                            cell.addSubview(&b);
                            let v: Retained<NSView> =
                                objc2::rc::Retained::into_super(objc2::rc::Retained::into_super(b));
                            v
                        }
                        None => {
                            let spacer = NSView::new(mtm);
                            spacer.setTranslatesAutoresizingMaskIntoConstraints(false);
                            cell.addSubview(&spacer);
                            spacer
                        }
                    };
                    label.setContentCompressionResistancePriority_forOrientation(
                        objc2_app_kit::NSLayoutPriorityDefaultLow,
                        objc2_app_kit::NSLayoutConstraintOrientation::Horizontal,
                    );
                    objc2_app_kit::NSLayoutConstraint::activateConstraints(
                        &objc2_foundation::NSArray::from_retained_slice(&[
                            label
                                .leadingAnchor()
                                .constraintEqualToAnchor_constant(&cell.leadingAnchor(), label_x),
                            label
                                .trailingAnchor()
                                .constraintLessThanOrEqualToAnchor_constant(
                                    &trailing.leadingAnchor(),
                                    -6.0,
                                ),
                            label
                                .centerYAnchor()
                                .constraintEqualToAnchor(&cell.centerYAnchor()),
                            trailing
                                .trailingAnchor()
                                .constraintEqualToAnchor_constant(&cell.trailingAnchor(), -6.0),
                            trailing
                                .centerYAnchor()
                                .constraintEqualToAnchor(&cell.centerYAnchor()),
                        ]),
                    );
                }
                if let Some(img) = icon {
                    let tint = self.ivars().tints.borrow().get(index).copied().flatten();
                    let iv = unsafe { objc2_app_kit::NSImageView::new(mtm) };
                    unsafe {
                        iv.setImage(Some(&img));
                        // Per-row tint (docs/vectors.md): the icons are template images, so
                        // contentTintColor recolors the alpha mask; None keeps the neutral look.
                        if let Some(t) = tint {
                            iv.setContentTintColor(Some(&nscolor(t)));
                        }
                        iv.setImageScaling(
                            objc2_app_kit::NSImageScaling::ScaleProportionallyUpOrDown,
                        );
                        iv.setFrame(NSRect::new(NSPoint::new(2.0, 2.0), NSSize::new(18.0, 18.0)));
                        cell.addSubview(&iv);
                        cell.setImageView(Some(&iv));
                    }
                }
                objc2::rc::Retained::into_super(cell)
            })
        }

        #[unsafe(method_id(outlineView:rowViewForItem:))]
        fn row_view_for(
            &self,
            _ov: &objc2_app_kit::NSOutlineView,
            _item: &objc2::runtime::AnyObject,
        ) -> Retained<objc2_app_kit::NSTableRowView> {
            let row: Retained<DayNavRowView> =
                unsafe { msg_send![DayNavRowView::alloc(self.mtm()), init] };
            objc2::rc::Retained::into_super(row)
        }

        #[unsafe(method(outlineViewSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            if self.ivars().suppress.get() {
                return;
            }
            let Some(obj) = (unsafe { notification.object() }) else {
                return;
            };
            let Ok(ov) = obj.downcast::<objc2_app_kit::NSOutlineView>() else {
                return;
            };
            let row = unsafe { ov.selectedRow() };
            if let Some(item) = self.item_of_row(row) {
                emit(self.ivars().node, Event::SelectionChanged(item as i64));
            }
        }
    }
);

fn resolve_nav_icons(icons: &[Option<String>]) -> Vec<Option<Retained<objc2_app_kit::NSImage>>> {
    // A bundled icon name → a template NSImage (tinted by the source list); `None` per iconless row.
    icons
        .iter()
        .map(|ic| {
            let name = ic.as_deref()?;
            // Prefer the glyph SVG (docs/vectors.md): NSImage renders it at display size.
            let path = day_spec::resource::resolve_vector_svg(name)
                .or_else(|| day_spec::resource::resolve_image_file(name))?;
            use objc2::AllocAnyThread as _;
            let img = unsafe {
                objc2_app_kit::NSImage::initWithContentsOfFile(
                    objc2_app_kit::NSImage::alloc(),
                    &NSString::from_str(&path.to_string_lossy()),
                )
            }?;
            unsafe { img.setTemplate(true) };
            Some(img)
        })
        .collect()
}

impl DayNavMenuData {
    #[allow(clippy::too_many_arguments)]
    fn new(
        mtm: MainThreadMarker,
        node: NodeId,
        items: &[String],
        icons: &[Option<String>],
        badges: &[Option<String>],
        sections: &[Option<String>],
        tints: &[Option<day_spec::Color>],
        menus: &[Vec<day_spec::MenuItem>],
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavMenuIvars {
            node,
            items: RefCell::new(items.iter().map(|s| NSString::from_str(s)).collect()),
            icons: RefCell::new(resolve_nav_icons(icons)),
            tints: RefCell::new(tints.to_vec()),
            menus: RefCell::new(menus.to_vec()),
            badges: RefCell::new(ns_badges(badges)),
            rows: RefCell::new(Vec::new()),
            row_objects: RefCell::new(Vec::new()),
            suppress: std::cell::Cell::new(false),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        this.rebuild_rows(items, sections);
        this
    }

    /// Data-driven rows changed (`NavMenuPatch::Items`): swap the stored decorations in place.
    fn set_items(
        &self,
        items: &[String],
        icons: &[Option<String>],
        badges: &[Option<String>],
        sections: &[Option<String>],
        tints: &[Option<day_spec::Color>],
        menus: &[Vec<day_spec::MenuItem>],
    ) {
        *self.ivars().items.borrow_mut() = items.iter().map(|s| NSString::from_str(s)).collect();
        *self.ivars().icons.borrow_mut() = resolve_nav_icons(icons);
        *self.ivars().tints.borrow_mut() = tints.to_vec();
        *self.ivars().menus.borrow_mut() = menus.to_vec();
        *self.ivars().badges.borrow_mut() = ns_badges(badges);
        self.rebuild_rows(items, sections);
    }

    /// Flatten items + section headers into the outline's display rows. Each row gets its own
    /// retained NSString so the outline can tell two rows apart by identity even when their
    /// labels match (two feeds legitimately share a title).
    fn rebuild_rows(&self, items: &[String], sections: &[Option<String>]) {
        let mut rows = Vec::with_capacity(items.len());
        let mut objects = Vec::with_capacity(items.len());
        for (i, label) in items.iter().enumerate() {
            if let Some(Some(header)) = sections.get(i) {
                rows.push(None);
                objects.push(NSString::from_str(header));
            }
            rows.push(Some(i));
            objects.push(NSString::from_str(label));
        }
        *self.ivars().rows.borrow_mut() = rows;
        *self.ivars().row_objects.borrow_mut() = objects;
    }

    /// Outline row → day item index.
    fn item_of_row(&self, row: isize) -> Option<usize> {
        if row < 0 {
            return None;
        }
        self.ivars()
            .rows
            .borrow()
            .get(row as usize)
            .copied()
            .flatten()
    }

    /// Day item index → outline row.
    fn row_of_item(&self, item: usize) -> Option<usize> {
        self.ivars()
            .rows
            .borrow()
            .iter()
            .position(|r| *r == Some(item))
    }
}

/// Badge strings, retained once per rebuild rather than per cell.
fn ns_badges(badges: &[Option<String>]) -> Vec<Option<Retained<NSString>>> {
    badges
        .iter()
        .map(|b| b.as_deref().map(NSString::from_str))
        .collect()
}

// ---------------------------------------------------------------------------
/// Force the table's visible rows to exist on the NEXT main-loop turn — outside any `with_tree`
/// borrow, so `bind_row` may build them. `layoutSubtreeIfNeeded` is not enough: an occluded
/// window's table skips row tiling entirely, so `viewAtColumn:row:makeIfNecessary:` is the only
/// reliable way to realize rows without a draw pass (§10; see the Reload patch for why).
fn post_realize_visible_rows(key: usize) {
    <AppKit as Platform>::post(Box::new(move || {
        LIST_STATE.with(|m| {
            if let Some((table, _)) = m.borrow().get(&key) {
                let range = unsafe { table.rowsInRect(table.visibleRect()) };
                for row in range.location..range.location + range.length {
                    let _ =
                        unsafe { table.viewAtColumn_row_makeIfNecessary(0, row as isize, true) };
                }
            }
        });
        // The builds above queued their styling effects; drain them now — outside the pump, no
        // event may arrive for a while (idle app), and a snapshot would capture bare rows.
        day_core::pump_events();
    }));
}

define_class!(
    #[unsafe(super(objc2_app_kit::NSTableRowView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayListRowView"]
    struct DayListRowView;

    impl DayListRowView {
        // NSTableRowView insets its cell views ~6pt per side even in the FullWidth table
        // style, while day lays row content out at the LIST's full frame width — the inset
        // shifted every row right and clipped the trailing edge of its content. Pin day
        // cells back to the row's full bounds after the standard layout.
        #[unsafe(method(layout))]
        fn layout(&self) {
            let _: () = unsafe { msg_send![super(self), layout] };
            let b = self.bounds();
            for sub in self.subviews().iter() {
                let is_cell = sub
                    .identifier()
                    .map(|i| i.to_string() == "day.cell")
                    .unwrap_or(false);
                if is_cell {
                    unsafe { sub.setFrame(b) };
                }
            }
        }
    }
);

// DayListData — NSTableView data-source + delegate for the recycling list (docs/list.md, §10)
// ---------------------------------------------------------------------------

struct ListIvars {
    node: NodeId,
    /// Injected by `attach_list` once day-core wires the driver.
    source: RefCell<Option<ListSource>>,
    selectable: std::cell::Cell<bool>,
    /// Multi-select mode (docs/list.md): every change reports the FULL selected set.
    multi: std::cell::Cell<bool>,
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: std::cell::Cell<bool>,
    /// The row-token order this table currently displays — the baseline `permutation_moves`
    /// diffs a Reload against (docs/list.md: a same-set reorder animates as row moves).
    tokens: RefCell<Vec<u64>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayListData"]
    #[ivars = ListIvars]
    struct DayListData;

    unsafe impl NSObjectProtocol for DayListData {}
    unsafe impl NSControlTextEditingDelegate for DayListData {}

    unsafe impl NSTableViewDataSource for DayListData {
        #[unsafe(method(numberOfRowsInTableView:))]
        fn number_of_rows(&self, _tv: &NSTableView) -> isize {
            // Reads the piece's snapshot only (no tree access) — safe even when called
            // synchronously from reloadData inside a with_tree borrow.
            self.ivars()
                .source
                .borrow()
                .as_ref()
                .map(|s| (s.len)() as isize)
                .unwrap_or(0)
        }

        // --- drag-to-reorder (docs/list.md): NSTableView's native drag pipeline. The dragged
        // row rides the pasteboard as a private-typed string; `validateDrop` consults the app's
        // guard live (the `.gap` feedback style opens the placeholder gap only where the guard
        // allows), and `acceptDrop` commits through the sync seam, then animates the native move.

        #[unsafe(method_id(tableView:pasteboardWriterForRow:))]
        fn pasteboard_writer_for_row(
            &self,
            _tv: &NSTableView,
            row: isize,
        ) -> Option<Retained<ProtocolObject<dyn objc2_app_kit::NSPasteboardWriting>>> {
            // (Closure body: define_class converts only the TAIL expression, so early `return`s
            // must not escape the method itself.)
            let make = || {
                // Only reorderable lists write drag items — no seam, no drag.
                let reorderable = self
                    .ivars()
                    .source
                    .borrow()
                    .as_ref()
                    .is_some_and(|s| s.reorder.is_some());
                if !reorderable || row < 0 {
                    return None;
                }
                let item = objc2_app_kit::NSPasteboardItem::new();
                unsafe {
                    item.setString_forType(
                        &NSString::from_str(&row.to_string()),
                        &NSString::from_str(DAY_ROW_PASTEBOARD_TYPE),
                    );
                }
                Some(ProtocolObject::from_retained(item))
            };
            make()
        }

        #[unsafe(method(tableView:validateDrop:proposedRow:proposedDropOperation:))]
        fn validate_drop(
            &self,
            tv: &NSTableView,
            info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>,
            row: isize,
            _op: objc2_app_kit::NSTableViewDropOperation,
        ) -> objc2_app_kit::NSDragOperation {
            let Some((from, len)) = self.drag_context(tv, info) else {
                return objc2_app_kit::NSDragOperation::None;
            };
            // Normalize to an ABOVE insertion point (the gap style's semantics), convert to the
            // post-removal target index, and ask the guard.
            let ins = row.clamp(0, len as isize) as usize;
            let to = insertion_to_target(from, ins, len);
            let accepted = self.can_move(from, to);
            if accepted < 0 {
                return objc2_app_kit::NSDragOperation::None;
            }
            let accepted = (accepted as usize).min(len.saturating_sub(1));
            if accepted != to {
                // The guard retargeted: move the gap to where the drop would actually land.
                unsafe {
                    tv.setDropRow_dropOperation(
                        target_to_insertion(from, accepted) as isize,
                        objc2_app_kit::NSTableViewDropOperation::Above,
                    );
                }
            }
            objc2_app_kit::NSDragOperation::Move
        }

        #[unsafe(method(tableView:acceptDrop:row:dropOperation:))]
        fn accept_drop(
            &self,
            tv: &NSTableView,
            info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>,
            row: isize,
            _op: objc2_app_kit::NSTableViewDropOperation,
        ) -> bool {
            // (Closure body: early `return`s must not escape the define_class method itself.)
            let drop = || {
                let Some((from, len)) = self.drag_context(tv, info) else {
                    return false;
                };
                let ins = row.clamp(0, len as isize) as usize;
                let to = insertion_to_target(from, ins, len);
                let accepted = self.can_move(from, to);
                if accepted < 0 {
                    return false;
                }
                let accepted = (accepted as usize).min(len.saturating_sub(1));
                if accepted == from {
                    return true; // dropped back home — nothing to commit
                }
                // Commit through the sync seam (rotates Day's snapshot, defers the app
                // callback), THEN animate the native move — the data source already answers in
                // the new order.
                let mv = self
                    .ivars()
                    .source
                    .borrow()
                    .as_ref()
                    .and_then(|s| s.reorder.as_ref().map(|r| r.move_row.clone()));
                let Some(mv) = mv else { return false };
                mv(from, accepted);
                // Keep the displayed-order cache in step (the commit's echo skips the Reload
                // that would otherwise refresh it — see `permutation_moves`).
                {
                    let mut toks = self.ivars().tokens.borrow_mut();
                    if from < toks.len() && accepted < toks.len() {
                        let t = toks.remove(from);
                        toks.insert(accepted, t);
                    }
                }
                unsafe { tv.moveRowAtIndex_toIndex(from as isize, accepted as isize) };
                true
            };
            drop()
        }
    }

    unsafe impl NSTableViewDelegate for DayListData {
        #[unsafe(method_id(tableView:viewForTableColumn:row:))]
        fn view_for_row(
            &self,
            tv: &NSTableView,
            _col: Option<&NSTableColumn>,
            row: isize,
        ) -> Option<Retained<NSView>> {
            let mtm = self.mtm();
            let ident = NSString::from_str("day.cell");
            // Recycle a cell view if one is free; else make a fresh flipped container.
            let cell: Retained<NSView> = unsafe { tv.makeViewWithIdentifier_owner(&ident, None) }
                .unwrap_or_else(|| {
                    let v: Retained<NSView> = Retained::into_super(DayFlipped::new(mtm));
                    unsafe { v.setIdentifier(Some(&ident)) };
                    v
                });
            // Day builds row content the first time it sees this cell, and rebinds (slot-write)
            // when the cell is recycled. NSTableView calls this outside reloadData's stack, so the
            // re-entry into with_tree is safe.
            if let Some(source) = self.ivars().source.borrow().as_ref() {
                let raw = Retained::as_ptr(&cell) as RawHandle;
                (source.bind_row)(row as usize, raw);
            }
            Some(cell)
        }

        #[unsafe(method_id(tableView:rowViewForRow:))]
        fn row_view_for_row(
            &self,
            _tv: &NSTableView,
            _row: isize,
        ) -> Option<Retained<objc2_app_kit::NSTableRowView>> {
            // Our row view pins day cells to the full row bounds (see DayListRowView).
            let rv: Retained<DayListRowView> =
                unsafe { msg_send![DayListRowView::alloc(self.mtm()), init] };
            Some(Retained::into_super(rv))
        }

        #[unsafe(method(tableViewSelectionDidChange:))]
        fn selection_did_change(&self, notification: &NSNotification) {
            if self.ivars().suppress.get() || !self.ivars().selectable.get() {
                return;
            }
            let Some(obj) = (unsafe { notification.object() }) else {
                return;
            };
            let Ok(tv) = obj.downcast::<NSTableView>() else {
                return;
            };
            if self.ivars().multi.get() {
                // Multi-select: report the FULL set (ascending; empty = cleared).
                let idx = unsafe { tv.selectedRowIndexes() };
                let mut rows = Vec::with_capacity(idx.count());
                let mut i = idx.firstIndex();
                while i != objc2_foundation::NSNotFound as usize {
                    rows.push(i as i64);
                    i = unsafe { idx.indexGreaterThanIndex(i) };
                }
                emit(self.ivars().node, Event::SelectionSet(rows));
            } else {
                let row = unsafe { tv.selectedRow() };
                if row >= 0 {
                    emit(self.ivars().node, Event::SelectionChanged(row as i64));
                }
            }
        }
    }
);

impl DayListData {
    fn new(mtm: MainThreadMarker, node: NodeId, selectable: bool, multi: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ListIvars {
            node,
            source: RefCell::new(None),
            selectable: std::cell::Cell::new(selectable),
            multi: std::cell::Cell::new(multi),
            suppress: std::cell::Cell::new(false),
            tokens: RefCell::new(Vec::new()),
        });
        unsafe { msg_send![super(this), init] }
    }

    /// If this Reload is the SAME row set in a new order, the incremental `moveRowAtIndex`
    /// steps (each `(from, to)` interpreted against the already-moved rows, NSTableView's
    /// semantics) that transform the displayed order into the new one — and `None` for
    /// anything else (insert/remove/content change/no change), which reloads flat. Always
    /// refreshes the displayed-order cache.
    ///
    /// Every row that changes position must have a REALIZED row view (`table`): moving a
    /// viewless row (its old position sat outside the viewport, so it was never bound)
    /// animates nothing and leaves the row unbound at its new position — its content, and any
    /// element ids inside it, would simply not exist. Such a reorder returns `None` and
    /// reloads flat, which realizes and binds the row at its new position.
    fn permutation_moves(&self, table: &NSTableView) -> Option<Vec<(usize, usize)>> {
        let source = self.ivars().source.borrow();
        let source = source.as_ref()?;
        let n = (source.len)();
        let new: Vec<u64> = (0..n).map(|i| (source.token_at)(i)).collect();
        let old = std::mem::replace(&mut *self.ivars().tokens.borrow_mut(), new.clone());
        if old.len() != n || n == 0 || old == new {
            return None;
        }
        let (mut so, mut sn) = (old.clone(), new.clone());
        so.sort_unstable();
        sn.sort_unstable();
        if so != sn {
            return None;
        }
        for i in 0..n {
            if old[i] != new[i]
                && unsafe { table.rowViewAtRow_makeIfNecessary(i as isize, false) }.is_none()
            {
                return None;
            }
        }
        let mut work = old;
        let mut moves = Vec::new();
        for i in 0..n {
            if work[i] == new[i] {
                continue;
            }
            let j = (i + 1..n).find(|&j| work[j] == new[i])?;
            let t = work.remove(j);
            work.insert(i, t);
            moves.push((j, i));
        }
        Some(moves)
    }

    /// The dragged row + current row count for an in-flight LOCAL reorder drag — `None` when the
    /// drag came from anywhere but this table (day never accepts foreign drops into a list) or
    /// the pasteboard doesn't carry the day row type.
    fn drag_context(
        &self,
        tv: &NSTableView,
        info: &ProtocolObject<dyn objc2_app_kit::NSDraggingInfo>,
    ) -> Option<(usize, usize)> {
        let src = unsafe { info.draggingSource() }?;
        if Retained::as_ptr(&src) as *const std::ffi::c_void
            != tv as *const NSTableView as *const std::ffi::c_void
        {
            return None;
        }
        let pb = unsafe { info.draggingPasteboard() };
        let from = unsafe { pb.stringForType(&NSString::from_str(DAY_ROW_PASTEBOARD_TYPE)) }?
            .to_string()
            .parse::<usize>()
            .ok()?;
        let len = self.ivars().source.borrow().as_ref().map(|s| (s.len)())?;
        (from < len).then_some((from, len))
    }

    /// The guard's verdict for `from -> to` (accepted index, or -1), through the sync seam.
    fn can_move(&self, from: usize, to: usize) -> i64 {
        self.ivars()
            .source
            .borrow()
            .as_ref()
            .and_then(|s| s.reorder.as_ref().map(|r| (r.can_move)(from, to)))
            .unwrap_or(-1)
    }
}

/// The private pasteboard type carrying a dragged day list row's index (local reorder only).
const DAY_ROW_PASTEBOARD_TYPE: &str = "dev.daybrite.day.row";

/// An NSTableView drop lands at an INSERTION point (`row` with `.Above` semantics, 0..=len);
/// day's seam speaks post-removal target indices. Dropping `from` at insertion `ins`:
fn insertion_to_target(from: usize, ins: usize, len: usize) -> usize {
    let to = if ins > from { ins - 1 } else { ins };
    to.min(len.saturating_sub(1))
}

/// The inverse, for retargeting the drop gap: where the gap sits for a post-removal target.
fn target_to_insertion(from: usize, to: usize) -> usize {
    if to > from { to + 1 } else { to }
}

/// A realized LIST's scroll view ptr → (table, data source) for attach_list / update / measure.
type ListEntry = (Retained<NSTableView>, Retained<DayListData>);

thread_local! {
    static LIST_STATE: RefCell<HashMap<usize, ListEntry>> = RefCell::new(HashMap::new());
}

/// A realized NAV_MENU's native outline view paired with its data-source object.
type NavMenuEntry = (
    Retained<objc2_app_kit::NSOutlineView>,
    Retained<DayNavMenuData>,
);

thread_local! {
    /// NAV_MENU scroll-view ptr → (outline, data source) for patches and measure.
    static NAV_MENUS: RefCell<HashMap<usize, NavMenuEntry>> = RefCell::new(HashMap::new());
}

fn ns_rect(r: day_spec::Rect) -> NSRect {
    NSRect::new(
        NSPoint::new(r.origin.x, r.origin.y),
        NSSize::new(r.size.width, r.size.height),
    )
}

fn draw_op(op: &DrawOp) {
    unsafe {
        match op {
            DrawOp::Fill(shape, paint) => match paint {
                day_spec::Paint::Solid(color) => {
                    nscolor(*color).setFill();
                    if let Some(p) = bezier(shape) {
                        p.fill();
                    }
                }
                day_spec::Paint::Linear(g) => {
                    // Native linear gradient: clip to the shape's path, then NSGradient along
                    // the line resolved from the gradient's unit points in the shape's bounds.
                    if let (Some(p), Some(grad)) = (bezier(shape), nsgradient(&g.stops)) {
                        let b = shape.bounds();
                        let (s, e) = (g.start.resolve(b), g.end.resolve(b));
                        NSGraphicsContext::saveGraphicsState_class();
                        p.addClip();
                        grad.drawFromPoint_toPoint_options(
                            NSPoint::new(s.x, s.y),
                            NSPoint::new(e.x, e.y),
                            objc2_app_kit::NSGradientDrawingOptions::DrawsBeforeStartingLocation
                                | objc2_app_kit::NSGradientDrawingOptions::DrawsAfterEndingLocation,
                        );
                        NSGraphicsContext::restoreGraphicsState_class();
                    }
                }
                day_spec::Paint::Radial(g) => {
                    // Native radial gradient: clip to the path, map unit space onto the bounds
                    // (elliptical in non-square bounds), draw circular in unit coordinates.
                    if let (Some(p), Some(grad)) = (bezier(shape), nsgradient(&g.stops)) {
                        let b = shape.bounds();
                        NSGraphicsContext::saveGraphicsState_class();
                        p.addClip();
                        concat_unit_to_bounds(b);
                        let c = NSPoint::new(g.center.x, g.center.y);
                        grad.drawFromCenter_radius_toCenter_radius_options(
                            c,
                            0.0,
                            c,
                            g.radius,
                            objc2_app_kit::NSGradientDrawingOptions::DrawsBeforeStartingLocation
                                | objc2_app_kit::NSGradientDrawingOptions::DrawsAfterEndingLocation,
                        );
                        NSGraphicsContext::restoreGraphicsState_class();
                    }
                }
            },
            DrawOp::Stroke(shape, color, width) => {
                nscolor(*color).setStroke();
                if let Some(p) = bezier(shape) {
                    p.setLineWidth(*width);
                    p.setLineCapStyle(objc2_app_kit::NSLineCapStyle::Round);
                    p.stroke();
                }
            }
            DrawOp::Text {
                text,
                at,
                size,
                color,
                anchor,
            } => {
                let font = NSFont::systemFontOfSize(*size);
                let col = nscolor(*color);
                let keys: [&NSString; 2] = [
                    objc2_app_kit::NSFontAttributeName,
                    objc2_app_kit::NSForegroundColorAttributeName,
                ];
                let objs: [&objc2::runtime::AnyObject; 2] = [
                    font.as_ref() as &objc2::runtime::AnyObject,
                    col.as_ref() as &objc2::runtime::AnyObject,
                ];
                let attrs = objc2_foundation::NSDictionary::from_slices::<NSString>(&keys, &objs);
                let ns = NSString::from_str(text);
                let mut origin = NSPoint::new(at.x, at.y);
                if *anchor == day_spec::TextAnchor::Centered {
                    let sz: NSSize = msg_send![&ns, sizeWithAttributes: &*attrs];
                    origin.x -= sz.width / 2.0;
                    origin.y -= sz.height / 2.0;
                }
                let _: () = msg_send![&ns, drawAtPoint: origin, withAttributes: &*attrs];
            }
            DrawOp::Save => NSGraphicsContext::saveGraphicsState_class(),
            DrawOp::Restore => NSGraphicsContext::restoreGraphicsState_class(),
            DrawOp::Concat(m) => {
                let t = NSAffineTransform::new();
                t.setTransformStruct(NSAffineTransformStruct {
                    m11: m.a,
                    m12: m.b,
                    m21: m.c,
                    m22: m.d,
                    tX: m.tx,
                    tY: m.ty,
                });
                t.concat();
            }
        }
    }
}

/// An `NSGradient` from a display-list gradient's stops (sRGB, like every canvas color).
fn nsgradient(
    stops: &[(f64, day_spec::Color)],
) -> Option<objc2::rc::Retained<objc2_app_kit::NSGradient>> {
    if stops.is_empty() {
        return None;
    }
    let colors = objc2_foundation::NSArray::from_retained_slice(
        &stops.iter().map(|(_, c)| nscolor(*c)).collect::<Vec<_>>(),
    );
    let locations: Vec<f64> = stops.iter().map(|(o, _)| *o).collect();
    unsafe {
        objc2_app_kit::NSGradient::initWithColors_atLocations_colorSpace(
            objc2_app_kit::NSGradient::alloc(),
            &colors,
            locations.as_ptr(),
            &objc2_app_kit::NSColorSpace::sRGBColorSpace(),
        )
    }
}

/// Concat a bounds-mapping transform (unit gradient space → `b`) onto the current context, so a
/// circular gradient drawn in unit coordinates renders elliptically stretched to the bounds.
fn concat_unit_to_bounds(b: day_spec::Rect) {
    let t = NSAffineTransform::new();
    t.setTransformStruct(NSAffineTransformStruct {
        m11: b.size.width,
        m12: 0.0,
        m21: 0.0,
        m22: b.size.height,
        tX: b.origin.x,
        tY: b.origin.y,
    });
    unsafe { t.concat() };
}

fn bezier(shape: &day_spec::Shape) -> Option<objc2::rc::Retained<objc2_app_kit::NSBezierPath>> {
    use day_spec::Shape;
    use objc2_app_kit::NSBezierPath;
    unsafe {
        Some(match shape {
            Shape::Rect(r) => NSBezierPath::bezierPathWithRect(ns_rect(*r)),
            Shape::RoundedRect(r, rad) => {
                NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(ns_rect(*r), *rad, *rad)
            }
            Shape::Ellipse(r) => NSBezierPath::bezierPathWithOvalInRect(ns_rect(*r)),
            Shape::Arc {
                rect,
                start_deg,
                sweep_deg,
            } => {
                let p = NSBezierPath::new();
                let center = NSPoint::new(
                    rect.origin.x + rect.size.width / 2.0,
                    rect.origin.y + rect.size.height / 2.0,
                );
                let radius = rect.size.width.min(rect.size.height) / 2.0;
                // Flipped view: increasing angle is visually clockwise, matching the spec.
                p.appendBezierPathWithArcWithCenter_radius_startAngle_endAngle(
                    center,
                    radius,
                    *start_deg,
                    *start_deg + *sweep_deg,
                );
                p
            }
            Shape::Line(a, b) => {
                let p = NSBezierPath::new();
                p.moveToPoint(NSPoint::new(a.x, a.y));
                p.lineToPoint(NSPoint::new(b.x, b.y));
                p
            }
            Shape::Polygon(pts) => {
                if pts.len() < 2 {
                    return None;
                }
                let p = NSBezierPath::new();
                p.moveToPoint(NSPoint::new(pts[0].x, pts[0].y));
                for pt in &pts[1..] {
                    p.lineToPoint(NSPoint::new(pt.x, pt.y));
                }
                p.closePath();
                p
            }
        })
    }
}

// ---------------------------------------------------------------------------
// DayWinDelegate — resize + close + key tracking, per window
// ---------------------------------------------------------------------------

/// `node`: the day window-root id this delegate's window reports to. `None` = the PRIMARY
/// window — resize goes to `WINDOW_NODE` and close terminates the app; a secondary window
/// (docs/windows.md) reports to its own root and close tears down just that window.
struct WinIvars {
    node: Option<NodeId>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayWinDelegate"]
    #[ivars = WinIvars]
    struct DayWinDelegate;

    unsafe impl NSObjectProtocol for DayWinDelegate {}

    unsafe impl NSWindowDelegate for DayWinDelegate {
        #[unsafe(method(windowDidResize:))]
        fn window_did_resize(&self, notification: &NSNotification) {
            if let Some(obj) = unsafe { notification.object() }
                && let Ok(win) = obj.downcast::<NSWindow>()
                && let Some(content) = win.contentView()
            {
                let b = content.bounds();
                let target = self.ivars().node.unwrap_or(WINDOW_NODE);
                emit(
                    target,
                    Event::WindowResized(Size::new(b.size.width, b.size.height)),
                );
            }
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            match self.ivars().node {
                // Secondary window: confirm the close to day-core, which tears the
                // subtree down on a DEFERRED hop (never inside this AppKit close frame).
                Some(node) => emit(node, Event::WindowClosed),
                // Primary window: closing it quits, taking secondaries with it
                // (docs/windows.md close policy).
                None => {
                    let app = NSApplication::sharedApplication(self.mtm());
                    unsafe { app.terminate(None) };
                }
            }
        }

        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            if let Some(node) = self.ivars().node {
                emit(node, Event::WindowFocused(true));
            }
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            if let Some(node) = self.ivars().node {
                emit(node, Event::WindowFocused(false));
            }
        }
    }

    // The tab bar's "+" button walks the responder chain (window → delegate) for
    // `newWindowForTab:` — present only when the app registered a new-window builder
    // (otherwise automatic tabbing is off and no "+" exists).
    impl DayWinDelegate {
        #[unsafe(method(newWindowForTab:))]
        fn new_window_for_tab(&self, _sender: Option<&NSObject>) {
            let _ = day_core::windows::open_new_window();
        }
    }
);

impl DayWinDelegate {
    fn new(mtm: MainThreadMarker, node: Option<NodeId>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WinIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// The backend
// ---------------------------------------------------------------------------

/// Renderers registered by external Day Piece crates (§8.2 layer 3 — linkme convenience).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<AppKit>];

/// A live secondary window (docs/windows.md). The delegate is RETAINED here —
/// `setDelegate:` holds it weakly, and unlike the primary's (kept alive because `run`
/// never returns) a secondary's delegate would die at the end of `open_window` otherwise.
struct SecondaryWin {
    window: Retained<NSWindow>,
    #[allow(dead_code)] // held for lifetime only; AppKit talks to it via the weak delegate ref
    delegate: Retained<DayWinDelegate>,
    content: Handle,
}

pub struct AppKit {
    mtm: MainThreadMarker,
    registry: Registry<AppKit>,
    window: Option<Retained<NSWindow>>,
    content: Option<Handle>,
    secondary: Vec<SecondaryWin>,
    app_name: String,
}

impl AppKit {
    pub fn new() -> Self {
        let mtm = MainThreadMarker::new().expect("day-appkit must start on the main thread");
        let mut registry = Registry::default();
        for f in RENDERERS {
            registry.register(f());
        }
        AppKit {
            mtm,
            registry,
            window: None,
            content: None,
            secondary: Vec::new(),
            app_name: "Day".into(),
        }
    }

    /// Public helper for external renderers.
    pub fn mtm(&self) -> MainThreadMarker {
        self.mtm
    }

    /// Build a Day window: titled + closable (Preferences kind drops resize/minimize and
    /// disallows tabbing per macOS convention; Normal windows share the `day.normal`
    /// tabbing group), a flipped content view, and a per-window delegate (`node`: `None` =
    /// primary). The window is NOT released-when-closed — Rust's `Retained` owns it, and
    /// the AppKit default would double-release under us on close.
    fn make_window(
        &self,
        title: &str,
        size: Size,
        min_size: Option<Size>,
        prefs_style: bool,
        node: Option<NodeId>,
    ) -> (Retained<NSWindow>, Retained<DayWinDelegate>, Handle) {
        let mtm = self.mtm;
        let content_rect =
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(size.width, size.height));
        let mut style = NSWindowStyleMask::Titled | NSWindowStyleMask::Closable;
        if !prefs_style {
            style |= NSWindowStyleMask::Miniaturizable | NSWindowStyleMask::Resizable;
        }
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content_rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(title));
        if let Some(min) = min_size {
            unsafe { window.setContentMinSize(NSSize::new(min.width, min.height)) };
        }
        if prefs_style {
            window.setTabbingMode(objc2_app_kit::NSWindowTabbingMode::Disallowed);
        } else {
            // Same-kind Day windows group as native tabs (System Settings "prefer tabs",
            // View ▸ Show Tab Bar, Merge All Windows) — meaningful once a second Normal
            // window can exist; inert while automatic tabbing is globally off (see `run`).
            unsafe { window.setTabbingIdentifier(&NSString::from_str("day.normal")) };
        }
        let delegate = DayWinDelegate::new(mtm, node);
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
        let content = view_of(DayFlipped::new(mtm));
        window.setContentView(Some(&content));
        (window, delegate, content)
    }
}

impl Default for AppKit {
    fn default() -> Self {
        Self::new()
    }
}

fn view_of<T: AsRef<NSView>>(x: Retained<T>) -> Handle {
    Retained::from(x.as_ref())
}

fn nscolor(c: day_spec::Color) -> Retained<NSColor> {
    unsafe { NSColor::colorWithSRGBRed_green_blue_alpha(c.r, c.g, c.b, c.a) }
}

/// The macOS native semantic text style for a logical [`Font`] (`None` for a custom size).
/// `NSFont.preferredFont(forTextStyle:)` gives the OS's own typography, tracking the system settings.
fn ns_text_style(f: Font) -> Option<&'static objc2_app_kit::NSFontTextStyle> {
    use objc2_app_kit::*;
    unsafe {
        Some(match f {
            Font::LargeTitle => NSFontTextStyleLargeTitle,
            Font::Title => NSFontTextStyleTitle1,
            Font::Title2 => NSFontTextStyleTitle2,
            Font::Title3 => NSFontTextStyleTitle3,
            Font::Headline => NSFontTextStyleHeadline,
            Font::Subheadline => NSFontTextStyleSubheadline,
            Font::Body => NSFontTextStyleBody,
            Font::Callout => NSFontTextStyleCallout,
            Font::Footnote => NSFontTextStyleFootnote,
            Font::Caption => NSFontTextStyleCaption1,
            Font::Caption2 => NSFontTextStyleCaption2,
            Font::System(_) | Font::Custom(..) => return None,
        })
    }
}

fn ns_weight(w: day_spec::FontWeight) -> objc2_app_kit::NSFontWeight {
    use day_spec::FontWeight as W;
    use objc2_app_kit::*;
    unsafe {
        match w {
            W::UltraLight => NSFontWeightUltraLight,
            W::Thin => NSFontWeightThin,
            W::Light => NSFontWeightLight,
            W::Regular => NSFontWeightRegular,
            W::Medium => NSFontWeightMedium,
            W::Semibold => NSFontWeightSemibold,
            W::Bold => NSFontWeightBold,
            W::Heavy => NSFontWeightHeavy,
            W::Black => NSFontWeightBlack,
        }
    }
}

/// Resolve a [`FontSpec`] to a native `NSFont`: a semantic style via `preferredFont(forTextStyle:)`
/// (or a custom system size), then an optional weight override (at the same size) and italic trait.
fn nsfont(spec: day_spec::FontSpec) -> Retained<NSFont> {
    use objc2_app_kit::*;
    let base: Retained<NSFont> = match spec.style {
        Font::System(pt) => {
            let w = spec
                .weight
                .map(ns_weight)
                .unwrap_or(unsafe { NSFontWeightRegular });
            unsafe { NSFont::systemFontOfSize_weight(pt, w) }
        }
        // A bundled family (§18.4), registered with CoreText in run(). fontWithName resolves
        // family, full, and PostScript names. A weight override maps to the bold trait (the
        // family decides what it can supply); unknown families fall back to the system font.
        Font::Custom(name, pt) => {
            match unsafe { NSFont::fontWithName_size(&NSString::from_str(name), pt) } {
                Some(f) => {
                    if spec
                        .weight
                        .is_some_and(|w| w >= day_spec::FontWeight::Semibold)
                    {
                        let mtm = objc2::MainThreadMarker::new()
                            .expect("labels realize on the main thread");
                        unsafe {
                            NSFontManager::sharedFontManager(mtm)
                                .convertFont_toHaveTrait(&f, NSFontTraitMask::BoldFontMask)
                        }
                    } else {
                        f
                    }
                }
                None => {
                    eprintln!(
                        "day: unknown font family {name:?} — falling back to the system font \
                         (is the file in the project's fonts/ directory?)"
                    );
                    let w = spec
                        .weight
                        .map(ns_weight)
                        .unwrap_or(unsafe { NSFontWeightRegular });
                    unsafe { NSFont::systemFontOfSize_weight(pt, w) }
                }
            }
        }
        style => {
            let ts = ns_text_style(style).expect("semantic style");
            let opts = objc2_foundation::NSDictionary::new();
            let f = unsafe { NSFont::preferredFontForTextStyle_options(ts, &opts) };
            match spec.weight {
                // A weight override keeps the style's (system-resolved) size but re-picks the weight.
                Some(w) => unsafe { NSFont::systemFontOfSize_weight(f.pointSize(), ns_weight(w)) },
                None => f,
            }
        }
    };
    // Tabular figures. Cocoa exposes them as a whole font, not a trait, so this re-picks the
    // system font at the resolved size/weight rather than converting — which is why it only
    // applies to the system styles: a bundled `Font::Custom` family keeps its own figures (asking
    // for the system's monospaced-digit face there would silently swap the typeface).
    let base = if spec.tabular && !matches!(spec.style, Font::Custom(..)) {
        let w = spec
            .weight
            .map(ns_weight)
            .unwrap_or(unsafe { NSFontWeightRegular });
        unsafe { NSFont::monospacedDigitSystemFontOfSize_weight(base.pointSize(), w) }
    } else {
        base
    };
    if spec.italic {
        let mtm = objc2::MainThreadMarker::new().expect("labels realize on the main thread");
        unsafe {
            NSFontManager::sharedFontManager(mtm)
                .convertFont_toHaveTrait(&base, NSFontTraitMask::ItalicFontMask)
        }
    } else {
        base
    }
}

/// Register every bundled font file (the project's `fonts/`, staged per §18.4) with CoreText for
/// this process, so `Font::Custom` family names resolve through `NSFont::fontWithName`. Called
/// once from `run()`; a failure (already registered on a hot relaunch, or a broken file) only
/// means that family won't resolve, so it logs and moves on.
fn register_bundled_fonts() {
    // CFURLRef is toll-free bridged with NSURL, so the NSURL pointer passes straight through.
    #[link(name = "CoreText", kind = "framework")]
    unsafe extern "C" {
        fn CTFontManagerRegisterFontsForURL(
            font_url: *const std::ffi::c_void,
            scope: u32, // kCTFontManagerScopeProcess = 1
            error: *mut *const std::ffi::c_void,
        ) -> bool;
    }
    for path in day_spec::fonts::bundled_fonts() {
        let url = unsafe {
            objc2_foundation::NSURL::fileURLWithPath(&NSString::from_str(&path.to_string_lossy()))
        };
        let ok = unsafe {
            CTFontManagerRegisterFontsForURL(
                Retained::as_ptr(&url) as *const std::ffi::c_void,
                1,
                std::ptr::null_mut(),
            )
        };
        if !ok {
            eprintln!("day: could not register bundled font {}", path.display());
        }
    }
}

fn configure_label_cell(tf: &NSTextField) {
    if let Some(cell) = unsafe { tf.cell() } {
        unsafe {
            cell.setWraps(true);
            cell.setUsesSingleLineMode(false);
            cell.setLineBreakMode(NSLineBreakMode::ByWordWrapping);
        }
    }
}

/// If `parent` is a scroll view, children go into its (flipped) document view.
fn content_of(parent: &Handle) -> Retained<NSView> {
    if let Some(sv) = parent.downcast_ref::<NSScrollView>()
        && let Some(doc) = unsafe { sv.documentView() }
    {
        return doc;
    }
    parent.clone()
}

/// Run `body` (which sets animatable properties on a layer-backed view) inside an
/// `NSAnimationContext` group so AppKit interpolates the change on the render server with the
/// curve's timing function, or immediately when `anim` is `None` (§8.4). `allowsImplicitAnimation`
/// makes direct property changes on layer-backed views animate. A spring maps to an overshooting
/// cubic-bezier timing function (a visible bounce without an explicit `CASpringAnimation`).
fn with_appkit_anim(anim: Option<&AnimSpec>, body: impl FnOnce()) {
    let Some(a) = anim else {
        body();
        return;
    };
    let (dur, timing) = appkit_timing(a);
    unsafe {
        NSAnimationContext::beginGrouping();
        let ctx = NSAnimationContext::currentContext();
        ctx.setDuration(dur);
        ctx.setTimingFunction(Some(&timing));
        ctx.setAllowsImplicitAnimation(true);
        body();
        NSAnimationContext::endGrouping();
    }
}

/// The `(duration_secs, timing function)` for a curve. Springs render as an overshooting bezier
/// (control point y > 1) whose overshoot grows as damping falls, over a response-derived duration.
fn appkit_timing(a: &AnimSpec) -> (f64, Retained<CAMediaTimingFunction>) {
    unsafe {
        let named = |n| {
            (
                a.duration_secs().max(0.01),
                CAMediaTimingFunction::functionWithName(n),
            )
        };
        match a.curve {
            Curve::Linear => named(kCAMediaTimingFunctionLinear),
            Curve::EaseIn => named(kCAMediaTimingFunctionEaseIn),
            Curve::EaseOut => named(kCAMediaTimingFunctionEaseOut),
            Curve::EaseInOut => named(kCAMediaTimingFunctionEaseInEaseOut),
            Curve::Spring { damping, .. } => {
                // Overshoot bezier over exactly the specified duration (authoritative timing).
                let overshoot = 1.0 + (1.0 - damping.clamp(0.0, 1.0)) as f32 * 0.55;
                (
                    a.duration_secs().max(0.05),
                    CAMediaTimingFunction::functionWithControlPoints(0.34, overshoot, 0.5, 1.0),
                )
            }
        }
    }
}

/// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling back to
/// a visible placeholder. A missing renderer usually means the piece's `appkit` feature wasn't enabled
/// (Tier A.2 derives it automatically under `day build`; a bare `cargo` build may miss it). Deduped
/// per kind so a placeholder rendered every frame doesn't spam the log.
fn warn_missing_renderer(kind: PieceKind) {
    day_spec::placeholder::report(kind, "appkit");
}

impl Toolkit for AppKit {
    type Handle = Handle;

    fn dark_mode(&mut self) -> bool {
        // The app's EFFECTIVE appearance — one source for the system setting, the DAY_THEME
        // launch force (applied as an NSApp override at startup), and a set_appearance call.
        let app = NSApplication::sharedApplication(self.mtm());
        app.effectiveAppearance()
            .name()
            .to_string()
            .contains("Dark")
    }

    /// macOS is the one platform whose badge takes arbitrary text: `NSDockTile.badgeLabel` is a
    /// `String`, so `Text` renders literally and a count is just its decimal form. A nil label
    /// clears it (docs/badge.md).
    fn set_app_badge(&mut self, badge: &day_spec::AppBadge) {
        use day_spec::AppBadge;
        let label = match badge {
            AppBadge::None => None,
            // Zero clears, matching the platform convention the doc states.
            AppBadge::Count(0) => None,
            AppBadge::Count(n) => Some(n.to_string()),
            AppBadge::Text(t) if t.is_empty() => None,
            AppBadge::Text(t) => Some(t.clone()),
            // No dedicated dot on the Dock; the smallest honest mark is a single bullet.
            AppBadge::Dot => Some("\u{2022}".to_string()),
        };
        let tile = NSApplication::sharedApplication(self.mtm()).dockTile();
        let ns = label.map(|l| objc2_foundation::NSString::from_str(&l));
        unsafe { tile.setBadgeLabel(ns.as_deref()) };
        tile.display();
    }

    fn set_appearance(&mut self, dark: Option<bool>) {
        let app = NSApplication::sharedApplication(self.mtm());
        let appearance = dark.and_then(|d| {
            objc2_app_kit::NSAppearance::appearanceNamed(unsafe {
                if d {
                    objc2_app_kit::NSAppearanceNameDarkAqua
                } else {
                    objc2_app_kit::NSAppearanceNameAqua
                }
            })
        });
        // `None` clears the override — back to the system appearance.
        unsafe { app.setAppearance(appearance.as_deref()) };
    }

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot
            | Cap::NativeSymbols
            | Cap::NavSplit
            | Cap::Dialogs
            | Cap::FileDialogs
            | Cap::Animation
            // NSTextView natively honors all three text-area attributes.
            | Cap::TextEditable
            | Cap::TextSelectable
            | Cap::TextSpellCheck
            // NSTableView's own drag pipeline, with the `.gap` placeholder (docs/list.md).
            | Cap::ListReorder
            // Real NSWindows with native tabbing + the Windows menu (docs/windows.md).
            | Cap::MultiWindow
            // A real NSToolbar in the title bar (docs/toolbars.md).
            | Cap::Toolbar
            // NSDockTile.badgeLabel is an arbitrary String, so all three payloads render — the
            // only backend where Text is real (docs/badge.md).
            | Cap::AppBadgeCount
            | Cap::AppBadgeText
            | Cap::AppBadgeDot
            | Cap::Appearance => Support::Native,
            // A topmost autoresizing child of the content view — not a system modal
            // (docs/cover.md's ArkUI tier).
            Cap::Cover => Support::Emulated,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Handle {
        let mtm = self.mtm;
        match Builtin::from_key(kind) {
            Some(Builtin::Container) => {
                let v = DayFlipped::new(mtm);
                if let Some(p) = props.downcast_ref::<ContainerProps>() {
                    if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                        v.set_section_card(p.corner_radius);
                    } else if p.background.is_some() || p.corner_radius > 0.0 || p.clips {
                        v.set_surface(p.background, p.corner_radius, p.clips);
                    }
                }
                view_of(v)
            }
            Some(Builtin::Scroll) => {
                let horizontal = props
                    .downcast_ref::<day_spec::props::ScrollProps>()
                    .map(|p| p.horizontal)
                    .unwrap_or(false);
                let sv = unsafe { NSScrollView::new(mtm) };
                unsafe {
                    sv.setHasVerticalScroller(!horizontal);
                    sv.setHasHorizontalScroller(horizontal);
                    sv.setDrawsBackground(false);
                    // Overlay scrollers ALWAYS: day lays content out at the scroll's full frame
                    // width, and a legacy scroller (the "always show scroll bars" system
                    // setting, or a mouse attached) would reserve ~15pt and clip trailing
                    // content. Overlay floats above content instead — and matches the GTK/Qt
                    // backends.
                    sv.setScrollerStyle(objc2_app_kit::NSScrollerStyle::Overlay);
                }
                let doc = DayFlipped::new(mtm);
                unsafe { sv.setDocumentView(Some(doc.as_ref())) };
                view_of(sv)
            }
            Some(Builtin::Label) => {
                let p = props.downcast_ref::<LabelProps>().unwrap();
                let tf = unsafe { NSTextField::labelWithString(&NSString::from_str(&p.text), mtm) };
                configure_label_cell(&tf);
                unsafe { tf.setFont(Some(&nsfont(p.font))) };
                if let Some(c) = p.color {
                    unsafe { tf.setTextColor(Some(&nscolor(c))) };
                }
                view_of(tf)
            }
            Some(Builtin::Button) => {
                let p = props.downcast_ref::<ButtonProps>().unwrap();
                let target = DayTarget::new(mtm, id);
                let btn = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str(&p.title),
                        Some(&*target),
                        Some(sel!(action:)),
                        mtm,
                    )
                };
                // Prominent = the return-key default button (accent-filled). Bordered is the
                // stock NSButton look already.
                if p.style == day_spec::props::ButtonStyleSpec::Prominent {
                    unsafe { btn.setKeyEquivalent(&NSString::from_str("\r")) };
                }
                let view = view_of(btn);
                TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                view
            }
            Some(Builtin::Toggle) => {
                let p = props.downcast_ref::<ToggleProps>().unwrap();
                let target = DayTarget::new(mtm, id);
                let sw = unsafe { NSSwitch::new(mtm) };
                unsafe {
                    sw.setState(if p.on { 1 } else { 0 });
                    sw.setEnabled(p.enabled);
                    sw.setTarget(Some(&*target));
                    sw.setAction(Some(sel!(action:)));
                }
                let view = view_of(sw);
                TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                view
            }
            Some(Builtin::Slider) => {
                let p = props.downcast_ref::<SliderProps>().unwrap();
                let target = DayTarget::new(mtm, id);
                let sl = unsafe {
                    NSSlider::sliderWithValue_minValue_maxValue_target_action(
                        p.value,
                        p.min,
                        p.max,
                        Some(&*target),
                        Some(sel!(action:)),
                        mtm,
                    )
                };
                unsafe { sl.setContinuous(true) };
                let view = view_of(sl);
                TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                view
            }
            Some(Builtin::Picker) => picker::realize_any(self, props, id),
            Some(Builtin::TextArea) => textarea::realize_any(self, props, id),
            Some(Builtin::TextField) => {
                let p = props.downcast_ref::<TextFieldProps>().unwrap();
                let target = DayTarget::new(mtm, id);
                // Retained<DayTextField> → Retained<NSTextField> (its declared superclass) so
                // the AsRef<NSView> bound on `view_of` resolves.
                let tf: Retained<NSTextField> = Retained::into_super(DayTextField::new(mtm, id));
                unsafe {
                    tf.setStringValue(&NSString::from_str(&p.text));
                    tf.setPlaceholderString(Some(&NSString::from_str(&p.placeholder)));
                    tf.setEditable(true);
                    tf.setBezeled(true);
                    tf.setDelegate(Some(ProtocolObject::from_ref(&*target)));
                }
                let view = view_of(tf);
                TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                view
            }
            Some(Builtin::Divider) => {
                let b = unsafe { NSBox::new(mtm) };
                unsafe { b.setBoxType(NSBoxType::Separator) };
                view_of(b)
            }
            Some(Builtin::Progress) => {
                let p = props.downcast_ref::<ProgressProps>().unwrap();
                let pi = unsafe { NSProgressIndicator::new(mtm) };
                unsafe {
                    match p.value {
                        Some(v) => {
                            pi.setStyle(NSProgressIndicatorStyle::Bar);
                            pi.setIndeterminate(false);
                            pi.setMinValue(0.0);
                            pi.setMaxValue(1.0);
                            pi.setDoubleValue(v);
                        }
                        None => {
                            pi.setStyle(NSProgressIndicatorStyle::Spinning);
                            pi.setIndeterminate(true);
                            pi.startAnimation(None);
                        }
                    }
                }
                view_of(pi)
            }
            Some(Builtin::Canvas) => view_of(DayCanvas::new(mtm)),
            Some(Builtin::Nav) => {
                let is_split = props
                    .downcast_ref::<NavProps>()
                    .map(|p| p.split)
                    .unwrap_or(true);
                let split = unsafe { objc2_app_kit::NSSplitView::new(mtm) };
                unsafe {
                    split.setVertical(true);
                    split.setDividerStyle(objc2_app_kit::NSSplitViewDividerStyle::Thin);
                }
                // Sidebar pane rides in an NSVisualEffectView for the standard
                // translucent source-list treatment.
                let effect = unsafe { objc2_app_kit::NSVisualEffectView::new(mtm) };
                unsafe {
                    effect.setMaterial(objc2_app_kit::NSVisualEffectMaterial::Sidebar);
                    effect.setBlendingMode(objc2_app_kit::NSVisualEffectBlendingMode::BehindWindow);
                }
                let sidebar_wrap = view_of(DayFlipped::new(mtm));
                unsafe {
                    sidebar_wrap.setFrame(effect.bounds());
                    sidebar_wrap.setAutoresizingMask(
                        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
                    );
                    effect.addSubview(&sidebar_wrap);
                }
                let detail_wrap = view_of(DayFlipped::new(mtm));
                let split_delegate = DaySplitDelegate::new(mtm);
                unsafe {
                    split.addArrangedSubview(&effect);
                    split.addArrangedSubview(&detail_wrap);
                    // (Holding priorities are a no-op when Day drives the split's frame
                    // directly — the sidebar-holds-width behaviour lives in the delegate's
                    // `splitView:shouldAdjustSizeOfSubview:`, plus `set_frame`'s divider
                    // restore after each window resize.)
                    split.setHoldingPriority_forSubviewAtIndex(260.0, 0);
                    split.setHoldingPriority_forSubviewAtIndex(250.0, 1);
                    split.setDelegate(Some(ProtocolObject::from_ref(&*split_delegate)));
                }
                // Stack presentation: a back header (hidden at root) — desktop has no system
                // back affordance, so a pushed page needs its own way out (docs/navigation.md).
                let header = if is_split {
                    None
                } else {
                    let root_title = props
                        .downcast_ref::<NavProps>()
                        .map(|p| p.title.clone())
                        .unwrap_or_default();
                    let bar = view_of(DayFlipped::new(mtm));
                    let target = DayTarget::new(mtm, id);
                    let back = unsafe {
                        let img = objc2_app_kit::NSImage::imageNamed(
                            objc2_app_kit::NSImageNameGoBackTemplate,
                        )
                        .expect("NSGoBackTemplate");
                        let tobj: &objc2::runtime::AnyObject = target.as_ref();
                        objc2_app_kit::NSButton::buttonWithImage_target_action(
                            &img,
                            Some(tobj),
                            Some(sel!(navBack:)),
                            mtm,
                        )
                    };
                    let title = unsafe {
                        NSTextField::labelWithString(&NSString::from_str(&root_title), mtm)
                    };
                    unsafe {
                        back.setFrame(NSRect::new(
                            NSPoint::new(6.0, 4.0),
                            NSSize::new(30.0, NAV_HEADER_H - 8.0),
                        ));
                        title.setFont(Some(&NSFont::boldSystemFontOfSize(13.0)));
                        title.setAlignment(objc2_app_kit::NSTextAlignment::Center);
                        title.setLineBreakMode(objc2_app_kit::NSLineBreakMode::ByTruncatingTail);
                        title.setFrame(NSRect::new(
                            NSPoint::new(44.0, 9.0),
                            NSSize::new((detail_wrap.bounds().size.width - 88.0).max(0.0), 18.0),
                        ));
                        title.setAutoresizingMask(
                            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable,
                        );
                        bar.setFrame(NSRect::new(
                            NSPoint::new(0.0, 0.0),
                            NSSize::new(detail_wrap.bounds().size.width, NAV_HEADER_H),
                        ));
                        bar.setAutoresizingMask(
                            objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable,
                        );
                        bar.addSubview(&back);
                        bar.addSubview(&title);
                        bar.setHidden(true);
                        detail_wrap.addSubview(&bar);
                    }
                    Some(NavHeader {
                        bar,
                        title,
                        titles: vec![root_title],
                        _target: target,
                    })
                };
                let view = view_of(split);
                NAV_STATE.with(|m| {
                    m.borrow_mut().insert(
                        ptr_of(&view),
                        NavState {
                            sidebar_wrap,
                            detail_wrap,
                            _split_delegate: split_delegate,
                            pages: Vec::new(),
                            positioned: false,
                            split: is_split,
                            header,
                        },
                    )
                });
                view
            }
            Some(Builtin::NavPage) => {
                let page = view_of(DayNavPage::new(mtm, id));
                NAV_PAGES.with(|set| set.borrow_mut().insert(ptr_of(&page)));
                page
            }
            // Emulated fullscreen cover (docs/cover.md, the ArkUI tier): a DayNavPage — its
            // setFrameSize: override reports FrameChanged, so a window resize re-lays the cover
            // content for free — parked hidden until CoverPatch::Present re-homes it on top.
            Some(Builtin::Cover) => {
                let page = DayNavPage::new(mtm, id);
                unsafe { page.setHidden(true) };
                view_of(page)
            }
            Some(Builtin::Tabs) => {
                let p = props.downcast_ref::<TabsProps>().unwrap();
                let tabview = unsafe { NSTabView::new(mtm) };
                let delegate = DayTabDelegate::new(mtm, id);
                unsafe { tabview.setDelegate(Some(ProtocolObject::from_ref(&*delegate))) };
                let view = view_of(tabview);
                TAB_STATE.with(|m| {
                    m.borrow_mut().insert(
                        ptr_of(&view),
                        TabState {
                            delegate,
                            initial: p.selected,
                        },
                    )
                });
                view
            }
            Some(Builtin::TabsPage) => {
                let p = props.downcast_ref::<TabsPageProps>().unwrap();
                // A tab page is a native-owned content view (like a nav page: reports its
                // allocated size via FrameChanged), tagged so set_frame skips it.
                let page = view_of(DayNavPage::new(mtm, id));
                NAV_PAGES.with(|set| set.borrow_mut().insert(ptr_of(&page)));
                TAB_TITLES.with(|m| m.borrow_mut().insert(ptr_of(&page), p.title.clone()));
                page
            }
            Some(Builtin::NavMenu) => {
                let p = props.downcast_ref::<NavMenuProps>().unwrap();
                let data = DayNavMenuData::new(
                    mtm,
                    id,
                    &p.items,
                    &p.icons,
                    &p.badges,
                    &p.sections,
                    &p.tints,
                    &p.menus,
                );
                // The Day subclass serves per-row context menus via menuForEvent:.
                let outline: Retained<objc2_app_kit::NSOutlineView> = {
                    let o: Retained<DayNavOutlineView> =
                        unsafe { msg_send![DayNavOutlineView::alloc(mtm), init] };
                    NAV_OUTLINE_MENUS.with(|m| {
                        m.borrow_mut()
                            .insert(Retained::as_ptr(&o) as usize, data.clone())
                    });
                    unsafe { msg_send![&*o, self] }
                };
                let col = unsafe {
                    objc2_app_kit::NSTableColumn::initWithIdentifier(
                        objc2_app_kit::NSTableColumn::alloc(mtm),
                        &NSString::from_str("item"),
                    )
                };
                unsafe {
                    outline.addTableColumn(&col);
                    outline.setOutlineTableColumn(Some(&col));
                    outline.setHeaderView(None);
                    // Inset style, NOT SourceList: the source-list style draws its selection
                    // and labels through the sidebar material/vibrancy pipeline, which needs a
                    // backdrop behind the window to composite. Screenshot captures render the
                    // window offscreen (`cacheDisplayInRect`, no backdrop), where that material
                    // came out as a black pill that swallowed the row label. Inset keeps the
                    // rounded-pill selection look with a plain accent fill that renders the
                    // same on screen and in captures.
                    outline.setStyle(objc2_app_kit::NSTableViewStyle::Inset);
                    outline.setSelectionHighlightStyle(
                        objc2_app_kit::NSTableViewSelectionHighlightStyle::Regular,
                    );
                    outline.setIndentationPerLevel(0.0);
                    outline.setDataSource(Some(ProtocolObject::from_ref(&*data)));
                    outline.setDelegate(Some(ProtocolObject::from_ref(&*data)));
                    outline.reloadData();
                }
                let scroll = unsafe { NSScrollView::new(mtm) };
                unsafe {
                    scroll.setDrawsBackground(false);
                    scroll.setHasVerticalScroller(true);
                    // Overlay scrollers ALWAYS — a legacy scroller would reserve ~15pt and
                    // clip the trailing edge of rows laid out at the full frame width (see
                    // the SCROLL realize).
                    scroll.setScrollerStyle(objc2_app_kit::NSScrollerStyle::Overlay);
                    // One column, sized to the pane: a sidebar never scrolls sideways.
                    scroll.setHasHorizontalScroller(false);
                    outline.setAutoresizingMask(
                        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable,
                    );
                    outline.setColumnAutoresizingStyle(
                        objc2_app_kit::NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle,
                    );
                    scroll.setDocumentView(Some(&outline));
                }
                let view = view_of(scroll);
                // Day selects by ITEM index; section headers make that differ from the
                // outline's row index, so every programmatic selection maps through `rows`.
                if let Some(row) = p.selected.and_then(|sel| data.row_of_item(sel)) {
                    data.ivars().suppress.set(true);
                    unsafe {
                        outline.selectRowIndexes_byExtendingSelection(
                            &objc2_foundation::NSIndexSet::indexSetWithIndex(row),
                            false,
                        )
                    };
                    data.ivars().suppress.set(false);
                }
                NAV_MENUS.with(|m| m.borrow_mut().insert(ptr_of(&view), (outline, data)));
                view
            }
            Some(Builtin::List) => {
                let p = props.downcast_ref::<ListProps>().unwrap();
                let table = unsafe { NSTableView::new(mtm) };
                let col = unsafe {
                    NSTableColumn::initWithIdentifier(
                        NSTableColumn::alloc(mtm),
                        &NSString::from_str("day.list.col"),
                    )
                };
                let data = DayListData::new(mtm, id, p.selectable, p.multi_select);
                unsafe {
                    if p.multi_select {
                        table.setAllowsMultipleSelection(true);
                    }
                    table.addTableColumn(&col);
                    table.setHeaderView(None);
                    // Full-width cells: the default "automatic" style (macOS 11+) insets cell
                    // views ~14pt per side, clipping day row content laid out at the host's
                    // full frame width (a trailing button loses its right edge).
                    table.setStyle(objc2_app_kit::NSTableViewStyle::FullWidth);
                    // ...and the default intercell spacing (3pt × 2pt) shaves the column the
                    // same way — the last few points of a trailing control. Day rows own all
                    // of their spacing.
                    table.setIntercellSpacing(NSSize::new(0.0, 0.0));
                    table.setColumnAutoresizingStyle(
                        objc2_app_kit::NSTableViewColumnAutoresizingStyle::UniformColumnAutoresizingStyle,
                    );
                    match p.row_height {
                        RowHeight::Uniform(h) => table.setRowHeight(h),
                        RowHeight::Automatic => table.setRowHeight(44.0),
                    }
                    if !p.selectable {
                        table.setSelectionHighlightStyle(
                            objc2_app_kit::NSTableViewSelectionHighlightStyle::None,
                        );
                    }
                    // Transparent: day panes own their backgrounds — the stock
                    // controlBackgroundColor would paint an appearance-colored slab that
                    // fights a themed app (visible whenever rows don't fill the viewport).
                    table.setBackgroundColor(&objc2_app_kit::NSColor::clearColor());
                    table.setDataSource(Some(ProtocolObject::from_ref(&*data)));
                    table.setDelegate(Some(ProtocolObject::from_ref(&*data)));
                    if p.reorderable {
                        // Native drag-to-reorder (docs/list.md): rows drag within this table
                        // only (move locally, nothing leaves the app), and the drop target shows
                        // macOS's temporary placeholder GAP while the drag is over the table.
                        table.registerForDraggedTypes(
                            &objc2_foundation::NSArray::from_retained_slice(&[NSString::from_str(
                                DAY_ROW_PASTEBOARD_TYPE,
                            )]),
                        );
                        table.setDraggingSourceOperationMask_forLocal(
                            objc2_app_kit::NSDragOperation::Move,
                            true,
                        );
                        table.setDraggingSourceOperationMask_forLocal(
                            objc2_app_kit::NSDragOperation::None,
                            false,
                        );
                        table.setDraggingDestinationFeedbackStyle(
                            objc2_app_kit::NSTableViewDraggingDestinationFeedbackStyle::Gap,
                        );
                    }
                }
                let scroll = unsafe { NSScrollView::new(mtm) };
                unsafe {
                    scroll.setDrawsBackground(false);
                    scroll.setHasVerticalScroller(true);
                    // Overlay scrollers ALWAYS — a legacy scroller would reserve ~15pt and
                    // clip the trailing edge of rows laid out at the full frame width (see
                    // the SCROLL realize).
                    scroll.setScrollerStyle(objc2_app_kit::NSScrollerStyle::Overlay);
                    scroll.setDocumentView(Some(&table));
                }
                let view = view_of(scroll);
                LIST_STATE.with(|m| m.borrow_mut().insert(ptr_of(&view), (table, data)));
                view
            }
            Some(Builtin::Image) => {
                let p = props.downcast_ref::<ImageProps>().unwrap();
                let iv = unsafe { objc2_app_kit::NSImageView::new(mtm) };
                // Scaling (§18.3): NSImageView has no crop-fill, so Fit/Fill both scale-to-fit
                // proportionally; Stretch scales each axis independently.
                let scaling = match p.content_mode {
                    ContentMode::Stretch => objc2_app_kit::NSImageScaling::ScaleAxesIndependently,
                    _ => objc2_app_kit::NSImageScaling::ScaleProportionallyUpOrDown,
                };
                unsafe { iv.setImageScaling(scaling) };
                // A vector glyph's SVG first (docs/vectors.md): NSImage renders SVG at display
                // size (macOS 11+), so vectors stay vector — no build-time raster resampling.
                // Then the shared image-file resolver (images/ then assets/ then bundle) —
                // macOS AppKit's native path is a bundle file loaded straight into NSImage (§18.3).
                if let Some(path) = day_spec::resource::resolve_vector_svg(&p.source)
                    .or_else(|| day_spec::resource::resolve_image_file(&p.source))
                {
                    use objc2::AllocAnyThread as _;
                    if let Some(img) = unsafe {
                        objc2_app_kit::NSImage::initWithContentsOfFile(
                            objc2_app_kit::NSImage::alloc(),
                            &NSString::from_str(&path.to_string_lossy()),
                        )
                    } {
                        // Vector-glyph tint (docs/vectors.md): template rendering + the view's
                        // content tint — AppKit recolors the alpha mask natively.
                        if p.tint.is_some() {
                            unsafe { img.setTemplate(true) };
                        }
                        unsafe { iv.setImage(Some(&img)) };
                    }
                }
                if let Some(t) = p.tint {
                    unsafe { iv.setContentTintColor(Some(&nscolor(t))) };
                }
                view_of(iv)
            }
            // A recycled list cell is ADOPTED from the native list, never realized
            // through this path; anything else is an extension piece.
            Some(Builtin::ListCell) | None => {
                if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                    return make(self, props, id);
                }
                // Unregistered kind: LOUD once-per-kind warning, then a visible-but-harmless
                // placeholder (§8.2's debug check will panic first in debug builds once the
                // required-kinds set lands).
                warn_missing_renderer(kind);
                view_of(unsafe {
                    NSTextField::labelWithString(&NSString::from_str(&format!("⟨{kind}⟩")), mtm)
                })
            }
        }
    }

    fn update(&mut self, h: &Handle, kind: PieceKind, patch: &dyn Any, _anim: Option<&AnimSpec>) {
        match kind {
            kinds::CONTAINER => {
                if let (Some(ContainerPatch::Background(c)), Ok(v)) = (
                    patch.downcast_ref::<ContainerPatch>(),
                    h.clone().downcast::<DayFlipped>(),
                ) {
                    // A background patch only targets a background container (corner radius 0).
                    // The AnimSpec is deliberately not honored: the fill is drawRect CONTENT
                    // (rasterized by our own drawing code so dynamic system colors re-resolve
                    // per appearance), not a CALayer property, and Core Animation only
                    // interpolates layer properties — there is nothing for the render server
                    // to tween. Day animates only what the toolkit's own animator can execute
                    // (§8.4), so an animated background change applies at commit here.
                    v.set_surface(*c, 0.0, false);
                }
            }
            kinds::LABEL => {
                if let (Some(p), Ok(tf)) = (
                    patch.downcast_ref::<LabelPatch>(),
                    h.clone().downcast::<NSTextField>(),
                ) {
                    match p {
                        LabelPatch::Text(t) => unsafe { tf.setStringValue(&NSString::from_str(t)) },
                        LabelPatch::Color(c) => unsafe {
                            tf.setTextColor(c.map(nscolor).as_deref())
                        },
                        LabelPatch::Font(f) => unsafe { tf.setFont(Some(&nsfont(*f))) },
                    }
                }
            }
            kinds::BUTTON => {
                if let (Some(p), Ok(btn)) = (
                    patch.downcast_ref::<ButtonPatch>(),
                    h.clone().downcast::<NSButton>(),
                ) {
                    match p {
                        ButtonPatch::Title(t) => unsafe { btn.setTitle(&NSString::from_str(t)) },
                        ButtonPatch::Enabled(e) => unsafe { btn.setEnabled(*e) },
                    }
                }
            }
            kinds::TOGGLE => {
                if let (Some(p), Ok(sw)) = (
                    patch.downcast_ref::<TogglePatch>(),
                    h.clone().downcast::<NSSwitch>(),
                ) {
                    match p {
                        TogglePatch::On(on) => {
                            let want = if *on { 1 } else { 0 };
                            if unsafe { sw.state() } != want {
                                unsafe { sw.setState(want) };
                            }
                        }
                        TogglePatch::Enabled(e) => unsafe { sw.setEnabled(*e) },
                    }
                }
            }
            kinds::SLIDER => {
                if let (Some(p), Ok(sl)) = (
                    patch.downcast_ref::<SliderPatch>(),
                    h.clone().downcast::<NSSlider>(),
                ) {
                    match p {
                        SliderPatch::Value(v) => {
                            if (unsafe { sl.doubleValue() } - v).abs() > 0.001 {
                                unsafe { sl.setDoubleValue(*v) };
                            }
                        }
                        SliderPatch::Enabled(e) => unsafe { sl.setEnabled(*e) },
                    }
                }
            }
            kinds::PROGRESS => {
                if let (Some(ProgressPatch::Value(v)), Ok(pi)) = (
                    patch.downcast_ref::<ProgressPatch>(),
                    h.clone().downcast::<NSProgressIndicator>(),
                ) {
                    unsafe {
                        match v {
                            Some(val) => {
                                if pi.isIndeterminate() {
                                    pi.stopAnimation(None);
                                    pi.setIndeterminate(false);
                                    pi.setStyle(NSProgressIndicatorStyle::Bar);
                                    pi.setMinValue(0.0);
                                    pi.setMaxValue(1.0);
                                }
                                if (pi.doubleValue() - val).abs() > 0.0001 {
                                    pi.setDoubleValue(*val);
                                }
                            }
                            None => {
                                pi.setIndeterminate(true);
                                pi.setStyle(NSProgressIndicatorStyle::Spinning);
                                pi.startAnimation(None);
                            }
                        }
                    }
                }
            }
            kinds::TABS => {
                if let Some(tabview) = h.downcast_ref::<NSTabView>() {
                    match patch.downcast_ref::<TabsPatch>() {
                        Some(TabsPatch::Selected(i)) => TAB_STATE.with(|m| {
                            if let Some(state) = m.borrow().get(&ptr_of(h)) {
                                state.delegate.ivars().suppress.set(true);
                                unsafe { tabview.selectTabViewItemAtIndex(*i as isize) };
                                state.delegate.ivars().suppress.set(false);
                            }
                        }),
                        // Data-driven tabs: the pages were added/removed via insert/remove; sync
                        // each remaining tab item's label to the new titles, then select.
                        Some(TabsPatch::Items {
                            titles, selected, ..
                        }) => TAB_STATE.with(|m| {
                            if let Some(state) = m.borrow().get(&ptr_of(h)) {
                                let items = unsafe { tabview.tabViewItems() };
                                // Enumerate the titles (the value we apply); `i` only indexes the
                                // separate `items` array, so this isn't a needless range loop.
                                for (i, title) in titles.iter().enumerate().take(items.count()) {
                                    unsafe {
                                        items.objectAtIndex(i).setLabel(&NSString::from_str(title));
                                    }
                                }
                                state.delegate.ivars().suppress.set(true);
                                if *selected < items.count() {
                                    unsafe { tabview.selectTabViewItemAtIndex(*selected as isize) };
                                }
                                state.delegate.ivars().suppress.set(false);
                            }
                        }),
                        None => {}
                    }
                }
            }
            kinds::NAV_MENU => {
                if let Some(NavMenuPatch::Items {
                    items,
                    icons,
                    badges,
                    sections,
                    tints,
                    menus,
                    selected,
                }) = patch.downcast_ref::<NavMenuPatch>()
                {
                    NAV_MENUS.with(|m| {
                        let m = m.borrow();
                        let Some((outline, data)) = m.get(&ptr_of(h)) else {
                            return;
                        };
                        data.set_items(items, icons, badges, sections, tints, menus);
                        data.ivars().suppress.set(true);
                        unsafe {
                            outline.reloadData();
                            match selected.and_then(|i| data.row_of_item(i)) {
                                Some(row) => outline.selectRowIndexes_byExtendingSelection(
                                    &objc2_foundation::NSIndexSet::indexSetWithIndex(row),
                                    false,
                                ),
                                None => outline.deselectAll(None),
                            }
                        }
                        data.ivars().suppress.set(false);
                    });
                } else if let Some(NavMenuPatch::Selected(sel)) =
                    patch.downcast_ref::<NavMenuPatch>()
                {
                    NAV_MENUS.with(|m| {
                        let m = m.borrow();
                        let Some((outline, data)) = m.get(&ptr_of(h)) else {
                            return;
                        };
                        data.ivars().suppress.set(true);
                        unsafe {
                            match sel.and_then(|i| data.row_of_item(i)) {
                                Some(row) => outline.selectRowIndexes_byExtendingSelection(
                                    &objc2_foundation::NSIndexSet::indexSetWithIndex(row),
                                    false,
                                ),
                                None => outline.deselectAll(None),
                            }
                        }
                        data.ivars().suppress.set(false);
                    });
                }
            }
            kinds::NAV => {
                if let Some(p) = patch.downcast_ref::<NavPatch>() {
                    NAV_STATE.with(|m| {
                        let mut m = m.borrow_mut();
                        let Some(state) = m.get_mut(&ptr_of(h)) else {
                            return;
                        };
                        match p {
                            NavPatch::Pushed { title, .. } => {
                                // Only the new top detail page stays visible.
                                let last = state.pages.len().saturating_sub(1);
                                for (i, page) in state.pages.iter().enumerate() {
                                    page.setHidden(i != last);
                                }
                                if let Some(hdr) = state.header.as_mut() {
                                    hdr.titles.push(title.clone());
                                    sync_nav_header(hdr, &state.detail_wrap, &state.pages);
                                }
                            }
                            NavPatch::Popped => {
                                // Hide the outgoing top; reveal its predecessor (Day
                                // removes the popped page right after this patch).
                                let n = state.pages.len();
                                if let Some(top) = state.pages.last() {
                                    top.setHidden(true);
                                }
                                if n >= 2 {
                                    state.pages[n - 2].setHidden(false);
                                }
                                if let Some(hdr) = state.header.as_mut() {
                                    if hdr.titles.len() > 1 {
                                        hdr.titles.pop();
                                    }
                                    // The popped page is still in `pages` here (Day removes
                                    // it right after this patch) — frame for depth-after-pop.
                                    let after: Vec<Retained<NSView>> = state
                                        .pages
                                        .iter()
                                        .take(n.saturating_sub(1))
                                        .cloned()
                                        .collect();
                                    sync_nav_header(hdr, &state.detail_wrap, &after);
                                }
                            }
                            NavPatch::Title(t) => {
                                if let Some(hdr) = state.header.as_mut() {
                                    if let Some(last) = hdr.titles.last_mut() {
                                        *last = t.clone();
                                    }
                                    unsafe {
                                        hdr.title.setStringValue(&NSString::from_str(t));
                                    }
                                }
                            }
                            // The custom back header always routes back through Day
                            // (NavBack{already_popped:false}), so there is no native auto-pop to
                            // suppress — the guard runs in the pieces layer (docs/navigation.md).
                            NavPatch::GuardTop(_) => {}
                        }
                    });
                }
            }
            // Emulated cover (docs/cover.md): present = re-home onto the window's content view
            // at full bounds, topmost, autoresized with the window (the DayNavPage handle
            // reports FrameChanged on every resize). Dismiss = hide + report "cover-hidden"
            // immediately (no transition on this tier). Interactive dismissal doesn't exist on
            // this backend, so DismissDisabled has nothing to disable.
            kinds::COVER => {
                if let (Some(p), Ok(page)) = (
                    patch.downcast_ref::<CoverPatch>(),
                    h.clone().downcast::<DayNavPage>(),
                ) {
                    let node = page.ivars().node;
                    match p {
                        CoverPatch::Present { background, .. } => {
                            // A cover must OCCLUDE the window (the native tiers' modal surfaces
                            // are opaque): default to the window background when the app sets
                            // no explicit color.
                            unsafe {
                                page.setWantsLayer(true);
                                if let Some(layer) = page.layer() {
                                    let color = match background {
                                        Some(bg) => nscolor(*bg),
                                        None => NSColor::windowBackgroundColor(),
                                    };
                                    layer.setBackgroundColor(Some(&color.CGColor()));
                                }
                            }
                            // The PRIMARY window's content, specifically — firstObject()
                            // is arbitrary once secondary windows exist (docs/windows.md).
                            if let Some(content) = primary_content() {
                                unsafe {
                                    page.removeFromSuperview();
                                    content.addSubview(&page);
                                    page.setFrame(content.bounds());
                                    page.setAutoresizingMask(
                                        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                                            | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
                                    );
                                    page.setHidden(false);
                                }
                                let b = content.bounds();
                                emit(
                                    node,
                                    Event::FrameChanged(Size::new(b.size.width, b.size.height)),
                                );
                            }
                        }
                        CoverPatch::DismissDisabled(_) => {}
                        CoverPatch::Dismiss => {
                            unsafe {
                                page.setHidden(true);
                                page.removeFromSuperview();
                            }
                            emit(node, Event::custom("cover-hidden", ""));
                        }
                    }
                }
            }
            kinds::PICKER => picker::update_any(self, h, patch),
            kinds::TEXT_AREA => textarea::update_any(self, h, patch),
            kinds::TEXT_FIELD => {
                if let (Some(p), Ok(tf)) = (
                    patch.downcast_ref::<TextFieldPatch>(),
                    h.clone().downcast::<NSTextField>(),
                ) {
                    match p {
                        TextFieldPatch::Text { text, from_native } => {
                            // Origin-tagged echo suppression (§4.4).
                            if !*from_native && unsafe { tf.stringValue() }.to_string() != *text {
                                unsafe { tf.setStringValue(&NSString::from_str(text)) };
                            }
                        }
                        TextFieldPatch::Placeholder(t) => unsafe {
                            tf.setPlaceholderString(Some(&NSString::from_str(t)))
                        },
                        TextFieldPatch::Enabled(e) => unsafe { tf.setEnabled(*e) },
                    }
                }
            }
            kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                Some(ListPatch::Reload) => {
                    LIST_STATE.with(|m| {
                        if let Some((table, data)) = m.borrow().get(&ptr_of(h)) {
                            // A reload whose rows are the SAME set in a new order (a shuffle,
                            // a programmatic sort) animates as native row moves instead of a
                            // blink — `moveRowAtIndex` batch, the same animation a drag commit
                            // gets. Anything else (insert/remove/content change) reloads flat.
                            // reloadData queries numberOfRows synchronously (snapshot only, no
                            // tree) and defers viewForRow, so both paths are safe in with_tree.
                            if let Some(moves) = data.permutation_moves(table) {
                                unsafe {
                                    table.beginUpdates();
                                    for (from, to) in moves {
                                        table.moveRowAtIndex_toIndex(from as isize, to as isize);
                                    }
                                    table.endUpdates();
                                }
                            } else {
                                unsafe { table.reloadData() };
                            }
                        }
                    });
                    // Realize the visible rows on the next main-loop turn, outside this borrow.
                    // An occluded window (locked screen, covered, headless CI) gets no normal
                    // draw pass — NSTableView would first realize these rows inside a snapshot's
                    // `cacheDisplayInRect`, where `bind_row` must skip the held borrow and the
                    // table would cache permanently blank cells.
                    post_realize_visible_rows(ptr_of(h));
                }
                Some(ListPatch::ScrollToRow(row)) => {
                    // Same deferral discipline as ScrollToEnd: the scroll tiles target rows
                    // synchronously, which must happen outside this `with_tree` borrow.
                    let (key, row) = (ptr_of(h), *row);
                    <AppKit as Platform>::post(Box::new(move || {
                        LIST_STATE.with(|m| {
                            if let Some((table, _)) = m.borrow().get(&key) {
                                let rows = unsafe { table.numberOfRows() };
                                if rows > 0 {
                                    unsafe {
                                        table.scrollRowToVisible((row as isize).min(rows - 1))
                                    };
                                }
                            }
                        });
                    }));
                    post_realize_visible_rows(ptr_of(h));
                }
                Some(ListPatch::ScrollToEnd) => {
                    // Deferred to the next main-loop turn: an actual scroll forces NSTableView
                    // to tile (realize) the target rows SYNCHRONOUSLY, and this patch arrives
                    // inside a `with_tree` borrow where `bind_row` must skip — the table would
                    // cache blank cells for every newly exposed row (see Reload above).
                    let key = ptr_of(h);
                    <AppKit as Platform>::post(Box::new(move || {
                        LIST_STATE.with(|m| {
                            if let Some((table, _)) = m.borrow().get(&key) {
                                let rows = unsafe { table.numberOfRows() };
                                if rows > 0 {
                                    unsafe { table.scrollRowToVisible(rows - 1) };
                                }
                            }
                        });
                    }));
                    // ...and realize whatever the scroll exposed, still outside the borrow.
                    post_realize_visible_rows(ptr_of(h));
                }
                Some(ListPatch::Selected(rows)) => {
                    // Programmatic selection sync (empty = clear) — suppressed, so the
                    // delegate does not echo it back as a selection event.
                    LIST_STATE.with(|m| {
                        if let Some((table, data)) = m.borrow().get(&ptr_of(h)) {
                            data.ivars().suppress.set(true);
                            unsafe {
                                if rows.is_empty() {
                                    table.deselectAll(None);
                                } else {
                                    let set = objc2_foundation::NSMutableIndexSet::new();
                                    for r in rows {
                                        set.addIndex(*r);
                                    }
                                    table.selectRowIndexes_byExtendingSelection(&set, false);
                                }
                            }
                            data.ivars().suppress.set(false);
                        }
                    });
                }
                // NSTableView has no per-row invalidation seam here: a row keeps its height
                // until the next Reload. `None` = a patch for another kind's enum.
                Some(ListPatch::RowSizeInvalidated(_)) | None => {}
            },
            _ => {
                if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                    update(self, h, patch);
                }
            }
        }
    }

    fn release(&mut self, h: Handle) {
        // A released window content = that window is gone (docs/windows.md teardown):
        // drop the SecondaryWin — closing a straggler NSWindow — and its delegate with it.
        // (A re-fired windowWillClose emits to a torn-down node: dropped, harmless.)
        self.secondary.retain(|w| {
            if ptr_of(&w.content) == ptr_of(&h) {
                w.window.close();
                false
            } else {
                true
            }
        });
        TARGETS.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        LIST_STATE.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        GESTURES.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        NAV_STATE.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        NAV_PAGES.with(|set| {
            set.borrow_mut().remove(&ptr_of(&h));
        });
        NAV_MENUS.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        TAB_STATE.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        TAB_TITLES.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        unsafe { h.removeFromSuperview() };
    }

    fn insert(&mut self, parent: &Handle, child: &Handle, index: usize) {
        // Tabs host: wrap the page in an NSTabViewItem with its label, then insert. NSTabView
        // owns the item view's frame; the page reports its content size via FrameChanged.
        let handled_tab = TAB_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&ptr_of(parent)) else {
                return false;
            };
            let Some(tabview) = parent.downcast_ref::<NSTabView>() else {
                return false;
            };
            let title = TAB_TITLES
                .with(|t| t.borrow().get(&ptr_of(child)).cloned())
                .unwrap_or_default();
            let item = unsafe { NSTabViewItem::new() };
            unsafe {
                item.setLabel(&NSString::from_str(&title));
                item.setView(Some(child));
                tabview.insertTabViewItem_atIndex(&item, index as isize);
            }
            // Select the requested initial tab once its item exists (suppress the echo).
            if index == state.initial {
                state.delegate.ivars().suppress.set(true);
                unsafe { tabview.selectTabViewItemAtIndex(index as isize) };
                state.delegate.ivars().suppress.set(false);
            }
            true
        });
        if handled_tab {
            return;
        }
        // Nav host: index 0 = sidebar page, the rest are detail (stack) pages. Pages fill
        // their pane via autoresizing — the pane, not Day, owns their frames.
        let handled = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&ptr_of(parent)) else {
                return false;
            };
            // Split (selector Sidebar): index 0 is the sidebar; the rest are detail pages.
            // Stack (`split == false`): every page — including the root — lives in the detail
            // pane so push/pop visibility covers them all.
            let (wrap, frame) = if state.split && index == 0 {
                (&state.sidebar_wrap, state.sidebar_wrap.bounds())
            } else {
                state.pages.push(child.clone());
                let visible = state.header.as_ref().is_some_and(|h| !h.bar.isHidden());
                (
                    &state.detail_wrap,
                    nav_page_frame(&state.detail_wrap, visible),
                )
            };
            unsafe {
                child.setFrame(frame);
                child.setAutoresizingMask(
                    objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable
                        | objc2_app_kit::NSAutoresizingMaskOptions::ViewHeightSizable,
                );
                wrap.addSubview(child);
            }
            true
        });
        if !handled {
            // Absolute positioning: z-order is build order; index is irrelevant for
            // non-overlapping frames (stack_z will need ordered insertion later).
            unsafe { content_of(parent).addSubview(child) };
        }
    }

    fn remove(&mut self, parent: &Handle, child: &Handle) {
        NAV_STATE.with(|m| {
            if let Some(state) = m.borrow_mut().get_mut(&ptr_of(parent)) {
                state.pages.retain(|p| ptr_of(p) != ptr_of(child));
            }
        });
        // Data-driven tabs: a removed page's NSTabViewItem must go too (removeFromSuperview
        // leaves an orphan empty tab otherwise).
        if let Some(tabview) = parent.downcast_ref::<NSTabView>() {
            let items = unsafe { tabview.tabViewItems() };
            for i in 0..items.count() {
                let it = items.objectAtIndex(i);
                if unsafe { it.view(self.mtm()) }.map(|v| ptr_of(&v)) == Some(ptr_of(child)) {
                    unsafe { tabview.removeTabViewItem(&it) };
                    break;
                }
            }
        }
        unsafe { child.removeFromSuperview() };
    }

    fn move_child(&mut self, parent: &Handle, child: &Handle, _to: usize) {
        unsafe { content_of(parent).addSubview(child) };
    }

    fn measure(&mut self, h: &Handle, kind: PieceKind, p: Proposal) -> Size {
        match kind {
            kinds::LABEL => {
                if let Some(tf) = h.downcast_ref::<NSTextField>()
                    && let Some(cell) = unsafe { tf.cell() }
                {
                    let w = p.width.unwrap_or(1.0e6);
                    let s = unsafe {
                        cell.cellSizeForBounds(NSRect::new(
                            NSPoint::new(0.0, 0.0),
                            NSSize::new(w, 1.0e6),
                        ))
                    };
                    return Size::new(s.width.ceil().min(w), s.height.ceil());
                }
                Size::ZERO
            }
            kinds::BUTTON | kinds::TOGGLE => {
                let s = unsafe { h.fittingSize() };
                Size::new(s.width.ceil(), s.height.ceil())
            }
            kinds::SLIDER => {
                let s = unsafe { h.fittingSize() };
                Size::new(p.width.unwrap_or(180.0), s.height.max(21.0).ceil())
            }
            kinds::PICKER => picker::measure_any(self, h, p),
            kinds::TEXT_AREA => textarea::measure_any(self, h, p),
            kinds::TEXT_FIELD => {
                let s = unsafe { h.fittingSize() };
                Size::new(
                    p.width.unwrap_or(s.width.max(160.0)),
                    s.height.max(22.0).ceil(),
                )
            }
            kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 5.0),
            kinds::PROGRESS => {
                // Indeterminate spinner is a fixed square; determinate bar fills width.
                let indeterminate = h
                    .clone()
                    .downcast::<NSProgressIndicator>()
                    .map(|pi| unsafe { pi.isIndeterminate() })
                    .unwrap_or(false);
                if indeterminate {
                    Size::new(20.0, 20.0)
                } else {
                    Size::new(p.width.unwrap_or(180.0), 20.0)
                }
            }
            kinds::NAV_MENU => {
                let rows = NAV_MENUS.with(|m| {
                    m.borrow()
                        .get(&ptr_of(h))
                        .map(|(_, d)| d.ivars().items.borrow().len())
                        .unwrap_or(0)
                });
                Size::new(
                    p.width.unwrap_or(220.0),
                    p.height.unwrap_or(rows as f64 * 32.0 + 12.0),
                )
            }
            // The recycling list fills the space it is offered (its native scroll owns overflow).
            kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
            _ => {
                if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                    measure(self, h, p)
                } else {
                    let s = unsafe { h.fittingSize() };
                    Size::new(p.width.unwrap_or(s.width), p.height.unwrap_or(s.height))
                }
            }
        }
    }

    fn set_opacity(&mut self, h: &Handle, opacity: f64, anim: Option<&AnimSpec>) {
        unsafe {
            h.setWantsLayer(true);
        }
        let v = h.clone();
        with_appkit_anim(anim, move || unsafe { v.setAlphaValue(opacity) });
    }

    fn set_transform(&mut self, h: &Handle, t: Transform, _size: Size, anim: Option<&AnimSpec>) {
        // Scale → rotate → translate about the layer's center anchor (matches UIKit); Day's
        // containers are flipped (y-down), so the sense matches the mobile backends.
        let th = t.rotate_deg.to_radians();
        let (s, c) = th.sin_cos();
        let cg = CGAffineTransform {
            a: t.sx * c,
            b: t.sx * s,
            c: -t.sy * s,
            d: t.sy * c,
            tx: t.tx,
            ty: t.ty,
        };
        let v = h.clone();
        with_appkit_anim(anim, move || unsafe {
            v.setWantsLayer(true);
            let layer: *mut objc2::runtime::AnyObject = msg_send![&*v, layer];
            if !layer.is_null() {
                let _: () = msg_send![layer, setAffineTransform: cg];
            }
        });
    }

    fn set_selectable(&mut self, h: &Handle, selectable: bool) {
        // A plain label backs onto an NSTextField (docs/text.md); make its text selectable
        // (copy/drag). The downcast is the guard: a backing that isn't a text field no-ops rather
        // than mis-cast — a future rich/link label on NSTextView would add its own arm.
        if let Some(tf) = h.downcast_ref::<NSTextField>() {
            unsafe { tf.setSelectable(selectable) };
        }
    }

    fn set_frame(&mut self, h: &Handle, frame: Rect, _anim: Option<&AnimSpec>) {
        // Nav pages: the splitter pane / nav container owns the frame (autoresized).
        if NAV_PAGES.with(|set| set.borrow().contains(&ptr_of(h))) {
            return;
        }
        // Every Day parent is flipped (DayFlipped containers, flipped scroll document views),
        // so top-left frames apply directly.
        let r = NSRect::new(
            NSPoint::new(frame.origin.x, frame.origin.y),
            NSSize::new(frame.size.width, frame.size.height),
        );
        // Nav host: the sidebar should HOLD its width when the window resizes, letting the
        // detail pane absorb the change (the standard Finder/Mail behavior). NSSplitView's
        // holding priorities don't take effect when Day drives the split's frame directly, so
        // we capture the current sidebar width, resize, then restore the divider to it.
        if let Some(split) = h.downcast_ref::<objc2_app_kit::NSSplitView>() {
            let (first, is_split) = NAV_STATE.with(|m| {
                m.borrow_mut()
                    .get_mut(&ptr_of(h))
                    .map(|s| (!std::mem::replace(&mut s.positioned, true), s.split))
                    .unwrap_or((false, true))
            });
            // Sidebar pane = arranged subview 0; its width is the divider position.
            let prev_sidebar = {
                let subs = split.subviews();
                if subs.count() > 0 {
                    subs.objectAtIndex(0).frame().size.width
                } else {
                    0.0
                }
            };
            unsafe {
                split.setFrame(r);
                split.layoutSubtreeIfNeeded();
                // A stack collapses the (empty) sidebar so the detail is full-width.
                let target = if !is_split {
                    0.0
                } else if first || prev_sidebar <= 1.0 {
                    day_spec::NAV_SIDEBAR_WIDTH
                } else {
                    prev_sidebar
                };
                split.setPosition_ofDividerAtIndex(target, 0);
            }
        } else {
            unsafe { h.setFrame(r) };
        }
    }

    fn set_scroll_content(&mut self, h: &Handle, content: Size) {
        if let Some(sv) = h.downcast_ref::<NSScrollView>()
            && let Some(doc) = unsafe { sv.documentView() }
        {
            unsafe { doc.setFrameSize(NSSize::new(content.width, content.height)) };
        }
    }

    fn scroll_to(&mut self, h: &Handle, target: Rect, _animated: bool) {
        if let Some(sv) = h.downcast_ref::<NSScrollView>()
            && let Some(doc) = unsafe { sv.documentView() }
        {
            unsafe {
                doc.scrollRectToVisible(NSRect::new(
                    NSPoint::new(target.origin.x, target.origin.y),
                    NSSize::new(target.size.width, target.size.height),
                ))
            };
        }
    }

    fn focus(&mut self, h: &Handle, _node: NodeId, focused: bool) {
        let Some(window) = h.window() else { return };
        let responder: &NSResponder = h;
        if focused {
            window.makeFirstResponder(Some(responder));
            return;
        }
        // Resign only while this view still owns focus, so a stale release can't blur a
        // sibling. A focused NSTextField's first responder is the shared field editor —
        // unwrap it back to the field via its delegate.
        let owns = window.firstResponder().is_some_and(|fr| {
            if Retained::as_ptr(&fr) as usize == ptr_of(h) {
                return true;
            }
            fr.downcast::<NSText>().is_ok_and(|text| {
                unsafe { text.delegate() }
                    .is_some_and(|d| Retained::as_ptr(&d) as *const () as usize == ptr_of(h))
            })
        });
        if owns {
            window.makeFirstResponder(None);
        }
    }

    fn set_event_sink(&mut self, sink: EventSink) {
        SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
    }

    fn attach_list(&mut self, host: &Handle, source: ListSource) {
        let key = ptr_of(host);
        LIST_STATE.with(|m| {
            if let Some((table, data)) = m.borrow().get(&key) {
                data.ivars().source.replace(Some(source));
                // Initial fill: numberOfRows reads the snapshot only; viewForRow is deferred.
                unsafe { table.reloadData() };
            }
        });
        // Force the table to realize its visible row views on the NEXT main-loop turn — OUTSIDE
        // any `with_tree` borrow — so `viewForRow`/`bind_row` build the cells then. Otherwise a
        // headless CI window never lays the table out until a snapshot's `cacheDisplayInRect`
        // forces it *inside* the snapshot borrow, where `bind_row` must skip (blank rows).
        <AppKit as Platform>::post(Box::new(move || {
            LIST_STATE.with(|m| {
                if let Some((table, _)) = m.borrow().get(&key) {
                    unsafe { table.layoutSubtreeIfNeeded() };
                }
            });
        }));
    }

    fn adopt(&mut self, raw: RawHandle) -> Handle {
        // A recycling NSTableView cell view — Day builds/rebinds its row content in place.
        let ptr = raw as *mut NSView;
        unsafe { Retained::retain(ptr) }.expect("adopt: null list cell handle")
    }

    fn set_toolbar(&mut self, h: &Handle, items: &[day_spec::ToolbarItem]) {
        self.install_toolbar(h, items);
    }

    fn update_toolbar(&mut self, h: &Handle, patch: &day_spec::ToolbarPatch) {
        self.patch_toolbar(h, patch);
    }

    fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
        let mtm = self.mtm;
        let app = NSApplication::sharedApplication(mtm);
        let menubar = NSMenu::new(mtm);
        // The Preferences item's standard macOS home is the App menu, under About — hoist
        // it out of wherever the model carries it (day-core injects it into File for the
        // other desktops, docs/windows.md).
        let mut items = items.to_vec();
        let prefs = extract_preferences(&mut items);
        // macOS mandates a leading app menu (shown as the app name); provide the standard one so the
        // app's `app_menu(...)` supplies only the rest (File/Edit/View/…), staying convention-native.
        let app_item = NSMenuItem::new(mtm);
        let mut app_menu_items = vec![
            day_spec::MenuItem::Action {
                id: 0,
                label: about_label(&self.app_name),
                shortcut: None,
                enabled: true,
                role: Some(day_spec::MenuRole::About),
            },
            day_spec::MenuItem::Separator,
        ];
        if let Some(p) = prefs {
            app_menu_items.push(p);
            app_menu_items.push(day_spec::MenuItem::Separator);
        }
        app_menu_items.push(day_spec::MenuItem::Action {
            id: 0,
            label: quit_label(&self.app_name),
            shortcut: None,
            enabled: true,
            role: Some(day_spec::MenuRole::Quit),
        });
        let app_menu = build_ns_menu(mtm, &self.app_name, &app_menu_items);
        app_item.setSubmenu(Some(&app_menu));
        menubar.addItem(&app_item);
        // Fill the standard slots the app did not claim, in the platform's bar order
        // (day-core owns that policy so every backend arranges its bar the same way).
        // Fill the standard slots the app left open, in macOS's bar order. The Window menu is
        // installed natively below, so the style deliberately supplies none.
        let items =
            day_core::menu::standard_menu_bar_for(day_core::menu::MenuBarStyle::Macos, items);
        let mut help_menu: Option<(String, Retained<NSMenu>)> = None;
        // Each top-level entry becomes a menu-bar menu.
        for item in &items {
            match item {
                day_spec::MenuItem::Submenu { label, items, role } => {
                    let sub = build_ns_menu(mtm, label, items);
                    let it = NSMenuItem::new(mtm);
                    // Help is held back: the Window menu is installed natively AFTER this loop
                    // (AppKit owns it so it can append the live window list), and Help sits
                    // last on every Mac menu bar.
                    if *role == Some(day_spec::MenuBarRole::Help)
                        || *label == day_l10n::t("day-help")
                    {
                        help_menu = Some((label.clone(), sub));
                        continue;
                    }
                    it.setTitle(&NSString::from_str(label));
                    it.setSubmenu(Some(&sub));
                    menubar.addItem(&it);
                }
                other => {
                    // A bare top-level action → wrap in a one-item menu so it has a submenu.
                    let sub = build_ns_menu(mtm, "", std::slice::from_ref(other));
                    let it = NSMenuItem::new(mtm);
                    it.setSubmenu(Some(&sub));
                    menubar.addItem(&it);
                }
            }
        }
        // The Window menu (docs/windows.md): auto-installed unless the app's model owns
        // `MenuRole::Minimize` (then the app is composing its own window management).
        if !model_has_role(&items, day_spec::MenuRole::Minimize) {
            install_windows_menu(mtm, &app, &menubar);
        }
        if let Some((label, help)) = help_menu {
            let it = NSMenuItem::new(mtm);
            it.setTitle(&NSString::from_str(&label));
            it.setSubmenu(Some(&help));
            menubar.addItem(&it);
            // Registering it makes AppKit add the Help search field and index the help book —
            // behavior no hand-built menu reproduces.
            unsafe { app.setHelpMenu(Some(&help)) };
        }
        app.setMainMenu(Some(&menubar));
    }

    fn set_context_menu(&mut self, h: &Handle, _node: NodeId, items: &[day_spec::MenuItem]) {
        let Some(view) = h.downcast_ref::<NSView>() else {
            return;
        };
        if items.is_empty() {
            unsafe { view.setMenu(None) };
            return;
        }
        let menu = build_ns_menu(self.mtm, "", items);
        // Attach to the view AND its current subviews: a right-click that hit-tests a child
        // control (the label inside a padded target) must find the menu on the view it hit —
        // AppKit does not walk ancestors for `.menu`, and controls swallow the event.
        // DayFlipped's `rightMouseDown:` pops the menu explicitly; native controls (labels)
        // show their `.menu` through their own default path.
        unsafe { view.setMenu(Some(&menu)) };
        fn apply_to_subviews(v: &NSView, menu: &NSMenu) {
            for sub in v.subviews() {
                unsafe { sub.setMenu(Some(menu)) };
                apply_to_subviews(&sub, menu);
            }
        }
        apply_to_subviews(view, &menu);
    }

    fn enable_gesture(&mut self, h: &Handle, node: NodeId, kind: day_spec::GestureKind) {
        let key = ptr_of(h);
        // Idempotent: attach each kind at most once per view.
        let already = GESTURES.with(|m| {
            m.borrow().get(&key).is_some_and(|v| {
                v.iter()
                    .any(|t| t.ivars().is_drag == matches!(kind, day_spec::GestureKind::Drag))
            })
        });
        if already {
            return;
        }
        let mtm = self.mtm;
        let is_drag = matches!(kind, day_spec::GestureKind::Drag);
        let target = DayGesture::new(mtm, node, is_drag);
        let recognizer: Retained<NSGestureRecognizer> = unsafe {
            match kind {
                day_spec::GestureKind::Drag => {
                    Retained::into_super(NSPanGestureRecognizer::initWithTarget_action(
                        NSPanGestureRecognizer::alloc(mtm),
                        Some(&target),
                        Some(sel!(fire:)),
                    ))
                }
                _ => Retained::into_super(NSClickGestureRecognizer::initWithTarget_action(
                    NSClickGestureRecognizer::alloc(mtm),
                    Some(&target),
                    Some(sel!(fire:)),
                )),
            }
        };
        unsafe { h.addGestureRecognizer(&recognizer) };
        GESTURES.with(|m| m.borrow_mut().entry(key).or_default().push(target));
    }

    fn set_a11y(&mut self, h: &Handle, a11y: &A11yProps) {
        unsafe {
            if let Some(id) = &a11y.identifier {
                h.setAccessibilityIdentifier(Some(&NSString::from_str(id)));
            }
            if let Some(label) = &a11y.label {
                h.setAccessibilityLabel(Some(&NSString::from_str(label)));
            }
            if let Some(hint) = &a11y.hint {
                h.setAccessibilityHelp(Some(&NSString::from_str(hint)));
            }
            if let Some(value) = &a11y.value {
                let ns = NSString::from_str(value);
                h.setAccessibilityValue(Some(ns.as_ref() as &objc2::runtime::AnyObject));
            }
            // Only apply an EXPLICIT role (canvas/custom pieces, e.g. a Meter): native controls
            // already report the right role, so Day records but doesn't override theirs (§13).
            if let Some(role) = ns_role(a11y.role) {
                h.setAccessibilityRole(Some(role));
            }
            // Decorative / hidden: drop from the AX tree entirely.
            if a11y.hidden {
                h.setAccessibilityElement(false);
            }
        }
    }

    fn read_a11y(&self, h: &Handle) -> day_spec::A11ySnapshot {
        unsafe {
            let role = h
                .accessibilityRole()
                .map(|r| day_role_from_ns(&r.to_string()))
                .unwrap_or(day_spec::Role::None);
            day_spec::A11ySnapshot {
                found: true,
                role,
                label: h.accessibilityLabel().map(|s| s.to_string()),
                value: h
                    .accessibilityValue()
                    .and_then(|v| v.downcast_ref::<NSString>().map(|s| s.to_string())),
                identifier: h
                    .accessibilityIdentifier()
                    .map(|s| s.to_string())
                    .filter(|s| !s.is_empty()),
            }
        }
    }

    fn replay(&mut self, h: &Handle, ops: &[DrawOp], _size: Size) {
        OPS.with(|m| m.borrow_mut().insert(ptr_of(h), ops.to_vec()));
        unsafe { h.setNeedsDisplay(true) };
    }

    fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
        let content = self.content.as_ref().ok_or("no window content")?;
        snapshot_view(content)
    }

    fn snapshot_window_of(&mut self, host: &Handle) -> Result<Vec<u8>, String> {
        snapshot_view(host)
    }

    fn open_window(
        &mut self,
        id: NodeId,
        options: &WindowOptions,
        kind: day_spec::WindowKind,
    ) -> day_spec::WindowOpenReply<Handle> {
        let prefs = kind == day_spec::WindowKind::Preferences;
        let (window, delegate, content) = self.make_window(
            &options.title,
            options.size,
            options.min_size,
            prefs,
            Some(id),
        );
        window.center();
        window.makeKeyAndOrderFront(None);
        // Same macOS 26 quirk as the primary (`run`): a window ordered front before its
        // first turn drops pre-run layer displays — nudge every layer once.
        mark_tree_needs_display(&content);
        self.secondary.push(SecondaryWin {
            window,
            delegate,
            content: content.clone(),
        });
        day_spec::WindowOpenReply::Open(content)
    }

    fn close_window(&mut self, host: &Handle) {
        // `close()` runs the full native path — windowWillClose → `WindowClosed` →
        // day-core teardown — so programmatic and title-bar closes are one code path.
        if let Some(w) = self
            .secondary
            .iter()
            .find(|w| ptr_of(&w.content) == ptr_of(host))
        {
            w.window.close();
        }
    }

    fn focus_window(&mut self, host: &Handle) {
        if let Some(w) = self
            .secondary
            .iter()
            .find(|w| ptr_of(&w.content) == ptr_of(host))
        {
            w.window.makeKeyAndOrderFront(None);
        }
    }

    fn set_window_title(&mut self, host: &Handle, title: &str) {
        if let Some(w) = self
            .secondary
            .iter()
            .find(|w| ptr_of(&w.content) == ptr_of(host))
        {
            w.window.setTitle(&NSString::from_str(title));
        }
    }

    fn present(&mut self, req: u64, spec: &present::PresentSpec) {
        use present::PresentSpec;
        let mtm = self.mtm;
        // Sheets attach to the KEY window at present time (docs/windows.md) — a dialog
        // raised from a secondary/preferences window belongs on it; primary is the
        // fallback when no DAY window is key (a system panel may hold key status).
        let app = NSApplication::sharedApplication(mtm);
        let key = app.keyWindow().filter(|k| {
            self.window
                .as_deref()
                .is_some_and(|w| std::ptr::eq(w, &**k))
                || self
                    .secondary
                    .iter()
                    .any(|s| std::ptr::eq(&*s.window, &**k))
        });
        let Some(window) = key.or_else(|| self.window.clone()) else {
            emit(
                WINDOW_NODE,
                Event::PresentResult {
                    req,
                    result: present::PresentResult::Dismissed,
                },
            );
            return;
        };
        // File pickers use a different native object (NSOpen/NSSavePanel), not an NSAlert.
        match spec {
            PresentSpec::OpenFile { filters, .. } => {
                let panel = unsafe { objc2_app_kit::NSOpenPanel::openPanel(mtm) };
                unsafe {
                    panel.setCanChooseFiles(true);
                    panel.setCanChooseDirectories(false);
                    panel.setAllowsMultipleSelection(false);
                    panel.setMessage(Some(&NSString::from_str(spec.title())));
                }
                apply_allowed_file_types(&panel, filters);
                let p = panel.clone();
                let handler: block2::RcBlock<dyn Fn(isize)> =
                    block2::RcBlock::new(move |resp: isize| {
                        emit_panel_result(req, resp, &p);
                    });
                unsafe { panel.beginSheetModalForWindow_completionHandler(&window, &handler) };
                PRESENT_PANELS.with(|m| m.borrow_mut().insert(req, Retained::into_super(panel)));
                return;
            }
            PresentSpec::SaveFile { suggested_name, .. } => {
                let panel = unsafe { objc2_app_kit::NSSavePanel::savePanel(mtm) };
                unsafe {
                    panel.setMessage(Some(&NSString::from_str(spec.title())));
                    panel.setNameFieldStringValue(&NSString::from_str(suggested_name));
                }
                apply_allowed_file_types(&panel, spec.filters());
                let p = panel.clone();
                let handler: block2::RcBlock<dyn Fn(isize)> =
                    block2::RcBlock::new(move |resp: isize| {
                        // The pieces layer copies the staged bytes to the chosen local path.
                        emit_panel_result(req, resp, &p);
                    });
                unsafe { panel.beginSheetModalForWindow_completionHandler(&window, &handler) };
                PRESENT_PANELS.with(|m| m.borrow_mut().insert(req, panel));
                return;
            }
            _ => {}
        }
        let alert = unsafe { objc2_app_kit::NSAlert::new(mtm) };
        unsafe { alert.setMessageText(&NSString::from_str(spec.title())) };
        if let Some(msg) = spec.message() {
            unsafe { alert.setInformativeText(&NSString::from_str(msg)) };
        }
        // The completion handler must outlive this call, so it's a heap (Rc) block.
        let handler: block2::RcBlock<dyn Fn(isize)> = match spec {
            PresentSpec::Dialog { buttons, .. } => {
                if buttons
                    .iter()
                    .any(|b| b.role == present::ButtonRole::Destructive)
                {
                    unsafe { alert.setAlertStyle(objc2_app_kit::NSAlertStyle::Warning) };
                }
                for b in buttons {
                    unsafe { alert.addButtonWithTitle(&NSString::from_str(&b.label)) };
                }
                block2::RcBlock::new(move |resp: isize| {
                    // NSAlertFirstButtonReturn = 1000; add order == spec order.
                    let idx = resp - 1000;
                    emit(
                        WINDOW_NODE,
                        Event::PresentResult {
                            req,
                            result: present::PresentResult::Button(idx as i64),
                        },
                    );
                    PRESENT_ALERTS.with(|m| {
                        m.borrow_mut().remove(&req);
                    });
                })
            }
            PresentSpec::Prompt {
                placeholder,
                initial,
                ok,
                cancel,
                ..
            } => {
                let tf = unsafe { NSTextField::new(mtm) };
                unsafe {
                    tf.setFrame(NSRect::new(
                        NSPoint::new(0.0, 0.0),
                        NSSize::new(260.0, 24.0),
                    ));
                    tf.setEditable(true);
                    tf.setBezeled(true);
                    tf.setStringValue(&NSString::from_str(initial));
                    tf.setPlaceholderString(Some(&NSString::from_str(placeholder)));
                    alert.setAccessoryView(Some(&tf));
                    alert.addButtonWithTitle(&NSString::from_str(ok)); // resp 1000
                    alert.addButtonWithTitle(&NSString::from_str(cancel)); // resp 1001
                }
                block2::RcBlock::new(move |resp: isize| {
                    let result = if resp == 1000 {
                        present::PresentResult::Text(unsafe { tf.stringValue() }.to_string())
                    } else {
                        present::PresentResult::Dismissed
                    };
                    emit(WINDOW_NODE, Event::PresentResult { req, result });
                    PRESENT_ALERTS.with(|m| {
                        m.borrow_mut().remove(&req);
                    });
                })
            }
            // File pickers returned early above.
            PresentSpec::OpenFile { .. } | PresentSpec::SaveFile { .. } => unreachable!(),
        };
        unsafe {
            alert.beginSheetModalForWindow_completionHandler(&window, Some(&handler));
        }
        PRESENT_ALERTS.with(|m| m.borrow_mut().insert(req, alert));
    }

    fn dismiss(&mut self, req: u64) {
        // Close the sheet; its completion handler fires but its (native) result is dropped
        // because day-core already removed the pending request when it resolved.
        let alert = PRESENT_ALERTS.with(|m| m.borrow_mut().remove(&req));
        if let (Some(alert), Some(window)) = (alert, self.window.clone()) {
            unsafe { window.endSheet(&alert.window()) };
        }
        // File-picker sheets are their own NSWindow.
        let panel = PRESENT_PANELS.with(|m| m.borrow_mut().remove(&req));
        if let (Some(panel), Some(window)) = (panel, self.window.clone()) {
            unsafe { window.endSheet(&panel) };
        }
    }

    fn open_url(&mut self, url: &str) {
        // NSWorkspace opens the URL in the user's default handler (Safari for http(s), Mail for
        // mailto:, …). An unparseable string yields no NSURL and is silently ignored.
        let nsurl = unsafe { objc2_foundation::NSURL::URLWithString(&NSString::from_str(url)) };
        if let Some(nsurl) = nsurl {
            unsafe { objc2_app_kit::NSWorkspace::sharedWorkspace().openURL(&nsurl) };
        }
    }
}

thread_local! {
    /// Live modal sheets keyed by request id (for programmatic dismissal).
    static PRESENT_ALERTS: RefCell<HashMap<u64, Retained<objc2_app_kit::NSAlert>>> =
        RefCell::new(HashMap::new());
    /// Live file-picker sheets (NSOpenPanel is stored via its NSSavePanel supertype).
    static PRESENT_PANELS: RefCell<HashMap<u64, Retained<objc2_app_kit::NSSavePanel>>> =
        RefCell::new(HashMap::new());
}

/// Apply a file dialog's extension filters (`allowedFileTypes` — deprecated but still the simplest
/// extension-based API; `UTType` would pull in another framework crate for no benefit here).
#[allow(deprecated)]
fn apply_allowed_file_types(
    panel: &objc2_app_kit::NSSavePanel,
    filters: &[day_spec::present::FileFilter],
) {
    let exts: Vec<Retained<NSString>> = filters
        .iter()
        .flat_map(|f| f.extensions.iter())
        .map(|e| NSString::from_str(e))
        .collect();
    if !exts.is_empty() {
        let refs: Vec<&NSString> = exts.iter().map(|r| &**r).collect();
        let arr = objc2_foundation::NSArray::from_slice(&refs);
        unsafe { panel.setAllowedFileTypes(Some(&arr)) };
    }
}

/// Turn an open/save panel completion into a `PresentResult` and enqueue it (NSModalResponseOK = 1).
fn emit_panel_result(req: u64, resp: isize, panel: &objc2_app_kit::NSSavePanel) {
    let result = if resp == 1 {
        unsafe { panel.URL() }
            .and_then(|url| unsafe { url.path() })
            .map(|p| present::PresentResult::Files(vec![p.to_string()]))
            .unwrap_or(present::PresentResult::Dismissed)
    } else {
        present::PresentResult::Dismissed
    };
    emit(WINDOW_NODE, Event::PresentResult { req, result });
    PRESENT_PANELS.with(|m| {
        m.borrow_mut().remove(&req);
    });
}

impl Platform for AppKit {
    const TARGET: &'static str = "macos-appkit";
    const TOOLKIT: &'static str = "appkit";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Handle, Size)>) {
        let mtm = self.mtm;
        // The App menu / About use the app's display name. `app_name` overrides the (possibly
        // decorated) window title; setting the process name also makes the standard About panel and
        // the bold App-menu title show it (an unbundled binary otherwise shows the exe name).
        self.app_name = options
            .app_name
            .clone()
            .unwrap_or_else(|| options.title.clone());
        // The bold App-menu title, the About panel and Quit all show the app's NAME. ALWAYS set
        // it, not just when `app_name` was given explicitly — an app that only set `title` was
        // showing its kebab-case cargo target ("day-sheets") where its name belongs.
        //
        // Two sources, because AppKit consults them in order: the main bundle's CFBundleName
        // wins when there is one, and `day launch` runs the binary UNBUNDLED during development,
        // where the name falls back to the executable's file name. Writing the info dictionary
        // covers the bundled case; the process name covers the unbundled one.
        unsafe {
            objc2_foundation::NSProcessInfo::processInfo()
                .setProcessName(&NSString::from_str(&self.app_name));
            let bundle = objc2_foundation::NSBundle::mainBundle();
            let info: Option<Retained<objc2_foundation::NSDictionary>> =
                msg_send![&bundle, infoDictionary];
            if let Some(info) = info {
                let key = NSString::from_str("CFBundleName");
                let value = NSString::from_str(&self.app_name);
                // `infoDictionary` is documented immutable, but the instance backing an
                // unbundled process is a mutable dictionary; guard the send so a genuinely
                // immutable one is left alone instead of raising.
                if info.respondsToSelector(objc2::sel!(setObject:forKey:)) {
                    let _: () = msg_send![&info, setObject: &*value, forKey: &*key];
                }
            }
        }
        // RTL locales (docs/localization): AppleTextDirection flips AppKit's app-wide
        // userInterfaceLayoutDirection (label alignment, slider fill, control mirroring) —
        // read at NSApplication init, so register BEFORE sharedApplication. Registration
        // domain only: volatile, never persisted to the app's defaults.
        if day_core::layout_direction() == day_spec::LayoutDirection::Rtl {
            unsafe {
                use objc2_foundation::{NSDictionary, NSNumber, NSUserDefaults};
                let yes: objc2::rc::Retained<objc2::runtime::AnyObject> = NSNumber::new_bool(true)
                    .into_super()
                    .into_super()
                    .into_super();
                let dict = NSDictionary::from_retained_objects(
                    &[
                        &*NSString::from_str("AppleTextDirection"),
                        &*NSString::from_str("NSForceRightToLeftWritingDirection"),
                    ],
                    &[yes.clone(), yes],
                );
                NSUserDefaults::standardUserDefaults().registerDefaults(&dict);
            }
        }
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        // DAY_THEME=light|dark forces the appearance app-wide (themed CI screenshot runs and
        // local theme checks); unset ⇒ follow the system.
        if let Ok(theme) = std::env::var("DAY_THEME") {
            let name = match theme.as_str() {
                "dark" => Some(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua }),
                "light" => Some(unsafe { objc2_app_kit::NSAppearanceNameAqua }),
                _ => None,
            };
            if let Some(name) = name {
                let appearance = objc2_app_kit::NSAppearance::appearanceNamed(name);
                unsafe { app.setAppearance(appearance.as_deref()) };
            }
        }
        // Dock icon (§18.2): `day launch` points DAY_APP_ICON at the project's macOS icon export;
        // an unbundled binary otherwise shows the generic executable icon in the Dock.
        if let Ok(icon) = std::env::var("DAY_APP_ICON") {
            use objc2::AllocAnyThread as _;
            if let Some(img) = unsafe {
                objc2_app_kit::NSImage::initWithContentsOfFile(
                    objc2_app_kit::NSImage::alloc(),
                    &NSString::from_str(&icon),
                )
            } {
                unsafe { app.setApplicationIconImage(Some(&img)) };
            }
        }

        // Bundled custom fonts (§18.4) must be registered before the first label realizes.
        register_bundled_fonts();

        // Default menu bar (standard app menu + Edit) so ⌘Q / Cut-Copy-Paste work before the app
        // installs its own via `app_menu(...)`.
        install_main_menu(mtm, &app, &self.app_name);
        // App activation / termination → day lifecycle events (docs/lifecycle.md).
        install_lifecycle_observers();
        install_appearance_observer();

        let (window, delegate, content) =
            self.make_window(&options.title, options.size, options.min_size, false, None);
        // Optional window-appearance override (opt-in via env). An app with a fixed light/dark
        // palette sets `DAY_APPEARANCE=light|dark` so native controls (list, fields, editor) match
        // its own colors instead of following the system appearance. Unset = follow the system.
        let appearance_name = match std::env::var("DAY_APPEARANCE").ok().as_deref() {
            Some("light") => Some(unsafe { objc2_app_kit::NSAppearanceNameAqua }),
            Some("dark") => Some(unsafe { objc2_app_kit::NSAppearanceNameDarkAqua }),
            _ => None,
        };
        if let Some(name) = appearance_name
            && let Some(appearance) = unsafe { objc2_app_kit::NSAppearance::appearanceNamed(name) }
        {
            window.setAppearance(Some(&appearance));
        }
        // The primary delegate lives for the process (this frame never returns); secondary
        // delegates are retained in `self.secondary`.
        std::mem::forget(delegate);

        self.window = Some(window.clone());
        self.content = Some(content.clone());
        PRIMARY_CONTENT.with(|c| *c.borrow_mut() = Some(content.clone()));

        ready(
            self,
            content,
            Size::new(options.size.width, options.size.height),
        );

        // `ready` ran the app's root() — registrations are in. Without a new-window
        // builder only one Normal window can ever exist: turn automatic tabbing off so no
        // tab bar, "+" button, or dead tab menu items appear (docs/windows.md).
        if day_core::windows::new_window_action_id() == 0 {
            unsafe { NSWindow::setAllowsAutomaticWindowTabbing(false, mtm) };
        }

        window.center();
        window.makeKeyAndOrderFront(None);
        app.activate();
        // The root was mounted before the window was shown (ready() runs first), and on
        // macOS 26 layer displays requested pre-run are dropped for a window ordered front
        // before the app finishes launching — the window stays blank until the next real
        // event. Re-mark the whole tree so the first run-loop turn commits a full frame.
        if let Some(content) = window.contentView() {
            mark_tree_needs_display(&content);
        }
        if std::env::var_os("DAY_DUMP").is_some() {
            std::thread::spawn(|| {
                std::thread::sleep(std::time::Duration::from_millis(2000));
                Self::post(Box::new(|| {
                    // The primary's content specifically (docs/windows.md).
                    if let Some(content) = primary_content() {
                        let desc: Retained<NSString> =
                            unsafe { msg_send![&*content, _subtreeDescription] };
                        eprintln!("{desc}");
                    }
                }));
            });
        }
        app.run();
    }

    fn post(f: Box<dyn FnOnce() + Send>) {
        dispatch2::DispatchQueue::main().exec_async(f);
    }

    fn locale_hints(&self) -> Vec<String> {
        // M6: NSLocale preferredLanguages.
        Vec::new()
    }
}

/// Capture a window content view as PNG (the dayscript screenshot seam, §14): offscreen
/// `cacheDisplayInRect` over a window-background pre-fill resolved for the view's
/// light/dark appearance.
fn snapshot_view(content: &NSView) -> Result<Vec<u8>, String> {
    let bounds = content.bounds();
    let rep =
        unsafe { content.bitmapImageRepForCachingDisplayInRect(bounds) }.ok_or("no bitmap rep")?;
    // Day containers are transparent (the window server paints the backdrop), so pre-fill
    // the rep with the window background — resolved for the window's light/dark appearance —
    // before compositing the view hierarchy over it (§14).
    let ctx =
        NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep).ok_or("no graphics context")?;
    NSGraphicsContext::saveGraphicsState_class();
    NSGraphicsContext::setCurrentContext(Some(&ctx));
    content
        .effectiveAppearance()
        .performAsCurrentDrawingAppearance(&block2::StackBlock::new(|| unsafe {
            NSColor::windowBackgroundColor().setFill();
            objc2_app_kit::NSRectFill(bounds);
        }));
    NSGraphicsContext::restoreGraphicsState_class();
    unsafe { content.cacheDisplayInRect_toBitmapImageRep(bounds, &rep) };
    let data = unsafe {
        rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &NSDictionary::new())
    }
    .ok_or("png encode failed")?;
    Ok(data.to_vec())
}

/// Recursively mark a view tree as needing display (startup first-frame fix, see `run`).
fn mark_tree_needs_display(view: &NSView) {
    view.setNeedsDisplay(true);
    for sub in view.subviews() {
        mark_tree_needs_display(&sub);
    }
}

/// Which lifecycle phases this desktop backend delivers (docs/lifecycle.md): the universal set.
/// macOS has no true background/foreground or memory-warning lifecycle. `const` so
/// `day::require_lifecycle!` can reject unsupported phases at compile time. Must agree with the
/// `Toolkit::supports_lifecycle` default (which also returns `is_universal`).
pub const fn lifecycle_supported(phase: day_spec::Lifecycle) -> bool {
    phase.is_universal()
}

/// Follow the SYSTEM appearance while the app runs: macOS posts the distributed
/// "AppleInterfaceThemeChangedNotification" on a theme switch, and AppKit updates
/// `effectiveAppearance` just after — so re-read on the next main-queue turn and refresh
/// day-core's reactive dark-mode signal (palette closures recolor live instead of going
/// stale until the next rebuild). The token is leaked to observe for the app's lifetime.
fn install_appearance_observer() {
    use objc2_foundation::{NSDistributedNotificationCenter, NSNotification, NSString};
    let center = unsafe { NSDistributedNotificationCenter::defaultCenter() };
    let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
        dispatch2::DispatchQueue::main().exec_async(day_core::note_appearance_changed);
    });
    let name = NSString::from_str("AppleInterfaceThemeChangedNotification");
    let token = unsafe {
        center.addObserverForName_object_queue_usingBlock(Some(&name), None, None, &block)
    };
    std::mem::forget(token);
}

/// Bridge the NSApplication activation/termination notifications to day lifecycle events
/// (docs/lifecycle.md). The observer tokens are leaked to live for the whole app.
fn install_lifecycle_observers() {
    use objc2_foundation::{NSNotification, NSNotificationCenter, NSNotificationName};
    let center = unsafe { NSNotificationCenter::defaultCenter() };
    let observe = |name: &NSNotificationName, phase: day_spec::Lifecycle| {
        let block = block2::RcBlock::new(move |_: std::ptr::NonNull<NSNotification>| {
            emit(day_spec::WINDOW_NODE, Event::Lifecycle(phase));
        });
        let token = unsafe {
            center.addObserverForName_object_queue_usingBlock(Some(name), None, None, &block)
        };
        std::mem::forget(token); // observe for the app's lifetime
    };
    unsafe {
        observe(
            NSApplicationDidBecomeActiveNotification,
            day_spec::Lifecycle::DidBecomeActive,
        );
        observe(
            NSApplicationWillResignActiveNotification,
            day_spec::Lifecycle::WillResignActive,
        );
        observe(
            NSApplicationWillTerminateNotification,
            day_spec::Lifecycle::WillTerminate,
        );
    }
}

/// Localized "About <App>" / "Quit <App>" for the standard App menu, with correct per-language word
/// order via the core catalog's `{$app}` interpolation (docs/localization.md).
fn about_label(app: &str) -> String {
    day_l10n::format_in(
        &day_l10n::locale().get(),
        "day-about-app",
        &[("app".to_string(), day_l10n::FArg::Str(app.to_string()))],
    )
}
fn quit_label(app: &str) -> String {
    day_l10n::format_in(
        &day_l10n::locale().get(),
        "day-quit-app",
        &[("app".to_string(), day_l10n::FArg::Str(app.to_string()))],
    )
}

/// The default main menu (§21.2 M2): App menu with Quit; Edit menu wired to the responder
/// chain so Cmd+C/V/X/A work in NSTextFields; Window menu basics.
fn install_main_menu(mtm: MainThreadMarker, app: &NSApplication, title: &str) {
    let menubar = NSMenu::new(mtm);

    let app_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);
    // Settings…/⌘, when the app registered a preferences piece (docs/windows.md) — this is
    // the no-`app_menu` path, so apps get the standard item with zero menu code.
    let prefs_id = day_core::windows::preferences_action_id();
    if prefs_id != 0 {
        let settings = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&format!("{}…", day_l10n::t("day-preferences"))),
                Some(sel!(fire:)),
                &NSString::from_str(","),
            )
        };
        let target = menu_target(mtm);
        let tobj: &objc2::runtime::AnyObject = target.as_ref();
        unsafe { settings.setTarget(Some(tobj)) };
        settings.setTag(prefs_id as isize);
        app_menu.addItem(&settings);
        app_menu.addItem(&NSMenuItem::separatorItem(mtm));
    }
    let quit = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(&quit_label(title)),
            Some(sel!(terminate:)),
            &NSString::from_str("q"),
        )
    };
    app_menu.addItem(&quit);
    app_item.setSubmenu(Some(&app_menu));
    menubar.addItem(&app_item);

    let edit_item = NSMenuItem::new(mtm);
    let edit_menu = unsafe {
        NSMenu::initWithTitle(
            NSMenu::alloc(mtm),
            &NSString::from_str(&day_l10n::t("day-edit")),
        )
    };
    let add = |key: &str, action: objc2::runtime::Sel, accel: &str| {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&day_l10n::t(key)),
                Some(action),
                &NSString::from_str(accel),
            )
        };
        edit_menu.addItem(&item);
    };
    add("day-undo", sel!(undo:), "z");
    add("day-redo", sel!(redo:), "Z");
    edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add("day-cut", sel!(cut:), "x");
    add("day-copy", sel!(copy:), "c");
    add("day-paste", sel!(paste:), "v");
    add("day-select-all", sel!(selectAll:), "a");
    edit_item.setSubmenu(Some(&edit_menu));
    menubar.addItem(&edit_item);

    install_windows_menu(mtm, app, &menubar);

    app.setMainMenu(Some(&menubar));
}

/// The standard Window menu (docs/windows.md): Minimize ⌘M / Zoom / Bring All to Front,
/// registered as `NSApp.windowsMenu` — AppKit then appends the open-window list and, with
/// tabbing live, the tab commands (Show Next/Previous Tab, Merge All Windows) itself. All
/// selectors are nil-targeted (responder chain), so they act on the key window.
fn install_windows_menu(mtm: MainThreadMarker, app: &NSApplication, menubar: &NSMenu) {
    let menu = unsafe {
        NSMenu::initWithTitle(
            NSMenu::alloc(mtm),
            &NSString::from_str(&day_l10n::t("day-window")),
        )
    };
    let add = |key: &str, action: objc2::runtime::Sel, accel: &str| {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(&day_l10n::t(key)),
                Some(action),
                &NSString::from_str(accel),
            )
        };
        menu.addItem(&item);
    };
    add("day-minimize", sel!(performMiniaturize:), "m");
    add("day-zoom", sel!(performZoom:), "");
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add("day-bring-all-front", sel!(arrangeInFront:), "");
    let item = NSMenuItem::new(mtm);
    item.setTitle(&NSString::from_str(&day_l10n::t("day-window")));
    item.setSubmenu(Some(&menu));
    menubar.addItem(&item);
    unsafe { app.setWindowsMenu(Some(&menu)) };
}

/// Remove and return the first `role(Preferences)` action from the model (searching
/// top-level submenus), trimming a separator left dangling at the submenu tail.
fn extract_preferences(items: &mut [day_spec::MenuItem]) -> Option<day_spec::MenuItem> {
    for it in items.iter_mut() {
        if let day_spec::MenuItem::Submenu { items, .. } = it
            && let Some(i) = items.iter().position(|m| {
                matches!(
                    m,
                    day_spec::MenuItem::Action {
                        role: Some(day_spec::MenuRole::Preferences),
                        ..
                    }
                )
            })
        {
            let item = items.remove(i);
            while matches!(items.last(), Some(day_spec::MenuItem::Separator)) {
                items.pop();
            }
            return Some(item);
        }
    }
    None
}

/// Whether any action in the model (recursively) carries `role`.
fn model_has_role(items: &[day_spec::MenuItem], role: day_spec::MenuRole) -> bool {
    items.iter().any(|it| match it {
        day_spec::MenuItem::Action { role: r, .. } => *r == Some(role),
        day_spec::MenuItem::Submenu { items, .. } => model_has_role(items, role),
        day_spec::MenuItem::Separator => false,
    })
}

// ---------------------------------------------------------------------------
// Menus (§ menus): render day's MenuItem model with NSMenu. Custom items route to a shared
// DayMenuTarget (id in the item's tag → Event::MenuAction); role items use the native selector.
// ---------------------------------------------------------------------------

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayMenuTarget"]
    #[ivars = ()]
    struct DayMenuTarget;

    unsafe impl NSObjectProtocol for DayMenuTarget {}

    impl DayMenuTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &NSMenuItem) {
            let id = sender.tag() as u64;
            if id != 0 {
                emit(day_spec::WINDOW_NODE, Event::MenuAction(id));
            }
        }
    }
);

thread_local! {
    // NSMenuItem does NOT retain its target — keep one shared target alive for the app's lifetime.
    static MENU_TARGET: std::cell::RefCell<Option<Retained<DayMenuTarget>>> =
        const { std::cell::RefCell::new(None) };
    /// The PRIMARY window's content view, for call sites without backend access that must
    /// address the main window specifically (cover re-homing, DAY_DUMP). With secondary
    /// windows open, `app.windows().firstObject()` is arbitrary (docs/windows.md).
    static PRIMARY_CONTENT: std::cell::RefCell<Option<Retained<NSView>>> =
        const { std::cell::RefCell::new(None) };
}

/// The primary window's content view (set once in `run`).
fn primary_content() -> Option<Retained<NSView>> {
    PRIMARY_CONTENT.with(|c| c.borrow().clone())
}

fn menu_target(mtm: MainThreadMarker) -> Retained<DayMenuTarget> {
    MENU_TARGET.with(|t| {
        t.borrow_mut()
            .get_or_insert_with(|| {
                let this = DayMenuTarget::alloc(mtm).set_ivars(());
                let obj: Retained<DayMenuTarget> = unsafe { msg_send![super(this), init] };
                obj
            })
            .clone()
    })
}

fn ns_modifiers(s: &day_spec::Shortcut) -> objc2_app_kit::NSEventModifierFlags {
    use objc2_app_kit::NSEventModifierFlags as F;
    let mut m = F::empty();
    if s.primary {
        m |= F::Command;
    }
    if s.shift {
        m |= F::Shift;
    }
    if s.alt {
        m |= F::Option;
    }
    if s.control {
        m |= F::Control;
    }
    m
}

/// A shortcut's key-equivalent string. Single chars pass through (lowercased); a few named keys map
/// to their control characters. Modifiers ride separately via `setKeyEquivalentModifierMask`.
fn ns_key_equivalent(key: &str) -> String {
    match key {
        "Return" | "Enter" => "\r".into(),
        "Tab" => "\t".into(),
        "Delete" | "Backspace" => "\u{8}".into(),
        "Escape" => "\u{1b}".into(),
        "Space" => " ".into(),
        k if k.chars().count() == 1 => k.to_lowercase(),
        _ => String::new(), // named keys we don't map get no key-equivalent (still shown in menu)
    }
}

/// A standard role → (default label, selector, default shortcut). Selector `None` = no native action
/// (the app should attach its own via a custom item); the role then only supplies label placement.
fn role_spec(
    role: day_spec::MenuRole,
) -> (
    &'static str,
    Option<objc2::runtime::Sel>,
    Option<day_spec::Shortcut>,
) {
    use day_spec::MenuRole as R;
    use day_spec::Shortcut as S;
    match role {
        R::Cut => ("Cut", Some(sel!(cut:)), Some(S::new("x"))),
        R::Copy => ("Copy", Some(sel!(copy:)), Some(S::new("c"))),
        R::Paste => ("Paste", Some(sel!(paste:)), Some(S::new("v"))),
        R::SelectAll => ("Select All", Some(sel!(selectAll:)), Some(S::new("a"))),
        R::Undo => ("Undo", Some(sel!(undo:)), Some(S::new("z"))),
        R::Redo => ("Redo", Some(sel!(redo:)), Some(S::new("z").shift())),
        R::Delete => ("Delete", Some(sel!(delete:)), None),
        R::About => ("About", Some(sel!(orderFrontStandardAboutPanel:)), None),
        R::Quit => ("Quit", Some(sel!(terminate:)), Some(S::new("q"))),
        R::Preferences => ("Settings…", None, Some(S::new(","))),
        R::Minimize => (
            "Minimize",
            Some(sel!(performMiniaturize:)),
            Some(S::new("m")),
        ),
        R::CloseWindow => ("Close", Some(sel!(performClose:)), Some(S::new("w"))),
        R::Fullscreen => (
            "Enter Full Screen",
            Some(sel!(toggleFullScreen:)),
            Some(S::new("f").control()),
        ),
        // No native selector (docs/windows.md): the item dispatches the registered
        // new-window action through the ordinary custom-item path (`fire:`).
        R::NewWindow => ("New Window", None, Some(S::new("n"))),
    }
}

pub(crate) fn build_ns_menu(
    mtm: MainThreadMarker,
    title: &str,
    items: &[day_spec::MenuItem],
) -> Retained<NSMenu> {
    use day_spec::MenuItem as MI;
    let menu = unsafe { NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title)) };
    let target = menu_target(mtm);
    for item in items {
        match item {
            MI::Separator => menu.addItem(&NSMenuItem::separatorItem(mtm)),
            MI::Submenu { label, items, .. } => {
                let sub = build_ns_menu(mtm, label, items);
                let it = NSMenuItem::new(mtm);
                it.setTitle(&NSString::from_str(label));
                it.setSubmenu(Some(&sub));
                menu.addItem(&it);
            }
            MI::Action {
                id,
                label,
                shortcut,
                enabled,
                role,
            } => {
                // Resolve label/selector/shortcut, folding in the role's native defaults.
                let (mut lbl, sel, mut sc) = match role {
                    Some(r) => {
                        let (dl, ds, dsc) = role_spec(*r);
                        (dl.to_string(), ds, dsc)
                    }
                    None => (String::new(), None, None),
                };
                if !label.is_empty() {
                    lbl = label.clone();
                }
                if shortcut.is_some() {
                    sc = shortcut.clone();
                }
                // Custom action (nonzero id) overrides any role selector and targets our trampoline.
                let custom = *id != 0;
                let key = sc
                    .as_ref()
                    .map(|s| ns_key_equivalent(&s.key))
                    .unwrap_or_default();
                let it = unsafe {
                    NSMenuItem::initWithTitle_action_keyEquivalent(
                        NSMenuItem::alloc(mtm),
                        &NSString::from_str(&lbl),
                        if custom { Some(sel!(fire:)) } else { sel },
                        &NSString::from_str(&key),
                    )
                };
                if let Some(s) = &sc {
                    it.setKeyEquivalentModifierMask(ns_modifiers(s));
                }
                if custom {
                    let tobj: &objc2::runtime::AnyObject = target.as_ref();
                    unsafe { it.setTarget(Some(tobj)) };
                    it.setTag(*id as isize);
                }
                it.setEnabled(*enabled);
                menu.addItem(&it);
            }
        }
    }
    menu
}
