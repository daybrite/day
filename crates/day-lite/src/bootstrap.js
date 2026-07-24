// day-lite bootstrap (docs/lite.md §5–§7): the ergonomic JS layer over the __day_* host
// hooks. Plain JS (no stripping). Pieces and signals are integer handles wrapped in
// classes whose marker keys (`__p` / `__s`) the host recognizes on the way back in.
"use strict";

class __Piece {
  constructor(h) { this.__p = h; }
}
for (const name of __day_modifiers) {
  __Piece.prototype[name] = function (...args) {
    __day_modify(this.__p, name, args);
    return this;
  };
}
for (const name of __day_ctors) {
  globalThis[name] = (...args) => new __Piece(__day_construct(name, args));
}

class __Sig {
  constructor(h) { this.__s = h; }
  get() { return JSON.parse(__day_sig_get(this.__s)); }
  set(v) { __day_sig_set(this.__s, v); }
  update(f) { this.set(f(this.get())); }
}
globalThis.signal = (initial) => new __Sig(__day_signal(initial));

globalThis.page = (route, builder, hooks) => __day_page(route, builder, hooks ?? {});
globalThis.App = (hooks) => __day_app(hooks ?? {});

globalThis.console = {
  log: (...a) => __day_log("log", a.map(String).join(" ")),
  info: (...a) => __day_log("log", a.map(String).join(" ")),
  warn: (...a) => __day_log("warn", a.map(String).join(" ")),
  error: (...a) => __day_log("error", a.map(String).join(" ")),
};

globalThis.setTimeout = (f, ms) => { __day_timeout(f, ms ?? 0); return 0; };
globalThis.t = (key, args) => __day_t(key, args ?? null);

globalThis.day = {
  can: (p) => __day_can(p.startsWith("day.permission.") ? p : "day.permission." + p),
  nav: {
    navigateTo: (route, params) => __day_nav("to", route, params ?? null),
    navigateBack: () => __day_nav("back", "", null),
    reLaunch: (route) => { __day_nav("relaunch", "", null); },
  },
  db: {
    migrate: (steps) => JSON.parse(__day_db("migrate", steps, null)),
    exec: (sql, params) => JSON.parse(__day_db("exec", sql, params ?? [])),
    query: (sql, params) => JSON.parse(__day_db("query", sql, params ?? [])),
  },
  fs: {
    read: (path) => JSON.parse(__day_fs("read", path, null)),
    write: (path, data) => { __day_fs("write", path, String(data)); },
    mkdir: (path) => { __day_fs("mkdir", path, null); },
    size: (path) => JSON.parse(__day_fs("size", path, null)),
    entries: (path) => JSON.parse(__day_fs("entries", path ?? "", null)),
    remove: (path, opts) => { __day_fs("remove", path, !!(opts && opts.recursive)); },
    exists: (path) => JSON.parse(__day_fs("exists", path, null)),
  },
  sys: { info: () => JSON.parse(__day_sysinfo()) },
  i18n: { t: (key, args) => __day_t(key, args ?? null) },
};

// Bridge namespaces (net, prefs, sensors, …) attach themselves under `day.` when installed.
