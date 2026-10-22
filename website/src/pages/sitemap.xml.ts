// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { routes, hubs, fixedRoutes, isPublished } from '../lib/content-routes.mjs';
import gallery from '../data/gallery-manifest.json';
import type { APIRoute } from 'astro';

export const GET: APIRoute = ({ site }) => {
  const paths = [...Object.values(routes).filter(isPublished).map((p) => p.route), ...hubs.map((p) => p.route),
    ...fixedRoutes, ...gallery.apps.map((app) => `/gallery/${app.id}`)];
  const escape = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/"/g, '&quot;');
  const items = [...new Set(paths)].sort().map((path) => `<url><loc>${escape(new URL(path, site).href)}</loc></url>`);
  return new Response(`<?xml version="1.0" encoding="UTF-8"?>\n<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">\n${items.join('\n')}\n</urlset>\n`, {
    headers: { 'Content-Type': 'application/xml' },
  });
};
