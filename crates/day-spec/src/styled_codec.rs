// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Import and export for [`StyledText`]: Markdown, HTML, and RTF (docs/texteditor.md).
//!
//! All three are **lossy on purpose**, in one direction each way. Reading, anything Day's model
//! cannot hold is dropped rather than approximated — a table, an image, a stylesheet. Writing,
//! Day emits only what it can read back, so `parse(write(doc)) == doc` for every document Day
//! itself produced. That round-trip is the contract these are tested against; matching Word or
//! a browser byte for byte is not one.
//!
//! Hand-rolled, with no new dependency. Each format is a few hundred lines because the subset is
//! deliberately small (docs/texteditor.md §RTF scope), and because a text codec that panics or
//! loops on hostile input is worse than one that misses a control word.

use crate::{
    Color, Font, FontWeight, ListStyle, ParagraphAlign, ParagraphRun, ParagraphStyle, RunStyle,
    StyledText, TextRun, Underline, coalesce_runs, paragraph_bounds,
};

/// A document's base style — the font a run says nothing about, and the size everything else is
/// relative to. Import needs it to place headings and code; export needs it to know what NOT to
/// write.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DocStyle {
    pub base: Font,
    /// The point size `base` resolves to, for formats that speak in absolute sizes (RTF's
    /// half-points, HTML's `px`). Only used to convert their sizes to a [`FontSpec::scale`] and
    /// back, so an imprecise value costs relative accuracy, not correctness.
    pub base_points: f64,
}

impl Default for DocStyle {
    fn default() -> Self {
        DocStyle {
            base: Font::Body,
            base_points: 16.0,
        }
    }
}

impl DocStyle {
    pub fn new(base: Font, base_points: f64) -> Self {
        DocStyle { base, base_points }
    }
    fn plain(&self) -> RunStyle {
        RunStyle::plain(self.base)
    }
}

/// The scale a heading level takes, so Markdown and HTML agree on what `#`/`<h1>` means. Roughly
/// the browser default ramp, which is also what a reader expects a heading to look like.
fn heading_scale(level: u8) -> f64 {
    match level {
        1 => 2.0,
        2 => 1.5,
        3 => 1.17,
        4 => 1.0,
        5 => 0.83,
        _ => 0.67,
    }
}

/// Which heading level a whole-line scale came from, if any.
///
/// `1.0` is deliberately not one: h4 is body-sized in every browser's default ramp, so a run at
/// scale 1.0 is indistinguishable from ordinary bold text. An imported `#### x` therefore comes
/// back out as `**x**` — the documented loss (docs/texteditor.md), and the alternative would be
/// exporting every bold word in the document as a heading.
fn heading_level_for(scale: f64) -> Option<u8> {
    if (scale - 1.0).abs() < 1e-6 {
        return None;
    }
    (1u8..=6).find(|level| (heading_scale(*level) - scale).abs() < 1e-6)
}

// ===========================================================================
// Markdown
// ===========================================================================

/// Parse markdown into a styled document: the inline grammar
/// ([`crate::markdown`](crate::markdown)) plus the block constructs a document editor needs —
/// ATX headings, `>` quotes, and `-`/`1.` lists.
///
/// Headings become a **run** (bold at [`heading_scale`]) rather than a paragraph attribute,
/// because a heading is a size and a weight, and Day's paragraph attributes are alignment,
/// indent and list decoration. Quotes and list items become paragraph runs, which is what they
/// are.
pub fn markdown_to_styled(md: &str, style: DocStyle) -> StyledText {
    let mut doc = StyledText::default();
    let mut paragraphs: Vec<ParagraphRun> = Vec::new();
    let mut ordinal = 1u32;
    let mut prev_ordered = false;

    for raw in md.lines() {
        let start = doc.text.len();
        let (body, para, heading) = split_block_markers(raw);

        // An ordered list numbers itself across consecutive items; anything else resets it.
        let para = match para.list {
            ListStyle::Ordered(_) => {
                if !prev_ordered {
                    ordinal = 1;
                }
                prev_ordered = true;
                let p = ParagraphStyle {
                    list: ListStyle::Ordered(ordinal),
                    ..para
                };
                ordinal += 1;
                p
            }
            _ => {
                prev_ordered = false;
                para
            }
        };

        let (text, mut runs) = crate::markdown::parse(body, style.base);
        // A heading scales and bolds the WHOLE line, under whatever inline styling it carries.
        if let Some(level) = heading {
            let scale = heading_scale(level);
            for r in &mut runs {
                r.font.scale = scale;
                if r.font.weight.is_none() {
                    r.font.weight = Some(FontWeight::Bold);
                }
            }
            let mut head = RunStyle::plain(style.base);
            head.font.scale = scale;
            head.set_bold(true);
            merge_base_run(&mut runs, text.len(), head);
        }
        for r in &mut runs {
            r.range = (r.range.start + start)..(r.range.end + start);
        }
        doc.runs.extend(runs);
        doc.text.push_str(&text);
        doc.text.push('\n');
        let end = doc.text.len();
        if para != ParagraphStyle::default() {
            paragraphs.push(ParagraphRun::new(start..end, para));
        }
    }
    // `lines()` drops the final terminator; a document that did not end in one should not gain it.
    if !md.ends_with('\n') && doc.text.ends_with('\n') {
        doc.text.pop();
        doc.clamp();
    }
    doc.paragraphs = paragraphs;
    doc.runs = coalesce_runs(std::mem::take(&mut doc.runs), style.base);
    doc.clamp();
    doc
}

/// Strip a line's block markers, returning the remaining inline text, the paragraph style they
/// imply, and the heading level if it is one.
fn split_block_markers(line: &str) -> (&str, ParagraphStyle, Option<u8>) {
    let mut s = line;
    let mut para = ParagraphStyle::default();
    // Blockquotes nest, and each level indents.
    while let Some(rest) = s.strip_prefix('>') {
        para.indent += 24.0;
        s = rest.strip_prefix(' ').unwrap_or(rest);
    }
    // One level of list nesting per two leading spaces, which is the CommonMark-ish rule and the
    // one a person typing into an editor expects.
    let indented = s.len() - s.trim_start_matches(' ').len();
    let trimmed = s.trim_start_matches(' ');
    if let Some(rest) = trimmed
        .strip_prefix("- ")
        .or_else(|| trimmed.strip_prefix("* "))
    {
        para.list = ListStyle::Bullet;
        para.list_level = (indented / 2).min(u8::MAX as usize) as u8;
        return (rest, para, None);
    }
    if let Some((num, rest)) = split_ordered_marker(trimmed) {
        para.list = ListStyle::Ordered(num);
        para.list_level = (indented / 2).min(u8::MAX as usize) as u8;
        return (rest, para, None);
    }
    // ATX heading, up to six `#`.
    let hashes = s.bytes().take_while(|b| *b == b'#').count();
    if (1..=6).contains(&hashes)
        && let Some(rest) = s[hashes..].strip_prefix(' ')
    {
        return (rest, para, Some(hashes as u8));
    }
    (s, para, None)
}

