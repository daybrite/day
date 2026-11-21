// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Markdown, parsed at run time (docs/text-runs.md, docs/markdown-view.md).
//!
//! Run time rather than a compile-time macro because the string a label shows is usually chosen
//! at run time: a translation picked from the locale bundle, a value from the network, text the
//! user typed. A macro can only see literals, which is the least interesting case.
//!
//! Two layers. `parse` reads the inline grammar, what fits in one label, into styled runs.
//! `MarkdownDocument` reads the block grammar (headings, lists, quotes, fenced code, rules)
//! into a flat list of `MarkdownBlock`s, each of which is one label's worth of inline source,
//! and re-parses only from the last top-level block boundary before an edit, so a reply that
//! arrives a token at a time costs one block per token. The `markdown()` piece in day-pieces
//! lays those blocks out with Day's own containers.

use crate::{Decoration, Font, FontSpec, FontWeight, Ink, ListStyle, SystemColor, TextRun};

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
/// `base` is the font the styles vary from. Pass the label's font, so a bold run inside a
/// `Footnote` paragraph stays footnote-sized.
///
/// Anything unrecognized is text: an unclosed `**`, a stray `_` inside a word, a `[` with no
/// `](…)` after it. Markdown's own rule, and the one that keeps a half-typed string in a live
/// editor from flickering between readings.
#[must_use]
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

/// The color a link run takes when the markdown does not say otherwise: the platform's link
/// color, matching the `link()` piece so the two read as the same affordance.
const LINK_COLOR: Ink = Ink::system(SystemColor::Link);

struct Parser<'a> {
    src: &'a [u8],
    at: usize,
    base: Font,
    out: String,
    runs: Vec<TextRun>,
    link_color: Ink,
}

impl Parser<'_> {
    /// Parse until `close` is found (or the end of input when it is `None`), emitting runs for
    /// everything under the current style set. Returns whether `close` was actually reached.
    fn inline(&mut self, styles: Styles, link: Option<&String>, close: Option<&[u8]>) -> bool {
        // Text accumulated under this style set, flushed as one run whenever a marker interrupts.
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
            // grammar, so pushing it raw cannot split a character.
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
    /// grammar is ASCII, so a multi-byte character arrives one byte at a time and lands intact.
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
            underline: link.is_some().then_some(Decoration::SINGLE),
            strikethrough: styles.strike.then_some(Decoration::SINGLE),
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
        // The nested text landed after `mark_out`, so the text still pending under the outer
        // styles is everything from `start` up to there. Flush it, then resume after the span.
        let outer_start = std::mem::replace(start, self.out.len());
        self.flush_range(outer_start, mark_out, outer, link);
        true
    }

    /// Flush an explicit range under the outer styles (the ones in force before this span).
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
            underline: link.is_some().then_some(Decoration::SINGLE),
            strikethrough: styles.strike.then_some(Decoration::SINGLE),
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

// ---------------------------------------------------------------------------
// Block Markdown
// ---------------------------------------------------------------------------

/// What kind of block a [`MarkdownBlock`] is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MarkdownBlockKind {
    /// Running text. [`MarkdownBlock::text`] is inline Markdown.
    Paragraph,
    /// An ATX heading, level 1 to 6. [`MarkdownBlock::text`] is inline Markdown.
    Heading(u8),
    /// A fenced code block. [`MarkdownBlock::text`] is the code, verbatim, and `language` is
    /// the first word after the opening fence (empty when there is none).
    Code { language: String },
    /// A thematic break (`---`, `***`, `___`). [`MarkdownBlock::text`] is empty.
    Rule,
}

/// One leaf block of a Markdown document, with the containers around it flattened into
/// depths: a paragraph inside a list item inside a quote is one block with
/// `quote_depth == 1` and `list_depth == 1`.
///
/// Flat, because a view lays a document out as a column of rows and updates it row by row. A
/// nested tree would make "the last row changed" a walk through every enclosing container.
#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownBlock {
    pub kind: MarkdownBlockKind,
    /// Inline Markdown for a paragraph or heading (give it to [`parse`]), the literal code
    /// for a code block, empty for a rule.
    pub text: String,
    /// How many block quotes enclose this block.
    pub quote_depth: u8,
    /// How many list items enclose this block: 0 outside a list, 1 in a top-level item.
    pub list_depth: u8,
    /// The marker to draw beside this block: `Bullet` or `Ordered(n)` on the first block of a
    /// list item, `None` everywhere else (a second paragraph in the same item, text outside
    /// a list). An ordered list counts up from the number its first item was written with.
    pub marker: ListStyle,
    /// A blank line separated this block from the one before it. A view uses it to tell a
    /// tight list from a loose one.
    pub blank_before: bool,
}

