//! Reusable pieces shared by more than one page (see the `pages` modules).

use day::prelude::*;

/// The current battery reading as a localized line (Fluent; the state name stays the API's
/// enum debug form — it is a value, not prose). Shared by the Battery and About pages.
pub(crate) fn battery_line() -> LocalizedText {
    match day_part_battery::status() {
        Some(b) => crate::res::str::battery_reading(
            b.percent()
                .map(|p| format!("{p}%"))
                .unwrap_or_else(|| "?".into()),
            format!("{:?}", b.state),
        ),
        None => crate::res::str::battery_reading_none(),
    }
}

pub(crate) fn gauge(value: Signal<f64>) -> AnyPiece {
    canvas(move |d, size| {
        if size.width <= 0.0 {
            return;
        }
        let r = Rect::from_size(size).inset(8.0);
        let track = Color::rgba(0.5, 0.5, 0.55, 0.35);
        let accent = Color::hex(0x2F6FDE);
        d.stroke(
            Shape::Arc {
                rect: r,
                start_deg: 135.0,
                sweep_deg: 270.0,
            },
            track,
            6.0,
        );
        let frac = (value.get() / 100.0).clamp(0.0, 1.0);
        if frac > 0.0 {
            d.stroke(
                Shape::Arc {
                    rect: r,
                    start_deg: 135.0,
                    sweep_deg: 270.0 * frac,
                },
                accent,
                6.0,
            );
        }
        d.text(
            &format!("{:.0}", value.get()),
            Point::new(size.width / 2.0, size.height / 2.0),
            TextStyle {
                size: 22.0,
                color: accent,
                anchor: TextAnchor::Centered,
            },
        );
    })
    // Accessibility (§13): a canvas has no inherent role, so Day applies `Meter` + a spoken value
    // and label. `.id`/`.a11y` go on the canvas leaf (before `.frame`, a handle-less layout node),
    // so they reach the native widget. Value is a build-time snapshot (reactive a11y is a follow-up).
    .a11y(move |a| {
        a.role(Role::Meter)
            .label(crate::res::str::volume_label().format())
            .value(format!("{:.0}", value.get_untracked()))
    })
    .id("gauge")
    .frame(110.0, 110.0)
}

pub(crate) fn history(count: Signal<i64>) -> AnyPiece {
    let entries = Signal::new(Vec::<(u64, i64)>::new());
    let next_id = Signal::new(0u64);
    watch(
        move || count.get(),
        move |new, _old| {
            let id = next_id.get_untracked();
            next_id.set(id + 1);
            let v = *new;
            entries.update(|e| {
                e.push((id, v));
                if e.len() > 8 {
                    e.remove(0);
                }
            });
        },
    );
    column((
        label(crate::res::str::history_title()).font(Font::Headline),
        each(
            move || entries.get(),
            |e| e.0,
            move |slot: ItemSlot<(u64, i64), u64>| {
                label(move || crate::res::str::history_entry(slot.field(|t| t.1)).format())
            },
        ),
    ))
    .spacing(4.0)
    .align(HAlign::Leading)
    .any()
}

/// Standard page scaffold (the showcase design pass): a title + optional caption header over a
/// scrollable, consistently padded content column. Every page uses it, so typography, spacing,
/// and scrolling behave identically across the app.
/// A page's title heading. When the native nav shows the destination title in its own header
/// (`Cap::NavHeader` — e.g. the Windows NavigationView), the big in-content title is redundant, so
/// it is dropped: the caption (or, lacking one, a de-emphasized title) carries the `title_id` so
/// scripts/tests still find the anchor. Elsewhere it renders the usual `Font::Title` + caption.
pub(crate) fn heading(
    title: LocalizedText,
    title_id: &'static str,
    caption: Option<LocalizedText>,
) -> AnyPiece {
    let native_header = capability(Cap::NavHeader) == Support::Native;
    match (native_header, caption) {
        (true, Some(c)) => label(c).font(Font::Subheadline).id(title_id).any(),
        (true, None) => label(title).font(Font::Subheadline).id(title_id).any(),
        (false, Some(c)) => column((
            label(title).font(Font::Title).id(title_id),
            label(c).font(Font::Footnote),
        ))
        .spacing(4.0)
        .align(HAlign::Leading)
        .any(),
        (false, None) => label(title).font(Font::Title).id(title_id).any(),
    }
}

pub(crate) fn page(
    title: LocalizedText,
    title_id: &'static str,
    caption: Option<LocalizedText>,
    body: AnyPiece,
) -> AnyPiece {
    scroll(
        column((heading(title, title_id, caption), body))
            .spacing(16.0)
            .align(HAlign::Leading)
            .padding(20.0),
    )
    .any()
}
