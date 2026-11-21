// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The arithmetic behind surfaces and the canvas's shadows and conic gradients
//! (docs/surfaces.md, docs/canvas.md): everything that turns Day's units into the numbers ArkUI
//! and `OH_Drawing` take, kept free of the NDK so it runs under `cargo test` on the host.
//!
//! Colors are `0xAARRGGBB`, the form both APIs take.

/// Blend two `0xAARRGGBB` colors channel by channel: `k` = 0 is `a`, 1 is `b`.
#[must_use]
pub fn lerp_argb(a: u32, b: u32, k: f32) -> u32 {
    let k = k.clamp(0.0, 1.0);
    let mut out = 0u32;
    for shift in [24u32, 16, 8, 0] {
        let (x, y) = (((a >> shift) & 0xFF) as f32, ((b >> shift) & 0xFF) as f32);
        out |= (((x + (y - x) * k).round() as u32) & 0xFF) << shift;
    }
    out
}

/// `color` with its alpha multiplied by `k` (0..=1): a shadow's color under a translucent
/// drawing.
#[must_use]
pub fn scale_alpha(color: u32, k: f32) -> u32 {
    let alpha = ((color >> 24) as f32 * k.clamp(0.0, 1.0)).round() as u32;
    (alpha.min(255) << 24) | (color & 0x00FF_FFFF)
}

/// The color of ascending `(offset, color)` stops at `t`, flat before the first stop and after
/// the last. `None` for no stops.
#[must_use]
pub fn sample_stops(stops: &[(f32, u32)], t: f32) -> Option<u32> {
    let first = stops.first()?;
    if t <= first.0 {
        return Some(first.1);
    }
    for pair in stops.windows(2) {
        let ((a_at, a), (b_at, b)) = (pair[0], pair[1]);
        if t <= b_at {
            let k = if b_at > a_at {
                (t - a_at) / (b_at - a_at)
            } else {
                1.0
            };
            return Some(lerp_argb(a, b, k));
        }
    }
    stops.last().map(|s| s.1)
}

/// Stops spanning exactly `0..=1`: offsets clamped and made ascending, with the first color
/// carried back to 0 and the last out to 1.
#[must_use]
pub fn spanning(stops: &[(f32, u32)]) -> Vec<(f32, u32)> {
    let mut out: Vec<(f32, u32)> = Vec::with_capacity(stops.len() + 2);
    for &(at, color) in stops {
        let at = at.clamp(0.0, 1.0).max(out.last().map_or(0.0, |s| s.0));
        if out.is_empty() && at > 0.0 {
            out.push((0.0, color));
        }
        out.push((at, color));
    }
    if let Some(&(at, color)) = out.last()
        && at < 1.0
    {
        out.push((1.0, color));
    }
    out
}

/// A conic gradient's stops as a sweep gradient that starts on the +x axis takes them.
///
/// `OH_Drawing`'s sweep gradient always begins at 0°, and the variant that takes a local matrix
/// arrived after this backend's minimum API level. A full turn has no ends, though, so starting
/// at `start_deg` is the same picture as starting at 0° with every stop moved round by
/// `start_deg / 360` and the list cut where it passes a full turn. The cut gets the color the
/// gradient has there, on both sides, so nothing changes at 0°; the place the first and last
/// stops meet keeps its hard edge as two stops at one offset.
#[must_use]
pub fn sweep_stops(start_deg: f32, stops: &[(f32, u32)]) -> Vec<(f32, u32)> {
    const EPS: f32 = 1e-6;
    let stops = spanning(stops);
    let shift = (start_deg / 360.0).rem_euclid(1.0);
    if stops.is_empty() || shift < EPS || shift > 1.0 - EPS {
        return stops;
    }
    // The original offset that lands on 0° once everything has moved round by `shift`.
    let cut = 1.0 - shift;
    let at_cut: Vec<u32> = stops
        .iter()
        .filter(|s| (s.0 - cut).abs() <= EPS)
        .map(|s| s.1)
        .collect();
    let (below, above) = if let (Some(&lo), Some(&hi)) = (at_cut.first(), at_cut.last()) {
        (lo, hi)
    } else {
        let c = sample_stops(&stops, cut).unwrap_or(0);
        (c, c)
    };
    let mut out = Vec::with_capacity(stops.len() + 2);
    out.push((0.0, above));
    out.extend(
        stops
            .iter()
            .filter(|s| s.0 > cut + EPS)
            .map(|s| ((s.0 + shift - 1.0).clamp(0.0, 1.0), s.1)),
    );
    out.extend(
        stops
            .iter()
            .filter(|s| s.0 < cut - EPS)
            .map(|s| ((s.0 + shift).clamp(0.0, 1.0), s.1)),
    );
    out.push((1.0, below));
    // Two stops that share an offset can come back a rounding step apart: keep them ascending.
    let mut floor = 0.0f32;
    for stop in &mut out {
        floor = floor.max(stop.0);
        stop.0 = floor;
    }
    out
}

