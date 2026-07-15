//! {{title}} — a [Day](https://daybrite.dev) app. `root()` is the whole UI, shared by every
//! platform; each navigation destination lives in its own module under `pages/`.

use day::prelude::*;

mod pages;
use crate::pages::*;

/// Typed constants for the files under `resource/`, generated at build time by `day-build` (§18.5):
/// `res::images::<stem>`, `res::assets::<file>`, `res::fonts::<family>`. Reference bundled resources
/// through these — `image(res::images::app_logo)` — so a typo is a compile error and the resource is
/// guaranteed present. Drop a file into `resource/images/` and its constant appears on the next build.
pub mod res {
    include!(concat!(env!("OUT_DIR"), "/day_resources.rs"));
}

day::routes! {
    /// The app's sections, typed (https://daybrite.dev/docs/navigation): each variant's key is
    /// what deep links, dayscript, and `current_route()` speak, and the `.item(Section::…)`
    /// declarations below are compile-checked against this enum.
    pub(crate) enum Section {
        Home => "home",
        Controls => "controls",
        Canvas => "canvas",
        Items => "items",
    }
}

pub fn root() -> AnyPiece {
    install_locales("en", &[("en", include_str!("../resource/locales/en/app.ftl"))]);
    // A sidebar selector bound to a Signal<Option<Section>> (`None` = nothing selected — the
    // collapsed list on mobile). Desktop shows sidebar + detail side by side; mobile shows a
    // list that pushes the detail. Deep links and dayscript address sections by key.
    let section = Signal::new(None::<Section>);
    selector(section)
        .style(SelectorStyle::Sidebar)
        .title(crate::res::str::app_title())
        .item(Section::Home, crate::res::str::nav_home(), home_page)
        .item(Section::Controls, crate::res::str::nav_controls(), controls_page)
        .item(Section::Canvas, crate::res::str::nav_canvas(), canvas_page)
        .item(Section::Items, crate::res::str::nav_items(), items_page)
        .id("nav")
        .any()
}

// Mobile / embedded entry points — each macro expands to nothing off its own platform.
day::ios_main!("{{title}}", root);
day::android_main!(root);
day::arkui_main!(root);
