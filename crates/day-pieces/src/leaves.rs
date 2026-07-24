//! Leaf pieces — the childless primitives: `label`, `link`, `button` (plus the `ButtonStyle`
//! hook), `toggle`, `slider`, `text_field`, `progress`/`spinner`, `divider`, and `spacer`.

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::bind_seeded;
use day_spec::props::*;
use day_spec::{Event, Font, Insets, Role, kinds};

use crate::*;

// ---------------------------------------------------------------------------
// Leaves
// ---------------------------------------------------------------------------

pub struct Label {
    // pub(crate): `forms` builds Label literals directly (they were co-located before the split).
    pub(crate) text: TextSource,
    pub(crate) font: Font,
    pub(crate) weight: Option<day_spec::FontWeight>,
    pub(crate) italic: bool,
    pub(crate) color: Option<day_spec::Color>,
}

pub fn label<M>(text: impl IntoText<M>) -> Label {
    Label {
        text: text.into_text(),
        font: Font::Body,
        weight: None,
        italic: false,
        color: None,
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
    pub fn color(mut self, c: day_spec::Color) -> Self {
        self.color = Some(c);
        self
    }
}

impl Piece for Label {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.text.initial();
        let node = cx.leaf(
            kinds::LABEL,
            &LabelProps {
                text: initial,
                font: day_spec::FontSpec {
                    style: self.font,
                    weight: self.weight,
                    italic: self.italic,
                },
                color: self.color,
                wraps: true,
            },
            Flex::default(),
        );
        self.text
            .bind_to(node, |t| Box::new(LabelPatch::Text(t)), true);
        node
    }
}

/// The platform "tint" blue (iOS system blue, `#007AFF`) used as the default [`link`] colour.
/// Override per-link with [`Link::color`] to match an app's accent.
const LINK_BLUE: day_spec::Color = day_spec::Color::rgb(0.0, 0.478, 1.0);

/// A tappable run of text that opens `url` in the platform's default handler — the system browser
/// for `http`/`https`, the mail client for `mailto:`, and so on. This is Day's analogue of
/// SwiftUI's `Link`.
///
/// It renders as accent-coloured [`label`] text and announces itself as actionable to assistive
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
    /// Override the link colour (default the platform tint blue).
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
}

pub fn button<M>(title: impl IntoText<M>) -> Button {
    Button {
        title: title.into_text(),
        action: None,
        native_style: day_spec::props::ButtonStyleSpec::Automatic,
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

    /// The platform's accent-filled / default-action button (iOS bordered-prominent, macOS
    /// return-key blue, GTK suggested-action, WinUI accent style). Use for the one primary
    /// action of a view.
    pub fn prominent(mut self) -> Self {
        self.native_style = day_spec::props::ButtonStyleSpec::Prominent;
        self
    }

    /// Render this button with a custom [`ButtonStyle`] (the SwiftUI `.buttonStyle(_)` analog):
    /// the style's `body` builds the visual from the button's label, and the button's action is
    /// wired via [`Decorate::on_tap`] on the composed result — a COMPOSED tappable view rather
    /// than the default native `button` leaf. The unstyled [`button`] keeps the native leaf.
    ///
    /// v1 has no pressed/hover state: the styled body is static. `s.label_color()` (if any) tints
    /// the label before `body` sees it (`body` receives it type-erased and cannot recolor it).
    pub fn style(self, s: impl ButtonStyle + 'static) -> AnyPiece {
        let Button { title, action, .. } = self;
        let lbl = Label {
            text: title,
            font: Font::Body,
            weight: None,
            italic: false,
            color: s.label_color(),
        };
        let styled = s.body(lbl.any());
        match action {
            Some(action) => styled.on_tap(move || action()),
            None => styled,
        }
    }
}

/// A pluggable button appearance (the SwiftUI `ButtonStyle` analog). Pure composition — a style
/// builds its body from existing pieces/decorators, so it needs no per-backend native code.
/// Apply one with [`Button::style`].
pub trait ButtonStyle {
    /// Build the button's visual body from its (type-erased) `label` piece.
    fn body(&self, label: AnyPiece) -> AnyPiece;
    /// An optional label color applied by [`Button::style`] BEFORE the label reaches `body`.
    /// Defaults to `None` (the label keeps its intrinsic color). A filled/colored style overrides
    /// this to guarantee contrast — since `body` gets the label type-erased and cannot recolor it,
    /// this is the seam through which a style tints its label. [`FilledButtonStyle`] returns white.
    fn label_color(&self) -> Option<day_spec::Color> {
        None
    }
}

/// A filled, rounded button style: a solid `color` background with a white label and comfortable
/// padding, composed from [`Decorate::padding`]/[`Decorate::background`]/[`Decorate::corner_radius`].
/// v1 is static (no pressed/hover feedback).
pub struct FilledButtonStyle {
    pub color: day_spec::Color,
}

impl ButtonStyle for FilledButtonStyle {
    fn body(&self, label: AnyPiece) -> AnyPiece {
        label
            .padding(Insets::symmetric(16.0, 8.0))
            .background(self.color)
            .corner_radius(8.0)
    }
    fn label_color(&self) -> Option<day_spec::Color> {
        Some(day_spec::Color::WHITE)
    }
}

impl Piece for Button {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.title.initial();
        let node = cx.leaf(
            kinds::BUTTON,
            &ButtonProps {
                title: initial,
                enabled: true,
                style: self.native_style,
            },
            Flex::default(),
        );
        if let Some(action) = self.action {
            cx.on(node, move |ev| {
                if matches!(ev, Event::Pressed) {
                    action();
                }
            });
        }
        self.title
            .bind_to(node, |t| Box::new(ButtonPatch::Title(t)), true);
        node
    }
}