/// The linear part of the canvas transform Day has concatenated so far, in points: what a
/// shadow needs to stay untransformed (docs/canvas.md "Shadows"). Column vectors, the layout of
/// `day_geometry::Affine`: `x' = a·x + c·y`, `y' = b·x + d·y`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Linear {
    pub a: f32,
    pub b: f32,
    pub c: f32,
    pub d: f32,
}

impl Linear {
    pub const IDENTITY: Linear = Linear {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
    };

    /// This transform with `m` applied first: what a canvas `concat(m)` leaves.
    #[must_use]
    pub fn concat(self, m: Linear) -> Linear {
        Linear {
            a: self.a * m.a + self.c * m.b,
            b: self.b * m.a + self.d * m.b,
            c: self.a * m.c + self.c * m.d,
            d: self.b * m.c + self.d * m.d,
        }
    }

    /// The vector that this transform maps onto (`dx`, `dy`): how far to translate, in the
    /// current coordinates, to move a drawing by that many points on the canvas. `None` when
    /// the transform has collapsed and nothing it draws is visible.
    #[must_use]
    pub fn unmap(self, dx: f32, dy: f32) -> Option<(f32, f32)> {
        let det = self.a * self.d - self.b * self.c;
        if !det.is_finite() || det.abs() < 1e-9 {
            return None;
        }
        Some((
            (self.d * dx - self.c * dy) / det,
            (self.a * dy - self.b * dx) / det,
        ))
    }

    /// How much this transform enlarges a blur radius: the geometric mean of what it does to
    /// the two axes, the rule Skia scales a blur's sigma by.
    #[must_use]
    pub fn radius_scale(self) -> f32 {
        (self.a.hypot(self.b) * self.c.hypot(self.d)).sqrt()
    }
}

/// A surface's linear gradient as ArkUI's `NODE_LINEAR_GRADIENT` takes one: an angle in
/// degrees (0 = upward, clockwise, the CSS rule) and stops along the line CSS draws at that
/// angle through the middle of a `w` × `h` box.
///
/// Day's gradient runs between two unit points of the frame. The CSS line has the same direction
/// but its own length, reaching the corners, so each stop is moved to where it falls on that
/// line. Stops that land outside it are replaced by the colors at its two ends; the result
/// always spans `0..=1`.
#[must_use]
pub fn css_linear_gradient(
    start: (f64, f64),
    end: (f64, f64),
    stops: &[(f64, u32)],
    w: f64,
    h: f64,
) -> (f32, Vec<(f32, u32)>) {
    let flat = |color: u32| (180.0, vec![(0.0, color), (1.0, color)]);
    let Some(&(_, last)) = stops.last() else {
        return flat(0);
    };
    let (w, h) = (w.max(1e-6), h.max(1e-6));
    let (p0, p1) = ((start.0 * w, start.1 * h), (end.0 * w, end.1 * h));
    let len = (p1.0 - p0.0).hypot(p1.1 - p0.1);
    if !len.is_finite() || len < 1e-9 {
        return flat(last);
    }
    let u = ((p1.0 - p0.0) / len, (p1.1 - p0.1) / len);
    let angle = u.0.atan2(-u.1).to_degrees().rem_euclid(360.0);
    let css_len = (w * u.0).abs() + (h * u.1).abs();
    let css_start = (w / 2.0 - u.0 * css_len / 2.0, h / 2.0 - u.1 * css_len / 2.0);
    let lead = (p0.0 - css_start.0) * u.0 + (p0.1 - css_start.1) * u.1;
    let moved: Vec<(f32, u32)> = stops
        .iter()
        .map(|&(at, color)| (((at * len + lead) / css_len) as f32, color))
        .collect();
    let mut out = Vec::with_capacity(moved.len() + 2);
    out.push((0.0, sample_stops(&moved, 0.0).unwrap_or(last)));
    out.extend(moved.iter().filter(|s| s.0 > 0.0 && s.0 < 1.0));
    out.push((1.0, sample_stops(&moved, 1.0).unwrap_or(last)));
    (angle as f32, out)
}

