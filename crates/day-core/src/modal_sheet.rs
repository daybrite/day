// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Shared card composition for sheet covers and modal window fallbacks.
use crate::{AnyPiece, Boundary, BuildCx, Flex, Layout, LayoutOps, Piece, RNode};
use day_spec::{Proposal, Rect, Size, kinds};
use std::rc::Rc;

/// The dimming laid over the page under a composed sheet.
const SHEET_SCRIM: day_spec::Color = day_spec::Color::rgba(0.0, 0.0, 0.0, 0.32);
/// A composed sheet card's corner radius.
const SHEET_RADIUS: f64 = 12.0;

/// A composed modal sheet (docs/cover.md "Modal sheet"): the cover's content on a card in the
/// window's own background color, centered over a dimmed fill that spans the cover. The cover
/// itself is clear and takes every click, so the page shows through but stays out of reach.
pub struct ComposedSheet {
    pub content: AnyPiece,
    /// The large card (docs/cover.md "Large sheet"), for a document or web page.
    pub large: bool,
}

impl Piece for ComposedSheet {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let fill = Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        };
        let node = cx.native(
            kinds::CONTAINER,
            &day_spec::props::ContainerProps::default(),
            Rc::new(SheetLayout { large: self.large }),
            fill,
            Boundary::Yes,
        );
        cx.under(node, |cx| {
            cx.native(
                kinds::CONTAINER,
                &day_spec::props::ContainerProps {
                    background: Some(SHEET_SCRIM),
                    role: Some(day_spec::SurfaceRole::Scrim),
                    ..Default::default()
                },
                Rc::new(FillLayout),
                fill,
                Boundary::No,
            );
            let card = cx.native(
                kinds::CONTAINER,
                &day_spec::props::ContainerProps {
                    background: None,
                    corner_radius: SHEET_RADIUS,
                    clips: true,
                    role: Some(day_spec::SurfaceRole::Window),
                },
                Rc::new(FillLayout),
                fill,
                Boundary::Yes,
            );
            cx.under(card, |cx| {
                let _ = self.content.build(cx);
            });
        });
        node
    }
}

/// A composed sheet card's size in a cover `area` large: the native AppKit sheet's proportions
/// (the window less a margin, at most [`day_spec::props::SHEET_MAX`], or [`day_spec::props::LARGE_SHEET_MAX`]
/// for a large sheet), never larger than the area itself.
fn sheet_card_size(area: Size, large: bool) -> Size {
    let max = if large {
        day_spec::props::LARGE_SHEET_MAX
    } else {
        day_spec::props::SHEET_MAX
    };
    Size::new(
        (area.width - 40.0).clamp(320.0, max.width).min(area.width),
        (area.height - 80.0)
            .clamp(300.0, max.height)
            .min(area.height),
    )
}

/// [`ComposedSheet`]'s layout: the dimmed fill over the whole cover, the card centered on it.
struct SheetLayout {
    large: bool,
}

impl Layout for SheetLayout {
    fn measure(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        let area = bounds.size;
        if let Some(&scrim) = children.first() {
            cx.place_child(scrim, Rect::from_size(area));
        }
        if let Some(&card) = children.get(1) {
            let size = sheet_card_size(area, self.large);
            cx.place_child(
                card,
                Rect::new(
                    ((area.width - size.width) / 2.0).floor(),
                    ((area.height - size.height) / 2.0).floor(),
                    size.width,
                    size.height,
                ),
            );
        }
    }
}

/// One child filling the container: the dimmed fill (no children) and the card's content.
struct FillLayout;

impl Layout for FillLayout {
    fn measure(&self, _cx: &mut dyn LayoutOps, _children: &[RNode], p: Proposal) -> Size {
        Size::new(p.width.unwrap_or(0.0), p.height.unwrap_or(0.0))
    }
    fn place(&self, cx: &mut dyn LayoutOps, children: &[RNode], bounds: Rect) {
        for &child in children {
            cx.place_child(child, Rect::from_size(bounds.size));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn large_card_grows_within_the_window() {
        let area = Size::new(1280.0, 800.0);
        assert_eq!(sheet_card_size(area, false), Size::new(640.0, 620.0));
        assert_eq!(sheet_card_size(area, true), Size::new(1040.0, 720.0));
        // A small window leaves both at the window less its margin.
        let small = Size::new(500.0, 400.0);
        assert_eq!(sheet_card_size(small, true), sheet_card_size(small, false));
    }
}
