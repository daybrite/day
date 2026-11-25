// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Small compositor-driven nav icon overlays. No timer, canvas loop, or table reload.
//!
//! The ring's stroke and disc are layer colors, which hold one appearance, so the overlay is a
//! view class of its own that sets them again when its effective appearance changes.
use day_spec::{Ink, SystemColor, ffi_guard};
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
    /// The view that hosts a ring over a nav icon, apart from AppKit's managed image layer.
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayNavProgressOverlay"]
    struct Overlay;

    unsafe impl NSObjectProtocol for Overlay {}

    impl Overlay {
        /// A light/dark switch (the app's, the window's or the system's) while the ring shows.
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            // SAFETY: forwards NSView's `viewDidChangeEffectiveAppearance` (no arguments, no
            // result).
            let _: () = unsafe { msg_send![super(self), viewDidChangeEffectiveAppearance] };
            ffi_guard::contain((), || recolor(self));
        }
    }
);

/// Set the ring's colors for the appearance `overlay` is shown in.
fn recolor(overlay: &NSView) {
    let Some(ring) = overlay.layer().and_then(|l| ring_of(&l)) else {
        return;
    };
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    ring.setStrokeColor(Some(&crate::color::cg_for(overlay, INK)));
    ring.setFillColor(Some(&crate::color::cg_for(overlay, BACKGROUND)));
    CATransaction::commit();
}

/// Size `ring` to the icon it covers, `w` × `h`: the largest circle that fits, centered. Bounds
/// and position rather than a frame, since the ring carries a rotation.
fn shape(ring: &CAShapeLayer, w: f64, h: f64) {
    let size = w.min(h).max(0.0);
    ring.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(size, size)));
    ring.setPosition(CGPoint::new(w / 2.0, h / 2.0));
    let inset = 1.5_f64.min(size / 2.0);
    // SAFETY: a null transform pointer is allowed and means none; the path is returned retained.
    let path = unsafe {
        CGPath::with_ellipse_in_rect(
            CGRect::new(
                CGPoint::new(inset, inset),
                CGSize::new(size - 2.0 * inset, size - 2.0 * inset),
            ),
            std::ptr::null(),
        )
    };
    ring.setPath(Some(&path));
}

/// The ring layer an overlay's root layer holds, if one is showing.
fn ring_of(parent: &CALayer) -> Option<Retained<CAShapeLayer>> {
    // SAFETY: unaudited Apple API; live operands on the main thread (crate note "Unsafe calls").
    unsafe { parent.sublayers() }.and_then(|layers| {
        layers
            .iter()
            .find(|l| l.name().as_ref().is_some_and(|n| n.to_string() == NAME))
            .and_then(|l| l.retain().downcast::<CAShapeLayer>().ok())
    })
}

#[expect(
    clippy::option_option,
    reason = "outer `None` removes the ring; the inner value is day-spec's progress, where `None` is indeterminate"
)]
fn paint(
    parent: &CALayer,
    state: Option<Option<f64>>,
    (w, h): (f64, f64),
    ink: &CGColor,
    background: &CGColor,
) {
    let existing = ring_of(parent);
    let Some(fraction) = state else {
        if let Some(layer) = existing {
            layer.removeAllAnimations();
            layer.removeFromSuperlayer();
        }
        return;
    };
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    let ring = existing.unwrap_or_else(|| {
        let ring = CAShapeLayer::new();
        ring.setName(Some(&NSString::from_str(NAME)));
        shape(&ring, w, h);
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

use objc2::{MainThreadOnly, rc::Retained};
use objc2_app_kit::{
    NSAutoresizingMaskOptions, NSImageView, NSUserInterfaceItemIdentification, NSView,
};
#[expect(
    clippy::option_option,
    reason = "outer `None` removes the ring; the inner value is day-spec's progress, where `None` is indeterminate"
)]
pub(crate) fn update(icon: &NSImageView, state: Option<Option<f64>>) {
    // Host the animation separately from AppKit's managed image backing layer.
    // SAFETY: `initWithFrame:` is NSView's initializer, sent to a fresh Overlay allocation (no
    // ivars); the other calls are unaudited Apple APIs on `icon`'s live views, on the main thread.
    unsafe {
        let existing = icon
            .subviews()
            .iter()
            .find(|view| {
                view.identifier()
                    .as_ref()
                    .is_some_and(|id| id.to_string() == NAME)
            })
            .map(|view| view.retain());
        if state.is_none() {
            if let Some(view) = existing {
                view.removeFromSuperview();
            }
            return;
        }
        let overlay: Retained<NSView> = existing.unwrap_or_else(|| {
            let view: Retained<Overlay> =
                msg_send![Overlay::alloc(icon.mtm()), initWithFrame: icon.bounds()];
            view.setIdentifier(Some(&NSString::from_str(NAME)));
            // The icon takes the size the sidebar's row size style gives it (the system's
            // Sidebar icon size), so the overlay follows it and `fit` reshapes the ring.
            view.setAutoresizingMask(
                NSAutoresizingMaskOptions::ViewWidthSizable
                    | NSAutoresizingMaskOptions::ViewHeightSizable,
            );
            view.setLayer(Some(&CALayer::new()));
            view.setWantsLayer(true);
            icon.addSubview(&view);
            view.into_super()
        });
        if let Some(layer) = overlay.layer() {
            let ink = crate::color::cg_for(&overlay, INK);
            let background = crate::color::cg_for(&overlay, BACKGROUND);
            let b = icon.bounds();
            paint(
                &layer,
                state,
                (b.size.width, b.size.height),
                &ink,
                &background,
            );
        }
    }
}

/// Re-fit a showing ring to its icon after the icon's size changed: the cell lays the icon out
/// at its row size style's size, and again when the system's Sidebar icon size changes.
pub(crate) fn fit(icon: &NSImageView) {
    let overlay = icon.subviews().iter().find_map(|view| {
        view.identifier()
            .as_ref()
            .is_some_and(|id| id.to_string() == NAME)
            .then(|| view.retain())
    });
    let Some(ring) = overlay.and_then(|v| v.layer()).and_then(|l| ring_of(&l)) else {
        return;
    };
    let b = icon.bounds();
    #[expect(
        clippy::float_cmp,
        reason = "unchanged-size check against the exact value `shape` stored"
    )]
    let fitted = ring.bounds().size.width == b.size.width.min(b.size.height);
    if fitted {
        return;
    }
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    shape(&ring, b.size.width, b.size.height);
    CATransaction::commit();
}
