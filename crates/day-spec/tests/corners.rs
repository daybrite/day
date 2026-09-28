// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Rounded corners (docs/canvas.md "Rounded corners"): fitting radii to a rectangle, choosing the
//! simplest shape, and the contour a mixed set of corners draws.

use day_spec::{CornerRadii, PathSeg, Point, Rect, Shape, Size, arc_cubics, rounded_rect_segs};

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn near_pt(a: Point, b: Point) -> bool {
    near(a.x, b.x) && near(a.y, b.y)
}

/// Every on-curve point of a contour (move, line and curve end points).
fn on_curve(segs: &[PathSeg]) -> Vec<Point> {
    segs.iter()
        .filter_map(|s| match s {
            PathSeg::Move(p) | PathSeg::Line(p) | PathSeg::Quad(_, p) | PathSeg::Cubic(_, _, p) => {
                Some(*p)
            }
            PathSeg::Close => None,
        })
        .collect()
}

#[test]
fn the_edge_constructors_round_only_their_edge() {
    assert_eq!(CornerRadii::top(4.0).to_array(), [4.0, 4.0, 0.0, 0.0]);
    assert_eq!(CornerRadii::bottom(4.0).to_array(), [0.0, 0.0, 4.0, 4.0]);
    assert_eq!(CornerRadii::left(4.0).to_array(), [4.0, 0.0, 0.0, 4.0]);
    assert_eq!(CornerRadii::right(4.0).to_array(), [0.0, 4.0, 4.0, 0.0]);
    assert_eq!(CornerRadii::uniform(4.0).to_array(), [4.0; 4]);
    assert!(CornerRadii::default().is_zero());
    assert!(CornerRadii::uniform(2.0).is_uniform());
    assert!(!CornerRadii::top(2.0).is_uniform());
}

#[test]
fn radii_that_fit_are_left_alone() {
    let r = CornerRadii::top(4.0).fitted(Size::new(20.0, 50.0));
    assert_eq!(r, CornerRadii::top(4.0));
}

#[test]
fn overlapping_radii_shrink_together_the_way_css_does() {
    // Along the 10-point top edge the two 10-point radii need 20: everything halves, the
    // bottom corners too, so the shape keeps its proportions.
    let r = CornerRadii {
        top_left: 10.0,
        top_right: 10.0,
        bottom_right: 4.0,
        bottom_left: 2.0,
    }
    .fitted(Size::new(10.0, 100.0));
    assert_eq!(r.to_array(), [5.0, 5.0, 2.0, 1.0]);
    // A top-rounded bar 8 wide and 3 tall: each side's radius fits its 3-point height, and the
    // two across the top fit its 8-point width, so both become 3.
    let r = CornerRadii::top(10.0).fitted(Size::new(8.0, 3.0));
    assert!(near(r.top_left, 3.0) && near(r.top_right, 3.0), "{r:?}");
    assert_eq!((r.bottom_left, r.bottom_right), (0.0, 0.0));
}

#[test]
fn negative_and_non_finite_radii_are_square() {
    let r = CornerRadii {
        top_left: -3.0,
        top_right: f64::NAN,
        bottom_right: f64::INFINITY,
        bottom_left: 2.0,
    }
    .fitted(Size::new(10.0, 10.0));
    assert_eq!(r.to_array(), [0.0, 0.0, 0.0, 2.0]);
    assert!(
        CornerRadii::uniform(5.0)
            .fitted(Size::new(0.0, 10.0))
            .is_zero()
    );
}

#[test]
fn the_simplest_shape_is_chosen() {
    let rect = Rect::new(0.0, 0.0, 20.0, 40.0);
    assert_eq!(
        Shape::rounded_rect(rect, CornerRadii::default()),
        Shape::Rect(rect)
    );
    assert_eq!(
        Shape::rounded_rect(rect, CornerRadii::uniform(6.0)),
        Shape::RoundedRect(rect, 6.0)
    );
    // A uniform radius too big for the rectangle becomes the fitted pill, still native.
    assert_eq!(
        Shape::rounded_rect(rect, CornerRadii::uniform(100.0)),
        Shape::RoundedRect(rect, 10.0)
    );
    assert!(matches!(
        Shape::rounded_rect(rect, CornerRadii::top(6.0)),
        Shape::Path(_)
    ));
    // An empty rectangle has nothing to round.
    let empty = Rect::new(0.0, 0.0, 0.0, 40.0);
    assert_eq!(
        Shape::rounded_rect(empty, CornerRadii::top(6.0)),
        Shape::Rect(empty)
    );
}

