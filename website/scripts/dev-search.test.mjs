// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import assert from 'node:assert/strict';
import { test } from 'node:test';
import { mkdtemp, mkdir, writeFile, symlink, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pagefindMiddleware } from '../integrations/dev-search.mjs';

test('dev search serves a rebuilt index, nested binary assets and HEAD, but cannot read outside its bundle', async () => {
  const temporary = await mkdtemp(join(tmpdir(), 'day-search-'));
  const directory = join(temporary, 'pagefind');
  const serve = pagefindMiddleware(directory);
  const request = async (url, method = 'GET') => {
    const result = {};
    await serve({ url, method }, {
      writeHead(status, headers) { Object.assign(result, { status, headers }); },
      end(body) { result.body = body; },
    }, () => { result.next = true; });
    return result;
  };
  try {
    assert.equal((await request('/pagefind-ui.js')).status, 404, 'missing index is recoverable');
    await mkdir(join(directory, 'index'), { recursive: true });
    await writeFile(join(directory, 'pagefind-ui.js'), '/* synthetic search UI */');
    const response = await request('/pagefind-ui.js?version=1');
    assert.equal(response.status, 200);
    assert.equal(response.headers['Content-Type'], 'text/javascript');
    assert.equal(response.headers['Cache-Control'], 'no-store');
    assert.equal(response.body.toString(), '/* synthetic search UI */');
    const head = await request('/pagefind-ui.js', 'HEAD');
    assert.equal(head.status, 200);
    assert.equal(head.body, undefined);
    assert.equal(head.headers['Content-Length'], response.body.length);
    await writeFile(join(directory, 'index', 'fixture.pf_index'), Buffer.from([0, 1, 255]));
    assert.deepEqual((await request('/index/fixture.pf_index')).body, Buffer.from([0, 1, 255]));
    await writeFile(join(directory, 'pagefind-ui.js'), '/* rebuilt fixture */');
    assert.equal((await request('/pagefind-ui.js')).body.toString(), '/* rebuilt fixture */');
    await writeFile(join(temporary, 'outside.js'), 'private fixture');
    await symlink(join(temporary, 'outside.js'), join(directory, 'escape.js'));
    for (const path of ['/..%2foutside.js', '/escape.js', '/%00', '/%invalid', '/index/']) {
      assert.equal((await request(path)).status, 404, path);
    }
    assert.equal((await request('/pagefind-ui.js', 'POST')).next, true);
  } finally { await rm(temporary, { recursive: true, force: true }); }
});
