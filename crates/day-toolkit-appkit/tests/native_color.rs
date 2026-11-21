// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// Inks on real AppKit views (docs/color.md, docs/surfaces.md): the native colors a label
/// takes are dynamic (they resolve per appearance with nothing re-applied), the colors a
/// surface's layer holds (its border) and a tinted button's layer are set again on an
/// appearance switch, and `ContainerPatch::Surface` replaces the whole style, clip included.
/// Day's own appearance override switches the app; the system setting is never touched. No
/// events are posted.
#[cfg(target_os = "macos")]
fn main() {
    use day_spec::props::{
        ButtonProps, ButtonStyleSpec, ContainerPatch, ContainerProps, LabelProps,
    };
    use day_spec::{Border, Color, Ink, NodeId, Rect, SystemColor, Toolkit, kinds};
    use day_toolkit_appkit::AppKit;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{
        NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication,
        NSBackingStoreType, NSColor, NSColorSpace, NSTextField, NSWindow, NSWindowStyleMask,
    };
    use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
    use std::cell::Cell;

    fn spin(secs: f64) {
        let until = NSDate::dateWithTimeIntervalSinceNow(secs);
        NSRunLoop::currentRunLoop().runUntilDate(&until);
    }
    fn check(ok: bool, what: &str) {
        if !ok {
            eprintln!("native color: FAILED {what}");
            std::process::exit(1);
        }
        println!("native color: {what}");
    }
    fn srgb(c: &NSColor) -> Option<Color> {
        let c = c.colorUsingColorSpace(&NSColorSpace::sRGBColorSpace())?;
        Some(Color::rgba(
            c.redComponent(),
            c.greenComponent(),
            c.blueComponent(),
            c.alphaComponent(),
        ))
    }
    fn near(a: Option<Color>, b: Option<Color>) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => {
                (a.r - b.r).abs() < 0.01
                    && (a.g - b.g).abs() < 0.01
                    && (a.b - b.b).abs() < 0.01
                    && (a.a - b.a).abs() < 0.01
            }
            _ => false,
        }
    }
    /// `color` resolved under aqua (`false`) or dark aqua (`true`).
    fn resolve_in(dark: bool, color: &NSColor) -> Option<Color> {
        let name = unsafe {
            if dark {
                NSAppearanceNameDarkAqua
            } else {
                NSAppearanceNameAqua
            }
        };
        let appearance = NSAppearance::appearanceNamed(name)?;
        let out = Cell::new(None);
        appearance.performAsCurrentDrawingAppearance(&block2::StackBlock::new(|| {
            out.set(srgb(color));
        }));
        out.get()
    }
    fn cg(
        color: Option<impl std::ops::Deref<Target = objc2_core_graphics::CGColor>>,
    ) -> Option<Color> {
        let color = color?;
        let ns = NSColor::colorWithCGColor(&color)?;
        srgb(&ns)
    }

    let mtm = MainThreadMarker::new().expect("main thread");
    let _app = NSApplication::sharedApplication(mtm);
    let mut toolkit = AppKit::new();
    toolkit.set_event_sink(Box::new(|_, _| {}));
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(100.0, 100.0), NSSize::new(400.0, 300.0)),
            NSWindowStyleMask::Titled,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    unsafe { window.setReleasedWhenClosed(false) };
    window.orderFront(None);
    let content = window.contentView().expect("content view");
    let stack = toolkit.realize(kinds::CONTAINER, &ContainerProps::default(), NodeId(1));
    stack.setFrame(NSRect::new(
        NSPoint::new(0.0, 0.0),
        NSSize::new(400.0, 300.0),
    ));
    content.addSubview(&stack);

    let (red, blue) = (Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
    let pair = Ink::adaptive(red, blue);

    // A label's colors are dynamic NSColors: each resolves per appearance by itself.
    let label = toolkit.realize(
        kinds::LABEL,
        &LabelProps {
            text: "Ink".into(),
            color: Some(pair),
            ..Default::default()
        },
        NodeId(2),
    );
    let field = label
        .downcast_ref::<NSTextField>()
        .expect("a label is a text field");
    let text = field.textColor().expect("a text color");
    check(
        near(resolve_in(false, &text), Some(red)) && near(resolve_in(true, &text), Some(blue)),
        "a label's light/dark pair resolves per appearance",
    );
    toolkit.update(
        &label,
        kinds::LABEL,
        &day_spec::props::LabelPatch::Color(Some(SystemColor::Text.with_alpha(0.5))),
        None,
    );
    let text = field.textColor().expect("a text color");
    let ink = |dark| resolve_in(dark, &NSColor::labelColor()).map(|c| c.with_alpha(0.5));
    check(
        near(resolve_in(false, &text), ink(false)) && near(resolve_in(true, &text), ink(true)),
        "a role with its alpha replaced still resolves per appearance",
    );

    // A surface: a pair fill, a role border, and a child covering it edge to edge.
    let surface = |radius: f64, border: Option<Border>, clips: bool| ContainerProps {
        fill: Some(pair.into()),
        corner_radius: radius,
        border,
        clips,
        ..Default::default()
    };
    let separator = Border {
        color: SystemColor::Separator.into(),
        width: 2.0,
    };
    let framed = toolkit.realize(
        kinds::CONTAINER,
        &surface(12.0, Some(separator), false),
        NodeId(3),
    );
    toolkit.insert(&stack, &framed, 0);
    toolkit.set_frame(&framed, Rect::new(10.0, 10.0, 80.0, 80.0), None);
    let child = toolkit.realize(
        kinds::CONTAINER,
        &ContainerProps {
            fill: Some(Color::rgb(0.0, 1.0, 0.0).into()),
            ..Default::default()
        },
        NodeId(4),
    );
    toolkit.insert(&framed, &child, 0);
    toolkit.set_frame(&child, Rect::new(0.0, 0.0, 80.0, 80.0), None);

    // A tinted button whose tint is a pair: its layer carries the tint.
    let button = toolkit.realize(
        kinds::BUTTON,
        &ButtonProps {
            title: "Tint".into(),
            enabled: true,
            style: ButtonStyleSpec::Tinted(pair),
            ..Default::default()
        },
        NodeId(5),
    );
    toolkit.insert(&stack, &button, 1);
    toolkit.set_frame(&button, Rect::new(10.0, 120.0, 100.0, 30.0), None);

    for dark in [false, true, false] {
        toolkit.set_appearance(Some(dark));
        spin(0.2);
        let side = if dark { blue } else { red };
        let layer = framed.layer().expect("a bordered surface is layer-backed");
        let separator_now = resolve_in(dark, &NSColor::separatorColor());
        check(
            (layer.borderWidth() - 2.0).abs() < 1e-9
                && near(cg(layer.borderColor()), separator_now),
            if dark {
                "the border is the dark separator after the switch"
            } else {
                "the border is the light separator"
            },
        );
        check(
            toolkit.read_displayed(&framed).background == Some(side),
            "the fill the surface shows is the pair's side for the appearance",
        );
        let tint = button.layer().and_then(|l| cg(l.backgroundColor()));
        check(
            near(tint, Some(side)),
            "a tinted button's layer follows the appearance",
        );
    }

    // The border is the layer's, which Core Animation draws over the sublayers: the child's
    // layer is one of them.
    let layer = framed.layer().expect("layer");
    let sublayers = unsafe { layer.sublayers() }.map_or(0, |s| s.count());
    check(
        sublayers > 0 && child.layer().is_some(),
        "the child is a sublayer of the bordered layer",
    );

    // `ContainerPatch::Surface` replaces the whole style: the clip, the radius and the border
    // come and go.
    let patch = |p: ContainerProps| ContainerPatch::Surface(Box::new(p));
    toolkit.update(
        &framed,
        kinds::CONTAINER,
        &patch(surface(0.0, None, false)),
        None,
    );
    check(
        !layer.masksToBounds() && layer.cornerRadius() == 0.0 && layer.borderWidth() == 0.0,
        "a surface patch to no radius and no border unclips and unborders",
    );
    toolkit.update(
        &framed,
        kinds::CONTAINER,
        &patch(surface(0.0, None, true)),
        None,
    );
    check(
        layer.masksToBounds(),
        "clips set after realize starts clipping",
    );
    toolkit.update(
        &framed,
        kinds::CONTAINER,
        &patch(surface(
            30.0,
            Some(Border {
                color: red.into(),
                width: 8.0,
            }),
            false,
        )),
        None,
    );
    check(
        layer.masksToBounds()
            && (layer.cornerRadius() - 30.0).abs() < 1e-9
            && (layer.borderWidth() - 8.0).abs() < 1e-9
            && near(cg(layer.borderColor()), Some(red)),
        "a radius and a border arrive by patch",
    );
    toolkit.update(
        &framed,
        kinds::CONTAINER,
        &patch(surface(200.0, None, false)),
        None,
    );
    check(
        (layer.cornerRadius() - 40.0).abs() < 1e-9,
        "a radius past half the size rounds fully",
    );

    // The offscreen capture (`cacheDisplayInRect:`, the snapshot's fallback) renders the layer
    // border too, over the child that covers the surface. The bitmap is in the display's
    // color space, so the channels are compared loosely.
    toolkit.update(
        &framed,
        kinds::CONTAINER,
        &patch(surface(
            0.0,
            Some(Border {
                color: red.into(),
                width: 8.0,
            }),
            false,
        )),
        None,
    );
    spin(0.2);
    let bounds = stack.bounds();
    let rep = stack
        .bitmapImageRepForCachingDisplayInRect(bounds)
        .expect("a bitmap for the capture");
    stack.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    let scale = rep.pixelsWide() as f64 / bounds.size.width;
    let at = |x: f64, y: f64| {
        rep.colorAtX_y((x * scale) as isize, (y * scale) as isize)
            .as_deref()
            .and_then(srgb)
    };
    // `framed` is at (10, 10) in the flipped stack: x 12 lies in the 8 pt border, x 50 in the
    // middle, where the green child is.
    check(
        at(12.0, 50.0).is_some_and(|c| c.r > 0.8 && c.g < 0.5)
            && at(50.0, 50.0).is_some_and(|c| c.g > 0.8 && c.r < 0.6),
        "the offscreen capture shows the border over the child",
    );

    toolkit.set_appearance(None);
    println!("native color: all checks passed");
    std::process::exit(0);
}
