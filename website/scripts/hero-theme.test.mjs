// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import test from 'node:test';
import assert from 'node:assert/strict';
import { heroCapture } from '../src/lib/hero-theme.mjs';

test('light-only captures never appear in the dark rotation', () => {
  // Synthetic image metadata; no screenshot files are needed for selection checks.
  const lightOnly = { src: 'light.webp', width: 1000, height: 658 };
  assert.deepEqual(heroCapture(lightOnly, 'light'), { src: 'light.webp', width: 1000, height: 658 });
  assert.equal(heroCapture(lightOnly, 'dark'), null);
});

test('theme changes use the actual dimensions of each capture', () => {
  const shot = { src: 'light.webp', width: 1000, height: 658,
    srcDark: 'dark.webp', widthDark: 1000, heightDark: 661 };
  assert.deepEqual(heroCapture(shot, 'dark'), { src: 'dark.webp', width: 1000, height: 661 });
  assert.equal(heroCapture(shot, 'light').height, 658);
});


test('reviewed dark-only selections stay out of light mode', () => {
  const shot = { srcDark: 'dark-only.webp', widthDark: 460, heightDark: 1000 };
  assert.equal(heroCapture(shot, 'light'), null);
  assert.deepEqual(heroCapture(shot, 'dark'), { src: 'dark-only.webp', width: 460, height: 1000 });
});
