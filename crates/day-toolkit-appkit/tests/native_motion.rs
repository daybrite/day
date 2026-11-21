// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(not(target_os = "macos"))]
fn main() {}

/// The animation contract on real layer-backed AppKit views (docs/tween.md): every channel is
/// read back mid-flight through `read_displayed` and compared with `AnimSpec::sample` at the
/// time Core Animation reports, and every run is checked to end on the model value. No events
/// are posted; the window is ordered front so Core Animation commits.
#[cfg(target_os = "macos")]
fn main() {
    use day_spec::props::{ContainerPatch, ContainerProps};
    use day_spec::{AnimSpec, Color, NodeId, Rect, Size, Toolkit, Transform, kinds};
    use day_toolkit_appkit::AppKit;
    use objc2::{MainThreadMarker, MainThreadOnly};
    use objc2_app_kit::{NSApplication, NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSDate, NSPoint, NSRect, NSRunLoop, NSSize};
    use std::time::Instant;

    fn spin(secs: f64) {
        let until = NSDate::dateWithTimeIntervalSinceNow(secs);
        NSRunLoop::currentRunLoop().runUntilDate(&until);
    }
    fn check(ok: bool, what: &str) {
        if !ok {
            eprintln!("native motion: FAILED {what}");
            std::process::exit(1);
        }
        println!("native motion: {what}");
    }
    /// The value `spec` gives at `elapsed` for a run from `from` to `to`, and how far a read
    /// may be from it: the spread over ±40 ms of clock (a frame or two between Core
    /// Animation's clock and this one) plus 3% of the distance.
    fn expect(spec: AnimSpec, from: f64, to: f64, elapsed: f64) -> (f64, f64) {
        let at = |t: f64| {
            let s = spec.sample(t.max(0.0));
            if s.done {
                to
            } else {
                from + (to - from) * s.progress
            }
        };
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for i in 0..=8 {
            let v = at(elapsed - 0.04 + 0.01 * f64::from(i));
            lo = lo.min(v);
            hi = hi.max(v);
        }
        (
            f64::midpoint(lo, hi),
            (hi - lo) / 2.0 + (to - from).abs() * 0.03 + 0.01,
        )
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
    let filled = |color: Color| ContainerProps {
        fill: Some(color.into()),
        ..Default::default()
    };
    let boxv = toolkit.realize(
        kinds::CONTAINER,
        &filled(Color::rgb(1.0, 0.0, 0.0)),
        NodeId(2),
    );
    toolkit.insert(&stack, &boxv, 0);
    toolkit.set_frame(&boxv, Rect::new(10.0, 20.0, 80.0, 40.0), None);
    spin(0.2);

    let shown = |toolkit: &AppKit| toolkit.read_displayed(&boxv);
    let rest = shown(&toolkit);
    check(
        rest.opacity == Some(1.0)
            && rest.translation == Some(day_spec::Point::ZERO)
            && rest.background == Some(Color::rgb(1.0, 0.0, 0.0))
            && rest.frame.is_some_and(|f| {
                (f.size.width - 80.0).abs() < 1e-6 && (f.origin.x - 10.0).abs() < 1e-6
            }),
        "a view at rest answers its model values",
    );

    // Opacity with a delay: the start holds, then the fade runs.
    let spec = AnimSpec::linear(400).delay(300);
    toolkit.set_opacity(&boxv, 0.0, Some(&spec));
    let t0 = Instant::now();
    for at in [0.15, 0.5, 0.65] {
        spin(at - t0.elapsed().as_secs_f64());
        let elapsed = t0.elapsed().as_secs_f64();
        let got = shown(&toolkit).opacity.unwrap_or(-1.0);
        let (want, tol) = expect(spec, 1.0, 0.0, elapsed);
        println!("   opacity at {elapsed:.3}: {got:.3} (spec {want:.3} ±{tol:.3})");
        check((got - want).abs() <= tol, "a delayed fade follows the spec");
    }
    spin(0.3);
    check(
        shown(&toolkit).opacity == Some(0.0) && boxv.alphaValue().abs() < 1e-9,
        "the fade rests on its target",
    );
    toolkit.set_opacity(&boxv, 1.0, None);

    // A frame change passes through the sizes between.
    let spec = AnimSpec::linear(600);
    toolkit.set_frame(&boxv, Rect::new(10.0, 20.0, 240.0, 40.0), Some(&spec));
    let t0 = Instant::now();
    let layer = boxv.layer().expect("a layer-backed view");
    println!("   animation keys: {:?}", layer.animationKeys());
    for at in [0.15, 0.3, 0.45] {
        spin(at - t0.elapsed().as_secs_f64());
        let elapsed = t0.elapsed().as_secs_f64();
        let got = shown(&toolkit).frame.map_or(-1.0, |f| f.size.width);
        let (want, tol) = expect(spec, 80.0, 240.0, elapsed);
        println!("   width at {elapsed:.3}: {got:.2} (spec {want:.2} ±{tol:.2})");
        check(
            (got - want).abs() <= tol,
            "an animated frame passes between",
        );
    }
    spin(0.3);
    check(
        (boxv.frame().size.width - 240.0).abs() < 1e-9
            && shown(&toolkit)
                .frame
                .is_some_and(|f| (f.size.width - 240.0).abs() < 1e-9),
        "the frame rests on its target, the model with it",
    );

    // A spring on the transform overshoots and settles.
    let spec = AnimSpec::spring(0.6, 0.3);
    toolkit.set_transform(
        &boxv,
        Transform::translate(100.0, 0.0),
        Size::new(240.0, 40.0),
        Some(&spec),
    );
    let t0 = Instant::now();
    let mut peak = 0.0_f64;
    for at in [0.1, 0.2, 0.3, 0.4, 0.5] {
        spin(at - t0.elapsed().as_secs_f64());
        let elapsed = t0.elapsed().as_secs_f64();
        let got = shown(&toolkit).translation.map_or(-1.0, |p| p.x);
        peak = peak.max(got);
        let (want, tol) = expect(spec, 0.0, 100.0, elapsed);
        println!("   translation at {elapsed:.3}: {got:.2} (spec {want:.2} ±{tol:.2})");
        check(
            (got - want).abs() <= tol,
            "a spring transform follows the physical spring",
        );
    }
    check(peak > 120.0, "the spring overshoots");
    spin(spec.duration_secs());
    check(
        shown(&toolkit).translation == Some(day_spec::Point::new(100.0, 0.0))
            && (boxv.frame().origin.x - 110.0).abs() < 1e-6,
        "the spring rests on its target",
    );

    // A fill passes through the colors between and goes back to drawRect after.
    let spec = AnimSpec::linear(600);
    toolkit.update(
        &boxv,
        kinds::CONTAINER,
        &ContainerPatch::Fill(Some(Color::rgb(0.0, 0.0, 1.0).into())),
        Some(&spec),
    );
    let t0 = Instant::now();
    spin(0.3 - t0.elapsed().as_secs_f64());
    let elapsed = t0.elapsed().as_secs_f64();
    let got = shown(&toolkit).background.unwrap_or(Color::BLACK);
    let (want, tol) = expect(spec, 1.0, 0.0, elapsed);
    println!(
        "   red at {elapsed:.3}: {:.3} blue {:.3} (spec red {want:.3} ±{tol:.3})",
        got.r, got.b
    );
    check(
        (got.r - want).abs() <= tol && (got.b - (1.0 - want)).abs() <= tol,
        "an animated fill passes between",
    );
    spin(0.6);
    check(
        shown(&toolkit).background == Some(Color::rgb(0.0, 0.0, 1.0))
            && layer.backgroundColor().is_none(),
        "the fill rests on its target and is drawRect's again",
    );

    // Forever until a plain set.
    let spec = AnimSpec::linear(300).repeat_forever(true);
    toolkit.set_opacity(&boxv, 0.0, Some(&spec));
    spin(1.0);
    let mid = shown(&toolkit).opacity.unwrap_or(-1.0);
    toolkit.set_opacity(&boxv, 0.5, None);
    spin(0.05);
    check(
        mid > 0.0 && mid < 1.0 && shown(&toolkit).opacity == Some(0.5),
        "a run for ever plays until a plain set stops it",
    );
    std::process::exit(0);
}
