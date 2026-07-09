use day::prelude::*;

/// Network playground (docs/network.md): the headless `day-part-network` part feeds an
/// online/offline readout with the connection kind; "Read Network" re-polls the snapshot.
pub(crate) fn network_page() -> AnyPiece {
    let reading = Signal::new(network_line().format());
    column((
        label(tr("nav-network"))
            .font(Font::Title)
            .id("network-title"),
        label(tr("network-caption")),
        row((
            button(tr("network-refresh"))
                .action(move || reading.set(network_line().format()))
                .id("network-refresh"),
            label(move || reading.get()).id("network-reading"),
        ))
        .spacing(8.0),
    ))
    .spacing(10.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}

/// The current connectivity snapshot as a localized line (Fluent; kind stays the API's enum
/// debug form — it is a value, not prose).
fn network_line() -> LocalizedText {
    match day_part_network::status() {
        Some(n) => {
            let line = if n.online {
                tr("network-reading-online")
            } else {
                tr("network-reading-offline")
            };
            line.arg("kind", format!("{:?}", n.kind)).arg(
                "expensive",
                match n.expensive {
                    Some(true) => "yes",
                    Some(false) => "no",
                    None => "?",
                },
            )
        }
        None => tr("network-reading-none"),
    }
}
