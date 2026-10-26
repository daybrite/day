// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// Execute the tutorial recipe. Reader commands share one shell; verification is separate.
import { readFile, writeFile, mkdir, appendFile, copyFile, cp, stat, realpath } from 'node:fs/promises';
import { existsSync, createWriteStream } from 'node:fs';
import { spawn, execFileSync } from 'node:child_process';
import { resolve, join, dirname, delimiter, relative } from 'node:path';
import { tmpdir } from 'node:os';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';
import { parseArgs } from 'node:util';
import { readRecipe, validateRecipe, recipeHash, resolvePlan } from '../src/lib/tutorials.mjs';

const root = fileURLToPath(new URL('../../', import.meta.url));
const self = fileURLToPath(import.meta.url);
export const quote = (value, shell) => shell === 'powershell'
  ? "'" + value.replaceAll("'", "''") + "'" : "'" + value.replaceAll("'", "'\\''") + "'";
const interpreter = (shell, body) => shell === 'powershell'
  ? ['pwsh', ['-NoLogo', '-NoProfile', '-NonInteractive', '-Command', "$ErrorActionPreference = 'Stop'; $PSNativeCommandUseErrorActionPreference = $true;\n" + body]]
  : ['bash', ['--noprofile', '--norc', '-e', '-o', 'pipefail', '-c', body]];
const json = async path => JSON.parse(await readFile(path, 'utf8'));
const put = (path, value) => writeFile(path, JSON.stringify(value, null, 2) + '\n');

export function mainCompatibility(plan, checkout) {
  return plan.map(step => {
    if (step.compatibility === 'install') return { ...step, body: `cargo install --path ${quote(join(checkout, 'crates/day-cli'), step.shell)} --locked\n` };
    if (step.kind === 'file' && step.path === 'Cargo.toml') {
      const day = 'day = { package = "day-app", version = "0.6" }';
      const build = 'day-build = "0.6"';
      if (!step.body.includes(day) || !step.body.includes(build)) throw new Error('Update the explicit main-compatibility dependency substitutions.');
      return { ...step, body: step.body.replace(day, `day = { package = "day-app", path = ${JSON.stringify(join(checkout, 'crates/day'))} }`)
        .replace(build, `day-build = { path = ${JSON.stringify(join(checkout, 'crates/day-build'))} }`) };
    }
    return step;
  });
}

export function shellProgram(plan, directory, shell, helper = self) {
  const call = (action, index) => `${shell === 'powershell' ? '& ' : ''}${quote(process.execPath, shell)} ${quote(helper, shell)} --helper ${action} --out ${quote(directory, shell)} --index ${index}`;
  const lines = [shell === 'powershell' ? "$ErrorActionPreference = 'Stop'\n$PSNativeCommandUseErrorActionPreference = $true" : 'set -euo pipefail'];
  for (const [index, step] of plan.entries()) {
    if (step.kind === 'note') continue;
    lines.push(call('begin', index));
    if (step.kind === 'run') lines.push(step.body);
    else lines.push(call(step.kind, index));
    lines.push(call('end', index));
  }
  return lines.join('\n') + '\n';
}

function terminateTree(running) {
  if (running.finished) return;
  if (process.platform === 'win32') {
    try { execFileSync('taskkill', ['/PID', String(running.process.pid), '/T', '/F'], {stdio:'ignore', timeout:10000}); } catch { /* Already exited. */ }
  } else {
    try { process.kill(-running.process.pid, 'SIGTERM'); }
    catch { running.process.kill('SIGTERM'); }
  }
}
function child(command, args, options, log) {
  const process = spawn(command, args, { ...options, stdio: ['ignore', 'pipe', 'pipe'] });
  process.stdout.pipe(log, { end: false }); process.stderr.pipe(log, { end: false });
  let finished = false;
  const done = new Promise((resolve, reject) => {
    process.on('error', reject);
    process.on('close', (code, signal) => { finished = true; resolve({ code, signal }); });
  });
  return { process, done, get finished() { return finished; } };
}
async function command(command, args, cwd, log, timeout = 120000) {
  const running = child(command, args, { cwd, env: process.env }, log);
  const timer = setTimeout(() => running.process.kill(), timeout);
  try {
    const result = await running.done;
    if (result.code !== 0) throw new Error(`${command} ${args[0]} failed: ${JSON.stringify(result)}`);
    return result;
  } finally { clearTimeout(timer); }
}

