// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-dom shim — the DOM half of the web-dom backend (toolkits/day-dom/src/lib.rs is the
// Rust half; the two mirror each other's tables). Owns every real DOM call, keyed by numeric
// element ids, and calls back into wasm through a handful of exports. Plain ES module: no
// bundler, no wasm-bindgen.

let wasm = null;            // wasm exports once instantiated
const els = [null, null];   // element registry; id 1 = the day root (set in start())
let lastSetRoute = null;    // the route we last wrote to the hash (echo suppression)
const PREF_NS = 'day.pref.'; // localStorage namespace for day-part-prefs
let scriptWs = null;        // dayscript WebSocket once armed (?dayscript= token present)
let scriptOutbox = [];      // reply lines queued while the socket is still connecting
let toolbarItems = {};      // toolbar item id → its element, for targeted patches

// Fill a <datalist> with a search field's completions (docs/search.md).
function setSuggestions(dl, list) {
  dl.textContent = '';
  for (const s of list) {
    const o = document.createElement('option');
    o.value = s;
    dl.append(o);
  }
}
const httpInflight = new Map(); // request id → AbortController (day-part-http's browser arm)
const utf8 = new TextDecoder();
const utf8enc = new TextEncoder();

// devicemotion state (see day_dom_sensor_*). One listener, both kinds.
const SENSOR_GRACE_MS = 2000;
// The live geolocation watch id (0 = none).
let geoWatch = 0;

const sensorState = { started: false, startedAt: 0, saw: false, accel: null, gyro: null, timers: [0, 0, 0] };

const mem = () => new Uint8Array(wasm.memory.buffer);
const f64 = (ptr, len) => new Float64Array(wasm.memory.buffer, ptr, len);
const str = (ptr, len) => utf8.decode(new Uint8Array(wasm.memory.buffer, ptr, len));
// Copy a JS string into wasm memory at `ptr` (capacity `cap`), returning the bytes written — the
// counterpart of `str` for bridge arms that hand a value back (docs/bridge.md).
const memWrite = (ptr, cap, text) => {
  const bytes = utf8enc.encode(text);
  const n = Math.min(bytes.length, cap);
  new Uint8Array(wasm.memory.buffer, ptr, n).set(bytes.subarray(0, n));
  return n;
};

// Send a JS string into wasm: allocate, copy, return [ptr, len].
function intoWasm(s) {
  const bytes = utf8enc.encode(s);
  const ptr = wasm.day_dom_alloc(bytes.length);
  mem().set(bytes, ptr);
  return [ptr, bytes.length];
}

// ---------------------------------------------------------------------------
// Element creation (mirrors EL_* in lib.rs)
// ---------------------------------------------------------------------------

function create(kind) {
  let el;
  switch (kind) {
    case 0: el = div('day-container'); break;
    case 1: el = div('day-label'); break;
    case 2: el = document.createElement('button'); el.className = 'day-btn'; el.type = 'button'; break;
    case 3: { // switch-styled checkbox
      el = document.createElement('label'); el.className = 'day-toggle';
      const input = document.createElement('input'); input.type = 'checkbox';
      const knob = div('day-toggle-knob');
      el.append(input, knob); el.__input = input; break;
    }
    case 4: el = document.createElement('input'); el.type = 'range'; el.className = 'day-slider'; break;
    case 5: el = document.createElement('input'); el.type = 'text'; el.className = 'day-field'; break;
    case 6: el = document.createElement('textarea'); el.className = 'day-area'; break;
    case 7: el = document.createElement('select'); el.className = 'day-select'; break;
    case 8: el = document.createElement('progress'); el.className = 'day-progress'; break;
    case 9: el = div('day-spinner'); break;
    case 10: el = document.createElement('img'); el.className = 'day-img'; el.alt = ''; break;
    case 11: el = document.createElement('canvas'); el.className = 'day-canvas'; break;
    case 12: { el = div('day-scroll'); const c = div('day-scroll-content'); el.append(c); el.__content = c; break; }
    case 13: el = div('day-divider'); break;
    case 14: el = div('day-nav'); break;
    case 15: el = div('day-page'); break;
    case 16: el = div('day-navmenu'); break;
    case 18: el = div('day-cell'); break;
    case 19: el = div('day-segmented'); break;
    case 20: el = div('day-radios'); break;
    // A styled run inside a label, and the link form of one (docs/text-runs.md). Inline by
    // nature, so the browser wraps the paragraph across them as if they were plain text.
    case 21: el = document.createElement('span'); el.className = 'day-run'; break;
    case 22: el = document.createElement('a'); el.className = 'day-run day-run-link'; break;
    default: el = div('day-container');
  }
  return register(el);
}

/// Give an element its shim-side id and keep it addressable. Shared by create() and the
/// tag-name escape hatch piece renderers use.
function register(el) {
  el.__id = els.length;
  els.push(el);
  return el.__id;
}

function div(cls) { const d = document.createElement('div'); d.className = cls; return d; }

// A nav host's chrome for one presentation: sidebar+detail panes, or a back bar over a single
// detail region. Shared by the initial realize and by a re-present, so the two can never drift.
// The four presentations (docs/size-classes.md), keyed by the `mode` lib.rs sends: 0 split,
// 1 stack, 2 tabs, 3 rail. `__side` is the CHROME slot — a sidebar pane, a tab bar, or a rail —
// and always holds the same NAV_MENU element, so switching between them re-parents one node
// rather than rebuilding the rows and their listeners.
const NAV_MODES = ['split', 'stack', 'tabs', 'rail'];

function navChrome(nav, id, mode) {
  const name = NAV_MODES[mode] || 'stack';
  nav.classList.add(name);
  if (name === 'stack') {
    const bar = div('day-nav-backbar');
    const btn = document.createElement('button'); btn.className = 'day-nav-back'; btn.textContent = '‹';
    const title = div('day-nav-title');
    bar.append(btn, title); bar.style.display = 'none';
    const detail = div('day-nav-detail');
    nav.append(bar, detail);
    nav.__bar = bar; nav.__title = title; nav.__detail = detail;
    btn.addEventListener('click', () => wasm.day_dom_event(id, 14, 0, 0, 0, 0));
    return;
  }
  const side = div(name === 'split' ? 'day-nav-sidebar' : name === 'tabs' ? 'day-nav-tabbar' : 'day-nav-rail');
  const detail = div('day-nav-detail');
  // A tab bar sits BELOW the content, the way every phone puts it; a sidebar and a rail lead it.
  if (name === 'tabs') nav.append(detail, side); else nav.append(side, detail);
  nav.__side = side; nav.__detail = detail;
}

const E = (id) => els[id];
// The element that carries value/checked/listeners (the toggle wraps its input).
const V = (id) => E(id).__input || E(id);

// ---------------------------------------------------------------------------
// Imports: the DOM verbs lib.rs declares
// ---------------------------------------------------------------------------

