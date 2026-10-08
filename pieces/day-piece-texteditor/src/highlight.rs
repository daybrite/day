// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Syntax highlighting as runs (docs/texteditor.md §9): a tokenizer is a pure function from text
//! to `Vec<TextRun>`, and [`TextEditor::highlight`](crate::TextEditor::highlight) re-runs it on
//! every edit and pushes the runs back as an attributes patch, so the caret and the undo stack
//! stay where they are. Nothing here touches a toolkit; the same runs restyle all eight arms,
//! and a test asserts them on the headless one.
//!
//! The grammars are small on purpose: an API client's bodies (JSON, XML, GraphQL), Day's own
//! samples (Rust), and the template tags Yaak-style clients write inside any of them
//! (`${[ name ]}`). A full grammar engine is a dependency this crate does not want; a new
//! language is a function below.

use day_spec::{Color, Font, FontWeight, RunStyle, TextRun};

/// The colors a highlighter paints with. Every one is readable on a light and on a dark surface,
/// which matters because the editor's background is the platform's and changes with the theme
/// while the runs do not.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    /// The style plain text takes: the font every run is set in (monospace by default) and the
    /// color unclaimed text keeps (`None` = the platform's label color).
    pub base: RunStyle,
    pub keyword: Color,
    pub string: Color,
    pub number: Color,
    pub comment: Color,
    /// An object key, an attribute name, an argument name, a variable.
    pub key: Color,
    /// An element or type name.
    pub name: Color,
    /// The fill behind a template tag (`${[ … ]}`), and its text color.
    pub template: Color,
    pub template_text: Color,
}

impl Default for Palette {
    fn default() -> Self {
        let mut base = RunStyle::plain(Font::Body);
        base.font.monospace = true;
        Palette {
            base,
            keyword: Color::hex(0x7C5CD6),
            string: Color::hex(0x1E9E86),
            number: Color::hex(0xE86A3C),
            comment: Color::hex(0x64748B),
            key: Color::hex(0x2F6FDE),
            name: Color::hex(0xC2491D),
            template: Color::rgba(0.94, 0.65, 0.30, 0.28),
            template_text: Color::hex(0xB45309),
        }
    }
}

impl Palette {
    /// The same colors over a proportional base font, for prose-like documents.
    pub fn proportional(mut self) -> Self {
        self.base.font.monospace = false;
        self
    }
    /// The base font the runs scale against (default `Body`).
    pub fn base_font(mut self, font: Font) -> Self {
        let monospace = self.base.font.monospace;
        self.base = RunStyle::plain(font);
        self.base.font.monospace = monospace;
        self
    }
}

/// What a text is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    /// Strings, numbers, `true`/`false`/`null`, object keys, and the `//` / `/* */` comments
    /// JSON-with-comments allows.
    Json,
    /// Tags, attributes, comments, processing instructions; HTML reads the same way.
    Xml,
    /// Keywords, names, arguments, `$variables`, `#` comments.
    GraphQl,
    /// Keywords, strings, numbers, `//` comments.
    Rust,
    /// No tokens: every character in the palette's base style.
    Plain,
}

impl Language {
    /// The language a MIME type names, for a response body: `application/json` and its `+json`
    /// suffixes, XML and HTML, GraphQL, else plain.
    pub fn for_mime(mime: &str) -> Language {
        let essence = mime
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if essence.ends_with("json") {
            Language::Json
        } else if essence.ends_with("xml") || essence == "text/html" {
            Language::Xml
        } else if essence.ends_with("graphql") {
            Language::GraphQl
        } else {
            Language::Plain
        }
    }
}

/// Tokenize `text` into runs covering every byte: claimed tokens in their colors, the gaps
/// between them in the palette's base style. Byte offsets throughout, which is what the document
/// indexes by, and why a multi-byte character in a comment cannot shift the styling of what
/// follows it.
pub fn highlight(language: Language, text: &str, palette: &Palette) -> Vec<TextRun> {
    let mut sink = Sink::new(palette);
    match language {
        Language::Json => json(text, &mut sink),
        Language::Xml => xml(text, &mut sink),
        Language::GraphQl => graphql(text, &mut sink),
        Language::Rust => rust(text, &mut sink),
        Language::Plain => {}
    }
    sink.finish(text.len())
}

/// [`highlight`], then every `${[ … ]}` template tag painted over whatever token it sits in.
pub fn highlight_with_templates(language: Language, text: &str, palette: &Palette) -> Vec<TextRun> {
    let runs = highlight(language, text, palette);
    template_tags(text, runs, palette)
}

