// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Inline markdown → styled runs, parsed at RUNTIME (docs/markdown.md).
//!
//! Runtime rather than a compile-time macro because the string a label shows is usually chosen at
//! run time: a translation picked from the locale bundle, a value from the network, text the user
//! typed. A macro can only see literals, which is the least interesting case.
//!
//! The grammar is the inline subset — what fits in one label. Block constructs (headings, lists,
//! quotes, tables) are layout, and layout is `column`/`form`/`list`, not a text attribute.

use crate::{Color, Font, FontSpec, FontWeight, TextRun, Underline};

/// The inline styles the parser recognizes, as a bitmask carried down the nesting stack.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
struct Styles {
    bold: bool,
    italic: bool,
    code: bool,
    strike: bool,
}

impl Styles {
    fn is_plain(self, link: Option<&String>) -> bool {
        self == Styles::default() && link.is_none()
    }
}

/// Parse inline markdown into the plain text a label shows plus the runs that style it.
///
/// Recognized:
///
/// | Markdown | Run |
/// | --- | --- |
/// | `**bold**`, `__bold__` | bold |
/// | `*italic*`, `_italic_` | italic |
/// | `` `code` `` | the monospaced face |
/// | `~~strike~~` | struck through |
/// | `[text](url)` | a link run |
/// | `\*` | the literal character |
///
/// `base` is the font the styles vary from — pass the label's own, so a bold run inside a
/// `Footnote` paragraph stays footnote-sized.
///
/// Anything unrecognized is text: an unclosed `**`, a stray `_` inside a word, a `[` with no
/// `](…)` after it. Markdown's own rule, and the one that keeps a half-typed string in a live
/// editor from flickering between readings.
pub fn parse(md: &str, base: Font) -> (String, Vec<TextRun>) {
    let mut p = Parser {
        src: md.as_bytes(),
        at: 0,
        base,
        out: String::with_capacity(md.len()),
        runs: Vec::new(),
        link_color: LINK_COLOR,
    };
    p.inline(Styles::default(), None, None);
    (p.out, p.runs)
}

/// The color a link run takes when the markdown does not say otherwise: the platform-ish tint
/// blue, matching the `link()` piece so the two read as the same affordance.
const LINK_COLOR: Color = Color::rgb(0.0, 0.478, 1.0);

struct Parser<'a> {
    src: &'a [u8],
    at: usize,
    base: Font,
    out: String,
    runs: Vec<TextRun>,
    link_color: Color,
}

