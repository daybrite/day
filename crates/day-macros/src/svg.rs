// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! An SVG path-data parser, reduced to the four segment kinds a [`day_spec::Path`] carries.
//!
//! The grammar is SVG 1.1 §8.3 in full: every command in both cases (absolute and relative),
//! implicit repetition of the previous command, the "smooth" curve forms that reflect the last
//! control point, elliptical arcs, and SVG's number syntax including exponents, leading dots and
//! sign-separated runs (`1.5.5` is two numbers, `10-5` is two numbers).
//!
//! Arcs are converted to cubics here rather than added to the segment vocabulary: an arc is the
//! one SVG command with no counterpart in any of the nine 2-D APIs Day draws through, so
//! converting once at compile time beats nine conversions at draw time.

/// One parsed segment. Mirrors `day_spec::PathSeg`, but lives here because a proc-macro crate
/// cannot depend on the crate it generates code for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    Move(f64, f64),
    Line(f64, f64),
    Quad(f64, f64, f64, f64),
    Cubic(f64, f64, f64, f64, f64, f64),
    Close,
}

/// Where parsing stopped, for a compile error that points at the offending text.
#[derive(Debug)]
pub struct ParseError {
    pub at: usize,
    pub message: String,
}

impl ParseError {
    fn new(at: usize, message: impl Into<String>) -> Self {
        ParseError {
            at,
            message: message.into(),
        }
    }
}

struct Scanner<'a> {
    src: &'a [u8],
    i: usize,
}

impl<'a> Scanner<'a> {
    fn new(src: &'a str) -> Self {
        Scanner {
            src: src.as_bytes(),
            i: 0,
        }
    }
    fn done(&mut self) -> bool {
        self.skip_ws();
        self.i >= self.src.len()
    }
    fn peek(&self) -> Option<u8> {
        self.src.get(self.i).copied()
    }
    /// Whitespace and commas separate numbers interchangeably in SVG path data.
    fn skip_ws(&mut self) {
        while let Some(c) = self.peek() {
            if c == b',' || c.is_ascii_whitespace() {
                self.i += 1;
            } else {
                break;
            }
        }
    }
    /// One SVG number. Deliberately hand-scanned rather than split-then-parse: SVG allows
    /// numbers to run together with no separator when the sign or a second dot ends the previous
    /// one (`1.5.5`, `10-5`), which splitting on whitespace gets wrong.
    fn number(&mut self) -> Result<f64, ParseError> {
        self.skip_ws();
        let start = self.i;
        if matches!(self.peek(), Some(b'+') | Some(b'-')) {
            self.i += 1;
        }
        let mut seen_digit = false;
        while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
            seen_digit = true;
        }
        if self.peek() == Some(b'.') {
            self.i += 1;
            while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
                seen_digit = true;
            }
        }
        if !seen_digit {
            return Err(ParseError::new(start, "expected a number"));
        }
        // Exponent, but only when it is actually followed by digits: `1e` is a number then a
        // stray letter, not a malformed exponent.
        if matches!(self.peek(), Some(b'e') | Some(b'E')) {
            let save = self.i;
            self.i += 1;
            if matches!(self.peek(), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            if matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                while matches!(self.peek(), Some(c) if c.is_ascii_digit()) {
                    self.i += 1;
                }
            } else {
                self.i = save;
            }
        }
        let text = std::str::from_utf8(&self.src[start..self.i])
            .map_err(|_| ParseError::new(start, "path data is not valid UTF-8"))?;
        text.parse::<f64>()
            .map_err(|_| ParseError::new(start, format!("`{text}` is not a number")))
    }
    /// An SVG flag: exactly one `0` or `1`, used by the arc command's two flags.
    fn flag(&mut self) -> Result<bool, ParseError> {
        self.skip_ws();
        match self.peek() {
            Some(b'0') => {
                self.i += 1;
                Ok(false)
            }
            Some(b'1') => {
                self.i += 1;
                Ok(true)
            }
            _ => Err(ParseError::new(self.i, "expected a 0 or 1 arc flag")),
        }
    }
}

