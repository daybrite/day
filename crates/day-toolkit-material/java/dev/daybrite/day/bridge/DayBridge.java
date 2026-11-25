// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.graphics.Typeface;
import android.graphics.drawable.GradientDrawable;
import android.os.Handler;
import android.os.Looper;
import android.text.Editable;
import android.text.TextWatcher;
import android.util.TypedValue;
import android.view.Choreographer;
import android.view.KeyEvent;
import android.view.Menu;
import android.view.MenuItem;
import android.view.MotionEvent;
import android.view.ScaleGestureDetector;
import android.view.VelocityTracker;
import android.view.SubMenu;
import android.view.View;
import android.view.ViewGroup;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputMethodManager;
import android.widget.AdapterView;
import androidx.recyclerview.widget.ItemTouchHelper;
import androidx.recyclerview.widget.LinearLayoutManager;
import androidx.recyclerview.widget.RecyclerView;
import android.widget.CompoundButton;
import android.widget.EditText;
import android.widget.ProgressBar;
import android.widget.HorizontalScrollView;
import android.widget.ScrollView;
import android.widget.TextView;

import com.google.android.material.button.MaterialButton;
import com.google.android.material.dialog.MaterialAlertDialogBuilder;
import com.google.android.material.color.MaterialColors;
import com.google.android.material.divider.MaterialDivider;
import com.google.android.material.loadingindicator.LoadingIndicator;
import com.google.android.material.materialswitch.MaterialSwitch;
import com.google.android.material.navigation.NavigationView;
import com.google.android.material.progressindicator.LinearProgressIndicator;
import com.google.android.material.slider.Slider;
import com.google.android.material.textfield.MaterialAutoCompleteTextView;
import com.google.android.material.textfield.TextInputEditText;
import com.google.android.material.textfield.TextInputLayout;

/** The Java shim (the Kotlin/C++-shim analogue for android.widget): creates native views,
 *  wires their listeners to the single native trampoline nativeOnEvent(id, kind, num, str)
 *  (kinds: 0=press 1=text 2=toggle 3=value 4=select), and exposes setters + measurement +
 *  absolute layout to Rust. Controls are Material 3 components (com.google.android.material,
 *  Theme.Material3Expressive; the app theme supplies color/shape/motion); containers/labels
 *  stay framework views. */
public final class DayBridge extends DayRuntime {
    /** App context + main-thread handler, set by DayActivity before nativeStart. */

    // --- natives (exported by the app's cdylib) ---
    public static native void nativeStart(View root, float density, int w, int h,
                                          String autodrive, String locale, String envBlob);
    public static native void nativeOnEvent(long id, int kind, double num, String str);
    /** A secondary DayWindowActivity's first laid-out root (docs/windows.md): completes the
     *  pending day::open_window. Returns false when the window was closed before connecting. */
    public static native boolean nativeStartWindow(View root, long node, float density,
            int w, int h);

