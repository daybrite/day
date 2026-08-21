// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The day-sql worker (docs/persistence.md, docs/web.md): SQLite over real OPFS, serving the
// main thread synchronously.
//
// This is the SECOND instantiation of the app's own wasm module — the day-sqlite-worker crate
// linked into it exports `day_sql_exec`, and everything UI-shaped is stubbed out. OPFS sync
// access handles exist only in workers like this one, so file I/O here is plain synchronous
// JS (`day_sql_fs_*` below), and the main thread reaches the engine over a SharedArrayBuffer:
// it writes a request, `Atomics.notify`, and spins the few microseconds until the reply state
// flips. Chunking makes the fixed buffer hold any size in either direction.
//
// Access handles are async to OBTAIN but sync to USE, so a POOL of `.day-sql/pool-<i>` files
// is pre-opened and database names map onto pool entries via `.day-sql/map.json` (rewritten
// synchronously through its own handle). The pool grows between requests when it runs low.

const STATE = 0, LEN = 1, TOTAL = 2;               // Int32 indices
const IDLE = 0, REQ = 1, REQ_ACK = 2, REPLY = 3, REPLY_ACK = 4, QUIT = 9;
const DATA_OFF = 16;

let I = null, B = null, CAP = 0;                    // channel views
let wasm = null;                                    // worker instance exports

const utf8enc = new TextEncoder();
const utf8dec = new TextDecoder();
const wmem = () => new Uint8Array(wasm.memory.buffer);
const wstr = (p, n) => utf8dec.decode(new Uint8Array(wasm.memory.buffer, p, n));

// ---------------------------------------------------------------------------
// The OPFS pool
// ---------------------------------------------------------------------------

// Sized ONCE at boot: the serve loop below never returns to the worker's event loop (WebKit
// does not schedule worker tasks while the page's main thread blocks in a sql call), so the
// pool cannot grow later — async handle acquisition would never resolve mid-serve. 64 entries
// ≈ dozens of documents plus their transient journals; exhaustion errors loudly as CANTOPEN.
const POOL_SIZE = 64;
let dir = null;                                     // .day-sql directory handle
let mapHandle = null;                               // map.json sync access handle
const slots = [];                                   // {handle, name: string|null, refs: number}

function readMap() {
  try {
    const size = mapHandle.getSize();
    if (size === 0) return {};
    const buf = new Uint8Array(size);
    mapHandle.read(buf, { at: 0 });
    return JSON.parse(utf8dec.decode(buf)) || {};
  } catch { return {}; }
}

function persistMap() {
  const names = {};
  slots.forEach((s, i) => { if (s.name !== null) names[s.name] = i; });
  const bytes = utf8enc.encode(JSON.stringify(names));
  mapHandle.truncate(0);
  mapHandle.write(bytes, { at: 0 });
  mapHandle.flush();
}

// WebKit materializes a handle's write path lazily, through brokering that stalls while the
// page's main thread spins inside a sql call — so touch every handle NOW, off the hot path,
// preserving any existing content.
function warm(h) {
  if (h.getSize() === 0) {
    h.write(new Uint8Array(1), { at: 0 });
    h.truncate(0);
  } else {
    const b = new Uint8Array(1);
    h.read(b, { at: 0 });
    h.write(b, { at: 0 });
  }
  h.flush();
}

// Access handles are EXCLUSIVE, and a reload's previous worker releases its handles only as
// the browser reaps it — racing that is normal, so acquisition retries briefly. A handle that
// never frees (a second live tab of the same app) exhausts the retries and boot reports dead:
// that tab runs memory-only, honestly.
async function acquireHandle(fileHandle) {
  const deadline = Date.now() + 10000;
  for (;;) {
    try {
      return await fileHandle.createSyncAccessHandle();
    } catch (e) {
      if (Date.now() > deadline) throw e;
      await new Promise((r) => setTimeout(r, 100));
    }
  }
}

