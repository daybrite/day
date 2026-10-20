#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0

"""Native touch regression tests for ArkUI's existing LongPress toolkit hook.

Build the conformance app for harmony-arkui first. Requires one connected hdc target,
an unlocked screen, and the SDK's uinput/uitest commands. Day drives fixture controls and
assertions only; every tested gesture comes through the OS input pipeline.
"""

import argparse
import json
import re
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps/conformance"


class Harness:
    def __init__(self, day, hdc):
        self.day = [day, "--project", str(APP)]
        self.hdc = hdc
        self.passed = []

    @staticmethod
    def run(args, timeout=60):
        result = subprocess.run(args, text=True, capture_output=True, timeout=timeout)
        if result.returncode:
            raise AssertionError(f"{' '.join(args)}\n{result.stdout}\n{result.stderr}")
        return result.stdout

    def drive(self, *steps):
        self.run(self.day + ["drive", "-t", "harmony-arkui", "--steps-json", json.dumps(steps)])

    def control(self, name):
        self.drive({"op": "tap", "id": name})

    def counts(self, holds, taps=0):
        self.drive(
            {"op": "assert_text", "id": "long_press::holds", "text": f"Holds: {holds}"},
            {"op": "assert_text", "id": "long_press::taps", "text": f"Taps: {taps}"},
        )

    def layout(self):
        path = "/data/local/tmp/day-native-long-press.json"
        self.run([self.hdc, "shell", "uitest", "dumpLayout", "-p", path])
        root = json.loads(self.run([self.hdc, "shell", "cat", path]))
        nodes = []

        def visit(node):
            nodes.append(node.get("attributes", {}))
            for child in node.get("children", []):
                visit(child)

        visit(root)
        return nodes

    def bounds(self, text, attribute="text"):
        for node in self.layout():
            if node.get(attribute) == text:
                bounds = tuple(map(int, re.findall(r"-?\d+", node["bounds"])))
                assert len(bounds) == 4, node
                return bounds
        raise AssertionError(f"Native {attribute} {text!r} missing from layout")

    def touch(self, *args):
        return self.run([self.hdc, "shell", "uinput", "-T", *map(str, args)])

    def press(self, x, y, duration):
        self.touch("-d", x, y, "-i", duration, "-u", x, y)

    def remount(self):
        self.control("long_press::mount")
        self.control("long_press::mount")
        self.drive({"op": "wait_for", "id": "long_press::pad"})

    def passed_case(self, name):
        self.passed.append(name)
        print(f"PASS {name}", flush=True)

    def tests(self):
        # Detached launch returns before the engine finishes replacing the previous
        # process's forwarded socket. Retry only this read-only startup check.
        deadline = time.monotonic() + 20
        while True:
            try:
                self.counts(0)
                break
            except AssertionError:
                if time.monotonic() >= deadline:
                    raise
                time.sleep(0.25)
        left, top, right, bottom = self.bounds("Hold here")
        x, y = round(left + (right - left) / 4), round((top + bottom) / 2)

        self.press(x, y, 100)
        self.counts(0, 1)
        self.passed_case("short press remains a tap")

        self.press(x, y, 1500)
        self.counts(1, 1)
        time.sleep(0.6)
        self.counts(1, 1)
        self.passed_case("hold fires once, without a release tap or duplicate registration")

        point = next(n["text"] for n in self.layout() if n.get("text", "").startswith("Point:"))
        actual = tuple(map(float, point.removeprefix("Point: ").split(",")))
        # The native label's frame is 220 x 100 Day points. Read its painted bounds so
        # this checks local origin AND px-to-vp conversion at the device's actual density.
        expected = ((x - left) * 220 / (right - left), (y - top) * 100 / (bottom - top))
        assert all(abs(a - b) < 2 for a, b in zip(actual, expected)), (actual, expected)
        self.passed_case("location is local to the native target, in Day points")

        self.control("long_press::reset")
        content_before = self.bounds("Scroll content")
        self.touch("-m", x, bottom - 10, x, top - 40, "-k", 800, 200)
        self.counts(0)
        content_after = self.bounds("Scroll content")
        assert content_after[1] < content_before[1], "The enclosing native scroll did not move"
        self.passed_case("movement cancels recognition and permits native scrolling")

        self.remount()
        left, top, right, bottom = self.bounds("Hold here")
        x, y = round((left + right) / 2), round((top + bottom) / 2)
        # Separate down/up commands keep a touch active while the fixture disposes the
        # native node. Always release, even if the assertion/control fails.
        self.touch("-d", x, y)
        try:
            self.control("long_press::mount")
            self.drive({"op": "assert_missing", "id": "long_press::pad"})
            time.sleep(0.8)
        finally:
            self.touch("-u", x, y)
        self.counts(0)
        self.control("long_press::mount")
        self.press(x, y, 800)
        self.counts(1)
        self.passed_case("pending hold is cancelled on disposal; remount works")

        for i in range(3):
            self.remount()
            self.press(x, y, 800)
            self.counts(i + 2)
        self.passed_case("repeated disposal/remount leaves one live recognizer")

        self.control("long_press::reset")
        self.control("long_press::remove_on_hold")
        self.press(x, y, 800)
        self.counts(1)
        self.drive({"op": "assert_missing", "id": "long_press::pad"})
        self.control("long_press::mount")
        self.press(x, y, 800)
        self.counts(2)
        self.passed_case("handler can remove its own target during acceptance")

        self.control("long_press::reset")
        self.control("long_press::mode")
        a = self.bounds("Alpha")
        c = self.bounds("Gamma")
        x, y = round((a[0] + a[2]) / 2), round((a[1] + a[3]) / 2)
        end_y = round((c[1] + c[3]) / 2)
        self.touch("-g", x, y, x, end_y, 800, 1800)
        self.drive({"op": "assert_text", "id": "long_press::order", "text": "Order: Alpha,Beta,Gamma,Delta"})
        self.counts(1)
        self.passed_case("child long press takes precedence in its own hit region")

        # ArkUI gives a child's explicit recognizer precedence over ancestor dragging.
        # Keep that native policy: the rest of the row must still start a native drag.
        # Do not use parallel priority, which would trigger both actions for one hold.
        self.control("long_press::reset")
        row = self.bounds("List", attribute="type")
        x = round((a[2] + row[2]) / 2)
        self.touch("-g", x, y, x, end_y, 800, 1800)
        self.drive({"op": "assert_text", "id": "long_press::order", "text": "Order: Beta,Gamma,Alpha,Delta"})
        self.counts(0)
        self.passed_case("native list reordering still works outside the child hit region")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--day", default=str(ROOT / "target/debug/day"))
    parser.add_argument("--hdc", default="hdc")
    parser.add_argument("--no-launch", action="store_true", help="Use an already running, fresh fixture")
    args = parser.parse_args()
    harness = Harness(args.day, args.hdc)
    if not args.no_launch:
        harness.run(harness.day + [
            "launch", "-t", "harmony-arkui", "--skip-build",
            "--env", "DAY_NATIVE_LONG_PRESS_TEST=1", "--detach",
        ], timeout=120)
    harness.tests()
    print(f"{len(harness.passed)} native-input checks passed")


if __name__ == "__main__":
    main()