/// `"12. text"` → `(12, "text")`.
fn split_ordered_marker(s: &str) -> Option<(u32, &str)> {
    let digits = s.bytes().take_while(|b| b.is_ascii_digit()).count();
    if digits == 0 || digits > 9 {
        return None;
    }
    let rest = s[digits..].strip_prefix(". ")?;
    s[..digits].parse().ok().map(|n| (n, rest))
}

/// Cover the whole line with `style`, folding it under any inline runs already there.
fn merge_base_run(runs: &mut Vec<TextRun>, len: usize, style: RunStyle) {
    let mut out = Vec::with_capacity(runs.len() * 2 + 1);
    let mut at = 0usize;
    for r in runs.iter() {
        if r.range.start > at {
            out.push(TextRun::styled(at..r.range.start, style.clone()));
        }
        out.push(r.clone());
        at = r.range.end;
    }
    if at < len {
        out.push(TextRun::styled(at..len, style));
    }
    *runs = out;
}

/// Write a styled document as markdown.
///
/// Lossy where markdown has no spelling: color, highlight, underline, alignment and relative
/// sizes that are not one of the six heading steps all vanish. What survives round-trips.
pub fn styled_to_markdown(doc: &StyledText, style: DocStyle) -> String {
    let mut out = String::with_capacity(doc.text.len() + 32);
    for (start, end) in paragraph_bounds(&doc.text) {
        let line_end = end - usize::from(doc.text[start..end].ends_with('\n'));
        let para = doc
            .paragraphs
            .iter()
            .find(|p| p.range.start <= start && p.range.end >= line_end.max(start + 1))
            .map(|p| p.style())
            .unwrap_or_default();
        // A whole-line scale that matches a heading step becomes `#`s.
        let heading = doc
            .style_of(start..line_end.max(start), style.base)
            .font
            .scale;
        let heading = if line_end > start {
            heading_level_for(heading)
        } else {
            None
        };
        for _ in 0..(para.indent / 24.0).round().max(0.0) as usize {
            out.push_str("> ");
        }
        for _ in 0..para.list_level {
            out.push_str("  ");
        }
        match para.list {
            ListStyle::Bullet => out.push_str("- "),
            ListStyle::Ordered(n) => out.push_str(&format!("{n}. ")),
            ListStyle::None => {}
        }
        if let Some(level) = heading {
            for _ in 0..level {
                out.push('#');
            }
            out.push(' ');
        }
        write_markdown_inline(doc, start..line_end, style, heading.is_some(), &mut out);
        if end < doc.text.len() || doc.text[start..end].ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

fn write_markdown_inline(
    doc: &StyledText,
    range: std::ops::Range<usize>,
    style: DocStyle,
    in_heading: bool,
    out: &mut String,
) {
    for (seg, s) in doc.segments(style.base) {
        let lo = seg.start.max(range.start);
        let hi = seg.end.min(range.end);
        if lo >= hi {
            continue;
        }
        let Some(text) = doc.text.get(lo..hi) else {
            continue;
        };
        // A heading's own bold is the `#`, not `**`.
        let bold = s.bold() && !in_heading;
        let (open, close) = markdown_markers(&s, bold);
        if let Some(url) = &s.link {
            out.push('[');
            out.push_str(&open);
            out.push_str(text);
            out.push_str(&close);
            out.push_str("](");
            out.push_str(url);
            out.push(')');
            continue;
        }
        out.push_str(&open);
        out.push_str(&escape_markdown(text));
        out.push_str(&close);
    }
}

fn markdown_markers(s: &RunStyle, bold: bool) -> (String, String) {
    let mut open = String::new();
    let mut close = String::new();
    if bold {
        open.push_str("**");
        close.insert_str(0, "**");
    }
    if s.font.italic {
        open.push('*');
        close.insert(0, '*');
    }
    if s.strikethrough {
        open.push_str("~~");
        close.insert_str(0, "~~");
    }
    if s.font.monospace {
        open.push('`');
        close.insert(0, '`');
    }
    (open, close)
}

fn escape_markdown(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(c, '*' | '_' | '`' | '~' | '[' | ']' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

// ===========================================================================
// HTML
// ===========================================================================

/// Write a styled document as an HTML fragment: one `<p>` per paragraph, `<span style="…">` per
/// run, with `<b>`/`<i>`/`<u>`/`<s>`/`<code>`/`<a>` where a tag says it more plainly.
pub fn styled_to_html(doc: &StyledText, style: DocStyle) -> String {
    let mut out = String::with_capacity(doc.text.len() * 2);
    for (start, end) in paragraph_bounds(&doc.text) {
        let line_end = end - usize::from(doc.text[start..end].ends_with('\n'));
        let para = doc
            .paragraphs
            .iter()
            .find(|p| p.range.start <= start && p.range.end >= line_end.max(start + 1))
            .map(|p| p.style())
            .unwrap_or_default();
        let (open_tag, close_tag) = match para.list {
            ListStyle::None => ("p", "p"),
            _ => ("li", "li"),
        };
        out.push('<');
        out.push_str(open_tag);
        let mut css = String::new();
        match para.align {
            ParagraphAlign::Natural => {}
            ParagraphAlign::Center => css.push_str("text-align:center;"),
            ParagraphAlign::Trailing => css.push_str("text-align:end;"),
            ParagraphAlign::Justified => css.push_str("text-align:justify;"),
        }
        let indent = para.indent + f64::from(para.list_level) * 24.0;
        if indent > 0.0 {
            css.push_str(&format!("margin-inline-start:{indent}px;"));
        }
        if para.space_before > 0.0 {
            css.push_str(&format!("margin-block-start:{}px;", para.space_before));
        }
        if para.space_after > 0.0 {
            css.push_str(&format!("margin-block-end:{}px;", para.space_after));
        }
        if !css.is_empty() {
            out.push_str(&format!(" style=\"{css}\""));
        }
        out.push('>');
        for (seg, s) in doc.segments(style.base) {
            let lo = seg.start.max(start);
            let hi = seg.end.min(line_end);
            if lo >= hi {
                continue;
            }
            let Some(text) = doc.text.get(lo..hi) else {
                continue;
            };
            write_html_run(&s, text, &mut out);
        }
        out.push_str("</");
        out.push_str(close_tag);
        out.push_str(">\n");
    }
    out
}

fn write_html_run(s: &RunStyle, text: &str, out: &mut String) {
    let mut css = String::new();
    if let Some(c) = s.color {
        css.push_str(&format!("color:{};", css_hex(c)));
    }
    if let Some(c) = s.background {
        css.push_str(&format!("background-color:{};", css_hex(c)));
    }
    if s.font.scale != 1.0 {
        css.push_str(&format!("font-size:{}em;", s.font.scale));
    }
    if let Font::System(pt) | Font::Custom(_, pt) = s.font.style {
        css.push_str(&format!("font-size:{pt}px;"));
    }
    let mut tags: Vec<&str> = Vec::new();
    if let Some(url) = &s.link {
        out.push_str("<a href=\"");
        escape_html(url, out);
        out.push_str("\">");
    }
    if !css.is_empty() {
        out.push_str(&format!("<span style=\"{css}\">"));
    }
    if s.bold() {
        tags.push("b");
    }
    if s.font.italic {
        tags.push("i");
    }
    if s.underline.is_on() {
        tags.push("u");
    }
    if s.strikethrough {
        tags.push("s");
    }
    if s.font.monospace {
        tags.push("code");
    }
    for t in &tags {
        out.push('<');
        out.push_str(t);
        out.push('>');
    }
    escape_html(text, out);
    for t in tags.iter().rev() {
        out.push_str("</");
        out.push_str(t);
        out.push('>');
    }
    if !css.is_empty() {
        out.push_str("</span>");
    }
    if s.link.is_some() {
        out.push_str("</a>");
    }
}

fn css_hex(c: Color) -> String {
    let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(c.r), q(c.g), q(c.b))
}

fn escape_html(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
}

/// Parse an HTML fragment into a styled document.
///
/// A tag-soup reader, not a browser: it tracks the inline tags and inline `style` properties Day
/// can express and ignores everything else, including `<script>`/`<style>` contents, which it
/// skips wholesale. Unknown tags are transparent — their TEXT survives, their meaning does not.
pub fn html_to_styled(html: &str, style: DocStyle) -> StyledText {
    let mut p = HtmlParser {
        src: html.as_bytes(),
        at: 0,
        doc: StyledText::default(),
        stack: vec![style.plain()],
        para_stack: vec![ParagraphStyle::default()],
        style,
        para_start: 0,
        list_ordinal: Vec::new(),
    };
    p.run();
    p.finish()
}

struct HtmlParser<'a> {
    src: &'a [u8],
    at: usize,
    doc: StyledText,
    stack: Vec<RunStyle>,
    para_stack: Vec<ParagraphStyle>,
    style: DocStyle,
    para_start: usize,
    list_ordinal: Vec<u32>,
}

impl HtmlParser<'_> {
    fn run(&mut self) {
        let mut text_start = 0usize;
        while self.at < self.src.len() {
            if self.src[self.at] == b'<' {
                self.flush(text_start);
                self.tag();
                text_start = self.doc.text.len();
            } else {
                let c = self.next_char();
                self.push_char(c);
            }
        }
        self.flush(text_start);
    }

    fn next_char(&mut self) -> char {
        // Entities first, then a whole UTF-8 scalar so a multi-byte character is never split.
        if self.src[self.at] == b'&'
            && let Some(end) = self.src[self.at..].iter().take(12).position(|b| *b == b';')
        {
            let ent = std::str::from_utf8(&self.src[self.at + 1..self.at + end]).unwrap_or("");
            let decoded = match ent {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" | "#39" => Some('\''),
                "nbsp" => Some('\u{a0}'),
                e => e
                    .strip_prefix('#')
                    .and_then(|n| {
                        n.strip_prefix('x')
                            .and_then(|h| u32::from_str_radix(h, 16).ok())
                            .or_else(|| n.parse().ok())
                    })
                    .and_then(char::from_u32),
            };
            if let Some(c) = decoded {
                self.at += end + 1;
                return c;
            }
        }
        let rest = &self.src[self.at..];
        let s = std::str::from_utf8(rest).unwrap_or("\u{fffd}");
        let c = s.chars().next().unwrap_or('\u{fffd}');
        self.at += c.len_utf8().min(rest.len());
        c
    }

    fn push_char(&mut self, c: char) {
        // Collapse runs of whitespace, the way HTML does; a `<br>` or block boundary is what
        // makes a line break.
        if c.is_whitespace() {
            if self.doc.text.ends_with(' ') || self.doc.text.ends_with('\n') {
                return;
            }
            self.doc.text.push(' ');
            return;
        }
        self.doc.text.push(c);
    }

    /// Emit everything since `from` as a run under the current style.
    fn flush(&mut self, from: usize) {
        let end = self.doc.text.len();
        let style = self
            .stack
            .last()
            .cloned()
            .unwrap_or_else(|| self.style.plain());
        if end > from && style != self.style.plain() {
            self.doc.runs.push(TextRun::styled(from..end, style));
        }
    }

    fn tag(&mut self) {
        let Some(close) = self.src[self.at..].iter().position(|b| *b == b'>') else {
            self.at = self.src.len();
            return;
        };
        let raw = std::str::from_utf8(&self.src[self.at + 1..self.at + close]).unwrap_or("");
        self.at += close + 1;
        let closing = raw.starts_with('/');
        let body = raw.trim_start_matches('/').trim_end_matches('/');
        let name: String = body
            .split([' ', '\t', '\n'])
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        // `<script>`/`<style>` contents are not text.
        if !closing && matches!(name.as_str(), "script" | "style") {
            let needle = format!("</{name}");
            if let Some(end) = find_ci(&self.src[self.at..], needle.as_bytes()) {
                self.at += end;
            } else {
                self.at = self.src.len();
            }
            return;
        }
        if name == "br" {
            self.end_paragraph();
            return;
        }
        if closing {
            self.close(&name);
        } else {
            self.open(&name, body);
        }
    }

    fn open(&mut self, name: &str, body: &str) {
        if is_block(name) {
            self.end_paragraph();
            let mut para = ParagraphStyle::default();
            if let Some(level) = heading_tag_level(name) {
                let mut s = self.style.plain();
                s.font.scale = heading_scale(level);
                s.set_bold(true);
                self.stack.push(s);
                self.para_stack.push(para);
                return;
            }
            if name == "li" {
                let ordered = self.list_ordinal.last().copied();
                para.list = match ordered {
                    Some(n) => ListStyle::Ordered(n),
                    None => ListStyle::Bullet,
                };
                para.list_level = self.list_ordinal.len().saturating_sub(1).min(255) as u8;
                if let Some(n) = self.list_ordinal.last_mut() {
                    *n += 1;
                }
            }
            if name == "ul" {
                self.list_ordinal.push(0);
                // A bullet list is marked by a ZERO ordinal slot, which `li` reads as "no number".
                if let Some(last) = self.list_ordinal.last_mut() {
                    *last = 0;
                }
                self.para_stack.push(para);
                self.stack.push(self.current_run());
                return;
            }
            if name == "ol" {
                self.list_ordinal.push(1);
                self.para_stack.push(para);
                self.stack.push(self.current_run());
                return;
            }
            if name == "blockquote" {
                para.indent += 24.0;
            }
            apply_block_css(&mut para, body);
            self.para_stack.push(para);
            self.stack.push(self.current_run());
            return;
        }
        let mut s = self.current_run();
        match name {
            "b" | "strong" => s.set_bold(true),
            "i" | "em" => s.set_italic(true),
            "u" | "ins" => s.underline = Underline::Single,
            "s" | "del" | "strike" => s.strikethrough = true,
            "code" | "kbd" | "samp" | "tt" => s.font.monospace = true,
            "a" => s.link = attr(body, "href").map(str::to_string),
            _ => {}
        }
        apply_inline_css(&mut s, body, self.style);
        self.stack.push(s);
    }

    fn close(&mut self, name: &str) {
        if is_block(name) {
            self.end_paragraph();
            if self.para_stack.len() > 1 {
                self.para_stack.pop();
            }
            if matches!(name, "ul" | "ol") {
                self.list_ordinal.pop();
            }
        }
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    fn current_run(&self) -> RunStyle {
        self.stack
            .last()
            .cloned()
            .unwrap_or_else(|| self.style.plain())
    }

    /// Close the paragraph in progress, recording its style and starting the next.
    fn end_paragraph(&mut self) {
        // Trim the collapsed trailing space a block boundary leaves behind.
        while self.doc.text.ends_with(' ') {
            self.doc.text.pop();
        }
        if self.doc.text.len() <= self.para_start {
            return;
        }
        self.doc.text.push('\n');
        let style = self.para_stack.last().copied().unwrap_or_default();
        if style != ParagraphStyle::default() {
            self.doc.paragraphs.push(ParagraphRun::new(
                self.para_start..self.doc.text.len(),
                style,
            ));
        }
        self.para_start = self.doc.text.len();
    }

    fn finish(mut self) -> StyledText {
        self.end_paragraph();
        // The last paragraph's terminator is an artifact of closing it, not part of the text.
        if self.doc.text.ends_with('\n') {
            self.doc.text.pop();
        }
        let base = self.style.base;
        self.doc.runs = coalesce_runs(std::mem::take(&mut self.doc.runs), base);
        self.doc.clamp();
        self.doc
    }
}

fn is_block(name: &str) -> bool {
    matches!(
        name,
        "p" | "div"
            | "li"
            | "ul"
            | "ol"
            | "blockquote"
            | "pre"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
    )
}

fn heading_tag_level(name: &str) -> Option<u8> {
    let n = name.strip_prefix('h')?;
    let level: u8 = n.parse().ok()?;
    (1..=6).contains(&level).then_some(level)
}

/// The value of `name="…"` (or `name='…'`) in a tag body.
fn attr<'a>(body: &'a str, name: &str) -> Option<&'a str> {
    let lower = body.to_ascii_lowercase();
    let at = lower.find(&format!("{name}="))? + name.len() + 1;
    let rest = &body[at..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        rest[1..].find(quote).map(|e| &rest[1..1 + e])
    } else {
        Some(rest.split([' ', '\t']).next().unwrap_or(rest))
    }
}

