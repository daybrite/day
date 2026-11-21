// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.view.View;
import android.view.ViewGroup;
import java.util.HashMap;

/** Absolute-positioning ViewGroup (the GtkFixed / flipped-NSView analogue). day's layout engine
 *  computes every child's rect in px and calls setChildFrame; this places them verbatim. When
 *  measured `UNSPECIFIED` (inside a ScrollView) it reports the content size set by day (§7.6). */
public class DayFixed extends ViewGroup {
    private final HashMap<View, int[]> frames = new HashMap<>();
    long panNode = 0;
    /** Navigation-page presentation, read by the host before the root is shown. */
    boolean immersivePage = false;
    @Override public boolean dispatchGenericMotionEvent(android.view.MotionEvent e) {
        if (panNode != 0 && DayBridge.horizontalPan(this, panNode, e)) return true;
        return super.dispatchGenericMotionEvent(e);
    }
    /** Inside a disabled group (docs/forms.md "Disabling a group") no touch reaches the
     *  children; the press goes on to whatever is around the group. */
    @Override public boolean onInterceptTouchEvent(android.view.MotionEvent e) {
        return DayBridge.underDisabledGroup(this) || super.onInterceptTouchEvent(e);
    }
    private int contentW = 0, contentH = 0;

    public DayFixed(Context c) { super(c); }

    /** Size-change hook (set by DayActivity): first fire starts native, later fires resize it. */
    public interface SizeListener { void onSize(int w, int h); }
    public SizeListener sizeListener;

    @Override protected void onSizeChanged(int w, int h, int oldW, int oldH) {
        super.onSizeChanged(w, h, oldW, oldH);
        if (sizeListener != null) sizeListener.onSize(w, h);
    }

    public void setChildFrame(View v, int x, int y, int w, int h) {
        // Day recomputes every rect on a relayout and hands back the ones that did not move, so
        // most calls here ask for the frame the child already has. Re-measuring and scheduling a
        // pass for those is pure churn, and when the request arrives during a layout pass (day's
        // engine runs off onSizeChanged), a requestLayout() also makes Android run a whole second
        // pass and log "requestLayout() improperly called ... during layout".
        int[] prev = frames.get(v);
        if (prev != null && prev[0] == x && prev[1] == y && prev[2] == w && prev[3] == h
                && v.getLeft() == x && v.getTop() == y
                && v.getWidth() == w && v.getHeight() == h) {
            return;
        }
        frames.put(v, new int[]{x, y, w, h});
        v.measure(MeasureSpec.makeMeasureSpec(w, MeasureSpec.EXACTLY),
                  MeasureSpec.makeMeasureSpec(h, MeasureSpec.EXACTLY));
        if (isInLayout()) {
            // Place it now, exactly as onLayout would from the same frames map: the position is
            // already final, so a second pass would only arrive at it again.
            v.layout(x, y, x + w, y + h);
        } else {
            requestLayout();
        }
    }

    /** The frame `v` was last placed at (px: x, y, w, h), or null before its first placement:
     *  where an animated frame change starts from. */
    int[] childFrame(View v) {
        int[] f = frames.get(v);
        return f == null ? null : f.clone();
    }

