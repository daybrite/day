#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Synthetic HTML fixtures for the shared site/Rustdoc presentation."""
import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('rustdoc_shell', Path(__file__).with_name('rustdoc-shell.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class RustdocShell(unittest.TestCase):
    def test_preserves_footer_and_styles_without_the_site_header(self):
        header = '<header class="nav" data-astro-cid-test><nav><a href="/docs">Docs</a></nav><script>const label="a>b";</script></header>'
        footer = '<footer class="footer"><nav><a href="/">Home</a></nav></footer>'
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            page = site / 'docs/reference/api/index.html'
            page.parent.mkdir(parents=True)
            page.write_text('<html><head>\n<link rel="stylesheet" href="/_astro/site.hash.css">'
                            '<script data-day-theme src="/scripts/theme.js"></script></head>\n'
                            '<body>' + header + '<main>API bridge</main>' + footer + '</body></html>')
            output = site / 'shell'
            module.build(site, output)
            self.assertEqual((output / 'before.html').read_text().strip(), '<div class="day-api-content">')
            self.assertNotIn('<header', (output / 'before.html').read_text())
            self.assertTrue((output / 'after.html').read_text().startswith('</div>'))
            self.assertIn(footer, (output / 'after.html').read_text())
            self.assertIn('href="/_astro/site.hash.css"', (output / 'head.html').read_text())
            self.assertIn('src="/scripts/theme.js"', (output / 'head.html').read_text())

    def test_missing_or_stale_site_build_fails_before_rustdoc(self):
        with tempfile.TemporaryDirectory() as directory:
            site = Path(directory)
            with self.assertRaisesRegex(ValueError, 'Build the site first'):
                module.build(site, site / 'shell')
            page = site / 'docs/reference/api/index.html'
            page.parent.mkdir(parents=True)
            page.write_text('<html><body>Old build</body></html>')
            with self.assertRaisesRegex(ValueError, 'rebuild the site'):
                module.build(site, site / 'shell')


if __name__ == '__main__':
    unittest.main()
