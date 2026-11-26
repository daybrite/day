// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// The CLI fills the content version and shell file list.
const VERSION = /*day:cache*/ "";
const PRECACHE = /*day:precache*/ [];
const ISOLATE = /*day:isolation*/ false;
const SCOPE = new URL(self.registration.scope);
const SCOPE_KEY = encodeURIComponent(SCOPE.pathname);
const SHELL_PREFIX = 'day-shell-' + SCOPE_KEY + '-';
const RESOURCE_PREFIX = 'day-resources-' + SCOPE_KEY + '-';
const CACHE = SHELL_PREFIX + VERSION;

self.addEventListener('install', event => {
  event.waitUntil((async () => {
    try {
      const cache = await caches.open(CACHE);
      await Promise.allSettled(PRECACHE.map(path => cache.add(new Request(path, {cache: 'reload'}))));
    } catch { /* Storage is optional; online loading still works. */ }
    await self.skipWaiting();
  })());
});

self.addEventListener('activate', event => {
  event.waitUntil((async () => {
    // Fetch the navigation while a stopped worker starts, rather than serializing the two.
    await self.registration.navigationPreload?.enable().catch(() => {});
    try {
      for (const key of await caches.keys()) {
        if (key.startsWith(SHELL_PREFIX) && key !== CACHE) await caches.delete(key);
      }
    } catch { /* Storage may be unavailable. */ }
    await self.clients.claim();
  })());
});

// Ask live documents which Wasm version they own. Unknown/suspended clients make cleanup
// conservative: retain caches, rather than breaking a tab that has not answered yet.
async function clientVersion(client) {
  const url = new URL(client.url);
  const match = url.pathname.slice(SCOPE.pathname.length).match(/^__day_bundle\/([a-f0-9]{64})\//);
  if (match) return match[1];
  return new Promise(resolve => {
    const channel = new MessageChannel();
    const finish = version => { clearTimeout(timer); channel.port1.close(); resolve(version); };
    const timer = setTimeout(() => finish(null), 1000);
    channel.port1.onmessage = event => finish(event.data?.version);
    client.postMessage({type: 'day-resource-version'}, [channel.port2]);
  });
}
async function cleanResources(current) {
  const clients = (await self.clients.matchAll({type: 'window', includeUncontrolled: true}))
    .filter(client => client.url.startsWith(SCOPE.href));
  const versions = await Promise.all(clients.map(clientVersion));
  if (versions.some(version => !/^[a-f0-9]{64}$/.test(version))) return;
  const live = new Set([current, ...versions].map(version => RESOURCE_PREFIX + version));
  // A new document may have appeared while the other tabs were answering. Defer that case.
  const latest = await self.clients.matchAll({type: 'window', includeUncontrolled: true});
  if (latest.some(client => client.url.startsWith(SCOPE.href) && !clients.some(old => old.id === client.id))) return;
  for (const key of await caches.keys()) {
    if (key.startsWith(RESOURCE_PREFIX) && !live.has(key)) await caches.delete(key);
  }
}
self.addEventListener('message', event => {
  if (event.data?.type === 'day-cross-origin-isolation') {
    event.ports[0]?.postMessage(ISOLATE);
    return;
  }
  if (event.data?.type === 'day-resource-protocol') {
    event.ports[0]?.postMessage(2);
    return;
  }
  if (event.data?.type === 'day-resource-cleanup' && /^[a-f0-9]{64}$/.test(event.data.version)
      && event.source?.url.startsWith(SCOPE.href)) {
    event.waitUntil(cleanResources(event.data.version).catch(() => {}));
  }
});

self.addEventListener('fetch', event => {
  const request = event.request;
  if (request.method !== 'GET') return;
  const url = new URL(request.url);
  if (url.origin !== SCOPE.origin || !url.pathname.startsWith(SCOPE.pathname)) return;
  const local = url.pathname.slice(SCOPE.pathname.length).match(/^__day_bundle\/([a-f0-9]{64})\/(assets\/data\/.*)$/);
  if (local) {
    // Navigation preload is for online shell requests. Consume/cancel it for virtual sites.
    event.waitUntil(Promise.resolve(event.preloadResponse).then(response => response?.body?.cancel()).catch(() => {}));
    event.respondWith((async () => {
      const cache = await caches.open(RESOURCE_PREFIX + local[1]);
      return await cache.match(new URL(local[2], SCOPE).href, {ignoreSearch: true})
        ?? new Response(null, {status: 404});
    })());
    return;
  }
  const network = Promise.resolve(event.preloadResponse).then(preloaded => preloaded || fetch(request));
  // The immutable shell snapshot was cached at installation. Rewriting app.wasm on
  // every online reload duplicates its transfer into browser storage and can contend with
  // navigation. A changed build installs a new snapshot; successful fetches need no storage.
  event.respondWith(network.catch(async error => {
    const cache = await caches.open(CACHE);
    const hit = await cache.match(request, {ignoreSearch: true});
    if (hit) return hit;
    if (request.mode === 'navigate') {
      const index = await cache.match(new URL('index.html', SCOPE).href);
      if (index) return index;
    }
    throw error;
  }).then(response => {
    // Only the top-level shell and the scripts that start dedicated/shared workers need
    // these policies. Leave virtual documents, cross-origin requests and redirects alone.
    if (!ISOLATE || !response.ok || response.redirected || response.type === 'opaque'
        || !['document', 'worker', 'sharedworker'].includes(request.destination)) return response;
    const headers = new Headers(response.headers);
    headers.set('Cross-Origin-Opener-Policy', 'same-origin');
    headers.set('Cross-Origin-Embedder-Policy', 'require-corp');
    return new Response(response.body, {status: response.status, statusText: response.statusText, headers});
  }));
});
