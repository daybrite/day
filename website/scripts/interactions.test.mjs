// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Run against a production build; DAY_SITE_TEST_URL can target a running preview or dev server.
import assert from 'node:assert/strict';
import { before, after, test } from 'node:test';
import { createServer } from 'node:http';
import { readFile, stat } from 'node:fs/promises';
import { extname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';
import { chromium, webkit } from 'playwright';

let server, browser, base;
const dist = resolve(fileURLToPath(new URL('../dist/', import.meta.url)));
before(async () => {
  base = process.env.DAY_SITE_TEST_URL;
  if (!base) {
    server = createServer(async (request, response) => {
      try {
        let path = resolve(dist, '.' + new URL(request.url, 'http://localhost').pathname);
        if (path !== dist && !path.startsWith(dist + sep)) throw new Error('Invalid path');
        if ((await stat(path)).isDirectory()) path = join(path, 'index.html');
        const mime = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm', '.svg': 'image/svg+xml', '.png': 'image/png', '.webp': 'image/webp' };
        response.writeHead(200, { 'Content-Type': mime[extname(path)] ?? 'application/octet-stream' });
        response.end(await readFile(path));
      } catch { response.writeHead(404); response.end(); }
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    base = `http://127.0.0.1:${server.address().port}`;
  }
  browser = await (process.env.DAY_SITE_BROWSER === 'webkit' ? webkit : chromium).launch();
});
after(async () => {
  await browser?.close();
  if (server) await new Promise(resolve => server.close(resolve));
});
async function visit(path, options = {}) {
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, ...options });
  const page = await context.newPage();
  // Isolate interactions from analytics and remote galleries; synthetic image data is a fixture.
  await page.route('**/*', async route => {
    if (new URL(route.request().url()).origin === new URL(base).origin) return route.continue();
    if (route.request().resourceType() === 'image') return route.fulfill({ contentType: 'image/svg+xml', body: '<svg xmlns="http://www.w3.org/2000/svg" width="800" height="600"><rect width="800" height="600" fill="slateblue"/></svg>' });
    return route.abort();
  });
  const errors = [];
  page.on('pageerror', error => errors.push(error.message));
  await page.goto(new URL(path, base).href, { waitUntil: 'load' });
  return { context, page, errors };
}

test('search loads once, stays dismissed during loading, and retries after failure', async () => {
  const { context, page, errors } = await visit('/showcase');
  try {
    let release;
    let requests = 0;
    const pending = new Promise(resolve => { release = resolve; });
    await page.route('**/pagefind/pagefind-ui.js', async route => {
      requests++;
      if (requests === 1) { await pending; return route.abort(); }
      return route.continue();
    });
    await page.locator('[data-search-open]').click();
    await page.locator('[data-search-dialog][open]').waitFor();
    await page.locator('[data-search-status]:visible').waitFor();
    await page.keyboard.press('Escape');
    release();
    await page.waitForFunction(() => !document.querySelector('[data-search-retry]').hidden);
    assert.equal(await page.locator('[data-search-dialog]').evaluate(d => d.open), false);
    await page.locator('[data-search-open]').click();
    await page.locator('.pagefind-ui__search-input').waitFor();
    await page.waitForFunction(() => document.activeElement?.classList.contains('pagefind-ui__search-input'));
    assert.equal(requests, 2);
    const rect = await page.locator('[data-search-dialog]').boundingBox();
    await page.mouse.click(rect.x + 3, rect.y + 3);
    assert.equal(await page.locator('[data-search-dialog]').evaluate(d => d.open), true, 'dialog padding must not dismiss');
    await page.locator('.pagefind-ui__search-input').fill('signal');
    await page.locator('.pagefind-ui__result-link').first().waitFor();
    await page.mouse.click(2, 2);
    await page.waitForFunction(() => !document.querySelector('[data-search-dialog]').open);
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});

test('gallery uses keyboard radios, keeps focus modal, navigates both axes and returns focus', async () => {
  const { context, page, errors } = await visit('/gallery/Day-Showcase');
  try {
    const radios = page.locator('[name=gallery-theme]');
    await radios.first().focus();
    const before = await radios.evaluateAll(all => all.find(r => r.checked)?.value);
    await page.keyboard.press('ArrowRight');
    assert.notEqual(await radios.evaluateAll(all => all.find(r => r.checked)?.value), before);
    const select = page.locator('select[data-seg=locale]');
    if (await select.count()) {
      const value = await select.locator('option').last().getAttribute('value');
      await select.selectOption(value);
      assert.equal(await select.inputValue(), value);
    }
    const trigger = page.locator('.tile:not([hidden]) .frame-open').first();
    await trigger.focus();
    await page.keyboard.press('Enter');
    await page.locator('dialog[data-lightbox][open]').waitFor();
    for (let i = 0; i < 9; i++) {
      await page.keyboard.press('Tab');
      // Browsers may briefly focus the document on wrap; the underlying page must stay inert.
      assert.equal(await page.evaluate(() => document.activeElement === document.body || !!document.activeElement.closest('[data-lightbox]')), true);
    }
    const platform = await page.locator('[data-lb-tk]').textContent();
    await page.keyboard.press('ArrowRight');
    assert.notEqual(await page.locator('[data-lb-tk]').textContent(), platform);
    const shot = await page.locator('[data-lb-shot]').textContent();
    await page.keyboard.press('ArrowDown');
    assert.notEqual(await page.locator('[data-lb-shot]').textContent(), shot);
    await page.keyboard.press('Escape');
    await page.waitForFunction(() => !document.querySelector('[data-lightbox]').open);
    assert.equal(await trigger.evaluate(el => document.activeElement === el), true);
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});

test('glossary definitions have stable descriptions and their links remain keyboard reachable', async () => {
  const { context, page, errors } = await visit('/docs/explanation/pieces');
  try {
    const term = page.locator('[data-gloss-pop]').first();
    await term.focus();
    const id = await term.getAttribute('data-gloss-pop');
    const pop = page.locator(`#${id}`);
    await pop.waitFor({ state: 'visible' });
    // Safari's default Tab policy skips links; Option-Tab includes them.
    await page.keyboard.press(process.env.DAY_SITE_BROWSER === 'webkit' ? 'Alt+Tab' : 'Tab');
    assert.equal(await pop.locator('a').evaluate(el => el === document.activeElement), true);
    await page.waitForTimeout(250);
    assert.equal(await pop.isVisible(), true);
    const box = await pop.boundingBox();
    assert.ok(box.x >= 0 && box.y >= 0 && box.x + box.width <= 1280);
    await page.keyboard.press('Escape');
    await pop.waitFor({ state: 'hidden' });
    const description = await term.getAttribute('aria-describedby');
    assert.equal(await page.locator(`#${description}`).count(), 1);
    await page.locator('[data-gloss-pop]').nth(1).focus();
    assert.equal(await term.getAttribute('aria-describedby'), description, 'each term keeps its own definition');
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});

test('clipboard failures and overlapping requests produce the latest feedback', async () => {
  const { context, page, errors } = await visit('/showcase');
  try {
    await page.evaluate(() => {
      window.copyCalls = [];
      navigator.clipboard.writeText = text => new Promise((resolve, reject) => window.copyCalls.push({ text, resolve, reject }));
    });
    const button = page.locator('[data-cmd]:visible').first();
    await button.click();
    await button.click();
    await page.evaluate(() => { window.copyCalls[1].reject(new Error('Synthetic permission denial')); window.copyCalls[0].resolve(); });
    await page.waitForFunction(() => document.querySelector('.qs-copy-label').textContent === 'Select the code to copy');
    await button.click();
    await page.evaluate(() => window.copyCalls[2].resolve());
    await page.waitForFunction(() => document.querySelector('.qs-copy-label').textContent === 'Copied');
    assert.equal(await page.evaluate(() => window.copyCalls[2].text), await button.getAttribute('data-cmd'));
    await page.waitForFunction(() => document.querySelector('.qs-copy-label').textContent === 'Copy');
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});

test('hero pauses for focus and changes in reduced motion, and renders without waiting for every image', async () => {
  const { context, page, errors } = await visit('/', { reducedMotion: 'reduce' });
  try {
    await page.locator('[data-hero-ready]').waitFor();
    const initial = await page.locator('.hc-slide.is-active .hc-img').getAttribute('src');
    assert.ok(await page.locator('.hc-img:not([src])').count() > 0, 'unused captures are not fetched eagerly');
    await page.emulateMedia({ reducedMotion: 'no-preference' });
    await page.locator('[data-hc-link]').focus();
    await page.waitForTimeout(3700);
    assert.equal(await page.locator('.hc-slide.is-active .hc-img').getAttribute('src'), initial);
    await page.emulateMedia({ reducedMotion: 'reduce' });
    await page.locator('[data-search-open]').focus();
    await page.waitForTimeout(3700);
    assert.equal(await page.locator('.hc-slide.is-active .hc-img').getAttribute('src'), initial);
    await page.locator('[data-theme-choice=dark]').click();
    await page.waitForFunction(() => document.querySelector('[data-hero-carousel]')?.dataset.heroTheme === 'dark');
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});

test('every reviewed hero capture loads in its theme with its gallery link and usable mobile indicators', async () => {
  const { shots } = JSON.parse(await readFile(new URL('../src/data/hero-shots.json', import.meta.url), 'utf8'));
  const { context, page, errors } = await visit('/', { reducedMotion: 'reduce' });
  try {
    await page.locator('[data-hero-ready]').waitFor();
    // Check both light → dark and dark → light; changing the pool must update every control.
    for (const theme of ['light', 'dark', 'light']) {
      await page.locator(`[data-theme-choice=${theme}]`).click();
      await page.waitForFunction(theme => document.querySelector('[data-hero-carousel]')?.dataset.heroTheme === theme, theme);
      const key = theme === 'dark' ? 'srcDark' : 'src';
      const expected = shots.filter(shot => shot[key]);
      assert.ok(expected.length >= 9, 'retain at least the original reviewed pool in each theme');
      const segments = page.locator('[data-hc-seg]:visible');
      assert.equal(await segments.count(), expected.length);
      const seen = new Set();
      for (let i = 0; i < expected.length; i++) {
        await segments.nth(i).focus();
        await page.waitForFunction(() => document.querySelector('[data-hero-carousel]')?.getAttribute('aria-busy') === 'false');
        const active = await page.locator('.hc-slide.is-active .hc-img').evaluate((img, theme) => {
          const rect = img.getBoundingClientRect(), stage = img.closest('.hc-stage').getBoundingClientRect();
          return {
            src: img.getAttribute('src'), expected: theme === 'dark' ? img.dataset.srcDark : img.dataset.srcLight,
            gallery: img.dataset.gallery, complete: img.complete && img.naturalWidth > 0,
            aspectError: Math.abs(rect.width / rect.height - img.naturalWidth / img.naturalHeight),
            fits: rect.width <= stage.width + 1 && rect.height <= stage.height + 1,
          };
        }, theme);
        assert.equal(active.src, active.expected);
        assert.equal(active.complete, true);
        assert.ok(active.aspectError < 0.02 && active.fits, `capture must fit without distortion: ${active.src}`);
        assert.equal(await page.locator('[data-hc-link]').getAttribute('href'), active.gallery);
        seen.add(active.src);
      }
      assert.equal(seen.size, expected.length, 'every reviewed image must be reachable');
      await page.setViewportSize({ width: 390, height: 844 });
      const indicators = await segments.evaluateAll(elements => elements.map(element => {
        const { x, y, width, height } = element.getBoundingClientRect();
        return { x, y, width, height };
      }));
      assert.ok(indicators.every(r => r.width >= 24 && r.height >= 24 && r.x >= 0 && r.x + r.width <= 390));
      if (expected.length > 15) assert.ok(new Set(indicators.map(r => r.y)).size > 1, 'large pools wrap instead of squeezing');
      const tablet = page.locator('[data-hc-seg]:visible[href$="#split"]');
      await tablet.focus();
      await page.waitForFunction(() => document.querySelector('.hc-slide.is-active .hc-img')?.dataset.shot === 'split');
      assert.ok(await page.locator('.hc-slide.is-active .hc-img').evaluate(img => {
        const rect = img.getBoundingClientRect(), stage = img.closest('.hc-stage').getBoundingClientRect();
        return Math.abs(rect.width / rect.height - img.naturalWidth / img.naturalHeight) < 0.02 && rect.height <= stage.height + 1;
      }), 'tablet captures must retain their aspect ratio on phones');
      await page.setViewportSize({ width: 1280, height: 900 });
    }
    assert.deepEqual(errors, []);
  } finally { await context.close(); }
});
