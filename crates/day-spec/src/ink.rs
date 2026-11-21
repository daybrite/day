// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The color a view is drawn in (docs/color.md): [`Ink`], which is a fixed [`Color`], a
//! light/dark pair or a [`SystemColor`] role, and the [`Palette`] every role resolves against.
//!
//! `Color` stays the concrete sRGB value an app computes with. `Ink` is what every color
//! parameter of a view takes, so a color that differs by appearance is a value, written once,
//! and the toolkit follows the appearance with no app code running: a dynamic `NSColor` or
//! `UIColor`, a CSS custom property, or a re-apply when a toolkit without dynamic colors sees
//! the theme change.

use std::cell::Cell;

use crate::Color;

/// A platform color role: what a color means, which each toolkit answers with its own
/// palette (docs/color.md "System colors"). Pass a role wherever a color is taken
/// (`.background(SystemColor::Surface)`, `label(..).color(SystemColor::Link)`) and the view
/// follows the appearance, the accent and the contrast setting.
///
/// Every role maps to a real platform color on every toolkit, or to the documented
/// light/dark pair in [`SystemColor::fallback`] where a platform has none.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum SystemColor {
    /// The app's accent: selection, the prominent button, a focus ring.
    Accent,
    /// Primary text.
    Text,
    /// De-emphasized text: a caption, a detail line.
    SecondaryText,
    /// The window's content background.
    Background,
    /// A region set off from the background: a panel, a card, a sidebar.
    Surface,
    /// Text a step quieter than secondary: a disabled-looking hint, a footnote.
    TertiaryText,
    /// A text field's placeholder.
    Placeholder,
    /// A link in running text.
    Link,
    /// A hairline between items: a rule, a list separator, a grid line.
    Separator,
    /// The fill of a control's track or well: a slider's track, a segmented control's
    /// background, a search field.
    ControlFill,
    /// The background behind grouped sections (an inset grouped list, a settings page).
    GroupedBackground,
    /// A surface raised above the content: a popover, a menu, a sheet in dark appearance.
    ElevatedSurface,
    /// The highlight behind selected text or selected content the app draws itself. Text
    /// keeps its own color on it.
    Selection,
    /// A destructive action or an error.
    Destructive,
    /// Success, a value that rose.
    Success,
    /// A warning, something to notice.
    Warning,
}

impl SystemColor {
    /// How many roles there are: the length of [`SystemColor::ALL`] and of a [`Palette`].
    pub const COUNT: usize = 16;

    /// All roles, in discriminant order (the wire order: a role crosses JNI, C++ and JS as its
    /// position here).
    pub const ALL: [Self; Self::COUNT] = [
        Self::Accent,
        Self::Text,
        Self::SecondaryText,
        Self::Background,
        Self::Surface,
        Self::TertiaryText,
        Self::Placeholder,
        Self::Link,
        Self::Separator,
        Self::ControlFill,
        Self::GroupedBackground,
        Self::ElevatedSurface,
        Self::Selection,
        Self::Destructive,
        Self::Success,
        Self::Warning,
    ];

    /// The role as its wire number: its position in [`SystemColor::ALL`].
    #[must_use]
    pub const fn wire(self) -> u8 {
        self as u8
    }

    /// The role with wire number `n`, if there is one.
    #[must_use]
    pub const fn from_wire(n: u8) -> Option<SystemColor> {
        if (n as usize) < Self::COUNT {
            Some(Self::ALL[n as usize])
        } else {
            None
        }
    }

    /// This role at opacity `alpha`, replacing the role's own alpha: a wash of the accent
    /// behind a selected chip (`SystemColor::Accent.with_alpha(0.14)`).
    #[must_use]
    pub const fn with_alpha(self, alpha: f64) -> Ink {
        Ink::System {
            role: self,
            alpha: Some(alpha),
        }
    }

