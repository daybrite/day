// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Leaf pieces — the childless primitives: `label`, `link`, `button`, `toggle`, `slider`, `text_field`, `progress`/`spinner`, `divider`, and `spacer`.

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{bind, bind_seeded};
use day_spec::props::*;
use day_spec::{Event, Font, Role, kinds};

use crate::*;

// ---------------------------------------------------------------------------
// Leaves
// ---------------------------------------------------------------------------

/// Build a styled paragraph run by run (docs/text-runs.md).
///
/// The point of a builder is that byte ranges are error-prone to write by hand and meaningless to
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
    /// The style the emphasis variants build on — set it to the label's own font so a bold run
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
    /// Highlighted — a color painted BEHIND the glyphs, for a search hit or a review mark.
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
    /// A link run. RENDERING it is `Cap::TextRuns`; ACTIVATING it is `Cap::TextLinks`, which
    /// fewer backends have — check before relying on the tap (docs/text-runs.md).
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
    pub(crate) weight: Option<day_spec::FontWeight>,
    pub(crate) italic: bool,
    pub(crate) tabular: bool,
    pub(crate) monospace: bool,
    pub(crate) color: Option<Reactive<day_spec::Color>>,
    /// Styled spans over `text` (docs/text-runs.md); empty is an ordinary uniform label.
    pub(crate) runs: Vec<day_spec::TextRun>,
    /// Parse the text as inline markdown instead of taking it literally (docs/markdown.md).
    pub(crate) markdown: bool,
    /// How wrapped lines sit within the label's own width (docs/text.md).
    pub(crate) align: day_spec::props::TextAlign,
    /// What a tapped link run does. `None` opens the target in the platform's default handler,
    /// which is what a link in a paragraph of text is normally expected to do.
    pub(crate) on_link: Option<LinkHandler>,
}

/// An app's handler for a tapped link run, shared because `Label` is cloned into its build.
pub(crate) type LinkHandler = Rc<dyn Fn(&str)>;

pub fn label<M>(text: impl IntoText<M>) -> Label {
    Label {
        text: text.into_text(),
        font: Font::Body,
        weight: None,
        italic: false,
        tabular: false,
        monospace: false,
        color: None,
        runs: Vec::new(),
        markdown: false,
        align: day_spec::props::TextAlign::Leading,
        on_link: None,
    }
}

