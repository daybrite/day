// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
import { ensureLiveDemo } from '../scripts/live-demo.mjs';

export default function liveDemo() {
  return {
    name: 'day-live-demo',
    hooks: {
      'astro:config:setup': async ({ logger }) => {
        await ensureLiveDemo();
        logger.info('shared-signal WebAssembly demo matches the app source');
      },
    },
  };
}
