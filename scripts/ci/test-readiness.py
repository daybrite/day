#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Exercise readiness checks in temporary repositories; never create commits."""

from contextlib import redirect_stdout
import io
import os
from pathlib import Path
import runpy
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest import mock


ROOT = Path(__file__).resolve().parents[2]


class DownstreamPolicyTests(unittest.TestCase):
    """Exercise checker verdicts with synthetic exports; no Git writes or network calls."""

    def check_downstream(self, fixed_in, *, strict=False, clone=False):
        script = ROOT / "scripts/ci/downstream-check.py"
        checker = runpy.run_path(str(script), run_name="downstream_policy_test")
        repo = checker["Repo"]("consumer", ROOT.parent / "consumer", "origin/main", 1, True)
        checked = []

        def check_root(_binary, _day, root, _env):
            checked.append(root.name)
            ok = root.name == fixed_in
            return ok, "" if ok else "synthetic missing package"

        args = [str(script), "--day", str(ROOT), "--day-bin", str(script)]
        if strict:
            args.append("--strict")
        if clone:
            args.append("--clone")
        output = io.StringIO()
        with mock.patch.dict(checker["main"].__globals__, {
            "git": lambda *_args: "fixture",
            "export_ref": mock.Mock(),
            "export_worktree": mock.Mock(),
            "sibling_repos": lambda *_args: [repo],
            "listed_repos": lambda *_args: ["example/consumer"],
            "run": lambda *_args, **_kwargs: subprocess.CompletedProcess([], 0, "", ""),
            "cargo_roots": lambda tree: [tree],
            "check_root": check_root,
        }), mock.patch.object(sys, "argv", args), redirect_stdout(output):
            status = checker["main"]()
        return status, output.getvalue(), checked

    def test_uncommitted_fix_warns_without_blocking(self):
        status, output, checked = self.check_downstream("worktree")
        self.assertEqual(status, 0)
        self.assertEqual(checked, ["pushed", "head", "worktree"])
        self.assertIn("FIXED, uncommitted", output)
        self.assertIn("warning: fixed only in the working tree", output)
        self.assertIn("do not block the day push", output)
        self.assertNotIn("every downstream repository resolves", output)

    def test_committed_fix_also_allows_local_push(self):
        status, output, checked = self.check_downstream("head")
        self.assertEqual(status, 0)
        self.assertEqual(checked, ["pushed", "head"])
        self.assertIn("FIXED, unpushed", output)

    def test_strict_requires_published_fixes(self):
        for fixed_in in ("head", "worktree"):
            with self.subTest(fixed_in=fixed_in):
                status, output, _ = self.check_downstream(fixed_in, strict=True)
                self.assertEqual(status, 1)
                self.assertNotIn("do not block the day push", output)

    def test_unresolved_breakage_still_blocks(self):
        status, output, _ = self.check_downstream(None)
        self.assertEqual(status, 1)
        self.assertIn("BROKEN", output)

    def test_ci_clone_cannot_use_a_local_fix(self):
        status, output, checked = self.check_downstream("worktree", clone=True)
        self.assertEqual(status, 1)
        self.assertEqual(checked, ["pushed"])
        self.assertIn("BROKEN", output)

    def test_published_fix_passes_local_and_ci_checks(self):
        for clone in (False, True):
            with self.subTest(clone=clone):
                status, output, checked = self.check_downstream("pushed", strict=True, clone=clone)
                self.assertEqual(status, 0)
                self.assertEqual(checked, ["pushed"])
                self.assertIn("every downstream repository resolves", output)


class ReadinessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="day-readiness-")
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        # Start from the real checkout, including manifests and existing generated tables.
        # Do not regenerate the baseline: doing so could bless an incomplete fixture whose
        # generator silently skipped the same inputs as the implementation under test.
        paths = subprocess.check_output(
            ["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT
        ).decode().split("\0")
        paths += [str(p.relative_to(ROOT)) for p in (ROOT / "scripts/ci").glob("*.sh")]
        paths += [str(p.relative_to(ROOT)) for p in (ROOT / "scripts/ci").glob("*.py")]
        paths += [".githooks/pre-commit"]
        for name in sorted(set(filter(None, paths))):
            if not (ROOT / name).exists():
                continue
            dest = self.root / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, dest, follow_symlinks=False)
        self.env = os.environ.copy()
        # Tests also run from a Git hook; its repository overrides must not leak here.
        for key in list(self.env):
            if key.startswith("GIT_"):
                del self.env[key]
        self.env["PYTHONDONTWRITEBYTECODE"] = "1"
        # The real suite runs outside the fixture. Record its nested invocation instead of
        # recursively running another suite; production has no self-test bypass switch.
        (self.root / "scripts/ci/test-readiness.py").write_text(
            'import os, sys\n'
            'from pathlib import Path\n'
            'Path("readiness-tests-called").touch()\n'
            'sys.exit(int(os.environ.get("DAY_TEST_SUITE_EXIT", "0")))\n'
        )
        self.env.pop("DAY_BIN", None)
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
        self.run_cmd("git", "rm", "--force", "pieces/day-piece-texteditor/Cargo.toml")
        result = self.run_cmd("bash", "scripts/ci/check-matrices.sh", "--staged", ok=False)
        self.assertIn("day-piece-texteditor", result.stderr)
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

    def test_android_runtime_rejects_backend_dependencies_and_java_references(self):
        self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py")
        manifest = self.root / "crates/day-android-runtime/Cargo.toml"
        original = manifest.read_text()
        manifest.write_text(original.replace("[dependencies]", "[dependencies]\nday-mdc.workspace = true"))
        result = self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py", ok=False)
        self.assertIn("day-android-runtime -> day-mdc", result.stderr)
        manifest.write_text(original)
        java = self.root / "parts/day-part-sensors/src/DaySensors.java"
        java.write_text(java.read_text() + "\nimport dev.daybrite.day.bridge.DayBridge;\n")
        result = self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py", ok=False)
        self.assertIn("DaySensors.java", result.stderr)

    def test_windows_runtime_rejects_ui_dependencies(self):
        manifest = self.root / "crates/day-windows-runtime/Cargo.toml"
        original = manifest.read_text()
        for dependency in ("day-winui", "day-xaml-sys", "day-core"):
            with self.subTest(dependency=dependency):
                manifest.write_text(original + f"\n{dependency}.workspace = true\n")
                result = self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py", ok=False)
                self.assertIn(f"day-windows-runtime -> {dependency}", result.stderr)
        manifest.write_text(original)
        self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py")

    def test_readiness_requires_fmt_and_every_host_clippy_command(self):
        # Synthetic cargo fixture: verify failures propagate without building copied crates.
        bindir = self.root / "test-bin"
        bindir.mkdir()
        cargo = bindir / "cargo"
        cargo.write_text(
            '#!/bin/sh\nprintf "%s\\n" "$*" >> "$DAY_TEST_CARGO_LOG"\n'
            'if [ "$1" = "$DAY_TEST_FAIL" ]; then exit 1; fi\n'
            'if [ "$(wc -l < "$DAY_TEST_CARGO_LOG")" -eq "$DAY_TEST_FAIL_AT" ]; then exit 1; fi\n'
            # Synthetic `day new app` output needed by the real scaffold overlay.
            'if [ "$1" = run ]; then\n'
            '  mkdir -p conformance/platform conformance/resource/vectors\n'
            '  touch conformance/Day.toml conformance/build.rs\n'
            'fi\n'
            'case "$*" in\n'
            '  *"--manifest-path apps/conformance/Cargo.toml"*)\n'
            '    test -f apps/conformance/Cargo.lock && test -f apps/conformance/build.rs && '
            'test -d apps/conformance/resource/fonts || exit 1;;\n'
            'esac\n'
        )
        cargo.chmod(0o755)
        log = self.root / "cargo.log"
        self.env["PATH"] = str(bindir) + os.pathsep + self.env["PATH"]
        self.env["DAY_TEST_CARGO_LOG"] = str(log)
        self.env["DAY_TEST_FAIL"] = ""
        self.env["DAY_TEST_FAIL_AT"] = "0"
        # The shared script owns the host lint combinations. Observe it independently so a new
        # combination automatically participates, without a stale count or a duplicated command
        # list. Comparing the full sequence still detects a readiness gate that skips host lint.
        log.write_text("")
        self.run_cmd("bash", "scripts/ci/host-clippy.sh")
        host_calls = log.read_text().splitlines()
        self.assertTrue(host_calls, "host-clippy.sh must run Clippy")
        self.assertTrue(any(call.startswith("clippy ") for call in host_calls), host_calls)
        self.assertTrue(all(call.startswith(("clippy ", "run --locked ")) for call in host_calls), host_calls)
        self.assertEqual(sum(call.startswith("run ") for call in host_calls), 1)
        self.assertIn("--manifest-path apps/conformance/Cargo.toml", host_calls[-1])
        # A failing regression suite must stop the local gate before invoking Cargo.
        self.env["DAY_TEST_SUITE_EXIT"] = "1"
        log.write_text("")
        self.run_cmd("bash", "scripts/ci/check-ready.sh", ok=False)
        self.assertTrue((self.root / "readiness-tests-called").exists())
        self.assertEqual(log.read_text(), "")
        self.env["DAY_TEST_SUITE_EXIT"] = "0"
        expected = ["fmt --all -- --check", *host_calls]
        # Fail each individual invocation, including later Clippy combinations, and require the
        # gate to stop there. A command-name failure only tested the first Clippy invocation.
        for fail_at in range(len(expected) + 1):
            with self.subTest(fail_at=fail_at):
                self.env["DAY_TEST_FAIL_AT"] = str(fail_at)
                log.write_text("")
                self.run_cmd("bash", "scripts/ci/check-ready.sh", ok=fail_at == 0)
                calls = log.read_text().splitlines()
                self.assertEqual(calls, expected[:fail_at] if fail_at else expected)
        self.env["DAY_TEST_FAIL_AT"] = "0"
        # The hook formats but leaves host Clippy to check-ready.sh and CI: it was most of the
        # commit time.
        for fail in ("fmt", "clippy", ""):
            self.env["DAY_TEST_FAIL"] = fail
            log.write_text("")
            self.run_cmd("bash", ".githooks/pre-commit", ok=fail != "fmt")
            self.assertEqual(log.read_text().splitlines(), ["fmt --all"])
        (self.root / "docs/duty-matrix.md").write_text("synthetic drift fixture\n")
        log.write_text("")
        self.run_cmd("bash", "scripts/ci/check-ready.sh", ok=False)
        self.assertEqual(log.read_text(), "")


if __name__ == "__main__":
    unittest.main()
