// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// AppKit: an `NSTextField` + `NSStepper` composite — macOS has no combined control, and the
// side-by-side pair is the platform's own idiom (every Keynote/Xcode inspector row). The
// container view is the action target for both: a stepper click syncs the field, a field
// commit (Return, or focus loss via `sendsActionOnEndEditing`) syncs the stepper, and both
// report the settled value. Programmatic `setDoubleValue:` never fires an action, so the
// patch path needs no echo guard.
// ---------------------------------------------------------------------------

use super::*;

use day_appkit::AppKit;
use day_spec::{NodeId, Proposal, Size};
use objc2::rc::Retained;
use objc2::runtime::AnyObject;
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSStepper, NSTextField, NSView};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

/// The gap between the field and its stepper, in points.
const GAP: f64 = 4.0;

struct FieldIvars {
    node: NodeId,
    decimals: u32,
    field: Retained<NSTextField>,
    stepper: Retained<NSStepper>,
}

define_class!(
    #[unsafe(super(NSView))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayStepperField"]
    #[ivars = FieldIvars]
    struct DayStepperField;

    impl DayStepperField {
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.lay_out(size);
        }

        #[unsafe(method(stepperChanged:))]
        fn stepper_changed(&self, _sender: &AnyObject) {
            let iv = self.ivars();
            let v = iv.stepper.doubleValue();
            self.show(v);
            self.report(v);
        }

        #[unsafe(method(fieldChanged:))]
        fn field_changed(&self, _sender: &AnyObject) {
            let iv = self.ivars();
            let (min, max) = (iv.stepper.minValue(), iv.stepper.maxValue());
            let v = iv.field.doubleValue().clamp(min, max);
            iv.stepper.setDoubleValue(v);
            // Normalize what the user typed to the display form ("07.50" → "7.5").
            self.show(v);
            self.report(v);
        }
    }
);

impl DayStepperField {
    fn new(mtm: MainThreadMarker, p: &StepperProps, node: NodeId) -> Retained<Self> {
        let field = NSTextField::new(mtm);
        let stepper = NSStepper::new(mtm);
        stepper.setMinValue(p.min);
        stepper.setMaxValue(p.max);
        stepper.setIncrement(p.step);
        stepper.setValueWraps(false);
        stepper.setAutorepeat(true);
        stepper.setDoubleValue(p.value);
        field.setStringValue(&NSString::from_str(&fmt_value(p.value, p.decimals)));
        // Commit on focus loss too, not only Return — the way an inspector field behaves.
        if let Some(cell) = field.cell() {
            let _: () = unsafe { msg_send![&cell, setSendsActionOnEndEditing: true] };
        }
        let this = Self::alloc(mtm).set_ivars(FieldIvars {
            node,
            decimals: p.decimals,
            field: field.clone(),
            stepper: stepper.clone(),
        });
        let this: Retained<Self> = unsafe { msg_send![super(this), init] };
        unsafe {
            field.setTarget(Some(&this));
            field.setAction(Some(sel!(fieldChanged:)));
            stepper.setTarget(Some(&this));
            stepper.setAction(Some(sel!(stepperChanged:)));
        }
        this.addSubview(&field);
        this.addSubview(&stepper);
        this
    }

    fn show(&self, v: f64) {
        let iv = self.ivars();
        iv.field
            .setStringValue(&NSString::from_str(&fmt_value(v, iv.decimals)));
    }

    fn report(&self, v: f64) {
        day_appkit::emit(self.ivars().node, Event::custom(VALUE_TAG, v.to_string()));
    }

    /// Field flexible on the left, stepper at its natural size on the right, both centered.
    fn lay_out(&self, size: NSSize) {
        let iv = self.ivars();
        let s = iv.stepper.fittingSize();
        let f = iv.field.fittingSize();
        let field_w = (size.width - s.width - GAP).max(0.0);
        let field_h = f.height.min(size.height);
        iv.field.setFrame(NSRect::new(
            NSPoint::new(0.0, ((size.height - field_h) / 2.0).max(0.0)),
            NSSize::new(field_w, field_h),
        ));
        iv.stepper.setFrame(NSRect::new(
            NSPoint::new(field_w + GAP, ((size.height - s.height) / 2.0).max(0.0)),
            NSSize::new(s.width, s.height.min(size.height)),
        ));
    }
}

fn make(backend: &mut AppKit, p: &StepperProps, id: NodeId) -> Retained<NSView> {
    let composite = DayStepperField::new(backend.mtm(), p, id);
    Retained::into_super(composite)
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &StepperPatch) {
    let StepperPatch::SetValue(v) = patch;
    let Some(composite) = h.downcast_ref::<DayStepperField>() else {
        return;
    };
    composite.ivars().stepper.setDoubleValue(*v);
    composite.show(*v);
}

/// Natural size: enough field for a handful of digits plus the stepper. The height follows
/// the taller control, which is the stepper's two arrows.
fn measure(_backend: &mut AppKit, h: &Retained<NSView>, _p: Proposal) -> Size {
    let Some(composite) = h.downcast_ref::<DayStepperField>() else {
        return Size::new(96.0, 24.0);
    };
    let iv = composite.ivars();
    let s = iv.stepper.fittingSize();
    let f = iv.field.fittingSize();
    Size::new(
        (56.0 + GAP + s.width).ceil(),
        f.height.max(s.height).ceil().max(22.0),
    )
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: StepperProps, patch: StepperPatch,
    make: make, update: update, measure: measure);
