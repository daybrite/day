package dev.daybrite.day.piece.pager;

import android.view.View;
import android.view.ViewGroup;
import android.view.MotionEvent;
import android.widget.FrameLayout;
import androidx.viewpager.widget.ViewPager;
import androidx.viewpager.widget.PagerAdapter;
import dev.daybrite.day.bridge.DayBridge;
import java.util.ArrayList;

/** Fixed identity page slots; only the reading order changes. Native scrolling owns every frame. */
public final class DayPager extends FrameLayout {
    private final long node;
    private final int capacity;
    private final ArrayList<View> slots = new ArrayList<>();
    private final ArrayList<Integer> order = new ArrayList<>();
    private final ViewPager pager;
    private long epoch;
    private boolean internal, updating, dragging;
    private String pendingOrder;
    private int pendingSelected;
    private long pendingEpoch;
    private final PagerAdapter adapter = new PagerAdapter() {
        @Override public int getCount() { return order.size(); }
        @Override public boolean isViewFromObject(View view, Object object) { return view == object; }
        @Override public Object instantiateItem(ViewGroup parent, int position) {
            View view = slots.get(order.get(position));
            if (view.getParent() instanceof ViewGroup) ((ViewGroup)view.getParent()).removeView(view);
            parent.addView(view);
            return view;
        }
        @Override public void destroyItem(ViewGroup parent, int position, Object object) { parent.removeView((View)object); }
        @Override public int getItemPosition(Object object) {
            int index = order.indexOf(slots.indexOf(object));
            return index < 0 ? POSITION_NONE : index;
        }
    };
    private DayPager(long node, int capacity) {
        super(DayBridge.ctx);
        this.node = node; this.capacity = capacity;
        pager = new ViewPager(DayBridge.ctx) {
            private float downX;
            @Override public boolean onInterceptTouchEvent(MotionEvent e) {
                if (e.getActionMasked() == MotionEvent.ACTION_DOWN) downX = e.getX();
                // Leave system back edges and multi-touch zoom to their existing owners.
                float edge = 24 * getResources().getDisplayMetrics().density;
                if (e.getPointerCount() > 1 || downX < edge || downX > getWidth() - edge) return false;
                return super.onInterceptTouchEvent(e);
            }
        };
        pager.setId(View.generateViewId());
        pager.setOffscreenPageLimit(2);
        pager.setAdapter(adapter);
        internal = true;
        addView(pager, new FrameLayout.LayoutParams(-1,-1));
        internal = false;
        pager.addOnPageChangeListener(new ViewPager.SimpleOnPageChangeListener() {
            @Override public void onPageScrollStateChanged(int state) {
                if (updating) return;
                // Accessibility and keyboard paging settle through the same path as touch.
                if (state != ViewPager.SCROLL_STATE_IDLE && !dragging) {
                    dragging = true;
                    emit("begin " + epoch);
                }
                if (state == ViewPager.SCROLL_STATE_IDLE && dragging) {
                    dragging = false;
                    int index = pager.getCurrentItem();
                    if (index < order.size()) emit("settle " + epoch + " " + order.get(index));
                    if (pendingOrder != null) {
                        String next = pendingOrder;
                        int selected = pendingSelected;
                        long nextEpoch = pendingEpoch;
                        pendingOrder = null;
                        post(() -> { if (nextEpoch >= epoch) apply(next, selected, nextEpoch); });
                    }
                }
            }
        });
    }
    private void emit(String text) { DayBridge.nativeOnEvent(node,12,0,text); }
    @Override public void onViewAdded(View child) {
        super.onViewAdded(child);
        if (!internal && child != pager && !slots.contains(child) && slots.size() < capacity) {
            slots.add(child);
            removeView(child);
        }
    }
    private void apply(String encoded, int selected, long epoch) {
        if (dragging) { pendingOrder=encoded; pendingSelected=selected; pendingEpoch=epoch; return; }
        updating = true;
        this.epoch = epoch;
        order.clear();
        if (!encoded.isEmpty()) for (String part : encoded.split(",")) {
            int slot = Integer.parseInt(part);
            if (slot >= 0 && slot < slots.size() && !order.contains(slot)) order.add(slot);
        }
        adapter.notifyDataSetChanged();
        int position = order.indexOf(selected);
        if (position >= 0) pager.setCurrentItem(position, false);
        updating = false;
    }
    public static View make(long id, int slots) { return new DayPager(id,slots); }
    public static void configure(View view, String order, int selected, long epoch) { ((DayPager)view).apply(order,selected,epoch); }
}
