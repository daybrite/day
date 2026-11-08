// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import { visit } from 'unist-util-visit';
import { generateLlmDocs, repository } from '../src/lib/llm-docs.mjs';
import { markdownRoute } from '../src/lib/llm-routes.mjs';

/** Check published bytes and destinations, independently of the generator's unit tests. */
export async function checkLlmDocs(directory = join(repository, 'website/dist'), origin = 'https://daybrite.dev') {
  const docs = generateLlmDocs(origin);
  const outputs = [['llms.txt', docs.index], ['llms-full.txt', docs.full],
    ...docs.guides.map((guide) => [markdownRoute(guide.route).slice(1), guide.markdown])];
  let links = 0;
  for (const [path, expected] of outputs) {
    const actual = await readFile(join(directory, path), 'utf8');
    assert.equal(actual, expected, `Stale or incorrect LLM output: ${path}`);
    const tree = unified().use(remarkParse).parse(actual);
    const destinations = [];
    visit(tree, (node) => { if (['link', 'definition', 'image'].includes(node.type)) destinations.push(node.url); });
    for (const destination of destinations) {
      const url = new URL(destination);
      if (url.origin !== new URL(origin).origin) continue;
      const path = join(directory, decodeURIComponent(url.pathname));
      const file = existsSync(join(path, 'index.html')) ? join(path, 'index.html') : path;
      assert(existsSync(file), `Missing LLM link destination: ${destination}`);
      if (url.hash && file.endsWith('.html')) {
        const html = await readFile(file, 'utf8');
        assert(html.includes(`id="${decodeURIComponent(url.hash.slice(1))}"`), `Missing LLM link fragment: ${destination}`);
      }
      links++;
    }
  }
  console.log(`[llm-docs] ${outputs.length} generated files match canonical sources; ${links} internal links resolve`);
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) await checkLlmDocs();
