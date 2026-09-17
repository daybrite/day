// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The datetime piece's C++/WinRT shim, parallel to src/lib-qt-shim.cpp. Compact date =
// CalendarDatePicker (button → calendar flyout); inline date = CalendarView; time = TimePicker
// flyout for both styles (XAML has no inline clock; a documented fallback, docs/datepicker.md).
// Dates cross the flat C ABI as a civil y/m/d triple (DayCivilDate) and times as seconds-of-day,
// so no epoch arithmetic happens on this side; the zone-dependent step — a civil day becomes an
// instant at local noon — is walked by Windows.Globalization.Calendar (see toDateTime). Elements
// are boxed into Day handles via
// the day_xaml_box/day_xaml_unbox functions day-xaml-sys exports, with zero edits to day's
// toolkit crates. Windows-only; compiled by build.rs, built in CI, not verified locally.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h> // IVector/IObservableVector methods; else C3779
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>

#include <winrt/Windows.Globalization.h> // Calendar — the civil⇄instant bridge, see fromEpochDays

#include <cstdint>
#include <vector> // the Calendar constructor's language list

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;
namespace WG = winrt::Windows::Globalization;

// The boxing functions, exported by day-xaml-sys (already linked into the app).
extern "C" void *day_xaml_box(void *iinspectable_abi);
extern "C" void *day_xaml_unbox(void *handle);
static constexpr int64_t TICKS_PER_SECOND = 10000000LL; // 100 ns ticks

// A civil date as it crosses the ABI: the three numbers a calendar names a day with. Deliberately
// NOT epoch days — a day number is not an instant until some zone says so, and the Rust side
// already owns that arithmetic (`DayDate`), so sending days would only mean undoing it here.
struct DayCivilDate {
    int32_t year;
    int32_t month; // 1-12
    int32_t day;   // 1-31
};

// `CalendarDatePicker`/`CalendarView` take a `DateTime` — a FILETIME, i.e. an instant in UTC — and
// render it through a `Windows.Globalization.Calendar` pinned to the VIEWER'S zone. Neither
// control exposes a TimeZone to redirect that (checked against the SDK headers), so the trick the
// other backends use is unavailable here: the AppKit arm pins NSDatePicker's own calendar/timeZone
// to GMT, Android pins its civil↔millis math to UTC, and Qt/GTK sidestep it entirely by speaking
// civil dates that never involve an instant. Hence this pair — but the walking is done by the
// platform's calendar, the same engine XAML renders with, not by arithmetic of ours.
//
// The anchor is local NOON. Midnight does not exist on DST-transition days in real zones (Brazil,
// Cuba, Iran and Chile have all jumped 23:59 → 01:00), where a midnight anchor lands on the
// adjacent civil day — the very bug this exists to prevent. Noon is unambiguous everywhere, and
// the controls display only the date part, so the hour is never seen.
static WG::Calendar localGregorian() {
    static const auto kLang = std::vector<winrt::hstring>{L"en-US"}; // numeric fields only, so the
    return WG::Calendar(kLang, WG::CalendarIdentifiers::Gregorian(), // language never shows
                        WG::ClockIdentifiers::TwentyFourHour());
}

static WF::DateTime toDateTime(DayCivilDate c) {
    auto cal = localGregorian();
    cal.Year(c.year);
    cal.Month(c.month);
    cal.Day(c.day);
    cal.Period(1); // one period on a 24-hour clock
    cal.Hour(12);
    cal.Minute(0);
    cal.Second(0);
    cal.Nanosecond(0);
    return cal.GetDateTime();
}

static DayCivilDate toCivil(WF::DateTime dt) {
    auto cal = localGregorian();
    cal.SetDateTime(dt);
    return DayCivilDate{cal.Year(), cal.Month(), cal.Day()};
}

static bool sameCivil(DayCivilDate a, DayCivilDate b) {
    return a.year == b.year && a.month == b.month && a.day == b.day;
}

