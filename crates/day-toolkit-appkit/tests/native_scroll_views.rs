// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

#[cfg(target_os = "macos")]
fn main() {
    native::run();
}

/// Scroll view preferences, animated and instant programmatic scrolls, zoom, and aligned list
/// scrolls on real AppKit views (docs/scroll.md, docs/list.md "Programmatic scrolling"). Every
/// scroll is programmatic; no events are posted.
#[cfg(target_os = "macos")]
mod native {
    use day_core::ScrollTarget;
    use day_pieces::prelude::*;
    use day_spec::{RowAlign, Size, WindowOptions};
    use day_toolkit_appkit::AppKit;
    use objc2::{MainThreadMarker, Message, rc::Retained};
    use objc2_app_kit::{NSApplication, NSScrollElasticity, NSScrollView, NSTableView, NSView};

    /// The first scroll view whose class name ends with `class` (AppKit may observe the view
    /// through a key-value-observing subclass named after it).
    fn find_scroll(view: &NSView, class: &str) -> Option<Retained<NSScrollView>> {
        if view
            .class()
            .name()
            .to_str()
            .is_ok_and(|n| n.ends_with(class))
            && let Some(sv) = view.downcast_ref::<NSScrollView>()
        {
            return Some(sv.retain());
        }
        view.subviews().iter().find_map(|v| find_scroll(&v, class))
    }

    fn find_table(view: &NSView) -> Option<Retained<NSTableView>> {
        if let Some(table) = view.downcast_ref::<NSTableView>() {
            return Some(table.retain());
        }
        view.subviews().iter().find_map(|v| find_table(&v))
    }

    fn root() -> Retained<NSView> {
        let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
        app.windows().objectAtIndex(0).contentView().unwrap()
    }

    fn origin_y(sv: &NSScrollView) -> f64 {
        sv.contentView().bounds().origin.y
    }

    fn check(ok: bool, what: &str) {
        if !ok {
            eprintln!("native scroll views: FAILED {what}");
            std::process::exit(1);
        }
        println!("native scroll views: {what}");
    }

