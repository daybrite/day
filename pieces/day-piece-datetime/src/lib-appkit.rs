// ---------------------------------------------------------------------------
// AppKit: NSDatePicker for both pieces — Compact → textFieldAndStepper, Inline → clockAndCalendar
// (the graphical month grid / analog clock); date-only resp. time-only element flags (seconds adds
// HourMinuteSecond). The control's calendar/timeZone are pinned to proleptic-Gregorian GMT so Day's
// civil DayDate/DayTime map 1:1 onto NSDate epoch seconds regardless of the user's zone — the
// LOCALE stays the user's, so month/weekday names render localized.
// ---------------------------------------------------------------------------

use super::*;
use std::cell::RefCell;
use std::collections::HashMap;

use day_appkit::AppKit;
use day_spec::{NodeId, Proposal, Size};
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSDatePicker, NSDatePickerElementFlags, NSDatePickerMode, NSDatePickerStyle, NSView,
};
use objc2_foundation::{NSCalendar, NSCalendarIdentifierGregorian, NSDate, NSObject, NSTimeZone};

struct TargetIvars {
    node: NodeId,
    /// `true` → this target belongs to a time picker (report seconds-of-day, not epoch days).
    time: bool,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayDateTimeTarget"]
    #[ivars = TargetIvars]
    struct DateTimeTarget;

    unsafe impl NSObjectProtocol for DateTimeTarget {}

    impl DateTimeTarget {
        #[unsafe(method(fire:))]
        fn fire(&self, sender: &AnyObject) {
            let Some(p) = sender.downcast_ref::<NSDatePicker>() else {
                return;
            };
            let ti = p.dateValue().timeIntervalSince1970();
            let iv = self.ivars();
            if iv.time {
                let t = DayTime::from_seconds_of_day(ti.rem_euclid(86_400.0).round() as i64);
                day_appkit::emit(iv.node, Event::custom("timepicker:value", t.to_string()));
            } else {
                let d = DayDate::from_epoch_days((ti / 86_400.0).floor() as i64);
                day_appkit::emit(iv.node, Event::custom("datepicker:value", d.to_string()));
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

/// A shared NSDatePicker skeleton pinned to Gregorian GMT (user locale kept for display).
fn make_picker(
    mtm: MainThreadMarker,
    style: Style,
    elements: NSDatePickerElementFlags,
    node: NodeId,
    time: bool,
) -> (Retained<NSDatePicker>, Retained<NSView>) {
    let p = NSDatePicker::new(mtm);
    p.setDatePickerMode(NSDatePickerMode::Single);
    p.setDatePickerStyle(match style {
        Style::Inline => NSDatePickerStyle::ClockAndCalendar,
        Style::Automatic | Style::Compact => NSDatePickerStyle::TextFieldAndStepper,
    });
    p.setDatePickerElements(elements);
    let gmt = NSTimeZone::timeZoneForSecondsFromGMT(0);
    p.setTimeZone(Some(&gmt));
    if let Some(cal) = NSCalendar::calendarWithIdentifier(unsafe { NSCalendarIdentifierGregorian })
    {
        cal.setTimeZone(&gmt);
        p.setCalendar(Some(&cal));
    }
    let target = DateTimeTarget::new(mtm, node, time);
    unsafe {
        p.setTarget(Some(&target));
        p.setAction(Some(sel!(fire:)));
    }
    let view: Retained<NSView> = Retained::from(<NSDatePicker as AsRef<NSView>>::as_ref(&p));
    TARGETS.with(|m| {
        m.borrow_mut()
            .insert((view.as_ref() as *const NSView) as usize, target)
    });
    (p, view)
}

fn set_if_changed(p: &NSDatePicker, secs: i64) {
    if p.dateValue().timeIntervalSince1970() as i64 != secs {
        p.setDateValue(&date_at_epoch_seconds(secs));
    }
}

fn measure_picker(h: &Retained<NSView>) -> Size {
    let s = h.fittingSize();
    Size::new(s.width.ceil().max(60.0), s.height.ceil().max(22.0))
}

mod date_renderer {
    use super::*;

    fn make(backend: &mut AppKit, p: &DateProps, id: NodeId) -> Retained<NSView> {
        let (picker, view) = make_picker(
            backend.mtm(),
            p.style,
            NSDatePickerElementFlags::YearMonthDay,
            id,
            false,
        );
        picker.setDateValue(&date_at_epoch_seconds(p.date.to_epoch_days() * 86_400));
        if let Some(min) = p.min {
            picker.setMinDate(Some(&date_at_epoch_seconds(min.to_epoch_days() * 86_400)));
        }
        if let Some(max) = p.max {
            picker.setMaxDate(Some(&date_at_epoch_seconds(max.to_epoch_days() * 86_400)));
        }
        view
    }

    fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        if let Some(p) = h.downcast_ref::<NSDatePicker>() {
            set_if_changed(p, d.to_epoch_days() * 86_400);
        }
    }

    fn measure(_backend: &mut AppKit, h: &Retained<NSView>, _p: Proposal) -> Size {
        measure_picker(h)
    }

    day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update, measure: measure);
}

mod time_renderer {
    use super::*;

    fn make(backend: &mut AppKit, p: &TimeProps, id: NodeId) -> Retained<NSView> {
        let (picker, view) = make_picker(
            backend.mtm(),
            p.style,
            if p.seconds {
                NSDatePickerElementFlags::HourMinuteSecond
            } else {
                NSDatePickerElementFlags::HourMinute
            },
            id,
            true,
        );
        picker.setDateValue(&date_at_epoch_seconds(p.time.seconds_of_day()));
        view
    }

    fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        if let Some(p) = h.downcast_ref::<NSDatePicker>() {
            set_if_changed(p, t.seconds_of_day());
        }
    }

    fn measure(_backend: &mut AppKit, h: &Retained<NSView>, _p: Proposal) -> Size {
        measure_picker(h)
    }

    day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update, measure: measure);
}
