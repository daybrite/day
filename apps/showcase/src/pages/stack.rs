use day::prelude::*;

/// Genuine push/pop navigation (docs/navigation.md): `stack` bound to a `Signal<Vec<String>>`
/// path. Pushing a detail appends to the path; Day reconciles the native UINavigationController
/// / AdwNavigationView / back-stack; the native back button writes the pop back into the path.
pub(crate) fn stack_page() -> AnyPiece {
    fn push(path: Signal<Vec<String>>) {
        let mut v = path.get_untracked();
        let n = v.len() + 1;
        v.push(format!("{n}"));
        path.set(v);
    }
    let path = Signal::new(Vec::<String>::new());
    let root = column((
        label(tr("stack-root-body")).id("stack-root"),
        button(tr("stack-push"))
            .action(move || push(path))
            .id("stack-push"),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0);
    stack(path, root)
        .destination(move |key| {
            let depth = key.to_string();
            column((
                label(tr("stack-detail-title").arg("depth", depth))
                    .font(Font::Title)
                    .id("stack-detail"),
                label(tr("stack-detail-body")),
                button(tr("stack-push"))
                    .action(move || push(path))
                    .id("stack-deeper"),
            ))
            .spacing(12.0)
            .align(HAlign::Leading)
            .padding(16.0)
        })
        .id("demo-stack")
}
