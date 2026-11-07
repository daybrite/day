#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Check that release bumps preserve wildcard dependencies without changing real repositories."""

import importlib.util
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("bump", Path(__file__).with_name("bump-version.py"))
bump = importlib.util.module_from_spec(spec)
spec.loader.exec_module(bump)


class VersionTests(unittest.TestCase):
    def test_workspace_manifests_follow_the_release_policy(self):
        _, version = bump.workspace_version(bump.ROOT / "Cargo.toml")
        for manifest in bump.tracked("*Cargo.toml"):
            with self.subTest(manifest=manifest):
                self.assertEqual(bump.stale_manifest_versions(manifest, version), [])

    def test_zero_minor_bumps_preserve_requirements(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            manifest.write_text('''[workspace.package]
version = "0.6.4"
[workspace.dependencies]
day-core = { path = "crates/day-core", version = "0.*" }
serde = "1"
''')
            for old, new in [("0.6.4", "0.7.0"), ("0.7.0", "0.8.0")]:
                bump.rewrite_manifest(manifest, old, new, True)
                self.assertEqual(bump.workspace_version(manifest)[1], new)
                self.assertEqual([v for _, _, v in bump.path_dep_versions(manifest.read_text())],
                                 ["0.*"])
                self.assertEqual(bump.stale_manifest_versions(manifest, new), [])
                self.assertIn('serde = "1"', manifest.read_text())

    def test_check_rejects_minor_and_exact_requirements(self):
        with tempfile.TemporaryDirectory() as directory:
            manifest = Path(directory) / "Cargo.toml"
            for requirement in ["0.6", "=0.6.4", ">=0.6.4, <1"]:
                manifest.write_text('[dependencies]\nday-core = '
                                    f'{{ path = "../day-core", version = "{requirement}" }}\n')
                self.assertEqual(bump.stale_manifest_versions(manifest, "0.6.4"),
                                 [(manifest, requirement)])


if __name__ == "__main__":
    unittest.main()
