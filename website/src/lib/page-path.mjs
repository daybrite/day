// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { canonicalPath } from './content-routes.mjs';

// GitHub Pages serves directory indexes with a trailing slash. Link there directly
// instead of paying a 301 round trip on every navigation; assets retain their names.
export function pagePath(href) {
  const canonical = canonicalPath(href);
  if (!canonical.startsWith('/') || canonical.startsWith('//')) return canonical;
  const [, path, suffix] = /^([^?#]*)(.*)$/.exec(canonical);
  const file = path.split('/').at(-1);
  return path + (path.endsWith('/') || file.includes('.') ? '' : '/') + suffix;
}