fn apply_inline_css(s: &mut RunStyle, body: &str, doc: DocStyle) {
    let Some(css) = attr(body, "style") else {
        return;
    };
    for decl in css.split(';') {
        let Some((k, v)) = decl.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "color" => s.color = parse_css_color(v),
            "background-color" | "background" => s.background = parse_css_color(v),
            "font-weight" => {
                let bold = v == "bold" || v.parse::<u32>().is_ok_and(|n| n >= 600);
                s.set_bold(bold);
            }
            "font-style" => s.set_italic(v == "italic" || v == "oblique"),
            "text-decoration" | "text-decoration-line" => {
                if v.contains("underline") {
                    s.underline = Underline::Single;
                }
                if v.contains("line-through") {
                    s.strikethrough = true;
                }
            }
            "font-family" => s.font.monospace = v.contains("monospace") || v.contains("Courier"),
            "font-size" => {
                if let Some(em) = v
                    .strip_suffix("em")
                    .and_then(|n| n.trim().parse::<f64>().ok())
                {
                    s.font.scale = em;
                } else if let Some(px) = v
                    .strip_suffix("px")
                    .and_then(|n| n.trim().parse::<f64>().ok())
                {
                    // An absolute size becomes an absolute style, which is exactly what
                    // `Font::System` is for (docs/texteditor.md).
                    s.font.style = Font::System(px);
                    s.font.scale = 1.0;
                } else if let Some(pct) = v
                    .strip_suffix('%')
                    .and_then(|n| n.trim().parse::<f64>().ok())
                {
                    s.font.scale = pct / 100.0;
                } else if let Some(pt) = v
                    .strip_suffix("pt")
                    .and_then(|n| n.trim().parse::<f64>().ok())
                {
                    s.font.style = Font::System(pt * 96.0 / 72.0);
                    s.font.scale = 1.0;
                }
                let _ = doc;
            }
            _ => {}
        }
    }
}

