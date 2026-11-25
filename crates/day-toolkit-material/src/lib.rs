// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! day-toolkit-material: the android-material backend (DESIGN.md §9). jni + the `DayBridge` Java shim
//! (java/dev/daybrite/day/bridge/, the Java analogue of the Qt C++ shim; controls are Material 3
//! components from com.google.android.material, M3 Expressive themed).
//! `Handle = AHandle(GlobalRef)`. Coordinates: Day works in dp; `set_frame` scales by density to
//! px and `measure` scales back. The JVM owns the main loop: `Platform::run` hands the
//! pre-registered root straight to `ready` (the Activity already called `init`).

#![allow(clippy::missing_safety_doc)]

#[cfg(target_os = "android")]
pub use imp::*;

/// Parity test for the event-kind wire table: the Java shim's `K_*` constants block in
/// DayBridge.java must mirror `day_spec::bridge::BridgeKind` exactly. Host-runnable (pure
/// text against the enum, no JNI), so a drifted or colliding kind fails `cargo test` on
/// any host as well as on a device.
#[cfg(target_os = "android")]
mod controls;

#[cfg(test)]
mod bridge_kinds_parity {
    #[test]
    fn java_constants_match_the_rust_enum() {
        use day_spec::bridge::BridgeKind;
        let java = include_str!("../java/dev/daybrite/day/bridge/DayBridge.java");
        let mut found = std::collections::BTreeMap::new();
        for line in java.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("public static final int K_")
                && let Some((name, value)) = rest.split_once(" = ")
            {
                let value: i32 = value
                    .trim_end_matches(';')
                    .parse()
                    .unwrap_or_else(|_| panic!("unparsable K_{name} line: {line}"));
                assert!(
                    found.insert(format!("K_{name}"), value).is_none(),
                    "duplicate Java constant K_{name}"
                );
            }
        }
        let expect = [
            ("K_PRESSED", BridgeKind::Pressed),
            ("K_REFRESH_REQUESTED", BridgeKind::RefreshRequested),
            ("K_TEXT_CHANGED", BridgeKind::TextChanged),
            ("K_TOGGLE_CHANGED", BridgeKind::ToggleChanged),
            ("K_VALUE_CHANGED", BridgeKind::ValueChanged),
            ("K_VALUE_COMMITTED", BridgeKind::ValueCommitted),
            ("K_SEARCH_CHANGED", BridgeKind::SearchChanged),
            ("K_NAV_PRESENTATION", BridgeKind::NavPresentation),
            ("K_SELECTION_CHANGED", BridgeKind::SelectionChanged),
            ("K_LIST_ACTIVATED", BridgeKind::ListActivated),
            ("K_NAV_BACK", BridgeKind::NavBack),
            ("K_FRAME_CHANGED", BridgeKind::FrameChanged),
            ("K_DEEPLINK", BridgeKind::Deeplink),
            ("K_DOCUMENT_OPENED", BridgeKind::DocumentOpened),
            ("K_SHARED_TEXT", BridgeKind::SharedText),
            ("K_PRESENT_BUTTON", BridgeKind::PresentButton),
            ("K_PRESENT_TEXT", BridgeKind::PresentText),
            ("K_PRESENT_DISMISSED", BridgeKind::PresentDismissed),
            ("K_GESTURE", BridgeKind::Gesture),
            ("K_CUSTOM", BridgeKind::Custom),
            ("K_MENU_ACTION", BridgeKind::MenuAction),
            ("K_LIFECYCLE", BridgeKind::Lifecycle),
            ("K_PRESENT_FILE", BridgeKind::PresentFile),
            ("K_FOCUS_CHANGED", BridgeKind::FocusChanged),
            ("K_SUBMITTED", BridgeKind::Submitted),
            ("K_WINDOW_RESIZED", BridgeKind::WindowResized),
            ("K_SAFE_AREA", BridgeKind::SafeArea),
            // These two were absent from this table while present in DayBridge.java, so the
            // parity assertion below was failing before the search work touched it.
            ("K_WINDOW_CLOSED", BridgeKind::WindowClosed),
            ("K_WINDOW_FOCUSED", BridgeKind::WindowFocused),
            // Same story again: DayBridge.java gained K_APPEARANCE_CHANGED without this row.
            ("K_APPEARANCE_CHANGED", BridgeKind::AppearanceChanged),
            ("K_REDUCE_MOTION_CHANGED", BridgeKind::ReduceMotionChanged),
            ("K_COVER_HIDDEN", BridgeKind::CoverHidden),
            ("K_LINK_ACTIVATED", BridgeKind::LinkActivated),
            ("K_TOOLBAR_CHANGED", BridgeKind::ToolbarChanged),
            ("K_KEY", BridgeKind::Key),
            ("K_TEXT", BridgeKind::Text),
            ("K_WINDOW_STATE_CHANGED", BridgeKind::WindowStateChanged),
        ];
        assert_eq!(
            found.len(),
            expect.len(),
            "Java K_* count differs from the enum: {found:?}"
        );
        for (name, kind) in expect {
            assert_eq!(
                found.get(name).copied(),
                Some(kind as i32),
                "{name} drifted from BridgeKind::{kind:?}"
            );
        }
    }
}

#[cfg(target_os = "android")]
mod picker;
#[cfg(target_os = "android")]
mod textarea;

/// A color as the packed `0xAARRGGBB` the Java shim takes.
#[cfg(any(target_os = "android", test))]
fn argb_u32(c: day_spec::Color) -> u32 {
    let ch = |x: f64| (x.clamp(0.0, 1.0) * 255.0).round() as u32;
    (ch(c.a) << 24) | (ch(c.r) << 16) | (ch(c.g) << 8) | ch(c.b)
}

/// Whether a container's props ask for more than a color and one radius: a gradient, a
/// border, a shadow, or a radius on some corners only (docs/surfaces.md). Those go to
/// `DayFixed.setSurfaceStyle`; the rest keep the `GradientDrawable` background they always had.
#[cfg(any(target_os = "android", test))]
fn is_styled_surface(p: &day_spec::props::ContainerProps) -> bool {
    p.fill
        .as_ref()
        .is_some_and(|f| !matches!(f, day_spec::Paint::Solid(_)))
        || p.border.is_some()
        || p.shadow.is_some()
        || (p.corner_radius > 0.0 && p.corners != day_spec::Corners::ALL)
}

/// The number of slots in a styled surface spec before its gradient stops.
#[cfg(any(target_os = "android", test))]
const SURFACE_HEADER: usize = 18;

/// A styled surface as the numbers `DayFixed.setSurfaceStyle` reads, lengths scaled to px by
/// `density` and colors packed ARGB: `[0]` has fill, `[1]` fill, `[2..=5]` radii top-left,
/// top-right, bottom-right, bottom-left, `[6]` border color, `[7]` border width, `[8]` shadow
/// color (0 for none), `[9]` blur, `[10]` dx, `[11]` dy, then the gradient from `[12]` as
/// [`gradient_spec`] lays it out. Every ink is resolved against the current palette: a light
/// or dark switch recreates the activity, and the re-mounted tree resolves again.
#[cfg(any(target_os = "android", test))]
fn surface_spec(p: &day_spec::props::ContainerProps, density: f64) -> Vec<f64> {
    let color = |c: day_spec::Ink| f64::from(argb_u32(c.resolved()));
    let background = solid_fill(p.fill.as_ref());
    let radii = p.corners.radii(p.corner_radius.max(0.0) * density);
    let mut spec = vec![
        f64::from(u8::from(background.is_some())),
        background.map_or(0.0, |c| f64::from(argb_u32(c))),
        radii.top_left,
        radii.top_right,
        radii.bottom_right,
        radii.bottom_left,
    ];
    // A border or shadow whose color is fully transparent draws nothing: both ride as "none".
    let border = p.border.filter(|b| b.width > 0.0);
    spec.extend([
        border.map_or(0.0, |b| color(b.color)),
        border.map_or(0.0, |b| b.width * density),
    ]);
    let shadow = p.shadow;
    spec.extend([
        shadow.map_or(0.0, |s| color(s.color)),
        shadow.map_or(0.0, |s| s.blur.max(0.0) * density),
        shadow.map_or(0.0, |s| s.dx * density),
        shadow.map_or(0.0, |s| s.dy * density),
    ]);
    spec.extend(gradient_spec(p.fill.as_ref()));
    spec
}

/// A gradient fill as `DayFixed.setSurfaceGradient` reads it: `[0]` stop count (0 for a solid
/// fill or none), `[1]` kind (0 linear, 1 radial, 2 conic), `[2..=5]` its geometry in the unit
/// space of the surface's frame (linear: start and end points; radial: center, radius, 0;
/// conic: center, start angle in degrees clockwise from +x, 0), then `(offset, color)` per stop
/// with the colors packed ARGB.
#[cfg(any(target_os = "android", test))]
fn gradient_spec(fill: Option<&day_spec::Paint>) -> Vec<f64> {
    use day_spec::Paint;
    let (kind, geometry, stops) = match fill {
        Some(Paint::Linear(g)) => (0.0, [g.start.x, g.start.y, g.end.x, g.end.y], &g.stops),
        Some(Paint::Radial(g)) => (1.0, [g.center.x, g.center.y, g.radius, 0.0], &g.stops),
        Some(Paint::Conic(g)) => (
            2.0,
            [g.center.x, g.center.y, g.start.degrees(), 0.0],
            &g.stops,
        ),
        Some(Paint::Solid(_)) | None => return vec![0.0; SURFACE_HEADER - 12],
    };
    if stops.is_empty() {
        return vec![0.0; SURFACE_HEADER - 12];
    }
    let mut spec = vec![stops.len() as f64, kind];
    spec.extend(geometry);
    for (offset, c) in day_spec::resolved_stops(stops) {
        spec.extend([offset.clamp(0.0, 1.0), f64::from(argb_u32(c))]);
    }
    spec
}

/// The solid color of a container's fill, resolved against the current palette; `None` for
/// no fill and for a gradient, which [`gradient_spec`] carries.
#[cfg(any(target_os = "android", test))]
fn solid_fill(fill: Option<&day_spec::Paint>) -> Option<day_spec::Color> {
    match fill {
        Some(day_spec::Paint::Solid(ink)) => Some(ink.resolved()),
        _ => None,
    }
}

#[cfg(test)]
mod surface_spec_tests {
    use super::{is_styled_surface, surface_spec};
    use day_spec::props::ContainerProps;
    use day_spec::{
        Angle, Border, Color, ConicGradient, Corners, LinearGradient, RadialGradient, Shadow,
        UnitPoint,
    };

    #[test]
    fn plain_surfaces_keep_the_drawable_path() {
        let plain = ContainerProps {
            fill: Some(Color::rgb(1.0, 0.0, 0.0).into()),
            corner_radius: 8.0,
            clips: true,
            ..Default::default()
        };
        assert!(!is_styled_surface(&plain));
        // Named corners with no radius round nothing, so they are plain too.
        let square = ContainerProps {
            corners: Corners::TOP,
            ..Default::default()
        };
        assert!(!is_styled_surface(&square));
        for styled in [
            ContainerProps {
                corner_radius: 8.0,
                corners: Corners::TOP,
                ..Default::default()
            },
            ContainerProps {
                border: Some(Border::new(Color::rgb(0.0, 0.0, 0.0), 1.0)),
                ..Default::default()
            },
            ContainerProps {
                shadow: Some(Shadow::new(Color::rgb(0.0, 0.0, 0.0), 4.0)),
                ..Default::default()
            },
            ContainerProps {
                fill: Some(
                    LinearGradient::vertical(Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0))
                        .into(),
                ),
                ..Default::default()
            },
        ] {
            assert!(is_styled_surface(&styled), "{styled:?}");
        }
    }

    #[test]
    fn lengths_scale_and_corners_select() {
        let p = ContainerProps {
            fill: Some(Color::rgba(0.0, 0.0, 1.0, 1.0).into()),
            corner_radius: 10.0,
            corners: Corners::TOP_LEFT | Corners::BOTTOM_RIGHT,
            border: Some(Border::new(Color::rgb(1.0, 0.0, 0.0), 2.0)),
            shadow: Some(Shadow::new(Color::rgba(0.0, 0.0, 0.0, 0.5), 4.0).offset(1.0, -3.0)),
            ..Default::default()
        };
        let spec = surface_spec(&p, 2.0);
        assert_eq!(spec.len(), super::SURFACE_HEADER);
        assert_eq!(spec[0], 1.0);
        assert_eq!(spec[1], f64::from(0xFF00_00FF_u32));
        assert_eq!(&spec[2..6], &[20.0, 0.0, 20.0, 0.0]);
        assert_eq!(spec[6], f64::from(0xFFFF_0000_u32));
        assert_eq!(spec[7], 4.0);
        assert_eq!(spec[8], f64::from(0x8000_0000_u32));
        assert_eq!(&spec[9..12], &[8.0, 2.0, -6.0]);
        assert_eq!(spec[12], 0.0);
    }

    #[test]
    fn gradient_stops_follow_the_header() {
        let p = ContainerProps {
            fill: Some(
                LinearGradient::vertical(Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0))
                    .into(),
            ),
            // A zero-width border is no border.
            border: Some(Border::new(Color::rgb(1.0, 0.0, 0.0), 0.0)),
            ..Default::default()
        };
        let spec = surface_spec(&p, 3.0);
        assert_eq!(spec[0], 0.0);
        assert_eq!(&spec[6..8], &[0.0, 0.0]);
        assert_eq!(spec[8], 0.0);
        assert_eq!(&spec[12..14], &[2.0, 0.0]);
        // Unit points are not lengths: the density leaves them alone.
        assert_eq!(&spec[14..18], &[0.5, 0.0, 0.5, 1.0]);
        assert_eq!(
            &spec[18..],
            &[
                0.0,
                f64::from(0xFFFF_0000_u32),
                1.0,
                f64::from(0xFF00_00FF_u32)
            ]
        );
    }

    #[test]
    fn radial_and_conic_fills_carry_their_geometry() {
        let radial = ContainerProps {
            fill: Some(
                RadialGradient::new(
                    UnitPoint::new(0.25, 0.75),
                    0.5,
                    [
                        (0.0, Color::rgb(1.0, 0.0, 0.0)),
                        (1.0, Color::rgb(0.0, 0.0, 1.0)),
                    ],
                )
                .into(),
            ),
            ..Default::default()
        };
        assert!(is_styled_surface(&radial));
        let spec = surface_spec(&radial, 2.0);
        assert_eq!(&spec[12..18], &[2.0, 1.0, 0.25, 0.75, 0.5, 0.0]);
        let conic = ContainerProps {
            fill: Some(
                ConicGradient::centered(
                    Angle::rad(std::f64::consts::FRAC_PI_2),
                    [
                        (0.0, Color::rgb(1.0, 0.0, 0.0)),
                        (1.0, Color::rgb(0.0, 0.0, 1.0)),
                    ],
                )
                .into(),
            ),
            ..Default::default()
        };
        assert!(is_styled_surface(&conic));
        let spec = surface_spec(&conic, 2.0);
        assert_eq!(&spec[12..14], &[2.0, 2.0]);
        assert_eq!(&spec[14..16], &[0.5, 0.5]);
        assert!((spec[16] - 90.0).abs() < 1e-9);
        assert_eq!(spec.len(), super::SURFACE_HEADER + 4);
    }

    #[test]
    fn inks_resolve_against_the_current_palette() {
        use day_spec::{Ink, Palette};
        let pair = Ink::adaptive(Color::rgb(1.0, 0.0, 0.0), Color::rgb(0.0, 0.0, 1.0));
        let p = ContainerProps {
            fill: Some(pair.into()),
            border: Some(Border::new(pair, 1.0)),
            ..Default::default()
        };
        Palette::set_current(Palette::fallback(true));
        let dark = surface_spec(&p, 1.0);
        Palette::set_current(Palette::fallback(false));
        let light = surface_spec(&p, 1.0);
        assert_eq!(dark[1], f64::from(0xFF00_00FF_u32));
        assert_eq!(dark[6], f64::from(0xFF00_00FF_u32));
        assert_eq!(light[1], f64::from(0xFFFF_0000_u32));
        assert_eq!(light[6], f64::from(0xFFFF_0000_u32));
    }
}

/// The `BridgeKind::Gesture` codes this backend sends besides the shared
/// [`day_spec::bridge::gesture`] ones (4 to 11): a tap and a drag's phases below them, and this
/// toolkit's own pinch, rotation, wheel pan and hover above them. DayBridge.java's `G_*`
/// constants mirror these; `gesture_codes_parity` holds the two files together.
#[cfg(any(target_os = "android", test))]
mod gesture_wire {
    pub const TAP: i32 = 0;
    pub const DRAG_BEGAN: i32 = 1;
    pub const DRAG_CHANGED: i32 = 2;
    pub const DRAG_ENDED: i32 = 3;
    pub const ROTATE_BEGAN: i32 = 13;
    pub const ROTATE_CHANGED: i32 = 14;
    pub const ROTATE_ENDED: i32 = 15;
    pub const PINCH_BEGAN: i32 = 16;
    pub const PINCH_CHANGED: i32 = 17;
    pub const PINCH_ENDED: i32 = 18;
    pub const PAN: i32 = 19;
    pub const HOVER_BEGAN: i32 = 20;
    pub const HOVER_CHANGED: i32 = 21;
    pub const HOVER_ENDED: i32 = 22;
}

/// One `K_GESTURE` report as the event it stands for (docs/gestures.md): `code` is the gesture
/// code, `text` its payload in px, `density` px per point. The pointer kind is the one report
/// that names no node: the caller sends it to the window. `None` for an unknown code or a
/// payload that does not parse.
#[cfg(any(target_os = "android", test))]
fn gesture_event(code: i32, text: &str, density: f64) -> Option<day_spec::Event> {
    use day_spec::bridge::gesture as shared;
    use day_spec::{DragPhase, Event, GestureKind, Point, PointerKind};
    use gesture_wire as own;
    match code {
        shared::POINTER_KIND => return PointerKind::from_name(text).map(Event::PointerKindChanged),
        shared::TOUCHES => return touches(text, density).map(Event::Touches),
        _ => {}
    }
    let p: Vec<f64> = text
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    if p.len() < 2 {
        return None;
    }
    let at = Point::new(p[0] / density, p[1] / density);
    match code {
        shared::LONG_PRESS => return Some(Event::LongPress(at)),
        shared::DOUBLE_TAP => return Some(Event::DoubleTap(at)),
        shared::PRESS_IN | shared::PRESS_OUT => {
            return Some(Event::Press {
                pressed: code == shared::PRESS_IN,
                location: at,
            });
        }
        // The platform recognizer's release velocity, px per second: just before `Ended`.
        shared::DRAG_VELOCITY | shared::PAN_VELOCITY => {
            let kind = if code == shared::DRAG_VELOCITY {
                GestureKind::Drag
            } else {
                GestureKind::Pan
            };
            return Some(Event::GestureVelocity { kind, velocity: at });
        }
        _ => {}
    }
    if p.len() < 4 {
        return None;
    }
    let tr = Point::new(p[2] / density, p[3] / density);
    let phase = |began, ended| {
        if code == began {
            DragPhase::Began
        } else if code == ended {
            DragPhase::Ended
        } else {
            DragPhase::Changed
        }
    };
    Some(match code {
        own::TAP => Event::Tap(at),
        // A drag's Began carries the translation already applied: a drag along an axis begins
        // once its movement has shown the direction (docs/gestures.md "Velocity").
        own::DRAG_BEGAN | own::DRAG_CHANGED | own::DRAG_ENDED => Event::Drag {
            phase: phase(own::DRAG_BEGAN, own::DRAG_ENDED),
            location: at,
            translation: tr,
        },
        // Pinch (docs/canvas.md "Zoom and pan"): the ScaleGestureDetector's focus and its span
        // relative to the span at Began, a cumulative scale.
        own::PINCH_BEGAN | own::PINCH_CHANGED | own::PINCH_ENDED => Event::Pinch {
            phase: phase(own::PINCH_BEGAN, own::PINCH_ENDED),
            scale: p[2],
            location: at,
        },
        // Rotation: the angle the line between the first two fingers turned since the second
        // went down, cumulative radians, clockwise positive on the y-down screen.
        own::ROTATE_BEGAN | own::ROTATE_CHANGED | own::ROTATE_ENDED => Event::Rotate {
            phase: phase(own::ROTATE_BEGAN, own::ROTATE_ENDED),
            rotation: p[2],
            location: at,
        },
        own::PAN => Event::Pan {
            phase: DragPhase::Changed,
            location: at,
            delta: tr,
        },
        // Hover (docs/canvas.md "Interaction"): a mouse or a stylus over the view. A finger
        // produces none: Android's hover events come from MotionEvent's HOVER_* actions only.
        own::HOVER_BEGAN | own::HOVER_CHANGED | own::HOVER_ENDED => Event::Hover {
            phase: phase(own::HOVER_BEGAN, own::HOVER_ENDED),
            location: at,
        },
        _ => return None,
    })
}

/// A touch report (`"id,phase,x,y,pressure,kind"` per contact, joined by `;`, px) as the
/// contacts it lists, in points. `None` when no contact parses.
#[cfg(any(target_os = "android", test))]
fn touches(text: &str, density: f64) -> Option<Vec<day_spec::Touch>> {
    use day_spec::{Point, PointerKind, Touch, TouchPhase};
    let list: Vec<Touch> = text
        .split(';')
        .filter_map(|contact| {
            let f: Vec<f64> = contact
                .split(',')
                .map(|s| s.trim().parse().ok())
                .collect::<Option<_>>()?;
            let [id, phase, x, y, pressure, kind] = f[..] else {
                return None;
            };
            let phase = match phase as i32 {
                0 => TouchPhase::Began,
                1 => TouchPhase::Moved,
                2 => TouchPhase::Stationary,
                3 => TouchPhase::Ended,
                _ => TouchPhase::Cancelled,
            };
            let kind = match kind as i32 {
                1 => PointerKind::Mouse,
                2 => PointerKind::Stylus,
                _ => PointerKind::Touch,
            };
            Some(
                Touch::new(
                    id.max(0.0) as u64,
                    phase,
                    Point::new(x / density, y / density),
                )
                .with_pressure(pressure)
                .with_kind(kind),
            )
        })
        .collect();
    (!list.is_empty()).then_some(list)
}

/// A key report (`K_KEY`, or `K_TEXT` when `typed`) as its event: `num` holds the modifier
/// mask in its low byte and the phase in bits 8 and 9 (docs/menus.md "Keys").
#[cfg(any(target_os = "android", test))]
fn key_event(typed: bool, name: String, num: f64) -> day_spec::KeyEvent {
    let mods = ((num as u32) & 0xff) as u8;
    let event = if typed {
        day_spec::KeyEvent::typed_text(name, mods)
    } else {
        day_spec::KeyEvent::named(name, mods)
    };
    event.with_phase(day_spec::KeyPhase::from_wire(num))
}

#[cfg(test)]
mod input_wire_tests {
    use super::{gesture_event, gesture_wire, key_event};
    use day_spec::bridge::gesture;
    use day_spec::{
        DragPhase, Event, GestureKind, Key, KeyPhase, Modifiers, Point, PointerKind, TouchPhase,
    };

    /// DayBridge.java's `G_*` constants are this file's codes, the shared and the own ones.
    #[test]
    fn gesture_codes_parity() {
        let java = include_str!("../java/dev/daybrite/day/bridge/DayBridge.java");
        let mut found = std::collections::BTreeMap::new();
        for line in java.lines() {
            if let Some(rest) = line.trim().strip_prefix("static final int G_")
                && let Some((name, value)) = rest.split_once(" = ")
            {
                let value: i32 = value.trim_end_matches(';').parse().expect("a number");
                assert!(
                    found.insert(name.to_owned(), value).is_none(),
                    "G_{name} twice"
                );
            }
        }
        let expect = [
            ("TAP", gesture_wire::TAP),
            ("DRAG_BEGAN", gesture_wire::DRAG_BEGAN),
            ("DRAG_CHANGED", gesture_wire::DRAG_CHANGED),
            ("DRAG_ENDED", gesture_wire::DRAG_ENDED),
            ("LONG_PRESS", gesture::LONG_PRESS),
            ("DOUBLE_TAP", gesture::DOUBLE_TAP),
            ("PRESS_IN", gesture::PRESS_IN),
            ("PRESS_OUT", gesture::PRESS_OUT),
            ("TOUCHES", gesture::TOUCHES),
            ("DRAG_VELOCITY", gesture::DRAG_VELOCITY),
            ("POINTER_KIND", gesture::POINTER_KIND),
            ("ROTATE_BEGAN", gesture_wire::ROTATE_BEGAN),
            ("ROTATE_CHANGED", gesture_wire::ROTATE_CHANGED),
            ("ROTATE_ENDED", gesture_wire::ROTATE_ENDED),
            ("PINCH_BEGAN", gesture_wire::PINCH_BEGAN),
            ("PINCH_CHANGED", gesture_wire::PINCH_CHANGED),
            ("PINCH_ENDED", gesture_wire::PINCH_ENDED),
            ("PAN", gesture_wire::PAN),
            ("HOVER_BEGAN", gesture_wire::HOVER_BEGAN),
            ("HOVER_CHANGED", gesture_wire::HOVER_CHANGED),
            ("HOVER_ENDED", gesture_wire::HOVER_ENDED),
        ];
        assert_eq!(found.len(), expect.len(), "{found:?}");
        let mut codes = std::collections::BTreeSet::new();
        for (name, code) in expect {
            assert_eq!(found.get(name), Some(&code), "G_{name}");
            assert!(codes.insert(code), "G_{name} collides with another code");
        }
        // The shared codes this backend never sends still must not collide with its own.
        assert!(!codes.contains(&gesture::PAN_VELOCITY));
    }

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    #[test]
    fn shared_gestures_decode_in_points() {
        let d = 2.0;
        assert_eq!(
            gesture_event(gesture::LONG_PRESS, "20,40", d),
            Some(Event::LongPress(p(10.0, 20.0)))
        );
        assert_eq!(
            gesture_event(gesture::DOUBLE_TAP, "2,4", d),
            Some(Event::DoubleTap(p(1.0, 2.0)))
        );
        assert_eq!(
            gesture_event(gesture::PRESS_IN, "2,4", d),
            Some(Event::Press {
                pressed: true,
                location: p(1.0, 2.0)
            })
        );
        assert_eq!(
            gesture_event(gesture::PRESS_OUT, "-2,4", d),
            Some(Event::Press {
                pressed: false,
                location: p(-1.0, 2.0)
            })
        );
        assert_eq!(
            gesture_event(gesture::DRAG_VELOCITY, "1200,-1600", d),
            Some(Event::GestureVelocity {
                kind: GestureKind::Drag,
                velocity: p(600.0, -800.0)
            })
        );
        assert_eq!(
            gesture_event(gesture::POINTER_KIND, "stylus", d),
            Some(Event::PointerKindChanged(PointerKind::Stylus))
        );
        assert_eq!(gesture_event(gesture::POINTER_KIND, "pen", d), None);
        assert_eq!(gesture_event(gesture::LONG_PRESS, "2", d), None);
    }

    #[test]
    fn own_gestures_keep_their_meaning() {
        let d = 2.0;
        assert_eq!(
            gesture_event(gesture_wire::TAP, "4,6,0,0", d),
            Some(Event::Tap(p(2.0, 3.0)))
        );
        // A Began after movement (an axis drag) keeps its translation.
        assert_eq!(
            gesture_event(gesture_wire::DRAG_BEGAN, "30,10,20,2", d),
            Some(Event::Drag {
                phase: DragPhase::Began,
                location: p(15.0, 5.0),
                translation: p(10.0, 1.0)
            })
        );
        assert!(matches!(
            gesture_event(gesture_wire::DRAG_CHANGED, "0,0,2,2", d),
            Some(Event::Drag {
                phase: DragPhase::Changed,
                ..
            })
        ));
        assert_eq!(
            gesture_event(gesture_wire::PINCH_ENDED, "10,10,1.5,0", d),
            Some(Event::Pinch {
                phase: DragPhase::Ended,
                scale: 1.5,
                location: p(5.0, 5.0)
            })
        );
        assert_eq!(
            gesture_event(gesture_wire::ROTATE_BEGAN, "10,10,0,0", d),
            Some(Event::Rotate {
                phase: DragPhase::Began,
                rotation: 0.0,
                location: p(5.0, 5.0)
            })
        );
        assert_eq!(
            gesture_event(gesture_wire::PAN, "0,0,8,-4", d),
            Some(Event::Pan {
                phase: DragPhase::Changed,
                location: p(0.0, 0.0),
                delta: p(4.0, -2.0)
            })
        );
        assert_eq!(
            gesture_event(gesture_wire::HOVER_ENDED, "2,2,0,0", d),
            Some(Event::Hover {
                phase: DragPhase::Ended,
                location: p(1.0, 1.0)
            })
        );
        assert_eq!(gesture_event(12, "0,0,0,0", d), None);
    }

    #[test]
    fn touches_list_every_contact() {
        let Some(Event::Touches(list)) =
            gesture_event(gesture::TOUCHES, "0,2,10,20,1,0;3,3,40,60,0.5,2", 2.0)
        else {
            panic!("a touch list");
        };
        assert_eq!(list.len(), 2);
        assert_eq!((list[0].id, list[0].phase), (0, TouchPhase::Stationary));
        assert_eq!(list[0].location, p(5.0, 10.0));
        assert_eq!(list[0].kind, PointerKind::Touch);
        assert_eq!((list[1].id, list[1].phase), (3, TouchPhase::Ended));
        assert!(!list[1].is_down());
        assert!((list[1].pressure - 0.5).abs() < f64::EPSILON);
        assert_eq!(list[1].kind, PointerKind::Stylus);
        assert_eq!(gesture_event(gesture::TOUCHES, "", 2.0), None);
        assert_eq!(gesture_event(gesture::TOUCHES, "1,2,3", 2.0), None);
    }

    #[test]
    fn key_phases_ride_above_the_modifiers() {
        let shift = f64::from(Modifiers::SHIFT.bits());
        let down = key_event(false, "ArrowRight".into(), shift);
        assert_eq!(down.key, Key::ArrowRight);
        assert_eq!(down.phase, KeyPhase::Down);
        assert!(down.modifiers.shift());
        let repeat = key_event(true, "a".into(), 256.0 + shift);
        assert_eq!(repeat.phase, KeyPhase::Repeat);
        assert_eq!(repeat.text.as_deref(), Some("a"));
        assert!(repeat.modifiers.shift());
        let up = key_event(false, "5".into(), 512.0);
        assert_eq!(up.phase, KeyPhase::Up);
        assert_eq!(up.modifiers, Modifiers::NONE);
        assert_eq!(up.text, None);
    }
}

/// The label wire shared with `DayBridge.setLabelRuns` / `DayLabel` (docs/text-runs.md): the
/// per-run flag bits, the custom-drawn decoration entries, and the selection report.
#[cfg(any(target_os = "android", test))]
mod text_wire {
    use day_spec::props::{TextAlign, Truncation};
    use day_spec::{Decoration, DecorationStyle, FontSpec, TextRun};

    /// `flags[i]` bits read by `DayBridge.setLabelRuns`.
    pub const BOLD: i32 = 1;
    pub const ITALIC: i32 = 2;
    pub const MONOSPACE: i32 = 4;
    /// A single strikethrough in the text color: the native `StrikethroughSpan`.
    pub const STRIKE: i32 = 8;
    pub const COLOR: i32 = 16;
    pub const BACKGROUND: i32 = 32;
    /// A single underline in the text color: the native `UnderlineSpan`.
    pub const UNDERLINE: i32 = 64;
    pub const SMALL_CAPS: i32 = 1024;
    pub const OLDSTYLE: i32 = 2048;
    pub const TABULAR: i32 = 4096;

    /// `DayLabel.DECO_*`: five ints per custom-drawn line (start, end, info, color, size per
    /// mille); the info word is bit 0 strikethrough, bits 1-2 the style, bit 3 an own color.
    pub const DECO_STRIDE: usize = 5;
    const DECO_STRIKE: i32 = 1;
    const DECO_COLORED: i32 = 8;

    /// The prefix of the selection report `DayLabel` sends through the custom channel.
    pub const SELECTION_PREFIX: &str = "day:text-selected:";

    /// The flag word for one run: the styles that ride a native span.
    pub fn run_flags(r: &TextRun) -> i32 {
        let mut f = 0;
        if r.font
            .weight
            .is_some_and(|w| w >= day_spec::FontWeight::Semibold)
        {
            f |= BOLD;
        }
        if r.font.italic {
            f |= ITALIC;
        }
        if r.font.monospace {
            f |= MONOSPACE;
        }
        if r.strikethrough.is_some_and(|d| natively_drawn(&d)) {
            f |= STRIKE;
        }
        if r.color.is_some() {
            f |= COLOR;
        }
        if r.background.is_some() {
            f |= BACKGROUND;
        }
        if r.underline.is_some_and(|d| natively_drawn(&d)) {
            f |= UNDERLINE;
        }
        if r.font.small_caps {
            f |= SMALL_CAPS;
        }
        if r.font.oldstyle_figures {
            f |= OLDSTYLE;
        }
        if r.font.tabular {
            f |= TABULAR;
        }
        f
    }

    /// Whether Android's own span draws this line: a single line in the text color.
    fn natively_drawn(d: &Decoration) -> bool {
        d.style == DecorationStyle::Single && d.color.is_none()
    }

    /// The custom-drawn decoration entries of one run spanning UTF-16 `start..end`: the lines
    /// Android has no span for (a double, dotted or wavy line, or one in its own color).
    pub fn decoration_entries(r: &TextRun, start: i32, end: i32, out: &mut Vec<i32>) {
        let scale = (r.font.scale * 1000.0).round() as i32;
        for (deco, strike) in [(r.underline, false), (r.strikethrough, true)] {
            let Some(d) = deco else { continue };
            if natively_drawn(&d) {
                continue;
            }
            let style = match d.style {
                DecorationStyle::Single => 0,
                DecorationStyle::Double => 1,
                DecorationStyle::Dotted => 2,
                DecorationStyle::Wavy => 3,
            };
            // No own color: the line takes the run's text color, which is the label's when
            // the run has none (DayLabel reads it at draw time, so it follows the theme).
            let color = d.color.or(r.color);
            let mut info = style << 1;
            if strike {
                info |= DECO_STRIKE;
            }
            if color.is_some() {
                info |= DECO_COLORED;
            }
            let entry: [i32; DECO_STRIDE] = [
                start,
                end,
                info,
                color.map_or(0, |c| super::argb_u32(c.resolved()) as i32),
                scale,
            ];
            out.extend_from_slice(&entry);
        }
    }

    /// `FontSpec::feature_tags` as Android's `setFontFeatureSettings` string (`None` for none).
    pub fn features(spec: &FontSpec) -> Option<String> {
        let tags = spec.feature_tags();
        (!tags.is_empty()).then(|| tags.join(","))
    }

    /// `DayLabel.TRUNCATE_*`.
    pub fn truncation_code(t: Truncation) -> i32 {
        match t {
            Truncation::Tail => 0,
            Truncation::Head => 1,
            Truncation::Middle => 2,
        }
    }

    /// `DayBridge.setLabelAlignment`'s code; 0 (leading) needs no call.
    pub fn align_code(a: TextAlign) -> i32 {
        match a {
            TextAlign::Leading => 0,
            TextAlign::Center => 1,
            TextAlign::Trailing => 2,
            TextAlign::Justified => 3,
        }
    }

    /// A paragraph value as `setLabelParagraph` takes it: nonpositive or nonfinite spacing
    /// and height mean the font's own.
    pub fn paragraph_args(p: &day_spec::props::LabelParagraph) -> (i32, i32, f32, f32) {
        let finite = |v: f64, positive: bool| {
            if !v.is_finite() || (positive && v <= 0.0) {
                0.0
            } else {
                v as f32
            }
        };
        (
            p.max_lines.min(i32::MAX as u32) as i32,
            truncation_code(p.truncation),
            finite(p.letter_spacing, false),
            finite(p.line_height, true),
        )
    }

    /// A selection report's payload (after [`SELECTION_PREFIX`]) as byte offsets, `start <=
    /// end`. `None` when it does not parse.
    pub fn selection(payload: &str) -> Option<(usize, usize)> {
        let (a, b) = payload.split_once(',')?;
        let (a, b): (usize, usize) = (a.trim().parse().ok()?, b.trim().parse().ok()?);
        Some((a.min(b), a.max(b)))
    }
}

#[cfg(test)]
mod text_wire_tests {
    use super::text_wire::{
        DECO_STRIDE, OLDSTYLE, SMALL_CAPS, STRIKE, TABULAR, UNDERLINE, align_code,
        decoration_entries, features, paragraph_args, run_flags, selection,
    };
    use day_spec::props::{LabelParagraph, TextAlign, Truncation};
    use day_spec::{Color, Decoration, DecorationStyle, FontSpec, TextRun};

    fn run(underline: Option<Decoration>, strikethrough: Option<Decoration>) -> TextRun {
        TextRun {
            range: 0..3,
            underline,
            strikethrough,
            ..TextRun::default()
        }
    }

    #[test]
    fn plain_lines_stay_native_spans() {
        let r = run(Some(Decoration::SINGLE), Some(Decoration::SINGLE));
        assert_eq!(run_flags(&r) & (UNDERLINE | STRIKE), UNDERLINE | STRIKE);
        let mut out = Vec::new();
        decoration_entries(&r, 0, 3, &mut out);
        assert_eq!(out, Vec::<i32>::new());
    }

    /// DayLabel.java reads the same wire: the decoration stride, the selection prefix and the
    /// feature flag bits.
    #[test]
    fn java_side_parity() {
        let label = include_str!("../java/dev/daybrite/day/bridge/DayLabel.java");
        assert!(label.contains(&format!("DECO_STRIDE = {DECO_STRIDE};")));
        assert!(label.contains(&format!("\"{}\"", super::text_wire::SELECTION_PREFIX)));
        let bridge = include_str!("../java/dev/daybrite/day/bridge/DayBridge.java");
        for (bit, tag) in [(TABULAR, "tnum"), (SMALL_CAPS, "smcp"), (OLDSTYLE, "onum")] {
            assert!(
                bridge.contains(&format!("(f & {bit}) != 0 ? \"{tag},\"")),
                "{tag} is bit {bit}"
            );
        }
    }

