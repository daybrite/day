// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Leaf pieces, the childless primitives: `label`, `link`, `button`, `toggle`, `slider`,
//! `text_field`, `progress`/`spinner`, `divider`, and `spacer`.

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{bind, bind_seeded};
use day_spec::props::*;
use day_spec::{Event, Font, Proposal, Rect, Role, Size, kinds};

use crate::*;

// ---------------------------------------------------------------------------
// Leaves
// ---------------------------------------------------------------------------

/// Build a styled paragraph run by run (docs/text-runs.md).
///
/// A builder exists because byte ranges are error-prone to write by hand and meaningless to
/// read: `TextRun { range: 12..19, .. }` says nothing about which word it covers. Appending text
/// and its style together keeps the two from drifting apart.
///
/// ```ignore
/// label("").runs_from(
///     TextBuilder::new()
///         .text("Saved to ")
///         .code("~/Documents")
///         .text(" just now — ")
///         .strong("do not close")
///         .text(" the window."),
/// )
/// ```
#[derive(Clone, Debug, Default)]
#[must_use]
pub struct TextBuilder {
    text: String,
    runs: Vec<day_spec::TextRun>,
    base: Font,
}

impl TextBuilder {
    pub fn new() -> Self {
        Self::default()
    }
    /// The style the emphasis variants build on. Set it to the label's font so a bold run
    /// inside a `Footnote` paragraph stays footnote-sized.
    pub fn base(mut self, font: Font) -> Self {
        self.base = font;
        self
    }
    /// Unstyled text: it draws with the label's own font.
    pub fn text(mut self, s: &str) -> Self {
        self.text.push_str(s);
        self
    }
    /// A run with a fully specified style.
    pub fn run(
        mut self,
        s: &str,
        run: impl FnOnce(std::ops::Range<usize>) -> day_spec::TextRun,
    ) -> Self {
        let start = self.text.len();
        self.text.push_str(s);
        self.runs.push(run(start..self.text.len()));
        self
    }
    /// Bold.
    pub fn strong(self, s: &str) -> Self {
        let base = self.base;
        self.run(s, move |range| {
            day_spec::TextRun::font(
                range,
                day_spec::FontSpec {
                    style: base,
                    weight: Some(day_spec::FontWeight::Bold),
                    ..Default::default()
                },
            )
        })
    }
    /// Italic.
    pub fn emphasis(self, s: &str) -> Self {
        let base = self.base;
        self.run(s, move |range| {
            day_spec::TextRun::font(
                range,
                day_spec::FontSpec {
                    style: base,
                    italic: true,
                    ..Default::default()
                },
            )
        })
    }
    /// Inline code: the platform's monospaced face at this style's size.
    pub fn code(self, s: &str) -> Self {
        let base = self.base;
        self.run(s, move |range| {
            day_spec::TextRun::font(
                range,
                day_spec::FontSpec {
                    style: base,
                    monospace: true,
                    ..Default::default()
                },
            )
        })
    }
    /// A colored phrase: any ink, so `SystemColor::Destructive` or a light/dark pair follows
    /// the appearance.
    pub fn colored(self, s: &str, color: impl Into<day_spec::Ink>) -> Self {
        let base = self.base;
        let color = color.into();
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            color: Some(color),
            ..day_spec::TextRun::default()
        })
    }
    /// Underlined with one solid line in the text color.
    pub fn underline(self, s: &str) -> Self {
        self.underline_styled(s, day_spec::Decoration::SINGLE)
    }
    /// Underlined with a line of your choosing: `Decoration::wavy()`, `double()` or
    /// `dotted()`, with `.color(c)` for a line that is not the text color. A diagnostic mark is
    /// `.underline_styled("unresolved", Decoration::wavy().color(red))`. A `DecorationStyle` converts.
    pub fn underline_styled(self, s: &str, decoration: impl Into<day_spec::Decoration>) -> Self {
        let base = self.base;
        let decoration = decoration.into();
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            underline: Some(decoration),
            ..day_spec::TextRun::default()
        })
    }
    /// Lowercase letters drawn as small capitals
    /// ([`FontSpec::small_caps`](day_spec::FontSpec::small_caps)).
    pub fn small_caps(self, s: &str) -> Self {
        let base = self.base;
        self.run(s, move |range| {
            day_spec::TextRun::font(
                range,
                day_spec::FontSpec {
                    style: base,
                    small_caps: true,
                    ..Default::default()
                },
            )
        })
    }
    /// Highlighted: a color painted behind the glyphs, for a search hit or a review mark.
    ///
    /// Sets the foreground too, through the same readable-on-a-fill rule `Button::tint` uses: a
    /// highlight is usually a pale wash, and the label's own text color is chosen for the window's
    /// background rather than for the swatch now sitting under it. On a dark theme that pairing
    /// puts light text on pale amber, which is the one combination a highlight must not produce.
    /// Use [`TextBuilder::run`] where an app wants to state both itself.
    pub fn highlight(self, s: &str, color: impl Into<day_spec::Ink>) -> Self {
        let base = self.base;
        let color = color.into();
        let on = color.map(day_spec::props::ButtonStyleSpec::on_tint);
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            background: Some(color),
            color: Some(on),
            ..day_spec::TextRun::default()
        })
    }
    /// A relative size: `1.5` is half again the base style's, `0.8` smaller
    /// ([`FontSpec::scale`](day_spec::FontSpec::scale)). Relative rather than a point size so the
    /// phrase still tracks the reader's accessibility text-size setting.
    pub fn sized(self, s: &str, scale: f64) -> Self {
        let base = self.base;
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base).scaled(scale),
            ..day_spec::TextRun::default()
        })
    }
    /// Struck through with one solid line in the text color.
    pub fn strikethrough(self, s: &str) -> Self {
        self.strikethrough_styled(s, day_spec::Decoration::SINGLE)
    }
    /// Struck through with a line of your choosing, as [`TextBuilder::underline_styled`].
    pub fn strikethrough_styled(
        self,
        s: &str,
        decoration: impl Into<day_spec::Decoration>,
    ) -> Self {
        let base = self.base;
        let decoration = decoration.into();
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            strikethrough: Some(decoration),
            ..day_spec::TextRun::default()
        })
    }
    /// A link run. A `#route` target navigates in-app, as with [`link`].
    /// Rendering it is `Cap::TextRuns`; activating it is `Cap::TextLinks`, which
    /// fewer backends have, so check before relying on the tap (docs/text-runs.md).
    pub fn link(self, s: &str, target: &str) -> Self {
        let base = self.base;
        let target = target.to_string();
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            link: Some(target),
            ..day_spec::TextRun::default()
        })
    }
    /// The assembled text and its runs.
    #[must_use]
    pub fn build(self) -> (String, Vec<day_spec::TextRun>) {
        (self.text, self.runs)
    }
}

#[must_use]
pub struct Label {
    pub(crate) text: TextSource,
    pub(crate) font: Font,
    pub(crate) font_scale: Reactive<f64>,
    pub(crate) max_text_scale: Option<f64>,
    pub(crate) weight: Option<day_spec::FontWeight>,
    pub(crate) italic: bool,
    pub(crate) tabular: bool,
    pub(crate) monospace: bool,
    pub(crate) small_caps: bool,
    pub(crate) oldstyle_figures: bool,
    pub(crate) wraps: bool,
    pub(crate) max_lines: Reactive<u32>,
    pub(crate) truncation: Reactive<Truncation>,
    pub(crate) letter_spacing: Reactive<f64>,
    pub(crate) line_height: Reactive<f64>,
    pub(crate) color: Option<Reactive<day_spec::Ink>>,
    /// What the text means, for the color the platform gives it (docs/text.md).
    pub(crate) role: day_spec::props::TextRole,
    /// Styled spans over `text` (docs/text-runs.md); empty is an ordinary uniform label.
    pub(crate) runs: Vec<day_spec::TextRun>,
    /// Parse the text as inline markdown instead of taking it literally (docs/text-runs.md).
    pub(crate) markdown: bool,
    /// How wrapped lines sit within the label's own width (docs/text.md).
    pub(crate) align: day_spec::props::TextAlign,
    /// A line under the whole text.
    pub(crate) underline: Option<day_spec::Decoration>,
    /// A line through the whole text.
    pub(crate) strikethrough: Option<day_spec::Decoration>,
    /// The target the whole label links to ([`link`]).
    pub(crate) link: Option<String>,
    /// What a tapped link run does. `None` uses [`open_link`].
    pub(crate) on_link: Option<LinkHandler>,
    /// Told when the reader's selection changes; setting it makes the label selectable.
    pub(crate) on_selection: Option<SelectionHandler>,
}

/// An app's handler for a tapped link run, shared because `Label` is cloned into its build.
pub(crate) type LinkHandler = Rc<dyn Fn(&str)>;

/// An app's handler for a selection change in a selectable label.
pub(crate) type SelectionHandler = Rc<dyn Fn(&SelectedText)>;

/// What the reader has selected in a label ([`Label::on_selection`]).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SelectedText {
    /// The selected text, as the label displays it (Markdown markers already stripped).
    /// Empty when the selection was cleared.
    pub text: String,
    /// The selection as a byte range into the displayed text. Empty when nothing is selected.
    pub range: std::ops::Range<usize>,
}

impl SelectedText {
    /// Whether nothing is selected.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.range.is_empty()
    }
}

pub fn label<M>(text: impl IntoText<M>) -> Label {
    Label::from_source(text.into_text())
}

impl Label {
    /// A plain `Body` label over an already converted text source.
    pub(crate) fn from_source(text: TextSource) -> Label {
        Label {
            text,
            font: Font::Body,
            font_scale: Reactive::Const(1.0),
            max_text_scale: None,
            weight: None,
            italic: false,
            tabular: false,
            monospace: false,
            small_caps: false,
            oldstyle_figures: false,
            wraps: true,
            max_lines: Reactive::Const(0),
            truncation: Reactive::Const(Truncation::Tail),
            letter_spacing: Reactive::Const(0.0),
            line_height: Reactive::Const(0.0),
            color: None,
            role: TextRole::default(),
            runs: Vec::new(),
            markdown: false,
            align: day_spec::props::TextAlign::Leading,
            underline: None,
            strikethrough: None,
            link: None,
            on_link: None,
            on_selection: None,
        }
    }
}

