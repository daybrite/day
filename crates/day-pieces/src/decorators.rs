// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `Decorate` — the chainable modifiers every piece inherits: padding and sizing, background and
//! corner radius, gestures (`on_tap`, drag), accessibility (`A11yBuilder`), and native-handle
//! capture (`NativeRef`) — plus the `Modifier` / `IntoInsets` supporting traits.
//!
//! Modifiers return [`Decorated<P>`], which keeps the decorated piece's OWN type, so a chain
//! never stops reaching that piece's builder methods (§5.2).

use std::cell::Cell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, bind};
use day_spec::props::*;
use day_spec::{A11yProps, AnimSpec, Color, Event, Insets, Role, Transform, kinds};

use crate::menus::lower_menu_scoped;
use crate::*;

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
///
/// **`AnyPiece` here is a trade, not a requirement.** It is not object safety: a `Modifier` is
/// never stored as `dyn Modifier`, and [`Decorate::modifier`] takes `impl Modifier` and applies
/// it on the spot. It is the closure impl below. A closure has ONE fixed parameter type, and
/// Rust has no `for<P> FnOnce(P) -> _` bound — higher-ranked bounds range over lifetimes, not
/// types — so making `apply` generic over the content (`fn apply<P: Piece>(self, c: P) ->
/// Self::Out<P>`, which a named modifier CAN satisfy with a GAT) would leave no closure able to
/// implement the trait at all. Pinning the input to the one piece type that accepts anything is
/// what keeps `|p| …` a modifier, and the erasure is what that costs — which is why this is the
/// single `Decorate` method that erases.
///
/// If a modifier ever needs to preserve its content's type, parameterize the TRAIT rather than
/// the method: `trait Modifier<P: Piece> { type Out: Piece; fn apply(self, c: P) -> Self::Out; }`.
/// Closures survive that (`impl<P: Piece, O: Piece, F: FnOnce(P) -> O> Modifier<P> for F`, and
/// their parameter still infers unannotated), while a named modifier such as day-piece-rating's
/// `Card` becomes `impl<P: Piece> Modifier<P> for Card` with `type Out = Decorated<P>`. The bill
/// is a `Decorate::modifier` whose return type varies per modifier, a generic impl for every
/// named one, and a breaking change — not worth paying while the tree has two implementors.
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
/// implicit animation. Layout-transparent (`FillThrough`), so it never affects sizing and a
/// granted stretch flows through to what it paints.
fn layer_node(cx: &mut BuildCx) -> RNode {
    cx.native(
        kinds::CONTAINER,
        &ContainerProps {
            background: None,
            corner_radius: 0.0,
            clips: false,
            role: None,
        },
        Rc::new(FillThrough),
        Flex::default(),
        Boundary::No,
    )
}

// ---------------------------------------------------------------------------
// Decorated — a piece plus its modifiers, with the piece's own type kept (§5.2)
// ---------------------------------------------------------------------------

/// The build of a piece plus every modifier applied to it so far.
type Build = Box<dyn FnOnce(&mut BuildCx) -> RNode>;

/// A piece with modifiers chained onto it, keeping the decorated piece's OWN type (§5.2).
///
/// Every [`Decorate`] modifier returns one of these rather than erasing to [`AnyPiece`], which is
/// what lets a chain keep reaching the piece's own builder methods — `label(…).padding(8.0)` is
/// still a decorated `Label`, so `.font(…)` after it resolves. Modifiers applied to a `Decorated`
/// append in place (the inherent methods below shadow the trait's), so a chain stays flat instead
/// of nesting `Decorated<Decorated<…>>`.
///
/// Erase explicitly with `.any()` when a single [`AnyPiece`] is what's needed (a `PieceVec`, a
/// `-> AnyPiece` signature).
pub struct Decorated<P> {
    inner: P,
    ops: Vec<Box<dyn FnOnce(Build) -> Build>>,
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
    pub fn new(inner: P) -> Self {
        Decorated {
            inner,
            ops: Vec::new(),
        }
    }

    fn push(mut self, op: impl FnOnce(Build) -> Build + 'static) -> Self {
        self.ops.push(Box::new(op));
        self
    }

