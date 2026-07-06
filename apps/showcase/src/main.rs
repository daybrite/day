fn main() {
    // Register app-lifecycle handlers before launch so `WillLaunch` is captured (docs/lifecycle.md).
    showcase::install_lifecycle_handlers();
    day::launch(
        day::WindowOptions {
            // Name the app "Day Showcase" and tag which native toolkit is rendering it, e.g.
            // "Day Showcase (AppKit)" / "(GTK)" / "(Qt)".
            title: format!("Day Showcase ({})", day::toolkit_name()),
            size: day::prelude::Size::new(480.0, 640.0),
            min_size: None,
        },
        showcase::root,
    );
}
