#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Exercise actual WASM discovery, including intentional loss, through Day's linker."""
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[2]
FIXTURE = ROOT / "crates/day-core/tests/fixtures/conformance/Cargo.toml"
TARGET = ROOT / "target/conformance-retention"
CLI = Path(os.environ.get("DAY_BIN", ROOT / "target/debug/day")).resolve()
if "DAY_BIN" not in os.environ:
    subprocess.run(["cargo", "build", "--locked", "-p", "day-cli"], cwd=ROOT, check=True)
sysroot = subprocess.check_output(["rustc", "--print", "sysroot"], text=True).strip()
version = subprocess.check_output(["rustc", "-vV"], text=True)
host = next(line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: "))
linker = Path(sysroot) / "lib/rustlib" / host / "bin" / ("rust-lld.exe" if os.name == "nt" else "rust-lld")
ENV = {**os.environ, "CARGO_TARGET_DIR": str(TARGET), "DAY_WEB_TEST_LINKER": str(linker),
       "DAY_WEB_TEST_PROVIDERS": json.dumps(["registry-widgets", "day-conformance-registry-fixture"])}
CHECK = r"""
const fs = require('fs'), assert = require('node:assert/strict');
const module = new WebAssembly.Module(fs.readFileSync(process.argv[1]));
const sections = WebAssembly.Module.customSections(module, 'day_test_expectations');
assert.equal(sections.length, 1);
const manifest = Buffer.from(sections[0]);
const names = manifest.toString().trim().split('\n').slice(1);
const missing = process.argv[2] === 'missing';
assert.equal(names.length, missing ? 5 : 4);
for (const name of ['dependency_unused', 'dependency_deep', 'dependency_inside_generic'])
  assert(names.some(n => n.endsWith('::' + name)));
assert(!names.some(n => n.includes('excluded')));
function instance(bytes) {
  const e = new WebAssembly.Instance(module, {}).exports;
  const error = () => Buffer.from(e.memory.buffer, e.error_ptr(), e.error_len()).toString();
  assert.equal(e.validate(), 0, 'uninitialized discovery must fail');
  const ptr = e.allocate(bytes.length);
  new Uint8Array(e.memory.buffer, ptr, bytes.length).set(bytes);
  return {e, ok: e.initialize(ptr, bytes.length), error};
}
const {e, ok, error} = instance(manifest);
assert.equal(ok, missing ? 0 : 1, error());
assert.equal(e.count(), 4);
assert.equal(e.validate(), missing ? 0 : 1, error());
if (missing) {
  assert.match(error(), /missing.*fixture::missing.*No tests executed/);
  assert.equal(e.rejected_filtered_run(), 1, 'dayscript must fail before filtering');
}
assert.equal(instance(Buffer.alloc(0)).ok, 0, 'missing expectations must fail');
assert.equal(instance(Buffer.from(manifest.toString().replace('::app_test', '::different_test'))).ok, 0);
const ptr = e.allocate(manifest.length);
new Uint8Array(e.memory.buffer, ptr, manifest.length).set(manifest);
e.initialize(ptr, manifest.length);
assert.equal(e.count(), 4, 'initialization is idempotent');
console.log(`${process.argv[2]}: ${names.length} declared; ${e.count()} registered; loss checks passed`);
"""

def build(profile, *, broken=False, provider=True):
    env = {**ENV}
    if profile == "release-lto":
        env.update(CARGO_PROFILE_RELEASE_LTO="true", CARGO_PROFILE_RELEASE_CODEGEN_UNITS="1")
    if profile == "debug-cgu1":
        env["CARGO_PROFILE_DEV_CODEGEN_UNITS"] = "1"
    flags = ["--release"] if profile.startswith("release") else []
    if broken:
        flags += ["--features", "missing-registration"]
    if not provider:
        flags += ["--no-default-features"]
    args = ["cargo", "rustc", "--locked", "--manifest-path", str(FIXTURE), "--lib",
            "--target", "wasm32-unknown-unknown", *flags, "--", f"-Clinker={CLI}", "-Clinker-flavor=wasm-ld"]
    if profile.startswith("release"):
        args += ["-Cstrip=symbols"]
    out = subprocess.run(args, env=env, text=True, capture_output=True)
    if not provider:
        assert out.returncode != 0 and "required conformance provider registry-widgets" in out.stderr, out.stderr
        print("missing provider: build correctly rejected")
        return
    if out.returncode:
        raise RuntimeError(out.stderr)
    wasm = TARGET / "wasm32-unknown-unknown" / ("release" if profile.startswith("release") else "debug") / "day_conformance_registry_fixture.wasm"
    subprocess.run(["node", "-e", CHECK, str(wasm), "missing" if broken else profile], check=True)

for profile in ("debug", "debug-cgu1", "release", "release-lto"):
    build(profile)
    build(profile, broken=True)
build("debug", provider=False)
