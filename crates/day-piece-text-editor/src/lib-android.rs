// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: an `EditText` subclass over its live `SpannableStringBuilder`, with the piece's Java
// (`dev.daybrite.day.piece.texteditor.DayTextEditor`, in `src/DayTextEditor.java`) pulled into
// the app's Gradle build by `[package.metadata.day.android]`, with no edits to day-toolkit-material.
//
// Runs cross as flat parallel int arrays, the shape day-toolkit-material's own `setLabelRuns` uses: one JNI
// call per patch rather than one per run, on a path a syntax highlighter runs on every keystroke.
// The flag bits are the same ones the label path defines, so Android has one span
// vocabulary rather than two.
//
// Offsets are Java `char`s (UTF-16 code units), so this arm shares the Apple conversion.
// ---------------------------------------------------------------------------

use super::*;

use day_toolkit_material::jni::objects::JString;
use day_toolkit_material::{AHandle, Material, java};
use day_spec::sidetable::SideTable;
use day_spec::{ListStyle, NodeId, ParagraphAlign, Proposal, Size};

// This piece's Java class (src/DayTextEditor.java, on the app classpath at build).
day_toolkit_material::java_class! {
    DayTextEditor => dev.daybrite.day.piece.texteditor.DayTextEditor,
    methods {
        static fn makeEditor(id: jlong, text: JString, placeholder: JString, editable: jboolean,
            spellcheck: jboolean, min_lines: jint, max_lines: jint, base_points: jfloat,
            single_line: jboolean, code: jboolean) -> android.view.View,
        static fn setRuns(view: android.view.View, starts: jint[], ends: jint[], flags: jint[],
            colors: jint[], backgrounds: jint[], scales: jint[]),
        static fn setParagraphs(view: android.view.View, starts: jint[], ends: jint[],
            aligns: jint[], indents: jint[], markers: jint[]),
        static fn setDocumentText(view: android.view.View, text: JString),
        static fn setSelectionRange(view: android.view.View, start: jint, end: jint),
        static fn setTypingStyle(view: android.view.View, flags: jint, color: jint,
            background: jint, scale: jint),
        static fn setEditable(view: android.view.View, on: jboolean),
        static fn insertText(view: android.view.View, at: jint, text: JString),
    },
}

/// One list level's indent, in dp, matching the Apple arms' points.
const LEVEL_INDENT: f64 = 24.0;
const MARKER_INDENT: f64 = 18.0;

struct EdState {
    /// The text the view holds, for converting byte ranges to UTF-16 offsets without a JNI read.
    text: String,
    base_points: f64,
    min_lines: u32,
    max_lines: u32,
}

day_core::tls_group! {
    static STATE: SideTable<EdState> = SideTable::new();

}

fn key(h: &AHandle) -> usize {
    std::sync::Arc::as_ptr(&h.0) as usize
}

fn argb(c: day_spec::Color) -> i32 {
    let q = |v: f64| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    ((q(c.a) << 24) | (q(c.r) << 16) | (q(c.g) << 8) | q(c.b)) as i32
}

/// The label path's flag bits, so both halves of Android speak one vocabulary
/// (1 bold, 2 italic, 4 monospace, 8 strikethrough, 16 color, 32 background, 64 underline).
fn style_flags(s: &RunStyle) -> i32 {
    let mut f = 0i32;
    if s.font
        .weight
        .is_some_and(|w| w >= day_spec::FontWeight::Semibold)
    {
        f |= 1;
    }
    if s.font.italic {
        f |= 2;
    }
    if s.font.monospace {
        f |= 4;
    }
    if s.strikethrough {
        f |= 8;
    }
    if s.color.is_some() {
        f |= 16;
    }
    if s.background.is_some() {
        f |= 32;
    }
    if s.underline.is_on() {
        f |= 64;
    }
    f
}

