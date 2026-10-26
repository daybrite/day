// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import routes from '../data/content-routes.json' with { type: 'json' };
import migrations from '../data/route-migrations.json' with { type: 'json' };
import referenceGroups from '../data/reference-navigation.json' with { type: 'json' };

export { routes };
export const isPublished = (page) => page.published !== false;
const unpublished = new Set(Object.values(routes).filter((p) => !isPublished(p)).map((p) => p.route));
export function isUnpublishedPath(href) {
  const path = canonicalPath(href).split(/[?#]/)[0].replace(/\/$/, '');
  return path === '/contributing/design' ||
    path === '/contributing/backend-contracts' || path.startsWith('/contributing/backend-contracts/') ||
    unpublished.has(path);
}

export function metadata(collection, id) {
  const result = routes[`${collection}/${id}`];
  if (!result) throw new Error(`Unplaced content: ${collection}/${id}; add content-routes.json metadata`);
  return result;
}

/** Resolve source-era links to canonical routes without publishing duplicate pages. */
export function canonicalPath(href) {
  const local = href.replace(/^https:\/\/daybrite\.dev(?=\/)/, '');
  if (!local.startsWith('/')) return href;
  const match = /^([^?#]*)(.*)$/.exec(local);
  const path = match[1].replace(/\/$/, '') || '/';
  const route = migrations[path] ?? path;
  return route + match[2];
}

export const hubs = [
  { route: '/docs', title: 'Start here', description: 'Choose a learning path, task guide, reference, or explanation.', audience: 'app developer' },
  { route: '/docs/tutorials', title: 'Tutorials', description: 'Build a Session counter on macOS, Windows, or Linux in six lessons, from your first window to events, state, layout, navigation, and a tested app.', audience: 'app developer' },
  { route: '/docs/how-to', title: 'How-to guides', description: 'Find a procedure for a specific app development task.', audience: 'app developer' },
  { route: '/docs/explanation', title: 'Explanation', description: 'Understand pieces, signals, layout, state, and lifecycle.', audience: 'app developer' },
  { route: '/contributing', title: 'Contributing', description: 'Set up framework development, understand the architecture, and build extensions.', audience: 'contributor' },
  { route: '/contributing/history', title: 'History', description: 'Retained migration and implementation history.', audience: 'contributor' },
  { route: '/contributing/tutorials', title: 'Contributor tutorials', description: 'Learn to create composite pieces, native pieces, and parts.', audience: 'contributor' },
  { route: '/contributing/documentation', title: 'Documentation authoring', description: 'The canonical style guide, terminology, authoring workflow, and validation commands.', audience: 'contributor' },
  { route: '/platforms', title: 'Platforms', description: 'Choose desktop and mobile targets by their support level, toolkit, and feature limits. Start with supported targets and inspect the full compatibility reference.', audience: 'evaluator' },
  { route: '/community', title: 'Community', description: 'Help make Day useful to more app creators: share a reproducible issue, improve a guide, or contribute a tested native integration.', audience: 'evaluator' },
];

export const fixedRoutes = ['/', '/gallery', '/showcase', '/privacy', '/examples', '/docs/reference/api'];
export const groups = [
  { name: 'Tutorials', route: '/docs/tutorials', audience: 'app developer', sequential: true },
  { name: 'How-to guides', route: '/docs/how-to', audience: 'app developer' },
  { name: 'Reference', route: '/docs/reference', audience: 'app developer' },
  { name: 'Explanation', route: '/docs/explanation', audience: 'app developer' },
  { name: 'Framework development', route: '/contributing', audience: 'contributor', direct: true },
  { name: 'Contributor tutorials', route: '/contributing/tutorials', audience: 'contributor', sequential: true },
  { name: 'Documentation', route: '/contributing/documentation', audience: 'contributor' },
  { name: 'History', route: '/contributing/history', audience: 'contributor' },
];

export function belongsTo(page, group) {
  return page.route.startsWith(group.route + '/') &&
    (!group.direct || !page.route.slice(group.route.length + 1).includes('/'));
}

const byNavigationTitle = (a, b) => (a.navTitle ?? a.title ?? a.route)
  .localeCompare(b.navTitle ?? b.title ?? b.route, 'en', { sensitivity: 'base' });

/** One placement per reference page; sort each group by its visible sidebar labels. */
export function referenceNavigation(pages) {
  const byRoute = new Map(pages.filter(isPublished).map((p) => [p.route, p]));
  const assigned = new Set();
  const sections = referenceGroups.map((group) => ({
    ...group,
    pages: group.pages.flatMap((slug) => {
      const route = `/docs/reference/${slug}`;
      if (assigned.has(route)) throw new Error(`Duplicate reference placement: ${route}`);
      assigned.add(route);
      return byRoute.has(route) ? [byRoute.get(route)] : [];
    }).sort(byNavigationTitle),
  }));
  for (const page of byRoute.values()) {
    if (page.route.startsWith('/docs/reference/') && !assigned.has(page.route)) {
      throw new Error(`Uncategorized reference: ${page.route}; add reference-navigation.json placement`);
    }
  }
  return sections.filter((group) => group.pages.length);
}

export function navigation(pages, audience) {
  return groups.filter((g) => g.audience === audience).map((group) => {
    const members = pages.filter((p) => isPublished(p) && belongsTo(p, group));
    const sections = group.route === '/docs/reference' ? referenceNavigation(members) : [];
    return {
      ...group, sections,
      pages: sections.length ? sections.flatMap((s) => s.pages) :
        members.sort((a, b) => (group.sequential ? a.order - b.order : 0) || byNavigationTitle(a, b)),
    };
  });
}
