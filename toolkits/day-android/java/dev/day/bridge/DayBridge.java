package dev.day.bridge;

import android.content.Context;
import android.graphics.Typeface;
import android.os.Handler;
import android.os.Looper;
import android.text.Editable;
import android.text.TextWatcher;
import android.util.TypedValue;
import android.view.View;
import android.view.ViewGroup;
import android.widget.AdapterView;
import android.widget.ArrayAdapter;
import android.widget.Button;
import android.widget.CompoundButton;
import android.widget.EditText;
import android.widget.ScrollView;
import android.widget.SeekBar;
import android.widget.Spinner;
import android.widget.Switch;
import android.widget.TextView;

/** The Java shim (the Kotlin/C++-shim analogue for android.widget): creates framework views,
 *  wires their listeners to the single native trampoline nativeOnEvent(id, kind, num, str)
 *  (kinds: 0=press 1=text 2=toggle 3=value 4=select), and exposes setters + measurement +
 *  absolute layout to Rust. Framework widgets only — zero AndroidX dependencies. */
public final class DayBridge {
    /** App context + main-thread handler, set by DayActivity before nativeStart. */
    public static Context ctx;
    public static Handler main = new Handler(Looper.getMainLooper());

    // --- natives (exported by the app's cdylib) ---
    public static native void nativeStart(View root, float density, int w, int h,
                                          String autodrive, String locale, String envBlob);
    public static native void nativeOnEvent(long id, int kind, double num, String str);
    public static native void nativeRunPosted(long token);

    /** Cross-thread → main-thread door for day's scheduler/Setter (§3.3). */
    public static void postMain(final long token) {
        main.post(new Runnable() {
            public void run() { nativeRunPosted(token); }
        });
    }

    // --- factories + setters (called from Rust over JNI) ---
    public static View makeContainer() { return new DayFixed(ctx); }

    public static View makeScroll() {
        ScrollView sv = new ScrollView(ctx);
        sv.setFillViewport(false);
        sv.addView(new DayFixed(ctx));
        return sv;
    }
    public static View contentOf(View v) {
        if (v instanceof ScrollView && ((ScrollView) v).getChildCount() > 0) {
            return ((ScrollView) v).getChildAt(0);
        }
        return v;
    }
    public static void setScrollContent(View v, int w, int h) {
        View content = contentOf(v);
        if (content instanceof DayFixed) ((DayFixed) content).setContentSize(w, h);
    }

    public static View makeLabel(String text) {
        TextView t = new TextView(ctx);
        t.setText(text);
        return t;
    }
    public static void setLabel(View v, String text) { ((TextView) v).setText(text); }
    public static void setLabelFont(View v, float dip, boolean bold) {
        TextView t = (TextView) v;
        t.setTextSize(TypedValue.COMPLEX_UNIT_DIP, dip);
        t.setTypeface(bold ? Typeface.DEFAULT_BOLD : Typeface.DEFAULT);
    }

    public static View makeButton(final long id, String title) {
        Button b = new Button(ctx);
        b.setText(title);
        b.setAllCaps(false);
        b.setOnClickListener(new View.OnClickListener() {
            public void onClick(View x) { nativeOnEvent(id, 0, 0, null); }
        });
        return b;
    }

    public static View makeTextField(final long id, String value, String placeholder) {
        EditText e = new EditText(ctx);
        e.setText(value);
        e.setHint(placeholder);
        e.setSingleLine(true);
        e.addTextChangedListener(new TextWatcher() {
            public void afterTextChanged(Editable s) { nativeOnEvent(id, 1, 0, s.toString()); }
            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            public void onTextChanged(CharSequence s, int a, int b, int c) {}
        });
        return e;
    }
    public static void setTextField(View v, String value) {
        EditText e = (EditText) v;
        if (!e.getText().toString().equals(value)) { // controlled input (§4.4)
            e.setText(value);
            e.setSelection(value.length());
        }
    }
    public static void setPlaceholder(View v, String value) { ((EditText) v).setHint(value); }

