//! day-pieces — the built-in piece library (DESIGN.md §5.3).
//!
//! Every constructor is a plain function returning a piece value; builder methods configure;
//! `build` runs once. Dynamic attributes become seeded bindings writing sparse typed patches
//! through the thread-local tree.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal, bind, bind_seeded, watch};
use day_spec::props::*;
use day_spec::{
    A11yProps, AnimSpec, Color, DrawOp, Event, Font, Insets, LinearGradient, Paint, Point,
    RadialGradient, Rect, Role, Shape, Size, Transform, UnitPoint, kinds,
};

// External-piece registration surface (§8.2): the `renderer!` macro + `fill_measure`, plus the
// re-exports the macro expands to (so a piece needs only a `day-pieces` dependency, not linkme).
pub mod render;
pub use day_spec::Renderer;
pub use linkme;
pub use render::fill_measure;

// ---------------------------------------------------------------------------
// Text sources (§12.2's IntoText, M1 subset — Fluent joins at M6)
// ---------------------------------------------------------------------------

pub enum TextSource {
    Static(String),
    Dyn(Rc<dyn Fn() -> String>),
}

impl TextSource {
    pub fn initial(&self) -> String {
        match self {
            TextSource::Static(s) => s.clone(),
            TextSource::Dyn(f) => day_reactive::untrack(|| f()),
        }
    }
    /// Install the text binding for a realized node (no-op for static text).
    fn bind_to(
        self,
        node: RNode,
        make_patch: impl Fn(String) -> Box<dyn std::any::Any> + 'static,
        affects_size: bool,
    ) {
        if let TextSource::Dyn(f) = self {
            let seed = day_reactive::untrack(|| f());
            bind_seeded(
                seed,
                move || f(),
                move |t: &String| {
                    let patch = make_patch(t.clone());
                    with_tree(|tr| tr.patch(node, patch, affects_size));
                },
            );
        }
    }
}

/// Disjoint-marker conversion (the coherent form of §12.2's IntoText):
/// literals, `String`, `Signal<String>`, and closures all convert, each under its own marker.
pub trait IntoText<M> {
    fn into_text(self) -> TextSource;
}

pub struct StaticMark;
pub struct SignalMark;
pub struct FnMark;

/// A focus-binding target for [`Decorate::focused`] (docs/focus.md): either a `Signal<bool>`
/// (one control) or a `(Signal<Option<K>>, K)` pair (one control of a group sharing a signal).
/// The two-marker split is the same E0119 dodge as [`IntoText`].
pub trait IntoFocusBinding<M> {
    /// Split into (desired-focus tracked read, native-change write-back).
    #[allow(clippy::type_complexity)]
    fn into_focus_binding(self) -> (Box<dyn Fn() -> bool>, Box<dyn Fn(bool)>);
}

pub struct FocusBoolMark;
pub struct FocusGroupMark;

impl IntoFocusBinding<FocusBoolMark> for Signal<bool> {
    fn into_focus_binding(self) -> (Box<dyn Fn() -> bool>, Box<dyn Fn(bool)>) {
        (Box::new(move || self.get()), Box::new(move |f| self.set(f)))
    }
}

impl<K: Copy + PartialEq + 'static> IntoFocusBinding<FocusGroupMark> for (Signal<Option<K>>, K) {
    fn into_focus_binding(self) -> (Box<dyn Fn() -> bool>, Box<dyn Fn(bool)>) {
        let (sig, key) = self;
        (
            Box::new(move || sig.get() == Some(key)),
            Box::new(move |f| {
                if f {
                    sig.set(Some(key));
                } else if sig.get_untracked() == Some(key) {
                    // Only clear if the signal still names THIS control — when focus moved to a
                    // sibling, the paired gain (dispatched first, docs/focus.md) already wrote
                    // the new value and the group signal never passes through `None`.
                    sig.set(None);
                }
            }),
        )
    }
}

impl IntoText<StaticMark> for &str {
    fn into_text(self) -> TextSource {
        TextSource::Static(self.to_owned())
    }
}
impl IntoText<StaticMark> for String {
    fn into_text(self) -> TextSource {
        TextSource::Static(self)
    }
}
impl IntoText<SignalMark> for Signal<String> {
    fn into_text(self) -> TextSource {
        TextSource::Dyn(Rc::new(move || self.get()))
    }
}
impl<F, S> IntoText<FnMark> for F
where
    F: Fn() -> S + 'static,
    S: Into<String>,
{
    fn into_text(self) -> TextSource {
        TextSource::Dyn(Rc::new(move || self().into()))
    }
}

// ---------------------------------------------------------------------------
// Fraction sources — the read-only numeric analogue of TextSource, for `progress`.
// ---------------------------------------------------------------------------

pub enum FractionSource {
    Static(f64),
    Dyn(Rc<dyn Fn() -> f64>),
}

impl FractionSource {
    /// Untracked seed value, clamped to `0.0..=1.0`.
    pub fn initial(&self) -> f64 {
        let v = match self {
            FractionSource::Static(v) => *v,
            FractionSource::Dyn(f) => day_reactive::untrack(|| f()),
        };
        v.clamp(0.0, 1.0)
    }
    /// Install the fraction binding (no-op for static values). Writes are clamped so a
    /// backend never sees an out-of-range fraction.
    fn bind_to(self, node: RNode) {
        if let FractionSource::Dyn(f) = self {
            let seed = day_reactive::untrack(|| f()).clamp(0.0, 1.0);
            bind_seeded(
                seed,
                move || f().clamp(0.0, 1.0),
                move |v: &f64| {
                    with_tree(|tr| tr.patch(node, Box::new(ProgressPatch::Value(Some(*v))), false));
                },
            );
        }
    }
}

/// Disjoint-marker conversion (like [`IntoText`]) so `progress(_)` accepts a constant `f64`,
/// a `Signal<f64>`, or a closure. Reuses the same marker types.
pub trait IntoFraction<M> {
    fn into_fraction(self) -> FractionSource;
}

impl IntoFraction<StaticMark> for f64 {
    fn into_fraction(self) -> FractionSource {
        FractionSource::Static(self)
    }
}
impl IntoFraction<SignalMark> for Signal<f64> {
    fn into_fraction(self) -> FractionSource {
        FractionSource::Dyn(Rc::new(move || self.get()))
    }
}
impl<F> IntoFraction<FnMark> for F
where
    F: Fn() -> f64 + 'static,
{
    fn into_fraction(self) -> FractionSource {
        FractionSource::Dyn(Rc::new(self))
    }
}

// ---------------------------------------------------------------------------
// Two-way binding surface (§5.3)
// ---------------------------------------------------------------------------

pub trait SignalRw<T: 'static>: Clone + 'static {
    /// Tracked read.
    fn get_rw(&self) -> T;
    fn get_untracked_rw(&self) -> T;
    fn set_rw(&self, v: T);
}

impl<T: Clone + 'static> SignalRw<T> for Signal<T> {
    fn get_rw(&self) -> T {
        self.get()
    }
    fn get_untracked_rw(&self) -> T {
        self.get_untracked()
    }
    fn set_rw(&self, v: T) {
        self.set(v);
    }
}

// ---------------------------------------------------------------------------
// Leaves
// ---------------------------------------------------------------------------

pub struct Label {
    text: TextSource,
    font: Font,
    weight: Option<day_spec::FontWeight>,
    italic: bool,
    color: Option<day_spec::Color>,
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

// ---------------------------------------------------------------------------
// Containers
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub enum HAlign {
    Leading,
    #[default]
    Center,
    Trailing,
}
#[derive(Clone, Copy, Default)]
pub enum VAlign {
    Top,
    #[default]
    Center,
    Bottom,
}

pub struct Column<C: PieceSeq> {
    children: C,
    spacing: f64,
    align: CrossAlign,
}

pub fn column<C: PieceSeq>(children: C) -> Column<C> {
    Column {
        children,
        spacing: 0.0,
        align: CrossAlign::Center,
    }
}

impl<C: PieceSeq> Column<C> {
    pub fn spacing(mut self, s: f64) -> Self {
        self.spacing = s;
        self
    }
    pub fn align(mut self, a: HAlign) -> Self {
        self.align = match a {
            HAlign::Leading => CrossAlign::Leading,
            HAlign::Center => CrossAlign::Center,
            HAlign::Trailing => CrossAlign::Trailing,
        };
        self
    }
}

impl<C: PieceSeq> Piece for Column<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(StackLayout {
                axis: Axis::Vertical,
                spacing: self.spacing,
                align: self.align,
            }),
            Flex::default(),
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

pub struct Row<C: PieceSeq> {
    children: C,
    spacing: f64,
    align: CrossAlign,
}

pub fn row<C: PieceSeq>(children: C) -> Row<C> {
    Row {
        children,
        spacing: 0.0,
        align: CrossAlign::Center,
    }
}

impl<C: PieceSeq> Row<C> {
    pub fn spacing(mut self, s: f64) -> Self {
        self.spacing = s;
        self
    }
    pub fn align(mut self, a: VAlign) -> Self {
        self.align = match a {
            VAlign::Top => CrossAlign::Leading,
            VAlign::Center => CrossAlign::Center,
            VAlign::Bottom => CrossAlign::Trailing,
        };
        self
    }
}

impl<C: PieceSeq> Piece for Row<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(StackLayout {
                axis: Axis::Horizontal,
                spacing: self.spacing,
                align: self.align,
            }),
            Flex::default(),
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

pub struct Grid<C: PieceSeq> {
    children: C,
    row_spacing: f64,
    column_spacing: f64,
    align: Alignment,
}

/// A SwiftUI-style eager grid (docs/grid.md): columns are inferred from [`grid_row`] children —
/// a column is as wide as its widest cell, a `grow_w` cell makes its column share the leftover
/// width evenly, and a non-row child becomes a full-width cell spanning every column. `spacer()`
/// inside a row is an inert empty cell that still occupies its column (a grid has explicit
/// gutters, so stack-style push-apart spacers don't apply). Cells opt into spans and per-cell
/// alignment with [`Decorate::grid_span`] / [`Decorate::grid_align`].
pub fn grid<C: PieceSeq>(children: C) -> Grid<C> {
    Grid {
        children,
        row_spacing: 0.0,
        column_spacing: 0.0,
        align: Alignment::Center,
    }
}

impl<C: PieceSeq> Grid<C> {
    /// Set both the row and column gutters.
    pub fn spacing(mut self, s: f64) -> Self {
        self.row_spacing = s;
        self.column_spacing = s;
        self
    }
    pub fn row_spacing(mut self, s: f64) -> Self {
        self.row_spacing = s;
        self
    }
    pub fn column_spacing(mut self, s: f64) -> Self {
        self.column_spacing = s;
        self
    }
    /// Default alignment of every cell within its cell rect (cell/row overrides win).
    pub fn align(mut self, a: Alignment) -> Self {
        self.align = a;
        self
    }
}

impl<C: PieceSeq> Piece for Grid<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(GridLayout {
                row_spacing: self.row_spacing,
                column_spacing: self.column_spacing,
                align: self.align,
            }),
            Flex::default(),
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

pub struct GridRow<C: PieceSeq> {
    children: C,
    valign: Option<CrossAlign>,
}

/// One row of a [`grid`]: each child is a cell, assigned to columns left to right. Outside a
/// grid a row degrades gracefully to a plain [`row`]. Rows are transparent carriers — the grid
/// places their cells directly — so decorating a `grid_row` itself is unsupported (decorate the
/// cells, or the grid).
pub fn grid_row<C: PieceSeq>(children: C) -> GridRow<C> {
    GridRow {
        children,
        valign: None,
    }
}

impl<C: PieceSeq> GridRow<C> {
    /// Vertical alignment override for this row's cells (the grid's alignment applies otherwise).
    pub fn align(mut self, a: VAlign) -> Self {
        self.valign = Some(match a {
            VAlign::Top => CrossAlign::Leading,
            VAlign::Center => CrossAlign::Center,
            VAlign::Bottom => CrossAlign::Trailing,
        });
        self
    }
}

impl<C: PieceSeq> Piece for GridRow<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        // A layout-only node whose StackLayout only runs when the row is NOT inside a grid
        // (the graceful-degrade path) — a grid introspects the cells and places them itself.
        let node = cx.layout_only(
            Rc::new(StackLayout {
                axis: Axis::Horizontal,
                spacing: 0.0,
                align: self.valign.unwrap_or_default(),
            }),
            Flex {
                grid: GridFacts {
                    is_row: true,
                    row_valign: self.valign,
                    ..Default::default()
                },
                ..Default::default()
            },
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

pub struct Scroll<P: Piece> {
    child: P,
    axis: Axis,
    target: Option<Signal<Option<day_core::ScrollTarget>>>,
}

pub fn scroll<P: Piece>(child: P) -> Scroll<P> {
    Scroll {
        child,
        axis: Axis::Vertical,
        target: None,
    }
}

impl<P: Piece> Scroll<P> {
    /// Scroll horizontally instead of vertically (a filmstrip of cards, a chip row). The content
    /// is measured unconstrained on the horizontal axis and the native view scrolls sideways.
    pub fn horizontal(mut self) -> Self {
        self.axis = Axis::Horizontal;
        self
    }
    /// Set the scroll axis explicitly.
    pub fn axis(mut self, axis: Axis) -> Self {
        self.axis = axis;
        self
    }

    /// Programmatic scrolling (docs/scroll.md): each `Some(target)` written to `sig` scrolls
    /// there (animated), then the signal resets to `None` — write-and-forget, so the same
    /// target can be sent twice in a row.
    ///
    /// ```ignore
    /// let jump = Signal::new(None);
    /// scroll(rows).scroll_target(jump);
    /// button("Bottom").action(move || jump.set(Some(ScrollTarget::Bottom)));
    /// ```
    pub fn scroll_target(mut self, sig: Signal<Option<day_core::ScrollTarget>>) -> Self {
        self.target = Some(sig);
        self
    }
}

impl<P: Piece> Piece for Scroll<P> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::SCROLL,
            &day_spec::props::ScrollProps {
                horizontal: matches!(self.axis, Axis::Horizontal),
            },
            Rc::new(ScrollLayout { axis: self.axis }),
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
            Boundary::Yes, // scroll viewports are layout boundaries (§7.4)
        );
        cx.under(node, |cx| {
            let _ = self.child.build(cx);
        });
        if let Some(sig) = self.target {
            watch(
                move || sig.get(),
                move |now, _| {
                    if let Some(t) = now.clone() {
                        day_core::with_tree(|tr| {
                            tr.scroll_to_target(node, &t, true);
                        });
                        sig.set(None); // consumed — ready for the next command
                    }
                },
            );
        }
        node
    }
}

/// A z-stack: children are layered back-to-front (the first child sits at the bottom), all
/// sharing the container bounds and positioned by the stack's [`Alignment`]. The stack sizes to
/// the UNION (max width/height) of its children — contrast [`Decorate::overlay`], which sizes to
/// its content and treats the overlaid piece as a non-sizing annotation. Pure composition: it is
/// the same native panel as [`column`]/[`row`], so there is no per-backend work.
pub struct ZStack<C: PieceSeq> {
    children: C,
    align: Alignment,
}

/// Build a [`ZStack`] from a tuple of children (or a [`PieceVec`]).
pub fn zstack<C: PieceSeq>(children: C) -> ZStack<C> {
    ZStack {
        children,
        align: Alignment::Center,
    }
}

impl<C: PieceSeq> ZStack<C> {
    /// Where children sit within the stack's bounds (default [`Alignment::Center`]).
    pub fn align(mut self, a: Alignment) -> Self {
        self.align = a;
        self
    }
}

impl<C: PieceSeq> Piece for ZStack<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::CONTAINER,
            &ContainerProps::default(),
            Rc::new(OverlayLayout {
                align: self.align,
                size_to_first: false,
            }),
            Flex::default(),
            Boundary::No,
        );
        cx.under(node, |cx| self.children.build_each(cx));
        node
    }
}

// ---------------------------------------------------------------------------
// Structure: when / each (§5.3–§5.4)
// ---------------------------------------------------------------------------

/// Reactive conditional subtree. The anchor is a layout-transparent group; the active arm
/// lives in its own child scope, disposed on switch (§4.3).
pub fn when<P: Piece>(
    cond: impl Fn() -> bool + 'static,
    build_arm: impl Fn() -> P + 'static,
) -> AnyPiece {
    piece_fn(move |cx| {
        let anchor = cx.layout_only(
            Rc::new(PassThrough),
            Flex {
                is_group: true,
                ..Default::default()
            },
            Boundary::No,
        );
        let state: Rc<RefCell<Option<Scope>>> = Rc::new(RefCell::new(None));
        let build_arm = Rc::new(build_arm);

        let mount = {
            let state = state.clone();
            let build_arm = build_arm.clone();
            move |on: bool| {
                if on {
                    let scope = Scope::child();
                    scope.enter(|| {
                        let mut cx = BuildCx::new(anchor);
                        let _ = build_arm().build(&mut cx);
                    });
                    *state.borrow_mut() = Some(scope);
                } else if let Some(scope) = state.borrow_mut().take() {
                    scope.dispose();
                    // Remove everything under the anchor.
                    while with_tree(|t| t.child_count(anchor)) > 0 {
                        let child = with_tree(|t| t.first_child(anchor));
                        match child {
                            Some(c) => with_tree(|t| t.remove_subtree(c)),
                            None => break,
                        }
                    }
                }
            }
        };

        let initial = day_reactive::untrack(&cond);
        mount(initial);
        watch(cond, move |now, old| {
            if Some(now) != old {
                mount(*now);
            }
        });
        anchor
    })
}

/// A `Copy` handle to one keyed item's state — the unified `each`/`list` contract (§5.4).
pub struct ItemSlot<T: 'static, K: 'static> {
    sig: Signal<T>,
    key: Signal<K>,
}

impl<T: 'static, K: 'static> Clone for ItemSlot<T, K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static, K: 'static> Copy for ItemSlot<T, K> {}

impl<T: Clone + 'static, K: Clone + 'static> ItemSlot<T, K> {
    /// Tracked whole-item read. **Read it inside a reactive closure** — e.g.
    /// `label(move || slot.get())` — not eagerly. A recycling [`list`] rebinds one physical row to
    /// many items, and only bindings that read the slot reactively update on rebind; an eager
    /// `let name = slot.get()` freezes the row at its first item.
    pub fn get(self) -> T {
        self.sig.get()
    }
    /// Tracked read via a projection. Read it inside a reactive closure (see [`get`](Self::get)).
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        self.sig.with(f)
    }
    /// Tracked field projection (equality-gating happens in the binding layer). Read it inside a
    /// reactive closure (see [`get`](Self::get)) so recycled rows update on rebind.
    pub fn field<V: Clone>(self, f: impl FnOnce(&T) -> V) -> V {
        self.sig.with(f)
    }
    pub fn key(self) -> K {
        self.key.get_untracked()
    }
}

/// Type-erased slot writer: feeds a surviving row's `ItemSlot` signal a new `&T` (§5.4).
type SlotWriter = Box<dyn Fn(&dyn std::any::Any)>;

struct EachRow<K> {
    key: K,
    scope: Scope,
    root: RNode,
    sig_set: SlotWriter,
}