/// A Markdown document parsed into [`MarkdownBlock`]s, which re-parses only what an edit
/// could have changed.
///
/// [`MarkdownDocument::update`] finds the first byte that differs from the source it holds,
/// goes back to the last top-level block boundary before it, and parses from there. For text
/// that grows at its end (a streamed reply) that is the block being written, whatever the
/// length of the document above it.
///
/// The grammar: paragraphs, ATX headings (`#` to `######`), bullet lists (`-`, `*`, `+`) and
/// ordered lists (`1.`, `1)`) with nesting by indentation, block quotes (`>`), fenced code
/// (three or more backticks or tildes, with an optional language), and thematic breaks. A
/// fence left open runs to the end of the text, so code shows while it is still arriving.
/// Tables, setext headings, indented code, HTML blocks and reference links are read as
/// paragraph text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MarkdownDocument {
    source: String,
    blocks: Vec<MarkdownBlock>,
    /// Every line start where no container and no leaf was open: everything before the
    /// offset is parsed for good, whatever follows.
    checkpoints: Vec<Checkpoint>,
}

/// A place parsing can resume from: the source offset of a line start, the number of blocks
/// before it, and whether a blank line directly precedes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Checkpoint {
    at: usize,
    blocks: usize,
    blank: bool,
}

impl MarkdownDocument {
    /// An empty document.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Parse `source` whole.
    #[must_use]
    pub fn parse(source: &str) -> Self {
        let mut doc = Self::new();
        doc.update(source);
        doc
    }

    /// The blocks, in document order.
    #[must_use]
    pub fn blocks(&self) -> &[MarkdownBlock] {
        &self.blocks
    }

    /// The source the blocks were parsed from.
    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    /// Replace the source and bring the blocks up to date, parsing only from the last
    /// top-level block boundary before the first changed byte.
    ///
    /// Returns the index of the first block that differs from before the call: every block
    /// below it is untouched, and it equals the new block count when nothing changed.
    pub fn update(&mut self, source: &str) -> usize {
        let common = self
            .source
            .bytes()
            .zip(source.bytes())
            .take_while(|(a, b)| a == b)
            .count();
        if common == self.source.len() && common == source.len() {
            return self.blocks.len();
        }
        // The last boundary at or before the change. A boundary sits just after a newline, so
        // it is a character boundary in both strings.
        let keep = self
            .checkpoints
            .iter()
            .rposition(|c| c.at <= common)
            .map_or(0, |i| i + 1);
        self.checkpoints.truncate(keep);
        let resume = self.checkpoints.last().copied().unwrap_or(Checkpoint {
            at: 0,
            blocks: 0,
            blank: false,
        });
        let kept_blocks = resume.blocks;
        let old_tail = self.blocks.split_off(kept_blocks);
        let mut parser = BlockParser {
            blocks: std::mem::take(&mut self.blocks),
            checkpoints: std::mem::take(&mut self.checkpoints),
            blank: resume.blank,
            ..BlockParser::default()
        };
        parser.run(source, resume.at);
        self.blocks = parser.blocks;
        self.checkpoints = parser.checkpoints;
        source.clone_into(&mut self.source);
        let new_tail = &self.blocks[kept_blocks..];
        let same = old_tail
            .iter()
            .zip(new_tail)
            .take_while(|(a, b)| a == b)
            .count();
        kept_blocks + same
    }
}

/// Parse block Markdown whole: [`MarkdownDocument::parse`] for a caller that keeps nothing.
#[must_use]
pub fn parse_blocks(md: &str) -> Vec<MarkdownBlock> {
    MarkdownDocument::parse(md).blocks
}

