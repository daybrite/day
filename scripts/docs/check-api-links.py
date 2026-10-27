#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Check the site's API links against a docs.rs-equivalent rustdoc build.

The site publishes no API reference of its own: every API link goes to docs.rs as
https://docs.rs/<package>/latest/<library>/<path>[#anchor]. For each rendered page of the built site,
each such link to a crate of this workspace (website/src/data/api-crates.json) must name a page and
anchor in the build scripts/build-api-docs.sh makes, which documents what docs.rs documents: its
default target and each crate's `[package.metadata.docs.rs]` features. A link that fails here would
fail on docs.rs too, for a reason a release does not fix (a typo, or an item behind a platform or a
feature docs.rs does not build). Rustdoc's redirect pages for re-exports are followed.

Links to crates outside the workspace (external pieces and parts) are not built here and are not
checked. A link to daybrite.dev/api/ or a site-relative /api/ fails: that reference no longer exists.

    check-api-links.py [--site website/dist] [--docs target/api-check/x86_64-unknown-linux-gnu/doc]
"""
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlsplit, unquote
import argparse
import json
import os
import re

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / '.cache/docs'
SITE = ROOT / 'website/dist'
DOCS = ROOT / 'target/api-check/x86_64-unknown-linux-gnu/doc'
DOCS_RS = re.compile(r'^https://docs\.rs/([^/]+)/[^/]+/([^/]+)/(.*)$')


class Page(HTMLParser):
    def __init__(self, path):
        super().__init__()
        self.ids, self.links, self.redirect = set(), [], None
        self.feed(path.read_text())

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if 'id' in attrs:
            self.ids.add(attrs['id'])
        if tag == 'a' and 'href' in attrs:
            self.links.append(attrs['href'])
        if tag == 'meta' and attrs.get('http-equiv', '').lower() == 'refresh':
            match = re.search(r'url=(.*)', attrs.get('content', ''), re.I)
            if match:
                self.redirect = match[1].strip()


def crates():
    """{package: library} for every crate the site documents."""
    manifest = json.loads((ROOT / 'website/src/data/api-crates.json').read_text())
    return {c['pkg']: c.get('lib') or c['pkg'].replace('-', '_')
            for group in manifest['groups'] for c in group['crates']}


def check(site, docs):
    workspace = crates()
    cache = {}

    def page(path):
        if path not in cache:
            cache[path] = Page(path)
        return cache[path]

    def resolve(path, fragment):
        """The reason a local rustdoc path and fragment do not resolve, or None."""
        for _ in range(10):
            if not path.is_file():
                if path.suffix:
                    return 'missing page'
                path /= 'index.html'
                continue
            target = page(path)
            if target.redirect:
                parsed = urlsplit(target.redirect)
                path = Path(os.path.normpath(path.parent / unquote(parsed.path)))
                fragment = parsed.fragment or fragment
                continue
            # Browsers try the literal ID before percent-decoding the fragment.
            if fragment and fragment not in target.ids and unquote(fragment) not in target.ids:
                return 'missing anchor'
            return None
        return 'redirect loop'

    failures, checked, external = [], 0, 0
    for source in sorted(site.rglob('*.html')):
        for href in page(source).links:
            parsed = urlsplit(href)
            where = str(source.relative_to(site))
            if (parsed.netloc == 'daybrite.dev' and parsed.path.startswith('/api/')) or \
                    (not parsed.netloc and parsed.path.startswith('/api/')):
                failures.append({'page': where, 'href': href, 'reason': 'self-hosted API link; use docs.rs'})
                continue
            match = DOCS_RS.match(f'{parsed.scheme}://{parsed.netloc}{parsed.path}')
            if not match:
                continue
            package, library, rest = match.groups()
            if package not in workspace:
                external += 1
                continue
            checked += 1
            if workspace[package] != library:
                failures.append({'page': where, 'href': href,
                                 'reason': f'{package} documents as {workspace[package]}, not {library}'})
                continue
            reason = resolve(docs / library / unquote(rest), parsed.fragment)
            if reason:
                failures.append({'page': where, 'href': href, 'reason': reason})

    # The API reference page lists every documented crate at its docs.rs home.
    index = site / 'docs/reference/api/index.html'
    listed = set(page(index).links) if index.is_file() else set()
    for package, library in sorted(workspace.items()):
        if not any(h.startswith(f'https://docs.rs/{package}/latest/{library}/') for h in listed):
            failures.append({'page': 'docs/reference/api/index.html', 'href': package,
                             'reason': 'crate missing from the API reference page'})
    return {'checked': checked, 'external_skipped': external, 'crates': len(workspace), 'failures': failures}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument('--site', type=Path, default=SITE)
    parser.add_argument('--docs', type=Path, default=DOCS)
    args = parser.parse_args(argv)
    if not (args.docs / 'day' / 'index.html').is_file():
        print(f'No rustdoc in {args.docs}; run scripts/build-api-docs.sh first.')
        return 1
    result = check(args.site.resolve(), args.docs.resolve())
    WORK.mkdir(parents=True, exist_ok=True)
    (WORK / 'api-links.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: value for key, value in result.items() if key != 'failures'}))
    print(f'Broken API links: {len(result["failures"])}; full details in {WORK / "api-links.json"}')
    for failure in result['failures']:
        print(f'{failure["page"]}: {failure["reason"]}: {failure["href"]}')
    return int(bool(result['failures']))


if __name__ == '__main__':
    raise SystemExit(main())
