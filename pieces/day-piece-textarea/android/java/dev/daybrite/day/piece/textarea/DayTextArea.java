// The textarea piece's OWN Android factory — bundled with the day-piece-textarea crate and pulled into
// the app's Gradle build automatically (via [package.metadata.day.android] → day-pieces.json), with ZERO
// edits to day-android. It uses only day-android's PUBLIC Java surface: DayBridge.ctx (the Android
// Context) and DayBridge.nativeOnEvent (the event trampoline). A multi-line EditText that grows between
// minLines and maxLines and scrolls internally past maxLines — the Android reference for a message
// composer field.
package dev.daybrite.day.piece.textarea;

import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.Gravity;
import android.view.View;
import android.widget.EditText;

import dev.daybrite.day.bridge.DayBridge;

public final class DayTextArea {
    // A multi-line EditText (capitalize sentences), top-aligned, that grows from minLines to maxLines and
    // then scrolls. Every edit reports back via DayBridge.nativeOnEvent kind 1 (TextChanged), like a
    // built-in text field. `maxLines == 0` means unbounded (grow with content, never scroll).
    public static View makeTextArea(final long id, String placeholder, String initial,
                                    int minLines, int maxLines) {
        EditText e = new EditText(DayBridge.ctx);
        e.setInputType(InputType.TYPE_CLASS_TEXT
                | InputType.TYPE_TEXT_FLAG_MULTI_LINE
                | InputType.TYPE_TEXT_FLAG_CAP_SENTENCES);
        e.setGravity(Gravity.TOP | Gravity.START);
        e.setHorizontallyScrolling(false);
        e.setVerticalScrollBarEnabled(true);
        e.setMinLines(Math.max(1, minLines));
        e.setMaxLines(maxLines > 0 ? maxLines : Integer.MAX_VALUE);
        e.setHint(placeholder);
        if (initial != null && !initial.isEmpty()) {
            e.setText(initial);
            e.setSelection(initial.length());
        }
        e.addTextChangedListener(new TextWatcher() {
            public void afterTextChanged(Editable s) {
                DayBridge.nativeOnEvent(id, 1, 0, s.toString());
            }
            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            public void onTextChanged(CharSequence s, int a, int b, int c) {}
        });
        return e;
    }

    // Programmatic sync from the bound signal. Guard on equality (a controlled input, §4.4): setting the
    // same text would fire the watcher again for nothing, and setting a new value keeps the caret at end.
    public static void setTextAreaText(View v, String text) {
        EditText e = (EditText) v;
        if (!e.getText().toString().equals(text)) {
            e.setText(text);
            e.setSelection(text.length());
        }
    }

    // Content-driven height for the proposed width, in dp (density-independent points — day works in dp,
    // so the density conversion happens here). The EditText's own onMeasure honors minLines/maxLines, so
    // the result is already clamped to the growing band.
    public static int measureHeight(View v, int wDp) {
        float dens = v.getResources().getDisplayMetrics().density;
        int wPx = Math.round(wDp * dens);
        v.measure(View.MeasureSpec.makeMeasureSpec(wPx, View.MeasureSpec.EXACTLY),
                  View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED));
        return Math.round(v.getMeasuredHeight() / dens);
    }
}