    /// Day's own light/dark pair for the role: what a toolkit answers where its platform has
    /// no such color, and what [`Palette::fallback`] holds. Fixed sRGB colors, close to the
    /// Apple system palette, not an approximation of a user-chosen accent.
    #[must_use]
    pub const fn fallback(self, dark: bool) -> Color {
        match (self, dark) {
            (Self::Accent, false) => Color::hex(0x000a_66ff),
            (Self::Accent, true) => Color::hex(0x003f_83ff),
            (Self::Text, false) => Color::hex(0x001d_1d1f),
            (Self::Text, true) => Color::hex(0x00f5_f5f7),
            (Self::SecondaryText, false) => Color::hex(0x006e_6e73),
            (Self::SecondaryText, true) => Color::hex(0x0098_989d),
            (Self::Background | Self::ElevatedSurface, false) => Color::hex(0x00ff_ffff),
            (Self::Background, true) => Color::hex(0x001e_1e20),
            (Self::Surface, false) => Color::hex(0x00f3_f2f7),
            (Self::Surface, true) => Color::hex(0x002c_2c2e),
            (Self::TertiaryText | Self::Placeholder, false) => rgba8(60, 60, 67, 0.3),
            (Self::TertiaryText | Self::Placeholder, true) => rgba8(235, 235, 245, 0.3),
            (Self::Link, false) => Color::hex(0x0000_66cc),
            (Self::Link, true) => Color::hex(0x004d_a3ff),
            (Self::Separator, false) => rgba8(60, 60, 67, 0.29),
            (Self::Separator, true) => rgba8(84, 84, 88, 0.6),
            (Self::ControlFill, false) => rgba8(120, 120, 128, 0.2),
            (Self::ControlFill, true) => rgba8(120, 120, 128, 0.36),
            (Self::GroupedBackground, false) => Color::hex(0x00f2_f2f7),
            (Self::GroupedBackground, true) => Color::hex(0x0016_1618),
            (Self::ElevatedSurface, true) => Color::hex(0x003a_3a3c),
            (Self::Selection, false) => rgba8(10, 102, 255, 0.25),
            (Self::Selection, true) => rgba8(63, 131, 255, 0.35),
            (Self::Destructive, false) => Color::hex(0x00ff_3b30),
            (Self::Destructive, true) => Color::hex(0x00ff_453a),
            (Self::Success, false) => Color::hex(0x0034_c759),
            (Self::Success, true) => Color::hex(0x0030_d158),
            (Self::Warning, false) => Color::hex(0x00ff_9500),
            (Self::Warning, true) => Color::hex(0x00ff_9f0a),
        }
    }
}

/// An sRGB color from 8-bit channels and a float alpha, for the translucent fallbacks.
const fn rgba8(r: u8, g: u8, b: u8, a: f64) -> Color {
    Color::rgba(r as f64 / 255.0, g as f64 / 255.0, b as f64 / 255.0, a)
}

/// The color a view is drawn in: a fixed color, a light/dark pair, or a platform role
/// (docs/color.md). Every color parameter of a view takes `impl Into<Ink>`, so
/// `Color::rgb(..)`, `SystemColor::Separator` and `Ink::adaptive(light, dark)` go anywhere a
/// color does, and an `Ink` is a `const`:
///
/// ```
/// use day_spec::{Color, Ink, SystemColor};
/// const INSET: Ink = Ink::adaptive(Color::rgba(0.0, 0.0, 0.0, 0.045), Color::rgba(0.0, 0.0, 0.0, 0.26));
/// const HAIRLINE: Ink = Ink::system(SystemColor::Separator);
/// const BRAND: Ink = Ink::Fixed(Color::hex(0x00e8_6a3c));
/// ```
///
/// A view keeps the ink, not a color resolved from it: the toolkit resolves it natively and
/// follows an appearance switch with no app code running. Code that needs the numbers (a
/// canvas, a contrast check) resolves an ink with `day::resolve(ink)`, a tracked read that
/// re-runs the reader when the appearance changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Ink {
    /// The same color in every appearance.
    Fixed(Color),
    /// `light` in the light appearance, `dark` in the dark one.
    Adaptive { light: Color, dark: Color },
    /// A platform role, resolved by the toolkit. `alpha`, when set, replaces the role's own
    /// opacity.
    System {
        role: SystemColor,
        alpha: Option<f64>,
    },
}

