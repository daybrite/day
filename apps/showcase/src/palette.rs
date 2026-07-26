//! The showcase's deliberate colors — everything else stays native-neutral (DESIGN §6.3).
//!
//! One ordered "sunrise" ramp derived from the Day identity: the app icon's dawn ambers and
//! rust-orange sun over the website's brand blue (`website/src/styles/global.css` `--brand`,
//! `--sun`, `--grad-day`). Mid-value hues that hold up on both the light and dark window
//! grounds; where a fill is pale (AMBER), pair it with [`INK`] rather than white.

use day::prelude::Color;

/// Dawn amber — the icon's ray color. A pale fill: use [`INK`] text on it, never white.
pub(crate) const AMBER: Color = Color::hex(0xF0A64C);
/// Sunrise coral — the icon sun's upper gradient stop.
pub(crate) const CORAL: Color = Color::hex(0xE86A3C);
/// Deep rust — the icon sun's base. The warm "hero action" fill (white text holds ≥4.5:1).
pub(crate) const RUST: Color = Color::hex(0xC2491D);
/// Dusk violet — the cool counterweight between the warm ramp and the brand blue.
pub(crate) const VIOLET: Color = Color::hex(0x7C5CD6);
/// Brand blue (`--brand` on the light theme) — the app-wide accent.
pub(crate) const SKY: Color = Color::hex(0x2F6FDE);
/// Light brand blue (`--brand` on the dark theme) — highlights and cold gradient stops.
pub(crate) const AZURE: Color = Color::hex(0x6AA4FF);
/// Daylight teal — the one green the ramp allows.
pub(crate) const TEAL: Color = Color::hex(0x1E9E86);
/// Neutral slate — de-emphasis, tracks, and quiet fills.
pub(crate) const SLATE: Color = Color::hex(0x64748B);
/// Near-black ink for text set on pale fills (AMBER pills stay readable in dark mode,
/// where the default label color flips to white).
pub(crate) const INK: Color = Color::hex(0x22293A);

/// The ordered sunrise ramp: cycle through it wherever a sequence needs one color per step
/// (e.g. the navigation stack's depth chips). Starts at CORAL so every stop takes white text.
pub(crate) const RAMP: [Color; 4] = [CORAL, VIOLET, SKY, TEAL];
