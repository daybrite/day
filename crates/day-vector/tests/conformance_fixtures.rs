// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use day_vector::{parse, render_png, tiny_skia::Pixmap, to_vector_drawable, to_xaml_geometry};

const FIXTURES: [&str; 7] = [
    include_str!("../fixtures/conformance/vector_outline.svg"),
    include_str!("../fixtures/conformance/vector_holes.svg"),
    include_str!("../fixtures/conformance/vector_winding.svg"),
    include_str!("../fixtures/conformance/vector_detail.svg"),
    include_str!("../fixtures/conformance/vector_slot.svg"),
    include_str!("../fixtures/conformance/vector_aspect.svg"),
    include_str!("../fixtures/conformance/vector_alpha.svg"),
];

#[test]
fn conformance_icons_convert_to_native_vector_formats() {
    for svg in FIXTURES {
        let tree = parse(svg.as_bytes()).unwrap();
        assert!(!to_xaml_geometry(&tree).unwrap().shapes.is_empty());
        assert!(to_vector_drawable(&tree).unwrap().contains("<vector"));
    }
    let outline = to_xaml_geometry(&parse(FIXTURES[0].as_bytes()).unwrap()).unwrap();
    assert!(
        outline
            .shapes
            .iter()
            .all(|s| s.fill.is_none() && s.stroke.is_some())
    );
    assert!(outline.shapes.iter().all(|s| s.cap == "Round"));
    assert!(
        to_xaml_geometry(&parse(FIXTURES[1].as_bytes()).unwrap())
            .unwrap()
            .shapes[0]
            .even_odd
    );
    assert!(
        !to_xaml_geometry(&parse(FIXTURES[2].as_bytes()).unwrap())
            .unwrap()
            .shapes[0]
            .even_odd
    );
}

#[test]
fn fine_detail_distinguishes_display_size_rendering_from_the_256px_cache() {
    let tree = parse(FIXTURES[3].as_bytes()).unwrap();
    let display = Pixmap::decode_png(&render_png(&tree, 1024).unwrap()).unwrap();
    // render_png centers the 8:1 artwork in a square output.
    assert_eq!(display.pixel(131, 512).unwrap().alpha(), 255);
    assert_eq!(display.pixel(139, 512).unwrap().alpha(), 0);

    // Negative control: even the darkest pixel in the cached image is too faint to
    // satisfy the native test's 64-channel tolerance after tinting onto black.
    // Upscaling/interpolation cannot recover opacity absent from every source pixel.
    let cached = Pixmap::decode_png(&render_png(&tree, 256).unwrap()).unwrap();
    let darkest = cached.pixels().iter().map(|p| p.alpha()).max().unwrap();
    assert!(
        darkest < 255 - 64,
        "raster fallback could pass: alpha={darkest}"
    );
}
