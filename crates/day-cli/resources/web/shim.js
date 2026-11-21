// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// day-toolkit-dom shim: the DOM half of the web-dom backend (crates/day-toolkit-dom/src/lib.rs is the
// Rust half; the two mirror each other's tables). Owns every DOM call, keyed by numeric
// element ids, and calls back into wasm through a handful of exports. A plain ES module with
// no bundler and no wasm-bindgen.

let wasm = null;            // wasm exports once instantiated
const els = [null, null];   // element registry; id 1 = the day root (set in start())
let lastSetRoute = null;    // the route we last wrote to the hash (echo suppression)
// The user's reduce-motion setting (docs/accessibility.md): read by `day_dom_env('reduce_motion')`
// and watched from start(), one query for both so the read and the change agree. Made on first
// use rather than at load, like the color-scheme queries, so the module still evaluates where
// there is no window (the node shim tests, scripts/ci/webdom-*-test.mjs).
let reduceMotionQuery = null;
function reduceMotion() {
  if (!reduceMotionQuery) reduceMotionQuery = matchMedia('(prefers-reduced-motion: reduce)');
  return reduceMotionQuery;
}
// The browser's light/dark preference, made on first use like the query above, and the page's
// appearance mode as `day_dom_set_dark` takes it: 0 light, 1 dark, 2 follow the browser.
let prefersDarkQuery = null;
function prefersDark() {
  if (!prefersDarkQuery) prefersDarkQuery = matchMedia('(prefers-color-scheme: dark)');
  return prefersDarkQuery;
}
let appearanceMode = 2;
// Show one appearance: day.css keys its palette on `html.dark`, and `color-scheme` picks the
// side of every `light-dark()` ink and the browser's own parts (scroll bars, form-control
// internals, the caret). Returns 1 for dark.
function showAppearance(dark) {
  document.documentElement.classList.toggle('dark', dark);
  document.documentElement.style.colorScheme = dark ? 'dark' : 'light';
  return dark ? 1 : 0;
}
const PREF_NS = 'day.pref.'; // localStorage namespace for day-part-preferences
let scriptWs = null;        // dayscript WebSocket once armed (?dayscript= token present)
let appStarted = false;     // day_dom_main has run; script lines before that queue in the inbox
const scriptInbox = [];
let scriptOutbox = [];      // reply lines queued while the socket is still connecting
// A docked toolbar strip (docs/toolbars.md): the bar element, its tracks by column, every item
// in Day's order with its element, the column it asked for and the track it sits in, and the
// items by id for targeted patches. One per window: the page's strip docks above `#day-root`;
// an emulated cover's (docs/cover.md) docks over the cover, right after it in the DOM so the
// two paint in the cover's own order, and the cover moves down by the strip's height so the
// frame Day lays the cover out in comes back that much shorter. `host` is the cover element,
// or null for the page.
function newStrip(host) {
  const strip = { host, bar: null, tracks: {}, order: [], items: {}, menuBtn: null, nav: 0 };
  // Follows the split's panes, so the tracks keep their widths as a pane is dragged, collapsed
  // or the window resized.
  strip.panes = new ResizeObserver(() => layoutToolbar(strip));
  return strip;
}
const pageStrip = newStrip(null);
const coverStrips = new Map(); // cover element → its strip
// The strip a toolbar edit addresses: the window root Day names is a cover element for a
// cover-tier window (its toolbar lives with it), anything else means the page.
function stripFor(hostId) {
  const el = els[hostId];
  if (!el || !el.classList || !el.classList.contains('day-cover')) return pageStrip;
  let strip = coverStrips.get(el);
  if (!strip) { strip = newStrip(el); coverStrips.set(el, strip); }
  return strip;
}
const coverStripOf = (el) => { const c = el.closest('.day-cover'); return (c && coverStrips.get(c)) || null; };
// The emulated app menu's button (docs/menus.md): last in the page strip's detail track,
// outside its `order` so Day's item indexes stay its own, and the strip stays up for it alone.
// Its accelerators (docs/menus.md "Keyboard shortcuts"): every enabled item with a shortcut
// the browser does not already route, matched on the document's capture phase so the menu
// answers before the focused widget, as a menu bar's key equivalents do.
let appMenuKeys = [];
const isMac = () => typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform);
const TOOLBAR_COLUMNS = ['sidebar', 'list', 'detail'];

// Dock a strip. Three tracks, one per column of a split window (docs/toolbars.md). Day draws
// this strip itself, so it can do what a desktop toolbar does: the sidebar's commands over the
// sidebar, the content list's over the list, the rest over the detail. A cover's strip is also
// where a navigation stack inside the cover puts its bar (`applyNavBar`): back button, title
// and the window's items share one bar, the way a phone's modal navigation bar does.
function openToolbar(strip) {
  strip.bar = div('day-toolbar');
  strip.tracks = {};
  if (strip.host) {
    strip.bar.classList.add('day-cover-toolbar');
    const back = document.createElement('button');
    back.className = 'day-nav-back'; back.textContent = '‹'; back.style.display = 'none';
    back.addEventListener('click', () => { if (strip.nav) wasm.day_dom_event(strip.nav, 14, 0, 0, 0, 0); });
    const title = div('day-toolbar-title');
    strip.back = back; strip.title = title;
    strip.bar.append(back, title);
    strip.host.after(strip.bar);
    strip.host.classList.add('day-has-toolbar');
    for (const nav of strip.host.querySelectorAll('.day-nav.stack')) applyNavBar(nav);
  } else {
    strip.bar.id = 'day-toolbar';
    document.body.prepend(strip.bar);
    document.body.classList.add('day-has-toolbar');
  }
  layoutToolbar(strip);
}
// Take the strip down once nothing is left on it: no toolbar item and no app menu button. A
// stack that had moved its bar into a cover's strip gets its own bar back.
function closeToolbarIfEmpty(strip) {
  if (!strip.bar || strip.order.length || strip.menuBtn) return;
  strip.panes.disconnect();
  strip.bar.remove();
  strip.bar = null;
  strip.tracks = {};
  if (strip.host) {
    strip.host.classList.remove('day-has-toolbar');
    if (strip.card) strip.card.classList.remove('day-sheet-card');
    strip.card = null;
    coverStrips.delete(strip.host);
    for (const nav of strip.host.querySelectorAll('.day-nav.stack')) applyNavBar(nav);
  } else {
    document.body.classList.remove('day-has-toolbar');
  }
}
// A cover leaving the document takes its strip with it, whatever order its items' removals
// arrive in.
function dropCoverStrip(el) {
  const strip = el && coverStrips.get(el);
  if (!strip) return;
  strip.order = []; strip.items = {}; strip.menuBtn = null;
  closeToolbarIfEmpty(strip);
}

// The track column `col`'s items go in right now. A column with no pane of its own (no split, a
// composed content list, a collapsed sidebar) has no width to align to, so its commands join the
// detail's track rather than claiming a strip of their own.
function toolbarColumn(strip, col) {
  const widths = strip.bar.__widths || {};
  return (col === 'sidebar' || col === 'list') && widths[col] ? col : 'detail';
}

function toolbarTrack(strip, col) {
  col = toolbarColumn(strip, col);
  if (!strip.tracks[col]) {
    const t = div('day-toolbar-track day-toolbar-' + col);
    // In column order, whatever order the tracks are first needed in.
    const next = TOOLBAR_COLUMNS.slice(TOOLBAR_COLUMNS.indexOf(col) + 1)
      .map((c) => strip.tracks[c]).find(Boolean);
    if (next) next.before(t); else strip.bar.append(t);
    strip.tracks[col] = t;
  }
  return strip.tracks[col];
}

// Fit a strip to its window as it is now: measure the split's panes, size the tracks to them,
// and put each item in the track its column now maps to. Only an item whose track changed moves
// (a pane appearing or collapsing), in Day's order, and the keyboard focus goes back to the
// element that had it; everything else stays exactly where it is.
function layoutToolbar(strip) {
  if (!strip.bar) return;
  if (strip.host) dockSheetStrip(strip);
  const nav = (strip.host || document).querySelector('.day-nav.split');
  const pane = (sel) => nav && nav.querySelector(sel);
  const widthOf = (el) => (el && el.offsetParent !== null ? Math.round(el.getBoundingClientRect().width) : 0);
  const side = pane('.day-nav-sidebar'), list = pane('.day-nav-list');
  strip.bar.__widths = { sidebar: widthOf(side), list: widthOf(list) };
  if (strip.bar.__nav !== nav) {
    strip.panes.disconnect();
    for (const el of [side, list]) if (el) strip.panes.observe(el);
    strip.bar.__nav = nav;
  }
  const focused = document.activeElement;
  for (const e of strip.order) {
    const track = toolbarTrack(strip, e.col);
    if (e.track !== track) { e.track = track; e.moved = true; }
  }
  for (const col of TOOLBAR_COLUMNS) {
    const t = strip.tracks[col];
    if (!t) continue;
    let prev = null;
    for (const e of strip.order) {
      if (e.track !== t) continue;
      if (e.moved || (prev ? prev.nextSibling !== e.el : t.firstChild !== e.el)) {
        if (prev) prev.after(e.el); else t.prepend(e.el);
        e.moved = false;
      }
      prev = e.el;
    }
    const w = strip.bar.__widths[col];
    t.style.flex = col !== 'detail' && w ? `0 0 ${w}px` : '';
    // A column that has lost its pane has nothing left in it.
    if (col !== 'detail' && !w && !t.firstChild) { t.remove(); delete strip.tracks[col]; }
  }
  if (focused && focused !== document.activeElement && focused.isConnected) focused.focus();
}
// A composed modal sheet's strip (docs/cover.md "Modal sheet") is the card's own header, not a
// bar across the window: back button, title and Done sit on the panel they act on. The card
// grows a band of the strip's height over its top edge (`.day-sheet-card`, day.css) and the
// strip lies on that band, so the page under the sheet stays frosted edge to edge. The cover
// leaves the same height off the bottom of the frame it reports, which keeps the card and its
// band centered together. A compact window presents the full cover instead, and the strip goes
// back across the top.
function sheetCard(cover) {
  if (!cover.classList.contains('day-sheet')) return null;
  const scrim = cover.querySelector('.day-scrim');
  return scrim ? scrim.nextElementSibling : null;
}
function dockSheetStrip(strip) {
  const card = sheetCard(strip.host);
  const was = strip.card;
  if (was && was !== card) was.classList.remove('day-sheet-card');
  strip.card = card;
  strip.bar.classList.toggle('day-sheet-strip', !!card);
  if (!card) {
    strip.bar.style.left = strip.bar.style.top = strip.bar.style.width = '';
    return;
  }
  card.classList.add('day-sheet-card');
  const r = card.getBoundingClientRect();
  strip.bar.style.left = `${r.left}px`;
  strip.bar.style.top = `${r.top}px`;
  strip.bar.style.width = `${r.width}px`;
}
// A frame Day gives a sheet's card moves its strip with it.
function followSheetCard(el) {
  const prev = el.previousElementSibling;
  if (!prev || !prev.classList.contains('day-scrim')) return;
  const cover = el.closest('.day-cover');
  const strip = cover && coverStrips.get(cover);
  if (strip && strip.bar) dockSheetStrip(strip);
}

// Every strip, after something that can change a split's panes (a nav re-present).
function layoutToolbars() {
  layoutToolbar(pageStrip);
  for (const strip of coverStrips.values()) layoutToolbar(strip);
}

// Where a stack's bar goes (docs/navigation.md): its own back bar over the detail, or, inside
// a cover that has a strip, that strip — the back button shows while pages are pushed, the
// title is the top page's, and the cover's own items (a Done button) keep their trailing edge.
// `__barState` is what Day last sent, so a strip arriving after the stack can take the bar over,
// and a strip leaving can hand it back.
function applyNavBar(n) {
  const st = n.__barState;
  if (!st || !n.__bar) return;
  const strip = coverStripOf(n);
  if (strip && strip.bar) {
    n.__bar.style.display = 'none';
    n.__detail.classList.remove('under-bar');
    strip.nav = n.__id;
    strip.back.style.display = st.visible ? '' : 'none';
    strip.title.textContent = st.title;
    return;
  }
  n.__bar.style.display = st.visible ? 'flex' : 'none';
  n.__detail.classList.toggle('under-bar', !!st.visible);
  n.__title.textContent = st.title;
}

// The element for one toolbar item.
// A nav row's content: the leading icon, the title, the count and the trailing status glyph.
// Shared by the full build and the in-place refresh (day_dom_navmenu_rows).
function fillNavRow(row, item) {
  if (item.icon) {
    // Template rendering, the iOS model: the icon is a mask painted with currentColor, so it
    // follows the row's text color (light in dark mode, white when selected). A row's tint
    // (docs/vectors.md) paints the mask with that color instead.
    const icon = div('day-navmenu-icon');
    icon.style.maskImage = `url("${resourceUrl(item.icon)}")`;
    icon.style.webkitMaskImage = `url("${resourceUrl(item.icon)}")`;
    if (item.tint) icon.style.backgroundColor = item.tint;
    row.append(icon);
  }
  const t = document.createElement('span'); t.textContent = item.title; row.append(t);
  // The row's count (docs/navigation.md `badge`), dim at the trailing edge.
  if (item.count) {
    const c = document.createElement('span');
    c.className = 'day-navmenu-count';
    c.textContent = item.count;
    row.append(c);
  }
  // The trailing status glyph (docs/navigation.md), masked like the leading icon so it follows
  // the row's text color unless the app named a color that means something.
  if (item.badgeIcon) {
    const badge = div('day-navmenu-badge');
    badge.style.maskImage = `url("${resourceUrl(item.badgeIcon)}")`;
    badge.style.webkitMaskImage = `url("${resourceUrl(item.badgeIcon)}")`;
    if (item.badgeTint) badge.style.backgroundColor = item.badgeTint;
    row.append(badge);
  }
}

function buildToolbarItem(strip, it) {
  let el = null;
  if (it.kind === '-') el = div('day-toolbar-sep');
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
        ic.style.maskImage = `url("${resourceUrl(seg.icon)}")`;
        ic.style.webkitMaskImage = `url("${resourceUrl(seg.icon)}")`;
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
    strip.bar.append(dl);
    // The virtual keyboard's action key reads "Search", as it does on a search form.
    el.setAttribute('enterkeyhint', 'search');
    // The text Day's model holds, so a submission sends `Text` first only when it differs.
    el.__sent = el.value;
    const sendText = () => {
      el.__sent = el.value;
      const [ptr, len] = intoWasm(el.value);
      wasm.day_dom_toolbar_text(it.action, ptr, len);
    };
    if (it.action) el.addEventListener('input', sendText);
    // Return submits (docs/search.md). The field sits in no <form>, so the key never navigates
    // the page. Composition (an IME confirming a word) and auto-repeat are not submissions. The
    // submission waits a task so a completion the same Return accepted from the <datalist>
    // lands (as `input`) before it, and the app searches what the field shows.
    if (it.action) el.addEventListener('keydown', (e) => {
      if (e.key !== 'Enter' || e.isComposing || e.keyCode === 229 || e.repeat) return;
      setTimeout(() => {
        if (!el.isConnected) return;
        if (el.value !== el.__sent) sendText();
        const [ptr, len] = intoWasm(el.value);
        wasm.day_dom_toolbar_submit(it.action, ptr, len);
      }, 0);
    });
  } else {
    // B, T, S and M are all buttons; only their click behavior differs.
    el = document.createElement('button');
    el.className = 'day-toolbar-btn';
    el.disabled = !it.enabled;
    el.title = it.tip || it.label;
    if (it.icon) {
      const ic = div('day-toolbar-icon');
      ic.style.maskImage = `url("${resourceUrl(it.icon)}")`;
      ic.style.webkitMaskImage = `url("${resourceUrl(it.icon)}")`;
      el.append(ic);
    }
    // Icon alone where there is one, as every desktop toolbar does; the label stays as the
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
    } else if (it.kind === 'M') {
      // A pull-down: the popup below, fed by the encoded item list. Toggling twice on
      // the same button closes it, like every native menu button.
      el.addEventListener('click', (e) => { e.stopPropagation(); toggleToolbarMenu(el, it.menu || []); });
    } else if (it.kind === 'S') {
      // The sidebar toggle owns its behavior: no app action to dispatch.
      el.classList.add('day-toolbar-sidebar');
      el.setAttribute('aria-expanded', 'true');
      el.addEventListener('click', () => {
        // `env`, not `wasm`: the sidebar toggle is a shim verb Rust imports, not a wasm
        // export. Called through `wasm` it is undefined, and the handler threw before
        // toggling anything, which is why the button did nothing at all.
        const shown = env.day_dom_toolbar_sidebar();
        if (!shown) el.disabled = true;
        else el.setAttribute('aria-expanded',
          document.querySelector('.day-nav.split.day-sidebar-hidden') ? 'false' : 'true');
      });
    } else if (it.action) {
      el.addEventListener('click', () => wasm.day_dom_toolbar_action(it.action));
    }
  }
  return el;
}

// The one open toolbar pull-down (docs/toolbars.md). Item activation rides
// day_dom_toolbar_action; the encoded ids are registered menu-action ids.
let toolbarMenu = null;
function closeToolbarMenu() {
  if (!toolbarMenu) return;
  const m = toolbarMenu;
  toolbarMenu = null;
  // The keyboard came in through the button: it goes back there, as a native menu's does.
  const restore = m.contains(document.activeElement) ? m.__anchor : null;
  m.__anchor.setAttribute('aria-expanded', 'false');
  m.remove();
  if (restore) restore.focus();
}
// The rows the keyboard can land on: every enabled command, in order.
const menuRows = (m) => [...m.querySelectorAll('.day-toolbar-menu-item:not(.header):not(.disabled)')];
function menuKeydown(e) {
  const m = toolbarMenu;
  if (!m) return;
  const rows = menuRows(m);
  const at = rows.indexOf(document.activeElement);
  const go = (i) => { if (rows.length) rows[(i + rows.length) % rows.length].focus(); };
  switch (e.key) {
    case 'ArrowDown': go(at + 1); break;
    case 'ArrowUp': go(at < 0 ? rows.length - 1 : at - 1); break;
    case 'Home': go(0); break;
    case 'End': go(rows.length - 1); break;
    case 'Enter': case ' ':
      if (at >= 0) rows[at].click();
      break;
    case 'Tab': closeToolbarMenu(); return; // the menu is not in the tab order
    case 'Escape': closeToolbarMenu(); break;
    default: return;
  }
  e.preventDefault();
  e.stopPropagation();
}
// A shortcut's hint, spelled the way the platform's menus spell it: "⇧⌘S" on a Mac,
// "Ctrl+Shift+S" elsewhere.
const KEY_GLYPHS = { Delete: '\u232B', Backspace: '\u232B', Return: '\u21A9', Enter: '\u21A9',
  Escape: '\u238B', Space: '\u2423', ArrowLeft: '\u2190', ArrowRight: '\u2192',
  ArrowUp: '\u2191', ArrowDown: '\u2193' };
function shortcutLabel(sc) {
  const mac = isMac();
  const key = sc.key.length === 1 ? sc.key.toUpperCase() : (mac && KEY_GLYPHS[sc.key]) || sc.key;
  if (mac) {
    return (sc.control ? '\u2303' : '') + (sc.alt ? '\u2325' : '') + (sc.shift ? '\u21E7' : '')
      + (sc.primary ? '\u2318' : '') + key;
  }
  const parts = [];
  if (sc.primary || sc.control) parts.push('Ctrl');
  if (sc.alt) parts.push('Alt');
  if (sc.shift) parts.push('Shift');
  parts.push(key);
  return parts.join('+');
}
// The key names a shortcut answers to: letters case-folded, the two delete keys folded onto
// each other (appkit and gtk do the same), the named keys under the browser's spellings.
function shortcutKeys(key) {
  if (key.length === 1) return [key.toLowerCase()];
  return { Delete: ['Delete', 'Backspace'], Backspace: ['Delete', 'Backspace'],
    Return: ['Enter'], Enter: ['Enter'], Space: [' '], Esc: ['Escape'] }[key] || [key];
}
function collectAppMenuKeys(items, out) {
  for (const it of items) {
    if (it.items) { collectAppMenuKeys(it.items, out); continue; }
    if (!it.shortcut || it.sys || it.enabled === false || !it.id) continue;
    out.push({ id: it.id, keys: shortcutKeys(it.shortcut.key), primary: !!it.shortcut.primary,
      shift: !!it.shortcut.shift, alt: !!it.shortcut.alt, control: !!it.shortcut.control });
  }
  return out;
}
document.addEventListener('keydown', (e) => {
  if (!appMenuKeys.length || e.isComposing) return;
  const t = e.target;
  // A canvas that took the whole keyboard (docs/menus.md "The full route") owns every chord
  // but the browser's own (⌘ on a Mac): its Ctrl+C is the terminal's, not the menu's.
  if (t && t.dataset && t.dataset.dayTextHost && !e.metaKey) return;
  const editable = t && t.closest && t.closest('input, textarea, [contenteditable]');
  const primary = e.metaKey || e.ctrlKey;
  const key = e.key.length === 1 ? e.key.toLowerCase() : e.key;
  for (const a of appMenuKeys) {
    if (a.primary !== primary || a.shift !== e.shiftKey || a.alt !== e.altKey) continue;
    if (a.control && !e.ctrlKey) continue;
    if (!a.keys.includes(key)) continue;
    // A bare key (Delete, a letter) is the focused text field's own; a command combination
    // reaches the menu from anywhere, the way a menu bar's does.
    if (!primary && editable) return;
    e.preventDefault();
    e.stopPropagation();
    closeToolbarMenu();
    wasm.day_dom_toolbar_action(a.id);
    return;
  }
}, true);
// `hints` draws each row's shortcut beside it: the app menu's rows, whose keys are live.
function toggleToolbarMenu(anchor, items, hints) {
  if (toolbarMenu && toolbarMenu.__anchor === anchor) { closeToolbarMenu(); return; }
  closeToolbarMenu();
  const m = document.createElement('div');
  m.className = 'day-toolbar-menu';
  m.setAttribute('role', 'menu');
  m.__anchor = anchor;
  anchor.setAttribute('aria-expanded', 'true');
  m.addEventListener('keydown', menuKeydown);
  const add = (list, depth) => {
    for (const it of list) {
      if (it.sep) {
        const sep = document.createElement('div');
        sep.className = 'day-toolbar-menu-sep';
        m.append(sep);
        continue;
      }
      const row = document.createElement('div');
      row.className = 'day-toolbar-menu-item';
      row.style.paddingLeft = `${12 + depth * 14}px`;
      if (it.items) {
        // Inlined submenu: a dimmed header, then its children indented, the same flattening
        // the composed context menu uses.
        row.classList.add('header');
        row.textContent = it.label;
        m.append(row);
        add(it.items, depth + 1);
        continue;
      }
      if (it.icon) {
        const ic = document.createElement('div');
        ic.className = 'day-toolbar-icon';
        ic.style.maskImage = `url("${resourceUrl(it.icon)}")`;
        ic.style.webkitMaskImage = `url("${resourceUrl(it.icon)}")`;
        row.append(ic);
      }
      // A checkable row carries the mark's column whichever way it sits, so a run of choices
      // lines up on its titles rather than stepping in and out by a tick's width.
      if (it.checked !== undefined) {
        const mark = document.createElement('span');
        mark.className = 'day-toolbar-menu-check';
        mark.textContent = it.checked ? '\u2713' : '';
        row.append(mark);
      }
      const t = document.createElement('span');
      t.textContent = it.label;
      row.append(t);
      if (hints && it.shortcut) {
        const k = document.createElement('span');
        k.className = 'day-toolbar-menu-key';
        k.textContent = shortcutLabel(it.shortcut);
        row.append(k);
      }
      if (it.name) row.dataset.menuId = it.name;
      // A real menu item: focusable by arrow (not by Tab), activated by Enter or Space, and
      // the pointer's hover is the keyboard's focus so both read as one highlight.
      row.setAttribute('role', it.checked !== undefined ? 'menuitemcheckbox' : 'menuitem');
      if (it.checked !== undefined) row.setAttribute('aria-checked', it.checked ? 'true' : 'false');
      if (it.enabled === false) {
        row.classList.add('disabled');
        row.setAttribute('aria-disabled', 'true');
      } else {
        row.tabIndex = -1;
        row.addEventListener('pointerenter', () => row.focus());
        row.addEventListener('click', () => {
          closeToolbarMenu();
          if (it.id) wasm.day_dom_toolbar_action(it.id);
        });
      }
      m.append(row);
    }
  };
  add(items, 0);
  const r = anchor.getBoundingClientRect();
  m.style.left = `${Math.round(r.left)}px`;
  m.style.top = `${Math.round(r.bottom + 4)}px`;
  // A long menu (an app menu with every submenu inlined) scrolls inside the window instead of
  // running off its bottom edge.
  m.style.maxHeight = `${Math.max(120, innerHeight - Math.round(r.bottom + 4) - 8)}px`;
  document.body.append(m);
  // Opened past the right edge: pull it back inside.
  const mr = m.getBoundingClientRect();
  if (mr.right > innerWidth - 4) m.style.left = `${Math.max(4, innerWidth - 4 - mr.width)}px`;
  toolbarMenu = m;
  // Opened from the keyboard (Enter/Space on the button): the first command takes focus so the
  // arrows work at once; a pointer open leaves focus on the button until the pointer moves.
  if (anchor.matches(':focus-visible')) menuRows(m)[0]?.focus();
}
document.addEventListener('pointerdown', (e) => {
  if (toolbarMenu && !toolbarMenu.contains(e.target) && !toolbarMenu.__anchor.contains(e.target)) closeToolbarMenu();
});
// Escape closes what is on top: an open toolbar menu, else the topmost modal sheet (docs/cover.md
// "Modal sheet"), as Escape cancels a sheet on a desktop, or the topmost cover with a strip of
// its own (a settings cover on a narrow window, whose Done is on that strip). A cover with no
// strip, a game's full window, keeps the key for its content. The cover hears it as the back
// request a phone's back gesture makes, so one that has turned interactive dismissal off stays
// up. A key something else already answered (a control's own Escape, a dialog's cancel) is
// left alone.
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Escape') return;
  if (toolbarMenu) { closeToolbarMenu(); return; }
  if (e.defaultPrevented || e.isComposing || document.querySelector('dialog[open]')) return;
  const covers = document.querySelectorAll('.day-cover.open');
  const top = covers[covers.length - 1];
  if (!top || !(top.classList.contains('day-sheet') || top.classList.contains('day-has-toolbar'))) return;
  e.preventDefault();
  wasm.day_dom_event(top.__id, 14, 0, 0, 0, 0);
});

// Fill a <datalist> with a search field's completions (docs/search.md).
function setSuggestions(dl, list) {
  dl.textContent = '';
  for (const s of list) {
    const o = document.createElement('option');
    o.value = s;
    dl.append(o);
  }
}
// Crate-owned browser behaviors release subscriptions before their element is destroyed.
const releaseHooks = new Map();