/// Reactive keyed collection (§5.4): keyed diff, per-key child scopes, slot writes for
/// surviving keys, debug key-uniqueness assertion.
pub fn each<T, K, P>(
    items: impl Fn() -> Vec<T> + 'static,
    key_of: impl Fn(&T) -> K + 'static,
    build_row: impl Fn(ItemSlot<T, K>) -> P + 'static,
) -> AnyPiece
where
    T: Clone + 'static,
    K: Eq + Hash + Clone + 'static,
    P: Piece,
{
    piece_fn(move |cx| {
        let anchor = cx.layout_only(
            Rc::new(PassThrough),
            Flex {
                is_group: true,
                ..Default::default()
            },
            Boundary::No,
        );
        let rows: Rc<RefCell<Vec<EachRow<K>>>> = Rc::new(RefCell::new(Vec::new()));
        let key_of = Rc::new(key_of);
        let build_row = Rc::new(build_row);

        let sync = {
            let rows = rows.clone();
            let key_of = key_of.clone();
            let build_row = build_row.clone();
            move |new_items: &Vec<T>| {
                let new_keys: Vec<K> = new_items.iter().map(|t| key_of(t)).collect();
                if cfg!(debug_assertions) {
                    let mut seen = HashSet::new();
                    for k in &new_keys {
                        assert!(seen.insert(k.clone()), "day: duplicate key in `each` diff");
                    }
                }
                let mut old = std::mem::take(&mut *rows.borrow_mut());
                let mut next: Vec<EachRow<K>> = Vec::with_capacity(new_keys.len());
                for (item, k) in new_items.iter().zip(new_keys.iter()) {
                    if let Some(pos) = old.iter().position(|r| &r.key == k) {
                        let row = old.remove(pos);
                        // Surviving key: one unconditional slot write (§5.4).
                        (row.sig_set)(item as &dyn std::any::Any);
                        next.push(row);
                    } else {
                        let scope = Scope::child();
                        let (root, sig) = scope.enter(|| {
                            let sig = Signal::new(item.clone());
                            let keysig = Signal::new(k.clone());
                            let slot = ItemSlot { sig, key: keysig };
                            let mut cx = BuildCx::new(anchor);
                            (build_row(slot).build(&mut cx), sig)
                        });
                        next.push(EachRow {
                            key: k.clone(),
                            scope,
                            root,
                            sig_set: Box::new(move |any| {
                                if let Some(v) = any.downcast_ref::<T>() {
                                    sig.set(v.clone());
                                }
                            }),
                        });
                    }
                }
                // Removals.
                for row in old {
                    row.scope.dispose();
                    with_tree(|t| t.remove_subtree(row.root));
                }
                // Order: reattach in the new sequence.
                let order: Vec<RNode> = next.iter().map(|r| r.root).collect();
                with_tree(|t| t.reorder_children(anchor, order));
                *rows.borrow_mut() = next;
            }
        };

        let initial = day_reactive::untrack(&items);
        sync(&initial);
        watch(items, move |new, _| sync(new));
        anchor
    })
}

// ---------------------------------------------------------------------------
// @Environment — ambient values over day-reactive's scope context (§4.3). No backend work.
// ---------------------------------------------------------------------------

/// Provide an ambient value `T` to `content` and its ENTIRE descendant subtree (the SwiftUI
/// `@Environment`/`.environment(_)` analog, layered over day-reactive's scope context). `content`
/// — and any piece built within it — reads it back with [`environment`]. A thin, non-reactive
/// wrapper: `T` is a snapshot captured here; for a value that must react, provide a `Signal<T>`
/// (or a `Memo<T>`) and read it reactively inside the subtree.
///
/// ```ignore
/// #[derive(Clone)] struct Theme { accent: Color }
/// with_environment(Theme { accent: BLUE }, || my_screen())
/// // deep inside my_screen():  let accent = environment::<Theme>().unwrap().accent;
/// ```
pub fn with_environment<T: Clone + 'static>(
    value: T,
    content: impl FnOnce() -> AnyPiece + 'static,
) -> AnyPiece {
    piece_fn(move |cx| {
        // A child scope carrying `T`, entered for the whole of `content`'s construction AND build,
        // so both `content`'s own body and every descendant piece's build resolve it via
        // `use_context` (which walks scope → ancestors). Owned by the current build scope, so it is
        // disposed with the enclosing subtree (e.g. a `when` arm) exactly like `when`/`each` scopes.
        let scope = Scope::child();
        scope.provide(value);
        scope.enter(|| content().build(cx))
    })
}

/// Read the nearest ambient `T` provided by an enclosing [`with_environment`], or `None` if none is
/// in scope. Call it while constructing or building a piece within that subtree.
pub fn environment<T: Clone + 'static>() -> Option<T> {
    Scope::current().use_context::<T>()
}

// ---------------------------------------------------------------------------
// `list` — native recycling list (docs/list.md, §10)
// ---------------------------------------------------------------------------

/// Stable u64 identity token for a key, for the native list's diffing.
fn key_token<K: Hash>(k: &K) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    k.hash(&mut h);
    h.finish()
}

/// Applies a fresh items snapshot (refresh the data-source view + tell the native host to reload).
type RefreshFn<T> = Rc<dyn Fn(&Vec<T>)>;

/// A native recycling list: the platform widget owns scrolling + cell reuse; Day builds each
/// visible row once and *rebinds* it (a slot-write into its `ItemSlot`) as cells recycle.
/// Shares the `ItemSlot` row contract with [`each`]; migrating is a one-word change.
pub struct List<T: 'static, K: 'static> {
    items: Rc<dyn Fn() -> Vec<T>>,
    key_of: Rc<dyn Fn(&T) -> K>,
    build_row: Rc<dyn Fn(ItemSlot<T, K>) -> AnyPiece>,
    row_height: RowHeight,
    on_select: Option<Rc<dyn Fn(K)>>,
    scroll_to_end: Option<day_reactive::Trigger>,
    stick_to_bottom: bool,
}

/// Build a recycling list from a reactive items closure, a key function, and a row builder.
pub fn list<T, K, P>(
    items: impl Fn() -> Vec<T> + 'static,
    key_of: impl Fn(&T) -> K + 'static,
    build_row: impl Fn(ItemSlot<T, K>) -> P + 'static,
) -> List<T, K>
where
    T: Clone + 'static,
    K: Clone + Hash + 'static,
    P: Piece,
{
    List {
        items: Rc::new(items),
        key_of: Rc::new(key_of),
        build_row: Rc::new(move |slot| AnyPiece::new(build_row(slot))),
        row_height: RowHeight::Automatic,
        on_select: None,
        scroll_to_end: None,
        stick_to_bottom: false,
    }
}

impl<T: Clone + 'static, K: Clone + Hash + 'static> List<T, K> {
    /// Row sizing: `Uniform(h)` (fastest) or `Automatic` (self-sizing).
    pub fn row_height(mut self, h: RowHeight) -> Self {
        self.row_height = h;
        self
    }
    /// Called with the selected row's key when the native list reports a selection.
    pub fn on_select(mut self, f: impl Fn(K) + 'static) -> Self {
        self.on_select = Some(Rc::new(f));
        self
    }
    /// Scroll the list so its LAST row is fully visible whenever `trigger` fires — e.g. a chat
    /// timeline sticking to the newest message. Fire it with [`day_reactive::Trigger::notify`]
    /// after appending. No-op while the list is empty. The scroll targets the native list
    /// (`NSTableView`/`UITableView`/`GtkListView`/`QListView`/`RecyclerView`), so it respects the
    /// platform's own scroll physics.
    pub fn scroll_to_end(mut self, trigger: day_reactive::Trigger) -> Self {
        self.scroll_to_end = Some(trigger);
        self
    }
    /// Best-effort auto-stick: after a data reload, scroll to the end so freshly appended rows stay
    /// visible. Convenience over [`Self::scroll_to_end`] for feeds that always follow the newest
    /// row; for finer control (only stick when the user is already near the bottom) drive
    /// `scroll_to_end` from your own logic instead. Off by default.
    pub fn stick_to_bottom(mut self, on: bool) -> Self {
        self.stick_to_bottom = on;
        self
    }
}

impl<T: Clone + 'static, K: Clone + Hash + 'static> Piece for List<T, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let props = ListProps {
            row_height: self.row_height,
            selectable: self.on_select.is_some(),
        };
        let node = cx.leaf(
            kinds::LIST,
            &props,
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        );

        // The data-source's view of the world: the current items + their tokens, refreshed by a
        // bind on the items closure. The native host queries these synchronously; the driver's
        // build/rebind closures read the same snapshot.
        let snapshot: Rc<RefCell<Vec<T>>> = Rc::new(RefCell::new(Vec::new()));
        let tokens: Rc<RefCell<Vec<u64>>> = Rc::new(RefCell::new(Vec::new()));

        // Selection → key (translate the native row index through the snapshot).
        if let Some(on_select) = self.on_select.clone() {
            let (snap, key_of) = (snapshot.clone(), self.key_of.clone());
            cx.on(node, move |ev| {
                if let Event::SelectionChanged(i) = ev
                    && let Some(item) = snap.borrow().get(*i as usize)
                {
                    on_select(key_of(item));
                }
            });
        }

        // The type-erased driver day-core drives on cell pulls.
        let driver = ListDriver {
            row_height: self.row_height,
            len: {
                let s = snapshot.clone();
                Box::new(move || s.borrow().len())
            },
            token_at: {
                let t = tokens.clone();
                Box::new(move |i| t.borrow().get(i).copied().unwrap_or(0))
            },
            build: {
                let (snapshot, key_of, build_row) = (
                    snapshot.clone(),
                    self.key_of.clone(),
                    self.build_row.clone(),
                );
                Box::new(move |index, anchor| {
                    let scope = Scope::child();
                    let rebind = scope.enter(|| {
                        let item = snapshot.borrow()[index].clone();
                        let sig = Signal::new(item.clone());
                        let keysig = Signal::new(key_of(&item));
                        let slot = ItemSlot { sig, key: keysig };
                        let mut rowcx = BuildCx::new(anchor);
                        build_row(slot).build(&mut rowcx);
                        // Rebind on recycle: one slot-write of the new row's item + key.
                        let (snap, key_of) = (snapshot.clone(), key_of.clone());
                        Rc::new(move |i: usize| {
                            let it = snap.borrow()[i].clone();
                            keysig.set(key_of(&it));
                            sig.set(it);
                        }) as Rc<dyn Fn(usize)>
                    });
                    BuiltRow { scope, rebind }
                })
            },
        };
        install_list(node, driver);

        // Keep the snapshot current and tell the native host to re-query on every change.
        // `watch` (not `bind`) so `T` need not be `PartialEq` — matching `each`; run once eagerly.
        let refresh: RefreshFn<T> = {
            let (snapshot, tokens, key_of) =
                (snapshot.clone(), tokens.clone(), self.key_of.clone());
            Rc::new(move |its: &Vec<T>| {
                *tokens.borrow_mut() = its.iter().map(|t| key_token(&key_of(t))).collect();
                *snapshot.borrow_mut() = its.clone();
                list_reload(node);
            })
        };
        let items = self.items.clone();
        let initial = day_reactive::untrack(|| items());
        refresh(&initial);
        {
            // On subsequent data changes: reload, then (if sticking) follow the newest row. The
            // initial eager `refresh` above deliberately does NOT auto-scroll.
            let (refresh, items, stick) = (refresh.clone(), items.clone(), self.stick_to_bottom);
            watch(
                move || items(),
                move |new: &Vec<T>, _| {
                    refresh(new);
                    if stick {
                        list_scroll_to_end(node);
                    }
                },
            );
        }

        // Imperative scroll-to-end: each `trigger.notify()` re-runs this watch (the trigger's
        // signal is the only tracked dep), whose callback scrolls the native list to its last row.
        // `watch` never fires for the initial run, so building the list does not force a scroll.
        if let Some(trigger) = self.scroll_to_end {
            watch(
                move || trigger.track(),
                move |_: &(), _| list_scroll_to_end(node),
            );
        }
        node
    }
}

// ---------------------------------------------------------------------------
// Menus — the app-side builder over day_spec's toolkit-neutral MenuItem model. Lowering registers each
// item's action closure with day-core (which dispatches `Event::MenuAction`) and assigns its id.
// ---------------------------------------------------------------------------

/// A menu entry under construction. Build a command with [`menu_item`], a nested submenu with
/// [`sub_menu`], a standard system command with [`menu_role`], and a divider with [`menu_separator`].
/// Attach to a Piece via [`Decorate::context_menu`] or install app-wide via [`app_menu`].
pub struct MenuEntry {
    label: String,
    shortcut: Option<day_spec::Shortcut>,
    enabled: bool,
    role: Option<day_spec::MenuRole>,
    action: Option<Rc<dyn Fn()>>,
    children: Option<Vec<MenuEntry>>,
    separator: bool,
}

impl MenuEntry {
    fn command(label: impl Into<String>) -> MenuEntry {
        MenuEntry {
            label: label.into(),
            shortcut: None,
            enabled: true,
            role: None,
            action: None,
            children: None,
            separator: false,
        }
    }
    /// Run `f` when the item is chosen.
    pub fn action(mut self, f: impl Fn() + 'static) -> MenuEntry {
        self.action = Some(Rc::new(f));
        self
    }
    /// Full shortcut spec, e.g. `Shortcut::new("s").shift()`.
    pub fn shortcut(mut self, s: day_spec::Shortcut) -> MenuEntry {
        self.shortcut = Some(s);
        self
    }
    /// Convenience: the platform's primary modifier (⌘ / Ctrl) + `key`.
    pub fn key(mut self, key: impl Into<String>) -> MenuEntry {
        self.shortcut = Some(day_spec::Shortcut::new(key));
        self
    }
    pub fn enabled(mut self, on: bool) -> MenuEntry {
        self.enabled = on;
        self
    }
    /// Tag a custom command with a standard [`day_spec::MenuRole`] (usually you use [`menu_role`]).
    pub fn role(mut self, role: day_spec::MenuRole) -> MenuEntry {
        self.role = Some(role);
        self
    }
}

/// A clickable command: `menu_item("Save").key("s").action(|| …)`.
pub fn menu_item(label: impl Into<String>) -> MenuEntry {
    MenuEntry::command(label)
}

/// A nested submenu: `sub_menu("File", vec![menu_item("New"), …])`.
pub fn sub_menu(label: impl Into<String>, items: Vec<MenuEntry>) -> MenuEntry {
    MenuEntry {
        children: Some(items),
        ..MenuEntry::command(label)
    }
}

/// A visual divider between items.
pub fn menu_separator() -> MenuEntry {
    MenuEntry {
        separator: true,
        ..MenuEntry::command("")
    }
}

/// A standard/system command (`MenuRole::Copy`, `MenuRole::Quit`, …) rendered with the platform's
/// NATIVE item — correct label, default shortcut, focus-targeting, and automatic enable/disable — so
/// default menu items (Edit ▸ Cut/Copy/Paste, the app's Quit/About) work without re-implementation.
pub fn menu_role(role: day_spec::MenuRole) -> MenuEntry {
    MenuEntry {
        role: Some(role),
        ..MenuEntry::command("")
    }
}

/// The core-catalog key for a standard menu command's label (docs/menus.md, docs/localization.md).
fn role_catalog_key(role: day_spec::MenuRole) -> &'static str {
    use day_spec::MenuRole as R;
    match role {
        R::Cut => "day-cut",
        R::Copy => "day-copy",
        R::Paste => "day-paste",
        R::SelectAll => "day-select-all",
        R::Undo => "day-undo",
        R::Redo => "day-redo",
        R::Delete => "day-delete",
        R::About => "day-about",
        R::Quit => "day-quit",
        R::Preferences => "day-preferences",
        R::Minimize => "day-minimize",
        R::CloseWindow => "day-close",
        R::Fullscreen => "day-fullscreen",
    }
}

/// Lower app-side entries to the spec model, registering action closures with day-core. A standard
/// `role` item with no explicit label gets its label from the localized core catalog here — so the
/// backends receive a ready, locale-correct label instead of each hardcoding English (day-l10n).
fn lower_menu(entries: Vec<MenuEntry>) -> Vec<day_spec::MenuItem> {
    entries
        .into_iter()
        .map(|e| {
            if e.separator {
                day_spec::MenuItem::Separator
            } else if let Some(children) = e.children {
                day_spec::MenuItem::Submenu {
                    label: e.label,
                    items: lower_menu(children),
                }
            } else {
                let id = e.action.map(day_core::register_menu_action).unwrap_or(0);
                let label = match (e.label.is_empty(), e.role) {
                    (true, Some(role)) => day_l10n::t(role_catalog_key(role)),
                    _ => e.label,
                };
                day_spec::MenuItem::Action {
                    id,
                    label,
                    shortcut: e.shortcut,
                    enabled: e.enabled,
                    role: e.role,
                }
            }
        })
        .collect()
}

/// Install the application menu — the native menu bar on desktop, the app-bar overflow on Android, the
/// UIMenuBuilder main menu on iPadOS/Catalyst. Top-level entries are usually `sub_menu(...)`s (the
/// menu-bar menus). Call at startup or whenever the menu changes; it replaces any previous app menu.
pub fn app_menu(menus: Vec<MenuEntry>) {
    day_core::set_app_menu(lower_menu(menus));
}

// ---------------------------------------------------------------------------
// Decorators (§5.2 Decorate)
// ---------------------------------------------------------------------------

pub trait IntoInsets {
    fn into_insets(self) -> Insets;
}
impl IntoInsets for f64 {
    fn into_insets(self) -> Insets {
        Insets::all(self)
    }
}
impl IntoInsets for Insets {
    fn into_insets(self) -> Insets {
        self
    }
}

/// A one-shot, by-value view transform (the SwiftUI `ViewModifier` analog): wrap a piece into a
/// new one. Pure composition — no per-backend work. A plain `FnOnce(AnyPiece) -> AnyPiece` closure
/// is a `Modifier` too (the blanket impl below), so the common case needs no new type. Apply one
/// with [`Decorate::modifier`].
pub trait Modifier {
    fn apply(self, content: AnyPiece) -> AnyPiece;
}

impl<F> Modifier for F
where
    F: FnOnce(AnyPiece) -> AnyPiece,
{
    fn apply(self, content: AnyPiece) -> AnyPiece {
        self(content)
    }
}

/// A liveness-checked reference to a mounted piece's realized node — the retained half of the
/// tweaks API (docs/tweaks.md). Capture one with [`Decorate::native_ref`], then reach the native
/// widget later (from event handlers, timers) through a toolkit ext accessor. `node`/`with` yield
/// `None` before mount and after the node's subtree is disposed, so async races are safe no-ops.
///
/// Reads are REACTIVE: inside a binding or memo, `node()` subscribes to the ref's mount/clear
/// transitions (a `Trigger` underneath), so a label like
/// `label(move || if r.node().is_some() { "live" } else { "cleared" })` updates when the
/// referenced piece unmounts — the toggle demo on the showcase Tweaks page. (The `when`-arm's
/// disposal lands at the turn boundary, after ordinary bindings re-ran — piggybacking on some
/// other signal would read a stale mount state; the trigger fires at the actual transition.)
/// Main-thread only, like every realized-tree type.
#[derive(Clone)]
pub struct NativeRef {
    cell: Rc<std::cell::Cell<Option<day_core::RNode>>>,
    changed: day_reactive::Trigger,
}

impl Default for NativeRef {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeRef {
    pub fn new() -> Self {
        NativeRef {
            cell: Rc::new(std::cell::Cell::new(None)),
            changed: day_reactive::Trigger::new(),
        }
    }

    /// The mounted node, if it is currently live. A tracked read (see the type docs).
    pub fn node(&self) -> Option<day_core::RNode> {
        self.changed.track();
        let node = self.cell.get()?;
        // Generational slotmap keys make a disposed node a clean miss, never a stale hit.
        let live = day_core::try_with_tree(|t| t.node_kind(node).is_some()).unwrap_or(false);
        live.then_some(node)
    }

    /// Run `f` with the live node (e.g. inside `day_appkit::with_native`); `None` if disposed.
    pub fn with<R>(&self, f: impl FnOnce(day_core::RNode) -> R) -> Option<R> {
        self.node().map(f)
    }

    fn transition(&self, node: Option<day_core::RNode>) {
        self.cell.set(node);
        self.changed.notify();
    }
}

/// A transparent native layer node (`CONTAINER`, no fill/clip/corner) used by the animatable
/// modifiers (`.opacity`/`.transform`/`.animation`) to carry a per-node opacity, transform, or
/// implicit animation. Layout-transparent (`PassThrough`), so it never affects sizing.
fn layer_node(cx: &mut BuildCx) -> RNode {
    cx.native(
        kinds::CONTAINER,
        &ContainerProps {
            background: None,
            corner_radius: 0.0,
            clips: false,
            role: None,
        },
        Rc::new(PassThrough),
        Flex::default(),
        Boundary::No,
    )
}

pub trait Decorate: Piece + Sized {
    /// Stable element identifier: a11y identifier + dayscript locator + lint uniqueness (§5.5).
    fn id(self, id: impl Into<String>) -> AnyPiece {
        let id = id.into();
        piece_fn(move |cx| {
            let n = self.build(cx);
            with_tree(|t| t.set_id(n, id));
            n
        })
    }

