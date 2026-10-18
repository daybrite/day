#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0

"""Native ArkUI dialog checks. Build the conformance app first; connect one hdc target.

Day drives fixture buttons and assertions. Dialog choices, text editing and Back travel
through the OS input pipeline. Explicit respond steps test programmatic dismissal only.
"""
import argparse
import json
import re
import subprocess
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]


class DialogTests:
    def __init__(self, day, hdc):
        self.day = [day, "--project", str(ROOT / "apps/conformance")]
        self.hdc = hdc
        self.passed = 0

    @staticmethod
    def run(args, timeout=60):
        p = subprocess.run(args, text=True, capture_output=True, timeout=timeout)
        if p.returncode:
            raise AssertionError(f"{' '.join(args)}\n{p.stdout}\n{p.stderr}")
        return p.stdout

    def drive(self, *steps):
        self.run(self.day + ["drive", "-t", "harmony-arkui", "--steps-json", json.dumps(steps)])

    def control(self, name):
        self.drive({"op": "tap", "id": name})

    def answer(self, text, id="answer"):
        self.drive({"op": "assert_text", "id": id, "text": text})

    def layout(self):
        path = "/data/local/tmp/day-native-dialogs.json"
        self.run([self.hdc, "shell", "uitest", "dumpLayout", "-p", path])
        root = json.loads(self.run([self.hdc, "shell", "cat", path]))
        nodes = []

        def visit(node):
            nodes.append(node.get("attributes", {}))
            for child in node.get("children", []):
                visit(child)

        visit(root)
        return nodes

    def find(self, value, attribute="text", missing=False):
        deadline = time.monotonic() + 15
        while True:
            found = next((n for n in self.layout() if n.get(attribute) == value), None)
            if (found is None) == missing:
                return found
            if time.monotonic() >= deadline:
                raise AssertionError(f"Expected native {attribute}={value!r}, missing={missing}")
            time.sleep(0.1)

    @staticmethod
    def center(node):
        left, top, right, bottom = map(int, re.findall(r"-?\d+", node["bounds"]))
        return round((left + right) / 2), round((top + bottom) / 2)

    def input(self, *args):
        self.run([self.hdc, "shell", "uitest", "uiInput", *map(str, args)])

    def button(self, index):
        self.input("click", *self.center(self.find(f"day.dialog.button.{index}", "id")))

    def closed(self):
        self.find("day.dialog.surface", "id", missing=True)
        self.drive({"op": "assert_not_presented"})

    def passed_case(self, name):
        self.passed += 1
        print(f"PASS {name}", flush=True)

    def edit(self, text):
        field = self.find("day.dialog.input", "id")
        self.input("click", *self.center(field))
        self.input("keyEvent", 2072, 2017)  # Ctrl+A (SDK oh_key_code.h)
        self.input("keyEvent", 2055)  # Delete selection
        if text:
            self.input("text", text)
        assert self.find("day.dialog.input", "id")["text"] == text

    def tests(self):
        self.find("Startup confirmation")
        self.button(1)
        self.answer("startup true")
        self.closed()
        self.passed_case("presentation from the startup task")

        self.control("confirm")
        self.find('Delete "東京"?')
        self.find("This cannot be undone.\nKeep a backup.")
        self.button(1)
        self.answer("confirm true")
        self.closed()
        self.control("confirm")
        self.button(0)
        self.answer("confirm false")
        self.closed()
        self.passed_case("confirmation choice/cancel and lossless title/message transport")

        self.control("confirm")
        self.find('Delete "東京"?')
        self.input("keyEvent", "Back")
        self.answer("confirm false")
        self.closed()
        self.control("confirm")
        self.find('Delete "東京"?')
        self.input("click", 5, 80)  # Modal mask inside the app, outside the centered dialog.
        self.answer("confirm false")
        self.closed()
        self.passed_case("Back and outside-tap cancellation")

        self.control("prompt")
        assert self.find("day.dialog.input", "id")["text"] == "Ada"
        self.edit("Grace")
        self.button(1)
        self.answer('prompt Some("Grace")')
        self.closed()
        self.passed_case("prompt initial value, native editing and Save")

        self.control("prompt")
        self.edit("")
        self.input("keyEvent", 2054)  # Enter / IME submit, preserving Some("").
        self.answer('prompt Some("")')
        self.closed()
        self.control("prompt")
        self.button(0)
        self.answer("prompt None")
        self.closed()
        self.passed_case("empty prompt submission is distinct from cancellation")

        self.control("sheet")
        self.find("Choose an action")
        surface = self.find("day.dialog.surface", "id")
        left, top, right, bottom = map(int, re.findall(r"-?\d+", surface["bounds"]))
        page = self.find("day.capture.0", "id")
        page_bounds = tuple(map(int, re.findall(r"-?\d+", page["bounds"])))
        assert (top + bottom) / 2 > (page_bounds[1] + page_bounds[3]) / 2, surface
        assert 0 < page_bounds[3] - bottom < 32, (surface, page)
        # No part of the sheet may be clipped by the navigation bar.
        assert surface["bounds"] == surface["origBounds"], surface
        self.button(2)
        self.answer("sheet Some(30)")
        self.closed()
        self.control("sheet")
        self.button(1)
        self.answer("sheet None")
        self.closed()
        self.passed_case("four-button action sheet preserves destructive/cancel indices")

        self.control("pair")
        self.find("Queue A")
        self.find("Queue B", missing=True)
        self.button(1)
        self.answer("first true", "first")
        self.find("Queue B")
        self.input("keyEvent", "Back")
        self.answer("second false", "second")
        self.closed()
        self.passed_case("concurrent requests present sequentially and answer their own futures")

        self.control("pair")
        self.find("Queue A")
        # Day's pending request is the most recent: Queue B has not opened yet.
        self.drive({"op": "respond", "dismiss": True})
        self.answer("second false", "second")
        self.find("Queue A")
        self.button(1)
        self.answer("first true", "first")
        self.closed()
        self.passed_case("programmatic dismissal removes a queued request without closing another")

        self.drive({"op": "tap", "id": "confirm"}, {"op": "respond", "button": 1},
                   {"op": "tap", "id": "prompt"})
        self.find("Your name")
        self.button(1)
        self.answer('prompt Some("Ada")')
        self.closed()
        self.passed_case("close during opening does not dismiss the following dialog")

        self.control("chain")
        self.find("Continue?")
        self.button(1)
        self.find("Next dialog")
        self.button(1)
        self.answer('chain Some("ready")')
        self.closed()
        self.passed_case("an answered future can immediately present another dialog")

        self.control("empty")
        self.find("No choices")
        self.input("keyEvent", "Back")
        self.answer("empty dismissed")
        self.closed()
        self.passed_case("buttonless alert remains dismissible")

        self.control("light")
        self.control("confirm")
        light = self.find("day.dialog.surface", "id")["backgroundColor"]
        self.button(0)
        self.closed()
        self.control("dark")
        try:
            self.control("confirm")
            dark = self.find("day.dialog.surface", "id")["backgroundColor"]
            assert light != dark, (light, dark)
            self.button(0)
            self.closed()
        finally:
            self.control("system")
        self.passed_case("dialog surface follows the application's light/dark appearance")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--day", default=str(ROOT / "target/debug/day"))
    parser.add_argument("--hdc", default="hdc")
    args = parser.parse_args()
    tests = DialogTests(args.day, args.hdc)
    tests.run(tests.day + ["launch", "-t", "harmony-arkui", "--skip-build",
                          "--env", "DAY_NATIVE_DIALOG_TEST=1", "--detach"], timeout=120)
    tests.tests()
    print(f"{tests.passed} native dialog checks passed")


if __name__ == "__main__":
    main()
