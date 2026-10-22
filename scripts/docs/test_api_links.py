#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Guide-to-Rustdoc regression fixtures; no network or real API build required."""
from contextlib import redirect_stdout
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('api_links', Path(__file__).with_name('check-api-links.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class GuideApiLinks(unittest.TestCase):
    def test_guide_destinations_fragments_and_aliases(self):
        with tempfile.TemporaryDirectory() as directory:
            dist = Path(directory)
            work = dist / 'evidence'
            work.mkdir()

            def put(name, body):
                path = dist / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(body)

            nav = ''.join(f'<a href="{href}">Guide</a>' for href in
                          ['/', '/docs/reference/api', '/docs/explanation/overview'])
            manifest = json.loads((module.ROOT / 'website/src/data/api-crates.json').read_text())
            crates = [c['pkg'].replace('-', '_') for g in manifest['groups'] for c in g['crates']]
            for crate in crates:
                put(f'api/{crate}/index.html', nav)
            put('index.html', '<h1>Synthetic home</h1>')
            put('docs/explanation/overview/index.html', '<h1>Synthetic overview</h1>')
            put('docs/reference/api/index.html', ''.join(
                f'<a href="/api/{c}/index.html">Crate</a>' for c in crates))
            put('api/day_part_haptics/fn.is_supported.html', nav + '<div id="contract"></div>')
            put('api/day_part_haptics/alias.html',
                '<meta http-equiv="refresh" content="0; url=fn.is_supported.html">')
            guide = 'docs/reference/haptics/index.html'
            valid = [
                '/api/day_part_haptics/fn.is_supported.html#contract',
                'https://daybrite.dev/api/day_part_haptics/fn.is_supported.html#contract',
                '../../../api/day_part_haptics/alias.html#contract',
            ]
            with patch.object(module, 'DIST', dist), patch.object(module, 'WORK', work):
                for href in valid + [
                    '/api/day_part_haptics/fn.missing.html',
                    '/api/day_part_haptics/alias.html#missing',
                ]:
                    with self.subTest(href=href):
                        put(guide, f'<a href="{href}">Synthetic API link</a>')
                        with redirect_stdout(io.StringIO()):
                            status = module.main()
                        self.assertEqual(status, 0 if href in valid else 1)
                        result = json.loads((work / 'api-links.json').read_text())
                        self.assertGreater(result['guide_api_links'], 0)
                        if status:
                            self.assertEqual(result['failures'][0]['page'], guide)


if __name__ == '__main__':
    unittest.main()