impl Default for Ink {
    /// Transparent.
    fn default() -> Self {
        Ink::Fixed(Color::CLEAR)
    }
}

impl From<Color> for Ink {
    fn from(c: Color) -> Self {
        Ink::Fixed(c)
    }
}

impl From<SystemColor> for Ink {
    fn from(role: SystemColor) -> Self {
        Ink::system(role)
    }
}

impl Ink {
    /// Transparent.
    pub const CLEAR: Ink = Ink::Fixed(Color::CLEAR);

    /// `light` in the light appearance and `dark` in the dark one.
    #[must_use]
    pub const fn adaptive(light: Color, dark: Color) -> Ink {
        Ink::Adaptive { light, dark }
    }

    /// The platform's color for `role`.
    #[must_use]
    pub const fn system(role: SystemColor) -> Ink {
        Ink::System { role, alpha: None }
    }

    /// The same ink at opacity `alpha` in every appearance: both colors of a pair, and the
    /// role's color with its own alpha replaced.
    #[must_use]
    pub const fn with_alpha(self, alpha: f64) -> Ink {
        match self {
            Ink::Fixed(c) => Ink::Fixed(Color::rgba(c.r, c.g, c.b, alpha)),
            Ink::Adaptive { light, dark } => Ink::Adaptive {
                light: Color::rgba(light.r, light.g, light.b, alpha),
                dark: Color::rgba(dark.r, dark.g, dark.b, alpha),
            },
            Ink::System { role, .. } => Ink::System {
                role,
                alpha: Some(alpha),
            },
        }
    }

    /// The color, when it is the same in every appearance.
    #[must_use]
    pub const fn fixed(self) -> Option<Color> {
        match self {
            Ink::Fixed(c) => Some(c),
            _ => None,
        }
    }

    /// Whether the ink depends on the appearance or the platform: a pair or a role.
    #[must_use]
    pub const fn is_dynamic(self) -> bool {
        !matches!(self, Ink::Fixed(_))
    }

    /// The color this ink is in `palette`'s appearance.
    #[must_use]
    pub fn resolve(self, palette: &Palette) -> Color {
        match self {
            Ink::Fixed(c) => c,
            Ink::Adaptive { light, dark } => {
                if palette.dark {
                    dark
                } else {
                    light
                }
            }
            Ink::System { role, alpha } => {
                let c = palette.color(role);
                alpha.map_or(c, |a| c.with_alpha(a))
            }
        }
    }

    /// The color this ink is right now, against the [`Palette::current`] Day keeps for the
    /// app's appearance. What a toolkit without dynamic colors applies, and re-applies when
    /// the appearance changes (docs/color.md "How a toolkit follows the appearance"). Not
    /// tracked: app code reads `day::resolve(ink)` instead.
    #[must_use]
    pub fn resolved(self) -> Color {
        self.resolve(&Palette::current())
    }

    /// The ink as three numbers for a shim in another language (Java, C++, JS): `[kind, a, b]`.
    ///
    /// | kind | `a` | `b` |
    /// | --- | --- | --- |
    /// | `0` fixed | the color as `0xAARRGGBB` | `0` |
    /// | `1` pair | the light color as `0xAARRGGBB` | the dark color |
    /// | `2` role | the role's [wire number](SystemColor::wire) | the alpha override as `0..=255`, or `256` for none |
    ///
    /// Channels are rounded to 8 bits. [`Ink::from_wire`] reads it back.
    #[must_use]
    pub fn wire(self) -> [u32; 3] {
        match self {
            Ink::Fixed(c) => [0, argb32(c), 0],
            Ink::Adaptive { light, dark } => [1, argb32(light), argb32(dark)],
            Ink::System { role, alpha } => [2, u32::from(role.wire()), alpha.map_or(256, channel8)],
        }
    }

