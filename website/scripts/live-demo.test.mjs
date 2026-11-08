// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Exercise the shipped Wasm through browser inputs, not a JavaScript imitation of the signal.
import assert from 'node:assert/strict';
import { after, before, test } from 'node:test';
import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, webkit } from 'playwright';
import { demoSource } from './live-demo.mjs';

const dist = resolve(fileURLToPath(new URL('../dist/', import.meta.url)));
let server, browser, base;
before(async () => {
  if (process.env.DAY_DEMO_TEST_URL) base = process.env.DAY_DEMO_TEST_URL;
  else {
    server = createServer(async (req, res) => {
      try {
        let path = resolve(dist, '.' + new URL(req.url, 'http://localhost').pathname);
        if (path !== dist && !path.startsWith(dist + sep)) throw new Error('Invalid path');
        if ((await stat(path)).isDirectory()) path = join(path, 'index.html');
        const mime = { '.wasm': 'application/wasm', '.js': 'text/javascript', '.html': 'text/html', '.css': 'text/css', '.json': 'application/json' };
        res.writeHead(200, { 'Content-Type': mime[extname(path)] || 'application/octet-stream' });
        res.end(await readFile(path));
      } catch { res.writeHead(404); res.end(); }
    });
    await new Promise(done => server.listen(0, '127.0.0.1', done));
    base = `http://127.0.0.1:${server.address().port}/`;
  }
  browser = await (process.env.DAY_DEMO_BROWSER === 'webkit' ? webkit : chromium).launch();
});
after(async () => {
  await browser?.close();
  if (server) await new Promise(done => server.close(done));
});

for (const [width, initialTheme] of [[1440, 'light'], [390, 'dark']]) {
  test(`shared signal, lazy loading and theme changes at ${width}px`, async () => {
    const context = await browser.newContext({ viewport: { width, height: 800 }, reducedMotion: 'reduce' });
    try {
      await context.addInitScript(theme => localStorage.setItem('day-theme', theme), initialTheme);
      const page = await context.newPage();
      const errors = [], requests = [];
      page.on('pageerror', e => errors.push(e.message));
      page.on('request', r => requests.push(r.url()));
      // Gallery media is unrelated to this test; an unavailable app site must not delay it.
      await page.route('**/*', route => new URL(route.request().url()).origin === new URL(base).origin
        ? route.continue() : route.abort());
      await page.goto(base, { waitUntil: 'domcontentloaded' });
      // Native lazy-load distances vary by browser and connection. The iframe exists in
      // the HTML; the browser may start fetching it before the visitor scrolls this far.
      assert.equal(await page.locator('[data-live-demo] iframe').getAttribute('loading'), 'lazy');
      assert.equal(await page.locator('.rust-example-source pre code').textContent(), await demoSource());
      await page.locator('[data-live-demo]').scrollIntoViewIfNeeded();
      await page.locator('[data-live-demo][data-ready=true]').waitFor();
      const demo = page.frames().find(frame => frame.url().includes('/demos/shared-signal/'));
      assert.ok(demo);
      const wasmRequests = () => requests.filter(url => url.endsWith('/app.wasm')).length;
      assert.equal(wasmRequests(), 1);
      const slider = demo.getByRole('slider');
      const field = demo.getByRole('textbox');
      const check = async value => {
        await demo.waitForFunction(n => Math.abs(Number(document.querySelector('input[type=range]')?.value) - n / 100) < 0.0001
          && document.querySelector('input[type=text]')?.value.replace(/[\u2066-\u2069]/g, '') === `${n}%`
          && Math.abs(document.querySelector('progress').value - n / 100) < 0.0001
          && Math.abs(Number(getComputedStyle(document.querySelector('canvas').parentElement).opacity) - n / 100) < 0.0001, value);
        const text = (await demo.locator('body').innerText()).replace(/[\u2066-\u2069]/g, '');
        assert.ok(text.includes(`Value: ${value}%`), text);
      };
      const checkTint = async () => {
        // Inspect the actual canvas paint, not a copied theme palette or the bridge's inputs.
        await demo.waitForFunction(() => {
          const canvas = document.querySelector('canvas');
          const [r, g, b, a] = canvas.getContext('2d').getImageData(canvas.width / 2, canvas.height / 2, 1, 1).data;
          return a === 255 && `rgb(${r}, ${g}, ${b})` === getComputedStyle(document.querySelector('input[type=range]')).accentColor;
        });
        assert.equal(await demo.locator('canvas').evaluate(el => el.getContext('2d').getImageData(0, 0, 1, 1).data[3]), 0, 'rounded corners remain unpainted');
        assert.equal(await demo.locator('canvas').evaluate(el => el.tabIndex), -1, 'decorative swatch is not a keyboard stop');
      };
      await check(40);
      await checkTint();
      await slider.focus();
      await slider.press('ArrowRight');
      await check(41);
      await demo.getByRole('button', { name: '+', exact: true }).click();
      await check(42);
      await field.fill('73%');
      await field.press('Enter');
      await check(73);
      await demo.getByRole('button', { name: '−', exact: true }).click();
      await check(72);

      const frameURL = demo.url();
      for (const theme of ['dark', 'light']) {
        if (await page.locator('html').getAttribute('data-theme') !== theme) {
          await page.locator(`[data-theme-choice="${theme}"]`).first().evaluate(button => button.click());
        }
        await demo.waitForFunction(dark => document.documentElement.classList.contains('dark') === dark, theme === 'dark');
        await check(72);
        await checkTint();
        assert.equal(demo.url(), frameURL);
        assert.equal(wasmRequests(), 1, 'a theme change must not reload the app');
      }
      await field.fill('150%');
      await field.press('Enter');
      await check(100);
      await field.fill('-10%');
      await field.press('Enter');
      await check(0);
      // Bare percentages use the same units; invalid edits restore the formatted value.
      await field.fill('40');
      await field.press('Enter');
      await check(40);
      for (const text of ['not a number', 'NaN', 'inf', '40%%', '40']) {
        await field.fill(text);
        await field.press('Enter');
        await check(40);
      }
      await field.fill('65%');
      await slider.focus();
      await check(65);
      await demo.getByRole('button', { name: 'Reset', exact: true }).click();
      await check(40);
      assert.ok(await demo.locator('input[type=range]').evaluate(el => {
        const r = el.getBoundingClientRect(); return r.left >= 0 && r.right <= innerWidth;
      }), 'slider fits inside the mobile iframe');
      assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth), 'page fits the viewport');
      assert.deepEqual(errors, []);
    } finally { await context.close(); }
  });
}