    /// Keyed id for collection items: rendered `prefix:key` (§5.5).
    fn id_keyed(self, prefix: &'static str, key: impl std::fmt::Display) -> AnyPiece {
        let id = format!("{prefix}:{key}");
        self.id(id)
    }

    /// Apply a **tweak**: `f` runs once at mount, after the native widget exists, with the
    /// realized node (docs/tweaks.md). Reach the typed native handle through the compiled
    /// backend's ext accessor (`day_appkit::with_native`, `day_gtk::with_native`, …) — or apply
    /// a packaged `day-tweak-*` crate's modifier instead of calling this directly. If the native
    /// change affects the widget's intrinsic size, follow it with
    /// [`day_core::invalidate_size`]. Day may overwrite *managed* properties (title, value,
    /// enabled, frame, a11y) on its next patch; unmanaged properties are stable.
    fn tweak(self, f: impl FnOnce(day_core::RNode) + 'static) -> AnyPiece {
        piece_fn(move |cx| {
            let n = self.build(cx);
            f(n);
            n
        })
    }

    /// Capture a [`NativeRef`] to this piece's realized node for later imperative access
    /// (docs/tweaks.md). The ref clears automatically when the piece's scope is disposed.
    fn native_ref(self, r: &NativeRef) -> AnyPiece {
        let r = r.clone();
        piece_fn(move |cx| {
            let n = self.build(cx);
            r.transition(Some(n));
            let cleared = r.clone();
            Scope::current().on_cleanup(move || cleared.transition(None));
            n
        })
    }

    fn padding(self, insets: impl IntoInsets) -> AnyPiece {
        let insets = insets.into_insets();
        piece_fn(move |cx| {
            let w = cx.layout_only(
                Rc::new(PaddingLayout { insets }),
                Flex::default(),
                Boundary::No,
            );
            cx.under(w, |cx| {
                let _ = self.build(cx);
            });
            w
        })
    }

    fn frame(self, width: f64, height: f64) -> AnyPiece {
        piece_fn(move |cx| {
            let w = cx.layout_only(
                Rc::new(FrameLayout {
                    width: Some(width),
                    height: Some(height),
                }),
                Flex::default(),
                Boundary::Yes, // two-axis fixed frame = layout boundary (§7.4)
            );
            cx.under(w, |cx| {
                let _ = self.build(cx);
            });
            w
        })
    }

    /// Fix this piece's WIDTH to `width` points while its height stays flexible (hugging its content
    /// or filling on the cross axis). The single-axis complement to [`Self::frame`] — e.g. a
    /// fixed-width sidebar pane in a `row` whose height fills the window.
    fn width(self, width: f64) -> AnyPiece {
        piece_fn(move |cx| {
            let w = cx.layout_only(
                Rc::new(FrameLayout {
                    width: Some(width),
                    height: None,
                }),
                Flex::default(),
                Boundary::No,
            );
            cx.under(w, |cx| {
                let _ = self.build(cx);
            });
            w
        })
    }

    /// Fix this piece's HEIGHT to `height` points while its width stays flexible. The single-axis
    /// complement to [`Self::frame`] — e.g. a fixed-height header/toolbar bar that fills its width.
    fn height(self, height: f64) -> AnyPiece {
        piece_fn(move |cx| {
            let w = cx.layout_only(
                Rc::new(FrameLayout {
                    width: None,
                    height: Some(height),
                }),
                Flex::default(),
                Boundary::No,
            );
            cx.under(w, |cx| {
                let _ = self.build(cx);
            });
            w
        })
    }

    fn a11y(self, f: impl FnOnce(A11yBuilder) -> A11yBuilder + 'static) -> AnyPiece {
        piece_fn(move |cx| {
            let n = self.build(cx);
            let props = f(A11yBuilder::default()).0;
            with_tree(|t| t.set_a11y(n, props));
            n
        })
    }

    /// Fire when this piece is tapped (bounding-box; shapes override with path-precise testing).
    fn on_tap(self, f: impl Fn() + 'static) -> AnyPiece {
        piece_fn(move |cx| {
            let n = self.build(cx);
            with_tree(|t| t.enable_gesture(n, GestureKind::Tap));
            cx.on(n, move |ev| {
                if matches!(ev, Event::Tap(_)) {
                    f();
                }
            });
            n
        })
    }

    /// Bind this control's keyboard focus to a signal (docs/focus.md), two-way like every other
    /// binding: native focus changes write the signal; writing the signal moves focus. Takes a
    /// `Signal<bool>` for one control, or `(Signal<Option<K>>, K::Variant)` binding one control
    /// of a group — writing `false`/`None` resigns focus (dismissing the soft keyboard on
    /// mobile). Focus applies asynchronously: a write is a request, resolved on the next turn,
    /// and the signal always ends up reflecting what the platform actually did.
    fn focused<M>(self, binding: impl IntoFocusBinding<M>) -> AnyPiece {
        let (want, on_native) = binding.into_focus_binding();
        piece_fn(move |cx| {
            let n = self.build(cx);
            // Echo cell: the control's focus state as last reported by the NATIVE side. An
            // apply whose desired state matches it is the echo of a native change (or already
            // satisfied) and must not re-drive the toolkit — the selector echo-cell rule.
            let native = Rc::new(Cell::new(false));
            {
                let native = native.clone();
                cx.on(n, move |ev| {
                    if let Event::FocusChanged(f) = ev {
                        native.set(*f);
                        on_native(*f);
                    }
                });
            }
            // Signal → native, deferred one turn (`on_main`): focus is async by contract, and
            // the deferral also lets a mount-time `Some(K::V)` land after the widget is in the
            // window (dialog default focus). The initial `false` is not applied — resigning
            // focus the control never had would steal it from whoever has it.
            let first = Cell::new(true);
            bind(want, move |want: &bool| {
                let want = *want;
                if first.replace(false) && !want {
                    return;
                }
                if native.get() == want {
                    return;
                }
                day_reactive::on_main(move || with_tree(|t| t.focus_node(n, want)));
            });
            n
        })
    }

    /// Attach a context menu, shown with the platform's native affordance on secondary-click (desktop)
    /// or long-press (mobile). Items are built with [`menu_item`]/[`sub_menu`]/[`menu_role`]/
    /// [`menu_separator`]. Passing an empty `Vec` removes any menu.
    fn context_menu(self, items: Vec<MenuEntry>) -> AnyPiece {
        piece_fn(move |cx| {
            let n = self.build(cx);
            let model = lower_menu(items);
            with_tree(|t| t.set_context_menu(n, model));
            n
        })
    }

    /// Fire on each phase of a drag over this piece.
    fn on_drag(self, f: impl Fn(Drag) + 'static) -> AnyPiece {
        piece_fn(move |cx| {
            let n = self.build(cx);
            with_tree(|t| t.enable_gesture(n, GestureKind::Drag));
            cx.on(n, move |ev| {
                if let Event::Drag {
                    phase,
                    location,
                    translation,
                } = ev
                {
                    f(Drag {
                        phase: *phase,
                        location: *location,
                        translation: *translation,
                    });
                }
            });
            n
        })
    }

    /// Fill the piece's bounds with a solid color painted behind it — a message-bubble / card /
    /// badge surface. Accepts a constant [`Color`], a `Signal<Color>`, or a `Fn() -> Color`; a
    /// reactive color repaints the surface when its source changes. Wraps the piece in a native
    /// container that carries the fill, so it composes with [`Self::corner_radius`] for a rounded
    /// colored surface and with [`Self::padding`] for interior inset.
    fn background<M>(self, color: impl IntoReactive<Color, M>) -> AnyPiece {
        let color = color.into_reactive();
        piece_fn(move |cx| {
            let node = cx.native(
                kinds::CONTAINER,
                &ContainerProps {
                    background: Some(color.get_untracked()),
                    corner_radius: 0.0,
                    clips: false,
                    role: None,
                },
                Rc::new(PassThrough),
                Flex::default(),
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = self.build(cx);
            });
            // Only a reactive source needs a binding; a constant fill is applied once at realize.
            if let Reactive::Dyn(_) = &color {
                bind(
                    move || color.get(),
                    move |c: &Color| {
                        with_tree(|t| {
                            t.patch(node, Box::new(ContainerPatch::Background(Some(*c))), false)
                        });
                    },
                );
            }
            node
        })
    }

    /// Round the piece's corners to `radius` points, clipping its background and content to the
    /// rounded rectangle. Compose after [`Self::background`] for a rounded colored surface, or use
    /// alone to round a clipped child (e.g. an avatar image).
    fn corner_radius(self, radius: f64) -> AnyPiece {
        piece_fn(move |cx| {
            let node = cx.native(
                kinds::CONTAINER,
                &ContainerProps {
                    background: None,
                    corner_radius: radius,
                    clips: true,
                    role: None,
                },
                Rc::new(PassThrough),
                Flex::default(),
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = self.build(cx);
            });
            node
        })
    }

    /// Animate/set the piece's opacity (`0.0` transparent … `1.0` opaque). Wrapped in a native
    /// layer so it composes with `.background`; the change animates when made inside
    /// [`with_animation`] or under a `.animation` ancestor (§8.4).
    fn opacity<M>(self, opacity: impl IntoReactive<f64, M>) -> AnyPiece {
        let op = opacity.into_reactive();
        piece_fn(move |cx| {
            let node = layer_node(cx);
            cx.under(node, |cx| {
                let _ = self.build(cx);
            });
            bind(
                move || op.get(),
                move |v: &f64| with_tree(|t| t.set_node_opacity(node, *v)),
            );
            node
        })
    }

    /// Apply an animatable [`Transform`] (translate/scale/rotate about the center) — the cheap
    /// movement/scaling channel that never triggers relayout (§8.4). Prefer this over `.offset`
    /// for animated motion.
    fn transform<M>(self, t: impl IntoReactive<Transform, M>) -> AnyPiece {
        let t = t.into_reactive();
        piece_fn(move |cx| {
            let node = layer_node(cx);
            cx.under(node, |cx| {
                let _ = self.build(cx);
            });
            bind(
                move || t.get(),
                move |v: &Transform| with_tree(|tr| tr.set_node_transform(node, *v)),
            );
            node
        })
    }

    /// Uniformly scale the piece by `factor` about its center (animatable). Convenience over
    /// [`Self::transform`].
    fn scale<M>(self, factor: impl IntoReactive<f64, M>) -> AnyPiece {
        let f = factor.into_reactive();
        self.transform(move || Transform::scale(f.get(), f.get()))
    }

    /// Rotate the piece by `degrees` clockwise about its center (animatable).
    fn rotation<M>(self, degrees: impl IntoReactive<f64, M>) -> AnyPiece {
        let d = degrees.into_reactive();
        self.transform(move || Transform::rotate(d.get()))
    }

    /// Translate the piece by (`x`, `y`) points WITHOUT relayout (animatable) — the
    /// animation-friendly sibling of `.offset`.
    fn translation<Mx, My>(
        self,
        x: impl IntoReactive<f64, Mx>,
        y: impl IntoReactive<f64, My>,
    ) -> AnyPiece {
        let (x, y) = (x.into_reactive(), y.into_reactive());
        self.transform(move || Transform::translate(x.get(), y.get()))
    }

    /// Attach an implicit animation (§8.4): changes to this piece's — and its descendants' —
    /// animatable properties animate with `anim` even outside a [`with_animation`]. SwiftUI's
    /// `.animation`. The ambient `with_animation` takes precedence when both apply.
    fn animation(self, anim: AnimSpec) -> AnyPiece {
        piece_fn(move |cx| {
            let node = layer_node(cx);
            with_tree(|t| t.set_implicit_anim(node, Some(anim)));
            cx.under(node, |cx| {
                let _ = self.build(cx);
            });
            node
        })
    }

    /// Apply a [`Modifier`] — or, via the blanket impl, a plain `FnOnce(AnyPiece) -> AnyPiece`
    /// closure — to this piece. Pure composition: `content.modifier(m) == m.apply(content.any())`.
    fn modifier(self, m: impl Modifier) -> AnyPiece {
        m.apply(self.any())
    }

    /// Draw `over` on top of this piece, centered, WITHOUT affecting layout size — a badge /
    /// annotation overlay. `self` is the sizing content (bottom of the z-order); `over` is proposed
    /// `self`'s size and drawn on top. For an explicit alignment use [`Self::overlay_aligned`]; for
    /// a stack that sizes to the UNION of its children use [`zstack`].
    fn overlay(self, over: impl Piece) -> AnyPiece {
        self.overlay_aligned(Alignment::Center, over)
    }

    /// [`Self::overlay`] with an explicit [`Alignment`] for the annotation (e.g. a corner badge with
    /// [`Alignment::TopTrailing`]).
    fn overlay_aligned(self, align: Alignment, over: impl Piece) -> AnyPiece {
        piece_fn(move |cx| {
            let node = cx.native(
                kinds::CONTAINER,
                &ContainerProps::default(),
                Rc::new(OverlayLayout {
                    align,
                    size_to_first: true,
                }),
                Flex::default(),
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = self.build(cx); // sizing content (bottom)
                let _ = over.build(cx); // annotation on top
            });
            node
        })
    }

    /// Expand to fill the available space on both axes (a filling pane / card that stretches to
    /// its container). Wraps the piece in a layout-only node carrying grow [`Flex`] — the stack
    /// offers it the space and it fills; no native backing, so this is a pure layout change.
    fn grow(self) -> AnyPiece {
        self.grow_axes(true, true)
    }

    /// Expand to fill the available horizontal space.
    fn grow_w(self) -> AnyPiece {
        self.grow_axes(true, false)
    }

    /// Expand to fill the available vertical space.
    fn grow_h(self) -> AnyPiece {
        self.grow_axes(false, true)
    }

    #[doc(hidden)]
    fn grow_axes(self, w: bool, h: bool) -> AnyPiece {
        piece_fn(move |cx| {
            let node = cx.layout_only(
                Rc::new(GrowLayout { w, h }),
                Flex {
                    grow_w: w,
                    grow_h: h,
                    ..Default::default()
                },
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = self.build(cx);
            });
            node
        })
    }

    /// Span `n` columns (n ≥ 1) of the enclosing [`grid`] (docs/grid.md). Grid modifiers set
    /// facts on the node the grid sees: apply them LAST (outermost), like `.grow_w()` — an
    /// outer wrapper would hide the facts from the grid.
    fn grid_span(self, n: usize) -> AnyPiece {
        piece_fn(move |cx| {
            let node = self.build(cx);
            with_tree(|t| {
                t.set_grid_facts(
                    node,
                    GridFacts {
                        col_span: n.clamp(1, u16::MAX as usize) as u16,
                        ..Default::default()
                    },
                )
            });
            node
        })
    }

    /// Override this cell's alignment within its cell rect of the enclosing [`grid`]
    /// (docs/grid.md). Apply LAST (outermost), like [`Self::grid_span`].
    fn grid_align(self, a: Alignment) -> AnyPiece {
        piece_fn(move |cx| {
            let node = self.build(cx);
            with_tree(|t| {
                t.set_grid_facts(
                    node,
                    GridFacts {
                        align: Some(a),
                        ..Default::default()
                    },
                )
            });
            node
        })
    }

    /// While this subtree is mounted, ask the OS to require a second swipe for its edge
    /// gestures on `edges` (docs/cover.md) — the SwiftUI `defersSystemGestures(on:)`
    /// analogue. Put it on a game or drawing surface whose touches run to the screen edge,
    /// so a swipe up from the bottom doesn't leave the app. iOS defers the chosen edges'
    /// system gestures; Android enters swipe-to-reveal immersive mode while any subtree
    /// requests deferral; desktop backends no-op.
    fn defers_system_gestures(self, edges: day_spec::Edges) -> AnyPiece {
        piece_fn(move |cx| {
            let token = day_core::shield::push_gesture_deferral(edges);
            Scope::current().on_cleanup(move || day_core::shield::pop_gesture_deferral(token));
            self.build(cx)
        })
    }

    /// While this subtree is mounted, the enclosing [`cover`] (or other modal surface) must
    /// not be dismissed interactively — the SwiftUI `interactiveDismissDisabled()` analogue
    /// (docs/cover.md). System back / sheet gestures are ignored; only programmatic writes
    /// (an explicit close control) dismiss it.
    fn interactive_dismiss_disabled(self) -> AnyPiece {
        piece_fn(move |cx| {
            let token = day_core::shield::push_dismiss_disabled();
            Scope::current().on_cleanup(move || day_core::shield::pop_dismiss_disabled(token));
            self.build(cx)
        })
    }

    fn any(self) -> AnyPiece {
        AnyPiece::new(self)
    }
}

impl<P: Piece> Decorate for P {}

#[derive(Default)]
pub struct A11yBuilder(A11yProps);

impl A11yBuilder {
    pub fn label(mut self, s: impl Into<String>) -> Self {
        self.0.label = Some(s.into());
        self
    }
    pub fn hint(mut self, s: impl Into<String>) -> Self {
        self.0.hint = Some(s.into());
        self
    }
    /// The control's current value read aloud by the screen reader (e.g. a `Meter`'s "72%").
    pub fn value(mut self, s: impl Into<String>) -> Self {
        self.0.value = Some(s.into());
        self
    }
    pub fn role(mut self, r: Role) -> Self {
        self.0.role = r;
        self
    }
    /// Hide this element from assistive tech (still visible on screen) — e.g. a redundant chrome
    /// element already announced by its labelled sibling.
    pub fn hidden(mut self) -> Self {
        self.0.hidden = true;
        self
    }
    /// Purely decorative (a background flourish): hidden from assistive tech and, for images,
    /// exempt from the "needs a label" lint (§13).
    pub fn decorative(mut self) -> Self {
        self.0.decorative = true;
        self.0.hidden = true;
        self
    }
}

// ---------------------------------------------------------------------------
// Prelude
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Picker (kinds::PICKER, docs/picker.md) — built-in since 2026-07.
// ---------------------------------------------------------------------------

/// A native picker bound two-way to `selected`. Style via `.menu()`/`.segmented()`/`.inline()`.
pub struct Picker {
    options: Vec<String>,
    selected: Signal<usize>,
    style: day_spec::props::PickerStyle,
}

/// `picker(["A", "B", "C"], choice).segmented()` — options are fixed, `selected` is the bound index.
pub fn picker<S: Into<String>>(
    options: impl IntoIterator<Item = S>,
    selected: Signal<usize>,
) -> Picker {
    Picker {
        options: options.into_iter().map(Into::into).collect(),
        selected,
        style: day_spec::props::PickerStyle::Menu,
    }
}

impl Picker {
    pub fn menu(mut self) -> Self {
        self.style = day_spec::props::PickerStyle::Menu;
        self
    }
    pub fn segmented(mut self) -> Self {
        self.style = day_spec::props::PickerStyle::Segmented;
        self
    }
    pub fn inline(mut self) -> Self {
        self.style = day_spec::props::PickerStyle::Inline;
        self
    }
    pub fn style(mut self, style: day_spec::props::PickerStyle) -> Self {
        self.style = style;
        self
    }
}

