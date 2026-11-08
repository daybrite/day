// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import sanitizeHtml from 'sanitize-html';

/** Preserve article markup while removing scripts, site styling, and hydration attributes. */
export function feedContent(html, permalink) {
  return sanitizeHtml(html, {
    allowedTags: [...sanitizeHtml.defaults.allowedTags, 'img', 'details', 'summary'],
    allowedAttributes: {
      ...sanitizeHtml.defaults.allowedAttributes,
      '*': ['id'],
      img: ['src', 'alt', 'title', 'width', 'height'],
    },
    transformTags: {
      '*': (tagName, attribs) => {
        for (const name of ['href', 'src']) {
          if (!(name in attribs)) continue;
          try {
            attribs[name] = new URL(attribs[name], permalink).href;
          } catch {
            delete attribs[name];
          }
        }
        return { tagName, attribs };
      },
    },
  });
}
