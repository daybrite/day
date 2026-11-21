// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Design tokens (docs/api-style.md "Tokens"): the semantic spacing scale every container,
//! padding, frame and wrap takes beside a raw point value, the corner radius scale every
//! rounded view and shape takes, and composable text presets an app names once.

use day_spec::{Font, FontWeight, Insets};

/// A semantic spacing step, accepted wherever a container or modifier takes a gap, an inset or
/// a length in points: `column(..).spacing(Space::S)`, `.padding(Space::M)`,
/// `.frame(Space::XL, Space::XL)`, `.min_width(Space::XL)`; an `Insets` constructor takes
/// `Space::L.points()`.
///
/// Fonts are semantic (`Font::Headline`) and the house rule forbids point sizes; spacing is the
/// same kind of thing. A step resolves to a point value per platform, so the platforms have room
/// to differ and an app has one vocabulary. Today every platform resolves the same scale: 4, 8,
/// 12, 16 and 24 points. A raw `f64` stays accepted everywhere a `Space` is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Space {
    /// The tightest gap: between an icon and its label, between a caption and its value.
    XS,
    /// The default gap between siblings in a row or a short column.
    S,
    /// The gap between groups within a page section.
    M,
    /// A page's edge inset, and the gap between sections.
    L,
    /// The gap between major regions of a page.
    XL,
}

impl Space {
    /// The step in points on this platform.
    #[must_use]
    pub const fn points(self) -> f64 {
        match self {
            Space::XS => 4.0,
            Space::S => 8.0,
            Space::M => 12.0,
            Space::L => 16.0,
            Space::XL => 24.0,
        }
    }
}

impl From<Space> for f64 {
    fn from(s: Space) -> f64 {
        s.points()
    }
}

impl From<Space> for Insets {
    fn from(s: Space) -> Insets {
        Insets::all(s.points())
    }
}

/// A semantic corner radius step, accepted wherever a corner radius is: `.corner_radius(Radius::M)`,
/// `Surface::new().corner_radius(Radius::L)`, `rounded_rectangle(Radius::S)`. Like [`Space`], a
/// step resolves to points per platform; today every platform resolves 4, 6, 10, 14 and 20.
/// [`Corner::CAPSULE`](crate::Corner::CAPSULE) and
/// [`Corner::Fraction`](crate::Corner::Fraction) are the frame-relative radii beside them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Radius {
    /// A small control's corner: a checkbox, a tag.
    XS,
    /// A field's or a button's corner.
    S,
    /// A card's corner, what `.card()` draws.
    M,
    /// A sheet's or a large panel's corner.
    L,
    /// A hero tile's corner.
    XL,
}

impl Radius {
    /// The step in points on this platform.
    #[must_use]
    pub const fn points(self) -> f64 {
        match self {
            Radius::XS => 4.0,
            Radius::S => 6.0,
            Radius::M => 10.0,
            Radius::L => 14.0,
            Radius::XL => 20.0,
        }
    }
}

impl From<Radius> for f64 {
    fn from(r: Radius) -> f64 {
        r.points()
    }
}

/// A named text style an app applies to many labels: `label(t).preset(&CAPTION)` for
/// `const CAPTION: TextPreset = TextPreset::new(Font::Caption).secondary();`. Every builder is
/// `const`, so a preset is a constant, not a runtime value, and it composes the same
/// `LabelBuilder` steps a chain would (docs/text.md).
#[derive(Clone, Copy, Debug, PartialEq)]
#[must_use]
pub struct TextPreset {
    pub(crate) font: Font,
    pub(crate) font_scale: f64,
    pub(crate) weight: Option<FontWeight>,
    pub(crate) italic: bool,
    pub(crate) tabular: bool,
    pub(crate) monospace: bool,
    pub(crate) secondary: bool,
    pub(crate) single_line: bool,
    pub(crate) color: Option<day_spec::Ink>,
    pub(crate) small_caps: bool,
    pub(crate) letter_spacing: Option<f64>,
    pub(crate) line_height: Option<f64>,
    pub(crate) max_text_scale: Option<f64>,
}

impl TextPreset {
    /// A preset in `font`, with nothing else set.
    pub const fn new(font: Font) -> TextPreset {
        TextPreset {
            font,
            font_scale: 1.0,
            weight: None,
            italic: false,
            tabular: false,
            monospace: false,
            secondary: false,
            single_line: false,
            color: None,
            small_caps: false,
            letter_spacing: None,
            line_height: None,
            max_text_scale: None,
        }
    }
    /// Scale the resolved font, as [`LabelBuilder::font_scale`](crate::LabelBuilder::font_scale).
    pub const fn font_scale(mut self, scale: f64) -> TextPreset {
        self.font_scale = scale;
        self
    }
    /// The font weight, as [`LabelBuilder::weight`](crate::LabelBuilder::weight).
    pub const fn weight(mut self, weight: FontWeight) -> TextPreset {
        self.weight = Some(weight);
        self
    }
    /// Shorthand for `.weight(FontWeight::Bold)`.
    pub const fn bold(self) -> TextPreset {
        self.weight(FontWeight::Bold)
    }
    pub const fn italic(mut self) -> TextPreset {
        self.italic = true;
        self
    }
    pub const fn tabular(mut self) -> TextPreset {
        self.tabular = true;
        self
    }
    pub const fn monospace(mut self) -> TextPreset {
        self.monospace = true;
        self
    }
    /// The platform's secondary label color, as [`Label::secondary`](crate::Label::secondary).
    pub const fn secondary(mut self) -> TextPreset {
        self.secondary = true;
        self
    }
    pub const fn single_line(mut self) -> TextPreset {
        self.single_line = true;
        self
    }
    /// The text's ink: a role (`Ink::system(SystemColor::Warning)`), a light/dark pair, or a
    /// fixed color (`Ink::Fixed(BRAND)`). `const`, so it takes an [`Ink`](day_spec::Ink)
    /// rather than anything that converts to one. Prefer [`secondary`](Self::secondary) for
    /// de-emphasis.
    pub const fn color(mut self, color: day_spec::Ink) -> TextPreset {
        self.color = Some(color);
        self
    }
    /// Small capitals, as [`LabelBuilder::small_caps`](crate::LabelBuilder::small_caps).
    pub const fn small_caps(mut self) -> TextPreset {
        self.small_caps = true;
        self
    }
    /// Letter spacing in ems, as
    /// [`LabelBuilder::letter_spacing`](crate::LabelBuilder::letter_spacing).
    pub const fn letter_spacing(mut self, ems: f64) -> TextPreset {
        self.letter_spacing = Some(ems);
        self
    }
    /// Line height as a multiple of the font size, as
    /// [`LabelBuilder::line_height`](crate::LabelBuilder::line_height).
    pub const fn line_height(mut self, multiple: f64) -> TextPreset {
        self.line_height = Some(multiple);
        self
    }
    /// A cap on growth with the system text size, as
    /// [`LabelBuilder::max_text_scale`](crate::LabelBuilder::max_text_scale).
    pub const fn max_text_scale(mut self, cap: f64) -> TextPreset {
        self.max_text_scale = Some(cap);
        self
    }
}
