use crate::Section;
use day::prelude::*;

/// A typed stack route that carries data (https://daybrite.dev/docs/navigation): `Item { id }`
/// encodes as the path segment `item-<id>` and parses back — the destination builder receives
/// the parsed value, and a deep link like `items/item-2` validates on the way in.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct Item {
    pub id: u32,
}

impl Route for Item {
    fn key(&self) -> String {
        format!("item-{}", self.id)
    }
    fn from_key(key: &str) -> Option<Self> {
        key.strip_prefix("item-")?
            .parse()
            .ok()
            .map(|id| Item { id })
    }
}

/// Drill-down navigation: a `stack` bound to a `Signal<Vec<Item>>` path. Pushing appends a
/// typed value; Day reconciles the native stack (UINavigationController, the Android fragment
/// back stack, AdwNavigationView, desktop back headers); the native back button and gestures
/// write the pop back into the path.
pub(crate) fn items_page() -> AnyPiece {
    fn open(path: Signal<Vec<Item>>, id: u32) {
        let mut v = path.get_untracked();
        v.push(Item { id });
        path.set(v);
    }
    let path = Signal::new(Vec::<Item>::new());
    let root = column((
        label(crate::res::str::items_title()).font(Font::Title).id("items-title"),
        label(crate::res::str::items_blurb()),
        button(crate::res::str::item_open("1"))
            .action(move || open(path, 1))
            .id("item-1"),
        button(crate::res::str::item_open("2"))
            .action(move || open(path, 2))
            .id("item-2"),
        // The same destination, addressed as an ABSOLUTE typed route with a query param —
        // one string reaches it from anywhere (a cold-start deep link included).
        nav_link_to(
            crate::res::str::item_link(),
            route(&Section::Items)
                .then(&Item { id: 3 })
                .param("via", "link"),
        )
        .id("item-link"),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0);
    stack(path, root)
        .destination(move |item: &Item| {
            let via: AnyPiece = match route_param("via") {
                Some(v) => label(crate::res::str::item_via(v))
                    .font(Font::Footnote)
                    .any(),
                None => label("").any(),
            };
            column((
                label(crate::res::str::item_title(item.id.to_string()))
                    .font(Font::Title)
                    .id("item-detail"),
                label(crate::res::str::item_body()),
                via,
                button(crate::res::str::item_home())
                    .action(|| {
                        let _ = navigate_to(&Section::Home);
                    })
                    .id("item-go-home"),
            ))
            .spacing(12.0)
            .align(HAlign::Leading)
            .padding(16.0)
        })
        .id("items-stack")
}