// Show `url` (a blob this page made) in an image element: an `<img>` loads it, and a masked
// element (the div a tinted or template image is, or an `<img>` tinted later) cuts its mask
// from it. Remembered on the node so a tint that arrives after the source can find it.
function showImageUrl(node, url) {
  if (!node) return;
  node.__dayUrl = url;
  if (node.tagName === 'IMG') node.src = url;
  if (node.tagName !== 'IMG' || node.__dayMasked) {
    node.style.maskImage = `url("${url}")`;
    node.style.webkitMaskImage = `url("${url}")`;
  }
}

// Decoded bitmaps (docs/images.md), keyed by the id day-core minted: the `ImageBitmap` a canvas
// draws, plus the object URL an `<img>` loads. Entries leave only through
// `day_dom_image_release`, which day-core calls when the app drops its last handle. The URL is
// revoked there, since an object URL the page forgets is a leak the GC cannot reach.
const bitmaps = new Map();
// MIME type → whether this engine can encode it (see `day_dom_image_can_encode`).
const encodeProbe = new Map();
const utf8 = new TextDecoder();
const utf8enc = new TextEncoder();

// devicemotion state (see day_dom_sensor_*). One listener, both kinds.
// The live geolocation watch id (0 = none).
let geoWatch = 0;

const sensorState = { generations: [0, 0], listener: null };

const mem = () => new Uint8Array(wasm.memory.buffer);
const f64 = (ptr, len) => new Float64Array(wasm.memory.buffer, ptr, len);
const str = (ptr, len) => utf8.decode(new Uint8Array(wasm.memory.buffer, ptr, len));
// Copy a JS string into wasm memory at `ptr` (capacity `cap`), returning the bytes written: the
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

// Hold the keyboard on a list or sidebar across the rebuild its own key press just caused. The
// detail that lands can contain an element the browser focuses as soon as it appears (a <video>,
// a web view's iframe), and the sidebar would lose the arrows after a single step. A native
// outline keeps focus while the detail changes, so this restores it once the rebuild has
// settled. Only for key-driven moves: a click that lands somewhere focusable is the user aiming
// there.
function keepFocus(el) {
  setTimeout(() => {
    if (el.isConnected && document.activeElement !== el) el.focus({ preventScroll: true });
  }, 0);
}

// A nav host's chrome for one presentation: sidebar+detail panes, or a back bar over a single
// detail region. Shared by the initial realize and by a re-present, so the two can never drift.
// The four presentations (docs/size-classes.md), keyed by the `mode` lib.rs sends: 0 split,
// 1 stack, 2 tabs, 3 rail. `__side` is the chrome slot (a sidebar pane, a tab bar, or a rail)
// and always holds the same NAV_MENU element, so switching between them re-parents one node
// rather than rebuilding the rows and their listeners.
const NAV_MODES = ['split', 'stack', 'tabs', 'rail'];

function navChrome(nav, id, mode) {
  const name = NAV_MODES[mode] || 'stack';
  // A split arriving or leaving after the toolbar (docs/toolbars.md) changes the columns it
  // follows; measured once the panes have their width.
  requestAnimationFrame(layoutToolbars);
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
  // A tab bar sits below the content, the way every phone puts it; a sidebar and a rail lead it.
  if (name === 'tabs') nav.append(detail, side); else nav.append(side, detail);
  nav.__side = side; nav.__detail = detail;
}

const E = (id) => els[id];
// The element that carries value/checked/listeners (the toggle wraps its input).
const V = (id) => E(id).__input || E(id);

// ---------------------------------------------------------------------------
// Imports: the DOM verbs lib.rs declares
// ---------------------------------------------------------------------------

// Satisfy import modules besides `env` with loud stubs. A dependency compiled with unreached
// wasm-bindgen code paths (chrono's default `wasmbind`, dragged in by a parsing library) still
// lists `__wbindgen_placeholder__.*` in the module's import table, and instantiation refuses a
// missing module outright. The stubs let the module start; a call into one (code that needs
// the wasm-bindgen runtime, which the day web pipeline does not carry) fails with a diagnosable
// message at the call site instead of a TypeError at startup.
const foreignStubs = (module, imports) => {
  for (const im of WebAssembly.Module.imports(module)) {
    if (im.module === 'env') continue;
    const mod = (imports[im.module] ||= {});
    if (im.name in mod) continue;
    if (im.kind === 'function') {
      mod[im.name] = () => {
        throw new Error(
          `day: ${im.module}.${im.name} called — this import needs a runtime (wasm-bindgen?) the day web pipeline does not provide`,
        );
      };
    } else if (im.kind === 'global') {
      mod[im.name] = new WebAssembly.Global({ value: 'i32', mutable: false }, 0);
    } else if (im.kind === 'table') {
      mod[im.name] = new WebAssembly.Table({ element: 'anyfunc', initial: 0 });
    } else if (im.kind === 'memory') {
      mod[im.name] = new WebAssembly.Memory({ initial: 1 });
    }
  }
  return imports;
};

// Minutes east of UTC at `ms`, for the page's zone or for the `?tz=` override the `tz` key
// honors. `Date.getTimezoneOffset` counts the other way (minutes to add to reach UTC), and a
// named zone states its offset through `Intl` as "GMT+09:00", "GMT" alone being UTC. An
// unknown id answers 0 rather than throwing into the app.
function tzOffsetMinutes(ms) {
  const override = new URLSearchParams(location.search).get('tz');
  if (!override) return -new Date(ms).getTimezoneOffset();
  try {
    const named = new Intl.DateTimeFormat('en-US', { timeZone: override, timeZoneName: 'longOffset' })
      .formatToParts(new Date(ms))
      .find((p) => p.type === 'timeZoneName')?.value ?? '';
    const m = /GMT([+-])(\d{1,2}):(\d{2})/.exec(named);
    if (!m) return 0;
    const mins = Number(m[2]) * 60 + Number(m[3]);
    return m[1] === '-' ? -mins : mins;
  } catch {
    return 0;
  }
}

const liveRegions = {}; // the announcement live regions, by politeness (day_dom_announce)

