// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.graphics.Bitmap;
import android.graphics.Canvas;
import android.graphics.Matrix;
import android.graphics.Paint;
import android.graphics.Rect;
import android.graphics.RectF;
import android.graphics.pdf.PdfDocument;
import android.os.Build;
import android.text.InputType;
import android.view.KeyCharacterMap;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;
import android.text.Editable;
import android.view.inputmethod.BaseInputConnection;
import android.view.inputmethod.EditorInfo;
import android.view.inputmethod.InputConnection;
import android.view.inputmethod.InputMethodManager;
import java.util.Arrays;
import java.util.HashSet;
import java.util.Set;

/** Replays day's display list (§11). Ops arrive dp-encoded; drawing scales by density. */
public class DayCanvasView extends View {
    /** The day node this canvas renders: focus and keys are reported against it. */
    final long id;
    double[] nums = new double[0];
    String[] texts = new String[0];

    public DayCanvasView(Context c, long id) {
        super(c);
        this.id = id;
        // Focus, and with it the keyboard (docs/focus.md, docs/menus.md): a canvas is the one
        // built-in piece with no native control under it, so nothing else would ever make it
        // the focused view. Focusable IN TOUCH MODE, or `requestFocus` refuses it on a phone.
        setFocusable(true);
        setFocusableInTouchMode(true);
        if (Build.VERSION.SDK_INT >= 26) {
            // Neither the grey wash Android lays over a focused view once a key is pressed,
            // nor the scroll that brings a newly focused view fully into sight: the app draws
            // its own selection, and a press on a tall canvas must not jump the page.
            setDefaultFocusHighlightEnabled(false);
            setRevealOnFocusHint(false);
        }
    }

    // A press focuses the canvas, the way tapping a text field does, but only when the app
    // hung `.on_key` on it: in touch mode a focus move is not free (it can dismiss a raised
    // soft keyboard), so a canvas that wants no keys leaves focus where it was. This runs
    // before the touch listener that carries the gestures, which still sees the press.
    @Override public boolean dispatchTouchEvent(MotionEvent ev) {
        if (ev.getActionMasked() == MotionEvent.ACTION_DOWN && !isFocused()
                && DayBridge.nativeHandlesKeys(id)) {
            requestFocus();
        }
        return super.dispatchTouchEvent(ev);
    }

    // Report focus both ways, so `.focused(signal)` binds two-way and dayscript's
    // `assert_focused` can see it.
    @Override protected void onFocusChanged(boolean gained, int direction, Rect previous) {
        super.onFocusChanged(gained, direction, previous);
        DayBridge.nativeOnEvent(id, DayBridge.K_FOCUS_CHANGED, gained ? 1 : 0, null);
        // The soft keyboard follows focus for a canvas that took the whole keyboard, the way it
        // follows a text field's (docs/menus.md "The full route").
        if (DayBridge.nativeHandlesText(id)) {
            InputMethodManager imm = (InputMethodManager) getContext()
                    .getSystemService(Context.INPUT_METHOD_SERVICE);
            if (imm == null) return;
            if (gained) imm.showSoftInput(this, 0);
            else {
                // A composition in progress goes with the focus: the input method is told
                // to start over, so it does not commit into whatever is focused next.
                imm.restartInput(this);
                imm.hideSoftInputFromWindow(getWindowToken(), 0);
            }
        }
    }

    // A hardware key while this canvas has focus. A key the route does not carry, and every
    // key when the app registered no handler for the node, goes to `super`, so an unclaimed
    // arrow still moves focus between views.
    @Override public boolean onKeyDown(int keyCode, KeyEvent ev) {
        if (sendKey(id, keyCode, ev)) return true;
        return super.onKeyDown(keyCode, ev);
    }

    /** Deliver a hardware key to node `id` on whichever route it took (docs/menus.md), and
     *  say whether it was claimed. Shared with the focusable containers. */
    static boolean sendKey(long id, int keyCode, KeyEvent ev) {
        if (!DayBridge.nativeHandlesKeys(id)) return false;
        if (!DayBridge.nativeHandlesText(id)) {
            String name = keyName(keyCode, ev);
            if (name == null) return false;
            DayBridge.nativeOnEvent(id, DayBridge.K_KEY, modifiers(ev), name);
            return true;
        }
        // The full route: a key that types nothing or a chord is a key, what typed is text.
        String name = fullKeyName(keyCode, ev);
        if (name == null) return false;
        int mods = modifiers(ev);
        boolean altgr = ev.isAltPressed() && name.equals(altGrText(ev));
        if (altgr) {
            DayBridge.nativeOnEvent(id, DayBridge.K_TEXT, mods & 1, name);
        } else if (NAMED.contains(name) || (mods & (2 | 4 | 8)) != 0) {
            DayBridge.nativeOnEvent(id, DayBridge.K_KEY, mods, name);
        } else {
            DayBridge.nativeOnEvent(id, DayBridge.K_TEXT, mods, name);
        }
        return true;
    }

