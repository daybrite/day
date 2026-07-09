use day::prelude::*;

/// Shape pieces (docs/shapes.md): the unified `shape` piece rendered atop the canvas — every
/// kind, fill/stroke, a slider-bound rotation, tap-to-recolor, and drag-to-move. Shapes bind to
/// signals and transform through the canvas CTM, so all of this is free reactivity + zero backend
/// geometry code.
pub(crate) fn shapes_page() -> AnyPiece {
    let angle = Signal::new(0.0f64);
    let tapped = Signal::new(false);
    let pos = Signal::new((0.0f64, 0.0f64));
    let base = Signal::new((0.0f64, 0.0f64));
    column((
        label(tr("nav-shapes")).font(Font::Title).id("shapes-title"),
        // Every shape kind: fills and strokes (two rows so all fit the split-detail pane).
        label(tr("shapes-kinds")).font(Font::Headline),
        row((
            rectangle()
                .fill(Color::hex(0x2F6FDE))
                .frame(56.0, 56.0)
                .id("shape-rect"),
            rounded_rectangle(12.0)
                .fill(Color::hex(0x8E44AD))
                .frame(56.0, 56.0)
                .id("shape-rrect"),
            circle()
                .fill(Color::hex(0x27AE60))
                .frame(56.0, 56.0)
                .id("shape-circle"),
        ))
        .spacing(12.0),
        row((
            capsule()
                .fill(Color::hex(0xE67E22))
                .frame(76.0, 40.0)
                .id("shape-capsule"),
            ellipse()
                .stroke(Color::hex(0xC0392B), 4.0)
                .frame(76.0, 48.0)
                .id("shape-ellipse"),
            arc(135.0, 270.0)
                .stroke(Color::hex(0x16A085), 6.0)
                .frame(56.0, 56.0)
                .id("shape-arc"),
        ))
        .spacing(12.0),
        // A rounded rectangle rotated live by a slider (canvas CTM transform).
        label(tr("shapes-transform")).font(Font::Headline),
        row((
            label(tr("shapes-angle")),
            slider(angle).range(0.0..=360.0).id("shapes-angle-slider"),
        ))
        .spacing(8.0),
        rounded_rectangle(10.0)
            .fill(Color::hex(0x2F6FDE))
            .rotate(move || angle.get())
            // Inset so the rotated square's corners stay within the canvas frame (backends that
            // clip children to bounds — e.g. Qt — would otherwise shave the corners at an angle).
            .inset(20.0)
            .frame(120.0, 120.0)
            .id("shapes-rotator"),
        // Tap to recolor (path-precise hit-testing).
        label(tr("shapes-tap")).font(Font::Headline),
        circle()
            .fill(move || {
                if tapped.get() {
                    Color::hex(0xE74C3C)
                } else {
                    Color::hex(0x3498DB)
                }
            })
            .on_tap(move || tapped.update(|t| *t = !*t))
            // `.id` before `.frame` so the identifier lands on the shape leaf (the gesture target),
            // not the frame wrapper — lets dayscript/autodrive address the tap directly.
            .id("shapes-tap-circle")
            .frame(90.0, 90.0),
        // Drag to move (offset bound to the drag translation).
        label(tr("shapes-drag")).font(Font::Headline),
        rectangle()
            .fill(Color::hex(0x9B59B6))
            .offset(move || pos.get().0, move || pos.get().1)
            .on_drag(move |dr| match dr.phase {
                DragPhase::Began => base.set(pos.get_untracked()),
                _ => {
                    let b = base.get_untracked();
                    pos.set((b.0 + dr.translation.x, b.1 + dr.translation.y));
                }
            })
            .id("shapes-drag-rect")
            .frame(90.0, 90.0),
    ))
    .spacing(12.0)
    .align(HAlign::Leading)
    .padding(16.0)
    .any()
}