/// Push runs and paragraphs into the live buffer, as two calls carrying flat arrays.
fn apply_attributes(h: &AHandle, text: &str, runs: &[TextRun], paragraphs: &[ParagraphRun]) {
    java(|env| {
        let mut starts = Vec::with_capacity(runs.len());
        let mut ends = Vec::with_capacity(runs.len());
        let mut flags = Vec::with_capacity(runs.len());
        let mut colors = Vec::with_capacity(runs.len());
        let mut backgrounds = Vec::with_capacity(runs.len());
        // Relative size in per-mille, so it rides the same int-array path as everything else.
        let mut scales = Vec::with_capacity(runs.len());
        for r in runs {
            let Some((start, len)) = utf16_range(text, &r.range) else {
                continue;
            };
            starts.push(start as i32);
            ends.push((start + len) as i32);
            flags.push(style_flags(&r.style()));
            colors.push(r.color.map(argb).unwrap_or(0));
            backgrounds.push(r.background.map(argb).unwrap_or(0));
            scales.push((r.font.scale * 1000.0).round() as i32);
        }
        let [sa, se, sf, sc, sb, sz] =
            int_arrays(env, [&starts, &ends, &flags, &colors, &backgrounds, &scales])?;
        DayTextEditor::setRuns(env, h.0.as_obj(), &sa, &se, &sf, &sc, &sb, &sz)?;

        let mut pstarts = Vec::with_capacity(paragraphs.len());
        let mut pends = Vec::with_capacity(paragraphs.len());
        let mut aligns = Vec::with_capacity(paragraphs.len());
        let mut indents = Vec::with_capacity(paragraphs.len());
        let mut markers = Vec::with_capacity(paragraphs.len());
        for p in paragraphs {
            let Some((start, len)) = utf16_range(text, &p.range) else {
                continue;
            };
            let s = p.style();
            pstarts.push(start as i32);
            pends.push((start + len) as i32);
            aligns.push(match s.align {
                ParagraphAlign::Natural => 0,
                ParagraphAlign::Center => 1,
                ParagraphAlign::Trailing => 2,
                ParagraphAlign::Justified => 3,
            });
            indents.push((s.indent + f64::from(s.list_level) * LEVEL_INDENT).round() as i32);
            markers.push(if s.list == ListStyle::None {
                0
            } else {
                MARKER_INDENT as i32
            });
        }
        let [pa, pe, al, ind, mk] =
            int_arrays(env, [&pstarts, &pends, &aligns, &indents, &markers])?;
        DayTextEditor::setParagraphs(env, h.0.as_obj(), &pa, &pe, &al, &ind, &mk)
    });
}

/// Allocate and fill N Java int arrays; a failed JNI array is not something to paper over
/// halfway through a patch, so any failure ends the whole push.
fn int_arrays<'a, const N: usize>(
    env: &mut day_toolkit_material::jni::Env<'a>,
    src: [&Vec<i32>; N],
) -> day_toolkit_material::jni::errors::Result<[day_toolkit_material::jni::objects::JIntArray<'a>; N]> {
    let mut out = Vec::with_capacity(N);
    for v in src {
        let arr = env.new_int_array(v.len())?;
        arr.set_region(env, 0, v)?;
        out.push(arr);
    }
    out.try_into()
        .map_err(|_| day_toolkit_material::jni::errors::Error::WrongJValueType("int[]", "array count"))
}

fn make(_backend: &mut Material, p: &EditorProps, id: NodeId) -> AHandle {
    let base_points = day_toolkit_material::font_style(p.base).0 as f64;
    let h = day_toolkit_material::realize("text_editor", |env| {
        let text = JString::from_str(env, &p.doc.text)?;
        let placeholder = JString::from_str(env, &p.placeholder)?;
        DayTextEditor::makeEditor(
            env,
            id.0 as i64,
            &text,
            &placeholder,
            p.editable,
            p.spellcheck,
            p.min_lines as i32,
            p.max_lines as i32,
            base_points as f32,
            p.single_line,
            p.code,
        )
    });
    if !p.doc.runs.is_empty() || !p.doc.paragraphs.is_empty() {
        apply_attributes(&h, &p.doc.text, &p.doc.runs, &p.doc.paragraphs);
    }
    STATE.with(|t| {
        t.insert(
            key(&h),
            EdState {
                text: p.doc.text.clone(),
                base_points,
                min_lines: p.min_lines,
                max_lines: p.max_lines,
            },
        )
    });
    h
}