/// The radius `NODE_CUSTOM_SHADOW` takes, in px, for a Day shadow blur of `blur` points.
///
/// ArkUI turns its radius into the Gaussian's standard deviation the way Skia does
/// (`0.57735 · radius + 0.5`), and Day's blur is twice the standard deviation, so the radius is
/// worked back from the sigma Day wants. ArkUI draws no shadow at all for a radius of zero, so
/// a hard-edged shadow asks for the smallest radius instead, half a pixel of softening.
#[must_use]
pub fn shadow_radius_px(blur: f64, density: f64) -> f32 {
    let sigma = blur.max(0.0) / 2.0 * density;
    (((sigma - 0.5) / 0.57735) as f32).max(0.01)
}

/// A surface's radial gradient (docs/surfaces.md) as the circle `NODE_RADIAL_GRADIENT` draws
/// in a `w` × `h` vp frame: the center in vp from the top-left corner and the radius in vp.
///
/// Day's radius is in the frame's unit space, so it is an ellipse in a frame that is not
/// square; ArkUI's gradient is a circle. The circle takes the radius along the frame's shorter
/// side, where the ellipse and the circle agree, and the last stop's color continues past it
/// along the longer one.
#[must_use]
pub fn radial_circle(center: (f64, f64), radius: f64, w: f64, h: f64) -> (f32, f32, f32) {
    let (w, h) = (w.max(0.0), h.max(0.0));
    (
        (center.0 * w) as f32,
        (center.1 * h) as f32,
        (radius.max(0.0) * w.min(h)) as f32,
    )
}

/// `radius` on the corners named by `bits` ([`day_spec::Corners::bits`]: 1 top-left, 2
/// top-right, 4 bottom-right, 8 bottom-left) in the order `NODE_BORDER_RADIUS` takes four
/// values: top-left, top-right, bottom-left, bottom-right.
#[must_use]
pub fn border_radii(bits: u8, radius: f32) -> [f32; 4] {
    let on = |bit: u8| if bits & bit != 0 { radius } else { 0.0 };
    [on(1), on(2), on(8), on(4)]
}

/// The `NODE_RESPONSE_REGION` that grows a `w` × `h` vp node by `slop` vp per edge (`[top,
/// leading, bottom, trailing]`, docs/gestures.md "Hit slop"): its offset from the node's top
/// left in vp and its size in percent of the node's own, the units the attribute takes.
#[must_use]
pub fn response_region(slop: [f64; 4], w: f64, h: f64) -> [f32; 4] {
    let [top, leading, bottom, trailing] = slop;
    if w <= 0.0 || h <= 0.0 {
        return [0.0, 0.0, 100.0, 100.0];
    }
    [
        -leading as f32,
        -top as f32,
        ((w + leading + trailing) / w * 100.0) as f32,
        ((h + top + bottom) / h * 100.0) as f32,
    ]
}

/// The phase of a key event on a platform that reports no repeat flag (ArkUI's key events
/// carry only down and up): a down for a key already down is the platform's auto-repeat, an
/// up only follows a down this tracker saw. Keys are tracked per node.
#[derive(Debug, Default)]
pub struct KeysDown {
    held: Vec<(u64, i32)>,
}

/// What [`KeysDown::note`] made of a key event.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyNote {
    /// The first down of a press.
    Down,
    /// A further down while the key is held.
    Repeat,
    /// The release of a key whose down was seen.
    Up,
    /// A release with no down seen for that node: nothing to report.
    Stray,
}

impl KeysDown {
    /// Note a down (`down = true`) or up of `code` on `node`.
    pub fn note(&mut self, node: u64, code: i32, down: bool) -> KeyNote {
        let held = self.held.iter().position(|&k| k == (node, code));
        match (down, held) {
            (true, Some(_)) => KeyNote::Repeat,
            (true, None) => {
                self.held.push((node, code));
                KeyNote::Down
            }
            (false, Some(i)) => {
                self.held.swap_remove(i);
                KeyNote::Up
            }
            (false, None) => KeyNote::Stray,
        }
    }

