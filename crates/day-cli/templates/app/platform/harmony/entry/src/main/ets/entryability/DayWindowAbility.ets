import { AbilityConstant, UIAbility, Want, common } from '@kit.AbilityKit';
import { window } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';

/** A secondary day window (docs/windows.md): one multiton instance per open window. The
 *  launching side (Index.ets's registerWindows open callback) passes the day root node id
 *  and title as want parameters; the DayWindow page completes the pending open through
 *  `windowStart`, and this ability reports focus/close back to native. */
export default class DayWindowAbility extends UIAbility {
  /** Live instances by day node id — the registerWindows close callback terminates them. */
  static active: Map<number, common.UIAbilityContext> = new Map();
  private node: number = 0;
  private title: string = '';

  static terminate(node: number): void {
    const ctx = DayWindowAbility.active.get(node);
    if (ctx) {
      ctx.terminateSelf();
    }
  }

  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    this.node = (want.parameters?.dayNode ?? 0) as number;
    this.title = (want.parameters?.dayTitle ?? '') as string;
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    if (this.node === 0) {
      // A recents-restored instance from a previous process: nothing to mount behind it.
      this.context.terminateSelf();
      return;
    }
    DayWindowAbility.active.set(this.node, this.context);
    const storage: LocalStorage = new LocalStorage();
    storage.setOrCreate('dayNode', this.node);
    storage.setOrCreate('dayTitle', this.title);
    windowStage.loadContent('pages/DayWindow', storage);
  }

  onForeground(): void {
    if (this.node !== 0) {
      nativeEntry.windowFocused(this.node, 1);
    }
  }

  onBackground(): void {
    if (this.node !== 0) {
      nativeEntry.windowFocused(this.node, 0);
    }
  }

  onDestroy(): void {
    if (this.node !== 0) {
      DayWindowAbility.active.delete(this.node);
      // A real close (back, recents swipe, terminateSelf): confirm to native, which tears
      // the window's day subtree down.
      nativeEntry.windowClosed(this.node);
    }
  }
}
