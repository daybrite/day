// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `Decorate`, the chainable modifiers every piece inherits: padding and sizing, background and
//! corner radius, gestures (`on_tap`, drag), accessibility (`A11yBuilder`), and native-handle
//! capture (`NativeRef`), plus the `Modifier` supporting trait.
//!
//! Modifiers return [`Decorated<P>`], which keeps the decorated piece's type, so a chain
//! never stops reaching that piece's builder methods (§5.2).

use std::cell::Cell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal, bind};
use day_spec::props::*;
use day_spec::{
    A11yProps, Angle, AnimSpec, Cursor, Event, Ink, Insets, Paint, Role, Transform, kinds,
};

use crate::menus::lower_menu_scoped;
use crate::*;

// ---------------------------------------------------------------------------
// Decorators (§5.2 Decorate)
// ---------------------------------------------------------------------------

/// A one-shot, by-value view transform (the SwiftUI `ViewModifier` analog): wrap a piece into a
/// new one. Pure composition, with no per-backend work. A plain `FnOnce(AnyPiece) -> AnyPiece`
/// closure is a `Modifier` too (the blanket impl below), so the common case needs no new type.
/// Apply one with [`Decorate::modifier`].
///
/// **`AnyPiece` here is a trade, not a requirement.** It is not object safety: a `Modifier` is
/// never stored as `dyn Modifier`, and [`Decorate::modifier`] takes `impl Modifier` and applies
/// it on the spot. It is the closure impl below. A closure has one fixed parameter type, and
/// Rust has no `for<P> FnOnce(P) -> _` bound (higher-ranked bounds range over lifetimes, not
/// types), so making `apply` generic over the content (`fn apply<P: Piece>(self, c: P) ->
/// Self::Out<P>`, which a named modifier can satisfy with a GAT) would leave no closure able to
/// implement the trait at all. Pinning the input to the one piece type that accepts anything is
/// what keeps `|p| …` a modifier, and the erasure is what that costs, which is why this is the
/// single `Decorate` method that erases.
///
/// If a modifier ever needs to preserve its content's type, parameterize the trait rather than
/// the method: `trait Modifier<P: Piece> { type Out: Piece; fn apply(self, c: P) -> Self::Out; }`.
/// Closures survive that (`impl<P: Piece, O: Piece, F: FnOnce(P) -> O> Modifier<P> for F`, and
/// their parameter still infers unannotated), while a named modifier such as day-piece-rating's
/// `Card` becomes `impl<P: Piece> Modifier<P> for Card` with `type Out = Decorated<P>`. The bill
/// is a `Decorate::modifier` whose return type varies per modifier, a generic impl for every
/// named one, and a breaking change: not worth paying while the tree has two implementors.
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

/// A liveness-checked reference to a mounted piece's realized node: the retained half of the
/// tweaks API (docs/tweaks.md). Capture one with [`Decorate::native_ref`], then reach the native
/// widget later (from event handlers, timers) through a toolkit ext accessor. `node`/`with` yield
/// `None` before mount and after the node's subtree is disposed, so async races are safe no-ops.
///
/// Reads are reactive: inside a binding or memo, `node()` subscribes to the ref's mount/clear
/// transitions (a `Trigger` underneath), so a label like
/// `label(move || if r.node().is_some() { "live" } else { "cleared" })` updates when the
/// referenced piece unmounts, as in the toggle demo on the showcase Tweaks page. (The
/// `when`-arm's disposal lands at the turn boundary, after ordinary bindings re-ran;
/// piggybacking on some other signal would read a stale mount state, while the trigger fires
/// at the actual transition.) Main-thread only, like every realized-tree type.
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
    #[must_use]
    pub fn new() -> Self {
        NativeRef {
            cell: Rc::new(std::cell::Cell::new(None)),
            changed: day_reactive::Trigger::new(),
        }
    }

    /// The mounted node, if it is currently live. A tracked read (see the type docs).
    #[must_use]
    pub fn node(&self) -> Option<day_core::RNode> {
        self.changed.track();
        let node = self.cell.get()?;
        // Generational slotmap keys make a disposed node a clean miss, never a stale hit.
        let live = day_core::try_with_tree(|t| t.node_kind(node).is_some()).unwrap_or(false);
        live.then_some(node)
    }

    /// Run `f` with the live node (e.g. inside `day_toolkit_appkit::with_native`); `None` if disposed.
    pub fn with<R>(&self, f: impl FnOnce(day_core::RNode) -> R) -> Option<R> {
        self.node().map(f)
    }

    fn transition(&self, node: Option<day_core::RNode>) {
        self.cell.set(node);
        self.changed.notify();
    }
}

/// A transparent native layer node (`CONTAINER`, no fill/clip/corner) used by the animatable
/// modifiers (`.opacity`/`.transform`) to carry a per-node opacity or transform. Layout-transparent (`FillThrough`), so it never affects sizing and a
/// granted stretch flows through to what it paints.
fn layer_node(cx: &mut BuildCx) -> RNode {
    cx.native(
        kinds::CONTAINER,
        &ContainerProps {
            corner_radius: 0.0,
            clips: false,
            role: None,
            ..Default::default()
        },
        Rc::new(FillThrough),
        Flex::default(),
        Boundary::No,
    )
}

// ---------------------------------------------------------------------------
// Decorated: a piece plus its modifiers, with the piece's type kept (§5.2)
// ---------------------------------------------------------------------------

/// The build of a piece plus every modifier applied to it so far.
type Build = Box<dyn FnOnce(&mut BuildCx) -> RNode>;

/// A piece with modifiers chained onto it, keeping the decorated piece's type (§5.2).
///
/// Every [`Decorate`] modifier returns one of these rather than erasing to [`AnyPiece`], which is
/// what lets a chain keep reaching the piece's builder methods: `label(…).padding(8.0)` is
/// still a decorated `Label`, so `.font(…)` after it resolves. Modifiers applied to a `Decorated`
/// append in place (the inherent methods below shadow the trait's), so a chain stays flat instead
/// of nesting `Decorated<Decorated<…>>`.
///
/// Modifiers fall into three classes (docs/api-style.md "Modifier classes"), and only one of
/// them is ordered:
///
/// - **Identity** (`id`, `id_of`) names the piece's own node, the one its build
///   returns, wherever it sits in the chain: `label(t).padding(8.0).id(day_core::id!(x))` and
///   `label(t).id(day_core::id!(x)).padding(8.0)` are the same tree.
/// - **Wrapping** modifiers (`padding`, `frame`, `background`, `grow`, …) each add a node around
///   what came before, so they compose outward in call order, as they read.
/// - **Outer-layout facts** (`grid_span`, `grid_align`) describe the piece to the container it
///   sits in, so they land on the outermost node whatever wrappers follow them.
///
/// `wrapper_id` is the one way to name a wrapper deliberately: it tags whatever node is
/// outermost at the point it is called, for a frame assertion on a padded box.
///
/// Erase explicitly with `.any()` when a single [`AnyPiece`] is what's needed (a `Vec<AnyPiece>`, a
/// `-> AnyPiece` signature).
#[must_use]
pub struct Decorated<P> {
    inner: P,
    /// Wrapping modifiers, applied outward in call order.
    ops: Vec<Box<dyn FnOnce(Build) -> Build>>,
    /// Identity modifiers, applied to the node the piece's own build returns.
    node_ops: Vec<Box<dyn FnOnce(RNode)>>,
    /// Outer-layout facts, applied to the outermost node once every wrapper is in place.
    outer_ops: Vec<Box<dyn FnOnce(RNode)>>,
}

impl<P: Piece> Decorated<P> {
    /// An undecorated piece, ready to have modifiers applied CONDITIONALLY. Starting here gives
    /// every branch the same type, so an optional modifier needs no erasure:
    ///
    /// ```ignore
    /// let leaf = Decorated::new(draw);
    /// let leaf = match id { Some(id) => leaf.id(id), None => leaf };
    /// let leaf = if editable { leaf.on_tap(f) } else { leaf };
    /// ```
    ///
    /// [`apply_if`](Self::apply_if) and [`apply_some`](Self::apply_some) are the same move as a
    /// chain step.
    pub fn new(inner: P) -> Self {
        Decorated {
            inner,
            ops: Vec::new(),
            node_ops: Vec::new(),
            outer_ops: Vec::new(),
        }
    }

    fn push(mut self, op: impl FnOnce(Build) -> Build + 'static) -> Self {
        self.ops.push(Box::new(op));
        self
    }

    fn push_node(mut self, op: impl FnOnce(RNode) + 'static) -> Self {
        self.node_ops.push(Box::new(op));
        self
    }

    fn push_outer(mut self, op: impl FnOnce(RNode) + 'static) -> Self {
        self.outer_ops.push(Box::new(op));
        self
    }

    /// Replace the undecorated piece, keeping the modifier chain: how a typed builder trait
    /// reaches through a decoration (docs/api-style.md "Typed builders"). `f` sees the piece as
    /// it was before any modifier, which is why modifier order stops mattering.
    pub fn map_inner<Q: Piece>(self, f: impl FnOnce(P) -> Q) -> Decorated<Q> {
        Decorated {
            inner: f(self.inner),
            ops: self.ops,
            node_ops: self.node_ops,
            outer_ops: self.outer_ops,
        }
    }

    /// Apply `f` to the chain when `cond` holds, else continue unchanged: the conditional
    /// modifier, with no `let x = if c { x.m() } else { x }` rebinding and no erasure.
    ///
    /// ```ignore
    /// nav(sel).apply_if(!primary, |n| n.local())
    /// ```
    ///
    /// `cond` is a plain `bool` resolved at build, like the arms of a `match`; the modifiers
    /// `f` applies may themselves be reactive.
    pub fn apply_if(self, cond: bool, f: impl FnOnce(Self) -> Self) -> Self {
        if cond { f(self) } else { self }
    }

    /// Apply `f` with the value when there is one, else continue unchanged:
    /// `canvas(draw).apply_some(id, |c, id| c.id(id))`.
    pub fn apply_some<T>(self, value: Option<T>, f: impl FnOnce(Self, T) -> Self) -> Self {
        match value {
            Some(v) => f(self, v),
            None => self,
        }
    }
}

impl<P: Piece> Piece for Decorated<P> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Decorated {
            inner,
            ops,
            node_ops,
            outer_ops,
        } = self;
        // The piece's own node first, with its identity; then the wrappers, composed outward
        // in call order (the first modifier is innermost); then the facts for the parent, on
        // whatever ended up outermost.
        let wraps = !ops.is_empty();
        let own = Rc::new(Cell::new(None::<RNode>));
        let own_out = own.clone();
        let mut build: Build = Box::new(move |cx| {
            let n = inner.build(cx);
            for op in node_ops {
                op(n);
            }
            own_out.set(Some(n));
            n
        });
        for op in ops {
            build = op(build);
        }
        let outer = build(cx);
        // The span a gesture addressed to the piece's own node (where `.id` sits) climbs to
        // reach a wrapper's `.on_tap` (day-core's `gesture_target`).
        if wraps && let Some(n) = own.get() {
            with_tree(|t| t.set_element_outer(n, outer));
        }
        for op in outer_ops {
            op(outer);
        }
        outer
    }
}

// --- The modifier bodies, written once and shared by the trait and the inherent impl ---

fn op_id(id: day_core::ElementId) -> impl FnOnce(RNode) {
    move |n| with_tree(|t| t.set_id(t.element_target(n), id.as_str().to_owned()))
}

fn op_id_of(id: impl Fn() -> day_core::ElementId + 'static) -> impl FnOnce(RNode) {
    move |n| {
        day_reactive::bind(id, move |s: &day_core::ElementId| {
            let s = s.as_str().to_owned();
            with_tree(|t| t.set_id(t.element_target(n), s));
        });
    }
}

/// Name whatever node is outermost at this point in the chain (see [`Decorated::wrapper_id`]).
fn op_wrapper_id(id: day_core::ElementId) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.set_id(n, id.as_str().to_owned()));
            n
        })
    }
}

fn op_tweak(f: impl FnOnce(day_core::RNode) + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            f(n);
            // Mark the node so a later backing swap (`.selectable()` on a toolkit that
            // rebuilds the widget) warns about the discarded tweak instead of losing it
            // silently (docs/tweaks.md).
            with_tree(|t| t.note_node_tweaked(n));
            n
        })
    }
}

fn op_selectable() -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.set_node_selectable(n, true));
            n
        })
    }
}

fn op_cursor(cursor: Reactive<Cursor>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            // Read before borrowing the tree: a reactive cursor is app code (a composed split's
            // follows the app's axis closure), and app code may itself reach the tree.
            let shape = cursor.get_untracked();
            with_tree(|t| t.set_node_cursor(n, shape));
            // Only a reactive source needs a binding; a constant shape is applied once at
            // mount. The setter is idempotent, so a re-run costs one native call.
            if let Reactive::Dyn(_) = &cursor {
                bind(
                    move || cursor.get(),
                    move |c: &Cursor| {
                        with_tree(|t| t.set_node_cursor(n, c.clone()));
                    },
                );
            }
            n
        })
    }
}

fn op_layout_direction(
    direction: Reactive<day_spec::LayoutDirection>,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            let seed = direction.get_untracked();
            with_tree(|t| t.set_node_direction(n, Some(seed)));
            if let Reactive::Dyn(_) = &direction {
                bind(
                    move || direction.get(),
                    move |d: &day_spec::LayoutDirection| {
                        with_tree(|t| {
                            t.set_node_direction(n, Some(*d));
                            t.layout_if_needed();
                        });
                    },
                );
            }
            n
        })
    }
}

fn op_window_drag_region() -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.set_node_drag_region(n, true));
            n
        })
    }
}

fn op_native_ref(r: NativeRef) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            r.transition(Some(n));
            let cleared = r.clone();
            Scope::current().on_cleanup(move || cleared.transition(None));
            n
        })
    }
}

fn op_padding(insets: Reactive<Insets>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let seed = insets.get_untracked();
            let layout = Rc::new(PaddingLayout {
                insets: Cell::new(seed),
            });
            let w = cx.layout_only(layout.clone(), Flex::default(), Boundary::No);
            cx.under(w, |cx| {
                let _ = inner(cx);
            });
            // A constant pads once; a signal or a closure (`move || day::safe_area()`) re-pads
            // in place when it changes, a rotation included.
            if let Reactive::Dyn(_) = &insets {
                day_reactive::bind_seeded(
                    seed,
                    move || insets.get(),
                    move |v: &Insets| {
                        layout.insets.set(*v);
                        with_tree(|t| t.mark_needs_measure(w));
                    },
                );
            }
            w
        })
    }
}

/// Follow a reactive length into the `Cell` a layout reads it from, re-measuring on change. A
/// constant needs no binding.
fn bind_length(value: Reactive<f64>, cell: impl Fn(f64) + 'static, node: RNode) {
    if let Reactive::Dyn(_) = &value {
        let seed = value.get_untracked();
        day_reactive::bind_seeded(
            seed,
            move || value.get(),
            move |v: &f64| {
                cell(*v);
                with_tree(|t| t.mark_needs_measure(node));
            },
        );
    }
}

fn op_max_width(max: Reactive<f64>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let layout = Rc::new(MaxWidthLayout {
                max: Cell::new(max.get_untracked()),
            });
            let w = cx.layout_only(layout.clone(), Flex::default(), Boundary::No);
            cx.under(w, |cx| {
                let _ = inner(cx);
            });
            bind_length(max, move |v| layout.max.set(v), w);
            w
        })
    }
}

fn op_max_height(max: Reactive<f64>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            // Yielding, so a column sizes its other children first and gives this one what is
            // left, up to `max`: a capped scroll gives way to the button under it on a short
            // screen instead of pushing it out of the column, and the column still hugs.
            let layout = Rc::new(day_core::MaxHeightLayout {
                max: Cell::new(max.get_untracked()),
            });
            let h = cx.layout_only(
                layout.clone(),
                Flex {
                    yields_h: true,
                    ..Flex::default()
                },
                Boundary::No,
            );
            cx.under(h, |cx| {
                let _ = inner(cx);
            });
            bind_length(max, move |v| layout.max.set(v), h);
            h
        })
    }
}

fn op_min_width(min: Reactive<f64>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let layout = Rc::new(day_core::MinWidthLayout {
                min: Cell::new(min.get_untracked()),
            });
            let w = cx.layout_only(layout.clone(), Flex::default(), Boundary::No);
            cx.under(w, |cx| {
                let _ = inner(cx);
            });
            bind_length(min, move |v| layout.min.set(v), w);
            w
        })
    }
}

fn op_min_height(min: Reactive<f64>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let layout = Rc::new(MinHeightLayout {
                min: Cell::new(min.get_untracked().max(0.0)),
            });
            let node = cx.layout_only(layout.clone(), Flex::default(), Boundary::No);
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            bind_length(min, move |v| layout.min.set(v.max(0.0)), node);
            node
        })
    }
}

fn op_reserving(sample: TextSource) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let wrapper = cx.layout_only(
                Rc::new(day_core::ReserveLayout),
                Flex::default(),
                Boundary::No,
            );
            cx.under(wrapper, |cx| {
                let content = inner(cx);
                // Find the label through single-child wrappers after all typed builders have
                // configured it. Multi-child pieces use the default Body font.
                let font = with_tree(|tree| {
                    let mut node = content;
                    loop {
                        if let Some(font) = tree.node_probe(node).and_then(|p| p.font) {
                            break font;
                        }
                        if tree.child_count(node) != 1 {
                            break day_spec::FontSpec::from(day_spec::Font::Body);
                        }
                        let Some(child) = tree.first_child(node) else {
                            break day_spec::FontSpec::from(day_spec::Font::Body);
                        };
                        node = child;
                    }
                });
                let sample_node = cx.leaf(
                    day_spec::kinds::LABEL,
                    &LabelProps {
                        text: sample.initial(),
                        font,
                        wraps: false,
                        ..Default::default()
                    },
                    Flex::default(),
                );
                with_tree(|tree| {
                    tree.set_node_opacity(sample_node, 0.0);
                    tree.set_a11y(
                        sample_node,
                        A11yProps {
                            hidden: true,
                            decorative: true,
                            ..Default::default()
                        },
                    );
                });
                sample.bind_to(sample_node, |s| Box::new(LabelPatch::Text(s)), true);
            });
            wrapper
        })
    }
}

// The layout reads stored sizes, never signals. Bindings update those sizes outside the
// tree borrow and invalidate measurement; native children and their scopes remain mounted.
struct ReactiveFrameLayout {
    size: Cell<(Option<f64>, Option<f64>)>,
}
impl ReactiveFrameLayout {
    fn frame(&self) -> FrameLayout {
        let (width, height) = self.size.get();
        FrameLayout { width, height }
    }
}
impl Layout for ReactiveFrameLayout {
    fn measure(
        &self,
        cx: &mut dyn LayoutOps,
        children: &[RNode],
        p: day_spec::Proposal,
    ) -> day_spec::Size {
        self.frame().measure(cx, children, p)
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: day_spec::Rect) {
        self.frame().place(cx, children, bounds);
    }
    fn baseline(
        &self,
        cx: &mut dyn LayoutOps,
        children: &[RNode],
        size: day_spec::Size,
    ) -> Option<f64> {
        self.frame().baseline(cx, children, size)
    }
}

fn frame_extent(value: f64) -> f64 {
    if value.is_finite() {
        value.max(0.0)
    } else {
        0.0
    }
}

fn op_frame(
    width: Option<Reactive<f64>>,
    height: Option<Reactive<f64>>,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let dynamic =
                matches!(width, Some(Reactive::Dyn(_))) || matches!(height, Some(Reactive::Dyn(_)));
            let read = move || {
                (
                    width.as_ref().map(|v| frame_extent(v.get())),
                    height.as_ref().map(|v| frame_extent(v.get())),
                )
            };
            let seed = day_reactive::untrack(&read);
            let layout = Rc::new(ReactiveFrameLayout {
                size: Cell::new(seed),
            });
            // Only a constant two-axis frame stops measurement propagation. A changing
            // frame must invalidate ancestors too, so siblings move to their new positions.
            let boundary = if !dynamic && seed.0.is_some() && seed.1.is_some() {
                Boundary::Yes
            } else {
                Boundary::No
            };
            let node = cx.layout_only(layout.clone(), Flex::default(), boundary);
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            if dynamic {
                day_reactive::bind_seeded(seed, read, move |size| {
                    layout.size.set(*size);
                    with_tree(|t| t.mark_needs_measure(node));
                });
            }
            node
        })
    }
}

fn op_a11y(f: impl FnOnce(A11yBuilder) -> A11yBuilder + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            let (props, live, selected) = f(A11yBuilder::default()).split();
            let seed = props.selected;
            with_tree(|t| t.set_a11y(n, props));
            // The selected state follows its signal the same way: re-sent alone, merged by
            // day-core onto the node's set.
            if let Some(selected) = selected {
                day_reactive::bind_seeded(
                    seed.unwrap_or(false),
                    move || selected.get(),
                    move |on: &bool| {
                        with_tree(|t| {
                            t.set_a11y(
                                n,
                                A11yProps {
                                    selected: Some(*on),
                                    ..Default::default()
                                },
                            );
                        });
                    },
                );
            }
            // A string that reads a signal or the locale re-sends itself alone; day-core
            // merges it onto the node's set and re-applies the whole picture (§13).
            for (src, patch) in live {
                if let TextSource::Dyn(read) = src {
                    let seed = day_reactive::untrack(|| read());
                    day_reactive::bind_seeded(
                        seed,
                        move || read(),
                        move |s: &String| with_tree(|t| t.set_a11y(n, patch(s.clone()))),
                    );
                }
            }
            n
        })
    }
}

fn op_on_tap(f: impl Fn() + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.enable_gesture(n, GestureKind::Tap));
            cx.on(n, move |ev| {
                if matches!(ev, Event::Tap(_)) {
                    f();
                }
            });
            n
        })
    }
}

fn op_on_tap_at(f: impl Fn(day_spec::Point) + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.enable_gesture(n, GestureKind::Tap));
            cx.on(n, move |ev| {
                if let Event::Tap(p) = ev {
                    f(*p);
                }
            });
            n
        })
    }
}

/// A key handler for presses (`released: false`, which includes repeats) or releases.
fn op_on_key(
    released: bool,
    f: impl Fn(&day_spec::KeyEvent) + 'static,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            // Declare the intent as well as listening: a backend whose focused view would have
            // to claim the key from the platform's dispatch checks this first. That check runs
            // on the view, which sits below `n` when `n` is a layout wrapper (`.frame(..)`), so
            // the view is marked too; the key it claims reaches this handler through the
            // wrapper (day-core's dispatch).
            day_spec::keys::mark(day_core::rnode_to_id(n));
            if let Some(view) = with_tree(|t| t.native_view_of(n))
                && view != n
            {
                day_spec::keys::mark(day_core::rnode_to_id(view));
            }
            cx.on(n, move |ev| {
                if let Event::Key(k) = ev
                    && k.is_release() == released
                {
                    f(k);
                }
            });
            n
        })
    }
}

