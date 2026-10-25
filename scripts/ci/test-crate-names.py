#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Regression checks for the rename guard, using disposable Git fixtures without commits."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("naming", HERE / "check-crate-names.py")
naming = importlib.util.module_from_spec(spec)
spec.loader.exec_module(naming)
manifest = json.loads((HERE / "crate-renames.json").read_text())

class NamingTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="day-names-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        subprocess.run(["git", "init", "-q", str(self.root)], check=True)

    def errors(self, content):
        (self.root / "fixture.txt").write_text(content)
        return naming.check(self.root, manifest)

    def test_packages_and_imports_are_rejected_in_untracked_files(self):
        for old in manifest["packages"]:
            for spelling in (old, old.replace("-", "_")):
                with self.subTest(spelling=spelling):
                    self.assertTrue(self.errors(f'"{spelling}"'))

    def test_canonical_names_pass(self):
        self.assertEqual(self.errors("\n".join(manifest["packages"].values())), [])

    def test_repository_paths_do_not_exempt_package_references_on_same_line(self):
        for old in manifest["external_repository_names_retained"]:
            url = f"https://github.com/daybrite/{old}"
            self.assertEqual(self.errors(f'git = "{url}"; checkout = "../{old}"'), [])
            self.assertTrue(self.errors(f'{old} = {{ git = "{url}" }}'))

    def test_old_layout_paths_fail_and_new_layout_paths_pass(self):
        for old, new in manifest["directories"].items():
            with self.subTest(path=old):
                self.assertTrue(self.errors(old + "/Cargo.toml"))
                self.assertTrue(self.errors(old.replace("/", "\\") + "\\Cargo.toml"))
                self.assertEqual(self.errors(new + "/Cargo.toml"), [])

    def test_worktree_repair_is_read_without_staging(self):
        old, new = next(iter(manifest["packages"].items()))
        self.errors(old)
        subprocess.run(["git", "-C", str(self.root), "add", "fixture.txt"], check=True)
        self.assertEqual(self.errors(new), [])

if __name__ == "__main__":
    unittest.main()