/// Paint every `${[ … ]}` tag in `text` over `runs`, splitting the runs it overlaps. The tag is
/// what a Yaak-style client renders a variable or a function call as; an unclosed `${[` is left
/// as it was typed.
pub fn template_tags(text: &str, runs: Vec<TextRun>, palette: &Palette) -> Vec<TextRun> {
    let mut tags: Vec<std::ops::Range<usize>> = Vec::new();
    let mut from = 0usize;
    while let Some(rel) = text[from..].find("${[") {
        let start = from + rel;
        let Some(len) = text[start + 3..].find("]}") else {
            break;
        };
        let end = start + 3 + len + 2;
        tags.push(start..end);
        from = end;
    }
    if tags.is_empty() {
        return runs;
    }
    // One merge pass over runs and tags, both sorted and disjoint: a run is split where a tag
    // starts or ends inside it, and the part under the tag takes the tag's colors over the
    // run's own font. (`StyledText::apply` per tag would be a pass over every run per tag,
    // which was quadratic on a body with a tag per line.)
    let mut out = Vec::with_capacity(runs.len() + 2 * tags.len());
    let mut ti = 0usize;
    for run in runs {
        let style = run.style();
        let mut at = run.range.start;
        let end = run.range.end;
        while at < end {
            while ti < tags.len() && tags[ti].end <= at {
                ti += 1;
            }
            match tags.get(ti) {
                Some(tag) if tag.start < end => {
                    if tag.start > at {
                        out.push(TextRun::styled(at..tag.start, style.clone()));
                    }
                    let (s, e) = (at.max(tag.start), end.min(tag.end));
                    let mut tagged = style.clone();
                    tagged.background = Some(palette.template);
                    tagged.color = Some(palette.template_text);
                    out.push(TextRun::styled(s..e, tagged));
                    at = e;
                }
                _ => {
                    out.push(TextRun::styled(at..end, style.clone()));
                    at = end;
                }
            }
        }
    }
    out
}

/// A closure for [`TextEditor::highlight`](crate::TextEditor::highlight): `language` tokens,
/// template tags on top when `templates` is set.
pub fn highlighter(
    language: Language,
    palette: Palette,
    templates: bool,
) -> impl Fn(&str) -> Vec<TextRun> + 'static {
    move |text| {
        if templates {
            highlight_with_templates(language, text, &palette)
        } else {
            highlight(language, text, &palette)
        }
    }
}

// ---------------------------------------------------------------------------
// The run sink: tokens arrive in order; the gaps between them become base-style runs, because
// a code document is monospaced from end to end, not only where a token was recognized.
// ---------------------------------------------------------------------------

struct Sink<'p> {
    palette: &'p Palette,
    runs: Vec<TextRun>,
    plain_from: usize,
}

#[derive(Clone, Copy)]
enum Tok {
    Keyword,
    String,
    Number,
    Comment,
    Key,
    Name,
}

impl<'p> Sink<'p> {
    fn new(palette: &'p Palette) -> Self {
        Sink {
            palette,
            runs: Vec::new(),
            plain_from: 0,
        }
    }

    fn gap(&mut self, upto: usize) {
        if self.plain_from < upto {
            self.runs.push(TextRun::styled(
                self.plain_from..upto,
                self.palette.base.clone(),
            ));
        }
        self.plain_from = upto;
    }

    fn token(&mut self, range: std::ops::Range<usize>, tok: Tok) {
        if range.start >= range.end {
            return;
        }
        self.gap(range.start);
        let mut style = self.palette.base.clone();
        style.color = Some(match tok {
            Tok::Keyword => self.palette.keyword,
            Tok::String => self.palette.string,
            Tok::Number => self.palette.number,
            Tok::Comment => self.palette.comment,
            Tok::Key => self.palette.key,
            Tok::Name => self.palette.name,
        });
        match tok {
            Tok::Keyword => style.font.weight = Some(FontWeight::Bold),
            Tok::Comment => style.font.italic = true,
            _ => {}
        }
        self.plain_from = range.end;
        self.runs.push(TextRun::styled(range, style));
    }

    fn finish(mut self, len: usize) -> Vec<TextRun> {
        self.gap(len);
        self.runs
    }
}

// ---------------------------------------------------------------------------
// Lexing helpers over bytes. Every scanner advances on character boundaries: a token never ends
// inside a multi-byte character, because the only bytes it stops on are ASCII.
// ---------------------------------------------------------------------------