/// Declare toolbar items for the chrome this piece sits under (docs/toolbars.md).
///
/// The chrome is resolved where the piece is built, not where the modifier was written: the
/// innermost navigation page being built, or the window when there is none. The design exists so
/// that a command sits beside the content it acts on, and the app never says twice which bar it
/// belongs to.
fn op_toolbar(source: crate::ToolbarSource) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            crate::contribute(day_core::current_chrome(), source);
            n
        })
    }
}

fn op_focusable() -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.set_focusable(n, true));
            n
        })
    }
}

fn op_keyboard() -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            // The same two marks `on_key` leaves (the node and the native view under any
            // layout wrapper), on the registry the full route reads.
            day_spec::keys::mark_text(day_core::rnode_to_id(n));
            if let Some(view) = with_tree(|t| t.native_view_of(n))
                && view != n
            {
                day_spec::keys::mark_text(day_core::rnode_to_id(view));
            }
            n
        })
    }
}

fn op_focused(
    want: Box<dyn Fn() -> bool>,
    on_native: Box<dyn Fn(bool)>,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            // Echo cell: the control's focus state as last reported by the native side. An
            // apply whose desired state matches it is the echo of a native change (or already
            // satisfied) and must not re-drive the toolkit: the nav host echo-cell rule.
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
            // window (dialog default focus). The initial `false` is not applied, because resigning
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
}

fn op_context_menu(items: Vec<MenuEntry>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            // The overlay host places the decorated subtree at its own bounds and the
            // composed-menu mount beside it (see `OverlayHost`; a plain sibling would sit
            // under a single-child layout, never placed).
            let w = cx.layout_only(Rc::new(OverlayHost), Flex::default(), Boundary::No);
            let mut n = w;
            cx.under(w, |cx| {
                n = inner(cx);
                // Scoped: the action closures die with the build scope, not the process;
                // an unscoped registration here leaks one closure per remount.
                let model = lower_menu_scoped(items);
                with_tree(|t| t.set_context_menu(n, model.clone()));
                // A backend with no native menu reports the summon instead; the composed
                // presenter replays the same lowered model (docs/menus.md).
                let model = std::rc::Rc::new(model);
                crate::menus::mount_composed_menu(cx, n, Rc::new(move |_p| (*model).clone()));
            });
            // Sized by its content, so a platform row stays one through the menu's wrapper.
            with_tree(|t| t.forward_whole_row(w, n));
            // Later ops decorate the content node: ids, gestures and menus belong to it,
            // while the wrapper only exists to keep the mount in the layout pass.
            n
        })
    }
}

/// The `.context_menu*` wrapper's layout: the decorated subtree fills the wrapper; every
/// other child (the composed menu's lazy mount) is layout-inert but must still be visited,
/// because a cover lays its content out from the size the backend reports, but only if the
/// place pass reaches it at all.
pub(crate) struct OverlayHost;

impl Layout for OverlayHost {
    fn measure(
        &self,
        cx: &mut dyn LayoutOps,
        children: &[day_core::RNode],
        p: day_spec::Proposal,
    ) -> day_spec::Size {
        match children.first() {
            Some(&c) => cx.measure_child(c, p),
            None => day_spec::Size::ZERO,
        }
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[day_core::RNode], bounds: day_spec::Rect) {
        let mut it = children.iter();
        if let Some(&c) = it.next() {
            cx.place_child(c, day_spec::Rect::from_size(bounds.size));
        }
        for &c in it {
            cx.place_child(c, day_spec::Rect::ZERO);
        }
    }
    fn baseline(
        &self,
        cx: &mut dyn LayoutOps,
        children: &[day_core::RNode],
        size: day_spec::Size,
    ) -> Option<f64> {
        children.first().and_then(|&c| cx.baseline_of(c, size))
    }
}

fn op_context_menu_fn(
    f: Rc<dyn Fn(day_spec::Point) -> Vec<MenuEntry>>,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let w = cx.layout_only(Rc::new(OverlayHost), Flex::default(), Boundary::No);
            let mut out = w;
            cx.under(w, |cx| {
                out = op_context_menu_fn_body(inner, f, cx);
            });
            // Sized by its content, so a platform row stays one through the menu's wrapper.
            with_tree(|t| t.forward_whole_row(w, out));
            out
        })
    }
}

fn op_context_menu_fn_body(
    inner: Build,
    f: Rc<dyn Fn(day_spec::Point) -> Vec<MenuEntry>>,
    cx: &mut day_core::BuildCx,
) -> day_core::RNode {
    {
        {
            let n = inner(cx);
            // Each summon lowers a fresh menu whose action closures live in their own scope,
            // disposed when the next summon replaces them (and with the build scope at
            // teardown), so per-click menus never accumulate registrations.
            let last: Rc<std::cell::RefCell<Option<day_reactive::Scope>>> = Rc::default();
            {
                let last = last.clone();
                day_reactive::Scope::current().on_cleanup(move || {
                    if let Some(s) = last.borrow_mut().take() {
                        s.dispose();
                    }
                });
            }
            let provider: day_spec::ContextMenuFn = Rc::new(move |p| {
                if let Some(s) = last.borrow_mut().take() {
                    s.dispose();
                }
                let scope = day_reactive::Scope::child();
                let items = scope.enter(|| crate::menus::lower_menu_scoped(f(p)));
                *last.borrow_mut() = Some(scope);
                items
            });
            with_tree(|t| t.set_context_menu_fn(n, provider.clone()));
            // A backend with no native menu reports the summon instead; the composed
            // presenter calls the same provider there (docs/menus.md).
            crate::menus::mount_composed_menu(cx, n, provider);
            n
        }
    }
}

/// Enable `kind` on the node the modifier wraps and hand its events to `on`.
fn op_gesture(
    kind: GestureKind,
    on: impl Fn(RNode, &Event) + 'static,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.enable_gesture(n, kind));
            cx.on(n, move |ev| on(n, ev));
            n
        })
    }
}

fn op_on_drag(
    axis: Option<day_spec::DragAxis>,
    f: impl Fn(Drag) + 'static,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| {
                t.enable_gesture(n, GestureKind::Drag);
                if axis.is_some() {
                    t.set_drag_axis(n, axis);
                }
            });
            cx.on(n, move |ev| {
                if let Event::Drag {
                    phase,
                    location,
                    translation,
                } = ev
                {
                    let velocity =
                        day_core::gesture::velocity(day_core::rnode_to_id(n), GestureKind::Drag);
                    f(Drag::new(*phase, *location, *translation).with_velocity(velocity));
                }
            });
            n
        })
    }
}

fn op_on_pinch(f: impl Fn(Pinch) + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::Pinch, move |_, ev| {
        if let Event::Pinch {
            phase,
            scale,
            location,
        } = ev
        {
            f(Pinch::new(*phase, *scale, *location));
        }
    })
}

fn op_on_rotate(f: impl Fn(Rotate) + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::Rotate, move |_, ev| {
        if let Event::Rotate {
            phase,
            rotation,
            location,
        } = ev
        {
            f(Rotate::new(*phase, Angle::rad(*rotation), *location));
        }
    })
}

fn op_on_hover(f: impl Fn(Option<day_spec::Point>) + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::Hover, move |_, ev| {
        if let Event::Hover { phase, location } = ev {
            // `None` on exit rather than a phase the caller has to match on: what a hover
            // handler wants to know is where the pointer IS, and "nowhere" is a state, not an
            // event kind. It is also what makes a handler that writes a
            // `Signal<Option<Point>>` a one-liner.
            f(match phase {
                day_spec::DragPhase::Ended => None,
                _ => Some(*location),
            });
        }
    })
}

fn op_on_pan(f: impl Fn(Pan) + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::Pan, move |n, ev| {
        if let Event::Pan {
            phase,
            delta,
            location,
        } = ev
        {
            let velocity = day_core::gesture::velocity(day_core::rnode_to_id(n), GestureKind::Pan);
            f(Pan::new(*phase, *delta, *location).with_velocity(velocity));
        }
    })
}

fn op_on_long_press(f: impl Fn(day_spec::Point) + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::LongPress, move |_, ev| {
        if let Event::LongPress(at) = ev {
            f(*at);
        }
    })
}

fn op_on_double_tap(f: impl Fn() + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::DoubleTap, move |_, ev| {
        if matches!(ev, Event::DoubleTap(_)) {
            f();
        }
    })
}

fn op_on_double_tap_at(f: impl Fn(day_spec::Point) + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::DoubleTap, move |_, ev| {
        if let Event::DoubleTap(at) = ev {
            f(*at);
        }
    })
}

fn op_on_pressed(f: impl Fn(bool) + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::Press, move |_, ev| {
        if let Event::Press { pressed, .. } = ev {
            f(*pressed);
        }
    })
}

fn op_on_touches(f: impl Fn(&[day_spec::Touch]) + 'static) -> impl FnOnce(Build) -> Build {
    op_gesture(GestureKind::Touches, move |_, ev| {
        if let Event::Touches(touches) = ev {
            f(touches);
        }
    })
}

fn op_hit_slop(slop: Insets) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.set_hit_slop(n, slop));
            n
        })
    }
}

fn op_interaction(state: InteractionState) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| {
                t.enable_gesture(n, GestureKind::Hover);
                t.enable_gesture(n, GestureKind::Press);
            });
            cx.on(n, move |ev| match ev {
                Event::Hover { phase, .. } => {
                    state.hovered.set(*phase != day_spec::DragPhase::Ended);
                }
                Event::Press { pressed, .. } => state.pressed.set(*pressed),
                Event::FocusChanged(focused) => state.focused.set(*focused),
                _ => {}
            });
            n
        })
    }
}

fn op_enabled(enabled: Reactive<bool>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            let apply = move |on: bool| {
                day_core::gesture::set_subtree_enabled(n, on);
                // The hover a disabled piece can no longer end is not left standing.
                if !on {
                    day_core::enqueue_event(
                        day_core::rnode_to_id(n),
                        Event::Hover {
                            phase: day_spec::DragPhase::Ended,
                            location: day_spec::Point::ZERO,
                        },
                    );
                }
            };
            // Only a reactive source needs a binding, and only a disabled start needs a call.
            if !enabled.get_untracked() {
                apply(false);
            }
            if let Reactive::Dyn(_) = &enabled {
                bind(move || enabled.get(), move |on: &bool| apply(*on));
            }
            n
        })
    }
}

/// The tracked interaction state of one piece: whether the pointer is over it, whether it is
/// pressed, and whether it has the keyboard (docs/gestures.md "State styling").
///
/// Create one, attach it with [`Decorate::interaction`], and read it from any reactive
/// modifier. Each read is tracked, so the modifier follows the state with no handler in between:
///
/// ```ignore
/// let state = InteractionState::new();
/// row((icon, title))
///     .interaction(state)
///     .background(move || if state.pressed() { PRESSED } else if state.hovered() { HOVER } else { REST })
///     .opacity(move || if state.pressed() { 0.7 } else { 1.0 })
/// ```
///
/// `hovered` follows the pointer where one exists and stays `false` on a touch-only device.
/// `pressed` is `true` from the pointer-down itself until the release, the pointer leaving, or
/// the platform taking the gesture away. `focused` follows keyboard focus on a piece that can
/// take it ([`Decorate::focusable`], or a control inside). All three read `false` while the
/// piece is disabled ([`Decorate::enabled`]).
#[derive(Clone, Copy)]
pub struct InteractionState {
    hovered: Signal<bool>,
    pressed: Signal<bool>,
    focused: Signal<bool>,
}

impl Default for InteractionState {
    fn default() -> Self {
        Self::new()
    }
}

impl InteractionState {
    /// A state with nothing hovered, pressed or focused, owned by the scope it is created in.
    #[must_use]
    pub fn new() -> Self {
        InteractionState {
            hovered: Signal::new(false),
            pressed: Signal::new(false),
            focused: Signal::new(false),
        }
    }
    /// Whether the pointer is over the piece. Tracked.
    #[must_use]
    pub fn hovered(&self) -> bool {
        self.hovered.get()
    }
    /// Whether the piece is pressed. Tracked.
    #[must_use]
    pub fn pressed(&self) -> bool {
        self.pressed.get()
    }
    /// Whether the piece has keyboard focus. Tracked.
    #[must_use]
    pub fn focused(&self) -> bool {
        self.focused.get()
    }
}

fn op_background(fill: Reactive<Paint>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let seed = fill.get_untracked();
            let node = cx.native(
                kinds::CONTAINER,
                &ContainerProps {
                    fill: Some(seed.clone()),
                    ..Default::default()
                },
                Rc::new(FillThrough),
                Flex::default(),
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            // Only a reactive source needs a binding; a constant fill is applied once at
            // realize, and an ink that follows the appearance is the toolkit's to follow.
            if let Reactive::Dyn(_) = &fill {
                day_reactive::bind_seeded(
                    seed,
                    move || fill.get(),
                    move |p: &Paint| {
                        with_tree(|t| {
                            t.patch(node, Box::new(ContainerPatch::Fill(Some(p.clone()))), false);
                        });
                    },
                );
            }
            node
        })
    }
}

/// A semantic surface around the content: the toolkit's material for `role`, clipped to
/// `radius` corners.
fn op_surface(role: day_spec::SurfaceRole, radius: f64) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let w = cx.native(
                kinds::CONTAINER,
                &ContainerProps {
                    corner_radius: radius,
                    clips: true,
                    role: Some(role),
                    ..Default::default()
                },
                Rc::new(FillThrough),
                Flex::default(),
                Boundary::No,
            );
            cx.under(w, |cx| {
                let _ = inner(cx);
            });
            w
        })
    }
}

/// A styled native surface behind a piece: fill, rounded corners, border and shadow on one
/// native view (docs/surfaces.md).
///
/// The single-purpose modifiers each wrap the piece in a view of their own, so a border added
/// after `.corner_radius(..)` is a square border around a rounded view. A surface is one view
/// that carries all of it, which is what lets the border follow the corners and the shadow take
/// their shape:
///
/// ```ignore
/// label("Sale")
///     .padding(Space::M)
///     .surface(
///         Surface::new()
///             .fill(LinearGradient::vertical(top, bottom))
///             .corner_radius(Radius::L)
///             .corners(Corners::TOP)
///             .border(SystemColor::Separator, 1.0),
///     )
/// ```
///
/// Every color is an [`Ink`], so a surface follows the appearance with no rebuild. A surface
/// with a corner radius clips its content to the rounded rectangle, as `.corner_radius(..)`
/// does. A surface with a shadow does not clip: the shadow lies outside the frame a clip would
/// cut to. Put a clipping `.corner_radius(..)` inside it when the content must be cut as well.
/// The border is drawn over the content.
#[derive(Clone, Debug, Default, PartialEq)]
#[must_use]
pub struct Surface {
    fill: Option<Paint>,
    radius: crate::Corner,
    corners: day_spec::Corners,
    border: Option<day_spec::Border>,
    shadow: Option<day_spec::Shadow>,
}

impl Surface {
    /// A surface with nothing on it yet: transparent, square, no border, no shadow.
    pub fn new() -> Self {
        Surface::default()
    }
    /// Fill with a [`Paint`]: a color, a [`SystemColor`](day_spec::SystemColor), an [`Ink`],
    /// or a linear, radial or conic gradient. A gradient's unit points resolve against the
    /// surface's frame and follow it as it resizes.
    pub fn fill(mut self, fill: impl Into<Paint>) -> Self {
        self.fill = Some(fill.into());
        self
    }
    /// Round the corners (all four, unless [`Self::corners`] says which): points, a
    /// [`Radius`](crate::Radius) step, or a frame-relative [`Corner`](crate::Corner) such as
    /// [`Corner::CAPSULE`](crate::Corner::CAPSULE).
    pub fn corner_radius(mut self, radius: impl Into<crate::Corner>) -> Self {
        self.radius = radius.into();
        self
    }
    /// Which corners the radius rounds; the others stay square.
    pub fn corners(mut self, corners: day_spec::Corners) -> Self {
        self.corners = corners;
        self
    }
    /// A line of `color`, `width` points wide, around the edge, inside the frame and over the
    /// content.
    pub fn border(mut self, color: impl Into<Ink>, width: f64) -> Self {
        self.border = (width > 0.0).then(|| day_spec::Border::new(color, width));
        self
    }
    /// A shadow of the surface's rounded rectangle, outside its frame. It takes no room in
    /// layout, so leave space around the piece for it, and it stops the surface clipping its
    /// content.
    pub fn shadow(mut self, shadow: day_spec::Shadow) -> Self {
        self.shadow = Some(shadow);
        self
    }

    /// The realize props in a frame of `size` (zero before the first layout, which only
    /// matters to a frame-relative radius).
    fn props(&self, size: day_spec::Size, clip: Clip) -> ContainerProps {
        let radius = match self.radius {
            crate::Corner::Fixed(points) => points.max(0.0),
            relative @ crate::Corner::Fraction(_) => relative.points(size),
        };
        let rounded = radius > 0.0 || self.radius.is_relative();
        ContainerProps {
            fill: self.fill.clone(),
            corner_radius: radius,
            corners: self.corners,
            border: self.border,
            shadow: self.shadow,
            clips: match clip {
                Clip::Always => true,
                Clip::WhenRounded => rounded && self.shadow.is_none(),
            },
            role: None,
        }
    }
}

/// When a surface clips its content.
#[derive(Clone, Copy)]
enum Clip {
    /// A rounded surface without a shadow clips: what `.surface(..)` does.
    WhenRounded,
    /// Always, square or not: what `.corner_radius(..)` does, so a radius that animates down
    /// to zero keeps the content inside the frame.
    Always,
}

/// The one surface op: realize the style, follow a reactive one with `ContainerPatch::Surface`,
/// and re-resolve a frame-relative radius as layout resizes the view. A constant surface with a
/// fixed radius binds nothing.
fn op_styled_surface(surface: Reactive<Surface>, clip: Clip) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let seed = surface.get_untracked();
            let size = Rc::new(Cell::new(day_spec::Size::ZERO));
            let node = cx.native(
                kinds::CONTAINER,
                &seed.props(size.get(), clip),
                Rc::new(FillThrough),
                Flex::default(),
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            let current = Rc::new(std::cell::RefCell::new(seed.clone()));
            let send = {
                let (size, current) = (size.clone(), current.clone());
                move || {
                    let props = current.borrow().props(size.get(), clip);
                    with_tree(|t| {
                        t.patch(
                            node,
                            Box::new(ContainerPatch::Surface(Box::new(props))),
                            false,
                        );
                    });
                }
            };
            let relative = match &surface {
                Reactive::Const(s) => s.radius.is_relative(),
                // A reactive surface may turn relative later; it watches its frame throughout.
                Reactive::Dyn(_) => true,
            };
            if relative {
                // Resolved in the layout pass that sizes the view, with its frame, so the first
                // frame the surface is shown at is already round.
                let (size, current) = (size.clone(), current.clone());
                day_core::patch_on_frame(
                    node,
                    Rc::new(move |new: day_spec::Size| {
                        if new.approx_eq(size.get(), 0.25) {
                            return None;
                        }
                        size.set(new);
                        let surface = current.try_borrow().ok()?;
                        surface.radius.is_relative().then(|| {
                            Box::new(ContainerPatch::Surface(Box::new(surface.props(new, clip))))
                                as Box<dyn std::any::Any>
                        })
                    }),
                );
                day_reactive::Scope::current()
                    .on_cleanup(move || day_core::stop_frame_patches(node));
            }
            if let Reactive::Dyn(_) = &surface {
                day_reactive::bind_seeded(
                    seed,
                    move || surface.get(),
                    move |s: &Surface| {
                        *current.borrow_mut() = s.clone();
                        send();
                    },
                );
            }
            node
        })
    }
}

/// Combine reactive parts into one reactive value: constant when every part is.
fn reactive_map2<A: Clone + 'static, B: Clone + 'static, T: Clone + 'static>(
    a: Reactive<A>,
    b: Reactive<B>,
    f: impl Fn(A, B) -> T + 'static,
) -> Reactive<T> {
    match (&a, &b) {
        (Reactive::Const(x), Reactive::Const(y)) => Reactive::Const(f(x.clone(), y.clone())),
        _ => Reactive::Dyn(Rc::new(move || f(a.get(), b.get()))),
    }
}

/// A reactive value mapped: constant when the source is.
fn reactive_map<A: Clone + 'static, T: Clone + 'static>(
    a: Reactive<A>,
    f: impl Fn(A) -> T + 'static,
) -> Reactive<T> {
    match &a {
        Reactive::Const(x) => Reactive::Const(f(x.clone())),
        Reactive::Dyn(_) => Reactive::Dyn(Rc::new(move || f(a.get()))),
    }
}

fn op_opacity(op: Reactive<f64>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let node = layer_node(cx);
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            bind(
                move || op.get(),
                move |v: &f64| with_tree(|t| t.set_node_opacity(node, *v)),
            );
            node
        })
    }
}

fn op_transform(t: Reactive<Transform>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let node = layer_node(cx);
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            bind(
                move || t.get(),
                move |v: &Transform| with_tree(|tr| tr.set_node_transform(node, *v)),
            );
            node
        })
    }
}

fn op_animation(anim: AnimSpec) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            // Layout-only: the implicit animation is Day's own state, which the patches beneath
            // look up through their ancestors, and needs no native view. A native layer here
            // also clipped what it held on the toolkits whose containers clip their children
            // (Qt always, Android by default), so a translated or scaled view under
            // `.animation(..)` vanished past the layer's frame.
            let node = cx.layout_only(Rc::new(FillThrough), Flex::default(), Boundary::No);
            with_tree(|t| t.set_implicit_anim(node, Some(anim)));
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            node
        })
    }
}

fn op_overlay_aligned(align: Alignment, over: impl Piece) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
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
                let _ = inner(cx); // sizing content (bottom)
                let _ = over.build(cx); // annotation on top
            });
            node
        })
    }
}

fn op_aspect_ratio(ratio: Reactive<f64>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            if let Reactive::Const(r) = &ratio
                && !(r.is_finite() && *r > 0.0)
            {
                return inner(cx);
            }
            let layout = Rc::new(AspectRatioLayout {
                ratio: Cell::new(ratio.get_untracked()),
            });
            let node = cx.layout_only(
                layout.clone(),
                Flex::default(),
                // Not a boundary: the child still measures itself, and the ratio only decides
                // the box it is offered.
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            bind_length(ratio, move |v| layout.ratio.set(v), node);
            node
        })
    }
}

fn op_aligned(align: Alignment) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let node = cx.layout_only(
                Rc::new(AlignmentLayout { align }),
                Flex::default(),
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            node
        })
    }
}