    public void setContentSize(int w, int h) {
        if (contentW == w && contentH == h) return;
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

    // Outline clipping (`setClipToOutline`, a `corner_radius` or `clips` surface) is a property
    // of the hardware render node: a software canvas, which is what `DayBridge.windowImage`
    // draws into, ignores it, so `day::window_image()` showed rounded views square. Clip to the
    // outline by hand there. `draw` covers the view's own background; `dispatchDraw` the
    // children of a view with no background, whose `draw` Android skips.
    private final android.graphics.Outline outline = new android.graphics.Outline();
    private final android.graphics.Rect outlineRect = new android.graphics.Rect();
    private final android.graphics.Path outlinePath = new android.graphics.Path();

    private boolean clipSoftware(android.graphics.Canvas canvas) {
        if (canvas.isHardwareAccelerated() || !getClipToOutline()) return false;
        android.view.ViewOutlineProvider provider = getOutlineProvider();
        if (provider == null) return false;
        outline.setEmpty();
        provider.getOutline(this, outline);
        if (!outline.getRect(outlineRect)) return false; // not a (rounded) rect: leave it
        float r = outline.getRadius();
        outlinePath.reset();
        outlinePath.addRoundRect(outlineRect.left, outlineRect.top, outlineRect.right,
                outlineRect.bottom, r, r, android.graphics.Path.Direction.CW);
        canvas.save();
        canvas.clipPath(outlinePath);
        return true;
    }

    @Override public void draw(android.graphics.Canvas canvas) {
        // The shadow lies outside the frame, so it is drawn before any clip to the outline.
        if (styled && shadowColor != 0) drawShadow(canvas);
        boolean clipped = clipSoftware(canvas);
        if (styled) drawFill(canvas);
        super.draw(canvas);
        if (clipped) canvas.restore();
    }

    @Override protected void dispatchDraw(android.graphics.Canvas canvas) {
        boolean clipped = clipSoftware(canvas);
        // Named corners have no outline a render node can clip to before API 33 (an outline
        // clips as a rect or as one radius), so that shape clips the children as a path.
        boolean shaped = styled && clips && !uniform;
        if (shaped) {
            shape();
            canvas.save();
            canvas.clipPath(outer);
        }
        super.dispatchDraw(canvas);
        if (shaped) canvas.restore();
        // Over the content, the way a CALayer's border is: a child that fills the frame (a
        // photo in a rounded surface) does not cover it.
        if (styled && borderWidth > 0f) drawBorder(canvas);
        if (clipped) canvas.restore();
    }

    // ---- Styled surface (docs/surfaces.md) -----------------------------------------------------
    // A surface with a gradient, a border, named corners or a shadow. The plain ones (a color,
    // one radius) stay the GradientDrawable background `DayBridge.setSurface` gives them; this
    // one is drawn here because no stock drawable has all four: GradientDrawable takes a
    // gradient only at 45-degree steps and stop offsets only from API 29, and nothing but the
    // view itself can draw a shadow outside its own frame.
    private boolean styled, clips, uniform;
    private boolean hasFill;
    private int fillColor, borderColor, shadowColor;
    private float borderWidth, shadowBlur, shadowDx, shadowDy;
    /** Corner radii as Path wants them: x and y per corner, clockwise from the top left. */
    private final float[] radii = new float[8];
    /** The gradient fill: kind (0 linear, 1 radial, 2 conic), its geometry in the frame's unit
     *  space (linear: start x, y, end x, y; radial: center x, y, radius; conic: center x, y,
     *  start angle in degrees clockwise from +x), and its stops. */
    private int gradKind;
    private final float[] gradGeom = new float[4];
    private int[] gradColors;
    private float[] gradOffsets;
    private android.graphics.Shader gradShader;
    private final android.graphics.Paint surfacePaint =
            new android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG);
    private final android.graphics.Path outer = new android.graphics.Path();
    private final android.graphics.Path ring = new android.graphics.Path();
    private int shapedW = -1, shapedH = -1;
    /** The blurred shadow as an alpha mask, for the hardware canvases that drop mask filters
     *  (before API 28), with the margin it carries around the frame. */
    private android.graphics.Bitmap shadowMask;
    private int shadowMaskPad;

    /** The slots of a styled surface spec before its gradient (src/lib.rs `surface_spec`). */
    private static final int GRADIENT_AT = 12;

    /** Apply a styled surface. `spec` is `surface_spec`'s layout (src/lib.rs), lengths in px
     *  and colors as packed ARGB: [0] has fill, [1] fill, [2..5] radii top-left, top-right,
     *  bottom-right, bottom-left, [6] border color, [7] border width, [8] shadow color (0 =
     *  none), [9] blur, [10] dx, [11] dy, then from [12] the gradient as
     *  {@link #readGradient} reads it. */
    void setSurfaceStyle(double[] spec, boolean clips) {
        if (spec == null || spec.length < GRADIENT_AT + 6) return;
        styled = true;
        this.clips = clips;
        hasFill = spec[0] != 0;
        fillColor = (int) (long) spec[1];
        for (int i = 0; i < 4; i++) {
            float r = Math.max(0f, (float) spec[2 + i]);
            radii[2 * i] = r;
            radii[2 * i + 1] = r;
        }
        borderColor = (int) (long) spec[6];
        borderWidth = (borderColor >>> 24) == 0 ? 0f : Math.max(0f, (float) spec[7]);
        shadowColor = (int) (long) spec[8];
        shadowBlur = Math.max(0f, (float) spec[9]);
        shadowDx = (float) spec[10];
        shadowDy = (float) spec[11];
        readGradient(spec, GRADIENT_AT);
        restyle();
    }

