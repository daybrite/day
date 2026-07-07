// The remote-image piece's OWN Android factory — bundled with the day-piece-remote-image crate and
// pulled into the app's Gradle build automatically (via [package.metadata.day.android] →
// day-pieces.json), with ZERO edits to day-android. It uses only day-android's PUBLIC Java surface:
// DayBridge.ctx (the Android Context). An ImageView decodes the pushed bytes with BitmapFactory;
// the circle / rounded clip is a ViewOutlineProvider + setClipToOutline (resize-correct — the
// outline is recomputed against the view's current size), and the placeholder is the view's
// background color, shown while there is no bitmap.
package dev.daybrite.day.piece.remoteimage;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Outline;
import android.view.View;
import android.view.ViewOutlineProvider;
import android.widget.ImageView;

import dev.daybrite.day.bridge.DayBridge;

public final class DayRemoteImage {
    // clip: 0 none, 1 circle, 2 rounded.  mode: 1 fill (CENTER_CROP), 0 fit (FIT_CENTER).
    // `placeholder` is a packed ARGB int shown while there is no bitmap.
    public static View makeImage(long id, int mode, int clip, double radius, int placeholder) {
        ImageView iv = new ImageView(DayBridge.ctx);
        iv.setScaleType(mode == 1 ? ImageView.ScaleType.CENTER_CROP : ImageView.ScaleType.FIT_CENTER);
        iv.setBackgroundColor(placeholder);
        if (clip != 0) {
            final int clipKind = clip;
            final float rpx =
                (float) (radius * DayBridge.ctx.getResources().getDisplayMetrics().density);
            iv.setOutlineProvider(new ViewOutlineProvider() {
                @Override
                public void getOutline(View v, Outline outline) {
                    int w = v.getWidth();
                    int h = v.getHeight();
                    if (w <= 0 || h <= 0) {
                        return;
                    }
                    if (clipKind == 1) { // centered circle
                        int d = Math.min(w, h);
                        int left = (w - d) / 2;
                        int top = (h - d) / 2;
                        outline.setOval(left, top, left + d, top + d);
                    } else { // rounded rect
                        outline.setRoundRect(0, 0, w, h, rpx);
                    }
                }
            });
            iv.setClipToOutline(true);
        }
        return iv;
    }

    // Programmatic push of decoded bytes (or a clear on null/empty → the placeholder background
    // shows). Guarded by BitmapFactory returning null on undecodable data (leaves the placeholder).
    public static void setBytes(View v, byte[] data) {
        ImageView iv = (ImageView) v;
        if (data == null || data.length == 0) {
            iv.setImageDrawable(null);
        } else {
            Bitmap bmp = BitmapFactory.decodeByteArray(data, 0, data.length);
            iv.setImageBitmap(bmp); // null bmp clears, showing the placeholder background
        }
    }
}