    public static View makeToggle(final long id, boolean value) {
        Switch s = new Switch(ctx);
        s.setChecked(value);
        s.setOnCheckedChangeListener(new CompoundButton.OnCheckedChangeListener() {
            public void onCheckedChanged(CompoundButton b, boolean on) {
                nativeOnEvent(id, 2, on ? 1 : 0, null);
            }
        });
        return s;
    }
    public static void setToggle(View v, boolean value) {
        Switch s = (Switch) v;
        if (s.isChecked() != value) s.setChecked(value);
    }

    private static final double SLIDER_STEPS = 1000.0;
    public static View makeSlider(final long id, double value, final double min, final double max) {
        SeekBar sb = new SeekBar(ctx);
        sb.setMax((int) SLIDER_STEPS);
        sb.setTag(new double[]{min, max});
        sb.setProgress((int) Math.round((value - min) / (max - min) * SLIDER_STEPS));
        sb.setOnSeekBarChangeListener(new SeekBar.OnSeekBarChangeListener() {
            public void onProgressChanged(SeekBar s, int p, boolean fromUser) {
                if (fromUser) nativeOnEvent(id, 3, min + (p / SLIDER_STEPS) * (max - min), null);
            }
            public void onStartTrackingTouch(SeekBar s) {}
            public void onStopTrackingTouch(SeekBar s) {}
        });
        return sb;
    }
    public static void setSlider(View v, double value, double ignoredMin) {
        SeekBar sb = (SeekBar) v;
        double[] r = (double[]) sb.getTag();
        int p = (int) Math.round((value - r[0]) / (r[1] - r[0]) * SLIDER_STEPS);
        if (sb.getProgress() != p) sb.setProgress(p);
    }

    public static View makeDivider() {
        View v = new View(ctx);
        v.setBackgroundColor(0x33888888);
        return v;
    }

    public static View makeSpinner(final long id, String joinedItems, int selected) {
        Spinner sp = new Spinner(ctx);
        ArrayAdapter<String> adapter = new ArrayAdapter<>(ctx,
                android.R.layout.simple_spinner_item, joinedItems.split("\n"));
        adapter.setDropDownViewResource(android.R.layout.simple_spinner_dropdown_item);
        sp.setAdapter(adapter);
        if (selected >= 0) sp.setSelection(selected);
        // Spinner fires once on first layout; suppress that initial callback.
        sp.setTag(new int[]{0});
        sp.setOnItemSelectedListener(new AdapterView.OnItemSelectedListener() {
            public void onItemSelected(AdapterView<?> p, View v, int pos, long rowId) {
                int[] fired = (int[]) p.getTag();
                if (fired[0]++ > 0) nativeOnEvent(id, 4, pos, null);
            }
            public void onNothingSelected(AdapterView<?> p) {}
        });
        return sp;
    }
    public static void setSpinnerSelected(View v, int idx) {
        Spinner sp = (Spinner) v;
        if (sp.getSelectedItemPosition() != idx && idx >= 0) sp.setSelection(idx);
    }

    public static void addChild(View parent, View child) {
        if (parent instanceof DayNavHost) { ((DayNavHost) parent).add(child); return; }
        View target = contentOf(parent);
        if (target instanceof ViewGroup) ((ViewGroup) target).addView(child);
    }
    public static void removeChild(View child) {
        ViewGroup p = (ViewGroup) child.getParent();
        if (p != null && p.getParent() instanceof DayNavHost) {
            ((DayNavHost) p.getParent()).removePage(child);
            return;
        }
        if (p != null) p.removeView(child);
    }
    public static void setFrame(View v, int x, int y, int w, int h) {
        ViewGroup p = (ViewGroup) v.getParent();
        // Nav pages are MATCH_PARENT in the host's page frame — their frames are native-owned.
        if (p != null && p.getParent() instanceof DayNavHost) return;
        if (p instanceof DayFixed) ((DayFixed) p).setChildFrame(v, x, y, w, h);
    }

