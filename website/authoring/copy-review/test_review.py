# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Synthetic decisions exercise the opt-in boundary and persistence contracts."""
import http.client
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading
import unittest
from unittest.mock import patch

import build
import serve

ROOT = Path(__file__).resolve().parent


class ReviewTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.local = Path(self.temp.name)
        self.addCleanup(self.temp.cleanup)
        self.segment = {
            'id': 'synthetic', 'name': 'Synthetic fixture',
            'fields': [{'id': 'f0', 'kind': 'eyebrow', 'text': 'Fixture'},
                       {'id': 'f1', 'kind': 'p', 'text': 'Original fixture text'}],
            'options': [{'id': 'A', 'values': {'f0': '', 'f1': 'Alternative fixture text'}}],
        }
        self.data = {'revision': 'synthetic-revision', 'source_commit': 'synthetic-commit',
                     'source_sha256': 'synthetic-hash',
                     'segments': [self.segment]}
        patcher = patch.multiple(serve, DATA=self.data, SEGMENTS={'synthetic': self.segment},
                                 LOCAL=self.local, STATE=self.local / 'review-state.json')
        patcher.start()
        self.addCleanup(patcher.stop)

    def choice(self, version=0):
        return {'revision': self.data['revision'], 'base_version': version, 'segment': 'synthetic',
                'record': {'option': 'A', 'values': {'f0': '', 'f1': 'Edited fixture <script>'}}}

    def test_both_entry_points_default_off(self):
        env = dict(os.environ)
        env.pop('DAY_COPY_REVIEW', None)
        for command in (['node', str(ROOT / 'start.mjs')],
                        ['python3', '-B', str(ROOT / 'serve.py')]):
            result = subprocess.run(command, env=env, capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('disabled', result.stdout)

    def test_exact_edits_persist_and_are_marked_edited(self):
        state = serve.save_choice(self.choice())
        self.assertEqual(state, serve.load_state())
        self.assertEqual(state['records']['synthetic']['values']['f1'], 'Edited fixture <script>')
        self.assertTrue(state['records']['synthetic']['edited'])
        self.assertEqual(state['version'], 1)
        self.assertEqual(state['source_sha256'], 'synthetic-hash')

    def test_stale_or_invalid_save_preserves_existing_choice(self):
        accepted = serve.save_choice(self.choice())
        with self.assertRaises(RuntimeError):
            serve.save_choice(self.choice())
        for change in ({'revision': 'stale'}, {'segment': 'unknown'},
                       {'record': {'option': 'A', 'values': {'f1': 'missing field'}}},
                       {'record': {'option': 'A', 'values': {'f0': '', 'f1': ''}}}):
            request = self.choice(1)
            request.update(change)
            with self.assertRaises(ValueError):
                serve.save_choice(request)
        self.assertEqual(serve.load_state(), accepted)

    def test_proposal_change_archives_decisions_and_same_review_keeps_them(self):
        (self.local / 'source.json').write_text(json.dumps({
            'source_commit': 'synthetic-commit', 'source_sha256': 'synthetic-hash'}))
        (self.local / 'inventory.json').write_text(json.dumps([self.segment]))
        alternatives = {'synthetic': [('Fixture option', ['', 'Alternative fixture text'])]}
        with patch.multiple(build, LOCAL=self.local, DRAFTS=alternatives):
            build.build()
            proposals = json.loads((self.local / 'proposals.json').read_text())
            with patch.dict(serve.DATA, {'revision': proposals['revision']}):
                serve.save_choice(self.choice())
            baseline = (self.local / 'review-state.json').read_bytes()
            build.build()
            self.assertEqual((self.local / 'review-state.json').read_bytes(), baseline)
            alternatives['synthetic'][0][1][1] = 'Changed fixture alternative'
            build.build()
        self.assertFalse((self.local / 'review-state.json').exists())
        archives = list((self.local / 'history').glob('*/review-state.json'))
        self.assertEqual(len(archives), 1)
        self.assertEqual(archives[0].read_bytes(), baseline)
        self.assertTrue((archives[0].parent / 'proposals.json').is_file())

    def test_http_origin_token_and_file_boundaries(self):
        (self.local / 'index.html').write_text('<p>Synthetic page</p>')
        server = serve.ThreadingHTTPServer(('127.0.0.1', 0), serve.Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        self.addCleanup(server.server_close)
        self.addCleanup(server.shutdown)
        port = server.server_port

        def request(method, route, body=None, headers=None):
            connection = http.client.HTTPConnection('127.0.0.1', port)
            connection.request(method, route, json.dumps(body) if body else None, headers or {})
            response = connection.getresponse()
            data = response.read()
            connection.close()
            return response.status, data

        headers = {'Content-Type': 'application/json', 'Origin': f'http://127.0.0.1:{port}',
                   'X-Review-Token': serve.TOKEN}
        self.assertEqual(request('GET', '/')[0], 200)
        self.assertEqual(request('GET', '/', headers={'Host': 'example.com'})[0], 403)
        self.assertEqual(request('POST', '/api/choice', self.choice(),
                                 {**headers, 'Origin': 'https://example.com'})[0], 403)
        self.assertEqual(request('POST', '/api/choice', self.choice(),
                                 {**headers, 'X-Review-Token': 'invalid'})[0], 403)
        self.assertEqual(request('POST', '/api/choice', self.choice(), headers)[0], 200)
        self.assertEqual(request('POST', '/api/choice', self.choice(), headers)[0], 409)
        self.assertEqual(request('GET', '/review-state.json')[0], 404)
        self.assertEqual(request('GET', '/assets/%2e%2e/%2e%2e/serve.py')[0], 404)


if __name__ == '__main__':
    unittest.main()
