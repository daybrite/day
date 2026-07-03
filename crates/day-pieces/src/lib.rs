//! day-pieces — the built-in piece library (DESIGN.md §5.3).
//!
//! Every constructor is a plain function returning a piece value; builder methods configure;
//! `build` runs once. Dynamic attributes become seeded bindings writing sparse typed patches
//! through the thread-local tree.

use std::cell::RefCell;
use std::collections::HashSet;
use std::hash::Hash;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal, bind_seeded, watch};
use day_spec::props::*;
use day_spec::{A11yProps, Color, DrawOp, Event, Font, Insets, Point, Role, Shape, Size, kinds};

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
    color: Option<day_spec::Color>,
}

pub fn label<M>(text: impl IntoText<M>) -> Label {
    Label {
        text: text.into_text(),
        font: Font::Body,
        color: None,
    }
}

impl Label {
    pub fn font(mut self, f: Font) -> Self {
        self.font = f;
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
                font: self.font,
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
        cx.on(node, move |ev| {
            if let Event::ValueChanged(val) = ev {
                v.set_rw(*val);
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
    pub fn role(mut self, r: Role) -> Self {
        self.0.role = r;
        self
    }
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
        A11yBuilder, Alert, Confirm, Decorate, Draw, HAlign, IntoFraction, IntoText, ItemSlot, Nav,
        Prompt, SignalRw, VAlign, alert, button, canvas, column, confirm, divider, each, image,
        label, nav, nav_back, nav_link, nav_menu, navigate, progress, prompt, row, scroll, slider,
        spacer, spinner, text_field, toggle, when,
    };
    pub use day_core::{AnyPiece, BuildCx, Piece, PieceSeq, PieceVec, piece_fn};
    pub use day_geometry::{Color, Insets, Point, Rect, Size};
    pub use day_reactive::{
        Effect, Memo, Scope, Setter, Signal, Trigger, batch, bind, untrack, watch,
    };
    pub use day_spec::{DrawOp, Shape, TextAnchor};
    pub use day_spec::{Font, Role};
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
}

/// The drawing closure is a binding: signal reads re-record; layout size changes re-record
/// (via FrameChanged); replay is equality-gated by DrawOp's PartialEq (§4.2).
pub fn canvas(draw: impl Fn(&mut Draw, Size) + 'static) -> AnyPiece {
    use day_reactive::{Trigger, bind};
    piece_fn(move |cx| {
        let node = cx.leaf(kinds::CANVAS, &CanvasProps::default(), Flex::default());
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
    })
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
// Navigation (docs/navigation.md)
// ---------------------------------------------------------------------------

/// Navigate to a registered route ("" returns to the root). SwiftUI-reset semantics:
/// the stack is replaced so `path` becomes the top. False = no host / unknown route.
pub fn navigate(path: &str) -> bool {
    day_core::navigate(path)
}

/// Pop one navigation level. False = already at the root.
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

struct RouteDef {
    path: String,
    title: TextSource,
    build: Box<dyn Fn() -> AnyPiece>,
}

/// Native navigation host. Mobile toolkits present a stack (UINavigationController /
/// toolbar+pages) with native back; desktop toolkits present sidebar+detail split panes,
/// with the root content as the sidebar. v1: must be the app root.
pub struct Nav {
    title: TextSource,
    root: AnyPiece,
    routes: Vec<RouteDef>,
}

pub fn nav<M>(title: impl IntoText<M>, root: impl Piece) -> Nav {
    Nav {
        title: title.into_text(),
        root: AnyPiece::new(root),
        routes: Vec::new(),
    }
}

impl Nav {
    /// Register a destination: `path` is its route string (dayscript / deep links /
    /// `nav_link` all address it); `title` its navigation title; `build` runs lazily
    /// on each push, in a scope disposed on pop.
    pub fn route<M, P: Piece>(
        mut self,
        path: &str,
        title: impl IntoText<M>,
        build: impl Fn() -> P + 'static,
    ) -> Self {
        self.routes.push(RouteDef {
            path: path.to_string(),
            title: title.into_text(),
            build: Box::new(move || AnyPiece::new(build())),
        });
        self
    }
}

/// Build-time context linking `nav_menu()` pieces to the enclosing `nav()`'s routes.
#[derive(Clone)]
struct NavBuildCtx {
    /// (path, title) in registration order.
    route_meta: Rc<Vec<(String, String)>>,
    /// Menu nodes registered during the root build (selection-synced on push/pop).
    menus: Rc<RefCell<Vec<RNode>>>,
}

thread_local! {
    static NAV_BUILD: RefCell<Option<NavBuildCtx>> = const { RefCell::new(None) };
}

/// The enclosing `nav()`'s routes as a NATIVE navigation list: NSOutlineView source list
/// (macOS), `navigation-sidebar` GtkListBox, QListWidget, inset-grouped chevron rows
/// (iOS), ripple list rows (Android). Selecting an item navigates to its route; the
/// active route is highlighted in split presentation. Build it inside `nav()`'s root.
pub fn nav_menu() -> AnyPiece {
    use day_spec::props::NavMenuProps;
    piece_fn(|cx| {
        let Some(ctx) = NAV_BUILD.with(|c| c.borrow().clone()) else {
            eprintln!("day: nav_menu() built outside a nav() root — rendering nothing");
            return cx.layout_only(Rc::new(PassThrough), Flex::default(), Boundary::No);
        };
        let items: Vec<String> = ctx.route_meta.iter().map(|(_, t)| t.clone()).collect();
        let node = cx.native(
            kinds::NAV_MENU,
            &NavMenuProps {
                items,
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
        ctx.menus.borrow_mut().push(node);
        let meta = ctx.route_meta.clone();
        cx.on(node, move |ev| {
            if let Event::SelectionChanged(i) = ev
                && let Some((path, _)) = meta.get(*i as usize)
            {
                let _ = day_core::navigate(path);
            }
        });
        node
    })
}

struct NavEntry {
    path: String,
    scope: Scope,
    page: RNode,
}

/// Create a NAV_PAGE under `host` and wire its FrameChanged size reports into `sizes`.
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
        Boundary::Yes, // pages are layout boundaries: their size is native-owned
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

impl Piece for Nav {
    fn build(self, cx: &mut BuildCx) -> RNode {
        use day_spec::props::{NavPageProps, NavPatch, NavProps};
        let split =
            with_tree(|t| t.capability(day_spec::Cap::NavSplit)) == day_spec::Support::Native;
        let title = self.title.initial();
        let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();

        let host = cx.native(
            kinds::NAV,
            &NavProps {
                title: title.clone(),
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

        // Route table with build-time-resolved titles (dynamic titles are post-v1):
        // (path, title, lazy builder).
        type RouteTable = Rc<Vec<(String, String, Box<dyn Fn() -> AnyPiece>)>>;
        let routes: RouteTable = Rc::new(
            self.routes
                .into_iter()
                .map(|r| (r.path, r.title.initial(), r.build))
                .collect(),
        );
        let route_meta: Rc<Vec<(String, String)>> = Rc::new(
            routes
                .iter()
                .map(|(p, t, _)| (p.clone(), t.clone()))
                .collect(),
        );
        let menus: Rc<RefCell<Vec<RNode>>> = Rc::default();

        // Root page: the sidebar in split mode, the first stack page otherwise. Built with
        // the nav context installed so nav_menu() can pick up the routes.
        let root_page = nav_page(
            host,
            &NavPageProps {
                title: title.clone(),
                sidebar: split,
            },
            &sizes,
        );
        NAV_BUILD.with(|c| {
            *c.borrow_mut() = Some(NavBuildCtx {
                route_meta: route_meta.clone(),
                menus: menus.clone(),
            })
        });
        cx.under(root_page, |cx| {
            let _ = self.root.build(cx);
        });
        NAV_BUILD.with(|c| c.borrow_mut().take());

        let stack: Rc<RefCell<Vec<NavEntry>>> = Rc::default();
        let nav_scope = Scope::current();

        // Highlight the active route in every registered menu (split presentation keeps
        // the selection; mobile roots clear it when the stack empties).
        let sync_menus = {
            let (stack, menus, route_meta) = (stack.clone(), menus.clone(), route_meta.clone());
            move || {
                let top = stack.borrow().last().map(|e| e.path.clone());
                let sel = top.and_then(|p| route_meta.iter().position(|(path, _)| *path == p));
                for &m in menus.borrow().iter() {
                    with_tree(|t| {
                        t.patch(
                            m,
                            Box::new(day_spec::props::NavMenuPatch::Selected(sel)),
                            false,
                        )
                    });
                }
            }
        };

        // One-level push: build the destination page, then present it.
        let do_push = {
            let (routes, stack, sizes) = (routes.clone(), stack.clone(), sizes.clone());
            let sync_menus = sync_menus.clone();
            move |path: &str| -> bool {
                let Some((_, page_title, build)) = routes.iter().find(|(p, _, _)| p == path) else {
                    return false;
                };
                let depth = stack.borrow().len() + 1;
                let page = nav_page(
                    host,
                    &NavPageProps {
                        title: page_title.clone(),
                        sidebar: false,
                    },
                    &sizes,
                );
                let scope = nav_scope.enter(Scope::child);
                let (content, header_title) = (build(), page_title.clone());
                scope.enter(|| {
                    let mut cx = BuildCx::new(page);
                    if split {
                        // Desktop detail header: title + back affordance below depth 1
                        // (the sidebar handles first-level selection).
                        let header: AnyPiece = if depth > 1 {
                            AnyPiece::new(
                                row((
                                    button("←")
                                        .action(|| {
                                            let _ = day_core::nav_back();
                                        })
                                        .id("nav-back"),
                                    label(header_title).font(Font::Headline),
                                ))
                                .spacing(8.0),
                            )
                        } else {
                            AnyPiece::new(
                                row((label(header_title).font(Font::Headline),)).spacing(8.0),
                            )
                        };
                        let _ = column((header, divider(), content))
                            .spacing(8.0)
                            .align(HAlign::Leading)
                            .padding(12.0)
                            .build(&mut cx);
                    } else {
                        let _ = content.build(&mut cx);
                    }
                });
                stack.borrow_mut().push(NavEntry {
                    path: path.to_string(),
                    scope,
                    page,
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
                sync_menus();
                true
            }
        };

        // One-level pop. `already_popped` = the native side presented the pop already
        // (iOS back button/swipe) — sync the stack without re-issuing the patch.
        let do_pop = {
            let (stack, sizes) = (stack.clone(), sizes.clone());
            let sync_menus = sync_menus.clone();
            move |already_popped: bool| -> bool {
                let Some(entry) = stack.borrow_mut().pop() else {
                    return false;
                };
                if !already_popped {
                    with_tree(|t| t.patch(host, Box::new(NavPatch::Popped), false));
                }
                entry.scope.dispose();
                sizes.borrow_mut().remove(&entry.page);
                with_tree(|t| {
                    t.remove_subtree(entry.page);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
                sync_menus();
                true
            }
        };

        // navigate(path) = reset-to semantics (sidebar selection replaces the detail).
        let reset_to = {
            let (stack, do_push, do_pop) = (stack.clone(), do_push.clone(), do_pop.clone());
            move |path: &str| -> bool {
                let current = stack.borrow().last().map(|e| e.path.clone());
                if current.as_deref() == Some(path) {
                    return true;
                }
                if path.is_empty() {
                    while do_pop(false) {}
                    return true;
                }
                if !routes.iter().any(|(p, _, _)| p == path) {
                    return false;
                }
                while do_pop(false) {}
                do_push(path)
            }
        };

        day_core::register_nav(day_core::NavController {
            push: Box::new(reset_to),
            pop: Box::new(do_pop.clone()),
            current: Box::new({
                let stack = stack.clone();
                move || {
                    stack
                        .borrow()
                        .last()
                        .map(|e| e.path.clone())
                        .unwrap_or_default()
                }
            }),
        });

        // Idiomatic split default: select the first route at launch — desktop sidebars
        // don't present empty detail panes. (Startup deep links still win: they dispatch
        // after mount and replace the stack.)
        if split && let Some((first, _)) = route_meta.first() {
            let _ = do_push(first);
            sync_menus();
        }

        // Native back (iOS back button/swipe, Android system back / toolbar up) and warm
        // deep links (Android onNewIntent routes them here as Custom("deeplink")).
        cx.on(host, move |ev| match ev {
            Event::NavBack { already_popped } => {
                let _ = do_pop(*already_popped);
            }
            Event::Custom("deeplink", route) => {
                let _ = day_core::navigate(route);
            }
            _ => {}
        });

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