impl Parser<'_> {
    /// Parse until `close` is found (or the end of input when it is `None`), emitting runs for
    /// everything under the current style set. Returns whether `close` was actually reached.
    fn inline(&mut self, styles: Styles, link: Option<&String>, close: Option<&[u8]>) -> bool {
        // Text accumulated under THIS style set, flushed as one run whenever a marker interrupts.
        let mut start = self.out.len();
        let mut closed = false;
        while self.at < self.src.len() {
            if let Some(c) = close
                && self.src[self.at..].starts_with(c)
            {
                self.at += c.len();
                closed = true;
                break;
            }
            let b = self.src[self.at];
            // A backslash escapes the next byte, which is always ASCII punctuation in this
            // grammar — so pushing it raw cannot split a character.
            if b == b'\\'
                && let Some(&next) = self.src.get(self.at + 1)
                && next.is_ascii_punctuation()
            {
                self.out.push(next as char);
                self.at += 2;
                continue;
            }
            // Inside a code span nothing else is a marker: `**` is two asterisks.
            if styles.code {
                self.push_byte();
                continue;
            }
            let consumed = match b {
                b'`' => self.code_span(styles, link, &mut start),
                b'*' | b'_' => self.emphasis(b, styles, link, &mut start),
                b'~' => self.strike(styles, link, &mut start),
                b'[' => self.link(styles, link, &mut start),
                _ => false,
            };
            if !consumed {
                self.push_byte();
            }
        }
        self.flush(start, styles, link);
        closed
    }

    /// Copy one byte of source to the output. Bytes rather than chars: every marker in this
    /// grammar is ASCII, so a multi-byte character simply arrives one byte at a time and lands
    /// intact.
    fn push_byte(&mut self) {
        // SAFETY-free equivalent: the byte is part of a valid UTF-8 string and is copied in
        // order, so `out` stays valid UTF-8. `push_str` on a subslice keeps that explicit.
        let b = self.at;
        self.at += 1;
        while self.at < self.src.len() && (self.src[self.at] & 0xC0) == 0x80 {
            self.at += 1;
        }
        if let Ok(s) = std::str::from_utf8(&self.src[b..self.at]) {
            self.out.push_str(s);
        }
    }

    /// Emit the text collected since `start` as a run, unless it is plain or empty.
    fn flush(&mut self, start: usize, styles: Styles, link: Option<&String>) {
        let end = self.out.len();
        if end == start || styles.is_plain(link) {
            return;
        }
        self.runs.push(TextRun {
            range: start..end,
            font: FontSpec {
                style: self.base,
                weight: styles.bold.then_some(FontWeight::Bold),
                italic: styles.italic,
                monospace: styles.code,
                ..Default::default()
            },
            color: link.is_some().then_some(self.link_color),
            // A link run is underlined too now that a run can say so: every platform's own
            // link rendering draws one, and Day's did not because `TextRun` had no way to.
            underline: if link.is_some() {
                Underline::Single
            } else {
                Underline::None
            },
            strikethrough: styles.strike,
            link: link.cloned(),
            ..TextRun::default()
        });
    }

    /// A nested span: flush what came before, parse the inside under `inner`, and resume. Rolls
    /// the whole attempt back if the closing marker never arrives, so the opener stays literal.
    fn nested(
        &mut self,
        open: usize,
        close: &[u8],
        outer: Styles,
        inner: Styles,
        link: Option<&String>,
        start: &mut usize,
    ) -> bool {
        let (mark_at, mark_out) = (self.at, self.out.len());
        self.at += open;
        let before = self.runs.len();
        if !self.inline(inner, link, Some(close)) {
            // Unclosed: undo everything the attempt produced and let the marker be text.
            self.runs.truncate(before);
            self.out.truncate(mark_out);
            self.at = mark_at;
            return false;
        }
        // The nested text landed after `mark_out`, so the text still pending under the OUTER
        // styles is everything from `start` up to there. Flush it, then resume after the span.
        let outer_start = std::mem::replace(start, self.out.len());
        self.flush_range(outer_start, mark_out, outer, link);
        true
    }

    /// Flush an explicit range under the OUTER styles (the ones in force before this span).
    fn flush_range(&mut self, start: usize, end: usize, styles: Styles, link: Option<&String>) {
        if end <= start || styles.is_plain(link) {
            return;
        }
        // Runs must stay ascending: the outer text precedes everything the nested parse pushed.
        let at = self
            .runs
            .iter()
            .position(|r| r.range.start >= end)
            .unwrap_or(self.runs.len());
        let run = TextRun {
            range: start..end,
            font: FontSpec {
                style: self.base,
                weight: styles.bold.then_some(FontWeight::Bold),
                italic: styles.italic,
                monospace: styles.code,
                ..Default::default()
            },
            color: link.is_some().then_some(self.link_color),
            underline: if link.is_some() {
                Underline::Single
            } else {
                Underline::None
            },
            strikethrough: styles.strike,
            link: link.cloned(),
            ..TextRun::default()
        };
        self.runs.insert(at, run);
    }

    fn code_span(&mut self, styles: Styles, link: Option<&String>, start: &mut usize) -> bool {
        let mut inner = styles;
        inner.code = true;
        self.nested(1, b"`", styles, inner, link, start)
    }

    fn emphasis(
        &mut self,
        marker: u8,
        styles: Styles,
        link: Option<&String>,
        start: &mut usize,
    ) -> bool {
        let double = self.src.get(self.at + 1) == Some(&marker);
        let mut inner = styles;
        if double {
            inner.bold = true;
            self.nested(2, &[marker, marker], styles, inner, link, start)
        } else {
            inner.italic = true;
            self.nested(1, &[marker], styles, inner, link, start)
        }
    }

    fn strike(&mut self, styles: Styles, link: Option<&String>, start: &mut usize) -> bool {
        if self.src.get(self.at + 1) != Some(&b'~') {
            return false;
        }
        let mut inner = styles;
        inner.strike = true;
        self.nested(2, b"~~", styles, inner, link, start)
    }

    /// `[text](url)`. The label is parsed for nested styles; the target is taken literally, since
    /// a URL's own punctuation is not markup.
    fn link(&mut self, styles: Styles, outer: Option<&String>, start: &mut usize) -> bool {
        let Some(close) = find(self.src, self.at + 1, b']') else {
            return false;
        };
        if self.src.get(close + 1) != Some(&b'(') {
            return false;
        }
        let Some(paren) = find(self.src, close + 2, b')') else {
            return false;
        };
        let Ok(url) = std::str::from_utf8(&self.src[close + 2..paren]) else {
            return false;
        };
        // A link inside a link is not a thing; the outer target wins.
        let target = outer.cloned().unwrap_or_else(|| url.trim().to_string());
        let mark_out = self.out.len();
        let saved_at = self.at;
        self.at += 1;
        let before = self.runs.len();
        if !self.inline(styles, Some(&target), Some(b"]")) {
            self.runs.truncate(before);
            self.out.truncate(mark_out);
            self.at = saved_at;
            return false;
        }
        self.at = paren + 1;
        let outer_end = std::mem::replace(start, self.out.len());
        self.flush_range(outer_end, mark_out, styles, outer);
        true
    }
}