#[day_macros::builder(LabelBuilder)]
impl Label {
    /// Keep the text on one line and end it with an ellipsis when it does not fit
    /// ([`Label::truncation`] says where). Constrain the width through the enclosing layout.
    pub fn single_line(mut self) -> Self {
        self.wraps = false;
        self
    }
    /// Show at most this many lines, ending the last one with an ellipsis when text is left
    /// over. Zero means unlimited. The label measures no taller than the limit, on every
    /// toolkit. A constant, a `Signal<u32>` or a closure: a "Show more" toggle is
    /// `.max_lines(move || if expanded.get() { 0 } else { 3 })`.
    pub fn max_lines<M>(mut self, lines: impl IntoReactive<u32, M>) -> Self {
        self.max_lines = lines.into_reactive();
        self
    }
    /// Where the ellipsis goes when the text does not fit: `Truncation::Tail` (the default),
    /// `Head`, or `Middle` for a file path or an identifier whose two ends both matter.
    /// Applies to a label held to one line or to [`Label::max_lines`]; `Head` and `Middle`
    /// are guaranteed on one line (docs/text.md "Truncation").
    pub fn truncation<M>(mut self, mode: impl IntoReactive<Truncation, M>) -> Self {
        self.truncation = mode.into_reactive();
        self
    }
    /// Extra space after each character, in ems: `0.05` adds 5% of the font size, a negative
    /// value tightens. Relative, so the tracking follows the reader's text size.
    pub fn letter_spacing<M>(mut self, ems: impl IntoReactive<f64, M>) -> Self {
        self.letter_spacing = ems.into_reactive();
        self
    }
    /// The distance between baselines as a multiple of the font size: `1.5` for airy body
    /// text, `1.0` for tightly set display type. The font's own line height (about `1.2`)
    /// applies until this is set, and again for a value of zero.
    pub fn line_height<M>(mut self, multiple: impl IntoReactive<f64, M>) -> Self {
        self.line_height = multiple.into_reactive();
        self
    }
    /// The semantic text style (`Font::Title`, `Font::Footnote`, …) or a custom `Font::System(pt)`.
    /// Backends render it with the platform's native style + accessibility text scaling.
    pub fn font(mut self, f: Font) -> Self {
        self.font = f;
        self
    }
    /// Scale the resolved native font, preserving its semantic style and accessibility size.
    /// A constant, a signal or a closure. Applies to the label's base font; explicit styled
    /// runs retain their own descriptors. A nonpositive or nonfinite factor reads as 1.0.
    pub fn font_scale<M>(mut self, scale: impl IntoReactive<f64, M>) -> Self {
        self.font_scale = scale.into_reactive();
        self
    }
    /// Stop this label growing with the reader's text-size setting past `cap` times its
    /// default size: `1.3` lets a tab label or a badge grow 30% and no further, `1.0` opts
    /// it out of growth. Text smaller than the default is never affected. Use it only where
    /// the layout cannot give the text more room; body text should scale without a cap
    /// (docs/text.md "Text scale").
    pub fn max_text_scale(mut self, cap: f64) -> Self {
        self.max_text_scale = (cap.is_finite() && cap > 0.0).then_some(cap);
        self
    }
    /// Override the font weight (e.g. `FontWeight::Semibold`). See also [`Label::bold`].
    pub fn weight(mut self, w: day_spec::FontWeight) -> Self {
        self.weight = Some(w);
        self
    }
    /// Shorthand for `.weight(FontWeight::Bold)`.
    pub fn bold(self) -> Self {
        self.weight(day_spec::FontWeight::Bold)
    }
    /// De-emphasize the text: the platform's secondary label color, whatever that is here
    /// (`secondaryLabelColor` on Apple, `?android:textColorSecondary`, a dim label on GTK).
    ///
    /// Semantic rather than a literal grey, for the reason a literal grey cannot solve: one that
    /// reads well on white is wrong on black, so an app that hard-codes it gets a label that is
    /// either washed out or nearly invisible in the other appearance. This is what an empty
    /// state's "nothing selected", a hint under a field, or a caption should wear.
    pub fn secondary(mut self) -> Self {
        self.role = day_spec::props::TextRole::Secondary;
        self
    }
    /// Render the text italic (slanted).
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
    /// Ask for tabular (monospaced) figures, so a changing number stops changing width.
    ///
    /// Pair it with [`Decorate::reserving`] for a readout beside a slider: reserving stops the box
    /// resizing when the digit count changes, tabular stops the digits shifting inside it because
    /// `1` is narrower than `8`. See [`day_spec::FontSpec::tabular`].
    pub fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }
    /// Ask for the platform's monospaced face at this style's size, which is what inline code
    /// wants.
    pub fn monospace(mut self) -> Self {
        self.monospace = true;
        self
    }
    /// Draw lowercase letters as small capitals, for an eyebrow heading or an acronym in
    /// running text. A face without small capitals draws its ordinary lowercase. See
    /// [`day_spec::FontSpec::small_caps`].
    pub fn small_caps(mut self) -> Self {
        self.small_caps = true;
        self
    }
    /// Ask for oldstyle figures, digits that sit in running text like lowercase letters. A
    /// face without them draws its ordinary figures. Combines with [`Label::tabular`].
    pub fn oldstyle_figures(mut self) -> Self {
        self.oldstyle_figures = true;
        self
    }
    /// Underline the whole text. `Decoration::SINGLE` is the plain line; `Decoration::wavy()`,
    /// `double()` and `dotted()` draw the other styles, and `.color(c)` gives the line a
    /// color of its own: `.underline(Decoration::wavy().color(Color::hex(0xD0021B)))`. A
    /// `DecorationStyle` converts, so `.underline(DecorationStyle::Double)` works too. For
    /// part of the text, use [`TextBuilder::underline_styled`].
    pub fn underline(mut self, decoration: impl Into<day_spec::Decoration>) -> Self {
        self.underline = Some(decoration.into());
        self
    }
    /// Strike the whole text through, with the same styles and color as [`Label::underline`].
    pub fn strikethrough(mut self, decoration: impl Into<day_spec::Decoration>) -> Self {
        self.strikethrough = Some(decoration.into());
        self
    }
    /// Style spans within this label's text (docs/text-runs.md): one wrapping paragraph with
    /// emphasis, color, code or a link inside it, rather than several labels in a row.
    ///
    /// Ranges are byte offsets into the label's text, ascending and non-overlapping; text not
    /// covered by a run draws with the label's own font. Invalid runs are rejected at build
    /// time with a warning and the label renders plain, because the alternative is eight
    /// different wrong renderings (and a panic on the backends that slice `str`).
    ///
    /// [`TextBuilder`] is the ergonomic way in; this is the direct one.
    pub fn runs(mut self, runs: Vec<day_spec::TextRun>) -> Self {
        self.runs = runs;
        self
    }
    /// Take both the text and its runs from a [`TextBuilder`], replacing whatever text the label
    /// was built with. This is the intended entry point: the builder guarantees the ranges match
    /// the string, which is the invariant hand-written runs get wrong.
    pub fn runs_from(mut self, b: TextBuilder) -> Self {
        let (text, runs) = b.build();
        self.text = TextSource::Static(text);
        self.runs = runs;
        self
    }
    /// Center (or trail, or justify) this label's lines within its own width, for the short
    /// wrapped block a welcome screen or an empty state uses. Only observable on a label that
    /// wraps, since a single line already fills its box.
    pub fn align(mut self, align: day_spec::props::TextAlign) -> Self {
        self.align = align;
        self
    }
    /// Read the label's text as inline Markdown (docs/text-runs.md): `**bold**`, `*italic*`,
    /// `` `code` ``, `~~strike~~` and `[text](url)` become styled runs, and the markers themselves
    /// are stripped.
    ///
    /// The parse happens at run time, on every change, so it works on a translated string chosen
    /// from the locale bundle, a value off the network, or text a user is typing, none of which a
    /// compile-time macro can see. The cost is a parse per update of a string that is a label's
    /// worth of text.
    ///
    /// ```ignore
    /// label(res::str::release_note()).markdown()
    /// label(move || draft.get()).markdown()   // live as the user types
    /// ```
    ///
    /// Unrecognized markup stays literal, so a half-typed `**` reads as two asterisks rather than
    /// flickering. Block constructs (headings, lists, quotes, code blocks) are not parsed
    /// here: a label is one paragraph. [`markdown`](crate::markdown()) is the piece for a
    /// document.
    pub fn markdown(mut self) -> Self {
        self.markdown = true;
        self
    }
    /// Handle a tapped link run yourself instead of opening its target.
    ///
    /// Without this, a `#route` target navigates in-app and other targets open in the platform's
    /// default handler, the same as the [`link`] piece. This handler overrides both behaviors;
    /// call [`open_link`] to delegate targets you do not handle back to the default.
    ///
    /// Activation is `Cap::TextLinks`, which is narrower than run rendering: on a backend
    /// without it the link still draws, and nothing calls this (docs/text-runs.md).
    pub fn on_link(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_link = Some(Rc::new(f));
        self
    }
    /// Make the label selectable and hear about the reader's selection as it changes: the
    /// selected text and its byte range in the displayed text, empty when the selection is
    /// cleared. For a "Quote selection" button or a lookup panel beside the text.
    ///
    /// `capability(Cap::LabelSelection)` says whether this toolkit reports selections. Where
    /// it does not, the label is still selectable and the callback never runs.
    pub fn on_selection(mut self, f: impl Fn(&SelectedText) + 'static) -> Self {
        self.on_selection = Some(Rc::new(f));
        self
    }
    /// The text color: any ink (a [`Color`](day_spec::Color), a
    /// [`SystemColor`](day_spec::SystemColor), `Ink::adaptive(light, dark)`), or a signal or
    /// closure of one. An ink follows the appearance by itself; a reactive source recolors the
    /// native label when it changes.
    pub fn color<M>(mut self, c: impl IntoReactive<day_spec::Ink, M>) -> Self {
        self.color = Some(c.into_reactive());
        self
    }
    /// Apply a named [`TextPreset`](crate::TextPreset): the font, weight, scale, emphasis,
    /// spacing and color an app defined once (`label(t).preset(&CAPTION)`).
    pub fn preset(mut self, preset: &crate::TextPreset) -> Self {
        self.font = preset.font;
        self.font_scale = Reactive::Const(preset.font_scale);
        if preset.weight.is_some() {
            self.weight = preset.weight;
        }
        self.italic |= preset.italic;
        self.tabular |= preset.tabular;
        self.monospace |= preset.monospace;
        self.small_caps |= preset.small_caps;
        if preset.secondary {
            self.role = day_spec::props::TextRole::Secondary;
        }
        if preset.single_line {
            self.wraps = false;
        }
        if let Some(c) = preset.color {
            self.color = Some(Reactive::Const(c));
        }
        if let Some(ems) = preset.letter_spacing {
            self.letter_spacing = Reactive::Const(ems);
        }
        if let Some(multiple) = preset.line_height {
            self.line_height = Reactive::Const(multiple);
        }
        if preset.max_text_scale.is_some() {
            self.max_text_scale = preset.max_text_scale;
        }
        self
    }
}

/// What a label's build keeps for its bindings: the styles that cover the whole text, and the
/// text it currently shows.
struct LabelState {
    markdown: bool,
    font: Font,
    /// The label's resolved base font, which whole-text runs are drawn in.
    spec: std::cell::Cell<day_spec::FontSpec>,
    color: Option<day_spec::Ink>,
    underline: Option<day_spec::Decoration>,
    strikethrough: Option<day_spec::Decoration>,
    link: Option<String>,
    /// The source text last given, before Markdown markers were stripped.
    source: RefCell<String>,
    /// The text the label displays, which a selection's byte range addresses.
    displayed: RefCell<String>,
}

impl LabelState {
    /// Does any style cover the whole text, so that text always travels with runs?
    fn covers(&self) -> bool {
        self.link.is_some() || self.underline.is_some() || self.strikethrough.is_some()
    }

    /// The text to display for `source` and the runs over it. `fixed` are the label's own
    /// `.runs(…)`, which address the text it was built with.
    fn shape(&self, source: &str, fixed: &[day_spec::TextRun]) -> (String, Vec<day_spec::TextRun>) {
        let (text, runs) = if self.markdown {
            day_spec::markdown::parse(source, self.font)
        } else {
            (source.to_owned(), fixed.to_vec())
        };
        // Validate once here rather than in eight backends: an overlapping or mid-character
        // range renders differently wrong on each, and panics on the ones that slice `str`.
        let runs = match day_spec::runs_are_valid(&text, &runs) {
            Ok(()) => runs,
            Err(why) => {
                log::warn!("label runs ignored: {why}; the text renders unstyled");
                Vec::new()
            }
        };
        let runs = if self.covers() {
            self.cover(&text, runs)
        } else {
            runs
        };
        source.clone_into(&mut self.source.borrow_mut());
        text.clone_into(&mut self.displayed.borrow_mut());
        (text, runs)
    }

    /// Spread the whole-text styles over `text`: runs that exist take them where they say
    /// nothing of their own, and the gaps between runs become runs in the label's font.
    fn cover(&self, text: &str, runs: Vec<day_spec::TextRun>) -> Vec<day_spec::TextRun> {
        let fill = |range: std::ops::Range<usize>| day_spec::TextRun {
            range,
            font: self.spec.get(),
            color: self.color,
            underline: self.underline,
            strikethrough: self.strikethrough,
            link: self.link.clone(),
            ..day_spec::TextRun::default()
        };
        let mut out = Vec::with_capacity(runs.len() * 2 + 1);
        let mut at = 0usize;
        for mut run in runs {
            if run.range.start > at {
                out.push(fill(at..run.range.start));
            }
            at = run.range.end;
            run.underline = run.underline.or(self.underline);
            run.strikethrough = run.strikethrough.or(self.strikethrough);
            if run.link.is_none() {
                run.link.clone_from(&self.link);
            }
            out.push(run);
        }
        if at < text.len() {
            out.push(fill(at..text.len()));
        }
        out
    }
}

/// The scale a label's font takes: its own factor, held under its text-scale cap. Both reads
/// are tracked, so a binding around this follows a reactive factor and the system setting.
fn label_font_scale(scale: &Reactive<f64>, font: Font, cap: Option<f64>) -> f64 {
    let own = scale.get();
    let own = if own.is_finite() && own > 0.0 {
        own
    } else {
        1.0
    };
    match cap {
        Some(cap) => {
            let system = day_core::text_scale_of(font);
            if system > cap {
                own * cap / system
            } else {
                own
            }
        }
        None => own,
    }
}

