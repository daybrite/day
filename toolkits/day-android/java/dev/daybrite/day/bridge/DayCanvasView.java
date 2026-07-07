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

    @Override protected void onDraw(Canvas cv) {
        float density = getResources().getDisplayMetrics().density;
        cv.save();
        cv.scale(density, density);
        int ti = 0;
        for (int i = 0; i + 8 < nums.length; i += 9) {
            int k = (int) nums[i];
            float a = (float) nums[i+1], b = (float) nums[i+2], c = (float) nums[i+3], d = (float) nums[i+4];
            float e = (float) nums[i+5], f = (float) nums[i+6], g = (float) nums[i+7];
            long col = (long) nums[i+8];
            paint.setColor((int) col);
            paint.setStrokeCap(Paint.Cap.ROUND);
            switch (k) {
                case 0: paint.setStyle(Paint.Style.FILL); cv.drawRect(a, b, a+c, b+d, paint); break;
                case 1: paint.setStyle(Paint.Style.STROKE); paint.setStrokeWidth(g); cv.drawRect(a, b, a+c, b+d, paint); break;
                case 2: paint.setStyle(Paint.Style.FILL); cv.drawRoundRect(new RectF(a, b, a+c, b+d), e, e, paint); break;
                case 13: paint.setStyle(Paint.Style.STROKE); paint.setStrokeWidth(g); cv.drawRoundRect(new RectF(a, b, a+c, b+d), e, e, paint); break;
                case 3: paint.setStyle(Paint.Style.FILL); cv.drawOval(new RectF(a, b, a+c, b+d), paint); break;
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
                        } catch (NumberFormatException ignored) {}
                    }
                    if (!first) {
                        path.close();
                        if (k == 11) {
                            paint.setStyle(Paint.Style.FILL);
                        } else {
                            paint.setStyle(Paint.Style.STROKE);
                            paint.setStrokeWidth(g);
                        }
                        cv.drawPath(path, paint);
                    }
                    break;
                }
            }
        }
        cv.restore();
    }
}
