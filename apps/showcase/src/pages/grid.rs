use day::prelude::*;

day::routes! {
    /// The Grid example's sub-pages, typed (docs/grid.md): tabs on desktop, push/pop pages on
    /// mobile — the same keys either way, so deep links and dayscript address both hosts alike.
    enum GridDemo {
        Basics => "basics",
        Sizing => "sizing",
        Spanning => "spanning",
        Composite => "composite",
        Stress => "stress",
    }
}

/// The `grid` piece (docs/grid.md) from simple to stress test. Desktop (`Cap::NavSplit`) gets a
/// flat tab strip; mobile gets a menu that pushes each demo onto a navigation stack.
pub(crate) fn grid_page() -> AnyPiece {
    if capability(Cap::NavSplit) == Support::Native {
        let tab = Signal::new(GridDemo::Basics);
        selector(tab)
            .style(SelectorStyle::Tabs)
            .item(
                GridDemo::Basics,
                crate::res::str::grid_tab_basics(),
                basics_demo,
            )
            .item(
                GridDemo::Sizing,
                crate::res::str::grid_tab_sizing(),
                sizing_demo,
            )
            .item(
                GridDemo::Spanning,
                crate::res::str::grid_tab_spanning(),
                spanning_demo,
            )
            .item(
                GridDemo::Composite,
                crate::res::str::grid_tab_composite(),
                composite_demo,
            )
            .item(
                GridDemo::Stress,
                crate::res::str::grid_tab_stress(),
                stress_demo,
            )
            .id("grid-tabs")
    } else {
        let path = Signal::new(Vec::<GridDemo>::new());
        let push = |demo: GridDemo| {
            move || {
                let mut v = path.get_untracked();
                v.push(demo);
                path.set(v);
            }
        };
        let menu = crate::widgets::page(
            crate::res::str::nav_grid(),
            "grid-title",
            Some(crate::res::str::grid_caption()),
            column((
                button(crate::res::str::grid_tab_basics())
                    .action(push(GridDemo::Basics))
                    .id("grid-open-basics"),
                button(crate::res::str::grid_tab_sizing())
                    .action(push(GridDemo::Sizing))
                    .id("grid-open-sizing"),
                button(crate::res::str::grid_tab_spanning())
                    .action(push(GridDemo::Spanning))
                    .id("grid-open-spanning"),
                button(crate::res::str::grid_tab_composite())
                    .action(push(GridDemo::Composite))
                    .id("grid-open-composite"),
                button(crate::res::str::grid_tab_stress())
                    .action(push(GridDemo::Stress))
                    .id("grid-open-stress"),
            ))
            .spacing(10.0)
            .align(HAlign::Leading)
            .any(),
        );
        stack(path, menu)
            .destination(|demo: &GridDemo| match demo {
                GridDemo::Basics => basics_demo(),
                GridDemo::Sizing => sizing_demo(),
                GridDemo::Spanning => spanning_demo(),
                GridDemo::Composite => composite_demo(),
                GridDemo::Stress => stress_demo(),
            })
            .id("grid-stack")
    }
}

