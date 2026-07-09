use day::prelude::*;

/// Preferences playground (docs/prefs.md): the headless `day-part-prefs` part persists a
/// string under a fixed key. Save writes the text field, Load reads it back into `#prefs-value`,
/// and Clear deletes it. The value survives app launches (NSUserDefaults / SharedPreferences / a
/// config file, per platform), so Load returns the stored value even after the field is typed over.
pub(crate) fn prefs_page() -> AnyPiece {
    const KEY: &str = "showcase.remembered";
    let field = Signal::new(String::new());
    let value = Signal::new(tr("prefs-empty").format());
    let status = Signal::new(tr("prefs-idle").format());
    column((
        label(tr("nav-prefs")).font(Font::Title).id("prefs-title"),
        label(tr("prefs-caption")),
        text_field(field)
            .placeholder(tr("prefs-placeholder"))
            .id("prefs-field"),
        row((
            button(tr("prefs-save"))
                .action(move || {
                    let ok = field.with(|t| day_part_prefs::set(KEY, t));
                    let msg = if ok {
                        tr("prefs-saved")
                    } else {
                        tr("prefs-save-failed")
                    };
                    status.set(msg.format());
                })
                .id("prefs-save"),
            button(tr("prefs-load"))
                .action(move || match day_part_prefs::get(KEY) {
                    Some(v) => {
                        value.set(v);
                        status.set(tr("prefs-loaded").format());
                    }
                    None => {
                        value.set(tr("prefs-empty").format());
                        status.set(tr("prefs-missing").format());
                    }
                })
                .id("prefs-load"),
            button(tr("prefs-clear"))
                .action(move || {
                    day_part_prefs::remove(KEY);
                    value.set(tr("prefs-empty").format());
                    status.set(tr("prefs-cleared").format());
                })
                .id("prefs-clear"),
        ))
        .spacing(8.0),
        label(move || status.get()).id("prefs-status"),
        row((
            label(tr("prefs-value-label")),
            label(move || value.get()).id("prefs-value"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
