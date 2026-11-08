// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import selection from '../data/llm-docs.json' with { type: 'json' };
import { routes, canonicalPath } from './content-routes.mjs';
export { selection };
export const markdownRoute = (route) => `${route}.md`;
export function hasLlmGuide(path) {
  return selection.guides.some(({ key }) => routes[key]?.route === canonicalPath(path));
}
