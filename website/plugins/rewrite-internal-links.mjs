// Rehype plugin — fix links in the INTERNAL reference docs only.
//
// The internal docs (src/content/internal/*.md) are symlinks to the repo's top-level `docs/*.md`,
// which are authored with GitHub-native relative links: sibling `battery.md`, repo paths like
// `../crates/day-pieces`. Those are correct when read on GitHub but 404 when the same file is rendered
// on the web at `/docs/internal/<slug>`. Rather than rewrite the source docs (which would break them on
// GitHub), rewrite the links at render time:
//
//   `foo.md` / `./foo.md`      -> `/docs/internal/foo`   (a sibling internal doc page)
//   `../crates/x`, `pieces/y`  -> the GitHub source URL  (a path that only exists in the repo)
//
// Scoped to files under `content/internal/` so the curated docs (which use root-absolute links) are
// untouched. Absolute (`/…`, `https://…`), anchor (`#…`), and mail/tel links are left as-is.

const GITHUB_TREE = 'https://github.com/daybrite/day/tree/main/';

export default function rewriteInternalLinks() {
  return (tree, file) => {
    const path = (file?.path || file?.history?.[0] || '').replace(/\\/g, '/');
    if (!path.includes('/content/internal/')) return;

    const rewrite = (href) => {
      if (/^(https?:|mailto:|tel:|#|\/)/.test(href)) return href; // absolute / anchor — leave
      const hashAt = href.indexOf('#');
      const bare = hashAt >= 0 ? href.slice(0, hashAt) : href;
      const hash = hashAt >= 0 ? href.slice(hashAt) : '';
      if (/\.md$/i.test(bare)) {
        // A sibling internal doc: keep only the basename (all internal docs are flat siblings).
        const slug = bare.replace(/\.md$/i, '').split('/').pop();
        return `/docs/internal/${slug}${hash}`;
      }
      // A repo path that has no web page — point at the GitHub source.
      const rel = bare.replace(/^(\.\.\/)+/, '').replace(/^\.\//, '');
      return `${GITHUB_TREE}${rel}${hash}`;
    };

    const visit = (node) => {
      if (
        node.type === 'element' &&
        node.tagName === 'a' &&
        typeof node.properties?.href === 'string'
      ) {
        node.properties.href = rewrite(node.properties.href);
      }
      if (Array.isArray(node.children)) node.children.forEach(visit);
    };
    visit(tree);
  };
}
