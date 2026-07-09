use day::prelude::*;
use day_piece_rating::{Card, badge, rating};

/// The composition-first tier (DESIGN §8): every widget here is built PURELY from Day's core
/// primitives — NO native/per-backend code and NO cargo features. `day-piece-rating` is a plain
/// dependency with no per-backend feature wiring, so it works on every backend for free; native
/// pieces are the exception, not the rule.
pub(crate) fn compose_page() -> AnyPiece {
    // A shared rating signal, driven by tapping stars. Its count is mirrored into a text field:
    // `bind` pushes each newly-tapped value into `rating_text`, so tapping a star updates the field.
    let stars = Signal::new(3usize);
    let rating_text = Signal::new(stars.get().to_string());
    bind(
        move || stars.get(),
        move |n: &usize| rating_text.set(n.to_string()),
    );
    // A custom ambient value flowed via `with_environment` and read back by a descendant.
    #[derive(Clone, Copy)]
    struct Accent(Color);
    let accent = Color::hex(0x30_B0_60);

    column((
        label(tr("nav-compose"))
            .font(Font::Title)
            .id("compose-title"),
        label(tr("compose-caption")),
        // 1) Interactive star rating (canvas-polygon compose piece): tap a star, and the text field
        //    beside "Stars selected:" updates with the count (the `bind` above drives it).
        label(tr("compose-rating-label")).font(Font::Headline),
        rating(stars).id("compose-rating"),
        row((
            label(tr("compose-rating-count")),
            text_field(rating_text)
                .placeholder(tr("compose-rating-placeholder"))
                .id("compose-rating-value"),
        ))
        .spacing(8.0),
        // 2) Card modifier — a reusable surface wrapping arbitrary content.
        label(tr("compose-card-label")).font(Font::Headline),
        column((
            label(tr("compose-card-title")).font(Font::Headline),
            label(tr("compose-card-body")),
        ))
        .spacing(4.0)
        .align(HAlign::Leading)
        .modifier(Card),
        // 3) badge overlay — a numbered pill on an icon's top-trailing corner.
        label(tr("compose-badge-label")).font(Font::Headline),
        badge(
            3,
            rounded_rectangle(10.0)
                .fill(Color::hex(0x8E_8E_93))
                .frame(48.0, 48.0),
        ),
        // 4) ButtonStyle — a FilledButtonStyle button next to a plain one for contrast.
        label(tr("compose-buttons-label")).font(Font::Headline),
        row((
            button(tr("compose-plain-btn")).id("compose-plain-btn"),
            button(tr("compose-styled-btn"))
                .style(FilledButtonStyle {
                    color: Color::hex(0x0A_84_FF),
                })
                .id("compose-styled-btn"),
        ))
        .spacing(12.0),
        // 5) Ambient environment flow — a descendant tints itself from the provided Accent.
        label(tr("compose-env-label")).font(Font::Headline),
        with_environment(Accent(accent), || {
            let tint = environment::<Accent>().map(|a| a.0).unwrap_or(Color::BLACK);
            label(tr("compose-env-value"))
                .font(Font::Headline)
                .color(tint)
                .id("compose-env-value")
        }),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
