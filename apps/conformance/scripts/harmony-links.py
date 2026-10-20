#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0

"""Native ArkUI link checks. Build conformance first; connect one hdc target.

Only fixture controls and assertions use Day's driver. Link activations are OS taps
inside native Text bounds; no synthetic LinkActivated or Tap event proves these checks.
"""
import argparse
import re
import runpy
from pathlib import Path

helpers = runpy.run_path(str(Path(__file__).with_name("harmony-dialogs.py")))


class LinkTests(helpers["DialogTests"]):
    def click(self, node, x=0.5, y=0.5):
        left, top, right, bottom = map(int, re.findall(r"-?\d+", node["bounds"]))
        self.input("click", round(left + x * (right - left)), round(top + y * (bottom - top)))

    def mixed(self, x):
        self.click(self.find("links::mixed", "id"), x)

    def result(self, count, target):
        self.answer(f"Clicks: {count}", "links::count")
        self.answer(target, "links::target")

    def tests(self):
        self.find("links::mixed", "id")  # Wait for startup before connecting to Day's driver.
        self.mixed(0.25)
        self.result(1, "#alpha")
        self.mixed(0.57)
        self.result(2, "https://example.invalid/β?x=1&y=2")
        self.mixed(0.90)
        self.result(2, "https://example.invalid/β?x=1&y=2")
        self.passed_case("mixed links preserve exact Unicode targets; plain text does not activate")

        wrapped = self.find("link link link link link link link link")
        self.click(wrapped, 0.08, 0.12)
        self.result(3, "#wrapped")
        self.click(self.find("link link link link link link link link"), 0.08, 0.88)
        self.result(4, "#wrapped")
        self.click(self.find("link link link link link link link link"), 0.94, 0.88)
        self.result(4, "#wrapped")
        self.passed_case("first and last wrapped lines activate; trailing whitespace does not")

        self.click(self.find("links::builder", "id"))
        self.result(5, "custom:東京?x=1&y=2")
        self.click(self.find("links::selectable", "id"))
        self.result(6, "#selectable")
        self.passed_case("TextBuilder links and selectable markdown labels activate")

        for cycle in range(3):
            self.control("links::retarget")
            self.mixed(0.25)
            self.result(7 + 2 * cycle, "#changed")
            self.control("links::restore")
            self.mixed(0.57)
            self.result(8 + 2 * cycle, "https://example.invalid/β?x=1&y=2")
        self.passed_case("repeated rebuilds use new targets exactly once, without stale handlers")

        self.control("links::plain")
        self.mixed(0.25)
        self.result(12, "https://example.invalid/β?x=1&y=2")
        self.control("links::restore")
        self.mixed(0.25)
        self.result(13, "#alpha")
        self.passed_case("link-to-plain-to-link transitions remove and restore click handlers")

        self.control("links::remove")
        self.mixed(0.57)
        self.result(14, "https://example.invalid/β?x=1&y=2")
        self.find("links::mixed", "id", missing=True)
        self.control("links::remove")
        self.control("links::mount")
        self.mixed(0.25)
        self.result(15, "#alpha")
        self.passed_case("a handler may remove its own label; remount has no stale span state")

        self.click(self.find("links::route_link", "id"))
        self.answer("Destination page", "links::destination")
        self.passed_case("standalone Link uses native span activation and default route navigation")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--day", default=str(helpers["ROOT"] / "target/debug/day"))
    parser.add_argument("--hdc", default="hdc")
    args = parser.parse_args()
    tests = LinkTests(args.day, args.hdc)
    tests.run(tests.day + ["launch", "-t", "harmony-arkui", "--skip-build",
                          "--env", "DAY_NATIVE_LINK_TEST=1", "--detach"], timeout=120)
    tests.tests()
    print(f"{tests.passed} native link checks passed")


if __name__ == "__main__":
    main()