fn op_grow(grow: Grow) -> impl FnOnce(Build) -> Build {
    let w = matches!(grow.dimension, Dimension::Width | Dimension::Both);
    let h = matches!(grow.dimension, Dimension::Height | Dimension::Both);
    move |inner| {
        Box::new(move |cx| {
            let node = cx.layout_only(
                Rc::new(GrowLayout { w, h }),
                Flex {
                    grow_w: w,
                    grow_h: h,
                    grow_weight: grow.weight,
                    ..Default::default()
                },
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            node
        })
    }
}

fn op_grid_facts(facts: GridFacts) -> impl FnOnce(RNode) {
    move |node| with_tree(|t| t.set_grid_facts(node, facts))
}

fn op_defers_system_gestures(edges: day_spec::Edges) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let token = day_core::shield::push_gesture_deferral(edges);
            Scope::current().on_cleanup(move || day_core::shield::pop_gesture_deferral(token));
            inner(cx)
        })
    }
}

fn op_status_bar_hidden(hidden: Reactive<bool>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let token = day_core::shield::push_status_bar_request(hidden.get_untracked());
            Scope::current().on_cleanup(move || day_core::shield::pop_status_bar_request(token));
            // A constant answer is registered once; a reactive one follows its source while
            // the subtree stays mounted (a reader's "hide status bar" setting).
            if let Reactive::Dyn(_) = &hidden {
                bind(
                    move || hidden.get(),
                    move |h: &bool| day_core::shield::set_status_bar_request(token, *h),
                );
            }
            inner(cx)
        })
    }
}

fn op_on_system_back(
    enabled: Reactive<bool>,
    handler: Rc<dyn Fn()>,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            crate::nav::install_back_handler(cx.parent(), enabled, handler);
            inner(cx)
        })
    }
}

fn op_interactive_dismiss_disabled() -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let token = day_core::shield::push_dismiss_disabled();
            Scope::current().on_cleanup(move || day_core::shield::pop_dismiss_disabled(token));
            inner(cx)
        })
    }
}

// --- The chained modifier surface, on an already-decorated piece ---
//
// These inherent methods shadow the `Decorate` trait's (inherent wins method resolution), so a
// modifier applied to a `Decorated` appends to its op list instead of wrapping it in another
// `Decorated`. Each is the same one-liner the trait method is; the bodies live in the `op_*`
// functions above. Documentation stays on the trait, which is the surface every piece has.
impl<P: Piece> Decorated<P> {
    pub fn id(self, id: day_core::ElementId) -> Self {
        self.push_node(op_id(id))
    }
    pub fn id_of(self, id: impl Fn() -> day_core::ElementId + 'static) -> Self {
        self.push_node(op_id_of(id))
    }
    pub fn wrapper_id(self, id: day_core::ElementId) -> Self {
        self.push(op_wrapper_id(id))
    }
    pub fn tweak(self, f: impl FnOnce(day_core::RNode) + 'static) -> Self {
        self.push(op_tweak(f))
    }
    pub fn selectable(self) -> Self {
        self.push(op_selectable())
    }
    /// See [`Decorate::tooltip`].
    pub fn tooltip<M>(self, text: impl IntoText<M>) -> Self {
        let text = text.into_text();
        self.push_node(move |n| crate::context::install_tooltip(n, text))
    }

    /// See [`Decorate::popover`].
    pub fn popover<C: Piece>(
        self,
        open: impl Binding<bool>,
        content: impl Fn() -> C + 'static,
    ) -> Self {
        self.push_node(move |n| crate::context::install_popover(n, open, content))
    }

    pub fn cursor<M>(self, cursor: impl IntoReactive<Cursor, M>) -> Self {
        self.push(op_cursor(cursor.into_reactive()))
    }
    pub fn layout_direction<M>(
        self,
        direction: impl IntoReactive<day_spec::LayoutDirection, M>,
    ) -> Self {
        self.push(op_layout_direction(direction.into_reactive()))
    }
    pub fn native_ref(self, r: &NativeRef) -> Self {
        self.push(op_native_ref(r.clone()))
    }
    pub fn window_drag_region(self) -> Self {
        self.push(op_window_drag_region())
    }
    pub fn padding<M>(self, insets: impl IntoReactive<Insets, M>) -> Self {
        self.push(op_padding(insets.into_reactive()))
    }
    pub fn max_width<M>(self, max: impl IntoReactive<f64, M>) -> Self {
        self.push(op_max_width(max.into_reactive()))
    }
    /// Never taller than `max`: the child is never proposed more height, so a scroll stops
    /// growing at `max` and still shrinks where the screen is shorter. In a column it is sized
    /// after its siblings and takes what they leave, up to `max`, so the controls around a
    /// capped scroll stay on screen.
    pub fn max_height<M>(self, max: impl IntoReactive<f64, M>) -> Self {
        self.push(op_max_height(max.into_reactive()))
    }
    /// Never narrower than `min`: a stretching control keeps a usable width, and a row that
    /// cannot give it that overflows, so a `labeled` row stacks the control under its label
    /// rather than squeezing it (docs/forms.md).
    pub fn min_width<M>(self, min: impl IntoReactive<f64, M>) -> Self {
        self.push(op_min_width(min.into_reactive()))
    }
    pub fn reserving<M>(self, sample: impl IntoText<M>) -> Self {
        self.push(op_reserving(sample.into_text()))
    }
    /// Set both dimensions from constants, signals, or reactive closures.
    /// Negative and non-finite sizes become zero. Updates preserve mounted content.
    pub fn frame<Mw, Mh>(
        self,
        width: impl IntoReactive<f64, Mw>,
        height: impl IntoReactive<f64, Mh>,
    ) -> Self {
        self.push(op_frame(
            Some(width.into_reactive()),
            Some(height.into_reactive()),
        ))
    }
    /// Set a constant or reactive width, leaving height flexible.
    pub fn width<M>(self, width: impl IntoReactive<f64, M>) -> Self {
        self.push(op_frame(Some(width.into_reactive()), None))
    }
    /// Set a constant or reactive height, leaving width flexible.
    pub fn height<M>(self, height: impl IntoReactive<f64, M>) -> Self {
        self.push(op_frame(None, Some(height.into_reactive())))
    }
    /// Keep at least `min` points of height, expanding for taller content.
    pub fn min_height<M>(self, min: impl IntoReactive<f64, M>) -> Self {
        self.push(op_min_height(min.into_reactive()))
    }
    pub fn a11y(self, f: impl FnOnce(A11yBuilder) -> A11yBuilder + 'static) -> Self {
        self.push(op_a11y(f))
    }
    /// Handle a tap gesture on this piece.
    ///
    /// Do not use this to activate a native button; use [`crate::ButtonBuilder::action`].
    /// A separate gesture recognizer can delay pressed feedback and bypass native keyboard,
    /// accessibility, and disabled behavior. Day warns when it attaches a tap to a button.
    pub fn on_tap(self, f: impl Fn() + 'static) -> Self {
        self.push(op_on_tap(f))
    }
    /// Handle a tap with its local position. Like [`Self::on_tap`], this is a gesture,
    /// not native button activation; use [`crate::ButtonBuilder::action`] for buttons.
    pub fn on_tap_at(self, f: impl Fn(day_spec::Point) + 'static) -> Self {
        self.push(op_on_tap_at(f))
    }
    pub fn focused<M>(self, binding: impl IntoFocusBinding<M>) -> Self {
        let (want, on_native) = binding.into_focus_binding();
        self.push(op_focused(want, on_native))
    }
    pub fn on_key(self, f: impl Fn(&day_spec::KeyEvent) + 'static) -> Self {
        self.push(op_on_key(false, f))
    }
    pub fn on_key_up(self, f: impl Fn(&day_spec::KeyEvent) + 'static) -> Self {
        self.push(op_on_key(true, f))
    }
    pub fn focusable(self) -> Self {
        self.push(op_focusable())
    }
    pub fn keyboard(self) -> Self {
        self.push(op_keyboard())
    }
    pub fn toolbar<M>(self, content: impl crate::ToolbarContent<M>) -> Self {
        self.push(op_toolbar(content.into_source()))
    }
    /// See [`Decorate::searchable`].
    pub fn searchable(self, search: crate::Search) -> Self {
        self.push(move |inner| {
            Box::new(move |cx| {
                let n = inner(cx);
                crate::nav::install_page_search(search);
                n
            })
        })
    }
    /// Offer data through the platform's native drag session.
    pub fn drag_source(
        self,
        source: impl Fn(day_spec::Point) -> Option<day_spec::transfer::Offer> + 'static,
    ) -> Self {
        self.push(move |inner| {
            Box::new(move |cx| {
                let n = layer_node(cx);
                cx.under(n, |cx| {
                    inner(cx);
                });
                let source = scoped_drag_source(Rc::new(source));
                with_tree(|t| t.set_drag_source(n, source));
                n
            })
        })
    }
    /// Accept drops through the platform's native drag session (docs/drag-and-drop.md).
    pub fn drop_target(self, target: day_spec::transfer::Target) -> Self {
        self.push(move |inner| {
            Box::new(move |cx| {
                let n = layer_node(cx);
                cx.under(n, |cx| {
                    inner(cx);
                });
                let target = scoped_drop_target(target);
                with_tree(|t| t.set_drop_target(n, target));
                n
            })
        })
    }
    pub fn context_menu<M>(self, menu: impl ContextMenuContent<M>) -> Self {
        match menu.into_context_menu() {
            ContextMenuSource::Fixed(items) => self.push(op_context_menu(items)),
            ContextMenuSource::OnSummon(f) => self.push(op_context_menu_fn(f)),
        }
    }
    pub fn on_drag(self, f: impl Fn(Drag) + 'static) -> Self {
        self.push(op_on_drag(None, f))
    }
    pub fn on_drag_along(self, axis: day_spec::DragAxis, f: impl Fn(Drag) + 'static) -> Self {
        self.push(op_on_drag(Some(axis), f))
    }
    pub fn on_long_press(self, f: impl Fn(day_spec::Point) + 'static) -> Self {
        self.push(op_on_long_press(f))
    }
    pub fn on_double_tap(self, f: impl Fn() + 'static) -> Self {
        self.push(op_on_double_tap(f))
    }
    /// See [`Decorate::on_double_tap_at`].
    pub fn on_double_tap_at(self, f: impl Fn(day_spec::Point) + 'static) -> Self {
        self.push(op_on_double_tap_at(f))
    }
    pub fn on_pressed(self, f: impl Fn(bool) + 'static) -> Self {
        self.push(op_on_pressed(f))
    }
    pub fn on_touches(self, f: impl Fn(&[day_spec::Touch]) + 'static) -> Self {
        self.push(op_on_touches(f))
    }
    pub fn hit_slop(self, slop: impl Into<Insets>) -> Self {
        self.push(op_hit_slop(slop.into()))
    }
    pub fn interaction(self, state: InteractionState) -> Self {
        self.push(op_interaction(state))
    }
    pub fn enabled<M>(self, enabled: impl IntoReactive<bool, M>) -> Self {
        self.push(op_enabled(enabled.into_reactive()))
    }
    pub fn on_files_dropped(self, f: impl Fn(Vec<std::path::PathBuf>) + 'static) -> Self {
        self.drop_target(day_spec::transfer::Target::files().on_drop(move |drop| {
            let paths = drop.file_paths();
            let any = !paths.is_empty();
            if any {
                f(paths);
            }
            any
        }))
    }
    pub fn on_pinch(self, f: impl Fn(Pinch) + 'static) -> Self {
        self.push(op_on_pinch(f))
    }
    pub fn on_rotate(self, f: impl Fn(Rotate) + 'static) -> Self {
        self.push(op_on_rotate(f))
    }
    pub fn on_pan(self, f: impl Fn(Pan) + 'static) -> Self {
        self.push(op_on_pan(f))
    }
    /// The pointer moving over this piece: `Some(point)` in its own coordinates while inside,
    /// `None` when it leaves (docs/canvas.md "Interaction").
    ///
    /// Pointer-only, which is not a gap: every desktop delivers it, an
    /// iPad does with a trackpad or pencil, an Android device with a mouse or stylus, and a
    /// touch-only phone delivers nothing at all. **Anything reachable by hover must also be
    /// reachable by a tap**: wire `on_tap_at` alongside it, exactly as a chart's selection does.
    pub fn on_hover(self, f: impl Fn(Option<day_spec::Point>) + 'static) -> Self {
        self.push(op_on_hover(f))
    }
    /// The platform's card: the grouped-content surface (`SurfaceRole::SectionCard`) with
    /// rounded corners and a `Space::M` inset. Drawn by the toolkit, so it follows the
    /// appearance and never steals hit testing from the content.
    pub fn card(self) -> Self {
        self.padding(crate::Space::M).push(op_surface(
            day_spec::SurfaceRole::SectionCard,
            crate::Radius::M.points(),
        ))
    }
    pub fn background<M>(self, fill: impl IntoReactive<Paint, M>) -> Self {
        self.push(op_background(fill.into_reactive()))
    }
    pub fn corner_radius<M>(self, radius: impl IntoReactive<crate::Corner, M>) -> Self {
        let surface = reactive_map(radius.into_reactive(), |r| Surface::new().corner_radius(r));
        self.push(op_styled_surface(surface, Clip::Always))
    }
    pub fn surface<M>(self, surface: impl IntoReactive<Surface, M>) -> Self {
        self.push(op_styled_surface(
            surface.into_reactive(),
            Clip::WhenRounded,
        ))
    }
    pub fn border<Mc, Mw>(
        self,
        color: impl IntoReactive<Ink, Mc>,
        width: impl IntoReactive<f64, Mw>,
    ) -> Self {
        let surface = reactive_map2(color.into_reactive(), width.into_reactive(), |c, w| {
            Surface::new().border(c, w)
        });
        self.push(op_styled_surface(surface, Clip::WhenRounded))
    }
    pub fn shadow<M>(self, shadow: impl IntoReactive<day_spec::Shadow, M>) -> Self {
        let surface = reactive_map(shadow.into_reactive(), |s| Surface::new().shadow(s));
        self.push(op_styled_surface(surface, Clip::WhenRounded))
    }
    pub fn opacity<M>(self, opacity: impl IntoReactive<f64, M>) -> Self {
        self.push(op_opacity(opacity.into_reactive()))
    }
    pub fn transform<M>(self, t: impl IntoReactive<Transform, M>) -> Self {
        self.push(op_transform(t.into_reactive()))
    }
    pub fn scale<M>(self, factor: impl IntoReactive<f64, M>) -> Self {
        let f = factor.into_reactive();
        self.transform(move || Transform::scale(f.get(), f.get()))
    }
    pub fn rotation<M>(self, angle: impl IntoReactive<Angle, M>) -> Self {
        let a = angle.into_reactive();
        self.transform(move || Transform::rotate(a.get()))
    }
    pub fn translation<Mx, My>(
        self,
        x: impl IntoReactive<f64, Mx>,
        y: impl IntoReactive<f64, My>,
    ) -> Self {
        let (x, y) = (x.into_reactive(), y.into_reactive());
        self.transform(move || Transform::translate(x.get(), y.get()))
    }
    pub fn animation(self, anim: AnimSpec) -> Self {
        self.push(op_animation(anim))
    }
    /// Erases, like [`Decorate::modifier`]: `Modifier` is defined over [`AnyPiece`].
    pub fn modifier(self, m: impl Modifier) -> AnyPiece {
        m.apply(self.any())
    }
    pub fn overlay(self, over: impl Piece) -> Self {
        self.overlay_aligned(Alignment::Center, over)
    }
    pub fn overlay_aligned(self, align: Alignment, over: impl Piece) -> Self {
        self.push(op_overlay_aligned(align, over))
    }
    pub fn aspect_ratio<M>(self, ratio: impl IntoReactive<f64, M>) -> Self {
        self.push(op_aspect_ratio(ratio.into_reactive()))
    }
    /// Position this piece within its assigned space; see [`Decorate::aligned`].
    pub fn aligned(self, alignment: Alignment) -> Self {
        self.push(op_aligned(alignment))
    }
    /// Expand in the selected dimensions; see [`Decorate::grow`].
    pub fn grow(self, grow: impl Into<Grow>) -> Self {
        self.push(op_grow(grow.into()))
    }
    #[deprecated(note = "use .grow(Dimension::Width)")]
    pub fn grow_w(self) -> Self {
        self.grow(Dimension::Width)
    }
    #[deprecated(note = "use .grow(Dimension::Height)")]
    pub fn grow_h(self) -> Self {
        self.grow(Dimension::Height)
    }
    pub fn grid_span(self, n: usize) -> Self {
        self.push_outer(op_grid_facts(GridFacts {
            col_span: n.clamp(1, u16::MAX as usize) as u16,
            ..Default::default()
        }))
    }
    pub fn grid_align(self, a: Alignment) -> Self {
        self.push_outer(op_grid_facts(GridFacts {
            align: Some(a),
            ..Default::default()
        }))
    }
    pub fn defers_system_gestures(self, edges: day_spec::Edges) -> Self {
        self.push(op_defers_system_gestures(edges))
    }
    pub fn interactive_dismiss_disabled(self) -> Self {
        self.push(op_interactive_dismiss_disabled())
    }
    pub fn on_system_back<M>(
        self,
        enabled: impl IntoReactive<bool, M>,
        handler: impl Fn() + 'static,
    ) -> Self {
        self.push(op_on_system_back(enabled.into_reactive(), Rc::new(handler)))
    }
    pub fn status_bar_hidden<M>(self, hidden: impl IntoReactive<bool, M>) -> Self {
        self.push(op_status_bar_hidden(hidden.into_reactive()))
    }
    /// Erase to a single [`AnyPiece`].
    pub fn any(self) -> AnyPiece {
        AnyPiece::new(self)
    }
}

/// Where a context menu's entries come from (see [`Decorate::context_menu`]).
pub enum ContextMenuSource {
    Fixed(Vec<MenuEntry>),
    OnSummon(Rc<dyn Fn(day_spec::Point) -> Vec<MenuEntry>>),
}

/// Anything [`Decorate::context_menu`] accepts: a `Vec` or array of entries, a closure building
/// them on summon, or a closure given the summon point. The marker parameter is the same E0119
/// dodge as [`IntoText`].
pub trait ContextMenuContent<M> {
    fn into_context_menu(self) -> ContextMenuSource;
}

/// Marker for a summon-time closure that takes the summon point.
pub struct AtPointMark;

impl ContextMenuContent<crate::ManyMark> for Vec<MenuEntry> {
    fn into_context_menu(self) -> ContextMenuSource {
        ContextMenuSource::Fixed(self)
    }
}

impl<const N: usize> ContextMenuContent<crate::ManyMark> for [MenuEntry; N] {
    fn into_context_menu(self) -> ContextMenuSource {
        ContextMenuSource::Fixed(Vec::from(self))
    }
}

impl<C, F> ContextMenuContent<crate::DerivedMark> for F
where
    C: IntoIterator<Item = MenuEntry>,
    F: Fn() -> C + 'static,
{
    fn into_context_menu(self) -> ContextMenuSource {
        ContextMenuSource::OnSummon(Rc::new(move |_| self().into_iter().collect()))
    }
}

impl<C, F> ContextMenuContent<AtPointMark> for F
where
    C: IntoIterator<Item = MenuEntry>,
    F: Fn(day_spec::Point) -> C + 'static,
{
    fn into_context_menu(self) -> ContextMenuSource {
        ContextMenuSource::OnSummon(Rc::new(move |p| self(p).into_iter().collect()))
    }
}

pub trait Decorate: Piece + Sized {
    /// Attach localized, reactive help to the piece. Empty text clears the tooltip.
    fn tooltip<M>(self, text: impl IntoText<M>) -> Decorated<Self> {
        Decorated::new(self).tooltip(text)
    }
    /// Present Day content beside this piece while `open` is true. Outside dismissal
    /// writes false. Each opening builds a fresh subtree in the trigger's scope.
    fn popover<P: Piece>(
        self,
        open: impl Binding<bool>,
        content: impl Fn() -> P + 'static,
    ) -> Decorated<Self> {
        Decorated::new(self).popover(open, content)
    }

    /// Stable element identifier: a11y identifier + dayscript locator + lint uniqueness (§5.5).
    ///
    /// Names the piece's own node wherever it sits in the chain (docs/api-style.md "Modifier
    /// classes"): `.padding(8.0).id(day_core::id!(x))` and `.id(day_core::id!(x)).padding(8.0)` tag the same node. To name
    /// a wrapper instead, use [`wrapper_id`](Self::wrapper_id).
    fn id(self, id: day_core::ElementId) -> Decorated<Self> {
        Decorated::new(self).id(id)
    }

    /// Name the node that is outermost at this point in the chain: the padding box, the frame,
    /// the background, whichever wrapper the modifiers before it built. For a dayscript frame
    /// assertion on the wrapper rather than on the piece. Ordered, unlike [`id`](Self::id).
    fn wrapper_id(self, id: day_core::ElementId) -> Decorated<Self> {
        Decorated::new(self).wrapper_id(id)
    }

    /// Apply `f` to the chain when `cond` holds, else continue unchanged
    /// (see [`Decorated::apply_if`]): `nav(sel).apply_if(!primary, |n| n.local())`. The
    /// closure receives the decorated piece, so forwarded builder methods and modifiers both
    /// chain inside it.
    fn apply_if(
        self,
        cond: bool,
        f: impl FnOnce(Decorated<Self>) -> Decorated<Self>,
    ) -> Decorated<Self> {
        Decorated::new(self).apply_if(cond, f)
    }

    /// Apply `f` with the value when there is one, else continue unchanged
    /// (see [`Decorated::apply_some`]): `canvas(draw).apply_some(id, |c, id| c.id(id))`.
    fn apply_some<T>(
        self,
        value: Option<T>,
        f: impl FnOnce(Decorated<Self>, T) -> Decorated<Self>,
    ) -> Decorated<Self> {
        Decorated::new(self).apply_some(value, f)
    }

    /// Reactive element id: the id for rows inside a recycling [`list`](crate::list). A plain
    /// [`id`](Self::id) is assigned once at build, but a recycled cell rebinds to different
    /// items over its life (and drag-to-reorder rebinds eagerly), so a static item-derived id
    /// keeps naming the first-bound item. This variant re-registers whenever the closure's
    /// value changes; read your `ItemSlot` inside it:
    /// `.id_of(move || day_core::id!(row_remove, slot.key()))`.
    fn id_of(self, id: impl Fn() -> day_core::ElementId + 'static) -> Decorated<Self> {
        Decorated::new(self).id_of(id)
    }

    /// Apply a **tweak**: `f` runs once at mount, after the native widget exists, with the
    /// realized node (docs/tweaks.md). Reach the typed native handle through the compiled
    /// backend's ext accessor (`day_toolkit_appkit::with_native`, `day_toolkit_gtk::with_native`, …), or apply
    /// a packaged `day-tweak-*` crate's modifier instead of calling this directly. If the native
    /// change affects the widget's intrinsic size, follow it with
    /// [`day_core::invalidate_size`]. Day may overwrite *managed* properties (title, value,
    /// enabled, frame, a11y) on its next patch; unmanaged properties are stable.
    ///
    /// Order it after any modifier that can rebuild the backing widget; today that is
    /// [`selectable`](Decorate::selectable), which on UIKit realizes the label as a different
    /// native class. Chained before it, the tweak runs against the widget the rebuild discards
    /// (Day warns at runtime); chained after, it sees the widget that ships.
    fn tweak(self, f: impl FnOnce(day_core::RNode) + 'static) -> Decorated<Self> {
        Decorated::new(self).tweak(f)
    }

