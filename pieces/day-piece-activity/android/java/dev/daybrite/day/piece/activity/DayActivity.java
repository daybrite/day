// The day-piece-activity crate's OWN Android backend — bundled here and folded into the app's Gradle
// build via [package.metadata.day.android], with ZERO edits to day-android. It uses only
// day-android's PUBLIC Java surface: DayBridge.ctx (the Context). android.widget.ProgressBar's
// default style is a circular indeterminate spinner, so the piece adds no Gradle dependencies and no
// permissions. See docs/extending.md + docs/activity.md.
package dev.daybrite.day.piece.activity;

import android.view.View;
import android.widget.ProgressBar;

import dev.daybrite.day.bridge.DayBridge;

/** Wraps android.widget.ProgressBar as an indeterminate activity/loading spinner. */
public final class DayActivity {
    private DayActivity() {}

    public static View makeActivity(boolean animating, boolean large) {
        // The default ProgressBar style is a circular indeterminate spinner.
        ProgressBar bar = new ProgressBar(DayBridge.ctx);
        bar.setIndeterminate(true);
        if (large) {
            // The circular drawable is drawn at its intrinsic size; scale it up for `.large`
            // (DayFixed does not clip, so the enlarged spinner renders fully).
            bar.setScaleX(1.5f);
            bar.setScaleY(1.5f);
        }
        setActivityAnimating(bar, animating);
        return bar;
    }

    /**
     * A default indeterminate circular ProgressBar always animates while VISIBLE. The closest to a
     * stopped-but-present spinner is INVISIBLE, which keeps the view's layout box (so surrounding
     * layout does not jump) while hiding the animation. setFrame never forces visibility, so this
     * sticks across relayouts.
     */
    public static void setActivityAnimating(View view, boolean animating) {
        if (!(view instanceof ProgressBar)) {
            return;
        }
        view.setVisibility(animating ? View.VISIBLE : View.INVISIBLE);
    }
}