/// The end of a quoted string starting at `i` (the opening quote), escapes honored; the end of
/// the text when it never closes.
fn string_end(src: &str, i: usize, quote: u8) -> usize {
    let b = src.as_bytes();
    let mut j = i + 1;
    while j < b.len() {
        match b[j] {
            b'\\' => j += 2,
            c if c == quote => return (j + 1).min(b.len()),
            _ => j += 1,
        }
    }
    b.len()
}

/// The end of the line starting at `i` (the newline excluded).
fn line_end(src: &str, i: usize) -> usize {
    src[i..].find('\n').map_or(src.len(), |n| i + n)
}

/// The end of a block comment opened at `i`, `close` included; the end of the text if unclosed.
fn block_end(src: &str, i: usize, open_len: usize, close: &str) -> usize {
    src[i + open_len..]
        .find(close)
        .map_or(src.len(), |n| i + open_len + n + close.len())
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The end of the word (letters, digits, `_`) starting at `i`.
fn word_end(src: &str, i: usize) -> usize {
    src[i..]
        .find(|c: char| !is_word(c))
        .map_or(src.len(), |n| i + n)
}

/// The end of a number starting at `i`: digits, one `.`, an exponent, a sign after it.
fn number_end(src: &str, i: usize) -> usize {
    let b = src.as_bytes();
    let mut j = i;
    if j < b.len() && b[j] == b'-' {
        j += 1;
    }
    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.' || b[j] == b'_') {
        j += 1;
    }
    if j < b.len() && (b[j] == b'e' || b[j] == b'E') {
        j += 1;
        if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
            j += 1;
        }
        while j < b.len() && b[j].is_ascii_digit() {
            j += 1;
        }
    }
    j
}

/// Whether the first non-blank character after `i` is `what`: what makes a JSON string a key.
fn followed_by(src: &str, i: usize, what: u8) -> bool {
    src.as_bytes()[i..]
        .iter()
        .find(|c| !c.is_ascii_whitespace())
        .is_some_and(|c| *c == what)
}

// ---------------------------------------------------------------------------
// The languages.
// ---------------------------------------------------------------------------