    #[test]
    fn styled_and_colored_lines_are_drawn() {
        let red = Color::hex(0x00D0_021B);
        let r = run(
            Some(Decoration::wavy().color(red)),
            Some(Decoration::from(DecorationStyle::Double)),
        );
        assert_eq!(run_flags(&r) & (UNDERLINE | STRIKE), 0);
        let mut out = Vec::new();
        decoration_entries(&r, 2, 9, &mut out);
        assert_eq!(out.len(), 2 * DECO_STRIDE);
        // The wavy underline: style 3, its own color.
        assert_eq!(&out[..3], &[2, 9, (3 << 1) | 8]);
        assert_eq!(out[3], super::argb_u32(red) as i32);
        assert_eq!(out[4], 1000);
        // The double strikethrough: style 1, struck, in the text color.
        assert_eq!(&out[5..8], &[2, 9, (1 << 1) | 1]);
        // A single line in a color of its own is drawn too.
        let colored = run(Some(Decoration::SINGLE.color(red)), None);
        let mut out = Vec::new();
        decoration_entries(&colored, 0, 3, &mut out);
        assert_eq!(out[2], 8);
    }

    #[test]
    fn features_follow_the_tags() {
        let mut f = FontSpec::default();
        assert_eq!(features(&f), None);
        f.small_caps = true;
        f.oldstyle_figures = true;
        f.tabular = true;
        assert_eq!(features(&f).as_deref(), Some("tnum,smcp,onum"));
        let r = TextRun {
            font: f,
            ..TextRun::default()
        };
        assert_eq!(
            run_flags(&r) & (SMALL_CAPS | OLDSTYLE),
            SMALL_CAPS | OLDSTYLE
        );
    }

    #[test]
    fn paragraph_and_alignment_codes() {
        let p = LabelParagraph {
            max_lines: 2,
            truncation: Truncation::Middle,
            letter_spacing: -0.02,
            line_height: f64::NAN,
        };
        assert_eq!(paragraph_args(&p), (2, 2, -0.02, 0.0));
        let p = LabelParagraph {
            line_height: -1.0,
            letter_spacing: f64::INFINITY,
            ..LabelParagraph::default()
        };
        assert_eq!(paragraph_args(&p), (0, 0, 0.0, 0.0));
        assert_eq!(align_code(TextAlign::Justified), 3);
        assert_eq!(align_code(TextAlign::Leading), 0);
    }

    #[test]
    fn selection_reports_parse() {
        assert_eq!(selection("3,9"), Some((3, 9)));
        assert_eq!(selection("9,3"), Some((3, 9)));
        assert_eq!(selection("0,0"), Some((0, 0)));
        assert_eq!(selection("x,1"), None);
        assert_eq!(selection("4"), None);
    }
}

/// The lists wire shared with `DayBridge` and `DayPager` (docs/list.md, docs/scroll.md,
/// docs/pager.md): a splice as `notifyItemRange*` operations, a scroll view's preference
/// word, a row scroll's alignment, and the reports that travel back.
#[cfg(any(target_os = "android", test))]
mod list_wire {
    use day_spec::props::ScrollSnap;
    use day_spec::props::{KeyboardDismiss, KeyboardTaps, RowDelta, ScrollIndicators, ScrollProps};
    use day_spec::{ListViewport, Point, RowAlign, ScrollPhase};

    /// `DayBridge.listSplice` operation kinds, each followed by two ints: rows inserted at
    /// `a`, `b` of them; rows removed at `a`, `b` of them; one row moved from `a` to `b`.
    pub const SPLICE_INSERT: i32 = 0;
    pub const SPLICE_REMOVE: i32 = 1;
    pub const SPLICE_MOVE: i32 = 2;

    /// Day's sequential row deltas as the adapter's range notifications, in order. Runs that
    /// stay contiguous fold into one range: an append of 200 rows (200 `Insert(n)`, each above
    /// the one before) is one `notifyItemRangeInserted(n, 200)`, and the highest-first removals
    /// day-core emits for a block are one `notifyItemRangeRemoved`.
    pub fn splice_ops(deltas: &[RowDelta]) -> Vec<i32> {
        let mut ops: Vec<i32> = Vec::with_capacity(deltas.len().min(64) * 3);
        for delta in deltas {
            let n = ops.len();
            let last = (n >= 3).then(|| (ops[n - 3], ops[n - 2], ops[n - 1]));
            match *delta {
                RowDelta::Insert(at) => {
                    let at = to_i32(at);
                    // A row inserted anywhere inside, or right after, a block just inserted
                    // keeps that block contiguous.
                    if let Some((SPLICE_INSERT, start, count)) = last
                        && (start..=start + count).contains(&at)
                    {
                        ops[n - 1] += 1;
                    } else {
                        ops.extend([SPLICE_INSERT, at, 1]);
                    }
                }
                RowDelta::Remove(at) => {
                    let at = to_i32(at);
                    match last {
                        // The row now at the block's start was right after it.
                        Some((SPLICE_REMOVE, start, _)) if at == start => ops[n - 1] += 1,
                        // The row right before the block.
                        Some((SPLICE_REMOVE, start, _)) if at + 1 == start => {
                            ops[n - 2] = at;
                            ops[n - 1] += 1;
                        }
                        _ => ops.extend([SPLICE_REMOVE, at, 1]),
                    }
                }
                RowDelta::Move(from, to) if from != to => {
                    ops.extend([SPLICE_MOVE, to_i32(from), to_i32(to)]);
                }
                RowDelta::Move(..) => {}
            }
        }
        ops
    }

    fn to_i32(n: usize) -> i32 {
        i32::try_from(n).unwrap_or(i32::MAX)
    }

    /// `DayBridge.makeScroll` preference bits (docs/scroll.md).
    pub const SCROLL_HIDE_INDICATORS: i32 = 1;
    pub const SCROLL_BOUNCE_NEVER: i32 = 2;
    pub const SCROLL_BOUNCE_ALWAYS: i32 = 4;
    pub const SCROLL_SNAP_PAGE: i32 = 8;
    pub const SCROLL_SNAP_INTERVAL: i32 = 16;
    pub const SCROLL_KEYBOARD_DRAG: i32 = 32;
    pub const SCROLL_KEYBOARD_TAPS: i32 = 64;
    pub const SCROLL_TRACK_OFFSET: i32 = 128;
    pub const SCROLL_TRACK_PHASE: i32 = 256;

    /// A scroll view's preference word and its snap interval in px (0 without one), unrounded
    /// so that the stops do not drift from multiples of the interval in points.
    pub fn scroll_flags(p: &ScrollProps, density: f64) -> (i32, f32) {
        let mut f = 0;
        if p.indicators == ScrollIndicators::Hidden {
            f |= SCROLL_HIDE_INDICATORS;
        }
        match p.bounce {
            Some(false) => f |= SCROLL_BOUNCE_NEVER,
            Some(true) => f |= SCROLL_BOUNCE_ALWAYS,
            None => {}
        }
        let mut interval = 0.0;
        match p.snap {
            ScrollSnap::Page => f |= SCROLL_SNAP_PAGE,
            ScrollSnap::Interval(points) if points.is_finite() && points > 0.0 => {
                let px = points * density;
                if px >= 1.0 {
                    f |= SCROLL_SNAP_INTERVAL;
                    interval = px.min(f64::from(f32::MAX)) as f32;
                }
            }
            ScrollSnap::Interval(_) | ScrollSnap::None => {}
        }
        if p.keyboard_dismiss != KeyboardDismiss::Never {
            f |= SCROLL_KEYBOARD_DRAG;
        }
        if p.keyboard_taps == KeyboardTaps::Dismiss {
            f |= SCROLL_KEYBOARD_TAPS;
        }
        if p.track_offset {
            f |= SCROLL_TRACK_OFFSET;
        }
        if p.track_phase {
            f |= SCROLL_TRACK_PHASE;
        }
        (f, interval)
    }

    /// `DayBridge.listScroll`'s alignment code.
    pub fn align_code(align: RowAlign) -> i32 {
        match align {
            RowAlign::Nearest => 0,
            RowAlign::Start => 1,
            RowAlign::Center => 2,
            RowAlign::End => 3,
        }
    }

    /// One viewport report from `DayListView` (rows, and the distance to the content's end in
    /// px) for a list of `len` rows, clamped to the contract: no report without rows, `last`
    /// never before `first`, the distance never negative.
    pub fn viewport(
        first: i32,
        last: i32,
        end_px: i32,
        density: f64,
        len: usize,
    ) -> Option<ListViewport> {
        if len == 0 || first < 0 || density <= 0.0 {
            return None;
        }
        let first = (first as usize).min(len - 1);
        let last = usize::try_from(last)
            .map_or(first, |l| l.min(len - 1))
            .max(first);
        Some(ListViewport {
            first,
            last,
            end_distance: f64::from(end_px.max(0)) / density,
        })
    }

    /// The prefix of a scroll view's offset report on the custom channel: `"x,y"` in px.
    pub const OFFSET_PREFIX: &str = "day:scroll:";
    /// The prefix of a scroll view's phase report: `0` idle, `1` dragging, `2` coasting.
    pub const PHASE_PREFIX: &str = "day:scroll-phase:";

    /// An offset report's point, in points.
    pub fn offset(payload: &str, density: f64) -> Option<Point> {
        let (x, y) = payload.split_once(',')?;
        let (x, y) = (x.trim().parse::<f64>().ok()?, y.trim().parse::<f64>().ok()?);
        (density > 0.0).then(|| Point::new(x / density, y / density))
    }

    /// A phase report's phase.
    pub fn phase(payload: &str) -> Option<ScrollPhase> {
        match payload.trim() {
            "0" => Some(ScrollPhase::Idle),
            "1" => Some(ScrollPhase::Dragging),
            "2" => Some(ScrollPhase::Coasting),
            _ => None,
        }
    }
}

#[cfg(test)]
mod list_wire_tests {
    use super::list_wire::{
        OFFSET_PREFIX, PHASE_PREFIX, SCROLL_BOUNCE_ALWAYS, SCROLL_BOUNCE_NEVER,
        SCROLL_HIDE_INDICATORS, SCROLL_KEYBOARD_DRAG, SCROLL_KEYBOARD_TAPS, SCROLL_SNAP_INTERVAL,
        SCROLL_SNAP_PAGE, SCROLL_TRACK_OFFSET, SCROLL_TRACK_PHASE, SPLICE_INSERT, SPLICE_MOVE,
        SPLICE_REMOVE, align_code, offset, phase, scroll_flags, splice_ops, viewport,
    };
    use day_spec::props::{
        KeyboardDismiss, KeyboardTaps, RowDelta, ScrollIndicators, ScrollProps, ScrollSnap,
    };
    use day_spec::{Point, RowAlign, ScrollPhase};

    /// Replays sequential deltas on a list of row ids; a fresh row is `100 + n`.
    fn replay_deltas(rows: &mut Vec<u32>, deltas: &[RowDelta]) {
        let mut fresh = 100;
        for d in deltas {
            match *d {
                RowDelta::Insert(at) => {
                    rows.insert(at, fresh);
                    fresh += 1;
                }
                RowDelta::Remove(at) => {
                    rows.remove(at);
                }
                RowDelta::Move(from, to) => {
                    let row = rows.remove(from);
                    rows.insert(to, row);
                }
            }
        }
    }

    /// Replays range operations the way `RecyclerView` reads them: the count and, for rows
    /// that survive, the order. Fresh rows are any id >= 100.
    fn replay_ops(rows: &mut Vec<u32>, ops: &[i32]) {
        let mut fresh = 100;
        for op in ops.chunks(3) {
            let (a, b) = (op[1] as usize, op[2] as usize);
            match op[0] {
                SPLICE_INSERT => {
                    for i in 0..b {
                        rows.insert(a + i, fresh);
                        fresh += 1;
                    }
                }
                SPLICE_REMOVE => {
                    rows.drain(a..a + b);
                }
                SPLICE_MOVE => {
                    let row = rows.remove(a);
                    rows.insert(b, row);
                }
                _ => panic!("unknown op"),
            }
        }
    }

    fn agree(start: &[u32], deltas: &[RowDelta]) -> Vec<i32> {
        let ops = splice_ops(deltas);
        let (mut a, mut b) = (start.to_vec(), start.to_vec());
        replay_deltas(&mut a, deltas);
        replay_ops(&mut b, &ops);
        let old = |v: &Vec<u32>| {
            v.iter()
                .map(|r| if *r >= 100 { 100 } else { *r })
                .collect::<Vec<_>>()
        };
        assert_eq!(old(&a), old(&b), "{deltas:?} -> {ops:?}");
        ops
    }

    #[test]
    fn an_append_is_one_range() {
        let deltas: Vec<_> = (0..200).map(|_| RowDelta::Insert(3)).collect();
        assert_eq!(agree(&[0, 1, 2], &deltas), vec![SPLICE_INSERT, 3, 200]);
    }

    #[test]
    fn a_removed_block_is_one_range() {
        // day-core removes highest first.
        let deltas = [
            RowDelta::Remove(4),
            RowDelta::Remove(3),
            RowDelta::Remove(2),
        ];
        assert_eq!(
            agree(&[0, 1, 2, 3, 4, 5], &deltas),
            vec![SPLICE_REMOVE, 2, 3]
        );
        let forward = [RowDelta::Remove(1), RowDelta::Remove(1)];
        assert_eq!(agree(&[0, 1, 2, 3], &forward), vec![SPLICE_REMOVE, 1, 2]);
    }

    #[test]
    fn mixed_deltas_keep_their_order() {
        let deltas = [
            RowDelta::Remove(5),
            RowDelta::Remove(1),
            RowDelta::Insert(0),
            RowDelta::Move(4, 0),
            RowDelta::Insert(2),
            RowDelta::Insert(2),
            RowDelta::Move(1, 1),
        ];
        let ops = agree(&[0, 1, 2, 3, 4, 5, 6], &deltas);
        assert_eq!(ops.len() % 3, 0);
        // The no-op move is dropped; the two inserts at 2 fold.
        assert_eq!(&ops[ops.len() - 3..], &[SPLICE_INSERT, 2, 2]);
    }

    #[test]
    fn the_keyed_case_lands_in_order() {
        // list-keyed-changes-land-in-order: insert at the top, the last row to the top.
        agree(&[1, 2, 3], &[RowDelta::Insert(0)]);
        assert_eq!(
            agree(&[9, 1, 2, 3], &[RowDelta::Move(3, 0)]),
            vec![SPLICE_MOVE, 3, 0]
        );
    }

    #[test]
    fn scroll_preferences_become_bits() {
        let plain = ScrollProps::default();
        assert_eq!(scroll_flags(&plain, 2.0), (0, 0.0));
        let all = ScrollProps {
            indicators: ScrollIndicators::Hidden,
            bounce: Some(false),
            snap: ScrollSnap::Interval(292.0),
            keyboard_dismiss: KeyboardDismiss::Interactive,
            keyboard_taps: KeyboardTaps::Dismiss,
            track_offset: true,
            track_phase: true,
            ..Default::default()
        };
        let (f, px) = scroll_flags(&all, 2.625);
        assert!((px - 766.5).abs() < 1e-3);
        assert_eq!(
            f,
            SCROLL_HIDE_INDICATORS
                | SCROLL_BOUNCE_NEVER
                | SCROLL_SNAP_INTERVAL
                | SCROLL_KEYBOARD_DRAG
                | SCROLL_KEYBOARD_TAPS
                | SCROLL_TRACK_OFFSET
                | SCROLL_TRACK_PHASE
        );
        let paged = ScrollProps {
            bounce: Some(true),
            snap: ScrollSnap::Page,
            ..Default::default()
        };
        assert_eq!(
            scroll_flags(&paged, 1.0),
            (SCROLL_BOUNCE_ALWAYS | SCROLL_SNAP_PAGE, 0.0)
        );
        let nothing = ScrollProps {
            snap: ScrollSnap::Interval(-4.0),
            ..Default::default()
        };
        assert_eq!(scroll_flags(&nothing, 1.0), (0, 0.0));
    }

    #[test]
    fn reports_decode_and_clamp() {
        assert_eq!(align_code(RowAlign::Nearest), 0);
        assert_eq!(align_code(RowAlign::End), 3);
        let v = viewport(3, 1, -5, 2.0, 10).map(|v| (v.first, v.last, v.end_distance));
        assert_eq!(v, Some((3, 3, 0.0)));
        let v = viewport(8, 40, 64, 2.0, 10).map(|v| (v.first, v.last, v.end_distance));
        assert_eq!(v, Some((8, 9, 32.0)));
        assert!(viewport(0, 0, 0, 2.0, 0).is_none());
        assert!(viewport(-1, 0, 0, 2.0, 4).is_none());
        assert_eq!(offset("2835,0", 2.625), Some(Point::new(1080.0, 0.0)));
        assert_eq!(offset("4", 1.0), None);
        assert_eq!(phase("2"), Some(ScrollPhase::Coasting));
        assert_eq!(phase("7"), None);
    }

    #[test]
    fn java_side_parity() {
        let bridge = include_str!("../java/dev/daybrite/day/bridge/DayBridge.java");
        for (name, value) in [
            ("SPLICE_INSERT", SPLICE_INSERT),
            ("SPLICE_REMOVE", SPLICE_REMOVE),
            ("SPLICE_MOVE", SPLICE_MOVE),
            ("SCROLL_HIDE_INDICATORS", SCROLL_HIDE_INDICATORS),
            ("SCROLL_BOUNCE_NEVER", SCROLL_BOUNCE_NEVER),
            ("SCROLL_BOUNCE_ALWAYS", SCROLL_BOUNCE_ALWAYS),
            ("SCROLL_SNAP_PAGE", SCROLL_SNAP_PAGE),
            ("SCROLL_SNAP_INTERVAL", SCROLL_SNAP_INTERVAL),
            ("SCROLL_KEYBOARD_DRAG", SCROLL_KEYBOARD_DRAG),
            ("SCROLL_KEYBOARD_TAPS", SCROLL_KEYBOARD_TAPS),
            ("SCROLL_TRACK_OFFSET", SCROLL_TRACK_OFFSET),
            ("SCROLL_TRACK_PHASE", SCROLL_TRACK_PHASE),
        ] {
            assert!(
                bridge.contains(&format!("static final int {name} = {value};")),
                "{name} = {value}"
            );
        }
        for (name, align) in [
            ("LIST_ALIGN_NEAREST", RowAlign::Nearest),
            ("LIST_ALIGN_START", RowAlign::Start),
            ("LIST_ALIGN_CENTER", RowAlign::Center),
            ("LIST_ALIGN_END", RowAlign::End),
        ] {
            let value = align_code(align);
            assert!(
                bridge.contains(&format!("static final int {name} = {value};")),
                "{name} = {value}"
            );
        }
        assert!(bridge.contains(&format!("\"{OFFSET_PREFIX}\"")));
        assert!(bridge.contains(&format!("\"{PHASE_PREFIX}\"")));
    }
}

/// How an `AnimSpec` crosses the bridge (docs/tween.md "The animation contract"): a timing
/// `int[]` for the `ValueAnimator` and the cycle's curve as a `float[]` of evenly spaced
/// progress samples, which `DayBridge.SampledCurve` plays as its interpolator; and how
/// `DayBridge.readDisplayed` comes back. Host-compiled so the arithmetic is unit tested.
#[cfg(any(target_os = "android", test))]
mod motion_wire {
    use day_spec::{AnimSpec, Color, Curve, Displayed, Point, Rect};

    /// `timing` slots, `DayBridge.T_*`: cycle length and delay in ms, the cycles after the
    /// first, autoreverse.
    pub const T_DURATION: usize = 0;
    pub const T_DELAY: usize = 1;
    pub const T_REPEAT: usize = 2;
    pub const T_REVERSE: usize = 3;
    pub const T_COUNT: usize = 4;

    /// `ValueAnimator.INFINITE`: the repeat of a spec that runs until the property is next set.
    pub const FOREVER: i32 = -1;

    /// The timing slots of `anim`, all zero for `None` (applied at once). Android's repeat
    /// count counts each pass after the first, reversed ones included, as `AnimSpec::repeat`
    /// does, so it is passed as is.
    pub fn timing(anim: Option<&AnimSpec>) -> [i32; T_COUNT] {
        let mut out = [0; T_COUNT];
        let Some(a) = anim else {
            return out;
        };
        let int = |v: u32| i32::try_from(v).unwrap_or(i32::MAX);
        out[T_DURATION] = int(a.duration_ms);
        out[T_DELAY] = int(a.delay_ms);
        out[T_REPEAT] = if a.repeats_forever() {
            FOREVER
        } else {
            int(a.repeat)
        };
        out[T_REVERSE] = i32::from(a.autoreverse);
        out
    }

    /// One forward cycle's progress at even offsets (`AnimSpec::keyframes`); empty for a
    /// linear cycle (and for `None`), which plays on a `LinearInterpolator`. The easing
    /// curves go as samples too, so each has the shape Day defines rather than the nearest
    /// Android interpolator's, and a spring is the physical one, overshoot included.
    pub fn keys(anim: Option<&AnimSpec>) -> Vec<f32> {
        match anim {
            Some(a) if !matches!(a.curve, Curve::Linear) && a.duration_ms > 0 => {
                a.keyframes().iter().map(|k| k.progress as f32).collect()
            }
            _ => Vec::new(),
        }
    }

    /// `DayBridge.readDisplayed`'s slots as a [`Displayed`]: `[0]` alpha, `[1]`/`[2]` the
    /// translation in dp, `[3]` 1 when a frame was read, `[4..=7]` that frame in dp, `[8]` 1
    /// when the view has a fill, `[9]` the fill as packed ARGB.
    pub fn displayed(slots: &[f64]) -> Displayed {
        let [alpha, tx, ty, has_frame, x, y, w, h, has_fill, fill] = slots else {
            return Displayed::default();
        };
        let channel = |argb: u32, shift: u32| f64::from((argb >> shift) & 0xff) / 255.0;
        // A packed ARGB is at most 0xFFFF_FFFF, which an f64 holds exactly.
        let argb = fill.clamp(0.0, f64::from(u32::MAX)) as u32;
        Displayed {
            opacity: Some(*alpha),
            frame: (*has_frame != 0.0).then(|| Rect::new(*x, *y, *w, *h)),
            translation: Some(Point::new(*tx, *ty)),
            background: (*has_fill != 0.0).then(|| {
                Color::rgba(
                    channel(argb, 16),
                    channel(argb, 8),
                    channel(argb, 0),
                    channel(argb, 24),
                )
            }),
        }
    }
}

#[cfg(test)]
mod motion_wire_tests {
    use super::motion_wire::*;
    use day_spec::{AnimSpec, Color, Point, Rect};

    #[test]
    fn timing_carries_delay_repeat_and_reverse() {
        assert_eq!(timing(None), [0; T_COUNT]);
        let t = timing(Some(&AnimSpec::linear(600).delay(900)));
        assert_eq!(t, [600, 900, 0, 0]);
        let t = timing(Some(&AnimSpec::linear(800).repeat(2, true)));
        assert_eq!((t[T_REPEAT], t[T_REVERSE]), (2, 1));
        let t = timing(Some(&AnimSpec::linear(600).repeat_forever(false)));
        assert_eq!((t[T_REPEAT], t[T_REVERSE]), (FOREVER, 0));
    }

    #[test]
    fn keys_sample_every_curve_but_linear() {
        assert_eq!(keys(None), Vec::<f32>::new());
        assert_eq!(keys(Some(&AnimSpec::linear(300))), Vec::<f32>::new());
        let ease = keys(Some(&AnimSpec::ease_in(1000)));
        assert_eq!(ease.len(), 61);
        assert!((ease[30] - 0.25).abs() < 1e-6, "t² at half way");
        let spring = AnimSpec::spring(0.9, 0.25);
        let k = keys(Some(&spring));
        assert_eq!(k.first().copied(), Some(0.0));
        assert_eq!(k.last().copied(), Some(1.0));
        assert!(k.iter().any(|&p| p > 1.3), "a loose spring overshoots");
    }

    /// The interpolator `DayBridge.SampledCurve` is, in Rust: what a spring cycle reads at
    /// an offset between two samples is within a hair of the spring itself.
    #[test]
    fn sampled_curve_follows_the_spring() {
        let spring = AnimSpec::spring(0.9, 0.25);
        let k = keys(Some(&spring));
        let last = (k.len() - 1) as f32;
        let read = |t: f32| {
            let x = t * last;
            let i = x as usize;
            let f = x - i as f32;
            k[i] + (k[(i + 1).min(k.len() - 1)] - k[i]) * f
        };
        for step in 0..100_u8 {
            let t = f64::from(step) / 100.0 + 0.003;
            let want = spring.cycle_progress(t);
            assert!(
                (f64::from(read(t as f32)) - want).abs() < 0.02,
                "{t}: {want}"
            );
        }
    }

    #[test]
    fn displayed_decodes_every_slot() {
        let d = displayed(&[
            0.5,
            10.0,
            -4.0,
            1.0,
            1.0,
            2.0,
            80.0,
            40.0,
            1.0,
            f64::from(0xFF80_4020_u32),
        ]);
        assert_eq!(d.opacity, Some(0.5));
        assert_eq!(d.translation, Some(Point::new(10.0, -4.0)));
        assert_eq!(d.frame, Some(Rect::new(1.0, 2.0, 80.0, 40.0)));
        let c = d.background.unwrap_or(Color::BLACK);
        assert!((c.r - 128.0 / 255.0).abs() < 1e-9 && (c.b - 32.0 / 255.0).abs() < 1e-9);
        let bare = displayed(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]);
        assert_eq!((bare.frame, bare.background), (None, None));
        assert_eq!(displayed(&[]).opacity, None);
    }

    #[test]
    fn java_side_parity() {
        let bridge = include_str!("../java/dev/daybrite/day/bridge/DayBridge.java");
        for (name, value) in [
            ("T_DURATION", T_DURATION),
            ("T_DELAY", T_DELAY),
            ("T_REPEAT", T_REPEAT),
            ("T_REVERSE", T_REVERSE),
            ("T_COUNT", T_COUNT),
        ] {
            assert!(
                bridge.contains(&format!("static final int {name} = {value};")),
                "{name} = {value}"
            );
        }
        assert!(bridge.contains("double[] out = new double[10];"));
    }
}

/// A typed binding to one Java class (docs/android-jni.md): `jni::bind_java_type!` with Day's
/// policies applied. Java names are kept verbatim; classes load through the app's class loader;
/// native methods catch panics and log a returned error; every binding is resolved by `init`; and
/// the class and members bound are kept from R8. Call bound methods inside [`java`].
#[cfg(target_os = "android")]
pub use day_platform_android::java_class;

#[cfg(target_os = "android")]
pub mod ext;
#[cfg(target_os = "android")]
pub use ext::*;

#[cfg(target_os = "android")]
mod imp {
    pub use day_platform_android::*;

    use framework::DayListItem;

    use std::any::Any;
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::OnceLock;

    use jni::Env;
    use jni::objects::{Global, JClass, JObject, JString};
    use linkme::distributed_slice;

    /// A shared global reference to a native View. jni 0.22's `Global` is a bare `'static` ref that
    /// is not `Clone` (cloning a global ref is a JNI call), so we wrap it in `Arc`, restoring the
    /// `Arc`-backed sharing `GlobalRef` had in 0.21, which `AHandle: Clone` (a day-core `Handle`)
    /// requires. The underlying JNI global ref is released when the last `Arc` owner drops.
    type Gref = std::sync::Arc<Global<JObject<'static>>>;

    use day_spec::bridge;
    use day_spec::props::*;
    use day_spec::{
        A11yProps, AnimSpec, Builtin, Cap, Cursor, DrawOp, Event, EventSink, Font, ListSource,
        NodeId, PieceKind, Platform, Point, Proposal, RawHandle, Rect, Registry, Renderer, Role,
        Size, Support, Toolkit, Transform, WindowOptions, kinds,
    };

    day_core::tls_group! {
        static CONTEXT_MENU_FNS: day_spec::sidetable::SideTable<day_spec::ContextMenuFn> = day_spec::sidetable::SideTable::new();
        /// Menu buttons' providers (`set_button_menu`, docs/menus.md "Menu buttons"), keyed
        /// like `CONTEXT_MENU_FNS` by the button's global ref, so a button may carry both.
        static BUTTON_MENU_FNS: day_spec::sidetable::SideTable<day_spec::ContextMenuFn> = day_spec::sidetable::SideTable::new();
        static TRANSFER_SOURCES: day_spec::sidetable::SideTable<day_spec::transfer::Source> = day_spec::sidetable::SideTable::new();
        static TRANSFER_TARGETS: day_spec::sidetable::SideTable<day_spec::transfer::Target> = day_spec::sidetable::SideTable::new();

        /// Recycling list (docs/list.md): row-pull sources keyed by list node id (Java passes it
        /// back in nativeListBind), and a stable GlobalRef per physical cell so day-core's cell
        /// map keys consistently across ListView recycling. Cells are grouped by list so that
        /// releasing a list frees its cells' JNI global refs with it; a long session would
        /// otherwise leak one global ref per physical cell toward the JNI table limit. Within a
        /// list, cells key by `identityHashCode`, the only stable identity the wire carries
        /// (nativeListBind sends `(hostId, position, cell)` and nothing else). Collisions are
        /// possible in principle; one would conflate two physical cells of the same list, and
        /// carrying a bridge-assigned cell id would widen the wire for a case never observed.
        static LIST_SOURCES: std::cell::RefCell<std::collections::HashMap<i64, ListSource>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        /// Latest logical row of a physical holder; invalidates deferred binds on recycle.
        static LIST_BINDINGS: std::cell::RefCell<std::collections::HashMap<(i64, i32), (usize, u64)>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        static LIST_NODE: std::cell::RefCell<std::collections::HashMap<usize, i64>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        /// A realized NAV_MENU's rows, by its own view ptr: `(node, joined titles, joined icons,
        /// joined document keys, joined badge texts)`.
        ///
        /// Recorded at realize, which is where the props are, and handed to the navigation suite
        /// at insert, the first moment the menu is in a hierarchy whose ancestors reach the
        /// suite. Where there is no suite above it (every presentation but `Tabs`) the handover
        /// finds nothing and the rows stay a list. Entries drop in `release`.
        /// Each realized list item's props, by its view ptr: a patch changes one part, and the
        /// Java view takes its three texts together (docs/list.md "List items").
        static LIST_ITEM_PROPS: std::cell::RefCell<
            std::collections::HashMap<usize, day_spec::props::ListItemProps>,
        > = std::cell::RefCell::new(std::collections::HashMap::new());
        static NAV_MENU_ROWS: std::cell::RefCell<
            std::collections::HashMap<usize, (i64, String, String, String, String)>,
        > = std::cell::RefCell::new(std::collections::HashMap::new());
        /// Each window's toolbar model (docs/toolbars.md), by its root view ptr: what the app-bar
        /// menu is repainted from.
        static WINDOW_BARS: std::cell::RefCell<
            std::collections::HashMap<usize, day_spec::ToolbarMirror>,
        > = std::cell::RefCell::new(std::collections::HashMap::new());
        /// Navigation-suite hosts by view ptr, so `NavPatch::Select` knows to drive the suite
        /// rather than a DayNavHost.
        static NAV_SUITES: std::cell::RefCell<std::collections::HashSet<usize>> =
            std::cell::RefCell::new(std::collections::HashSet::new());
        /// Label view ptr → node id, so a `LabelPatch::Runs` (which carries no id) can still tell
        /// a link's ClickableSpan which node to report against. Entries drop in `release`.
        static LABEL_NODE: std::cell::RefCell<std::collections::HashMap<usize, i64>> =
            std::cell::RefCell::new(std::collections::HashMap::new());
        #[allow(clippy::type_complexity)] // host id → (cell identityHashCode → its GlobalRef)
        static LIST_CELLS: std::cell::RefCell<
            std::collections::HashMap<i64, std::collections::HashMap<i32, Gref>>,
        > = std::cell::RefCell::new(std::collections::HashMap::new());
        /// The lists whose rows size themselves (`RowHeight::Automatic`), by node id: each bind
        /// hands the native row the height day-core laid it at.
        static LIST_AUTOMATIC: std::cell::RefCell<std::collections::HashSet<i64>> =
            std::cell::RefCell::new(std::collections::HashSet::new());
        /// Programmatic selection per list (docs/list.md `ListPatch::Selected`): the Java side
        /// paints from this at bind (nativeListIsSelected) and on a sync
        /// (listPaintSelection walking the visible holders).
        static LIST_SELECTED: std::cell::RefCell<
            std::collections::HashMap<i64, std::collections::BTreeSet<usize>>,
        > = std::cell::RefCell::new(std::collections::HashMap::new());

        static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
        static DENSITY: Cell<f64> = const { Cell::new(1.0) };
        static ROOT: RefCell<Option<(AHandle, Size)>> = const { RefCell::new(None) };

        /// Secondary DayWindowActivity roots (docs/windows.md): (day node, the root's
        /// global ref, kept alive alongside the tree's adopted ref).
        static SECONDARY: RefCell<Vec<(u64, Gref)>> = const { RefCell::new(Vec::new()) };

    }

    type SectionTreeEntry = (day_spec::TreeSource, Vec<u64>, Vec<u64>);
    thread_local! {
        static TREE_SOURCES: RefCell<std::collections::HashMap<i64, SectionTreeEntry>> = RefCell::new(std::collections::HashMap::new());
    }
    fn tree_reload(host: &AHandle) {
        use std::fmt::Write as _;
        let handle = host.clone();
        let id = LIST_NODE.with(|m| {
            m.borrow()
                .get(&(host.0.as_obj().as_raw() as usize))
                .copied()
        });
        let Some(id) = id else {
            return;
        };
        Material::post(Box::new(move || {
            let source = TREE_SOURCES.with(|m| m.borrow().get(&id).map(|e| e.0.clone()));
            let Some(source) = source else {
                return;
            };
            let mut tokens = Vec::new();
            let mut data = String::new();
            for g in 0..(source.children_len)(None) {
                let token = (source.child_token)(None, g);
                let index = tokens.len();
                tokens.push(token);
                let _ = writeln!(
                    data,
                    "{token}:-1:{}:{}",
                    u8::from((source.section_header)(token)),
                    u8::from((source.expanded)(token))
                );
                for c in 0..(source.children_len)(Some(token)) {
                    let child = (source.child_token)(Some(token), c);
                    tokens.push(child);
                    let _ = writeln!(data, "{child}:{index}:0:0");
                }
            }
            TREE_SOURCES.with(|m| {
                if let Some(e) = m.borrow_mut().get_mut(&id) {
                    e.1 = tokens;
                    let selected =
                        e.1.iter()
                            .enumerate()
                            .filter(|(_, t)| e.2.contains(t))
                            .map(|(i, _)| i)
                            .collect();
                    LIST_SELECTED.with(|m| m.borrow_mut().insert(id, selected));
                }
            });
            with_env(|env| {
                let data = jstr(env, &data);
                let _ = java_in(env, |env| {
                    DayBridge::treeReload(env, handle.0.as_obj(), &data)
                });
            });
        }));
    }
    fn tree_patch(host: &AHandle, patch: &TreePatch) {
        match patch {
            TreePatch::Reload => tree_reload(host),
            TreePatch::Expand(token, open) => {
                // ExpandableListView synchronously recycles children on collapse. Run
                // outside the core update borrow so their Day IDs can actually be cleared.
                let (host, token, open) = (host.clone(), *token, *open);
                Material::post(Box::new(move || {
                    bridge(|env| DayBridge::treeExpand(env, host.0.as_obj(), token as i64, open));
                }));
            }
            TreePatch::Reveal(token) => {
                bridge(|env| DayBridge::treeReveal(env, host.0.as_obj(), *token as i64));
            }
            TreePatch::Selected(tokens) => {
                if let Some(id) = LIST_NODE.with(|m| {
                    m.borrow()
                        .get(&(host.0.as_obj().as_raw() as usize))
                        .copied()
                }) {
                    let selected = TREE_SOURCES.with(|m| {
                        m.borrow_mut().get_mut(&id).map_or_default(|e| {
                            e.2.clone_from(tokens);
                            e.1.iter()
                                .enumerate()
                                .filter(|(_, t)| tokens.contains(t))
                                .map(|(i, _)| i)
                                .collect()
                        })
                    });
                    LIST_SELECTED.with(|m| m.borrow_mut().insert(id, selected));
                }
                bridge(|env| DayBridge::treeSelection(env, host.0.as_obj()));
            }
        }
    }

    /// Row count, pulled by the Java adapter's getCount (reads the snapshot only; no tree).
    /// A JNI up-call entry: the body is contained, since a panic unwinding the frame would abort.
    #[must_use]
    pub fn list_len(host_id: i64) -> usize {
        day_spec::ffi_guard::contain(0, || {
            LIST_SOURCES.with(|m| m.borrow().get(&host_id).map_or(0, |s| (s.len)()))
        })
    }

    /// Give a self-sizing list's native row the height day-core laid its row at (docs/list.md
    /// "Row heights"). Called right after `bind_row`, inside `RecyclerView`'s own bind, which is
    /// where a row's layout params belong: the layout manager measures the holder next. A
    /// uniform list's rows keep the height their holders were made with, except a section
    /// header and a supplementary row (the list's header, footer, empty state, padding), which
    /// size to their content in every list (docs/list.md "Sections", "Header, footer and empty
    /// state"); both pool apart from items, so the height never reaches an item's holder.
    fn size_list_cell(
        host_id: i64,
        source: &ListSource,
        index: usize,
        cell: &JObject,
        raw: RawHandle,
    ) {
        let header = source.is_header.as_ref().is_some_and(|f| f(index))
            || source.is_supplement.as_ref().is_some_and(|f| f(index));
        if !header && !LIST_AUTOMATIC.with(|m| m.borrow().contains(&host_id)) {
            return;
        }
        let height = (source.cell_height)(raw);
        if height > 0.0 {
            set_list_cell_height(cell, height);
        }
    }

    /// Set one native row's height, in points (`DayBridge.setListCellHeight`).
    fn set_list_cell_height(cell: &JObject, height: f64) {
        let px = (height * DENSITY.with(std::cell::Cell::get)).round() as i32;
        bridge(|env| DayBridge::setListCellHeight(env, cell, px));
    }

