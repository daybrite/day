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
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSAccessibility as _;
use objc2_app_kit::NSAppearanceCustomization as _;
use objc2_app_kit::NSUserInterfaceItemIdentification as _;
use objc2_app_kit::{
    NSAffineTransformNSAppKitAdditions, NSClickGestureRecognizer, NSGestureRecognizer,
    NSGestureRecognizerState, NSPanGestureRecognizer,
};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageFileType, NSBox,
    NSBoxType, NSButton, NSColor, NSControl, NSControlTextEditingDelegate, NSFont,
    NSGraphicsContext, NSLineBreakMode, NSMenu, NSMenuItem, NSProgressIndicator,
    NSProgressIndicatorStyle, NSScrollView, NSSlider, NSSwitch, NSTabView, NSTabViewItem,
    NSTextField, NSTextFieldDelegate, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_app_kit::{
    NSApplicationDidBecomeActiveNotification, NSApplicationWillResignActiveNotification,
    NSApplicationWillTerminateNotification,
};
use objc2_app_kit::{NSOutlineViewDataSource, NSOutlineViewDelegate, NSTabViewDelegate};
use objc2_app_kit::{NSTableColumn, NSTableView, NSTableViewDataSource, NSTableViewDelegate};
use objc2_foundation::{
    NSAffineTransform, NSAffineTransformStruct, NSDictionary, NSNotification, NSObject, NSPoint,
    NSRect, NSSize, NSString,
};

use day_spec::present;
use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, ListSource, NodeId, PieceKind,
    Platform, Point, Proposal, RawHandle, Rect, Registry, Renderer, Size, Support, Toolkit,
    WINDOW_NODE, WindowOptions, kinds,
};

pub type Handle = Retained<NSView>;

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
                emit(node, Event::ValueChanged(unsafe { sender.doubleValue() }));
            } else {
                emit(node, Event::Pressed);
            }
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
    }
);

impl DayTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
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

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _dirty: NSRect) {
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

struct NavState {
    sidebar_wrap: Retained<NSView>,
    detail_wrap: Retained<NSView>,
    /// Detail pages in stack order (the sidebar page is not in here in split mode; in stack
    /// mode `split == false`, the root page is here too, so push/pop visibility covers it).
    pages: Vec<Retained<NSView>>,
    positioned: bool,
    /// Sidebar+detail split (a selector Sidebar) vs. a pure push/pop stack (a `stack`).
    split: bool,
}

thread_local! {
    static NAV_STATE: RefCell<HashMap<usize, NavState>> = RefCell::new(HashMap::new());
    /// Handles whose frames are native-owned (nav pages): set_frame skips them.
    static NAV_PAGES: RefCell<std::collections::HashSet<usize>> =
        RefCell::new(std::collections::HashSet::new());
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
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: std::cell::Cell<bool>,
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
                self.ivars().items.borrow().len() as isize
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
            let items = self.ivars().items.borrow();
            let ns = items[index as usize].clone();
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
        #[unsafe(method_id(outlineView:viewForTableColumn:item:))]
        fn view_for(
            &self,
            _ov: &objc2_app_kit::NSOutlineView,
            _col: Option<&objc2_app_kit::NSTableColumn>,
            item: &objc2::runtime::AnyObject,
        ) -> Option<Retained<NSView>> {
            let mtm = self.mtm();
            // No early returns: the method_id macro owns the return conversion.
            item.downcast_ref::<NSString>().map(|text| {
                let cell = unsafe { objc2_app_kit::NSTableCellView::new(mtm) };
                let label = unsafe { NSTextField::labelWithString(text, mtm) };
                unsafe {
                    label.setFrame(NSRect::new(NSPoint::new(0.0, 3.0), NSSize::new(10.0, 17.0)));
                    label.setAutoresizingMask(
                        objc2_app_kit::NSAutoresizingMaskOptions::ViewWidthSizable,
                    );
                    cell.addSubview(&label);
                    cell.setTextField(Some(&label));
                }
                objc2::rc::Retained::into_super(cell)
            })
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
            if row >= 0 {
                emit(self.ivars().node, Event::SelectionChanged(row as i64));
            }
        }
    }
);