    // --- navigation (docs/navigation.md) ---
    public static View makeNavHost(long id, String title) {
        return new DayNavHost(ctx, id, title);
    }
    public static View makeNavPage(final long id) {
        DayFixed page = new DayFixed(ctx);
        page.addOnLayoutChangeListener(new View.OnLayoutChangeListener() {
            @Override public void onLayoutChange(View v, int l, int t, int r, int b,
                    int ol, int ot, int or2, int ob) {
                int w = r - l, h = b - t;
                if (w != or2 - ol || h != ob - ot) {
                    // kind 6 = FrameChanged, "w,h" in px (Rust divides by density).
                    nativeOnEvent(id, 6, 0.0, w + "," + h);
                }
            }
        });
        return page;
    }
    public static void navPush(View host, String title) { ((DayNavHost) host).push(title); }
    public static void navPop(View host) { ((DayNavHost) host).pop(); }
    /** nav_menu(): standard tappable list rows (ripple, 48dp) for the route table. */
    public static View makeNavMenu(final long id, String joinedItems) {
        android.widget.LinearLayout list = new android.widget.LinearLayout(ctx);
        list.setOrientation(android.widget.LinearLayout.VERTICAL);
        String[] items = joinedItems.isEmpty() ? new String[0] : joinedItems.split("\u001f");
        android.util.TypedValue tv = new android.util.TypedValue();
        ctx.getTheme().resolveAttribute(android.R.attr.selectableItemBackground, tv, true);
        float d = ctx.getResources().getDisplayMetrics().density;
        for (int i = 0; i < items.length; i++) {
            final int index = i;
            TextView row = new TextView(ctx);
            row.setText(items[i]);
            row.setTextSize(16f);
            row.setMinHeight((int) (48 * d));
            row.setGravity(android.view.Gravity.CENTER_VERTICAL);
            row.setPadding((int) (16 * d), 0, (int) (16 * d), 0);
            row.setBackgroundResource(tv.resourceId);
            row.setClickable(true);
            row.setOnClickListener(new View.OnClickListener() {
                @Override public void onClick(View v) {
                    nativeOnEvent(id, 4, index, null); // kind 4 = SelectionChanged
                }
            });
            list.addView(row, new android.widget.LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        }
        return list;
    }

    // --- imperative presentation (docs/dialogs.md) ---
    static final java.util.HashMap<Long, android.app.AlertDialog> presents = new java.util.HashMap<>();

    /** A native alert / confirm / action sheet; onClick reports the spec button index. */
    public static void present(final long req, boolean sheet, String title, String message,
            String buttonsJoined, String rolesJoined) {
        final String[] labels = buttonsJoined.isEmpty() ? new String[0] : buttonsJoined.split("");
        android.app.AlertDialog.Builder b = new android.app.AlertDialog.Builder(ctx);
        b.setTitle(title);
        if (sheet) {
            // A titled list of choices — the Android idiom for an action sheet.
            b.setItems(labels, new android.content.DialogInterface.OnClickListener() {
                @Override public void onClick(android.content.DialogInterface d, int which) {
                    presents.remove(req);
                    nativeOnEvent(req, 8, (double) which, null); // 8 = present button
                }
            });
        } else {
            if (message != null && !message.isEmpty()) b.setMessage(message);
            String[] roles = rolesJoined.isEmpty() ? new String[0] : rolesJoined.split(",");
            boolean positiveUsed = false;
            for (int i = 0; i < labels.length; i++) {
                final int idx = i;
                int role = (i < roles.length) ? Integer.parseInt(roles[i]) : 0;
                android.content.DialogInterface.OnClickListener cb =
                    new android.content.DialogInterface.OnClickListener() {
                        @Override public void onClick(android.content.DialogInterface d, int w) {
                            presents.remove(req);
                            nativeOnEvent(req, 8, (double) idx, null);
                        }
                    };
                if (role == 1) b.setNegativeButton(labels[i], cb);          // cancel
                else if (!positiveUsed) { b.setPositiveButton(labels[i], cb); positiveUsed = true; }
                else b.setNeutralButton(labels[i], cb);
            }
        }
        b.setOnCancelListener(new android.content.DialogInterface.OnCancelListener() {
            @Override public void onCancel(android.content.DialogInterface d) {
                presents.remove(req);
                nativeOnEvent(req, 10, 0.0, null); // 10 = dismissed
            }
        });
        android.app.AlertDialog dlg = b.create();
        presents.put(req, dlg);
        dlg.show();
    }

    /** A native text prompt (EditText); OK reports the entered text. */
    public static void presentPrompt(final long req, String title, String message,
            String placeholder, String initial, String ok, String cancel) {
        final android.widget.EditText input = new android.widget.EditText(ctx);
        input.setHint(placeholder);
        input.setText(initial);
        input.setSingleLine(true);
        android.app.AlertDialog.Builder b = new android.app.AlertDialog.Builder(ctx);
        b.setTitle(title);
        if (message != null && !message.isEmpty()) b.setMessage(message);
        b.setView(input);
        b.setPositiveButton(ok, new android.content.DialogInterface.OnClickListener() {
            @Override public void onClick(android.content.DialogInterface d, int w) {
                presents.remove(req);
                nativeOnEvent(req, 9, 0.0, input.getText().toString()); // 9 = present text
            }
        });
        b.setNegativeButton(cancel, new android.content.DialogInterface.OnClickListener() {
            @Override public void onClick(android.content.DialogInterface d, int w) {
                presents.remove(req);
                nativeOnEvent(req, 10, 0.0, null);
            }
        });
        b.setOnCancelListener(new android.content.DialogInterface.OnCancelListener() {
            @Override public void onCancel(android.content.DialogInterface d) {
                presents.remove(req);
                nativeOnEvent(req, 10, 0.0, null);
            }
        });
        android.app.AlertDialog dlg = b.create();
        presents.put(req, dlg);
        dlg.show();
    }

    public static void dismissPresent(long req) {
        android.app.AlertDialog dlg = presents.remove(req);
        if (dlg != null) dlg.dismiss();
    }

    public static int measureWidth(View v) {
        v.measure(View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
                  View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED));
        return v.getMeasuredWidth();
    }
    public static int measureHeight(View v) {
        v.measure(View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED),
                  View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED));
        return v.getMeasuredHeight();
    }
    /** Height-for-width (§7.2): AT_MOST width probe, never EXACTLY (child-chooses). */
    public static int measureHeightForWidth(View v, int wPx) {
        v.measure(View.MeasureSpec.makeMeasureSpec(wPx, View.MeasureSpec.AT_MOST),
                  View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED));
        return v.getMeasuredHeight();
    }
    public static int measureWidthForWidth(View v, int wPx) {
        v.measure(View.MeasureSpec.makeMeasureSpec(wPx, View.MeasureSpec.AT_MOST),
                  View.MeasureSpec.makeMeasureSpec(0, View.MeasureSpec.UNSPECIFIED));
        return v.getMeasuredWidth();
    }

    public static void setEnabled(View v, boolean b) { v.setEnabled(b); }

    public static View makeCanvas() { return new DayCanvasView(ctx); }
    public static void setCanvasOps(View v, double[] nums, String textsJoined) {
        ((DayCanvasView) v).setOps(nums, textsJoined);
    }
    public static View makeImage(String assetPath) {
        android.widget.ImageView iv = new android.widget.ImageView(ctx);
        try {
            android.graphics.Bitmap bm =
                    android.graphics.BitmapFactory.decodeStream(ctx.getAssets().open(assetPath));
            iv.setImageBitmap(bm);
            iv.setScaleType(android.widget.ImageView.ScaleType.FIT_CENTER);
        } catch (Exception ignored) {}
        return iv;
    }
    public static void setA11y(View v, String label) { v.setContentDescription(label); }
}