fn apply_block_css(para: &mut ParagraphStyle, body: &str) {
    let Some(css) = attr(body, "style") else {
        return;
    };
    for decl in css.split(';') {
        let Some((k, v)) = decl.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        let px = |v: &str| v.trim_end_matches("px").trim().parse::<f64>().ok();
        match k.as_str() {
            "text-align" => {
                para.align = match v {
                    "center" => ParagraphAlign::Center,
                    "right" | "end" => ParagraphAlign::Trailing,
                    "justify" => ParagraphAlign::Justified,
                    _ => ParagraphAlign::Natural,
                }
            }
            "margin-inline-start" | "margin-left" | "padding-inline-start" => {
                para.indent = px(v).unwrap_or(para.indent)
            }
            "margin-block-start" | "margin-top" => {
                para.space_before = px(v).unwrap_or(para.space_before)
            }
            "margin-block-end" | "margin-bottom" => {
                para.space_after = px(v).unwrap_or(para.space_after)
            }
            _ => {}
        }
    }
}

/// `#rgb`, `#rrggbb`, or `rgb(r, g, b)`.
fn parse_css_color(v: &str) -> Option<Color> {
    let v = v.trim();
    if let Some(rest) = v.strip_prefix("rgb(").and_then(|r| r.strip_suffix(')')) {
        let mut it = rest.split(',').map(|n| n.trim().parse::<f64>().ok());
        return Some(Color::rgb(
            it.next()?? / 255.0,
            it.next()?? / 255.0,
            it.next()?? / 255.0,
        ));
    }
    Color::parse(v)
}

/// Case-insensitive substring search, for finding a closing tag.
fn find_ci(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len())
        .position(|w| w.eq_ignore_ascii_case(needle))
}

// ===========================================================================
// RTF
// ===========================================================================