const env = {
  // Seconds since the Unix epoch, for day-piece-datetime: wasm32-unknown-unknown has no clock of
  // its own, and `SystemTime::now()` traps there rather than failing.
  day_datetime_now_secs: () => Date.now() / 1000,

  day_dom_create: (kind) => create(kind),

  day_dom_insert(parent, child, index) {
    const p = E(parent); const target = p.__content || p;
    const el = E(child);
    const ref = target.children[index] ?? null;
    // Already where it belongs. `insertBefore` does not check: it REMOVES the node and puts it
    // back, and removing a subtree that holds the focused element blurs it. Day re-inserts a
    // child at its existing index whenever a sibling's props change, so a text field lost focus
    // on every keystroke — the caret went to <body> and only the first character landed.
    if (el === ref || (el.parentNode === target && el.nextSibling === ref)) return;
    target.insertBefore(el, ref);
  },
  day_dom_remove: (child) => E(child)?.remove(),
  day_dom_release(id) { E(id)?.remove(); els[id] = null; },

  day_dom_set_frame(id, x, y, w, h) {
    const s = E(id).style;
    s.position = 'absolute';
    s.left = x + 'px'; s.top = y + 'px';
    s.width = w + 'px'; s.height = h + 'px';
  },
  // Options and selection for a picker, a segmented control or a radio group — one verb over
  // three shapes, because each is "a list of choices with one active" and the element decides
  // how that is drawn (docs/controls.md).
  day_dom_options(id, json, len) {
    const el = E(id); const spec = JSON.parse(str(json, len));
    if (el.tagName === 'SELECT') {
      el.textContent = '';
      spec.options.forEach((o) => { const opt = document.createElement('option'); opt.textContent = o; el.append(opt); });
      el.selectedIndex = spec.selected;
      return;
    }
    el.textContent = '';
    const radios = el.classList.contains('day-radios');
    spec.options.forEach((o, i) => {
      const b = document.createElement('button'); b.type = 'button';
      b.className = radios ? 'day-radio' : 'day-seg';
      if (radios) { const dot = div('day-radio-dot'); b.append(dot, document.createTextNode(o)); }
      else b.textContent = o;
      if (i === spec.selected) b.classList.add('selected');
      b.addEventListener('click', () => {
        selectAmong(el, i);
        wasm.day_dom_event(id, 6, i, 0, 0, 0);
      });
      el.append(b);
    });
  },
  day_dom_options_select(id, idx) {
    const el = E(id);
    if (el.tagName === 'SELECT') { el.selectedIndex = idx; return; }
    selectAmong(el, idx);
  },
  day_dom_set_text(id, ptr, len) {
    const el = E(id); const t = str(ptr, len);
    if (el.tagName === 'TEXTAREA') {
      // A text area's own edit echoes back through the binding, and assigning `value` moves the
      // caret to the end — typing in the MIDDLE of existing notes would jump away after each
      // character. The echo carries the text it already has, so an equal value is nothing to do.
      if (el.value !== t) el.value = t;
    } else {
      el.textContent = t;
    }
  },
  day_dom_set_style(id, p, pl, v, vl) { E(id).style.setProperty(str(p, pl), str(v, vl)); },
  // A styled run's link (docs/text-runs.md). `owner` is the LABEL's element, since that is the
  // one Rust knows as a node — the run spans are not nodes. The anchor's own navigation is
  // cancelled: what the target does is the app's `.on_link()` call, and the default there opens
  // the URL through the same path every other backend uses.
  day_dom_link(id, owner, p, l) {
    const a = E(id); const url = str(p, l);
    a.href = url;
    a.addEventListener('click', (e) => {
      e.preventDefault();
      const [q, n] = intoWasm(url);
      wasm.day_dom_event_text(owner, 16, q, n);
    });
  },
  day_dom_set_attr(id, a, al, v, vl) {
    const el = V(id); const name = str(a, al); const val = str(v, vl);
    if (name === 'value') { el.value = val; return; }
    // The inline web view's link-policy hook (docs/webview.md) rides an attribute because a
    // piece renderer has no way to add shim code of its own.
    if (name === 'data-day-inline-base') { dayInlineHook(id, el, val); return; }
    // Boolean attrs use a marker convention from the Rust side: "" removes, "-" sets.
    if (name === 'disabled' || name === 'readonly') {
      val === '' ? el.removeAttribute(name) : el.setAttribute(name, '');
    } else el.setAttribute(name, val);
  },
  // The piece-renderer escape hatch (docs/extending.md): day-dom's own EL_* kind codes cover only
  // the built-in vocabulary, so an external piece creates its element by tag name and drives it
  // with zero-argument method calls (`play`, `pause`, `load`, …).
  day_dom_create_tag(t, tl) { return register(document.createElement(str(t, tl))); },
  // Styled-text editing (docs/texteditor.md): the markup comes from Day's own HTML serializer,
  // which escapes every character of app text, and the caret survives the swap.
  day_dom_set_html(id, p, l) { dayEditorSetHtml(V(id), str(p, l)); },
  day_dom_editor_select(id, a, b) { dayEditorSelect(V(id), a, b); },
  day_dom_call(id, m, ml) {
    const el = V(id); const name = str(m, ml);
    try { el[name]?.(); } catch (e) { console.error('day: ' + name + '()', e); }
  },
  day_dom_set_class(id, ptr, len, on) { E(id).classList.toggle(str(ptr, len), !!on); },
  day_dom_set_value(id, v) {
    const el = V(id);
    if (el.tagName === 'SELECT') el.selectedIndex = v;
    else el.value = v;
  },
  day_dom_set_checked(id, on) { V(id).checked = !!on; },

  day_dom_listen: (id, mask) => listen(id, mask),

  day_dom_measure_text(t, tl, f, fl, maxW, out) {
    let text2, font;
    if (t === 0) { // measure element `tl`'s own text and computed font
      const el = E(tl); text2 = el.textContent || ''; font = getComputedStyle(el).font;
    } else { text2 = str(t, tl); font = str(f, fl); }
    const [w, h] = measure(text2, font, maxW);
    f64(out, 2).set([w, h]);
  },
  // First text baseline from the element's top, in px, for a box `boxH` tall
  // (docs/baseline.md). The browser knows the exact metrics: a canvas TextMetrics reports the
  // font's ascent, and the element's own computed padding/border says where its text box starts
  // — so an <input> with a border reports a lower baseline than a bare <div>, which is the
  // whole point. Returns -1 when the element has no text of its own.
  day_dom_baseline(id, boxH) {
    const el = E(id);
    if (!el) return -1;
    const cs = getComputedStyle(el);
    const m = baselineMetrics(cs.font);
    if (!m) return -1;
    // Content box: what the border and padding leave for the line.
    const top = parseFloat(cs.borderTopWidth) || 0;
    const padT = parseFloat(cs.paddingTop) || 0;
    const padB = parseFloat(cs.paddingBottom) || 0;
    const borderB = parseFloat(cs.borderBottomWidth) || 0;
    const inner = Math.max(0, boxH - top - padT - padB - borderB);
    // One line, centered in the content box — the same model every control uses for its text.
    return top + padT + Math.max(0, (inner - m.line) / 2) + m.ascent;
  },
  day_dom_width: (id) => E(id).clientWidth,

  day_dom_scroll_to(id, x, y, animated) {
    E(id).scrollTo({ left: x, top: y, behavior: animated ? 'smooth' : 'instant' });
  },
  day_dom_scroll_edge(id, edge, animated) {
    const el = E(id);
    const top = edge === 0 ? 0 : el.scrollHeight;
    el.scrollTo({ top, behavior: animated ? 'smooth' : 'instant' });
  },
  day_dom_scroll_offset(id, out) { const el = E(id); f64(out, 2).set([el.scrollLeft, el.scrollTop]); },
  // Pointer-drag reorder for the emulated list (docs/list.md): the browser has no native list
  // reorder, so this fakes the affordance — lift the pressed cell, slide a gap under it (CSS
  // transitions on the other cells), autoscroll near the edges — while the DECISIONS stay
  // Day's: every hovered slot is vetted synchronously through wasm.day_dom_list_can_move (the
  // app's guard), and the drop commits through wasm.day_dom_list_move, which re-binds the cells.
  day_dom_list_reorder(id) {
    const host = E(id);
    host.classList.add('day-reorder');
    let d = null; // in-flight drag
    const cells = () => [...host.querySelectorAll('.day-cell')];
    const cleanup = () => {
      if (!d) return;
      for (const c of cells()) { c.style.transform = ''; c.classList.remove('day-drag'); }
      host.classList.remove('day-no-drop');
      clearTimeout(d.hold);
      d = null;
    };
    host.addEventListener('pointerdown', (e) => {
      const cell = e.target.closest('.day-cell');
      if (!cell || d) return;
      const rowH = cell.offsetHeight || 1;
      d = {
        cell, rowH,
        from: Math.round(cell.offsetTop / rowH),
        startY: e.clientY, startScroll: host.scrollTop,
        engaged: false, accepted: null, pid: e.pointerId,
        // Touch engages after a hold (so plain swipes still scroll); mouse/pen on first move.
        hold: e.pointerType === 'touch' ? setTimeout(() => { if (d) engage(); }, 300) : null,
      };
    });
    const engage = () => {
      d.engaged = true;
      host.setPointerCapture(d.pid);
      d.cell.classList.add('day-drag');
    };
    host.addEventListener('pointermove', (e) => {
      if (!d) return;
      const dy = (e.clientY - d.startY) + (host.scrollTop - d.startScroll);
      if (!d.engaged) {
        if (e.pointerType === 'touch') return;      // waiting for the hold timer
        if (Math.abs(dy) < 5) return;
        engage();
      }
      e.preventDefault();
      d.cell.style.transform = `translateY(${dy}px)`;
      // Autoscroll near the viewport edges so long lists are reachable.
      const r = host.getBoundingClientRect();
      if (e.clientY < r.top + 24) host.scrollTop -= 12;
      else if (e.clientY > r.bottom - 24) host.scrollTop += 12;
      // The slot under the dragged cell's center, vetted by the app's guard.
      const center = d.cell.offsetTop + dy + d.rowH / 2;
      const n = cells().length;
      const slot = Math.max(0, Math.min(n - 1, Math.floor(center / d.rowH)));
      const verdict = wasm.day_dom_list_can_move(id, d.from, slot);
      d.accepted = verdict < 0 ? null : verdict;
      host.classList.toggle('day-no-drop', d.accepted === null);
      for (const c of cells()) {
        if (c === d.cell) continue;
        const row = Math.round(c.offsetTop / d.rowH);
        let shift = 0;
        if (d.accepted !== null) {
          if (d.from < d.accepted && row > d.from && row <= d.accepted) shift = -d.rowH;
          else if (d.from > d.accepted && row >= d.accepted && row < d.from) shift = d.rowH;
        }
        c.style.transform = shift ? `translateY(${shift}px)` : '';
      }
    });
    const finish = (commit) => {
      if (!d) return;
      const { engaged, from, accepted } = d;
      cleanup();
      if (commit && engaged && accepted !== null && accepted !== from) {
        wasm.day_dom_list_move(id, from, accepted);
      }
    };
    host.addEventListener('pointerup', () => finish(true));
    host.addEventListener('pointercancel', () => finish(false));
  },
  day_dom_scroll_content(id, w, h) {
    const c = E(id).__content; if (!c) return;
    c.style.position = 'relative';
    c.style.width = w + 'px'; c.style.height = h + 'px';
  },
  day_dom_focus(id, focused) { const el = V(id); focused ? el.focus() : el.blur(); },

  day_dom_canvas_replay: (id, ops, opsLen, strs, strsLen, w, h) =>
    replay(E(id), f64(ops, opsLen), new Uint8Array(wasm.memory.buffer, strs, strsLen), w, h),

  day_dom_present: (req, json, len) => present(req, JSON.parse(str(json, len))),
  day_dom_dismiss(req) { dialogs.get(req)?.close('day-dismiss'); },

  day_dom_nav_mode(id, mode, t, tl) { navChrome(E(id), id, mode); },
  // Re-present a LIVE host after a size-class change (docs/size-classes.md). The chrome is
  // rebuilt, but the pages are not: detaching an element leaves it in `els`, so each page keeps
  // its DOM subtree — and with it every scroll offset, text selection, and focused field —
  // until Day re-homes it with `day_dom_nav_add_page`.
  day_dom_nav_present(id, mode) {
    const nav = E(id);
    nav.textContent = '';
    nav.classList.remove(...NAV_MODES);
    nav.__side = nav.__detail = nav.__bar = nav.__title = undefined;
    navChrome(nav, id, mode);
  },
  day_dom_nav_add_page(nav, page, chrome) {
    const n = E(nav);
    // A stack has no chrome slot: its rows page is the stack root and lands in the detail area.
    (chrome && n.__side ? n.__side : n.__detail).append(E(page));
  },
  day_dom_nav_back_bar(nav, visible, t, tl) {
    const n = E(nav); if (!n.__bar) return;
    n.__bar.style.display = visible ? 'flex' : 'none';
    n.__detail.classList.toggle('under-bar', !!visible);
    n.__title.textContent = str(t, tl);
  },

  day_dom_navmenu(id, json, len) {
    const el = E(id); const spec = JSON.parse(str(json, len));
    el.textContent = '';
    spec.items.forEach((item, i) => {
      const row = div('day-navmenu-row');
      if (item.icon) {
        // Template rendering, the iOS model: the icon is a MASK painted with currentColor,
        // so it follows the row's text color — light in dark mode, white when selected.
        // A row's own tint (docs/vectors.md) paints the mask with that color instead.
        const icon = div('day-navmenu-icon');
        icon.style.maskImage = `url("${item.icon}")`;
        icon.style.webkitMaskImage = `url("${item.icon}")`;
        if (item.tint) icon.style.backgroundColor = item.tint;
        row.append(icon);
      }
      const t = document.createElement('span'); t.textContent = item.title; row.append(t);
      // The trailing status glyph (docs/navigation.md), masked like the leading icon so it
      // follows the row's text color unless the app named a color that means something.
      if (item.badgeIcon) {
        const badge = div('day-navmenu-badge');
        badge.style.maskImage = `url("${item.badgeIcon}")`;
        badge.style.webkitMaskImage = `url("${item.badgeIcon}")`;
        if (item.badgeTint) badge.style.backgroundColor = item.badgeTint;
        row.append(badge);
      }
      if (i === spec.selected) row.classList.add('selected');
      row.addEventListener('click', () => wasm.day_dom_event(id, 6, i, 0, 0, 0));
      el.append(row);
    });
  },
  // --- window toolbar (docs/toolbars.md) -----------------------------------
  // The web has no window chrome, so the bar is a strip docked above the app root. One spec
  // rebuilds the whole strip; day_dom_toolbar_patch carries targeted changes so a search field
  // the user is typing in is never rebuilt out from under them.
  day_dom_toolbar(json, len) {
    const spec = JSON.parse(str(json, len));
    let bar = document.getElementById('day-toolbar');
    if (bar) bar.remove();
    if (!spec.items.length) { document.body.classList.remove('day-has-toolbar'); return; }
    bar = div('day-toolbar'); bar.id = 'day-toolbar';
    toolbarItems = {};
    let trailing = false;
    for (const it of spec.items) {
      let el = null;
      if (it.kind === '>') { trailing = true; el = div('day-toolbar-flex'); }
      else if (it.kind === '-') el = div('day-toolbar-sep');
      else if (it.kind === '_') el = div('day-toolbar-gap');
      else if (it.kind === 'L') { el = div('day-toolbar-label'); el.textContent = it.label; }
      else if (it.kind === 'G') {
        // A segmented control, reusing the picker piece's own `.day-segmented` styling so the
        // one in the bar and the one on a page are the same control.
        el = div('day-segmented day-toolbar-segmented');
        el.setAttribute('role', 'radiogroup');
        (it.segments || []).forEach((seg, n) => {
          const b = document.createElement('button');
          b.className = 'day-seg' + (n === it.selected ? ' selected' : '');
          b.type = 'button';
          b.disabled = !it.enabled;
          b.title = seg.title;
          b.setAttribute('role', 'radio');
          b.setAttribute('aria-checked', n === it.selected ? 'true' : 'false');
          b.setAttribute('aria-label', seg.title);
          if (seg.icon) {
            const ic = div('day-toolbar-icon');
            ic.style.maskImage = `url("${seg.icon}")`;
            ic.style.webkitMaskImage = `url("${seg.icon}")`;
            b.append(ic);
          } else {
            b.textContent = seg.title;
          }
          b.addEventListener('click', () => {
            if (b.classList.contains('selected')) return; // already the choice
            selectAmong(el, n);
            if (it.action) wasm.day_dom_toolbar_value(it.action, n);
          });
          el.append(b);
        });
      }
      else if (it.kind === 'F') {
        el = document.createElement('input');
        el.type = 'search'; el.className = 'day-toolbar-search';
        el.value = it.text || ''; el.placeholder = it.placeholder || '';
        el.disabled = !it.enabled;
        // A native <datalist>: the browser draws the completion popup, so the keyboard handling
        // and the styling are the platform's (docs/search.md).
        const dl = document.createElement('datalist');
        dl.id = 'day-search-suggestions';
        el.setAttribute('list', dl.id);
        el.__datalist = dl;
        setSuggestions(dl, it.suggestions || []);
        bar.append(dl);
        if (it.action) el.addEventListener('input', () => {
          const [ptr, len] = intoWasm(el.value);
          wasm.day_dom_toolbar_text(it.action, ptr, len);
        });
      } else {
        // B, T, S and M are all buttons; only their click behavior differs.
        el = document.createElement('button');
        el.className = 'day-toolbar-btn';
        el.disabled = !it.enabled;
        el.title = it.tip || it.label;
        if (it.icon) {
          const ic = div('day-toolbar-icon');
          ic.style.maskImage = `url("${it.icon}")`;
          ic.style.webkitMaskImage = `url("${it.icon}")`;
          el.append(ic);
        }
        // Icon ALONE where there is one, as every desktop toolbar does — the label stays as the
        // tooltip and the accessible name, so nothing is lost to a screen reader or a hover. An
        // item with no icon keeps its text, which is also what a desktop bar does with one.
        if (it.icon) {
          el.setAttribute('aria-label', it.label);
        } else {
          const t = document.createElement('span'); t.textContent = it.label; el.append(t);
        }
        if (it.kind === 'T') {
          el.classList.add('day-toolbar-toggle');
          el.setAttribute('aria-pressed', it.on ? 'true' : 'false');
          el.addEventListener('click', () => {
            const on = el.getAttribute('aria-pressed') !== 'true';
            el.setAttribute('aria-pressed', on ? 'true' : 'false');
            if (it.action) wasm.day_dom_toolbar_on(it.action, on ? 1 : 0);
          });
        } else if (it.kind === 'S') {
          // The sidebar toggle owns its behavior: no app action to dispatch.
          el.classList.add('day-toolbar-sidebar');
          el.setAttribute('aria-expanded', 'true');
          el.addEventListener('click', () => {
            // `env`, not `wasm`: the sidebar toggle is a shim verb Rust IMPORTS, not a wasm
            // export. Called through `wasm` it is simply undefined, and the handler threw before
            // toggling anything — which is why the button did nothing at all.
            const shown = env.day_dom_toolbar_sidebar();
            if (!shown) el.disabled = true;
            else el.setAttribute('aria-expanded',
              document.querySelector('.day-nav.split.day-sidebar-hidden') ? 'false' : 'true');
          });
        } else if (it.action) {
          el.addEventListener('click', () => wasm.day_dom_toolbar_action(it.action));
        }
      }
      if (it.id) toolbarItems[it.id] = el;
      if (trailing) el.classList.add('trailing');
      bar.append(el);
    }
    document.body.prepend(bar);
    document.body.classList.add('day-has-toolbar');
  },
  day_dom_toolbar_patch(json, len) {
    const p = JSON.parse(str(json, len));
    const el = toolbarItems[p.item];
    if (!el) return;
    if (p.text !== undefined && el.value !== p.text) el.value = p.text;
    if (p.on !== undefined) el.setAttribute('aria-pressed', p.on ? 'true' : 'false');
    // A segmented item: move the selection without firing its click handler (see the toggle
    // echo the native backends guard against).
    if (p.selected !== undefined && el.classList.contains('day-segmented')) selectAmong(el, p.selected);
    if (p.enabled !== undefined && el.classList.contains('day-segmented')) {
      [...el.children].forEach((b) => { b.disabled = !p.enabled; });
    }
    if (p.enabled !== undefined) el.disabled = !p.enabled;
    if (p.suggestions !== undefined && el.__datalist) setSuggestions(el.__datalist, p.suggestions);
  },
  // Show/hide the split nav's sidebar. 0 when this page has no split nav, which is how the
  // caller (and the dayscript duty) knows to report the item disabled.
  day_dom_toolbar_sidebar() {
    const nav = document.querySelector('.day-nav.split');
    if (!nav) return 0;
    nav.classList.toggle('day-sidebar-hidden');
    // Day frames the panes itself — the CSS only decides how much room the detail HAS, not how
    // wide its page was told to be. Report the detail's new size or the page keeps the width it
    // was given and the hidden sidebar leaves a gap. A frame later, so the class has taken
    // effect and the rect is the one the browser settled on.
    requestAnimationFrame(() => {
      const box = nav.__detail && nav.__detail.getBoundingClientRect();
      if (!box) return;
      for (const page of nav.__detail.children) {
        if (page.__id) wasm.day_dom_event(page.__id, 13, box.width, box.height, 0, 0);
      }
    });
    return 1;
  },

  day_dom_navmenu_select(id, idx) {
    [...E(id).children].forEach((row, i) => row.classList.toggle('selected', i === idx));
  },

  day_dom_set_hash(ptr, len, replace) {
    const route = str(ptr, len);
    lastSetRoute = route;
    const url = route ? '#' + route : location.pathname + location.search;
    if (replace || route === location.hash.slice(1)) history.replaceState(null, '', url);
    else if (route) location.hash = route;
    else history.pushState(null, '', url);
  },

  // Motion sensors (docs/sensors.md): the browser arm of day-part-sensors, over `devicemotion`.
  //
  // ONE listener feeds both kinds — the event carries acceleration and rotation together. The
  // magnetometer has no cross-browser API at all (Chromium's Generic Sensor `Magnetometer` is
  // flag-gated and absent from Safari and Firefox), so kind 2 is always unavailable.
  //
  // Availability can only be known in retrospect: `'DeviceMotionEvent' in window` is true on a
  // desktop browser with no hardware, so this reports "available" until a grace period passes with
  // no event, and "unavailable" after — which is the honest answer for a laptop.
  day_dom_sensor_start(kind) {
    if (kind === 2 || sensorState.started) return;
    sensorState.started = true;
    sensorState.startedAt = Date.now();
    addEventListener('devicemotion', (e) => {
      const a = e.accelerationIncludingGravity;
      if (a && a.x !== null) {
        sensorState.accel = [a.x, a.y, a.z];
        sensorState.saw = true;
      }
      const r = e.rotationRate;
      if (r && r.alpha !== null) {
        // day's gyroscope contract is rad/s about the device axes; the event reports deg/s, with
        // beta about x, gamma about y and alpha about z.
        const d = Math.PI / 180;
        sensorState.gyro = [r.beta * d, r.gamma * d, r.alpha * d];
        sensorState.saw = true;
      }
    });
  },
  /// 1 when a sample was written to `out` as three f64s, 0 when none has arrived.
  day_dom_sensor_read(kind, out) {
    const v = kind === 0 ? sensorState.accel : kind === 1 ? sensorState.gyro : null;
    if (!v) return 0;
    f64(out, 3).set(v);
    return 1;
  },
  // The feed timer. wasm32 has no threads, so the browser drives sampling: this calls the module's
  // exported day_sensors_tick(kind), which fans the newest reading out to that sensor's watchers.
  day_dom_sensor_feed(kind, ms) {
    if (sensorState.timers[kind]) return;
    sensorState.timers[kind] = setInterval(() => {
      try { wasm.day_sensors_tick(kind); } catch (e) { console.error('day: sensor tick', e); }
    }, ms);
  },
  day_dom_sensor_unfeed(kind) {
    clearInterval(sensorState.timers[kind]);
    sensorState.timers[kind] = 0;
  },
  day_dom_sensor_available(kind) {
    if (kind === 2 || typeof DeviceMotionEvent === 'undefined') return 0;
    if (sensorState.saw) return 1;
    // Not started yet, or still inside the grace period.
    return !sensorState.started || Date.now() - sensorState.startedAt < SENSOR_GRACE_MS ? 1 : 0;
  },

  // Location (docs/location.md): the browser arm of day-part-location, over
  // `navigator.geolocation.watchPosition`. The browser's API is already a subscription with an
  // error channel, so it maps almost one-to-one.
  //
  // A field the browser did not measure is `null`, which crosses to Rust as NaN — the part turns a
  // non-finite value back into `None` rather than inventing a zero.
  day_dom_geo_available() {
    return navigator.geolocation ? 1 : 0;
  },
  day_dom_geo_start(high) {
    if (geoWatch !== 0 || !navigator.geolocation) return;
    const n = (v) => (v === null || v === undefined ? NaN : v);
    geoWatch = navigator.geolocation.watchPosition(
      (p) => {
        const c = p.coords;
        wasm.day_location_fix(
          c.latitude, c.longitude, n(c.altitude), n(c.accuracy),
          n(c.altitudeAccuracy), n(c.speed), n(c.heading), n(p.timestamp),
        );
      },
      (e) => wasm.day_location_error(e.code),
      { enableHighAccuracy: high !== 0, timeout: 30000, maximumAge: 0 },
    );
  },
  day_dom_geo_stop() {
    if (geoWatch === 0) return;
    navigator.geolocation.clearWatch(geoWatch);
    geoWatch = 0;
  },

  // Preferences (docs/prefs.md): the browser arm of day-part-prefs. localStorage can throw
  // (private browsing, storage pressure) — failures report as absent/uncommitted, matching
  // the part's contract on every platform.
  day_dom_pref_set(k, kl, v, vl) {
    try { localStorage.setItem(PREF_NS + str(k, kl), str(v, vl)); return 1; }
    catch { return 0; }
  },
  day_dom_pref_get(k, kl, out, cap) {
    let v;
    try { v = localStorage.getItem(PREF_NS + str(k, kl)); } catch { v = null; }
    if (v === null) return -1;
    const bytes = utf8enc.encode(v);
    mem().set(bytes.slice(0, cap), out);
    return bytes.length;
  },
  day_dom_pref_remove(k, kl) {
    const key = PREF_NS + str(k, kl);
    try {
      const had = localStorage.getItem(key) !== null;
      localStorage.removeItem(key);
      return had ? 1 : 0;
    } catch { return 0; }
  },
  day_dom_pref_has(k, kl) {
    try { return localStorage.getItem(PREF_NS + str(k, kl)) !== null ? 1 : 0; }
    catch { return 0; }
  },

  // HTTP (docs/http.md): the browser arm of day-part-http. One fetch() per request id, with
  // an AbortController serving both day_dom_http_abort and the timeout timer (the timer
  // bounds connect + response head; the body phase is uncapped — Rust-fallback parity). The
  // completion re-enters wasm EXACTLY once per id: day_http_done, or day_http_failed with
  // kind 1 BadUrl / 2 Timeout / 3 Cancelled / 0 Io (a browser hides DNS/connect/TLS detail).
  // Headers cross as flat `u32-LE len, bytes` key/value records both ways — no JSON escaping,
  // order and duplicates preserved. Request buffers are COPIED out before the first await:
  // a day_http_alloc call may grow (and move) wasm memory under any borrowed view.
  day_dom_http_start(id, m, ml, u, ul, h, hl, b, bl, hasBody, timeoutMs) {
    const method = str(m, ml);
    let url;
    try { url = new URL(str(u, ul), document.baseURI).toString(); }
    catch { httpFail(id, 1, str(u, ul)); return; }
    const headers = new Headers();
    const hb = new Uint8Array(wasm.memory.buffer, h, hl).slice();
    const hv = new DataView(hb.buffer);
    for (let i = 0; i + 4 <= hb.length;) {
      const kl = hv.getUint32(i, true); const k = utf8.decode(hb.subarray(i + 4, i + 4 + kl)); i += 4 + kl;
      const vl = hv.getUint32(i, true); const v = utf8.decode(hb.subarray(i + 4, i + 4 + vl)); i += 4 + vl;
      headers.append(k, v);
    }
    const body = hasBody ? new Uint8Array(wasm.memory.buffer, b, bl).slice() : undefined;
    const ctl = new AbortController();
    let timedOut = false;
    const timer = setTimeout(() => { timedOut = true; ctl.abort(); }, timeoutMs);
    httpInflight.set(id, ctl);
    (async () => {
      try {
        const resp = await fetch(url, { method, headers, body, signal: ctl.signal });
        clearTimeout(timer); // head arrived — the body phase runs uncapped
        const bodyBytes = new Uint8Array(await resp.arrayBuffer());
        const recs = [];
        let hdrLen = 0;
        for (const [k, v] of resp.headers) {
          const kb = utf8enc.encode(k); const vb = utf8enc.encode(v);
          const rec = new Uint8Array(8 + kb.length + vb.length);
          const dv = new DataView(rec.buffer);
          dv.setUint32(0, kb.length, true); rec.set(kb, 4);
          dv.setUint32(4 + kb.length, vb.length, true); rec.set(vb, 8 + kb.length);
          recs.push(rec); hdrLen += rec.length;
        }
        const hdr = new Uint8Array(hdrLen);
        let off = 0;
        for (const rec of recs) { hdr.set(rec, off); off += rec.length; }
        // Allocate-then-copy per buffer, refreshing the memory view after each alloc (see
        // the note above about memory growth).
        const hp = hdr.length ? wasm.day_http_alloc(hdr.length) : 0;
        if (hdr.length) mem().set(hdr, hp);
        const bp = bodyBytes.length ? wasm.day_http_alloc(bodyBytes.length) : 0;
        if (bodyBytes.length) mem().set(bodyBytes, bp);
        wasm.day_http_done(id, resp.status, hp, hdr.length, bp, bodyBytes.length);
      } catch (e) {
        if (e && e.name === 'AbortError') httpFail(id, timedOut ? 2 : 3, '');
        else httpFail(id, 0, String((e && e.message) || e));
      } finally {
        clearTimeout(timer);
        httpInflight.delete(id);
      }
    })();
  },
  day_dom_http_abort(id) { httpInflight.get(id)?.abort(); },

  // App-local files (docs/fs.md): the browser arm of day-part-fs, stored in the Origin
  // Private File System — a real origin-scoped file hierarchy.
  // One operation per request id (op: 0 read, 1 write, 2 remove, 3 list); the completion
  // re-enters wasm EXACTLY once: day_fs_done (bytes; list joins names with \u001f,
  // directories carrying a trailing slash) or day_fs_failed (kind 1 NotFound, 2 no OPFS in
  // this context — pre-OPFS browsers and private-browsing/ephemeral sessions, which WebKit
  // gives no storage backing — 0 everything else). Request buffers are COPIED out before the
  // first await. OPFS only, no fallback store: scripted runs use a persistent browser
  // profile (scripts/ci/webdom-driver.mjs) so real OPFS is what CI exercises.
  day_dom_fs_start(id, op, p, pl, d, dl) {
    const path = str(p, pl);
    const data = new Uint8Array(wasm.memory.buffer, d, dl).slice();
    const fail = (kind, msg) => {
      const bytes = utf8enc.encode(msg);
      const mp = bytes.length ? wasm.day_fs_alloc(bytes.length) : 0;
      if (bytes.length) mem().set(bytes, mp);
      wasm.day_fs_failed(id, kind, mp, bytes.length);
    };
    const done = (bytes) => {
      const bp = bytes.length ? wasm.day_fs_alloc(bytes.length) : 0;
      if (bytes.length) mem().set(bytes, bp);
      wasm.day_fs_done(id, bp, bytes.length);
    };
    (async () => {
      if (!(navigator.storage && navigator.storage.getDirectory)) {
        fail(2, '');
        return;
      }
      try {
        done(await fsOpfs(op, path, data));
      } catch (e) {
        if (e && e.name === 'NotFoundError') fail(1, '');
        else fail(0, String((e && e.message) || e));
      }
    })();
  },

  day_dom_script_send(ptr, len) {
    const line = str(ptr, len);
    if (!scriptWs) return; // scripting not armed — nothing is listening
    if (scriptWs.readyState === WebSocket.OPEN) scriptWs.send(line);
    else scriptOutbox.push(line);
  },

  day_dom_schedule_post: () => queueMicrotask(() => wasm.day_dom_posted()),
  day_dom_schedule_delayed: (token, ms) => setTimeout(() => wasm.day_dom_delayed(token), ms),
  day_dom_request_frame: () => requestAnimationFrame((t) => wasm.day_dom_frame(t / 1000)),
  day_dom_set_title(ptr, len) { document.title = str(ptr, len); },
  // App badge (docs/badge.md). The Badging API is Chromium + Safari-for-installed-PWAs; Firefox
  // has none, so every call is feature-guarded. `count < 0` clears. The promises are ignored:
  // a rejection here (not installed, insecure context) is not something the app can act on.
  day_dom_set_app_badge(count) {
    try {
      if (count < 0) { navigator.clearAppBadge?.(); }
      else if (count === 0) { navigator.setAppBadge?.(); }   // no argument = a dot
      else { navigator.setAppBadge?.(count); }
    } catch (_) { /* unsupported or blocked — the cap already says Emulated */ }
  },
  day_dom_open_url(ptr, len) { window.open(str(ptr, len), '_blank', 'noopener'); },

  day_dom_env(k, kl, out, cap) {
    const key = str(k, kl);
    const q = new URLSearchParams(location.search);
    let v = '';
    switch (key) {
      case 'vw': v = String(root().clientWidth); break;
      case 'vh': v = String(root().clientHeight); break;
      case 'dpr': v = String(devicePixelRatio || 1); break;
      case 'dark': v = (q.get('theme') ?? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')) === 'dark' ? '1' : '0'; break;
      case 'locales': v = (q.get('locale') ? [q.get('locale')] : navigator.languages).join(','); break;
      case 'route': v = location.hash.slice(1) || q.get('route') || ''; break;
      // The browser's IANA time zone (day-part-timezone's local-zone source on web).
      // `?tz=` overrides for testing, mirroring the other reserved keys.
      case 'tz': v = q.get('tz') ?? (Intl.DateTimeFormat().resolvedOptions().timeZone || ''); break;
      default:
        // Reserved `vector:` keys answer "is NAME a bundled vector glyph?" from the list
        // the assemble step injects into the page (docs/vectors.md) — '1' or empty.
        if (key.startsWith('vector:')) {
          v = (window.__DAY_VECTORS || []).includes(key.slice(7)) ? '1' : '';
          break;
        }
        v = q.get(key) ?? '';
    }
    const bytes = utf8enc.encode(v).slice(0, cap);
    mem().set(bytes, out);
    return bytes.length;
  },
  day_dom_warn: (ptr, len) => console.warn(str(ptr, len)),
  // Wall clock for the wasm side (SystemTime::now() aborts on wasm32-unknown-unknown).
  day_dom_now_ms: () => Date.now(),

  // Appearance override (Toolkit::set_appearance): 0 light, 1 dark, 2 follow the browser.
  // Returns the effective mode so the wasm side's dark_mode cache stays truthful.
  day_dom_set_dark(mode) {
    const dark = mode === 2 ? matchMedia('(prefers-color-scheme: dark)').matches : mode === 1;
    document.documentElement.classList.toggle('dark', dark);
    return dark ? 1 : 0;
  },
};

function selectAmong(group, idx) {
  [...group.children].forEach((b, i) => b.classList.toggle('selected', i === idx));
}

// ---- day-part-fs store (see day_dom_fs_start) ---------------------------------------------

/** Resolve `path` under the origin's private OPFS root and run `op`. */
async function fsOpfs(op, path, data) {
  const root = await navigator.storage.getDirectory();
  const segs = path === '' ? [] : path.split('/');
  const walk = async (upTo, create) => {
    let dir = root;
    for (let i = 0; i < upTo; i++) dir = await dir.getDirectoryHandle(segs[i], { create });
    return dir;
  };
  if (op === 0) { // read
    const dir = await walk(segs.length - 1, false);
    const f = await (await dir.getFileHandle(segs[segs.length - 1])).getFile();
    return new Uint8Array(await f.arrayBuffer());
  }
  if (op === 1) { // write
    const dir = await walk(segs.length - 1, true);
    const fh = await dir.getFileHandle(segs[segs.length - 1], { create: true });
    const w = await fh.createWritable();
    await w.write(data);
    await w.close();
    return new Uint8Array(0);
  }
  if (op === 2) { // remove (a file, or an empty directory)
    const dir = await walk(segs.length - 1, false);
    await dir.removeEntry(segs[segs.length - 1]);
    return new Uint8Array(0);
  }
  // list ('' = the root); a missing directory lists as empty (the first-run state).
  let dir;
  try {
    dir = await walk(segs.length, false);
  } catch (e) {
    if (e && e.name === 'NotFoundError') return new Uint8Array(0);
    throw e;
  }
  const names = [];
  for await (const [name, handle] of dir.entries()) {
    names.push(handle.kind === 'directory' ? name + '/' : name);
  }
  names.sort();
  return utf8enc.encode(names.join('\u001f'));
}

// Deliver a day-part-http failure: kind per the taxonomy on day_dom_http_start above.
function httpFail(id, kind, msg) {
  const bytes = utf8enc.encode(msg);
  const p = bytes.length ? wasm.day_http_alloc(bytes.length) : 0;
  if (bytes.length) mem().set(bytes, p);
  wasm.day_http_failed(id, kind, p, bytes.length);
}

// ---------------------------------------------------------------------------
// Events (mirrors mod ev in lib.rs)
// ---------------------------------------------------------------------------

function mods(e) { return (e.ctrlKey || e.metaKey ? 1 : 0) | (e.shiftKey ? 2 : 0); }

// The inline web view's link policy (docs/webview.md). The bundled site deploys beside the app
// under assets/data/, so its iframe is SAME-ORIGIN and the shim can reach inside: on every
// document the frame loads, a capture-phase click listener resolves each followed <a> against
// the site base — in-site links navigate the frame as normal, leaving ones are cancelled and
// reported to the piece (num -1 on the Custom channel), whose Rust side runs the app's
// LinkPolicy. Re-armed per load: each navigation is a fresh document.
function dayInlineHook(id, frame, base) {
  const absBase = new URL(base, document.baseURI).href;
  frame.addEventListener('load', () => {
    let doc = null;
    try { doc = frame.contentDocument; } catch { return; } // foreign document: nothing to police
    if (!doc) return;
    doc.addEventListener('click', (e) => {
      const t = e.target;
      const a = t && t.closest ? t.closest('a[href]') : null;
      if (!a) return;
      let url;
      try { url = new URL(a.getAttribute('href'), doc.baseURI).href; } catch { return; }
      if (url.startsWith(absBase) || url === 'about:blank') return;
      e.preventDefault();
      const [p, l] = intoWasm(url);
      wasm.day_dom_piece_event(id, -1, p, l);
    }, true);
  });
}

// ---------------------------------------------------------------------------
// Styled-text editing (day-piece-texteditor, docs/texteditor.md).
//
// A `contenteditable` element IS the browser's rich text editing — IME, undo, spell-check,
// drag-and-drop and the accessibility tree all come with it, and none of them can be rebuilt in a
// canvas. What it does NOT give is a stable document model: pressing Enter inserts a <div> in one
// browser and a <p> in another, and a paste brings whatever markup it came with. So Day reads the
// DOM through ONE flattening (dayEditorText) and writes it back in ONE canonical shape (spans
// inside <p> blocks), and every offset it exchanges with Rust is a UTF-8 BYTE offset, because
// that is what a Rust `String` indexes by.
// ---------------------------------------------------------------------------

const dayEnc = new TextEncoder();
const dayBlockTags = new Set(['P', 'DIV', 'LI', 'H1', 'H2', 'H3', 'H4', 'H5', 'H6', 'BLOCKQUOTE', 'PRE']);
const dayByteLen = (s) => dayEnc.encode(s).length;

// Is this <br> the filler that makes an empty block visible, rather than a line break the user
// typed? A trailing <br> is the browser's own convention for "this block is empty".
function dayFillerBr(node) {
  return node.nodeName === 'BR' && node.nextSibling === null;
}

// The DOM ⇄ flat-text mapping, and the ONE traversal all of it goes through.
//
// Day's serializer writes ONE BLOCK PER LINE of the document, so read backwards the text is the
// blocks' text JOINED with "\n" — an empty block is an empty line and still contributes its
// separator. Inside a block a <br> is a line break too, except the filler one above.
//
// Both directions must agree byte for byte, which is why they share this walk rather than each
// having their own. When they disagreed (the locator counted only text nodes) two things broke:
// restoring a selection after a restyle landed one character further right per preceding line,
// and the first keystroke reported a text that had lost every blank line — which Day then read
// as a deletion and reflowed the paragraph runs onto the wrong lines.
//
// `onEvent(kind, node)` sees, in document order: 'break' where the text gains a "\n", 'text' for
// each text node, and 'enter' for each element (which is how a position inside an EMPTY block is
// named at all). Returning true stops the walk.
function dayEditorScan(el, onEvent) {
  let started = false; // a join has no leading separator
  let stop = false;
  const visit = (node) => {
    for (const child of node.childNodes) {
      if (stop) return;
      if (child.nodeType === 3) {
        started = true;
        stop = onEvent('text', child) === true;
      } else if (child.nodeName === 'BR') {
        if (!dayFillerBr(child)) {
          started = true;
          stop = onEvent('break', child) === true;
        }
      } else if (child.nodeType === 1) {
        if (dayBlockTags.has(child.nodeName)) {
          if (started) stop = onEvent('break', child) === true;
          started = true;
          if (stop) return;
        }
        stop = onEvent('enter', child) === true;
        if (stop) return;
        visit(child);
      }
    }
  };
  visit(el);
}

// The element's text, as Day's model spells it.
function dayEditorText(el) {
  let out = '';
  dayEditorScan(el, (kind, node) => {
    if (kind === 'text') out += node.nodeValue;
    else if (kind === 'break') out += '\n';
  });
  return out;
}

// A selection endpoint can be an ELEMENT with a CHILD INDEX rather than a text node with a
// character offset — a triple-click, or a caret sitting in an empty block. Resolve it to what it
// denotes: a text position where there is text, else the element itself, whose 'enter' names the
// empty line.
function dayEditorTextPoint(node, offset) {
  if (node.nodeType === 3) return [node, offset];
  const child = node.childNodes[offset];
  if (child && child.nodeType === 3) return [child, 0];
  if (child && child.nodeType === 1) {
    const first = document.createTreeWalker(child, NodeFilter.SHOW_TEXT).nextNode();
    if (first) return [first, 0];
    return [node, 0]; // e.g. a <br>-only block: the block's own start
  }
  // Past the last child: the end of this element's text, else the element itself.
  const walker = document.createTreeWalker(node, NodeFilter.SHOW_TEXT);
  let last = null;
  for (let n = walker.nextNode(); n; n = walker.nextNode()) last = n;
  return last ? [last, last.nodeValue.length] : [node, 0];
}

// A DOM position as a BYTE offset into the flattened text.
function dayEditorOffset(el, node, offset) {
  if (!node || !el.contains(node)) return 0;
  const [target, into] = dayEditorTextPoint(node, offset);
  let bytes = 0;
  let found = null;
  dayEditorScan(el, (kind, n) => {
    if (kind === 'break') {
      bytes += 1;
      return false;
    }
    if (kind === 'enter') {
      if (n === target) {
        found = bytes;
        return true;
      }
      return false;
    }
    if (n === target) {
      found = bytes + dayByteLen(n.nodeValue.slice(0, into));
      return true;
    }
    bytes += dayByteLen(n.nodeValue);
    return false;
  });
  return found === null ? bytes : found;
}

// The inverse: a byte offset as a [node, offset] pair to put a caret at.
function dayEditorLocate(el, target) {
  let bytes = 0;
  let hit = null;
  let emptyLine = null;
  let last = null;
  dayEditorScan(el, (kind, n) => {
    if (kind === 'break') {
      bytes += 1;
      return false;
    }
    if (kind === 'enter') {
      // A block starting exactly at the target with no text of its own IS the position — an
      // empty line, which no text node can name.
      if (bytes === target && emptyLine === null) emptyLine = [n, 0];
      return false;
    }
    const len = dayByteLen(n.nodeValue);
    // Both bounds: a target that fell on a separator belongs to the line before it, not to the
    // first text node after it.
    if (target >= bytes && target <= bytes + len) {
      const want = target - bytes;
      let acc = 0;
      for (let i = 0; i <= n.nodeValue.length; i++) {
        // Walk the code units until their byte length reaches the remainder, so an emoji or a
        // CJK character never lands a caret inside its own encoding.
        if (acc >= want) {
          hit = [n, i];
          return true;
        }
        acc = dayByteLen(n.nodeValue.slice(0, i + 1));
      }
      hit = [n, n.nodeValue.length];
      return true;
    }
    bytes += len;
    last = n;
    return false;
  });
  if (hit) return hit;
  if (emptyLine) return emptyLine;
  return last ? [last, last.nodeValue.length] : [el, 0];
}

function dayEditorSelect(el, startByte, endByte, force) {
  const sel = window.getSelection();
  if (!sel) return;
  const lo = Math.min(startByte, endByte);
  const hi = Math.max(startByte, endByte);
  if (!force) {
    if (el.__daySel && el.__daySel[0] === lo && el.__daySel[1] === hi) return;
    if (el.__dayDragging) return;
  }
  const [startNode, startOffset] = dayEditorLocate(el, startByte);
  const [endNode, endOffset] = dayEditorLocate(el, endByte);
  const range = document.createRange();
  try {
    range.setStart(startNode, startOffset);
    range.setEnd(endNode, endOffset);
  } catch { return; }
  el.__daySel = [lo, hi];
  sel.removeAllRanges();
  sel.addRange(range);
}

// Replace the markup, keeping the caret. Day sends fresh HTML on every attribute change — a
// syntax highlighter does it per keystroke — so preserving the caret here is what makes the arm
// usable at all. A rewrite during IME composition is SKIPPED: replacing the nodes mid-composition
// cancels the candidate window, and Day's next patch repaints anyway.
function dayEditorSetHtml(el, html) {
  if (el.__dayComposing) return;
  const sel = window.getSelection();
  let caret = null;
  if (sel && sel.rangeCount > 0 && el.contains(sel.anchorNode)) {
    caret = [
      dayEditorOffset(el, sel.anchorNode, sel.anchorOffset),
      dayEditorOffset(el, sel.focusNode, sel.focusOffset),
    ];
  }
  el.innerHTML = html;
  // `force`: the swap threw away the nodes the old selection pointed at, so restoring it is not
  // a redundant write even when the byte offsets are unchanged.
  if (caret) dayEditorSelect(el, caret[0], caret[1], true);
}

// The editable listener set (listen bit 512). `input` reports the flattened text; the document's
// `selectionchange` reports the caret, filtered to selections that are actually inside this
// element.
function dayEditorListen(id, el) {
  el.addEventListener('compositionstart', () => { el.__dayComposing = true; });
  el.addEventListener('compositionend', () => {
    el.__dayComposing = false;
    const [p, l] = intoWasm(dayEditorText(el));
    wasm.day_dom_event_text(id, 2, p, l);
  });
  el.addEventListener('input', () => {
    if (el.__dayComposing) return; // reported once, on compositionend
    const [p, l] = intoWasm(dayEditorText(el));
    wasm.day_dom_event_text(id, 2, p, l);
  });
  // A drag owns the selection until the button comes up — including a release outside the
  // element, which is why the end listeners are on the document.
  el.addEventListener('pointerdown', () => { el.__dayDragging = true; });
  document.addEventListener('pointerup', () => { el.__dayDragging = false; });
  document.addEventListener('pointercancel', () => { el.__dayDragging = false; });
  document.addEventListener('selectionchange', () => {
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0 || !el.contains(sel.anchorNode)) return;
    const a = dayEditorOffset(el, sel.anchorNode, sel.anchorOffset);
    const b = dayEditorOffset(el, sel.focusNode, sel.focusOffset);
    // Remembered ORDERED, because the piece reports a backwards drag ordered as well — an
    // unordered comparison would miss the echo and re-anchor the drag at its far end.
    el.__daySel = [Math.min(a, b), Math.max(a, b)];
    const [p, l] = intoWasm('sel ' + a + ' ' + b);
    wasm.day_dom_event_text(id, 17, p, l);
  });
}

