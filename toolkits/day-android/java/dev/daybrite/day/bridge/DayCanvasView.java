// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Matrix;
import android.graphics.Paint;
import android.graphics.Rect;
import android.graphics.RectF;
import android.os.Build;
import android.view.KeyEvent;
import android.view.MotionEvent;
import android.view.View;

/** Replays day's display list (§11). Ops arrive dp-encoded; drawing scales by density. */
public class DayCanvasView extends View {
    /** The day node this canvas renders: focus and keys are reported against it. */
    final long id;
    double[] nums = new double[0];
    String[] texts = new String[0];
    final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    // A decoded kind-18 record (stroke style), applied to the NEXT stroke record only.
    private boolean stylePending = false;
    private int sCap = 0, sJoin = 0;
    private float sMiter = 10f, sPhase = 0f;
    private float[] sDash = null;
    // A decoded kind-19 record (font), applied to the NEXT text record only (docs/fonts.md).
    private boolean fontPending = false;
    private int fWeight = 0;
    private boolean fItalic = false;
    private String fFamily = "";

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
    }

    // A hardware key while this canvas has focus. A key the route does not carry, and every
    // key when the app registered no handler for the node, goes to `super`, so an unclaimed
    // arrow still moves focus between views.
    @Override public boolean onKeyDown(int keyCode, KeyEvent ev) {
        String name = keyName(keyCode, ev);
        if (name != null && DayBridge.nativeHandlesKeys(id)) {
            int mods = 0;
            if (ev.isShiftPressed()) mods |= 1; // day KeyEvent::SHIFT
            if (ev.isCtrlPressed()) mods |= 2;  // PRIMARY
            if (ev.isAltPressed()) mods |= 4;   // ALT
            DayBridge.nativeOnEvent(id, DayBridge.K_KEY, mods, name);
            return true;
        }
        return super.onKeyDown(keyCode, ev);
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
        texts = joined.isEmpty() ? new String[0] : joined.split("\u001F", -1); // keep empties: one per record
        invalidate();
    }

    // A decoded kind-14 record (set-gradient): type (0 linear, 1 radial) + unit geometry +
    // parsed stops, applied as the paint's shader for the NEXT fill-shape record (resolved
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

    @Override protected void onDraw(Canvas cv) {
        float density = getResources().getDisplayMetrics().density;
        cv.save();
        cv.scale(density, density);
        int ti = 0;
        gradPending = false;
        // A decoded kind-20 record (stamp): the positions the NEXT shape record is drawn at, once
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
                // point's y rides the slot other records use for their color. The LAST record of a
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
            // SAME entry every repetition and consumes it exactly once overall.
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
                case 19: { // font for the NEXT text: a weight (0 default), b italic; family on texts
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
                    String t = ti < texts.length ? texts[ti++] : "";
                    android.graphics.Path path = new android.graphics.Path();
                    boolean first = true;
                    for (String pair : t.split(" ")) {
                        int comma = pair.indexOf(',');
                        if (comma <= 0) continue;
                        try {
                            float x = Float.parseFloat(pair.substring(0, comma));
                            float y = Float.parseFloat(pair.substring(comma + 1));
                            if (first) { path.moveTo(x, y); first = false; } else { path.lineTo(x, y); }
                        } catch (NumberFormatException nfe) {
                            android.util.Log.w("Day", "canvas point parse failed: " + pair, nfe);
                        }
                    }
                    if (!first) {
                        path.close();
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
                    android.graphics.Path path = parsePath(t, (int) f);
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
                        case 3: clip = parsePath(ti < texts.length ? texts[ti++] : "", (int) e); break;
                        case 4: {
                            String tp = ti < texts.length ? texts[ti++] : "";
                            boolean first = true;
                            for (String pair : tp.split(" ")) {
                                int comma = pair.indexOf(',');
                                if (comma <= 0) continue;
                                try {
                                    float x = Float.parseFloat(pair.substring(0, comma));
                                    float y = Float.parseFloat(pair.substring(comma + 1));
                                    if (first) { clip.moveTo(x, y); first = false; } else { clip.lineTo(x, y); }
                                } catch (NumberFormatException nfe) {
                                    android.util.Log.w("Day", "clip point parse failed: " + pair, nfe);
                                }
                            }
                            if (!first) clip.close();
                            break;
                        }
                        default: clip.addRect(a, b, a+c, b+d, android.graphics.Path.Direction.CW); break;
                    }
                    // Canvas.clipPath intersects with the current clip, which is the spec's rule.
                    cv.clipPath(clip);
                    break;
                }
                case 18: { // stroke style for the NEXT stroke: a cap, b join, c miter, d phase
                    String t = ti < texts.length ? texts[ti++] : "";
                    sCap = (int) a; sJoin = (int) b; sMiter = c; sPhase = d;
                    String[] parts = t.trim().isEmpty() ? new String[0] : t.split(" ");
                    // DashPathEffect needs an EVEN count of at least two entries; an odd
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
            }
            if (!stampAt.isEmpty()) cv.restore();
            }
            stampAt.clear();
            // A style record applies to ONE stroke, so anything else clears it; a font record
            // likewise applies to one text.
            if (k != 18) stylePending = false;
            if (k != 19) fontPending = false;
        }
        cv.restore();
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
