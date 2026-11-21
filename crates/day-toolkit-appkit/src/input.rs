// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Pointer and key input the Day views share (docs/gestures.md, docs/menus.md "Keys",
//! docs/forms.md "Disabling a group"): which views take the pointer at all (pass-through and hit
//! slop, decided in `hitTest:`), the press observer, the input kind, key releases, and subtree
//! enablement.
//!
//! One local event monitor watches the left button, mouse movement, tablet proximity and key
//! releases for the whole app. It never consumes a pointer event: a press is observed, not
//! claimed, so taps, drags, native controls and scroll views run exactly as if it were absent.
//! It consumes a key release only when Day delivered that key's press (the release is Day's then).
use super::*;
use day_spec::{Insets, KeyPhase, PointerKind};
use objc2_app_kit::{
    NSEventMask, NSEventSubtype, NSPointingDeviceType, NSUserInterfaceLayoutDirection,
};

// ---------------------------------------------------------------------------
// Pure logic (unit-tested below)
// ---------------------------------------------------------------------------

/// `rect` grown by `slop` on each edge, in a view's own coordinates. `flipped`: y runs down (the
/// top edge is the minimum y). `rtl`: the leading edge is the right one.
pub(super) fn grow_rect(rect: NSRect, slop: Insets, flipped: bool, rtl: bool) -> NSRect {
    let (left, right) = if rtl {
        (slop.trailing, slop.leading)
    } else {
        (slop.leading, slop.trailing)
    };
    let (low_y, high_y) = if flipped {
        (slop.top, slop.bottom)
    } else {
        (slop.bottom, slop.top)
    };
    NSRect::new(
        NSPoint::new(rect.origin.x - left, rect.origin.y - low_y),
        NSSize::new(
            (rect.size.width + left + right).max(0.0),
            (rect.size.height + low_y + high_y).max(0.0),
        ),
    )
}

/// Whether `p` lies in `rect`, half-open on the far edges as `mouse:inRect:` counts.
pub(super) fn rect_contains(rect: NSRect, p: NSPoint) -> bool {
    p.x >= rect.origin.x
        && p.x < rect.origin.x + rect.size.width
        && p.y >= rect.origin.y
        && p.y < rect.origin.y + rect.size.height
}

/// What a recognized single click becomes (docs/gestures.md "Double tap").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TapDisposition {
    /// The node has no double tap: report the tap now.
    Immediate,
    /// The node has a double tap too: hold the tap for the double-click interval, and report it
    /// only if no double tap arrives in that time.
    Pending,
    /// The click that completes a double click: the double-tap recognizer reports it.
    Drop,
}

/// A click with `click_count` (AppKit's `NSEvent.clickCount`, 1 for a single click, 2 for the
/// second click of a double, 3 for a triple) on a node that does or does not also listen for
/// double taps. A triple click is a double tap followed by a pending single.
pub(super) fn tap_disposition(double_enabled: bool, click_count: isize) -> TapDisposition {
    if !double_enabled {
        TapDisposition::Immediate
    } else if click_count >= 2 && click_count % 2 == 0 {
        TapDisposition::Drop
    } else {
        TapDisposition::Pending
    }
}

/// The input kind an event says is behind the pointer, or `None` for an event that says
/// nothing (docs/gestures.md "Input kind"). A pen's events are mouse events with the tablet
/// subtype; its proximity event names the device as it comes near. Trackpads and mice are both
/// `Mouse`, and leaving proximity changes nothing until the next pointer event.
pub(super) fn classify_pointer(
    kind: NSEventType,
    subtype: NSEventSubtype,
    entering: bool,
    device: NSPointingDeviceType,
) -> Option<PointerKind> {
    if kind == NSEventType::TabletProximity {
        let pen = device == NSPointingDeviceType::Pen || device == NSPointingDeviceType::Eraser;
        return (entering && pen).then_some(PointerKind::Stylus);
    }
    if kind == NSEventType::TabletPoint {
        return Some(PointerKind::Stylus);
    }
    if subtype == NSEventSubtype::TabletPoint || subtype == NSEventSubtype::TabletProximity {
        Some(PointerKind::Stylus)
    } else {
        Some(PointerKind::Mouse)
    }
}

/// The key presses Day delivered and whose release it still owes (docs/menus.md "Keys"): one
/// entry per held key code, naming the node that heard the press, the key as it was named, and
/// the responder that held focus. A release is reported only to that node, only while that
/// responder still holds focus, and only once.
#[derive(Default)]
pub(super) struct KeyClaims {
    held: Vec<KeyClaim>,
}

#[derive(Clone, Debug, PartialEq)]
struct KeyClaim {
    code: u16,
    node: NodeId,
    key: String,
    responder: usize,
}

impl KeyClaims {
    /// Day delivered the press of `code` to `node`, named `key`, while `responder` had focus.
    pub(super) fn down(&mut self, code: u16, node: NodeId, key: String, responder: usize) {
        self.held.retain(|c| c.code != code);
        self.held.push(KeyClaim {
            code,
            node,
            key,
            responder,
        });
    }

    /// A fresh press of `code` arrived (not a repeat): whatever was held for it is stale (its
    /// release went to another app, or ⌘ swallowed it).
    pub(super) fn fresh_press(&mut self, code: u16) {
        self.held.retain(|c| c.code != code);
    }

