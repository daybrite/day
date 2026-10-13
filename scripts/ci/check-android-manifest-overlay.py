#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Exercise AGP manifest merging in an isolated copy of a prepared Day Android host.

Usage: python3 scripts/ci/check-android-manifest-overlay.py /path/to/prepared-app
Requires Gradle, a supported JDK, and the Android SDK, as does `day build`.
"""

from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import xml.etree.ElementTree as ET

ANDROID = "{http://schemas.android.com/apk/res/android}"
TASKS = [f":app:process{variant}MainManifest" for variant in ("Debug", "Release", "Qa")]


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def run(host, success=True):
    result = subprocess.run(
        ["gradle", "--console=plain", "--configuration-cache", *TASKS],
        cwd=host, text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
    )
    if (result.returncode == 0) != success:
        raise AssertionError(result.stdout)
    return result.stdout


def merged(host, variant):
    path = host / f"app/build/intermediates/merged_manifest/{variant}/process{variant.title()}MainManifest/AndroidManifest.xml"
    return ET.parse(path).getroot()


def check(prepared):
    with tempfile.TemporaryDirectory(prefix="day manifest overlay ") as scratch:
        root = Path(scratch)
        host = root / "build/android"
        shutil.copytree(prepared / "build/android", host,
                        ignore=shutil.ignore_patterns("build", ".gradle"))
        stage = root / "build/day/android"
        stage.mkdir(parents=True)
        source = prepared / "build/day/android"
        for name in ("day-app.properties", "day-pieces.json"):
            shutil.copy2(source / name, stage / name)
        (stage / "gradle-plugin").symlink_to(source / "gradle-plugin", target_is_directory=True)
        with (host / "app/build.gradle.kts").open("a") as f:
            f.write('\nandroid { buildTypes { create("qa") { initWith(getByName("debug")) } } }\n')
        # Synthetic dependency contribution verifies precedence without changing the source app.
        write(stage / "day-pieces-manifest.xml", '''<manifest xmlns:android="http://schemas.android.com/apk/res/android">
    <application><meta-data android:name="dev.day.test.OVERLAY" android:value="piece" /></application>
</manifest>''')
        overlay = root / "platform/android/AndroidManifest.xml"
        run(host)
        for variant in ("debug", "release", "qa"):
            assert merged(host, variant).find("application").get(ANDROID + "supportsRtl") == "true"
        print("PASS: absent overlay uses defaults in debug, release and custom build type", flush=True)

        content = '''<manifest xmlns:android="http://schemas.android.com/apk/res/android"
    xmlns:tools="http://schemas.android.com/tools">
    <uses-permission android:name="android.permission.INTERNET" tools:node="remove" />
    <uses-feature android:name="android.hardware.camera.any" android:required="false" />
    <application android:supportsRtl="false" tools:replace="android:supportsRtl">
        <meta-data android:name="dev.day.test.OVERLAY" android:value="app" tools:replace="android:value" />
    </application>
</manifest>'''
        write(overlay, content)
        run(host)
        for variant in ("debug", "release", "qa"):
            manifest = merged(host, variant)
            app = manifest.find("application")
            assert app.get(ANDROID + "supportsRtl") == "false"
            assert any(m.get(ANDROID + "name") == "dev.day.test.OVERLAY" and
                       m.get(ANDROID + "value") == "app" for m in app.findall("meta-data"))
            assert not any(p.get(ANDROID + "name") == "android.permission.INTERNET"
                           for p in manifest.findall("uses-permission"))
            assert any(f.get(ANDROID + "name") == "android.hardware.camera.any"
                       for f in manifest.findall("uses-feature"))
            assert any(a.get(ANDROID + "name") == "dev.daybrite.day.bridge.DayActivity"
                       for a in app.findall("activity"))
        assert overlay.read_text() == content
        assert "Reusing configuration cache" in run(host)
        print("PASS: app overlay wins over host and pieces; tools rules and configuration cache work", flush=True)

        write(overlay, content.replace(' tools:replace="android:supportsRtl"', ""))
        output = run(host, success=False)
        assert "supportsRtl" in output and "tools:replace" in output
        assert "platform/android/AndroidManifest.xml" in output
        write(overlay, "<manifest><application></manifest>")
        output = run(host, success=False)
        assert "AndroidManifest.xml" in output
        print("PASS: conflicts and malformed XML fail with source diagnostics", flush=True)

        overlay.unlink()
        run(host)
        for variant in ("debug", "release", "qa"):
            manifest = merged(host, variant)
            assert manifest.find("application").get(ANDROID + "supportsRtl") == "true"
            assert any(p.get(ANDROID + "name") == "android.permission.INTERNET"
                       for p in manifest.findall("uses-permission"))
        print("PASS: deleting the overlay restores defaults without stale declarations", flush=True)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        sys.exit(__doc__)
    check(Path(sys.argv[1]).resolve())
