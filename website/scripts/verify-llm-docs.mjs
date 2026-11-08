// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import { createHash } from 'node:crypto';
import { readFile, writeFile, mkdir, readdir, rm, copyFile } from 'node:fs/promises';
import { createWriteStream } from 'node:fs';
import { spawn, execFileSync } from 'node:child_process';
import { join, dirname, relative, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { parse, stringify } from 'smol-toml';
import { generateLlmDocs, repository } from '../src/lib/llm-docs.mjs';
import { readRecipe, platformIds, lessonFiles, resolvePlan } from '../src/lib/tutorials.mjs';
import { mainCompatibility } from './tutorial-walkthrough.mjs';

const root = repository;
const cache = join(root, '.cache/docs/llm');
const fixture = join(root, 'scripts/docs/fixtures/llm-tutorial.rs');
const sha = (value) => createHash('sha256').update(value).digest('hex');

async function files(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const paths = [];
  for (const entry of entries.sort((a, b) => a.name.localeCompare(b.name))) {
    if (['target', 'build', 'node_modules', '.git', '.cache', '__pycache__'].includes(entry.name)) continue;
    const path = join(directory, entry.name);
    if (entry.isDirectory()) paths.push(...await files(path));
    else if (entry.isFile()) paths.push(path);
  }
  return paths;
}

export async function validationFingerprint() {
  const inputs = [join(root, 'Cargo.toml'), join(root, 'Cargo.lock'),
    ...await files(join(root, '.cargo')).catch((error) => { if (error.code === 'ENOENT') return []; throw error; }),
    ...await files(join(root, 'crates')),
    ...await files(join(root, 'website/tutorials')),
    ...['src/lib/llm-docs.mjs', 'src/lib/llm-routes.mjs', 'src/lib/tutorials.mjs',
      'src/lib/content-routes.mjs', 'src/data/llm-docs.json', 'src/data/content-routes.json',
      'src/data/route-migrations.json', 'plugins/content-links.mjs',
      'scripts/verify-llm-docs.mjs', 'scripts/tutorial-walkthrough.mjs'].map((path) => join(root, 'website', path)),
    join(root, 'website/package-lock.json'), fixture];
  const docs = generateLlmDocs();
  const hash = createHash('sha256').update(docs.index).update(docs.full);
  for (const path of [...new Set(inputs)].sort()) hash.update(relative(root, path)).update('\0').update(await readFile(path));
  const toolchain = execFileSync('rustc', ['-vV'], { encoding: 'utf8' });
  hash.update(toolchain).update(process.platform).update(process.arch);
  for (const key of ['RUSTFLAGS', 'CARGO_ENCODED_RUSTFLAGS', 'RUSTUP_TOOLCHAIN', 'CARGO_BUILD_TARGET']) hash.update(`${key}=${process.env[key] ?? ''}\0`);
  return { fingerprint: hash.digest('hex'), toolchain, docs };
}

async function run(command, args, directory, logFile, env) {
  const log = createWriteStream(logFile);
  log.write(`cwd: ${directory}\ncommand: ${command} ${args.join(' ')}\n`);
  try {
    const result = await new Promise((resolve, reject) => {
      const child = spawn(command, args, { cwd: directory, env, stdio: ['ignore', 'pipe', 'pipe'] });
      child.stdout.pipe(log, { end: false }); child.stderr.pipe(log, { end: false });
      child.once('error', reject);
      child.once('close', (code) => resolve(code));
    });
    if (result !== 0) throw new Error(`${command} ${args.join(' ')} failed (${result}); see ${logFile}`);
  } finally { await new Promise((resolve) => log.end(resolve)); }
}

/** A new executable file must acquire a validator instead of silently escaping compilation. */
export function validateTutorialFiles(fileMap) {
  const known = new Set(['Cargo.toml', 'Day.toml', 'build.rs', 'src/lib.rs', 'src/main.rs',
    'resource/locales/en/app.ftl', 'dayscript/session-counter.yaml']);
  for (const path of fileMap.keys()) if (!known.has(path)) throw new Error(`Add validation for tutorial file: ${path}`);
  for (const path of ['Cargo.toml', 'Day.toml', 'build.rs', 'src/lib.rs', 'src/main.rs', 'resource/locales/en/app.ftl']) {
    if (!fileMap.has(path)) throw new Error(`Incomplete tutorial project: ${path}`);
  }
  parse(fileMap.get('Day.toml'));
  const manifest = parse(fileMap.get('Cargo.toml'));
  if (JSON.stringify(manifest.features?.default) !== '["mock"]') throw new Error('Tutorial compilation requires the documented mock default feature');
  return manifest;
}

export function tutorialCommands(recipe) {
  const lines = new Set(platformIds.flatMap((platform) => resolvePlan(recipe, platform))
    .filter((step) => ['run', 'launch'].includes(step.kind)).flatMap((step) => step.body.trim().split('\n')));
  // These setup commands are exercised by the native walkthrough runner. Keep an explicit
  // list so a newly introduced shell operation cannot silently inherit validation coverage.
  const setup = new Set(["cargo install day-cli --locked", "cargo add 'day-app@0.*' --rename day",
    "cargo add --build 'day-build@0.*'", 'mkdir -p first-app/src first-app/resource/locales/en',
    'cd first-app', 'mkdir -p dayscript',
    'New-Item -ItemType Directory -Path first-app/src, first-app/resource/locales/en',
    'Set-Location first-app', 'New-Item -ItemType Directory -Force -Path dayscript']);
  return [...lines].map((line) => {
    if (setup.has(line)) return { command: line, validation: 'native walkthrough CI' };
    if (/^day(?: [\w./-]+)+$/.test(line)) return { command: line, validation: 'CLI parser and native walkthrough CI' };
    throw new Error(`Add instruction validation for tutorial command: ${line}`);
  });
}

export async function verifyLlmDocs({ force = false } = {}) {
  const before = await validationFingerprint();
  const receiptPath = join(cache, 'passed.json');
  const previous = JSON.parse(await readFile(receiptPath, 'utf8').catch(() => 'null'));
  if (!force && previous?.fingerprint === before.fingerprint && previous.status === 'pass') {
    console.log('[llm-docs] compilation and interaction checks match the documentation and framework sources');
    return previous;
  }
  await mkdir(cache, { recursive: true });
  // No partial run can leave a success receipt behind.
  await rm(receiptPath, { force: true });
  const session = join(cache, `${before.fingerprint.slice(0, 12)}-${process.pid}-${Date.now()}`);
  const app = join(session, 'app');
  await mkdir(app, { recursive: true });
  const recipe = readRecipe();
  const commands = tutorialCommands(recipe);
  for (const platform of platformIds) mainCompatibility(resolvePlan(recipe, platform), root);
  for (const lesson of recipe.lessons) {
    const rustPaths = new Set();
    for (const step of lesson.steps) {
      if (step.kind !== 'file' || !step.path.endsWith('.rs')) continue;
      if (rustPaths.has(step.path)) throw new Error(`Add intermediate compilation for ${lesson.id}/${step.path}`);
      rustPaths.add(step.path);
    }
  }
  const env = { ...process.env,
    CARGO_NET_OFFLINE: process.env.CARGO_NET_OFFLINE ?? 'true',
    CARGO_TARGET_DIR: process.env.CARGO_TARGET_DIR ?? join(root, 'target'),
    DAY_DATA_DIR: join(session, 'data'), DAY_PREFS_DIR: join(session, 'prefs'),
  };
  const records = [];
  // --help checks syntax against this checkout's actual CLI without installing packages,
  // launching native apps, or writing to the reader's project. Native CI executes the commands.
  for (const [index, entry] of commands.entries()) {
    if (!entry.command.startsWith('day ')) continue;
    const args = entry.command.split(' ').slice(1);
    await run('cargo', ['run', '--locked', '-p', 'day-cli', '--', ...args, '--help'], root, join(session, `command-${index}.log`), env);
  }
  let resolved = false;
  for (const platform of platformIds) {
    for (let index = 0; index < recipe.lessons.length; index++) {
      const lesson = recipe.lessons[index];
      const authored = lessonFiles(recipe, platform, index + 1);
      const manifest = validateTutorialFiles(authored);
      // These are the same substitutions as native main-compatibility CI: the public
      // cargo-add commands resolve Day from this checkout instead of crates.io.
      manifest.dependencies = { ...manifest.dependencies, day: { package: 'day-app', path: join(root, 'crates/day') } };
      manifest['build-dependencies'] = { ...manifest['build-dependencies'], 'day-build': { path: join(root, 'crates/day-build') } };
      manifest['dev-dependencies'] = Object.fromEntries(['day-core', 'day-mock', 'day-script', 'day-script-protocol'].map((name) => [name, { path: join(root, 'crates', name) }]));
      await rm(join(app, 'dayscript'), { recursive: true, force: true });
      for (const [path, content] of authored) {
        await mkdir(dirname(join(app, path)), { recursive: true });
        await writeFile(join(app, path), content);
      }
      await writeFile(join(app, 'Cargo.toml'), stringify(manifest));
      await mkdir(join(app, 'tests'), { recursive: true });
      await copyFile(fixture, join(app, 'tests/tutorial.rs'));
      const checks = resolvePlan({ ...recipe, lessons: [lesson] }, platform).filter((s) => s.kind === 'launch').flatMap((s) => s.checks);
      if (!checks.length) throw new Error(`No interaction assertions for ${lesson.id}`);
      await writeFile(join(app, 'tests/steps.json'), JSON.stringify({ flow: checks }));
      const prefix = join(session, `${platform}-${lesson.id}`);
      if (!resolved) {
        await copyFile(join(root, 'Cargo.lock'), join(app, 'Cargo.lock'));
        await run('cargo', ['generate-lockfile'], app, `${prefix}-resolve.log`, env);
        resolved = true;
      }
      console.log(`[llm-docs] compiling ${platform}/${lesson.id} and checking its authored interactions`);
      await run('cargo', ['check', '--locked', '--all-targets'], app, `${prefix}-check.log`, env);
      await run('cargo', ['test', '--locked', '--test', 'tutorial', '--', '--test-threads=1'], app, `${prefix}-test.log`, env);
      records.push({ platform, lesson: lesson.id, files: Object.fromEntries([...authored].map(([path, body]) => [path, sha(body)])),
        assertions: checks.length, lockfile: sha(await readFile(join(app, 'Cargo.lock'))) });
    }
  }
  // Source edits during a long compiler run invalidate its result rather than blessing new code.
  const after = await validationFingerprint();
  if (after.fingerprint !== before.fingerprint) throw new Error('Documentation or framework changed during LLM validation; rerun the build');
  const receipt = { status: 'pass', fingerprint: before.fingerprint, toolchain: before.toolchain,
    substitutions: ['Day dependencies resolve to this checkout', 'mock toolkit; native SDKs and renderers are checked by doctor-verify CI'],
    records, commands, logs: relative(root, session) };
  await writeFile(receiptPath, JSON.stringify(receipt, null, 2) + '\n');
  console.log(`[llm-docs] passed ${records.length} tutorial projects; logs: ${session}`);
  return receipt;
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  await verifyLlmDocs({ force: process.argv.includes('--force') });
}