/// A container a line can sit inside.
#[derive(Clone, Copy, Debug)]
enum Container {
    Quote,
    /// A list item: the column its content starts at, and the marker its first block has yet
    /// to draw.
    Item {
        content: usize,
        marker: ListStyle,
    },
}

/// The leaf block being collected.
#[derive(Debug)]
enum Leaf {
    Paragraph(String),
    Fence {
        /// The fence character and its run length; the closing fence is at least as long.
        mark: u8,
        len: usize,
        language: String,
        code: String,
        /// Whether any code line has been pushed, so the first is not preceded by a newline.
        started: bool,
    },
}

#[derive(Default)]
struct BlockParser {
    blocks: Vec<MarkdownBlock>,
    checkpoints: Vec<Checkpoint>,
    stack: Vec<Container>,
    leaf: Option<Leaf>,
    /// The depths the open leaf started at (a lazy continuation line can have fewer
    /// containers than its paragraph).
    leaf_depths: (u8, u8),
    leaf_marker: ListStyle,
    leaf_blank: bool,
    /// The last ordinal used at each list depth, so sibling items count up.
    ordinals: Vec<u32>,
    /// A blank line has been seen since the last block.
    blank: bool,
}

impl BlockParser {
    /// Parse `source[from..]`, appending to `blocks` and recording boundaries.
    fn run(&mut self, source: &str, from: usize) {
        let mut at = from;
        while at < source.len() {
            let rest = &source[at..];
            let (line, next) = match rest.find('\n') {
                Some(i) => (&rest[..i], at + i + 1),
                None => (rest, source.len()),
            };
            self.line(line.strip_suffix('\r').unwrap_or(line));
            // A boundary only after a complete line: text still arriving on the last line
            // could turn it into anything.
            if (next < source.len() || source.ends_with('\n'))
                && self.stack.is_empty()
                && self.leaf.is_none()
            {
                self.checkpoints.push(Checkpoint {
                    at: next,
                    blocks: self.blocks.len(),
                    blank: self.blank,
                });
            }
            at = next;
        }
        self.close_leaf();
    }

    fn depths(&self) -> (u8, u8) {
        let quotes = self
            .stack
            .iter()
            .filter(|c| matches!(c, Container::Quote))
            .count();
        let items = self.stack.len() - quotes;
        (
            u8::try_from(quotes).unwrap_or(u8::MAX),
            u8::try_from(items).unwrap_or(u8::MAX),
        )
    }

