// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Matrix;
import android.graphics.Paint;
import android.graphics.RectF;
import android.view.View;

/** Replays day's display list (§11). Ops arrive dp-encoded; drawing scales by density. */
public class DayCanvasView extends View {
    double[] nums = new double[0];
    String[] texts = new String[0];
    final Paint paint = new Paint(Paint.ANTI_ALIAS_FLAG);
    // A decoded kind-18 record (stroke style), applied to the NEXT stroke record only.
    private boolean stylePending = false;
    private int sCap = 0, sJoin = 0;
    private float sMiter = 10f, sPhase = 0f;
    private float[] sDash = null;

    public DayCanvasView(Context c) { super(c); }

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
                case 7: {
                    String t = ti < texts.length ? texts[ti++] : "";
                    paint.setStyle(Paint.Style.FILL);
                    paint.setTextSize(e);
                    float x = a, y = b;
                    if (f > 0.5f) {
                        x -= paint.measureText(t) / 2f;
                        y += (paint.getFontMetrics().descent - paint.getFontMetrics().ascent) / 2f
                                - paint.getFontMetrics().descent;
                    }
                    cv.drawText(t, x, y, paint);
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
            // A style record applies to ONE stroke, so anything else clears it.
            if (k != 18) stylePending = false;
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