const env = {
  // Seconds since the Unix epoch, for day-piece-datetime: wasm32-unknown-unknown has no clock of
  // its own, and `SystemTime::now()` traps there rather than failing.
  day_datetime_now_secs: () => Date.now() / 1000,

  day_dom_drag_source: (id) => installDragSource(id),
  day_dom_drop_target: (id) => installDropTarget(id),
  day_dom_drag_offer: (p, n) => { dragOffer = mem().slice(p, p + n); },
  day_dom_create: (kind) => create(kind),

  day_dom_insert(parent, child, index) {
    const p = E(parent); const target = p.__content || p;
    const el = E(child);
    const ref = dayChildAt(target, index);
    // Already where it belongs. `insertBefore` does not check: it removes the node and puts it
    // back, and removing a subtree that holds the focused element blurs it. Day re-inserts a
    // child at its existing index whenever a sibling's props change, so a text field lost focus
    // on every keystroke: the caret went to <body> and only the first character landed.
    if (el === ref || el.__daySlopEl === ref || (el.parentNode === target && el.nextSibling === ref)) return;
    target.insertBefore(el, ref);
    placeSlopSibling(el);
    // A cover's strip travels with it (each present re-homes the cover at the end of the root),
    // so it stays the next sibling: painted over the cover, and hidden with it when it closes.
    const strip = coverStrips.get(el);
    if (strip && strip.bar) el.after(strip.bar);
  },
  day_dom_remove: (child) => { const el = E(child); dropCoverStrip(el); el?.__daySlopEl?.remove(); el?.remove(); },
  day_dom_release(id) {
    E(id)?.__datalist?.remove();
    const callbacks = releaseHooks.get(id);
    releaseHooks.delete(id);
    for (const dispose of callbacks ?? []) {
      try { dispose(); } catch (error) { console.error("day: extension cleanup", error); }
    }
    if (E(id)) dropSlopSibling(E(id));
    E(id)?.remove();
    els[id] = null;
  },

  day_dom_set_frame(id, x, y, w, h) {
    const el = E(id);
    const s = el.style;
    s.position = 'absolute';
    if (el.__dayRuns?.[3]) animateProp(el, 3, [x, y, w, h], null);
    else { s.left = x + 'px'; s.top = y + 'px'; s.width = w + 'px'; s.height = h + 'px'; }
    followSheetCard(E(id));
    placeSlopSibling(E(id));
  },
  // Options and selection for a picker, a segmented control or a radio group: one verb over
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
      b.disabled = el.hasAttribute('disabled');
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
      // A text area's edit echoes back through the binding, and assigning `value` moves the
      // caret to the end, so typing in the middle of existing notes would jump away after each
      // character. The echo carries the text it already has, so an equal value is nothing to do.
      if (el.value !== t) el.value = t;
    } else {
      el.textContent = t;
    }
  },
  day_dom_set_style(id, p, pl, v, vl) { E(id).style.setProperty(str(p, pl), resourceStyle(str(v, vl))); },
  // A styled run's link (docs/text-runs.md). `owner` is the label's element, since that is the
  // one Rust knows as a node; the run spans are not nodes. The anchor's navigation is
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
    if (name === 'data-day-presented') {
      el.style.visibility = val === 'true' ? 'visible' : 'hidden';
      el.__dayHidden = val !== 'true';
      syncInert(el);
      placeSlopSibling(E(id));
      return;
    }
    if (name === 'data-day-animating') {
      el.style.animationPlayState = val === 'true' ? 'running' : 'paused';
      return;
    }
    if (name === 'data-day-completions') {
      if (!el.__datalist) {
        const dl = document.createElement('datalist');
        dl.id = `day-completions-${id}`;
        el.__datalist = dl;
        el.setAttribute('list', dl.id);
        document.body.append(dl);
      }
      setSuggestions(el.__datalist, JSON.parse(val));
      return;
    }
    if (name === 'value') { el.value = val; return; }
    // Attributes whose empty value means "none" rather than an empty string: an empty
    // `aria-label` would name the element with nothing, and `dir=""` is not the inherited one.
    if (val === '' && (name === 'aria-label' || name === 'dir' || name === 'role' || name === 'aria-selected'
      || name === 'aria-level' || name.startsWith('data-day-'))) {
      el.removeAttribute(name);
      return;
    }
    // Boolean attrs use a marker convention from the Rust side: "" removes, "-" sets.
    if (name === 'disabled' || name === 'readonly') {
      val === '' ? el.removeAttribute(name) : el.setAttribute(name, '');
      // A segmented or radio picker is a <div> of buttons, which `disabled` does not reach:
      // the group keeps the attribute as its state (day_dom_options rebuilds read it) and
      // passes it to each choice.
      if (name === 'disabled' && isChoiceGroup(el)) {
        [...el.children].forEach((b) => { b.disabled = val !== ''; });
        el.setAttribute('aria-disabled', val === '' ? 'false' : 'true');
      }
      return;
    }
    if (name === 'src' && el.tagName === 'IFRAME') { setResourceFrameSource(el, val); return; }
    el.setAttribute(name, name === 'src' ? resourceUrl(val) : val);
    // Accessibility (docs/accessibility.md). A canvas or a div retyped as a button needs the
    // tab stop a real one has; the document's keydown route (below `listen`) makes Enter and
    // Space act on it. Hidden takes the subtree out of the accessibility tree, and here out
    // of the tab order too: `aria-hidden` alone leaves a focusable element reachable, and a
    // screen reader landing on it would find nothing to read. Descendants realized later are
    // not covered, which is why this runs on every call rather than once.
    if (name === 'role' && val === 'button' && !el.matches(NATIVE_CONTROLS)) {
      if (!el.hasAttribute('tabindex')) el.tabIndex = 0;
    } else if (name === 'aria-hidden' && val === 'true') {
      for (const f of [el, ...el.querySelectorAll(FOCUSABLE)]) f.tabIndex = -1;
    }
  },
  // The piece-renderer escape hatch (docs/extending.md): day-toolkit-dom's own EL_* kind codes cover only
  // the built-in vocabulary, so an external piece creates its element by tag name and drives it
  // with zero-argument method calls (`play`, `pause`, `load`, …).
  day_dom_create_tag(t, tl) { return register(document.createElement(str(t, tl))); },
  // Styled-text editing (docs/texteditor.md): the markup comes from Day's own HTML serializer,
  // which escapes every character of app text, and the caret survives the swap.
  day_dom_set_html(id, p, l) { dayEditorSetHtml(V(id), str(p, l)); },
  day_dom_editor_select(id, a, b) { dayEditorSelect(V(id), a, b); },
  day_dom_editor_insert(id, at, p, l) { dayEditorInsert(V(id), at, str(p, l)); },
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
  // A text field's selection (docs/textfield.md "Selection"): `a..b` in characters (Unicode
  // scalar values), walked onto the input's UTF-16 value a surrogate pair at a time and held to
  // its end. The caret lands on `b`.
  day_dom_field_select(id, a, b) {
    const el = V(id);
    if (typeof el.setSelectionRange !== 'function') return;
    const v = el.value;
    const at = (n) => {
      let i = 0;
      while (n > 0 && i < v.length) {
        const c = v.charCodeAt(i);
        i += c >= 0xd800 && c <= 0xdbff && i + 1 < v.length ? 2 : 1;
        n--;
      }
      return i;
    };
    try { el.setSelectionRange(at(a), at(b)); } catch (_) { /* a type without a selection */ }
  },
  // Text entry traits (docs/textfield.md): one object of attribute name to value from the Rust
  // side, "" removing the attribute. Only a text field's <input> takes them. `type` is assigned
  // as the property: the value and focus survive the change, and the caret is put back.
  day_dom_input_traits(id, p, l) {
    const el = V(id);
    if (el.tagName !== 'INPUT' || !el.classList.contains('day-field')) return;
    for (const [name, val] of Object.entries(JSON.parse(str(p, l)))) {
      if (name === 'type') {
        if (el.type === val) continue;
        const a = el.selectionStart, b = el.selectionEnd;
        el.type = val;
        if (document.activeElement === el) el.setSelectionRange(a, b);
      } else if (val === '') el.removeAttribute(name);
      else if (el.getAttribute(name) !== val) el.setAttribute(name, val);
    }
  },

  day_dom_listen: (id, mask) => listen(id, mask),
  // Animation (docs/tween.md): `prop` and the target numbers as in `animateProp`; `samples`
  // `[offset, progress]` pairs of one forward cycle; `duration` 0 for a plain set; `iterations`
  // -1 for ever.
  day_dom_animate(id, prop, to, toLen, samples, samplesLen, duration, delay, iterations, alternate) {
    const el = E(id);
    const target = Array.from(f64(to, toLen));
    let spec = duration > 0 ? {
      duration, delay, iterations, alternate: !!alternate, samples: Array.from(f64(samples, samplesLen)),
    } : null;
    // A fill appearing applies at once (rule 8): there is no color to pass from.
    if (prop === 2 && !ownFill(el)) spec = null;
    if (prop === 3) el.style.position = 'absolute';
    animateProp(el, prop, target, spec);
    if (prop === 3) placeSlopSibling(el);
  },
  // What an element displays right now (docs/tween.md "Mid-flight read-back"): `out` gets
  // [opacity, x, y, w, h, tx, ty, r, g, b, a]; the frame in the root's space, before the
  // element's own transform. Returns 1 when the element shows a background color of its own.
  day_dom_displayed(id, out) {
    const el = E(id);
    if (!el || !el.isConnected) return 0;
    const cs = getComputedStyle(el);
    const o = root().getBoundingClientRect();
    const pr = (el.offsetParent || el.parentElement)?.getBoundingClientRect() ?? o;
    const t = parseTransform(cs.transform);
    const bg = parseColor(cs.backgroundColor) ?? [0, 0, 0, 0];
    f64(out, 11).set([
      parseFloat(cs.opacity),
      pr.left - o.left + (parseFloat(cs.left) || 0), pr.top - o.top + (parseFloat(cs.top) || 0),
      parseFloat(cs.width) || 0, parseFloat(cs.height) || 0,
      t[0], t[1], ...bg,
    ]);
    return ownFill(el) || el.__dayRuns?.[2] ? 1 : 0;
  },
  // Input (docs/gestures.md): the grown hit area, the drag's axis (0 any, 1 horizontal,
  // 2 vertical), a disabled subtree, the kind of pointer in use, and the pass-through switch.
  day_dom_hit_slop: (id, t, l, b, r) => setHitSlop(id, t, l, b, r),
  day_dom_drag_axis(id, axis) {
    const el = E(id);
    el.__dayAxis = axis;
    applyTouchAction(el);
  },
  // `inert` takes input, focus and the accessibility tree in one; a count, since a view can sit
  // under two disabled groups that day-core reports separately. A press under way ends.
  day_dom_subtree_enabled(id, on) {
    const el = E(id);
    el.__dayDisabled = Math.max(0, (el.__dayDisabled || 0) + (on ? -1 : 1));
    if (el.__dayDisabled) el.setAttribute('aria-disabled', 'true');
    else el.removeAttribute('aria-disabled');
    if (el.__dayDisabled && el.contains(document.activeElement)) document.activeElement.blur();
    syncInert(el);
  },
  day_dom_pointer_kind: () => pointerKind,
  day_dom_takes_pointer: (id) => takesPointer(E(id)),

  day_dom_measure_text(t, tl, f, fl, maxW, out) {
    let text2, font, numeric = 'normal';
    if (t === 0) { // measure element `tl`'s own text and computed font
      const el = E(tl);
      const cs = getComputedStyle(el);
      font = elementFont(cs);
      numeric = cs.fontVariantNumeric;
      if (el.firstElementChild) {
        // Styled runs (docs/text-runs.md): a bold span is wider than the same words in the
        // base font, so the flat text would under-measure and the last word of a wrapped
        // paragraph would land on a clipped third line. Measure the spans themselves.
        const [w, h] = measureNodes(el, font, maxW, numeric);
        f64(out, 2).set([w, h]);
        return;
      }
      text2 = el.textContent || '';
    } else { text2 = str(t, tl); font = str(f, fl); }
    const [w, h] = measure(text2, font, maxW, numeric);
    f64(out, 2).set([w, h]);
  },
  // First text baseline from the element's top, in px, for a box `boxH` tall
  // (docs/baseline.md). The browser knows the exact metrics: a canvas TextMetrics reports the
  // font's ascent, and the element's computed padding/border says where its text box starts,
  // so an <input> with a border reports a lower baseline than a bare <div>, which is what the
  // caller needs. Returns -1 when the element has no text of its own.
  day_dom_baseline(id, boxH) {
    const el = E(id);
    if (!el) return -1;
    const cs = getComputedStyle(el);
    const m = baselineMetrics(elementFont(cs));
    if (!m) return -1;
    // Content box: what the border and padding leave for the line.
    const top = parseFloat(cs.borderTopWidth) || 0;
    const padT = parseFloat(cs.paddingTop) || 0;
    const padB = parseFloat(cs.paddingBottom) || 0;
    const borderB = parseFloat(cs.borderBottomWidth) || 0;
    const inner = Math.max(0, boxH - top - padT - padB - borderB);
    // A label's first line is its own line box (`line-height`, docs/text.md "Line height"):
    // the glyphs sit centered in it however many lines follow.
    const lh = parseFloat(cs.lineHeight);
    if (el.classList.contains('day-label') && Number.isFinite(lh) && lh > 0) {
      return top + padT + (lh - m.line) / 2 + m.ascent;
    }
    // One line, centered in the content box: the same model every control uses for its text.
    return top + padT + Math.max(0, (inner - m.line) / 2) + m.ascent;
  },
  day_dom_width: (id) => E(id).clientWidth,
  // A label in its own computed style (docs/text.md): font, letter spacing, line height, line
  // clamp, wrapping and features all copied to a measuring twin, so the size is the one the
  // element will draw. `t`/`tl` stand in for the element's content when `tl` is nonzero (an
  // elided label measures the text it stands for).
  day_dom_measure_label(id, t, tl, maxW, out) {
    const el = E(id);
    const twin = labelTwin(el, maxW);
    if (tl) twin.textContent = str(t, tl) || ' ';
    else if (el.firstElementChild) twin.replaceChildren(...Array.from(el.childNodes, (n) => n.cloneNode(true)));
    else twin.textContent = el.textContent || ' ';
    const r = twin.getBoundingClientRect();
    twin.textContent = '';
    f64(out, 2).set([r.width, r.height]);
  },
  // The browser's text size against the one Day's ramp assumes: 16 px, or 17 px where the root
  // follows iOS Dynamic Type (day.css anchors a coarse-pointer page to -apple-system-body).
  day_dom_text_scale: () => textScale(),

  day_dom_scroll_to(id, x, y, animated) {
    const el = E(id);
    glide(el, animated ? y : null);
    el.scrollTo({ left: x, top: y, behavior: animated ? 'smooth' : 'instant' });
  },
  day_dom_scroll_edge(id, edge, animated) {
    const el = E(id);
    const top = edge === 0 ? 0 : el.scrollHeight;
    glide(el, animated ? top : null);
    el.scrollTo({ top, behavior: animated ? 'smooth' : 'instant' });
  },
  day_dom_scroll_offset(id, out) { const el = E(id); f64(out, 2).set([el.scrollLeft, el.scrollTop]); },
  // What an emulated list is showing (docs/list.md): the scrolled offset and the
  // visible height. The list positions every row itself, so this is the only way it can know
  // which handful of a ten-thousand-row source needs building.
  day_dom_list_viewport(id, out) { const el = E(id); f64(out, 2).set([el.scrollTop, el.clientHeight]); },
  // Report scrolling, so rows coming into view get built before they are looked at. Passive:
  // this never calls preventDefault, and saying so keeps the scroll off the main thread's
  // critical path.
  day_dom_list_on_scroll(id) {
    const el = E(id);
    // The reader's scrolls first: Day hears that the reader took the list before the viewport
    // report this scroll leads to (each listener's turn ends in a microtask checkpoint).
    trackListReader(id, el);
    el.addEventListener('scroll', () => wasm.day_dom_list_scrolled(id), { passive: true });
  },
  // Keyboard selection for the emulated list (docs/list.md). Every native list gives its rows
  // arrow keys; a scrolled <div> of cells gives them nothing, so the tab stop, the listbox role
  // and the key route are all built here. The host takes the focus rather than the rows: the
  // rows are recycled as the list scrolls, and focus parked on one would evaporate under it.
  // The web half of "keys follow focus" (docs/menus.md). A <canvas> is not focusable on its
  // own, so nothing the app draws in one could ever hold the keyboard; this gives it the tab
  // stop and the key route, and takes focus on a press the way a real control does.
  day_dom_canvas_keynav(id) {
    const el = E(id);
    el.tabIndex = 0;
    if (el.dataset.dayKeynav) return;
    el.dataset.dayKeynav = 'true';
    // The full route's input host (docs/menus.md "The full route"). A <canvas> cannot hold a
    // composition, take a dead key or raise a phone's keyboard, so a canvas that took the
    // whole keyboard focuses a transparent <textarea> laid over it instead. The host's
    // keydown names the keys that type nothing and the chords; its input and composition
    // events deliver the text, once, after the browser and the input method have composed
    // it. Day sees one focus: the canvas's, held by either element.
    const NAMED = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Enter', 'Tab', 'Escape', 'Backspace',
      'Delete', 'Home', 'End', 'PageUp', 'PageDown', 'Insert', 'F1', 'F2', 'F3', 'F4', 'F5', 'F6', 'F7', 'F8',
      'F9', 'F10', 'F11', 'F12'];
    const MODIFIERS = ['Shift', 'Control', 'Alt', 'Meta', 'CapsLock', 'NumLock', 'ScrollLock', 'Fn', 'AltGraph'];
    const isMac = /Mac|iPhone|iPad/.test(navigator.platform || '');
    let host = null;
    let lastKey = null;
    let reported = false;
    // Phases (docs/menus.md "Keys"): bit 8 of the modifier word marks a repeat, bit 9 a
    // release. A release goes only to the element whose press it claimed, keyed by the physical
    // key (a held Shift changes what the key names), and none after the focus has gone.
    const REPEAT = 256, UP = 512;
    const claimed = new Map();
    const takesKeyboard = () => !!wasm.day_dom_canvas_text(id);
    const send = (key, text, mods) => {
      const [kp, kl] = intoWasm(key);
      const [tp, tl] = text ? intoWasm(text) : [0, 0];
      return wasm.day_dom_canvas_text_key(id, kp, kl, tp, tl, mods);
    };
    // One focus report for the pair: what matters is whether either element holds it.
    const reportFocus = () => queueMicrotask(() => {
      const has = document.activeElement === el || (host && document.activeElement === host);
      if (has === reported) return;
      reported = has;
      wasm.day_dom_event(id, 7, has ? 1 : 0, 0, 0, 0);
    });
    const textHost = () => {
      if (host) return host;
      host = document.createElement('textarea');
      host.className = 'day-canvas-text-host';
      host.setAttribute('aria-hidden', 'true');
      host.setAttribute('autocapitalize', 'off');
      host.setAttribute('autocomplete', 'off');
      host.setAttribute('autocorrect', 'off');
      host.spellcheck = false;
      host.tabIndex = -1;
      host.dataset.dayTextHost = String(id);
      Object.assign(host.style, { position: 'fixed', left: '0', top: '0', width: '1px', height: '1px',
        opacity: '0', padding: '0', border: '0', margin: '0', resize: 'none', overflow: 'hidden',
        pointerEvents: 'none', zIndex: '-1' });
      document.body.appendChild(host);
      host.addEventListener('focus', reportFocus);
      host.addEventListener('blur', () => { host.value = ''; lastKey = null; claimed.clear(); reportFocus(); });
      host.addEventListener('keyup', (e) => {
        const key = claimed.get(e.code);
        if (key === undefined) return;
        claimed.delete(e.code);
        const mods = (e.shiftKey ? 1 : 0) | (e.ctrlKey ? 2 | 8 : 0) | (e.altKey ? 4 : 0);
        if (send(key, null, mods | UP)) e.preventDefault();
      });
      host.addEventListener('keydown', (e) => {
        // Mid-composition the input method owns the keys; a lone modifier is not a key; a
        // ⌘ chord is the browser's, and so is its paste, which types into the host below.
        if (e.isComposing || e.key === 'Process' || e.key === 'Dead' || e.key === 'Unidentified') return;
        if (MODIFIERS.includes(e.key) || e.metaKey) return;
        const mods = (e.shiftKey ? 1 : 0) | (e.ctrlKey ? 2 | 8 : 0) | (e.altKey ? 4 : 0);
        const paste = e.ctrlKey && !e.altKey && e.key.toLowerCase() === 'v';
        if (paste) return;
        // A chord types nothing: Ctrl anywhere, Alt where Alt does not type (on a Mac ⌥ does,
        // and its glyph arrives through `input` with the Alt bit kept).
        const chord = e.ctrlKey || (e.altKey && !isMac);
        const phase = e.repeat ? REPEAT : 0;
        if (NAMED.includes(e.key) || chord) {
          if (send(e.key, null, mods | phase)) {
            e.preventDefault();
            if (!e.repeat) claimed.set(e.code, e.key);
          }
          return;
        }
        // What types reaches `input`, named by this key.
        lastKey = { key: e.key.length === 1 ? e.key : '', mods, phase, code: e.code };
      });
      host.addEventListener('input', (e) => {
        // A composition's text comes once, from compositionend; a paste comes as the
        // browser's own text; anything else is the key that just typed.
        if (e.isComposing || e.inputType === 'insertCompositionText' || e.inputType === 'insertFromComposition') return;
        const text = e.data != null ? e.data : host.value;
        host.value = '';
        if (!text) return;
        const byKey = lastKey && lastKey.key && [...text].length === 1 && e.inputType === 'insertText';
        // A held key that types repeats as text too, and its release is the key's.
        if (send(byKey ? lastKey.key : '', text, byKey ? lastKey.mods | lastKey.phase : 0) && byKey && !lastKey.phase) {
          claimed.set(lastKey.code, lastKey.key);
        }
        lastKey = null;
      });
      host.addEventListener('compositionend', (e) => {
        host.value = '';
        lastKey = null;
        if (e.data) send('', e.data, 0);
      });
      return host;
    };
    // Put the host where the canvas is, so a phone scrolls the canvas, not a corner, into view.
    const focusHost = () => {
      const h = textHost();
      const r = el.getBoundingClientRect();
      h.style.left = `${Math.max(0, r.left)}px`;
      h.style.top = `${Math.max(0, r.top)}px`;
      h.focus({ preventScroll: true });
    };
    el.addEventListener('focus', () => {
      if (takesKeyboard()) { focusHost(); return; }
      reportFocus();
    });
    el.addEventListener('blur', () => { claimed.clear(); reportFocus(); });
    // Capture phase: a press focuses the element before its own recognizers hear the press
    // (docs/focus.md), and takes nothing from them.
    el.addEventListener('pointerdown', e => {
      if (e.target.closest('button,input,select,textarea,a,[contenteditable]')) return;
      if (takesKeyboard()) {
        if (!host || document.activeElement !== host) focusHost();
        return;
      }
      if (document.activeElement !== el) el.focus({ preventScroll: true });
    }, { capture: true, passive: true });
    el.addEventListener('keydown', (e) => {
      if (e.target !== el) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const keys = { ArrowLeft: 0, ArrowRight: 1, ArrowUp: 2, ArrowDown: 3, Delete: 4, Backspace: 5 };
      // A digit is 10 + its value; the browser names the main row and the keypad alike, by
      // what the key types.
      const code = e.key in keys ? keys[e.key] : /^[0-9]$/.test(e.key) ? 10 + Number(e.key) : -1;
      if (code < 0) return;
      // Claimed only if the app actually took it: an unclaimed key stays the browser's, so a
      // page with a canvas on it still scrolls with the keyboard (and Backspace still means
      // whatever the browser wants it to mean).
      if (wasm.day_dom_canvas_key(id, code, (e.shiftKey ? 1 : 0) | (e.repeat ? REPEAT : 0))) {
        e.preventDefault();
        if (!e.repeat) claimed.set(e.code, code);
      }
    });
    el.addEventListener('keyup', (e) => {
      if (e.target !== el) return;
      const code = claimed.get(e.code);
      if (code === undefined) return;
      claimed.delete(e.code);
      if (wasm.day_dom_canvas_key(id, code, (e.shiftKey ? 1 : 0) | UP)) e.preventDefault();
    });
  },
  day_dom_list_keynav(id, multi) {
    const host = E(id);
    host.tabIndex = 0;
    host.setAttribute('role', 'listbox');
    if (multi) host.setAttribute('aria-multiselectable', 'true');
    // Clicking a row is how a list normally comes into focus, and the cells are children, so
    // the press has to be caught on the way down. `preventScroll` because the browser would
    // otherwise scroll the freshly focused host into view and undo the list's own position.
    host.addEventListener('pointerdown', (e) => {
      if (e.target.closest('button,input,select,textarea,a,[contenteditable]')) return;
      if (document.activeElement !== host) host.focus({ preventScroll: true });
    });
    host.addEventListener('keydown', (e) => {
      if (e.target !== host) return;
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const dirs = { ArrowUp: 0, ArrowDown: 1, Home: 2, End: 3, Enter: 4 };
      if (!(e.key in dirs)) return;
      if (e.key === 'Enter') {
        if (wasm.day_dom_list_key(id, dirs[e.key], 0)) {
          e.preventDefault();
          keepFocus(host);
        }
        return;
      }
      // Claimed even at the ends of the list, where the row does not move: the alternative is
      // the page scrolling under a list that plainly has the keyboard.
      e.preventDefault();
      wasm.day_dom_list_key(id, dirs[e.key], e.shiftKey ? 1 : 0);
      keepFocus(host);
    });
  },
  // Pointer-drag reorder for the emulated list (docs/list.md): the browser has no native list
  // reorder, so this fakes the affordance (lift the pressed cell, slide a gap under it with CSS
  // transitions on the other cells, autoscroll near the edges) while the decisions stay
  // Day's: every hovered slot is vetted synchronously through wasm.day_dom_list_can_move (the
  // app's guard), and the drop commits through wasm.day_dom_list_move, which re-binds the cells.
  // Rows come from wasm.day_dom_list_row_at, which knows each row's height, so rows of different
  // heights and section headers land where they look. A header (and the pinned copy of one)
  // never lifts and never shifts: a row drags only within its own section.
  day_dom_list_reorder(id) {
    const host = E(id);
    host.classList.add('day-reorder');
    let d = null; // in-flight drag
    const header = (c) => c.getAttribute('role') === 'heading';
    const cells = () => [...host.querySelectorAll('.day-cell')].filter((c) => !header(c));
    const rowOf = (c) => wasm.day_dom_list_row_at(id, c.offsetTop + c.offsetHeight / 2);
    const cleanup = () => {
      if (!d) return;
      for (const c of cells()) { c.style.transform = ''; c.classList.remove('day-drag'); }
      host.classList.remove('day-no-drop');
      clearTimeout(d.hold);
      d = null;
    };
    host.addEventListener('pointerdown', (e) => {
      const cell = e.target.closest('.day-cell');
      if (!cell || header(cell) || d) return;
      const rowH = cell.offsetHeight || 1;
      const from = rowOf(cell);
      if (from < 0) return;
      d = {
        cell, rowH, from,
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
      const slot = wasm.day_dom_list_row_at(id, center);
      const verdict = wasm.day_dom_list_can_move(id, d.from, slot);
      d.accepted = verdict < 0 ? null : verdict;
      host.classList.toggle('day-no-drop', d.accepted === null);
      for (const c of cells()) {
        if (c === d.cell) continue;
        const row = rowOf(c);
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
    layoutSnap(E(id));
  },
  // A scroll view's realize-time preferences (docs/scroll.md): `flags` bit 0 hides the
  // indicators, bit 1 stops the content dead at its edges, bit 2 reports the user's scroll
  // phases; `snap` 0 none, 1 page, 2 every `interval` points.
  day_dom_scroll_setup(id, flags, snap, interval) {
    const el = E(id);
    el.classList.toggle('day-no-indicators', !!(flags & 1));
    if (flags & 2) el.style.overscrollBehavior = 'none';
    if (flags & 4) trackScrollPhase(id, el);
    if (snap) {
      el.__daySnap = { page: snap === 1, interval, markers: [] };
      el.style.scrollSnapType = `${el.classList.contains('horizontal') ? 'x' : 'y'} mandatory`;
      new ResizeObserver(() => layoutSnap(el)).observe(el);
      layoutSnap(el);
    }
  },
  // Touch screens raise a software keyboard that a scroll view's drag can dismiss
  // (docs/scroll.md "Keyboard").
  day_dom_touch_points: () => navigator.maxTouchPoints || 0,
  day_dom_focus(id, focused) { const el = V(id); focused ? el.focus() : el.blur(); },

  day_dom_canvas_replay: (id, ops, opsLen, strs, strsLen, w, h) =>
    replay(E(id), f64(ops, opsLen), new Uint8Array(wasm.memory.buffer, strs, strsLen), w, h),
  // A drawing exported to a bitmap (docs/canvas.md "Exporting"): painted into a detached canvas
  // of `pw` × `ph` pixels scaled to `scale` pixels per point, transparent where nothing is drawn.
  // The paint is synchronous, so the op views over wasm memory are read before anything awaits.
  // The result joins `bitmaps` like a decode, as the painted canvas itself: a canvas is an image
  // source `drawImage` takes directly, where an `ImageBitmap` made from it would read the pixels
  // back (tens of milliseconds for a tile-sized bitmap in Chromium), and a PNG for the object
  // URL an `<img>` loads is encoded only when one asks. The answer comes back through
  // `day_dom_bitmap_rendered` on a later turn, as a decode's does, with the engine's error text
  // when it refuses (a canvas past its size limit has no 2D context).
  day_dom_canvas_render_bitmap: (req, id, ops, opsLen, strs, strsLen, pw, ph, scale) => {
    const fail = (e) => {
      const msg = utf8enc.encode(String((e && e.message) || e || 'the canvas could not render'));
      const p = msg.length ? wasm.day_dom_alloc(msg.length) : 0;
      if (msg.length) mem().set(msg, p);
      wasm.day_dom_bitmap_rendered(req, id, 0, p, msg.length);
    };
    let c;
    try {
      c = Object.assign(document.createElement('canvas'), { width: pw, height: ph });
      const ctx = c.getContext('2d');
      if (!ctx) throw new Error(`no 2D context for a ${pw}x${ph} canvas`);
      ctx.scale(scale, scale);
      const t0 = performance.now();
      paint(ctx, f64(ops, opsLen), new Uint8Array(wasm.memory.buffer, strs, strsLen));
      if (debugLog) console.debug(`DEBUG day_toolkit_dom: bitmap ${pw}x${ph} painted in ${(performance.now() - t0).toFixed(1)} ms (${opsLen} words)`);
    } catch (e) { fail(e); return; }
    bitmaps.set(id, { bitmap: c, canvas: c });
    Promise.resolve().then(() => {
      if (bitmaps.get(id)?.canvas === c) wasm.day_dom_bitmap_rendered(req, id, 1, 0, 0);
    });
  },

  // Raster images from bytes (docs/images.md). A browser decodes asynchronously, so the answer
  // comes back through the exported `day_dom_image_decoded` rather than from this call, which is
  // exactly what day-core's request-id design is for. Ids ride as f64: a wasm i64 would reach JS
  // as a BigInt, and an f64 carries a minted id exactly well past any count this page can reach.
  day_dom_image_decode: (req, id, ptr, len) => {
    // `slice` copies out of wasm memory: a Blob over the live buffer would tear the moment the
    // heap grows.
    const blob = new Blob([mem().slice(ptr, ptr + len)]);
    createImageBitmap(blob)
      .then((bitmap) => {
        bitmaps.set(id, { bitmap, url: URL.createObjectURL(blob) });
        wasm.day_dom_image_decoded(req, id, bitmap.width, bitmap.height);
      })
      .catch(() => wasm.day_dom_image_decoded(req, id, 0, 0));
  },
  day_dom_image_encode: (req, id, fmtPtr, fmtLen, quality, fitW, fitH) => {
    const b = bitmaps.get(id);
    if (!b || !b.bitmap) { wasm.day_dom_image_encoded(req, 0, 0); return; }
    const type = str(fmtPtr, fmtLen);
    let w = b.bitmap.width, h = b.bitmap.height;
    // `fit` scales the longest side down first. A box LARGER than the original is ignored:
    // upscaling on an export path inflates the bytes without adding any detail.
    if (fitW > 0 && fitH > 0) {
      const s = Math.min(fitW / w, fitH / h, 1);
      if (s < 1) { w = Math.max(1, Math.round(w * s)); h = Math.max(1, Math.round(h * s)); }
    }
    const off = typeof OffscreenCanvas === 'function' ? new OffscreenCanvas(w, h) : null;
    const canvas = off || Object.assign(document.createElement('canvas'), { width: w, height: h });
    canvas.getContext('2d').drawImage(b.bitmap, 0, 0, w, h);
    const q = quality >= 0 ? quality : undefined;
    const encoded = canvas.convertToBlob
      ? canvas.convertToBlob(q === undefined ? { type } : { type, quality: q })
      : new Promise((res, rej) =>
          canvas.toBlob((out) => (out ? res(out) : rej(new Error('encode'))), type, q));
    encoded
      .then((out) => {
        // `toBlob` substitutes PNG for a type the engine cannot write (WebKit reads WebP but
        // will not encode it). The contract is to refuse, so a substituted answer is a failure.
        if (out.type !== type) throw new Error('unwritable');
        return out.arrayBuffer();
      })
      .then((buf) => {
        const u8 = new Uint8Array(buf);
        const p = wasm.day_dom_alloc(u8.length);
        mem().set(u8, p);
        wasm.day_dom_image_encoded(req, p, u8.length);
      })
      .catch(() => wasm.day_dom_image_encoded(req, 0, 0));
  },
  // Whether the engine writes `type`, which is what `encode_formats` answers from, so an app
  // never offers a format the browser would quietly turn into PNG. Probed once per type on a 1×1
  // canvas: `toDataURL` answers synchronously and prefixes the type it actually produced.
  day_dom_image_can_encode: (fmtPtr, fmtLen) => {
    const type = str(fmtPtr, fmtLen);
    if (!encodeProbe.has(type)) {
      let ok = false;
      try {
        const c = Object.assign(document.createElement('canvas'), { width: 1, height: 1 });
        ok = c.toDataURL(type).startsWith('data:' + type);
      } catch (e) { /* an engine with no canvas here writes nothing */ }
      encodeProbe.set(type, ok);
    }
    return encodeProbe.get(type) ? 1 : 0;
  },
  day_dom_image_release: (id) => {
    const b = bitmaps.get(id);
    if (!b) return;
    if (b.url) URL.revokeObjectURL(b.url);
    if (b.bitmap && b.bitmap.close) b.bitmap.close();
    bitmaps.delete(id);
  },
  // An `<img>` showing a decoded bitmap: the object URL the decode already made, or, for a
  // rendered drawing, one encoded now from the canvas it was painted into.
  day_dom_image_set_bitmap: (el, id) => {
    const b = bitmaps.get(id);
    if (!b) return;
    if (b.url) { showImageUrl(E(el), b.url); return; }
    if (!b.canvas) return;
    b.canvas.toBlob((png) => {
      if (!png || bitmaps.get(id) !== b) return;
      b.url = URL.createObjectURL(png);
      showImageUrl(E(el), b.url);
    }, 'image/png');
  },
  // An `<img>` showing raw encoded bytes the app holds, with no staged resource behind them;
  // or, for a tinted/template glyph (a masked div, docs/vectors.md), the mask those bytes cut.
  // SVG is the one format a browser will not sniff, so markup bytes get their MIME type.
  day_dom_image_set_bytes: (el, ptr, len) => {
    const node = E(el);
    if (!node) return;
    const bytes = mem().slice(ptr, ptr + len);
    let i = 0;
    if (bytes[0] === 0xEF && bytes[1] === 0xBB && bytes[2] === 0xBF) i = 3;
    while (i < bytes.length && (bytes[i] === 0x20 || bytes[i] === 0x0A || bytes[i] === 0x0D || bytes[i] === 0x09)) i++;
    const type = bytes[i] === 0x3C ? { type: 'image/svg+xml' } : {};
    const url = URL.createObjectURL(new Blob([bytes], type));
    // Revoke the blob this element made last time: a signal-driven swap would otherwise leak one
    // URL per change, and nothing else is holding them.
    if (node.__dayBlob) URL.revokeObjectURL(node.__dayBlob);
    node.__dayBlob = url;
    showImageUrl(node, url);
  },
  // A tint over a source whose URL lives here (docs/images.md "Tinting"): on, the element's
  // mask is cut from the URL it shows, now and at every later swap (`showImageUrl`); off, the
  // swaps stop following. The mask's paint and scaling are day-toolkit-dom's (`apply_image_tint`).
  day_dom_image_mask: (el, on) => {
    const node = E(el);
    if (!node) return;
    node.__dayMasked = !!on;
    if (on && node.__dayUrl) showImageUrl(node, node.__dayUrl);
  },
  // Release an element's own blob URL when the element goes (day-toolkit-dom's `release`).
  day_dom_image_revoke: (el) => {
    const node = E(el);
    if (node && node.__dayBlob) { URL.revokeObjectURL(node.__dayBlob); node.__dayBlob = null; }
  },

  day_dom_present: (req, json, len) => present(req, JSON.parse(str(json, len))),
  day_dom_modifiers: () => modifierMask,
  day_dom_prepare_snapshot: (host, revision) => {
    const root = host ? E(host) : document.documentElement;
    if (!root || !root.isConnected) return 0;
    if (!root.__dayCapture || root.__dayCapture.revision !== revision) {
      const fence = { revision, ready: false };
      root.__dayCapture = fence;
      // Give layout/paint a turn; the browser screenshot API owns final compositor
      // synchronization. rAF alone is not proof of physical display presentation.
      requestAnimationFrame(() => requestAnimationFrame(() => { fence.ready = true; }));
    }
    return root.__dayCapture.ready ? 1 : 0;
  },
  // The browser clipboard for day-part-clipboard (docs/menus.md). Inside a live copy/cut/
  // paste DOM event (dispatched below while the event is on the stack) the calls use the
  // event's clipboardData, the synchronous path; outside one, byte operations use
  // navigator.clipboard and report permission failures to the caller.
  day_dom_clipboard_read_bytes: (req, p, n) => {
    const preferred = str(p, n).split('\n');
    const event = activeClipboardEvent;
    // Snapshot clipboardData while it is still accessible; File bytes may finish later.
    const files = event?.type === 'paste' ? Array.from(event.clipboardData?.files || []) : null;
    const strings = event?.type === 'paste' ? new Map(preferred.map(m => [m, event.clipboardData?.getData(m)])) : null;
    const precedingWrites = Array.from(pendingClipboardWrites);
    Promise.resolve().then(async () => {
      if (files) {
        for (const mime of preferred) {
          const f = files.find(f => f.type === mime);
          if (f) { if (f.size > 64*1024*1024) throw new RangeError(); return clipPacket([{mime, bytes: new Uint8Array(await f.arrayBuffer())}]); }
          if (strings.get(mime)) return clipPacket([{mime, bytes: utf8enc.encode(strings.get(mime))}]);
        }
        return clipPacket([]);
      }
      // A programmatic Paste can follow Copy before the browser has published it.
      // Wait for writes already issued at the time of this read, not future copies.
      // Native paste events above keep their own synchronous clipboard snapshot.
      await Promise.allSettled(precedingWrites);
      if (!navigator.clipboard?.read) throw new TypeError();
      const items = await navigator.clipboard.read();
      for (const mime of preferred) for (const item of items) if (item.types.includes(mime)) {
        const b = await item.getType(mime); if (b.size > 64*1024*1024) throw new RangeError();
        return clipPacket([{mime, bytes: new Uint8Array(await b.arrayBuffer())}]);
      }
      return clipPacket([]);
    }).then(bytes => clipResult(req,0,bytes), e => clipResult(req,e instanceof RangeError ? 3 : e instanceof TypeError ? 2 : 1));
  },
  day_dom_clipboard_write_bytes: (req, p, n) => {
    const data = new Uint8Array(mem().slice(p,p+n));
    const view = new DataView(data.buffer); let at = 4; const reps = [];
    for (let i=0;i<view.getUint32(0,true);i++) {
      const m=view.getUint32(at,true), b=view.getUint32(at+4,true); at+=8;
      const mime=utf8.decode(data.subarray(at,at+m)); at+=m;
      reps.push({mime,bytes:data.slice(at,at+b)}); at+=b;
    }
    const event = activeClipboardEvent;
    const native = reps.filter(r => typeof ClipboardItem !== 'undefined' && ClipboardItem.supports?.(r.mime));
    // Preserve app-specific/SVG strings synchronously for normal copy events. Binary
    // representations use the async system API, whose permission failures are reported.
    const eventTypes=[];
    if (event && event.type !== 'paste') for (const r of reps) {
      if (r.mime.startsWith('text/') || r.mime.endsWith('+svg') || r.mime === 'image/svg+xml') {
        event.clipboardData.setData(r.mime,utf8.decode(r.bytes)); eventTypes.push(r.mime);
      }
    }
    if (native.length && navigator.clipboard?.write) {
      const item = new ClipboardItem(Object.fromEntries(native.map(r => [r.mime,new Blob([r.bytes],{type:r.mime})])));
      const pending = navigator.clipboard.write([item]);
      pendingClipboardWrites.add(pending);
      // Track settlement separately from result delivery, and consume rejection so
      // a failed write does not prevent a later read of the existing clipboard.
      pending.then(() => pendingClipboardWrites.delete(pending), () => pendingClipboardWrites.delete(pending));
      pending.then(() => clipResult(req,0,utf8enc.encode(native.map(r=>r.mime).join('\n'))), () => {
        clipResult(req,eventTypes.length ? 0 : 1,utf8enc.encode(eventTypes.join('\n')));
      });
    } else queueMicrotask(() => clipResult(req,eventTypes.length ? 0 : 2,utf8enc.encode(eventTypes.join('\n'))));
  },
  day_dom_clipboard_set: (p, n) => {
    const text = str(p, n);
    clipboardMirror = text;
    if (activeClipboardEvent && activeClipboardEvent.type !== 'paste' && activeClipboardEvent.clipboardData) {
      activeClipboardEvent.clipboardData.setData('text/plain', text);
    } else {
      navigator.clipboard?.writeText(text).catch(() => {});
    }
    return 1;
  },
  day_dom_clipboard_get: () => {
    let text = null;
    if (activeClipboardEvent && activeClipboardEvent.type === 'paste' && activeClipboardEvent.clipboardData) {
      text = activeClipboardEvent.clipboardData.getData('text/plain') || null;
    }
    text = text ?? clipboardMirror;
    if (text == null) return -1;
    clipboardStaged = utf8enc.encode(text);
    return clipboardStaged.length;
  },
  day_dom_clipboard_take: (p) => {
    if (clipboardStaged) new Uint8Array(wasm.memory.buffer, p, clipboardStaged.length).set(clipboardStaged);
    clipboardStaged = null;
  },
  day_dom_clipboard_has: () => {
    if (activeClipboardEvent && activeClipboardEvent.type === 'paste' && activeClipboardEvent.clipboardData) {
      return activeClipboardEvent.clipboardData.types.includes('text/plain') ? 1 : 0;
    }
    return clipboardMirror != null ? 1 : 0;
  },
  // Host services: wall-clock time and entropy (day-toolkit-dom's `getrandom` bridge, docs/web.md).
  day_dom_now_ms: () => Date.now(),
  day_dom_entropy: (p, n) => crypto.getRandomValues(new Uint8Array(wasm.memory.buffer, p, n)),
  // Save flow: bytes must be copied out before this call returns (Rust frees them after).
  day_dom_present_save: (req, json, len, bytes, blen) =>
    presentSave(req, JSON.parse(str(json, len)), new Uint8Array(wasm.memory.buffer, bytes, blen).slice()),
  day_dom_dismiss(req) { dialogs.get(req)?.close('day-dismiss'); },

  day_dom_nav_mode(id, mode, t, tl) { navChrome(E(id), id, mode); },
  // Re-present a live host after a size-class change (docs/size-classes.md). The chrome is
  // rebuilt, but the pages are not: detaching an element leaves it in `els`, so each page keeps
  // its DOM subtree (and with it every scroll offset, text selection, and focused field)
  // until Day re-homes it with `day_dom_nav_add_page`.
  day_dom_nav_present(id, mode) {
    const nav = E(id);
    nav.textContent = '';
    nav.classList.remove(...NAV_MODES);
    nav.__side = nav.__detail = nav.__bar = nav.__title = undefined;
    navChrome(nav, id, mode);
    applyNavBar(nav);
  },
  day_dom_nav_add_page(nav, page, chrome) {
    const n = E(nav);
    // A stack has no chrome slot: its rows page is the stack root and lands in the detail area.
    (chrome && n.__side ? n.__side : n.__detail).append(E(page));
  },
  day_dom_nav_back_bar(nav, visible, t, tl) {
    const n = E(nav); if (!n.__bar) return;
    n.__barState = { visible: !!visible, title: str(t, tl) };
    applyNavBar(n);
  },

  day_dom_navmenu(id, json, len) {
    const el = E(id); const spec = JSON.parse(str(json, len));
    // The keyboard route, armed once and kept across every rebuild below (the rows are thrown
    // away and remade; the host is not). A sidebar is the web's stand-in for the outline view
    // the desktops give this piece, and an outline walks with the arrow keys.
    if (!el.__dayKeynav) {
      el.__dayKeynav = true;
      el.tabIndex = 0;
      el.setAttribute('role', 'listbox');
      el.addEventListener('pointerdown', () => {
        if (document.activeElement !== el) el.focus({ preventScroll: true });
      });
      el.addEventListener('keydown', (e) => {
        if (e.metaKey || e.ctrlKey || e.altKey) return;
        // Rows only: a section heading sits between them but is not one of Day's rows, and the
        // arrow keys step straight past it.
        const rows = [...el.querySelectorAll(':scope > .day-navmenu-row')];
        const last = rows.length - 1;
        if (last < 0) return;
        const cur = rows.findIndex((r) => r.classList.contains('selected'));
        let next;
        if (e.key === 'ArrowUp') next = cur < 0 ? last : Math.max(cur - 1, 0);
        else if (e.key === 'ArrowDown') next = cur < 0 ? 0 : Math.min(cur + 1, last);
        else if (e.key === 'Home') next = 0;
        else if (e.key === 'End') next = last;
        else return;
        // Claimed even when the selection cannot move, so the page does not scroll instead.
        e.preventDefault();
        // Report only, exactly as the click handler does: which row ends up selected is the
        // app's route to decide, and it comes back through day_dom_navmenu_select.
        if (next !== cur) {
          wasm.day_dom_event(id, 6, next, 0, 0, 0);
          keepFocus(el);
        }
      });
    }
    el.textContent = '';
    spec.items.forEach((item, i) => {
      // The section heading that opens a group before this row (docs/navigation.md). A sibling
      // of the rows rather than a wrapper around them, so the rows stay direct children in Day's
      // order and `i` stays their index everywhere: the click below, the arrow keys above,
      // day_dom_navmenu_select. Tabs and rails hide it in CSS; the grouping is a sidebar's.
      if (item.section) {
        const heading = div('day-navmenu-heading');
        heading.setAttribute('role', 'presentation');
        heading.textContent = item.section;
        el.append(heading);
      }
      const row = div('day-navmenu-row');
      row.setAttribute('role', 'option');
      row.setAttribute('aria-selected', i === spec.selected ? 'true' : 'false');
      fillNavRow(row, item);
      if (i === spec.selected) row.classList.add('selected');
      row.addEventListener('click', () => wasm.day_dom_event(id, 6, i, 0, 0, 0));
      el.append(row);
    });
  },
  // New words or decorations for the listed rows of the same row set (NavMenuPatch::Items
  // `rows_changed`): refilled in place, so the row elements, the selection and the keyboard focus
  // all stay where they are.
  day_dom_navmenu_rows(id, json, len) {
    const spec = JSON.parse(str(json, len));
    const rows = [...E(id).querySelectorAll(':scope > .day-navmenu-row')];
    for (const i of spec.rows) {
      const row = rows[i];
      if (!row || !spec.items[i]) continue;
      row.textContent = '';
      fillNavRow(row, spec.items[i]);
    }
  },
  // --- window toolbar (docs/toolbars.md) -----------------------------------
  // The web has no window chrome, so the bar is a strip docked above the app root. It is edited
  // one item at a time, the way every native backend edits its bar: an op adds or removes one
  // element, and nothing else in the strip is touched, so a search field being typed into keeps
  // its focus while the page commands around it change. day_dom_toolbar_patch carries the
  // targeted value changes.
  day_dom_toolbar_edit(host, json, len) {
    const strip = stripFor(host);
    const spec = JSON.parse(str(json, len));
    for (const op of spec.ops) {
      if (op.op === 'remove') {
        const at = strip.order.findIndex((e) => e.id === op.id);
        if (at < 0) continue;
        const e = strip.order[at];
        if (e.el.__datalist) e.el.__datalist.remove();
        e.el.remove();
        strip.order.splice(at, 1);
        delete strip.items[op.id];
      } else if (op.op === 'insert') {
        if (!strip.bar) openToolbar(strip);
        const it = op.item;
        const el = buildToolbarItem(strip, it);
        // Within a track: leading roles first, then the trailing ones pushed to that column's own
        // right edge. `Principal` centers, the way it does on every other backend.
        if (it.place === 'primary' || it.place === 'secondary' || it.place === 'auto') {
          el.classList.add('trailing');
        }
        if (it.place === 'mid') el.classList.add('principal');
        const col = it.col || 'detail';
        const track = toolbarTrack(strip, col);
        // Right after the nearest earlier item in the same track, or first in it.
        const at = Math.min(op.index, strip.order.length);
        let prev = null;
        for (let k = at - 1; k >= 0; k--) {
          if (strip.order[k].track === track) { prev = strip.order[k]; break; }
        }
        if (prev) prev.el.after(el); else track.prepend(el);
        strip.order.splice(at, 0, { id: it.id, el, col, track });
        if (it.id) strip.items[it.id] = el;
      }
    }
    if (strip.bar && !strip.order.length && !strip.menuBtn) closeToolbarIfEmpty(strip);
    else layoutToolbar(strip);
  },
  // A cover's strip height, for the frame Day seeds the cover with at present time (the
  // ResizeObserver reports the settled size a frame later).
  day_dom_toolbar_height(host) {
    const strip = coverStrips.get(els[host]);
    return strip && strip.bar ? strip.bar.offsetHeight : 0;
  },
  // The emulated app menu (docs/menus.md "The composed presentation (web-dom)"): the model as
  // a trailing pull-down button on the strip, presented by the same popover a toolbar menu
  // item gets, so items, separators, inlined submenus, check marks, disabled rows, the
  // outside-press and Escape dismissals are one implementation. Re-sent whole on every
  // install (a locale change, a moved check mark); an empty list takes the button away.
  day_dom_app_menu(json, len) {
    const spec = JSON.parse(str(json, len));
    const strip = pageStrip;
    if (strip.menuBtn) {
      if (toolbarMenu && toolbarMenu.__anchor === strip.menuBtn) closeToolbarMenu();
      strip.menuBtn.remove();
      strip.menuBtn = null;
    }
    appMenuKeys = collectAppMenuKeys(spec.items, []);
    if (!spec.items.length) { closeToolbarIfEmpty(strip); return; }
    if (!strip.bar) openToolbar(strip);
    const el = document.createElement('button');
    el.className = 'day-toolbar-btn day-app-menu';
    el.id = 'day-app-menu';
    el.title = spec.label;
    el.setAttribute('aria-label', spec.label);
    el.setAttribute('aria-haspopup', 'menu');
    if (spec.icon) {
      const ic = div('day-toolbar-icon');
      ic.style.maskImage = `url("${resourceUrl(spec.icon)}")`;
      ic.style.webkitMaskImage = `url("${resourceUrl(spec.icon)}")`;
      el.append(ic);
    } else {
      const t = document.createElement('span'); t.textContent = spec.label; el.append(t);
    }
    el.addEventListener('click', (e) => { e.stopPropagation(); toggleToolbarMenu(el, spec.items, true); });
    toolbarTrack(strip, 'detail').append(el);
    strip.menuBtn = el;
    layoutToolbar(strip);
  },
  // A menu button's press (docs/menus.md "Menu buttons"): the items its provider built just
  // now, under the button, in the popover a toolbar pull-down gets. A second press closes it.
  day_dom_button_menu(el, json, len) {
    const anchor = els[el];
    if (anchor) toggleToolbarMenu(anchor, JSON.parse(str(json, len)));
  },
  day_dom_toolbar_patch(host, json, len) {
    closeToolbarMenu();
    const p = JSON.parse(str(json, len));
    const el = stripFor(host).items[p.item];
    if (!el) return;
    if (p.text !== undefined && el.value !== p.text) el.value = p.text;
    if (p.text !== undefined && '__sent' in el) el.__sent = p.text;
    if (p.on !== undefined) el.setAttribute('aria-pressed', p.on ? 'true' : 'false');
    // A segmented item: move the selection without firing its click handler (see the toggle
    // echo the native backends guard against).
    if (p.selected !== undefined && el.classList.contains('day-segmented')) selectAmong(el, p.selected);
    if (p.enabled !== undefined && el.classList.contains('day-segmented')) {
      [...el.children].forEach((b) => { b.disabled = !p.enabled; });
    }
    if (p.enabled !== undefined) el.disabled = !p.enabled;
    if (p.focus) { el.focus(); el.select?.(); }
    if (p.suggestions !== undefined && el.__datalist) setSuggestions(el.__datalist, p.suggestions);
    // New words (a language change): an icon button names itself by aria-label, a text button
    // by its span, a label by its text; the tooltip follows either way.
    if (p.label !== undefined) {
      el.title = p.tip || p.label;
      if (el.classList.contains('day-toolbar-label')) el.textContent = p.label;
      else if (el.hasAttribute('aria-label')) el.setAttribute('aria-label', p.label);
      else { const t = el.querySelector(':scope > span'); if (t) t.textContent = p.label; }
    }
    if (p.placeholder !== undefined) el.placeholder = p.placeholder;
  },
  // Show/hide the split nav's sidebar. 0 when this page has no split nav, which is how the
  // caller (and the dayscript duty) knows to report the item disabled.
  day_dom_toolbar_sidebar() {
    const nav = document.querySelector('.day-nav.split');
    if (!nav) return 0;
    nav.classList.toggle('day-sidebar-hidden');
    // Day frames the panes itself; the CSS only decides how much room the detail has, not how
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
    // Rows only, in Day's order; the section headings between them are not addressed by index.
    [...E(id).querySelectorAll(':scope > .day-navmenu-row')].forEach((row, i) => {
      row.classList.toggle('selected', i === idx);
      row.setAttribute('aria-selected', i === idx ? 'true' : 'false');
    });
  },

  day_dom_set_hash(ptr, len, replace) {
    const route = str(ptr, len);
    const url = route ? '#' + route : location.pathname + location.search;
    if (replace || route === location.hash.slice(1)) history.replaceState(null, '', url);
    else if (route) location.hash = route;
    else history.pushState(null, '', url);
    // URL assignment canonicalizes spaces, quotes and non-ASCII characters. Compare the
    // browser's actual hash on the echo, not the pre-normalized app route.
    lastSetRoute = location.hash.slice(1);
  },

  // Motion sensors (docs/sensors.md): the browser arm of day-part-sensors, over `devicemotion`.
  //
  // One listener feeds both kinds, since the event carries acceleration and rotation together. The
  // magnetometer has no cross-browser API at all (Chromium's Generic Sensor `Magnetometer` is
  // flag-gated and absent from Safari and Firefox), so kind 2 is always unavailable.
  //
  // API presence is not proof of hardware or permission. Only events produce readings.
  day_dom_sensor_start(kind, generation) {
    if (kind > 1 || !env.day_dom_sensor_available(kind)) return;
    sensorState.generations[kind] = generation;
    if (sensorState.listener) return;
    sensorState.listener = e => {
      // Capture both epochs before invoking Rust: callbacks may stop/restart either kind.
      const [accel, gyro] = sensorState.generations;
      const valid = values => values.every(v => typeof v === 'number' && Number.isFinite(v));
      const a = e.accelerationIncludingGravity;
      if (accel && a && valid([a.x, a.y, a.z]))
        wasm.day_sensors_sample(0, accel, a.x, a.y, a.z);
      const r = e.rotationRate;
      if (gyro && r && valid([r.beta, r.gamma, r.alpha])) {
        const d = Math.PI / 180;
        wasm.day_sensors_sample(1, gyro, r.beta * d, r.gamma * d, r.alpha * d);
      }
    };
    addEventListener('devicemotion', sensorState.listener);
  },
  day_dom_sensor_stop(kind, generation) {
    if (kind > 1 || sensorState.generations[kind] !== generation) return;
    sensorState.generations[kind] = 0;
    if (!sensorState.generations.some(Boolean) && sensorState.listener) {
      removeEventListener('devicemotion', sensorState.listener);
      sensorState.listener = null;
    }
  },
  day_dom_sensor_available(kind) {
    return kind < 2 && globalThis.isSecureContext === true && typeof DeviceMotionEvent !== 'undefined' ? 1 : 0;
  },

  // Location (docs/location.md): the browser arm of day-part-location, over
  // `navigator.geolocation.watchPosition`. The browser's API is already a subscription with an
  // error channel, so it maps almost one-to-one.
  //
  // A field the browser did not measure is `null`, which crosses to Rust as NaN; the part turns a
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

  // Preferences (docs/prefs.md): the browser arm of day-part-preferences. localStorage can throw
  // (private browsing, storage pressure); failures report as absent/uncommitted, matching
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

  // App-local files (docs/fs.md): the browser arm of day-part-filesystem, stored in the Origin
  // Private File System, an origin-scoped file hierarchy.
  // One operation per request id (op: 0 read, 1 write, 2 remove, 3 list); the completion
  // re-enters wasm exactly once: day_fs_done (bytes; list joins names with \u001f,
  // directories carrying a trailing slash) or day_fs_failed (kind 1 NotFound, 2 no OPFS in
  // this context, meaning pre-OPFS browsers and private-browsing/ephemeral sessions, which WebKit
  // gives no storage backing; 0 everything else). Request buffers are copied out before the
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
    if (!scriptWs) return; // scripting not armed, so nothing is listening
    if (scriptWs.readyState === WebSocket.OPEN) scriptWs.send(line);
    else scriptOutbox.push(line);
  },

  day_dom_schedule_post: () => queueMicrotask(() => wasm.day_dom_posted()),
  day_dom_schedule_delayed: (token, ms) => setTimeout(() => wasm.day_dom_delayed(token), ms),
  day_dom_request_frame: (token) => requestAnimationFrame((t) => wasm.day_dom_frame(token, t / 1000)),
  day_dom_cancel_frame: (id) => cancelAnimationFrame(id),
  day_dom_set_title(ptr, len) { document.title = str(ptr, len); },
  // App badge (docs/badge.md). The Badging API is Chromium + Safari-for-installed-PWAs; Firefox
  // has none, so every call is feature-guarded. `count < 0` clears. The promises are ignored:
  // a rejection here (not installed, insecure context) is not something the app can act on.
  // Fullscreen (docs/windows.md "Window properties"), on the document element so the toolbar
  // and dialogs come along. `on` 1 enters, 0 leaves. Returns -1 while the browser holds the
  // request: `fullscreenchange` (start()) reports the outcome, and a refusal (no user gesture,
  // a permissions policy) reports the state the page kept as a declined request. Otherwise
  // nothing was asked and the answer is the current state code (0 Normal, 3 Fullscreen).
  day_dom_set_fullscreen(on) {
    const now = fullscreenCode();
    if (!document.fullscreenEnabled || (on === 1) === (now === 3)) return now;
    try {
      const request = on === 1 ? document.documentElement.requestFullscreen() : document.exitFullscreen();
      Promise.resolve(request).catch(() => wasm.day_dom_window_state(fullscreenCode(), 1));
      return -1;
    } catch (_) {
      return fullscreenCode();
    }
  },
  // The primary window's chrome (docs/window-chrome.md), once at boot. A browser draws its own
  // frame; the one chrome a page can change is the title bar of an installed Chromium app whose
  // manifest lists `window-controls-overlay` in `display_override`, and only after the user
  // hides the title bar with its toggle. `overlay` 1: the content runs under the overlay; its
  // height is the answer (Rust seeds the safe area with it before the root builds) and every
  // `geometrychange` reports it again as the safe-area top inset. 0 (Standard): the body is
  // pushed below the overlay, behind a draggable strip, so an app that never asked for an
  // overlay still looks the same when the user turns it on.
  day_dom_window_chrome(overlay) {
    installChromeStyle();
    document.documentElement.classList.add(overlay ? 'day-chrome-overlay' : 'day-chrome-standard');
    const wco = navigator.windowControlsOverlay;
    if (!overlay || !wco) return 0;
    wco.addEventListener('geometrychange', () => wasm.day_dom_titlebar_inset(titlebarInset()));
    return titlebarInset();
  },
  day_dom_set_app_badge(count) {
    try {
      if (count < 0) { navigator.clearAppBadge?.(); }
      else if (count === 0) { navigator.setAppBadge?.(); }   // no argument = a dot
      else { navigator.setAppBadge?.(count); }
    } catch (_) { /* unsupported or blocked; the cap already says Emulated */ }
  },
  // Screen-reader announcements (docs/accessibility.md): two ARIA live regions, one per
  // politeness, made on first use and kept for the document's life. Visually hidden by a
  // clipped 1px box (`.day-live`), never `display:none`, which would drop them from the
  // accessibility tree. The text is cleared first and set on the next frame, so the same
  // sentence twice in a row is still a change the screen reader speaks. Nothing is spoken
  // when no screen reader runs: a live region is inert without one.
  day_dom_announce(p, l, urgent) {
    const kind = urgent ? 'assertive' : 'polite';
    liveRegions[kind] ||= (() => {
      const r = div('day-live');
      r.setAttribute('aria-live', kind);
      r.setAttribute('aria-atomic', 'true');
      document.body.append(r);
      return r;
    })();
    const r = liveRegions[kind]; const text = str(p, l);
    r.textContent = '';
    requestAnimationFrame(() => { r.textContent = text; });
  },
  day_dom_share_support() { return typeof navigator.share === 'function'; },
  day_dom_share_url(ptr, len, title, titleLen) {
    if (!navigator.share || !navigator.userActivation?.isActive) return false;
    try {
      const data = { url: str(ptr, len), title: str(title, titleLen) };
      if (navigator.canShare && !navigator.canShare(data)) return false;
      navigator.share(data).catch(() => {}); // Cancellation is the user's choice.
      return true;
    } catch (_) { return false; }
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
      // The user's reduce-motion setting, as the browser relays it (docs/accessibility.md).
      case 'reduce_motion': v = reduceMotion().matches ? '1' : ''; break;
      // Whether the browser has the Window Controls Overlay API (Chromium on the desktop), the
      // one place a title-bar overlay and `app-region` drag regions can take effect.
      case 'wco': v = navigator.windowControlsOverlay ? '1' : ''; break;
      // Whether the page may go fullscreen at all (Cap::WindowFullscreen).
      case 'fullscreen': v = document.fullscreenEnabled ? '1' : ''; break;
      case 'locales': v = (q.get('locale') ? [q.get('locale')] : navigator.languages).join(','); break;
      case 'day_url': v = q.get('day_url') || ''; break;
      case 'route': v = location.hash.slice(1) || q.get('route') || ''; break;
      // The browser's IANA time zone (a zone-aware app's local-zone source on web).
      // `?tz=` overrides for testing, mirroring the other reserved keys.
      case 'tz': v = q.get('tz') ?? (Intl.DateTimeFormat().resolvedOptions().timeZone || ''); break;
      // The offset that zone is running at, in minutes east of UTC, for the current instant.
      case 'tzoffset': v = String(tzOffsetMinutes(Date.now())); break;
      // Which color functions the engine parses, for day-toolkit-dom's `ink_css`: `d` for
      // `light-dark()`, `r` for relative color syntax.
      case 'css_colors':
        v = (CSS.supports('color', 'light-dark(#000, #fff)') ? 'd' : '')
          + (CSS.supports('color', 'rgb(from #000 r g b / 0.5)') ? 'r' : '');
        break;
      default:
        if (key.startsWith('system_color_')) {
          // The sixteen roles in wire order (`SystemColor::ALL`), as day.css defines them.
          const variable = SYSTEM_COLOR_VARS[Number(key.slice(13))];
          if (variable) v = cssColorParts(`var(${variable})`);
          break;
        }
        // Reserved `vector:` keys answer "is `<name>` a bundled vector glyph?" from the list
        // the assemble step injects into the page (docs/vectors.md): '1' or empty.
        // `tzoffset:<epoch-ms>` answers for a given moment rather than for now: daylight saving
        // makes the offset a function of the instant, which is what a "local midnight" boundary
        // needs. The browser's tz database answers, so a web app bundles none.
        if (key.startsWith('tzoffset:')) {
          const ms = Number(key.slice(9));
          v = Number.isFinite(ms) ? String(tzOffsetMinutes(ms)) : '';
          break;
        }
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
  // An element read back for `a11y_audit` and `assert_native` (docs/testing.md), by the
  // day_dom_env buffer protocol, as fields joined by U+001F:
  //   role, label, value, id — the ARIA attributes, where the value is `aria-valuetext` or else
  //     the description (a non-range role has no value slot of its own, so Day folds its value
  //     into the description);
  //   number, checked, enabled, visible — "" when this element has none, booleans as 1/0;
  //   x, y, w, h — the frame in CSS px relative to #day-root, all "" when it has no box;
  //   text — LAST, since a field's contents may hold any character: "" when the element shows
  //     no text of its own (or masks it), else "=" followed by the text.
  // The ARIA attributes and the value-carrying state live on V (the toggle's <input>); the box
  // is E's, since the toggle's input is an invisible overlay on its wrapper. An empty answer is
  // an element the page no longer has.
  day_dom_read_native(id, out, cap) {
    const el = V(id);
    const box = E(id);
    let v = '';
    if (el) {
      const a = (n) => el.getAttribute(n) ?? '';
      const tag = el.tagName;
      let text = null;
      // An elided label (head or middle truncation, docs/text.md) shows a shorter copy and keeps
      // the text it stands for in `data-day-text`.
      if (box.classList.contains('day-label')) text = box.dataset.dayText ?? box.textContent;
      else if (tag === 'BUTTON' && el.classList.contains('day-btn')) {
        // An icon button (apply_button_content) holds a hidden glyph span, then the title in a
        // span of its own; icon-only shows no title at all.
        const icon = el.querySelector(':scope > span[aria-hidden="true"]');
        if (!icon) text = el.textContent;
        else text = el.querySelector(':scope > span:not([aria-hidden])')?.textContent ?? null;
      } else if (tag === 'INPUT' && el.type === 'text') text = el.value;
      else if (tag === 'TEXTAREA') text = el.value;
      else if (tag === 'SELECT') text = el.selectedOptions[0]?.text ?? null;
      else if (isChoiceGroup(el)) text = el.querySelector(':scope > .selected')?.textContent ?? null;
      let number = '';
      if (tag === 'INPUT' && el.type === 'range') number = String(Number(el.value));
      else if (tag === 'PROGRESS') number = String(el.position >= 0 ? el.value : '');
      // A toggle's state, or a button's selected state (day-toolkit-dom's set_button_selected).
      const checked = tag === 'INPUT' && el.type === 'checkbox' ? (el.checked ? '1' : '0')
        : tag === 'BUTTON' && el.hasAttribute('aria-pressed') ? (el.getAttribute('aria-pressed') === 'true' ? '1' : '0') : '';
      let enabled = '';
      if (el.matches('button, input, select, textarea')) enabled = el.matches(':disabled') ? '0' : '1';
      else if (isChoiceGroup(el)) enabled = el.hasAttribute('disabled') ? '0' : '1';
      let frame = ['', '', '', ''];
      let visible = '0';
      if (box.isConnected && box.getClientRects().length > 0) {
        const r = box.getBoundingClientRect();
        const o = root().getBoundingClientRect();
        frame = [r.left - o.left, r.top - o.top, r.width, r.height].map(String);
        let shown = r.width > 0 && r.height > 0;
        if (shown && box.checkVisibility) shown = box.checkVisibility({ visibilityProperty: true });
        else if (shown) {
          for (let n = box; n && n.nodeType === 1; n = n.parentElement) {
            const cs = getComputedStyle(n);
            if (cs.display === 'none' || (n === box && cs.visibility !== 'visible')) { shown = false; break; }
          }
        }
        visible = shown ? '1' : '0';
      }
      v = [a('role'), a('aria-label'), a('aria-valuetext') || a('aria-description'), el.id || '',
        number, checked, enabled, visible, ...frame, text === null ? '' : '=' + text]
        .join('\u001f');
    }
    const bytes = utf8enc.encode(v).slice(0, cap);
    mem().set(bytes, out);
    return bytes.length;
  },
  // The font list (docs/fonts.md), by the day_dom_env buffer protocol: a truncated answer
  // returns the buffer's size, and the wasm side retries with a larger one.
  day_dom_fonts(out, cap) {
    const bytes = utf8enc.encode(fontListText()).slice(0, cap);
    mem().set(bytes, out);
    return bytes.length;
  },
  // One line of canvas text measured in the font the replay draws it in, into eight f64 at `out`:
  // width, the line box (ascent + descent), the ascent, the cap height, then the ink box relative
  // to the line box's top-leading corner (docs/fonts.md).
  day_dom_canvas_measure_text(text, len, size, weight, italic, fam, famLen, out) {
    const ctx = measureContext();
    const t = str(text, len);
    ctx.font = canvasFont(size, weight, italic, famLen > 0 ? str(fam, famLen) : '');
    const [asc, desc] = fontBox(ctx, t, size);
    const m = ctx.measureText(t);
    const width = m.width;
    // Canvas2D's `actualBoundingBox*` is the ink box, reported from the drawing origin on the
    // baseline with left/ascent measured positive in the -x / -y directions.
    const num = (v, fallback) => (Number.isFinite(v) ? v : fallback);
    const inkL = -num(m.actualBoundingBoxLeft, 0);
    const inkR = num(m.actualBoundingBoxRight, width);
    const inkA = num(m.actualBoundingBoxAscent, asc);
    const inkD = num(m.actualBoundingBoxDescent, desc);
    // No cap-height metric that Safari reports, so take the ink ascent of a capital, which is
    // exactly what a cap height is. Memoized per (size, font) on day's side.
    const capM = ctx.measureText('H');
    const cap = num(capM.actualBoundingBoxAscent, size * 0.7);
    new Float64Array(wasm.memory.buffer, out, 8).set([
      width, asc + desc, asc, cap,
      inkL, asc - inkA, inkR - inkL, inkA + inkD,
    ]);
  },
  day_dom_warn: (ptr, len) => console.warn(str(ptr, len)),
  // Logging (docs/logging.md). std's stdout/stderr on wasm32-unknown-unknown accept bytes and
  // drop them, so the console is the only sink a page has. `level` is log's ordering (1 Error,
  // 2 Warn, 3 Info, 4 Debug, 5 Trace) mapped onto the console methods a browser filters by, so
  // the devtools level selector works on Day's output the way it does on the page's own.
  day_dom_log: (level, ptr, len) => {
    const msg = str(ptr, len);
    if (level <= 1) console.error(msg);
    else if (level === 2) console.warn(msg);
    else if (level === 3) console.info(msg);
    else if (level === 4) console.debug(msg);
    else console.debug(msg);
  },
  // Wall clock for the wasm side (SystemTime::now() aborts on wasm32-unknown-unknown).
  day_dom_now_ms: () => Date.now(),
  // Monotonic clock for the wasm side (Instant::now() aborts there too).
  day_dom_perf_now: () => performance.now(),

  // Appearance override (Toolkit::set_appearance): 0 light, 1 dark, 2 follow the browser.
  // Returns the effective mode so the wasm side's dark_mode cache stays truthful.
  day_dom_set_dark(mode) {
    appearanceMode = mode;
    return showAppearance(mode === 2 ? prefersDark().matches : mode === 1);
  },
  // Window frame and screen (docs/windows.md "Monitors"), in CSS px on the desktop.
  day_dom_window_frame(out) {
    if (!(outerWidth > 0 && outerHeight > 0)) return 0;
    f64(out, 4).set([screenX, screenY, outerWidth, outerHeight]);
    return 1;
  },
  day_dom_screen(out) {
    const s = window.screen;
    const x = s.left ?? 0, y = s.top ?? 0;
    f64(out, 9).set([
      x, y, s.width, s.height,
      s.availLeft ?? x, s.availTop ?? y, s.availWidth, s.availHeight,
      devicePixelRatio || 1,
    ]);
  },
};

/** How far the title-bar overlay reaches into the page, in CSS px; 0 while it is hidden. */
function titlebarInset() {
  const wco = navigator.windowControlsOverlay;
  if (!wco?.visible) return 0;
  const r = wco.getTitlebarAreaRect();
  return r.y + r.height;
}

// Drag regions (`.window_drag_region()`, docs/window-chrome.md): `app-region: drag` on the
// region, and `no-drag` on the controls inside it so they keep their clicks. Chromium honors it
// only inside an installed app's window-controls overlay, where a press moves the window and a
// double click maximizes it; everywhere else the region is an ordinary element. The Standard
// rules keep an app that did not ask for an overlay below it (see day_dom_window_chrome).
// `env(titlebar-area-*)` is undefined, so the fallbacks apply, while no overlay shows.
let chromeStyle = null;
function installChromeStyle() {
  if (chromeStyle) return;
  const drag = '-webkit-app-region: drag; app-region: drag;';
  const noDrag = '-webkit-app-region: no-drag; app-region: no-drag;';
  chromeStyle = document.createElement('style');
  chromeStyle.textContent = `
.day-drag-region { ${drag} }
.day-drag-region :is(button, input, select, textarea, a, [contenteditable], [tabindex],
  [role=button], [role=link], [role=checkbox], [role=switch], [role=slider], [role=option],
  [role=tab], [role=menuitem], [role=listbox], [role=textbox]) { ${noDrag} }
html.day-chrome-standard body { box-sizing: border-box; padding-top: env(titlebar-area-height, 0px); }
html.day-chrome-standard body::before {
  content: ""; position: fixed; z-index: 2147483647;
  left: env(titlebar-area-x, 0px); top: env(titlebar-area-y, 0px);
  width: env(titlebar-area-width, 0px); height: env(titlebar-area-height, 0px); ${drag}
}`;
  document.head.append(chromeStyle);
}

/** The page's display state as a `WindowState` code: 3 Fullscreen, else 0 Normal. */
function fullscreenCode() {
  return document.fullscreenElement ? 3 : 0;
}

function selectAmong(group, idx) {
  [...group.children].forEach((b, i) => b.classList.toggle('selected', i === idx));
}

/** A segmented or inline (radio) picker: a `<div>` whose children are the choice buttons. */
function isChoiceGroup(el) {
  return el.classList.contains('day-segmented') || el.classList.contains('day-radios');
}

// ---- day-part-filesystem store (see day_dom_fs_start) ---------------------------------------------

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

// ---------------------------------------------------------------------------
// Events (mirrors mod ev in lib.rs)
// ---------------------------------------------------------------------------

function mods(e) { return (e.ctrlKey || e.metaKey ? 1 : 0) | (e.shiftKey ? 2 : 0); }

// ---------------------------------------------------------------------------
// Styled-text editing (day-piece-text-editor, docs/texteditor.md).
//
// A `contenteditable` element is the browser's rich text editing: IME, undo, spell-check,
// drag-and-drop and the accessibility tree all come with it, and none of them can be rebuilt in a
// canvas. What it does not give is a stable document model: pressing Enter inserts a <div> in one
// browser and a <p> in another, and a paste brings whatever markup it came with. So Day reads the
// DOM through one flattening (dayEditorText) and writes it back in one canonical shape (spans
// inside <p> blocks), and every offset it exchanges with Rust is a UTF-8 byte offset, because
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

// The DOM ⇄ flat-text mapping, and the one traversal all of it goes through.
//
// Day's serializer writes one block per line of the document, so read backwards the text is the
// blocks' text joined with "\n"; an empty block is an empty line and still contributes its
// separator. Inside a block a <br> is a line break too, except the filler one above.
//
// Both directions must agree byte for byte, which is why they share this walk rather than each
// having their own. When they disagreed (the locator counted only text nodes) two things broke:
// restoring a selection after a restyle landed one character further right per preceding line,
// and the first keystroke reported a text that had lost every blank line, which Day then read
// as a deletion and reflowed the paragraph runs onto the wrong lines.
//
// `onEvent(kind, node)` sees, in document order: 'break' where the text gains a "\n", 'text' for
// each text node, and 'enter' for each element (which is how a position inside an empty block is
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

// A selection endpoint can be an element with a child index rather than a text node with a
// character offset (a triple-click, or a caret sitting in an empty block). Resolve it to what it
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

// A DOM position as a byte offset into the flattened text.
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
      // A block starting exactly at the target with no text of its own is the position: an
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

// Insert text at a byte offset of the flattened text, the way typing would: the caret is put
// there and `insertText` goes through the browser's editing command, so the undo stack records
// it and the `input` event reports the result. This is the one `execCommand` in the shim: it is
// the only insertion a browser records for undo, and it inserts plain text, not markup, so the
// reason the rest of this file avoids the API (normalizing away `<b>`/`<font>`) does not apply.
// Where it is refused (a read-only element, a browser without it), the text goes in as a node
// and the input event is raised by hand, which keeps the report but not the undo entry.
function dayEditorInsert(el, atByte, text) {
  if (!text) return;
  dayEditorSelect(el, atByte, atByte, true);
  el.focus({ preventScroll: true });
  let done = false;
  try { done = document.execCommand('insertText', false, text); } catch { done = false; }
  if (done) return;
  const sel = window.getSelection();
  if (!sel || sel.rangeCount === 0) return;
  const range = sel.getRangeAt(0);
  range.deleteContents();
  const node = document.createTextNode(text);
  range.insertNode(node);
  range.setStartAfter(node);
  range.collapse(true);
  sel.removeAllRanges();
  sel.addRange(range);
  el.dispatchEvent(new Event('input', { bubbles: true }));
}

// Replace the markup, keeping the caret. Day sends fresh HTML on every attribute change (a
// syntax highlighter does it per keystroke), so preserving the caret here is what makes the arm
// usable at all. A rewrite during IME composition is skipped: replacing the nodes mid-composition
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
  // The keys the editor's two modes claim (data-day-single-line: Enter submits; data-day-code:
  // Tab indents), reported on the piece channel as `key <name>` and kept out of the element,
  // so no line break and no focus move. A modified key stays the browser's.
  el.addEventListener('keydown', (e) => {
    if (e.isComposing || e.metaKey || e.ctrlKey || e.altKey || e.shiftKey) return;
    const name = e.key === 'Enter' && el.hasAttribute('data-day-single-line') ? 'Enter'
      : e.key === 'Tab' && el.hasAttribute('data-day-code') ? 'Tab' : null;
    if (!name) return;
    e.preventDefault();
    const [p, l] = intoWasm('key ' + name);
    wasm.day_dom_event_text(id, 17, p, l);
  });
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
  // A drag owns the selection until the button comes up, including a release outside the
  // element, which is why the end listeners are on the document.
  el.addEventListener('pointerdown', () => { el.__dayDragging = true; });
  document.addEventListener('pointerup', () => { el.__dayDragging = false; });
  document.addEventListener('pointercancel', () => { el.__dayDragging = false; });
  document.addEventListener('selectionchange', () => {
    const sel = window.getSelection();
    if (!sel || sel.rangeCount === 0 || !el.contains(sel.anchorNode)) return;
    const a = dayEditorOffset(el, sel.anchorNode, sel.anchorOffset);
    const b = dayEditorOffset(el, sel.focusNode, sel.focusOffset);
    // Remembered ordered, because the piece reports a backwards drag ordered as well; an
    // unordered comparison would miss the echo and re-anchor the drag at its far end.
    el.__daySel = [Math.min(a, b), Math.max(a, b)];
    const [p, l] = intoWasm('sel ' + a + ' ' + b);
    wasm.day_dom_event_text(id, 17, p, l);
  });
}

// The elements the browser already makes focusable and activates from the keyboard; everything
// else needs the tab stop and the Enter/Space route below when an app retypes it as a button.
const NATIVE_CONTROLS = 'button,input,select,textarea,a[href],summary,[contenteditable]';
const FOCUSABLE = NATIVE_CONTROLS + ',[tabindex]';

// Keyboard activation for a retyped button (docs/accessibility.md): Enter or Space on an
// element carrying `role="button"` that is not a real control. A tap host gets the same
// `ev::TAP` the pointer path reports, at its center, because a tap is decided from pointer
// events and a synthetic `click()` would raise none; anything else gets the click, which
// reaches `ev::CLICK` when the element listens for one. Installed once, on the document.
document.addEventListener('keydown', (e) => {
  if (e.key !== 'Enter' && e.key !== ' ') return;
  if (e.metaKey || e.ctrlKey || e.altKey) return;
  const el = e.target;
  if (!(el instanceof Element) || el.getAttribute('role') !== 'button' || el.matches(NATIVE_CONTROLS)) return;
  e.preventDefault();
  if (el.__dayTap) {
    const r = el.getBoundingClientRect();
    wasm.day_dom_event(el.__id, 8, r.width / 2, r.height / 2, 0, 0);
  } else el.click();
});

// Pinch (mask 4096, events 21-23) and rotate (mask 8192, events 24-26) over one element
// (docs/canvas.md "Zoom and pan"). They are one hand movement, so a single two-finger tracker
// serves both, installed the first time either is enabled; each `listen` call only switches
// its report on. A gesture reports each quantity only when it changed: a twist that keeps
// its span sends no stream of scale-1 pinches, and a pinch that keeps its angle no rotation.
//
// Two fingers: the scale is the distance between them over the distance at Began, the rotation
// the angle the line between them has turned since Began (radians, clockwise positive, as the
// y-down page measures it), both cumulative, the location their midpoint. A trackpad pinch is
// what the browser reports as a wheel with ctrlKey set: each tick is a lone Changed whose
// scale is the tick's own factor, and the default is prevented so the page does not zoom with
// it. touch-action none keeps the browser from taking the two fingers for its own zoom (and
// cancelling the pointers) on the one element that handles them.
function twoFingerListen(id, host, mask) {
  const first = !host.__dayTwoFinger;
  const want = host.__dayTwoFinger || (host.__dayTwoFinger = { pinch: false, rotate: false });
  if (mask & 4096) want.pinch = true;
  if (mask & 8192) want.rotate = true;
  if (!first) return;
  host.style.touchAction = 'none';
  const fingers = new Map();
  let span0 = 0, angle0 = 0, pinching = false, lastScale = 1, lastAngle = 0;
  const local = (x, y) => { const r = host.getBoundingClientRect(); return [x - r.left, y - r.top]; };
  const geometry = () => {
    const [a, b] = [...fingers.values()];
    const [cx, cy] = local((a[0] + b[0]) / 2, (a[1] + b[1]) / 2);
    return [Math.hypot(a[0] - b[0], a[1] - b[1]), Math.atan2(b[1] - a[1], b[0] - a[0]), cx, cy];
  };
  // Phase 0 began, 1 changed, 2 ended: the event is 21+phase for a pinch, 24+phase for a turn.
  const report = (phase, cx, cy, scale, angle) => {
    if (want.pinch && (phase !== 1 || scale !== lastScale)) wasm.day_dom_event(id, 21 + phase, cx, cy, scale, 0);
    if (want.rotate && (phase !== 1 || angle !== lastAngle)) wasm.day_dom_event(id, 24 + phase, cx, cy, angle, 0);
    lastScale = scale; lastAngle = angle;
  };
  host.addEventListener('pointerdown', (e) => {
    if (e.pointerType !== 'touch' || fingers.size >= 2) return;
    fingers.set(e.pointerId, [e.clientX, e.clientY]);
    if (fingers.size < 2) return;
    const [d, a, cx, cy] = geometry();
    span0 = d; angle0 = a; pinching = true;
    report(0, cx, cy, 1, 0);
  });
  host.addEventListener('pointermove', (e) => {
    if (!fingers.has(e.pointerId)) return;
    fingers.set(e.pointerId, [e.clientX, e.clientY]);
    if (!pinching || fingers.size < 2) return;
    const [d, a, cx, cy] = geometry();
    // Unwrapped against the last angle, so turning past half a circle keeps counting rather
    // than jumping by a full turn where atan2 wraps.
    let turn = a - angle0 - lastAngle;
    turn -= Math.round(turn / (2 * Math.PI)) * 2 * Math.PI;
    report(1, cx, cy, span0 > 0 ? d / span0 : 1, lastAngle + turn);
  });
  const lift = (e) => {
    if (!fingers.has(e.pointerId)) return;
    if (pinching) {
      const [, , cx, cy] = geometry();
      report(2, cx, cy, lastScale, lastAngle);
      pinching = false;
    }
    fingers.clear(); // both fingers must lift before another gesture begins
  };
  host.addEventListener('pointerup', lift);
  host.addEventListener('pointercancel', lift);
  // Safari reports a trackpad pinch through its gesture events, with a cumulative `scale`, a
  // cumulative `rotation` in degrees (clockwise positive) and a began/ended bracket, and does
  // not always also send the ctrl+wheel the other browsers do. While a gesture is in progress
  // it carries the pinch and the wheel below stands down, so a browser that sends both never
  // applies a tick twice.
  let gesturing = false;
  const at = (e) => local(e.clientX, e.clientY);
  const radians = (e) => (e.rotation || 0) * Math.PI / 180;
  host.addEventListener('gesturestart', (e) => {
    e.preventDefault();
    if (pinching) return; // two real fingers are already reporting
    gesturing = true;
    report(0, ...at(e), 1, 0);
  });
  host.addEventListener('gesturechange', (e) => {
    e.preventDefault();
    if (!gesturing) return;
    report(1, ...at(e), e.scale || 1, radians(e));
  });
  host.addEventListener('gestureend', (e) => {
    e.preventDefault();
    if (!gesturing) return;
    gesturing = false;
    report(2, ...at(e), e.scale || lastScale, e.rotation === undefined ? lastAngle : radians(e));
  });
  host.addEventListener('wheel', (e) => {
    if (!e.ctrlKey || !want.pinch) return;
    e.preventDefault(); e.stopPropagation();
    if (gesturing) return;
    const r = host.getBoundingClientRect();
    // A trackpad tick is a few pixels; a mouse notch with ctrl held is a hundred, and
    // clamping keeps that to about a quarter step instead of a nearly threefold jump.
    const dy = e.deltaMode === 0 ? e.deltaY : e.deltaY * 40;
    const factor = Math.exp(Math.max(-25, Math.min(25, -dy)) * 0.01);
    wasm.day_dom_event(id, 22, e.clientX - r.left, e.clientY - r.top, factor, 0);
  }, {passive: false});
}

// ---------------------------------------------------------------------------
// Pointer recognizers beyond tap and drag (docs/gestures.md "Recognizers"), hit slop, the drag
// axis, pass-through and the input kind. Event kinds mirror `ev::` in lib.rs: 27 long press,
// 28 double tap, 29 press (c = 1 in, 0 out); touches ride `day_dom_event_text` kind 18.
// ---------------------------------------------------------------------------

// Pass-through (docs/gestures.md "Pass-through"): lib.rs realizes containers and passive leaves
// with `day-passive-host` / `day-passive-leaf` (`pointer-events: none` in day.css), and the
// first listener, menu, drag source or drop target, focus stop or selectable text turns the
// element into a pointer target for good, as on the native toolkits.
function takesPointer(el) {
  el.classList.remove('day-passive-host', 'day-passive-leaf');
}

// Point in the element's own coordinates.
function localPoint(el, e) {
  const r = el.getBoundingClientRect();
  return [e.clientX - r.left, e.clientY - r.top];
}

// Whether a local point lies outside the element's hit area: its bounds grown by its hit slop.
function outsideHit(el, x, y) {
  const [t, l, b, r] = el.__daySlop ?? [0, 0, 0, 0];
  return x < -l || y < -t || x > el.offsetWidth + r || y > el.offsetHeight + b;
}

// The `touch-action` an element's recognizers need: none while it drags in every direction or
// tracks raw contacts or two fingers, `pan-y` / `pan-x` for a horizontal / vertical drag, so
// the browser scrolls the other axis natively (and cancels the pointer to the element).
function applyTouchAction(el) {
  if (el.__dayTouches || el.__dayTwoFinger) { el.style.touchAction = 'none'; return; }
  if (!el.__dayDrag) return;
  el.style.touchAction = el.__dayAxis === 1 ? 'pan-y' : el.__dayAxis === 2 ? 'pan-x' : 'none';
}

// Long press: a timer from the primary press, cancelled by a release, a cancel, or movement
// beyond 10 px; it fires at the press point while the pointer is still down, and the release
// that follows is no tap (`__dayLongPressed`, read by the tap recognizer) and no click. A touch
// long press on an element with a context menu is the menu's.
function longPressListen(id, host) {
  let lp = null;
  const cancel = () => { if (lp) { clearTimeout(lp.timer); lp = null; } };
  host.addEventListener('pointerdown', (e) => {
    cancel();
    host.__dayLongPressed = undefined;
    if (!e.isPrimary || e.button !== 0) return;
    if (e.pointerType === 'touch' && host.__dayMenu) return;
    const [x, y] = localPoint(host, e);
    const pointer = e.pointerId;
    lp = { pointer, cx: e.clientX, cy: e.clientY, timer: setTimeout(() => {
      lp = null;
      host.__dayLongPressed = pointer;
      wasm.day_dom_event(id, 27, x, y, 0, 0);
    }, 500) };
  });
  host.addEventListener('pointermove', (e) => {
    if (lp && e.pointerId === lp.pointer && Math.hypot(e.clientX - lp.cx, e.clientY - lp.cy) > 10) cancel();
  });
  for (const name of ['pointerup', 'pointercancel', 'pointerleave']) {
    host.addEventListener(name, (e) => { if (lp && e.pointerId === lp.pointer) cancel(); });
  }
  host.addEventListener('click', (e) => {
    if (host.__dayLongPressed === undefined) return;
    host.__dayLongPressed = undefined;
    e.stopPropagation();
    e.preventDefault();
  }, true);
}

// Double tap: the browser's own `dblclick`, at the second tap. With a tap recognizer on the same
// element the single taps it holds are cancelled; a `dblclick` whose first tap already went out
// as a single (two slow taps the platform still counts as one double click) is not a double here.
function doubleTapListen(id, host) {
  host.__dayDouble = true;
  host.__dayPendingTaps ||= new Set();
  host.addEventListener('dblclick', (e) => {
    const pending = host.__dayPendingTaps;
    if (host.__dayTap && pending.size < 2) return;
    for (const t of pending) clearTimeout(t);
    pending.clear();
    const [x, y] = localPoint(host, e);
    wasm.day_dom_event(id, 28, x, y, 0, 0);
  });
}

// Pointers pressed on some element right now, so a window that loses focus ends them.
const livePresses = new Set();
// Guarded: the shim also loads in tests with no window, where nothing is ever pressed.
if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') window.addEventListener('blur', () => { for (const end of [...livePresses]) end(); });

// Press: in at the primary pointer-down itself, out exactly once, at the release, a cancel, the
// pointer leaving the hit area while down, or the window losing focus. An observer: passive,
// nothing prevented, nothing captured.
function pressListen(id, host) {
  let press = null;
  const out = () => {
    if (!press) return;
    const p = press;
    press = null;
    livePresses.delete(out);
    wasm.day_dom_event(id, 29, p.x, p.y, 0, 0);
  };
  host.__dayPressOut = out;
  host.addEventListener('pointerdown', (e) => {
    if (press || !e.isPrimary || e.button !== 0) return;
    const [x, y] = localPoint(host, e);
    press = { pointer: e.pointerId, x, y };
    livePresses.add(out);
    wasm.day_dom_event(id, 29, x, y, 1, 0);
  }, { passive: true });
  host.addEventListener('pointermove', (e) => {
    if (!press || e.pointerId !== press.pointer) return;
    const [x, y] = localPoint(host, e);
    if (outsideHit(host, x, y)) { out(); return; }
    press.x = x; press.y = y;
  }, { passive: true });
  for (const name of ['pointerup', 'pointercancel', 'pointerleave', 'lostpointercapture']) {
    host.addEventListener(name, (e) => {
      if (!press || e.pointerId !== press.pointer) return;
      // A captured pointer reports its leave only at the release, and an uncaptured one leaves
      // into the slop area, which is a descendant, without a `pointerleave`: both arrive here.
      if (name !== 'pointerleave' && name !== 'lostpointercapture') {
        const [x, y] = localPoint(host, e);
        if (!outsideHit(host, x, y)) { press.x = x; press.y = y; }
      }
      out();
    }, { passive: true });
  }
}

// Touches: every contact that went down on the element, captured so a finger sliding off keeps
// reporting, one event per pointer callback listing them all ("id,phase,x,y,pressure,kind;…",
// phase 0 began 1 moved 2 stationary 3 ended 4 cancelled, kind 0 touch 1 mouse 2 stylus). A
// mouse is one contact, id 0, while its primary button is down.
function touchesListen(id, host) {
  host.__dayTouches = true;
  applyTouchAction(host);
  const contacts = new Map();
  const kindOf = (e) => (e.pointerType === 'mouse' ? 1 : e.pointerType === 'pen' ? 2 : 0);
  const track = (e) => {
    const [x, y] = localPoint(host, e);
    // A mouse has no force, and a pen or finger that reports none says 0; a lifted contact
    // (pressure 0 at its `pointerup`) keeps the force it last had.
    const had = contacts.get(e.pointerId);
    const pressure = e.pointerType === 'mouse' ? 1 : e.pressure > 0 ? e.pressure : had ? had.pressure : 1;
    contacts.set(e.pointerId, { id: e.pointerType === 'mouse' ? 0 : e.pointerId, x, y, pressure, kind: kindOf(e) });
  };
  const send = (pointer, phase) => {
    const parts = [];
    for (const [p, c] of contacts) {
      parts.push(`${c.id},${p === pointer ? phase : 2},${c.x},${c.y},${c.pressure},${c.kind}`);
    }
    if (phase >= 3) contacts.delete(pointer);
    const [ptr, len] = intoWasm(parts.join(';'));
    wasm.day_dom_event_text(id, 18, ptr, len);
  };
  host.addEventListener('pointerdown', (e) => {
    if (e.pointerType === 'mouse' && e.button !== 0) return;
    try { host.setPointerCapture(e.pointerId); } catch { /* an inactive pointer: report anyway */ }
    track(e);
    send(e.pointerId, 0);
  });
  host.addEventListener('pointermove', (e) => {
    if (!contacts.has(e.pointerId)) return;
    track(e);
    send(e.pointerId, 1);
  });
  host.addEventListener('pointerup', (e) => {
    if (!contacts.has(e.pointerId)) return;
    track(e);
    send(e.pointerId, 3);
  });
  for (const name of ['pointercancel', 'lostpointercapture']) {
    host.addEventListener(name, (e) => { if (contacts.has(e.pointerId)) send(e.pointerId, 4); });
  }
}

// Hit slop (docs/gestures.md "Hit slop"), in physical sides (lib.rs resolves leading and
// trailing). Where the element can carry it, the grown area is its own `::before` (day.css
// `.day-hit-slop`): a pseudo-element is the element for hit testing, so every recognizer and a
// control's own press answer there, in the element's coordinates, and nothing is added to the
// tree. A <canvas>, an <img> or a form control has no `::before`, and an element that clips its
// overflow would clip the area: there the area is a transparent sibling just behind the element
// (`.day-slop`), kept on its frame and its visibility, that hands its pointer events on.
let slopSiblings = 0;
const SLOP_SIBLING_EVENTS = ['pointerdown', 'pointermove', 'pointerup', 'pointercancel', 'click', 'dblclick', 'contextmenu'];
function setHitSlop(id, t, l, b, r) {
  const host = E(id);
  const any = t > 0 || l > 0 || b > 0 || r > 0;
  host.__daySlop = any ? [t, l, b, r] : null;
  for (const [k, v] of [['top', t], ['left', l], ['bottom', b], ['right', r]]) host.style.setProperty(`--day-slop-${k}`, `${-v}px`);
  const pseudo = !(host.tagName === 'CANVAS' || host.tagName === 'IMG' || host.matches(NATIVE_CONTROLS)
    || host.classList.contains('day-cover') || getComputedStyle(host).overflow !== 'visible');
  host.classList.toggle('day-hit-slop', any && pseudo);
  if (!any || pseudo) { dropSlopSibling(host); return; }
  let s = host.__daySlopEl;
  if (!s) {
    s = div('day-slop');
    s.__daySlopFor = host;
    host.__daySlopEl = s;
    slopSiblings += 1;
    for (const name of SLOP_SIBLING_EVENTS) {
      s.addEventListener(name, (e) => {
        e.stopPropagation();
        const Kind = name.startsWith('pointer') ? PointerEvent : MouseEvent;
        if (!host.dispatchEvent(new Kind(name, e))) e.preventDefault();
      });
    }
  }
  placeSlopSibling(host);
}
function dropSlopSibling(host) {
  if (!host.__daySlopEl) return;
  host.__daySlopEl.remove();
  host.__daySlopEl = null;
  slopSiblings -= 1;
}
// Keep a slop sibling just behind its element, on the element's frame grown by the slop, and
// hidden with it.
function placeSlopSibling(host) {
  const s = host.__daySlopEl;
  if (!s || !host.__daySlop) return;
  if (!host.parentNode) { s.remove(); return; }
  if (s.nextSibling !== host) host.before(s);
  const [t, l, b, r] = host.__daySlop;
  const st = host.style;
  Object.assign(s.style, {
    left: `${parseFloat(st.left || '0') - l}px`, top: `${parseFloat(st.top || '0') - t}px`,
    width: `${host.offsetWidth + l + r}px`, height: `${host.offsetHeight + t + b}px`,
    visibility: st.visibility, display: st.display,
  });
}
// The `index`-th child Day placed in `target`, skipping slop siblings, which Day never counts:
// the node to insert before (an element's own slop sibling stands in for it).
function dayChildAt(target, index) {
  if (!slopSiblings) return target.children[index] ?? null;
  let i = 0;
  for (const c of target.children) {
    if (c.__daySlopFor) continue;
    if (i++ === index) return c.__daySlopEl && c.__daySlopEl.nextSibling === c ? c.__daySlopEl : c;
  }
  return null;
}

// An element is inert while it is not presented or sits in a disabled subtree.
function syncInert(el) {
  el.inert = !!el.__dayHidden || (el.__dayDisabled || 0) > 0;
}

// ---------------------------------------------------------------------------
// Animation (docs/tween.md "The animation contract"): the Web Animations API, one run per
// (element, property). The target goes into the inline style at once, so it is the resting
// value when the run ends (rule 5) and the model value throughout; the run plays from the value
// displayed when the call arrives (rule 7), holding it through the delay (`fill: backwards`),
// for every cycle (`iterations`), backwards on alternate ones (`direction: alternate`). The
// curve is Day's own samples as keyframes with offsets, so springs overshoot exactly and the
// eases match every other toolkit; no `linear()` easing is needed, whatever the engine.
// Properties: 0 opacity [o], 1 transform [tx, ty, deg, sx, sy], 2 background [r, g, b, a] in
// 0..1, 3 frame [x, y, w, h].
// ---------------------------------------------------------------------------

const ANIM_PROPS = ['opacity', 'transform', 'background', 'frame'];

function transformCss(v) {
  return `translate(${v[0]}px,${v[1]}px) rotate(${v[2]}deg) scale(${v[3]},${v[4]})`;
}
function colorCss(v) {
  const c = (x) => Math.round(Math.min(1, Math.max(0, x)) * 255);
  return `rgba(${c(v[0])},${c(v[1])},${c(v[2])},${Math.min(1, Math.max(0, v[3]))})`;
}
// A computed color as [r, g, b, a] in 0..1: the legacy `rgb()`/`rgba()` form, or the
// `color(srgb …)` form engines give a color made with relative color syntax or `color-mix`.
function parseColor(text) {
  const t = text || '';
  const srgb = /color\(srgb\s+([^)]+)\)/.exec(t);
  if (srgb) {
    const p = srgb[1].split(/[\s/]+/).filter(Boolean).map(Number);
    return [p[0], p[1], p[2], p.length > 3 ? p[3] : 1];
  }
  const m = /rgba?\(([^)]+)\)/.exec(t);
  if (!m) return null;
  const p = m[1].split(/[,\s/]+/).filter(Boolean).map(Number);
  return [p[0] / 255, p[1] / 255, p[2] / 255, p.length > 3 ? p[3] : 1];
}
// Whether an element paints a background color of its own: an inline one (a fixed color or a
// dynamic ink's `light-dark()`/`var()`) that resolves to something visible.
function ownFill(el) {
  return !!el.style.backgroundColor && (parseColor(getComputedStyle(el).backgroundColor)?.[3] ?? 0) > 0;
}
// The custom property day.css defines for each `SystemColor`, in wire order.
const SYSTEM_COLOR_VARS = [
  '--day-accent', '--day-fg', '--day-fg-2', '--day-bg', '--day-surface', '--day-fg-3',
  '--day-placeholder', '--day-link', '--day-separator', '--day-control', '--day-grouped',
  '--day-elevated', '--day-selection', '--day-destructive', '--day-success', '--day-warning',
];
// Any CSS color as "r,g,b,a" in 0..1, as it resolves on the page now: the computed value
// where the engine reports it numerically, else through a canvas, which parses any syntax.
function cssColorParts(css) {
  const probe = document.createElement('span');
  probe.style.color = css;
  probe.hidden = true;
  document.documentElement.append(probe);
  const color = getComputedStyle(probe).color;
  probe.remove();
  const parsed = parseColor(color);
  if (parsed) return parsed.join(',');
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 1;
  const context = canvas.getContext('2d', { willReadFrequently: true });
  if (!context) return '';
  context.fillStyle = color;
  context.fillRect(0, 0, 1, 1);
  return Array.from(context.getImageData(0, 0, 1, 1).data, n => n / 255).join(',');
}
// A computed transform's translation, rotation and scale (Day's transforms carry no skew).
function parseTransform(text) {
  if (!text || text === 'none') return [0, 0, 0, 1, 1];
  const m = /matrix(3d)?\(([^)]+)\)/.exec(text);
  if (!m) return [0, 0, 0, 1, 1];
  const v = m[2].split(',').map(Number);
  const [a, b, c, d, e, f] = m[1] ? [v[0], v[1], v[4], v[5], v[12], v[13]] : v;
  const sx = Math.hypot(a, b);
  const sy = sx ? (a * d - b * c) / sx : Math.hypot(c, d);
  return [e, f, Math.atan2(b, a) * 180 / Math.PI, sx, sy];
}
// What the element displays now, mid-run included, as the property's numbers.
function shownValue(el, prop) {
  const cs = getComputedStyle(el);
  switch (prop) {
    case 0: return [parseFloat(cs.opacity)];
    case 1: return parseTransform(cs.transform);
    case 2: return parseColor(cs.backgroundColor);
    default: return [parseFloat(cs.left) || 0, parseFloat(cs.top) || 0, parseFloat(cs.width) || 0, parseFloat(cs.height) || 0];
  }
}
// The property's numbers as style declarations.
function styleOf(prop, v) {
  switch (prop) {
    case 0: return { opacity: String(v[0]) };
    case 1: return { transform: transformCss(v) };
    case 2: return { backgroundColor: colorCss(v) };
    default: return { left: `${v[0]}px`, top: `${v[1]}px`, width: `${v[2]}px`, height: `${v[3]}px` };
  }
}
function sameValue(a, b) {
  return !!a && !!b && a.length === b.length && a.every((x, i) => Math.abs(x - b[i]) < 1e-6);
}
// Set a property, animated when `spec` is given ({ duration, delay, iterations, alternate,
// samples: [offset, progress, …] }); a plain set cancels a run on that property (rule 7). A
// plain set that names the value a run is already heading for leaves the run alone: layout
// re-sends an unchanged frame.
function animateProp(el, prop, to, spec) {
  const runs = el.__dayRuns || (el.__dayRuns = {});
  const run = runs[prop];
  if (!spec && run && sameValue(run.to, to)) return;
  const from = spec ? shownValue(el, prop) : null;
  if (run) { run.anim.cancel(); delete runs[prop]; }
  Object.assign(el.style, styleOf(prop, to));
  if (!spec || !from || spec.duration <= 0) return;
  const frames = [];
  for (let i = 0; i < spec.samples.length; i += 2) {
    const k = spec.samples[i + 1];
    frames.push({ offset: spec.samples[i], ...styleOf(prop, from.map((f, j) => f + (to[j] - f) * k)) });
  }
  const anim = el.animate(frames, {
    duration: spec.duration, delay: spec.delay, easing: 'linear', fill: 'backwards',
    iterations: spec.iterations < 0 ? Infinity : spec.iterations,
    direction: spec.alternate ? 'alternate' : 'normal',
  });
  runs[prop] = { anim, to };
  anim.onfinish = () => { if (runs[prop]?.anim === anim) delete runs[prop]; };
  anim.oncancel = anim.onfinish;
}

