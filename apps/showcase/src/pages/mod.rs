//! One module per navigation destination in the showcase (wired up in `crate::root`).

pub(crate) mod about;
pub(crate) mod canvas;
pub(crate) mod controls;
pub(crate) mod list;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) mod map;
pub(crate) mod media;
pub(crate) mod menus;
pub(crate) mod resources;
pub(crate) mod services;
pub(crate) mod stack;
pub(crate) mod system;
pub(crate) mod tabs;
pub(crate) mod text;
pub(crate) mod tweaks;
pub(crate) mod webview;

pub(crate) use about::about_page;
pub(crate) use canvas::canvas_page;
pub(crate) use controls::controls_page;
pub(crate) use list::list_page;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) use map::map_page;
pub(crate) use media::media_page;
pub(crate) use menus::{install_app_menu, menus_page};
pub(crate) use resources::resources_page;
pub(crate) use services::services_page;
pub(crate) use stack::stack_page;
pub(crate) use system::system_page;
pub(crate) use tabs::tabs_page;
pub(crate) use text::text_page;
pub(crate) use tweaks::tweaks_page;
pub(crate) use webview::webview_page;