async function bootPool() {
  const root = await navigator.storage.getDirectory();
  dir = await root.getDirectoryHandle('.day-sql', { create: true });
  const mapFh = await dir.getFileHandle('map.json', { create: true });
  mapHandle = await acquireHandle(mapFh);
  warm(mapHandle);
  const names = readMap();
  const top = Math.max(POOL_SIZE, ...Object.values(names).map((i) => i + 1));
  for (let i = 0; i < top; i++) {
    const fh = await dir.getFileHandle(`pool-${i}`, { create: true });
    const handle = await acquireHandle(fh);
    warm(handle);
    slots.push({ handle, name: null, refs: 0 });
  }
  for (const [name, i] of Object.entries(names)) if (slots[i]) slots[i].name = name;
}

// The ten synchronous primitives day-sqlite-worker's VFS imports.
const fsEnv = {
  day_sql_fs_open(p, n, create) {
    const name = n ? wstr(p, n) : '';
    if (name) {
      const i = slots.findIndex((s) => s.name === name);
      if (i >= 0) { slots[i].refs += 1; return i; }
      if (!create) return -1;
    }
    const i = slots.findIndex((s) => s.name === null && s.refs === 0);
    if (i < 0) return -1;                            // pool exhausted (POOL_SIZE) — CANTOPEN
    slots[i].handle.truncate(0);
    slots[i].refs = 1;
    if (name) { slots[i].name = name; persistMap(); }
    return i;
  },
  day_sql_fs_read(slot, off, p, n) {
    const s = slots[slot]; if (!s) return -1;
    try { return s.handle.read(new Uint8Array(wasm.memory.buffer, p, n), { at: off }); }
    catch { return -1; }
  },
  day_sql_fs_write(slot, off, p, n) {
    const s = slots[slot]; if (!s) return -1;
    try { return s.handle.write(new Uint8Array(wasm.memory.buffer, p, n), { at: off }) === n ? 0 : -1; }
    catch { return -1; }
  },
  day_sql_fs_truncate(slot, size) {
    const s = slots[slot]; if (!s) return -1;
    try { s.handle.truncate(size); return 0; } catch { return -1; }
  },
  day_sql_fs_size(slot) {
    const s = slots[slot]; if (!s) return -1;
    try { return s.handle.getSize(); } catch { return -1; }
  },
  day_sql_fs_flush(slot) {
    const s = slots[slot]; if (!s) return -1;
    try { s.handle.flush(); return 0; } catch { return -1; }
  },
  day_sql_fs_close(slot, del) {
    const s = slots[slot]; if (!s) return;
    s.refs = Math.max(0, s.refs - 1);
    if (del && s.refs === 0) {
      try { s.handle.truncate(0); } catch { /* stays as garbage until reuse */ }
      if (s.name !== null) { s.name = null; persistMap(); }
    }
  },
  day_sql_fs_delete(p, n) {
    const name = wstr(p, n);
    const i = slots.findIndex((s) => s.name === name);
    if (i < 0) return -1;
    try { slots[i].handle.truncate(0); } catch { /* reused later regardless */ }
    slots[i].name = null;
    persistMap();
    return 0;
  },
  day_sql_fs_exists(p, n) {
    const name = wstr(p, n);
    return slots.some((s) => s.name === name) ? 1 : 0;
  },
  day_sql_log(p, n) {
    // The engine's statement trace (driver `trace_sql`): devtools is the worker's stderr.
    console.debug('[day-sql]', wstr(p, n));
  },
  day_sql_fs_list(p, cap) {
    const joined = slots.filter((s) => s.name !== null).map((s) => s.name).join('\u001f');
    const bytes = utf8enc.encode(joined);
    if (cap === 0) return bytes.length;
    if (bytes.length > cap) return -1;
    wmem().set(bytes, p);
    return bytes.length;
  },
};

// ---------------------------------------------------------------------------
// The channel loop
// ---------------------------------------------------------------------------