    /** The web names of the keys that type nothing (`day_spec::KeyEvent::NAMED`). */
    static final Set<String> NAMED = new HashSet<>(Arrays.asList(
            "ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Enter", "Tab", "Escape", "Backspace",
            "Delete", "Home", "End", "PageUp", "PageDown", "Insert", "F1", "F2", "F3", "F4", "F5", "F6",
            "F7", "F8", "F9", "F10", "F11", "F12"));

    /** The day modifier mask for a key event: shift 1, primary 2, alt 4, control 8. Ctrl is
     *  both the primary key and the Control key here, as on every platform without ⌘. */
    static int modifiers(KeyEvent ev) {
        int mods = 0;
        if (ev.isShiftPressed()) mods |= 1;
        if (ev.isCtrlPressed()) mods |= 2 | 8;
        if (ev.isAltPressed()) mods |= 4;
        return mods;
    }

    /** The day name for a hardware key on the full route (docs/menus.md "The full route"): a
     *  key that types nothing by its web name, one that types text by that text. Under Ctrl or
     *  Alt the unmodified character names it, so Ctrl+C is "c" with the modifier set. Null for
     *  a press that types nothing (a lone modifier, a dead key) and for a Meta chord. */
    static String fullKeyName(int keyCode, KeyEvent ev) {
        switch (keyCode) {
            case KeyEvent.KEYCODE_DPAD_LEFT: return "ArrowLeft";
            case KeyEvent.KEYCODE_DPAD_RIGHT: return "ArrowRight";
            case KeyEvent.KEYCODE_DPAD_UP: return "ArrowUp";
            case KeyEvent.KEYCODE_DPAD_DOWN: return "ArrowDown";
            case KeyEvent.KEYCODE_ENTER: case KeyEvent.KEYCODE_NUMPAD_ENTER: return "Enter";
            case KeyEvent.KEYCODE_TAB: return "Tab";
            case KeyEvent.KEYCODE_ESCAPE: return "Escape";
            case KeyEvent.KEYCODE_DEL: return "Backspace";
            case KeyEvent.KEYCODE_FORWARD_DEL: return "Delete";
            case KeyEvent.KEYCODE_MOVE_HOME: return "Home";
            case KeyEvent.KEYCODE_MOVE_END: return "End";
            case KeyEvent.KEYCODE_PAGE_UP: return "PageUp";
            case KeyEvent.KEYCODE_PAGE_DOWN: return "PageDown";
            case KeyEvent.KEYCODE_INSERT: return "Insert";
            default: break;
        }
        if (keyCode >= KeyEvent.KEYCODE_F1 && keyCode <= KeyEvent.KEYCODE_F12) {
            return "F" + (keyCode - KeyEvent.KEYCODE_F1 + 1);
        }
        if (ev.isMetaPressed()) return null;
        // A layout that types a character under Alt (its AltGr: the @ on a German one) is
        // typing, not chording: that character is the name, and `sendKey` sends it as text.
        String altgr = altGrText(ev);
        if (altgr != null) return altgr;
        int meta = ev.getMetaState() & ~(KeyEvent.META_CTRL_MASK | KeyEvent.META_ALT_MASK);
        int c = ev.getUnicodeChar(meta);
        if (c == 0 || (c & KeyCharacterMap.COMBINING_ACCENT) != 0 || Character.isISOControl(c)) return null;
        return new String(Character.toChars(c));
    }

    /** The character a key types under Alt on this layout when that differs from what it
     *  types without, or null: an Alt that chords types nothing. */
    static String altGrText(KeyEvent ev) {
        if (!ev.isAltPressed() || ev.isCtrlPressed()) return null;
        int with = ev.getUnicodeChar(ev.getMetaState() & ~KeyEvent.META_CTRL_MASK);
        int without = ev.getUnicodeChar(ev.getMetaState() & ~(KeyEvent.META_CTRL_MASK | KeyEvent.META_ALT_MASK));
        if (with == 0 || with == without || (with & KeyCharacterMap.COMBINING_ACCENT) != 0
                || Character.isISOControl(with)) return null;
        return new String(Character.toChars(with));
    }

