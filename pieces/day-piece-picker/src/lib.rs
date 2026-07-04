//! day-piece-picker — an EXTERNAL Day Piece (DESIGN.md §15 tier 1): one Rust API, three SwiftUI-style
//! stylings (`.menu`, `.segmented`, `.inline`) each realized as a NATIVE control per toolkit, registered
//! link-time into each backend's renderer slice with **zero edits** to day. Bound two-way to a selection.

use day_core::{BuildCx, Flex, Piece, RNode, with_tree};
use day_pieces::SignalRw;
use day_reactive::{Signal, bind_seeded};
use day_spec::Event;

pub const KIND: &str = "day.piece.picker";

/// SwiftUI's `pickerStyle` analogue. Each maps to a distinct native control per toolkit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickerStyle {
    /// A dropdown/pop-up menu (NSPopUpButton / GtkDropDown / QComboBox / UIButton+UIMenu / Spinner).
    #[default]
    Menu,
    /// A horizontal segmented control (NSSegmentedControl / UISegmentedControl / linked toggles / …).
    Segmented,
    /// A vertical radio-button group laid out inline (NSButton radios / GtkCheckButton group / …).
    Inline,
}

/// Full props (realize). `options`/`style` are set once at build; only `selected` patches.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickerProps {
    pub options: Vec<String>,
    pub selected: usize,
    pub style: PickerStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PickerPatch {
    Selected(usize),
}

/// A native picker bound two-way to `selected`. Style via `.menu()`/`.segmented()`/`.inline()`.
pub struct Picker {
    options: Vec<String>,
    selected: Signal<usize>,
    style: PickerStyle,
}

/// `picker(["A", "B", "C"], choice).segmented()` — options are fixed, `selected` is the bound index.
pub fn picker<S: Into<String>>(
    options: impl IntoIterator<Item = S>,
    selected: Signal<usize>,
) -> Picker {
    Picker {
        options: options.into_iter().map(Into::into).collect(),
        selected,
        style: PickerStyle::Menu,
    }
}

impl Picker {
    pub fn menu(mut self) -> Self {
        self.style = PickerStyle::Menu;
        self
    }
    pub fn segmented(mut self) -> Self {
        self.style = PickerStyle::Segmented;
        self
    }
    pub fn inline(mut self) -> Self {
        self.style = PickerStyle::Inline;
        self
    }
    pub fn style(mut self, style: PickerStyle) -> Self {
        self.style = style;
        self
    }
}

impl Piece for Picker {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Picker {
            options,
            selected,
            style,
        } = self;
        let initial = PickerProps {
            options,
            selected: selected.get_untracked(),
            style,
        };
        let node = cx.leaf(KIND, &initial, Flex::default());
        bind_seeded(
            initial.selected,
            move || selected.get(),
            move |v: &usize| {
                with_tree(|t| t.patch(node, Box::new(PickerPatch::Selected(*v)), false));
            },
        );
        cx.on(node, move |ev| {
            if let Event::SelectionChanged(i) = ev
                && *i >= 0
            {
                selected.set_rw(*i as usize);
            }
        });
        node
    }
}

// ---------------------------------------------------------------------------
// Per-toolkit native renderers — one file per backend (this crate is a reference
// implementation, so each toolkit is split out for clarity). Every module registers a
// `Renderer` link-time into its backend's `RENDERERS` slice; the `#[cfg]` gates each to
// its feature + target, and `#[path]` keeps the files grouped next to lib.rs.
// ---------------------------------------------------------------------------

#[cfg(all(feature = "appkit", target_os = "macos"))]
#[path = "lib-appkit.rs"]
mod appkit_impl;

#[cfg(feature = "gtk")]
#[path = "lib-gtk.rs"]
mod gtk_impl;

#[cfg(feature = "qt")]
#[path = "lib-qt.rs"]
mod qt_impl;

#[cfg(all(feature = "uikit", target_os = "ios"))]
#[path = "lib-uikit.rs"]
mod uikit_impl;

#[cfg(all(feature = "widget", target_os = "android"))]
#[path = "lib-android.rs"]
mod android_impl;

#[cfg(all(feature = "winui", windows))]
#[path = "lib-winui.rs"]
mod winui_impl;