    /// Replace the undecorated piece, keeping the modifier chain — how a typed builder trait
    /// reaches through a decoration (docs/api-style.md "Typed builders"). `f` sees the piece as
    /// it was before any modifier, which is why modifier order stops mattering.
    pub fn map_inner<Q: Piece>(self, f: impl FnOnce(P) -> Q) -> Decorated<Q> {
        Decorated {
            inner: f(self.inner),
            ops: self.ops,
        }
    }
}

impl<P: Piece> Piece for Decorated<P> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Decorated { inner, ops } = self;
        // Ops compose outward in call order: the FIRST modifier is innermost, matching the
        // per-modifier wrapper chain this replaced.
        let mut build: Build = Box::new(move |cx| inner.build(cx));
        for op in ops {
            build = op(build);
        }
        build(cx)
    }
}

// --- The modifier bodies, written once and shared by the trait and the inherent impl ---

fn op_id(id: String) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.set_id(n, id));
            n
        })
    }
}

fn op_id_of(id: impl Fn() -> String + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            day_reactive::bind(id, move |s: &String| {
                let s = s.clone();
                with_tree(|t| t.set_id(n, s));
            });
            n
        })
    }
}

fn op_tweak(f: impl FnOnce(day_core::RNode) + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            f(n);
            // Mark the node so a LATER backing swap (`.selectable()` on a toolkit that
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

fn op_padding(insets: Insets) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let w = cx.layout_only(
                Rc::new(PaddingLayout { insets }),
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

fn op_max_width(max: f64) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let w = cx.layout_only(
                Rc::new(MaxWidthLayout { max }),
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

fn op_reserving(sample: String) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let w = cx.layout_only(
                Rc::new(day_core::ReserveLayout),
                Flex::default(),
                Boundary::No,
            );
            cx.under(w, |cx| {
                // children[0]: the measured-but-invisible sample.
                let _ = crate::label(sample.clone()).opacity(0.0).build(cx);
                // children[1]: the real content.
                let _ = inner(cx);
            });
            w
        })
    }
}

/// `frame`/`width`/`height`: a fixed size on one or both axes. Two fixed axes make a layout
/// boundary (§7.4); one does not.
fn op_frame(
    width: Option<f64>,
    height: Option<f64>,
    boundary: Boundary,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let w = cx.layout_only(
                Rc::new(FrameLayout { width, height }),
                Flex::default(),
                boundary,
            );
            cx.under(w, |cx| {
                let _ = inner(cx);
            });
            w
        })
    }
}

fn op_a11y(f: impl FnOnce(A11yBuilder) -> A11yBuilder + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            let props = f(A11yBuilder::default()).0;
            with_tree(|t| t.set_a11y(n, props));
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

fn op_on_key(f: impl Fn(&day_spec::KeyEvent) + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            // Declare the intent as well as listening: a backend whose focused view would have
            // to CLAIM the key from the platform's own dispatch checks this first.
            day_spec::keys::mark(day_core::rnode_to_id(n));
            cx.on(n, move |ev| {
                if let Event::Key(k) = ev {
                    f(k);
                }
            });
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

fn op_focused(
    want: Box<dyn Fn() -> bool>,
    on_native: Box<dyn Fn(bool)>,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
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
}

fn op_context_menu(items: Vec<MenuEntry>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            // The overlay host places the decorated subtree at its own bounds and the
            // composed-menu mount beside it (see `OverlayHost` — a plain sibling would sit
            // under a single-child layout, never placed).
            let w = cx.layout_only(Rc::new(OverlayHost), Flex::default(), Boundary::No);
            let mut n = w;
            cx.under(w, |cx| {
                n = inner(cx);
                // Scoped: the action closures die with the build scope, not the process —
                // an unscoped registration here leaks one closure per remount.
                let model = lower_menu_scoped(items);
                with_tree(|t| t.set_context_menu(n, model.clone()));
                // A backend with no native menu reports the summon instead; the composed
                // presenter replays the same lowered model (docs/menus.md).
                let model = std::rc::Rc::new(model);
                crate::menus::mount_composed_menu(cx, n, Rc::new(move |_p| (*model).clone()));
            });
            // Later ops decorate the CONTENT node: ids, gestures and menus belong to it,
            // while the wrapper only exists to keep the mount in the layout pass.
            n
        })
    }
}

