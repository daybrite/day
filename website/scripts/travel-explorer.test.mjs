import test from 'node:test';
import assert from 'node:assert/strict';
import { travelCaptures, travelTargets, travelLocales } from '../src/lib/travel-explorer.mjs';

const picture = { src: '/synthetic.png', width: 360, height: 720 };
const row = (id, column, captures) => ({ id, byColumn: [{ column, captures }] });
test('phone pairs never substitute another language or theme for a missing position', () => {
  const app = { shots: [row('top', 'ios-uikit', { 'light|en': picture, 'dark|en': picture }), row('bottom', 'ios-uikit', { 'dark|en': picture, 'light|fr': picture })] };
  const captures = travelCaptures(app);
  assert.equal(captures.length, 1);
  assert.equal(captures[0].theme, 'dark');
  assert.equal(captures[0].locale, 'en');
  assert.deepEqual(captures[0].images.map(i => i.position), ['top', 'bottom']);
});
test('desktop uses overview and excludes the retired and development toolkit targets', () => {
  const app = { shots: [{ id: 'overview', byColumn: ['macos-appkit', 'macos-gtk', 'macos-qt', 'windows-xaml'].map(column => ({ column, captures: { 'light|en': picture } })) }] };
  assert.deepEqual(travelCaptures(app).map(c => c.target), ['macos-appkit']);
  assert.deepEqual(travelTargets.map(t => t[0]), ['ios-uikit','android-material','harmony-arkui','macos-appkit','windows-winui','linux-gtk','linux-qt','web-dom']);
  assert.equal(travelLocales.at(-1)[0], 'ar');
});
test('missing data does not fabricate a screenshot', () => {
  assert.deepEqual(travelCaptures(undefined), []);
  assert.deepEqual(travelCaptures({ shots: [row('overview', 'macos-appkit', { 'light|en': { ...picture, width: 0 } })] }), []);
});
test('city releases use Paris for the exact locale/theme on phones and desktop', () => {
  const app = { shots: [{ id: 'paris', label: 'Paris', byColumn: [
    { column: 'ios-uikit', captures: { 'dark|fr': picture } },
    { column: 'macos-appkit', captures: { 'light|en': picture } },
  ] }, row('top', 'ios-uikit', { 'light|en': picture })] };
  const captures = travelCaptures(app);
  assert.equal(captures.length, 2);
  assert.deepEqual(captures.map(c => [c.target, c.locale, c.theme]), [['ios-uikit', 'fr', 'dark'], ['macos-appkit', 'en', 'light']]);
  assert.ok(captures.every(c => c.images.length === 1 && c.images[0].position === 'paris'));
});
