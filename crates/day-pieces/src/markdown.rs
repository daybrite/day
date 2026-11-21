// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The Markdown view (docs/markdown-view.md): a read-only document that takes its height from
//! its content.
//!
//! Composed here, once, from native labels and Day's containers, so it is the same piece on
//! every toolkit and needs no toolkit duty: a paragraph or heading is a `label(…).markdown()`,
//! a code block is a monospace label on the platform's card surface, a quote bar and a rule
//! are dividers, and a list item is a marker label beside its content. The document is a
//! column of such rows, one per [`MarkdownBlock`].
//!
//! The rows are kept, not rebuilt. When the text changes, [`MarkdownDocument::update`] parses
//! from the last top-level block boundary before the change and says which block is the first
//! to differ; rows below it are untouched, a row whose block kept its shape gets its new text
//! through a signal (one label patch), and only rows whose shape changed are built again. A
//! reply that streams in a token at a time therefore costs one block parse and one label patch
//! per token, whatever the length of the text above it.

use std::cell::RefCell;
use std::rc::Rc;

use day_core::*;
use day_reactive::{Scope, Signal, watch};
use day_spec::markdown::{MarkdownBlock, MarkdownBlockKind, MarkdownDocument};
use day_spec::{Font, Insets, ListStyle, Role};

use crate::*;

/// The width a list marker takes before its item's text, and the gap after it.
const MARKER_WIDTH: f64 = 22.0;
/// How far each list level sits in from the one above: the marker and the gap after it.
const LIST_INDENT: f64 = MARKER_WIDTH + ROW_GAP;
/// The gap between a quote bar, a marker and the text.
const ROW_GAP: f64 = 8.0;

/// A read-only Markdown document that sizes to its content (docs/markdown-view.md).
///
/// Built by [`markdown()`]. It does not scroll and cannot be edited: put it in a chat bubble, a
/// release note, a help page, or inside a `scroll` when the text can be long.
#[must_use]
pub struct Markdown {
    text: TextSource,
    font: Font,
    color: Option<Reactive<day_spec::Ink>>,
    spacing: f64,
    selectable: bool,
    on_link: Option<crate::leaves::LinkHandler>,
    code_accessory: Option<CodeAccessory>,
    block_id: Option<Rc<dyn Fn(usize) -> ElementId>>,
}

/// Builds the piece shown in a code block's header.
type CodeAccessory = Rc<dyn Fn(MarkdownCode) -> AnyPiece>;

/// Show block Markdown: headings, paragraphs with inline styles and links, bullet and numbered
/// lists, block quotes, fenced code blocks and rules.
///
/// ```ignore
/// markdown(res::str::release_notes())
/// markdown(move || reply.get())          // a streamed reply: only the last block re-parses
///     .code_accessory(|code| {
///         button(res::str::copy())
///             .icon(Symbol::Copy)
///             .icon_only()
///             .plain()
///             .action(move || copy_text(code.text()))
///     })
/// ```
///
/// The text is a literal, a localized message, a `Signal<String>` or a closure. The view takes
/// the width it is given and the height its content needs. Links open through [`open_link`]
/// unless [`Markdown::on_link`] handles them.
pub fn markdown<M>(text: impl IntoText<M>) -> Markdown {
    Markdown {
        text: text.into_text(),
        font: Font::Body,
        color: None,
        spacing: Space::M.points(),
        selectable: false,
        on_link: None,
        code_accessory: None,
        block_id: None,
    }
}

/// A fenced code block in a [`Markdown`] view, as its accessory sees it. `Copy`, and its reads
/// are tracked, so a closure or an action that calls them always gets the current text, also
/// while the block is still streaming in.
#[derive(Clone, Copy)]
pub struct MarkdownCode {
    block: Signal<MarkdownBlock>,
}

impl MarkdownCode {
    /// The code, verbatim, without the fences.
    #[must_use]
    pub fn text(&self) -> String {
        self.block.with(|b| b.text.clone())
    }
    /// The language named after the opening fence; empty when there is none.
    #[must_use]
    pub fn language(&self) -> String {
        self.block.with(|b| match &b.kind {
            MarkdownBlockKind::Code { language } => language.clone(),
            _ => String::new(),
        })
    }
}

