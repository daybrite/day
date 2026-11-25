// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Small compositor-driven nav icon overlays. No timer, canvas loop, or table reload.
//!
//! The ring's stroke and disc are layer colors, which hold one appearance, so the overlay is a
//! view class of its own that sets them again when its trait collection's color appearance
//! changes.
use day_spec::{Ink, SystemColor, ffi_guard};
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{Message, define_class, msg_send};
use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColor, CGPath};
use objc2_foundation::{NSNumber, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CALayer, CAMediaTiming, CAShapeLayer, CATransaction, CATransform3D,
};

const NAME: &str = "day.nav.icon-progress";
/// The ring's stroke.
const INK: Ink = Ink::system(SystemColor::Text);
/// The disc under the ring, which keeps it readable over any icon.
const BACKGROUND: Ink = Ink::system(SystemColor::Background).with_alpha(0.82);

define_class!(
    /// The view that hosts a ring over a nav icon.
    #[unsafe(super(UIView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayNavProgressOverlay"]
    struct Overlay;

    unsafe impl NSObjectProtocol for Overlay {}

    impl Overlay {
        /// A light/dark switch (the app's, the window's or the system's) while the ring shows.
        #[unsafe(method(traitCollectionDidChange:))]
        fn trait_collection_did_change(
            &self,
            previous: Option<&objc2_ui_kit::UITraitCollection>,
        ) {
            // SAFETY: UIView implements `traitCollectionDidChange:` (nullable UITraitCollection,
            // void); this override forwards its own argument.
            let _: () = unsafe { msg_send![super(self), traitCollectionDidChange: previous] };
            ffi_guard::contain((), || {
                use objc2_ui_kit::UITraitEnvironment as _;
                let changed = previous.is_none_or(|old| {
                    self.traitCollection()
                        .hasDifferentColorAppearanceComparedToTraitCollection(Some(old))
                });
                if changed {
                    recolor(self);
                }
            });
        }
    }
);

/// Set the ring's colors for the traits `overlay` is shown with.
fn recolor(overlay: &UIView) {
    let layer = overlay.layer();
    // SAFETY: unaudited Apple API; live operands on the main thread (crate note "Unsafe calls").
    let ring = unsafe { layer.sublayers() }.and_then(|layers| {
        layers
            .iter()
            .find(|l| l.name().as_ref().is_some_and(|n| n.to_string() == NAME))
            .and_then(|l| l.retain().downcast::<CAShapeLayer>().ok())
    });
    let Some(ring) = ring else {
        return;
    };
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    ring.setStrokeColor(Some(&crate::imp::ink_cg(overlay, INK)));
    ring.setFillColor(Some(&crate::imp::ink_cg(overlay, BACKGROUND)));
    CATransaction::commit();
}

