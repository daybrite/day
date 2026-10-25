#!/usr/bin/env python3
# Copyright © The Daybrite Project
# SPDX-License-Identifier: MPL-2.0
"""Compile exact priority-guide excerpts with explicit synthetic supporting fixtures.

This complements verify-recipes.py's complete modules. Excerpts remain fragments,
not runnable apps. The check does not execute examples or show native prompts.
Cargo uses cached dependencies by default; CARGO_NET_OFFLINE=false allows dependency downloads.
"""
from pathlib import Path
import hashlib
import json
import os
import re
import shutil
import struct
import subprocess
import time

ROOT = Path(__file__).resolve().parents[2]
PAGES = ['reactivity', 'layout', 'navigation', 'resources', 'localization',
         'accessibility', 'guide-storage', 'guide-http', 'guide-permissions',
         'guide-commands', 'packaging']
RUST = re.compile(r'^```rust\n(.*?)\n```', re.M | re.S)
FTL = re.compile(r'^```ftl\n(.*?)\n```', re.M | re.S)


def blocks(page):
    return list(RUST.finditer(page))


def digest(data):
    return hashlib.sha256(data).hexdigest()


def put(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def font_fixture():
    # A name-table-only fixture, matching day-fonts' unit-test construction.
    # It tests generated FontFamily names; it is not a renderable Pacifico font.
    value = 'Pacifico'.encode('utf-16-be')
    table = struct.pack('>HHH', 0, 1, 18) + struct.pack('>HHHHHH', 3, 1, 0x409, 1, len(value), 0) + value
    return struct.pack('>IHHHH', 0x10000, 1, 0, 0, 0) + b'name' + struct.pack('>III', 0, 28, len(table)) + table


def main():
    session = ROOT / '.cache/docs' / ('priority-excerpts-' + time.strftime('%Y%m%d-%H%M%S'))
    session.mkdir(parents=True)
    lock = (ROOT / 'Cargo.lock').read_bytes()
    (session / 'root-lock.original').write_bytes(lock)
    for name in ('Cargo.toml', 'Cargo.lock'):
        shutil.copyfile(ROOT / name, session / name)
    for folder in ('crates',):
        (session / folder).symlink_to(ROOT / folder, target_is_directory=True)
    app = session / 'apps/excerpts'
    texts = {name: (ROOT / f'website/src/content/docs/{name}.md').read_text() for name in PAGES}
    catalog = (ROOT / 'docs/snippet-catalog.md').read_text()
    setup = dict(re.findall(r'<!-- technical-file: ([\w./-]+) -->\n```\w+\n(.*?)\n```', catalog, re.S))
    manifest = setup['Cargo.toml'].replace('[build-dependencies]', '''day-core = { path = "../../crates/day-core" }
serde_json = "1"

[build-dependencies]''')
    put(app / 'Cargo.toml', manifest + '\n')
    put(app / 'Day.toml', setup['Day.toml'] + '\n')
    loc_blocks = blocks(texts['localization'])
    put(app / 'build.rs', loc_blocks[0][1] + '\n')
    put(app / 'src/lib.rs', loc_blocks[1][1] + '\n')

    # Use real documented messages. Last definition wins only when merging fixture
    # catalogs: guides deliberately reuse common example keys such as app_title.
    messages = {}
    for text in [catalog, *texts.values()]:
        for match in FTL.finditer(text):
            for entry in re.finditer(r'^(-?[\w-]+)\s*=.*?(?=^-?[\w-]+\s*=|\Z)', match[1], re.M | re.S):
                messages[entry[1]] = entry[0].rstrip()
    put(app / 'resource/locales/en/app.ftl', '\n'.join(messages.values()) + '\n')
    put(app / 'resource/assets/stations.json', '{"stations": []}\n')
    for name in ('wave', 'hero_banner'):
        dest = app / f'resource/images/{name}.png'
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(ROOT / 'crates/day-cli/resources/icons/day-icon-48.png', dest)
    put(app / 'resource/vectors/home.svg', '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><path d="M0 0h24v24H0z"/></svg>\n')
    dest = app / 'resource/fonts/fixture.ttf'
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_bytes(font_fixture())

    # Compile the private catalog's exact build script, include macro, accessor
    # excerpt, and the two documented FTL files in their own crate namespace.
    private = session / 'apps/blockblast'
    put(private / 'Cargo.toml', '''[package]
name = "blockblast"
version = "0.1.0"
edition = "2024"
[workspace]
[dependencies]
day-localization-ui = { path = "../../crates/day-localization-ui" }
[build-dependencies]
day-build = { path = "../../crates/day-build" }
''')
    put(private / 'build.rs', loc_blocks[6][1] + '\n')
    put(private / 'src/lib.rs', loc_blocks[7][1] + '\n#[allow(dead_code)]\nfn excerpt() {\n' + loc_blocks[8][1] + '\n}\n')
    for filename in ('app', 'board'):
        found = [m[1] for m in FTL.finditer(texts['localization']) if m[1].startswith(f'# resource/locales/en/{filename}.ftl')]
        put(private / f'resource/locales/en/{filename}.ftl', found[-1] + '\n')

    helpers = '''#![allow(dead_code, unused_imports, unused_variables)]
// Synthetic supporting types and functions for integration-fragment compilation.
use day::prelude::*;
use day_core::{LayoutOps, RNode};
use dayapp::res;
type Stations = serde_json::Value;
#[derive(Clone)]
struct Room { id: String, name: String }
fn home_page() -> day::AnyPiece { label(res::str::nav_home()).any() }
fn library_page() -> day::AnyPiece { label(res::str::nav_library()).any() }
fn settings_page() -> day::AnyPiece { label(res::str::nav_settings()).any() }
fn detail_page(_: &String) -> day::AnyPiece { library_page() }
fn room_page(_: &String) -> day::AnyPiece { library_page() }
fn editor() -> day::AnyPiece { library_page() }
fn detail(_: &String) -> day::AnyPiece { library_page() }
fn album_page<K>(_: K, _: Option<String>) -> day::AnyPiece { library_page() }
fn track_page(_: u32) -> day::AnyPiece { library_page() }
fn draw_gauge(_: &mut Draw, _: Size, _: f64) {}
day::routes! { pub enum Section { Home => "home", Library => "library", Settings => "settings" } }
'''
    # The typed-route excerpt supplies its own one-argument album_page helper.
    typed = blocks(texts['navigation'])[8][1].split('let path =')[0]
    helpers += typed + '\n'
    prelude = {
        ('layout', 1): 'let enabled = Signal::new(false);',
        ('layout', 2): '\n'.join(f'let {n} = spacer();' for n in ('a','b','c','x','y','photo','badge','long_column')),
        ('navigation', 4): 'let current = Signal::new(String::new()); let rooms = Signal::new(Vec::<Room>::new());',
        ('navigation', 5): 'let path = Signal::new(Vec::<String>::new()); let dirty = Signal::new(false);',
        ('navigation', 6): 'let path = Signal::new(Vec::<String>::new()); let root = home_page();',
        ('navigation', 7): 'let section = Signal::new(String::from("home")); let path = Signal::new(Vec::<String>::new()); let home = home_page();',
        ('navigation', 9): 'fn album_page(_: u32) -> day::AnyPiece { library_page() }',
        ('resources', 4): 'let accent = Color::rgb(0.2, 0.4, 0.6);',
        ('localization', 5): 'let count = Signal::new(3_i64); let folder = Signal::new(String::from("Fixture folder"));',
        ('localization', 6): 'let finished = false;',
        ('accessibility', 1): 'let value = Signal::new(42.0_f64);',
        ('guide-commands', 3): 'let star = Command::new(id!(fixture_star), res::str::star(), || {});',
    }
    records = []
    for name, text in texts.items():
        for index, match in enumerate(blocks(text), 1):
            prefix = text[max(0, match.start()-100):match.start()]
            record = {'source': f'website/src/content/docs/{name}.md', 'block': index,
                      'line': text[:match.start()].count('\n') + 1, 'sha256': digest(match[1].encode())}
            if '<!-- technical-example:' in prefix:
                record['validation'] = 'complete module: verify-recipes.py'
            elif name == 'localization' and index in (1, 2, 7, 8, 9):
                record['validation'] = 'exact setup/private-catalog excerpt'
            else:
                record['validation'] = 'exact fragment with synthetic context; compile only'
                helpers += f'\nfn {name.replace("-", "_")}_{index}() -> Result<(), Box<dyn std::error::Error>> {{\n'
                helpers += prelude.get((name, index), '') + '\nlet _ = {\n' + match[1] + '\n};\nOk(())\n}\n'
            records.append(record)
    put(app / 'tests/excerpts.rs', helpers)
    env = dict(os.environ, CARGO_NET_OFFLINE=os.environ.get('CARGO_NET_OFFLINE', 'true'),
               CARGO_TARGET_DIR=os.environ.get('CARGO_TARGET_DIR', str(ROOT / 'target')),
               GIT_OPTIONAL_LOCKS='0')
    result = 0
    try:
        for name, project in [('app', app), ('private', private)]:
            shutil.copyfile(ROOT / 'Cargo.lock', project / 'Cargo.lock')
            # Day comes from this checkout; the environment controls third-party downloads.
            for action, command in [('resolve', ['cargo', 'generate-lockfile']),
                                    ('check', ['cargo', 'check', '--locked', '--all-targets'])]:
                with (session / f'{name}-{action}.log').open('w') as log:
                    log.write(f'cwd: {project}\ncommand: {command}\n'); log.flush()
                    run = subprocess.run(command, cwd=project, env=env, stdout=log, stderr=subprocess.STDOUT)
                    log.write(f'\nexit: {run.returncode}\n')
                print(f'{name}-{action}: {run.returncode}', flush=True)
                if run.returncode:
                    print((session / f'{name}-{action}.log').read_text()[-10000:])
                    return run.returncode
        put(session / 'excerpts.json', json.dumps(records, indent=2) + '\n')
        print(f'{len(records)} Rust blocks accounted for; evidence: {session}')
    finally:
        if (ROOT / 'Cargo.lock').read_bytes() != lock:
            raise SystemExit('Root lock changed; original backup retained in the session directory')
    return result


if __name__ == '__main__':
    raise SystemExit(main())
