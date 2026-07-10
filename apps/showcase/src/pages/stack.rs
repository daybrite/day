use crate::Section;
use day::prelude::*;

/// The stack's typed destinations (docs/navigation.md): a data-carrying [`Route`].
/// `Depth(n)` ↔ `"n"` and `Item { id }` ↔ `"item-<id>"`, so the wire route
/// `stack/item-42?hint=linked` parses back into `Item { id: 42 }` — the destination builder
/// matches on the typed value instead of string-splitting keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Drill {
    Depth(u32),
    Item { id: u32 },
}

impl Route for Drill {
    fn key(&self) -> String {
        match self {
            Drill::Depth(n) => n.to_string(),
            Drill::Item { id } => format!("item-{id}"),
        }
    }
    fn from_key(key: &str) -> Option<Self> {
        if let Some(id) = key.strip_prefix("item-") {
            return id.parse().ok().map(|id| Drill::Item { id });
        }
        key.parse().ok().map(Drill::Depth)
    }
}

/// Genuine push/pop navigation (docs/navigation.md): `stack` bound to a `Signal<Vec<Drill>>`
/// path. Pushing a detail appends a typed value to the path; Day reconciles the native
/// UINavigationController / AdwNavigationView / back-stack; the native back button writes the
/// pop back into the path.
pub(crate) fn stack_page() -> AnyPiece {
    fn push(path: Signal<Vec<Drill>>) {
        let mut v = path.get_untracked();
        let n = v.len() as u32 + 1;
        v.push(Drill::Depth(n));
        path.set(v);
    }
    let path = Signal::new(Vec::<Drill>::new());
    let root = column((
        label(tr("stack-root-body")).id("stack-root"),
        button(tr("stack-push"))
            .action(move || push(path))
            .id("stack-push"),
        // An ABSOLUTE route with query params (docs/navigation.md), built typed: it anchors
        // the enclosing selector at Section::Stack, resets this stack, pushes Item { id: 42 };
        // the destination builder reads the ?hint= param via route_param().
        nav_link_to(
            tr("stack-link-42"),
            route(&Section::Stack)
                .then(&Drill::Item { id: 42 })
                .param("hint", "linked"),
        )
        .id("stack-link"),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0);
    stack(path, root)
        .destination(move |d: &Drill| {
            // The typed value arrives parsed — match, don't split strings.
            let title = match d {
                Drill::Depth(n) => tr("stack-detail-title").arg("depth", n.to_string()),
                Drill::Item { id } => tr("stack-item-title").arg("id", id.to_string()),
            };
            // Params travel with navigate() — a push performed by writing the path signal
            // carries its data in the route value itself (docs/navigation.md).
            let hint: AnyPiece = match route_param("hint") {
                Some(h) => label(tr("stack-param-hint").arg("hint", h))
                    .font(Font::Footnote)
                    .id("stack-param")
                    .any(),
                None => label("").any(),
            };
            column((
                label(title).font(Font::Title).id("stack-detail"),
                label(tr("stack-detail-body")),
                hint,
                button(tr("stack-push"))
                    .action(move || push(path))
                    .id("stack-deeper"),
            ))
            .spacing(12.0)
            .align(HAlign::Leading)
            .padding(16.0)
        })
        .id("demo-stack")
}