#[expect(
    clippy::option_option,
    reason = "outer `None` removes the ring; the inner value is day-spec's progress, where `None` is indeterminate"
)]
fn paint(
    parent: &CALayer,
    state: Option<Option<f64>>,
    size: f64,
    ink: &CGColor,
    background: &CGColor,
) {
    // SAFETY: unaudited Apple API; live operands on the main thread (crate note "Unsafe calls").
    let existing = unsafe { parent.sublayers() }.and_then(|layers| {
        layers
            .iter()
            .find(|l| l.name().as_ref().is_some_and(|n| n.to_string() == NAME))
            .map(|l| l.retain())
    });
    let Some(fraction) = state else {
        if let Some(layer) = existing {
            layer.removeAllAnimations();
            layer.removeFromSuperlayer();
        }
        return;
    };
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    let ring = existing
        .and_then(|l| l.downcast::<CAShapeLayer>().ok())
        .unwrap_or_else(|| {
            let ring = CAShapeLayer::new();
            ring.setName(Some(&NSString::from_str(NAME)));
            ring.setFrame(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(size, size)));
            // SAFETY: CGPathCreateWithEllipseInRect accepts a null transform; the rect is finite.
            let path = unsafe {
                CGPath::with_ellipse_in_rect(
                    CGRect::new(CGPoint::new(1.5, 1.5), CGSize::new(size - 3.0, size - 3.0)),
                    std::ptr::null(),
                )
            };
            ring.setPath(Some(&path));
            ring.setLineWidth(2.0);
            ring.setTransform(CATransform3D::new_rotation(
                -std::f64::consts::FRAC_PI_2,
                0.0,
                0.0,
                1.0,
            ));
            parent.addSublayer(&ring);
            ring
        });
    ring.setFillColor(Some(background));
    ring.setStrokeColor(Some(ink));
    let key = NSString::from_str("spin");
    if let Some(value) = fraction {
        ring.removeAnimationForKey(&key);
        ring.setStrokeEnd(value.clamp(0.0, 1.0));
    } else {
        ring.setStrokeEnd(0.25);
        // SAFETY: unaudited Apple API; live operands on the main thread (crate note "Unsafe calls").
        if unsafe { ring.animationForKey(&key) }.is_none() {
            let spin = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str(
                "transform.rotation",
            )));
            // SAFETY: unaudited Apple API; live operands on the main thread (crate note "Unsafe calls").
            unsafe {
                spin.setFromValue(Some(&NSNumber::numberWithDouble(
                    -std::f64::consts::FRAC_PI_2,
                )));
                spin.setToValue(Some(&NSNumber::numberWithDouble(
                    3.0 * std::f64::consts::FRAC_PI_2,
                )));
            }
            spin.setDuration(0.9);
            spin.setRepeatCount(f32::INFINITY);
            ring.addAnimation_forKey(&spin, Some(&key));
        }
    }
    CATransaction::commit();
}

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_foundation::NSArray;
use objc2_ui_kit::NSObjectUIAccessibility;
use objc2_ui_kit::{UICollectionViewListCell, UIImageView, UIView};
const TAG: isize = 0x44504;
fn find_icon(view: &UIView) -> Option<Retained<UIImageView>> {
    for child in view.subviews() {
        if let Some(icon) = child.downcast_ref::<UIImageView>() {
            return Some(icon.retain());
        }
        if let Some(icon) = find_icon(&child) {
            return Some(icon);
        }
    }
    None
}
#[expect(
    clippy::option_option,
    reason = "outer `None` removes the ring; the inner value is day-spec's progress, where `None` is indeterminate"
)]
pub(crate) fn update(
    cell: &UICollectionViewListCell,
    state: Option<Option<f64>>,
    mtm: MainThreadMarker,
) {
    // SAFETY: UIView's `initWithFrame:` (CGRect) on a fresh Overlay returns the view; the rest is
    // unaudited Apple API on live views, main thread (`mtm`).
    unsafe {
        cell.layoutIfNeeded();
        let Some(icon) = find_icon(&cell.contentView()) else {
            return;
        };
        let existing = icon.viewWithTag(TAG);
        if state.is_none() {
            if let Some(view) = existing {
                view.layer().removeAllAnimations();
                view.removeFromSuperview();
            }
            return;
        }
        let overlay = existing.unwrap_or_else(|| {
            let view: Retained<Overlay> = msg_send![
                Overlay::alloc(mtm),
                initWithFrame: CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(20.0, 20.0))
            ];
            let view = view.into_super();
            view.setTag(TAG);
            view.setUserInteractionEnabled(false);
            view.setIsAccessibilityElement(false, mtm);
            view.setTranslatesAutoresizingMaskIntoConstraints(false);
            icon.addSubview(&view);
            objc2_ui_kit::NSLayoutConstraint::activateConstraints(
                &NSArray::from_retained_slice(&[
                    view.centerXAnchor()
                        .constraintEqualToAnchor(&icon.centerXAnchor()),
                    view.centerYAnchor()
                        .constraintEqualToAnchor(&icon.centerYAnchor()),
                    view.widthAnchor().constraintEqualToConstant(20.0),
                    view.heightAnchor().constraintEqualToConstant(20.0),
                ]),
                mtm,
            );
            view
        });
        let ink = crate::imp::ink_cg(&overlay, INK);
        let background = crate::imp::ink_cg(&overlay, BACKGROUND);
        paint(&overlay.layer(), state, 20.0, &ink, &background);
    }
}