pub struct Toggle<S: SignalRw<bool>> {
    value: S,
}

pub fn toggle<S: SignalRw<bool>>(value: S) -> Toggle<S> {
    Toggle { value }
}

impl<S: SignalRw<bool>> Piece for Toggle<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.value.get_untracked_rw();
        let node = cx.leaf(
            kinds::TOGGLE,
            &ToggleProps {
                on: initial,
                enabled: true,
            },
            Flex::default(),
        );
        let v = self.value.clone();
        bind_seeded(
            initial,
            move || v.get_rw(),
            move |on: &bool| {
                with_tree(|t| t.patch(node, Box::new(TogglePatch::On(*on)), false));
            },
        );
        let v = self.value;
        cx.on(node, move |ev| {
            if let Event::ToggleChanged(on) = ev {
                v.set_rw(*on);
            }
        });
        node
    }
}

pub struct Slider<S: SignalRw<f64>> {
    value: S,
    min: f64,
    max: f64,
    step: Option<f64>,
}

pub fn slider<S: SignalRw<f64>>(value: S) -> Slider<S> {
    Slider {
        value,
        min: 0.0,
        max: 1.0,
        step: None,
    }
}

impl<S: SignalRw<f64>> Slider<S> {
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

impl<S: SignalRw<f64>> Piece for Slider<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.value.get_untracked_rw();
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
            move || v.get_rw(),
            move |val: &f64| {
                with_tree(|t| t.patch(node, Box::new(SliderPatch::Value(*val)), false));
            },
        );
        let v = self.value;
        let (step, min, max) = (self.step, self.min, self.max);
        cx.on(node, move |ev| {
            if let Event::ValueChanged(val) = ev {
                // Honor `.step(_)` at the framework layer so EVERY backend produces stepped values —
                // several native sliders (e.g. iOS `UISlider`) have no native step and emit a
                // continuous stream while dragging. Snapping here keeps the bound signal (and the
                // thumb, via `bind_seeded` above) on the step grid, and stops a `.step`-bound consumer
                // from being hammered ~60×/s with sub-step deltas during a drag.
                let snapped = match step {
                    Some(s) if s > 0.0 => (min + ((val - min) / s).round() * s).clamp(min, max),
                    _ => *val,
                };
                v.set_rw(snapped);
            }
        });
        node
    }
}

pub struct TextField<S: SignalRw<String>> {
    value: S,
    placeholder: Option<TextSource>,
    on_submit: Option<Rc<dyn Fn()>>,
}

pub fn text_field<S: SignalRw<String>>(value: S) -> TextField<S> {
    TextField {
        value,
        placeholder: None,
        on_submit: None,
    }
}

impl<S: SignalRw<String>> TextField<S> {
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

impl<S: SignalRw<String>> Piece for TextField<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let initial = self.value.get_untracked_rw();
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
            move || v.get_rw(),
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
        cx.on(node, move |ev| match ev {
            Event::TextChanged(t) => {
                *guard.borrow_mut() = Some(t.clone());
                v.set_rw(t.clone());
            }
            Event::Submitted => {
                if let Some(f) = &submit {
                    f();
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