impl Piece for Picker {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Picker {
            options,
            selected,
            style,
        } = self;
        let initial = day_spec::props::PickerProps {
            options,
            selected: selected.get_untracked(),
            style,
        };
        let node = cx.leaf(kinds::PICKER, &initial, Flex::default());
        bind_seeded(
            initial.selected,
            move || selected.get(),
            move |v: &usize| {
                with_tree(|t| {
                    t.patch(
                        node,
                        Box::new(day_spec::props::PickerPatch::Selected(*v)),
                        false,
                    )
                });
            },
        );
        cx.on(node, move |ev| {
            if let Event::SelectionChanged(i) = ev
                && *i >= 0
            {
                selected.set_rw(*i as usize);
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Text area (kinds::TEXT_AREA, docs/textarea.md) — built-in since 2026-07.
// ---------------------------------------------------------------------------

/// A native multi-line text editor bound two-way to `text`. Configure a prompt with
/// `.placeholder(_)` and the auto-growing height band with `.min_lines(_)` / `.max_lines(_)`.
pub struct TextArea {
    text: Signal<String>,
    placeholder: Option<TextSource>,
    min_lines: u32,
    max_lines: u32,
}

/// `text_area(text)` — a native multi-line editor whose contents mirror `text` in both directions.
pub fn text_area(text: Signal<String>) -> TextArea {
    TextArea {
        text,
        placeholder: None,
        min_lines: 1,
        max_lines: 0,
    }
}

impl TextArea {
    /// The empty-state prompt shown when the editor is empty (a constant, `Signal<String>`, or
    /// closure — evaluated once for the initial value; not reactive after build).
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
        self
    }

    /// The minimum height, in text lines (default 1): the editor never shrinks below this.
    pub fn min_lines(mut self, lines: u32) -> Self {
        self.min_lines = lines.max(1);
        self
    }

    /// The maximum height, in text lines, before the editor scrolls internally. `0` (the
    /// default) means unbounded — the editor keeps growing and never scrolls.
    pub fn max_lines(mut self, lines: u32) -> Self {
        self.max_lines = lines;
        self
    }
}

impl Piece for TextArea {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let TextArea {
            text,
            placeholder,
            min_lines,
            max_lines,
        } = self;
        let initial = text.get_untracked();
        let ph = placeholder.map(|p| p.initial()).unwrap_or_default();
        let node = cx.leaf(
            kinds::TEXT_AREA,
            &day_spec::props::TextAreaProps {
                text: initial.clone(),
                placeholder: ph,
                min_lines,
                // A 0 max is "unbounded"; a non-zero max is floored to min so the band is
                // never inverted.
                max_lines: if max_lines == 0 {
                    0
                } else {
                    max_lines.max(min_lines)
                },
            },
            // A composer fills the available width; height is content-driven (the backend's
            // measure grows it between min/max lines), so it is NOT a height-growing leaf.
            Flex {
                grow_w: true,
                ..Default::default()
            },
        );
        // Controlled input with origin tracking (§4.4): the echo guard remembers the last value
        // that arrived FROM the native widget so bind_seeded does not patch it straight back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let g = guard.clone();
        bind_seeded(
            initial,
            move || text.get(),
            move |t: &String| {
                let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
                if !from_native {
                    with_tree(|tr| {
                        tr.patch(
                            node,
                            Box::new(day_spec::props::TextAreaPatch::SetText(t.clone())),
                            true,
                        )
                    });
                }
            },
        );
        cx.on(node, move |ev| {
            if let Event::TextChanged(t) = ev {
                *guard.borrow_mut() = Some(t.clone());
                text.set(t.clone());
            }
        });
        node
    }
}

pub mod prelude {
    pub use crate::TextStyle;
    pub use crate::routes;
    pub use crate::{
        A11yBuilder, Alert, ButtonStyle, Confirm, Corner, Cover, Decorate, Drag, Draw, FileUrl,
        FilledButtonStyle, FormSection, Grid, GridRow, HAlign, IntoFocusBinding, IntoFraction,
        IntoReactive, IntoText, ItemSlot, Link, List, MenuEntry, Modifier, NativeRef, OpenFile,
        Prompt, Reactive, Route, RoutePath, SaveFile, Selector, SelectorStyle, ShapeKind,
        ShapePiece, SignalRw, Stack, VAlign, ZStack, alert, app_menu, arc, button, canvas, capsule,
        circle, column, confirm, cover, current_route, divider, each, ellipse, environment, form,
        frame_clock, grid, grid_row, image, label, labeled, line, link, list, menu_item, menu_role,
        menu_separator, nav_back, nav_link, nav_link_to, navigate, navigate_to, open_file, picker,
        polygon, progress, prompt, rectangle, rounded_rectangle, route, route_param, route_params,
        row, save_file, scroll, section, selector, shape, shape_group, shape_group_fn, slider,
        spacer, spinner, stack, sub_menu, text_area, text_field, toggle, when, with_environment,
        zstack,
    };
    pub use crate::{Picker, TextArea};
    pub use day_core::{
        Alignment, AnyPiece, BuildCx, Piece, PieceSeq, PieceVec, RNode, ScrollTarget,
        invalidate_size, open_url, piece_fn, with_animation,
    };
    pub use day_geometry::{Affine, Animatable, Color, Insets, Point, Rect, Size, Transform};
    pub use day_reactive::{
        Effect, Memo, Scope, Setter, Signal, Trigger, batch, bind, untrack, watch,
    };
    pub use day_spec::props::PickerStyle;
    pub use day_spec::props::RowHeight;
    pub use day_spec::{AnimSpec, AnimSpec as Animation, Curve};
    pub use day_spec::{AssetName, FontFamily, ImageName};
    pub use day_spec::{DragPhase, Edges, GestureKind};
    pub use day_spec::{
        DrawOp, LinearGradient, Paint, RadialGradient, Shape, TextAnchor, UnitPoint,
    };
    pub use day_spec::{Font, FontSpec, FontWeight, Role};
    pub use day_spec::{MenuItem, MenuRole, Shortcut};
    pub use std::time::Duration;
}

// ---------------------------------------------------------------------------
// Canvas (§11): record a display list reactively; backends replay natively.
// ---------------------------------------------------------------------------

pub struct Draw {
    ops: Vec<DrawOp>,
}

/// Canvas text styling (named fields per the API style rule, docs/api-style.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextStyle {
    pub size: f64,
    pub color: Color,
    pub anchor: day_spec::TextAnchor,
}

impl Draw {
    /// Fill a shape with a solid color or a [`LinearGradient`] (both convert to [`Paint`];
    /// gradient unit points resolve against the shape's bounding box — docs/shapes.md §3.2).
    pub fn fill(&mut self, shape: Shape, paint: impl Into<Paint>) {
        self.ops.push(DrawOp::Fill(shape, paint.into()));
    }
    pub fn stroke(&mut self, shape: Shape, color: Color, width: f64) {
        self.ops.push(DrawOp::Stroke(shape, color, width));
    }
    pub fn text(&mut self, text: &str, at: Point, style: TextStyle) {
        self.ops.push(DrawOp::Text {
            text: text.to_owned(),
            at,
            size: style.size,
            color: style.color,
            anchor: style.anchor,
        });
    }
    /// Save the current transform/clip; pair with [`Draw::restore`].
    pub fn save(&mut self) {
        self.ops.push(DrawOp::Save);
    }
    /// Restore the transform/clip saved by the matching [`Draw::save`].
    pub fn restore(&mut self) {
        self.ops.push(DrawOp::Restore);
    }
    /// Multiply an affine onto the current transform (shape rotate/scale/offset, §11).
    pub fn concat(&mut self, m: day_geometry::Affine) {
        self.ops.push(DrawOp::Concat(m));
    }
    /// Draw within `m` applied to the CTM, restoring afterwards.
    pub fn transformed(&mut self, m: day_geometry::Affine, f: impl FnOnce(&mut Draw)) {
        self.save();
        self.concat(m);
        f(self);
        self.restore();
    }
}

/// Create + wire a reactive canvas leaf with a given flex: the draw closure re-records on any
/// tracked read and on `FrameChanged`; replay is equality-gated by `DrawOp: PartialEq` (§4.2).
/// Shared by [`canvas`] (intrinsic) and [`shape`] (grows to fill, §shapes).
pub(crate) fn canvas_leaf(
    cx: &mut BuildCx,
    flex: Flex,
    draw: impl Fn(&mut Draw, Size) + 'static,
) -> RNode {
    use day_reactive::{Trigger, bind};
    let node = cx.leaf(kinds::CANVAS, &CanvasProps::default(), flex);
    let trig = Trigger::new();
    cx.on(node, move |ev| {
        if matches!(ev, Event::FrameChanged(_)) {
            trig.notify();
        }
    });
    let draw = std::rc::Rc::new(draw);
    let d2 = draw.clone();
    bind(
        move || {
            trig.track();
            let size = with_tree(|t| t.node_frame(node))
                .map(|f| f.size)
                .unwrap_or(Size::new(0.0, 0.0));
            let mut d = Draw { ops: Vec::new() };
            (d2)(&mut d, size);
            d.ops
        },
        move |ops: &Vec<DrawOp>| {
            with_tree(|t| t.replay(node, ops.clone()));
        },
    );
    node
}

/// The drawing closure is a binding: signal reads re-record; layout size changes re-record
/// (via FrameChanged); replay is equality-gated by DrawOp's PartialEq (§4.2).
pub fn canvas(draw: impl Fn(&mut Draw, Size) + 'static) -> AnyPiece {
    piece_fn(move |cx| canvas_leaf(cx, Flex::default(), draw))
}

/// A frame clock (§8.4): an invisible, zero-size piece that calls `tick` every animation frame with
/// the wall-clock delta since the previous frame, for as long as it is mounted. Drop it into the
/// tree (e.g. behind a `canvas` in a `zstack`) to drive a game loop or self-driven animation: the
/// tick mutates state `Signal`s, and a `canvas` reading them re-records that frame.
///
/// Backend-executed vsync: Day re-arms the platform's display link only while a `frame_clock` (or
/// other consumer) is live and stops when the last one unmounts — no idle wakeups. The delta is
/// clamped (≤100 ms) so a backgrounded window can't deliver a huge jump.
///
/// ```ignore
/// zstack((
///     canvas(move |d, sz| draw(d, sz, state)).grow(),
///     frame_clock(move |dt| step(dt, state)),
/// ))
/// ```
pub fn frame_clock(tick: impl FnMut(std::time::Duration) + 'static) -> AnyPiece {
    type TickSlot = Rc<RefCell<Option<Box<dyn FnMut(std::time::Duration)>>>>;
    // Registered on first build (in the mounting scope) and removed when that scope is disposed.
    let slot: TickSlot = Rc::new(RefCell::new(Some(Box::new(tick))));
    piece_fn(move |cx| {
        if let Some(cb) = slot.borrow_mut().take() {
            let id = day_core::add_frame_consumer(cb);
            Scope::current().on_cleanup(move || day_core::remove_frame_consumer(id));
        }
        label("").frame(0.0, 0.0).build(cx)
    })
    .any()
}

// ---------------------------------------------------------------------------
// Reactive<T>: a value, a Signal, or a closure — the generalisation of IntoText/IntoFraction.
// ---------------------------------------------------------------------------

/// A parameter that is either a constant or a reactive source. `get()` is a tracked read, so any
/// `Reactive` used inside a canvas draw closure makes that shape re-record when the source changes.
pub enum Reactive<T: Clone + 'static> {
    Const(T),
    Dyn(Rc<dyn Fn() -> T>),
}
impl<T: Clone + 'static> Clone for Reactive<T> {
    fn clone(&self) -> Self {
        match self {
            Reactive::Const(v) => Reactive::Const(v.clone()),
            Reactive::Dyn(f) => Reactive::Dyn(f.clone()),
        }
    }
}
impl<T: Clone + 'static> Reactive<T> {
    pub fn get(&self) -> T {
        match self {
            Reactive::Const(v) => v.clone(),
            Reactive::Dyn(f) => f(),
        }
    }
    pub fn get_untracked(&self) -> T {
        match self {
            Reactive::Const(v) => v.clone(),
            Reactive::Dyn(f) => day_reactive::untrack(|| f()),
        }
    }
}
/// Disjoint-marker conversion (like [`IntoText`]): accepts `T`, `Signal<T>`, or `Fn() -> T`.
pub trait IntoReactive<T: Clone + 'static, M> {
    fn into_reactive(self) -> Reactive<T>;
}
impl<T: Clone + 'static> IntoReactive<T, StaticMark> for T {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Const(self)
    }
}
impl<T: Clone + 'static> IntoReactive<T, SignalMark> for Signal<T> {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Dyn(Rc::new(move || self.get()))
    }
}
impl<T: Clone + 'static, F: Fn() -> T + 'static> IntoReactive<T, FnMark> for F {
    fn into_reactive(self) -> Reactive<T> {
        Reactive::Dyn(Rc::new(self))
    }
}

// ---------------------------------------------------------------------------
// Shapes (docs/shapes.md): high-level shape pieces atop the canvas display list. Frame-relative
// geometry, reactive fill/stroke, rotate/scale/offset transforms, and tap/drag gestures.
// ---------------------------------------------------------------------------

use day_geometry::Affine;
use day_geometry::Proposal;
pub use day_spec::{DragPhase, GestureKind};

/// A shape's geometry, resolved against the rect layout assigns it (frame-relative, SwiftUI-style).
#[derive(Clone, Debug, PartialEq)]
pub enum ShapeKind {
    Rectangle,
    RoundedRectangle {
        corner: Corner,
    },
    Circle,
    Ellipse,
    Capsule,
    /// A stroked arc of the inscribed ellipse; degrees, 0 = +x, clockwise.
    Arc {
        start_deg: f64,
        sweep_deg: f64,
    },
    /// A stroked segment between two unit points of the resolved rect (stroke-only; fills are
    /// ignored). Unit points resolve unclamped, like every unit-space resolve.
    Line {
        from: UnitPoint,
        to: UnitPoint,
    },
    /// A filled/stroked polygon of unit points resolved against the rect (unclamped — points may
    /// deliberately sit outside 0..1).
    Polygon {
        points: Rc<[UnitPoint]>,
    },
}

/// A corner radius: absolute points, or a 0..1 fraction of `min(width, height)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Corner {
    Fixed(f64),
    Fraction(f64),
}
impl From<f64> for Corner {
    fn from(v: f64) -> Self {
        Corner::Fixed(v)
    }
}
impl Corner {
    fn resolve(self, rect: Rect) -> f64 {
        let cap = rect.size.width.min(rect.size.height) / 2.0;
        match self {
            Corner::Fixed(v) => v.clamp(0.0, cap),
            Corner::Fraction(f) => f.clamp(0.0, 1.0) * cap,
        }
    }
}
impl ShapeKind {
    /// Lower to a drawable geometry within `rect`.
    fn geometry(self, rect: Rect) -> Shape {
        match self {
            ShapeKind::Rectangle => Shape::Rect(rect),
            ShapeKind::RoundedRectangle { corner } => {
                Shape::RoundedRect(rect, corner.resolve(rect))
            }
            ShapeKind::Ellipse => Shape::Ellipse(rect),
            ShapeKind::Capsule => {
                Shape::RoundedRect(rect, rect.size.width.min(rect.size.height) / 2.0)
            }
            ShapeKind::Circle => {
                let d = rect.size.width.min(rect.size.height);
                let c = rect.center();
                Shape::Ellipse(Rect::new(c.x - d / 2.0, c.y - d / 2.0, d, d))
            }
            ShapeKind::Arc {
                start_deg,
                sweep_deg,
            } => Shape::Arc {
                rect,
                start_deg,
                sweep_deg,
            },
            ShapeKind::Line { from, to } => Shape::Line(from.resolve(rect), to.resolve(rect)),
            ShapeKind::Polygon { points } => {
                Shape::Polygon(points.iter().map(|p| p.resolve(rect)).collect())
            }
        }
    }
    /// Point-in-shape test (in the shape's own, untransformed coordinates).
    fn contains(self, rect: Rect, p: Point) -> bool {
        fn in_rect(r: Rect, p: Point) -> bool {
            p.x >= r.min_x() && p.x <= r.max_x() && p.y >= r.min_y() && p.y <= r.max_y()
        }
        fn in_ellipse(r: Rect, p: Point) -> bool {
            if r.size.width <= 0.0 || r.size.height <= 0.0 {
                return false;
            }
            let c = r.center();
            let dx = (p.x - c.x) / (r.size.width / 2.0);
            let dy = (p.y - c.y) / (r.size.height / 2.0);
            dx * dx + dy * dy <= 1.0
        }
        /// Even-odd ray cast: count edge crossings of the +x ray from `p`.
        fn in_polygon(pts: &[Point], p: Point) -> bool {
            if pts.len() < 3 {
                return false;
            }
            let mut inside = false;
            let mut j = pts.len() - 1;
            for i in 0..pts.len() {
                let (a, b) = (pts[i], pts[j]);
                if (a.y > p.y) != (b.y > p.y) {
                    let x = a.x + (p.y - a.y) / (b.y - a.y) * (b.x - a.x);
                    if p.x < x {
                        inside = !inside;
                    }
                }
                j = i;
            }
            inside
        }
        match self.geometry(rect) {
            Shape::Ellipse(r) => in_ellipse(r, p),
            Shape::Rect(r) | Shape::RoundedRect(r, _) => in_rect(r, p),
            Shape::Polygon(pts) => in_polygon(&pts, p),
            _ => in_rect(rect, p), // arc / line: bounding-box fallback
        }
    }
}

/// Drag info delivered to a shape's `.on_drag` handler.
#[derive(Clone, Copy, Debug)]
pub struct Drag {
    pub phase: DragPhase,
    pub location: Point,
    pub translation: Point,
}

/// The drawable description of a shape — everything but gestures. Cloneable so [`shape_group`]
/// can collect many descriptions into one canvas closure (docs/shapes.md §3.6).
#[derive(Clone)]
struct ShapeSpec {
    kind: Reactive<ShapeKind>,
    fill: Option<Reactive<Color>>,
    fill_linear: Option<Reactive<LinearGradient>>,
    fill_radial: Option<Reactive<RadialGradient>>,
    stroke: Option<(Reactive<Color>, Reactive<f64>)>,
    inset: Reactive<f64>,
    rotate: Reactive<f64>,
    scale: Reactive<f64>,
    offset: (Reactive<f64>, Reactive<f64>),
    /// Unit-space sub-rect of the bounds this shape resolves in (`.at`).
    at: Option<Rect>,
}

/// A shape piece — one data-oriented piece parameterised by `ShapeKind`, rendered atop the canvas.
pub struct ShapePiece {
    spec: ShapeSpec,
    on_tap: Option<Rc<dyn Fn()>>,
    on_drag: Option<Rc<dyn Fn(Drag)>>,
}

/// The unified constructor: `shape(ShapeKind::RoundedRectangle { corner: 12.0.into() })`.
pub fn shape<M>(kind: impl IntoReactive<ShapeKind, M>) -> ShapePiece {
    ShapePiece {
        spec: ShapeSpec {
            kind: kind.into_reactive(),
            fill: None,
            fill_linear: None,
            fill_radial: None,
            stroke: None,
            inset: Reactive::Const(0.0),
            rotate: Reactive::Const(0.0),
            scale: Reactive::Const(1.0),
            offset: (Reactive::Const(0.0), Reactive::Const(0.0)),
            at: None,
        },
        on_tap: None,
        on_drag: None,
    }
}
/// SwiftUI-ergonomic sugar — all build the same `ShapePiece`.
pub fn rectangle() -> ShapePiece {
    shape(ShapeKind::Rectangle)
}
pub fn circle() -> ShapePiece {
    shape(ShapeKind::Circle)
}
pub fn ellipse() -> ShapePiece {
    shape(ShapeKind::Ellipse)
}
pub fn capsule() -> ShapePiece {
    shape(ShapeKind::Capsule)
}
pub fn rounded_rectangle(corner: impl Into<Corner>) -> ShapePiece {
    shape(ShapeKind::RoundedRectangle {
        corner: corner.into(),
    })
}
pub fn arc(start_deg: f64, sweep_deg: f64) -> ShapePiece {
    shape(ShapeKind::Arc {
        start_deg,
        sweep_deg,
    })
}
/// A stroked segment between two unit points of the frame: `line((0.16, 0.5), (0.84, 0.5))`.
pub fn line(from: (f64, f64), to: (f64, f64)) -> ShapePiece {
    shape(ShapeKind::Line {
        from: UnitPoint::new(from.0, from.1),
        to: UnitPoint::new(to.0, to.1),
    })
}
/// A polygon of unit points of the frame: `polygon([(0.5, 0.0), (1.0, 1.0), (0.0, 1.0)])`.
pub fn polygon(points: impl IntoIterator<Item = (f64, f64)>) -> ShapePiece {
    shape(ShapeKind::Polygon {
        points: points
            .into_iter()
            .map(|(x, y)| UnitPoint::new(x, y))
            .collect(),
    })
}

