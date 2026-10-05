# Harmony navigation isolation regression

This fixture has two resident tabs with independent `nav_stack`s, plus a plain
Settings tab. Before the October 2026 fix, Library's root wore the title
**Catalogs**, pushing Library used Catalogs' global bookkeeping, and Settings left
Library highlighted. This is a toolkit composition defect, not a Stanza model
defect; apps with only one navigation host need not exhibit it.

Generate a throwaway app outside the Day workspace with the Day CLI, then replace
its generated `src/lib.rs` with `app.rs` from this directory:

```sh
day new app Harmony-Nav-Probe --toolkit harmony-arkui \
  --appid dev.daybrite.navprobe --no-github --no-website --no-input
cp /path/to/day/toolkits/day-arkui/tests/navigation/app.rs Harmony-Nav-Probe/src/lib.rs
day patch --project Harmony-Nav-Probe --local /path/to/day
day build --project Harmony-Nav-Probe -p harmony-arkui
day launch --project Harmony-Nav-Probe -p harmony-arkui --skip-build --detach
python3 /path/to/day/toolkits/day-arkui/tests/navigation/emulator.py \
  --project Harmony-Nav-Probe --day /path/to/day/target/debug/day
```

The emulator must be the 360×720 phone; `DAY_OHOS_TARGET` defaults to
`127.0.0.1:55555`. Run after launch completes, from a fresh app process. Mouse tab
coordinates target the bottom bar. The test polls native `uitest dumpLayout`
TitleBars, so retained but hidden Day nodes cannot falsely satisfy its checks.
It uses real mouse input for tab selection and system Back for pops. Results and
a Settings screenshot are saved under the generated app's `build/navigation-validation/`.

The test checks 65 native title states: five repeated cycles of independent
two-level history, tab switching and native Back; pushing a hidden sibling;
consuming and then allowing guarded Back; an immediate push/pop; and destroying
both hosts with live destinations and rebuilding them. The fixture's toolbar
commands also make incorrect cross-tab headers visible.

To check the no-navigation window toolbar, launch separately with
`--env PROBE_PLAIN=1`. Clicking the native plus button must change the label to
`WINDOW ACTION WORKED`. This checks that moving Navigation into its own component
does not remove toolbar support from plain single-page apps.

Local 2026-10-05 validation: all 65 native title checks passed on OpenHarmony
7.0.0.39 x86_64. Plain-window toolbar input passed separately. Stanza's full
store walkthrough also passed 141 steps and eight screenshots. These are local
emulator results; this fixture is not yet a remote CI job.

## Scroll viewport and content extents

Launch with `--env PROBE_SCROLL=1`, then run `scroll_emulator.py` with the same
`--project` and `--day` arguments. Its Settings tab contains long wrapping text,
optional dynamic content, and a final button. The driver checks the native scroll
viewport against the tab bar and verifies that the entire final button is visible.
It tests initial layout, growing/shrinking content, automatic offset clamping,
revealing the last control, physical swipes, and destroying/recreating the tabs.

Before the sizing fix, the viewport ended at y=673 and its final button at y=665,
under a tab bar starting at y=632. The fixed viewport ends at y=620 and the button
fits above it. Dynamic content grew from 1070 to 2792 pixels and shrank back while
remaining fully scrollable. The six native geometry checks passed locally.