impl Piece for Label {
    fn build(mut self, cx: &mut BuildCx) -> RNode {
        // A link on a toolkit without activatable link runs is accent text with a tap handler
        // and a button role, which every toolkit can do.
        if self.link.is_some()
            && day_core::capability(day_spec::Cap::TextLinks) != day_spec::Support::Native
            && let Some(url) = self.link.take()
        {
            if self.color.is_none() {
                self.color = Some(Reactive::Const(LINK_INK));
            }
            let handler = self.on_link.clone();
            return self
                .on_tap(move || match &handler {
                    Some(f) => f(&url),
                    None => open_link(&url),
                })
                .a11y(|b| b.role(Role::Button))
                .build(cx);
        }
        let scale = label_font_scale_untracked(&self.font_scale, self.font, self.max_text_scale);
        let spec = |scale: f64| day_spec::FontSpec {
            style: self.font,
            scale,
            weight: self.weight,
            italic: self.italic,
            tabular: self.tabular,
            monospace: self.monospace,
            small_caps: self.small_caps,
            oldstyle_figures: self.oldstyle_figures,
        };
        let color = self
            .color
            .as_ref()
            .map(super::canvas::Reactive::get_untracked);
        let state = Rc::new(LabelState {
            markdown: self.markdown,
            font: self.font,
            spec: std::cell::Cell::new(spec(scale)),
            color,
            underline: self.underline,
            strikethrough: self.strikethrough,
            link: self.link.clone(),
            source: RefCell::new(String::new()),
            displayed: RefCell::new(String::new()),
        });
        let (initial, runs) = state.shape(&self.text.initial(), &self.runs);
        let paragraph = {
            let (max_lines, truncation) = (self.max_lines.clone(), self.truncation.clone());
            let (letter_spacing, line_height) =
                (self.letter_spacing.clone(), self.line_height.clone());
            move || LabelParagraph {
                max_lines: max_lines.get(),
                truncation: truncation.get(),
                letter_spacing: letter_spacing.get(),
                line_height: line_height.get(),
            }
        };
        let seed = day_reactive::untrack(&paragraph);
        let node = cx.leaf(
            kinds::LABEL,
            &LabelProps {
                align: self.align,
                text: initial,
                font: state.spec.get(),
                color,
                role: self.role,
                wraps: self.wraps,
                max_lines: seed.max_lines,
                truncation: seed.truncation,
                letter_spacing: seed.letter_spacing,
                line_height: seed.line_height,
                reports_selection: self.on_selection.is_some(),
                runs,
            },
            Flex::default(),
        );
        // A label that can carry link runs listens for their activation. Markdown labels qualify
        // whatever they currently hold, since a later parse may produce a link that this one did
        // not. Labels without any prospect of a link register nothing.
        let could_link = self.markdown
            || self.link.is_some()
            || self.on_link.is_some()
            || self.runs.iter().any(|r| r.link.is_some());
        if could_link {
            let handler = self.on_link.clone();
            let whole = self.link.clone();
            cx.on(node, move |ev| match ev {
                Event::LinkActivated(url) => match &handler {
                    Some(f) => f(url),
                    None => open_link(url),
                },
                // A synthesized tap on a whole-label link follows the same policy; the native
                // activation arrives as `LinkActivated`.
                Event::Tap(_) => {
                    if let Some(url) = &whole {
                        match &handler {
                            Some(f) => f(url),
                            None => open_link(url),
                        }
                    }
                }
                _ => {}
            });
        }
        // Reporting a selection needs a selection: the label becomes selectable, and the byte
        // range the toolkit reports is read against the text on display.
        if let Some(handler) = self.on_selection.clone() {
            with_tree(|t| t.set_node_selectable(node, true));
            let state = state.clone();
            cx.on(node, move |ev| {
                if let Event::TextSelected { start, end } = ev {
                    let shown = state.displayed.borrow();
                    let range = (*start).min(shown.len())..(*end).min(shown.len());
                    let text = shown.get(range.clone()).unwrap_or_default().to_owned();
                    drop(shown);
                    let range = if text.is_empty() { 0..0 } else { range };
                    handler(&SelectedText { text, range });
                }
            });
        }
        // Text and runs travel together whenever the label has any: a range only means
        // something against the string it was computed from.
        let styled = self.markdown || state.covers();
        {
            let state = state.clone();
            self.text.bind_to(
                node,
                move |t| {
                    let (text, runs) = state.shape(&t, &[]);
                    if styled {
                        Box::new(LabelPatch::Runs(text, runs))
                    } else {
                        Box::new(LabelPatch::Text(text))
                    }
                },
                true,
            );
        }
        // The font follows a reactive scale and, under a cap, the system text size.
        if matches!(self.font_scale, Reactive::Dyn(_)) || self.max_text_scale.is_some() {
            let (font_scale, font, cap) = (self.font_scale.clone(), self.font, self.max_text_scale);
            let state = state.clone();
            // Seeded with the scale the label was realized at, so only a change patches.
            bind_seeded(
                scale,
                move || label_font_scale(&font_scale, font, cap),
                move |scale: &f64| {
                    let mut next = state.spec.get();
                    next.scale = *scale;
                    state.spec.set(next);
                    with_tree(|t| t.patch(node, Box::new(LabelPatch::Font(next)), true));
                    // Whole-text runs are drawn in the label's font, so they follow it.
                    if state.covers() {
                        let source = state.source.borrow().clone();
                        let (text, runs) = state.shape(&source, &[]);
                        with_tree(|t| t.patch(node, Box::new(LabelPatch::Runs(text, runs)), true));
                    }
                },
            );
        }
        // The paragraph attributes follow their sources as one patch.
        let reactive_paragraph = matches!(self.max_lines, Reactive::Dyn(_))
            || matches!(self.truncation, Reactive::Dyn(_))
            || matches!(self.letter_spacing, Reactive::Dyn(_))
            || matches!(self.line_height, Reactive::Dyn(_));
        if reactive_paragraph {
            bind_seeded(seed, paragraph, move |p: &LabelParagraph| {
                with_tree(|t| t.patch(node, Box::new(LabelPatch::Paragraph(*p)), true));
            });
        }
        // A reactive color recolors in place; a constant was applied once at realize.
        if let Some(Reactive::Dyn(f)) = self.color {
            bind(
                move || f(),
                move |c: &day_spec::Ink| {
                    with_tree(|t| t.patch(node, Box::new(LabelPatch::Color(Some(*c))), false));
                },
            );
        }
        node
    }
}

/// [`label_font_scale`] without subscribing, for the value a label is realized with.
fn label_font_scale_untracked(scale: &Reactive<f64>, font: Font, cap: Option<f64>) -> f64 {
    day_reactive::untrack(|| label_font_scale(scale, font, cap))
}

/// The platform's link color a [`link`] takes on a toolkit with no native link runs. Override
/// per link with [`Label::color`] to match an app's accent.
const LINK_INK: day_spec::Ink = day_spec::Ink::system(day_spec::SystemColor::Link);

/// Activate a link target, using the same policy as [`link`] and markdown labels.
///
/// A leading `#` denotes an in-app route: `#settings` calls [`navigate`](crate::navigate) with
/// `settings`. Route paths, percent escapes, and query parameters use the navigation system's
/// existing rules; `#` alone navigates to the empty route (pop to root). An unknown route is
/// ignored, never handed to an external application. All other targets go to
/// [`day_core::open_url`] unchanged, including URLs with fragments such as `https://daybrite.dev/#docs`.
///
/// Use this from [`Label::on_link`] to keep the default behavior for targets your handler does
/// not override.
pub fn open_link(target: &str) {
    if let Some(route) = target.strip_prefix('#') {
        let _ = day_core::navigate(route);
    } else {
        day_core::open_url(target);
    }
}

/// A label whose whole text is a link: it navigates to a `#route`, or opens a URL in the
/// platform's default handler (the system browser for `http`/`https`, the mail client for
/// `mailto:`, and so on).
///
/// The result is a [`Label`], so every label builder applies: `.font`, `.bold`, `.color`
/// (reactive), `.max_lines`, `.truncation`, `.underline`, `.on_link` to handle the tap
/// yourself. Where the toolkit has activatable link runs (`Cap::TextLinks`) the text is a
/// native link run, with the platform's link color, hover and pressed states, keyboard focus
/// and accessibility. Elsewhere it is accent-colored text with a tap handler and a button
/// role.
///
/// ```ignore
/// link(res::str::project_site(), "https://daybrite.dev")
/// link(res::str::settings(), "#settings").bold()
/// link(res::str::email_us(), "mailto:hi@example.com").font(Font::Footnote)
/// ```
pub fn link<M>(text: impl IntoText<M>, url: impl Into<String>) -> Label {
    let mut label = label(text);
    label.link = Some(url.into());
    label
}

#[must_use]
pub struct Button {
    title: TextSource,
    icon: Reactive<Option<day_spec::Icon>>,
    icon_only: bool,
    action: Option<Rc<dyn Fn()>>,
    native_style: day_spec::props::ButtonStyleSpec,
    /// A reactive tint, kept apart from `native_style` so the color can follow a signal. Set by
    /// [`Button::tint`]; it wins over `bordered`/`prominent` because it is the more specific ask.
    tint: Option<Reactive<day_spec::Ink>>,
    enabled: Reactive<bool>,
    selected: Reactive<bool>,
}

pub fn button<M>(title: impl IntoText<M>) -> Button {
    Button {
        title: title.into_text(),
        icon: Reactive::Const(None),
        icon_only: false,
        action: None,
        native_style: day_spec::props::ButtonStyleSpec::Automatic,
        tint: None,
        enabled: true.into_reactive(),
        selected: false.into_reactive(),
    }
}

#[day_macros::builder(ButtonBuilder)]
impl Button {
    /// Show a platform symbol. A signal or closure updates it without replacing the button.
    pub fn icon<M>(mut self, symbol: impl IntoReactive<day_spec::Symbol, M>) -> Self {
        let symbol = symbol.into_reactive();
        self.icon = match symbol {
            Reactive::Const(s) => Reactive::Const(Some(day_spec::Icon::Symbol(s))),
            Reactive::Dyn(f) => Reactive::Dyn(Rc::new(move || Some(day_spec::Icon::Symbol(f())))),
        };
        self
    }

    /// Show a bundled image or vector alongside the label.
    pub fn image(mut self, name: impl Into<day_spec::ImageName>) -> Self {
        self.icon = Reactive::Const(Some(day_spec::Icon::Image(name.into().as_str().to_owned())));
        self
    }

    /// Hide the visible label when an icon is available; retain it for accessibility and help.
    pub fn icon_only(mut self) -> Self {
        self.icon_only = true;
        self
    }

    /// Handle native button activation by pointer, keyboard, or accessibility action.
    /// Disabled buttons do not invoke this callback.
    ///
    /// Use this instead of [`Decorate::on_tap`]. A tap decorator adds a separate gesture
    /// recognizer that can delay pressed feedback and bypass native control behavior.
    /// Day logs a warning when a tap gesture is attached to a native button.
    pub fn action(mut self, f: impl Fn() + 'static) -> Self {
        self.action = Some(Rc::new(f));
        self
    }

    /// Ask for a visually CONTAINED native button on toolkits whose stock look is borderless
    /// (iOS's plain system button reads as a link); a no-op where buttons are already bordered.
    pub fn bordered(mut self) -> Self {
        self.native_style = day_spec::props::ButtonStyleSpec::Bordered;
        self
    }

    /// Whether the button is interactive (default `true`; `false` = disabled/grayed by the native
    /// control). Reactive, so it can follow app state, e.g. `.enabled(move || !busy.get())` to
    /// lock a control while a long operation runs.
    ///
    /// This drives the platform's own disabled rendering through `ButtonPatch::Enabled`; it is not
    /// a painted imitation, and a disabled button stops delivering `Event::Pressed` at the source.
    pub fn enabled<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.enabled = v.into_reactive();
        self
    }

    /// The platform's accent-filled / default-action button (iOS bordered-prominent, macOS
    /// return-key blue, GTK suggested-action, XAML accent style). Use for the one primary
    /// action of a view.
    pub fn prominent(mut self) -> Self {
        self.native_style = day_spec::props::ButtonStyleSpec::Prominent;
        self
    }

    /// A filled button in a color of your choosing, still drawn by the native control.
    ///
    /// The platform keeps everything that makes a button a button: its pressed and hover
    /// rendering, its focus ring, its disabled look, its accessibility role, and keyboard
    /// activation. Only the fill is yours. The label color is chosen for contrast against the
    /// fill, so a pale tint gets dark text and a saturated one white.
    ///
    /// Reactive, so the color can follow app state: `.tint(move || if recording { RUST } else
    /// { SKY })` recolors in place rather than rebuilding the button.
    ///
    /// A backend that cannot recolor its button ignores the tint and draws its ordinary button
    /// (docs/buttons.md), because a plain button on one platform is a far smaller loss than a
    /// colored rectangle that is no longer a button.
    pub fn tint<M>(mut self, color: impl IntoReactive<day_spec::Ink, M>) -> Self {
        self.tint = Some(color.into_reactive());
        self
    }

    /// A button no wider than its title: a stepper's "−" and "+", a chip's "×". Drops the
    /// minimum width and wide insets of toolkits that have them (Material); a no-op where the
    /// stock button already hugs its title. A [`plain`](Self::plain) button stays plain: it
    /// already hugs its title everywhere, and Material's compact button is a filled one.
    pub fn compact(mut self) -> Self {
        if self.native_style != day_spec::props::ButtonStyleSpec::Plain {
            self.native_style = day_spec::props::ButtonStyleSpec::Compact;
        }
        self
    }

    /// Borderless native action button for toolbars and list-row accessories.
    /// Preserves native interaction, focus, disabled rendering, and accessibility.
    pub fn plain(mut self) -> Self {
        self.native_style = day_spec::props::ButtonStyleSpec::Plain;
        self
    }

    /// Mark this button as the current choice among its peers: a document tab, one view of a
    /// switcher. The platform draws its own checked or selected look (an `NSButton`'s on state,
    /// `UIButton.isSelected`, GTK's `:checked`, a checked `QPushButton`, a checked
    /// `MaterialButton`, `aria-pressed` on the web) and announces it as selected, while the
    /// button stays enabled, focusable and activatable, which a disabled stand-in is not.
    ///
    /// Reactive, so it follows the selection: `.selected(move || current.get() == key)`. A press
    /// never toggles the state natively; the app's handler decides, and the next value of the
    /// closure is what the control shows.
    pub fn selected<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.selected = v.into_reactive();
        self
    }
}