    /// Fill a recycled cell; the Java adapter's getView calls this. A stable `GlobalRef` per
    /// physical cell (keyed by identityHashCode under its list) gives day-core a consistent
    /// cell key. A JNI up-call entry running app row builders: contained like `list_len`.
    /// `pinned`: the cell is the list's pinned header copy, bound through
    /// `ListSource::bind_pinned` (docs/list.md "Sections").
    #[expect(
        clippy::needless_pass_by_value,
        reason = "public signature; the JNI entry hands over the cell's local reference"
    )]
    pub fn list_bind(env: &mut Env, host_id: i64, position: i32, cell: JObject, pinned: bool) {
        day_spec::ffi_guard::contain((), || {
            let hash = framework::System::identityHashCode(env, &cell).unwrap_or(0);
            let cached = LIST_CELLS.with(|m| {
                m.borrow()
                    .get(&host_id)
                    .and_then(|cells| cells.get(&hash).cloned())
            });
            let gref = if let Some(g) = cached {
                g
            } else {
                // A failed global ref (JNI table exhausted) skips this bind (the next
                // bind retries) rather than panicking out of the up-call.
                let Ok(g) = env.new_global_ref(&cell) else {
                    return;
                };
                let g = std::sync::Arc::new(g);
                LIST_CELLS.with(|m| {
                    m.borrow_mut()
                        .entry(host_id)
                        .or_default()
                        .insert(hash, g.clone())
                });
                g
            };
            let raw = gref.as_obj().as_raw() as RawHandle;
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            if let Some(source) = source {
                let index = position as usize;
                if index >= (source.len)() {
                    return;
                }
                let token = (source.token_at)(index);
                LIST_BINDINGS.with(|m| m.borrow_mut().insert((host_id, hash), (index, token)));
                let bind = move |source: &ListSource| {
                    if pinned {
                        source.bind_pinned.clone()
                    } else {
                        source.bind_row.clone()
                    }
                };
                if day_core::try_with_tree(|_| ()).is_some() {
                    bind(&source)(index, raw);
                    size_list_cell(host_id, &source, index, &cell, raw);
                } else {
                    // DayFixed.measure can make RecyclerView bind during Day's layout borrow.
                    // Core refuses reentry, but RecyclerView considers that empty holder bound
                    // and need not ask again. Retain the JNI cell and retry on the next UI turn,
                    // rejecting a stale row or disposed list before touching it.
                    <Material as Platform>::post(Box::new(move || {
                        if !LIST_BINDINGS
                            .with(|m| m.borrow().get(&(host_id, hash)) == Some(&(index, token)))
                        {
                            return;
                        }
                        let current = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
                        if let Some(current) = current
                            && index < (current.len)()
                            && (current.token_at)(index) == token
                        {
                            let raw = gref.as_obj().as_raw() as RawHandle;
                            bind(&current)(index, raw);
                            size_list_cell(host_id, &current, index, gref.as_obj(), raw);
                        }
                    }));
                }
            } else {
                let entry = TREE_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
                if let Some((source, tokens, _)) = entry
                    && let Some(token) = tokens.get(position as usize)
                {
                    (source.bind_row)(*token, raw);
                    // SAFETY: a list cell is the DayFixed View the adapter created.
                    let width = unsafe { env.as_cast_unchecked::<framework::View>(&cell) }
                        .getWidth(env)
                        .unwrap_or(0);
                    if width > 0 {
                        (source.layout_cell)(
                            raw,
                            f64::from(width) / DENSITY.with(std::cell::Cell::get),
                        );
                    }
                }
            }
        });
    }

    /// A holder left the visible set (`RecyclerView`'s onViewRecycled): clear the cell
    /// subtree's dayscript ids so pooled rows stop answering lookups, through day-core's
    /// `list_recycle_cell`, keyed by the same per-cell `GlobalRef` `list_bind` binds with.
    /// A JNI up-call entry: contained.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "public signature; the JNI entry hands over the cell's local reference"
    )]
    pub fn list_recycle(env: &mut Env, host_id: i64, cell: JObject) {
        day_spec::ffi_guard::contain((), || {
            let hash = framework::System::identityHashCode(env, &cell).unwrap_or(0);
            LIST_BINDINGS.with(|m| m.borrow_mut().remove(&(host_id, hash)));
            let gref = LIST_CELLS.with(|m| {
                m.borrow()
                    .get(&host_id)
                    .and_then(|cells| cells.get(&hash).cloned())
            });
            let Some(gref) = gref else { return };
            if day_core::try_with_tree(|_| ()).is_some() {
                recycle_cell(host_id, &gref);
            } else {
                // Inside Day's borrow: a reload or splice the tree is applying, which makes
                // RecyclerView recycle its cached holders (a row it prefetched while scrolling)
                // on the spot. `ListSource::recycle` would drop the park there and leave the
                // row's ids answering; park on the next turn instead, unless the cell was bound
                // again in between.
                <Material as Platform>::post(Box::new(move || {
                    if !LIST_BINDINGS.with(|m| m.borrow().contains_key(&(host_id, hash))) {
                        recycle_cell(host_id, &gref);
                    }
                }));
            }
        });
    }

    /// Hand a cell that left the visible set to its list's (or tree's) source.
    fn recycle_cell(host_id: i64, gref: &Gref) {
        let raw = gref.as_obj().as_raw() as RawHandle;
        let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
        if let Some(source) = source {
            (source.recycle)(raw);
        } else if let Some(source) =
            TREE_SOURCES.with(|m| m.borrow().get(&host_id).map(|e| e.0.clone()))
        {
            (source.recycle)(raw);
        }
    }

    /// A sectioned list is going away: hand day-core its pinned header cell for good
    /// (`ListSource::release`), the one cell `RecyclerView` never pooled (docs/list.md
    /// "Sections"). The cell keys by the `GlobalRef` `list_bind` made for it.
    fn release_pinned_header(list: &AHandle, nid: i64) {
        let Some(source) = LIST_SOURCES.with(|m| m.borrow().get(&nid).cloned()) else {
            return;
        };
        if source.is_header.is_none() {
            return;
        }
        let gref = with_env(|env| {
            let Ok(cell) = DayBridge::listPinnedCell(env, list.0.as_obj()) else {
                if env.exception_check() {
                    env.exception_clear();
                }
                return None;
            };
            if cell.is_null() {
                return None;
            }
            let hash = framework::System::identityHashCode(env, &cell).ok()?;
            LIST_CELLS.with(|m| m.borrow().get(&nid).and_then(|c| c.get(&hash).cloned()))
        });
        if let Some(gref) = gref {
            (source.release)(gref.as_obj().as_raw() as RawHandle);
        }
    }

    /// Whether row `position` is in the list's programmatic selection; the Java bind path
    /// paints newly bound holders from this. A JNI up-call entry: contained.
    #[must_use]
    pub fn list_is_selected(host_id: i64, position: i32) -> bool {
        day_spec::ffi_guard::contain(false, || {
            LIST_SELECTED.with(|m| {
                m.borrow()
                    .get(&host_id)
                    .is_some_and(|set| set.contains(&(position.max(0) as usize)))
            })
        })
    }

    /// The adapter's view type for row `position` (`getItemViewType`): 1 for a section header
    /// (docs/list.md "Sections"), 2 for a supplementary row (the list's header, footer, empty
    /// state or content padding, docs/list.md "Header, footer and empty state"), 0 for an
    /// item. Each pools apart, so a recycled cell never carries one kind's height or chrome
    /// to another. A JNI up-call entry: contained.
    #[must_use]
    pub fn list_row_kind(host_id: i64, position: i32) -> i32 {
        day_spec::ffi_guard::contain(0, || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(s) = source else { return 0 };
            let index = position.max(0) as usize;
            if index >= (s.len)() {
                return 0;
            }
            if s.is_supplement.as_ref().is_some_and(|f| f(index)) {
                return 2;
            }
            i32::from(s.is_header.as_ref().is_some_and(|f| f(index)))
        })
    }

    /// The list's viewport report from `DayListView.reportViewport` (docs/list.md "Visible
    /// rows"), passed to `ListSource::viewport`, which only enqueues. A JNI up-call entry:
    /// contained.
    pub fn list_viewport(host_id: i64, first: i32, last: i32, end_px: i32) {
        day_spec::ffi_guard::contain((), || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(source) = source else { return };
            let Some(report) = &source.viewport else {
                return;
            };
            if let Some(v) = crate::list_wire::viewport(
                first,
                last,
                end_px,
                DENSITY.with(std::cell::Cell::get),
                (source.len)(),
            ) {
                report(v);
            }
        });
    }

    /// The reader moved the list (`DayListView.onScrolled`, docs/list.md "Following the end"),
    /// passed to `ListSource::reader_scroll`, which only enqueues. A JNI up-call entry:
    /// contained.
    pub fn list_reader_scroll(host_id: i64) {
        day_spec::ffi_guard::contain((), || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            if let Some(report) = source.as_ref().and_then(|s| s.reader_scroll.as_ref()) {
                report();
            }
        });
    }

    /// The display row of the header that opens the section holding row `position`, or -1:
    /// which header the pinned copy shows (docs/list.md "Sections"). Walks back from the row;
    /// row 0 of a sectioned list is always a header. A JNI up-call entry: contained.
    #[must_use]
    pub fn list_header_of(host_id: i64, position: i32) -> i32 {
        day_spec::ffi_guard::contain(-1, || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(source) = source else { return -1 };
            let Some(is_header) = source.is_header.as_ref() else {
                return -1;
            };
            let len = (source.len)();
            if position < 0 || len == 0 {
                return -1;
            }
            let start = (position as usize).min(len - 1);
            // The list's own header, footer or padding at the top: no section is pinned.
            if source.is_supplement.as_ref().is_some_and(|f| f(start)) {
                return -1;
            }
            (0..=start)
                .rev()
                .find(|&i| is_header(i))
                .map_or(-1, |i| i32::try_from(i).unwrap_or(-1))
        })
    }

    /// One drag-and-drop up-call from `DayTransfer` (docs/drag-and-drop.md): `kind` 0 asks the
    /// source `token` for its encoded offer, 1 asks the target `token` whether it accepts the
    /// types listed in `bytes`, 2 delivers the offer encoded in `bytes`, and 3 tells the target
    /// the session left it or ended. Answers the reply bytes for Java.
    ///
    /// # Errors
    ///
    /// Returns the JNI error when `bytes` cannot be read or the reply array cannot be made.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "public signature; the JNI entry hands over the array's local reference"
    )]
    pub fn transfer_call<'l>(
        env: &mut Env<'l>,
        kind: i32,
        token: i64,
        x: f64,
        y: f64,
        bytes: jni::objects::JByteArray,
        local: bool,
    ) -> Result<jni::objects::JByteArray<'l>, jni::errors::Error> {
        use day_spec::transfer::*;
        let input = env.convert_byte_array(&bytes)?;
        let output = day_spec::ffi_guard::contain(Vec::new(), || match kind {
            0 => TRANSFER_SOURCES
                .with(|t| t.get(token as usize))
                .and_then(|f| f(Point::new(x, y)))
                .and_then(|o| o.encode())
                .unwrap_or_default(),
            1 => {
                let at = Location {
                    position: Point::new(x, y),
                    types: String::from_utf8_lossy(&input)
                        .split('\n')
                        .map(str::to_owned)
                        .collect(),
                    allowed: vec![Operation::Copy],
                    local,
                };
                vec![u8::from(
                    TRANSFER_TARGETS
                        .with(|t| t.get(token as usize))
                        .is_some_and(|t| t.proposal(&at) == Operation::Copy),
                )]
            }
            2 => {
                let ok = Offer::decode(&input).is_some_and(|offer| {
                    TRANSFER_TARGETS
                        .with(|t| t.get(token as usize))
                        .is_some_and(|t| {
                            t.deliver(
                                Location {
                                    position: Point::new(x, y),
                                    types: offer.types(),
                                    allowed: vec![Operation::Copy],
                                    local,
                                },
                                offer,
                            )
                        })
                });
                vec![u8::from(ok)]
            }
            // The session left the target without dropping, or ended: the hover is over.
            3 => {
                if let Some(t) = TRANSFER_TARGETS.with(|t| t.get(token as usize)) {
                    t.leave();
                }
                Vec::new()
            }
            _ => Vec::new(),
        });
        env.byte_array_from_slice(&output)
    }
    /// Make `h` a drag source (`DayTransfer.source`) or a drop target (`DayTransfer.target`),
    /// keyed by its view pointer.
    fn transfer_install(h: &AHandle, source: bool) {
        let view = h.0.as_obj();
        let token = view.as_raw() as i64;
        bridge(|env| {
            if source {
                DayTransfer::source(env, view, token)
            } else {
                DayTransfer::target(env, view, token)
            }
        });
    }

    /// The reorder guard's verdict for a hovered drop (docs/list.md), pulled synchronously by
    /// `ItemTouchHelper`'s canDropOver. `ItemTouchHelper` cannot relocate the gap, so a Retarget
    /// verdict (accepted != proposed) reads as a deny for that hover. The source is cloned out
    /// before the guard runs, so no thread-local borrow is held.
    #[must_use]
    pub fn list_can_drop(host_id: i64, from: i32, to: i32) -> bool {
        // JNI up-call entry (ItemTouchHelper's canDropOver): contain the app's guard.
        day_spec::ffi_guard::contain(false, || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(source) = source else { return false };
            let Some(r) = source.reorder.as_ref() else {
                return false;
            };
            let len = (source.len)();
            let (from, to) = (from.max(0) as usize, to.max(0) as usize);
            if from >= len || to >= len {
                return false;
            }
            (r.can_move)(from, to) == to as i64
        })
    }

    /// Commit one incremental `ItemTouchHelper` swap through the source's synchronous `move_row`
    /// (rotates day's snapshot, defers the app callback). Returns whether the swap was accepted.
    #[must_use]
    pub fn list_move(host_id: i64, from: i32, to: i32) -> bool {
        // JNI up-call entry: contain the synchronous commit.
        day_spec::ffi_guard::contain(false, || {
            if !list_can_drop(host_id, from, to) {
                return false;
            }
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(r) = source.and_then(|s| s.reorder) else {
                return false;
            };
            let (from, to) = (from as usize, to as usize);
            if from != to {
                (r.move_row)(from, to);
            }
            true
        })
    }

    /// May this row be swiped away? Consulted from `getMovementFlags`, so a protected row
    /// reports no swipe direction and never moves under the finger (docs/list.md).
    #[must_use]
    pub fn list_can_delete(host_id: i64, index: i32) -> bool {
        // JNI up-call entry (getMovementFlags): contain the app's guard.
        day_spec::ffi_guard::contain(false, || {
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(source) = source else { return false };
            let Some(d) = source.delete.as_ref() else {
                return false;
            };
            let index = index.max(0) as usize;
            index < (source.len)() && (d.can_delete)(index)
        })
    }

    /// Commit a swipe-to-delete through the source's synchronous `delete_row` (shortens day's
    /// snapshot, defers the app callback). Returns whether the delete was accepted.
    #[must_use]
    pub fn list_delete(host_id: i64, index: i32) -> bool {
        // JNI up-call entry: contain the synchronous commit.
        day_spec::ffi_guard::contain(false, || {
            if !list_can_delete(host_id, index) {
                return false;
            }
            let source = LIST_SOURCES.with(|m| m.borrow().get(&host_id).cloned());
            let Some(d) = source.and_then(|s| s.delete) else {
                return false;
            };
            (d.delete_row)(index as usize);
            true
        })
    }

    #[unsafe(no_mangle)]
    pub extern "system" fn Java_dev_daybrite_day_bridge_DayBridge_nativePopoverDismissed(
        _env: *mut std::ffi::c_void,
        _class: *mut std::ffi::c_void,
        id: i64,
    ) {
        day_spec::ffi_guard::contain((), || emit(NodeId(id as u64), Event::PopoverDismissed));
    }

    pub const BRIDGE: &str = "dev/daybrite/day/bridge/DayBridge";

    // DayBridge (java/dev/daybrite/day/bridge/DayBridge.java): every static method Rust calls,
    // named exactly as Java names it (docs/android-jni.md). `init` resolves the whole class, so a
    // method renamed or retyped on the Java side stops a debug or scripted launch by name.
    crate::java_class! {
        pub DayBridge => dev.daybrite.day.bridge.DayBridge,
        methods {
        static fn addChild(parent: android.view.View, child: android.view.View),
        static fn setTooltip(v: android.view.View, text: JString),
        static fn openPopover(anchor: android.view.View, content: android.view.View, w: jint, h: jint, id: jlong) -> jboolean,
        static fn resizePopover(content: android.view.View, w: jint, h: jint),
        static fn closePopover(content: android.view.View),
        static fn showToast(host: android.view.View, text: JString),
        static fn announce(text: JString, urgent: jboolean),
        static fn baselineAt(v: android.view.View, w_px: jint, h_px: jint) -> jint,
        static fn cancelFrame(token: jlong),
        static fn canSetAppearance() -> jboolean,
        static fn canSetLauncherShortcuts() -> jboolean,
        static fn closeWindow(node: jlong),
        static fn coverDismiss(cover: android.view.View),
        static fn coverPresent(cover: android.view.View, bg: jint, has_bg: jboolean, dismiss_disabled: jboolean),
        static fn coverSetDismissDisabled(cover: android.view.View, d: jboolean),
        static fn dismissPresent(req: jlong),
        static fn displayInfo() -> JString,
        static fn documentNavChrome(suite: android.view.View, visible: jboolean),
        static fn documentNavSuite(suite: android.view.View, new_label: JString, close_label: JString, can_add: jboolean),
        static fn enableGesture(v: android.view.View, id: jlong, is_drag: jboolean),
        static fn enableRecognizer(v: android.view.View, id: jlong, kind: jint),
        static fn enableHover(v: android.view.View, id: jlong),
        static fn enableKeys(v: android.view.View, id: jlong),
        static fn enablePan(v: android.view.View, id: jlong),
        static fn enablePinch(v: android.view.View, id: jlong),
        static fn enableRotate(v: android.view.View, id: jlong),
        static fn focusNavSearch(nav_host: android.view.View),
        static fn focusView(v: android.view.View, focused: jboolean),
        static fn focusWindow(node: jlong),
        static fn fontFamilies() -> JString,
        static fn imageDecode(id: jlong, bytes: jbyte[]) -> JString,
        static fn imageEncode(id: jlong, format: JString, quality: jint, fit_w: jdouble, fit_h: jdouble) -> jbyte[],
        static fn imageRelease(id: jlong),
        static fn isDarkMode() -> jboolean,
        static fn systemColor(role: jint) -> jint,
        static fn listPaintSelection(v: android.view.View, host_id: jlong),
        static fn listReload(v: android.view.View),
        static fn listConfigure(v: android.view.View, viewport: jboolean, reader: jboolean, supplements: jboolean),
        static fn listSplice(v: android.view.View, ops: jint[]),
        static fn listScroll(v: android.view.View, row: jint, align: jint, animated: jboolean),
        static fn listScrollToEnd(v: android.view.View),
        static fn listScrollToRow(v: android.view.View, row: jint),
        static fn listSetSections(v: android.view.View, on: jboolean),
        static fn listPinnedCell(v: android.view.View) -> android.view.View,
        static fn localeTags() -> JString,
        static fn makeButton(id: jlong, title: JString) -> android.view.View,
        static fn makeCanvas(id: jlong) -> android.view.View,
        static fn makeContainer() -> android.view.View,
        static fn makeCover(node: jlong, half: jboolean) -> android.view.View,
        static fn makeDivider(vertical: jboolean) -> android.view.View,
        static fn makeImage(name: JString, mode: jint, tint: jint) -> android.view.View,
        static fn makeImageBitmap(id: jlong, mode: jint) -> android.view.View,
        static fn makeImageBytes(bytes: jbyte[], mode: jint) -> android.view.View,
        static fn makeLabel(text: JString) -> android.view.View,
        static fn makeList(host_id: jlong, row_height_px: jint, selectable: jboolean, reorderable: jboolean, deletable: jboolean, delete_label: JString) -> android.view.View,
        static fn makeNavHost(id: jlong, title: JString, adaptive: jboolean, tile_min_dp: jfloat) -> android.view.View,
        static fn makeNavMenu(id: jlong, joined_items: JString, joined_icons: JString, joined_sections: JString) -> android.view.View,
        static fn makeNavPage(id: jlong, immersive: jboolean) -> android.view.View,
        static fn makeNavSuite(id: jlong, initial: jint) -> android.view.View,
        static fn makeProgress(determinate: jboolean, fraction: jdouble) -> android.view.View,
        static fn makeScroll(horizontal: jboolean, flags: jint, snap_px: jfloat, node: jlong) -> android.view.View,
        static fn makeSlider(id: jlong, value: jdouble, min: jdouble, max: jdouble) -> android.view.View,
        static fn makeTextField(id: jlong, value: JString, placeholder: JString) -> android.view.View,
        static fn makeToggle(id: jlong, value: jboolean, enabled: jboolean) -> android.view.View,
        static fn makeTree(id: jlong, height: jint, selectable: jboolean) -> android.view.View,
        static fn markNavChromePage(page: android.view.View),
        static fn measureHeight(v: android.view.View) -> jint,
        static fn measureHeightForWidth(v: android.view.View, w_px: jint) -> jint,
        static fn measureText(text: JString, size: jdouble, weight: jint, italic: jboolean, family: JString) -> JString,
        static fn measureWidth(v: android.view.View) -> jint,
        static fn mountSettled(),
        static fn moveChild(parent: android.view.View, child: android.view.View, index: jint),
        static fn nativeBack() -> jboolean,
        static fn navLoad(key: JString) -> JString,
        static fn navPop(host: android.view.View),
        static fn navPush(host: android.view.View, title: JString, immersive: jboolean),
        static fn navSave(key: JString, value: JString),
        static fn navSetGuard(host: android.view.View, on: jboolean),
        static fn navSetTitle(host: android.view.View, title: JString),
        static fn navTake(key: JString) -> JString,
        static fn openUrl(url: JString),
        static fn openWindow(node: jlong, title: JString),
        static fn prepareSnapshot(host: android.view.View, revision: jlong) -> jint,
        static fn present(req: jlong, sheet: jboolean, title: JString, message: JString, buttons_joined: JString, roles_joined: JString),
        static fn presentFileOpen(req: jlong, title: JString, filters_joined: JString, multiple: jboolean),
        static fn presentFileSave(req: jlong, title: JString, suggested: JString, src_path: JString, filters_joined: JString),
        static fn presentPrompt(req: jlong, title: JString, message: JString, placeholder: JString, initial: JString, ok: JString, cancel: JString),
        static fn readNative(v: android.view.View) -> JString[],
        static fn reduceMotion() -> jboolean,
        static fn removeChild(child: android.view.View),
        static fn reorderNavSuite(suite: android.view.View, order: JString),
        static fn requestFrame(token: jlong),
        static fn scrollToRect(v: android.view.View, x: jint, y: jint, w: jint, h: jint, animated: jboolean),
        static fn navScrollToEdge(v: android.view.View, bottom: jboolean) -> jboolean,
        static fn setA11y(v: android.view.View, label: JString, hint: JString, value: JString, id: JString, role: jint, level: jint, hidden: jboolean, selected: jint),
        static fn setAppBack(on: jboolean),
        static fn setAppearance(mode: jint),
        static fn setAppMenu(spec: JString),
        static fn setButtonContent(view: android.view.View, title: JString, icon_name: JString, icon_only: jboolean),
        static fn setButtonSelected(v: android.view.View, on: jboolean),
        static fn setButtonStyle(v: android.view.View, kind: jint, argb: jint, fg_argb: jint),
        static fn setCanvasOps(v: android.view.View, nums: jdouble[], texts_joined: JString,
            geometry: jdouble[]),
        static fn setContextMenu(v: android.view.View, spec: JString),
        static fn setDragAxis(v: android.view.View, axis: jint),
        static fn setHitSlop(v: android.view.View, top: jint, leading: jint, bottom: jint, trailing: jint),
        static fn setSubtreeEnabled(root: android.view.View, enabled: jboolean),
        static fn primaryPointer() -> JString,
        static fn setContextMenuFn(v: android.view.View, token: jlong),
        static fn setButtonMenu(v: android.view.View, token: jlong),
        static fn releaseContextMenu(v: android.view.View),
        static fn setDeferSystemGestures(on: jboolean),
        static fn setDocumentTabKeys(menu: android.view.View, keys: JString),
        static fn setEnabled(v: android.view.View, b: jboolean),
        static fn setFrame(v: android.view.View, x: jint, y: jint, w: jint, h: jint, timing: jint[], keys: jfloat[]),
        static fn setViewDirection(v: android.view.View, rtl: jboolean),
        static fn textScale(sp: jfloat) -> jfloat,
        static fn setImageBitmap(v: android.view.View, id: jlong),
        static fn setImageBytes(v: android.view.View, bytes: jbyte[]),
        static fn setImageName(v: android.view.View, name: JString),
        static fn setImageTint(v: android.view.View, tint: jint),
        static fn initImageTint(v: android.view.View, tint: jint),
        static fn setInputTraits(v: android.view.View, secure: jboolean, read_only: jboolean, purpose: jint, submit_label: jint),
        static fn setLabel(v: android.view.View, text: JString),
        static fn setLabelAlignment(view: android.view.View, align: jint),
        static fn setLabelColor(v: android.view.View, argb: jint, on: jboolean),
        static fn setLabelFont(v: android.view.View, sp: jfloat, weight: jint, italic: jboolean, family: JString, tabular: jboolean),
        static fn setLabelRuns(v: android.view.View, node: jlong, text: JString, starts: jint[], ends: jint[], flags: jint[], colors: jint[], backgrounds: jint[], scales: jint[], links_joined: JString, decorations: jint[]),
        static fn setLabelSecondary(v: android.view.View, on: jboolean),
        static fn setLabelSingleLine(view: android.view.View),
        static fn setLabelParagraph(view: android.view.View, max_lines: jint, truncation: jint, letter_spacing: jfloat, line_height: jfloat),
        static fn setLabelReportsSelection(view: android.view.View, node: jlong),
        static fn setLabelFeatures(view: android.view.View, features: JString),
        static fn setLauncherShortcuts(packed: JString),
        static fn setListCellHeight(cell: android.view.View, px: jint),
        static fn setNavMenuBadges(nav_menu: android.view.View, joined_icons: JString, joined_tints: JString, joined_texts: JString),
        static fn setNavMenuSelected(nav_menu: android.view.View, index: jint),
        static fn setNavMenuTints(nav_menu: android.view.View, joined_tints: JString),
        static fn setNavRefreshing(view: android.view.View, busy: jboolean),
        static fn setNavRowMenus(nav_menu: android.view.View, joined_specs: JString),
        static fn setNavSearch(nav_host: android.view.View, id: jlong, prompt: JString, text: JString, hides_on_scroll: jboolean, initially_hidden: jboolean),
        static fn setNavSearchText(nav_host: android.view.View, text: JString),
        static fn setNavSuiteRows(menu: android.view.View, titles: JString, icons: JString, badges: JString, menu_node: jlong),
        static fn updateNavMenuRows(v: android.view.View, joined_rows: JString, joined_items: JString, joined_icons: JString),
        static fn setNavSuiteSelected(suite: android.view.View, index: jint),
        static fn setOpacity(v: android.view.View, alpha: jfloat, timing: jint[], keys: jfloat[]),
        static fn setPlaceholder(v: android.view.View, value: JString),
        static fn setPrimaryWindowTitle(title: JString),
        static fn setProgress(v: android.view.View, fraction: jdouble),
        static fn setScrollContent(v: android.view.View, w: jint, h: jint),
        static fn setSectionCard(v: android.view.View, radius_px: jfloat),
        static fn setSlider(v: android.view.View, value: jdouble, ignored_min: jdouble),
        static fn setStatusBarHidden(hidden: jboolean),
        static fn setSurface(v: android.view.View, argb: jint, has_bg: jboolean, radius_px: jfloat, clips: jboolean),
        static fn setSurfaceFill(v: android.view.View, argb: jint, has_bg: jboolean),
        static fn animateSurfaceFill(v: android.view.View, argb: jint, has_bg: jboolean, timing: jint[], keys: jfloat[]),
        static fn setSurfaceStyle(v: android.view.View, spec: jdouble[], clips: jboolean),
        static fn setSurfaceGradient(v: android.view.View, spec: jdouble[]),
        static fn resetSurface(v: android.view.View),
        static fn setTextField(v: android.view.View, value: JString),
        static fn setTextFieldSelection(v: android.view.View, start: jint, end: jint),
        static fn setToggle(v: android.view.View, value: jboolean),
        static fn setTransform(v: android.view.View, tx: jfloat, ty: jfloat, sx: jfloat, sy: jfloat, rot: jfloat, timing: jint[], keys: jfloat[]),
        static fn readDisplayed(v: android.view.View) -> jdouble[],
        static fn setWindowSecure(node: jlong, on: jboolean),
        static fn setWindowState(node: jlong, code: jint),
        static fn setWindowSurface(v: android.view.View, radius_px: jfloat, clips: jboolean),
        static fn setWindowTitle(node: jlong, title: JString),
        static fn setWindowToolbar(root: android.view.View, spec: JString),
        static fn share(url: JString, title: JString, actions: JString) -> jboolean,
        static fn snapshotOrigin(root: android.view.View) -> JString,
        static fn treeExpand(view: android.view.View, token: jlong, open: jboolean),
        static fn treeReload(view: android.view.View, data: JString),
        static fn treeReveal(view: android.view.View, token: jlong),
        static fn treeSelection(view: android.view.View),
        static fn uiIdle() -> jboolean,
        static fn updateNavMenu(v: android.view.View, joined_items: JString, joined_icons: JString, joined_sections: JString),
        static fn updateWindowToolbar(root: android.view.View, id: JString, op: jint, num: jdouble),
        static fn updateWindowToolbarText(root: android.view.View, id: JString, text: JString),
        static fn windowImage(chrome: jboolean) -> jbyte[],
            // Read by pieces outside the toolkit (day-piece-crash-reporter's crash store root).
            static fn postMain(token: jlong),
            static fn postMainDelayed(token: jlong, delay_ms: jlong),
        },
        // Java's way into Rust, registered when `init` resolves the binding (nativeStart and
        // nativeStartWindow stay with the app's `day_start_android!`: they start the app's root).
        native_methods {
            static fn nativeOnEvent(id: jlong, kind: jint, num: jdouble, text: JString),
            static fn nativeHandlesKeys(id: jlong) -> jboolean,
            static fn nativeHandlesText(id: jlong) -> jboolean,
            static fn nativeContextMenu(token: jlong, x: jfloat, y: jfloat) -> JString,
            static fn nativeButtonMenu(token: jlong) -> JString,
            static fn nativeRunPosted(token: jlong),
            static fn nativeDoFrame(token: jlong, frame_time_nanos: jlong),
            static fn nativeListLen(host_id: jlong) -> jint,
            static fn nativeListBind(host_id: jlong, position: jint, cell: android.view.View),
            static fn nativeListBindPinned(host_id: jlong, position: jint, cell: android.view.View),
            static fn nativeListRecycle(host_id: jlong, cell: android.view.View),
            static fn nativeListIsSelected(host_id: jlong, position: jint) -> jboolean,
            static fn nativeListRowKind(host_id: jlong, position: jint) -> jint,
            static fn nativeListViewport(host_id: jlong, first: jint, last: jint, end_px: jint),
            static fn nativeListReaderScroll(host_id: jlong),
            static fn nativeListHeaderOf(host_id: jlong, position: jint) -> jint,
            static fn nativeListCanDrop(host_id: jlong, from: jint, to: jint) -> jboolean,
            static fn nativeListMove(host_id: jlong, from: jint, to: jint) -> jboolean,
            static fn nativeListCanDelete(host_id: jlong, index: jint) -> jboolean,
            static fn nativeListDelete(host_id: jlong, index: jint) -> jboolean,
        },
    }

    /// Android and Java classes the toolkit calls instance methods on, each bound where the
    /// method is declared. Platform classes are never shrunk by R8, so their keep rules change
    /// nothing; the bindings give the calls types and the startup check a say.
    pub(crate) mod framework {
        // A list row's standard content (docs/list.md "List items"): DayListItem.java.
        crate::java_class! {
            pub DayListItem => dev.daybrite.day.bridge.DayListItem,
            methods {
                static fn make(title: JString, subtitle: JString, detail: JString, icon: JString,
                    tint: jint, checked: jboolean) -> android.view.View,
                static fn setTexts(v: android.view.View, title: JString, subtitle: JString,
                    detail: JString),
                static fn setIcon(v: android.view.View, name: JString, tint: jint),
                static fn setChecked(v: android.view.View, on: jboolean),
            },
        }

        crate::java_class! {
            pub System => java.lang.System,
            methods {
                static fn identityHashCode(object: JObject) -> jint,
            },
        }

        crate::java_class! {
            pub Context => android.content.Context,
            methods {
                fn getAssets() -> android.content.res.AssetManager,
            },
        }

        crate::java_class! {
            pub View => android.view.View,
            methods {
                fn getWidth() -> jint,
                fn setPointerIcon(icon: android.view.PointerIcon),
            },
        }

        crate::java_class! {
            pub TextView => android.widget.TextView,
            methods {
                fn setTextIsSelectable(selectable: jboolean),
            },
        }

        crate::java_class! {
            pub PointerIcon => android.view.PointerIcon,
            methods {
                static fn getSystemIcon(context: android.content.Context, kind: jint)
                    -> android.view.PointerIcon,
            },
        }
    }

    // DayTransfer (java/dev/daybrite/day/bridge/DayTransfer.java): drag-and-drop sources and
    // targets (docs/drag-and-drop.md).
    crate::java_class! {
        DayTransfer => dev.daybrite.day.bridge.DayTransfer,
        methods {
            static fn source(view: android.view.View, token: jlong),
            static fn target(view: android.view.View, token: jlong),
            static fn release(view: android.view.View),
        },
        native_methods {
            static fn nativeCall(kind: jint, token: jlong, x: jdouble, y: jdouble, bytes: jbyte[],
                local: jboolean) -> jbyte[],
        },
    }

    // DayCanvasView (java/dev/daybrite/day/bridge/DayCanvasView.java): the canvas exports, which
    // paint with the OpPainter the view's own onDraw uses (docs/canvas.md "Exporting").
    crate::java_class! {
        DayCanvasView => dev.daybrite.day.bridge.DayCanvasView,
        methods {
            static fn exportError() -> JString,
            static fn renderBitmap(id: jlong, nums: jdouble[], texts_joined: JString,
                geometry: jdouble[], pw: jint, ph: jint, scale: jdouble) -> JString,
            static fn renderPdf(nums: jdouble[], num_counts: jint[], texts_joined: JString,
                text_counts: jint[], sizes: jdouble[]) -> jbyte[],
        },
    }

    /// The natives `DayBridge` declares, each forwarding to the function that does the work.
    /// jni's wrapper around each catches a panic and logs a returned error, so none of these
    /// can unwind into the JVM.
    impl DayBridgeNativeInterface for DayBridgeAPI {
        type Error = jni::errors::Error;

        fn nativeOnEvent<'l>(
            env: &mut Env<'l>,
            _class: JClass<'l>,
            id: i64,
            kind: i32,
            num: f64,
            text: JString<'l>,
        ) -> Result<(), Self::Error> {
            dispatch_event(env, id, kind, num, &text);
            Ok(())
        }

        fn nativeContextMenu<'l>(
            env: &mut Env<'l>,
            _class: JClass<'l>,
            token: i64,
            x: f32,
            y: f32,
        ) -> Result<JString<'l>, Self::Error> {
            let spec = day_spec::ffi_guard::contain(String::new(), || {
                // Clone before calling app code: a provider may replace or release its view.
                let provider = CONTEXT_MENU_FNS.with(|menus| menus.get(token as usize));
                let mut spec = String::new();
                if let Some(provider) = provider {
                    let items = provider(Point {
                        x: f64::from(x) / density(),
                        y: f64::from(y) / density(),
                    });
                    serialize_menu(&items, &mut spec);
                }
                spec
            });
            env.new_string(spec)
        }

        fn nativeButtonMenu<'l>(
            env: &mut Env<'l>,
            _class: JClass<'l>,
            token: i64,
        ) -> Result<JString<'l>, Self::Error> {
            let spec = day_spec::ffi_guard::contain(String::new(), || {
                // Cloned before the call: the provider is app code and may release its button.
                let provider = BUTTON_MENU_FNS.with(|menus| menus.get(token as usize));
                let mut spec = String::new();
                if let Some(provider) = provider {
                    // The provider may remove its button; then no menu opens (docs/menus.md).
                    let still = || {
                        BUTTON_MENU_FNS
                            .with(|menus| menus.get(token as usize))
                            .is_some()
                    };
                    if let Some(items) = day_spec::resolve_button_menu(&provider, still) {
                        serialize_menu(&items, &mut spec);
                    }
                }
                spec
            });
            env.new_string(spec)
        }

        fn nativeHandlesKeys<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            id: i64,
        ) -> Result<bool, Self::Error> {
            Ok(handles_keys(id))
        }

        fn nativeHandlesText<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            id: i64,
        ) -> Result<bool, Self::Error> {
            Ok(handles_text(id))
        }

        fn nativeRunPosted<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            token: i64,
        ) -> Result<(), Self::Error> {
            run_posted(token);
            Ok(())
        }

        fn nativeDoFrame<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            token: i64,
            frame_time_nanos: i64,
        ) -> Result<(), Self::Error> {
            run_frame(token, frame_time_nanos);
            Ok(())
        }

        fn nativeListLen<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
        ) -> Result<i32, Self::Error> {
            Ok(i32::try_from(list_len(host_id)).unwrap_or(i32::MAX))
        }

        fn nativeListBind<'l>(
            env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            position: i32,
            cell: JObject<'l>,
        ) -> Result<(), Self::Error> {
            list_bind(env, host_id, position, cell, false);
            Ok(())
        }

        fn nativeListBindPinned<'l>(
            env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            position: i32,
            cell: JObject<'l>,
        ) -> Result<(), Self::Error> {
            list_bind(env, host_id, position, cell, true);
            Ok(())
        }

        fn nativeListRecycle<'l>(
            env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            cell: JObject<'l>,
        ) -> Result<(), Self::Error> {
            list_recycle(env, host_id, cell);
            Ok(())
        }

        fn nativeListIsSelected<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            position: i32,
        ) -> Result<bool, Self::Error> {
            Ok(list_is_selected(host_id, position))
        }

        fn nativeListRowKind<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            position: i32,
        ) -> Result<i32, Self::Error> {
            Ok(list_row_kind(host_id, position))
        }

        fn nativeListViewport<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            first: i32,
            last: i32,
            end_px: i32,
        ) -> Result<(), Self::Error> {
            list_viewport(host_id, first, last, end_px);
            Ok(())
        }

        fn nativeListReaderScroll<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
        ) -> Result<(), Self::Error> {
            list_reader_scroll(host_id);
            Ok(())
        }

        fn nativeListHeaderOf<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            position: i32,
        ) -> Result<i32, Self::Error> {
            Ok(list_header_of(host_id, position))
        }

        fn nativeListCanDrop<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            from: i32,
            to: i32,
        ) -> Result<bool, Self::Error> {
            Ok(list_can_drop(host_id, from, to))
        }

        fn nativeListMove<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            from: i32,
            to: i32,
        ) -> Result<bool, Self::Error> {
            Ok(list_move(host_id, from, to))
        }

        fn nativeListCanDelete<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            index: i32,
        ) -> Result<bool, Self::Error> {
            Ok(list_can_delete(host_id, index))
        }

        fn nativeListDelete<'l>(
            _env: &mut Env<'l>,
            _class: JClass<'l>,
            host_id: i64,
            index: i32,
        ) -> Result<bool, Self::Error> {
            Ok(list_delete(host_id, index))
        }
    }

    impl DayTransferNativeInterface for DayTransferAPI {
        type Error = jni::errors::Error;

        #[allow(clippy::too_many_arguments)]
        fn nativeCall<'l>(
            env: &mut Env<'l>,
            _class: JClass<'l>,
            kind: i32,
            token: i64,
            x: f64,
            y: f64,
            bytes: jni::objects::JByteArray<'l>,
            local: bool,
        ) -> Result<jni::objects::JByteArray<'l>, Self::Error> {
            transfer_call(env, kind, token, x, y, bytes, local)
        }
    }

    #[derive(Clone)]
    pub struct AHandle(pub Gref);

    // --- Bundled data resources via the NDK AAssetManager (§18.3) --------------------------------
    // `resource("name")` reads the APK asset `name` with a zero-copy pointer into the (uncompressed)
    // asset via AAsset_getBuffer, the native AssetManager path the user asked for.
    #[allow(non_camel_case_types)]
    mod aasset {
        use std::os::raw::{c_char, c_int, c_void};
        pub enum AAssetManager {}
        pub enum AAsset {}
        pub const AASSET_MODE_BUFFER: c_int = 3;
        #[link(name = "android")]
        unsafe extern "C" {
            pub fn AAssetManager_fromJava(
                env: *mut jni::sys::JNIEnv,
                mgr: jni::sys::jobject,
            ) -> *mut AAssetManager;
            pub fn AAssetManager_open(
                mgr: *mut AAssetManager,
                filename: *const c_char,
                mode: c_int,
            ) -> *mut AAsset;
            pub fn AAsset_getBuffer(asset: *mut AAsset) -> *const c_void;
            pub fn AAsset_getLength64(asset: *mut AAsset) -> i64;
            pub fn AAsset_close(asset: *mut AAsset);
        }
    }

    /// The app's `AAssetManager` plus a `GlobalRef` to the Java `AssetManager` that keeps it alive.
    struct AssetMgr {
        aam: *mut aasset::AAssetManager,
        _keepalive: Global<JObject<'static>>,
    }
    // The AAssetManager pointer is valid for the app lifetime; resource() runs on the main thread.
    unsafe impl Send for AssetMgr {}
    unsafe impl Sync for AssetMgr {}
    static ASSET_MGR: OnceLock<AssetMgr> = OnceLock::new();

    /// Capture the `AAssetManager` from `DayBridge.ctx.getAssets()` and register the opener (init).
    fn register_resource_opener(env: &mut Env) {
        // Held globally: the native manager is valid only while its Java one is reachable.
        let Some(keepalive) = java_in(env, |env| {
            let ctx = DayRuntime::ctx(env)?;
            // SAFETY: `DayBridge.ctx` is declared `android.content.Context`.
            let am = unsafe { env.as_cast_unchecked::<framework::Context>(&ctx) }.getAssets(env)?;
            env.new_global_ref(am)
        }) else {
            return;
        };
        let aam =
            unsafe { aasset::AAssetManager_fromJava(env.get_raw(), keepalive.as_obj().as_raw()) };
        if aam.is_null() {
            return;
        }
        let _ = ASSET_MGR.set(AssetMgr {
            aam,
            _keepalive: keepalive,
        });
        day_spec::resource::set_resource_opener(open_resource);
    }

    /// Opener: `resource("name")` -> the APK asset `name`, zero-copy from `AAsset_getBuffer`.
    fn open_resource(name: &str) -> Option<day_spec::resource::Resource> {
        struct AssetGuard(*mut aasset::AAsset);
        impl Drop for AssetGuard {
            fn drop(&mut self) {
                unsafe { aasset::AAsset_close(self.0) };
            }
        }
        let mgr = ASSET_MGR.get()?.aam;
        let cname = std::ffi::CString::new(name).ok()?;
        let asset =
            unsafe { aasset::AAssetManager_open(mgr, cname.as_ptr(), aasset::AASSET_MODE_BUFFER) };
        if asset.is_null() {
            return None;
        }
        let len = unsafe { aasset::AAsset_getLength64(asset) };
        let ptr = unsafe { aasset::AAsset_getBuffer(asset) }.cast::<u8>();
        if ptr.is_null() || len < 0 {
            unsafe { aasset::AAsset_close(asset) };
            return None;
        }
        // Safety: `ptr`/`len` are the asset's buffer, valid until AAsset_close (held by the guard).
        Some(unsafe {
            day_spec::resource::Resource::from_raw(ptr, len as usize, Box::new(AssetGuard(asset)))
        })
    }

    /// The day-core event sink (node-id keyed).
    type Sink = Rc<dyn Fn(NodeId, Event)>;

    pub fn emit(id: NodeId, ev: Event) {
        let sink = SINK.with(|s| s.borrow().clone());
        if let Some(sink) = sink {
            sink(id, ev);
        }
    }

    fn density() -> f64 {
        DENSITY.with(std::cell::Cell::get)
    }

    /// Run with an attached `Env` (public: external renderers use this too). jni 0.22's
    /// `attach_current_thread` is callback-scoped; the callback returns `Ok` so the outer
    /// `Result` just unwraps.
    /// `PointerIcon.TYPE_*` for a [`Cursor`] (docs/cursor.md): the full CSS vocabulary has a
    /// system icon on Android, so nothing here approximates. `None` releases the view.
    fn pointer_icon_type(c: &Cursor) -> Option<i32> {
        Some(match c {
            Cursor::Default => return None,
            Cursor::Pointer => 1002,
            Cursor::Text => 1008,
            Cursor::VerticalText => 1009,
            Cursor::Crosshair => 1007,
            Cursor::Move => 1013,
            Cursor::Grab => 1020,
            Cursor::Grabbing => 1021,
            Cursor::NotAllowed => 1012,
            Cursor::Wait | Cursor::Progress => 1004,
            Cursor::Help => 1003,
            Cursor::ContextMenu => 1001,
            Cursor::Copy => 1011,
            Cursor::Alias => 1010,
            Cursor::Cell => 1006,
            Cursor::ZoomIn => 1018,
            Cursor::ZoomOut => 1019,
            Cursor::None => 0,
            Cursor::NsResize | Cursor::RowResize => 1015,
            Cursor::EwResize | Cursor::ColResize => 1014,
            Cursor::NeswResize => 1016,
            Cursor::NwseResize => 1017,
            Cursor::Native(name) => match &**name {
                "all_scroll" => 1013,
                "no_drop" => 1012,
                "top_right_diagonal_double_arrow" => 1016,
                "top_left_diagonal_double_arrow" => 1017,
                _ => return None,
            },
        })
    }

    /// A View returned by a Java factory, as a Day handle: `Err` for a null View, which a
    /// factory returns when it could not build one.
    ///
    /// # Errors
    ///
    /// Returns [`jni::errors::Error::NullPtr`] for a null View, and the JNI error when no
    /// global reference to the View can be made.
    pub fn view_handle(env: &mut Env, view: JObject) -> jni::errors::Result<AHandle> {
        if view.is_null() {
            return Err(jni::errors::Error::NullPtr("factory returned a null View"));
        }
        Ok(AHandle(std::sync::Arc::new(env.new_global_ref(view)?)))
    }

    /// A piece's native view, made by a Java factory through a [`java_class!`](crate::java_class)
    /// binding: the factory's View as a Day handle, or, when the factory throws or returns null,
    /// the visible `⟨what⟩` placeholder. A failed make must not panic inside realize: the panic
    /// would unwind through a JNI up-call and abort the process.
    ///
    /// ```ignore
    /// day_toolkit_material::realize("activity", |env| DayActivity::makeActivity(env, true, false))
    /// ```
    pub fn realize<F>(what: &str, make: F) -> AHandle
    where
        F: for<'a, 'l> FnOnce(&'a mut Env<'l>) -> jni::errors::Result<JObject<'l>>,
    {
        java(|env| {
            let view = make(env)?;
            view_handle(env, view)
        })
        .unwrap_or_else(|| {
            log::warn!(
                "day-toolkit-material: {what}: the Java factory failed; substituting a placeholder"
            );
            AHandle(with_env(|env| placeholder_view(env, what)))
        })
    }

    /// Pack a color the way `android.graphics.Color` wants it (`0xAARRGGBB`, as a signed int).
    #[must_use]
    pub fn argb(c: day_spec::Color) -> i32 {
        let f = |v: f64| (v.clamp(0.0, 1.0) * 255.0) as u32;
        ((f(c.a) << 24) | (f(c.r) << 16) | (f(c.g) << 8) | f(c.b)) as i32
    }

    fn apply_button_content(
        env: &mut Env,
        view: &Gref,
        title: &str,
        icon: Option<&day_spec::Icon>,
        icon_only: bool,
    ) {
        let title = jstr(env, title);
        let icon = jstr(env, &icon_name(icon));
        let _ = java_in(env, |env| {
            DayBridge::setButtonContent(env, view.as_obj(), &title, &icon, icon_only)
        });
    }

    /// Style a button in place through the bridge. The view stays a `MaterialButton`, so its
    /// ripple, state overlays, focus and accessibility role are Material's, not ours.
    pub fn apply_button_style(env: &mut Env, v: &Gref, style: day_spec::props::ButtonStyleSpec) {
        use day_spec::props::ButtonStyleSpec as S;
        let (kind, fill) = match style {
            S::Automatic => (0, day_spec::Color::CLEAR),
            S::Bordered => (1, day_spec::Color::CLEAR),
            S::Prominent => (2, day_spec::Color::CLEAR),
            S::Tinted(c) => (3, c.resolved()),
            // Kind 4 drops Material's 88 dp minimum width and its wide insets (docs/buttons.md).
            S::Compact => (4, day_spec::Color::CLEAR),
            S::Plain => (5, day_spec::Color::CLEAR),
        };
        let _ = java_in(env, |env| {
            DayBridge::setButtonStyle(env, v.as_obj(), kind, argb(fill), argb(S::on_tint(fill)))
        });
    }

    /// Send a label's text + runs across in one call (docs/text-runs.md).
    ///
    /// Offsets convert from Rust BYTES to Java's UTF-16 indices here, per run: any emoji or CJK
    /// in the string makes the two disagree, and an off-by-N range styles the wrong words.
    pub fn set_label_runs(
        env: &mut Env,
        node: i64,
        v: &Gref,
        text: &str,
        runs: &[day_spec::TextRun],
    ) {
        // Flat parallel arrays: one JNI call rather than one per run, on a path that runs for
        // every label patch.
        let mut starts = Vec::with_capacity(runs.len());
        let mut ends = Vec::with_capacity(runs.len());
        let mut flags = Vec::with_capacity(runs.len());
        let mut colors = Vec::with_capacity(runs.len());
        let mut backgrounds = Vec::with_capacity(runs.len());
        // Relative size, in per-mille: an int array so it rides the same cheap `set_region`
        // path as the rest rather than needing a float array of its own.
        let mut scales = Vec::with_capacity(runs.len());
        let mut links: Vec<Option<String>> = Vec::with_capacity(runs.len());
        // The lines Android has no span for, five ints each (`DayLabel.DECO_STRIDE`).
        let mut decorations = Vec::new();
        for r in runs {
            let Some(slice) = text.get(r.range.clone()) else {
                continue;
            };
            let start = text[..r.range.start].encode_utf16().count() as i32;
            let end = start + slice.encode_utf16().count() as i32;
            starts.push(start);
            ends.push(end);
            let f = crate::text_wire::run_flags(r);
            crate::text_wire::decoration_entries(r, start, end, &mut decorations);
            flags.push(f);
            colors.push(r.color.map_or(0, |c| argb(c.resolved())));
            backgrounds.push(r.background.map_or(0, |c| argb(c.resolved())));
            scales.push((r.font.scale * 1000.0).round() as i32);
            links.push(r.link.clone());
        }
        let (Ok(sa), Ok(se), Ok(sf), Ok(sc), Ok(sb), Ok(sz), Ok(sd)) = (
            env.new_int_array(starts.len()),
            env.new_int_array(ends.len()),
            env.new_int_array(flags.len()),
            env.new_int_array(colors.len()),
            env.new_int_array(backgrounds.len()),
            env.new_int_array(scales.len()),
            env.new_int_array(decorations.len()),
        ) else {
            return;
        };
        if sa.set_region(env, 0, &starts).is_err()
            || se.set_region(env, 0, &ends).is_err()
            || sf.set_region(env, 0, &flags).is_err()
            || sc.set_region(env, 0, &colors).is_err()
            || sb.set_region(env, 0, &backgrounds).is_err()
            || sz.set_region(env, 0, &scales).is_err()
            || sd.set_region(env, 0, &decorations).is_err()
        {
            return;
        }
        // Link targets ride one joined string, as the canvas op stream already does: a Java
        // object array through JNI costs a class lookup and a per-element store for a payload
        // that is empty in almost every label.
        let joined = links
            .iter()
            .map(|l| l.clone().unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\u{1f}");
        let s = jstr(env, text);
        let jl = jstr(env, &joined);
        let _ = java_in(env, |env| {
            DayBridge::setLabelRuns(
                env,
                v.as_obj(),
                node,
                &s,
                &sa,
                &se,
                &sf,
                &sc,
                &sb,
                &sz,
                &jl,
                &sd,
            )
        });
    }

    /// A View from a `DayBridge` factory, or the visible `⟨what⟩` placeholder when the factory
    /// throws or returns null (degrade loudly, keep the tree building).
    fn bridge_view<F>(env: &mut Env, what: &str, make: F) -> Gref
    where
        F: for<'a, 'l> FnOnce(&'a mut Env<'l>) -> jni::errors::Result<JObject<'l>>,
    {
        match try_bridge_view(env, make) {
            Ok(g) => g,
            Err(e) => {
                log::warn!(
                    "day-toolkit-material: DayBridge.{what} failed ({e}); substituting a placeholder view"
                );
                placeholder_view(env, what)
            }
        }
    }

    /// [`bridge_view`] without the placeholder: `Err` on a Java throw (described and cleared) or
    /// a null View.
    fn try_bridge_view<F>(env: &mut Env, make: F) -> Result<Gref, jni::errors::Error>
    where
        F: for<'a, 'l> FnOnce(&'a mut Env<'l>) -> jni::errors::Result<JObject<'l>>,
    {
        let made = match make(env) {
            Ok(view) => view_handle(env, view).map(|h| h.0),
            Err(e) => Err(e),
        };
        if made.is_err() && env.exception_check() {
            env.exception_describe();
            env.exception_clear();
        }
        made
    }

    /// The visible stand-in for a native make that failed: the same `⟨name⟩` label the
    /// missing-renderer path shows (degrade loudly, keep the tree building).
    ///
    /// # Panics
    ///
    /// Panics when `DayBridge.makeLabel` itself fails, which means the bridge is not staged
    /// and nothing can render.
    pub fn placeholder_view(env: &mut Env, what: &str) -> Gref {
        let text = format!("⟨{what}⟩");
        try_bridge_view(env, |env| {
            let text = JString::from_str(env, &text)?;
            DayBridge::makeLabel(env, &text)
        })
        // The bridge itself cannot build a plain label: nothing can render. Aborting
        // here is warranted, and it is the only remaining panic on this path.
        .expect("day-toolkit-material: DayBridge.makeLabel unavailable — bridge not staged?")
    }

    /// One `DayBridge` call made for its effect, inside [`java`]: a throw is logged and cleared.
    fn bridge(call: impl FnOnce(&mut Env) -> jni::errors::Result<()>) {
        let _ = java(call);
    }

    /// An `AnimSpec` as the bridge's animation arguments (`motion_wire`): the timing `int[]`
    /// and the cycle's sampled curve as a `float[]`. `None` is all zeros and no samples, which
    /// `DayBridge` applies at once, ending any run on that property.
    fn anim_arrays<'local>(
        env: &mut Env<'local>,
        anim: Option<&AnimSpec>,
    ) -> jni::errors::Result<(
        jni::objects::JIntArray<'local>,
        jni::objects::JFloatArray<'local>,
    )> {
        let timing = crate::motion_wire::timing(anim);
        let keys = crate::motion_wire::keys(anim);
        let t = env.new_int_array(timing.len())?;
        t.set_region(env, 0, &timing)?;
        let k = env.new_float_array(keys.len())?;
        k.set_region(env, 0, &keys)?;
        Ok((t, k))
    }

    /// Apply a `background`/`corner_radius` surface: a rounded `GradientDrawable` background +
    /// `clipToOutline`. The radius is density-scaled here (Java takes px). Idempotent; used at
    /// realize and on a reactive background patch.
    fn apply_surface(h: &AHandle, bg: Option<day_spec::Color>, corner_radius: f64, clips: bool) {
        let d = DENSITY.with(std::cell::Cell::get);
        bridge(|env| {
            DayBridge::setSurface(
                env,
                h.0.as_obj(),
                bg.map_or(0, argb_i32),
                bg.is_some(),
                (corner_radius * d) as f32,
                clips,
            )
        });
    }

    /// Apply a surface with a gradient, a border, named corners or a shadow
    /// (docs/surfaces.md): the container draws it itself, from [`super::surface_spec`].
    fn apply_styled_surface(h: &AHandle, p: &ContainerProps) {
        let spec = super::surface_spec(p, DENSITY.with(std::cell::Cell::get));
        bridge(|env| {
            let arr = env.new_double_array(spec.len())?;
            arr.set_region(env, 0, &spec)?;
            DayBridge::setSurfaceStyle(env, h.0.as_obj(), &arr, p.clips)
        });
    }

    /// A container's surface from its props, at realize (`fresh`) and again for every
    /// `ContainerPatch::Surface`: the styled path for a gradient, a border, named corners or a
    /// shadow, the plain `GradientDrawable` path for the rest. On a patch,
    /// `DayBridge.resetSurface` first clears what the previous style left (a drawable, an
    /// outline clip, a self-drawn surface), so a surface moves between the two paths in place.
    fn apply_container_surface(h: &AHandle, p: &ContainerProps, fresh: bool) {
        if !fresh {
            bridge(|env| DayBridge::resetSurface(env, h.0.as_obj()));
        }
        if super::is_styled_surface(p) {
            apply_styled_surface(h, p);
        } else if p.fill.is_some() || p.corner_radius > 0.0 || p.clips {
            apply_surface(
                h,
                super::solid_fill(p.fill.as_ref()),
                p.corner_radius,
                p.clips,
            );
        }
    }

    /// A `DayBridge` measurement of `h`'s View, in px. Runs inside the layout pass (a JNI
    /// up-call): a Java throw degrades to a zero measurement, never a panic-abort.
    fn measure_call<F>(h: &AHandle, measure: F) -> f64
    where
        F: for<'a, 'l> FnOnce(&'a mut Env<'l>, &JObject) -> jni::errors::Result<i32>,
    {
        f64::from(java(|env| measure(env, h.0.as_obj())).unwrap_or(0))
    }

    /// Initialize globals from the Activity's nativeStart (called by `day::android_start`).
    /// A JNI up-call entry: contained, so a startup failure logs instead of aborting.
    pub fn init(env: &mut Env, root: JObject, density_: f32, w: i32, h: i32) {
        day_spec::ffi_guard::contain((), || {
            // A re-mount (docs/settings.md "Appearance on Android"): an activity recreation
            // calls this a second time with a brand-new view hierarchy. Everything below maps a native view pointer (or a
            // host id) to something owned by the tree that is going away, and `LIST_SOURCES` in
            // particular holds closures that captured the previous mount's state. Left in place,
            // a callback from a stale view (or a recycled pointer that now means something else)
            // dispatches into the old graph and reads signals that were disposed with it.
            //
            // Cleared unconditionally rather than behind a "did we already start" flag: `init`
            // means "a tree is being mounted here", and on a first launch these are empty anyway.
            LIST_SOURCES.with(|m| m.borrow_mut().clear());
            LIST_NODE.with(|m| m.borrow_mut().clear());
            LIST_CELLS.with(|m| m.borrow_mut().clear());
            LIST_BINDINGS.with(|m| m.borrow_mut().clear());
            LIST_SELECTED.with(|m| m.borrow_mut().clear());
            LIST_AUTOMATIC.with(|m| m.borrow_mut().clear());
            NAV_MENU_ROWS.with(|m| m.borrow_mut().clear());
            NAV_SUITES.with(|m| m.borrow_mut().clear());
            WINDOW_BARS.with(|m| m.borrow_mut().clear());
            LABEL_NODE.with(|m| m.borrow_mut().clear());
            // Secondary windows do not survive the primary's recreation: their activities were
            // torn down with it, so the day-side records are stale global refs.
            SECONDARY.with(|m| m.borrow_mut().clear());
            if java_in(env, |env| {
                let class = env.find_class(jni::jni_str!("dev/daybrite/day/bridge/DayRuntime"))?;
                day_platform_android::init(env, &class)
            })
            .is_none()
            {
                return;
            }
            day_process::services::initialize();
            register_resource_opener(env);
            let d = f64::from(density_);
            DENSITY.with(|x| x.set(d));
            // Only fails when the JNI global-ref table is already exhausted; without a root
            // there is nothing to run, so degrade loudly rather than panic-abort the up-call.
            let Ok(root_ref) = env.new_global_ref(root) else {
                log::error!(
                    "day-toolkit-material: init could not take a global ref on the root view"
                );
                return;
            };
            let handle = AHandle(std::sync::Arc::new(root_ref));
            let size = Size::new(f64::from(w) / d, f64::from(h) / d);
            ROOT.with(|r| *r.borrow_mut() = Some((handle, size)));
            // Android's OS temp dir isn't app-writable; use the app cache dir for the file-save
            // staging area (docs/files.md) so `save_file(..)` can write its temp before handing
            // off to SAF.
            if let Some(path) = java_in(env, |env| {
                let dir = DayRuntime::cacheDirPath(env)?;
                string_of(env, &dir)
            }) && !path.is_empty()
            {
                day_spec::present::set_app_temp_dir(path);
            }
        });
    }

    /// A secondary `DayWindowActivity`'s first laid-out root (the `nativeStartWindow` JNI
    /// export lands here): adopt it as the parked day window's content
    /// (docs/windows.md). `false` ⇒ the window was closed before the activity finished
    /// connecting, and the activity finishes itself.
    #[expect(
        clippy::needless_pass_by_value,
        reason = "public signature; the JNI entry hands over the root's local reference"
    )]
    pub fn window_started(env: &mut Env, root: JObject, node: i64, w: i32, h: i32) -> bool {
        // JNI up-call entry (nativeStartWindow): contain the adoption + day-core completion.
        day_spec::ffi_guard::contain(false, || {
            let d = DENSITY.with(std::cell::Cell::get);
            let Ok(gref) = env.new_global_ref(&root) else {
                return false;
            };
            let gref = std::sync::Arc::new(gref);
            let raw = gref.as_obj().as_raw() as day_spec::RawHandle;
            SECONDARY.with(|s| s.borrow_mut().push((node as u64, gref)));
            let ok = day_core::finish_window_open(
                day_spec::NodeId(node as u64),
                raw,
                Size::new(f64::from(w) / d, f64::from(h) / d),
            );
            if !ok {
                SECONDARY.with(|s| s.borrow_mut().retain(|(n, _)| *n != node as u64));
            }
            ok
        })
    }

    /// The day node of the secondary window whose adopted content is `host`, if any.
    fn secondary_node_of(env: &mut Env, host: &AHandle) -> Option<u64> {
        SECONDARY.with(|s| {
            s.borrow()
                .iter()
                .find(|(_, gref)| {
                    env.is_same_object(host.0.as_obj(), gref.as_obj())
                        .unwrap_or(false)
                })
                .map(|(n, _)| *n)
        })
    }

    // The wire table (day_spec::bridge) as const match patterns; the Java side's K_* constants
    // mirror these, and day-toolkit-material's parity test holds the two files together.
    const K_PRESSED: i32 = bridge::BridgeKind::Pressed as i32;
    const K_TEXT_CHANGED: i32 = bridge::BridgeKind::TextChanged as i32;
    const K_TOGGLE_CHANGED: i32 = bridge::BridgeKind::ToggleChanged as i32;
    const K_VALUE_CHANGED: i32 = bridge::BridgeKind::ValueChanged as i32;
    const K_VALUE_COMMITTED: i32 = bridge::BridgeKind::ValueCommitted as i32;
    const K_SEARCH_CHANGED: i32 = bridge::BridgeKind::SearchChanged as i32;
    const K_NAV_PRESENTATION: i32 = bridge::BridgeKind::NavPresentation as i32;
    const K_LIST_ACTIVATED: i32 = bridge::BridgeKind::ListActivated as i32;
    const K_SELECTION_CHANGED: i32 = bridge::BridgeKind::SelectionChanged as i32;
    const K_NAV_BACK: i32 = bridge::BridgeKind::NavBack as i32;
    const K_FRAME_CHANGED: i32 = bridge::BridgeKind::FrameChanged as i32;
    const K_DEEPLINK: i32 = bridge::BridgeKind::Deeplink as i32;
    const K_PRESENT_BUTTON: i32 = bridge::BridgeKind::PresentButton as i32;
    const K_PRESENT_TEXT: i32 = bridge::BridgeKind::PresentText as i32;
    const K_PRESENT_DISMISSED: i32 = bridge::BridgeKind::PresentDismissed as i32;
    const K_GESTURE: i32 = bridge::BridgeKind::Gesture as i32;
    const K_CUSTOM: i32 = bridge::BridgeKind::Custom as i32;
    const K_MENU_ACTION: i32 = bridge::BridgeKind::MenuAction as i32;
    const K_LIFECYCLE: i32 = bridge::BridgeKind::Lifecycle as i32;
    const K_PRESENT_FILE: i32 = bridge::BridgeKind::PresentFile as i32;
    const K_FOCUS_CHANGED: i32 = bridge::BridgeKind::FocusChanged as i32;
    const K_SUBMITTED: i32 = bridge::BridgeKind::Submitted as i32;
    const K_WINDOW_RESIZED: i32 = bridge::BridgeKind::WindowResized as i32;
    const K_SAFE_AREA: i32 = bridge::BridgeKind::SafeArea as i32;
    const K_WINDOW_CLOSED: i32 = bridge::BridgeKind::WindowClosed as i32;
    const K_WINDOW_FOCUSED: i32 = bridge::BridgeKind::WindowFocused as i32;
    const K_APPEARANCE_CHANGED: i32 = bridge::BridgeKind::AppearanceChanged as i32;
    const K_REDUCE_MOTION_CHANGED: i32 = bridge::BridgeKind::ReduceMotionChanged as i32;
    const K_COVER_HIDDEN: i32 = bridge::BridgeKind::CoverHidden as i32;
    const K_LINK_ACTIVATED: i32 = bridge::BridgeKind::LinkActivated as i32;
    const K_TOOLBAR_CHANGED: i32 = bridge::BridgeKind::ToolbarChanged as i32;
    const K_KEY: i32 = bridge::BridgeKind::Key as i32;
    const K_TEXT: i32 = bridge::BridgeKind::Text as i32;
    const K_WINDOW_STATE_CHANGED: i32 = bridge::BridgeKind::WindowStateChanged as i32;

    /// The single native trampoline (the app's `nativeOnEvent` forwards here). The kind
    /// numbers are `day_spec::bridge::BridgeKind`, the shared wire table. A JNI up-call
    /// entry: the decode + dispatch is contained (`day_spec::ffi_guard`), since a panic
    /// unwinding this frame would abort the process.
    pub fn dispatch_event(env: &mut Env, id: i64, kind: i32, num: f64, jstr: &JString) {
        day_spec::ffi_guard::contain((), || {
            dispatch_event_inner(env, id, kind, num, jstr);
        });
    }

    /// Whether node `id` has a `Decorate::on_key` handler (docs/menus.md). A canvas asks before
    /// it claims a key or takes focus on a press, so one nobody wanted keys from leaves them to
    /// the platform's own focus navigation. A JNI up-call entry, contained.
    #[must_use]
    pub fn handles_keys(id: i64) -> bool {
        day_spec::ffi_guard::contain(false, || day_spec::keys::handled(NodeId(id as u64)))
    }

    /// Whether node `id` took the whole keyboard (`Decorate::keyboard`, docs/menus.md). A
    /// canvas asks before it raises the soft keyboard or names a letter. A JNI up-call
    /// entry, contained.
    #[must_use]
    pub fn handles_text(id: i64) -> bool {
        day_spec::ffi_guard::contain(false, || day_spec::keys::text(NodeId(id as u64)))
    }

    fn dispatch_event_inner(env: &mut Env, id: i64, kind: i32, num: f64, jstr: &JString) {
        if kind == K_CUSTOM {
            let text = string_of(env, jstr).ok().unwrap_or_default();
            // DayActivity is about to recreate itself (DayBridge.willRecreate): the route goes
            // into the activity's instance state before the tree it belongs to is torn down.
            if text == "day:will-recreate" {
                save_route_for_remount(env);
                return;
            }
            // A label's selection changed (`DayLabel.onSelectionChanged`, Label::on_selection):
            // byte offsets into the text the label displays, computed on the Java side where
            // the UTF-16 text is. Rides the custom channel because the kind table is shared
            // wire format and no kind carries a selection.
            if let Some(payload) = text.strip_prefix(crate::text_wire::SELECTION_PREFIX) {
                if let Some((start, end)) = crate::text_wire::selection(payload) {
                    emit(NodeId(id as u64), Event::TextSelected { start, end });
                }
                return;
            }
            // A scroll view's offset and phase (docs/scroll.md "Offset and phase"), sent by its
            // ScrollTrack only when the props asked: the shared kind table has neither.
            if let Some(payload) = text.strip_prefix(crate::list_wire::OFFSET_PREFIX) {
                if let Some(at) = crate::list_wire::offset(payload, density()) {
                    emit(NodeId(id as u64), Event::ScrollChanged(at));
                }
                return;
            }
            if let Some(payload) = text.strip_prefix(crate::list_wire::PHASE_PREFIX) {
                if let Some(phase) = crate::list_wire::phase(payload) {
                    emit(NodeId(id as u64), Event::ScrollPhase(phase));
                }
                return;
            }
            for tag in ["day:tab-new", "day:tab-close", "day:tab-move"] {
                if let Some(key) = text.strip_prefix(tag).and_then(|s| s.strip_prefix(':')) {
                    emit(
                        NodeId(id as u64),
                        Event::Custom {
                            tag,
                            num,
                            text: key.to_owned(),
                        },
                    );
                    return;
                }
            }
            if let Some(token) = text
                .strip_prefix("day-tree-expand:")
                .and_then(|s| s.parse::<u64>().ok())
            {
                emit(
                    NodeId(id as u64),
                    Event::TreeExpanded {
                        token,
                        expanded: num != 0.0,
                    },
                );
                return;
            }
            if let Some(token) = text
                .strip_prefix("day-tree-select:")
                .and_then(|s| s.parse::<u64>().ok())
            {
                TREE_SOURCES.with(|m| {
                    if let Some(e) = m.borrow_mut().get_mut(&id) {
                        e.2 = vec![token];
                    }
                });
                emit(NodeId(id as u64), Event::TreeSelection(vec![token]));
                return;
            }
        }

        if kind == K_CUSTOM
            && string_of(env, jstr).ok().as_deref() == Some("day-list-first-visible")
        {
            let source = LIST_SOURCES.with(|map| map.borrow().get(&id).cloned());
            if let Some(source) = source
                && let Some(report) = &source.first_visible
            {
                report((num.max(0.0) as usize).min((source.len)().saturating_sub(1)));
            }
            return;
        }
        let ev = match kind {
            K_PRESSED => Event::Pressed,
            k if k == bridge::BridgeKind::RefreshRequested as i32 => Event::RefreshRequested,
            K_TEXT_CHANGED => {
                let text = string_of(env, jstr).ok().unwrap_or_default();
                Event::TextChanged(text)
            }
            K_TOGGLE_CHANGED => Event::ToggleChanged(num != 0.0),
            K_VALUE_CHANGED => Event::ValueChanged(num),
            K_VALUE_COMMITTED => Event::ValueCommitted(num),
            // Inline search on the nav list (docs/search.md): the field's new text, or with
            // `num` 1 the text the keyboard's Search key submitted (DayNavHost sends that after
            // the last change).
            K_SEARCH_CHANGED => {
                let text = string_of(env, jstr).ok().unwrap_or_default();
                #[expect(
                    clippy::float_cmp,
                    reason = "`DayNavHost` sends the submit flag as exactly 1"
                )]
                let submitted = num == 1.0;
                if submitted {
                    Event::SearchSubmitted(text)
                } else {
                    Event::SearchChanged(text)
                }
            }
            // SlidingPaneLayout settled on a presentation; Day reconciles rather than drives.
            K_NAV_PRESENTATION => Event::NavPresentationChanged(if num >= 0.5 {
                day_spec::props::NavPresentation::Split
            } else {
                day_spec::props::NavPresentation::Stack
            }),
            K_SELECTION_CHANGED => Event::SelectionChanged(num as i64),
            K_LIST_ACTIVATED => Event::ListActivated(num as usize),
            // Navigation (docs/navigation.md): system back / gesture / toolbar up. num == 1.0
            // means the native FragmentManager already popped (predictive back commit, back
            // button, up arrow), so Rust updates the path without re-issuing the pop.
            // Id 0 is the app's own system-back callback (DayBridge.appBack, armed by
            // `set_system_back`): a back no cover or nav host took, for the app's handlers.
            K_NAV_BACK if id == 0 => {
                emit(
                    day_spec::WINDOW_NODE,
                    Event::NavBack {
                        already_popped: false,
                    },
                );
                return;
            }
            K_NAV_BACK => Event::NavBack {
                already_popped: num != 0.0,
            },
            // Nav page size report, "w,h" in px.
            K_FRAME_CHANGED => {
                let text: String = string_of(env, jstr).ok().unwrap_or_default();
                let Some((w, h)) = text.split_once(',') else {
                    return;
                };
                let d = DENSITY.with(std::cell::Cell::get);
                let (Ok(w), Ok(h)) = (w.parse::<f64>(), h.parse::<f64>()) else {
                    return;
                };
                Event::FrameChanged(Size::new(w / d, h / d))
            }
            // Warm deep link: the nav piece handles RouteRequested.
            k if k == bridge::BridgeKind::DocumentOpened as i32 => {
                if let Ok(path) = string_of(env, jstr) {
                    day_core::request_open_files(vec![path]);
                }
                return;
            }
            K_DEEPLINK => {
                let route: String = string_of(env, jstr).ok().unwrap_or_default();
                day_core::request_open_url(&route);
                return;
            }
            // The share sheet (docs/deep-links.md): a web link in it takes the deep-link path.
            k if k == bridge::BridgeKind::SharedText as i32 => {
                let payload: String = string_of(env, jstr).ok().unwrap_or_default();
                let (text, subject) = match payload.split_once('\u{1f}') {
                    Some((text, subject)) => (text, Some(subject)),
                    None => (payload.as_str(), None),
                };
                day_core::request_shared_text(text, subject);
                return;
            }
            // Presentation answers (docs/dialogs.md): id == request id.
            K_PRESENT_BUTTON => Event::PresentResult {
                req: id as u64,
                result: day_spec::present::PresentResult::Button(num as i64),
            },
            K_PRESENT_TEXT => {
                let text: String = string_of(env, jstr).ok().unwrap_or_default();
                Event::PresentResult {
                    req: id as u64,
                    result: day_spec::present::PresentResult::Text(text),
                }
            }
            K_PRESENT_DISMISSED => Event::PresentResult {
                req: id as u64,
                result: day_spec::present::PresentResult::Dismissed,
            },
            // File-picker answer (docs/files.md): string = chosen locators (a cache path for open,
            // a content:// URI for save), joined by the unit separator. Reuse the `decode` tag 3.
            K_PRESENT_FILE => {
                let text: String = string_of(env, jstr).ok().unwrap_or_default();
                Event::PresentResult {
                    req: id as u64,
                    result: day_spec::present::PresentResult::decode(3, 0, text),
                }
            }
            // Gestures (docs/gestures.md): `num` = the gesture code, `text` = its payload in
            // px (`gesture_event`). The input kind is the window's, whatever node sent it.
            K_GESTURE => {
                let text: String = string_of(env, jstr).ok().unwrap_or_default();
                let d = DENSITY.with(std::cell::Cell::get);
                match crate::gesture_event(num as i32, &text, d) {
                    Some(ev @ Event::PointerKindChanged(_)) => {
                        emit(day_spec::WINDOW_NODE, ev);
                        return;
                    }
                    Some(ev) => ev,
                    None => return,
                }
            }
            // Piece-defined custom event (§8.2's open event channel): a `&'static str` tag can't cross
            // JNI, so the tag is empty and the piece reads the primitive `num`/`text` payload. A piece
            // (e.g. day-piece-webview) calls `DayBridge.nativeOnEvent(id, 12, num, text)`.
            K_CUSTOM => {
                let text: String = string_of(env, jstr).ok().unwrap_or_default();
                Event::Custom { tag: "", num, text }
            }
            // A cover's hide slide finished (docs/cover.md): DayCover reports so Rust can
            // dispose the content.
            K_COVER_HIDDEN => Event::CoverHidden,
            // A styled run's link was tapped (docs/text-runs.md): the ClickableSpan reports its
            // target, and day-core routes it to the label's `.on_link()`.
            K_LINK_ACTIVATED => Event::LinkActivated(string_of(env, jstr).ok().unwrap_or_default()),
            // A key from a focused canvas or focusable view (docs/menus.md): the day key name,
            // with the modifier mask and the phase in `num`. The view already asked whether
            // this node claims keys.
            K_KEY => Event::Key(crate::key_event(
                false,
                string_of(env, jstr).ok().unwrap_or_default(),
                num,
            )),
            // Text the full route committed (docs/menus.md "The full route"): what a key or
            // the input method typed, with the modifier mask and the phase in `num`.
            K_TEXT => Event::Key(crate::key_event(
                true,
                string_of(env, jstr).ok().unwrap_or_default(),
                num,
            )),
            // A toolbar item's value (docs/toolbars.md): "sel" carries a segment index, a page
            // search field's "text:" / "submit:" its text after the prefix, anything else a
            // toggle's new state. Plain buttons arrive as MENU_ACTION, like the menus.
            K_TOOLBAR_CHANGED => {
                let text: String = string_of(env, jstr).ok().unwrap_or_default();
                let value = if text == "sel" {
                    day_spec::ToolbarValue::Selected(num.max(0.0) as usize)
                } else if let Some(t) = text.strip_prefix("text:") {
                    note_search_text(id as u64, t);
                    day_spec::ToolbarValue::Text(t.to_string())
                } else if let Some(t) = text.strip_prefix("submit:") {
                    day_spec::ToolbarValue::Submit(t.to_string())
                } else {
                    day_spec::ToolbarValue::On(num != 0.0)
                };
                Event::ToolbarChanged {
                    action: id as u64,
                    value,
                }
            }
            // Menu selection (docs/menus.md): `id` == the chosen action's dispatch id (0 for a
            // role/standard item, which dispatches to nothing). Routed by the pump to the closure.
            K_MENU_ACTION => Event::MenuAction(id as u64),
            // Activity lifecycle (docs/lifecycle.md): `num` is the phase code (day_spec::Lifecycle
            // order). DayActivity forwards onResume/onPause/onStart/onStop/onTrimMemory/onDestroy.
            K_LIFECYCLE => match android_lifecycle(num as i32) {
                Some(phase) => Event::Lifecycle(phase),
                None => return,
            },
            // Root size change (px as "w,h" text): the safe-area root grew or shrank (a late
            // inset pass, the soft keyboard, rotation, or a system-bar change). Routed to the
            // root as a window resize so Day relayouts; same rail as appkit's windowDidResize.
            // (18: the first free kind; 15 already carries file-picker answers.)
            // Safe-area report (edge-to-edge mode, docs/layout.md): update the global insets
            // signal; apps read `day::safe_area()`. Not an Event: nothing to route to a node.
            K_SAFE_AREA => {
                let text: String = string_of(env, jstr).ok().unwrap_or_default();
                let p: Vec<f64> = text.split(',').filter_map(|s| s.parse().ok()).collect();
                if p.len() < 4 {
                    return;
                }
                let d = DENSITY.with(std::cell::Cell::get);
                day_core::set_safe_area(day_spec::Insets {
                    top: p[0] / d,
                    bottom: p[1] / d,
                    leading: p[2] / d,
                    trailing: p[3] / d,
                });
                return;
            }
            // Light/dark switch (DayActivity.onConfigurationChanged). Not an Event: nothing to
            // route to a node; day-core restyles what it owns and rebuilds app-painted surfaces.
            K_APPEARANCE_CHANGED => {
                day_core::note_appearance_changed();
                return;
            }
            // The user's "Remove animations" switch (DayBridge.watchReduceMotion). Likewise not
            // an Event: day-core re-reads `reduce_motion` and lands every finite animation
            // (docs/accessibility.md).
            K_REDUCE_MOTION_CHANGED => {
                day_core::note_motion_changed();
                return;
            }
            K_WINDOW_RESIZED => {
                let text: String = string_of(env, jstr).ok().unwrap_or_default();
                let p: Vec<f64> = text.split(',').filter_map(|s| s.parse().ok()).collect();
                if p.len() < 2 {
                    return;
                }
                let d = DENSITY.with(std::cell::Cell::get);
                // id 0 = the primary activity (WINDOW_NODE); a secondary DayWindowActivity
                // reports against its own day root (docs/windows.md).
                let target = if id == 0 {
                    day_spec::WINDOW_NODE
                } else {
                    day_spec::NodeId(id as u64)
                };
                let size = Size::new(p[0] / d, p[1] / d);
                // The third field: the soft keyboard's share of the shrink, which the size
                // class adds back (docs/size-classes.md).
                let keyboard = p.get(2).map_or(0.0, |k| k.max(0.0) / d);
                day_core::note_keyboard_overlap(target, size, keyboard);
                emit(target, Event::WindowResized(size));
                return;
            }
            // A window's display state (docs/windows.md): the system bars hid or showed for a
            // fullscreen request, or a request the platform declined reports what the window
            // kept. Id 0 is the primary activity (WINDOW_NODE), like the resize rail.
            K_WINDOW_STATE_CHANGED => {
                let state = day_spec::WindowState::from_code(num as i32);
                log::debug!("android: window state now {state:?}");
                let target = if id == 0 {
                    day_spec::WINDOW_NODE
                } else {
                    day_spec::NodeId(id as u64)
                };
                emit(target, Event::WindowStateChanged(state));
                return;
            }
            // Secondary-window lifecycle (docs/windows.md): the id is the window's root.
            K_WINDOW_CLOSED => Event::WindowClosed,
            K_WINDOW_FOCUSED => Event::WindowFocused(num != 0.0),
            // Focus pair + IME submit action (docs/focus.md).
            K_FOCUS_CHANGED => Event::FocusChanged(num != 0.0),
            K_SUBMITTED => Event::Submitted,
            unknown => {
                // A silently dropped kind is how the kind-15 collision hid for weeks: say so
                // once per kind in debug builds (release stays quiet; this is a dev signal).
                #[cfg(debug_assertions)]
                {
                    use std::sync::{Mutex, OnceLock};
                    static SEEN: OnceLock<Mutex<std::collections::BTreeSet<i32>>> = OnceLock::new();
                    let seen = SEEN.get_or_init(|| Mutex::new(std::collections::BTreeSet::new()));
                    if let Ok(mut g) = seen.lock()
                        && g.insert(unknown)
                    {
                        log::warn!("day-toolkit-material: dropping unknown event kind {unknown}");
                    }
                }
                let _ = unknown;
                return;
            }
        };
        emit(NodeId(id as u64), ev);
    }

    /// Posted-closure trampoline (the app's `nativeRunPosted` forwards here). The closure is
    /// arbitrary app/day-core code inside a JNI up-call, so it runs contained.
    pub fn run_posted(token: i64) {
        // SAFETY: `token` is the Box::into_raw pointer `Platform::post` minted; Java hands it
        // back exactly once.
        let f: Box<Box<dyn FnOnce() + Send>> =
            unsafe { Box::from_raw(token as *mut Box<dyn FnOnce() + Send>) };
        day_spec::ffi_guard::contain((), f);
    }

    /// Frame-callback trampoline (the app's `nativeDoFrame` forwards here). `frame_nanos` is
    /// `Choreographer`'s frame time in nanoseconds; day-core wants seconds. Runs on the UI
    /// thread, contained like `run_posted`.
    pub fn run_frame(token: i64, frame_nanos: i64) {
        day_core::frame::native::deliver(
            token as u64,
            day_spec::FrameStamp::new(frame_nanos as f64 / 1_000_000_000.0),
        );
    }

    #[distributed_slice]
    pub static RENDERERS: [fn() -> Renderer<Material>];

    pub struct Material {
        registry: Registry<Material>,
    }

    impl Material {
        #[must_use]
        pub fn new() -> Self {
            let mut registry = Registry::default();
            for f in RENDERERS {
                registry.register(f());
            }
            Material { registry }
        }
    }

    impl Default for Material {
        fn default() -> Self {
            Self::new()
        }
    }

    /// `DayBridge.displayInfo()`'s tab-separated line as a [`day_spec::Monitor`]: id, name,
    /// scale, frame width and height, work-area left, top, width and height (dp).
    fn parse_display_info(line: &str) -> Option<day_spec::Monitor> {
        let f: Vec<&str> = line.split('\t').collect();
        let [id, name, rest @ ..] = f.as_slice() else {
            return None;
        };
        let n: Vec<f64> = rest
            .iter()
            .map(|s| s.parse::<f64>().ok())
            .collect::<Option<_>>()?;
        let [scale, w, h, wl, wt, ww, wh] = n.as_slice() else {
            return None;
        };
        Some(day_spec::Monitor {
            id: (*id).to_string(),
            name: (*name).to_string(),
            frame: day_spec::Rect::new(0.0, 0.0, *w, *h),
            work_area: day_spec::Rect::new(*wl, *wt, *ww, *wh),
            scale: *scale,
            primary: true,
        })
    }

    fn jstr(env: &mut Env, s: &str) -> jni::objects::JString<'static> {
        // SAFETY: local ref used immediately within the same JNI frame.
        unsafe { std::mem::transmute(env.new_string(s).expect("new_string")) }
    }

    /// Map an Android lifecycle phase code (`day_spec::Lifecycle` order) to the enum (docs/lifecycle.md).
    fn android_lifecycle(code: i32) -> Option<day_spec::Lifecycle> {
        use day_spec::Lifecycle::{
            DidBecomeActive, DidEnterBackground, DidExit, DidReceiveMemoryWarning,
            WillEnterForeground, WillResignActive, WillTerminate,
        };
        Some(match code {
            2 => DidBecomeActive,
            3 => WillResignActive,
            4 => WillEnterForeground,
            5 => DidEnterBackground,
            6 => DidReceiveMemoryWarning,
            7 => WillTerminate,
            8 => DidExit,
            _ => return None,
        })
    }

    /// Mobile backends deliver the full lifecycle (docs/lifecycle.md). `const` for
    /// `day::require_lifecycle!` compile-time guards.
    #[must_use]
    pub const fn lifecycle_supported(_phase: day_spec::Lifecycle) -> bool {
        true
    }

    /// Default label for a standard role left unlabeled by the app. (Android's own text-selection
    /// toolbar handles the actual Cut/Copy/Paste on editable views; a role in a day menu is shown
    /// for parity and dispatches nothing; see docs/menus.md.)
    fn android_role_label(role: day_spec::MenuRole) -> &'static str {
        use day_spec::MenuRole::{
            About, CheckForUpdates, CloseWindow, Copy, Cut, Delete, Fullscreen, Minimize,
            NewWindow, Paste, Preferences, Quit, Redo, SelectAll, Undo,
        };
        match role {
            Cut => "Cut",
            Copy => "Copy",
            Paste => "Paste",
            SelectAll => "Select All",
            Undo => "Undo",
            Redo => "Redo",
            Delete => "Delete",
            About => "About",
            Quit => "Quit",
            Preferences => "Settings",
            Minimize => "Minimize",
            CloseWindow => "Close",
            Fullscreen => "Full Screen",
            NewWindow => "New Window",
            CheckForUpdates => "Check for Updates…",
        }
    }

    /// Flatten the day-neutral menu tree to the line format `DayBridge.buildMenu` parses:
    /// `kind \t action \t enabled \t checked \t label` per line, where kind ∈ {A action,
    /// S submenu-open, E submenu-close, `-` separator}. Roles become plain actions with action 0,
    /// and `checked` is -1 for a plain command or 0/1 for a checkable one.
    /// The window toolbar as one record per item: `\u{1e}` between records, `\u{1f}` between
    /// fields: id, kind, label, icon, enabled, action, extra. A menu item's extra is the app-menu
    /// spec `serialize_menu` writes (its own `\t`/`\n` never collide with these separators); a
    /// segmented item's is the selected index and the segment titles, `\u{1d}`-separated, each
    /// title followed by its glyph after a `\u{1c}`. A page's search field (`day::search`,
    /// docs/search.md) is a `search` record whose extra is its text and its placeholder,
    /// `\u{1d}`-separated: the bar draws it as a collapsible search action. The sidebar toggle
    /// never reaches a phone's bar and is left out: this backend has no pane the toggle could
    /// move (`toggle_sidebar` answers false). A navigation surface's own search never arrives
    /// here either: it rides the navigation list (`Cap::ToolbarSearch` is unsupported).
    ///
    /// The showing page's own commands are written first, ahead of the window's chrome. A
    /// Material app bar shows icon actions in order and folds the rest into its overflow, and
    /// the page's commands are what a person came to the page for; the window's New Window and
    /// appearance chooser are one tap further away, in the overflow, when the bar is that
    /// narrow. The item's `column` tells the two apart: a destination's commands come from the
    /// detail (or list) column, the host's own from the sidebar column, and both ride the same
    /// merged window model here.
    fn serialize_toolbar(items: &[day_spec::ToolbarItem]) -> String {
        use day_spec::ToolbarItemKind as K;
        fn clean(s: &str) -> String {
            s.replace(['\u{1d}', '\u{1e}', '\u{1f}'], " ")
        }
        let (page, window): (Vec<_>, Vec<_>) = items
            .iter()
            .filter(|i| i.id != day_spec::SIDEBAR_TOGGLE_ID)
            .partition(|i| {
                matches!(
                    i.column,
                    day_spec::ToolbarColumn::Detail | day_spec::ToolbarColumn::List
                )
            });
        let mut out = String::new();
        for item in page.into_iter().chain(window) {
            let (kind, extra) = match &item.kind {
                K::Button => ("button", String::new()),
                K::Toggle { on } => ("toggle", u8::from(*on).to_string()),
                K::Menu { items } => {
                    let mut spec = String::new();
                    serialize_menu(items, &mut spec);
                    ("menu", spec)
                }
                K::Segmented { segments, selected } => {
                    // Each segment carries its title and its glyph (0x1C between them): the bar
                    // shows the control as one icon button (the segment in force's glyph)
                    // that opens the choices, so it needs every segment's art.
                    let mut e = selected.to_string();
                    for seg in segments {
                        e.push('\u{1d}');
                        e.push_str(&clean(&seg.title));
                        e.push('\u{1c}');
                        e.push_str(&clean(&icon_name(seg.icon.as_ref())));
                    }
                    ("segmented", e)
                }
                K::Label => ("label", String::new()),
                K::Separator => ("sep", String::new()),
                K::Search {
                    text, placeholder, ..
                } => (
                    "search",
                    format!("{}\u{1d}{}", clean(text), clean(placeholder)),
                ),
            };
            // A search action without an icon of its own shows the magnifier, the glyph every
            // Android app bar uses for it.
            let icon = match (&item.kind, &item.icon) {
                (K::Search { .. }, None) => symbol_drawable(day_spec::Symbol::Search),
                (_, icon) => icon_name(icon.as_ref()),
            };
            if !out.is_empty() {
                out.push('\u{1e}');
            }
            // An eighth field: the item's placement (docs/toolbars.md). The Java parser reads
            // fields positionally with `f.length >` guards, so a spec without it still parses.
            let placement = match item.placement {
                day_spec::ToolbarPlacement::Navigation => "nav",
                day_spec::ToolbarPlacement::Principal => "mid",
                day_spec::ToolbarPlacement::Primary => "primary",
                day_spec::ToolbarPlacement::Secondary => "secondary",
                // The nav host's bottom app bar; a docked bar folds these into its overflow.
                day_spec::ToolbarPlacement::Bottom => "bottom",
                day_spec::ToolbarPlacement::Automatic => "auto",
            };
            out.push_str(
                &[
                    clean(&item.id),
                    kind.to_string(),
                    clean(&item.label),
                    clean(&icon),
                    u8::from(item.enabled).to_string(),
                    item.action.to_string(),
                    extra,
                    placement.to_string(),
                ]
                .join("\u{1f}"),
            );
        }
        out
    }

    /// Record text the user typed into a page's search action in the bar's model, so the next
    /// repaint carries what the field shows. day-core's own copy follows the same event
    /// (`mirror_value`), but this backend repaints from its mirror, and a repaint carrying
    /// the text from before the typing would put it back into the field.
    fn note_search_text(action: u64, text: &str) {
        WINDOW_BARS.with(|m| {
            for bar in m.borrow_mut().values_mut() {
                let Some(item) = bar
                    .by_action(action)
                    .filter(|i| matches!(i.kind, day_spec::ToolbarItemKind::Search { .. }))
                    .map(|i| i.id.clone())
                else {
                    continue;
                };
                bar.patch(&day_spec::ToolbarPatch::Text {
                    item,
                    text: text.to_string(),
                });
            }
        });
    }

    /// The drawable name an item's icon resolves to on the Java side. A bundled image is named
    /// after itself (docs/vectors.md); a `Symbol` names the toolkit's glyph for it,
    /// `day_symbol_<snake_case>`, one Material Symbols vector per variant shipped in this crate's
    /// `res/`, so a symbol-only item has an icon to show in the bar, the way an SF Symbol gives
    /// it one on Apple. A variant with no glyph resolves to nothing and lands as text.
    fn icon_name(icon: Option<&day_spec::Icon>) -> String {
        match icon {
            Some(day_spec::Icon::Image(name)) => name.clone(),
            Some(day_spec::Icon::Symbol(sym)) => symbol_drawable(*sym),
            None => String::new(),
        }
    }

    /// `Symbol::ZoomIn` → `day_symbol_zoom_in`: the variant's name in snake case. Public so a
    /// piece drawing a standard glyph in its own Java view (day-piece-settings' group rows)
    /// resolves the same drawable a nav item does.
    #[must_use]
    pub fn symbol_drawable(sym: day_spec::Symbol) -> String {
        let mut out = String::from("day_symbol");
        for c in format!("{sym:?}").chars() {
            if c.is_ascii_uppercase() {
                out.push('_');
                out.push(c.to_ascii_lowercase());
            } else {
                out.push(c);
            }
        }
        out
    }

    fn serialize_menu(items: &[day_spec::MenuItem], out: &mut String) {
        use std::fmt::Write as _;
        fn clean(s: &str) -> String {
            s.replace(['\t', '\n'], " ")
        }
        // `kind \t action \t enabled \t checked \t label`. The label stays last so its own
        // spaces need no escaping, and every kind writes the full field count so the Java side
        // can index it; `checked` is -1 for a plain command, 0/1 for a checkable one.
        for item in items {
            match item {
                day_spec::MenuItem::Separator => out.push_str("-\t0\t1\t-1\t\n"),
                day_spec::MenuItem::Submenu { label, items, .. } => {
                    let _ = writeln!(out, "S\t0\t1\t-1\t{}", clean(label));
                    serialize_menu(items, out);
                    out.push_str("E\t0\t1\t-1\t\n");
                }
                day_spec::MenuItem::Action {
                    action,
                    label,
                    shortcut: _,
                    enabled,
                    checked,
                    role,
                    // The rest go unread: Material's overflow menu is text-only by
                    // convention (an app-bar action carries the icon), so `icon` never applies
                    // here, and `id` is the app's name for a script rather than anything the
                    // native item needs.
                    ..
                } => {
                    let text = match role {
                        Some(r) if label.is_empty() => android_role_label(*r).to_string(),
                        _ => label.clone(),
                    };
                    let _ = writeln!(
                        out,
                        "A\t{}\t{}\t{}\t{}",
                        action,
                        i32::from(*enabled),
                        checked.map_or(-1, i32::from),
                        clean(&text)
                    );
                }
            }
        }
    }

    /// Size (in **sp**, which scales with Settings ▸ Display ▸ Font size, the Android
    /// accessibility text scale) + the style's inherent weight for a logical [`Font`]. Mobile
    /// scale, aligned with iOS. Public for standalone pieces (docs/extending.md), which resolve
    /// the same scale.
    #[must_use]
    pub fn font_style(f: Font) -> (f32, day_spec::FontWeight) {
        use day_spec::FontWeight::{Regular, Semibold};
        match f {
            Font::LargeTitle => (34.0, Regular),
            Font::Title => (28.0, Regular),
            Font::Title2 => (22.0, Regular),
            Font::Title3 => (20.0, Regular),
            Font::Headline => (17.0, Semibold),
            Font::Subheadline => (15.0, Regular),
            Font::Body => (17.0, Regular),
            Font::Callout => (16.0, Regular),
            Font::Footnote => (13.0, Regular),
            Font::Caption => (12.0, Regular),
            Font::Caption2 => (11.0, Regular),
            Font::System(pt) | Font::Custom(_, pt) => (pt as f32, Regular),
        }
    }

    /// The bundled family name when the spec is `Font::Custom` (§18.4), passed to Java as the
    /// nullable `family` argument of `DayBridge.setLabelFont`, which resolves it to the
    /// `res/font/` resource `day build` staged from the project's `fonts/` directory.
    fn custom_family(spec: day_spec::FontSpec) -> Option<&'static str> {
        match spec.style {
            Font::Custom(name, _) => Some(name),
            _ => None,
        }
    }

    /// Day weight → Android font weight (Thin=100 … Black=900, for `Typeface.create(_, weight, _)`).
    fn android_weight(w: day_spec::FontWeight) -> i32 {
        use day_spec::FontWeight as W;
        match w {
            W::Thin => 100,
            W::UltraLight => 200,
            W::Light => 300,
            W::Regular => 400,
            W::Medium => 500,
            W::Semibold => 600,
            W::Bold => 700,
            W::Heavy => 800,
            W::Black => 900,
        }
    }

    /// (sp size, Android weight, italic, tabular) for `DayBridge.setLabelFont`. The size is
    /// the style's times [`day_spec::FontSpec::scale`], still in sp, so it keeps following the
    /// reader's font size setting (and a `max_text_scale` cap's scale lands here).
    fn font_params(spec: day_spec::FontSpec) -> (f32, i32, bool, bool) {
        let (sp, inherent) = font_style(spec.style);
        let weight = android_weight(spec.weight.unwrap_or(inherent));
        let sp = spec.resolved_points(f64::from(sp)) as f32;
        (sp, weight, spec.italic, spec.tabular)
    }

    /// A label's whole font (`LabelProps::font`, `LabelPatch::Font`): size, weight, slant, the
    /// family (a bundled one, or Android's `monospace` alias for `FontSpec::monospace`) and
    /// the OpenType features (`tnum`, `smcp`, `onum`).
    fn apply_label_font(env: &mut Env, view: &Gref, spec: day_spec::FontSpec) {
        let (sp, weight, italic, tabular) = font_params(spec);
        // A whole-label monospace ask rides the family parameter: "monospace" is Android's own
        // alias for the system fixed-pitch face, so this is the same request
        // `TypefaceSpan("monospace")` makes for a single run.
        let fam = match (custom_family(spec), spec.monospace) {
            (Some(f), _) => jstr(env, f),
            (None, true) => jstr(env, "monospace"),
            (None, false) => JString::default(),
        };
        let _ = java_in(env, |env| {
            DayBridge::setLabelFont(env, view.as_obj(), sp, weight, italic, &fam, tabular)
        });
        // `setLabelFont` asks for tabular figures alone; small capitals or oldstyle figures
        // replace that with the whole list.
        if spec.small_caps || spec.oldstyle_figures {
            let features = crate::text_wire::features(&spec).unwrap_or_default();
            let f = jstr(env, &features);
            let _ = java_in(env, |env| {
                DayBridge::setLabelFeatures(env, view.as_obj(), &f)
            });
        }
    }

    /// A label's paragraph attributes (`LabelProps::paragraph`, `LabelPatch::Paragraph`):
    /// the line limit, the truncation, the letter spacing and the line height, whole. Day
    /// measures the label again afterward, and the `TextView`'s own measurement honors them.
    fn apply_label_paragraph(env: &mut Env, view: &Gref, p: &day_spec::props::LabelParagraph) {
        let (lines, truncation, spacing, height) = crate::text_wire::paragraph_args(p);
        let _ = java_in(env, |env| {
            DayBridge::setLabelParagraph(env, view.as_obj(), lines, truncation, spacing, height)
        });
    }

    /// Day `Color` (0–1 floats) → a packed `0xAARRGGBB` int for `android.graphics.Color`.
    /// Per-row nav icon tints as an index-aligned joined string ("0" = untinted), the
    /// best-effort `setNavMenuTints` wire format (docs/vectors.md).
    fn nav_tints_joined(tints: &[Option<day_spec::Color>]) -> String {
        tints
            .iter()
            .map(|t| t.map_or(0, argb_i32).to_string())
            .collect::<Vec<_>>()
            .join("\u{1f}")
    }

    /// Per-row nav context menus (docs/menus.md) as one string: each row's
    /// [`serialize_menu`] spec (empty = no menu), joined by U+001E, a separator the line
    /// format itself never contains. Ridden best-effort after makeNavMenu/updateNavMenu,
    /// like the tints.
    /// Move the sidebar's active indicator to `sel`, or clear it (`NavMenuProps::selected`).
    ///
    /// Best-effort, like the tint and badge follow-ups: a decoration is never worth taking the
    /// nav host's build path down with it (the navhost lesson, docs/navigation.md).
    fn nav_menu_select(env: &mut Env, h: &AHandle, sel: Option<usize>) {
        let _ = java_in(env, |env| {
            DayBridge::setNavMenuSelected(env, h.0.as_obj(), sel.map_or(-1, |i| i as i32))
        });
        if env.exception_check() {
            env.exception_clear();
        }
    }

    fn nav_menus_joined(menus: &[Vec<day_spec::MenuItem>]) -> String {
        menus
            .iter()
            .map(|items| {
                let mut spec = String::new();
                serialize_menu(items, &mut spec);
                spec
            })
            .collect::<Vec<_>>()
            .join("\u{1e}")
    }

    fn argb_i32(c: day_spec::Color) -> i32 {
        super::argb_u32(c) as i32
    }

    /// `makeImage`/`setImageTint`'s "theme text color" value (docs/vectors.md "Template"):
    /// alpha 0, which no real tint has. The bridge's `TEMPLATE_TINT`.
    const TEMPLATE_TINT: i32 = 1;

    /// The bridge's image-tint channel: a packed ARGB, `0` for the authored colors, or
    /// [`TEMPLATE_TINT`] for a template glyph that takes the theme's text color.
    /// A list item's glyph as a drawable name: a staged image by its own name, a standard
    /// symbol by the `day_symbol_*` drawable it ships as.
    fn list_item_icon(icon: Option<&day_spec::Icon>) -> String {
        match icon {
            Some(day_spec::Icon::Image(name)) => name.clone(),
            Some(day_spec::Icon::Symbol(sym)) => symbol_drawable(*sym),
            None => String::new(),
        }
    }

    /// Apply one change to a realized list item (docs/list.md "List items"). The Java view
    /// sets its texts together, so the props are kept here to hand it the other two.
    fn list_item_patch(h: &AHandle, patch: &day_spec::props::ListItemPatch) {
        use day_spec::props::ListItemPatch as P;
        let key = h.0.as_obj().as_raw() as usize;
        let Some(p) = LIST_ITEM_PROPS.with(|m| {
            let mut m = m.borrow_mut();
            let p = m.get_mut(&key)?;
            match patch {
                P::Title(t) => p.title.clone_from(t),
                P::Subtitle(t) => p.subtitle.clone_from(t),
                P::Detail(t) => p.detail.clone_from(t),
                P::Icon(i) => p.icon.clone_from(i),
                P::IconTint(c) => p.icon_tint = *c,
                P::Checked(on) => p.checked = *on,
            }
            Some(p.clone())
        }) else {
            return;
        };
        with_env(|env| {
            let _ = match patch {
                P::Title(_) | P::Subtitle(_) | P::Detail(_) => {
                    let (t, s, d) = (
                        jstr(env, &p.title),
                        jstr(env, &p.subtitle),
                        jstr(env, &p.detail),
                    );
                    java_in(env, |env| {
                        DayListItem::setTexts(env, h.0.as_obj(), &t, &s, &d)
                    })
                }
                P::Icon(_) | P::IconTint(_) => {
                    let name = jstr(env, &list_item_icon(p.icon.as_ref()));
                    let tint = p.icon_tint.map_or(0, |c| argb_i32(c.resolved()));
                    java_in(env, |env| {
                        DayListItem::setIcon(env, h.0.as_obj(), &name, tint)
                    })
                }
                P::Checked(on) => {
                    java_in(env, |env| DayListItem::setChecked(env, h.0.as_obj(), *on))
                }
            };
            if env.exception_check() {
                env.exception_clear();
            }
        });
    }

    fn image_tint(tint: Option<day_spec::Color>, template: bool) -> i32 {
        match tint {
            Some(c) => argb_i32(c),
            None if template => TEMPLATE_TINT,
            None => 0,
        }
    }

    /// Warn once per kind that this backend has no registered renderer for `kind`, before falling
    /// back to a visible placeholder. A missing renderer usually means the piece's `material` feature
    /// wasn't enabled (Tier A.2 derives it automatically under `day build`). The message goes to
    /// both stderr (which `redirect_stdio_to_logcat` routes to logcat) and directly to logcat at
    /// `ERROR`, so it surfaces even before the redirect installs. Deduped per kind so it doesn't
    /// spam the log.
    fn warn_missing_renderer(kind: PieceKind) {
        day_spec::placeholder::report(kind, "android");
    }

    /// Realize fallback for a builtin arm whose props failed to downcast
    /// ([`day_spec::props_of`] already reported it): the same visible `⟨kind⟩` label a
    /// missing renderer shows, so one mismatched piece cannot abort the tree build.
    fn realize_placeholder(kind: PieceKind) -> AHandle {
        with_env(|env| AHandle(placeholder_view(env, kind)))
    }

    thread_local! {
        /// `BitmapId` → what the decode reported (docs/images.md). Only the METADATA lives here;
        /// the pixels are the shim's, in `DayBridge.bitmaps` under the same id.
        static BITMAP_INFO: RefCell<std::collections::HashMap<u64, day_spec::BitmapInfo>> =
            RefCell::new(std::collections::HashMap::new());
    }

    /// Re-encode a decoded bitmap through the shim (docs/images.md).
    ///
    /// A free function rather than the duty's body so each refusal reads as an early return
    /// instead of another copy of the same emit. The pixels never cross the boundary: Java
    /// compresses and hands back one `byte[]`.
    fn encode_bitmap(
        id: day_spec::BitmapId,
        spec: &day_spec::EncodeSpec,
    ) -> Result<Vec<u8>, day_spec::ImageError> {
        use day_spec::ImageFormat as F;
        let format = match spec.format {
            F::Png => "png",
            F::Jpeg => "jpeg",
            // `Bitmap.compress` writes nothing else under this minSdk; `encode_formats` says so
            // before an app asks.
            _ => return Err(day_spec::ImageError::Encode),
        };
        if !BITMAP_INFO.with(|m| m.borrow().contains_key(&id.0)) {
            return Err(day_spec::ImageError::Gone);
        }
        if !vm_ready() {
            return Err(day_spec::ImageError::Unsupported);
        }
        // Android takes quality as 0..=100; -1 means the format's own default.
        let quality = spec
            .quality
            .map_or(-1, |q| (q.clamp(0.0, 1.0) * 100.0).round() as i32);
        let (fit_w, fit_h) = spec.fit.map_or((0.0, 0.0), |f| (f.width, f.height));
        with_env(|env| {
            let jfmt = jstr(env, format);
            java_in(env, |env| {
                let arr = DayBridge::imageEncode(env, id.0 as i64, &jfmt, quality, fit_w, fit_h)?;
                if arr.is_null() {
                    return Ok(None);
                }
                env.convert_byte_array(&arr).map(Some)
            })
            .flatten()
        })
        .ok_or(day_spec::ImageError::Encode)
    }

    /// The platform's reason for the export that just failed, or `fallback` when the call
    /// itself failed before Java could record one.
    fn export_failure(env: &mut Env, fallback: &str) -> day_spec::ExportError {
        let why = java_string(env, DayCanvasView::exportError)
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| fallback.to_string());
        day_spec::ExportError::Failed(why)
    }

    /// `render_bitmap`'s body, on a painting thread (docs/canvas.md "Exporting"): the ops
    /// encoded for the wire, then one JNI call that paints and stores the bitmap, with the
    /// facts of what was stored as the answer. `with_env` attaches this thread to the JVM for
    /// the call, and the binding resolves `DayCanvasView` through the app class loader, so
    /// the call works from a thread Java never saw. A free function so each refusal is an
    /// early return.
    fn export_bitmap(
        id: day_spec::BitmapId,
        ops: &[DrawOp],
        pw: u32,
        ph: u32,
        scale: f64,
    ) -> Result<day_spec::BitmapInfo, day_spec::ExportError> {
        // day-core caps a side at 16,384 pixels, so both fit an `int`.
        let (Ok(jw), Ok(jh)) = (i32::try_from(pw), i32::try_from(ph)) else {
            return Err(day_spec::ExportError::TooLarge {
                width: pw,
                height: ph,
            });
        };
        let (nums, texts, geometry) = day_spec::encode_ops_with_geometry(ops);
        with_env(|env| {
            let refused = java_in(env, |env| {
                let arr = env.new_double_array(nums.len())?;
                arr.set_region(env, 0, &nums)?;
                let geo = env.new_double_array(geometry.len())?;
                geo.set_region(env, 0, &geometry)?;
                let joined = JString::from_str(env, texts.join("\u{1f}"))?;
                let why = DayCanvasView::renderBitmap(
                    env,
                    id.0 as i64,
                    &arr,
                    &joined,
                    &geo,
                    jw,
                    jh,
                    scale,
                )?;
                if why.is_null() {
                    return Ok(None);
                }
                Ok(Some(string_of(env, &why).unwrap_or_default()))
            });
            match refused {
                Some(None) => Ok(day_spec::BitmapInfo {
                    pixels: Size::new(f64::from(pw), f64::from(ph)),
                    scale,
                    format: None,
                    // Created ARGB_8888 on a transparent field.
                    has_alpha: true,
                }),
                Some(Some(why)) => Err(day_spec::ExportError::Failed(if why.is_empty() {
                    "the bitmap could not be created".to_string()
                } else {
                    why
                })),
                None => Err(day_spec::ExportError::Failed(
                    "the bitmap render call failed".into(),
                )),
            }
        })
    }

    /// `render_pdf`'s body. The pages cross JNI in one call: every page's numbers in one
    /// `double[]` and its texts in one joined string, with per-page counts to split them by,
    /// so no page state lives in Java between calls.
    fn export_pdf(pages: &[day_spec::PdfPage]) -> Result<Vec<u8>, day_spec::ExportError> {
        if !vm_ready() {
            return Err(day_spec::ExportError::Unsupported);
        }
        let mut nums = Vec::new();
        let mut texts = Vec::new();
        let mut num_counts = Vec::with_capacity(pages.len());
        let mut text_counts = Vec::with_capacity(pages.len());
        let mut sizes = Vec::with_capacity(pages.len() * 2);
        for page in pages {
            let (n, t) = day_spec::encode_ops(&page.ops);
            let counts = (i32::try_from(n.len()), i32::try_from(t.len()));
            let (Ok(nc), Ok(tc)) = counts else {
                return Err(day_spec::ExportError::Invalid);
            };
            num_counts.push(nc);
            text_counts.push(tc);
            nums.extend_from_slice(&n);
            texts.extend(t);
            sizes.push(page.size.width);
            sizes.push(page.size.height);
        }
        with_env(|env| {
            let bytes = java_in(env, |env| {
                let jnums = env.new_double_array(nums.len())?;
                jnums.set_region(env, 0, &nums)?;
                let jnum_counts = env.new_int_array(num_counts.len())?;
                jnum_counts.set_region(env, 0, &num_counts)?;
                let jtext_counts = env.new_int_array(text_counts.len())?;
                jtext_counts.set_region(env, 0, &text_counts)?;
                let jsizes = env.new_double_array(sizes.len())?;
                jsizes.set_region(env, 0, &sizes)?;
                let joined = JString::from_str(env, texts.join("\u{1f}"))?;
                let arr = DayCanvasView::renderPdf(
                    env,
                    &jnums,
                    &jnum_counts,
                    &joined,
                    &jtext_counts,
                    &jsizes,
                )?;
                if arr.is_null() {
                    return Ok(None);
                }
                env.convert_byte_array(&arr).map(Some)
            });
            match bytes {
                Some(Some(pdf)) => Ok(pdf),
                Some(None) => Err(export_failure(env, "the PDF could not be written")),
                None => Err(day_spec::ExportError::Failed(
                    "the PDF render call failed".into(),
                )),
            }
        })
    }

    /// Ask the bridge for a PNG of this app's window (docs/window-image.md).
    ///
    /// The bytes come back as a Java `byte[]` and are copied out with `convert_byte_array`: no
    /// base64 round trip for what is already binary.
    fn android_window_image(chrome: bool) -> Result<Vec<u8>, String> {
        with_env(|env| {
            let arr = DayBridge::windowImage(env, chrome).map_err(|e| {
                env.exception_clear();
                format!("windowImage: {e}")
            })?;
            if arr.is_null() {
                return Err("no window to capture".to_string());
            }
            env.convert_byte_array(&arr)
                .map_err(|e| format!("windowImage: {e}"))
        })
    }

    impl Toolkit for Material {
        type Handle = AHandle;

        fn set_drag_source(&mut self, h: &AHandle, source: day_spec::transfer::Source) {
            TRANSFER_SOURCES.with(|t| t.insert(h.0.as_obj().as_raw() as usize, source));
            transfer_install(h, true);
        }
        fn set_drop_target(&mut self, h: &AHandle, target: day_spec::transfer::Target) {
            TRANSFER_TARGETS.with(|t| t.insert(h.0.as_obj().as_raw() as usize, target));
            transfer_install(h, false);
        }
        #[expect(
            clippy::match_same_arms,
            reason = "one arm per capability, each documented with the Android mechanism behind it"
        )]
        fn capability(&self, cap: Cap) -> Support {
            match cap {
            // `ChooserAction`s on the system chooser, which Android 14 (API 34) introduced.
            Cap::ShareActions if day_platform_android::sdk_int() >= 34 => Support::Native,
            Cap::ShareActions => Support::Unsupported,
            Cap::SearchField => Support::Native,
            Cap::ComboBox => Support::Native,
            Cap::Stepper => Support::Emulated,
            Cap::DateTimePicker => Support::Native,
            Cap::ColorPicker => Support::Emulated,
            Cap::PullRefresh => Support::Native,
            Cap::Pager => Support::Native,
            Cap::PullRefreshGesture => Support::Native,
                Cap::DragDrop | Cap::DragExternalImport | Cap::DragExternalExport | Cap::DragMultipleItems => Support::Native,
                // `View.setPointerIcon` with a system `PointerIcon` per view (docs/cursor.md).
                Cap::Cursor => Support::Native,
                // `.focusable()` (DayBridge.enableKeys): focusable in touch mode, focus on a
                // primary press, both focus reports, the short and full key routes, the
                // platform's default focus highlight for keyboard focus (docs/focus.md).
                Cap::Focusable => Support::Native,
                // `KeyEvent.getRepeatCount` for repeats and `ACTION_UP` for releases, each
                // only for the node its down was delivered to (DayCanvasView.sendKey).
                Cap::KeyPhases => Support::Native,
                // The parent's TouchDelegate (DayBridge `SlopDelegate`); it cannot reach past
                // the parent's own bounds (docs/gestures.md "Toolkit support").
                Cap::HitSlop => Support::Native,
                // The families `fonts.xml` names, the ones `Typeface.create` resolves
                // (docs/fonts.md).
                Cap::FontList => Support::Native,
                // DayListItem: a Material 3 list item (docs/list.md "List items").
                Cap::ListItem => Support::Native,
                // RecyclerView has no sticky headers: header rows take their own view type, and
                // DayListView pins a copy of the current one in the list's overlay, pushed up by
                // the next (docs/list.md "Sections").
                Cap::ListSections => Support::Emulated,
                // `RecyclerView.OnScrollListener` and every layout pass report the first and
                // last visible rows (`findFirst/LastVisibleItemPosition`) and the distance to
                // the end (docs/list.md "Visible rows").
                Cap::ListViewport => Support::Native,
                // DayListView reports the first move of a drag (`SCROLL_STATE_DRAGGING`, which
                // a touch that catches a glide also enters) and of the scroll a wheel, a
                // scrolling key or an accessibility scroll action starts; Day's own scrolls
                // settle without dragging (docs/list.md "Following the end").
                Cap::ListReaderScroll => Support::Native,
                // `scrollToPositionWithOffset` for a jump, RecyclerView's `smoothScrollBy` over
                // the aligned distance for an animation (docs/list.md "Programmatic scrolling").
                Cap::ListScrollAlign => Support::Native,
                // `onScrollChanged`, one report per frame plus the resting one (docs/scroll.md).
                Cap::ScrollOffset => Support::Native,
                // ScrollView has no scroll-state callback; its subclass reads the moments from
                // its own touch stream and `fling()`: a drag moving the content, a fling or
                // snap after the lift, the content still for four frames (docs/scroll.md).
                Cap::ScrollPhases => Support::Native,
                // ScrollView does not snap: its subclass takes over `fling()` and the release
                // and `smoothScrollTo`s the stop the fling would reach, rounded to the page or
                // the interval (docs/scroll.md "Paging and snapping").
                Cap::ScrollSnap => Support::Emulated,
                // The IME is hidden (`hideSoftInputFromWindow`, focus cleared) when a drag
                // begins; `KeyboardTaps::Dismiss` intercepts a tap off a text field while it is
                // up. No interactive mode: it behaves as `OnDrag` (docs/scroll.md "Keyboard").
                Cap::ScrollKeyboardDismiss => Support::Native,
                // No Android scroll view zooms; `ScrollProps::zoom` and `ScrollPatch::Zoom` are
                // ignored (docs/scroll.md "Zoom").
                Cap::ScrollZoom => Support::Unsupported,
                // ViewPager's `onPageScrolled` (docs/pager.md "Progress").
                Cap::PagerProgress => Support::Native,
                // `BitmapFactory` decodes every container Android reads, and `Bitmap.compress`
                // writes back PNG and JPEG (docs/images.md). WebP decodes here but is not
                // offered for encode: the modern `WEBP_LOSSY`/`WEBP_LOSSLESS` constants are
                // API 30 and the scaffold's minSdk is 24. `Cap::ImageProperties` is
                // absent because there is no metadata reader on this path, and an empty struct
                // would read as "this file records nothing" rather than "nobody looked".
                Cap::ImageDecode | Cap::ImageEncode => Support::Native,
                // Canvas exports paint with DayCanvasView's own OpPainter: into a `Bitmap` for the
                // bitmap, onto `PdfDocument` pages for the PDF (docs/canvas.md "Exporting").
                Cap::CanvasBitmap | Cap::CanvasPdf => Support::Native,
                // `View.draw(Canvas)` renders this app's own window into a bitmap
                // (docs/window-image.md); surface-backed content is the documented gap.
                Cap::Snapshot => Support::Native,
                // An app-level light/dark override, through `UiModeManager` (API 31 and up).
                // Answered from the device rather than pinned for the backend: below 31 the only
                // route is AppCompat's delegate, which does not restyle the plain
                // `FragmentActivity` Day runs in, so there the answer is that there is no such
                // control, and `day-piece-settings` draws no appearance row rather than one
                // that does nothing.
                Cap::Appearance => {
                    if with_env(|env| {
                        DayBridge::canSetAppearance(env)
                        .unwrap_or(false)
                    }) {
                        Support::Native
                    } else {
                        Support::Unsupported
                    }
                }
                // SpannableString spans, drawn by the one TextView (docs/text-runs.md).
                // A link run is a ClickableSpan; the TextView takes LinkMovementMethod when one
                // is present, which is what makes the tap land.
                Cap::TextRuns | Cap::TextLinks => Support::Native,
                // A selectable label is a `DayLabel` (a TextView) whose `onSelectionChanged`
                // reports each new range (docs/text.md "Hearing about the selection").
                Cap::LabelSelection => Support::Native,
                // `announceForAccessibility` on the window's content view: the platform's
                // announcement event, sent only while an accessibility service is enabled
                // (docs/accessibility.md).
                Cap::Announce => Support::Native,
                // The global animator duration scale, zero under Accessibility's "Remove
                // animations", read from the settings provider and watched through a
                // ContentObserver (docs/accessibility.md).
                Cap::ReduceMotion => Support::Native,
                // EditText honors editable / selectable / spell-check (DayTextArea shim).
                Cap::Dialogs
                | Cap::FileDialogs
                | Cap::Animation
                | Cap::Cover
                // WindowInsetsControllerCompat hides the status bar (docs/cover.md).
                | Cap::StatusBarHidden
                // The MaterialToolbar names the destination on every page (DayNavHost
                // syncChrome), so content needn't repeat the title (docs/navigation.md).
                | Cap::NavHeader
                // The window toolbar goes in the nav host's app bar (docs/toolbars.md), as
                // menu items shown as actions: one bar per window, at every width, with what
                // the bar cannot fit folding into its overflow.
                | Cap::Toolbar
                // A SlidingPaneLayout hosts every `nav(Sidebar)`, so two panes are
                // available wherever they fit: a tablet, a foldable open, a phone in landscape
                // if the widths allow (docs/size-classes.md).
                | Cap::NavSplit
                | Cap::TextEditable
                | Cap::TextSelectable
                | Cap::TextSpellCheck
                // ItemTouchHelper on the RecyclerView list: long-press lift, elevation,
                // incremental swaps: the platform's own reorder (docs/list.md).
                | Cap::ListReorder
                // RecyclerView's holder pool: `onBindViewHolder` rebinds Day's cells for new
                // rows and `onViewRecycled` parks them (docs/list.md).
                | Cap::ListRecycling
                // ItemTouchHelper's swipe half, with the Material red field revealing behind
                // the row (docs/list.md).
                | Cap::ListDelete
                // The window's insets controller hides the system bars, swipe to reveal them
                // transiently; the state reports once the bars have moved (docs/windows.md).
                | Cap::WindowFullscreen
                // FLAG_SECURE on the window's activity: screenshots, recordings, casting and
                // the recents thumbnail see black (docs/windows.md). `Cap::WindowStates` stays
                // Unsupported: an app cannot minimize or maximize its own window here, so
                // those requests report the state the window kept.
                | Cap::ContentProtection
                // Document-style DayWindowActivity instances (docs/windows.md): separate
                // recents entries; side-by-side in split-screen/freeform/desktop windowing.
                | Cap::MultiWindow
                // View.getBaseline(), the platform's own answer (docs/baseline.md).
                | Cap::BaselineAlignment
                // The navigation suite (DayTabs): a BottomNavigationView, a NavigationRailView or
                // a permanent NavigationView drawer over resident pages: Material's own
                // destination chrome, in the form the width calls for (docs/navigation.md).
                | Cap::NavTabs
                // And Android should grow one as it narrows: a bottom bar is the idiomatic
                // compact answer here, the way it is on iOS and unlike any desktop.
                | Cap::NavTabsAdaptive => Support::Native,
                // Keep navigation on Material NavigationView. A reorderable RecyclerView
                // substitutes generic rows and its long-press drag steals feed menus.
                // Generic lists still support reordering; navigation needs an edit-mode API.
                Cap::NavReorder => Support::Unsupported,
                // `Emulated`: SlidingPaneLayout decides at measure time whether both panes fit, so
                // the platform owns the presentation and Day observes it through
                // `Event::NavPresentationChanged` rather than pushing one in
                // (docs/size-classes.md).
                Cap::NavRepresent => Support::Emulated,
                // The display the primary window is on: one, however many are attached
                // (docs/windows.md).
                Cap::Monitors => Support::Emulated,
                // ShortcutManager's dynamic shortcuts, from API 25 (docs/deep-links.md).
                Cap::DynamicShortcuts => {
                    if with_env(|env| {
                        DayBridge::canSetLauncherShortcuts(env)
                            .unwrap_or(false)
                    }) {
                        Support::Native
                    } else {
                        Support::Unsupported
                    }
                }
                // The composed tree (docs/tree.md M2/M5): the piece flattens onto this
                // backend's RecyclerView list; disclosure, indentation and row content are
                // day pieces. No native drag wiring yet, so `Cap::TreeMove` stays
                // Unsupported (`tree_move:` drives the move synthetically).
                Cap::Tree => Support::Emulated,
                Cap::TreeSections => Support::Native,
                _ => Support::Unsupported,
            }
        }

        fn defer_system_gestures(&mut self, edges: day_spec::Edges) {
            // Any non-empty request enters swipe-to-reveal immersive mode (docs/cover.md).
            bridge(|env| DayBridge::setDeferSystemGestures(env, !edges.is_empty()));
        }

        fn set_status_bar_hidden(&mut self, hidden: bool) {
            // The insets controller hides the status bar; the edge-to-edge root then sees the
            // smaller top inset through its usual inset listener (docs/cover.md).
            bridge(|env| DayBridge::setStatusBarHidden(env, hidden));
        }

        fn present(&mut self, req: u64, spec: &day_spec::present::PresentSpec) {
            use day_spec::present::PresentSpec;
            let reqj = req as i64;
            match spec {
                PresentSpec::Dialog { sheet, .. } => with_env(|env| {
                    let title = jstr(env, spec.title());
                    let message = jstr(env, spec.message().unwrap_or(""));
                    let buttons = jstr(env, &spec.buttons_joined());
                    let roles = jstr(env, &spec.roles_joined());
                    let _ = java_in(env, |env| {
                        DayBridge::present(env, reqj, *sheet, &title, &message, &buttons, &roles)
                    });
                }),
                PresentSpec::Prompt {
                    placeholder,
                    initial,
                    ok,
                    cancel,
                    ..
                } => with_env(|env| {
                    let title = jstr(env, spec.title());
                    let message = jstr(env, spec.message().unwrap_or(""));
                    let ph = jstr(env, placeholder);
                    let init = jstr(env, initial);
                    let okj = jstr(env, ok);
                    let cancelj = jstr(env, cancel);
                    let _ = java_in(env, |env| {
                        DayBridge::presentPrompt(
                            env, reqj, &title, &message, &ph, &init, &okj, &cancelj,
                        )
                    });
                }),
                // Storage Access Framework (docs/files.md). Java launches ACTION_OPEN_DOCUMENT /
                // ACTION_CREATE_DOCUMENT and, on result, copies through the ContentResolver: open →
                // an app cache file (readable path); save → the chosen content:// URI.
                PresentSpec::OpenFile { multiple, .. } => with_env(|env| {
                    let title = jstr(env, spec.title());
                    let filters = jstr(env, &spec.filters_joined());
                    let _ = java_in(env, |env| {
                        DayBridge::presentFileOpen(env, reqj, &title, &filters, *multiple)
                    });
                }),
                // No folder chooser on Android (`Cap::FolderDialog` is Unsupported): a tree
                // URI's persisted access is its own feature, so the request resolves at once.
                PresentSpec::OpenFolder { .. } => emit(
                    day_spec::WINDOW_NODE,
                    Event::PresentResult {
                        req,
                        result: day_spec::present::PresentResult::Dismissed,
                    },
                ),
                PresentSpec::SaveFile {
                    suggested_name,
                    src_path,
                    ..
                } => with_env(|env| {
                    let title = jstr(env, spec.title());
                    let name = jstr(env, suggested_name);
                    let src = jstr(env, src_path);
                    let filters = jstr(env, &spec.filters_joined());
                    let _ = java_in(env, |env| {
                        DayBridge::presentFileSave(env, reqj, &title, &name, &src, &filters)
                    });
                }),
            }
        }

        fn dismiss(&mut self, req: u64) {
            bridge(|env| DayBridge::dismissPresent(env, req as i64));
        }

        fn share_support(&self) -> day_spec::Support {
            day_spec::Support::Native
        }
        fn share(
            &mut self,
            spec: &day_spec::share::ShareSpec,
            _anchor: Option<&Self::Handle>,
        ) -> bool {
            use std::fmt::Write as _;
            // The app's actions cross as lines of `id \t drawable \t title`; the drawable is
            // the action's symbol (`day_symbol_*`), or empty for the chooser's default glyph.
            let actions = spec.actions.iter().fold(String::new(), |mut out, a| {
                let title: String = a
                    .title
                    .chars()
                    .map(|c| if c == '\t' || c == '\n' { ' ' } else { c })
                    .collect();
                let icon = a.symbol.map_or_default(symbol_drawable);
                let _ = writeln!(out, "{}\t{icon}\t{title}", a.id);
                out
            });
            with_env(|env| {
                let url = jstr(env, &spec.url);
                let title = jstr(env, &spec.title);
                let actions = jstr(env, &actions);
                java_in(env, |env| DayBridge::share(env, &url, &title, &actions)).unwrap_or(false)
            })
        }

        fn open_url(&mut self, url: &str) {
            with_env(|env| {
                let u = jstr(env, url);
                let _ = java_in(env, |env| DayBridge::openUrl(env, &u));
            });
        }

        fn realize(&mut self, kind: PieceKind, props: &dyn Any, id: NodeId) -> AHandle {
            let idj = id.0 as i64;
            match Builtin::from_key(kind) {
                Some(Builtin::Container) => {
                    let h = with_env(|env| {
                        AHandle(bridge_view(env, "makeContainer", |env| {
                            DayBridge::makeContainer(env)
                        }))
                    });
                    if let Some(p) = props.downcast_ref::<ContainerProps>() {
                        if p.role == Some(day_spec::SurfaceRole::SectionCard) {
                            let d = DENSITY.with(std::cell::Cell::get);
                            bridge(|env| {
                                DayBridge::setSectionCard(
                                    env,
                                    h.0.as_obj(),
                                    (p.corner_radius * d) as f32,
                                )
                            });
                        } else if p.role == Some(day_spec::SurfaceRole::Window) {
                            let d = DENSITY.with(std::cell::Cell::get);
                            bridge(|env| {
                                DayBridge::setWindowSurface(
                                    env,
                                    h.0.as_obj(),
                                    (p.corner_radius * d) as f32,
                                    p.clips,
                                )
                            });
                        } else {
                            apply_container_surface(&h, p, true);
                        }
                    }
                    h
                }
                Some(Builtin::Scroll) => {
                    // The preferences ride one flag word (docs/scroll.md; `list_wire`): Android
                    // has no zoom, so `zoom` is left out (`Cap::ScrollZoom` is Unsupported).
                    let p = props.downcast_ref::<day_spec::props::ScrollProps>();
                    let horizontal = p.is_some_and(|p| p.horizontal);
                    let (flags, snap_px) =
                        p.map_or((0, 0.0), |p| crate::list_wire::scroll_flags(p, density()));
                    with_env(|env| {
                        AHandle(bridge_view(env, "makeScroll", |env| {
                            DayBridge::makeScroll(env, horizontal, flags, snap_px, id.0 as i64)
                        }))
                    })
                }
                Some(Builtin::Tree) => {
                    let Some(p) = day_spec::props_of::<TreeProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    let height = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 36.0,
                    };
                    let handle = with_env(|env| {
                        AHandle(bridge_view(env, "makeTree", |env| {
                            DayBridge::makeTree(
                                env,
                                id.0 as i64,
                                (height * DENSITY.with(std::cell::Cell::get)).round() as i32,
                                p.selectable,
                            )
                        }))
                    });
                    LIST_NODE.with(|m| {
                        m.borrow_mut()
                            .insert(handle.0.as_obj().as_raw() as usize, id.0 as i64)
                    });
                    handle
                }
                Some(Builtin::List) => {
                    let Some(p) = day_spec::props_of::<ListProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    let d = DENSITY.with(std::cell::Cell::get);
                    let rowh = match p.row_height {
                        RowHeight::Uniform(h) => h,
                        RowHeight::Automatic => 44.0,
                    };
                    let handle = with_env(|env| {
                        let del_label = jstr(env, &p.delete_label);
                        AHandle(bridge_view(env, "makeList", |env| {
                            DayBridge::makeList(
                                env,
                                id.0 as i64,
                                (rowh * d).round() as i32,
                                p.selectable,
                                p.reorderable,
                                p.deletable,
                                &del_label,
                            )
                        }))
                    });
                    LIST_NODE.with(|m| {
                        m.borrow_mut()
                            .insert(handle.0.as_obj().as_raw() as usize, id.0 as i64)
                    });
                    if p.row_height == RowHeight::Automatic {
                        LIST_AUTOMATIC.with(|m| m.borrow_mut().insert(id.0 as i64));
                    }
                    // Before the first bind (attach_list reloads): headers take their own view
                    // type from the start (docs/list.md "Sections").
                    if p.sections {
                        bridge(|env| DayBridge::listSetSections(env, handle.0.as_obj(), true));
                    }
                    handle
                }
                Some(Builtin::Nav) => {
                    let Some(p) = day_spec::props_of::<NavProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    // Where the rows are the chrome, the host is a navigation suite rather than a
                    // pane layout: pages resident behind a bottom bar, a rail or a permanent
                    // drawer, whichever the width calls for (DayTabs). One container across every
                    // width is what lets the host report `Tabs` once and keep it, so day-core
                    // holds the pages and drives them with `Select` instead of push/pop, the
                    // same contract `.tabSidebar` gives on iOS.
                    if p.presentation.rows_are_chrome() {
                        let host = with_env(|env| {
                            AHandle(bridge_view(env, "makeNavSuite", |env| {
                                DayBridge::makeNavSuite(env, idj, 0)
                            }))
                        });
                        if let Some(config) = &p.documents {
                            with_env(|env| {
                                let new_label = jstr(env, &config.new_label);
                                let close_label = jstr(env, &config.close_label);
                                let _ = java_in(env, |env| {
                                    DayBridge::documentNavSuite(
                                        env,
                                        host.0.as_obj(),
                                        &new_label,
                                        &close_label,
                                        config.can_add,
                                    )
                                });
                            });
                        }
                        NAV_SUITES
                            .with(|m| m.borrow_mut().insert(host.0.as_obj().as_raw() as usize));
                        // Tell day-core what it got. The suite changes its chrome as it resizes
                        // but never its presentation, so this is reported once and never revised.
                        emit(
                            id,
                            Event::NavPresentationChanged(day_spec::props::NavPresentation::Tabs),
                        );
                        return host;
                    }
                    // `Stack` in props is literal, a host that is a stack at every size (a
                    // nested `nav_stack()` under a split host, docs/size-classes.md), so it gets a
                    // plain single-pane host. Only an adaptive host builds a SlidingPaneLayout;
                    // nesting one inside a pane re-runs the whole tiling decision at pane width.
                    let adaptive = p.presentation != day_spec::props::NavPresentation::Stack;
                    let host = with_env(|env| {
                        let s = jstr(env, &p.title);
                        // The tiling threshold comes from Day's breakpoint table, so the platform's
                        // measure-time answer agrees with it.
                        let split = day_spec::SizeClass::SPLIT_MIN_WIDTH as f32;
                        AHandle(bridge_view(env, "makeNavHost", |env| {
                            DayBridge::makeNavHost(env, idj, &s, adaptive, split)
                        }))
                    });
                    // Inline search (docs/search.md): the field above the navigation list. The app
                    // bar holds actions, never a field (`Cap::ToolbarSearch` is unsupported), so
                    // every placement resolves here. Applied after the host is built, and
                    // best-effort, since a throw on the host's own build path blanks the app.
                    if let Some(sp) = p
                        .search
                        .as_ref()
                        .filter(|sp| sp.placement == day_spec::props::SearchPlacement::Inline)
                    {
                        let (prompt, text) = (sp.prompt.clone(), sp.text.clone());
                        with_env(|env| {
                            let pr = jstr(env, &prompt);
                            let tx = jstr(env, &text);
                            let _ = java_in(env, |env| {
                                DayBridge::setNavSearch(
                                    env,
                                    host.0.as_obj(),
                                    idj,
                                    &pr,
                                    &tx,
                                    sp.hides_on_scroll,
                                    sp.initially_hidden,
                                )
                            });
                            if env.exception_check() {
                                env.exception_clear();
                            }
                        });
                    }
                    host
                }
                Some(Builtin::NavPage) => with_env(|env| {
                    let h = AHandle(bridge_view(env, "makeNavPage", |env| {
                        DayBridge::makeNavPage(
                            env,
                            idj,
                            day_spec::props_of::<NavPageProps>(kind, "android", props)
                                .is_some_and(|p| p.immersive),
                        )
                    }));
                    // A sidebar page inside a navigation suite is the page whose rows BECAME the
                    // chrome, so the suite keeps it attached but never shows it (drawing the rows
                    // again as a list would be the same navigation twice). Marked here because
                    // `insert` sees only handles; harmless anywhere else, where nothing reads it.
                    let sidebar = day_spec::props_of::<NavPageProps>(kind, "android", props)
                        .is_some_and(|p| p.pane == day_spec::props::Pane::Sidebar);
                    if sidebar {
                        let _ = java_in(env, |env| DayBridge::markNavChromePage(env, h.0.as_obj()));
                    }
                    h
                }),
                // Fullscreen cover (docs/cover.md): a DayCover shell whose content pane is the
                // Day mount point; CoverPatch::Present re-homes it over the activity content.
                // A half sheet (`CoverProps::half`) is the same shell as a Material bottom
                // sheet over the bottom half of the window.
                Some(Builtin::Cover) => {
                    let half = day_spec::props_of::<CoverProps>(kind, "android", props)
                        .is_some_and(|p| p.half);
                    with_env(|env| {
                        AHandle(bridge_view(env, "makeCover", |env| {
                            DayBridge::makeCover(env, idj, half)
                        }))
                    })
                }
                Some(Builtin::NavMenu) => {
                    let Some(p) = day_spec::props_of::<NavMenuProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    let joined = p.items.join("\u{1f}");
                    // Parallel, index-aligned icon NAMES ("" = no icon for that row).
                    let joined_icons = p
                        .icons
                        .iter()
                        .map(|o| o.clone().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\u{1f}");
                    let joined_tints = nav_tints_joined(
                        &p.tints
                            .iter()
                            .map(|t| t.map(day_spec::Ink::resolved))
                            .collect::<Vec<_>>(),
                    );
                    let joined_badge_icons = p
                        .badge_icons
                        .iter()
                        .map(|o| o.clone().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\u{1f}");
                    let joined_badge_tints = nav_tints_joined(
                        &p.badge_tints
                            .iter()
                            .map(|t| t.map(day_spec::Ink::resolved))
                            .collect::<Vec<_>>(),
                    );
                    let joined_menus = nav_menus_joined(&p.menus);
                    // Headings, index-aligned with the rows ("" continues the current group).
                    // NavigationView draws them as M3 subheaders; the flat list this replaced
                    // dropped them, so the showcase's eight groups arrived as twenty bare rows.
                    let joined_sections = p
                        .sections
                        .iter()
                        .map(|o| o.clone().unwrap_or_default())
                        .collect::<Vec<_>>()
                        .join("\u{1f}");
                    with_env(|env| {
                        let s = jstr(env, &joined);
                        let si = jstr(env, &joined_icons);
                        let ss = jstr(env, &joined_sections);
                        let handle = AHandle(bridge_view(env, "makeNavMenu", |env| {
                            DayBridge::makeNavMenu(env, idj, &s, &si, &ss)
                        }));
                        // Keep the rows for `insert`, which is where a navigation suite above
                        // this menu can finally be found and handed them.
                        let texts = p
                            .badges
                            .iter()
                            .map(|s| s.as_deref().unwrap_or(""))
                            .collect::<Vec<_>>()
                            .join("\u{1f}");
                        NAV_MENU_ROWS.with(|m| {
                            m.borrow_mut().insert(
                                handle.0.as_obj().as_raw() as usize,
                                (
                                    idj,
                                    joined.clone(),
                                    joined_icons.clone(),
                                    p.document_keys.join("\u{1f}"),
                                    texts.clone(),
                                ),
                            )
                        });
                        // Per-row icon tints ride a best-effort follow-up (docs/vectors.md):
                        // decoration stays off makeNavMenu's critical path, so a tint problem
                        // can never abort the tree build (the navhost lesson).
                        let st = jstr(env, &joined_tints);
                        let _ = java_in(env, |env| {
                            DayBridge::setNavMenuTints(env, handle.0.as_obj(), &st)
                        });
                        // The trailing status glyph (docs/navigation.md), same best-effort rule.
                        let counts = jstr(env, &texts);
                        let sb = jstr(env, &joined_badge_icons);
                        let sbt = jstr(env, &joined_badge_tints);
                        let _ = java_in(env, |env| {
                            DayBridge::setNavMenuBadges(env, handle.0.as_obj(), &sb, &sbt, &counts)
                        });
                        // The active indicator the model already carries: a rebuilt sidebar
                        // (a language change, a data-driven item set) has to come back marking
                        // the same page rather than blank.
                        nav_menu_select(env, &handle, p.selected);
                        // Per-row context menus (docs/menus.md): same best-effort follow-up.
                        let sm = jstr(env, &joined_menus);
                        let _ = java_in(env, |env| {
                            DayBridge::setNavRowMenus(env, handle.0.as_obj(), &sm)
                        });
                        if env.exception_check() {
                            env.exception_clear();
                        }
                        handle
                    })
                }
                Some(Builtin::Label) => {
                    let Some(p) = day_spec::props_of::<LabelProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    with_env(|env| {
                        let s = jstr(env, &p.text);
                        let view =
                            bridge_view(env, "makeLabel", |env| DayBridge::makeLabel(env, &s));
                        // The font first: a line height is a multiple of its size.
                        apply_label_font(env, &view, p.font);
                        if !p.wraps {
                            let _ = java_in(env, |env| {
                                DayBridge::setLabelSingleLine(env, view.as_obj())
                            });
                        }
                        let paragraph = p.paragraph();
                        if paragraph != day_spec::props::LabelParagraph::default() {
                            apply_label_paragraph(env, &view, &paragraph);
                        }
                        if let Some(col) = p.color {
                            let _ = java_in(env, |env| {
                                DayBridge::setLabelColor(
                                    env,
                                    view.as_obj(),
                                    argb_i32(col.resolved()),
                                    true,
                                )
                            });
                        } else if p.role == day_spec::props::TextRole::Secondary {
                            // The theme's own secondary color, resolved on the Java side so it
                            // follows the app's theme and the system's light/dark setting.
                            let _ = java_in(env, |env| {
                                DayBridge::setLabelSecondary(env, view.as_obj(), true)
                            });
                        }
                        // A label patch carries no node id, and a link's ClickableSpan needs one
                        // to report through, so remember it here, where the id is in hand.
                        LABEL_NODE
                            .with(|m| m.borrow_mut().insert(view.as_obj().as_raw() as usize, idj));
                        if !p.runs.is_empty() {
                            set_label_runs(env, idj, &view, &p.text, &p.runs);
                        }
                        let align = crate::text_wire::align_code(p.align);
                        if align != 0 {
                            let _ = java_in(env, |env| {
                                DayBridge::setLabelAlignment(env, view.as_obj(), align)
                            });
                        }
                        // `Label::on_selection` (docs/text.md "Hearing about the selection"):
                        // `set_selectable` follows right after realize.
                        if p.reports_selection {
                            let _ = java_in(env, |env| {
                                DayBridge::setLabelReportsSelection(env, view.as_obj(), idj)
                            });
                        }
                        AHandle(view)
                    })
                }
                Some(Builtin::Button) => {
                    let Some(p) = day_spec::props_of::<ButtonProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    with_env(|env| {
                        let s = jstr(env, &p.title);
                        let v = bridge_view(env, "makeButton", |env| {
                            DayBridge::makeButton(env, idj, &s)
                        });
                        let _ =
                            java_in(env, |env| DayBridge::setEnabled(env, v.as_obj(), p.enabled));
                        let _ = java_in(env, |env| {
                            DayBridge::setButtonSelected(env, v.as_obj(), p.selected)
                        });
                        apply_button_style(env, &v, p.style);
                        if p.icon.is_some() {
                            apply_button_content(env, &v, &p.title, p.icon.as_ref(), p.icon_only);
                        }
                        AHandle(v)
                    })
                }
                Some(Builtin::Toggle) => {
                    let Some(p) = day_spec::props_of::<ToggleProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    with_env(|env| {
                        AHandle(bridge_view(env, "makeToggle", |env| {
                            DayBridge::makeToggle(env, idj, p.on, p.enabled)
                        }))
                    })
                }
                Some(Builtin::Slider) => {
                    let Some(p) = day_spec::props_of::<SliderProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    with_env(|env| {
                        AHandle(bridge_view(env, "makeSlider", |env| {
                            DayBridge::makeSlider(env, idj, p.value, p.min, p.max)
                        }))
                    })
                }
                Some(Builtin::Picker) => crate::picker::realize_any(self, props, id),
                Some(Builtin::TextArea) => crate::textarea::realize_any(self, props, id),
                Some(Builtin::TextField) => {
                    let Some(p) = day_spec::props_of::<TextFieldProps>(kind, "android", props)
                    else {
                        return realize_placeholder(kind);
                    };
                    with_env(|env| {
                        let v = jstr(env, &p.text);
                        let ph = jstr(env, &p.placeholder);
                        AHandle(bridge_view(env, "makeTextField", |env| {
                            DayBridge::makeTextField(env, idj, &v, &ph)
                        }))
                    })
                }
                Some(Builtin::Divider) => {
                    let vertical = day_spec::props_of::<DividerProps>(kind, "android", props)
                        .is_some_and(|p| p.vertical);
                    with_env(|env| {
                        AHandle(bridge_view(env, "makeDivider", |env| {
                            DayBridge::makeDivider(env, vertical)
                        }))
                    })
                }
                Some(Builtin::Progress) => {
                    let Some(p) = day_spec::props_of::<ProgressProps>(kind, "android", props)
                    else {
                        return realize_placeholder(kind);
                    };
                    if p.value.is_none() {
                        return crate::controls::spinner::make(self, p, id);
                    }
                    with_env(|env| {
                        AHandle(bridge_view(env, "makeProgress", |env| {
                            DayBridge::makeProgress(env, p.value.is_some(), p.value.unwrap_or(0.0))
                        }))
                    })
                }
                Some(Builtin::Canvas) => with_env(|env| {
                    AHandle(bridge_view(env, "makeCanvas", |env| {
                        DayBridge::makeCanvas(env, idj)
                    }))
                }),
                Some(Builtin::Image) => {
                    let Some(p) = day_spec::props_of::<ImageProps>(kind, "android", props) else {
                        return realize_placeholder(kind);
                    };
                    // Scaling (§18.3): 0=fit (FIT_CENTER), 1=fill (CENTER_CROP), 2=stretch (FIT_XY).
                    let mode = match p.content_mode {
                        ContentMode::Fit => 0,
                        ContentMode::Fill => 1,
                        ContentMode::Stretch => 2,
                    };
                    // Vector-glyph tint (docs/vectors.md) as ARGB; 0 = none (a real tint always
                    // has alpha 0xFF, so 0 is unambiguous), and `TEMPLATE_TINT` asks for the
                    // theme's primary text color — a template glyph with no tint of its own.
                    let tint = image_tint(p.tint.map(day_spec::Ink::resolved), p.template);
                    // Named is the staged-drawable path; Bytes and Decoded arrive from
                    // `day::decode_image` (docs/images.md), so an `image()` piece can show a
                    // download or a pasted PNG with no staged resource behind it. Every source
                    // takes the tint (docs/images.md "Tinting"): it is the image view's tint
                    // list, which recolors whatever drawable the view holds by its alpha.
                    let handle = with_env(|env| match &p.source {
                        day_spec::ImageSource::Named(named) => {
                            let s = jstr(env, named);
                            AHandle(bridge_view(env, "makeImage", |env| {
                                DayBridge::makeImage(env, &s, mode, tint)
                            }))
                        }
                        day_spec::ImageSource::Bytes(bytes) => {
                            let Ok(arr) = env.byte_array_from_slice(bytes) else {
                                return AHandle(bridge_view(env, "makeImageBytes", |env| {
                                    DayBridge::makeImageBytes(
                                        env,
                                        jni::objects::JByteArray::default(),
                                        mode,
                                    )
                                }));
                            };
                            AHandle(bridge_view(env, "makeImageBytes", |env| {
                                DayBridge::makeImageBytes(env, &arr, mode)
                            }))
                        }
                        day_spec::ImageSource::Decoded(id) => {
                            AHandle(bridge_view(env, "makeImageBitmap", |env| {
                                DayBridge::makeImageBitmap(env, id.0 as i64, mode)
                            }))
                        }
                    });
                    if tint != 0 && !matches!(p.source, day_spec::ImageSource::Named(_)) {
                        bridge(|env| DayBridge::initImageTint(env, handle.0.as_obj(), tint));
                    }
                    handle
                }
                Some(Builtin::ListItem) => {
                    let Some(p) = day_spec::props_of::<day_spec::props::ListItemProps>(
                        kind, "android", props,
                    ) else {
                        return realize_placeholder(kind);
                    };
                    let handle = with_env(|env| {
                        let (t, s, d) = (
                            jstr(env, &p.title),
                            jstr(env, &p.subtitle),
                            jstr(env, &p.detail),
                        );
                        let icon = jstr(env, &list_item_icon(p.icon.as_ref()));
                        let tint = p.icon_tint.map_or(0, |c| argb_i32(c.resolved()));
                        AHandle(bridge_view(env, "makeListItem", |env| {
                            DayListItem::make(env, &t, &s, &d, &icon, tint, p.checked)
                        }))
                    });
                    LIST_ITEM_PROPS.with(|m| {
                        m.borrow_mut()
                            .insert(handle.0.as_obj().as_raw() as usize, p.clone())
                    });
                    handle
                }
                // A recycled list cell is adopted from the native list, never realized
                // through this path; anything else is an extension piece.
                Some(
                    Builtin::ListCell
                    | Builtin::Inspector
                    | Builtin::InspectorPane
                    | Builtin::Split
                    | Builtin::SplitPane,
                )
                | None => {
                    if let Some(make) = self.registry.get(kind).map(|r| r.make) {
                        return make(self, props, id);
                    }
                    warn_missing_renderer(kind);
                    with_env(|env| {
                        let s = jstr(env, &format!("⟨{kind}⟩"));
                        AHandle(bridge_view(env, "makeLabel", |env| {
                            DayBridge::makeLabel(env, &s)
                        }))
                    })
                }
            }
        }

        fn update(
            &mut self,
            h: &AHandle,
            kind: PieceKind,
            patch: &dyn Any,
            anim: Option<&AnimSpec>,
        ) {
            if let Some(patch) = patch.downcast_ref::<TreePatch>() {
                tree_patch(h, patch);
                return;
            }
            match kind {
                kinds::LIST_ITEM => {
                    if let Some(p) = patch.downcast_ref::<day_spec::props::ListItemPatch>() {
                        list_item_patch(h, p);
                    }
                }
                kinds::IMAGE => {
                    if let Some(p) = patch.downcast_ref::<day_spec::props::ImagePatch>() {
                        match p {
                            day_spec::props::ImagePatch::Tint(c) => {
                                // Drawable tint, as at realize (docs/vectors.md); 0 = authored
                                // colors, or the theme text color again on a template glyph —
                                // the bridge remembers which views were realized as one.
                                let tint = image_tint((*c).map(day_spec::Ink::resolved), false);
                                with_env(|env| {
                                    let _ = java_in(env, |env| {
                                        DayBridge::setImageTint(env, h.0.as_obj(), tint)
                                    });
                                });
                            }
                            // A source swap repaints the same view (docs/images.md), so an
                            // `image()` bound to a signal shows new pixels without rebuilding
                            // its subtree.
                            day_spec::props::ImagePatch::Source(source) => {
                                with_env(|env| match source {
                                    day_spec::ImageSource::Named(named) => {
                                        let s = jstr(env, named);
                                        let _ = java_in(env, |env| {
                                            DayBridge::setImageName(env, h.0.as_obj(), &s)
                                        });
                                    }
                                    day_spec::ImageSource::Bytes(bytes) => {
                                        if let Ok(arr) = env.byte_array_from_slice(bytes) {
                                            let _ = java_in(env, |env| {
                                                DayBridge::setImageBytes(env, h.0.as_obj(), &arr)
                                            });
                                        }
                                    }
                                    day_spec::ImageSource::Decoded(id) => {
                                        let _ = java_in(env, |env| {
                                            DayBridge::setImageBitmap(
                                                env,
                                                h.0.as_obj(),
                                                id.0 as i64,
                                            )
                                        });
                                    }
                                });
                            }
                        }
                    }
                }
                kinds::CONTAINER => match patch.downcast_ref::<ContainerPatch>() {
                    // A solid fill (or none) keeps the motion contract: through `update` with
                    // an animation, it passes between the two colors (docs/tween.md, rule 8).
                    // A styled surface keeps its border, corners and shadow and takes only the
                    // color; a plain one is repainted as it always was.
                    Some(ContainerPatch::Fill(fill @ (None | Some(day_spec::Paint::Solid(_))))) => {
                        let c = super::solid_fill(fill.as_ref());
                        bridge(|env| {
                            let (timing, keys) = anim_arrays(env, anim)?;
                            DayBridge::animateSurfaceFill(
                                env,
                                h.0.as_obj(),
                                c.map_or(0, argb_i32),
                                c.is_some(),
                                &timing,
                                &keys,
                            )
                        });
                    }
                    // A gradient applies at once, and the surface keeps the rest of its style.
                    Some(ContainerPatch::Fill(fill)) => {
                        let spec = super::gradient_spec(fill.as_ref());
                        bridge(|env| {
                            let arr = env.new_double_array(spec.len())?;
                            arr.set_region(env, 0, &spec)?;
                            DayBridge::setSurfaceGradient(env, h.0.as_obj(), &arr)
                        });
                    }
                    // A reactive surface, border, radius or shadow, or a frame-relative radius
                    // re-resolved (docs/surfaces.md "Reactive surfaces"): the realize-time
                    // surface again, on the same view, at once.
                    Some(ContainerPatch::Surface(p)) => apply_container_surface(h, p, false),
                    None => {}
                },
                kinds::NAV_MENU => {
                    if let Some(p) = patch.downcast_ref::<day_spec::props::RefreshPatch>() {
                        with_env(|env| {
                            let _ = java_in(env, |env| {
                                DayBridge::setNavRefreshing(env, h.0.as_obj(), p.refreshing)
                            });
                        });
                        return;
                    }
                    match patch.downcast_ref::<NavMenuPatch>() {
                        // A data-driven `.items(signal, …)` block re-derived: rebuild the native
                        // rows so each click listener reports its current index; stale rows
                        // shift every selection after a removed item by one and drop the last
                        // row's selection entirely.
                        Some(NavMenuPatch::Items {
                            items,
                            icons,
                            tints,
                            menus,
                            badges,
                            badge_icons,
                            badge_tints,
                            sections,
                            selected,
                            document_keys,
                            rows_changed,
                            // `movable` is for toolkits whose own sidebar drags (Cap::NavReorder).
                            ..
                        }) => {
                            let joined = items.join("\u{1f}");
                            let joined_icons = icons
                                .iter()
                                .map(|o| o.clone().unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join("\u{1f}");
                            let joined_tints = nav_tints_joined(
                                &tints
                                    .iter()
                                    .map(|t| t.map(day_spec::Ink::resolved))
                                    .collect::<Vec<_>>(),
                            );
                            let joined_badge_icons = badge_icons
                                .iter()
                                .map(|o| o.clone().unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join("\u{1f}");
                            let joined_badge_tints = nav_tints_joined(
                                &badge_tints
                                    .iter()
                                    .map(|t| t.map(day_spec::Ink::resolved))
                                    .collect::<Vec<_>>(),
                            );
                            let joined_menus = nav_menus_joined(menus);
                            let joined_sections = sections
                                .iter()
                                .map(|o| o.clone().unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join("\u{1f}");
                            with_env(|env| {
                                let s = jstr(env, &joined);
                                let si = jstr(env, &joined_icons);
                                let ss = jstr(env, &joined_sections);
                                // The same rows with new words or counts: update those rows
                                // where they stand. A new row set is rebuilt.
                                let _ = match rows_changed {
                                    Some(rows) => {
                                        let r = jstr(
                                            env,
                                            &rows
                                                .iter()
                                                .map(std::string::ToString::to_string)
                                                .collect::<Vec<_>>()
                                                .join(","),
                                        );
                                        java_in(env, |env| {
                                            DayBridge::updateNavMenuRows(
                                                env,
                                                h.0.as_obj(),
                                                &r,
                                                &s,
                                                &si,
                                            )
                                        })
                                    }
                                    None => java_in(env, |env| {
                                        DayBridge::updateNavMenu(env, h.0.as_obj(), &s, &si, &ss)
                                    }),
                                };
                                let keys = jstr(env, &document_keys.join("\u{1f}"));
                                let _ = java_in(env, |env| {
                                    DayBridge::setDocumentTabKeys(env, h.0.as_obj(), &keys)
                                });
                                // A rebuilt menu comes back unchecked, so re-apply the selection
                                // the patch carries with it.
                                nav_menu_select(env, h, *selected);
                                let st = jstr(env, &joined_tints);
                                let _ = java_in(env, |env| {
                                    DayBridge::setNavMenuTints(env, h.0.as_obj(), &st)
                                });
                                let texts = badges
                                    .iter()
                                    .map(|s| s.as_deref().unwrap_or(""))
                                    .collect::<Vec<_>>()
                                    .join("\u{1f}");
                                let counts = jstr(env, &texts);
                                let sb = jstr(env, &joined_badge_icons);
                                let sbt = jstr(env, &joined_badge_tints);
                                let _ = java_in(env, |env| {
                                    DayBridge::setNavMenuBadges(
                                        env,
                                        h.0.as_obj(),
                                        &sb,
                                        &sbt,
                                        &counts,
                                    )
                                });
                                let sm = jstr(env, &joined_menus);
                                let _ = java_in(env, |env| {
                                    DayBridge::setNavRowMenus(env, h.0.as_obj(), &sm)
                                });
                                if env.exception_check() {
                                    env.exception_clear();
                                }
                            });
                        }
                        // The active indicator follows the model (docs/navigation.md). This
                        // used to be a no-op on the grounds that "mobile selection is transient
                        // (rows ripple, then push)", which was true of the hand-built list that
                        // could not draw a resting state; a NavigationView can, and a sidebar
                        // beside its detail is exactly where that state means "you are here".
                        Some(NavMenuPatch::Selected(sel)) => {
                            with_env(|env| nav_menu_select(env, h, *sel));
                        }
                        Some(NavMenuPatch::IconProgress(_)) | None => {}
                    }
                }
                kinds::NAV => {
                    if let Some(p) = patch.downcast_ref::<day_spec::props::DocumentTabsPatch>() {
                        with_env(|env| {
                            let _ = java_in(env, |env| {
                                DayBridge::documentNavChrome(env, h.0.as_obj(), p.native)
                            });
                        });
                        return;
                    }
                    // Inline search (docs/search.md): the app writing its query patches the live
                    // field, so a sync never rebuilds it or takes the insertion point. The Java
                    // side guards the echo while it writes.
                    if matches!(
                        patch.downcast_ref::<day_spec::props::SearchPatch>(),
                        Some(day_spec::props::SearchPatch::Focus)
                    ) {
                        with_env(|env| {
                            let _ =
                                java_in(env, |env| DayBridge::focusNavSearch(env, h.0.as_obj()));
                            if env.exception_check() {
                                env.exception_clear();
                            }
                        });
                    }
                    if let Some(day_spec::props::SearchPatch::Text(t)) =
                        patch.downcast_ref::<day_spec::props::SearchPatch>()
                    {
                        let text = t.clone();
                        with_env(|env| {
                            let tx = jstr(env, &text);
                            let _ = java_in(env, |env| {
                                DayBridge::setNavSearchText(env, h.0.as_obj(), &tx)
                            });
                            if env.exception_check() {
                                env.exception_clear();
                            }
                        });
                    }
                    if let Some(p) = patch.downcast_ref::<NavPatch>() {
                        match p {
                            NavPatch::Pushed { title, immersive } => with_env(|env| {
                                let s = jstr(env, title);
                                let _ = java_in(env, |env| {
                                    DayBridge::navPush(env, h.0.as_obj(), &s, *immersive)
                                });
                            }),
                            NavPatch::Popped => bridge(|env| DayBridge::navPop(env, h.0.as_obj())),
                            NavPatch::Title(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = java_in(env, |env| {
                                    DayBridge::navSetTitle(env, h.0.as_obj(), &s)
                                });
                            }),
                            NavPatch::GuardTop(on) => with_env(|env| {
                                let _ = java_in(env, |env| {
                                    DayBridge::navSetGuard(env, h.0.as_obj(), *on)
                                });
                            }),
                            // Unreachable: this backend answers `Cap::NavRepresent =
                            // Unsupported`, so the pieces layer never sends it. The plan for
                            // Android is `SlidingPaneLayout`, which decides at measure time and
                            // is OBSERVED rather than told (docs/size-classes.md).
                            #[expect(
                                clippy::match_same_arms,
                                reason = "each group of no-op patches keeps its own arm and the comment saying why"
                            )]
                            NavPatch::Presentation(_) => {}
                            NavPatch::Reorder(order) => with_env(|env| {
                                let text = jstr(
                                    env,
                                    &order
                                        .iter()
                                        .map(usize::to_string)
                                        .collect::<Vec<_>>()
                                        .join(","),
                                );
                                let _ = java_in(env, |env| {
                                    DayBridge::reorderNavSuite(env, h.0.as_obj(), &text)
                                });
                            }),
                            // The resident-page switch (docs/navigation.md): the app moved the
                            // selection, so the suite shows that page and syncs its chrome
                            // Without reporting the move back as a tap.
                            NavPatch::Select(i) => {
                                bridge(|env| {
                                    DayBridge::setNavSuiteSelected(env, h.0.as_obj(), *i as i32)
                                });
                            }
                            // Never arrives: this backend answers `Cap::NavContentList`
                            // Unsupported, so the pieces layer composes the pane itself
                            // (docs/navigation.md).
                            NavPatch::ListVisible(_) | NavPatch::ListInStack(_) => {}
                        }
                    }
                }
                kinds::COVER => {
                    if let Some(p) = patch.downcast_ref::<CoverPatch>() {
                        match p {
                            CoverPatch::Present {
                                background,
                                dismiss_disabled,
                            } => bridge(|env| {
                                DayBridge::coverPresent(
                                    env,
                                    h.0.as_obj(),
                                    background.map_or(0, |c| argb_i32(c.resolved())),
                                    background.is_some(),
                                    *dismiss_disabled,
                                )
                            }),
                            CoverPatch::DismissDisabled(d) => bridge(|env| {
                                DayBridge::coverSetDismissDisabled(env, h.0.as_obj(), *d)
                            }),
                            CoverPatch::Dismiss => {
                                bridge(|env| DayBridge::coverDismiss(env, h.0.as_obj()));
                            }
                        }
                    }
                }
                kinds::LABEL => {
                    if let Some(p) = patch.downcast_ref::<LabelPatch>() {
                        match p {
                            LabelPatch::Paragraph(p) => {
                                with_env(|env| apply_label_paragraph(env, &h.0, p));
                            }
                            LabelPatch::Text(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ =
                                    java_in(env, |env| DayBridge::setLabel(env, h.0.as_obj(), &s));
                            }),
                            LabelPatch::Font(f) => with_env(|env| apply_label_font(env, &h.0, *f)),
                            LabelPatch::Runs(text, runs) => {
                                let node = LABEL_NODE.with(|m| {
                                    m.borrow()
                                        .get(&(h.0.as_obj().as_raw() as usize))
                                        .copied()
                                        .unwrap_or(0)
                                });
                                with_env(|env| set_label_runs(env, node, &h.0, text, runs));
                            }
                            LabelPatch::Color(c) => {
                                bridge(|env| {
                                    DayBridge::setLabelColor(
                                        env,
                                        h.0.as_obj(),
                                        c.map_or(0, |c| argb_i32(c.resolved())),
                                        c.is_some(),
                                    )
                                });
                            }
                        }
                    }
                }
                kinds::BUTTON => {
                    if let Some(p) = patch.downcast_ref::<ButtonPatch>() {
                        match p {
                            ButtonPatch::Content(c) => with_env(|env| {
                                apply_button_content(
                                    env,
                                    &h.0,
                                    &c.title,
                                    c.icon.as_ref(),
                                    c.icon_only,
                                );
                            }),
                            ButtonPatch::Title(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ =
                                    java_in(env, |env| DayBridge::setLabel(env, h.0.as_obj(), &s));
                            }),
                            ButtonPatch::Enabled(e) => {
                                bridge(|env| DayBridge::setEnabled(env, h.0.as_obj(), *e));
                            }
                            ButtonPatch::Selected(on) => {
                                bridge(|env| DayBridge::setButtonSelected(env, h.0.as_obj(), *on));
                            }
                            ButtonPatch::Style(s) => {
                                with_env(|env| apply_button_style(env, &h.0, *s));
                            }
                        }
                    }
                }
                kinds::TOGGLE => {
                    if let Some(p) = patch.downcast_ref::<TogglePatch>() {
                        match p {
                            TogglePatch::On(on) => {
                                bridge(|env| DayBridge::setToggle(env, h.0.as_obj(), *on));
                            }
                            TogglePatch::Enabled(e) => {
                                bridge(|env| DayBridge::setEnabled(env, h.0.as_obj(), *e));
                            }
                        }
                    }
                }
                kinds::SLIDER => {
                    if let Some(p) = patch.downcast_ref::<SliderPatch>() {
                        match p {
                            SliderPatch::Value(v) => {
                                // min recovered from the widget tag
                                bridge(|env| DayBridge::setSlider(env, h.0.as_obj(), *v, 0.0));
                            }
                            SliderPatch::Enabled(e) => {
                                bridge(|env| DayBridge::setEnabled(env, h.0.as_obj(), *e));
                            }
                        }
                    }
                }
                kinds::PROGRESS => {
                    if let Some(p @ ProgressPatch::Animating(_)) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        crate::controls::spinner::update(self, h, p);
                        return;
                    }

                    if let Some(ProgressPatch::Value(Some(v))) =
                        patch.downcast_ref::<ProgressPatch>()
                    {
                        bridge(|env| DayBridge::setProgress(env, h.0.as_obj(), *v));
                    }
                }
                kinds::PICKER => crate::picker::update_any(self, h, patch),
                kinds::TEXT_AREA => crate::textarea::update_any(self, h, patch),
                kinds::TEXT_FIELD => {
                    if let Some(p) = patch.downcast_ref::<TextFieldPatch>() {
                        match p {
                            TextFieldPatch::Text { text, from_native } => {
                                if !*from_native {
                                    with_env(|env| {
                                        let s = jstr(env, text);
                                        let _ = java_in(env, |env| {
                                            DayBridge::setTextField(env, h.0.as_obj(), &s)
                                        });
                                    });
                                }
                            }
                            TextFieldPatch::Placeholder(t) => with_env(|env| {
                                let s = jstr(env, t);
                                let _ = java_in(env, |env| {
                                    DayBridge::setPlaceholder(env, h.0.as_obj(), &s)
                                });
                            }),
                            TextFieldPatch::Enabled(e) => {
                                bridge(|env| DayBridge::setEnabled(env, h.0.as_obj(), *e));
                            }
                            TextFieldPatch::Select(sel) => {
                                let at = |n: usize| n.min(i32::MAX as usize) as i32;
                                bridge(|env| {
                                    DayBridge::setTextFieldSelection(
                                        env,
                                        h.0.as_obj(),
                                        at(sel.start),
                                        at(sel.end),
                                    )
                                });
                            }
                        }
                    }
                }
                kinds::LIST => match patch.downcast_ref::<ListPatch>() {
                    Some(ListPatch::Reload) => {
                        // notifyDataSetChanged: getCount reads the snapshot, getView is deferred to
                        // the next layout, so it is safe inside a with_tree borrow.
                        bridge(|env| DayBridge::listReload(env, h.0.as_obj()));
                    }
                    Some(ListPatch::Splice(deltas)) => {
                        // The deltas as the adapter's range notifications, in one bridge call
                        // (docs/list.md "Keyed updates"). Like a reload, they only mark the rows
                        // changed; RecyclerView binds at its next layout, outside this borrow.
                        let ops = crate::list_wire::splice_ops(deltas);
                        java(|env| {
                            let array = env.new_int_array(ops.len())?;
                            array.set_region(env, 0, &ops)?;
                            DayBridge::listSplice(env, h.0.as_obj(), &array)
                        });
                    }
                    Some(ListPatch::ScrollToEnd) => {
                        // Posts smoothScrollToPosition(count-1) on the RecyclerView (no-op if empty).
                        bridge(|env| DayBridge::listScrollToEnd(env, h.0.as_obj()));
                    }
                    Some(ListPatch::ScrollToRow(row)) => {
                        bridge(|env| DayBridge::listScrollToRow(env, h.0.as_obj(), *row as i32));
                    }
                    Some(ListPatch::Selected(rows)) => {
                        // Record, then repaint the visible holders; newly bound holders pick
                        // the state up from nativeListIsSelected in the bind path. Paint
                        // only; no selection event echoes back.
                        let key = h.0.as_obj().as_raw() as usize;
                        if let Some(nid) = LIST_NODE.with(|m| m.borrow().get(&key).copied()) {
                            LIST_SELECTED.with(|m| {
                                m.borrow_mut().insert(nid, rows.iter().copied().collect());
                            });
                            bridge(|env| DayBridge::listPaintSelection(env, h.0.as_obj(), nid));
                        }
                    }
                    Some(ListPatch::RowSizeInvalidated { cell, height }) => {
                        // A row re-laid at a new height after its bind (its text changed). This
                        // arrives inside day-core's layout pass, so the resize waits for the next
                        // turn, and it carries its height: nothing asks the source back.
                        let key = h.0.as_obj().as_raw() as usize;
                        let list = LIST_NODE.with(|m| m.borrow().get(&key).copied());
                        let (cell, height) = (*cell, *height);
                        if let Some(nid) = list {
                            <Material as Platform>::post(Box::new(move || {
                                // The cell's own GlobalRef, while its list still holds it.
                                let gref = LIST_CELLS.with(|m| {
                                    m.borrow().get(&nid).and_then(|cells| {
                                        cells
                                            .values()
                                            .find(|g| g.as_obj().as_raw() as usize == cell)
                                            .cloned()
                                    })
                                });
                                if let Some(gref) = gref {
                                    set_list_cell_height(gref.as_obj(), height);
                                }
                            }));
                        }
                    }
                    // No editing mode (Cap::ListEditing is Unsupported): rows drag and swipe
                    // as they are.
                    Some(ListPatch::Editing(_)) | None => {}
                },
                _ => {
                    if let Some(update) = self.registry.get(kind).map(|r| r.update) {
                        update(self, h, patch);
                    }
                }
            }
        }

        /// Offer a satellite piece its teardown hook before `release` frees the handle (§15.2).
        fn release_piece(&mut self, kind: day_spec::PieceKind, h: &Self::Handle) {
            // Copy the fn pointer out first: the registry lookup borrows `self` immutably and
            // the hook needs it mutably.
            let f = self.registry.get(kind).and_then(|r| r.release);
            if let Some(f) = f {
                f(self, h);
            }
        }
        fn release(&mut self, h: AHandle) {
            bridge(|env| DayBridge::releaseContextMenu(env, h.0.as_obj()));
            bridge(|env| DayTransfer::release(env, h.0.as_obj()));
            // A released window root drops its `SECONDARY` record (docs/windows.md teardown;
            // the activity itself already finished or is finishing).
            with_env(|env| {
                SECONDARY.with(|s| {
                    s.borrow_mut().retain(|(_, gref)| {
                        !env.is_same_object(h.0.as_obj(), gref.as_obj())
                            .unwrap_or(false)
                    });
                });
            });
            let key = h.0.as_obj().as_raw() as usize;
            // One sweep drops this view's entry from every registered `SideTable`, present
            // and future, so ptr-keyed side state cannot outlive the handle
            // (day_spec::sidetable; the maps below predate it and stay manual).
            day_spec::sidetable::sweep(key);
            LABEL_NODE.with(|m| m.borrow_mut().remove(&key));
            NAV_MENU_ROWS.with(|m| m.borrow_mut().remove(&key));
            LIST_ITEM_PROPS.with(|m| m.borrow_mut().remove(&key));
            if let Some(nid) = LIST_NODE.with(|m| m.borrow_mut().remove(&key)) {
                release_pinned_header(&h, nid);
                TREE_SOURCES.with(|m| m.borrow_mut().remove(&nid));
                LIST_SOURCES.with(|m| {
                    m.borrow_mut().remove(&nid);
                });
                // Drop the list's per-cell GlobalRefs with it (see LIST_CELLS): each Arc drop
                // releases its JNI global ref, keeping long sessions off the JNI table limit.
                LIST_CELLS.with(|m| {
                    m.borrow_mut().remove(&nid);
                });
                LIST_BINDINGS.with(|m| m.borrow_mut().retain(|(host, _), _| *host != nid));
                LIST_SELECTED.with(|m| {
                    m.borrow_mut().remove(&nid);
                });
                LIST_AUTOMATIC.with(|m| m.borrow_mut().remove(&nid));
            }
            bridge(|env| DayBridge::removeChild(env, h.0.as_obj()));
        }

        fn insert(&mut self, parent: &AHandle, child: &AHandle, _index: usize) {
            bridge(|env| DayBridge::addChild(env, parent.0.as_obj(), child.0.as_obj()));
            // A nav menu now has ancestors: if one of them is a navigation suite, its rows are
            // that suite's chrome. Best-effort and after the insert, like every other decoration
            // on this backend: a failure here must not abort the tree build.
            let rows = NAV_MENU_ROWS.with(|m| {
                m.borrow()
                    .get(&(child.0.as_obj().as_raw() as usize))
                    .cloned()
            });
            if let Some((node, titles, icons, keys, badges)) = rows {
                with_env(|env| {
                    let t = jstr(env, &titles);
                    let i = jstr(env, &icons);
                    let b = jstr(env, &badges);
                    let _ = java_in(env, |env| {
                        DayBridge::setNavSuiteRows(env, child.0.as_obj(), &t, &i, &b, node)
                    });
                    let keys = jstr(env, &keys);
                    let _ = java_in(env, |env| {
                        DayBridge::setDocumentTabKeys(env, child.0.as_obj(), &keys)
                    });
                    if env.exception_check() {
                        env.exception_clear();
                    }
                });
            }
        }

        fn remove(&mut self, _parent: &AHandle, child: &AHandle) {
            bridge(|env| DayBridge::removeChild(env, child.0.as_obj()));
        }

        fn move_child(&mut self, parent: &AHandle, child: &AHandle, to: usize) {
            bridge(|env| DayBridge::moveChild(env, parent.0.as_obj(), child.0.as_obj(), to as i32));
        }

        fn measure(&mut self, h: &AHandle, kind: PieceKind, p: Proposal) -> Size {
            let d = density();
            match kind {
                kinds::LABEL => {
                    let natural_w = measure_call(h, |env, v| DayBridge::measureWidth(env, v)) / d;
                    match p.width {
                        Some(pw) if natural_w > pw => {
                            let wpx = (pw * d).round() as i32;
                            let hh = with_env(|env| {
                                f64::from(
                                    DayBridge::measureHeightForWidth(env, h.0.as_obj(), wpx)
                                        .unwrap_or(0),
                                )
                            });
                            Size::new(pw, hh / d)
                        }
                        _ => Size::new(
                            natural_w,
                            measure_call(h, |env, v| DayBridge::measureHeight(env, v)) / d,
                        ),
                    }
                }
                kinds::NAV_MENU => Size::new(
                    p.width.unwrap_or(320.0),
                    p.height.unwrap_or_else(|| {
                        measure_call(h, |env, v| DayBridge::measureHeight(env, v)) / d
                    }),
                ),
                // The Material list item measures itself: its height at the row's width.
                kinds::LIST_ITEM => {
                    let w = p.width.unwrap_or_else(|| {
                        measure_call(h, |env, v| DayBridge::measureWidth(env, v)) / d
                    });
                    let wpx = (w * d).round() as i32;
                    let hh = with_env(|env| {
                        f64::from(
                            DayBridge::measureHeightForWidth(env, h.0.as_obj(), wpx).unwrap_or(0),
                        )
                    });
                    Size::new(w, hh / d)
                }
                kinds::SLIDER => Size::new(
                    p.width.unwrap_or(180.0),
                    (measure_call(h, |env, v| DayBridge::measureHeight(env, v)) / d).max(24.0),
                ),
                // `PICKER` falls to the native measureWidth/measureHeight default below.
                kinds::TEXT_AREA => crate::textarea::measure_any(self, h, p),
                kinds::TEXT_FIELD => Size::new(
                    p.width.unwrap_or(180.0),
                    (measure_call(h, |env, v| DayBridge::measureHeight(env, v)) / d).max(40.0),
                ),
                kinds::DIVIDER => Size::new(p.width.unwrap_or(0.0), 1.0),
                kinds::LIST => Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0)),
                // A canvas has no content of its own to measure (`DayCanvasView` is a bare
                // `View`, which measures 0×0), so it takes what the layout offers, the way the
                // other toolkits' default arms already answer (`p.width.unwrap_or(natural)`).
                // Without this a canvas in a form row got the row's height from `.height(…)`
                // and no width at all: the Showcase's sensor strip charts drew nothing here
                // while they filled their rows on iOS.
                kinds::CANVAS => Size::new(
                    p.width.unwrap_or_else(|| {
                        measure_call(h, |env, v| DayBridge::measureWidth(env, v)) / d
                    }),
                    p.height.unwrap_or_else(|| {
                        measure_call(h, |env, v| DayBridge::measureHeight(env, v)) / d
                    }),
                ),
                _ => {
                    if let Some(measure) = self.registry.get(kind).and_then(|r| r.measure) {
                        return measure(self, h, p);
                    }
                    Size::new(
                        measure_call(h, |env, v| DayBridge::measureWidth(env, v)) / d,
                        measure_call(h, |env, v| DayBridge::measureHeight(env, v)) / d,
                    )
                }
            }
        }

        /// `View.getBaseline()`, the same answer `LinearLayout`'s `baselineAligned` uses
        /// (docs/baseline.md). `TextView` and everything built on it override it; the base View
        /// returns -1, which is "no baseline".
        fn first_baseline(&mut self, h: &AHandle, kind: PieceKind, size: Size) -> Option<f64> {
            if !day_spec::kind_has_baseline(kind) {
                return None;
            }
            let d = density();
            let px = java(|env| {
                DayBridge::baselineAt(
                    env,
                    h.0.as_obj(),
                    (size.width * d).round() as i32,
                    (size.height * d).round() as i32,
                )
            })?;
            (px >= 0).then(|| f64::from(px) / d)
        }

        fn set_frame(&mut self, h: &AHandle, frame: Rect, anim: Option<&AnimSpec>) {
            // DayFixed placement; animated, a ValueAnimator places the child at the frames in
            // between, from the one it shows (docs/tween.md "The animation contract").
            let d = density();
            bridge(|env| {
                let (timing, keys) = anim_arrays(env, anim)?;
                DayBridge::setFrame(
                    env,
                    h.0.as_obj(),
                    (frame.origin.x * d).round() as i32,
                    (frame.origin.y * d).round() as i32,
                    (frame.size.width * d).round() as i32,
                    (frame.size.height * d).round() as i32,
                    &timing,
                    &keys,
                )
            });
        }

        fn set_opacity(&mut self, h: &AHandle, opacity: f64, anim: Option<&AnimSpec>) {
            bridge(|env| {
                let (timing, keys) = anim_arrays(env, anim)?;
                DayBridge::setOpacity(env, h.0.as_obj(), opacity as f32, &timing, &keys)
            });
        }

        fn set_transform(
            &mut self,
            h: &AHandle,
            t: Transform,
            _size: Size,
            anim: Option<&AnimSpec>,
        ) {
            let d = density();
            bridge(|env| {
                let (timing, keys) = anim_arrays(env, anim)?;
                DayBridge::setTransform(
                    env,
                    h.0.as_obj(),
                    (t.tx * d) as f32,
                    (t.ty * d) as f32,
                    t.sx as f32,
                    t.sy as f32,
                    t.rotation.degrees() as f32,
                    &timing,
                    &keys,
                )
            });
        }

        /// `DayBridge.readDisplayed`: the view's own alpha, translation, frame and fill, which
        /// a run writes every frame, so a read mid-flight is what is on screen and a read at
        /// rest is the model value (docs/tween.md "The animation contract").
        fn read_displayed(&self, h: &AHandle) -> day_spec::Displayed {
            if !vm_ready() {
                return day_spec::Displayed::default();
            }
            java(|env| {
                let arr = DayBridge::readDisplayed(env, h.0.as_obj())?;
                let mut slots = vec![0.0; arr.len(env)?];
                arr.get_region(env, 0, &mut slots)?;
                Ok(crate::motion_wire::displayed(&slots))
            })
            .unwrap_or_default()
        }

        fn set_selectable(&mut self, h: &AHandle, selectable: bool) -> Option<AHandle> {
            // A plain label is an android.widget.TextView; make its text selectable (long-press →
            // copy, docs/text.md). A direct instance call; no DayBridge method needed.
            bridge(|env| {
                downcast::<framework::TextView>(env, h.0.as_obj())?
                    .setTextIsSelectable(env, selectable)
            });
            None
        }

        fn set_input_traits(
            &mut self,
            h: &AHandle,
            traits: &day_spec::InputTraits,
        ) -> Option<AHandle> {
            use day_spec::{InputPurpose, SubmitLabel};
            // One EditText takes every member as a property (docs/textfield.md), so the handle
            // never changes. Secure and purpose merge into a single Android input type, which
            // is why the whole set crosses in one call; `DayBridge.setInputTraits` documents
            // the same integer codes and ignores a view that is not a text field.
            let purpose = match traits.purpose {
                InputPurpose::Text => 0,
                InputPurpose::Name => 1,
                InputPurpose::Email => 2,
                InputPurpose::Url => 3,
                InputPurpose::Phone => 4,
                InputPurpose::Number => 5,
                InputPurpose::Decimal => 6,
                InputPurpose::Username => 7,
                InputPurpose::Password => 8,
                InputPurpose::NewPassword => 9,
                InputPurpose::OneTimeCode => 10,
            };
            let submit = match traits.submit_label {
                SubmitLabel::Return => 0,
                SubmitLabel::Done => 1,
                SubmitLabel::Go => 2,
                SubmitLabel::Next => 3,
                SubmitLabel::Search => 4,
                SubmitLabel::Send => 5,
            };
            // `max_length` stays with day-pieces: it counts characters, and Android's
            // `LengthFilter` counts UTF-16 units, so a native bound would refuse text the
            // bound allows.
            with_env(|env| {
                let _ = DayBridge::setInputTraits(
                    env,
                    h.0.as_obj(),
                    traits.secure,
                    traits.read_only,
                    purpose,
                    submit,
                );
                if env.exception_check() {
                    env.exception_clear();
                }
            });
            None
        }

        fn set_cursor(&mut self, h: &AHandle, cursor: Cursor) {
            // `View.setPointerIcon` (API 24): a system icon by `PointerIcon.TYPE_*`, or null to
            // release the view to the platform's own choice (docs/cursor.md). Only a mouse or
            // trackpad ever shows it; a finger never does, which is correct.
            let ty = pointer_icon_type(&cursor);
            bridge(|env| {
                let icon = match ty {
                    None => JObject::null(),
                    Some(t) => {
                        let ctx = DayRuntime::ctx(env)?;
                        framework::PointerIcon::getSystemIcon(env, &ctx, t)?.into()
                    }
                };
                downcast::<framework::View>(env, h.0.as_obj())?.setPointerIcon(env, &icon)
            });
        }

        fn set_scroll_content(&mut self, h: &AHandle, content: Size) {
            let d = density();
            bridge(|env| {
                DayBridge::setScrollContent(
                    env,
                    h.0.as_obj(),
                    (content.width * d).round() as i32,
                    (content.height * d).round() as i32,
                )
            });
        }

        /// Material's one-line list item: 16dp sides, 56dp tall (8dp keeps taller content off the edges).
        fn list_row_metrics(&mut self, _h: &AHandle) -> day_spec::RowMetrics {
            day_spec::RowMetrics {
                insets: day_spec::Insets {
                    top: 8.0,
                    leading: 16.0,
                    bottom: 8.0,
                    trailing: 16.0,
                },
                min_height: 56.0,
            }
        }

        /// A row scroll (docs/list.md "Programmatic scrolling"): `DayBridge.listScroll` posts
        /// it behind any pending row update, then `scrollToPositionWithOffset` for a jump or
        /// `smoothScrollBy` over the aligned distance for an animation. The list's viewport
        /// report follows from its scroll and layout callbacks.
        fn list_scroll(&mut self, h: &AHandle, to: day_spec::ListScroll) -> bool {
            let row = to.row.map_or(-1, |r| i32::try_from(r).unwrap_or(i32::MAX));
            let align = crate::list_wire::align_code(to.align);
            java(|env| DayBridge::listScroll(env, h.0.as_obj(), row, align, to.animated)).is_some()
        }

        fn nav_scroll_to_edge(&mut self, h: &AHandle, bottom: bool) -> bool {
            with_env(|env| DayBridge::navScrollToEdge(env, h.0.as_obj(), bottom).unwrap_or(false))
        }

        fn scroll_to(&mut self, h: &AHandle, target: Rect, animated: bool) {
            let d = density();
            bridge(|env| {
                DayBridge::scrollToRect(
                    env,
                    h.0.as_obj(),
                    (target.origin.x * d).round() as i32,
                    (target.origin.y * d).round() as i32,
                    (target.size.width * d).round() as i32,
                    (target.size.height * d).round() as i32,
                    animated,
                )
            });
        }

        fn set_focusable(&mut self, h: &AHandle, node: NodeId, focusable: bool) {
            if focusable {
                bridge(|env| DayBridge::enableKeys(env, h.0.as_obj(), node.0 as i64));
            }
        }

        fn focus(&mut self, h: &AHandle, _node: NodeId, focused: bool) {
            // DayBridge pairs the request with the IME (show on gain, hide on resign) and
            // resigns to the focusable content root, since Android focus is never "nowhere".
            bridge(|env| DayBridge::focusView(env, h.0.as_obj(), focused));
        }

        fn set_event_sink(&mut self, sink: EventSink) {
            SINK.with(|s| *s.borrow_mut() = Some(Rc::from(sink)));
        }

        fn enable_gesture(&mut self, h: &AHandle, node: NodeId, kind: day_spec::GestureKind) {
            // Pan is a wheel listener and pinch a ScaleGestureDetector fed from the shared
            // touch listener (docs/canvas.md "Zoom and pan"); neither falls through to the tap
            // wire below.
            if kind == day_spec::GestureKind::Pan {
                bridge(|env| DayBridge::enablePan(env, h.0.as_obj(), node.0 as i64));
                return;
            }
            if kind == day_spec::GestureKind::Pinch {
                bridge(|env| DayBridge::enablePinch(env, h.0.as_obj(), node.0 as i64));
                return;
            }
            // Rotation is a two-pointer tracker in the bridge, on the same touch listener
            // (ScaleGestureDetector reports no angle), so it runs alongside the pinch.
            if kind == day_spec::GestureKind::Rotate {
                bridge(|env| DayBridge::enableRotate(env, h.0.as_obj(), node.0 as i64));
                return;
            }
            // Hover is a separate listener (`setOnHoverListener`), not a touch one: MotionEvent's
            // HOVER_* actions arrive only from a mouse or a stylus, so wiring it into the touch
            // path would mean testing for a pointer kind on every finger event.
            if matches!(kind, day_spec::GestureKind::Hover) {
                bridge(|env| DayBridge::enableHover(env, h.0.as_obj(), node.0 as i64));
                return;
            }
            // Long press and double tap are the platform's GestureDetector, press an observer
            // that never claims the stream, touches every pointer of the MotionEvent; all of
            // them ride the shared touch listener beside the tap and the drag (DayBridge
            // `Recognizers`, docs/gestures.md).
            let recognizer = match kind {
                day_spec::GestureKind::LongPress => Some(bridge::gesture::LONG_PRESS),
                day_spec::GestureKind::DoubleTap => Some(bridge::gesture::DOUBLE_TAP),
                day_spec::GestureKind::Press => Some(bridge::gesture::PRESS_IN),
                day_spec::GestureKind::Touches => Some(bridge::gesture::TOUCHES),
                _ => None,
            };
            if let Some(code) = recognizer {
                bridge(|env| DayBridge::enableRecognizer(env, h.0.as_obj(), node.0 as i64, code));
                return;
            }
            let is_drag = matches!(kind, day_spec::GestureKind::Drag);
            bridge(|env| DayBridge::enableGesture(env, h.0.as_obj(), node.0 as i64, is_drag));
        }

        fn gesture_support(&self, _kind: day_spec::GestureKind) -> Support {
            // Every kind has its recognizer here: `GestureDetector` for the long press and
            // the double tap, the MotionEvent's own pointers for press and touches, and the
            // tap, drag, pinch, rotation, wheel pan and hover listeners there always were.
            Support::Native
        }

        fn set_hit_slop(&mut self, h: &AHandle, _node: NodeId, slop: day_spec::Insets) {
            // The parent's TouchDelegate (DayBridge `SlopDelegate`), in px.
            let d = density();
            let px = |v: f64| (v.max(0.0) * d).round() as i32;
            bridge(|env| {
                DayBridge::setHitSlop(
                    env,
                    h.0.as_obj(),
                    px(slop.top),
                    px(slop.leading),
                    px(slop.bottom),
                    px(slop.trailing),
                )
            });
        }

        fn set_drag_axis(&mut self, h: &AHandle, _node: NodeId, axis: Option<day_spec::DragAxis>) {
            let code = match axis {
                None => 0,
                Some(day_spec::DragAxis::Horizontal) => 1,
                Some(day_spec::DragAxis::Vertical) => 2,
            };
            bridge(|env| DayBridge::setDragAxis(env, h.0.as_obj(), code));
        }

        fn set_subtree_enabled(&mut self, h: &AHandle, _node: NodeId, enabled: bool) {
            bridge(|env| DayBridge::setSubtreeEnabled(env, h.0.as_obj(), enabled));
        }

        fn primary_pointer(&self) -> day_spec::PointerKind {
            // Touch where the device has a touch screen, a mouse where it has none (a
            // Chromebook, a TV box with a mouse).
            with_env(|env| java_string(env, DayBridge::primaryPointer))
                .and_then(|name| day_spec::PointerKind::from_name(&name))
                .unwrap_or_else(day_spec::PointerKind::platform_default)
        }

        fn set_context_menu_fn(&mut self, h: &AHandle, _node: NodeId, f: day_spec::ContextMenuFn) {
            let key = h.0.as_obj().as_raw() as usize;
            CONTEXT_MENU_FNS.with(|menus| menus.insert(key, f));
            bridge(|env| DayBridge::setContextMenuFn(env, h.0.as_obj(), key as i64));
        }

        fn set_button_menu(&mut self, h: &AHandle, _node: NodeId, f: day_spec::ContextMenuFn) {
            // The click opens a PopupMenu under the button (DayBridge.setButtonMenu), built from
            // the spec the provider lowers at that moment, in place of the `Pressed` report.
            let key = h.0.as_obj().as_raw() as usize;
            BUTTON_MENU_FNS.with(|menus| menus.insert(key, f));
            bridge(|env| DayBridge::setButtonMenu(env, h.0.as_obj(), key as i64));
        }

        fn set_context_menu(&mut self, h: &AHandle, _node: NodeId, items: &[day_spec::MenuItem]) {
            CONTEXT_MENU_FNS.with(|menus| menus.remove(h.0.as_obj().as_raw() as usize));
            let mut spec = String::new();
            serialize_menu(items, &mut spec);
            with_env(|env| {
                let jspec = jstr(env, &spec);
                let _ = java_in(env, |env| {
                    DayBridge::setContextMenu(env, h.0.as_obj(), &jspec)
                });
            });
        }

        fn edit_toolbar(&mut self, h: &AHandle, ops: &[day_spec::ToolbarOp]) -> bool {
            // The window toolbar (docs/toolbars.md) is the nav host's app-bar menu: actions the
            // platform lays out and folds into its overflow itself, built on the Java side from
            // a record-per-item spec. A menu item holds no state of its own, so an edit is
            // applied to the model and the menu repainted from it. The one exception, a page's
            // search action, keeps its SearchView across the repaint (DayNavHost.WindowBar). Best-effort like every other nav decoration: a throw must never
            // reach the tree build.
            let key = h.0.as_obj().as_raw() as usize;
            let spec = WINDOW_BARS.with(|m| {
                let mut m = m.borrow_mut();
                let bar = m.entry(key).or_default();
                for op in ops {
                    bar.apply(op);
                }
                serialize_toolbar(bar.items())
            });
            with_env(|env| {
                let jspec = jstr(env, &spec);
                let _ = java_in(env, |env| {
                    DayBridge::setWindowToolbar(env, h.0.as_obj(), &jspec)
                });
                if env.exception_check() {
                    env.exception_clear();
                }
            });
            true
        }

        fn update_toolbar(&mut self, h: &AHandle, patch: &day_spec::ToolbarPatch) {
            use day_spec::ToolbarPatch as P;
            // The model first, so the next repaint shows the value too.
            let key = h.0.as_obj().as_raw() as usize;
            WINDOW_BARS.with(|m| {
                if let Some(bar) = m.borrow_mut().get_mut(&key) {
                    bar.patch(patch);
                }
            });
            let (item, op, num) = match patch {
                P::Enabled { item, on } => (item, 0, f64::from(u8::from(*on))),
                P::On { item, on } => (item, 1, f64::from(u8::from(*on))),
                P::Selected { item, index } => (item, 2, *index as f64),
                // A page's search action (docs/search.md): reveal the field and give it focus.
                P::Focus { item } => (item, 3, 0.0),
                // The app wrote the page's query: into the live field, without its listeners
                // echoing it back, and without rebuilding the field under the user's typing.
                P::Text { item, text } => {
                    with_env(|env| {
                        let jid = jstr(env, item);
                        let jtext = jstr(env, text);
                        let _ = java_in(env, |env| {
                            DayBridge::updateWindowToolbarText(env, h.0.as_obj(), &jid, &jtext)
                        });
                        if env.exception_check() {
                            env.exception_clear();
                        }
                    });
                    return;
                }
                // The app bar's SearchView offers no completions (docs/search.md).
                P::Suggestions { .. } => return,
                // New words or a new prompt: the menu is repainted from the model, which is how
                // an Android app bar takes a changed item (its SearchView survives the repaint).
                // The bottom bar's status takes new words in place, the way the page's search
                // field takes its text: a repaint would rebuild the bar's buttons for every
                // step of a sync.
                P::Label { item, label, .. }
                    if WINDOW_BARS.with(|m| {
                        m.borrow().get(&key).is_some_and(|bar| {
                            bar.items().iter().any(|i| {
                                i.id == *item
                                    && i.placement == day_spec::ToolbarPlacement::Bottom
                                    && matches!(i.kind, day_spec::ToolbarItemKind::Label)
                            })
                        })
                    }) =>
                {
                    with_env(|env| {
                        let jid = jstr(env, item);
                        let jtext = jstr(env, label);
                        let _ = java_in(env, |env| {
                            DayBridge::updateWindowToolbarText(env, h.0.as_obj(), &jid, &jtext)
                        });
                        if env.exception_check() {
                            env.exception_clear();
                        }
                    });
                    return;
                }
                P::Label { .. } | P::Placeholder { .. } => {
                    let spec = WINDOW_BARS.with(|m| {
                        m.borrow()
                            .get(&key)
                            .map(|bar| serialize_toolbar(bar.items()))
                    });
                    if let Some(spec) = spec {
                        with_env(|env| {
                            let jspec = jstr(env, &spec);
                            let _ = java_in(env, |env| {
                                DayBridge::setWindowToolbar(env, h.0.as_obj(), &jspec)
                            });
                            if env.exception_check() {
                                env.exception_clear();
                            }
                        });
                    }
                    return;
                }
            };
            with_env(|env| {
                let jid = jstr(env, item);
                let _ = java_in(env, |env| {
                    DayBridge::updateWindowToolbar(env, h.0.as_obj(), &jid, op, num)
                });
                if env.exception_check() {
                    env.exception_clear();
                }
            });
        }

        fn set_app_menu(&mut self, items: &[day_spec::MenuItem]) {
            // Android has no persistent menu bar; the platform convention for a global app menu is
            // the app-bar overflow (⋮). DayActivity.onCreateOptionsMenu builds from this spec.
            let mut spec = String::new();
            serialize_menu(items, &mut spec);
            with_env(|env| {
                let jspec = jstr(env, &spec);
                let _ = java_in(env, |env| DayBridge::setAppMenu(env, &jspec));
            });
        }

        fn supports_lifecycle(&self, phase: day_spec::Lifecycle) -> bool {
            lifecycle_supported(phase)
        }

        fn attach_tree(&mut self, host: &AHandle, source: day_spec::TreeSource) {
            if let Some(id) = LIST_NODE.with(|m| {
                m.borrow()
                    .get(&(host.0.as_obj().as_raw() as usize))
                    .copied()
            }) {
                TREE_SOURCES.with(|m| m.borrow_mut().insert(id, (source, Vec::new(), Vec::new())));
                tree_reload(host);
            }
        }
        fn attach_list(&mut self, host: &AHandle, source: ListSource) {
            // A source that observes the viewport gets the whole report (DayListView reports
            // after its first layout with rows), and never the first-row-only one.
            let viewport = source.viewport.is_some();
            if !viewport && let Some(report) = &source.first_visible {
                report(0);
            }
            let reader = source.reader_scroll.is_some();
            let supplements = source.is_supplement.is_some();
            bridge(|env| {
                DayBridge::listConfigure(env, host.0.as_obj(), viewport, reader, supplements)
            });
            let key = host.0.as_obj().as_raw() as usize;
            if let Some(nid) = LIST_NODE.with(|m| m.borrow().get(&key).copied()) {
                LIST_SOURCES.with(|m| {
                    m.borrow_mut().insert(nid, source);
                });
            }
            bridge(|env| DayBridge::listReload(env, host.0.as_obj()));
        }

        fn adopt(&mut self, raw: RawHandle) -> AHandle {
            // A recycling ListView cell (a DayFixed); Day fills/rebinds its row content in place.
            with_env(|env| {
                let obj = unsafe { JObject::from_raw(env, raw as jni::sys::jobject) };
                AHandle(std::sync::Arc::new(
                    // Only fails when the JNI global-ref table is exhausted (a process-fatal
                    // leak elsewhere); there is no cell to hand back without the ref.
                    env.new_global_ref(&obj).expect("adopt: global ref"),
                ))
            })
        }

        fn native_document_tabs(&self) -> bool {
            true
        }

        fn open_window(
            &mut self,
            id: NodeId,
            options: &day_spec::WindowOptions,
            kind: day_spec::WindowKind,
        ) -> day_spec::WindowOpenReply<AHandle> {
            // Preferences stay modal on mobile (docs/windows.md); the cover fallback is
            // the platform settings idiom; Normal windows become document activities.
            if kind.is_panel() {
                return day_spec::WindowOpenReply::Unsupported;
            }
            let ok = with_env(|env| {
                let Ok(title) = env.new_string(&options.title) else {
                    return false;
                };
                java_in(env, |env| DayBridge::openWindow(env, id.0 as i64, &title)).is_some()
            });
            if ok {
                day_spec::WindowOpenReply::Pending
            } else {
                day_spec::WindowOpenReply::Unsupported
            }
        }

        fn close_window(&mut self, host: &AHandle) {
            with_env(|env| {
                if let Some(node) = secondary_node_of(env, host) {
                    let _ = java_in(env, |env| DayBridge::closeWindow(env, node as i64));
                }
            });
        }

        fn focus_window(&mut self, host: &AHandle) {
            with_env(|env| {
                if let Some(node) = secondary_node_of(env, host) {
                    let _ = java_in(env, |env| DayBridge::focusWindow(env, node as i64));
                }
            });
        }

        /// Window properties (docs/windows.md): fullscreen through the activity window's
        /// insets controller, content protection through `FLAG_SECURE`. `DayBridge` addresses
        /// a window by its day node, 0 for the primary activity, and reports the outcome
        /// back as `K_WINDOW_STATE_CHANGED`.
        fn apply_window(&mut self, host: &AHandle, change: &day_spec::WindowChange) {
            use day_spec::WindowChange as C;
            with_env(|env| {
                let node = secondary_node_of(env, host).unwrap_or(0) as i64;
                match change {
                    C::State(state) => {
                        let _ = java_in(env, |env| {
                            DayBridge::setWindowState(env, node, state.code())
                        });
                    }
                    C::ContentProtected(on) => {
                        let _ = java_in(env, |env| DayBridge::setWindowSecure(env, node, *on));
                    }
                    // Size, position and stacking belong to the system and the user here.
                    // `Appearance` stays Unsupported too: the activities are FragmentActivity,
                    // and the one per-activity route (an overridden uiMode configuration)
                    // recreates the activity, losing what the window shows; the app-wide
                    // `set_appearance` still applies to every window.
                    _ => {}
                }
            });
        }

        /// The display the primary window is on (`Cap::Monitors` Emulated), in dp, with the
        /// work area inside the system bars and any cutout.
        fn monitors(&self) -> Vec<day_spec::Monitor> {
            let line = with_env(|env| {
                let info = DayBridge::displayInfo(env).ok()?;
                if info.is_null() {
                    return None;
                }
                string_of(env, &info).ok()
            });
            line.and_then(|l| parse_display_info(&l))
                .into_iter()
                .collect()
        }

        /// Dynamic shortcuts through `ShortcutManager` (docs/deep-links.md): each opens its
        /// route through the same deep-link intent the Day.toml `[[shortcuts]]` carry.
        fn set_launcher_shortcuts(&mut self, shortcuts: &[day_spec::LauncherShortcut]) {
            // One string over JNI: records split by U+001E, route and label by U+001F.
            let packed = shortcuts
                .iter()
                .map(|sc| {
                    let clean = |s: &str| s.replace(['\u{1e}', '\u{1f}'], " ");
                    format!("{}\u{1f}{}", clean(&sc.route), clean(&sc.label))
                })
                .collect::<Vec<_>>()
                .join("\u{1e}");
            with_env(|env| {
                let jpacked = jstr(env, &packed);
                let _ = java_in(env, |env| DayBridge::setLauncherShortcuts(env, &jpacked));
            });
        }

        fn set_window_title(&mut self, host: &AHandle, title: &str) {
            with_env(|env| {
                let Ok(jtitle) = env.new_string(title) else {
                    return;
                };
                match secondary_node_of(env, host) {
                    Some(node) => {
                        let _ = java_in(env, |env| {
                            DayBridge::setWindowTitle(env, node as i64, &jtitle)
                        });
                    }
                    // The primary is an ordinary window (docs/windows.md) and gets a recents
                    // card of its own, so it takes a label like any other; it just has no
                    // `day.node` to be addressed by.
                    None => {
                        let _ = java_in(env, |env| DayBridge::setPrimaryWindowTitle(env, &jtitle));
                    }
                }
            });
        }

        fn set_tooltip(&mut self, h: &AHandle, text: &str) {
            with_env(|env| {
                let text = jstr(env, text);
                let _ = java_in(env, |env| DayBridge::setTooltip(env, h.0.as_obj(), &text));
            });
        }
        fn open_popover(
            &mut self,
            anchor: &AHandle,
            content: &AHandle,
            size: Size,
            id: NodeId,
        ) -> bool {
            let d = density();
            with_env(|env| {
                java_in(env, |env| {
                    DayBridge::openPopover(
                        env,
                        anchor.0.as_obj(),
                        content.0.as_obj(),
                        (size.width * d).ceil() as i32,
                        (size.height * d).ceil() as i32,
                        id.0 as i64,
                    )
                })
            })
            .unwrap_or(false)
        }
        fn resize_popover(&mut self, content: &AHandle, size: Size) {
            let d = density();
            with_env(|env| {
                let _ = java_in(env, |env| {
                    DayBridge::resizePopover(
                        env,
                        content.0.as_obj(),
                        (size.width * d).ceil() as i32,
                        (size.height * d).ceil() as i32,
                    )
                });
            });
        }
        fn close_popover(&mut self, content: &AHandle) {
            with_env(|env| {
                let _ = java_in(env, |env| DayBridge::closePopover(env, content.0.as_obj()));
            });
        }
        fn show_toast(&mut self, h: &AHandle, text: &str) {
            with_env(|env| {
                let text = jstr(env, text);
                let _ = java_in(env, |env| DayBridge::showToast(env, h.0.as_obj(), &text));
            });
        }

        fn set_a11y(&mut self, h: &AHandle, a11y: &A11yProps) {
            // The role crosses as its `day_spec::Role` declaration index (DayBridge's
            // `ROLE_*` constants) with the heading level beside it; an empty string is an
            // unset member, and the bridge leaves those alone.
            let (role, level) = match a11y.role {
                Role::None => (0, 0),
                Role::Button => (1, 0),
                Role::Toggle => (2, 0),
                Role::Slider => (3, 0),
                Role::TextInput => (4, 0),
                Role::Heading(level) => (5, i32::from(level)),
                Role::Image => (6, 0),
                Role::Meter => (7, 0),
                Role::Group => (8, 0),
                Role::Tree => (9, 0),
                Role::TreeItem => (10, 0),
                Role::Tab => (11, 0),
                Role::TabList => (12, 0),
            };
            with_env(|env| {
                let label = jstr(env, a11y.label.as_deref().unwrap_or(""));
                let hint = jstr(env, a11y.hint.as_deref().unwrap_or(""));
                let value = jstr(env, a11y.value.as_deref().unwrap_or(""));
                let id = jstr(env, a11y.identifier.as_deref().unwrap_or(""));
                // -1 leaves the view's own selected state alone.
                let selected = a11y.selected.map_or(-1, i32::from);
                let _ = java_in(env, |env| {
                    DayBridge::setA11y(
                        env,
                        h.0.as_obj(),
                        &label,
                        &hint,
                        &value,
                        &id,
                        role,
                        level,
                        a11y.hidden,
                        selected,
                    )
                });
            });
        }

        /// `DayBridge.readNative`: every field in one call, as a `String[]` whose slots (the
        /// bridge's `RN_*` order) are null where the view was not read.
        fn read_native(&self, h: &AHandle) -> day_spec::NativeSnapshot {
            if !vm_ready() {
                return day_spec::NativeSnapshot::default();
            }
            let slots: Vec<Option<String>> = java(|env| {
                let arr = DayBridge::readNative(env, h.0.as_obj())?;
                let len = arr.len(env)?;
                let mut slots = Vec::with_capacity(len);
                for i in 0..len {
                    let s = arr.get_element(env, i)?;
                    slots.push(read_jstring(env, &s));
                }
                Ok(slots)
            })
            .unwrap_or_default();
            let slot = |i: usize| slots.get(i).cloned().flatten();
            let flag = |i: usize| slot(i).map(|s| s == "1");
            let found = flag(0) == Some(true);
            // `DayBridge.ROLE_*`, `set_a11y`'s numbering; the node carries no heading level.
            let role = match slot(1).and_then(|s| s.parse::<i32>().ok()) {
                Some(1) => Role::Button,
                Some(2) => Role::Toggle,
                Some(3) => Role::Slider,
                Some(4) => Role::TextInput,
                Some(5) => Role::Heading(1),
                Some(6) => Role::Image,
                Some(7) => Role::Meter,
                Some(8) => Role::Group,
                _ => Role::None,
            };
            let frame = slot(10).and_then(|s| {
                let v: Vec<f64> = s.split(',').filter_map(|n| n.parse().ok()).collect();
                match v[..] {
                    [x, y, w, h] => Some(Rect::new(x, y, w, h)),
                    _ => None,
                }
            });
            day_spec::NativeSnapshot {
                found,
                role,
                label: found.then(|| slot(2)).flatten(),
                value: found.then(|| slot(3)).flatten(),
                identifier: found.then(|| slot(4)).flatten().filter(|s| !s.is_empty()),
                text: slot(5),
                number: slot(6).and_then(|s| s.parse().ok()),
                checked: flag(7),
                enabled: flag(8),
                visible: flag(9),
                frame,
            }
        }

        fn announce(&mut self, text: &str, urgent: bool) {
            with_env(|env| {
                let text = jstr(env, text);
                let _ = java_in(env, |env| DayBridge::announce(env, &text, urgent));
            });
        }

        fn request_frame(
            &mut self,
            _host: &AHandle,
            cb: day_spec::FrameCallback,
        ) -> day_spec::CancelFrame {
            let token = day_core::frame::native::register(cb);
            bridge(|env| DayBridge::requestFrame(env, token as i64));
            Box::new(move || {
                day_core::frame::native::cancel(token);
                bridge(|env| DayBridge::cancelFrame(env, token as i64));
            })
        }

        /// The display list to `DayCanvasView` on three channels: the records, the texts, and
        /// the path and polygon points as numbers (`encode_ops_with_geometry`), which cross JNI
        /// as one `double[]` instead of being formatted here and parsed there every frame.
        fn replay(&mut self, h: &AHandle, ops: &[DrawOp], _size: Size) {
            let (nums, texts, geometry) = day_spec::encode_ops_with_geometry(ops);
            with_env(|env| {
                // Allocation failure (OOM-class) skips the frame instead of panicking out
                // of a JNI up-call (which aborts); the next replay redraws in full.
                let Ok(arr) = env.new_double_array(nums.len()) else {
                    return;
                };
                if arr.set_region(env, 0, &nums).is_err() {
                    return;
                }
                let Ok(geo) = env.new_double_array(geometry.len()) else {
                    return;
                };
                if geo.set_region(env, 0, &geometry).is_err() {
                    return;
                }
                let joined = jstr(env, &texts.join("\u{1f}"));
                let _ = java_in(env, |env| {
                    DayBridge::setCanvasOps(env, h.0.as_obj(), &arr, &joined, &geo)
                });
            });
        }

        /// `DayBridge.fontFamilies()`: the `fonts.xml` families in Day's list text
        /// (docs/fonts.md), one string across JNI like `locale_hints`.
        /// `DayBridge.snapshotOrigin`: the root's offset in the edge-to-edge content capture.
        fn snapshot_origin(&mut self, root: &AHandle) -> Option<Point> {
            if !vm_ready() {
                return None;
            }
            let text = with_env(|env| {
                let obj = DayBridge::snapshotOrigin(env, root.0.as_obj()).ok()?;
                if obj.is_null() {
                    return None;
                }
                string_of(env, &obj).ok()
            })?;
            let (x, y) = text.split_once(',')?;
            Some(Point::new(x.parse().ok()?, y.parse().ok()?))
        }

        fn font_families(&mut self) -> Vec<day_spec::FontFamilyInfo> {
            if !vm_ready() {
                return Vec::new();
            }
            let text = with_env(|env| {
                let obj = DayBridge::fontFamilies(env).ok()?;
                if obj.is_null() {
                    return None;
                }
                string_of(env, &obj).ok()
            });
            day_spec::parse_font_list(&text.unwrap_or_default())
        }

        /// `DayBridge.measureText(...)`: "width,height,ascent" in dp from the same `Paint`
        /// the canvas draws with (docs/fonts.md).
        fn measure_text(
            &mut self,
            text: &str,
            size: f64,
            font: &day_spec::CanvasFont,
        ) -> Option<day_spec::TextMetrics> {
            if !vm_ready() {
                return None;
            }
            let reply = with_env(|env| {
                let jtext = jstr(env, text);
                let jfamily = jstr(env, font.family_str());
                let weight = i32::from(font.css_weight());
                java_string(env, |env| {
                    DayBridge::measureText(env, &jtext, size, weight, font.italic, &jfamily)
                })
            })?;
            // The eight numbers `DayBridge.measureText` joins, in the order `from_slots` reads.
            let mut out = [0.0f64; 8];
            let mut it = reply.split(',');
            for slot in &mut out {
                *slot = it.next()?.trim().parse::<f64>().ok()?;
            }
            Some(day_spec::TextMetrics::from_slots(&out))
        }

        /// Decode bytes with `BitmapFactory` (docs/images.md), which reads every container
        /// Android knows: PNG, JPEG, WebP, GIF, BMP and HEIF on API 28+.
        ///
        /// The reply is the same comma-joined shape `measureText` uses, because that is this
        /// backend's established way to bring a few numbers back across JNI.
        fn decode_image(&mut self, req: u64, id: day_spec::BitmapId, bytes: &[u8]) {
            let reply = if vm_ready() {
                with_env(|env| {
                    let arr = env.byte_array_from_slice(bytes).ok()?;
                    java_string(env, |env| DayBridge::imageDecode(env, id.0 as i64, &arr))
                })
            } else {
                None
            };
            // "w,h,alpha", and unlike most backends here, the alpha answer is read from the
            // decoded bitmap rather than inferred from the container format.
            let parsed = reply.and_then(|r| {
                let mut it = r.split(',');
                let w = it.next()?.trim().parse::<f64>().ok()?;
                let h = it.next()?.trim().parse::<f64>().ok()?;
                let alpha = it.next()?.trim().parse::<i32>().ok()?;
                Some((w, h, alpha != 0))
            });
            let event = match parsed {
                Some((w, h, has_alpha)) => {
                    let info = day_spec::BitmapInfo {
                        pixels: Size::new(w, h),
                        // Decoded bytes carry no density; a PNG is its pixels.
                        scale: 1.0,
                        format: day_spec::ImageFormat::sniff(bytes),
                        has_alpha,
                    };
                    BITMAP_INFO.with(|m| m.borrow_mut().insert(id.0, info));
                    Event::ImageDecoded {
                        req,
                        result: Ok(info),
                    }
                }
                None => Event::ImageDecoded {
                    req,
                    result: Err(day_spec::ImageError::Decode),
                },
            };
            emit(day_spec::WINDOW_NODE, event);
        }

        fn image_info(&mut self, id: day_spec::BitmapId) -> Option<day_spec::BitmapInfo> {
            BITMAP_INFO.with(|m| m.borrow().get(&id.0).copied())
        }

        fn encode_image(&mut self, req: u64, id: day_spec::BitmapId, spec: &day_spec::EncodeSpec) {
            let result = encode_bitmap(id, spec);
            emit(day_spec::WINDOW_NODE, Event::ImageEncoded { req, result });
        }

        /// What `Bitmap.compress` writes under this scaffold's minSdk (24). Android reads more
        /// (WebP among them), and the asymmetry is the platform's own: `WEBP_LOSSY` and
        /// `WEBP_LOSSLESS` arrived in API 30, and the pre-30 `WEBP` constant is deprecated there.
        fn encode_formats(&mut self) -> Vec<day_spec::ImageFormat> {
            use day_spec::ImageFormat::{Jpeg, Png};
            vec![Png, Jpeg]
        }

        fn release_image(&mut self, id: day_spec::BitmapId) {
            BITMAP_INFO.with(|m| {
                m.borrow_mut().remove(&id.0);
            });
            if vm_ready() {
                with_env(|env| {
                    let _ = DayBridge::imageRelease(env, id.0 as i64);
                });
            }
        }

        /// Paint `ops` into a new `Bitmap` with the painter `replay` drives (`DayCanvasView`'s
        /// `OpPainter`), stored in `DayBridge.bitmaps` beside decoded bitmaps, so every image
        /// duty and the image piece take it (docs/canvas.md "Exporting").
        ///
        /// The whole job — encoding the ops for the wire, the JNI call, Skia's software paint
        /// into the `Bitmap` — runs on a `day_spec::offscreen` thread: a `Bitmap`-backed
        /// `Canvas` draws from any thread, and a map tile of a thousand paths took most of a
        /// second on a Pixel 6. `DayBridge.bitmaps` is a concurrent map, so the images the ops
        /// draw are read there directly and a bitmap that lands is visible to the UI thread at
        /// once; the answer, and the `BITMAP_INFO` entry before it, are posted back here.
        fn render_bitmap(
            &mut self,
            req: u64,
            id: day_spec::BitmapId,
            ops: &[DrawOp],
            size: Size,
            scale: f64,
        ) {
            if !vm_ready() {
                let result = Err(day_spec::ExportError::Unsupported);
                emit(day_spec::WINDOW_NODE, Event::BitmapRendered { req, result });
                return;
            }
            let (pw, ph) = day_spec::export_pixels(size, scale);
            let ops = ops.to_vec();
            day_spec::offscreen::spawn(move || {
                let result = day_spec::ffi_guard::contain(
                    Err(day_spec::ExportError::Failed(
                        "the offscreen painter panicked".into(),
                    )),
                    || export_bitmap(id, &ops, pw, ph, scale),
                );
                <Material as Platform>::post(Box::new(move || {
                    if let Ok(info) = result {
                        BITMAP_INFO.with(|m| m.borrow_mut().insert(id.0, info));
                    }
                    emit(day_spec::WINDOW_NODE, Event::BitmapRendered { req, result });
                }));
            });
        }

        /// One `PdfDocument` page per drawing, painted by the same `OpPainter`. The writer has
        /// no metadata API, so `info` (title, author) goes unwritten, and its pages take whole
        /// points.
        fn render_pdf(&mut self, req: u64, pages: &[day_spec::PdfPage], _info: &day_spec::PdfInfo) {
            let result = export_pdf(pages);
            emit(day_spec::WINDOW_NODE, Event::PdfRendered { req, result });
        }

        fn snapshot_window(&mut self) -> Result<Vec<u8>, String> {
            android_window_image(false)
        }

        /// The decor view rather than the content view: the window with its action bar and
        /// system-bar backgrounds (docs/window-image.md).
        fn snapshot_window_chrome(&mut self) -> Result<Vec<u8>, String> {
            android_window_image(true)
        }

        /// Apply an app-level appearance (docs/settings.md "Appearance on Android"). The uiMode
        /// change the platform makes comes straight back as a configuration change that
        /// `DayActivity` turns into `appearanceChanged()` and a recreation, so nothing has to
        /// be reported from here.
        fn set_appearance(&mut self, dark: Option<bool>) {
            let mode = match dark {
                Some(false) => 0i32,
                Some(true) => 1,
                None => 2,
            };
            with_env(|env| {
                let _ = DayBridge::setAppearance(env, mode);
            });
        }

        /// The theme's Material 3 color for each role (DayBridge.systemColor, by the role's
        /// wire number), so custom drawing matches the window and the status bar behind it.
        /// Success and warning have no Material 3 role and take Day's fallback, as every role
        /// does before a themed context exists.
        fn system_color(&mut self, role: day_spec::SystemColor) -> day_spec::Color {
            use day_spec::SystemColor;
            if matches!(role, SystemColor::Success | SystemColor::Warning) {
                return role.fallback(self.dark_mode());
            }
            let index = i32::from(role.wire());
            let argb = with_env(|env| DayBridge::systemColor(env, index).unwrap_or(0)) as u32;
            if argb == 0 {
                return role.fallback(self.dark_mode());
            }
            let channel = |shift: u32| f64::from((argb >> shift) & 0xff) / 255.0;
            day_spec::Color::rgba(channel(16), channel(8), channel(0), channel(24))
        }

        /// The system color mode, `DAY_THEME` override first (themed capture runs).
        fn dark_mode(&mut self) -> bool {
            match std::env::var("DAY_THEME").ok().as_deref() {
                Some("dark") => return true,
                Some("light") => return false,
                _ => {}
            }
            with_env(|env| DayBridge::isDarkMode(env).unwrap_or(false))
        }

        /// The user's "Remove animations" setting (docs/accessibility.md): the global animator
        /// duration scale read as zero (DayBridge.reduceMotion). The observer `DayActivity`
        /// registers reports each flip as `K_REDUCE_MOTION_CHANGED`, after which day-core asks
        /// here again.
        fn reduce_motion(&mut self) -> bool {
            with_env(|env| DayBridge::reduceMotion(env).unwrap_or(false))
        }

        /// The reader's font size setting for `style` (docs/text.md "Text scale"): the pixels
        /// Android gives the style's sp size over those it gives it at the default setting
        /// (DayBridge.textScale). That is `Configuration.fontScale`, except on Android 14+,
        /// whose non-linear scaling grows large sizes less, so the answer is per style. A
        /// change recreates the activity (DayActivity.onConfigurationChanged), and the next
        /// mount reports it ([`mount_settled`]).
        fn text_scale(&mut self, style: Font) -> f64 {
            let sp = font_style(style).0;
            let scale = java(|env| DayBridge::textScale(env, sp)).map_or(1.0, f64::from);
            if scale.is_finite() && scale > 0.0 {
                scale
            } else {
                1.0
            }
        }

        /// A widget's reading direction (docs/localization.md "Direction of a subtree"):
        /// `setLayoutDirection` + `setTextDirection` on the leaf (DayBridge.setViewDirection).
        /// Day's hosts are skipped: Day mirrors their children's frames itself, and the
        /// property on a view group only re-resolves start and end for those children.
        fn set_layout_direction(
            &mut self,
            h: &AHandle,
            kind: PieceKind,
            direction: day_spec::LayoutDirection,
        ) {
            if matches!(
                kind,
                kinds::CONTAINER
                    | kinds::SCROLL
                    | kinds::NAV
                    | kinds::NAV_PAGE
                    | kinds::LIST
                    | kinds::LIST_CELL
                    | kinds::TREE
                    | kinds::COVER
                    | kinds::INSPECTOR
                    | kinds::INSPECTOR_PANE
                    | kinds::SPLIT
                    | kinds::SPLIT_PANE
            ) {
                return;
            }
            let rtl = direction == day_spec::LayoutDirection::Rtl;
            bridge(|env| DayBridge::setViewDirection(env, h.0.as_obj(), rtl));
        }

        /// The system back, pressed (dayscript's `nav_back: { native: true }`): through the
        /// activity's `OnBackPressedDispatcher`, so the guard callback and the fragment
        /// manager's own pop run exactly as a gesture does (DayBridge.nativeBack). `false`
        /// when nothing would pop, since the dispatcher would finish the activity instead.
        fn native_back(&mut self) -> bool {
            with_env(|env| DayBridge::nativeBack(env).unwrap_or(false))
        }

        /// The app's own `OnBackPressedCallback`, beneath every other (DayBridge.appBack):
        /// enabled while an `on_system_back` handler is, so a back no cover or pushed page
        /// takes reaches Day instead of finishing the activity, and disabled otherwise, so the
        /// predictive "back to home" preview still shows when the press would leave.
        fn set_system_back(&mut self, armed: bool) {
            with_env(|env| {
                let _ = java_in(env, |env| DayBridge::setAppBack(env, armed));
            });
        }

        /// Whether native transitions have settled (dayscript screenshots wait on this):
        /// currently the cover slide, since a capture mid-present/mid-dismiss shows a half-slid
        /// surface (DayBridge.uiIdle / DayCover.slidesInFlight).
        fn prepare_snapshot(
            &mut self,
            host: Option<&Self::Handle>,
            revision: u32,
        ) -> Result<day_spec::capture::Readiness, String> {
            with_env(|env| {
                let null = JObject::null();
                let obj = host.map_or(&null, |h| h.0.as_obj());
                let ready = DayBridge::prepareSnapshot(env, obj, i64::from(revision))
                    .map_err(|e| format!("frame commit: {e}"))?;
                Ok(if ready == 1 {
                    day_spec::capture::Readiness::Ready
                } else {
                    day_spec::capture::Readiness::Pending
                })
            })
        }

        fn ui_idle(&mut self) -> bool {
            with_env(|env| DayBridge::uiIdle(env).unwrap_or(true))
        }
    }

    /// The `DayBridge.navState` key the full route of a mount being torn down travels under
    /// (docs/settings.md "Appearance on Android"). Reserved: `.restore` keys are the app's,
    /// and none of Day's own start with `day.`.
    const REMOUNT_ROUTE_KEY: &str = "day.route";

    /// `DayActivity` is about to recreate itself (a theme or language switch): save the full
    /// current route, so the re-mounted tree can come back to it. The activity recreation
    /// re-runs the app's `root()`, so every signal made there starts over; only a nav surface
    /// with a `.restore(key)` reads its place back by itself, and an app without one came back
    /// on its first tab. Saved straight into `DayBridge.navState`, not through the installed
    /// `NavStore`: the map is the activity's own instance state, exactly the lifetime a
    /// recreation needs, and an app that installed a disk-backed store must not find a
    /// transient route in it.
    fn save_route_for_remount(env: &mut Env) {
        let Some(route) = day_core::current_route() else {
            return;
        };
        let k = jstr(env, REMOUNT_ROUTE_KEY);
        let v = jstr(env, &route);
        let _ = java_in(env, |env| DayBridge::navSave(env, &k, &v));
    }

    /// Read and clear a `DayBridge.navState` entry (`DayBridge.navTake`).
    fn nav_take(env: &mut Env, key: &str) -> Option<String> {
        let k = jstr(env, key);
        java_string(env, |env| DayBridge::navTake(env, &k))
    }

    /// A mount's first build is done (`day::android::start`, after the tree is up). On a
    /// re-mount, put the app back where the previous mount left it; then tell the activity the
    /// tree has settled (`DayBridge.mountSettled`), which is what brings the recreation's
    /// crossfade down (docs/settings.md "Appearance on Android").
    ///
    /// Deferred one turn, behind the closure `launch_with` defers its own launch deep link
    /// into, for the same reason that one is: the first frame mounts before a destination
    /// pushes. A pending launch deep link wins over the saved route, as it wins over
    /// `.restore`; the saved route is consumed either way, so it can never apply to a later
    /// mount. A route a nav surface's `.restore` already restored is a no-op here, since the
    /// current route then equals the saved one.
    pub fn mount_settled(remount: bool) {
        day_reactive::on_main(move || {
            if remount {
                let saved = with_env(|env| nav_take(env, REMOUNT_ROUTE_KEY));
                if let Some(route) = saved {
                    if day_core::has_launch_deeplink() {
                        log::info!(
                            "day-toolkit-material: re-mount keeps the launch deep link over {route:?}"
                        );
                    } else if day_core::current_route().as_deref() != Some(route.as_str())
                        && !day_core::navigate(&route)
                    {
                        log::warn!(
                            "day-toolkit-material: re-mount could not restore route {route:?}"
                        );
                    }
                }
            }
            note_text_scale_of_mount();
            with_env(|env| {
                let _ = DayBridge::mountSettled(env);
            });
        });
    }

    thread_local! {
        /// The reader's font scale the previous mount was built at (`Toolkit::text_scale` for
        /// `Body`). Outlives a re-mount on purpose: it is what tells this mount whether the
        /// setting moved.
        static MOUNT_TEXT_SCALE: std::cell::Cell<Option<f64>> = const { std::cell::Cell::new(None) };
    }

    /// A change of the reader's font size recreates the activity
    /// (`DayActivity.onConfigurationChanged`), so the views come back at the new size. What lives above the
    /// tree does not: a `day::text_scale()` reader in the app's root scope, or a
    /// `max_text_scale` cap's font binding. When this mount's scale differs from the last one,
    /// tell day-core (`note_text_size_changed`), as UIKit does from its content-size
    /// notification.
    fn note_text_scale_of_mount() {
        let now = day_core::text_scale_of(Font::Body);
        let before = MOUNT_TEXT_SCALE.with(|c| c.replace(Some(now)));
        if let Some(b) = before.filter(|b| (b - now).abs() > 1e-6) {
            log::info!("day-toolkit-material: text scale {b:.2} -> {now:.2}, re-notifying");
            day_core::note_text_size_changed();
        }
    }

    /// Navigation persistence backed by the Activity's saved instance state (docs/navigation.md).
    /// The map lives on the Java side (`DayBridge.navState`) because that is where the platform
    /// hands it out and takes it back: `DayActivity` restores it in `onCreate` before native
    /// starts, and writes it into the outgoing Bundle in `onSaveInstanceState`. Its lifetime is
    /// the task's: a process the system reclaims comes back on the page the user left, while a
    /// cold launch, or a task the user swiped off Recents, starts clean.
    struct InstanceNavStore;

    impl day_core::NavStore for InstanceNavStore {
        fn load(&self, key: &str) -> Option<String> {
            if !vm_ready() {
                return None;
            }
            with_env(|env| {
                let k = jstr(env, key);
                java_string(env, |env| DayBridge::navLoad(env, &k))
            })
        }

        fn save(&self, key: &str, value: &str) {
            if !vm_ready() {
                return;
            }
            with_env(|env| {
                let k = jstr(env, key);
                let v = jstr(env, value);
                let _ = java_in(env, |env| DayBridge::navSave(env, &k, &v));
            });
        }
    }

    impl Platform for Material {
        const TARGET: &'static str = "android-material";
        const TOOLKIT: &'static str = "material";
        // The host owns the loop: `run` hands back at once, and DidExit is its last callback.
        const RUN_ENDS_APP: bool = false;

        fn run(self, _options: WindowOptions, ready: Box<dyn FnOnce(Self, AHandle, Size)>) {
            // The ActivityThread owns the loop; init() already registered the root.
            let (root, size) = ROOT
                .with(|r| r.borrow_mut().take())
                .expect("day-toolkit-material: init() not called");
            // Navigation restore is the platform's own instance-state contract here, not an app
            // opt-in (docs/navigation.md): every Android app is expected to come back where the
            // user left it after the system reclaims its process. Installed before `ready` builds
            // the tree, so the first build of a `.restore(key)` surface reads it; an app that
            // installs its own store later still replaces this one.
            day_core::set_nav_store(std::rc::Rc::new(InstanceNavStore));
            ready(self, root, size);
        }

        fn locale_hints(&self) -> Vec<String> {
            // The device's ordered language preference, which is the ambient locale Day
            // negotiates its catalogs against (§12.2, docs/localization.md). Comma-joined on the
            // Java side because a `String[]` return would need array marshaling for a list that
            // is never more than a handful of tags.
            if !vm_ready() {
                return Vec::new();
            }
            let joined = with_env(|env| {
                let obj = DayBridge::localeTags(env).ok()?;
                if obj.is_null() {
                    return None;
                }
                string_of(env, &obj).ok()
            });
            joined
                .unwrap_or_default()
                .split(',')
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string)
                .collect()
        }

        fn post(f: Box<dyn FnOnce() + Send>) {
            let token = Box::into_raw(Box::new(f)) as i64;
            // The binding loads DayBridge through the app class loader, so this works from the
            // native-spawned threads that see only the system loader.
            bridge(|env| DayBridge::postMain(env, token));
        }

        fn post_delayed(ms: u32, f: Box<dyn FnOnce() + Send>) {
            // The timer door behind `day::sleep` (docs/async.md): the main `Handler`'s
            // `postDelayed`, which hands the token back through `nativeRunPosted` exactly like
            // `postMain`. No helper thread; the delay travels as a `long`, whole.
            let token = Box::into_raw(Box::new(f)) as i64;
            bridge(|env| DayBridge::postMainDelayed(env, token, i64::from(ms)));
        }
    }
}
