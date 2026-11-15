// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The renderer registration surface (DESIGN.md §8.2). The `renderer!` macro registers a piece's
//! per-toolkit native renderer into a backend's `RENDERERS` slice with **typed** `make`/`update`
//! (the macro inserts the `&dyn Any` downcast) and no hand-written linkme boilerplate;
//! `dom_renderer!` is its counterpart for the web's runtime registry. `fill_measure` is the
//! shared "growing leaf" sizing, so pieces stop hand-rolling it per backend.
//!
//! These live beside [`Renderer`](crate::Renderer), the type they fill in, so a backend and a
//! renderer-only crate register through the contract crate without the widget builders.

/// A leaf `measure` that fills the space it's proposed, for growing leaves (a web view, a canvas,
/// a Lottie view). Use as `measure: day_spec::fill_measure` in `renderer!`. This gives one
/// uniform "fill" answer across every backend (some backends' `measure: None` default returns a
/// view's natural size, which collapses a size-less native view).
pub fn fill_measure<B: crate::Toolkit>(
    _backend: &mut B,
    _handle: &B::Handle,
    proposal: crate::Proposal,
) -> crate::Size {
    crate::Size::new(
        proposal.width.unwrap_or(0.0),
        proposal.height.unwrap_or(0.0),
    )
}

/// Register a piece's per-toolkit native renderer into `$slice` (a backend's `RENDERERS`).
///
/// The author writes typed functions and one macro line; the macro supplies the
/// `#[distributed_slice]`, the `Renderer {}` literal, and the `downcast_ref` in the bodies:
/// ```ignore
/// fn make(b: &mut AppKit, p: &MyProps, id: NodeId) -> Retained<NSView> { todo!("construct the native view") }
/// fn update(b: &mut AppKit, h: &Retained<NSView>, patch: &MyPatch) { todo!("construct the native view") }
/// day_spec::renderer!(day_toolkit_appkit::RENDERERS, AppKit,
///     kind: KIND, props: MyProps, patch: MyPatch, make: make, update: update);
/// ```
/// Add `measure: f` (e.g. `measure: day_spec::fill_measure`) for custom sizing; omit it to use the
/// backend's default. One `renderer!` per module (it defines a module-level static).
#[macro_export]
macro_rules! renderer {
    // …+ release: teardown for a piece that owns per-view state of its own (a retained delegate,
    // a session map). Runs from the release queue just before the backend frees the handle.
    ($slice:path, $backend:ty, kind: $kind:expr, props: $props:ty, patch: $patch:ty,
     make: $make:expr, update: $update:expr, measure: $measure:expr, release: $release:expr $(,)?) => {
        $crate::__renderer!(
            $slice,
            $backend,
            $kind,
            $props,
            $patch,
            $make,
            $update,
            ::core::option::Option::Some($measure),
            ::core::option::Option::Some($release)
        );
    };
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
            ::core::option::Option::Some($measure),
            ::core::option::Option::None
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
            ::core::option::Option::None,
            ::core::option::Option::None
        );
    };
    // patchless: props + make (+ measure), for pieces configured once with no updates (e.g.
    // Lottie).
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
            ::core::option::Option::Some($measure),
            ::core::option::Option::None
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
            ::core::option::Option::None,
            ::core::option::Option::None
        );
    };
}

/// Register a piece's **web-dom** renderer, whose registry is populated at RUNTIME.
///
/// `linkme` has no `wasm32-unknown-unknown` implementation (DESIGN.md §8.2), so `day-toolkit-dom` keeps a
/// `Registry<Dom>` that pieces add to by calling `day_toolkit_dom::register_renderer`. This macro is
/// `renderer!`'s counterpart for that registry: same typed `make`/`update`/`measure`, same
/// inserted downcast, but it defines an idempotent `pub(crate) fn register()` instead of a static,
/// and the piece's constructor must call it, which is the one line web pieces write that
/// link-time ones don't:
///
/// ```ignore
/// // lib-dom.rs
/// day_spec::dom_renderer!(day_toolkit_dom::register_renderer, day_toolkit_dom::Dom,
///     kind: KIND, props: MyProps, patch: MyPatch, make: make, update: update, measure: measure);
///
/// // lib.rs: a constructor always runs before the node it returns is realized.
/// pub fn my_piece() -> MyPiece {
///     #[cfg(all(feature = "dom", target_arch = "wasm32"))]
///     dom_impl::register();
///     todo!("construct the piece")
/// }
/// ```
///
/// One `dom_renderer!` per module.
#[macro_export]
macro_rules! dom_renderer {
    // …+ release: teardown for a piece that keeps per-element state of its own. Same shape as
    // `renderer!`'s, and it runs from the same release queue.
    ($register:path, $backend:ty, kind: $kind:expr, props: $props:ty, patch: $patch:ty,
     make: $make:expr, update: $update:expr, measure: $measure:expr, release: $release:expr $(,)?) => {
        $crate::__dom_renderer!(
            $register,
            $backend,
            $kind,
            $props,
            $patch,
            $make,
            $update,
            ::core::option::Option::Some($measure),
            ::core::option::Option::Some($release)
        );
    };
    ($register:path, $backend:ty, kind: $kind:expr, props: $props:ty, patch: $patch:ty,
     make: $make:expr, update: $update:expr, measure: $measure:expr $(,)?) => {
        $crate::__dom_renderer!(
            $register,
            $backend,
            $kind,
            $props,
            $patch,
            $make,
            $update,
            ::core::option::Option::Some($measure),
            ::core::option::Option::None
        );
    };
    ($register:path, $backend:ty, kind: $kind:expr, props: $props:ty, patch: $patch:ty,
     make: $make:expr, update: $update:expr $(,)?) => {
        $crate::__dom_renderer!(
            $register,
            $backend,
            $kind,
            $props,
            $patch,
            $make,
            $update,
            ::core::option::Option::None,
            ::core::option::Option::None
        );
    };
    // patchless: props + make (+ measure), mirroring `renderer!`.
    ($register:path, $backend:ty, kind: $kind:expr, props: $props:ty,
     make: $make:expr, measure: $measure:expr $(,)?) => {
        $crate::__dom_renderer!(
            $register,
            $backend,
            $kind,
            $props,
            (),
            $make,
            (|_b, _h, _p| {}),
            ::core::option::Option::Some($measure),
            ::core::option::Option::None
        );
    };
    ($register:path, $backend:ty, kind: $kind:expr, props: $props:ty, make: $make:expr $(,)?) => {
        $crate::__dom_renderer!(
            $register,
            $backend,
            $kind,
            $props,
            (),
            $make,
            (|_b, _h, _p| {}),
            ::core::option::Option::None,
            ::core::option::Option::None
        );
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __dom_renderer {
    ($register:path, $backend:ty, $kind:expr, $props:ty, $patch:ty, $make:expr, $update:expr,
     $measure:expr, $release:expr) => {
        /// Add this piece's renderer to web-dom's runtime registry. Idempotent: the registry
        /// keeps the first entry per kind, so calling it from every constructor is free.
        pub(crate) fn register() {
            ($register)(|| $crate::Renderer::<$backend> {
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
                release: $release,
            });
        }
    };
}

#[macro_export]
#[doc(hidden)]
macro_rules! __renderer {
    ($slice:path, $backend:ty, $kind:expr, $props:ty, $patch:ty, $make:expr, $update:expr,
     $measure:expr, $release:expr) => {
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
            release: $release,
        };
    };
}
