// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// AppKit: NSPopUpButton (menu) / NSSegmentedControl (segmented) / NSButton radio group (inline)
// ---------------------------------------------------------------------------

use day_spec::Event;
use day_spec::ffi_guard;
use day_spec::props::{PickerPatch, PickerProps, PickerStyle};
use day_spec::sidetable::SideTable;

use crate::AppKit;
use day_spec::{NodeId, Proposal, Size};
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
        // Contained like every trampoline (§8.5): a panic must not unwind into AppKit.
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            ffi_guard::contain((), || {
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
                    crate::emit(self.ivars().node, Event::SelectionChanged(idx as i64));
                }
            })
        }
    }
);

impl PickerTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PickerIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

day_core::tls_group! {
    /// Keeps each picker's target alive (the control holds it weakly). A [`SideTable`], so
    /// the backend's release sweep drops it with its view — this map had no release path.
    static TARGETS: SideTable<Retained<PickerTarget>> = SideTable::new();

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
    // Out-of-range app state must not reach selectItemAtIndex: it raises an NSException,
    // which Rust cannot catch — the process aborts (same guard as the segmented arm).
    if p.selected < p.options.len() {
        popup.selectItemAtIndex(p.selected as isize);
    }
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

fn make(backend: &mut AppKit, p: &PickerProps, id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    let target = PickerTarget::new(mtm, id);
    let view = match p.style {
        PickerStyle::Menu => make_menu(mtm, p, &target),
        PickerStyle::Segmented => make_segmented(mtm, p, &target),
        PickerStyle::Inline => make_inline(mtm, p, &target),
    };
    TARGETS.with(|t| t.insert((view.as_ref() as *const NSView) as usize, target));
    view
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &PickerPatch) {
    let i = match patch {
        PickerPatch::Selected(i) => *i,
        PickerPatch::Options(opts) => return set_options(h, opts),
    };
    // Range guards throughout: an out-of-range index raises an NSException in AppKit,
    // which Rust cannot catch — the process aborts.
    if let Some(popup) = h.downcast_ref::<NSPopUpButton>() {
        if (i as isize) < popup.numberOfItems() && popup.indexOfSelectedItem() != i as isize {
            popup.selectItemAtIndex(i as isize);
        }
    } else if let Some(seg) = h.downcast_ref::<NSSegmentedControl>() {
        if (i as isize) < seg.segmentCount() && seg.selectedSegment() != i as isize {
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

/// New option labels, in place. The selected INDEX is preserved where it still exists —
/// AppKit resets a rebuilt pop-up to item 0 and drops a segmented control's selection, so
/// each arm restores it explicitly (clamped, since an NSException here would abort).
fn set_options(h: &Retained<NSView>, opts: &[String]) {
    if let Some(popup) = h.downcast_ref::<NSPopUpButton>() {
        let want = popup.indexOfSelectedItem().max(0) as usize;
        popup.removeAllItems();
        for o in opts {
            popup.addItemWithTitle(&NSString::from_str(o));
        }
        if !opts.is_empty() {
            popup.selectItemAtIndex(want.min(opts.len() - 1) as isize);
        }
    } else if let Some(seg) = h.downcast_ref::<NSSegmentedControl>() {
        let want = seg.selectedSegment().max(0) as usize;
        seg.setSegmentCount(opts.len() as isize);
        for (i, o) in opts.iter().enumerate() {
            seg.setLabel_forSegment(&NSString::from_str(o), i as isize);
        }
        if !opts.is_empty() {
            seg.setSelectedSegment(want.min(opts.len() - 1) as isize);
        }
    } else if let Some(stack) = h.downcast_ref::<NSStackView>() {
        // Inline: the radios ARE the options, so relabel in place and add/remove the tail.
        // Rebuilding every button would drop the group's shared target/action wiring.
        let subs = stack.arrangedSubviews();
        let existing: Vec<Retained<NSButton>> = subs
            .iter()
            .filter_map(|v| v.downcast_ref::<NSButton>().map(Retained::from))
            .collect();
        for (i, o) in opts.iter().enumerate() {
            match existing.get(i) {
                Some(b) => b.setTitle(&NSString::from_str(o)),
                None => {
                    let Some(first) = existing.first() else { break };
                    let radio = unsafe {
                        NSButton::radioButtonWithTitle_target_action(
                            &NSString::from_str(o),
                            first.target().as_deref(),
                            first.action(),
                            stack.mtm(),
                        )
                    };
                    radio.setTag(i as isize);
                    stack.addArrangedSubview(<NSButton as AsRef<NSView>>::as_ref(&radio));
                }
            }
        }
        for b in existing.iter().skip(opts.len()) {
            stack.removeArrangedSubview(<NSButton as AsRef<NSView>>::as_ref(b));
            b.removeFromSuperview();
        }
    }
}

fn measure(_backend: &mut AppKit, h: &Retained<NSView>, _p: Proposal) -> Size {
    let s = h.fittingSize();
    Size::new(s.width.ceil().max(60.0), s.height.ceil().max(22.0))
}

// Built-in dispatch adapters: the backend's realize/update matches call these (the downcasts
// the satellite-era `renderer!` macro used to generate).
pub(crate) fn realize_any(
    b: &mut AppKit,
    props: &dyn std::any::Any,
    id: day_spec::NodeId,
) -> crate::Handle {
    // A mismatched payload warns once and degrades to the shared placeholder (never panics
    // inside a native up-call) — same policy as the builtin arms in lib.rs.
    match day_spec::props_of::<PickerProps>(day_spec::kinds::PICKER, "appkit", props) {
        Some(p) => make(b, p, id),
        None => crate::placeholder_view(b.mtm(), day_spec::kinds::PICKER),
    }
}

pub(crate) fn update_any(b: &mut AppKit, h: &crate::Handle, patch: &dyn std::any::Any) {
    if let Some(p) = patch.downcast_ref::<PickerPatch>() {
        update(b, h, p);
    }
}

pub(crate) fn measure_any(
    b: &mut AppKit,
    h: &crate::Handle,
    p: day_spec::Proposal,
) -> day_spec::Size {
    measure(b, h, p)
}
