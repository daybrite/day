// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Small compositor-driven nav icon overlays. No timer, canvas loop, or table reload.
use objc2::Message;

use objc2_core_foundation::{CGPoint, CGRect, CGSize};
use objc2_core_graphics::{CGColor, CGPath};
use objc2_foundation::{NSNumber, NSString};
use objc2_quartz_core::{
    CABasicAnimation, CALayer, CAMediaTiming, CAShapeLayer, CATransaction, CATransform3D,
};

const NAME: &str = "day.nav.icon-progress";

/// Size `ring` to the icon it covers, `w` × `h`: the largest circle that fits, centered. Bounds
/// and position rather than a frame, since the ring carries a rotation.
fn shape(ring: &CAShapeLayer, w: f64, h: f64) {
    let size = w.min(h).max(0.0);
    ring.setBounds(CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(size, size)));
    ring.setPosition(CGPoint::new(w / 2.0, h / 2.0));
    let inset = 1.5_f64.min(size / 2.0);
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
    unsafe { parent.sublayers() }.and_then(|layers| {
        layers
            .iter()
            .find(|l| l.name().as_ref().is_some_and(|n| n.to_string() == NAME))
            .and_then(|l| l.retain().downcast::<CAShapeLayer>().ok())
    })
}

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
        ring.setFillColor(Some(background));
        ring.setStrokeColor(Some(ink));
        ring.setTransform(unsafe {
            CATransform3D::new_rotation(-std::f64::consts::FRAC_PI_2, 0.0, 0.0, 1.0)
        });
        parent.addSublayer(&ring);
        ring
    });
    let key = NSString::from_str("spin");
    if let Some(value) = fraction {
        ring.removeAnimationForKey(&key);
        ring.setStrokeEnd(value.clamp(0.0, 1.0));
    } else {
        ring.setStrokeEnd(0.25);
        if unsafe { ring.animationForKey(&key) }.is_none() {
            let spin = CABasicAnimation::animationWithKeyPath(Some(&NSString::from_str(
                "transform.rotation",
            )));
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
    NSAutoresizingMaskOptions, NSColor, NSImageView, NSUserInterfaceItemIdentification, NSView,
};
pub(crate) fn update(icon: &NSImageView, state: Option<Option<f64>>) {
    // Host the animation separately from AppKit's managed image backing layer.
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
            let view = NSView::initWithFrame(NSView::alloc(icon.mtm()), icon.bounds());
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
            view
        });
        if let Some(layer) = overlay.layer() {
            let ink = NSColor::labelColor().CGColor();
            let background = NSColor::windowBackgroundColor()
                .colorWithAlphaComponent(0.82)
                .CGColor();
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
    let overlay = unsafe { icon.subviews() }.iter().find_map(|view| {
        view.identifier()
            .as_ref()
            .is_some_and(|id| id.to_string() == NAME)
            .then(|| view.retain())
    });
    let Some(ring) = overlay.and_then(|v| v.layer()).and_then(|l| ring_of(&l)) else {
        return;
    };
    let b = icon.bounds();
    if ring.bounds().size.width == b.size.width.min(b.size.height) {
        return;
    }
    CATransaction::begin();
    CATransaction::setDisableActions(true);
    shape(&ring, b.size.width, b.size.height);
    CATransaction::commit();
}
