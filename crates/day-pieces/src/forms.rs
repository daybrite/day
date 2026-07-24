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
/// WinUI card brush), so it follows light/dark mode with no app code.
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
        // The row spans the proposed width (labels align form-wide; controls may stretch),
        // and hugs the taller of its two children vertically.
        Size::new(
            p.width.unwrap_or(natural).max(natural),
            ls.height.max(cs.height),
        )
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
        cx.place_child(
            lbl,
            Rect::new(
                (colw - ls.width).max(0.0),
                ((h - ls.height) / 2.0).max(0.0),
                ls.width,
                ls.height,
            ),
        );
        // `.grow()` controls fill the remaining width (text fields, sliders); others hug.
        let cw = if cx.flex_of(ctl).grow_w {
            avail
        } else {
            cs.width.min(avail)
        };
        cx.place_child(
            ctl,
            Rect::new(
                colw + LABELED_GAP,
                ((h - cs.height) / 2.0).max(0.0),
                cw,
                cs.height,
            ),
        );
    }
}