// ---------------------------------------------------------------------------
// Scroll views (docs/scroll.md): indicators, bounce, snapping and the user's scroll phases.
// ---------------------------------------------------------------------------

const HAS_SCROLLEND = typeof window !== 'undefined' && 'onscrollend' in window;

// Snap stops (docs/scroll.md "Snapping"): the content is placed by Day, so the snap areas are
// markers of the scroller's own, one per stop along the scroll axis plus one at the very end,
// so the last, shorter stretch stays reachable. `Page` stops on every viewport extent and moves
// one page per swipe (`scroll-snap-stop: always`).
function layoutSnap(el) {
  const snap = el.__daySnap;
  if (!snap) return;
  const horizontal = el.classList.contains('horizontal');
  const view = horizontal ? el.clientWidth : el.clientHeight;
  const content = el.__content;
  const extent = content ? (horizontal ? content.offsetWidth : content.offsetHeight) : 0;
  const step = snap.page ? view : snap.interval;
  const markers = snap.markers;
  if (!(step > 0) || !(view > 0)) { for (const m of markers.splice(0)) m.remove(); return; }
  const end = Math.max(0, extent - view);
  const stops = [];
  for (let at = 0; at < end - 0.5 && stops.length < 4096; at += step) stops.push(at);
  stops.push(end);
  while (markers.length > stops.length) markers.pop().remove();
  while (markers.length < stops.length) {
    const m = div('day-snap');
    if (snap.page) m.style.scrollSnapStop = 'always';
    el.append(m);
    markers.push(m);
  }
  stops.forEach((at, i) => {
    const m = markers[i];
    m.style.left = horizontal ? `${at}px` : '0px';
    m.style.top = horizontal ? '0px' : `${at}px`;
  });
}

