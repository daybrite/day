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
    A11yProps, Color, DrawOp, Event, Font, Insets, Point, Rect, Role, Shape, Size, kinds,
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

pub struct Button {
    title: TextSource,
    action: Option<Rc<dyn Fn()>>,
}

pub fn button<M>(title: impl IntoText<M>) -> Button {
    Button {
        title: title.into_text(),
        action: None,
    }
}

impl Button {
    pub fn action(mut self, f: impl Fn() + 'static) -> Self {
        self.action = Some(Rc::new(f));
        self
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
}

pub fn text_field<S: SignalRw<String>>(value: S) -> TextField<S> {
    TextField {
        value,
        placeholder: None,
    }
}

impl<S: SignalRw<String>> TextField<S> {
    pub fn placeholder<M>(mut self, t: impl IntoText<M>) -> Self {
        self.placeholder = Some(t.into_text());
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
        cx.on(node, move |ev| match ev {
            Event::TextChanged(t) => {
                *guard.borrow_mut() = Some(t.clone());
                v.set_rw(t.clone());
            }
            Event::Submitted => {}
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

pub struct Scroll<P: Piece> {
    child: P,
    axis: Axis,
}

pub fn scroll<P: Piece>(child: P) -> Scroll<P> {
    Scroll {
        child,
        axis: Axis::Vertical,
    }
}

impl<P: Piece> Piece for Scroll<P> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::SCROLL,
            &ContainerProps::default(),
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
    /// Tracked whole-item read.
    pub fn get(self) -> T {
        self.sig.get()
    }
    pub fn with<R>(self, f: impl FnOnce(&T) -> R) -> R {
        self.sig.with(f)
    }
    /// Tracked field projection (equality-gating happens in the binding layer).
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
            let (refresh, items) = (refresh.clone(), items.clone());
            watch(move || items(), move |new: &Vec<T>, _| refresh(new));
        }
        node
    }
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

pub mod prelude {
    pub use crate::TextStyle;
    pub use crate::{
        A11yBuilder, Alert, Confirm, Corner, Decorate, Drag, Draw, HAlign, IntoFraction,
        IntoReactive, IntoText, ItemSlot, List, Prompt, Reactive, Selector, SelectorStyle,
        ShapeKind, ShapePiece, SignalRw, Stack, VAlign, alert, arc, button, canvas, capsule,
        circle, column, confirm, divider, each, ellipse, image, label, list, nav_back, nav_link,
        navigate, progress, prompt, rectangle, rounded_rectangle, row, scroll, selector, shape,
        slider, spacer, spinner, stack, text_field, toggle, when,
    };
    pub use day_core::{AnyPiece, BuildCx, Piece, PieceSeq, PieceVec, piece_fn};
    pub use day_geometry::{Affine, Color, Insets, Point, Rect, Size};
    pub use day_reactive::{
        Effect, Memo, Scope, Setter, Signal, Trigger, batch, bind, untrack, watch,
    };
    pub use day_spec::props::RowHeight;
    pub use day_spec::{DragPhase, GestureKind};
    pub use day_spec::{DrawOp, Shape, TextAnchor};
    pub use day_spec::{Font, FontSpec, FontWeight, Role};
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
    pub fn fill(&mut self, shape: Shape, color: Color) {
        self.ops.push(DrawOp::Fill(shape, color));
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
pub use day_spec::{DragPhase, GestureKind};

/// A shape's geometry, resolved against the rect layout assigns it (frame-relative, SwiftUI-style).
#[derive(Clone, Copy, Debug, PartialEq)]
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
        match self.geometry(rect) {
            Shape::Ellipse(r) => in_ellipse(r, p),
            Shape::Rect(r) | Shape::RoundedRect(r, _) => in_rect(r, p),
            _ => in_rect(rect, p), // arc / line / polygon: bounding-box fallback
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

/// A shape piece — one data-oriented piece parameterised by `ShapeKind`, rendered atop the canvas.
pub struct ShapePiece {
    kind: Reactive<ShapeKind>,
    fill: Option<Reactive<Color>>,
    stroke: Option<(Reactive<Color>, Reactive<f64>)>,
    inset: Reactive<f64>,
    rotate: Reactive<f64>,
    scale: Reactive<f64>,
    offset: (Reactive<f64>, Reactive<f64>),
    on_tap: Option<Rc<dyn Fn()>>,
    on_drag: Option<Rc<dyn Fn(Drag)>>,
}

/// The unified constructor: `shape(ShapeKind::RoundedRectangle { corner: 12.0.into() })`.
pub fn shape<M>(kind: impl IntoReactive<ShapeKind, M>) -> ShapePiece {
    ShapePiece {
        kind: kind.into_reactive(),
        fill: None,
        stroke: None,
        inset: Reactive::Const(0.0),
        rotate: Reactive::Const(0.0),
        scale: Reactive::Const(1.0),
        offset: (Reactive::Const(0.0), Reactive::Const(0.0)),
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

impl ShapePiece {
    pub fn fill<M>(mut self, p: impl IntoReactive<Color, M>) -> Self {
        self.fill = Some(p.into_reactive());
        self
    }
    pub fn stroke<M1, M2>(
        mut self,
        color: impl IntoReactive<Color, M1>,
        width: impl IntoReactive<f64, M2>,
    ) -> Self {
        self.stroke = Some((color.into_reactive(), width.into_reactive()));
        self
    }
    /// Uniform inset applied before resolving geometry (keeps strokes inside the frame).
    pub fn inset<M>(mut self, v: impl IntoReactive<f64, M>) -> Self {
        self.inset = v.into_reactive();
        self
    }
    /// Rotate the drawn shape about its centre, in degrees.
    pub fn rotate<M>(mut self, deg: impl IntoReactive<f64, M>) -> Self {
        self.rotate = deg.into_reactive();
        self
    }
    /// Scale the drawn shape about its centre (uniform).
    pub fn scale<M>(mut self, s: impl IntoReactive<f64, M>) -> Self {
        self.scale = s.into_reactive();
        self
    }
    /// Translate the drawn shape within its frame.
    pub fn offset<M1, M2>(
        mut self,
        x: impl IntoReactive<f64, M1>,
        y: impl IntoReactive<f64, M2>,
    ) -> Self {
        self.offset = (x.into_reactive(), y.into_reactive());
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

impl Piece for ShapePiece {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let ShapePiece {
            kind,
            fill,
            stroke,
            inset,
            rotate,
            scale,
            offset,
            on_tap,
            on_drag,
        } = self;

        // A shape greedily fills its proposed size (SwiftUI semantics).
        let grow = Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        };
        let (dk, di, dr, ds, dox, doy) = (
            kind.clone(),
            inset.clone(),
            rotate.clone(),
            scale.clone(),
            offset.0.clone(),
            offset.1.clone(),
        );
        let node = canvas_leaf(cx, grow, move |d, size| {
            let rect = Rect::from_size(size).inset(di.get());
            if rect.size.width <= 0.0 || rect.size.height <= 0.0 {
                return;
            }
            let geom = dk.get().geometry(rect);
            let m = shape_transform(rect, dr.get(), ds.get(), dox.get(), doy.get());
            let transformed = !m.is_identity();
            if transformed {
                d.save();
                d.concat(m);
            }
            if let Some(fill) = &fill {
                d.fill(geom.clone(), fill.get());
            }
            if let Some((c, w)) = &stroke {
                d.stroke(geom, c.get(), w.get());
            }
            if transformed {
                d.restore();
            }
        });

        // Path-precise tap: inverse-transform the point, then test against the resolved geometry.
        if let Some(on_tap) = on_tap {
            with_tree(|t| t.enable_gesture(node, GestureKind::Tap));
            let (kind, inset, rotate, scale, offset) = (
                kind,
                inset,
                rotate,
                scale,
                (offset.0.clone(), offset.1.clone()),
            );
            cx.on(node, move |ev| {
                if let Event::Tap(p) = ev
                    && let Some(f) = with_tree(|t| t.node_frame(node))
                {
                    let rect = Rect::from_size(f.size).inset(inset.get_untracked());
                    let m = shape_transform(
                        rect,
                        rotate.get_untracked(),
                        scale.get_untracked(),
                        offset.0.get_untracked(),
                        offset.1.get_untracked(),
                    );
                    let local = m.invert_apply(*p).unwrap_or(*p);
                    if kind.get_untracked().contains(rect, local) {
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

pub fn image(asset_name: &str) -> AnyPiece {
    let name = asset_name.to_owned();
    piece_fn(move |cx| {
        cx.leaf(
            kinds::IMAGE,
            &ImageProps {
                source: name,
                decorative: false,
            },
            Flex::default(),
        )
    })
}

// ---------------------------------------------------------------------------
// Navigation & tabs (docs/navigation.md, docs/tabs.md) — selector + stack, each a
// projection of an app-owned Signal.
// ---------------------------------------------------------------------------

/// Navigate to a registered route ("" = the surface's root). Reaches the innermost route
/// surface first (docs/navigation.md); for a `selector` this sets the active key, for a
/// `stack` it pushes. False = no surface / unknown key.
pub fn navigate(path: &str) -> bool {
    day_core::navigate(path)
}

/// Pop one navigation level. False = nothing to pop.
pub fn nav_back() -> bool {
    day_core::nav_back()
}

/// A tappable link that navigates to `path` when pressed.
pub fn nav_link<M>(label: impl IntoText<M>, path: &str) -> Button {
    let path = path.to_string();
    button(label).action(move || {
        let _ = day_core::navigate(&path);
    })
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
fn register_route_surface(
    push: impl Fn(&str) -> bool + 'static,
    pop: impl Fn(bool) -> bool + 'static,
    current: impl Fn() -> String + 'static,
) {
    let token = day_core::register_nav(day_core::NavController {
        push: Box::new(push),
        pop: Box::new(pop),
        current: Box::new(current),
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

struct SelItem {
    key: String,
    title: TextSource,
    build: Box<dyn Fn() -> AnyPiece>,
}

/// A sidebar item resolved for the detail switcher: (key, resolved title, lazy builder).
type ResolvedItems = Rc<Vec<(String, String, Box<dyn Fn() -> AnyPiece>)>>;

/// A one-of-N selector whose active key is an app-owned `Signal<String>` (two-way, exactly
/// like `Picker`/`Toggle`). Deep links and dayscript address items by key (docs/navigation.md).
///
/// ```ignore
/// let section = Signal::new("home".to_string());
/// selector(section).style(SelectorStyle::Sidebar)
///     .item("home", tr("home"), home_page)
///     .item("settings", tr("settings"), settings_page)
/// ```
pub struct Selector<S: SignalRw<String>> {
    selection: S,
    style: SelectorStyle,
    title: TextSource,
    header: Option<Box<dyn FnOnce() -> AnyPiece>>,
    items: Vec<SelItem>,
}

pub fn selector<S: SignalRw<String>>(selection: S) -> Selector<S> {
    Selector {
        selection,
        style: SelectorStyle::Sidebar,
        title: TextSource::Static(String::new()),
        header: None,
        items: Vec::new(),
    }
}

impl<S: SignalRw<String>> Selector<S> {
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
    /// its label; `build` runs when the item is first shown.
    pub fn item<M, P: Piece>(
        mut self,
        key: &str,
        title: impl IntoText<M>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        self.items.push(SelItem {
            key: key.to_string(),
            title: title.into_text(),
            build: Box::new(move || AnyPiece::new(build())),
        });
        self
    }
}

impl<S: SignalRw<String>> Piece for Selector<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        match self.style {
            SelectorStyle::Tabs => build_tabs(self, cx),
            SelectorStyle::Sidebar => build_sidebar(self, cx),
        }
    }
}

fn build_tabs<S: SignalRw<String>>(sel: Selector<S>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{TabsPageProps, TabsPatch, TabsProps};
    let selection = sel.selection;
    let metas: Vec<(String, String)> = sel
        .items
        .iter()
        .map(|it| (it.key.clone(), it.title.initial()))
        .collect();
    let titles: Vec<String> = metas.iter().map(|(_, t)| t.clone()).collect();
    let keys: Rc<Vec<String>> = Rc::new(metas.iter().map(|(k, _)| k.clone()).collect());
    let initial = selection.get_untracked_rw();
    let initial_idx = keys.iter().position(|k| *k == initial).unwrap_or(0);

    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let host = cx.native(
        kinds::TABS,
        &TabsProps {
            titles,
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
            },
            &sizes,
        );
        let content = (it.build)();
        let mut pcx = BuildCx::new(page);
        let _ = content.build(&mut pcx);
    }

    // Two-way: signal → native selection (skip the echo of a native tap).
    let echo: Rc<Cell<Option<usize>>> = Rc::new(Cell::new(None));
    {
        let (keys, echo, s) = (keys.clone(), echo.clone(), selection.clone());
        bind_seeded(
            initial_idx,
            move || keys.iter().position(|k| *k == s.get_rw()).unwrap_or(0),
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
        let (keys, echo, s) = (keys.clone(), echo.clone(), selection.clone());
        cx.on(host, move |ev| match ev {
            Event::SelectionChanged(i) if *i >= 0 => {
                let idx = *i as usize;
                if let Some(k) = keys.get(idx) {
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
    // string-route adapter
    let (ks_push, s_push) = (keys.clone(), selection.clone());
    let s_cur = selection.clone();
    register_route_surface(
        move |k| {
            if ks_push.iter().any(|x| x == k) {
                s_push.set_rw(k.to_string());
                true
            } else {
                false
            }
        },
        |_| false,
        move || s_cur.get_untracked_rw(),
    );
    host
}

fn build_sidebar<S: SignalRw<String>>(sel: Selector<S>, cx: &mut BuildCx) -> RNode {
    use day_spec::props::{NavMenuPatch, NavMenuProps, NavPageProps, NavPatch, NavProps};
    let split = with_tree(|t| t.capability(day_spec::Cap::NavSplit)) == day_spec::Support::Native;
    let selection = sel.selection;
    let title_s = sel.title.initial();
    let metas: Vec<(String, String)> = sel
        .items
        .iter()
        .map(|it| (it.key.clone(), it.title.initial()))
        .collect();
    let keys: Rc<Vec<String>> = Rc::new(metas.iter().map(|(k, _)| k.clone()).collect());
    let titles: Vec<String> = metas.iter().map(|(_, t)| t.clone()).collect();
    let builders: ResolvedItems = Rc::new(
        sel.items
            .into_iter()
            .enumerate()
            .map(|(i, it)| (it.key, metas[i].1.clone(), it.build))
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
        let (mh, ks, s, titles2) = (
            menu_holder.clone(),
            keys.clone(),
            selection.clone(),
            titles.clone(),
        );
        let menu_piece = piece_fn(move |mcx| {
            let node = mcx.native(
                kinds::NAV_MENU,
                &NavMenuProps {
                    items: titles2,
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
        let mut pcx = BuildCx::new(root_page);
        let _ = content.build(&mut pcx);
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
        let (builders, current, sizes, keys, sync_menu) = (
            builders.clone(),
            current.clone(),
            sizes.clone(),
            keys.clone(),
            sync_menu.clone(),
        );
        move |key: &str| {
            if current.borrow().as_ref().map(|(k, _, _)| k.as_str()) == Some(key) {
                return;
            }
            if let Some((_, scope, page)) = current.borrow_mut().take() {
                with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                scope.dispose();
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
            let scope = nav_scope.enter(Scope::child);
            let content = build();
            scope.enter(|| {
                let mut c = BuildCx::new(page);
                let _ = content.build(&mut c);
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
        && selection.get_untracked_rw().is_empty()
        && let Some(k) = keys.first()
    {
        selection.set_rw(k.clone());
    }
    {
        let s = selection.clone();
        bind(move || s.get_rw(), move |key: &String| show(key));
    }

    // Native back (mobile up-arrow / system back) returns to the list; warm deep links.
    {
        let s = selection.clone();
        cx.on(host, move |ev| match ev {
            Event::NavBack { .. } => s.set_rw(String::new()),
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

    // string-route adapter over `selection`
    let (ks_push, s_push) = (keys.clone(), selection.clone());
    let s_pop = selection.clone();
    let s_cur = selection.clone();
    register_route_surface(
        move |k| {
            if k.is_empty() {
                s_push.set_rw(String::new());
                true
            } else if ks_push.iter().any(|x| x == k) {
                s_push.set_rw(k.to_string());
                true
            } else {
                false
            }
        },
        move |_| {
            if s_pop.get_untracked_rw().is_empty() {
                false
            } else {
                s_pop.set_rw(String::new());
                true
            }
        },
        move || s_cur.get_untracked_rw(),
    );
    host
}

// ===========================================================================
// Stack — a genuine push/pop navigation stack bound to a Signal<Vec<String>>.
// The native UINavigationController / AdwNavigationView / back-stack is reconciled
// to the path; the back button writes the pop back into the path.
// ===========================================================================

struct StackEntry {
    key: String,
    scope: Scope,
    page: RNode,
}

/// A push/pop navigation stack whose contents are an app-owned `Signal<Vec<String>>` (the
/// path above the root). Day reconciles the native stack to the path; the native back button
/// writes the pop back into it (docs/navigation.md).
///
/// ```ignore
/// let path = Signal::new(Vec::<String>::new());
/// stack(path.clone(), home_view).destination(|key| detail_view(key))
/// // push:  path.update(|p| p.push("item-42".into()));
/// ```
pub struct Stack<S: SignalRw<Vec<String>>> {
    path: S,
    title: TextSource,
    root: AnyPiece,
    destination: Rc<dyn Fn(&str) -> AnyPiece>,
}

pub fn stack<S: SignalRw<Vec<String>>>(path: S, root: impl Piece) -> Stack<S> {
    Stack {
        path,
        title: TextSource::Static(String::new()),
        root: AnyPiece::new(root),
        destination: Rc::new(|_| {
            piece_fn(|cx| cx.layout_only(Rc::new(PassThrough), Flex::default(), Boundary::No))
        }),
    }
}

impl<S: SignalRw<Vec<String>>> Stack<S> {
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = t.into_text();
        self
    }
    /// Build the view for a pushed `key`.
    pub fn destination<P: Piece>(mut self, build: impl Fn(&str) -> P + 'static) -> Self {
        self.destination = Rc::new(move |k| AnyPiece::new(build(k)));
        self
    }
}

impl<S: SignalRw<Vec<String>>> Piece for Stack<S> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::props::{NavPageProps, NavPatch, NavProps};
        let path = self.path;
        let title_s = self.title.initial();
        let dest = self.destination;

        let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
        let host = cx.native(
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
        let root_page = nav_page(
            host,
            &NavPageProps {
                title: title_s,
                sidebar: false,
            },
            &sizes,
        );
        {
            let mut pcx = BuildCx::new(root_page);
            let _ = self.root.build(&mut pcx);
        }

        let entries: Rc<RefCell<Vec<StackEntry>>> = Rc::default();
        let native_popped: Rc<Cell<usize>> = Rc::new(Cell::new(0));
        let nav_scope = Scope::current();

        // Reconcile the native stack to `want`: keep the common prefix, pop the rest, push
        // the new suffix. A pop the native already performed (iOS back) is not re-issued.
        let reconcile = {
            let (entries, sizes, dest, native_popped) = (
                entries.clone(),
                sizes.clone(),
                dest.clone(),
                native_popped.clone(),
            );
            move |want: &Vec<String>| {
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
                }
                for key in want.iter().skip(common) {
                    let page = nav_page(
                        host,
                        &NavPageProps {
                            title: key.clone(),
                            sidebar: false,
                        },
                        &sizes,
                    );
                    let scope = nav_scope.enter(Scope::child);
                    let content = (dest)(key);
                    scope.enter(|| {
                        let mut c = BuildCx::new(page);
                        let _ = content.build(&mut c);
                    });
                    with_tree(|t| {
                        t.patch(
                            host,
                            Box::new(NavPatch::Pushed { title: key.clone() }),
                            false,
                        )
                    });
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
            bind(
                move || p.get_rw(),
                move |want: &Vec<String>| reconcile(want),
            );
        }

        // Native back → pop the path (origin-tagged so reconcile doesn't re-issue it).
        {
            let (p, native_popped) = (path.clone(), native_popped.clone());
            cx.on(host, move |ev| match ev {
                Event::NavBack { already_popped } => {
                    if *already_popped {
                        native_popped.set(native_popped.get() + 1);
                    }
                    let mut v = p.get_untracked_rw();
                    if v.pop().is_some() {
                        p.set_rw(v);
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

        // string-route adapter. A stack is driven by its `path` (app state / buttons), not by
        // magic navigate-strings: only "" (pop to root) is claimed, so `navigate("<sibling>")`
        // falls through to the enclosing surface. `pop` falls through once the stack is empty.
        let p_push = path.clone();
        let p_pop = path.clone();
        let p_cur = path.clone();
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
            move || p_cur.get_untracked_rw().last().cloned().unwrap_or_default(),
        );
        host
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
        confirm: "OK".into(),
        cancel: "Cancel".into(),
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
        ok: "OK".into(),
        cancel: "Cancel".into(),
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