fn json(src: &str, sink: &mut Sink<'_>) {
    let b = src.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'"' {
            let end = string_end(src, i, b'"');
            let tok = if followed_by(src, end, b':') {
                Tok::Key
            } else {
                Tok::String
            };
            sink.token(i..end, tok);
            i = end;
        } else if c == b'/' && src[i..].starts_with("//") {
            let end = line_end(src, i);
            sink.token(i..end, Tok::Comment);
            i = end;
        } else if c == b'/' && src[i..].starts_with("/*") {
            let end = block_end(src, i, 2, "*/");
            sink.token(i..end, Tok::Comment);
            i = end;
        } else if c.is_ascii_digit() || (c == b'-' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let end = number_end(src, i);
            sink.token(i..end, Tok::Number);
            i = end;
        } else if c.is_ascii_alphabetic() {
            let end = word_end(src, i);
            if matches!(&src[i..end], "true" | "false" | "null") {
                sink.token(i..end, Tok::Keyword);
            }
            i = end;
        } else {
            i += src[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
}

fn xml(src: &str, sink: &mut Sink<'_>) {
    let b = src.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        if src[i..].starts_with("<!--") {
            let end = block_end(src, i, 4, "-->");
            sink.token(i..end, Tok::Comment);
            i = end;
        } else if src[i..].starts_with("<![CDATA[") {
            let end = block_end(src, i, 9, "]]>");
            sink.token(i..end, Tok::String);
            i = end;
        } else if b[i] == b'<' {
            // A tag: `<name`, `</name`, `<?name`, `<!NAME`, then attributes to the closing `>`.
            let mut j = i + 1;
            while j < b.len() && matches!(b[j], b'/' | b'?' | b'!') {
                j += 1;
            }
            let name_end = src[j..]
                .find(|c: char| !(is_word(c) || c == ':' || c == '-' || c == '.'))
                .map_or(src.len(), |n| j + n);
            sink.token(j..name_end, Tok::Name);
            j = name_end;
            // Attributes until `>`; a quoted value is a string, a bare word before `=` a key.
            while j < b.len() && b[j] != b'>' {
                let c = b[j];
                if c == b'"' || c == b'\'' {
                    let end = string_end(src, j, c);
                    sink.token(j..end, Tok::String);
                    j = end;
                } else if is_word(c as char) {
                    let end = src[j..]
                        .find(|c: char| !(is_word(c) || c == ':' || c == '-' || c == '.'))
                        .map_or(src.len(), |n| j + n);
                    sink.token(j..end, Tok::Key);
                    j = end;
                } else {
                    j += src[j..].chars().next().map_or(1, char::len_utf8);
                }
            }
            i = (j + 1).min(b.len());
        } else {
            i += src[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
}

const GRAPHQL_KEYWORDS: &[&str] = &[
    "query",
    "mutation",
    "subscription",
    "fragment",
    "on",
    "type",
    "input",
    "enum",
    "interface",
    "union",
    "scalar",
    "schema",
    "extend",
    "directive",
    "implements",
    "repeatable",
    "true",
    "false",
    "null",
];

fn graphql(src: &str, sink: &mut Sink<'_>) {
    let b = src.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'#' {
            let end = line_end(src, i);
            sink.token(i..end, Tok::Comment);
            i = end;
        } else if src[i..].starts_with("\"\"\"") {
            let end = block_end(src, i, 3, "\"\"\"");
            sink.token(i..end, Tok::String);
            i = end;
        } else if c == b'"' {
            let end = string_end(src, i, b'"');
            sink.token(i..end, Tok::String);
            i = end;
        } else if c == b'$' {
            let end = word_end(src, i + 1);
            sink.token(i..end, Tok::Key);
            i = end;
        } else if c.is_ascii_digit() || (c == b'-' && b.get(i + 1).is_some_and(u8::is_ascii_digit))
        {
            let end = number_end(src, i);
            sink.token(i..end, Tok::Number);
            i = end;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let end = word_end(src, i);
            let word = &src[i..end];
            if GRAPHQL_KEYWORDS.contains(&word) {
                sink.token(i..end, Tok::Keyword);
            } else if followed_by(src, end, b':') {
                sink.token(i..end, Tok::Key);
            } else if word.chars().next().is_some_and(char::is_uppercase) {
                sink.token(i..end, Tok::Name);
            }
            i = end;
        } else {
            i += src[i..].chars().next().map_or(1, char::len_utf8);
        }
    }
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub",
    "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type",
    "unsafe", "use", "where", "while",
];

fn rust(src: &str, sink: &mut Sink<'_>) {
    let b = src.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if src[i..].starts_with("//") {
            let end = line_end(src, i);
            sink.token(i..end, Tok::Comment);
            i = end;
        } else if src[i..].starts_with("/*") {
            let end = block_end(src, i, 2, "*/");
            sink.token(i..end, Tok::Comment);
            i = end;
        } else if c == b'"' {
            let end = string_end(src, i, b'"');
            sink.token(i..end, Tok::String);
            i = end;
        } else if c.is_ascii_digit() {
            let end = word_end(src, i).max(number_end(src, i));
            sink.token(i..end, Tok::Number);
            i = end;
        } else if is_word(c as char) || c >= 0x80 {
            let end = word_end(src, i);
            if RUST_KEYWORDS.contains(&&src[i..end]) {
                sink.token(i..end, Tok::Keyword);
            }
            i = end.max(i + src[i..].chars().next().map_or(1, char::len_utf8));
        } else {
            i += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn colors(runs: &[TextRun], text: &str) -> Vec<(String, Option<Color>)> {
        runs.iter()
            .map(|r| (text[r.range.clone()].to_string(), r.color))
            .collect()
    }

    #[test]
    fn runs_cover_every_byte_in_order() {
        let p = Palette::default();
        for (lang, text) in [
            (Language::Json, "{\"a\": [1, true, \"é\"]} // c"),
            (Language::Xml, "<a href=\"x\">é<!-- c --></a>"),
            (
                Language::GraphQl,
                "query Q($id: ID!) { user(id: $id) { name } } # é",
            ),
            (Language::Rust, "fn main() { let s = \"é\"; } // c"),
            (Language::Plain, "anything é"),
        ] {
            let runs = highlight(lang, text, &p);
            let mut at = 0;
            for r in &runs {
                assert_eq!(r.range.start, at, "{lang:?}: gap before {:?}", r.range);
                assert!(text.is_char_boundary(r.range.end));
                assert!(r.font.monospace);
                at = r.range.end;
            }
            assert_eq!(at, text.len(), "{lang:?}");
        }
    }

    #[test]
    fn json_tells_keys_from_strings_and_colors_the_literals() {
        let p = Palette::default();
        let text = "{\"name\": \"Ada\", \"n\": -1.5e3, \"ok\": null}";
        let got = colors(&highlight(Language::Json, text, &p), text);
        assert!(got.contains(&("\"name\"".into(), Some(p.key))));
        assert!(got.contains(&("\"Ada\"".into(), Some(p.string))));
        assert!(got.contains(&("-1.5e3".into(), Some(p.number))));
        assert!(got.contains(&("null".into(), Some(p.keyword))));
        assert!(got.contains(&(": ".into(), None)), "{got:?}");
    }

    #[test]
    fn json_comments_run_to_their_end() {
        let p = Palette::default();
        let text = "// top\n{/* in */ \"a\": 1}";
        let got = colors(&highlight(Language::Json, text, &p), text);
        assert_eq!(got[0], ("// top".into(), Some(p.comment)));
        assert!(got.contains(&("/* in */".into(), Some(p.comment))));
    }

    #[test]
    fn xml_names_attributes_and_values() {
        let p = Palette::default();
        let text = "<?xml version=\"1.0\"?><ns:item id='7'>text</ns:item>";
        let got = colors(&highlight(Language::Xml, text, &p), text);
        assert!(got.contains(&("xml".into(), Some(p.name))));
        assert!(got.contains(&("ns:item".into(), Some(p.name))));
        assert!(got.contains(&("id".into(), Some(p.key))));
        assert!(got.contains(&("'7'".into(), Some(p.string))));
        // Element text is part of the plain gap between the tags.
        assert!(
            got.iter().any(|(t, c)| t.contains("text") && c.is_none()),
            "{got:?}"
        );
    }

    #[test]
    fn graphql_keywords_arguments_and_variables() {
        let p = Palette::default();
        let text = "query Q($id: ID!) { user(id: $id) { name } }";
        let got = colors(&highlight(Language::GraphQl, text, &p), text);
        assert!(got.contains(&("query".into(), Some(p.keyword))));
        assert!(got.contains(&("$id".into(), Some(p.key))));
        assert!(got.contains(&("id".into(), Some(p.key))));
        assert!(got.contains(&("ID".into(), Some(p.name))));
        // A plain field name stays in the gap with its braces.
        assert!(
            got.iter().any(|(t, c)| t.contains("name") && c.is_none()),
            "{got:?}"
        );
    }

    #[test]
    fn template_tags_paint_over_tokens_and_leave_an_unclosed_one() {
        let p = Palette::default();
        let text = "{\"url\": \"${[ base ]}/x\", \"k\": \"${[ open\"}";
        let runs = highlight_with_templates(Language::Json, text, &p);
        let tag = runs
            .iter()
            .find(|r| &text[r.range.clone()] == "${[ base ]}")
            .expect("the tag is one run");
        assert_eq!(tag.background, Some(p.template));
        assert_eq!(tag.color, Some(p.template_text));
        // The string around it is still a string, split in two.
        let after = runs
            .iter()
            .find(|r| &text[r.range.clone()] == "/x\"")
            .expect("the tail of the string");
        assert_eq!(after.color, Some(p.string));
        assert!(
            runs.iter()
                .all(|r| r.background.is_none() || &text[r.range.clone()] == "${[ base ]}")
        );
    }

    /// The tokenizer's cost on a large body, printed rather than asserted: run with
    /// `cargo test -p day-piece-texteditor highlight_cost -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn highlight_cost_on_a_large_body() {
        let p = Palette::default();
        let item = "  {\"id\": 12345, \"name\": \"${[ user ]}\", \"tags\": [\"a\", \"b\"], \"ok\": true, \"note\": null},\n";
        for kb in [100usize, 1000] {
            let mut text = String::from("[\n");
            while text.len() < kb * 1024 {
                text.push_str(item);
            }
            text.push_str("]\n");
            let t = std::time::Instant::now();
            let runs = highlight_with_templates(Language::Json, &text, &p);
            let took = t.elapsed();
            println!("{kb} KB: {} runs in {took:?}", runs.len());
        }
    }

    #[test]
    fn mime_types_pick_a_language() {
        assert_eq!(
            Language::for_mime("application/json; charset=utf-8"),
            Language::Json
        );
        assert_eq!(
            Language::for_mime("application/problem+json"),
            Language::Json
        );
        assert_eq!(Language::for_mime("text/html"), Language::Xml);
        assert_eq!(Language::for_mime("image/svg+xml"), Language::Xml);
        assert_eq!(Language::for_mime("text/plain"), Language::Plain);
    }
}
