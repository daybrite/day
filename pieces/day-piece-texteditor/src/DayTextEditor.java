// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The styled-text editor's OWN Android backing — bundled with the day-piece-texteditor crate and
// pulled into the app's Gradle build through [package.metadata.day.android], with no edits to
// day-android. It uses only DayBridge's PUBLIC surface: `ctx`, `nativeOnEvent`, and the two event
// kind constants.
//
// Two things make this more than "an EditText with spans":
//
// - **Attributes are applied to the LIVE Editable.** An EditText's text IS a
//   SpannableStringBuilder, so re-styling means removing this class's spans and setting new ones on
//   the buffer the user is typing in. The characters, the caret, the IME composition and the undo
//   stack all survive — which `setText` with a fresh Spannable would destroy on every keystroke.
// - **Selection needs a subclass.** `onSelectionChanged` is a protected TextView method with no
//   listener equivalent, so reporting the caret at all requires extending EditText.
//
// Span removal is by CLASS, and only the classes this file sets. The IME's composing spans and the
// framework's own selection spans live in the same buffer and must survive: removing them cancels
// a half-typed Japanese or Korean word mid-composition.
package dev.daybrite.day.piece.texteditor;

import android.content.Context;
import android.graphics.Typeface;
import android.text.Editable;
import android.text.InputType;
import android.text.Layout;
import android.text.Spanned;
import android.text.TextWatcher;
import android.text.style.AlignmentSpan;
import android.text.style.BackgroundColorSpan;
import android.text.style.ForegroundColorSpan;
import android.text.style.LeadingMarginSpan;
import android.text.style.RelativeSizeSpan;
import android.text.style.StrikethroughSpan;
import android.text.style.StyleSpan;
import android.text.style.TypefaceSpan;
import android.text.style.UnderlineSpan;
import android.util.TypedValue;
import android.view.Gravity;
import android.view.View;
import android.widget.EditText;

import dev.daybrite.day.bridge.DayBridge;

public final class DayTextEditor {

    /** The EditText subclass, for `onSelectionChanged` — which has no listener form. */
    public static final class DayEditText extends EditText {
        long node = 0;
        /** Set while Day itself writes, so its own edits never echo back as user input. */
        boolean suppress = false;
        /** The input type as built, so `setEditable(true)` can put it back — `setKeyListener`
         *  resets it as a side effect. */
        int baseInputType = InputType.TYPE_CLASS_TEXT;

        public DayEditText(Context c) {
            super(c);
        }

        @Override
        protected void onSelectionChanged(int start, int end) {
            super.onSelectionChanged(start, end);
            // Called from TextView's constructor, before `node` is assigned — hence the guard.
            if (suppress || node == 0) {
                return;
            }
            DayBridge.nativeOnEvent(node, DayBridge.K_CUSTOM, 0, "sel " + start + " " + end);
        }
    }

    public static View makeEditor(final long id, String text, String placeholder, boolean editable,
                                  boolean spellcheck, int minLines, int maxLines, float basePt) {
        final DayEditText e = new DayEditText(DayBridge.ctx);
        e.node = id;
        int type = InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_MULTI_LINE;
        if (!spellcheck) {
            type |= InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS;
        }
        e.setInputType(type);
        e.baseInputType = type;
        e.setSingleLine(false);
        e.setGravity(Gravity.TOP | Gravity.START);
        e.setHint(placeholder);
        e.setFocusable(editable);
        e.setFocusableInTouchMode(editable);
        e.setCursorVisible(editable);
        e.setTextSize(TypedValue.COMPLEX_UNIT_SP, basePt);
        e.setMinLines(Math.max(1, minLines));
        if (maxLines > 0) {
            e.setMaxLines(maxLines);
            e.setVerticalScrollBarEnabled(true);
        }
        if (text != null && !text.isEmpty()) {
            e.suppress = true;
            e.setText(text);
            e.setSelection(text.length());
            e.suppress = false;
        }
        e.addTextChangedListener(new TextWatcher() {
            public void afterTextChanged(Editable s) {
                if (e.suppress) {
                    return;
                }
                DayBridge.nativeOnEvent(id, DayBridge.K_TEXT_CHANGED, 0, s.toString());
            }

            public void beforeTextChanged(CharSequence s, int a, int b, int c) {}

            public void onTextChanged(CharSequence s, int a, int b, int c) {}
        });
        return e;
    }

