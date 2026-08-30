//! The Navigate section's editor: everything here edits the one item `navigate.rs` chose.
//! The two files share exactly one signal, `selected()`.

use super::navigate::selected;
use crate::model::{self, ItemFields, KINDS};
use crate::res;
use day::prelude::*;

/// The selected row's editor, or the empty state. `each` is keyed on the id, so every item
/// gets its own scope and fresh field bindings.
pub(crate) fn editor_pane() -> impl Piece {
    column((
        when(
            move || selected().get().is_none_or(|id| model::find(id).is_none()),
            || {
                column((label(res::str::item_none()).font(Font::Title3),))
                    .align(HAlign::Center)
                    .padding(24.0)
                    .grow()
            },
        ),
        each(
            items(
                move || {
                    selected()
                        .get()
                        .filter(|id| model::find(*id).is_some())
                        .into_iter()
                        .collect::<Vec<u32>>()
                },
                |id: &u32| *id,
            ),
            |slot: ItemSlot<u32, u32>| editor(slot.key()),
        ),
    ))
    .grow()
}

/// Two-way field bindings over one item: each control reads and writes its store field
/// directly, so every edit persists as it is made
/// (https://daybrite.dev/docs/forms, https://daybrite.dev/docs/model).
pub(crate) fn editor(id: u32) -> impl Piece {
    let it = model::items().elem(id as u64);

    form((
        section((
            labeled(
                res::str::field_name(),
                text_field(it.name())
                    .placeholder(res::str::field_name_hint())
                    .id("field-name"),
            ),
            labeled(res::str::field_count(), stepper(it.count())),
            // The store keeps the ISO string; `.map` converts to the picker's `DayDate` both ways.
            labeled(
                res::str::field_date(),
                day_piece_datetime::date_picker(it.date().map(date_of, iso_of)).id("field-date"),
            ),
        ))
        .title(res::str::section_basics()),
        section((
            labeled(
                res::str::field_kind(),
                picker(KINDS.iter().map(|k| tr(k).format()), it.kind())
                    .segmented()
                    .id("field-kind"),
            ),
            labeled(res::str::field_done(), toggle(it.done()).id("field-done")),
            labeled(
                res::str::field_rating(),
                day_piece_rating::rating(it.rating())
                    .max(5)
                    .id("field-rating"),
            ),
            labeled(
                res::str::field_color(),
                day_piece_colorpicker::color_picker(it.color().map(color_of, hex_of))
                    .id("field-color"),
            ),
        ))
        .title(res::str::section_details()),
        section((text_area(it.notes()).min_lines(5).id("field-notes"),))
            .title(res::str::section_notes()),
    ))
    // A phone's pane is the whole window; keep the rows off its edges.
    .padding(Insets {
        top: 0.0,
        leading: 16.0,
        bottom: 0.0,
        trailing: 16.0,
    })
}

/// A composed stepper; no toolkit in Day's set ships one as a single widget.
fn stepper(value: impl Binding<i64> + Copy) -> impl Piece {
    row((
        button("−")
            .action(move || value.write((value.peek() - 1).max(0)))
            .id("field-count-dec"),
        label(move || value.read().to_string())
            .tabular()
            .reserving("000")
            .id("field-count"),
        button("+")
            .action(move || value.write((value.peek() + 1).min(999)))
            .id("field-count-inc"),
    ))
    .spacing(8.0)
}

// --- the field conversions `.map` binds through: plain fns, so the binding stays `Copy` --------

/// ISO string → `DayDate`; malformed user input falls back to today.
//
// `&String` exactly, not `&str`: `.map` takes fn pointers over `V = String`, and deref coercion
// does not apply to fn-pointer types.
#[allow(clippy::ptr_arg)]
fn date_of(s: &String) -> day_piece_datetime::DayDate {
    day_piece_datetime::DayDate::parse_iso(s).unwrap_or_else(|| {
        // `debug!`: a half-typed date is normal while the user is still typing.
        debug!("date {s:?} is not ISO-8601; showing today");
        day_piece_datetime::DayDate::today()
    })
}

fn iso_of(d: &day_piece_datetime::DayDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// `#RRGGBB` → color; malformed falls back. Also tints the list rows' kind glyphs.
// `&String` for the same fn-pointer reason as `date_of`.
#[allow(clippy::ptr_arg)]
pub(crate) fn color_of(s: &String) -> Color {
    let h = s.trim_start_matches('#');
    u32::from_str_radix(h, 16)
        .ok()
        .filter(|_| h.len() == 6)
        .map(Color::hex)
        .unwrap_or(Color::hex(0x3B82F6))
}

fn hex_of(c: &Color) -> String {
    let (r, g, b) = (
        (c.r * 255.0).round() as u8,
        (c.g * 255.0).round() as u8,
        (c.b * 255.0).round() as u8,
    );
    format!("#{r:02X}{g:02X}{b:02X}")
}
