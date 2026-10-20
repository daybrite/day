// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Design tokens (docs/api-style.md "Tokens"): the semantic spacing scale every container,
//! padding and wrap takes beside a raw point value, and composable text presets an app names
//! once.

use day_spec::{Font, FontWeight, Insets};

use crate::IntoInsets;

/// A semantic spacing step, accepted wherever a container or modifier takes a gap or an inset
/// in points: `column(..).spacing(Space::S)`, `.padding(Space::M)`; an `Insets` constructor
/// takes `Space::L.points()`.
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

impl IntoInsets for Space {
    fn into_insets(self) -> Insets {
        Insets::all(self.points())
    }
}

/// A named text style an app applies to many labels: `label(t).preset(&CAPTION)` for
/// `const CAPTION: TextPreset = TextPreset::new(Font::Caption).secondary();`. Every builder is
/// `const`, so a preset is a constant, not a runtime value, and it composes the same
/// `LabelBuilder` steps a chain would (docs/text.md).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextPreset {
    pub(crate) font: Font,
    pub(crate) font_scale: f64,
    pub(crate) weight: Option<FontWeight>,
    pub(crate) italic: bool,
    pub(crate) tabular: bool,
    pub(crate) monospace: bool,
    pub(crate) secondary: bool,
    pub(crate) single_line: bool,
    pub(crate) color: Option<day_spec::Color>,
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
    /// A fixed text color. Prefer [`secondary`](Self::secondary) for de-emphasis, which follows
    /// the appearance; a fixed color is for meaning (a warning, a brand).
    pub const fn color(mut self, color: day_spec::Color) -> TextPreset {
        self.color = Some(color);
        self
    }
}
