#!/usr/bin/env node
// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// The shipped shim's list reader watch (docs/list.md "Following the end", Cap::ListReaderScroll),
// over a fake scroller and clock: the reader's wheel, keys, scroll bar and touch are reported,
// Day's own scrolls never are, and input that lands on Day's smooth scroll stops it.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = fs.readFileSync(new URL('../../crates/day-cli/resources/web/shim.js', import.meta.url), 'utf8');

class FakeNode {
  constructor(parent = null, tag = 'div') { this.parent = parent; this.tag = tag; this.listeners = {}; }
  addEventListener(type, f) { (this.listeners[type] ??= []).push(f); }
  removeEventListener(type, f) { this.listeners[type] = (this.listeners[type] ?? []).filter(g => g !== f); }
  fire(type, init = {}) { for (const f of [...(this.listeners[type] ?? [])]) f({ target: this, ...init }); }
  closest(selector) {
    for (let n = this; n; n = n.parent) {
      if (selector.split(',').some(s => s === n.tag || (s === '.day-list' && n.isList))) return n;
    }
    return null;
  }
  contains(t) { for (let n = t; n; n = n.parent) if (n === this) return true; return false; }
}

class FakeScroller extends FakeNode {
  constructor() {
    super();
    this.isList = true;
    this.scrollTop = 0; this.scrollHeight = 4000; this.clientHeight = 300; this.calls = [];
  }
  scrollTo({ top, behavior }) {
    this.calls.push(['scrollTo', top, behavior]);
    if (behavior === 'instant') this.scrollTop = Math.max(0, Math.min(top, this.scrollHeight - this.clientHeight));
  }
  scrollBy({ top }) { this.calls.push(['scrollBy', top]); this.scrollTop += top; }
  // The browser moves the content, then tells the page.
  moveTo(top) { this.scrollTop = top; this.fire('scroll'); }
}

function boot() {
  let clock = 1000;
  const doc = new FakeNode(null, 'document');
  doc.body = new FakeNode(doc, 'body');
  doc.documentElement = doc;
  const win = new FakeNode(null, 'window');
  const readers = [];
  const context = vm.createContext({
    TextEncoder, TextDecoder, Blob, URLSearchParams, location: new URL('https://fixture.invalid/'),
    document: doc, ResizeObserver: class {}, Element: class {},
    performance: { now: () => clock },
    addEventListener: (type, f) => win.addEventListener(type, f),
    removeEventListener: (type, f) => win.removeEventListener(type, f),
  });
  vm.runInContext(source.replace('export async function start', 'async function start') + `
    wasm = { memory: { buffer: new ArrayBuffer(65536) },
      day_dom_list_reader: (id) => globalThis.readers.push(id), day_dom_list_scrolled() {} };
    globalThis.attach = (el) => { els.push(el); const id = els.length - 1; env.day_dom_list_on_scroll(id); return id; };
    globalThis.env = env;
  `, Object.assign(context, { readers }));
  const el = new FakeScroller();
  el.parent = doc.body;
  const id = context.attach(el);
  return { context, doc, win, el, id, readers, tick: (ms) => { clock += ms; } };
}

test("Day's own scrolls are never the reader's", () => {
  const { context, el, id, readers, tick } = boot();
  context.env.day_dom_scroll_to(id, 0, 1200, 1);
  for (const top of [300, 700, 1100, 1200]) { tick(16); el.moveTo(top); }
  context.env.day_dom_scroll_edge(id, 1, 0);
  tick(16); el.moveTo(3700);
  assert.deepEqual(readers, []);
});