    fn line(&mut self, line: &str) {
        // 1. How many open containers does this line continue?
        let mut col = 0usize; // bytes of `line` consumed
        let mut matched = 0usize;
        let blank_line = line.trim().is_empty();
        for c in &self.stack {
            match c {
                Container::Quote => {
                    let indent = leading_spaces(&line[col..]);
                    if indent <= 3 && line[col + indent..].starts_with('>') {
                        col += indent + 1;
                        if line[col..].starts_with(' ') {
                            col += 1;
                        }
                    } else {
                        break;
                    }
                }
                Container::Item { content, .. } => {
                    if blank_line {
                        // A blank line stays inside the item; what follows decides.
                    } else if leading_spaces(&line[col.min(line.len())..]) + col >= *content {
                        col = (*content).min(line.len());
                    } else {
                        break;
                    }
                }
            }
            matched += 1;
        }
        let rest = &line[col.min(line.len())..];

        // 2. Inside a fence everything is code until the closing fence or a lost container.
        if let Some(Leaf::Fence {
            mark,
            len,
            code,
            started,
            ..
        }) = &mut self.leaf
        {
            if matched == self.stack.len() {
                let trimmed = rest.trim();
                let closes = trimmed.len() >= *len
                    && trimmed.bytes().all(|b| b == *mark)
                    && leading_spaces(rest) <= 3;
                if closes {
                    self.close_leaf();
                } else {
                    if *started {
                        code.push('\n');
                    }
                    code.push_str(rest);
                    *started = true;
                }
                return;
            }
            self.close_leaf();
        }

        // 3. A line that drops containers either continues the open paragraph (a lazy
        // continuation) or closes them.
        if matched < self.stack.len() {
            let lazy =
                matches!(self.leaf, Some(Leaf::Paragraph(_))) && !blank_line && !starts_block(rest);
            if lazy {
                self.push_text(rest.trim());
                return;
            }
            self.close_leaf();
            self.stack.truncate(matched);
        }
        if blank_line {
            self.close_leaf();
            self.blank = true;
            return;
        }

        // 4. New containers this line opens.
        let mut rest = rest;
        let mut opened_item = false;
        loop {
            let indent = leading_spaces(rest);
            if indent > 3 {
                break;
            }
            let body = &rest[indent..];
            if is_rule(body) {
                break;
            }
            if let Some(after) = body.strip_prefix('>') {
                self.close_leaf();
                self.stack.push(Container::Quote);
                rest = after.strip_prefix(' ').unwrap_or(after);
                continue;
            }
            if let Some((marker, width)) = list_marker(body) {
                self.close_leaf();
                let depth = self
                    .stack
                    .iter()
                    .filter(|c| matches!(c, Container::Item { .. }))
                    .count()
                    + 1;
                let marker = match marker {
                    ListStyle::Ordered(start) => {
                        if self.ordinals.len() >= depth {
                            self.ordinals.truncate(depth);
                            let n = self.ordinals[depth - 1].saturating_add(1);
                            self.ordinals[depth - 1] = n;
                            ListStyle::Ordered(n)
                        } else {
                            self.ordinals.resize(depth - 1, 0);
                            self.ordinals.push(start);
                            ListStyle::Ordered(start)
                        }
                    }
                    other => {
                        self.ordinals.truncate(depth - 1);
                        self.ordinals.resize(depth, 0);
                        other
                    }
                };
                rest = &body[width..];
                self.stack.push(Container::Item {
                    content: line.len() - rest.len(),
                    marker,
                });
                opened_item = true;
                continue;
            }
            break;
        }
        if !opened_item {
            // Content that is not a new item ends any deeper list's numbering.
            let (_, items) = self.depths();
            self.ordinals.truncate(usize::from(items));
        }

        // 5. The leaf.
        let body = rest.trim();
        if body.is_empty() {
            return; // a marker with nothing after it yet
        }
        if is_rule(body) {
            self.close_leaf();
            self.emit(MarkdownBlockKind::Rule, String::new());
            return;
        }
        if let Some((level, text)) = heading(body) {
            self.close_leaf();
            self.emit(MarkdownBlockKind::Heading(level), text.to_owned());
            return;
        }
        if let Some((mark, len, info)) = fence_open(body) {
            self.close_leaf();
            self.open_leaf(Leaf::Fence {
                mark,
                len,
                language: info.split_whitespace().next().unwrap_or("").to_owned(),
                code: String::new(),
                started: false,
            });
            return;
        }
        if matches!(self.leaf, Some(Leaf::Paragraph(_))) {
            self.push_text(rest.trim_start());
        } else {
            self.open_leaf(Leaf::Paragraph(String::new()));
            self.push_text(rest.trim_start());
        }
    }

    /// Append one source line to the open paragraph: a soft break is a space, and a line
    /// ending in two spaces or a backslash is a hard break.
    fn push_text(&mut self, line: &str) {
        let Some(Leaf::Paragraph(text)) = &mut self.leaf else {
            return;
        };
        if !text.is_empty() {
            if let Some(stripped) = text.strip_suffix('\\') {
                let keep = stripped.len();
                text.truncate(keep);
                text.push('\n');
            } else if text.ends_with("  ") {
                let keep = text.trim_end().len();
                text.truncate(keep);
                text.push('\n');
            } else {
                let keep = text.trim_end().len();
                text.truncate(keep);
                text.push(' ');
            }
        }
        text.push_str(line);
    }

    fn open_leaf(&mut self, leaf: Leaf) {
        self.leaf_depths = self.depths();
        self.leaf_marker = self.take_marker();
        self.leaf_blank = std::mem::take(&mut self.blank);
        self.leaf = Some(leaf);
    }

    /// The marker the innermost open item still owes its first block.
    fn take_marker(&mut self) -> ListStyle {
        for c in self.stack.iter_mut().rev() {
            if let Container::Item { marker, .. } = c {
                return std::mem::take(marker);
            }
        }
        ListStyle::None
    }

