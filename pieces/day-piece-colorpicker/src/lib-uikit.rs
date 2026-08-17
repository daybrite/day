// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// UIKit: `UIColorWell` (iOS 14+, so inside Day's iOS 15 floor) — the swatch control that presents
// the system color picker itself. The whole `UIColorPickerViewController` presentation, its grid /
// spectrum / sliders tabs, the eyedropper and the iPad popover anchoring come with it, which is
// why this arm presents nothing by hand: a hand-rolled presentation would have to find the right
// presenting view controller, and getting that wrong on iPad is a crash rather than a cosmetic
// difference.
//
// `getRed:green:blue:alpha:` returns NO for a color with no RGB representation (a pattern color).
// It cannot happen from this picker — every tab produces an RGB-convertible color — but the return
// value is still checked, because a `false` leaves the out-parameters untouched and reporting
// uninitialized stack as a color would be worse than dropping the pick.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::{NodeId, Proposal, Size};
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_core_foundation::{CGFloat, CGSize};
use objc2_foundation::NSString;
use objc2_ui_kit::{UIColor, UIColorWell, UIControlEvents, UIView};

struct TargetIvars {
    node: NodeId,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayColorWellUIKitTarget"]
    #[ivars = TargetIvars]
    struct WellTarget;

    unsafe impl NSObjectProtocol for WellTarget {}

    impl WellTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            let Some(well) = sender.downcast_ref::<UIColorWell>() else {
                return;
            };
            let Some(color) = well.selectedColor() else {
                return; // the well can be cleared; the piece keeps the last real pick
            };
            if let Some(c) = to_day_color(&color) {
                day_uikit::emit(self.ivars().node, Event::custom(PICK_TAG, c.to_string()));
            }
        }
    }
);

impl WellTarget {
    fn new(mtm: MainThreadMarker, node: NodeId) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { node });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static TARGETS: RefCell<HashMap<usize, Retained<WellTarget>>> = RefCell::new(HashMap::new());
}

fn to_day_color(c: &UIColor) -> Option<Color> {
    let (mut r, mut g, mut b, mut a) = (0.0 as CGFloat, 0.0, 0.0, 0.0);
    // SAFETY: four valid pointers to live stack slots, which is the whole contract of the method.
    let ok = unsafe { c.getRed_green_blue_alpha(&mut r, &mut g, &mut b, &mut a) };
    ok.then(|| Color::rgba(r, g, b, a))
}

fn to_ui_color(c: Color) -> Retained<UIColor> {
    UIColor::colorWithRed_green_blue_alpha(c.r, c.g, c.b, c.a)
}

fn make(_backend: &mut Uikit, p: &ColorProps, id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    let well = UIColorWell::new(mtm);
    well.setSelectedColor(Some(&to_ui_color(p.color)));
    well.setSupportsAlpha(p.alpha);
    if !p.title.is_empty() {
        well.setTitle(Some(&NSString::from_str(&p.title)));
    }
    let target = WellTarget::new(mtm, id);
    unsafe {
        well.addTarget_action_forControlEvents(
            Some(&target as &WellTarget as &AnyObject),
            sel!(fire:),
            UIControlEvents::ValueChanged,
        );
    }
    let view: Retained<UIView> = Retained::from(<UIColorWell as AsRef<UIView>>::as_ref(&well));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const UIView) as usize, target)
    });
    view
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &ColorPatch) {
    let ColorPatch::SetColor(c) = patch;
    let Some(well) = (**h).downcast_ref::<UIColorWell>() else {
        return;
    };
    // No-op on an unchanged value: `setSelectedColor:` does not fire `.valueChanged`, but the
    // guard also keeps the well from flickering its swatch on every unrelated signal flush.
    if well.selectedColor().as_deref().and_then(to_day_color) != Some(*c) {
        well.setSelectedColor(Some(&to_ui_color(*c)));
    }
}

/// The well is a fixed-size swatch on iOS; `sizeThatFits` reports it regardless of the probe.
fn measure(_backend: &mut Uikit, h: &Retained<UIView>, _p: Proposal) -> Size {
    let s = h.sizeThatFits(CGSize::new(1.0e6, 1.0e6));
    Size::new(s.width.ceil().max(28.0), s.height.ceil().max(28.0))
}

/// Drop the retained target when the well goes away — same address-reuse hazard as every other
/// arm that keeps per-view state in a map keyed by the view's pointer.
fn release(_backend: &mut Uikit, h: &Retained<UIView>) {
    TARGETS.with(|m| {
        m.borrow_mut()
            .remove(&((h.as_ref() as *const UIView) as usize));
    });
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: ColorProps, patch: ColorPatch,
    make: make, update: update, measure: measure, release: release);
