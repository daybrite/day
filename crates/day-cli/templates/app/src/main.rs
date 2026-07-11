fn main() {
    day::launch(
        day::WindowOptions {
            title: "{{title}}".into(),
            // A desktop-appropriate default size; mobile fills the screen regardless.
            size: day::prelude::Size::new(960.0, 640.0),
            ..Default::default()
        },
        {{ident}}::root,
    );
}
