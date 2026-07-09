//! One module per navigation destination in the showcase (wired up in `crate::root`).

pub(crate) mod about;
pub(crate) mod activity;
pub(crate) mod battery;
pub(crate) mod clipboard;
pub(crate) mod compose;
pub(crate) mod controls;
pub(crate) mod deviceinfo;
pub(crate) mod files;
pub(crate) mod gauge;
pub(crate) mod haptics;
pub(crate) mod list;
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) mod lottie;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) mod map;
pub(crate) mod media;
pub(crate) mod menus;
pub(crate) mod modals;
pub(crate) mod network;
pub(crate) mod pickers;
pub(crate) mod prefs;
pub(crate) mod resources;
pub(crate) mod search;
pub(crate) mod sensors;
pub(crate) mod shapes;
pub(crate) mod stack;
pub(crate) mod tabs;
pub(crate) mod text;
pub(crate) mod tweaks;
pub(crate) mod webview;

pub(crate) use about::about_page;
pub(crate) use activity::activity_page;
pub(crate) use battery::battery_page;
pub(crate) use clipboard::clipboard_page;
pub(crate) use compose::compose_page;
pub(crate) use controls::controls_page;
pub(crate) use deviceinfo::deviceinfo_page;
pub(crate) use files::files_page;
pub(crate) use gauge::gauge_page;
pub(crate) use haptics::haptics_page;
pub(crate) use list::list_page;
#[cfg(any(target_os = "ios", target_os = "android"))]
pub(crate) use lottie::lottie_page;
#[cfg(any(target_os = "macos", target_os = "ios"))]
pub(crate) use map::map_page;
pub(crate) use media::media_page;
pub(crate) use menus::{install_app_menu, menus_page};
pub(crate) use modals::modals_page;
pub(crate) use network::network_page;
pub(crate) use pickers::pickers_page;
pub(crate) use prefs::prefs_page;
pub(crate) use resources::resources_page;
pub(crate) use search::search_page;
pub(crate) use sensors::sensors_page;
pub(crate) use shapes::shapes_page;
pub(crate) use stack::stack_page;
pub(crate) use tabs::tabs_page;
pub(crate) use text::text_page;
pub(crate) use tweaks::tweaks_page;
pub(crate) use webview::webview_page;
