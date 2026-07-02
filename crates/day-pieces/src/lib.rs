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
use day_spec::{A11yProps, Color, DrawOp, Event, Font, Insets, Point, Rect, Role, Shape, Size, kinds};

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
    fn bind_to(self, node: RNode, make_patch: impl Fn(String) -> Box<dyn std::any::Any> + 'static, affects_size: bool) {
        if let TextSource::Dyn(f) = self {
            let seed = day_reactive::untrack(|| f());
            bind_seeded(seed, move || f(), move |t: &String| {
                let patch = make_patch(t.clone());
                with_tree(|tr| tr.patch(node, patch, affects_size));
            });
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
    Label { text: text.into_text(), font: Font::Body, color: None }
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
            &LabelProps { text: initial, font: self.font, color: self.color, wraps: true },
            Flex::default(),
        );
        self.text.bind_to(node, |t| Box::new(LabelPatch::Text(t)), true);
        node
    }
}

pub struct Button {
    title: TextSource,
    action: Option<Rc<dyn Fn()>>,
}

pub fn button<M>(title: impl IntoText<M>) -> Button {
    Button { title: title.into_text(), action: None }
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
            &ButtonProps { title: initial, enabled: true },
            Flex::default(),
        );
        if let Some(action) = self.action {
            cx.on(node, move |ev| {
                if matches!(ev, Event::Pressed) {
                    action();
                }
            });
        }
        self.title.bind_to(node, |t| Box::new(ButtonPatch::Title(t)), true);
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
        let node = cx.leaf(kinds::TOGGLE, &ToggleProps { on: initial, enabled: true }, Flex::default());
        let v = self.value.clone();
        bind_seeded(initial, move || v.get_rw(), move |on: &bool| {
            with_tree(|t| t.patch(node, Box::new(TogglePatch::On(*on)), false));
        });
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
    Slider { value, min: 0.0, max: 1.0, step: None }
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
            &SliderProps { value: initial, min: self.min, max: self.max, step: self.step, enabled: true },
            Flex { grow_w: true, ..Default::default() },
        );
        let v = self.value.clone();
        bind_seeded(initial, move || v.get_rw(), move |val: &f64| {
            with_tree(|t| t.patch(node, Box::new(SliderPatch::Value(*val)), false));
        });
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
    TextField { value, placeholder: None }
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
        let ph = self.placeholder.as_ref().map(|p| p.initial()).unwrap_or_default();
        let node = cx.leaf(
            kinds::TEXT_FIELD,
            &TextFieldProps { text: initial.clone(), placeholder: ph, enabled: true },
            Flex { grow_w: true, ..Default::default() },
        );
        // Controlled input with origin-tagged writes (§4.4): the echo guard remembers the
        // last value that came FROM the native widget so its own change is not written back.
        let guard: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));
        let v = self.value.clone();
        let g = guard.clone();
        bind_seeded(initial, move || v.get_rw(), move |t: &String| {
            let from_native = g.borrow_mut().take().as_deref() == Some(t.as_str());
            with_tree(|tr| {
                tr.patch(
                    node,
                    Box::new(TextFieldPatch::Text { text: t.clone(), from_native }),
                    false,
                )
            });
        });
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

pub struct Divider;

pub fn divider() -> Divider {
    Divider
}

impl Piece for Divider {
    fn build(self, cx: &mut BuildCx) -> RNode {
        cx.leaf(kinds::DIVIDER, &(), Flex { grow_w: true, ..Default::default() })
    }
}

pub struct Spacer;

pub fn spacer() -> Spacer {
    Spacer
}

impl Piece for Spacer {
    fn build(self, cx: &mut BuildCx) -> RNode {
        cx.layout_only(Rc::new(PassThrough), Flex { is_spacer: true, ..Default::default() }, false)
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
    Column { children, spacing: 0.0, align: CrossAlign::Center }
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
            Rc::new(StackLayout { axis: Axis::Vertical, spacing: self.spacing, align: self.align }),
            Flex::default(),
            false,
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
    Row { children, spacing: 0.0, align: CrossAlign::Center }
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
            Rc::new(StackLayout { axis: Axis::Horizontal, spacing: self.spacing, align: self.align }),
            Flex::default(),
            false,
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
    Scroll { child, axis: Axis::Vertical }
}

impl<P: Piece> Piece for Scroll<P> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let node = cx.native(
            kinds::SCROLL,
            &ContainerProps::default(),
            Rc::new(ScrollLayout { axis: self.axis }),
            Flex { grow_w: true, grow_h: true, ..Default::default() },
            true, // scroll viewports are layout boundaries (§7.4)
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
            Flex { is_group: true, ..Default::default() },
            false,
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

        let initial = day_reactive::untrack(|| cond());
        mount(initial);
        watch(move || cond(), move |now, old| {
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

struct EachRow<K> {
    key: K,
    scope: Scope,
    root: RNode,
    sig_set: Box<dyn Fn(&dyn std::any::Any)>,
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
            Flex { is_group: true, ..Default::default() },
            false,
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

        let initial = day_reactive::untrack(|| items());
        sync(&initial);
        watch(move || items(), move |new, _| sync(new));
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
            let w = cx.layout_only(Rc::new(PaddingLayout { insets }), Flex::default(), false);
            cx.under(w, |cx| {
                let _ = self.build(cx);
            });
            w
        })
    }

    fn frame(self, width: f64, height: f64) -> AnyPiece {
        piece_fn(move |cx| {
            let w = cx.layout_only(
                Rc::new(FrameLayout { width: Some(width), height: Some(height) }),
                Flex::default(),
                true, // two-axis fixed frame = layout boundary (§7.4)
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
    pub use crate::{
        A11yBuilder, Decorate, Draw, HAlign, IntoText, ItemSlot, SignalRw, VAlign, button,
        canvas, column, divider, each, image, label, row, scroll, slider, spacer, text_field,
        toggle, when,
    };
    pub use day_core::{AnyPiece, BuildCx, Piece, PieceSeq, PieceVec, piece_fn};
    pub use day_geometry::{Color, Insets, Point, Rect, Size};
    pub use day_spec::{DrawOp, Shape};
    pub use day_reactive::{
        Effect, Memo, Scope, Setter, Signal, Trigger, batch, bind, untrack, watch,
    };
    pub use day_spec::{Font, Role};
}

// ---------------------------------------------------------------------------
// Canvas (§11): record a display list reactively; backends replay natively.
// ---------------------------------------------------------------------------

pub struct Draw {
    ops: Vec<DrawOp>,
}

impl Draw {
    pub fn fill(&mut self, shape: Shape, color: Color) {
        self.ops.push(DrawOp::Fill(shape, color));
    }
    pub fn stroke(&mut self, shape: Shape, color: Color, width: f64) {
        self.ops.push(DrawOp::Stroke(shape, color, width));
    }
    pub fn text(&mut self, text: &str, at: Point, size: f64, color: Color, centered: bool) {
        self.ops.push(DrawOp::Text { text: text.to_owned(), at, size, color, centered });
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
            &ImageProps { source: name, decorative: false },
            Flex::default(),
        )
    })
}
