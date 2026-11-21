// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The conic-gradient arithmetic of the shim's canvas painter (docs/canvas.md "Conic
//! gradients"): the platform's 2-D API has no conic brush, so the shim draws a fan of flat
//! wedges, and what each wedge is filled with and where its edges fall is plain arithmetic
//! this can check with no window and no device.

#![cfg(windows)]

use day_toolkit_xaml_sys::day_xaml_conic_probe;

struct Probe {
    color: u32,
    edges: i32,
    steps: i32,
}

fn probe(stops: &[(f64, u32)], reach: f64, turn: f64) -> Probe {
    let offsets: Vec<f64> = stops.iter().map(|s| s.0).collect();
    let colors: Vec<u32> = stops.iter().map(|s| s.1).collect();
    let (mut color, mut edges) = (0u32, 0i32);
    // SAFETY: both arrays hold `stops.len()` entries and the two out-pointers name live locals.
    let steps = unsafe {
        day_xaml_conic_probe(
            offsets.as_ptr(),
            colors.as_ptr(),
            i32::try_from(stops.len()).expect("a handful of stops"),
            reach,
            turn,
            &raw mut color,
            &raw mut edges,
        )
    };
    Probe {
        color,
        edges,
        steps,
    }
}

const RED: u32 = 0xffff_0000;
const BLUE: u32 = 0xff00_00ff;

#[test]
fn color_interpolates_between_stops_channel_by_channel() {
    let stops = [(0.0, RED), (1.0, BLUE)];
    assert_eq!(probe(&stops, 10.0, 0.0).color, RED);
    assert_eq!(probe(&stops, 10.0, 1.0).color, BLUE);
    assert_eq!(probe(&stops, 10.0, 0.5).color, 0xff80_0080);
    // Alpha is a channel like the others.
    let fade = [(0.0, 0x0000_0000), (1.0, 0xff00_ff00)];
    assert_eq!(probe(&fade, 10.0, 0.25).color, 0x4000_4000);
}

#[test]
fn color_holds_flat_outside_the_stops() {
    let stops = [(0.25, RED), (0.75, BLUE)];
    assert_eq!(probe(&stops, 10.0, 0.0).color, RED);
    assert_eq!(probe(&stops, 10.0, 0.2).color, RED);
    assert_eq!(probe(&stops, 10.0, 0.9).color, BLUE);
    assert_eq!(probe(&[], 10.0, 0.5).color, 0);
}

#[test]
fn a_hard_stop_changes_color_at_its_offset() {
    let stops = [(0.0, RED), (0.25, RED), (0.25, BLUE), (1.0, BLUE)];
    assert_eq!(probe(&stops, 10.0, 0.2499).color, RED);
    assert_eq!(probe(&stops, 10.0, 0.2501).color, BLUE);
}

#[test]
fn wedges_are_a_degree_or_finer_and_bounded() {
    // A small fan is never coarser than one wedge per degree…
    assert_eq!(probe(&[], 1.0, 0.0).steps, 360);
    // …a large one gets finer with its circumference, up to a quarter of a degree.
    assert_eq!(probe(&[], 200.0, 0.0).steps, 629);
    assert_eq!(probe(&[], 100_000.0, 0.0).steps, 1440);
}

#[test]
fn every_stop_offset_is_a_wedge_edge() {
    // 360 steps make 361 edges. A stop on the grid (0.25 = 90/360) adds none; one between
    // grid lines adds one; the ends of the turn are already edges.
    let on_grid = [(0.0, RED), (0.25, BLUE), (1.0, RED)];
    assert_eq!(probe(&on_grid, 1.0, 0.0).edges, 361);
    let off_grid = [(0.0, RED), (0.123_45, BLUE), (0.123_45, RED), (1.0, RED)];
    assert_eq!(probe(&off_grid, 1.0, 0.0).edges, 362);
}