    /** `ContainerPatch::Fill` with a gradient (src/lib.rs `gradient_spec`): the new fill, at
     *  once, keeping the corners, border and shadow. A plain surface (the GradientDrawable
     *  `DayBridge.setSurface` gave it) turns styled, keeping its radius and clip. */
    void setSurfaceGradient(double[] spec) {
        if (spec == null || spec.length < 6) return;
        if (!styled) {
            float r = 0f;
            android.graphics.drawable.Drawable bg = getBackground();
            if (bg instanceof android.graphics.drawable.GradientDrawable) {
                // GradientDrawable.getCornerRadius is API 24, the minimum.
                r = ((android.graphics.drawable.GradientDrawable) bg).getCornerRadius();
                r = Math.max(0f, r);
            }
            clips = getClipToOutline();
            setBackground(null);
            java.util.Arrays.fill(radii, r);
            borderWidth = 0f;
            borderColor = 0;
            shadowColor = 0;
            styled = true;
        }
        hasFill = false;
        readGradient(spec, 0);
        restyle();
    }

    /** Back to a container that paints nothing: what `DayBridge.resetSurface` leaves before a
     *  `ContainerPatch::Surface` applies the new style. */
    void clearSurfaceStyle() {
        if (!styled) return;
        styled = false;
        hasFill = false;
        gradColors = null;
        gradShader = null;
        borderWidth = 0f;
        shadowColor = 0;
        shadowMask = null;
        shapedW = -1;
        invalidate();
    }

    /** The gradient at `spec[at..]`: [0] stop count (0 = none), [1] kind, [2..5] geometry, then
     *  (offset, color) per stop. The stops are padded to span 0 to 1 with the end colors, which
     *  is Day's "flat before the first stop and after the last" on every kind. */
    private void readGradient(double[] spec, int at) {
        int stops = Math.min((int) spec[at], (spec.length - at - 6) / 2);
        if (stops <= 0) {
            gradColors = null;
            return;
        }
        gradKind = (int) spec[at + 1];
        for (int i = 0; i < 4; i++) gradGeom[i] = (float) spec[at + 2 + i];
        float first = (float) spec[at + 6];
        float last = (float) spec[at + 6 + 2 * (stops - 1)];
        boolean head = first > 0f, tail = last < 1f || stops == 1;
        int n = stops + (head ? 1 : 0) + (tail ? 1 : 0);
        gradColors = new int[n];
        gradOffsets = new float[n];
        int k = 0;
        if (head) {
            gradOffsets[k] = 0f;
            gradColors[k++] = (int) (long) spec[at + 7];
        }
        for (int i = 0; i < stops; i++) {
            float o = (float) spec[at + 6 + 2 * i];
            // Offsets never run backwards: a shader given a decreasing one draws garbage.
            gradOffsets[k] = k > 0 ? Math.max(gradOffsets[k - 1], o) : o;
            gradColors[k++] = (int) (long) spec[at + 7 + 2 * i];
        }
        if (tail) {
            gradOffsets[k] = 1f;
            gradColors[k] = gradColors[k - 1];
        }
    }

    /** Re-derive what the drawing reads after a style change. */
    private void restyle() {
        uniform = radii[0] == radii[2] && radii[2] == radii[4] && radii[4] == radii[6];
        shapedW = -1;
        shadowMask = null;
        // A ViewGroup with no background skips `draw`; this one paints there.
        setWillNotDraw(false);
        if (clips && uniform) {
            final float radius = radii[0];
            setOutlineProvider(new android.view.ViewOutlineProvider() {
                @Override public void getOutline(View view, android.graphics.Outline o) {
                    o.setRoundRect(0, 0, view.getWidth(), view.getHeight(), radius);
                }
            });
            setClipToOutline(true);
        } else {
            setOutlineProvider(android.view.ViewOutlineProvider.BACKGROUND);
            setClipToOutline(false);
        }
        if (shadowColor != 0 && isAttachedToWindow()) unclipAncestors();
        invalidate();
    }

