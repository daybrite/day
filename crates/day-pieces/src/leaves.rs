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
    /// A colored phrase.
    pub fn colored(self, s: &str, color: day_spec::Color) -> Self {
        let base = self.base;
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            color: Some(color),
            ..day_spec::TextRun::default()
        })
    }
    /// Underlined.
    pub fn underline(self, s: &str) -> Self {
        let base = self.base;
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            underline: day_spec::Underline::Single,
            ..day_spec::TextRun::default()
        })
    }
    /// Highlighted: a color painted behind the glyphs, for a search hit or a review mark.
    ///
    /// Sets the foreground too, through the same readable-on-a-fill rule `Button::tint` uses: a
    /// highlight is usually a pale wash, and the label's own text color is chosen for the window's
    /// background rather than for the swatch now sitting under it. On a dark theme that pairing
    /// puts light text on pale amber, which is the one combination a highlight must not produce.
    /// Use [`TextBuilder::run`] where an app wants to state both itself.
    pub fn highlight(self, s: &str, color: day_spec::Color) -> Self {
        let base = self.base;
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            background: Some(color),
            color: Some(day_spec::props::ButtonStyleSpec::on_tint(color)),
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
    /// Struck through.
    pub fn strikethrough(self, s: &str) -> Self {
        let base = self.base;
        self.run(s, move |range| day_spec::TextRun {
            range,
            font: day_spec::FontSpec::from(base),
            strikethrough: true,
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
    pub fn build(self) -> (String, Vec<day_spec::TextRun>) {
        (self.text, self.runs)
    }
}

pub struct Label {
    // pub(crate): `forms` builds Label literals directly (they were co-located before the split).
    pub(crate) text: TextSource,
    pub(crate) font: Font,
    pub(crate) font_scale: f64,
    pub(crate) weight: Option<day_spec::FontWeight>,
    pub(crate) italic: bool,
    pub(crate) tabular: bool,
    pub(crate) monospace: bool,
    pub(crate) wraps: bool,
    pub(crate) max_lines: u32,
    pub(crate) color: Option<Reactive<day_spec::Color>>,
    /// What the text means, for the color the platform gives it (docs/text.md).
    pub(crate) role: day_spec::props::TextRole,
    /// Styled spans over `text` (docs/text-runs.md); empty is an ordinary uniform label.
    pub(crate) runs: Vec<day_spec::TextRun>,
    /// Parse the text as inline markdown instead of taking it literally (docs/markdown.md).
    pub(crate) markdown: bool,
    /// How wrapped lines sit within the label's own width (docs/text.md).
    pub(crate) align: day_spec::props::TextAlign,
    /// What a tapped link run does. `None` uses [`open_link`].
    pub(crate) on_link: Option<LinkHandler>,
}

/// An app's handler for a tapped link run, shared because `Label` is cloned into its build.
pub(crate) type LinkHandler = Rc<dyn Fn(&str)>;

pub fn label<M>(text: impl IntoText<M>) -> Label {
    Label {
        text: text.into_text(),
        font: Font::Body,
        font_scale: 1.0,
        weight: None,
        italic: false,
        tabular: false,
        monospace: false,
        wraps: true,
        max_lines: 0,
        color: None,
        role: Default::default(),
        runs: Vec::new(),
        markdown: false,
        align: day_spec::props::TextAlign::Leading,
        on_link: None,
    }
}

#[day_macros::builder(LabelBuilder)]
impl Label {
    /// Keep plain text on one line. UIKit, Android, GTK, AppKit and DOM truncate with an ellipsis.
    pub fn single_line(mut self) -> Self {
        self.wraps = false;
        self
    }
    /// Limit a plain label to this many lines, truncating the last line on native Apple
    /// backends. Zero means unlimited. Rebuild the label to change its line limit.
    pub fn max_lines(mut self, lines: u32) -> Self {
        self.max_lines = lines;
        self
    }
    /// The semantic text style (`Font::Title`, `Font::Footnote`, …) or a custom `Font::System(pt)`.
    /// Backends render it with the platform's native style + accessibility text scaling.
    pub fn font(mut self, f: Font) -> Self {
        self.font = f;
        self
    }
    /// Scale the resolved native font, preserving its semantic style and accessibility size.
    /// Applies to the label's base font; explicit styled runs retain their own descriptors.
    pub fn font_scale(mut self, scale: f64) -> Self {
        self.font_scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
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
    /// Style spans within this label's text (docs/text-runs.md): one wrapping paragraph with
    /// emphasis, color, code or a link inside it, rather than several labels in a row.
    ///
    /// Ranges are byte offsets into the label's text, ascending and non-overlapping; text not
    /// covered by a run draws with the label's own font. Invalid runs are REJECTED at build time
    /// with a warning and the label renders plain, because the alternative is eight different
    /// wrong renderings (and a panic on the backends that slice `str`).
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
    /// Read the label's text as inline Markdown (docs/markdown.md): `**bold**`, `*italic*`,
    /// `` `code` ``, `~~strike~~` and `[text](url)` become styled runs, and the markers themselves
    /// are stripped.
    ///
    /// The parse happens at run time, on every change, so it works on a translated string chosen
    /// from the locale bundle, a value off the network, or text a user is typing, none of which a
    /// compile-time macro can see. The cost is a parse per update of a string that is a label's
    /// worth of text.
    ///
    /// ```ignore
    /// label(tr("release-note")).markdown()
    /// label(move || draft.get()).markdown()   // live as the user types
    /// ```
    ///
    /// Unrecognized markup stays literal, so a half-typed `**` reads as two asterisks rather than
    /// flickering. Block constructs (headings, lists, quotes) are not parsed: they are layout,
    /// which is `column`/`form`/`list`.
    /// Center (or trail) this label's lines within its own width, for the short wrapped block
    /// a welcome screen or an empty state uses. Only observable on a label that wraps, since a
    /// single line already fills its box.
    pub fn align(mut self, align: day_spec::props::TextAlign) -> Self {
        self.align = align;
        self
    }
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
    /// The text color: a constant, a `Signal<Color>`, or a `Fn() -> Color`. A reactive
    /// source recolors the native label when it changes (theme systems ride this).
    pub fn color<M>(mut self, c: impl IntoReactive<day_spec::Color, M>) -> Self {
        self.color = Some(c.into_reactive());
        self
    }
    /// Apply a named [`TextPreset`](crate::TextPreset): the font, weight, scale, emphasis and
    /// color an app defined once (`label(t).preset(&CAPTION)`).
    pub fn preset(mut self, preset: &crate::TextPreset) -> Self {
        self.font = preset.font;
        self.font_scale = preset.font_scale;
        if preset.weight.is_some() {
            self.weight = preset.weight;
        }
        self.italic |= preset.italic;
        self.tabular |= preset.tabular;
        self.monospace |= preset.monospace;
        if preset.secondary {
            self.role = day_spec::props::TextRole::Secondary;
        }
        if preset.single_line {
            self.wraps = false;
        }
        if let Some(c) = preset.color {
            self.color = Some(Reactive::Const(c));
        }
        self
    }
}

impl Piece for Label {
    fn build(self, cx: &mut BuildCx) -> RNode {
        // `.markdown()` replaces both the text and the runs: the markers are stripped from what
        // the label shows, so the two have to be produced together.
        let (initial, runs) = if self.markdown {
            crate::markdown::parse(&self.text.initial(), self.font)
        } else {
            (self.text.initial(), self.runs.clone())
        };
        // Validate once here rather than in eight backends: an overlapping or mid-character
        // range renders differently wrong on each, and panics on the ones that slice `str`.
        let runs = match day_spec::runs_are_valid(&initial, &runs) {
            Ok(()) => runs,
            Err(why) => {
                log::warn!("label runs ignored — {why}; the text renders unstyled");
                Vec::new()
            }
        };
        let node = cx.leaf(
            kinds::LABEL,
            &LabelProps {
                align: self.align,
                text: initial,
                font: day_spec::FontSpec {
                    style: self.font,
                    scale: self.font_scale,
                    weight: self.weight,
                    italic: self.italic,
                    tabular: self.tabular,
                    monospace: self.monospace,
                },
                color: self.color.as_ref().map(|c| c.get_untracked()),
                role: self.role,
                wraps: self.wraps,
                max_lines: self.max_lines,
                runs,
            },
            Flex::default(),
        );
        // A label that can carry link runs listens for their activation. Markdown labels qualify
        // whatever they currently hold, since a later parse may produce a link that this one did
        // not. Labels without any prospect of a link register nothing.
        let could_link =
            self.markdown || self.on_link.is_some() || self.runs.iter().any(|r| r.link.is_some());
        if could_link {
            let handler = self.on_link.clone();
            cx.on(node, move |ev| {
                if let Event::LinkActivated(url) = ev {
                    match &handler {
                        Some(f) => f(url),
                        None => open_link(url),
                    }
                }
            });
        }
        // A reactive markdown label re-parses on every change and patches text and runs together,
        // since the ranges only mean anything against the string they were parsed from.
        let font = self.font;
        let md = self.markdown;
        self.text.bind_to(
            node,
            move |t| {
                if md {
                    let (text, runs) = crate::markdown::parse(&t, font);
                    Box::new(LabelPatch::Runs(text, runs))
                } else {
                    Box::new(LabelPatch::Text(t))
                }
            },
            true,
        );
        // A reactive color recolors in place; a constant was applied once at realize.
        if let Some(Reactive::Dyn(f)) = self.color {
            bind(
                move || f(),
                move |c: &day_spec::Color| {
                    with_tree(|t| t.patch(node, Box::new(LabelPatch::Color(Some(*c))), false));
                },
            );
        }
        node
    }
}

/// The platform "tint" blue (iOS system blue, `#007AFF`) used as the default [`link`] color.
/// Override per-link with [`Link::color`] to match an app's accent.
const LINK_BLUE: day_spec::Color = day_spec::Color::rgb(0.0, 0.478, 1.0);

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

/// A tappable run of text that navigates to a `#route` or opens a URL in the platform's default
/// handler: the system browser for `http`/`https`, the mail client for `mailto:`, and so on.
///
/// It renders as accent-colored [`label`] text and announces itself as actionable to assistive
/// technology. The opening itself is delegated to the running backend
/// ([`Toolkit::open_url`](day_spec::Toolkit::open_url)), so it works the same on
/// every platform.
///
/// ```ignore
/// link(res::str::project_site(), "https://daybrite.dev")
/// link(res::str::settings(), "#settings")
/// link(res::str::email_us(), "mailto:hi@example.com").font(Font::Footnote)
/// ```
pub struct Link {
    label: Label,
    url: String,
}

/// Build a [`Link`] that activates `url` through [`open_link`] when tapped.
pub fn link<M>(text: impl IntoText<M>, url: impl Into<String>) -> Link {
    Link {
        label: label(text),
        url: url.into(),
    }
}

#[day_macros::builder(LinkBuilder)]
impl Link {
    /// The text style (default [`Font::Body`]).
    pub fn font(mut self, f: Font) -> Self {
        self.label = self.label.font(f);
        self
    }
    /// Override the link color (default the platform tint blue).
    pub fn color(mut self, c: day_spec::Color) -> Self {
        self.label = self.label.color(c);
        self
    }
    /// Render the link text bold.
    pub fn bold(mut self) -> Self {
        self.label = self.label.bold();
        self
    }
}

impl Piece for Link {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let url = self.url;
        let mut label = self.label;
        if day_core::capability(day_spec::Cap::TextLinks) == day_spec::Support::Native {
            // Use native link runs for theme colors, hover/pressed states, keyboard focus
            // and accessibility. Keep ranges together with localized/reactive text.
            let source = label.text.clone();
            let font = day_spec::FontSpec {
                style: label.font,
                scale: label.font_scale,
                weight: label.weight,
                italic: label.italic,
                ..Default::default()
            };
            let color = label.color.as_ref().map(|c| c.get_untracked());
            let target = url.clone();
            let runs = move |text: &str| {
                vec![day_spec::TextRun {
                    range: 0..text.len(),
                    font,
                    color,
                    link: Some(target.clone()),
                    ..Default::default()
                }]
            };
            let initial = source.initial();
            label.runs = runs(&initial);
            label.text = TextSource::Static(initial);
            let node = label.build(cx);
            source.bind_to(
                node,
                move |text| {
                    let spans = runs(&text);
                    Box::new(LabelPatch::Runs(text, spans))
                },
                true,
            );
            // Synthetic taps still exercise link policy; native activation is LinkActivated.
            cx.on(node, move |ev| {
                if matches!(ev, Event::Tap(_)) {
                    open_link(&url);
                }
            });
            node
        } else {
            if label.color.is_none() {
                label = label.color(LINK_BLUE);
            }
            label
                .on_tap(move || open_link(&url))
                .a11y(|b| b.role(Role::Button))
                .build(cx)
        }
    }
}

pub struct Button {
    title: TextSource,
    icon: Reactive<Option<day_spec::Icon>>,
    icon_only: bool,
    action: Option<Rc<dyn Fn()>>,
    native_style: day_spec::props::ButtonStyleSpec,
    /// A reactive tint, kept apart from `native_style` so the color can follow a signal. Set by
    /// [`Button::tint`]; it wins over `bordered`/`prominent` because it is the more specific ask.
    tint: Option<Reactive<day_spec::Color>>,
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
    pub fn tint<M>(mut self, color: impl IntoReactive<day_spec::Color, M>) -> Self {
        self.tint = Some(color.into_reactive());
        self
    }

    /// A button no wider than its title: a stepper's "−" and "+", a chip's "×". Drops the
    /// minimum width and wide insets of toolkits that have them (Material); a no-op where the
    /// stock button already hugs its title.
    pub fn compact(mut self) -> Self {
        self.native_style = day_spec::props::ButtonStyleSpec::Compact;
        self
    }

    /// Borderless native action button for toolbars and list-row accessories.
    /// Preserves native interaction, focus, disabled rendering, and accessibility.
    pub fn plain(mut self) -> Self {
        self.native_style = day_spec::props::ButtonStyleSpec::Plain;
        self
    }

    /// Mark this button as the current choice among its peers: a document tab, one view of a
    /// switcher. The platform draws its own checked or selected look (an NSButton's on state,
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
                move |col: &day_spec::Color| {
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(ButtonPatch::Style(
                                day_spec::props::ButtonStyleSpec::Tinted(*col),
                            )),
                            false,
                        )
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
                        t.patch(node, Box::new(ButtonPatch::Content(content.clone())), true)
                    });
                },
            );
        }
        node
    }
}

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
            if let Event::ToggleChanged(on) = ev {
                v.write(*on);
            }
        });
        node
    }
}

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
}

