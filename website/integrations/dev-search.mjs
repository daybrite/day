// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { existsSync } from 'node:fs';
import { readFile, realpath } from 'node:fs/promises';
import { extname, join, resolve, sep } from 'node:path';
import { fileURLToPath } from 'node:url';

/** Serve only the last built Pagefind bundle; never expose other build output or source files. */
export function pagefindMiddleware(directory) {
  return async (request, response, next) => {
    if (!['GET', 'HEAD'].includes(request.method)) return next();
    try {
      const pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname);
      const path = resolve(directory, '.' + pathname);
      if (!path.startsWith(directory + sep)) throw new Error('Outside search bundle');
      const [root, file] = await Promise.all([realpath(directory), realpath(path)]);
      if (!file.startsWith(root + sep)) throw new Error('Outside search bundle');
      const body = await readFile(file);
      const types = { '.js': 'text/javascript', '.css': 'text/css', '.json': 'application/json', '.wasm': 'application/wasm' };
      response.writeHead(200, {
        'Content-Type': types[extname(file)] ?? 'application/octet-stream',
        'Content-Length': body.length,
        'Cache-Control': 'no-store',
      });
      response.end(request.method === 'HEAD' ? undefined : body);
    } catch {
      response.writeHead(404);
      response.end();
    }
  };
}

export default function devSearch() {
  let directory, prefix;
  return {
    name: 'day-dev-search',
    hooks: {
      'astro:config:done': ({ config }) => {
        directory = join(fileURLToPath(config.outDir), 'pagefind');
        prefix = `${config.base.replace(/\/$/, '')}/pagefind`;
      },
      'astro:server:setup': ({ server, logger }) => {
        server.middlewares.use(prefix, pagefindMiddleware(directory));
        if (!existsSync(join(directory, 'pagefind-entry.json'))) {
          logger.warn('Local search needs a built index. Run npm run build, then retry search.');
        }
      },
    },
  };
}
