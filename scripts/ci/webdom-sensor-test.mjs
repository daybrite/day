#!/usr/bin/env node
// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// webdom-sensor-test.mjs <url> — verify day-part-sensors' browser arm end to end.
//
// Headless WebKit has no motion hardware, so the walkthrough alone can only ever prove the
// "unavailable" path. This dispatches a synthetic `devicemotion` with KNOWN values and asserts the
// showcase's rows show the converted numbers — which pins the two things a browser sensor arm
// realistically gets wrong: the unit conversion (rotationRate is deg/s, day's contract is rad/s)
// and the axis mapping (beta→x, gamma→y, alpha→z).
//
// Usage: node scripts/ci/webdom-sensor-test.mjs http://127.0.0.1:PORT/
import { createRequire } from 'node:module';
import path from 'node:path';

const url = process.argv[2];
if (!url) {
  console.error('usage: webdom-sensor-test.mjs <url>');
  process.exit(2);
}

// Resolve playwright the way webdom-driver.mjs does: CI installs it into a scratch directory, and
// an ESM `import` would not find it there (ESM ignores NODE_PATH).
let webkit;
const roots = [process.env.DAY_WEB_DRIVER_PLAYWRIGHT, process.cwd()].filter(Boolean);
for (const root of roots) {
  try {
    webkit = createRequire(path.join(root, 'resolve-anchor.js'))('playwright').webkit;
    break;
  } catch {
    /* try the next root */
  }
}
if (!webkit) {
  console.error(`webdom-sensor-test: playwright not found under: ${roots.join(', ')}`);
  process.exit(3);
}

const browser = await webkit.launch();
const page = await browser.newPage();
const pageErrors = [];
page.on('pageerror', (e) => pageErrors.push(String(e)));

const failures = [];
const check = (what, actual, expected) => {
  if (!actual.includes(expected)) {
    failures.push(`${what}: expected ${JSON.stringify(expected)} in ${JSON.stringify(actual)}`);
  }
};

try {
  await page.goto(url, { waitUntil: 'load' });
  await page.waitForTimeout(1500);
  await page.evaluate(() => {
    location.hash = '#system';
  });
  await page.waitForTimeout(800);

  await page.evaluate(() => {
    const e = new Event('devicemotion');
    // m/s², passed through unchanged.
    e.accelerationIncludingGravity = { x: 1.25, y: -2.5, z: 9.81 };
    // deg/s, and deliberately asymmetric so a swapped axis cannot pass by coincidence.
    e.rotationRate = { alpha: 180, beta: 90, gamma: -45 };
    window.dispatchEvent(e);
  });
  // Longer than the feed's sample interval, so a tick is guaranteed to have run.
  await page.waitForTimeout(600);

  const text = (id) =>
    page.evaluate((i) => document.getElementById(i)?.textContent ?? '(missing)', id);
  const accel = await text('sensor-accel');
  const gyro = await text('sensor-gyro');
  const magnet = await text('sensor-magnet');

  // The readouts carry Fluent bidi isolate marks around each value, so match on the numbers only.
  check('accelerometer x', accel, '+1.25');
  check('accelerometer y', accel, '-2.50');
  check('accelerometer z', accel, '+9.81');
  // 90 deg/s → π/2 rad/s; -45 → -π/4; 180 → π.
  check('gyroscope x (beta, deg→rad)', gyro, '+1.57');
  check('gyroscope y (gamma, deg→rad)', gyro, '-0.79');
  check('gyroscope z (alpha, deg→rad)', gyro, '+3.14');
  // No cross-browser magnetometer API exists — reporting one would be a lie.
  check('magnetometer', magnet, 'unavailable');

  console.log(`accel : ${accel}`);
  console.log(`gyro  : ${gyro}`);
  console.log(`magnet: ${magnet}`);
} finally {
  await browser.close();
}

if (pageErrors.length) {
  // An unresolved wasm import shows up here as a LinkError — the failure mode that leaves the page
  // blank, so it must never pass silently.
  failures.push(`page errors: ${pageErrors.slice(0, 3).join(' | ')}`);
}
if (failures.length) {
  console.error('\nweb sensor check FAILED:');
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log('\nweb sensor check passed');