impl Piece for Button {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.title.initial();
        // A tint is the most specific style ask, so it wins over bordered/prominent.
        let style = match &self.tint {
            Some(c) => day_spec::props::ButtonStyleSpec::Tinted(c.get_untracked()),
            None => self.native_style,
        };
        let node = cx.leaf(
            kinds::BUTTON,
            &ButtonProps {
                title: initial.clone(),
                icon: self.icon.get_untracked(),
                icon_only: self.icon_only,
                enabled: self.enabled.get_untracked(),
                style,
                selected: self.selected.get_untracked(),
            },
            Flex::default(),
        );
        // A reactive `selected` patches on change, like `enabled` below.
        if let selected @ Reactive::Dyn(_) = self.selected {
            bind(
                move || selected.get(),
                move |on: &bool| {
                    with_tree(|t| t.patch(node, Box::new(ButtonPatch::Selected(*on)), false));
                },
            );
        }
        // A reactive tint recolors in place; a constant one was applied at realize above.
        if let Some(c @ Reactive::Dyn(_)) = self.tint.clone() {
            bind(
                move || c.get(),
                move |col: &day_spec::Ink| {
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(ButtonPatch::Style(
                                day_spec::props::ButtonStyleSpec::Tinted(*col),
                            )),
                            false,
                        );
                    });
                },
            );
        }
        // A reactive `enabled` patches on change; a constant is applied once at realize, the same
        // shape `Toggle` uses.
        let enabled = self.enabled;
        let enabled_gate = enabled.clone();
        if let Reactive::Dyn(_) = &enabled {
            bind(
                move || enabled.get(),
                move |e: &bool| {
                    with_tree(|t| t.patch(node, Box::new(ButtonPatch::Enabled(*e)), false));
                },
            );
        }
        if let Some(action) = self.action {
            // Gate the action on `enabled` as well as telling the native control. A real touch on a
            // disabled UIButton/MaterialButton never produces `Pressed`, so this is belt-and-braces
            // for users, but an event delivered by another route (a dayscript `tap`, which
            // dispatches to the node rather than simulating a touch) would otherwise fire an action
            // the user cannot reach. `.enabled(false)` should mean "cannot fire", not "looks gray".
            let gate = enabled_gate;
            cx.on(node, move |ev| {
                if matches!(ev, Event::Pressed) && gate.get() {
                    action();
                }
            });
        }
        if matches!(&self.icon, Reactive::Const(None)) {
            // Preserve the existing patch contract for plain buttons and external toolkits.
            self.title
                .bind_to(node, |t| Box::new(ButtonPatch::Title(t)), true);
        } else {
            let icon = self.icon;
            let icon_only = self.icon_only;
            let seed = day_spec::props::ButtonContent {
                title: initial,
                icon: icon.get_untracked(),
                icon_only,
            };
            let title = self.title;
            bind_seeded(
                seed,
                move || day_spec::props::ButtonContent {
                    title: title.resolve(),
                    icon: icon.get(),
                    icon_only,
                },
                move |content: &day_spec::props::ButtonContent| {
                    with_tree(|t| {
                        t.patch(node, Box::new(ButtonPatch::Content(content.clone())), true);
                    });
                },
            );
        }
        node
    }
}

#[must_use]
pub struct Toggle<S: Binding<bool>> {
    value: S,
    enabled: Reactive<bool>,
}

pub fn toggle<S: Binding<bool>>(value: S) -> Toggle<S> {
    Toggle {
        value,
        enabled: true.into_reactive(),
    }
}

#[day_macros::builder(ToggleBuilder)]
impl<S: Binding<bool>> Toggle<S> {
    /// Whether the toggle is interactive (default `true`; `false` = disabled/grayed). Reactive,
    /// e.g. `.enabled(capability(Cap::TextSpellCheck) == Support::Native)` to gray it out where a
    /// backend can't honor the thing it controls.
    pub fn enabled<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.enabled = v.into_reactive();
        self
    }
}

impl<S: Binding<bool>> Piece for Toggle<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.value.peek();
        let node = cx.leaf(
            kinds::TOGGLE,
            &ToggleProps {
                on: initial,
                enabled: self.enabled.get_untracked(),
            },
            Flex::default(),
        );
        let v = self.value.clone();
        bind_seeded(
            initial,
            move || v.read(),
            move |on: &bool| {
                with_tree(|t| t.patch(node, Box::new(TogglePatch::On(*on)), false));
            },
        );
        // A reactive `enabled` patches on change; a constant is applied once at realize.
        let enabled = self.enabled;
        if let Reactive::Dyn(_) = &enabled {
            bind(
                move || enabled.get(),
                move |e: &bool| {
                    with_tree(|t| t.patch(node, Box::new(TogglePatch::Enabled(*e)), false));
                },
            );
        }
        let v = self.value;
        cx.on(node, move |ev| {
            // A report of the value the binding already holds changes nothing: it is the
            // control echoing the app's own write (GTK's `notify::active`, Qt's `toggled` and
            // Android's checked listener all fire for a programmatic set), the second layer of
            // echo suppression (DESIGN.md §4.4). Written anyway, it would notify every watcher
            // of the binding a second time for one change.
            if let Event::ToggleChanged(on) = ev
                && v.peek() != *on
            {
                v.write(*on);
            }
        });
        node
    }
}

#[must_use]
pub struct Slider<S: Binding<f64>> {
    value: S,
    min: f64,
    max: f64,
    step: Option<f64>,
    enabled: Reactive<bool>,
}

pub fn slider<S: Binding<f64>>(value: S) -> Slider<S> {
    Slider {
        value,
        min: 0.0,
        max: 1.0,
        step: None,
        enabled: Reactive::Const(true),
    }
}

#[day_macros::builder(SliderBuilder)]
impl<S: Binding<f64>> Slider<S> {
    /// Whether the slider takes input (default `true`); a constant or a reactive `bool`.
    pub fn enabled<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.enabled = v.into_reactive();
        self
    }
    pub fn range(mut self, r: std::ops::RangeInclusive<f64>) -> Self {
        self.min = *r.start();
        self.max = *r.end();
        self
    }
    pub fn step(mut self, s: f64) -> Self {
        self.step = Some(s);
        self
    }
}

impl<S: Binding<f64>> Piece for Slider<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.value.peek();
        let node = cx.leaf(
            kinds::SLIDER,
            &SliderProps {
                value: initial,
                min: self.min,
                max: self.max,
                step: self.step,
                enabled: self.enabled.get_untracked(),
            },
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
        let enabled = self.enabled;
        if let Reactive::Dyn(_) = &enabled {
            bind(
                move || enabled.get(),
                move |e: &bool| {
                    with_tree(|t| t.patch(node, Box::new(SliderPatch::Enabled(*e)), false));
                },
            );
        }
        let v = self.value.clone();
        bind_seeded(
            initial,
            move || v.read(),
            move |val: &f64| {
                with_tree(|t| t.patch(node, Box::new(SliderPatch::Value(*val)), false));
            },
        );
        let v = self.value;
        let (step, min, max) = (self.step, self.min, self.max);
        cx.on(node, move |ev| {
            // Honor `.step(_)` at the framework layer so every backend produces stepped values:
            // several native sliders (e.g. iOS `UISlider`) have no native step and emit a
            // continuous stream while dragging. Snapping here keeps the bound signal (and the
            // thumb, via `bind_seeded` above) on the step grid, and stops a `.step`-bound consumer
            // from being hammered ~60×/s with sub-step deltas during a drag.
            let snap = |val: f64| match step {
                Some(s) if s > 0.0 => (min + ((val - min) / s).round() * s).clamp(min, max),
                _ => val,
            };
            match ev {
                // The live half of the pair: readers follow the thumb; nothing durable keys
                // off it (a day-model field opens a preview session here).
                Event::ValueChanged(val) => v.write_preview(snap(*val)),
                // The settled value: One record for the whole drag. A backend that cannot
                // tell the two apart never sends this, and the preview default (a plain
                // write) keeps it correct: chattier, never wrong.
                Event::ValueCommitted(val) => v.write_commit(snap(*val)),
                _ => {}
            }
        });
        node
    }
}

/// A single-line text input bound to a `String` (docs/textfield.md).
///
/// How it takes its text is set with `.secure(_)`, `.read_only(_)`, `.input_purpose(_)`,
/// `.submit_label(_)` and `.max_length(_)`; [`secure_field`] is the password-field shorthand.
#[must_use]
pub struct TextField<S: Binding<String>> {
    value: S,
    placeholder: Option<TextSource>,
    on_submit: Option<Rc<dyn Fn()>>,
    secure: Reactive<bool>,
    read_only: Reactive<bool>,
    purpose: day_spec::InputPurpose,
    submit_label: day_spec::SubmitLabel,
    max_length: Option<u32>,
    enabled: Reactive<bool>,
    select: Option<(day_reactive::Trigger, Rc<dyn Fn() -> TextSelection>)>,
}

pub fn text_field<S: Binding<String>>(value: S) -> TextField<S> {
    TextField {
        value,
        placeholder: None,
        on_submit: None,
        secure: Reactive::Const(false),
        read_only: Reactive::Const(false),
        purpose: day_spec::InputPurpose::Text,
        submit_label: day_spec::SubmitLabel::Return,
        max_length: None,
        enabled: Reactive::Const(true),
        select: None,
    }
}

/// A password field: a [`text_field`] that hides its characters and tells the platform it
/// holds a password, so the password manager offers to fill it (docs/textfield.md).
///
/// It is the same piece, so every `text_field` builder applies. `.secure(shown)` with a signal
/// makes a show-password switch; `.input_purpose(InputPurpose::NewPassword)` marks a sign-up
/// form's field.
pub fn secure_field<S: Binding<String>>(value: S) -> TextField<S> {
    text_field(value)
        .secure(true)
        .input_purpose(day_spec::InputPurpose::Password)
}

#[day_macros::builder(TextFieldBuilder)]
impl<S: Binding<String>> TextField<S> {
    /// Whether the field takes input (default `true`); a constant or a reactive `bool`. A
    /// disabled field neither edits nor focuses; [`TextField::read_only`] keeps it selectable.
    pub fn enabled<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.enabled = v.into_reactive();
        self
    }
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }
    /// Fire when the user submits the field (Return / the keyboard's action key). Field
    /// chaining is a focus write inside the handler: `focus.set(Some(Field::Next))`
    /// (docs/focus.md).
    pub fn on_submit(mut self, f: impl Fn() + 'static) -> Self {
        self.on_submit = Some(Rc::new(f));
        self
    }
    /// Hide the characters as they are typed (default `false`). Reactive: bind it to a signal
    /// for a show-password switch. The text, the placeholder and the focus carry across the
    /// change on every backend, including the two whose secure field is a different native
    /// class.
    pub fn secure<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.secure = v.into_reactive();
        self
    }
    /// Show the text and let it be selected and copied, and take no edits (default `false`).
    /// Reactive. Unlike `.disabled(true)` the field keeps its ordinary look and still takes
    /// focus.
    pub fn read_only<M>(mut self, v: impl IntoReactive<bool, M>) -> Self {
        self.read_only = v.into_reactive();
        self
    }
    /// What the field collects: the on-screen keyboard, capitalization and correction, and
    /// what the system offers to fill in all follow from it.
    pub fn input_purpose(mut self, purpose: day_spec::InputPurpose) -> Self {
        self.purpose = purpose;
        self
    }
    /// What the on-screen keyboard's action key says. The key fires `on_submit` either way.
    pub fn submit_label(mut self, label: day_spec::SubmitLabel) -> Self {
        self.submit_label = label;
        self
    }
    /// The most characters the field takes. Typing or pasting past it is cut to fit before
    /// the bound value sees it.
    pub fn max_length(mut self, characters: u32) -> Self {
        self.max_length = Some(characters);
        self
    }
    /// Select part of the text, or place the caret, each time `trigger` fires
    /// (docs/textfield.md "Selection"). `range` is read once the turn settles, after any text
    /// the same turn wrote has reached the field, and counts characters (Unicode scalar
    /// values): `..` selects everything, `3..` from the fourth character to the end, `3..3`
    /// puts the caret after the third. Ends past the text are held to it.
    ///
    /// An address bar's inline completion writes the completed text and selects what the user
    /// did not type, so the next keystroke replaces it:
    ///
    /// ```ignore
    /// let complete = Trigger::new();
    /// let typed = Signal::new(0usize);
    /// text_field(address).select(complete, move || typed.get_untracked()..)
    /// // on input "git": typed.set(3); address.set("github.com".into()); complete.notify();
    /// ```
    ///
    /// The selection is for a focused field; fire it from the same turn that focuses the field,
    /// or from a [`Decorate::focused`] change, to select everything on focus.
    pub fn select<R: std::ops::RangeBounds<usize> + 'static>(
        mut self,
        trigger: day_reactive::Trigger,
        range: impl Fn() -> R + 'static,
    ) -> Self {
        self.select = Some((
            trigger,
            Rc::new(move || {
                use std::ops::Bound;
                let r = range();
                let start = match r.start_bound() {
                    Bound::Included(&n) => n,
                    Bound::Excluded(&n) => n.saturating_add(1),
                    Bound::Unbounded => 0,
                };
                let end = match r.end_bound() {
                    Bound::Included(&n) => n.saturating_add(1),
                    Bound::Excluded(&n) => n,
                    Bound::Unbounded => usize::MAX,
                };
                TextSelection::new(start, end)
            }),
        ));
        self
    }
}

