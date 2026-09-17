// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: this crate's C++/WinRT shim (src/lib-xaml-shim.cpp). Compact date =
// CalendarDatePicker (button → calendar flyout), Inline date = CalendarView, time = TimePicker
// flyout for both styles (XAML has no inline clock; a documented fallback). Boxed into Day handles
// via the day_xaml_box/day_xaml_unbox functions day-xaml-sys exports, mirroring the picker piece.
// Windows-only, built in CI, not verified locally.
// ---------------------------------------------------------------------------

use super::*;
use std::os::raw::{c_int, c_void};

use day_spec::{NodeId, Proposal, Size};
use day_xaml::{WinHandle, Xaml};

/// A civil date as it crosses to the shim: the three numbers a calendar names a day with.
///
/// Epoch days stop at this boundary. XAML's calendar controls take an INSTANT and render it in the
/// viewer's zone, so a day number would have to be turned back into y/m/d over there before it
/// could be anchored — arithmetic `DayDate` already owns, done twice. Qt and GTK speak civil dates
/// to their toolkits for the same reason; XAML is only unusual in needing the anchoring step at all
/// (see the shim's `toDateTime`).
#[repr(C)]
#[derive(Clone, Copy)]
struct CivilDate {
    year: i32,
    month: i32,
    day: i32,
}

impl CivilDate {
    fn of(d: DayDate) -> Self {
        CivilDate { year: d.year, month: d.month as i32, day: d.day as i32 }
    }
    /// The filler for an absent min/max, which the `has_*` flag tells the shim to ignore.
    const UNSET: Self = CivilDate { year: 0, month: 0, day: 0 };
}

unsafe extern "C" {
    fn day_datetime_xaml_date_new(
        inline_style: c_int,
        date: CivilDate,
        has_min: c_int,
        min: CivilDate,
        has_max: c_int,
        max: CivilDate,
        id: u64,
        cb: extern "C" fn(u64, i32, i32, i32),
    ) -> *mut c_void;
    fn day_datetime_xaml_date_set(h: *mut c_void, date: CivilDate);
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

    extern "C" fn on_date(id: u64, year: i32, month: i32, day: i32) {
        // The event stays epoch days: that is the piece's cross-backend contract
        // (`Event::Custom.num`), and only the ABI below it changed. `DayDate::new` validates
        // rather than trusting the boundary — a triple that is not a real calendar day is dropped
        // instead of becoming a wrong date.
        let Some(d) = DayDate::new(year, month as u8, day as u8) else {
            return;
        };
        day_xaml::emit(
            NodeId(id),
            Event::Custom {
                tag: "datepicker:value",
                num: d.to_epoch_days() as f64,
                text: String::new(),
            },
        );
    }

    fn make(_backend: &mut Xaml, p: &DateProps, id: NodeId) -> WinHandle {
        WinHandle(unsafe {
            day_datetime_xaml_date_new(
                (p.style == Style::Inline) as c_int,
                CivilDate::of(p.date),
                p.min.is_some() as c_int,
                p.min.map_or(CivilDate::UNSET, CivilDate::of),
                p.max.is_some() as c_int,
                p.max.map_or(CivilDate::UNSET, CivilDate::of),
                id.0,
                on_date,
            )
        })
    }

    fn update(_backend: &mut Xaml, h: &WinHandle, patch: &DatePatch) {
        let DatePatch::SetDate(d) = patch;
        unsafe { day_datetime_xaml_date_set(h.0, CivilDate::of(*d)) };
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
