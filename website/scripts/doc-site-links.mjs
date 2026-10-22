// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

/** Check public documentation URLs in CLI help, scaffold templates, and source comments. */
export function checkDocSiteLinks(text, routes) {
  const failures = [];
  for (const [index, line] of text.split('\n').entries()) {
    for (const match of line.matchAll(/https:\/\/daybrite\.dev(\/docs(?:\/[A-Za-z0-9_-]+)*\/?)/g)) {
      const route = match[1].replace(/\/$/, '');
      if (!routes.has(route)) failures.push({ line: index + 1, route });
    }
  }
  return failures;
}
