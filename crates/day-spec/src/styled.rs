// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Styled text: the document `label().runs(…)` renders and `day-piece-texteditor` edits
//! (docs/texteditor.md).
//!
//! [`StyledText`] is plain text plus two range vectors over it — [`TextRun`]s for character
//! attributes and [`ParagraphRun`]s for paragraph ones. Text and ranges travel together by
//! construction, because a range only means something against a particular string.
//!
//! Everything an editor's toolbar needs is a pure function here rather than a round trip into the
//! toolkit: [`StyledText::style_of`] answers "what is the selection's style" (with a mixed state
//! where the runs disagree), [`StyledText::apply`] changes it, and [`StyledText::reflow`] moves
//! the ranges over an edit the native editor already made. That is what lets the same logic run
//! on nine backends and be tested on the headless one.

use crate::{Color, Font, FontSpec, FontWeight, TextRun, runs_are_valid};

// ---------------------------------------------------------------------------
// Character attributes
// ---------------------------------------------------------------------------

/// A line under a run.
///
/// An enum rather than a `bool` because every toolkit distinguishes at least single from double,
/// and because a wavy underline is how an app draws its OWN diagnostics — a spelling or grammar
/// mark of its own making — without fighting the platform's checker for the same pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Underline {
    #[default]
    None,
    Single,
    Double,
    Dotted,
    /// The squiggle. A backend with no wavy style draws `Single`.
    Wavy,
}

impl Underline {
    pub fn is_on(self) -> bool {
        self != Underline::None
    }
    /// The toolbar's on/off toggle: `Single` when turning it on, `None` when off.
    pub fn toggled(self) -> Self {
        if self.is_on() {
            Underline::None
        } else {
            Underline::Single
        }
    }
}

/// A [`TextRun`]'s attributes with no range attached: what a toolbar toggles, what an editor
/// applies to a selection, and what it styles the NEXT typed character with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunStyle {
    pub font: FontSpec,
    pub color: Option<Color>,
    pub background: Option<Color>,
    pub underline: Underline,
    pub strikethrough: bool,
    pub link: Option<String>,
}

impl RunStyle {
    /// The style of unstyled text in a document whose base font is `base`.
    pub fn plain(base: Font) -> Self {
        RunStyle {
            font: FontSpec::new(base),
            ..RunStyle::default()
        }
    }
    /// Bold in the sense a toolbar button means it: semibold or heavier.
    pub fn bold(&self) -> bool {
        self.font.weight.is_some_and(|w| w >= FontWeight::Semibold)
    }
    pub fn set_bold(&mut self, on: bool) {
        self.font.weight = on.then_some(FontWeight::Bold);
    }
    pub fn italic(&self) -> bool {
        self.font.italic
    }
    pub fn set_italic(&mut self, on: bool) {
        self.font.italic = on;
    }
}

// ---------------------------------------------------------------------------
// Paragraph attributes
// ---------------------------------------------------------------------------

/// How a paragraph's lines sit in the width they are given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ParagraphAlign {
    /// The reading direction's start edge — left under LTR, right under RTL.
    #[default]
    Natural,
    Center,
    /// The reading direction's end edge.
    Trailing,
    Justified,
}

/// A paragraph's list decoration, if it is a list item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum ListStyle {
    #[default]
    None,
    /// A bullet; the glyph is the platform's.
    Bullet,
    /// A number, which the APP computes — nothing here renumbers a list, because nothing here
    /// knows where the list began or whether the paragraph above belongs to it.
    Ordered(u32),
}

/// A [`ParagraphRun`]'s attributes with no range attached.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ParagraphStyle {
    pub align: ParagraphAlign,
    /// Leading indent in points, from the reading direction's start edge. A list item's total
    /// indent is this plus whatever the platform reserves for its marker.
    pub indent: f64,
    pub space_before: f64,
    pub space_after: f64,
    pub list: ListStyle,
    /// Nesting depth for a list item (0 = outermost). Ignored when `list` is `None`.
    pub list_level: u8,
}

/// One styled paragraph.
///
/// A SECOND range vector alongside [`TextRun`] rather than more fields on one, because these
/// attributes apply to whole paragraphs: folding alignment into a character run would let a
/// program say "center these three words", which no text system can honor and every one of them
/// would resolve differently.
///
/// `range` is a byte range into the same string the runs address and covers whole paragraphs. A
/// gap is a paragraph with the document's own defaults.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ParagraphRun {
    pub range: std::ops::Range<usize>,
    pub align: ParagraphAlign,
    pub indent: f64,
    pub space_before: f64,
    pub space_after: f64,
    pub list: ListStyle,
    pub list_level: u8,
}