/// Write a styled document as RTF, in the subset [`rtf_to_styled`] reads back
/// (docs/texteditor.md).
pub fn styled_to_rtf(doc: &StyledText, style: DocStyle) -> String {
    // The color table is 1-based in `\cfN`; index 0 is "the default", so the table starts with an
    // empty entry and every real color is written after it.
    let mut colors: Vec<Color> = Vec::new();
    let color_index = |c: Color, colors: &mut Vec<Color>| -> usize {
        match colors.iter().position(|x| *x == c) {
            Some(i) => i + 1,
            None => {
                colors.push(c);
                colors.len()
            }
        }
    };
    let mut body = String::new();
    for (start, end) in paragraph_bounds(&doc.text) {
        let line_end = end - usize::from(doc.text[start..end].ends_with('\n'));
        let para = doc
            .paragraphs
            .iter()
            .find(|p| p.range.start <= start && p.range.end >= line_end.max(start + 1))
            .map(|p| p.style())
            .unwrap_or_default();
        body.push_str("\\pard");
        match para.align {
            ParagraphAlign::Natural => body.push_str("\\ql"),
            ParagraphAlign::Center => body.push_str("\\qc"),
            ParagraphAlign::Trailing => body.push_str("\\qr"),
            ParagraphAlign::Justified => body.push_str("\\qj"),
        }
        let indent = para.indent + f64::from(para.list_level) * 24.0;
        if indent > 0.0 {
            // RTF measures in twips: 20 per point.
            body.push_str(&format!("\\li{}", (indent * 20.0).round() as i64));
        }
        if para.space_before > 0.0 {
            body.push_str(&format!(
                "\\sb{}",
                (para.space_before * 20.0).round() as i64
            ));
        }
        if para.space_after > 0.0 {
            body.push_str(&format!("\\sa{}", (para.space_after * 20.0).round() as i64));
        }
        match para.list {
            ListStyle::Bullet => body.push_str("\\bullet\\tab "),
            ListStyle::Ordered(n) => body.push_str(&format!(" {n}.\\tab ")),
            // The paragraph words above always end in one, so this space is their delimiter and
            // is consumed by the reader rather than becoming text.
            ListStyle::None => body.push(' '),
        }
        for (seg, s) in doc.segments(style.base) {
            let lo = seg.start.max(start);
            let hi = seg.end.min(line_end);
            if lo >= hi {
                continue;
            }
            let Some(text) = doc.text.get(lo..hi) else {
                continue;
            };
            body.push('{');
            // Every control word below needs a delimiter, and in RTF a single space after one IS
            // that delimiter. But a run with NO control words must not get one, or the space
            // becomes literal text — which is how "bold italic" came back as "bold  italic".
            let attrs_at = body.len();
            if s.bold() {
                body.push_str("\\b");
            }
            if s.font.italic {
                body.push_str("\\i");
            }
            match s.underline {
                Underline::None => {}
                Underline::Double => body.push_str("\\uldb"),
                Underline::Dotted => body.push_str("\\uld"),
                Underline::Wavy => body.push_str("\\ulwave"),
                Underline::Single => body.push_str("\\ul"),
            }
            if s.strikethrough {
                body.push_str("\\strike");
            }
            // Half-points, which is RTF's unit.
            let pts = s.font.resolved_points(style.base_points);
            if (pts - style.base_points).abs() > 0.01 {
                body.push_str(&format!("\\fs{}", (pts * 2.0).round() as i64));
            }
            if let Some(c) = s.color {
                body.push_str(&format!("\\cf{}", color_index(c, &mut colors)));
            }
            if let Some(c) = s.background {
                body.push_str(&format!("\\highlight{}", color_index(c, &mut colors)));
            }
            if s.font.monospace {
                body.push_str("\\f1");
            }
            if body.len() > attrs_at {
                body.push(' ');
            }
            escape_rtf(text, &mut body);
            body.push('}');
        }
        body.push_str("\\par\n");
    }

    let mut out = String::with_capacity(body.len() + 256);
    out.push_str("{\\rtf1\\ansi\\deff0");
    out.push_str("{\\fonttbl{\\f0 ");
    out.push_str(rtf_family(style.base));
    out.push_str(";}{\\f1 Courier New;}}");
    out.push_str("{\\colortbl;");
    for c in &colors {
        let q = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        out.push_str(&format!(
            "\\red{}\\green{}\\blue{};",
            q(c.r),
            q(c.g),
            q(c.b)
        ));
    }
    out.push('}');
    out.push_str(&format!(
        "\\fs{}\n",
        (style.base_points * 2.0).round() as i64
    ));
    out.push_str(&body);
    out.push('}');
    out
}

fn rtf_family(base: Font) -> &'static str {
    match base {
        Font::Custom(name, _) => {
            // A `&'static str` family name can go straight in; RTF has no escaping inside a
            // font-table entry beyond the ones `escape_rtf` handles, and a family with a brace
            // in it is not a thing.
            name
        }
        _ => "Helvetica",
    }
}

fn escape_rtf(s: &str, out: &mut String) {
    for c in s.chars() {
        match c {
            '\\' | '{' | '}' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\line "),
            c if (c as u32) < 128 => out.push(c),
            // Non-ASCII rides `\uN?`: a signed 16-bit code unit plus a replacement character for
            // readers that do not understand it. Astral characters take two units, which is why
            // this encodes UTF-16 rather than the scalar.
            c => {
                let mut buf = [0u16; 2];
                for unit in c.encode_utf16(&mut buf) {
                    out.push_str(&format!("\\u{}?", *unit as i16));
                }
            }
        }
    }
}

/// Parse the RTF subset Day writes, plus the control words a real editor puts around it
/// (docs/texteditor.md).
///
/// Reads: `\b \i \ul \uldb \uld \ulwave \strike \fs \cf \highlight \f`, the `\fonttbl` and
/// `\colortbl` tables, `\par \line \tab`, `\pard \ql \qc \qr \qj \li \sb \sa`, `\'hh` and `\uN`
/// escapes, and group nesting for style scoping. Skips `{\*\…}` destinations whole. Everything
/// else is ignored, which is what keeps a Word document from producing garbage rather than
/// nothing.
pub fn rtf_to_styled(rtf: &str, style: DocStyle) -> StyledText {
    let mut p = RtfParser {
        src: rtf.as_bytes(),
        at: 0,
        doc: StyledText::default(),
        stack: vec![RtfState::new(style)],
        colors: Vec::new(),
        mono_fonts: Vec::new(),
        style,
        run_start: 0,
        para_start: 0,
        para: ParagraphStyle::default(),
        pending_utf16: Vec::new(),
    };
    p.run();
    p.finish()
}

#[derive(Clone)]
struct RtfState {
    run: RunStyle,
    /// Skip this group's text entirely (a `\*` destination, a font or color table).
    skip: bool,
}

impl RtfState {
    fn new(style: DocStyle) -> Self {
        RtfState {
            run: style.plain(),
            skip: false,
        }
    }
}

struct RtfParser<'a> {
    src: &'a [u8],
    at: usize,
    doc: StyledText,
    stack: Vec<RtfState>,
    colors: Vec<Color>,
    mono_fonts: Vec<u32>,
    style: DocStyle,
    run_start: usize,
    para_start: usize,
    para: ParagraphStyle,
    pending_utf16: Vec<u16>,
}

