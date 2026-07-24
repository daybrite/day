// The ArkTS host page for a Day app. It hosts HarmonyOS's own Navigation/NavPathStack: the base
// content is a NodeContent slot the native Rust cdylib (libentry.so) mounts the Day tree into,
// and every page Day pushes becomes a real NavDestination (native transition, title bar, system
// back gesture) whose content is a per-page NodeContent (docs/navigation.md). Day owns layout,
// so we pass each slot's size (vp) and the display density (px per vp).
//
// It also registers a file picker (docs/files.md): HarmonyOS's open/save UI lives in the ArkTS
// @kit.CoreFileKit DocumentViewPicker, so Day's native `open_file()`/`save_file()` calls up into
// this host, which drives the picker and copies bytes through @ohos.file.fs.

import { NodeContent } from '@kit.ArkUI';
import { display } from '@kit.ArkUI';
import { picker } from '@kit.CoreFileKit';
import { fileIo as fs } from '@kit.CoreFileKit';
import { common, Want } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import nativeEntry from 'libentry.so';

@Entry
@Component
struct Index {
  private content: NodeContent = new NodeContent();
  private started: boolean = false;
  // Navigation state (docs/navigation.md): one NavPathStack; per-page NodeContents keyed by the
  // Day page id; titles in @State so a NavPatch::Title re-renders the destination's title bar.
  private pathStack: NavPathStack = new NavPathStack();
  private navContents: Map<number, NodeContent> = new Map();
  @State navTitles: Map<number, string> = new Map();
  private navKeys: number[] = [];

  // Scripted sessions (dayscript, docs/agent.md) push/pop WITHOUT transition animation: a
  // pop issued while a push transition is in flight is DROPPED by ArkUI Navigation and the
  // stack wedges on a stale destination — the same transactional hazard the UIKit backend
  // queues around for modals (docs/dialogs.md). Human sessions keep the native transition.
  private animated(): boolean {
    return AppStorage.get<boolean>('dayDriven') !== true;
  }

  private startDay(widthVp: number, heightVp: number): void {
    if (this.started || widthVp <= 0 || heightVp <= 0) {
      return;
    }
    this.started = true;

    // Wire the Navigation bridge BEFORE start(): a deep link (DAY_DEMO_ROUTE) can push during
    // the initial tree build.
    nativeEntry.registerNav(
      (key: number, title: string): NodeContent => {
        const pageContent = new NodeContent();
        this.navContents.set(key, pageContent);
        this.navTitles.set(key, title);
        this.navKeys.push(key);
        // The name is UNIQUE per page: ArkUI Navigation diffs the old and new stacks by
        // destination name, so a same-turn pop+push of two 'day-page' entries would collapse
        // into "page is not change — don't transition", leaving the OLD (already-detached,
        // empty) destination on screen forever. The generic `.navDestination` builder
        // dispatches every name, so the suffix costs nothing.
        this.pathStack.pushPathByName('day-page-' + key, key, this.animated());
        return pageContent;
      },
      (): void => {
        this.pathStack.pop(this.animated());
      },
      (title: string): void => {
        const top: number | undefined = this.navKeys[this.navKeys.length - 1];
        if (top !== undefined) {
          this.navTitles.set(top, title);
        }
      }
    );
    const d = display.getDefaultDisplaySync();
    // native `start(nodeContent, widthVp, heightVp, density)` → day::arkui::start → launch_with.
    nativeEntry.start(this.content, widthVp, heightVp, d.densityPixels);

    // Hand the ArkTS resourceManager to native so `resource("name")` can read bundled data via the
    // NDK rawfile API (§18.3) — the manager can only be obtained on the ArkTS side.
    nativeEntry.registerResourceManager(
      (this.getUIContext().getHostContext() as common.UIAbilityContext).resourceManager
    );

    // Bridge Day's native file open/save to the ArkTS DocumentViewPicker (docs/files.md).
    const ctx = this.getUIContext().getHostContext() as common.UIAbilityContext;
    nativeEntry.registerFilePicker(
      (req: number, mode: number, name: string, src: string, filters: string): void => {
        this.pickFile(req, mode, name, src, filters);
      },
      ctx.cacheDir
    );

    // Bridge Day's `link` piece to the system browser via an implicit viewData Want.
    nativeEntry.registerOpenUrl((url: string): void => {
      const want: Want = { action: 'ohos.want.action.viewData', uri: url };
      ctx.startAbility(want).catch((err: BusinessError) => {
        console.error(`day: openUrl failed: ${err.code} ${err.message}`);
      });
    });
  }

