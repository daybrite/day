// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, mkdir, copyFile, readFile, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { pathToFileURL } from 'node:url';

test('publication requires matching screenshots, UI source, and translated messages', async () => {
  const dir = await mkdtemp(join(tmpdir(), 'day-marketing-evidence-'));
  const root = new URL('../', import.meta.url);
  const files = [
    'scripts/verify-marketing-evidence.mjs',
    'public/images/tutorial/provenance.json',
    'public/images/tutorial/counter-start.png',
    'public/images/tutorial/counter-two.png',
    ...['getting-started', 'tutorial-events', 'tutorial-state'].map((p) => `src/content/docs/${p}.md`),
  ];
  try {
    for (const file of files) {
      const target = join(dir, file);
      await mkdir(join(target, '..'), { recursive: true });
      await copyFile(new URL(file, root), target);
    }
    const { verifyMarketingEvidence: verify } = await import(pathToFileURL(join(dir, files[0])));
    assert.equal(await verify(), 2);
    const image = join(dir, 'public/images/tutorial/counter-two.png');
    const original = await readFile(image);
    await writeFile(image, Buffer.concat([original, Buffer.from('changed capture')]));
    await assert.rejects(verify, /Capture changed without new provenance/);
    await writeFile(image, original);
    const lesson = join(dir, 'src/content/docs/tutorial-state.md');
    const text = await readFile(lesson, 'utf8');
    await writeFile(lesson, text.replace('saturating_add(1)', 'saturating_add(2)'));
    await assert.rejects(verify, /Tutorial changed/);
    await writeFile(lesson, text.replace('add_one = Add one', 'add_one = Increment'));
    await assert.rejects(verify, /Tutorial changed/);
    await writeFile(lesson, text + '\nEditorial clarification outside the app source.\n');
    assert.equal(await verify(), 2, 'Prose-only changes do not invalidate a capture');
    const recordPath = join(dir, 'public/images/tutorial/provenance.json');
    const record = JSON.parse(await readFile(recordPath, 'utf8'));
    record.target = 'windows-winui';
    await writeFile(recordPath, JSON.stringify(record));
    await assert.rejects(verify, /does not match/);
  } finally {
    await rm(dir, { recursive: true, force: true });
  }
});
