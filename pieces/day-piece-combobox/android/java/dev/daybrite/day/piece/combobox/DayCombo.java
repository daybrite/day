// The combo-box piece's OWN Android factory — bundled with the day-piece-combobox crate and
// pulled into the app's Gradle build automatically (via [package.metadata.day.android] →
// day-pieces.json), with ZERO edits to day-android. It uses only day-android's PUBLIC Java
// surface: DayBridge.ctx (the Android Context), DayBridge.nativeOnEvent (the event trampoline),
// and the K_* event-kind constants.
//
// Android's real combo box is AutoCompleteTextView: free-form text plus a dropdown of
// suggestions — prefix-filtered while typing, and popped open on a plain tap or focus so the
// list is reachable without typing (the combo half). Picking an item writes the text, so BOTH
// change paths report through the one TextWatcher as K_TEXT_CHANGED.
package dev.daybrite.day.piece.combobox;

import android.text.Editable;
import android.text.InputType;
import android.text.TextWatcher;
import android.view.View;
import android.widget.ArrayAdapter;
import android.widget.AutoCompleteTextView;

import dev.daybrite.day.bridge.DayBridge;

public final class DayCombo {
    public static View makeCombo(final long id, String itemsJoined, String text, String placeholder) {
        AutoCompleteTextView v = new AutoCompleteTextView(DayBridge.ctx);
        v.setSingleLine(true);
        v.setInputType(InputType.TYPE_CLASS_TEXT);
        v.setHint(placeholder);
        v.setThreshold(1);
        v.setAdapter(adapter(itemsJoined));
        if (text != null && !text.isEmpty()) {
            v.setText(text);
            v.setSelection(text.length());
        }
        // The combo half: the dropdown opens on tap/focus, not only after typing.
        v.setOnClickListener(w -> showList((AutoCompleteTextView) w));
        v.setOnFocusChangeListener((w, has) -> {
            if (has) showList((AutoCompleteTextView) w);
        });
        v.addTextChangedListener(new TextWatcher() {
            public void afterTextChanged(Editable s) {
                DayBridge.nativeOnEvent(id, DayBridge.K_TEXT_CHANGED, 0, s.toString());
            }
            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            public void onTextChanged(CharSequence s, int a, int b, int c) {}
        });
        return v;
    }

    public static void setComboItems(View view, String itemsJoined) {
        ((AutoCompleteTextView) view).setAdapter(adapter(itemsJoined));
    }

    // Programmatic sync from the bound signal. Guard on equality (a controlled input, §4.4):
    // setting the same text would fire the watcher again for nothing, and setting a new value
    // keeps the caret at the end.
    public static void setComboText(View view, String text) {
        AutoCompleteTextView v = (AutoCompleteTextView) view;
        if (!v.getText().toString().equals(text)) {
            v.setText(text);
            v.setSelection(text.length());
        }
    }

    private static void showList(AutoCompleteTextView v) {
        if (v.getAdapter() != null && v.getAdapter().getCount() > 0 && v.isAttachedToWindow()) {
            v.showDropDown();
        }
    }

    private static ArrayAdapter<String> adapter(String itemsJoined) {
        String[] items = (itemsJoined == null || itemsJoined.isEmpty())
                ? new String[0]
                : itemsJoined.split("\n", -1);
        return new ArrayAdapter<>(DayBridge.ctx, android.R.layout.simple_dropdown_item_1line, items);
    }
}
