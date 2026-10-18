// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The remote-image piece's Android factory, bundled with the day-piece-remote-image crate and
// pulled into the app's Gradle build automatically (via [package.metadata.day.android] →
// day-pieces.json), with no edits to day-android. It uses only day-android's public Java surface:
// DayBridge.ctx (the Android Context). An ImageView decodes the pushed bytes with BitmapFactory;
// the circle / rounded clip is a ViewOutlineProvider + setClipToOutline (resize-correct: the
// outline is recomputed against the view's current size). A matching Canvas clip covers
// software snapshots, where Android does not apply outline clipping. The placeholder is the
// view's background color, shown while there is no bitmap.
package dev.daybrite.day.piece.remoteimage;

import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Canvas;
import android.graphics.Outline;
import android.graphics.Path;
import android.view.View;
import android.view.ViewOutlineProvider;
import android.widget.ImageView;

import dev.daybrite.day.bridge.DayBridge;

public final class DayRemoteImage {
    private static final class ClippedImageView extends ImageView {
        private final int clipKind;
        private final float radius;
        private final Path softwareClip = new Path();

        ClippedImageView(int clipKind, float radius) {
            super(DayBridge.ctx);
            this.clipKind = clipKind;
            this.radius = radius;
        }

        @Override
        protected void onSizeChanged(int w, int h, int oldw, int oldh) {
            super.onSizeChanged(w, h, oldw, oldh);
            softwareClip.rewind();
            if (clipKind == 1) {
                int d = Math.min(w, h);
                int left = (w - d) / 2;
                int top = (h - d) / 2;
                softwareClip.addOval(left, top, left + d, top + d, Path.Direction.CW);
            } else if (clipKind == 2) {
                softwareClip.addRoundRect(0, 0, w, h, radius, radius, Path.Direction.CW);
            }
            invalidateOutline();
        }

        @Override
        public void draw(Canvas canvas) {
            if (clipKind == 0 || canvas.isHardwareAccelerated()) {
                super.draw(canvas);
                return;
            }
            // Clip draw(), not onDraw(): the placeholder background needs the same mask
            // as the bitmap. Keep the hardware outline path for normal screen rendering.
            int saved = canvas.save();
            try {
                canvas.clipPath(softwareClip);
                super.draw(canvas);
            } finally {
                canvas.restoreToCount(saved);
            }
        }
    }

    // clip: 0 none, 1 circle, 2 rounded.  mode: 1 fill (CENTER_CROP), 0 fit (FIT_CENTER).
    // `placeholder` is a packed ARGB int shown while there is no bitmap.
    public static View makeImage(long id, int mode, int clip, double radius, int placeholder) {
        final float rpx =
                (float) (radius * DayBridge.ctx.getResources().getDisplayMetrics().density);
        ImageView iv = new ClippedImageView(clip, rpx);
        iv.setScaleType(mode == 1 ? ImageView.ScaleType.CENTER_CROP : ImageView.ScaleType.FIT_CENTER);
        iv.setBackgroundColor(placeholder);
        if (clip != 0) {
            final int clipKind = clip;
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
