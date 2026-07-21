//! Grid acceptance (docs/grid.md) on the mock toolkit: column inference, spans, alignment
//! precedence, height-for-width, reactive reflow — and the performance contract (two measure
//! proposals per cell, bounded re-measures on update) as golden assertions, per §7.4.

use day_core::AnyPiece;
use day_mock::{MockProbe, MockToolkit};
use day_pieces::prelude::*;
use day_reactive::flush_sync;
use day_spec::{Event, NodeId, WindowOptions};

fn boot(root: impl FnOnce() -> AnyPiece + 'static) -> MockProbe {
    day_core::uninstall_tree();
    let (mock, probe) = MockToolkit::new();
    let options = WindowOptions {
        title: "test".into(),
        size: Size::new(400.0, 600.0),
        ..Default::default()
    };
    day_core::launch_with(mock, options, root);
    probe
}

/// Frames of every `day.label`, in handle-creation (= declaration) order.
fn label_frames(probe: &MockProbe) -> Vec<Rect> {
    probe
        .find_by_kind("day.label")
        .iter()
        .map(|(_, w)| w.frame)
        .collect()
}

#[test]
fn grid_columns_size_to_max_cell() {
    let probe = boot(|| {
        grid((
            grid_row((label("aa"), label("bbbb"))),
            grid_row((label("cccccc"), label("d"))),
        ))
        .spacing(10.0)
        .align(Alignment::TopLeading)
        .any()
    });
    // Mock metrics are 8pt/char × 16pt line: col0 = max(16, 48) = 48, col1 = max(32, 8) = 32.
    let f = label_frames(&probe);
    assert_eq!(f[0], Rect::new(0.0, 0.0, 16.0, 16.0), "{f:?}");
    assert_eq!(
        f[1],
        Rect::new(58.0, 0.0, 32.0, 16.0),
        "col1 = col0 + gutter"
    );
    assert_eq!(
        f[2],
        Rect::new(0.0, 26.0, 48.0, 16.0),
        "row1 = row0 + gutter"
    );
    assert_eq!(f[3], Rect::new(58.0, 26.0, 8.0, 16.0), "{f:?}");
}

#[test]
fn grid_spacer_is_empty_cell() {
    let probe = boot(|| {
        grid((
            grid_row((spacer(), label("bb"))),
            grid_row((label("aaaa"), label("cc"))),
        ))
        .spacing(10.0)
        .align(Alignment::TopLeading)
        .any()
    });
    // The spacer occupies col0 without contributing width or placement: col0 = "aaaa" = 32,
    // and both col1 cells land at the same x — no `spacer().width(40)` placeholder needed.
    let f = label_frames(&probe);
    assert_eq!(f[0], Rect::new(42.0, 0.0, 16.0, 16.0), "{f:?}");
    assert_eq!(f[1], Rect::new(0.0, 26.0, 32.0, 16.0), "{f:?}");
    assert_eq!(f[2], Rect::new(42.0, 26.0, 16.0, 16.0), "{f:?}");
}

#[test]
fn grid_flexible_column_shares_leftover() {
    let probe = boot(|| {
        grid((
            grid_row((label("aaaa"), rectangle().fill(Color::WHITE))),
            grid_row((label("bb"), label("cc"))),
        ))
        .align(Alignment::TopLeading)
        .height(32.0) // pin the height: the grow_h row would stretch to the proposal
        .any()
    });
    // A shape grows on both axes, so col1 is flexible: 400 (window) − 32 (col0) = 368. The
    // grow_h cell stretches to the row height set by its 16pt neighbour.
    let canvas = &probe.find_by_kind("day.canvas")[0].1;
    assert_eq!(canvas.frame, Rect::new(32.0, 0.0, 368.0, 16.0));
    // A rigid cell in the flexible column keeps its own width.
    let f = label_frames(&probe);
    assert_eq!(f[2], Rect::new(32.0, 16.0, 16.0, 16.0), "{f:?}");
}