fn update(_backend: &mut Material, h: &AHandle, patch: &EditorPatch) {
    match patch {
        EditorPatch::SetDocument(doc) => {
            java(|env| {
                let text = JString::from_str(env, &doc.text)?;
                DayTextEditor::setDocumentText(env, h.0.as_obj(), &text)
            });
            STATE.with(|t| t.with(key(h), |st| st.text = doc.text.clone()));
            apply_attributes(h, &doc.text, &doc.runs, &doc.paragraphs);
        }
        EditorPatch::SetAttributes(attrs) => {
            // The patch carries the text too, so the cached copy never goes a keystroke stale
            // under a live highlighter, which would style the wrong characters.
            STATE.with(|t| t.with(key(h), |st| st.text = attrs.text.clone()));
            apply_attributes(h, &attrs.text, &attrs.runs, &attrs.paragraphs);
        }
        EditorPatch::SetSelection(r) => {
            let text = STATE
                .with(|t| t.with(key(h), |st| st.text.clone()))
                .unwrap_or_default();
            let Some((start, len)) = utf16_range(&text, r) else {
                return;
            };
            java(|env| {
                DayTextEditor::setSelectionRange(
                    env,
                    h.0.as_obj(),
                    start as i32,
                    (start + len) as i32,
                )
            });
        }
        EditorPatch::SetTypingStyle(s) => {
            java(|env| {
                DayTextEditor::setTypingStyle(
                    env,
                    h.0.as_obj(),
                    style_flags(s),
                    s.color.map(argb).unwrap_or(0),
                    s.background.map(argb).unwrap_or(0),
                    (s.font.scale * 1000.0).round() as i32,
                )
            });
        }
        EditorPatch::SetEditable(v) => {
            java(|env| DayTextEditor::setEditable(env, h.0.as_obj(), *v));
        }
        EditorPatch::Insert { at, text } => {
            // The BYTE offset crosses as is: this arm's text cache is current only as of the
            // last patch, and an insert follows a keystroke the view reported since, so the
            // Java side converts against the live buffer instead.
            java(|env| {
                let text = JString::from_str(env, text)?;
                DayTextEditor::insertText(env, h.0.as_obj(), *at as i32, &text)
            });
        }
    }
}

/// A growing leaf: fill the proposed width, and take a height from the line band. Android measures
/// its own content, but a JNI measure per layout pass is the expensive way to ask; the band is
/// what the piece promises, and the EditText scrolls inside it.
fn measure(_backend: &mut Material, h: &AHandle, p: Proposal) -> Size {
    let wanted_w = p.width.unwrap_or(320.0);
    let avail_w = wanted_w.max(width_floor(0));
    STATE
        .with(|t| {
            t.with(key(h), |st| {
                let avail_w = wanted_w.max(width_floor(st.max_lines));
                let line_h = st.base_points * 1.4 + 4.0;
                let lines = st.text.lines().count().max(1) as f64;
                let min_h = f64::from(st.min_lines) * line_h;
                let max_h = if st.max_lines > 0 {
                    f64::from(st.max_lines) * line_h
                } else {
                    f64::MAX
                };
                Size::new(avail_w, (lines * line_h).clamp(min_h, max_h).ceil() + 16.0)
            })
        })
        .unwrap_or_else(|| Size::new(avail_w, 88.0))
}

fn release(_backend: &mut Material, h: &AHandle) {
    STATE.with(|t| {
        t.remove(key(h));
    });
}

day_pieces::renderer!(day_toolkit_material::RENDERERS, Material,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: measure, release: release);