    // The soft keyboard's way in (docs/menus.md "The full route"): a canvas that took the whole
    // keyboard is an input target whose "text" is the key stream. Raw text, with the
    // suggestions and the auto-correction off: a terminal wants what was typed. The
    // connection keeps the editable the contract asks for, so an input method can compose in
    // it (a Pinyin reading, a word being suggested) the way it would in a field; nothing
    // reaches Day until the composition is committed or finished, and then the committed
    // text goes once, as text. Its delete is Backspace, its action key is Enter, and a key
    // event it synthesizes (an arrow on a keyboard with one) takes the hardware route above.
    @Override public InputConnection onCreateInputConnection(EditorInfo out) {
        if (!DayBridge.nativeHandlesText(id)) return super.onCreateInputConnection(out);
        out.inputType = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
                | InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD;
        out.imeOptions = EditorInfo.IME_ACTION_NONE | EditorInfo.IME_FLAG_NO_EXTRACT_UI
                | EditorInfo.IME_FLAG_NO_FULLSCREEN;
        return new BaseInputConnection(this, true) {
            /** Send what the editable holds, once, and empty it. */
            private void flush() {
                Editable e = getEditable();
                if (e == null) return;
                String t = e.toString();
                e.clear();
                if (!t.isEmpty()) DayBridge.nativeOnEvent(id, DayBridge.K_TEXT, 0, t);
            }
            @Override public boolean commitText(CharSequence text, int newCursorPosition) {
                // The contract: the composing region is replaced by the text. Let the base
                // class edit the editable, then deliver whatever it holds.
                boolean ok = super.commitText(text, newCursorPosition);
                flush();
                return ok;
            }
            @Override public boolean finishComposingText() {
                // The composition became final without a commit of its own.
                boolean ok = super.finishComposingText();
                flush();
                return ok;
            }
            @Override public boolean deleteSurroundingText(int before, int after) {
                // With nothing composing there is nothing to delete here: the deletes are
                // key presses for the piece. Under a composition the base class edits it.
                Editable e = getEditable();
                if (e != null && e.length() > 0) return super.deleteSurroundingText(before, after);
                for (int i = 0; i < before; i++) {
                    DayBridge.nativeOnEvent(id, DayBridge.K_KEY, 0, "Backspace");
                }
                for (int i = 0; i < after; i++) {
                    DayBridge.nativeOnEvent(id, DayBridge.K_KEY, 0, "Delete");
                }
                return true;
            }
            @Override public boolean deleteSurroundingTextInCodePoints(int before, int after) {
                Editable e = getEditable();
                if (e != null && e.length() > 0) return super.deleteSurroundingTextInCodePoints(before, after);
                return deleteSurroundingText(before, after);
            }
            @Override public boolean performEditorAction(int action) {
                flush();
                DayBridge.nativeOnEvent(id, DayBridge.K_KEY, 0, "Enter");
                return true;
            }
            @Override public boolean sendKeyEvent(KeyEvent ev) {
                // A key the input method sends as an event (its Enter, a hardware-style
                // key): the same route a physical press takes, after any composition.
                if (ev.getAction() == KeyEvent.ACTION_DOWN) { flush(); return onKeyDown(ev.getKeyCode(), ev); }
                return true;
            }
        };
    }

    @Override public boolean onCheckIsTextEditor() {
        return DayBridge.nativeHandlesText(id);
    }

    /** The day name for a key the route carries (docs/menus.md), or null. Android draws no
     *  menu bar, so no accelerator owns the delete keys and they ride the route too. A digit
     *  is named by what it types, main row or keypad, and never under Ctrl, Alt or Meta,
     *  which belong to shortcuts. */
    static String keyName(int keyCode, KeyEvent ev) {
        switch (keyCode) {
            case KeyEvent.KEYCODE_DPAD_LEFT: return "ArrowLeft";
            case KeyEvent.KEYCODE_DPAD_RIGHT: return "ArrowRight";
            case KeyEvent.KEYCODE_DPAD_UP: return "ArrowUp";
            case KeyEvent.KEYCODE_DPAD_DOWN: return "ArrowDown";
            case KeyEvent.KEYCODE_DEL: return "Backspace";
            case KeyEvent.KEYCODE_FORWARD_DEL: return "Delete";
            default: break;
        }
        if (ev.isCtrlPressed() || ev.isAltPressed() || ev.isMetaPressed()) return null;
        int c = ev.getUnicodeChar();
        return c >= '0' && c <= '9' ? String.valueOf((char) c) : null;
    }

    public void setOps(double[] n, String joined) {
        nums = n;
        texts = splitTexts(joined);
        invalidate();
    }

    /** The texts channel as `setOps` and the exports receive it: one entry per record that
     *  carries text, joined with U+001F. Empties are kept, since the channel is positional. */
    static String[] splitTexts(String joined) {
        return joined == null || joined.isEmpty() ? new String[0] : joined.split("\u001F", -1);
    }

    // ---- Export (docs/canvas.md "Exporting") ------------------------------------------------

    /** The platform's words for the last export that failed, read by Rust after a false or
     *  null answer. Exports run on the UI thread only, so one slot is enough. */
    private static String exportError = "";

    public static String exportError() {
        return exportError;
    }

