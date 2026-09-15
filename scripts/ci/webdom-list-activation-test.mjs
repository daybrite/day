#!/usr/bin/env node
// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Test Showcase using browser input, rather than dayscript event injection.
// Usage: DAY_WEB_DRIVER_PLAYWRIGHT=<install-dir> node scripts/ci/webdom-list-activation-test.mjs <url>
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const url = process.argv[2];
if (!url) throw new Error('usage: webdom-list-activation-test.mjs <Showcase URL>');
let playwright;
for (const root of [process.env.DAY_WEB_DRIVER_PLAYWRIGHT, process.cwd()].filter(Boolean)) {
  try {
    playwright = createRequire(path.join(root, 'resolve-anchor.js'))('playwright');
    break;
  } catch { /* Try the next installation. */ }
}
if (!playwright) throw new Error('Playwright not found; set DAY_WEB_DRIVER_PLAYWRIGHT');

for (const engine of ['chromium', 'webkit']) {
  const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'day-list-input-'));
  // WebKit's private context omits OPFS on macOS.
  const context = await playwright[engine].launchPersistentContext(profile, {
    viewport: { width: 1100, height: 900 }, hasTouch: true,
  });
  try {
    const page = context.pages()[0] ?? await context.newPage();
    await page.goto(url, { waitUntil: 'load' });
    await page.locator('#nav').waitFor();
    await page.evaluate(() => { location.hash = '#list'; });
    const list = page.locator('#demo-list');
    await list.waitFor();
    const rows = list.locator('[role="option"]');
    const activation = n => page.waitForFunction(expected =>
      document.querySelector('#list-activated')?.textContent.replace(/[\u2066-\u2069]/g, '')
        .includes(`: ${expected} (`), n);
    await page.evaluate(() => {
      window.addEventListener('keydown', e => {
        if (e.key === 'Enter') window.listTestEnterHandled = e.defaultPrevented;
      });
    });
    async function enter(target, handled) {
      await page.evaluate(() => { window.listTestEnterHandled = null; });
      await target.press('Enter');
      assert.equal(await page.evaluate(() => window.listTestEnterHandled), handled,
        `${engine}: Enter handled=${handled}`);
    }
    await activation(0);
    await enter(list, false); // No selected row: leave the default action available.
    await rows.nth(1).click();
    await activation(0); // Selection must not activate a row.
    await enter(list, true);
    await activation(2);
    await rows.nth(3).dblclick();
    await activation(4);
    await list.press('ArrowDown');
    await activation(4);

    // Embedded controls retain their input; bubbling clicks/Enter must not invoke the row.
    await rows.nth(4).evaluate(el => {
      const input = document.createElement('input'); input.id = 'list-test-input'; el.append(input);
      const button = document.createElement('button');
      button.id = 'list-test-button'; button.textContent = 'Embedded'; el.append(button);
    });
    await enter(page.locator('#list-test-input'), false);
    await page.locator('#list-test-button').dblclick();
    await activation(4);

    // Real touch input: WebKit can report touch pointerdown/up but a mouse click.
    await rows.nth(5).tap();
    await activation(6);
    await rows.nth(6).click();
    await activation(6); // A later mouse click must not inherit the touch source.
    await page.locator('#list-clear').click();
    await enter(list, false);

    // A selectable list without on_activate must not claim Enter, even with a selection.
    await page.evaluate(() => { location.hash = '#model'; });
    const selectionOnly = page.locator('#model-list');
    await selectionOnly.waitFor();
    await selectionOnly.locator('[role="option"]').first().click();
    await enter(selectionOnly, false);
    if (process.env.DAY_LIST_TEST_SCREENSHOTS) {
      await page.screenshot({ path: path.join(process.env.DAY_LIST_TEST_SCREENSHOTS, `list-${engine}.png`) });
    }
    console.log(`${engine}: PASS — touch, mouse, keyboard, embedded controls, and unhandled Enter`);
  } finally {
    await context.close();
    fs.rmSync(profile, { recursive: true, force: true });
  }
}
