#!/usr/bin/env node
// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Usage: DAY_WEB_DRIVER_PLAYWRIGHT=<install-dir> node scripts/ci/webdom-button-test.mjs <Showcase URL>
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
const url = process.argv[2];
if (!url) throw new Error('Provide a Showcase URL');
const root = process.env.DAY_WEB_DRIVER_PLAYWRIGHT ?? process.cwd();
const playwright = createRequire(path.join(root, 'resolve-anchor.js'))('playwright');
for (const engine of ['chromium', 'webkit']) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'day-button-input-'));
  const context = await playwright[engine].launchPersistentContext(profile, {
    viewport: { width: 1100, height: 1000 },
  });
  try {
    const page = context.pages()[0] ?? await context.newPage();
    await page.goto(url);
    await page.locator('#nav').waitFor();
    await page.evaluate(() => { location.hash = '#controls'; });
    const player = page.locator('#btn-icon-only');
    await player.waitFor();
    const name = expected => page.waitForFunction(value =>
      document.querySelector('#btn-icon-only')?.getAttribute('aria-label') === value, expected);
    await name('Play');
    assert.equal(await player.getAttribute('title'), 'Play');
    assert.equal((await player.textContent()).trim(), '');
    await player.evaluate(el => { window.testButton = el; });
    await player.click();
    await name('Pause');
    assert.equal(await player.getAttribute('title'), 'Pause');
    assert.ok(await player.evaluate(el => el === window.testButton));
    await player.press('Space');
    await name('Play');
    assert.ok(await player.evaluate(el => el === document.activeElement));
    for (const id of ['btn-plain', 'btn-icon-label', 'btn-image']) {
      const button = page.locator(`#${id}`);
      assert.ok(!(await button.getAttribute('title')), `${id}: no automatic tooltip`);
      assert.ok((await button.textContent()).trim(), `${id}: visible title`);
      await button.click();
    }
    await page.waitForFunction(() => document.querySelector('#btn-presses')?.textContent.trim() === '3');
    assert.ok(await page.locator('#btn-disabled').isDisabled());
    await page.locator('#btn-disabled').click({ force: true });
    assert.equal((await page.locator('#btn-presses').textContent()).trim(), '3');
    for (const id of ['btn-icon-label', 'btn-image', 'btn-icon-only']) {
      const icon = page.locator(`#${id} [aria-hidden="true"]`);
      assert.equal(await icon.count(), 1);
      const box = await icon.boundingBox();
      assert.ok(box && box.width >= 16 && box.height >= 16, `${id}: visible icon dimensions`);
      const mask = await icon.evaluate(el => getComputedStyle(el).maskImage);
      assert.notEqual(mask, 'none');
    }
    if (process.env.DAY_BUTTON_TEST_SCREENSHOTS) {
      await player.scrollIntoViewIfNeeded();
      await page.screenshot({ path: path.join(process.env.DAY_BUTTON_TEST_SCREENSHOTS, `buttons-${engine}.png`) });
    }
    console.log(`${engine}: PASS — reactive icons and names, keyboard, tooltips, disabled buttons`);
  } finally {
    await context.close();
    fs.rmSync(profile, { recursive: true, force: true });
  }
}