impl Label {
    /// The semantic text style (`Font::Title`, `Font::Footnote`, …) or a custom `Font::System(pt)`.
    /// Backends render it with the platform's native style + accessibility text scaling.
    pub fn font(mut self, f: Font) -> Self {
        self.font = f;
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
    /// Render the text italic (slanted).
    pub fn italic(mut self) -> Self {
        self.italic = true;
        self
    }
    /// Ask for TABULAR (monospaced) figures, so a changing number stops changing width.
    ///
    /// Pair it with [`Decorate::reserving`] for a readout beside a slider: reserving stops the box
    /// resizing when the digit COUNT changes, tabular stops the digits shifting inside it because
    /// `1` is narrower than `8`. See [`day_spec::FontSpec::tabular`].
    pub fn tabular(mut self) -> Self {
        self.tabular = true;
        self
    }
    /// Ask for the platform's monospaced face at this style's size — what inline code wants.
    pub fn monospace(mut self) -> Self {
        self.monospace = true;
        self
    }
    /// Style spans WITHIN this label's text (docs/text-runs.md): one wrapping paragraph with
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
    /// Take BOTH the text and its runs from a [`TextBuilder`], replacing whatever text the label
    /// was built with. This is the intended entry point — the builder guarantees the ranges match
    /// the string, which is the invariant hand-written runs get wrong.
    pub fn runs_from(mut self, b: TextBuilder) -> Self {
        let (text, runs) = b.build();
        self.text = TextSource::Static(text);
        self.runs = runs;
        self
    }
    /// Read the label's text as inline MARKDOWN (docs/markdown.md): `**bold**`, `*italic*`,
    /// `` `code` ``, `~~strike~~` and `[text](url)` become styled runs, and the markers themselves
    /// are stripped.
    ///
    /// The parse happens at run time, on every change — so it works on a translated string chosen
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
    /// flickering. Block constructs (headings, lists, quotes) are NOT parsed: they are layout,
    /// which is `column`/`form`/`list`.
    /// Center (or trail) this label's lines within its own width — for the short wrapped block
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
    /// Without this, a link opens in the platform's default handler, the same as the [`link`]
    /// piece. Set it to route in-app (a `day://` scheme, a route name) or to confirm first.
    ///
    /// Activation is `Cap::TextLinks`, which is narrower than run RENDERING — on a backend
    /// without it the link still draws, and nothing calls this (docs/text-runs.md).
    pub fn on_link(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_link = Some(Rc::new(f));
        self
    }
    /// The text color: a constant, a `Signal<Color>`, or a `Fn() -> Color` — a reactive
    /// source recolors the native label when it changes (theme systems ride this).
    pub fn color<M>(mut self, c: impl IntoReactive<day_spec::Color, M>) -> Self {
        self.color = Some(c.into_reactive());
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
        // Validate ONCE here rather than in eight backends: an overlapping or mid-character
        // range renders differently wrong on each, and panics on the ones that slice `str`.
        let runs = match day_spec::runs_are_valid(&initial, &runs) {
            Ok(()) => runs,
            Err(why) => {
                eprintln!("day: label runs ignored — {why}; the text renders unstyled");
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
                    weight: self.weight,
                    italic: self.italic,
                    tabular: self.tabular,
                    monospace: self.monospace,
                    ..day_spec::FontSpec::default()
                },
                color: self.color.as_ref().map(|c| c.get_untracked()),
                wraps: true,
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
                        // The unhandled case is the common one: open it, like the `link` piece.
                        None => day_core::open_url(url),
                    }
                }
            });
        }
        // A reactive markdown label re-parses on every change and patches text AND runs together,
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

/// A tappable run of text that opens `url` in the platform's default handler — the system browser
/// for `http`/`https`, the mail client for `mailto:`, and so on. This is Day's analogue of
/// SwiftUI's `Link`.
///
/// It renders as accent-colored [`label`] text and announces itself as actionable to assistive
/// technology. The opening itself is delegated to the running backend
/// ([`Toolkit::open_url`](../day_spec/trait.Toolkit.html#method.open_url)), so it works the same on
/// every platform.
///
/// ```ignore
/// link("daybrite.dev", "https://daybrite.dev")
/// link(tr("email-us"), "mailto:hi@example.com").font(Font::Footnote)
/// ```
pub struct Link {
    label: Label,
    url: String,
}

/// Build a [`Link`] that opens `url` when tapped.
pub fn link<M>(text: impl IntoText<M>, url: impl Into<String>) -> Link {
    Link {
        label: label(text).color(LINK_BLUE),
        url: url.into(),
    }
}

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
        self.label
            .on_tap(move || day_core::open_url(&url))
            .a11y(|b| b.role(Role::Button))
            .build(cx)
    }
}

pub struct Button {
    title: TextSource,
    action: Option<Rc<dyn Fn()>>,
    native_style: day_spec::props::ButtonStyleSpec,
    /// A reactive tint, kept apart from `native_style` so the color can follow a signal. Set by
    /// [`Button::tint`]; it wins over `bordered`/`prominent` because it is the more specific ask.
    tint: Option<Reactive<day_spec::Color>>,
    enabled: Reactive<bool>,
}

pub fn button<M>(title: impl IntoText<M>) -> Button {
    Button {
        title: title.into_text(),
        action: None,
        native_style: day_spec::props::ButtonStyleSpec::Automatic,
        tint: None,
        enabled: true.into_reactive(),
    }
}

impl Button {
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
    /// control). Reactive, so it can follow app state — e.g. `.enabled(move || !busy.get())` to
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

