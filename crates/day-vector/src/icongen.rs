// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Seeded app-icon generator (docs/icons.md): one `u64` seed → a deterministic layered SVG
//! master (`day:background` / `day:foreground` / `day:monochrome`, the contract `day icon`
//! consumes), designed to read well through every downstream form — iOS squircle, Android
//! adaptive + themed monochrome, plain PNG.
//!
//! The compositions encode the published icon-design guidance rather than free-form noise:
//!
//! * **One or two focal points in simple geometry** — icons are judged at small sizes, so a
//!   single dominant motif with at most a couple of supporting accents (Apple HIG).
//! * **Safe-zone placement** — primary content stays inside the central region so the iOS
//!   squircle and Android circle masks never clip it; the backdrop alone bleeds full-canvas.
//! * **A limited, harmonious palette** — a background tone plus at most two accent hues,
//!   drawn from the classic color-harmony schemes (analogous, complementary,
//!   split-complementary, triadic) with saturation/lightness held to bands that keep
//!   figure-ground contrast high on both dark and light backdrops.
//! * **Flat or subtly gradient backgrounds** — a gentle vertical two-stop gradient of one
//!   hue (the HIG's "subtle top-to-bottom gradient adds depth without looking dated").
//! * **Balance** — compositions are either symmetric (centered, rotational) or
//!   golden-section asymmetric with a small counterweight, the two classical routes to
//!   visual equilibrium.
//!
//! Determinism is part of the contract: `day new` seeds from the app id so the same id
//! always regenerates the same icon, and `day icon --generate --seed N` reproduces exactly.

use std::fmt::Write as _;

/// Canvas edge in user units. All downstream renders scale from the viewBox, so the exact
/// number only sets the coordinate vocabulary below.
const EDGE: f32 = 1024.0;
const CENTER: f32 = EDGE / 2.0;

/// Hash an arbitrary string (an app id, a pet name) into a seed — FNV-1a 64, hand-rolled so
/// the mapping is stable across Rust versions (std's hashers are randomly keyed).
pub fn seed_from_str(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// splitmix64 — tiny, well-distributed, and dependency-free; every aesthetic choice below
/// draws from this stream in a fixed order, which is what makes a seed reproducible.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    /// Uniform in `[0, 1)`.
    fn f(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    /// Uniform in `[lo, hi)`.
    fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + self.f() * (hi - lo)
    }
    /// Uniform integer in `[0, n)`.
    fn pick(&mut self, n: u32) -> u32 {
        (self.next() % u64::from(n)) as u32
    }
    fn chance(&mut self, p: f32) -> bool {
        self.f() < p
    }
}

/// HSL (h in degrees, s/l in 0..1) → `#rrggbb`. Hand-rolled: palettes are authored in HSL
/// because the harmony schemes are angle arithmetic on the hue wheel.
fn hsl(h: f32, s: f32, l: f32) -> String {
    let h = h.rem_euclid(360.0);
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = l - c / 2.0;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let q = |v: f32| ((v + m).clamp(0.0, 1.0) * 255.0).round() as u8;
    format!("#{:02x}{:02x}{:02x}", q(r), q(g), q(b))
}

/// The palette: a background gradient pair plus two accents and an "ink" (the near-neutral
/// detail color). Accent lightness bands are chosen against the backdrop so figure-ground
/// contrast holds by construction.
struct Palette {
    bg_top: String,
    bg_bottom: String,
    a: String,
    b: String,
    ink: String,
    dark: bool,
}

