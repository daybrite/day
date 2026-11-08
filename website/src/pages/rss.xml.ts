// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import rss from '@astrojs/rss';
import type { APIRoute } from 'astro';
import { render } from 'astro:content';
import { experimental_AstroContainer } from 'astro/container';
import { loadRenderers } from 'astro:container';
import { getContainerRenderer } from '@astrojs/mdx/container-renderer';
import { blogPosts, postPath } from '../lib/blog';
import { feedContent } from '../lib/blog-feed.mjs';
import { url } from '../lib/site';
import { res } from '../generated/strings.mjs';

export const GET: APIRoute = async ({ site }) => {
  const container = await experimental_AstroContainer.create({
    renderers: await loadRenderers([getContainerRenderer()]),
  });
  const items = [];
  for (const post of await blogPosts()) {
    // Keep the trailing slash already used by RSS permalinks and GUIDs.
    const link = new URL(`${url(postPath(post.id))}/`, site!).href;
    const { Content } = await render(post);
    const html = await container.renderToString(Content, { request: new Request(link) });
    items.push({
      title: post.data.title,
      description: post.data.description,
      pubDate: post.data.pubDate,
      link,
      content: feedContent(html, link),
    });
  }
  return rss({
    title: res.str.blog_title(),
    description: res.str.blog_description(),
    site: new URL(url('blog'), site!).href,
    items,
    customData: '<language>en</language>',
  });
};
