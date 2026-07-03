package dev.day.bridge;

import android.content.Context;
import android.graphics.Typeface;
import android.view.Gravity;
import android.view.View;
import android.view.ViewGroup;
import android.widget.FrameLayout;
import android.widget.LinearLayout;
import android.widget.TextView;
import java.util.ArrayList;

/**
 * Tabs host (docs/tabs.md): a top tab strip (row of tab buttons) over a page
 * {@link FrameLayout} that shows the selected page. Tapping a tab dispatches event kind 4
 * (SelectionChanged); the Rust route controller owns day's state and drives selection back via
 * {@link #select}. Android's native tabbed containers (TabLayout / BottomNavigationView) live
 * in the Material library; day stays dependency-free with an equivalent strip built from the
 * framework widgets, all tabs resident so each keeps its own state.
 */
public class DayTabs extends LinearLayout {
    final long hostNode;
    private final LinearLayout bar;
    private final FrameLayout pages;
    private final ArrayList<View> tabViews = new ArrayList<>();
    private final ArrayList<TextView> tabButtons = new ArrayList<>();
    private int selected;

    public DayTabs(Context ctx, long hostNode, int initial) {
        super(ctx);
        setOrientation(VERTICAL);
        this.hostNode = hostNode;
        this.selected = Math.max(0, initial);

        bar = new LinearLayout(ctx);
        bar.setOrientation(HORIZONTAL);
        addView(bar, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT,
                ViewGroup.LayoutParams.WRAP_CONTENT));

        pages = new FrameLayout(ctx);
        addView(pages, new LayoutParams(ViewGroup.LayoutParams.MATCH_PARENT, 0, 1f));
    }

    /** Append a tab (in insertion order) carrying `page` under the label `title`. */
    void addTab(View page, String title) {
        final int index = tabViews.size();
        TextView btn = new TextView(getContext());
        btn.setText(title == null ? "" : title);
        btn.setGravity(Gravity.CENTER);
        btn.setAllCaps(true);
        btn.setTextSize(13f);
        int pad = dp(12);
        btn.setPadding(pad, pad, pad, pad);
        btn.setClickable(true);
        btn.setOnClickListener(new OnClickListener() {
            @Override public void onClick(View v) {
                select(index);
                DayBridge.nativeOnEvent(hostNode, 4, (double) index, null); // 4 = SelectionChanged
            }
        });
        bar.addView(btn, new LinearLayout.LayoutParams(0,
                ViewGroup.LayoutParams.WRAP_CONTENT, 1f));
        tabButtons.add(btn);

        pages.addView(page, new FrameLayout.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));
        tabViews.add(page);
        refresh();
    }

    /** Show tab `index` (from a native tap or a programmatic TabsPatch::Selected). */
    void select(int index) {
        if (index < 0 || index >= tabViews.size()) return;
        selected = index;
        refresh();
    }

    private void refresh() {
        for (int i = 0; i < tabViews.size(); i++) {
            tabViews.get(i).setVisibility(i == selected ? View.VISIBLE : View.GONE);
        }
        for (int i = 0; i < tabButtons.size(); i++) {
            TextView b = tabButtons.get(i);
            boolean sel = i == selected;
            b.setTypeface(null, sel ? Typeface.BOLD : Typeface.NORMAL);
            b.setBackgroundColor(sel ? 0x14000000 : 0x00000000);
        }
    }

    private int dp(int v) {
        return (int) (v * getResources().getDisplayMetrics().density);
    }
}
