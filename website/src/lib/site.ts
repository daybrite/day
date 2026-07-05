// Small helpers for base-path-aware URLs. Astro sets `import.meta.env.BASE_URL` to the configured
// `base` (with a trailing slash), so every internal link / static asset must be built through here.

export const BASE: string = import.meta.env.BASE_URL;

/** Join a path onto the site base, e.g. url('docs/overview') -> '/day/docs/overview'. */
export function url(path = ''): string {
  return BASE.replace(/\/$/, '') + '/' + path.replace(/^\//, '');
}

export const site = {
  name: 'Day',
  tagline: 'One Rust codebase. Real native widgets on every platform.',
  description:
    'Day is an industry-strength Rust framework for cross-platform apps that are genuinely native — SwiftUI-like Pieces realized with real AppKit, UIKit, GTK, Qt, WinUI, and Android widgets, with build-once/bind-forever reactivity.',
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
