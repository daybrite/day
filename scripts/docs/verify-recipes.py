#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Extract and compile every designated technical recipe against this working tree.

Recipe tests make no network requests, platform prompts, or user-store writes.
Cargo uses cached dependencies by default; CARGO_NET_OFFLINE=false allows dependency downloads.
The independent app has its own lock; the repository lock is checked before and after.
"""
from pathlib import Path
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
WORK = ROOT / '.cache/docs/recipes'
BLOCK = re.compile(r'<!-- technical-(example|catalog): ([a-z_]+) -->\n```(?:rust|ftl)\n(.*?)\n```', re.S)


def main():
    session = WORK / ('recipes-' + time.strftime('%Y%m%d-%H%M%S'))
    session.mkdir(parents=True)
    app = session / 'apps/technical-recipes'
    (app / 'src').mkdir(parents=True)
    (app / 'resource/locales/en').mkdir(parents=True)
    (session / 'tmp').mkdir()
    lock = (ROOT / 'Cargo.lock').read_bytes()
    (session / 'root-lock.original').write_bytes(lock)
    for folder in ('crates',):
        (session / folder).symlink_to(ROOT / folder, target_is_directory=True)
    shutil.copyfile(ROOT / 'Cargo.toml', session / 'Cargo.toml')
    shutil.copyfile(ROOT / 'Cargo.lock', session / 'Cargo.lock')
    setup = (ROOT / 'docs/snippet-catalog.md').read_text()
    files = re.findall(r'<!-- technical-file: ([\w./-]+) -->\n```\w+\n(.*?)\n```', setup, re.S)
    expected = {'Cargo.toml', 'Day.toml', 'build.rs', 'src/lib.rs'}
    if {name for name, _ in files} != expected:
        raise SystemExit('Missing or unexpected recipe setup blocks')
    for name, body in files:
        (app / name).write_text(body + '\n')
    modules, catalogs, hashes = {}, {}, {}
    # Scan canonical sources, not publishing symlinks. New designations cannot escape the check.
    candidates = sorted((ROOT / 'website/src/content/docs').rglob('*.md')) + sorted((ROOT / 'docs').rglob('*.md'))
    for source in candidates:
        text = source.read_text()
        for name, body in re.findall(r'<!-- technical-resource: ([\w./-]+) -->\n```\w+\n(.*?)\n```', text, re.S):
            if not name.startswith('resource/assets/') or '..' in Path(name).parts:
                raise SystemExit(f'Unexpected resource fixture path: {name}')
            dest = app / name
            if dest.exists():
                raise SystemExit(f'Duplicate resource fixture: {name}')
            dest.parent.mkdir(parents=True, exist_ok=True)
            dest.write_text(body + '\n')
        matches = list(BLOCK.finditer(text))
        if len(matches) != text.count('<!-- technical-example:') + text.count('<!-- technical-catalog:'):
            raise SystemExit(f'Invalid marker: {source}')
        for match in matches:
            kind, name, body = match.groups()
            destination = modules if kind == 'example' else catalogs
            if name in destination:
                raise SystemExit(f'Duplicate {kind}: {name}')
            destination[name] = body
            hashes[str(source.relative_to(ROOT))] = hashlib.sha256(source.read_bytes()).hexdigest()
    if not modules or modules.keys() != catalogs.keys():
        raise SystemExit('Every designated module needs a matching catalog block')
    for name, body in modules.items():
        (app / f'src/{name}.rs').write_text(body + '\n')
    declared = set(re.findall(r'^pub mod (\w+);', (app / 'src/lib.rs').read_text(), re.M))
    if declared != modules.keys():
        raise SystemExit(f'Setup modules {declared} differ from recipes {modules.keys()}')
    (app / 'resource/locales/en/app.ftl').write_text('app_title = Technical recipes\n' + '\n'.join(catalogs.values()) + '\n')
    hashes['docs/snippet-catalog.md'] = hashlib.sha256((ROOT / 'docs/snippet-catalog.md').read_bytes()).hexdigest()
    (session / 'sources.json').write_text(json.dumps(hashes, indent=2) + '\n')
    # Copying gives Cargo a stable starting resolution; only the extracted app may update it.
    shutil.copyfile(ROOT / 'Cargo.lock', app / 'Cargo.lock')
    env = {k: os.environ[k] for k in ('PATH', 'HOME', 'USER', 'LOGNAME', 'SHELL',
                                    'CARGO_HOME', 'RUSTUP_HOME', 'RUSTFLAGS',
                                    'CARGO_ENCODED_RUSTFLAGS', 'CARGO_INCREMENTAL') if k in os.environ}
    env.update(CARGO_NET_OFFLINE=os.environ.get('CARGO_NET_OFFLINE', 'true'),
               CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')),
               TMPDIR=str(session / 'tmp'), GIT_OPTIONAL_LOCKS='0',
               DAY_DATA_DIR=str(session / 'data'), DAY_PREFS_DIR=str(session / 'prefs'),
               DAY_ASSET_ROOT=str(app / 'resource/assets'))
    # These are synthetic, side-effect-bounded regression fixtures, not shipped UI.
    (app / 'tests').mkdir()
    (app / 'tests/contracts.rs').write_text('''use day::prelude::*;
use dayapp::{commands, storage};

#[test]
fn command_guard_and_preference_round_trip() {
    let count = Signal::new(9_i64);
    let command = commands::increment_command(count);
    assert!(command.invoke());
    assert_eq!(count.get(), 10);
    assert!(!command.invoke());
    assert!(storage::save_setting(true));
    assert_eq!(day::prefs::get("settings.reminders").as_deref(), Some("true"));
}

#[test]
fn file_paths_and_round_trip() {
    assert!(matches!(day_part_filesystem::read("../outside"), Err(day_part_filesystem::FsError::BadPath)));
    day_part_filesystem::write("fixture/data.txt", b"synthetic fixture").unwrap();
    assert_eq!(day_part_filesystem::read("fixture/data.txt").unwrap(), b"synthetic fixture");
    assert_eq!(day_part_filesystem::list("fixture").unwrap(), vec!["data.txt"]);
}
''')
    # Test-only dependencies and synthetic fixtures do not alter the published modules.
    with (app / 'Cargo.toml').open('a') as manifest:
        manifest.write('\n[dev-dependencies]\n'
                       'day-core = { path = "../../crates/day-core" }\n'
                       'day-spec = { path = "../../crates/day-spec" }\n'
                       'day-mock = { path = "../../crates/day-mock" }\n')
    shutil.copyfile(ROOT / 'scripts/docs/fixtures/priority-recipes.rs', app / 'tests/priority.rs')
    commands_to_run = [
        # Let CARGO_NET_OFFLINE control downloads; --offline would override CI's false.
        # Day dependencies remain paths into this checkout, including on a fresh runner.
        ('resolve', ['cargo', 'generate-lockfile']),
        ('format', ['cargo', 'fmt', '--all']),
        ('format-check', ['cargo', 'fmt', '--all', '--', '--check']),
        ('check', ['cargo', 'check', '--locked', '--all-targets']),
        ('clippy', ['cargo', 'clippy', '--locked', '--all-targets', '--', '-D', 'warnings']),
        ('test', ['cargo', 'test', '--locked', '--test', 'contracts']),
        ('priority-tests', ['cargo', 'test', '--locked', '--test', 'priority', '--', '--test-threads=1']),
    ]
    result = 0
    try:
        for name, command in commands_to_run:
            with (session / f'{name}.log').open('w') as log:
                log.write(f'cwd: {app}\ncommand: {" ".join(command)}\n'); log.flush()
                completed = subprocess.run(command, cwd=app, env=env, stdout=log, stderr=subprocess.STDOUT)
                log.write(f'\nexit: {completed.returncode}\n')
            print(f'{name}: {completed.returncode}', flush=True)
            if completed.returncode:
                print((session / f'{name}.log').read_text()[-7000:])
                result = completed.returncode
                break
        # Formatting must not silently alter any designated Rust source.
        for name, body in files:
            if name.endswith('.rs') and (app / name).read_text() != body + '\n':
                print(f'FAIL: format the authored setup file {name}')
                result = 1
        for name, body in modules.items():
            if (app / f'src/{name}.rs').read_text() != body + '\n':
                print(f'FAIL: format the authored module {name} to match {app / "src" / (name + ".rs")}')
                result = 1
    finally:
        if (ROOT / 'Cargo.lock').read_bytes() != lock:
            raise SystemExit('Repository Cargo.lock changed; preserve and investigate before further work')
    print(f'{len(modules)} exact modules; logs: {session}')
    return result


if __name__ == '__main__':
    sys.exit(main())
