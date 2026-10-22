// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { getCollection } from 'astro:content';
import { metadata, isPublished } from './content-routes.mjs';
import { internalTitle, internalExcerpt } from './site';

export async function contentPages() {
  const collections = await Promise.all(['docs', 'internal', 'repository'].map(async (collection) => {
    const entries = await getCollection(collection as 'docs' | 'internal' | 'repository');
    return entries.filter((entry) => isPublished(metadata(collection, entry.id))).map((entry) => ({
      ...metadata(collection, entry.id), entry, collection,
      title: entry.data.title ?? entry.body?.match(/^# (.+)$/m)?.[1].replace(/`/g, '') ?? internalTitle(entry.id),
      description: entry.data.description ?? internalExcerpt(entry.body ?? ''),
      order: entry.data.order ?? 99,
      navTitle: 'navTitle' in entry.data ? entry.data.navTitle : undefined,
    }));
  }));
  return collections.flat();
}
