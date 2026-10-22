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
              {"op": "assert_text", "id": "native_canvas::drags", "text": f"Drags: {drags}"},
              {"op": "assert_text", "id": "native_canvas::began", "text": f"Began: {drags}"})

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

    def mouse(flags, dx=0, dy=0):
        event = INPUT(0, INPUTUNION(mi=MOUSEINPUT(dx, dy, 0, flags, 0, 0)))
        if user.SendInput(1, ctypes.byref(event), ctypes.sizeof(INPUT)) != 1:
            raise ctypes.WinError(ctypes.get_last_error())

    def move(x, y):
        left, top = user.GetSystemMetrics(76), user.GetSystemMetrics(77)
        width, height = user.GetSystemMetrics(78), user.GetSystemMetrics(79)
        mouse(0xC001, round((x - left) * 65535 / (width - 1)),
              round((y - top) * 65535 / (height - 1)))

    # The fixture places its 300x200 DIP canvas at the client origin. Resolve the
    # actual native window, then convert that point to physical screen coordinates.
    # This deliberately does not depend on automation peers to deliver mouse input.
    callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
    user.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
    user.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
    user.IsWindowVisible.argtypes = [wintypes.HWND]
    user.SetForegroundWindow.argtypes = [wintypes.HWND]
    user.ClientToScreen.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.POINT)]
    user.GetDpiForWindow.argtypes = [wintypes.HWND]
    user.GetDC.argtypes = [wintypes.HWND]
    user.GetDC.restype = wintypes.HDC
    user.ReleaseDC.argtypes = [wintypes.HWND, wintypes.HDC]
    gdi = ctypes.WinDLL("gdi32", use_last_error=True)
    gdi.GetPixel.argtypes = [wintypes.HDC, ctypes.c_int, ctypes.c_int]
    gdi.GetPixel.restype = wintypes.DWORD

    def native_window():
        pid = int(run(["powershell.exe", "-NoProfile", "-Command",
                       "(Get-Process -Name conformance).Id"]).strip())
        found = []

        @callback_type
        def visit(hwnd, _):
            owner = wintypes.DWORD()
            user.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
            if owner.value == pid and user.IsWindowVisible(hwnd):
                found.append(hwnd)
            return True

        user.EnumWindows(visit, 0)
        assert len(found) == 1, f"Expected one visible conformance window, found {len(found)}"
        return found[0]

    # The detached GUI may inherit stdout. A real log file avoids waiting for a pipe
    # to reach EOF while that GUI is deliberately still running.
    launch_log = APP / "build/day/windows-canvas-launch.log"
    with launch_log.open("w", encoding="utf-8") as output:
        subprocess.run(list(map(str, day + ["launch", "-t", "windows-winui", "--skip-build", "--detach",
                                            "--env", "DAY_NATIVE_CANVAS_TEST=1"])),
                       stdout=output, stderr=subprocess.STDOUT, check=True, timeout=60)
    dc = None
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
        hwnd = native_window()
        if not user.SetForegroundWindow(hwnd):
            raise AssertionError("Could not foreground the fixture; use an unlocked desktop")
        time.sleep(0.2)
        scale = user.GetDpiForWindow(hwnd) / 96
        point = wintypes.POINT(round(100 * scale), round(100 * scale))
        if not user.ClientToScreen(hwnd, ctypes.byref(point)):
            raise ctypes.WinError(ctypes.get_last_error())
        x, y = point.x, point.y
        colors = set()
        dc = user.GetDC(None)
        if not dc:
            raise ctypes.WinError(ctypes.get_last_error())
        for n in range(1, 4):
            move(x, y)
            mouse(0x0002)
            for _ in range(20):
                colors.add(gdi.GetPixel(dc, x + 20, y + 20))
                time.sleep(0.015)  # Sample actual Win2D frames while holding the press.
            mouse(0x0004)
            counts(n, 0)
        print("PASS: three native clicks survive continuous repaint", flush=True)
        drive({"op": "assert_focused", "id": "native_canvas::pad", "focused": True})
        move(x, y)
        mouse(0x0002)
        time.sleep(0.05)
        for n in range(1, 11):
            move(x + round(n * 8 * scale), y + round(n * 3 * scale))
            time.sleep(0.04)
        mouse(0x0004)
        counts(3, 1)
        drive({"op": "assert_text", "id": "native_canvas::changed", "text": "Changed: true"},
              {"op": "assert_text", "id": "native_canvas::delta_valid", "text": "Delta valid: true"})
        # COLORREF is 0x00BBGGRR; both alternating fixture colors must reach the screen.
        assert {0xcc8844, 0xcc8855} <= colors, f"Missing repainted canvas colors: {colors}"
        print("PASS: native drag completes across redraws without a release tap", flush=True)
        # Capture must survive leaving the canvas, and a drag back to its origin must
        # not become a click merely because press/release positions coincide.
        for drag_count, offsets in [(2, [80, 160, 240, 320]), (3, [40, 80, 40, 0])]:
            move(x, y)
            mouse(0x0002)
            time.sleep(0.05)
            for offset in offsets:
                move(x + round(offset * scale), y)
                time.sleep(0.08)
            mouse(0x0004)
            counts(3, drag_count)
            drive({"op": "assert_text", "id": "native_canvas::delta_valid", "text": "Delta valid: true"})
        move(x, y)
        mouse(0x0002)
        time.sleep(0.08)
        mouse(0x0004)
        counts(4, 3)
        print("PASS: capture outside the canvas, return-to-origin drag and subsequent click", flush=True)
    finally:
        if dc:
            user.ReleaseDC(None, dc)
        mouse(0x0004)
        run(day + ["stop", "-t", "windows-winui"])


if __name__ == "__main__":
    main()