    fn close_leaf(&mut self) {
        let Some(leaf) = self.leaf.take() else { return };
        let (kind, text) = match leaf {
            Leaf::Paragraph(text) => (MarkdownBlockKind::Paragraph, text.trim_end().to_owned()),
            Leaf::Fence { language, code, .. } => (MarkdownBlockKind::Code { language }, code),
        };
        let (quote_depth, list_depth) = self.leaf_depths;
        self.blocks.push(MarkdownBlock {
            kind,
            text,
            quote_depth,
            list_depth,
            marker: self.leaf_marker,
            blank_before: self.leaf_blank,
        });
    }

    /// A block that is complete on its own line.
    fn emit(&mut self, kind: MarkdownBlockKind, text: String) {
        let (quote_depth, list_depth) = self.depths();
        let marker = self.take_marker();
        let blank_before = std::mem::take(&mut self.blank);
        self.blocks.push(MarkdownBlock {
            kind,
            text,
            quote_depth,
            list_depth,
            marker,
            blank_before,
        });
    }
}

fn leading_spaces(s: &str) -> usize {
    s.bytes().take_while(|b| *b == b' ').count()
}

/// `---`, `***` or `___`: three or more of one character, spaces allowed between.
fn is_rule(body: &str) -> bool {
    let mut chars = body.bytes().filter(|b| *b != b' ' && *b != b'\t');
    let Some(first) = chars.next() else {
        return false;
    };
    if !matches!(first, b'-' | b'*' | b'_') {
        return false;
    }
    let mut count = 1;
    for b in chars {
        if b != first {
            return false;
        }
        count += 1;
    }
    count >= 3
}

/// `# text` to `###### text`, with an optional closing run of `#`.
fn heading(body: &str) -> Option<(u8, &str)> {
    let level = body.bytes().take_while(|b| *b == b'#').count();
    if !(1..=6).contains(&level) {
        return None;
    }
    let after = &body[level..];
    if !(after.is_empty() || after.starts_with(' ')) {
        return None;
    }
    let text = after.trim();
    let text = match text.trim_end_matches('#') {
        closed if closed.len() < text.len() && (closed.is_empty() || closed.ends_with(' ')) => {
            closed.trim_end()
        }
        _ => text,
    };
    Some((u8::try_from(level).unwrap_or(6), text))
}

/// An opening fence: the character, the run length and the info string after it.
fn fence_open(body: &str) -> Option<(u8, usize, &str)> {
    let mark = *body.as_bytes().first()?;
    if !matches!(mark, b'`' | b'~') {
        return None;
    }
    let len = body.bytes().take_while(|b| *b == mark).count();
    if len < 3 {
        return None;
    }
    let info = body[len..].trim();
    // A backtick fence's info string cannot hold a backtick: that is inline code.
    if mark == b'`' && info.contains('`') {
        return None;
    }
    Some((mark, len, info))
}

/// A list marker at the start of `body`: the marker and how many bytes it and its following
/// space take. `- `, `* `, `+ `, `1. `, `1) `; a bare marker at the end of the line counts.
fn list_marker(body: &str) -> Option<(ListStyle, usize)> {
    let bytes = body.as_bytes();
    let first = *bytes.first()?;
    if matches!(first, b'-' | b'*' | b'+') {
        return match bytes.get(1) {
            None => Some((ListStyle::Bullet, 1)),
            Some(b' ') => Some((ListStyle::Bullet, 2)),
            _ => None,
        };
    }
    let digits = bytes.iter().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    if !matches!(bytes.get(digits), Some(b'.' | b')')) {
        return None;
    }
    let start = body[..digits].parse::<u32>().ok()?;
    match bytes.get(digits + 1) {
        None => Some((ListStyle::Ordered(start), digits + 1)),
        Some(b' ') => Some((ListStyle::Ordered(start), digits + 2)),
        _ => None,
    }
}