#[day_macros::builder(MarkdownBuilder)]
impl Markdown {
    /// The font of paragraphs, list items, quotes and code (default [`Font::Body`]). Headings
    /// take the semantic title styles whatever this is.
    pub fn font(mut self, font: Font) -> Self {
        self.font = font;
        self
    }
    /// The text color of every block: a constant, a signal or a closure. Without it each block
    /// draws in the platform's label color.
    pub fn color<M>(mut self, color: impl IntoReactive<day_spec::Ink, M>) -> Self {
        self.color = Some(color.into_reactive());
        self
    }
    /// The gap between blocks: a [`Space`] step or points (default `Space::M`). Items of a
    /// tight list sit closer, and a first- or second-level heading takes half again as much.
    pub fn spacing(mut self, spacing: impl Into<f64>) -> Self {
        self.spacing = spacing.into();
        self
    }
    /// Let the reader select and copy the text of each block. Selection stays inside one
    /// block: every block is its own native label.
    pub fn selectable_text(mut self) -> Self {
        self.selectable = true;
        self
    }
    /// Handle a tapped link yourself. Without this a `#route` target navigates in-app and any
    /// other target opens in the platform's default handler ([`open_link`]).
    pub fn on_link(mut self, f: impl Fn(&str) + 'static) -> Self {
        self.on_link = Some(Rc::new(f));
        self
    }
    /// Put a piece in the header of every code block, at its trailing edge: a copy button, a
    /// "Run" button, a language badge of your own. The closure runs once per code block and
    /// gets a [`MarkdownCode`] whose `text()` and `language()` read the block as it is now.
    ///
    /// Copying is the app's to do, because the clipboard is a part the app chooses to link
    /// (`day-part-clipboard`).
    pub fn code_accessory<P: Piece>(mut self, f: impl Fn(MarkdownCode) -> P + 'static) -> Self {
        self.code_accessory = Some(Rc::new(move |code| AnyPiece::new(f(code))));
        self
    }
    /// Give each block's text an element id for tests and `DayScript`, from its position in the
    /// document: `.block_ids(|i| id!(reply_block, i))`. A rule has no text and takes no id.
    pub fn block_ids(mut self, id: impl Fn(usize) -> ElementId + 'static) -> Self {
        self.block_id = Some(Rc::new(id));
        self
    }
}

/// What decides how a block's row is built. A row whose block keeps its shape is updated
/// through its signal; one whose shape changes is built again.
#[derive(Clone, Debug, PartialEq)]
struct RowShape {
    kind: ShapeKind,
    quote_depth: u8,
    list_depth: u8,
    marker: ListStyle,
    /// The space above the row.
    gap: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShapeKind {
    Paragraph,
    Heading(u8),
    /// A code block, and whether it shows a header (a language or an accessory).
    Code {
        header: bool,
    },
    Rule,
}

/// One built row and the signal its labels read.
struct BlockRow {
    shape: RowShape,
    block: Signal<MarkdownBlock>,
    scope: Scope,
    root: RNode,
}

/// The settings every row is built from.
struct RowStyle {
    font: Font,
    color: Option<Reactive<day_spec::Ink>>,
    spacing: f64,
    selectable: bool,
    on_link: Option<crate::leaves::LinkHandler>,
    code_accessory: Option<CodeAccessory>,
    block_id: Option<Rc<dyn Fn(usize) -> ElementId>>,
}

impl RowStyle {
    fn shape(&self, block: &MarkdownBlock, previous: Option<&MarkdownBlock>) -> RowShape {
        let kind = match &block.kind {
            MarkdownBlockKind::Paragraph => ShapeKind::Paragraph,
            MarkdownBlockKind::Heading(level) => ShapeKind::Heading(*level),
            MarkdownBlockKind::Code { language } => ShapeKind::Code {
                header: !language.is_empty() || self.code_accessory.is_some(),
            },
            MarkdownBlockKind::Rule => ShapeKind::Rule,
        };
        let gap = match previous {
            None => 0.0,
            // Items of a tight list, and the blocks inside one item, sit close together.
            Some(prev) if !block.blank_before && block.list_depth > 0 && prev.list_depth > 0 => {
                self.spacing / 3.0
            }
            Some(_) if matches!(kind, ShapeKind::Heading(1 | 2)) => self.spacing * 1.5,
            Some(_) => self.spacing,
        };
        RowShape {
            kind,
            quote_depth: block.quote_depth,
            list_depth: block.list_depth,
            marker: block.marker,
            gap,
        }
    }

