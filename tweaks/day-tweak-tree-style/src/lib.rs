// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-tweak-tree-style — native styling for day's `tree(…)` (docs/tree.md, docs/tweaks.md):
//! the platform treatments a tree can wear without the piece growing options for them.
//!
//! | toolkit | mechanism | sidebar | alternating rows |
//! |---|---|---|---|
//! | AppKit | objc2 (`NSOutlineView` via `Subcontrol::Content`) | ✓ (clear backgrounds — the pane shows through, the source-list look) | ✓ (`usesAlternatingRowBackgroundColors`) |
//! | GTK | gtk4-rs (`navigation-sidebar` CSS class on the `GtkListView`) | ✓ | ✗ (GTK4 lists have no stock zebra class) |
//! | UIKit / Qt / web / Android / ArkUI / XAML | — documented no-op: UIKit's list config styles per-cell, and the emulated trees render day pieces an app already styles directly | | |
//!
//! ```ignore
//! use day_tweak_tree_style::{TreeStyle, TreeStyleTweak};
//! tree(source, row_view).tree_style(TreeStyle::sidebar())
//! ```
//!
//! The styling is UNMANAGED (day never patches it), so it survives reloads and expansion
//! patches. Where the table says ✗ or no-op, that's the platform's reality — reported here
//! rather than faked.

use day_core::RNode;
use day_pieces::Decorate;

/// The tree treatments to apply. Combine with the builder methods; each backend applies
/// what it supports (see the crate table).
#[derive(Clone, Copy, Debug, Default)]
pub struct TreeStyle {
    /// The sidebar look: the tree's own background goes clear so the hosting pane shows
    /// through (AppKit), or the platform's sidebar list class applies (GTK).
    pub sidebar: bool,
    /// Zebra striping via the platform's own alternating-row rendering (AppKit).
    pub alternating_rows: bool,
}

impl TreeStyle {
    /// The sidebar treatment alone — what a leading layer/navigator pane usually wants.
    pub fn sidebar() -> Self {
        TreeStyle {
            sidebar: true,
            ..Default::default()
        }
    }
    /// Alternating row backgrounds alone.
    pub fn alternating() -> Self {
        TreeStyle {
            alternating_rows: true,
            ..Default::default()
        }
    }
    pub fn with_sidebar(mut self, on: bool) -> Self {
        self.sidebar = on;
        self
    }
    pub fn with_alternating_rows(mut self, on: bool) -> Self {
        self.alternating_rows = on;
        self
    }
}

/// `.tree_style(…)` on any piece whose native widget is a tree (i.e. `tree(…)`).
pub trait TreeStyleTweak: Decorate + Sized {
    #[allow(unused_variables)]
    fn tree_style(self, style: TreeStyle) -> day_pieces::Decorated<Self> {
        self.tweak(move |n| apply(n, style))
    }
}

impl<P: Decorate> TreeStyleTweak for P {}

#[allow(unused_variables)]
fn apply(node: RNode, s: TreeStyle) {
    #[cfg(feature = "appkit")]
    {
        use objc2_app_kit::{NSOutlineView, NSScrollView};
        // The outline lives inside the tree node's scroller — `Subcontrol::Content`
        // (docs/tree.md "Customization"); on a backend whose tree is COMPOSED the node is a
        // list of day pieces, the downcast misses, and the tweak is the documented no-op.
        let _ = day_appkit::with_native_subcontrol(
            node,
            day_spec::Subcontrol::Content,
            |view, _class, _mtm| {
                if let Some(outline) = view.downcast_ref::<NSOutlineView>() {
                    outline.setUsesAlternatingRowBackgroundColors(s.alternating_rows);
                }
            },
        );
        if s.sidebar {
            let _ = day_appkit::with_native(node, |host, _class, _mtm| {
                if let Some(sv) = host.downcast_ref::<NSScrollView>() {
                    // Clear the scroller's own fill; the hosting pane's background shows
                    // through — the source-list look without NSTableViewStyle::Inset, which
                    // is unusable under day's fixed-frame layout (docs/tree.md M1 notes).
                    sv.setDrawsBackground(false);
                }
            });
        }
    }
    #[cfg(feature = "gtk")]
    {
        use gtk4::prelude::*;
        if s.sidebar {
            let _ = day_gtk::with_native(node, |w, _class| {
                if let Some(sw) = w.downcast_ref::<gtk4::ScrolledWindow>()
                    && let Some(child) = sw.child()
                    && child.is::<gtk4::ListView>()
                {
                    // Adwaita's own sidebar list treatment — row shapes, selection fill,
                    // spacing (the class GNOME apps put on their nav lists).
                    child.add_css_class("navigation-sidebar");
                }
            });
        }
        // alternating_rows: GTK4 ships no stock zebra class for GtkListView — documented, not
        // emulated.
    }
}