    /// Make this piece's text **user-selectable**, so the reader can select and copy it
    /// (docs/text.md). Most useful on a [`label`](crate::label): text is not selectable by default
    /// on any backend, matching each platform's native behavior.
    ///
    /// Every backend honors it on a label: most flip the native widget's selection affordance
    /// (AppKit, GTK, Qt, XAML, HarmonyOS, Android, web); UIKit, whose `UILabel` has none,
    /// rebuilds the label as a read-only `UITextView` behind the same handle. On other widgets
    /// it is best-effort: a backing with no selection affordance leaves the text unselectable
    /// rather than erroring, and a container cascades only where the platform's affordance does
    /// (the web), so prefer the label itself. Selection visuals and the copy shortcut are the
    /// platform's own. Unmanaged: set once at mount, and it survives Day's text updates.
    fn selectable(self) -> Decorated<Self> {
        Decorated::new(self).selectable()
    }

    /// Shape the pointer while it is over this piece and its descendants (docs/cursor.md): a
    /// [`Cursor`] constant, a `Signal<Cursor>`, or a closure, so a canvas tool or a busy state
    /// moves the shape without rebuilding the piece. The nearest ancestor's cursor wins, and
    /// `Cursor::Default` releases the piece back to the platform. `capability(Cap::Cursor)`
    /// says whether this toolkit draws the requested shapes, a nearest neighbor, or nothing;
    /// a touch-only host never shows one, which is correct rather than a failure.
    fn cursor<M>(self, cursor: impl IntoReactive<Cursor, M>) -> Decorated<Self> {
        Decorated::new(self).cursor(cursor)
    }

    /// Lay this piece and everything inside it out in one reading direction, whatever the
    /// app's is (docs/localization.md "Direction of a subtree"): `LayoutDirection::Ltr` for a
    /// phone number, a code sample or a media transport inside a right-to-left app,
    /// `LayoutDirection::Rtl` for a preview of Arabic content inside a left-to-right one.
    ///
    /// Day mirrors placement itself: inside the subtree leading means the direction's start
    /// edge, so rows run the other way, leading and trailing insets swap sides and
    /// alignments flip. Each native widget in the subtree is given the direction too, so text
    /// takes that base writing direction and controls draw their own insides to match. The
    /// piece keeps its place in its parent, which follows the parent's direction. A nested
    /// `.layout_direction` overrides an outer one. A signal or closure re-lays the subtree out
    /// when it changes.
    ///
    /// A canvas draws in its own coordinates and is not mirrored; `day::is_rtl()` answers for
    /// the app, not for a subtree.
    fn layout_direction<M>(
        self,
        direction: impl IntoReactive<day_spec::LayoutDirection, M>,
    ) -> Decorated<Self> {
        Decorated::new(self).layout_direction(direction)
    }

    /// Let the user move the window by dragging this piece, as a title bar does
    /// (docs/window-chrome.md): what a custom title bar in a [`WindowChrome::Overlay`] or
    /// [`WindowChrome::Frameless`] window needs. The piece's own background starts the drag,
    /// with the platform's snapping and tiling; buttons and fields inside it keep their clicks;
    /// a double click does what the user's title-bar setting says (zoom, minimize, nothing).
    /// `capability(Cap::DragRegion)`; on a phone the piece is an ordinary view.
    ///
    /// [`WindowChrome::Overlay`]: day_spec::WindowChrome::Overlay
    /// [`WindowChrome::Frameless`]: day_spec::WindowChrome::Frameless
    fn window_drag_region(self) -> Decorated<Self> {
        Decorated::new(self).window_drag_region()
    }

    /// Capture a [`NativeRef`] to this piece's realized node for later imperative access
    /// (docs/tweaks.md). The ref clears automatically when the piece's scope is disposed.
    fn native_ref(self, r: &NativeRef) -> Decorated<Self> {
        Decorated::new(self).native_ref(r)
    }

    /// Space between the piece and its frame: points on all four sides, a
    /// [`Space`](crate::Space) step, an [`Insets`], or a signal or closure of any of them. A
    /// reactive inset re-pads in place, so `.padding(move || day::safe_area())` follows a
    /// rotation or a title bar that grows (docs/window-chrome.md "Safe area").
    fn padding<M>(self, insets: impl IntoReactive<Insets, M>) -> Decorated<Self> {
        Decorated::new(self).padding(insets)
    }

    /// Cap this piece's width at `max` points: the child is never PROPOSED more, so text
    /// wraps inside the cap (chat bubbles, readable columns) while narrower content hugs.
    /// Takes points, a [`Space`](crate::Space) step, a signal or a closure; a change re-lays
    /// the piece out without rebuilding it.
    fn max_width<M>(self, max: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).max_width(max)
    }
    /// Cap this piece's height at `max` points, the vertical twin of [`Decorate::max_width`]: a
    /// scroll or another piece that takes the height it is offered stops at `max`, and still
    /// gets less where its parent has less to give. Reactive like [`Decorate::max_width`].
    fn max_height<M>(self, max: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).max_height(max)
    }
    /// Never narrower than `min` points: a stretching control keeps a usable width, and a row
    /// that cannot give it that overflows, so a `labeled` row stacks the control under its
    /// label rather than squeezing it (docs/forms.md). Reactive like [`Decorate::max_width`].
    fn min_width<M>(self, min: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).min_width(min)
    }

    /// Keep at least `min` points of height, expanding for taller content. Reactive like
    /// [`Decorate::max_width`].
    fn min_height<M>(self, min: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).min_height(min)
    }

    /// Reserve at least the space `sample` needs, so this piece's size stops changing with its
    /// content.
    ///
    /// For a numeric readout beside a slider: `label(move || value()).reserving("100")` keeps the
    /// row still while the number changes, because the reservation is a real measurement of
    /// `"100"` in this piece's own font: it scales with the platform's accessibility text size
    /// instead of being a point value that clips when someone turns text up.
    ///
    /// Pass the widest value the field can show (`"100"`, `"-99.9"`, `"88:88"`). Pair it with
    /// tabular numbers so the digits themselves stop shifting inside the reservation.
    /// Localized text, signals and closures update the reservation without rebuilding the label.
    /// Single-child wrappers preserve the label's font; other pieces use the default Body font.
    /// The sample is placed at zero size and hidden from assistive technology.
    fn reserving<M>(self, sample: impl IntoText<M>) -> Decorated<Self> {
        Decorated::new(self).reserving(sample)
    }

    fn frame<Mw, Mh>(
        self,
        width: impl IntoReactive<f64, Mw>,
        height: impl IntoReactive<f64, Mh>,
    ) -> Decorated<Self> {
        Decorated::new(self).frame(width, height)
    }

    /// Fix this piece's width to `width` points while its height stays flexible (hugging its
    /// content or filling on the cross axis). The single-axis complement to [`Self::frame`], e.g.
    /// a fixed-width sidebar pane in a `row` whose height fills the window.
    fn width<M>(self, width: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).width(width)
    }

    /// Fix this piece's height to `height` points while its width stays flexible. The single-axis
    /// complement to [`Self::frame`], e.g. a fixed-height header/toolbar bar that fills its width.
    fn height<M>(self, height: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).height(height)
    }

    fn a11y(self, f: impl FnOnce(A11yBuilder) -> A11yBuilder + 'static) -> Decorated<Self> {
        Decorated::new(self).a11y(f)
    }

    /// Fire when this piece is tapped (bounding-box; shapes override with path-precise testing).
    ///
    /// Do not use this to activate a native button; use [`crate::Button::action`] instead.
    /// A separate gesture recognizer can delay pressed feedback and bypass native keyboard,
    /// accessibility, and disabled behavior. Day warns when it attaches a tap to a button.
    fn on_tap(self, f: impl Fn() + 'static) -> Decorated<Self> {
        Decorated::new(self).on_tap(f)
    }

    /// [`on_tap`](Self::on_tap), told where: the point in the piece's own coordinate space,
    /// origin at its top-leading corner.
    ///
    /// What a drawn control needs and a native one does not: a canvas showing a color wheel, a
    /// map, or a waveform has to turn "the user pressed here" into a value, and only the piece
    /// knows how. Pair it with [`on_drag`](Self::on_drag), which already reports a location, to
    /// track a press that turns into a drag; the two are idempotent together, so a backend that
    /// reports a tap as a zero-length drag costs nothing.
    ///
    /// `Event::Tap` has always carried the point; this is the decorator that stops throwing it
    /// away.
    ///
    /// For native buttons use [`crate::Button::action`], not this gesture decorator.
    fn on_tap_at(self, f: impl Fn(day_spec::Point) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_tap_at(f)
    }

    /// Bind this control's keyboard focus to a signal (docs/focus.md), two-way like every other
    /// binding: native focus changes write the signal; writing the signal moves focus. Takes a
    /// `Signal<bool>` for one control, or `(Signal<Option<K>>, K::Variant)` binding one control
    /// of a group; writing `false`/`None` resigns focus (dismissing the soft keyboard on
    /// mobile). Focus applies asynchronously: a write is a request, resolved on the next turn,
    /// and the signal always ends up reflecting what the platform actually did.
    fn focused<M>(self, binding: impl IntoFocusBinding<M>) -> Decorated<Self> {
        Decorated::new(self).focused(binding)
    }

    /// Handle the non-text keys (the arrows) that reach this piece while it has focus
    /// (docs/menus.md). Keys follow focus, so pair it with [`Decorate::focused`] or with a
    /// piece the user can click into: a canvas takes focus on a press, and only the focused
    /// piece hears the keys, so a nudge handler can never fire while a text field, a list or a
    /// sidebar is the one being typed into.
    ///
    /// Which pieces can hold focus is the platform's to answer (docs/focus.md): a `canvas`
    /// is focusable on the backends that draw one from a real view (appkit, web-dom today).
    ///
    /// The handler hears each press and each repeat while the key is held
    /// ([`KeyEvent::is_repeat`](day_spec::KeyEvent::is_repeat) tells them apart, so an action
    /// that should fire once per press skips the repeats). The key is a typed
    /// [`Key`](day_spec::Key):
    ///
    /// ```ignore
    /// canvas(draw).focusable().on_key(move |k| match k.key {
    ///     Key::ArrowLeft => nudge(-1.0),
    ///     Key::ArrowRight => nudge(1.0),
    ///     Key::Character(' ') if !k.is_repeat() => fire(),
    ///     _ => {}
    /// })
    /// ```
    fn on_key(self, f: impl Fn(&day_spec::KeyEvent) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_key(f)
    }

    /// Handle the release of a key this piece heard go down (docs/menus.md "Keys"): the other
    /// half of held-key movement, where [`Decorate::on_key`] starts moving and this stops.
    /// Delivered where `Cap::KeyPhases` is supported; elsewhere it never fires, so step per
    /// press there instead.
    fn on_key_up(self, f: impl Fn(&day_spec::KeyEvent) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_key_up(f)
    }

    /// Declare toolbar items on the chrome this piece sits under (docs/toolbars.md).
    ///
    /// Takes one item, a list of them, or a closure that derives the list and re-runs whenever
    /// its reactive reads change:
    ///
    /// ```ignore
    /// page.toolbar(toolbar_button(day_core::id!(share), tr("share")).icon(Symbol::Share).action(share))
    /// page.toolbar([reply, forward, archive])
    /// page.toolbar(move || vec![toolbar_button(day_core::id!(undo), tr("undo")).enabled(can_undo)])
    /// ```
    ///
    /// Which chrome carries them follows from where this piece is built (a destination page's
    /// own bar, a content-list pane's, or the window's if it is under no page at all), and the
    /// items are withdrawn when this piece is disposed. Where on that chrome they sit is
    /// [`ToolbarEntry::placement`](crate::ToolbarEntry::placement).
    fn toolbar<M>(self, content: impl crate::ToolbarContent<M>) -> Decorated<Self> {
        Decorated::new(self).toolbar(content)
    }

    /// A search field for this navigation page (docs/search.md, "Page search"), drawn where the
    /// platform puts a page's search: the window toolbar on a desktop, the page's navigation bar
    /// on iOS (a `UISearchController`), its app bar on Android. It shows while this page does, and
    /// its text is the app's `query` both ways. For a search the server runs, act on
    /// [`crate::Search::on_submit`]; for a local filter, read the query. Declare it on a
    /// `nav_stack` destination, as `.toolbar(…)` is declared; a whole navigation surface takes
    /// [`crate::Nav::searchable`] instead.
    fn searchable(self, search: crate::Search) -> Decorated<Self> {
        Decorated::new(self).searchable(search)
    }

    /// Opt this piece into the platform's focus system (docs/focus.md): the canvas contract
    /// for anything composed: it joins the key loop, takes focus on a press, reports through
    /// `.focused(…)`, and hears the arrows through `.on_key(…)` while focused. A composed
    /// list column is the motivating case (docs/navigation.md). On a backend without the
    /// `set_focusable` duty the piece renders normally and never takes focus.
    fn focusable(self) -> Decorated<Self> {
        Decorated::new(self).focusable()
    }

    /// Take the whole keyboard while this piece is focused (docs/menus.md "The full route"):
    /// every key the platform delivers reaches [`Decorate::on_key`], not only the arrows and
    /// digits. A key that types nothing is named by its web name, a key that types by its
    /// unmodified character, and what it typed rides in [`day_spec::KeyEvent::text`]: the
    /// character, or the whole string an input method committed (a dead-key accent, a
    /// Japanese phrase), delivered once, after the composition. Control chords arrive with
    /// [`day_spec::Modifiers::CONTROL`] set and no text; only the platform's own command
    /// chords (⌘ on Apple platforms) stay with the menu. On a phone, focusing the piece
    /// raises the soft keyboard, the way focusing a text field does, and what it types
    /// arrives the same way. For a terminal, a code editor or a game that reads letters, on
    /// a `canvas`:
    ///
    /// ```ignore
    /// canvas(draw).keyboard().on_key(move |k| match k.typed() {
    ///     Some(text) => term.type_text(text),
    ///     None => term.press(&k.key, k.modifiers),
    /// })
    /// ```
    ///
    /// Pair it with [`Decorate::on_key`]: it declares the route, the handler hears it, and a
    /// declaration with no handler claims nothing. The route is the canvas's (the one piece
    /// with no native control under it); a `.focusable()` container keeps the short route.
    /// A piece that takes the whole keyboard also takes the Tab key, so it leaves the
    /// platform's focus walking: a press elsewhere, or writing `.focused(false)`, gives the
    /// keyboard back.
    fn keyboard(self) -> Decorated<Self> {
        Decorated::new(self).keyboard()
    }

    /// Offer data when a drag starts on this piece (docs/drag-and-drop.md "Offering data"):
    /// `source` gets the press point in the piece's own coordinates and answers what to carry,
    /// or `None` to let the press be something else (a tap, a scroll).
    fn drag_source(
        self,
        source: impl Fn(day_spec::Point) -> Option<day_spec::transfer::Offer> + 'static,
    ) -> Decorated<Self> {
        Decorated::new(self).drag_source(source)
    }
    /// Accept drops through the platform's native drag session (docs/drag-and-drop.md
    /// "Accepting drops"). The target is one value: its types, an optional acceptance policy,
    /// and what runs as a drag enters, leaves and drops.
    ///
    /// ```ignore
    /// column(rows).drop_target(
    ///     DropTarget::new(["text/plain"])
    ///         .on_enter(move |_| hovering.set(true))
    ///         .on_leave(move || hovering.set(false))
    ///         .on_drop(move |drop| take(drop.items)),
    /// )
    /// ```
    fn drop_target(self, target: day_spec::transfer::Target) -> Decorated<Self> {
        Decorated::new(self).drop_target(target)
    }

    /// Accept files dropped on this piece: `f` receives their local paths. The short form of
    /// [`Decorate::drop_target`] for the common case; use the full target for a hover
    /// highlight or another data type.
    fn on_files_dropped(self, f: impl Fn(Vec<std::path::PathBuf>) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_files_dropped(f)
    }
    /// A context menu (docs/menus.md): shown on secondary-click / long-press, built from the
    /// same `menu_item`/`sub_menu` entries as everywhere else. Takes a fixed list, or a closure
    /// built when the menu is summoned ("Dynamic context menus"): with no argument, or with the
    /// summon point in this piece's own coordinates, so a canvas can select what is under the
    /// pointer and offer commands for that selection. An empty result shows nothing.
    ///
    /// ```ignore
    /// label(t).context_menu([menu_item(res::str::copy()).action(copy)])
    /// canvas(draw).context_menu(move |at: Point| commands_for(hit_test(at)))
    /// ```
    fn context_menu<M>(self, menu: impl ContextMenuContent<M>) -> Decorated<Self> {
        Decorated::new(self).context_menu(menu)
    }

    /// Fire on each phase of a press-drag over this piece (docs/gestures.md "Gestures"). The
    /// [`Drag`] carries where the pointer is, how far it has come, and how fast it is moving;
    /// at `Ended` the velocity is the release velocity. Once the drag has begun it belongs to
    /// this piece: an enclosing scroll view does not take it over.
    fn on_drag(self, f: impl Fn(Drag) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_drag(f)
    }

    /// [`Decorate::on_drag`] for a drag that follows one axis, inside a scroll view that
    /// scrolls along the other: a horizontal swipe on a row of a vertical list. A pointer that
    /// sets off along `axis` is this piece's drag; one that sets off across it scrolls, and
    /// the handler hears nothing of it.
    ///
    /// ```ignore
    /// row(content).on_drag_along(DragAxis::Horizontal, move |d| offset.set(d.translation.x))
    /// ```
    fn on_drag_along(
        self,
        axis: day_spec::DragAxis,
        f: impl Fn(Drag) + 'static,
    ) -> Decorated<Self> {
        Decorated::new(self).on_drag_along(axis, f)
    }

    /// Fire when a press is held still on this piece (docs/gestures.md "Long press"), with the
    /// press point in the piece's coordinates. It fires while the pointer is still down, and
    /// the release that follows is not a tap, so `.on_tap(..).on_long_press(..)` are
    /// alternatives. Pair it with another route to the same command (a menu, a button): a
    /// long press is not discoverable.
    fn on_long_press(self, f: impl Fn(day_spec::Point) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_long_press(f)
    }

    /// Fire on two taps in quick succession (docs/gestures.md "Double tap"). On a piece that
    /// also has [`Decorate::on_tap`], a single tap waits out the double-tap interval and the
    /// two never both fire; with a double tap alone nothing waits.
    fn on_double_tap(self, f: impl Fn() + 'static) -> Decorated<Self> {
        Decorated::new(self).on_double_tap(f)
    }

    /// [`on_double_tap`](Self::on_double_tap), told where: the second tap's point in the
    /// piece's own coordinate space, origin at its top-leading corner. What a map or an image
    /// viewer zooms around.
    fn on_double_tap_at(self, f: impl Fn(day_spec::Point) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_double_tap_at(f)
    }

    /// Follow the press on this piece: `true` at the pointer-down itself, `false` when the
    /// press ends (the release, the pointer leaving, or the platform taking the gesture away).
    /// Every `true` is followed by one `false`, so a handler that writes a `Signal<bool>` is a
    /// one-liner, and it never delays or cancels a tap or a drag on the same piece
    /// (docs/gestures.md "Press"). For styling, [`Decorate::interaction`] already tracks it.
    fn on_pressed(self, f: impl Fn(bool) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_pressed(f)
    }

    /// Receive the raw contacts on this piece (docs/gestures.md "Touches"): every finger that is
    /// down, each with a stable id, plus the ones that lifted in this event. For a drawing
    /// surface, a multi-finger instrument or a recognizer of your own. Where the platform
    /// reports one pointer (`day::gesture_support(GestureKind::Touches)` is `Emulated`), the
    /// list holds that one contact with id 0.
    fn on_touches(self, f: impl Fn(&[day_spec::Touch]) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_touches(f)
    }

    /// Grow the area in which this piece takes pointer input by `slop` points on each edge,
    /// without moving or resizing it (docs/gestures.md "Hit slop"): a 12-point dot with
    /// `.hit_slop(16.0)` is a 44-point target. Layout and drawing do not change. Where
    /// `Cap::HitSlop` is unsupported the piece keeps its own bounds.
    fn hit_slop(self, slop: impl Into<Insets>) -> Decorated<Self> {
        Decorated::new(self).hit_slop(slop)
    }

    /// Track this piece's hover, press and focus in `state` (docs/gestures.md "State styling"),
    /// for reactive modifiers to read. See [`InteractionState`].
    fn interaction(self, state: InteractionState) -> Decorated<Self> {
        Decorated::new(self).interaction(state)
    }

    /// Enable or disable this piece and everything in it (docs/forms.md "Disabling a group").
    /// While disabled, the controls inside are disabled the platform's way (dimmed, out of
    /// the focus order, reported disabled to assistive technology) and no tap, drag, hover,
    /// press or key reaches a handler in the subtree. Takes a constant, a signal or a closure:
    ///
    /// ```ignore
    /// form((name_field, email_field, send_button)).enabled(move || !sending.get())
    /// ```
    ///
    /// A control's own `.enabled(..)` builder still applies inside: a control is enabled when
    /// it and every group around it are.
    fn enabled<M>(self, enabled: impl IntoReactive<bool, M>) -> Decorated<Self> {
        Decorated::new(self).enabled(enabled)
    }

    /// Fire on each phase of a pinch/magnify over this piece (docs/canvas.md "Zoom and
    /// pan"). Only backends with a native recognizer wired emit it; pair a zoom with
    /// visible controls.
    fn on_pinch(self, f: impl Fn(Pinch) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_pinch(f)
    }

    /// Fire on each phase of a two-finger rotation over this piece (docs/canvas.md "Zoom and
    /// pan"). [`Rotate::rotation`] is an [`Angle`], cumulative within a gesture, positive
    /// clockwise; a trackpad tick arrives as a lone `Changed`. Pair it with a visible way back
    /// to north.
    fn on_rotate(self, f: impl Fn(Rotate) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_rotate(f)
    }

    /// Fire on each viewport-pan event over this piece (docs/canvas.md "Zoom and pan"):
    /// trackpad two-finger scroll, two-finger touch pan. `delta` is incremental; apply it
    /// as it arrives.
    fn on_pan(self, f: impl Fn(Pan) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_pan(f)
    }

    /// The pointer moving over this piece: `Some(point)` in its own coordinates while inside,
    /// `None` when it leaves (docs/canvas.md "Interaction"). Pointer-only; wire `on_tap_at`
    /// alongside it so a touch-only device can reach the same thing.
    fn on_hover(self, f: impl Fn(Option<day_spec::Point>) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_hover(f)
    }

    /// The platform's card around this piece: the grouped-content surface with
    /// [`Radius::M`](crate::Radius::M) corners and a `Space::M` inset. Drawn by the toolkit, so
    /// it follows the appearance and never steals hit testing from the content.
    fn card(self) -> Decorated<Self> {
        Decorated::new(self).card()
    }

    /// Fill the piece's bounds with a [`Paint`] painted behind it: a message-bubble / card /
    /// badge surface. Takes a [`Color`](day_spec::Color), a
    /// [`SystemColor`](day_spec::SystemColor), an [`Ink`] (`Ink::adaptive(light, dark)`), a
    /// gradient, or a signal or closure of any of them. An ink follows the appearance without a
    /// rebuild or a closure; a reactive source repaints when it changes. Wraps the piece in a
    /// native container that carries the fill, so it composes with [`Self::corner_radius`] for
    /// a rounded colored surface and with [`Self::padding`] for interior inset.
    fn background<M>(self, fill: impl IntoReactive<Paint, M>) -> Decorated<Self> {
        Decorated::new(self).background(fill)
    }

    /// Round the piece's corners, clipping its background and content to the rounded
    /// rectangle. Takes points, a [`Radius`](crate::Radius) step, a frame-relative
    /// [`Corner`](crate::Corner) (`Corner::CAPSULE` for round ends), or a signal or closure
    /// of any of them; a change patches the view in place. Compose after [`Self::background`]
    /// for a rounded colored surface, or use alone to round a clipped child (an avatar image).
    fn corner_radius<M>(self, radius: impl IntoReactive<crate::Corner, M>) -> Decorated<Self> {
        Decorated::new(self).corner_radius(radius)
    }

    /// Put a styled native [`Surface`] behind the piece: fill (any [`Paint`]), rounded corners
    /// (all or some), a border and a shadow, all on one native view so the border follows the
    /// corners and the shadow takes their shape (docs/surfaces.md). Takes a surface, or a
    /// signal or closure of one; a change patches the view in place.
    fn surface<M>(self, surface: impl IntoReactive<Surface, M>) -> Decorated<Self> {
        Decorated::new(self).surface(surface)
    }

    /// A square line of `color`, `width` points wide, around the piece's frame, inside it and
    /// over its content. Either argument may be a signal or a closure. For a border that
    /// follows rounded corners, use [`Self::surface`].
    fn border<Mc, Mw>(
        self,
        color: impl IntoReactive<Ink, Mc>,
        width: impl IntoReactive<f64, Mw>,
    ) -> Decorated<Self> {
        Decorated::new(self).border(color, width)
    }

    /// A shadow of the piece's frame, drawn outside it by the toolkit. Takes a
    /// [`Shadow`](day_spec::Shadow), or a signal or closure of one. For a rounded shadow, or
    /// one under a filled surface, use [`Self::surface`].
    fn shadow<M>(self, shadow: impl IntoReactive<day_spec::Shadow, M>) -> Decorated<Self> {
        Decorated::new(self).shadow(shadow)
    }

    /// Animate/set the piece's opacity (`0.0` transparent … `1.0` opaque). Wrapped in a native
    /// layer so it composes with `.background`; the change animates when made inside
    /// [`with_animation`] or under a `.animation` ancestor (§8.4).
    fn opacity<M>(self, opacity: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).opacity(opacity)
    }

    /// Apply an animatable [`Transform`] (translate/scale/rotate about the center): the cheap
    /// movement/scaling channel that never triggers relayout (§8.4). [`Self::translation`],
    /// [`Self::scale`] and [`Self::rotation`] are its one-property forms.
    fn transform<M>(self, t: impl IntoReactive<Transform, M>) -> Decorated<Self> {
        Decorated::new(self).transform(t)
    }

    /// Uniformly scale the piece by `factor` about its center (animatable). Convenience over
    /// [`Self::transform`].
    fn scale<M>(self, factor: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).scale(factor)
    }

    /// Rotate the piece by `angle` clockwise about its center (animatable):
    /// `.rotation(Angle::deg(15.0))`.
    fn rotation<M>(self, angle: impl IntoReactive<Angle, M>) -> Decorated<Self> {
        Decorated::new(self).rotation(angle)
    }

    /// Translate the piece by (`x`, `y`) points without relayout (animatable). Layout still
    /// places the piece where it was; only what is drawn moves.
    fn translation<Mx, My>(
        self,
        x: impl IntoReactive<f64, Mx>,
        y: impl IntoReactive<f64, My>,
    ) -> Decorated<Self> {
        Decorated::new(self).translation(x, y)
    }

    /// Attach an implicit animation (§8.4): changes to this piece's (and its descendants')
    /// animatable properties animate with `anim` even outside a [`with_animation`]. SwiftUI's
    /// `.animation`. The ambient `with_animation` takes precedence when both apply.
    fn animation(self, anim: AnimSpec) -> Decorated<Self> {
        Decorated::new(self).animation(anim)
    }

    /// Apply a [`Modifier`] (or, via the blanket impl, a plain `FnOnce(AnyPiece) -> AnyPiece`
    /// closure) to this piece. Pure composition: `content.modifier(m) == m.apply(content.any())`.
    ///
    /// The one modifier that erases: `Modifier` is defined over [`AnyPiece`], so the piece's own
    /// type cannot survive it.
    fn modifier(self, m: impl Modifier) -> AnyPiece {
        m.apply(self.any())
    }

    /// Draw `over` on top of this piece, centered, without affecting layout size: a badge /
    /// annotation overlay. `self` is the sizing content (bottom of the z-order); `over` is proposed
    /// `self`'s size and drawn on top. For an explicit alignment use [`Self::overlay_aligned`]; for
    /// a stack that sizes to the union of its children use [`zstack`].
    fn overlay(self, over: impl Piece) -> Decorated<Self> {
        Decorated::new(self).overlay(over)
    }

    /// [`Self::overlay`] with an explicit [`Alignment`] for the annotation (e.g. a corner badge with
    /// [`Alignment::TopTrailing`]).
    fn overlay_aligned(self, align: Alignment, over: impl Piece) -> Decorated<Self> {
        Decorated::new(self).overlay_aligned(align, over)
    }

    /// Constrain this piece to a `width / height` ratio: the largest box of that shape which
    /// fits whatever the parent offers (SwiftUI's `.aspectRatio(_:contentMode: .fit)`).
    ///
    /// Pair it with [`Self::grow`] and [`Dimension::Width`] to take the available width and
    /// derive the height from it: a `canvas` whose drawing keeps its proportions as the window
    /// resizes, say. `image` has carried this since it shipped; this is the same layout, for any
    /// piece.
    ///
    /// A ratio that is not finite and positive describes no box, so it is ignored. The ratio
    /// may be a signal or a closure; a change re-lays the piece out.
    fn aspect_ratio<M>(self, ratio: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).aspect_ratio(ratio)
    }

    /// Position this piece within its assigned space without stretching its content.
    ///
    /// Alignment alone hugs the content. Apply growth or a frame afterward to provide room:
    /// `piece.aligned(Alignment::Center).grow(Dimension::Width)` centers a naturally sized
    /// piece across the available width. Reversing that order centers already-expanded content.
    /// Leading and trailing follow the locale's layout direction. This adds no native view,
    /// accessibility role, or focus stop; text alignment remains a separate piece setting.
    fn aligned(self, alignment: Alignment) -> Decorated<Self> {
        Decorated::new(self).aligned(alignment)
    }

    /// Expand to fill the space offered in the selected dimensions.
    ///
    /// Other dimensions hug the content. An unbounded proposal (such as a scroll view's
    /// scrolling axis) keeps the content's measured size. This layout-only wrapper carries
    /// grow [`Flex`] so its parent can allocate extra space; it adds no native view.
    ///
    /// Siblings that grow along the same line share the spare space equally. Give one a
    /// larger share with a weight: `.grow(Dimension::Width.weight(2.0))` beside a plain
    /// `.grow(Dimension::Width)` takes two thirds. In a wrapping row
    /// (`row(..).fit(RowFit::Wrap { .. })`) a line breaks on the children's natural widths
    /// and the growing ones then share what the line has left, so
    /// `.min_width(50.0).grow(Dimension::Width)` reads "at least 50 points, then share the
    /// line".
    fn grow(self, grow: impl Into<Grow>) -> Decorated<Self> {
        Decorated::new(self).grow(grow)
    }

    #[deprecated(note = "use .grow(Dimension::Width)")]
    fn grow_w(self) -> Decorated<Self> {
        Decorated::new(self).grow(Dimension::Width)
    }

    #[deprecated(note = "use .grow(Dimension::Height)")]
    fn grow_h(self) -> Decorated<Self> {
        Decorated::new(self).grow(Dimension::Height)
    }

    /// Span `n` columns (n ≥ 1) of the enclosing [`grid`] (docs/grid.md). Grid modifiers apply
    /// to the outermost layout node after wrapping modifiers, regardless of their chain position.
    fn grid_span(self, n: usize) -> Decorated<Self> {
        Decorated::new(self).grid_span(n)
    }

    /// Override this cell's alignment within its cell rect of the enclosing [`grid`]
    /// (docs/grid.md). Like [`Decorated::grid_span`], this applies outside wrapping modifiers
    /// regardless of its position in the chain.
    fn grid_align(self, a: Alignment) -> Decorated<Self> {
        Decorated::new(self).grid_align(a)
    }

    /// While this subtree is mounted, ask the OS to require a second swipe for its edge
    /// gestures on `edges` (docs/cover.md), the SwiftUI `defersSystemGestures(on:)`
    /// analogue. Put it on a game or drawing surface whose touches run to the screen edge,
    /// so a swipe up from the bottom doesn't leave the app. iOS defers the chosen edges'
    /// system gestures; Android enters swipe-to-reveal immersive mode while any subtree
    /// requests deferral; desktop backends no-op.
    fn defers_system_gestures(self, edges: day_spec::Edges) -> Decorated<Self> {
        Decorated::new(self).defers_system_gestures(edges)
    }

    /// While this subtree is mounted and `hidden` is true, hide the system status bar, the
    /// SwiftUI `statusBarHidden(_:)` analogue (docs/cover.md). `hidden` is a constant, a
    /// `Signal`, or a closure, so a setting can turn it on and off in place. Put it on a page
    /// that wants the whole screen (a reader, a game, a photo viewer); the bar returns when
    /// the page unmounts. Any mounted request that answers true hides the bar. iOS, Android and
    /// HarmonyOS hide it (`Cap::StatusBarHidden`); desktop and web have none and ignore it.
    fn status_bar_hidden<M>(self, hidden: impl IntoReactive<bool, M>) -> Decorated<Self> {
        Decorated::new(self).status_bar_hidden(hidden)
    }

    /// While this subtree is mounted, the enclosing [`cover`] (or other modal surface) must
    /// not be dismissed interactively, the SwiftUI `interactiveDismissDisabled()` analogue
    /// (docs/cover.md). System back / sheet gestures are ignored; only programmatic writes
    /// (an explicit close control) dismiss it.
    fn interactive_dismiss_disabled(self) -> Decorated<Self> {
        Decorated::new(self).interactive_dismiss_disabled()
    }

    /// While this subtree is mounted and `enabled` holds, a back nothing else takes runs
    /// `handler` instead of the platform's default: Jetpack Compose's `BackHandler(enabled)`
    /// (docs/navigation.md "Handling the system Back"). `enabled` is a constant, a `Signal`, or
    /// a closure, followed as it changes.
    ///
    /// The back is Android's system back button and gesture (predictive back included),
    /// HarmonyOS's back gesture, [`nav_back()`](crate::nav_back), and dayscript's `nav_back`
    /// step in both forms. Navigation answers first: a presented [`cover`] closes and a pushed
    /// page pops before any handler runs, so the handler takes only the back that would have
    /// left the app (on a phone) or done nothing (on a desktop). Among several, the handler in
    /// the topmost cover wins, then the one nested most deeply in the piece tree, then the most
    /// recently mounted. A page's own "leave this page?" policy belongs on
    /// [`NavStack::on_back`] instead.
    ///
    /// Disabled (or unmounted) on Android, the system back is released again, so the
    /// predictive "back to home" preview shows whenever the press would leave the app.
    ///
    /// ```ignore
    /// // A browser: Back goes back in the page's history while the tab can.
    /// page_view(tab).on_system_back(move || tab.can_go_back(), move || tab.go_back())
    /// ```
    fn on_system_back<M>(
        self,
        enabled: impl IntoReactive<bool, M>,
        handler: impl Fn() + 'static,
    ) -> Decorated<Self> {
        Decorated::new(self).on_system_back(enabled, handler)
    }

    /// Erase to a single [`AnyPiece`]: for a `Vec<AnyPiece>`, a `-> AnyPiece` signature, or any other
    /// place one concrete type is required. [`AnyPiece::any`] is inherent and returns `self`, so
    /// erasing an already-erased piece costs nothing.
    fn any(self) -> AnyPiece {
        AnyPiece::new(self)
    }
}

