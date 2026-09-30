#!/usr/bin/env node
// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// The actual shipped shim, with WHATWG URL canonicalization and a minimal Wasm ABI.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const source = fs.readFileSync(new URL('../../crates/day-cli/resources/web/shim.js', import.meta.url), 'utf8');
test('app route echoes compare against the browser-normalized hash', () => {
  const location = new URL('https://fixture.invalid/app?test=1');
  const context = vm.createContext({
    TextEncoder, TextDecoder, Blob, location,
    document: {addEventListener() {}}, ResizeObserver: class {},
    history: {
      replaceState(_state, _title, url) { location.href = new URL(url, location).href; },
      pushState(_state, _title, url) { location.href = new URL(url, location).href; },
    },
  });
  vm.runInContext(source.replace('export async function start', 'async function start') + `
    wasm = {memory: {buffer: new ArrayBuffer(65536)}};
    globalThis.setRoute = (route, replace) => {
      const bytes = utf8enc.encode(route); mem().set(bytes, 0);
      env.day_dom_set_hash(0, bytes.length, replace);
      return lastSetRoute;
    };
  `, context);
  for (const route of ['library/Titles/Book("Alice in Wonderland")', 'library/Écrivains/你好', '', 'library']) {
    for (const replace of [false, true]) {
      assert.equal(context.setRoute(route, replace), location.hash.slice(1));
    }
  }
});