// The user's scroll phases (docs/scroll.md "Phases"; event 30: 0 idle, 1 dragging, 2 coasting):
// a scroll that a finger, a wheel or trackpad, the keyboard or the scroll bar drives is
// dragging; after the finger lifts the content coasts until it stops; a programmatic scroll
// has no phase.
function trackScrollPhase(id, el) {
  let phase = 0, touching = false, held = false, input = -1e9, quiet = 0;
  const set = (p) => { if (p !== phase) { phase = p; wasm.day_dom_event(id, 30, p, 0, 0, 0); } };
  const rest = () => { clearTimeout(quiet); if (!touching && !held) set(0); };
  const user = () => { input = performance.now(); };
  el.addEventListener('touchstart', () => { touching = true; user(); }, { passive: true });
  el.addEventListener('touchmove', user, { passive: true });
  const lift = () => {
    if (!touching) return;
    touching = false;
    if (phase === 1) set(2);
    // No momentum: nothing more scrolls, and the content is at rest already.
    clearTimeout(quiet);
    quiet = setTimeout(rest, 120);
  };
  el.addEventListener('touchend', lift, { passive: true });
  el.addEventListener('touchcancel', lift, { passive: true });
  el.addEventListener('wheel', user, { passive: true });
  el.addEventListener('keydown', user, { passive: true });
  // A press on the scroll bar lands on the scroller itself; the content covers the rest.
  el.addEventListener('pointerdown', (e) => { if (e.target === el) { held = true; user(); } }, { passive: true });
  addEventListener('pointerup', () => { if (held) { held = false; clearTimeout(quiet); quiet = setTimeout(rest, 120); } }, { passive: true });
  el.addEventListener('scroll', () => {
    // Moving under the input is dragging; moving on after it, with nothing held, is the
    // content coasting (momentum, a smooth wheel step, the snap to a stop).
    const driven = touching || held || performance.now() - input < 100;
    if (phase === 0 && driven) set(1);
    else if (phase === 1 && !driven) set(2);
    if (phase !== 0) {
      clearTimeout(quiet);
      // Where `scrollend` exists it says when the content stopped; a quiet timer elsewhere.
      quiet = setTimeout(rest, HAS_SCROLLEND ? 400 : 150);
    }
  }, { passive: true });
  // A snap can start right after the scroll that ended: a short grace keeps it one gesture.
  if (HAS_SCROLLEND) el.addEventListener('scrollend', () => { clearTimeout(quiet); quiet = setTimeout(rest, 80); });
}

