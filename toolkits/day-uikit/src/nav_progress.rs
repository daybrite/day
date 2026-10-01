// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Small compositor-driven nav icon overlays. No timer, canvas loop, or table reload.
use objc2::Message;
use objc2::rc::Retained;
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

use objc2::{MainThreadMarker, MainThreadOnly};
use objc2_foundation::NSArray;
use objc2_ui_kit::NSObjectUIAccessibility;
use objc2_ui_kit::{UICollectionViewListCell, UIColor, UIImageView, UIView};
const TAG: isize = 0x44504;
fn find_icon(view: &UIView) -> Option<Retained<UIImageView>> {
    for child in unsafe { view.subviews() } {
        if let Some(icon) = child.downcast_ref::<UIImageView>() {
            return Some(icon.retain());
        }
        if let Some(icon) = find_icon(&child) {
            return Some(icon);
        }
    }
    None
}
pub(crate) fn update(
    cell: &UICollectionViewListCell,
    state: Option<Option<f64>>,
    mtm: MainThreadMarker,
) {
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
            let view = UIView::initWithFrame(
                UIView::alloc(mtm),
                CGRect::new(CGPoint::new(0.0, 0.0), CGSize::new(20.0, 20.0)),
            );
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
        let ink = UIColor::labelColor().CGColor();
        let background = UIColor::systemBackgroundColor()
            .colorWithAlphaComponent(0.82)
            .CGColor();
        paint(&overlay.layer(), state, 20.0, &ink, &background);
    }
}