    /// The ink [`Ink::wire`] encoded, if the numbers are one.
    #[must_use]
    pub fn from_wire(wire: [u32; 3]) -> Option<Ink> {
        match wire {
            [0, c, _] => Some(Ink::Fixed(from_argb32(c))),
            [1, l, d] => Some(Ink::adaptive(from_argb32(l), from_argb32(d))),
            [2, role, alpha] => Some(Ink::System {
                role: SystemColor::from_wire(u8::try_from(role).ok()?)?,
                alpha: (alpha <= 255).then(|| f64::from(alpha) / 255.0),
            }),
            _ => None,
        }
    }

    /// The pair this ink resolves to in the light and the dark appearance, roles answered
    /// from `light` and `dark`. What a toolkit builds one dynamic native color from.
    #[must_use]
    pub fn pair(self, light: &Palette, dark: &Palette) -> (Color, Color) {
        (self.resolve(light), self.resolve(dark))
    }

    /// A new ink made by applying `f` to this one's color in each appearance: the readable
    /// text color on a fill, a darker shade of a brand color. A fixed ink stays fixed; a pair
    /// maps both sides; a role is resolved against [`Palette::both`] first, so the result is a
    /// pair that no longer follows a later change of the system accent.
    #[must_use]
    pub fn map(self, f: impl Fn(Color) -> Color) -> Ink {
        match self {
            Ink::Fixed(c) => Ink::Fixed(f(c)),
            Ink::Adaptive { light, dark } => Ink::adaptive(f(light), f(dark)),
            Ink::System { .. } => {
                let (light, dark) = Palette::both();
                let (l, d) = self.pair(&light, &dark);
                Ink::adaptive(f(l), f(d))
            }
        }
    }
}

/// A `0..=1` channel as `0..=255`.
fn channel8(v: f64) -> u32 {
    // In range by the clamp, so the cast cannot truncate or wrap.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to 0..=255 and rounded first"
    )]
    let byte = (v.clamp(0.0, 1.0) * 255.0).round() as u32;
    byte
}

/// A color as `0xAARRGGBB`.
fn argb32(c: Color) -> u32 {
    (channel8(c.a) << 24) | (channel8(c.r) << 16) | (channel8(c.g) << 8) | channel8(c.b)
}

/// `0xAARRGGBB` as a color.
fn from_argb32(v: u32) -> Color {
    let ch = |shift: u32| f64::from((v >> shift) & 0xff) / 255.0;
    Color::rgba(ch(16), ch(8), ch(0), ch(24))
}

/// Gradient stops resolved for the current appearance ([`Ink::resolved`] on each): the form a
/// toolkit hands a native gradient that takes concrete colors.
#[must_use]
pub fn resolved_stops(stops: &[(f64, Ink)]) -> Vec<(f64, Color)> {
    stops
        .iter()
        .map(|(at, ink)| (*at, ink.resolved()))
        .collect()
}

/// The color of every [`SystemColor`] role in one appearance: what an [`Ink`] resolves
/// against. Day keeps the toolkit's answer for the app's current appearance as
/// [`Palette::current`] and refreshes it when the appearance changes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    /// Whether this is the dark appearance.
    pub dark: bool,
    /// One color per role, in [`SystemColor::ALL`] order.
    pub colors: [Color; SystemColor::COUNT],
}

thread_local! {
    static CURRENT: Cell<Option<Palette>> = const { Cell::new(None) };
}

impl Palette {
    /// Day's own palette for one appearance ([`SystemColor::fallback`] for every role).
    #[must_use]
    pub const fn fallback(dark: bool) -> Palette {
        let mut colors = [Color::CLEAR; SystemColor::COUNT];
        let mut i = 0;
        while i < SystemColor::COUNT {
            colors[i] = SystemColor::ALL[i].fallback(dark);
            i += 1;
        }
        Palette { dark, colors }
    }

    /// The color of `role`.
    #[must_use]
    pub const fn color(&self, role: SystemColor) -> Color {
        self.colors[role as usize]
    }