/// Does this text start a block of its own, so it cannot lazily continue a paragraph?
fn starts_block(rest: &str) -> bool {
    let indent = leading_spaces(rest);
    if indent > 3 {
        return false;
    }
    let body = &rest[indent..];
    body.starts_with('>')
        || is_rule(body)
        || heading(body).is_some()
        || fence_open(body).is_some()
        || list_marker(body).is_some()
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
        assert_eq!(r, [] as [TextRun; 0]);
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
        assert_eq!(r, [] as [TextRun; 0]);
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
        // runs, so validity is the parser's contract, checked over a spread of shapes.
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

    fn kinds(md: &str) -> Vec<(MarkdownBlockKind, String)> {
        parse_blocks(md)
            .into_iter()
            .map(|b| (b.kind, b.text))
            .collect()
    }

    #[test]
    fn paragraphs_split_on_blank_lines_and_join_soft_breaks() {
        assert_eq!(
            kinds("one\ntwo\n\nthree"),
            vec![
                (MarkdownBlockKind::Paragraph, "one two".to_owned()),
                (MarkdownBlockKind::Paragraph, "three".to_owned()),
            ]
        );
        let blocks = parse_blocks("one\n\nthree");
        assert!(!blocks[0].blank_before);
        assert!(blocks[1].blank_before);
    }

    #[test]
    fn a_hard_break_keeps_its_newline() {
        assert_eq!(kinds("one  \ntwo")[0].1, "one\ntwo");
        assert_eq!(kinds("one\\\ntwo")[0].1, "one\ntwo");
    }

    #[test]
    fn headings_rules_and_fences() {
        let got = kinds("# Title\n\n---\n\n```rust\nfn main() {}\n\n// **not bold**\n```\nafter");
        assert_eq!(
            got,
            vec![
                (MarkdownBlockKind::Heading(1), "Title".to_owned()),
                (MarkdownBlockKind::Rule, String::new()),
                (
                    MarkdownBlockKind::Code {
                        language: "rust".to_owned()
                    },
                    "fn main() {}\n\n// **not bold**".to_owned()
                ),
                (MarkdownBlockKind::Paragraph, "after".to_owned()),
            ]
        );
        assert_eq!(
            kinds("### Closed ###")[0],
            (MarkdownBlockKind::Heading(3), "Closed".to_owned())
        );
        assert_eq!(kinds("#hashtag")[0].0, MarkdownBlockKind::Paragraph);
    }

    #[test]
    fn an_open_fence_runs_to_the_end() {
        let got = kinds("```\nlet x = 1;\nlet y");
        assert_eq!(
            got,
            vec![(
                MarkdownBlockKind::Code {
                    language: String::new()
                },
                "let x = 1;\nlet y".to_owned()
            )]
        );
    }

    #[test]
    fn lists_carry_depth_and_markers() {
        let blocks = parse_blocks("- one\n- two\n  - nested\n\n  second paragraph\n- three");
        let got: Vec<_> = blocks
            .iter()
            .map(|b| (b.text.as_str(), b.list_depth, b.marker))
            .collect();
        assert_eq!(
            got,
            vec![
                ("one", 1, ListStyle::Bullet),
                ("two", 1, ListStyle::Bullet),
                ("nested", 2, ListStyle::Bullet),
                ("second paragraph", 1, ListStyle::None),
                ("three", 1, ListStyle::Bullet),
            ]
        );
    }

    #[test]
    fn ordered_lists_count_from_their_first_number() {
        let blocks = parse_blocks("3. a\n3. b\n   1. inner\n   1. inner\n3. c\n\ntext\n\n1. fresh");
        let got: Vec<_> = blocks.iter().map(|b| b.marker).collect();
        assert_eq!(
            got,
            vec![
                ListStyle::Ordered(3),
                ListStyle::Ordered(4),
                ListStyle::Ordered(1),
                ListStyle::Ordered(2),
                ListStyle::Ordered(5),
                ListStyle::None,
                ListStyle::Ordered(1),
            ]
        );
    }

    #[test]
    fn quotes_nest_and_take_lazy_lines() {
        let blocks = parse_blocks("> quoted\ncontinues\n> > deeper\n\nout");
        let got: Vec<_> = blocks
            .iter()
            .map(|b| (b.text.as_str(), b.quote_depth))
            .collect();
        assert_eq!(
            got,
            vec![("quoted continues", 1), ("deeper", 2), ("out", 0)]
        );
    }

    #[test]
    fn a_list_inside_a_quote_and_code_inside_a_list() {
        let blocks = parse_blocks("> - item\n>   ```\n>   code\n>   ```\n");
        assert_eq!(blocks.len(), 2);
        assert_eq!((blocks[0].quote_depth, blocks[0].list_depth), (1, 1));
        assert_eq!(blocks[0].marker, ListStyle::Bullet);
        assert_eq!(blocks[1].text, "code");
        assert_eq!((blocks[1].quote_depth, blocks[1].list_depth), (1, 1));
        assert_eq!(blocks[1].marker, ListStyle::None);
    }

    const SAMPLE: &str = "# Title\n\nFirst *paragraph*\nwraps here.\n\n- one\n- two\n\n  more\n  1. inner\n\n> quote\n> ```rs\n> code\n> ```\n\n```\nopen fence\n\nstill code\n```\n\n---\ntail é 🌍 text\n\n1. a\n2. b\n\nend";

    #[test]
    fn an_incremental_update_matches_a_full_parse_at_every_prefix() {
        // Streaming: the source grows one character at a time, and at every step the
        // incrementally maintained blocks must equal a parse from scratch.
        let mut doc = MarkdownDocument::new();
        let mut upto = 0;
        for (i, c) in SAMPLE.char_indices() {
            upto = i + c.len_utf8();
            let prefix = &SAMPLE[..upto];
            let before = doc.blocks().to_vec();
            let first = doc.update(prefix);
            let full = parse_blocks(prefix);
            assert_eq!(doc.blocks(), full.as_slice(), "prefix {prefix:?}");
            assert_eq!(
                &before[..first.min(before.len())],
                &full[..first.min(before.len())],
                "blocks below the returned index must be untouched at {prefix:?}"
            );
        }
        assert_eq!(upto, SAMPLE.len());
    }

    #[test]
    fn an_append_reparses_only_the_block_being_written() {
        let mut doc = MarkdownDocument::parse("# Title\n\nFirst paragraph.\n\nSecond");
        assert_eq!(doc.blocks().len(), 3);
        // The heading and the first paragraph are behind a boundary: a token appended to the
        // last paragraph reports block 2 as the first changed one.
        assert_eq!(doc.update("# Title\n\nFirst paragraph.\n\nSecond one"), 2);
        assert_eq!(doc.blocks()[2].text, "Second one");
        // A new paragraph leaves all three alone.
        assert_eq!(
            doc.update("# Title\n\nFirst paragraph.\n\nSecond one\n\nThird"),
            3
        );
        // Nothing changed: the index is the block count.
        assert_eq!(
            doc.update("# Title\n\nFirst paragraph.\n\nSecond one\n\nThird"),
            4
        );
    }

    #[test]
    fn an_edit_in_the_middle_and_a_truncation_match_a_full_parse() {
        let edits = [
            SAMPLE.replace("- two", "two"),
            SAMPLE.replace("```rs", "``"),
            SAMPLE.replace("# Title", "Title"),
            SAMPLE[..SAMPLE.len() / 2].to_owned(),
            String::new(),
            SAMPLE.replace("open fence", "open\n```\nfence"),
        ];
        for edit in edits {
            let mut doc = MarkdownDocument::parse(SAMPLE);
            doc.update(&edit);
            assert_eq!(doc.blocks(), parse_blocks(&edit).as_slice(), "{edit:?}");
            // And back again, from the edited state.
            doc.update(SAMPLE);
            assert_eq!(doc.blocks(), parse_blocks(SAMPLE).as_slice());
        }
    }

    #[test]
    fn hostile_block_input_does_not_panic() {
        for md in [
            ">>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>>> deep",
            "- - - - - - - - - - - - - - - -",
            "1. 2. 3. 4. 5. x",
            "```\n```\n```\n```",
            "\r\n\r\n- a\r\n- b\r\n",
            ">\n>\n> -\n>   -\n",
            "  \t \n\t- x",
            "é\n> é\n- é\n```é\né\n",
            "99999999999999999999. big",
        ] {
            let full = parse_blocks(md);
            let mut doc = MarkdownDocument::new();
            let mut end = 0;
            for (i, c) in md.char_indices() {
                end = i + c.len_utf8();
                doc.update(&md[..end]);
            }
            assert_eq!(end, md.len());
            assert_eq!(doc.blocks(), full.as_slice(), "{md:?}");
        }
    }
}
