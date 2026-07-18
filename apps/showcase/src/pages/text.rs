use day::prelude::*;

use crate::widgets::page;

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
    let styles = section((
        specimen(
            "Large Title",
            crate::res::str::text_style_large_title(),
            Font::LargeTitle,
        ),
        specimen("Title", crate::res::str::text_style_title(), Font::Title),
        specimen(
            "Title 2",
            crate::res::str::text_style_title2(),
            Font::Title2,
        ),
        specimen(
            "Title 3",
            crate::res::str::text_style_title3(),
            Font::Title3,
        ),
        specimen(
            "Headline",
            crate::res::str::text_style_headline(),
            Font::Headline,
        ),
        specimen(
            "Subheadline",
            crate::res::str::text_style_subheadline(),
            Font::Subheadline,
        ),
        specimen("Body", crate::res::str::text_style_body(), Font::Body),
        specimen(
            "Callout",
            crate::res::str::text_style_callout(),
            Font::Callout,
        ),
        specimen(
            "Footnote",
            crate::res::str::text_style_footnote(),
            Font::Footnote,
        ),
        specimen(
            "Caption",
            crate::res::str::text_style_caption(),
            Font::Caption,
        ),
        specimen(
            "Caption 2",
            crate::res::str::text_style_caption2(),
            Font::Caption2,
        ),
    ))
    .title(crate::res::str::text_styles_header());
    // Font weights on a body-size line.
    let weights = section((
        label(crate::res::str::text_weight_ultralight())
            .weight(FontWeight::UltraLight)
            .id("text-w-ultralight"),
        label(crate::res::str::text_weight_light()).weight(FontWeight::Light),
        label(crate::res::str::text_weight_regular()).weight(FontWeight::Regular),
        label(crate::res::str::text_weight_medium()).weight(FontWeight::Medium),
        label(crate::res::str::text_weight_semibold()).weight(FontWeight::Semibold),
        label(crate::res::str::text_weight_bold())
            .weight(FontWeight::Bold)
            .id("text-w-bold"),
        label(crate::res::str::text_weight_heavy()).weight(FontWeight::Heavy),
        label(crate::res::str::text_weight_black()).weight(FontWeight::Black),
    ))
    .title(crate::res::str::text_weights_header());
    // Bold / italic / both, and everything-at-once.
    let styling = section((
        label(crate::res::str::text_bold()).bold().id("text-bold"),
        label(crate::res::str::text_italic())
            .italic()
            .id("text-italic"),
        label(crate::res::str::text_bolditalic())
            .bold()
            .italic()
            .id("text-bolditalic"),
        label(crate::res::str::text_emphasis_label())
            .font(Font::Title2)
            .weight(FontWeight::Heavy)
            .italic()
            .color(Color::hex(0x8E44AD))
            .id("text-emphasis"),
    ))
    .title(crate::res::str::text_styling_header());
    // Color.
    let colors = section((row((
        label(crate::res::str::color_red()).color(Color::hex(0xE74C3C)),
        label(crate::res::str::color_green()).color(Color::hex(0x27AE60)),
        label(crate::res::str::color_blue()).color(Color::hex(0x2F6FDE)),
        label(crate::res::str::color_orange()).color(Color::hex(0xE67E22)),
    ))
    .spacing(12.0),))
    .title(crate::res::str::text_colors_header());
    // Custom sizes — Font::System(pt), still scaled by the platform accessibility text size.
    let custom = section((
        label(crate::res::str::text_custom_note()).font(Font::Footnote),
        label("13 pt").font(Font::System(13.0)),
        label("20 pt").font(Font::System(20.0)),
        label("28 pt").font(Font::System(28.0)).id("text-custom-28"),
        label("40 pt")
            .font(Font::System(40.0))
            .weight(FontWeight::Bold),
    ))
    .title(crate::res::str::text_custom_header());
    // Bundled custom fonts (docs/resources.md): the three families ship in the app's fonts/
    // directory; `Font::Custom` references them by FAMILY name (what the font file reports),
    // and `day build` + the backend make that name resolve on every platform.
    let fonts = section((
        label(crate::res::str::text_fonts_note()).font(Font::Footnote),
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
    .title(crate::res::str::text_fonts_header());

    // Links (docs/text.md): tappable accent-coloured text that opens a URL in the system browser
    // (or the mail client for `mailto:`) via the backend's `open_url`. `.color()` overrides the
    // default tint; `.font()` and `.bold()` style the run like a label.
    let links = section((
        label("Tap a link to open it in the system browser.").font(Font::Footnote),
        link("daybrite.dev", "https://daybrite.dev").id("text-link-web"),
        link(
            "Material Symbols on Google Fonts",
            "https://fonts.google.com/icons",
        )
        .font(Font::Footnote)
        .id("text-link-icons"),
        link("Email the team", "mailto:hello@daybrite.dev")
            .color(Color::hex(0x27AE60))
            .id("text-link-mail"),
    ))
    .title("Links");

    // icu4x-backed formatting + collation (docs/localization.md "Formatted values"/"Sorting"):
    // the SAME `NUMBER()`/`DATETIME()` calls in every locale file render locale-correctly
    // (grouping, digit systems, CLDR date patterns), and the fruit list re-sorts with real
    // collation (zh = pinyin) — all reactive to the run's locale.
    let formatting = section((
        label(crate::res::str::fmt_caption()).font(Font::Footnote),
        labeled(
            crate::res::str::fmt_number_label(),
            label(crate::res::str::fmt_number(1234567.891)).id("fmt-number"),
        ),
        labeled(
            crate::res::str::fmt_percent_label(),
            label(crate::res::str::fmt_percent(0.72)).id("fmt-percent"),
        ),
        labeled(
            crate::res::str::fmt_date_label(),
            label(crate::res::str::fmt_date("2026-07-18")).id("fmt-date"),
        ),
        labeled(
            crate::res::str::fmt_time_label(),
            label(crate::res::str::fmt_time("14:45")).id("fmt-time"),
        ),
        labeled(
            crate::res::str::fmt_sorted_label(),
            label(move || {
                let mut fruits = vec![
                    crate::res::str::fruit_banana().format(),
                    crate::res::str::fruit_cherry().format(),
                    crate::res::str::fruit_apple().format(),
                    crate::res::str::fruit_elderberry().format(),
                    crate::res::str::fruit_date().format(),
                ];
                sort_localized(&mut fruits);
                fruits.join(" · ")
            })
            .id("fmt-sorted"),
        ),
    ))
    .title(crate::res::str::fmt_title());

    // Bundled fonts lead the page: the most visually distinctive section, and the one the
    // walkthrough screenshot must show above the fold.
    page(
        crate::res::str::nav_text(),
        "text-title",
        Some(crate::res::str::text_caption()),
        form((
            fonts, styles, weights, styling, colors, custom, formatting, links,
        ))
        .any(),
    )
}