impl ShapePiece {
    pub fn fill<M>(mut self, p: impl IntoReactive<Color, M>) -> Self {
        self.spec.fill = Some(p.into_reactive());
        self
    }
    /// Fill with a [`LinearGradient`] (unit points resolve against the shape's bounds). Takes
    /// precedence over [`Self::fill`] when both are set; reactive like every other property:
    /// `rectangle().fill_linear(move || sky_gradient(state.get()))`.
    pub fn fill_linear<M>(mut self, g: impl IntoReactive<LinearGradient, M>) -> Self {
        self.spec.fill_linear = Some(g.into_reactive());
        self
    }
    /// Fill with a [`RadialGradient`] (center + radius in the unit space of the shape's bounds,
    /// stretching elliptically in non-square bounds). Precedence: radial over linear over solid.
    pub fn fill_radial<M>(mut self, g: impl IntoReactive<RadialGradient, M>) -> Self {
        self.spec.fill_radial = Some(g.into_reactive());
        self
    }
    pub fn stroke<M1, M2>(
        mut self,
        color: impl IntoReactive<Color, M1>,
        width: impl IntoReactive<f64, M2>,
    ) -> Self {
        self.spec.stroke = Some((color.into_reactive(), width.into_reactive()));
        self
    }
    /// Uniform inset applied before resolving geometry (keeps strokes inside the frame).
    pub fn inset<M>(mut self, v: impl IntoReactive<f64, M>) -> Self {
        self.spec.inset = v.into_reactive();
        self
    }
    /// Rotate the drawn shape about its centre, in degrees.
    pub fn rotate<M>(mut self, deg: impl IntoReactive<f64, M>) -> Self {
        self.spec.rotate = deg.into_reactive();
        self
    }
    /// Scale the drawn shape about its centre (uniform).
    pub fn scale<M>(mut self, s: impl IntoReactive<f64, M>) -> Self {
        self.spec.scale = s.into_reactive();
        self
    }
    /// Translate the drawn shape within its frame.
    pub fn offset<M1, M2>(
        mut self,
        x: impl IntoReactive<f64, M1>,
        y: impl IntoReactive<f64, M2>,
    ) -> Self {
        self.spec.offset = (x.into_reactive(), y.into_reactive());
        self
    }
    /// Resolve this shape inside the fractional sub-rect `(fx, fy, fw, fh)` of its bounds —
    /// unit-space, applied before [`Self::inset`]. The workhorse for composing glyphs in a
    /// [`shape_group`], mirroring hand-drawn `Rect::new(ox + fx * s, oy + fy * s, …)` canvas code.
    pub fn at(mut self, fx: f64, fy: f64, fw: f64, fh: f64) -> Self {
        self.spec.at = Some(Rect::new(fx, fy, fw, fh));
        self
    }
    /// Fire when the shape is tapped (path-precise — the tap is tested against the resolved path).
    pub fn on_tap(mut self, f: impl Fn() + 'static) -> Self {
        self.on_tap = Some(Rc::new(f));
        self
    }
    /// Fire on each phase of a drag over the shape.
    pub fn on_drag(mut self, f: impl Fn(Drag) + 'static) -> Self {
        self.on_drag = Some(Rc::new(f));
        self
    }
}

/// Compose the shape's rotate/scale (about its centre) + offset into one affine.
fn shape_transform(rect: Rect, rot_deg: f64, scale: f64, ox: f64, oy: f64) -> Affine {
    let c = rect.center();
    Affine::translate(-c.x, -c.y)
        .then(Affine::scale(scale, scale))
        .then(Affine::rotate(rot_deg.to_radians()))
        .then(Affine::translate(c.x, c.y))
        .then(Affine::translate(ox, oy))
}

/// Map an `.at` unit-space sub-rect into `bounds` (identity when unset).
fn resolve_at(at: Option<Rect>, bounds: Rect) -> Rect {
    match at {
        Some(u) => Rect::new(
            bounds.origin.x + u.origin.x * bounds.size.width,
            bounds.origin.y + u.origin.y * bounds.size.height,
            u.size.width * bounds.size.width,
            u.size.height * bounds.size.height,
        ),
        None => bounds,
    }
}

/// Record one shape description into `d`, resolved within `bounds` — shared by
/// [`ShapePiece`]'s own canvas leaf and by [`shape_group`] / [`shape_group_fn`].
fn record_shape(spec: &ShapeSpec, d: &mut Draw, bounds: Rect) {
    let bounds = resolve_at(spec.at, bounds);
    let kind = spec.kind.get();
    // A centered stroke overflows the geometry by half its width; inset closed shapes by w/2 so
    // the whole stroke stays inside the view bounds — backends that clip a canvas to its bounds
    // (Qt/Android/WinUI) would otherwise cut the stroke's outer edge. (SwiftUI `strokeBorder`
    // behavior.) Fill-only shapes are unaffected (stroke_half = 0). Line/Polygon are exempt:
    // they resolve exactly at their authored unit points, and a line's rect is legitimately
    // degenerate (zero-height for a horizontal segment), so they skip the empty-rect bail too.
    let open = matches!(kind, ShapeKind::Line { .. } | ShapeKind::Polygon { .. });
    let stroke_half = if open {
        0.0
    } else {
        spec.stroke
            .as_ref()
            .map(|(_, w)| w.get() / 2.0)
            .unwrap_or(0.0)
    };
    let rect = bounds.inset(spec.inset.get() + stroke_half);
    if !open && (rect.size.width <= 0.0 || rect.size.height <= 0.0) {
        return;
    }
    let geom = kind.geometry(rect);
    let m = shape_transform(
        rect,
        spec.rotate.get(),
        spec.scale.get(),
        spec.offset.0.get(),
        spec.offset.1.get(),
    );
    let transformed = !m.is_identity();
    if transformed {
        d.save();
        d.concat(m);
    }
    if !matches!(geom, Shape::Line(..)) {
        if let Some(g) = &spec.fill_radial {
            d.fill(geom.clone(), g.get());
        } else if let Some(g) = &spec.fill_linear {
            d.fill(geom.clone(), g.get());
        } else if let Some(fill) = &spec.fill {
            d.fill(geom.clone(), fill.get());
        }
    }
    if let Some((c, w)) = &spec.stroke {
        d.stroke(geom, c.get(), w.get());
    }
    if transformed {
        d.restore();
    }
}

/// A shape greedily fills its proposed size (SwiftUI semantics).
fn shape_flex() -> Flex {
    Flex {
        grow_w: true,
        grow_h: true,
        ..Default::default()
    }
}

/// Flatten many shape descriptions into ONE canvas leaf — one native view no matter how many
/// shapes (docs/shapes.md §3.6). Shapes draw in order; reactive properties re-record the group.
/// Child gestures are not wired inside a group — put `.on_tap` on the group via [`Decorate`].
pub fn shape_group(shapes: impl IntoIterator<Item = ShapePiece>) -> AnyPiece {
    let specs: Vec<ShapeSpec> = shapes.into_iter().map(|s| s.spec).collect();
    piece_fn(move |cx| {
        canvas_leaf(cx, shape_flex(), move |d, size| {
            let bounds = Rect::from_size(size);
            for spec in &specs {
                record_shape(spec, d, bounds);
            }
        })
    })
}

/// Size-aware [`shape_group`]: the closure derives the shapes from the laid-out size and re-runs
/// on `FrameChanged`, exactly like [`canvas`] — for geometry that depends on the final size
/// (e.g. data mapped along the width).
pub fn shape_group_fn(shapes: impl Fn(Size) -> Vec<ShapePiece> + 'static) -> AnyPiece {
    piece_fn(move |cx| {
        canvas_leaf(cx, shape_flex(), move |d, size| {
            let bounds = Rect::from_size(size);
            for piece in shapes(size) {
                record_shape(&piece.spec, d, bounds);
            }
        })
    })
}

impl Piece for ShapePiece {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let ShapePiece {
            spec,
            on_tap,
            on_drag,
        } = self;

        let draw_spec = spec.clone();
        let node = canvas_leaf(cx, shape_flex(), move |d, size| {
            record_shape(&draw_spec, d, Rect::from_size(size));
        });

        // Path-precise tap: inverse-transform the point, then test against the resolved geometry.
        if let Some(on_tap) = on_tap {
            with_tree(|t| t.enable_gesture(node, GestureKind::Tap));
            cx.on(node, move |ev| {
                if let Event::Tap(p) = ev
                    && let Some(f) = with_tree(|t| t.node_frame(node))
                {
                    let bounds = resolve_at(spec.at, Rect::from_size(f.size));
                    let rect = bounds.inset(spec.inset.get_untracked());
                    let m = shape_transform(
                        rect,
                        spec.rotate.get_untracked(),
                        spec.scale.get_untracked(),
                        spec.offset.0.get_untracked(),
                        spec.offset.1.get_untracked(),
                    );
                    let local = m.invert_apply(*p).unwrap_or(*p);
                    if spec.kind.get_untracked().contains(rect, local) {
                        on_tap();
                    }
                }
            });
        }
        if let Some(on_drag) = on_drag {
            with_tree(|t| t.enable_gesture(node, GestureKind::Drag));
            cx.on(node, move |ev| {
                if let Event::Drag {
                    phase,
                    location,
                    translation,
                } = ev
                {
                    on_drag(Drag {
                        phase: *phase,
                        location: *location,
                        translation: *translation,
                    });
                }
            });
        }
        node
    }
}

// ---------------------------------------------------------------------------
// Image (§18.2, MVP): sources resolve via DAY_ASSET_ROOT (desktop dev), the app
// bundle (ios), or AssetManager (android).
// ---------------------------------------------------------------------------

/// A bundled image, resolved by name through the backend's native image pipeline (§18.3). Scales
/// with [`ContentMode::Fit`] by default (never stretches); tune with `.content_mode()` / `.fill()` /
/// `.stretch()`, and optionally constrain the frame with `.aspect_ratio(w/h)`.
pub struct Image {
    source: String,
    content_mode: ContentMode,
    aspect_ratio: Option<f64>,
    decorative: bool,
}

pub fn image(name: impl Into<day_spec::ImageName>) -> Image {
    Image {
        source: name.into().as_str().to_owned(),
        content_mode: ContentMode::default(),
        aspect_ratio: None,
        decorative: false,
    }
}

impl Image {
    /// How the image scales within its frame (default [`ContentMode::Fit`]).
    pub fn content_mode(mut self, m: ContentMode) -> Self {
        self.content_mode = m;
        self
    }
    /// Scale to fit entirely inside the frame, preserving aspect ratio (the default).
    pub fn fit(self) -> Self {
        self.content_mode(ContentMode::Fit)
    }
    /// Scale to fill the frame, preserving aspect ratio and cropping the overflow.
    pub fn fill(self) -> Self {
        self.content_mode(ContentMode::Fill)
    }
    /// Stretch to fill the frame exactly, ignoring aspect ratio.
    pub fn stretch(self) -> Self {
        self.content_mode(ContentMode::Stretch)
    }
    /// Constrain the view to a `width / height` ratio (e.g. `16.0 / 9.0`).
    pub fn aspect_ratio(mut self, ratio: f64) -> Self {
        if ratio > 0.0 {
            self.aspect_ratio = Some(ratio);
        }
        self
    }
    /// Mark the image decorative (hidden from accessibility).
    pub fn decorative(mut self) -> Self {
        self.decorative = true;
        self
    }
}

impl Piece for Image {
    fn build(self, cx: &mut BuildCx) -> day_core::RNode {
        let props = ImageProps {
            source: self.source,
            decorative: self.decorative,
            content_mode: self.content_mode,
            aspect_ratio: self.aspect_ratio,
        };
        match self.aspect_ratio {
            Some(ratio) => cx.native(
                kinds::IMAGE,
                &props,
                std::rc::Rc::new(AspectRatioLayout { ratio }),
                Flex::default(),
                day_core::Boundary::No,
            ),
            None => cx.leaf(kinds::IMAGE, &props, Flex::default()),
        }
    }
}

/// Self-measuring layout for `.aspect_ratio(r)`: reports the largest `width/height == r` box that
/// fits the proposal (SwiftUI's `.aspectRatio(_:contentMode: .fit)`).
struct AspectRatioLayout {
    ratio: f64,
}
impl day_core::Layout for AspectRatioLayout {
    fn measure(
        &self,
        cx: &mut dyn day_core::LayoutOps,
        _children: &[day_core::RNode],
        p: day_geometry::Proposal,
    ) -> day_geometry::Size {
        match (p.width, p.height) {
            (Some(w), Some(h)) => {
                if w / h > self.ratio {
                    day_geometry::Size::new(h * self.ratio, h)
                } else {
                    day_geometry::Size::new(w, w / self.ratio)
                }
            }
            (Some(w), None) => day_geometry::Size::new(w, w / self.ratio),
            (None, Some(h)) => day_geometry::Size::new(h * self.ratio, h),
            (None, None) => cx.measure_leaf(p),
        }
    }
    fn place(
        &self,
        _cx: &mut dyn day_core::LayoutOps,
        _children: &[day_core::RNode],
        _bounds: day_geometry::Rect,
    ) {
    }
}

// ---------------------------------------------------------------------------
// Navigation & tabs (docs/navigation.md, docs/tabs.md) — selector + stack, each a
// projection of an app-owned Signal.
// ---------------------------------------------------------------------------

/// Navigate to a route (docs/navigation.md).
///
/// * A single key (`navigate("inbox")`) is RELATIVE — the innermost route surface is tried
///   first, falling through outward; `""` pops the innermost stack to its root.
/// * A `/`-separated path (`navigate("mail/inbox/msg-42")`) is ABSOLUTE — anchored at the
///   outermost surface that knows the first segment, everything inside reset, the remaining
///   segments consumed inward (surfaces mounting during the cascade take theirs as they appear).
/// * A trailing `?name=value&…` carries [`route_params`] to the destination builders.
///
/// False = no surface recognized the (first) segment.
pub fn navigate(path: &str) -> bool {
    day_core::navigate(path)
}

/// Pop one navigation level. False = nothing to pop.
pub fn nav_back() -> bool {
    day_core::nav_back()
}

/// The FULL current route — every mounted surface's contribution, outermost to innermost,
/// `/`-joined. Round-trips through [`navigate`]: persist it on exit, `navigate(&saved)` on
/// launch (docs/navigation.md).
pub fn current_route() -> Option<String> {
    day_core::current_route()
}

/// The query params of the most recent [`navigate`] (`?name=value&…`) — read inside a
/// destination builder. See docs/navigation.md for when params apply.
pub fn route_params() -> std::rc::Rc<Vec<(String, String)>> {
    day_core::route_params()
}

/// One query param of the most recent [`navigate`] (`None` = not present).
pub fn route_param(name: &str) -> Option<String> {
    day_core::route_param(name)
}

/// A tappable link that navigates to `path` when pressed.
pub fn nav_link<M>(label: impl IntoText<M>, path: &str) -> Button {
    let path = path.to_string();
    button(label).action(move || {
        let _ = day_core::navigate(&path);
    })
}

// ---------------------------------------------------------------------------
// Typed routes (docs/navigation.md) — routes as data instead of string encoding.
// ---------------------------------------------------------------------------

/// A typed route key — the compile-checked alternative to raw string keys.
///
/// Implement on an enum (one variant per destination) and use it everywhere a key goes:
/// `selector(Signal<Option<Section>>)` + `.item(Section::Controls, …)`,
/// `stack(Signal<Vec<Drill>>, …)` + `.destination(|d: &Drill| …)`, [`navigate_to`], [`route`].
/// The string layer stays the wire format — deep links, dayscript, and [`current_route`]
/// still speak [`Route::key`] strings — but app code never assembles or splits them.
///
/// Variants can carry data (`Item { id: u32 }` ↔ `"item-42"`): encode it in [`Route::key`],
/// parse it back in [`Route::from_key`], and destination builders receive the typed value.
/// For plain data-free enums the [`routes!`] macro writes both sides.
pub trait Route: Clone + PartialEq + 'static {
    /// The path segment this value occupies in a route string. Must round-trip through
    /// [`Route::from_key`] and must not be empty — `""` means "no selection" (see the
    /// `Option<R>` impl).
    fn key(&self) -> String;
    /// Parse a path segment back into the typed value; `None` = not one of this type's routes.
    fn from_key(key: &str) -> Option<Self>;
    /// The human-readable title shown in the native navigation bar when this route is the top of
    /// a [`stack`]. Defaults to [`key`](Route::key); override it to show a display name (e.g. an
    /// app's name) instead of the wire key.
    fn title(&self) -> String {
        self.key()
    }
}

/// Raw string keys — the untyped baseline. Every segment parses.
impl Route for String {
    fn key(&self) -> String {
        self.clone()
    }
    fn from_key(key: &str) -> Option<Self> {
        Some(key.to_string())
    }
}

