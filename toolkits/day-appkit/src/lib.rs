//! day-appkit — the macos-appkit backend (DESIGN.md §9). objc2, pure Rust, no shim.
//!
//! `Handle = Retained<NSView>`. Containers are flipped `NSView`s (top-left origin, so day's
//! frames apply directly and survive diffing). One custom target class (`DayTarget`) forwards
//! target/action + text-delegate callbacks into the day event sink, node-id keyed (§8.3).

#![allow(unused_unsafe)]
#![cfg(target_os = "macos")]

use std::any::Any;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use linkme::distributed_slice;
use objc2::rc::Retained;
use objc2::runtime::{NSObjectProtocol, ProtocolObject};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::NSAccessibility as _;
use objc2_app_kit::NSAppearanceCustomization as _;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSBitmapImageFileType, NSBox,
    NSBoxType, NSButton, NSColor, NSControl, NSControlTextEditingDelegate, NSFont,
    NSGraphicsContext, NSLineBreakMode, NSMenu, NSMenuItem, NSScrollView, NSSlider, NSSwitch,
    NSTextField, NSTextFieldDelegate, NSView, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_app_kit::{NSOutlineViewDataSource, NSOutlineViewDelegate};
use objc2_foundation::{NSDictionary, NSNotification, NSObject, NSPoint, NSRect, NSSize, NSString};

use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Cap, DrawOp, Event, EventSink, Font, NodeId, PieceKind, Platform,
    Proposal, Rect, Registry, Renderer, Size, Support, Toolkit, WINDOW_NODE, WindowOptions, kinds,
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

struct FlippedIvars;

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
    }
);

impl DayFlipped {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(FlippedIvars);
        unsafe { msg_send![super(this), init] }
    }
}

// ---------------------------------------------------------------------------
// DayCanvas — a flipped view replaying the day display list in drawRect (§11)
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
// Navigation (docs/navigation.md): NSSplitView host, sidebar + detail panes.
// Page FRAMES are pane-owned (autoresized); day lays content inside the size each
// page reports from setFrameSize:. day's set_frame on pages is skipped.
// ---------------------------------------------------------------------------