    /** A new fill color for a styled surface (`ContainerPatch::Fill` with a color, or one
     *  frame of its animation), keeping its border, corners and shadow; it replaces a gradient
     *  fill. False when this view is not one. */
    boolean setSurfaceFill(int argb, boolean has) {
        if (!styled) return false;
        hasFill = has;
        fillColor = argb;
        if (gradColors != null) {
            gradColors = null;
            gradShader = null;
            shapedW = -1;
        }
        invalidate();
        return true;
    }

    /** Whether this is a styled surface, which paints its own fill. */
    boolean isStyled() { return styled; }

    /** A styled surface's fill color, or null when it has none. */
    Integer styledFill() { return styled && hasFill ? fillColor : null; }

    /** The opaque color this surface fills with, or 0: what a list reads for its backdrop. */
    int opaqueFill() {
        return styled && hasFill && gradColors == null && (fillColor >>> 24) == 0xFF ? fillColor : 0;
    }

    @Override protected void onAttachedToWindow() {
        super.onAttachedToWindow();
        if (styled && shadowColor != 0) unclipAncestors();
    }

    /** Let the shadow show outside this view. A parent clips each child to the child's own
     *  frame unless told not to, and is itself clipped by its parent the same way, so the
     *  flag goes up through Day's containers until one that clips its content on purpose, or
     *  a native container (a scroll view, a pager, a list), which keeps its edge. */
    private void unclipAncestors() {
        android.view.ViewParent p = getParent();
        for (boolean first = true; p instanceof ViewGroup; first = false) {
            ViewGroup g = (ViewGroup) p;
            boolean ours = g instanceof DayFixed;
            if (!first && !ours) return;
            g.setClipChildren(false);
            if (!ours || g.getClipToOutline() || (((DayFixed) g).styled && ((DayFixed) g).clips)) return;
            p = g.getParent();
        }
    }

    /** Rebuild the surface's paths and gradient for the current size. */
    private void shape() {
        int w = getWidth(), h = getHeight();
        if (w == shapedW && h == shapedH) return;
        shapedW = w;
        shapedH = h;
        outer.rewind();
        outer.addRoundRect(0, 0, w, h, radii, android.graphics.Path.Direction.CW);
        // The border is the band between the frame and the same shape drawn `borderWidth`
        // in: it lies wholly inside the frame, and its inner corners tighten by the width,
        // as a CSS border's do.
        ring.rewind();
        if (borderWidth > 0f) {
            ring.setFillType(android.graphics.Path.FillType.EVEN_ODD);
            ring.addRoundRect(0, 0, w, h, radii, android.graphics.Path.Direction.CW);
            float b = borderWidth;
            if (w > 2 * b && h > 2 * b) {
                float[] inner = new float[8];
                for (int i = 0; i < 8; i++) inner[i] = Math.max(0f, radii[i] - b);
                ring.addRoundRect(b, b, w - b, h - b, inner, android.graphics.Path.Direction.CW);
            }
        }
        gradShader = gradColors == null ? null : gradient(w, h);
        shadowMask = null;
    }

    /** The gradient fill's shader for a `w` by `h` frame: Android's own LinearGradient,
     *  RadialGradient (scaled to an ellipse in a frame that is not square, the radius a
     *  fraction of each side) or SweepGradient (turned to start at the start angle; Android's
     *  sweep, like Day's conic, runs clockwise from +x on screen). */
    private android.graphics.Shader gradient(int w, int h) {
        android.graphics.Shader.TileMode clamp = android.graphics.Shader.TileMode.CLAMP;
        float[] g = gradGeom;
        switch (gradKind) {
            case 1: {
                float cx = g[0] * w, cy = g[1] * h, r = g[2] * w;
                // A radius of nothing (or a frame of no width) is its last color throughout.
                if (!(r > 0f) || h <= 0) {
                    return new android.graphics.LinearGradient(0, 0, 1, 0,
                            gradColors[gradColors.length - 1], gradColors[gradColors.length - 1],
                            clamp);
                }
                android.graphics.RadialGradient s = new android.graphics.RadialGradient(
                        cx, cy, r, gradColors, gradOffsets, clamp);
                android.graphics.Matrix m = new android.graphics.Matrix();
                m.setScale(1f, h / (float) w, cx, cy);
                s.setLocalMatrix(m);
                return s;
            }
            case 2: {
                float cx = g[0] * w, cy = g[1] * h;
                android.graphics.SweepGradient s = new android.graphics.SweepGradient(
                        cx, cy, gradColors, gradOffsets);
                android.graphics.Matrix m = new android.graphics.Matrix();
                m.setRotate(g[2], cx, cy);
                s.setLocalMatrix(m);
                return s;
            }
            default:
                return new android.graphics.LinearGradient(g[0] * w, g[1] * h, g[2] * w, g[3] * h,
                        gradColors, gradOffsets, clamp);
        }
    }

