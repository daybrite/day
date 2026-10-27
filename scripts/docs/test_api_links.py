#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Guide-to-docs.rs link fixtures, checked against a synthetic rustdoc tree; no network or real build."""
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

HAPTICS = 'https://docs.rs/day-part-haptics/latest/day_part_haptics'


class GuideApiLinks(unittest.TestCase):
    def test_docs_rs_destinations_fragments_and_redirects(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            site, docs, work = root / 'site', root / 'doc', root / 'evidence'
            work.mkdir()

            def put(name, body):
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(body)

            crates = module.crates()
            put('site/docs/reference/api/index.html', ''.join(
                f'<a href="https://docs.rs/{pkg}/latest/{lib}/">Crate</a>' for pkg, lib in crates.items()))
            for lib in crates.values():
                put(f'doc/{lib}/index.html', f'<h1>{lib}</h1>')
            put('doc/day_part_haptics/fn.is_supported.html', '<div id="contract"></div>')
            put('doc/day_part_haptics/alias.html',
                '<meta http-equiv="refresh" content="0; url=fn.is_supported.html">')
            guide = 'docs/reference/haptics/index.html'
            valid = [
                f'{HAPTICS}/fn.is_supported.html#contract',
                f'{HAPTICS}/alias.html#contract',
                f'{HAPTICS}/',
                'https://docs.rs/day-piece-webview/latest/day_piece_webview/struct.Unbuilt.html',
            ]
            invalid = [
                f'{HAPTICS}/fn.missing.html',
                f'{HAPTICS}/alias.html#missing',
                'https://docs.rs/day-part-haptics/latest/haptics/fn.is_supported.html',
                'https://daybrite.dev/api/day_part_haptics/fn.is_supported.html',
                '/api/day_part_haptics/fn.is_supported.html',
            ]
            with patch.object(module, 'WORK', work):
                for href in valid + invalid:
                    with self.subTest(href=href):
                        put(f'site/{guide}', f'<a href="{href}">Synthetic API link</a>')
                        output = io.StringIO()
                        with redirect_stdout(output):
                            status = module.main(['--site', str(site), '--docs', str(docs)])
                        self.assertEqual(status, 0 if href in valid else 1, output.getvalue())
                        result = json.loads((work / 'api-links.json').read_text())
                        if status:
                            self.assertEqual(result['failures'][0]['page'], guide)
                            self.assertIn(f'{guide}: {result["failures"][0]["reason"]}: {href}',
                                          output.getvalue())

            # Every documented crate must appear on the API reference page.
            put('site/docs/reference/api/index.html', '<h1>No crates</h1>')
            put(f'site/{guide}', '<p>No links</p>')
            with patch.object(module, 'WORK', work), redirect_stdout(io.StringIO()):
                self.assertEqual(module.main(['--site', str(site), '--docs', str(docs)]), 1)


if __name__ == '__main__':
    unittest.main()