fn palette(rng: &mut Rng) -> Palette {
    let base = rng.f() * 360.0;
    // Classic harmony schemes: the second accent's hue offset from the first.
    let offset = match rng.pick(4) {
        0 => {
            // Analogous: adjacent hues, cohesive and calm.
            if rng.chance(0.5) { 30.0 } else { -30.0 }
        }
        1 => 180.0, // complementary: maximum hue tension, still consonant
        2 => {
            // Split-complementary: the softer complement.
            if rng.chance(0.5) { 150.0 } else { 210.0 }
        }
        _ => {
            // Triadic.
            if rng.chance(0.5) { 120.0 } else { 240.0 }
        }
    };
    let dark = rng.chance(0.62);
    let drift = rng.range(-14.0, 14.0);
    let (bg_top, bg_bottom) = if dark {
        let s = rng.range(0.30, 0.55);
        (
            hsl(base + drift, s, rng.range(0.24, 0.32)),
            hsl(base, s, rng.range(0.12, 0.18)),
        )
    } else {
        let s = rng.range(0.25, 0.50);
        (
            hsl(base + drift, s, rng.range(0.93, 0.97)),
            hsl(base, s, rng.range(0.84, 0.90)),
        )
    };
    let acc = |rng: &mut Rng, h: f32| {
        if dark {
            hsl(h, rng.range(0.62, 0.85), rng.range(0.56, 0.70))
        } else {
            hsl(h, rng.range(0.55, 0.80), rng.range(0.38, 0.50))
        }
    };
    let a = acc(rng, base);
    let b = acc(rng, base + offset);
    let ink = if dark {
        hsl(base, rng.range(0.08, 0.20), rng.range(0.92, 0.97))
    } else {
        hsl(base, rng.range(0.30, 0.50), rng.range(0.16, 0.26))
    };
    Palette {
        bg_top,
        bg_bottom,
        a,
        b,
        ink,
        dark,
    }
}

/// One drawable element of the composition. `core` marks the shapes that carry the icon's
/// identity — the monochrome layer re-emits exactly those as a single-color silhouette and
/// drops the decorative rest (halos, glows, low-alpha accents).
struct Shape {
    kind: Kind,
    fill: String,
    opacity: f32,
    core: bool,
}

enum Kind {
    Circle {
        cx: f32,
        cy: f32,
        r: f32,
    },
    /// Stroked circle; `dash` < 1.0 leaves a gap (an open arc), rotated by `rot`.
    Ring {
        cx: f32,
        cy: f32,
        r: f32,
        width: f32,
        dash: f32,
        rot: f32,
    },
    /// Rounded rect centered at (cx, cy), rotated by `rot` degrees.
    Rect {
        cx: f32,
        cy: f32,
        w: f32,
        h: f32,
        rx: f32,
        rot: f32,
    },
    /// Semicircle (flat edge through the center line), rotated by `rot`.
    Semi {
        cx: f32,
        cy: f32,
        r: f32,
        rot: f32,
    },
}

impl Shape {
    /// Emit as SVG. `mono` overrides every color with black and squashes opacity to 1 —
    /// the themed-icon silhouette (the platform supplies the tint).
    fn svg(&self, mono: bool) -> String {
        let fill = if mono { "#000000" } else { self.fill.as_str() };
        let op = if mono || self.opacity >= 0.999 {
            String::new()
        } else {
            format!(" opacity=\"{:.2}\"", self.opacity)
        };
        match self.kind {
            Kind::Circle { cx, cy, r } => {
                format!("<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"{fill}\"{op}/>")
            }
            Kind::Ring {
                cx,
                cy,
                r,
                width,
                dash,
                rot,
            } => {
                let circ = std::f32::consts::TAU * r;
                let dasharray = if dash < 0.999 {
                    format!(
                        " stroke-dasharray=\"{:.1} {:.1}\" stroke-linecap=\"round\"",
                        circ * dash,
                        circ,
                    )
                } else {
                    String::new()
                };
                format!(
                    "<circle cx=\"{cx:.1}\" cy=\"{cy:.1}\" r=\"{r:.1}\" fill=\"none\" \
                     stroke=\"{fill}\" stroke-width=\"{width:.1}\"{dasharray}{op} \
                     transform=\"rotate({rot:.1} {cx:.1} {cy:.1})\"/>"
                )
            }
            Kind::Rect {
                cx,
                cy,
                w,
                h,
                rx,
                rot,
            } => {
                let x = cx - w / 2.0;
                let y = cy - h / 2.0;
                let t = if rot.abs() > 0.01 {
                    format!(" transform=\"rotate({rot:.1} {cx:.1} {cy:.1})\"")
                } else {
                    String::new()
                };
                format!(
                    "<rect x=\"{x:.1}\" y=\"{y:.1}\" width=\"{w:.1}\" height=\"{h:.1}\" \
                     rx=\"{rx:.1}\" fill=\"{fill}\"{op}{t}/>"
                )
            }
            Kind::Semi { cx, cy, r, rot } => {
                format!(
                    "<path d=\"M {} {cy:.1} A {r:.1} {r:.1} 0 0 1 {} {cy:.1} Z\" \
                     fill=\"{fill}\"{op} transform=\"rotate({rot:.1} {cx:.1} {cy:.1})\"/>",
                    cx - r,
                    cx + r,
                )
            }
        }
    }
}

