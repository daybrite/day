#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Reuse the built site's footer and styles through Rustdoc's HTML hooks."""
import argparse
from html.parser import HTMLParser
from pathlib import Path


class Shell(HTMLParser):
    def __init__(self, source):
        super().__init__(convert_charrefs=False)
        self.source = source
        self.lines = source.splitlines(keepends=True)
        self.styles = []
        self.theme = None
        self.fragments = {}
        self.start = None
        self.feed(source)

    def source_offset(self):
        line, column = self.getpos()
        return sum(map(len, self.lines[:line - 1])) + column

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        if tag == 'link' and attrs.get('rel') == 'stylesheet':
            self.styles.append(self.get_starttag_text())
        if tag == 'script' and 'data-day-theme' in attrs:
            self.theme = self.get_starttag_text() + '</script>'
        if tag == 'footer':
            self.start = (tag, self.source_offset())

    def handle_endtag(self, tag):
        if self.start and self.start[0] == tag:
            self.fragments[tag] = self.source[self.start[1]:self.source_offset() + len(tag) + 3]
            self.start = None


def build(site, output):
    bridge = site / 'docs/reference/api/index.html'
    if not bridge.is_file():
        raise ValueError(f'Build the site first (npm --prefix website run build:site): missing {bridge}')
    shell = Shell(bridge.read_text())
    if not shell.styles or not shell.theme or 'footer' not in shell.fragments:
        raise ValueError('API bridge is missing the shared styles, theme script or footer; rebuild the site.')
    output.mkdir(parents=True, exist_ok=True)
    (output / 'head.html').write_text('\n'.join(shell.styles + [shell.theme,
        '<link rel="stylesheet" href="/styles/rustdoc.css">',
        '<link rel="icon" type="image/svg+xml" href="/favicon.svg">']) + '\n')
    (output / 'before.html').write_text('<div class="day-api-content">\n')
    (output / 'after.html').write_text(
        '</div>\n<nav class="day-api-links container" aria-label="Day API navigation">'
        '<a href="/docs/reference/api">All Day crates</a>'
        '<a href="/docs/explanation/overview">Guides overview</a></nav>\n' + shell.fragments['footer'] + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--site-dir', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    args = parser.parse_args()
    build(args.site_dir, args.out)
