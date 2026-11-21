// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

package dev.daybrite.day.bridge;

import android.content.Context;
import android.graphics.Canvas;
import android.graphics.Paint;
import android.graphics.Path;
import android.graphics.RectF;
import android.os.Build;
import android.text.Layout;
import android.text.SpannableString;
import android.text.Spanned;
import android.text.TextPaint;
import android.text.TextUtils;
import android.text.style.LineHeightSpan;
import android.text.style.MetricAffectingSpan;
import android.view.Gravity;
import android.widget.TextView;

/**
 * A Day label (docs/text.md): a framework `TextView` that also keeps what Day asked for, so the
 * paragraph attributes, the decorations and the selection reports survive each other's updates.
 *
 * <ul>
 * <li>Line limit and truncation: the TextView's own `maxLines` and `ellipsize`. Android places a
 * head or middle ellipsis on a single line only, so a one-line label with those modes runs in
 * single-line mode; several lines end with a tail ellipsis.</li>
 * <li>Letter spacing: `setLetterSpacing`, which already takes ems.</li>
 * <li>Line height: a {@link LineHeightSpan} over the whole text that gives every line exactly
 * `multiple * textSize` pixels, the extra split above and below (the CSS model). `setLineHeight`
 * is API 28 and puts the extra under the line only.</li>
 * <li>Decorations Android has no span for (a double, dotted or wavy line, or a line in its own
 * color) are stroked in {@link #onDraw} over the laid-out text; a plain single line stays the
 * native `UnderlineSpan`/`StrikethroughSpan`.</li>
 * <li>Selection reports (`Label::on_selection`): {@link #onSelectionChanged}, coalesced to one
 * report per looper turn, as UTF-8 byte offsets into the text Day set.</li>
 * </ul>
 *
 * The reported text (`getText()`) is always the whole text Day set: the ellipsis lives in the
 * layout only.
 */
public final class DayLabel extends TextView {
    /** Truncation codes, `day_spec::props::Truncation` order. */
    static final int TRUNCATE_TAIL = 0, TRUNCATE_HEAD = 1, TRUNCATE_MIDDLE = 2;

    /** Decoration entry layout in {@link #decorations}: five ints each. */
    static final int DECO_STRIDE = 5;
    /** Bits of a decoration entry's info word: bit 0 = strikethrough (else underline), bits 1-2 =
     *  the style (0 single, 1 double, 2 dotted, 3 wavy), bit 3 = an explicit color. */
    static final int DECO_STRIKE = 1, DECO_STYLE_SHIFT = 1, DECO_COLORED = 8;

    private CharSequence source = "";
    private boolean singleLine;
    private boolean inSingleLineMode;
    private int maxLines;
    private int truncation = TRUNCATE_TAIL;
    private float lineHeight;
    /** start, end (UTF-16), info, color, size per mille; null when nothing is custom-drawn. */
    private int[] decorations;

    private long reportNode;
    private boolean selectionPosted;
    private int reportedStart, reportedEnd;

    private final Path scratchPath = new Path();
    private final RectF scratchRect = new RectF();
    private final Paint decoPaint = new Paint(Paint.ANTI_ALIAS_FLAG);
    private final TextPaint metricsPaint = new TextPaint(Paint.ANTI_ALIAS_FLAG);

    public DayLabel(Context context) {
        super(context);
    }

    /** The text Day set (a plain string, or a spanned string carrying runs). */
    void setDayText(CharSequence text) {
        source = text == null ? "" : text;
        applyText();
    }

    /** `.single_line()`: one line whatever the limit says (realize-time). */
    void setDaySingleLine(boolean on) {
        singleLine = on;
        applyLines();
    }

    /** The paragraph attributes (`LabelParagraph`), whole; idempotent. */
    void setParagraph(int maxLines, int truncation, float letterSpacing, float lineHeight) {
        this.maxLines = Math.max(0, maxLines);
        this.truncation = truncation;
        if (getLetterSpacing() != letterSpacing) setLetterSpacing(letterSpacing);
        boolean heightChanged = this.lineHeight != lineHeight;
        this.lineHeight = lineHeight;
        applyLines();
        if (heightChanged) applyText();
    }

    /** The font size changed: the line height is a multiple of it. */
    void fontChanged() {
        if (lineHeight > 0f) applyText();
    }

    /** The custom-drawn decorations (see {@link #DECO_STRIDE}); null or empty for none. */
    void setDecorations(int[] entries) {
        decorations = entries != null && entries.length >= DECO_STRIDE ? entries : null;
        invalidate();
    }

    /** Report selection changes for Day node `node` (`LabelProps::reports_selection`). */
    void reportSelection(long node) {
        reportNode = node;
    }

