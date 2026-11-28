// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const shim = fs.readFileSync(new URL('../../crates/day-cli/resources/web/shim.js', import.meta.url), 'utf8');

test('the browser clock supplies the signed offset in seconds at the requested instant', () => {
  const original = process.env.TZ;
  try {
    const context = vm.createContext({
      TextEncoder, TextDecoder, URLSearchParams, location: new URL('https://fixture.invalid/'),
      document: { addEventListener() {} }, ResizeObserver: class {},
    });
    const offset = vm.runInContext(shim.replace('export async function start', 'async function start')
      + '\nenv.day_datetime_local_offset_secs', context);
    process.env.TZ = 'America/New_York';
    assert.equal(offset(Date.parse('2024-03-10T06:59:59Z') / 1000), -5 * 3600);
    assert.equal(offset(Date.parse('2024-03-10T07:00:00Z') / 1000), -4 * 3600);
    process.env.TZ = 'Asia/Kathmandu';
    assert.equal(offset(Date.parse('2024-01-01T20:30:00Z') / 1000), 5 * 3600 + 45 * 60);
  } finally {
    if (original === undefined) delete process.env.TZ;
    else process.env.TZ = original;
  }
});
