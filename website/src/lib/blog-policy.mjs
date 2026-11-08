// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

/** Check the source path too: frontmatter may override the entry's slug. */
export function isDraftPost(post) {
  return post.data.draft === true || /(?:^|\/)drafts(?:\/|$)/.test(post.filePath ?? post.id);
}

export function newestPostsFirst(a, b) {
  return b.data.pubDate.valueOf() - a.data.pubDate.valueOf() || a.id.localeCompare(b.id);
}
