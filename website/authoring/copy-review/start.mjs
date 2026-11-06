// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// The review UI is disabled by default and never imported by the public website.
// Enable for one editing session, from the repository root:
//   DAY_COPY_REVIEW=1 npm --prefix website run copy:review
// Stop with Ctrl-C. Ordinary dev/build/preview commands never start this server.
// Do not put DAY_COPY_REVIEW in .env or a CI workflow; keep activation local and explicit.
// See README.md here for drafting alternatives and locating saved decisions.
import {spawn, spawnSync} from 'node:child_process';
import {createHash} from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';

if (process.env.DAY_COPY_REVIEW !== '1') {
  console.log('Copy review is disabled. Enable with DAY_COPY_REVIEW=1 npm run copy:review.');
  process.exit(0);
}
const tool = path.dirname(fileURLToPath(import.meta.url));
const website = path.resolve(tool, '../..');
const local = path.join(tool, '.local');
fs.mkdirSync(local, {recursive: true});
// The exclusive lock protects saved decisions from a second launcher rebuilding their
// baseline underneath an open review. Remove it manually only after its PID has exited.
const lock = path.join(local, 'session.lock');
let fd;
try {
  fd = fs.openSync(lock, 'wx');
} catch {
  console.error(`A review session already holds ${lock}. Stop that session first.`);
  process.exit(1);
}
fs.writeFileSync(fd, `${process.pid}\n`);
fs.closeSync(fd);
const release = () => { try { fs.unlinkSync(lock); } catch {} };
process.on('exit', release);
const run = (command, args) => {
  const result = spawnSync(command, args, {cwd: website, stdio: 'inherit'});
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status}`);
};
const source = path.join(website, 'src/pages/index.astro');
const digest = () => createHash('sha256').update(fs.readFileSync(source)).digest('hex');
try {
  const before = digest();
  run(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['run', 'build']);
  if (before !== digest()) throw new Error('Landing copy changed during the build. Run the review again.');
  run(process.execPath, [path.join(tool, 'snapshot.mjs')]);
  run('python3', ['-B', path.join(tool, 'build.py')]);
  const child = spawn('python3', ['-B', path.join(tool, 'serve.py'), ...process.argv.slice(2)], {
    cwd: website, stdio: 'inherit', env: process.env,
  });
  for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal));
  child.on('error', error => { console.error(error.message); process.exit(1); });
  child.on('exit', code => process.exit(code ?? 0));
} catch (error) {
  console.error(error.message);
  process.exit(1);
}