    /** Render a display list into a new transparent ARGB_8888 bitmap of `pw` x `ph` pixels,
     *  the canvas scaled by `scale` so the ops' points land where the screen puts them, and
     *  store it under `id` in {@link DayBridge#bitmaps}, the table decoded bitmaps live in. So
     *  `imageEncode`, `imageRelease`, the image view and the canvas image op all take it. False
     *  when the platform refuses, with the reason in {@link #exportError}. */
    public static boolean renderBitmap(long id, double[] nums, String textsJoined, int pw, int ph,
            double scale) {
        try {
            Bitmap bmp = Bitmap.createBitmap(pw, ph, Bitmap.Config.ARGB_8888);
            Canvas cv = new Canvas(bmp);
            cv.scale((float) scale, (float) scale);
            new OpPainter().draw(cv, nums, splitTexts(textsJoined));
            DayBridge.bitmaps.put(id, bmp);
            return true;
        } catch (RuntimeException | OutOfMemoryError e) {
            // An exception thrown back across JNI would abort the process; the allocation is
            // the likely failure (a large bitmap against the per-app heap).
            exportError = String.valueOf(e);
            return false;
        }
    }

    /** Write one PDF page per entry of `numCounts`: page i replays the next `numCounts[i]`
     *  numbers and `textCounts[i]` texts, on a `sizes[2i]` x `sizes[2i+1]` page. PdfDocument
     *  pages are in PDF points already, so the canvas is not scaled, and they take whole points
     *  only. PdfDocument has no metadata API, so a title or author cannot be written. Null when
     *  the writer fails, with the reason in {@link #exportError}. */
    public static byte[] renderPdf(double[] nums, int[] numCounts, String textsJoined,
            int[] textCounts, double[] sizes) {
        String[] all = splitTexts(textsJoined);
        PdfDocument doc = new PdfDocument();
        try {
            // One painter for the document, so a path drawn on several pages parses once.
            OpPainter painter = new OpPainter();
            int ni = 0, ti = 0;
            for (int i = 0; i < numCounts.length; i++) {
                int w = Math.max(1, (int) Math.round(sizes[2 * i]));
                int h = Math.max(1, (int) Math.round(sizes[2 * i + 1]));
                PdfDocument.Page page =
                        doc.startPage(new PdfDocument.PageInfo.Builder(w, h, i + 1).create());
                double[] pageNums = java.util.Arrays.copyOfRange(
                        nums, Math.min(ni, nums.length), Math.min(ni + numCounts[i], nums.length));
                String[] pageTexts = java.util.Arrays.copyOfRange(
                        all, Math.min(ti, all.length), Math.min(ti + textCounts[i], all.length));
                ni += numCounts[i];
                ti += textCounts[i];
                painter.draw(page.getCanvas(), pageNums, pageTexts);
                doc.finishPage(page);
            }
            java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
            doc.writeTo(out);
            return out.toByteArray();
        } catch (java.io.IOException | RuntimeException | OutOfMemoryError e) {
            exportError = String.valueOf(e);
            return null;
        } finally {
            doc.close();
        }
    }

    /** This view's painter, kept across frames so its Paint and path cache are reused. */
    private final OpPainter painter = new OpPainter();

    @Override protected void onDraw(Canvas cv) {
        float density = getResources().getDisplayMetrics().density;
        cv.save();
        cv.scale(density, density);
        painter.draw(cv, nums, texts);
        cv.restore();
    }

    /** The one painter behind the screen and the exports (docs/canvas.md "Exporting"): it replays
     *  an encoded display list onto any Canvas, in canvas points (dp). The caller scales the
     *  canvas first: by the density for {@link #onDraw}, by the export scale for
     *  {@link #renderBitmap}, and not at all for a {@link #renderPdf} page, which is already in
     *  points. Everything a record leaves pending for the next one lives here, so an export
     *  never shares it with a view drawing at the same time. */
    static final class OpPainter {
        final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
        // A decoded kind-18 record (stroke style), applied to the next stroke record only.
        private boolean stylePending = false;
        private int sCap = 0, sJoin = 0;
        private float sMiter = 10f, sPhase = 0f;
        private float[] sDash = null;
        // A decoded kind-19 record (font), applied to the next text record only (docs/fonts.md).
        private boolean fontPending = false;
        private int fWeight = 0;
        private boolean fItalic = false;
        private String fFamily = "";
        // A decoded kind-23 record (image source): the region, in the bitmap's own pixels, the
        // next image record draws instead of the whole bitmap.
        private boolean srcPending = false;
        private float isx = 0f, isy = 0f, isw = 0f, ish = 0f;

        /** Decoded geometry, keyed by the encoder's content key (day_spec::geometry_key).
         *
         *  Paths and polygons are the only ops whose geometry arrives as TEXT, and re-parsing it was
         *  this backend's per-frame cost: a drawing re-recorded every frame re-parsed every segment of
         *  every path, float by float. The key IS the content, so an entry can never go stale — the
         *  cap exists only to stop a long-lived view from holding every path it has ever drawn. Key 0
         *  means the encoder offered none, and that geometry is parsed as it always was. */
        private final java.util.HashMap<Long, android.graphics.Path> pathCache = new java.util.HashMap<>();

