use day::prelude::*;

pub(crate) fn deviceinfo_page() -> AnyPiece {
    // Read the device identity once now (headless day-part-deviceinfo); Refresh re-polls it.
    let (m, s, sim) = deviceinfo_lines();
    let model = Signal::new(m);
    let system = Signal::new(s);
    let simulator = Signal::new(sim);
    column((
        label(tr("nav-deviceinfo"))
            .font(Font::Title)
            .id("deviceinfo-title"),
        label(tr("deviceinfo-caption")),
        label(move || model.get()).id("deviceinfo-model"),
        label(move || system.get()).id("deviceinfo-system"),
        label(move || simulator.get()).id("deviceinfo-simulator"),
        button(tr("deviceinfo-refresh"))
            .action(move || {
                let (m, s, sim) = deviceinfo_lines();
                model.set(m);
                system.set(s);
                simulator.set(sim);
            })
            .id("deviceinfo-refresh"),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// Read the native device identity and format each field as a localized line:
/// `(model, "name version", simulator)`. Values vary by host, so nothing is asserted exactly.
fn deviceinfo_lines() -> (String, String, String) {
    let d = day_part_deviceinfo::get();
    let model = tr("deviceinfo-model").arg("value", d.model).format();
    let system = tr("deviceinfo-system")
        .arg("name", d.system_name)
        .arg("version", d.system_version)
        .format();
    // Each branch is a full literal tr(...) call so `day lint` sees both keys (never tr(if ...)).
    let sim_value = if d.is_simulator {
        tr("deviceinfo-yes").format()
    } else {
        tr("deviceinfo-no").format()
    };
    let simulator = tr("deviceinfo-simulator").arg("value", sim_value).format();
    (model, system, simulator)
}
