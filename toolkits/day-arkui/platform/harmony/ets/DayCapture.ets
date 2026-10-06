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
      settleDayPieces().catch((): void => {}).then((): void => {
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
