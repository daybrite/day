// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// Usage: node scripts/ci/web-resources-test.mjs MAP_DIST SHOWCASE_DIST [OUTPUT_DIR]
// CI: node scripts/ci/web-resources-test.mjs --showcase-only SHOWCASE_DIST [OUTPUT_DIR]
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs/promises';
import http from 'node:http';
import path from 'node:path';
import { chromium, webkit } from '../../website/node_modules/playwright/index.mjs';

assert(process.argv[2] && process.argv[3], 'Provide map and Showcase web distribution directories');
const output = path.resolve(process.argv[4] ?? 'build/test-results/web-resources');
await fs.mkdir(output, { recursive: true });
const showcaseOnly = process.argv[2] === '--showcase-only';
const roots = { showcase: path.resolve(process.argv[3]) };
if (!showcaseOnly) roots.map = path.resolve(process.argv[2]);
roots.isolated = roots.showcase;
roots.fallback = roots.showcase;
const servedAssets = [];
const mime = {
  '.wasm': 'application/wasm', '.js': 'text/javascript', '.html': 'text/html',
  '.css': 'text/css', '.json': 'application/json', '.svg': 'image/svg+xml', '.png': 'image/png',
};
const server = http.createServer(async (req, res) => {
  try {
    const url = new URL(req.url, 'http://localhost');
    const [, app, ...parts] = url.pathname.split('/');
    const filename = path.resolve(roots[app] ?? '/nonexistent', parts.join('/') || 'index.html');
    if (!roots[app] || !filename.startsWith(roots[app] + path.sep)) throw Error('Bad path');
    if (parts[0] === 'assets') {
      servedAssets.push(url.pathname);
      if (app !== 'fallback') { res.writeHead(503); res.end(); return; }
    }
    let data = await fs.readFile(filename);
    if (filename.endsWith('shim.js')) {
      const hook = '  const resourcesReady = installResources();';
      assert(data.toString().includes(hook), 'Review resource test instrumentation after bootstrap changes');
      data = data.toString().replace(hook,
        hook + '\n window.__resourcesTest = {wasm, resourceBytes, bundledFiles, bundledUrls};');
    }
    if (filename.endsWith('sw.js')) {
      // Keep the worker in registration.active with state='activating' while the app boots.
      // Cold/loaded CI runners hit this naturally; fast local runs must cover it too.
      const hook = 'await self.clients.claim();';
      assert(data.toString().includes(hook), 'Review worker activation test instrumentation');
      data = data.toString().replace(hook,
        'await new Promise(resolve => setTimeout(resolve, 1000)); ' + hook);
    }
    // Exercise both ordinary static hosting and the isolation headers used by day launch.
    res.writeHead(200, {
      'Content-Type': mime[path.extname(filename)] ?? 'application/octet-stream',
      'Cache-Control': 'no-store',
      ...(app === 'isolated' ? {'Cross-Origin-Opener-Policy': 'same-origin', 'Cross-Origin-Embedder-Policy': 'require-corp'} : {}),
    });
    res.end(data);
  } catch (error) {
    res.writeHead(404);
    res.end(String(error));
  }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const origin = `http://127.0.0.1:${server.address().port}`;
const report = [];

async function verifyBytes(page, app) {
  const hashes = await page.evaluate(async () => {
    // Any cached JS view is now detached. Resource consumers must reacquire their views.
    __resourcesTest.wasm.memory.grow(1);
    const hashes = [];
    for (const name of __resourcesTest.bundledFiles.keys()) {
      const digest = await crypto.subtle.digest('SHA-256', __resourcesTest.resourceBytes(name).slice());
      hashes.push([name, [...new Uint8Array(digest)].map(b => b.toString(16).padStart(2, '0')).join('')]);
    }
    return hashes;
  });
  for (const [name, hash] of hashes) {
    const expected = createHash('sha256').update(await fs.readFile(path.join(roots[app], name))).digest('hex');
    assert.equal(hash, expected, name);
  }
}

async function verifyCore(browser, type, app) {
  const context = await browser.newContext({
    serviceWorkers: 'block', viewport: { width: 1280, height: 1000 },
  });
  try {
    await context.addInitScript(() => {
      window.__audioProbe = { decoded: 0, started: 0 };
      const ac = window.AudioContext || window.webkitAudioContext;
      if (ac) {
        const decode = ac.prototype.decodeAudioData;
        ac.prototype.decodeAudioData = function (...args) {
          window.__audioContext = this;
          return decode.apply(this, args).then(value => { window.__audioProbe.decoded++; return value; });
        };
      }
      const start = AudioBufferSourceNode.prototype.start;
      AudioBufferSourceNode.prototype.start = function (...args) {
        window.__audioProbe.started++;
        return start.apply(this, args);
      };
    });
    const page = await context.newPage();
    const errors = [], assetRequests = [];
    page.on('pageerror', error => errors.push(String(error)));
    page.on('console', message => { if (message.type() === 'error') errors.push(message.text()); });
    await page.route('**/assets/**', route => {
      assetRequests.push(route.request().url());
      return route.abort();
    });
    await page.goto(`${origin}/${app}/` + (app === 'showcase' ? '#resources' : ''));
    await page.waitForFunction(() => window.__resourcesTest, null, { timeout: 120000 });
    const info = await page.evaluate(() => ({
      files: __resourcesTest.bundledFiles.size, isolated: crossOriginIsolated,
      fonts: [...document.fonts].map(font => ({ family: font.family, status: font.status })),
    }));
    assert.equal(info.isolated, false);
    await verifyBytes(page, app);
    if (app === 'map') {
      await page.locator('#map_source').waitFor({ timeout: 60000 });
      await page.locator('#map_source').selectOption({ index: 8 });
      await page.waitForFunction(() => document.querySelector('#map_status')?.textContent === 'Ready',
        null, { timeout: 30000 });
      await page.waitForFunction(() => {
        const text = document.querySelector('#map_frame')?.textContent.replace(/[\u2066-\u2069]/g, '') ?? '';
        return /[1-9]\d* tiles drawn/.test(text) && text.includes('0 loading');
      });
      info.frame = await page.locator('#map_frame').textContent();
      await page.locator('#map_rotate').click();
      await page.waitForFunction(() => document.querySelector('#map_bearing')?.textContent.includes('45'));
      // A second tab must initialize independently without an exclusive resource lock.
      const second = await context.newPage();
      await second.goto(`${origin}/map/`);
      await second.locator('#map_source').waitFor();
      await second.locator('#map_source').selectOption({ index: 8 });
      await second.waitForFunction(() => document.querySelector('#map_status')?.textContent === 'Ready');
      await second.close();
    } else {
      await page.locator('#resources_numbers').waitFor({ timeout: 60000 });
      info.numbers = await page.locator('#resources_numbers').textContent();
      info.greeting = await page.locator('#resources_greeting').textContent();
      assert(info.numbers.includes('256') && info.numbers.includes('100'), info.numbers);
      assert(!info.greeting.includes('unavailable'), info.greeting);
      await page.waitForFunction(() => [...document.images].filter(image => image.src.startsWith('blob:'))
        .every(image => image.complete && image.naturalWidth > 0));
      info.images = await page.evaluate(() => [...document.images]
        .map(image => ({ src: image.src.slice(0, 5), width: image.naturalWidth })));
      assert(info.images.some(image => image.src === 'blob:' && image.width > 0));
      assert(info.fonts.length >= 3 && info.fonts.every(font => font.status === 'loaded'));
      await page.evaluate(() => location.hash = 'canvas');
      await page.waitForFunction(() => document.querySelector('#canvas_image_readout')?.textContent.match(/png/i),
        null, { timeout: 20000 });
      info.canvas = await page.locator('#canvas_image_readout').textContent();
      await page.evaluate(() => location.hash = 'speech');
      await page.locator('#sound_click').waitFor();
      await page.waitForFunction(() => window.__audioProbe.decoded >= 7, null, { timeout: 20000 });
      await page.locator('#sound_click').click();
      await page.waitForFunction(() => window.__audioProbe.started > 0 && window.__audioContext.state === 'running');
      info.audio = await page.evaluate(() => window.__audioProbe);
    }
    await page.screenshot({ path: path.join(output, `${type.name()}-${app}.png`), fullPage: true });
    report.push({ browser: type.name(), app, info, assetRequests, errors });
    assert.equal(assetRequests.length, 0, JSON.stringify(assetRequests));
    assert.equal(errors.length, 0, JSON.stringify(errors));
  } finally {
    await context.close();
  }
}

async function verifySite(browser, type, fallback, isolated = false) {
  const app = fallback ? 'fallback' : isolated ? 'isolated' : 'showcase';
  const context = await browser.newContext({
    serviceWorkers: fallback ? 'block' : 'allow', viewport: { width: 1280, height: 1000 },
  });
  const errors = [], warnings = [];
  try {
    await context.addInitScript(() => {
      window.__siteWrites = [];
      const put = Cache.prototype.put;
      Cache.prototype.put = function (request, response) {
        window.__siteWrites.push(String(request.url ?? request));
        return put.call(this, request, response);
      };
    });
    const page = await context.newPage();
    page.on('pageerror', error => errors.push(String(error)));
    page.on('console', message => {
      if (message.type() === 'error') errors.push(message.text());
      if (message.type() === 'warning') warnings.push(message.text());
    });
    await page.goto(`${origin}/${app}/#webview`);
    const frame = page.frameLocator('#webview_embedded');
    await page.waitForFunction(() => document.querySelector('#webview_embedded')?.getAttribute('src'),
      null, {timeout: 30000});
    const src = await page.locator('#webview_embedded').getAttribute('src');
    assert.equal(src.includes('__day_bundle/'), !fallback, `${src}\n${warnings.join('\n')}`);
    await frame.locator('#count').waitFor({ timeout: 30000 });
    await frame.locator('#count').click();
    await frame.getByText('Clicked 1 time', { exact: false }).waitFor();
    assert(await frame.locator('img').evaluate(image => image.complete && image.naturalWidth > 0));
    await frame.locator('a[href="pages/features.html"]').click();
    await frame.locator('a[href="../index.html"]').waitFor();
    await frame.locator('a[href="../index.html"]').click();
    await frame.locator('#count').waitFor();
    const result = { browser: type.name(), app, document: src };
    if (!fallback) {
      result.reloads = [];
      for (let reload = 0; reload < 3; reload++) {
        await page.reload();
        await page.frameLocator('#webview_embedded').locator('#count').waitFor({timeout: 30000});
        result.reloads.push(await page.evaluate(() => ({
          navigationMs: performance.getEntriesByType('navigation')[0].duration,
          readyMs: performance.now(), writes: window.__siteWrites,
        })));
        assert.equal(result.reloads.at(-1).writes.length, 0, 'warm reload must not rewrite site files');
      }
      await context.setOffline(true);
      await page.reload();
      await page.frameLocator('#webview_embedded').locator('#count').waitFor({ timeout: 30000 });
      await context.setOffline(false);
      // Dependency-contributed site and animation data must also resolve without HTTP assets.
      await page.evaluate(() => location.hash = 'lottie');
      await page.waitForFunction(() => [...document.querySelectorAll('iframe')]
        .some(frame => frame.contentDocument?.body?.dataset.dayLottie === 'playing' && frame.contentDocument.querySelector('svg')), null, { timeout: 30000 });
      result.lottie = 'playing';
      await page.screenshot({path: path.join(output, `${type.name()}-lottie.png`)});

    }
    assert.equal(errors.length, 0, JSON.stringify(errors));
    report.push(result);
  } catch (error) {
    const state = await context.pages()[0]?.evaluate(() => ({
      text: document.body.innerText,
      frames: [...document.querySelectorAll('iframe')].map(frame => {
        try { return {src: frame.src, body: frame.contentDocument?.body?.innerText, state: frame.contentWindow?.dayLottie?.state()}; }
        catch (error) { return {src: frame.src, error: String(error)}; }
      }),
    })).catch(diagnosticError => ({error: String(diagnosticError)}));
    report.push({browser: type.name(), app, failure: String(error), errors, warnings, state});
    console.error({browser: type.name(), app, errors, warnings, state});
    throw error;
  } finally {
    await context.close();
  }
}

try {
  for (const type of [chromium, webkit]) {
    const browser = await type.launch({ headless: true });
    try {
      for (const app of showcaseOnly ? ['showcase'] : ['map', 'showcase']) await verifyCore(browser, type, app);
      for (const fallback of [false, true]) await verifySite(browser, type, fallback);
      await verifySite(browser, type, false, true);
    } finally {
      await browser.close();
    }
  }
  assert(!servedAssets.some(url => !url.startsWith('/fallback/')), JSON.stringify(servedAssets));
} finally {
  server.close();
  await fs.writeFile(path.join(output, 'report.json'), JSON.stringify(report, null, 2));
  console.log(JSON.stringify(report, null, 2));
}