function listen(id, mask) {
  const host = E(id); const el = V(id);
  if (mask & 1) el.addEventListener('click', (e) => wasm.day_dom_event(id, 1, mods(e), 0, 0, 0));
  if (mask & 2) el.addEventListener('input', () => {
    if (el.type === 'range') wasm.day_dom_event(id, 5, Number(el.value), 0, 0, 0);
    else { const [p, l] = intoWasm(el.value); wasm.day_dom_event_text(id, 2, p, l); }
  });
  if (mask & 4) el.addEventListener('change', () => {
    if (el.type === 'checkbox') wasm.day_dom_event(id, 4, el.checked ? 1 : 0, 0, 0, 0);
    else if (el.tagName === 'SELECT') wasm.day_dom_event(id, 6, el.selectedIndex, 0, 0, 0);
    // A range's `change` is the settled value: the DOM fires `input` as the thumb moves and
    // `change` once, when the user lets go (event 15 — mirrors ev::VALUE_COMMITTED in lib.rs).
    else if (el.type === 'range') wasm.day_dom_event(id, 15, Number(el.value), 0, 0, 0);
  });
  if (mask & 8) {
    el.addEventListener('focus', () => wasm.day_dom_event(id, 7, 1, 0, 0, 0));
    el.addEventListener('blur', () => wasm.day_dom_event(id, 7, 0, 0, 0, 0));
  }
  if (mask & 16) el.addEventListener('keydown', (e) => { if (e.key === 'Enter') wasm.day_dom_event(id, 3, 0, 0, 0, 0); });
  if (mask & 512) dayEditorListen(id, el);
  if (mask & 32) resizeObserver.observe(host);
  if (mask & 64) host.addEventListener('scroll', () => wasm.day_dom_event(id, 12, host.scrollLeft, host.scrollTop, 0, 0));
  if (mask & 128) host.addEventListener('pointerdown', (e) => {
    const r = host.getBoundingClientRect();
    wasm.day_dom_event(id, 8, e.clientX - r.left, e.clientY - r.top, 0, 0);
  });
  if (mask & 256) {
    let start = null;
    host.addEventListener('pointerdown', (e) => {
      host.setPointerCapture(e.pointerId);
      const r = host.getBoundingClientRect();
      start = [e.clientX, e.clientY, r.left, r.top];
      wasm.day_dom_event(id, 9, e.clientX - r.left, e.clientY - r.top, 0, 0);
    });
    host.addEventListener('pointermove', (e) => {
      if (!start) return;
      wasm.day_dom_event(id, 10, e.clientX - start[2], e.clientY - start[3], e.clientX - start[0], e.clientY - start[1]);
    });
    host.addEventListener('pointerup', (e) => {
      if (!start) return;
      wasm.day_dom_event(id, 11, e.clientX - start[2], e.clientY - start[3], e.clientX - start[0], e.clientY - start[1]);
      start = null;
    });
  }
}

