// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Small helpers for base-path-aware URLs. Astro sets `import.meta.env.BASE_URL` to the configured
// `base` (with a trailing slash), so every internal link / static asset must be built through here.

export const BASE: string = import.meta.env.BASE_URL;

/** Join a path onto the site base, e.g. url('docs/overview') -> '/day/docs/overview'. */
export function url(path = ''): string {
  return BASE.replace(/\/$/, '') + '/' + path.replace(/^\//, '');
}

// The internal reference docs (repo `docs/*.md`, symlinked into src/content/internal) carry no
// frontmatter, so their titles and descriptions are derived here from the filename / body at render
// time. Special-case the ids where plain title-casing is wrong; everything else is title-cased.
const INTERNAL_TITLES: Record<string, string> = {
  'api-style': 'API style',
  baseline: 'Baseline alignment',
  harmonyos: 'HarmonyOS',
  vscode: 'VS Code extension',
  deviceinfo: 'Device info',
  searchfield: 'Search field',
  webview: 'Web view',
};

/** Human-readable title for an internal doc id, e.g. `navigation` -> "Navigation",
 * `api-style` -> "API style", `harmonyos` -> "HarmonyOS". */
export function internalTitle(id: string): string {
  const key = id.replace(/\.md$/, '').split('/').pop() || id;
  if (INTERNAL_TITLES[key]) return INTERNAL_TITLES[key];
  return key
    .split(/[-_]/)
    .map((w) => (w ? w[0].toUpperCase() + w.slice(1) : w))
    .join(' ');
}

/** A short one-line description for an internal doc, taken from the first prose paragraph after the
 * leading `# H1` and stripped of markdown syntax. Robust to blockquote "Status:" leads, and to the
 * HTML comments these files open with. */
export function internalExcerpt(body: string, max = 155): string {
  const lines = (body || '').replace(/\r/g, '').split('\n');
  let i = 0;
  const skipBlank = () => {
    while (i < lines.length && lines[i].trim() === '') i++;
  };
  // These docs carry HTML comments — the CC-BY-SA-4.0 header, and a "generated, do not edit" line
  // on the matrices. Without skipping them the "first prose paragraph" below is the licence, which
  // is what every card on /docs/internal used to show. Three shapes appear in the tree: `<!--`
  // alone on its line, an opener with text after it, and a whole comment on one line — so skip to
  // whichever line closes it, and loop for a doc that leads with two.
  const skipComments = () => {
    while (i < lines.length && lines[i].trimStart().startsWith('<!--')) {
      while (i < lines.length && !lines[i].includes('-->')) i++;
      i++; // the closing line itself
      skipBlank();
    }
  };
  skipBlank();
  // Both sides of the H1: most docs lead with the licence comment, while the generated matrices
  // put their heading first and the "do not edit" banner under it.
  skipComments();
  if (i < lines.length && /^#\s/.test(lines[i])) i++; // skip the H1
  skipBlank();
  skipComments();
  const para: string[] = [];
  while (i < lines.length && lines[i].trim() !== '') {
    para.push(lines[i]);
    i++;
  }
  let text = para
    .join(' ')
    .replace(/^>\s?/gm, '') // blockquote markers
    .replace(/!\[[^\]]*\]\([^)]*\)/g, '') // images
    .replace(/\[([^\]]+)\]\([^)]*\)/g, '$1') // links -> text
    .replace(/`([^`]+)`/g, '$1') // inline code
    .replace(/\*\*([^*]+)\*\*/g, '$1') // bold
    .replace(/\*([^*]+)\*/g, '$1') // italic
    .replace(/_([^_]+)_/g, '$1') // underscore emphasis
    .replace(/\s+/g, ' ')
    .trim();
  if (text.length > max) text = text.slice(0, max).replace(/\s+\S*$/, '').trim() + '…';
  return text;
}

export const site = {
  name: 'Day',
  tagline: 'Create native apps for every platform under the sun from a single Rust codebase.',
  description:
    'Day is a framework for Rust app development that builds for Android, iOS, HarmonyOS, Windows, macOS, Linux, and the web using each platform’s native interface components, so your product looks and works the way users of each platform expect.',
  repo: 'https://github.com/daybrite/day',
  /** The showcase app's repository — it is its own project, released and deployed from there. */
  showcaseRepo: 'https://github.com/daybrite/Day-Showcase',
  /** The live web-dom build, on the showcase's own subdomain. Deployed by the showcase's OWN CI,
   *  not hosted here: this site documents the framework, and the app publishes itself. Deep links
   *  use a URL fragment as the app's route (`#canvas`), which the DOM shim reads on load. The
   *  trailing slash matters — the fragment is appended directly. */
  showcaseWeb: 'https://showcase.daybrite.dev/webapp/',
};
