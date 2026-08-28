fn main() {
    day::launch(
        day::WindowOptions {
            // The catalog's name, not a literal — see `dayapp::window_title`.
            title: dayapp::window_title(),
            // A desktop-appropriate default size; mobile fills the screen regardless.
            size: day::prelude::Size::new(960.0, 640.0),
            ..Default::default()
        },
        dayapp::root,
    );
}
