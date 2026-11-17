// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Deterministic regressions against the shipped host and worker, without sibling repositories.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const shim = fs.readFileSync(new URL('../../crates/day-cli/resources/web/shim.js', import.meta.url), 'utf8');
const worker = fs.readFileSync(new URL('../../crates/day-cli/resources/web/service-worker.js', import.meta.url), 'utf8');
const version = 'a'.repeat(64), oldVersion = 'b'.repeat(64);
const scope = 'https://fixture.invalid/app/';
const prefix = 'day-resources-%2Fapp%2F-';
const deferred = () => { let resolve; const promise = new Promise(r => resolve = r); return {promise, resolve}; };
function storage() {
  const stores = new Map(), writes = [];
  const key = value => String(value.url ?? value);
  return {stores, writes,
    async open(name) {
      if (!stores.has(name)) stores.set(name, new Map());
      const store = stores.get(name);
      return {
        async match(request) { return store.get(key(request))?.clone(); },
        async put(request, response) { writes.push(key(request)); store.set(key(request), response.clone()); },
      };
    },
    async keys() { return [...stores.keys()]; },
    async delete(name) { return stores.delete(name); },
  };
}
function host(caches = storage(), overrides = {}) {
  const messages = [];
  const context = vm.createContext({
    TextEncoder, TextDecoder, Blob, URL, URLSearchParams, Response, MessageChannel, setTimeout, clearTimeout,
    document: {baseURI: scope, addEventListener() {}}, ResizeObserver: class {},
    window: {__DAY_BUNDLE: version}, location: new URL(scope), console: {warn() {}}, caches,
    navigator: {serviceWorker: {
      addEventListener() {},
      async register() { return {active: {state: 'activated', postMessage(data, ports) {
        if (data.type === 'day-resource-protocol') { ports[0].postMessage(2); ports[0].close(); }
        else messages.push(data);
      }}}; },
    }}, ...overrides,
  });
  vm.runInContext(shim.replace('export async function start', 'async function start') + `
    wasm = {memory: {buffer: new ArrayBuffer(64)}};
    globalThis.fixture = {
      seed(names) { for (const name of names) bundledFiles.set(name, [0, 1]); },
      install: installResources, prepare: prepareResourceSite, url: resourceSiteUrl,
      source: setResourceFrameSource, image: resourceUrl,
    };
  `, context);
  return {context, api: context.fixture, caches, messages};
}
function service(caches = storage(), extra = {}) {
  const handlers = {}, clients = [];
  const context = vm.createContext({
    URL, Request, Response, MessageChannel, setTimeout, clearTimeout, caches,
    fetch: async () => new Response('network'),
    self: {registration: {scope}, addEventListener: (type, listener) => handlers[type] = listener,
      clients: {matchAll: async () => clients}},
    ...extra,
  });
  vm.runInContext(worker, context);
  return {handlers, clients, caches, fetch(request, preloadResponse) {
    let response; const pending = [];
    handlers.fetch({request, preloadResponse, waitUntil: value => pending.push(value), respondWith: value => response = value});
    return {response, done: () => Promise.all(pending)};
  }, clean() {
    let pending;
    handlers.message({data: {type: 'day-resource-cleanup', version}, source: {url: scope}, waitUntil: value => pending = value});
    return pending;
  }};
}
function basic(text) { const response = new Response(text); Object.defineProperty(response, 'type', {value: 'basic'}); return response; }