impl DayNavMenuData {
    fn new(mtm: MainThreadMarker, node: NodeId, items: &[String]) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(NavMenuIvars {
            node,
            items: RefCell::new(items.iter().map(|s| NSString::from_str(s)).collect()),
            suppress: std::cell::Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// DayListData — NSTableView data-source + delegate for the recycling list (docs/list.md, §10)
// ---------------------------------------------------------------------------

struct ListIvars {
    node: NodeId,
    /// Injected by `attach_list` once day-core wires the driver.
    source: RefCell<Option<ListSource>>,
    selectable: std::cell::Cell<bool>,
    /// Programmatic selection in flight: don't re-emit SelectionChanged.
    suppress: std::cell::Cell<bool>,
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
            let row = unsafe { tv.selectedRow() };
            if row >= 0 {
                emit(self.ivars().node, Event::SelectionChanged(row as i64));
            }
        }
    }
);

impl DayListData {
    fn new(mtm: MainThreadMarker, node: NodeId, selectable: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ListIvars {
            node,
            source: RefCell::new(None),
            selectable: std::cell::Cell::new(selectable),
            suppress: std::cell::Cell::new(false),
        });
        unsafe { msg_send![super(this), init] }
    }
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
            DrawOp::Fill(shape, color) => {
                nscolor(*color).setFill();
                if let Some(p) = bezier(shape) {
                    p.fill();
                }
            }
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
// DayWinDelegate — resize + close
// ---------------------------------------------------------------------------

struct WinIvars;

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
                emit(
                    WINDOW_NODE,
                    Event::WindowResized(Size::new(b.size.width, b.size.height)),
                );
            }
        }

        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            let app = NSApplication::sharedApplication(self.mtm());
            unsafe { app.terminate(None) };
        }
    }
);

impl DayWinDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(WinIvars);
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// The backend
// ---------------------------------------------------------------------------

/// Renderers registered by external Day Piece crates (§8.2 layer 3 — linkme convenience).
#[distributed_slice]
pub static RENDERERS: [fn() -> Renderer<AppKit>];

