// The DAY_WEB_DRIVER browser for scripted web-dom runs (docs/web.md): day-cli spawns this as
//   node scripts/ci/webdom-driver.mjs <url> <control-port>
// It opens the page in headless WebKit (Playwright) and serves the driver control protocol on
// the control port: GET /screenshot → PNG of the page, GET /quit → close and exit.
//
// Playwright is resolved from DAY_WEB_DRIVER_PLAYWRIGHT (a directory whose node_modules holds
// it), else from the working directory — it is a CI/dev dependency, deliberately not vendored.
import { createRequire } from 'node:module';
import http from 'node:http';
import path from 'node:path';

const [url, controlPort] = process.argv.slice(2);
if (!url || !controlPort) {
  console.error('usage: webdom-driver.mjs <url> <control-port>');
  process.exit(2);
}

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
  console.error(`webdom-driver: playwright not found under: ${roots.join(', ')}`);
  process.exit(3);
}

const browser = await webkit.launch();
// Match the showcase's desktop window (1000×720) at 2× — the same pixel density the native
// macOS gallery captures have.
const page = await browser.newPage({
  viewport: { width: 1000, height: 720 },
  deviceScaleFactor: 2,
});
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
    await browser.close();
    process.exit(0);
  }
  res.writeHead(404, { Connection: 'close' });
  res.end();
});
server.listen(Number(controlPort), '127.0.0.1');

browser.on('disconnected', () => process.exit(1));
