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
 * leading `# H1` and stripped of markdown syntax. Robust to blockquote "Status:" leads. */
export function internalExcerpt(body: string, max = 155): string {
  const lines = (body || '').replace(/\r/g, '').split('\n');
  let i = 0;
  while (i < lines.length && lines[i].trim() === '') i++;
  if (i < lines.length && /^#\s/.test(lines[i])) i++; // skip the H1
  while (i < lines.length && lines[i].trim() === '') i++;
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
    'Day is a Rust framework that builds genuinely native applications for macOS, Windows, Linux, iOS, Android, and HarmonyOS from one codebase — using each platform’s own interface components, so your product looks and works the way users of each platform expect.',
  repo: 'https://github.com/daybrite/day',
  targets: [
    'macOS · AppKit',
    'iOS · UIKit',
    'Android · widget',
    'Linux · GTK 4',
    'Linux · Qt 6',
    'Windows · WinUI 3',
    'macOS/Windows · GTK & Qt',
  ],
};