const resizeObserver = new ResizeObserver((entries) => {
  if (!wasm) return;
  for (const en of entries) {
    const id = en.target.__id;
    if (id) wasm.day_dom_event(id, 13, en.contentRect.width, en.contentRect.height, 0, 0);
  }
});

// ---------------------------------------------------------------------------
// Text measurement: an offscreen node, so wrapping metrics match real labels.
// ---------------------------------------------------------------------------

let measurer = null;
// Font ascent + line height for a CSS `font` shorthand, from a canvas TextMetrics (the only
// place the browser exposes real font metrics). Cached per font string — a form asks for the
// same two or three fonts on every layout pass.
let metricsCtx = null;
const metricsCache = new Map();
function baselineMetrics(font) {
  if (!font) return null;
  const hit = metricsCache.get(font);
  if (hit !== undefined) return hit;
  if (!metricsCtx) metricsCtx = document.createElement('canvas').getContext('2d');
  metricsCtx.font = font;
  const m = metricsCtx.measureText('Hxg');
  // fontBoundingBox* is the font's own ascent/descent; actualBoundingBox* is this string's ink,
  // which would make the answer depend on which letters happen to be in the label.
  const ascent = m.fontBoundingBoxAscent;
  const descent = m.fontBoundingBoxDescent;
  const out = ascent > 0 ? { ascent, line: ascent + descent } : null;
  metricsCache.set(font, out);
  return out;
}

