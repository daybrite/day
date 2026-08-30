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

    /** Close a secondary window (docs/windows.md): finish its activity; onDestroy confirms. */
    public static void closeWindow(long node) {
        DayWindowActivity a = DayWindowActivity.ACTIVE.get(node);
        if (a != null) a.finish();
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

    // --- event kinds -----------------------------------------------------------
    // Mirror of day_spec::bridge::BridgeKind (the shared wire table). day-android's
    // bridge_kinds_parity test reads THIS block and asserts each value against the Rust enum —
    // edit both together. Public so piece-owned Java (the K_CUSTOM channel) can use them.
    public static final int K_PRESSED = 0;
    public static final int K_TEXT_CHANGED = 1;
    public static final int K_TOGGLE_CHANGED = 2;
    public static final int K_VALUE_CHANGED = 3;
    public static final int K_SELECTION_CHANGED = 4;
    public static final int K_NAV_BACK = 5;
    public static final int K_FRAME_CHANGED = 6;
    public static final int K_DEEPLINK = 7;
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
    public static final int K_COVER_HIDDEN = 26;
    /** A styled run's link was tapped (docs/text-runs.md); the string is the target. */
    public static final int K_LINK_ACTIVATED = 27;
    public static final int K_SAFE_AREA = 19;
    public static native void nativeRunPosted(long token);
    /** Frame clock (§8.4): Choreographer's per-vsync callback forwards here with the frame time. */
    public static native void nativeDoFrame(long token, long frameTimeNanos);
    /** Recycling list (docs/list.md): the adapter pulls row count + fills recycled cells. */
    public static native int nativeListLen(long hostId);
    public static native void nativeListBind(long hostId, int position, View cell);
    /** A holder left the visible set — day clears its dayscript ids (docs/list.md). */
    public static native void nativeListRecycle(long hostId, View cell);
    /** Whether row `position` is in the list's programmatic selection (ListPatch::Selected). */
    public static native boolean nativeListIsSelected(long hostId, int position);
    /** Drag-to-reorder (docs/list.md): may `from` drop over `to`? (The app guard's live veto —
     *  a Retarget verdict reads as deny here, since ItemTouchHelper can't relocate the gap.) */
    public static native boolean nativeListCanDrop(long hostId, int from, int to);
    /** Commit one incremental drag swap through day's seam; false = refused, don't move. */
    public static native boolean nativeListMove(long hostId, int from, int to);
    /** May row `index` be swiped away? (docs/list.md — the delete guard.) */
    public static native boolean nativeListCanDelete(long hostId, int index);
    /** Commit a swipe-to-delete through the seam; false if the guard refused. */
    public static native boolean nativeListDelete(long hostId, int index);

    /** Cross-thread → main-thread door for day's scheduler/Setter (§3.3). */
    public static void postMain(final long token) {
        main.post(new Runnable() {
            public void run() { nativeRunPosted(token); }
        });
    }

    /**
     * Frame clock (§8.4): schedule one Choreographer frame callback (called on the UI thread, so
     * getInstance() yields the UI thread's Choreographer). One-shot — day-core re-arms while a
     * frame consumer is live.
     */
    public static void requestFrame(final long token) {
        Choreographer.getInstance().postFrameCallback(new Choreographer.FrameCallback() {
            public void doFrame(long frameTimeNanos) { nativeDoFrame(token, frameTimeNanos); }
        });
    }

    // --- factories + setters (called from Rust over JNI) ---
    /**
     * The device's ordered language preference as BCP-47 tags, comma-joined ("fr-FR,en-US").
     *
     * The CONFIGURATION's list, not `Locale.getDefault()`: it carries every language the user
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
    /** SurfaceRole::SectionCard — the M3 grouped-card fill, resolved from the ACTIVE theme
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

    public static void setSurface(View v, int argb, boolean hasBg, float radiusPx, boolean clips) {
        GradientDrawable d = new GradientDrawable();
        if (hasBg) d.setColor(argb);
        if (radiusPx > 0f) d.setCornerRadius(radiusPx);
        v.setBackground(d);
        if (clips || radiusPx > 0f) v.setClipToOutline(true);
    }

    /** Animatable opacity (§8.4). durMs&lt;=0 sets instantly; otherwise ViewPropertyAnimator
     *  runs it on the render thread with a curve-matched interpolator. */
    public static void setOpacity(View v, float alpha, int durMs, int curve) {
        if (durMs <= 0) { v.animate().cancel(); v.setAlpha(alpha); return; }
        v.animate().alpha(alpha).setDuration(durMs).setInterpolator(interp(curve)).start();
    }

    /** Animatable transform (§8.4): translation (px, additive over the laid-out position),
     *  scale, and rotation (degrees) about the view's center pivot — no relayout. A repeated
     *  animate() call retargets the running animator. */
    public static void setTransform(View v, float tx, float ty, float sx, float sy, float rot,
                                    int durMs, int curve) {
        // See addChild: a presented cover shell is positioned/animated natively.
        if (v instanceof DayCover && ((DayCover) v).presented) return;
        if (durMs <= 0) {
            v.animate().cancel();
            v.setTranslationX(tx); v.setTranslationY(ty);
            v.setScaleX(sx); v.setScaleY(sy);
            v.setRotation(rot);
            return;
        }
        v.animate().translationX(tx).translationY(ty).scaleX(sx).scaleY(sy).rotation(rot)
            .setDuration(durMs).setInterpolator(interp(curve)).start();
    }

    /** Day curve code (§8.4) → Android interpolator. Spring approximates as fast-out/slow-in
     *  (a true spring would pull in androidx.dynamicanimation). */
    private static android.view.animation.Interpolator interp(int curve) {
        switch (curve) {
            case 0: return new android.view.animation.LinearInterpolator();
            case 1: return new android.view.animation.AccelerateInterpolator();
            case 2: return new android.view.animation.DecelerateInterpolator();
            case 4: return new android.view.animation.OvershootInterpolator(); // spring
            default: return new android.view.animation.AccelerateDecelerateInterpolator();
        }
    }

    /** A ViewHolder wrapping one DayFixed cell (docs/list.md). */
    static final class DayCellHolder extends RecyclerView.ViewHolder {
        DayCellHolder(DayFixed cell) { super(cell); }
    }

    /** A native recycling list (docs/list.md): a RecyclerView — the platform's recycling
     *  widget — whose adapter reuses DayFixed cells, day filling each via nativeListBind.
     *  With `reorderable`, an ItemTouchHelper drives the native drag-to-reorder (long-press
     *  lift, elevation, incremental row swaps): every hover is vetted synchronously through
     *  nativeListCanDrop (the app's guard) and each swap commits through nativeListMove. */
    public static View makeList(final long hostId, final int rowHeightPx, final boolean selectable,
                                final boolean reorderable, final boolean deletable,
                                final String deleteLabel) {
        final RecyclerView rv = new RecyclerView(ctx);
        rv.setLayoutManager(new LinearLayoutManager(ctx));
        rv.setAdapter(new RecyclerView.Adapter<DayCellHolder>() {
            public int getItemCount() { return nativeListLen(hostId); }
            public DayCellHolder onCreateViewHolder(ViewGroup parent, int viewType) {
                DayFixed cell = new DayFixed(ctx);
                cell.setLayoutParams(new RecyclerView.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, rowHeightPx));
                if (selectable) {
                    // Touch feedback, which a bare ViewGroup with a click listener does not have:
                    // a row that responds to a tap has to SAY so under the finger, and on Android
                    // that is the bounded ripple every Material list item draws.
                    //
                    // As the FOREGROUND, not the background — day fills this cell with its own
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
                if (selectable) {
                    // A rebound holder inherits the row's selection state — the sync patch
                    // (listPaintSelection) covers the holders already on screen.
                    paintSelected(h.itemView, nativeListIsSelected(hostId, position));
                    h.itemView.setOnClickListener(new View.OnClickListener() {
                        public void onClick(View v) {
                            int pos = h.getBindingAdapterPosition();
                            if (pos != RecyclerView.NO_POSITION) {
                                nativeOnEvent(hostId, K_SELECTION_CHANGED, pos, ""); // kind 4
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
            // One ItemTouchHelper drives BOTH gestures — the platform arbitrates between a
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
                    int drag = reorderable ? (ItemTouchHelper.UP | ItemTouchHelper.DOWN) : 0;
                    // START, not LEFT: it resolves against the layout direction, so the gesture
                    // is a trailing-edge swipe in both LTR and RTL — the same edge iOS uses.
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
                        && nativeListCanDrop(hostId, from, to);
                }
                public boolean onMove(RecyclerView r, RecyclerView.ViewHolder vh,
                                      RecyclerView.ViewHolder target) {
                    // ItemTouchHelper commits INCREMENTALLY — one adjacent swap per callback
                    // while the row is dragged — so each step goes through the seam.
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
                        // The seam already shortened day's snapshot; tell the adapter so the
                        // removal animates out of the swipe instead of snapping via a reload.
                        if (a != null) a.notifyItemRemoved(pos);
                    } else if (a != null) {
                        // Refused after the fact: put the row back where it was.
                        a.notifyItemChanged(pos);
                    }
                }
            }).attachToRecyclerView(rv);
        }
        return rv;
    }
    /** Repaint the VISIBLE holders' selection state from day's record (ListPatch::Selected):
     *  newly bound holders take theirs in onBindViewHolder. Paint only — no events. */
    public static void listPaintSelection(View v, long hostId) {
        if (!(v instanceof RecyclerView)) return;
        RecyclerView rv = (RecyclerView) v;
        for (int i = 0; i < rv.getChildCount(); i++) {
            View child = rv.getChildAt(i);
            int pos = rv.getChildAdapterPosition(child);
            if (pos != RecyclerView.NO_POSITION) {
                paintSelected(child, nativeListIsSelected(hostId, pos));
            }
        }
    }
    private static Integer selectionColor;
    /** A selected row's fill — the theme's accent at 20% alpha, resolved once (colorPrimary
     *  is a plain color int on Material themes; colorControlHighlight is usually a state-list
     *  REFERENCE, which TypedValue.data cannot carry as a color). As the BACKGROUND: day's
     *  row content paints above it, and the ripple foreground stays free for touch feedback. */
    static void paintSelected(View cell, boolean on) {
        if (selectionColor == null) {
            selectionColor = 0x1F888888;
            android.util.TypedValue tv = new android.util.TypedValue();
            if (ctx.getTheme().resolveAttribute(androidx.appcompat.R.attr.colorPrimary, tv, true)
                    && tv.type >= android.util.TypedValue.TYPE_FIRST_COLOR_INT
                    && tv.type <= android.util.TypedValue.TYPE_LAST_COLOR_INT) {
                selectionColor = (tv.data & 0x00FFFFFF) | 0x33000000;
            }
        }
        cell.setBackgroundColor(on ? selectionColor : 0x00000000);
    }

    public static void listReload(View v) {
        if (v instanceof RecyclerView && ((RecyclerView) v).getAdapter() != null) {
            ((RecyclerView) v).getAdapter().notifyDataSetChanged();
        }
    }
    /** Scroll the list so row `row` is visible (docs/list.md), realizing it if needed. Posted
     *  like listScrollToEnd; clamped to the count; no-op when empty. */
    public static void listScrollToRow(View v, final int row) {
        if (!(v instanceof RecyclerView)) return;
        final RecyclerView rv = (RecyclerView) v;
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
        rv.post(new Runnable() {
            public void run() {
                RecyclerView.Adapter<?> a = rv.getAdapter();
                int n = (a == null) ? 0 : a.getItemCount();
                if (n > 0) rv.smoothScrollToPosition(n - 1);
            }
        });
    }

    public static View makeScroll(boolean horizontal) {
        // The onSizeChanged overrides are the keyboard-avoidance reveal (docs/focus.md): when
        // this viewport SHRINKS while a descendant holds focus (the IME just rose and Day
        // relaid out), scroll the focused view back in. Posted so it runs after the layout
        // pass that delivered the new size; requestRectangleOnScreen is a minimal scroll.
        ViewGroup sv;
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
            };
        }
        if (sv instanceof ScrollView) ((ScrollView) sv).setFillViewport(false);
        else ((HorizontalScrollView) sv).setFillViewport(false);
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
    /** Minimal scroll so `[x,y,w,h]` (content px) is visible — scrollRectToVisible semantics.
     *  Serves both scroll axes; the widget clamps to its own scroll range. */
    public static void scrollToRect(final View v, final int x, final int y,
                                    final int w, final int h, final boolean animated) {
        main.post(new Runnable() {
            private int tries = 0;
            public void run() {
                // A freshly built page's widgets have no size until the next layout pass —
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
     * Set a label's text with styled RUNS (docs/text-runs.md).
     *
     * The runs arrive as flat parallel arrays rather than objects: one JNI call with three
     * primitive arrays beats N calls building a Java object per run, and this runs on every
     * label patch. Layout: starts[i], ends[i] are UTF-16 offsets (Java string indices — Rust
     * converts from its byte offsets), and flags[i] packs bold/italic/mono/strike/hasColor with
     * colors[i] holding the ARGB when the flag says so. `links[i]` is null for a plain run.
     *
     * The spans are Android's own, so the text stays ONE TextView: it wraps, selects and is read
     * by TalkBack as a single paragraph, which is the entire point of runs.
     */
    public static void setLabelRuns(
            View v, final long node, String text, int[] starts, int[] ends, int[] flags, int[] colors,
            int[] backgrounds, int[] scales, String linksJoined) {
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
            // Android has ONE underline span: a dotted or wavy request draws a plain line
            // (docs/text-runs.md records which toolkits distinguish them).
            if ((f & 64) != 0) s.setSpan(new android.text.style.UnderlineSpan(), a, b, EXCL);
            // Relative size (FontSpec::scale) as a RelativeSizeSpan, which is exactly its shape —
            // a multiplier over the inherited size, so the run still tracks the user's Font Size
            // setting rather than freezing at a pixel value.
            if (i < scales.length && scales[i] != 1000 && scales[i] > 0) {
                s.setSpan(new android.text.style.RelativeSizeSpan(scales[i] / 1000f), a, b, EXCL);
            }
            if (i < links.length && !links[i].isEmpty()) {
                // A ClickableSpan rather than a URLSpan: the target goes back to Rust, so the
                // app's `.on_link()` decides (route in-app, confirm, open) instead of Android
                // firing an implicit VIEW intent behind Day's back. It keeps URLSpan's own
                // rendering — accent color and underline — from updateDrawState.
                final String target = links[i];
                s.setSpan(new android.text.style.ClickableSpan() {
                    @Override public void onClick(View widget) {
                        nativeOnEvent(node, K_LINK_ACTIVATED, 0.0, target);
                    }
                }, a, b, EXCL);
                anyLink = true;
            }
        }
        tv.setText(s);
        // Clicks on spans need a movement method. It is set ONLY when a link is present: it
        // replaces the selection movement method, so a selectable label without links keeps
        // its selection behavior intact (docs/text-runs.md).
        if (anyLink) {
            tv.setMovementMethod(android.text.method.LinkMovementMethod.getInstance());
        }
    }
    /**
     * `sp` size (scales with the accessibility Font Size setting), font weight (100–900), italic,
     * and an optional bundled font family (null for the system font — §18.4).
     */
    public static void setLabelFont(
            View v, float sp, int weight, boolean italic, String family, boolean tabular) {
        TextView t = (TextView) v;
        // Tabular figures via the OpenType feature, so the typeface is untouched and only the
        // digits change metrics. A font without `tnum` ignores the request.
        t.setFontFeatureSettings(tabular ? "tnum" : null);
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

    /**
     * Each button's tint as Material styled it, so clearing an app tint can put the THEME's
     * container color back.
     *
     * `setBackgroundTintList(null)` does not restore a default — it removes the tint outright, and
     * a MaterialButton's background is a shape drawable that the tint is what colors. Left null
     * it draws raw black, which is how every untinted button on this backend came out.
     */
    private static final java.util.Map<View, android.content.res.ColorStateList> buttonTints =
            new java.util.WeakHashMap<>();

    public static View makeButton(final long id, String title) {
        MaterialButton b = new MaterialButton(ctx); // M3 filled button (Expressive shape/motion)
        buttonTints.put(b, b.getBackgroundTintList());
        b.setText(title);
        b.setOnClickListener(new View.OnClickListener() {
            public void onClick(View x) { nativeOnEvent(id, K_PRESSED, 0, null); }
        });
        return b;
    }

    /**
     * Style a button in place: kind 0 automatic, 1 bordered, 2 prominent, 3 tinted (argb/fg).
     *
     * A tint is `backgroundTint` on the MaterialButton, so Material keeps drawing the ripple, the
     * state overlays and the disabled alpha itself — the view stays a MaterialButton, with its
     * role, focus and keyboard activation intact. Anything but a tint leaves the stock M3 look,
     * which is already the filled button day's `prominent` asks for.
     */
    public static void setButtonStyle(View v, int kind, int argb, int fgArgb) {
        if (!(v instanceof MaterialButton)) return;
        MaterialButton b = (MaterialButton) v;
        if (kind != 3) {
            // Back to what the theme dressed it in (see buttonTints), not to no tint at all.
            b.setBackgroundTintList(buttonTints.get(b));
            return;
        }
        b.setBackgroundTintList(android.content.res.ColorStateList.valueOf(argb));
        b.setTextColor(fgArgb);
        b.setIconTint(android.content.res.ColorStateList.valueOf(fgArgb));
    }

    /** Attach a tap or drag recognizer to a view (docs/shapes.md). Coordinates are px; Rust
     *  converts to dp. Event kind 11; num = phase (0=tap 1=began 2=changed 3=ended). */
    /** Per-view enabled gestures `{wantsTap, wantsDrag}` — a view can carry both (tap + drag), so
     *  the single OnTouchListener must emit whichever the node asked for. UIKit's recognizers
     *  coexist; a bare setOnTouchListener does not, so we accumulate here rather than overwrite. */
    static final java.util.WeakHashMap<View, boolean[]> gestureFlags = new java.util.WeakHashMap<>();

    public static void enableGesture(View v, final long id, final boolean isDrag) {
        boolean[] flags = gestureFlags.get(v);
        if (flags == null) { flags = new boolean[2]; gestureFlags.put(v, flags); }
        if (isDrag) flags[1] = true; else flags[0] = true;
        final boolean[] f = flags; // {wantsTap, wantsDrag}
        v.setOnTouchListener(new View.OnTouchListener() {
            float sx, sy;
            public boolean onTouch(View view, MotionEvent ev) {
                float x = ev.getX(), y = ev.getY();
                switch (ev.getActionMasked()) {
                    case MotionEvent.ACTION_DOWN:
                        sx = x; sy = y;
                        if (f[1]) nativeOnEvent(id, K_GESTURE, 1, x + "," + y + ",0,0");
                        return true;
                    case MotionEvent.ACTION_MOVE:
                        if (f[1]) nativeOnEvent(id, K_GESTURE, 2, x + "," + y + "," + (x - sx) + "," + (y - sy));
                        return true;
                    case MotionEvent.ACTION_UP:
                        if (f[1]) nativeOnEvent(id, K_GESTURE, 3, x + "," + y + "," + (x - sx) + "," + (y - sy));
                        if (f[0] && Math.abs(x - sx) < 40 && Math.abs(y - sy) < 40) {
                            nativeOnEvent(id, K_GESTURE, 0, x + "," + y + ",0,0");
                            view.performClick();
                        }
                        return true;
                    case MotionEvent.ACTION_CANCEL:
                        if (f[1]) nativeOnEvent(id, K_GESTURE, 3, x + "," + y + "," + (x - sx) + "," + (y - sy));
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
            public void afterTextChanged(Editable s) { nativeOnEvent(id, K_TEXT_CHANGED, 0, s.toString()); }
            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}
            public void onTextChanged(CharSequence s, int a, int b, int c) {}
        });
        // Focus + submit (docs/focus.md): kind 16 reports the gain/loss pair; kind 17 is the
        // IME action ("done"/enter). Returning false keeps the platform default (dismiss).
        e.setOnFocusChangeListener(new View.OnFocusChangeListener() {
            public void onFocusChange(View x, boolean hasFocus) {
                nativeOnEvent(id, K_FOCUS_CHANGED, hasFocus ? 1 : 0, null);
            }
        });
        e.setOnEditorActionListener(new TextView.OnEditorActionListener() {
            public boolean onEditorAction(TextView x, int actionId, KeyEvent ev) {
                // A real IME action (done/next/go/...), or a hardware-enter key-DOWN — the
                // unspecified-action key-UP call must not fire a second submit.
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
            e.setText(value);
            e.setSelection(value.length());
        }
    }
    public static void setPlaceholder(View v, String value) {
        if (v instanceof TextInputLayout) ((TextInputLayout) v).setHint(value);
        else ((EditText) v).setHint(value);
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
            if (imm != null) imm.hideSoftInputFromWindow(target.getWindowToken(), 0);
            // Android focus is never "nowhere": DayActivity's root is focusable-in-touch-mode,
            // so clearing lands there instead of snapping back to the first focusable field.
            target.clearFocus();
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
        // touch, so it IS settled the moment it lands. The flag is a one-element array because a
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
        // A PRESENTED cover shell lives under the activity content root; a day-tree
        // re-insert (z-order re-sync among its siblings) must not re-parent it — doing so
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
     * Put an existing child at `index` among its siblings — Day's z-order resync
     * (Toolkit::move_child), which walks the whole sibling row and asks for each position in
     * turn.
     *
     * A ViewGroup has no move primitive, so the fallback is remove-then-add — and that DETACHES:
     * {@link ViewGroup#removeView} unfocuses the view and everything under it, so a moved subtree
     * containing the field the user is typing into loses its focus and the soft keyboard
     * (docs/focus.md). Hence the two rules here: a child already at `index` is left alone, and one
     * that has to move is re-added AT that index rather than appended — appending would make the
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
        // See addChild: a presented cover shell is positioned natively (fullscreen).
        if (v instanceof DayCover && ((DayCover) v).presented) return;
        ViewGroup p = (ViewGroup) v.getParent();
        // Nav / tab pages fill the host's page frame — their frames are native-owned.
        if (p != null && p.getParent() instanceof DayNavHost) return;
        if (p != null && p.getParent() instanceof DayTabs) return;
        if (p instanceof DayFixed) ((DayFixed) p).setChildFrame(v, x, y, w, h);
    }

    // --- navigation (docs/navigation.md) ---
    public static View makeNavHost(long id, String title, boolean adaptive, float tileMinDp) {
        return new DayNavHost(ctx, id, title, adaptive, tileMinDp);
    }

    // Trailing nav-bar action (docs/navigation.md, NavProps::bar_action): applied AFTER the host is
    // built, and wrapped so a failure here can never propagate into the native tree build (where it
    // would abort the whole surface and blank the app). `action == 0` or a non-nav view is a no-op.
    // One call for ALL of the host's actions, `\n`-joined per field — the same wire shape
    // `setNavMenuTints`/`setNavMenuBadges` use. Parallel arrays rather than one call per action so
    // a partial failure cannot leave half a bar installed: this either adds them all or logs and
    // adds none. `rootOnly` is "1"/"0" per action (NavBarScope::RootPage).
    public static void setNavMenu(View navHost, String icons, String labels, String actions,
            String rootOnly) {
        if (!(navHost instanceof DayNavHost) || actions == null || actions.isEmpty()) {
            return;
        }
        try {
            String[] ic = icons == null ? new String[0] : icons.split("\n", -1);
            String[] lb = labels == null ? new String[0] : labels.split("\n", -1);
            String[] ac = actions.split("\n", -1);
            String[] ro = rootOnly == null ? new String[0] : rootOnly.split("\n", -1);
            for (int i = 0; i < ac.length; i++) {
                long id = Long.parseLong(ac[i]);
                if (id == 0) {
                    continue;
                }
                ((DayNavHost) navHost).addBarAction(
                        i < ic.length ? ic[i] : "",
                        i < lb.length ? lb[i] : "",
                        id,
                        i < ro.length && "1".equals(ro[i]));
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav bar action setup failed; continuing without it", t);
        }
    }
    /**
     * Inline search on the navigation list (docs/search.md). Applied AFTER the host is built and
     * wrapped, for the same reason `setNavMenu` above is: a throw on `makeNavHost`'s own path
     * aborts the native tree build and leaves the app blank, so decoration never rides it.
     *
     * No auto-hide. iOS reveals its field by over-scrolling past the top of the list; Material has
     * no equivalent gesture, so the field simply stays put (docs/search.md).
     */
    public static void setNavSearch(View navHost, long id, String prompt, String text) {
        if (!(navHost instanceof DayNavHost)) {
            return;
        }
        try {
            ((DayNavHost) navHost).setSearch(id, prompt, text);
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav search setup failed; continuing without it", t);
        }
    }

    /** Write the app's own query back into the field, without echoing it back as a change. */
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

    public static View makeNavPage(final long id) {
        DayFixed page = new DayFixed(ctx);
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
    public static void navSetTitle(View host, String title) { ((DayNavHost) host).retitle(title); }
    public static void navSetGuard(View host, boolean on) { ((DayNavHost) host).setGuard(on); }

    // --- fullscreen cover (docs/cover.md) ---
    public static View makeCover(long node) { return new DayCover(ctx, node); }
    public static void coverPresent(View cover, int bg, boolean hasBg, boolean dismissDisabled) {
        DayCover c = (DayCover) cover;
        if (hasBg) c.setBackgroundColor(bg);
        c.present(dismissDisabled);
    }
    public static void coverSetDismissDisabled(View cover, boolean d) {
        ((DayCover) cover).setDismissDisabled(d);
    }
    public static void coverDismiss(View cover) { ((DayCover) cover).dismissCover(); }

    /** Whether native transitions have settled (Toolkit::ui_idle): dayscript screenshots
     *  wait on this so captures never show a cover mid-slide. */
    public static boolean uiIdle() { return DayCover.slidesInFlight == 0; }

    /** Whether the system renders in dark appearance (Toolkit::dark_mode). */
    public static boolean isDarkMode() {
        int night = ((android.content.Context) ctx).getResources().getConfiguration().uiMode
                & android.content.res.Configuration.UI_MODE_NIGHT_MASK;
        return night == android.content.res.Configuration.UI_MODE_NIGHT_YES;
    }

    /** Whether this device can be told to use a light or dark appearance for THIS APP alone.
     *
     *  `UiModeManager.setApplicationNightMode` arrived in API 31. The older route,
     *  `AppCompatDelegate.setDefaultNightMode`, only restyles an activity that runs through
     *  AppCompat's delegate, and `DayActivity` is a plain `FragmentActivity` — so below 31 there
     *  is nothing to offer, and `Cap::Appearance` says so rather than showing a control that
     *  would do nothing. */
    public static boolean canSetAppearance() {
        return android.os.Build.VERSION.SDK_INT >= 31;
    }

    /** Apply an app-level appearance (Toolkit::set_appearance): 0 light, 1 dark, 2 follow system.
     *
     *  Applying it changes the app's uiMode, which the OS delivers to `DayActivity` as a
     *  configuration change — the manifest lists `uiMode`, so the activity is not recreated — and
     *  that path already calls `appearanceChanged()`. So this only has to ask; the report back to
     *  day-core is the same one a user flipping the system theme produces. */
    public static void setAppearance(int mode) {
        if (!canSetAppearance() || ctx == null) return;
        // IDEMPOTENT, and it has to be. The settings row re-applies its stored value every time
        // the tree is built, and applying one recreates the activity — which builds the tree
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
        um.setApplicationNightMode(night);
        // A DayNight theme picks its variant when the activity's theme is RESOLVED, which is at
        // creation. The uiMode change alone leaves every view — and every view inflated after it —
        // on the colors chosen at startup, so the appearance has to be re-resolved by recreating.
        // Day's tree is rebuilt from `onCreate`, the same path a cold start takes.
        if (ctx instanceof android.app.Activity) ((android.app.Activity) ctx).recreate();
    }

    /** The appearance last applied, so re-applying the stored choice does not recreate again. */
    private static int appliedNightMode = Integer.MIN_VALUE;

    /** Report a light/dark switch to native (event kind 25), once the app has started.
     *  DayActivity calls this from onConfigurationChanged; day-core restyles what it owns and
     *  rebuilds app-painted surfaces. */
    public static void appearanceChanged() {
        if (started) nativeOnEvent(0L, K_APPEARANCE_CHANGED, 0, "");
    }

    /** A PNG of this app's own window (docs/window-image.md) — `null` when there is nothing
     *  to draw.
     *
     *  `View.draw(Canvas)` rather than `PixelCopy`: it is SYNCHRONOUS, which is what lets
     *  `day::window_image()` stay a plain call on every backend. The cost is that
     *  surface-backed content (a `VideoView`, and any future GL or camera view) draws EMPTY —
     *  those pixels live on a surface the view tree never touches, and only the async PixelCopy
     *  can read them back.
     *
     *  `chrome` picks the decor view (the whole window, action bar and system-bar backgrounds)
     *  over the app's content view. */
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

    /** Deferred system gestures (docs/cover.md): while any `defers_system_gestures` subtree
     *  is mounted, enter swipe-to-reveal immersive mode — the platform's "first swipe shows
     *  the bars, second swipe acts" behavior, the closest analogue of iOS's screen-edge
     *  deferral. Restores normal bars when the last request unmounts. */
    public static void setDeferSystemGestures(boolean on) {
        android.app.Activity act = (android.app.Activity) ctx;
        androidx.core.view.WindowInsetsControllerCompat c =
                androidx.core.view.WindowCompat.getInsetsController(
                        act.getWindow(), act.getWindow().getDecorView());
        if (on) {
            c.setSystemBarsBehavior(androidx.core.view.WindowInsetsControllerCompat
                    .BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE);
            c.hide(androidx.core.view.WindowInsetsCompat.Type.systemBars());
        } else {
            c.show(androidx.core.view.WindowInsetsCompat.Type.systemBars());
            c.setSystemBarsBehavior(androidx.core.view.WindowInsetsControllerCompat
                    .BEHAVIOR_DEFAULT);
        }
    }

    // --- navigation suite (docs/navigation.md) ---
    // The NAV host in its `Tabs` presentation: the same container, reached from the nav path.

    /** Pages whose rows ARE the chrome — marked before insertion, consumed by `addChild`. */
    private static final java.util.Set<View> navChromePages =
            java.util.Collections.newSetFromMap(new java.util.WeakHashMap<View, Boolean>());

    public static View makeNavSuite(long id, int initial) { return new DayTabs(ctx, id, initial); }

    /** Mark `page` as the suite's chrome source before it is inserted (see `addChild`). */
    public static void markNavChromePage(View page) { navChromePages.add(page); }

    /**
     * Hand a nav menu's rows to the suite that will draw them as chrome.
     *
     * Called with the MENU, not the suite: the menu knows its rows, and by the time it is inserted
     * its ancestors reach the suite, so the walk up is what connects the two. A menu outside a
     * suite — every non-tabs presentation — finds nothing and this is a no-op.
     */
    public static void setNavSuiteRows(View menu, String titles, String icons, long menuNode) {
        for (android.view.ViewParent p = menu.getParent(); p != null; p = p.getParent()) {
            if (p instanceof DayTabs) {
                ((DayTabs) p).setRows(titles, icons, menuNode);
                return;
            }
        }
    }

    public static void setNavSuiteSelected(View suite, int index) {
        if (suite instanceof DayTabs) ((DayTabs) suite).select(index);
    }
    /** nav_menu(): standard tappable list rows (ripple, 48dp) for the route table. `joinedIcons`
     *  is a parallel, index-aligned list of bundled image NAMES ("" = no icon), shown as a tinted
     *  leading drawable on each row — the Material navigation-drawer idiom. */
    public static View makeNavMenu(final long id, String joinedItems, String joinedIcons) {
        android.widget.LinearLayout list = new android.widget.LinearLayout(ctx);
        list.setOrientation(android.widget.LinearLayout.VERTICAL);
        // Immersive mode (docs/layout.md): the page runs under the transparent status bar and
        // floating app bar — start the rows below them. The list lives inside its own
        // ScrollView, so the padding scrolls away naturally.
        if (DayActivity.edgeToEdge) {
            android.util.TypedValue abs = new android.util.TypedValue();
            int bar = 0;
            if (ctx.getTheme().resolveAttribute(android.R.attr.actionBarSize, abs, true)) {
                bar = android.util.TypedValue.complexToDimensionPixelSize(
                        abs.data, ctx.getResources().getDisplayMetrics());
            }
            list.setPadding(0, DayActivity.statusInsetPx + bar, 0, 0);
            list.setClipToPadding(false);
        }
        fillNavMenu(id, list, joinedItems, joinedIcons);
        // The nav menu can have more items than fit on screen (the showcase sidebar has ~20), so it
        // must scroll — wrap the row column in a vertical ScrollView (fillViewport so it still fills
        // when short).
        ScrollView sv = new ScrollView(ctx);
        sv.setFillViewport(true);
        sv.setTag(id); // updateNavMenu re-reads it when rebuilding rows
        sv.addView(list, new ScrollView.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        return sv;
    }

    /** The item set changed (day's NavMenuPatch::Items — a data-driven `selector().items(…)`
     *  block re-derived): rebuild every row so each click listener carries its CURRENT index.
     *  Reusing stale rows after a removal shifts every later selection by one and drops the
     *  last row's selection entirely. */
    public static void updateNavMenu(View v, String joinedItems, String joinedIcons) {
        ScrollView sv = (ScrollView) v;
        long id = (Long) sv.getTag();
        android.widget.LinearLayout list = (android.widget.LinearLayout) sv.getChildAt(0);
        list.removeAllViews();
        fillNavMenu(id, list, joinedItems, joinedIcons);
    }

    /** Build one tappable row per item into `list` (ripple, 48dp, optional tinted leading icon —
     *  the Material navigation-drawer idiom). Each row reports its index on click. */
    private static void fillNavMenu(final long id, android.widget.LinearLayout list,
            String joinedItems, String joinedIcons) {
        String[] items = joinedItems.isEmpty() ? new String[0] : joinedItems.split("\u001f");
        android.util.TypedValue tv = new android.util.TypedValue();
        ctx.getTheme().resolveAttribute(android.R.attr.selectableItemBackground, tv, true);
        float d = ctx.getResources().getDisplayMetrics().density;
        // KeepEmptyParts (limit -1) so icon names stay index-aligned with rows lacking an icon.
        String[] icons = joinedIcons.isEmpty() ? new String[0] : joinedIcons.split("\u001f", -1);
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
            // Leading icon: a template glyph tinted to the row's text color (so it reads in light
            // and dark), 24dp, with padding before the label — the Material nav-drawer idiom.
            String iconName = i < icons.length ? icons[i] : "";
            android.graphics.drawable.Drawable icon = drawableByName(ctx, iconName);
            if (icon != null) {
                int sz = (int) (24 * d);
                icon = icon.mutate();
                icon.setBounds(0, 0, sz, sz);
                icon.setTint(row.getCurrentTextColor());
                row.setCompoundDrawablesRelative(icon, null, null, null);
                row.setCompoundDrawablePadding((int) (16 * d));
            }
            row.setOnClickListener(new View.OnClickListener() {
                @Override public void onClick(View v) {
                    nativeOnEvent(id, K_SELECTION_CHANGED, index, null); // kind 4 = SelectionChanged
                }
            });
            list.addView(row, new android.widget.LinearLayout.LayoutParams(
                    ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        }
    }

    /** Per-row nav icon tints (docs/vectors.md), index-aligned ARGB ints ("0" = untinted —
     *  the row keeps its text-color template tint). Best-effort by design: called AFTER
     *  makeNavMenu/updateNavMenu so a failure here can never abort the native tree build. */
    public static void setNavMenuTints(View navMenu, String joinedTints) {
        try {
            if (!(navMenu instanceof android.widget.ScrollView)) return;
            android.view.ViewGroup list =
                    (android.view.ViewGroup) ((android.widget.ScrollView) navMenu).getChildAt(0);
            if (list == null) return;
            String[] tints = joinedTints.isEmpty() ? new String[0] : joinedTints.split("\u001f", -1);
            for (int i = 0; i < list.getChildCount() && i < tints.length; i++) {
                if (!(list.getChildAt(i) instanceof TextView)) continue;
                TextView row = (TextView) list.getChildAt(i);
                android.graphics.drawable.Drawable[] ds = row.getCompoundDrawablesRelative();
                if (ds.length == 0 || ds[0] == null) continue;
                long tint;
                try { tint = Long.parseLong(tints[i].trim()); } catch (NumberFormatException e) { continue; }
                if (tint != 0) ds[0].setTint((int) tint);
            }
        } catch (Throwable t) {
            android.util.Log.e("Day", "nav menu tints skipped", t);
        }
    }

    /** Per-row trailing status glyphs (docs/navigation.md) — a starred page's star. Index-aligned
     *  names ("" = none) with matching ARGB tints ("0" = keep the row's text-color template
     *  tint). The glyph goes in the compound drawable's END slot, so the row needs no new layout
     *  and the label still ellipsizes into what is left.
     *
     *  Best-effort and called AFTER makeNavMenu/updateNavMenu, exactly like setNavMenuTints: a
     *  throw on the nav host's own build path takes the whole tree down with it, and a decoration
     *  is never worth that. */
    public static void setNavMenuBadges(View navMenu, String joinedIcons, String joinedTints) {
        try {
            if (!(navMenu instanceof android.widget.ScrollView)) return;
            android.view.ViewGroup list =
                    (android.view.ViewGroup) ((android.widget.ScrollView) navMenu).getChildAt(0);
            if (list == null) return;
            String[] icons = joinedIcons.isEmpty() ? new String[0] : joinedIcons.split("\u001f", -1);
            String[] tints = joinedTints.isEmpty() ? new String[0] : joinedTints.split("\u001f", -1);
            float d = ctx.getResources().getDisplayMetrics().density;
            for (int i = 0; i < list.getChildCount() && i < icons.length; i++) {
                if (!(list.getChildAt(i) instanceof TextView)) continue;
                TextView row = (TextView) list.getChildAt(i);
                android.graphics.drawable.Drawable[] ds = row.getCompoundDrawablesRelative();
                android.graphics.drawable.Drawable badge = drawableByName(ctx, icons[i]);
                if (badge != null) {
                    int sz = (int) (18 * d);
                    badge = badge.mutate();
                    badge.setBounds(0, 0, sz, sz);
                    long tint = 0;
                    if (i < tints.length) {
                        try { tint = Long.parseLong(tints[i].trim()); } catch (NumberFormatException e) { tint = 0; }
                    }
                    badge.setTint(tint != 0 ? (int) tint : row.getCurrentTextColor());
                }
                row.setCompoundDrawablesRelative(ds.length > 0 ? ds[0] : null, null, badge, null);
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
            // A titled list of choices — the Android idiom for an action sheet.
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

    /** Open a URL in the system's default handler (browser for http(s), mail app for mailto:, ...).
     *  Backs the `link` piece. NEW_TASK is required because ctx may be the application context. */
    public static void openUrl(String url) {
        if (ctx == null || url == null) return;
        try {
            android.content.Intent intent = new android.content.Intent(
                    android.content.Intent.ACTION_VIEW, android.net.Uri.parse(url));
            intent.addFlags(android.content.Intent.FLAG_ACTIVITY_NEW_TASK);
            ctx.startActivity(intent);
        } catch (Exception ignored) {
            // No handler for the scheme, or the URI was malformed — nothing to open.
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
        android.net.Uri uri = (resultCode == android.app.Activity.RESULT_OK && data != null) ? data.getData() : null;
        if (uri == null) {
            nativeOnEvent(req, K_PRESENT_DISMISSED, 0.0, null); // dismissed
            return;
        }
        try {
            if (src != null) {
                // Save: stream the Day-staged temp file into the chosen document; return its URI.
                copyStream(new java.io.FileInputStream(src),
                        ctx.getContentResolver().openOutputStream(uri));
                nativeOnEvent(req, K_PRESENT_FILE, 0.0, uri.toString()); // 15 = files
            } else {
                // Open: copy the picked document into an app cache file, return that readable path.
                String name = displayName(uri);
                java.io.File out = new java.io.File(ctx.getCacheDir(), "day-open-" + req + "-" + name);
                copyStream(ctx.getContentResolver().openInputStream(uri),
                        new java.io.FileOutputStream(out));
                nativeOnEvent(req, K_PRESENT_FILE, 0.0, out.getAbsolutePath());
            }
        } catch (Exception e) {
            android.util.Log.w("Day", "file open/save transfer failed", e);
            nativeOnEvent(req, K_PRESENT_DISMISSED, 0.0, null);
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

    /** First text baseline from the view's top, in px, for a view laid out at `wPx` x `hPx`
     *  (docs/baseline.md). `View.getBaseline()` is the platform's own answer — TextView and its
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

    public static void setEnabled(View v, boolean b) { v.setEnabled(b); }

    public static View makeCanvas() { return new DayCanvasView(ctx); }
    public static void setCanvasOps(View v, double[] nums, String textsJoined) {
        ((DayCanvasView) v).setOps(nums, textsJoined);
    }
    /** `ImagePatch::Tint`: repaint a realized glyph. 0 restores the authored colors. */
    public static void setImageTint(View v, int tint) {
        if (!(v instanceof android.widget.ImageView)) {
            return;
        }
        ((android.widget.ImageView) v).setImageTintList(
                tint == 0 ? null : android.content.res.ColorStateList.valueOf(tint));
    }

    public static View makeImage(String name, int mode, int tint) {
        View v = makeImageInner(name, mode);
        // Vector-glyph tint (docs/vectors.md): drawable tint keeps a VectorDrawable/PNG's alpha
        // as the mask. 0 = untinted (a real tint always carries alpha 0xFF).
        if (tint != 0 && v instanceof android.widget.ImageView) {
            ((android.widget.ImageView) v).setImageTintList(
                    android.content.res.ColorStateList.valueOf(tint));
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
        // Resolution goes through drawableByName, which is ALSO where the weight-alias fallback
        // lives (docs/vectors.md): a plain SVG stages one asset, so `<glyph>__light`/`__bold` must
        // land back on `<glyph>` rather than draw nothing. This path used to do its own
        // getIdentifier + assets lookup and skip that fallback, which is why every
        // `vector(plain).weight(Light|Bold)` was blank on Android while it resolved everywhere
        // else — invisible to `assert_visible`, since the ImageView had a frame either way.
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
    /** Load a bundled image by NAME (docs/navigation.md) as a mutable Drawable: a processed
     *  `res/drawable/<name>` resource (aapt2-crunched), else a raw asset by path; null if neither
     *  resolves or `name` is empty. Callers tint it (nav rows) or let the widget tint it (tabs). */
    static android.graphics.drawable.Drawable drawableByName(Context c, String name) {
        if (name == null || name.isEmpty()) return null;
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
        try {
            android.graphics.Bitmap bm =
                    android.graphics.BitmapFactory.decodeStream(c.getAssets().open(name));
            if (bm != null) return new android.graphics.drawable.BitmapDrawable(c.getResources(), bm);
        } catch (Exception e) {
            // fall through to null
        }
        return null;
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
    // nativeOnEvent(id, K_MENU_ACTION, 0, "") = MenuAction.

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
        if (started) nativeOnEvent(0L, K_LIFECYCLE, code, "");
    }

    // --- Navigation state (docs/navigation.md) --------------------------------
    // A nav surface's `.restore(key)` persists through here, and DayActivity carries the map in
    // the activity's SAVED INSTANCE STATE. That is deliberately not the same lifetime as prefs:
    // Android reclaims a backgrounded process routinely, and a user returning through Recents
    // expects the page they left, so the map has to survive process death. It must NOT survive
    // the task, though — swiping the app off Recents, or launching it fresh, is the user asking
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

    /** Forward a root size change (px) to native as a window resize (event kind 18). Posted:
     *  onSizeChanged fires inside the layout pass, and the native relayout it triggers must not
     *  re-enter it. */
    public static void resized(final int w, final int h) {
        if (!started) return;
        main.post(new Runnable() {
            public void run() { nativeOnEvent(0L, K_WINDOW_RESIZED, 0, w + "," + h); }
        });
    }

    /** Per-row nav context menus (docs/menus.md): one {@link #setContextMenu} spec per row,
     *  joined by U+001E (empty entry = no menu for that row). Best-effort by design — called
     *  AFTER makeNavMenu/updateNavMenu, like setNavMenuTints, so a failure here can never
     *  abort the native tree build. */
    public static void setNavRowMenus(View navMenu, String joinedSpecs) {
        try {
            if (!(navMenu instanceof android.widget.ScrollView)) return;
            android.view.ViewGroup list =
                    (android.view.ViewGroup) ((android.widget.ScrollView) navMenu).getChildAt(0);
            if (list == null) return;
            String[] specs = joinedSpecs.isEmpty() ? new String[0] : joinedSpecs.split("\u001e", -1);
            for (int i = 0; i < list.getChildCount() && i < specs.length; i++) {
                setContextMenu(list.getChildAt(i), specs[i]);
            }
        } catch (Throwable t) {
            android.util.Log.w("day", "setNavRowMenus (best-effort)", t);
        }
    }

    /**
     * Hand a plain tap on `child` to the nearest ancestor that handles one.
     *
     * A long-press menu and a row tap have to coexist. `setOnLongClickListener` makes a view
     * long-clickable, and `View.onTouchEvent` treats long-clickable as clickable — so the row's
     * CONTENT (where the menu is attached) swallows the tap, and the RecyclerView cell's own
     * click listener, which is what performs selection, never runs.
     *
     * Resolved at tap time rather than at attach time: Day builds a row's content and configures
     * its menu BEFORE binding it into a cell, so the parent chain does not exist yet when the
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

    /** Attach `spec` as `v`'s context menu (long-press). An empty spec detaches it. */
    /** Give `v` the bounded ripple every Material row draws under a finger, as its FOREGROUND —
     *  day fills these views with its own children, and a background ripple would be painted
     *  underneath them and never seen (`android:foreground="?attr/selectableItemBackground"` is
     *  what a Material list item uses, for the same reason).
     *
     *  It goes on whichever view actually RECEIVES the touch, which is not always the same one:
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
            return; // already carries one (or its own decoration) — don't fight it
        }
        android.util.TypedValue tv = new android.util.TypedValue();
        if (v.getContext().getTheme().resolveAttribute(
                android.R.attr.selectableItemBackground, tv, true) && tv.resourceId != 0) {
            v.setForeground(v.getContext().getDrawable(tv.resourceId));
        }
    }

    public static void setContextMenu(final View v, final String spec) {
        if (spec == null || spec.isEmpty()) {
            v.setOnLongClickListener(null);
            v.setLongClickable(false);
            v.setOnClickListener(null);
            v.setClickable(false);
            setTouchFeedback(v, false);
            return;
        }
        // See forwardClickToRow: the menu makes this view eat touches, so the tap it eats has to
        // be handed on. Only where the view has no click behavior of its own — a button with a
        // context menu keeps its own action.
        if (!v.hasOnClickListeners()) {
            v.setOnClickListener(new View.OnClickListener() {
                public void onClick(View child) {
                    forwardClickToRow(child);
                }
            });
            // This view is now the row's touch target, so the row's press feedback is its job.
            setTouchFeedback(v, true);
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
