// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const source = fs.readFileSync(new URL('../../crates/day-cli/resources/web/shim.js', import.meta.url), 'utf8');
function fixture() {
  const listeners = new Set(), readings = [];
  const context = vm.createContext({
    TextEncoder, TextDecoder, Blob, isSecureContext: true, DeviceMotionEvent: class {},
    document: { addEventListener() {} }, ResizeObserver: class {},
    addEventListener: (name, fn) => { if (name === 'devicemotion') listeners.add(fn); },
    removeEventListener: (name, fn) => { if (name === 'devicemotion') listeners.delete(fn); },
    sample: (...args) => readings.push(args),
    setInterval() { throw Error('sensor delivery must not start a polling timer'); },
  });
  vm.runInContext(source.replace('export async function start', 'async function start') + `
    wasm = { day_sensors_sample: (...args) => sample(...args) };
    globalThis.sensors = env;
  `, context);
  return { env: context.sensors, context, listeners, readings,
    emit: e => [...listeners].forEach(fn => fn(e)) };
}
const event = { accelerationIncludingGravity: {x: 1, y: 2, z: 3}, rotationRate: {beta: 90, gamma: -45, alpha: 180} };
test('new events deliver immediately, share one listener and release it on last stop', () => {
  const f = fixture();
  assert.equal(f.env.day_dom_sensor_available(0), 1);
  assert.equal(f.listeners.size, 0, 'availability probing must not subscribe');
  f.env.day_dom_sensor_start(0, 1); f.env.day_dom_sensor_start(1, 2);
  assert.equal(f.listeners.size, 1); assert.equal(f.readings.length, 0);
  f.emit(event);
  assert.deepEqual(f.readings, [[0, 1, 1, 2, 3], [1, 2, Math.PI/2, -Math.PI/4, Math.PI]]);
  f.env.day_dom_sensor_stop(0, 1); assert.equal(f.listeners.size, 1);
  f.emit(event); assert.equal(f.readings.length, 3);
  f.env.day_dom_sensor_stop(1, 2); assert.equal(f.listeners.size, 0);
  f.emit(event); assert.equal(f.readings.length, 3);
});
test('old stop cannot cancel a new subscription; partial or nonfinite events are ignored', () => {
  const f = fixture();
  f.env.day_dom_sensor_start(0, 1); f.env.day_dom_sensor_stop(0, 1);
  f.env.day_dom_sensor_start(0, 2); f.env.day_dom_sensor_stop(0, 1);
  assert.equal(f.listeners.size, 1);
  for (const y of [null, undefined, NaN, Infinity]) f.emit({accelerationIncludingGravity: {x: 1, y, z: 3}});
  assert.equal(f.readings.length, 0);
  f.emit(event); assert.deepEqual(f.readings, [[0, 2, 1, 2, 3]]);
  assert.equal(f.env.day_dom_sensor_available(2), 0);
  f.context.isSecureContext = false;
  assert.equal(f.env.day_dom_sensor_available(0), 0);
});