async function verifyLaunch(step, directory, index) {
  const log = createWriteStream(join(directory, 'evidence', `${index}-ui.log`));
  const [shell, args] = interpreter(step.shell, step.body);
  const started = Date.now();
  const launch = child(shell, args, { cwd: process.cwd(), env: process.env }, log);
  const day = join(process.env.CARGO_HOME, 'bin', process.platform === 'win32' ? 'day.exe' : 'day');
  let checked = false, cleanup = false;
  try {
    // Wait only for discovery; never retry an assertion or replay an interaction after failure.
    while (true) {
      if (launch.finished) throw new Error('The documented launch exited before UI verification.');
      if (Date.now() - started > 30 * 60 * 1000) throw new Error('Launch did not register a session within 30 minutes.');
      let sessions = [];
      try { sessions = await json(join(process.cwd(), 'build/day/sessions.json')); } catch { /* Not created until launch. */ }
      if (sessions.some(s => s.target === step.target && s.startedAt >= started)) break;
      await delay(500);
    }
    for (const check of step.checks) {
      await command(day, ['drive', '-t', step.target, '--steps-json', JSON.stringify([check])], process.cwd(), log);
    }
    if (launch.finished) throw new Error('The app exited unexpectedly during UI verification.');
    checked = true;
  } finally {
    // Teardown is not a tutorial command and is not claimed as testing a physical close action.
    cleanup = true;
    try { await command(day, ['stop', '-t', step.target], process.cwd(), log, 60000); }
    finally {
      const timer = setTimeout(() => launch.process.kill(), 30000);
      let result;
      try { result = await launch.done; } finally { clearTimeout(timer); }
      await put(join(directory, 'evidence', `${index}-launch.json`), { command: step.body, checksPassed: checked, stoppedByHarness: cleanup, exit: result });
      log.end();
    }
  }
}

async function helper(action, directory, index) {
  const plan = await json(join(directory, 'evidence/plan.json'));
  const step = plan[index];
  if (!step) throw new Error(`No step ${index}`);
  const expected = resolve(directory, 'work', step.cwd);
  if (action !== 'end' && await realpath(process.cwd()) !== await realpath(expected)) throw new Error(`${step.key}: expected cwd ${expected}, got ${process.cwd()}`);
  if (action === 'begin' || action === 'end') {
    await appendFile(join(directory, 'evidence/steps.jsonl'), JSON.stringify({ key: step.key, action, at: new Date().toISOString(), cwd: relative(await realpath(join(directory, 'work')), await realpath(process.cwd())) || '.' }) + '\n');
    console.log(`${action}: ${step.key}`);
  }
  if (action === 'file') {
    const destination = join(expected, step.path);
    // Creating the parent would hide a missing documented directory-creation step.
    if (!existsSync(dirname(destination))) throw new Error(`Missing documented parent directory: ${step.path}`);
    if (step.mode === 'append') { await stat(destination); await appendFile(destination, step.body); }
    else await writeFile(destination, step.body);
  }
  if (action === 'launch') await verifyLaunch(step, directory, index);
}

