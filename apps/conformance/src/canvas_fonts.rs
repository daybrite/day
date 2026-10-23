// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Bundled fonts must work in the native canvas, its metrics, and its export painter.
//! The fixture's filename contains spaces and a non-ASCII character to exercise Win2D file URIs.

use day::prelude::*;
use day::{Case, Drive};

fn specimen(d: &mut Draw, size: Size) {
    d.fill(Shape::Rect(Rect::from_size(size)), Color::hex(0xffffff));
    d.text(
        "Pacifico",
        Point::new(8.0, 8.0),
        TextStyle {
            size: 32.0,
            color: Color::hex(0x000000),
            anchor: TextAnchor::LEADING,
            font: CanvasFont {
                family: Some("Pacifico".into()),
                ..Default::default()
            },
        },
    );
}

#[day::test]
fn canvas_bundled_font() -> Case {
    Case::new()
        .requires(Cap::CanvasBitmap)
        .requires(Cap::Snapshot)
        .page(|| {
            let custom = day::measure_text(
                "Pacifico",
                32.0,
                &CanvasFont {
                    family: Some("Pacifico".into()),
                    ..Default::default()
                },
            );
            let fallback = day::measure_text("Pacifico", 32.0, &CanvasFont::default());
            // A silent system-font substitution is also a failure, even if drawing survives.
            let distinct = (custom.width - fallback.width).abs() > 1.0
                || (custom.height - fallback.height).abs() > 1.0;
            let size = Size::new(200.0, 100.0);
            let exported = Signal::new(None::<day::Bitmap>);
            let state = Signal::new("rendering".to_string());
            day::task(async move {
                match Drawing::record(size, specimen).to_bitmap(1.0).await {
                    Ok(bitmap) => {
                        exported.set(Some(bitmap));
                        state.set("rendered".into());
                    }
                    Err(error) => state.set(format!("{error:?}")),
                }
            });
            column((
                canvas(specimen)
                    .frame(size.width, size.height)
                    .id(day::id!(canvas_font_test::live)),
                keyed(
                    move || exported.get(),
                    move |bitmap| {
                        image(bitmap)
                            .stretch()
                            .frame(size.width, size.height)
                            .id(day::id!(canvas_font_test::exported))
                    },
                ),
                label(if distinct {
                    "custom metrics"
                } else {
                    "fallback metrics"
                })
                .id(day::id!(canvas_font_test::metrics)),
                label(move || state.get()).id(day::id!(canvas_font_test::state)),
            ))
        })
        .drive(|d: Drive| async move {
            d.assert_text("canvas_font_test::metrics", "custom metrics")
                .await?;
            d.assert_text("canvas_font_test::state", "rendered").await?;
            d.assert_visible("canvas_font_test::live").await?;
            d.assert_visible("canvas_font_test::exported").await?;
            d.shot("bundled-font").await
        })
}
