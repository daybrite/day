// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The picker piece's OWN Android factory — bundled with the day-piece-picker crate and pulled into
// the app's Gradle build automatically (via [package.metadata.day.android] → day-pieces.json), with
// ZERO edits to day-android. It uses only day-android's PUBLIC Java surface: DayBridge.ctx (the
// Android Context) and DayBridge.nativeOnEvent (the event trampoline). This is the reference pattern
// for a standalone piece that carries both its front-end (Rust) and its backend (Java) toolkit code.
package dev.daybrite.day.piece.picker;

import android.view.View;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.LinearLayout;
import android.widget.RadioButton;
import android.widget.RadioGroup;
import android.widget.Spinner;

import dev.daybrite.day.bridge.DayBridge;

public final class DayPicker {
    // style 0 = menu (Spinner), 1 = segmented (button row), 2 = inline (RadioGroup). All report
    // selection via DayBridge.nativeOnEvent kind 4 (SelectionChanged), like any built-in.
    public static View makePicker(final long id, int style, String joinedItems, int selected) {
        String[] items = joinedItems.isEmpty() ? new String[0] : joinedItems.split("\n");
        if (style == 0) {
            Spinner sp = new Spinner(DayBridge.ctx);
            ArrayAdapter<String> ad = new ArrayAdapter<>(
                    DayBridge.ctx, android.R.layout.simple_spinner_item, items);
            ad.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
            sp.setAdapter(ad);
            if (selected >= 0 && selected < items.length) sp.setSelection(selected);
            final int[] fired = {0};
            sp.setOnItemSelectedListener(new android.widget.AdapterView.OnItemSelectedListener() {
                public void onItemSelected(android.widget.AdapterView<?> p, View v, int pos, long i) {
                    if (fired[0]++ > 0) DayBridge.nativeOnEvent(id, 4, pos, null);
                }
                public void onNothingSelected(android.widget.AdapterView<?> p) {}
            });
            return sp;
        } else if (style == 1) {
            LinearLayout row = new LinearLayout(DayBridge.ctx);
            row.setOrientation(LinearLayout.HORIZONTAL);
            for (int i = 0; i < items.length; i++) {
                final int idx = i;
                Button b = new Button(DayBridge.ctx);
                b.setText(items[i]);
                b.setAllCaps(false);
                b.setOnClickListener(new View.OnClickListener() {
                    public void onClick(View x) {
                        selectSegment(row, idx);
                        DayBridge.nativeOnEvent(id, 4, idx, null);
                    }
                });
                row.addView(b);
            }
            selectSegment(row, selected);
            return row;
        } else {
            RadioGroup g = new RadioGroup(DayBridge.ctx);
            for (int i = 0; i < items.length; i++) {
                RadioButton rb = new RadioButton(DayBridge.ctx);
                rb.setText(items[i]);
                rb.setId(i + 1); // 0 is "no id"; offset by 1
                g.addView(rb);
            }
            if (selected >= 0 && selected < items.length) g.check(selected + 1);
            g.setOnCheckedChangeListener(new RadioGroup.OnCheckedChangeListener() {
                public void onCheckedChanged(RadioGroup grp, int checkedId) {
                    if (checkedId > 0) DayBridge.nativeOnEvent(id, 4, checkedId - 1, null);
                }
            });
            return g;
        }
    }

    static void selectSegment(LinearLayout row, int sel) {
        for (int i = 0; i < row.getChildCount(); i++) {
            View c = row.getChildAt(i);
            c.setSelected(i == sel);
            c.setAlpha(i == sel ? 1.0f : 0.55f); // dim the unselected segments
        }
    }

    // New option labels, in place (PickerPatch::Options). The spinner swaps its adapter; the
    // button row and the radio group relabel what they have and add or drop the tail, so a
    // listener that captured its own index keeps working. The selection survives where the
    // index still exists.
    public static void setPickerOptions(View v, long id, String joinedItems) {
        String[] items = joinedItems.isEmpty() ? new String[0] : joinedItems.split("\n");
        if (v instanceof Spinner) {
            Spinner sp = (Spinner) v;
            int keep = Math.max(sp.getSelectedItemPosition(), 0);
            ArrayAdapter<String> ad = new ArrayAdapter<>(
                    DayBridge.ctx, android.R.layout.simple_spinner_item, items);
            ad.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
            sp.setAdapter(ad);
            if (items.length > 0) sp.setSelection(Math.min(keep, items.length - 1));
        } else if (v instanceof RadioGroup) {
            RadioGroup g = (RadioGroup) v;
            int keep = Math.max(g.getCheckedRadioButtonId() - 1, 0);
            for (int i = 0; i < items.length; i++) {
                if (i < g.getChildCount()) {
                    ((RadioButton) g.getChildAt(i)).setText(items[i]);
                } else {
                    RadioButton rb = new RadioButton(DayBridge.ctx);
                    rb.setText(items[i]);
                    rb.setId(i + 1);
                    g.addView(rb);
                }
            }
            while (g.getChildCount() > items.length) g.removeViewAt(g.getChildCount() - 1);
            if (items.length > 0) g.check(Math.min(keep, items.length - 1) + 1);
        } else if (v instanceof LinearLayout) {
            final LinearLayout row = (LinearLayout) v;
            int keep = 0;
            for (int i = 0; i < row.getChildCount(); i++) {
                if (row.getChildAt(i).isSelected()) keep = i;
            }
            for (int i = 0; i < items.length; i++) {
                if (i < row.getChildCount()) {
                    ((Button) row.getChildAt(i)).setText(items[i]);
                } else {
                    final int idx = i;
                    final long nid = id;
                    Button b = new Button(DayBridge.ctx);
                    b.setText(items[i]);
                    b.setAllCaps(false);
                    b.setOnClickListener(new View.OnClickListener() {
                        public void onClick(View x) {
                            selectSegment(row, idx);
                            DayBridge.nativeOnEvent(nid, 4, idx, null);
                        }
                    });
                    row.addView(b);
                }
            }
            while (row.getChildCount() > items.length) row.removeViewAt(row.getChildCount() - 1);
            if (items.length > 0) selectSegment(row, Math.min(keep, items.length - 1));
        }
    }

    public static void setPickerSelected(View v, int idx) {
        if (v instanceof Spinner) {
            ((Spinner) v).setSelection(idx);
        } else if (v instanceof RadioGroup) {
            if (idx >= 0) ((RadioGroup) v).check(idx + 1);
        } else if (v instanceof LinearLayout) {
            selectSegment((LinearLayout) v, idx);
        }
    }
}
