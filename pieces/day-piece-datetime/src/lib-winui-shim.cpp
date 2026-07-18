// The datetime piece's OWN C++/WinRT shim — parallel to src/lib-qt-shim.cpp. Compact date =
// CalendarDatePicker (button → calendar flyout); inline date = CalendarView; time = TimePicker
// flyout for BOTH styles (WinUI has no inline clock — documented fallback, docs/datepicker.md).
// Values cross the flat C ABI as epoch days / seconds-of-day; DateTime conversion pins to the
// Windows 1601 epoch offset so civil dates never shift. Elements are boxed into Day handles via
// the day_winui_box/day_winui_unbox seam day-winui-sys exports — zero edits to day's toolkit
// crates. Windows-only; compiled by build.rs, built in CI, not verified locally.

#include <winrt/Windows.Foundation.h>
#include <winrt/Windows.Foundation.Collections.h> // IVector/IObservableVector methods — else C3779
#include <winrt/Windows.UI.Xaml.h>
#include <winrt/Windows.UI.Xaml.Controls.h>
#include <winrt/Windows.UI.Xaml.Controls.Primitives.h>

#include <cstdint>

using namespace winrt;
namespace WF = winrt::Windows::Foundation;
namespace WUXC = winrt::Windows::UI::Xaml::Controls;

// The boxing seam, exported by day-winui-sys (already linked into the app).
extern "C" void *day_winui_box(void *iinspectable_abi);
extern "C" void *day_winui_unbox(void *handle);

// Seconds between the Windows epoch (1601-01-01) and the Unix epoch (1970-01-01).
static constexpr int64_t EPOCH_1601_TO_1970 = 11644473600LL;
static constexpr int64_t TICKS_PER_SECOND = 10000000LL; // 100 ns ticks

static WF::DateTime fromEpochDays(int64_t days) {
    return WF::DateTime{
        WF::TimeSpan{(days * 86400 + EPOCH_1601_TO_1970) * TICKS_PER_SECOND}};
}

static int64_t toEpochDays(WF::DateTime dt) {
    int64_t secs = dt.time_since_epoch().count() / TICKS_PER_SECOND - EPOCH_1601_TO_1970;
    // Floor division: pre-1970 dates land on their own day, not the next one.
    return (secs >= 0 ? secs : secs - 86399) / 86400;
}

extern "C" {

void *day_datetime_winui_date_new(int inline_style, int64_t days, int has_min, int64_t min_days,
                                  int has_max, int64_t max_days, uint64_t id,
                                  void (*cb)(uint64_t, int64_t)) {
    WF::DateTime value = fromEpochDays(days);
    if (inline_style) {
        WUXC::CalendarView cv;
        cv.SelectionMode(WUXC::CalendarViewSelectionMode::Single);
        if (has_min)
            cv.MinDate(fromEpochDays(min_days));
        if (has_max)
            cv.MaxDate(fromEpochDays(max_days));
        cv.SelectedDates().Append(value);
        cv.SetDisplayDate(value);
        cv.SelectedDatesChanged([id, cb](WUXC::CalendarView const &,
                                         WUXC::CalendarViewSelectedDatesChangedEventArgs const &args) {
            auto added = args.AddedDates();
            if (added.Size() > 0)
                cb(id, toEpochDays(added.GetAt(0)));
        });
        return day_winui_box(winrt::get_abi(cv));
    }
    WUXC::CalendarDatePicker p;
    if (has_min)
        p.MinDate(fromEpochDays(min_days));
    if (has_max)
        p.MaxDate(fromEpochDays(max_days));
    p.Date(value);
    p.DateChanged([id, cb](WUXC::CalendarDatePicker const &,
                           WUXC::CalendarDatePickerDateChangedEventArgs const &args) {
        auto d = args.NewDate();
        if (d) // null = cleared; the piece keeps the last real pick
            cb(id, toEpochDays(d.Value()));
    });
    return day_winui_box(winrt::get_abi(p));
}

void day_datetime_winui_date_set(void *handle, int64_t days) {
    WF::IInspectable e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
    WF::DateTime value = fromEpochDays(days);
    if (auto p = e.try_as<WUXC::CalendarDatePicker>()) {
        auto cur = p.Date();
        if (!cur || toEpochDays(cur.Value()) != days)
            p.Date(value);
        return;
    }
    if (auto cv = e.try_as<WUXC::CalendarView>()) {
        auto sel = cv.SelectedDates();
        if (sel.Size() == 1 && toEpochDays(sel.GetAt(0)) == days)
            return;
        sel.Clear();
        sel.Append(value);
        cv.SetDisplayDate(value);
    }
}

void *day_datetime_winui_time_new(int64_t secs, uint64_t id, void (*cb)(uint64_t, int64_t)) {
    WUXC::TimePicker p;
    p.Time(WF::TimeSpan{secs * TICKS_PER_SECOND});
    p.TimeChanged([id, cb](WF::IInspectable const &,
                           WUXC::TimePickerValueChangedEventArgs const &args) {
        cb(id, args.NewTime().count() / TICKS_PER_SECOND);
    });
    return day_winui_box(winrt::get_abi(p));
}

void day_datetime_winui_time_set(void *handle, int64_t secs) {
    WF::IInspectable e{nullptr};
    winrt::copy_from_abi(e, day_winui_unbox(handle));
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