/// The `.context_menu*` wrapper's layout: the decorated subtree fills the wrapper; every
/// other child (the composed menu's lazy mount) is layout-inert but must still be VISITED —
/// a cover lays its content out from the size the backend reports, but only if the place
/// pass reaches it at all.
struct OverlayHost;

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
    f: impl Fn(day_spec::Point) -> Vec<MenuEntry> + 'static,
) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let w = cx.layout_only(Rc::new(OverlayHost), Flex::default(), Boundary::No);
            let mut out = w;
            cx.under(w, |cx| {
                out = op_context_menu_fn_body(inner, f, cx);
            });
            out
        })
    }
}

fn op_context_menu_fn_body(
    inner: Build,
    f: impl Fn(day_spec::Point) -> Vec<MenuEntry> + 'static,
    cx: &mut day_core::BuildCx,
) -> day_core::RNode {
    {
        {
            let n = inner(cx);
            // Each summon lowers a fresh menu whose action closures live in their own scope,
            // disposed when the NEXT summon replaces them (and with the build scope at
            // teardown) — so per-click menus never accumulate registrations.
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

fn op_on_drag(f: impl Fn(Drag) + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
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
}

fn op_on_pinch(f: impl Fn(Pinch) + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.enable_gesture(n, GestureKind::Pinch));
            cx.on(n, move |ev| {
                if let Event::Pinch {
                    phase,
                    scale,
                    location,
                } = ev
                {
                    f(Pinch {
                        phase: *phase,
                        scale: *scale,
                        location: *location,
                    });
                }
            });
            n
        })
    }
}

fn op_on_pan(f: impl Fn(Pan) + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.enable_gesture(n, GestureKind::Pan));
            cx.on(n, move |ev| {
                if let Event::Pan {
                    phase,
                    delta,
                    location,
                } = ev
                {
                    f(Pan {
                        phase: *phase,
                        delta: *delta,
                        location: *location,
                    });
                }
            });
            n
        })
    }
}

fn op_background(color: Reactive<Color>) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let node = cx.native(
                kinds::CONTAINER,
                &ContainerProps {
                    background: Some(color.get_untracked()),
                    corner_radius: 0.0,
                    clips: false,
                    role: None,
                },
                Rc::new(FillThrough),
                Flex::default(),
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = inner(cx);
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
}

fn op_corner_radius(radius: f64) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let node = cx.native(
                kinds::CONTAINER,
                &ContainerProps {
                    background: None,
                    corner_radius: radius,
                    clips: true,
                    role: None,
                },
                Rc::new(FillThrough),
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
            let node = layer_node(cx);
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

fn op_aspect_ratio(ratio: f64) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            if !(ratio.is_finite() && ratio > 0.0) {
                return inner(cx);
            }
            let node = cx.layout_only(
                Rc::new(AspectRatioLayout { ratio }),
                Flex::default(),
                // NOT a boundary: the child still measures itself, and the ratio only decides
                // the box it is offered.
                Boundary::No,
            );
            cx.under(node, |cx| {
                let _ = inner(cx);
            });
            node
        })
    }
}

fn op_grow_axes(w: bool, h: bool) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
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
                let _ = inner(cx);
            });
            node
        })
    }
}

