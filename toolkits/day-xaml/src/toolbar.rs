// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// ---------------------------------------------------------------------------
// XAML: a Fluent `CommandBar` docked under the menu bar (docs/toolbars.md). That is the Windows
// toolbar — the commands are AppBarButtons, so they take the icon size, the label position and
// the pressed/checked visuals of every other Fluent app, which is why nothing here styles them.
// The CommandBar template right-aligns `PrimaryCommands` and left-aligns `Content`, so the
// model's flexible space IS that split: items before it become leading `Content`, items after it
// become primary commands — the same rule the GTK backend applies with pack_start/pack_end.
//
// The whole model crosses the FFI as ONE tab-separated blob, exactly like the menu spec next
// door: `serialize_toolbar` writes it, the shim's `day_xaml_set_toolbar` parses it, and a menu
// item's own spec nests inside it (see the format comment on `serialize_toolbar`).
// ---------------------------------------------------------------------------

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

use day_spec::{Event, Icon, Symbol, ToolbarItem, ToolbarItemKind, ToolbarPatch, ToolbarValue};
use day_xaml_sys as ffi;

use crate::{WinHandle, Xaml, cstr, emit, icon_file_name, serialize_menu_xaml};

/// The Segoe Fluent Icons code point each standard symbol draws as, as hex — the shim turns it
/// into the `FontIcon` glyph. Windows 10 ships this font's predecessor, Segoe MDL2 Assets, under
/// the same code points for every glyph here, so one table serves both releases (the shim picks
/// the family that is installed).
/// The Segoe Fluent glyph for a symbol — shared with the menu builder (docs/menus.md).
pub(crate) fn glyph_for(s: Symbol) -> &'static str {
    glyph(s)
}

fn glyph(s: Symbol) -> &'static str {
    match s {
        Symbol::Add => "E710",
        Symbol::Remove => "E738",
        Symbol::Delete => "E74D",
        Symbol::Edit => "E70F",
        Symbol::New => "E7C3",
        Symbol::Open => "E8E5",
        Symbol::Save => "E74E",
        Symbol::Print => "E749",
        Symbol::Refresh => "E72C",
        Symbol::Search => "E721",
        Symbol::Share => "E72D",
        Symbol::Settings => "E713",
        Symbol::Info => "E946",
        Symbol::Star => "E734",
        Symbol::Bookmark => "E8A4",
        Symbol::Back => "E72B",
        Symbol::Forward => "E72A",
        // Chevrons rather than arrows, matching the AppKit mapping — Up/Down are the "move
        // through this list" commands, not a file transfer.
        Symbol::Up => "E70E",
        Symbol::Down => "E70D",
        Symbol::Home => "E80F",
        // The hamburger: on Windows the pane toggle is what shows and hides a sidebar.
        Symbol::Sidebar => "E700",
        Symbol::Filter => "E71C",
        Symbol::Sort => "E8CB",
        Symbol::More => "E712",
        Symbol::Play => "E768",
        Symbol::Pause => "E769",
        Symbol::Stop => "E71A",
        Symbol::Camera => "E722",
        Symbol::Code => "E943",
        Symbol::Light => "E706",
        Symbol::Dark => "E708",
        Symbol::Auto => "E793",
        Symbol::ZoomIn => "E8A3",
        Symbol::ZoomOut => "E71F",
        Symbol::Undo => "E7A7",
        Symbol::Redo => "E7A6",
        Symbol::Copy => "E8C8",
        Symbol::Cut => "E8C6",
        Symbol::Paste => "E77F",
        Symbol::Mail => "E715",
        Symbol::Folder => "E8B7",
        Symbol::Document => "E8A5",
        Symbol::Check => "E73E",
        Symbol::Close => "E711",
        Symbol::Warning => "E7BA",
        // Segoe Fluent's own shape glyphs (the Paint/Whiteboard shape vocabulary).
        Symbol::Rectangle => "E739",
        Symbol::Oval => "E91F",
        // The vocabulary is `#[non_exhaustive]`: an unmapped symbol gets no glyph rather than an
        // arbitrary wrong one — the item still shows its label.
        _ => "",
    }
}