impl<S: Binding<String>> Piece for TextField<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.value.peek();
        let ph = self
            .placeholder
            .as_ref()
            .map_or_default(super::sources::TextSource::initial);
        let node = cx.leaf(
            kinds::TEXT_FIELD,
            &TextFieldProps {
                text: initial.clone(),
                placeholder: ph,
                enabled: self.enabled.get_untracked(),
            },
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
        if let Reactive::Dyn(_) = &self.enabled {
            let enabled = self.enabled.clone();
            bind(
                move || enabled.get(),
                move |e: &bool| {
                    with_tree(|t| t.patch(node, Box::new(TextFieldPatch::Enabled(*e)), false));
                },
            );
        }
        // Text entry traits (docs/textfield.md). The duty is skipped for a field that asks for
        // nothing, so a plain field costs what it did; a reactive member re-sends the whole
        // set, which is what lets a backend rebuild the widget as its secure class and dress
        // the replacement in one call.
        let (secure, read_only) = (self.secure, self.read_only);
        let (purpose, submit_label, max_length) =
            (self.purpose, self.submit_label, self.max_length);
        let traits = move |secure: bool, read_only: bool| day_spec::InputTraits {
            secure,
            read_only,
            purpose,
            submit_label,
            max_length,
        };
        let initial_traits = traits(secure.get_untracked(), read_only.get_untracked());
        let reactive_traits =
            matches!(secure, Reactive::Dyn(_)) || matches!(read_only, Reactive::Dyn(_));
        if initial_traits != day_spec::InputTraits::default() {
            with_tree(|t| t.set_node_input_traits(node, &initial_traits));
        }
        if reactive_traits {
            bind_seeded(
                initial_traits,
                move || traits(secure.get(), read_only.get()),
                move |tr: &day_spec::InputTraits| {
                    with_tree(|t| t.set_node_input_traits(node, tr));
                },
            );
        }
        // Controlled input with origin-tagged writes (§4.4): the echo guard remembers the
        // last value that came from the native widget so its own change is not written back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        // The typing session's latest text, sealed on Return or focus loss (below).
        let last: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let v = self.value.clone();
        let session = self.value.clone();
        let (g, typed) = (guard.clone(), last.clone());
        bind_seeded(
            initial,
            move || v.read(),
            move |t: &String| {
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                if !from_native && typed.borrow_mut().take().is_some() {
                    // An app write (an address bar's completion, Escape putting the URL back)
                    // replaces the typing session: sealing it later would write the typed
                    // text back over this one, and the binding's open preview session still
                    // holds the text from before the typing. The app's write recorded its own
                    // change; close the session so the next keystroke starts from it.
                    session.end_preview();
                }
                with_tree(|tr| {
                    tr.patch(
                        node,
                        Box::new(TextFieldPatch::Text {
                            text: t.clone(),
                            from_native,
                        }),
                        false,
                    );
                });
            },
        );
        let value = self.value.clone();
        let v = self.value;
        let submit = self.on_submit;
        // Typing is a session: each keystroke is a preview (readers follow, nothing durable
        // fires), sealed into one committed change on Return or focus loss by the typing
        // coalescer. For a plain Signal binding preview defaults to write, so nothing changes
        // where no session semantics exist. Teardown seals too: navigating away from a page
        // mid-type must not leave the last burst outside the change log.
        {
            let (v, last) = (v.clone(), last.clone());
            day_reactive::Scope::current().on_cleanup(move || {
                if let Some(t) = last.borrow_mut().take() {
                    v.write_teardown(t);
                }
            });
        }
        cx.on(node, move |ev| match ev {
            Event::TextChanged(t) => {
                // The length bound, held here so it counts the same characters on every
                // backend, whatever unit a native limit would count. The cut text is painted back as
                // an app write: the native field is showing the longer one, and the bound
                // value may not change at all (typing at the limit), so no binding would.
                let cut = max_length
                    .and_then(|max| t.char_indices().nth(max as usize))
                    .map(|(at, _)| t[..at].to_string());
                let t = match cut {
                    Some(cut) => {
                        with_tree(|tr| {
                            tr.patch(
                                node,
                                Box::new(TextFieldPatch::Text {
                                    text: cut.clone(),
                                    from_native: false,
                                }),
                                false,
                            );
                        });
                        cut
                    }
                    None => t.clone(),
                };
                *guard.borrow_mut() = Some(t.clone());
                *last.borrow_mut() = Some(t.clone());
                v.write_preview(t);
            }
            Event::Submitted => {
                if let Some(t) = last.borrow_mut().take() {
                    v.write_commit(t);
                }
                if let Some(f) = &submit {
                    f();
                }
            }
            Event::FocusChanged(false) => {
                if let Some(t) = last.borrow_mut().take() {
                    v.write_commit(t);
                }
            }
            _ => {}
        });
        if let Some(p) = self.placeholder {
            p.bind_to(node, |t| Box::new(TextFieldPatch::Placeholder(t)), false);
        }
        // Selection requests (docs/textfield.md "Selection"). Each firing is a request, so the
        // same range applies again on the next one (select-all on every focus). The range is
        // read once the turn settles, so the text written in the same turn has reached the
        // field first, whatever order the app wrote the two in, and it is held to that text.
        // The patch itself is posted, as a focus request is (`Decorate::focused`): one made in
        // the same turn lands first, and AppKit and UIKit select only in a focused field.
        if let Some((trigger, range)) = self.select {
            let scope = day_reactive::Scope::current();
            day_reactive::watch(
                move || trigger.track(),
                move |(), _| {
                    let (range, value) = (range.clone(), value.clone());
                    day_reactive::at_turn_end(move || {
                        if !scope.is_alive() {
                            return;
                        }
                        let sel = range().clamped(&value.peek());
                        day_reactive::on_main(move || {
                            with_tree(|t| {
                                t.patch(node, Box::new(TextFieldPatch::Select(sel)), false);
                            });
                        });
                    });
                },
            );
        }
        node
    }
}

/// A progress indicator: a determinate bar (from [`progress`]) or an indeterminate spinner
/// (from [`spinner`]). See docs/progress.md.
#[must_use]
pub struct Progress {
    /// `None` = indeterminate (spinner); `Some` = a determinate fraction source.
    value: Option<FractionSource>,
    animating: crate::Reactive<bool>,
    large: bool,
}

/// An indeterminate, animated progress indicator (a spinner / busy bar) for work with no
/// known extent.
pub fn spinner() -> Progress {
    Progress {
        value: None,
        animating: crate::Reactive::Const(true),
        large: false,
    }
}

/// A determinate progress bar. `fraction` is the completed portion in `0.0..=1.0`; pass a
/// constant, a `Signal<f64>`, or a closure and it tracks reactively (out-of-range values are
/// clamped).
pub fn progress<M>(fraction: impl IntoFraction<M>) -> Progress {
    Progress {
        value: Some(fraction.into_fraction()),
        animating: crate::Reactive::Const(true),
        large: false,
    }
}

#[day_macros::builder(ProgressBuilder)]
impl Progress {
    /// Start or stop an indeterminate indicator, keeping its layout space.
    pub fn animating<M>(mut self, source: impl crate::IntoReactive<bool, M>) -> Self {
        self.animating = source.into_reactive();
        self
    }
    /// Use the platform's large indeterminate indicator.
    pub fn large(mut self) -> Self {
        self.large = true;
        self
    }
}

impl Piece for Progress {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let determinate = self.value.is_some();
        let initial = self
            .value
            .as_ref()
            .map(super::sources::FractionSource::initial);
        let node = cx.leaf(
            kinds::PROGRESS,
            &ProgressProps {
                value: initial,
                animating: self.animating.get_untracked(),
                large: self.large,
            },
            // A determinate bar fills the available width (like a slider); a spinner keeps its
            // fixed intrinsic size.
            Flex {
                grow_w: determinate,
                ..Default::default()
            },
        );
        if !determinate && let crate::Reactive::Dyn(f) = self.animating {
            day_reactive::bind_seeded(
                f(),
                move || f(),
                move |on| {
                    day_core::with_tree(|t| {
                        t.patch(node, Box::new(ProgressPatch::Animating(*on)), false);
                    });
                },
            );
        }
        if let Some(src) = self.value {
            src.bind_to(node);
        }
        node
    }
}

/// A hairline between neighbors: across a column's width by default, down a row's height
/// with [`Divider::vertical`].
#[must_use]
pub struct Divider {
    vertical: bool,
}

pub fn divider() -> Divider {
    Divider { vertical: false }
}

#[day_macros::builder(DividerBuilder)]
impl Divider {
    /// Turn the rule on its side: a thin vertical line that fills the row's height, the way the
    /// horizontal one fills a column's width. Between a row's panes, where a horizontal rule
    /// would draw a stray line across one of them.
    pub fn vertical(mut self) -> Self {
        self.vertical = true;
        self
    }
}

impl Piece for Divider {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let props = DividerProps {
            vertical: self.vertical,
        };
        if self.vertical {
            cx.native(
                kinds::DIVIDER,
                &props,
                Rc::new(VerticalLeaf),
                Flex {
                    grow_h: true,
                    ..Default::default()
                },
                Boundary::No,
            )
        } else {
            cx.leaf(
                kinds::DIVIDER,
                &props,
                Flex {
                    grow_w: true,
                    ..Default::default()
                },
            )
        }
    }
}

/// A native leaf measured with its axes swapped: the toolkit answers as it does for a
/// horizontal rule (the offered width, by its hairline thickness), and the vertical rule is that
/// answer turned on its side. One measurement arm per toolkit, whichever way the line runs.
struct VerticalLeaf;

impl Layout for VerticalLeaf {
    fn measure(&self, cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        let s = cx.measure_leaf(Proposal::new(p.height, p.width));
        Size::new(s.height, s.width)
    }
    fn place(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], _bounds: Rect) {}
}

pub struct Spacer;

#[must_use]
pub fn spacer() -> Spacer {
    Spacer
}

impl Piece for Spacer {
    fn build(self, cx: &mut BuildCx) -> RNode {
        cx.layout_only(
            Rc::new(PassThrough),
            Flex {
                is_spacer: true,
                ..Default::default()
            },
            Boundary::No,
        )
    }
}

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md): each leaf's behavior on the real toolkit
// ---------------------------------------------------------------------------

