// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-piece-swiftui — custom SwiftUI views inside a Day app, on macos-appkit + ios-uikit only.
//!
//! The native half resolves a provider class named `@objc(DayView_<name>)` (dots in `name` become
//! underscores), asks it for a SwiftUI body, and hosts that body in an `NSHostingView` (macOS) or a
//! `UIHostingController`'s view (iOS) — returned to Day as an ordinary native handle, framed and
//! snapshotted like any built-in. Two ways in (docs/swiftui.md):
//!
//! - **Generated bindings** (the usual way): point `[package.metadata.day.ios/macos]`
//!   `swift-packages` at a local SwiftPM package; day-build scans its public `View` structs and
//!   emits `crate::swiftui::MyView(param1, param2)` constructors, while `day build` emits the
//!   matching provider glue. Apps then never touch this crate's API beyond the Cargo dependency.
//! - **The provider escape hatch**: subclass `DaySwiftUIProvider` in Swift, name it
//!   `@objc(DayView_mything)`, and call `swiftui("mything")` — for views that need wiring the
//!   binding subset can't express.
//!
//! Params ride as a JSON string. A reactive `.params(...)` re-invokes the provider's body on every
//! change and swaps the hosting view's `rootView`; SwiftUI diffing preserves `@State` as long as
//! the provider returns the same underlying view type each call.
//!
//! It's a growing leaf, so constrain it with `.frame(w, h)` when it shouldn't fill the pane.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
pub use day_pieces::{IntoReactive, Reactive};
use day_reactive::bind_seeded;

pub const KIND: &str = "day.piece.swiftui";

/// Full props (realize). `name` picks the provider class; `params` seeds the JSON the provider's
/// body first receives (`None` when [`SwiftUi::params`] was never called).
#[derive(Clone, Debug, PartialEq)]
pub struct SwiftUiProps {
    /// The provider name — class `DayView_<name>` with `.` mapped to `_`. Generated bindings use
    /// `Module.View`; hand-written providers pick any dot-free name.
    pub name: String,
    /// The initial JSON params string.
    pub params: Option<String>,
    /// The state-retention key ([`SwiftUi::state_key`]) — `None` hosts a fresh view per mount.
    pub state_key: Option<String>,
}

/// Sparse reconcile patch — only `params` changes after build (`name` is fixed).
#[derive(Clone, Debug, PartialEq)]
pub enum SwiftUiPatch {
    /// New JSON params — pushed whenever the bound params source changes; the native half
    /// re-invokes the provider's body and replaces the hosting view's root.
    Params(String),
}

/// A hosted SwiftUI view. Bind its data reactively with `.params(...)`; keep its `@State` across
/// unmount/remount with `.state_key(...)`.
pub struct SwiftUi {
    name: String,
    params: Option<Reactive<String>>,
    state_key: Option<String>,
}

/// `swiftui("Module.View")` — host the SwiftUI view exported as `@objc(DayView_Module_View)`.
pub fn swiftui(name: impl Into<String>) -> SwiftUi {
    SwiftUi {
        name: name.into(),
        params: None,
        state_key: None,
    }
}

impl SwiftUi {
    /// The JSON params string — a constant, a `Signal<String>`, or a `Fn() -> String`. When it's
    /// reactive the hosted view follows it live: each change re-invokes the provider's body with
    /// the new JSON (`@State` inside the view survives, see the crate docs).
    pub fn params<M>(mut self, params: impl IntoReactive<String, M>) -> Self {
        self.params = Some(params.into_reactive());
        self
    }

    /// Keep the hosted view's SwiftUI state across unmount/remount. Without a key, leaving the
    /// piece's branch (a tab switch, a `when()` going false, a page navigation) disposes the
    /// hosting view and its `@State` with it; with one, the native half retains the hosting view
    /// under `key` and hands the SAME instance back on the next mount — sliders, scroll positions,
    /// `@State`/`@StateObject` all survive, and the mount's current params are re-applied.
    ///
    /// The key pins one hosting view for the app's lifetime, so use it for the handful of views
    /// that want persistence, not per-row content. At most one live instance per key: two mounted
    /// pieces sharing a key would fight over one native view.
    pub fn state_key(mut self, key: impl Into<String>) -> Self {
        self.state_key = Some(key.into());
        self
    }
}

impl Piece for SwiftUi {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let params = self.params;
        let seed = params.as_ref().map(|p| p.get_untracked());
        let props = SwiftUiProps {
            name: self.name,
            params: seed.clone(),
            state_key: self.state_key,
        };
        // A hosted SwiftUI view fills the space it's offered (constrain via `.frame(w, h)`).
        let node = cx.leaf(
            KIND,
            &props,
            Flex {
                grow_w: true,
                grow_h: true,
                ..Default::default()
            },
        );
        if let Some(p) = params {
            // `Const` params read the same value forever, so this seeds once and never patches; a
            // `Signal`/`Fn` params re-runs and pushes a `Params` patch on every change.
            bind_seeded(
                seed.unwrap_or_default(),
                move || p.get(),
                move |v: &String| {
                    with_tree(|t| t.patch(node, Box::new(SwiftUiPatch::Params(v.clone())), false));
                },
            );
        }
        node
    }
}

