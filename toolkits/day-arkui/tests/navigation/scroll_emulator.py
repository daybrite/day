"""Scroll geometry regression; launch the fixture with --env PROBE_SCROLL=1.

Checks native viewport/content bounds rather than logical Day node visibility.
"""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess
import time

parser = argparse.ArgumentParser()
parser.add_argument("--project", type=Path, required=True)
parser.add_argument("--day", type=Path, required=True)
args = parser.parse_args()
project = args.project.resolve()
day = args.day.resolve()
out = project / "build/scroll-validation"
out.mkdir(parents=True, exist_ok=True)
target = os.environ.get("DAY_OHOS_TARGET", "127.0.0.1:55555")
results = []


def hdc(*command):
    return subprocess.run(
        ["hdc", "-t", target, *map(str, command)],
        check=True, text=True, capture_output=True,
    ).stdout


def drive(*steps):
    response = subprocess.run(
        [str(day), "drive", "--project", str(project), "-p", "harmony-arkui",
         "--steps-json", json.dumps([*steps, {"wait_idle": {}}])],
        check=True, text=True, capture_output=True,
    )
    assert json.loads(response.stdout)["failed"] == 0, response.stdout


def all_nodes(node):
    return [node] + [n for child in node.get("children", []) for n in all_nodes(child)]


def rect(node):
    a = node["attributes"]
    return list(map(int, re.findall(r"-?\d+", a.get("origBounds") or a["bounds"])))


def check(name):
    deadline = time.monotonic() + 10
    while True:
        hdc("shell", "uitest", "dumpLayout", "-p", "/data/local/tmp/scroll-check.json")
        hdc("file", "recv", "/data/local/tmp/scroll-check.json", out / f"{name}.json")
        nodes = all_nodes(json.loads((out / f"{name}.json").read_text()))
        scroll = next(n for n in nodes if n["attributes"].get("type") == "Scroll")
        bars = [n for n in nodes if n["attributes"].get("type") == "Row"
                and {"Library", "Catalogs", "Settings"}.issubset(
                    {c["attributes"].get("text") for c in all_nodes(n)})]
        bar = min(bars, key=lambda n: rect(n)[3] - rect(n)[1])
        end = next((n for n in nodes if n["attributes"].get("text") == "BOTTOM SENTINEL"), None)
        viewport = rect(scroll)
        content = rect(scroll["children"][0])
        sentinel = rect(end) if end else None
        if (sentinel and viewport[3] <= rect(bar)[1]
                and viewport[1] <= sentinel[1] < sentinel[3] <= viewport[3]):
            break
        if time.monotonic() >= deadline:
            raise AssertionError({"case": name, "viewport": viewport,
                                  "sentinel": sentinel, "tab_bar": rect(bar)})
    result = {"case": name, "viewport": viewport, "sentinel": sentinel,
              "content_height": content[3] - content[1]}
    results.append(result)
    print("PASS", result, flush=True)
    return result["content_height"]


def bottom():
    drive({"scroll_to": {"id": "probe-scroll", "edge": "bottom"}})


drive({"navigate": {"route": "settings"}})
bottom()
initial = check("initial")
drive({"tap": {"id": "resize-scroll-content"}})
bottom()
expanded = check("expanded")
assert expanded > initial + 200, (initial, expanded)
drive({"tap": {"id": "resize-scroll-content"}})
# Shrinking at the old bottom must clamp the offset without an explicit scroll.
shrunk = check("shrunk")
assert abs(shrunk - initial) <= 2, (initial, shrunk)
drive({"scroll_to": {"id": "probe-scroll", "edge": "top"}},
      {"scroll_to": {"id": "scroll-end"}})
check("reveal-last-control")
# Restore the top, then move by physical touch gestures, not script scrolling.
drive({"scroll_to": {"id": "probe-scroll", "edge": "top"}})
for _ in range(10):
    hdc("shell", "uitest", "uiInput", "swipe", 180, 590, 180, 220, 800)
check("touch-bottom")
# Destroy and rebuild the tabs while the overall window size stays unchanged.
drive({"tap": {"id": "mount-tabs"}}, {"tap": {"id": "mount-tabs"}},
      {"navigate": {"route": "settings"}})
bottom()
check("remounted")
(out / "result.json").write_text(json.dumps(results, indent=2) + "\n")
