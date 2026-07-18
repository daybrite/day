// ---------------------------------------------------------------------------
// UIKit: UIDatePicker for both pieces — Compact → .compact (field → calendar popover / time
// keypad, the modern iOS idiom), Inline → .inline for dates (embedded calendar) and .wheels for
// times (iOS has no inline clock face; wheels ARE its embedded time UI). Calendar/timeZone pinned
// to proleptic-Gregorian GMT (locale stays the user's) so civil values map 1:1 onto NSDate epoch
// seconds. iOS 15 floor ⊇ every style used here.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_spec::{NodeId, Proposal, Size};
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_core_foundation::CGSize;
use objc2_foundation::{NSCalendar, NSCalendarIdentifierGregorian, NSDate, NSTimeZone};
use objc2_ui_kit::{UIControlEvents, UIDatePicker, UIDatePickerMode, UIDatePickerStyle, UIView};

struct TargetIvars {
    node: NodeId,
    /// `true` → this target belongs to a time picker (report seconds-of-day, not epoch days).
    time: bool,
}

define_class!(
    #[unsafe(super(objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayDateTimeUIKitTarget"]
    #[ivars = TargetIvars]
    struct DateTimeTarget;

    unsafe impl NSObjectProtocol for DateTimeTarget {}

    impl DateTimeTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            let Some(p) = sender.downcast_ref::<UIDatePicker>() else {
                return;
            };
            let ti = p.date().timeIntervalSince1970();
            let iv = self.ivars();
            if iv.time {
                let t = DayTime::from_seconds_of_day(ti.rem_euclid(86_400.0).round() as i64);
                day_uikit::emit(iv.node, Event::custom("timepicker:value", t.to_string()));
            } else {
                let d = DayDate::from_epoch_days((ti / 86_400.0).floor() as i64);
                day_uikit::emit(iv.node, Event::custom("datepicker:value", d.to_string()));
            }
        }
    }
);

impl DateTimeTarget {
    fn new(mtm: MainThreadMarker, node: NodeId, time: bool) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(TargetIvars { node, time });
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    static TARGETS: RefCell<HashMap<usize, Retained<DateTimeTarget>>> =
        RefCell::new(HashMap::new());
}

fn date_at_epoch_seconds(secs: i64) -> Retained<NSDate> {
    NSDate::dateWithTimeIntervalSince1970(secs as f64)
}

/// A shared UIDatePicker skeleton pinned to Gregorian GMT (user locale kept for display).
fn make_picker(
    mode: UIDatePickerMode,
    style: UIDatePickerStyle,
    node: NodeId,
    time: bool,
) -> (Retained<UIDatePicker>, Retained<UIView>) {
    let mtm = MainThreadMarker::new().unwrap();
    let p = UIDatePicker::new(mtm);
    p.setDatePickerMode(mode);
    p.setPreferredDatePickerStyle(style);
    let gmt = NSTimeZone::timeZoneForSecondsFromGMT(0);
    p.setTimeZone(Some(&gmt));
    if let Some(cal) = NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian })
    {
        cal.setTimeZone(&gmt);
        p.setCalendar(Some(&cal));
    }
    let target = DateTimeTarget::new(mtm, node, time);
    unsafe {
        p.addTarget_action_forControlEvents(
            Some(&target as &DateTimeTarget as &AnyObject),
            sel!(fire:),
            UIControlEvents::ValueChanged,
        );
    }
    let view: Retained<UIView> = Retained::from(<UIDatePicker as AsRef<UIView>>::as_ref(&p));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const UIView) as usize, target)
    });
    (p, view)
}

fn set_if_changed(p: &UIDatePicker, secs: i64) {
    if p.date().timeIntervalSince1970() as i64 != secs {
        p.setDate(&date_at_epoch_seconds(secs));
    }
}

fn measure_picker(h: &Retained<UIView>) -> Size {
    let s = h.sizeThatFits(CGSize::new(1.0e6, 1.0e6));
    Size::new(s.width.ceil().max(60.0), s.height.ceil().max(28.0))
}

mod date_renderer {
    use super::*;

    fn make(_backend: &mut Uikit, p: &DateProps, id: NodeId) -> Retained<UIView> {
        let (picker, view) = make_picker(
            UIDatePickerMode::Date,
            match p.style {
                Style::Inline => UIDatePickerStyle::Inline,
                Style::Automatic | Style::Compact => UIDatePickerStyle::Compact,
            },
            id,
            false,
        );
        picker.setDate(&date_at_epoch_seconds(p.date.to_epoch_days() * 86_400));
        if let Some(min) = p.min {
            picker.setMinimumDate(Some(&date_at_epoch_seconds(min.to_epoch_days() * 86_400)));
        }
        if let Some(max) = p.max {
            picker.setMaximumDate(Some(&date_at_epoch_seconds(max.to_epoch_days() * 86_400)));
        }
        view
    }

    fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        if let Some(p) = (**h).downcast_ref::<UIDatePicker>() {
            set_if_changed(p, d.to_epoch_days() * 86_400);
        }
    }

    fn measure(_backend: &mut Uikit, h: &Retained<UIView>, _p: Proposal) -> Size {
        measure_picker(h)
    }

    day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update, measure: measure);
}

mod time_renderer {
    use super::*;

    fn make(_backend: &mut Uikit, p: &TimeProps, id: NodeId) -> Retained<UIView> {
        // `seconds` is a documented no-op: UIDatePicker has no seconds field (docs/datepicker.md).
        let (picker, view) = make_picker(
            UIDatePickerMode::Time,
            match p.style {
                // iOS has no inline clock face; wheels are its embedded time UI.
                Style::Inline => UIDatePickerStyle::Wheels,
                Style::Automatic | Style::Compact => UIDatePickerStyle::Compact,
            },
            id,
            true,
        );
        picker.setDate(&date_at_epoch_seconds(p.time.seconds_of_day()));
        view
    }

    fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        if let Some(p) = (**h).downcast_ref::<UIDatePicker>() {
            set_if_changed(p, t.seconds_of_day());
        }
    }

    fn measure(_backend: &mut Uikit, h: &Retained<UIView>, _p: Proposal) -> Size {
        measure_picker(h)
    }

    day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update, measure: measure);
}