/// `None` ↔ `""` (no selection) — the key type for a sidebar [`selector`], whose collapsed
/// mobile state IS "nothing selected". `.item(Section::X, …)` still takes the bare value
/// (`Section: Into<Option<Section>>`).
impl<R: Route> Route for Option<R> {
    fn key(&self) -> String {
        match self {
            Some(r) => r.key(),
            None => String::new(),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        if key.is_empty() {
            Some(None)
        } else {
            R::from_key(key).map(Some)
        }
    }
}

/// Define a plain routes enum and its [`Route`] impl in one shot:
///
/// ```ignore
/// day::routes! {
///     pub enum Section { Controls => "controls", Text => "text" }
/// }
/// selector(section).item(Section::Controls, tr("controls"), controls_page)
/// ```
///
/// Variants that carry data (`Item { id: u32 }` ↔ `"item-42"`) implement [`Route`] by hand.
#[macro_export]
macro_rules! routes {
    ($(#[$meta:meta])* $vis:vis enum $name:ident {
        $($(#[$vmeta:meta])* $variant:ident => $key:literal),+ $(,)?
    }) => {
        $(#[$meta])*
        #[derive(Clone, Copy, PartialEq, Eq, Debug)]
        $vis enum $name { $($(#[$vmeta])* $variant),+ }
        impl $crate::Route for $name {
            fn key(&self) -> String {
                match self { $(Self::$variant => ($key).to_string()),+ }
            }
            fn from_key(key: &str) -> Option<Self> {
                match key { $($key => Some(Self::$variant),)+ _ => None }
            }
        }
    };
}

/// A typed absolute route: segments built from [`Route`] values plus query params.
/// `route(&Section::Stack).then(&Drill::Item { id: 42 }).param("hint", "linked")` encodes to
/// `"stack/item-42?hint=linked"` — [`RoutePath::navigate`] it, or hand it to [`nav_link_to`].
#[derive(Clone, Debug, Default)]
pub struct RoutePath {
    segments: Vec<String>,
    params: Vec<(String, String)>,
}

/// Start a typed [`RoutePath`] at the outermost segment.
pub fn route(first: &impl Route) -> RoutePath {
    RoutePath {
        segments: vec![first.key()],
        params: Vec::new(),
    }
}

impl RoutePath {
    /// Append the next-inner segment.
    pub fn then(mut self, next: &impl Route) -> Self {
        self.segments.push(next.key());
        self
    }
    /// Append a query param (the destination reads it via [`route_param`]).
    pub fn param(mut self, name: &str, value: impl std::fmt::Display) -> Self {
        self.params.push((name.to_string(), value.to_string()));
        self
    }
    /// The encoded route string (percent-escaped where needed) — what [`navigate`] accepts.
    pub fn to_route(&self) -> String {
        day_core::encode_route(&self.segments, &self.params)
    }
    /// Navigate to this path. False = no surface recognized the first segment.
    pub fn navigate(&self) -> bool {
        day_core::navigate(&self.to_route())
    }
}

impl std::fmt::Display for RoutePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_route())
    }
}

/// Navigate to a single typed key, RELATIVE (innermost surface first) — the typed
/// `navigate(&r.key())`, percent-escaped. For absolute paths chain a [`route`].
pub fn navigate_to(r: &impl Route) -> bool {
    day_core::navigate(&day_core::encode_route(std::slice::from_ref(&r.key()), &[]))
}

/// A tappable link that navigates to a typed [`RoutePath`] when pressed.
pub fn nav_link_to<M>(label: impl IntoText<M>, path: RoutePath) -> Button {
    let path = path.to_route();
    button(label).action(move || {
        let _ = day_core::navigate(&path);
    })
}

// ---------------------------------------------------------------------------
// Nested-nav merge (docs/navigation.md): a `stack()` built inside a page of an enclosing NAV
// host that presents as a push stack (mobile, `split == false`) pushes its pages onto THAT host
// instead of minting a second native container — one native nav chain, one back button. The
// enclosing host is threaded to nested pieces at build time via a thread-local context stack;
// `owners` is the per-host ordered stack of "what a back on the topmost page does".
// ---------------------------------------------------------------------------

/// Performs the topmost page's back action. Arg = the toolkit already popped natively (iOS/Android
/// system back), so the owner must not re-issue a pop.
type PopOwner = Rc<dyn Fn(bool)>;

#[derive(Clone)]
struct NavHostCx {
    host: RNode,
    sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>>,
    /// One entry per page pushed above the root, in native order; the host's single `NavBack`
    /// handler invokes the last.
    owners: Rc<RefCell<Vec<PopOwner>>>,
    /// The enclosing host presents as split panes (desktop). A nested stack does NOT merge into a
    /// split host — it keeps its own detail-pane stack.
    split: bool,
}

thread_local! {
    /// Build-time stack of enclosing nav hosts. `None` is a barrier (a resident container such as
    /// tabs) that a nested stack must not merge through.
    static NAV_HOST_CX: RefCell<Vec<Option<NavHostCx>>> = const { RefCell::new(Vec::new()) };
}

/// Run `f` with `cx` as the innermost nav-host context (a barrier when `None`), restoring after.
fn with_nav_host<R>(cx: Option<NavHostCx>, f: impl FnOnce() -> R) -> R {
    NAV_HOST_CX.with(|s| s.borrow_mut().push(cx));
    let r = f();
    NAV_HOST_CX.with(|s| {
        s.borrow_mut().pop();
    });
    r
}

/// The innermost mergeable nav host, if any (a barrier or an empty stack yields `None`).
fn current_nav_host() -> Option<NavHostCx> {
    NAV_HOST_CX.with(|s| s.borrow().last().cloned().flatten())
}

/// Create a NAV_PAGE under `host` and wire its FrameChanged size reports into `sizes`
/// (the native container owns each page's frame; Day lays content out at the reported size).
fn nav_page(
    host: RNode,
    props: &day_spec::props::NavPageProps,
    sizes: &Rc<RefCell<std::collections::HashMap<RNode, Size>>>,
) -> RNode {
    let mut cx = BuildCx::new(host);
    let page = cx.native(
        kinds::NAV_PAGE,
        props,
        Rc::new(PassThrough),
        Flex::default(),
        Boundary::Yes,
    );
    let sizes = sizes.clone();
    cx.on(page, move |ev| {
        if let Event::FrameChanged(sz) = ev {
            let changed = sizes.borrow().get(&page) != Some(sz);
            if changed {
                sizes.borrow_mut().insert(page, *sz);
                with_tree(|t| {
                    t.mark_needs_measure(page);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        }
    });
    page
}

/// Create a TABS_PAGE under `host`, wiring its FrameChanged reports into `sizes`.
fn tabs_page(
    host: RNode,
    props: &day_spec::props::TabsPageProps,
    sizes: &Rc<RefCell<std::collections::HashMap<RNode, Size>>>,
) -> RNode {
    let mut cx = BuildCx::new(host);
    let page = cx.native(
        kinds::TABS_PAGE,
        props,
        Rc::new(PassThrough),
        Flex::default(),
        Boundary::Yes,
    );
    let sizes = sizes.clone();
    cx.on(page, move |ev| {
        if let Event::FrameChanged(sz) = ev {
            let changed = sizes.borrow().get(&page) != Some(sz);
            if changed {
                sizes.borrow_mut().insert(page, *sz);
                with_tree(|t| {
                    t.mark_needs_measure(page);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        }
    });
    page
}

/// Register a string-route adapter over a route surface's own signal, so `navigate()` /
/// deep links / dayscript keep working by key. This is a *convenience layer* — the surface
/// itself is driven by the signal, not by this registry (docs/navigation.md).
///
/// `enter` consumes one segment of an ABSOLUTE path (`navigate("a/b/c")`); `segments` is the
/// surface's contribution to the full [`current_route`].
fn register_route_surface(
    push: impl Fn(&str) -> bool + 'static,
    pop: impl Fn(bool) -> bool + 'static,
    current: impl Fn() -> String + 'static,
    enter: impl Fn(&str) -> bool + 'static,
    segments: impl Fn() -> Vec<String> + 'static,
) {
    let token = day_core::register_nav(day_core::NavController {
        push: Box::new(push),
        pop: Box::new(pop),
        current: Box::new(current),
        enter: Box::new(enter),
        segments: Box::new(segments),
    });
    Scope::current().on_cleanup(move || day_core::unregister_nav(token));
}

// ===========================================================================
// Selector — one-of-N, bound to a Signal<String> of the active key.
// ===========================================================================

/// How a [`selector`] presents its one-of-N choice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SelectorStyle {
    /// A native tab widget: NSTabView / UITabBarController / GtkNotebook / QTabWidget /
    /// Android tab strip. All pages resident; each keeps its state.
    Tabs,
    /// A NavigationSplitView: a sidebar list + a detail. Desktop shows both panes (on GTK an
    /// `AdwNavigationSplitView`); mobile collapses to a list that pushes the detail.
    Sidebar,
}

struct SelItem<K> {
    key: K,
    title: TextSource,
    /// Optional bundled-image name for the item's native icon (docs/navigation.md).
    icon: Option<String>,
    build: Box<dyn Fn() -> AnyPiece>,
}

/// A sidebar item resolved for the detail switcher: (encoded key, resolved title, lazy builder).
type ResolvedItems = Rc<Vec<(String, String, Box<dyn Fn() -> AnyPiece>)>>;

/// A one-of-N selector whose active key is an app-owned signal (two-way, exactly like
/// `Picker`/`Toggle`). Deep links and dayscript address items by key (docs/navigation.md).
///
/// The key type is any [`Route`]: `String` for raw keys, or a typed enum — use
/// `Signal<Option<Section>>` for a sidebar (`None` = the collapsed mobile list) and
/// `Signal<Tab>` for tabs (always selected).
///
/// ```ignore
/// let section = Signal::new("home".to_string());   // or Signal::new(None::<Section>)
/// selector(section).style(SelectorStyle::Sidebar)
///     .item("home", tr("home"), home_page)         // or .item(Section::Home, …)
///     .item("settings", tr("settings"), settings_page)
/// ```
pub struct Selector<S: SignalRw<K>, K: Route = String> {
    selection: S,
    style: SelectorStyle,
    title: TextSource,
    header: Option<Box<dyn FnOnce() -> AnyPiece>>,
    items: Vec<SelItem<K>>,
}

pub fn selector<K: Route, S: SignalRw<K>>(selection: S) -> Selector<S, K> {
    Selector {
        selection,
        style: SelectorStyle::Sidebar,
        title: TextSource::Static(String::new()),
        header: None,
        items: Vec::new(),
    }
}

impl<K: Route, S: SignalRw<K>> Selector<S, K> {
    pub fn style(mut self, style: SelectorStyle) -> Self {
        self.style = style;
        self
    }
    /// The sidebar / window title (Sidebar style).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text();
        self
    }
    /// An optional piece shown above the sidebar list (a logo, app name…).
    pub fn header<P: Piece>(mut self, build: impl FnOnce() -> P + 'static) -> Self {
        self.header = Some(Box::new(move || AnyPiece::new(build())));
        self
    }
    /// Add a destination. `key` addresses it (navigate / deep link / dayscript); `title` is
    /// its label; `build` runs when the item is first shown. For a typed selector over
    /// `Option<Section>` pass the bare `Section::X`.
    pub fn item<M, P: Piece>(
        mut self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        self.items.push(SelItem {
            key: key.into(),
            title: title.into_text(),
            icon: None,
            build: Box::new(move || AnyPiece::new(build())),
        });
        self
    }
    /// Like [`item`](Self::item) but with a native icon: `icon` is a bundled-image name (typed
    /// [`ImageName`](day_spec::ImageName), resolved like [`image`], e.g. `res::images::nav_home`)
    /// shown beside the label where the backend's nav supports it (e.g. the Windows
    /// NavigationView, the iOS/macOS source list). Backends that can't decorate rows ignore it.
    pub fn item_icon<M, P: Piece>(
        mut self,
        key: impl Into<K>,
        title: impl IntoText<M>,
        icon: impl Into<day_spec::ImageName>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        self.items.push(SelItem {
            key: key.into(),
            title: title.into_text(),
            icon: Some(icon.into().as_str().to_owned()),
            build: Box::new(move || AnyPiece::new(build())),
        });
        self
    }
}

impl<K: Route, S: SignalRw<K>> Piece for Selector<S, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        match self.style {
            SelectorStyle::Tabs => build_tabs(self, cx),
            SelectorStyle::Sidebar => build_sidebar(self, cx),
        }
    }
}

fn build_tabs<K: Route, S: SignalRw<K>>(sel: Selector<S, K>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{TabsPageProps, TabsPatch, TabsProps};
    let selection = sel.selection;
    let metas: Vec<(String, String)> = sel
        .items
        .iter()
        .map(|it| (it.key.key(), it.title.initial()))
        .collect();
    let titles: Vec<String> = metas.iter().map(|(_, t)| t.clone()).collect();
    let icons: Vec<Option<String>> = sel.items.iter().map(|it| it.icon.clone()).collect();
    let keys: Rc<Vec<String>> = Rc::new(metas.iter().map(|(k, _)| k.clone()).collect());
    let typed: Rc<Vec<K>> = Rc::new(sel.items.iter().map(|it| it.key.clone()).collect());
    let initial = selection.get_untracked_rw().key();
    let initial_idx = keys.iter().position(|k| *k == initial).unwrap_or(0);

    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let host = cx.native(
        kinds::TABS,
        &TabsProps {
            titles,
            icons,
            selected: initial_idx,
        },
        Rc::new(NavLayout {
            sizes: sizes.clone(),
            split: false,
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        Boundary::Yes,
    );
    for (i, it) in sel.items.into_iter().enumerate() {
        let page = tabs_page(
            host,
            &TabsPageProps {
                title: metas[i].1.clone(),
                icon: it.icon.clone(),
            },
            &sizes,
        );
        let content = (it.build)();
        // Barrier: tabs are resident, not a push stack, so a stack inside a tab must not merge
        // through this container into an outer nav host — it keeps its own (docs/navigation.md).
        with_nav_host(None, || {
            let mut pcx = BuildCx::new(page);
            let _ = content.build(&mut pcx);
        });
    }

    // Two-way: signal → native selection (skip the echo of a native tap).
    let echo: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    {
        let (keys, echo, s) = (keys.clone(), echo.clone(), selection.clone());
        bind_seeded(
            initial_idx,
            move || {
                let cur = s.get_rw().key();
                keys.iter().position(|k| *k == cur).unwrap_or(0)
            },
            move |idx: &usize| {
                if echo.replace(None) == Some(*idx) {
                    return;
                }
                with_tree(|t| t.patch(host, Box::new(TabsPatch::Selected(*idx)), false));
            },
        );
    }
    // native selection → signal
    {
        let (typed, echo, s) = (typed.clone(), echo.clone(), selection.clone());
        cx.on(host, move |ev| match ev {
            Event::SelectionChanged(i) if *i >= 0 => {
                let idx = *i as usize;
                if let Some(k) = typed.get(idx) {
                    echo.set(Some(idx));
                    s.set_rw(k.clone());
                }
            }
            Event::Custom {
                tag: "deeplink",
                text: route,
                ..
            } => {
                let _ = day_core::navigate(route);
            }
            _ => {}
        });
    }
    // string-route adapter (the typed key decodes at this boundary; app code stays typed)
    let (ks_push, ts_push, s_push) = (keys.clone(), typed.clone(), selection.clone());
    let s_cur = selection.clone();
    let (ks_enter, ts_enter, s_enter) = (keys.clone(), typed.clone(), selection.clone());
    let s_seg = selection.clone();
    register_route_surface(
        move |k| {
            if let Some(i) = ks_push.iter().position(|x| x == k) {
                s_push.set_rw(ts_push[i].clone());
                true
            } else {
                false
            }
        },
        |_| false,
        move || s_cur.get_untracked_rw().key(),
        // Absolute-path segment: same as push — a tab key is a declared key.
        move |k| {
            if let Some(i) = ks_enter.iter().position(|x| x == k) {
                s_enter.set_rw(ts_enter[i].clone());
                true
            } else {
                false
            }
        },
        move || {
            let k = s_seg.get_untracked_rw().key();
            if k.is_empty() { Vec::new() } else { vec![k] }
        },
    );
    host
}

fn build_sidebar<K: Route, S: SignalRw<K>>(sel: Selector<S, K>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{NavMenuPatch, NavMenuProps, NavPageProps, NavPatch, NavProps};
    let split = with_tree(|t| t.capability(day_spec::Cap::NavSplit)) == day_spec::Support::Native;
    let selection = sel.selection;
    let title_s = sel.title.initial();
    let metas: Vec<(String, String)> = sel
        .items
        .iter()
        .map(|it| (it.key.key(), it.title.initial()))
        .collect();
    let keys: Rc<Vec<String>> = Rc::new(metas.iter().map(|(k, _)| k.clone()).collect());
    let typed: Rc<Vec<K>> = Rc::new(sel.items.iter().map(|it| it.key.clone()).collect());
    let titles: Vec<String> = metas.iter().map(|(_, t)| t.clone()).collect();
    let icons: Vec<Option<String>> = sel.items.iter().map(|it| it.icon.clone()).collect();
    let builders: ResolvedItems = Rc::new(
        sel.items
            .into_iter()
            .enumerate()
            .map(|(i, it)| (metas[i].0.clone(), metas[i].1.clone(), it.build))
            .collect(),
    );

    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let host = cx.native(
        kinds::NAV,
        &NavProps {
            title: title_s.clone(),
            split,
        },
        Rc::new(NavLayout {
            sizes: sizes.clone(),
            split,
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        Boundary::Yes,
    );

    // The per-host back-owner stack (docs/navigation.md): the detail page pushes its "deselect"
    // owner, and a nested stack that merges into this host pushes its page owners on top. The
    // context is threaded to nested pieces built under our pages.
    let owners: Rc<RefCell<Vec<PopOwner>>> = Rc::default();
    let host_cx = NavHostCx {
        host,
        sizes: sizes.clone(),
        owners: owners.clone(),
        split,
    };

    // Sidebar / root page: optional header + native item list.
    let root_page = nav_page(
        host,
        &NavPageProps {
            title: title_s.clone(),
            sidebar: split,
        },
        &sizes,
    );
    let menu_holder: Rc<Cell<Option<RNode>>> = Rc::new(Cell::new(None));
    {
        let (mh, ks, s, titles2, icons2) = (
            menu_holder.clone(),
            typed.clone(),
            selection.clone(),
            titles.clone(),
            icons.clone(),
        );
        let menu_piece = piece_fn(move |mcx| {
            let node = mcx.native(
                kinds::NAV_MENU,
                &NavMenuProps {
                    items: titles2,
                    icons: icons2,
                    selected: None,
                },
                Rc::new(LeafLayout),
                Flex {
                    grow_w: true,
                    grow_h: true,
                    ..Default::default()
                },
                Boundary::No,
            );
            mh.set(Some(node));
            mcx.on(node, move |ev| {
                if let Event::SelectionChanged(i) = ev
                    && let Some(k) = ks.get(*i as usize)
                {
                    s.set_rw(k.clone());
                }
            });
            node
        });
        let content: AnyPiece = match sel.header {
            Some(h) => column((h(), menu_piece))
                .spacing(4.0)
                .align(HAlign::Leading)
                .any(),
            None => column((menu_piece,))
                .spacing(4.0)
                .align(HAlign::Leading)
                .any(),
        };
        with_nav_host(Some(host_cx.clone()), || {
            let mut pcx = BuildCx::new(root_page);
            let _ = content.build(&mut pcx);
        });
    }

    let sync_menu = {
        let mh = menu_holder.clone();
        move |idx: Option<usize>| {
            if let Some(m) = mh.get() {
                with_tree(|t| t.patch(m, Box::new(NavMenuPatch::Selected(idx)), false));
            }
        }
    };

    // Detail: `selection` drives which item's page is shown (reset-to; depth ≤ 1).
    let current: Rc<RefCell<Option<(String, Scope, RNode)>>> = Rc::default();
    let nav_scope = Scope::current();
    let show = {
        let (builders, current, sizes, keys, sync_menu, owners, host_cx, selection) = (
            builders.clone(),
            current.clone(),
            sizes.clone(),
            keys.clone(),
            sync_menu.clone(),
            owners.clone(),
            host_cx.clone(),
            selection.clone(),
        );
        move |key: &str| {
            if current.borrow().as_ref().map(|(k, _, _)| k.as_str()) == Some(key) {
                return;
            }
            if let Some((_, scope, page)) = current.borrow_mut().take() {
                // Dispose the detail scope FIRST: a merged inner stack's cleanup pops its pages
                // (which sit on top natively) before we pop the detail itself, so the native pop
                // order stays top-down (iOS pops the topmost VC; Android's INCLUSIVE pop unwinds
                // everything above an entry).
                scope.dispose();
                with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                owners.borrow_mut().pop();
                sizes.borrow_mut().remove(&page);
                with_tree(|t| {
                    t.remove_subtree(page);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
            if key.is_empty() {
                sync_menu(None);
                return;
            }
            let Some((_, page_title, build)) = builders.iter().find(|(k, _, _)| k == key) else {
                sync_menu(None);
                return;
            };
            let page = nav_page(
                host,
                &NavPageProps {
                    title: page_title.clone(),
                    sidebar: false,
                },
                &sizes,
            );
            // The detail page's back action = deselect (return to the list). Pushed BEFORE the
            // content builds, so a merged inner stack's page owners stack on top of it.
            let owner: PopOwner = {
                let s = selection.clone();
                Rc::new(move |_already_popped| {
                    if let Some(root) = K::from_key("") {
                        s.set_rw(root);
                    }
                })
            };
            owners.borrow_mut().push(owner);
            let scope = nav_scope.enter(Scope::child);
            let content = build();
            scope.enter(|| {
                with_nav_host(Some(host_cx.clone()), || {
                    let mut c = BuildCx::new(page);
                    let _ = content.build(&mut c);
                });
            });
            with_tree(|t| {
                t.patch(
                    host,
                    Box::new(NavPatch::Pushed {
                        title: page_title.clone(),
                    }),
                    false,
                );
                t.mark_layout_dirty();
                t.layout_if_needed();
            });
            *current.borrow_mut() = Some((key.to_string(), scope, page));
            sync_menu(keys.iter().position(|k| k == key));
        }
    };

    // Desktop split never shows an empty detail: default to the first item.
    if split
        && selection.get_untracked_rw().key().is_empty()
        && let Some(k) = typed.first()
    {
        selection.set_rw(k.clone());
    }
    {
        let s = selection.clone();
        bind(move || s.get_rw().key(), move |key: &String| show(key));
    }

    // Native back (mobile up-arrow / system back) → the topmost page's owner. With only this
    // sidebar on the host, that's always the detail's deselect owner (returns to the list); when
    // a nested stack has merged its pages on top, its owners run first (docs/navigation.md). A
    // typed key deselects via its "" decoding (`Option<Section>` → `None`); a bare enum has no
    // list-only state so its owner's deselect is a no-op — back is effectively ignored.
    {
        let owners = owners.clone();
        cx.on(host, move |ev| match ev {
            Event::NavBack { already_popped } => {
                let top = owners.borrow().last().cloned();
                if let Some(f) = top {
                    f(*already_popped);
                }
            }
            Event::Custom {
                tag: "deeplink",
                text: route,
                ..
            } => {
                let _ = day_core::navigate(route);
            }
            _ => {}
        });
    }

    // string-route adapter over `selection` (typed keys decode at this boundary)
    let (ks_push, ts_push, s_push) = (keys.clone(), typed.clone(), selection.clone());
    let s_pop = selection.clone();
    let s_cur = selection.clone();
    let (ks_enter, ts_enter, s_enter) = (keys.clone(), typed.clone(), selection.clone());
    let s_seg = selection.clone();
    register_route_surface(
        move |k| {
            if k.is_empty() {
                if let Some(root) = K::from_key("") {
                    s_push.set_rw(root);
                    true
                } else {
                    false // no empty state (bare-enum key) — let the parent handle ""
                }
            } else if let Some(i) = ks_push.iter().position(|x| x == k) {
                s_push.set_rw(ts_push[i].clone());
                true
            } else {
                false
            }
        },
        move |_| {
            if s_pop.get_untracked_rw().key().is_empty() {
                false
            } else if let Some(root) = K::from_key("") {
                s_pop.set_rw(root);
                true
            } else {
                false
            }
        },
        move || s_cur.get_untracked_rw().key(),
        // Absolute-path segment: a declared item key selects it (no "" — segments are non-empty).
        move |k| {
            if let Some(i) = ks_enter.iter().position(|x| x == k) {
                s_enter.set_rw(ts_enter[i].clone());
                true
            } else {
                false
            }
        },
        move || {
            let k = s_seg.get_untracked_rw().key();
            if k.is_empty() { Vec::new() } else { vec![k] }
        },
    );
    host
}

// ===========================================================================
// Stack — a genuine push/pop navigation stack bound to a Signal<Vec<String>>.
// The native UINavigationController / AdwNavigationView / back-stack is reconciled
// to the path; the back button writes the pop back into the path.
// ===========================================================================

struct StackEntry<K> {
    key: K,
    scope: Scope,
    page: RNode,
}

/// A push/pop navigation stack whose contents are an app-owned `Signal<Vec<K>>` (the path
/// above the root). Day reconciles the native stack to the path; the native back button
/// writes the pop back into it (docs/navigation.md).
///
/// The key type is any [`Route`]: `String` for raw keys, or a typed enum whose variants can
/// carry data — the destination builder then receives the typed value, and an absolute
/// `navigate("…/item-42")` parses each segment via [`Route::from_key`] (rejecting segments
/// that don't parse; `String` accepts everything).
///
/// ```ignore
/// let path = Signal::new(Vec::<Drill>::new());
/// stack(path.clone(), home_view).destination(|d: &Drill| detail_view(d))
/// // push:  path.update(|p| p.push(Drill::Item { id: 42 }));
/// ```
pub struct Stack<S: SignalRw<Vec<K>>, K: Route = String> {
    path: S,
    title: TextSource,
    root: AnyPiece,
    destination: Rc<dyn Fn(&K) -> AnyPiece>,
}

pub fn stack<K: Route, S: SignalRw<Vec<K>>>(path: S, root: impl Piece) -> Stack<S, K> {
    Stack {
        path,
        title: TextSource::Static(String::new()),
        root: AnyPiece::new(root),
        destination: Rc::new(|_| {
            piece_fn(|cx| cx.layout_only(Rc::new(PassThrough), Flex::default(), Boundary::No))
        }),
    }
}

impl<K: Route, S: SignalRw<Vec<K>>> Stack<S, K> {
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text();
        self
    }
    /// Build the view for a pushed key (`&String` for raw keys, the typed value otherwise).
    pub fn destination<P: Piece>(mut self, build: impl Fn(&K) -> P + 'static) -> Self {
        self.destination = Rc::new(move |k| AnyPiece::new(build(k)));
        self
    }
}

impl<K: Route, S: SignalRw<Vec<K>>> Piece for Stack<S, K> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::props::{NavPageProps, NavPatch, NavProps};
        let Stack {
            path,
            title,
            root,
            destination: dest,
        } = self;
        let title_s = title.initial();

        // If we're built inside a page of an enclosing NAV host that presents as a push stack
        // (mobile, `split == false`), MERGE: push our pages onto that host instead of minting a
        // second native container — one native nav chain, one back button (docs/navigation.md).
        // A split host (desktop) is not merged into; a stack keeps its own detail-pane stack.
        let merge = current_nav_host().filter(|c| !c.split);

        let entries: Rc<RefCell<Vec<StackEntry<K>>>> = Rc::default();
        let native_popped: Rc<Cell<usize>> = Rc::new(Cell::new(0));

        let host: RNode;
        let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>>;
        let owners: Rc<RefCell<Vec<PopOwner>>>;
        let host_cx: NavHostCx;
        let ret_node: RNode;
        let merged: bool;
        if let Some(ctx) = merge {
            // MERGED: reuse the enclosing host; our root renders inline in the current page (which
            // is already a NAV_PAGE), and only our pushed destinations become new pages.
            host = ctx.host;
            sizes = ctx.sizes.clone();
            owners = ctx.owners.clone();
            host_cx = ctx;
            let hc = host_cx.clone();
            ret_node = with_nav_host(Some(hc), || root.build(cx));
            merged = true;
        } else {
            // STANDALONE: create the native host + root page (an app-root stack, or a nested stack
            // under a split/desktop host).
            sizes = Rc::default();
            host = cx.native(
                kinds::NAV,
                &NavProps {
                    title: title_s.clone(),
                    split: false, // a stack is a stack (no sidebar)
                },
                Rc::new(NavLayout {
                    sizes: sizes.clone(),
                    split: false,
                }),
                Flex {
                    grow_w: true,
                    grow_h: true,
                    ..Default::default()
                },
                Boundary::Yes,
            );
            owners = Rc::default();
            host_cx = NavHostCx {
                host,
                sizes: sizes.clone(),
                owners: owners.clone(),
                split: false,
            };
            let root_page = nav_page(
                host,
                &NavPageProps {
                    title: title_s,
                    sidebar: false,
                },
                &sizes,
            );
            let hc = host_cx.clone();
            with_nav_host(Some(hc), || {
                let mut pcx = BuildCx::new(root_page);
                let _ = root.build(&mut pcx);
            });
            ret_node = host;
            merged = false;
        }

        let nav_scope = Scope::current();

        // This stack's back owner (one Rc shared by all its pages): bump the native-pop absorb
        // counter when the toolkit already popped, then pop the path.
        let stack_owner: PopOwner = {
            let (p, native_popped) = (path.clone(), native_popped.clone());
            Rc::new(move |already_popped: bool| {
                if already_popped {
                    native_popped.set(native_popped.get() + 1);
                }
                let mut v = p.get_untracked_rw();
                if v.pop().is_some() {
                    p.set_rw(v);
                }
            })
        };

        // Reconcile the native stack to `want`: keep the common prefix, pop the rest, push
        // the new suffix. A pop the native already performed (iOS back) is not re-issued. Pages
        // and owners land on `host` (our own, or the enclosing one when merged).
        let reconcile = {
            let (entries, sizes, dest, native_popped, owners, host_cx, stack_owner) = (
                entries.clone(),
                sizes.clone(),
                dest.clone(),
                native_popped.clone(),
                owners.clone(),
                host_cx.clone(),
                stack_owner.clone(),
            );
            move |want: &Vec<K>| {
                let common = {
                    let ents = entries.borrow();
                    let mut i = 0;
                    while i < ents.len() && i < want.len() && ents[i].key == want[i] {
                        i += 1;
                    }
                    i
                };
                while entries.borrow().len() > common {
                    let e = entries.borrow_mut().pop().unwrap();
                    if native_popped.get() > 0 {
                        native_popped.set(native_popped.get() - 1);
                    } else {
                        with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                    }
                    e.scope.dispose();
                    sizes.borrow_mut().remove(&e.page);
                    with_tree(|t| t.remove_subtree(e.page));
                    owners.borrow_mut().pop();
                }
                for key in want.iter().skip(common) {
                    let title = key.title();
                    let page = nav_page(
                        host,
                        &NavPageProps {
                            title: title.clone(),
                            sidebar: false,
                        },
                        &sizes,
                    );
                    let scope = nav_scope.enter(Scope::child);
                    let content = (dest)(key);
                    let hc = host_cx.clone();
                    scope.enter(|| {
                        with_nav_host(Some(hc), || {
                            let mut c = BuildCx::new(page);
                            let _ = content.build(&mut c);
                        });
                    });
                    with_tree(|t| t.patch(host, Box::new(NavPatch::Pushed { title }), false));
                    owners.borrow_mut().push(stack_owner.clone());
                    entries.borrow_mut().push(StackEntry {
                        key: key.clone(),
                        scope,
                        page,
                    });
                }
                with_tree(|t| {
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        };
        {
            let p = path.clone();
            bind(move || p.get_rw(), move |want: &Vec<K>| reconcile(want));
        }

        // Standalone: own the host's single NavBack dispatcher (→ topmost page's owner) and the
        // deeplink handler. Merged: the enclosing host's creator already owns both.
        if !merged {
            let owners_h = owners.clone();
            cx.on(host, move |ev| match ev {
                Event::NavBack { already_popped } => {
                    let top = owners_h.borrow().last().cloned();
                    if let Some(f) = top {
                        f(*already_popped);
                    }
                }
                Event::Custom {
                    tag: "deeplink",
                    text: route,
                    ..
                } => {
                    let _ = day_core::navigate(route);
                }
                _ => {}
            });
        }

        // Merged: our pages live on the enclosing host, so the enclosing detail's
        // `remove_subtree` won't reach them — pop every remaining page (top-down) off that host
        // when our scope disposes (e.g. the section switches). Guarded for app teardown.
        if merged {
            let (entries_c, sizes_c, owners_c, native_popped_c) = (
                entries.clone(),
                sizes.clone(),
                owners.clone(),
                native_popped.clone(),
            );
            nav_scope.on_cleanup(move || {
                let alive = with_tree(|t| t.node_kind(host).is_some());
                loop {
                    let e = entries_c.borrow_mut().pop();
                    let Some(e) = e else { break };
                    if alive {
                        if native_popped_c.get() > 0 {
                            native_popped_c.set(native_popped_c.get() - 1);
                        } else {
                            with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                        }
                        sizes_c.borrow_mut().remove(&e.page);
                        with_tree(|t| t.remove_subtree(e.page));
                        owners_c.borrow_mut().pop();
                    }
                }
                if alive {
                    with_tree(|t| {
                        t.mark_layout_dirty();
                        t.layout_if_needed();
                    });
                }
            });
        }

        // string-route adapter. A stack is driven by its `path` (app state / buttons), not by
        // magic navigate-strings: a RELATIVE `navigate("<key>")` claims only "" (pop to root),
        // so sibling keys fall through to the enclosing surface — but an ABSOLUTE path's
        // segments (`enter`) push any segment the key type parses: a `String` stack is
        // open-ended, a typed stack validates via `Route::from_key`, and an explicit `a/b/c`
        // path IS the stack's state. `pop` falls through once empty.
        let p_push = path.clone();
        let p_pop = path.clone();
        let p_cur = path.clone();
        let p_enter = path.clone();
        let p_seg = path.clone();
        register_route_surface(
            move |k| {
                if k.is_empty() {
                    let mut v = p_push.get_untracked_rw();
                    if v.is_empty() {
                        return false; // already at root — let the parent handle ""
                    }
                    v.clear();
                    p_push.set_rw(v);
                    true
                } else {
                    false
                }
            },
            move |_| {
                let mut v = p_pop.get_untracked_rw();
                if v.pop().is_some() {
                    p_pop.set_rw(v);
                    true
                } else {
                    false
                }
            },
            move || {
                p_cur
                    .get_untracked_rw()
                    .last()
                    .map(|k| k.key())
                    .unwrap_or_default()
            },
            move |k| {
                let Some(parsed) = K::from_key(k) else {
                    return false; // not one of this stack's routes — leave it queued
                };
                let mut v = p_enter.get_untracked_rw();
                v.push(parsed);
                p_enter.set_rw(v);
                true
            },
            move || p_seg.get_untracked_rw().iter().map(|k| k.key()).collect(),
        );
        ret_node
    }
}

// ===========================================================================
// Cover — a fullscreen modal surface bound to a Signal<Option<Route>> (docs/cover.md).
// ===========================================================================

/// A fullscreen cover: the modal counterpart of [`stack`], bound to a `Signal<Option<R>>`.
/// `Some(r)` presents the built content over the whole window (edge-to-edge, slide-up where
/// the platform animates modals); `None` dismisses it. The SwiftUI analogue is
/// `fullScreenCover(item:)`. Build one with [`cover`].
///
/// The open value is app state, exactly like a stack's path: set it and the cover presents;
/// a native dismissal (Android system back) writes `None` back — unless an
/// [`interactive_dismiss_disabled`](Decorate::interactive_dismiss_disabled) subtree is
/// mounted inside the content, in which case only programmatic writes close it.
/// A cover's per-route surface color (see [`Cover::background`]).
type CoverBackground<R> = Rc<dyn Fn(&R) -> day_spec::Color>;

pub struct Cover<S, R: Route> {
    open: S,
    build: Rc<dyn Fn(&R) -> AnyPiece>,
    background: Option<CoverBackground<R>>,
    _marker: std::marker::PhantomData<R>,
}

/// A fullscreen cover over `open`: `Some(r)` presents `build(&r)`, `None` dismisses
/// (docs/cover.md). Registers a string-route adapter, so `navigate("<key>")` opens it and
/// `nav_back()` closes it, and `current_route()` reports the presented key.
pub fn cover<R: Route, S: SignalRw<Option<R>>>(
    open: S,
    build: impl Fn(&R) -> AnyPiece + 'static,
) -> Cover<S, R> {
    Cover {
        open,
        build: Rc::new(build),
        background: None,
        _marker: std::marker::PhantomData,
    }
}

impl<S: SignalRw<Option<R>>, R: Route> Cover<S, R> {
    /// The surface color painted edge-to-edge behind the content (under the status bar and
    /// home indicator) while `r` is presented. Without it the platform's default surface
    /// color shows in the unsafe areas.
    pub fn background(mut self, f: impl Fn(&R) -> day_spec::Color + 'static) -> Self {
        self.background = Some(Rc::new(f));
        self
    }
}

impl<S: SignalRw<Option<R>>, R: Route> Piece for Cover<S, R> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::props::{CoverPatch, CoverProps};
        let Cover {
            open,
            build,
            background,
            ..
        } = self;

        let size: Rc<RefCell<Option<Size>>> = Rc::default();
        let node = cx.native(
            kinds::COVER,
            &CoverProps::default(),
            Rc::new(day_core::CoverLayout { size: size.clone() }),
            Flex::default(),
            Boundary::Yes,
        );

        // The presented content's scope, and whether a dismiss transition is in flight
        // (content stays mounted until the backend reports "cover-hidden", so the surface
        // isn't blank while it slides out).
        struct Presented<R> {
            key: R,
            scope: Scope,
        }
        let current: Rc<RefCell<Option<Presented<R>>>> = Rc::default();
        let closing: Rc<Cell<bool>> = Rc::default();
        let owner_scope = Scope::current();

        let dispose_content = {
            let current = current.clone();
            move || {
                if let Some(p) = current.borrow_mut().take() {
                    p.scope.dispose();
                }
                while with_tree(|t| t.child_count(node)) > 0 {
                    match with_tree(|t| t.first_child(node)) {
                        Some(c) => with_tree(|t| t.remove_subtree(c)),
                        None => break,
                    }
                }
            }
        };

        // Reconcile the presented surface to the signal.
        let reconcile = {
            let (current, closing, dispose_content) =
                (current.clone(), closing.clone(), dispose_content.clone());
            move |want: &Option<R>| match want {
                Some(r) => {
                    let already =
                        !closing.get() && current.borrow().as_ref().is_some_and(|p| p.key == *r);
                    if already {
                        return;
                    }
                    dispose_content();
                    closing.set(false);
                    let scope = owner_scope.enter(Scope::child);
                    // Run the app's builder INSIDE the presentation scope: side effects it
                    // performs eagerly (state restore, autosave/cleanup registration, signals)
                    // must belong to the presented content's lifetime, not the cover's.
                    scope.enter(|| {
                        let content = (build)(r);
                        let mut c = BuildCx::new(node);
                        let _ = content.build(&mut c);
                    });
                    *current.borrow_mut() = Some(Presented {
                        key: r.clone(),
                        scope,
                    });
                    // Content is mounted, so any `interactive_dismiss_disabled` inside it has
                    // registered — the present patch carries the resolved flag.
                    let bg = background.as_ref().map(|f| f(r));
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(CoverPatch::Present {
                                background: bg,
                                dismiss_disabled: day_core::shield::dismiss_disabled(),
                            }),
                            false,
                        );
                        t.mark_needs_measure(node);
                        t.mark_layout_dirty();
                        t.layout_if_needed();
                    });
                }
                None => {
                    if current.borrow().is_some() && !closing.get() {
                        closing.set(true);
                        with_tree(|t| t.patch(node, Box::new(CoverPatch::Dismiss), false));
                    }
                }
            }
        };
        {
            let o = open.clone();
            bind(move || o.get_rw(), move |want: &Option<R>| reconcile(want));
        }

        // While presented, keep the backend's dismiss-disabled flag in sync with the
        // mounted `interactive_dismiss_disabled` modifiers (the shield's change counter
        // makes this binding re-run as they mount/unmount).
        {
            let current = current.clone();
            bind(
                day_core::shield::dismiss_disabled,
                move |disabled: &bool| {
                    if current.borrow().is_some() {
                        with_tree(|t| {
                            t.patch(
                                node,
                                Box::new(CoverPatch::DismissDisabled(*disabled)),
                                false,
                            )
                        });
                    }
                },
            );
        }

        {
            let (o, size, closing, dispose_content) = (
                open.clone(),
                size.clone(),
                closing.clone(),
                dispose_content.clone(),
            );
            cx.on(node, move |ev| match ev {
                // The backend sized the presented content container (safe-area bounds).
                Event::FrameChanged(sz) => {
                    if *size.borrow() != Some(*sz) {
                        *size.borrow_mut() = Some(*sz);
                        with_tree(|t| {
                            t.mark_needs_measure(node);
                            t.mark_layout_dirty();
                            t.layout_if_needed();
                        });
                    }
                }
                // Native dismissal request (Android system back). Honored unless an
                // `interactive_dismiss_disabled` subtree is mounted.
                Event::NavBack { .. } => {
                    if !day_core::shield::dismiss_disabled() && o.get_untracked_rw().is_some() {
                        o.set_rw(None);
                    }
                }
                // The hide transition finished — now the content can go.
                Event::Custom { tag, text, .. }
                    if (*tag == "cover-hidden" || text.as_str() == "cover-hidden")
                        && closing.get() =>
                {
                    closing.set(false);
                    dispose_content();
                }
                _ => {}
            });
        }

        // String-route adapter (docs/navigation.md): `navigate("<key>")` presents, `nav_back()`
        // dismisses, and the presented key is this surface's `current_route()` contribution.
        let o_push = open.clone();
        let o_pop = open.clone();
        let o_cur = open.clone();
        let o_enter = open.clone();
        let o_seg = open;
        let push = move |k: &str, sig: &S| match R::from_key(k) {
            Some(r) => {
                sig.set_rw(Some(r));
                true
            }
            None => false,
        };
        let push2 = push;
        register_route_surface(
            move |k| push(k, &o_push),
            move |_| {
                if o_pop.get_untracked_rw().is_some() {
                    o_pop.set_rw(None);
                    true
                } else {
                    false
                }
            },
            move || {
                o_cur
                    .get_untracked_rw()
                    .map(|r| r.key())
                    .unwrap_or_default()
            },
            move |k| push2(k, &o_enter),
            move || {
                o_seg
                    .get_untracked_rw()
                    .map(|r| vec![r.key()])
                    .unwrap_or_default()
            },
        );

        node
    }
}

// ===========================================================================
// Forms (docs/forms.md): form / section / labeled — grouped, label-aligned settings UI.
// ===========================================================================

/// Shared label-column state for one [`form`]: every [`labeled`] row inside registers its
/// label's width during measurement and lays its label out in a common, form-wide column —
/// the "aligned labels" look every settings UI converges on. The width is per-layout-pass
/// monotonic: all rows measure before any row places (the enclosing stacks measure all
/// children first), so alignment is consistent within a pass without invalidation dances.
#[derive(Clone)]
struct FormLabelColumn(Rc<Cell<f64>>);

const SECTION_RADIUS: f64 = 10.0;
const LABELED_GAP: f64 = 12.0;

/// A settings-style form: a vertical run of [`section`]s whose [`labeled`] rows share one
/// label column across the WHOLE form.
///
/// ```ignore
/// form((
///     section((
///         labeled(tr("volume"), slider(volume)),
///         labeled(tr("enabled"), toggle(enabled)),
///     ))
///     .title(tr("sound")),
///     section((labeled(tr("name"), text_field(name)),)),
/// ))
/// ```
pub fn form<C: PieceSeq + 'static>(sections: C) -> AnyPiece {
    with_environment(FormLabelColumn(Rc::new(Cell::new(0.0))), move || {
        column(sections).spacing(16.0).align(HAlign::Leading).any()
    })
}

/// One grouped form section (created by [`section`]): an optional header above a rounded card
/// whose background is the platform's own theme-adaptive grouped-content material
/// (`SurfaceRole::SectionCard` — quaternary fill on AppKit, libadwaita `.card`, Qt
/// `palette(alternate-base)`, tertiary system fill on iOS, Material surface-container, the
/// WinUI card brush), so it follows light/dark mode with no app code.
pub struct FormSection<C: PieceSeq> {
    title: Option<TextSource>,
    children: C,
}

/// A grouped card of form rows; `.title(…)` adds the header. Works inside a [`form`] (shared
/// label column) or standalone.
pub fn section<C: PieceSeq + 'static>(children: C) -> FormSection<C> {
    FormSection {
        title: None,
        children,
    }
}

impl<C: PieceSeq + 'static> FormSection<C> {
    /// The section header, shown above the card in the footnote style.
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = Some(t.into_text());
        self
    }
}

impl<C: PieceSeq + 'static> Piece for FormSection<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let children = self.children;
        let card = piece_fn(move |cx: &mut BuildCx| {
            let node = cx.native(
                kinds::CONTAINER,
                &ContainerProps {
                    background: None,
                    corner_radius: SECTION_RADIUS,
                    clips: true,
                    role: Some(day_spec::SurfaceRole::SectionCard),
                },
                Rc::new(SectionCardLayout),
                Flex {
                    grow_w: true,
                    ..Default::default()
                },
                Boundary::No,
            );
            let inner = column(children)
                .spacing(10.0)
                .align(HAlign::Leading)
                .padding(14.0);
            cx.under(node, |cx| {
                let _ = AnyPiece::new(inner).build(cx);
            });
            node
        });
        match self.title {
            Some(t) => {
                let header = Label {
                    text: t,
                    font: Font::Footnote,
                    weight: None,
                    italic: false,
                    color: None,
                };
                column((header, card))
                    .spacing(6.0)
                    .align(HAlign::Leading)
                    .build(cx)
            }
            None => card.build(cx),
        }
    }
}

/// The card fills the width its parent proposes (uniform card widths down a form) and hugs
/// its padded content vertically.
struct SectionCardLayout;

impl day_core::Layout for SectionCardLayout {
    fn measure(&self, cx: &mut dyn day_core::LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let cs = children
            .first()
            .map(|&c| cx.measure_child(c, Proposal::new(p.width, None)))
            .unwrap_or(Size::ZERO);
        Size::new(p.width.unwrap_or(cs.width).max(cs.width), cs.height)
    }
    fn place(&self, cx: &mut dyn day_core::LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            let s = cx.measure_child(c, Proposal::new(Some(bounds.size.width), None));
            cx.place_child(c, Rect::new(0.0, 0.0, bounds.size.width, s.height));
        }
    }
}

/// A form row: `label` sits in the form-wide aligned label column (right-aligned, vertically
/// centered), `control` beside it. Outside a [`form`] the label column is just this row's own
/// label width. A control with `.grow()` stretches to the row's remaining width.
pub fn labeled<M, P: Piece>(text: impl IntoText<M>, control: P) -> AnyPiece {
    let text = text.into_text();
    piece_fn(move |cx: &mut BuildCx| {
        // Read the enclosing form's shared column at BUILD time (environment is scoped).
        let col = environment::<FormLabelColumn>();
        let node = cx.layout_only(
            Rc::new(LabeledLayout { col }),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            Boundary::No,
        );
        cx.under(node, |cx| {
            let row_label = Label {
                text,
                font: Font::Body,
                weight: None,
                italic: false,
                color: None,
            };
            let _ = row_label.build(cx);
            let _ = AnyPiece::new(control).build(cx);
        });
        node
    })
}

struct LabeledLayout {
    col: Option<FormLabelColumn>,
}

impl LabeledLayout {
    /// The label column width in effect: register OUR label width, read back the max.
    fn column_width(&self, label_w: f64) -> f64 {
        match &self.col {
            Some(c) => {
                if label_w > c.0.get() {
                    c.0.set(label_w);
                }
                c.0.get()
            }
            None => label_w,
        }
    }
}

impl day_core::Layout for LabeledLayout {
    fn measure(&self, cx: &mut dyn day_core::LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let (Some(&lbl), Some(&ctl)) = (children.first(), children.get(1)) else {
            return Size::ZERO;
        };
        let ls = cx.measure_child(lbl, Proposal::UNCONSTRAINED);
        let colw = self.column_width(ls.width);
        let avail = p.width.map(|w| (w - colw - LABELED_GAP).max(0.0));
        let cs = cx.measure_child(ctl, Proposal::new(avail, None));
        let natural = colw + LABELED_GAP + cs.width;
        // The row spans the proposed width (labels align form-wide; controls may stretch),
        // and hugs the taller of its two children vertically.
        Size::new(
            p.width.unwrap_or(natural).max(natural),
            ls.height.max(cs.height),
        )
    }
    fn place(&self, cx: &mut dyn day_core::LayoutOps, children: &[RNode], bounds: Rect) {
        let (Some(&lbl), Some(&ctl)) = (children.first(), children.get(1)) else {
            return;
        };
        let ls = cx.measure_child(lbl, Proposal::UNCONSTRAINED);
        let colw = self.column_width(ls.width);
        let avail = (bounds.size.width - colw - LABELED_GAP).max(0.0);
        let cs = cx.measure_child(ctl, Proposal::new(Some(avail), None));
        let h = bounds.size.height;
        cx.place_child(
            lbl,
            Rect::new(
                (colw - ls.width).max(0.0),
                ((h - ls.height) / 2.0).max(0.0),
                ls.width,
                ls.height,
            ),
        );
        // `.grow()` controls fill the remaining width (text fields, sliders); others hug.
        let cw = if cx.flex_of(ctl).grow_w {
            avail
        } else {
            cs.width.min(avail)
        };
        cx.place_child(
            ctl,
            Rect::new(
                colw + LABELED_GAP,
                ((h - cs.height) / 2.0).max(0.0),
                cw,
                cs.height,
            ),
        );
    }
}

// ---------------------------------------------------------------------------
// Imperative presentation (docs/dialogs.md)
// ---------------------------------------------------------------------------

use std::future::{Future, IntoFuture};
use std::pin::Pin;

use day_spec::present::{ButtonRole, PresentButton, PresentResult, PresentSpec};

/// Boxed future the awaitable presenters resolve to — one alloc per dialog, negligible.
type Presenting<T> = Pin<Box<dyn Future<Output = T>>>;

/// A dialog / confirmation / action sheet. Buttons carry a typed payload `T`; `.present()`
/// awaits and returns the chosen button's payload, or `None` on cancel/dismiss.
///
/// ```ignore
/// let choice = Alert::new(tr("delete-title"))
///     .message(tr("delete-body"))
///     .destructive(tr("delete"), Choice::Delete)
///     .cancel(tr("cancel"))
///     .present().await;   // Option<Choice>
/// ```
pub struct Alert<T> {
    title: String,
    message: Option<String>,
    sheet: bool,
    /// (label, role, payload) in presentation order; cancel buttons carry `None`.
    buttons: Vec<(String, ButtonRole, Option<T>)>,
}

pub fn alert<M>(title: impl IntoText<M>) -> Alert<()> {
    Alert {
        title: title.into_text().initial(),
        message: None,
        sheet: false,
        buttons: Vec::new(),
    }
}

impl<T> Alert<T> {
    pub fn new<M>(title: impl IntoText<M>) -> Alert<T> {
        Alert {
            title: title.into_text().initial(),
            message: None,
            sheet: false,
            buttons: Vec::new(),
        }
    }
    pub fn message<M>(mut self, m: impl IntoText<M>) -> Self {
        self.message = Some(m.into_text().initial());
        self
    }
    /// Present as a bottom action sheet on mobile (desktop falls back to an alert).
    pub fn sheet(mut self) -> Self {
        self.sheet = true;
        self
    }
    /// A normal choice carrying `value`.
    pub fn button<M>(mut self, label: impl IntoText<M>, value: T) -> Self {
        self.buttons.push((
            label.into_text().initial(),
            ButtonRole::Default,
            Some(value),
        ));
        self
    }
    /// A destructive choice (red on Apple) carrying `value`.
    pub fn destructive<M>(mut self, label: impl IntoText<M>, value: T) -> Self {
        self.buttons.push((
            label.into_text().initial(),
            ButtonRole::Destructive,
            Some(value),
        ));
        self
    }
    /// The cancel affordance; choosing it (or dismissing) resolves to `None`.
    pub fn cancel<M>(mut self, label: impl IntoText<M>) -> Self {
        self.buttons
            .push((label.into_text().initial(), ButtonRole::Cancel, None));
        self
    }

