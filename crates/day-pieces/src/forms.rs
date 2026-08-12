// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Grouped, label-aligned settings UI: `form` and its `section` cards, with `labeled` rows that
//! share one aligned label column across the whole form.

use std::cell::Cell;
use std::rc::Rc;

use day_core::*;
use day_spec::props::*;
use day_spec::{Font, Rect, Size, kinds};

use crate::*;
use day_geometry::Proposal;

// ===========================================================================
// Forms (docs/forms.md): form / section / labeled — grouped, label-aligned settings UI.
// ===========================================================================

/// Shared label-column state for one [`form`]: every [`labeled`] row inside registers its
/// label's width during measurement and lays its label out in a common, form-wide column —
/// the "aligned labels" look every settings UI converges on. The width is per-layout-pass
/// monotonic: all rows measure before any row places (the enclosing stacks measure all
/// children first), so alignment is consistent within a pass without invalidation dances.
#[derive(Clone)]
struct FormLabelColumn(Rc<Cell<f64>>);

const SECTION_RADIUS: f64 = 10.0;
const LABELED_GAP: f64 = 12.0;

/// A settings-style form: a vertical run of [`section`]s whose [`labeled`] rows share one
/// label column across the WHOLE form.
///
/// ```ignore
/// form((
///     section((
///         labeled(tr("volume"), slider(volume)),
///         labeled(tr("enabled"), toggle(enabled)),
///     ))
///     .title(tr("sound")),
///     section((labeled(tr("name"), text_field(name)),)),
/// ))
/// ```
pub fn form<C: PieceSeq + 'static>(sections: C) -> AnyPiece {
    with_environment(FormLabelColumn(Rc::new(Cell::new(0.0))), move || {
        column(sections).spacing(16.0).align(HAlign::Leading).any()
    })
}

/// One grouped form section (created by [`section`]): an optional header above a rounded card
/// whose background is the platform's own theme-adaptive grouped-content material
/// (`SurfaceRole::SectionCard` — quaternary fill on AppKit, libadwaita `.card`, Qt
/// `palette(alternate-base)`, tertiary system fill on iOS, Material surface-container, the
/// XAML card brush), so it follows light/dark mode with no app code.
pub struct FormSection<C: PieceSeq> {
    title: Option<TextSource>,
    children: C,
}

/// A grouped card of form rows; `.title(…)` adds the header. Works inside a [`form`] (shared
/// label column) or standalone.
pub fn section<C: PieceSeq + 'static>(children: C) -> FormSection<C> {
    FormSection {
        title: None,
        children,
    }
}

impl<C: PieceSeq + 'static> FormSection<C> {
    /// The section header, shown above the card in the footnote style.
    pub fn title<M>(mut self, t: impl IntoText<M>) -> Self {
        self.title = Some(t.into_text());
        self
    }
}

impl<C: PieceSeq + 'static> Piece for FormSection<C> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let children = self.children;
        let card = piece_fn(move |cx: &mut BuildCx| {
            let node = cx.native(
                kinds::CONTAINER,
                &ContainerProps {
                    background: None,
                    corner_radius: SECTION_RADIUS,
                    clips: true,
                    role: Some(day_spec::SurfaceRole::SectionCard),
                },
                Rc::new(SectionCardLayout),
                Flex {
                    grow_w: true,
                    ..Default::default()
                },
                Boundary::No,
            );
            let inner = column(children)
                .spacing(10.0)
                .align(HAlign::Leading)
                .padding(14.0);
            cx.under(node, |cx| {
                let _ = AnyPiece::new(inner).build(cx);
            });
            node
        });
        match self.title {
            Some(t) => {
                let header = Label {
                    text: t,
                    font: Font::Footnote,
                    weight: None,
                    italic: false,
                    tabular: false,
                    color: None,
                };
                column((header, card))
                    .spacing(6.0)
                    .align(HAlign::Leading)
                    .build(cx)
            }
            None => card.build(cx),
        }
    }
}

/// The card fills the width its parent proposes (uniform card widths down a form) and hugs
/// its padded content vertically.
struct SectionCardLayout;

impl day_core::Layout for SectionCardLayout {
    fn measure(&self, cx: &mut dyn day_core::LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let cs = children
            .first()
            .map(|&c| cx.measure_child(c, Proposal::new(p.width, None)))
            .unwrap_or(Size::ZERO);
        Size::new(p.width.unwrap_or(cs.width).max(cs.width), cs.height)
    }
    fn place(&self, cx: &mut dyn day_core::LayoutOps, children: &[RNode], bounds: Rect) {
        if let Some(&c) = children.first() {
            let s = cx.measure_child(c, Proposal::new(Some(bounds.size.width), None));
            cx.place_child(c, Rect::new(0.0, 0.0, bounds.size.width, s.height));
        }
    }
}

/// A form row: `label` sits in the form-wide aligned label column (right-aligned, vertically
/// centered), `control` beside it. Outside a [`form`] the label column is just this row's own
/// label width. A control with `.grow()` stretches to the row's remaining width.
pub fn labeled<M, P: Piece>(text: impl IntoText<M>, control: P) -> AnyPiece {
    let text = text.into_text();
    piece_fn(move |cx: &mut BuildCx| {
        // Read the enclosing form's shared column at BUILD time (environment is scoped).
        let col = environment::<FormLabelColumn>();
        let node = cx.layout_only(
            Rc::new(LabeledLayout { col }),
            Flex {
                grow_w: true,
                ..Default::default()
            },
            Boundary::No,
        );
        cx.under(node, |cx| {
            let row_label = Label {
                text,
                font: Font::Body,
                weight: None,
                italic: false,
                tabular: false,
                color: None,
            };
            let _ = row_label.build(cx);
            let _ = AnyPiece::new(control).build(cx);
        });
        node
    })
}

