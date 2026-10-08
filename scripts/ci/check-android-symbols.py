#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Ensure the scaffold's release APK/AAB packages stripped, dynamically linkable libraries."""

import pathlib
import re
import subprocess
import sys
import tempfile
import zipfile


def check(package, ndk):
    tools = list(ndk.glob("toolchains/llvm/prebuilt/*/bin/llvm-readelf"))
    tools += list(ndk.glob("toolchains/llvm/prebuilt/*/bin/llvm-readelf.exe"))
    if len(tools) != 1:
        raise RuntimeError(f"expected one llvm-readelf under {ndk}, found {len(tools)}")
    with zipfile.ZipFile(package) as archive, tempfile.TemporaryDirectory() as scratch:
        libraries = [
            name for name in archive.namelist()
            if re.fullmatch(r"(?:base/)?lib/[^/]+/[^/]+\.so", name)
        ]
        if not libraries:
            raise RuntimeError(f"{package}: no packaged native libraries")
        for name in libraries:
            library = pathlib.Path(scratch) / "library.so"
            library.write_bytes(archive.read(name))
            sections = subprocess.check_output(
                [str(tools[0]), "--sections", "--wide", str(library)], text=True
            )
            if re.search(r"\]\s+\.symtab\s", sections):
                raise RuntimeError(f"{package}: {name} still contains .symtab; AGP did not strip it")
            if not re.search(r"\]\s+\.dynsym\s", sections):
                raise RuntimeError(f"{package}: {name} is missing its runtime .dynsym")
            print(f"Verified {package.name}: {name} is stripped and retains .dynsym")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        sys.exit("usage: check-android-symbols.py PACKAGE NDK_DIRECTORY")
    check(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]))