    /// Whether a repeat of `code` belongs to the press Day delivered while `responder` had
    /// focus.
    pub(super) fn owns_repeat(&self, code: u16, responder: usize) -> bool {
        self.held
            .iter()
            .any(|c| c.code == code && c.responder == responder)
    }

    /// The release of `code` while `responder` has focus: the node and key to report it to, or
    /// `None` when Day did not deliver the press or focus has moved since. The claim ends
    /// either way.
    pub(super) fn up(&mut self, code: u16, responder: usize) -> Option<(NodeId, String)> {
        let at = self.held.iter().position(|c| c.code == code)?;
        let claim = self.held.remove(at);
        (claim.responder == responder).then_some((claim.node, claim.key))
    }

    /// `node` lost focus: its held keys report no release.
    pub(super) fn forget_node(&mut self, node: NodeId) {
        self.held.retain(|c| c.node != node);
    }
}

/// What a disabled subtree changed on one native view, to put back when the last disabled root
/// above it is enabled again (docs/forms.md "Disabling a group").
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Prior {
    /// An `NSControl` and the enabled state it had.
    Control(bool),
    /// An `NSTextView` (a text area) and its editable and selectable flags.
    Text { editable: bool, selectable: bool },
    /// A composed Day view: only its accessibility state changed.
    Composed,
}

/// The disabled roots and the views each one turned off. A view under two disabled roots
/// stays off until both are enabled, and keeps the state it had before the first.
pub(super) struct Ledger<H> {
    roots: Vec<LedgerRoot<H>>,
}

struct LedgerRoot<H> {
    key: usize,
    depth: u32,
    views: Vec<(usize, H, Prior)>,
}

impl<H> Default for Ledger<H> {
    fn default() -> Self {
        Ledger { roots: Vec::new() }
    }
}

impl<H: Clone> Ledger<H> {
    /// Disable the root `key`. `collect` lists the views under it with their current state;
    /// it runs only for a root not already disabled. Answers the views to turn off now: a view
    /// another root already holds is off already and keeps the state that root recorded.
    pub(super) fn disable(
        &mut self,
        key: usize,
        collect: impl FnOnce() -> Vec<(usize, H, Prior)>,
    ) -> Vec<H> {
        if let Some(root) = self.roots.iter_mut().find(|r| r.key == key) {
            root.depth += 1;
            return Vec::new();
        }
        let mut fresh = Vec::new();
        let mut views = Vec::new();
        for (view_key, handle, prior) in collect() {
            if let Some(held) = self.prior_of(view_key) {
                views.push((view_key, handle, held));
            } else {
                fresh.push(handle.clone());
                views.push((view_key, handle, prior));
            }
        }
        self.roots.push(LedgerRoot {
            key,
            depth: 1,
            views,
        });
        fresh
    }

    /// Enable the root `key` once. Answers the views no disabled root holds any more, with
    /// the state to restore; empty while the root is still disabled by another call.
    pub(super) fn enable(&mut self, key: usize) -> Vec<(H, Prior)> {
        let Some(at) = self.roots.iter().position(|r| r.key == key) else {
            return Vec::new();
        };
        self.roots[at].depth -= 1;
        if self.roots[at].depth > 0 {
            return Vec::new();
        }
        self.release_root(at)
    }

    /// The root `key` left the tree while disabled: restore what it alone held.
    pub(super) fn forget(&mut self, key: usize) -> Vec<(H, Prior)> {
        match self.roots.iter().position(|r| r.key == key) {
            Some(at) => self.release_root(at),
            None => Vec::new(),
        }
    }

    fn release_root(&mut self, at: usize) -> Vec<(H, Prior)> {
        let root = self.roots.remove(at);
        root.views
            .into_iter()
            .filter(|(view_key, _, _)| self.prior_of(*view_key).is_none())
            .map(|(_, handle, prior)| (handle, prior))
            .collect()
    }

    fn prior_of(&self, view_key: usize) -> Option<Prior> {
        self.roots.iter().find_map(|r| {
            r.views
                .iter()
                .find(|(k, _, _)| *k == view_key)
                .map(|(_, _, p)| *p)
        })
    }

    /// Whether `key` is a disabled root.
    pub(super) fn is_root(&self, key: usize) -> bool {
        self.roots.iter().any(|r| r.key == key)
    }

    /// Whether nothing is disabled.
    pub(super) fn is_empty(&self) -> bool {
        self.roots.is_empty()
    }
}

// ---------------------------------------------------------------------------
// State
// ---------------------------------------------------------------------------

/// A press under way (docs/gestures.md "Press").
struct ActivePress {
    view: Retained<NSView>,
    node: NodeId,
    window: usize,
    last: Point,
}

struct InputMonitor(Retained<AnyObject>);
impl Drop for InputMonitor {
    fn drop(&mut self) {
        unsafe { NSEvent::removeMonitor(&self.0) };
    }
}

