// ---------------------------------------------------------------------------
// Qt: this crate's OWN shim (src/lib-qt-shim.cpp) — Compact date = QDateEdit with calendar popup,
// Inline date = QCalendarWidget, time = QTimeEdit (with a seconds field when asked; Qt has no
// inline clock, so both time styles are the sectioned field). Values cross the flat C ABI as
// epoch days / seconds-of-day; the wire's numeric `Event::Custom` path decodes them.
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};

use day_qt::{Qt, QtHandle};
use day_spec::{NodeId, Proposal, Size};

unsafe extern "C" {
    fn day_datetime_date_new(
        inline_style: c_int,
        epoch_days: i64,
        has_min: c_int,
        min_days: i64,
        has_max: c_int,
        max_days: i64,
        id: u64,
        cb: extern "C" fn(u64, i64),
    ) -> *mut c_void;
    fn day_datetime_date_set(w: *mut c_void, epoch_days: i64);
    fn day_datetime_time_new(
        with_seconds: c_int,
        secs: i64,
        id: u64,
        cb: extern "C" fn(u64, i64),
    ) -> *mut c_void;
    fn day_datetime_time_set(w: *mut c_void, secs: i64);
    // From day-qt-sys (already linked into the binary):
    fn day_qt_size_hint(w: *mut c_void, out_w: *mut f64, out_h: *mut f64);
}

fn measure(_backend: &mut Qt, h: &QtHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_qt_size_hint(h.0, &mut w, &mut hh) };
    Size::new(w.max(60.0), hh.max(22.0))
}

mod date_renderer {
    use super::*;

    extern "C" fn on_date(id: u64, epoch_days: i64) {
        day_qt::emit(
            NodeId(id),
            Event::Custom {
                tag: "datepicker:value",
                num: epoch_days as f64,
                text: String::new(),
            },
        );
    }

    fn make(_backend: &mut Qt, p: &DateProps, id: NodeId) -> QtHandle {
        QtHandle(unsafe {
            day_datetime_date_new(
                (p.style == Style::Inline) as c_int,
                p.date.to_epoch_days(),
                p.min.is_some() as c_int,
                p.min.map_or(0, DayDate::to_epoch_days),
                p.max.is_some() as c_int,
                p.max.map_or(0, DayDate::to_epoch_days),
                id.0,
                on_date,
            )
        })
    }

    fn update(_backend: &mut Qt, h: &QtHandle, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        unsafe { day_datetime_date_set(h.0, d.to_epoch_days()) };
    }

    day_pieces::renderer!(day_qt::RENDERERS, Qt,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update, measure: measure);
}

mod time_renderer {
    use super::*;

    extern "C" fn on_time(id: u64, secs: i64) {
        day_qt::emit(
            NodeId(id),
            Event::Custom {
                tag: "timepicker:value",
                num: secs as f64,
                text: String::new(),
            },
        );
    }

    fn make(_backend: &mut Qt, p: &TimeProps, id: NodeId) -> QtHandle {
        QtHandle(unsafe {
            day_datetime_time_new(p.seconds as c_int, p.time.seconds_of_day(), id.0, on_time)
        })
    }

    fn update(_backend: &mut Qt, h: &QtHandle, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        unsafe { day_datetime_time_set(h.0, t.seconds_of_day()) };
    }

    day_pieces::renderer!(day_qt::RENDERERS, Qt,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update, measure: measure);
}