/// Parse SVG path data into absolute segments.
pub fn parse(data: &str) -> Result<Vec<Seg>, ParseError> {
    let mut sc = Scanner::new(data);
    let mut out: Vec<Seg> = Vec::new();
    // Current point, current subpath start, and the previous curve's control point (for the
    // smooth forms, which reflect it).
    let (mut cx, mut cy) = (0.0f64, 0.0f64);
    let (mut sx, mut sy) = (0.0f64, 0.0f64);
    let (mut last_cubic_ctl, mut last_quad_ctl) = (None::<(f64, f64)>, None::<(f64, f64)>);
    let mut cmd = 0u8;

    while !sc.done() {
        let c = sc.peek().unwrap_or(b'\0');
        if c.is_ascii_alphabetic() {
            cmd = c;
            sc.i += 1;
        } else if cmd == 0 {
            return Err(ParseError::new(sc.i, "path data must start with a command"));
        } else if cmd == b'M' {
            // An implicit repeat of moveto is a lineto, per the spec.
            cmd = b'L';
        } else if cmd == b'm' {
            cmd = b'l';
        }
        let rel = cmd.is_ascii_lowercase();
        let (ox, oy) = if rel { (cx, cy) } else { (0.0, 0.0) };
        match cmd.to_ascii_uppercase() {
            b'M' => {
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Move(x, y));
                (cx, cy) = (x, y);
                (sx, sy) = (x, y);
                (last_cubic_ctl, last_quad_ctl) = (None, None);
            }
            b'L' => {
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Line(x, y));
                (cx, cy) = (x, y);
                (last_cubic_ctl, last_quad_ctl) = (None, None);
            }
            b'H' => {
                let x = sc.number()? + ox;
                out.push(Seg::Line(x, cy));
                cx = x;
                (last_cubic_ctl, last_quad_ctl) = (None, None);
            }
            b'V' => {
                let y = sc.number()? + oy;
                out.push(Seg::Line(cx, y));
                cy = y;
                (last_cubic_ctl, last_quad_ctl) = (None, None);
            }
            b'C' => {
                let (x1, y1) = (sc.number()? + ox, sc.number()? + oy);
                let (x2, y2) = (sc.number()? + ox, sc.number()? + oy);
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Cubic(x1, y1, x2, y2, x, y));
                (cx, cy) = (x, y);
                last_cubic_ctl = Some((x2, y2));
                last_quad_ctl = None;
            }
            b'S' => {
                // Smooth cubic: the first control point mirrors the previous one about the
                // current point; with no previous cubic it coincides with the current point.
                let (x1, y1) = match last_cubic_ctl {
                    Some((px, py)) => (2.0 * cx - px, 2.0 * cy - py),
                    None => (cx, cy),
                };
                let (x2, y2) = (sc.number()? + ox, sc.number()? + oy);
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Cubic(x1, y1, x2, y2, x, y));
                (cx, cy) = (x, y);
                last_cubic_ctl = Some((x2, y2));
                last_quad_ctl = None;
            }
            b'Q' => {
                let (x1, y1) = (sc.number()? + ox, sc.number()? + oy);
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Quad(x1, y1, x, y));
                (cx, cy) = (x, y);
                last_quad_ctl = Some((x1, y1));
                last_cubic_ctl = None;
            }
            b'T' => {
                let (x1, y1) = match last_quad_ctl {
                    Some((px, py)) => (2.0 * cx - px, 2.0 * cy - py),
                    None => (cx, cy),
                };
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                out.push(Seg::Quad(x1, y1, x, y));
                (cx, cy) = (x, y);
                last_quad_ctl = Some((x1, y1));
                last_cubic_ctl = None;
            }
            b'A' => {
                let (rx, ry) = (sc.number()?, sc.number()?);
                let rot = sc.number()?;
                let large = sc.flag()?;
                let sweep = sc.flag()?;
                let (x, y) = (sc.number()? + ox, sc.number()? + oy);
                arc_to_cubics(cx, cy, rx, ry, rot, large, sweep, x, y, &mut out);
                (cx, cy) = (x, y);
                (last_cubic_ctl, last_quad_ctl) = (None, None);
            }
            b'Z' => {
                out.push(Seg::Close);
                (cx, cy) = (sx, sy);
                (last_cubic_ctl, last_quad_ctl) = (None, None);
            }
            other => {
                return Err(ParseError::new(
                    sc.i.saturating_sub(1),
                    format!("`{}` is not an SVG path command", other as char),
                ));
            }
        }
    }
    Ok(out)
}

