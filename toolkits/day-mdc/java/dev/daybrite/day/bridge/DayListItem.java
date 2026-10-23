// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.content.res.ColorStateList;
import android.graphics.drawable.Drawable;
import android.text.TextUtils;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.widget.ImageView;
import android.widget.LinearLayout;
import android.widget.TextView;

import com.google.android.material.color.MaterialColors;

/**
 * A list row's standard content (docs/list.md "List items"), laid out as a Material 3 list item:
 * a 24dp leading icon in the on-surface-variant color (or the app's tint), the headline in the
 * body-large style, the supporting line in body-medium on-surface-variant, trailing text in
 * label-small, and a trailing check mark in the primary color, 16dp apart. It is Material's
 * height too: 56dp for one line, 72dp with a supporting line, 8dp above and below; the list gives
 * it Material's 16dp side margins (docs/list.md "Row metrics").
 */
public final class DayListItem extends LinearLayout {
    private static final int ICON_DP = 24;
    private static final int GAP_DP = 16;
    private static final int ONE_LINE_DP = 56;
    private static final int TWO_LINE_DP = 72;
    private static final int PAD_V_DP = 8;

    private final ImageView icon;
    private final TextView headline;
    private final TextView supporting;
    private final TextView trailing;
    private final ImageView check;

    private DayListItem(Context c) {
        super(c);
        setOrientation(HORIZONTAL);
        setGravity(Gravity.CENTER_VERTICAL);
        float d = c.getResources().getDisplayMetrics().density;
        setPadding(0, Math.round(PAD_V_DP * d), 0, Math.round(PAD_V_DP * d));
        setMinimumHeight(Math.round(ONE_LINE_DP * d));
        int size = Math.round(ICON_DP * d);
        int gap = Math.round(GAP_DP * d);
        int onSurface = MaterialColors.getColor(c,
                com.google.android.material.R.attr.colorOnSurface, 0xFF1D1B20);
        int onSurfaceVariant = MaterialColors.getColor(c,
                com.google.android.material.R.attr.colorOnSurfaceVariant, 0xFF49454F);

        icon = new ImageView(c);
        icon.setScaleType(ImageView.ScaleType.FIT_CENTER);
        LayoutParams ip = new LayoutParams(size, size);
        ip.setMarginEnd(gap);
        addView(icon, ip);

        LinearLayout texts = new LinearLayout(c);
        texts.setOrientation(VERTICAL);
        headline = new TextView(c);
        applyAppearance(headline, com.google.android.material.R.attr.textAppearanceBodyLarge);
        headline.setTextColor(onSurface);
        headline.setMaxLines(1);
        headline.setEllipsize(TextUtils.TruncateAt.END);
        supporting = new TextView(c);
        applyAppearance(supporting, com.google.android.material.R.attr.textAppearanceBodyMedium);
        supporting.setTextColor(onSurfaceVariant);
        supporting.setMaxLines(2);
        supporting.setEllipsize(TextUtils.TruncateAt.END);
        texts.addView(headline);
        texts.addView(supporting);
        addView(texts, new LayoutParams(0, LayoutParams.WRAP_CONTENT, 1f));

        trailing = new TextView(c);
        applyAppearance(trailing, com.google.android.material.R.attr.textAppearanceLabelSmall);
        trailing.setTextColor(onSurfaceVariant);
        LayoutParams tp = new LayoutParams(LayoutParams.WRAP_CONTENT, LayoutParams.WRAP_CONTENT);
        tp.setMarginStart(gap);
        addView(trailing, tp);

        check = new ImageView(c);
        Drawable mark = DayBridge.drawableByName(c, "day_symbol_check");
        if (mark != null) {
            mark = mark.mutate();
            mark.setTint(MaterialColors.getColor(c,
                    androidx.appcompat.R.attr.colorPrimary, 0xFF6750A4));
            check.setImageDrawable(mark);
        }
        LayoutParams cp = new LayoutParams(size, size);
        cp.setMarginStart(gap);
        addView(check, cp);
    }

    private static void applyAppearance(TextView t, int attr) {
        TypedValue tv = new TypedValue();
        if (t.getContext().getTheme().resolveAttribute(attr, tv, true) && tv.resourceId != 0) {
            t.setTextAppearance(tv.resourceId);
        }
    }

    private static void show(TextView t, String text) {
        t.setText(text);
        t.setVisibility(text.isEmpty() ? GONE : VISIBLE);
    }

    // --- the factory and commands the Rust arm calls (day-mdc lib.rs) ---

    public static View make(String title, String subtitle, String detail, String iconName,
            int tint, boolean checked) {
        DayListItem item = new DayListItem(DayBridge.ctx);
        setTexts(item, title, subtitle, detail);
        setIcon(item, iconName, tint);
        setChecked(item, checked);
        return item;
    }

    public static void setTexts(View v, String title, String subtitle, String detail) {
        if (!(v instanceof DayListItem)) return;
        DayListItem item = (DayListItem) v;
        show(item.headline, title);
        show(item.supporting, subtitle);
        show(item.trailing, detail);
        float d = item.getResources().getDisplayMetrics().density;
        item.setMinimumHeight(Math.round((subtitle.isEmpty() ? ONE_LINE_DP : TWO_LINE_DP) * d));
    }

    /** `tint` is ARGB; 0 takes Material's on-surface-variant, the color of a leading icon. */
    public static void setIcon(View v, String name, int tint) {
        if (!(v instanceof DayListItem)) return;
        DayListItem item = (DayListItem) v;
        Drawable d = name.isEmpty() ? null : DayBridge.drawableByName(item.getContext(), name);
        if (d != null) {
            d = d.mutate();
            int color = tint != 0 ? tint : MaterialColors.getColor(item.getContext(),
                    com.google.android.material.R.attr.colorOnSurfaceVariant, 0xFF49454F);
            d.setTintList(ColorStateList.valueOf(color));
        }
        item.icon.setImageDrawable(d);
        item.icon.setVisibility(d == null ? GONE : VISIBLE);
    }

    /** The trailing check mark, announced as the row's selected state. */
    public static void setChecked(View v, boolean on) {
        if (!(v instanceof DayListItem)) return;
        DayListItem item = (DayListItem) v;
        item.check.setVisibility(on ? VISIBLE : GONE);
        item.setSelected(on);
    }
}
