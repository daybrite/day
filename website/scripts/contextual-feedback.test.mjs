// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { after, before, test } from 'node:test';
import { chromium } from 'playwright';

const shim = await readFile(new URL('../../crates/day-cli/resources/web/shim.js', import.meta.url), 'utf8');
const css = await readFile(new URL('../../crates/day-cli/resources/web/day.css', import.meta.url), 'utf8');
let browser;
before(async () => { browser = await chromium.launch(); });
after(async () => { await browser?.close(); });

for (const native of [true, false]) {
  test(`feedback with ${native ? 'native' : 'emulated'} popovers`, async () => {
    const page = await browser.newPage();
    try {
      await page.setContent('<button id="anchor">Open fixture</button><button id="outside">Outside fixture</button>');
      await page.addStyleTag({ content: css });
      if (!native) await page.evaluate(() => { delete HTMLElement.prototype.showPopover; });
      await page.addScriptTag({ type: 'module', content: shim + `
        let opened = false;
        window.opens = 0;
        const anchor = els[2] = document.querySelector('#anchor');
        const memory = new WebAssembly.Memory({initial:1});
        const bytes = new TextEncoder().encode('Fixture toast');
        new Uint8Array(memory.buffer).set(bytes);
        wasm = {memory, day_dom_popover_dismissed() {
          opened = false;
          env.day_dom_close_popover(3);
        }};
        anchor.onclick = () => {
          if (opened) return;
          opened = true;
          window.opens++;
          els[3] = div('panel');
          env.day_dom_popover(2, 3, 100, 50);
        };
        window.resizePanel = () => env.day_dom_resize_popover(3, 160, 90);
        window.toast = (length) => env.day_dom_toast(0, length);
      ` });
      const anchor = page.locator('#anchor');
      const popup = page.locator('.day-context-popover');
      await anchor.click();
      await popup.waitFor();
      await anchor.click();
      await page.evaluate(() => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve))));
      assert.equal(await page.evaluate(() => window.opens), 1);
      assert.equal(await popup.count(), 1);
      await page.evaluate(() => window.resizePanel());
      assert.equal((await page.locator('.panel').boundingBox()).height, 90);
      await page.locator('#outside').click();
      await popup.waitFor({state:'detached'});
      await anchor.click();
      await page.keyboard.press('Escape');
      await popup.waitFor({state:'detached'});
      await page.clock.install();
      await page.evaluate(() => window.toast(7));
      const toast = page.locator('.day-context-toast');
      assert.equal(await toast.innerText(), 'Fixture');
      await page.clock.runFor(1000);
      await page.evaluate(() => window.toast(13));
      assert.equal(await toast.count(), 1);
      await page.clock.runFor(2000);
      assert.equal(await toast.innerText(), 'Fixture toast');
      await page.clock.runFor(1000);
      assert.equal(await toast.count(), 0);
    } finally {
      await page.close();
    }
  });
}