struct NavState {
    sidebar_wrap: Retained<NSView>,
    detail_wrap: Retained<NSView>,
    /// Detail pages in stack order (the sidebar page is not in here).
    pages: Vec<Retained<NSView>>,
    positioned: bool,
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
            Shape::Polygon(_) => return None,
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

fn nsfont(f: Font) -> Retained<NSFont> {
    unsafe {
        match f {
            Font::Title => NSFont::boldSystemFontOfSize(24.0),
            Font::Headline => NSFont::boldSystemFontOfSize(15.0),
            Font::Body => NSFont::systemFontOfSize(NSFont::systemFontSize()),
            Font::Caption => NSFont::systemFontOfSize(11.0),
            Font::System(pt) => NSFont::systemFontOfSize(pt),
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

impl Toolkit for AppKit {
    type Handle = Handle;

    fn capability(&self, cap: Cap) -> Support {
        match cap {
            Cap::Snapshot | Cap::NativeSymbols | Cap::NavSplit => Support::Native,
            _ => Support::Unsupported,
        }
    }

    fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> Handle {
        let mtm = self.mtm;
        match kind {
            kinds::CONTAINER => view_of(DayFlipped::new(mtm)),
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
            kinds::CANVAS => view_of(DayCanvas::new(mtm)),
            kinds::NAV => {
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
                    // (Holding priorities are a no-op when day drives the split's frame
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
                // Unregistered kind: visible-but-harmless placeholder (§8.2's debug check
                // will panic first in debug builds once the required-kinds set lands).
                view_of(unsafe {
                    NSTextField::labelWithString(&NSString::from_str(&format!("⟨{kind}⟩")), mtm)
                })
            }
        }
    }

    fn update(&mut self, h: &Handle, kind: PieceKind, patch: &dyn Any, _anim: Option<&AnimSpec>) {
        match kind {
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
                                // Hide the outgoing top; reveal its predecessor (day
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
        NAV_STATE.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        NAV_PAGES.with(|set| {
            set.borrow_mut().remove(&ptr_of(&h));
        });
        NAV_MENUS.with(|m| {
            m.borrow_mut().remove(&ptr_of(&h));
        });
        unsafe { h.removeFromSuperview() };
    }

    fn insert(&mut self, parent: &Handle, child: &Handle, index: usize) {
        // Nav host: index 0 = sidebar page, the rest are detail (stack) pages. Pages fill
        // their pane via autoresizing — the pane, not day, owns their frames.
        let handled = NAV_STATE.with(|m| {
            let mut m = m.borrow_mut();
            let Some(state) = m.get_mut(&ptr_of(parent)) else {
                return false;
            };
            let wrap = if index == 0 {
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
        // Every day parent is flipped (DayFlipped containers, flipped scroll document views),
        // so top-left frames apply directly.
        let r = NSRect::new(
            NSPoint::new(frame.origin.x, frame.origin.y),
            NSSize::new(frame.size.width, frame.size.height),
        );
        // Nav host: the sidebar should HOLD its width when the window resizes, letting the
        // detail pane absorb the change (the standard Finder/Mail behavior). NSSplitView's
        // holding priorities don't take effect when day drives the split's frame directly, so
        // we capture the current sidebar width, resize, then restore the divider to it.
        if let Some(split) = h.downcast_ref::<objc2_app_kit::NSSplitView>() {
            let first = NAV_STATE.with(|m| {
                m.borrow_mut()
                    .get_mut(&ptr_of(h))
                    .map(|s| !std::mem::replace(&mut s.positioned, true))
                    .unwrap_or(false)
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
                let target = if first || prev_sidebar <= 1.0 {
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

    fn set_a11y(&mut self, h: &Handle, a11y: &A11yProps) {
        unsafe {
            if let Some(id) = &a11y.identifier {
                h.setAccessibilityIdentifier(Some(&NSString::from_str(id)));
            }
            if let Some(label) = &a11y.label {
                h.setAccessibilityLabel(Some(&NSString::from_str(label)));
            }
            if a11y.hidden {
                h.setAccessibilityElement(false);
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
}

impl Platform for AppKit {
    const TARGET: &'static str = "macos-appkit";
    const TOOLKIT: &'static str = "appkit";

    fn run(mut self, options: WindowOptions, ready: Box<dyn FnOnce(Self, Handle, Size)>) {
        let mtm = self.mtm;
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);

        install_main_menu(mtm, &app, &options.title);

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

/// The default main menu (§21.2 M2): App menu with Quit; Edit menu wired to the responder
/// chain so Cmd+C/V/X/A work in NSTextFields; Window menu basics.
fn install_main_menu(mtm: MainThreadMarker, app: &NSApplication, title: &str) {
    let menubar = NSMenu::new(mtm);

    let app_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);
    let quit = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(&format!("Quit {title}")),
            Some(sel!(terminate:)),
            &NSString::from_str("q"),
        )
    };
    app_menu.addItem(&quit);
    app_item.setSubmenu(Some(&app_menu));
    menubar.addItem(&app_item);

    let edit_item = NSMenuItem::new(mtm);
    let edit_menu =
        unsafe { NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("Edit")) };
    let add = |label: &str, action: objc2::runtime::Sel, key: &str| {
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(label),
                Some(action),
                &NSString::from_str(key),
            )
        };
        edit_menu.addItem(&item);
    };
    add("Undo", sel!(undo:), "z");
    add("Redo", sel!(redo:), "Z");
    edit_menu.addItem(&NSMenuItem::separatorItem(mtm));
    add("Cut", sel!(cut:), "x");
    add("Copy", sel!(copy:), "c");
    add("Paste", sel!(paste:), "v");
    add("Select All", sel!(selectAll:), "a");
    edit_item.setSubmenu(Some(&edit_menu));
    menubar.addItem(&edit_item);

    app.setMainMenu(Some(&menubar));
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