impl ParagraphRun {
    pub fn new(range: std::ops::Range<usize>, style: ParagraphStyle) -> Self {
        ParagraphRun {
            range,
            align: style.align,
            indent: style.indent,
            space_before: style.space_before,
            space_after: style.space_after,
            list: style.list,
            list_level: style.list_level,
        }
    }
    pub fn style(&self) -> ParagraphStyle {
        ParagraphStyle {
            align: self.align,
            indent: self.indent,
            space_before: self.space_before,
            space_after: self.space_after,
            list: self.list,
            list_level: self.list_level,
        }
    }
}

// ---------------------------------------------------------------------------
// The document
// ---------------------------------------------------------------------------

/// A styled document: plain text, the character runs over it, and the paragraph attributes over
/// the same offsets.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StyledText {
    pub text: String,
    pub runs: Vec<TextRun>,
    pub paragraphs: Vec<ParagraphRun>,
}

impl From<String> for StyledText {
    fn from(text: String) -> Self {
        StyledText::plain(text)
    }
}

impl From<&str> for StyledText {
    fn from(text: &str) -> Self {
        StyledText::plain(text)
    }
}

impl StyledText {
    /// Unstyled text.
    pub fn plain(text: impl Into<String>) -> Self {
        StyledText {
            text: text.into(),
            runs: Vec::new(),
            paragraphs: Vec::new(),
        }
    }

    /// Parse markdown — the ergonomic way to seed a document
    /// ([`styled_codec`](crate::styled_codec)).
    pub fn markdown(md: &str, base: Font) -> Self {
        crate::styled_codec::markdown_to_styled(
            md,
            crate::styled_codec::DocStyle {
                base,
                ..Default::default()
            },
        )
    }

    /// Parse an HTML fragment.
    pub fn html(html: &str, base: Font) -> Self {
        crate::styled_codec::html_to_styled(
            html,
            crate::styled_codec::DocStyle {
                base,
                ..Default::default()
            },
        )
    }

    /// Parse the RTF subset Day reads (docs/texteditor.md).
    pub fn rtf(rtf: &str, base: Font) -> Self {
        crate::styled_codec::rtf_to_styled(
            rtf,
            crate::styled_codec::DocStyle {
                base,
                ..Default::default()
            },
        )
    }

    /// Write this document as markdown / HTML / RTF. Each is lossy in the ways
    /// [`styled_codec`](crate::styled_codec) documents.
    pub fn to_markdown(&self, base: Font) -> String {
        crate::styled_codec::styled_to_markdown(
            self,
            crate::styled_codec::DocStyle {
                base,
                ..Default::default()
            },
        )
    }

    pub fn to_html(&self, base: Font) -> String {
        crate::styled_codec::styled_to_html(
            self,
            crate::styled_codec::DocStyle {
                base,
                ..Default::default()
            },
        )
    }

    pub fn to_rtf(&self, base: Font) -> String {
        crate::styled_codec::styled_to_rtf(
            self,
            crate::styled_codec::DocStyle {
                base,
                ..Default::default()
            },
        )
    }

