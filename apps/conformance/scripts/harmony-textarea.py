#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0

"""Native TextArea regression checks. Build conformance first; connect one hdc target.

Only fixture controls/assertions use Day's driver. Editing, clipboard shortcuts and touch
selection go through OS input. Both the actual native text and bound model are checked.
"""
import argparse
import re
import runpy
from pathlib import Path

# Share the native layout/input helpers with the dialog driver.
helpers = runpy.run_path(str(Path(__file__).with_name("harmony-dialogs.py")))


class TextAreaTests(helpers["DialogTests"]):
    def area(self):
        return self.find("textarea::area", "id")

    def focus(self):
        self.input("click", *self.center(self.area()))

    def shortcut(self, key):
        self.input("keyEvent", 2072, key)

    def text(self, expected):
        self.answer(expected, "textarea::area")
        self.answer("Model: " + expected, "textarea::model")
        assert self.area()["text"] == expected, self.area()

    def clipboard(self, expected):
        # Use native Paste, which carries the OS paste authorization; a direct clipboard
        # API read is not equivalent and can be denied even in a foreground app.
        self.control("textarea::clear_clipboard")
        self.input("click", *self.center(self.find("textarea::clipboard", "id")))
        self.shortcut(2038)
        self.answer(expected, "textarea::clipboard")
        self.input("keyEvent", "Back")

    def tests(self):
        self.area()  # Native mounting signals startup completion before the first drive.
        self.text("Alpha Beta")
        self.focus()
        self.input("keyEvent", 2042)  # Hardware Z, independently of IME text injection.
        self.input("text", "Z")
        self.input("keyEvent", 2055)  # Backspace
        self.input("keyEvent", 2054)  # Enter
        self.text("Alpha Beta")
        self.passed_case("initial read-only blocks native typing, deletion and newline")

        self.control("textarea::seed")
        self.focus()
        self.shortcut(2017)  # A
        self.shortcut(2019)  # C
        self.clipboard("Alpha Beta")
        self.focus()
        self.shortcut(2017)
        self.shortcut(2040)  # X
        self.text("Alpha Beta")
        self.control("textarea::seed")
        self.focus()
        self.shortcut(2038)  # V
        self.text("Alpha Beta")
        self.passed_case("read-only allows copy but rejects cut and paste")

        self.control("textarea::replace")
        self.text("Gamma Delta")
        self.focus()
        self.input("text", "Z")
        self.text("Gamma Delta")
        self.passed_case("programmatic writes update the read-only value held by the gate")

        self.control("textarea::edit")
        self.focus()
        self.shortcut(2017)
        self.input("text", "Typed")
        self.text("Typed")
        self.input("keyEvent", "Back")  # Reveal the fixture's lower controls.
        self.control("textarea::seed")
        self.focus()
        self.shortcut(2017)
        self.shortcut(2038)
        self.text("PasteMarker")
        self.control("textarea::readonly")
        self.focus()
        self.input("text", "Z")
        self.text("PasteMarker")
        self.passed_case("reactive editability restores typing/paste and can lock again")

        self.control("textarea::reset")
        self.control("textarea::seed")
        self.focus()
        self.shortcut(2017)  # Existing selection must also collapse on the patch.
        self.control("textarea::noselect")
        self.shortcut(2019)
        self.clipboard("PasteMarker")
        self.focus()
        self.shortcut(2017)
        self.shortcut(2019)
        self.clipboard("PasteMarker")
        self.passed_case("disabling selection collapses an existing selection and blocks Select All/Copy")

        # Long press on the text itself, not the empty center of the three-line editor.
        area = self.area()
        x, y, _, _ = map(int, re.findall(r"-?\d+", area["bounds"]))
        self.input("longClick", x + 30, y + 18)
        self.find("Copy", missing=True)
        self.shortcut(2019)
        self.clipboard("PasteMarker")
        self.text("Alpha Beta")
        self.passed_case("unselectable long press exposes no copy menu or copied text")

        self.control("textarea::edit")
        self.focus()
        self.shortcut(2017)
        self.input("text", "Z")
        actual = self.area()["text"]
        assert actual != "Z" and actual.replace("Z", "") == "Alpha Beta", actual
        self.text(actual)
        # Shift+Left must not leave a range that the next keystroke replaces.
        self.input("keyEvent", 2047, 2014)
        self.input("text", "Q")
        after = self.area()["text"]
        assert len(after) == len(actual) + 1 and after.replace("Q", "") == actual, after
        self.text(after)
        self.passed_case("selection-disabled editor still types, without replacing Ctrl+A/Shift+Arrow ranges")

        self.control("textarea::select")
        self.focus()
        self.shortcut(2017)
        self.input("text", "Restored")
        self.text("Restored")
        self.passed_case("re-enabling selection restores native Select All and replacement")

        self.control("textarea::readonly")
        self.control("textarea::noselect")
        self.control("textarea::mount")
        self.control("textarea::mount")
        self.text("Restored")
        self.focus()
        self.input("text", "Z")
        self.text("Restored")
        self.control("textarea::edit")
        self.control("textarea::select")
        self.focus()
        self.shortcut(2017)
        self.input("text", "Remounted")
        self.text("Remounted")
        self.passed_case("remount applies initial flags and releases stale editor state")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--day", default=str(helpers["ROOT"] / "target/debug/day"))
    parser.add_argument("--hdc", default="hdc")
    args = parser.parse_args()
    tests = TextAreaTests(args.day, args.hdc)
    tests.run(tests.day + ["launch", "-t", "harmony-arkui", "--skip-build",
                          "--env", "DAY_NATIVE_TEXTAREA_TEST=1", "--detach"], timeout=120)
    tests.tests()
    print(f"{tests.passed} native text-area checks passed")


if __name__ == "__main__":
    main()
