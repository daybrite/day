// The day-piece-lottie crate's OWN Android backend — a Java shim over airbnb/lottie-android. It is
// bundled with THIS crate and folded into the app's Gradle build via [package.metadata.day.android]
// (which also declares the com.airbnb.android:lottie dependency), with ZERO edits to day-android. It
// uses only day-android's PUBLIC surface (DayBridge.ctx). It is the Android twin of ios/swift/DayLottie.swift.
package dev.daybrite.day.piece.lottie;

import android.view.View;

import com.airbnb.lottie.LottieAnimationView;
import com.airbnb.lottie.LottieDrawable;

import dev.daybrite.day.bridge.DayBridge;

/** Wraps LottieAnimationView, loading `name`(.json) from the app's assets. */
public final class DayLottie {
    private DayLottie() {}

    public static View makeLottie(String name, boolean looping, boolean autoplay, float speed) {
        LottieAnimationView view = new LottieAnimationView(DayBridge.ctx);
        // Lottie loads from src/main/assets by filename; the app bundles the JSON there.
        view.setAnimation(name.endsWith(".json") ? name : name + ".json");
        view.setRepeatCount(looping ? LottieDrawable.INFINITE : 0);
        view.setSpeed(speed);
        if (autoplay) {
            view.playAnimation();
        }
        return view;
    }

    /** Update the playback rate of an existing view (from a `Speed` patch). */
    public static void setSpeed(View view, float speed) {
        if (view instanceof LottieAnimationView) {
            ((LottieAnimationView) view).setSpeed(speed);
        }
    }
}