        private android.graphics.Path cachedPath(double key, String spec, int rule) {
            long k = (long) key;
            if (k == 0) return parsePath(spec, rule);
            android.graphics.Path hit = pathCache.get(k);
            if (hit != null) return hit;
            android.graphics.Path built = parsePath(spec, rule);
            if (pathCache.size() > 512) pathCache.clear();
            pathCache.put(k, built);
            return built;
        }

        /** The polygon counterpart: "x,y x,y …" as one closed path, cached the same way. Returns an
         *  EMPTY path when the payload held no points, which is what the callers already test for. */
        private android.graphics.Path cachedPolygon(double key, String pts) {
            long k = (long) key;
            if (k != 0) {
                android.graphics.Path hit = pathCache.get(k);
                if (hit != null) return hit;
            }
            android.graphics.Path built = new android.graphics.Path();
            boolean first = true;
            for (String pair : pts.split(" ")) {
                int comma = pair.indexOf(',');
                if (comma <= 0) continue;
                try {
                    float x = Float.parseFloat(pair.substring(0, comma));
                    float y = Float.parseFloat(pair.substring(comma + 1));
                    if (first) { built.moveTo(x, y); first = false; } else { built.lineTo(x, y); }
                } catch (NumberFormatException nfe) {
                    android.util.Log.w("Day", "canvas point parse failed: " + pair, nfe);
                }
            }
            if (!first) built.close();
            if (k != 0) {
                if (pathCache.size() > 512) pathCache.clear();
                pathCache.put(k, built);
            }
            return built;
        }

        // A decoded kind-14 record (set-gradient): type (0 linear, 1 radial) + unit geometry +
        // parsed stops, applied as the paint's shader for the next fill-shape record (resolved
        // against that shape's bounds).
        private boolean gradPending = false;
        private int gradType = 0;
        private float gsx, gsy, gex, gey;
        private int[] gradColors = new int[0];
        private float[] gradOffsets = new float[0];

        /** Install the pending gradient shader for a fill over `bounds`; caller clears it after. */
        private void applyGradient(RectF bounds) {
            android.graphics.Shader shader;
            if (gradType == 1) {
                // Radial, elliptical-to-bounds: circular in unit space (a,b = center gsx,gsy,
                // c = radius gex), stretched onto the bounds by the shader's local matrix.
                android.graphics.RadialGradient rg = new android.graphics.RadialGradient(
                        gsx, gsy, Math.max(gex, 1e-4f),
                        gradColors, gradOffsets, android.graphics.Shader.TileMode.CLAMP);
                Matrix m = new Matrix();
                m.setScale(bounds.width(), bounds.height());
                m.postTranslate(bounds.left, bounds.top);
                rg.setLocalMatrix(m);
                shader = rg;
            } else {
                shader = new android.graphics.LinearGradient(
                        bounds.left + gsx * bounds.width(), bounds.top + gsy * bounds.height(),
                        bounds.left + gex * bounds.width(), bounds.top + gey * bounds.height(),
                        gradColors, gradOffsets, android.graphics.Shader.TileMode.CLAMP);
            }
            paint.setShader(shader);
            gradPending = false;
        }