  // Day → ArkTS file picker. mode 0 = open (copy the pick into app cache, hand back a readable
  // path); mode 1 = save (write the Day-staged temp file `src` into the chosen document). Every
  // path answers `onFileResult(req, path)` — "" means the user cancelled.
  private async pickFile(req: number, mode: number, name: string, src: string, filters: string): Promise<void> {
    const ctx = this.getUIContext().getHostContext() as common.UIAbilityContext;
    try {
      const viewPicker: picker.DocumentViewPicker = new picker.DocumentViewPicker(ctx);
      if (mode === 0) {
        const options: picker.DocumentSelectOptions = new picker.DocumentSelectOptions();
        const suffixes: Array<string> = Index.suffixFilters(filters);
        if (suffixes.length > 0) {
          options.fileSuffixFilters = suffixes;
        }
        const uris: Array<string> = await viewPicker.select(options);
        if (uris.length > 0) {
          const leaf: string = uris[0].split('/').pop() ?? ('day-open-' + req);
          const destPath: string = ctx.cacheDir + '/' + leaf;
          const file = fs.openSync(uris[0], fs.OpenMode.READ_ONLY);
          try {
            fs.copyFileSync(file.fd, destPath);
          } finally {
            fs.closeSync(file);
          }
          nativeEntry.onFileResult(req, destPath);
        } else {
          nativeEntry.onFileResult(req, '');
        }
      } else {
        const options: picker.DocumentSaveOptions = new picker.DocumentSaveOptions();
        if (name.length > 0) {
          options.newFileNames = [name];
        }
        const uris: Array<string> = await viewPicker.save(options);
        if (uris.length > 0) {
          if (src.length > 0) {
            // Write the Day-staged bytes into the newly created document.
            const file = fs.openSync(uris[0], fs.OpenMode.READ_WRITE | fs.OpenMode.TRUNC);
            try {
              fs.copyFileSync(src, file.fd);
            } finally {
              fs.closeSync(file);
            }
          }
          nativeEntry.onFileResult(req, uris[0]);
        } else {
          nativeEntry.onFileResult(req, '');
        }
      }
    } catch (err) {
      const e = err as BusinessError;
      console.error(`day file picker failed: ${e.code} ${e.message}`);
      nativeEntry.onFileResult(req, '');
    }
  }

  // Convert Day's flattened filter list ("name|ext1,ext2" joined by 0x1f) into the picker's
  // `fileSuffixFilters` form ("name|.ext1,.ext2"). Best-effort — no filters shows all files.
  private static suffixFilters(filters: string): Array<string> {
    const out: Array<string> = [];
    if (filters.length === 0) {
      return out;
    }
    for (const group of filters.split('\u001f')) {
      const bar: number = group.indexOf('|');
      const label: string = bar >= 0 ? group.substring(0, bar) : group;
      const exts: string = bar >= 0 ? group.substring(bar + 1) : '';
      const dotted: Array<string> = [];
      for (const ext of exts.split(',')) {
        if (ext.length > 0) {
          dotted.push(ext.startsWith('.') ? ext : ('.' + ext));
        }
      }
      if (dotted.length > 0) {
        out.push(label + '|' + dotted.join(','));
      }
    }
    return out;
  }

  // One pushed Day page: a NavDestination whose content is the page's NodeContent. The system
  // title bar (with its native back affordance) shows the pushed title; disappearance — system
  // back gesture, title-bar back, or a Day-initiated pop — reports back via `navPopped` so the
  // native route state stays in sync (docs/navigation.md).
  @Builder
  dayPage(name: string, param: object) {
    NavDestination() {
      Stack() {
        ContentSlot(this.navContents.get(Number(param)))
      }
      .width('100%')
      .height('100%')
      .onAreaChange((_oldArea: Area, newArea: Area) => {
        nativeEntry.navPageArea(Number(param), newArea.width as number, newArea.height as number);
      })
    }
    .title(this.navTitles.get(Number(param)) ?? '')
    .onDisAppear(() => {
      const key: number = Number(param);
      this.navContents.delete(key);
      this.navTitles.delete(key);
      const i: number = this.navKeys.indexOf(key);
      if (i >= 0) {
        this.navKeys.splice(i, 1);
      }
      nativeEntry.navPopped(key);
    })
  }

  build() {
    Navigation(this.pathStack) {
      Stack() {
        // Native ArkUI nodes added via OH_ArkUI_NodeContent_AddNode render here (the root page).
        ContentSlot(this.content)
      }
      .width('100%')
      .height('100%')
      // First area = start; later areas (keyboard RESIZE avoidance, rotation, window
      // resize) flow to Day as window resizes. Area dimensions are in vp.
      .onAreaChange((_oldArea: Area, newArea: Area) => {
        if (this.started) {
          nativeEntry.resized(newArea.width as number, newArea.height as number);
        } else {
          this.startDay(newArea.width as number, newArea.height as number);
        }
      })
    }
    .hideTitleBar(true)
    .mode(NavigationMode.Stack)
    .navDestination(this.dayPage)
    // Theme-adaptive page ground: the C-API Day tree is transparent where it doesn't paint, so
    // without this the window shows the (light) start-window color even under dark colorMode.
    .backgroundColor($r('sys.color.ohos_id_color_sub_background'))
    .width('100%')
    .height('100%')
  }
}