    #[expect(
        clippy::too_many_lines,
        reason = "one sequential native scenario on the main thread"
    )]
    pub fn run() {
        std::thread::spawn(|| {
            std::thread::sleep(std::time::Duration::from_secs(30));
            eprintln!("native scroll views test timed out");
            std::process::exit(2);
        });
        let jump = Signal::new(None::<ScrollTarget>);
        let offset = Signal::new(Point::ZERO);
        let scale = Signal::new(1.0_f64);
        let row = Signal::new(None::<RowTarget>);
        let visible = Signal::new(0_usize..0);
        day_core::launch_with(
            AppKit::new(),
            WindowOptions {
                title: "Fixture scroll views".into(),
                size: Size::new(500., 500.),
                ..Default::default()
            },
            move || {
                day_core::task(async move {
                    day_core::sleep(300).await;
                    let sv = find_scroll(&root(), "DayScrollView").unwrap();
                    let phased = find_scroll(&root(), "DayPhasedScrollView");
                    check(phased.is_some(), "a paging scroll view reads wheel phases");
                    check(
                        !sv.hasVerticalScroller() && !sv.hasHorizontalScroller(),
                        "hidden indicators remove both scrollers",
                    );
                    check(
                        sv.verticalScrollElasticity() == NSScrollElasticity::None,
                        "no bounce stops at the edges",
                    );
                    check(
                        sv.allowsMagnification()
                            && (sv.minMagnification() - 1.0).abs() < 1e-9
                            && (sv.maxMagnification() - 4.0).abs() < 1e-9,
                        "the zoom range is the magnification range",
                    );

                    // An animated scroll passes between its ends and lands on the bottom.
                    // Sampled until it lands rather than once at a fixed time: on a slow runner
                    // the animation may not have started yet at any one instant.
                    jump.set(Some(ScrollTarget::Bottom));
                    let mut between = false;
                    for _ in 0..300 {
                        day_core::sleep(5).await;
                        let y = origin_y(&sv);
                        between |= y > 0.5 && y < 1079.5;
                        if (y - 1080.0).abs() < 0.5 {
                            break;
                        }
                    }
                    check(between, "an animated scroll_to passes between its ends");
                    day_core::sleep(200).await;
                    check(
                        (origin_y(&sv) - 1080.0).abs() < 0.5
                            && (offset.get().y - 1080.0).abs() < 0.5,
                        "it rests at the bottom and the offset binding says so",
                    );

                    // A jump during an animation wins.
                    jump.set(Some(ScrollTarget::Top));
                    day_core::sleep(60).await;
                    day_core::with_tree(|t| {
                        let node = t.find_by_id("fixture_scroll").unwrap();
                        let mut scroll = node;
                        while t.node_kind(scroll) != Some(day_spec::kinds::SCROLL) {
                            scroll = t.first_child(scroll).unwrap();
                        }
                        t.scroll_to_target(scroll, &ScrollTarget::Bottom, false);
                    });
                    day_core::sleep(600).await;
                    check(
                        (origin_y(&sv) - 1080.0).abs() < 0.5,
                        "a jump replaces the animated scroll still running",
                    );

                    // Zoom keeps the viewport's center and reports the scale it rests on.
                    let before = sv.contentView().bounds();
                    let center = before.origin.y + before.size.height / 2.0;
                    scale.set(2.0);
                    day_core::sleep(100).await;
                    let m = sv.magnification();
                    check(m > 1.0 && m < 2.0, "an animated zoom is in flight");
                    day_core::sleep(700).await;
                    let after = sv.contentView().bounds();
                    check(
                        (sv.magnification() - 2.0).abs() < 1e-6 && (scale.get() - 2.0).abs() < 1e-6,
                        "the zoom rests at 2 and the binding says so",
                    );
                    check(
                        (after.origin.y + after.size.height / 2.0 - center).abs() < 1.0,
                        "the viewport's center stays put",
                    );
                    scale.set(9.0);
                    day_core::sleep(700).await;
                    check(
                        (sv.magnification() - 4.0).abs() < 1e-6 && (scale.get() - 4.0).abs() < 1e-6,
                        "a scale past the range clamps and is reported",
                    );

                    // An animated, centered row scroll lands with the row in the middle.
                    let table = find_table(&root()).unwrap();
                    let list = table.enclosingScrollView().unwrap();
                    row.set(Some(RowTarget::new(150).align(RowAlign::Center)));
                    day_core::sleep(90).await;
                    let mid = origin_y(&list);
                    check(
                        mid > 0.0 && mid < 4720.0,
                        "an animated row scroll is in flight",
                    );
                    day_core::sleep(700).await;
                    let bounds = list.contentView().bounds();
                    let rect = table.rectOfRow(150);
                    check(
                        (rect.origin.y + rect.size.height / 2.0
                            - (bounds.origin.y + bounds.size.height / 2.0))
                            .abs()
                            < 0.5,
                        "the row rests centered",
                    );
                    check(
                        visible.get().contains(&150) && visible.get().len() >= 5,
                        "visible rows follow the scroll",
                    );
                    row.set(Some(RowTarget::new(0).align(RowAlign::End).instant()));
                    day_core::sleep(200).await;
                    check(
                        origin_y(&list).abs() < 0.5,
                        "row 0 at the end stays at the top",
                    );
                    check(visible.get().start == 0, "the first row shows again");
                    std::process::exit(0);
                });
                column((
                    scroll(column(
                        (0..30)
                            .map(|_| {
                                rectangle()
                                    .fill(Color::rgb(0.5, 0.5, 0.5))
                                    .frame(120.0, 40.0)
                                    .any()
                            })
                            .collect::<Vec<_>>(),
                    ))
                    .scroll_target(jump)
                    .scroll_offset(offset)
                    .zoom(1.0..=4.0)
                    .zoom_scale(scale)
                    .hide_indicators()
                    .no_bounce()
                    .id(day_core::id!(fixture_scroll))
                    .frame(120.0, 120.0),
                    scroll(label("Paged")).paging().frame(120.0, 40.0),
                    list(
                        items(|| (0..300).collect::<Vec<usize>>(), |n: &usize| *n),
                        |slot| label(move || format!("Row {}", slot.get())),
                    )
                    .row_height(RowHeight::Uniform(32.0))
                    .scroll_to_row(row)
                    .visible_rows(visible)
                    .height(160.0),
                ))
                .any()
            },
        );
    }
}
