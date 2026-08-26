// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// HarmonyOS: the ArkTS `RichEditor`. Unlike the other seven arms there is no native widget to
// construct here — the ArkUI C node API has no rich editor at all — so this crate ships its OWN
// ArkTS (ohos/ets/Index.ets), staged into the app's hvigor project by `day build` through
// `[package.metadata.day.ohos]`, exactly as day-piece-webview established.
//
// The whole channel is strings: one props string at realize, (cmd, arg) pairs after, and reports
// back through the shim's `pieceEvent` as the Custom event kind (§8.2). That is why the editor's
// text report rides `TEXT_PREFIX` rather than `Event::TextChanged` — this bridge has one event.
//
// Offsets are ArkTS string indices, which are UTF-16 code units.
// ---------------------------------------------------------------------------

use super::*;
use day_arkui::{AHandle, ArkUi, piece};
use day_spec::sidetable::SideTable;
use day_spec::{ListStyle, NodeId, ParagraphAlign};

/// Field separator inside a props or command string; runs are separated by the record separator.
const SEP: char = '\u{1f}';
const RUN_SEP: char = '\u{1e}';

const LEVEL_INDENT: f64 = 24.0;
const MARKER_INDENT: f64 = 18.0;

fn argb(c: day_spec::Color) -> u32 {
    let q = |v: f64| ((v.clamp(0.0, 1.0) * 255.0).round() as u32) & 0xFF;
    (q(c.a) << 24) | (q(c.r) << 16) | (q(c.g) << 8) | q(c.b)
}

/// The flag bits the whole piece shares (see the Android arm): 1 bold, 2 italic, 4 monospace,
/// 8 strikethrough, 16 color, 32 background, 64 underline.
fn style_flags(s: &RunStyle) -> u32 {
    let mut f = 0;
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

/// `start,end,flags,color,background,scale` per run, records separated — all numbers, so the
/// document's own text can never collide with a separator.
fn encode_runs(text: &str, runs: &[TextRun]) -> String {
    let mut out = String::new();
    for r in runs {
        let Some((start, len)) = utf16_range(text, &r.range) else {
            continue;
        };
        if !out.is_empty() {
            out.push(RUN_SEP);
        }
        let s = r.style();
        out.push_str(&format!(
            "{},{},{},{},{},{}",
            start,
            start + len,
            style_flags(&s),
            r.color.map(argb).unwrap_or(0),
            r.background.map(argb).unwrap_or(0),
            (r.font.scale * 1000.0).round() as i64,
        ));
    }
    out
}

/// `start,end,align,indent,marker` per paragraph.
fn encode_paragraphs(text: &str, paragraphs: &[ParagraphRun]) -> String {
    let mut out = String::new();
    for p in paragraphs {
        let Some((start, len)) = utf16_range(text, &p.range) else {
            continue;
        };
        if !out.is_empty() {
            out.push(RUN_SEP);
        }
        let s = p.style();
        out.push_str(&format!(
            "{},{},{},{},{}",
            start,
            start + len,
            match s.align {
                ParagraphAlign::Natural => 0,
                ParagraphAlign::Center => 1,
                ParagraphAlign::Trailing => 2,
                ParagraphAlign::Justified => 3,
            },
            (s.indent + f64::from(s.list_level) * LEVEL_INDENT).round() as i64,
            if s.list == ListStyle::None {
                0
            } else {
                MARKER_INDENT as i64
            },
        ));
    }
    out
}

fn push_attributes(h: &AHandle, doc_text: &str, runs: &[TextRun], paragraphs: &[ParagraphRun]) {
    piece::update(h, "runs", &encode_runs(doc_text, runs));
    piece::update(h, "paragraphs", &encode_paragraphs(doc_text, paragraphs));
}

day_core::tls_group! {
    /// The text each editor holds, keyed by its ArkTS frame node — what an attribute or selection
    /// patch converts its byte ranges against, with no round trip into ArkTS.
    static TEXT: SideTable<String> = SideTable::new();

}

fn key(h: &AHandle) -> usize {
    h.0 as usize
}

fn text_of(h: &AHandle) -> String {
    TEXT.with(|t| t.with(key(h), |s| s.clone())).unwrap_or_default()
}

fn make(_backend: &mut ArkUi, p: &EditorProps, id: NodeId) -> AHandle {
    // TEXT LAST, so a document containing the separator still arrives intact: the ArkTS side
    // rejoins everything after the sixth field.
    let props = format!(
        "{base}{SEP}{editable}{SEP}{spell}{SEP}{min}{SEP}{max}{SEP}{placeholder}{SEP}{text}",
        base = day_arkui::font_vp(day_spec::FontSpec::new(p.base)),
        editable = u8::from(p.editable),
        spell = u8::from(p.spellcheck),
        min = p.min_lines,
        max = p.max_lines,
        placeholder = p.placeholder.replace(SEP, " "),
        text = p.doc.text,
    );
    let h = piece::make(KIND, id, &props);
    TEXT.with(|t| t.insert(key(&h), p.doc.text.clone()));
    if !p.doc.runs.is_empty() || !p.doc.paragraphs.is_empty() {
        push_attributes(&h, &p.doc.text, &p.doc.runs, &p.doc.paragraphs);
    }
    h
}

fn update(_backend: &mut ArkUi, h: &AHandle, patch: &EditorPatch) {
    match patch {
        EditorPatch::SetDocument(doc) => {
            piece::update(h, "text", &doc.text);
            TEXT.with(|t| t.with(key(h), |s| *s = doc.text.clone()));
            push_attributes(h, &doc.text, &doc.runs, &doc.paragraphs);
        }
        EditorPatch::SetAttributes(attrs) => {
            // The patch carries the text, so a keystroke's re-highlight encodes its ranges
            // against the string the editor holds RIGHT NOW rather than the one before it.
            TEXT.with(|t| t.with(key(h), |s| *s = attrs.text.clone()));
            push_attributes(h, &attrs.text, &attrs.runs, &attrs.paragraphs);
        }
        EditorPatch::SetSelection(r) => {
            let text = text_of(h);
            if let Some((start, len)) = utf16_range(&text, r) {
                piece::update(h, "selection", &format!("{},{}", start, start + len));
            }
        }
        EditorPatch::SetTypingStyle(s) => {
            piece::update(
                h,
                "typing",
                &format!(
                    "{},{},{},{}",
                    style_flags(s),
                    s.color.map(argb).unwrap_or(0),
                    s.background.map(argb).unwrap_or(0),
                    (s.font.scale * 1000.0).round() as i64,
                ),
            );
        }
        EditorPatch::SetEditable(v) => piece::update(h, "editable", if *v { "1" } else { "0" }),
    }
}

fn release(_backend: &mut ArkUi, h: &AHandle) {
    TEXT.with(|t| {
        t.remove(key(h));
    });
}

day_pieces::renderer!(day_arkui::RENDERERS, ArkUi,
    kind: KIND, props: EditorProps, patch: EditorPatch,
    make: make, update: update, measure: day_pieces::fill_measure, release: release);
