#!/usr/bin/env node
// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Run: node --test scripts/ci/webdom-clipboard-test.mjs
// Exercise the shipped shim with controlled browser promises and a minimal Wasm ABI.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = fs.readFileSync(new URL('../../crates/day-cli/resources/web/shim.js', import.meta.url), 'utf8');
const tick = () => new Promise(resolve => setImmediate(resolve));
function fixture() {
  const writes = [];
  const results = new Map();
  let reads = 0;
  const context = vm.createContext({
    TextEncoder, TextDecoder, Blob, queueMicrotask,
    document: { addEventListener() {} },
    ResizeObserver: class { constructor() {} },
    ClipboardItem: class { static supports() { return true; } },
    navigator: { clipboard: {
      write: () => new Promise((resolve, reject) => writes.push({ resolve, reject })),
      read: async () => { reads++; return []; },
    } },
    result: (id, status, bytes) => results.set(id, { status, bytes }),
  });
  vm.runInContext(source.replace('export async function start', 'async function start') + `
    wasm = {
      memory: { buffer: new ArrayBuffer(65536) },
      day_dom_alloc: () => 32768,
      day_clipboard_result: (id, status, p, n) => result(id, status, mem().slice(p, p+n)),
    };
    globalThis.write = id => {
      const bytes = clipPacket([{ mime: 'text/plain', bytes: utf8enc.encode('copy') }]);
      mem().set(bytes, 0);
      env.day_dom_clipboard_write_bytes(id, 0, bytes.length);
    };
    globalThis.read = id => {
      const bytes = utf8enc.encode('text/plain');
      mem().set(bytes, 0);
      env.day_dom_clipboard_read_bytes(id, 0, bytes.length);
    };
    globalThis.pasteEvent = () => {
      activeClipboardEvent = { type: 'paste', clipboardData: {
        files: [], getData: () => 'native paste',
      } };
    };
  `, context);
  return { context, writes, results, reads: () => reads };
}

test('a byte read waits for all preceding writes, even if they settle out of order', async () => {
  const f = fixture();
  f.context.write(1);
  f.context.write(2);
  f.context.read(3);
  await tick();
  assert.equal(f.reads(), 0);
  f.writes[1].resolve();
  await tick();
  assert.equal(f.reads(), 0);
  f.writes[0].resolve();
  await tick();
  assert.equal(f.reads(), 1);
  assert.equal(f.results.get(3).status, 0);
});

test('a rejected write reports failure but does not poison the following read', async () => {
  const f = fixture();
  f.context.write(1);
  f.context.read(2);
  f.writes[0].reject(new Error('permission denied'));
  await tick();
  assert.equal(f.results.get(1).status, 1);
  assert.equal(f.reads(), 1);
  assert.equal(f.results.get(2).status, 0);
});

test('native paste snapshots bypass pending writes and the async clipboard API', async () => {
  const f = fixture();
  f.context.write(1);
  f.context.pasteEvent();
  f.context.read(2);
  await tick();
  assert.equal(f.reads(), 0);
  assert.equal(f.results.get(2).status, 0);
  assert.ok(new TextDecoder().decode(f.results.get(2).bytes).endsWith('native paste'));
  f.writes[0].resolve();
  await tick();
});