    /// A block's text as a label that follows its signal, with the view's color, link
    /// handling, selection and id applied.
    fn text(&self, block: Signal<MarkdownBlock>, index: usize, look: &TextLook) -> AnyPiece {
        let mut text = label(move || block.with(|b| b.text.clone())).font(look.font);
        if look.inline {
            text = text.markdown();
        }
        if look.bold {
            text = text.bold();
        }
        if look.monospace {
            text = text.monospace();
        }
        text.color.clone_from(&self.color);
        text.on_link.clone_from(&self.on_link);
        let mut piece = text.any();
        if self.selectable {
            piece = piece.selectable().any();
        }
        if let Some(id) = &self.block_id {
            piece = piece.id(id(index)).any();
        }
        piece
    }

    fn build(&self, shape: &RowShape, block: Signal<MarkdownBlock>, index: usize) -> AnyPiece {
        let body = match shape.kind {
            ShapeKind::Paragraph => self.text(
                block,
                index,
                &TextLook {
                    font: self.font,
                    inline: true,
                    bold: false,
                    monospace: false,
                },
            ),
            ShapeKind::Heading(level) => self
                .text(
                    block,
                    index,
                    &TextLook {
                        font: heading_font(level),
                        inline: true,
                        // The two smallest levels share a size with body text and captions,
                        // so weight is what marks them as headings.
                        bold: level >= 5,
                        monospace: false,
                    },
                )
                .a11y(move |a| a.role(Role::Heading(level)))
                .any(),
            ShapeKind::Code { header } => self.code(block, index, header),
            ShapeKind::Rule => divider().any(),
        };
        let top = Insets::top(shape.gap);
        let mut cells: Vec<AnyPiece> = Vec::new();
        for _ in 0..shape.quote_depth {
            cells.push(divider().vertical().any());
        }
        let outer = f64::from(shape.list_depth.saturating_sub(1)) * LIST_INDENT;
        let marker = match shape.marker {
            ListStyle::None => None,
            ListStyle::Bullet => Some("\u{2022}".to_owned()),
            ListStyle::Ordered(n) => Some(format!("{n}.")),
        };
        let has_marker = marker.is_some();
        let body_indent = if let Some(marker) = marker {
            cells.push(
                label(marker)
                    .font(self.font)
                    .secondary()
                    .min_width(MARKER_WIDTH)
                    .padding(Insets {
                        top: shape.gap,
                        leading: outer,
                        ..Insets::default()
                    })
                    .any(),
            );
            0.0
        } else {
            f64::from(shape.list_depth) * LIST_INDENT
        };
        cells.push(
            body.padding(Insets {
                leading: body_indent,
                ..top
            })
            .grow(Dimension::Width)
            .any(),
        );
        row(cells)
            .spacing(ROW_GAP)
            .align(if has_marker {
                VAlign::FirstBaseline
            } else {
                VAlign::Top
            })
            .any()
    }