    /// Present natively and await the chosen payload (`None` = cancel / dismissed).
    pub async fn present(self) -> Option<T> {
        let spec = PresentSpec::Dialog {
            title: self.title,
            message: self.message,
            buttons: self
                .buttons
                .iter()
                .map(|(label, role, _)| PresentButton {
                    label: label.clone(),
                    role: *role,
                })
                .collect(),
            sheet: self.sheet,
        };
        let mut payloads: Vec<Option<T>> = self.buttons.into_iter().map(|(_, _, v)| v).collect();
        match day_core::present(spec).await {
            PresentResult::Button(i) => {
                let i = i as usize;
                if i < payloads.len() {
                    payloads[i].take()
                } else {
                    None
                }
            }
            _ => None,
        }
    }
}

impl<T: 'static> IntoFuture for Alert<T> {
    type Output = Option<T>;
    type IntoFuture = Presenting<Option<T>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// A yes/no confirmation. Resolves to `true` only if the confirm button is chosen.
pub struct Confirm {
    title: String,
    message: Option<String>,
    confirm: String,
    cancel: String,
    destructive: bool,
}

pub fn confirm<M>(title: impl IntoText<M>) -> Confirm {
    Confirm {
        title: title.into_text().initial(),
        message: None,
        // Localized from the core catalog (docs/dialogs.md); `.confirm_label`/`.cancel_label`
        // override. Resolved in the current locale at build time.
        confirm: day_l10n::t("day-ok"),
        cancel: day_l10n::t("day-cancel"),
        destructive: false,
    }
}

impl Confirm {
    pub fn message<M>(mut self, m: impl IntoText<M>) -> Self {
        self.message = Some(m.into_text().initial());
        self
    }
    pub fn confirm_label<M>(mut self, label: impl IntoText<M>) -> Self {
        self.confirm = label.into_text().initial();
        self
    }
    pub fn cancel_label<M>(mut self, label: impl IntoText<M>) -> Self {
        self.cancel = label.into_text().initial();
        self
    }
    /// Style the confirm button as destructive.
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
    pub async fn present(self) -> bool {
        let confirm_role = if self.destructive {
            ButtonRole::Destructive
        } else {
            ButtonRole::Default
        };
        let spec = PresentSpec::Dialog {
            title: self.title,
            message: self.message,
            buttons: vec![
                PresentButton {
                    label: self.cancel,
                    role: ButtonRole::Cancel,
                },
                PresentButton {
                    label: self.confirm,
                    role: confirm_role,
                },
            ],
            sheet: false,
        };
        // index 1 = the confirm button.
        matches!(day_core::present(spec).await, PresentResult::Button(1))
    }
}

impl IntoFuture for Confirm {
    type Output = bool;
    type IntoFuture = Presenting<bool>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// A single-line text prompt. Resolves to `Some(text)` on OK, `None` on cancel/dismiss.
pub struct Prompt {
    title: String,
    message: Option<String>,
    placeholder: String,
    initial: String,
    ok: String,
    cancel: String,
}

pub fn prompt<M>(title: impl IntoText<M>) -> Prompt {
    Prompt {
        title: title.into_text().initial(),
        message: None,
        placeholder: String::new(),
        initial: String::new(),
        // Localized from the core catalog (docs/dialogs.md); `.ok_label`/`.cancel_label` override.
        ok: day_l10n::t("day-ok"),
        cancel: day_l10n::t("day-cancel"),
    }
}

impl Prompt {
    pub fn message<M>(mut self, m: impl IntoText<M>) -> Self {
        self.message = Some(m.into_text().initial());
        self
    }
    pub fn placeholder<M>(mut self, p: impl IntoText<M>) -> Self {
        self.placeholder = p.into_text().initial();
        self
    }
    pub fn initial<M>(mut self, v: impl IntoText<M>) -> Self {
        self.initial = v.into_text().initial();
        self
    }
    pub async fn present(self) -> Option<String> {
        let spec = PresentSpec::Prompt {
            title: self.title,
            message: self.message,
            placeholder: self.placeholder,
            initial: self.initial,
            ok: self.ok,
            cancel: self.cancel,
        };
        match day_core::present(spec).await {
            PresentResult::Text(t) => Some(t),
            _ => None,
        }
    }
}

impl IntoFuture for Prompt {
    type Output = Option<String>;
    type IntoFuture = Presenting<Option<String>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

// ---------------------------------------------------------------------------
// File open / save (docs/files.md)
// ---------------------------------------------------------------------------

use day_spec::present::FileFilter;

/// A cross-platform handle to a file the user chose in a native open/save picker.
///
/// Internally a single **locator string**. On desktop and iOS it is an absolute filesystem
/// path; on Android it may be a `content://` URI, since the Storage Access Framework does not
/// expose real filesystem paths. That is why Day uses a bespoke type rather than
/// [`std::path::PathBuf`] (which cannot represent a `content://` URI) or a bare `String` (no
/// type-safety / helpers): a `FileUrl` is the lossless union with ergonomic accessors.
///
/// Files returned from [`open_file`] are always readable via [`FileUrl::read_to_string`] /
/// [`FileUrl::read`] — backends copy a picked file into app storage first where the platform
/// requires it, so the local path "just works" everywhere.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FileUrl(String);

impl FileUrl {
    /// Wrap a locator string (a filesystem path or a URI). Usually produced by the pickers.
    pub fn new(locator: impl Into<String>) -> Self {
        FileUrl(locator.into())
    }
    /// The raw locator: a filesystem path, or a `content://`-style URI on Android.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// The locator as a filesystem path — `Some` for local paths (and `file://` URLs), `None`
    /// for opaque URIs such as Android's `content://`.
    pub fn local_path(&self) -> Option<std::path::PathBuf> {
        if self.0.contains("://") && !self.0.starts_with("file://") {
            return None;
        }
        let p = self.0.strip_prefix("file://").unwrap_or(&self.0);
        Some(std::path::PathBuf::from(p))
    }
    /// The last path component, for display (e.g. `notes.txt`). Best-effort for opaque URIs.
    pub fn file_name(&self) -> Option<String> {
        let s = self.0.trim_end_matches(['/', '\\']);
        let tail = s.rsplit(['/', '\\']).next().unwrap_or(s);
        if tail.is_empty() {
            None
        } else {
            Some(tail.to_string())
        }
    }
    /// Read the file's bytes. Local paths only; opaque URIs return an `Unsupported` error.
    pub fn read(&self) -> std::io::Result<Vec<u8>> {
        match self.local_path() {
            Some(p) => std::fs::read(p),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "content:// URIs are not directly readable",
            )),
        }
    }
    /// Read the file as UTF-8 text.
    pub fn read_to_string(&self) -> std::io::Result<String> {
        match self.local_path() {
            Some(p) => std::fs::read_to_string(p),
            None => Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "content:// URIs are not directly readable",
            )),
        }
    }
}

