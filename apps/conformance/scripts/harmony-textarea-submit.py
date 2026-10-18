#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0

"""Native TextArea submission checks. Build conformance first; connect one hdc target.

Enter, Shift+Enter, clipboard paste and the keyboard action use OS input, never Day's
synthetic submit operation. The fixture counts callbacks and records their bound text.
"""
import argparse
import json
import runpy
from pathlib import Path

helpers = runpy.run_path(str(Path(__file__).with_name("harmony-dialogs.py")))


class SubmitTests(helpers["DialogTests"]):
    def area(self):
        return self.find("area", "id")

    def focus(self):
        self.input("click", *self.center(self.area()))

    def text(self, text):
        self.answer(text, "area")
        self.answer("Model: " + json.dumps(text, ensure_ascii=False), "model")
        native = self.area()
        assert native["text"] == text, native

    def submitted(self, count, text):
        self.answer(f"Submits: {count}", "submits")
        self.answer("Sent: " + json.dumps(text, ensure_ascii=False), "sent")

    def replace(self, text):
        self.focus()
        self.input("keyEvent", 2072, 2017)  # Ctrl+A
        self.input("text", text)

    def tests(self):
        self.area()  # Wait for native startup before using the engine connection.
        self.replace("Fresh")
        self.input("keyEvent", 2054)  # Hardware Enter
        self.submitted(1, "Fresh")
        self.text("Fresh")
        self.passed_case("hardware Enter submits exactly once with current text and no newline")

        self.focus()
        self.input("keyEvent", 2047, 2054)  # Shift+Enter
        self.text("Fresh\n")
        self.submitted(1, "Fresh")
        self.input("keyEvent", 2072, 2042)  # Ctrl+Z: the newline is one native undo step.
        self.text("Fresh")
        self.input("keyEvent", 2072, 2041)  # Ctrl+Y
        self.text("Fresh\n")
        self.input("text", "next")
        self.input("keyEvent", 2054)
        self.submitted(2, "Fresh\nnext")
        self.text("Fresh\nnext")
        self.passed_case("Shift+Enter inserts a newline without submission and preserves native undo/redo")

        self.focus()
        self.input("click", *self.center(self.find("Send")))  # Actual soft-keyboard button.
        self.submitted(3, "Fresh\nnext")
        self.text("Fresh\nnext")
        self.passed_case("the soft-keyboard Send action submits exactly once")

        self.control("seed")
        self.replace("Old")
        self.input("keyEvent", 2072, 2017)
        self.input("keyEvent", 2072, 2038)  # Ctrl+V
        self.text("first\nsecond")
        self.submitted(3, "Fresh\nnext")
        self.input("keyEvent", 2054)
        self.submitted(4, "first\nsecond")
        self.passed_case("multiline paste inserts text without triggering submission")

        self.control("unicode")
        self.focus()
        self.input("keyEvent", 2082)  # End
        self.input("keyEvent", 2014)  # Left, before B
        self.input("keyEvent", 2047, 2054)
        self.text("A😀東京\nB")
        self.input("text", "X")
        self.text("A😀東京\nXB")
        self.submitted(4, "first\nsecond")
        self.input("keyEvent", 2072, 2017)
        self.input("keyEvent", 2047, 2054)
        self.text("\n")
        self.passed_case("Shift+Enter respects Unicode text, retains the caret and replaces selections")

        self.control("editable")  # Switch read-only on, ending the editing session.
        self.focus()
        self.input("keyEvent", 2054)
        self.input("keyEvent", 2047, 2054)
        self.submitted(4, "first\nsecond")
        self.text("\n")
        self.control("editable")
        self.passed_case("read-only composers reject Enter and Shift+Enter")

        self.control("clear")
        self.replace("ClearMe")
        self.input("keyEvent", 2054)
        self.submitted(5, "ClearMe")
        self.text("")
        self.input("keyEvent", 2054)
        self.submitted(6, "")
        self.text("")
        self.passed_case("callbacks may clear the editor; empty submission is delivered once")

        self.control("remove")
        self.replace("RemoveMe")
        self.input("keyEvent", 2054)
        self.submitted(7, "RemoveMe")
        self.find("area", "id", missing=True)
        self.control("remove")
        self.control("clear")
        self.control("mount")
        self.replace("Again")
        self.input("keyEvent", 2054)
        self.submitted(8, "Again")
        self.text("Again")
        self.passed_case("callbacks may remove the composer; remount delivers to the new node")

        self.control("mode")
        self.replace("Plain")
        self.input("keyEvent", 2054)
        self.text("Plain\n")
        self.submitted(8, "Again")
        self.passed_case("ordinary TextAreas retain native multiline Enter behavior")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--day", default=str(helpers["ROOT"] / "target/debug/day"))
    parser.add_argument("--hdc", default="hdc")
    args = parser.parse_args()
    tests = SubmitTests(args.day, args.hdc)
    tests.run(tests.day + ["launch", "-t", "harmony-arkui", "--skip-build",
                          "--env", "DAY_NATIVE_TEXTAREA_SUBMIT_TEST=1", "--detach"], timeout=120)
    tests.tests()
    print(f"{tests.passed} native TextArea submission checks passed")


if __name__ == "__main__":
    main()
