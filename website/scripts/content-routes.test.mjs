// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import test from 'node:test';
import { checkDocSiteLinks } from './doc-site-links.mjs';
import assert from 'node:assert/strict';
import { canonicalPath, metadata, navigation, referenceNavigation, routes, hubs, isPublished, isUnpublishedPath } from '../src/lib/content-routes.mjs';
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
  assert.equal(resolveContentLink('design/badge.md', 'docs/badge.md'), null);
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
  assert.deepEqual(navigation(pages, 'contributor').flatMap((g) => g.pages), []);
});

test('reference navigation covers every published page once and groups related tasks', () => {
  const pages = Object.values(routes).filter(isPublished);
  const groups = referenceNavigation(pages);
  const actual = groups.flatMap((group) => group.pages.map((p) => p.route));
  assert.equal(actual.length, new Set(actual).size);
  assert.deepEqual(new Set(actual), new Set(pages.filter((p) => p.route.startsWith('/docs/reference/')).map((p) => p.route)));
  const groupOf = (slug) => groups.find((g) => g.pages.some((p) => p.route === `/docs/reference/${slug}`))?.id;
  assert.equal(groupOf('buttons'), groupOf('grid'));
  assert.equal(groupOf('clipboard'), groupOf('logging'));
  assert.equal(groupOf('haptics'), groupOf('location'));
  assert.notEqual(groupOf('commands'), groupOf('buttons'));
  assert.equal(groupOf('platforms/harmony-arkui'), 'platforms');
  assert.throws(() => referenceNavigation([{ route: '/docs/reference/synthetic-new-page' }]), /Uncategorized reference/);
});

test('design records stay in the source catalog but cannot become public links or navigation', () => {
  assert.equal(metadata('internal', 'design/badge').published, false);
  assert.equal(hubs.some((p) => p.route === '/contributing/design'), false);
  for (const href of ['/contributing/design', '/contributing/design/badge#history', 'https://daybrite.dev/contributing/design/badge']) {
    assert.equal(isUnpublishedPath(href), true);
    assert.equal(resolveContentLink(href, 'docs/badge.md'), null);
  }
  assert.equal(isUnpublishedPath('https://example.org/contributing/design/badge'), false);
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

test('backend contracts are unpublished and retired URLs are not migrated', () => {
  const retired = ['android-jni', 'api-style', 'bridge', 'coverage-matrix', 'duty-matrix',
    'element-ids', 'extending', 'focus', 'grid', 'native-project-migration', 'recorder-matrix', 'testing'];
  const pages = Object.values(routes).filter(isPublished);
  assert.equal([...pages, ...hubs].some((p) => p.route.startsWith('/contributing/backend-contracts')), false);
  assert.equal(navigation(pages, 'contributor').some((g) => g.route === '/contributing/backend-contracts'), false);
  for (const slug of ['', ...retired.map((s) => '/' + s)]) {
    const path = '/contributing/backend-contracts' + slug;
    assert.equal(canonicalPath(path + '?from=old#details'), path + '?from=old#details');
    assert.equal(isUnpublishedPath(path), true);
    assert.equal(resolveContentLink(path, 'docs/focus.md'), null);
  }
  for (const slug of retired.filter((s) => !['focus', 'grid'].includes(s))) {
    assert.equal(canonicalPath('/docs/internal/' + slug), '/docs/internal/' + slug);
  }
  assert.equal(resolveContentLink('bridge.md#streams', 'docs/http.md'),
    'https://github.com/daybrite/day/blob/main/docs/bridge.md#streams');
  assert.equal(resolveContentLink('backend-contracts/focus.md', 'docs/focus.md'),
    'https://github.com/daybrite/day/blob/main/docs/backend-contracts/focus.md');
  for (const [source, route] of [
    ['element-ids', '/docs/reference/element-ids'],
    ['testing', '/docs/how-to/testing'],
    ['native-project-migration', '/docs/how-to/native-projects'],
  ]) {
    assert.equal(metadata('internal', source).route, route);
    assert.equal(isPublished(metadata('internal', source)), true);
    assert.equal(resolveContentLink(source + '.md', 'docs/buttons.md'), route);
  }
});