impl std::fmt::Display for FileUrl {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn parse_filter(name: impl Into<String>, extensions: &[&str]) -> FileFilter {
    FileFilter {
        name: name.into(),
        extensions: extensions
            .iter()
            .map(|e| e.trim_start_matches('.').to_string())
            .collect(),
    }
}

/// A native "open file" picker. `.await` (or `.present().await`) resolves to the chosen
/// [`FileUrl`], or `None` if the user cancels.
///
/// ```ignore
/// let file = open_file().filter("Text", &["txt", "md"]).await;
/// if let Some(f) = file { let body = f.read_to_string()?; }
/// ```
pub struct OpenFile {
    title: String,
    filters: Vec<FileFilter>,
}

/// Start a native open-file picker (docs/files.md).
pub fn open_file() -> OpenFile {
    OpenFile {
        title: day_l10n::t("day-open"),
        filters: Vec::new(),
    }
}

impl OpenFile {
    /// Override the picker's title (localizable via `tr()`).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text().initial();
        self
    }
    /// Add a named file-type filter, e.g. `.filter("Text", &["txt", "md"])`. No filter = all files.
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push(parse_filter(name, extensions));
        self
    }
    pub async fn present(self) -> Option<FileUrl> {
        let spec = PresentSpec::OpenFile {
            title: self.title,
            filters: self.filters,
        };
        match day_core::present(spec).await {
            PresentResult::Files(mut v) if !v.is_empty() => Some(FileUrl(v.remove(0))),
            _ => None,
        }
    }
}

impl IntoFuture for OpenFile {
    type Output = Option<FileUrl>;
    type IntoFuture = Presenting<Option<FileUrl>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// A native "save file" picker carrying the bytes to write. `.await` resolves to the chosen
/// destination [`FileUrl`], or `None` on cancel.
///
/// ```ignore
/// let saved = save_file(text.into_bytes())
///     .suggested_name("notes.txt")
///     .filter("Text", &["txt"])
///     .await;
/// ```
pub struct SaveFile {
    title: String,
    suggested_name: String,
    filters: Vec<FileFilter>,
    data: Vec<u8>,
}

/// Start a native save-file picker for `data` (docs/files.md).
pub fn save_file(data: impl Into<Vec<u8>>) -> SaveFile {
    SaveFile {
        title: day_l10n::t("day-save"),
        suggested_name: "untitled.txt".to_string(),
        filters: Vec::new(),
        data: data.into(),
    }
}

impl SaveFile {
    /// Override the picker's title (localizable via `tr()`).
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text().initial();
        self
    }
    /// The default file name shown in the picker.
    pub fn suggested_name(mut self, name: impl Into<String>) -> Self {
        self.suggested_name = name.into();
        self
    }
    /// Add a named file-type filter, e.g. `.filter("Text", &["txt"])`.
    pub fn filter(mut self, name: impl Into<String>, extensions: &[&str]) -> Self {
        self.filters.push(parse_filter(name, extensions));
        self
    }
    pub async fn present(self) -> Option<FileUrl> {
        // Stage the bytes in an app-writable temp file the backend hands to the native picker.
        let mut src = day_core::app_temp_dir();
        src.push(format!(
            "day-save-{}-{}",
            std::process::id(),
            sanitize_name(&self.suggested_name)
        ));
        if std::fs::write(&src, &self.data).is_err() {
            return None;
        }
        let spec = PresentSpec::SaveFile {
            title: self.title,
            suggested_name: self.suggested_name,
            src_path: src.to_string_lossy().into_owned(),
            filters: self.filters,
        };
        let dest = match day_core::present(spec).await {
            PresentResult::Files(mut v) if !v.is_empty() => FileUrl(v.remove(0)),
            _ => {
                let _ = std::fs::remove_file(&src);
                return None;
            }
        };
        // Best-effort deliver the bytes to a local destination (desktop, and headless dayscript).
        // On Android the destination is a `content://` URI the backend already wrote (no
        // `local_path`, so the copy is skipped); iOS delivers via the document exporter.
        if let Some(p) = dest.local_path()
            && p != src
        {
            let _ = std::fs::copy(&src, &p);
        }
        let _ = std::fs::remove_file(&src);
        Some(dest)
    }
}

impl IntoFuture for SaveFile {
    type Output = Option<FileUrl>;
    type IntoFuture = Presenting<Option<FileUrl>>;
    fn into_future(self) -> Self::IntoFuture {
        Box::pin(self.present())
    }
}

/// Keep a suggested file name safe as a temp-file component (path-separator / control-char free).
fn sanitize_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if s.is_empty() {
        "untitled".to_string()
    } else {
        s
    }
}