/// Shared sub-page scaffold: title + caption over scrollable padded content (the tabs.rs pane
/// shape, plus scrolling so the stress page works everywhere).
fn pane(title: LocalizedText, caption: LocalizedText, body: AnyPiece) -> AnyPiece {
    scroll(
        column((
            label(title).font(Font::Title),
            label(caption).font(Font::Footnote),
            body,
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .padding(16.0),
    )
    .any()
}

/// Columns infer from rows; each column takes its widest cell's width. The `divider()` is a bare
/// (non-row) child, so it spans the full grid.
fn basics_demo() -> AnyPiece {
    fn score_row(name: &'static str, wins: &'static str, points: &'static str) -> AnyPiece {
        grid_row((
            label(name),
            label(wins).grid_align(Alignment::Trailing),
            label(points).grid_align(Alignment::Trailing),
        ))
        .any()
    }
    pane(
        crate::res::str::grid_tab_basics(),
        crate::res::str::grid_basics_caption(),
        grid((
            grid_row((
                label(crate::res::str::grid_col_name()).font(Font::Headline),
                label(crate::res::str::grid_col_wins())
                    .font(Font::Headline)
                    .grid_align(Alignment::Trailing),
                label(crate::res::str::grid_col_points())
                    .font(Font::Headline)
                    .grid_align(Alignment::Trailing),
            )),
            divider(),
            score_row("Ada", "3", "128"),
            score_row("Grace", "11", "1024"),
            score_row("Katherine", "7", "512"),
        ))
        .column_spacing(24.0)
        .row_spacing(6.0)
        .align(Alignment::Leading)
        .id("grid-basics"),
    )
}

/// One grid, three column behaviors: a fixed 80 pt column (`.width`), a content-sized column,
/// and a flexible column (`.grow_w`) that takes whatever width is left.
fn sizing_demo() -> AnyPiece {
    fn bar(color: u32) -> AnyPiece {
        capsule().fill(Color::hex(color)).height(10.0).grow_w()
    }
    pane(
        crate::res::str::grid_tab_sizing(),
        crate::res::str::grid_sizing_caption(),
        grid((
            grid_row((
                label(crate::res::str::grid_sizing_fixed())
                    .font(Font::Footnote)
                    .width(80.0),
                label(crate::res::str::grid_sizing_short()),
                bar(0x2F6FDE),
            )),
            grid_row((
                label(crate::res::str::grid_sizing_fixed())
                    .font(Font::Footnote)
                    .width(80.0),
                label(crate::res::str::grid_sizing_longer()),
                bar(0x27AE60),
            )),
            grid_row((
                label(crate::res::str::grid_sizing_fixed())
                    .font(Font::Footnote)
                    .width(80.0),
                label(crate::res::str::grid_sizing_content()),
                bar(0xE67E22),
            )),
        ))
        .spacing(10.0)
        .align(Alignment::Leading)
        .id("grid-sizing"),
    )
}

/// A week planner: a full-width title, seven day columns, and event cells spanning two and
/// three columns via `.grid_span` — with `spacer()` holding the empty day slots.
fn spanning_demo() -> AnyPiece {
    fn day_cells(from: u32) -> AnyPiece {
        let cells: Vec<AnyPiece> = (from..from + 7)
            .map(|n| {
                // Grid modifiers go LAST so the facts land on the outermost (width) wrapper.
                label(n.to_string())
                    .width(32.0)
                    .grid_align(Alignment::Center)
            })
            .collect();
        grid_row(PieceVec(cells)).any()
    }
    fn event(text: LocalizedText, color: u32, span: usize) -> AnyPiece {
        label(text)
            .font(Font::Footnote)
            .padding(Insets::symmetric(8.0, 4.0))
            .background(Color::hex(color))
            .corner_radius(9.0)
            .grid_span(span)
    }
    pane(
        crate::res::str::grid_tab_spanning(),
        crate::res::str::grid_spanning_caption(),
        grid((
            label(crate::res::str::grid_month_title()).font(Font::Headline),
            day_cells(1),
            grid_row((
                event(crate::res::str::grid_event_focus(), 0xBBDEFB, 2),
                spacer(),
                event(crate::res::str::grid_event_review(), 0xFFD24A, 3),
            )),
            day_cells(8),
        ))
        .spacing(8.0)
        .align(Alignment::Leading)
        .id("grid-spanning"),
    )
}

const SUN: Color = Color {
    r: 0.98,
    g: 0.78,
    b: 0.19,
    a: 1.0,
};
const CLOUD: Color = Color {
    r: 0.62,
    g: 0.68,
    b: 0.76,
    a: 1.0,
};
const BOLT: Color = Color {
    r: 1.0,
    g: 0.84,
    b: 0.25,
    a: 1.0,
};

/// A sun: eight `line` rays around a `circle` disc, flattened into ONE canvas leaf.
fn sun_glyph(size: f64) -> AnyPiece {
    let mut shapes = vec![circle().fill(SUN).at(0.28, 0.28, 0.44, 0.44)];
    for k in 0..8 {
        let (s, c) = (k as f64 * std::f64::consts::FRAC_PI_4).sin_cos();
        shapes.push(
            line(
                (0.5 + 0.36 * c, 0.5 + 0.36 * s),
                (0.5 + 0.48 * c, 0.5 + 0.48 * s),
            )
            .stroke(SUN, size * 0.07),
        );
    }
    shape_group(shapes).frame(size, size)
}

/// A storm cloud: rounded-rect body, two ellipse puffs, and a `polygon` lightning bolt.
fn storm_glyph(size: f64) -> AnyPiece {
    shape_group([
        rounded_rectangle(size * 0.09)
            .fill(CLOUD)
            .at(0.08, 0.3, 0.84, 0.34),
        ellipse().fill(CLOUD).at(0.14, 0.12, 0.42, 0.42),
        ellipse().fill(CLOUD).at(0.42, 0.18, 0.42, 0.42),
        polygon([
            (0.52, 0.64),
            (0.40, 0.88),
            (0.50, 0.88),
            (0.44, 1.02),
            (0.62, 0.78),
            (0.52, 0.78),
            (0.60, 0.64),
        ])
        .fill(BOLT),
    ])
    .frame(size, size)
}

/// A low→high range bar whose geometry derives from the laid-out width (`shape_group_fn`).
fn range_bar(low: f64, high: f64, wmin: f64, wmax: f64) -> AnyPiece {
    shape_group_fn(move |size| {
        if size.width <= 0.0 || size.height <= 0.0 {
            return Vec::new();
        }
        let h = (6.0 / size.height).min(1.0);
        let y = 0.5 - h / 2.0;
        let span = (wmax - wmin).max(1.0);
        let x0 = ((low - wmin) / span).clamp(0.0, 1.0);
        let x1 = ((high - wmin) / span).clamp(0.0, 1.0);
        let w = (x1 - x0).max(6.0 / size.width);
        vec![
            capsule()
                .fill(Color::rgba(0.5, 0.55, 0.6, 0.25))
                .at(0.0, y, 1.0, h),
            capsule()
                .fill_linear(LinearGradient::new(
                    UnitPoint::LEADING,
                    UnitPoint::TRAILING,
                    vec![(0.0, Color::hex(0x5AA9E6)), (1.0, Color::hex(0xE67E22))],
                ))
                .at(x0, y, w, h),
        ]
    })
    .height(20.0)
    .grow_w()
}

/// Shapes and grid together — the Day Skies forecast shape: content-sized day and temperature
/// columns, glyph groups, and ONE flexible column (the range bar) taking the leftover width.
fn composite_demo() -> AnyPiece {
    const DAYS: [(i64, bool, f64, f64); 5] = [
        (1, true, 14.0, 24.0),
        (2, true, 16.0, 27.0),
        (3, false, 11.0, 18.0),
        (4, false, 9.0, 15.0),
        (5, true, 12.0, 22.0),
    ];
    let wmin = 9.0;
    let wmax = 27.0;
    let rows: Vec<AnyPiece> = DAYS
        .iter()
        .map(|&(n, sunny, low, high)| {
            let glyph = if sunny {
                sun_glyph(26.0)
            } else {
                storm_glyph(26.0)
            };
            grid_row((
                label(crate::res::str::grid_day_n(n)).grid_align(Alignment::Leading),
                glyph,
                label(format!("{low:.0}°"))
                    .font(Font::Footnote)
                    .grid_align(Alignment::Trailing),
                range_bar(low, high, wmin, wmax),
                label(format!("{high:.0}°")).grid_align(Alignment::Trailing),
            ))
            .any()
        })
        .collect();
    pane(
        crate::res::str::grid_tab_composite(),
        crate::res::str::grid_composite_caption(),
        grid(PieceVec(rows))
            .column_spacing(12.0)
            .row_spacing(8.0)
            .id("grid-composite"),
    )
}

/// An eager 100×8-cell grid (grows by 50 rows per tap) driven through `each`, with one
/// signal-bound cell: bumping it exercises the single-dirty-cell relayout path.
fn stress_demo() -> AnyPiece {
    const COLS: i64 = 8;
    let rows = Signal::new(100i64);
    let hot = Signal::new(0i64);
    let body = grid((each(
        move || (0..rows.get()).collect::<Vec<i64>>(),
        |r| *r,
        move |slot| {
            let r = slot.key();
            let mut cells: Vec<AnyPiece> = Vec::with_capacity(COLS as usize);
            for c in 0..COLS {
                if r == 0 && c == 0 {
                    cells.push(
                        label(move || hot.get().to_string())
                            .id("grid-stress-hot")
                            .any(),
                    );
                } else if c == COLS - 1 {
                    cells.push(
                        capsule()
                            .fill(Color::hex(if r % 2 == 0 { 0x2F6FDE } else { 0x27AE60 }))
                            .height(8.0)
                            .grow_w(),
                    );
                } else {
                    cells.push(
                        label(((r * 37 + c * 11) % 97).to_string())
                            .font(Font::Footnote)
                            .grid_align(Alignment::Trailing)
                            .any(),
                    );
                }
            }
            grid_row(PieceVec(cells)).any()
        },
    ),))
    .spacing(4.0)
    .align(Alignment::Leading)
    .id("grid-stress");
    pane(
        crate::res::str::grid_tab_stress(),
        crate::res::str::grid_stress_cells(rows),
        column((
            row((
                button(crate::res::str::grid_stress_add())
                    .action(move || rows.update(|r| *r += 50))
                    .id("grid-stress-add"),
                button(crate::res::str::grid_stress_bump())
                    .action(move || hot.update(|h| *h += 1))
                    .id("grid-stress-bump"),
            ))
            .spacing(12.0),
            body,
        ))
        .spacing(12.0)
        .align(HAlign::Leading)
        .any(),
    )
}
