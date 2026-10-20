// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Real staged SVG assets, through the same named-vector pipeline as application icons.
//! Pixel assertions prove paint/geometry; the fine-detail case also rejects the current
//! 256px fallback cache. These are app cases because their resources must be bundled.

use crate::res;
use day::prelude::*;
use day::{Case, Drive};

#[day::test]
fn vector_tint_preserves_strokes() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            vector(res::vectors::vector_outline)
                .tint(Color::hex(0xff0000))
                .id("outline")
                .frame(128.0, 128.0)
                .background(Color::hex(0x000000))
        })
        .drive(|d: Drive| async move {
            d.sample_pixel("outline", 12.0 / 64.0, 0.5, "#ff0000")
                .await?;
            d.sample_pixel("outline", 0.5, 0.5, "#ff0000").await?;
            // Rounded cap extends beyond the open line's start at x=20.
            d.sample_pixel("outline", 18.0 / 64.0, 0.5, "#ff0000")
                .await?;
            d.sample_pixel("outline", 24.0 / 64.0, 24.0 / 64.0, "#000000")
                .await?;
            d.sample_pixel("outline", 4.0 / 64.0, 4.0 / 64.0, "#000000")
                .await?;
            d.shot("tinted-outline").await
        })
}

#[day::test]
fn vector_tint_preserves_fill_rules() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            row((
                vector(res::vectors::vector_holes)
                    .tint(Color::hex(0x0000ff))
                    .id("evenodd")
                    .frame(96.0, 96.0),
                vector(res::vectors::vector_winding)
                    .tint(Color::hex(0xff0000))
                    .id("nonzero")
                    .frame(96.0, 96.0),
            ))
            .background(Color::hex(0x000000))
        })
        .drive(|d: Drive| async move {
            d.sample_pixel("evenodd", 0.25, 0.5, "#0000ff").await?;
            d.sample_pixel("evenodd", 0.5, 0.5, "#000000").await?;
            d.sample_pixel("nonzero", 0.25, 0.5, "#ff0000").await?;
            d.sample_pixel("nonzero", 0.5, 0.5, "#ff0000").await?;
            d.shot("fill-rules").await
        })
}

#[day::test]
fn vector_tint_at_multiple_sizes() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            row((
                vector(res::vectors::vector_outline)
                    .tint(Color::hex(0xff0000))
                    .id("small")
                    .frame(32.0, 32.0),
                vector(res::vectors::vector_outline)
                    .tint(Color::hex(0x808080))
                    .id("medium")
                    .frame(64.0, 64.0),
                vector(res::vectors::vector_outline)
                    .tint(Color::hex(0xff0000))
                    .id("large")
                    .frame(128.0, 128.0),
            ))
            .background(Color::hex(0x000000))
        })
        .drive(|d: Drive| async move {
            for (id, size, color) in [
                ("small", 32.0, "#ff0000"),
                ("medium", 64.0, "#808080"),
                ("large", 128.0, "#ff0000"),
            ] {
                d.assert_size(id, size, size).await?;
                d.sample_pixel(id, 12.0 / 64.0, 0.5, color).await?;
                d.sample_pixel(id, 24.0 / 64.0, 24.0 / 64.0, "#000000")
                    .await?;
            }
            Ok(())
        })
}

#[day::test]
fn vector_fine_detail_exceeds_raster_cache() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            // Only the first 200 points are needed, even on a small phone. A content
            // container keeps the image's native bounds distinct from the scroll viewport.
            scroll(row((vector(res::vectors::vector_detail)
                .tint(Color::hex(0xff0000))
                .id("detail")
                .frame(1024.0, 128.0),)))
            .horizontal()
            // Leave room for toolkits whose scrollbar consumes layout space.
            .frame(200.0, 160.0)
            .background(Color::hex(0x000000))
        })
        .drive(|d: Drive| async move {
            d.assert_size("detail", 1024.0, 128.0).await?;
            d.sample_pixel("detail", 131.0 / 1024.0, 0.5, "#ff0000")
                .await?;
            d.sample_pixel("detail", 139.0 / 1024.0, 0.5, "#000000")
                .await?;
            d.shot("fine-detail").await
        })
}