fn find(src: &[u8], from: usize, b: u8) -> Option<usize> {
    (from..src.len()).find(|&i| src[i] == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(md: &str) -> (String, Vec<TextRun>) {
        parse(md, Font::Body)
    }

    #[test]
    fn plain_text_produces_no_runs() {
        let (t, r) = render("nothing to see");
        assert_eq!(t, "nothing to see");
        assert!(r.is_empty());
    }

    #[test]
    fn the_markers_are_stripped_from_the_text() {
        let (t, _) = render("**a** *b* `c` ~~d~~ [e](u)");
        assert_eq!(t, "a b c d e");
    }

    #[test]
    fn each_style_lands_on_its_own_word() {
        let (t, r) = render("**bold** and *italic*");
        assert_eq!(t, "bold and italic");
        assert_eq!(r.len(), 2);
        assert_eq!(&t[r[0].range.clone()], "bold");
        assert_eq!(r[0].font.weight, Some(FontWeight::Bold));
        assert_eq!(&t[r[1].range.clone()], "italic");
        assert!(r[1].font.italic);
    }

    #[test]
    fn code_spans_are_literal_inside() {
        let (t, r) = render("`**not bold**`");
        assert_eq!(t, "**not bold**");
        assert_eq!(r.len(), 1);
        assert!(r[0].font.monospace);
        assert_eq!(r[0].font.weight, None);
    }

    #[test]
    fn a_link_carries_its_target_and_a_color() {
        let (t, r) = render("see [the docs](https://day.dev/x) now");
        assert_eq!(t, "see the docs now");
        assert_eq!(r.len(), 1);
        assert_eq!(&t[r[0].range.clone()], "the docs");
        assert_eq!(r[0].link.as_deref(), Some("https://day.dev/x"));
        assert!(r[0].color.is_some());
    }

    #[test]
    fn styles_nest() {
        let (t, r) = render("**bold with *italic* inside**");
        assert_eq!(t, "bold with italic inside");
        // Three runs: the bold head, the bold-italic middle, the bold tail.
        assert_eq!(r.len(), 3);
        assert!(r.iter().all(|x| x.font.weight == Some(FontWeight::Bold)));
        assert_eq!(r.iter().filter(|x| x.font.italic).count(), 1);
        assert_eq!(&t[r[1].range.clone()], "italic");
        assert!(crate::runs_are_valid(&t, &r).is_ok());
    }

    #[test]
    fn a_link_can_hold_styles() {
        let (t, r) = render("[**bold** link](u)");
        assert_eq!(t, "bold link");
        assert!(r.iter().all(|x| x.link.as_deref() == Some("u")));
        assert_eq!(r.iter().filter(|x| x.font.weight.is_some()).count(), 1);
        assert!(crate::runs_are_valid(&t, &r).is_ok());
    }

    #[test]
    fn an_unclosed_marker_stays_literal() {
        for md in ["**half bold", "a * b", "`open", "~~nope", "[text](no-close"] {
            let (t, r) = render(md);
            assert_eq!(t, md, "unclosed marker should be text: {md}");
            assert!(r.is_empty(), "unclosed marker should not style: {md}");
        }
    }

    #[test]
    fn backslash_escapes_a_marker() {
        let (t, r) = render(r"\*not italic\*");
        assert_eq!(t, "*not italic*");
        assert!(r.is_empty());
    }

    #[test]
    fn multibyte_text_keeps_its_ranges_on_boundaries() {
        let (t, r) = render("héllo **wörld** 🌍 ~~ok~~");
        assert_eq!(t, "héllo wörld 🌍 ok");
        assert_eq!(&t[r[0].range.clone()], "wörld");
        assert_eq!(&t[r[1].range.clone()], "ok");
        assert!(crate::runs_are_valid(&t, &r).is_ok());
    }

    #[test]
    fn every_parse_produces_valid_runs() {
        // The parser's output feeds `label().runs()`, which rejects overlapping or misordered
        // runs — so validity is the parser's contract, checked over a spread of shapes.
        for md in [
            "",
            "plain",
            "**a**b*c*`d`~~e~~[f](g)",
            "**a *b* c** d",
            "[a **b** c](u) d",
            "~~**both**~~",
            "`a` `b` `c`",
            "**",
            "*_*_*",
            r"\[not a link\](x)",
        ] {
            let (t, r) = render(md);
            assert!(
                crate::runs_are_valid(&t, &r).is_ok(),
                "invalid runs for {md:?}: {r:?}"
            );
        }
    }
}
