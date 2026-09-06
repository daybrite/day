//! The Navigate section's detail side: one item's editor, and the pane that hosts it.
//! `navigate.rs` chooses the item; this file edits it. The two meet at `scene.selected`.

use crate::model::{ItemFields, KINDS, Scene};
use crate::res;
use day::prelude::*;

/// The desktop detail pane: the selected row's editor, or the empty state.
///
/// `each` over a zero-or-one list rather than `when`, so the editor is rebuilt when the
/// selection changes and each item gets its own scope and bindings.
pub(crate) fn editor_pane(scene: Scene) -> impl Piece {
    column((
        when(
            move || {
                scene
                    .selected
                    .get()
                    .is_none_or(|id| scene.find(id).is_none())
            },
            || {
                // The empty state, centered. Spacers because a column stacks from the top.
                column((
                    spacer(),
                    label(res::str::item_none())
                        .font(Font::Title3)
                        .secondary()
                        // The label fills the width, so center the text.
                        .align(TextAlign::Center),
                    spacer(),
                ))
                .align(HAlign::Center)
                // Grown inside the padding to fill the pane, and outside so the parent sizes it.
                .grow()
                .padding(24.0)
                .grow()
            },
        ),
        each(
            items(
                move || {
                    scene
                        .selected
                        .get()
                        .filter(|id| scene.find(*id).is_some())
                        .into_iter()
                        .collect::<Vec<u32>>()
                },
                |id: &u32| *id,
            ),
            move |slot: ItemSlot<u32, u32>| editor(scene, slot.key()),
        ),
    ))
    .grow()
}

/// The editor: a native form of two-way bindings over one item. `it.name()` is both the read
/// and the write, so the form writes the store as the user types.
pub(crate) fn editor(scene: Scene, id: u32) -> impl Piece {
    let it = scene.items.elem(id as u64);

    form((
        section((
            labeled(
                res::str::field_name(),
                text_field(it.name())
                    .placeholder(res::str::field_name_hint())
                    .id("field-name"),
            ),
            labeled(res::str::field_count(), stepper(it.count())),
            // The store keeps the ISO string and the picker speaks `DayDate`; `.map` converts
            // both ways and the result is still a two-way binding.
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
    // On a phone the form is the whole window, so give the rows some room at the edges.
    .padding(Insets {
        top: 0.0,
        leading: 16.0,
        bottom: 0.0,
        trailing: 16.0,
    })
}

/// A number field with its own +/− pair. No toolkit ships a stepper as one widget, so this is
/// three pieces over one binding, which is also how any missing control gets built.
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

// --- conversions for `.map`: plain fns, so the binding stays `Copy` --------------------------

/// Stored ISO string to the date piece's type. Malformed values fall back to today.
//
// `&String`, not `&str`: `Mapped::map` takes function pointers over the field's own type, and
// deref coercion does not apply to fn-pointer types, so clippy's ptr_arg is wrong here.
#[allow(clippy::ptr_arg)]
fn date_of(s: &String) -> day_piece_datetime::DayDate {
    day_piece_datetime::DayDate::parse_iso(s).unwrap_or_else(|| {
        // `debug!` rather than `warn!`: a half-typed date is normal, so this is for whoever is
        // debugging the field (`DAY_LOG=debug`).
        debug!("date {s:?} is not ISO-8601 — showing today");
        day_piece_datetime::DayDate::today()
    })
}

fn iso_of(d: &day_piece_datetime::DayDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

/// `#RRGGBB` to a color, falling back rather than failing. The list rows tint their glyph with
/// it too.
// `&String` for the same reason as `date_of`.
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