    /// Text with character runs and no paragraph attributes — what a label carries today.
    pub fn new(text: impl Into<String>, runs: Vec<TextRun>) -> Self {
        StyledText {
            text: text.into(),
            runs,
            paragraphs: Vec::new(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Are both range vectors well formed for the text (ascending, non-overlapping, in bounds,
    /// on character boundaries)?
    pub fn validate(&self) -> Result<(), String> {
        runs_are_valid(&self.text, &self.runs)?;
        paragraphs_are_valid(&self.text, &self.paragraphs)
    }

    /// The whole text as consecutive `(range, style)` segments, unstyled gaps included as `base`.
    ///
    /// One view that the query and the mutation below are both written against, so neither has to
    /// reason about where a run ends and a gap begins.
    pub fn segments(&self, base: Font) -> Vec<(std::ops::Range<usize>, RunStyle)> {
        let plain = RunStyle::plain(base);
        let mut out = Vec::with_capacity(self.runs.len() * 2 + 1);
        let mut at = 0usize;
        for r in &self.runs {
            if r.range.start > at {
                out.push((at..r.range.start, plain.clone()));
            }
            if !r.range.is_empty() {
                out.push((r.range.clone(), r.style()));
            }
            at = at.max(r.range.end);
        }
        if at < self.text.len() {
            out.push((at..self.text.len(), plain));
        }
        out
    }

    /// The style in effect across `range`.
    ///
    /// Where the segments under it disagree, the differing attribute comes back as its DEFAULT
    /// rather than as one of the values. That is what a toolbar renders as its mixed state, and
    /// it is what makes pressing the button set the whole selection instead of toggling half of
    /// it.
    ///
    /// A collapsed caret takes the style of the segment that ENDS at it, preferring that over the
    /// one it starts: typing after a bold word continues bold, which is what every editor does.
    pub fn style_of(&self, range: std::ops::Range<usize>, base: Font) -> RunStyle {
        let segs = self.segments(base);
        if range.is_empty() {
            let at = range.start;
            return segs
                .iter()
                .find(|(r, _)| r.start < at && r.end >= at)
                .or_else(|| segs.iter().find(|(r, _)| r.contains(&at)))
                .map(|(_, s)| s.clone())
                .unwrap_or_else(|| RunStyle::plain(base));
        }
        let mut acc: Option<RunStyle> = None;
        for (r, s) in &segs {
            if r.end <= range.start || r.start >= range.end {
                continue;
            }
            acc = Some(merge_style(acc, s));
        }
        acc.unwrap_or_else(|| RunStyle::plain(base))
    }

    /// Apply `f` to the style of every segment overlapping `range`, splitting at its boundaries so
    /// only the selected text changes, then coalescing what came out.
    pub fn apply(&mut self, range: std::ops::Range<usize>, base: Font, f: impl Fn(&mut RunStyle)) {
        if range.start >= range.end || range.start >= self.text.len() {
            return;
        }
        let range = range.start..range.end.min(self.text.len());
        let mut out: Vec<TextRun> = Vec::new();
        for (r, s) in self.segments(base) {
            // Up to three pieces per segment: before the selection, inside it, after it.
            for (piece, inside) in [
                (r.start..r.end.min(range.start), false),
                (r.start.max(range.start)..r.end.min(range.end), true),
                (r.start.max(range.end)..r.end, false),
            ] {
                if piece.start >= piece.end {
                    continue;
                }
                let mut style = s.clone();
                if inside {
                    f(&mut style);
                }
                out.push(TextRun::styled(piece, style));
            }
        }
        self.runs = coalesce_runs(out, base);
    }

    /// The paragraph style in effect across `range`, by the same mixed-state rule as
    /// [`StyledText::style_of`].
    pub fn paragraph_style_of(&self, range: std::ops::Range<usize>) -> ParagraphStyle {
        let mut acc: Option<ParagraphStyle> = None;
        for (start, end) in paragraph_bounds(&self.text) {
            if !touches(start, end, &range) {
                continue;
            }
            let style = self.paragraph_at(start, end);
            acc = Some(match acc {
                None => style,
                Some(a) if a == style => a,
                Some(_) => ParagraphStyle::default(),
            });
        }
        acc.unwrap_or_default()
    }

    /// Apply `f` to every paragraph the selection touches. Paragraph boundaries are `\n`, so a
    /// caret anywhere inside a paragraph styles the whole of it — which is what every editor does.
    pub fn apply_paragraph(
        &mut self,
        range: std::ops::Range<usize>,
        f: impl Fn(&mut ParagraphStyle),
    ) {
        let mut out: Vec<ParagraphRun> = Vec::new();
        for (start, end) in paragraph_bounds(&self.text) {
            let mut style = self.paragraph_at(start, end);
            if touches(start, end, &range) {
                f(&mut style);
            }
            if style != ParagraphStyle::default() {
                out.push(ParagraphRun::new(start..end, style));
            }
        }
        self.paragraphs = out;
    }

    /// The style recorded for the paragraph spanning `start..end`, or the default.
    fn paragraph_at(&self, start: usize, end: usize) -> ParagraphStyle {
        self.paragraphs
            .iter()
            .find(|p| p.range.start <= start && p.range.end >= end.min(p.range.end).max(start + 1))
            .map(|p| p.style())
            .unwrap_or_default()
    }

    /// Move the ranges over a CHARACTER edit the native editor already made: `removed` bytes at
    /// `offset` replaced by `inserted` bytes.
    ///
    /// This is why a keystroke does not have to ship the whole document back. Ranges before the
    /// edit are untouched, ranges after it shift, a range containing it grows or shrinks, and a
    /// range the edit emptied is dropped — the rule every attributed-string implementation uses,
    /// exact, and O(runs) rather than O(document).
    ///
    /// `self.text` must ALREADY be the new text; this only moves ranges.
    pub fn reflow(&mut self, offset: usize, removed: usize, inserted: usize) {
        let shift = |p: usize| -> usize {
            if p <= offset {
                p
            } else if p >= offset + removed {
                p - removed + inserted
            } else {
                offset // inside the deleted span: collapse to the edit point
            }
        };
        for r in &mut self.runs {
            r.range = shift(r.range.start)..shift(r.range.end);
        }
        for p in &mut self.paragraphs {
            p.range = shift(p.range.start)..shift(p.range.end);
        }
        // A run the edit ended INSIDE grows over the insertion, so typing at the end of a bold
        // word stays bold. Applied after the shift so the comparison is in new coordinates.
        if inserted > 0 {
            for r in &mut self.runs {
                if r.range.end == offset && r.range.start < offset {
                    r.range.end = offset + inserted;
                }
            }
            for p in &mut self.paragraphs {
                if p.range.end == offset && p.range.start < offset {
                    p.range.end = offset + inserted;
                }
            }
        }
        self.clamp();
    }

    /// Clamp every range into the text and onto character boundaries, dropping what cannot be
    /// saved. The last defense before a range reaches a backend and panics an `str` slice.
    pub fn clamp(&mut self) {
        let text = std::mem::take(&mut self.text);
        let fix = |p: usize| -> usize {
            let mut p = p.min(text.len());
            while p > 0 && !text.is_char_boundary(p) {
                p -= 1;
            }
            p
        };
        for r in &mut self.runs {
            r.range = fix(r.range.start)..fix(r.range.end);
        }
        for p in &mut self.paragraphs {
            p.range = fix(p.range.start)..fix(p.range.end);
        }
        self.text = text;
        self.runs.retain(|r| r.range.start < r.range.end);
        self.paragraphs.retain(|p| p.range.start < p.range.end);
    }

    /// Replace `range` with `text`, moving every other range over the edit. The app-side twin of
    /// what a keystroke does, for a programmatic insert or delete.
    pub fn splice(&mut self, range: std::ops::Range<usize>, text: &str) {
        let range = range.start.min(self.text.len())..range.end.min(self.text.len());
        if !self.text.is_char_boundary(range.start) || !self.text.is_char_boundary(range.end) {
            return;
        }
        let removed = range.end.saturating_sub(range.start);
        self.text.replace_range(range.clone(), text);
        self.reflow(range.start, removed, text.len());
    }
}

/// Fold `b` into the accumulated selection style: an attribute both agree on survives, one they
/// differ on resets to its default — the toolbar's mixed state.
fn merge_style(acc: Option<RunStyle>, b: &RunStyle) -> RunStyle {
    let Some(a) = acc else { return b.clone() };
    RunStyle {
        font: FontSpec {
            style: if a.font.style == b.font.style {
                a.font.style
            } else {
                Font::Body
            },
            weight: if a.font.weight == b.font.weight {
                a.font.weight
            } else {
                None
            },
            italic: a.font.italic && b.font.italic,
            tabular: a.font.tabular && b.font.tabular,
            monospace: a.font.monospace && b.font.monospace,
            scale: if a.font.scale == b.font.scale {
                a.font.scale
            } else {
                1.0
            },
        },
        color: if a.color == b.color { a.color } else { None },
        background: if a.background == b.background {
            a.background
        } else {
            None
        },
        underline: if a.underline == b.underline {
            a.underline
        } else {
            Underline::None
        },
        strikethrough: a.strikethrough && b.strikethrough,
        link: if a.link == b.link { a.link } else { None },
    }
}

/// Does the paragraph `start..end` fall under `sel`? A collapsed caret counts against the
/// paragraph it sits in, including at its very end.
fn touches(start: usize, end: usize, sel: &std::ops::Range<usize>) -> bool {
    if sel.start == sel.end {
        return sel.start >= start && sel.start <= end;
    }
    start < sel.end && end > sel.start
}

/// Merge adjacent runs whose styles came out equal, and drop runs that say nothing beyond `base`.
///
/// Every `apply` produces fragments; without this a document accumulates one run per edit until
/// the run vector is longer than the text it describes.
pub fn coalesce_runs(runs: Vec<TextRun>, base: Font) -> Vec<TextRun> {
    let plain = RunStyle::plain(base);
    let mut out: Vec<TextRun> = Vec::with_capacity(runs.len());
    for r in runs {
        if r.range.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(prev) if prev.range.end == r.range.start && prev.style() == r.style() => {
                prev.range.end = r.range.end;
            }
            _ => out.push(r),
        }
    }
    out.retain(|r| r.style() != plain);
    out
}

/// The `\n`-delimited paragraphs of `text`, as byte ranges INCLUDING the terminator.
///
/// An empty string has no paragraphs. A trailing newline makes an empty final paragraph, the way
/// an editor shows a blank last line — so `"a\n"` is two paragraphs, not one.
pub fn paragraph_bounds(text: &str) -> Vec<(usize, usize)> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0usize;
    for (i, b) in text.bytes().enumerate() {
        if b == b'\n' {
            out.push((start, i + 1));
            start = i + 1;
        }
    }
    out.push((start, text.len()));
    out
}

/// Are these paragraph runs well formed for `text` — [`runs_are_valid`]'s rules, applied to the
/// second range vector.
pub fn paragraphs_are_valid(text: &str, paragraphs: &[ParagraphRun]) -> Result<(), String> {
    let mut prev_end = 0usize;
    for (i, p) in paragraphs.iter().enumerate() {
        if p.range.start < prev_end {
            return Err(format!(
                "paragraph {i} starts at {} but the previous ends at {prev_end} — paragraph runs \
                 must be ascending and non-overlapping",
                p.range.start
            ));
        }
        if p.range.end > text.len() {
            return Err(format!(
                "paragraph {i} ends at {} but the text is {} bytes",
                p.range.end,
                text.len()
            ));
        }
        if p.range.start > p.range.end {
            return Err(format!("paragraph {i} has an inverted range {:?}", p.range));
        }
        if !text.is_char_boundary(p.range.start) || !text.is_char_boundary(p.range.end) {
            return Err(format!(
                "paragraph {i}'s range {:?} splits a multi-byte character",
                p.range
            ));
        }
        prev_end = p.range.end;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bold_run(range: std::ops::Range<usize>) -> TextRun {
        TextRun::styled(range, {
            let mut s = RunStyle::plain(Font::Body);
            s.set_bold(true);
            s
        })
    }

    fn doc() -> StyledText {
        // "hello world" with "hello" bold.
        StyledText::new("hello world", vec![bold_run(0..5)])
    }

    #[test]
    fn segments_cover_the_whole_text_including_gaps() {
        let segs = doc().segments(Font::Body);
        assert_eq!(
            segs.iter().map(|(r, _)| r.clone()).collect::<Vec<_>>(),
            vec![0..5, 5..11]
        );
        assert!(segs[0].1.bold());
        assert!(!segs[1].1.bold());
    }

    #[test]
    fn style_of_a_uniform_selection_is_that_style() {
        assert!(doc().style_of(0..5, Font::Body).bold());
        assert!(!doc().style_of(5..11, Font::Body).bold());
    }

    #[test]
    fn style_of_a_mixed_selection_resets_what_differs() {
        // Half bold, half not: the toolbar shows "not bold", so pressing it bolds everything.
        let s = doc().style_of(0..11, Font::Body);
        assert!(!s.bold(), "a mixed weight comes back as the default");
    }

    #[test]
    fn a_caret_takes_the_style_of_the_run_it_ends() {
        // Typing at offset 5 — right after "hello" — continues bold.
        assert!(doc().style_of(5..5, Font::Body).bold());
        // And at 11, after the plain tail, it does not.
        assert!(!doc().style_of(11..11, Font::Body).bold());
        // At offset 0 there is nothing behind the caret, so it takes the run AHEAD instead —
        // typing at the very start of a document that begins bold gives bold, which is what
        // TextEdit, Word and every web editor do.
        assert!(doc().style_of(0..0, Font::Body).bold());
    }

    #[test]
    fn apply_splits_runs_at_the_selection_boundaries() {
        let mut d = doc();
        d.apply(3..8, Font::Body, |s| s.set_italic(true));
        // bold 0..3, bold+italic 3..5, italic 5..8, plain 8..11 (dropped as plain).
        let got: Vec<_> = d
            .runs
            .iter()
            .map(|r| (r.range.clone(), r.style().bold(), r.style().italic()))
            .collect();
        assert_eq!(
            got,
            vec![(0..3, true, false), (3..5, true, true), (5..8, false, true),]
        );
    }

    #[test]
    fn apply_coalesces_and_drops_plain() {
        let mut d = doc();
        d.apply(0..11, Font::Body, |s| s.set_bold(false));
        assert!(
            d.runs.is_empty(),
            "unbolding everything leaves nothing to say: {:?}",
            d.runs
        );
    }

    #[test]
    fn reflow_moves_ranges_over_an_insert() {
        let mut d = doc();
        // Type "XX" at offset 8 (inside " world").
        d.text = "hello woXXrld".into();
        d.reflow(8, 0, 2);
        assert_eq!(d.runs[0].range, 0..5, "a run before the edit is untouched");

        let mut d = doc();
        // Type at 2, inside the bold run: it grows.
        d.text = "heXXllo world".into();
        d.reflow(2, 0, 2);
        assert_eq!(d.runs[0].range, 0..7);
    }

    #[test]
    fn typing_at_the_end_of_a_run_continues_it() {
        let mut d = doc();
        d.text = "helloX world".into();
        d.reflow(5, 0, 1);
        assert_eq!(d.runs[0].range, 0..6, "the bold run took the new character");
    }

    #[test]
    fn reflow_shrinks_and_drops_over_a_delete() {
        let mut d = doc();
        // Delete "ello " (1..6): the bold run shrinks to 0..1.
        d.text = "hworld".into();
        d.reflow(1, 5, 0);
        assert_eq!(d.runs[0].range, 0..1);

        let mut d = doc();
        // Delete the whole bold word: the run goes away rather than becoming empty.
        d.text = " world".into();
        d.reflow(0, 5, 0);
        assert!(d.runs.is_empty(), "{:?}", d.runs);
    }

    #[test]
    fn splice_edits_text_and_ranges_together() {
        let mut d = doc();
        d.splice(5..5, ",");
        assert_eq!(d.text, "hello, world");
        assert_eq!(d.runs[0].range, 0..6);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn clamp_pulls_ranges_off_multi_byte_boundaries() {
        let mut d = StyledText::new("héllo", vec![bold_run(0..2)]);
        d.text = "h".into(); // as if the tail were deleted without a reflow
        d.clamp();
        assert!(d.validate().is_ok());
        assert!(d.runs.iter().all(|r| r.range.end <= 1));
    }

    #[test]
    fn paragraph_bounds_include_the_terminator_and_the_blank_last_line() {
        assert_eq!(paragraph_bounds(""), Vec::new());
        assert_eq!(paragraph_bounds("a"), vec![(0, 1)]);
        assert_eq!(paragraph_bounds("a\n"), vec![(0, 2), (2, 2)]);
        assert_eq!(paragraph_bounds("a\nbb\nc"), vec![(0, 2), (2, 5), (5, 6)]);
    }

    #[test]
    fn apply_paragraph_styles_the_whole_paragraph_a_caret_sits_in() {
        let mut d = StyledText::plain("one\ntwo\nthree");
        d.apply_paragraph(5..5, |p| p.align = ParagraphAlign::Center);
        assert_eq!(d.paragraphs.len(), 1);
        assert_eq!(d.paragraphs[0].range, 4..8, "the whole second paragraph");
        assert_eq!(d.paragraphs[0].align, ParagraphAlign::Center);
        assert_eq!(d.paragraph_style_of(5..5).align, ParagraphAlign::Center);
        assert_eq!(
            d.paragraph_style_of(0..13).align,
            ParagraphAlign::Natural,
            "a mixed selection reports the default"
        );
        assert!(d.validate().is_ok());
    }

    #[test]
    fn paragraph_ranges_reflow_with_the_text() {
        let mut d = StyledText::plain("one\ntwo");
        d.apply_paragraph(5..5, |p| p.list = ListStyle::Bullet);
        assert_eq!(d.paragraphs[0].range, 4..7);
        d.splice(0..0, "XY");
        assert_eq!(d.paragraphs[0].range, 6..9);
        assert!(d.validate().is_ok());
    }

    #[test]
    fn font_scale_resolves_against_the_platforms_style_size() {
        let s = FontSpec::new(Font::Body).scaled(1.5);
        assert_eq!(s.resolved_points(16.0), 24.0);
        // An absolute style carries its own size and ignores the platform's.
        let abs = FontSpec::new(Font::System(14.0));
        assert_eq!(abs.resolved_points(16.0), 14.0);
        assert_eq!(abs.scaled(2.0).resolved_points(16.0), 28.0);
    }
}
