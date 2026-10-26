#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Compile the tutorial files in lesson order against a local framework fixture.

The app is fresh; installed toolchains and Cargo's dependency caches are reused.
No framework file, global installation, or other repository is modified.
The documented registry dependencies are replaced only in the isolated test copy;
this validates lesson code, not installation from crates.io or a fresh dependency cache.
"""
from pathlib import Path
import argparse
import hashlib
import json
import os
import shlex
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / '.cache/docs/tutorial'
PAGES = ROOT / 'website/src/content/docs'
RECIPE = json.loads(subprocess.check_output(
    ['node', str(ROOT / 'website/scripts/tutorial-export.mjs'), 'macos'], text=True))
LESSONS = [lesson['source'] for lesson in RECIPE['recipe']['lessons']]


def file_steps(lesson):
    lesson_id = next(item['id'] for item in RECIPE['recipe']['lessons'] if item['source'] == lesson)
    return [step for step in RECIPE['steps'] if step['key'].startswith(lesson_id + '/')
            and step['kind'] == 'file']


# Local compatibility checks use the same expectations as the nightly reader walkthrough.
SMOKE_STEPS = [step['checks'] for step in RECIPE['steps'] if step['kind'] == 'launch']


def run(name, args, cwd, env, logs):
    start = time.monotonic()
    with (logs / f'{name}.log').open('w') as log:
        log.write(f'cwd: {cwd}\ncommand: {shlex.join(map(str, args))}\n')
        log.flush()
        result = subprocess.run(args, cwd=cwd, env=env, stdout=log, stderr=subprocess.STDOUT)
        log.write(f'\nexit: {result.returncode}\nelapsed_seconds: {time.monotonic() - start:.2f}\n')
    print(f'{name}: exit {result.returncode}', flush=True)
    if result.returncode:
        print((logs / f'{name}.log').read_text()[-8000:], flush=True)
        raise SystemExit(result.returncode)


def local_framework_fixture(app):
    """Keep the reader manifest, then select local sources for compatibility validation."""
    manifest = app / 'Cargo.toml'
    source = manifest.read_text()
    (app / 'Cargo.toml.documented').write_text(source)
    for published, local in (
        ('day = { package = "day-app", version = "0.6" }',
         'day = { package = "day-app", path = "../../crates/day" }'),
        ('day-build = "0.6"', 'day-build = { path = "../../crates/day-build" }'),
    ):
        assert source.count(published) == 1, f'Unexpected tutorial dependency: {published}'
        source = source.replace(published, local)
    manifest.write_text(source)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-native', action='store_true', help='Also launch each lesson and run its AppKit walkthrough')
    args = parser.parse_args()
    # Never overwrite an existing run. Keep failed runs for diagnosis.
    session = WORK / ('samples-' + time.strftime('%Y%m%d-%H%M%S'))
    session.mkdir(parents=True)
    root_lock = (ROOT / 'Cargo.lock').read_bytes()
    logs = session / 'logs'
    logs.mkdir()
    app = session / 'apps/first-app'
    app.mkdir(parents=True)
    # Isolate the local dependency fixture; these paths are not reader instructions.
    for folder in ('crates',):
        (session / folder).symlink_to(ROOT / folder, target_is_directory=True)
    # Cargo resolves workspace membership lexically through the symlink paths.
    # Mirror the workspace manifest without copying or editing framework sources.
    shutil.copyfile(ROOT / 'Cargo.toml', session / 'Cargo.toml')
    shutil.copyfile(ROOT / 'Cargo.lock', session / 'Cargo.lock')
    (session / 'tmp').mkdir()
    env = {k: os.environ[k] for k in ('PATH', 'HOME', 'USER', 'LOGNAME', 'SHELL', 'DEVELOPER_DIR') if k in os.environ}
    env.update(CARGO_NET_OFFLINE='true', TMPDIR=str(session / 'tmp'),
               GIT_OPTIONAL_LOCKS='0', CARGO_TARGET_DIR=str(ROOT / 'target'),
               PATH=str(ROOT / 'target/debug') + os.pathsep + env.get('PATH', ''))
    hashes = {'website/tutorials/session-counter.yaml': hashlib.sha256(
        (ROOT / 'website/tutorials/session-counter.yaml').read_bytes()).hexdigest()}
    for index, lesson in enumerate(LESSONS, 1):
        source = (PAGES / lesson).read_text()
        for step in file_steps(lesson):
            dest = app / step['path']
            dest.parent.mkdir(parents=True, exist_ok=True)
            with dest.open('a' if step['mode'] == 'append' else 'w') as out:
                out.write(step['body'])
        hashes[lesson] = hashlib.sha256(source.encode()).hexdigest()
        if index == 1:
            local_framework_fixture(app)
            run('host-sources', ['day', 'project', 'add-target', 'macos-appkit'], app, env, logs)
        run(f'{index}-format', ['cargo', 'fmt', '--all', '--', '--check'], app, env, logs)
        run(f'{index}-build', ['day', 'build', '-t', 'macos-appkit'], app, env, logs)
        if args.run_native and index <= len(SMOKE_STEPS):
            fixture = app / 'dayscript/lesson-smoke.yaml'
            fixture.parent.mkdir(exist_ok=True)
            fixture.write_text(f'name: lesson-{index}\non_failure: stop\nflow:\n' +
                               ''.join('  - ' + json.dumps(step) + '\n' for step in SMOKE_STEPS[index - 1]))
            run(f'{index}-native', ['day', 'launch', '-t', 'macos-appkit', '--skip-build',
                                   '--locale', 'en', '--script', 'dayscript/lesson-smoke.yaml'], app, env, logs)
    run('clippy', ['cargo', 'clippy', '--offline', '--locked', '--no-default-features',
                  '--features', 'appkit', '--all-targets', '--', '-D', 'warnings'], app, env, logs)
    if args.run_native:
        run('native', ['day', 'launch', '-t', 'macos-appkit', '--locale', 'en',
                       '--script', 'dayscript/session-counter.yaml'], app, env, logs)
        for name, size in [('initial-size', 'window'), ('minimum-size', '800x640@2')]:
            run(name, ['day', 'launch', '-t', 'macos-appkit', '--skip-build', '--locale', 'en',
                       '--capture-size', size, '--script', 'dayscript/session-counter.yaml'], app, env, logs)
    if (ROOT / 'Cargo.lock').read_bytes() != root_lock:
        raise SystemExit('Root Cargo.lock changed during validation; original copy is in the session directory.')
    (session / 'sources.json').write_text(json.dumps(hashes, indent=2) + '\n')
    print(f'Validated six lessons. Evidence: {session}', flush=True)


if __name__ == '__main__':
    main()