test('a wheel that moves the list reports once; one that moves nothing reports nothing', () => {
  const { el, id, readers, tick } = boot();
  el.scrollTop = 1000;
  el.fire('wheel', { deltaY: -120, deltaMode: 0 });
  tick(10); el.moveTo(900);
  tick(16); el.moveTo(880);
  assert.deepEqual(readers, [id]);
  // At the end, a wheel down moves nothing: no scroll, no report, and a later scroll of Day's
  // is not taken for it.
  el.fire('wheel', { deltaY: 120, deltaMode: 0 });
  tick(400); el.moveTo(1000);
  assert.deepEqual(readers, [id]);
  // Ctrl+wheel zooms; a horizontal wheel moves no rows.
  el.fire('wheel', { deltaY: -120, deltaMode: 0, ctrlKey: true });
  el.fire('wheel', { deltaY: 0, deltaX: 40, deltaMode: 0 });
  tick(10); el.moveTo(990);
  assert.deepEqual(readers, [id]);
});

test("input that lands on Day's smooth scroll stops it and is the reader's", () => {
  const { context, el, id, readers, tick } = boot();
  el.scrollTop = 1000;
  context.env.day_dom_scroll_to(id, 0, 1300, 1);
  tick(16); el.moveTo(1100);
  el.calls.length = 0;
  el.fire('wheel', { deltaY: -3, deltaMode: 1 });
  // Stopped where it was, then the wheel's own step from there (three lines).
  assert.deepEqual(el.calls, [['scrollTo', 1100, 'instant'], ['scrollBy', -120]]);
  assert.deepEqual(readers, [id]);
  tick(10); el.moveTo(980);
  assert.deepEqual(readers, [id], 'one report per input');
  // The glide is over: the next wheel scrolls natively, with no stop.
  el.calls.length = 0;
  tick(200);
  el.fire('wheel', { deltaY: -120, deltaMode: 0 });
  assert.deepEqual(el.calls, []);
});

test('the scroll bar, a finger and the scrolling keys are the reader', () => {
  const { context, doc, win, el, id, readers, tick } = boot();
  el.scrollTop = 1000;
  // A press on the bar lands on the scroller itself; the drag scrolls until the release.
  el.fire('pointerdown', { pointerType: 'mouse' });
  tick(50); el.moveTo(950);
  assert.equal(readers.length, 1);
  win.fire('pointerup');
  tick(500); el.moveTo(1000);
  assert.equal(readers.length, 1, 'after the release, scrolls are Day\'s again');
  // A press on a row is no scroll bar.
  const row = new FakeNode(el);
  el.fire('pointerdown', { target: row, pointerType: 'mouse' });
  tick(500); el.moveTo(1010);
  assert.equal(readers.length, 1);
  // A finger.
  el.fire('touchstart');
  tick(30); el.moveTo(900);
  el.fire('touchend');
  tick(500); el.moveTo(1000);
  assert.equal(readers.length, 2);
  // PageUp on the page after a press in the list: the browser scrolls the list.
  doc.fire('pointerdown', { target: row });
  doc.fire('keydown', { target: doc.body, key: 'PageUp' });
  tick(20); el.moveTo(720);
  assert.equal(readers.length, 3);
  // Space in a row's button presses it, and keys in a field stay there.
  const button = new FakeNode(row, 'button');
  const field = new FakeNode(row, 'input');
  tick(500);
  doc.fire('keydown', { target: button, key: ' ' });
  doc.fire('keydown', { target: field, key: 'ArrowUp' });
  tick(20); el.moveTo(700);
  assert.equal(readers.length, 3);
  // An arrow key on the list's own focus.
  tick(500);
  doc.fire('keydown', { target: el, key: 'ArrowUp' });
  tick(20); el.moveTo(660);
  assert.deepEqual(readers, [id, id, id, id]);
  // A key on Day's smooth scroll stops it at once.
  context.env.day_dom_scroll_to(id, 0, 1300, 1);
  tick(16); el.moveTo(800);
  el.calls.length = 0;
  tick(500);
  doc.fire('keydown', { target: el, key: 'PageDown' });
  assert.deepEqual(el.calls, [['scrollTo', 800, 'instant']]);
  assert.equal(readers.length, 5);
});
