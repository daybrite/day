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

fn paint(
    parent: &CALayer,
    state: Option<Option<f64>>,
    size: f64,
    ink: &CGColor,
    background: &CGColor,
) {
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
            let path = unsafe {
                CGPath::with_ellipse_in_rect(
                    CGRect::new(CGPoint::new(1.5, 1.5), CGSize::new(size - 3.0, size - 3.0)),
                    std::ptr::null(),
                )
            };
            ring.setPath(Some(&path));
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
use objc2_app_kit::{NSColor, NSImageView, NSUserInterfaceItemIdentification, NSView};
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
            paint(&layer, state, 18.0, &ink, &background);
        }
    }
}
