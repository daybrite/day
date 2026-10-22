#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Build and photograph the exact lesson-three app using the local AppKit backend."""
from pathlib import Path
import hashlib
import importlib.util
import json
import os
import shutil
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]


def main():
    spec = importlib.util.spec_from_file_location('tutorial', ROOT / 'scripts/docs/verify-tutorial.py')
    tutorial = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(tutorial)
    session = ROOT / '.cache/docs' / ('tutorial-capture-' + time.strftime('%Y%m%d-%H%M%S'))
    app = session / 'apps/first-app'
    app.mkdir(parents=True)
    logs = session / 'logs'
    logs.mkdir()
    for folder in ('crates', 'parts', 'pieces', 'toolkits', 'tweaks'):
        (session / folder).symlink_to(ROOT / folder, target_is_directory=True)
    for name in ('Cargo.toml', 'Cargo.lock'):
        shutil.copyfile(ROOT / name, session / name)
    lock = (ROOT / 'Cargo.lock').read_bytes()
    (session / 'root-lock.original').write_bytes(lock)
    (session / 'tmp').mkdir()
    env = {k: os.environ[k] for k in ('PATH', 'HOME', 'USER', 'LOGNAME', 'SHELL', 'DEVELOPER_DIR') if k in os.environ}
    env.update(CARGO_NET_OFFLINE='true', TMPDIR=str(session / 'tmp'),
               GIT_OPTIONAL_LOCKS='0', CARGO_TARGET_DIR=str(ROOT / 'target'),
               PATH=str(ROOT / 'target/debug') + os.pathsep + env.get('PATH', ''),
               DAY_DATA_DIR=str(session / 'data'), DAY_PREFS_DIR=str(session / 'prefs'))
    sources = {}
    for lesson in tutorial.LESSONS[:3]:
        source = (tutorial.PAGES / lesson).read_text()
        sources[str((tutorial.PAGES / lesson).relative_to(ROOT))] = hashlib.sha256(source.encode()).hexdigest()
        for match in tutorial.BLOCK.finditer(source):
            mode, name, body = match.groups()
            assert name in tutorial.ALLOWED
            dest = app / name
            dest.parent.mkdir(parents=True, exist_ok=True)
            with dest.open('a' if mode == 'append' else 'w') as out:
                out.write(body + '\n')
    tutorial.local_framework_fixture(app)
    script = app / 'dayscript/website.yaml'
    script.parent.mkdir(exist_ok=True)
    script.write_text('''name: website-counter
on_failure: stop
flow:
  - assert_text: { id: counter_label, key: counter_value, args: { count: 0 } }
  - screenshot: counter-start
  - tap: { id: increment_button }
  - tap: { id: increment_button }
  - assert_text: { id: counter_label, key: counter_value, args: { count: 2 } }
  - screenshot: counter-two
''')
    def run(name, args):
        tutorial.run(name, args, app, env, logs)
    try:
        run('host-sources', ['day', 'project', 'add-target', 'macos-appkit'])
        run('format', ['cargo', 'fmt', '--all', '--', '--check'])
        run('build', ['day', 'build', '-t', 'macos-appkit'])
        run('clippy', ['cargo', 'clippy', '--offline', '--locked', '--no-default-features',
                       '--features', 'appkit', '--all-targets', '--', '-D', 'warnings'])
        run('capture', ['day', 'launch', '-t', 'macos-appkit', '--skip-build', '--locale', 'en',
                        '--themes', 'light', '--capture-size', '800x640@2', '--script', 'dayscript/website.yaml'])
        captures = list((app / 'build/day/screenshots').rglob('*.png'))
        assert len(captures) == 2, captures
        evidence = {
            'captured_at': time.strftime('%Y-%m-%dT%H:%M:%S%z'),
            'framework_head': subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT).decode().strip(),
            'framework_state': 'working tree; see framework_hashes, not HEAD alone',
            'framework_hashes': {str(p.relative_to(ROOT)): hashlib.sha256(p.read_bytes()).hexdigest()
                                 for folder in ('crates', 'toolkits', 'parts', 'pieces', 'tweaks')
                                 for p in (ROOT / folder).rglob('*') if p.is_file() and p.suffix in ('.rs', '.toml', '.m', '.mm', '.cpp', '.h', '.swift')},
            'tutorial_sources': sources,
            'app_files': {str(p.relative_to(app)): hashlib.sha256(p.read_bytes()).hexdigest()
                          for folder in ('src', 'resource', 'dayscript') for p in (app / folder).rglob('*') if p.is_file()},
            'target': 'macos-appkit', 'locale': 'en', 'theme': 'light',
            'capture_size': '800x640@2', 'session': str(session.relative_to(ROOT)),
            'captures': {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in captures},
        }
        (session / 'evidence.json').write_text(json.dumps(evidence, indent=2) + '\n')
        print(f'Capture evidence: {session}', flush=True)
    finally:
        assert (ROOT / 'Cargo.lock').read_bytes() == lock, 'Root lockfile changed; backup retained'


if __name__ == '__main__':
    main()
