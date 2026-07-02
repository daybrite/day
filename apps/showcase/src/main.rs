fn main() {
    day::launch(
        day::WindowOptions {
            title: "Day Showcase".into(),
            size: day::prelude::Size::new(480.0, 640.0),
            min_size: None,
        },
        showcase::root,
    );
}
