// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The inspector (docs/inspector.md): window content beside a trailing properties panel,
//! show/hidden by one app-owned `Signal<bool>` — the same signal a `toolbar_toggle` and a menu
//! item bind to, so every affordance stays in step. Where `Cap::Inspector` is `Native` the
//! split is the toolkit's own trailing-pane container; everywhere else the pane is composed
//! from plain containers, and on a compact window the panel presents as a fullscreen sheet
//! instead of a pane.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use day_core::*;
use day_geometry::Insets;
use day_reactive::{bind_seeded, untrack};
use day_spec::props::{InspectorPaneProps, InspectorPatch, InspectorProps, PaneEdge};
use day_spec::{Cap, Event, Size, Support, kinds};

use crate::*;

/// The pane's default width in points — Keynote-class inspectors sit in the 250–300 range.
pub const INSPECTOR_WIDTH: f64 = 280.0;

/// Window content beside a trailing inspector panel, visibility bound to `visible`
/// (docs/inspector.md).
///
/// ```ignore
/// let show = Signal::global(false);
/// inspector(show, editor(), || form((section((/* property rows */,)),)))
/// ```
///
/// `panel` is a builder rather than a piece because the panel can be re-homed: on a compact
/// window the composed form presents it inside a fullscreen sheet instead of a side pane, and
/// each home builds it fresh in its own scope.
pub struct Inspector<V: Binding<bool>> {
    visible: V,
    width: f64,
    edge: PaneEdge,
    sheet_done: TextSource,
    content: AnyPiece,
    panel: Rc<dyn Fn() -> AnyPiece>,
}

/// Build an [`Inspector`]: `content` beside a trailing panel toggled by `visible`.
pub fn inspector<V: Binding<bool>, C: Piece, P: Piece>(
    visible: V,
    content: C,
    panel: impl Fn() -> P + 'static,
) -> Inspector<V> {
    Inspector {
        visible,
        width: INSPECTOR_WIDTH,
        edge: PaneEdge::Trailing,
        // A language-neutral glyph, because day carries no "Done" in its own catalog; apps
        // localize with `.sheet_done(…)`.
        sheet_done: "✕".into_text(),
        content: AnyPiece::new(content),
        panel: Rc::new(move || AnyPiece::new(panel())),
    }
}

impl<V: Binding<bool>> Inspector<V> {
    /// The panel's preferred width in points (default [`INSPECTOR_WIDTH`]). Where the native
    /// pane has a user-draggable divider this is the initial width, not a limit.
    pub fn width(mut self, width: f64) -> Self {
        self.width = width;
        self
    }

    /// The label of the compact sheet's dismiss button (default `✕`). Pass a localized
    /// "Done" — the sheet is the one home where the panel needs its own way out.
    pub fn sheet_done<M>(mut self, t: impl IntoText<M>) -> Self {
        self.sheet_done = t.into_text();
        self
    }

    /// Put the pane on the LEADING side of the content — a layer panel rather than a
    /// properties inspector (docs/tree.md). Default [`PaneEdge::Trailing`].
    pub fn edge(mut self, edge: PaneEdge) -> Self {
        self.edge = edge;
        self
    }
}

/// [`Inspector`]'s builders, forwarded through `Decorated` (docs/api-style.md).
pub trait InspectorBuilder: Sized {
    fn width(self, width: f64) -> Self;
    fn sheet_done<M>(self, t: impl IntoText<M>) -> Self;
    fn edge(self, edge: PaneEdge) -> Self;
}

impl<V: Binding<bool>> InspectorBuilder for Inspector<V> {
    fn width(self, width: f64) -> Self {
        Inspector::width(self, width)
    }
    fn sheet_done<M>(self, t: impl IntoText<M>) -> Self {
        Inspector::sheet_done(self, t)
    }
    fn edge(self, edge: PaneEdge) -> Self {
        Inspector::edge(self, edge)
    }
}

impl<Inner: InspectorBuilder + Piece> InspectorBuilder for Decorated<Inner> {
    fn width(self, width: f64) -> Self {
        self.map_inner(|inner| inner.width(width))
    }
    fn sheet_done<M>(self, t: impl IntoText<M>) -> Self {
        self.map_inner(|inner| inner.sheet_done(t))
    }
    fn edge(self, edge: PaneEdge) -> Self {
        self.map_inner(|inner| inner.edge(edge))
    }
}

impl<V: Binding<bool>> Piece for Inspector<V> {
    fn build(self, cx: &mut BuildCx) -> RNode {
        if day_core::capability(Cap::Inspector) == Support::Native {
            build_native(self, cx)
        } else {
            build_composed(self, cx)
        }
    }
}

/// TRACKED: is this window compact? The un-reported case (`None` — a backend with no window
/// geometry) reads as NOT compact: those are desktop-shaped surfaces, and a sheet that can
/// never be resized away would strand the panel.
fn compact(window: RNode) -> bool {
    day_core::window_size_class(window).is_some_and(|c| c.width == day_spec::WidthClass::Compact)
}

// ---------------------------------------------------------------------------
// Native: the toolkit's own trailing-pane container (`Cap::Inspector == Native`)
// ---------------------------------------------------------------------------

