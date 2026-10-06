// The ArkTS host page for a Day app — the framework's, not the app's. `day build` stages this
// directory into the app's hvigor project at `entry/src/main/ets/day/` (gitignored) from the
// day-arkui crate the app depends on, the way Gradle reads the Java shim from day-android
// (docs/harmonyos.md). The window mounts the Day tree and a separate cover layer.
// DayNavigation.ets hosts a Navigation/NavPathStack inside EACH navigation surface,
// so sibling tabs and independently mounted stacks cannot share titles or back history. Day owns layout,
// so we pass each slot's size (vp) and the display density (px per vp).
//
// It also registers a file picker (docs/files.md): HarmonyOS's open/save UI lives in the ArkTS
// @kit.CoreFileKit DocumentViewPicker, so Day's native `open_file()`/`save_file()` calls up into
// this host, which drives the picker and copies bytes through @ohos.file.fs.

import { NodeContent } from '@kit.ArkUI';
import { display } from '@kit.ArkUI';
import { window } from '@kit.ArkUI';
import { picker } from '@kit.CoreFileKit';
import { fileIo as fs } from '@kit.CoreFileKit';
import { abilityAccessCtrl, common, Permissions, PermissionRequestResult, Want } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
import nativeEntry from 'libentry.so';
import { registerDayPieces } from '../daypieces/DayPieces';
import { daySelect, daySegmented } from './DaySelect';
import DayWindowAbility from './DayWindowAbility';
import { DayToolbar, DayToolbarItem } from './DayToolbar';
import { dayNavigation, registerDayNavigation } from './DayNavigation';
import { DayCapture } from './DayCapture';

@Entry
@Component
struct Index {
  private content: NodeContent = new NodeContent();
  private coverContent: NodeContent = new NodeContent();
  private coverWidthVp: number = 0;
  private coverHeightVp: number = 0;
  @State coverShown: boolean = false;
  private started: boolean = false;
  @State windowMenus: DayToolbarItem[] = [];
  onBackPress(): boolean {
    return this.coverShown && nativeEntry.coverBackRequested();
  }

