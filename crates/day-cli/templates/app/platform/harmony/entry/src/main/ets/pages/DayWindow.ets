// The host page for a SECONDARY day window (docs/windows.md): a bare NodeContent slot the
// native side mounts the window's day subtree into — no Navigation host (secondary windows
// carry their own content; the primary Index page owns the app's nav stack). Size flows
// like the primary: the first laid-out area completes the pending open (`windowStart`),
// every later change reports `windowResized`.

import { NodeContent } from '@kit.ArkUI';
import { common } from '@kit.AbilityKit';
import nativeEntry from 'libentry.so';

@Entry({ useSharedStorage: true })
@Component
struct DayWindow {
  @LocalStorageProp('dayNode') node: number = 0;
  private content: NodeContent = new NodeContent();
  private started: boolean = false;

  build() {
    Column() {
      ContentSlot(this.content)
    }
    .width('100%')
    .height('100%')
    .onAreaChange((_oldArea: Area, newArea: Area) => {
      const w = newArea.width as number;
      const h = newArea.height as number;
      if (!this.started && w > 0 && h > 0 && this.node > 0) {
        this.started = true;
        if (!nativeEntry.windowStart(this.content, this.node, w, h)) {
          // Closed before we connected — drop this instance again.
          (this.getUIContext().getHostContext() as common.UIAbilityContext).terminateSelf();
        }
      } else if (this.started && this.node > 0) {
        nativeEntry.windowResized(this.node, w, h);
      }
    })
  }
}
