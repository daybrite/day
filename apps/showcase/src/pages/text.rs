use day::prelude::*;

/// Typography playground: every semantic text style (mapped to the platform's native styles + Dynamic
/// Type / font-scale accessibility sizing), font weights, bold/italic, color, and accessibility-scaled
/// custom sizes. See docs/text.md.
pub(crate) fn text_page() -> AnyPiece {
    // A style name rendered IN its own style — a self-documenting type specimen.
    fn specimen(name: &'static str, f: Font) -> AnyPiece {
        label(name).font(f).id_keyed("text-style", name).any()
    }
    // Every semantic style (largest → smallest), each rendered in its own style.
    let styles = column((
        label(tr("text-styles-header")).font(Font::Headline),
        specimen("Large Title", Font::LargeTitle),
        specimen("Title", Font::Title),
        specimen("Title 2", Font::Title2),
        specimen("Title 3", Font::Title3),
        specimen("Headline", Font::Headline),
        specimen("Subheadline", Font::Subheadline),
        specimen("Body", Font::Body),
        specimen("Callout", Font::Callout),
        specimen("Footnote", Font::Footnote),
        specimen("Caption", Font::Caption),
        specimen("Caption 2", Font::Caption2),
    ))
    .spacing(6.0)
    .align(HAlign::Leading);
    // Font weights on a body-size line.
    let weights = column((
        label(tr("text-weights-header")).font(Font::Headline),
        label("Ultra Light")
            .weight(FontWeight::UltraLight)
            .id("text-w-ultralight"),
        label("Light").weight(FontWeight::Light),
        label("Regular").weight(FontWeight::Regular),
        label("Medium").weight(FontWeight::Medium),
        label("Semibold").weight(FontWeight::Semibold),
        label("Bold").weight(FontWeight::Bold).id("text-w-bold"),
        label("Heavy").weight(FontWeight::Heavy),
        label("Black").weight(FontWeight::Black),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);
    // Bold / italic / both, and everything-at-once.
    let styling = column((
        label(tr("text-styling-header")).font(Font::Headline),
        label("Bold text").bold().id("text-bold"),
        label("Italic text").italic().id("text-italic"),
        label("Bold italic").bold().italic().id("text-bolditalic"),
        label("Emphasis")
            .font(Font::Title2)
            .weight(FontWeight::Heavy)
            .italic()
            .color(Color::hex(0x8E44AD))
            .id("text-emphasis"),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);
    // Color.
    let colors = column((
        label(tr("text-colors-header")).font(Font::Headline),
        row((
            label("Red").color(Color::hex(0xE74C3C)),
            label("Green").color(Color::hex(0x27AE60)),
            label("Blue").color(Color::hex(0x2F6FDE)),
            label("Orange").color(Color::hex(0xE67E22)),
        ))
        .spacing(12.0),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);
    // Custom sizes — Font::System(pt), still scaled by the platform accessibility text size.
    let custom = column((
        label(tr("text-custom-header")).font(Font::Headline),
        label(tr("text-custom-note")).font(Font::Footnote),
        label("13 pt").font(Font::System(13.0)),
        label("20 pt").font(Font::System(20.0)),
        label("28 pt").font(Font::System(28.0)).id("text-custom-28"),
        label("40 pt")
            .font(Font::System(40.0))
            .weight(FontWeight::Bold),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);

    scroll(
        column((
            label(tr("nav-text"))
                .font(Font::LargeTitle)
                .id("text-title"),
            label(tr("text-caption")).font(Font::Subheadline),
            styles,
            divider(),
            weights,
            divider(),
            styling,
            divider(),
            colors,
            divider(),
            custom,
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(16.0),
    )
    .any()
}