        /** Replay `nums` (nine numbers a record) and `texts` (one entry per record that carries
         *  text, in order) onto `cv`. */
        void draw(Canvas cv, double[] nums, String[] texts) {
            // Pending records never carry over from the previous draw.
            stylePending = false;
            fontPending = false;
            int ti = 0;
            gradPending = false;
            // A decoded kind-20 record (stamp): the positions the next shape record is drawn at, once
            // each. Empty means the ordinary one-shape-one-record case (docs/canvas.md "Stamping").
            java.util.ArrayList<float[]> stampAt = new java.util.ArrayList<float[]>();
            int stampN = 0;
            for (int i = 0; i + 8 < nums.length; i += 9) {
                int k = (int) nums[i];
                float a = (float) nums[i+1], b = (float) nums[i+2], c = (float) nums[i+3], d = (float) nums[i+4];
                float e = (float) nums[i+5], f = (float) nums[i+6], g = (float) nums[i+7];
                long col = (long) nums[i+8];
                paint.setColor((int) col);
                // Day's default cap is BUTT (this view used to force ROUND); a kind-18 record
                // overrides cap/join/miter/dash for the one stroke that follows it.
                if (stylePending) {
                    paint.setStrokeCap(sCap == 1 ? Paint.Cap.ROUND : sCap == 2 ? Paint.Cap.SQUARE : Paint.Cap.BUTT);
                    paint.setStrokeJoin(sJoin == 1 ? Paint.Join.ROUND : sJoin == 2 ? Paint.Join.BEVEL : Paint.Join.MITER);
                    paint.setStrokeMiter(sMiter);
                    paint.setPathEffect(sDash != null ? new android.graphics.DashPathEffect(sDash, sPhase) : null);
                } else {
                    paint.setStrokeCap(Paint.Cap.BUTT);
                    paint.setStrokeJoin(Paint.Join.MITER);
                    paint.setStrokeMiter(10f);
                    paint.setPathEffect(null);
                }
                if (!gradPending) paint.setShader(null);
                // Stamp prefix and its coordinate records: collected, never drawn on their own.
                if (k == 20) { stampAt.clear(); stampN = (int) a; continue; }
                if (k == 21) {
                    // Four points per record, in slot pairs (1,2) (3,4) (5,6) (7,8) — the fourth
                    // point's y rides the slot other records use for their color. The last record of a
                    // run is padded with zeros, so the header's count is what says where the real ones
                    // stop.
                    float[] xs = { a, c, e, g };
                    float[] ys = { b, d, f, (float) nums[i+8] };
                    for (int q = 0; q < 4 && stampAt.size() < stampN; q++) {
                        stampAt.add(new float[] { xs[q], ys[q] });
                    }
                    continue;
                }
                // The template is replayed once per position under a translated canvas. `ti` is
                // rewound each time so a template with a texts payload (a polygon, a path) reads the
                // Same entry every repetition and consumes it exactly once overall.
                int reps = stampAt.isEmpty() ? 1 : stampAt.size();
                int tiStart = ti;
                for (int rep = 0; rep < reps; rep++) {
                ti = tiStart;
                if (!stampAt.isEmpty()) { cv.save(); cv.translate(stampAt.get(rep)[0], stampAt.get(rep)[1]); }
                switch (k) {
                    case 0: paint.setStyle(Paint.Style.FILL);
                            if (gradPending) applyGradient(new RectF(a, b, a+c, b+d));
                            cv.drawRect(a, b, a+c, b+d, paint);
                            paint.setShader(null); break;
                    case 1: paint.setStyle(Paint.Style.STROKE); paint.setStrokeWidth(g); cv.drawRect(a, b, a+c, b+d, paint); break;
                    case 2: {
                        paint.setStyle(Paint.Style.FILL);
                        RectF r2 = new RectF(a, b, a+c, b+d);
                        if (gradPending) applyGradient(r2);
                        cv.drawRoundRect(r2, e, e, paint);
                        paint.setShader(null); break;
                    }
                    case 13: paint.setStyle(Paint.Style.STROKE); paint.setStrokeWidth(g); cv.drawRoundRect(new RectF(a, b, a+c, b+d), e, e, paint); break;
                    case 3: {
                        paint.setStyle(Paint.Style.FILL);
                        RectF r3 = new RectF(a, b, a+c, b+d);
                        if (gradPending) applyGradient(r3);
                        cv.drawOval(r3, paint);
                        paint.setShader(null); break;
                    }
                    case 4: paint.setStyle(Paint.Style.STROKE); paint.setStrokeWidth(g); cv.drawOval(new RectF(a, b, a+c, b+d), paint); break;
                    case 5: paint.setStyle(Paint.Style.STROKE); paint.setStrokeWidth(g);
                            cv.drawArc(new RectF(a, b, a+c, b+d), e, f, false, paint); break;
                    case 6: paint.setStyle(Paint.Style.STROKE); paint.setStrokeWidth(g); cv.drawLine(a, b, c, d, paint); break;
                    case 7: { // text at (a,b); e=size, f=anchor packed as h*4+v (TextAnchor::pack)
                        String t = ti < texts.length ? texts[ti++] : "";
                        paint.setStyle(Paint.Style.FILL);
                        paint.setTextSize(e);
                        paint.setTypeface(fontPending ? DayBridge.canvasTypeface(fFamily, fWeight, fItalic) : null);
                        // drawText takes the BASELINE; the anchor positions the line box
                        // (ascent + descent, Skia-style: ascent negative), the box measureText
                        // reports — the same arithmetic as TextAnchor::offset in day-spec.
                        Paint.FontMetrics fm = paint.getFontMetrics();
                        int ah = (int) f / 4, av = (int) f % 4;
                        float x = a, y = b - fm.ascent;
                        if (ah != 0) {
                            float w = paint.measureText(t);
                            x = a + (ah == 1 ? -w / 2f : -w);
                        }
                        if (av == 1) {
                            y = b - (fm.ascent + fm.descent) / 2f;
                        } else if (av == 2) {
                            y = b; // `at` IS the baseline
                        } else if (av == 3) {
                            y = b - fm.descent;
                        }
                        cv.drawText(t, x, y, paint);
                        paint.setTypeface(null);
                        break;
                    }
                    case 19: { // font for the next text: a weight (0 default), b italic; family on texts
                        fFamily = ti < texts.length ? texts[ti++] : "";
                        fWeight = (int) a;
                        fItalic = b > 0.5f;
                        fontPending = true;
                        break;
                    }
                    case 8: cv.save(); break;
                    case 9: cv.restore(); break;
                    case 10: {
                        // Packed affine (a,b,c,d,tx,ty) → Android Matrix (row-major 3x3); same
                        // row-vector meaning. Applied within the density-scaled space (dp units).
                        Matrix m = new Matrix();
                        m.setValues(new float[]{a, c, e, b, d, f, 0f, 0f, 1f});
                        cv.concat(m);
                        break;
                    }
                    case 11: case 12: { // polygon (11 fill / 12 stroke); points ride texts as "x,y x,y …"
                        // The texts entry is consumed either way: that channel is positional, so a
                        // cache hit that skipped it would desynchronize every record after this one.
                        String t = ti < texts.length ? texts[ti++] : "";
                        android.graphics.Path path = cachedPolygon(a, t);
                        if (!path.isEmpty()) {
                            if (k == 11) {
                                paint.setStyle(Paint.Style.FILL);
                                if (gradPending) {
                                    RectF pb = new RectF();
                                    path.computeBounds(pb, true);
                                    applyGradient(pb);
                                }
                            } else {
                                paint.setStyle(Paint.Style.STROKE);
                                paint.setStrokeWidth(g);
                            }
                            cv.drawPath(path, paint);
                            paint.setShader(null);
                        }
                        break;
                    }
                    case 15: case 16: { // path (15 fill / 16 stroke); segments ride texts, f = fill rule
                        String t = ti < texts.length ? texts[ti++] : "";
                        android.graphics.Path path = cachedPath(a, t, (int) f);
                        if (k == 15) {
                            paint.setStyle(Paint.Style.FILL);
                            if (gradPending) {
                                RectF pb = new RectF();
                                path.computeBounds(pb, true);
                                applyGradient(pb);
                            }
                        } else {
                            paint.setStyle(Paint.Style.STROKE);
                            paint.setStrokeWidth(g);
                            if (gradPending) {
                                RectF pb = new RectF();
                                path.computeBounds(pb, true);
                                applyGradient(pb);
                            }
                        }
                        cv.drawPath(path, paint);
                        paint.setShader(null);
                        break;
                    }
                    case 17: { // clip: f names the shape, a..d geometry, e radius or fill rule
                        android.graphics.Path clip = new android.graphics.Path();
                        switch ((int) f) {
                            case 1: clip.addRoundRect(new RectF(a, b, a+c, b+d), e, e, android.graphics.Path.Direction.CW); break;
                            case 2: clip.addOval(new RectF(a, b, a+c, b+d), android.graphics.Path.Direction.CW); break;
                            // These two REPLACE the path rather than adding to it, so the cached
                            // object is never mutated afterwards — `cv.clipPath` only reads it.
                            case 3: clip = cachedPath(a, ti < texts.length ? texts[ti++] : "", (int) e); break;
                            case 4: clip = cachedPolygon(a, ti < texts.length ? texts[ti++] : ""); break;
                            default: clip.addRect(a, b, a+c, b+d, android.graphics.Path.Direction.CW); break;
                        }
                        // Canvas.clipPath intersects with the current clip, which is the spec's rule.
                        cv.clipPath(clip);
                        break;
                    }
                    case 18: { // stroke style for the next stroke: a cap, b join, c miter, d phase
                        String t = ti < texts.length ? texts[ti++] : "";
                        sCap = (int) a; sJoin = (int) b; sMiter = c; sPhase = d;
                        String[] parts = t.trim().isEmpty() ? new String[0] : t.split(" ");
                        // DashPathEffect needs an even count of at least two entries; an odd
                        // pattern repeats to become even, which is what every other backend does.
                        float[] dash = null;
                        if (parts.length > 0) {
                            int n = parts.length % 2 == 0 ? parts.length : parts.length * 2;
                            dash = new float[n];
                            boolean ok = true;
                            for (int q = 0; q < n; q++) {
                                try { dash[q] = Float.parseFloat(parts[q % parts.length]); }
                                catch (NumberFormatException nfe) { ok = false; break; }
                            }
                            if (!ok) dash = null;
                        }
                        sDash = dash;
                        stylePending = true;
                        break;
                    }
                    case 14: { // set-gradient (f = type): stops ride texts as "offset,aarrggbb …"
                        String t = ti < texts.length ? texts[ti++] : "";
                        gradType = (int) f;
                        String[] parts = t.split(" ");
                        int[] colors = new int[parts.length];
                        float[] offsets = new float[parts.length];
                        int n = 0;
                        for (String pair : parts) {
                            int comma = pair.indexOf(',');
                            if (comma <= 0) continue;
                            try {
                                offsets[n] = Float.parseFloat(pair.substring(0, comma));
                                colors[n] = (int) Long.parseLong(pair.substring(comma + 1), 16);
                                n++;
                            } catch (NumberFormatException nfe) {
                                android.util.Log.w("Day", "gradient stop parse failed: " + pair, nfe);
                            }
                        }
                        if (n >= 2) {
                            gradColors = java.util.Arrays.copyOf(colors, n);
                            gradOffsets = java.util.Arrays.copyOf(offsets, n);
                            gsx = a; gsy = b; gex = c; gey = d;
                            gradPending = true;
                        }
                        break;
                    }
                    case 22: { // image: a,b origin · c,d size · e the BitmapId · f opacity (docs/images.md)
                        android.graphics.Bitmap bmp = DayBridge.bitmapFor((long) e);
                        // A released bitmap draws nothing rather than a placeholder: the canvas
                        // re-records on every tracked read, so a handle can be dropped between the
                        // record and this replay.
                        if (bmp != null && c > 0 && d > 0) {
                            // Its OWN Paint rather than the shared one: an alpha set here would
                            // otherwise leak into later records, and the shared paint carries a
                            // shader and stroke state this op has no business touching. Images are
                            // rare in a display list, so the allocation is not on a hot path.
                            Paint ip = new Paint(Paint.FILTER_BITMAP_FLAG);
                            ip.setAlpha((int) Math.round(Math.max(0f, Math.min(1f, f)) * 255f));
                            RectF dst = new RectF(a, b, a + c, b + d);
                            Rect src = null;
                            if (srcPending) {
                                // The region is in the bitmap's pixels, clamped to it with its
                                // mapping into the rect kept (day_spec::clamp_image_source); an
                                // empty intersection draws nothing. `drawBitmap` takes the source
                                // as an int Rect, so its edges round to whole pixels.
                                float x0 = Math.max(0f, isx), y0 = Math.max(0f, isy);
                                float x1 = Math.min(bmp.getWidth(), isx + isw);
                                float y1 = Math.min(bmp.getHeight(), isy + ish);
                                if (!(isw > 0f && ish > 0f && x1 > x0 && y1 > y0)) break;
                                float kx = c / isw, ky = d / ish;
                                dst = new RectF(a + (x0 - isx) * kx, b + (y0 - isy) * ky,
                                                a + (x1 - isx) * kx, b + (y1 - isy) * ky);
                                src = new Rect(Math.round(x0), Math.round(y0), Math.round(x1), Math.round(y1));
                            }
                            cv.drawBitmap(bmp, src, dst, ip);
                        }
                        break;
                    }
                    case 23: { // image source for the next image: a,b origin · c,d size, in pixels
                        isx = a; isy = b; isw = c; ish = d;
                        srcPending = true;
                        break;
                    }
                }
                if (!stampAt.isEmpty()) cv.restore();
                }
                stampAt.clear();
                // A style record applies to one stroke, so anything else clears it; a font record
                // likewise applies to one text.
                if (k != 18) stylePending = false;
                if (k != 19) fontPending = false;
                if (k != 23) srcPending = false;
            }
        }
    }

