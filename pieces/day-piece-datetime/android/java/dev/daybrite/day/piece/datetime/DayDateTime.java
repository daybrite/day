// The datetime piece's OWN Android factory — bundled with the day-piece-datetime crate and pulled
// into the app's Gradle build automatically (via [package.metadata.day.android]), with ZERO edits
// to day-android. Compact = the Material idiom: a value button that launches the modal
// MaterialDatePicker / MaterialTimePicker via DayActivity's FragmentManager (DayActivity extends
// FragmentActivity). Inline = the framework DatePicker (calendar) / TimePicker (clock) widgets.
// Values cross as epoch days / seconds-of-day through DayBridge.nativeOnEvent kind 12 (the open
// Custom-event channel); all civil↔millis math is pinned to UTC so dates never shift by zone.
package dev.daybrite.day.piece.datetime;

import android.view.View;
import android.widget.Button;
import android.widget.DatePicker;
import android.widget.TimePicker;

import androidx.fragment.app.FragmentActivity;
import androidx.fragment.app.FragmentManager;

import com.google.android.material.datepicker.CalendarConstraints;
import com.google.android.material.datepicker.CompositeDateValidator;
import com.google.android.material.datepicker.DateValidatorPointBackward;
import com.google.android.material.datepicker.DateValidatorPointForward;
import com.google.android.material.datepicker.MaterialDatePicker;
import com.google.android.material.timepicker.MaterialTimePicker;
import com.google.android.material.timepicker.TimeFormat;

import java.text.DateFormat;
import java.util.ArrayList;
import java.util.Calendar;
import java.util.Date;
import java.util.List;
import java.util.TimeZone;

import dev.daybrite.day.bridge.DayBridge;

public final class DayDateTime {
    private static final long DAY_MS = 86_400_000L;

    // --- civil ↔ epoch helpers, all in UTC ---------------------------------

    private static long epochDays(int year, int month0, int day) {
        Calendar c = Calendar.getInstance(TimeZone.getTimeZone("UTC"));
        c.clear();
        c.set(year, month0, day);
        return c.getTimeInMillis() / DAY_MS;
    }

    private static Calendar utcCalendar(long epochDays) {
        Calendar c = Calendar.getInstance(TimeZone.getTimeZone("UTC"));
        c.setTimeInMillis(epochDays * DAY_MS);
        return c;
    }

    private static String dateLabel(long epochDays) {
        DateFormat df = DateFormat.getDateInstance(DateFormat.MEDIUM);
        df.setTimeZone(TimeZone.getTimeZone("UTC"));
        return df.format(new Date(epochDays * DAY_MS));
    }

    private static String timeLabel(long secs) {
        DateFormat tf = DateFormat.getTimeInstance(DateFormat.SHORT);
        tf.setTimeZone(TimeZone.getTimeZone("UTC"));
        return tf.format(new Date(secs * 1000L));
    }

    private static FragmentManager fragments() {
        return DayBridge.ctx instanceof FragmentActivity
                ? ((FragmentActivity) DayBridge.ctx).getSupportFragmentManager()
                : null;
    }

    // The piece-shipped time-picker dialog theme (android/res/values/themes.xml): Material's own
    // overlay leaves ?attr/borderlessButtonStyle pointing at an Expressive button style that a
    // plain framework Button (Day's non-AppCompat inflation) cannot resolve — InflateException.
    // Resolved by NAME because the app's R package differs per app; 0 (missing) keeps the default.
    private static int timePickerTheme() {
        return DayBridge.ctx.getResources().getIdentifier(
                "DayPieceDatetimeTimePickerTheme", "style", DayBridge.ctx.getPackageName());
    }

    // --- date picker --------------------------------------------------------

