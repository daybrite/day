---
title: "Charts"
description: "Compose reactive charts from marks, scales, axes, and legends."
---

<!--
Copyright © The Daybrite Project
SPDX-License-Identifier: CC-BY-SA-4.0
-->

# Charts

Charts turn data into visual comparisons that help people spot trends, differences, and
relationships. The choice of marks and scales determines which patterns stand out.

`day-piece-charts` combines marks such as bars or points with data values, scales, axes, and a
coordinate system. The resulting [canvas](/docs/reference/canvas) drawing uses the same layout logic
across toolkits.

## Add to your app

Add [`day-piece-charts`](https://crates.io/crates/day-piece-charts) to your app:

```sh
cargo add day-piece-charts
```

[Crate](https://crates.io/crates/day-piece-charts) ·
[API reference](https://docs.rs/day-piece-charts/latest/day_piece_charts/) ·
[Source](https://github.com/daybrite/day-piece-charts).
There are no chart-specific toolkit features or `Day.toml` entries. Give the chart a size or
place it in a container that allocates space. The mock toolkit can check logic but draws no UI.

## Build a chart

This component plots caller-supplied category names and values. App-authored labels use the
following messages in an app with generated [resources](/docs/reference/resources):

```ftl
chart_category = Category
chart_amount = Amount
```

```rust
use day::prelude::*;
use day_piece_charts::{bar, chart, value};

pub fn totals_chart(rows: Signal<Vec<(String, f64)>>) -> impl Piece {
    chart(move || {
        let category = res::str::chart_category().format();
        let amount = res::str::chart_amount().format();
        rows.get().into_iter().map(|(name, total)| {
            bar(value(category.clone(), name), value(amount.clone(), total))
        }).collect()
    })
    .y_label(res::str::chart_amount())
    .frame(480.0, 280.0)
}
```

[`chart()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/fn.chart.html)
reruns its marks closure when a signal read by that closure changes. Resizing also recomputes
layout. [`value()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/data/fn.value.html)
associates a channel label with a numeric or categorical value. Use
[`time()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/data/fn.time.html)
for timestamps in seconds since the Unix epoch. Axis numbers and date labels follow the locale.

## Marks, scales, and grouping

| Need | Building blocks |
| --- | --- |
| Compare categories | [`bar()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/mark/fn.bar.html) with categorical and numeric values. |
| Show a trend or range | [`line()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/mark/fn.line.html), [`area()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/mark/fn.area.html), or a mark's range modifiers. |
| Show a distribution or relationship | [`point()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/mark/fn.point.html) and [`rect()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/mark/fn.rect.html). |
| Compare series | [`Mark::by_series()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/mark/struct.Mark.html#method.by_series) for color/series identity; [`by_position()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/mark/struct.Mark.html#method.by_position) for side-by-side positioning. |
| Show proportions | [`sector()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/mark/fn.sector.html) or stacked marks in polar coordinates. |
| Change a numeric scale | [`ScaleKind`](https://docs.rs/day-piece-charts/latest/day_piece_charts/scale/enum.ScaleKind.html), including linear, log, power, and time scales. |

Keep category ordering stable when comparing updates. Non-finite values are omitted; validate
input before treating a missing mark as zero. Use explicit domains when charts need comparable
scales. Logarithmic scales require suitable positive values.

The [`Chart`](https://docs.rs/day-piece-charts/latest/day_piece_charts/struct.Chart.html)
builder controls axis labels, tick configuration, legends, colors, annotations, and coordinates.
Use [`LegendPosition`](https://docs.rs/day-piece-charts/latest/day_piece_charts/axis/enum.LegendPosition.html)
to place the legend. App-authored series names and annotations belong in the translation catalog.

## Interaction and accessibility

[`Inspect`](https://docs.rs/day-piece-charts/latest/day_piece_charts/interaction/struct.Inspect.html)
connects pointer or touch inspection to a selection signal. Configure nearest-x or nearest-mark
selection with [`Snap`](https://docs.rs/day-piece-charts/latest/day_piece_charts/select/enum.Snap.html)
and choose guide lines with [`Guides`](https://docs.rs/day-piece-charts/latest/day_piece_charts/select/struct.Guides.html).
Attach an interaction with
[`interact()`](https://docs.rs/day-piece-charts/latest/day_piece_charts/struct.Chart.html#method.interact).
Read the resulting selection to show details in ordinary Day controls.

A canvas chart is not automatically an accessible data table. Give the chart an appropriate
[accessibility label](/docs/reference/accessibility), provide a textual summary or table, and
use labels or shapes as well as color to distinguish important values. Keep important tasks
available without precise pointer selection.

## Export

[`ChartExports`](https://docs.rs/day-piece-charts/latest/day_piece_charts/struct.ChartExports.html)
collects recorded chart drawings for export. File destinations and permission handling remain
app concerns; see [file dialogs](/docs/reference/files).

Charts use the selected toolkit's canvas, font, and image facilities. Text metrics and input
behavior can differ between platforms. Test dense labels, resizing, and touch targets in the
platforms your app supports. See [canvas](/docs/reference/canvas),
[fonts](/docs/reference/fonts), and [colors](/docs/reference/color).

## Examples

Embed the chart examples as a playground, or select individual pages for a data-visualization
demo. `day-piece-charts-gallery` supplies the pages used by the chart demo, including a chart
construction walkthrough, mark combinations, and interactive examples.

### Add the example library

```sh
cargo add day-piece-charts-gallery
```

[Crate](https://crates.io/crates/day-piece-charts-gallery) ·
[API reference](https://docs.rs/day-piece-charts-gallery/latest/day_piece_charts_gallery/) ·
[Source](https://github.com/daybrite/day-piece-charts/tree/main/gallery).
Build with the [Day CLI](/docs/reference/cli) so the library's resources are included.

### Show the gallery

The selected-page signal belongs to your app. Changing it selects another example:

```rust
use day::prelude::*;
use day_piece_charts_gallery::{ChartPage, gallery};

pub fn chart_examples() -> impl Piece {
    gallery(Signal::new(ChartPage::Scatter))
}
```

[`gallery()`](https://docs.rs/day-piece-charts-gallery/latest/day_piece_charts_gallery/fn.gallery.html)
provides a picker and the selected page. For custom navigation, use
[`pages()`](https://docs.rs/day-piece-charts-gallery/latest/day_piece_charts_gallery/fn.pages.html)
and the returned [`Entry`](https://docs.rs/day-piece-charts-gallery/latest/day_piece_charts_gallery/struct.Entry.html)
values, or request one [`page()`](https://docs.rs/day-piece-charts-gallery/latest/day_piece_charts_gallery/fn.page.html)
by its [`ChartPage`](https://docs.rs/day-piece-charts-gallery/latest/day_piece_charts_gallery/enum.ChartPage.html).
The library's localized titles follow the host locale. Keep the app's surrounding navigation
and commands localized too.

The example pages include export controls. Apps supplying a menu can use
[`export_menu_items()`](https://docs.rs/day-piece-charts-gallery/latest/day_piece_charts_gallery/fn.export_menu_items.html)
with the current page. Saving still depends on the platform's file-dialog and export support.

### Scope and platform behavior

These are sample pages with their own data and UI; they are not a chart editor or a data-import
API. The same canvas and font limitations apply to both libraries. The mock toolkit does not render
a visible gallery.

Explore the [browser demo](https://daybrite.github.io/day-piece-charts/webapp/) or compose your
own navigation using [navigation](/docs/reference/navigation). For another reusable visual
playground, see [Lottie examples](/docs/reference/lottie#examples).
