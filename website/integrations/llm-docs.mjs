// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { generateLlmDocs } from '../src/lib/llm-docs.mjs';
import { verifyLlmDocs, validationFingerprint } from '../scripts/verify-llm-docs.mjs';
import { checkLlmDocs } from '../scripts/check-llm-docs.mjs';
import { readFile, writeFile, mkdir } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

export default function llmDocs() {
  let config;
  let validated;
  const generate = async () => {
    const { index, full, guides } = generateLlmDocs(config.site);
    const output = new URL('../.cache/llm-docs.json', import.meta.url);
    const value = JSON.stringify({ index, full, guides: guides.map(({ key, route, title, markdown }) => ({ key, route, title, markdown })) });
    await mkdir(new URL('../.cache/', import.meta.url), { recursive: true });
    if (await readFile(output, 'utf8').catch(() => '') !== value) await writeFile(output, value);
  };
  return {
    name: 'day-llm-docs',
    hooks: {
      'astro:config:setup': async ({ command, config: resolved }) => {
        config = resolved;
        if (command === 'build') validated = await verifyLlmDocs();
        await generate();
      },
      'astro:server:setup': ({ server, logger }) => {
        server.watcher.on('change', (path) => {
          if (/\/(src\/content\/docs|src\/data|tutorials)\//.test(path)) {
            generate().catch((error) => logger.error(error.message));
          }
        });
      },
      'astro:build:done': async ({ dir }) => {
        await checkLlmDocs(fileURLToPath(dir), config.site);
        if ((await validationFingerprint()).fingerprint !== validated.fingerprint) {
          throw new Error('LLM validation sources changed during the site build; rebuild before publication');
        }
      },
    },
  };
}