    /// A filled button in a color of your choosing, still drawn by the NATIVE control.
    ///
    /// The platform keeps everything that makes a button a button: its pressed and hover
    /// rendering, its focus ring, its disabled look, its accessibility role, and keyboard
    /// activation. Only the fill is yours. The label color is chosen for contrast against the
    /// fill, so a pale tint gets dark text and a saturated one white.
    ///
    /// Reactive, so the color can follow app state — `.tint(move || if recording { RUST } else
    /// { SKY })` recolors in place rather than rebuilding the button.
    ///
    /// A backend that cannot recolor its button ignores the tint and draws its ordinary button
    /// (docs/buttons.md). That is deliberate: a plain button on one platform is a far smaller
    /// loss than a colored rectangle that is no longer a button.
    pub fn tint<M>(mut self, color: impl IntoReactive<day_spec::Color, M>) -> Self {
        self.tint = Some(color.into_reactive());
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
                title: initial,
                enabled: self.enabled.get_untracked(),
                style,
            },
            Flex::default(),
        );
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
        // A reactive `enabled` patches on change; a constant is applied once at realize — the same
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
            // for users — but an event delivered by another route (a dayscript `tap`, which
            // dispatches to the node rather than simulating a touch) would otherwise fire an action
            // the user cannot reach. `.enabled(false)` should mean "cannot fire", not "looks gray".
            let gate = enabled_gate;
            cx.on(node, move |ev| {
                if matches!(ev, Event::Pressed) && gate.get() {
                    action();
                }
            });
        }
        self.title
            .bind_to(node, |t| Box::new(ButtonPatch::Title(t)), true);
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

impl<S: Binding<bool>> Toggle<S> {
    /// Whether the toggle is interactive (default `true`; `false` = disabled/grayed). Reactive —
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
}

pub fn slider<S: Binding<f64>>(value: S) -> Slider<S> {
    Slider {
        value,
        min: 0.0,
        max: 1.0,
        step: None,
    }
}

impl<S: Binding<f64>> Slider<S> {
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
                enabled: true,
            },
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
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
            // Honor `.step(_)` at the framework layer so EVERY backend produces stepped values —
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
                // The settled value: ONE record for the whole drag. A backend that cannot
                // tell the two apart never sends this, and the preview default (a plain
                // write) keeps it correct — chattier, never wrong.
                Event::ValueCommitted(val) => v.write_commit(snap(*val)),
                _ => {}
            }
        });
        node
    }
}

pub struct TextField<S: Binding<String>> {
    value: S,
    placeholder: Option<TextSource>,
    on_submit: Option<Rc<dyn Fn()>>,
}

pub fn text_field<S: Binding<String>>(value: S) -> TextField<S> {
    TextField {
        value,
        placeholder: None,
        on_submit: None,
    }
}

impl<S: Binding<String>> TextField<S> {
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
                enabled: true,
            },
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
        // Controlled input with origin-tagged writes (§4.4): the echo guard remembers the
        // last value that came FROM the native widget so its own change is not written back.
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
        // Typing is a session: each keystroke is a PREVIEW (readers follow, nothing durable
        // fires), sealed into one committed change on Return or focus loss — the typing
        // coalescer. For a plain Signal binding preview defaults to write, so nothing changes
        // where no session semantics exist. TEARDOWN seals too: navigating away from a page
        // mid-type must not leave the last burst outside the change log.
        let last: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        {
            let (v, last) = (v.clone(), last.clone());
            day_reactive::Scope::current().on_cleanup(move || {
                if let Some(t) = last.borrow_mut().take() {
                    v.write_commit(t);
                }
            });
        }
        cx.on(node, move |ev| match ev {
            Event::TextChanged(t) => {
                *guard.borrow_mut() = Some(t.clone());
                *last.borrow_mut() = Some(t.clone());
                v.write_preview(t.clone());
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

pub struct Divider;

pub fn divider() -> Divider {
    Divider
}

impl Piece for Divider {
    fn build(self, cx: &mut BuildCx) -> RNode {
        cx.leaf(
            kinds::DIVIDER,
            &(),
            Flex {
                grow_w: true,
                ..Default::default()
            },
        )
    }
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
