#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Regression checks for standard-library links in re-exported Rustdoc HTML."""
from pathlib import Path
import importlib.util
import unittest

spec = importlib.util.spec_from_file_location(
    'rustdoc_source', Path(__file__).resolve().parents[1] / 'rustdoc-github-source.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class HostedStdLinks(unittest.TestCase):
    def test_reexport_at_different_depths(self):
        for depth in (1, 2, 4):
            source = f'<a href="{"../" * depth}std/time/struct.Instant.html#method.now">Instant</a>'
            rewritten, count = module.hosted_std_links(source)
            self.assertEqual(count, 1)
            self.assertIn('https://doc.rust-lang.org/stable/std/time/struct.Instant.html#method.now', rewritten)

    def test_keeps_day_absolute_and_fragment_links(self):
        source = '<a href="../day_spec/trait.Toolkit.html">Toolkit</a><a href="#local">Local</a><a href="https://example.org/std/time">External</a>'
        self.assertEqual(module.hosted_std_links(source), (source, 0))

    def test_core_and_alloc_with_single_quotes(self):
        for crate in ('core', 'alloc'):
            source = f"<a href='../{crate}/index.html'>Crate</a>"
            rewritten, count = module.hosted_std_links(source)
            self.assertEqual(count, 1)
            self.assertIn(f"href='https://doc.rust-lang.org/stable/{crate}/index.html'", rewritten)


if __name__ == '__main__':
    unittest.main()
