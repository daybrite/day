// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// Android: an `EditText` subclass over its live `SpannableStringBuilder`, with the piece's OWN Java
// (`dev.daybrite.day.piece.texteditor.DayTextEditor`) bundled under `android/java` and pulled into
// the app's Gradle build by `[package.metadata.day.android]` — no edits to day-android.
//
// Runs cross as flat parallel int arrays, the shape day-android's own `setLabelRuns` uses: one JNI
// call per patch rather than one per run, on a path a syntax highlighter runs on every keystroke.
// The flag bits are deliberately the SAME ones the label path defines, so Android has one span
// vocabulary rather than two.
//
// Offsets are Java `char`s — UTF-16 code units — so this arm shares the Apple conversion.
// ---------------------------------------------------------------------------

use super::*;

use day_android::jni::objects::JValue;
use day_android::{AHandle, Android, DayEnv, with_env};
use day_spec::sidetable::SideTable;
use day_spec::{ListStyle, NodeId, ParagraphAlign, Proposal, Size};

/// This piece's OWN Java class (in the crate's android/java, on the app classpath at build).
const EDITOR_CLASS: &str = "dev/daybrite/day/piece/texteditor/DayTextEditor";

/// One list level's indent, in dp — matching the Apple arms' points.
const LEVEL_INDENT: f64 = 24.0;
const MARKER_INDENT: f64 = 18.0;

struct EdState {
    /// The text the view holds, for converting byte ranges to UTF-16 offsets without a JNI read.
    text: String,
    base_points: f64,
    min_lines: u32,
    max_lines: u32,
}

thread_local! {
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
    with_env(|env| {
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
        if let Some([sa, se, sf, sc, sb, sz]) =
            int_arrays(env, [&starts, &ends, &flags, &colors, &backgrounds, &scales])
        {
            let _ = env.dcall_static(
                EDITOR_CLASS,
                "setRuns",
                "(Landroid/view/View;[I[I[I[I[I[I)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Object(&sa),
                    JValue::Object(&se),
                    JValue::Object(&sf),
                    JValue::Object(&sc),
                    JValue::Object(&sb),
                    JValue::Object(&sz),
                ],
            );
        }

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
        if let Some([pa, pe, al, ind, mk]) =
            int_arrays(env, [&pstarts, &pends, &aligns, &indents, &markers])
        {
            let _ = env.dcall_static(
                EDITOR_CLASS,
                "setParagraphs",
                "(Landroid/view/View;[I[I[I[I[I)V",
                &[
                    JValue::Object(h.0.as_obj()),
                    JValue::Object(&pa),
                    JValue::Object(&pe),
                    JValue::Object(&al),
                    JValue::Object(&ind),
                    JValue::Object(&mk),
                ],
            );
        }
    })
}

/// Allocate and fill N Java int arrays, or `None` if any allocation or copy fails — a failed JNI
/// array is not something to paper over halfway through a patch.
fn int_arrays<'a, const N: usize>(
    env: &mut day_android::jni::Env<'a>,
    src: [&Vec<i32>; N],
) -> Option<[day_android::jni::objects::JIntArray<'a>; N]> {
    let mut out = Vec::with_capacity(N);
    for v in src {
        let arr = env.new_int_array(v.len()).ok()?;
        arr.set_region(env, 0, v).ok()?;
        out.push(arr);
    }
    out.try_into().ok()
}

fn make(_backend: &mut Android, p: &EditorProps, id: NodeId) -> AHandle {
    let base_points = day_android::font_style(p.base).0 as f64;
    let h = with_env(|env| {
        let text = env.new_string(&p.doc.text).expect("document text");
        let ph = env.new_string(&p.placeholder).expect("placeholder");
        // `try_make_view_on`, not `make_view`: the latter looks the factory up on DayBridge, and
        // this piece's factory is its OWN staged class (docs/bridge.md).
        let made = day_android::try_make_view_on(
            env,
            EDITOR_CLASS,
            "makeEditor",
            "(JLjava/lang/String;Ljava/lang/String;ZZIIF)Landroid/view/View;",
            &[
                JValue::Long(id.0 as i64),
                JValue::Object(&text),
                JValue::Object(&ph),
                JValue::Bool(p.editable),
                JValue::Bool(p.spellcheck),
                JValue::Int(p.min_lines as i32),
                JValue::Int(p.max_lines as i32),
                JValue::Float(base_points as f32),
            ],
        );
        let view = match made {
            Ok(v) => v,
            Err(e) => {
                // Degrade loudly, as every other native make does: a visible placeholder leaf
                // rather than a panic inside a JNI up-call (§8.5).
                log::warn!(
                    "day-piece-texteditor: DayTextEditor.makeEditor failed ({e}); \
                     substituting a placeholder view"
                );
                day_android::placeholder_view(env, "makeEditor")
            }
        };
        AHandle(view)
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

fn update(_backend: &mut Android, h: &AHandle, patch: &EditorPatch) {
    match patch {
        EditorPatch::SetDocument(doc) => {
            with_env(|env| {
                let s = env.new_string(&doc.text).expect("document text");
                let _ = env.dcall_static(
                    EDITOR_CLASS,
                    "setDocumentText",
                    "(Landroid/view/View;Ljava/lang/String;)V",
                    &[JValue::Object(h.0.as_obj()), JValue::Object(&s)],
                );
            });
            STATE.with(|t| t.with(key(h), |st| st.text = doc.text.clone()));
            apply_attributes(h, &doc.text, &doc.runs, &doc.paragraphs);
        }
        EditorPatch::SetAttributes(attrs) => {
            // The patch carries the text too, so the cached copy never goes a keystroke stale
            // under a live highlighter — which would style the wrong characters.
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
            with_env(|env| {
                let _ = env.dcall_static(
                    EDITOR_CLASS,
                    "setSelectionRange",
                    "(Landroid/view/View;II)V",
                    &[
                        JValue::Object(h.0.as_obj()),
                        JValue::Int(start as i32),
                        JValue::Int((start + len) as i32),
                    ],
                );
            });
        }
        EditorPatch::SetTypingStyle(s) => {
            with_env(|env| {
                let _ = env.dcall_static(
                    EDITOR_CLASS,
                    "setTypingStyle",
                    "(Landroid/view/View;IIII)V",
                    &[
                        JValue::Object(h.0.as_obj()),
                        JValue::Int(style_flags(s)),
                        JValue::Int(s.color.map(argb).unwrap_or(0)),
                        JValue::Int(s.background.map(argb).unwrap_or(0)),
                        JValue::Int((s.font.scale * 1000.0).round() as i32),
                    ],
                );
            });
        }
        EditorPatch::SetEditable(v) => {
            with_env(|env| {
                let _ = env.dcall_static(
                    EDITOR_CLASS,
                    "setEditable",
                    "(Landroid/view/View;Z)V",
                    &[JValue::Object(h.0.as_obj()), JValue::Bool(*v)],
                );
            });
        }
    }
}

/// A growing leaf: fill the proposed width, and take a height from the line band. Android measures
/// its own content, but a JNI measure per layout pass is the expensive way to ask — the band is
/// what the piece promises, and the EditText scrolls inside it.
fn measure(_backend: &mut Android, h: &AHandle, p: Proposal) -> Size {
    let avail_w = p.width.unwrap_or(320.0).max(120.0);
    STATE
        .with(|t| {
            t.with(key(h), |st| {
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

fn release(_backend: &mut Android, h: &AHandle) {
    STATE.with(|t| {
        t.remove(key(h));
    });
}

day_pieces::renderer!(day_android::RENDERERS, Android,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: measure, release: release);
