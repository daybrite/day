#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Prove ID retention in native and final WASM artifacts, debug and stripped LTO release."""
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "crates/day-core/tests/fixtures/element-ids/Cargo.toml"
TARGET = ROOT / "target/element-id-retention"
ENV = {**os.environ, "CARGO_TARGET_DIR": str(TARGET)}
EXPECTED = {
    "fixture::live", "fixture::unused", "fixture::repeated",
    "dependency::live", "dependency::unused", "dependency::unreferenced_module",
}
for profile in ("debug", "release"):
    flags = ["--release"] if profile == "release" else []
    command = ["cargo", "run", "--locked", "--quiet", "--manifest-path", str(FIXTURE), *flags]
    native = subprocess.check_output(command, env=ENV, text=True)
    assert set(native.splitlines()) == EXPECTED, native
    subprocess.run(
        ["cargo", "build", "--locked", "--manifest-path", str(FIXTURE), "--lib",
         "--target", "wasm32-unknown-unknown", *flags],
        env=ENV, check=True,
    )
    wasm = TARGET / "wasm32-unknown-unknown" / profile / "day_id_registry_fixture.wasm"
    code = """
const fs = require('fs');
const module = new WebAssembly.Module(fs.readFileSync(process.argv[1]));
const names = new Set();
let sentinel = false;
for (const section of WebAssembly.Module.customSections(module, 'day_ids')) {
  const bytes = Buffer.from(section);
  for (let i = 0; i < bytes.length;) {
    const length = bytes.readUInt32LE(i); i += 4;
    const fields = bytes.subarray(i, i + length).toString('utf8').split('\\0'); i += length;
    if (fields.length === 1 && fields[0] === 'D1') { sentinel = true; continue; }
    if (fields.length !== 5 || fields[0] !== 'D1') throw Error('invalid metadata');
    names.add(fields[2]);
  }
}
if (!sentinel) throw Error('missing sentinel');
process.stdout.write(JSON.stringify([...names]));
"""
    names = json.loads(subprocess.check_output(["node", "-e", code, str(wasm)], text=True))
    assert set(names) == EXPECTED, (profile, names)
    print(f"{profile}: native and WASM retained all {len(EXPECTED)} families; cfg exclusion absent")