    /** Replace the characters, keeping the caret as close to where the user left it as the new
     *  text allows. Guarded so the watcher does not report Day's own write back. */
    public static void setDocumentText(View v, String text) {
        DayEditText e = (DayEditText) v;
        if (e.getText().toString().equals(text)) {
            return;
        }
        int caret = e.getSelectionStart();
        e.suppress = true;
        e.setText(text);
        e.setSelection(Math.max(0, Math.min(caret, text.length())));
        e.suppress = false;
    }

    /**
     * Re-style the live buffer: drop this class's spans, then set the ones the runs describe.
     * Flags match the label path's (docs/text-runs.md) so the two stay one vocabulary:
     * 1 bold, 2 italic, 4 monospace, 8 strikethrough, 16 color, 32 background, 64 underline.
     */
    public static void setRuns(View v, int[] starts, int[] ends, int[] flags, int[] colors,
                               int[] backgrounds, int[] scales) {
        DayEditText e = (DayEditText) v;
        Editable buf = e.getText();
        int len = buf.length();
        e.suppress = true;
        clearDaySpans(buf, len);
        final int EXCL = Spanned.SPAN_EXCLUSIVE_EXCLUSIVE;
        for (int i = 0; i < starts.length; i++) {
            int a = Math.max(0, Math.min(starts[i], len));
            int b = Math.max(a, Math.min(ends[i], len));
            if (a == b) {
                continue;
            }
            int f = flags[i];
            boolean bold = (f & 1) != 0;
            boolean italic = (f & 2) != 0;
            if (bold && italic) {
                buf.setSpan(new StyleSpan(Typeface.BOLD_ITALIC), a, b, EXCL);
            } else if (bold) {
                buf.setSpan(new StyleSpan(Typeface.BOLD), a, b, EXCL);
            } else if (italic) {
                buf.setSpan(new StyleSpan(Typeface.ITALIC), a, b, EXCL);
            }
            if ((f & 4) != 0) {
                buf.setSpan(new TypefaceSpan("monospace"), a, b, EXCL);
            }
            if ((f & 8) != 0) {
                buf.setSpan(new StrikethroughSpan(), a, b, EXCL);
            }
            if ((f & 16) != 0) {
                buf.setSpan(new ForegroundColorSpan(colors[i]), a, b, EXCL);
            }
            if ((f & 32) != 0) {
                buf.setSpan(new BackgroundColorSpan(backgrounds[i]), a, b, EXCL);
            }
            // Android has ONE underline span, so dotted and wavy both draw a plain rule.
            if ((f & 64) != 0) {
                buf.setSpan(new UnderlineSpan(), a, b, EXCL);
            }
            if (i < scales.length && scales[i] != 1000 && scales[i] > 0) {
                // A multiplier, not a pixel size, so the run still tracks the user's Font Size
                // accessibility setting.
                buf.setSpan(new RelativeSizeSpan(scales[i] / 1000f), a, b, EXCL);
            }
        }
        e.suppress = false;
    }

    /**
     * Paragraph attributes: alignment and indent. `align` is 0 natural, 1 center, 2 trailing,
     * 3 justified — Android has no per-paragraph justification (only a whole-view
     * `setJustificationMode`), so justified paragraphs align naturally. Paragraph spacing has no
     * span equivalent either; docs/texteditor.md records both.
     */
    public static void setParagraphs(View v, int[] starts, int[] ends, int[] aligns, int[] indents,
                                     int[] markers) {
        DayEditText e = (DayEditText) v;
        Editable buf = e.getText();
        int len = buf.length();
        e.suppress = true;
        final int PARA = Spanned.SPAN_PARAGRAPH;
        for (int i = 0; i < starts.length; i++) {
            int a = Math.max(0, Math.min(starts[i], len));
            int b = Math.max(a, Math.min(ends[i], len));
            if (a == b) {
                continue;
            }
            // SPAN_PARAGRAPH requires both ends to sit on a paragraph boundary; anything else
            // throws. Fall back to the inclusive-exclusive flag when they do not.
            int flag = (a == 0 || buf.charAt(a - 1) == '\n')
                    && (b == len || buf.charAt(b - 1) == '\n')
                    ? PARA
                    : Spanned.SPAN_EXCLUSIVE_EXCLUSIVE;
            Layout.Alignment al = aligns[i] == 1
                    ? Layout.Alignment.ALIGN_CENTER
                    : aligns[i] == 2 ? Layout.Alignment.ALIGN_OPPOSITE : Layout.Alignment.ALIGN_NORMAL;
            buf.setSpan(new AlignmentSpan.Standard(al), a, b, flag);
            int indent = indents[i];
            int marker = markers[i];
            if (indent != 0 || marker != 0) {
                // The marker hangs in the gap: the first line starts at `indent`, the wrapped
                // lines at `indent + marker`.
                buf.setSpan(new LeadingMarginSpan.Standard(indent, indent + marker), a, b, flag);
            }
        }
        e.suppress = false;
    }