function measure(text, font, maxW) {
  if (!measurer) {
    measurer = div('');
    measurer.style.cssText = 'position:absolute;left:-99999px;top:0;visibility:hidden;white-space:pre-wrap;overflow-wrap:break-word;';
    document.body.append(measurer);
  }
  measurer.style.font = font;
  measurer.style.maxWidth = (maxW < 1e5 ? maxW : 100000) + 'px';
  measurer.textContent = text || ' ';
  const r = measurer.getBoundingClientRect();
  return [r.width, r.height];
}

// ---------------------------------------------------------------------------
// Canvas replay (§11): interpret the f64 op stream from encode_ops (lib.rs).
// ---------------------------------------------------------------------------

function rgba(packed) {
  const v = packed >>> 0;
  return `rgba(${(v >>> 24) & 255},${(v >>> 16) & 255},${(v >>> 8) & 255},${(v & 255) / 255})`;
}

// The region a stroke of the CURRENT lineWidth covers, as a clip path. Canvas2D exposes no
// "convert stroke to path", so this is the honest approximation available to it: clip to the
// path's own outline. A gradient stroke therefore paints the gradient across the whole path
// interior on web, which reads correctly for thin lines and diverges for very thick ones.
function strokeRegion(ctx, p) { return p; }

function replay(canvas, ops, strs, w, h) {
  const dpr = devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(w * dpr));
  canvas.height = Math.max(1, Math.round(h * dpr));
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  let i = 0;
  const next = () => ops[i++];
  const readPaint = () => {
    const kind = next();
    if (kind === 0) return rgba(next());
    if (kind === 1) {
      const g = ctx.createLinearGradient(next(), next(), next(), next());
      const n = next();
      for (let k = 0; k < n; k++) g.addColorStop(Math.min(1, Math.max(0, next())), rgba(next()));
      return g;
    }
    // radial (elliptical): unit-circle gradient scaled to (rx, ry) about the center.
    const cx = next(), cy = next(), rx = next(), ry = next(), n = next();
    const g = ctx.createRadialGradient(0, 0, 0, 0, 0, 1);
    for (let k = 0; k < n; k++) g.addColorStop(Math.min(1, Math.max(0, next())), rgba(next()));
    return { __radial: [cx, cy, Math.max(rx, 0.01), Math.max(ry, 0.01)], g };
  };
  const path = () => {
    const kind = next(); const p = new Path2D();
    if (kind === 0) p.rect(next(), next(), next(), next());
    else if (kind === 1) { const x = next(), y = next(), pw = next(), ph = next(), r = next(); p.roundRect(x, y, pw, ph, r); }
    else if (kind === 2) { const x = next(), y = next(), pw = next(), ph = next(); p.ellipse(x + pw / 2, y + ph / 2, pw / 2, ph / 2, 0, 0, Math.PI * 2); }
    else if (kind === 3) {
      const x = next(), y = next(), pw = next(), ph = next(), start = next(), sweep = next();
      p.ellipse(x + pw / 2, y + ph / 2, pw / 2, ph / 2, 0, (start * Math.PI) / 180, ((start + sweep) * Math.PI) / 180);
    } else if (kind === 4) { p.moveTo(next(), next()); p.lineTo(next(), next()); }
    else if (kind === 5) {
      const n = next();
      for (let k = 0; k < n; k++) { const x = next(), y = next(); k === 0 ? p.moveTo(x, y) : p.lineTo(x, y); }
      p.closePath();
    } else if (kind === 6) {
      // Arbitrary path: [rule, segCount, then per segment kind + points].
      p.__rule = next() === 1 ? 'evenodd' : 'nonzero';
      const n = next();
      for (let k = 0; k < n; k++) {
        const s = next();
        if (s === 0) p.moveTo(next(), next());
        else if (s === 1) p.lineTo(next(), next());
        else if (s === 2) p.quadraticCurveTo(next(), next(), next(), next());
        else if (s === 3) p.bezierCurveTo(next(), next(), next(), next(), next(), next());
        else p.closePath();
      }
    }
    return p;
  };
  while (i < ops.length) {
    const op = next();
    if (op === 0) { // fill
      const paint = readPaint(); const p = path();
      if (paint.__radial) {
        const [cx, cy, rx, ry] = paint.__radial;
        ctx.save(); ctx.clip(p); ctx.translate(cx, cy); ctx.scale(rx, ry);
        ctx.fillStyle = paint.g; ctx.fillRect(-1, -1, 2, 2); ctx.restore();
      } else { ctx.fillStyle = paint; ctx.fill(p, p.__rule || 'nonzero'); }
    } else if (op === 1) { // stroke: width, cap, join, miter, dash phase + pattern, paint, shape
      ctx.lineWidth = next();
      ctx.lineCap = ['butt', 'round', 'square'][next()] || 'butt';
      ctx.lineJoin = ['miter', 'round', 'bevel'][next()] || 'miter';
      ctx.miterLimit = next();
      ctx.lineDashOffset = next();
      const nd = next(); const dash = [];
      for (let k = 0; k < nd; k++) dash.push(next());
      ctx.setLineDash(dash);
      const paint = readPaint(); const p = path();
      if (paint.__radial) {
        // No gradient-stroke primitive: clip to the stroked region, then paint the gradient
        // through it. Canvas2D has no "stroke to path", so the clip IS the stroke geometry.
        const [cx, cy, rx, ry] = paint.__radial;
        ctx.save(); ctx.strokeStyle = '#000'; ctx.clip(strokeRegion(ctx, p));
        ctx.translate(cx, cy); ctx.scale(rx, ry);
        ctx.fillStyle = paint.g; ctx.fillRect(-1, -1, 2, 2); ctx.restore();
      } else { ctx.strokeStyle = paint; ctx.stroke(p); }
      ctx.setLineDash([]);
    } else if (op === 6) { // clip
      const p = path();
      ctx.clip(p, p.__rule || 'nonzero');
    } else if (op === 2) { // text
      ctx.fillStyle = rgba(next());
      const size = next(), anchor = next(), x = next(), y = next(), off = next(), len = next();
      ctx.font = `${size}px -apple-system, BlinkMacSystemFont, sans-serif`;
      ctx.textAlign = anchor === 1 ? 'center' : 'left';
      ctx.textBaseline = anchor === 1 ? 'middle' : 'alphabetic';
      ctx.fillText(utf8.decode(strs.slice(off, off + len)), x, y);
    } else if (op === 3) ctx.save();
    else if (op === 4) ctx.restore();
    else if (op === 5) { const a = next(), b = next(), c = next(), d = next(), e = next(), f = next(); ctx.transform(a, b, c, d, e, f); }
  }
}