fn op_grid_facts(facts: GridFacts) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let node = inner(cx);
            with_tree(|t| t.set_grid_facts(node, facts));
            node
        })
    }
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
// These INHERENT methods shadow the `Decorate` trait's (inherent wins method resolution), so a
// modifier applied to a `Decorated` appends to its op list instead of wrapping it in another
// `Decorated`. Each is the same one-liner the trait method is; the bodies live in the `op_*`
// functions above. Documentation stays on the trait, which is the surface every piece has.
impl<P: Piece> Decorated<P> {
    pub fn id(self, id: impl Into<String>) -> Self {
        self.push(op_id(id.into()))
    }
    pub fn id_of(self, id: impl Fn() -> String + 'static) -> Self {
        self.push(op_id_of(id))
    }
    pub fn id_keyed(self, prefix: &'static str, key: impl std::fmt::Display) -> Self {
        self.id(format!("{prefix}:{key}"))
    }
    pub fn tweak(self, f: impl FnOnce(day_core::RNode) + 'static) -> Self {
        self.push(op_tweak(f))
    }
    pub fn selectable(self) -> Self {
        self.push(op_selectable())
    }
    pub fn native_ref(self, r: &NativeRef) -> Self {
        self.push(op_native_ref(r.clone()))
    }
    pub fn padding(self, insets: impl IntoInsets) -> Self {
        self.push(op_padding(insets.into_insets()))
    }
    pub fn max_width(self, max: f64) -> Self {
        self.push(op_max_width(max))
    }
    pub fn reserving(self, sample: impl Into<String>) -> Self {
        self.push(op_reserving(sample.into()))
    }
    pub fn frame(self, width: f64, height: f64) -> Self {
        self.push(op_frame(Some(width), Some(height), Boundary::Yes))
    }
    pub fn width(self, width: f64) -> Self {
        self.push(op_frame(Some(width), None, Boundary::No))
    }
    pub fn height(self, height: f64) -> Self {
        self.push(op_frame(None, Some(height), Boundary::No))
    }
    pub fn a11y(self, f: impl FnOnce(A11yBuilder) -> A11yBuilder + 'static) -> Self {
        self.push(op_a11y(f))
    }
    pub fn on_tap(self, f: impl Fn() + 'static) -> Self {
        self.push(op_on_tap(f))
    }
    pub fn on_tap_at(self, f: impl Fn(day_spec::Point) + 'static) -> Self {
        self.push(op_on_tap_at(f))
    }
    pub fn focused<M>(self, binding: impl IntoFocusBinding<M>) -> Self {
        let (want, on_native) = binding.into_focus_binding();
        self.push(op_focused(want, on_native))
    }
    pub fn on_key(self, f: impl Fn(&day_spec::KeyEvent) + 'static) -> Self {
        self.push(op_on_key(f))
    }
    pub fn focusable(self) -> Self {
        self.push(op_focusable())
    }
    pub fn context_menu(self, items: Vec<MenuEntry>) -> Self {
        self.push(op_context_menu(items))
    }
    /// A context menu built AT SUMMON TIME (docs/menus.md "Dynamic context menus"): the
    /// closure runs when the user summons the menu, with the location in this piece's own
    /// coordinates, and whatever it returns is shown — so a canvas can select what is under
    /// the pointer and offer commands for THAT selection. An empty result shows nothing.
    pub fn context_menu_fn(self, f: impl Fn(day_spec::Point) -> Vec<MenuEntry> + 'static) -> Self {
        self.push(op_context_menu_fn(f))
    }
    pub fn on_drag(self, f: impl Fn(Drag) + 'static) -> Self {
        self.push(op_on_drag(f))
    }
    pub fn on_pinch(self, f: impl Fn(Pinch) + 'static) -> Self {
        self.push(op_on_pinch(f))
    }
    pub fn on_pan(self, f: impl Fn(Pan) + 'static) -> Self {
        self.push(op_on_pan(f))
    }
    pub fn background<M>(self, color: impl IntoReactive<Color, M>) -> Self {
        self.push(op_background(color.into_reactive()))
    }
    pub fn corner_radius(self, radius: f64) -> Self {
        self.push(op_corner_radius(radius))
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
    pub fn rotation<M>(self, degrees: impl IntoReactive<f64, M>) -> Self {
        let d = degrees.into_reactive();
        self.transform(move || Transform::rotate(d.get()))
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
    /// Erases, like [`Decorate::modifier`] — `Modifier` is defined over [`AnyPiece`].
    pub fn modifier(self, m: impl Modifier) -> AnyPiece {
        m.apply(self.any())
    }
    pub fn overlay(self, over: impl Piece) -> Self {
        self.overlay_aligned(Alignment::Center, over)
    }
    pub fn overlay_aligned(self, align: Alignment, over: impl Piece) -> Self {
        self.push(op_overlay_aligned(align, over))
    }
    pub fn aspect_ratio(self, ratio: f64) -> Self {
        self.push(op_aspect_ratio(ratio))
    }
    pub fn grow(self) -> Self {
        self.grow_axes(true, true)
    }
    pub fn grow_w(self) -> Self {
        self.grow_axes(true, false)
    }
    pub fn grow_h(self) -> Self {
        self.grow_axes(false, true)
    }
    #[doc(hidden)]
    pub fn grow_axes(self, w: bool, h: bool) -> Self {
        self.push(op_grow_axes(w, h))
    }
    pub fn grid_span(self, n: usize) -> Self {
        self.push(op_grid_facts(GridFacts {
            col_span: n.clamp(1, u16::MAX as usize) as u16,
            ..Default::default()
        }))
    }
    pub fn grid_align(self, a: Alignment) -> Self {
        self.push(op_grid_facts(GridFacts {
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
    /// Erase to a single [`AnyPiece`].
    pub fn any(self) -> AnyPiece {
        AnyPiece::new(self)
    }
}

pub trait Decorate: Piece + Sized {
    /// Stable element identifier: a11y identifier + dayscript locator + lint uniqueness (§5.5).
    fn id(self, id: impl Into<String>) -> Decorated<Self> {
        Decorated::new(self).id(id)
    }

    /// Reactive element id — the id for rows inside a recycling [`list`](crate::list). A plain
    /// [`id`](Self::id) is assigned once at build, but a recycled cell REBINDS to different
    /// items over its life (and drag-to-reorder rebinds eagerly), so a static item-derived id
    /// keeps naming the first-bound item. This variant re-registers whenever the closure's
    /// value changes — read your `ItemSlot` inside it:
    /// `.id_of(move || format!("row-remove-{}", slot.key()))`.
    fn id_of(self, id: impl Fn() -> String + 'static) -> Decorated<Self> {
        Decorated::new(self).id_of(id)
    }

    /// Keyed id for collection items: rendered `prefix:key` (§5.5).
    fn id_keyed(self, prefix: &'static str, key: impl std::fmt::Display) -> Decorated<Self> {
        Decorated::new(self).id_keyed(prefix, key)
    }

    /// Apply a **tweak**: `f` runs once at mount, after the native widget exists, with the
    /// realized node (docs/tweaks.md). Reach the typed native handle through the compiled
    /// backend's ext accessor (`day_appkit::with_native`, `day_gtk::with_native`, …) — or apply
    /// a packaged `day-tweak-*` crate's modifier instead of calling this directly. If the native
    /// change affects the widget's intrinsic size, follow it with
    /// [`day_core::invalidate_size`]. Day may overwrite *managed* properties (title, value,
    /// enabled, frame, a11y) on its next patch; unmanaged properties are stable.
    ///
    /// Order it AFTER any modifier that can rebuild the backing widget — today
    /// [`selectable`](Decorate::selectable), which on UIKit realizes the label as a different
    /// native class. Chained before it, the tweak runs against the widget the rebuild discards
    /// (Day warns at runtime); chained after, it sees the widget that ships.
    fn tweak(self, f: impl FnOnce(day_core::RNode) + 'static) -> Decorated<Self> {
        Decorated::new(self).tweak(f)
    }

    /// Make this piece's text **user-selectable** — the reader can select and copy it
    /// (docs/text.md). Most useful on a [`label`](crate::label): text is NOT selectable by default
    /// on any backend, matching each platform's native behavior.
    ///
    /// Every backend honors it on a label: most flip the native widget's selection affordance
    /// (AppKit, GTK, Qt, XAML, HarmonyOS, Android, web); UIKit — whose `UILabel` has none —
    /// rebuilds the label as a read-only `UITextView` behind the same handle. On other widgets
    /// it is best-effort: a backing with no selection affordance leaves the text unselectable
    /// rather than erroring, and a container cascades only where the platform's affordance does
    /// (the web) — prefer the label itself. Selection visuals and the copy shortcut are the
    /// platform's own. Unmanaged — set once at mount, and it survives Day's text updates.
    fn selectable(self) -> Decorated<Self> {
        Decorated::new(self).selectable()
    }

    /// Capture a [`NativeRef`] to this piece's realized node for later imperative access
    /// (docs/tweaks.md). The ref clears automatically when the piece's scope is disposed.
    fn native_ref(self, r: &NativeRef) -> Decorated<Self> {
        Decorated::new(self).native_ref(r)
    }

    fn padding(self, insets: impl IntoInsets) -> Decorated<Self> {
        Decorated::new(self).padding(insets)
    }

    /// Cap this piece's width at `max` points: the child is never PROPOSED more, so text
    /// wraps inside the cap (chat bubbles, readable columns) while narrower content hugs.
    fn max_width(self, max: f64) -> Decorated<Self> {
        Decorated::new(self).max_width(max)
    }

    /// Reserve at least the space `sample` needs, so this piece's size stops changing with its
    /// content.
    ///
    /// For a numeric readout beside a slider: `label(move || value()).reserving("100")` keeps the
    /// row still while the number changes, because the reservation is a real measurement of
    /// `"100"` in this piece's own font — it scales with the platform's accessibility text size
    /// instead of being a point value that clips when someone turns text up.
    ///
    /// Pass the WIDEST value the field can show (`"100"`, `"-99.9"`, `"88:88"`). Pair it with
    /// tabular numbers so the digits themselves stop shifting inside the reservation.
    /// The sample never paints and never takes hit-testing area.
    fn reserving(self, sample: impl Into<String>) -> Decorated<Self> {
        Decorated::new(self).reserving(sample)
    }

    fn frame(self, width: f64, height: f64) -> Decorated<Self> {
        Decorated::new(self).frame(width, height)
    }

    /// Fix this piece's WIDTH to `width` points while its height stays flexible (hugging its content
    /// or filling on the cross axis). The single-axis complement to [`Self::frame`] — e.g. a
    /// fixed-width sidebar pane in a `row` whose height fills the window.
    fn width(self, width: f64) -> Decorated<Self> {
        Decorated::new(self).width(width)
    }

    /// Fix this piece's HEIGHT to `height` points while its width stays flexible. The single-axis
    /// complement to [`Self::frame`] — e.g. a fixed-height header/toolbar bar that fills its width.
    fn height(self, height: f64) -> Decorated<Self> {
        Decorated::new(self).height(height)
    }

    fn a11y(self, f: impl FnOnce(A11yBuilder) -> A11yBuilder + 'static) -> Decorated<Self> {
        Decorated::new(self).a11y(f)
    }

    /// Fire when this piece is tapped (bounding-box; shapes override with path-precise testing).
    fn on_tap(self, f: impl Fn() + 'static) -> Decorated<Self> {
        Decorated::new(self).on_tap(f)
    }

    /// [`on_tap`](Self::on_tap), told WHERE — the point in the piece's own coordinate space,
    /// origin at its top-leading corner.
    ///
    /// What a drawn control needs and a native one does not: a canvas showing a color wheel, a
    /// map, or a waveform has to turn "the user pressed here" into a value, and only the piece
    /// knows how. Pair it with [`on_drag`](Self::on_drag) — which already reports a location — to
    /// track a press that turns into a drag; the two are idempotent together, so a backend that
    /// reports a tap as a zero-length drag costs nothing.
    ///
    /// `Event::Tap` has always carried the point; this is the decorator that stops throwing it
    /// away.
    fn on_tap_at(self, f: impl Fn(day_spec::Point) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_tap_at(f)
    }

    /// Bind this control's keyboard focus to a signal (docs/focus.md), two-way like every other
    /// binding: native focus changes write the signal; writing the signal moves focus. Takes a
    /// `Signal<bool>` for one control, or `(Signal<Option<K>>, K::Variant)` binding one control
    /// of a group — writing `false`/`None` resigns focus (dismissing the soft keyboard on
    /// mobile). Focus applies asynchronously: a write is a request, resolved on the next turn,
    /// and the signal always ends up reflecting what the platform actually did.
    fn focused<M>(self, binding: impl IntoFocusBinding<M>) -> Decorated<Self> {
        Decorated::new(self).focused(binding)
    }

    /// Handle the non-text keys — the arrows — that reach this piece WHILE IT HAS FOCUS
    /// (docs/menus.md). Keys follow focus, so pair it with [`Decorate::focused`] or with a
    /// piece the user can click into: a canvas takes focus on a press, and only the focused
    /// piece hears the keys, so a nudge handler can never fire while a text field, a list or a
    /// sidebar is the one being typed into.
    ///
    /// Which pieces can hold focus is the platform's own question (docs/focus.md) — a `canvas`
    /// is focusable on the backends that draw one from a real view (appkit, web-dom today).
    fn on_key(self, f: impl Fn(&day_spec::KeyEvent) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_key(f)
    }

    /// Opt this piece into the platform's focus system (docs/focus.md) — the canvas contract
    /// for anything composed: it joins the key loop, takes focus on a press, reports through
    /// `.focused(…)`, and hears the arrows through `.on_key(…)` while focused. A composed
    /// list column is the motivating case (docs/navigation.md). On a backend without the
    /// `set_focusable` duty the piece renders normally and simply never takes focus.
    fn focusable(self) -> Decorated<Self> {
        Decorated::new(self).focusable()
    }

    /// Attach a context menu, shown with the platform's native affordance on secondary-click (desktop)
    /// or long-press (mobile). Items are built with [`menu_item`]/[`sub_menu`]/[`menu_role`]/
    /// [`menu_separator`]. Passing an empty `Vec` removes any menu.
    fn context_menu(self, items: Vec<MenuEntry>) -> Decorated<Self> {
        Decorated::new(self).context_menu(items)
    }
    /// See [`Decorated::context_menu_fn`]: a context menu built at summon time.
    fn context_menu_fn(
        self,
        f: impl Fn(day_spec::Point) -> Vec<MenuEntry> + 'static,
    ) -> Decorated<Self> {
        Decorated::new(self).context_menu_fn(f)
    }

    /// Fire on each phase of a drag over this piece.
    fn on_drag(self, f: impl Fn(Drag) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_drag(f)
    }

    /// Fire on each phase of a pinch/magnify over this piece (docs/canvas.md "Zoom and
    /// pan"). Only backends with a native recognizer wired emit it — pair a zoom with
    /// visible controls.
    fn on_pinch(self, f: impl Fn(Pinch) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_pinch(f)
    }

    /// Fire on each viewport-pan event over this piece (docs/canvas.md "Zoom and pan"):
    /// trackpad two-finger scroll, two-finger touch pan. `delta` is incremental — apply it
    /// as it arrives.
    fn on_pan(self, f: impl Fn(Pan) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_pan(f)
    }

    /// Fill the piece's bounds with a solid color painted behind it — a message-bubble / card /
    /// badge surface. Accepts a constant [`Color`], a `Signal<Color>`, or a `Fn() -> Color`; a
    /// reactive color repaints the surface when its source changes. Wraps the piece in a native
    /// container that carries the fill, so it composes with [`Self::corner_radius`] for a rounded
    /// colored surface and with [`Self::padding`] for interior inset.
    fn background<M>(self, color: impl IntoReactive<Color, M>) -> Decorated<Self> {
        Decorated::new(self).background(color)
    }

    /// Round the piece's corners to `radius` points, clipping its background and content to the
    /// rounded rectangle. Compose after [`Self::background`] for a rounded colored surface, or use
    /// alone to round a clipped child (e.g. an avatar image).
    fn corner_radius(self, radius: f64) -> Decorated<Self> {
        Decorated::new(self).corner_radius(radius)
    }

    /// Animate/set the piece's opacity (`0.0` transparent … `1.0` opaque). Wrapped in a native
    /// layer so it composes with `.background`; the change animates when made inside
    /// [`with_animation`] or under a `.animation` ancestor (§8.4).
    fn opacity<M>(self, opacity: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).opacity(opacity)
    }

    /// Apply an animatable [`Transform`] (translate/scale/rotate about the center) — the cheap
    /// movement/scaling channel that never triggers relayout (§8.4). Prefer this over `.offset`
    /// for animated motion.
    fn transform<M>(self, t: impl IntoReactive<Transform, M>) -> Decorated<Self> {
        Decorated::new(self).transform(t)
    }

    /// Uniformly scale the piece by `factor` about its center (animatable). Convenience over
    /// [`Self::transform`].
    fn scale<M>(self, factor: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).scale(factor)
    }

    /// Rotate the piece by `degrees` clockwise about its center (animatable).
    fn rotation<M>(self, degrees: impl IntoReactive<f64, M>) -> Decorated<Self> {
        Decorated::new(self).rotation(degrees)
    }

    /// Translate the piece by (`x`, `y`) points WITHOUT relayout (animatable) — the
    /// animation-friendly sibling of `.offset`.
    fn translation<Mx, My>(
        self,
        x: impl IntoReactive<f64, Mx>,
        y: impl IntoReactive<f64, My>,
    ) -> Decorated<Self> {
        Decorated::new(self).translation(x, y)
    }

    /// Attach an implicit animation (§8.4): changes to this piece's — and its descendants' —
    /// animatable properties animate with `anim` even outside a [`with_animation`]. SwiftUI's
    /// `.animation`. The ambient `with_animation` takes precedence when both apply.
    fn animation(self, anim: AnimSpec) -> Decorated<Self> {
        Decorated::new(self).animation(anim)
    }

    /// Apply a [`Modifier`] — or, via the blanket impl, a plain `FnOnce(AnyPiece) -> AnyPiece`
    /// closure — to this piece. Pure composition: `content.modifier(m) == m.apply(content.any())`.
    ///
    /// The one modifier that ERASES: `Modifier` is defined over [`AnyPiece`], so the piece's own
    /// type cannot survive it.
    fn modifier(self, m: impl Modifier) -> AnyPiece {
        m.apply(self.any())
    }

    /// Draw `over` on top of this piece, centered, WITHOUT affecting layout size — a badge /
    /// annotation overlay. `self` is the sizing content (bottom of the z-order); `over` is proposed
    /// `self`'s size and drawn on top. For an explicit alignment use [`Self::overlay_aligned`]; for
    /// a stack that sizes to the UNION of its children use [`zstack`].
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
    /// Pair it with [`Self::grow_w`] for a piece that takes the width available and derives its
    /// height from it — a `canvas` whose drawing has to keep its proportions as the window
    /// resizes, say. `image` has carried this since it shipped; this is the same layout, for any
    /// piece.
    ///
    /// A ratio that is not finite and positive describes no box, so it is ignored.
    fn aspect_ratio(self, ratio: f64) -> Decorated<Self> {
        Decorated::new(self).aspect_ratio(ratio)
    }

    /// Expand to fill the available space on both axes (a filling pane / card that stretches to
    /// its container). Wraps the piece in a layout-only node carrying grow [`Flex`] — the stack
    /// offers it the space and it fills; no native backing, so this is a pure layout change.
    fn grow(self) -> Decorated<Self> {
        Decorated::new(self).grow()
    }

    /// Expand to fill the available horizontal space.
    fn grow_w(self) -> Decorated<Self> {
        Decorated::new(self).grow_w()
    }

    /// Expand to fill the available vertical space.
    fn grow_h(self) -> Decorated<Self> {
        Decorated::new(self).grow_h()
    }

    #[doc(hidden)]
    fn grow_axes(self, w: bool, h: bool) -> Decorated<Self> {
        Decorated::new(self).grow_axes(w, h)
    }

    /// Span `n` columns (n ≥ 1) of the enclosing [`grid`] (docs/grid.md). Grid modifiers set
    /// facts on the node the grid sees: apply them LAST (outermost), like `.grow_w()` — an
    /// outer wrapper would hide the facts from the grid.
    fn grid_span(self, n: usize) -> Decorated<Self> {
        Decorated::new(self).grid_span(n)
    }

    /// Override this cell's alignment within its cell rect of the enclosing [`grid`]
    /// (docs/grid.md). Apply LAST (outermost), like [`Self::grid_span`].
    fn grid_align(self, a: Alignment) -> Decorated<Self> {
        Decorated::new(self).grid_align(a)
    }

    /// While this subtree is mounted, ask the OS to require a second swipe for its edge
    /// gestures on `edges` (docs/cover.md) — the SwiftUI `defersSystemGestures(on:)`
    /// analogue. Put it on a game or drawing surface whose touches run to the screen edge,
    /// so a swipe up from the bottom doesn't leave the app. iOS defers the chosen edges'
    /// system gestures; Android enters swipe-to-reveal immersive mode while any subtree
    /// requests deferral; desktop backends no-op.
    fn defers_system_gestures(self, edges: day_spec::Edges) -> Decorated<Self> {
        Decorated::new(self).defers_system_gestures(edges)
    }

    /// While this subtree is mounted, the enclosing [`cover`] (or other modal surface) must
    /// not be dismissed interactively — the SwiftUI `interactiveDismissDisabled()` analogue
    /// (docs/cover.md). System back / sheet gestures are ignored; only programmatic writes
    /// (an explicit close control) dismiss it.
    fn interactive_dismiss_disabled(self) -> Decorated<Self> {
        Decorated::new(self).interactive_dismiss_disabled()
    }

    /// Erase to a single [`AnyPiece`] — for a `PieceVec`, a `-> AnyPiece` signature, or any other
    /// place one concrete type is required. [`AnyPiece::any`] is inherent and returns `self`, so
    /// erasing an already-erased piece costs nothing.
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
    /// element already announced by its labeled sibling.
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
