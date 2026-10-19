import XCTest

// Runs against installed English Day apps, using real UIKit gestures rather than route injection.
final class SearchTests: XCTestCase {
    func testShowcaseDrawer() {
        exerciseDrawer(bundle: "dev.daybrite.showcase", title: "Day Showcase", destination: "About")
    }

    func testNewsDrawer() {
        exerciseDrawer(bundle: "dev.daybrite.news", title: "Day News", destination: "All Articles")
    }

    private func exerciseDrawer(bundle: String, title: String, destination: String) {
        continueAfterFailure = false
        let app = XCUIApplication(bundleIdentifier: bundle)
        // Separate offline fixture (seed-news.py); no production credentials or subscriptions.
        app.launchEnvironment["DAY_NEWS_DATA_DIR"] = "/tmp/day-search-drawer-fixture"
        app.launchEnvironment["DAY_NEWS_SYNC_TEST"] = "1"
        app.launch()
        let rootBar = app.navigationBars[title].firstMatch
        if !rootBar.waitForExistence(timeout: 2) || !rootBar.isHittable {
            tapBack(app, title: title)
        }
        visible(rootBar)
        let search = app.searchFields.firstMatch
        XCTAssertFalse(search.isHittable, "Search must start collapsed")
        XCTAssertLessThan(rootBar.frame.height, 65, "Title must occupy the compact toolbar")
        let rootTitle = rootBar.staticTexts[title].firstMatch
        XCTAssertTrue(rootBar.frame.contains(rootTitle.frame))
        photograph(app, "Initially hidden search")

        pull(app, bar: rootBar, down: true)
        visible(search)
        photograph(app, "Pull down reveals search")
        search.tap()
        search.typeText(destination)
        XCTAssertEqual(search.value as? String, destination)
        // Reactive filtering/layout must not collapse an active search or lose its text.
        visible(search)
        let row = app.staticTexts[destination].firstMatch
        visible(row)
        photograph(app, "Editing revealed search")
        let cancel = app.buttons["Cancel"].firstMatch
        visible(cancel)
        cancel.tap()
        XCTAssertFalse(app.keyboards.firstMatch.exists)

        // A navigation round trip must keep the root reachable and search discoverable.
        visible(row)
        row.tap()
        if app.frame.width < 600 {
            tapBack(app, title: title)
            visible(rootBar)
        }
        pull(app, bar: rootBar, down: true)
        visible(search)
        XCTAssertTrue((search.value as? String ?? "").isEmpty || search.value as? String == search.placeholderValue)
        photograph(app, "Search after navigation")
        pull(app, bar: rootBar, down: false)
        let hidden = XCTNSPredicateExpectation(predicate: NSPredicate { _, _ in !search.isHittable }, object: nil)
        XCTAssertEqual(XCTWaiter.wait(for: [hidden], timeout: 5), .completed)
        photograph(app, "Search collapsed by scrolling")
    }

    private func tapBack(_ app: XCUIApplication, title: String) {
        let back = app.buttons.matching(NSPredicate(format: "label == 'Back' OR label == %@", title)).firstMatch
        if back.exists && back.isHittable {
            back.tap()
        } else {
            // The custom page wrapper on iOS 17 sometimes omits its navigation bar
            // from the AX snapshot. Still exercise a real tap at its native Back
            // position on the test phone, never a programmatic route pop.
            app.coordinate(withNormalizedOffset: CGVector(dx: 0.06, dy: 0.085)).tap()
        }
    }

    private func pull(_ app: XCUIApplication, bar: XCUIElement, down: Bool) {
        // Stay within the sidebar on iPad; do not scroll the adjacent article pane.
        let x = bar.frame.midX
        let y = bar.frame.maxY + 40
        let start = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: x, dy: down ? y : y + 120))
        let end = app.coordinate(withNormalizedOffset: .zero).withOffset(CGVector(dx: x, dy: down ? y + 120 : y))
        start.press(forDuration: 0.05, thenDragTo: end)
    }

    private func visible(_ element: XCUIElement) {
        XCTAssertTrue(element.waitForExistence(timeout: 8))
        XCTAssertTrue(element.isHittable)
    }

    private func photograph(_ app: XCUIApplication, _ name: String) {
        let attachment = XCTAttachment(screenshot: app.screenshot())
        attachment.name = name
        attachment.lifetime = .keepAlways
        add(attachment)
    }
}
