#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Check the generated API bundle, including Rustdoc alias redirects and encoded IDs."""
from pathlib import Path
from html.parser import HTMLParser
from urllib.parse import urlsplit, unquote
import json
import sys
import os
import re

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / '.cache/docs'
WORK.mkdir(parents=True, exist_ok=True)
DIST = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else ROOT / 'website/dist'


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


def main():
    pages = {path: Page(path) for path in (DIST / 'api').rglob('*.html')}
    api_pages = list(pages)
    files = {path for path in DIST.rglob('*') if path.is_file()}
    failures = []
    checked = external = 0

    def destination(source, href):
        parsed = urlsplit(href)
        if parsed.scheme or parsed.netloc:
            return None, parsed.fragment
        path = unquote(parsed.path)
        target = DIST / path.lstrip('/') if path.startswith('/') else source.parent / path if path else source
        target = Path(os.path.normpath(target))
        if target not in files and not target.suffix:
            target /= 'index.html'
        return target, parsed.fragment

    for source in api_pages:
        for href in pages[source].links:
            target, fragment = destination(source, href)
            if target is None:
                external += 1
                continue
            checked += 1
            reason = None
            for _ in range(10):
                if target not in files:
                    reason = 'missing file'
                    break
                if target.suffix != '.html':
                    break
                if target not in pages:
                    pages[target] = Page(target)
                page = pages[target]
                if page.redirect:
                    target, next_fragment = destination(target, page.redirect)
                    fragment = next_fragment or fragment
                    if target is None:
                        external += 1
                        break
                    continue
                # Browsers try the literal ID before percent-decoding the fragment.
                if fragment and fragment not in page.ids and unquote(fragment) not in page.ids:
                    reason = 'missing anchor'
                break
            else:
                reason = 'redirect loop'
            if reason:
                failures.append({'page': str(source.relative_to(DIST)), 'href': href, 'reason': reason})

    manifest = json.loads((ROOT / 'website/src/data/api-crates.json').read_text())
    crates = [crate['pkg'].replace('-', '_') for group in manifest['groups'] for crate in group['crates']]
    bridge = Page(DIST / 'docs/reference/api/index.html')
    navigation_failures = []
    for crate in crates:
        route = f'/api/{crate}/index.html'
        if route not in bridge.links or DIST / route.lstrip('/') not in files:
            navigation_failures.append(route)
    for path in api_pages:
        page = pages[path]
        if page.redirect:
            continue
        for href in ['/docs/reference/api', '/docs/explanation/overview', '/']:
            if href not in page.links:
                navigation_failures.append(str(path.relative_to(DIST)) + ': ' + href)

    result = {'html_pages': len(api_pages), 'checked': checked, 'external_skipped': external,
              'crates': len(crates), 'navigation_failures': navigation_failures, 'failures': failures}
    (WORK / 'api-links.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: value for key, value in result.items() if key != 'failures'}))
    print(f'Broken API links: {len(failures)}; full details in {WORK / "api-links.json"}')
    return int(bool(failures or navigation_failures))


if __name__ == '__main__':
    raise SystemExit(main())
