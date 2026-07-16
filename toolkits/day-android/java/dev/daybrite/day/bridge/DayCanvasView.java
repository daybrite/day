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
            paint.setStrokeCap(Paint.Cap.ROUND);
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
        }
        cv.restore();
    }
}