extern "C" {

void *day_datetime_xaml_date_new(int inline_style, DayCivilDate date, int has_min,
                                  DayCivilDate min, int has_max, DayCivilDate max, uint64_t id,
                                  void (*cb)(uint64_t, int32_t, int32_t, int32_t)) {
    WF::DateTime value = toDateTime(date);
    if (inline_style) {
        WUXC::CalendarView cv;
        cv.SelectionMode(WUXC::CalendarViewSelectionMode::Single);
        if (has_min)
            cv.MinDate(toDateTime(min));
        if (has_max)
            cv.MaxDate(toDateTime(max));
        cv.SelectedDates().Append(value);
        cv.SetDisplayDate(value);
        cv.SelectedDatesChanged([id, cb](WUXC::CalendarView const &,
                                         WUXC::CalendarViewSelectedDatesChangedEventArgs const &args) {
            auto added = args.AddedDates();
            if (added.Size() > 0) {
                DayCivilDate c = toCivil(added.GetAt(0));
                cb(id, c.year, c.month, c.day);
            }
        });
        return day_xaml_box(winrt::get_abi(cv));
    }
    WUXC::CalendarDatePicker p;
    if (has_min)
        p.MinDate(toDateTime(min));
    if (has_max)
        p.MaxDate(toDateTime(max));
    p.Date(value);
    p.DateChanged([id, cb](WUXC::CalendarDatePicker const &,
                           WUXC::CalendarDatePickerDateChangedEventArgs const &args) {
        auto d = args.NewDate();
        if (d) { // null = cleared; the piece keeps the last real pick
            DayCivilDate c = toCivil(d.Value());
            cb(id, c.year, c.month, c.day);
        }
    });
    return day_xaml_box(winrt::get_abi(p));
}

void day_datetime_xaml_date_set(void *handle, DayCivilDate date) {
    WF::IInspectable e{nullptr};
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    WF::DateTime value = toDateTime(date);
    // Compare as CIVIL dates, not as instants: two DateTimes for the same day differ whenever the
    // anchor moves (and the control hands back its own, not ours), so an instant comparison would
    // re-set the control on every update and echo a spurious change back through the callback.
    if (auto p = e.try_as<WUXC::CalendarDatePicker>()) {
        auto cur = p.Date();
        if (!cur || !sameCivil(toCivil(cur.Value()), date))
            p.Date(value);
        return;
    }
    if (auto cv = e.try_as<WUXC::CalendarView>()) {
        auto sel = cv.SelectedDates();
        if (sel.Size() == 1 && sameCivil(toCivil(sel.GetAt(0)), date))
            return;
        sel.Clear();
        sel.Append(value);
        cv.SetDisplayDate(value);
    }
}

void *day_datetime_xaml_time_new(int64_t secs, uint64_t id, void (*cb)(uint64_t, int64_t)) {
    WUXC::TimePicker p;
    p.Time(WF::TimeSpan{secs * TICKS_PER_SECOND});
    p.TimeChanged([id, cb](WF::IInspectable const &,
                           WUXC::TimePickerValueChangedEventArgs const &args) {
        cb(id, args.NewTime().count() / TICKS_PER_SECOND);
    });
    return day_xaml_box(winrt::get_abi(p));
}

void day_datetime_xaml_time_set(void *handle, int64_t secs) {
    WF::IInspectable e{nullptr};
    winrt::copy_from_abi(e, day_xaml_unbox(handle));
    if (auto p = e.try_as<WUXC::TimePicker>()) {
        // The system-XAML TimePicker has only minute resolution (no seconds column), so re-display
        // only when the minute changes. Re-setting it for a seconds-only delta would round to the
        // minute and echo that back via TimeChanged, dropping the sub-minute part the signal holds.
        int64_t cur = p.Time().count() / TICKS_PER_SECOND;
        if (cur / 60 != secs / 60)
            p.Time(WF::TimeSpan{secs * TICKS_PER_SECOND});
    }
}

} // extern "C"