fn build_native<V: Binding<bool>>(inspector: Inspector<V>, cx: &mut BuildCx) -> RNode {
    let Inspector {
        visible,
        width,
        edge,
        content,
        panel,
        ..
    } = inspector;
    let initial = visible.peek();
    let sizes: Rc<RefCell<std::collections::HashMap<RNode, Size>>> = Rc::default();
    let visible_cell = Rc::new(Cell::new(initial));
    let node = cx.native(
        kinds::INSPECTOR,
        &InspectorProps {
            visible: initial,
            width,
            edge,
        },
        Rc::new(InspectorLayout {
            sizes: sizes.clone(),
            visible: visible_cell.clone(),
            width,
        }),
        Flex {
            grow_w: true,
            grow_h: true,
            ..Default::default()
        },
        Boundary::Yes,
    );

    // The two panes: content, then the panel (`InspectorPaneProps::panel`). Frames are
    // native-owned, reported like nav pages.
    let content_pane = pane(node, false, &sizes);
    let mut ccx = BuildCx::new(content_pane);
    let _ = content.build(&mut ccx);
    let panel_pane = pane(node, true, &sizes);
    let mut pcx = BuildCx::new(panel_pane);
    let _ = scroll(panel()).build(&mut pcx);

    // Signal → native pane, as a targeted patch (no rebuild). The layout cell keeps the
    // pre-report fallback split in step.
    {
        let v = visible.clone();
        let cell = visible_cell;
        bind_seeded(
            initial,
            move || v.read(),
            move |show: &bool| {
                cell.set(*show);
                with_tree(|t| {
                    t.patch(node, Box::new(InspectorPatch::Visible(*show)), false);
                    t.mark_needs_measure(node);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            },
        );
    }
    // Native pane → signal: a dock close button, a divider dragged shut. The peek guard
    // keeps the patch→event→write loop from echoing.
    cx.on(node, move |ev| {
        if let Event::InspectorChanged(show) = ev
            && visible.peek() != *show
        {
            visible.write(*show);
        }
    });
    node
}

/// One native-owned pane container under the inspector host, with the same `FrameChanged`
/// wiring as a nav page: the backend reports each pane's real frame, and Day lays the pane's
/// content out inside it.
fn pane(
    host: RNode,
    panel: bool,
    sizes: &Rc<RefCell<std::collections::HashMap<RNode, Size>>>,
) -> RNode {
    let mut cx = BuildCx::new(host);
    let pane = cx.native(
        kinds::INSPECTOR_PANE,
        &InspectorPaneProps { panel },
        Rc::new(PassThrough),
        Flex::default(),
        Boundary::Yes,
    );
    let sizes = sizes.clone();
    cx.on(pane, move |ev| {
        if let Event::FrameChanged(sz) = ev {
            let changed = sizes.borrow().get(&pane) != Some(sz);
            if changed {
                sizes.borrow_mut().insert(pane, *sz);
                with_tree(|t| {
                    t.mark_needs_measure(pane);
                    t.mark_layout_dirty();
                    t.layout_if_needed();
                });
            }
        }
    });
    pane
}

// ---------------------------------------------------------------------------
// Composed: plain containers + a compact-width sheet (every other backend)
// ---------------------------------------------------------------------------

/// The sheet's open signal, derived rather than stored: the panel is "presented as a sheet"
/// exactly while it is visible AND the window is compact, so a window resized across the
/// breakpoint re-homes the panel with no extra state to reconcile. Dismissing the sheet
/// (system back) writes straight through to the app's own signal.
struct SheetOpen<V: Binding<bool>> {
    visible: V,
    window: RNode,
}

impl<V: Binding<bool>> Clone for SheetOpen<V> {
    fn clone(&self) -> Self {
        SheetOpen {
            visible: self.visible.clone(),
            window: self.window,
        }
    }
}

impl<V: Binding<bool>> Binding<Option<String>> for SheetOpen<V> {
    fn read(&self) -> Option<String> {
        (self.visible.read() && compact(self.window)).then(|| "inspector".to_string())
    }
    fn peek(&self) -> Option<String> {
        let visible = self.visible.peek();
        (visible && untrack(|| compact(self.window))).then(|| "inspector".to_string())
    }
    fn write(&self, v: Option<String>) {
        self.visible.write(v.is_some());
    }
}

fn build_composed<V: Binding<bool>>(inspector: Inspector<V>, cx: &mut BuildCx) -> RNode {
    let Inspector {
        visible,
        width,
        edge: inspector_edge,
        sheet_done,
        content,
        panel,
    } = inspector;
    let window = day_core::toolbar::current_window();
    let side_visible = visible.clone();
    let lead_visible = visible.clone();
    let side_panel = panel.clone();
    let lead_panel = panel.clone();
    let sheet_panel = panel;
    let sheet_close = visible.clone();
    let done = sheet_done.initial();
    let edge = inspector_edge;
    row((
        // A LEADING pane sits before the content (docs/tree.md) — same mount rule as the
        // trailing one below.
        when(
            move || edge == PaneEdge::Leading && lead_visible.read() && !compact(window),
            move || row((scroll(lead_panel()).width(width), divider())),
        ),
        content.grow(),
        // The side pane: mounted only while visible on a non-compact window, so the compact
        // home (the sheet below) is never doubled.
        when(
            move || edge == PaneEdge::Trailing && side_visible.read() && !compact(window),
            move || row((divider(), scroll(side_panel()).width(width))),
        ),
        // The compact home: a fullscreen sheet. Unrouted — the inspector is chrome, not a
        // place (`Cover::unrouted`) — and carrying its own way out, since a fullscreen
        // modal has no divider to drag shut.
        cover(SheetOpen { visible, window }, move |_: &String| {
            let close = sheet_close.clone();
            column((
                row((
                    spacer(),
                    button(done.clone())
                        .action(move || close.write(false))
                        .id("day-inspector-done"),
                ))
                .padding(Insets::symmetric(12.0, 8.0)),
                scroll(sheet_panel()).grow(),
            ))
        })
        .unrouted(),
    ))
    .build(cx)
}