day_core::tls_group! {
    /// View ptr → node for `GestureKind::Press`: the views the press observer reports for.
    pub(super) static PRESS_NODES: SideTable<NodeId> = SideTable::new();
    /// View ptr → its hit slop (`set_hit_slop`). Read by the enclosing container's `hitTest:`.
    pub(super) static HIT_SLOPS: SideTable<Insets> = SideTable::new();
    /// Set once any view has had a slop: until then a container's `hitTest:` keeps AppKit's
    /// fast rejection of points outside its frame.
    static ANY_SLOP: Cell<bool> = const { Cell::new(false) };
    /// View ptr → the hover tracking area `enable_gesture(Hover)` installed, so a slop change
    /// can replace it with one over the grown rectangle.
    pub(super) static HOVER_AREAS: SideTable<Retained<NSTrackingArea>> = SideTable::new();
    /// View ptr → the generation of the single tap it is holding back for the double-click
    /// interval. A double tap, or a newer tap, supersedes it.
    static PENDING_TAPS: SideTable<u64> = SideTable::new();
    static TAP_GENERATION: Cell<u64> = const { Cell::new(0) };
    /// Views whose long press fired in the current click sequence: the click that ends that
    /// sequence is not a tap.
    pub(super) static LONG_HELD: SideTable<()> = SideTable::new();
    static PRESSES: RefCell<Vec<ActivePress>> = const { RefCell::new(Vec::new()) };
    static PRESS_POLL: Cell<bool> = const { Cell::new(false) };
    static KEY_CLAIMS: RefCell<KeyClaims> = RefCell::new(KeyClaims::default());
    static LEDGER: RefCell<Ledger<Retained<NSView>>> = RefCell::new(Ledger::default());
    static POINTER: Cell<PointerKind> = const { Cell::new(PointerKind::Mouse) };
    static MONITOR: RefCell<Option<InputMonitor>> = const { RefCell::new(None) };
}

// ---------------------------------------------------------------------------
// Coordinates
// ---------------------------------------------------------------------------

/// `p`, in `view`'s own coordinates, as Day's node-local point (origin top left).
pub(super) fn top_left(view: &NSView, p: NSPoint) -> Point {
    if view.isFlipped() {
        Point::new(p.x, p.y)
    } else {
        Point::new(p.x, view.bounds().size.height - p.y)
    }
}

/// The window location of `event`, in `view`'s coordinates, as Day's node-local point.
fn event_point(view: &NSView, event: &NSEvent) -> Point {
    top_left(
        view,
        view.convertPoint_fromView(event.locationInWindow(), None),
    )
}

fn is_rtl(view: &NSView) -> bool {
    view.userInterfaceLayoutDirection() == NSUserInterfaceLayoutDirection::RightToLeft
}

/// The area in which `view` takes the pointer, in its own coordinates: its bounds, grown by
/// its hit slop.
pub(super) fn hit_area(view: &NSView) -> NSRect {
    let bounds = view.bounds();
    match HIT_SLOPS.with(|t| t.get(ptr_of(view))) {
        Some(slop) => grow_rect(bounds, slop, view.isFlipped(), is_rtl(view)),
        None => bounds,
    }
}

// ---------------------------------------------------------------------------
// Pass-through and hit slop (docs/gestures.md "Pass-through", "Hit slop")
// ---------------------------------------------------------------------------

/// Whether `view` takes pointer input of its own: a recognizer is attached (any gesture kind),
/// a context menu is set, it is a drag source, a drop target or a window drag region, it
/// joined the focus order, or it shows a tooltip (AppKit's own hover, which a view behind
/// would otherwise answer for). `set_cursor` alone does not count.
pub(super) fn takes_pointer(view: &NSView) -> bool {
    let key = ptr_of(view);
    GESTURES.with(|m| m.borrow().contains_key(&key))
        || PRESS_NODES.with(|t| t.contains(key))
        || PAN_NODES.with(|t| t.contains(key))
        || ROTATE_NODES.with(|t| t.contains(key))
        || HOVER_NODES.with(|t| t.contains(key))
        || FOCUSABLE_NODES.with(|t| t.contains(key))
        || CTX_MENU_FNS.with(|t| t.contains(key))
        || view.menu().is_some()
        || view.toolTip().is_some_and(|t| t.length() > 0)
        || transfer::involved(view)
        || window_chrome::is_drag_region(view)
}

/// A native leaf that never takes the pointer on its own: a non-editable image view and a
/// separator box. A container skips it when it has nothing attached, so a press reaches
/// whatever is behind it.
fn passive_leaf(view: &NSView) -> bool {
    let leaf = view
        .downcast_ref::<objc2_app_kit::NSImageView>()
        .is_some_and(|iv| !unsafe { iv.isEditable() })
        || view.downcast_ref::<NSBox>().is_some();
    leaf && !takes_pointer(view)
}

/// `hitTest:` for Day's containers: the children front to back, each answering inside its
/// hit slop, passive leaves skipped, then the container itself only if it takes the pointer.
/// `point` is in the superview's coordinates, as AppKit passes it.
pub(super) fn container_hit_test(this: &NSView, point: NSPoint) -> Option<Retained<NSView>> {
    if this.isHidden() {
        return None;
    }
    let local = match unsafe { this.superview() } {
        Some(sup) => this.convertPoint_fromView(point, Some(&sup)),
        None => point,
    };
    let inside = rect_contains(this.bounds(), local);
    // A child's slop may reach outside this view; without any slop in the app, keep AppKit's
    // rejection of a point outside the frame.
    if !inside && !ANY_SLOP.with(Cell::get) {
        return None;
    }
    if subtree_disabled(this) {
        return None;
    }
    for child in unsafe { this.subviews() }.to_vec().into_iter().rev() {
        if child.isHidden() {
            continue;
        }
        if let Some(hit) = child.hitTest(local)
            && (ptr_of(&hit) != ptr_of(&child) || !passive_leaf(&child))
        {
            return Some(hit);
        }
        if let Some(slop) = HIT_SLOPS.with(|t| t.get(ptr_of(&child)))
            && takes_pointer(&child)
            && rect_contains(
                grow_rect(child.frame(), slop, this.isFlipped(), is_rtl(&child)),
                local,
            )
        {
            return Some(child.clone());
        }
    }
    (inside && takes_pointer(this)).then(|| Retained::from(this))
}

