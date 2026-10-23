// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { unlinkedReferences } from './api-reference-coverage.mjs';
const names = new Set(['on_submit', 'text_area', 'get', 'day_cancel']);

test('unlinked calls, methods, qualified references, and nested calls require owner review', async () => {
  const refs = await unlinkedReferences('`text_area(s)`; `.on_submit(f)`; `.on_submit`; `Search::on_submit`; `.on_submit(|| s.get())`', names);
  assert.equal(refs.length, 5);
  assert.deepEqual(refs.at(-1).functions, ['on_submit', 'get']);
  assert.ok(refs.every(r => !('url' in r)), 'ambiguous names must never auto-select an owner');
});
test('links, reference links and fenced samples are excluded, generated app accessors are not Day APIs', async () => {
  const refs = await unlinkedReferences('[`.on_submit(f)`](/api/day_pieces/trait.TextAreaBuilder.html#tymethod.on_submit)\n\n[`Search::on_submit`][submit]\n\n[submit]: /api/day_pieces/struct.Search.html#method.on_submit\n\n```rust\ntext_area(s).on_submit(f)\n```\n\n`res::str::get()`; `res::str::day_cancel()`', names);
  assert.deepEqual(refs.map(r => r.code), ['res::str::day_cancel()']);
});
test('diagnostic lines include front matter; ordinary words and fields are not inferred as calls', async () => {
  const refs = await unlinkedReferences('---\ntitle: Test\n---\n\n`on_submit` is a name; `.on_submit(f)` is a call.', names);
  assert.equal(refs.length, 1);
  assert.equal(refs[0].line, 5);
});

test('slash-separated modifier lists require links too', async () => {
  const refs = await unlinkedReferences('`.on_submit/.get`', names);
  assert.deepEqual(refs[0].functions, ['on_submit', 'get']);
});

test('generic function calls retain their callable name', async () => {
  const refs = await unlinkedReferences('`environment::<Vec<T>>()`', new Set(['environment']));
  assert.deepEqual(refs[0].functions, ['environment']);
});
