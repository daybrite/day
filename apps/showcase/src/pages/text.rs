use day::prelude::*;

use crate::widgets::heading;

/// Typography playground: every semantic text style (mapped to the platform's native styles + Dynamic
/// Type / font-scale accessibility sizing), font weights, bold/italic, color, and accessibility-scaled
/// custom sizes. See docs/text.md.
pub(crate) fn text_page() -> AnyPiece {
    // A style name (localized) rendered IN its own style — a self-documenting type specimen.
    // The dayscript id keeps the stable English style id regardless of locale.
    fn specimen(id: &'static str, name: LocalizedText, f: Font) -> AnyPiece {
        label(name).font(f).id_keyed("text-style", id).any()
    }
    // Every semantic style (largest → smallest), each rendered in its own style.
    let styles = column((
        label(tr("text-styles-header")).font(Font::Headline),
        specimen(
            "Large Title",
            tr("text-style-large-title"),
            Font::LargeTitle,
        ),
        specimen("Title", tr("text-style-title"), Font::Title),
        specimen("Title 2", tr("text-style-title2"), Font::Title2),
        specimen("Title 3", tr("text-style-title3"), Font::Title3),
        specimen("Headline", tr("text-style-headline"), Font::Headline),
        specimen(
            "Subheadline",
            tr("text-style-subheadline"),
            Font::Subheadline,
        ),
        specimen("Body", tr("text-style-body"), Font::Body),
        specimen("Callout", tr("text-style-callout"), Font::Callout),
        specimen("Footnote", tr("text-style-footnote"), Font::Footnote),
        specimen("Caption", tr("text-style-caption"), Font::Caption),
        specimen("Caption 2", tr("text-style-caption2"), Font::Caption2),
    ))
    .spacing(6.0)
    .align(HAlign::Leading);
    // Font weights on a body-size line.
    let weights = column((
        label(tr("text-weights-header")).font(Font::Headline),
        label(tr("text-weight-ultralight"))
            .weight(FontWeight::UltraLight)
            .id("text-w-ultralight"),
        label(tr("text-weight-light")).weight(FontWeight::Light),
        label(tr("text-weight-regular")).weight(FontWeight::Regular),
        label(tr("text-weight-medium")).weight(FontWeight::Medium),
        label(tr("text-weight-semibold")).weight(FontWeight::Semibold),
        label(tr("text-weight-bold"))
            .weight(FontWeight::Bold)
            .id("text-w-bold"),
        label(tr("text-weight-heavy")).weight(FontWeight::Heavy),
        label(tr("text-weight-black")).weight(FontWeight::Black),
    ))
    .spacing(4.0)
    .align(HAlign::Leading);
    // Bold / italic / both, and everything-at-once.
    let styling = column((
        label(tr("text-styling-header")).font(Font::Headline),
        label(tr("text-bold")).bold().id("text-bold"),
        label(tr("text-italic")).italic().id("text-italic"),
        label(tr("text-bolditalic"))
            .bold()
            .italic()
            .id("text-bolditalic"),
        label(tr("text-emphasis-label"))
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
            label(tr("color-red")).color(Color::hex(0xE74C3C)),
            label(tr("color-green")).color(Color::hex(0x27AE60)),
            label(tr("color-blue")).color(Color::hex(0x2F6FDE)),
            label(tr("color-orange")).color(Color::hex(0xE67E22)),
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
    // Bundled custom fonts (docs/resources.md): the three families ship in the app's fonts/
    // directory; `Font::Custom` references them by FAMILY name (what the font file reports),
    // and `day build` + the backend make that name resolve on every platform.
    let fonts = column((
        label(tr("text-fonts-header")).font(Font::Headline),
        label(tr("text-fonts-note")).font(Font::Footnote),
        label("Pacifico — flowing script")
            .font(Font::custom(crate::res::fonts::pacifico, 24.0))
            .id("text-font-pacifico"),
        label("BUNGEE — chromatic display")
            .font(Font::custom(crate::res::fonts::bungee, 20.0))
            .id("text-font-bungee"),
        label("Special Elite — typewriter keys")
            .font(Font::custom(crate::res::fonts::special_elite, 20.0))
            .id("text-font-specialelite"),
        label("Pacifico at 36 points")
            .font(Font::custom(crate::res::fonts::pacifico, 36.0))
            .color(Color::hex(0x2F6FDE))
            .id("text-font-pacifico-lg"),
    ))
    .spacing(6.0)
    .align(HAlign::Leading);

    scroll(
        column((
            heading(tr("nav-text"), "text-title", Some(tr("text-caption"))),
            // Bundled fonts lead the page: the most visually distinctive section, and the one
            // the walkthrough screenshot must show above the fold.
            fonts,
            divider(),
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
