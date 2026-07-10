package dev.daybrite.day.bridge;

import android.content.Context;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Handler;
import android.os.Looper;
import android.text.Editable;
import android.text.TextWatcher;
import android.util.TypedValue;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.SubMenu;
import android.view.View;
import android.view.ViewGroup;
import android.widget.AbsListView;
import android.widget.AdapterView;
import android.widget.BaseAdapter;
import android.widget.ListView;
import android.widget.CompoundButton;
import android.widget.EditText;
import android.widget.ProgressBar;
import android.widget.ScrollView;
import android.widget.TextView;

import com.google.android.material.button.MaterialButton;
import com.google.android.material.dialog.MaterialAlertDialogBuilder;
import com.google.android.material.divider.MaterialDivider;
import com.google.android.material.loadingindicator.LoadingIndicator;
import com.google.android.material.materialswitch.MaterialSwitch;
import com.google.android.material.progressindicator.LinearProgressIndicator;
import com.google.android.material.slider.Slider;
import com.google.android.material.textfield.MaterialAutoCompleteTextView;
import com.google.android.material.textfield.TextInputEditText;
import com.google.android.material.textfield.TextInputLayout;

/** The Java shim (the Kotlin/C++-shim analogue for android.widget): creates native views,
 *  wires their listeners to the single native trampoline nativeOnEvent(id, kind, num, str)
 *  (kinds: 0=press 1=text 2=toggle 3=value 4=select), and exposes setters + measurement +
 *  absolute layout to Rust. Controls are Material 3 components (com.google.android.material,
 *  Theme.Material3Expressive — the app theme supplies color/shape/motion); containers/labels
 *  stay framework views. */
public final class DayBridge {
    /** App context + main-thread handler, set by DayActivity before nativeStart. */
    public static Context ctx;
    public static Handler main = new Handler(Looper.getMainLooper());

    // --- natives (exported by the app's cdylib) ---
    public static native void nativeStart(View root, float density, int w, int h,
                                          String autodrive, String locale, String envBlob);
    public static native void nativeOnEvent(long id, int kind, double num, String str);
    public static native void nativeRunPosted(long token);
    /** Recycling list (docs/list.md): the adapter pulls row count + fills recycled cells. */
    public static native int nativeListLen(long hostId);
    public static native void nativeListBind(long hostId, int position, View cell);

    /** Cross-thread → main-thread door for day's scheduler/Setter (§3.3). */
    public static void postMain(final long token) {
        main.post(new Runnable() {
            public void run() { nativeRunPosted(token); }
        });
    }

    // --- factories + setters (called from Rust over JNI) ---
    public static View makeContainer() { return new DayFixed(ctx); }

    /** A `background`/`corner_radius` surface: a GradientDrawable (rounded rect) as the view's
     *  background, plus clipToOutline so a corner radius also clips child views. `argb` is packed
     *  0xAARRGGBB (used only when `hasBg`); `radiusPx` is already density-scaled. */
    public static void setSurface(View v, int argb, boolean hasBg, float radiusPx, boolean clips) {
        GradientDrawable d = new GradientDrawable();
        if (hasBg) d.setColor(argb);
        if (radiusPx > 0f) d.setCornerRadius(radiusPx);
        v.setBackground(d);
        if (clips || radiusPx > 0f) v.setClipToOutline(true);
    }

