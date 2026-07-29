// ---------------------------------------------------------------------------
// XAML: this crate's OWN C++/WinRT shim (src/lib-xaml-shim.cpp) — Compact date =
// CalendarDatePicker (button → calendar flyout), Inline date = CalendarView, time = TimePicker
// flyout for BOTH styles (XAML has no inline clock — documented fallback). Boxed into Day handles
// via the day_xaml_box/day_xaml_unbox seam day-xaml-sys exports, mirroring the picker piece.
// Windows-only, built in CI, not verified locally.
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_xaml::{WinHandle, Xaml};

unsafe extern "C" {
    fn day_datetime_xaml_date_new(
        inline_style: c_int,
        days: i64,
        has_min: c_int,
        min_days: i64,
        has_max: c_int,
        max_days: i64,
        id: u64,
        cb: extern "C" fn(u64, i64),
    ) -> *mut c_void;
    fn day_datetime_xaml_date_set(h: *mut c_void, days: i64);
    fn day_datetime_xaml_time_new(secs: i64, id: u64, cb: extern "C" fn(u64, i64)) -> *mut c_void;
    fn day_datetime_xaml_time_set(h: *mut c_void, secs: i64);
    // Generic size hint from day-xaml-sys (already linked).
    fn day_xaml_measure(
        w: *mut c_void,
        avail_w: f64,
        avail_h: f64,
        out_w: *mut f64,
        out_h: *mut f64,
    );
}

fn measure(_backend: &mut Xaml, h: &WinHandle, _p: Proposal) -> Size {
    let mut w = 0.0;
    let mut hh = 0.0;
    unsafe { day_xaml_measure(h.0, -1.0, -1.0, &mut w, &mut hh) };
    Size::new(w.max(120.0), hh.max(32.0))
}

mod date_renderer {
    use super::*;

    extern "C" fn on_date(id: u64, days: i64) {
        day_xaml::emit(
            NodeId(id),
            Event::Custom {
                tag: "datepicker:value",
                num: days as f64,
                text: String::new(),
            },
        );
    }

    fn make(_backend: &mut Xaml, p: &DateProps, id: NodeId) -> WinHandle {
        WinHandle(unsafe {
            day_datetime_xaml_date_new(
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

    fn update(_backend: &mut Xaml, h: &WinHandle, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        unsafe { day_datetime_xaml_date_set(h.0, d.to_epoch_days()) };
    }

    day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
        kind: DATE_KIND, props: DateProps, patch: DatePatch,
        make: make, update: update, measure: measure);
}

mod time_renderer {
    use super::*;

    extern "C" fn on_time(id: u64, secs: i64) {
        day_xaml::emit(
            NodeId(id),
            Event::Custom {
                tag: "timepicker:value",
                num: secs as f64,
                text: String::new(),
            },
        );
    }

    fn make(_backend: &mut Xaml, p: &TimeProps, id: NodeId) -> WinHandle {
        // `seconds` is a documented no-op: TimePicker edits hours/minutes only.
        WinHandle(unsafe { day_datetime_xaml_time_new(p.time.seconds_of_day(), id.0, on_time) })
    }

    fn update(_backend: &mut Xaml, h: &WinHandle, patch: &TimePatch) {
        let TimePatch::SetTime(t) = patch;
        unsafe { day_datetime_xaml_time_set(h.0, t.seconds_of_day()) };
    }

    day_pieces::renderer!(day_xaml::RENDERERS, Xaml,
        kind: TIME_KIND, props: TimeProps, patch: TimePatch,
        make: make, update: update, measure: measure);
}
