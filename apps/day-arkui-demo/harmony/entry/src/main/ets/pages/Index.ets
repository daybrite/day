// The ArkTS host page for a Day app. It owns a NodeContent slot and hands its handle to the native
// Rust cdylib (libentry.so), which builds the ArkUI node tree and mounts it into the slot. Day owns
// layout, so we pass the slot's size (vp) and the display density (px per vp).

import { NodeContent } from '@kit.ArkUI';
import { display } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';

@Entry
@Component
struct Index {
  private content: NodeContent = new NodeContent();
  private started: boolean = false;

  private startDay(widthVp: number, heightVp: number): void {
    if (this.started || widthVp <= 0 || heightVp <= 0) {
      return;
    }
    this.started = true;
    const d = display.getDefaultDisplaySync();
    // native `start(nodeContent, widthVp, heightVp, density)` → day::arkui::start → launch_with.
    nativeEntry.start(this.content, widthVp, heightVp, d.densityPixels);
  }

  build() {
    Stack() {
      // Native ArkUI nodes added via OH_ArkUI_NodeContent_AddNode render here.
      ContentSlot(this.content)
    }
    .width('100%')
    .height('100%')
    // Start once the slot has a real size (Area dimensions are in vp).
    .onAreaChange((_oldArea: Area, newArea: Area) => {
      this.startDay(newArea.width as number, newArea.height as number);
    })
  }
}
