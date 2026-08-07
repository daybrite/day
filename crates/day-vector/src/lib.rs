//! day-vector — Day's vector-graphics engine (docs/vectors.md, docs/icons.md).
//!
//! One crate, two consumers:
//!
//! * **Build time** (day-cli): `day icon` renders app-icon masters into every platform's icon
//!   formats, and resource staging (§18.3) converts `resource/vectors/` sources into the form
//!   each toolkit loads natively — a VectorDrawable on Android, a rasterized PNG ladder where
//!   the toolkit has no vector path.
//! * **Runtime** (future, opt-in): the same pure byte-in/byte-out functions can rasterize a
//!   downloaded SVG inside an app. Nothing here touches day-cli types, the filesystem, or a
//!   toolkit, so the crate compiles anywhere (wasm included).
//!
//! The engine is [`resvg`]/[`usvg`] with the `text` feature OFF — masters and vector assets
//! must carry outlined text; a `<text>` element is reported as an error by the callers rather
//! than rendered wrong. Everything else is hand-rolled here precisely because the formats are
//! small: ICO and ICNS are trivial PNG containers, a VectorDrawable is a constrained XML
//! serialization of the usvg tree, and an SF Symbol template is sliced/assembled textually.
//!
//! Sources understood by [`classify`]:
//! * plain SVG — any static SVG 1.1 art;
//! * an SF Symbol **template** SVG (the SF Symbols app's export, and third-party generators of
//!   the same shape): `#Symbols` holding `Weight-Scale` variant groups, `#Guides` with
//!   caplines/baselines — [`extract_variant`] cuts one variant out as a standalone glyph;
//! * a `.symbolset` bundle (handled by the caller: its inner template SVG routes through here).

pub use resvg;
pub use resvg::tiny_skia;
pub use resvg::usvg;
pub use roxmltree;

mod classify;
mod icns;
mod ico;
pub mod icongen;
mod raster;
mod symbolset;
mod vd;

/// The render-engine identity stamped into `icons.lock.json` — byte-stable renders hold only
/// within one engine version, so `day icon --check` compares generators before bytes.
pub const ENGINE: &str = "resvg-0.45";

pub use classify::{SourceKind, classify, extract_variant};
pub use icns::pack_icns;
pub use ico::pack_ico;
pub use raster::{content_bbox, parse, render_png, render_png_padded};
pub use symbolset::wrap_symbolset;
pub use vd::{Unsupported, to_vector_drawable};
