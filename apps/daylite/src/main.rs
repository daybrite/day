fn main() {
    day::launch(
        day::WindowOptions {
            title: "Daylite".into(),
            size: day::prelude::Size::new(420.0, 760.0),
            ..Default::default()
        },
        daylite::root,
    );
}
