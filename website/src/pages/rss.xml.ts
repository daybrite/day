// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import rss from '@astrojs/rss';
import type { APIRoute } from 'astro';
import { blogPosts, postPath } from '../lib/blog';
import { url } from '../lib/site';
import { res } from '../generated/strings.mjs';

export const GET: APIRoute = async ({ site }) => rss({
  title: res.str.blog_title(),
  description: res.str.blog_description(),
  site: new URL(url('blog'), site!).href,
  items: (await blogPosts()).map((post) => ({
    title: post.data.title,
    description: post.data.description,
    pubDate: post.data.pubDate,
    link: new URL(url(postPath(post.id)), site!).href,
  })),
  customData: '<language>en</language>',
});