impl RtfParser<'_> {
    fn run(&mut self) {
        while self.at < self.src.len() {
            match self.src[self.at] {
                b'{' => {
                    self.at += 1;
                    let top = self
                        .stack
                        .last()
                        .cloned()
                        .unwrap_or(RtfState::new(self.style));
                    self.stack.push(top);
                }
                b'}' => {
                    self.at += 1;
                    self.flush_run();
                    if self.stack.len() > 1 {
                        self.stack.pop();
                    }
                }
                b'\\' => self.control(),
                b'\r' | b'\n' => self.at += 1, // literal newlines are formatting, not text
                _ => {
                    let b = self.src[self.at];
                    self.at += 1;
                    self.push_byte(b);
                }
            }
        }
    }

    fn state(&self) -> RtfState {
        self.stack
            .last()
            .cloned()
            .unwrap_or_else(|| RtfState::new(self.style))
    }

    fn skipping(&self) -> bool {
        self.stack.last().is_some_and(|s| s.skip)
    }

    fn push_byte(&mut self, b: u8) {
        self.flush_utf16();
        if self.skipping() {
            return;
        }
        self.doc.text.push(b as char);
    }

    fn push_char(&mut self, c: char) {
        if self.skipping() {
            return;
        }
        self.doc.text.push(c);
    }

    /// A `\uN` escape may be half a surrogate pair; hold units until they decode.
    fn push_utf16(&mut self, unit: u16) {
        if self.skipping() {
            return;
        }
        self.pending_utf16.push(unit);
        let decoded: String = String::from_utf16_lossy(&self.pending_utf16);
        // A lone high surrogate decodes lossily, so hold it for its partner — but only for ONE
        // more unit, or an unpaired surrogate would swallow the rest of the document.
        if !decoded.contains('\u{fffd}') || self.pending_utf16.len() >= 2 {
            self.doc.text.push_str(&decoded);
            self.pending_utf16.clear();
        }
    }

    fn flush_utf16(&mut self) {
        if !self.pending_utf16.is_empty() {
            let s = String::from_utf16_lossy(&self.pending_utf16);
            self.pending_utf16.clear();
            if !self.skipping() {
                self.doc.text.push_str(&s);
            }
        }
    }

    /// Emit everything typed since the last style change as a run.
    fn flush_run(&mut self) {
        self.flush_utf16();
        let end = self.doc.text.len();
        let run = self.state().run;
        if end > self.run_start && run != self.style.plain() {
            self.doc
                .runs
                .push(TextRun::styled(self.run_start..end, run));
        }
        self.run_start = end;
    }

    /// Consume the rest of the current group without producing text — for the tables, whose
    /// contents this reader parses from the source directly.
    fn skip_group(&mut self) {
        let mut depth = 1i32;
        while self.at < self.src.len() && depth > 0 {
            match self.src[self.at] {
                b'{' => depth += 1,
                b'}' => depth -= 1,
                // An escaped brace is text, not nesting.
                b'\\' => self.at += 1,
                _ => {}
            }
            self.at += 1;
        }
        if self.stack.len() > 1 {
            self.stack.pop();
        }
        self.run_start = self.doc.text.len();
    }

    fn control(&mut self) {
        self.at += 1;
        if self.at >= self.src.len() {
            return;
        }
        let b = self.src[self.at];
        // `\'hh` — a raw byte in the document's codepage, read as Latin-1, which is what an
        // `\ansi` document means and the only codepage this subset promises.
        if b == b'\'' {
            let hex = self
                .src
                .get(self.at + 1..self.at + 3)
                .and_then(|h| std::str::from_utf8(h).ok())
                .and_then(|h| u8::from_str_radix(h, 16).ok());
            self.at += 3.min(self.src.len() - self.at);
            if let Some(v) = hex {
                self.flush_utf16();
                self.push_char(v as char);
            }
            return;
        }
        // `\*` is RTF's own "skip this destination if you do not know it", and knowing it is
        // exactly what a subset reader does not. Marking the GROUP rather than guessing at the
        // control word after it is what keeps `{\*\expandedcolortbl;;}` and every future
        // destination from leaking punctuation into the text.
        if b == b'*' {
            self.at += 1;
            if let Some(st) = self.stack.last_mut() {
                st.skip = true;
            }
            return;
        }
        if !b.is_ascii_alphabetic() {
            // An escaped literal: `\\`, `\{`, `\}`, and the ones nothing else claims.
            self.at += 1;
            self.flush_utf16();
            self.push_char(b as char);
            return;
        }
        let word_start = self.at;
        while self.at < self.src.len() && self.src[self.at].is_ascii_alphabetic() {
            self.at += 1;
        }
        let word = std::str::from_utf8(&self.src[word_start..self.at])
            .unwrap_or("")
            .to_string();
        // An optional signed parameter, then one optional space that belongs to the control word.
        let num_start = self.at;
        if self.at < self.src.len() && (self.src[self.at] == b'-') {
            self.at += 1;
        }
        while self.at < self.src.len() && self.src[self.at].is_ascii_digit() {
            self.at += 1;
        }
        let param: Option<i64> = std::str::from_utf8(&self.src[num_start..self.at])
            .ok()
            .and_then(|s| s.parse().ok());
        if self.at < self.src.len() && self.src[self.at] == b' ' {
            self.at += 1;
        }
        self.word(&word, param);
    }

    fn word(&mut self, word: &str, param: Option<i64>) {
        // A style change ends the run it was in.
        let styling = matches!(
            word,
            "b" | "i"
                | "ul"
                | "ulnone"
                | "uldb"
                | "uld"
                | "ulwave"
                | "strike"
                | "striked"
                | "fs"
                | "cf"
                | "highlight"
                | "f"
                | "plain"
        );
        if styling {
            self.flush_run();
        }
        let on = param != Some(0);
        let st = self.stack.last_mut();
        let Some(st) = st else { return };
        match word {
            // Destinations whose contents are not body text.
            "fonttbl" | "colortbl" | "stylesheet" | "info" | "generator" | "pict" | "object"
            | "listtable" | "listoverridetable" => st.skip = true,
            "b" => st.run.set_bold(on),
            "i" => st.run.set_italic(on),
            "ul" => {
                st.run.underline = if on {
                    Underline::Single
                } else {
                    Underline::None
                }
            }
            "ulnone" => st.run.underline = Underline::None,
            "uldb" => st.run.underline = Underline::Double,
            "uld" => st.run.underline = Underline::Dotted,
            "ulwave" => st.run.underline = Underline::Wavy,
            "strike" | "striked" => st.run.strikethrough = on,
            "plain" => st.run = self.style.plain(),
            "fs" => {
                if let Some(half) = param {
                    let pts = half as f64 / 2.0;
                    if (pts - self.style.base_points).abs() < 0.01 {
                        st.run.font.style = self.style.base;
                        st.run.font.scale = 1.0;
                    } else {
                        st.run.font.style = Font::System(pts);
                        st.run.font.scale = 1.0;
                    }
                }
            }
            "cf" => st.run.color = color_at(&self.colors, param),
            "highlight" => st.run.background = color_at(&self.colors, param),
            "f" => {
                st.run.font.monospace = param.is_some_and(|n| self.mono_fonts.contains(&(n as u32)))
            }
            "par" => {
                self.flush_run();
                self.push_char('\n');
                self.run_start = self.doc.text.len();
                self.close_paragraph();
            }
            "line" => {
                self.flush_run();
                self.push_char('\n');
                self.run_start = self.doc.text.len();
            }
            "tab" => {
                self.push_char('\t');
            }
            "bullet" => self.push_char('\u{2022}'),
            "emdash" => self.push_char('—'),
            "endash" => self.push_char('–'),
            "lquote" => self.push_char('\u{2018}'),
            "rquote" => self.push_char('\u{2019}'),
            "ldblquote" => self.push_char('\u{201c}'),
            "rdblquote" => self.push_char('\u{201d}'),
            "u" => {
                if let Some(n) = param {
                    self.push_utf16(n as i16 as u16);
                    // A `\uN` is followed by a replacement character for old readers; skip it.
                    if self.at < self.src.len() && self.src[self.at] == b'?' {
                        self.at += 1;
                    }
                }
            }
            "pard" => self.para = ParagraphStyle::default(),
            "ql" => self.para.align = ParagraphAlign::Natural,
            "qc" => self.para.align = ParagraphAlign::Center,
            "qr" => self.para.align = ParagraphAlign::Trailing,
            "qj" => self.para.align = ParagraphAlign::Justified,
            "li" => self.para.indent = param.unwrap_or(0) as f64 / 20.0,
            "sb" => self.para.space_before = param.unwrap_or(0) as f64 / 20.0,
            "sa" => self.para.space_after = param.unwrap_or(0) as f64 / 20.0,
            _ => {}
        }
        // The tables are read from the SOURCE rather than as body text, because a group marked
        // `skip` produces none. Each leaves the cursor inside its group; `skip_group` then walks
        // to the matching brace so `run` never sees the contents.
        if word == "colortbl" {
            self.read_color_table();
            self.skip_group();
        }
        if word == "fonttbl" {
            self.read_font_table();
            self.skip_group();
        }
    }

    /// `\red0\green0\blue255;` entries up to the group's close. Index 0 is "the default", so an
    /// empty leading entry is normal and is recorded as black.
    fn read_color_table(&mut self) {
        let mut rgb = [0u8; 3];
        let mut seen = false;
        while self.at < self.src.len() && self.src[self.at] != b'}' {
            if self.src[self.at] == b'\\' {
                let start = self.at + 1;
                let mut e = start;
                while e < self.src.len() && self.src[e].is_ascii_alphabetic() {
                    e += 1;
                }
                let word = std::str::from_utf8(&self.src[start..e]).unwrap_or("");
                let ns = e;
                while e < self.src.len() && self.src[e].is_ascii_digit() {
                    e += 1;
                }
                let n: u8 = std::str::from_utf8(&self.src[ns..e])
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(0);
                match word {
                    "red" => {
                        rgb[0] = n;
                        seen = true;
                    }
                    "green" => {
                        rgb[1] = n;
                        seen = true;
                    }
                    "blue" => {
                        rgb[2] = n;
                        seen = true;
                    }
                    _ => {}
                }
                self.at = e;
                continue;
            }
            if self.src[self.at] == b';' {
                self.colors.push(if seen {
                    Color::rgb(
                        f64::from(rgb[0]) / 255.0,
                        f64::from(rgb[1]) / 255.0,
                        f64::from(rgb[2]) / 255.0,
                    )
                } else {
                    Color::BLACK
                });
                rgb = [0u8; 3];
                seen = false;
            }
            self.at += 1;
        }
    }

    /// `{\f1 Courier New;}` entries — only WHICH indices are monospaced, which is all the model
    /// can carry. Reads to the end of the enclosing group without consuming its closing brace;
    /// `skip_group` does that.
    fn read_font_table(&mut self) {
        let mut depth = 1i32;
        let mut current: Option<u32> = None;
        let mut name = String::new();
        let from = self.at;
        let mut at = self.at;
        while at < self.src.len() && depth > 0 {
            match self.src[at] {
                b'{' => {
                    depth += 1;
                    at += 1;
                }
                b'}' | b';' => {
                    if self.src[at] == b'}' {
                        depth -= 1;
                    }
                    if let Some(i) = current.take()
                        && is_mono_family(&name)
                    {
                        self.mono_fonts.push(i);
                    }
                    name.clear();
                    at += 1;
                }
                b'\\' => {
                    let s = at + 1;
                    let mut e = s;
                    while e < self.src.len() && self.src[e].is_ascii_alphabetic() {
                        e += 1;
                    }
                    let word = std::str::from_utf8(&self.src[s..e]).unwrap_or("");
                    let ns = e;
                    while e < self.src.len() && self.src[e].is_ascii_digit() {
                        e += 1;
                    }
                    let n: Option<u32> = std::str::from_utf8(&self.src[ns..e])
                        .ok()
                        .and_then(|s| s.parse().ok());
                    match word {
                        "f" => current = n,
                        // RTF's own marker for a fixed-pitch family, which is more reliable than
                        // the name for a font this reader has never heard of.
                        "fmodern" => {
                            if let Some(i) = current {
                                self.mono_fonts.push(i);
                            }
                        }
                        _ => {}
                    }
                    at = e;
                    if at < self.src.len() && self.src[at] == b' ' {
                        at += 1;
                    }
                }
                b => {
                    name.push(b as char);
                    at += 1;
                }
            }
        }
        // Leave the cursor where it started: `skip_group` walks the braces.
        self.at = from;
    }

    fn close_paragraph(&mut self) {
        if self.doc.text.len() <= self.para_start {
            self.para_start = self.doc.text.len();
            return;
        }
        if self.para != ParagraphStyle::default() {
            self.doc.paragraphs.push(ParagraphRun::new(
                self.para_start..self.doc.text.len(),
                self.para,
            ));
        }
        self.para_start = self.doc.text.len();
    }

    fn finish(mut self) -> StyledText {
        self.flush_run();
        self.close_paragraph();
        // The final `\par` is the document's terminator, not a blank last line.
        if self.doc.text.ends_with('\n') {
            self.doc.text.pop();
        }
        let base = self.style.base;
        self.doc.runs = coalesce_runs(std::mem::take(&mut self.doc.runs), base);
        self.doc.clamp();
        self.doc
    }
}