    public static View makeDatePicker(final long id, boolean inline, long epochDays,
                                      final boolean hasMin, final long minDays,
                                      final boolean hasMax, final long maxDays) {
        if (inline) {
            DatePicker dp = new DatePicker(DayBridge.ctx);
            if (hasMin) dp.setMinDate(minDays * DAY_MS);
            if (hasMax) dp.setMaxDate(maxDays * DAY_MS);
            Calendar c = utcCalendar(epochDays);
            dp.init(c.get(Calendar.YEAR), c.get(Calendar.MONTH), c.get(Calendar.DAY_OF_MONTH),
                    (view, y, m, d) -> DayBridge.nativeOnEvent(id, 12, epochDays(y, m, d), null));
            return dp;
        }
        // Compact: a value button that launches the modal MaterialDatePicker (the Material idiom —
        // a DIALOG, not a popover; same gesture contract, platform chrome).
        final Button b = new Button(DayBridge.ctx);
        b.setAllCaps(false);
        b.setTag(epochDays);
        b.setText(dateLabel(epochDays));
        b.setOnClickListener(v -> {
            FragmentManager fm = fragments();
            String tag = "day-datepicker-" + id;
            if (fm == null || fm.findFragmentByTag(tag) != null) return; // guard double-open
            MaterialDatePicker.Builder<Long> builder = MaterialDatePicker.Builder.datePicker()
                    .setSelection(((Long) b.getTag()) * DAY_MS);
            if (hasMin || hasMax) {
                CalendarConstraints.Builder cc = new CalendarConstraints.Builder();
                List<CalendarConstraints.DateValidator> vs = new ArrayList<>();
                if (hasMin) {
                    cc.setStart(minDays * DAY_MS);
                    vs.add(DateValidatorPointForward.from(minDays * DAY_MS));
                }
                if (hasMax) {
                    cc.setEnd(maxDays * DAY_MS);
                    // `before` is exclusive — extend one day so the max day itself stays pickable.
                    vs.add(DateValidatorPointBackward.before((maxDays + 1) * DAY_MS));
                }
                cc.setValidator(CompositeDateValidator.allOf(vs));
                builder.setCalendarConstraints(cc.build());
            }
            MaterialDatePicker<Long> picker = builder.build();
            picker.addOnPositiveButtonClickListener(sel -> {
                long days = sel / DAY_MS;
                b.setTag(days);
                b.setText(dateLabel(days));
                DayBridge.nativeOnEvent(id, 12, days, null);
            });
            picker.show(fm, tag);
        });
        return b;
    }

    public static void setDate(View v, long epochDays) {
        if (v instanceof DatePicker) {
            Calendar c = utcCalendar(epochDays);
            ((DatePicker) v).updateDate(
                    c.get(Calendar.YEAR), c.get(Calendar.MONTH), c.get(Calendar.DAY_OF_MONTH));
        } else if (v instanceof Button) {
            ((Button) v).setTag(epochDays);
            ((Button) v).setText(dateLabel(epochDays));
        }
    }

    // --- time picker --------------------------------------------------------

    public static View makeTimePicker(final long id, boolean inline, long secs) {
        boolean is24h = android.text.format.DateFormat.is24HourFormat(DayBridge.ctx);
        if (inline) {
            TimePicker tp = new TimePicker(DayBridge.ctx);
            tp.setIs24HourView(is24h);
            tp.setHour((int) (secs / 3600));
            tp.setMinute((int) (secs / 60 % 60));
            tp.setOnTimeChangedListener(
                    (view, h, m) -> DayBridge.nativeOnEvent(id, 12, h * 3600L + m * 60L, null));
            return tp;
        }
        final Button b = new Button(DayBridge.ctx);
        b.setAllCaps(false);
        b.setTag(secs);
        b.setText(timeLabel(secs));
        b.setOnClickListener(v -> {
            FragmentManager fm = fragments();
            String tag = "day-timepicker-" + id;
            if (fm == null || fm.findFragmentByTag(tag) != null) return; // guard double-open
            long cur = (Long) b.getTag();
            MaterialTimePicker.Builder builder = new MaterialTimePicker.Builder()
                    .setTimeFormat(is24h ? TimeFormat.CLOCK_24H : TimeFormat.CLOCK_12H)
                    .setHour((int) (cur / 3600))
                    .setMinute((int) (cur / 60 % 60));
            int theme = timePickerTheme();
            if (theme != 0) builder.setTheme(theme);
            MaterialTimePicker picker = builder.build();
            picker.addOnPositiveButtonClickListener(x -> {
                long s = picker.getHour() * 3600L + picker.getMinute() * 60L;
                b.setTag(s);
                b.setText(timeLabel(s));
                DayBridge.nativeOnEvent(id, 12, s, null);
            });
            picker.show(fm, tag);
        });
        return b;
    }

    public static void setTime(View v, long secs) {
        if (v instanceof TimePicker) {
            ((TimePicker) v).setHour((int) (secs / 3600));
            ((TimePicker) v).setMinute((int) (secs / 60 % 60));
        } else if (v instanceof Button) {
            ((Button) v).setTag(secs);
            ((Button) v).setText(timeLabel(secs));
        }
    }
}