// ---------------------------------------------------------------------------
// Dialogs (docs/dialogs.md): <dialog>-backed alert/confirm/sheet/prompt.
// ---------------------------------------------------------------------------

const dialogs = new Map();

function present(req, spec) {
  const dlg = document.createElement('dialog');
  dlg.className = 'day-dialog' + (spec.sheet ? ' sheet' : '');
  const title = div('day-dialog-title'); title.textContent = spec.title; dlg.append(title);
  if (spec.message) { const m = div('day-dialog-msg'); m.textContent = spec.message; dlg.append(m); }
  const answer = (which, text) => {
    dialogs.delete(req); dlg.close(); dlg.remove();
    if (text !== undefined) { const [p, l] = intoWasm(text); wasm.day_dom_present_result(req, which, p, l); }
    else wasm.day_dom_present_result(req, which, 0, 0);
  };
  if (spec.kind === 'prompt') {
    const input = document.createElement('input');
    input.type = 'text'; input.className = 'day-field'; input.placeholder = spec.placeholder; input.value = spec.initial;
    dlg.append(input);
    const rowEl = div('day-dialog-buttons');
    const cancel = document.createElement('button'); cancel.textContent = spec.cancel; cancel.className = 'day-btn';
    const ok = document.createElement('button'); ok.textContent = spec.ok; ok.className = 'day-btn prominent';
    cancel.addEventListener('click', () => answer(-1));
    ok.addEventListener('click', () => answer(0, input.value));
    rowEl.append(cancel, ok); dlg.append(rowEl);
  } else {
    const rowEl = div('day-dialog-buttons');
    spec.buttons.forEach((b, i) => {
      const btn = document.createElement('button'); btn.textContent = b.label;
      btn.className = 'day-btn' + (b.role === 'destructive' ? ' destructive' : i === spec.buttons.length - 1 && b.role !== 'cancel' ? ' prominent' : '');
      btn.addEventListener('click', () => answer(b.role === 'cancel' ? -1 : i));
      rowEl.append(btn);
    });
    dlg.append(rowEl);
  }
  dlg.addEventListener('cancel', (e) => { e.preventDefault(); answer(-1); });
  document.body.append(dlg);
  dialogs.set(req, dlg);
  dlg.showModal();
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

function root() { return document.getElementById('day-root'); }

export async function start(wasmUrl) {
  try {
    await boot(wasmUrl);
  } catch (err) {
    console.error('day: failed to start', err);
    const r = root();
    r.textContent = '';
    const msg = div('day-boot-error');
    msg.textContent = `Day could not start: ${err}. Reload the page to try again.`;
    r.append(msg);
  }
}

async function boot(wasmUrl) {
  // Register bundled fonts before first layout, so custom families measure correctly.
  try {
    const manifest = await (await fetch('assets/fonts/fonts.json')).json();
    await Promise.all(manifest.map(async ({ family, url }) => {
      const face = new FontFace(family, `url(${url})`);
      await face.load(); document.fonts.add(face);
    }));
  } catch { /* no bundled fonts */ }

  const dark = (new URLSearchParams(location.search).get('theme')
    ?? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light')) === 'dark';
  document.documentElement.classList.toggle('dark', dark);

  const r = root();
  r.__id = 1; els[1] = r;

  // Streaming instantiation needs the server to answer `application/wasm`; fall back to a
  // buffered instantiate so the page also works on static hosts with looser MIME tables.
  // daybridge (docs/bridge.md): each bridged crate's web arm ships as its own ES module beside
  // this shim, rather than being hand-written into it. `day build` lists them in
  // `window.__DAY_BRIDGES`; each exports `register(rt)` returning the imports it implements, which
  // join `env` before instantiation. A name collision between two crates is impossible — every
  // import is `day_bridge_<crate>_<fn>` — but a module that fails to load must not take the app
  // down with it, so a failure is logged and its arm simply stays unimplemented.
  for (const url of window.__DAY_BRIDGES ?? []) {
    try {
      const mod = await import(url);
      Object.assign(env, mod.register({ str, mem, memWrite }));
    } catch (e) {
      console.error(`day-bridge: ${url} failed to load`, e);
    }
  }

  let instance;
  try {
    ({ instance } = await WebAssembly.instantiateStreaming(fetch(wasmUrl), { env }));
  } catch {
    const bytes = await (await fetch(wasmUrl)).arrayBuffer();
    ({ instance } = await WebAssembly.instantiate(bytes, { env }));
  }
  wasm = instance.exports;

  new ResizeObserver(() => wasm.day_dom_resized(r.clientWidth, r.clientHeight)).observe(r);
  document.addEventListener('visibilitychange', () =>
    wasm.day_dom_lifecycle(document.visibilityState === 'visible' ? 0 : 1));
  // Hash changes we did not write ourselves (back/forward, a hand-edited URL) are route
  // requests for the app.
  window.addEventListener('hashchange', () => {
    const route = location.hash.slice(1);
    if (route === lastSetRoute) return;
    lastSetRoute = route;
    const [p, l] = intoWasm(route);
    wasm.day_dom_hash_changed(p, l);
  });

  // dayscript (docs/web.md): when the serving `day launch` session armed scripting
  // (?dayscript= token), open a same-origin WebSocket the dev server bridges to the runner's
  // TCP protocol, and pipe request lines into the engine.
  if (new URLSearchParams(location.search).get('dayscript')) {
    scriptWs = new WebSocket(`ws://${location.host}/dayscript`);
    scriptWs.addEventListener('open', () => {
      for (const line of scriptOutbox.splice(0)) scriptWs.send(line);
    });
    scriptWs.addEventListener('message', (ev) => {
      const [p, l] = intoWasm(String(ev.data));
      wasm.day_dom_script_line(p, l);
    });
  }

  wasm.day_dom_main();
}
