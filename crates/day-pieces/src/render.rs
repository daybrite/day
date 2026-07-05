//! External-piece registration surface (DESIGN.md §8.2). The `renderer!` macro registers a piece's
//! per-toolkit native renderer into a backend's `RENDERERS` slice with **typed** `make`/`update` (the
//! macro inserts the `&dyn Any` downcast) and no hand-written linkme boilerplate. `fill_measure` is the
//! shared "growing leaf" sizing, so pieces stop hand-rolling it per backend.

/// A leaf `measure` that FILLS the space it's proposed — for growing leaves (a web view, a canvas, a
/// Lottie view). Use as `measure: day_pieces::fill_measure` in `renderer!`. This gives one uniform
/// "fill" answer across every backend (some backends' `measure: None` default returns a view's natural
/// size, which collapses a size-less native view).
pub fn fill_measure<B: day_spec::Toolkit>(
    _backend: &mut B,
    _handle: &B::Handle,
    proposal: day_spec::Proposal,
) -> day_spec::Size {
    day_spec::Size::new(
        proposal.width.unwrap_or(0.0),
        proposal.height.unwrap_or(0.0),
    )
}

/// Register a piece's per-toolkit native renderer into `$slice` (a backend's `RENDERERS`).
///
/// The author writes typed functions and one macro line — no `#[distributed_slice]`, no `Renderer {}`
/// literal, no `downcast_ref` in the bodies:
/// ```ignore
/// fn make(b: &mut AppKit, p: &MyProps, id: NodeId) -> Retained<NSView> { … }
/// fn update(b: &mut AppKit, h: &Retained<NSView>, patch: &MyPatch) { … }
/// day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
///     kind: KIND, props: MyProps, patch: MyPatch, make: make, update: update);
/// ```
/// Add `measure: f` (e.g. `measure: day_pieces::fill_measure`) for custom sizing; omit it to use the
/// backend's default. One `renderer!` per module (it defines a module-level static).
#[macro_export]
macro_rules! renderer {
    // props + patch + make + update (+ measure)
    ($slice:path, $backend:ty, kind: $kind:expr, props: $props:ty, patch: $patch:ty,
     make: $make:expr, update: $update:expr, measure: $measure:expr $(,)?) => {
        $crate::__renderer!(
            $slice,
            $backend,
            $kind,
            $props,
            $patch,
            $make,
            $update,
            ::core::option::Option::Some($measure)
        );
    };
    ($slice:path, $backend:ty, kind: $kind:expr, props: $props:ty, patch: $patch:ty,
     make: $make:expr, update: $update:expr $(,)?) => {
        $crate::__renderer!(
            $slice,
            $backend,
            $kind,
            $props,
            $patch,
            $make,
            $update,
            ::core::option::Option::None
        );
    };
    // patchless: props + make (+ measure) — for pieces configured once with no updates (e.g. Lottie).
    ($slice:path, $backend:ty, kind: $kind:expr, props: $props:ty,
     make: $make:expr, measure: $measure:expr $(,)?) => {
        $crate::__renderer!(
            $slice,
            $backend,
            $kind,
            $props,
            (),
            $make,
            (|_b, _h, _p| {}),
            ::core::option::Option::Some($measure)
        );
    };
    ($slice:path, $backend:ty, kind: $kind:expr, props: $props:ty, make: $make:expr $(,)?) => {
        $crate::__renderer!(
            $slice,
            $backend,
            $kind,
            $props,
            (),
            $make,
            (|_b, _h, _p| {}),
            ::core::option::Option::None
        );
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __renderer {
    ($slice:path, $backend:ty, $kind:expr, $props:ty, $patch:ty, $make:expr, $update:expr, $measure:expr) => {
        #[$crate::linkme::distributed_slice($slice)]
        static __DAY_RENDERER: fn() -> $crate::Renderer<$backend> = || $crate::Renderer {
            kind: $kind,
            make: |__b, __props, __id| {
                let __p = __props
                    .downcast_ref::<$props>()
                    .expect(concat!("day renderer: props are not ", stringify!($props)));
                ($make)(__b, __p, __id)
            },
            update: |__b, __h, __patch| {
                if let ::core::option::Option::Some(__p) = __patch.downcast_ref::<$patch>() {
                    ($update)(__b, __h, __p)
                }
            },
            measure: $measure,
        };
    };
}
