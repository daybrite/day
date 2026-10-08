// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Status items (docs/status-item.md): an icon in the macOS menu bar, the Windows notification
//! area or a Linux tray, with a menu, built reactively like `app_menu_reactive`.

use std::rc::Rc;

use day_reactive::Scope;
use day_spec::{Icon, StatusImage, StatusItemSpec, Symbol};

use crate::MenuEntry;
use crate::menus::lower_menu;

/// What a status item shows: its icon, text and menu. Built by the closure handed to
/// [`status_item`], which re-runs whenever a signal it reads changes.
#[derive(Default)]
pub struct StatusItem {
    icon: Option<Icon>,
    raster: Option<StatusImage>,
    template: bool,
    title: String,
    tooltip: String,
    menu: Vec<MenuEntry>,
    on_activate: Option<Rc<dyn Fn()>>,
}

impl StatusItem {
    /// An item with nothing set yet.
    pub fn new() -> StatusItem {
        StatusItem::default()
    }

    /// A platform symbol as the icon, drawn in the menu bar's or panel's own color.
    pub fn icon(mut self, symbol: Symbol) -> StatusItem {
        self.icon = Some(Icon::Symbol(symbol));
        self.template = true;
        self
    }

    /// A bundled vector glyph as the icon (`res::vectors::…`), drawn as a template: the shape
    /// in the menu bar's or panel's own color. The usual choice for a status item.
    pub fn vector(mut self, name: impl Into<day_spec::VectorName>) -> StatusItem {
        self.icon = Some(Icon::Image(name.into().as_str().to_owned()));
        self.template = true;
        self
    }

    /// A bundled image as the icon (`res::images::…`), drawn as it is. Call [`template`]
    /// for a monochrome glyph that should take the menu bar's color instead.
    ///
    /// [`template`]: StatusItem::template
    pub fn image(mut self, name: impl Into<day_spec::ImageName>) -> StatusItem {
        self.icon = Some(Icon::Image(name.into().as_str().to_owned()));
        self
    }

    /// A runtime RGBA image, preserving its logical width (a chart or compact status grid).
    /// AppKit renders this in color; `.template(true)` opts into a monochrome template.
    /// Other backends keep using the `icon`/`vector`/`image` fallback, if supplied. The image
    /// owns its pixels and survives closing the window that produced it.
    pub fn raster(mut self, image: StatusImage) -> Self {
        self.raster = Some(image);
        self.template = false;
        self
    }

    /// Draw the image by its shape alone, in the menu bar's or panel's own color, so one glyph
    /// reads in light and dark (a macOS template image).
    pub fn template(mut self, template: bool) -> StatusItem {
        self.template = template;
        self
    }

    /// Text beside the icon where the platform shows it (the macOS menu bar): a timer, a count,
    /// a short status.
    pub fn title(mut self, title: impl Into<String>) -> StatusItem {
        self.title = title.into();
        self
    }

    /// The hover text.
    pub fn tooltip(mut self, tooltip: impl Into<String>) -> StatusItem {
        self.tooltip = tooltip.into();
        self
    }

    /// The item's menu, built from the same entries as `app_menu`. Include a
    /// `menu_role(MenuRole::Quit)` (or an item calling `day::quit()`) in an app that keeps
    /// running without a window, or the user has no way to end it.
    pub fn menu(mut self, entries: Vec<MenuEntry>) -> StatusItem {
        self.menu = entries;
        self
    }

    /// Run `f` when the user clicks the item itself, where the platform tells a click apart
    /// from opening the menu (a left click on macOS and Windows; a Linux tray's primary
    /// action). The menu then opens on a right click. Without it, any click opens the menu.
    pub fn on_activate(mut self, f: impl Fn() + 'static) -> StatusItem {
        self.on_activate = Some(Rc::new(f));
        self
    }

    fn lower(self, id: &str) -> StatusItemSpec {
        StatusItemSpec {
            id: id.to_owned(),
            icon: self.icon,
            raster: self.raster,
            template: self.template,
            title: self.title,
            tooltip: self.tooltip,
            menu: lower_menu(self.menu),
            activate: self.on_activate.map_or(0, day_core::register_menu_action),
        }
    }
}

/// A shown status item. Dropping the handle leaves the item up; call [`remove`] to take it
/// down.
///
/// [`remove`]: StatusItemHandle::remove
pub struct StatusItemHandle {
    id: String,
    scope: Scope,
}

impl StatusItemHandle {
    /// Take the item down and stop rebuilding it.
    pub fn remove(self) {
        self.scope.dispose();
        day_core::remove_status_item(&self.id);
    }
}

/// Show a status item (docs/status-item.md): an icon in the macOS menu bar, the Windows
/// notification area, or a Linux tray. `build` runs now and again whenever a signal it reads
/// changes, so a timer's title or a "Pause sync" check mark follows the app's state. `id` names
/// the item; showing a second item with the same id replaces the first.
///
/// An app with a status item keeps running when its last window closes (`KeepRunning::
/// Automatic`, docs/windows.md); its menu should offer a way to quit. Probe
/// `capability(Cap::StatusItem)`: a phone and the web show nothing, and on Linux the item needs a
/// running tray host, which stock GNOME lacks.
pub fn status_item(id: &str, build: impl Fn() -> StatusItem + 'static) -> StatusItemHandle {
    let scope = Scope::root().enter(Scope::child);
    let key = id.to_owned();
    scope.enter(|| {
        day_reactive::bind(
            move || {
                let _ = day_l10n::locale().get();
                build().lower(&key)
            },
            |spec: &StatusItemSpec| day_core::set_status_item(spec.clone()),
        );
    });
    StatusItemHandle {
        id: id.to_owned(),
        scope,
    }
}