#[day::test]
fn vector_native_icon_slot_preserves_strokes() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            // Control icon slots can use a different renderer from vector(): on WinUI
            // this goes through PathIcon, which must widen strokes before filling them.
            // A black button supplies a deterministic background and white foreground.
            button("Outlined icon")
                .image(res::vectors::vector_slot)
                .icon_only()
                .tint(Color::hex(0x000000))
                .id("icon-button")
                .frame(96.0, 64.0)
        })
        .drive(|d: Drive| async move {
            d.assert_size("icon-button", 96.0, 64.0).await?;
            d.sample_pixel("icon-button", 0.5, 0.5, "#000000").await?;
            // The thick stroke covers this offset at native 16–32 point icon sizes.
            // Checking paint as well as the hole also rejects a missing icon.
            d.sample_pixel("icon-button", 53.0 / 96.0, 0.5, "#ffffff")
                .await?;
            d.shot("native-icon-slot").await
        })
}

/// Grow the existing native view after it has painted at a smaller size.
#[day::test]
fn vector_resize_retains_fine_detail() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            let compact = Signal::new(true);
            column((
                scroll(
                    row((
                        vector(res::vectors::vector_detail)
                            .tint(Color::hex(0xff0000))
                            .height(128.0)
                            .grow_w()
                            // The sized box, not the vector's own node.
                            .wrapper_id("detail"),
                        when(move || compact.get(), || spacer().width(768.0)),
                    ))
                    .spacing(0.0)
                    .frame(1024.0, 128.0),
                )
                .horizontal()
                .frame(200.0, 160.0)
                .background(Color::hex(0x000000)),
                button("Resize")
                    .action(move || compact.set(!compact.get()))
                    .id("resize"),
            ))
        })
        .drive(|d: Drive| async move {
            d.assert_size("detail", 256.0, 128.0).await?;
            d.shot("small-before-resize").await?;
            for _ in 0..2 {
                d.tap("resize").await?;
                d.assert_size("detail", 1024.0, 128.0).await?;
                d.sample_pixel("detail", 131.0 / 1024.0, 0.5, "#ff0000")
                    .await?;
                d.sample_pixel("detail", 139.0 / 1024.0, 0.5, "#000000")
                    .await?;
                d.tap("resize").await?;
                d.assert_size("detail", 256.0, 128.0).await?;
            }
            Ok(())
        })
}

#[day::test]
fn vector_reactive_tint_preserves_alpha() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            let blue = Signal::new(false);
            column((
                button("Recolor")
                    .action(move || blue.set(!blue.get()))
                    .id("recolor"),
                vector(res::vectors::vector_alpha)
                    .tint(move || {
                        if blue.get() {
                            Color::hex(0x0000ff)
                        } else {
                            Color::rgba(1.0, 0.0, 0.0, 0.25)
                        }
                    })
                    .frame(96.0, 96.0)
                    .background(Color::hex(0x000000))
                    // The black box around the vector is what the corner sample reads.
                    .wrapper_id("alpha"),
                vector(res::vectors::vector_outline)
                    .tint(move || {
                        if blue.get() {
                            Color::hex(0x0000ff)
                        } else {
                            Color::rgba(1.0, 0.0, 0.0, 0.25)
                        }
                    })
                    .frame(96.0, 96.0)
                    .background(Color::hex(0x000000))
                    .wrapper_id("stroke"),
            ))
        })
        .drive(|d: Drive| async move {
            d.sample_pixel("alpha", 0.5, 0.5, "#200000").await?;
            d.sample_pixel("stroke", 12.0 / 64.0, 0.5, "#400000")
                .await?;
            d.sample_pixel("alpha", 0.05, 0.05, "#000000").await?;
            d.tap("recolor").await?;
            d.sample_pixel("alpha", 0.5, 0.5, "#000080").await?;
            d.sample_pixel("stroke", 12.0 / 64.0, 0.5, "#0000ff")
                .await?;
            d.sample_pixel("stroke", 24.0 / 64.0, 24.0 / 64.0, "#000000")
                .await?;
            d.tap("recolor").await?;
            d.sample_pixel("alpha", 0.5, 0.5, "#200000").await?;
            d.sample_pixel("stroke", 12.0 / 64.0, 0.5, "#400000").await
        })
}