impl<P: Piece> Decorate for P {}

/// One a11y string that reads a signal or the locale, with the patch that re-sends it alone.
type LiveA11yString = (TextSource, fn(String) -> A11yProps);

/// The annotations `.a11y(|a| …)` collects (docs/accessibility.md). The three strings are
/// [`TextSource`]s, so each takes a literal, a `String`, a `Signal<String>` or a closure: a
/// gauge's value follows the gauge, and a localized label follows the locale.
#[derive(Default)]
#[must_use]
pub struct A11yBuilder {
    label: Option<TextSource>,
    hint: Option<TextSource>,
    value: Option<TextSource>,
    /// The selected state, reactive so a tab or row follows the selection.
    selected: Option<Reactive<bool>>,
    props: A11yProps,
}

impl A11yBuilder {
    /// What the screen reader calls the element.
    pub fn label<M>(mut self, s: impl IntoText<M>) -> Self {
        self.label = Some(s.into_text());
        self
    }
    /// A longer description of what the element does.
    pub fn hint<M>(mut self, s: impl IntoText<M>) -> Self {
        self.hint = Some(s.into_text());
        self
    }
    /// The control's current value read aloud by the screen reader (e.g. a `Meter`'s "72%").
    /// Reactive like the others: `.value(move || format!("{:.0}%", level.get()))` re-sends the
    /// value whenever `level` changes.
    pub fn value<M>(mut self, s: impl IntoText<M>) -> Self {
        self.value = Some(s.into_text());
        self
    }
    pub fn role(mut self, r: Role) -> Self {
        self.props.role = r;
        self
    }
    /// Whether this element is the selected one among its peers (a `Role::Tab`, a row): the
    /// platform's selected state, which a screen reader reads as "selected". Reactive like the
    /// strings: `.selected(move || current.get() == key)` re-sends it when the selection moves.
    pub fn selected<M>(mut self, on: impl IntoReactive<bool, M>) -> Self {
        self.selected = Some(on.into_reactive());
        self
    }
    /// Hide this element from assistive tech (still visible on screen), e.g. a redundant chrome
    /// element already announced by its labeled sibling.
    pub fn hidden(mut self) -> Self {
        self.props.hidden = true;
        self
    }
    /// Purely decorative (a background flourish): hidden from assistive tech and, for images,
    /// exempt from the "needs a label" lint (§13).
    pub fn decorative(mut self) -> Self {
        self.props.decorative = true;
        self.props.hidden = true;
        self
    }

    /// The annotations as first applied, with every string resolved once, the sources that
    /// can change, each paired with the patch that re-sends it, and the selected state when it
    /// reads a signal.
    fn split(self) -> (A11yProps, Vec<LiveA11yString>, Option<Reactive<bool>>) {
        let mut props = self.props;
        let mut live = Vec::new();
        let selected = match self.selected {
            Some(Reactive::Const(on)) => {
                props.selected = Some(on);
                None
            }
            Some(dynamic @ Reactive::Dyn(_)) => {
                props.selected = Some(day_reactive::untrack(|| dynamic.get()));
                Some(dynamic)
            }
            None => None,
        };
        let mut take =
            |src: Option<TextSource>, slot: &mut Option<String>, patch: fn(String) -> A11yProps| {
                if let Some(src) = src {
                    *slot = Some(src.initial());
                    if matches!(src, TextSource::Dyn(_)) {
                        live.push((src, patch));
                    }
                }
            };
        take(self.label, &mut props.label, |label| A11yProps {
            label: Some(label),
            ..Default::default()
        });
        take(self.hint, &mut props.hint, |hint| A11yProps {
            hint: Some(hint),
            ..Default::default()
        });
        take(self.value, &mut props.value, |value| A11yProps {
            value: Some(value),
            ..Default::default()
        });
        (props, live, selected)
    }
}

