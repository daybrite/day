use day::prelude::*;

use crate::widgets::page;

/// Keyboard focus as app state (docs/focus.md): one optional enum signal steering a whole form,
/// a plain Bool binding on a single field, and focus on non-text controls where the platform
/// allows it — each permutation in its own section with a live readout.
pub(crate) fn focus_page() -> AnyPiece {
    page(
        crate::res::str::nav_focus(),
        "focus-title",
        Some(crate::res::str::focus_caption()),
        form((group_section(), bool_section(), probe_section())).any(),
    )
}

/// Which field of the group form owns focus (`None` = nobody).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Field {
    Name,
    Email,
    City,
}

impl Field {
    /// Return-key chaining order: Name → Email → City → nobody.
    fn after(self) -> Option<Field> {
        match self {
            Field::Name => Some(Field::Email),
            Field::Email => Some(Field::City),
            Field::City => None,
        }
    }

    /// The field's localized label, for the live readout ("—" for nobody).
    fn display(this: Option<Field>) -> String {
        match this {
            None => "—".into(),
            Some(Field::Name) => crate::res::str::focus_name_label().format(),
            Some(Field::Email) => crate::res::str::focus_email_label().format(),
            Some(Field::City) => crate::res::str::focus_city_label().format(),
        }
    }
}

/// One `Signal<Option<Field>>` steering three fields: clicking or tabbing writes it, the
/// buttons write it back, and Return chains to the next field via `on_submit`. The "next"
/// button cycles from the last field the signal named — not from the live value, which a
/// click-to-focus toolkit clears the moment the button itself is clicked.
fn group_section() -> impl Piece {
    let focus = Signal::new(None::<Field>);
    let name = Signal::new(String::new());
    let email = Signal::new(String::new());
    let city = Signal::new(String::new());
    let last = Signal::new(None::<Field>);
    watch(
        move || focus.get(),
        move |f: &Option<Field>, _old| {
            if f.is_some() {
                last.set(*f);
            }
        },
    );
    section((
        labeled(
            crate::res::str::focus_name_label(),
            text_field(name)
                .on_submit(move || focus.set(Some(Field::Email)))
                .id("focus-name-field")
                .focused((focus, Field::Name)),
        ),
        labeled(
            crate::res::str::focus_email_label(),
            text_field(email)
                .on_submit(move || focus.set(Some(Field::City)))
                .id("focus-email-field")
                .focused((focus, Field::Email)),
        ),
        labeled(
            crate::res::str::focus_city_label(),
            text_field(city)
                .on_submit(move || focus.set(None))
                .id("focus-city-field")
                .focused((focus, Field::City)),
        ),
        labeled(
            crate::res::str::focus_current_label(),
            label(move || Field::display(focus.get())).id("focus-current"),
        ),
        row((
            button(crate::res::str::focus_next())
                .action(move || {
                    let next = last
                        .get_untracked()
                        .and_then(Field::after)
                        .unwrap_or(Field::Name);
                    focus.set(Some(next));
                })
                .id("focus-next-button"),
            button(crate::res::str::focus_clear())
                .action(move || {
                    focus.set(None);
                    last.set(None);
                })
                .id("focus-clear-button"),
        ))
        .spacing(12.0),
        label(crate::res::str::focus_group_caption()).font(Font::Footnote),
    ))
    .title(crate::res::str::focus_group_section())
}

/// The plain shape: one field, one `Signal<bool>`. The buttons drive it; the readout and the
/// native focus ring always agree because the signal is written back from the toolkit.
fn bool_section() -> impl Piece {
    let editing = Signal::new(false);
    let text = Signal::new(String::new());
    section((
        text_field(text)
            .placeholder(crate::res::str::focus_bool_placeholder())
            .id("focus-bool-field")
            .focused(editing),
        labeled(
            crate::res::str::focus_state_label(),
            label(move || {
                if editing.get() {
                    crate::res::str::focus_state_on().format()
                } else {
                    crate::res::str::focus_state_off().format()
                }
            })
            .id("focus-bool-state"),
        ),
        row((
            button(crate::res::str::focus_focus_btn())
                .action(move || editing.set(true))
                .id("focus-bool-focus"),
            button(crate::res::str::focus_blur_btn())
                .action(move || editing.set(false))
                .id("focus-bool-blur"),
        ))
        .spacing(12.0),
        label(crate::res::str::focus_bool_caption()).font(Font::Footnote),
    ))
    .title(crate::res::str::focus_bool_section())
}

/// Focus on non-text controls. Desktop toolkits honor these bindings (with each platform's own
/// keyboard-access rules); touch platforms mostly reserve focus for text input, so this section
/// is expected to stay quiet there (docs/focus.md).
fn probe_section() -> impl Piece {
    let on_toggle = Signal::new(false);
    let on_slider = Signal::new(false);
    let on_button = Signal::new(false);
    let flag = Signal::new(true);
    let level = Signal::new(30.0f64);
    section((
        labeled(
            crate::res::str::focus_probe_toggle(),
            toggle(flag).id("focus-probe-toggle").focused(on_toggle),
        ),
        labeled(
            crate::res::str::focus_probe_slider(),
            slider(level)
                .range(0.0..=100.0)
                .id("focus-probe-slider")
                .focused(on_slider),
        ),
        button(crate::res::str::focus_probe_button())
            .id("focus-probe-button")
            .focused(on_button),
        labeled(
            crate::res::str::focus_current_label(),
            label(move || {
                if on_toggle.get() {
                    crate::res::str::focus_probe_toggle().format()
                } else if on_slider.get() {
                    crate::res::str::focus_probe_slider().format()
                } else if on_button.get() {
                    crate::res::str::focus_probe_button().format()
                } else {
                    "—".into()
                }
            })
            .id("focus-probe-current"),
        ),
        label(crate::res::str::focus_probe_caption()).font(Font::Footnote),
    ))
    .title(crate::res::str::focus_probe_section())
}