    private void applyLines() {
        // Android ellipsizes at the head or in the middle on a single line only: a wrapped line
        // limit of one would put the ellipsis inside the first line's own text, not the whole
        // text's. Single-line mode lays the text out as one line, so those modes see all of it.
        boolean wantSingle = singleLine || (maxLines == 1 && truncation != TRUNCATE_TAIL);
        if (wantSingle != inSingleLineMode) {
            setSingleLine(wantSingle);
            inSingleLineMode = wantSingle;
        }
        if (wantSingle) {
            setEllipsize(truncation == TRUNCATE_HEAD ? TextUtils.TruncateAt.START
                    : truncation == TRUNCATE_MIDDLE ? TextUtils.TruncateAt.MIDDLE
                    : TextUtils.TruncateAt.END);
        } else if (maxLines > 0) {
            setMaxLines(maxLines);
            setEllipsize(TextUtils.TruncateAt.END);
        } else {
            setMaxLines(Integer.MAX_VALUE);
            setEllipsize(null);
        }
    }

    private void applyText() {
        CharSequence shown = source;
        if (lineHeight > 0f && source.length() > 0) {
            SpannableString s = new SpannableString(source);
            s.setSpan(new LineBox(Math.round(lineHeight * getTextSize())), 0, s.length(),
                    Spanned.SPAN_INCLUSIVE_INCLUSIVE);
            shown = s;
        }
        setText(shown);
        scheduleSelectionReport();
    }

    /** Every line exactly `height` px tall, the difference from the font's own line split
     *  evenly above and below, so the glyphs stay centered in the line box. */
    private static final class LineBox implements LineHeightSpan {
        private final int height;

        LineBox(int height) {
            this.height = height;
        }

        @Override
        public void chooseHeight(CharSequence text, int start, int end, int spanstartv, int lineHeight,
                Paint.FontMetricsInt fm) {
            int extra = height - (fm.descent - fm.ascent);
            int above = Math.floorDiv(extra, 2);
            fm.ascent -= above;
            fm.descent += extra - above;
            // Top and bottom follow, so the font padding of the first and last line adds nothing.
            fm.top = fm.ascent;
            fm.bottom = fm.descent;
        }
    }

    /** OpenType features for one run (`FontSpec::feature_tags`), added to what the label's own
     *  paint asks for. */
    static final class FeatureSpan extends MetricAffectingSpan {
        private final String tags;

        FeatureSpan(String tags) {
            this.tags = tags;
        }

        private void apply(TextPaint p) {
            String base = p.getFontFeatureSettings();
            p.setFontFeatureSettings(base == null || base.isEmpty() ? tags : base + "," + tags);
        }

        @Override
        public void updateMeasureState(TextPaint p) {
            apply(p);
        }

        @Override
        public void updateDrawState(TextPaint p) {
            apply(p);
        }
    }

    // --- Selection reports ------------------------------------------------------------------

    @Override
    protected void onSelectionChanged(int selStart, int selEnd) {
        super.onSelectionChanged(selStart, selEnd);
        scheduleSelectionReport();
    }

    private void scheduleSelectionReport() {
        if (reportNode == 0 || selectionPosted) return;
        selectionPosted = true;
        // One report per looper turn: a drag of the selection handles changes the selection on
        // every move, and Day wants each distinct range once.
        post(this::reportSelectionNow);
    }

    private void reportSelectionNow() {
        selectionPosted = false;
        if (reportNode == 0) return;
        CharSequence t = getText();
        int a = getSelectionStart(), b = getSelectionEnd();
        int lo = Math.min(a, b), hi = Math.max(a, b);
        int start = 0, end = 0;
        if (lo >= 0 && hi > lo && hi <= t.length()) {
            start = utf8Offset(t, lo);
            end = utf8Offset(t, hi);
        }
        if (start == end) start = end = 0;
        if (start == reportedStart && end == reportedEnd) return;
        reportedStart = start;
        reportedEnd = end;
        DayBridge.nativeOnEvent(reportNode, DayBridge.K_CUSTOM, 0, "day:text-selected:" + start + "," + end);
    }

    /** The UTF-8 byte offset of UTF-16 index `index` in `t` (Rust's `str` offsets). */
    static int utf8Offset(CharSequence t, int index) {
        int n = 0;
        for (int i = 0; i < index && i < t.length(); i++) {
            char c = t.charAt(i);
            if (c < 0x80) {
                n += 1;
            } else if (c < 0x800) {
                n += 2;
            } else if (Character.isHighSurrogate(c) && i + 1 < t.length()
                    && Character.isLowSurrogate(t.charAt(i + 1))) {
                n += 4;
                i++;
            } else {
                n += 3;
            }
        }
        return n;
    }

    // --- Custom decorations -----------------------------------------------------------------