    /** Launch a secondary day window (docs/windows.md): a document-style DayWindowActivity
     *  carrying the day node id + title. Rust calls this from the open_window duty. */
    public static void openWindow(long node, String title) {
        android.content.Context c = ctx;
        if (c == null) return;
        android.content.Intent i = new android.content.Intent(c, DayWindowActivity.class);
        i.putExtra("day.node", node);
        i.putExtra("day.title", title);
        i.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK
                | android.content.Intent.FLAG_ACTIVITY_NEW_DOCUMENT
                | android.content.Intent.FLAG_ACTIVITY_MULTIPLE_TASK);
        c.startActivity(i);
    }

    /** Close a secondary window (docs/windows.md): finish its activity and confirm the close
     *  to day right away. finish() tears the activity down asynchronously, and while its
     *  onDestroy was still pending day kept the window in its registry; a re-open in that gap (a
     *  walkthrough's second Preferences, a second later) focused the dying activity and showed
     *  nothing. */
    public static void closeWindow(long node) {
        DayWindowActivity a = DayWindowActivity.ACTIVE.get(node);
        if (a == null) return;
        a.closeReported = true;
        a.finish();
        if (started) nativeOnEvent(node, K_WINDOW_CLOSED, 0, null);
    }

    /** Bring a secondary window's task to the front. */
    public static void focusWindow(long node) {
        DayWindowActivity a = DayWindowActivity.ACTIVE.get(node);
        if (a == null) return;
        android.app.ActivityManager am = (android.app.ActivityManager)
                a.getSystemService(android.content.Context.ACTIVITY_SERVICE);
        if (am != null) am.moveTaskToFront(a.getTaskId(), 0);
    }

    /** Retitle a secondary window (label + recents card). */
    public static void setWindowTitle(long node, String title) {
        DayWindowActivity a = DayWindowActivity.ACTIVE.get(node);
        if (a != null) {
            a.setTitle(title);
            a.setTaskDescription(new android.app.ActivityManager.TaskDescription(title));
        }
    }

    /** The primary window's label in the recents switcher (docs/windows.md).
     *
     *  Android's answer to "list the app's windows" is one recents card per task, labeled by
     *  its task description, so the main activity needs this for the same reason a secondary
     *  one does. It has no `day.node` to be addressed by, hence its own entry point. */
    public static void setPrimaryWindowTitle(String title) {
        if (ctx instanceof android.app.Activity) {
            android.app.Activity a = (android.app.Activity) ctx;
            a.setTitle(title);
            a.setTaskDescription(new android.app.ActivityManager.TaskDescription(title));
        }
    }

    // --- event kinds -----------------------------------------------------------
    // Mirror of day_spec::bridge::BridgeKind (the shared wire table). day-toolkit-material's
    // bridge_kinds_parity test reads this block and asserts each value against the Rust enum, so
    // edit both together. Public so piece-owned Java (the K_CUSTOM channel) can use them.
    public static final int K_REFRESH_REQUESTED = 35;
    public static final int K_PRESSED = 0;
    public static final int K_TEXT_CHANGED = 1;
    public static final int K_TOGGLE_CHANGED = 2;
    public static final int K_VALUE_CHANGED = 3;
    public static final int K_SELECTION_CHANGED = 4;
    public static final int K_LIST_ACTIVATED = 31;
    public static final int K_NAV_BACK = 5;
    public static final int K_FRAME_CHANGED = 6;
    public static final int K_DEEPLINK = 7;
    public static final int K_DOCUMENT_OPENED = 32;
    /** Text from the share sheet (Day.toml share_target, docs/deep-links.md): the text, then
     *  the unit separator and the subject when there is one. */
    public static final int K_SHARED_TEXT = 37;
    public static final int K_PRESENT_BUTTON = 8;
    public static final int K_PRESENT_TEXT = 9;
    public static final int K_PRESENT_DISMISSED = 10;
    public static final int K_GESTURE = 11;
    public static final int K_CUSTOM = 12;
    public static final int K_MENU_ACTION = 13;
    public static final int K_LIFECYCLE = 14;
    public static final int K_PRESENT_FILE = 15;
    public static final int K_FOCUS_CHANGED = 16;
    public static final int K_SUBMITTED = 17;
    public static final int K_WINDOW_RESIZED = 18;
    public static final int K_WINDOW_CLOSED = 20;
    public static final int K_WINDOW_FOCUSED = 21;
    public static final int K_VALUE_COMMITTED = 22;
    /** Inline search on a `.searchable()` nav surface (docs/search.md): the field's new text. */
    public static final int K_SEARCH_CHANGED = 23;
    /** A nav host's SlidingPaneLayout settled on a presentation (docs/size-classes.md). */
    public static final int K_NAV_PRESENTATION = 24;
    public static final int K_APPEARANCE_CHANGED = 25;
    /** The user's "Remove animations" setting flipped (docs/accessibility.md); no payload. */
    public static final int K_REDUCE_MOTION_CHANGED = 33;
    public static final int K_COVER_HIDDEN = 26;
    /** A styled run's link was tapped (docs/text-runs.md); the string is the target. */
    public static final int K_LINK_ACTIVATED = 27;
    /** A toolbar item produced a value (docs/toolbars.md): "on" + num, or "sel" + index. */
    public static final int K_TOOLBAR_CHANGED = 30;
    /** A key reached a focused canvas (docs/menus.md): the day key name, with the modifier mask. */
    public static final int K_KEY = 29;
    /** Text committed on the full key route (docs/menus.md "The full route"): what the input
     *  method or the key typed, with the modifier mask in `num`. */
    public static final int K_TEXT = 36;
    public static final int K_SAFE_AREA = 19;
    /** A window's display state changed (docs/windows.md): num = the day WindowState code
     *  (0 normal, 3 fullscreen here), against the window's day node or 0 for the primary. */
    public static final int K_WINDOW_STATE_CHANGED = 34;

    // --- gesture codes (K_GESTURE's `num`) -------------------------------------------------
    // 0 to 3 are a tap and a drag's three phases, 4 to 11 are day_spec::bridge::gesture, and
    // the codes from 13 up are this toolkit's own (lib.rs `gesture_wire`). The
    // gesture_codes_parity test reads these lines, one constant each, so edit both together.
    static final int G_TAP = 0;
    static final int G_DRAG_BEGAN = 1;
    static final int G_DRAG_CHANGED = 2;
    static final int G_DRAG_ENDED = 3;
    static final int G_LONG_PRESS = 4;
    static final int G_DOUBLE_TAP = 5;
    static final int G_PRESS_IN = 6;
    static final int G_PRESS_OUT = 7;
    static final int G_TOUCHES = 8;
    static final int G_DRAG_VELOCITY = 9;
    static final int G_POINTER_KIND = 11;
    static final int G_ROTATE_BEGAN = 13;
    static final int G_ROTATE_CHANGED = 14;
    static final int G_ROTATE_ENDED = 15;
    static final int G_PINCH_BEGAN = 16;
    static final int G_PINCH_CHANGED = 17;
    static final int G_PINCH_ENDED = 18;
    static final int G_PAN = 19;
    static final int G_HOVER_BEGAN = 20;
    static final int G_HOVER_CHANGED = 21;
    static final int G_HOVER_ENDED = 22;

    /** Report gesture `code` with `payload` (px) for node `id`. */
    static void gesture(long id, int code, String payload) {
        nativeOnEvent(id, K_GESTURE, code, payload);
    }

    /** Whether node `id` has an `.on_key` handler (docs/menus.md): a canvas asks before it
     *  claims a key or takes focus on a press. */
    public static native boolean nativeHandlesKeys(long id);
    /** Whether node `id` took the whole keyboard (`Decorate::keyboard`, docs/menus.md): a canvas
     *  asks before it raises the soft keyboard or names a letter. */
    public static native boolean nativeHandlesText(long id);
    public static native String nativeContextMenu(long token, float x, float y);
    /** A menu button's menu spec, lowered by its provider now (docs/menus.md "Menu buttons"). */
    public static native String nativeButtonMenu(long token);
    public static native void nativeRunPosted(long token);
    /** Frame clock (§8.4): Choreographer's per-vsync callback forwards here with the frame time. */
    public static native void nativeDoFrame(long token, long frameTimeNanos);
    /** Native two-level disclosure. The general n-level tree remains the composed piece. */
    private static final class DaySectionTree extends android.widget.ExpandableListView {
        final long host;
        final int rowHeight;
        final boolean selectable;
        boolean restoring;
        final java.util.ArrayList<Long> tokens = new java.util.ArrayList<>();
        final java.util.ArrayList<Integer> roots = new java.util.ArrayList<>();
        final java.util.HashMap<Integer, java.util.ArrayList<Integer>> children = new java.util.HashMap<>();
        final java.util.HashSet<Integer> headers = new java.util.HashSet<>();
        final java.util.HashMap<Integer, View> bound = new java.util.HashMap<>();
        final android.widget.BaseExpandableListAdapter adapter;
        DaySectionTree(long id, int height, boolean choose) {
            super(ctx); host=id; rowHeight=height; selectable=choose;
            // AbsListView defaults to non-nested scrolling. Sectioned navigation must
            // forward scroll deltas to the host's Material AppBarLayout, like RecyclerView.
            setNestedScrollingEnabled(true);
            setDividerHeight(0);
            setOnGroupClickListener((parent, view, group, ignored) -> {
                int row=roots.get(group);
                if(headers.contains(row)) return false;
                select(row); return true;
            });
            setOnChildClickListener((parent, view, group, child, ignored) -> { select(children.get(roots.get(group)).get(child)); return true; });
            setOnGroupExpandListener(group -> disclosure(group,true));
            setOnGroupCollapseListener(group -> {
                for(int row: children.get(roots.get(group))) {
                    View cell=bound.get(row); if(cell!=null) nativeListRecycle(host,cell);
                }
                disclosure(group,false);
            });
            adapter = new android.widget.BaseExpandableListAdapter() {
                public int getGroupCount() { return roots.size(); }
                public int getChildrenCount(int group) { return children.get(roots.get(group)).size(); }
                public Object getGroup(int group) { return roots.get(group); }
                public Object getChild(int group,int child) { return children.get(roots.get(group)).get(child); }
                public long getGroupId(int group) { return tokens.get(roots.get(group)); }
                public long getChildId(int group,int child) { return tokens.get((Integer)getChild(group,child)); }
                public boolean hasStableIds() { return true; }
                public boolean isChildSelectable(int group,int child) { return selectable; }
                public View getGroupView(int group,boolean expanded,View reused,ViewGroup parent) { return bind(roots.get(group),reused); }
                public View getChildView(int group,int child,boolean last,View reused,ViewGroup parent) { return bind((Integer)getChild(group,child),reused); }
            };
            setAdapter(adapter);
        }
        void select(int row) {
            if(selectable) nativeOnEvent(host,K_CUSTOM,0,"day-tree-select:"+Long.toUnsignedString(tokens.get(row)));
        }
        void disclosure(int group,boolean open) {
            if(!restoring) nativeOnEvent(host,K_CUSTOM,open?1:0,"day-tree-expand:"+Long.toUnsignedString(tokens.get(roots.get(group))));
        }
        View bind(int row, View reused) {
            android.widget.FrameLayout wrap;
            DayFixed cell;
            if(reused instanceof android.widget.FrameLayout) {
                wrap=(android.widget.FrameLayout)reused; cell=(DayFixed)wrap.getChildAt(0);
                nativeListRecycle(host,cell);
                bound.values().remove(cell);
            } else {
                wrap=new android.widget.FrameLayout(ctx);
                cell=new DayFixed(ctx) {
                    @Override protected void onDetachedFromWindow() { nativeListRecycle(host,this); super.onDetachedFromWindow(); }
                };
                android.widget.FrameLayout.LayoutParams lp=new android.widget.FrameLayout.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,rowHeight);
                lp.leftMargin=Math.round(32*ctx.getResources().getDisplayMetrics().density);
                wrap.addView(cell,lp);
                wrap.setLayoutParams(new android.widget.AbsListView.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,rowHeight));
            }
            bound.put(row,cell);
            nativeListBind(host,row,cell);
            paintSelected(wrap,!headers.contains(row) && nativeListIsSelected(host,row));
            return wrap;
        }
        void reload(String data) {
            restoring=true;
            for(View cell:bound.values()) nativeListRecycle(host,cell);
            bound.clear(); tokens.clear(); roots.clear(); children.clear(); headers.clear();
            java.util.ArrayList<Boolean> open=new java.util.ArrayList<>();
            if(!data.isEmpty()) for(String line:data.split("\n")) {
                String[] fields=line.split(":"); int row=tokens.size();
                tokens.add(Long.parseUnsignedLong(fields[0]));
                int parent=Integer.parseInt(fields[1]);
                children.put(row,new java.util.ArrayList<>());
                if(parent<0) roots.add(row); else children.get(parent).add(row);
                if(fields[2].equals("1")) headers.add(row);
                open.add(fields[3].equals("1"));
            }
            adapter.notifyDataSetChanged();
            for(int group=0;group<roots.size();++group) {
                if(open.get(roots.get(group))) expandGroup(group); else collapseGroup(group);
            }
            restoring=false;
        }
        void expand(long token,boolean open) {
            restoring=true;
            for(int g=0;g<roots.size();++g) if(tokens.get(roots.get(g))==token) {
                if(open) expandGroup(g); else collapseGroup(g);
            }
            restoring=false;
        }
    }
    public static View makeTree(long id,int height,boolean selectable) { return new DaySectionTree(id,height,selectable); }
    public static void treeReload(View view,String data) { if(view instanceof DaySectionTree) ((DaySectionTree)view).reload(data); }
    public static void treeExpand(View view,long token,boolean open) { if(view instanceof DaySectionTree) ((DaySectionTree)view).expand(token,open); }
    public static void treeSelection(View view) { if(view instanceof DaySectionTree) ((DaySectionTree)view).adapter.notifyDataSetChanged(); }
    public static void treeReveal(View view,long token) {
        if(!(view instanceof DaySectionTree)) return;
        DaySectionTree t=(DaySectionTree)view;
        for(int g=0;g<t.roots.size();++g) {
            int root=t.roots.get(g);
            if(t.tokens.get(root)==token) { t.setSelectedGroup(g); return; }
            java.util.ArrayList<Integer> children=t.children.get(root);
            for(int c=0;c<children.size();++c) if(t.tokens.get(children.get(c))==token) { t.setSelectedChild(g,c,true); return; }
        }
    }

    /** Recycling list (docs/list.md): the adapter pulls row count + fills recycled cells. */
    public static native int nativeListLen(long hostId);
    public static native void nativeListBind(long hostId, int position, View cell);
    public static native void nativeListBindPinned(long hostId, int position, View cell);
    /** A holder left the visible set: day clears its dayscript ids (docs/list.md). */
    public static native void nativeListRecycle(long hostId, View cell);
    /** Whether row `position` is in the list's programmatic selection (ListPatch::Selected). */
    public static native boolean nativeListIsSelected(long hostId, int position);
    /** The adapter's view type of row `position`: LIST_ROW, LIST_HEADER (a section header,
     *  docs/list.md "Sections") or LIST_SUPPLEMENT (the list's header, footer, empty state or
     *  content padding, docs/list.md "Header, footer and empty state"). */
    public static native int nativeListRowKind(long hostId, int position);
    /** What the list shows (docs/list.md "Visible rows"): the first and last rows with any part
     *  in view, and the distance from the viewport's end to the content's end, in px. */
    public static native void nativeListViewport(long hostId, int first, int last, int endPx);
    /** The reader moved the list themselves (docs/list.md "Following the end"): a drag, a
     *  wheel, a key or an accessibility scroll, never a scroll Day asked for. */
    public static native void nativeListReaderScroll(long hostId);
    /** The header row of the section holding display row `position`, or -1. */
    public static native int nativeListHeaderOf(long hostId, int position);
    /** Drag-to-reorder (docs/list.md): may `from` drop over `to`? (The app guard's live veto;
     *  a Retarget verdict reads as deny here, since ItemTouchHelper can't relocate the gap.) */
    public static native boolean nativeListCanDrop(long hostId, int from, int to);
    /** Commit one incremental drag swap to day; false = refused, don't move. */
    public static native boolean nativeListMove(long hostId, int from, int to);
    /** May row `index` be swiped away? (docs/list.md, the delete guard.) */
    public static native boolean nativeListCanDelete(long hostId, int index);
    /** Commit a swipe-to-delete to day; false if the guard refused. */
    public static native boolean nativeListDelete(long hostId, int index);

    /** Cross-thread → main-thread door for day's scheduler/Setter (§3.3). */
    public static void postMain(final long token) {
        main.post(new Runnable() {
            public void run() { nativeRunPosted(token); }
        });
    }

    /** The timer door behind day::sleep (docs/async.md): postMain after `ms` on the main Handler. */
    public static void postMainDelayed(final long token, final long ms) {
        main.postDelayed(new Runnable() {
            public void run() { nativeRunPosted(token); }
        }, ms);
    }

    /**
     * Frame clock (§8.4): schedule one Choreographer frame callback (called on the UI thread, so
     * getInstance() yields the UI thread's Choreographer). One-shot; day-core re-arms while a
     * frame consumer is live.
     */
    private static final java.util.Map<Long, Choreographer.FrameCallback> frames = new java.util.HashMap<>();
    public static void requestFrame(final long token) {
        Choreographer.FrameCallback callback = new Choreographer.FrameCallback() {
            public void doFrame(long frameTimeNanos) {
                if (frames.remove(token) != null) nativeDoFrame(token, frameTimeNanos);
            }
        };
        frames.put(token, callback);
        Choreographer.getInstance().postFrameCallback(callback);
    }
    public static void cancelFrame(long token) {
        Choreographer.FrameCallback callback = frames.remove(token);
        if (callback != null) Choreographer.getInstance().removeFrameCallback(callback);
    }

    // --- factories + setters (called from Rust over JNI) ---
    // ---- canvas fonts (docs/fonts.md) -----------------------------------------------------

    /** Guarded by its own monitor: the offscreen painter resolves typefaces from a painting
     *  thread while the UI thread measures and draws (docs/canvas.md "Exporting"). */
    private static final java.util.Map<String, Typeface> CANVAS_FONT_CACHE = new java.util.HashMap<>();

    /**
     * The typeface canvas text draws and measures with: a bundled `res/font/` family, else a
     * system family or alias by name (`sans-serif-condensed`, `serif`, …, which `Typeface.create`
     * resolves from `fonts.xml`; an unknown name gives the default face), at a CSS weight (0 =
     * default) and slant. API 28+ picks the exact weight; older releases take the bold flag.
     * Callable from any thread; `Typeface` objects are immutable.
     */
    public static Typeface canvasTypeface(String family, int weight, boolean italic) {
        String key = family + "|" + weight + "|" + italic;
        synchronized (CANVAS_FONT_CACHE) {
            Typeface cached = CANVAS_FONT_CACHE.get(key);
            if (cached != null) return cached;
        }
        Typeface base = Typeface.DEFAULT;
        if (family != null && !family.isEmpty()) {
            base = bundledFontOrNull(family);
            if (base == null) base = Typeface.create(family, Typeface.NORMAL);
        }
        Typeface tf;
        if (android.os.Build.VERSION.SDK_INT >= 28) {
            tf = Typeface.create(base, weight > 0 ? weight : 400, italic);
        } else {
            int style = (weight >= 600 ? Typeface.BOLD : Typeface.NORMAL) | (italic ? Typeface.ITALIC : 0);
            tf = Typeface.create(base, style);
        }
        synchronized (CANVAS_FONT_CACHE) {
            CANVAS_FONT_CACHE.put(key, tf);
        }
        return tf;
    }

    /**
     * Every family Android's font configuration names, with one face per `<font>` entry, in
     * Day's list format (U+001E between families; U+001F-separated fields: family, then
     * (face, CSS weight, italic) per face). There is no public API for family names, only the
     * `SystemFonts` API (29+) that lists files, so this reads `/system/etc/fonts.xml`, the
     * configuration `Typeface.create(name, …)` resolves from, so every listed name draws.
     * An `<alias>` with a weight is a one-face family at that weight; one without takes its
     * target's faces. Unnamed (fallback-only) families are skipped.
     */
    public static String fontFamilies() {
        java.util.LinkedHashMap<String, java.util.List<String>> families = new java.util.LinkedHashMap<>();
        java.util.List<String[]> aliases = new java.util.ArrayList<>();
        try (java.io.FileInputStream in = new java.io.FileInputStream("/system/etc/fonts.xml")) {
            org.xmlpull.v1.XmlPullParser p = android.util.Xml.newPullParser();
            p.setInput(in, null);
            String current = null;
            for (int ev = p.getEventType(); ev != org.xmlpull.v1.XmlPullParser.END_DOCUMENT; ev = p.next()) {
                if (ev == org.xmlpull.v1.XmlPullParser.START_TAG) {
                    String tag = p.getName();
                    if ("family".equals(tag)) {
                        current = p.getAttributeValue(null, "name");
                        if (current != null && !families.containsKey(current)) {
                            families.put(current, new java.util.ArrayList<String>());
                        }
                    } else if ("font".equals(tag) && current != null) {
                        String w = p.getAttributeValue(null, "weight");
                        String st = p.getAttributeValue(null, "style");
                        int weight = 400;
                        try { if (w != null) weight = Integer.parseInt(w.trim()); } catch (NumberFormatException ignored) {}
                        boolean italic = "italic".equals(st);
                        families.get(current).add("\u001F" + weight + "\u001F" + (italic ? "1" : "0"));
                    } else if ("alias".equals(tag)) {
                        String name = p.getAttributeValue(null, "name");
                        String to = p.getAttributeValue(null, "to");
                        String w = p.getAttributeValue(null, "weight");
                        if (name != null && to != null) aliases.add(new String[] { name, to, w });
                    }
                } else if (ev == org.xmlpull.v1.XmlPullParser.END_TAG && "family".equals(p.getName())) {
                    current = null;
                }
            }
        } catch (Exception e) {
            android.util.Log.w("DayBridge", "fonts.xml unreadable — the font list is empty: " + e);
        }
        for (String[] alias : aliases) {
            java.util.List<String> target = families.get(alias[1]);
            if (target == null || families.containsKey(alias[0])) continue;
            java.util.List<String> faces = new java.util.ArrayList<>();
            if (alias[2] != null) {
                int weight = 400;
                try { weight = Integer.parseInt(alias[2].trim()); } catch (NumberFormatException ignored) {}
                faces.add("\u001F" + weight + "\u001F0");
                for (String face : target) {
                    if (face.endsWith("\u001F1") && face.startsWith("\u001F" + weight + "\u001F")) {
                        faces.add("\u001F" + weight + "\u001F1");
                        break;
                    }
                }
            } else {
                faces.addAll(target);
            }
            families.put(alias[0], faces);
        }
        // The bundled families (`day build` writes their names to res/values/day_fonts.xml):
        // one Regular face each (the file is the face), drawable by name through
        // bundledFontOrNull.
        if (ctx != null) {
            int id = ctx.getResources().getIdentifier("day_fonts", "array", ctx.getPackageName());
            if (id != 0) {
                try {
                    for (String family : ctx.getResources().getStringArray(id)) {
                        if (family != null && !family.isEmpty() && !families.containsKey(family)) {
                            java.util.List<String> faces = new java.util.ArrayList<>();
                            faces.add("\u001F400\u001F0");
                            families.put(family, faces);
                        }
                    }
                } catch (Exception ignored) {
                    // A missing or malformed array: the system list stands.
                }
            }
        }
        StringBuilder sb = new StringBuilder();
        boolean first = true;
        for (java.util.Map.Entry<String, java.util.List<String>> fam : families.entrySet()) {
            if (!first) sb.append('\u001E');
            first = false;
            sb.append(fam.getKey().replace('\u001F', ' ').replace('\u001E', ' '));
            for (String face : fam.getValue()) {
                sb.append('\u001F'); // an empty face name: Day synthesizes it from weight + slant
                sb.append(face);
            }
        }
        return sb.toString();
    }

    /**
     * Measure one line of canvas text with the typeface `canvasTypeface` resolves, at an
     * unscaled size (the canvas replay scales by density, so the answer is in dp): the advance
     * width, the line height and the ascent, comma-joined.
     */
    /**
     * Comma-joined: advance, line height, ascent, cap height, then the ink box relative to the
     * line box's top-leading corner (x, y, w, h): the eight numbers day-spec's TextMetrics reads.
     *
     * Paint has no cap-height metric, but getTextBounds gives ink bounds, and the ink ascent of a
     * capital is the cap height. That is one extra measurement of a one-character string,
     * memoized per (size, font) on day's side, so a process pays for it once (docs/fonts.md).
     */
    public static String measureText(String text, double size, int weight, boolean italic, String family) {
        android.graphics.Paint paint = new android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG);
        paint.setTextSize((float) size);
        paint.setTypeface(canvasTypeface(family, weight, italic));
        android.graphics.Paint.FontMetrics fm = paint.getFontMetrics();
        float ascent = -fm.ascent;
        android.graphics.Rect ink = new android.graphics.Rect();
        paint.getTextBounds(text, 0, text.length(), ink);
        android.graphics.Rect cap = new android.graphics.Rect();
        paint.getTextBounds("H", 0, 1, cap);
        return paint.measureText(text)
                + "," + (fm.descent - fm.ascent)
                + "," + ascent
                + "," + (-cap.top)
                // getTextBounds is baseline-relative with y down (top negative): shift by the
                // ascent to make it relative to the line box's top.
                + "," + ink.left
                + "," + (ink.top + ascent)
                + "," + ink.width()
                + "," + ink.height();
    }

    /**
     * The device's ordered language preference as BCP-47 tags, comma-joined ("fr-FR,en-US").
     *
     * The configuration's list, not `Locale.getDefault()`: it carries every language the user
     * ranked in Settings, and it honors a per-app language override (Android 13+). Day negotiates
     * its catalogs against the whole list (docs/localization.md).
     */
    public static String localeTags() {
        android.os.LocaleList list = ctx != null
                ? ctx.getResources().getConfiguration().getLocales()
                : android.os.LocaleList.getDefault();
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < list.size(); i++) {
            if (i > 0) {
                sb.append(',');
            }
            sb.append(list.get(i).toLanguageTag());
        }
        return sb.toString();
    }

    public static View makeContainer() { return new DayFixed(ctx); }

    /** A `background`/`corner_radius` surface: a GradientDrawable (rounded rect) as the view's
     *  background, plus clipToOutline so a corner radius also clips child views. `argb` is packed
     *  0xAARRGGBB (used only when `hasBg`); `radiusPx` is already density-scaled. */
    /** SurfaceRole::SectionCard: the M3 grouped-card fill, resolved from the active theme
     *  (colorSurfaceContainer, falling back to colorSurfaceVariant), so it adapts to the app's
     *  light/dark configuration. */
    public static void setSectionCard(View v, float radiusPx) {
        android.util.TypedValue tv = new android.util.TypedValue();
        boolean ok = ctx.getTheme().resolveAttribute(
                com.google.android.material.R.attr.colorSurfaceContainer, tv, true);
        if (!ok) {
            ok = ctx.getTheme().resolveAttribute(
                    com.google.android.material.R.attr.colorSurfaceVariant, tv, true);
        }
        GradientDrawable d = new GradientDrawable();
        if (ok) d.setColor(tv.data);
        d.setCornerRadius(radiusPx);
        v.setBackground(d);
        v.setClipToOutline(true);
    }

    /** SurfaceRole::Window: the theme's window background, the color an opaque DayCover
     *  paints (a composed sheet's card, docs/cover.md "Modal sheet"). */
    public static void setWindowSurface(View v, float radiusPx, boolean clips) {
        android.util.TypedValue tv = new android.util.TypedValue();
        int bg = ctx.getTheme().resolveAttribute(android.R.attr.colorBackground, tv, true)
                ? tv.data : 0xFFFFFFFF;
        setSurface(v, bg, true, radiusPx, clips);
    }

    public static void setSurface(View v, int argb, boolean hasBg, float radiusPx, boolean clips) {
        GradientDrawable d = new GradientDrawable();
        if (hasBg) d.setColor(argb);
        if (radiusPx > 0f) d.setCornerRadius(radiusPx);
        v.setBackground(d);
        if (clips || radiusPx > 0f) v.setClipToOutline(true);
    }

    /** A surface with a gradient, a border, named corners or a shadow (docs/surfaces.md):
     *  drawn by the container itself, see {@link DayFixed#setSurfaceStyle}. */
    public static void setSurfaceStyle(View v, double[] spec, boolean clips) {
        if (v instanceof DayFixed) ((DayFixed) v).setSurfaceStyle(spec, clips);
    }

    /** `ContainerPatch::Fill` with a gradient: drawn by the container, keeping its corners,
     *  border and shadow (see {@link DayFixed#setSurfaceGradient}). */
    public static void setSurfaceGradient(View v, double[] spec) {
        cancelRun(v, ANIM_FILL);
        if (v instanceof DayFixed) ((DayFixed) v).setSurfaceGradient(spec);
    }

    /** Before `ContainerPatch::Surface` applies a whole new style: drop what the previous one
     *  left, a self-drawn surface, a GradientDrawable background, an outline clip, and a fill
     *  animation in flight. */
    public static void resetSurface(View v) {
        cancelRun(v, ANIM_FILL);
        if (v instanceof DayFixed) ((DayFixed) v).clearSurfaceStyle();
        v.setBackground(null);
        v.setOutlineProvider(android.view.ViewOutlineProvider.BACKGROUND);
        v.setClipToOutline(false);
    }

    /** `ContainerPatch::Fill` with a color: a new fill color. A styled surface keeps everything
     *  else it was given; a plain one is the square-cornered fill it always was. */
    public static void setSurfaceFill(View v, int argb, boolean hasBg) {
        if (v instanceof DayFixed && ((DayFixed) v).setSurfaceFill(argb, hasBg)) return;
        setSurface(v, argb, hasBg, 0f, false);
    }

    // ---- Animation (docs/tween.md "The animation contract", §8.4) ------------------------------
    // Every animated property runs on a ValueAnimator from 0 to 1 whose update listener writes
    // `start + (target - start) * progress`, one animator per (view, channel). That one native
    // animator carries the whole AnimSpec contract: setStartDelay holds the start value once
    // before the first cycle; setRepeatCount with RESTART or REVERSE plays the cycles (Android
    // counts each pass, as Day's `repeat` does, also when reversing, and plays a reversed pass
    // as the curve read backwards); the interpolator is the cycle's curve, sampled in Rust
    // (`AnimSpec::keyframes`) and read back with linear interpolation, so a spring's overshoot
    // shows. ViewPropertyAnimator has no repeat, which is why it is not used here. The values
    // are written to the view's own properties, so what the view answers mid-flight is what it
    // draws (readDisplayed).

    /** Animated channels: the index into a view's run table. */
    static final int ANIM_OPACITY = 0, ANIM_TRANSFORM = 1, ANIM_FRAME = 2, ANIM_FILL = 3;
    /** `timing` slots (src/lib.rs `motion_wire`): cycle length and delay in ms, the cycles
     *  after the first (-1 = until the next set), autoreverse (0 or 1). */
    static final int T_DURATION = 0;
    static final int T_DELAY = 1;
    static final int T_REPEAT = 2;
    static final int T_REVERSE = 3;
    static final int T_COUNT = 4;

    /** The run in flight per view and channel. Weak in the view: a finished or cancelled run
     *  leaves its slot, and a run on a view that left its window ends itself (AnimRun). */
    private static final java.util.WeakHashMap<View, android.animation.ValueAnimator[]> runs =
            new java.util.WeakHashMap<>();

    /** One forward cycle's progress at even offsets (`AnimSpec::keyframes`), read with linear
     *  interpolation between neighbors. Above 1 where a spring overshoots. */
    static final class SampledCurve implements android.animation.TimeInterpolator {
        private final float[] keys;
        SampledCurve(float[] keys) { this.keys = keys; }
        @Override public float getInterpolation(float t) {
            int last = keys.length - 1;
            if (t <= 0f) return keys[0];
            if (t >= 1f) return keys[last];
            float x = t * last;
            int i = (int) x;
            float f = x - i;
            return keys[i] + (keys[Math.min(i + 1, last)] - keys[i]) * f;
        }
    }

    /** What an animated channel writes for a progress: 0 is the start, 1 the target. */
    interface AnimStep { void at(float p); }

    /** Whether `timing` asks for a run: a cycle, or a delay before the target applies. */
    static boolean animates(int[] timing) {
        return timing != null && timing.length >= T_COUNT
                && (timing[T_DURATION] > 0 || timing[T_DELAY] > 0);
    }

    /** Cancel the run on `channel`. The property keeps the value it shows. */
    static void cancelRun(View v, int channel) {
        android.animation.ValueAnimator[] r = runs.get(v);
        if (r == null || r[channel] == null) return;
        android.animation.ValueAnimator a = r[channel];
        r[channel] = null;
        a.cancel();
    }

    /** One run's listener: the step per frame, and the rest once a run ends uncancelled. */
    private static final class AnimRun extends android.animation.AnimatorListenerAdapter
            implements android.animation.ValueAnimator.AnimatorUpdateListener {
        private final View view;
        private final int channel;
        private final AnimStep step;
        private final Runnable rest;
        private boolean cancelled, attached, ending;
        AnimRun(View view, int channel, AnimStep step, Runnable rest) {
            this.view = view;
            this.channel = channel;
            this.step = step;
            this.rest = rest;
        }
        @Override public void onAnimationUpdate(android.animation.ValueAnimator a) {
            // A view that has left its window is not drawn: end the run on its target rather
            // than keep a run that repeats for ever (and the view with it) alive. Ended from
            // the looper, not from here: end() delivers one more update, which would re-enter
            // this listener.
            if (view.isAttachedToWindow()) {
                attached = true;
            } else if (attached) {
                if (!ending) {
                    ending = true;
                    main.post(() -> {
                        if (a.isStarted()) a.end();
                    });
                }
                return;
            }
            step.at((Float) a.getAnimatedValue());
        }
        @Override public void onAnimationCancel(android.animation.Animator a) { cancelled = true; }
        @Override public void onAnimationEnd(android.animation.Animator a) {
            android.animation.ValueAnimator[] r = runs.get(view);
            if (r != null && r[channel] == a) r[channel] = null;
            // Rule 5: a finite run rests on the target, also after a backward last cycle.
            if (!cancelled) rest.run();
        }
    }

    /** Play `step` from 0 to 1 under `timing` and `keys` on `channel`, replacing the run
     *  there. The caller read the start from the view, so a retarget starts from the value
     *  on screen (rule 7). */
    static void startRun(View v, int channel, int[] timing, float[] keys, AnimStep step,
                         Runnable rest) {
        cancelRun(v, channel);
        android.animation.ValueAnimator a = android.animation.ValueAnimator.ofFloat(0f, 1f);
        a.setDuration(Math.max(0, timing[T_DURATION]));
        a.setStartDelay(Math.max(0, timing[T_DELAY]));
        int repeat = timing[T_REPEAT];
        a.setRepeatCount(repeat < 0 ? android.animation.ValueAnimator.INFINITE : repeat);
        a.setRepeatMode(timing[T_REVERSE] != 0
                ? android.animation.ValueAnimator.REVERSE
                : android.animation.ValueAnimator.RESTART);
        a.setInterpolator(keys != null && keys.length >= 2
                ? new SampledCurve(keys)
                : new android.view.animation.LinearInterpolator());
        AnimRun run = new AnimRun(v, channel, step, rest);
        a.addUpdateListener(run);
        a.addListener(run);
        android.animation.ValueAnimator[] r = runs.get(v);
        if (r == null) {
            r = new android.animation.ValueAnimator[4];
            runs.put(v, r);
        }
        r[channel] = a;
        a.start();
    }

    private static float lerp(float a, float b, float p) { return a + (b - a) * p; }

    /** Opacity (§8.4): `timing`/`keys` from src/lib.rs `motion_wire`; no timing sets it at
     *  once and ends any run on it, one that repeats for ever included. */
    public static void setOpacity(View v, float alpha, int[] timing, float[] keys) {
        cancelRun(v, ANIM_OPACITY);
        if (!animates(timing)) {
            v.setAlpha(alpha);
            return;
        }
        final float from = v.getAlpha();
        startRun(v, ANIM_OPACITY, timing, keys,
                p -> v.setAlpha(Math.max(0f, Math.min(1f, lerp(from, alpha, p)))),
                () -> v.setAlpha(alpha));
    }

    /** Transform (§8.4): translation (px, additive over the laid-out position), scale, and
     *  rotation (degrees) about the view's center pivot, with no relayout, animated as one. */
    public static void setTransform(View v, float tx, float ty, float sx, float sy, float rot,
                                    int[] timing, float[] keys) {
        // See addChild: a presented cover shell is positioned/animated natively.
        if (v instanceof DayCover && ((DayCover) v).presented) return;
        cancelRun(v, ANIM_TRANSFORM);
        Runnable target = () -> {
            v.setTranslationX(tx); v.setTranslationY(ty);
            v.setScaleX(sx); v.setScaleY(sy);
            v.setRotation(rot);
        };
        if (!animates(timing)) {
            target.run();
            return;
        }
        final float tx0 = v.getTranslationX(), ty0 = v.getTranslationY();
        final float sx0 = v.getScaleX(), sy0 = v.getScaleY(), rot0 = v.getRotation();
        startRun(v, ANIM_TRANSFORM, timing, keys, p -> {
            v.setTranslationX(lerp(tx0, tx, p)); v.setTranslationY(lerp(ty0, ty, p));
            v.setScaleX(lerp(sx0, sx, p)); v.setScaleY(lerp(sy0, sy, p));
            v.setRotation(lerp(rot0, rot, p));
        }, target);
    }

    /** `ContainerPatch::Fill` with a color, through `update` with an animation: from the fill
     *  shown to `argb`, blended by the platform's ArgbEvaluator (linear light from API 26, sRGB
     *  before), with the progress clamped to the two ends so a spring cannot leave them. A
     *  fill appearing or going away applies at once (rule 8). */
    public static void animateSurfaceFill(View v, int argb, boolean hasBg, int[] timing,
                                          float[] keys) {
        cancelRun(v, ANIM_FILL);
        Integer shown = hasBg ? shownFill(v) : null;
        if (!animates(timing) || shown == null) {
            setSurfaceFill(v, argb, hasBg);
            return;
        }
        final int from = shown;
        final android.animation.ArgbEvaluator blend = new android.animation.ArgbEvaluator();
        startRun(v, ANIM_FILL, timing, keys,
                p -> paintFill(v, (Integer) blend.evaluate(Math.max(0f, Math.min(1f, p)),
                        from, argb)),
                () -> setSurfaceFill(v, argb, true));
    }

    /** One in-between fill: the styled surface's color, or the plain background's
     *  GradientDrawable recolored in place (a new drawable per frame would churn). */
    private static void paintFill(View v, int argb) {
        if (v instanceof DayFixed && ((DayFixed) v).setSurfaceFill(argb, true)) return;
        android.graphics.drawable.Drawable bg = v.getBackground();
        if (bg instanceof GradientDrawable) {
            ((GradientDrawable) bg).setColor(argb);
        } else {
            setSurface(v, argb, true, 0f, false);
        }
    }

    /** The fill color a view shows now, or null when it has none: a styled DayFixed's fill,
     *  else a plain GradientDrawable or ColorDrawable background. */
    static Integer shownFill(View v) {
        if (v instanceof DayFixed && ((DayFixed) v).isStyled()) {
            return ((DayFixed) v).styledFill();
        }
        android.graphics.drawable.Drawable bg = v.getBackground();
        if (bg instanceof GradientDrawable) {
            // GradientDrawable.getColor is API 24, the minimum.
            android.content.res.ColorStateList c = ((GradientDrawable) bg).getColor();
            return c == null ? null : c.getDefaultColor();
        }
        if (bg instanceof android.graphics.drawable.ColorDrawable) {
            return ((android.graphics.drawable.ColorDrawable) bg).getColor();
        }
        return null;
    }

    /** `Toolkit::read_displayed`: what the view shows now, a run in flight included, since
     *  every run writes the view's own properties each frame. Slots (src/lib.rs
     *  `motion_wire::displayed`): [0] alpha, [1] [2] translation in dp, [3] 1 when a frame
     *  was read, [4..7] the frame in dp from the Day root before the view's own transform,
     *  [8] 1 when the view has a fill, [9] that fill as packed ARGB. */
    public static double[] readDisplayed(View v) {
        double[] out = new double[10];
        float d = v.getResources().getDisplayMetrics().density;
        if (d <= 0f) d = 1f;
        out[0] = v.getAlpha();
        out[1] = v.getTranslationX() / d;
        out[2] = v.getTranslationY() / d;
        View root = v.isAttachedToWindow() ? dayRootOf(v) : null;
        if (root != null) {
            int[] at = new int[2];
            int[] origin = new int[2];
            root.getLocationInWindow(origin);
            double x, y;
            if (v == root || !(v.getParent() instanceof View)) {
                v.getLocationInWindow(at);
                x = at[0];
                y = at[1];
            } else {
                // The parent's place (its own and its ancestors' transforms included) plus
                // this view's laid-out offset: the frame before this view's transform.
                View p = (View) v.getParent();
                p.getLocationInWindow(at);
                x = at[0] + v.getLeft() - p.getScrollX();
                y = at[1] + v.getTop() - p.getScrollY();
            }
            out[3] = 1;
            out[4] = (x - origin[0]) / d;
            out[5] = (y - origin[1]) / d;
            out[6] = v.getWidth() / (double) d;
            out[7] = v.getHeight() / (double) d;
        }
        Integer fill = shownFill(v);
        if (fill != null) {
            out[8] = 1;
            out[9] = fill & 0xFFFFFFFFL;
        }
        return out;
    }

    /** A ViewHolder wrapping one DayFixed cell (docs/list.md). */
    static final class DayCellHolder extends RecyclerView.ViewHolder {
        DayCellHolder(DayFixed cell) { super(cell); }
    }

    /** The adapter's view types: a sectioned list's header rows pool apart from its item rows
     *  (docs/list.md "Sections"), so a recycled header cell is never handed to an item. */
    static final int LIST_ROW = 0;
    static final int LIST_HEADER = 1;
    /** A supplementary row (docs/list.md "Header, footer and empty state"): plain content
     *  sized to itself, with no ripple, selection, swipe, drag or pin. Pooled apart, so a
     *  recycled padding row never lends its height to an item. */
    static final int LIST_SUPPLEMENT = 2;

    /** `listSplice` operations (lib.rs `list_wire`), each followed by two ints. */
    static final int SPLICE_INSERT = 0;
    static final int SPLICE_REMOVE = 1;
    static final int SPLICE_MOVE = 2;

    /** The RecyclerView behind every `list` (docs/list.md). An unsectioned list is a plain
     *  RecyclerView. A sectioned one (docs/list.md "Sections") also pins the current section's
     *  header, the Contacts idiom: one extra cell, which Day binds to the header of the section
     *  under the top edge, drawn in the list's own ViewGroupOverlay (so it never joins the
     *  adapter's children or the layout manager's count). The next header pushes it up as it
     *  arrives, and it hides while the real header row sits fully in view at the top. */
    static final class DayListView extends RecyclerView {
        final long hostId;
        final int rowHeightPx;
        /** Set once by listSetSections, before the first bind. */
        boolean sections;
        /** The pinned copy and the display row it is bound to (-1: unbound, or stale after a
         *  reload). Recycled while hidden, so its dayscript ids never shadow the real row's. */
        DayFixed pinned;
        int pinnedRow = -1;
        boolean pinShown;
        /** The list has supplementary rows (listConfigure): they take their own view type. */
        boolean supplements;
        /** Day observes the viewport (listConfigure): report it, never the first row alone. */
        boolean viewport;
        /** Day follows the end (listConfigure): report the scrolls the reader starts
         *  (docs/list.md "Following the end"). */
        boolean reader;
        /** The current drag has moved the list and been reported. */
        boolean dragReported;
        /** A wheel event, a scrolling key or an accessibility scroll action came in and the
         *  scroll it starts has not moved the list yet. Cleared by that first move, by Day's
         *  own scrolls (appScroll), and shortly after the input when nothing moved. */
        boolean readerInput;
        final Runnable readerInputExpired = new Runnable() {
            public void run() { readerInput = false; }
        };
        /** The row count the adapter last told RecyclerView about (a reload, a splice, a
         *  swipe-delete): a splice must start from it and land on Day's count, or it would
         *  throw RecyclerView's inconsistency exception. -1 before the first reload. */
        int shown = -1;
        /** The last viewport report, so a scroll tick that changed nothing sends nothing;
         *  cleared by every data change, which day-core must hear about even when unchanged. */
        int reportedFirst = -1, reportedLast = -1, reportedEnd = -1;

        DayListView(Context c, long hostId, int rowHeightPx) {
            super(c);
            this.hostId = hostId;
            this.rowHeightPx = rowHeightPx;
        }

        /** The adapter's view type for row `position` (docs/list.md "Sections", "Header,
         *  footer and empty state"): asked across JNI only where such rows exist. */
        int rowKind(int position) {
            return sections || supplements ? nativeListRowKind(hostId, position) : LIST_ROW;
        }

        /** Whether a viewport report is already posted for this turn. */
        boolean viewportQueued;
        final Runnable viewportPost = new Runnable() {
            public void run() {
                viewportQueued = false;
                sendViewport();
            }
        };

        /** Report what the list shows (docs/list.md "Visible rows"), after a scroll and after a
         *  layout pass (a resize, a reload or splice laid out). Posted, coalesced to one per
         *  turn: the report enters Day, which runs the app's handlers right away when its tree
         *  is free (an `on_end_reached` that appends rows, a `follow_end` scroll), and those
         *  change the adapter, which RecyclerView refuses inside its own scroll and layout
         *  callbacks. */
        void reportViewport() {
            if (!viewport || viewportQueued) return;
            viewportQueued = true;
            post(viewportPost);
        }

        /** The report itself. The distance to the end is exact while the last row is laid out,
         *  the layout manager's estimate otherwise (at least a pixel: an unlaid last row is past
         *  the viewport). */
        void sendViewport() {
            if (!viewport) return;
            LinearLayoutManager lm = (LinearLayoutManager) getLayoutManager();
            Adapter<?> a = getAdapter();
            int n = a == null ? 0 : a.getItemCount();
            if (lm == null || n == 0 || getHeight() == 0) return;
            int first = lm.findFirstVisibleItemPosition();
            int last = lm.findLastVisibleItemPosition();
            if (first == NO_POSITION) return;
            if (last == NO_POSITION || last < first) last = first;
            int bottom = getHeight() - getPaddingBottom();
            View end = lm.findViewByPosition(n - 1);
            int distance;
            if (end != null) {
                distance = Math.max(0, lm.getDecoratedBottom(end) - bottom);
            } else {
                distance = Math.max(1, computeVerticalScrollRange() - computeVerticalScrollOffset()
                        - computeVerticalScrollExtent());
            }
            if (first == reportedFirst && last == reportedLast && distance == reportedEnd) return;
            reportedFirst = first;
            reportedLast = last;
            reportedEnd = distance;
            nativeListViewport(hostId, first, last, distance);
        }

        /** Bring row `pos` to where `align` says (docs/list.md "Programmatic scrolling").
         *  Nearest resolves to Start or End by the side the row is on, or to nothing when it is
         *  already fully in view.
         *
         *  Animated: RecyclerView's own smoothScrollBy over a distance known in advance: from
         *  the row itself when it is laid out, from its laid-out neighbors and the row pitch
         *  when it is within two screens, else after a jump that puts the row exactly one
         *  viewport short of its place (Android's own lists jump near a far target too, so the
         *  animation covers the last screen rather than every row between). Not
         *  LinearSmoothScroller: it seeks a target view, and Day's row binds lay the list out
         *  mid-scroll, which drops the view it found ("Passed over target position") and sends
         *  it on to the end of the list. Once the glide rests, the row is checked and set
         *  right without animation if the pitch was off.
         *
         *  Instant: scrollToPositionWithOffset, the offset from the row's height (laid out, else
         *  the uniform pitch), corrected once before the frame draws when the real height
         *  differs. The layout manager clamps to the content either way. */
        void scrollRow(final int pos, final int align, boolean animated) {
            final LinearLayoutManager lm = (LinearLayoutManager) getLayoutManager();
            Adapter<?> adapter = getAdapter();
            final int count = adapter == null ? 0 : adapter.getItemCount();
            if (lm == null || count == 0) return;
            // A newer scroll wins: the deferred parts of this one check they are still current.
            final int gen = ++scrollGen;
            restGen = -1;
            appScroll();
            stopScroll();
            final int top = getPaddingTop();
            final int box = getHeight() - getPaddingTop() - getPaddingBottom();
            View view = lm.findViewByPosition(pos);
            int first = lm.findFirstVisibleItemPosition();
            int last = lm.findLastVisibleItemPosition();
            int alignTo = align;
            if (align == LIST_ALIGN_NEAREST) {
                if (view != null) {
                    int t = lm.getDecoratedTop(view), b = lm.getDecoratedBottom(view);
                    if (t >= top && b <= top + box) {
                        reportViewport();
                        return;
                    }
                    alignTo = t < top ? LIST_ALIGN_START : LIST_ALIGN_END;
                } else {
                    alignTo = first != NO_POSITION && pos < first
                            ? LIST_ALIGN_START : LIST_ALIGN_END;
                }
            }
            final int to = alignTo;
            final int guess = view != null ? lm.getDecoratedMeasuredHeight(view) : rowHeightPx;
            final int want = top + rowOffset(to, box, guess);
            if (animated && first != NO_POSITION && last != NO_POSITION) {
                int span = Math.max(1, last - first + 1);
                View edge;
                int dy;
                if (view != null) {
                    dy = lm.getDecoratedTop(view) - want;
                } else if (pos > last && pos <= last + 2 * span
                        && (edge = lm.findViewByPosition(last)) != null) {
                    dy = lm.getDecoratedBottom(edge) + (pos - last - 1) * rowHeightPx - want;
                } else if (pos < first && pos >= first - 2 * span
                        && (edge = lm.findViewByPosition(first)) != null) {
                    dy = lm.getDecoratedTop(edge) - (first - pos) * rowHeightPx - want;
                } else {
                    final int glide = pos > last ? box : -box;
                    // The row's top one viewport short of its place, set through the row that
                    // then sits at the top of the viewport (at the row pitch), so the jump lays
                    // out one screen of rows, as any jump does. Anchoring the far-off row itself
                    // lays out every row between it and the viewport, which the pool cannot
                    // take back: the surplus holders are discarded with their Day rows.
                    int pitch = Math.max(1, rowHeightPx);
                    int base = want - top + glide;
                    int steps = -Math.floorDiv(base, pitch);
                    int anchor = Math.max(0, Math.min(count - 1, pos + steps));
                    lm.scrollToPositionWithOffset(anchor, base + (anchor - pos) * pitch);
                    // Glides once that jump is laid out.
                    post(new Runnable() {
                        private int tries = 0;
                        public void run() {
                            if (gen != scrollGen) return;
                            if ((isLayoutRequested() || hasPendingAdapterUpdates()) && tries++ < 10) {
                                post(this);
                                return;
                            }
                            glide(gen, pos, to, glide);
                        }
                    });
                    return;
                }
                glide(gen, pos, to, dy);
                return;
            }
            lm.scrollToPositionWithOffset(pos, want - top);
            if (to == LIST_ALIGN_START) return;
            getViewTreeObserver().addOnPreDrawListener(
                    new android.view.ViewTreeObserver.OnPreDrawListener() {
                @Override public boolean onPreDraw() {
                    getViewTreeObserver().removeOnPreDrawListener(this);
                    if (gen != scrollGen) return true;
                    View laid = lm.findViewByPosition(pos);
                    if (laid == null) return true;
                    int h = lm.getDecoratedMeasuredHeight(laid);
                    if (h == guess) return true;
                    lm.scrollToPositionWithOffset(pos, rowOffset(to, box, h));
                    return false;
                }
            });
        }

        /** Counts row scrolls (scrollRow), so the parts of one that run later can tell a newer
         *  one has replaced it. */
        int scrollGen;
        /** The animated row scroll to check once the list rests (-1: none), and its row. */
        int restGen = -1, restPos, restAlign;

        void glide(int gen, int pos, int align, int dy) {
            if (dy == 0) {
                reportViewport();
                return;
            }
            restGen = gen;
            restPos = pos;
            restAlign = align;
            smoothScrollBy(0, dy);
        }

        /** An animated row scroll at rest: its row where the alignment puts it, or set there
         *  without animation (the pitch it was estimated from was off). A user's drag cancels
         *  the check; an edge of the content that stopped the glide short is where it rests. */
        @Override public void onScrollStateChanged(int state) {
            super.onScrollStateChanged(state);
            dragReported = false;
            if (state == SCROLL_STATE_DRAGGING) restGen = -1;
            if (state != SCROLL_STATE_IDLE || restGen < 0 || restGen != scrollGen) return;
            restGen = -1;
            LinearLayoutManager lm = (LinearLayoutManager) getLayoutManager();
            if (lm == null) return;
            final int pos = restPos, align = restAlign, gen = scrollGen;
            View v = lm.findViewByPosition(pos);
            if (v != null) {
                int box = getHeight() - getPaddingTop() - getPaddingBottom();
                int want = getPaddingTop() + rowOffset(align, box, lm.getDecoratedMeasuredHeight(v));
                int t = lm.getDecoratedTop(v);
                if (t == want || (t > want && !canScrollVertically(1))
                        || (t < want && !canScrollVertically(-1))) {
                    return;
                }
            }
            post(new Runnable() {
                public void run() {
                    if (gen == scrollGen && getAdapter() != null
                            && pos < getAdapter().getItemCount()) {
                        scrollRow(pos, align, false);
                    }
                }
            });
        }

        /** The reader's scrolls (docs/list.md "Following the end"), reported as they first move
         *  the list, so the viewport report that follows always differs from the one before:
         *  a drag (RecyclerView's SCROLL_STATE_DRAGGING, which a touch that catches a glide
         *  also enters), and the scroll a wheel, a key or an accessibility action starts.
         *  Day's own scrolls (scrollRow, the end and row props) settle without dragging and
         *  clear any input waiting to move the list; a reload or splice reports no movement. */
        @Override public void onScrolled(int dx, int dy) {
            super.onScrolled(dx, dy);
            if (!reader || dy == 0) return;
            if (getScrollState() == SCROLL_STATE_DRAGGING) {
                if (dragReported) return;
                dragReported = true;
            } else if (!readerInput) {
                return;
            }
            readerInput = false;
            nativeListReaderScroll(hostId);
        }

        /** Input that may start a scroll: its first move is the reader's. Expires when nothing
         *  moved within a few frames (a key that only moved focus, a wheel at the edge). */
        void readerInput() {
            if (!reader) return;
            readerInput = true;
            removeCallbacks(readerInputExpired);
            postDelayed(readerInputExpired, 150);
        }

        /** Day scrolls the list itself: input still waiting to move it is not the cause. */
        void appScroll() {
            readerInput = false;
            removeCallbacks(readerInputExpired);
        }

        /** A mouse wheel or trackpad scroll, which RecyclerView applies right away. */
        @Override public boolean onGenericMotionEvent(MotionEvent e) {
            if (ScrollTrack.isWheel(e)) readerInput();
            return super.onGenericMotionEvent(e);
        }

        /** The keys that move focus or the list: the focus change that follows scrolls the row
         *  into view. Only keys for the focused row or the list itself pass through here. */
        @Override public boolean dispatchKeyEvent(KeyEvent e) {
            if (e.getAction() == KeyEvent.ACTION_DOWN) {
                switch (e.getKeyCode()) {
                    case KeyEvent.KEYCODE_DPAD_UP: case KeyEvent.KEYCODE_DPAD_DOWN:
                    case KeyEvent.KEYCODE_PAGE_UP: case KeyEvent.KEYCODE_PAGE_DOWN:
                    case KeyEvent.KEYCODE_MOVE_HOME: case KeyEvent.KEYCODE_MOVE_END:
                    case KeyEvent.KEYCODE_TAB: case KeyEvent.KEYCODE_SPACE:
                        readerInput();
                        break;
                    default:
                        break;
                }
            }
            return super.dispatchKeyEvent(e);
        }

        /** An accessibility service's scroll (TalkBack's scroll gestures, Switch Access). */
        @Override public boolean performAccessibilityAction(int action, android.os.Bundle args) {
            if (action == android.view.accessibility.AccessibilityNodeInfo.ACTION_SCROLL_FORWARD
                    || action == android.view.accessibility.AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD
                    || action == android.R.id.accessibilityActionScrollUp
                    || action == android.R.id.accessibilityActionScrollDown
                    || action == android.R.id.accessibilityActionScrollToPosition) {
                readerInput();
            }
            return super.performAccessibilityAction(action, args);
        }

        /** Where a row `h` px tall goes for an alignment, from the top of a `box` px viewport. */
        static int rowOffset(int align, int box, int h) {
            if (align == LIST_ALIGN_CENTER) return (box - h) / 2;
            if (align == LIST_ALIGN_END) return box - h;
            return 0;
        }

        /** The rows changed: the next report goes out even with the same numbers. */
        void dataChanged() {
            reportedFirst = reportedLast = reportedEnd = -1;
            pinStale();
        }

        /** Re-place the pinned header for the current scroll offset: called from the scroll
         *  listener and after every layout pass, which is when rows move under it. */
        void updatePin() {
            if (!sections) return;
            LinearLayoutManager lm = (LinearLayoutManager) getLayoutManager();
            int first = lm == null ? NO_POSITION : lm.findFirstVisibleItemPosition();
            int header = first == NO_POSITION || getWidth() == 0
                    ? -1 : nativeListHeaderOf(hostId, first);
            int top = getPaddingTop();
            ViewHolder real = header < 0 ? null : findViewHolderForAdapterPosition(header);
            if (header < 0 || (real != null && real.itemView.getTop() >= top)) {
                hidePin();
                return;
            }
            if (pinned == null) {
                pinned = new DayFixed(getContext()) {
                    // A cell in an overlay has no parent that lays it out: dirty the list
                    // instead, whose next draw lays the cell out (dispatchDraw below).
                    @Override public void requestLayout() {
                        super.requestLayout();
                        DayListView.this.invalidate();
                    }
                };
                pinned.setLayoutParams(new ViewGroup.LayoutParams(
                        ViewGroup.LayoutParams.MATCH_PARENT, rowHeightPx));
                pinned.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
                getOverlay().add(pinned);
            }
            if (header != pinnedRow) {
                pinnedRow = header;
                // The bind every row takes, for a pinned copy: Day lays the header at its own
                // height and hands that back through setListCellHeight, and the copy carries no
                // element ids, so lookups find the real header row.
                nativeListBindPinned(hostId, header, pinned);
                // Opaque, so the rows scrolling under it never show through, and the color
                // the real header row shows through it: the first opaque fill behind the list.
                // Resolved per bind, so it follows a light/dark change.
                pinned.setBackgroundColor(backdrop());
            }
            int height = pinHeight();
            // The next header on screen pushes the pinned one up rather than sliding under it.
            int offset = 0;
            for (int i = 0; i < getChildCount(); i++) {
                View child = getChildAt(i);
                ViewHolder vh = getChildViewHolder(child);
                if (vh == null || vh.getItemViewType() != LIST_HEADER) continue;
                int pos = vh.getBindingAdapterPosition();
                if (pos == NO_POSITION || pos <= header) continue;
                offset = Math.min(offset, child.getTop() - top - height);
            }
            layoutPin(height);
            pinned.setTranslationY(top + offset);
            if (!pinShown) {
                pinShown = true;
                pinned.setVisibility(View.VISIBLE);
            }
        }

        /** The color behind the list's rows: the nearest opaque fill on the list or an
         *  ancestor (an app's pane color, a card), else Material's surface. */
        int backdrop() {
            for (View v = this; v != null;
                    v = v.getParent() instanceof View ? (View) v.getParent() : null) {
                android.graphics.drawable.Drawable bg = v.getBackground();
                int color = 0;
                if (bg instanceof android.graphics.drawable.ColorDrawable) {
                    color = ((android.graphics.drawable.ColorDrawable) bg).getColor();
                } else if (bg instanceof GradientDrawable
                        && ((GradientDrawable) bg).getColor() != null) {
                    color = ((GradientDrawable) bg).getColor().getDefaultColor();
                }
                if (bg != null && bg.getAlpha() == 255 && (color >>> 24) == 0xFF) return color;
                // A styled surface paints itself and has no background drawable to ask.
                if (v instanceof DayFixed && ((DayFixed) v).opaqueFill() != 0) {
                    return ((DayFixed) v).opaqueFill();
                }
            }
            return MaterialColors.getColor(this,
                    com.google.android.material.R.attr.colorSurface, 0xFFFFFFFF);
        }

        int pinHeight() {
            ViewGroup.LayoutParams lp = pinned.getLayoutParams();
            return lp != null && lp.height > 0 ? lp.height : rowHeightPx;
        }

        void layoutPin(int height) {
            int w = getWidth();
            if (!pinned.isLayoutRequested() && pinned.getWidth() == w
                    && pinned.getHeight() == height) {
                return;
            }
            pinned.measure(MeasureSpec.makeMeasureSpec(w, MeasureSpec.EXACTLY),
                    MeasureSpec.makeMeasureSpec(height, MeasureSpec.EXACTLY));
            pinned.layout(0, 0, w, height);
        }

        void hidePin() {
            if (pinned == null || !pinShown) return;
            pinShown = false;
            pinned.setVisibility(View.INVISIBLE);
            // Unbind: a hidden copy must not answer the header's dayscript ids.
            nativeListRecycle(hostId, pinned);
            pinnedRow = -1;
        }

        /** The rows changed (a reload): the pinned row may now name another section. */
        void pinStale() {
            pinnedRow = -1;
        }

        /** A touch on the visible pinned header goes to the list itself: it still scrolls the
         *  list, but never presses or selects the row hidden underneath. */
        @Override public boolean onInterceptTouchEvent(MotionEvent e) {
            boolean intercepted = super.onInterceptTouchEvent(e);
            if (pinShown && e.getActionMasked() == MotionEvent.ACTION_DOWN
                    && e.getY() < pinned.getTranslationY() + pinned.getHeight()) {
                return true;
            }
            return intercepted;
        }

        @Override protected void onLayout(boolean changed, int l, int t, int r, int b) {
            super.onLayout(changed, l, t, r, b);
            updatePin();
            reportViewport();
        }

        @Override protected void dispatchDraw(android.graphics.Canvas canvas) {
            if (pinShown && pinned.isLayoutRequested()) layoutPin(pinHeight());
            super.dispatchDraw(canvas);
        }
    }

    /** Mark a list as sectioned (docs/list.md "Sections"): header rows take their own view type
     *  and the current one pins over the top. Called once, right after makeList. */
    public static void listSetSections(View v, boolean on) {
        if (v instanceof DayListView) ((DayListView) v).sections = on;
    }

    /** The pinned header cell of a sectioned list, or null: released with the list. */
    public static View listPinnedCell(View v) {
        return v instanceof DayListView ? ((DayListView) v).pinned : null;
    }

    /** A native recycling list (docs/list.md): a RecyclerView (the platform's recycling
     *  widget) whose adapter reuses DayFixed cells, day filling each via nativeListBind.
     *  With `reorderable`, an ItemTouchHelper drives the native drag-to-reorder (long-press
     *  lift, elevation, incremental row swaps): every hover is vetted synchronously through
     *  nativeListCanDrop (the app's guard) and each swap commits through nativeListMove. */
    public static View makeList(final long hostId, final int rowHeightPx, final boolean selectable,
                                final boolean reorderable, final boolean deletable,
                                final String deleteLabel) {
        final DayListView rv = new DayListView(ctx, hostId, rowHeightPx);
        if (skipTransitions()) rv.setItemAnimator(null);
        rv.setLayoutManager(new LinearLayoutManager(ctx));
        rv.addOnScrollListener(new RecyclerView.OnScrollListener() {
            int previous = -1;
            @Override public void onScrolled(RecyclerView view, int dx, int dy) {
                if (rv.viewport) {
                    // Every scroll, the user's or a programmatic one, each step of an animated
                    // one included (docs/list.md "Visible rows").
                    rv.reportViewport();
                } else {
                    int first = ((LinearLayoutManager) view.getLayoutManager()).findFirstVisibleItemPosition();
                    if (first >= 0 && first != previous) {
                        previous = first;
                        nativeOnEvent(hostId, K_CUSTOM, first, "day-list-first-visible");
                    }
                }
                rv.updatePin();
            }
        });
        rv.setAdapter(new RecyclerView.Adapter<DayCellHolder>() {
            public int getItemCount() { return nativeListLen(hostId); }
            @Override public int getItemViewType(int position) {
                return rv.rowKind(position);
            }
            public DayCellHolder onCreateViewHolder(ViewGroup parent, int viewType) {
                DayFixed cell = new DayFixed(ctx);
                cell.setLayoutParams(new RecyclerView.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, rowHeightPx));
                if (viewType == LIST_SUPPLEMENT) {
                    // The list's own header, footer, empty state or padding: content that
                    // scrolls with the rows, not a row. No ripple, no click, no focus stop;
                    // Day sizes it at bind, whatever the items' pitch.
                    cell.setFocusable(false);
                } else if (viewType == LIST_HEADER) {
                    // A section header (docs/list.md "Sections"): no ripple, no click, no focus
                    // stop for the arrow keys; Day sizes it to its content at bind, whatever the
                    // items' pitch. TalkBack reads it as a heading to navigate by.
                    cell.setFocusable(false);
                    androidx.core.view.ViewCompat.setAccessibilityHeading(cell, true);
                } else if (selectable) {
                    // Touch feedback, which a bare ViewGroup with a click listener does not have:
                    // a row that responds to a tap has to say so under the finger, and on Android
                    // that is the bounded ripple every Material list item draws.
                    //
                    // As the foreground, not the background: day fills this cell with its own
                    // views, and a background ripple would be painted underneath them and never
                    // seen. `android:foreground="?attr/selectableItemBackground"` is what a
                    // Material list item uses, for the same reason. The View pipes hotspot
                    // changes to the foreground too, so the ripple still starts at the finger.
                    setTouchFeedback(cell, true);
                }
                return new DayCellHolder(cell);
            }
            public void onBindViewHolder(DayCellHolder h, int position) {
                nativeListBind(hostId, position, h.itemView);
                if (selectable && h.getItemViewType() == LIST_ROW) {
                    // A rebound holder inherits the row's selection state; the sync patch
                    // (listPaintSelection) covers the holders already on screen.
                    paintSelected(h.itemView, nativeListIsSelected(hostId, position));
                    h.itemView.setOnClickListener(new View.OnClickListener() {
                        public void onClick(View v) {
                            int pos = h.getBindingAdapterPosition();
                            if (pos != RecyclerView.NO_POSITION) {
                                nativeOnEvent(hostId, K_SELECTION_CHANGED, pos, "");
                                nativeOnEvent(hostId, K_LIST_ACTIVATED, pos, "");
                            }
                        }
                    });
                }
            }
            public void onViewRecycled(DayCellHolder h) {
                // The pooled cell keeps its day content for the next bind, but its dayscript
                // ids must stop answering lookups (docs/list.md).
                nativeListRecycle(hostId, h.itemView);
            }
        });
        if (reorderable || deletable) {
            // One ItemTouchHelper drives both gestures: the platform arbitrates between a
            // long-press drag and a swipe itself, which is why they share a callback rather
            // than fighting over the same touch stream (docs/list.md).
            final android.graphics.Paint swipePaint = new android.graphics.Paint();
            swipePaint.setColor(0xFFB3261E); // M3 error container
            final android.graphics.Paint swipeText = new android.graphics.Paint();
            swipeText.setColor(0xFFFFFFFF);
            swipeText.setAntiAlias(true);
            swipeText.setTextSize(14f * ctx.getResources().getDisplayMetrics().scaledDensity);
            new ItemTouchHelper(new ItemTouchHelper.Callback() {
                public int getMovementFlags(RecyclerView r, RecyclerView.ViewHolder vh) {
                    // A section header never lifts or slides (docs/list.md "Sections"); a row
                    // drags within its section, which Day's guard enforces at each swap. A
                    // supplementary row is not a row at all.
                    if (vh.getItemViewType() != LIST_ROW) return 0;
                    int drag = reorderable ? (ItemTouchHelper.UP | ItemTouchHelper.DOWN) : 0;
                    // `START`, not `LEFT`: it resolves against the layout direction, so the gesture
                    // is a trailing-edge swipe in both LTR and RTL, the same edge iOS uses.
                    int swipe = 0;
                    if (deletable) {
                        int pos = vh.getBindingAdapterPosition();
                        // A guarded row reports NO swipe direction, so it never moves under the
                        // finger rather than sliding back after a refusal.
                        if (pos != RecyclerView.NO_POSITION && nativeListCanDelete(hostId, pos)) {
                            swipe = ItemTouchHelper.START;
                        }
                    }
                    return makeMovementFlags(drag, swipe);
                }

                /** The Material affordance: the row slides to reveal a red field carrying the
                 *  app's own delete word (or nothing, when it supplied none). */
                @Override public void onChildDraw(android.graphics.Canvas c, RecyclerView r,
                        RecyclerView.ViewHolder vh, float dX, float dY, int state,
                        boolean isActive) {
                    if (state == ItemTouchHelper.ACTION_STATE_SWIPE && dX != 0f) {
                        View row = vh.itemView;
                        float left = dX < 0 ? row.getRight() + dX : row.getLeft();
                        float right = dX < 0 ? row.getRight() : row.getLeft() + dX;
                        c.drawRect(left, row.getTop(), right, row.getBottom(), swipePaint);
                        if (deleteLabel != null && !deleteLabel.isEmpty()) {
                            float pad = 16f * ctx.getResources().getDisplayMetrics().density;
                            float ty = row.getTop() + (row.getHeight()
                                    - (swipeText.descent() + swipeText.ascent())) / 2f;
                            float tw = swipeText.measureText(deleteLabel);
                            // Keep the word inside the revealed field as it grows.
                            float tx = dX < 0 ? row.getRight() - pad - tw : row.getLeft() + pad;
                            if (Math.abs(dX) > tw + pad * 2f) c.drawText(deleteLabel, tx, ty, swipeText);
                        }
                    }
                    super.onChildDraw(c, r, vh, dX, dY, state, isActive);
                }
                @Override public boolean isLongPressDragEnabled() { return true; }
                @Override public boolean canDropOver(RecyclerView r, RecyclerView.ViewHolder cur,
                                                     RecyclerView.ViewHolder target) {
                    int from = cur.getBindingAdapterPosition();
                    int to = target.getBindingAdapterPosition();
                    return from != RecyclerView.NO_POSITION && to != RecyclerView.NO_POSITION
                        && target.getItemViewType() != LIST_SUPPLEMENT
                        && nativeListCanDrop(hostId, from, to);
                }
                public boolean onMove(RecyclerView r, RecyclerView.ViewHolder vh,
                                      RecyclerView.ViewHolder target) {
                    // ItemTouchHelper commits incrementally (one adjacent swap per callback
                    // while the row is dragged), so each step goes through day.
                    int from = vh.getBindingAdapterPosition();
                    int to = target.getBindingAdapterPosition();
                    if (from == RecyclerView.NO_POSITION || to == RecyclerView.NO_POSITION) return false;
                    if (!nativeListMove(hostId, from, to)) return false;
                    RecyclerView.Adapter<?> a = r.getAdapter();
                    if (a != null) a.notifyItemMoved(from, to);
                    return true;
                }
                public void onSwiped(RecyclerView.ViewHolder vh, int direction) {
                    int pos = vh.getBindingAdapterPosition();
                    if (pos == RecyclerView.NO_POSITION) return;
                    RecyclerView.Adapter<?> a = rv.getAdapter();
                    if (nativeListDelete(hostId, pos)) {
                        // nativeListDelete already shortened day's snapshot; tell the adapter
                        // so the removal animates out of the swipe instead of snapping via a
                        // reload.
                        if (a != null) {
                            a.notifyItemRemoved(pos);
                            if (rv.shown > 0) rv.shown--;
                        }
                    } else if (a != null) {
                        // Refused after the fact: put the row back where it was.
                        a.notifyItemChanged(pos);
                    }
                }
            }).attachToRecyclerView(rv);
        }
        return rv;
    }
    /** Give one row of a self-sizing list (RowHeight::Automatic) the height day laid it at, in
     *  px (docs/list.md "Row heights"). Called from the bind, before the layout manager measures
     *  the holder, and posted after a row was re-laid at a new height. The holder was made at
     *  an estimate; a row whose height already matches is left alone, so a rebind of an
     *  unchanged row costs no layout pass. */
    public static void setListCellHeight(View cell, int px) {
        if (cell == null || px <= 0) return;
        ViewGroup.LayoutParams lp = cell.getLayoutParams();
        if (lp == null || lp.height == px) return;
        lp.height = px;
        cell.setLayoutParams(lp);
    }
    /** Repaint the visible holders' selection state from day's record (ListPatch::Selected):
     *  newly bound holders take theirs in onBindViewHolder. Paint only; no events. */
    public static void listPaintSelection(View v, long hostId) {
        if (!(v instanceof RecyclerView)) return;
        RecyclerView rv = (RecyclerView) v;
        for (int i = 0; i < rv.getChildCount(); i++) {
            View child = rv.getChildAt(i);
            int pos = rv.getChildAdapterPosition(child);
            RecyclerView.ViewHolder vh = rv.getChildViewHolder(child);
            // A section header is never selected and keeps its own look.
            if (pos != RecyclerView.NO_POSITION && vh.getItemViewType() == LIST_ROW) {
                paintSelected(child, nativeListIsSelected(hostId, pos));
            }
        }
    }
    /** A selected row's fill: Material's secondary container, the color the navigation rail and
     *  drawer give their selected item. Resolved from the cell's own themed context on every
     *  paint, so it follows a light/dark change. As the background: day's row content paints
     *  above it, and the ripple foreground stays free for touch feedback. */
    static void paintSelected(View cell, boolean on) {
        cell.setActivated(on);
        cell.setBackgroundColor(on
                ? MaterialColors.getColor(cell,
                        com.google.android.material.R.attr.colorSecondaryContainer, 0x33888888)
                : 0x00000000);
    }

    /** What the attached source offers (docs/list.md): `viewport`, Day observes the viewport
     *  (`ListSource::viewport`); `reader`, Day follows the end and hears the reader's scrolls
     *  (`ListSource::reader_scroll`); `supplements`, the list has supplementary rows
     *  (`ListSource::is_supplement`). Called by attach_list, before its reload binds a row. */
    public static void listConfigure(View v, boolean viewport, boolean reader,
                                     boolean supplements) {
        if (!(v instanceof DayListView)) return;
        DayListView rv = (DayListView) v;
        rv.viewport = viewport;
        rv.reader = reader;
        rv.supplements = supplements;
        rv.dataChanged();
    }

    public static void listReload(View v) {
        if (v instanceof DayListView) {
            DayListView rv = (DayListView) v;
            rv.dataChanged();
            rv.shown = nativeListLen(rv.hostId);
        }
        if (v instanceof RecyclerView && ((RecyclerView) v).getAdapter() != null) {
            ((RecyclerView) v).getAdapter().notifyDataSetChanged();
        }
    }

    /** Apply Day's row deltas (ListPatch::Splice, docs/list.md "Keyed updates") as the
     *  adapter's own range notifications, in order: triples of SPLICE_* kind and two ints
     *  (lib.rs `list_wire::splice_ops`). The item animator animates them unless reduce motion
     *  took it away.
     *
     *  RecyclerView throws (an inconsistency in its next layout) when the notifications do not
     *  carry the count it last saw to the count the adapter now reports. So the operations are
     *  checked first against `shown`, and they must land on Day's count; a splice that would not
     *  (a delta missed by a code path that changed the rows without telling the list) becomes
     *  a reload that keeps the first visible row where it was. */
    public static void listSplice(View v, int[] ops) {
        if (!(v instanceof DayListView)) {
            listReload(v);
            return;
        }
        DayListView rv = (DayListView) v;
        RecyclerView.Adapter<?> a = rv.getAdapter();
        LinearLayoutManager lm = (LinearLayoutManager) rv.getLayoutManager();
        if (a == null || lm == null) return;
        rv.dataChanged();
        int count = rv.shown;
        boolean valid = count >= 0 && ops != null && ops.length % 3 == 0;
        for (int i = 0; valid && i < ops.length; i += 3) {
            int at = ops[i + 1], b = ops[i + 2];
            switch (ops[i]) {
                case SPLICE_INSERT: valid = at >= 0 && at <= count && b > 0; count += b; break;
                case SPLICE_REMOVE: valid = at >= 0 && b > 0 && at + b <= count; count -= b; break;
                case SPLICE_MOVE: valid = at >= 0 && at < count && b >= 0 && b < count; break;
                default: valid = false;
            }
        }
        int actual = nativeListLen(rv.hostId);
        if (!valid || count != actual) {
            android.util.Log.w("day", "list splice does not match the source (" + rv.shown
                    + " rows, ends at " + count + ", source has " + actual + "): reloading");
            int first = lm.findFirstVisibleItemPosition();
            View top = first == RecyclerView.NO_POSITION ? null : lm.findViewByPosition(first);
            int offset = top == null ? 0 : lm.getDecoratedTop(top) - rv.getPaddingTop();
            rv.shown = actual;
            a.notifyDataSetChanged();
            if (first != RecyclerView.NO_POSITION && actual > 0) {
                lm.scrollToPositionWithOffset(Math.min(first, actual - 1), offset);
            }
            return;
        }
        // A list resting at its top stays there, as a scroll view's offset would: the layout
        // manager otherwise anchors on the row that was first, and a row inserted or moved
        // above it lands out of view.
        boolean atTop = rv.getChildCount() > 0 && !rv.canScrollVertically(-1);
        for (int i = 0; i < ops.length; i += 3) {
            switch (ops[i]) {
                case SPLICE_INSERT: a.notifyItemRangeInserted(ops[i + 1], ops[i + 2]); break;
                case SPLICE_REMOVE: a.notifyItemRangeRemoved(ops[i + 1], ops[i + 2]); break;
                default: a.notifyItemMoved(ops[i + 1], ops[i + 2]); break;
            }
        }
        rv.shown = actual;
        if (atTop && actual > 0) lm.scrollToPositionWithOffset(0, 0);
    }

    /** `listScroll` alignments (lib.rs `list_wire::align_code`). */
    static final int LIST_ALIGN_NEAREST = 0;
    static final int LIST_ALIGN_START = 1;
    static final int LIST_ALIGN_CENTER = 2;
    static final int LIST_ALIGN_END = 3;

    /** Scroll the list to row `row`, or to the very end of its content for -1
     *  (Toolkit::list_scroll, docs/list.md "Programmatic scrolling"). Posted: the scroll may
     *  arrive in the same turn as the splice or reload that made its row, and a post runs
     *  after the layout pass those requested. */
    public static void listScroll(View v, final int row, final int align, final boolean animated) {
        if (!(v instanceof DayListView)) return;
        final DayListView rv = (DayListView) v;
        rv.post(new Runnable() {
            private int tries = 0;
            public void run() {
                RecyclerView.Adapter<?> a = rv.getAdapter();
                int n = a == null ? 0 : a.getItemCount();
                if (n == 0) return;
                // Not laid out yet, or rows still pending: after the next layout pass
                // (bounded, in case the list never lays out).
                if ((rv.getHeight() == 0 || rv.hasPendingAdapterUpdates() || rv.isLayoutRequested())
                        && tries++ < 10) {
                    rv.post(this);
                    return;
                }
                if (row < 0) rv.scrollRow(n - 1, LIST_ALIGN_END, animated);
                else rv.scrollRow(Math.min(row, n - 1), align, animated);
            }
        });
    }
    /** Scroll the list so row `row` is visible (docs/list.md), realizing it if needed. Posted
     *  like listScrollToEnd; clamped to the count; no-op when empty. */
    public static void listScrollToRow(View v, final int row) {
        if (!(v instanceof RecyclerView)) return;
        final RecyclerView rv = (RecyclerView) v;
        if (rv instanceof DayListView) ((DayListView) rv).appScroll();
        rv.post(new Runnable() {
            public void run() {
                RecyclerView.Adapter<?> a = rv.getAdapter();
                int n = (a == null) ? 0 : a.getItemCount();
                if (n > 0) rv.scrollToPosition(Math.min(row, n - 1));
            }
        });
    }
    /** Scroll the list so its last row is fully visible (a chat sticking to the newest message).
     *  Posted so it runs after any pending notifyDataSetChanged relayout; no-op when empty. */
    public static void listScrollToEnd(View v) {
        if (!(v instanceof RecyclerView)) return;
        final RecyclerView rv = (RecyclerView) v;
        if (rv instanceof DayListView) ((DayListView) rv).appScroll();
        rv.post(new Runnable() {
            public void run() {
                RecyclerView.Adapter<?> a = rv.getAdapter();
                int n = (a == null) ? 0 : a.getItemCount();
                if (n > 0) {
                    if (skipTransitions()) rv.scrollToPosition(n - 1);
                    else rv.smoothScrollToPosition(n - 1);
                }
            }
        });
    }

    /** `makeScroll` preference bits (lib.rs `list_wire::scroll_flags`, docs/scroll.md). */
    static final int SCROLL_HIDE_INDICATORS = 1;
    static final int SCROLL_BOUNCE_NEVER = 2;
    static final int SCROLL_BOUNCE_ALWAYS = 4;
    static final int SCROLL_SNAP_PAGE = 8;
    static final int SCROLL_SNAP_INTERVAL = 16;
    static final int SCROLL_KEYBOARD_DRAG = 32;
    static final int SCROLL_KEYBOARD_TAPS = 64;
    static final int SCROLL_TRACK_OFFSET = 128;
    static final int SCROLL_TRACK_PHASE = 256;
    static final int PHASE_IDLE = 0, PHASE_DRAGGING = 1, PHASE_COASTING = 2;

    /** What a Day scroll view adds to ScrollView / HorizontalScrollView (docs/scroll.md), one
     *  per view, fed by both subclasses' overrides:
     *  - the offset, sent at most once per frame on the custom channel and always once more
     *    after the last change ("day:scroll:x,y", content px from the top-left corner);
     *  - the phase, which ScrollView has no callback for: Dragging when the content moves under
     *    a finger (or a wheel turns), Coasting when the finger lifts into a fling or a snap,
     *    Idle when the content has held still for a few frames or the finger lifted with no
     *    momentum ("day:scroll-phase:n");
     *  - snapping, by taking over fling() and the release: the stop a fling would reach
     *    (predicted with the same OverScroller physics ScrollView flings with), rounded to a
     *    page or an interval, then smoothScrollTo it;
     *  - the software keyboard, hidden when a drag begins and, with KeyboardTaps::Dismiss, by a
     *    tap that is not on a text field, which the scroll view then keeps from its children. */
    static final class ScrollTrack {
        final ViewGroup view;
        final long node;
        final int flags;
        /** The snap interval in px, unrounded: stop k is at round(k * snapPx). */
        final float snapPx;
        final boolean horizontal;
        final int slop;
        int phase = PHASE_IDLE;
        boolean fingerDown, flung, tapDismiss, offsetPosted, watching;
        float downX, downY;
        int startLead, lastPos, stillFrames;
        final Runnable sendOffset = new Runnable() {
            public void run() {
                offsetPosted = false;
                // Content space, as every toolkit reports it and as Toolkit::scroll_to takes
                // it: an RTL horizontal view still counts from its left edge. Only the
                // snapping below measures from the leading edge.
                int at = raw();
                nativeOnEvent(node, K_CUSTOM, 0, "day:scroll:"
                        + (horizontal ? at + ",0" : "0," + at));
            }
        };
        final Runnable watch = new Runnable() {
            public void run() {
                if (phase != PHASE_COASTING) {
                    watching = false;
                    return;
                }
                int p = raw();
                if (p == lastPos) stillFrames++;
                else {
                    stillFrames = 0;
                    lastPos = p;
                }
                if (stillFrames >= 4) {
                    watching = false;
                    enter(PHASE_IDLE);
                } else {
                    view.postOnAnimation(this);
                }
            }
        };
        final Runnable wheelIdle = new Runnable() {
            public void run() {
                if (fingerDown || phase != PHASE_DRAGGING) return;
                enter(snapRest() ? PHASE_COASTING : PHASE_IDLE);
            }
        };

        ScrollTrack(ViewGroup view, long node, int flags, float snapPx, boolean horizontal) {
            this.view = view;
            this.node = node;
            this.flags = flags;
            this.snapPx = snapPx;
            this.horizontal = horizontal;
            this.slop = android.view.ViewConfiguration.get(view.getContext()).getScaledTouchSlop();
        }

        boolean has(int bit) { return (flags & bit) != 0; }
        boolean snaps() { return has(SCROLL_SNAP_PAGE) || has(SCROLL_SNAP_INTERVAL); }
        int raw() { return horizontal ? view.getScrollX() : view.getScrollY(); }
        int extent() {
            return horizontal ? view.getWidth() - view.getPaddingLeft() - view.getPaddingRight()
                    : view.getHeight() - view.getPaddingTop() - view.getPaddingBottom();
        }
        int range() {
            if (view.getChildCount() == 0) return 0;
            View c = view.getChildAt(0);
            return Math.max(0, (horizontal ? c.getWidth() : c.getHeight()) - extent());
        }
        boolean rtl() {
            return horizontal && view.getLayoutDirection() == View.LAYOUT_DIRECTION_RTL;
        }
        /** A raw scroll position from the leading edge (an RTL horizontal view counts from
         *  its right edge), and back: the same map both ways. */
        int lead(int raw) { return rtl() ? range() - raw : raw; }

        void scrolled() {
            if (has(SCROLL_TRACK_OFFSET) && !offsetPosted) {
                offsetPosted = true;
                view.postOnAnimation(sendOffset);
            }
            if (fingerDown && phase != PHASE_DRAGGING) enter(PHASE_DRAGGING);
        }

        void enter(int next) {
            if (next == phase) return;
            phase = next;
            if (has(SCROLL_TRACK_PHASE)) nativeOnEvent(node, K_CUSTOM, 0, "day:scroll-phase:" + next);
            if (next == PHASE_DRAGGING && fingerDown && has(SCROLL_KEYBOARD_DRAG)) dismissKeyboard();
            if (next == PHASE_COASTING && !watching) {
                watching = true;
                lastPos = raw();
                stillFrames = 0;
                view.postOnAnimation(watch);
            }
        }

        /** Before the view dispatches a touch. */
        void touchBefore(MotionEvent e) {
            if (e.getActionMasked() != MotionEvent.ACTION_DOWN) return;
            fingerDown = true;
            flung = false;
            downX = e.getX();
            downY = e.getY();
            startLead = lead(raw());
            // A finger on coasting content stops it (ScrollView aborts the fling at the down).
            if (phase == PHASE_COASTING) enter(PHASE_IDLE);
        }

        /** After the view dispatched a touch: its onTouchEvent may have flung on the up. */
        void touchAfter(MotionEvent e) {
            int action = e.getActionMasked();
            if (action == MotionEvent.ACTION_MOVE && tapDismiss
                    && Math.hypot(e.getX() - downX, e.getY() - downY) > slop) {
                tapDismiss = false;
            }
            if (action != MotionEvent.ACTION_UP && action != MotionEvent.ACTION_CANCEL) return;
            fingerDown = false;
            if (tapDismiss) {
                tapDismiss = false;
                if (action == MotionEvent.ACTION_UP) dismissKeyboard();
            }
            if (flung) {
                if (phase == PHASE_DRAGGING) enter(PHASE_COASTING);
            } else if (snapRest()) {
                enter(PHASE_COASTING);
            } else if (phase == PHASE_DRAGGING) {
                enter(PHASE_IDLE);
            }
        }

        /** The view's onInterceptTouchEvent: true keeps a keyboard-dismissing tap from the
         *  children (the view's own onTouchEvent still scrolls if it turns into a drag). */
        boolean intercept(MotionEvent e) {
            if (e.getActionMasked() != MotionEvent.ACTION_DOWN) return tapDismiss;
            tapDismiss = has(SCROLL_KEYBOARD_TAPS) && imeVisible(view)
                    && !textEditorAt(view, e.getX(), e.getY());
            return tapDismiss;
        }

        /** The view's fling(): true when snapping took it over. */
        boolean fling(int velocity) {
            flung = true;
            if (!snaps()) return false;
            int v = rtl() ? -velocity : velocity;
            float unit = snapUnit();
            if (unit < 1f) return false;
            int lead = lead(raw());
            int stop;
            if (has(SCROLL_SNAP_PAGE)) {
                // The next page in the fling's direction, never more than one from the page
                // the swipe started on.
                stop = (int) (v > 0 ? Math.ceil(lead / unit) : Math.floor(lead / unit));
                int start = Math.round(startLead / unit);
                stop = Math.max(start - 1, Math.min(start + 1, stop));
            } else {
                android.widget.OverScroller predict = new android.widget.OverScroller(view.getContext());
                predict.fling(0, lead, 0, v, 0, 0, 0, range());
                stop = Math.round(predict.getFinalY() / unit);
                predict.abortAnimation();
            }
            smoothTo(Math.round(stop * unit));
            return true;
        }

        /** At rest off a stop: glide to the nearest one. True when it moves. */
        boolean snapRest() {
            if (!snaps()) return false;
            float unit = snapUnit();
            if (unit < 1f) return false;
            int lead = lead(raw());
            int stop = Math.round(lead / unit);
            if (has(SCROLL_SNAP_PAGE)) {
                int start = Math.round(startLead / unit);
                stop = Math.max(start - 1, Math.min(start + 1, stop));
            }
            int target = Math.max(0, Math.min(range(), Math.round(stop * unit)));
            if (target == lead) return false;
            smoothTo(target);
            return true;
        }

        /** The distance between stops: the viewport for paging, else the interval. */
        float snapUnit() { return has(SCROLL_SNAP_PAGE) ? extent() : snapPx; }

        void smoothTo(int lead) {
            int clamped = Math.max(0, Math.min(range(), lead));
            int raw = rtl() ? range() - clamped : clamped;
            if (view instanceof HorizontalScrollView) ((HorizontalScrollView) view).smoothScrollTo(raw, 0);
            else if (view instanceof ScrollView) ((ScrollView) view).smoothScrollTo(0, raw);
        }

        /** A wheel or trackpad scroll the view handled: a short drag of its own. */
        void wheel() {
            if (phase == PHASE_IDLE) {
                startLead = lead(raw());
                enter(PHASE_DRAGGING);
            }
            view.removeCallbacks(wheelIdle);
            view.postDelayed(wheelIdle, 150);
        }

        void dismissKeyboard() {
            if (!imeVisible(view)) return;
            InputMethodManager imm = (InputMethodManager)
                    view.getContext().getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm != null) imm.hideSoftInputFromWindow(view.getWindowToken(), 0);
            View focused = view.getRootView().findFocus();
            if (focused != null && focused.onCheckIsTextEditor()) focused.clearFocus();
        }

        /** Whether the software keyboard is up (WindowInsetsCompat: exact on API 30+, inferred
         *  from the root insets below). */
        static boolean imeVisible(View v) {
            androidx.core.view.WindowInsetsCompat insets = androidx.core.view.ViewCompat.getRootWindowInsets(v);
            return insets != null && insets.isVisible(androidx.core.view.WindowInsetsCompat.Type.ime());
        }

        /** Whether the touch at (x, y) in `v`'s coordinates lands on a text input. */
        static boolean textEditorAt(View v, float x, float y) {
            if (v.onCheckIsTextEditor()) return true;
            if (!(v instanceof ViewGroup)) return false;
            ViewGroup g = (ViewGroup) v;
            for (int i = g.getChildCount() - 1; i >= 0; i--) {
                View c = g.getChildAt(i);
                if (c.getVisibility() != View.VISIBLE) continue;
                float cx = x + g.getScrollX() - c.getLeft() - c.getTranslationX();
                float cy = y + g.getScrollY() - c.getTop() - c.getTranslationY();
                if (cx >= 0 && cy >= 0 && cx < c.getWidth() && cy < c.getHeight()) {
                    return textEditorAt(c, cx, cy);
                }
            }
            return false;
        }

        static boolean isWheel(MotionEvent e) {
            return e.getActionMasked() == MotionEvent.ACTION_SCROLL
                    && (e.getSource() & android.view.InputDevice.SOURCE_CLASS_POINTER) != 0;
        }
    }

    /** The scroll piece (docs/scroll.md): a ScrollView or HorizontalScrollView with its
     *  preferences (SCROLL_* bits) and a ScrollTrack. `snapPx` is the interval for
     *  SCROLL_SNAP_INTERVAL; `node` is where the offset and phase reports go. */
    public static View makeScroll(boolean horizontal, int flags, float snapPx, long node) {
        // The onSizeChanged overrides are the keyboard-avoidance reveal (docs/focus.md): when
        // this viewport SHRINKS while a descendant holds focus (the IME just rose and Day
        // relaid out), scroll the focused view back in. Posted so it runs after the layout
        // pass that delivered the new size; requestRectangleOnScreen is a minimal scroll.
        ViewGroup sv;
        final ScrollTrack[] track = new ScrollTrack[1];
        if (horizontal) {
            sv = new HorizontalScrollView(ctx) {
                @Override protected void onSizeChanged(int w, int h, int oldW, int oldH) {
                    super.onSizeChanged(w, h, oldW, oldH);
                    if (w < oldW) post(new Runnable() { public void run() { revealFocus(); } });
                }
                private void revealFocus() {
                    View f = findFocus();
                    if (f != null && f != this) {
                        f.requestRectangleOnScreen(new android.graphics.Rect(
                                0, 0, f.getWidth(), f.getHeight()), false);
                    }
                }
                @Override public boolean dispatchTouchEvent(MotionEvent e) {
                    track[0].touchBefore(e);
                    boolean handled = super.dispatchTouchEvent(e);
                    track[0].touchAfter(e);
                    return handled;
                }
                @Override public boolean onInterceptTouchEvent(MotionEvent e) {
                    boolean intercepted = super.onInterceptTouchEvent(e);
                    return track[0].intercept(e) || intercepted;
                }
                @Override public void fling(int velocityX) {
                    if (!track[0].fling(velocityX)) super.fling(velocityX);
                }
                @Override protected void onScrollChanged(int l, int t, int oldL, int oldT) {
                    super.onScrollChanged(l, t, oldL, oldT);
                    track[0].scrolled();
                }
                @Override public boolean onGenericMotionEvent(MotionEvent e) {
                    boolean handled = super.onGenericMotionEvent(e);
                    if (handled && ScrollTrack.isWheel(e)) track[0].wheel();
                    return handled;
                }
            };
        } else {
            sv = new ScrollView(ctx) {
                @Override protected void onSizeChanged(int w, int h, int oldW, int oldH) {
                    super.onSizeChanged(w, h, oldW, oldH);
                    if (h < oldH) post(new Runnable() { public void run() { revealFocus(); } });
                }
                private void revealFocus() {
                    View f = findFocus();
                    if (f != null && f != this) {
                        f.requestRectangleOnScreen(new android.graphics.Rect(
                                0, 0, f.getWidth(), f.getHeight()), false);
                    }
                }
                @Override public boolean dispatchTouchEvent(MotionEvent e) {
                    track[0].touchBefore(e);
                    boolean handled = super.dispatchTouchEvent(e);
                    track[0].touchAfter(e);
                    return handled;
                }
                @Override public boolean onInterceptTouchEvent(MotionEvent e) {
                    boolean intercepted = super.onInterceptTouchEvent(e);
                    return track[0].intercept(e) || intercepted;
                }
                @Override public void fling(int velocityY) {
                    if (!track[0].fling(velocityY)) super.fling(velocityY);
                }
                @Override protected void onScrollChanged(int l, int t, int oldL, int oldT) {
                    super.onScrollChanged(l, t, oldL, oldT);
                    track[0].scrolled();
                }
                @Override public boolean onGenericMotionEvent(MotionEvent e) {
                    boolean handled = super.onGenericMotionEvent(e);
                    if (handled && ScrollTrack.isWheel(e)) track[0].wheel();
                    return handled;
                }
            };
        }
        track[0] = new ScrollTrack(sv, node, flags, snapPx, horizontal);
        if (sv instanceof ScrollView) ((ScrollView) sv).setFillViewport(false);
        else ((HorizontalScrollView) sv).setFillViewport(false);
        if ((flags & SCROLL_HIDE_INDICATORS) != 0) {
            sv.setVerticalScrollBarEnabled(false);
            sv.setHorizontalScrollBarEnabled(false);
        }
        if ((flags & SCROLL_BOUNCE_NEVER) != 0) sv.setOverScrollMode(View.OVER_SCROLL_NEVER);
        else if ((flags & SCROLL_BOUNCE_ALWAYS) != 0) sv.setOverScrollMode(View.OVER_SCROLL_ALWAYS);
        sv.addView(new DayFixed(ctx));
        return sv;
    }
    public static View contentOf(View v) {
        if ((v instanceof ScrollView || v instanceof HorizontalScrollView)
                && ((ViewGroup) v).getChildCount() > 0) {
            return ((ViewGroup) v).getChildAt(0);
        }
        if (v instanceof DayCover) return ((DayCover) v).content;
        return v;
    }
    /** Minimal scroll so `[x,y,w,h]` (content px) is visible (scrollRectToVisible semantics).
     *  Serves both scroll axes; the widget clamps to its own scroll range. */
    public static void scrollToRect(final View v, final int x, final int y,
                                    final int w, final int h, final boolean animated) {
        main.post(new Runnable() {
            private int tries = 0;
            public void run() {
                // A freshly built page's widgets have no size until the next layout pass, so
                // defer until the scroll axis has a real viewport (bounded, in case the node
                // never lays out).
                boolean vertical = v instanceof android.widget.ScrollView;
                int viewport = vertical ? v.getHeight() : v.getWidth();
                if (viewport == 0 && tries++ < 10) {
                    main.post(this);
                    return;
                }
                if (v instanceof android.widget.ScrollView) {
                    android.widget.ScrollView sv = (android.widget.ScrollView) v;
                    int cur = sv.getScrollY();
                    int view = sv.getHeight();
                    int target = cur;
                    if (y + h > cur + view) target = y + h - view;
                    if (y < target) target = y;
                    if (animated) sv.smoothScrollTo(0, target);
                    else sv.scrollTo(0, target);
                } else if (v instanceof android.widget.HorizontalScrollView) {
                    android.widget.HorizontalScrollView sv = (android.widget.HorizontalScrollView) v;
                    int cur = sv.getScrollX();
                    int view = sv.getWidth();
                    int target = cur;
                    if (x + w > cur + view) target = x + w - view;
                    if (x < target) target = x;
                    if (animated) sv.smoothScrollTo(target, 0);
                    else sv.scrollTo(target, 0);
                }
            }
        });
    }

    /** Scroll the native navigation list without selecting or opening a destination. */
    public static boolean navScrollToEdge(View v, boolean bottom) {
        if (!(v instanceof NavigationView)) return false;
        androidx.recyclerview.widget.RecyclerView list = findNavigationList((ViewGroup) v);
        if (list == null) return false;
        main.post(() -> {
            androidx.recyclerview.widget.RecyclerView.Adapter<?> adapter = list.getAdapter();
            if (adapter == null || adapter.getItemCount() == 0) return;
            for (android.view.ViewParent parent = v.getParent(); parent != null;
                    parent = parent.getParent()) {
                if (parent instanceof DayNavHost) {
                    ((DayNavHost) parent).scrollNavigationChrome(bottom);
                    break;
                }
            }
            list.stopScroll();
            // The app bar changes the viewport on the next layout pass. Resolve the edge
            // after that pass, rather than positioning the last row against the old height.
            list.getViewTreeObserver().addOnPreDrawListener(
                    new android.view.ViewTreeObserver.OnPreDrawListener() {
                @Override public boolean onPreDraw() {
                    list.getViewTreeObserver().removeOnPreDrawListener(this);
                    list.scrollToPosition(bottom ? adapter.getItemCount() - 1 : 0);
                    return false;
                }
            });
            list.requestLayout();
        });
        return true;
    }

    private static androidx.recyclerview.widget.RecyclerView findNavigationList(ViewGroup group) {
        for (int i = 0; i < group.getChildCount(); i++) {
            View child = group.getChildAt(i);
            if (child instanceof androidx.recyclerview.widget.RecyclerView)
                return (androidx.recyclerview.widget.RecyclerView) child;
            if (child instanceof ViewGroup) {
                androidx.recyclerview.widget.RecyclerView found = findNavigationList((ViewGroup) child);
                if (found != null) return found;
            }
        }
        return null;
    }

    public static void setScrollContent(View v, int w, int h) {
        View content = contentOf(v);
        if (content instanceof DayFixed) ((DayFixed) content).setContentSize(w, h);
    }

    /** A label's `TextAlign`: 1 = center, 2 = trailing, 3 = justified. Leading keeps the default,
     *  which follows the layout direction. Justified stretches every line but the last
     *  (`JUSTIFICATION_MODE_INTER_WORD`, API 26); below that it stays leading. */
    public static void setLabelAlignment(View view, int align) {
        TextView t = (TextView) view;
        if (align == 3) {
            if (android.os.Build.VERSION.SDK_INT >= 26) {
                t.setJustificationMode(android.text.Layout.JUSTIFICATION_MODE_INTER_WORD);
            }
            return;
        }
        t.setTextAlignment(align == 1 ? View.TEXT_ALIGNMENT_CENTER : View.TEXT_ALIGNMENT_VIEW_END);
    }

    /** `.single_line()`: one line whatever the limit, ended as the label's truncation says. */
    public static void setLabelSingleLine(View view) {
        if (view instanceof DayLabel) {
            ((DayLabel) view).setDaySingleLine(true);
            return;
        }
        TextView label = (TextView) view;
        label.setSingleLine(true);
        label.setEllipsize(android.text.TextUtils.TruncateAt.END);
    }

    /** Cap a wrapping label at `lines` lines and end the last with an ellipsis; measurement honors the cap. */
    public static void setLabelMaxLines(View view, int lines) {
        TextView label = (TextView) view;
        label.setMaxLines(lines);
        label.setEllipsize(android.text.TextUtils.TruncateAt.END);
    }

    /**
     * A label's paragraph attributes (`LabelParagraph`, docs/text.md), whole and idempotent:
     * the line limit (0 = unlimited), the truncation (0 tail, 1 head, 2 middle), the letter
     * spacing in ems and the line height as a multiple of the font size (0 = the font's own).
     * The next measurement answers for them.
     */
    public static void setLabelParagraph(View view, int maxLines, int truncation, float letterSpacing, float lineHeight) {
        if (view instanceof DayLabel) {
            ((DayLabel) view).setParagraph(maxLines, truncation, letterSpacing, lineHeight);
        } else if (view instanceof TextView) {
            TextView t = (TextView) view;
            t.setLetterSpacing(letterSpacing);
            if (maxLines > 0) setLabelMaxLines(view, maxLines);
        }
    }

    /** Report the reader's selection in this label to Day node `node` (`Label::on_selection`). */
    public static void setLabelReportsSelection(View view, long node) {
        if (view instanceof DayLabel) ((DayLabel) view).reportSelection(node);
    }

    /** The whole label's OpenType features (`FontSpec::feature_tags`, comma-joined), after
     *  `setLabelFont`, which sets tabular figures alone. */
    public static void setLabelFeatures(View view, String features) {
        ((TextView) view).setFontFeatureSettings(features == null || features.isEmpty() ? null : features);
    }

    public static View makeLabel(String text) {
        DayLabel t = new DayLabel(ctx);
        t.setDayText(text);
        return t;
    }
    public static void setLabel(View v, String text) {
        if (v instanceof DayLabel) ((DayLabel) v).setDayText(text);
        else ((TextView) v).setText(text);
    }

    /**
     * One widget's reading direction (`Toolkit::set_layout_direction`, docs/localization.md
     * "Direction of a subtree"): its layout direction (a control's inner arrangement, which edge
     * `View.TEXT_ALIGNMENT_VIEW_END` means) and its text direction (a label's or a field's base
     * direction, so `Leading` is the right edge in a right-to-left subtree). Day places every
     * child itself, so Day's own hosts are left alone: on a view group the property would only
     * re-resolve start and end for children whose frames Day already mirrored.
     */
    public static void setViewDirection(View v, boolean rtl) {
        if (v instanceof DayFixed || v instanceof ScrollView || v instanceof HorizontalScrollView
                || v instanceof RecyclerView) {
            return;
        }
        v.setLayoutDirection(rtl ? View.LAYOUT_DIRECTION_RTL : View.LAYOUT_DIRECTION_LTR);
        v.setTextDirection(rtl ? View.TEXT_DIRECTION_RTL : View.TEXT_DIRECTION_LTR);
    }

    /**
     * The factor the reader's font size setting applies to text of `sp` scaled pixels
     * (`Toolkit::text_scale`, docs/text.md "Text scale"): the pixels Android gives that size
     * over the pixels it would give it at the default setting. Android 14 scales large sizes
     * less than small ones (non-linear font scaling), which `applyDimension` follows; before
     * that it is `Configuration.fontScale` for every size.
     */
    public static float textScale(float sp) {
        android.util.DisplayMetrics m = ctx.getResources().getDisplayMetrics();
        if (sp <= 0f || m.density <= 0f) return 1f;
        return TypedValue.applyDimension(TypedValue.COMPLEX_UNIT_SP, sp, m) / (sp * m.density);
    }

    /**
     * Set a label's text with styled RUNS (docs/text-runs.md).
     *
     * The runs arrive as flat parallel arrays rather than objects: one JNI call with three
     * primitive arrays beats N calls building a Java object per run, and this runs on every
     * label patch. Layout: starts[i], ends[i] are UTF-16 offsets (Java string indices; Rust
     * converts from its byte offsets), and flags[i] packs bold/italic/mono/strike/hasColor with
     * colors[i] holding the ARGB when the flag says so. `links[i]` is null for a plain run.
     * Flag bits: 1 bold, 2 italic, 4 monospace, 8 a plain strikethrough, 16 a color, 32 a
     * background, 64 a plain underline, 1024 small capitals (`smcp`), 2048 oldstyle figures
     * (`onum`), 4096 tabular figures (`tnum`). A decoration Android has no span for (a double,
     * dotted or wavy line, or one in its own color) arrives in `decorations` instead, five ints
     * per line (`DayLabel.DECO_STRIDE`), and the label strokes it over its layout.
     *
     * The spans are Android's own, so the text stays one TextView: it wraps, selects and is read
     * by TalkBack as a single paragraph, which is the entire point of runs.
     */
    public static void setLabelRuns(
            View v, final long node, String text, int[] starts, int[] ends, int[] flags, int[] colors,
            int[] backgrounds, int[] scales, String linksJoined, int[] decorations) {
        TextView tv = (TextView) v;
        boolean anyLink = false;
        android.text.SpannableString s = new android.text.SpannableString(text);
        String[] links = linksJoined.isEmpty() ? new String[0] : linksJoined.split("\u001f", -1);
        final int EXCL = android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE;
        for (int i = 0; i < starts.length; i++) {
            int a = Math.max(0, Math.min(starts[i], text.length()));
            int b = Math.max(a, Math.min(ends[i], text.length()));
            if (a == b) continue;
            int f = flags[i];
            boolean bold = (f & 1) != 0, italic = (f & 2) != 0;
            if (bold && italic) {
                s.setSpan(new android.text.style.StyleSpan(android.graphics.Typeface.BOLD_ITALIC), a, b, EXCL);
            } else if (bold) {
                s.setSpan(new android.text.style.StyleSpan(android.graphics.Typeface.BOLD), a, b, EXCL);
            } else if (italic) {
                s.setSpan(new android.text.style.StyleSpan(android.graphics.Typeface.ITALIC), a, b, EXCL);
            }
            if ((f & 4) != 0) s.setSpan(new android.text.style.TypefaceSpan("monospace"), a, b, EXCL);
            if ((f & 8) != 0) s.setSpan(new android.text.style.StrikethroughSpan(), a, b, EXCL);
            if ((f & 16) != 0) s.setSpan(new android.text.style.ForegroundColorSpan(colors[i]), a, b, EXCL);
            if ((f & 32) != 0) s.setSpan(new android.text.style.BackgroundColorSpan(backgrounds[i]), a, b, EXCL);
            // A plain single underline is the native span; any other style or color arrives in
            // `decorations` and DayLabel strokes it.
            if ((f & 64) != 0) s.setSpan(new android.text.style.UnderlineSpan(), a, b, EXCL);
            // Relative size (FontSpec::scale) as a RelativeSizeSpan, which is exactly its shape:
            // a multiplier over the inherited size, so the run still tracks the user's Font Size
            // setting rather than freezing at a pixel value.
            String tags = ((f & 4096) != 0 ? "tnum," : "") + ((f & 1024) != 0 ? "smcp," : "")
                    + ((f & 2048) != 0 ? "onum," : "");
            if (!tags.isEmpty()) {
                s.setSpan(new DayLabel.FeatureSpan(tags.substring(0, tags.length() - 1)), a, b, EXCL);
            }
            if (i < scales.length && scales[i] != 1000 && scales[i] > 0) {
                s.setSpan(new android.text.style.RelativeSizeSpan(scales[i] / 1000f), a, b, EXCL);
            }
            if (i < links.length && !links[i].isEmpty()) {
                // A ClickableSpan rather than a URLSpan: the target goes back to Rust, so the
                // app's `.on_link()` decides (route in-app, confirm, open) instead of Android
                // firing an implicit `VIEW` intent behind Day's back. It keeps URLSpan's
                // rendering (accent color and underline) from updateDrawState.
                final String target = links[i];
                s.setSpan(new android.text.style.ClickableSpan() {
                    @Override public void onClick(View widget) {
                        nativeOnEvent(node, K_LINK_ACTIVATED, 0.0, target);
                    }
                }, a, b, EXCL);
                anyLink = true;
            }
        }
        if (tv instanceof DayLabel) {
            ((DayLabel) tv).setDayText(s);
            ((DayLabel) tv).setDecorations(decorations);
        } else {
            tv.setText(s);
        }
        // Clicks on spans need a movement method. It is set only when a link is present: it
        // replaces the selection movement method, so a selectable label without links keeps
        // its selection behavior intact (docs/text-runs.md).
        if (anyLink) {
            tv.setMovementMethod(android.text.method.LinkMovementMethod.getInstance());
        }
    }
    /**
     * `sp` size (scales with the accessibility Font Size setting), font weight (100–900), italic,
     * and an optional bundled font family (null for the system font; §18.4).
     */
    public static void setLabelFont(
            View v, float sp, int weight, boolean italic, String family, boolean tabular) {
        TextView t = (TextView) v;
        // Tabular figures via the OpenType feature, so the typeface is untouched and only the
        // digits change metrics. A font without `tnum` ignores the request.
        t.setFontFeatureSettings(tabular ? "tnum" : null);
        // COMPLEX_UNIT_SP applies the user's font scale (Settings ▸ Display ▸ Font size), the
        // Android accessibility text scale; COMPLEX_UNIT_DIP does not.
        t.setTextSize(TypedValue.COMPLEX_UNIT_SP, sp);
        // The Rust whole-label monospace flag uses Android's system family alias.
        // It is not an app font resource and must never enter the bundled-font cache.
        Typeface base = "monospace".equals(family) ? Typeface.MONOSPACE
                : (family != null && !family.isEmpty()) ? bundledFont(family) : Typeface.DEFAULT;
        if (android.os.Build.VERSION.SDK_INT >= 28) {
            // Exact numeric weight + italic (API 28+); a custom base picks (or synthesizes) the
            // nearest face the family provides.
            t.setTypeface(Typeface.create(base, weight, italic));
        } else {
            int style = (weight >= 600 ? Typeface.BOLD : Typeface.NORMAL) | (italic ? Typeface.ITALIC : 0);
            t.setTypeface(Typeface.create(base, style));
        }
        // A line height is a multiple of the size just set.
        if (t instanceof DayLabel) ((DayLabel) t).fontChanged();
    }

    private static final java.util.Map<String, Typeface> FONT_CACHE = new java.util.HashMap<>();
    /**
     * Resolve a bundled font family (§18.4). `day build` stages each `fonts/` file as
     * `res/font/<ident>.ttf`, where `<ident>` is the font's family name sanitized to Android
     * resource rules (lowercase `[a-z0-9_]`, leading letter; the same derivation as day-spec's
     * `font_ident`). Re-derive the ident here and look up `R.font.<ident>`, so no side table is
     * needed. Unknown families (or API < 26, which predates font resources) log and fall back to
     * the system typeface.
     */
    private static Typeface bundledFont(String family) {
        Typeface tf = bundledFontOrNull(family);
        if (tf == null) {
            android.util.Log.w("DayBridge", "unknown font family \"" + family
                    + "\" — falling back to the system font (is the file in the project's fonts/ directory?)");
            tf = Typeface.DEFAULT;
            FONT_CACHE.put(family, tf);
        }
        return tf;
    }

    /** The bundled-font lookup of `bundledFont`, answering null (quietly) for an unknown family. */
    private static Typeface bundledFontOrNull(String family) {
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
                    // Broken resource: fall through to the loud default below.
                }
            }
        }
        if (tf != null) FONT_CACHE.put(family, tf);
        return tf;
    }
    /**
     * The theme's secondary text color: what a hint, a caption, or an empty state's "nothing
     * selected" wears (`Label::secondary`).
     *
     * Resolved from `?android:attr/textColorSecondary` rather than written as a constant, so it
     * follows the app's theme and the system's light/dark setting. A literal grey cannot: one
     * chosen against a light background is close to invisible on a dark one.
     */
    public static void setLabelSecondary(View v, boolean on) {
        TextView t = (TextView) v;
        if (!on) {
            t.setTextColor(new TextView(ctx).getTextColors());
            return;
        }
        android.content.res.TypedArray a =
                v.getContext().obtainStyledAttributes(new int[] {android.R.attr.textColorSecondary});
        android.content.res.ColorStateList csl = a.getColorStateList(0);
        a.recycle();
        if (csl != null) {
            t.setTextColor(csl);
        }
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

    /**
     * Each button's tint as Material styled it, so clearing an app tint can put the theme's
     * container color back.
     *
     * `setBackgroundTintList(null)` does not restore a default; it removes the tint outright, and
     * a MaterialButton's background is a shape drawable that the tint is what colors. Left null
     * it draws raw black, which is how every untinted button on this backend came out.
     */
    private static final java.util.Map<View, android.content.res.ColorStateList> buttonTints =
            new java.util.WeakHashMap<>();

    /** Each button's stock metrics {minWidth, minimumWidth, paddingStart, paddingEnd}, so a
     *  later style patch away from Compact can put them back. */
    private static final java.util.WeakHashMap<View, android.content.res.ColorStateList> buttonTextColors = new java.util.WeakHashMap<>();
    private static final java.util.WeakHashMap<View, android.content.res.ColorStateList> buttonIconColors = new java.util.WeakHashMap<>();
    static final java.util.WeakHashMap<View, int[]> buttonMetrics = new java.util.WeakHashMap<>();

    public static void setButtonContent(View view, String title, String iconName, boolean iconOnly) {
        if (!(view instanceof com.google.android.material.button.MaterialButton)) return;
        com.google.android.material.button.MaterialButton button = (com.google.android.material.button.MaterialButton) view;
        android.graphics.drawable.Drawable icon = drawableByName(view.getContext(), iconName);
        button.setIcon(icon);
        button.setIconSize((int) (20 * view.getResources().getDisplayMetrics().density));
        button.setIconGravity(com.google.android.material.button.MaterialButton.ICON_GRAVITY_TEXT_START);
        button.setIconPadding(iconOnly ? 0 : (int) (6 * view.getResources().getDisplayMetrics().density));
        button.setText(iconOnly && icon != null ? "" : title);
        // The title is the spoken name, unless the app named the button itself (setA11y); writing
        // it here then replaced the app's label on every title or icon change.
        if (!appLabeled.containsKey(button)) button.setContentDescription(title);
        // `View.setTooltipText` is API 26; the compat call reaches the scaffold's minSdk of 24 (docs/size-classes.md).
        setViewTooltip(button, iconOnly && icon != null ? title : null);
        if (iconOnly && icon != null) buttonIconOnly.put(button, Boolean.TRUE);
        else buttonIconOnly.remove(button);
        applyPlainIconTint(button);
    }

    /** Buttons showing only their icon, and the plain ones (style kind 5): together an M3
     *  standard icon button, whatever order the content and style patches arrive in. */
    private static final java.util.WeakHashMap<View, Boolean> buttonIconOnly = new java.util.WeakHashMap<>();
    private static final java.util.WeakHashMap<View, Boolean> buttonPlain = new java.util.WeakHashMap<>();

    /** A plain icon-only button is M3's standard icon button: its glyph in onSurfaceVariant,
     *  and onSurface at 38% when disabled, not the primary color of a text button. */
    private static void applyPlainIconTint(MaterialButton b) {
        if (!buttonPlain.containsKey(b) || !buttonIconOnly.containsKey(b)) return;
        int normal = MaterialColors.getColor(b, com.google.android.material.R.attr.colorOnSurfaceVariant);
        int onSurface = MaterialColors.getColor(b, com.google.android.material.R.attr.colorOnSurface);
        int disabled = (onSurface & 0x00FFFFFF) | (0x61 << 24); // 38%
        b.setIconTint(new android.content.res.ColorStateList(
                new int[][] { new int[] { -android.R.attr.state_enabled }, new int[] {} },
                new int[] { disabled, normal }));
    }

    public static View makeButton(final long id, String title) {
        final MaterialButton b = new MaterialButton(ctx); // M3 filled button (Expressive shape/motion)
        buttonTints.put(b, b.getBackgroundTintList());
        buttonTextColors.put(b, b.getTextColors());
        buttonIconColors.put(b, b.getIconTint());
        buttonMetrics.put(b, new int[] {
                b.getMinWidth(), b.getMinimumWidth(), b.getPaddingStart(), b.getPaddingEnd() });
        b.setText(title);
        // One line, like every other toolkit's button: in a tight row Material would wrap the
        // title and the pill's fixed height would cut the second line off.
        b.setMaxLines(1);
        b.setEllipsize(android.text.TextUtils.TruncateAt.END);
        b.setOnClickListener(new View.OnClickListener() {
            public void onClick(View x) {
                nativeOnEvent(id, K_PRESSED, 0, null);
                // A checkable button toggled itself on the click; Day owns the selected state
                // (setButtonSelected), so put back what it last asked for.
                Boolean want = buttonSelected.get(b);
                if (want != null && b.isCheckable()) b.setChecked(want);
            }
        });
        return b;
    }

    /** Each button's selected state (`ButtonProps::selected`): what the click listener restores
     *  and `readNative` reports as checked. */
    static final java.util.WeakHashMap<View, Boolean> buttonSelected = new java.util.WeakHashMap<>();

    /**
     * `ButtonProps::selected`: a checked MaterialButton, Material's own selected look, plus the
     * view's selected state, which TalkBack reads as "selected". The button turns checkable only
     * once it is first selected, so an ordinary button keeps its momentary behavior.
     */
    public static void setButtonSelected(View v, boolean on) {
        buttonSelected.put(v, on);
        v.setSelected(on);
        if (!(v instanceof MaterialButton)) return;
        MaterialButton b = (MaterialButton) v;
        if (on || b.isCheckable()) {
            b.setCheckable(true);
            b.setChecked(on);
        }
    }

    /**
     * Style a button in place: kind 0 automatic, 1 bordered, 2 prominent, 3 tinted (argb/fg),
     * 4 compact: no minimum width and glyph-sized insets, for a stepper's "−"/"+" (the M3
     * button's 88 dp minimum made two of them wider than an inspector pane).
     *
     * A tint is `backgroundTint` on the MaterialButton, so Material keeps drawing the ripple, the
     * state overlays and the disabled alpha itself; the view stays a MaterialButton, with its
     * role, focus and keyboard activation intact. Anything but a tint leaves the stock M3 look,
     * which is already the filled button day's `prominent` asks for.
     */
    public static void setButtonStyle(View v, int kind, int argb, int fgArgb) {
        if (!(v instanceof MaterialButton)) return;
        MaterialButton b = (MaterialButton) v;
        int[] stock = buttonMetrics.get(b);
        if (kind == 4 || kind == 5) {
            float d = ctx.getResources().getDisplayMetrics().density;
            b.setMinWidth(0);
            b.setMinimumWidth(0);
            b.setPadding((int) (12 * d), b.getPaddingTop(), (int) (12 * d), b.getPaddingBottom());
        } else if (stock != null) {
            b.setMinWidth(stock[0]);
            b.setMinimumWidth(stock[1]);
            b.setPadding(stock[2], b.getPaddingTop(), stock[3], b.getPaddingBottom());
        }
        if (kind == 5) {
            // Use the theme's text-button state colors, including disabled and pressed states.
            MaterialButton text = new MaterialButton(ctx, null,
                androidx.appcompat.R.attr.borderlessButtonStyle);
            b.setBackgroundTintList(android.content.res.ColorStateList.valueOf(android.graphics.Color.TRANSPARENT));
            b.setTextColor(text.getTextColors());
            b.setIconTint(text.getIconTint());
            buttonPlain.put(b, Boolean.TRUE);
            applyPlainIconTint(b);
            return;
        }
        buttonPlain.remove(b);
        if (kind != 3) {
            b.setTextColor(buttonTextColors.get(b));
            b.setIconTint(buttonIconColors.get(b));
            // Back to what the theme dressed it in (see buttonTints), not to no tint at all.
            b.setBackgroundTintList(buttonTints.get(b));
            return;
        }
        b.setBackgroundTintList(android.content.res.ColorStateList.valueOf(argb));
        b.setTextColor(fgArgb);
        b.setIconTint(android.content.res.ColorStateList.valueOf(fgArgb));
    }

    /** Views made focusable by `.focusable()` (docs/focus.md): a primary press focuses them. */
    private static final java.util.Set<View> pressFocus =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<View, Boolean>());

    /** `.focusable()` on any view (docs/focus.md): it joins the focus order (in touch mode
     *  too, so a request and a hardware keyboard reach it), reports focus both ways, takes
     *  focus at a primary press before the press reaches its recognizers, and hands the keys
     *  of the short route (or the full one) to node `id` with their phases while focused. */
    public static void enableKeys(View v, final long id) {
        v.setFocusable(true); v.setFocusableInTouchMode(true);
        v.setOnFocusChangeListener((view, hasFocus) -> {
            // Keys held when focus goes are not this node's to release (docs/menus.md).
            if (!hasFocus) DayCanvasView.forgetKeys(id);
            nativeOnEvent(id, K_FOCUS_CHANGED, hasFocus ? 1 : 0, null);
        });
        v.setOnKeyListener((view, code, e) -> {
            if (e.getAction() == KeyEvent.ACTION_UP) return DayCanvasView.sendKey(id, code, e);
            if (!view.isFocused() || e.getAction() != KeyEvent.ACTION_DOWN) return false;
            return DayCanvasView.sendKey(id, code, e);
        });
        pressFocus.add(v);
        updateTouchListener(v);
    }

    static boolean horizontalPan(View v, long id, MotionEvent e) {
        if (e.getActionMasked() != MotionEvent.ACTION_SCROLL) return false;
        float x = e.getAxisValue(MotionEvent.AXIS_HSCROLL);
        float y = e.getAxisValue(MotionEvent.AXIS_VSCROLL);
        if (Math.abs(x) <= Math.abs(y)) return false;
        float step = 40 * v.getResources().getDisplayMetrics().density;
        gesture(id, G_PAN, e.getX() + "," + e.getY() + "," + (x * step) + "," + (y * step));
        return true;
    }
    public static void enablePan(View v, final long id) {
        if (v instanceof DayFixed) ((DayFixed)v).panNode = id;
        else v.setOnGenericMotionListener((view, event) -> horizontalPan(view, id, event));
    }

    /**
     * Hover: a mouse or a stylus moving over the view (docs/canvas.md "Interaction"): Began,
     * Changed and Ended of day_spec::Event::Hover. A finger generates no HOVER_* actions at
     * all, so a touch-only device wires this and never reports; that is the contract.
     */
    public static void enableHover(View v, final long id) {
        v.setOnHoverListener(new View.OnHoverListener() {
            float lx, ly;
            public boolean onHover(View view, MotionEvent ev) {
                float x = ev.getX(), y = ev.getY();
                switch (ev.getActionMasked()) {
                    case MotionEvent.ACTION_HOVER_ENTER:
                        lx = x; ly = y;
                        gesture(id, G_HOVER_BEGAN, x + "," + y + ",0,0");
                        return true;
                    case MotionEvent.ACTION_HOVER_MOVE:
                        lx = x; ly = y;
                        gesture(id, G_HOVER_CHANGED, x + "," + y + ",0,0");
                        return true;
                    case MotionEvent.ACTION_HOVER_EXIT:
                        // The exit's own coordinates can already be outside; the contract is the
                        // last point seen inside.
                        gesture(id, G_HOVER_ENDED, lx + "," + ly + ",0,0");
                        return true;
                }
                return false;
            }
        });
    }

    private static final java.util.WeakHashMap<View, ScaleGestureDetector> pinchDetectors =
            new java.util.WeakHashMap<>();

    /**
     * Pinch (docs/canvas.md "Zoom and pan"): a ScaleGestureDetector fed from the shared touch
     * listener. G_PINCH_BEGAN/CHANGED/ENDED are day_spec::Event::Pinch's Began/Changed/Ended; the payload is the
     * focus point in px and the scale, which is CUMULATIVE since Began (the current span over
     * the span the gesture started with, not the detector's per-event factor). Quick scale
     * (double-tap-and-drag) stays off so a double tap remains two taps.
     */
    public static void enablePinch(View v, final long id) {
        // Idempotent, like the WinUI side's gesture set: a detector swapped in mid-gesture
        // would never have seen the press. Note that the detector only engages once the span
        // passes ViewConfiguration's minimum scaling span (27 mm), Android's own threshold.
        if (pinchDetectors.containsKey(v)) return;
        ScaleGestureDetector d = new ScaleGestureDetector(v.getContext(),
                new ScaleGestureDetector.SimpleOnScaleGestureListener() {
            float span0, scale;
            @Override public boolean onScaleBegin(ScaleGestureDetector sd) {
                span0 = sd.getCurrentSpan(); scale = 1f;
                gesture(id, G_PINCH_BEGAN, sd.getFocusX() + "," + sd.getFocusY() + ",1,0");
                return true;
            }
            @Override public boolean onScale(ScaleGestureDetector sd) {
                if (span0 > 0) scale = sd.getCurrentSpan() / span0;
                gesture(id, G_PINCH_CHANGED,
                        sd.getFocusX() + "," + sd.getFocusY() + "," + scale + ",0");
                return true;
            }
            @Override public void onScaleEnd(ScaleGestureDetector sd) {
                // The end's own span is measured over the finger still down (a single one
                // spans nothing), so the gesture ends at the scale it last reported.
                gesture(id, G_PINCH_ENDED,
                        sd.getFocusX() + "," + sd.getFocusY() + "," + scale + ",0");
            }
        });
        d.setQuickScaleEnabled(false);
        d.setStylusScaleEnabled(false);
        pinchDetectors.put(v, d);
        // Clickable, so View keeps the touch stream flowing after a press that nothing of ours
        // consumed (the same reason the drag listener below sets it).
        v.setClickable(true);
        updateTouchListener(v);
    }

    private static final java.util.WeakHashMap<View, RotateTracker> rotateTrackers =
            new java.util.WeakHashMap<>();

    /** A pinch or a rotation is wired to this view: a second finger belongs to them. */
    private static boolean twoFinger(View v) {
        return pinchDetectors.containsKey(v) || rotateTrackers.containsKey(v);
    }

    /**
     * Rotation (docs/canvas.md "Zoom and pan"). Android has no rotation detector, so this
     * follows the first two fingers itself, fed from the shared touch listener beside the
     * pinch's ScaleGestureDetector. G_ROTATE_BEGAN/CHANGED/ENDED are day_spec::Event::Rotate's
     * Began/Changed/Ended; the payload is the midpoint in px and the angle, in radians, that
     * the line between the two fingers has turned since the second went down: cumulative, and
     * clockwise positive because the screen's y axis points down. A move that leaves the angle
     * where it was (a pure pinch, a two-finger pan) reports nothing.
     */
    public static void enableRotate(View v, final long id) {
        if (rotateTrackers.containsKey(v)) return; // idempotent, as enablePinch is
        rotateTrackers.put(v, new RotateTracker(id));
        v.setClickable(true);
        updateTouchListener(v);
    }

    private static final class RotateTracker {
        final long id;
        boolean active;
        int p0, p1; // the two pointer ids being followed
        double raw, total; // last atan2 of the line between them, and the turn since Began
        float fx, fy;

        RotateTracker(long id) { this.id = id; }

        /** The current angle of the line between the two fingers, or NaN if either lifted. */
        private double angle(MotionEvent ev) {
            int i0 = ev.findPointerIndex(p0), i1 = ev.findPointerIndex(p1);
            if (i0 < 0 || i1 < 0) return Double.NaN;
            float x0 = ev.getX(i0), y0 = ev.getY(i0), x1 = ev.getX(i1), y1 = ev.getY(i1);
            fx = (x0 + x1) / 2f; fy = (y0 + y1) / 2f;
            return Math.atan2(y1 - y0, x1 - x0);
        }

        private void send(int phase) {
            gesture(id, phase, fx + "," + fy + "," + total + ",0");
        }

        void onTouchEvent(MotionEvent ev) {
            switch (ev.getActionMasked()) {
                case MotionEvent.ACTION_POINTER_DOWN:
                    if (active || ev.getPointerCount() < 2) return;
                    p0 = ev.getPointerId(0); p1 = ev.getPointerId(1);
                    raw = angle(ev);
                    if (Double.isNaN(raw)) return;
                    total = 0; active = true;
                    send(G_ROTATE_BEGAN);
                    return;
                case MotionEvent.ACTION_MOVE: {
                    if (!active) return;
                    double a = angle(ev);
                    if (Double.isNaN(a)) return;
                    // Unwrapped, so a turn past half a circle keeps counting rather than
                    // jumping a full turn where atan2 wraps.
                    double turn = a - raw;
                    turn -= Math.rint(turn / (2 * Math.PI)) * 2 * Math.PI;
                    raw = a;
                    if (turn == 0) return;
                    total += turn;
                    send(G_ROTATE_CHANGED);
                    return;
                }
                case MotionEvent.ACTION_POINTER_UP: {
                    if (!active) return;
                    int lifted = ev.getPointerId(ev.getActionIndex());
                    if (lifted != p0 && lifted != p1) return;
                    active = false;
                    send(G_ROTATE_ENDED); // at the angle it last reported, as the pinch ends at its scale
                    return;
                }
                case MotionEvent.ACTION_UP:
                case MotionEvent.ACTION_CANCEL:
                    if (!active) return;
                    active = false;
                    send(G_ROTATE_ENDED);
                    return;
            }
        }
    }

    // ---- Recognizers (docs/gestures.md) ----------------------------------------------------
    // One view carries any mix of tap, drag, long press, double tap, press and touches, all fed
    // from the one OnTouchListener `updateTouchListener` installs (a bare setOnTouchListener
    // would replace, where UIKit's recognizers coexist). Each kind reports for the node that
    // asked for it. Coordinates are px; Rust converts to dp.

    /** What Day attached to one view's touch stream. */
    static final class Recognizers {
        boolean tap, drag, longPress, doubleTap, press, touches;
        long tapId, dragId, longPressId, doubleTapId, pressId, touchesId;
        /** The drag's axis: 0 any direction, 1 horizontal only, 2 vertical only. */
        int axis;
        /** The platform's long-press and double-tap recognizer, while either is enabled. */
        android.view.GestureDetector detector;
        Detector detectorListener;
        VelocityTracker velocity;
        /** Where the stream went down, and the last point seen (the click path's tap point). */
        float sx, sy, tx, ty;
        /** The drag claimed this stream: Began was sent and the stream is held. */
        boolean dragging;
        /** An axis drag that set off across its axis: the stream is not the drag's. */
        boolean declined;
        /** A second finger ended the drag; the rest of the stream is the pinch's. */
        boolean pinched;
        /** The long press fired in this stream: its release is not a tap. */
        boolean held;
        /** A touch stream just lifted: the click View raises for it is not a fresh tap. */
        boolean touchUp;
        /** The press under way: its stream's down time, the last point, and where the view
         *  sat on screen at the down (a scroll that moves it ends the press). */
        boolean pressed;
        long pressDown;
        float px, py;
        final int[] pressAt = new int[2];
        boolean watchesDetach;
        /** The contacts down on the view and where each was last reported, by pointer id. */
        final android.util.SparseArray<float[]> contacts = new android.util.SparseArray<>();
    }

    static final java.util.WeakHashMap<View, Recognizers> recognizers = new java.util.WeakHashMap<>();

    private static Recognizers recognizersOf(View v) {
        Recognizers r = recognizers.get(v);
        if (r == null) { r = new Recognizers(); recognizers.put(v, r); }
        return r;
    }

    /** Attach a tap (`isDrag` false) or drag recognizer for node `id` (docs/gestures.md). */
    public static void enableGesture(View v, final long id, final boolean isDrag) {
        Recognizers r = recognizersOf(v);
        if (isDrag) { r.drag = true; r.dragId = id; } else { r.tap = true; r.tapId = id; }
        installRecognizers(v, r);
    }

    /** Attach a long press, double tap, press or touches recognizer for node `id`: `kind` is
     *  G_LONG_PRESS, G_DOUBLE_TAP, G_PRESS_IN or G_TOUCHES. Idempotent. */
    public static void enableRecognizer(View v, final long id, int kind) {
        Recognizers r = recognizersOf(v);
        switch (kind) {
            case G_LONG_PRESS:
                clearCompatTooltip(v);
                r.longPress = true; r.longPressId = id; break;
            case G_DOUBLE_TAP: r.doubleTap = true; r.doubleTapId = id; break;
            case G_PRESS_IN: r.press = true; r.pressId = id; break;
            case G_TOUCHES: r.touches = true; r.touchesId = id; break;
            default: return;
        }
        installRecognizers(v, r);
    }

    /** The drag's axis (docs/gestures.md "Gestures inside a scroll view"): 0 any direction,
     *  1 horizontal, 2 vertical. */
    public static void setDragAxis(View v, int axis) {
        recognizersOf(v).axis = axis;
    }

    private static void installRecognizers(final View v, final Recognizers r) {
        if (r.tap && !r.drag) {
            // A tap alone rides View's own click: it keeps native long-press menus and a
            // RecyclerView's scroll interception arbitrating as they always do, and the
            // accessibility click and the keyboard's Enter reach it too.
            contextMenuForwardedClicks.remove(v);
            gestureTapClicks.add(v);
            v.setOnClickListener(DayBridge::clickTap);
        } else if (r.drag && gestureTapClicks.remove(v)) {
            v.setOnClickListener(null);
        }
        // Clickable, so View takes the stream at the press and the recognizers see all of
        // it, while View's own handling (the click, a context menu's long click, the press
        // state) still runs. Press alone stays an observer and claims nothing.
        if (r.drag || r.longPress || r.doubleTap || r.touches) v.setClickable(true);
        if (r.longPress || r.doubleTap) {
            if (r.detector == null) {
                r.detectorListener = new Detector(v, r);
                r.detector = new android.view.GestureDetector(v.getContext(), r.detectorListener,
                        main != null ? main : new Handler(Looper.getMainLooper()));
            }
            r.detector.setIsLongpressEnabled(r.longPress);
            r.detector.setOnDoubleTapListener(r.doubleTap ? r.detectorListener : null);
        }
        if (r.press && !r.watchesDetach) {
            r.watchesDetach = true;
            v.addOnAttachStateChangeListener(new View.OnAttachStateChangeListener() {
                @Override public void onViewAttachedToWindow(View view) {}
                @Override public void onViewDetachedFromWindow(View view) { pressOut(view, r); }
            });
        }
        updateTouchListener(v);
    }

    /** The platform's long press (ViewConfiguration's timeout and slop) and double tap
     *  (its interval and distance), over the stream the shared listener feeds it. */
    private static final class Detector extends android.view.GestureDetector.SimpleOnGestureListener {
        // Weak: the recognizers are the value of a WeakHashMap keyed by this view.
        final java.lang.ref.WeakReference<View> view;
        final Recognizers r;
        Detector(View view, Recognizers r) { this.view = new java.lang.ref.WeakReference<>(view); this.r = r; }
        @Override public boolean onDown(MotionEvent e) { return true; }
        @Override public void onLongPress(MotionEvent e) {
            // A view whose long press opens its context menu keeps that (View's own long
            // click shows it), and no LongPress is sent.
            View v = view.get();
            if (!r.longPress || v == null || contextMenus.containsKey(v)) return;
            r.held = true;
            gesture(r.longPressId, G_LONG_PRESS, e.getX() + "," + e.getY());
        }
        @Override public boolean onSingleTapConfirmed(MotionEvent e) {
            // With a double tap enabled too, the single tap waits out the interval.
            if (r.tap && r.doubleTap && !r.held) {
                gesture(r.tapId, G_TAP, e.getX() + "," + e.getY() + ",0,0");
            }
            return true;
        }
        @Override public boolean onDoubleTap(MotionEvent e) {
            if (r.doubleTap) gesture(r.doubleTapId, G_DOUBLE_TAP, e.getX() + "," + e.getY());
            return true;
        }
    }

    /** View's click on a tap target. A click a touch stream raised is a tap unless the long
     *  press took that stream or the double-tap recognizer times the single tap; a click from
     *  accessibility or the keyboard is always one. */
    private static void clickTap(View view) {
        Recognizers r = recognizers.get(view);
        if (r == null || !r.tap) return;
        if (r.touchUp) {
            r.touchUp = false;
            if (r.held || r.doubleTap) return;
        }
        gesture(r.tapId, G_TAP, r.tx + "," + r.ty + ",0,0");
    }

    // ---- A Day gesture keeps its touch stream ---------------------------------------------
    // Every scrolling ancestor watches a child's touch stream and takes it over once the
    // finger has moved past the touch slop: the child gets ACTION_CANCEL and the page scrolls.
    // A piece with a drag handler (or a pinch or rotation under way) is the one the finger
    // is on, so it asks its ancestors not to, as UIKit's recognizers settle it on iOS.

    /** The view whose Day gesture holds the touch stream in progress, and that stream's down
     *  time: a holder left behind by a stream that never ended is not taken for the next one. */
    private static java.lang.ref.WeakReference<View> touchHolder;
    private static long touchHolderDown;

    /** Whether `e` belongs to a touch stream a Day drag, pinch or rotation holds. A native
     *  container that ignores `requestDisallowInterceptTouchEvent` (SwipeRefreshLayout does,
     *  over a child that is not a nested-scrolling one) asks this before intercepting. */
    public static boolean gestureHoldsTouch(MotionEvent e) {
        return touchHolder != null && touchHolder.get() != null
                && e.getDownTime() == touchHolderDown;
    }

    /** Take (`hold`) or give back the touch stream `e` belongs to, for `v`. The request goes
     *  to every ancestor in turn, not only the parent: a ViewGroup passes it up itself, but
     *  one that overrides the call to ignore it would otherwise stop it there, and the
     *  containers above (a pager, the nav host) would never hear it. */
    private static void holdTouch(View v, MotionEvent e, boolean hold) {
        if (hold) {
            touchHolder = new java.lang.ref.WeakReference<>(v);
            touchHolderDown = e.getDownTime();
        } else if (touchHolder != null && touchHolder.get() == v) {
            touchHolder = null;
        } else {
            return; // never held: nothing to give back
        }
        for (android.view.ViewParent p = v.getParent(); p != null; p = p.getParent()) {
            p.requestDisallowInterceptTouchEvent(hold);
        }
    }

    /** One listener per view for everything Day watches on its touch stream: a context menu's
     *  press point, the recognizers, the pinch and rotation trackers and focus on press. It
     *  consumes only what a drag or a two-finger gesture owns. */
    private static void updateTouchListener(View v) {
        if (!contextMenus.containsKey(v) && !recognizers.containsKey(v) && !twoFinger(v)
                && !pressFocus.contains(v)) {
            v.setOnTouchListener(null);
            return;
        }
        final boolean[] multi = new boolean[1]; // a second finger landed in this touch stream
        v.setOnTouchListener((view, event) -> {
            int action = event.getActionMasked();
            boolean finished = action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL;
            // A focusable view takes focus before its recognizers see the press (docs/focus.md).
            if (action == MotionEvent.ACTION_DOWN && pressFocus.contains(view) && !view.isFocused()
                    && primaryButton(event)) {
                view.requestFocus();
            }
            ContextMenuBinding menu = contextMenus.get(view);
            if (menu != null) {
                menu.x = finished ? 0 : event.getX();
                menu.y = finished ? 0 : event.getY();
            }
            if (action == MotionEvent.ACTION_DOWN) multi[0] = false;
            else if (action == MotionEvent.ACTION_POINTER_DOWN) multi[0] = true;
            Recognizers r = recognizers.get(view);
            boolean consumed = r != null && onRecognizers(view, r, event);
            // The detector sees each event after the drag listener, so a second finger ends
            // the drag before the pinch it starts reports its Began.
            ScaleGestureDetector pinch = pinchDetectors.get(view);
            if (pinch != null) pinch.onTouchEvent(event);
            RotateTracker rotate = rotateTrackers.get(view);
            if (rotate != null) rotate.onTouchEvent(event);
            // A second finger on a pinch or rotate target: the two-finger gesture owns the
            // stream from here, and no scrolling ancestor takes it for a scroll.
            if (action == MotionEvent.ACTION_POINTER_DOWN && (pinch != null || rotate != null)) {
                holdTouch(view, event, true);
            }
            if (finished) holdTouch(view, event, false);
            // A stream that arrived through a parent's hit slop: the recognizers read the true
            // point, View's own press handling the one its bounds understand.
            if (slopStream == view && !consumed) clampIntoSlop(view, event);
            if ((pinch == null && rotate == null) || !multi[0]) return consumed;
            // Two fingers: ours, not View's. Its own onTouchEvent never saw the second finger,
            // so the release would still count as the click the press armed; cancel that
            // instead, and a pinch never ends in a tap.
            if (finished) {
                MotionEvent cancel = MotionEvent.obtain(event);
                cancel.setAction(MotionEvent.ACTION_CANCEL);
                view.onTouchEvent(cancel);
                cancel.recycle();
            }
            return true;
        });
    }

    /** Whether the press `e` began is a primary one: any finger or stylus, and a mouse only
     *  with its primary button (docs/gestures.md "Press"). */
    private static boolean primaryButton(MotionEvent e) {
        return e.getToolType(0) != MotionEvent.TOOL_TYPE_MOUSE
                || (e.getButtonState() & MotionEvent.BUTTON_PRIMARY) != 0
                || e.getButtonState() == 0;
    }

    /** Run one event through a view's recognizers; true when the drag consumed it. */
    private static boolean onRecognizers(View view, Recognizers r, MotionEvent ev) {
        int action = ev.getActionMasked();
        if (action == MotionEvent.ACTION_DOWN) {
            r.sx = ev.getX(); r.sy = ev.getY();
            r.held = false; r.touchUp = false; r.pinched = false;
            r.dragging = false; r.declined = false;
        }
        r.tx = ev.getX(); r.ty = ev.getY();
        if (r.press) pressTouch(view, r, ev);
        if (r.touches) reportTouches(r, ev);
        if (r.detector != null) r.detector.onTouchEvent(ev);
        if (action == MotionEvent.ACTION_UP) {
            // View posts the click this release raises; the flag lives until it has run.
            r.touchUp = true;
            view.post(() -> view.post(() -> r.touchUp = false));
        }
        if (!r.drag) return false; // View's own onTouchEvent takes the click
        return drag(view, r, ev);
    }

    /** The drag recognizer: Began at the press (or, with an axis, once the movement shows its
     *  direction), the stream held from Began to Ended, the release velocity before Ended. */
    private static boolean drag(View view, Recognizers r, MotionEvent ev) {
        float x = ev.getX(), y = ev.getY();
        String at = x + "," + y + "," + (x - r.sx) + "," + (y - r.sy);
        switch (ev.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                trackVelocity(r, ev, true);
                if (r.axis == 0) {
                    // The drag begins with the press, so the stream is held from the press:
                    // an enclosing scroll view, pager or pull-to-refresh would otherwise
                    // cancel it a few pixels in and scroll instead. Released by the shared
                    // listener when the stream ends.
                    r.dragging = true;
                    holdTouch(view, ev, true);
                    gesture(r.dragId, G_DRAG_BEGAN, x + "," + y + ",0,0");
                }
                return true;
            case MotionEvent.ACTION_POINTER_DOWN:
                // On a pinch or rotate target the second finger hands the stream to those
                // trackers: the one-finger drag ends here, and the two fingers never come out
                // as drags (docs/canvas.md "Zoom and pan").
                if (!twoFinger(view)) return false;
                if (!r.pinched) {
                    r.pinched = true;
                    if (r.dragging) gesture(r.dragId, G_DRAG_ENDED, at);
                    r.dragging = false;
                }
                return true;
            case MotionEvent.ACTION_MOVE:
                trackVelocity(r, ev, false);
                if (r.pinched || r.declined) return true;
                if (!r.dragging) {
                    // An axis drag decides at half the touch slop, before any scrolling
                    // ancestor (which waits for the whole slop) can take the stream: along its
                    // axis it is the drag's from here; across it, it is the scroll view's,
                    // and the drag hears nothing of it.
                    float dx = x - r.sx, dy = y - r.sy;
                    float decide = android.view.ViewConfiguration.get(view.getContext())
                            .getScaledTouchSlop() / 2f;
                    if (Math.hypot(dx, dy) < decide) return true;
                    boolean along = r.axis == 1 ? Math.abs(dx) >= Math.abs(dy)
                                                : Math.abs(dy) >= Math.abs(dx);
                    if (!along) { r.declined = true; return true; }
                    r.dragging = true;
                    holdTouch(view, ev, true);
                    gesture(r.dragId, G_DRAG_BEGAN, at);
                    return true;
                }
                gesture(r.dragId, G_DRAG_CHANGED, at);
                return true;
            case MotionEvent.ACTION_UP:
                trackVelocity(r, ev, false);
                if (r.pinched) { r.velocity = recycle(r.velocity); return true; }
                if (r.dragging) {
                    // The platform's own measure of the release, sent just before Ended.
                    // The finger that lifts last, which is the one the drag followed at the end.
                    if (r.velocity != null) {
                        int finger = ev.getPointerId(ev.getActionIndex());
                        r.velocity.computeCurrentVelocity(1000);
                        gesture(r.dragId, G_DRAG_VELOCITY, r.velocity.getXVelocity(finger)
                                + "," + r.velocity.getYVelocity(finger));
                    }
                    gesture(r.dragId, G_DRAG_ENDED, at);
                    r.dragging = false;
                }
                r.velocity = recycle(r.velocity);
                if (r.tap && !r.doubleTap && !r.held
                        && Math.abs(x - r.sx) < 40 && Math.abs(y - r.sy) < 40) {
                    gesture(r.tapId, G_TAP, x + "," + y + ",0,0");
                    view.performClick();
                }
                return true;
            case MotionEvent.ACTION_CANCEL:
                if (r.dragging && !r.pinched) gesture(r.dragId, G_DRAG_ENDED, at);
                r.dragging = false;
                r.velocity = recycle(r.velocity);
                return true;
        }
        return false;
    }

    /** Feed the drag's VelocityTracker in screen coordinates, so a piece the drag moves does
     *  not measure its own motion. */
    private static void trackVelocity(Recognizers r, MotionEvent ev, boolean start) {
        if (start) {
            r.velocity = recycle(r.velocity);
            r.velocity = VelocityTracker.obtain();
        }
        if (r.velocity == null) return;
        MotionEvent screen = MotionEvent.obtain(ev);
        screen.offsetLocation(ev.getRawX() - ev.getX(), ev.getRawY() - ev.getY());
        r.velocity.addMovement(screen);
        screen.recycle();
    }

    private static VelocityTracker recycle(VelocityTracker t) {
        if (t != null) t.recycle();
        return null;
    }

    // ---- Press (docs/gestures.md "Press") --------------------------------------------------
    // An observer: the view's listener sees the press go down and never claims the stream, so
    // a tap on the row around it still lands. Whoever does take the stream, the activity sees
    // all of it (observeTouch), which is where the press ends: at the release, when the finger
    // leaves the view's hit area, or when a scroll view takes the stream and moves the view.

    /** The views pressed right now. */
    private static final java.util.Set<View> pressing =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<View, Boolean>());

    private static void pressTouch(View view, Recognizers r, MotionEvent ev) {
        switch (ev.getActionMasked()) {
            case MotionEvent.ACTION_DOWN:
                if (!primaryButton(ev)) return;
                pressOut(view, r);
                r.pressed = true;
                r.pressDown = ev.getDownTime();
                r.px = ev.getX(); r.py = ev.getY();
                view.getLocationOnScreen(r.pressAt);
                pressing.add(view);
                gesture(r.pressId, G_PRESS_IN, r.px + "," + r.py);
                return;
            case MotionEvent.ACTION_MOVE:
                if (!r.pressed) return;
                r.px = ev.getX(); r.py = ev.getY();
                if (!insideSlop(view, r.px, r.py)) pressOut(view, r);
                return;
            case MotionEvent.ACTION_UP:
                if (r.pressed) { r.px = ev.getX(); r.py = ev.getY(); }
                pressOut(view, r);
                return;
            case MotionEvent.ACTION_CANCEL:
                pressOut(view, r);
                return;
        }
    }

    private static void pressOut(View view, Recognizers r) {
        if (!r.pressed) return;
        r.pressed = false;
        pressing.remove(view);
        gesture(r.pressId, G_PRESS_OUT, r.px + "," + r.py);
    }

    /** End every press under way: the window lost focus or went to the background. */
    public static void cancelPresses() {
        for (View v : new java.util.ArrayList<>(pressing)) {
            Recognizers r = recognizers.get(v);
            if (r != null) pressOut(v, r); else pressing.remove(v);
        }
    }

    /** Whether `(x, y)` in `v`'s own px lies in its hit area: its bounds and its hit slop. */
    private static boolean insideSlop(View v, float x, float y) {
        int[] s = slopEdges(v);
        return x >= -s[1] && x < v.getWidth() + s[2] && y >= -s[0] && y < v.getHeight() + s[3];
    }

    /** Every touch event a Day window receives, after its views had it (DayActivity and
     *  DayWindowActivity call this from dispatchTouchEvent): the input kind, and the presses
     *  whose view no longer owns the stream. */
    public static void observeTouch(MotionEvent ev) {
        if (!started) return;
        int action = ev.getActionMasked();
        if (action == MotionEvent.ACTION_DOWN || action == MotionEvent.ACTION_POINTER_DOWN) {
            notePointerKind(ev.getToolType(ev.getActionIndex()));
        }
        if (pressing.isEmpty()) return;
        // Screen coordinates of the first finger, which is the one that pressed.
        float ox = ev.getRawX() - ev.getX(), oy = ev.getRawY() - ev.getY();
        int[] at = new int[2];
        for (View v : new java.util.ArrayList<>(pressing)) {
            Recognizers r = recognizers.get(v);
            if (r == null || !r.pressed) { pressing.remove(v); continue; }
            // A new stream ends a press its view never saw end (the view registered this
            // stream's own press before the activity sees the down).
            boolean stale = action == MotionEvent.ACTION_DOWN && ev.getDownTime() != r.pressDown;
            if (stale || !v.isAttachedToWindow() || !v.isShown()) {
                pressOut(v, r);
                continue;
            }
            if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) {
                pressOut(v, r);
                continue;
            }
            if (action != MotionEvent.ACTION_MOVE) continue;
            v.getLocationOnScreen(at);
            r.px = ev.getX(0) + ox - at[0];
            r.py = ev.getY(0) + oy - at[1];
            if (!insideSlop(v, r.px, r.py)) { pressOut(v, r); continue; }
            // The view moved under the finger: a scroll view, a pager or a pull-to-refresh
            // took the stream. A Day drag that moves its own piece holds the stream instead.
            if ((at[0] != r.pressAt[0] || at[1] != r.pressAt[1]) && !gestureHoldsTouch(ev)) {
                pressOut(v, r);
            }
        }
    }

    /** Hover and wheel events a Day window receives (dispatchGenericMotionEvent): a mouse or a
     *  stylus moving over it sets the input kind. */
    public static void observeGenericMotion(MotionEvent ev) {
        if (!started) return;
        int action = ev.getActionMasked();
        if (action == MotionEvent.ACTION_HOVER_MOVE || action == MotionEvent.ACTION_HOVER_ENTER) {
            notePointerKind(ev.getToolType(0));
        }
    }

    // ---- Input kind (docs/gestures.md "Input kind") ----------------------------------------

    private static String pointerKind;

    /** The kind of pointer before any input: touch where the device has a touch screen. */
    public static String primaryPointer() {
        if (pointerKind == null) {
            pointerKind = ctx != null && !ctx.getPackageManager().hasSystemFeature(
                    android.content.pm.PackageManager.FEATURE_TOUCHSCREEN) ? "mouse" : "touch";
        }
        return pointerKind;
    }

    private static String kindName(int toolType) {
        switch (toolType) {
            case MotionEvent.TOOL_TYPE_MOUSE: return "mouse";
            case MotionEvent.TOOL_TYPE_STYLUS:
            case MotionEvent.TOOL_TYPE_ERASER: return "stylus";
            default: return "touch";
        }
    }

    private static void notePointerKind(int toolType) {
        String kind = kindName(toolType);
        if (kind.equals(primaryPointer())) return;
        pointerKind = kind;
        gesture(0, G_POINTER_KIND, kind);
    }

    // ---- Touches (docs/gestures.md "Touches") ----------------------------------------------

    /** Every contact on the view, one report per MotionEvent: "id,phase,x,y,pressure,kind"
     *  joined by ';' (phase 0 began, 1 moved, 2 stationary, 3 ended, 4 cancelled; kind 0
     *  touch, 1 mouse, 2 stylus). */
    private static void reportTouches(Recognizers r, MotionEvent ev) {
        int action = ev.getActionMasked(), actor = ev.getActionIndex();
        if (action == MotionEvent.ACTION_DOWN) r.contacts.clear();
        StringBuilder out = new StringBuilder();
        for (int i = 0; i < ev.getPointerCount(); i++) {
            int id = ev.getPointerId(i);
            float x = ev.getX(i), y = ev.getY(i);
            int phase;
            if (action == MotionEvent.ACTION_CANCEL) phase = 4;
            else if (action == MotionEvent.ACTION_DOWN
                    || (action == MotionEvent.ACTION_POINTER_DOWN && i == actor)) phase = 0;
            else if (action == MotionEvent.ACTION_UP
                    || (action == MotionEvent.ACTION_POINTER_UP && i == actor)) phase = 3;
            else {
                float[] last = r.contacts.get(id);
                phase = last != null && last[0] == x && last[1] == y ? 2 : 1;
            }
            if (phase >= 3) r.contacts.remove(id); else r.contacts.put(id, new float[]{x, y});
            float pressure = ev.getPressure(i);
            if (!(pressure > 0)) pressure = 1; // a device that reports no force
            int tool = ev.getToolType(i);
            int kind = tool == MotionEvent.TOOL_TYPE_MOUSE ? 1
                    : tool == MotionEvent.TOOL_TYPE_STYLUS || tool == MotionEvent.TOOL_TYPE_ERASER ? 2 : 0;
            if (out.length() > 0) out.append(';');
            out.append(id).append(',').append(phase).append(',').append(x).append(',').append(y)
                    .append(',').append(Math.min(pressure, 1f)).append(',').append(kind);
        }
        if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) r.contacts.clear();
        gesture(r.touchesId, G_TOUCHES, out.toString());
    }

    // ---- Hit slop (docs/gestures.md "Hit slop") --------------------------------------------

    /** Each view's hit slop in px: top, leading, bottom, trailing. */
    private static final java.util.WeakHashMap<View, int[]> slops = new java.util.WeakHashMap<>();
    /** The view a SlopDelegate is handing an event to, during that dispatch. */
    private static View slopStream;

    /** The view's slop as top, left, right, bottom px, leading and trailing resolved. */
    private static int[] slopEdges(View v) {
        int[] s = slops.get(v);
        if (s == null) return new int[4];
        boolean rtl = v.getLayoutDirection() == View.LAYOUT_DIRECTION_RTL;
        return new int[]{s[0], rtl ? s[3] : s[1], rtl ? s[1] : s[3], s[2]};
    }

    /** Grow the area in which `v` takes pointer input by the given px on each edge, without
     *  moving or resizing it. Zero on every edge restores its own bounds. */
    public static void setHitSlop(View v, int top, int leading, int bottom, int trailing) {
        if (top == 0 && leading == 0 && bottom == 0 && trailing == 0) {
            slops.remove(v);
            return;
        }
        boolean first = !slops.containsKey(v);
        slops.put(v, new int[]{top, leading, bottom, trailing});
        installSlopDelegate(v);
        if (first) {
            // The parent the delegate serves is the one the view is attached under.
            v.addOnAttachStateChangeListener(new View.OnAttachStateChangeListener() {
                @Override public void onViewAttachedToWindow(View view) { installSlopDelegate(view); }
                @Override public void onViewDetachedFromWindow(View view) {}
            });
        }
    }

    private static void installSlopDelegate(View v) {
        if (!(v.getParent() instanceof ViewGroup)) return;
        ViewGroup parent = (ViewGroup) v.getParent();
        android.view.TouchDelegate current = parent.getTouchDelegate();
        if (current instanceof SlopDelegate) return;
        if (current != null) return; // a delegate of the platform's own: leave it in charge
        parent.setTouchDelegate(new SlopDelegate(parent, v));
    }

    /** Put a slop stream's point where View's own press handling expects it: inside the
     *  view's bounds while it is inside the slop area, well outside once it leaves. */
    private static void clampIntoSlop(View v, MotionEvent e) {
        float x = e.getX(), y = e.getY();
        if (insideSlop(v, x, y)) {
            e.setLocation(Math.max(0, Math.min(x, v.getWidth() - 1)),
                    Math.max(0, Math.min(y, v.getHeight() - 1)));
        } else {
            float out = -2 * android.view.ViewConfiguration.get(v.getContext()).getScaledTouchSlop();
            e.setLocation(out, out);
        }
    }

    /**
     * The hit slop of a parent's children: its TouchDelegate, which View asks when no child
     * took a press inside its own bounds. The platform's TouchDelegate serves one child and
     * moves every point to that child's center; this one serves every child with a slop, the
     * front one first, and hands the stream over in the child's own coordinates.
     */
    static final class SlopDelegate extends android.view.TouchDelegate {
        final ViewGroup parent;
        View target;
        float dx, dy;

        SlopDelegate(ViewGroup parent, View first) {
            super(new android.graphics.Rect(), first);
            this.parent = parent;
        }

        @Override public boolean onTouchEvent(MotionEvent ev) {
            int action = ev.getActionMasked();
            if (action == MotionEvent.ACTION_DOWN) {
                target = null;
                for (int i = parent.getChildCount() - 1; i >= 0; i--) {
                    View c = parent.getChildAt(i);
                    if (!slops.containsKey(c) || c.getVisibility() != View.VISIBLE) continue;
                    float ox = parent.getScrollX() - c.getX(), oy = parent.getScrollY() - c.getY();
                    if (insideSlop(c, ev.getX() + ox, ev.getY() + oy)) {
                        target = c; dx = ox; dy = oy;
                        break;
                    }
                }
            }
            View t = target;
            if (t == null) return false;
            if (action == MotionEvent.ACTION_UP || action == MotionEvent.ACTION_CANCEL) target = null;
            MotionEvent copy = MotionEvent.obtain(ev);
            copy.offsetLocation(dx, dy);
            // A view with Day's listener reads the true point and clamps it for View itself;
            // a native control only ever sees a point its bounds understand.
            boolean listens = contextMenus.containsKey(t) || recognizers.containsKey(t)
                    || twoFinger(t) || pressFocus.contains(t);
            if (!listens) clampIntoSlop(t, copy);
            slopStream = listens ? t : null;
            try {
                return t.dispatchTouchEvent(copy);
            } finally {
                slopStream = null;
                copy.recycle();
            }
        }
    }

    // ---- Disabling a group (docs/forms.md "Disabling a group") ----------------------------

    /** The views a disabled group root was called for, with how many roots disabled each. */
    private static final java.util.WeakHashMap<View, Integer> disabledRoots = new java.util.WeakHashMap<>();
    /** The enabled state each view had of its own before a group disabled it. */
    private static final java.util.WeakHashMap<View, Boolean> ownEnabled = new java.util.WeakHashMap<>();
    /** The descendant focusability each group had before a disabled group blocked it. */
    private static final java.util.WeakHashMap<ViewGroup, Integer> ownFocusability =
            new java.util.WeakHashMap<>();

    /** Disable (or enable again) everything under `root`: native controls go disabled (dimmed,
     *  unavailable to accessibility), the subtree leaves the focus order and takes no touches.
     *  Calls nest: a view stays disabled while any root above it is. Composed content keeps
     *  its look; the app styles that. */
    public static void setSubtreeEnabled(View root, boolean enabled) {
        Integer n = disabledRoots.get(root);
        int count = (n == null ? 0 : n) + (enabled ? -1 : 1);
        if (count > 0) disabledRoots.put(root, count); else disabledRoots.remove(root);
        if (!enabled) {
            View focused = root.findFocus();
            if (focused != null) focused.clearFocus();
        }
        applyEnabled(root);
    }

    /** Whether a disabled group covers `v`: DayFixed refuses touches for its subtree then. */
    static boolean underDisabledGroup(View v) {
        if (disabledRoots.isEmpty()) return false;
        for (Object p = v; p instanceof View; p = ((View) p).getParent()) {
            if (disabledRoots.containsKey(p)) return true;
        }
        return false;
    }

    /** A view a person operates: what disabling a group disables. Plain containers, labels
     *  and images keep their own state (and look). */
    private static boolean operable(View v) {
        return v.isClickable() || v.isLongClickable() || v.isFocusable()
                || v instanceof EditText || v instanceof CompoundButton
                || v instanceof android.widget.AbsSeekBar || v instanceof Slider
                || v instanceof AdapterView;
    }

    private static void applyEnabled(View v) {
        boolean off = underDisabledGroup(v);
        if (off) {
            if (!ownEnabled.containsKey(v) && operable(v)) {
                ownEnabled.put(v, v.isEnabled());
                v.setEnabled(false);
            }
        } else {
            Boolean own = ownEnabled.remove(v);
            if (own != null) v.setEnabled(own);
        }
        if (!(v instanceof ViewGroup)) return;
        ViewGroup g = (ViewGroup) v;
        if (off && !ownFocusability.containsKey(g)) {
            ownFocusability.put(g, g.getDescendantFocusability());
            g.setDescendantFocusability(ViewGroup.FOCUS_BLOCK_DESCENDANTS);
        } else if (!off && ownFocusability.containsKey(g)) {
            g.setDescendantFocusability(ownFocusability.remove(g));
        }
        for (int i = 0; i < g.getChildCount(); i++) applyEnabled(g.getChildAt(i));
    }

    /** The editable inside a Material text box (TextInputLayout), or the view itself. */
    private static EditText editTextOf(View v) {
        if (v instanceof TextInputLayout) return ((TextInputLayout) v).getEditText();
        return (EditText) v;
    }

    /** What `setInputTraits` last applied to one text field, plus the input type the field
     *  was born with, which the plain-text purpose restores. `quiet` holds the field's
     *  listeners while the traits are applied: swapping the transformation method re-sets the
     *  text and toggling selectability drops focus, and neither is a change the user made.
     *  Keyed by the outer TextInputLayout, so a view `makeTextField` did not build has no
     *  entry and takes no traits. */
    private static final class InputState {
        int baseType;
        boolean applied, secure, readOnly, quiet;
        int purpose, submitLabel;
    }
    static final java.util.WeakHashMap<View, InputState> inputStates = new java.util.WeakHashMap<>();

    public static View makeTextField(final long id, String value, String placeholder) {
        // M3 text box: TextInputLayout (theme's default box style; placeholder = floating label)
        // wrapping a TextInputEditText. Rust talks to the outer view; setters reach the editable.
        TextInputLayout box = new TextInputLayout(ctx);
        box.setHint(placeholder);
        // Given more height than it asks for (a toolbar or a fixed-height row), the box keeps
        // its field in the middle, as UIKit and AppKit do, instead of stacking it at the top.
        box.setGravity(android.view.Gravity.CENTER_VERTICAL);
        TextInputEditText e = new TextInputEditText(box.getContext());
        e.setText(value);
        e.setSingleLine(true);
        final InputState st = new InputState();
        st.baseType = e.getInputType();
        inputStates.put(box, st);
        e.addTextChangedListener(new TextWatcher() {
            public void afterTextChanged(Editable s) {
                if (!st.quiet) nativeOnEvent(id, K_TEXT_CHANGED, 0, s.toString());
            }
            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            public void onTextChanged(CharSequence s, int a, int b, int c) {}
        });
        // Focus + submit (docs/focus.md): kind 16 reports the gain/loss pair; kind 17 is the
        // IME action ("done"/enter). Returning false keeps the platform default (dismiss).
        e.setOnFocusChangeListener(new View.OnFocusChangeListener() {
            public void onFocusChange(View x, boolean hasFocus) {
                if (!st.quiet) nativeOnEvent(id, K_FOCUS_CHANGED, hasFocus ? 1 : 0, null);
            }
        });
        e.setOnEditorActionListener(new TextView.OnEditorActionListener() {
            public boolean onEditorAction(TextView x, int actionId, KeyEvent ev) {
                // A real IME action (done/next/go/search/send, whatever `setInputTraits`
                // labeled the key), or a hardware-enter key-down: the unspecified-action
                // key-up call must not fire a second submit.
                boolean action = actionId != EditorInfo.IME_ACTION_NONE
                        && actionId != EditorInfo.IME_ACTION_UNSPECIFIED;
                boolean enter = ev != null && ev.getKeyCode() == KeyEvent.KEYCODE_ENTER
                        && ev.getAction() == KeyEvent.ACTION_DOWN;
                if (action || enter) nativeOnEvent(id, K_SUBMITTED, 0, null);
                return false;
            }
        });
        box.addView(e, new TextInputLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        return box;
    }
    public static void setTextField(View v, String value) {
        EditText e = editTextOf(v);
        if (!e.getText().toString().equals(value)) { // controlled input (§4.4)
            // Quiet: Day wrote this text, so it is not the user's edit. Reported, it arrived
            // later than the write through the event queue, and an app that had moved on (an
            // address bar that completed "kit" to "kitchen.example") read the stale echo as the
            // user deleting the completion. UIKit and AppKit do not report a programmatic set.
            InputState st = inputStates.get(v);
            boolean wasQuiet = st != null && st.quiet;
            if (st != null) st.quiet = true;
            try {
                e.setText(value);
                e.setSelection(value.length());
            } finally {
                if (st != null) st.quiet = wasQuiet;
            }
        }
    }
    /** Select code points `start..end` of a text field (docs/textfield.md "Selection"). Day
     *  counts Unicode scalar values and an EditText UTF-16 units, so each end is walked onto the
     *  text, held to its end. The caret lands on `end`; an EditText keeps the selection while
     *  it is not focused. */
    public static void setTextFieldSelection(View v, int start, int end) {
        EditText e = editTextOf(v);
        CharSequence t = e.getText();
        int n = Character.codePointCount(t, 0, t.length());
        int a = Character.offsetByCodePoints(t, 0, Math.min(Math.max(start, 0), n));
        int b = Character.offsetByCodePoints(t, 0, Math.min(Math.max(end, 0), n));
        e.setSelection(a, b);
    }
    public static void setPlaceholder(View v, String value) {
        if (v instanceof TextInputLayout) ((TextInputLayout) v).setHint(value);
        else ((EditText) v).setHint(value);
    }

    /** How a text field takes its text (docs/textfield.md), the whole set on every call.
     *  `purpose` is day-spec's `InputPurpose` in declaration order: 0 text, 1 name, 2 email,
     *  3 url, 4 phone, 5 number, 6 decimal, 7 username, 8 password, 9 new password, 10 one-time
     *  code. `submitLabel` is `SubmitLabel`: 0 return, 1 done, 2 go, 3 next, 4 search, 5 send.
     *  The length bound stays with day-pieces, which counts characters; a `LengthFilter`
     *  counts UTF-16 units and would refuse text the bound allows. A view `makeTextField` did
     *  not build is left alone. */
    public static void setInputTraits(View v, boolean secure, boolean readOnly, int purpose,
            int submitLabel) {
        final InputState st = inputStates.get(v);
        if (st == null) return;
        if (st.applied && st.secure == secure && st.readOnly == readOnly
                && st.purpose == purpose && st.submitLabel == submitLabel) return;
        final EditText e = editTextOf(v);
        st.quiet = true;
        try {
            // The calls below disturb three things that are not theirs: `setInputType` swaps
            // the typeface (monospace for a password variation, the default one on the way
            // out), a transformation swap re-sets the text and parks the caret at 0, and a
            // selectability flip drops focus. Each is put back at the end.
            Typeface face = e.getTypeface();
            int selStart = e.getSelectionStart(), selEnd = e.getSelectionEnd();
            boolean focused = e.hasFocus();
            InputMethodManager imm =
                    (InputMethodManager) ctx.getSystemService(Context.INPUT_METHOD_SERVICE);

            // Mask first, so nothing below can leave a secure field readable if it throws.
            if (secure) {
                e.setTransformationMethod(
                        android.text.method.PasswordTransformationMethod.getInstance());
            }

            // Purpose and secure merge into the one input type. No type here carries the
            // multi-line flag, so the field stays single-line through every change.
            final int T = android.text.InputType.TYPE_CLASS_TEXT;
            final int N = android.text.InputType.TYPE_CLASS_NUMBER;
            final int quietText = T | android.text.InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
            int type;
            switch (purpose) {
                case 1: type = quietText | android.text.InputType.TYPE_TEXT_VARIATION_PERSON_NAME
                        | android.text.InputType.TYPE_TEXT_FLAG_CAP_WORDS; break;
                case 2: type = T | android.text.InputType.TYPE_TEXT_VARIATION_EMAIL_ADDRESS; break;
                case 3: type = T | android.text.InputType.TYPE_TEXT_VARIATION_URI; break;
                case 4: type = android.text.InputType.TYPE_CLASS_PHONE; break;
                case 5: type = N; break;
                case 6: type = N | android.text.InputType.TYPE_NUMBER_FLAG_DECIMAL; break;
                case 7: case 10: type = quietText; break;
                // A password shown in the clear still wants the password keyboard: no
                // suggestions, nothing learned.
                case 8: case 9:
                    type = T | android.text.InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD; break;
                default: type = st.baseType; break;
            }
            if (secure) {
                int cls = type & android.text.InputType.TYPE_MASK_CLASS;
                if (cls == T) type = T | android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD;
                else if (cls == N) type |= android.text.InputType.TYPE_NUMBER_VARIATION_PASSWORD;
                // The phone class has no password variation; the transformation below masks it.
            }
            boolean restart = false;
            if (readOnly) {
                // No key listener means no edits and (its input type being TYPE_NULL) no IME.
                // The input type waits until the field takes edits again: `setInputType`
                // would put a key listener straight back.
                if (e.getKeyListener() != null) e.setKeyListener(null);
            } else if (e.getInputType() != type || e.getKeyListener() == null) {
                e.setInputType(type); // also re-installs the key listener read-only removed
            }
            // The input type only manages the transformation for its own password variations,
            // and un-masks on its way out of one even when the next type (phone) is still
            // secure. Naming it outright covers every pairing, both directions.
            e.setTransformationMethod(secure
                    ? android.text.method.PasswordTransformationMethod.getInstance()
                    : android.text.method.SingleLineTransformationMethod.getInstance());

            // Read-only: selectable and copyable, focusable, no edits, no keyboard.
            // `setTextIsSelectable` is what keeps the selection handles alive without a key
            // listener; switching it off again strips the movement method and the focusable
            // and clickable flags an EditText needs, so those go back by hand.
            if (e.isTextSelectable() != readOnly) {
                e.setTextIsSelectable(readOnly);
                if (!readOnly) {
                    e.setMovementMethod(android.text.method.ArrowKeyMovementMethod.getInstance());
                    e.setClickable(true);
                    e.setLongClickable(true);
                }
                e.setFocusable(true);
                e.setFocusableInTouchMode(true);
                if (readOnly && focused && imm != null) {
                    imm.hideSoftInputFromWindow(e.getWindowToken(), 0);
                }
                restart = true;
            }
            e.setShowSoftInputOnFocus(!readOnly);

            // The action key: only the action bits change, any flags stay.
            int action;
            switch (submitLabel) {
                case 1: action = EditorInfo.IME_ACTION_DONE; break;
                case 2: action = EditorInfo.IME_ACTION_GO; break;
                case 3: action = EditorInfo.IME_ACTION_NEXT; break;
                case 4: action = EditorInfo.IME_ACTION_SEARCH; break;
                case 5: action = EditorInfo.IME_ACTION_SEND; break;
                default: action = EditorInfo.IME_ACTION_UNSPECIFIED; break; // the IME's own key
            }
            int ime = (e.getImeOptions() & ~EditorInfo.IME_MASK_ACTION) | action;
            if (ime != e.getImeOptions()) {
                e.setImeOptions(ime);
                restart = true;
            }

            // Autofill (API 26): the hint names are the framework's and androidx.autofill's.
            if (android.os.Build.VERSION.SDK_INT >= 26) {
                String hint;
                switch (purpose) {
                    case 1: hint = "name"; break;
                    case 2: hint = "emailAddress"; break;
                    case 4: hint = "phone"; break;
                    case 7: hint = "username"; break;
                    case 8: hint = "password"; break;
                    case 9: hint = "newPassword"; break;
                    case 10: hint = "smsOTPCode"; break;
                    default: hint = null; break;
                }
                if (hint != null) e.setAutofillHints(hint);
                else e.setAutofillHints((String[]) null);
                e.setImportantForAutofill(hint != null
                        ? View.IMPORTANT_FOR_AUTOFILL_YES : View.IMPORTANT_FOR_AUTOFILL_AUTO);
            }

            if (e.getTypeface() != face) e.setTypeface(face);
            if (focused && !e.hasFocus()) e.requestFocus();
            int len = e.length();
            if (selStart >= 0 && selEnd >= 0) {
                e.setSelection(Math.min(selStart, len), Math.min(selEnd, len));
            }
            // The IME caches the editor's type and action key; tell it they moved.
            if (restart && imm != null) imm.restartInput(e);

            st.applied = true;
            st.secure = secure;
            st.readOnly = readOnly;
            st.purpose = purpose;
            st.submitLabel = submitLabel;
        } catch (Throwable t) {
            android.util.Log.w("Day", "setInputTraits: " + t);
        } finally {
            st.quiet = false;
        }
    }

    /** Drive focus (docs/focus.md): request it (raising the IME for editables), or resign it
     *  to the focusable content root, dismissing the IME. Resign only acts while this view (or
     *  its inner editable) still owns focus, so a stale release can't blur a sibling. */
    public static void focusView(View v, boolean focused) {
        final View target = (v instanceof TextInputLayout) ? editTextOf(v) : v;
        InputMethodManager imm =
                (InputMethodManager) ctx.getSystemService(Context.INPUT_METHOD_SERVICE);
        if (focused) {
            if (target.requestFocus() && target instanceof EditText && imm != null) {
                imm.showSoftInput(target, 0);
            }
        } else if (target.hasFocus()) {
            if (target.onCheckIsTextEditor() || target instanceof android.webkit.WebView) {
                DayKeyboard.endEditing(target);
            } else {
                target.clearFocus();
            }
        }
    }

    public static View makeToggle(final long id, boolean value, boolean enabled) {
        MaterialSwitch s = new MaterialSwitch(ctx); // M3 switch
        s.setChecked(value);
        s.setEnabled(enabled);
        s.setOnCheckedChangeListener(new CompoundButton.OnCheckedChangeListener() {
            public void onCheckedChanged(CompoundButton b, boolean on) {
                nativeOnEvent(id, K_TOGGLE_CHANGED, on ? 1 : 0, null);
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
        // Two facts per interaction: the live value, and the one the user settled on. A drag
        // streams the first and ends with the second; a keyboard or a11y change never starts a
        // touch, so it is settled the moment it lands. The flag is a one-element array because a
        // Java anonymous class can only capture effectively-final locals.
        final boolean[] dragging = { false };
        s.addOnChangeListener(new Slider.OnChangeListener() {
            @Override public void onValueChange(Slider slider, float v, boolean fromUser) {
                if (!fromUser) return;
                nativeOnEvent(id, K_VALUE_CHANGED, v, null);
                if (!dragging[0]) nativeOnEvent(id, K_VALUE_COMMITTED, v, null);
            }
        });
        s.addOnSliderTouchListener(new Slider.OnSliderTouchListener() {
            @Override public void onStartTrackingTouch(Slider slider) {
                dragging[0] = true;
            }
            @Override public void onStopTrackingTouch(Slider slider) {
                dragging[0] = false;
                nativeOnEvent(id, K_VALUE_COMMITTED, slider.getValue(), null);
            }
        });
        return s;
    }
    public static void setSlider(View v, double value, double ignoredMin) {
        Slider s = (Slider) v;
        float f = (float) Math.max(s.getValueFrom(), Math.min(s.getValueTo(), value));
        // A stepped slider (e.g. day-tweak-slider-tick-marks) hard-crashes at the next layout pass
        // unless every value is valueFrom + n*stepSize (BaseSlider.validateValues throws), so
        // snap programmatic writes onto the step grid defensively.
        float step = s.getStepSize();
        if (step > 0f) {
            f = s.getValueFrom() + Math.round((f - s.getValueFrom()) / step) * step;
            f = Math.max(s.getValueFrom(), Math.min(s.getValueTo(), f));
        }
        if (s.getValue() != f) s.setValue(f); // programmatic: listener sees fromUser=false, no echo
    }

    public static View makeDivider(boolean vertical) {
        if (!vertical) return new MaterialDivider(ctx); // themed hairline (colorOutlineVariant)
        // MaterialDivider draws a horizontal line at the top of its bounds whatever its shape,
        // so a vertical rule is a plain view filled with the same theme color; layout gives it
        // its 1dp-wide frame.
        View v = new View(ctx);
        v.setBackgroundColor(MaterialColors.getColor(
                ctx, com.google.android.material.R.attr.colorOutlineVariant, 0x33808080));
        return v;
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

    /** Combobox (day-piece-combobox): the M3 exposed dropdown menu: a TextInputLayout in the
     *  theme's filled-dropdown style hosting a non-editable MaterialAutoCompleteTextView. */
    public static View makeSpinner(final long id, String joinedItems, int selected) {
        final String[] items = joinedItems.split("\n");
        TextInputLayout box = new TextInputLayout(ctx, null,
                com.google.android.material.R.attr.textInputFilledExposedDropdownMenuStyle);
        MaterialAutoCompleteTextView tv = new MaterialAutoCompleteTextView(box.getContext());
        tv.setInputType(android.text.InputType.TYPE_NULL); // select-only, no free text
        tv.setSimpleItems(items);
        // Size to the widest item (an `UNSPECIFIED` probe of the box ignores prospective values):
        // text width + the box's start padding and end (dropdown-icon) inset. The minimum goes on
        // the TextInputLayout itself: LinearLayout honors its own suggested minimum during an
        // `UNSPECIFIED` measure, but nothing propagates a child EditText minimum up through the
        // box.
        float widest = 0f;
        for (String it : items) widest = Math.max(widest, tv.getPaint().measureText(it));
        float d = ctx.getResources().getDisplayMetrics().density;
        box.setMinimumWidth((int) (widest + 76 * d));
        if (selected >= 0 && selected < items.length) tv.setText(items[selected], false);
        tv.setOnItemClickListener(new AdapterView.OnItemClickListener() {
            public void onItemClick(AdapterView<?> p, View v, int pos, long rowId) {
                nativeOnEvent(id, K_SELECTION_CHANGED, pos, null);
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
        // A presented cover shell lives under the activity content root; a day-tree
        // re-insert (z-order re-sync among its siblings) must not re-parent it; doing so
        // froze its slide mid-flight and stranded its posted callbacks (docs/cover.md).
        if (child instanceof DayCover && ((DayCover) child).presented) return;
        if (parent instanceof DayNavHost) { ((DayNavHost) parent).add(child); return; }
        if (parent instanceof DayTabs) {
            // The suite's own pages. The chrome-source page is marked before insertion and shown
            // as no tab; every other page is a destination, in row order.
            if (navChromePages.remove(child)) {
                ((DayTabs) parent).addChromePage(child);
            } else {
                ((DayTabs) parent).addPage(child);
            }
            return;
        }
        View target = contentOf(parent);
        if (target instanceof ViewGroup) ((ViewGroup) target).addView(child);
    }

    /**
     * Put an existing child at `index` among its siblings: Day's z-order resync
     * (Toolkit::move_child), which walks the whole sibling row and asks for each position in
     * turn.
     *
     * A ViewGroup has no move primitive, so the fallback is remove-then-add, and that detaches:
     * {@link ViewGroup#removeView} unfocuses the view and everything under it, so a moved subtree
     * containing the field the user is typing into loses its focus and the soft keyboard
     * (docs/focus.md). Hence the two rules here: a child already at `index` is left alone, and one
     * that has to move is re-added at that index rather than appended; appending would make the
     * caller's next position wrong and drag every later sibling through a detach as well.
     */
    public static void moveChild(View parent, View child, int index) {
        // See addChild: a presented cover shell is native-owned and keeps its own z-order.
        if (child instanceof DayCover && ((DayCover) child).presented) return;
        View target = contentOf(parent);
        if (target instanceof ViewGroup && child.getParent() == target) {
            ViewGroup g = (ViewGroup) target;
            if (g.indexOfChild(child) == index) return;
            g.removeView(child);
            g.addView(child, Math.max(0, Math.min(index, g.getChildCount())));
            return;
        }
        // Not a plain child of this group: a nav page, a tab page, a presented cover. Those route
        // through their host, which owns their order itself.
        removeChild(child);
        addChild(parent, child);
    }

    public static void removeChild(View child) {
        // See addChild: a presented cover shell is native-owned.
        if (child instanceof DayCover && ((DayCover) child).presented) return;
        DayKeyboard.endEditing(child);
        // Nav pages route through their host (looked up by view, since the FragmentManager may
        // have the page detached mid-transition, so the parent chain can't be relied on).
        DayNavHost navHost = DayNavHost.pageHosts.get(child);
        if (navHost != null) {
            navHost.removePage(child);
            return;
        }
        ViewGroup p = (ViewGroup) child.getParent();
        if (p != null && p.getParent() instanceof DayTabs) {
            ((DayTabs) p.getParent()).removePage(child);
            return;
        }
        if (p != null) p.removeView(child);
    }
    /** A child's frame in px in its DayFixed. With `timing` (src/lib.rs `motion_wire`) it
     *  moves there from the frame it shows, through a run that places it each frame; without,
     *  it is placed at once and any run on it ends. */
    public static void setFrame(View v, int x, int y, int w, int h, int[] timing, float[] keys) {
        // See addChild: a presented cover shell is positioned natively (fullscreen).
        if (v instanceof DayCover && ((DayCover) v).presented) return;
        ViewGroup p = (ViewGroup) v.getParent();
        // Nav / tab pages fill the host's page frame; their frames are native-owned.
        if (p != null && p.getParent() instanceof DayNavHost) return;
        if (p != null && p.getParent() instanceof DayTabs) return;
        if (!(p instanceof DayFixed)) return;
        final DayFixed f = (DayFixed) p;
        cancelRun(v, ANIM_FRAME);
        // A child placed for the first time has no frame to move from: it appears in place.
        final int[] from = f.childFrame(v);
        if (!animates(timing) || from == null) {
            f.setChildFrame(v, x, y, w, h);
            return;
        }
        startRun(v, ANIM_FRAME, timing, keys, q -> {
            if (v.getParent() != f) return;
            f.setChildFrame(v,
                    Math.round(lerp(from[0], x, q)), Math.round(lerp(from[1], y, q)),
                    Math.max(0, Math.round(lerp(from[2], w, q))),
                    Math.max(0, Math.round(lerp(from[3], h, q))));
        }, () -> {
            if (v.getParent() == f) f.setChildFrame(v, x, y, w, h);
        });
    }

    // --- navigation (docs/navigation.md) ---
    public static View makeNavHost(long id, String title, boolean adaptive, float tileMinDp) {
        // The window's toolbar reaches the host once the host is in a window and its window is
        // known (navHostAttached): a host built for a cover must never carry the items of the
        // window underneath.
        return new DayNavHost(ctx, id, title, adaptive, tileMinDp);
    }

    /** The window toolbar spec as the app last set it (docs/toolbars.md), per WINDOW ROOT: the
     *  view the Rust side hands setWindowToolbar, which is the day root for the primary window
     *  and the DayCover shell for a window presented as a cover (docs/cover.md). Weak, so a
     *  closed cover's record goes with its view. */
    static final java.util.WeakHashMap<View, String> windowToolbarSpecs =
            new java.util.WeakHashMap<>();

    /** The window root a view belongs to: the DayCover shell when it sits inside a cover, else
     *  the nearest ancestor a toolbar call has named (the primary window's day root). Null when
     *  no toolbar call has reached this window yet; setWindowToolbar then finds the host under
     *  the root itself. */
    static View windowRootOf(View v) {
        for (View p = v; p != null; p = p.getParent() instanceof View ? (View) p.getParent() : null) {
            if (p instanceof DayCover) return p;
            if (windowToolbarSpecs.containsKey(p) || toolbarHosts.containsKey(p)) return p;
        }
        return null;
    }

    /** The host carrying `root`'s bar, when one has claimed it and is still alive. */
    static DayNavHost toolbarHostOf(View root) {
        java.lang.ref.WeakReference<DayNavHost> ref = toolbarHosts.get(root);
        DayNavHost h = ref == null ? null : ref.get();
        if (h == null && ref != null) toolbarHosts.remove(root);
        return h;
    }

    /** A nav host has joined a window (DayNavHost.onAttachedToWindow): the outermost host of
     *  that window carries the window's toolbar from here on. The toolbar is usually set before
     *  the host exists (an app installs its bar as the first thing the window body does), so a
     *  host born after the spec arrived takes it now, and the bar the window carried itself
     *  comes down (docs/toolbars.md).
     *
     *  The outermost host only. A window can hold more than one (a list-backed destination
     *  composes a nested host for its two layers), and giving each of them the window's items
     *  painted a second app bar directly under the first, which is what a tablet showed as the
     *  toolbar drawn twice. A parent attaches before its children, so the first claim is the
     *  outer host's. Best-effort, like the bar actions: a throw never reaches the tree. */
    static void navHostAttached(DayNavHost host) {
        try {
            View root = windowRootOf(host);
            if (root == null) return;
            DayNavHost owner = toolbarHostOf(root);
            // A host on a hidden tab page never takes the bar from the window's own docked bar:
            // the page showing has no host of its own, and its commands ride that bar.
            if (owner == null && !host.isShown() && DayNavHost.WindowBar.DOCKED.containsKey(root)) {
                return;
            }
            if (owner != null && owner != host) {
                // A tab page the window opens on can attach after the first tab's host, which
                // claimed the bar from inside a hidden page: the shown host takes it over.
                if (owner.isShown() || !host.isShown() || isInside(host, owner)) return;
                owner.setWindowToolbar("");
                owner.toolbarRoot = null;
            }
            toolbarHosts.put(root, new java.lang.ref.WeakReference<DayNavHost>(host));
            host.toolbarRoot = new java.lang.ref.WeakReference<View>(root);
            String spec = windowToolbarSpecs.get(root);
            if (spec != null && !spec.isEmpty()) {
                host.setWindowToolbar(spec);
                DayNavHost.WindowBar.undock(root);
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "window toolbar setup failed; continuing without it", t);
        }
    }

    /** A tab page has come to the front (DayTabs): the window's bar moves to the outermost
     *  navigation host on that page. Every tab's page stays attached, hidden while another tab
     *  shows, so the host that claimed the bar first is usually on a page nobody can see: a
     *  second tab showed no commands at all, on its root or on any page pushed over it, while
     *  its items sat on the first tab's hidden app bar. A page with no navigation host (a
     *  section a wide window composes into panes beside the sidebar) carries the window's bar
     *  itself, docked across the top, for the same reason. */
    static void windowToolbarFollow(View page) {
        try {
            // Only the window's own navigation suite moves the bar. A tab set inside a page of a
            // navigation host (a section's Basics / Sizing tabs) leaves it on that host's app
            // bar: docking a bar for its host-less tab page stacked a second bar above the page.
            for (android.view.ViewParent p = page.getParent(); p != null; p = p.getParent()) {
                if (p instanceof DayNavHost) return;
            }
            View root = windowRootOf(page);
            if (root == null) return;
            DayNavHost next = findNavHost(page);
            DayNavHost owner = toolbarHostOf(root);
            if (next != null && next == owner) return;
            if (owner != null) {
                owner.setWindowToolbar("");
                owner.toolbarRoot = null;
                toolbarHosts.remove(root);
            }
            String docked = windowToolbarSpecs.get(root);
            if (next == null) {
                if (docked != null && !docked.isEmpty() && !(root instanceof DayCover)) {
                    DayNavHost.WindowBar.dock(root, docked);
                }
                return;
            }
            toolbarHosts.put(root, new java.lang.ref.WeakReference<DayNavHost>(next));
            next.toolbarRoot = new java.lang.ref.WeakReference<View>(root);
            String spec = windowToolbarSpecs.get(root);
            next.setWindowToolbar(spec == null ? "" : spec);
            DayNavHost.WindowBar.undock(root);
        } catch (Throwable t) {
            android.util.Log.e("Day", "window toolbar move failed; continuing", t);
        }
    }

    /** Whether `v` sits somewhere under `ancestor`. */
    private static boolean isInside(View v, View ancestor) {
        for (android.view.ViewParent p = v.getParent(); p instanceof View; p = p.getParent()) {
            if (p == ancestor) return true;
        }
        return false;
    }

    /** The host carrying a window's bar has left its window (a dismissed cover, a re-parented
     *  host): the slot opens for the next host, and a window still on screen gets its own bar
     *  back so its commands never vanish with the host. A cover's bar goes with its host: the
     *  cover is leaving too (its content is disposed once it has slid away), and a docked bar
     *  for it would land on the activity content, over the window underneath. */
    static void navHostDetached(DayNavHost host) {
        try {
            View root = host.toolbarRoot == null ? null : host.toolbarRoot.get();
            host.toolbarRoot = null;
            if (root == null || toolbarHostOf(root) != host) return;
            toolbarHosts.remove(root);
            if (root instanceof DayCover) return;
            String spec = windowToolbarSpecs.get(root);
            if (spec != null && !spec.isEmpty() && root.isAttachedToWindow()) {
                DayNavHost.WindowBar.dock(root, spec);
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "window toolbar restore failed; continuing without it", t);
        }
    }

    /** A tree is about to be mounted on a new activity (DayActivity, before nativeStart): drop
     *  every process-wide reference to the hosts of the one that went away. day-toolkit-material clears
     *  its own per-mount tables in `init` for the same reason; these are their Java twins.
     *
     *  Without this a recreation (the theme switch, docs/settings.md "Appearance on Android")
     *  left the toolbar slot and `DayNavHost.active` on the destroyed activity's host. The weak
     *  reference stays live until a collection happens to run, so the new tree's first host
     *  never claimed the bar (the slot looked taken), and every setWindowToolbar after it painted
     *  onto a view no window showed: the app bar came up with no items and none ever returned. */
    static void beginMount() {
        toolbarHosts.clear();
        windowToolbarSpecs.clear();
        DayNavHost.active = null;
        // The FragmentManager also restored the previous activity's PageFragments and their
        // back-stack entries from the saved instance state: content-less zombies, since a
        // page's view is Rust-owned and never saved. Left in place, a zombie entry carries the
        // same `day-nav-<node>-<depth>` name the new host gives its own push, so the first pop
        // (inclusive, by name) took both entries at once, and the root came back laid out in
        // full, every row with its frame, yet drawing nothing: a blank navigation list after
        // a theme switch on a pushed page. Clear them before the new tree's hosts register
        // their own; at this point nothing of the new tree has been added yet.
        if (!(ctx instanceof androidx.fragment.app.FragmentActivity)) return;
        androidx.fragment.app.FragmentManager fm =
                ((androidx.fragment.app.FragmentActivity) ctx).getSupportFragmentManager();
        try {
            if (fm.getBackStackEntryCount() > 0) {
                fm.popBackStackImmediate(null,
                        androidx.fragment.app.FragmentManager.POP_BACK_STACK_INCLUSIVE);
            }
            androidx.fragment.app.FragmentTransaction tx = null;
            for (androidx.fragment.app.Fragment f : fm.getFragments()) {
                if (f instanceof DayNavHost.PageFragment
                        && ((DayNavHost.PageFragment) f).content == null) {
                    if (tx == null) tx = fm.beginTransaction().setReorderingAllowed(true);
                    tx.remove(f);
                }
            }
            if (tx != null) tx.commitNowAllowingStateLoss();
        } catch (Throwable t) {
            android.util.Log.e("Day", "restored fragment state not cleared; continuing", t);
        }
    }
    /** The host drawing each window's bar, by window root (see `windowToolbarSpecs`). A window
     *  can hold several hosts (a list-backed destination composes a nested one for its layers),
     *  and only the first to appear carries the window's items; the rest would stack a second
     *  app bar under the first (docs/toolbars.md). Weak both ways, so a closed window's root and
     *  host are collectable. */
    static final java.util.WeakHashMap<View, java.lang.ref.WeakReference<DayNavHost>> toolbarHosts =
            new java.util.WeakHashMap<>();

    /** The host that carries `root`'s bar: the one that claimed it, else a shown host under
     *  `root`. Null while the window docks a bar of its own (the page showing has no host), and
     *  while the only hosts sit on hidden tab pages. */
    static DayNavHost barHostOf(View root) {
        DayNavHost h = toolbarHostOf(root);
        if (h != null) return h;
        if (DayNavHost.WindowBar.DOCKED.containsKey(root)) return null;
        h = findNavHost(root);
        return h != null && h.isShown() ? h : null;
    }

    /** The nav host under `root`, if one has been built. */
    static DayNavHost findNavHost(View root) {
        if (root instanceof DayNavHost) return (DayNavHost) root;
        if (root instanceof ViewGroup) {
            ViewGroup g = (ViewGroup) root;
            for (int i = 0; i < g.getChildCount(); i++) {
                DayNavHost h = findNavHost(g.getChildAt(i));
                if (h != null) return h;
            }
        }
        return null;
    }

    // The window toolbar (docs/toolbars.md): one record per item (day-toolkit-material
    // `serialize_toolbar`), painted onto the app bar of the nav host under `root`, and
    // remembered for the host that comes later (makeNavHost). A window with no nav host gets a
    // bar of its own instead: `Cap::Toolbar` answered Native before the app built the window,
    // so an app that skipped its in-content strip on that answer would otherwise show no
    // commands at all. Wrapped like setNavMenu: a throw here can never reach the tree build.
    public static void setWindowToolbar(View root, String spec) {
        windowToolbarSpecs.put(root, spec == null ? "" : spec);
        try {
            // The host that owns THIS window's bar, once one has claimed it, never a nested one
            // built later for a destination's own layers, and never another window's: the
            // settings cover's stack must not carry the editor's items (and the editor's bar
            // must not come down for it), so the lookup never leaves `root`'s subtree.
            DayNavHost h = barHostOf(root);
            if (h != null && toolbarHostOf(root) == null) {
                toolbarHosts.put(root, new java.lang.ref.WeakReference<DayNavHost>(h));
                h.toolbarRoot = new java.lang.ref.WeakReference<View>(root);
            }
            if (h != null) {
                DayNavHost.WindowBar.undock(root);
                h.setWindowToolbar(spec);
            } else if (spec == null || spec.isEmpty()) {
                DayNavHost.WindowBar.undock(root);
            } else if (!(root instanceof DayCover)) {
                // A cover's items ride the navigation host inside it, which takes them the
                // moment it attaches (navHostAttached); a docked bar would sit on the activity
                // content, over the window the cover is presented on.
                DayNavHost.WindowBar.dock(root, spec);
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "window toolbar setup failed; continuing without it", t);
        }
    }

    /** A page search field's text, written by the app (docs/search.md): into the live field,
     *  without it reporting the text back. */
    public static void updateWindowToolbarText(View root, String id, String text) {
        try {
            DayNavHost h = barHostOf(root);
            if (h != null) {
                h.updateWindowToolbarText(id, text);
            } else {
                DayNavHost.WindowBar.updateDockedText(root, id, text);
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "window toolbar text update failed; continuing", t);
        }
    }

    /** A targeted change to one live toolbar item: op 0 = enabled, 1 = toggle on, 2 = segment,
     *  3 = focus the search field. */
    public static void updateWindowToolbar(View root, String id, int op, double num) {
        try {
            DayNavHost h = barHostOf(root);
            if (h != null) {
                h.updateWindowToolbar(id, op, num);
            } else {
                DayNavHost.WindowBar.updateDocked(root, id, op, num);
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "window toolbar update failed; continuing", t);
        }
    }

    /**
     * Inline search on the navigation list (docs/search.md). Applied after the host is built and
     * wrapped, for the same reason `setNavMenu` above is: a throw on `makeNavHost`'s own path
     * aborts the native tree build and leaves the app blank, so decoration never rides it.
     *
     * Scrolling chrome is opt-in: Material's scroll/enter-always app bar returns on a
     * downward scroll; pinned search remains the default (docs/search.md).
     */
    public static void setNavSearch(View navHost, long id, String prompt, String text, boolean hidesOnScroll,
            boolean initiallyHidden) {
        if (!(navHost instanceof DayNavHost)) {
            return;
        }
        try {
            ((DayNavHost) navHost).setSearch(id, prompt, text, hidesOnScroll, initiallyHidden);
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav search setup failed; continuing without it", t);
        }
    }

    /** Write the app's own query back into the field, without echoing it back as a change. */
    public static void focusNavSearch(View navHost) {
        if (navHost instanceof DayNavHost) ((DayNavHost) navHost).focusSearch();
    }

    public static void setNavSearchText(View navHost, String text) {
        if (!(navHost instanceof DayNavHost)) {
            return;
        }
        try {
            ((DayNavHost) navHost).setSearchText(text);
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav search text sync failed", t);
        }
    }

    public static View makeNavPage(final long id, boolean immersive) {
        DayFixed page = new DayFixed(ctx);
        page.immersivePage = immersive;
        page.addOnLayoutChangeListener(new View.OnLayoutChangeListener() {
            @Override public void onLayoutChange(View v, int l, int t, int r, int b,
                    int ol, int ot, int or2, int ob) {
                int w = r - l, h = b - t;
                if (w != or2 - ol || h != ob - ot) {
                    // kind 6 = FrameChanged, "w,h" in px (Rust divides by density).
                    nativeOnEvent(id, K_FRAME_CHANGED, 0.0, w + "," + h);
                }
            }
        });
        return page;
    }
    public static void navPush(View host, String title, boolean immersive) { ((DayNavHost) host).push(title, immersive); }
    public static void navPop(View host) { ((DayNavHost) host).pop(); }
    public static void navSetTitle(View host, String title) {
        // Tab/rail suites have no title bar; their selected destination owns its own host.
        if (host instanceof DayNavHost) ((DayNavHost) host).retitle(title);
    }
    public static void navSetGuard(View host, boolean on) { ((DayNavHost) host).setGuard(on); }

    // --- fullscreen cover (docs/cover.md) ---
    public static View makeCover(long node, boolean half) { return new DayCover(ctx, node, half); }
    public static void coverPresent(View cover, int bg, boolean hasBg, boolean dismissDisabled) {
        DayCover c = (DayCover) cover;
        if (hasBg) {
            c.setBackgroundColor(bg);
        } else if (!c.half) {
            // Back to the theme's window background: the shell outlives one presentation, and
            // a composed sheet's presentation left it clear (docs/cover.md "Modal sheet").
            android.util.TypedValue tv = new android.util.TypedValue();
            c.setBackgroundColor(ctx.getTheme().resolveAttribute(android.R.attr.colorBackground, tv, true)
                    ? tv.data : 0xFFFFFFFF);
        }
        c.present(dismissDisabled);
    }
    public static void coverSetDismissDisabled(View cover, boolean d) {
        ((DayCover) cover).setDismissDisabled(d);
    }
    public static void coverDismiss(View cover) { ((DayCover) cover).dismissCover(); }

    /** Whether native transitions have settled (Toolkit::ui_idle): dayscript screenshots
     *  wait on this so captures never show a cover mid-slide, and a scripted theme switch
     *  waits through the whole recreation it causes (DayActivity.recreating). */
    public static boolean uiIdle() {
        return DayCover.slidesInFlight == 0 && !DayActivity.recreating();
    }

    static boolean fastAnimations;

    /** Whether the shim's own native transitions (page push, cover slide, list item moves)
     *  jump to their end state: a scripted run (`fastAnimations`) or a user who removed
     *  animations (docs/accessibility.md). The platform already drives its animators at zero
     *  duration for the latter; skipping them outright lands the same frame synchronously, so
     *  a completion callback runs in the same pass instead of one frame later. */
    static boolean skipTransitions() { return fastAnimations || reduceMotion(); }

    private static final class CaptureFence {
        long revision;
        volatile boolean ready;
    }
    private static final java.util.WeakHashMap<View, CaptureFence> captureFences =
        new java.util.WeakHashMap<>();

    /** Return after a fresh frame is submitted, without blocking the UI thread. The
     * external screencap still synchronizes with SurfaceFlinger. An OnDraw fallback
     * covers software rendering and API levels before frame-commit callbacks. */
    public static int prepareSnapshot(View host, long revision) {
        View root = host == null ? ((android.app.Activity) ctx).getWindow().getDecorView()
                                 : host.getRootView();
        if (!root.isAttachedToWindow()) return 0;
        CaptureFence fence = captureFences.get(root);
        if (fence == null || fence.revision != revision) {
            fence = new CaptureFence();
            fence.revision = revision;
            captureFences.put(root, fence);
            final CaptureFence pending = fence;
            if (android.os.Build.VERSION.SDK_INT >= 29 && root.isHardwareAccelerated()) {
                root.getViewTreeObserver().registerFrameCommitCallback(() -> pending.ready = true);
            } else {
                android.view.ViewTreeObserver.OnDrawListener listener =
                    new android.view.ViewTreeObserver.OnDrawListener() {
                        boolean posted;
                        @Override public void onDraw() {
                            if (posted) return;
                            posted = true;
                            root.post(() -> {
                                if (root.getViewTreeObserver().isAlive())
                                    root.getViewTreeObserver().removeOnDrawListener(this);
                                pending.ready = true;
                            });
                        }
                    };
                root.getViewTreeObserver().addOnDrawListener(listener);
            }
            root.invalidate();
        }
        return fence.ready ? 1 : 0;
    }

    /** The app's own system-back callback (Toolkit::set_system_back, docs/navigation.md
     *  "Handling the system Back"). Added first, from DayActivity.onCreate, so it sits beneath
     *  every callback added later: a cover's, a nav host's guard and section back, and the
     *  FragmentManager's (re-added at each ON_START). It hears a back only when none of those
     *  takes it, which is when no cover is up and no page is pushed, and passes it to Rust's
     *  handlers on WINDOW_NODE (id 0). Enabled only while Rust arms it, so a back nothing wants
     *  still leaves the app, predictive "back to home" preview and all. */
    static androidx.activity.OnBackPressedCallback appBack;

    static void installAppBack(androidx.activity.ComponentActivity activity) {
        // A recreated activity starts released: its re-mounted tree arms it again.
        appBack = new androidx.activity.OnBackPressedCallback(false) {
            @Override public void handleOnBackPressed() {
                nativeOnEvent(0L, K_NAV_BACK, 0.0, null);
            }
        };
        activity.getOnBackPressedDispatcher().addCallback(appBack);
    }

    /** Arm or release the app's system-back callback (Toolkit::set_system_back). */
    public static void setAppBack(boolean on) {
        if (appBack != null) appBack.setEnabled(on);
    }

    /** Press the system back (Toolkit::native_back): the dispatcher runs the nav host's guard
     *  callback or the fragment manager's pop, the same path a back gesture takes. Only when
     *  a nav host has something to pop or a guard armed, or the app's handler armed the system
     *  back; with none, the dispatcher would finish the activity, which no walkthrough means. */
    public static boolean nativeBack() {
        // All tabs may be built eagerly. The last constructed host need not be visible.
        View root = ((android.app.Activity) ctx).getWindow().getDecorView();
        if (!hasVisibleBackHost(root) && !(appBack != null && appBack.isEnabled())) return false;
        ((androidx.fragment.app.FragmentActivity) ctx).getOnBackPressedDispatcher().onBackPressed();
        return true;
    }

    private static boolean hasVisibleBackHost(View view) {
        if (!view.isShown()) return false;
        if (view instanceof DayNavHost && ((DayNavHost) view).canBack()) return true;
        if (view instanceof ViewGroup) {
            ViewGroup group = (ViewGroup) view;
            for (int i = 0; i < group.getChildCount(); i++) {
                if (hasVisibleBackHost(group.getChildAt(i))) return true;
            }
        }
        return false;
    }

    /** `Toolkit::system_color`: the theme's Material 3 color for a Day `SystemColor`, by its
     *  wire number (`SystemColor::ALL` order), as ARGB: 0 accent (colorPrimary), 1 text
     *  (colorOnSurface), 2 secondary text (colorOnSurfaceVariant), 3 background (colorSurface,
     *  what the window and the status bar behind it show), 4 surface (colorSurfaceContainer),
     *  5 tertiary text (colorOnSurfaceVariant at 60%), 6 placeholder (textColorHint), 7 link
     *  (textColorLink), 8 separator (colorOutlineVariant), 9 control fill
     *  (colorSurfaceContainerHighest), 10 grouped background (colorSurfaceContainerLow),
     *  11 elevated surface (colorSurfaceContainerHigh), 12 selection (textColorHighlight),
     *  13 destructive (colorError). 0, fully transparent, for success and warning, which
     *  Material 3 has no role for, and when there is no themed context yet: Rust then uses
     *  Day's fallback for that role.
     *
     *  Read from the activity's theme, which is the one its views were inflated with. A light
     *  or dark switch recreates the activity (DayActivity.onConfigurationChanged), and the
     *  re-mount reads every role again from the new one (day-core reads the palette when the
     *  tree is installed) before the app's views are built and resolve their inks, so the
     *  palette Day resolves inks against is always the theme on screen. */
    public static int systemColor(int role) {
        if (ctx == null) return 0;
        int attr;
        int alpha = -1;
        switch (role) {
            case 0: attr = androidx.appcompat.R.attr.colorPrimary; break;
            case 1: attr = com.google.android.material.R.attr.colorOnSurface; break;
            case 2: attr = com.google.android.material.R.attr.colorOnSurfaceVariant; break;
            case 3: attr = com.google.android.material.R.attr.colorSurface; break;
            case 4: attr = com.google.android.material.R.attr.colorSurfaceContainer; break;
            case 5: attr = com.google.android.material.R.attr.colorOnSurfaceVariant; alpha = 153; break;
            case 6: attr = android.R.attr.textColorHint; break;
            case 7: attr = android.R.attr.textColorLink; break;
            case 8: attr = com.google.android.material.R.attr.colorOutlineVariant; break;
            case 9: attr = com.google.android.material.R.attr.colorSurfaceContainerHighest; break;
            case 10: attr = com.google.android.material.R.attr.colorSurfaceContainerLow; break;
            case 11: attr = com.google.android.material.R.attr.colorSurfaceContainerHigh; break;
            case 12: attr = android.R.attr.textColorHighlight; break;
            case 13: attr = androidx.appcompat.R.attr.colorError; break;
            default: return 0;
        }
        int argb = MaterialColors.getColor((android.content.Context) ctx, attr, 0);
        if (alpha >= 0 && argb != 0) argb = (argb & 0x00FFFFFF) | (alpha << 24);
        return argb;
    }

    /** Whether the system renders in dark appearance (Toolkit::dark_mode). */
    public static boolean isDarkMode() {
        int night = ((android.content.Context) ctx).getResources().getConfiguration().uiMode
                & android.content.res.Configuration.UI_MODE_NIGHT_MASK;
        return night == android.content.res.Configuration.UI_MODE_NIGHT_YES;
    }

    /** Whether this device can be told to use a light or dark appearance for this app alone.
     *
     *  `UiModeManager.setApplicationNightMode` arrived in API 31. The older route,
     *  `AppCompatDelegate.setDefaultNightMode`, only restyles an activity that runs through
     *  AppCompat's delegate, and `DayActivity` is a plain `FragmentActivity`, so below 31 there
     *  is nothing to offer, and `Cap::Appearance` says so rather than showing a control that
     *  would do nothing. */
    public static boolean canSetAppearance() {
        return android.os.Build.VERSION.SDK_INT >= 31;
    }

    /** Apply an app-level appearance (Toolkit::set_appearance): 0 light, 1 dark, 2 follow system.
     *
     *  Applying it changes the app's uiMode, which the OS delivers to `DayActivity` as a
     *  configuration change (the manifest lists `uiMode`, so the activity is not recreated), and
     *  that path already calls `appearanceChanged()`. So this only has to ask; the report back to
     *  day-core is the same one a user flipping the system theme produces. */
    public static void setAppearance(int mode) {
        if (!canSetAppearance() || ctx == null) return;
        // Idempotent, and it has to be. The settings row re-applies its stored value every time
        // the tree is built, and applying one recreates the activity, which builds the tree
        // again. Without this the app recreates itself forever.
        //
        // For an explicit light or dark the current uiMode answers exactly, which also means a
        // cold start whose stored choice already matches the system costs no recreation at all.
        // "Follow the system" cannot be read back, so it leans on the remembered value; a static
        // is enough because recreation keeps the process.
        if (mode == appliedNightMode) return;
        if (mode != 2 && (mode == 1) == isDarkMode()) {
            appliedNightMode = mode;
            return;
        }
        appliedNightMode = mode;
        android.app.UiModeManager um = (android.app.UiModeManager)
                ((android.content.Context) ctx).getSystemService(android.content.Context.UI_MODE_SERVICE);
        if (um == null) return;
        int night;
        switch (mode) {
            case 0: night = android.app.UiModeManager.MODE_NIGHT_NO; break;
            case 1: night = android.app.UiModeManager.MODE_NIGHT_YES; break;
            default: night = android.app.UiModeManager.MODE_NIGHT_AUTO; break;
        }
        // Ask the platform for the mode, and stop there. Nothing here recreates the activity:
        // this call is a binder round trip, so a `recreate()` issued beside it re-resolves the
        // theme against whichever configuration has landed by then, which is how a Dark-to-Light
        // pick came back with a light app bar over a dark sidebar. The platform answers with a
        // uiMode configuration change once the override is in force, and `DayActivity` recreates
        // from there (docs/settings.md "Appearance on Android"), so the new activity always
        // resolves against the appearance that was asked for. A pick the platform has nothing
        // to do (the mode already in force) produces no configuration change and no
        // recreation, which is why the guards above can afford to be exact.
        DayActivity.armRecreate();
        um.setApplicationNightMode(night);
    }

    /** The appearance last applied, so re-applying the stored choice does not recreate again. */
    private static int appliedNightMode = Integer.MIN_VALUE;

    /** Report a light/dark switch to native (event kind 25), once the app has started.
     *  DayActivity calls this from onConfigurationChanged; day-core restyles what it owns and
     *  rebuilds app-painted surfaces. */
    public static void appearanceChanged() {
        if (started) nativeOnEvent(0L, K_APPEARANCE_CHANGED, 0, "");
    }

    /** The activity is about to recreate itself (DayActivity.onConfigurationChanged: a theme or
     *  language switch): day-toolkit-material saves the full current route into `navState` under its
     *  reserved `day.route` key, so the re-mounted tree comes back to the same section and
     *  pushed page (docs/settings.md "Appearance on Android"). Routed through the custom-event
     *  trampoline rather than a bridge kind of its own: the kind table is shared wire format,
     *  and this never leaves the Android shim. */
    public static void willRecreate() {
        if (started) nativeOnEvent(0L, K_CUSTOM, 0, "day:will-recreate");
    }

    /** The mounted tree's first build has settled, and on a re-mount its route is restored
     *  (day-toolkit-material `mount_settled`, one turn after `nativeStart`): the activity may now fade
     *  the previous window's capture away. Called from Rust on the main thread. */
    public static void mountSettled() {
        if (ctx instanceof DayActivity) ((DayActivity) ctx).mountSettled();
    }

    /** Whether the user asked the system for less motion (Toolkit::reduce_motion,
     *  docs/accessibility.md): Accessibility's "Remove animations", which writes the global
     *  animator duration scale to zero (Developer options' "Animator duration scale: off" is
     *  the same write).
     *
     *  Read from the settings provider every time rather than from
     *  `ValueAnimator.areAnimatorsEnabled()`: that answer is the scale the window manager last
     *  pushed into this process, on a binder path of its own, so at the moment the observer
     *  below fires it can still hold the old value. The provider is current by definition, and
     *  its in-process cache makes a repeat read cheap. */
    public static boolean reduceMotion() {
        if (ctx == null) return false;
        try {
            return android.provider.Settings.Global.getFloat(ctx.getContentResolver(),
                    android.provider.Settings.Global.ANIMATOR_DURATION_SCALE, 1f) == 0f;
        } catch (RuntimeException e) {
            return false;
        }
    }

    /** Watch the animator duration scale (docs/accessibility.md) and report each flip to native
     *  as event kind 33, the way a light/dark switch is reported as 25. The caller keeps the
     *  returned observer and hands it back to `unwatchReduceMotion`: DayActivity registers from
     *  onCreate and unregisters from onDestroy, and holding it per activity keeps a recreation
     *  honest whichever of the two runs first. The observer's handler is the main looper, so
     *  `onChange` arrives on Day's UI thread and the report crosses straight into Rust, where
     *  day-core re-reads `reduceMotion()`. `null` when the registration failed. */
    public static android.database.ContentObserver watchReduceMotion(Context host) {
        android.database.ContentObserver observer = new android.database.ContentObserver(main) {
            @Override public void onChange(boolean selfChange) {
                if (started) nativeOnEvent(0L, K_REDUCE_MOTION_CHANGED, 0, "");
            }
        };
        try {
            host.getContentResolver().registerContentObserver(
                    android.provider.Settings.Global.getUriFor(
                            android.provider.Settings.Global.ANIMATOR_DURATION_SCALE),
                    false, observer);
        } catch (RuntimeException e) {
            android.util.Log.w("Day", "animator duration scale observer not registered", e);
            return null;
        }
        return observer;
    }

    /** Take down an observer `watchReduceMotion` returned. */
    public static void unwatchReduceMotion(Context host, android.database.ContentObserver observer) {
        if (observer == null) return;
        try {
            host.getContentResolver().unregisterContentObserver(observer);
        } catch (RuntimeException e) {
            android.util.Log.w("Day", "animator duration scale observer not unregistered", e);
        }
    }

    /** A PNG of this app's own window (docs/window-image.md), or `null` when there is nothing
     *  to draw.
     *
     *  `View.draw(Canvas)` rather than `PixelCopy`: it is synchronous, which is what lets
     *  `day::window_image()` stay a plain call on every backend. The cost is that
     *  surface-backed content (a `VideoView`, and any future GL or camera view) draws empty:
     *  those pixels live on a surface the view tree never touches, and only the async PixelCopy
     *  can read them back.
     *
     *  `chrome` picks the decor view (the whole window, action bar and system-bar backgrounds)
     *  over the app's content view. */
    /** Where `root` sits in {@link #windowImage}'s content capture, as "x,y" in dp, or null
     *  when there is no window. The content view runs edge-to-edge, under the status bar above
     *  and the navigation bar below, while Day's root is inset from both. */
    public static String snapshotOrigin(View root) {
        try {
            if (!(ctx instanceof android.app.Activity)) return null;
            View content = ((android.app.Activity) ctx).getWindow()
                    .findViewById(android.R.id.content);
            if (content == null || root == null) return null;
            int[] at = new int[2];
            int[] origin = new int[2];
            root.getLocationInWindow(at);
            content.getLocationInWindow(origin);
            float d = root.getResources().getDisplayMetrics().density;
            if (d <= 0f) return null;
            return ((at[0] - origin[0]) / (double) d) + "," + ((at[1] - origin[1]) / (double) d);
        } catch (Throwable t) {
            return null;
        }
    }

    public static byte[] windowImage(boolean chrome) {
        try {
            if (!(ctx instanceof android.app.Activity)) return null;
            android.app.Activity act = (android.app.Activity) ctx;
            View view = chrome
                    ? act.getWindow().getDecorView()
                    : act.getWindow().findViewById(android.R.id.content);
            if (view == null || view.getWidth() <= 0 || view.getHeight() <= 0) return null;
            android.graphics.Bitmap bmp = android.graphics.Bitmap.createBitmap(
                    view.getWidth(), view.getHeight(), android.graphics.Bitmap.Config.ARGB_8888);
            view.draw(new android.graphics.Canvas(bmp));
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            bmp.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out);
            bmp.recycle();
            return out.toByteArray();
        } catch (Throwable t) {
            android.util.Log.e("Day", "window image failed", t);
            return null;
        }
    }

    // --- launcher shortcuts and the display (docs/deep-links.md, docs/windows.md) ----------

    /** Whether this device keeps shortcuts the app sets while it runs: ShortcutManager is
     *  API 25, one above the minimum. */
    public static boolean canSetLauncherShortcuts() {
        return android.os.Build.VERSION.SDK_INT >= 25;
    }

    /** Rust's `set_launcher_shortcuts`: the app's dynamic shortcuts, replaced as a whole.
     *  `packed` holds one record per shortcut, separated by U+001E, each a route and a label
     *  separated by U+001F (control characters neither can contain). Each carries the same intent the
     *  Day.toml [[shortcuts]] declare (`day build` writes those): VIEW of `<scheme>://<route>`
     *  at the launcher activity, so a press reaches the deep-link intake (`DAY_OPEN_URL` on a
     *  cold start, `onNewIntent` on a warm one) exactly as theirs does. The launcher shows the
     *  static ones too, and the platform caps the two together per activity, so what does not
     *  fit is left out rather than failing the whole call. */
    public static void setLauncherShortcuts(String packed) {
        if (!canSetLauncherShortcuts() || ctx == null) return;
        try {
            String[] records = packed.isEmpty() ? new String[0] : packed.split("\u001e", -1);
            Context app = ctx.getApplicationContext();
            android.content.pm.ShortcutManager sm =
                    app.getSystemService(android.content.pm.ShortcutManager.class);
            if (sm == null) return;
            android.content.Intent launch =
                    app.getPackageManager().getLaunchIntentForPackage(app.getPackageName());
            android.content.ComponentName target = launch == null ? null : launch.getComponent();
            if (target == null) return;
            String scheme = deepLinkScheme(app);
            int room = Math.max(0,
                    sm.getMaxShortcutCountPerActivity() - sm.getManifestShortcuts().size());
            int icon = app.getApplicationInfo().icon;
            java.util.List<android.content.pm.ShortcutInfo> list =
                    new java.util.ArrayList<android.content.pm.ShortcutInfo>();
            for (int i = 0; i < records.length && list.size() < room; i++) {
                String[] fields = records[i].split("\u001f", -1);
                String route = fields[0];
                String text = fields.length > 1 ? fields[1] : "";
                String link = scheme == null ? route : scheme + "://" + route;
                android.content.Intent intent = new android.content.Intent(
                        android.content.Intent.ACTION_VIEW, android.net.Uri.parse(link))
                        .setComponent(target);
                String label = text.isEmpty() ? route : text;
                if (label.isEmpty()) continue;
                android.content.pm.ShortcutInfo.Builder b =
                        new android.content.pm.ShortcutInfo.Builder(app, "day_dynamic_" + i)
                                .setShortLabel(label)
                                .setLongLabel(label)
                                .setIntent(intent)
                                .setRank(i);
                if (icon != 0) b.setIcon(android.graphics.drawable.Icon.createWithResource(app, icon));
                list.add(b.build());
            }
            // False when the platform rate-limits a background app; the app is in front
            // whenever its own code asks, so this is not expected to bite.
            if (!sm.setDynamicShortcuts(list)) {
                android.util.Log.w("Day", "setDynamicShortcuts was rate-limited");
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "setLauncherShortcuts failed", t);
        }
    }

    private static boolean schemeRead;
    private static String scheme;

    /** The app's deep-link scheme, as `day build` picks it for the static shortcuts: the first
     *  `<data android:scheme>` in the manifest. The scaffold fills it in from Day.toml at build
     *  time, so the compiled manifest in the APK is where it can be read back. `null` when there
     *  is none: the shortcut then carries the bare route, which the intake passes through. */
    static String deepLinkScheme(Context app) {
        if (schemeRead) return scheme;
        schemeRead = true;
        try (android.content.res.XmlResourceParser p =
                     app.getAssets().openXmlResourceParser("AndroidManifest.xml")) {
            for (int ev = p.next(); ev != org.xmlpull.v1.XmlPullParser.END_DOCUMENT; ev = p.next()) {
                if (ev != org.xmlpull.v1.XmlPullParser.START_TAG || !"data".equals(p.getName())) {
                    continue;
                }
                String s = p.getAttributeValue("http://schemas.android.com/apk/res/android", "scheme");
                if (s != null && !s.isEmpty()) {
                    scheme = s;
                    break;
                }
            }
        } catch (Throwable t) {
            android.util.Log.w("Day", "could not read the deep-link scheme", t);
        }
        return scheme;
    }

    /** The display the primary window is on, for Rust's `monitors()`, as one line of
     *  tab-separated fields: id, name, scale, then the frame's width and height and the work
     *  area's left, top, width and height, all in dp. The work area leaves out the system
     *  bars and any display cutout. `null` before an activity exists. */
    public static String displayInfo() {
        try {
            android.app.Activity a = primaryActivity.get();
            Context c = a != null ? a : ctx;
            if (c == null) return null;
            android.view.WindowManager wm = (android.view.WindowManager)
                    c.getSystemService(Context.WINDOW_SERVICE);
            if (wm == null) return null;
            @SuppressWarnings("deprecation")
            android.view.Display display = wm.getDefaultDisplay();
            float density = c.getResources().getDisplayMetrics().density;
            int w, h, left, top, right, bottom;
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                android.view.WindowMetrics m = wm.getMaximumWindowMetrics();
                android.graphics.Rect b = m.getBounds();
                android.graphics.Insets in = m.getWindowInsets().getInsetsIgnoringVisibility(
                        android.view.WindowInsets.Type.systemBars()
                                | android.view.WindowInsets.Type.displayCutout());
                w = b.width(); h = b.height();
                left = in.left; top = in.top; right = in.right; bottom = in.bottom;
            } else {
                // Before WindowMetrics: the real size is the display's, and the application
                // size leaves out the navigation bar; the status bar comes off the top.
                android.util.DisplayMetrics real = new android.util.DisplayMetrics();
                android.util.DisplayMetrics usable = new android.util.DisplayMetrics();
                display.getRealMetrics(real);
                display.getMetrics(usable);
                w = real.widthPixels; h = real.heightPixels;
                int sb = c.getResources().getIdentifier("status_bar_height", "dimen", "android");
                left = 0;
                top = sb > 0 ? c.getResources().getDimensionPixelSize(sb) : 0;
                right = w - usable.widthPixels;
                bottom = h - usable.heightPixels;
            }
            String name = display.getName();
            return display.getDisplayId() + "\t" + (name == null ? "" : name.replace('\t', ' '))
                    + "\t" + density
                    + "\t" + (w / density) + "\t" + (h / density)
                    + "\t" + (left / density) + "\t" + (top / density)
                    + "\t" + (Math.max(0, w - left - right) / density)
                    + "\t" + (Math.max(0, h - top - bottom) / density);
        } catch (Throwable t) {
            android.util.Log.e("Day", "displayInfo failed", t);
            return null;
        }
    }

    // --- window properties (docs/windows.md) ----------------------------------------------
    // Fullscreen hides the system bars through the window's insets controller; content
    // protection is FLAG_SECURE. Both are per window: the primary activity (node 0) or a
    // secondary DayWindowActivity (its day node).

    /** The primary activity. `ctx` follows whichever activity was resumed last, so a request
     *  addressed to the primary window needs its own reference. */
    static java.lang.ref.WeakReference<android.app.Activity> primaryActivity =
            new java.lang.ref.WeakReference<android.app.Activity>(null);
    /** Windows the app asked fullscreen, by day node (0 = the primary). */
    private static final java.util.Set<Long> fullscreenWindows = new java.util.HashSet<Long>();
    /** Windows the app asked to keep out of captures, by day node (0 = the primary). */
    private static final java.util.Set<Long> secureWindows = new java.util.HashSet<Long>();
    /** The last state code reported per window, so an inset pass reports only a real change. */
    private static final java.util.Map<Long, Integer> reportedWindowState =
            new java.util.HashMap<Long, Integer>();

    /** The activity showing day window `node`, or null once it is gone. */
    static android.app.Activity windowActivity(long node) {
        if (node == 0) return primaryActivity.get();
        return DayWindowActivity.ACTIVE.get(node);
    }

    /** The day node an activity shows: its own for a secondary window, 0 for the primary. */
    static long windowNodeOf(android.app.Activity a) {
        return a instanceof DayWindowActivity ? ((DayWindowActivity) a).node : 0L;
    }

    /** A new primary activity: a fresh tree mounts in it, whose window starts normal and
     *  unprotected, so the previous activity's requests are forgotten rather than reapplied. */
    static void primaryCreated(android.app.Activity a) {
        primaryActivity = new java.lang.ref.WeakReference<android.app.Activity>(a);
        fullscreenWindows.remove(0L);
        secureWindows.remove(0L);
        reportedWindowState.remove(0L);
    }

    /** A secondary window's activity was (re)created: its day window carries on, so the
     *  properties it was given come back with it. */
    static void secondaryCreated(DayWindowActivity a) {
        if (secureWindows.contains(a.node)) applySecure(a, true);
        if (fullscreenWindows.contains(a.node)) applyBars(a);
    }

    /** A secondary window closed for good: drop what was recorded for it. */
    static void secondaryClosed(long node) {
        fullscreenWindows.remove(node);
        secureWindows.remove(node);
        reportedWindowState.remove(node);
    }

    /** Rust's `apply_window` for `WindowChange::State`: `code` is the day WindowState code.
     *  Fullscreen hides the system bars, transiently revealed by a swipe from the edge, the
     *  platform's immersive mode; normal shows them. Minimized and maximized are not states a
     *  phone or tablet app can put itself in, so those report the state the window kept. */
    public static void setWindowState(final long node, int code) {
        try {
            android.app.Activity a = windowActivity(node);
            if (a == null) return;
            if (code != 0 && code != 3) {
                // Posted, like every report: this runs inside a call from Rust.
                main.post(new Runnable() {
                    public void run() { reportWindowState(node, true); }
                });
                return;
            }
            if (code == 3) fullscreenWindows.add(node); else fullscreenWindows.remove(node);
            applyBars(a);
            // The inset pass that follows the bars moving reports the outcome. This settles the
            // request when they never move (already hidden, or a windowing mode that keeps
            // them): a window that did not become what was asked reports even unchanged.
            final int wanted = code;
            main.postDelayed(new Runnable() {
                public void run() {
                    android.app.Activity w = windowActivity(node);
                    if (w != null) reportWindowState(node, windowStateOf(w, node) != wanted);
                }
            }, 600);
        } catch (Throwable t) {
            android.util.Log.e("Day", "setWindowState failed", t);
        }
    }

    /** Rust's `apply_window` for `WindowChange::ContentProtected`: FLAG_SECURE keeps the window
     *  out of screenshots, screen recordings, casting and the recents thumbnail. */
    public static void setWindowSecure(long node, boolean on) {
        try {
            if (on) secureWindows.add(node); else secureWindows.remove(node);
            android.app.Activity a = windowActivity(node);
            if (a != null) applySecure(a, on);
        } catch (Throwable t) {
            android.util.Log.e("Day", "setWindowSecure failed", t);
        }
    }

    private static void applySecure(android.app.Activity a, boolean on) {
        if (on) {
            a.getWindow().addFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE);
        } else {
            a.getWindow().clearFlags(android.view.WindowManager.LayoutParams.FLAG_SECURE);
        }
    }

    /** An activity's root saw new window insets (its insets listener): report its window's
     *  state if the bars moved. Posted, since the listener runs inside a layout pass. */
    static void windowInsetsChanged(android.app.Activity a) {
        if (!started) return;
        final long node = windowNodeOf(a);
        main.post(new Runnable() {
            public void run() { reportWindowState(node, false); }
        });
    }

    /** What window `node` shows as: fullscreen while the app asked for it AND the status bar is
     *  actually hidden (a mode that keeps the bars leaves the window normal). */
    private static int windowStateOf(android.app.Activity a, long node) {
        if (!fullscreenWindows.contains(node)) return 0;
        androidx.core.view.WindowInsetsCompat in =
                androidx.core.view.ViewCompat.getRootWindowInsets(a.getWindow().getDecorView());
        if (in == null) return 0;
        return in.isVisible(androidx.core.view.WindowInsetsCompat.Type.statusBars()) ? 0 : 3;
    }

    /** Report window `node`'s state to day: on a change, or every time with `always` (a request
     *  the platform declined still reports, so the app's state signal settles on the truth). */
    static void reportWindowState(long node, boolean always) {
        if (!started) return;
        android.app.Activity a = windowActivity(node);
        if (a == null) return;
        int code = windowStateOf(a, node);
        Integer last = reportedWindowState.put(node, code);
        int previous = last == null ? 0 : last;
        if (always || previous != code) {
            nativeOnEvent(node, K_WINDOW_STATE_CHANGED, code, null);
        }
    }

    /** Deferred system gestures (docs/cover.md): while any `defers_system_gestures` subtree
     *  is mounted, enter swipe-to-reveal immersive mode, the platform's "first swipe shows
     *  the bars, second swipe acts" behavior, the closest analogue of iOS's screen-edge
     *  deferral. Restores normal bars when the last request unmounts. */
    public static void setDeferSystemGestures(boolean on) {
        deferSystemGestures = on;
        applySystemBars();
    }

    /** The status bar (docs/cover.md): hidden while any mounted `status_bar_hidden` subtree
     *  asks for it. A swipe from the top edge shows it transiently, as everywhere else on the
     *  platform; the navigation bar stays. */
    public static void setStatusBarHidden(boolean hidden) {
        statusBarHidden = hidden;
        applySystemBars();
    }

    private static boolean deferSystemGestures;
    private static boolean statusBarHidden;

    private static void applySystemBars() {
        applyBars((android.app.Activity) ctx);
    }

    /** One insets-controller state for every request, so none undoes another: deferral and a
     *  fullscreen window (docs/windows.md) hide every system bar, the status-bar request only
     *  the status bar. */
    private static void applyBars(android.app.Activity act) {
        boolean fullscreen = fullscreenWindows.contains(windowNodeOf(act));
        androidx.core.view.WindowInsetsControllerCompat c =
                androidx.core.view.WindowCompat.getInsetsController(
                        act.getWindow(), act.getWindow().getDecorView());
        int bars = androidx.core.view.WindowInsetsCompat.Type.systemBars();
        int status = androidx.core.view.WindowInsetsCompat.Type.statusBars();
        if (deferSystemGestures || statusBarHidden || fullscreen) {
            c.setSystemBarsBehavior(androidx.core.view.WindowInsetsControllerCompat
                    .BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
        } else {
            c.setSystemBarsBehavior(androidx.core.view.WindowInsetsControllerCompat
                    .BEHAVIOR_DEFAULT);
        }
        if (deferSystemGestures || fullscreen) {
            c.hide(bars);
        } else if (statusBarHidden) {
            c.show(androidx.core.view.WindowInsetsCompat.Type.navigationBars());
            c.hide(status);
        } else {
            c.show(bars);
        }
    }

    // --- navigation suite (docs/navigation.md) ---
    // The nav host in its `Tabs` presentation: the same container, reached from the nav path.

    /** Pages whose rows are the chrome: marked before insertion, consumed by `addChild`. */
    private static final java.util.Set<View> navChromePages =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<View, Boolean>());

    public static View makeNavSuite(long id, int initial) { return new DayTabs(ctx, id, initial); }

    /** Mark `page` as the suite's chrome source before it is inserted (see `addChild`). */
    public static void markNavChromePage(View page) { navChromePages.add(page); }

    /**
     * Hand a nav menu's rows to the suite that will draw them as chrome.
     *
     * Called with the menu, not the suite: the menu knows its rows, and by the time it is inserted
     * its ancestors reach the suite, so the walk up is what connects the two. A menu outside a
     * suite (every non-tabs presentation) finds nothing and this is a no-op.
     */
    public static void setNavSuiteRows(View menu, String titles, String icons, String badges,
            long menuNode) {
        DayTabs suite = suiteOf(menu);
        if (suite != null) suite.setRows(titles, icons, badges, menuNode);
    }

    /** The navigation suite a nav menu's rows are drawn by, when it sits in one. */
    private static DayTabs suiteOf(View menu) {
        for (android.view.ViewParent p = menu.getParent(); p != null; p = p.getParent()) {
            if (p instanceof DayTabs) return (DayTabs) p;
        }
        return null;
    }

    /**
     * New titles and icons for the listed rows of the same row set (NavMenuPatch::Items
     * `rows_changed`), on the menu items already shown and on the suite drawing them: a count or
     * a language change updates rows in place, the way every Material component expects to be
     * told, instead of clearing and refilling the menu. Tints and badges follow through their
     * own calls, which already work in place.
     */
    public static void updateNavMenuRows(View v, String joinedRows, String joinedItems,
            String joinedIcons) {
        try {
            if (!(v instanceof NavigationView)) return;
            String[] rowStrs = joinedRows.isEmpty() ? new String[0] : joinedRows.split(",");
            int[] rows = new int[rowStrs.length];
            for (int i = 0; i < rows.length; i++) rows[i] = Integer.parseInt(rowStrs[i]);
            String[] items = joinedItems.isEmpty() ? new String[0] : joinedItems.split("\u001f", -1);
            String[] icons = joinedIcons.isEmpty() ? new String[0] : joinedIcons.split("\u001f", -1);
            java.util.List<MenuItem> menu = navItems((NavigationView) v);
            for (int i : rows) {
                if (i < 0 || i >= menu.size()) continue;
                MenuItem row = menu.get(i);
                if (i < items.length) row.setTitle(items[i]);
                if (i < icons.length) {
                    android.graphics.drawable.Drawable icon = icons[i].isEmpty() ? null
                            : drawableByName(ctx, icons[i]);
                    row.setIcon(icon);
                }
            }
            DayTabs suite = suiteOf(v);
            if (suite != null) suite.updateRows(rows, joinedItems);
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav menu row update skipped", t);
        }
    }

    public static void reorderNavSuite(View suite, String order) {
        if (suite instanceof DayTabs)
            ((DayTabs) suite).reorder(order);
    }
    public static void documentNavChrome(View suite, boolean visible) {
        if (suite instanceof DayTabs)
            ((DayTabs) suite).documentChrome(visible);
    }
    public static void documentNavSuite(
            View suite, String newLabel, String closeLabel, boolean canAdd) {
        if (suite instanceof DayTabs)
            ((DayTabs) suite).documents(newLabel, closeLabel, canAdd);
    }
    public static void setDocumentTabKeys(View menu, String keys) {
        for (android.view.ViewParent p = menu.getParent(); p != null; p = p.getParent()) {
            if (p instanceof DayTabs) {
                ((DayTabs) p).documentKeys(keys);
                return;
            }
        }
    }
    public static void setNavSuiteSelected(View suite, int index) {
        if (suite instanceof DayTabs) ((DayTabs) suite).select(index);
    }
    /** The navigation surface for `nav(NavStyle::Sidebar)` (docs/navigation.md): a
     *  Material {@link NavigationView}, the class Android means for exactly this.
     *
     *  It used to be a hand-built `LinearLayout` of `TextView` rows in a `ScrollView`, with the
     *  48dp height, the padding, the ripple and the 24dp leading glyph all measured out here; a
     *  comment in it called that "the Material nav-drawer idiom", which it was imitating rather
     *  than using. NavigationView is that idiom: M3 row metrics, the ripple, the icon slot, the
     *  subheaders between groups, a RecyclerView underneath so a long sidebar recycles, and the
     *  fully-rounded active indicator behind the checked row, the thing the hand-built list had
     *  no way to draw, which is why the showcase's sidebar marked nothing at all.
     *
     *  Rows are index-aligned with day's model throughout: a menu item's id is its index, which
     *  is what `setNavMenuSelected` and the tint/badge follow-ups address them by. `sections`
     *  opens a new group before the row at the same index ("" continues the current one), so the
     *  indices never shift when a heading is added.
     */
    private static final class DayRefreshNavigationView extends NavigationView {
        androidx.swiperefreshlayout.widget.SwipeRefreshLayout refresh;
        ViewGroup menuList;
        DayRefreshNavigationView(android.content.Context context) { super(context); }
    }

    /** Lazily add the native Android pull affordance; selection and long-press still go to
     *  Material's original RecyclerView. The widget and listener die with the navigation view. */
    public static void setNavRefreshing(View view, boolean busy) {
        if (!(view instanceof DayRefreshNavigationView)) return;
        DayRefreshNavigationView nav = (DayRefreshNavigationView) view;
        if (nav.refresh == null) {
            ViewGroup rows = navRecycler(nav);
            if (rows == null) return;
            nav.menuList = rows;
            ViewGroup.LayoutParams layout = rows.getLayoutParams();
            int index = nav.indexOfChild(rows);
            nav.removeView(rows);
            nav.refresh = new androidx.swiperefreshlayout.widget.SwipeRefreshLayout(nav.getContext());
            nav.refresh.addView(rows, new ViewGroup.LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
            nav.refresh.setOnChildScrollUpCallback((parent, child) -> rows.canScrollVertically(-1));
            nav.refresh.setOnRefreshListener(() -> nativeOnEvent(((Number) nav.getTag()).longValue(), K_REFRESH_REQUESTED, 0, null));
            nav.addView(nav.refresh, index, layout);
        }
        nav.refresh.setRefreshing(busy);
    }

    public static View makeNavMenu(final long id, String joinedItems, String joinedIcons,
            String joinedSections) {
        NavigationView nav = new DayRefreshNavigationView(ctx);
        // The pane's own background is the window's; the drawer's elevated surface would read as
        // a second card floating inside the split (docs/navigation.md).
        nav.setBackgroundColor(android.graphics.Color.TRANSPARENT);
        // Day tints each glyph itself (setNavMenuTints), so the view must not re-tint them all to
        // one color afterwards.
        nav.setItemIconTintList(null);
        nav.setTag(id);
        // Immersive mode (docs/layout.md): the page runs under the transparent status bar and the
        // floating app bar, so start the rows below them. `clipToPadding=false` keeps the padding
        // scrollable rather than fixed.
        if (DayActivity.edgeToEdge) {
            android.util.TypedValue abs = new android.util.TypedValue();
            int bar = 0;
            if (ctx.getTheme().resolveAttribute(android.R.attr.actionBarSize, abs, true)) {
                bar = android.util.TypedValue.complexToDimensionPixelSize(
                        abs.data, ctx.getResources().getDisplayMetrics());
            }
            nav.setPadding(0, DayActivity.statusInsetPx + bar, 0, 0);
            nav.setClipToPadding(false);
        }
        nav.setNavigationItemSelectedListener(new NavigationView.OnNavigationItemSelectedListener() {
            @Override public boolean onNavigationItemSelected(MenuItem item) {
                // The index is the id. Report and let day answer with NavMenuPatch::Selected,
                // which is what moves the indicator, the same rule the other backends follow, so
                // a route the app declines leaves the sidebar showing where it actually is.
                nativeOnEvent(id, K_SELECTION_CHANGED, item.getItemId() - ROW_ID_BASE, null);
                return true;
            }
        });
        // Group headings are Material's own subheaders (fillNavMenu), one per group, scrolling
        // with their rows the way every Material list does. A painted floating copy used to
        // ride on top of them, and read as exactly that: a heading drawn over the list, doubled
        // against the real one as a group scrolled under it.
        fillNavMenu(nav, joinedItems, joinedIcons, joinedSections);
        return nav;
    }

    /** The item set changed (day's NavMenuPatch::Items, a data-driven `nav().items(…)`
     *  block re-derived): rebuild the menu so every item's id carries its current index.
     *  Reusing stale rows after a removal shifts every later selection by one. */
    public static void updateNavMenu(View v, String joinedItems, String joinedIcons,
            String joinedSections) {
        if (!(v instanceof NavigationView)) return;
        fillNavMenu((NavigationView) v, joinedItems, joinedIcons, joinedSections);
        setNavSuiteRows(v, joinedItems, joinedIcons, null, 0);
    }

    /** Where a sidebar row's menu-item id starts.
     *
     *  A menu id is not a view id, except that `NavigationMenuItemView` copies its item's id onto
     *  Itself, which drops both into the one namespace `findViewById` searches. Day's own
     *  containers take `View.generateViewId()`, which counts up from 1, so a row keyed by its bare
     *  index collided with them immediately: `containerId` was 1, the Controls row (index 1) became
     *  a view with id 1, it sits earlier in the traversal than the detail container, and
     *  `FragmentTransaction.replace(containerId, …)` therefore built every detail page inside that
     *  sidebar row: the detail pane stayed empty and the page's first line drew inside the
     *  selected row's indicator.
     *
     *  `generateViewId` never returns above 0x00FFFFFF and aapt's ids start at 0x7F000000, so this
     *  range belongs to nobody. Section headings get their own so they cannot answer `findItem` for
     *  a row (`Menu.NONE` is 0, which shadowed row 0). */
    private static final int ROW_ID_BASE = 0x0100_0000;
    private static final int SECTION_ID_BASE = 0x0200_0000;

    /** Build one checkable menu item per row, grouped by `sections`. */
    private static void fillNavMenu(NavigationView nav, String joinedItems, String joinedIcons,
            String joinedSections) {
        Menu menu = nav.getMenu();
        // What is checked now, so a rebuild comes back marking the same row. A data-driven
        // `.items(…)` block re-derives on its own schedule (the showcase's does it once as the
        // sidebar first paints), and `menu.clear()` takes the indicator with it. day re-applies
        // the selection it knows about afterwards, but only if the patch that rebuilt the menu
        // carried one; this covers the rebuild that does not.
        int checked = -1;
        for (MenuItem prev : navItems(nav)) {
            if (prev.isChecked()) {
                checked = prev.getItemId() - ROW_ID_BASE;
                break;
            }
        }
        menu.clear();
        String[] items = joinedItems.isEmpty() ? new String[0] : joinedItems.split("\u001f");
        // KeepEmptyParts (limit -1) so the parallel arrays stay index-aligned with rows that
        // carry no icon and rows that open no section.
        String[] icons = joinedIcons.isEmpty() ? new String[0] : joinedIcons.split("\u001f", -1);
        String[] sections =
                joinedSections.isEmpty() ? new String[0] : joinedSections.split("\u001f", -1);
        float d = ctx.getResources().getDisplayMetrics().density;
        Menu into = menu;
        int group = 0;
        for (int i = 0; i < items.length; i++) {
            if (i < sections.length && !sections[i].isEmpty()) {
                // A heading opens a new group. NavigationView draws a SubMenu's title as an M3
                // subheader and separates the groups, which is what the flat list had no room for.
                group++;
                // The heading's own id must stay out of the row-index space: a row's id is its
                // index, `Menu.NONE` is 0, and a heading carrying 0 made `findItem(0)` answer
                // with the subheader instead of the first row, so the indicator never appeared
                // on it. Anything past the row count works; this is unmistakable in a log.
                into = menu.addSubMenu(group, SECTION_ID_BASE + group, group, sections[i]);
            }
            MenuItem it = into.add(group, ROW_ID_BASE + i, i, items[i]);
            it.setCheckable(true);
            android.graphics.drawable.Drawable icon = drawableByName(ctx, i < icons.length ? icons[i] : "");
            if (icon != null) {
                int sz = (int) (24 * d);
                icon = icon.mutate();
                icon.setBounds(0, 0, sz, sz);
                it.setIcon(icon);
            }
        }
        if (checked >= 0) setNavMenuSelected(nav, checked);
    }

    /** Move the active indicator to `index`, or clear it (day's `NavMenuProps::selected`).
     *
     *  Applied without re-emitting: `setCheckedItem` does not run the selection listener, so
     *  there is no echo to suppress. Best-effort like the other follow-ups. */
    public static void setNavMenuSelected(View navMenu, int index) {
        try {
            if (!(navMenu instanceof NavigationView)) return;
            NavigationView nav = (NavigationView) navMenu;
            if (index < 0) {
                // Grouped destinations live inside submenus. Clearing only the top
                // menu leaves a stale selected row after returning to the root.
                for (MenuItem item : navItems(nav)) item.setChecked(false);
                return;
            }
            MenuItem it = nav.getMenu().findItem(ROW_ID_BASE + index);
            if (it != null) nav.setCheckedItem(it);
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav menu selection skipped", t);
        }
    }

    /** Every menu item in `nav`, in day's index order; the tint and badge follow-ups address
     *  rows by index, and a grouped menu nests them inside SubMenus. */
    private static java.util.List<MenuItem> navItems(NavigationView nav) {
        java.util.ArrayList<MenuItem> out = new java.util.ArrayList<>();
        Menu m = nav.getMenu();
        for (int i = 0; i < m.size(); i++) {
            MenuItem it = m.getItem(i);
            if (it.hasSubMenu()) {
                android.view.SubMenu sub = it.getSubMenu();
                for (int j = 0; j < sub.size(); j++) out.add(sub.getItem(j));
            } else {
                out.add(it);
            }
        }
        return out;
    }

    /** Per-row nav icon tints (docs/vectors.md), index-aligned ARGB ints ("0" = untinted;
     *  the row keeps its text-color template tint). Best-effort: called after
     *  makeNavMenu/updateNavMenu so a failure here can never abort the native tree build. */
    public static void setNavMenuTints(View navMenu, String joinedTints) {
        try {
            if (!(navMenu instanceof NavigationView)) return;
            java.util.List<MenuItem> rows = navItems((NavigationView) navMenu);
            String[] tints = joinedTints.isEmpty() ? new String[0] : joinedTints.split("\u001f", -1);
            for (int i = 0; i < rows.size() && i < tints.length; i++) {
                android.graphics.drawable.Drawable icon = rows.get(i).getIcon();
                if (icon == null) continue;
                long tint;
                try { tint = Long.parseLong(tints[i].trim()); } catch (NumberFormatException e) { continue; }
                if (tint != 0) icon.setTint((int) tint);
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav menu tints skipped", t);
        }
    }

    /** Per-row trailing status glyphs (docs/navigation.md): a starred page's star. Index-aligned
     *  names ("" = none) with matching ARGB tints ("0" = keep the row's text-color template
     *  tint). The glyph goes in the compound drawable's end slot, so the row needs no new layout
     *  and the label still ellipsizes into what is left.
     *
     *  Best-effort and called after makeNavMenu/updateNavMenu, exactly like setNavMenuTints: a
     *  throw on the nav host's own build path takes the whole tree down with it, and a decoration
     *  is never worth that. */
    public static void setNavMenuBadges(View navMenu, String joinedIcons, String joinedTints, String joinedTexts) {
        try {
            // A tabs host draws this menu's rows as its own chrome (DayTabs), so the counts go
            // there too: without it the bottom bar and the rail never showed a badge.
            for (android.view.ViewParent p = navMenu.getParent(); p != null; p = p.getParent()) {
                if (p instanceof DayTabs) {
                    // Not when this IS the suite's drawer: the suite applying its own counts.
                    if (!((DayTabs) p).isChrome(navMenu)) ((DayTabs) p).setBadges(joinedTexts);
                    break;
                }
            }
            if (!(navMenu instanceof NavigationView)) return;
            java.util.List<MenuItem> rows = navItems((NavigationView) navMenu);
            String[] icons = joinedIcons.isEmpty() ? new String[0] : joinedIcons.split("\u001f", -1);
            String[] tints = joinedTints.isEmpty() ? new String[0] : joinedTints.split("\u001f", -1);
            String[] texts = joinedTexts.isEmpty() ? new String[0] : joinedTexts.split("\u001f", -1);
            float d = ctx.getResources().getDisplayMetrics().density;
            for (int i = 0; i < rows.size(); i++) {
                MenuItem row = rows.get(i);
                android.graphics.drawable.Drawable badge = i < icons.length ? drawableByName(ctx, icons[i]) : null;
                String text = i < texts.length ? texts[i] : "";
                android.widget.TextView count = new android.widget.TextView(ctx);
                count.setTextAppearance(android.R.style.TextAppearance_Material_Body2);
                count.setTextColor(MaterialColors.getColor(navMenu, com.google.android.material.R.attr.colorOnSurfaceVariant));
                count.setText(text);
                count.setGravity(android.view.Gravity.CENTER_VERTICAL | android.view.Gravity.END);
                count.setLayoutParams(new android.widget.FrameLayout.LayoutParams(
                        -2, -2, android.view.Gravity.CENTER_VERTICAL | android.view.Gravity.END));
                if (badge == null) {
                    // Never null: NavigationMenuItemView only REPLACES an action view, so
                    // clearing with null left the last count drawn (a "1" after the last item
                    // was done). An empty count draws nothing.
                    row.setActionView(count);
                    continue;
                }
                // The end slot is a menu item's action view here, which NavigationView lays out
                // opposite the label; the old compound-drawable trick had no menu to hang on.
                badge = badge.mutate();
                long tint = 0;
                if (i < tints.length) {
                    try { tint = Long.parseLong(tints[i].trim()); } catch (NumberFormatException e) { tint = 0; }
                }
                android.widget.ImageView iv = new android.widget.ImageView(ctx);
                iv.setImageDrawable(badge);
                int sz = (int) (18 * d);
                // NavigationView adds the action view to a FrameLayout that spans the row's
                // height; plain ViewGroup params convert to a FrameLayout's default gravity,
                // top|start, which parked the star at the top of the row. Center it.
                iv.setLayoutParams(new android.widget.FrameLayout.LayoutParams(sz, sz,
                        android.view.Gravity.CENTER_VERTICAL | android.view.Gravity.END));
                if (tint != 0) {
                    iv.setImageTintList(android.content.res.ColorStateList.valueOf((int) tint));
                }
                if (text.isEmpty()) {
                    row.setActionView(iv);
                } else {
                    android.widget.LinearLayout accessories = new android.widget.LinearLayout(ctx);
                    accessories.setOrientation(android.widget.LinearLayout.HORIZONTAL);
                    accessories.setGravity(android.view.Gravity.CENTER_VERTICAL);
                    accessories.addView(count);
                    android.widget.LinearLayout.LayoutParams iconParams = new android.widget.LinearLayout.LayoutParams(sz, sz);
                    iconParams.setMarginStart((int) (6 * d));
                    accessories.addView(iv, iconParams);
                    row.setActionView(accessories);
                }
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav menu badges skipped", t);
        }
    }

    // --- imperative presentation (docs/dialogs.md) ---
    static final java.util.HashMap<Long, android.app.Dialog> presents = new java.util.HashMap<>();

    /** A native alert / confirm / action sheet; onClick reports the spec button index. */
    public static void present(final long req, boolean sheet, String title, String message,
            String buttonsJoined, String rolesJoined) {
        final String[] labels = buttonsJoined.isEmpty() ? new String[0] : buttonsJoined.split("\u001f");
        MaterialAlertDialogBuilder b = new MaterialAlertDialogBuilder(ctx); // M3 dialog
        b.setTitle(title);
        if (sheet) {
            // A titled list of choices, the Android idiom for an action sheet.
            b.setItems(labels, new android.content.DialogInterface.OnClickListener() {
                @Override public void onClick(android.content.DialogInterface d, int which) {
                    presents.remove(req);
                    nativeOnEvent(req, K_PRESENT_BUTTON, (double) which, null); // 8 = present button
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
                            nativeOnEvent(req, K_PRESENT_BUTTON, (double) idx, null);
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
                nativeOnEvent(req, K_PRESENT_DISMISSED, 0.0, null); // 10 = dismissed
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
                nativeOnEvent(req, K_PRESENT_TEXT, 0.0, input.getText().toString()); // 9 = present text
            }
        });
        b.setNegativeButton(cancel, new android.content.DialogInterface.OnClickListener() {
            @Override public void onClick(android.content.DialogInterface d, int w) {
                presents.remove(req);
                nativeOnEvent(req, K_PRESENT_DISMISSED, 0.0, null);
            }
        });
        b.setOnCancelListener(new android.content.DialogInterface.OnCancelListener() {
            @Override public void onCancel(android.content.DialogInterface d) {
                presents.remove(req);
                nativeOnEvent(req, K_PRESENT_DISMISSED, 0.0, null);
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
        // after recording the answer); without this the picker stays on screen over the app.
        Integer rc = fileDayToReq.remove(req);
        if (rc != null) {
            fileReqToDay.remove(rc);
            fileSaveSrc.remove(rc);
            if (ctx instanceof android.app.Activity) {
                ((android.app.Activity) ctx).finishActivity(rc);
            }
        }
    }

    /** Open a URL in the system's default handler (browser for http(s), mail app for mailto:, ...).
     *  Backs the `link` piece. NEW_TASK is required because ctx may be the application context. */
    /** The broadcast a share chooser's app action sends back (docs/sharing.md). */
    static final String SHARE_ACTION = "dev.daybrite.day.SHARE_ACTION";
    private static boolean shareReceiverRegistered = false;

    /**
     * Present the system chooser for {@code url}. {@code actions} holds the app's own actions as
     * lines of {@code id \t drawable \t title}; on Android 14 (API 34) and later each becomes a
     * {@link android.service.chooser.ChooserAction} whose choice reaches native as a menu action.
     * Earlier versions show the system's targets only.
     */
    public static boolean share(String url, String title, String actions) {
        if (ctx == null || url == null) return false;
        try {
            android.content.Intent intent = new android.content.Intent(android.content.Intent.ACTION_SEND);
            intent.setType("text/plain");
            intent.putExtra(android.content.Intent.EXTRA_TEXT, url);
            intent.putExtra(android.content.Intent.EXTRA_SUBJECT, title);
            android.content.Intent chooser = android.content.Intent.createChooser(intent, null);
            if (android.os.Build.VERSION.SDK_INT >= 34 && actions != null && !actions.isEmpty()) {
                android.service.chooser.ChooserAction[] custom = shareActions(actions);
                if (custom.length > 0) {
                    chooser.putExtra(android.content.Intent.EXTRA_CHOOSER_CUSTOM_ACTIONS, custom);
                }
            }
            chooser.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK);
            ctx.startActivity(chooser);
            return true;
        } catch (Exception ignored) { return false; }
    }

    @android.annotation.TargetApi(34)
    private static android.service.chooser.ChooserAction[] shareActions(String spec) {
        registerShareReceiver();
        java.util.ArrayList<android.service.chooser.ChooserAction> out = new java.util.ArrayList<>();
        String pkg = ctx.getPackageName();
        for (String line : spec.split("\n")) {
            String[] f = line.split("\t", 3);
            if (f.length < 3) continue;
            long id;
            try { id = Long.parseLong(f[0]); } catch (NumberFormatException e) { continue; }
            int res = f[1].isEmpty() ? 0 : ctx.getResources().getIdentifier(f[1], "drawable", pkg);
            android.graphics.drawable.Icon icon = res != 0
                    ? android.graphics.drawable.Icon.createWithResource(ctx, res)
                    : android.graphics.drawable.Icon.createWithResource(ctx, android.R.drawable.ic_menu_add);
            // Explicit to this package, so the not-exported receiver below takes it, and keyed
            // by id so each action keeps its own PendingIntent.
            android.content.Intent back = new android.content.Intent(SHARE_ACTION)
                    .setPackage(pkg)
                    .putExtra("id", id);
            android.app.PendingIntent pending = android.app.PendingIntent.getBroadcast(
                    ctx, (int) id, back,
                    android.app.PendingIntent.FLAG_IMMUTABLE | android.app.PendingIntent.FLAG_UPDATE_CURRENT);
            out.add(new android.service.chooser.ChooserAction.Builder(icon, f[2], pending).build());
        }
        return out.toArray(new android.service.chooser.ChooserAction[0]);
    }

    @android.annotation.TargetApi(34)
    private static void registerShareReceiver() {
        if (shareReceiverRegistered) return;
        shareReceiverRegistered = true;
        android.content.BroadcastReceiver receiver = new android.content.BroadcastReceiver() {
            @Override public void onReceive(android.content.Context c, android.content.Intent i) {
                long id = i.getLongExtra("id", 0L);
                if (id != 0L && started) nativeOnEvent(id, K_MENU_ACTION, 0.0, "");
            }
        };
        // The PendingIntent is sent with this app's identity, so a receiver no other app can
        // reach still receives it.
        ctx.getApplicationContext().registerReceiver(receiver,
                new android.content.IntentFilter(SHARE_ACTION),
                android.content.Context.RECEIVER_NOT_EXPORTED);
    }

    public static void openUrl(String url) {
        if (ctx == null || url == null) return;
        try {
            android.content.Intent intent = new android.content.Intent(
                    android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url));
            intent.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK);
            ctx.startActivity(intent);
        } catch (Exception ignored) {
            // No handler for the scheme, or the URI was malformed: nothing to open.
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

    /** `multiple` lets the document UI select several files; onFileResult then reads the
     *  ClipData and reports every copy, joined by the unit separator (docs/files.md). */
    public static void presentFileOpen(final long req, String title, String filtersJoined,
            boolean multiple) {
        android.content.Intent intent =
            new android.content.Intent(android.content.Intent.ACTION_OPEN_DOCUMENT);
        intent.addCategory(android.content.Intent.CATEGORY_OPENABLE);
        intent.setType("*/*");
        String[] mimes = fileMimeTypes(filtersJoined);
        if (mimes.length > 0) intent.putExtra(android.content.Intent.EXTRA_MIME_TYPES, mimes);
        if (multiple) intent.putExtra(android.content.Intent.EXTRA_ALLOW_MULTIPLE, true);
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
            nativeOnEvent(req, K_PRESENT_DISMISSED, 0.0, null); // 10 = dismissed (no Activity to host the picker)
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
            nativeOnEvent(req, K_PRESENT_DISMISSED, 0.0, null);
        }
    }

    /** Called by DayActivity.onActivityResult for our file requests. */
    static void onFileResult(int requestCode, int resultCode, android.content.Intent data) {
        long[] slot = fileReqToDay.remove(requestCode);
        if (slot == null) return;
        long req = slot[0];
        fileDayToReq.remove(req);
        String src = fileSaveSrc.remove(requestCode);
        // A single pick arrives in getData(); a multiple-selection pick (EXTRA_ALLOW_MULTIPLE)
        // arrives as ClipData items, in the user's order. Collect whichever the document UI sent.
        final java.util.ArrayList<android.net.Uri> uris = new java.util.ArrayList<>();
        if (resultCode == android.app.Activity.RESULT_OK && data != null) {
            if (data.getData() != null) uris.add(data.getData());
            android.content.ClipData clip = data.getClipData();
            if (clip != null) {
                for (int i = 0; i < clip.getItemCount(); i++) {
                    android.net.Uri u = clip.getItemAt(i).getUri();
                    if (u != null && !uris.contains(u)) uris.add(u);
                }
            }
        }
        if (uris.isEmpty()) {
            nativeOnEvent(req, K_PRESENT_DISMISSED, 0.0, null); // dismissed
            return;
        }
        final android.content.Context context = ctx;
        new Thread(() -> {
            StringBuilder joined = new StringBuilder();
            for (android.net.Uri uri : uris) {
                java.io.File staged = null;
                String result;
                try {
                    if (src != null) {
                        copyStream(new java.io.FileInputStream(src), context.getContentResolver().openOutputStream(uri));
                        result = uri.toString();
                    } else {
                        // The copy keeps the document's own name, in a folder of its own: the
                        // name is what `FileUrl::file_name` shows, what tells a file's type, and
                        // what a web page receives for a file chosen in an upload form.
                        java.io.File folder = java.io.File.createTempFile("day-open-", "", context.getCacheDir());
                        if (!folder.delete() || !folder.mkdirs()) throw new java.io.IOException("no folder for the picked file");
                        String name = displayName(uri).replace('/', '_').replace('\\', '_').replace('\u0000', '_');
                        staged = new java.io.File(folder, name.isEmpty() || name.equals(".") || name.equals("..") ? "file" : name);
                        copyStream(context.getContentResolver().openInputStream(uri), new java.io.FileOutputStream(staged));
                        result = staged.getAbsolutePath();
                    }
                } catch (Exception e) {
                    if (staged != null) staged.delete();
                    android.util.Log.w("Day", "file open/save transfer failed", e);
                    result = uri.toString();
                }
                if (joined.length() > 0) joined.append('\u001f');
                joined.append(result);
            }
            final String locators = joined.toString();
            new android.os.Handler(android.os.Looper.getMainLooper()).post(() -> nativeOnEvent(req, K_PRESENT_FILE, 0.0, locators));
        }, "day-file-picker").start();
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
        for (String f : filtersJoined.split("\u001f")) {
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
    /** Height-for-width (§7.2): AT_MOST width probe, never exactly (child-chooses). */
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

    /** First text baseline from the view's top, in px, for a view laid out at `wPx` x `hPx`
     *  (docs/baseline.md). `View.getBaseline()` is the platform's answer: TextView and its
     *  subclasses (so EditText, MaterialButton, the pickers) override it, and the base View
     *  returns -1 for "no baseline", which is exactly the distinction day wants.
     *
     *  Measured at the size day settled on first: a TextView's baseline moves with its height
     *  whenever gravity centers its text in a taller box. */
    public static int baselineAt(View v, int wPx, int hPx) {
        v.measure(View.MeasureSpec.makeMeasureSpec(wPx, View.MeasureSpec.EXACTLY),
                  View.MeasureSpec.makeMeasureSpec(hPx, View.MeasureSpec.EXACTLY));
        return v.getBaseline();
    }

    public static void setEnabled(View v, boolean b) {
        if (!b) DayKeyboard.endEditing(v);
        v.setEnabled(b);
    }

    public static View makeCanvas(long id) { return new DayCanvasView(ctx, id); }
    public static void setCanvasOps(View v, double[] nums, String textsJoined, double[] geometry) {
        ((DayCanvasView) v).setOps(nums, textsJoined, geometry);
    }
    // ---- Raster images from bytes (docs/images.md) -----------------------------------------

    /** Decoded bitmaps, keyed by the id day-core minted. A bitmap outlives any one view and may
     *  be drawn by several at once, so this is keyed by id rather than by View. Entries leave
     *  only through {@link #imageRelease}, which day-core calls when the app drops its last
     *  handle. Concurrent because the offscreen painter reads and fills it from a painting
     *  thread (docs/canvas.md "Exporting"); the bitmaps themselves are never mutated once
     *  stored, so drawing one from two threads at once is safe. */
    static final java.util.concurrent.ConcurrentHashMap<Long, android.graphics.Bitmap> bitmaps =
            new java.util.concurrent.ConcurrentHashMap<>();

    /** The bitmap `id` names, or null once released: what DayCanvasView's image op draws. */
    public static android.graphics.Bitmap bitmapFor(long id) {
        return bitmaps.get(id);
    }

    /** Decode `bytes` into the registry under `id`. Answers "w,h,alpha" (alpha 1/0), the same
     *  comma-joined reply shape `measureText` uses, or null when the bytes are not an image
     *  this platform decodes. */
    public static String imageDecode(long id, byte[] bytes) {
        // A null array would NPE inside decodeByteArray, and an exception thrown back across
        // JNI aborts the process rather than failing the call.
        if (bytes == null) {
            return null;
        }
        android.graphics.Bitmap bmp =
                android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.length);
        if (bmp == null) {
            return null;
        }
        bitmaps.put(id, bmp);
        // `hasAlpha` is a real query here, unlike the backends that must infer it from the
        // container format.
        return bmp.getWidth() + "," + bmp.getHeight() + "," + (bmp.hasAlpha() ? 1 : 0);
    }

    /** Re-encode a decoded bitmap as "png" or "jpeg", optionally scaled to fit `fitW`×`fitH`
     *  (0 = no fit), at `quality` 0..100 (-1 = the format's default). Null when the id names
     *  nothing or the writer fails. */
    public static byte[] imageEncode(long id, String format, int quality, double fitW, double fitH) {
        android.graphics.Bitmap bmp = bitmaps.get(id);
        if (bmp == null) {
            return null;
        }
        // `fit` scales the longest side down first. A box LARGER than the original is ignored:
        // upscaling on an export path inflates the bytes without adding any detail.
        if (fitW > 0 && fitH > 0 && bmp.getWidth() > 0 && bmp.getHeight() > 0) {
            double s = Math.min(Math.min(fitW / bmp.getWidth(), fitH / bmp.getHeight()), 1.0);
            if (s < 1.0) {
                bmp = android.graphics.Bitmap.createScaledBitmap(
                        bmp,
                        Math.max(1, (int) Math.round(bmp.getWidth() * s)),
                        Math.max(1, (int) Math.round(bmp.getHeight() * s)),
                        true);
            }
        }
        android.graphics.Bitmap.CompressFormat fmt = "jpeg".equals(format)
                ? android.graphics.Bitmap.CompressFormat.JPEG
                : android.graphics.Bitmap.CompressFormat.PNG;
        java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
        if (!bmp.compress(fmt, quality < 0 ? 90 : quality, out)) {
            return null;
        }
        return out.toByteArray();
    }

    /** Drop a decoded bitmap; day-core calls this when the app's last handle goes.
     *
     *  Not `recycle()`: a view may still be showing this bitmap, and drawing a recycled one
     *  is a hard crash ("trying to use a recycled bitmap"). Dropping the last reference is
     *  enough; the GC frees the pixels once nothing draws them. */
    public static void imageRelease(long id) {
        bitmaps.remove(id);
    }

    /** An image view showing a DECODED bitmap rather than a staged drawable. */
    public static View makeImageBitmap(long id, int mode) {
        android.widget.ImageView iv = imageViewWithMode(mode);
        android.graphics.Bitmap bmp = bitmaps.get(id);
        if (bmp != null) {
            iv.setImageBitmap(bmp);
        }
        return iv;
    }

    /** An image view showing raw encoded bytes the app already holds, with no staged resource
     *  behind them. */
    public static View makeImageBytes(byte[] bytes, int mode) {
        android.widget.ImageView iv = imageViewWithMode(mode);
        // An empty view rather than an NPE: allocating the array can fail on the Rust side, and
        // an exception crossing back through JNI aborts the process.
        if (bytes == null) {
            return iv;
        }
        android.graphics.Bitmap bmp =
                android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.length);
        if (bmp != null) {
            iv.setImageBitmap(bmp);
        }
        return iv;
    }

    /** Point a realized image view at a different staged drawable (an `ImageSource::Named`
     *  swap). Goes through `drawableByName` so the weight-alias fallback still applies, and
     *  `.mutate()` so a later tint cannot follow every other view showing the same glyph. */
    public static void setImageName(View v, String name) {
        if (!(v instanceof android.widget.ImageView)) {
            return;
        }
        android.graphics.drawable.Drawable d = drawableByName(ctx, name);
        if (d != null || name == null || name.isEmpty()) {
            // The default/empty source explicitly clears a recycled image. An unresolved
            // nonempty asset still preserves its predecessor (ImagePatch's failure policy).
            ((android.widget.ImageView) v).setImageDrawable(d == null ? null : d.mutate());
        }
    }

    /** Point a realized image view at a decoded bitmap (an `ImageSource::Decoded` swap). */
    public static void setImageBitmap(View v, long id) {
        if (!(v instanceof android.widget.ImageView)) {
            return;
        }
        android.graphics.Bitmap bmp = bitmaps.get(id);
        if (bmp != null) {
            ((android.widget.ImageView) v).setImageBitmap(bmp);
        }
    }

    /** Point a realized image view at raw encoded bytes (an `ImageSource::Bytes` swap). */
    public static void setImageBytes(View v, byte[] bytes) {
        if (!(v instanceof android.widget.ImageView) || bytes == null) {
            return;
        }
        android.graphics.Bitmap bmp =
                android.graphics.BitmapFactory.decodeByteArray(bytes, 0, bytes.length);
        if (bmp != null) {
            ((android.widget.ImageView) v).setImageBitmap(bmp);
        }
    }

    /** A plain ImageView carrying day's scale-type mapping: 0=fit, 1=fill (crop), 2=stretch. */
    private static android.widget.ImageView imageViewWithMode(int mode) {
        android.widget.ImageView iv = new android.widget.ImageView(ctx);
        iv.setScaleType(
                mode == 2 ? android.widget.ImageView.ScaleType.FIT_XY
                        : mode == 1 ? android.widget.ImageView.ScaleType.CENTER_CROP
                                : android.widget.ImageView.ScaleType.FIT_CENTER);
        return iv;
    }

    /** The tint channel's "template" value (docs/vectors.md "Template"): alpha 0, which no real
     *  tint carries. The glyph takes the theme's primary text color — light on a dark surface,
     *  dark on a light one — and keeps it across tint patches that clear the tint. */
    private static final int TEMPLATE_TINT = 1;

    /** The image views realized as templates, so a cleared tint returns them to the theme color.
     *  Weak: a view leaves the set when the layout drops it. */
    private static final java.util.Set<View> templateImages =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<View, Boolean>());

    /** The theme's primary text color, what an untinted template glyph draws in. */
    private static int templateTint() {
        android.util.TypedValue tv = new android.util.TypedValue();
        if (ctx.getTheme().resolveAttribute(android.R.attr.textColorPrimary, tv, true)) {
            if (tv.resourceId != 0) return ctx.getColor(tv.resourceId);
            return tv.data;
        }
        return 0xFF000000;
    }

    /** Apply the tint channel to an image view: a packed ARGB, {@link #TEMPLATE_TINT} for the
     *  theme text color, or 0 for the authored colors — unless the view was realized as a
     *  template, which 0 returns to the theme color rather than to the art. */
    private static void applyImageTint(android.widget.ImageView iv, int tint) {
        if (tint == 0 && templateImages.contains(iv)) {
            tint = TEMPLATE_TINT;
        }
        int color = tint == TEMPLATE_TINT ? templateTint() : tint;
        iv.setImageTintList(
                tint == 0 ? null : android.content.res.ColorStateList.valueOf(color));
    }

    /** `ImagePatch::Tint`: repaint a realized glyph. 0 restores the authored colors (the theme
     *  text color on a template glyph). */
    public static void setImageTint(View v, int tint) {
        if (!(v instanceof android.widget.ImageView)) {
            return;
        }
        applyImageTint((android.widget.ImageView) v, tint);
    }

    /** The tint a raster image is realized with (docs/images.md "Tinting"): the same channel
     *  as {@link #makeImage}'s, for a view made from bytes or a decoded bitmap. The tint list
     *  belongs to the view, so a later source swap draws the new pixels in it too. */
    public static void initImageTint(View v, int tint) {
        if (!(v instanceof android.widget.ImageView) || tint == 0) {
            return;
        }
        if (tint == TEMPLATE_TINT) templateImages.add(v);
        applyImageTint((android.widget.ImageView) v, tint);
    }

    public static View makeImage(String name, int mode, int tint) {
        View v = makeImageInner(name, mode);
        // Vector-glyph tint (docs/vectors.md): drawable tint keeps a VectorDrawable/PNG's alpha
        // as the mask. 0 = untinted (a real tint always carries alpha 0xFF); TEMPLATE_TINT =
        // the theme text color, remembered so a cleared tint lands back on it.
        if (tint != 0 && v instanceof android.widget.ImageView) {
            android.widget.ImageView iv = (android.widget.ImageView) v;
            if (tint == TEMPLATE_TINT) templateImages.add(iv);
            applyImageTint(iv, tint);
        }
        return v;
    }

    private static View makeImageInner(String name, int mode) {
        android.widget.ImageView iv = new android.widget.ImageView(ctx);
        // Scaling (§18.3): 0=fit, 1=fill (crop), 2=stretch.
        iv.setScaleType(
                mode == 2 ? android.widget.ImageView.ScaleType.FIT_XY
                        : mode == 1 ? android.widget.ImageView.ScaleType.CENTER_CROP
                                : android.widget.ImageView.ScaleType.FIT_CENTER);
        // Resolution goes through drawableByName, which is also where the weight-alias fallback
        // lives (docs/vectors.md): a plain SVG stages one asset, so `<glyph>__light`/`__bold` must
        // land back on `<glyph>` rather than draw nothing. This path used to do its own
        // getIdentifier + assets lookup and skip that fallback, which is why every
        // `vector(plain).weight(Light|Bold)` was blank on Android while it resolved everywhere
        // else, and invisible to `assert_visible`, since the ImageView had a frame either way.
        //
        // `.mutate()` because the drawable comes from the shared resource cache and the caller
        // tints it: without it a tint would follow every other view showing the same glyph.
        android.graphics.drawable.Drawable d = drawableByName(ctx, name);
        if (d == null) {
            android.util.Log.w("Day", "no drawable or asset resolved for image " + name);
            return iv;
        }
        iv.setImageDrawable(d.mutate());
        return iv;
    }
    /** Resolve bundled artwork or a downloaded local image. Absolute paths and local file:
     *  URIs are files, never AssetManager names. All image channels (views, navigation rows,
     *  badges and toolbars) share this resolver and preserve authored colors unless tinted. */
    static android.graphics.drawable.Drawable drawableByName(Context c, String name) {
        if (name == null || name.isEmpty()) return null;
        String filePath = null;
        if (new java.io.File(name).isAbsolute()) {
            filePath = name;
        } else if (name.startsWith("file:")) {
            android.net.Uri uri = android.net.Uri.parse(name);
            String authority = uri.getAuthority();
            if (authority != null && !authority.isEmpty() && !"localhost".equals(authority)) return null;
            filePath = uri.getPath();
            if (filePath == null || !new java.io.File(filePath).isAbsolute()) return null;
        }
        if (filePath != null) {
            android.graphics.Bitmap bitmap = android.graphics.BitmapFactory.decodeFile(filePath);
            return bitmap == null ? null : new android.graphics.drawable.BitmapDrawable(c.getResources(), bitmap);
        }
        int id = c.getResources().getIdentifier(name, "drawable", c.getPackageName());
        if (id != 0) return c.getResources().getDrawable(id, c.getTheme());
        // A weight variant with no art of its own resolves to the base glyph (docs/vectors.md):
        // only SF-template sources stage `__light`/`__bold`, so a plain SVG's weight names land
        // here and must fall back rather than draw nothing.
        for (String suffix : new String[] {"__light", "__bold"}) {
            if (name.endsWith(suffix)) {
                String base = name.substring(0, name.length() - suffix.length());
                if (!base.isEmpty()) return drawableByName(c, base);
            }
        }
        try (java.io.InputStream stream = c.getAssets().open(name)) {
            android.graphics.Bitmap bm =
                    android.graphics.BitmapFactory.decodeStream(stream);
            if (bm != null) return new android.graphics.drawable.BitmapDrawable(c.getResources(), bm);
        } catch (Exception e) {
            // fall through to null
        }
        return null;
    }

    // --- Accessibility (docs/accessibility.md) --------------------------------
    // What the View itself carries goes straight on it: contentDescription = label,
    // importantForAccessibility hides a decorative element with its subtree, stateDescription =
    // value (API 30+). Hint and role live only on the AccessibilityNodeInfo TalkBack builds, so
    // they come from an AccessibilityDelegate reading the traits stored per view below.

    /** Day's `Role`, by declaration order (day_spec::Role): 0 None 1 Button 2 Toggle 3 Slider
     *  4 TextInput 5 Heading 6 Image 7 Meter 8 Group 9 Tree 10 TreeItem 11 Tab 12 TabList.
     *  `set_a11y` sends it as the int, with the heading level beside it. */
    static final int ROLE_NONE = 0, ROLE_BUTTON = 1, ROLE_TOGGLE = 2, ROLE_SLIDER = 3,
            ROLE_TEXT_INPUT = 4, ROLE_HEADING = 5, ROLE_IMAGE = 6, ROLE_METER = 7, ROLE_GROUP = 8,
            ROLE_TAB = 11, ROLE_TAB_LIST = 12;

    /** The delegate-borne traits of one view: the hint, the explicit role, and the Day id. */
    private static final class A11yTraits {
        String hint = "";
        int role = ROLE_NONE;
        String id = "";
    }
    /** Per-view traits, read by `A11yDelegate` each time TalkBack builds the node. */
    private static final java.util.WeakHashMap<View, A11yTraits> a11yTraits =
            new java.util.WeakHashMap<>();

    /** The one delegate `setA11y` installs. It wraps whatever delegate the widget already had
     *  (Material's Slider and TextInputLayout, the RecyclerView list) and forwards every call to
     *  it, so the widget's own node tree, actions and events stay intact and Day only adds its
     *  traits on top. A view takes exactly one of these: repeated calls find it in place. */
    private static final class A11yDelegate extends androidx.core.view.AccessibilityDelegateCompat {
        /** The widget's own delegate, or null for a plain view. */
        private final androidx.core.view.AccessibilityDelegateCompat inner;

        A11yDelegate(androidx.core.view.AccessibilityDelegateCompat inner) {
            this.inner = inner;
        }

        @Override public void onInitializeAccessibilityNodeInfo(View host,
                androidx.core.view.accessibility.AccessibilityNodeInfoCompat info) {
            if (inner != null) inner.onInitializeAccessibilityNodeInfo(host, info);
            else super.onInitializeAccessibilityNodeInfo(host, info);
            A11yTraits t = a11yTraits.get(host);
            if (t == null) return;
            // hintText is TalkBack's "usage hint", read after the label; the visible tooltip
            // twin is left alone because its long-press trigger would collide with Day's
            // long-press context menu (docs/menus.md).
            if (!t.hint.isEmpty()) info.setHintText(t.hint);
            // The automation id (§13): `uniqueId` is what UiAutomator and Appium address a
            // node by. The platform carries it from API 33; the compat call stores it in the
            // node's extras below that, where only an in-process reader finds it.
            if (!t.id.isEmpty()) info.setUniqueId(t.id);
            // TalkBack names the control type from the node's class name, so a canvas gauge
            // reads as a progress bar and a drawn button as a button. Only an explicit role
            // is applied: a native control's own class already says what it is.
            switch (t.role) {
                case ROLE_BUTTON: info.setClassName(android.widget.Button.class.getName()); break;
                case ROLE_TOGGLE:
                    info.setClassName(android.widget.Switch.class.getName());
                    info.setCheckable(true);
                    break;
                case ROLE_SLIDER: info.setClassName(android.widget.SeekBar.class.getName()); break;
                case ROLE_TEXT_INPUT: info.setClassName(EditText.class.getName()); break;
                case ROLE_HEADING: info.setHeading(true); break;
                case ROLE_IMAGE: info.setClassName(android.widget.ImageView.class.getName()); break;
                case ROLE_METER: info.setClassName(ProgressBar.class.getName()); break;
                case ROLE_GROUP: info.setClassName(ViewGroup.class.getName()); break;
                // A tab reads as a button; its selected state (setA11y's `selected`, the view's
                // own) is what tells the current one apart. The strip is a plain group.
                case ROLE_TAB: info.setClassName(android.widget.Button.class.getName()); break;
                case ROLE_TAB_LIST: info.setClassName(ViewGroup.class.getName()); break;
                default: break; // None, Tree, TreeItem: the platform has no node type for them
            }
        }

        // Everything else is the wrapped widget's business.
        @Override public void sendAccessibilityEvent(View host, int eventType) {
            if (inner != null) inner.sendAccessibilityEvent(host, eventType);
            else super.sendAccessibilityEvent(host, eventType);
        }
        @Override public void sendAccessibilityEventUnchecked(View host,
                android.view.accessibility.AccessibilityEvent event) {
            if (inner != null) inner.sendAccessibilityEventUnchecked(host, event);
            else super.sendAccessibilityEventUnchecked(host, event);
        }
        @Override public boolean dispatchPopulateAccessibilityEvent(View host,
                android.view.accessibility.AccessibilityEvent event) {
            return inner != null
                    ? inner.dispatchPopulateAccessibilityEvent(host, event)
                    : super.dispatchPopulateAccessibilityEvent(host, event);
        }
        @Override public void onPopulateAccessibilityEvent(View host,
                android.view.accessibility.AccessibilityEvent event) {
            if (inner != null) inner.onPopulateAccessibilityEvent(host, event);
            else super.onPopulateAccessibilityEvent(host, event);
        }
        @Override public void onInitializeAccessibilityEvent(View host,
                android.view.accessibility.AccessibilityEvent event) {
            if (inner != null) inner.onInitializeAccessibilityEvent(host, event);
            else super.onInitializeAccessibilityEvent(host, event);
        }
        @Override public boolean onRequestSendAccessibilityEvent(ViewGroup host, View child,
                android.view.accessibility.AccessibilityEvent event) {
            return inner != null
                    ? inner.onRequestSendAccessibilityEvent(host, child, event)
                    : super.onRequestSendAccessibilityEvent(host, child, event);
        }
        @Override public androidx.core.view.accessibility.AccessibilityNodeProviderCompat
                getAccessibilityNodeProvider(View host) {
            return inner != null
                    ? inner.getAccessibilityNodeProvider(host)
                    : super.getAccessibilityNodeProvider(host);
        }
        @Override public boolean performAccessibilityAction(View host, int action,
                android.os.Bundle args) {
            return inner != null
                    ? inner.performAccessibilityAction(host, action, args)
                    : super.performAccessibilityAction(host, action, args);
        }
    }

    /** Views the app gave an accessibility label (setA11y), whose content description a later
     *  button content change must leave alone. Weak: a released view drops out with it. */
    static final java.util.Map<View, Boolean> appLabeled = new java.util.WeakHashMap<>();

    private static final java.util.Map<View, android.widget.PopupWindow> popovers = new java.util.IdentityHashMap<>();

    public static void setTooltip(View v, String text) {
        setViewTooltip(v, text.isEmpty() ? null : text);
    }

    private static final java.util.Map<View, String> tooltipHints = new java.util.WeakHashMap<>();

    private static final java.util.Set<View> compatTooltips =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<>());

    private static void clearCompatTooltip(View v) {
        if (compatTooltips.remove(v)) androidx.appcompat.widget.TooltipCompat.setTooltipText(v, null);
    }

    private static void setViewTooltip(View v, String text) {
        if (android.os.Build.VERSION.SDK_INT >= 26) {
            v.setTooltipText(text);
            return;
        }
        // TooltipCompat owns the single long-click listener on these Android releases.
        // A text editor's native selection gesture also needs that listener.
        if (text == null) {
            tooltipHints.remove(v);
            clearCompatTooltip(v);
            return;
        }
        tooltipHints.put(v, text);
        Recognizers r = recognizers.get(v);
        if (contextMenus.containsKey(v) || (r != null && r.longPress)
                || (v.isLongClickable() && !compatTooltips.contains(v))) return;
        androidx.appcompat.widget.TooltipCompat.setTooltipText(v, text);
        compatTooltips.add(v);
    }

    public static boolean openPopover(View anchor, View content, int w, int h, long id) {
        if (!anchor.isAttachedToWindow()) return false;
        android.widget.PopupWindow popup = new android.widget.PopupWindow(content, w, h, true);
        android.util.TypedValue color = new android.util.TypedValue();
        anchor.getContext().getTheme().resolveAttribute(android.R.attr.colorBackground, color, true);
        popup.setBackgroundDrawable(new android.graphics.drawable.ColorDrawable(color.data));
        popup.setOutsideTouchable(true);
        popup.setElevation(6 * anchor.getResources().getDisplayMetrics().density);
        popup.setOnDismissListener(() -> nativePopoverDismissed(id));
        popup.showAsDropDown(anchor);
        popovers.put(content, popup);
        return true;
    }

    private static native void nativePopoverDismissed(long id);

    public static void resizePopover(View content, int width, int height) {
        android.widget.PopupWindow popup = popovers.get(content);
        if (popup != null) popup.update(width, height);
    }

    public static void closePopover(View content) {
        android.widget.PopupWindow popup = popovers.remove(content);
        if (popup != null) {
            popup.setOnDismissListener(null);
            popup.dismiss();
        }
    }

    public static void showToast(View host, String text) {
        if (host.isAttachedToWindow()) {
            // FragmentActivity inflates the Snackbar's hidden action as a framework Button.
            // Supply the size overlay that MaterialButton normally applies to Expressive styles.
            Context theme = new android.view.ContextThemeWrapper(host.getContext(),
                    com.google.android.material.R.style.SizeOverlay_Material3Expressive_Button_Small);
            com.google.android.material.snackbar.Snackbar.make(theme, host, text,
                    com.google.android.material.snackbar.Snackbar.LENGTH_SHORT).show();
        }
    }

    /** Apply a node's whole accessibility set (docs/accessibility.md). Empty strings and
     *  `ROLE_NONE` mean "not set": those members leave the native default alone, and a member
     *  set once stays set, so re-sending the same values changes nothing. `level` is the heading
     *  level, informational here: Android's node has heading yes/no and no level. */
    public static void setA11y(View v, String label, String hint, String value, String id,
            int role, int level, boolean hidden, int selected) {
        try {
            // The selected state (-1 unset): the view's own, which TalkBack reads as "selected".
            if (selected >= 0) v.setSelected(selected == 1);
            boolean hasLabel = label != null && !label.isEmpty();
            if (hasLabel) appLabeled.put(v, Boolean.TRUE); else appLabeled.remove(v);
            boolean hasValue = value != null && !value.isEmpty();
            if (hasValue && android.os.Build.VERSION.SDK_INT >= 30) {
                v.setStateDescription(value);
                if (hasLabel) v.setContentDescription(label);
            } else if (hasValue) {
                // No stateDescription before API 30: the value rides on the description, after
                // the label, so a custom control's reading still carries it.
                v.setContentDescription(hasLabel ? label + ", " + value : value);
            } else if (hasLabel) {
                v.setContentDescription(label);
            }
            if (hidden) {
                v.setImportantForAccessibility(View.IMPORTANT_FOR_ACCESSIBILITY_NO_HIDE_DESCENDANTS);
            }
            boolean hasHint = hint != null && !hint.isEmpty();
            boolean hasId = id != null && !id.isEmpty();
            if (!hasHint && !hasId && role == ROLE_NONE) return;
            A11yTraits t = a11yTraits.get(v);
            if (t == null) {
                // First traits for this view: the delegate goes on once, wrapping the one the
                // widget already has. The traits entry is what says it is in place, since the
                // delegate cannot be read back below API 29 without reflection.
                t = new A11yTraits();
                a11yTraits.put(v, t);
                androidx.core.view.ViewCompat.setAccessibilityDelegate(v,
                        new A11yDelegate(androidx.core.view.ViewCompat.getAccessibilityDelegate(v)));
            }
            if (hasHint) t.hint = hint;
            if (hasId) t.id = id;
            if (role != ROLE_NONE) t.role = role;
        } catch (Throwable t) {
            android.util.Log.w("day", "setA11y (best-effort)", t);
        }
    }

    /** Slots of `readNative`'s answer, in the order Rust's `read_native` unpacks them. */
    static final int RN_FOUND = 0, RN_ROLE = 1, RN_LABEL = 2, RN_VALUE = 3, RN_ID = 4,
            RN_TEXT = 5, RN_NUMBER = 6, RN_CHECKED = 7, RN_ENABLED = 8, RN_VISIBLE = 9,
            RN_FRAME = 10, RN_COUNT = 11;

    /** Read a view's actual native state back for `Toolkit::read_native` (docs/testing.md), in
     *  one call. A null slot is a field this reader did not take from the view; Rust reports it
     *  as unread, never as a value. Flags are "1"/"0", numbers `Double.toString`, the frame
     *  "x,y,w,h" in dp from the Day root's top left.
     *
     *  The accessibility group comes from the node the platform itself builds for TalkBack
     *  (`createAccessibilityNodeInfo`, so `setA11y`'s delegate has run on it). It is only
     *  reported from API 30: below that `setA11y` folds the value into the content description
     *  and the label can no longer be told apart from it. */
    public static String[] readNative(View v) {
        String[] out = new String[RN_COUNT];
        if (v == null) return out;
        try {
            if (android.os.Build.VERSION.SDK_INT >= 30) {
                androidx.core.view.accessibility.AccessibilityNodeInfoCompat info =
                        androidx.core.view.accessibility.AccessibilityNodeInfoCompat.wrap(
                                v.createAccessibilityNodeInfo());
                CharSequence label = info.getContentDescription();
                CharSequence value = info.getStateDescription();
                out[RN_LABEL] = label != null ? label.toString() : null;
                out[RN_VALUE] = value != null ? value.toString() : null;
                out[RN_ID] = info.getUniqueId();
                out[RN_ROLE] = Integer.toString(roleOfNode(info));
                out[RN_FOUND] = "1";
            }
        } catch (Throwable t) {
            // A widget's own delegate threw while building the node: report the group unread.
            for (int i = RN_FOUND; i <= RN_ID; i++) out[i] = null;
            android.util.Log.w("day", "readNative a11y (best-effort)", t);
        }
        try {
            // The control itself: a text field's handle is its TextInputLayout box.
            View c = v instanceof TextInputLayout ? editTextOf(v) : v;
            if (c == null) c = v;
            if (c instanceof EditText) {
                EditText e = (EditText) c;
                if (!isMasked(e)) out[RN_TEXT] = e.getText().toString();
            } else if (c instanceof CompoundButton) {
                out[RN_CHECKED] = ((CompoundButton) c).isChecked() ? "1" : "0";
            } else if (c instanceof MaterialButton && buttonSelected.containsKey(c)) {
                // A Day button: its title, and its selected state as checked (setButtonSelected).
                MaterialButton b = (MaterialButton) c;
                out[RN_TEXT] = b.getText().toString();
                out[RN_CHECKED] = b.isCheckable() && b.isChecked() ? "1" : "0";
            } else if (c instanceof TextView) {
                // A label (runs flatten to their plain string) or a button's title.
                out[RN_TEXT] = ((TextView) c).getText().toString();
            } else if (c instanceof Slider) {
                // Material's Slider holds Day's own range (makeSlider sets valueFrom/valueTo).
                out[RN_NUMBER] = Double.toString(((Slider) c).getValue());
            } else if (c instanceof LinearProgressIndicator) {
                // makeProgress scales Day's 0..1 fraction onto 0..1000 ticks; only that exact
                // scale is inverted, and an indeterminate bar has no value to read.
                LinearProgressIndicator pb = (LinearProgressIndicator) c;
                if (!pb.isIndeterminate() && pb.getMax() == 1000) {
                    out[RN_NUMBER] = Double.toString(pb.getProgress() / 1000.0);
                }
            } else if (dev.daybrite.day.piece.picker.DayPicker.isPicker(c)) {
                // A picker reads as its selected option's label (null with nothing selected).
                out[RN_TEXT] = dev.daybrite.day.piece.picker.DayPicker.selectedText(c);
            }
            out[RN_ENABLED] = v.isEnabled() && c.isEnabled() ? "1" : "0";
            out[RN_VISIBLE] = v.isShown() ? "1" : "0";
            out[RN_FRAME] = frameInDayRoot(v);
        } catch (Throwable t) {
            android.util.Log.w("day", "readNative (best-effort)", t);
        }
        return out;
    }

    /** Day's `Role` index for what the platform node says it is: its class name, or the
     *  heading flag. `ROLE_NONE` where the class names nothing Day has a role for, a plain
     *  ViewGroup included: every container is one, so it cannot stand for `Group`. */
    private static int roleOfNode(androidx.core.view.accessibility.AccessibilityNodeInfoCompat info) {
        if (info.isHeading()) return ROLE_HEADING;
        CharSequence cls = info.getClassName();
        if (cls == null) return ROLE_NONE;
        switch (cls.toString()) {
            case "android.widget.Button": return ROLE_BUTTON;
            case "android.widget.Switch":
            case "android.widget.CheckBox":
            case "android.widget.ToggleButton":
            case "android.widget.CompoundButton": return ROLE_TOGGLE;
            case "android.widget.SeekBar": return ROLE_SLIDER;
            case "android.widget.EditText":
            case "android.widget.AutoCompleteTextView":
            case "android.widget.MultiAutoCompleteTextView": return ROLE_TEXT_INPUT;
            case "android.widget.ImageView": return ROLE_IMAGE;
            case "android.widget.ProgressBar": return ROLE_METER;
            default: return ROLE_NONE;
        }
    }

    /** Does this field mask what it holds? A password transformation, or a password input
     *  variation (the visible-password variation shows its text, so it does not count). */
    private static boolean isMasked(EditText e) {
        if (e.getTransformationMethod() instanceof android.text.method.PasswordTransformationMethod) {
            return true;
        }
        int type = e.getInputType();
        int cls = type & android.text.InputType.TYPE_MASK_CLASS;
        int variation = type & android.text.InputType.TYPE_MASK_VARIATION;
        if (cls == android.text.InputType.TYPE_CLASS_TEXT) {
            return variation == android.text.InputType.TYPE_TEXT_VARIATION_PASSWORD
                    || variation == android.text.InputType.TYPE_TEXT_VARIATION_WEB_PASSWORD;
        }
        return cls == android.text.InputType.TYPE_CLASS_NUMBER
                && variation == android.text.InputType.TYPE_NUMBER_VARIATION_PASSWORD;
    }

    /** A view's frame as "x,y,w,h" in dp from the top left of the Day root it sits in: the
     *  DayFixed that DayActivity/DayWindowActivity hold in a wrapper under android.R.id.content,
     *  which is where Day's frames start. Null when the view is not attached under one (a
     *  dialog's own window, a detached view): there is no Day origin to measure from. */
    /** The Day root `v` sits in: the DayFixed held in a wrapper under android.R.id.content.
     *  Null when there is none above it (a dialog's own window, a detached view). */
    private static View dayRootOf(View v) {
        for (View p = v; p != null; ) {
            android.view.ViewParent wp = p.getParent();
            if (p instanceof DayFixed && wp instanceof View) {
                android.view.ViewParent cp = ((View) wp).getParent();
                if (cp instanceof View && ((View) cp).getId() == android.R.id.content) {
                    return p;
                }
            }
            p = wp instanceof View ? (View) wp : null;
        }
        return null;
    }

    private static String frameInDayRoot(View v) {
        if (!v.isAttachedToWindow()) return null;
        View root = dayRootOf(v);
        if (root == null) return null;
        int[] at = new int[2];
        int[] origin = new int[2];
        v.getLocationInWindow(at);
        root.getLocationInWindow(origin);
        float d = v.getResources().getDisplayMetrics().density;
        if (d <= 0f) return null;
        return ((at[0] - origin[0]) / (double) d) + "," + ((at[1] - origin[1]) / (double) d) + ","
                + (v.getWidth() / (double) d) + "," + (v.getHeight() / (double) d);
    }

    /** Speak `text` through the running screen reader without moving its focus
     *  (docs/accessibility.md). Nothing is sent while no accessibility service is enabled.
     *  `urgent` cuts off what is being read first: the announcement event itself has no
     *  queue/interrupt flag, so `AccessibilityManager.interrupt` supplies the distinction. The
     *  deprecated `announceForAccessibility` stays the one request that is an announcement
     *  (its suggested replacements, pane titles and live regions, describe views). */
    @SuppressWarnings("deprecation")
    public static void announce(String text, boolean urgent) {
        try {
            if (text == null || text.isEmpty()) return;
            if (!(ctx instanceof android.app.Activity)) return;
            android.view.accessibility.AccessibilityManager am =
                    (android.view.accessibility.AccessibilityManager)
                            ctx.getSystemService(Context.ACCESSIBILITY_SERVICE);
            if (am == null || !am.isEnabled()) return;
            View root = ((android.app.Activity) ctx).getWindow().findViewById(android.R.id.content);
            if (root == null) return;
            if (urgent) am.interrupt();
            root.announceForAccessibility(text);
        } catch (Throwable t) {
            android.util.Log.w("day", "announce (best-effort)", t);
        }
    }

    // --- Menus (docs/menus.md) -------------------------------------------------
    // The context menu is a PopupMenu shown on long-press (the Android touch convention); the app
    // menu is the app-bar overflow (⋮), built by DayActivity.onCreateOptionsMenu. Both parse the
    // same tab-separated spec (kind\tid\tenabled\tlabel per line) and route item clicks to
    // nativeOnEvent(id, K_MENU_ACTION, 0, "") = MenuAction.

    /** The current app (overflow) menu spec, or null. Set by setAppMenu; read by DayActivity. */
    public static String appMenuSpec = null;

    // --- Lifecycle (docs/lifecycle.md) ----------------------------------------
    // True once nativeStart has run; lifecycle events before that are dropped (native isn't ready).
    // DayActivity forwards Activity lifecycle transitions here with the phase code (day_spec::Lifecycle
    // order: 2=DidBecomeActive 3=WillResignActive 4=WillEnterForeground 5=DidEnterBackground
    // 6=DidReceiveMemoryWarning 7=WillTerminate 8=DidExit), delivered to native as event kind 14.
    public static volatile boolean started = false;

    /** Forward an Activity lifecycle phase to native, once the app has started. */
    public static void lifecycle(int code) {
        if (started) nativeOnEvent(0L, K_LIFECYCLE, code, "");
    }

    // --- Navigation state (docs/navigation.md) --------------------------------
    // A nav surface's `.restore(key)` persists through here, and DayActivity carries the map in
    // the activity's saved instance state. That is not the same lifetime as prefs: Android
    // reclaims a backgrounded process routinely, and a user returning through Recents expects
    // the page they left, so the map has to survive process death. It must not survive the
    // task, though; swiping the app off Recents, or launching it fresh, is the user asking
    // for a clean start, and instance state is discarded in exactly those cases. Persisting to
    // prefs instead would restore stale navigation onto a cold launch.
    public static final java.util.HashMap<String, String> navState = new java.util.HashMap<>();

    /** Read a `.restore` key (null when this launch carried no saved state). Called from Rust. */
    public static String navLoad(String key) {
        return navState.get(key);
    }

    /** Persist a `.restore` key for the next restore of this task instance. Called from Rust. */
    public static void navSave(String key, String value) {
        navState.put(key, value);
    }

    /** Read and clear a key: how a re-mount takes the route the previous mount saved under
     *  `day.route` (willRecreate), so it can never apply to a later mount. Called from Rust. */
    public static String navTake(String key) {
        return navState.remove(key);
    }

    /** Forward a root size change (px) to native as a window resize (event kind 18), with the
     *  part of the shrink the soft keyboard caused (px, 0 while it is down). Posted:
     *  onSizeChanged fires inside the layout pass, and the native relayout it triggers must not
     *  re-enter it. */
    public static void resized(final int w, final int h, final int keyboard) {
        if (!started) return;
        main.post(new Runnable() {
            public void run() {
                nativeOnEvent(0L, K_WINDOW_RESIZED, 0, w + "," + h + "," + keyboard);
            }
        });
    }

    /** Per-row nav context menus (docs/menus.md): one {@link #setContextMenu} spec per row,
     *  joined by U+001E (empty entry = no menu for that row). Best-effort; called after
     *  makeNavMenu/updateNavMenu, like setNavMenuTints, so a failure here can never abort the
     *  native tree build. */
    public static void setNavRowMenus(View navMenu, String joinedSpecs) {
        try {
            if (!(navMenu instanceof NavigationView)) return;
            final NavigationView nav = (NavigationView) navMenu;
            final String[] specs =
                    joinedSpecs.isEmpty() ? new String[0] : joinedSpecs.split("\u001e", -1);
            navRowMenus.put(nav, specs);
            // Rows are recycled now, so a menu cannot be attached once and left: a cell is bound
            // to a different row every time it scrolls back into view. Attach on the one event
            // that fires for each binding (the child joining the RecyclerView) and read the row
            // it is currently showing from its own menu item rather than from its position, which
            // headings and dividers shift.
            android.view.ViewGroup list = navRecycler(nav);
            if (list == null) return;
            if (navRowMenuWatchers.add(nav)) {
                ((androidx.recyclerview.widget.RecyclerView) list)
                        .addOnChildAttachStateChangeListener(
                                new androidx.recyclerview.widget.RecyclerView
                                        .OnChildAttachStateChangeListener() {
                            @Override public void onChildViewAttachedToWindow(View child) {
                                applyNavRowMenu(nav, child);
                            }
                            @Override public void onChildViewDetachedFromWindow(View child) {}
                        });
            }
            for (int i = 0; i < list.getChildCount(); i++) {
                applyNavRowMenu(nav, list.getChildAt(i));
            }
        } catch (Throwable t) {
            android.util.Log.w("day", "setNavRowMenus (best-effort)", t);
        }
    }

    /** The specs a sidebar's rows carry, by nav view, read back as cells recycle. */
    private static final java.util.WeakHashMap<View, String[]> navRowMenus =
            new java.util.WeakHashMap<>();
    /** Nav views whose recycler already has the attach watcher, so it goes on once. */
    private static final java.util.Set<View> navRowMenuWatchers =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<View, Boolean>());

    /** A NavigationView's list, which is the RecyclerView it keeps as its only real child. */
    private static android.view.ViewGroup navRecycler(NavigationView nav) {
        if (nav instanceof DayRefreshNavigationView && ((DayRefreshNavigationView) nav).menuList != null)
            return ((DayRefreshNavigationView) nav).menuList;
        for (int i = 0; i < nav.getChildCount(); i++) {
            if (nav.getChildAt(i) instanceof androidx.recyclerview.widget.RecyclerView) {
                return (android.view.ViewGroup) nav.getChildAt(i);
            }
        }
        return null;
    }

    /** Give one bound cell the menu of the row it is showing (docs/menus.md). */
    private static void applyNavRowMenu(NavigationView nav, View child) {
        try {
            if (!(child instanceof com.google.android.material.internal.NavigationMenuItemView)) {
                return;
            }
            MenuItem item = ((com.google.android.material.internal.NavigationMenuItemView) child)
                    .getItemData();
            String[] specs = navRowMenus.get(nav);
            if (item == null || specs == null) return;
            int row = item.getItemId() - ROW_ID_BASE;
            if (row < 0 || row >= specs.length) return;
            setContextMenu(child, specs[row]);
        } catch (Throwable t) {
            android.util.Log.w("day", "nav row menu (best-effort)", t);
        }
    }

    /**
     * Hand a plain tap on `child` to the nearest ancestor that handles one.
     *
     * A long-press menu and a row tap have to coexist. `setOnLongClickListener` makes a view
     * long-clickable, and `View.onTouchEvent` treats long-clickable as clickable, so the row's
     * content (where the menu is attached) swallows the tap, and the RecyclerView cell's own
     * click listener, which is what performs selection, never runs.
     *
     * Resolved at tap time rather than at attach time: Day builds a row's content and configures
     * its menu before binding it into a cell, so the parent chain does not exist yet when the
     * menu goes on.
     */
    private static void forwardClickToRow(View child) {
        for (android.view.ViewParent p = child.getParent(); p instanceof View; p = p.getParent()) {
            View pv = (View) p;
            if (pv.hasOnClickListeners()) {
                pv.performClick();
                return;
            }
        }
    }

    private static final java.util.Set<View> contextMenuForwardedClicks =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<View, Boolean>());
    private static final java.util.Set<View> gestureTapClicks =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<View, Boolean>());

    /** Attach `spec` as `v`'s context menu (long-press). An empty spec detaches it. */
    /** Give `v` the bounded ripple every Material row draws under a finger, as its foreground:
     *  day fills these views with its own children, and a background ripple would be painted
     *  underneath them and never seen (`android:foreground="?attr/selectableItemBackground"` is
     *  what a Material list item uses, for the same reason).
     *
     *  It goes on whichever view receives the touch, which is not always the same one:
     *  a plain row is handled by the RecyclerView cell, but a row carrying a context menu is
     *  handled by the menu's own view, because being long-clickable makes it eat the touch and
     *  hand the tap on (see setContextMenu / forwardClickToRow). Feedback has to follow the
     *  finger, so it belongs on the eater, not on the view that ends up running the click. */
    static void setTouchFeedback(View v, boolean on) {
        if (!on) {
            v.setForeground(null);
            return;
        }
        if (v.getForeground() != null) {
            return; // already carries one (or its own decoration); don't fight it
        }
        android.util.TypedValue tv = new android.util.TypedValue();
        if (v.getContext().getTheme().resolveAttribute(
                android.R.attr.selectableItemBackground, tv, true) && tv.resourceId != 0) {
            v.setForeground(v.getContext().getDrawable(tv.resourceId));
        }
    }

    interface ContextMenuProvider {
        String build(float x, float y);
    }

    private static final class ContextMenuBinding {
        final ContextMenuProvider provider;
        float x, y;
        android.widget.PopupMenu popup;
        ContextMenuBinding(ContextMenuProvider provider) { this.provider = provider; }
    }

    private static final java.util.WeakHashMap<View, ContextMenuBinding> contextMenus =
            new java.util.WeakHashMap<>();

    public static void setContextMenu(final View v, final String spec) {
        installContextMenu(v, spec == null || spec.isEmpty() ? null : (x, y) -> spec);
    }

    public static void setContextMenuFn(final View v, final long token) {
        installContextMenu(v, (x, y) -> nativeContextMenu(token, x, y));
    }

    /** A menu button (docs/menus.md "Menu buttons"): the click asks Rust for the menu at that
     *  moment and shows it as a PopupMenu anchored under the button, in place of the click
     *  listener makeButton installed, so the press is never also reported as `Pressed`. A
     *  second click while it is open rebuilds it, as a context menu's long press does. */
    public static void setButtonMenu(final View v, final long token) {
        final android.widget.PopupMenu[] open = { null };
        v.setOnClickListener(anchor -> {
            if (open[0] != null) open[0].dismiss();
            String spec = nativeButtonMenu(token);
            // App code may have removed the button while building its menu.
            if (spec == null || spec.isEmpty() || !anchor.isAttachedToWindow()) return;
            android.widget.PopupMenu popup = new android.widget.PopupMenu(
                    anchor.getContext(), anchor, android.view.Gravity.START);
            buildMenu(popup.getMenu(), spec);
            if (popup.getMenu().size() == 0) return;
            open[0] = popup;
            popup.setOnDismissListener(dismissed -> {
                if (open[0] == dismissed) open[0] = null;
            });
            popup.show();
        });
    }

    public static void releaseContextMenu(View v) {
        if (contextMenus.containsKey(v)) installContextMenu(v, null);
    }

    static void installContextMenu(final View v, ContextMenuProvider provider) {
        ContextMenuBinding old = contextMenus.remove(v);
        if (old != null && old.popup != null) old.popup.dismiss();
        if (provider == null) {
            v.setOnLongClickListener(null);
            v.setLongClickable(false);
            if (android.os.Build.VERSION.SDK_INT < 26 && tooltipHints.containsKey(v))
                setViewTooltip(v, tooltipHints.get(v));
            // Remove only the forwarding listener this menu installed. Native menu
            // items, buttons and declared tap gestures own their own click actions.
            if (contextMenuForwardedClicks.remove(v)) {
                v.setOnClickListener(null);
                v.setClickable(false);
                setTouchFeedback(v, false);
            }
            updateTouchListener(v);
            return;
        }
        final ContextMenuBinding binding = new ContextMenuBinding(provider);
        clearCompatTooltip(v);
        contextMenus.put(v, binding);
        updateTouchListener(v);
        // A long-clickable child consumes ordinary taps too; forward them to its row.
        if (!v.hasOnClickListeners()) {
            contextMenuForwardedClicks.add(v);
            v.setOnClickListener(DayBridge::forwardClickToRow);
            setTouchFeedback(v, true);
        }
        v.setOnLongClickListener(anchor -> {
            // Dismiss before rebuilding: the Rust provider replaces the previous action scope.
            if (binding.popup != null) binding.popup.dismiss();
            String spec = binding.provider.build(binding.x, binding.y);
            // App code may remove or reconfigure this view while constructing its menu.
            if (contextMenus.get(anchor) != binding || !anchor.isAttachedToWindow()) return true;
            if (spec == null || spec.isEmpty()) return true;
            android.widget.PopupMenu popup = new android.widget.PopupMenu(anchor.getContext(), anchor);
            buildMenu(popup.getMenu(), spec);
            if (popup.getMenu().size() == 0) return true;
            binding.popup = popup;
            popup.setOnDismissListener(dismissed -> {
                if (binding.popup == dismissed) binding.popup = null;
            });
            popup.show();
            return true;
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
            // kind \t action \t enabled \t checked \t label; the label is last so its spaces
            // survive, and every kind writes all five fields so the indices never shift.
            String[] f = line.split("\t", 5);
            if (f.length < 1) continue;
            String kind = f[0];
            Menu cur = stack.get(stack.size() - 1);
            if (kind.equals("-")) {
                group[0]++; // next items land in a new group → a divider is drawn between them
            } else if (kind.equals("S")) {
                String label = f.length > 4 ? f[4] : "";
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
                String checked = f.length > 3 ? f[3] : "-1";
                String label = f.length > 4 ? f[4] : "";
                MenuItem it = cur.add(group[0], Menu.NONE, order[0]++, label);
                it.setEnabled(enabled);
                // A checkable row draws Material's own check. Android never flips it by itself,
                // so the mark stays exactly what the app's next menu install says it is.
                if (!checked.equals("-1")) {
                    it.setCheckable(true);
                    it.setChecked(checked.equals("1"));
                }
                it.setOnMenuItemClickListener(new MenuItem.OnMenuItemClickListener() {
                    public boolean onMenuItemClick(MenuItem mi) {
                        nativeOnEvent(id, K_MENU_ACTION, 0.0, "");
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