/// Whether this build hosts SwiftUI natively: `Native` on macos-appkit and ios-uikit, else
/// `Unsupported`. The gate app code should branch on — never a backend-feature `cfg`.
pub fn support() -> day_spec::Support {
    #[cfg(any(
        all(feature = "appkit", target_os = "macos"),
        all(feature = "uikit", target_os = "ios"),
    ))]
    {
        day_spec::Support::Native
    }
    #[cfg(not(any(
        all(feature = "appkit", target_os = "macos"),
        all(feature = "uikit", target_os = "ios"),
    )))]
    {
        day_spec::Support::Unsupported
    }
}

/// Minimal JSON rendering for the params channel — enough for the generated bindings (flat objects
/// of strings/numbers/bools) without a serde dependency. Hand-written params can use it too, or
/// bring their own serializer.
pub mod json {
    /// A rendered JSON value (already escaped/formatted).
    pub struct Value(String);

    /// A JSON string value.
    pub fn string(s: &str) -> Value {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('"');
        escape_into(s, &mut out);
        out.push('"');
        Value(out)
    }

    /// A JSON integer value.
    pub fn int(n: i64) -> Value {
        Value(n.to_string())
    }

    /// A JSON number value (non-finite floats have no JSON spelling and render as `0`).
    pub fn float(n: f64) -> Value {
        Value(if n.is_finite() {
            n.to_string()
        } else {
            "0".into()
        })
    }

    /// A JSON boolean value.
    pub fn boolean(b: bool) -> Value {
        Value(if b { "true" } else { "false" }.into())
    }

    /// A JSON object from `(key, value)` fields, in the given order.
    pub fn object(fields: &[(&str, Value)]) -> String {
        let mut out = String::from("{");
        for (i, (key, value)) in fields.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push('"');
            escape_into(key, &mut out);
            out.push_str("\":");
            out.push_str(&value.0);
        }
        out.push('}');
        out
    }

    fn escape_into(s: &str, out: &mut String) {
        for c in s.chars() {
            match c {
                '"' => out.push_str("\\\""),
                '\\' => out.push_str("\\\\"),
                '\n' => out.push_str("\\n"),
                '\r' => out.push_str("\\r"),
                '\t' => out.push_str("\\t"),
                c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
                c => out.push(c),
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn objects_render_escaped_and_in_order() {
            let json = object(&[
                ("title", string("say \"hi\"\n")),
                ("count", int(-3)),
                ("ratio", float(0.5)),
                ("on", boolean(true)),
            ]);
            assert_eq!(
                json,
                r#"{"title":"say \"hi\"\n","count":-3,"ratio":0.5,"on":true}"#
            );
        }

        #[test]
        fn non_finite_floats_render_as_zero() {
            let json = object(&[("x", float(f64::NAN)), ("y", float(f64::INFINITY))]);
            assert_eq!(json, r#"{"x":0,"y":0}"#);
        }
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — AppKit + UIKit only. Each registers a `Renderer` link-time into
// its backend's `RENDERERS` slice; `#[cfg]` gates each to its feature + target.
// ---------------------------------------------------------------------------

day_pieces::glue_modules!(appkit, uikit);

// --- Typed builders, forwarded through `Decorated` (docs/api-style.md) ---

/// [`SwiftUi`]'s own builders, reachable THROUGH a decoration (§5.2): `day_pieces::Decorated` forwards them
/// to the piece it wraps, so generic modifiers and typed ones chain in any order.
pub trait SwiftUiBuilder: Sized {
    fn params<M>(self, params: impl IntoReactive<String, M>) -> Self;
    fn state_key(self, key: impl Into<String>) -> Self;
}

impl SwiftUiBuilder for SwiftUi {
    fn params<M>(self, params: impl IntoReactive<String, M>) -> Self {
        SwiftUi::params(self, params)
    }
    fn state_key(self, key: impl Into<String>) -> Self {
        SwiftUi::state_key(self, key)
    }
}

impl<Inner: SwiftUiBuilder + day_pieces::prelude::Piece> SwiftUiBuilder
    for day_pieces::Decorated<Inner>
{
    fn params<M>(self, params: impl IntoReactive<String, M>) -> Self {
        self.map_inner(|inner_piece| inner_piece.params(params))
    }
    fn state_key(self, key: impl Into<String>) -> Self {
        self.map_inner(|inner_piece| inner_piece.state_key(key))
    }
}