/// `hitTest:` for a Day leaf (a canvas, a label) whose own answer was `hit`: the leaf keeps the
/// pointer only when `takes` (it has something attached, or it is selectable) and its subtree
/// is enabled.
pub(super) fn leaf_hit_test(
    this: &NSView,
    hit: Option<Retained<NSView>>,
    takes: bool,
) -> Option<Retained<NSView>> {
    let hit = hit?;
    if subtree_disabled(this) {
        return None;
    }
    if ptr_of(&hit) == ptr_of(this) && !takes {
        return None;
    }
    Some(hit)
}

/// `set_hit_slop`: record the slop and, for a hovering view, move its tracking area to the
/// grown rectangle.
pub(super) fn set_hit_slop(view: &NSView, slop: Insets) {
    let key = ptr_of(view);
    if slop == Insets::ZERO {
        HIT_SLOPS.with(|t| t.take(key));
    } else {
        HIT_SLOPS.with(|t| t.insert(key, slop));
        ANY_SLOP.with(|c| c.set(true));
    }
    if HOVER_AREAS.with(|t| t.contains(key)) {
        install_hover_area(view);
    }
}

/// The hover tracking area (docs/canvas.md "Interaction"): the view's visible rect, or with a
/// hit slop its grown bounds. Replaces the one installed before, so a view never has two.
pub(super) fn install_hover_area(view: &NSView) {
    let key = ptr_of(view);
    if let Some(old) = HOVER_AREAS.with(|t| t.take(key)) {
        unsafe { view.removeTrackingArea(&old) };
    }
    let base = NSTrackingAreaOptions::MouseEnteredAndExited
        | NSTrackingAreaOptions::MouseMoved
        | NSTrackingAreaOptions::ActiveInKeyWindow;
    let (rect, opts) = if HIT_SLOPS.with(|t| t.contains(key)) {
        (hit_area(view), base)
    } else {
        (NSRect::ZERO, base | NSTrackingAreaOptions::InVisibleRect)
    };
    let owner: &AnyObject = view;
    let area = unsafe {
        NSTrackingArea::initWithRect_options_owner_userInfo(
            NSTrackingArea::alloc(),
            rect,
            opts,
            Some(owner),
            None,
        )
    };
    unsafe { view.addTrackingArea(&area) };
    HOVER_AREAS.with(|t| t.insert(key, area));
}

/// One hover phase on a Day view, for the node that asked (docs/canvas.md "Interaction",
/// docs/gestures.md "State styling"), in the view's own coordinates.
///
/// `mouseEntered:`/`mouseExited:` are scoped by the tracking area, but `mouseMoved:` is not: it
/// walks the responder chain, so a focused view hears motion anywhere in its window. Hence the
/// test against the hover area (the bounds, grown by a hit slop), without which a view reports
/// a hover for a pointer nowhere near it. An exit is reported wherever it happens. Answers
/// whether the view hovers at all (whether the event was Day's).
pub(super) fn report_hover(view: &NSView, event: &NSEvent, phase: day_spec::DragPhase) -> bool {
    let Some(node) = HOVER_NODES.with(|t| t.get(ptr_of(view))) else {
        return false;
    };
    let loc = view.convertPoint_fromView(event.locationInWindow(), None);
    let b = hit_area(view);
    let inside = loc.x >= b.origin.x
        && loc.x <= b.origin.x + b.size.width
        && loc.y >= b.origin.y
        && loc.y <= b.origin.y + b.size.height;
    if inside || phase == day_spec::DragPhase::Ended {
        emit(
            node,
            Event::Hover {
                phase,
                location: top_left(view, loc),
            },
        );
    }
    true
}

/// From `updateTrackingAreas`: a slopped hover area follows the view's new bounds.
pub(super) fn refresh_hover_area(view: &NSView) {
    let key = ptr_of(view);
    if HOVER_AREAS.with(|t| t.contains(key)) && HIT_SLOPS.with(|t| t.contains(key)) {
        install_hover_area(view);
    }
}

// ---------------------------------------------------------------------------
// The monitor: press, input kind, key releases
// ---------------------------------------------------------------------------

/// Install the app's input monitor once (the event sink's arrival does it).
pub(super) fn install_monitor() {
    MONITOR.with(|slot| {
        if slot.borrow().is_some() {
            return;
        }
        let block = block2::RcBlock::new(|raw: std::ptr::NonNull<NSEvent>| -> *mut NSEvent {
            let event = unsafe { raw.as_ref() };
            ffi_guard::contain(raw.as_ptr(), || {
                if observe(event) {
                    std::ptr::null_mut()
                } else {
                    raw.as_ptr()
                }
            })
        });
        let mask = NSEventMask::LeftMouseDown
            | NSEventMask::LeftMouseDragged
            | NSEventMask::LeftMouseUp
            | NSEventMask::MouseMoved
            | NSEventMask::TabletPoint
            | NSEventMask::TabletProximity
            | NSEventMask::KeyDown
            | NSEventMask::KeyUp;
        *slot.borrow_mut() =
            unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &block) }
                .map(InputMonitor);
    });
}

