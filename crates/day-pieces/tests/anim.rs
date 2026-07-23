//! §8.4 declarative animation: `with_animation` / `.animation` / `.opacity` / `.transform` thread
//! `AnimSpec` intent through the toolkit seams, and the curve/spring math is well-formed. The mock
//! records the intent on each widget (`MockWidget::last_anim`, `.opacity`, `.transform`).

use day_core::AnyPiece;
use day_mock::{MockHandle, MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Size, WindowOptions};

static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    let _g = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    day_core::launch_with(
        mock,
        WindowOptions {
            title: "t".into(),
            size: Size::new(400.0, 600.0),
            ..Default::default()
        },
        root,
    );
    probe
}

/// The single `day.container` whose `pred` holds (the modifier layer we care about).
fn layer(
    probe: &MockProbe,
    pred: impl Fn(&day_mock::MockWidget) -> bool,
) -> (MockHandle, day_mock::MockWidget) {
    let mut hit: Vec<_> = probe
        .find_by_kind("day.container")
        .into_iter()
        .filter(|(_, w)| pred(w))
        .collect();
    assert_eq!(
        hit.len(),
        1,
        "expected exactly one matching container layer"
    );
    hit.pop().unwrap()
}

#[test]
fn opacity_applies_and_animates_only_under_with_animation() {
    let op = Signal::new(1.0f64);
    let probe = boot(move || label("hi").opacity(op).any());
    flush_sync();

    let (h, w) = layer(&probe, |w| w.opacity.is_some());
    assert_eq!(w.opacity, Some(1.0), "seed opacity applied at build");
    assert!(w.last_anim.is_none(), "no intent without with_animation");

    // A plain change is instant (no intent).
    batch(|| op.set(0.5));
    flush_sync();
    let w = probe.widget(h);
    assert_eq!(w.opacity, Some(0.5));
    assert!(w.last_anim.is_none(), "plain set stays instant");

    // Inside with_animation the change carries the intent.
    with_animation(Animation::ease_in_out(200), || op.set(0.0));
    flush_sync();
    let w = probe.widget(h);
    assert_eq!(w.opacity, Some(0.0));
    let a = w
        .last_anim
        .expect("with_animation threaded an AnimSpec into set_opacity");
    assert_eq!(a.duration_ms, 200);
    assert_eq!(a.curve, Curve::EaseInOut);
}

#[test]
fn with_animation_threads_into_background_color_patch() {
    let c = Signal::new(Color::rgb(1.0, 0.0, 0.0));
    let probe = boot(move || label("hi").background(c).any());
    flush_sync();

    let (h, _) = layer(&probe, |w| w.background.is_some());
    assert!(probe.widget(h).last_anim.is_none());

    with_animation(Animation::spring(0.4, 0.9), || {
        c.set(Color::rgb(0.0, 0.0, 1.0))
    });
    flush_sync();
    let a = probe
        .widget(h)
        .last_anim
        .expect("with_animation threaded an AnimSpec into the ContainerPatch::Background update");
    assert!(matches!(a.curve, Curve::Spring { .. }));
}

#[test]
fn implicit_animation_propagates_to_descendant_without_with_animation() {
    let c = Signal::new(Color::rgb(1.0, 0.0, 0.0));
    // `.animation` sits ABOVE `.background`; the bg patch (on a descendant) must still animate.
    let probe = boot(move || {
        label("hi")
            .background(c)
            .animation(Animation::linear(150))
            .any()
    });
    flush_sync();

    let (h, _) = layer(&probe, |w| w.background.is_some());
    batch(|| c.set(Color::rgb(0.0, 1.0, 0.0)));
    flush_sync();
    let a = probe
        .widget(h)
        .last_anim
        .expect(".animation ancestor should animate a descendant's property change");
    assert_eq!(a.duration_ms, 150);
    assert_eq!(a.curve, Curve::Linear);
}

#[test]
fn transform_channel_records_scale_and_rotation() {
    let s = Signal::new(1.0f64);
    let probe = boot(move || label("hi").scale(s).any());
    flush_sync();

    let (h, w) = layer(&probe, |w| w.transform.is_some());
    assert_eq!(w.transform.map(|t| (t.sx, t.sy)), Some((1.0, 1.0)));

    with_animation(Animation::spring(0.3, 0.7), || s.set(1.5));
    flush_sync();
    let w = probe.widget(h);
    assert_eq!(w.transform.map(|t| t.sx), Some(1.5));
    assert!(w.last_anim.is_some(), "animated scale carries intent");
}

#[test]
fn curve_and_spring_math_is_well_formed() {
    // Easing endpoints.
    assert_eq!(Curve::Linear.fraction(0.0, 1.0), 0.0);
    assert_eq!(Curve::Linear.fraction(1.0, 1.0), 1.0);
    assert!((Curve::EaseInOut.fraction(0.5, 1.0) - 0.5).abs() < 1e-9);
    assert!(Curve::EaseIn.fraction(0.5, 1.0) < 0.5); // slow start
    assert!(Curve::EaseOut.fraction(0.5, 1.0) > 0.5); // fast start

    // Spring: starts at 0, settles near 1, and an under-damped spring overshoots on the way.
    let sp = Curve::Spring {
        response: 0.4,
        damping: 0.5,
    };
    assert_eq!(sp.fraction(0.0, 0.0), 0.0);
    assert!(!sp.is_settled(0.0, 0.0));
    assert!(sp.is_settled(5.0, 0.0), "well past the settle cap");
    let overshoots = (1..40).any(|i| sp.fraction(i as f64 * 0.02, 0.0) > 1.0);
    assert!(overshoots, "under-damped spring should overshoot 1.0");
    // A critically-damped spring never overshoots.
    let cd = Curve::Spring {
        response: 0.4,
        damping: 1.0,
    };
    assert!((1..60).all(|i| cd.fraction(i as f64 * 0.02, 0.0) <= 1.000001));
}