    /// A code block: the code in the monospaced face on the platform's card surface, under a
    /// header that names the language and holds the app's accessory.
    fn code(&self, block: Signal<MarkdownBlock>, index: usize, header: bool) -> AnyPiece {
        let code = self.text(
            block,
            index,
            &TextLook {
                font: self.font,
                inline: false,
                bold: false,
                monospace: true,
            },
        );
        let mut lines: Vec<AnyPiece> = Vec::new();
        if header {
            let handle = MarkdownCode { block };
            let mut cells: Vec<AnyPiece> = vec![
                label(move || handle.language())
                    .font(Font::Caption)
                    .secondary()
                    .any(),
                spacer().any(),
            ];
            if let Some(accessory) = &self.code_accessory {
                cells.push(accessory(handle));
            }
            lines.push(row(cells).any());
        }
        lines.push(code);
        column(lines)
            .spacing(Space::XS)
            .align(HAlign::Leading)
            .card()
            .any()
    }
}

/// How one block's label is drawn.
struct TextLook {
    font: Font,
    /// Read the text as inline Markdown (a paragraph or heading) or literally (code).
    inline: bool,
    bold: bool,
    monospace: bool,
}

/// The semantic style a heading level takes, so headings follow the platform's type ramp and
/// the reader's text size like every other label.
fn heading_font(level: u8) -> Font {
    match level {
        1 => Font::Title,
        2 => Font::Title2,
        3 => Font::Title3,
        4 => Font::Headline,
        5 => Font::Subheadline,
        _ => Font::Footnote,
    }
}

impl Piece for Markdown {
    fn build(self, cx: &mut BuildCx) -> RNode {
        let Markdown {
            text,
            font,
            color,
            spacing,
            selectable,
            on_link,
            code_accessory,
            block_id,
        } = self;
        let style = Rc::new(RowStyle {
            font,
            color,
            spacing,
            selectable,
            on_link,
            code_accessory,
            block_id,
        });
        let rows = piece_fn(move |cx| {
            let anchor = cx.layout_only(
                Rc::new(PassThrough),
                Flex {
                    is_group: true,
                    ..Default::default()
                },
                Boundary::No,
            );
            // Rows are built from a reaction, where `Scope::child()` would parent them to the
            // root scope; they belong to this piece (the same capture `each` makes).
            let owner = Scope::current();
            let document = Rc::new(RefCell::new(MarkdownDocument::new()));
            let built: Rc<RefCell<Vec<BlockRow>>> = Rc::new(RefCell::new(Vec::new()));
            let sync = move |source: &str| {
                if !with_tree(|t| t.node_exists(anchor)) {
                    return;
                }
                let first = document.borrow_mut().update(source);
                let document = document.borrow();
                let blocks = document.blocks();
                let mut rows = built.borrow_mut();
                for index in first..blocks.len() {
                    let block = &blocks[index];
                    let previous = index.checked_sub(1).and_then(|i| blocks.get(i));
                    let shape = style.shape(block, previous);
                    if let Some(row) = rows.get(index)
                        && row.shape == shape
                    {
                        // Same shape: the row's labels follow the signal.
                        if row.block.with_untracked(|b| b != block) {
                            row.block.set(block.clone());
                        }
                        continue;
                    }
                    // The shape changed here: this row and the ones after it are built again,
                    // so the rows stay in document order by appending.
                    let stale = index.min(rows.len());
                    for row in rows.drain(stale..) {
                        row.scope.dispose();
                        with_tree(|t| t.remove_subtree(row.root));
                    }
                    let scope = owner.enter(Scope::child);
                    let (signal, root) = scope.enter(|| {
                        let signal = Signal::new(block.clone());
                        let mut cx = BuildCx::new(anchor);
                        (signal, style.build(&shape, signal, index).build(&mut cx))
                    });
                    rows.push(BlockRow {
                        shape,
                        block: signal,
                        scope,
                        root,
                    });
                }
                let extra = blocks.len().min(rows.len());
                for row in rows.drain(extra..) {
                    row.scope.dispose();
                    with_tree(|t| t.remove_subtree(row.root));
                }
            };
            sync(&text.initial());
            if let TextSource::Dyn(source) = text {
                watch(move || source(), move |now: &String, _| sync(now));
            }
            anchor
        });
        column((rows,)).align(HAlign::Leading).build(cx)
    }
}

// ---------------------------------------------------------------------------
// Conformance cases (docs/testing.md)
// ---------------------------------------------------------------------------

/// The Markdown view's `#[day::test]` cases. Compiled only under the `conformance` feature.
#[cfg(feature = "conformance")]
pub(crate) mod conformance {
    use day_core::Piece;
    use day_core::conformance::{Case, Drive, FrameExpect};
    use day_reactive::Signal;
    use day_spec::{Cap, kinds};

    use crate::*;

    /// A marker box, so a case can compare heights by where two markers sit.
    fn foot(id: day_core::ElementId) -> impl Piece {
        rectangle()
            .fill(day_spec::Color::hex(0x0033_6699))
            .frame(10.0, 4.0)
            .id(id)
    }

    const DOCUMENT: &str = "# Release notes\n\nThis build is **faster** and has `fewer` \
        [bugs](https://daybrite.dev/bugs).\n\n- First item\n- Second item\n  1. Nested step\n\n\
        > A quoted line\n\n```rust\nfn main() {}\n```\n\n---\n\nThe end.";