// The reader's own scrolls of a list (docs/list.md "Following the end", Cap::ListReaderScroll):
// a scroll that moves under a finger, while the scroll bar is held, or within a moment of a
// wheel or a key that scrolls, and any of those landing on the list's own smooth scroll. Day's
// own `scrollTo` fires `scroll` with none of those before it, and an input that moves nothing
// (a wheel at the end) reports nothing. Each input reports once.
const LIST_SCROLL_KEYS = new Set(['ArrowUp', 'ArrowDown', 'PageUp', 'PageDown', 'Home', 'End', ' ']);
// The scrolling keys (docs/list.md "Following the end"): pressed in a list, or on the page with
// nothing focused, where the browser scrolls the scroller the reader last pressed. A field keeps
// its keys, and a control's Space presses it. Each list hears its own through `__dayReaderKey`.
let listPressed = null;
if (typeof document !== 'undefined') {
  document.addEventListener('pointerdown', (e) => { listPressed = e.target; }, { capture: true, passive: true });
  document.addEventListener('keydown', (e) => {
    if (!LIST_SCROLL_KEYS.has(e.key) || e.ctrlKey || e.metaKey || e.altKey) return;
    const t = e.target;
    const page = t === document.body || t === document.documentElement;
    if (!page && t.closest?.(e.key === ' ' ? 'input,textarea,select,[contenteditable],button,a' : 'input,textarea,select,[contenteditable]')) return;
    const list = (page ? listPressed : t)?.closest?.('.day-list');
    list?.__dayReaderKey?.(performance.now());
  }, { capture: true, passive: true });
}
// The smooth scroll Day asked of a scroller, while it runs: where it is going, or null. A browser
// carries such a scroll on to its target even as the reader wheels or drags against it.
function glide(el, top) {
  el.__dayGlide = top === null ? null : Math.max(0, Math.min(top, el.scrollHeight - el.clientHeight));
}
function trackListReader(id, el) {
  let touchAt = 0, heldAt = 0, wheelAt = -1e9, keyAt = -1e9, told = 0;
  const report = (stamp) => {
    if (stamp === told) return;
    told = stamp;
    wasm.day_dom_list_reader(id);
  };
  // Input that lands on the list's own smooth scroll stops it where it is: that is the reader
  // taking the list, reported at once. The browsers drop the input's own scroll then (it was
  // fighting the running one), so the wheel's step is applied here.
  const halt = (stamp) => {
    if (el.__dayGlide == null) return false;
    el.__dayGlide = null;
    el.scrollTo({ top: el.scrollTop, behavior: 'instant' });
    report(stamp);
    return true;
  };
  el.addEventListener('touchstart', () => { touchAt = performance.now(); halt(touchAt); }, { passive: true });
  const lift = () => { touchAt = 0; };
  el.addEventListener('touchend', lift, { passive: true });
  el.addEventListener('touchcancel', lift, { passive: true });
  el.addEventListener('wheel', (e) => {
    if (!e.deltaY || e.ctrlKey) return;
    wheelAt = performance.now();
    const unit = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? el.clientHeight : 1;
    if (halt(wheelAt)) el.scrollBy({ top: e.deltaY * unit, behavior: 'instant' });
  }, { passive: true });
  // A press on the scroll bar lands on the scroller itself; the rows cover the rest.
  el.addEventListener('pointerdown', (e) => {
    if (e.target !== el || e.pointerType === 'touch') return;
    heldAt = performance.now();
    halt(heldAt);
    const release = () => {
      heldAt = 0;
      removeEventListener('pointerup', release, true);
      removeEventListener('pointercancel', release, true);
    };
    addEventListener('pointerup', release, { capture: true, passive: true });
    addEventListener('pointercancel', release, { capture: true, passive: true });
  }, { passive: true });
  el.__dayReaderKey = (stamp) => { keyAt = stamp; halt(stamp); };
  el.addEventListener('scroll', () => {
    if (el.__dayGlide != null && Math.abs(el.scrollTop - el.__dayGlide) < 1) el.__dayGlide = null;
    const now = performance.now();
    const stamp = touchAt || heldAt || (now - wheelAt < 100 ? wheelAt : 0) || (now - keyAt < 100 ? keyAt : 0);
    if (stamp) report(stamp);
  }, { passive: true });
  if (HAS_SCROLLEND) el.addEventListener('scrollend', () => { el.__dayGlide = null; }, { passive: true });
}

// The input kind (docs/gestures.md "Input kind"): the device behind the latest pointer down or
// hover, reported on a change only. Starts from the primary pointer the media query names.
let pointerKind = typeof matchMedia === 'function' && matchMedia('(pointer: coarse)').matches ? 0 : 1;
function notePointerKind(e) {
  const kind = e.pointerType === 'touch' ? 0 : e.pointerType === 'pen' ? 2 : 1;
  if (kind === pointerKind || !wasm) return;
  pointerKind = kind;
  wasm.day_dom_pointer_kind_changed(kind);
}
document.addEventListener('pointerdown', notePointerKind, { capture: true, passive: true });
document.addEventListener('pointermove', notePointerKind, { capture: true, passive: true });

