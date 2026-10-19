# Native UIKit search drawer regression

Build and install the English iOS versions of Day-News and Day-Showcase with
`.search_initially_hidden(true)` on a phone and an iPad simulator. Tests use real native
swipes and taps: they check the initially collapsed drawer and compact title, pull-down
reveal, editing, cancellation, destination navigation and subsequent collapse. Screenshots
are retained in the result bundle. Day-News runs with a separate fixture library;
these tests never change the production library or credentials.

Before the full scrolling test, launch Day-News once with
`DAY_NEWS_DATA_DIR=/tmp/day-search-drawer-fixture` and `DAY_NEWS_SYNC_TEST=1`, then run
`python3 tests/ios-search/seed-news.py` to supply 40 offline feed rows. An empty fixture
can check initial hiding and reveal, but UIKit keeps revealed search visible if the list
has too little content to scroll. No artificial content offset overrides that behavior.

Generate the test project with `xcodegen generate --spec tests/ios-search/project.yml`,
then run its `SearchProbe` scheme with `xcodebuild test -parallel-testing-enabled NO`
and an explicit `-destination 'platform=iOS Simulator,id=…'`. Use
`-only-testing:SearchProbe/SearchTests/testNewsDrawer` or `testShowcaseDrawer` to test one app.
Run on iPhone 15 Pro / iOS 17 and iPad / iOS 17.5 or later. The iOS 17 custom page wrapper
occasionally omits the Back button from its accessibility snapshot; the phone test falls
back to tapping its observed native screen position, and verifies that the root returns.
No programmatic route pop substitutes for that tap.

When checking the default/opt-out presentation, remove `search_initially_hidden(true)`
(or set it to `false`) in the app and rebuild: existing large-title and
`search_hides_on_scroll` behavior must be retained. Desktop and Android search behavior
is unaffected by the new preference.
