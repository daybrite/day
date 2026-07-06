fn main() {
    // Register app-lifecycle handlers before launch so `WillLaunch` is captured (docs/lifecycle.md).
    showcase::install_lifecycle_handlers();
    day::launch(
        day::WindowOptions {
            // Name the app "Day Showcase" and tag which native toolkit is rendering it, e.g.
            // "Day Showcase (AppKit)" / "(GTK)" / "(Qt)".
            title: format!("Day Showcase ({})", day::toolkit_name()),
            // A desktop-appropriate default (the sidebar + detail split wants room); mobile ignores
            // this and fills the screen.
            size: day::prelude::Size::new(1000.0, 720.0),
            min_size: Some(day::prelude::Size::new(640.0, 480.0)),
            // The App menu / About show "Showcase", not the toolkit-tagged window title.
            app_name: Some("Showcase".into()),
        },
        showcase::root,
    );
}
