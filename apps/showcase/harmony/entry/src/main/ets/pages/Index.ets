// The ArkTS host page for a Day app. It owns a NodeContent slot and hands its handle to the native
// Rust cdylib (libentry.so), which builds the ArkUI node tree and mounts it into the slot. Day owns
// layout, so we pass the slot's size (vp) and the display density (px per vp).
//
// It also registers a file picker (docs/files.md): HarmonyOS's open/save UI lives in the ArkTS
// @kit.CoreFileKit DocumentViewPicker, so Day's native `open_file()`/`save_file()` calls up into
// this host, which drives the picker and copies bytes through @ohos.file.fs.

import { NodeContent } from '@kit.ArkUI';
import { display } from '@kit.ArkUI';
import { picker } from '@kit.CoreFileKit';
import { fileIo as fs } from '@kit.CoreFileKit';
import { common } from '@kit.AbilityKit';
import { BusinessError } from '@kit.BasicServicesKit';
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

    // Hand the ArkTS resourceManager to native so `resource("name")` can read bundled data via the
    // NDK rawfile API (§18.3) — the manager can only be obtained on the ArkTS side.
    nativeEntry.registerResourceManager(getContext(this).resourceManager);

    // Bridge Day's native file open/save to the ArkTS DocumentViewPicker (docs/files.md).
    const ctx = getContext(this) as common.UIAbilityContext;
    nativeEntry.registerFilePicker(
      (req: number, mode: number, name: string, src: string, filters: string): void => {
        this.pickFile(req, mode, name, src, filters);
      },
      ctx.cacheDir
    );
  }

  // Day → ArkTS file picker. mode 0 = open (copy the pick into app cache, hand back a readable
  // path); mode 1 = save (write the Day-staged temp file `src` into the chosen document). Every
  // path answers `onFileResult(req, path)` — "" means the user cancelled.
  private async pickFile(req: number, mode: number, name: string, src: string, filters: string): Promise<void> {
    const ctx = getContext(this) as common.UIAbilityContext;
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
