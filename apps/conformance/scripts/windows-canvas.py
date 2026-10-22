#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Real Windows mouse input across continuously repainted Win2D frames.

Build conformance for windows-winui first. Unlike dayscript's synthetic gestures,
this exercises XAML hit testing, pointer capture and the native tap/drag recognizers.
Requires an unlocked interactive Windows desktop. No third-party Python packages.
"""
import argparse
import ctypes
from ctypes import wintypes
import json
from pathlib import Path
import subprocess
import time

ROOT = Path(__file__).resolve().parents[3]
APP = ROOT / "apps/conformance"


def run(args):
    result = subprocess.run(list(map(str, args)), capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=90)
    if result.returncode:
        raise AssertionError(result.stdout + result.stderr)
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--day", type=Path, default=ROOT / "target/debug/day.exe")
    args = parser.parse_args()
    day = [args.day, "--project", APP]

    def drive(*steps):
        return run(day + ["drive", "-t", "windows-winui", "--steps-json", json.dumps(steps)])

    def counts(taps, drags):
        drive({"op": "assert_text", "id": "native_canvas::taps", "text": f"Taps: {taps}"},
              {"op": "assert_text", "id": "native_canvas::drags", "text": f"Drags: {drags}"})

    class MOUSEINPUT(ctypes.Structure):
        _fields_ = [("dx", wintypes.LONG), ("dy", wintypes.LONG), ("mouseData", wintypes.DWORD),
                    ("dwFlags", wintypes.DWORD), ("time", wintypes.DWORD), ("dwExtraInfo", ctypes.c_size_t)]

    class INPUTUNION(ctypes.Union):
        _fields_ = [("mi", MOUSEINPUT)]

    class INPUT(ctypes.Structure):
        _fields_ = [("type", wintypes.DWORD), ("value", INPUTUNION)]

    user = ctypes.WinDLL("user32", use_last_error=True)
    user.SendInput.argtypes = [wintypes.UINT, ctypes.POINTER(INPUT), ctypes.c_int]
    user.SendInput.restype = wintypes.UINT
    user.SetProcessDpiAwarenessContext(ctypes.c_void_p(-4))

    def mouse(flags):
        event = INPUT(0, INPUTUNION(mi=MOUSEINPUT(0, 0, 0, flags, 0, 0)))
        if user.SendInput(1, ctypes.byref(event), ctypes.sizeof(INPUT)) != 1:
            raise ctypes.WinError(ctypes.get_last_error())

    # UI Automation reports physical screen bounds; it does not invoke the tested control.
    bounds_script = r'''
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$condition = New-Object System.Windows.Automation.PropertyCondition([System.Windows.Automation.AutomationElement]::NameProperty, 'Native canvas input')
$element = [System.Windows.Automation.AutomationElement]::RootElement.FindFirst([System.Windows.Automation.TreeScope]::Descendants, $condition)
if ($null -eq $element) { throw 'Native canvas input was not found' }
$bounds = $element.Current.BoundingRectangle
@{x=$bounds.X; y=$bounds.Y; w=$bounds.Width; h=$bounds.Height} | ConvertTo-Json -Compress
'''
    run(day + ["launch", "-t", "windows-winui", "--skip-build", "--detach", "--env", "DAY_NATIVE_CANVAS_TEST=1"])
    try:
        deadline = time.monotonic() + 25
        while True:
            try:
                counts(0, 0)
                break
            except AssertionError:
                if time.monotonic() > deadline:
                    raise
                time.sleep(0.25)
        bounds = json.loads(run(["powershell.exe", "-NoProfile", "-Command", bounds_script]))
        x, y = round(bounds["x"] + bounds["w"] / 3), round(bounds["y"] + bounds["h"] / 2)
        for n in range(1, 4):
            user.SetCursorPos(x, y)
            mouse(0x0002)
            time.sleep(0.25)  # Several redraws occur while the native press is held.
            mouse(0x0004)
            counts(n, 0)
        print("PASS: three native clicks survive continuous repaint", flush=True)
        drive({"op": "assert_focused", "id": "native_canvas::pad", "focused": True})
        user.SetCursorPos(x, y)
        mouse(0x0002)
        for n in range(1, 11):
            user.SetCursorPos(x + n * 8, y + n * 3)
            time.sleep(0.04)
        mouse(0x0004)
        counts(3, 1)
        print("PASS: native drag completes across redraws without a release tap", flush=True)
    finally:
        mouse(0x0004)
        run(day + ["stop", "-t", "windows-winui"])


if __name__ == "__main__":
    main()
