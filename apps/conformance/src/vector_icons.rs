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

day::tests! {
    vector_tint_preserves_strokes,
    vector_tint_preserves_fill_rules,
    vector_tint_at_multiple_sizes,
    vector_fine_detail_exceeds_raster_cache,
    vector_native_icon_slot_preserves_strokes,
}