    @Override
    protected void onDraw(Canvas canvas) {
        super.onDraw(canvas);
        int[] d = decorations;
        Layout layout = getLayout();
        if (d == null || layout == null) return;
        CharSequence text = layout.getText();
        canvas.save();
        canvas.translate(getCompoundPaddingLeft(), getExtendedPaddingTop() + verticalOffset(layout));
        for (int i = 0; i + DECO_STRIDE <= d.length; i += DECO_STRIDE) {
            int s = Math.max(0, Math.min(d[i], text.length()));
            int e = Math.max(s, Math.min(d[i + 1], text.length()));
            if (s == e) continue;
            int first = layout.getLineForOffset(s);
            int last = layout.getLineForOffset(e - 1);
            for (int line = first; line <= last && line < layout.getLineCount(); line++) {
                int ls = layout.getLineStart(line);
                int le = layout.getLineEnd(line);
                // No line under the trailing spaces or newline a wrapped line ends with.
                while (le > ls && Character.isWhitespace(text.charAt(le - 1))) le--;
                int a = Math.max(s, ls), b = Math.min(e, le);
                int count = layout.getEllipsisCount(line);
                if (count > 0) {
                    // The ellipsized characters are not drawn: stroke what remains on each side.
                    int es = ls + layout.getEllipsisStart(line), ee = es + count;
                    strokeRange(canvas, layout, line, a, Math.min(b, es), d, i);
                    strokeRange(canvas, layout, line, Math.max(a, ee), b, d, i);
                } else {
                    strokeRange(canvas, layout, line, a, b, d, i);
                }
            }
        }
        canvas.restore();
    }

    /** Where TextView itself draws the layout vertically inside the padding (its gravity). */
    private int verticalOffset(Layout layout) {
        int gravity = getGravity() & Gravity.VERTICAL_GRAVITY_MASK;
        if (gravity == Gravity.TOP) return 0;
        int box = getMeasuredHeight() - getExtendedPaddingTop() - getExtendedPaddingBottom();
        int h = layout.getHeight();
        if (h >= box) return 0;
        return gravity == Gravity.BOTTOM ? box - h : (box - h) >> 1;
    }

    private void strokeRange(Canvas canvas, Layout layout, int line, int a, int b, int[] d, int i) {
        if (b <= a) return;
        scratchPath.reset();
        layout.getSelectionPath(a, b, scratchPath);
        scratchPath.computeBounds(scratchRect, true);
        float x0 = scratchRect.left, x1 = scratchRect.right;
        if (x1 <= x0) return;
        int info = d[i + 2];
        boolean strike = (info & DECO_STRIKE) != 0;
        int style = (info >> DECO_STYLE_SHIFT) & 3;
        int color = (info & DECO_COLORED) != 0 ? d[i + 3] : getCurrentTextColor();
        float size = getTextSize() * (d[i + 4] > 0 ? d[i + 4] / 1000f : 1f);
        metricsPaint.set(getPaint());
        metricsPaint.setTextSize(size);
        // Where the font puts its lines (API 29), else the proportions Skia uses for a face
        // that does not say: underline 1/9 of the size below the baseline, strikethrough 2/7
        // above it, both 1/18 of the size thick.
        float pos, thick;
        if (Build.VERSION.SDK_INT >= 29) {
            pos = strike ? metricsPaint.getStrikeThruPosition() : metricsPaint.getUnderlinePosition();
            thick = strike ? metricsPaint.getStrikeThruThickness() : metricsPaint.getUnderlineThickness();
        } else {
            pos = strike ? -size * 2f / 7f : size / 9f;
            thick = size / 18f;
        }
        thick = Math.max(thick, 1f);
        float baseline = layout.getLineBaseline(line);
        float top = baseline + pos;
        float mid = top + thick / 2f;
        decoPaint.setColor(color);
        decoPaint.setPathEffect(null);
        switch (style) {
            case 1: { // double: two rules a rule's width apart, the pair centered on a strike
                float gap = thick;
                float y0 = strike ? mid - gap / 2f - thick : top;
                decoPaint.setStyle(Paint.Style.FILL);
                canvas.drawRect(x0, y0, x1, y0 + thick, decoPaint);
                canvas.drawRect(x0, y0 + thick + gap, x1, y0 + 2f * thick + gap, decoPaint);
                break;
            }
            case 2: { // dotted: round dots one dot apart
                // Circles rather than a zero-length DashPathEffect with round caps, which the
                // API 24 renderer draws as a solid line.
                float w = Math.max(thick, 2f);
                decoPaint.setStyle(Paint.Style.FILL);
                for (float x = x0 + w / 2f; x <= x1 - w / 2f; x += 2f * w) {
                    canvas.drawCircle(x, mid, w / 2f, decoPaint);
                }
                break;
            }
            case 3: { // wavy: quadratic half-waves, clipped to the range
                float amp = Math.max(thick, 1.5f);
                float half = 2.5f * amp;
                scratchPath.reset();
                scratchPath.moveTo(x0, mid);
                boolean up = true;
                for (float x = x0; x < x1; x += half) {
                    scratchPath.quadTo(x + half / 2f, up ? mid - 2f * amp : mid + 2f * amp, x + half, mid);
                    up = !up;
                }
                decoPaint.setStyle(Paint.Style.STROKE);
                decoPaint.setStrokeWidth(thick);
                decoPaint.setStrokeCap(Paint.Cap.BUTT);
                canvas.save();
                canvas.clipRect(x0, mid - 3f * amp, x1, mid + 3f * amp);
                canvas.drawPath(scratchPath, decoPaint);
                canvas.restore();
                break;
            }
            default:
                decoPaint.setStyle(Paint.Style.FILL);
                canvas.drawRect(x0, top, x1, top + thick, decoPaint);
                break;
        }
    }
}