export async function runWalkthrough({ recipe, platform, mode = 'release', directory, source = 'checkout', dryRun = false }) {
  validateRecipe(recipe);
  if (!['release', 'main'].includes(mode)) throw new Error(`Unknown tutorial mode: ${mode}`);
  const expectedOS = { macos: 'darwin', windows: 'win32', linux: 'linux' }[platform];
  if (!dryRun && process.platform !== expectedOS) throw new Error(`Run ${platform} on its native operating system.`);
  const original = resolvePlan(recipe, platform);
  const plan = mode === 'main' ? mainCompatibility(original, root) : original;
  directory = resolve(directory);
  await mkdir(directory); // Never overwrite a previous run.
  await mkdir(join(directory, 'evidence')); await mkdir(join(directory, 'work'));
  await put(join(directory, 'evidence/recipe.json'), { recipe, sha256: recipeHash(recipe) });
  await put(join(directory, 'evidence/reader-plan.json'), original);
  await put(join(directory, 'evidence/plan.json'), plan);
  const shell = recipe.platforms[platform].shell;
  const script = shellProgram(plan, directory, shell);
  const scriptPath = join(directory, 'evidence', shell === 'powershell' ? 'walkthrough.ps1' : 'walkthrough.sh');
  await writeFile(scriptPath, script);
  const result = { mode, source, platform, recipeHash: recipeHash(recipe), status: dryRun ? 'not-run' : 'running', substitutions: mode === 'main' ? ['CLI install from checkout', 'app dependencies from checkout'] : [], started: new Date().toISOString() };
  try {
    result.checkout = execFileSync('git', ['rev-parse', 'HEAD'], { cwd: root, encoding: 'utf8' }).trim();
    result.checkoutDirty = Boolean(execFileSync('git', ['status', '--porcelain'], {cwd:root, encoding:'utf8'}).trim());
  } catch { /* Exported source tree. */ }
  const resultPath = join(directory, 'evidence/result.json');
  await put(resultPath, result);
  if (dryRun) return directory;
  const cargoHome = join(directory, 'cargo-home');
  await mkdir(cargoHome);
  const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !/^(CARGO_|DAY_|RUSTFLAGS$|RUSTDOCFLAGS$)/.test(key)));
  env.CARGO_HOME = cargoHome;
  env.CARGO_TARGET_DIR = join(directory, 'target');
  env.PATH = join(cargoHome, 'bin') + delimiter + process.env.PATH;
  env.DAY_DATA_DIR = join(directory, 'app-data');
  env.DAY_PREFS_DIR = join(directory, 'app-prefs');
  const log = createWriteStream(join(directory, 'evidence/commands.log'));
  const args = shell === 'powershell' ? ['-NoLogo', '-NoProfile', '-NonInteractive', '-File', scriptPath] : ['--noprofile', '--norc', scriptPath];
  const running = child(shell === 'powershell' ? 'pwsh' : 'bash', args, { cwd: join(directory, 'work'), env, detached: process.platform !== 'win32' }, log);
  let interrupted = false;
  const stop = () => { interrupted = true; terminateTree(running); };
  process.once('SIGINT', stop); process.once('SIGTERM', stop);
  const timer = setTimeout(stop, 100 * 60 * 1000);
  try {
    const exit = await running.done;
    result.exit = exit;
    if (interrupted || exit.code !== 0) throw new Error(`Walkthrough failed (${JSON.stringify(exit)}); see ${directory}/evidence.`);
    result.status = 'pass';
    const day = join(cargoHome, 'bin', process.platform === 'win32' ? 'day.exe' : 'day');
    result.cli = execFileSync(day, ['--version'], { env, encoding: 'utf8' }).trim();
  } catch (error) { result.status = 'fail'; result.error = error.message; throw error; }
  finally {
    clearTimeout(timer); process.off('SIGINT', stop); process.off('SIGTERM', stop); log.end(); result.finished = new Date().toISOString();
    const app = join(directory, 'work', recipe.project);
    const day = join(cargoHome, 'bin', process.platform === 'win32' ? 'day.exe' : 'day');
    if (existsSync(day)) {
      try { result.cli = execFileSync(day, ['--version'], {env, encoding:'utf8', timeout:10000}).trim(); } catch { /* Installation may have failed. */ }
      if (result.status !== 'pass' && existsSync(join(app, 'Day.toml'))) {
        try { execFileSync(day, ['stop', '-t', recipe.platforms[platform].target], {cwd:app, env, stdio:'ignore', timeout:60000}); } catch { /* Preserve the original failure. */ }
      }
    }
    for (const path of ['Cargo.toml', 'Cargo.lock', 'Day.toml']) if (existsSync(join(app, path))) await copyFile(join(app, path), join(directory, 'evidence', path));
    for (const path of ['src', 'resource', 'dayscript', 'build/day/screenshots']) if (existsSync(join(app, path))) await cp(join(app, path), join(directory, 'evidence', path.split('/').at(-1)), { recursive: true });
    await put(resultPath, result);
  }
  return directory;
}

async function main() {
  const { values } = parseArgs({ options: { platform: {type:'string'}, mode:{type:'string',default:'release'}, source:{type:'string',default:'checkout'}, out:{type:'string'}, 'dry-run':{type:'boolean'}, helper:{type:'string'}, index:{type:'string'} } });
  if (values.helper) return helper(values.helper, resolve(values.out), Number(values.index));
  if (!['release', 'main'].includes(values.mode) || !['checkout', 'published'].includes(values.source)) throw new Error('Expected mode release/main and source checkout/published.');
  if (values.mode === 'main' && values.source !== 'checkout') throw new Error('Main compatibility uses checkout sources.');
  let recipe = readRecipe();
  if (values.source === 'published') {
    const response = await fetch('https://daybrite.dev/tutorials/session-counter.json', {signal:AbortSignal.timeout(30000)});
    if (!response.ok) throw new Error(`Published recipe fetch failed: ${response.status}`);
    const bundle = await response.json();
    recipe = validateRecipe(bundle.recipe);
    if (recipeHash(recipe) !== bundle.sha256) throw new Error('Published recipe hash mismatch.');
  }
  const directory = values.out ?? join(tmpdir(), `day-tutorial-${values.platform}-${values.mode}-${Date.now()}`);
  console.log(await runWalkthrough({ recipe, platform:values.platform, mode:values.mode, source:values.source, directory, dryRun:values['dry-run'] }));
}
if (process.argv[1] && resolve(process.argv[1]) === self) main().catch(error => { console.error(error.stack); process.exitCode = 1; });