/// One event, before AppKit dispatches it. Answers whether Day consumed it (only ever a key
/// release Day owes).
fn observe(event: &NSEvent) -> bool {
    let kind = event.r#type();
    if kind == NSEventType::KeyDown {
        if !event.isARepeat() {
            KEY_CLAIMS.with(|k| k.borrow_mut().fresh_press(event.keyCode()));
        }
        return false;
    }
    if kind == NSEventType::KeyUp {
        return key_released(event);
    }
    let pointer = if kind == NSEventType::TabletProximity {
        classify_pointer(
            kind,
            NSEventSubtype::MouseEvent,
            unsafe { event.isEnteringProximity() },
            unsafe { event.pointingDeviceType() },
        )
    } else {
        classify_pointer(
            kind,
            unsafe { event.subtype() },
            false,
            NSPointingDeviceType::Unknown,
        )
    };
    if let Some(now) = pointer
        && POINTER.with(|p| p.replace(now)) != now
    {
        emit(WINDOW_NODE, Event::PointerKindChanged(now));
    }
    if kind == NSEventType::LeftMouseDown {
        press_down(event);
    } else if kind == NSEventType::LeftMouseDragged {
        press_moved(event);
    } else if kind == NSEventType::LeftMouseUp {
        press_up(event);
    }
    false
}

/// The kind of pointer seen last (`Toolkit::primary_pointer`).
pub(super) fn pointer_kind() -> PointerKind {
    POINTER.with(Cell::get)
}

// --- Press ---------------------------------------------------------------------------------