    /// The palette of the app's current appearance. Day sets it when the tree is installed and
    /// whenever the toolkit reports an appearance change; before that it is Day's light
    /// fallback (or the dark one under `DAY_THEME=dark`).
    #[must_use]
    pub fn current() -> Palette {
        CURRENT.with(Cell::get).unwrap_or_else(|| {
            Palette::fallback(std::env::var("DAY_THEME").ok().as_deref() == Some("dark"))
        })
    }

    /// A light and a dark palette: the [current](Palette::current) one for its own appearance
    /// and Day's [fallback](Palette::fallback) for the other, the best answer for an appearance
    /// the toolkit is not showing.
    #[must_use]
    pub fn both() -> (Palette, Palette) {
        let current = Palette::current();
        if current.dark {
            (Palette::fallback(false), current)
        } else {
            (current, Palette::fallback(true))
        }
    }

    /// Make `palette` the current one. Called by Day's core when it reads the toolkit's
    /// palette; a toolkit does not call it.
    pub fn set_current(palette: Palette) {
        CURRENT.with(|c| c.set(Some(palette)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roles_round_trip_their_wire_numbers() {
        for (i, role) in SystemColor::ALL.iter().enumerate() {
            assert_eq!(role.wire() as usize, i);
            assert_eq!(SystemColor::from_wire(role.wire()), Some(*role));
        }
        assert_eq!(SystemColor::from_wire(SystemColor::COUNT as u8), None);
    }

    #[test]
    fn inks_resolve_per_appearance() {
        let (light, dark) = (Palette::fallback(false), Palette::fallback(true));
        let pair = Ink::adaptive(Color::WHITE, Color::BLACK);
        assert_eq!(pair.resolve(&light), Color::WHITE);
        assert_eq!(pair.resolve(&dark), Color::BLACK);
        let role = Ink::from(SystemColor::Text);
        assert_eq!(role.resolve(&dark), SystemColor::Text.fallback(true));
        let wash = SystemColor::Accent.with_alpha(0.25).resolve(&light);
        assert!((wash.a - 0.25).abs() < 1e-12);
        assert_eq!(
            Ink::from(Color::BLACK).with_alpha(0.5),
            Ink::Fixed(Color::rgba(0.0, 0.0, 0.0, 0.5))
        );
        assert_eq!(pair.with_alpha(0.0).resolve(&dark).a, 0.0);
        assert!(!Ink::from(Color::WHITE).is_dynamic() && pair.is_dynamic());
    }

    #[test]
    fn inks_round_trip_their_wire_form() {
        for ink in [
            Ink::Fixed(Color::rgba(1.0, 0.0, 0.0, 0.5)),
            Ink::adaptive(Color::WHITE, Color::BLACK),
            Ink::system(SystemColor::Separator),
            SystemColor::Accent.with_alpha(0.2),
        ] {
            let back = Ink::from_wire(ink.wire()).expect("decodes");
            let (light, dark) = (Palette::fallback(false), Palette::fallback(true));
            for p in [light, dark] {
                let (a, b) = (ink.resolve(&p), back.resolve(&p));
                assert!(
                    (a.r - b.r).abs() < 0.003 && (a.a - b.a).abs() < 0.003,
                    "{ink:?}"
                );
            }
        }
        assert_eq!(Ink::system(SystemColor::Link).wire(), [2, 7, 256]);
        assert_eq!(Ink::from_wire([3, 0, 0]), None);
        assert_eq!(Ink::from_wire([2, 99, 256]), None);
    }

    #[test]
    fn the_current_palette_is_what_resolved_reads() {
        let dark = Palette::fallback(true);
        Palette::set_current(dark);
        assert_eq!(
            Ink::system(SystemColor::Background).resolved(),
            SystemColor::Background.fallback(true)
        );
        Palette::set_current(Palette::fallback(false));
        assert_eq!(
            Ink::system(SystemColor::Background).resolved(),
            Color::WHITE
        );
    }
}