function listen(id, mask) {
  const host = E(id); const el = V(id);
  if (mask & 1) {
    const rowControl = (e) => el.getAttribute('role') === 'option' && e.target.closest('button,input,select,textarea,a,[contenteditable]');
    // WebKit can report a touch-generated click as mouse. Preserve the pointerdown
    // source for one click, then clear it so keyboard clicks cannot inherit a stale touch.
    let pointerType;
    let clickPointerType;
    el.addEventListener('pointerdown', (e) => { pointerType = e.pointerType; });
    el.addEventListener('pointercancel', () => { pointerType = undefined; });
    el.addEventListener('click', (e) => {
      clickPointerType = pointerType ?? e.pointerType;
      pointerType = undefined;
      if (!rowControl(e)) wasm.day_dom_event(id, 1, mods(e), 1, clickPointerType === 'touch' ? 1 : 0, 0);
    });
    el.addEventListener('dblclick', (e) => {
      if (el.getAttribute('role') === 'option' && clickPointerType !== 'touch' && !rowControl(e)) wasm.day_dom_event(id, 1, mods(e), 2, 0, 0);
    });
  }
  if (mask & 2) el.addEventListener('input', () => {
    if (el.type === 'range') wasm.day_dom_event(id, 5, Number(el.value), 0, 0, 0);
    else { const [p, l] = intoWasm(el.value); wasm.day_dom_event_text(id, 2, p, l); }
  });
  if (mask & 4) el.addEventListener('change', () => {
    if (el.type === 'checkbox') wasm.day_dom_event(id, 4, el.checked ? 1 : 0, 0, 0, 0);
    else if (el.tagName === 'SELECT') wasm.day_dom_event(id, 6, el.selectedIndex, 0, 0, 0);
    // A range's `change` is the settled value: the DOM fires `input` as the thumb moves and
    // `change` once, when the user lets go (event 15, mirroring ev::VALUE_COMMITTED in lib.rs).
    else if (el.type === 'range') wasm.day_dom_event(id, 15, Number(el.value), 0, 0, 0);
  });
  if (mask & 8) {
    el.addEventListener('focus', () => wasm.day_dom_event(id, 7, 1, 0, 0, 0));
    el.addEventListener('blur', () => wasm.day_dom_event(id, 7, 0, 0, 0, 0));
  }
  if (mask & 16) {
    // A field's claimed keys report their repeats (bit 8) and their release (bit 9) too.
    const claimed = new Set();
    const fieldMods = (e) => (e.shiftKey ? 1 : 0) | (e.metaKey || e.ctrlKey ? 2 : 0) | (e.altKey ? 4 : 0);
    el.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') wasm.day_dom_event(id, 3, 0, 0, 0, 0);
      // The keys a field with a key handler claims (docs/textfield.md "Keys"), never in the
      // middle of a composition, whose candidate list owns them.
      const field = { ArrowUp: 0, ArrowDown: 1, Escape: 2 }[e.key];
      if (field === undefined || e.isComposing || e.keyCode === 229) return;
      if (wasm.day_dom_field_key(id, field, fieldMods(e) | (e.repeat ? 256 : 0))) {
        e.preventDefault();
        claimed.add(field);
      }
    });
    el.addEventListener('keyup', (e) => {
      const field = { ArrowUp: 0, ArrowDown: 1, Escape: 2 }[e.key];
      if (field === undefined || !claimed.delete(field)) return;
      if (wasm.day_dom_field_key(id, field, fieldMods(e) | 512)) e.preventDefault();
    });
    el.addEventListener('blur', () => claimed.clear());
  }
  if (mask & 512) dayEditorListen(id, el);
  // Any listener at all makes the element a pointer target (docs/gestures.md "Pass-through").
  if (mask & ~(8 | 32 | 64)) takesPointer(host);
  if (mask & 32) resizeObserver.observe(host);
  if (mask & 64) {
    // The offset (docs/scroll.md "Offset"): at most once per frame while it moves, and always
    // the value it comes to rest at, from `scrollend` where the engine has it and a quiet
    // timer where it does not.
    let raf = 0, quiet = 0;
    const report = () => {
      if (raf) cancelAnimationFrame(raf);
      raf = 0;
      wasm.day_dom_event(id, 12, host.scrollLeft, host.scrollTop, 0, 0);
    };
    host.addEventListener('scroll', () => {
      if (!raf) raf = requestAnimationFrame(report);
      if (!HAS_SCROLLEND) { clearTimeout(quiet); quiet = setTimeout(report, 100); }
    }, { passive: true });
    if (HAS_SCROLLEND) host.addEventListener('scrollend', report);
  }
  if (mask & 128) {
    // A tap is a press released without meaningful movement, decided at pointerup the way
    // every native recognizer does it, and reported at the press point. Firing on pointerdown
    // broke coexisting drag gestures: the tap handler ran before the drag's Began and could
    // re-target selection out from under it (Day-Sketch's resize handles were unreachable
    // outside a shape's geometry, on web only).
    let tap = null;
    host.__dayTap = true; // the keyboard route above sends this host a center tap
    host.__dayPendingTaps ||= new Set();
    host.addEventListener('pointerdown', (e) => {
      // Primary button only: a right-click is a summon (the contextmenu listener above), and
      // the pointerup it also produces must not come out as a tap; on Day-Sketch the phantom
      // tap re-targeted the selection under the menu the summon had just built. A second
      // finger is a pinch, not a tap, however little it moves.
      if (e.button !== 0 || tap) { tap = null; return; }
      const r = host.getBoundingClientRect();
      tap = [e.clientX - r.left, e.clientY - r.top, e.clientX, e.clientY, e.pointerId];
    });
    host.addEventListener('pointermove', (e) => {
      if (tap && e.pointerId === tap[4] && Math.hypot(e.clientX - tap[2], e.clientY - tap[3]) > 4) tap = null;
    });
    host.addEventListener('pointerup', (e) => {
      if (!tap || e.pointerId !== tap[4]) return;
      const [x, y] = tap;
      tap = null;
      // A press that already came out as a long press is not also a tap.
      if (host.__dayLongPressed === e.pointerId) return;
      // With a double tap on the same element, a single tap waits out the interval and a
      // double tap cancels it (the `dblclick` listener below); the web exposes no system
      // interval, so it is the common 300 ms.
      if (host.__dayDouble) {
        const pending = host.__dayPendingTaps;
        const t = setTimeout(() => { pending.delete(t); wasm.day_dom_event(id, 8, x, y, 0, 0); }, 300);
        pending.add(t);
        return;
      }
      wasm.day_dom_event(id, 8, x, y, 0, 0);
    });
    host.addEventListener('pointercancel', () => { tap = null; });
    // Leaving the element ends the tap: an uncaptured pointer reports no moves outside, so a
    // press dragged off and back would otherwise still come out as one.
    host.addEventListener('pointerleave', (e) => { if (tap && e.pointerId === tap[4]) tap = null; });
  }
  if (mask & 2048) {
    el.dataset.dayPan = 'true';
    el.addEventListener('wheel', e => {
      if (e.target.closest('[data-day-pan]') !== el || e.ctrlKey || e.metaKey) return;
      if (el.tagName !== 'CANVAS' && Math.abs(e.deltaX) <= Math.abs(e.deltaY)) return;
      const r = el.getBoundingClientRect();
      const scale = e.deltaMode === 1 ? 40 : e.deltaMode === 2 ? r.width : 1;
      e.preventDefault(); e.stopPropagation();
      wasm.day_dom_event(id, 20, e.clientX-r.left, e.clientY-r.top, -e.deltaX*scale, -e.deltaY*scale);
    }, {passive:false});
  }
  if (mask & 1024) {
    // A touch long press on this element is its context menu's (the long-press recognizer
    // below stands down for touch).
    host.__dayMenu = true;
    // The browser's own context menu yields to the app's: Day presents a composed menu from
    // the same model every other backend shows natively (event 16, ev::CONTEXT in lib.rs).
    el.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      e.stopPropagation();
      const r = host.getBoundingClientRect();
      wasm.day_dom_event(id, 16, e.clientX - r.left, e.clientY - r.top, e.clientX, e.clientY);
    });
  }
  if (mask & 256) {
    // The drag engages only past the same small slop, with Began at the original press point,
    // so a clean click never produces a zero-length drag, and a real drag never produces a
    // tap.
    // One pointer drags, the one that pressed first: a second finger ends the drag where it
    // was and no drag starts again until every finger has lifted, so the two fingers of a
    // pinch never come out as drags (docs/canvas.md "Zoom and pan").
    // The sequence is the element's from Began to Ended (docs/gestures.md "Gestures inside a
    // scroll view"): `touch-action: none` keeps an enclosing scroller from taking a touch drag,
    // unless the drag follows one axis (`day_dom_drag_axis`), which leaves the other to the
    // browser's own scrolling.
    host.__dayDrag = true;
    applyTouchAction(host);
    let start = null, dragging = false, pointer = null, last = null;
    const down = new Set();
    const finish = (e) => {
      if (!start) return;
      if (dragging) wasm.day_dom_event(id, 11, e.clientX - start[2], e.clientY - start[3], e.clientX - start[0], e.clientY - start[1]);
      start = null;
      dragging = false;
    };
    host.addEventListener('pointerdown', (e) => {
      if (e.button !== 0) return; // primary-button drags only (see the tap guard above)
      down.add(e.pointerId);
      if (down.size > 1) { if (last) finish(last); return; }
      host.setPointerCapture(e.pointerId);
      const r = host.getBoundingClientRect();
      start = [e.clientX, e.clientY, r.left, r.top];
      pointer = e.pointerId;
      last = e;
      dragging = false;
    });
    host.addEventListener('pointermove', (e) => {
      if (!start || e.pointerId !== pointer) return;
      last = e;
      if (!dragging) {
        const dx = e.clientX - start[0], dy = e.clientY - start[1];
        if (Math.hypot(dx, dy) < 4) return;
        // An axis drag decides at the slop: across its axis, the sequence is not this drag's at
        // all (a touch has already gone to the scroller through touch-action; a mouse sends
        // nothing until it is released).
        const axis = host.__dayAxis;
        if ((axis === 1 && Math.abs(dx) < Math.abs(dy)) || (axis === 2 && Math.abs(dy) < Math.abs(dx))) {
          if (host.hasPointerCapture?.(e.pointerId)) host.releasePointerCapture(e.pointerId);
          start = null;
          return;
        }
        dragging = true;
        wasm.day_dom_event(id, 9, start[0] - start[2], start[1] - start[3], 0, 0);
      }
      wasm.day_dom_event(id, 10, e.clientX - start[2], e.clientY - start[3], e.clientX - start[0], e.clientY - start[1]);
    });
    const lift = (e) => {
      down.delete(e.pointerId);
      if (e.pointerId === pointer) { finish(e); pointer = null; last = null; }
    };
    host.addEventListener('pointerup', lift);
    host.addEventListener('pointercancel', lift);
  }
  if (mask & (4096 | 8192)) twoFingerListen(id, host, mask);
  if (mask & 16384) longPressListen(id, host);
  if (mask & 32768) doubleTapListen(id, host);
  if (mask & 65536) pressListen(id, host);
  if (mask & 131072) touchesListen(id, host);
  // Its own bit, not the editor's 512: an element that hovers is not a text editor, and the
  // editor's `input` listener on a hover host would report every field typed into inside it.
  if (mask & 262144) {
    // Hover: pointer entry, motion and exit. `pointerType` gates it to a real pointer: a touch
    // produces a pointerdown/up pair with an enter and a leave around it, and reporting those as
    // hovers would make every tap look like a hover on a phone (docs/canvas.md "Interaction").
    let last = [0, 0];
    const local = (e) => { const r = host.getBoundingClientRect(); return [e.clientX - r.left, e.clientY - r.top]; };
    host.addEventListener('pointerenter', (e) => {
      if (e.pointerType === 'touch') return;
      last = local(e);
      wasm.day_dom_event(id, 17, last[0], last[1], 0, 0);
    });
    host.addEventListener('pointermove', (e) => {
      if (e.pointerType === 'touch') return;
      last = local(e);
      wasm.day_dom_event(id, 18, last[0], last[1], 0, 0);
    });
    host.addEventListener('pointerleave', (e) => {
      if (e.pointerType === 'touch') return;
      // The leave's own coordinates are outside the element; the contract is the last point seen
      // Inside, so that is what goes out.
      wasm.day_dom_event(id, 19, last[0], last[1], 0, 0);
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
// place the browser exposes font metrics). Cached per font string, since a form asks for the
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

// An element's font as a `font` shorthand, for the measurer and the metrics cache. The computed
// `font` serializes to "" whenever a longhand it cannot express is set, and every tabular label
// sets one (`font-variant-numeric: tabular-nums`). An empty font leaves the measurer in whatever
// font it held last, so a large bold score measured as a small caption and drew clipped. Rebuilt
// from the longhands the shorthand covers instead; the numeric variant travels on its own.
function elementFont(cs) {
  return cs.font || `${cs.fontStyle} ${cs.fontWeight} ${cs.fontSize}/${cs.lineHeight} ${cs.fontFamily}`;
}

function prepareMeasurer(font, maxW, numeric) {
  if (!measurer) {
    measurer = div('');
    measurer.style.cssText = 'position:absolute;left:-99999px;top:0;visibility:hidden;white-space:pre-wrap;overflow-wrap:break-word;';
    document.body.append(measurer);
  }
  measurer.style.font = font;
  // After the shorthand, which resets it: tabular digits are wider than proportional ones.
  measurer.style.fontVariantNumeric = numeric || 'normal';
  measurer.style.maxWidth = (maxW < 1e5 ? maxW : 100000) + 'px';
}

// The measuring twin of a label: every property that shapes its lines, copied from the element.
const TWIN_PROPS = ['font-family', 'font-size', 'font-weight', 'font-style', 'font-stretch',
  'font-variant-caps', 'font-variant-numeric', 'font-feature-settings', 'font-kerning',
  'letter-spacing', 'word-spacing', 'line-height', 'text-transform', 'white-space',
  '-webkit-box-orient', '-webkit-line-clamp', 'overflow-wrap', 'word-break', 'direction',
  'padding-top', 'padding-bottom', 'padding-left', 'padding-right'];
let twin = null;
function labelTwin(el, maxW) {
  if (!twin) {
    twin = div('');
    twin.style.cssText = 'position:absolute;left:-99999px;top:0;visibility:hidden;overflow:hidden;box-sizing:content-box;';
    document.body.append(twin);
  }
  const cs = getComputedStyle(el);
  for (const p of TWIN_PROPS) twin.style.setProperty(p, cs.getPropertyValue(p));
  // Not the computed `display`: Chromium computes a clamped `-webkit-box` as `flow-root`, and
  // copying that would drop the clamp.
  const clamp = cs.getPropertyValue('-webkit-line-clamp');
  twin.style.display = clamp && clamp !== 'none' ? '-webkit-box' : 'block';
  twin.style.maxWidth = (maxW < 1e5 ? maxW : 100000) + 'px';
  return twin;
}

// The browser's text size factor (docs/text.md "Text size"), and the change watch: no event
// exists, so a resize and a return to the page re-read it.
function textScale() {
  const base = matchMedia('(pointer: coarse)').matches && CSS.supports('font', '-apple-system-body') ? 17 : 16;
  const px = parseFloat(getComputedStyle(document.documentElement).fontSize);
  return px > 0 ? px / base : 1;
}
let lastTextScale = null;
function watchTextScale() {
  const now = textScale();
  if (lastTextScale !== null && Math.abs(now - lastTextScale) > 1e-6 && wasm) wasm.day_dom_text_scale_changed();
  lastTextScale = now;
}
if (typeof addEventListener === 'function') addEventListener('resize', watchTextScale);
document.addEventListener('visibilitychange', watchTextScale);

// Selection reports (docs/text.md "Selection"): a range inside a label flagged
// `data-day-reports-selection`, as UTF-16 offsets from the label's start; leaving a label (or
// collapsing) reports it cleared, once.
let selectedLabel = null;
let selectedRange = '';
document.addEventListener('selectionchange', () => {
  if (!wasm) return;
  const sel = document.getSelection();
  let label = null, start = 0, end = 0;
  if (sel && sel.rangeCount) {
    const r = sel.getRangeAt(0);
    const at = (n) => (n.nodeType === 1 ? n : n.parentElement)?.closest('[data-day-reports-selection]');
    const a = at(r.startContainer), b = at(r.endContainer);
    if (a && a === b && !r.collapsed) {
      label = a;
      const pre = document.createRange();
      pre.setStart(a, 0);
      pre.setEnd(r.startContainer, r.startOffset);
      start = pre.toString().length;
      end = start + r.toString().length;
    }
  }
  if (selectedLabel && selectedLabel !== label && selectedLabel.__id) {
    wasm.day_dom_label_selected(selectedLabel.__id, 0, 0);
  }
  const key = label ? `${start},${end}` : '';
  if (label && (label !== selectedLabel || key !== selectedRange)) wasm.day_dom_label_selected(label.__id, start, end);
  selectedLabel = label;
  selectedRange = key;
});

function measure(text, font, maxW, numeric) {
  prepareMeasurer(font, maxW, numeric);
  measurer.textContent = text || ' ';
  const r = measurer.getBoundingClientRect();
  return [r.width, r.height];
}

// Measure an element's children (a label's styled spans) wrapped at `maxW`, in the same
// measurer: clones keep each span's own weight, slant and family, so the answer is the
// paragraph the browser will lay out.
function measureNodes(el, font, maxW, numeric) {
  prepareMeasurer(font, maxW, numeric);
  measurer.replaceChildren(...Array.from(el.childNodes, (n) => n.cloneNode(true)));
  const r = measurer.getBoundingClientRect();
  measurer.textContent = '';
  return [r.width, r.height];
}

// ---------------------------------------------------------------------------
// Canvas replay (§11): interpret the f64 op stream from encode_ops (lib.rs).
// ---------------------------------------------------------------------------

function rgba(packed) {
  const v = packed >>> 0;
  return `rgba(${(v >>> 24) & 255},${(v >>> 16) & 255},${(v >>> 8) & 255},${(v & 255) / 255})`;
}

// The region a stroke of the current lineWidth covers, as a clip path. Canvas2D exposes no
// "convert stroke to path", so this is the closest approximation available to it: clip to the
// path's own outline. A gradient stroke therefore paints the gradient across the whole path
// interior on web, which reads correctly for thin lines and diverges for very thick ones.
function strokeRegion(ctx, p) { return p; }

// ---- canvas fonts (docs/fonts.md) ----------------------------------------------------------

// The CSS font shorthand canvas text draws and measures in: slant, CSS weight (0 = the
// default), an absolute pixel size, then the requested family ahead of the platform stack, so
// a family the page cannot resolve falls through to the same face the default draws.
function canvasFont(size, weight, italic, family) {
  const fam = family ? `${JSON.stringify(family)}, ` : '';
  return `${italic ? 'italic ' : ''}${weight > 0 ? weight : 400} ${size}px ${fam}-apple-system, BlinkMacSystemFont, sans-serif`;
}

// Ascent and descent of the font's line box at `ctx.font`, from the measured
// fontBoundingBox (Baseline 2023); an engine without it gets the portable guess.
function fontBox(ctx, text, size) {
  const m = ctx.measureText(text);
  const asc = Number.isFinite(m.fontBoundingBoxAscent) ? m.fontBoundingBoxAscent : size * 0.9;
  const desc = Number.isFinite(m.fontBoundingBoxDescent) ? m.fontBoundingBoxDescent : size * 0.3;
  return [asc, desc];
}

let measureCtx = null;
// One offscreen 2D context for day_dom_canvas_measure_text, created on first use.
function measureContext() {
  if (!measureCtx) measureCtx = document.createElement('canvas').getContext('2d');
  return measureCtx;
}

// The page's font list in Day's list text: U+001E between families, U+001F between fields
// (family, then face/weight/italic per face). The CSS generic families come first, each with
// the four faces a browser synthesizes for any family; then every bundled FontFace `boot`
// registered, grouped by family. queryLocalFonts() is not asked: Chromium only,
// asynchronous, and behind a permission prompt (Cap::FontList answers Emulated).
function fontListText() {
  const FS = '\u001f', RS = '\u001e';
  const synth = `${FS}Regular${FS}400${FS}0${FS}Bold${FS}700${FS}0${FS}Italic${FS}400${FS}1${FS}Bold Italic${FS}700${FS}1`;
  const families = [];
  for (const g of ['system-ui', 'sans-serif', 'serif', 'monospace', 'cursive', 'fantasy']) {
    families.push(g + synth);
  }
  const bundled = new Map();
  try {
    document.fonts.forEach((face) => {
      const family = String(face.family || '').replace(/^["']|["']$/g, '');
      if (!family) return;
      const weight = parseInt(face.weight, 10) || 400;
      const italic = face.style && face.style !== 'normal' ? 1 : 0;
      const faces = bundled.get(family) || [];
      faces.push(`${FS}${FS}${weight}${FS}${italic}`); // an empty name: Day synthesizes it
      bundled.set(family, faces);
    });
  } catch (_) { /* no FontFaceSet; the generic list stands */ }
  for (const [family, faces] of bundled) {
    families.push(family.replace(/[\u001e\u001f]/g, ' ') + faces.join(''));
  }
  return families.join(RS);
}

// Decoded path geometry, keyed by the encoder's content key (day_spec::geometry_key). It lives
// out here because a cache inside `replay` would be built and thrown away every frame, which
// is the cost it exists to remove. Entries can never go stale (the key is the content), so the
// cap is only to keep a long-lived page from holding every path it has ever drawn.
const pathCache = new Map();

// The on-screen canvas: a backing store at the device pixel ratio, cleared, then painted.
function replay(canvas, ops, strs, w, h) {
  const dpr = devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.round(w * dpr));
  canvas.height = Math.max(1, Math.round(h * dpr));
  const ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  ctx.clearRect(0, 0, w, h);
  paint(ctx, ops, strs);
}

// Whether the page asked for debug logging (`?DAY_LOG=debug`, docs/logging.md): the shim's own
// timing lines follow the same switch as the wasm side's.
const debugLog = new URLSearchParams(location.search).get('DAY_LOG') === 'debug';

// The one painter: the op stream drawn into `ctx` in canvas points, under whatever transform
// the caller set. The on-screen canvas (`replay`) and an exported bitmap
// (`day_dom_canvas_render_bitmap`) both paint through it, so the two cannot drift apart.
function paint(ctx, ops, strs) {
  let i = 0;
  const next = () => ops[i++];
  // Backing-store pixels per canvas point. A context's shadow offset and blur are in those
  // pixels and ignore the transform (docs/canvas.md "Shadows"), so points are scaled by this.
  const entry = ctx.getTransform();
  const unit = Math.hypot(entry.a, entry.b) || 1;
  // Whether a shadow is set, tracked beside the context's own save/restore stack: the
  // gradients painted through a clip need to know (`shadowed`).
  let shadowOn = false;
  const shadowStack = [];
  const readPaint = () => {
    const kind = next();
    if (kind === 0) return rgba(next());
    if (kind === 1) {
      const g = ctx.createLinearGradient(next(), next(), next(), next());
      const n = next();
      for (let k = 0; k < n; k++) g.addColorStop(Math.min(1, Math.max(0, next())), rgba(next()));
      return g;
    }
    if (kind === 3) {
      // conic: center, start angle in degrees, stops. The context's own conic gradient has the
      // spec's geometry (0 = +x, clockwise, offsets as fractions of a turn). An engine without
      // it (Safari before 16.1) gets a fan of wedges, made at the draw (`solve`).
      const cx = next(), cy = next(), start = next(), n = next();
      const stops = [];
      for (let k = 0; k < n; k++) stops.push([Math.min(1, Math.max(0, next())), next()]);
      if (!ctx.createConicGradient) return { __conic: [cx, cy, start, stops] };
      const g = ctx.createConicGradient((start * Math.PI) / 180, cx, cy);
      for (const [at, color] of stops) g.addColorStop(at, rgba(color));
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
    else if (kind === 1) { const x = next(), y = next(), pw = next(), ph = next(), r = next(); p.roundRect(x, y, pw, ph, Math.max(r, 0)); }
    else if (kind === 2) { const x = next(), y = next(), pw = next(), ph = next(); p.ellipse(x + pw / 2, y + ph / 2, pw / 2, ph / 2, 0, 0, Math.PI * 2); }
    else if (kind === 3) {
      const x = next(), y = next(), pw = next(), ph = next(), start = next(), sweep = next();
      p.ellipse(x + pw / 2, y + ph / 2, pw / 2, ph / 2, 0, (start * Math.PI) / 180, ((start + sweep) * Math.PI) / 180);
    } else if (kind === 4) { p.moveTo(next(), next()); p.lineTo(next(), next()); }
    else if (kind === 5 || kind === 6) {
      // Variable-length geometry: [key, payloadLen, …]. A key already decoded keeps its Path2D
      // and the payload is jumped rather than walked, so a drawing re-recorded every frame
      // rebuilds no geometry here.
      const key = next(), len = next(), at = i;
      const hit = key ? pathCache.get(key) : undefined;
      if (hit) { i = at + len; return hit; }
      if (kind === 5) {
        const n = next();
        for (let k = 0; k < n; k++) { const x = next(), y = next(); k === 0 ? p.moveTo(x, y) : p.lineTo(x, y); }
        p.closePath();
      } else {
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
      if (key) {
        if (pathCache.size > 512) pathCache.clear();
        pathCache.set(key, p);
      }
    }
    return p;
  };
  // A paint as something a context takes for a style, under the transform it will draw in.
  const solve = (paint) => (paint.__conic ? conicPattern(ctx, ...paint.__conic) : paint);
  // A radial gradient is painted THROUGH A CLIP, and a clip cuts the shadow as well as the
  // paint. So under a shadow the drawing goes to a layer the size of the backing store first,
  // and the layer is drawn back as one image: the context then shadows (and blends) the
  // layer's own silhouette. `draw` gets the context to paint into.
  const shadowed = (draw) => {
    if (!shadowOn) { draw(ctx); return; }
    const layer = Object.assign(document.createElement('canvas'),
      { width: ctx.canvas.width, height: ctx.canvas.height });
    const l = layer.getContext('2d');
    if (!l) { draw(ctx); return; }
    l.setTransform(ctx.getTransform());
    draw(l);
    ctx.save(); ctx.setTransform(1, 0, 0, 1, 0, 0); ctx.drawImage(layer, 0, 0); ctx.restore();
  };
  const fillWith = (paint, p) => {
    if (paint.__radial) {
      const [cx, cy, rx, ry] = paint.__radial;
      shadowed((c) => {
        c.save(); c.clip(p); c.translate(cx, cy); c.scale(rx, ry);
        c.fillStyle = paint.g; c.fillRect(-1, -1, 2, 2); c.restore();
      });
    } else { ctx.fillStyle = solve(paint); ctx.fill(p, p.__rule || 'nonzero'); }
  };
  // The stroke style (width, cap, join, dash) is already on `ctx`.
  const strokeWith = (paint, p) => {
    if (paint.__radial) {
      // No gradient-stroke primitive: clip to the stroked region, then paint the gradient
      // through it. Canvas2D has no "stroke to path", so the clip is the stroke geometry.
      const [cx, cy, rx, ry] = paint.__radial;
      shadowed((c) => {
        c.save(); c.strokeStyle = '#000'; c.clip(strokeRegion(c, p));
        c.translate(cx, cy); c.scale(rx, ry);
        c.fillStyle = paint.g; c.fillRect(-1, -1, 2, 2); c.restore();
      });
    } else { ctx.strokeStyle = solve(paint); ctx.stroke(p); }
  };
  while (i < ops.length) {
    const op = next();
    if (op === 0) { // fill
      const paint = readPaint(); const p = path();
      fillWith(paint, p);
    } else if (op === 7) { // stamp: strokeFlag, (style)?, count, xy…, paint, template shape
      const stroked = next();
      if (stroked) {
        ctx.lineWidth = next();
        ctx.lineCap = ['butt', 'round', 'square'][next()] || 'butt';
        ctx.lineJoin = ['miter', 'round', 'bevel'][next()] || 'miter';
        ctx.miterLimit = next();
        ctx.lineDashOffset = next();
        const nd = next(); const dash = [];
        for (let k = 0; k < nd; k++) dash.push(next());
        ctx.setLineDash(dash);
      }
      const n = next();
      const at = new Float64Array(n * 2);
      for (let k = 0; k < n * 2; k++) at[k] = next();
      const paint = readPaint(); const p = path();
      if (typeof paint === 'string') {
        // One Path2D holding every copy, then one fill or stroke: the whole batch enters the
        // rasterizer once, however many positions it has (docs/canvas.md "Stamping").
        const batch = new Path2D();
        const m = new DOMMatrix();
        for (let k = 0; k < n; k++) {
          m.e = at[k * 2]; m.f = at[k * 2 + 1];
          batch.addPath(p, m);
        }
        if (stroked) { ctx.strokeStyle = paint; ctx.stroke(batch); }
        else { ctx.fillStyle = paint; ctx.fill(batch, p.__rule || 'nonzero'); }
      } else {
        // A gradient is placed against the template's own bounds, so each copy is drawn where
        // the template is, moved: every copy is shaded alike.
        for (let k = 0; k < n; k++) {
          ctx.save(); ctx.translate(at[k * 2], at[k * 2 + 1]);
          if (stroked) strokeWith(paint, p); else fillWith(paint, p);
          ctx.restore();
        }
      }
      if (stroked) ctx.setLineDash([]);
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
      strokeWith(paint, p);
      ctx.setLineDash([]);
    } else if (op === 6) { // clip
      const p = path();
      ctx.clip(p, p.__rule || 'nonzero');
    } else if (op === 2) { // text: anchor packed as h * 4 + v (TextAnchor::pack)
      ctx.fillStyle = rgba(next());
      const size = next(), anchor = next(), x = next(), y = next(), off = next(), len = next();
      const weight = next(), italic = next(), famOff = next(), famLen = next();
      const family = famLen > 0 ? utf8.decode(strs.slice(famOff, famOff + famLen)) : '';
      const text = utf8.decode(strs.slice(off, off + len));
      ctx.font = canvasFont(size, weight, italic, family);
      // fillText takes the BASELINE; the line box is the font's bounding box, the same
      // numbers day_dom_measure_text reports, so a measured frame hugs what is drawn.
      const [asc, desc] = fontBox(ctx, text, size);
      ctx.textBaseline = 'alphabetic';
      // Horizontal alignment is the canvas's own; vertical is an offset from the line box, which
      // is what `textBaseline: top/middle/bottom` would give but computed from the same ascent
      // day_dom_measure_text reports, so the drawn box and the measured one agree exactly.
      const h = Math.floor(anchor / 4), v = anchor % 4;
      ctx.textAlign = h === 1 ? 'center' : h === 2 ? 'right' : 'left';
      const dy = v === 1 ? -(asc + desc) / 2 : v === 2 ? -asc : v === 3 ? -(asc + desc) : 0;
      ctx.fillText(text, x, y + dy + asc);
    } else if (op === 3) { ctx.save(); shadowStack.push(shadowOn); }
    else if (op === 4) { ctx.restore(); if (shadowStack.length) shadowOn = shadowStack.pop(); }
    else if (op === 10) { // shadow: dx, dy, blur in points, color (clear = none)
      // The context's shadow is the spec's: cast by every fill, stroke, text and image, outside
      // the transform, part of save/restore, and `shadowBlur` is twice the Gaussian's sigma.
      const dx = next(), dy = next(), blur = next(), color = next();
      shadowOn = ((color >>> 0) & 255) !== 0;
      ctx.shadowColor = shadowOn ? rgba(color) : 'transparent';
      ctx.shadowOffsetX = dx * unit; ctx.shadowOffsetY = dy * unit;
      ctx.shadowBlur = Math.max(0, blur) * unit;
    } else if (op === 11) { // blend: the mode's number (day_spec::BlendMode::wire)
      // The backdrop is this canvas's own pixels, and `destination-out` leaves them clear, so
      // the element behind shows through an erased hole. Part of save/restore.
      ctx.globalCompositeOperation = BLEND_MODES[next()] || 'source-over';
    }
    else if (op === 5) { const a = next(), b = next(), c = next(), d = next(), e = next(), f = next(); ctx.transform(a, b, c, d, e, f); }
    else if (op === 8 || op === 9) { // image: id, x, y, w, h, opacity (docs/images.md)
      // Every slot is read before the bitmap is looked up: this stream is variable-length, so a
      // branch that consumed fewer slots would desync every record after it. Op 9 is the same
      // with a source region (sx, sy, sw, sh) in the bitmap's own pixels after the opacity.
      const id = next(), x = next(), y = next(), w = next(), h = next(), alpha = next();
      let sx = 0, sy = 0, sw = 0, sh = 0;
      if (op === 9) { sx = next(); sy = next(); sw = next(); sh = next(); }
      const b = bitmaps.get(id);
      // A released bitmap draws nothing rather than a placeholder: a canvas re-records on every
      // tracked read, so a handle can be dropped between the record and this replay.
      if (b && b.bitmap && w > 0 && h > 0) {
        const was = ctx.globalAlpha;
        ctx.globalAlpha = was * Math.max(0, Math.min(1, alpha));
        if (op === 8) ctx.drawImage(b.bitmap, x, y, w, h);
        else {
          // The region clamped to the bitmap with its mapping into the rect kept
          // (day_spec::clamp_image_source): the part inside draws where it would have, an empty
          // intersection draws nothing. Done here rather than left to `drawImage` so every
          // engine agrees, and so an out-of-range source can never throw mid-replay.
          const x0 = Math.max(0, sx), y0 = Math.max(0, sy);
          const x1 = Math.min(b.bitmap.width, sx + sw), y1 = Math.min(b.bitmap.height, sy + sh);
          if (sw > 0 && sh > 0 && x1 > x0 && y1 > y0) {
            const kx = w / sw, ky = h / sh;
            ctx.drawImage(b.bitmap, x0, y0, x1 - x0, y1 - y0,
              x + (x0 - sx) * kx, y + (y0 - sy) * ky, (x1 - x0) * kx, (y1 - y0) * ky);
          }
        }
        ctx.globalAlpha = was;
      }
    }
  }
}

// `globalCompositeOperation` for each day_spec::BlendMode, in wire order; a number past the end
// draws as Normal.
const BLEND_MODES = ['source-over', 'multiply', 'screen', 'overlay', 'darken', 'lighten', 'lighter', 'destination-out'];

// The color of a conic gradient `t` of a turn past its start (day_spec::ConicGradient::color_at):
// linear between stops, channel by channel, flat before the first and after the last. `stops`
// is [[offset, packed rgba], …]; the answer is [r, g, b, a] with a in 0..1.
function conicColorAt(stops, t) {
  const un = (v) => [(v >>> 24) & 255, (v >>> 16) & 255, (v >>> 8) & 255, (v & 255) / 255];
  if (!stops.length) return [0, 0, 0, 0];
  if (t <= stops[0][0]) return un(stops[0][1] >>> 0);
  for (let k = 1; k < stops.length; k++) {
    const [a0, ca] = stops[k - 1], [a1, cb] = stops[k];
    if (t <= a1) {
      const f = a1 > a0 ? (t - a0) / (a1 - a0) : 1;
      const a = un(ca >>> 0), b = un(cb >>> 0);
      return a.map((v, n) => v + (b[n] - v) * f);
    }
  }
  return un(stops[stops.length - 1][1] >>> 0);
}

// A conic gradient for an engine with no `createConicGradient`, as a pattern `ctx` can fill or
// stroke with under its current transform: a fan of half-degree wedges about (cx, cy), each in
// the color at its middle, painted into a canvas the size of the backing store. The wedges
// share edges exactly and are summed (`lighter`), so the antialiased halves of a shared edge
// add up to full coverage: no seam, no doubled alpha. The pattern is pinned to the backing
// store's pixels, so it lines up whatever the transform.
function conicPattern(ctx, cx, cy, startDeg, stops) {
  const w = ctx.canvas.width, h = ctx.canvas.height;
  const fan = Object.assign(document.createElement('canvas'), { width: w, height: h });
  const f = fan.getContext('2d');
  const m = ctx.getTransform();
  if (!f || !(Math.abs(m.a * m.d - m.b * m.c) > 1e-12)) return 'rgba(0,0,0,0)';
  const inv = m.inverse();
  // Far enough to cover the whole store from the center, in canvas points.
  let reach = 1;
  for (const [px, py] of [[0, 0], [w, 0], [0, h], [w, h]]) {
    const q = inv.transformPoint({ x: px, y: py });
    reach = Math.max(reach, Math.hypot(q.x - cx, q.y - cy));
  }
  const WEDGES = 720, step = (Math.PI * 2) / WEDGES;
  reach = (reach + 2) / Math.cos(step / 2);
  f.setTransform(m);
  f.globalCompositeOperation = 'lighter';
  const start = (startDeg * Math.PI) / 180;
  for (let k = 0; k < WEDGES; k++) {
    const a0 = start + k * step, a1 = a0 + step;
    const [r, g, b, a] = conicColorAt(stops, (k + 0.5) / WEDGES);
    f.fillStyle = `rgba(${Math.round(r)},${Math.round(g)},${Math.round(b)},${a})`;
    f.beginPath();
    f.moveTo(cx, cy);
    f.lineTo(cx + reach * Math.cos(a0), cy + reach * Math.sin(a0));
    f.lineTo(cx + reach * Math.cos(a1), cy + reach * Math.sin(a1));
    f.closePath();
    f.fill();
  }
  const pattern = ctx.createPattern(fan, 'no-repeat');
  pattern.setTransform(inv);
  return pattern;
}

// Owned byte transport for day-part-clipboard; callbacks consume day_dom_alloc buffers.
function clipResult(req,status,bytes=new Uint8Array()) {
  const p = bytes.length ? wasm.day_dom_alloc(bytes.length) : 0;
  if (bytes.length) mem().set(bytes,p);
  wasm.day_clipboard_result(req,status,p,bytes.length);
}
function clipPacket(reps) {
  const out=new Uint8Array(4+reps.reduce((n,r)=>n+8+utf8enc.encode(r.mime).length+r.bytes.length,0));
  const view=new DataView(out.buffer); view.setUint32(0,reps.length,true); let at=4;
  for (const r of reps) { const m=utf8enc.encode(r.mime); view.setUint32(at,m.length,true);view.setUint32(at+4,r.bytes.length,true);at+=8;out.set(m,at);at+=m.length;out.set(r.bytes,at);at+=r.bytes.length; }
  return out;
}

// Clipboard plumbing (docs/menus.md): the live DOM clipboard event while wasm handles it,
// the staged outbound bytes, and the page-local mirror of the last in-page copy.
let activeClipboardEvent = null, clipboardStaged = null, clipboardMirror = null;
const pendingClipboardWrites = new Set();
// The modifier keys as last observed (bit0 shift, bit1 primary = meta|ctrl, bit2 alt);
// wasm pulls this ambiently for interactions modifiers change (shift-click multi-select).
let modifierMask = 0;
const trackModifiers = (e) => {
  modifierMask = (e.shiftKey ? 1 : 0) | ((e.metaKey || e.ctrlKey) ? 2 : 0) | (e.altKey ? 4 : 0);
};

// ---------------------------------------------------------------------------
// Dialogs (docs/dialogs.md): <dialog>-backed alert/confirm/sheet/prompt.
// ---------------------------------------------------------------------------

const dialogs = new Map();

function present(req, spec) {
  // The open picker is the browser's file input: hidden, clicked, answered on change/cancel.
  if (spec.kind === 'open') {
    const input = document.createElement('input');
    input.type = 'file';
    input.multiple = !!spec.multiple;
    if (spec.accept) input.accept = spec.accept;
    input.style.display = 'none';
    document.body.append(input);
    input.addEventListener('change', async () => {
      const files = Array.from(input.files);
      input.remove();
      if (!files.length) { wasm.day_dom_present_result(req, -1, 0, 0); return; }
      // Every file but the last is staged; the last one closes the pick with all of them.
      for (let i = 0; i < files.length; i++) {
        const f = files[i];
        const buf = new Uint8Array(await f.arrayBuffer());
        const [np, nl] = intoWasm(f.name);
        const bp = wasm.day_dom_alloc(buf.length);
        mem().set(buf, bp);
        if (i < files.length - 1) wasm.day_dom_present_file_stage(req, np, nl, bp, buf.length);
        else wasm.day_dom_present_files(req, np, nl, bp, buf.length);
      }
    });
    input.addEventListener('cancel', () => { input.remove(); wasm.day_dom_present_result(req, -1, 0, 0); });
    input.click();
    return;
  }
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

// The save flow: wrap the staged bytes in a Blob, click a download link (the browser's
// "save file" surface). The answer is deferred a tick: this runs synchronously inside the wasm
// present call, and answering re-enters wasm while the presenting frame is still live.
function presentSave(req, spec, bytes) {
  const name = spec.name || 'download';
  const url = URL.createObjectURL(new Blob([bytes]));
  const a = document.createElement('a');
  a.href = url; a.download = name;
  document.body.append(a); a.click(); a.remove();
  setTimeout(() => URL.revokeObjectURL(url), 60000);
  setTimeout(() => {
    const [np, nl] = intoWasm(name);
    wasm.day_dom_present_files(req, np, nl, 0, 0);
  }, 0);
}

// ---------------------------------------------------------------------------
// Boot
// ---------------------------------------------------------------------------

// Embedded resources: borrowed Wasm bytes, and browser-owned URLs cached per file.
const bundledFiles = new Map();
const bundledUrls = new Map();
function unpackResource(pair) { return [Number(pair & 0xffffffffn), Number(pair >> 32n)]; }
function resourceBytes(path) {
  const entry = bundledFiles.get(path);
  if (!entry) throw new Error(`Missing bundled resource: ${path}`);
  return new Uint8Array(wasm.memory.buffer, entry[0], entry[1]);
}
function resourceMime(path) {
  const ext = path.split('.').pop().toLowerCase();
  return ({ png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', svg: 'image/svg+xml',
    webp: 'image/webp', gif: 'image/gif', ico: 'image/x-icon', avif: 'image/avif',
    ttf: 'font/ttf', otf: 'font/otf', woff: 'font/woff', woff2: 'font/woff2',
    wav: 'audio/wav', mp3: 'audio/mpeg', ogg: 'audio/ogg', mp4: 'video/mp4',
    html: 'text/html', htm: 'text/html', css: 'text/css', js: 'text/javascript', json: 'application/json',
    txt: 'text/plain', wasm: 'application/wasm' })[ext] ?? 'application/octet-stream';
}
function resourceUrl(path) {
  if (!bundledFiles.has(path) && path.startsWith('assets/images/')) {
    const stem = path.replace(/\.[^.]+$/, '');
    const base = stem.replace(/__(light|bold)$/, '');
    const candidates = [stem + '.svg', base + '.svg', stem + '.png', base + '.png', stem + '.jpg', stem + '.jpeg', stem + '.webp'];
    path = candidates.find(name => bundledFiles.has(name)) ?? path;
  }
  if (!bundledFiles.has(path)) return path;
  if (!bundledUrls.has(path)) {
    bundledUrls.set(path, URL.createObjectURL(new Blob([resourceBytes(path)], {type: resourceMime(path)})));
  }
  return bundledUrls.get(path);
}
function resourceStyle(value) {
  return value.replace(/url\(["']?(assets\/[^"')]+)["']?\)/g, (_, path) => `url("${resourceUrl(path)}")`);
}
const readySiteRoots = new Set();
const siteInstalls = new Map();
let siteRoots = [];
let siteWorker;
function bundledPath(path) {
  const url = new URL(path, document.baseURI);
  const root = new URL('./', document.baseURI);
  if (url.origin !== root.origin || !url.pathname.startsWith(root.pathname)) return null;
  const relative = decodeURIComponent(url.pathname.slice(root.pathname.length));
  return relative.startsWith('assets/data/') ? {url, root, relative} : null;
}
function bundledSitePath(path, directory) {
  const found = bundledPath(path);
  if (!found) return null;
  const {relative} = found;
  const site = directory && directory.startsWith('assets/data/') && directory.endsWith('/')
    && relative.startsWith(directory) && [...bundledFiles.keys()].some(name => name.startsWith(directory))
    ? directory : siteRoots.find(root => relative.startsWith(root));
  return site ? {...found, site} : null;
}
function resourceSiteUrl(path) {
  const found = bundledPath(path);
  if (!found || ![...readySiteRoots].some(site => found.relative.startsWith(site))) return path;
  const {url, root, relative} = found;
  return new URL('__day_bundle/' + window.__DAY_BUNDLE + '/' + relative.split('/').map(encodeURIComponent).join('/') + url.search + url.hash, root).href;
}
function supportsResourceSites(worker) {
  return new Promise(resolve => {
    const channel = new MessageChannel();
    const finish = supported => { clearTimeout(timer); channel.port1.close(); resolve(supported); };
    const timer = setTimeout(() => finish(false), 1000);
    channel.port1.onmessage = event => finish(event.data === 2);
    worker.postMessage({type: 'day-resource-protocol'}, [channel.port2]);
  });
}
async function activateSiteWorker() {
  const reg = await navigator.serviceWorker.register(new URL('sw.js', document.baseURI));
  // An update may be waiting for old fetch events to finish. An active worker with this
  // protocol can serve any module version already, so the frame need not await the update.
  if (reg.active?.state === 'activated' && await supportsResourceSites(reg.active)) return reg.active;
  // Promotion to registration.active precedes the end of activation. During that interval
  // installing and waiting are null, but the active worker is still a valid candidate.
  const worker = reg.installing || reg.waiting || reg.active;
  if (!worker) throw new Error('No compatible resource service worker');
  if (worker.state !== 'activated') await new Promise((resolve, reject) => {
    const changed = () => {
      if (worker.state === 'activated' || worker.state === 'redundant') {
        worker.removeEventListener('statechange', changed);
        worker.state === 'activated' ? resolve() : reject(new Error('Resource worker installation failed'));
      }
    };
    worker.addEventListener('statechange', changed); changed();
  });
  if (!await supportsResourceSites(worker)) throw new Error('Unsupported resource service worker');
  return worker;
}
async function populateSite(site) {
  const version = window.__DAY_BUNDLE;
  if (!/^[a-f0-9]{64}$/.test(version)) throw new Error('Missing bundle version');
  const root = new URL('./', document.baseURI);
  const cache = await caches.open('day-resources-' + encodeURIComponent(root.pathname) + '-' + version);
  const marker = new URL('__day_ready/2/' + encodeURIComponent(site), root);
  // A completion marker is written last: interrupted installs are retried, complete sites
  // are reused without rewriting files. Concurrent tabs may safely write identical bytes.
  if (!await cache.match(marker)) {
    await Promise.all([...bundledFiles.keys()].filter(name => name.startsWith(site)).map(name =>
      cache.put(new URL(name, root), new Response(resourceBytes(name).slice(), {
        headers: {'Content-Type': resourceMime(name), 'Cross-Origin-Embedder-Policy': 'require-corp'},
      }))));
    await cache.put(marker, new Response('1'));
  }
  siteWorker ??= activateSiteWorker();
  const worker = await siteWorker;
  worker.postMessage({type: 'day-resource-cleanup', version});
}
async function prepareResourceSite(path, directory) {
  const found = bundledSitePath(path, directory);
  if (!found || !('serviceWorker' in navigator) || !globalThis.caches) return path;
  if (!siteInstalls.has(found.site)) {
    siteInstalls.set(found.site, (async () => {
      let timer;
      try {
        // Only the requesting iframe waits. Storage/worker hangs cannot hold app startup.
        await Promise.race([populateSite(found.site), new Promise((_, reject) => {
          timer = setTimeout(() => reject(new Error('Resource site setup timed out')), 10000);
        })]);
        readySiteRoots.add(found.site);
      } catch (error) {
        console.warn('day: bundled document uses published URLs:', error);
      } finally { clearTimeout(timer); }
    })());
  }
  await siteInstalls.get(found.site);
  return resourceSiteUrl(path);
}
function setResourceFrameSource(frame, path) {
  const request = {};
  frame.__daySource = request;
  // Remote pages and non-site sources retain synchronous DOM attribute behavior.
  const directory = frame.getAttribute?.('data-day-site-root');
  if (!bundledSitePath(path, directory)) { frame.setAttribute('src', path); return; }
  void prepareResourceSite(path, directory).then(url => {
    if (frame.__daySource === request && frame.isConnected) frame.setAttribute('src', url);
  });
}
async function installResources() {
  // Resource generation is optional (and the CLI and app can have different versions).
  // An older app keeps published image/site URLs; its resource table is simply empty.
  wasm.day_dom_resources_init?.();
  const count = wasm.day_dom_resources_count?.() ?? 0;
  for (let i = 0; i < count; i++) {
    const [p, n] = unpackResource(wasm.day_dom_resources_name(i));
    bundledFiles.set(str(p, n), unpackResource(wasm.day_dom_resources_bytes(i)));
  }
  const roots = [...bundledFiles.keys()].filter(name => name.startsWith('assets/data/') && /\.html?$/i.test(name))
    .map(name => name.slice(0, name.lastIndexOf('/') + 1)).sort();
  siteRoots = roots.filter((root, index) => !roots.slice(0, index).some(parent => root.startsWith(parent)));
  navigator.serviceWorker?.addEventListener('message', event => {
    if (event.data?.type === 'day-resource-version') event.ports[0]?.postMessage({version: window.__DAY_BUNDLE});
  });
  const cleanup = () => navigator.serviceWorker?.controller?.postMessage({type: 'day-resource-cleanup', version: window.__DAY_BUNDLE});
  navigator.serviceWorker?.addEventListener('controllerchange', cleanup);
  cleanup();
  const manifest = bundledFiles.has('assets/fonts/fonts.json')
    ? JSON.parse(new TextDecoder().decode(resourceBytes('assets/fonts/fonts.json'))) : [];
  await Promise.all(manifest.map(async ({family, url}) => {
    const face = new FontFace(family, resourceBytes(url));
    await face.load();
    document.fonts.add(face);
  }));
}

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
  // `?theme=` pins the appearance at boot; without it the page follows the browser, live.
  const theme = new URLSearchParams(location.search).get('theme');
  if (theme) appearanceMode = theme === 'dark' ? 1 : 0;
  showAppearance((theme ?? (prefersDark().matches ? 'dark' : 'light')) === 'dark');

  const r = root();
  r.__id = 1; els[1] = r;

  // Streaming instantiation needs the server to answer `application/wasm`; fall back to a
  // buffered instantiate so the page also works on static hosts with looser MIME tables.
  // Web modules (docs/web.md): a crate's browser half ships as its own ES module beside this
  // shim, rather than being hand-written into it. `day build` lists them in
  // `window.__DAY_MODULES`: the daybridge web arms it generates (docs/bridge.md) and the modules
  // crates declare in `[package.metadata.day.web]`. Each may export `register(rt)`, returning the
  // wasm imports it implements, which join `env` before instantiation, and `start(rt, module)`,
  // which gets the compiled app module and is awaited before the app's entry point runs. A
  // bridge import cannot collide with another crate's (every one is `day_bridge_<crate>_<fn>`),
  // and a module that fails to load must not take the page down silently, so a failure is
  // logged and its imports stay unimplemented.
  // The runtime a module gets. `exports` is a lazy accessor because the instance is bound
  // only after every module has registered; an asynchronous arm's completion (docs/bridge.md
  // "Callbacks") reaches wasm through it, handing strings and bytes over as (ptr, len) pairs it
  // allocated with `intoWasm` / `bytesIntoWasm`.
  const bridgeRt = {
    str, mem, memWrite, intoWasm,
    // Module callers own their bytes, just like rt.bytes(ptr, len). They may retain or
    // transfer them without detaching or mutating Rust's immutable static allocation.
    resources: { bytes: path => resourceBytes(path).slice(), url: resourceUrl, siteUrl: resourceSiteUrl, prepareSite: prepareResourceSite },
    dom: {
      element: (id) => E(id),
      onRelease(id, dispose) {
        if (!E(id)) throw new Error(`day: unknown element ${id}`);
        const callbacks = releaseHooks.get(id) ?? [];
        callbacks.push(dispose);
        releaseHooks.set(id, callbacks);
      },
      emit(id, num, text = '') {
        if (!E(id)) return;
        const [ptr, len] = intoWasm(text);
        wasm.day_dom_piece_event(id, num, ptr, len);
      },
    },
    // A `&[u8]` argument, copied out of wasm memory so the arm may keep it.
    bytes: (ptr, len) => new Uint8Array(wasm.memory.buffer, ptr, len).slice(),
    bytesIntoWasm: (bytes) => {
      const u8 = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes ?? []);
      const ptr = wasm.day_dom_alloc(u8.length);
      mem().set(u8, ptr);
      return [ptr, u8.length];
    },
    exports: () => wasm,
  };
  const starters = [];
  for (const url of window.__DAY_MODULES ?? []) {
    try {
      const mod = await import(url);
      if (mod.register) Object.assign(env, mod.register(bridgeRt));
      if (mod.start) starters.push([url, mod.start]);
    } catch (e) {
      console.error(`day: web module ${url} failed to load`, e);
    }
  }

  // The compiled module is structured-cloneable, so a web module that runs the app's wasm a
  // second time (day-sqlite-worker's SQL worker does) gets it from `start` instead of compiling
  // the bytes again. The table is installed synchronously after instantiation; module starters
  // run alongside font loading and are awaited just before day_dom_main: by the time app code runs, whatever a module boots is either up or
  // definitively absent, so Rust needs no async path to find out.
  let module;
  try {
    module = await WebAssembly.compileStreaming(fetch(wasmUrl));
  } catch {
    module = await WebAssembly.compile(await (await fetch(wasmUrl)).arrayBuffer());
  }
  wasm = (await WebAssembly.instantiate(module, foreignStubs(module, { env }))).exports;
  const resourcesReady = installResources();
  const started = Promise.all(starters.map(async ([url, start]) => {
    try {
      await start(bridgeRt, module);
    } catch (e) {
      console.error(`day: web module ${url} failed to start`, e);
    }
  }));

  // Registry comes from the final served module, never from mounted UI. All section
  // contributions are framed records; Rust validates schema and the empty-catalog sentinel.
  const idSections = WebAssembly.Module.customSections(module, 'day_ids');
  const idLength = idSections.reduce((n, section) => n + section.byteLength, 0);
  if (idLength > 16 * 1024 * 1024) throw new Error('Day element registry exceeds 16 MiB');
  const idPtr = wasm.day_dom_alloc(idLength);
  let idOffset = 0;
  for (const section of idSections) {
    new Uint8Array(wasm.memory.buffer, idPtr + idOffset, section.byteLength).set(new Uint8Array(section));
    idOffset += section.byteLength;
  }
  wasm.day_dom_install_ids(idPtr, idLength);

  // Independent expectations recorded from rustc's exports BEFORE the linker ran.
  // Missing/duplicate sections fail closed in Rust; never derive this from inventory.
  const testSections = WebAssembly.Module.customSections(module, 'day_test_expectations');
  const testBytes = testSections.length === 1 ? new Uint8Array(testSections[0]) : new Uint8Array();
  if (testBytes.length > 16 * 1024 * 1024) throw new Error('Day test manifest exceeds 16 MiB');
  const testPtr = wasm.day_dom_alloc(testBytes.length);
  new Uint8Array(wasm.memory.buffer, testPtr, testBytes.length).set(testBytes);
  wasm.day_dom_install_tests(testPtr, testBytes.length);


  // Installed PWA file handling (Chromium). Other browsers retain the ordinary picker.
  if ('launchQueue' in window) window.launchQueue.setConsumer(async launch => {
    for (const handle of launch.files || []) {
      try {
        const file = await handle.getFile();
        if (file.size > 512 * 1024 * 1024) throw new Error('Document exceeds staging limit');
        const bytes = new Uint8Array(await file.arrayBuffer());
        const [np, nl] = intoWasm(file.name);
        const bp = wasm.day_dom_alloc(bytes.length);
        new Uint8Array(wasm.memory.buffer, bp, bytes.length).set(bytes);
        wasm.day_dom_present_files(0, np, nl, bp, bytes.length);
      } catch (error) {
        console.error('Day document activation failed', error);
        const [np, nl] = intoWasm('document-unavailable');
        wasm.day_dom_present_files(0, np, nl, 0, 0);
      }
    }
  });

  new ResizeObserver(() => wasm.day_dom_resized(r.clientWidth, r.clientHeight)).observe(r);
  document.addEventListener('visibilitychange', () =>
    wasm.day_dom_lifecycle(document.visibilityState === 'visible' ? 0 : 1));
  // The page's last moment (a navigation, a reload, a closed tab): DidExit, after which
  // nothing of the app runs.
  addEventListener('pagehide', () => wasm.day_dom_lifecycle(2));
  // Fullscreen came or went, by the app's request or the user's (Esc, the browser's own UI).
  document.addEventListener('fullscreenchange', () => wasm.day_dom_window_state(fullscreenCode(), 0));
  // The reduce-motion setting flipped under the app: day-core re-reads it through
  // `day_dom_env('reduce_motion')` and re-gates its transitions.
  reduceMotion().addEventListener('change', () => wasm.day_dom_motion_changed());
  // The browser's appearance changed (the system's, or a devtools emulation): a page following
  // it switches, and every ink follows in CSS (day-toolkit-dom `ink_css`); Day re-reads its
  // palette and redraws its canvases.
  prefersDark().addEventListener('change', (e) => {
    if (appearanceMode !== 2) return;
    showAppearance(e.matches);
    wasm.day_dom_appearance_changed(e.matches ? 1 : 0);
  });
  // Hash changes we did not write ourselves (back/forward, a hand-edited URL) are route
  // requests for the app.
  window.addEventListener('hashchange', () => {
    const route = location.hash.slice(1);
    if (route === lastSetRoute) return;
    lastSetRoute = route;
    const [p, l] = intoWasm(route);
    wasm.day_dom_hash_changed(p, l);
  });

  // Modifier tracking rides the events that carry the flags.
  for (const ev of ['keydown', 'keyup', 'pointerdown', 'pointermove', 'pointerup']) {
    document.addEventListener(ev, trackModifiers, true);
  }

  // The platform-standard keys the browser has no document-level route for: undo (⌘Z /
  // Ctrl+Z, shifted or Ctrl+Y for redo) and select-all (⌘A), skipped while an editable
  // element has them (its own text behavior applies).
  //
  // The arrows are not here. They follow focus instead, delivered by whichever
  // widget holds it (day_dom_list_keynav, day_dom_navmenu, day_dom_canvas_keynav), because a
  // document-level route cannot tell a nudge the app wants from the keys a focused list needs.
  document.addEventListener('keydown', (e) => {
    const t = e.target;
    if (t && t.closest && t.closest('input, textarea, [contenteditable]')) return;
    if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
    const k = e.key.toLowerCase();
    if (k === 'z') { e.preventDefault(); wasm.day_dom_undo(e.shiftKey ? 1 : 0); }
    else if (k === 'y') { e.preventDefault(); wasm.day_dom_undo(1); }
    else if (k === 'a') { e.preventDefault(); wasm.day_dom_edit(3); }
  });

  // The browser's own edit-command route (⌘X/C/V, its Edit menu): when no editable element
  // claims the event, it is the app's, forwarded synchronously so the handler's clipboard
  // calls hit this very event's clipboardData (docs/menus.md).
  const editOps = { cut: 0, copy: 1, paste: 2 };
  for (const kind of Object.keys(editOps)) {
    document.addEventListener(kind, (e) => {
      const t = e.target;
      if (t && t.closest && t.closest('input, textarea, [contenteditable]')) return;
      e.preventDefault();
      activeClipboardEvent = e;
      try { wasm.day_dom_edit(editOps[kind]); } finally { activeClipboardEvent = null; }
    });
  }

  // dayscript (docs/web.md): when the serving `day launch` session armed scripting
  // (?dayscript= token), open a same-origin WebSocket the dev server bridges to the runner's
  // TCP protocol, and pipe request lines into the engine.
  if (new URLSearchParams(location.search).get('dayscript')) {
    scriptWs = new WebSocket(`ws://${location.host}/dayscript`);
    scriptWs.addEventListener('open', () => {
      for (const line of scriptOutbox.splice(0)) scriptWs.send(line);
    });
    scriptWs.addEventListener('message', (ev) => {
      // Lines can arrive while boot still awaits the web modules below, before
      // day_dom_main, when the engine cannot take them. Queue until the app starts.
      if (!appStarted) { scriptInbox.push(String(ev.data)); return; }
      const [p, l] = intoWasm(String(ev.data));
      wasm.day_dom_script_line(p, l);
    });
  }

  await Promise.all([started, resourcesReady]);
  wasm.day_dom_main();
  appStarted = true;
  for (const line of scriptInbox.splice(0)) {
    const [p, l] = intoWasm(line);
    wasm.day_dom_script_line(p, l);
  }
}

// Native HTML transfer. DataTransfer is accessed only during its permitted event lifetime.
const dragBundle = 'application/vnd.day.transfer';
const dragLimit = 64 * 1024 * 1024;
let dragOffer = null, localDrag = null;
function transferCall(fn, id, x, y, bytes, local) {
  const p = wasm.day_dom_alloc(bytes.length);
  mem().set(bytes, p);
  return fn(id, x, y, p, bytes.length, !!local);
}
function dragPacket(items) {
  const parts = items.map(clipPacket);
  const n = 12 + parts.reduce((n, b) => n + b.length, 0);
  if (!items.length || items.length > 256 || n > dragLimit) throw new RangeError('Transfer too large');
  const out = new Uint8Array(n); out.set([68,65,89,68,78,68,0,1]);
  new DataView(out.buffer).setUint32(8, items.length, true);
  let at = 12; for (const p of parts) { out.set(p,at); at += p.length; }
  return out;
}
function dragRepresentations(packet) {
  const v = new DataView(packet.buffer,packet.byteOffset,packet.byteLength);
  let at=12; const items=[];
  for (let i=0;i<v.getUint32(8,true);i++) {
    const count=v.getUint32(at,true); at+=4; const reps=[];
    for(let j=0;j<count;j++) {
      const m=v.getUint32(at,true), n=v.getUint32(at+4,true); at+=8;
      const mime=utf8.decode(packet.subarray(at,at+m)); at+=m;
      reps.push({mime,bytes:packet.slice(at,at+n)}); at+=n;
    }
    items.push(reps);
  }
  return items;
}
function dragBase64(bytes) {
  let s=''; for(let i=0;i<bytes.length;i+=16384) s+=String.fromCharCode(...bytes.subarray(i,i+16384));
  return btoa(s);
}
function installDragSource(id) {
  const el=E(id); el.draggable=true; takesPointer(el);
  el.addEventListener('dragstart',e=>{
    // Nested registered sources own their own gesture.
    if(e.target.closest('[draggable="true"]')!==el) return;
    dragOffer=null; const r=el.getBoundingClientRect();
    wasm.day_dom_drag_prepare(id,e.clientX-r.left,e.clientY-r.top);
    const packet=dragOffer; dragOffer=null;
    if(!packet || !e.dataTransfer) { e.preventDefault(); return; }
    try {
      e.dataTransfer.effectAllowed='copy';
      e.dataTransfer.setData(dragBundle,dragBase64(packet));
      for(const rep of dragRepresentations(packet)[0]) {
        // HTML custom formats carry strings. Preserve arbitrary bytes through the bundle;
        // expose their MIME names during protected hover without decoding them as text.
        e.dataTransfer.setData(rep.mime,rep.mime.startsWith('text/') ? utf8.decode(rep.bytes) : dragBase64(rep.bytes));
      }
      localDrag={id}; e.stopPropagation();
    } catch { localDrag=null; e.preventDefault(); }
  });
  el.addEventListener('dragend',()=>{ localDrag=null; });
  const hooks=releaseHooks.get(id)??[]; hooks.push(()=>{if(localDrag?.id===id)localDrag=null;}); releaseHooks.set(id,hooks);
}
// Drop targets with a hover under way, so the end of a drag session anywhere ends them.
const dropHovers=new Set();
document.addEventListener('dragend',()=>{ for(const end of [...dropHovers]) end(); },true);
document.addEventListener('drop',()=>{ queueMicrotask(()=>{ for(const end of [...dropHovers]) end(); }); },true);
function installDropTarget(id) {
  const el=E(id);
  takesPointer(el);
  // Enter/leave pairing (docs/drag-and-drop.md "Accepting drops"): the browser sends a
  // dragenter and a dragleave for every child the drag crosses, the new one's enter before the
  // old one's leave, so the hover ends when the count of entered elements falls back to zero.
  // Counted in the capture phase, ahead of any nested target that stops the event.
  let depth=0;
  const end=()=>{ if(!depth) return; depth=0; dropHovers.delete(end); wasm.day_dom_drag_leave(id); };
  el.addEventListener('dragenter',()=>{ depth+=1; dropHovers.add(end); },true);
  el.addEventListener('dragleave',()=>{ if(depth && --depth===0) { depth=1; end(); } },true);
  const proposal=e=>{
    const r=el.getBoundingClientRect(), dt=e.dataTransfer;
    if(!dt) return false;
    const types=new Set(dt.types);
    for(const item of dt.items) if(item.type) types.add(item.type);
    return transferCall(wasm.day_dom_drag_accept,id,e.clientX-r.left,e.clientY-r.top,utf8enc.encode([...types].join('\n')),localDrag);
  };
  for(const name of ['dragenter','dragover']) el.addEventListener(name,e=>{
    if(proposal(e)) { e.preventDefault(); e.stopPropagation(); e.dataTransfer.dropEffect='copy'; }
  });
  el.addEventListener('drop',e=>{
    // The drop ends the hover (the target's own `deliver` does that for an accepted one).
    depth=0; dropHovers.delete(end);
    if(!proposal(e)) { wasm.day_dom_drag_leave(id); return; }
    e.preventDefault(); e.stopPropagation();
    const r=el.getBoundingClientRect(), x=e.clientX-r.left,y=e.clientY-r.top, local=!!localDrag;
    // Capture every File and initiate every string read before returning from drop.
    const dt=e.dataTransfer, encoded=dt.getData(dragBundle);
    const files=Array.from(dt.files);
    // Read synchronously while the drop is live: `getAsString`'s callback is not delivered at
    // all for some drag sessions (Chromium under an automation driver), and the data store is
    // readable only during this handler anyway.
    const strings=Array.from(dt.items).filter(i=>i.kind==='string').map(i=>({mime:i.type,bytes:utf8enc.encode(dt.getData(i.type))}));
    const expires=performance.now()+30000;
    (async()=>{
      let packet;
      if(encoded) {
        if(encoded.length>Math.ceil(dragLimit/3)*4) return;
        // Native apps may publish the same MIME as binary rather than HTML base64.
        // Keep a standard File fallback available instead of discarding a valid image.
        try {
          const candidate=Uint8Array.from(atob(encoded),c=>c.charCodeAt(0));
          if(candidate.length>=12 && [68,65,89,68,78,68,0,1].every((b,i)=>candidate[i]===b)) packet=candidate;
        } catch { /* Try the standard File representations. */ }
      }
      if(!packet && files.length) {
        if(files.length>256 || files.reduce((n,f)=>n+f.size,0)>dragLimit) return;
        packet=dragPacket(await Promise.all(files.map(async f=>[{mime:f.type||'application/octet-stream',bytes:new Uint8Array(await f.arrayBuffer())}])));
      } else if(!packet) {
        const reps=strings.filter(r=>r.mime.includes('/')&&r.mime!==dragBundle);
        if(!reps.length||reps.length>32) return;
        packet=dragPacket([reps]);
      }
      // Disposed targets cannot receive a late File read.
      if(performance.now()<expires && els[id]===el && el.isConnected && packet.length<=dragLimit)
        transferCall(wasm.day_dom_drag_receive,id,x,y,packet,local);
    })().catch(error=>console.warn('day: drop read failed',error));
  });
}
