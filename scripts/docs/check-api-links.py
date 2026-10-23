#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Check Rustdoc and guide-to-API links, including alias redirects and encoded IDs."""
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
    guide_links = 0

    def destination(source, href):
        parsed = urlsplit(href)
        if (parsed.scheme or parsed.netloc) and parsed.netloc != 'daybrite.dev':
            return None, parsed.fragment
        path = unquote(parsed.path)
        target = DIST / path.lstrip('/') if path.startswith('/') else source.parent / path if path else source
        target = Path(os.path.normpath(target))
        if target not in files and not target.suffix:
            target /= 'index.html'
        return target, parsed.fragment

    # Guide links used to escape both checkers: the site checker skips /api/, while this
    # checker only walked Rustdoc. Include every guide's API links in the same fragment check.
    guide_pages = []
    for source in sorted(files):
        if source.suffix != '.html' or source in pages:
            continue
        page = Page(source)
        api_links = []
        for href in page.links:
            target, _ = destination(source, href)
            if target is not None and target.is_relative_to(DIST / 'api'):
                api_links.append(href)
        if api_links:
            page.links = api_links
            pages[source] = page
            guide_pages.append(source)
            guide_links += len(api_links)

    for source in api_pages + guide_pages:
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
    bridge_path = DIST / 'docs/reference/api/index.html'
    bridge = Page(bridge_path)
    bridge_targets = {destination(bridge_path, href)[0] for href in bridge.links}
    navigation_failures = []
    for crate in crates:
        route = f'/api/{crate}/index.html'
        target = DIST / route.lstrip('/')
        if target not in bridge_targets or target not in files:
            navigation_failures.append(route)
    for path in api_pages:
        page = pages[path]
        if page.redirect:
            continue
        for href in ['/docs/reference/api', '/docs/explanation/overview', '/']:
            if href not in page.links:
                navigation_failures.append(str(path.relative_to(DIST)) + ': ' + href)

    result = {'html_pages': len(api_pages), 'guide_api_links': guide_links,
              'checked': checked, 'external_skipped': external,
              'crates': len(crates), 'navigation_failures': navigation_failures, 'failures': failures}
    (WORK / 'api-links.json').write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: value for key, value in result.items() if key != 'failures'}))
    print(f'Broken API links: {len(failures)}; full details in {WORK / "api-links.json"}')
    for failure in failures:
        print(f'{failure["page"]}: {failure["reason"]}: {failure["href"]}')
    return int(bool(failures or navigation_failures))


if __name__ == '__main__':
    raise SystemExit(main())