    /** Parse "M x y L x y Q .. C .. Z" (day_spec::encode_path) into an Android Path. */
    private static android.graphics.Path parsePath(String spec, int rule) {
        android.graphics.Path path = new android.graphics.Path();
        path.setFillType(rule == 1 ? android.graphics.Path.FillType.EVEN_ODD
                                   : android.graphics.Path.FillType.WINDING);
        String[] tok = spec.trim().isEmpty() ? new String[0] : spec.split(" ");
        int i = 0;
        try {
            while (i < tok.length) {
                String op = tok[i++];
                if (op.equals("M") && i + 1 < tok.length) {
                    path.moveTo(Float.parseFloat(tok[i++]), Float.parseFloat(tok[i++]));
                } else if (op.equals("L") && i + 1 < tok.length) {
                    path.lineTo(Float.parseFloat(tok[i++]), Float.parseFloat(tok[i++]));
                } else if (op.equals("Q") && i + 3 < tok.length) {
                    path.quadTo(Float.parseFloat(tok[i++]), Float.parseFloat(tok[i++]),
                                Float.parseFloat(tok[i++]), Float.parseFloat(tok[i++]));
                } else if (op.equals("C") && i + 5 < tok.length) {
                    path.cubicTo(Float.parseFloat(tok[i++]), Float.parseFloat(tok[i++]),
                                 Float.parseFloat(tok[i++]), Float.parseFloat(tok[i++]),
                                 Float.parseFloat(tok[i++]), Float.parseFloat(tok[i++]));
                } else if (op.equals("Z")) {
                    path.close();
                }
            }
        } catch (NumberFormatException nfe) {
            // Draw what parsed rather than dropping the frame.
            android.util.Log.w("Day", "canvas path parse failed", nfe);
        }
        return path;
    }
}
