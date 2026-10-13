// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// The day-sql channel (docs/persistence.md): synchronous SQLite over the day-sql worker.
//
// This is the page half of day-sqlite-worker, a web module the Day host page imports before it
// instantiates the app (`[package.metadata.day.web]`, docs/web.md). `register` supplies the
// wasm imports the crate declares; `start` boots the worker beside this file with the compiled
// app module and resolves once the channel is up or definitively absent, which the page awaits
// before the app's entry point runs, so Rust needs no async path to open a database.
//
// The worker holds databases on OPFS sync access handles; this side writes a request into a
// SharedArrayBuffer and spins the few microseconds until the reply state flips, so wasm's
// `day_sql_call` is fully synchronous. Needs cross-origin isolation (the day server sends
// COOP/COEP); without it `ready` stays false and file databases refuse loudly in Rust.

const STATE = 0, LEN = 1, TOTAL = 2;                // Int32 indices
const IDLE = 0, REQ = 1, REQ_ACK = 2, REPLY = 3, REPLY_ACK = 4, QUIT = 9;
const DATA_OFF = 16, SAB_SIZE = (1 << 22) + DATA_OFF;

let I = null, B = null, CAP = 0;                    // channel views
let ready = false, replyBuf = null;

export function register(rt) {
  return {
    day_sql_ready: () => (ready ? 1 : 0),
    // The reply is staged; a second call copies it out.
    day_sql_call: (p, n) => call(rt.bytes(p, n)),
    day_sql_reply: (p) => { rt.mem().set(replyBuf, p); replyBuf = null; },
    // Host services the engine's Rust shims import.
    day_sql_now_ms: () => Date.now(),
    day_sql_entropy: (p, n) => { crypto.getRandomValues(rt.mem().subarray(p, p + n)); },
    day_sql_log: () => 0,
    // day_sql_fs_*: the OPFS primitives the worker instance implements. The app instance
    // links them (same module) but never calls them, so every arm here just refuses.
    day_sql_fs_open: () => -1,
    day_sql_fs_read: () => -1,
    day_sql_fs_write: () => -1,
    day_sql_fs_truncate: () => -1,
    day_sql_fs_size: () => -1,
    day_sql_fs_flush: () => -1,
    day_sql_fs_close: () => 0,
    day_sql_fs_delete: () => -1,
    day_sql_fs_exists: () => 0,
    day_sql_fs_list: () => 0,
  };
}

// One compile serves both instantiations: the compiled module is structured-cloneable, so the
// worker gets it over postMessage instead of compiling the bytes again.
export async function start(rt, module) {
  if (!crossOriginIsolated || typeof SharedArrayBuffer === 'undefined') return;
  try {
    const sab = new SharedArrayBuffer(SAB_SIZE);
    const worker = new Worker(new URL('./day-sql-worker.js', import.meta.url));
    const booted = new Promise((resolve) => {
      worker.onmessage = (e) => {
        if (e.data === 'ready') { ready = true; }
        else { ready = false; console.error('day-sql worker:', e.data); }
        resolve();
      };
      setTimeout(resolve, 20000);
    });
    worker.postMessage({ sab, module });
    await booted;
    if (ready) {
      I = new Int32Array(sab);
      B = new Uint8Array(sab, DATA_OFF);
      CAP = B.length;
      // On the way out (reload, navigation), tell the worker to close its OPFS handles so
      // the next page load can acquire them immediately.
      addEventListener('pagehide', () => {
        Atomics.store(I, STATE, QUIT);
        Atomics.notify(I, STATE);
      });
    }
  } catch (e) {
    console.error('day-sql worker failed to start', e);
    ready = false;
  }
}

// Spin until the state word equals v. Bounded: a wedged worker downgrades the channel
// instead of hanging the page.
function spin(v) {
  const deadline = performance.now() + 20000;
  while (Atomics.load(I, STATE) !== v) {
    if (performance.now() > deadline) { ready = false; return false; }
  }
  return true;
}

function call(req) {
  if (!ready) return -1;
  I[TOTAL] = req.length;
  let off = 0;
  for (;;) {
    const n = Math.min(CAP, req.length - off);
    B.set(req.subarray(off, off + n));
    I[LEN] = n;
    off += n;
    Atomics.store(I, STATE, REQ);
    Atomics.notify(I, STATE);
    if (off >= req.length) break;
    if (!spin(REQ_ACK)) return -1;
  }
  if (!spin(REPLY)) return -1;
  const total = I[TOTAL];
  replyBuf = new Uint8Array(total);
  let got = 0;
  for (;;) {
    const n = I[LEN];
    replyBuf.set(B.subarray(0, n), got);
    got += n;
    if (got >= total) break;
    Atomics.store(I, STATE, REPLY_ACK);
    Atomics.notify(I, STATE);
    if (!spin(REPLY)) return -1;
  }
  Atomics.store(I, STATE, IDLE);
  Atomics.notify(I, STATE);
  return total;
}
