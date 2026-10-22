// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import test from 'node:test';
import { checkDocSiteLinks } from './doc-site-links.mjs';
import assert from 'node:assert/strict';
import { canonicalPath, metadata, navigation } from '../src/lib/content-routes.mjs';
import { resolveContentLink } from '../plugins/content-links.mjs';

test('migrated links retain query strings and fragments, and leave other hosts alone', () => {
  assert.equal(canonicalPath('/docs/platforms?target=mac#support-tiers'), '/docs/reference/platforms?target=mac#support-tiers');
  assert.equal(canonicalPath('https://daybrite.dev/docs/internal/color'), '/docs/reference/color');
  assert.equal(canonicalPath('https://example.org/docs/platforms'), 'https://example.org/docs/platforms');
  assert.equal(canonicalPath('/gallery/Day-Showcase/'), '/gallery/Day-Showcase');
  assert.equal(canonicalPath('/showcase/'), '/showcase');
  assert.equal(canonicalPath('/api/day/'), '/api/day');
});

test('repository links resolve from the canonical source, including nested splits and standards', () => {
  assert.equal(resolveContentLink('../badge.md', 'docs/design/badge.md'), '/docs/reference/badge');
  assert.equal(resolveContentLink('design/badge.md', 'docs/badge.md'), '/contributing/design/badge');
  assert.equal(resolveContentLink('style-guide.md', 'website/authoring/authoring.md'), '/contributing/documentation/style-guide');
  assert.equal(resolveContentLink('../crates/day-spec/src/lib.rs', 'docs/badge.md'), 'https://github.com/daybrite/day/blob/main/crates/day-spec/src/lib.rs');
  assert.equal(resolveContentLink('./LICENSE', 'CONTRIBUTING.md'), 'https://github.com/daybrite/day/blob/main/LICENSE');
  assert.equal(resolveContentLink('#local-heading', 'docs/badge.md'), '#local-heading');
});

test('unplaced content fails and audience navigation keeps proposals out of app reference', () => {
  assert.throws(() => metadata('internal', 'synthetic-missing-fixture'), /Unplaced content/);
  const pages = ['internal/badge', 'internal/design/badge'].map((key) => {
    const [collection, ...id] = key.split('/');
    return { ...metadata(collection, id.join('/')), title: key, order: 0 };
  });
  assert.deepEqual(navigation(pages, 'app developer').flatMap((g) => g.pages).map((p) => p.route), ['/docs/reference/badge']);
  assert.deepEqual(navigation(pages, 'contributor').flatMap((g) => g.pages).map((p) => p.route), ['/contributing/design/badge']);
});

// Synthetic source fixtures cover prose, Rust strings, and scaffold comments.
test('CLI documentation links require current routes, including URLs with fragments', () => {
  const routes = new Set(['/docs', '/docs/reference/cli']);
  assert.deepEqual(checkDocSiteLinks('"https://daybrite.dev/docs/reference/cli/#build"\nhttps://daybrite.dev/docs', routes), []);
  assert.deepEqual(checkDocSiteLinks('// https://daybrite.dev/docs/cli/\n// https://daybrite.dev/docs/missing#topic', routes), [
    { line: 1, route: '/docs/cli' }, { line: 2, route: '/docs/missing' },
  ]);
  assert.deepEqual(checkDocSiteLinks('https://example.invalid/docs/cli', routes), []);
});