    private void drawFill(android.graphics.Canvas canvas) {
        if (gradColors == null && !hasFill) return;
        shape();
        surfacePaint.setStyle(android.graphics.Paint.Style.FILL);
        surfacePaint.setMaskFilter(null);
        surfacePaint.setColor(gradShader != null ? 0xFFFFFFFF : fillColor);
        surfacePaint.setShader(gradShader);
        canvas.drawPath(outer, surfacePaint);
        surfacePaint.setShader(null);
    }

    private void drawBorder(android.graphics.Canvas canvas) {
        shape();
        surfacePaint.setStyle(android.graphics.Paint.Style.FILL);
        surfacePaint.setMaskFilter(null);
        surfacePaint.setShader(null);
        surfacePaint.setColor(borderColor);
        canvas.drawPath(ring, surfacePaint);
    }

    /** The surface's shape, blurred, in the shadow's color, moved by its offset. The blur is
     *  the platform's Gaussian mask filter; Day's blur is twice its sigma. */
    private void drawShadow(android.graphics.Canvas canvas) {
        shape();
        float radius = DayCanvasView.blurRadius(shadowBlur / 2f);
        surfacePaint.setStyle(android.graphics.Paint.Style.FILL);
        surfacePaint.setShader(null);
        surfacePaint.setColor(shadowColor);
        if (radius > 0f && canvas.isHardwareAccelerated() && android.os.Build.VERSION.SDK_INT < 28) {
            // That canvas ignores mask filters: blur into a mask on a software canvas once
            // per size, and draw the mask in the shadow's color.
            if (shadowMask == null) {
                int pad = (int) Math.ceil(shadowBlur * 1.5f) + 2;
                android.graphics.Bitmap mask;
                try {
                    mask = android.graphics.Bitmap.createBitmap(Math.max(1, getWidth() + 2 * pad),
                            Math.max(1, getHeight() + 2 * pad), android.graphics.Bitmap.Config.ALPHA_8);
                } catch (RuntimeException | OutOfMemoryError e) {
                    return; // no shadow rather than no surface
                }
                android.graphics.Canvas into = new android.graphics.Canvas(mask);
                android.graphics.Paint p = new android.graphics.Paint(android.graphics.Paint.ANTI_ALIAS_FLAG);
                p.setMaskFilter(new android.graphics.BlurMaskFilter(
                        radius, android.graphics.BlurMaskFilter.Blur.NORMAL));
                into.translate(pad, pad);
                into.drawPath(outer, p);
                shadowMask = mask;
                shadowMaskPad = pad;
            }
            surfacePaint.setMaskFilter(null);
            canvas.drawBitmap(shadowMask, shadowDx - shadowMaskPad, shadowDy - shadowMaskPad, surfacePaint);
            return;
        }
        surfacePaint.setMaskFilter(radius > 0f ? new android.graphics.BlurMaskFilter(
                radius, android.graphics.BlurMaskFilter.Blur.NORMAL) : null);
        canvas.save();
        canvas.translate(shadowDx, shadowDy);
        canvas.drawPath(outer, surfacePaint);
        canvas.restore();
        surfacePaint.setMaskFilter(null);
    }

    @Override protected void onLayout(boolean changed, int l, int t, int r, int b) {
        for (int i = 0; i < getChildCount(); i++) {
            View c = getChildAt(i);
            int[] f = frames.get(c);
            if (f != null) c.layout(f[0], f[1], f[0] + f[2], f[1] + f[3]);
        }
    }
}
