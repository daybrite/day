use crate::Section;
use crate::palette::{CORAL, RAMP, SKY, VIOLET};
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
        stack_glyph(),
        label(crate::res::str::stack_root_body()).id("stack-root"),
        button(crate::res::str::stack_push())
            .prominent()
            .action(move || push(path))
            .id("stack-push"),
        // An ABSOLUTE route with query params (docs/navigation.md), built typed: it anchors
        // the enclosing selector at Section::Stack, resets this stack, pushes Item { id: 42 };
        // the destination builder reads the ?hint= param via route_param().
        nav_link_to(
            crate::res::str::stack_link_42(),
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
            let (title, depth) = match d {
                Drill::Depth(n) => (crate::res::str::stack_detail_title(n.to_string()), *n),
                Drill::Item { id } => (crate::res::str::stack_item_title(id.to_string()), 1),
            };
            // Params travel with navigate() — a push performed by writing the path signal
            // carries its data in the route value itself (docs/navigation.md). The hint row
            // exists only when the param does, so paramless pushes get no phantom gap.
            let mut parts: Vec<AnyPiece> = vec![
                depth_dots(depth),
                label(title).font(Font::Title).id("stack-detail").any(),
                label(crate::res::str::stack_detail_body()).any(),
            ];
            if let Some(h) = route_param("hint") {
                parts.push(
                    label(crate::res::str::stack_param_hint(h))
                        .font(Font::Footnote)
                        .id("stack-param")
                        .any(),
                );
            }
            parts.push(
                button(crate::res::str::stack_push())
                    .prominent()
                    .action(move || push(path))
                    .id("stack-deeper")
                    .any(),
            );
            column(PieceVec(parts))
                .spacing(12.0)
                .align(HAlign::Leading)
                .padding(16.0)
        })
        .id("demo-stack")
}

/// The page's motif: three offset cards, drawn as one canvas leaf — a stack you can see.
fn stack_glyph() -> AnyPiece {
    shape_group([
        rounded_rectangle(8.0).fill(SKY).at(0.16, 0.0, 0.68, 0.42),
        rounded_rectangle(8.0)
            .fill(VIOLET)
            .at(0.08, 0.26, 0.84, 0.42),
        rounded_rectangle(8.0).fill(CORAL).at(0.0, 0.54, 1.0, 0.46),
    ])
    .frame(84.0, 60.0)
}

/// The path so far, one numbered dot per level in the sunrise ramp's order — each push adds
/// a dot, each pop removes one, mirroring the `Vec<Drill>` behind the native stack.
fn depth_dots(depth: u32) -> AnyPiece {
    let dots: Vec<AnyPiece> = (1..=depth)
        .map(|n| {
            zstack((
                circle()
                    .fill(RAMP[((n - 1) % RAMP.len() as u32) as usize])
                    .frame(24.0, 24.0),
                label(n.to_string())
                    .font(Font::Caption)
                    .color(Color::WHITE)
                    .bold(),
            ))
            .any()
        })
        .collect();
    row(PieceVec(dots)).spacing(6.0).any()
}