#[test]
fn grid_span_and_full_width() {
    let probe = boot(|| {
        grid((
            grid_row((label("aaaa"), label("bb"))),
            grid_row((label("cccccccccc").grid_span(2),)),
            label("dddd"), // a non-row child = full-width cell
        ))
        .column_spacing(10.0)
        .align(Alignment::TopLeading)
        .any()
    });
    // Initial ideals: col0 = 32, col1 = 16. The span-2 cell needs 80 > 32 + 10 + 16 = 58, so
    // its 22pt deficit distributes evenly: col0 = 43, col1 = 27.
    let f = label_frames(&probe);
    assert_eq!(f[0], Rect::new(0.0, 0.0, 32.0, 16.0), "{f:?}");
    assert_eq!(f[1], Rect::new(53.0, 0.0, 16.0, 16.0), "col1 x = 43 + 10");
    assert_eq!(f[2], Rect::new(0.0, 16.0, 80.0, 16.0), "span cell");
    assert_eq!(f[3], Rect::new(0.0, 32.0, 32.0, 16.0), "full-width cell");
}

#[test]
fn grid_alignment_precedence() {
    let probe = boot(|| {
        grid((
            grid_row((
                rectangle().fill(Color::WHITE).frame(20.0, 40.0),
                label("aa"),
            )),
            grid_row((
                rectangle().fill(Color::WHITE).frame(20.0, 40.0),
                label("aa"),
            ))
            .align(VAlign::Bottom),
            grid_row((
                rectangle().fill(Color::WHITE).frame(20.0, 40.0),
                label("aa").grid_align(Alignment::BottomTrailing),
            )),
            grid_row((spacer(), label("bbbb"))),
        ))
        .align(Alignment::TopLeading)
        .any()
    });
    // col0 = 20 (fixed frames), col1 = 32 ("bbbb"); 40pt rows for r0-r2, 16pt for r3.
    let f = label_frames(&probe);
    // r0: grid alignment (top-leading).
    assert_eq!(f[0], Rect::new(20.0, 0.0, 16.0, 16.0), "{f:?}");
    // r1: the row's VAlign::Bottom overrides the grid's vertical alignment.
    assert_eq!(
        f[1],
        Rect::new(20.0, 64.0, 16.0, 16.0),
        "row valign: y = 40 + (40 − 16)"
    );
    // r2: the cell's .grid_align overrides both, on both axes.
    assert_eq!(
        f[2],
        Rect::new(36.0, 104.0, 16.0, 16.0),
        "cell align: x = 20 + (32 − 16), y = 80 + (40 − 16)"
    );
    assert_eq!(f[3], Rect::new(20.0, 120.0, 32.0, 16.0), "{f:?}");
}

#[test]
fn grid_height_for_width_rewrap() {
    let probe = boot(|| {
        grid((grid_row((
            label("aa"),
            // 30 chars × 8 = 240pt ideal; the flexible column will be 84pt wide.
            label("abcdefghijklmnopqrstuvwxyz1234").grow_w(),
        )),))
        .align(Alignment::TopLeading)
        .width(100.0)
        .any()
    });
    // Pass B re-measures the flexible cell at its final 100 − 16 = 84pt column width, so the
    // text wraps to 3 lines (10 chars/line) and the ROW grows to fit — height-for-width.
    let f = label_frames(&probe);
    assert_eq!(f[1].origin.x, 16.0, "{f:?}");
    assert_eq!(f[1].size, Size::new(84.0, 48.0), "3 wrapped lines: {f:?}");
}

