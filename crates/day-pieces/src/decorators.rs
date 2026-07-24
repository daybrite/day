//! `Decorate` — the chainable modifiers every piece inherits: padding and sizing, background and
//! corner radius, gestures (`on_tap`, drag), accessibility (`A11yBuilder`), and native-handle
//! capture (`NativeRef`) — plus the `Modifier` / `IntoInsets` supporting traits.

use std::cell::Cell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, bind};
use day_spec::props::*;
use day_spec::{A11yProps, AnimSpec, Color, Event, Insets, Role, Transform, kinds};

use crate::menus::lower_menu;
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