/// Values from the shim: kind 0 = a toggle's new state, kind 1 = a search field's text.
pub(crate) extern "C" fn on_toolbar_value(
    action: u64,
    kind: c_int,
    on: c_int,
    text: *const c_char,
) {
    // Contained: a panic unwinding into the C++/WinRT shim frame is UB (day-spec's ffi_guard).
    day_spec::ffi_guard::contain((), || {
        let value = if kind == 0 {
            ToolbarValue::On(on != 0)
        } else {
            let text = unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned();
            ToolbarValue::Text(text)
        };
        emit(
            day_spec::WINDOW_NODE,
            Event::ToolbarChanged { action, value },
        );
    });
}

/// Tabs and newlines are the record separators, so they can never appear inside a field.
fn clean(s: &str) -> String {
    s.replace(['\t', '\n'], " ")
}

/// Serialize the toolbar model to the shim's line format — one item per line:
///
/// `kind \t id \t action \t enabled \t on \t glyph \t image \t label \t tooltip \t text \t placeholder \t suggestions \t geom`
///
/// with kinds `B` button, `T` toggle, `M` menu, `F` search field, `L` label, `-` separator,
/// `_` fixed space and `>` flexible space (the Content/PrimaryCommands split). `on` seeds a
/// toggle and `text` a search field; `glyph` is a Segoe Fluent Icons code point in hex and
/// `image` a bundled image FILE NAME (the shim loads it as `ms-appx:///images/<file>`).
///
/// An `M` line is followed by that item's MENU spec — the very lines [`serialize_menu_xaml`]
/// already writes for the menu bar — closed by an `X` line, so the shim can slice the sub-spec
/// out and hand it to its own `build_menu_items`. One blob, one entry point, both formats.
fn serialize_toolbar(items: &[ToolbarItem]) -> String {
    let mut out = String::new();
    for item in items {
        // Completions ride a 12th field, unit-separated — tabs and newlines are the record
        // separators, so they can never appear inside one (docs/search.md).
        let mut sug = String::new();
        let (kind, on, text, placeholder) = match &item.kind {
            ToolbarItemKind::Button => ("B", 0, "", ""),
            ToolbarItemKind::Toggle { on } => ("T", *on as i32, "", ""),
            // "G" — the segment lines follow, as a menu's items do, closed by the same `X`.
            // `on` carries the selected index.
            ToolbarItemKind::Segmented { selected, .. } => ("G", *selected as i32, "", ""),
            ToolbarItemKind::Menu { .. } => ("M", 0, "", ""),
            ToolbarItemKind::Search {
                text,
                placeholder,
                suggestions,
            } => {
                sug = suggestions.join("\x1f");
                ("F", 0, text.as_str(), placeholder.as_str())
            }
            // "S" — the shim maps it to NavigationView.IsPaneOpen (docs/toolbars.md).
            ToolbarItemKind::SidebarToggle => ("S", 0, "", ""),
            ToolbarItemKind::Label => ("L", 0, "", ""),
            ToolbarItemKind::Separator => ("-", 0, "", ""),
            ToolbarItemKind::Space => ("_", 0, "", ""),
            ToolbarItemKind::FlexibleSpace => (">", 0, "", ""),
        };
        // A bundled image crosses as the staged file's name, the same way nav icons do; only one
        // of the two icon fields is ever set.
        let (glyph_hex, image) = match &item.icon {
            Some(Icon::Symbol(s)) => (glyph(*s), String::new()),
            Some(Icon::Image(name)) => ("", icon_file_name(name)),
            None => ("", String::new()),
        };
        // …and its VECTOR form rides a 13th field, exactly as a nav row's does: the shim prefers
        // geometry and keeps `image` as the fallback for art that would not convert. Without this
        // a vector-only icon had nothing to draw at all — the raster is deliberately not staged
        // for a toolkit that renders vectors (docs/vectors.md), so the slot came out empty.
        //
        // BOTH of this line format's separators have to be escaped, because a `.xamlgeom` spec
        // contains both: newlines between shapes, and a TAB between a shape's paint attributes
        // and its path data. Passing it through `clean` instead (which turns them into spaces)
        // silently destroys the path data and the geometry parses to nothing at all — a blank
        // icon slot, which is not distinguishable from "this glyph did not convert".
        let geom = match &item.icon {
            Some(Icon::Image(name)) => crate::vector_geometry(name)
                .map(|s| s.replace('\n', "\x1f").replace('\t', "\x1e"))
                .unwrap_or_default(),
            _ => String::new(),
        };
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
            kind,
            clean(&item.id),
            item.action,
            item.enabled as i32,
            on,
            glyph_hex,
            image,
            clean(&item.label),
            clean(item.tooltip.as_deref().unwrap_or(&item.label)),
            clean(text),
            clean(placeholder),
            clean(&sug),
            geom, // already escaped above; `clean` would eat the spec's own tabs
        ));
        if let ToolbarItemKind::Menu { items } = &item.kind {
            serialize_menu_xaml(items, &mut out);
            out.push_str("X\t\n");
        }
        if let ToolbarItemKind::Segmented { segments, .. } = &item.kind {
            for seg in segments {
                let (g, img) = match &seg.icon {
                    Some(Icon::Symbol(s)) => (glyph(*s), String::new()),
                    Some(Icon::Image(name)) => ("", icon_file_name(name)),
                    None => ("", String::new()),
                };
                out.push_str(&format!("g\t{}\t{}\t{}\n", g, img, clean(&seg.title)));
            }
            out.push_str("X\t\n");
        }
    }
    out
}