/// The dominant-motif vocabulary: filled, compact, and legible at 16 px.
fn motif(rng: &mut Rng, cx: f32, cy: f32, r: f32, fill: &str) -> Shape {
    let kind = match rng.pick(5) {
        0 => Kind::Circle { cx, cy, r },
        1 => Kind::Ring {
            cx,
            cy,
            r: r * 0.82,
            width: r * 0.36,
            dash: 1.0,
            rot: 0.0,
        },
        2 => Kind::Rect {
            cx,
            cy,
            w: r * 1.84,
            h: r * 1.84,
            rx: r * 0.55,
            rot: if rng.chance(0.3) { 45.0 } else { 0.0 },
        },
        3 => Kind::Rect {
            cx,
            cy,
            w: r * 2.0,
            h: r * 1.15,
            rx: r * 0.575,
            rot: rng.range(-30.0, 30.0),
        },
        _ => Kind::Semi {
            cx,
            cy: cy + r * 0.25,
            r: r * 1.05,
            rot: *[0.0, 180.0, -90.0, 90.0]
                .get(rng.pick(4) as usize)
                .unwrap_or(&0.0),
        },
    };
    Shape {
        kind,
        fill: fill.to_string(),
        opacity: 1.0,
        core: true,
    }
}

/// Compose the foreground: a `Vec<Shape>` whose `core` subset is also the monochrome
/// silhouette. Templates are the two classical balance strategies — symmetry (centered,
/// rotational, stacked) and golden-section asymmetry with a counterweight.
fn compose(rng: &mut Rng, p: &Palette) -> Vec<Shape> {
    let mut shapes = Vec::new();
    match rng.pick(5) {
        // Centered: one dominant motif, optional halo ring behind it.
        0 => {
            let r = rng.range(240.0, 300.0);
            if rng.chance(0.55) {
                shapes.push(Shape {
                    kind: Kind::Ring {
                        cx: CENTER,
                        cy: CENTER,
                        r: r + rng.range(60.0, 90.0),
                        width: rng.range(20.0, 34.0),
                        dash: if rng.chance(0.5) {
                            rng.range(0.55, 0.8)
                        } else {
                            1.0
                        },
                        rot: rng.f() * 360.0,
                    },
                    fill: p.b.clone(),
                    opacity: 0.85,
                    core: false,
                });
            }
            shapes.push(motif(rng, CENTER, CENTER, r, &p.a));
            if rng.chance(0.5) {
                // A small satellite where the halo would sit — the second focal point.
                let ang = rng.f() * std::f32::consts::TAU;
                let d = rng.range(0.78, 0.95) * (r + 70.0);
                shapes.push(Shape {
                    kind: Kind::Circle {
                        cx: CENTER + ang.cos() * d,
                        cy: CENTER + ang.sin() * d,
                        r: rng.range(42.0, 66.0),
                    },
                    fill: p.ink.clone(),
                    opacity: 1.0,
                    core: true,
                });
            }
        }
        // Rotational symmetry: N petals on a circle, optional center dot — mandala-adjacent.
        1 => {
            let n = 3 + rng.pick(4); // 3..=6
            let orbit = rng.range(190.0, 240.0);
            // Fewer petals get proportionally bigger ones, so a sparse ring still fills the
            // composition instead of floating.
            let pr = rng.range(92.0, 128.0) * (4.5 / n as f32).sqrt();
            // Sparse rings of capsules read scattered; below four petals stay with the
            // compact shapes.
            let petal = if n >= 4 { rng.pick(3) } else { rng.pick(2) * 2 };
            let alternate = rng.chance(0.45);
            // Anchored just off "12 o'clock": a recognizably upright arrangement still
            // varies seed-to-seed without ever reading as randomly strewn.
            let phase = -90.0 + rng.range(-16.0, 16.0);
            for i in 0..n {
                let ang = phase + 360.0 * i as f32 / n as f32;
                let rad = ang.to_radians();
                let (cx, cy) = (CENTER + rad.cos() * orbit, CENTER + rad.sin() * orbit);
                let fill = if alternate && i % 2 == 1 { &p.b } else { &p.a };
                let kind = match petal {
                    0 => Kind::Circle { cx, cy, r: pr },
                    1 => Kind::Rect {
                        cx,
                        cy,
                        w: pr * 2.0,
                        h: pr * 1.25,
                        rx: pr * 0.62,
                        rot: ang + 90.0,
                    },
                    _ => Kind::Ring {
                        cx,
                        cy,
                        r: pr * 0.72,
                        width: pr * 0.5,
                        dash: 1.0,
                        rot: 0.0,
                    },
                };
                shapes.push(Shape {
                    kind,
                    fill: fill.clone(),
                    opacity: 1.0,
                    core: true,
                });
            }
            if rng.chance(0.7) {
                shapes.push(Shape {
                    kind: Kind::Circle {
                        cx: CENTER,
                        cy: CENTER,
                        r: rng.range(58.0, 92.0),
                    },
                    fill: p.ink.clone(),
                    opacity: 1.0,
                    core: true,
                });
            }
        }
        // Golden-section asymmetry: dominant motif near a golden point, a clear counterweight
        // pulled in along the diagonal toward the opposite one — balance without symmetry,
        // and the shared axis is what makes the pair read as designed rather than scattered.
        2 => {
            let lo = EDGE * 0.382;
            let hi = EDGE * 0.618;
            let (gx, gy) = match rng.pick(4) {
                0 => (lo, lo),
                1 => (hi, lo),
                2 => (lo, hi),
                _ => (hi, hi),
            };
            // Ease both anchors toward the center: cohesion beats literal golden points.
            let mx = CENTER + (gx - CENTER) * 0.72;
            let my = CENTER + (gy - CENTER) * 0.72;
            let (ox, oy) = (CENTER + (CENTER - gx) * 0.62, CENTER + (CENTER - gy) * 0.62);
            let r = rng.range(205.0, 250.0);
            // Compact, rotation-stable motifs only — a tilted capsule off-center reads as
            // clutter, not asymmetry.
            let kind = match rng.pick(3) {
                0 => Kind::Circle { cx: mx, cy: my, r },
                1 => Kind::Ring {
                    cx: mx,
                    cy: my,
                    r: r * 0.82,
                    width: r * 0.36,
                    dash: 1.0,
                    rot: 0.0,
                },
                _ => Kind::Rect {
                    cx: mx,
                    cy: my,
                    w: r * 1.84,
                    h: r * 1.84,
                    rx: r * 0.55,
                    rot: 0.0,
                },
            };
            shapes.push(Shape {
                kind,
                fill: p.a.clone(),
                opacity: 1.0,
                core: true,
            });
            let cr = rng.range(72.0, 100.0);
            shapes.push(Shape {
                kind: if rng.chance(0.5) {
                    Kind::Circle {
                        cx: ox,
                        cy: oy,
                        r: cr,
                    }
                } else {
                    Kind::Ring {
                        cx: ox,
                        cy: oy,
                        r: cr * 0.85,
                        width: cr * 0.42,
                        dash: 1.0,
                        rot: 0.0,
                    }
                },
                fill: p.b.clone(),
                opacity: 1.0,
                core: true,
            });
            if rng.chance(0.45) {
                // A third beat on the same diagonal — rhythm, and it ties the pair together.
                shapes.push(Shape {
                    kind: Kind::Circle {
                        cx: (mx + ox) / 2.0,
                        cy: (my + oy) / 2.0,
                        r: rng.range(30.0, 44.0),
                    },
                    fill: p.ink.clone(),
                    opacity: 1.0,
                    core: true,
                });
            }
        }
        // Stacked bars: 2–3 descending capsules — abstract "text", mirror-balanced.
        3 => {
            let n = 2 + rng.pick(2);
            let h = rng.range(88.0, 112.0);
            let gap = rng.range(56.0, 76.0);
            let total = n as f32 * h + (n - 1) as f32 * gap;
            let top = CENTER - total / 2.0 + h / 2.0;
            let centered = rng.chance(0.5);
            let left = CENTER - 250.0;
            let widths = [500.0, rng.range(320.0, 400.0), rng.range(180.0, 260.0)];
            for i in 0..n {
                let w = widths[i as usize];
                let cx = if centered { CENTER } else { left + w / 2.0 };
                let fill = match i {
                    0 => &p.a,
                    1 => &p.b,
                    _ => &p.ink,
                };
                shapes.push(Shape {
                    kind: Kind::Rect {
                        cx,
                        cy: top + i as f32 * (h + gap),
                        w,
                        h,
                        rx: h / 2.0,
                        rot: 0.0,
                    },
                    fill: fill.clone(),
                    opacity: 1.0,
                    core: true,
                });
            }
        }
        // Orbit: dominant circle, an open arc around it, a satellite on the arc.
        _ => {
            let r = rng.range(180.0, 230.0);
            let ring = r + rng.range(90.0, 130.0);
            let rot = rng.f() * 360.0;
            shapes.push(motif(rng, CENTER, CENTER, r, &p.a));
            shapes.push(Shape {
                kind: Kind::Ring {
                    cx: CENTER,
                    cy: CENTER,
                    r: ring,
                    width: rng.range(26.0, 38.0),
                    dash: rng.range(0.6, 0.85),
                    rot,
                },
                fill: p.b.clone(),
                opacity: 1.0,
                core: true,
            });
            let rad = rot.to_radians();
            shapes.push(Shape {
                kind: Kind::Circle {
                    cx: CENTER + rad.cos() * ring,
                    cy: CENTER + rad.sin() * ring,
                    r: rng.range(40.0, 58.0),
                },
                fill: p.ink.clone(),
                opacity: 1.0,
                core: true,
            });
        }
    }
    shapes
}

