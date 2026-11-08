// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { cp, mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
import { dirname, join, relative } from 'node:path';
import { fileURLToPath } from 'node:url';

const site = fileURLToPath(new URL('../', import.meta.url));
const repo = dirname(site);
const app = join(repo, 'apps/website-controls');
const stamp = join(site, '.cache/live-demo.json');

export async function demoSource() {
  const source = await readFile(join(app, 'src/lib.rs'), 'utf8');
  const code = source.match(/^pub fn root\(\) -> impl Piece \{[\s\S]*?^\}/m)?.[0];
  if (!code) throw new Error('Missing website-controls root function');
  return code;
}

async function files(dir) {
  const found = [];
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    if (['target', 'build', '.git'].includes(entry.name)) continue;
    const path = join(dir, entry.name);
    if (entry.isDirectory()) found.push(...await files(path));
    else if (entry.isFile()) found.push(path);
  }
  return found.sort();
}
const digest = bytes => createHash('sha256').update(bytes).digest('hex');
async function fingerprint() {
  // Rust, code generation, and the CLI's embedded web runtime all affect the served app.
  // Cosmetic Astro edits do not: a warm dev-server restart needs no Rust compilation.
  const inputs = [join(repo, 'Cargo.toml'), join(repo, 'Cargo.lock'), fileURLToPath(import.meta.url),
    ...await files(app), ...await files(join(repo, 'crates'))];
  const hash = createHash('sha256');
  for (const path of inputs) hash.update(relative(repo, path)).update('\0').update(await readFile(path));
  return hash.digest('hex');
}
async function writeChanged(path, value) {
  if (await readFile(path, 'utf8').catch(() => '') === value) return;
  await mkdir(dirname(path), { recursive: true });
  await writeFile(path, value);
}
function build() {
  return new Promise((resolve, reject) => {
    const child = spawn('cargo', ['run', '--locked', '-p', 'day-cli', '--', 'build',
      '--project', app, '-t', 'web-dom', '--profile', 'release'], { cwd: repo, stdio: 'inherit' });
    child.on('error', reject);
    child.on('exit', code => code === 0 ? resolve() : reject(new Error(`Live demo build failed (${code})`)));
  });
}

export async function ensureLiveDemo() {
  let key = await fingerprint();
  let cached = JSON.parse(await readFile(stamp, 'utf8').catch(() => 'null'));
  let valid = cached?.key === key && cached.files?.length > 0;
  if (valid) {
    for (const [path, sha] of cached.files) {
      if (digest(await readFile(join(site, 'public', path)).catch(() => Buffer.alloc(0))) !== sha) {
        valid = false;
        break;
      }
    }
  }
  if (!valid) {
    console.info('[live-demo] building the shared-signal app for web-dom');
    await build();
    key = await fingerprint(); // A first build also resolves the app's Cargo.lock.
    const base = `demos/shared-signal/${key.slice(0, 16)}`;
    const dest = join(site, 'public', base);
    await mkdir(dest, { recursive: true });
    await cp(join(app, 'build/day/cargo/web-dom/release/dist'), dest, { recursive: true });
    // The iframe uses the standard Day runtime, with a small host that announces readiness
    // and leaves service-worker ownership to the containing site.
    await cp(join(app, 'web/index.html'), join(dest, 'index.html'));
    // This app uses system fonts. Give the standard loader an empty manifest instead of a 404.
    await mkdir(join(dest, 'assets/fonts'), { recursive: true });
    await writeFile(join(dest, 'assets/fonts/fonts.json'), '[]\n');
    const outputs = [];
    for (const path of await files(dest)) outputs.push([relative(join(site, 'public'), path), digest(await readFile(path))]);
    cached = { key, src: `${base}/index.html`, files: outputs };
    await writeChanged(stamp, JSON.stringify(cached, null, 2) + '\n');
  }
  await writeChanged(join(site, 'src/data/live-demo.json'), JSON.stringify({ src: cached.src, code: await demoSource() }, null, 2) + '\n');
  // Publish only the current generated app, rather than accumulating previous local builds.
  const generated = join(site, 'public/demos/shared-signal');
  for (const entry of await readdir(generated, { withFileTypes: true })) {
    if (entry.isDirectory() && /^[a-f0-9]{16}$/.test(entry.name) && entry.name !== key.slice(0, 16)) {
      await rm(join(generated, entry.name), { recursive: true });
    }
  }
  return cached.src;
}
