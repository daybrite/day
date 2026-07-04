package dev.daybrite.day.bridge;

import android.content.Context;
import android.view.View;
import android.view.ViewGroup;
import java.util.HashMap;

/** Absolute-positioning ViewGroup (the GtkFixed / flipped-NSView analogue). day's layout engine
 *  computes every child's rect in px and calls setChildFrame; this places them verbatim. When
 *  measured UNSPECIFIED (inside a ScrollView) it reports the content size set by day (§7.6). */
public class DayFixed extends ViewGroup {
    private final HashMap<View, int[]> frames = new HashMap<>();
    private int contentW = 0, contentH = 0;

    public DayFixed(Context c) { super(c); }

    public void setChildFrame(View v, int x, int y, int w, int h) {
        frames.put(v, new int[]{x, y, w, h});
        v.measure(MeasureSpec.makeMeasureSpec(w, MeasureSpec.EXACTLY),
                  MeasureSpec.makeMeasureSpec(h, MeasureSpec.EXACTLY));
        requestLayout();
    }

    public void setContentSize(int w, int h) {
        contentW = w;
        contentH = h;
        requestLayout();
    }

    @Override protected void onMeasure(int wSpec, int hSpec) {
        int w = MeasureSpec.getMode(wSpec) == MeasureSpec.UNSPECIFIED
                ? contentW : MeasureSpec.getSize(wSpec);
        int h = MeasureSpec.getMode(hSpec) == MeasureSpec.UNSPECIFIED
                ? contentH : MeasureSpec.getSize(hSpec);
        setMeasuredDimension(w, h);
        for (int i = 0; i < getChildCount(); i++) {
            View c = getChildAt(i);
            int[] f = frames.get(c);
            if (f != null) {
                c.measure(MeasureSpec.makeMeasureSpec(f[2], MeasureSpec.EXACTLY),
                          MeasureSpec.makeMeasureSpec(f[3], MeasureSpec.EXACTLY));
            }
        }
    }

    @Override protected void onLayout(boolean changed, int l, int t, int r, int b) {
        for (int i = 0; i < getChildCount(); i++) {
            View c = getChildAt(i);
            int[] f = frames.get(c);
            if (f != null) c.layout(f[0], f[1], f[0] + f[2], f[1] + f[3]);
        }
    }
}
