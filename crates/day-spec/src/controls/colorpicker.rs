// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use crate::Color;

pub const KIND: &str = "day.piece.colorpicker";

/// The tag every in-process native arm reports a pick under. Across a JNI / C-ABI / JS boundary
/// the tag cannot be a `&'static str`, so it arrives empty and only the payload matters (§8.2);
/// the front-end reads the text either way.
pub const PICK_TAG: &str = "colorpicker:value";

/// Full props (realize) for the native leaf. `alpha` and `title` are set once at build; only
/// `color` patches. The composed idiom realizes no leaf and never builds these.
#[derive(Clone, Debug, PartialEq)]
pub struct ColorProps {
    pub color: Color,
    /// Offer an opacity channel in the chooser. Off by default: most pickers hide the alpha
    /// slider unless asked, and a color that can go transparent behind the app's back is rarely
    /// what a tint or a brand swatch wants.
    pub alpha: bool,
    /// The chooser's title, where the platform shows one (`""` = the platform default).
    pub title: String,
}

impl Default for ColorProps {
    fn default() -> Self {
        ColorProps {
            color: Color::BLACK,
            alpha: false,
            title: String::new(),
        }
    }
}

/// The single imperative update: show `color` in the well (and in the chooser, if it is open).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ColorPatch {
    SetColor(Color),
}