fn color_at(colors: &[Color], param: Option<i64>) -> Option<Color> {
    // `\cf0` is "the document's default color", i.e. nothing to say. The table's leading `;`
    // terminates that default entry, so the parsed vector holds it at index 0 and `\cfN` indexes
    // straight in — which is the off-by-one this indexed past when it subtracted.
    let i = param?;
    if i <= 0 {
        return None;
    }
    colors.get(i as usize).copied()
}

fn is_mono_family(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    ["courier", "consolas", "mono", "menlo", "monaco"]
        .iter()
        .any(|m| n.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn style() -> DocStyle {
        DocStyle::new(Font::Body, 16.0)
    }

    fn plain_doc(t: &str) -> StyledText {
        StyledText::plain(t)
    }

    #[test]
    fn markdown_inline_round_trips() {
        let md = "Save **now** or *lose* it.";
        let doc = markdown_to_styled(md, style());
        assert_eq!(doc.text, "Save now or lose it.");
        assert!(doc.validate().is_ok());
        assert_eq!(styled_to_markdown(&doc, style()), md);
    }

    #[test]
    fn markdown_headings_become_scaled_bold_runs() {
        let doc = markdown_to_styled("# Title\nbody", style());
        assert_eq!(doc.text, "Title\nbody");
        let s = doc.style_of(0..5, Font::Body);
        assert!(s.bold());
        assert_eq!(s.font.scale, 2.0);
        assert!(!doc.style_of(6..10, Font::Body).bold());
        assert_eq!(styled_to_markdown(&doc, style()), "# Title\nbody");
    }

    #[test]
    fn markdown_lists_become_paragraph_runs() {
        let doc = markdown_to_styled("- one\n- two\n1. a\n2. b", style());
        assert_eq!(doc.text, "one\ntwo\na\nb");
        let kinds: Vec<_> = doc.paragraphs.iter().map(|p| p.list).collect();
        assert_eq!(
            kinds,
            vec![
                ListStyle::Bullet,
                ListStyle::Bullet,
                ListStyle::Ordered(1),
                ListStyle::Ordered(2)
            ]
        );
        assert_eq!(
            styled_to_markdown(&doc, style()),
            "- one\n- two\n1. a\n2. b"
        );
    }

    #[test]
    fn markdown_quotes_indent() {
        let doc = markdown_to_styled("> quoted", style());
        assert_eq!(doc.text, "quoted");
        assert_eq!(doc.paragraphs[0].indent, 24.0);
        assert_eq!(styled_to_markdown(&doc, style()), "> quoted");
    }

    #[test]
    fn markdown_escapes_what_would_reparse() {
        let doc = plain_doc("2 * 3 * 4 and _x_");
        let md = styled_to_markdown(&doc, style());
        assert_eq!(markdown_to_styled(&md, style()).text, doc.text);
    }

    #[test]
    fn html_round_trips_the_attributes_it_can_write() {
        let mut doc = StyledText::plain("hello world");
        doc.apply(0..5, Font::Body, |s| {
            s.set_bold(true);
            s.underline = Underline::Single;
            s.color = Some(Color::hex(0xff0000));
        });
        doc.apply(6..11, Font::Body, |s| {
            s.strikethrough = true;
            s.background = Some(Color::hex(0x00ff00));
        });
        let html = styled_to_html(&doc, style());
        let back = html_to_styled(&html, style());
        assert_eq!(back.text, "hello world");
        let a = back.style_of(0..5, Font::Body);
        assert!(a.bold() && a.underline.is_on());
        assert_eq!(a.color, Some(Color::hex(0xff0000)));
        let b = back.style_of(6..11, Font::Body);
        assert!(b.strikethrough);
        assert_eq!(b.background, Some(Color::hex(0x00ff00)));
    }

    #[test]
    fn html_paragraphs_and_headings() {
        let doc = html_to_styled(
            "<h2>Title</h2><p style=\"text-align:center\">mid</p>",
            style(),
        );
        assert_eq!(doc.text, "Title\nmid");
        assert_eq!(doc.style_of(0..5, Font::Body).font.scale, 1.5);
        assert_eq!(doc.paragraph_style_of(6..9).align, ParagraphAlign::Center);
    }

    #[test]
    fn html_lists_number_themselves() {
        let doc = html_to_styled("<ol><li>a</li><li>b</li></ol>", style());
        assert_eq!(doc.text, "a\nb");
        assert_eq!(doc.paragraphs[0].list, ListStyle::Ordered(1));
        assert_eq!(doc.paragraphs[1].list, ListStyle::Ordered(2));
    }

    #[test]
    fn html_ignores_scripts_and_unknown_tags() {
        let doc = html_to_styled(
            "<p>a<script>var x = '<b>';</script><weird>b</weird></p>",
            style(),
        );
        assert_eq!(doc.text, "ab", "script contents are not text");
    }

    #[test]
    fn html_entities_and_multibyte_survive() {
        let doc = html_to_styled("<p>a &amp; b &#233; caf\u{e9} \u{1f600}</p>", style());
        assert_eq!(doc.text, "a & b é café 😀");
        assert!(doc.validate().is_ok());
    }

    #[test]
    fn rtf_round_trips_every_attribute_day_writes() {
        let mut doc = StyledText::plain("bold italic under strike big red");
        doc.apply(0..4, Font::Body, |s| s.set_bold(true));
        doc.apply(5..11, Font::Body, |s| s.set_italic(true));
        doc.apply(12..17, Font::Body, |s| s.underline = Underline::Single);
        doc.apply(18..24, Font::Body, |s| s.strikethrough = true);
        doc.apply(25..28, Font::Body, |s| s.font.scale = 1.5);
        doc.apply(29..32, Font::Body, |s| s.color = Some(Color::hex(0xcc0000)));

        let rtf = styled_to_rtf(&doc, style());
        let back = rtf_to_styled(&rtf, style());
        assert_eq!(back.text, doc.text);
        assert!(back.style_of(0..4, Font::Body).bold());
        assert!(back.style_of(5..11, Font::Body).font.italic);
        assert!(back.style_of(12..17, Font::Body).underline.is_on());
        assert!(back.style_of(18..24, Font::Body).strikethrough);
        assert_eq!(
            back.style_of(29..32, Font::Body).color,
            Some(Color::hex(0xcc0000))
        );
        // A relative scale becomes an absolute size in RTF and comes back as one.
        let big = back.style_of(25..28, Font::Body);
        assert_eq!(big.font.resolved_points(16.0), 24.0);
    }

    #[test]
    fn rtf_paragraph_attributes_round_trip() {
        let mut doc = StyledText::plain("one\ntwo");
        doc.apply_paragraph(0..1, |p| p.align = ParagraphAlign::Center);
        doc.apply_paragraph(5..5, |p| p.indent = 36.0);
        let back = rtf_to_styled(&styled_to_rtf(&doc, style()), style());
        assert_eq!(back.text, "one\ntwo");
        assert_eq!(back.paragraph_style_of(0..1).align, ParagraphAlign::Center);
        assert_eq!(back.paragraph_style_of(5..5).indent, 36.0);
    }

    #[test]
    fn rtf_escapes_and_unicode_survive() {
        let doc = plain_doc("a{b}c\\d é 😀");
        let back = rtf_to_styled(&styled_to_rtf(&doc, style()), style());
        assert_eq!(back.text, "a{b}c\\d é 😀");
    }

    #[test]
    fn rtf_skips_destinations_and_tables() {
        // What TextEdit puts around a document: a `\*` destination and a font table with a name
        // that must not leak into the text.
        let rtf = "{\\rtf1\\ansi{\\fonttbl{\\f0\\fswiss Helvetica;}}{\\*\\expandedcolortbl;;}\
                   {\\*\\generator Riched20;}\\fs32 hello\\par}";
        let doc = rtf_to_styled(rtf, style());
        assert_eq!(doc.text, "hello", "got {:?}", doc.text);
    }

    #[test]
    fn every_codec_survives_hostile_input_without_panicking() {
        for bad in [
            "",
            "{",
            "}",
            "\\",
            "{\\rtf1\\ansi\\u",
            "{\\rtf1{{{{{{",
            "{\\rtf1\\'zz}",
            "{\\colortbl",
            "<",
            "<p",
            "<p style=",
            "<a href=",
            "&#",
            "&#xZZ;",
            "<script>",
            "**unclosed",
            "# ",
            "- ",
            "1. ",
        ] {
            let _ = rtf_to_styled(bad, style());
            let _ = html_to_styled(bad, style());
            let d = markdown_to_styled(bad, style());
            assert!(d.validate().is_ok(), "markdown {bad:?} produced bad ranges");
        }
    }
}