test('online responses never touch storage or copy the Wasm response', {timeout: 1000}, async () => {
  let opened = false;
  const caches = {open() { opened = true; return new Promise(() => {}); }};
  const response = basic('fresh');
  response.clone = () => assert.fail('online response should not be cloned');
  const sw = service(caches, {fetch: async () => response});
  const result = sw.fetch(new Request(scope + 'app.wasm'));
  assert.equal(await (await result.response).text(), 'fresh');
  assert.equal(opened, false); await result.done();
});
test('unavailable CacheStorage cannot break online loading', async () => {
  const sw = service({open: async () => { throw Error('storage disabled'); }}, {fetch: async () => basic('fresh')});
  const result = sw.fetch(new Request(scope));
  assert.equal(await (await result.response).text(), 'fresh'); await result.done();
});
test('navigation preload is used without a duplicate fetch', async () => {
  const sw = service(storage(), {fetch: () => assert.fail('duplicate fetch')});
  const result = sw.fetch(new Request(scope), Promise.resolve(basic('preloaded')));
  assert.equal(await (await result.response).text(), 'preloaded'); await result.done();
});
test('offline network failure falls back to the shell cache', async () => {
  const caches = storage();
  await (await caches.open('day-shell-%2Fapp%2F-')).put(scope + 'app.wasm', new Response('cached'));
  const sw = service(caches, {fetch: async () => { throw Error('offline'); }});
  const result = sw.fetch(new Request(scope + 'app.wasm'));
  assert.equal(await (await result.response).text(), 'cached'); await result.done();
});
test('apps without generated resource exports still initialize', async () => {
  const {api} = host(); await api.install();
  assert.equal(api.image('assets/images/fixture.png'), 'assets/images/fixture.png');
});
test('boot never waits for site storage; only selected site subtrees are cached once', async () => {
  const caches = storage(), first = host(caches);
  const names = ['assets/data/site/index.html', 'assets/data/site/pages/next.html', 'assets/data/site/image.png', 'assets/data/other/index.html', 'assets/data/large.bin'];
  first.api.seed(names); await first.api.install();
  assert.equal(caches.writes.length, 0);
  assert.match(await first.api.prepare('assets/data/site/index.html'), /__day_bundle/);
  assert.equal(caches.writes.length, 4); // Three site files and its completion marker.
  const html = await (await caches.open(prefix + version)).match(scope + 'assets/data/site/index.html');
  assert.equal(html.headers.get('Cross-Origin-Embedder-Policy'), 'require-corp');
  assert(!caches.writes.some(name => name.includes('/other/') || name.endsWith('large.bin')));
  const reloaded = host(caches); reloaded.api.seed(names); await reloaded.api.install();
  await reloaded.api.prepare('assets/data/site/pages/next.html');
  assert.equal(caches.writes.length, 4, 'no file is rewritten on reload');
});
test('partial population retries; storage denial uses published URLs', async () => {
  const caches = storage();
  await (await caches.open(prefix + version)).put(scope + 'assets/data/site/index.html', new Response('partial'));
  const h = host(caches); h.api.seed(['assets/data/site/index.html']); await h.api.install();
  await h.api.prepare('assets/data/site/index.html');
  assert.equal(caches.writes.length, 3);
  const blocked = host({open: async () => { throw Error('denied'); }});
  blocked.api.seed(['assets/data/site/index.html']); await blocked.api.install();
  assert.equal(await blocked.api.prepare('assets/data/site/index.html'), 'assets/data/site/index.html');
});
test('a delayed site cannot overwrite a newer iframe navigation or a released frame', async () => {
  const gate = deferred(), caches = storage();
  const h = host({...caches, open: async name => { await gate.promise; return caches.open(name); }});
  h.api.seed(['assets/data/site/index.html']); await h.api.install();
  const frame = {isConnected: true, setAttribute(_, value) { this.src = value; }};
  h.api.source(frame, 'assets/data/site/index.html'); h.api.source(frame, 'https://fixture.invalid/new');
  gate.resolve(); await h.api.prepare('assets/data/site/index.html');
  assert.equal(frame.src, 'https://fixture.invalid/new');
  frame.isConnected = false; h.api.source(frame, 'assets/data/site/index.html');
  await new Promise(resolve => setTimeout(resolve, 0));
  assert.equal(frame.src, 'https://fixture.invalid/new');
});
test('cleanup retains versions in other tabs and removes them after those tabs close', async () => {
  const caches = storage();
  for (const key of [prefix + version, prefix + oldVersion, 'day-resources-%2Fother%2F-' + oldVersion]) await caches.open(key);
  const sw = service(caches);
  sw.clients.push({id: 'old-tab', url: scope + '__day_bundle/' + oldVersion + '/assets/data/site/index.html'});
  await sw.clean(); assert(caches.stores.has(prefix + oldVersion));
  sw.clients.pop(); await sw.clean();
  assert(!caches.stores.has(prefix + oldVersion));
  assert(caches.stores.has(prefix + version));
  assert(caches.stores.has('day-resources-%2Fother%2F-' + oldVersion));
});
test('cleanup asks app tabs for their version and retains caches if a tab is suspended', async () => {
  const caches = storage(); await caches.open(prefix + oldVersion);
  const sw = service(caches);
  sw.clients.push({id: 'app', url: scope, postMessage(_message, ports) { ports[0].postMessage({version: oldVersion}); ports[0].close(); }});
  await sw.clean(); assert(caches.stores.has(prefix + oldVersion));
  sw.clients[0].postMessage = (_message, ports) => ports[0].close();
  await sw.clean(); assert(caches.stores.has(prefix + oldVersion));
});

test('declared AssetDir includes sibling files when the entry HTML is nested', async () => {
  const h = host();
  h.api.seed(['assets/data/site/pages/index.html', 'assets/data/site/shared.css', 'assets/data/unrelated.bin']);
  await h.api.install();
  assert.match(await h.api.prepare('assets/data/site/pages/index.html', 'assets/data/site/'), /__day_bundle/);
  assert(h.caches.writes.includes(scope + 'assets/data/site/shared.css'));
  assert(!h.caches.writes.includes(scope + 'assets/data/unrelated.bin'));
  assert.match(h.api.url('assets/data/site/'), /__day_bundle/);
});

test('an active compatible worker serves sites while an update is waiting', {timeout: 1000}, async () => {
  const active = {state: 'activated', postMessage(data, ports) {
    if (data.type === 'day-resource-protocol') { ports[0].postMessage(2); ports[0].close(); }
  }};
  const h = host(storage(), {navigator: {serviceWorker: {
    addEventListener() {}, async register() { return {active, waiting: {
      state: 'installed', addEventListener() { assert.fail('must not wait for an update'); },
    }}; },
  }}});
  h.api.seed(['assets/data/site/index.html']); await h.api.install();
  assert.match(await h.api.prepare('assets/data/site/index.html'), /__day_bundle/);
});

test('a worker already in registration.active may still be activating', {timeout: 1000}, async () => {
  const listeners = new Set();
  const active = {
    state: 'activating',
    addEventListener(type, listener) {
      assert.equal(type, 'statechange');
      listeners.add(listener);
      queueMicrotask(() => { this.state = 'activated'; listener(); });
    },
    removeEventListener(_type, listener) { listeners.delete(listener); },
    postMessage(data, ports) {
      assert.equal(this.state, 'activated');
      if (data.type === 'day-resource-protocol') { ports[0].postMessage(2); ports[0].close(); }
    },
  };
  const h = host(storage(), {navigator: {serviceWorker: {
    addEventListener() {}, async register() { return {active, installing: null, waiting: null}; },
  }}});
  h.api.seed(['assets/data/site/index.html']); await h.api.install();
  assert.match(await h.api.prepare('assets/data/site/index.html'), /__day_bundle/);
  assert.equal(active.state, 'activated');
  assert.equal(listeners.size, 0);
});
