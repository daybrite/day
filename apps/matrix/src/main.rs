//! Day Matrix client entry point.
fn main() {
    // The client uses a fixed light palette; force a light window appearance so native controls
    // (room list, text fields, composer) match it rather than following the system's dark mode.
    // Set before `day::launch` (main thread, no other threads yet); AppKit reads it at window
    // creation and other backends ignore it.
    #[cfg(target_os = "macos")]
    std::env::set_var("DAY_APPEARANCE", "light");
    day::launch(
        day::WindowOptions {
            title: format!("Matrix ({})", day::toolkit_name()),
            size: day::prelude::Size::new(1100.0, 760.0),
            min_size: Some(day::prelude::Size::new(360.0, 480.0)),
            app_name: Some("Matrix".into()),
        },
        matrix::root,
    );
}