/// The left button went down: every view with a press observer under the pointer, the one hit
/// and the ones around it, is pressed. A Control-click is the context-menu click, a secondary
/// press, so it presses nothing; a click that only activates its window reaches no view.
fn press_down(event: &NSEvent) {
    // Whatever was still pressed lost its release to a tracking loop.
    release_all(None);
    if event
        .modifierFlags()
        .contains(NSEventModifierFlags::Control)
    {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Some(window) = event.window(mtm) else {
        return;
    };
    let Some(content) = window.contentView() else {
        return;
    };
    // `hitTest:` takes a point in the receiver's superview's coordinates: the window's.
    let mut view = content.hitTest(event.locationInWindow());
    if !window.isKeyWindow()
        && view
            .as_ref()
            .is_none_or(|v| !v.acceptsFirstMouse(Some(event)))
    {
        return;
    }
    let mut pressed = Vec::new();
    while let Some(v) = view {
        if let Some(node) = PRESS_NODES.with(|t| t.get(ptr_of(&v))) {
            let at = event_point(&v, event);
            emit(
                node,
                Event::Press {
                    pressed: true,
                    location: at,
                },
            );
            pressed.push(ActivePress {
                view: v.clone(),
                node,
                window: Retained::as_ptr(&window) as usize,
                last: at,
            });
        }
        view = unsafe { v.superview() };
    }
    if pressed.is_empty() {
        return;
    }
    PRESSES.with(|p| p.borrow_mut().extend(pressed));
    schedule_press_poll();
}

/// The pointer moved with the button down: a press whose view the pointer left (its bounds plus
/// slop) ends; re-entering does not press again.
fn press_moved(event: &NSEvent) {
    let left: Vec<ActivePress> = PRESSES.with(|p| {
        let mut p = p.borrow_mut();
        let mut left = Vec::new();
        let mut i = 0;
        while i < p.len() {
            let local = p[i]
                .view
                .convertPoint_fromView(event.locationInWindow(), None);
            let at = top_left(&p[i].view, local);
            if rect_contains(hit_area(&p[i].view), local) {
                p[i].last = at;
                i += 1;
            } else {
                let mut gone = p.remove(i);
                gone.last = at;
                left.push(gone);
            }
        }
        left
    });
    for press in left {
        end_press(&press);
    }
}

/// The button came up: every press ends where the pointer is.
fn press_up(event: &NSEvent) {
    let ended: Vec<ActivePress> = PRESSES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    for mut press in ended {
        press.last = event_point(&press.view, event);
        end_press(&press);
    }
}

fn end_press(press: &ActivePress) {
    emit(
        press.node,
        Event::Press {
            pressed: false,
            location: press.last,
        },
    );
}

/// End the presses that match `filter` (all when `None`) at their last known point.
fn release_all(filter: Option<&dyn Fn(&ActivePress) -> bool>) {
    let ended: Vec<ActivePress> = PRESSES.with(|p| {
        let mut p = p.borrow_mut();
        let (gone, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut *p)
            .into_iter()
            .partition(|press| filter.is_none_or(|f| f(press)));
        *p = keep;
        gone
    });
    for press in &ended {
        end_press(press);
    }
}

/// While a press is under way, look every 50 ms for the ways AppKit ends a press without a
/// mouse-up reaching the monitor: a control's or a drag session's tracking loop took the
/// release, the window stopped being key, the view left its window or was hidden.
fn schedule_press_poll() {
    if PRESS_POLL.with(|p| p.replace(true)) {
        return;
    }
    let when = dispatch2::DispatchTime::try_from(std::time::Duration::from_millis(50))
        .unwrap_or(dispatch2::DispatchTime::NOW);
    let queued = dispatch2::DispatchQueue::main().after(when, || {
        ffi_guard::contain((), press_poll);
    });
    if queued.is_err() {
        PRESS_POLL.with(|p| p.set(false));
    }
}

fn press_poll() {
    PRESS_POLL.with(|p| p.set(false));
    let button_up = NSEvent::pressedMouseButtons() & 1 == 0;
    release_all(Some(&|press: &ActivePress| {
        button_up
            || press.view.isHiddenOrHasHiddenAncestor()
            || press
                .view
                .window()
                .is_none_or(|w| Retained::as_ptr(&w) as usize != press.window || !w.isKeyWindow())
    }));
    if PRESSES.with(|p| !p.borrow().is_empty()) {
        schedule_press_poll();
    }
}

// --- Taps ----------------------------------------------------------------------------------

/// Hold a single tap for the double-click interval (docs/gestures.md "Double tap"); a double
/// tap on the same view, or a newer pending tap, cancels it.
pub(super) fn hold_tap(view: &NSView, node: NodeId, at: Point) {
    let key = ptr_of(view);
    let generation = TAP_GENERATION.with(|g| {
        let next = g.get().wrapping_add(1);
        g.set(next);
        next
    });
    PENDING_TAPS.with(|t| t.insert(key, generation));
    let interval = NSEvent::doubleClickInterval().clamp(0.0, 5.0);
    let when = dispatch2::DispatchTime::try_from(std::time::Duration::from_secs_f64(interval))
        .unwrap_or(dispatch2::DispatchTime::NOW);
    let queued = dispatch2::DispatchQueue::main().after(when, move || {
        ffi_guard::contain((), || {
            if PENDING_TAPS.with(|t| t.get(key)) == Some(generation) {
                PENDING_TAPS.with(|t| t.take(key));
                emit(node, Event::Tap(at));
            }
        });
    });
    if queued.is_err() {
        PENDING_TAPS.with(|t| t.take(key));
        emit(node, Event::Tap(at));
    }
}

/// A double tap landed on `view`: the single tap it was holding never happened.
pub(super) fn cancel_held_tap(view: &NSView) {
    PENDING_TAPS.with(|t| t.take(ptr_of(view)));
}

// --- Keys ----------------------------------------------------------------------------------

/// The responder holding focus in `event`'s window, as a pointer to compare.
fn responder_of(event: &NSEvent) -> Option<usize> {
    let mtm = MainThreadMarker::new()?;
    let window = event.window(mtm)?;
    let responder = window.firstResponder()?;
    Some(Retained::as_ptr(&responder) as usize)
}

/// Day delivered the press `event` to `node` as `key`: its release is owed to `node`.
pub(super) fn claim_key(event: &NSEvent, node: NodeId, key: String) {
    if let Some(responder) = responder_of(event) {
        install_monitor();
        KEY_CLAIMS.with(|k| {
            k.borrow_mut().down(event.keyCode(), node, key, responder);
        });
    }
}

/// Whether the repeat `event` continues a press Day delivered to whatever holds focus now.
pub(super) fn owns_repeat(event: &NSEvent) -> bool {
    responder_of(event).is_some_and(|responder| {
        KEY_CLAIMS.with(|k| k.borrow().owns_repeat(event.keyCode(), responder))
    })
}

/// `node` lost focus: no release is reported for the keys it heard go down.
pub(super) fn forget_keys(node: NodeId) {
    KEY_CLAIMS.with(|k| k.borrow_mut().forget_node(node));
}

/// A key came up: report the release to the node that heard its press, with the modifiers held
/// now and no text, and keep the event from going further (the press was Day's).
fn key_released(event: &NSEvent) -> bool {
    let Some(responder) = responder_of(event) else {
        KEY_CLAIMS.with(|k| k.borrow_mut().fresh_press(event.keyCode()));
        return false;
    };
    let Some((node, key)) = KEY_CLAIMS.with(|k| k.borrow_mut().up(event.keyCode(), responder))
    else {
        return false;
    };
    emit(
        node,
        Event::Key(
            day_spec::KeyEvent::named(key, key_modifiers(event.modifierFlags()))
                .with_phase(KeyPhase::Up),
        ),
    );
    true
}

// ---------------------------------------------------------------------------
// Subtree enablement (docs/forms.md "Disabling a group")
// ---------------------------------------------------------------------------

/// Whether `view` or a view above it is a disabled root.
pub(super) fn subtree_disabled(view: &NSView) -> bool {
    LEDGER.with(|l| {
        let l = l.borrow();
        if l.is_empty() {
            return false;
        }
        let mut cur = Some(Retained::from(view));
        while let Some(v) = cur {
            if l.is_root(ptr_of(&v)) {
                return true;
            }
            cur = unsafe { v.superview() };
        }
        false
    })
}

/// What disabling would change on `view`, or `None` for a view it leaves alone: labels and
/// images keep their look (composed content is the app's to dim), scrollers keep scrolling,
/// and the shared field editor is never touched.
fn prior_of(view: &NSView) -> Option<Prior> {
    if view.downcast_ref::<DayFlipped>().is_some() || view.downcast_ref::<DayCanvas>().is_some() {
        return Some(Prior::Composed);
    }
    if let Some(text) = view.downcast_ref::<NSTextView>() {
        if text.isFieldEditor() {
            return None;
        }
        return Some(Prior::Text {
            editable: text.isEditable(),
            selectable: text.isSelectable(),
        });
    }
    if let Some(field) = view.downcast_ref::<NSTextField>()
        && !field.isEditable()
    {
        return None;
    }
    if view.downcast_ref::<objc2_app_kit::NSImageView>().is_some()
        || view.downcast_ref::<objc2_app_kit::NSScroller>().is_some()
    {
        return None;
    }
    view.downcast_ref::<NSControl>()
        .map(|c| Prior::Control(c.isEnabled()))
}

fn collect(view: &NSView, out: &mut Vec<(usize, Retained<NSView>, Prior)>) {
    if let Some(prior) = prior_of(view) {
        out.push((ptr_of(view), Retained::from(view), prior));
    }
    for sub in &unsafe { view.subviews() } {
        collect(&sub, out);
    }
}

fn turn_off(view: &NSView) {
    match prior_of(view) {
        Some(Prior::Composed) => view.setAccessibilityEnabled(false),
        Some(Prior::Text { .. }) => {
            if let Some(text) = view.downcast_ref::<NSTextView>() {
                text.setEditable(false);
                text.setSelectable(false);
            }
        }
        Some(Prior::Control(_)) => {
            if let Some(control) = view.downcast_ref::<NSControl>() {
                control.setEnabled(false);
            }
        }
        None => {}
    }
}

fn restore(view: &NSView, prior: Prior) {
    match prior {
        Prior::Composed => view.setAccessibilityEnabled(true),
        Prior::Text {
            editable,
            selectable,
        } => {
            if let Some(text) = view.downcast_ref::<NSTextView>() {
                text.setSelectable(selectable);
                text.setEditable(editable);
            }
        }
        Prior::Control(enabled) => {
            if let Some(control) = view.downcast_ref::<NSControl>() {
                control.setEnabled(enabled);
            }
        }
    }
}

/// `set_subtree_enabled`: AppKit has no enabled state that cascades, so a disabled root walks
/// its subviews. Every control is disabled and remembers its own state, a text area stops
/// editing and selecting, Day's composed views report themselves unavailable to assistive
/// technology, focus inside leaves (reporting `FocusChanged(false)`), presses under way end
/// without another report (day-core ended them), and the root's `hitTest:` answers nil.
pub(super) fn set_subtree_enabled(root: &NSView, enabled: bool) {
    let key = ptr_of(root);
    if enabled {
        let back = LEDGER.with(|l| l.borrow_mut().enable(key));
        for (view, prior) in back {
            restore(&view, prior);
        }
        return;
    }
    if let Some(window) = root.window()
        && let Some(responder) = window.firstResponder()
        && responder_inside(&responder, root)
    {
        window.makeFirstResponder(None);
    }
    PRESSES.with(|p| {
        p.borrow_mut()
            .retain(|press| !unsafe { press.view.isDescendantOf(root) });
    });
    let fresh = LEDGER.with(|l| {
        l.borrow_mut().disable(key, || {
            let mut views = Vec::new();
            collect(root, &mut views);
            views
        })
    });
    for view in fresh {
        turn_off(&view);
    }
}

/// Whether `responder` is `root`, a view inside it, or the field editor of a field inside it.
fn responder_inside(responder: &NSResponder, root: &NSView) -> bool {
    if let Some(text) = responder.downcast_ref::<NSText>()
        && let Some(delegate) = unsafe { text.delegate() }
        && let Some(field) = AsRef::<AnyObject>::as_ref(&*delegate).downcast_ref::<NSView>()
        && unsafe { field.isDescendantOf(root) }
    {
        return true;
    }
    responder
        .downcast_ref::<NSView>()
        .is_some_and(|v| unsafe { v.isDescendantOf(root) })
}

/// A disabled root left the tree: put back what it alone held.
pub(super) fn forget_disabled_root(key: usize) {
    let back = LEDGER.with(|l| {
        let mut l = l.borrow_mut();
        if l.is_root(key) {
            l.forget(key)
        } else {
            Vec::new()
        }
    });
    for (view, prior) in back {
        restore(&view, prior);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    #[test]
    fn slop_grows_each_edge_in_flipped_coordinates() {
        let slop = Insets {
            top: 1.0,
            leading: 2.0,
            bottom: 3.0,
            trailing: 4.0,
        };
        let r = grow_rect(rect(10.0, 10.0, 12.0, 12.0), slop, true, false);
        assert_eq!(
            (r.origin.x, r.origin.y, r.size.width, r.size.height),
            (8.0, 9.0, 18.0, 16.0)
        );
        // Right to left: leading is the right edge.
        let r = grow_rect(rect(10.0, 10.0, 12.0, 12.0), slop, true, true);
        assert_eq!((r.origin.x, r.size.width), (6.0, 18.0));
        // Not flipped: the top edge is the high y.
        let r = grow_rect(rect(10.0, 10.0, 12.0, 12.0), slop, false, false);
        assert_eq!((r.origin.y, r.size.height), (7.0, 16.0));
    }

    #[test]
    fn a_dot_with_slop_takes_presses_near_it_only() {
        let dot = rect(0.0, 0.0, 12.0, 12.0);
        let grown = grow_rect(dot, Insets::all(30.0), true, false);
        // 20 points outside the dot lands on it; 40 does not.
        assert!(rect_contains(grown, NSPoint::new(-20.0, 6.0)));
        assert!(!rect_contains(grown, NSPoint::new(-40.0, 6.0)));
        assert!(rect_contains(dot, NSPoint::new(0.0, 0.0)));
        assert!(!rect_contains(dot, NSPoint::new(12.0, 6.0)));
    }

    #[test]
    fn taps_wait_only_beside_a_double_tap() {
        assert_eq!(tap_disposition(false, 1), TapDisposition::Immediate);
        assert_eq!(tap_disposition(false, 2), TapDisposition::Immediate);
        assert_eq!(tap_disposition(true, 1), TapDisposition::Pending);
        assert_eq!(tap_disposition(true, 2), TapDisposition::Drop);
        // A triple click: the double, then a pending single.
        assert_eq!(tap_disposition(true, 3), TapDisposition::Pending);
        assert_eq!(tap_disposition(true, 4), TapDisposition::Drop);
    }

    #[test]
    fn a_pen_is_a_stylus_and_a_trackpad_a_mouse() {
        let plain = NSEventSubtype::MouseEvent;
        let none = NSPointingDeviceType::Unknown;
        assert_eq!(
            classify_pointer(NSEventType::LeftMouseDown, plain, false, none),
            Some(PointerKind::Mouse)
        );
        assert_eq!(
            classify_pointer(
                NSEventType::LeftMouseDown,
                NSEventSubtype::TabletPoint,
                false,
                none
            ),
            Some(PointerKind::Stylus)
        );
        assert_eq!(
            classify_pointer(NSEventType::MouseMoved, NSEventSubtype::Touch, false, none),
            Some(PointerKind::Mouse)
        );
        assert_eq!(
            classify_pointer(
                NSEventType::TabletProximity,
                plain,
                true,
                NSPointingDeviceType::Pen
            ),
            Some(PointerKind::Stylus)
        );
        // Leaving proximity says nothing; the next pointer event decides.
        assert_eq!(
            classify_pointer(
                NSEventType::TabletProximity,
                plain,
                false,
                NSPointingDeviceType::Pen
            ),
            None
        );
        assert_eq!(
            classify_pointer(
                NSEventType::TabletProximity,
                plain,
                true,
                NSPointingDeviceType::Cursor
            ),
            None
        );
    }

    #[test]
    fn a_release_goes_to_the_node_that_heard_the_press() {
        let mut k = KeyClaims::default();
        let (canvas, other) = (NodeId(1), NodeId(2));
        k.down(124, canvas, "ArrowRight".into(), 10);
        assert!(k.owns_repeat(124, 10));
        assert!(!k.owns_repeat(124, 11), "focus moved: not this press");
        assert_eq!(k.up(124, 10), Some((canvas, "ArrowRight".to_string())));
        assert_eq!(k.up(124, 10), None, "one release per press");
        // A key held into a newly focused view: its release is not that view's.
        assert_eq!(k.up(125, 10), None);
        // Focus left before the release: nothing, and the claim is gone.
        k.down(126, other, "ArrowUp".into(), 10);
        assert_eq!(k.up(126, 11), None);
        assert!(!k.owns_repeat(126, 10));
        // Focus loss forgets the node's held keys.
        k.down(123, canvas, "ArrowLeft".into(), 10);
        k.forget_node(canvas);
        assert_eq!(k.up(123, 10), None);
        // A fresh press replaces a claim whose release never came.
        k.down(18, canvas, "1".into(), 10);
        k.fresh_press(18);
        assert_eq!(k.up(18, 10), None);
    }

    #[test]
    fn a_view_under_two_disabled_roots_waits_for_both() {
        let mut l: Ledger<usize> = Ledger::default();
        // Root 1 holds views 10 (an enabled button) and 11 (a disabled one).
        let off = l.disable(1, || {
            vec![
                (10, 10, Prior::Control(true)),
                (11, 11, Prior::Control(false)),
            ]
        });
        assert_eq!(off, vec![10, 11]);
        // Root 2 is inside root 1 and holds view 10 again, now disabled by root 1.
        let off = l.disable(2, || vec![(10, 10, Prior::Control(false))]);
        assert_eq!(off, Vec::<usize>::new(), "already off");
        assert_eq!(l.enable(1), vec![(11, Prior::Control(false))]);
        // View 10 comes back only with root 2, and with the state it had before root 1.
        assert_eq!(l.enable(2), vec![(10, Prior::Control(true))]);
        assert!(l.is_empty());
    }

    #[test]
    fn the_same_root_disabled_twice_needs_two_enables() {
        let mut l: Ledger<usize> = Ledger::default();
        l.disable(1, || vec![(10, 10, Prior::Composed)]);
        let again = l.disable(1, || panic!("not collected twice"));
        assert_eq!(again, Vec::<usize>::new());
        assert_eq!(l.enable(1), Vec::new());
        assert!(l.is_root(1));
        assert_eq!(l.enable(1), vec![(10, Prior::Composed)]);
        assert!(!l.is_root(1));
        // Enabling what is not disabled changes nothing.
        assert_eq!(l.enable(1), Vec::new());
        // A root released while disabled gives back what it held.
        l.disable(3, || {
            vec![(
                12,
                12,
                Prior::Text {
                    editable: true,
                    selectable: true,
                },
            )]
        });
        assert_eq!(
            l.forget(3),
            vec![(
                12,
                Prior::Text {
                    editable: true,
                    selectable: true
                }
            )]
        );
    }
}