    /// Every block of a document becomes its own label, with the Markdown markers stripped
    /// and the code left verbatim.
    #[day_macros::test(day_core)]
    fn markdown_view_blocks() -> Case {
        Case::new()
            .proves(kinds::LABEL)
            .requires(Cap::TextRuns)
            .page(|| {
                markdown(DOCUMENT)
                    .block_ids(|i| day_core::id!(md_block, i))
                    .width(300.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_text("md_block:0", "Release notes").await?;
                d.assert_text("md_block:1", "This build is faster and has fewer bugs.")
                    .await?;
                d.assert_text("md_block:2", "First item").await?;
                d.assert_text("md_block:3", "Second item").await?;
                d.assert_text("md_block:4", "Nested step").await?;
                d.assert_text("md_block:5", "A quoted line").await?;
                d.assert_text("md_block:6", "fn main() {}").await?;
                // Block 7 is the rule, which has no text and no id.
                d.assert_missing("md_block:7").await?;
                d.assert_text("md_block:8", "The end.").await
            })
    }

    /// The view takes its height from its content: what follows it sits directly under its
    /// last block, and moves down when the text grows. A reply that arrives a piece at a time
    /// updates the block being written and adds blocks as they start.
    #[day_macros::test(day_core)]
    fn markdown_view_streams() -> Case {
        let reply = Signal::new("# Answer\n\nThe first".to_owned());
        Case::new()
            .proves(kinds::LABEL)
            .requires(Cap::TextRuns)
            .page(move || {
                column((
                    row((
                        column((
                            markdown(reply)
                                .block_ids(|i| day_core::id!(mds_block, i))
                                .width(200.0),
                            foot(day_core::id!(mds_after)),
                        ))
                        .align(HAlign::Leading),
                        column((
                            label("Short").width(100.0),
                            foot(day_core::id!(mds_reference)),
                        ))
                        .align(HAlign::Leading),
                    ))
                    .align(VAlign::Top),
                    button("More")
                        .action(move || reply.update(|r| r.push_str(" paragraph")))
                        .id(day_core::id!(mds_more)),
                    button("List")
                        .action(move || reply.update(|r| r.push_str(".\n\n- one\n- two")))
                        .id(day_core::id!(mds_list)),
                    button("Replace")
                        .action(move || reply.set("Short".into()))
                        .id(day_core::id!(mds_replace)),
                ))
                .align(HAlign::Leading)
            })
            .drive(|d: Drive| async move {
                d.assert_text("mds_block:0", "Answer").await?;
                d.assert_text("mds_block:1", "The first").await?;
                d.tap("mds_more").await?;
                d.assert_text("mds_block:1", "The first paragraph").await?;
                d.assert_text("mds_block:0", "Answer").await?;
                d.tap("mds_list").await?;
                d.assert_text("mds_block:1", "The first paragraph.").await?;
                d.assert_text("mds_block:2", "one").await?;
                d.assert_text("mds_block:3", "two").await?;
                d.tap("mds_replace").await?;
                d.wait_idle().await?;
                d.assert_text("mds_block:0", "Short").await?;
                d.assert_missing("mds_block:1").await?;
                // One paragraph left: the view is as tall as a plain label with the same text,
                // so what follows each sits at the same height.
                d.assert_frame(
                    "mds_after",
                    FrameExpect {
                        y: Some(0.0),
                        relative_to: Some("mds_reference".into()),
                        ..Default::default()
                    },
                )
                .await
            })
    }

    /// A code block's accessory reads the block's current code and language.
    #[day_macros::test(day_core)]
    fn markdown_view_code_accessory() -> Case {
        let source = Signal::new("```swift\nlet day = 1\n```".to_owned());
        let copied = Signal::new(String::new());
        Case::new()
            .proves(kinds::LABEL)
            .page(move || {
                column((
                    markdown(source)
                        .block_ids(|i| day_core::id!(mdc_block, i))
                        .code_accessory(move |code| {
                            button("Copy")
                                .action(move || {
                                    copied.set(format!("{}: {}", code.language(), code.text()));
                                })
                                .id(day_core::id!(mdc_copy))
                        })
                        .width(300.0),
                    label(move || copied.get()).id(day_core::id!(mdc_copied)),
                    button("Grow")
                        .action(move || {
                            source.set("```swift\nlet day = 1\nlet night = 2\n```".into());
                        })
                        .id(day_core::id!(mdc_grow)),
                ))
                .align(HAlign::Leading)
                .spacing(8.0)
            })
            .shot("default")
            .drive(|d: Drive| async move {
                d.assert_text("mdc_block:0", "let day = 1").await?;
                d.tap("mdc_copy").await?;
                d.assert_text("mdc_copied", "swift: let day = 1").await?;
                d.tap("mdc_grow").await?;
                d.assert_text("mdc_block:0", "let day = 1\nlet night = 2")
                    .await?;
                d.tap("mdc_copy").await?;
                d.assert_text("mdc_copied", "swift: let day = 1\nlet night = 2")
                    .await
            })
    }
}
