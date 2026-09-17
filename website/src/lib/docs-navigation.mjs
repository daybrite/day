// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
export const groups = [
  { name: 'Learn', description: 'Install Day, run your first app, and learn how its UI and state work.' },
  { name: 'Build', description: 'Add app features, test them, and prepare a release.' },
  { name: 'Reference', description: 'Look up commands, platform requirements, APIs, and terminology.' },
  { name: 'Contribute', description: 'Understand the framework and develop pieces, parts, and backends.' },
];

export function groupOf(page) {
  if (page.id === 'troubleshooting') return 'Build';
  if (['local-development', 'architecture', 'rendering'].includes(page.id) || page.data.section === 'Extend') return 'Contribute';
  if (['system-requirements', 'cli', 'platforms', 'project-structure'].includes(page.id) || ['Platforms', 'Reference'].includes(page.data.section)) return 'Reference';
  if (['Start here', 'Coming from', 'Concepts'].includes(page.data.section)) return 'Learn';
  return 'Build';
}

// Keep the first-app reading path together; migration guides are optional further reading.
const learningPath = ['overview', 'benefits', 'getting-started', 'pieces', 'reactivity', 'layout', 'styling', 'api-tour'];

export function navigation(docs) {
  const sorted = [...docs].sort((a, b) => a.data.order - b.data.order);
  return groups.map((group) => ({ ...group, pages: sorted.filter((page) => groupOf(page) === group.name).sort((a, b) => {
    if (group.name !== 'Learn') return 0;
    const rank = (page) => { const index = learningPath.indexOf(page.id); return index < 0 ? learningPath.length : index; };
    return rank(a) - rank(b);
  }) }));
}