// Block until the state word equals one of `vs`; answers the observed value. DELIBERATELY
// never yields to the event loop: WebKit does not schedule worker tasks (timers, message
// continuations) while the page's main thread is blocked spinning in a sql call — a wait
// loop built on them deadlocks there. Plain `Atomics.wait` needs nothing from the event
// loop; the main thread's `Atomics.notify` after every state store wakes it directly.
function waitFor(...vs) {
  for (;;) {
    const cur = Atomics.load(I, STATE);
    if (vs.includes(cur)) return cur;
    Atomics.wait(I, STATE, cur);
  }
}

function callWasm(req) {
  const p = wasm.day_sql_alloc(req.length);
  wmem().set(req, p);
  const out = wasm.day_sql_exec(p, req.length);
  const head = new DataView(wasm.memory.buffer, out, 4);
  const len = head.getUint32(0, true);
  return new Uint8Array(wasm.memory.buffer, out + 4, len).slice();
}

// The serve loop is fully synchronous and never returns (see waitFor). Everything it needs —
// the SAB, the handles, the wasm instance — was acquired before it started.
function serve() {
  for (;;) {
    // QUIT arrives from the page's pagehide: close every handle NOW, so the next page load
    // (a reload, a navigation back) can acquire them without waiting for the browser to reap
    // this thread — WebKit releases a parked worker's handles too slowly to rely on.
    if (waitFor(REQ, QUIT) === QUIT) {
      for (const s of slots) { try { s.handle.close(); } catch { /* already gone */ } }
      try { mapHandle.close(); } catch { /* already gone */ }
      return;
    }
    // Collect the (possibly chunked) request.
    const total = I[TOTAL];
    const req = new Uint8Array(total);
    let got = 0;
    for (;;) {
      const n = I[LEN];
      req.set(B.subarray(0, n), got);
      got += n;
      if (got >= total) break;
      Atomics.store(I, STATE, REQ_ACK);
      Atomics.notify(I, STATE);
      waitFor(REQ);
    }
    // Execute. A trapped instance stays trapped — every later request gets the same error,
    // and the driver surfaces it; the loop itself never dies.
    let reply;
    try {
      reply = callWasm(req);
    } catch (e) {
      // Protocol reply 255 = error, with a UTF-8 message.
      const msg = utf8enc.encode(`sql worker crashed: ${e}`);
      reply = new Uint8Array(5 + msg.length);
      reply[0] = 255;
      new DataView(reply.buffer).setUint32(1, msg.length, true);
      reply.set(msg, 5);
    }
    // Stream the reply back.
    I[TOTAL] = reply.length;
    let off = 0;
    for (;;) {
      const n = Math.min(CAP, reply.length - off);
      B.set(reply.subarray(off, off + n));
      I[LEN] = n;
      off += n;
      Atomics.store(I, STATE, REPLY);
      Atomics.notify(I, STATE);
      if (off >= reply.length) break;
      waitFor(REPLY_ACK);
    }
    // The main thread stores IDLE and may store the NEXT request's REQ before this thread
    // observes either — accepting both closes the missed-transition window (REQ implies the
    // IDLE happened); the loop top then takes the request immediately.
    waitFor(IDLE, REQ);
  }
}

onmessage = async (e) => {
  const { sab, module } = e.data;
  try {
    I = new Int32Array(sab);
    B = new Uint8Array(sab, DATA_OFF);
    CAP = B.length;
    // Stub every UI-shaped import: this instance only ever runs the engine path.
    const env = new Proxy(fsEnv, {
      get: (t, k) => t[k] ?? ((k === 'day_dom_now_ms') ? () => Date.now()
        : (k === 'day_dom_entropy') ? (p, n) => crypto.getRandomValues(new Uint8Array(wasm.memory.buffer, p, n))
          : () => 0),
    });
    // The page compiled the module once; this instantiation is cheap.
    const instantiated = WebAssembly.instantiate(module, { env });
    await bootPool();
    wasm = (await instantiated).exports;
    postMessage('ready');
    serve();
  } catch (err) {
    postMessage(`dead: ${err}`);
  }
};
