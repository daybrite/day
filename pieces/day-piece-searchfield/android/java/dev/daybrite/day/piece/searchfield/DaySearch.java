// The search-field piece's OWN Android factory — bundled with the day-piece-searchfield crate and
// pulled into the app's Gradle build automatically (via [package.metadata.day.android] →
// day-pieces.json), with ZERO edits to day-android. It uses only day-android's PUBLIC Java surface:
// DayBridge.ctx (the Android Context) and DayBridge.nativeOnEvent (the event trampoline). This is the
// reference pattern for a standalone two-way piece carrying both its front-end (Rust) and backend (Java).
package dev.daybrite.day.piece.searchfield;

import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.View;
import android.view.inputmethod.EditorInfo;
import android.widget.EditText;

import dev.daybrite.day.bridge.DayBridge;

public final class DaySearch {
    // An EditText styled for search (single line, search IME action). Every edit reports back via
    // DayBridge.nativeOnEvent kind 1 (TextChanged), like a built-in text field.
    public static View makeSearch(final long id, String placeholder, String initial) {
        EditText e = new EditText(DayBridge.ctx);
        e.setSingleLine(true);
        e.setInputType(InputType.TYPE_CLASS_TEXT);
        e.setImeOptions(EditorInfo.IME_ACTION_SEARCH);
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

    // Programmatic sync from the bound signal. Guard on equality (a controlled input, §4.4): setting
    // the same text would fire the watcher again for nothing, and setting a new value keeps the caret
    // at the end.
    public static void setSearchText(View v, String text) {
        EditText e = (EditText) v;
        if (!e.getText().toString().equals(text)) {
            e.setText(text);
            e.setSelection(text.length());
        }
    }
}