    /** A native recycling list (docs/list.md): a framework ListView whose BaseAdapter reuses
     *  DayFixed cell views (convertView) and lets day fill each via nativeListBind. */
    public static View makeList(final long hostId, final int rowHeightPx, final boolean selectable) {
        final ListView lv = new ListView(ctx);
        lv.setDivider(null);
        lv.setDividerHeight(0);
        lv.setAdapter(new BaseAdapter() {
            public int getCount() { return nativeListLen(hostId); }
            public Object getItem(int p) { return null; }
            public long getItemId(int p) { return p; }
            public View getView(int position, View convertView, ViewGroup parent) {
                DayFixed cell = (convertView instanceof DayFixed) ? (DayFixed) convertView : null;
                if (cell == null) {
                    cell = new DayFixed(ctx);
                    cell.setLayoutParams(new AbsListView.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, rowHeightPx));
                }
                nativeListBind(hostId, position, cell);
                return cell;
            }
        });
        if (selectable) {
            lv.setOnItemClickListener(new AdapterView.OnItemClickListener() {
                public void onItemClick(AdapterView<?> p, View v, int pos, long rowId) {
                    nativeOnEvent(hostId, 4, pos, ""); // kind 4 = select
                }
            });
        }
        return lv;
    }
    public static void listReload(View lv) {
        if (lv instanceof ListView && ((ListView) lv).getAdapter() instanceof BaseAdapter) {
            ((BaseAdapter) ((ListView) lv).getAdapter()).notifyDataSetChanged();
        }
    }
    /** Scroll the list so its last row is fully visible (a chat sticking to the newest message).
     *  Posted so it runs after any pending notifyDataSetChanged relayout; no-op when empty. */
    public static void listScrollToEnd(View v) {
        if (!(v instanceof ListView)) return;
        final ListView lv = (ListView) v;
        lv.post(new Runnable() {
            public void run() {
                int n = lv.getCount();
                if (n > 0) lv.smoothScrollToPosition(n - 1);
            }
        });
    }

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
    /**
     * `sp` size (scales with the accessibility Font Size setting), font weight (100–900), italic,
     * and an optional bundled font family (null for the system font — §18.4).
     */
    public static void setLabelFont(View v, float sp, int weight, boolean italic, String family) {
        TextView t = (TextView) v;
        // COMPLEX_UNIT_SP applies the user's font scale (Settings ▸ Display ▸ Font size) — the Android
        // accessibility text-scale — unlike DIP which does not.
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, sp);
        Typeface base = (family != null && !family.isEmpty()) ? bundledFont(family) : Typeface.DEFAULT;
        if (android.os.Build.VERSION.SDK_INT >= 28) {
            // Exact numeric weight + italic (API 28+); a custom base picks (or synthesizes) the
            // nearest face the family provides.
            t.setTypeface(Typeface.create(base, weight, italic));
        } else {
            int style = (weight >= 600 ? Typeface.BOLD : Typeface.NORMAL) | (italic ? Typeface.ITALIC : 0);
            t.setTypeface(Typeface.create(base, style));
        }
    }

    private static final java.util.Map<String, Typeface> FONT_CACHE = new java.util.HashMap<>();
    /**
     * Resolve a bundled font family (§18.4). `day build` stages each `fonts/` file as
     * `res/font/<ident>.ttf`, where `<ident>` is the font's family name sanitized to Android
     * resource rules (lowercase `[a-z0-9_]`, leading letter — the same derivation as day-spec's
     * `font_ident`). Re-derive the ident here and look up `R.font.<ident>`, so no side table is
     * needed. Unknown families (or API < 26, which predates font resources) log and fall back to
     * the system typeface.
     */
    private static Typeface bundledFont(String family) {
        Typeface cached = FONT_CACHE.get(family);
        if (cached != null) return cached;
        StringBuilder sb = new StringBuilder();
        for (char c : family.toCharArray()) {
            char lc = (c >= 'A' && c <= 'Z') ? (char) (c - 'A' + 'a') : c;
            boolean ok = (lc >= 'a' && lc <= 'z') || (lc >= '0' && lc <= '9') || lc == '_';
            sb.append(ok ? lc : '_');
        }
        if (sb.length() == 0 || sb.charAt(0) < 'a' || sb.charAt(0) > 'z') sb.insert(0, 'r');
        Typeface tf = null;
        if (android.os.Build.VERSION.SDK_INT >= 26) {
            int id = ctx.getResources().getIdentifier(sb.toString(), "font", ctx.getPackageName());
            if (id != 0) {
                try {
                    tf = ctx.getResources().getFont(id);
                } catch (Exception e) {
                    // Broken resource — fall through to the loud default below.
                }
            }
        }
        if (tf == null) {
            android.util.Log.w("DayBridge", "unknown font family \"" + family
                    + "\" — falling back to the system font (is the file in the project's fonts/ directory?)");
            tf = Typeface.DEFAULT;
        }
        FONT_CACHE.put(family, tf);
        return tf;
    }
    /** Text color as a packed 0xAARRGGBB int; `on=false` restores the theme default. */
    public static void setLabelColor(View v, int argb, boolean on) {
        TextView t = (TextView) v;
        if (on) {
            t.setTextColor(argb);
        } else {
            t.setTextColor(new TextView(ctx).getTextColors());
        }
    }

    public static View makeButton(final long id, String title) {
        MaterialButton b = new MaterialButton(ctx); // M3 filled button (Expressive shape/motion)
        b.setText(title);
        b.setOnClickListener(new View.OnClickListener() {
            public void onClick(View x) { nativeOnEvent(id, 0, 0, null); }
        });
        return b;
    }

    /** Attach a tap or drag recognizer to a view (docs/shapes.md). Coordinates are px; Rust
     *  converts to dp. Event kind 11; num = phase (0=tap 1=began 2=changed 3=ended). */
    public static void enableGesture(View v, final long id, final boolean isDrag) {
        v.setOnTouchListener(new View.OnTouchListener() {
            float sx, sy;
            public boolean onTouch(View view, MotionEvent ev) {
                float x = ev.getX(), y = ev.getY();
                switch (ev.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        sx = x; sy = y;
                        if (isDrag) nativeOnEvent(id, 11, 1, x + "," + y + ",0,0");
                        return true;
                    case MotionEvent.ACTION_MOVE:
                        if (isDrag) nativeOnEvent(id, 11, 2, x + "," + y + "," + (x - sx) + "," + (y - sy));
                        return true;
                    case MotionEvent.ACTION_UP:
                        if (isDrag) {
                            nativeOnEvent(id, 11, 3, x + "," + y + "," + (x - sx) + "," + (y - sy));
                        } else if (Math.abs(x - sx) < 40 && Math.abs(y - sy) < 40) {
                            nativeOnEvent(id, 11, 0, x + "," + y + ",0,0");
                            view.performClick();
                        }
                        return true;
                    case MotionEvent.ACTION_CANCEL:
                        if (isDrag) nativeOnEvent(id, 11, 3, x + "," + y + "," + (x - sx) + "," + (y - sy));
                        return true;
                }
                return false;
            }
        });
    }

    /** The editable inside a Material text box (TextInputLayout), or the view itself. */
    private static EditText editTextOf(View v) {
        if (v instanceof TextInputLayout) return ((TextInputLayout) v).getEditText();
        return (EditText) v;
    }

    public static View makeTextField(final long id, String value, String placeholder) {
        // M3 text box: TextInputLayout (theme's default box style; placeholder = floating label)
        // wrapping a TextInputEditText. Rust talks to the outer view; setters reach the editable.
        TextInputLayout box = new TextInputLayout(ctx);
        box.setHint(placeholder);
        TextInputEditText e = new TextInputEditText(box.getContext());
        e.setText(value);
        e.setSingleLine(true);
        e.addTextChangedListener(new TextWatcher() {
            public void afterTextChanged(Editable s) { nativeOnEvent(id, 1, 0, s.toString()); }
            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            public void onTextChanged(CharSequence s, int a, int b, int c) {}
        });
        box.addView(e, new TextInputLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        return box;
    }
    public static void setTextField(View v, String value) {
        EditText e = editTextOf(v);
        if (!e.getText().toString().equals(value)) { // controlled input (§4.4)
            e.setText(value);
            e.setSelection(value.length());
        }
    }
    public static void setPlaceholder(View v, String value) {
        if (v instanceof TextInputLayout) ((TextInputLayout) v).setHint(value);
        else ((EditText) v).setHint(value);
    }

    public static View makeToggle(final long id, boolean value) {
        MaterialSwitch s = new MaterialSwitch(ctx); // M3 switch
        s.setChecked(value);
        s.setOnCheckedChangeListener(new CompoundButton.OnCheckedChangeListener() {
            public void onCheckedChanged(CompoundButton b, boolean on) {
                nativeOnEvent(id, 2, on ? 1 : 0, null);
            }
        });
        return s;
    }
    public static void setToggle(View v, boolean value) {
        CompoundButton s = (CompoundButton) v;
        if (s.isChecked() != value) s.setChecked(value);
    }

    public static View makeSlider(final long id, double value, final double min, final double max) {
        Slider s = new Slider(ctx); // M3 slider; real value range, no step quantization
        s.setValueFrom((float) min);
        s.setValueTo((float) max);
        s.setValue((float) Math.max(min, Math.min(max, value)));
        s.addOnChangeListener(new Slider.OnChangeListener() {
            @Override public void onValueChange(Slider slider, float v, boolean fromUser) {
                if (fromUser) nativeOnEvent(id, 3, v, null);
            }
        });
        return s;
    }
    public static void setSlider(View v, double value, double ignoredMin) {
        Slider s = (Slider) v;
        float f = (float) Math.max(s.getValueFrom(), Math.min(s.getValueTo(), value));
        // A stepped slider (e.g. day-tweak-slider-tickmarks) hard-crashes at the next layout pass
        // unless EVERY value is valueFrom + n*stepSize (BaseSlider.validateValues throws) — snap
        // programmatic writes onto the step grid defensively.
        float step = s.getStepSize();
        if (step > 0f) {
            f = s.getValueFrom() + Math.round((f - s.getValueFrom()) / step) * step;
            f = Math.max(s.getValueFrom(), Math.min(s.getValueTo(), f));
        }
        if (s.getValue() != f) s.setValue(f); // programmatic: listener sees fromUser=false, no echo
    }

    public static View makeDivider() {
        return new MaterialDivider(ctx); // themed hairline (colorOutlineVariant)
    }

    // Progress: an M3 linear determinate indicator (0..1000), or the M3 Expressive
    // LoadingIndicator (morphing-shape spinner) when indeterminate.
    public static View makeProgress(boolean determinate, double fraction) {
        if (determinate) {
            LinearProgressIndicator pb = new LinearProgressIndicator(ctx);
            pb.setMax(1000);
            pb.setIndeterminate(false);
            pb.setProgress(progressTicks(fraction));
            return pb;
        }
        return new LoadingIndicator(ctx);
    }
    public static void setProgress(View v, double fraction) {
        if (!(v instanceof ProgressBar)) return; // LoadingIndicator has no progress to sync
        ProgressBar pb = (ProgressBar) v;
        int p = progressTicks(fraction);
        if (pb.getProgress() != p) pb.setProgress(p);
    }
    private static int progressTicks(double fraction) {
        return (int) Math.round(Math.max(0.0, Math.min(1.0, fraction)) * 1000);
    }

    /** Combobox (day-piece-combobox): the M3 exposed dropdown menu — a TextInputLayout in the
     *  theme's filled-dropdown style hosting a non-editable MaterialAutoCompleteTextView. */
    public static View makeSpinner(final long id, String joinedItems, int selected) {
        final String[] items = joinedItems.split("\n");
        TextInputLayout box = new TextInputLayout(ctx, null,
                com.google.android.material.R.attr.textInputFilledExposedDropdownMenuStyle);
        MaterialAutoCompleteTextView tv = new MaterialAutoCompleteTextView(box.getContext());
        tv.setInputType(android.text.InputType.TYPE_NULL); // select-only, no free text
        tv.setSimpleItems(items);
        // Size to the widest item (an UNSPECIFIED probe of the box ignores prospective values):
        // text width + the box's start padding and end (dropdown-icon) inset. The minimum goes on
        // the TextInputLayout itself — LinearLayout honors its own suggested minimum during an
        // UNSPECIFIED measure, but nothing propagates a child EditText minimum up through the box.
        float widest = 0f;
        for (String it : items) widest = Math.max(widest, tv.getPaint().measureText(it));
        float d = ctx.getResources().getDisplayMetrics().density;
        box.setMinimumWidth((int) (widest + 76 * d));
        if (selected >= 0 && selected < items.length) tv.setText(items[selected], false);
        tv.setOnItemClickListener(new AdapterView.OnItemClickListener() {
            public void onItemClick(AdapterView<?> p, View v, int pos, long rowId) {
                nativeOnEvent(id, 4, pos, null);
            }
        });
        box.setTag(items);
        box.addView(tv, new TextInputLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        return box;
    }
    public static void setSpinnerSelected(View v, int idx) {
        String[] items = (String[]) v.getTag();
        EditText e = editTextOf(v);
        if (idx >= 0 && idx < items.length && e instanceof MaterialAutoCompleteTextView
                && !e.getText().toString().equals(items[idx])) {
            ((MaterialAutoCompleteTextView) e).setText(items[idx], false); // false: no filter/echo
        }
    }

    public static void addChild(View parent, View child) {
        if (parent instanceof DayNavHost) { ((DayNavHost) parent).add(child); return; }
        if (parent instanceof DayTabs) {
            ((DayTabs) parent).addTab(child, (String) child.getTag());
            return;
        }
        View target = contentOf(parent);
        if (target instanceof ViewGroup) ((ViewGroup) target).addView(child);
    }
    public static void removeChild(View child) {
        // Nav pages route through their host (looked up by view — the FragmentManager may
        // have the page detached mid-transition, so the parent chain can't be relied on).
        DayNavHost navHost = DayNavHost.pageHosts.get(child);
        if (navHost != null) {
            navHost.removePage(child);
            return;
        }
        ViewGroup p = (ViewGroup) child.getParent();
        if (p != null) p.removeView(child);
    }
    public static void setFrame(View v, int x, int y, int w, int h) {
        ViewGroup p = (ViewGroup) v.getParent();
        // Nav / tab pages fill the host's page frame — their frames are native-owned.
        if (p != null && p.getParent() instanceof DayNavHost) return;
        if (p != null && p.getParent() instanceof DayTabs) return;
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

    // Tabs (docs/tabs.md): a DayTabs strip; each page is a DayFixed carrying its title as a tag.
    public static View makeTabs(long id, int initial) { return new DayTabs(ctx, id, initial); }
    public static View makeTabPage(final long id, String title) {
        DayFixed page = new DayFixed(ctx);
        page.setTag(title);
        page.addOnLayoutChangeListener(new View.OnLayoutChangeListener() {
            @Override public void onLayoutChange(View v, int l, int t, int r, int b,
                    int ol, int ot, int or2, int ob) {
                int w = r - l, h = b - t;
                if (w != or2 - ol || h != ob - ot) {
                    nativeOnEvent(id, 6, 0.0, w + "," + h); // kind 6 = FrameChanged
                }
            }
        });
        return page;
    }
    public static void setTabsSelected(View tabs, int index) { ((DayTabs) tabs).select(index); }
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
        // The nav menu can have more items than fit on screen (the showcase sidebar has ~20), so it
        // must scroll — wrap the row column in a vertical ScrollView (fillViewport so it still fills
        // when short).
        ScrollView sv = new ScrollView(ctx);
        sv.setFillViewport(true);
        sv.addView(list, new ScrollView.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        return sv;
    }

    // --- imperative presentation (docs/dialogs.md) ---
    static final java.util.HashMap<Long, android.app.Dialog> presents = new java.util.HashMap<>();

    /** A native alert / confirm / action sheet; onClick reports the spec button index. */
    public static void present(final long req, boolean sheet, String title, String message,
            String buttonsJoined, String rolesJoined) {
        final String[] labels = buttonsJoined.isEmpty() ? new String[0] : buttonsJoined.split("");
        MaterialAlertDialogBuilder b = new MaterialAlertDialogBuilder(ctx); // M3 dialog
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
        android.app.Dialog dlg = b.create();
        presents.put(req, dlg);
        dlg.show();
    }

    /** A native M3 text prompt (a TextInputLayout box); OK reports the entered text. */
    public static void presentPrompt(final long req, String title, String message,
            String placeholder, String initial, String ok, String cancel) {
        TextInputLayout box = new TextInputLayout(ctx);
        box.setHint(placeholder);
        final TextInputEditText input = new TextInputEditText(box.getContext());
        input.setText(initial);
        input.setSingleLine(true);
        box.addView(input, new TextInputLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        // The dialog content area has no inherent padding; give the box the M3 24dp side inset.
        android.widget.FrameLayout wrap = new android.widget.FrameLayout(ctx);
        int inset = (int) (24 * ctx.getResources().getDisplayMetrics().density);
        wrap.setPadding(inset, inset / 2, inset, 0);
        wrap.addView(box, new android.widget.FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        MaterialAlertDialogBuilder b = new MaterialAlertDialogBuilder(ctx); // M3 dialog
        b.setTitle(title);
        if (message != null && !message.isEmpty()) b.setMessage(message);
        b.setView(wrap);
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
        android.app.Dialog dlg = b.create();
        presents.put(req, dlg);
        dlg.show();
    }

    public static void dismissPresent(long req) {
        android.app.Dialog dlg = presents.remove(req);
        if (dlg != null) dlg.dismiss();
        // A pending SAF picker (docs/files.md): cancel the child DocumentsUI activity. Reached when
        // a scripted respond answers the request Day-side (day-core dismisses the native control
        // after recording the answer) — without this the picker stays on screen over the app.
        Integer rc = fileDayToReq.remove(req);
        if (rc != null) {
            fileReqToDay.remove(rc);
            fileSaveSrc.remove(rc);
            if (ctx instanceof android.app.Activity) {
                ((android.app.Activity) ctx).finishActivity(rc);
            }
        }
    }

    // --- Native file open/save via the Storage Access Framework (docs/files.md) ---------------
    // startActivityForResult carries an int requestCode, so a small table correlates it back to the
    // Day request id (+ save mode/source). DayActivity.onActivityResult routes results here.

    static final int FILE_REQUEST_BASE = 0x0DA7;
    static int fileRequestNext = FILE_REQUEST_BASE;
    static final java.util.HashMap<Integer, long[]> fileReqToDay = new java.util.HashMap<>();
    static final java.util.HashMap<Integer, String> fileSaveSrc = new java.util.HashMap<>();
    /** Reverse map (Day request id → requestCode) so dismissPresent can cancel a pending picker. */
    static final java.util.HashMap<Long, Integer> fileDayToReq = new java.util.HashMap<>();

    /** The app cache dir (app-writable temp area for save staging). */
    public static String cacheDirPath() {
        try {
            return ctx.getCacheDir().getAbsolutePath();
        } catch (Exception e) {
            android.util.Log.w("Day", "cacheDirPath failed", e);
            return "";
        }
    }

    /** The app-private files dir (app-writable, persistent — for app data stores). */
    public static String filesDirPath() {
        try {
            return ctx.getFilesDir().getAbsolutePath();
        } catch (Exception e) {
            android.util.Log.w("Day", "filesDirPath failed", e);
            return "";
        }
    }

    public static void presentFileOpen(final long req, String title, String filtersJoined) {
        android.content.Intent intent =
            new android.content.Intent(android.content.Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(android.content.Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        String[] mimes = fileMimeTypes(filtersJoined);
        if (mimes.length > 0) intent.putExtra(android.content.Intent.EXTRA_MIME_TYPES, mimes);
        launchFile(req, intent, null);
    }

    public static void presentFileSave(final long req, String title, String suggested,
            String srcPath, String filtersJoined) {
        android.content.Intent intent =
            new android.content.Intent(android.content.Intent.ACTION_CREATE_DOCUMENT);
        intent.addCategory(android.content.Intent.CATEGORY_OPENABLE);
        intent.setType(mimeForName(suggested));
        if (suggested != null && !suggested.isEmpty())
            intent.putExtra(android.content.Intent.EXTRA_TITLE, suggested);
        launchFile(req, intent, srcPath);
    }

    private static void launchFile(long req, android.content.Intent intent, String srcPath) {
        if (!(ctx instanceof android.app.Activity)) {
            nativeOnEvent(req, 10, 0.0, null); // 10 = dismissed (no Activity to host the picker)
            return;
        }
        int rc = fileRequestNext++;
        fileReqToDay.put(rc, new long[] { req });
        fileDayToReq.put(req, rc);
        if (srcPath != null) fileSaveSrc.put(rc, srcPath);
        try {
            ((android.app.Activity) ctx).startActivityForResult(intent, rc);
        } catch (Exception e) {
            android.util.Log.w("Day", "file picker startActivityForResult failed", e);
            fileReqToDay.remove(rc);
            fileDayToReq.remove(req);
            fileSaveSrc.remove(rc);
            nativeOnEvent(req, 10, 0.0, null);
        }
    }

    /** Called by DayActivity.onActivityResult for our file requests. */
    static void onFileResult(int requestCode, int resultCode, android.content.Intent data) {
        long[] slot = fileReqToDay.remove(requestCode);
        if (slot == null) return;
        long req = slot[0];
        fileDayToReq.remove(req);
        String src = fileSaveSrc.remove(requestCode);
        android.net.Uri uri = (resultCode == android.app.Activity.RESULT_OK && data != null) ? data.getData() : null;
        if (uri == null) {
            nativeOnEvent(req, 10, 0.0, null); // dismissed
            return;
        }
        try {
            if (src != null) {
                // Save: stream the Day-staged temp file into the chosen document; return its URI.
                copyStream(new java.io.FileInputStream(src),
                        ctx.getContentResolver().openOutputStream(uri));
                nativeOnEvent(req, 15, 0.0, uri.toString()); // 15 = files
            } else {
                // Open: copy the picked document into an app cache file, return that readable path.
                String name = displayName(uri);
                java.io.File out = new java.io.File(ctx.getCacheDir(), "day-open-" + req + "-" + name);
                copyStream(ctx.getContentResolver().openInputStream(uri),
                        new java.io.FileOutputStream(out));
                nativeOnEvent(req, 15, 0.0, out.getAbsolutePath());
            }
        } catch (Exception e) {
            android.util.Log.w("Day", "file open/save transfer failed", e);
            nativeOnEvent(req, 10, 0.0, null);
        }
    }

    private static void copyStream(java.io.InputStream in, java.io.OutputStream out)
            throws java.io.IOException {
        try (java.io.InputStream i = in; java.io.OutputStream o = out) {
            byte[] buf = new byte[8192];
            int n;
            while ((n = i.read(buf)) > 0) o.write(buf, 0, n);
            o.flush();
        }
    }

    private static String displayName(android.net.Uri uri) {
        String name = "file";
        try (android.database.Cursor c = ctx.getContentResolver().query(uri, null, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                int i = c.getColumnIndex(android.provider.OpenableColumns.DISPLAY_NAME);
                if (i >= 0 && c.getString(i) != null) name = c.getString(i);
            }
        } catch (Exception e) {
            android.util.Log.w("Day", "display-name lookup failed for " + uri, e);
        }
        return name.replaceAll("[/\\\\]", "_");
    }

    // Map Day's "name|ext1,ext2" filter list (0x1f-joined) to MIME types for EXTRA_MIME_TYPES.
    private static String[] fileMimeTypes(String filtersJoined) {
        if (filtersJoined == null || filtersJoined.isEmpty()) return new String[0];
        java.util.LinkedHashSet<String> mimes = new java.util.LinkedHashSet<>();
        for (String f : filtersJoined.split("")) {
            int bar = f.indexOf('|');
            String exts = bar >= 0 ? f.substring(bar + 1) : "";
            for (String e : exts.split(",")) if (!e.isEmpty()) mimes.add(mimeForExt(e));
        }
        return mimes.toArray(new String[0]);
    }

    private static String mimeForName(String name) {
        int dot = name == null ? -1 : name.lastIndexOf('.');
        return dot >= 0 ? mimeForExt(name.substring(dot + 1)) : "application/octet-stream";
    }

    private static String mimeForExt(String ext) {
        String m = android.webkit.MimeTypeMap.getSingleton()
                .getMimeTypeFromExtension(ext.toLowerCase());
        return m != null ? m : "application/octet-stream";
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
    public static View makeImage(String name, int mode) {
        android.widget.ImageView iv = new android.widget.ImageView(ctx);
        // Scaling (§18.3): 0=fit, 1=fill (crop), 2=stretch.
        iv.setScaleType(
                mode == 2 ? android.widget.ImageView.ScaleType.FIT_XY
                        : mode == 1 ? android.widget.ImageView.ScaleType.CENTER_CROP
                                : android.widget.ImageView.ScaleType.FIT_CENTER);
        // Prefer a processed drawable resource by name (§18.3): images/<name> is staged into
        // res/drawable -> R.drawable.<name>, crunched/optimized by aapt2. Fall back to a raw asset
        // by path (back-compat for image("file.png") loaded straight from assets/).
        int id = ctx.getResources().getIdentifier(name, "drawable", ctx.getPackageName());
        if (id != 0) {
            iv.setImageResource(id);
            return iv;
        }
        try {
            android.graphics.Bitmap bm =
                    android.graphics.BitmapFactory.decodeStream(ctx.getAssets().open(name));
            iv.setImageBitmap(bm);
        } catch (Exception e) {
            android.util.Log.w("Day", "image asset decode failed for " + name, e);
        }
        return iv;
    }
    /** Accessibility (§13): contentDescription = label (TalkBack reads it); importantForAccessibility
     *  hides decorative elements + their subtree; stateDescription = value on API 30+. */
    public static void setA11y(View v, String label, String value, boolean hidden) {
        if (label != null && !label.isEmpty()) v.setContentDescription(label);
        v.setImportantForAccessibility(hidden
            ? View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS
            : View.IMPORTANT_FOR_ACCESSIBILITY_AUTO);
        if (value != null && !value.isEmpty() && android.os.Build.VERSION.SDK_INT >= 30) {
            v.setStateDescription(value);
        }
    }

    // --- Menus (docs/menus.md) -------------------------------------------------
    // The context menu is a PopupMenu shown on long-press (the Android touch convention); the app
    // menu is the app-bar overflow (⋮), built by DayActivity.onCreateOptionsMenu. Both parse the
    // same tab-separated spec (kind\tid\tenabled\tlabel per line) and route item clicks to
    // nativeOnEvent(id, 13, 0, "") = MenuAction.

    /** The current app (overflow) menu spec, or null. Set by setAppMenu; read by DayActivity. */
    public static String appMenuSpec = null;

    // --- Lifecycle (docs/lifecycle.md) ----------------------------------------
    // True once nativeStart has run; lifecycle events before that are dropped (native isn't ready).
    // DayActivity forwards Activity lifecycle transitions here with the phase code (day_spec::Lifecycle
    // order: 2=DidBecomeActive 3=WillResignActive 4=WillEnterForeground 5=DidEnterBackground
    // 6=DidReceiveMemoryWarning 7=WillTerminate), delivered to native as event kind 14.
    public static volatile boolean started = false;

    /** Forward an Activity lifecycle phase to native, once the app has started. */
    public static void lifecycle(int code) {
        if (started) nativeOnEvent(0L, 14, code, "");
    }

    /** Attach `spec` as `v`'s context menu (long-press). An empty spec detaches it. */
    public static void setContextMenu(final View v, final String spec) {
        if (spec == null || spec.isEmpty()) {
            v.setOnLongClickListener(null);
            v.setLongClickable(false);
            return;
        }
        v.setOnLongClickListener(new View.OnLongClickListener() {
            public boolean onLongClick(View anchor) {
                android.widget.PopupMenu popup = new android.widget.PopupMenu(anchor.getContext(), anchor);
                buildMenu(popup.getMenu(), spec);
                popup.show();
                return true;
            }
        });
    }

    /** Record the app menu spec + refresh the Activity's overflow menu. */
    public static void setAppMenu(String spec) {
        appMenuSpec = spec;
        if (ctx instanceof android.app.Activity) {
            ((android.app.Activity) ctx).invalidateOptionsMenu();
        }
    }

    /** Populate `menu` from `spec`. Android SubMenus can't nest, so deeper submenus flatten into
     *  the nearest SubMenu. Separators become group boundaries (dividers on API 28+). */
    public static void buildMenu(Menu menu, String spec) {
        if (spec == null || spec.isEmpty()) return;
        if (android.os.Build.VERSION.SDK_INT >= 28) menu.setGroupDividerEnabled(true);
        // A stack of the menu we are currently adding into (index 0 = root).
        java.util.ArrayList<Menu> stack = new java.util.ArrayList<Menu>();
        stack.add(menu);
        int[] order = {0};
        int[] group = {0};
        for (String line : spec.split("\n")) {
            if (line.isEmpty()) continue;
            String[] f = line.split("\t", 4);
            if (f.length < 1) continue;
            String kind = f[0];
            Menu cur = stack.get(stack.size() - 1);
            if (kind.equals("-")) {
                group[0]++; // next items land in a new group → a divider is drawn between them
            } else if (kind.equals("S")) {
                String label = f.length > 3 ? f[3] : "";
                // SubMenu.addSubMenu is unsupported; when already in a submenu, flatten.
                if (cur instanceof SubMenu) {
                    stack.add(cur);
                } else {
                    stack.add(cur.addSubMenu(group[0], Menu.NONE, order[0]++, label));
                }
            } else if (kind.equals("E")) {
                if (stack.size() > 1) stack.remove(stack.size() - 1);
            } else { // "A" = action (roles too, with id 0)
                final long id = f.length > 1 ? parseLong(f[1]) : 0L;
                boolean enabled = f.length > 2 && f[2].equals("1");
                String label = f.length > 3 ? f[3] : "";
                MenuItem it = cur.add(group[0], Menu.NONE, order[0]++, label);
                it.setEnabled(enabled);
                it.setOnMenuItemClickListener(new MenuItem.OnMenuItemClickListener() {
                    public boolean onMenuItemClick(MenuItem mi) {
                        nativeOnEvent(id, 13, 0.0, "");
                        return true;
                    }
                });
            }
        }
    }

    private static long parseLong(String s) {
        try { return Long.parseLong(s); }
        catch (NumberFormatException e) { android.util.Log.w("Day", "parseLong failed for " + s, e); return 0L; }
    }
}