struct LabeledLayout {
    col: Option<FormLabelColumn>,
}

impl LabeledLayout {
    /// The label column width in effect: register OUR label width, read back the max.
    fn column_width(&self, label_w: f64) -> f64 {
        match &self.col {
            Some(c) => {
                if label_w > c.0.get() {
                    c.0.set(label_w);
                }
                c.0.get()
            }
            None => label_w,
        }
    }
}

impl LabeledLayout {
    /// How far to push each of the two children down so their text sits on ONE line
    /// (docs/baseline.md), plus the height the row needs to hold them once pushed.
    ///
    /// `None` when either side has no baseline to offer — a toolkit that does not report them,
    /// or a control with no text at all (a toggle, a slider, an image). The row then keeps the
    /// centering it has always done, which is what makes this safe to have on by default.
    fn baseline_shift(
        &self,
        cx: &mut dyn day_core::LayoutOps,
        lbl: RNode,
        ls: Size,
        ctl: RNode,
        cs: Size,
    ) -> Option<(f64, f64, f64)> {
        let lb = cx.baseline_of(lbl, ls)?;
        let cb = cx.baseline_of(ctl, cs)?;
        let deepest = lb.max(cb);
        // Descent below the shared line decides the rest of the height.
        let height = deepest + (ls.height - lb).max(cs.height - cb);
        Some((deepest - lb, deepest - cb, height))
    }
}

impl day_core::Layout for LabeledLayout {
    fn measure(&self, cx: &mut dyn day_core::LayoutOps, children: &[RNode], p: Proposal) -> Size {
        let (Some(&lbl), Some(&ctl)) = (children.first(), children.get(1)) else {
            return Size::ZERO;
        };
        let ls = cx.measure_child(lbl, Proposal::UNCONSTRAINED);
        let colw = self.column_width(ls.width);
        let avail = p.width.map(|w| (w - colw - LABELED_GAP).max(0.0));
        let cs = cx.measure_child(ctl, Proposal::new(avail, None));
        let natural = colw + LABELED_GAP + cs.width;
        // The row spans the proposed width (labels align form-wide; controls may stretch), and
        // hugs its content vertically — the taller child, or, once the two are sitting on one
        // baseline, whatever the shifted pair needs.
        let boxes = ls.height.max(cs.height);
        let height = match self.baseline_shift(cx, lbl, ls, ctl, cs) {
            Some((_, _, h)) => h.max(boxes),
            None => boxes,
        };
        Size::new(p.width.unwrap_or(natural).max(natural), height)
    }

    /// The row's own baseline is the line its label and control were put on, so a `labeled`
    /// nested inside another baseline-aligned row joins that line too (docs/baseline.md).
    fn baseline(
        &self,
        cx: &mut dyn day_core::LayoutOps,
        children: &[RNode],
        size: Size,
    ) -> Option<f64> {
        let (Some(&lbl), Some(&ctl)) = (children.first(), children.get(1)) else {
            return None;
        };
        let ls = cx.measure_child(lbl, Proposal::UNCONSTRAINED);
        let colw = self.column_width(ls.width);
        let avail = (size.width - colw - LABELED_GAP).max(0.0);
        let cs = cx.measure_child(ctl, Proposal::new(Some(avail), None));
        let lb = cx.baseline_of(lbl, ls)?;
        let (shift, _, _) = self.baseline_shift(cx, lbl, ls, ctl, cs)?;
        Some(shift + lb)
    }
    fn place(&self, cx: &mut dyn day_core::LayoutOps, children: &[RNode], bounds: Rect) {
        let (Some(&lbl), Some(&ctl)) = (children.first(), children.get(1)) else {
            return;
        };
        let ls = cx.measure_child(lbl, Proposal::UNCONSTRAINED);
        let colw = self.column_width(ls.width);
        let avail = (bounds.size.width - colw - LABELED_GAP).max(0.0);
        let cs = cx.measure_child(ctl, Proposal::new(Some(avail), None));
        let h = bounds.size.height;
        // Baseline first, centering as the fallback (docs/baseline.md): a label beside a
        // bordered field or a stepper has its text a few points off from the field's, because
        // the two put their text at different heights inside boxes of different heights.
        // Centering the boxes preserves that offset; this removes it.
        let (lbl_y, ctl_y) = match self.baseline_shift(cx, lbl, ls, ctl, cs) {
            Some((dl, dc, used)) => {
                // Center the aligned PAIR in whatever height the row was actually given, so a
                // row stretched by a taller sibling keeps its text group centered rather than
                // pinned to the top.
                let slack = ((h - used) / 2.0).max(0.0);
                (slack + dl, slack + dc)
            }
            None => (
                ((h - ls.height) / 2.0).max(0.0),
                ((h - cs.height) / 2.0).max(0.0),
            ),
        };
        cx.place_child(
            lbl,
            Rect::new((colw - ls.width).max(0.0), lbl_y, ls.width, ls.height),
        );
        // `.grow()` controls fill the remaining width (text fields, sliders); others hug.
        let cw = if cx.flex_of(ctl).grow_w {
            avail
        } else {
            cs.width.min(avail)
        };
        cx.place_child(ctl, Rect::new(colw + LABELED_GAP, ctl_y, cw, cs.height));
    }
}
