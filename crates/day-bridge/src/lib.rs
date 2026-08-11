// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! daybridge runtime (docs/bridge.md, DESIGN.md §15.6) — foreign-language implementations of a
//! Rust API.
//!
//! A crate declares one API and supplies implementations per platform; `day build` generates the
//! adapters and the glue. This crate is the small runtime half: the [`bridge!`] macro, the
//! [`Error`] that crosses every boundary, and the [`Support`] an arm reports.
//!
//! ```ignore
//! day_bridge::bridge! {
//!     #[day_bridge::declare]
//!     extern "day" {
//!         fn speak_native(text: &str) -> Result<(), day_bridge::Error>;
//!     }
//!
//!     #[day_bridge::impl(rust, platforms = [other])]
//!     fn speak_native(_text: &str) -> Result<(), day_bridge::Error> {
//!         Err(day_bridge::Error::Unsupported)
//!     }
//! }
//! ```
//!
//! The generator is [`day_build::bridge`](../day_build/bridge/index.html), called from the crate's
//! `build.rs`. Nothing here parses anything: see [`bridge!`] for why.

use std::fmt;

/// What a target's arm promises, re-exported from day-spec so a bridged crate's `available()`
/// answers in the same vocabulary as `day::capability()`.
pub use day_spec::Support;

/// The single error type crossing a bridge boundary (docs/bridge.md "Errors").
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    /// No arm claims this target — what the `other` fallback returns.
    Unsupported,
    /// The arm failed: a Swift `throws`, a Kotlin exception, a thrown JS error, a nonzero C status.
    /// The string is the platform's own message, which is the only detail that survives.
    Foreign(String),
    /// An argument or result was not valid UTF-8.
    Encoding,
    /// The platform runtime was unavailable — no JVM, no `Context`, COM init refused.
    Runtime,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Unsupported => write!(f, "unsupported on this platform"),
            Error::Foreign(msg) => write!(f, "{msg}"),
            Error::Encoding => write!(f, "invalid UTF-8 across the bridge"),
            Error::Runtime => write!(f, "platform runtime unavailable"),
        }
    }
}

impl std::error::Error for Error {}

/// Declare a crate's bridge: the API, its per-platform implementations, and their preludes.
///
/// **The body is discarded.** This macro expands to nothing but an `include!` of the code
/// day-build generated from the same source text, which is what lets an arm contain Swift, Kotlin,
/// ArkTS, JavaScript, C, or C++ — the tokens are never resolved by rustc, only lexed. It is also
/// why daybridge needs no procedural macro, and why DESIGN.md §5.1's "no required macro anywhere in
/// the framework" still holds: this is opt-in sugar that lowers to plain generated Rust.
///
/// Foreign code inside an arm must nevertheless *lex* as Rust tokens, which idiomatic JavaScript and
/// ArkTS do not — a backtick is not a Rust token, and `'zh-CN'` lexes as a malformed lifetime. That
/// is why inline arms carry their body in a raw string.
#[macro_export]
macro_rules! bridge {
    ($($body:tt)*) => {
        include!(concat!(env!("OUT_DIR"), "/day-bridge/mod.rs"));
    };
}