#[day::test]
fn vector_non_square_content_modes() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            row((
                image(res::vectors::vector_aspect)
                    .fit()
                    .id("fit")
                    .frame(80.0, 80.0),
                image(res::vectors::vector_aspect)
                    .fill()
                    .id("fill")
                    .frame(40.0, 80.0),
                image(res::vectors::vector_aspect)
                    .stretch()
                    .id("stretch")
                    .frame(80.0, 80.0),
            ))
            .background(Color::hex(0x000000))
        })
        .drive(|d: Drive| async move {
            d.sample_pixel("fit", 0.25, 0.1, "#000000").await?;
            d.sample_pixel("fit", 0.25, 0.5, "#ff0000").await?;
            d.sample_pixel("fit", 0.75, 0.5, "#0000ff").await?;
            for id in ["fill", "stretch"] {
                d.sample_pixel(id, 0.25, 0.1, "#ff0000").await?;
                d.sample_pixel(id, 0.75, 0.9, "#0000ff").await?;
            }
            Ok(())
        })
}

/// A source patch must replace SVG state with bitmap state, and back, on one native view.
#[day::test]
fn vector_source_swaps_with_bitmap() -> Case {
    // A lossless 1x1 blue PNG; kept inline so this case exercises the Bytes source arm.
    const BLUE: &[u8] = &[
        137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 73, 72, 68, 82, 0, 0, 0, 1, 0, 0, 0, 1, 8, 2,
        0, 0, 0, 144, 119, 83, 222, 0, 0, 0, 12, 73, 68, 65, 84, 120, 156, 99, 96, 96, 248, 15, 0,
        1, 3, 1, 0, 8, 137, 194, 236, 0, 0, 0, 0, 73, 69, 78, 68, 174, 66, 96, 130,
    ];
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            let bitmap = Signal::new(false);
            column((
                button("Swap")
                    .action(move || bitmap.set(!bitmap.get()))
                    .id("swap"),
                image(move || {
                    if bitmap.get() {
                        day::ImageSource::Bytes(BLUE.to_vec().into())
                    } else {
                        day::ImageSource::Named(res::vectors::vector_aspect.as_str().to_string())
                    }
                })
                .frame(80.0, 80.0)
                .background(Color::hex(0x000000))
                // The black box around the image is what the top sample reads.
                .wrapper_id("source"),
            ))
        })
        .drive(|d: Drive| async move {
            // Reuse the same viewport across repeated swaps, checking the full bitmap frame
            // as well as the vector's restored letterboxing and both authored colors.
            for _ in 0..2 {
                d.sample_pixel("source", 0.25, 0.5, "#ff0000").await?;
                d.sample_pixel("source", 0.75, 0.5, "#0000ff").await?;
                d.sample_pixel("source", 0.25, 0.1, "#000000").await?;
                d.tap("swap").await?;
                d.sample_pixel("source", 0.25, 0.5, "#0000ff").await?;
                d.sample_pixel("source", 0.25, 0.1, "#0000ff").await?;
                d.tap("swap").await?;
            }
            d.sample_pixel("source", 0.25, 0.5, "#ff0000").await
        })
}

#[day::test]
fn vector_native_icon_follows_button_tint() -> Case {
    Case::new()
        .requires(Cap::Snapshot)
        .page(|| {
            let light = Signal::new(false);
            column((
                button("Change tint")
                    .action(move || light.set(!light.get()))
                    .id("change"),
                button("Icon")
                    .image(res::vectors::vector_slot)
                    .icon_only()
                    .tint(move || Color::hex(if light.get() { 0xffffff } else { 0x000000 }))
                    .id("button")
                    .frame(96.0, 64.0),
            ))
        })
        .drive(|d: Drive| async move {
            d.sample_pixel("button", 53.0 / 96.0, 0.5, "#ffffff")
                .await?;
            d.sample_pixel("button", 0.5, 0.5, "#000000").await?;
            d.tap("change").await?;
            d.sample_pixel("button", 53.0 / 96.0, 0.5, "#000000")
                .await?;
            d.sample_pixel("button", 0.5, 0.5, "#ffffff").await
        })
}

day::tests! {
    vector_resize_retains_fine_detail,
    vector_reactive_tint_preserves_alpha,
    vector_non_square_content_modes,
    vector_source_swaps_with_bitmap,
    vector_native_icon_follows_button_tint,
    vector_tint_preserves_strokes,
    vector_tint_preserves_fill_rules,
    vector_tint_at_multiple_sizes,
    vector_fine_detail_exceeds_raster_cache,
    vector_native_icon_slot_preserves_strokes,
}