pub struct AppKit {
    mtm: MainThreadMarker,
    registry: Registry<AppKit>,
    window: Option<Retained<NSWindow>>,
    content: Option<Handle>,
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
            app_name: "Day".into(),
        }
    }

    /// Public helper for external renderers.
    pub fn mtm(&self) -> MainThreadMarker {
        self.mtm
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
            Font::System(_) => return None,
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

/// Warn ONCE per kind that this backend has no registered renderer for `kind`, before falling back to
/// a visible placeholder. A missing renderer usually means the piece's `appkit` feature wasn't enabled
/// (Tier A.2 derives it automatically under `day build`; a bare `cargo` build may miss it). Deduped
/// per kind so a placeholder rendered every frame doesn't spam the log.
fn warn_missing_renderer(kind: PieceKind) {
    static SEEN: std::sync::Mutex<Option<std::collections::HashSet<&'static str>>> =
        std::sync::Mutex::new(None);
    let Ok(mut guard) = SEEN.lock() else { return };
    if guard
        .get_or_insert_with(std::collections::HashSet::new)
        .insert(kind)
    {
        eprintln!(
            "day: no renderer for piece kind \"{kind}\" on appkit \
             — is the piece's appkit feature enabled? (rendering a placeholder)"
        );
    }
}

impl Toolkit for AppKit {
    type Handle = Handle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot
            | Cap::NativeSymbols
            | Cap::NavSplit
            | Cap::Dialogs
            | Cap::FileDialogs => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Handle {
        let mtm = self.mtm;
        match kind {
            kinds::CONTAINER => {
                let v = DayFlipped::new(mtm);
                if let Some(p) = props.downcast_ref::<ContainerProps>()
                    && (p.background.is_some() || p.corner_radius > 0.0 || p.clips)
                {
                    v.set_surface(p.background, p.corner_radius, p.clips);
                }
                view_of(v)
            }
            kinds::SCROLL => {
                let sv = unsafe { NSScrollView::new(mtm) };
                unsafe {
                    sv.setHasVerticalScroller(true);
                    sv.setDrawsBackground(false);
                }
                let doc = DayFlipped::new(mtm);
                unsafe { sv.setDocumentView(Some(doc.as_ref())) };
                view_of(sv)
            }
            kinds::LABEL => {
                let p = props.downcast_ref::<LabelProps>().unwrap();
                let tf = unsafe { NSTextField::labelWithString(&NSString::from_str(&p.text), mtm) };
                configure_label_cell(&tf);
                unsafe { tf.setFont(Some(&nsfont(p.font))) };
                if let Some(c) = p.color {
                    unsafe { tf.setTextColor(Some(&nscolor(c))) };
                }
                view_of(tf)
            }
            kinds::BUTTON => {
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
                let view = view_of(btn);
                TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                view
            }
            kinds::TOGGLE => {
                let p = props.downcast_ref::<ToggleProps>().unwrap();
                let target = DayTarget::new(mtm, id);
                let sw = unsafe { NSSwitch::new(mtm) };
                unsafe {
                    sw.setState(if p.on { 1 } else { 0 });
                    sw.setTarget(Some(&*target));
                    sw.setAction(Some(sel!(action:)));
                }
                let view = view_of(sw);
                TARGETS.with(|m| m.borrow_mut().insert(ptr_of(&view), target));
                view
            }
            kinds::SLIDER => {
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
            kinds::TEXT_FIELD => {
                let p = props.downcast_ref::<TextFieldProps>().unwrap();
                let target = DayTarget::new(mtm, id);
                let tf = unsafe { NSTextField::new(mtm) };
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
            kinds::DIVIDER => {
                let b = unsafe { NSBox::new(mtm) };
                unsafe { b.setBoxType(NSBoxType::Separator) };
                view_of(b)
            }
            kinds::PROGRESS => {
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
            kinds::CANVAS => view_of(DayCanvas::new(mtm)),
            kinds::NAV => {
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
                unsafe {
                    split.addArrangedSubview(&effect);
                    split.addArrangedSubview(&detail_wrap);
                    // (Holding priorities are a no-op when Day drives the split's frame
                    // directly — the sidebar-holds-width behaviour lives in `set_frame`,
                    // which restores the divider position after each window resize.)
                    split.setHoldingPriority_forSubviewAtIndex(260.0, 0);
                    split.setHoldingPriority_forSubviewAtIndex(250.0, 1);
                }
                let view = view_of(split);
                NAV_STATE.with(|m| {
                    m.borrow_mut().insert(
                        ptr_of(&view),
                        NavState {
                            sidebar_wrap,
                            detail_wrap,
                            pages: Vec::new(),
                            positioned: false,
                            split: is_split,
                        },
                    )
                });
                view
            }
            kinds::NAV_PAGE => {
                let page = view_of(DayNavPage::new(mtm, id));
                NAV_PAGES.with(|set| set.borrow_mut().insert(ptr_of(&page)));
                page
            }
            kinds::TABS => {
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
            kinds::TABS_PAGE => {
                let p = props.downcast_ref::<TabsPageProps>().unwrap();
                // A tab page is a native-owned content view (like a nav page: reports its
                // allocated size via FrameChanged), tagged so set_frame skips it.
                let page = view_of(DayNavPage::new(mtm, id));
                NAV_PAGES.with(|set| set.borrow_mut().insert(ptr_of(&page)));
                TAB_TITLES.with(|m| m.borrow_mut().insert(ptr_of(&page), p.title.clone()));
                page
            }
            kinds::NAV_MENU => {
                let p = props.downcast_ref::<NavMenuProps>().unwrap();
                let data = DayNavMenuData::new(mtm, id, &p.items);
                let outline = unsafe { objc2_app_kit::NSOutlineView::new(mtm) };
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
                    outline.setStyle(objc2_app_kit::NSTableViewStyle::SourceList);
                    outline.setIndentationPerLevel(0.0);
                    outline.setDataSource(Some(ProtocolObject::from_ref(&*data)));
                    outline.setDelegate(Some(ProtocolObject::from_ref(&*data)));
                    outline.reloadData();
                }
                let scroll = unsafe { NSScrollView::new(mtm) };
                unsafe {
                    scroll.setDrawsBackground(false);
                    scroll.setHasVerticalScroller(true);
                    scroll.setDocumentView(Some(&outline));
                }
                let view = view_of(scroll);
                if let Some(sel) = p.selected {
                    data.ivars().suppress.set(true);
                    unsafe {
                        outline.selectRowIndexes_byExtendingSelection(
                            &objc2_foundation::NSIndexSet::indexSetWithIndex(sel),
                            false,
                        )
                    };
                    data.ivars().suppress.set(false);
                }
                NAV_MENUS.with(|m| m.borrow_mut().insert(ptr_of(&view), (outline, data)));
                view
            }
            kinds::LIST => {
                let p = props.downcast_ref::<ListProps>().unwrap();
                let table = unsafe { NSTableView::new(mtm) };
                let col = unsafe {
                    NSTableColumn::initWithIdentifier(
                        NSTableColumn::alloc(mtm),
                        &NSString::from_str("day.list.col"),
                    )
                };
                let data = DayListData::new(mtm, id, p.selectable);
                unsafe {
                    table.addTableColumn(&col);
                    table.setHeaderView(None);
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
                    table.setDataSource(Some(ProtocolObject::from_ref(&*data)));
                    table.setDelegate(Some(ProtocolObject::from_ref(&*data)));
                }
                let scroll = unsafe { NSScrollView::new(mtm) };
                unsafe {
                    scroll.setDrawsBackground(false);
                    scroll.setHasVerticalScroller(true);
                    scroll.setDocumentView(Some(&table));
                }
                let view = view_of(scroll);
                LIST_STATE.with(|m| m.borrow_mut().insert(ptr_of(&view), (table, data)));
                view
            }
            kinds::IMAGE => {
                let p = props.downcast_ref::<ImageProps>().unwrap();
                let iv = unsafe { objc2_app_kit::NSImageView::new(mtm) };
                if let Some(path) = resolve_asset(&p.source) {
                    use objc2::AllocAnyThread as _;
                    if let Some(img) = unsafe {
                        objc2_app_kit::NSImage::initWithContentsOfFile(
                            objc2_app_kit::NSImage::alloc(),
                            &NSString::from_str(&path),
                        )
                    } {
                        unsafe { iv.setImage(Some(&img)) };
                    }
                }
                view_of(iv)
            }
            _ => {
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
                if let Some(TabsPatch::Selected(i)) = patch.downcast_ref::<TabsPatch>()
                    && let Some(tabview) = h.downcast_ref::<NSTabView>()
                {
                    TAB_STATE.with(|m| {
                        if let Some(state) = m.borrow().get(&ptr_of(h)) {
                            state.delegate.ivars().suppress.set(true);
                            unsafe { tabview.selectTabViewItemAtIndex(*i as isize) };
                            state.delegate.ivars().suppress.set(false);
                        }
                    });
                }
            }
            kinds::NAV_MENU => {
                if let Some(NavMenuPatch::Selected(sel)) = patch.downcast_ref::<NavMenuPatch>() {
                    NAV_MENUS.with(|m| {
                        let m = m.borrow();
                        let Some((outline, data)) = m.get(&ptr_of(h)) else {
                            return;
                        };
                        data.ivars().suppress.set(true);
                        unsafe {
                            match sel {
                                Some(i) => outline.selectRowIndexes_byExtendingSelection(
                                    &objc2_foundation::NSIndexSet::indexSetWithIndex(*i),
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
                            NavPatch::Pushed { .. } => {
                                // Only the new top detail page stays visible.
                                let last = state.pages.len().saturating_sub(1);
                                for (i, page) in state.pages.iter().enumerate() {
                                    page.setHidden(i != last);
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
                            }
                            NavPatch::Title(_) => {}
                        }
                    });
                }
            }
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
                        if let Some((table, _)) = m.borrow().get(&ptr_of(h)) {
                            // reloadData queries numberOfRows synchronously (snapshot only, no
                            // tree) and defers viewForRow, so this is safe inside with_tree.
                            unsafe { table.reloadData() };
                        }
                    });
                }
                Some(ListPatch::ScrollToEnd) => {
                    LIST_STATE.with(|m| {
                        if let Some((table, _)) = m.borrow().get(&ptr_of(h)) {
                            let rows = unsafe { table.numberOfRows() };
                            if rows > 0 {
                                unsafe { table.scrollRowToVisible(rows - 1) };
                            }
                        }
                    });
                }
                _ => {}
            },
            _ => {
                if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                    update(self, h, patch);
                }
            }
        }
    }

    fn release(&mut self, h: Handle) {
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
            let wrap = if state.split && index == 0 {
                &state.sidebar_wrap
            } else {
                state.pages.push(child.clone());
                &state.detail_wrap
            };
            unsafe {
                child.setFrame(wrap.bounds());
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

    fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
        let mtm = self.mtm;
        let app = NSApplication::sharedApplication(mtm);
        let menubar = NSMenu::new(mtm);
        // macOS mandates a leading app menu (shown as the app name); provide the standard one so the
        // app's `app_menu(...)` supplies only the rest (File/Edit/View/…), staying convention-native.
        let app_item = NSMenuItem::new(mtm);
        let app_menu = build_ns_menu(
            mtm,
            &self.app_name,
            &[
                day_spec::MenuItem::Action {
                    id: 0,
                    label: about_label(&self.app_name),
                    shortcut: None,
                    enabled: true,
                    role: Some(day_spec::MenuRole::About),
                },
                day_spec::MenuItem::Separator,
                day_spec::MenuItem::Action {
                    id: 0,
                    label: quit_label(&self.app_name),
                    shortcut: None,
                    enabled: true,
                    role: Some(day_spec::MenuRole::Quit),
                },
            ],
        );
        app_item.setSubmenu(Some(&app_menu));
        menubar.addItem(&app_item);
        // Each top-level entry becomes a menu-bar menu.
        for item in items {
            match item {
                day_spec::MenuItem::Submenu { label, items } => {
                    let sub = build_ns_menu(mtm, label, items);
                    let it = NSMenuItem::new(mtm);
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
        // NSView (via NSResponder) shows this on right-click automatically; setMenu retains it.
        unsafe { view.setMenu(Some(&menu)) };
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
        let bounds = content.bounds();
        let rep = unsafe { content.bitmapImageRepForCachingDisplayInRect(bounds) }
            .ok_or("no bitmap rep")?;
        // Day containers are transparent (the window server paints the backdrop), so pre-fill
        // the rep with the window background — resolved for the window's light/dark appearance —
        // before compositing the view hierarchy over it (§14).
        let ctx = NSGraphicsContext::graphicsContextWithBitmapImageRep(&rep)
            .ok_or("no graphics context")?;
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

    fn present(&mut self, req: u64, spec: &present::PresentSpec) {
        use present::PresentSpec;
        let mtm = self.mtm;
        let Some(window) = self.window.clone() else {
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
        if let Some(name) = &options.app_name {
            unsafe {
                objc2_foundation::NSProcessInfo::processInfo()
                    .setProcessName(&NSString::from_str(name));
            }
        }
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

        // Default menu bar (standard app menu + Edit) so ⌘Q / Cut-Copy-Paste work before the app
        // installs its own via `app_menu(...)`.
        install_main_menu(mtm, &app, &self.app_name);
        // App activation / termination → day lifecycle events (docs/lifecycle.md).
        install_lifecycle_observers();

        let content_rect = NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(options.size.width, options.size.height),
        );
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                content_rect,
                style,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        window.setTitle(&NSString::from_str(&options.title));
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
        if let Some(min) = options.min_size {
            unsafe { window.setContentMinSize(NSSize::new(min.width, min.height)) };
        }
        let delegate = DayWinDelegate::new(mtm);
        window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

        let content = view_of(DayFlipped::new(mtm));
        window.setContentView(Some(&content));

        self.window = Some(window.clone());
        self.content = Some(content.clone());

        ready(
            self,
            content,
            Size::new(options.size.width, options.size.height),
        );

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
                    let mtm = MainThreadMarker::new().unwrap();
                    let app = NSApplication::sharedApplication(mtm);
                    if let Some(win) = app.windows().firstObject()
                        && let Some(content) = win.contentView()
                    {
                        let desc: Retained<NSString> =
                            unsafe { msg_send![&*content, _subtreeDescription] };
                        eprintln!("{desc}");
                    }
                }));
            });
        }
        // Keep the delegate alive for the app lifetime.
        std::mem::forget(delegate);
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

    app.setMainMenu(Some(&menubar));
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
    }
}

fn build_ns_menu(
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
            MI::Submenu { label, items } => {
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

/// Resolve an asset name: DAY_ASSET_ROOT (dev runs / CLI launch) or the app bundle Resources.
fn resolve_asset(name: &str) -> Option<String> {
    if let Ok(root) = std::env::var("DAY_ASSET_ROOT") {
        let p = std::path::Path::new(&root).join(name);
        if p.exists() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    let exe = std::env::current_exe().ok()?;
    let res = exe.parent()?.parent()?.join("Resources/assets").join(name);
    if res.exists() {
        Some(res.to_string_lossy().into_owned())
    } else {
        None
    }
}
