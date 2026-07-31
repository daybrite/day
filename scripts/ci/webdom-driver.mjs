// The DAY_WEB_DRIVER browser for scripted web-dom runs (docs/web.md): day-cli spawns this as
//   node scripts/ci/webdom-driver.mjs <url> <control-port>
// It opens the page headless (Playwright) and serves the driver control protocol on the
// control port: GET /screenshot → PNG of the page, GET /quit → close and exit.
//
// The engine comes from DAY_WEB_DRIVER_BROWSER (webkit | chromium | firefox), default WebKit.
// Linux CI sets chromium: Playwright's Linux WebKit (the WPE port) ships no OPFS at all, so
// day-part-fs — OPFS-only by design (docs/fs.md) — can only be exercised there under
// Chromium; macOS WebKit has OPFS and stays the local default.
//
// Playwright is resolved from DAY_WEB_DRIVER_PLAYWRIGHT (a directory whose node_modules holds
// it), else from the working directory — it is a CI/dev dependency, deliberately not vendored.
import { createRequire } from 'node:module';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';

const [url, controlPort] = process.argv.slice(2);
if (!url || !controlPort) {
  console.error('usage: webdom-driver.mjs <url> <control-port>');
  process.exit(2);
}

const browserName = process.env.DAY_WEB_DRIVER_BROWSER || 'webkit';
let browserType;
const roots = [process.env.DAY_WEB_DRIVER_PLAYWRIGHT, process.cwd()].filter(Boolean);
for (const root of roots) {
  try {
    browserType = createRequire(path.join(root, 'resolve-anchor.js'))('playwright')[browserName];
    break;
  } catch {
    /* try the next root */
  }
}
if (!browserType) {
  console.error(
    `webdom-driver: playwright (${browserName}) not found under: ${roots.join(', ')}`,
  );
  process.exit(3);
}

// A THROWAWAY persistent profile, not the default ephemeral context: WebKit gives an
// ephemeral (private-browsing-style) session no OPFS backing, so every day-part-fs operation
// fails with a generic UnknownError (playwright#18235). A fresh temp profile per run keeps
// the isolation ephemeral contexts were giving us AND real storage; removed again on /quit.
const profile = fs.mkdtempSync(path.join(os.tmpdir(), 'day-webdom-profile-'));
const dropProfile = () => {
  try {
    fs.rmSync(profile, { recursive: true, force: true });
  } catch {
    /* best-effort — the OS temp dir reaps leftovers */
  }
};
// Match the showcase's desktop window (1000×720) at 2× — the same pixel density the native
// macOS gallery captures have.
const context = await browserType.launchPersistentContext(profile, {
  viewport: { width: 1000, height: 720 },
  deviceScaleFactor: 2,
});
const page = context.pages()[0] ?? (await context.newPage());
page.on('console', (m) => {
  if (m.type() === 'error' || m.type() === 'warning') console.error(`page ${m.type()}: ${m.text()}`);
});
await page.goto(url, { waitUntil: 'load' });

const server = http.createServer(async (req, res) => {
  const route = (req.url ?? '/').split('?')[0];
  if (route === '/screenshot') {
    try {
      const png = await page.screenshot({ type: 'png' });
      res.writeHead(200, { 'Content-Type': 'image/png', 'Content-Length': png.length, Connection: 'close' });
      res.end(png);
    } catch (e) {
      res.writeHead(500, { Connection: 'close' });
      res.end(String(e));
    }
    return;
  }
  if (route === '/quit') {
    res.writeHead(200, { Connection: 'close' });
    res.end('bye');
    server.close();
    quitting = true;
    await context.close();
    dropProfile();
    process.exit(0);
  }
  res.writeHead(404, { Connection: 'close' });
  res.end();
});
server.listen(Number(controlPort), '127.0.0.1');

// An unexpected context close (crash, external kill) ends the driver with an error; the
// /quit path above closes deliberately and must not be pre-empted by this handler.
let quitting = false;
context.on('close', () => {
  if (quitting) return;
  dropProfile();
  process.exit(1);
});
