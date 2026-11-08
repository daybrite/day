// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { getCollection } from 'astro:content';
import { isDraftPost, newestPostsFirst } from './blog-policy.mjs';

export { isDraftPost };

// RSS and sitemap always use the default. Only local HTML previews opt into drafts.
export async function blogPosts(includeDrafts = false) {
  return (await getCollection('blog', (post) => !isDraftPost(post) ||
    (includeDrafts && import.meta.env.DEV))).sort(newestPostsFirst);
}

export function postPath(id: string) {
  return `blog/${id.split('/').map(encodeURIComponent).join('/')}`;
}

export function postDate(date: Date) {
  return new Intl.DateTimeFormat('en', { dateStyle: 'long', timeZone: 'UTC' }).format(date);
}
