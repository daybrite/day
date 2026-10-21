// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The day-piece-settings crate's Android row (docs/settings.md): a Material 3 list item in the
// preference-screen idiom. One class draws every line of a settings page: the preference row
// itself (headline, supporting text, an optional leading icon, a trailing chevron on a row that
// pushes a page), the category header above a group of rows, the footnote under one, and the
// large About header the root list starts with. Bundled here and folded into the app's Gradle
// build via [package.metadata.day.android]. The row is a DayFixed, so the Day control the Rust
// side mounts as its child (the real switch, spinner, slider or field, carrying the row's element
// id) is placed by day's layout engine exactly like any other Day child; the texts and the icon
// are the row's own, drawn from offscreen-measured TextViews and a Drawable so they never shift
// the index of a Day child. A tappable row wears the theme's `selectableItemBackground` ripple;
// day wires the click itself (`enableGesture`) when the Rust side attaches a tap, and a choice
// row forwards its click to the spinner it hosts, so the whole row opens the choice the way a
// ListPreference does. Uses only day-android's public Java surface.
package dev.daybrite.day.piece.settings;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.drawable.Drawable;
import android.util.TypedValue;
import android.view.View;
import android.view.ViewGroup;
import android.widget.ImageView;
import android.widget.TextView;

import com.google.android.material.color.MaterialColors;

import dev.daybrite.day.bridge.DayBridge;
import dev.daybrite.day.bridge.DayFixed;

/** One line of a settings page: a preference row, a category header, a footnote or the About
 *  header. Texts and icon leading (drawn natively), a Day child or a chevron trailing. */
public final class DaySettingsRow extends DayFixed {
    /** Row styles (the Rust side's `Style`). */
    static final int STYLE_ITEM = 0, STYLE_CATEGORY = 1, STYLE_NOTE = 2, STYLE_HEADER = 3;
    /** Flag bits (the Rust side's `flags`). */
    static final int F_CHEVRON = 1, F_DESTRUCTIVE = 2, F_RIPPLE = 4, F_FORWARD = 8,
            F_TEMPLATE_ICON = 16, F_FIRST = 32;
    /** M3 list-item metrics, in dp (lib-android.rs holds the same numbers). */
    static final float PAD_H = 16f, GAP = 16f, CHEVRON = 24f, TEXT_GAP = 2f, ICON = 24f,
            HEADER_ICON = 56f, HEADER_ICON_RADIUS = 12f;
    static final float ITEM_PAD_V = 8f, HEADER_PAD_V = 16f, CATEGORY_TOP = 24f,
            CATEGORY_TOP_FIRST = 8f, CATEGORY_BOTTOM = 8f, NOTE_TOP = 8f, NOTE_BOTTOM = 12f;

    private final TextView headline;
    private final TextView supporting;
    private final Drawable icon;
    private final int style;
    private final int flags;
    private final Paint chevronPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final Path chevronPath = new Path();
    private final float density;

    private DaySettingsRow(Context c, String title, String caption, String iconName, int style,
                           int flags, boolean enabled) {
        super(c);
        this.style = style;
        this.flags = flags;
        density = c.getResources().getDisplayMetrics().density;
        setWillNotDraw(false);

        int onSurface = MaterialColors.getColor(c,
                com.google.android.material.R.attr.colorOnSurface, 0xFF000000);
        int onSurfaceVariant = MaterialColors.getColor(c,
                com.google.android.material.R.attr.colorOnSurfaceVariant, 0xFF444444);
        int primary = MaterialColors.getColor(c, androidx.appcompat.R.attr.colorPrimary, 0xFF6750A4);
        int error = MaterialColors.getColor(c, androidx.appcompat.R.attr.colorError, 0xFFB3261E);

        headline = new TextView(c);
        supporting = new TextView(c);
        // The texts are drawn by hand and never added as children, so they get the params a parent
        // would give them: a later setText (a language change) reads them and threw without.
        for (TextView t : new TextView[]{headline, supporting}) {
            t.setLayoutParams(new ViewGroup.LayoutParams(
                    ViewGroup.LayoutParams.WRAP_CONTENT, ViewGroup.LayoutParams.WRAP_CONTENT));
        }
        switch (style) {
            case STYLE_CATEGORY:
                // The preference category: a title-small line in the primary color.
                applyAppearance(headline, com.google.android.material.R.attr.textAppearanceTitleSmall);
                headline.setTextColor(primary);
                break;
            case STYLE_HEADER:
                applyAppearance(headline, com.google.android.material.R.attr.textAppearanceTitleLarge);
                headline.setTextColor(onSurface);
                break;
            default:
                applyAppearance(headline, com.google.android.material.R.attr.textAppearanceBodyLarge);
                headline.setTextColor((flags & F_DESTRUCTIVE) != 0 ? error : onSurface);
        }
        applyAppearance(supporting, com.google.android.material.R.attr.textAppearanceBodyMedium);
        supporting.setTextColor(onSurfaceVariant);
        setTexts(title, caption);

        // The leading glyph: a staged drawable (a group's vector, the app icon, a Material Symbol
        // of day-android's), through the bridge's own resolution. A template glyph takes the
        // list item's icon color; the app icon keeps its authored colors.
        Drawable d = null;
        if (iconName != null && !iconName.isEmpty()) {
            View iv = DayBridge.makeImage(iconName, 0, 0);
            if (iv instanceof ImageView) d = ((ImageView) iv).getDrawable();
            if (d != null) {
                d = d.mutate();
                if ((flags & F_TEMPLATE_ICON) != 0) d.setTint(onSurfaceVariant);
            }
        }
        icon = d;

        chevronPaint.setStyle(Paint.Style.STROKE);
        chevronPaint.setStrokeWidth(2f * density);
        chevronPaint.setStrokeCap(Paint.Cap.ROUND);
        chevronPaint.setStrokeJoin(Paint.Join.ROUND);
        chevronPaint.setColor(onSurfaceVariant);

        if ((flags & (F_RIPPLE | F_FORWARD)) != 0) {
            // The whole row is the tap target: the themed ripple says so. Day wires the click
            // itself when the Rust side attaches a tap; a forwarding row clicks its child.
            TypedValue tv = new TypedValue();
            if (c.getTheme().resolveAttribute(android.R.attr.selectableItemBackground, tv, true)) {
                Drawable ripple = c.getDrawable(tv.resourceId);
                if (ripple != null) setForeground(ripple);
            }
        }
        if ((flags & F_FORWARD) != 0) {
            setClickable(true);
            setOnClickListener(new View.OnClickListener() {
                public void onClick(View v) {
                    for (int i = 0; i < getChildCount(); i++) {
                        View child = getChildAt(i);
                        if (child.getVisibility() != GONE && child.isEnabled()) {
                            child.performClick();
                            return;
                        }
                    }
                }
            });
        }
        setRowEnabled(enabled);
    }