impl Xaml {
    /// Install `items` as the window's toolbar (docs/toolbars.md). An empty slice removes it.
    pub(crate) fn install_toolbar(&mut self, h: &WinHandle, items: &[ToolbarItem]) {
        // Into the window that asked for it. Secondary windows carry the same docked chrome as
        // the primary (docs/windows.md), so an app that installs a toolbar per window — which is
        // what `register_new_window` builders do — gets one in each.
        let Some(win) = self.window_token(h) else {
            return;
        };
        let spec = serialize_toolbar(items);
        unsafe {
            if win == self.window {
                ffi::day_xaml_set_toolbar(win, cstr(&spec).as_ptr());
            } else {
                ffi::day_xaml_window_set_toolbar2(win, cstr(&spec).as_ptr());
            }
        }
    }

    /// Apply a targeted change to one live item, in the window that owns it — item ids repeat
    /// across windows, so the window is half the address.
    pub(crate) fn patch_toolbar(&mut self, h: &WinHandle, patch: &ToolbarPatch) {
        let Some(win) = self.window_token(h) else {
            return;
        };
        match patch {
            ToolbarPatch::Text { item, text } => {
                let (id, text) = (cstr(item), cstr(text));
                unsafe { ffi::day_xaml_toolbar_set_text(win, id.as_ptr(), text.as_ptr()) };
            }
            ToolbarPatch::On { item, on } => {
                let id = cstr(item);
                unsafe { ffi::day_xaml_toolbar_set_checked(win, id.as_ptr(), *on as c_int) };
            }
            ToolbarPatch::Selected { item, index } => {
                let id = cstr(item);
                unsafe { ffi::day_xaml_toolbar_set_selected(win, id.as_ptr(), *index as c_int) };
            }
            ToolbarPatch::Suggestions { item, list } => {
                let (id, joined) = (cstr(item), cstr(&list.join("\x1f")));
                unsafe { ffi::day_xaml_toolbar_set_suggestions(win, id.as_ptr(), joined.as_ptr()) };
            }
            ToolbarPatch::Enabled { item, on } => {
                let id = cstr(item);
                unsafe { ffi::day_xaml_toolbar_set_enabled(win, id.as_ptr(), *on as c_int) };
            }
        }
    }
}