/// Generate the master SVG for `seed`. Deterministic: the same seed always yields the same
/// bytes (the `day new` app-id contract).
pub fn generate(seed: u64) -> String {
    let mut rng = Rng(seed);
    let p = palette(&mut rng);
    let shapes = compose(&mut rng, &p);

    // Background decoration lives in the background LAYER: the adaptive-icon pipeline
    // tightens the foreground to its content box, so full-bleed depth cues (glow, gloss)
    // must not inflate the foreground's extent.
    let glow = rng.chance(0.7);
    let gloss = p.dark && rng.chance(0.35);

    let mut svg = String::with_capacity(4096);
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 1024 1024\">\
         <defs>\
         <linearGradient id=\"day-bg\" x1=\"0\" y1=\"0\" x2=\"0\" y2=\"1\">\
         <stop offset=\"0\" stop-color=\"{}\"/>\
         <stop offset=\"1\" stop-color=\"{}\"/>\
         </linearGradient>\
         <radialGradient id=\"day-glow\">\
         <stop offset=\"0\" stop-color=\"{}\" stop-opacity=\"{}\"/>\
         <stop offset=\"1\" stop-color=\"{}\" stop-opacity=\"0\"/>\
         </radialGradient>\
         </defs>",
        p.bg_top,
        p.bg_bottom,
        p.a,
        if p.dark { "0.30" } else { "0.18" },
        p.a,
    );
    let _ = write!(
        svg,
        "<g id=\"day:background\">\
         <rect width=\"1024\" height=\"1024\" fill=\"url(#day-bg)\"/>"
    );
    if glow {
        let _ = write!(
            svg,
            "<circle cx=\"512\" cy=\"512\" r=\"470\" fill=\"url(#day-glow)\"/>"
        );
    }
    if gloss {
        let _ = write!(
            svg,
            "<ellipse cx=\"512\" cy=\"-120\" rx=\"820\" ry=\"560\" fill=\"#ffffff\" opacity=\"0.06\"/>"
        );
    }
    svg.push_str("</g>");

    svg.push_str("<g id=\"day:foreground\">");
    for s in &shapes {
        svg.push_str(&s.svg(false));
    }
    svg.push_str("</g>");

    // Hidden in the master so plain viewers (and the composite) show the icon as shipped;
    // the pipeline's monochrome-only document re-enables the layer (icon.rs unhide_layer).
    svg.push_str("<g id=\"day:monochrome\" display=\"none\">");
    for s in shapes.iter().filter(|s| s.core) {
        svg.push_str(&s.svg(true));
    }
    svg.push_str("</g>");

    svg.push_str("</svg>");
    svg
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic() {
        assert_eq!(generate(42), generate(42));
        assert_eq!(
            generate(seed_from_str("dev.example.app")),
            generate(seed_from_str("dev.example.app"))
        );
    }

    #[test]
    fn distinct_across_seeds() {
        let mut seen = std::collections::HashSet::new();
        for seed in 0..96u64 {
            assert!(seen.insert(generate(seed)), "seed {seed} collided");
        }
    }

    #[test]
    fn carries_the_master_layer_contract() {
        let svg = generate(7);
        for id in ["day:background", "day:foreground", "day:monochrome"] {
            assert!(svg.contains(id), "missing {id}");
        }
        assert!(!svg.contains("<text"), "text must be outlined");
    }

    #[test]
    fn monochrome_stays_inside_the_vectordrawable_subset() {
        // Android's themed icon ships the monochrome layer as a VectorDrawable only when it
        // fits the subset (docs/icons.md) — generated masters must never fall back to the
        // bitmap mask. Reconstructs the pipeline's monochrome-only doc from the authored
        // layer markers.
        for seed in [0u64, 1, 7, 42, 99, 3_427_929_162_618_665_977] {
            let svg = generate(seed);
            let header_end = svg.find("<g id=\"day:background\"").expect("bg layer");
            let mono_start = svg.find("<g id=\"day:monochrome\"").expect("mono layer");
            let mono = format!("{}{}", &svg[..header_end], &svg[mono_start..])
                .replace(" display=\"none\"", "");
            let tree = crate::parse(mono.as_bytes()).expect("mono parses");
            if let Err(e) = crate::to_vector_drawable(&tree) {
                panic!("seed {seed}: monochrome left the VectorDrawable subset: {e:?}");
            }
        }
    }

    #[test]
    fn every_seed_parses_and_renders_content() {
        for seed in [0u64, 1, 17, 0xdead_beef, u64::MAX] {
            let svg = generate(seed);
            let tree = crate::parse(svg.as_bytes()).expect("parses");
            let png = crate::render_png(&tree, 64).expect("renders");
            // A generated icon is never blank: the opaque backdrop alone guarantees pixels.
            assert!(png.len() > 200, "seed {seed} rendered nearly nothing");
        }
    }
}