// Native callbacks arrive outside the event dispatcher. Enter their owning scope so
// window ambient state resolves correctly, and reject callbacks retained after disposal.
fn scoped_drag_source(source: day_spec::transfer::Source) -> day_spec::transfer::Source {
    let scope = Scope::current();
    let alive = Rc::new(Cell::new(true));
    let cleanup = alive.clone();
    scope.on_cleanup(move || cleanup.set(false));
    Rc::new(move |point| alive.get().then(|| scope.enter(|| source(point))).flatten())
}
fn scoped_drop_target(target: day_spec::transfer::Target) -> day_spec::transfer::Target {
    let scope = Scope::current();
    let alive = Rc::new(Cell::new(true));
    let cleanup = alive.clone();
    scope.on_cleanup(move || cleanup.set(false));
    target.guarded(Rc::new(move |run: &mut dyn FnMut()| {
        if alive.get() {
            scope.enter(run);
        }
    }))
}

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The modifiers' `#[day::test]` cases, next to the `Decorate` methods they prove.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::AnyPiece;
    use day_core::conformance::{Case, Drive, FrameExpect, NativeExpect};
    use day_reactive::{Signal, untrack};
    use day_spec::{Cap, Color, DragPhase, Point, Transform};

    use crate::*;

    fn size(width: f64, height: f64) -> FrameExpect {
        FrameExpect {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        }
    }

    #[day_macros::test(day_core)]
    fn reactive_frame_and_minimum_height() -> Case {
        let width = Signal::new(80.0);
        let height = Signal::new(44.0);
        Case::new()
            .proves_modifier("width")
            .proves_modifier("height")
            .proves_modifier("frame")
            .proves_modifier("min_height")
            .page(move || {
                column((
                    rectangle()
                        .fill(day_spec::Color::hex(0x0033_6699))
                        .frame(width, height)
                        .id(day_core::id!(resizing)),
                    rectangle()
                        .fill(day_spec::Color::hex(0x0066_3399))
                        .width(width)
                        .height(20.0)
                        .min_height(44.0)
                        .id(day_core::id!(minimum)),
                    button("Resize fixture")
                        .action(move || {
                            width.set(120.0);
                            height.set(64.0);
                        })
                        .id(day_core::id!(resize)),
                ))
                .spacing(0.0)
            })
            .drive(|d: Drive| async move {
                d.assert_frame("resizing", size(80.0, 44.0)).await?;
                d.assert_frame("minimum", size(80.0, 44.0)).await?;
                d.tap("resize").await?;
                d.assert_frame("resizing", size(120.0, 64.0)).await?;
                d.assert_frame("minimum", size(120.0, 44.0)).await
            })
    }

    /// A subtree's reading direction mirrors its placement and nothing outside it: in a
    /// right-to-left subtree the first box sits at the right edge, a left-to-right subtree
    /// nested inside it runs the usual way again, and a flip of the signal re-lays it out.
    #[day_macros::test(day_core)]
    fn layout_direction_mirrors_a_subtree() -> Case {
        let direction = Signal::new(day_spec::LayoutDirection::Rtl);
        let pair = |first: day_core::ElementId, second: day_core::ElementId| {
            row((
                rectangle()
                    .fill(Color::hex(0x0033_6699))
                    .frame(40.0, 20.0)
                    .id(first),
                rectangle()
                    .fill(Color::hex(0x0099_3366))
                    .frame(60.0, 20.0)
                    .id(second),
            ))
        };
        Case::new()
            .proves_modifier("layout_direction")
            .proves_duty("set_layout_direction")
            .page(move || {
                column((
                    column((
                        pair(day_core::id!(dir_a), day_core::id!(dir_b))
                            .width(200.0)
                            .wrapper_id(day_core::id!(dir_outer)),
                        pair(day_core::id!(dir_c), day_core::id!(dir_d))
                            .width(200.0)
                            .wrapper_id(day_core::id!(dir_inner))
                            .layout_direction(day_spec::LayoutDirection::Ltr),
                        label("Text in the subtree").id(day_core::id!(dir_text)),
                    ))
                    .align(HAlign::Leading)
                    .layout_direction(direction),
                    button("Flip")
                        .action(move || direction.set(day_spec::LayoutDirection::Ltr))
                        .id(day_core::id!(dir_flip)),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .shot("rtl")
            .drive(|d: Drive| async move {
                let at = |x: f64, outer: &str| FrameExpect {
                    x: Some(x),
                    relative_to: Some(outer.into()),
                    ..Default::default()
                };
                // Right to left: the first box ends at the row's right edge.
                d.assert_frame("dir_a", at(160.0, "dir_outer")).await?;
                d.assert_frame("dir_b", at(100.0, "dir_outer")).await?;
                // The nested left-to-right row is not mirrored.
                d.assert_frame("dir_c", at(0.0, "dir_inner")).await?;
                d.assert_frame("dir_d", at(40.0, "dir_inner")).await?;
                d.assert_text("dir_text", "Text in the subtree").await?;
                d.tap("dir_flip").await?;
                d.wait_idle().await?;
                d.assert_frame("dir_a", at(0.0, "dir_outer")).await?;
                d.assert_frame("dir_b", at(40.0, "dir_outer")).await
            })
    }

    #[day_macros::test(day_core)]
    fn text_reservation() -> Case {
        let value = Signal::new("1".to_owned());
        let sample = Signal::new("8888".to_owned());
        Case::new()
            .proves_modifier("reserving")
            .page(move || {
                column((
                    label(value)
                        .reserving(sample)
                        .font(day_spec::Font::Title2)
                        .tabular()
                        .id(day_core::id!(reserved_readout)),
                    label("8888")
                        .font(day_spec::Font::Title2)
                        .tabular()
                        .id(day_core::id!(reservation_reference)),
                    label("88888888")
                        .font(day_spec::Font::Title2)
                        .tabular()
                        .id(day_core::id!(reservation_expanded)),
                    button("Update fixture value")
                        .action(move || value.set("88".into()))
                        .id(day_core::id!(reservation_update)),
                    button("Expand fixture sample")
                        .action(move || sample.set("88888888".into()))
                        .id(day_core::id!(reservation_expand)),
                ))
                .align(HAlign::Leading)
            })
            .drive(|d: Drive| async move {
                let dimensions = |id: &str| {
                    day_core::with_tree(|tree| {
                        tree.node_frame(tree.find_by_id(id).unwrap()).unwrap().size
                    })
                };
                let reference = dimensions("reservation_reference");
                d.assert_frame("reserved_readout", size(reference.width, reference.height))
                    .await?;
                d.tap("reservation_update").await?;
                d.assert_text("reserved_readout", "88").await?;
                d.assert_frame("reserved_readout", size(reference.width, reference.height))
                    .await?;
                d.tap("reservation_expand").await?;
                let expanded = dimensions("reservation_expanded");
                d.assert_frame("reserved_readout", size(expanded.width, expanded.height))
                    .await?;
                d.assert_text("reserved_readout", "88").await
            })
    }

    /// Padding grows the frame by its insets and moves the content in by them.
    #[day_macros::test(day_core)]
    fn padding_insets() -> Case {
        Case::new()
            .proves_modifier("padding")
            .proves_modifier("wrapper_id")
            .page(|| {
                // `id` names the rectangle wherever it sits; the padding box is a wrapper, so
                // it is named with `wrapper_id`, the one ordered identity modifier.
                rectangle()
                    .fill(Color::rgb(1.0, 0.0, 0.0))
                    .id(day_core::id!(inner))
                    .frame(40.0, 20.0)
                    .padding(10.0)
                    .wrapper_id(day_core::id!(outer))
            })
            .drive(|d: Drive| async move {
                d.assert_frame("outer", size(60.0, 40.0)).await?;
                d.assert_frame(
                    "inner",
                    FrameExpect {
                        x: Some(10.0),
                        y: Some(10.0),
                        relative_to: Some("outer".into()),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A minimum width widens a narrow piece; a maximum narrows a wide one.
    #[day_macros::test(day_core)]
    fn min_max_width() -> Case {
        Case::new()
            .proves_modifier("min_width")
            .proves_modifier("max_width")
            .page(|| {
                column((
                    // The widened box, not the label's own node.
                    label("Hi").min_width(120.0).wrapper_id(day_core::id!(min)),
                    label("A label long enough that it would run past eighty points")
                        .max_width(80.0)
                        .wrapper_id(day_core::id!(max)),
                ))
                .align(HAlign::Leading)
            })
            .drive(|d: Drive| async move {
                d.assert_frame(
                    "min",
                    FrameExpect {
                        width: Some(120.0),
                        ..Default::default()
                    },
                )
                .await?;
                d.assert_frame(
                    "max",
                    FrameExpect {
                        width: Some(80.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A maximum height stops a scroll, which takes all the height it is offered, at the cap.
    #[day_macros::test(day_core)]
    fn max_height_caps_a_scroll() -> Case {
        Case::new()
            .proves_modifier("max_height")
            .page(|| {
                column((
                    scroll(column((
                        label("One"),
                        label("Two"),
                        label("Three"),
                        label("Four"),
                        label("Five"),
                        label("Six"),
                    )))
                    .max_height(60.0)
                    .wrapper_id(day_core::id!(capped)),
                    spacer(),
                ))
                .align(HAlign::Leading)
            })
            .drive(|d: Drive| async move {
                d.assert_frame(
                    "capped",
                    FrameExpect {
                        height: Some(60.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// An aspect ratio sets the height from the width.
    #[day_macros::test(day_core)]
    fn aspect_ratio_height() -> Case {
        Case::new()
            .proves_modifier("aspect_ratio")
            .page(|| {
                rectangle()
                    .fill(Color::rgb(0.0, 0.0, 1.0))
                    .aspect_ratio(2.0)
                    .id(day_core::id!(shape))
                    .width(100.0)
            })
            .drive(|d: Drive| async move { d.assert_frame("shape", size(100.0, 50.0)).await })
    }

    /// A growing child takes the row's remaining width.
    #[day_macros::test(day_core)]
    fn grow_width() -> Case {
        Case::new()
            .proves_modifier("grow")
            .page(|| {
                row((
                    rectangle()
                        .fill(Color::rgb(1.0, 0.0, 0.0))
                        .frame(40.0, 20.0),
                    rectangle()
                        .fill(Color::rgb(0.0, 0.0, 1.0))
                        .height(20.0)
                        .grow(Dimension::Width)
                        .id(day_core::id!(grower)),
                ))
                .spacing(0.0)
                .width(200.0)
            })
            .drive(|d: Drive| async move {
                d.assert_frame(
                    "grower",
                    FrameExpect {
                        width: Some(160.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// Alignment preserves the child's size inside an explicitly sized area.
    #[day_macros::test(day_core)]
    fn aligned_intrinsic_frame() -> Case {
        Case::new()
            .proves_modifier("aligned")
            .page(|| {
                // A native container gives both layout and toolkit probes the same origin,
                // independent of window chrome and mobile safe-area insets.
                zstack((rectangle()
                    .fill(Color::rgb(0.2, 0.5, 0.8))
                    .frame(40.0, 20.0)
                    .id(day_core::id!(aligned_child))
                    .aligned(day_core::Alignment::BottomTrailing)
                    .frame(200.0, 100.0),))
                .id(day_core::id!(aligned_container))
                .frame(200.0, 100.0)
            })
            .drive(|d: Drive| async move {
                d.assert_frame("aligned_container", size(200.0, 100.0))
                    .await?;
                d.assert_frame(
                    "aligned_child",
                    FrameExpect {
                        x: Some(160.0),
                        y: Some(80.0),
                        width: Some(40.0),
                        height: Some(20.0),
                        relative_to: Some("aligned_container".into()),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// An aligned overlay draws over its base, in the corner it names.
    #[day_macros::test(day_core)]
    fn overlay_aligned_corner() -> Case {
        Case::new()
            .proves_modifier("overlay_aligned")
            .requires(Cap::Snapshot)
            .page(|| {
                rectangle()
                    .fill(Color::rgb(1.0, 0.0, 0.0))
                    .frame(100.0, 60.0)
                    .overlay_aligned(
                        day_core::Alignment::BottomTrailing,
                        rectangle()
                            .fill(Color::rgb(0.0, 0.0, 1.0))
                            .frame(20.0, 20.0),
                    )
                    .id(day_core::id!(base))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("base", 0.95, 0.9, "#0000ff").await?;
                d.sample_pixel("base", 0.1, 0.1, "#ff0000").await
            })
    }

    /// A background fills the piece's frame.
    #[day_macros::test(day_core)]
    fn background_fill() -> Case {
        Case::new()
            .proves_modifier("background")
            .requires(Cap::Snapshot)
            .page(|| {
                label("")
                    .frame(80.0, 40.0)
                    .background(Color::rgb(1.0, 0.0, 0.0))
                    .id(day_core::id!(filled))
            })
            .drive(|d: Drive| async move { d.sample_pixel("filled", 0.5, 0.5, "#ff0000").await })
    }

    /// A corner radius cuts a background's corners and keeps its middle.
    #[day_macros::test(day_core)]
    fn corner_radius_cuts() -> Case {
        Case::new()
            .proves_modifier("corner_radius")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    label("")
                        .frame(80.0, 80.0)
                        .background(Color::rgb(0.0, 0.0, 1.0))
                        .corner_radius(30.0),
                ))
                .id(day_core::id!(ground))
                .frame(80.0, 80.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.03, 0.03, "#ffffff").await
            })
    }

    /// A surface's border is drawn inside its frame, over the fill's edge and not its middle.
    #[day_macros::test(day_core)]
    fn surface_border_frames_fill() -> Case {
        Case::new()
            .proves_modifier("surface")
            .proves_modifier("border")
            .requires(Cap::Snapshot)
            .page(|| {
                label("")
                    .frame(80.0, 80.0)
                    .surface(
                        Surface::new()
                            .fill(Color::rgb(0.0, 0.0, 1.0))
                            .border(Color::rgb(1.0, 0.0, 0.0), 8.0),
                    )
                    .id(day_core::id!(framed))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("framed", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("framed", 0.5, 0.05, "#ff0000").await?;
                d.sample_pixel("framed", 0.05, 0.5, "#ff0000").await?;
                d.sample_pixel("framed", 0.95, 0.5, "#ff0000").await?;
                d.sample_pixel("framed", 0.5, 0.95, "#ff0000").await
            })
    }

    /// A surface rounds only the corners it is told to: the top two are cut, the bottom two
    /// are still filled.
    #[day_macros::test(day_core)]
    fn surface_rounds_named_corners() -> Case {
        Case::new()
            .proves_modifier("surface")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    label("").frame(80.0, 80.0).surface(
                        Surface::new()
                            .fill(Color::rgb(0.0, 0.0, 1.0))
                            .corner_radius(30.0)
                            .corners(day_spec::Corners::TOP),
                    ),
                ))
                .id(day_core::id!(ground))
                .frame(80.0, 80.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.03, 0.03, "#ffffff").await?;
                d.sample_pixel("ground", 0.97, 0.03, "#ffffff").await?;
                d.sample_pixel("ground", 0.03, 0.97, "#0000ff").await?;
                d.sample_pixel("ground", 0.97, 0.97, "#0000ff").await
            })
    }

    /// A surface's gradient runs from its first stop at one edge to its last at the other.
    #[day_macros::test(day_core)]
    fn surface_gradient_fill() -> Case {
        Case::new()
            .proves_modifier("surface")
            .requires(Cap::Snapshot)
            .page(|| {
                label("")
                    .frame(80.0, 80.0)
                    .surface(Surface::new().fill(day_spec::LinearGradient::vertical(
                        Color::rgb(1.0, 0.0, 0.0),
                        Color::rgb(0.0, 0.0, 1.0),
                    )))
                    .id(day_core::id!(graded))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("graded", 0.5, 0.06, "#ff0000").await?;
                d.sample_pixel("graded", 0.5, 0.5, "#800080").await?;
                d.sample_pixel("graded", 0.5, 0.94, "#0000ff").await
            })
    }

    /// A surface's shadow is drawn outside its frame, moved by its offset, and leaves the
    /// surface itself and the ground beyond the shadow alone.
    #[day_macros::test(day_core)]
    fn surface_shadow_falls_outside() -> Case {
        Case::new()
            .proves_modifier("surface")
            .proves_modifier("shadow")
            .requires(Cap::Snapshot)
            .page(|| {
                // The surface is the middle 60 points of a 120-point ground, so its shadow,
                // 16 points down, shows in the band from 90 to 106.
                zstack((
                    rectangle().fill(Color::WHITE),
                    label("").frame(60.0, 60.0).surface(
                        Surface::new().fill(Color::rgb(0.0, 0.0, 1.0)).shadow(
                            day_spec::Shadow::new(Color::rgb(0.0, 0.0, 0.0), 4.0).offset(0.0, 16.0),
                        ),
                    ),
                ))
                .id(day_core::id!(ground))
                .frame(120.0, 120.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.5, 0.815, "#000000").await?;
                d.sample_pixel("ground", 0.5, 0.97, "#ffffff").await?;
                d.sample_pixel("ground", 0.5, 0.1, "#ffffff").await
            })
    }

    /// Buttons that switch the app's appearance, for the cases that prove a view follows it.
    fn appearance_switches() -> impl day_core::Piece {
        row((
            button("Light")
                .action(|| day_core::set_appearance(Some(false)))
                .id(day_core::id!(appearance::light)),
            button("Dark")
                .action(|| day_core::set_appearance(Some(true)))
                .id(day_core::id!(appearance::dark)),
            button("System")
                .action(|| day_core::set_appearance(None))
                .id(day_core::id!(appearance::system)),
        ))
        .spacing(8.0)
    }

    /// A fill given as an ink follows an appearance switch with no app code running: no
    /// closure, no signal, no rebuild. A light/dark pair recolors in place, and a platform
    /// role shows what `day::resolve` answers for it in each appearance (docs/color.md).
    #[day_macros::test(day_core)]
    fn ink_follows_the_appearance() -> Case {
        const PAIR: day_spec::Ink =
            day_spec::Ink::adaptive(Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
        Case::new()
            .proves_modifier("background")
            .proves_cap(Cap::Appearance)
            .requires(Cap::Appearance)
            .page(|| {
                column((
                    label("")
                        .frame(80.0, 40.0)
                        .background(PAIR)
                        .id(day_core::id!(paired)),
                    label("")
                        .frame(80.0, 40.0)
                        .background(day_spec::SystemColor::Background)
                        .border(day_spec::SystemColor::Separator, 2.0)
                        .id(day_core::id!(role)),
                    appearance_switches(),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                let shown = |c: Color| day_core::conformance::DisplayedExpect {
                    background: Some(c),
                    ..Default::default()
                };
                d.tap("appearance::light").await?;
                d.sample_pixel("paired", 0.5, 0.5, "#ff0000").await?;
                d.assert_displayed("paired", shown(Color::rgb(1.0, 0.0, 0.0)))
                    .await?;
                let light = day_core::resolve(day_spec::SystemColor::Background);
                d.assert_displayed("role", shown(light)).await?;
                d.tap("appearance::dark").await?;
                d.sample_pixel("paired", 0.5, 0.5, "#0000ff").await?;
                d.assert_displayed("paired", shown(Color::rgb(0.0, 0.0, 1.0)))
                    .await?;
                let dark = day_core::resolve(day_spec::SystemColor::Background);
                d.assert_displayed("role", shown(dark)).await?;
                d.tap("appearance::system").await
            })
    }

    /// A canvas that draws with an ink records the color of the current appearance and draws
    /// again when the appearance changes (docs/canvas.md "Colors").
    #[day_macros::test(day_core)]
    fn canvas_ink_follows_the_appearance() -> Case {
        Case::new()
            .proves(day_spec::kinds::CANVAS)
            .requires(Cap::Appearance)
            .requires(Cap::Snapshot)
            .page(|| {
                column((
                    canvas(|d, size| {
                        d.fill(
                            day_spec::Shape::Rect(day_spec::Rect::from_size(size)),
                            day_spec::Ink::adaptive(
                                Color::rgb(1.0, 0.0, 0.0),
                                Color::rgb(0.0, 0.0, 1.0),
                            ),
                        );
                    })
                    .frame(80.0, 40.0)
                    .id(day_core::id!(drawn)),
                    appearance_switches(),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.tap("appearance::light").await?;
                d.sample_pixel("drawn", 0.5, 0.5, "#ff0000").await?;
                d.tap("appearance::dark").await?;
                d.sample_pixel("drawn", 0.5, 0.5, "#0000ff").await?;
                d.tap("appearance::system").await
            })
    }

    /// A surface's border is drawn over its content: a child that fills the surface edge to
    /// edge never covers the border (docs/surfaces.md "Rules").
    #[day_macros::test(day_core)]
    fn surface_border_over_content() -> Case {
        Case::new()
            .proves_modifier("surface")
            .proves_modifier("border")
            .requires(Cap::Snapshot)
            .page(|| {
                rectangle()
                    .fill(Color::rgb(0.0, 0.0, 1.0))
                    .frame(80.0, 80.0)
                    .surface(
                        Surface::new()
                            .corner_radius(12.0)
                            .border(Color::rgb(1.0, 0.0, 0.0), 8.0),
                    )
                    .id(day_core::id!(framed))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("framed", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("framed", 0.5, 0.04, "#ff0000").await?;
                d.sample_pixel("framed", 0.04, 0.5, "#ff0000").await?;
                d.sample_pixel("framed", 0.5, 0.96, "#ff0000").await
            })
    }

    /// A surface takes a radial and a conic paint like a shape does: the radial gradient's
    /// first stop at the middle and its last at the edge; the conic gradient's quarters
    /// clockwise from +x (docs/surfaces.md).
    #[day_macros::test(day_core)]
    fn surface_radial_and_conic_fill() -> Case {
        Case::new()
            .proves_modifier("surface")
            .requires(Cap::Snapshot)
            .page(|| {
                let quarter = |at: f64, c: Color| [(at, c), (at + 0.25, c)];
                let conic = [
                    quarter(0.0, Color::rgb(1.0, 0.0, 0.0)),
                    quarter(0.25, Color::rgb(0.0, 1.0, 0.0)),
                    quarter(0.5, Color::rgb(0.0, 0.0, 1.0)),
                    quarter(0.75, Color::rgb(1.0, 1.0, 0.0)),
                ]
                .concat();
                row((
                    label("")
                        .frame(80.0, 80.0)
                        .surface(Surface::new().fill(day_spec::RadialGradient::new(
                            day_spec::UnitPoint::CENTER,
                            0.5,
                            [
                                (0.0, Color::rgb(1.0, 0.0, 0.0)),
                                (0.2, Color::rgb(1.0, 0.0, 0.0)),
                                (0.8, Color::rgb(0.0, 0.0, 1.0)),
                                (1.0, Color::rgb(0.0, 0.0, 1.0)),
                            ],
                        )))
                        .id(day_core::id!(radial)),
                    label("")
                        .frame(80.0, 80.0)
                        .background(day_spec::ConicGradient::centered(
                            day_spec::Angle::ZERO,
                            conic,
                        ))
                        .id(day_core::id!(conic)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("radial", 0.5, 0.5, "#ff0000").await?;
                d.sample_pixel("radial", 0.5, 0.97, "#0000ff").await?;
                // Clockwise from +x on screen: right-below red, left-below green, left-above
                // blue, right-above yellow.
                d.sample_pixel("conic", 0.8, 0.6, "#ff0000").await?;
                d.sample_pixel("conic", 0.2, 0.6, "#00ff00").await?;
                d.sample_pixel("conic", 0.2, 0.4, "#0000ff").await?;
                d.sample_pixel("conic", 0.8, 0.4, "#ffff00").await
            })
    }

    /// Corner radius, border and shadow follow their signals in place, and a capsule radius
    /// follows the frame as it resizes (docs/surfaces.md "Reactive surfaces").
    #[day_macros::test(day_core)]
    fn surface_follows_its_signals() -> Case {
        Case::new()
            .proves_modifier("corner_radius")
            .proves_modifier("border")
            .requires(Cap::Snapshot)
            .page(|| {
                let styled = Signal::new(false);
                let width = Signal::new(80.0);
                column((
                    zstack((
                        rectangle().fill(Color::WHITE),
                        label("")
                            .frame(80.0, 80.0)
                            .background(Color::rgb(0.0, 0.0, 1.0))
                            .border(Color::rgb(1.0, 0.0, 0.0), move || {
                                if styled.get() { 8.0 } else { 0.0 }
                            })
                            .corner_radius(move || if styled.get() { 30.0 } else { 0.0 }),
                    ))
                    .frame(80.0, 80.0)
                    .id(day_core::id!(styled)),
                    zstack((
                        rectangle().fill(Color::WHITE),
                        label("")
                            .frame(width, 40.0)
                            .background(Color::rgb(0.0, 0.0, 1.0))
                            .corner_radius(crate::Corner::CAPSULE),
                    ))
                    .frame(160.0, 40.0)
                    .id(day_core::id!(pill)),
                    button("Style")
                        .action(move || {
                            styled.set(true);
                            width.set(160.0);
                        })
                        .id(day_core::id!(restyle)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("styled", 0.03, 0.03, "#0000ff").await?;
                d.sample_pixel("styled", 0.5, 0.04, "#0000ff").await?;
                d.sample_pixel("pill", 0.02, 0.06, "#ffffff").await?;
                d.tap("restyle").await?;
                d.sample_pixel("styled", 0.03, 0.03, "#ffffff").await?;
                d.sample_pixel("styled", 0.5, 0.04, "#ff0000").await?;
                d.sample_pixel("styled", 0.5, 0.5, "#0000ff").await?;
                // The capsule's ends stay round at the new width.
                d.sample_pixel("pill", 0.01, 0.06, "#ffffff").await?;
                d.sample_pixel("pill", 0.99, 0.94, "#ffffff").await?;
                d.sample_pixel("pill", 0.5, 0.5, "#0000ff").await
            })
    }

    /// Size limits and an aspect ratio follow their signals: the piece is laid out again at
    /// the new limit, without being rebuilt (docs/layout.md).
    #[day_macros::test(day_core)]
    fn size_limits_follow_their_signals() -> Case {
        Case::new()
            .proves_modifier("max_width")
            .proves_modifier("min_height")
            .proves_modifier("aspect_ratio")
            .page(|| {
                let wide = Signal::new(false);
                column((
                    rectangle()
                        .fill(Color::rgb(0.2, 0.4, 0.9))
                        .max_width(move || if wide.get() { 120.0 } else { 60.0 })
                        .min_height(move || if wide.get() { 30.0 } else { 20.0 })
                        .frame(200.0, 10.0)
                        .id(day_core::id!(capped)),
                    rectangle()
                        .fill(Color::rgb(0.9, 0.4, 0.2))
                        .aspect_ratio(move || if wide.get() { 4.0 } else { 2.0 })
                        .width(80.0)
                        .id(day_core::id!(ratio)),
                    button("Widen")
                        .action(move || wide.set(true))
                        .id(day_core::id!(widen)),
                ))
                .spacing(8.0)
            })
            .drive(|d: Drive| async move {
                d.assert_size("capped", 60.0, 20.0).await?;
                d.assert_size("ratio", 80.0, 40.0).await?;
                d.tap("widen").await?;
                d.assert_size("capped", 120.0, 30.0).await?;
                d.assert_size("ratio", 80.0, 20.0).await
            })
    }

    /// An `Angle` means the same turn in either unit: a quarter turn given in radians turns a
    /// two-color bar exactly as `Angle::deg(90.0)` does (`rotation_turns_clockwise`).
    #[day_macros::test(day_core)]
    fn rotation_takes_an_angle_in_either_unit() -> Case {
        Case::new()
            .proves_modifier("rotation")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    shape_group([
                        rectangle()
                            .fill(Color::rgb(1.0, 0.0, 0.0))
                            .at(0.0, 0.0, 0.5, 1.0),
                        rectangle()
                            .fill(Color::rgb(0.0, 0.0, 1.0))
                            .at(0.5, 0.0, 0.5, 1.0),
                    ])
                    .frame(80.0, 40.0)
                    .rotation(day_spec::Angle::rad(std::f64::consts::FRAC_PI_2)),
                ))
                .id(day_core::id!(ground))
                .frame(120.0, 120.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.4, "#ff0000").await?;
                d.sample_pixel("ground", 0.5, 0.6, "#0000ff").await
            })
    }

    /// Opacity zero leaves what is below showing.
    #[day_macros::test(day_core)]
    fn opacity_hides() -> Case {
        Case::new()
            .proves_duty("set_opacity")
            .proves_modifier("opacity")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    rectangle().fill(Color::rgb(1.0, 0.0, 0.0)).opacity(0.0),
                ))
                .id(day_core::id!(ground))
                .frame(80.0, 40.0)
            })
            .drive(|d: Drive| async move { d.sample_pixel("ground", 0.5, 0.5, "#ffffff").await })
    }

    /// A translation moves what is drawn without moving the layout.
    #[day_macros::test(day_core)]
    fn translation_moves_drawing() -> Case {
        Case::new()
            .proves_modifier("translation")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    rectangle()
                        .fill(Color::rgb(0.0, 0.0, 1.0))
                        .frame(40.0, 40.0)
                        .translation(40.0, 0.0),
                ))
                .align(day_core::Alignment::Leading)
                .id(day_core::id!(ground))
                .frame(120.0, 40.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.1, 0.5, "#ffffff").await
            })
    }

    /// A translation moves a plain view's drawing too, not only a canvas's.
    #[day_macros::test(day_core)]
    fn translation_moves_view() -> Case {
        Case::new()
            .proves_modifier("translation")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    label("")
                        .frame(40.0, 40.0)
                        .background(Color::rgb(0.0, 0.0, 1.0))
                        .translation(40.0, 0.0),
                ))
                .align(day_core::Alignment::Leading)
                .id(day_core::id!(ground))
                .frame(120.0, 40.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.1, 0.5, "#ffffff").await
            })
    }

    /// A scale grows the drawing about its center without moving the layout.
    #[day_macros::test(day_core)]
    fn scale_grows_drawing() -> Case {
        Case::new()
            .proves_modifier("scale")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    label("")
                        .frame(40.0, 40.0)
                        .background(Color::rgb(0.0, 0.0, 1.0))
                        .scale(2.0),
                ))
                .id(day_core::id!(ground))
                .frame(120.0, 80.0)
            })
            .drive(|d: Drive| async move {
                // Unscaled, the square spans x 40..80 of 120; scaled twice it spans 20..100.
                d.sample_pixel("ground", 0.25, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.08, 0.5, "#ffffff").await
            })
    }

    /// A rotation turns the drawing clockwise (y points down): a bar red on the left and blue
    /// on the right, turned 90°, is red on top and blue below.
    #[day_macros::test(day_core)]
    fn rotation_turns_clockwise() -> Case {
        Case::new()
            .proves_modifier("rotation")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    shape_group([
                        rectangle()
                            .fill(Color::rgb(1.0, 0.0, 0.0))
                            .at(0.0, 0.0, 0.5, 1.0),
                        rectangle()
                            .fill(Color::rgb(0.0, 0.0, 1.0))
                            .at(0.5, 0.0, 0.5, 1.0),
                    ])
                    .frame(80.0, 40.0)
                    .rotation(day_spec::Angle::QUARTER),
                ))
                .id(day_core::id!(ground))
                .frame(120.0, 120.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.4, "#ff0000").await?;
                d.sample_pixel("ground", 0.5, 0.6, "#0000ff").await
            })
    }

    // ---- input: every op injects Day's own event, the stream a native recognizer delivers,
    // so these prove the routing and the handler, not that the platform's recognizer fires
    // (docs/testing.md "Known gaps").

    /// A plain target for gestures: a filled rectangle of a known size.
    fn pad(id: day_core::ElementId) -> Decorated<ShapePiece> {
        rectangle()
            .fill(Color::rgb(0.2, 0.4, 0.9))
            .frame(120.0, 80.0)
            .id(id)
    }

    /// A tap runs `.on_tap`'s work, once per tap.
    #[day_macros::test(day_core)]
    fn on_tap_runs() -> Case {
        Case::new()
            .proves_modifier("on_tap")
            .page(|| {
                let taps = Signal::new(0i64);
                column((
                    label(move || format!("taps {}", taps.get())).id(day_core::id!(taps)),
                    label("Tap me")
                        .padding(12.0)
                        .on_tap(move || taps.update(|n| *n += 1))
                        .id(day_core::id!(target)),
                ))
            })
            .drive(|d: Drive| async move {
                d.tap("target").await?;
                d.tap("target").await?;
                d.assert_text("taps", "taps 2").await
            })
    }

    /// `.on_tap_at` hears where in the element the tap landed.
    #[day_macros::test(day_core)]
    fn on_tap_at_reports_point() -> Case {
        Case::new()
            .proves_modifier("on_tap_at")
            .page(|| {
                let at = Signal::new(String::from("none"));
                column((
                    label(move || format!("at {}", at.get())).id(day_core::id!(at)),
                    rectangle()
                        .fill(Color::rgb(0.2, 0.4, 0.9))
                        .frame(120.0, 80.0)
                        .on_tap_at(move |p| at.set(format!("{:.0},{:.0}", p.x, p.y)))
                        .id(day_core::id!(target)),
                ))
            })
            .drive(|d: Drive| async move {
                d.tap_at("target", 30.0, 20.0).await?;
                d.assert_text("at", "at 30,20").await
            })
    }

    /// A drag reports its phases, and its translation from where it began.
    #[day_macros::test(day_core)]
    fn on_drag_reports_translation() -> Case {
        Case::new()
            .proves_duty("enable_gesture")
            .proves_modifier("on_drag")
            .page(|| {
                let state = Signal::new(String::from("idle"));
                column((
                    label(move || state.get()).id(day_core::id!(state)),
                    pad(day_core::id!(target)).on_drag(move |d: Drag| {
                        state.set(match d.phase {
                            DragPhase::Began => "began".into(),
                            DragPhase::Changed => "moving".into(),
                            DragPhase::Ended => {
                                format!("moved {:.0},{:.0}", d.translation.x, d.translation.y)
                            }
                        });
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.drag("target", (10.0, 10.0), (50.0, 30.0)).await?;
                d.assert_text("state", "moved 40,20").await
            })
    }

    /// Hover reports the pointer's place while it is over the element, and `None` on leaving.
    #[day_macros::test(day_core)]
    fn on_hover_enters_and_leaves() -> Case {
        Case::new()
            .proves_modifier("on_hover")
            .page(|| {
                let state = Signal::new(String::from("out"));
                column((
                    label(move || state.get()).id(day_core::id!(state)),
                    pad(day_core::id!(target)).on_hover(move |p| {
                        state.set(match p {
                            Some(p) => format!("over {:.0},{:.0}", p.x, p.y),
                            None => "out".into(),
                        });
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.hover("target", Some((5.0, 6.0))).await?;
                d.assert_text("state", "over 5,6").await?;
                d.hover_leave("target").await?;
                d.assert_text("state", "out").await
            })
    }

    /// A pan delivers incremental deltas that add up to the whole movement.
    #[day_macros::test(day_core)]
    fn on_pan_adds_deltas() -> Case {
        Case::new()
            .proves_modifier("on_pan")
            .page(|| {
                let total = Signal::new((0.0f64, 0.0f64));
                column((
                    label(move || {
                        let (x, y) = total.get();
                        format!("pan {x:.0},{y:.0}")
                    })
                    .id(day_core::id!(total)),
                    pad(day_core::id!(target)).on_pan(move |p: Pan| {
                        total.update(|(x, y)| {
                            *x += p.delta.x;
                            *y += p.delta.y;
                        });
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.pan("target", 40.0, -20.0).await?;
                d.assert_text("total", "pan 40,-20").await
            })
    }

    /// A pinch's scale is cumulative from where it began; its end carries the final scale.
    #[day_macros::test(day_core)]
    fn on_pinch_reports_scale() -> Case {
        Case::new()
            .proves_modifier("on_pinch")
            .page(|| {
                let scale = Signal::new(String::from("none"));
                column((
                    label(move || format!("scale {}", scale.get())).id(day_core::id!(scale)),
                    pad(day_core::id!(target)).on_pinch(move |p: Pinch| {
                        if p.phase == DragPhase::Ended {
                            scale.set(format!("{:.1}", p.scale));
                        }
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.pinch("target", 2.5).await?;
                d.assert_text("scale", "scale 2.5").await
            })
    }

    /// A rotation is cumulative from where it began, in radians; its end carries the final
    /// angle.
    #[day_macros::test(day_core)]
    fn on_rotate_reports_rotation() -> Case {
        Case::new()
            .proves_modifier("on_rotate")
            .page(|| {
                let angle = Signal::new(String::from("none"));
                column((
                    label(move || format!("rotation {}", angle.get())).id(day_core::id!(angle)),
                    pad(day_core::id!(target)).on_rotate(move |r: Rotate| {
                        if r.phase == DragPhase::Ended {
                            angle.set(format!("{:.2}", r.rotation.radians()));
                        }
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.rotate("target", 0.75).await?;
                d.assert_text("angle", "rotation 0.75").await
            })
    }

    /// A focusable element takes focus, and the keys pressed while it holds it reach `.on_key`.
    #[day_macros::test(day_core)]
    fn on_key_reaches_focused() -> Case {
        Case::new()
            .proves_duty("focus")
            .proves_duty("set_focusable")
            .proves_modifier("on_key")
            .proves_modifier("focusable")
            .page(|| {
                let last = Signal::new(String::from("none"));
                column((
                    label(move || format!("key {}", last.get())).id(day_core::id!(last)),
                    pad(day_core::id!(target))
                        .focusable()
                        .on_key(move |k| last.set(k.key.name())),
                ))
            })
            .drive(|d: Drive| async move {
                // Focus, then a key to whatever holds focus: the route a real press takes.
                d.focus("target").await?;
                d.assert_focused("target", true).await?;
                d.key(None, "ArrowRight").await?;
                d.assert_text("last", "key ArrowRight").await
            })
    }

    /// A drag's release carries the velocity the platform's recognizer measured, and the
    /// press point is `location - translation` in every phase.
    #[day_macros::test(day_core)]
    fn on_drag_reports_release_velocity() -> Case {
        Case::new()
            .proves_modifier("on_drag")
            .page(|| {
                let state = Signal::new(String::from("idle"));
                column((
                    label(move || state.get()).id(day_core::id!(state)),
                    pad(day_core::id!(target)).on_drag(move |d: Drag| {
                        if d.phase == DragPhase::Ended {
                            state.set(format!(
                                "from {:.0},{:.0} at {:.0},{:.0} speed {:.0}",
                                d.start().x,
                                d.start().y,
                                d.velocity.x,
                                d.velocity.y,
                                d.speed()
                            ));
                        }
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.fling("target", (10.0, 20.0), (70.0, 20.0), (600.0, -800.0))
                    .await?;
                d.assert_text("state", "from 10,20 at 600,-800 speed 1000")
                    .await?;
                // A drag whose samples measure nothing reports no velocity, not a stale one.
                d.drag("target", (10.0, 20.0), (70.0, 20.0)).await?;
                d.assert_text("state", "from 10,20 at 0,0 speed 0").await
            })
    }

    /// A drag declared along one axis hears the drags that set off along it, and none of the
    /// ones that set off across it.
    #[day_macros::test(day_core)]
    fn on_drag_along_withholds_cross_axis() -> Case {
        Case::new()
            .proves_modifier("on_drag_along")
            .proves_duty("set_drag_axis")
            .page(|| {
                let state = Signal::new(String::from("none"));
                let events = Signal::new(0i64);
                column((
                    label(move || format!("{} after {}", state.get(), events.get()))
                        .id(day_core::id!(state)),
                    pad(day_core::id!(target)).on_drag_along(
                        day_spec::DragAxis::Horizontal,
                        move |d: Drag| {
                            events.update(|n| *n += 1);
                            if d.phase == DragPhase::Ended {
                                state.set(format!("moved {:.0}", d.translation.x));
                            }
                        },
                    ),
                ))
            })
            .drive(|d: Drive| async move {
                d.drag("target", (60.0, 10.0), (60.0, 70.0)).await?;
                d.assert_text("state", "none after 0").await?;
                d.drag("target", (10.0, 40.0), (90.0, 44.0)).await?;
                // Began, four samples and Ended: the whole drag, from its first event.
                d.assert_text("state", "moved 80 after 6").await
            })
    }

    /// Synthetic help, popup content, and status feedback exercise a single mounted trigger.
    #[day_macros::test(day_core)]
    fn contextual_feedback_lifecycle() -> Case {
        Case::new()
            .proves_modifier("popover")
            .proves_modifier("tooltip")
            .proves_duty("open_popover")
            .proves_duty("close_popover")
            .proves_duty("set_tooltip")
            .page(|| {
                let open = Signal::new(false);
                let saved = Signal::new(false);
                let cleared = Signal::new(false);
                let help = move || {
                    if cleared.get() {
                        ""
                    } else if saved.get() {
                        "Saved fixture"
                    } else {
                        "Edit fixture"
                    }
                };
                column((
                    button("Open fixture")
                        .id(day_core::id!(feedback_anchor))
                        .action(move || open.set(true))
                        .tooltip(help)
                        .popover(open, move || {
                            button("Apply fixture")
                                .id(day_core::id!(feedback_apply))
                                .action(move || {
                                    saved.set(true);
                                    open.set(false);
                                    crate::toast("Saved fixture");
                                })
                                .padding(10.0)
                        }),
                    label(help).id(day_core::id!(feedback_state)),
                    button("Clear fixture hint")
                        .id(day_core::id!(feedback_clear))
                        .action(move || cleared.set(true)),
                ))
            })
            .drive(|d: Drive| async move {
                let tooltip = || {
                    day_core::with_tree(|t| {
                        t.find_by_id("feedback_anchor")
                            .and_then(|n| t.node_probe(n))
                            .and_then(|p| p.tooltip)
                    })
                };
                d.check_eq(tooltip().as_deref(), Some("Edit fixture"))?;
                d.assert_missing("feedback_apply").await?;
                for _ in 0..2 {
                    d.tap("feedback_anchor").await?;
                    d.assert_visible("feedback_apply").await?;
                    d.tap("feedback_apply").await?;
                    d.assert_missing("feedback_apply").await?;
                    d.assert_text("feedback_state", "Saved fixture").await?;
                    d.check_eq(tooltip().as_deref(), Some("Saved fixture"))?;
                }
                d.tap("feedback_clear").await?;
                d.check_eq(tooltip(), None)
            })
    }

    /// Expanding and collapsing content resizes the mounted panel and preserves its controls.
    #[day_macros::test(day_core)]
    fn contextual_feedback_resizes() -> Case {
        Case::new()
            .proves_duty("resize_popover")
            .page(|| {
                let open = Signal::new(false);
                button("Open fixture")
                    .id(day_core::id!(size_anchor))
                    .action(move || open.set(true))
                    .popover(open, || {
                        let expanded = Signal::new(false);
                        column((
                            // XAML applies its button template on load. Give this fixture a
                            // stable baseline so it measures only the conditional row.
                            button("Resize fixture")
                                .height(44.0)
                                .id(day_core::id!(size_toggle))
                                .action(move || expanded.update(|v| *v = !*v)),
                            when(
                                move || expanded.get(),
                                || label("Expanded fixture").height(64.0),
                            ),
                        ))
                    })
            })
            .drive(|d: Drive| async move {
                let layout = || {
                    day_core::with_tree(|tree| {
                        let control = tree.find_by_id("size_toggle")?;
                        let mut root = control;
                        while let Some(parent) = tree.parent_of(root) {
                            root = parent;
                        }
                        Some((control, tree.node_frame(root)?.size.height))
                    })
                };
                d.tap("size_anchor").await?;
                d.wait_idle().await?;
                let initial = layout()
                    .ok_or_else(|| day_core::conformance::Fail("missing initial panel".into()))?;
                d.tap("size_toggle").await?;
                d.wait_idle().await?;
                let expanded = layout()
                    .ok_or_else(|| day_core::conformance::Fail("missing expanded panel".into()))?;
                d.check_eq(expanded.0, initial.0)?;
                d.check(
                    expanded.1 >= initial.1 + 63.0,
                    "panel grows to fit its new content",
                )?;
                d.tap("size_toggle").await?;
                d.wait_idle().await?;
                d.check_eq(layout(), Some(initial))
            })
    }

    /// The trigger needs a native parent even when the binding starts open.
    #[day_macros::test(day_core)]
    fn contextual_feedback_initially_open() -> Case {
        Case::new()
            .page(|| {
                let open = Signal::new(true);
                button("Fixture trigger").popover(open, move || {
                    button("Close fixture")
                        .id(day_core::id!(initial_feedback_close))
                        .action(move || open.set(false))
                })
            })
            .drive(|d: Drive| async move {
                d.wait_idle().await?;
                d.assert_visible("initial_feedback_close").await?;
                d.tap("initial_feedback_close").await?;
                d.assert_missing("initial_feedback_close").await
            })
    }

    /// A long press reports the press point, and a piece with a tap handler too hears the
    /// long press alone.
    #[day_macros::test(day_core)]
    fn on_long_press_fires() -> Case {
        Case::new()
            .proves_modifier("on_long_press")
            .proves_duty("gesture_support")
            .page(|| {
                let held = Signal::new(String::from("none"));
                let taps = Signal::new(0i64);
                column((
                    label(move || format!("held {} taps {}", held.get(), taps.get()))
                        .id(day_core::id!(state)),
                    pad(day_core::id!(target))
                        .on_tap(move || taps.update(|n| *n += 1))
                        .on_long_press(move |at| held.set(format!("{:.0},{:.0}", at.x, at.y))),
                ))
            })
            .drive(|d: Drive| async move {
                d.long_press("target").await?;
                d.assert_text("state", "held 60,40 taps 0").await
            })
    }

    /// A double tap runs its handler and not the single tap's.
    #[day_macros::test(day_core)]
    fn on_double_tap_fires() -> Case {
        Case::new()
            .proves_modifier("on_double_tap")
            .page(|| {
                let doubles = Signal::new(0i64);
                let taps = Signal::new(0i64);
                column((
                    label(move || format!("doubles {} taps {}", doubles.get(), taps.get()))
                        .id(day_core::id!(state)),
                    pad(day_core::id!(target))
                        .on_tap(move || taps.update(|n| *n += 1))
                        .on_double_tap(move || doubles.update(|n| *n += 1)),
                ))
            })
            .drive(|d: Drive| async move {
                d.double_tap("target").await?;
                d.assert_text("state", "doubles 1 taps 0").await
            })
    }

    /// A double tap tells `.on_double_tap_at` where it landed, in the piece's coordinates.
    #[day_macros::test(day_core)]
    fn on_double_tap_at_reports_the_point() -> Case {
        Case::new()
            .proves_modifier("on_double_tap_at")
            .page(|| {
                let at = Signal::new(String::from("none"));
                column((
                    label(move || format!("double at {}", at.get())).id(day_core::id!(state)),
                    pad(day_core::id!(target))
                        .on_double_tap_at(move |p| at.set(format!("{:.0},{:.0}", p.x, p.y))),
                ))
            })
            .drive(|d: Drive| async move {
                d.double_tap("target").await?;
                d.assert_text("state", "double at 60,40").await
            })
    }

    /// Press-in and press-out reach `.on_pressed`, one of each per press.
    #[day_macros::test(day_core)]
    fn on_pressed_follows_press() -> Case {
        Case::new()
            .proves_modifier("on_pressed")
            .page(|| {
                let down = Signal::new(false);
                let changes = Signal::new(0i64);
                column((
                    label(move || {
                        let state = if down.get() { "down" } else { "up" };
                        format!("{state} after {}", changes.get())
                    })
                    .id(day_core::id!(state)),
                    pad(day_core::id!(target)).on_pressed(move |pressed| {
                        down.set(pressed);
                        changes.update(|n| *n += 1);
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.press("target", true).await?;
                d.assert_text("state", "down after 1").await?;
                d.press("target", false).await?;
                d.assert_text("state", "up after 2").await
            })
    }

    /// Raw contacts arrive as a list, each with its own id, and say when they lift.
    #[day_macros::test(day_core)]
    fn on_touches_reports_contacts() -> Case {
        Case::new()
            .proves_modifier("on_touches")
            .page(|| {
                let state = Signal::new(String::from("none"));
                column((
                    label(move || state.get()).id(day_core::id!(state)),
                    pad(day_core::id!(target)).on_touches(move |touches: &[day_spec::Touch]| {
                        let down = touches.iter().filter(|t| t.is_down()).count();
                        let ids: Vec<String> = touches.iter().map(|t| t.id.to_string()).collect();
                        let last = touches.last().map_or(Point::ZERO, |t| t.location);
                        state.set(format!(
                            "{down} down ids {} last {:.0},{:.0}",
                            ids.join("+"),
                            last.x,
                            last.y
                        ));
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                let points = [(10.0, 10.0), (50.0, 30.0)];
                d.touches("target", &points, day_spec::TouchPhase::Began)
                    .await?;
                d.assert_text("state", "2 down ids 0+1 last 50,30").await?;
                d.touches("target", &points, day_spec::TouchPhase::Ended)
                    .await?;
                d.assert_text("state", "0 down ids 0+1 last 50,30").await
            })
    }

    /// A hit slop changes neither the piece's frame nor its neighbor's, and the piece still
    /// takes its tap. That a press in the slop area lands on it is the platform's hit test,
    /// which injected input does not exercise (docs/gestures.md "Hit slop").
    #[day_macros::test(day_core)]
    fn hit_slop_leaves_layout_alone() -> Case {
        Case::new()
            .proves_modifier("hit_slop")
            .proves_duty("set_hit_slop")
            .proves_cap(Cap::HitSlop)
            .page(|| {
                let taps = Signal::new(0i64);
                column((
                    label(move || format!("taps {}", taps.get())).id(day_core::id!(taps)),
                    row((
                        rectangle()
                            .fill(Color::rgb(0.9, 0.2, 0.2))
                            .frame(12.0, 12.0)
                            .hit_slop(30.0)
                            .on_tap(move || taps.update(|n| *n += 1))
                            .id(day_core::id!(dot)),
                        rectangle()
                            .fill(Color::rgb(0.2, 0.4, 0.9))
                            .frame(12.0, 12.0)
                            .id(day_core::id!(next)),
                    ))
                    .spacing(4.0),
                ))
                .align(HAlign::Leading)
            })
            .drive(|d: Drive| async move {
                d.assert_frame("dot", size(12.0, 12.0)).await?;
                d.assert_frame(
                    "next",
                    FrameExpect {
                        x: Some(16.0),
                        y: Some(0.0),
                        relative_to: Some("dot".into()),
                        ..FrameExpect::default()
                    },
                )
                .await?;
                d.tap("dot").await?;
                d.assert_text("taps", "taps 1").await
            })
    }

    /// An interaction state follows hover, press and focus, each on its own.
    #[day_macros::test(day_core)]
    fn interaction_state_tracks_hover_press_focus() -> Case {
        Case::new()
            .proves_modifier("interaction")
            .page(|| {
                let state = InteractionState::new();
                column((
                    label(move || {
                        format!(
                            "hovered {} pressed {} focused {}",
                            state.hovered(),
                            state.pressed(),
                            state.focused()
                        )
                    })
                    .id(day_core::id!(state)),
                    pad(day_core::id!(target)).focusable().interaction(state),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("state", "hovered false pressed false focused false")
                    .await?;
                d.hover("target", None).await?;
                d.assert_text("state", "hovered true pressed false focused false")
                    .await?;
                d.press("target", true).await?;
                d.assert_text("state", "hovered true pressed true focused false")
                    .await?;
                d.press("target", false).await?;
                d.hover_leave("target").await?;
                d.assert_text("state", "hovered false pressed false focused false")
                    .await?;
                d.focus("target").await?;
                d.assert_text("state", "hovered false pressed false focused true")
                    .await
            })
    }

    /// A reactive modifier reads the interaction state: the background follows the press.
    #[day_macros::test(day_core)]
    fn interaction_state_drives_background() -> Case {
        Case::new()
            .proves_modifier("interaction")
            .requires(Cap::Snapshot)
            .page(|| {
                let state = InteractionState::new();
                label("")
                    .frame(80.0, 40.0)
                    .interaction(state)
                    .background(move || {
                        if state.pressed() {
                            Color::rgb(1.0, 0.0, 0.0)
                        } else {
                            Color::rgb(0.0, 0.0, 1.0)
                        }
                    })
                    .id(day_core::id!(card))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("card", 0.5, 0.5, "#0000ff").await?;
                d.press("card", true).await?;
                d.sample_pixel("card", 0.5, 0.5, "#ff0000").await?;
                d.press("card", false).await?;
                d.sample_pixel("card", 0.5, 0.5, "#0000ff").await
            })
    }

    /// `.enabled(false)` disables the controls in a subtree natively and stops its gestures;
    /// enabling it again restores both.
    #[day_macros::test(day_core)]
    fn enabled_disables_subtree() -> Case {
        Case::new()
            .proves_modifier("enabled")
            .proves_duty("set_subtree_enabled")
            .page(|| {
                let on = Signal::new(true);
                let presses = Signal::new(0i64);
                let drags = Signal::new(0i64);
                let pressed = Signal::new(false);
                column((
                    label(move || {
                        format!(
                            "presses {} drags {} pressed {}",
                            presses.get(),
                            drags.get(),
                            pressed.get()
                        )
                    })
                    .id(day_core::id!(state)),
                    button("Flip")
                        .action(move || on.update(|v| *v = !*v))
                        .id(day_core::id!(flip)),
                    column((
                        button("Inner")
                            .action(move || presses.update(|n| *n += 1))
                            .id(day_core::id!(inner)),
                        pad(day_core::id!(target))
                            .on_pressed(move |down| pressed.set(down))
                            .on_drag(move |d: Drag| {
                                if d.phase == DragPhase::Ended {
                                    drags.update(|n| *n += 1);
                                }
                            }),
                    ))
                    .enabled(on),
                ))
            })
            .drive(|d: Drive| async move {
                let native = |enabled| NativeExpect {
                    enabled: Some(enabled),
                    ..NativeExpect::default()
                };
                d.assert_enabled("inner", true).await?;
                d.drag("target", (10.0, 10.0), (60.0, 10.0)).await?;
                d.assert_text("state", "presses 0 drags 1 pressed false")
                    .await?;
                // Disabled while pressed: the press ends, and nothing more arrives.
                d.press("target", true).await?;
                d.assert_text("state", "presses 0 drags 1 pressed true")
                    .await?;
                d.tap("flip").await?;
                d.assert_text("state", "presses 0 drags 1 pressed false")
                    .await?;
                d.assert_enabled("inner", false).await?;
                d.assert_native("inner", native(false)).await?;
                d.drag("target", (10.0, 10.0), (60.0, 10.0)).await?;
                d.press("target", true).await?;
                d.assert_text("state", "presses 0 drags 1 pressed false")
                    .await?;
                d.tap("flip").await?;
                d.assert_enabled("inner", true).await?;
                d.assert_native("inner", native(true)).await?;
                d.tap("inner").await?;
                d.drag("target", (10.0, 10.0), (60.0, 10.0)).await?;
                d.assert_text("state", "presses 1 drags 2 pressed false")
                    .await
            })
    }

    /// A key reports each phase of its press: down and repeat reach `.on_key`, the release
    /// reaches `.on_key_up`, and the key is typed.
    #[day_macros::test(day_core)]
    fn key_phases_reach_their_handlers() -> Case {
        Case::new()
            .proves_modifier("on_key_up")
            .proves_cap(Cap::KeyPhases)
            .page(|| {
                let counts = Signal::new((0i64, 0i64, 0i64));
                column((
                    label(move || {
                        let (down, repeat, up) = counts.get();
                        format!("down {down} repeat {repeat} up {up}")
                    })
                    .id(day_core::id!(state)),
                    pad(day_core::id!(target))
                        .focusable()
                        .on_key(move |k| {
                            if k.key == day_spec::Key::ArrowRight {
                                counts.update(|c| {
                                    if k.is_repeat() {
                                        c.1 += 1;
                                    } else {
                                        c.0 += 1;
                                    }
                                });
                            }
                        })
                        .on_key_up(move |k| {
                            if k.key == day_spec::Key::ArrowRight && k.is_release() {
                                counts.update(|c| c.2 += 1);
                            }
                        }),
                ))
            })
            .drive(|d: Drive| async move {
                d.focus("target").await?;
                d.key(None, "ArrowRight").await?;
                d.assert_text("state", "down 1 repeat 0 up 0").await?;
                d.key_phase(None, "ArrowRight", day_spec::KeyPhase::Repeat)
                    .await?;
                d.key_phase(None, "ArrowRight", day_spec::KeyPhase::Repeat)
                    .await?;
                d.assert_text("state", "down 1 repeat 2 up 0").await?;
                d.key_phase(None, "ArrowRight", day_spec::KeyPhase::Up)
                    .await?;
                d.assert_text("state", "down 1 repeat 2 up 1").await
            })
    }

    /// A focusable piece joins the focus system: it takes focus, says so, hears keys while
    /// it holds it, and gives it up to the next control.
    #[day_macros::test(day_core)]
    fn focusable_piece_holds_and_yields_focus() -> Case {
        Case::new()
            .proves_cap(Cap::Focusable)
            .proves_duty("set_focusable")
            .requires(Cap::Focusable)
            .page(|| {
                let focused = Signal::new(false);
                let keys = Signal::new(0i64);
                let text = Signal::new(String::new());
                column((
                    label(move || format!("focused {} keys {}", focused.get(), keys.get()))
                        .id(day_core::id!(state)),
                    pad(day_core::id!(target))
                        .focusable()
                        .focused(focused)
                        .on_key(move |_| keys.update(|n| *n += 1)),
                    text_field(text).id(day_core::id!(field)),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("state", "focused false keys 0").await?;
                d.focus("target").await?;
                d.assert_focused("target", true).await?;
                d.assert_text("state", "focused true keys 0").await?;
                d.key(None, "ArrowDown").await?;
                d.assert_text("state", "focused true keys 1").await?;
                // Focus moves to the field: the piece reports the loss.
                d.focus("field").await?;
                d.assert_focused("field", true).await?;
                d.assert_focused("target", false).await?;
                d.assert_text("state", "focused false keys 1").await
            })
    }

    /// The input kind follows what the platform reports, and readers of it update.
    #[day_macros::test(day_core)]
    fn primary_pointer_follows_input() -> Case {
        Case::new()
            .proves_duty("primary_pointer")
            .page(|| label(move || day_core::primary_pointer().name()).id(day_core::id!(kind)))
            .drive(|d: Drive| async move {
                let before = untrack(day_core::primary_pointer);
                d.pointer(day_spec::PointerKind::Touch).await?;
                d.assert_text("kind", "touch").await?;
                d.pointer(day_spec::PointerKind::Stylus).await?;
                d.assert_text("kind", "stylus").await?;
                d.pointer(day_spec::PointerKind::Mouse).await?;
                d.assert_text("kind", "mouse").await?;
                // Leave the app as the platform had it.
                d.pointer(before).await
            })
    }

    /// A drop target hears the drag come over it, leave, and drop, and each hover ends once.
    #[day_macros::test(day_core)]
    fn drop_target_reports_enter_leave_drop() -> Case {
        Case::new()
            .proves_modifier("drop_target")
            .proves_duty("set_drop_target")
            .page(|| {
                let hovering = Signal::new(0i64);
                let got = Signal::new(String::from("nothing"));
                column((
                    label(move || format!("hovering {} got {}", hovering.get(), got.get()))
                        .id(day_core::id!(state)),
                    pad(day_core::id!(zone)).drop_target(
                        DropTarget::new(["text/plain"])
                            .on_enter(move |_| hovering.update(|n| *n += 1))
                            .on_leave(move || hovering.update(|n| *n -= 1))
                            .on_drop(move |drop| {
                                let text = drop
                                    .items
                                    .first()
                                    .and_then(|item| item.get("text/plain"))
                                    .and_then(|bytes| std::str::from_utf8(bytes).ok());
                                got.set(text.unwrap_or("?").to_owned());
                                text.is_some()
                            }),
                    ),
                ))
            })
            .drive(|d: Drive| async move {
                d.drag_enter("zone", "hello").await?;
                // The platform asks again as the drag moves: still one hover.
                d.drag_enter("zone", "hello").await?;
                d.assert_text("state", "hovering 1 got nothing").await?;
                d.drag_leave("zone").await?;
                d.assert_text("state", "hovering 0 got nothing").await?;
                d.drop_text("zone", "hello").await?;
                d.assert_text("state", "hovering 0 got hello").await
            })
    }

    /// `.on_files_dropped` hands over the local paths of the dropped files.
    #[day_macros::test(day_core)]
    fn on_files_dropped_receives_paths() -> Case {
        Case::new()
            .proves_modifier("on_files_dropped")
            .page(|| {
                let got = Signal::new(String::from("none"));
                column((
                    label(move || format!("files {}", got.get())).id(day_core::id!(state)),
                    pad(day_core::id!(zone)).on_files_dropped(move |paths| {
                        let names: Vec<String> = paths
                            .iter()
                            .filter_map(|p| p.file_name())
                            .map(|n| n.to_string_lossy().into_owned())
                            .collect();
                        got.set(names.join("+"));
                    }),
                ))
            })
            .drive(|d: Drive| async move {
                d.drop_files("zone", &["/work/first.txt", "/work/second%20draft.md"])
                    .await?;
                d.assert_text("state", "files first.txt+second draft.md")
                    .await
            })
    }

    /// A focus binding moves focus when the app sets it, and follows the field when it is
    /// focused natively.
    #[day_macros::test(day_core)]
    fn focused_binding_moves_focus() -> Case {
        Case::new()
            .proves_modifier("focused")
            .page(|| {
                let want = Signal::new(false);
                column((
                    label(move || format!("focused {}", want.get())).id(day_core::id!(state)),
                    button("Focus")
                        .action(move || want.set(true))
                        .id(day_core::id!(go)),
                    text_field(Signal::new(String::new()))
                        .focused(want)
                        .id(day_core::id!(field)),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_focused("field", false).await?;
                d.tap("go").await?;
                d.assert_focused("field", true).await?;
                d.assert_text("state", "focused true").await
            })
    }

    /// Destroy a focused input and its containing page without first moving focus to a
    /// button. Win32 GTK must detach the IME filter before the input loses its native root.
    #[day_macros::test(day_core)]
    fn focused_inputs_survive_page_removal() -> Case {
        Case::new()
            .proves_duty("focus")
            .page(|| {
                let shown = Signal::new(true);
                column((
                    button("Toggle page")
                        .action(move || shown.set(!shown.get()))
                        .id(day_core::id!(toggle)),
                    text_field(Signal::new(String::new())).id(day_core::id!(survivor)),
                    when(
                        move || shown.get(),
                        || {
                            column((
                                text_field(Signal::new(String::new())).id(day_core::id!(entry)),
                            ))
                            .background(Color::rgb(0.5, 0.5, 0.5))
                        },
                    ),
                ))
            })
            .drive(|d: Drive| async move {
                for _ in 0..4 {
                    d.focus("entry").await?;
                    d.assert_focused("entry", true).await?;
                    // Scripted activation doesn't move native focus to the button first.
                    d.tap("toggle").await?;
                    d.assert_missing("entry").await?;
                    d.wait_idle().await?;
                    d.tap("toggle").await?;
                }
                d.focus("survivor").await?;
                d.tap("toggle").await?;
                d.assert_focused("survivor", true).await?;
                d.input("survivor", "still editable").await?;
                d.assert_text("survivor", "still editable").await
            })
    }

    // ---- layout and the remaining modifiers -------------------------------------------------

    /// `.height` fixes the height and leaves the width to the layout.
    #[day_macros::test(day_core)]
    fn height_fixes_height() -> Case {
        Case::new()
            .proves_modifier("height")
            .page(|| {
                column((rectangle()
                    .fill(Color::rgb(1.0, 0.0, 0.0))
                    .id(day_core::id!(bar))
                    .height(37.0),))
                .width(100.0)
            })
            .drive(|d: Drive| async move {
                d.assert_frame(
                    "bar",
                    FrameExpect {
                        height: Some(37.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// `.grow(Dimension::Height)` takes the height its column has left, in a column and in a row alike.
    #[day_macros::test(day_core)]
    fn grow_vertical_fills_column() -> Case {
        Case::new()
            .proves_modifier("grow")
            .page(|| {
                row((
                    column((
                        rectangle()
                            .fill(Color::rgb(1.0, 0.0, 0.0))
                            .id(day_core::id!(top))
                            .frame(40.0, 20.0),
                        rectangle()
                            .fill(Color::rgb(0.0, 0.0, 1.0))
                            .id(day_core::id!(rest))
                            .width(40.0)
                            .grow(Dimension::Height),
                    ))
                    .spacing(0.0)
                    .height(120.0),
                    rectangle()
                        .fill(Color::rgb(0.0, 1.0, 0.0))
                        .id(day_core::id!(both))
                        .width(30.0)
                        .grow(Dimension::Height),
                ))
                .spacing(0.0)
                .align(VAlign::Top)
                .height(120.0)
            })
            .drive(|d: Drive| async move {
                d.assert_frame(
                    "rest",
                    FrameExpect {
                        height: Some(100.0),
                        ..Default::default()
                    },
                )
                .await?;
                d.assert_frame(
                    "both",
                    FrameExpect {
                        height: Some(120.0),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A grid cell spans the columns it asks for, and aligns within its cell as asked.
    #[day_macros::test(day_core)]
    fn grid_span_and_align() -> Case {
        Case::new()
            .proves_modifier("grid_span")
            .proves_modifier("grid_align")
            .page(|| {
                let cell = |id: day_core::ElementId, w: f64| {
                    rectangle()
                        .fill(Color::rgb(0.2, 0.4, 0.9))
                        .id(id)
                        .frame(w, 20.0)
                };
                grid((
                    grid_row((cell(day_core::id!(a), 40.0), cell(day_core::id!(b), 60.0))),
                    grid_row((cell(day_core::id!(wide), 30.0)
                        .grid_align(day_core::Alignment::TopTrailing)
                        .grid_span(2),)),
                ))
                .spacing(0.0)
                .align(day_core::Alignment::TopLeading)
            })
            .drive(|d: Drive| async move {
                // The spanning cell's rect is both columns, 100 wide; trailing puts its
                // 30-wide content at x 70 from the first column's start.
                d.assert_frame(
                    "wide",
                    FrameExpect {
                        x: Some(70.0),
                        relative_to: Some("a".into()),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// `.overlay` draws its piece centered over the content.
    #[day_macros::test(day_core)]
    fn overlay_centers() -> Case {
        Case::new()
            .proves_modifier("overlay")
            .requires(Cap::Snapshot)
            .page(|| {
                rectangle()
                    .fill(Color::rgb(1.0, 0.0, 0.0))
                    .frame(90.0, 90.0)
                    .overlay(
                        rectangle()
                            .fill(Color::rgb(0.0, 0.0, 1.0))
                            .frame(30.0, 30.0),
                    )
                    .id(day_core::id!(base))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("base", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("base", 0.1, 0.1, "#ff0000").await
            })
    }

    /// `.transform` takes a whole transform: here a translation and a scale together.
    #[day_macros::test(day_core)]
    fn transform_applies_whole() -> Case {
        Case::new()
            .proves_duty("set_transform")
            .proves_modifier("transform")
            .requires(Cap::Snapshot)
            .page(|| {
                zstack((
                    rectangle().fill(Color::WHITE),
                    label("")
                        .frame(20.0, 20.0)
                        .background(Color::rgb(0.0, 0.0, 1.0))
                        .transform(Transform {
                            tx: 30.0,
                            sx: 2.0,
                            sy: 2.0,
                            ..Transform::default()
                        }),
                ))
                .id(day_core::id!(ground))
                .frame(120.0, 60.0)
            })
            .drive(|d: Drive| async move {
                // Centered at x 60, moved to 90, scaled to 40 wide: 70..110 of 120.
                d.sample_pixel("ground", 0.75, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.5, 0.5, "#ffffff").await
            })
    }

    /// An implicit animation still lands on the final value: a square slid across by an
    /// animated translation ends where the translation says.
    #[day_macros::test(day_core)]
    fn animation_lands_on_final() -> Case {
        Case::new()
            .proves_modifier("animation")
            .requires(Cap::Snapshot)
            .page(|| {
                let moved = Signal::new(false);
                column((
                    button("Move")
                        .action(move || moved.set(true))
                        .id(day_core::id!(move_button)),
                    zstack((
                        rectangle().fill(Color::WHITE),
                        label("")
                            .frame(40.0, 40.0)
                            .background(Color::rgb(0.0, 0.0, 1.0))
                            .translation(move || if moved.get() { 80.0 } else { 0.0 }, 0.0)
                            .animation(day_spec::AnimSpec::linear(150)),
                    ))
                    .align(day_core::Alignment::Leading)
                    .id(day_core::id!(ground))
                    .frame(120.0, 40.0),
                ))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.1, 0.5, "#0000ff").await?;
                d.tap("move_button").await?;
                d.wait_idle().await?;
                d.pause(0.4).await?;
                d.sample_pixel("ground", 0.85, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.1, 0.5, "#ffffff").await
            })
    }

    /// Each way of naming an element finds it: a fixed id, a derived one, a keyed one.
    #[day_macros::test(day_core)]
    fn ids_name_elements() -> Case {
        Case::new()
            .proves_modifier("id")
            .proves_modifier("id_of")
            .page(|| {
                let n = Signal::new(1i64);
                column((
                    label("Fixed").id(day_core::id!(fixed)),
                    label("Derived").id_of(move || day_core::id!(derived, n.get())),
                    label("Keyed").id(day_core::id!(keyed, 7)),
                    button("Next")
                        .action(move || n.set(2))
                        .id(day_core::id!(next)),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("fixed", "Fixed").await?;
                d.assert_text("keyed:7", "Keyed").await?;
                d.assert_text("derived:1", "Derived").await?;
                d.tap("next").await?;
                d.assert_missing("derived:1").await?;
                d.assert_text("derived:2", "Derived").await
            })
    }

    /// `.tweak` runs with the realized node, and `.native_ref` holds it while the piece lives.
    #[day_macros::test(day_core)]
    fn tweak_and_native_ref_see_node() -> Case {
        Case::new()
            .proves_modifier("tweak")
            .proves_modifier("native_ref")
            .page(|| {
                let tweaked = Signal::new(false);
                let r = NativeRef::new();
                let held = r.clone();
                column((
                    label(move || format!("tweaked {}", tweaked.get())).id(day_core::id!(tweaked)),
                    label(move || format!("held {}", held.node().is_some()))
                        .id(day_core::id!(held)),
                    label("Target")
                        .tweak(move |_node| tweaked.set(true))
                        .native_ref(&r),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("tweaked", "tweaked true").await?;
                d.assert_text("held", "held true").await
            })
    }

    /// `.modifier` applies a reusable modifier: here, a closure that pads.
    #[day_macros::test(day_core)]
    fn modifier_applies() -> Case {
        Case::new()
            .proves_modifier("modifier")
            .page(|| {
                let padded = |content: AnyPiece| content.padding(10.0).any();
                column((label("Inner")
                    .id(day_core::id!(inner))
                    .modifier(padded)
                    .id(day_core::id!(outer)),))
                .align(HAlign::Leading)
            })
            .drive(|d: Drive| async move {
                d.assert_frame(
                    "inner",
                    FrameExpect {
                        x: Some(10.0),
                        y: Some(10.0),
                        relative_to: Some("outer".into()),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// An announcement reaches the screen reader (the toolkit's `announce`) and the run.
    #[day_macros::test(day_core)]
    fn announce_reaches_reader() -> Case {
        Case::new()
            .proves_duty("announce")
            .proves_cap(Cap::Announce)
            .page(|| {
                button("Save")
                    .action(|| day_core::announce("Saved"))
                    .id(day_core::id!(save))
            })
            .drive(|d: Drive| async move {
                d.tap("save").await?;
                d.assert_announced("Saved").await
            })
    }
}

#[cfg(test)]
mod transfer_tests {
    use super::*;
    use day_spec::transfer::*;

    #[test]
    fn transfer_callbacks_restore_owner_scope_and_reject_after_disposal() {
        let owner = Scope::child();
        let foreign = Scope::child();
        foreign.provide(22u32);
        let hovers = Rc::new(Cell::new(0i32));
        let leaves = Rc::new(Cell::new(0i32));
        let counts = (hovers.clone(), leaves.clone());
        let (source, target) = owner.enter(|| {
            owner.provide(11u32);
            let check = || assert_eq!(day_core::environment::<u32>(), Some(11));
            let (hovers, leaves) = counts;
            let entered = hovers.clone();
            (
                scoped_drag_source(Rc::new(move |_| {
                    check();
                    Some(Offer::default())
                })),
                scoped_drop_target(
                    Target::new(Vec::<String>::new())
                        .accept(move |_| {
                            check();
                            Operation::Copy
                        })
                        .on_enter(move |_| {
                            check();
                            entered.set(entered.get() + 1);
                        })
                        .on_leave(move || {
                            check();
                            hovers.set(hovers.get() - 1);
                            leaves.set(leaves.get() + 1);
                        })
                        .on_drop(move |_| {
                            check();
                            true
                        }),
                ),
            )
        });
        let location = || Location {
            position: day_spec::Point::new(40., 20.),
            types: vec![],
            allowed: vec![Operation::Copy],
            local: true,
        };
        foreign.enter(|| {
            assert!(source(day_spec::Point::new(0., 0.)).is_some());
            // The hover is entered once however often the platform asks, and left once: by
            // the session leaving, and again by the drop that ends the next hover.
            assert_eq!(target.proposal(&location()), Operation::Copy);
            assert_eq!(target.proposal(&location()), Operation::Copy);
            assert_eq!((hovers.get(), leaves.get()), (1, 0));
            assert!(target.is_hovering());
            target.leave();
            target.leave();
            assert_eq!((hovers.get(), leaves.get()), (0, 1));
            assert_eq!(target.proposal(&location()), Operation::Copy);
            assert!(target.deliver(location(), Offer::default()));
            assert_eq!((hovers.get(), leaves.get()), (0, 2));
            assert!(!target.is_hovering());
            assert_eq!(day_core::environment::<u32>(), Some(22));
            owner.dispose();
            assert!(source(day_spec::Point::new(0., 0.)).is_none());
            assert_eq!(target.proposal(&location()), Operation::None);
            assert!(!target.deliver(location(), Offer::default()));
            assert_eq!((hovers.get(), leaves.get()), (0, 2));
        });
        foreign.dispose();
    }
}