    private static void applyAppearance(TextView t, int attr) {
        TypedValue tv = new TypedValue();
        if (t.getContext().getTheme().resolveAttribute(attr, tv, true) && tv.resourceId != 0) {
            t.setTextAppearance(tv.resourceId);
        }
    }

    // --- the factory and commands the Rust arm calls (lib-android.rs) ---

    public static View makeRow(String title, String caption, String icon, int style, int flags,
                               boolean enabled) {
        return new DaySettingsRow(DayBridge.ctx, title, caption, icon, style, flags, enabled);
    }

    public static void setRowTexts(View v, String title, String caption) {
        if (v instanceof DaySettingsRow) ((DaySettingsRow) v).setTexts(title, caption);
    }

    public static void setRowEnabled(View v, boolean on) {
        if (v instanceof DaySettingsRow) ((DaySettingsRow) v).setRowEnabled(on);
    }

    /** The natural (unwrapped) width of the text block, in dp. */
    public static float rowTextWidth(View v) {
        if (!(v instanceof DaySettingsRow)) return 0f;
        DaySettingsRow r = (DaySettingsRow) v;
        return Math.max(r.natural(r.headline), r.natural(r.supporting)) / r.density;
    }

    /** The text block's height when wrapped at `wDp`, in dp (0 when both lines are empty). */
    public static float rowTextHeight(View v, float wDp) {
        if (!(v instanceof DaySettingsRow)) return 0f;
        DaySettingsRow r = (DaySettingsRow) v;
        return r.textHeight(Math.max(0, Math.round(wDp * r.density))) / r.density;
    }

    // --- the row itself ---

    private void setTexts(String title, String caption) {
        headline.setText(title);
        headline.setVisibility(title.isEmpty() ? GONE : VISIBLE);
        supporting.setText(caption);
        supporting.setVisibility(caption.isEmpty() ? GONE : VISIBLE);
        setContentDescription(title.isEmpty() ? caption
                : caption.isEmpty() ? title : title + ". " + caption);
        invalidate();
    }

    private void setRowEnabled(boolean on) {
        // M3 disabled content: 38% opacity on the texts and the icon; the Day control greys
        // itself. A disabled row neither ripples nor fires its click.
        float alpha = on ? 1f : 0.38f;
        headline.setAlpha(alpha);
        supporting.setAlpha(alpha);
        if (icon != null) icon.setAlpha(on ? 255 : 97);
        setEnabled(on);
        invalidate();
    }

