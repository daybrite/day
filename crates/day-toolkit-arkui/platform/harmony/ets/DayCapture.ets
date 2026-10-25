// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

import { UIContext } from '@kit.ArkUI';
import { BusinessError } from '@kit.BasicServicesKit';
import nativeEntry from 'libentry.so';
import { settleDayPieces } from '../daypieces/DayPieces';

// RenderService completes this snapshot only after ArkUI renders pending work.
// Discard the tiny PixelMap: the runner still captures the WHOLE device screen,
// including system bars, with uitest. This is a checkpoint, not the gallery image.
// Pieces rendering outside that work (a web view's own renderer) settle first
// (`settle` in DayPiece.ets): ArkUI's frame can be done while theirs still trails.
// Each piece caps its own wait, and this caps them all again: a piece that breaks that
// promise (an engine that never answers) makes a capture late, and never holds it, or
// every capture after it, past the screenshot step's own timeout.
const SETTLE_CAP_MS: number = 3000;

function settlePieces(): Promise<void> {
  let timer: number = -1;
  const expired: Promise<void> = new Promise<void>((resolve: () => void) => {
    timer = setTimeout(resolve, SETTLE_CAP_MS);
  });
  let settled: Promise<void>;
  try {
    settled = settleDayPieces().catch((): void => {});
  } catch (err) {
    settled = Promise.resolve();
  }
  return Promise.race([settled, expired]).then((): void => {
    clearTimeout(timer);
  });
}

export class DayCapture {
  private static contexts: Map<number, UIContext> = new Map();

  static register(window: number, context: UIContext): void {
    DayCapture.contexts.set(window, context);
    nativeEntry.registerCapture((revision: number, node: number): void => {
      const ui = DayCapture.contexts.get(node);
      if (ui === undefined) {
        nativeEntry.captureReady(revision, node, 'capture window is gone');
        return;
      }
      settlePieces().then((): void => {
        ui.getComponentSnapshot().get('day.capture.' + node,
          { scale: 0.01, waitUntilRenderFinished: true }).then((pixels) => {
            pixels.release();
            nativeEntry.captureReady(revision, node, '');
          }).catch((error: BusinessError) => {
            nativeEntry.captureReady(revision, node, 'render checkpoint: ' + error.code + ' ' + error.message);
          });
      });
    });
  }

  static unregister(window: number): void {
    DayCapture.contexts.delete(window);
  }
}
