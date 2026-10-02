#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Exercise readiness checks in temporary repositories; never create commits."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]


class ReadinessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="day-readiness-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        # Start from the real checkout, including manifests and existing generated tables.
        # Do not regenerate the baseline: doing so could bless an incomplete fixture whose
        # generator silently skipped the same inputs as the implementation under test.
        paths = subprocess.check_output(
            ["git", "ls-files", "-z"], cwd=ROOT
        ).decode().split("\0")
        paths += [str(p.relative_to(ROOT)) for p in (ROOT / "scripts/ci").glob("*.sh")]
        paths += [str(p.relative_to(ROOT)) for p in (ROOT / "scripts/ci").glob("*.py")]
        paths += [".githooks/pre-commit"]
        for name in sorted(set(filter(None, paths))):
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest, follow_symlinks=False)
        self.env = os.environ.copy()
        # Tests also run from a Git hook; its repository overrides must not leak here.
        for key in list(self.env):
            if key.startswith("GIT_"):
                del self.env[key]
        self.env["PYTHONDONTWRITEBYTECODE"] = "1"
        self.run_cmd("git", "init", "-q")
        self.run_cmd("git", "add", "--force", ".")

    def run_cmd(self, *args, ok=True):
        result = subprocess.run(
            args, cwd=self.root, env=self.env, capture_output=True, text=True
        )
        if ok:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def test_current_worktree_and_index_pass(self):
        self.run_cmd("bash", "scripts/ci/check-matrices.sh")
        self.run_cmd("bash", "scripts/ci/check-matrices.sh", "--staged")
        for name in ("duty", "coverage", "recorder"):
            path = Path(f"docs/{name}-matrix.md")
            self.assertEqual((self.root / path).read_bytes(), (ROOT / path).read_bytes())

    def test_piece_manifests_are_inputs_to_the_staged_coverage_check(self):
        # Removing only a manifest must change coverage even when all Rust files remain.
        self.run_cmd("git", "rm", "--force", "pieces/day-piece-activity/Cargo.toml")
        result = self.run_cmd("bash", "scripts/ci/check-matrices.sh", "--staged", ok=False)
        self.assertIn("day-piece-activity", result.stderr)
        self.run_cmd("bash", "scripts/ci/coverage-matrix.sh")
        self.run_cmd("git", "add", "docs/coverage-matrix.md")
        self.run_cmd("bash", "scripts/ci/check-matrices.sh", "--staged")

    def test_stale_and_missing_tables_fail_without_rewriting(self):
        for name in ("duty", "coverage", "recorder"):
            path = self.root / f"docs/{name}-matrix.md"
            path.write_text("synthetic stale matrix fixture\n")
            result = self.run_cmd("bash", f"scripts/ci/{name}-matrix.sh", "--check", ok=False)
            self.assertIn("is stale", result.stderr)
            self.assertEqual(path.read_text(), "synthetic stale matrix fixture\n")
            path.unlink()
            self.run_cmd("bash", f"scripts/ci/{name}-matrix.sh", "--check", ok=False)
            self.assertFalse(path.exists())
            self.run_cmd("bash", f"scripts/ci/{name}-matrix.sh")

    def test_unstaged_repair_cannot_hide_stale_index(self):
        path = self.root / "docs/duty-matrix.md"
        path.write_text("synthetic stale staged fixture\n")
        self.run_cmd("git", "add", "docs/duty-matrix.md")
        self.run_cmd("bash", "scripts/ci/duty-matrix.sh")
        self.run_cmd("bash", "scripts/ci/check-matrices.sh")
        self.run_cmd("bash", "scripts/ci/check-matrices.sh", "--staged", ok=False)
        self.run_cmd("bash", ".githooks/pre-commit", ok=False)
        staged = self.run_cmd("git", "show", ":docs/duty-matrix.md").stdout
        self.assertEqual(staged, "synthetic stale staged fixture\n")
        self.run_cmd("git", "add", "docs/duty-matrix.md")
        self.run_cmd("bash", "scripts/ci/check-matrices.sh", "--staged")

    def test_staged_source_is_checked_even_if_worktree_was_reverted(self):
        path = self.root / "crates/day-spec/src/lib.rs"
        original = path.read_text()
        start = original.index("pub trait Toolkit")
        brace = original.index("{", start) + 1
        path.write_text(original[:brace] + "\n    fn synthetic_fixture_duty(&self) {}\n" + original[brace:])
        self.run_cmd("git", "add", "crates/day-spec/src/lib.rs")
        path.write_text(original)
        self.run_cmd("bash", "scripts/ci/check-matrices.sh")
        result = self.run_cmd("bash", "scripts/ci/check-matrices.sh", "--staged", ok=False)
        self.assertIn("synthetic_fixture_duty", result.stderr)

    def test_readiness_requires_fmt_and_every_host_clippy_command(self):
        # Synthetic cargo fixture: verify failures propagate without building copied crates.
        bindir = self.root / "test-bin"
        bindir.mkdir()
        cargo = bindir / "cargo"
        cargo.write_text(
            '#!/bin/sh\nprintf "%s\\n" "$*" >> "$DAY_TEST_CARGO_LOG"\n'
            'if [ "$1" = "$DAY_TEST_FAIL" ]; then exit 1; fi\n'
        )
        cargo.chmod(0o755)
        log = self.root / "cargo.log"
        self.env["PATH"] = str(bindir) + os.pathsep + self.env["PATH"]
        self.env["DAY_TEST_CARGO_LOG"] = str(log)
        for fail in ("fmt", "clippy", ""):
            self.env["DAY_TEST_FAIL"] = fail
            log.write_text("")
            self.run_cmd("bash", "scripts/ci/check-ready.sh", ok=not fail)
            calls = log.read_text().splitlines()
            self.assertEqual(calls[0], "fmt --all -- --check")
            self.assertEqual(len(calls), {"fmt": 1, "clippy": 2, "": 4}[fail])
        for fail in ("fmt", "clippy", ""):
            self.env["DAY_TEST_FAIL"] = fail
            log.write_text("")
            self.run_cmd("bash", ".githooks/pre-commit", ok=not fail)
            calls = log.read_text().splitlines()
            self.assertEqual(calls[0], "fmt --all")
            self.assertEqual(len(calls), {"fmt": 1, "clippy": 3, "": 5}[fail])
        (self.root / "docs/duty-matrix.md").write_text("synthetic drift fixture\n")
        log.write_text("")
        self.run_cmd("bash", "scripts/ci/check-ready.sh", ok=False)
        self.assertEqual(log.read_text(), "")


if __name__ == "__main__":
    unittest.main()