    private int natural(TextView t) {
        if (t.getVisibility() == GONE) return 0;
        t.measure(MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED),
                  MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED));
        return t.getMeasuredWidth();
    }

    /** Measures both texts wrapped at `wPx` and returns the block height (lines plus gap). */
    private int textHeight(int wPx) {
        int h = 0;
        int lines = 0;
        for (TextView t : new TextView[]{headline, supporting}) {
            if (t.getVisibility() == GONE) continue;
            t.measure(MeasureSpec.makeMeasureSpec(wPx, MeasureSpec.AT_MOST),
                      MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED));
            h += t.getMeasuredHeight();
            lines++;
        }
        if (lines == 2) h += Math.round(TEXT_GAP * density);
        return h;
    }

    private float iconSize() {
        return style == STYLE_HEADER ? HEADER_ICON : ICON;
    }

    /** Where the text column starts: past the padding and the icon slot. */
    private int textLeft() {
        int x = Math.round(PAD_H * density);
        if (icon != null) x += Math.round((iconSize() + GAP) * density);
        return x;
    }

    /** The text column's right edge: short of a Day child that shares its vertical band, and of
     *  the chevron. */
    private int textRight(int textTop, int textBottom) {
        int right = getWidth() - Math.round(PAD_H * density);
        if ((flags & F_CHEVRON) != 0) right -= Math.round((CHEVRON + GAP) * density);
        int gap = Math.round(GAP * density);
        for (int i = 0; i < getChildCount(); i++) {
            View c = getChildAt(i);
            if (c.getVisibility() == GONE) continue;
            boolean overlaps = c.getTop() < textBottom && c.getBottom() > textTop;
            if (overlaps) right = Math.min(right, c.getLeft() - gap);
        }
        return right;
    }

    /** The top padding of a header-style line (a category, a footnote). */
    private int lineTop() {
        float dp = style == STYLE_CATEGORY
                ? ((flags & F_FIRST) != 0 ? CATEGORY_TOP_FIRST : CATEGORY_TOP)
                : NOTE_TOP;
        return Math.round(dp * density);
    }

    @Override protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        int padV = Math.round((style == STYLE_HEADER ? HEADER_PAD_V : ITEM_PAD_V) * density);
        int left = textLeft();
        boolean chevron = (flags & F_CHEVRON) != 0;
        // First pass at the full width to find the band the texts occupy, then narrow the
        // column past any Day child beside it and lay the lines out for real.
        int fullW = Math.max(0, getWidth() - left - Math.round(PAD_H * density)
                - (chevron ? Math.round((CHEVRON + GAP) * density) : 0));
        int th = textHeight(fullW);
        boolean beside = false;
        for (int i = 0; i < getChildCount(); i++) {
            View c = getChildAt(i);
            if (c.getVisibility() != GONE && c.getTop() < padV + th) { beside = true; break; }
        }
        int right = textRight(padV, padV + th);
        int w = Math.max(0, right - left);
        th = textHeight(w);
        int y;
        if (style == STYLE_CATEGORY || style == STYLE_NOTE) {
            y = lineTop();
        } else {
            // Beside a control (or alone): centered in the row. Above a stacked control: at the top.
            boolean centered = beside || getChildCount() == 0;
            y = centered ? Math.max(padV, (getHeight() - th) / 2) : padV;
        }
        boolean rtl = getLayoutDirection() == LAYOUT_DIRECTION_RTL;
        int textTop = y;
        for (TextView t : new TextView[]{headline, supporting}) {
            if (t.getVisibility() == GONE) continue;
            int tw = Math.min(w, t.getMeasuredWidth());
            int x = rtl ? getWidth() - left - tw : left;
            t.layout(0, 0, tw, t.getMeasuredHeight());
            canvas.save();
            canvas.translate(x, y);
            t.draw(canvas);
            canvas.restore();
            y += t.getMeasuredHeight() + Math.round(TEXT_GAP * density);
        }
        if (icon != null) {
            // Centered on the text block, in the leading slot.
            int size = Math.round(iconSize() * density);
            int iconLeft = rtl ? getWidth() - Math.round(PAD_H * density) - size
                    : Math.round(PAD_H * density);
            int iconTop = textTop + (th - size) / 2;
            if (style == STYLE_HEADER) iconTop = Math.max(padV, (getHeight() - size) / 2);
            icon.setBounds(iconLeft, iconTop, iconLeft + size, iconTop + size);
            if (style == STYLE_HEADER) {
                // The app icon as a rounded tile, the launcher's shape.
                float r = HEADER_ICON_RADIUS * density;
                canvas.save();
                Path clip = new Path();
                clip.addRoundRect(iconLeft, iconTop, iconLeft + size, iconTop + size, r, r, Path.Direction.CW);
                canvas.clipPath(clip);
                icon.draw(canvas);
                canvas.restore();
            } else {
                icon.draw(canvas);
            }
        }
        if (chevron) {
            float size = CHEVRON * density;
            float cx = rtl ? Math.round(PAD_H * density) + size / 2f
                    : getWidth() - Math.round(PAD_H * density) - size / 2f;
            float cy = getHeight() / 2f;
            float half = 5f * density;
            float dir = rtl ? -1f : 1f;
            chevronPath.reset();
            chevronPath.moveTo(cx - dir * half / 2f, cy - half);
            chevronPath.lineTo(cx + dir * half / 2f, cy);
            chevronPath.lineTo(cx - dir * half / 2f, cy + half);
            canvas.drawPath(chevronPath, chevronPaint);
        }
    }
}