#[test]
fn a_top_rounded_bar_is_square_at_its_base_and_curved_at_its_top() {
    let rect = Rect::new(10.0, 20.0, 30.0, 60.0);
    let segs = rounded_rect_segs(rect, CornerRadii::top(5.0));
    assert!(matches!(segs.first(), Some(PathSeg::Move(_))));
    assert!(matches!(segs.last(), Some(PathSeg::Close)));
    let pts = on_curve(&segs);
    // The bottom corners are the rectangle's own corners, exactly.
    assert!(pts.iter().any(|p| near_pt(*p, Point::new(40.0, 80.0))));
    assert!(pts.iter().any(|p| near_pt(*p, Point::new(10.0, 80.0))));
    // The top corners are not: the curves end where the edges begin.
    assert!(!pts.iter().any(|p| near_pt(*p, Point::new(10.0, 20.0))));
    assert!(!pts.iter().any(|p| near_pt(*p, Point::new(40.0, 20.0))));
    for p in [
        Point::new(10.0, 25.0),
        Point::new(15.0, 20.0),
        Point::new(35.0, 20.0),
        Point::new(40.0, 25.0),
    ] {
        assert!(pts.iter().any(|q| near_pt(*q, p)), "curve passes {p:?}");
    }
    // Two quarter-turn curves, one per rounded corner.
    let curves = segs
        .iter()
        .filter(|s| matches!(s, PathSeg::Cubic(..)))
        .count();
    assert_eq!(curves, 2);
    // And it stays inside its rectangle, control points included.
    let bounds = day_spec::Path {
        segs: segs.clone(),
        rule: Default::default(),
    }
    .bounds();
    assert!(near(bounds.origin.x, 10.0) && near(bounds.origin.y, 20.0));
    assert!(near(bounds.size.width, 30.0) && near(bounds.size.height, 60.0));
}

#[test]
fn a_right_rounded_bar_is_square_on_the_left() {
    let rect = Rect::new(0.0, 0.0, 50.0, 10.0);
    let pts = on_curve(&rounded_rect_segs(rect, CornerRadii::right(4.0)));
    assert!(pts.iter().any(|p| near_pt(*p, Point::new(0.0, 0.0))));
    assert!(pts.iter().any(|p| near_pt(*p, Point::new(0.0, 10.0))));
    assert!(!pts.iter().any(|p| near_pt(*p, Point::new(50.0, 0.0))));
    assert!(!pts.iter().any(|p| near_pt(*p, Point::new(50.0, 10.0))));
}

#[test]
fn a_quarter_arc_is_one_cubic_through_its_ends_and_nothing_degenerate_draws() {
    let segs = arc_cubics(Point::new(0.0, 0.0), 10.0, 0.0, 90.0);
    assert_eq!(segs.len(), 1);
    let PathSeg::Cubic(_, _, end) = segs[0] else {
        panic!("a cubic")
    };
    // Clockwise in device space: from +x down to +y.
    assert!(near_pt(end, Point::new(0.0, 10.0)));
    assert_eq!(arc_cubics(Point::new(0.0, 0.0), 10.0, 0.0, 270.0).len(), 3);
    assert!(arc_cubics(Point::new(0.0, 0.0), 0.0, 0.0, 90.0).is_empty());
    assert!(arc_cubics(Point::new(0.0, 0.0), 10.0, 0.0, 0.0).is_empty());
    assert!(arc_cubics(Point::new(0.0, 0.0), f64::NAN, 0.0, 90.0).is_empty());
}