#[test]
fn grid_each_rows_reflow() {
    let items = Signal::new(vec!["aa".to_string(), "bb".to_string()]);
    let probe = boot(move || {
        grid((each(
            move || items.get(),
            |s| s.clone(),
            |slot| grid_row((label(move || slot.get()), label("x"))).any(),
        ),))
        .column_spacing(10.0)
        .align(Alignment::TopLeading)
        .any()
    });
    // col0 = 16 → every col1 cell at x = 26.
    let xs: Vec<f64> = label_frames(&probe).iter().map(|f| f.origin.x).collect();
    assert_eq!(xs, vec![0.0, 26.0, 0.0, 26.0], "{xs:?}");

    // A wider appended row renegotiates col0 for EVERY row (cross-row reflow).
    batch(|| items.update(|v| v.push("cccc".to_string())));
    flush_sync();
    let f = label_frames(&probe);
    let xs: Vec<f64> = f.iter().map(|f| f.origin.x).collect();
    assert_eq!(
        xs,
        vec![0.0, 42.0, 0.0, 42.0, 0.0, 42.0],
        "col0 must widen to 32 for all rows: {f:?}"
    );
}

#[test]
fn grid_in_scroll_reports_content() {
    let probe = boot(|| {
        let rows: Vec<AnyPiece> = (0..40)
            .map(|r| grid_row((label(format!("row {r}")), label("x"))).any())
            .collect();
        scroll(grid(PieceVec(rows)).spacing(10.0)).any()
    });
    // Unconstrained-height measure inside the viewport: 40 × 16 + 39 × 10 = 1030 (> the 600pt
    // window, so the viewport clamp doesn't mask it).
    let scrolls = probe.find_by_kind("day.scroll");
    assert_eq!(scrolls[0].1.scroll_content.height, 1030.0);
}

#[test]
fn grid_rtl_mirrors_columns() {
    day_core::set_layout_direction(day_geometry::LayoutDirection::Rtl);
    let probe = boot(|| {
        grid((grid_row((label("aa"), rectangle().fill(Color::WHITE))),))
            .align(Alignment::TopLeading)
            .frame(400.0, 16.0)
            .any()
    });
    // Geometry is computed LTR and mirrored at place time around the GRID's width (400):
    // col0 ("aa", 16pt) lands on the right edge, the flexible shape on the left.
    let f = label_frames(&probe);
    assert_eq!(f[0], Rect::new(384.0, 0.0, 16.0, 16.0), "{f:?}");
    let canvas = &probe.find_by_kind("day.canvas")[0].1;
    assert_eq!(canvas.frame, Rect::new(0.0, 0.0, 384.0, 16.0));
}

#[test]
fn grid_measure_calls_bounded() {
    const ROWS: usize = 100;
    const COLS: usize = 6;
    let hot = Signal::new("aaaa".to_string());
    let probe = boot(move || {
        let rows: Vec<AnyPiece> = (0..ROWS)
            .map(|r| {
                let mut cells: Vec<AnyPiece> = Vec::with_capacity(COLS);
                for c in 0..COLS {
                    if r == 0 && c == 0 {
                        cells.push(label(move || hot.get()).any());
                    } else {
                        cells.push(label(format!("r{r}c{c}")).any());
                    }
                }
                grid_row(PieceVec(cells)).any()
            })
            .collect();
        column((
            button("+").action(move || hot.set("bbbb".to_string())),
            grid(PieceVec(rows)).spacing(2.0),
        ))
        .any()
    });
    let cells = ROWS * COLS;
    // THE performance contract (docs/grid.md): two proposals per cell — unconstrained (pass A)
    // and at the final column width (pass B). `place` re-runs the same proposals from cache.
    assert!(
        probe.measure_calls() <= 2 * cells + 60,
        "boot measured {} times for {} cells",
        probe.measure_calls(),
        cells
    );

    // One dirty cell re-measures only itself (same metrics → no frame ops, one text op).
    probe.clear_log(); // also resets the measure counter
    let btn = probe.find_by_kind("day.button")[0].1.node;
    probe.emit(NodeId(btn), Event::Pressed);
    let muts: Vec<String> = probe
        .mutations()
        .into_iter()
        .filter(|m| !m.starts_with("a11y"))
        .collect();
    assert_eq!(muts.len(), 1, "expected one mutation, got: {muts:?}");
    assert!(
        probe.measure_calls() <= 6,
        "single-cell update re-measured {} times",
        probe.measure_calls()
    );
}
