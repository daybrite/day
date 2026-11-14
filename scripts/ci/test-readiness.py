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
        # The website suite is exercised separately. Observe readiness's invocation and
        # propagate a synthetic failure without installing dependencies in each fixture.
        (self.root / "scripts/ci/check-website-source.sh").write_text(
            '#!/bin/sh\ntouch website-check-called\nexit "${DAY_TEST_WEBSITE_EXIT:-0}"\n'
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
        self.run_cmd("git", "rm", "--force", "crates/day-piece-text-editor/Cargo.toml")
        result = self.run_cmd("bash", "scripts/ci/check-matrices.sh", "--staged", ok=False)
        self.assertIn("day-piece-text-editor", result.stderr)
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
        manifest = self.root / "crates/day-platform-android/Cargo.toml"
        original = manifest.read_text()
        manifest.write_text(original.replace("[dependencies]", "[dependencies]\nday-toolkit-material.workspace = true"))
        result = self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py", ok=False)
        self.assertIn("day-platform-android -> day-toolkit-material", result.stderr)
        manifest.write_text(original)
        java = self.root / "crates/day-part-sensors/src/DaySensors.java"
        java.write_text(java.read_text() + "\nimport dev.daybrite.day.bridge.DayBridge;\n")
        result = self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py", ok=False)
        self.assertIn("DaySensors.java", result.stderr)

    def test_windows_runtime_rejects_ui_dependencies(self):
        manifest = self.root / "crates/day-platform-windows/Cargo.toml"
        original = manifest.read_text()
        for dependency in ("day-toolkit-winui", "day-toolkit-xaml-sys", "day-core"):
            with self.subTest(dependency=dependency):
                # Under the dependency table, not at the end of the file: the manifest closes
                # with `[lints]`, where an appended line would be a lint setting.
                table = "[target.'cfg(windows)'.dependencies]"
                self.assertIn(table, original)
                manifest.write_text(
                    original.replace(table, f"{table}\n{dependency}.workspace = true")
                )
                result = self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py", ok=False)
                self.assertIn(f"day-platform-windows -> {dependency}", result.stderr)
        manifest.write_text(original)
        self.run_cmd("python3", "scripts/ci/check-platform-runtimes.py")

    def test_website_source_checks_rebuild_api_before_coverage(self):
        # Synthetic tools exercise the shared entry point without building a second site.
        script = self.root / "scripts/ci/check-website-source.sh"
        script.write_text((ROOT / "scripts/ci/check-website-source.sh").read_text())
        modules = self.root / "website/node_modules"
        # A local dependency symlink can be copied as an untracked fixture input.
        if modules.is_symlink():
            modules.unlink()
        modules.mkdir(exist_ok=True)
        (self.root / "scripts/ci/docs-symlinks.sh").write_text("exit 0\n")
        (self.root / "scripts/ci/doc-links.py").write_text("pass\n")
        (self.root / "scripts/build-api-docs.sh").write_text("touch rebuilt-api\n")
        bindir = self.root / "test-bin"
        bindir.mkdir()
        (bindir / "npm").write_text('#!/bin/sh\necho "$*" >> npm.log\n')
        (bindir / "node").write_text(
            '#!/bin/sh\n'
            'test -f rebuilt-api || exit 2\n'
            'test "$*" = website/scripts/check-api-references.mjs || exit 3\n'
            'touch checked-api\n'
            'exit "${DAY_TEST_API_EXIT:-0}"\n'
        )
        for tool in ("npm", "node"):
            (bindir / tool).chmod(0o755)
        self.env["PATH"] = str(bindir) + os.pathsep + self.env["PATH"]
        self.run_cmd("bash", "scripts/ci/check-website-source.sh")
        self.assertTrue((self.root / "checked-api").exists())
        self.assertEqual((self.root / "npm.log").read_text().splitlines(), [
            f"--prefix website run {name}" for name in (
                "docs:check", "docs:test", "blog:test", "llms:test", "strings:check",
                "docs:architecture",
            )
        ])
        self.env["DAY_TEST_API_EXIT"] = "1"
        self.run_cmd("bash", "scripts/ci/check-website-source.sh", ok=False)
        shutil.rmtree(modules)
        result = self.run_cmd("bash", "scripts/ci/check-website-source.sh", ok=False)
        self.assertIn("npm --prefix website ci", result.stderr)
        workflow = (self.root / ".github/workflows/ci.yml").read_text().split("\n  website:\n")[1]
        self.assertLess(
            workflow.index("bash scripts/ci/check-website-source.sh"),
            workflow.index("run: npm run build:site"),
        )

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
        self.assertTrue(
            all(call.startswith(("metadata --locked ", "clippy ", "run --locked "))
                for call in host_calls),
            host_calls,
        )
        for fixture in ("element-ids", "conformance"):
            manifest = f"--manifest-path crates/day-core/tests/fixtures/{fixture}/Cargo.toml"
            self.assertEqual(
                sum(call.startswith("metadata --locked ") and manifest in call
                    for call in host_calls),
                1,
                host_calls,
            )
        self.assertEqual(sum(call.startswith("run ") for call in host_calls), 1)
        self.assertIn("--manifest-path apps/conformance/Cargo.toml", host_calls[-1])
        # A failing regression suite must stop the local gate before invoking Cargo.
        self.env["DAY_TEST_SUITE_EXIT"] = "1"
        log.write_text("")
        result = self.run_cmd("bash", "scripts/ci/check-ready.sh", ok=False)
        self.assertTrue(
            (self.root / "readiness-tests-called").exists(), result.stdout + result.stderr
        )
        self.assertEqual(log.read_text(), "")
        self.env["DAY_TEST_SUITE_EXIT"] = "0"
        expected = ["fmt --all -- --check", *host_calls]
        # Fail each invocation, including fixture resolution and later Clippy combinations, and
        # require the gate to stop there. A command-name failure only tested the first Clippy call.
        for fail_at in range(len(expected) + 1):
            with self.subTest(fail_at=fail_at):
                website_called = self.root / "website-check-called"
                website_called.unlink(missing_ok=True)
                self.env["DAY_TEST_FAIL_AT"] = str(fail_at)
                log.write_text("")
                self.run_cmd("bash", "scripts/ci/check-ready.sh", ok=fail_at == 0)
                calls = log.read_text().splitlines()
                self.assertEqual(calls, expected[:fail_at] if fail_at else expected)
                self.assertEqual(website_called.exists(), fail_at == 0)
        self.env["DAY_TEST_FAIL_AT"] = "0"
        self.env["DAY_TEST_WEBSITE_EXIT"] = "1"
        self.run_cmd("bash", "scripts/ci/check-ready.sh", ok=False)
        self.env["DAY_TEST_WEBSITE_EXIT"] = "0"
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