    /// Forget a down that was not claimed: its release and repeats are not the node's.
    pub fn forget(&mut self, node: u64, code: i32) {
        self.held.retain(|&k| k != (node, code));
    }

    /// Forget every key held on `node`: it lost focus, so no release reaches it.
    pub fn clear(&mut self, node: u64) {
        self.held.retain(|&(n, _)| n != node);
    }
}

/// Progress of a whole run of `cycles` cycles at `fraction` (0..=1) of its length: each cycle
/// follows `cycle` (one cycle's progress at its own fraction, `AnimSpec::cycle_progress`),
/// backwards on every second cycle when the run `autoreverse`s, and the very end is the target
/// (1), also after a backward cycle (docs/tween.md rule 5).
#[must_use]
pub fn run_progress(
    cycle: impl Fn(f64) -> f64,
    autoreverse: bool,
    cycles: u32,
    fraction: f64,
) -> f64 {
    let fraction = fraction.clamp(0.0, 1.0);
    if fraction >= 1.0 {
        return 1.0;
    }
    let t = fraction * f64::from(cycles.max(1));
    let index = t.floor();
    let p = cycle(t - index);
    if autoreverse && (index as u64) % 2 == 1 {
        1.0 - p
    } else {
        p
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_radial_gradient_is_the_circle_on_the_shorter_side() {
        assert_eq!(
            radial_circle((0.5, 0.5), 0.5, 80.0, 80.0),
            (40.0, 40.0, 40.0)
        );
        assert_eq!(
            radial_circle((0.25, 0.5), 0.5, 200.0, 80.0),
            (50.0, 40.0, 40.0)
        );
        assert_eq!(radial_circle((0.5, 0.5), 1.0, 0.0, 0.0), (0.0, 0.0, 0.0));
    }

    #[test]
    fn run_progress_plays_every_cycle_and_rests_on_the_target() {
        let linear = |x: f64| x;
        // Forward through the first cycle, back through the second, the target at the end.
        assert!((run_progress(linear, true, 2, 0.25) - 0.5).abs() < 1e-9);
        assert!((run_progress(linear, true, 2, 0.75) - 0.5).abs() < 1e-9);
        assert!((run_progress(linear, true, 2, 0.999) - 0.002).abs() < 1e-9);
        assert!((run_progress(linear, true, 2, 1.0) - 1.0).abs() < 1e-9);
        // Without autoreverse each cycle starts again.
        assert!((run_progress(linear, false, 3, 0.5) - 0.5).abs() < 1e-9);
        // An overshooting cycle keeps its overshoot, and the run ends exactly on 1.
        let ring = |x: f64| x + (x * std::f64::consts::PI).sin() * 0.5;
        assert!(run_progress(ring, false, 1, 0.8) > 1.0);
        assert!((run_progress(ring, false, 1, 1.0) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn response_region_grows_by_the_slop() {
        // A 20 × 10 node grown by 5 on top, 10 leading, 0 bottom, 10 trailing.
        let r = response_region([5.0, 10.0, 0.0, 10.0], 20.0, 10.0);
        assert_eq!(r, [-10.0, -5.0, 200.0, 150.0]);
        // No slop is the node itself; an unsized node keeps its own bounds.
        assert_eq!(
            response_region([0.0; 4], 20.0, 10.0),
            [0.0, 0.0, 100.0, 100.0]
        );
        assert_eq!(
            response_region([5.0; 4], 0.0, 10.0),
            [0.0, 0.0, 100.0, 100.0]
        );
    }

    #[test]
    fn keys_down_reads_repeats_and_releases() {
        let mut keys = KeysDown::default();
        assert_eq!(keys.note(1, 21, true), KeyNote::Down);
        assert_eq!(keys.note(1, 21, true), KeyNote::Repeat);
        assert_eq!(keys.note(1, 21, true), KeyNote::Repeat);
        // Another node's key is its own.
        assert_eq!(keys.note(2, 21, true), KeyNote::Down);
        assert_eq!(keys.note(1, 21, false), KeyNote::Up);
        assert_eq!(keys.note(1, 21, false), KeyNote::Stray);
        // Focus loss drops what the node held; an unclaimed down is forgotten.
        keys.clear(2);
        assert_eq!(keys.note(2, 21, false), KeyNote::Stray);
        assert_eq!(keys.note(3, 7, true), KeyNote::Down);
        keys.forget(3, 7);
        assert_eq!(keys.note(3, 7, false), KeyNote::Stray);
    }

    const RED: u32 = 0xFFFF_0000;
    const GREEN: u32 = 0xFF00_FF00;
    const BLUE: u32 = 0xFF00_00FF;
    const YELLOW: u32 = 0xFFFF_FF00;

    fn close(a: u32, b: u32) -> bool {
        [24u32, 16, 8, 0]
            .iter()
            .all(|s| ((a >> s) & 0xFF).abs_diff((b >> s) & 0xFF) <= 2)
    }

    /// What the gradient paints at `deg` when it starts at `start_deg`: the reference the
    /// rotated stop list has to reproduce.
    fn conic_at(start_deg: f32, stops: &[(f32, u32)], deg: f32) -> u32 {
        sample_stops(
            &spanning(stops),
            ((deg - start_deg) / 360.0).rem_euclid(1.0),
        )
        .unwrap()
    }

    #[test]
    fn lerp_blends_each_channel() {
        assert_eq!(lerp_argb(0xFF00_0000, 0xFFFF_FFFF, 0.0), 0xFF00_0000);
        assert_eq!(lerp_argb(0xFF00_0000, 0xFFFF_FFFF, 1.0), 0xFFFF_FFFF);
        assert_eq!(lerp_argb(0x0000_0000, 0xFF00_00FE, 0.5), 0x8000_007F);
    }

    #[test]
    fn scale_alpha_keeps_the_color() {
        assert_eq!(scale_alpha(0xFF12_3456, 1.0), 0xFF12_3456);
        assert_eq!(scale_alpha(0xFF12_3456, 0.5), 0x8012_3456);
        assert_eq!(scale_alpha(0x8012_3456, 0.5), 0x4012_3456);
        assert_eq!(scale_alpha(0xFF12_3456, 0.0), 0x0012_3456);
        assert_eq!(scale_alpha(0xFF12_3456, 7.0), 0xFF12_3456);
    }

    #[test]
    fn sweep_without_a_start_angle_keeps_its_stops() {
        let quarters = [
            (0.0, RED),
            (0.25, RED),
            (0.25, GREEN),
            (0.5, GREEN),
            (0.5, BLUE),
            (0.75, BLUE),
            (0.75, YELLOW),
            (1.0, YELLOW),
        ];
        assert_eq!(sweep_stops(0.0, &quarters), quarters.to_vec());
        assert_eq!(sweep_stops(360.0, &quarters), quarters.to_vec());
    }

    #[test]
    fn sweep_spans_the_whole_turn() {
        let s = sweep_stops(0.0, &[(0.25, RED), (0.5, BLUE)]);
        assert_eq!(s, vec![(0.0, RED), (0.25, RED), (0.5, BLUE), (1.0, BLUE)]);
    }

    #[test]
    fn rotated_sweep_matches_the_gradient_everywhere() {
        let cases: [&[(f32, u32)]; 3] = [
            &[(0.0, RED), (1.0, BLUE)],
            &[(0.0, RED), (0.75, GREEN)],
            &[
                (0.0, RED),
                (0.25, RED),
                (0.25, GREEN),
                (0.5, GREEN),
                (0.5, BLUE),
                (1.0, YELLOW),
            ],
        ];
        for stops in cases {
            for start in [30.0f32, 90.0, 135.0, 180.0, 271.5, -45.0, 725.0] {
                let rotated = sweep_stops(start, stops);
                assert_eq!(rotated.first().map(|s| s.0), Some(0.0));
                assert_eq!(rotated.last().map(|s| s.0), Some(1.0));
                assert!(rotated.windows(2).all(|p| p[0].0 <= p[1].0), "{rotated:?}");
                // Half-degree steps, offset so no sample sits on a hard edge.
                for step in 0..720 {
                    let deg = step as f32 * 0.5 + 0.13;
                    let got = sample_stops(&rotated, deg / 360.0).unwrap();
                    let want = conic_at(start, stops, deg);
                    assert!(
                        close(got, want),
                        "start {start} at {deg}: {got:08x} != {want:08x}"
                    );
                }
            }
        }
    }

    #[test]
    fn rotated_sweep_keeps_the_seam_hard() {
        // Red to blue from 90°: just before 90° is blue, just after is red.
        let rotated = sweep_stops(90.0, &[(0.0, RED), (1.0, BLUE)]);
        let seam: Vec<u32> = rotated
            .iter()
            .filter(|s| (s.0 - 0.25).abs() < 1e-6)
            .map(|s| s.1)
            .collect();
        assert_eq!(seam, vec![BLUE, RED]);
    }

    #[test]
    fn rotated_sweep_cut_on_a_hard_edge_keeps_both_colors() {
        // The edge at half a turn lands on 0° when the gradient starts at 180°.
        let rotated = sweep_stops(180.0, &[(0.0, RED), (0.5, RED), (0.5, BLUE), (1.0, BLUE)]);
        assert_eq!(rotated.first(), Some(&(0.0, BLUE)));
        assert_eq!(rotated.last(), Some(&(1.0, RED)));
    }

    #[test]
    fn linear_concat_and_unmap() {
        let scale = Linear::IDENTITY.concat(Linear {
            a: 2.0,
            b: 0.0,
            c: 0.0,
            d: 2.0,
        });
        // 20 points down the canvas is 10 units in a doubled coordinate system.
        assert_eq!(scale.unmap(0.0, 20.0), Some((0.0, 10.0)));
        assert!((scale.radius_scale() - 2.0).abs() < 1e-6);

        // A quarter turn clockwise (y down): local +x points down the canvas.
        let turn = Linear {
            a: 0.0,
            b: 1.0,
            c: -1.0,
            d: 0.0,
        };
        let (vx, vy) = turn.unmap(0.0, 6.0).unwrap();
        assert!((vx - 6.0).abs() < 1e-6 && vy.abs() < 1e-6);
        assert!((turn.radius_scale() - 1.0).abs() < 1e-6);

        // Whatever the transform, mapping the answer back gives the offset asked for.
        let m = scale.concat(turn).concat(Linear {
            a: 1.0,
            b: 0.3,
            c: -0.2,
            d: 1.5,
        });
        let (vx, vy) = m.unmap(3.0, -7.0).unwrap();
        assert!((m.a * vx + m.c * vy - 3.0).abs() < 1e-4);
        assert!((m.b * vx + m.d * vy + 7.0).abs() < 1e-4);
    }

    #[test]
    fn collapsed_transform_casts_no_shadow() {
        let flat = Linear {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 0.0,
        };
        assert_eq!(flat.unmap(1.0, 1.0), None);
    }

    #[test]
    fn axis_gradients_are_the_css_angles() {
        let stops = [(0.0, RED), (1.0, BLUE)];
        let (angle, s) = css_linear_gradient((0.5, 0.0), (0.5, 1.0), &stops, 80.0, 80.0);
        assert_eq!(angle, 180.0);
        assert_eq!(s, vec![(0.0, RED), (1.0, BLUE)]);
        let (angle, _) = css_linear_gradient((0.0, 0.5), (1.0, 0.5), &stops, 200.0, 40.0);
        assert_eq!(angle, 90.0);
        let (angle, _) = css_linear_gradient((0.5, 1.0), (0.5, 0.0), &stops, 200.0, 40.0);
        assert_eq!(angle, 0.0);
        let (angle, _) = css_linear_gradient((1.0, 0.5), (0.0, 0.5), &stops, 200.0, 40.0);
        assert_eq!(angle, 270.0);
    }

    #[test]
    fn diagonal_gradient_follows_the_frame() {
        let stops = [(0.0, RED), (1.0, BLUE)];
        // Corner to corner of a square is CSS's own 135° line, stops unmoved.
        let (angle, s) = css_linear_gradient((0.0, 0.0), (1.0, 1.0), &stops, 100.0, 100.0);
        assert!((angle - 135.0).abs() < 1e-4);
        assert!((s[0].0).abs() < 1e-6 && (s[s.len() - 1].0 - 1.0).abs() < 1e-6);
        assert_eq!((s[0].1, s[s.len() - 1].1), (RED, BLUE));
        // In a wide frame the same unit points give a flatter line.
        let (wide, _) = css_linear_gradient((0.0, 0.0), (1.0, 1.0), &stops, 300.0, 100.0);
        assert!((wide - (300.0f32).atan2(-100.0).to_degrees()).abs() < 1e-3);
    }

    /// Day's own answer for a point of a `w` × `h` frame, to hold the CSS form against.
    fn day_color(
        start: (f64, f64),
        end: (f64, f64),
        stops: &[(f64, u32)],
        (w, h): (f64, f64),
        p: (f64, f64),
    ) -> u32 {
        let (p0, p1) = ((start.0 * w, start.1 * h), (end.0 * w, end.1 * h));
        let d = (p1.0 - p0.0, p1.1 - p0.1);
        let t = ((p.0 - p0.0) * d.0 + (p.1 - p0.1) * d.1) / (d.0 * d.0 + d.1 * d.1);
        let s: Vec<(f32, u32)> = stops.iter().map(|s| (s.0 as f32, s.1)).collect();
        sample_stops(&s, t as f32).unwrap()
    }

    /// What CSS paints at a point for an angle and stops along its corner-reaching line.
    fn css_color(angle: f32, stops: &[(f32, u32)], w: f64, h: f64, p: (f64, f64)) -> u32 {
        let a = f64::from(angle).to_radians();
        let u = (a.sin(), -a.cos());
        let len = (w * u.0).abs() + (h * u.1).abs();
        let s = (w / 2.0 - u.0 * len / 2.0, h / 2.0 - u.1 * len / 2.0);
        let t = ((p.0 - s.0) * u.0 + (p.1 - s.1) * u.1) / len;
        sample_stops(stops, t as f32).unwrap()
    }

    #[test]
    fn css_form_paints_what_day_specifies() {
        let stops = [(0.0, RED), (0.4, GREEN), (1.0, BLUE)];
        let lines = [
            ((0.5, 0.0), (0.5, 1.0)),
            ((0.0, 0.0), (1.0, 1.0)),
            ((0.5, 0.25), (0.5, 0.75)),
            ((0.2, 0.9), (0.9, 0.1)),
            ((-0.5, 0.5), (1.5, 0.5)),
        ];
        for (w, h) in [(80.0, 80.0), (240.0, 60.0), (50.0, 300.0)] {
            for (start, end) in lines {
                let (angle, css) = css_linear_gradient(start, end, &stops, w, h);
                assert!(css.windows(2).all(|p| p[0].0 <= p[1].0), "{css:?}");
                for it in 0..=10 {
                    for ix in 0..=10 {
                        let unit = (f64::from(ix) / 10.0, f64::from(it) / 10.0);
                        let at = (unit.0 * w, unit.1 * h);
                        let want = day_color(start, end, &stops, (w, h), at);
                        let got = css_color(angle, &css, w, h, at);
                        assert!(
                            close(got, want),
                            "{start:?}->{end:?} in {w}x{h} at {unit:?}: {got:08x} != {want:08x}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn degenerate_gradient_is_flat() {
        let (_, s) = css_linear_gradient(
            (0.5, 0.5),
            (0.5, 0.5),
            &[(0.0, RED), (1.0, BLUE)],
            10.0,
            10.0,
        );
        assert_eq!(s, vec![(0.0, BLUE), (1.0, BLUE)]);
        let (_, s) = css_linear_gradient((0.0, 0.0), (1.0, 1.0), &[], 10.0, 10.0);
        assert_eq!(s.len(), 2);
    }

    #[test]
    fn shadow_radius_gives_the_sigma_day_wants() {
        // blur 4 points at 3 px per point: sigma 6 px.
        let r = shadow_radius_px(4.0, 3.0);
        assert!((0.57735 * r + 0.5 - 6.0).abs() < 1e-4);
        // A hard edge still has to be a shadow ArkUI draws.
        assert!(shadow_radius_px(0.0, 3.0) > 0.0);
        assert!(shadow_radius_px(-5.0, 3.0) > 0.0);
    }

    #[test]
    fn border_radii_are_in_arkui_order() {
        assert_eq!(border_radii(15, 8.0), [8.0; 4]);
        assert_eq!(border_radii(0, 8.0), [0.0; 4]);
        // Top corners: ArkUI's first two values.
        assert_eq!(border_radii(1 | 2, 8.0), [8.0, 8.0, 0.0, 0.0]);
        // Bottom-right is Day's bit 4 and ArkUI's fourth value.
        assert_eq!(border_radii(4, 8.0), [0.0, 0.0, 0.0, 8.0]);
        assert_eq!(border_radii(8, 8.0), [0.0, 0.0, 8.0, 0.0]);
    }
}
