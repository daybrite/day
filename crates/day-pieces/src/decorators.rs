// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! `Decorate`, the chainable modifiers every piece inherits: padding and sizing, background and
//! corner radius, gestures (`on_tap`, drag), accessibility (`A11yBuilder`), and native-handle
//! capture (`NativeRef`), plus the `Modifier` / `IntoInsets` supporting traits.
//!
//! Modifiers return [`Decorated<P>`], which keeps the decorated piece's type, so a chain
//! never stops reaching that piece's builder methods (§5.2).

use std::cell::Cell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, bind};
use day_spec::props::*;
use day_spec::{A11yProps, AnimSpec, Color, Cursor, Event, Insets, Role, Transform, kinds};

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
/// modifiers (`.opacity`/`.transform`) to carry a per-node opacity or transform. Layout-transparent (`FillThrough`), so it never affects sizing and a
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

    /// Replace the undecorated piece, keeping the modifier chain: how a typed builder trait
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
        // Ops compose outward in call order: the first modifier is innermost, matching the
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
            with_tree(|t| t.set_node_cursor(n, cursor.get_untracked()));
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

fn op_min_width(min: f64) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let w = cx.layout_only(
                Rc::new(day_core::MinWidthLayout { min }),
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
            let (props, live) = f(A11yBuilder::default()).split();
            with_tree(|t| t.set_a11y(n, props));
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

fn op_on_key(f: impl Fn(&day_spec::KeyEvent) + 'static) -> impl FnOnce(Build) -> Build {
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
                if let Event::Key(k) = ev {
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

fn op_on_hover(f: impl Fn(Option<day_spec::Point>) + 'static) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            let n = inner(cx);
            with_tree(|t| t.enable_gesture(n, GestureKind::Hover));
            cx.on(n, move |ev| {
                if let Event::Hover { phase, location } = ev {
                    // `None` on exit rather than a phase the caller has to match on: what a
                    // hover handler wants to know is where the pointer IS, and "nowhere" is a
                    // state, not an event kind. It is also what makes a handler that writes a
                    // `Signal<Option<Point>>` a one-liner.
                    f(match phase {
                        day_spec::DragPhase::Ended => None,
                        _ => Some(*location),
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

fn op_aspect_ratio(ratio: f64) -> impl FnOnce(Build) -> Build {
    move |inner| {
        Box::new(move |cx| {
            if !(ratio.is_finite() && ratio > 0.0) {
                return inner(cx);
            }
            let node = cx.layout_only(
                Rc::new(AspectRatioLayout { ratio }),
                Flex::default(),
                // Not a boundary: the child still measures itself, and the ratio only decides
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
    pub fn cursor<M>(self, cursor: impl IntoReactive<Cursor, M>) -> Self {
        self.push(op_cursor(cursor.into_reactive()))
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
    /// Never narrower than `min`: a stretching control keeps a usable width, and a row that
    /// cannot give it that overflows, so a `labeled` row stacks the control under its label
    /// rather than squeezing it (docs/forms.md).
    pub fn min_width(self, min: f64) -> Self {
        self.push(op_min_width(min))
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
        self.push(op_on_key(f))
    }
    pub fn focusable(self) -> Self {
        self.push(op_focusable())
    }
    pub fn toolbar<M>(self, content: impl crate::ToolbarContent<M>) -> Self {
        self.push(op_toolbar(content.into_source()))
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
    /// Attach native drop acceptance policy and an owned-data receiver.
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
    pub fn context_menu(self, items: Vec<MenuEntry>) -> Self {
        self.push(op_context_menu(items))
    }
    /// A context menu built when the menu is summoned (docs/menus.md "Dynamic context menus"):
    /// the closure runs at that moment, with the location in this piece's own coordinates, and
    /// whatever it returns is shown, so a canvas can select what is under the pointer and offer
    /// commands for that selection. An empty result shows nothing.
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
    pub fn status_bar_hidden<M>(self, hidden: impl IntoReactive<bool, M>) -> Self {
        self.push(op_status_bar_hidden(hidden.into_reactive()))
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

    /// Reactive element id: the id for rows inside a recycling [`list`](crate::list). A plain
    /// [`id`](Self::id) is assigned once at build, but a recycled cell rebinds to different
    /// items over its life (and drag-to-reorder rebinds eagerly), so a static item-derived id
    /// keeps naming the first-bound item. This variant re-registers whenever the closure's
    /// value changes; read your `ItemSlot` inside it:
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
    /// backend's ext accessor (`day_appkit::with_native`, `day_gtk::with_native`, …), or apply
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
    /// See [`Decorated::min_width`].
    fn min_width(self, min: f64) -> Decorated<Self> {
        Decorated::new(self).min_width(min)
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
    /// The sample never paints and never takes hit-testing area.
    fn reserving(self, sample: impl Into<String>) -> Decorated<Self> {
        Decorated::new(self).reserving(sample)
    }

    fn frame(self, width: f64, height: f64) -> Decorated<Self> {
        Decorated::new(self).frame(width, height)
    }

    /// Fix this piece's width to `width` points while its height stays flexible (hugging its
    /// content or filling on the cross axis). The single-axis complement to [`Self::frame`], e.g.
    /// a fixed-width sidebar pane in a `row` whose height fills the window.
    fn width(self, width: f64) -> Decorated<Self> {
        Decorated::new(self).width(width)
    }

    /// Fix this piece's height to `height` points while its width stays flexible. The single-axis
    /// complement to [`Self::frame`], e.g. a fixed-height header/toolbar bar that fills its width.
    fn height(self, height: f64) -> Decorated<Self> {
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
    fn on_key(self, f: impl Fn(&day_spec::KeyEvent) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_key(f)
    }

    /// Declare toolbar items on the chrome this piece sits under (docs/toolbars.md).
    ///
    /// Takes one item, a list of them, or a closure that derives the list and re-runs whenever
    /// its reactive reads change:
    ///
    /// ```ignore
    /// page.toolbar(toolbar_button("share", tr("share")).icon(Symbol::Share).action(share))
    /// page.toolbar([reply, forward, archive])
    /// page.toolbar(move || vec![toolbar_button("undo", tr("undo")).enabled_when(can_undo)])
    /// ```
    ///
    /// Which chrome carries them follows from where this piece is built (a destination page's
    /// own bar, a content-list pane's, or the window's if it is under no page at all), and the
    /// items are withdrawn when this piece is disposed. Where on that chrome they sit is
    /// [`ToolbarEntry::placement`](crate::ToolbarEntry::placement).
    fn toolbar<M>(self, content: impl crate::ToolbarContent<M>) -> Decorated<Self> {
        Decorated::new(self).toolbar(content)
    }

    /// Opt this piece into the platform's focus system (docs/focus.md): the canvas contract
    /// for anything composed: it joins the key loop, takes focus on a press, reports through
    /// `.focused(…)`, and hears the arrows through `.on_key(…)` while focused. A composed
    /// list column is the motivating case (docs/navigation.md). On a backend without the
    /// `set_focusable` duty the piece renders normally and never takes focus.
    fn focusable(self) -> Decorated<Self> {
        Decorated::new(self).focusable()
    }

    /// Attach a context menu, shown with the platform's native affordance on secondary-click (desktop)
    /// or long-press (mobile). Items are built with [`menu_item`]/[`sub_menu`]/[`menu_role`]/
    /// [`menu_separator`]. Passing an empty `Vec` removes any menu.
    fn drag_source(
        self,
        source: impl Fn(day_spec::Point) -> Option<day_spec::transfer::Offer> + 'static,
    ) -> Decorated<Self> {
        Decorated::new(self).drag_source(source)
    }
    fn drop_target(self, target: day_spec::transfer::Target) -> Decorated<Self> {
        Decorated::new(self).drop_target(target)
    }
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
    /// pan"). Only backends with a native recognizer wired emit it; pair a zoom with
    /// visible controls.
    fn on_pinch(self, f: impl Fn(Pinch) + 'static) -> Decorated<Self> {
        Decorated::new(self).on_pinch(f)
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

    /// Fill the piece's bounds with a solid color painted behind it: a message-bubble / card /
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

    /// Apply an animatable [`Transform`] (translate/scale/rotate about the center): the cheap
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

    /// Translate the piece by (`x`, `y`) points without relayout (animatable): the
    /// animation-friendly sibling of `.offset`.
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
    /// Pair it with [`Self::grow_w`] for a piece that takes the width available and derives its
    /// height from it: a `canvas` whose drawing has to keep its proportions as the window
    /// resizes, say. `image` has carried this since it shipped; this is the same layout, for any
    /// piece.
    ///
    /// A ratio that is not finite and positive describes no box, so it is ignored.
    fn aspect_ratio(self, ratio: f64) -> Decorated<Self> {
        Decorated::new(self).aspect_ratio(ratio)
    }

    /// Expand to fill the available space on both axes (a filling pane / card that stretches to
    /// its container). Wraps the piece in a layout-only node carrying grow [`Flex`]: the stack
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
    /// facts on the node the grid sees: apply them last (outermost), like `.grow_w()`; an
    /// outer wrapper would hide the facts from the grid.
    fn grid_span(self, n: usize) -> Decorated<Self> {
        Decorated::new(self).grid_span(n)
    }

    /// Override this cell's alignment within its cell rect of the enclosing [`grid`]
    /// (docs/grid.md). Apply last (outermost), like [`Self::grid_span`].
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

    /// Erase to a single [`AnyPiece`]: for a `PieceVec`, a `-> AnyPiece` signature, or any other
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
pub struct A11yBuilder {
    label: Option<TextSource>,
    hint: Option<TextSource>,
    value: Option<TextSource>,
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

    /// The annotations as first applied, with every string resolved once, and the sources
    /// that can change, each paired with the patch that re-sends it.
    fn split(self) -> (A11yProps, Vec<LiveA11yString>) {
        let mut props = self.props;
        let mut live = Vec::new();
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
        (props, live)
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
    let accept_alive = alive.clone();
    day_spec::transfer::Target {
        types: target.types,
        accept: Rc::new(move |location| {
            if accept_alive.get() {
                scope.enter(|| (target.accept)(location))
            } else {
                day_spec::transfer::Operation::None
            }
        }),
        receive: Rc::new(move |drop| alive.get() && scope.enter(|| (target.receive)(drop))),
    }
}

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The modifiers' `#[day::test]` cases, next to the `Decorate` methods they prove.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::AnyPiece;
    use day_core::conformance::{Case, Drive, FrameExpect};
    use day_reactive::Signal;
    use day_spec::{Cap, Color, DragPhase, Transform};

    use crate::*;

    fn size(width: f64, height: f64) -> FrameExpect {
        FrameExpect {
            width: Some(width),
            height: Some(height),
            ..Default::default()
        }
    }

    /// Padding grows the frame by its insets and moves the content in by them.
    #[day_macros::test(day_core)]
    fn padding_insets() -> Case {
        Case::new()
            .proves_modifier("padding")
            .page(|| {
                rectangle()
                    .fill(Color::rgb(1.0, 0.0, 0.0))
                    .id("inner")
                    .frame(40.0, 20.0)
                    .padding(10.0)
                    .id("outer")
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
                    label("Hi").min_width(120.0).id("min"),
                    label("A label long enough that it would run past eighty points")
                        .max_width(80.0)
                        .id("max"),
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

    /// An aspect ratio sets the height from the width.
    #[day_macros::test(day_core)]
    fn aspect_ratio_height() -> Case {
        Case::new()
            .proves_modifier("aspect_ratio")
            .page(|| {
                rectangle()
                    .fill(Color::rgb(0.0, 0.0, 1.0))
                    .aspect_ratio(2.0)
                    .id("shape")
                    .width(100.0)
            })
            .drive(|d: Drive| async move { d.assert_frame("shape", size(100.0, 50.0)).await })
    }

    /// A growing child takes the row's remaining width.
    #[day_macros::test(day_core)]
    fn grow_width() -> Case {
        Case::new()
            .proves_modifier("grow_w")
            .page(|| {
                row((
                    rectangle()
                        .fill(Color::rgb(1.0, 0.0, 0.0))
                        .frame(40.0, 20.0),
                    rectangle()
                        .fill(Color::rgb(0.0, 0.0, 1.0))
                        .height(20.0)
                        .grow_w()
                        .id("grower"),
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
                    .id("base")
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
                    .id("filled")
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
                .id("ground")
                .frame(80.0, 80.0)
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.5, 0.5, "#0000ff").await?;
                d.sample_pixel("ground", 0.03, 0.03, "#ffffff").await
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
                .id("ground")
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
                .id("ground")
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
                .id("ground")
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
                .id("ground")
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
                    .rotation(90.0),
                ))
                .id("ground")
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
    fn pad(id: &str) -> Decorated<ShapePiece> {
        rectangle()
            .fill(Color::rgb(0.2, 0.4, 0.9))
            .frame(120.0, 80.0)
            .id(id.to_string())
    }

    /// A tap runs `.on_tap`'s work, once per tap.
    #[day_macros::test(day_core)]
    fn on_tap_runs() -> Case {
        Case::new()
            .proves_modifier("on_tap")
            .page(|| {
                let taps = Signal::new(0i64);
                column((
                    label(move || format!("taps {}", taps.get())).id("taps"),
                    label("Tap me")
                        .padding(12.0)
                        .on_tap(move || taps.update(|n| *n += 1))
                        .id("target"),
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
                    label(move || format!("at {}", at.get())).id("at"),
                    rectangle()
                        .fill(Color::rgb(0.2, 0.4, 0.9))
                        .frame(120.0, 80.0)
                        .on_tap_at(move |p| at.set(format!("{:.0},{:.0}", p.x, p.y)))
                        .id("target"),
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
                    label(move || state.get()).id("state"),
                    pad("target").on_drag(move |d: Drag| {
                        state.set(match d.phase {
                            DragPhase::Began => "began".into(),
                            DragPhase::Changed => "moving".into(),
                            DragPhase::Ended => {
                                format!("moved {:.0},{:.0}", d.translation.x, d.translation.y)
                            }
                        })
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
                    label(move || state.get()).id("state"),
                    pad("target").on_hover(move |p| {
                        state.set(match p {
                            Some(p) => format!("over {:.0},{:.0}", p.x, p.y),
                            None => "out".into(),
                        })
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
                    .id("total"),
                    pad("target").on_pan(move |p: Pan| {
                        total.update(|(x, y)| {
                            *x += p.delta.x;
                            *y += p.delta.y;
                        })
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
                    label(move || format!("scale {}", scale.get())).id("scale"),
                    pad("target").on_pinch(move |p: Pinch| {
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
                    label(move || format!("key {}", last.get())).id("last"),
                    pad("target")
                        .focusable()
                        .on_key(move |k| last.set(k.key.clone())),
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

    /// A focus binding moves focus when the app sets it, and follows the field when it is
    /// focused natively.
    #[day_macros::test(day_core)]
    fn focused_binding_moves_focus() -> Case {
        Case::new()
            .proves_modifier("focused")
            .page(|| {
                let want = Signal::new(false);
                column((
                    label(move || format!("focused {}", want.get())).id("state"),
                    button("Focus").action(move || want.set(true)).id("go"),
                    text_field(Signal::new(String::new()))
                        .focused(want)
                        .id("field"),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_focused("field", false).await?;
                d.tap("go").await?;
                d.assert_focused("field", true).await?;
                d.assert_text("state", "focused true").await
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
                    .id("bar")
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

    /// `.grow_h` takes the height its column has left; `.grow_axes` grows on the axes named.
    #[day_macros::test(day_core)]
    fn grow_vertical_fills_column() -> Case {
        Case::new()
            .proves_modifier("grow_h")
            .proves_modifier("grow_axes")
            .page(|| {
                row((
                    column((
                        rectangle()
                            .fill(Color::rgb(1.0, 0.0, 0.0))
                            .id("top")
                            .frame(40.0, 20.0),
                        rectangle()
                            .fill(Color::rgb(0.0, 0.0, 1.0))
                            .id("rest")
                            .width(40.0)
                            .grow_h(),
                    ))
                    .spacing(0.0)
                    .height(120.0),
                    rectangle()
                        .fill(Color::rgb(0.0, 1.0, 0.0))
                        .id("both")
                        .width(30.0)
                        .grow_axes(false, true),
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
                let cell = |id: &str, w: f64| {
                    rectangle()
                        .fill(Color::rgb(0.2, 0.4, 0.9))
                        .id(id.to_string())
                        .frame(w, 20.0)
                };
                grid((
                    grid_row((cell("a", 40.0), cell("b", 60.0))),
                    grid_row((cell("wide", 30.0)
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
                    .id("base")
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
                .id("ground")
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
                    button("Move").action(move || moved.set(true)).id("move"),
                    zstack((
                        rectangle().fill(Color::WHITE),
                        label("")
                            .frame(40.0, 40.0)
                            .background(Color::rgb(0.0, 0.0, 1.0))
                            .translation(move || if moved.get() { 80.0 } else { 0.0 }, 0.0)
                            .animation(day_spec::AnimSpec::linear(150)),
                    ))
                    .align(day_core::Alignment::Leading)
                    .id("ground")
                    .frame(120.0, 40.0),
                ))
            })
            .drive(|d: Drive| async move {
                d.sample_pixel("ground", 0.1, 0.5, "#0000ff").await?;
                d.tap("move").await?;
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
            .proves_modifier("id_keyed")
            .page(|| {
                let n = Signal::new(1i64);
                column((
                    label("Fixed").id("fixed"),
                    label("Derived").id_of(move || format!("derived-{}", n.get())),
                    label("Keyed").id_keyed("keyed", 7),
                    button("Next").action(move || n.set(2)).id("next"),
                ))
            })
            .drive(|d: Drive| async move {
                d.assert_text("fixed", "Fixed").await?;
                d.assert_text("keyed:7", "Keyed").await?;
                d.assert_text("derived-1", "Derived").await?;
                d.tap("next").await?;
                d.assert_missing("derived-1").await?;
                d.assert_text("derived-2", "Derived").await
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
                    label(move || format!("tweaked {}", tweaked.get())).id("tweaked"),
                    label(move || format!("held {}", held.node().is_some())).id("held"),
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
                column((label("Inner").id("inner").modifier(padded).id("outer"),))
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
                    .id("save")
            })
            .drive(|d: Drive| async move {
                d.tap("save").await?;
                d.assert_announced("Saved").await
            })
    }

    day_core::tests! {
        padding_insets,
        on_tap_runs,
        on_tap_at_reports_point,
        on_drag_reports_translation,
        on_hover_enters_and_leaves,
        on_pan_adds_deltas,
        on_pinch_reports_scale,
        on_key_reaches_focused,
        focused_binding_moves_focus,
        height_fixes_height,
        grow_vertical_fills_column,
        grid_span_and_align,
        overlay_centers,
        transform_applies_whole,
        animation_lands_on_final,
        ids_name_elements,
        tweak_and_native_ref_see_node,
        modifier_applies,
        announce_reaches_reader,
        rotation_turns_clockwise,
        scale_grows_drawing,
        translation_moves_view,
        min_max_width,
        aspect_ratio_height,
        grow_width,
        overlay_aligned_corner,
        background_fill,
        corner_radius_cuts,
        opacity_hides,
        translation_moves_drawing,
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
        let (source, target) = owner.enter(|| {
            owner.provide(11u32);
            let check = || assert_eq!(day_core::environment::<u32>(), Some(11));
            (
                scoped_drag_source(Rc::new(move |_| {
                    check();
                    Some(Offer::default())
                })),
                scoped_drop_target(Target {
                    types: vec![],
                    accept: Rc::new(move |_| {
                        check();
                        Operation::Copy
                    }),
                    receive: Rc::new(move |_| {
                        check();
                        true
                    }),
                }),
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
            assert!(target.deliver(location(), Offer::default()));
            assert_eq!(day_core::environment::<u32>(), Some(22));
            owner.dispose();
            assert!(source(day_spec::Point::new(0., 0.)).is_none());
            assert_eq!(target.proposal(&location()), Operation::None);
            assert!(!(target.receive)(Drop {
                local: true,
                position: day_spec::Point::new(0., 0.),
                operation: Operation::Copy,
                items: vec![]
            }));
        });
        foreign.dispose();
    }
}