/// The leaves' `#[day::test]` cases, next to the constructors they prove. Compiled only under
/// the `conformance` feature, so a shipping app links none of it.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::Piece;
    use day_core::conformance::{Case, Drive, FrameExpect, NativeExpect};
    use day_reactive::Signal;
    use day_spec::props::Truncation;
    use day_spec::{Cap, Font, Role, kinds};

    use crate::*;

    /// A button fires its action on each press and counts it.
    #[day_macros::test(day_core)]
    fn button_press() -> Case {
        let presses = Signal::new(0i64);
        Case::new()
            .proves(kinds::BUTTON)
            .page(move || {
                column((
                    button("Press")
                        .action(move || presses.update(|n| *n += 1))
                        .id(day_core::id!(press)),
                    label(move || presses.get().to_string()).id(day_core::id!(count)),
                ))
                .spacing(8.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_text("press", "Press").await?;
                d.tap("press").await?;
                d.assert_text("count", "1").await?;
                d.tap("press").await?;
                d.assert_text("count", "2").await
            })
    }

    /// A disabled button reports itself disabled, in Day and natively, and takes presses once
    /// enabled. (A tap on a disabled button is not drivable: the `tap` step waits for the
    /// element to be enabled, as a person would.)
    #[day_macros::test(day_core)]
    fn button_disabled() -> Case {
        let presses = Signal::new(0i64);
        let enabled = Signal::new(false);
        Case::new()
            .proves(kinds::BUTTON)
            .proves_duty("set_enabled")
            .page(move || {
                column((
                    button("Off")
                        .enabled(move || enabled.get())
                        .action(move || presses.update(|n| *n += 1))
                        .id(day_core::id!(off)),
                    label(move || presses.get().to_string()).id(day_core::id!(count)),
                    button("Enable")
                        .action(move || enabled.set(true))
                        .id(day_core::id!(enable)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_enabled("off", false).await?;
                d.assert_text("count", "0").await?;
                d.tap("enable").await?;
                d.assert_enabled("off", true).await?;
                d.tap("off").await?;
                d.assert_text("count", "1").await
            })
    }

    /// A toggle reports its flips to its signal, and a write to the signal reaches it.
    #[day_macros::test(day_core)]
    fn toggle_binding() -> Case {
        let on = Signal::new(false);
        Case::new()
            .proves(kinds::TOGGLE)
            .page(move || {
                column((
                    toggle(on).id(day_core::id!(tgl_switch)),
                    label(move || if on.get() { "on" } else { "off" }.to_owned())
                        .id(day_core::id!(tgl_state)),
                    button("Set on")
                        .action(move || on.set(true))
                        .id(day_core::id!(tgl_set)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.toggle("tgl_switch", true).await?;
                d.assert_text("tgl_state", "on").await?;
                d.toggle("tgl_switch", false).await?;
                d.assert_text("tgl_state", "off").await?;
                d.tap("tgl_set").await?;
                d.assert_on("tgl_switch", true).await
            })
    }

    /// One flip is one change (DESIGN.md §4.4): the control echoing a value written to it, by
    /// the flip itself or by the app, does not notify the binding's watchers a second time. A
    /// watcher that toggles something on each change (reverse a list) would undo its own work.
    #[day_macros::test(day_core)]
    fn toggle_flip_is_one_change() -> Case {
        let on = Signal::new(false);
        let changes = Signal::new(0u32);
        Case::new()
            .proves(kinds::TOGGLE)
            .page(move || {
                day_reactive::watch(move || on.get(), move |_, _| changes.update(|n| *n += 1));
                column((
                    toggle(on).id(day_core::id!(tgl_once)),
                    button("Set on")
                        .action(move || on.set(true))
                        .id(day_core::id!(tgl_once_set)),
                    label(move || format!("changes {}", changes.get()))
                        .id(day_core::id!(tgl_once_changes)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.toggle("tgl_once", true).await?;
                d.wait_idle().await?;
                d.pause(0.3).await?;
                d.assert_text("tgl_once_changes", "changes 1").await?;
                d.toggle("tgl_once", false).await?;
                d.wait_idle().await?;
                d.pause(0.3).await?;
                d.assert_text("tgl_once_changes", "changes 2").await?;
                d.tap("tgl_once_set").await?;
                d.wait_idle().await?;
                d.pause(0.3).await?;
                d.assert_on("tgl_once", true).await?;
                d.assert_text("tgl_once_changes", "changes 3").await
            })
    }

    /// A text field writes what is typed to its signal, and shows what the signal is set to.
    #[day_macros::test(day_core)]
    fn text_field_binding() -> Case {
        let name = Signal::new(String::new());
        Case::new()
            .proves(kinds::TEXT_FIELD)
            .page(move || {
                column((
                    text_field(name)
                        .placeholder("Name")
                        .id(day_core::id!(tf_field)),
                    label(move || name.get()).id(day_core::id!(tf_echo)),
                    button("Set")
                        .action(move || name.set("Ada".into()))
                        .id(day_core::id!(tf_set)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.input("tf_field", "Grace").await?;
                d.assert_text("tf_echo", "Grace").await?;
                d.tap("tf_set").await?;
                d.assert_text("tf_field", "Ada").await
            })
    }

    /// A secure field keeps its text and its focus across a show-password flip: on AppKit and
    /// WinUI the flip rebuilds the widget as its other native class (docs/textfield.md).
    #[day_macros::test(day_core)]
    fn text_field_secure() -> Case {
        let password = Signal::new(String::new());
        let shown = Signal::new(false);
        Case::new()
            .proves(kinds::TEXT_FIELD)
            .proves_duty("set_input_traits")
            .page(move || {
                column((
                    secure_field(password)
                        .secure(move || !shown.get())
                        .placeholder("Password")
                        .id(day_core::id!(tfs_pass)),
                    toggle(shown).id(day_core::id!(tfs_show)),
                    label(move || password.get().chars().count().to_string())
                        .id(day_core::id!(tfs_len)),
                ))
                .spacing(8.0)
            })
            .shot("masked")
            .drive(|d: Drive| async move {
                d.input("tfs_pass", "correct horse").await?;
                d.assert_text("tfs_len", "13").await?;
                d.focus("tfs_pass").await?;
                d.toggle("tfs_show", true).await?;
                d.wait_idle().await?;
                d.shot("shown").await?;
                d.assert_focused("tfs_pass", true).await?;
                d.input("tfs_pass", "battery").await?;
                d.assert_text("tfs_len", "7").await?;
                d.toggle("tfs_show", false).await?;
                d.wait_idle().await?;
                d.assert_text("tfs_pass", "battery").await
            })
    }

    /// A field held to four characters cuts what is typed past them, on every toolkit alike.
    #[day_macros::test(day_core)]
    fn text_field_max_length() -> Case {
        let pin = Signal::new(String::new());
        Case::new()
            .proves(kinds::TEXT_FIELD)
            .page(move || {
                column((
                    text_field(pin).max_length(4).id(day_core::id!(tfm_pin)),
                    label(move || pin.get()).id(day_core::id!(tfm_echo)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.input("tfm_pin", "123456").await?;
                d.assert_text("tfm_echo", "1234").await?;
                d.assert_text("tfm_pin", "1234").await
            })
    }

    /// A slider's value reaches its signal on the step grid the piece declares, whatever the
    /// native control's own stepping.
    #[day_macros::test(day_core)]
    fn slider_range() -> Case {
        let value = Signal::new(40.0f64);
        Case::new()
            .proves(kinds::SLIDER)
            .page(move || {
                column((
                    slider(value)
                        .range(0.0..=100.0)
                        .step(10.0)
                        .id(day_core::id!(sl_slider)),
                    label(move || format!("{:.0}", value.get())).id(day_core::id!(sl_value)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.set_value("sl_slider", 72.0).await?;
                d.assert_text("sl_value", "70").await?;
                d.assert_value("sl_slider", 70.0).await
            })
    }

    /// A label shows its text, follows a signal, and reports its explicit role natively where
    /// the toolkit reads its tree back.
    #[day_macros::test(day_core)]
    fn label_heading() -> Case {
        let text = Signal::new("Hello".to_owned());
        Case::new()
            .proves(kinds::LABEL)
            .proves_duty("set_a11y")
            .page(move || {
                column((
                    label(move || text.get())
                        .font(Font::Headline)
                        .a11y(|a| a.role(Role::Heading(2)))
                        .id(day_core::id!(lbl_heading)),
                    button("Rename")
                        .action(move || text.set("Renamed".into()))
                        .id(day_core::id!(lbl_rename)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("lbl_heading", "Hello").await?;
                d.tap("lbl_rename").await?;
                d.assert_text("lbl_heading", "Renamed").await?;
                d.a11y_audit(Some("lbl_heading")).await
            })
    }

    /// A label shows its text, natively too.
    #[day_macros::test(day_core)]
    fn label_text() -> Case {
        Case::new()
            .proves(kinds::LABEL)
            .page(|| label("Plain text").id(day_core::id!(text)))
            .drive(|d: Drive| async move { d.assert_text("text", "Plain text").await })
    }

    /// A label bound to a signal follows every write.
    #[day_macros::test(day_core)]
    fn label_reactive() -> Case {
        let count = Signal::new(0i64);
        Case::new()
            .proves(kinds::LABEL)
            .proves_duty("update")
            .proves_duty("read_native")
            .page(move || {
                column((
                    label(move || format!("Count {}", count.get())).id(day_core::id!(text)),
                    button("Add")
                        .action(move || count.update(|n| *n += 1))
                        .id(day_core::id!(add)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("text", "Count 0").await?;
                d.tap("add").await?;
                d.tap("add").await?;
                d.assert_text("text", "Count 2").await
            })
    }

    /// Styled runs draw as one string: the native text is the runs joined, without markup.
    #[day_macros::test(day_core)]
    fn label_runs() -> Case {
        Case::new()
            .proves(kinds::LABEL)
            .proves_cap(Cap::TextRuns)
            .requires(Cap::TextRuns)
            .page(|| {
                label("")
                    .runs_from(
                        TextBuilder::new()
                            .text("Plain, ")
                            .strong("strong")
                            .text(" and ")
                            .emphasis("emphasized"),
                    )
                    .id(day_core::id!(runs))
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_text("runs", "Plain, strong and emphasized").await
            })
    }

    /// A label's own color stays under its runs when new runs replace the old ones.
    #[day_macros::test(day_core)]
    fn label_runs_keep_the_label_color() -> Case {
        let text = Signal::new(
            "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} **\u{2588}**".to_string(),
        );
        Case::new()
            .proves(kinds::LABEL)
            .requires(Cap::TextRuns)
            .requires(Cap::Snapshot)
            .page(move || {
                column((
                    label(text)
                        .markdown()
                        .color(day_spec::Color::rgb(1.0, 0.0, 0.0))
                        .id(day_core::id!(colored)),
                    button("Restyle")
                        .id(day_core::id!(restyle))
                        .action(move || {
                            text.set(
                                "\u{2588}\u{2588}\u{2588}\u{2588}\u{2588}\u{2588} *\u{2588}*"
                                    .into(),
                            );
                        }),
                ))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("colored", 0.1, 0.5, "#ff0000").await?;
                d.tap("restyle").await?;
                d.sample_pixel("colored", 0.1, 0.5, "#ff0000").await
            })
    }

    /// Replacing link runs, clearing them and restoring them preserves the native text.
    /// Native input suites separately exercise each platform's span hit testing.
    #[day_macros::test(day_core)]
    fn label_link_runs_reactive() -> Case {
        let text = Signal::new("😀 [Alpha](#alpha) / [Beta](#beta)".to_string());
        Case::new()
            .proves(kinds::LABEL)
            .requires(Cap::TextRuns)
            .page(move || {
                column((
                    label(text).markdown().id(day_core::id!(links)),
                    button("Replace")
                        .id(day_core::id!(replace))
                        .action(move || {
                            text.set("[東京](#tokyo) then [again](#again)".into());
                        }),
                    button("Plain")
                        .id(day_core::id!(plain))
                        .action(move || text.set("Plain 😀".into())),
                    button("Restore")
                        .id(day_core::id!(restore))
                        .action(move || text.set("[Restored](#restored)".into())),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("links", "😀 Alpha / Beta").await?;
                d.tap("replace").await?;
                d.assert_text("links", "東京 then again").await?;
                d.tap("plain").await?;
                d.assert_text("links", "Plain 😀").await?;
                d.tap("restore").await?;
                d.assert_text("links", "Restored").await
            })
    }

    /// A marker box under a label, so a case can compare two labels' heights by where their
    /// markers sit.
    fn foot(id: day_core::ElementId) -> impl Piece {
        rectangle()
            .fill(day_spec::Color::hex(0x0033_6699))
            .frame(10.0, 4.0)
            .id(id)
    }

    const LONG: &str = "Day lays this sentence out over many lines when its label is narrow, \
        so a line limit has plenty of text to cut and the ellipsis has something to stand for.";

    /// A line limit caps the label's height on every toolkit: held to two lines, a long label
    /// is exactly as tall as a two-line one, and held to one, as tall as a one-line one. The
    /// limit follows its signal, and the text Day reports stays whole.
    #[day_macros::test(day_core)]
    fn label_max_lines() -> Case {
        let limit = Signal::new(0u32);
        Case::new()
            .proves(kinds::LABEL)
            .page(move || {
                column((
                    row((
                        column((
                            label(LONG)
                                .max_lines(limit)
                                .width(100.0)
                                .id(day_core::id!(ml_text)),
                            foot(day_core::id!(ml_foot)),
                        ))
                        .align(HAlign::Leading),
                        column((label("Ag\nAg").width(100.0), foot(day_core::id!(ml_two))))
                            .align(HAlign::Leading),
                        column((label("Ag").width(100.0), foot(day_core::id!(ml_one))))
                            .align(HAlign::Leading),
                    ))
                    .align(VAlign::Top),
                    button("Two lines")
                        .action(move || limit.set(2))
                        .id(day_core::id!(ml_set_two)),
                    button("One line")
                        .action(move || limit.set(1))
                        .id(day_core::id!(ml_set_one)),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .shot("unlimited")
            .drive(|d: Drive| async move {
                let level_with = |other: &str| FrameExpect {
                    y: Some(0.0),
                    relative_to: Some(other.into()),
                    ..Default::default()
                };
                d.tap("ml_set_two").await?;
                d.wait_idle().await?;
                d.assert_frame("ml_foot", level_with("ml_two")).await?;
                d.shot("two").await?;
                d.tap("ml_set_one").await?;
                d.wait_idle().await?;
                d.assert_frame("ml_foot", level_with("ml_one")).await?;
                d.assert_text("ml_text", LONG).await
            })
    }

    /// A one-line label stays one line tall and as wide as its box in each truncation mode,
    /// and the text Day reports is the whole text. Where the ellipsis sits is for the capture.
    #[day_macros::test(day_core)]
    fn label_truncation_modes() -> Case {
        let mode = Signal::new(Truncation::Middle);
        Case::new()
            .proves(kinds::LABEL)
            .page(move || {
                column((
                    row((
                        column((
                            label("/usr/local/share/day/templates/app/src/pages.rs")
                                .single_line()
                                .truncation(mode)
                                .width(120.0)
                                .id(day_core::id!(tr_path)),
                            foot(day_core::id!(tr_foot)),
                        ))
                        .align(HAlign::Leading),
                        column((label("Ag").width(120.0), foot(day_core::id!(tr_one))))
                            .align(HAlign::Leading),
                    ))
                    .align(VAlign::Top),
                    button("Head")
                        .action(move || mode.set(Truncation::Head))
                        .id(day_core::id!(tr_head)),
                    button("Tail")
                        .action(move || mode.set(Truncation::Tail))
                        .id(day_core::id!(tr_tail)),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .shot("middle")
            .drive(|d: Drive| async move {
                let one_line = || FrameExpect {
                    y: Some(0.0),
                    relative_to: Some("tr_one".into()),
                    ..Default::default()
                };
                d.assert_frame("tr_foot", one_line()).await?;
                d.tap("tr_head").await?;
                d.wait_idle().await?;
                d.assert_frame("tr_foot", one_line()).await?;
                d.shot("head").await?;
                d.tap("tr_tail").await?;
                d.wait_idle().await?;
                d.assert_frame("tr_foot", one_line()).await?;
                d.assert_text("tr_path", "/usr/local/share/day/templates/app/src/pages.rs")
                    .await
            })
    }

    /// Line height is a multiple of the em, the font size as drawn, compared here without
    /// assuming how many units a point is. Two one-line labels at 2.0 (four ems) end level with
    /// three lines at 4/3 and with two lines at 2.0; a change of the multiple to 1.5 re-measures
    /// both, so the pair (three ems) ends level with two lines at 1.5. Every reference stays at
    /// or above a font's natural line height, so no platform floor gets in the way.
    #[day_macros::test(day_core)]
    fn label_line_height() -> Case {
        let multiple = Signal::new(2.0);
        let line = |text: &'static str| label(text).font(Font::System(20.0));
        Case::new()
            .proves(kinds::LABEL)
            .page(move || {
                column((
                    row((
                        column((
                            line("Ag").line_height(multiple).id(day_core::id!(lh_one)),
                            line("Ag").line_height(multiple),
                            foot(day_core::id!(lh_pair_end)),
                        ))
                        .spacing(0.0),
                        column((
                            line("Ag\nAg\nAg").line_height(4.0 / 3.0),
                            foot(day_core::id!(lh_four_end)),
                        ))
                        .spacing(0.0),
                        column((
                            line("Ag\nAg").line_height(2.0).id(day_core::id!(lh_two)),
                            foot(day_core::id!(lh_two_end)),
                        ))
                        .spacing(0.0),
                        column((
                            line("Ag\nAg").line_height(1.5),
                            foot(day_core::id!(lh_three_end)),
                        ))
                        .spacing(0.0),
                    ))
                    .spacing(8.0)
                    .align(VAlign::Top),
                    button("Tighter")
                        .action(move || multiple.set(1.5))
                        .id(day_core::id!(lh_tighter)),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                let level_with = |reference: &str| FrameExpect {
                    y: Some(0.0),
                    relative_to: Some(reference.into()),
                    tolerance: Some(2.0),
                    ..Default::default()
                };
                d.assert_frame("lh_pair_end", level_with("lh_four_end"))
                    .await?;
                d.assert_frame("lh_two_end", level_with("lh_four_end"))
                    .await?;
                d.tap("lh_tighter").await?;
                d.wait_idle().await?;
                d.assert_frame("lh_pair_end", level_with("lh_three_end"))
                    .await
            })
    }

    /// Letter spacing is in ems of the label's own font as drawn, compared without assuming
    /// how many units a point is. Every row holds two labels, so a label's own inset (AppKit's
    /// text field cell adds two points a side) and its rounding cancel, and every spaced label
    /// has a spaced partner of the same length, so it does not matter whether a platform adds
    /// the spacing after the last character too.
    ///
    /// Three characters of a 40-point font spaced by 0.75 em gain exactly what three of a
    /// 20-point font spaced by 1.5 em gain, so the two rows end level within a point of
    /// rounding; spacing that ignored the font's size would not. Then the size of the em: eight
    /// characters spaced by a quarter em gain one em more than four, which one em space
    /// (U+2003) stands in for, so an em taken as a point where the font draws larger (Linux
    /// draws a 20-point font 26.7 units tall) falls a quarter of an em short. An em space is a
    /// font's em by definition but not always to the unit (San Francisco's is 0.98 em at 20
    /// points), so that comparison allows half a point more than rounding.
    #[day_macros::test(day_core)]
    fn label_letter_spacing() -> Case {
        let at = |text: &'static str, points: f64| label(text).font(Font::System(points));
        let level = |reference: &str, tolerance: f64| FrameExpect {
            x: Some(0.0),
            relative_to: Some(reference.into()),
            tolerance: Some(tolerance),
            ..Default::default()
        };
        Case::new()
            .proves(kinds::LABEL)
            .page(move || {
                column((
                    row((
                        at("HHH", 40.0).letter_spacing(0.75),
                        at("HHH", 20.0),
                        foot(day_core::id!(ls_large_end)),
                    )),
                    row((
                        at("HHH", 40.0),
                        at("HHH", 20.0).letter_spacing(1.5),
                        foot(day_core::id!(ls_small_end)),
                    )),
                    row((
                        at("HHHHHHHH", 20.0)
                            .letter_spacing(0.25)
                            .id(day_core::id!(ls_spaced)),
                        at("HH", 20.0),
                        foot(day_core::id!(ls_spaced_end)),
                    )),
                    row((
                        at("HHHH", 20.0).letter_spacing(0.25),
                        at("HH\u{2003}HHHH", 20.0),
                        foot(day_core::id!(ls_plain_end)),
                    )),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .shot("default")
            .drive(move |d: Drive| async move {
                d.assert_frame("ls_large_end", level("ls_small_end", 1.0))
                    .await?;
                d.assert_frame("ls_spaced_end", level("ls_plain_end", 1.5))
                    .await?;
                d.assert_text("ls_spaced", "HHHHHHHH").await
            })
    }

    /// Decorations and font features change how text draws, never what it says: a label with a
    /// wavy colored underline on one word, a double underline over the whole text, small caps
    /// and oldstyle figures still reports its plain text.
    #[day_macros::test(day_core)]
    fn label_decorations_and_features() -> Case {
        Case::new()
            .proves(kinds::LABEL)
            .requires(Cap::TextRuns)
            .page(|| {
                column((
                    label("")
                        .runs_from(
                            TextBuilder::new()
                                .text("Check ")
                                .underline_styled(
                                    "unknown",
                                    day_spec::Decoration::wavy()
                                        .color(day_spec::Color::hex(0x00D0_021B)),
                                )
                                .text(" and ")
                                .strikethrough_styled("old", day_spec::DecorationStyle::Double)
                                .text(" in ")
                                .small_caps("Small Caps"),
                        )
                        .id(day_core::id!(dec_runs)),
                    label("Doubly underlined")
                        .underline(day_spec::Decoration::double())
                        .id(day_core::id!(dec_whole)),
                    label("Struck in red")
                        .strikethrough(
                            day_spec::Decoration::SINGLE.color(day_spec::Color::hex(0x00D0_021B)),
                        )
                        .id(day_core::id!(dec_struck)),
                    label("Chapter 1234567890")
                        .small_caps()
                        .oldstyle_figures()
                        .id(day_core::id!(dec_features)),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_text("dec_runs", "Check unknown and old in Small Caps")
                    .await?;
                d.assert_text("dec_whole", "Doubly underlined").await?;
                d.assert_text("dec_struck", "Struck in red").await?;
                d.assert_text("dec_features", "Chapter 1234567890").await
            })
    }

    /// A label that asks for selection reports is selectable and still shows its text. The
    /// reports themselves need a native selection gesture, which each toolkit's native input
    /// suite makes; the mock harness raises the event directly (`tests/text_mock.rs`).
    #[day_macros::test(day_core)]
    fn label_on_selection() -> Case {
        let chosen = Signal::new(String::new());
        Case::new()
            .proves(kinds::LABEL)
            .proves_duty("set_selectable")
            .page(move || {
                column((
                    label("Select part of this sentence")
                        .on_selection(move |s| chosen.set(s.text.clone()))
                        .id(day_core::id!(sel_text)),
                    label(move || chosen.get()).id(day_core::id!(sel_echo)),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("sel_text", "Select part of this sentence")
                    .await?;
                d.assert_text("sel_echo", "").await
            })
    }

    /// The system text scale is a positive number, the same for `Body` through either reader.
    #[day_macros::test(day_core)]
    fn text_scale_reads() -> Case {
        Case::new()
            .proves_duty("text_scale")
            .page(|| {
                label(|| format!("{:.2}", day_core::text_scale()))
                    .max_text_scale(1.0)
                    .id(day_core::id!(ts_scale))
            })
            .drive(|d: Drive| async move {
                let scale = day_core::text_scale();
                d.check(
                    scale.is_finite() && scale > 0.0,
                    "the text scale is positive",
                )?;
                d.check_eq(scale, day_core::text_scale_of(Font::Body))?;
                d.assert_text("ts_scale", &format!("{scale:.2}")).await
            })
    }

    /// A link is a label: it takes the label's builders, follows a reactive text, and a
    /// handler set with `on_link` replaces the default opening.
    #[day_macros::test(day_core)]
    fn link_takes_label_builders() -> Case {
        let title = Signal::new("Read the guide".to_owned());
        let handled = Signal::new(String::new());
        Case::new()
            .proves_duty("open_url")
            .page(move || {
                column((
                    link(title, "https://daybrite.dev/docs")
                        .font(Font::Footnote)
                        .bold()
                        .single_line()
                        .truncation(Truncation::Middle)
                        .id(day_core::id!(lk_default)),
                    link("Handled here", "app:custom")
                        .on_link(move |target| handled.set(target.to_owned()))
                        .id(day_core::id!(lk_handled)),
                    label(move || handled.get()).id(day_core::id!(lk_echo)),
                    button("Retitle")
                        .action(move || title.set("Open the guide".into()))
                        .id(day_core::id!(lk_retitle)),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("lk_default", "Read the guide").await?;
                d.tap("lk_retitle").await?;
                d.assert_text("lk_default", "Open the guide").await?;
                d.tap("lk_default").await?;
                d.assert_opened_url("https://daybrite.dev/docs").await?;
                d.tap("lk_handled").await?;
                d.assert_text("lk_echo", "app:custom").await
            })
    }

    /// A selectable label still shows its text. Every toolkit implements label selection
    /// (`set_selectable`); `Cap::TextSelectable` is the text area's toggle, not this.
    #[day_macros::test(day_core)]
    fn label_selectable() -> Case {
        Case::new()
            .proves(kinds::LABEL)
            .proves_modifier("selectable")
            .proves_duty("set_selectable")
            .page(|| label("Copy me").selectable().id(day_core::id!(text)))
            .drive(|d: Drive| async move { d.assert_text("text", "Copy me").await })
    }

    /// A button's title shows natively, and a title bound to a signal follows it.
    #[day_macros::test(day_core)]
    fn button_title() -> Case {
        let saved = Signal::new(false);
        Case::new()
            .proves(kinds::BUTTON)
            .page(move || {
                button(move || if saved.get() { "Saved" } else { "Save" }.to_owned())
                    .action(move || saved.set(true))
                    .id(day_core::id!(save))
            })
            .drive(|d: Drive| async move {
                d.assert_text("save", "Save").await?;
                d.tap("save").await?;
                d.assert_text("save", "Saved").await
            })
    }

    /// A button's accessibility label, as the platform reports it.
    #[day_macros::test(day_core)]
    fn button_a11y() -> Case {
        Case::new()
            .proves(kinds::BUTTON)
            .proves_modifier("a11y")
            .page(|| {
                button("Send")
                    .a11y(|a| a.label("Send the message"))
                    .id(day_core::id!(send))
            })
            .drive(|d: Drive| async move { d.a11y_audit(Some("send")).await })
    }

    /// An app's accessibility label survives a title change on a button with an icon (Windows
    /// and Android used to replace it with the new title).
    #[day_macros::test(day_core)]
    fn button_a11y_survives_title_change() -> Case {
        let playing = Signal::new(false);
        Case::new()
            .proves(kinds::BUTTON)
            .proves_modifier("a11y")
            .page(move || {
                button(move || if playing.get() { "Pause" } else { "Play" }.to_owned())
                    .icon(day_spec::Symbol::Play)
                    .action(move || playing.set(true))
                    .a11y(|a| a.label("Playback"))
                    .id(day_core::id!(play))
            })
            .drive(|d: Drive| async move {
                d.a11y_audit(Some("play")).await?;
                d.tap("play").await?;
                d.assert_text("play", "Pause").await?;
                d.a11y_audit(Some("play")).await
            })
    }

    /// A disabled toggle reports itself disabled, natively too, until it is enabled.
    #[day_macros::test(day_core)]
    fn toggle_disabled() -> Case {
        let on = Signal::new(false);
        let enabled = Signal::new(false);
        Case::new()
            .proves(kinds::TOGGLE)
            .page(move || {
                column((
                    toggle(on)
                        .enabled(move || enabled.get())
                        .id(day_core::id!(switch)),
                    button("Enable")
                        .action(move || enabled.set(true))
                        .id(day_core::id!(enable)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_enabled("switch", false).await?;
                d.tap("enable").await?;
                d.assert_enabled("switch", true).await?;
                d.toggle("switch", true).await?;
                d.assert_on("switch", true).await
            })
    }

    /// A toggle built disabled from a constant is disabled from the start.
    #[day_macros::test(day_core)]
    fn toggle_disabled_constant() -> Case {
        let on = Signal::new(true);
        Case::new()
            .proves(kinds::TOGGLE)
            .page(move || toggle(on).enabled(false).id(day_core::id!(switch)))
            .drive(|d: Drive| async move {
                d.assert_enabled("switch", false).await?;
                d.assert_on("switch", true).await
            })
    }

    /// A toggle's accessibility label, as the platform reports it.
    #[day_macros::test(day_core)]
    fn toggle_a11y() -> Case {
        let on = Signal::new(false);
        Case::new()
            .proves(kinds::TOGGLE)
            .page(move || {
                toggle(on)
                    .a11y(|a| a.label("Notifications"))
                    .id(day_core::id!(switch))
            })
            .drive(|d: Drive| async move { d.a11y_audit(Some("switch")).await })
    }

    /// A disabled slider reports itself disabled, natively too, until it is enabled.
    #[day_macros::test(day_core)]
    fn slider_disabled() -> Case {
        let value = Signal::new(0.5f64);
        let enabled = Signal::new(false);
        Case::new()
            .proves(kinds::SLIDER)
            .page(move || {
                column((
                    slider(value)
                        .enabled(move || enabled.get())
                        .id(day_core::id!(slider)),
                    button("Enable")
                        .action(move || enabled.set(true))
                        .id(day_core::id!(enable)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_enabled("slider", false).await?;
                d.tap("enable").await?;
                d.assert_enabled("slider", true).await?;
                d.set_value("slider", 0.25).await?;
                d.assert_value("slider", 0.25).await
            })
    }

    /// A write to a slider's signal moves the native thumb.
    #[day_macros::test(day_core)]
    fn slider_follows_signal() -> Case {
        let value = Signal::new(10.0f64);
        Case::new()
            .proves(kinds::SLIDER)
            .page(move || {
                column((
                    slider(value).range(0.0..=50.0).id(day_core::id!(slider)),
                    button("Set 40")
                        .action(move || value.set(40.0))
                        .id(day_core::id!(set)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_value("slider", 10.0).await?;
                d.tap("set").await?;
                d.assert_value("slider", 40.0).await
            })
    }

    /// The app moving a slider is not the reader moving it (DESIGN.md §4.4): the native value
    /// changes, and nothing comes back through the binding as if a hand had dragged the thumb.
    /// A media scrubber that echoed its own position would seek on every tick.
    #[day_macros::test(day_core)]
    fn slider_set_by_the_app_is_not_echoed() -> Case {
        /// A slider value that counts the writes the slider makes to it.
        #[derive(Clone, Copy)]
        struct Counted {
            value: Signal<f64>,
            writes: Signal<u32>,
        }
        impl Binding<f64> for Counted {
            fn read(&self) -> f64 {
                self.value.get()
            }
            fn peek(&self) -> f64 {
                self.value.get_untracked()
            }
            fn write(&self, v: f64) {
                self.writes.update(|n| *n += 1);
                self.value.set(v);
            }
        }
        let value = Signal::new(10.0f64);
        let writes = Signal::new(0u32);
        Case::new()
            .proves(kinds::SLIDER)
            .page(move || {
                column((
                    slider(Counted { value, writes })
                        .range(0.0..=50.0)
                        .id(day_core::id!(slider)),
                    button("Step")
                        .action(move || value.update(|v| *v += 7.5))
                        .id(day_core::id!(step)),
                    label(move || format!("writes {}", writes.get())).id(day_core::id!(writes)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_value("slider", 10.0).await?;
                for _ in 0..3 {
                    d.tap("step").await?;
                }
                d.assert_value("slider", 32.5).await?;
                d.wait_idle().await?;
                d.pause(0.3).await?;
                d.assert_text("writes", "writes 0").await
            })
    }

    /// A slider's accessibility label, as the platform reports it.
    #[day_macros::test(day_core)]
    fn slider_a11y() -> Case {
        let value = Signal::new(0.5f64);
        Case::new()
            .proves(kinds::SLIDER)
            .page(move || {
                slider(value)
                    .a11y(|a| a.label("Volume"))
                    .id(day_core::id!(slider))
            })
            .drive(|d: Drive| async move { d.a11y_audit(Some("slider")).await })
    }

    /// A disabled text field reports itself disabled, natively too, until it is enabled.
    #[day_macros::test(day_core)]
    fn text_field_disabled() -> Case {
        let text = Signal::new("Fixed".to_owned());
        let enabled = Signal::new(false);
        Case::new()
            .proves(kinds::TEXT_FIELD)
            .page(move || {
                column((
                    text_field(text)
                        .enabled(move || enabled.get())
                        .id(day_core::id!(field)),
                    button("Enable")
                        .action(move || enabled.set(true))
                        .id(day_core::id!(enable)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_enabled("field", false).await?;
                d.assert_text("field", "Fixed").await?;
                d.tap("enable").await?;
                d.assert_enabled("field", true).await?;
                d.input("field", "Changed").await?;
                d.assert_text("field", "Changed").await
            })
    }

    /// A read-only field shows what its signal holds and stays enabled (selectable).
    #[day_macros::test(day_core)]
    fn text_field_read_only() -> Case {
        let text = Signal::new("Shown".to_owned());
        Case::new()
            .proves(kinds::TEXT_FIELD)
            .proves_duty("set_input_traits")
            .page(move || {
                column((
                    text_field(text).read_only(true).id(day_core::id!(field)),
                    button("Replace")
                        .action(move || text.set("Replaced".into()))
                        .id(day_core::id!(replace)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_text("field", "Shown").await?;
                d.assert_enabled("field", true).await?;
                d.tap("replace").await?;
                d.assert_text("field", "Replaced").await
            })
    }

    /// Return in a field runs its submit action.
    #[day_macros::test(day_core)]
    fn text_field_submit() -> Case {
        let text = Signal::new(String::new());
        let sent = Signal::new(String::new());
        Case::new()
            .proves(kinds::TEXT_FIELD)
            .page(move || {
                column((
                    text_field(text)
                        .submit_label(day_spec::SubmitLabel::Send)
                        .on_submit(move || sent.set(text.get_untracked()))
                        .id(day_core::id!(field)),
                    label(move || sent.get()).id(day_core::id!(sent)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.input("field", "hello").await?;
                d.submit("field").await?;
                d.assert_text("sent", "hello").await
            })
    }

    /// A determinate bar shows its fraction natively and follows its signal.
    #[day_macros::test(day_core)]
    fn progress_value() -> Case {
        let done = Signal::new(0.25f64);
        Case::new()
            .proves(kinds::PROGRESS)
            .page(move || {
                column((
                    progress(done).id(day_core::id!(bar)),
                    button("Advance")
                        .action(move || done.set(0.75))
                        .id(day_core::id!(advance)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_value("bar", 0.25).await?;
                d.tap("advance").await?;
                d.assert_value("bar", 0.75).await
            })
    }

    /// A spinner shows, and stays shown under reduced motion (it keeps moving).
    #[day_macros::test(day_core)]
    fn spinner_renders() -> Case {
        Case::new()
            .proves(kinds::PROGRESS)
            .page(|| spinner().id(day_core::id!(spin)))
            .drive(|d: Drive| async move {
                d.assert_visible("spin").await?;
                d.assert_native(
                    "spin",
                    NativeExpect {
                        visible: Some(true),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// Synthetic fixture: large indicators retain their native size when animation stops.
    #[day_macros::test(day_core)]
    fn spinner_size_and_animation() -> Case {
        Case::new()
            .proves(kinds::PROGRESS)
            .page(|| {
                let running = Signal::new(true);
                column((
                    spinner()
                        .animating(running)
                        .id(day_core::id!(spinner_fixture::small)),
                    spinner()
                        .animating(running)
                        .large()
                        .id(day_core::id!(spinner_fixture::large)),
                    button("Stop fixture")
                        .action(move || running.set(false))
                        .id(day_core::id!(spinner_fixture::stop)),
                ))
            })
            .drive(|d: Drive| async move {
                let sizes = match day_core::toolkit_key() {
                    Some("appkit") => Some((20.0, 32.0)),
                    Some("uikit") => Some((20.0, 37.0)),
                    Some("gtk") => Some((24.0, 48.0)),
                    Some("arkui") => Some((32.0, 64.0)),
                    Some("xaml" | "winui") => Some((32.0, 48.0)),
                    Some("dom") => Some((22.0, 48.0)),
                    _ => None, // Qt uses a busy bar; Android uses its theme's dimensions.
                };
                for stopped in [false, true] {
                    if stopped {
                        d.tap("spinner_fixture::stop").await?;
                    }
                    d.assert_visible("spinner_fixture::small").await?;
                    d.assert_visible("spinner_fixture::large").await?;
                    if let Some((small, large)) = sizes {
                        d.assert_size("spinner_fixture::small", small, small)
                            .await?;
                        d.assert_size("spinner_fixture::large", large, large)
                            .await?;
                    }
                }
                Ok(())
            })
    }

    /// A divider takes the width it is given and draws as a thin rule.
    #[day_macros::test(day_core)]
    fn divider_renders() -> Case {
        Case::new()
            .proves(kinds::DIVIDER)
            .page(|| {
                column((
                    label("Above"),
                    divider().id(day_core::id!(rule)),
                    label("Below"),
                ))
                .spacing(8.0)
                .width(200.0)
            })
            .drive(|d: Drive| async move {
                d.assert_visible("rule").await?;
                d.assert_frame(
                    "rule",
                    FrameExpect {
                        width: Some(200.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A vertical divider takes the height it is given, with a width of its own: a hairline
    /// down a row, not across it.
    #[day_macros::test(day_core)]
    fn vertical_divider_fills_height() -> Case {
        Case::new()
            .proves(kinds::DIVIDER)
            .page(|| {
                row((
                    label("Left"),
                    divider().vertical().id(day_core::id!(rule)),
                    label("Right"),
                ))
                .spacing(8.0)
                .height(120.0)
                .id(day_core::id!(row))
            })
            .drive(|d: Drive| async move {
                d.assert_visible("rule").await?;
                // The thickness is the toolkit's (a point or a few); the height is the row's.
                d.assert_frame(
                    "rule",
                    FrameExpect {
                        height: Some(120.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A spacer takes the room between its neighbors.
    #[day_macros::test(day_core)]
    fn spacer_fills() -> Case {
        Case::new()
            .proves_modifier("width")
            .page(|| {
                row((
                    // The sized boxes, not the labels' own nodes.
                    label("A").width(50.0).wrapper_id(day_core::id!(first)),
                    spacer(),
                    label("B").width(50.0).wrapper_id(day_core::id!(last)),
                ))
                .width(300.0)
                .wrapper_id(day_core::id!(row))
            })
            .drive(|d: Drive| async move {
                d.assert_frame(
                    "last",
                    FrameExpect {
                        width: Some(50.0),
                        x: Some(250.0),
                        relative_to: Some("row".into()),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A link opens its URL (recorded during a test run, so nothing opens).
    #[day_macros::test(day_core)]
    fn link_opens_url() -> Case {
        Case::new()
            .proves_duty("open_url")
            .page(|| link("daybrite.dev", "https://daybrite.dev/").id(day_core::id!(link)))
            .drive(|d: Drive| async move {
                d.assert_text("link", "daybrite.dev").await?;
                d.tap("link").await?;
                d.assert_opened_url("https://daybrite.dev/").await
            })
    }
}

#[cfg(test)]
mod button_style_tests {
    use super::*;
    use day_spec::props::ButtonStyleSpec;

    #[test]
    fn compact_keeps_a_plain_button_plain() {
        assert_eq!(
            button("×").plain().compact().native_style,
            ButtonStyleSpec::Plain
        );
        assert_eq!(button("−").compact().native_style, ButtonStyleSpec::Compact);
        // The last style asked for still wins otherwise.
        assert_eq!(
            button("+").compact().plain().native_style,
            ButtonStyleSpec::Plain
        );
    }
}
