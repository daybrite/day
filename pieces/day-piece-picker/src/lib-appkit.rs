// ---------------------------------------------------------------------------
// AppKit: NSPopUpButton (menu) / NSSegmentedControl (segmented) / NSButton radio group (inline)
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_appkit::AppKit;
use day_spec::{NodeId, Proposal, Renderer, Size};
use linkme::distributed_slice;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSButton, NSControlStateValueOn, NSPopUpButton, NSSegmentSwitchTracking, NSSegmentedControl,
    NSStackView, NSUserInterfaceLayoutOrientation, NSView,
};
use objc2_foundation::{NSObject, NSPoint, NSRect, NSSize, NSString};

struct PickerIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayPickerTarget"]
    #[ivars = PickerIvars]
    struct PickerTarget;

    unsafe impl NSObjectProtocol for PickerTarget {}

    impl PickerTarget {
        // One action for all three styles — read the selected index off whichever sender fired.
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            let idx = if let Some(p) = sender.downcast_ref::<NSPopUpButton>() {
                p.indexOfSelectedItem()
            } else if let Some(s) = sender.downcast_ref::<NSSegmentedControl>() {
                s.selectedSegment()
            } else if let Some(b) = sender.downcast_ref::<NSButton>() {
                b.tag()
            } else {
                -1
            };
            if idx >= 0 {
                day_appkit::emit(self.ivars().node, Event::SelectionChanged(idx as i64));
            }
        }
    }
);

impl PickerTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PickerIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static TARGETS: RefCell<HashMap<usize, Retained<PickerTarget>>> =
        RefCell::new(HashMap::new());
}

fn zero_rect() -> NSRect {
    NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(0.0, 0.0))
}

fn make_menu(mtm: MainThreadMarker, p: &PickerProps, target: &PickerTarget) -> Retained<NSView> {
    let popup =
        NSPopUpButton::initWithFrame_pullsDown(NSPopUpButton::alloc(mtm), zero_rect(), false);
    for opt in &p.options {
        popup.addItemWithTitle(&NSString::from_str(opt));
    }
    popup.selectItemAtIndex(p.selected as isize);
    unsafe {
        popup.setTarget(Some(target));
        popup.setAction(Some(sel!(fire:)));
    }
    Retained::from(<NSPopUpButton as AsRef<NSView>>::as_ref(&popup))
}

fn make_segmented(
    mtm: MainThreadMarker,
    p: &PickerProps,
    target: &PickerTarget,
) -> Retained<NSView> {
    let seg = NSSegmentedControl::new(mtm);
    seg.setSegmentCount(p.options.len() as isize);
    seg.setTrackingMode(NSSegmentSwitchTracking::SelectOne);
    for (i, opt) in p.options.iter().enumerate() {
        seg.setLabel_forSegment(&NSString::from_str(opt), i as isize);
    }
    if p.selected < p.options.len() {
        seg.setSelectedSegment(p.selected as isize);
    }
    unsafe {
        seg.setTarget(Some(target));
        seg.setAction(Some(sel!(fire:)));
    }
    Retained::from(<NSSegmentedControl as AsRef<NSView>>::as_ref(&seg))
}

fn make_inline(mtm: MainThreadMarker, p: &PickerProps, target: &PickerTarget) -> Retained<NSView> {
    let stack = NSStackView::new(mtm);
    stack.setOrientation(NSUserInterfaceLayoutOrientation::Vertical);
    stack.setSpacing(4.0);
    stack.setAlignment(objc2_app_kit::NSLayoutAttribute::Leading);
    for (i, opt) in p.options.iter().enumerate() {
        // Radio buttons sharing a superview + action auto-group (mutually exclusive).
        let radio = unsafe {
            NSButton::radioButtonWithTitle_target_action(
                &NSString::from_str(opt),
                Some(target),
                Some(sel!(fire:)),
                mtm,
            )
        };
        radio.setTag(i as isize);
        if i == p.selected {
            radio.setState(NSControlStateValueOn);
        }
        stack.addArrangedSubview(<NSButton as AsRef<NSView>>::as_ref(&radio));
    }
    Retained::from(<NSStackView as AsRef<NSView>>::as_ref(&stack))
}

fn make(backend: &mut AppKit, props: &dyn std::any::Any, id: NodeId) -> Retained<NSView> {
    let p = props.downcast_ref::<PickerProps>().unwrap();
    let mtm = backend.mtm();
    let target = PickerTarget::new(mtm, id);
    let view = match p.style {
        PickerStyle::Menu => make_menu(mtm, p, &target),
        PickerStyle::Segmented => make_segmented(mtm, p, &target),
        PickerStyle::Inline => make_inline(mtm, p, &target),
    };
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const NSView) as usize, target)
    });
    view
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &dyn std::any::Any) {
    let Some(PickerPatch::Selected(i)) = patch.downcast_ref::<PickerPatch>() else {
        return;
    };
    let i = *i;
    if let Some(popup) = h.downcast_ref::<NSPopUpButton>() {
        if popup.indexOfSelectedItem() != i as isize {
            popup.selectItemAtIndex(i as isize);
        }
    } else if let Some(seg) = h.downcast_ref::<NSSegmentedControl>() {
        if seg.selectedSegment() != i as isize {
            seg.setSelectedSegment(i as isize);
        }
    } else if let Some(stack) = h.downcast_ref::<NSStackView>() {
        // Inline: turn on the i-th radio (its group turns the others off).
        let subs = stack.arrangedSubviews();
        if let Some(v) = subs.iter().nth(i)
            && let Some(b) = v.downcast_ref::<NSButton>()
        {
            b.setState(NSControlStateValueOn);
        }
    }
}

fn measure(_backend: &mut AppKit, h: &Retained<NSView>, _p: Proposal) -> Size {
    let s = h.fittingSize();
    Size::new(s.width.ceil().max(60.0), s.height.ceil().max(22.0))
}

#[distributed_slice(day_appkit::RENDERERS)]
static PICKER_APPKIT: fn() -> Renderer<AppKit> = || Renderer {
    kind: KIND,
    make,
    update,
    measure: Some(measure),
};