    /** Remove exactly the span types this class sets, leaving the IME's composing spans and the
     *  framework's selection spans alone. */
    private static void clearDaySpans(Editable buf, int len) {
        removeAll(buf, len, StyleSpan.class);
        removeAll(buf, len, TypefaceSpan.class);
        removeAll(buf, len, StrikethroughSpan.class);
        removeAll(buf, len, ForegroundColorSpan.class);
        removeAll(buf, len, BackgroundColorSpan.class);
        removeAll(buf, len, UnderlineSpan.class);
        removeAll(buf, len, RelativeSizeSpan.class);
        removeAll(buf, len, AlignmentSpan.Standard.class);
        removeAll(buf, len, LeadingMarginSpan.Standard.class);
    }

    private static <T> void removeAll(Editable buf, int len, Class<T> type) {
        for (Object span : buf.getSpans(0, len, type)) {
            buf.removeSpan(span);
        }
    }

    /** Move the caret / selection. Suppressed: this IS the app's own write. */
    public static void setSelectionRange(View v, int start, int end) {
        DayEditText e = (DayEditText) v;
        int len = e.getText().length();
        e.suppress = true;
        e.setSelection(Math.max(0, Math.min(start, len)), Math.max(0, Math.min(end, len)));
        e.suppress = false;
    }

    /**
     * What the next typed character takes. Android's own mechanism for this is span FLAGS —
     * SPAN_INCLUSIVE_EXCLUSIVE on a zero-length span at the caret, which the framework then
     * extends over what is typed into it. The piece also applies the style in its own model, so
     * this is the frame-one appearance rather than the source of truth.
     */
    public static void setTypingStyle(View v, int flags, int color, int background, int scale) {
        DayEditText e = (DayEditText) v;
        Editable buf = e.getText();
        int at = e.getSelectionStart();
        if (at < 0 || at != e.getSelectionEnd()) {
            return; // a real selection: the app styles it through the document instead
        }

        final int INCL = Spanned.SPAN_INCLUSIVE_EXCLUSIVE;
        e.suppress = true;
        boolean bold = (flags & 1) != 0;
        boolean italic = (flags & 2) != 0;
        if (bold || italic) {
            int style = bold && italic ? Typeface.BOLD_ITALIC : bold ? Typeface.BOLD : Typeface.ITALIC;
            buf.setSpan(new StyleSpan(style), at, at, INCL);
        }
        if ((flags & 4) != 0) {
            buf.setSpan(new TypefaceSpan("monospace"), at, at, INCL);
        }
        if ((flags & 8) != 0) {
            buf.setSpan(new StrikethroughSpan(), at, at, INCL);
        }
        if ((flags & 16) != 0) {
            buf.setSpan(new ForegroundColorSpan(color), at, at, INCL);
        }
        if ((flags & 32) != 0) {
            buf.setSpan(new BackgroundColorSpan(background), at, at, INCL);
        }
        if ((flags & 64) != 0) {
            buf.setSpan(new UnderlineSpan(), at, at, INCL);
        }
        if (scale != 1000 && scale > 0) {
            buf.setSpan(new RelativeSizeSpan(scale / 1000f), at, at, INCL);
        }
        e.suppress = false;
    }

    public static void setEditable(View v, boolean on) {
        DayEditText e = (DayEditText) v;
        e.setFocusable(on);
        e.setFocusableInTouchMode(on);
        e.setCursorVisible(on);
        // An EditText with no key listener is a read-only view that still selects and scrolls.
        e.setKeyListener(on ? android.text.method.TextKeyListener.getInstance() : null);
        if (on) {
            e.setInputType(e.baseInputType); // setKeyListener resets it
        }
    }
}