impl<S: Binding<String>> Piece for TextField<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.value.peek();
        let ph = self
            .placeholder
            .as_ref()
            .map(|p| p.initial())
            .unwrap_or_default();
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
        let v = self.value.clone();
        let g = guard.clone();
        bind_seeded(
            initial,
            move || v.read(),
            move |t: &String| {
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                with_tree(|tr| {
                    tr.patch(
                        node,
                        Box::new(TextFieldPatch::Text {
                            text: t.clone(),
                            from_native,
                        }),
                        false,
                    )
                });
            },
        );
        let v = self.value;
        let submit = self.on_submit;
        // Typing is a session: each keystroke is a preview (readers follow, nothing durable
        // fires), sealed into one committed change on Return or focus loss by the typing
        // coalescer. For a plain Signal binding preview defaults to write, so nothing changes
        // where no session semantics exist. Teardown seals too: navigating away from a page
        // mid-type must not leave the last burst outside the change log.
        let last: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
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
                            )
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
        node
    }
}

/// A progress indicator: a determinate bar (from [`progress`]) or an indeterminate spinner
/// (from [`spinner`]). See docs/progress.md.
pub struct Progress {
    /// `None` = indeterminate (spinner); `Some` = a determinate fraction source.
    value: Option<FractionSource>,
}

/// An indeterminate, animated progress indicator (a spinner / busy bar) for work with no
/// known extent.
pub fn spinner() -> Progress {
    Progress { value: None }
}

/// A determinate progress bar. `fraction` is the completed portion in `0.0..=1.0`; pass a
/// constant, a `Signal<f64>`, or a closure and it tracks reactively (out-of-range values are
/// clamped).
pub fn progress<M>(fraction: impl IntoFraction<M>) -> Progress {
    Progress {
        value: Some(fraction.into_fraction()),
    }
}

impl Piece for Progress {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let determinate = self.value.is_some();
        let initial = self.value.as_ref().map(|f| f.initial());
        let node = cx.leaf(
            kinds::PROGRESS,
            &ProgressProps { value: initial },
            // A determinate bar fills the available width (like a slider); a spinner keeps its
            // fixed intrinsic size.
            Flex {
                grow_w: determinate,
                ..Default::default()
            },
        );
        if let Some(src) = self.value {
            src.bind_to(node);
        }
        node
    }
}

/// A hairline between neighbors: across a column's width by default, down a row's height
/// with [`Divider::vertical`].
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
    use day_core::conformance::{Case, Drive, FrameExpect, NativeExpect};
    use day_reactive::Signal;
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
