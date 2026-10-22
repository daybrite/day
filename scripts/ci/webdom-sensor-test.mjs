#!/usr/bin/env node
// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// webdom-sensor-test.mjs <url>: verify day-part-sensors' browser arm end to end.
//
// Headless WebKit has no motion hardware, so the walkthrough alone can only ever prove the
// "no samples" path. This dispatches a synthetic `devicemotion` with known values and asserts the
// showcase's rows show the converted numbers, which pins the two things a browser sensor arm
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
  await page.waitForSelector('#sensor_accel');

  await page.waitForSelector('#sensor_accel_live');
  await page.waitForSelector('#sensor_gyro_live');
  const layout = await page.evaluate(() => ({
    magnetFirst: !!(document.getElementById('sensor_magnet').compareDocumentPosition(
      document.getElementById('sensor_accel')) & Node.DOCUMENT_POSITION_FOLLOWING),
    calibrationControls: document.querySelectorAll('[id^="sensor_calibrated:"]').length,
  }));
  if (!layout.magnetFirst) failures.push('magnetometer must precede the other sensors');
  if (layout.calibrationControls !== 0) failures.push('web must not offer unsupported calibration modes');

  await page.evaluate(() => {
    const e = new Event('devicemotion');
    // m/s², passed through unchanged.
    e.accelerationIncludingGravity = { x: 1.25, y: -2.5, z: 9.81 };
    // deg/s, and asymmetric so a swapped axis cannot pass by coincidence.
    e.rotationRate = { alpha: 180, beta: 90, gamma: -45 };
    window.dispatchEvent(e);
  });
  await page.waitForFunction(() => document.getElementById('sensor_accel')?.textContent.includes('+1.25'));

  const text = (id) =>
    page.evaluate((i) => document.getElementById(i)?.textContent ?? '(missing)', id);
  const accel = await text('sensor_accel');
  const gyro = await text('sensor_gyro');
  const magnet = await text('sensor_magnet');

  // The readouts carry Fluent bidi isolate marks around each value, so match on the numbers only.
  check('accelerometer x', accel, '+1.25');
  check('accelerometer y', accel, '-2.50');
  check('accelerometer z', accel, '+9.81');
  // 90 deg/s → π/2 rad/s; -45 → -π/4; 180 → π.
  check('gyroscope x (beta, deg→rad)', gyro, '+1.57');
  check('gyroscope y (gamma, deg→rad)', gyro, '-0.79');
  check('gyroscope z (alpha, deg→rad)', gyro, '+3.14');
  // No cross-browser magnetometer API exists; reporting one would be a lie.
  check('magnetometer', magnet, 'unavailable');

  // Pause one sensor while the shared browser motion listener continues feeding the other.
  const setRunning = async (id, value) => {
    await page.locator(`[id="sensor_running:${id}"]`).setChecked(value, { force: true });
    await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))));
  };
  const emit = async (x, beta) => page.evaluate(({ x, beta }) => {
    const event = new Event('devicemotion');
    event.accelerationIncludingGravity = { x, y: -2.5, z: 9.81 };
    event.rotationRate = { alpha: 180, beta, gamma: -45 };
    window.dispatchEvent(event);
  }, { x, beta });
  const width = () => page.locator('#sensor_accel').evaluate((el) => el.getBoundingClientRect().width);
  const initialWidth = await width();
  await setRunning('sensor_accel', false);
  await emit(-123.45, 180);
  await page.waitForFunction(() => document.getElementById('sensor_gyro')?.textContent.includes('+3.14'));
  check('paused acceleration stays unchanged', await text('sensor_accel'), accel);
  await setRunning('sensor_accel', true);
  await emit(-123.45, 270);
  await page.waitForFunction(() => document.getElementById('sensor_accel')?.textContent.includes('-123.45'));
  if (Math.abs(await width() - initialWidth) > 0.1) failures.push('readout width changed with digit count');
  await setRunning('sensor_gyro', false);
  const pausedGyro = await text('sensor_gyro');
  await emit(12.5, 360);
  await page.waitForFunction(() => document.getElementById('sensor_accel')?.textContent.includes('+12.50'));
  check('paused gyro stays unchanged', await text('sensor_gyro'), pausedGyro);
  // Drop the last listener, then resume both: starting again must produce fresh readings.
  await setRunning('sensor_accel', false);
  await emit(99, 90);
  await setRunning('sensor_accel', true);
  await setRunning('sensor_gyro', true);
  await emit(42, 90);
  await page.waitForFunction(() => document.getElementById('sensor_accel')?.textContent.includes('+42.00')
    && document.getElementById('sensor_gyro')?.textContent.includes('+1.57'));
  console.log('independent pause/resume, last-listener restart, and reserved readout width passed');

  console.log(`accel : ${accel}`);
  console.log(`gyro  : ${gyro}`);
  console.log(`magnet: ${magnet}`);
} finally {
  await browser.close();
}

if (pageErrors.length) {
  // An unresolved wasm import shows up here as a LinkError, the failure mode that leaves the page
  // blank, so it must never pass silently.
  failures.push(`page errors: ${pageErrors.slice(0, 3).join(' | ')}`);
}
if (failures.length) {
  console.error('\nweb sensor check FAILED:');
  for (const f of failures) console.error(`  - ${f}`);
  process.exit(1);
}
console.log('\nweb sensor check passed');