/// Convert an elliptical arc to cubics, following SVG 1.1 §F.6 (endpoint to centre
/// parameterization), then splitting the sweep into pieces of at most 90° because a single cubic
/// only approximates a circular arc well up to about that.
#[allow(clippy::too_many_arguments)]
fn arc_to_cubics(
    x1: f64,
    y1: f64,
    rx: f64,
    ry: f64,
    rot_deg: f64,
    large: bool,
    sweep: bool,
    x2: f64,
    y2: f64,
    out: &mut Vec<Seg>,
) {
    // Degenerate cases the spec calls out: a zero-length arc draws nothing, and a zero radius
    // makes it a straight line.
    if (x1 - x2).abs() < f64::EPSILON && (y1 - y2).abs() < f64::EPSILON {
        return;
    }
    let (mut rx, mut ry) = (rx.abs(), ry.abs());
    if rx < f64::EPSILON || ry < f64::EPSILON {
        out.push(Seg::Line(x2, y2));
        return;
    }
    let phi = rot_deg.to_radians();
    let (sin_phi, cos_phi) = phi.sin_cos();
    let dx2 = (x1 - x2) / 2.0;
    let dy2 = (y1 - y2) / 2.0;
    let x1p = cos_phi * dx2 + sin_phi * dy2;
    let y1p = -sin_phi * dx2 + cos_phi * dy2;

    // §F.6.6: scale the radii up when they are too small to span the endpoints.
    let lambda = (x1p * x1p) / (rx * rx) + (y1p * y1p) / (ry * ry);
    if lambda > 1.0 {
        let s = lambda.sqrt();
        rx *= s;
        ry *= s;
    }

    let sign = if large == sweep { -1.0 } else { 1.0 };
    let num = (rx * rx) * (ry * ry) - (rx * rx) * (y1p * y1p) - (ry * ry) * (x1p * x1p);
    let den = (rx * rx) * (y1p * y1p) + (ry * ry) * (x1p * x1p);
    let coef = sign * (num / den).max(0.0).sqrt();
    let cxp = coef * (rx * y1p) / ry;
    let cyp = coef * -(ry * x1p) / rx;
    let cx = cos_phi * cxp - sin_phi * cyp + (x1 + x2) / 2.0;
    let cy = sin_phi * cxp + cos_phi * cyp + (y1 + y2) / 2.0;

    let angle = |ux: f64, uy: f64, vx: f64, vy: f64| {
        let dot = ux * vx + uy * vy;
        let len = ((ux * ux + uy * uy) * (vx * vx + vy * vy)).sqrt();
        let mut a = (dot / len).clamp(-1.0, 1.0).acos();
        if ux * vy - uy * vx < 0.0 {
            a = -a;
        }
        a
    };
    let ux = (x1p - cxp) / rx;
    let uy = (y1p - cyp) / ry;
    let vx = (-x1p - cxp) / rx;
    let vy = (-y1p - cyp) / ry;
    let theta1 = angle(1.0, 0.0, ux, uy);
    let mut delta = angle(ux, uy, vx, vy);
    if !sweep && delta > 0.0 {
        delta -= std::f64::consts::TAU;
    } else if sweep && delta < 0.0 {
        delta += std::f64::consts::TAU;
    }

    let pieces = (delta.abs() / std::f64::consts::FRAC_PI_2).ceil().max(1.0) as usize;
    let step = delta / pieces as f64;
    // The cubic control-point distance for an arc of `step` radians.
    let alpha = (4.0 / 3.0) * (step / 4.0).tan();
    let mut theta = theta1;
    for _ in 0..pieces {
        let (s1, c1) = theta.sin_cos();
        let (s2, c2) = (theta + step).sin_cos();
        // Point and tangent on the unit circle, mapped through the ellipse and its rotation.
        let map = |c: f64, s: f64| {
            (
                cx + rx * c * cos_phi - ry * s * sin_phi,
                cy + rx * c * sin_phi + ry * s * cos_phi,
            )
        };
        let dmap = |c: f64, s: f64| {
            (
                -rx * s * cos_phi - ry * c * sin_phi,
                -rx * s * sin_phi + ry * c * cos_phi,
            )
        };
        let (px, py) = map(c1, s1);
        let (qx, qy) = map(c2, s2);
        let (d1x, d1y) = dmap(c1, s1);
        let (d2x, d2y) = dmap(c2, s2);
        out.push(Seg::Cubic(
            px + alpha * d1x,
            py + alpha * d1y,
            qx - alpha * d2x,
            qy - alpha * d2y,
            qx,
            qy,
        ));
        theta += step;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absolute_commands_parse() {
        let segs = parse("M 20,20 C 80,60 100,40 120,20 Z").unwrap();
        assert_eq!(
            segs,
            vec![
                Seg::Move(20.0, 20.0),
                Seg::Cubic(80.0, 60.0, 100.0, 40.0, 120.0, 20.0),
                Seg::Close,
            ]
        );
    }

    #[test]
    fn relative_commands_accumulate() {
        let segs = parse("m 10,10 l 5,5 h 10 v -5").unwrap();
        assert_eq!(
            segs,
            vec![
                Seg::Move(10.0, 10.0),
                Seg::Line(15.0, 15.0),
                Seg::Line(25.0, 15.0),
                Seg::Line(25.0, 10.0),
            ]
        );
    }

    #[test]
    fn a_repeated_moveto_becomes_a_lineto() {
        let segs = parse("M 0,0 10,0 20,0").unwrap();
        assert_eq!(
            segs,
            vec![
                Seg::Move(0.0, 0.0),
                Seg::Line(10.0, 0.0),
                Seg::Line(20.0, 0.0),
            ]
        );
    }

    #[test]
    fn smooth_curves_reflect_the_previous_control_point() {
        // The S's first control point must be the C's second, mirrored about (20,0).
        let segs = parse("M 0,0 C 5,10 15,10 20,0 S 35,-10 40,0").unwrap();
        assert_eq!(segs[2], Seg::Cubic(25.0, -10.0, 35.0, -10.0, 40.0, 0.0));
    }

    #[test]
    fn numbers_may_run_together() {
        // `.5.5` is two numbers, and `10-5` is 10 then -5 — SVG's implicit separators.
        let segs = parse("M .5.5 L 10-5").unwrap();
        assert_eq!(segs, vec![Seg::Move(0.5, 0.5), Seg::Line(10.0, -5.0)]);
    }

    #[test]
    fn exponents_parse() {
        let segs = parse("M 1e2,2E-1").unwrap();
        assert_eq!(segs, vec![Seg::Move(100.0, 0.2)]);
    }

    #[test]
    fn a_half_circle_arc_becomes_cubics_that_end_where_asked() {
        let segs = parse("M 0,0 A 10,10 0 0 1 20,0").unwrap();
        assert!(segs.len() > 1, "the arc must emit at least one cubic");
        match segs.last().unwrap() {
            Seg::Cubic(_, _, _, _, x, y) => {
                assert!((x - 20.0).abs() < 1e-9, "{x}");
                assert!(y.abs() < 1e-9, "{y}");
            }
            other => panic!("expected a cubic, got {other:?}"),
        }
        // The sweep passes BELOW the axis for sweep-flag 1 in SVG's y-down space.
        let mid = match segs[1] {
            Seg::Cubic(_, y1, _, _, _, _) => y1,
            _ => unreachable!(),
        };
        assert!(mid < 0.0, "sweep 1 curves to negative y: {mid}");
    }

    #[test]
    fn a_zero_radius_arc_is_a_line() {
        let segs = parse("M 0,0 A 0,0 0 0 1 10,10").unwrap();
        assert_eq!(segs, vec![Seg::Move(0.0, 0.0), Seg::Line(10.0, 10.0)]);
    }

    #[test]
    fn errors_point_at_the_offending_text() {
        // Data that opens with a coordinate has no command to apply it to.
        assert!(parse("10,10 L 2,2").is_err(), "must start with a command");
        let e = parse("M 0,0 X 5").unwrap_err();
        assert!(e.message.contains('X'), "{}", e.message);
        // A truncated command reports the missing number rather than drawing half a segment.
        assert!(parse("M 0,0 C 1,1 2,2").is_err(), "cubic wants three pairs");
    }

    /// Starting with something other than a moveto is out of spec, but every renderer treats the
    /// current point as the origin and so does this — rejecting it would fail on real-world data
    /// for no benefit.
    #[test]
    fn a_missing_initial_moveto_starts_at_the_origin() {
        assert_eq!(parse("L 10,10").unwrap(), vec![Seg::Line(10.0, 10.0)]);
    }
}