  private startDay(widthVp: number, heightVp: number): void {
    if (this.started || widthVp <= 0 || heightVp <= 0) {
      return;
    }
    this.started = true;
    DayCapture.register(0, this.getUIContext());
    nativeEntry.registerCoverLayer(this.coverContent, (shown: boolean): void => {
      this.coverShown = shown;
    });
    nativeEntry.coverLayerResized(this.coverWidthVp || widthVp, this.coverHeightVp || heightVp);

    registerDayNavigation(this.getUIContext(), (items: DayToolbarItem[]): void => {
      this.windowMenus = items;
    });
    // Standalone pieces whose component exists only in ArkTS (docs/extending.md) — the native
    // Web view is one, since the ArkUI C node API has no Web kind. `day build` regenerates
    // DayPieces.ets from the app's piece closure (an empty set when it has none), and the
    // framework's own ArkTS-only components ride along: the menu-style picker's `Select`
    // (DaySelect.ets). Before start(): a piece node can be realized during the very first tree
    // build.
    registerDayPieces(this.getUIContext(), [daySelect, daySegmented, dayNavigation]);

    // Hand the ArkTS resourceManager to native so `resource("name")` can read bundled data via the
    // NDK rawfile API (§18.3) — the manager can only be obtained on the ArkTS side. Before
    // start(): the very first tree build already probes rawfile (nav-row vector icons,
    // docs/vectors.md) and reads data resources.
    nativeEntry.registerResourceManager(
      (this.getUIContext().getHostContext() as common.UIAbilityContext).resourceManager
    );

    // The status bar (docs/cover.md): hidden while a mounted `status_bar_hidden` asks for it.
    // Before start(): the first tree build can already carry the request. A dayscript run
    // skips the bar's slide, like every other transition: a screenshot right after a page
    // that hides the bar would otherwise catch the window mid-resize.
    nativeEntry.registerStatusBar((hidden: boolean): void => {
      const animated: boolean = AppStorage.get<boolean>('dayDriven') !== true;
      window.getLastWindow(this.getUIContext().getHostContext() as common.UIAbilityContext)
        .then((win: window.Window) => win.setSpecificSystemBarEnabled('status', !hidden, animated))
        .catch((err: BusinessError) => {
          console.error(`day: status bar update failed: ${err.code} ${err.message}`);
        });
    });

    const d = display.getDefaultDisplaySync();
    // native `start(nodeContent, widthVp, heightVp, density)` → day::arkui::start → launch_with.
    nativeEntry.start(this.content, widthVp, heightVp, d.densityPixels);

    // Bridge Day's native file open/save to the ArkTS DocumentViewPicker (docs/files.md).
    const ctx = this.getUIContext().getHostContext() as common.UIAbilityContext;
    nativeEntry.registerFilePicker(
      (req: number, mode: number, name: string, src: string, filters: string): void => {
        this.pickFile(req, mode, name, src, filters);
      },
      ctx.cacheDir
    );

    // Bridge Day's permission prompts to abilityAccessCtrl (docs/permissions.md): `names` is
    // 0x1F-separated, and the answer is a bit mask of the grants, in the same order.
    nativeEntry.registerPermissions((req: number, names: string): void => {
      const list: Array<Permissions> = names.split('\u001F') as Array<Permissions>;
      abilityAccessCtrl.createAtManager().requestPermissionsFromUser(ctx, list)
        .then((result: PermissionRequestResult) => {
          let mask: number = 0;
          for (let i = 0; i < result.authResults.length; i++) {
            if (result.authResults[i] === 0) {
              mask |= (1 << i);
            }
          }
          nativeEntry.onPermissionResult(req, mask);
        })
        .catch((err: BusinessError) => {
          console.error(`day: requestPermissionsFromUser failed: ${err.code} ${err.message}`);
          nativeEntry.onPermissionResult(req, 0);
        });
    });

    // Bridge Day's `link` piece to the system browser via an implicit viewData Want.
    nativeEntry.registerOpenUrl((url: string): void => {
      const want: Want = { action: 'ohos.want.action.viewData', uri: url };
      ctx.startAbility(want).catch((err: BusinessError) => {
        console.error(`day: openUrl failed: ${err.code} ${err.message}`);
      });

    });

    // Secondary day windows (docs/windows.md): each open_window becomes a multiton
    // DayWindowAbility instance (its own task card; freeform on tablets), carrying the
    // day root node id + title as want parameters.
    nativeEntry.registerWindows(
      (node: number, title: string): void => {
        const want: Want = {
          bundleName: ctx.abilityInfo.bundleName,
          abilityName: 'DayWindowAbility',
          parameters: { dayNode: node, dayTitle: title }
        };
        ctx.startAbility(want).catch((err: BusinessError) => {
          console.error(`day: openWindow failed: ${err.code} ${err.message}`);
        });
      },
      (node: number): void => {
        DayWindowAbility.terminate(node);
      }
    );

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

  build() {
    Stack() {
      Column() {
        if (this.windowMenus.length > 0) {
          Row() { DayToolbar({ items: this.windowMenus }) }
            .width('100%').height(56).justifyContent(FlexAlign.End)
            .backgroundColor($r('sys.color.ohos_id_color_sub_background'))
        }
        Stack() { ContentSlot(this.content) }
          .width('100%').layoutWeight(1).clip(true)
          .onAreaChange((_old: Area, area: Area) => {
            if (this.started) nativeEntry.resized(Number(area.width), Number(area.height));
            else this.startDay(Number(area.width), Number(area.height));
          })
      }.width('100%').height('100%')
      // Covers remain above all tab-local Navigation hosts and their destinations.
      Stack() { ContentSlot(this.coverContent) }
        .width('100%').height('100%')
        .visibility(this.coverShown ? Visibility.Visible : Visibility.Hidden)
    }
    .id('day.capture.0').width('100%').height('100%')
    .backgroundColor($r('sys.color.ohos_id_color_sub_background'))
    .onAreaChange((_old: Area, area: Area) => {
      this.coverWidthVp = Number(area.width);
      this.coverHeightVp = Number(area.height);
      nativeEntry.coverLayerResized(this.coverWidthVp, this.coverHeightVp);
    })
  }
}
