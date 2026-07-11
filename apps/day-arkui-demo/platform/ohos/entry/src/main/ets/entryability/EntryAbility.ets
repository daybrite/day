import { AbilityConstant, ConfigurationConstant, UIAbility, Want } from '@kit.AbilityKit';
import { font, window } from '@kit.ArkUI';
import { util } from '@kit.ArkTS';
import nativeEntry from 'libentry.so';

// One entry of the rawfile `day/fonts.json` manifest `day build` stages from the project's
// `fonts/` directory (§18.4).
interface DayFontEntry {
  family: string;
  file: string;
}

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    // Deliver launcher-provided environment (the dayscript engine port + token, plus locale /
    // autodrive) into the native process BEFORE the UI — and thus `day_script::init()` inside
    // `start()` — runs. This is the HarmonyOS analogue of Android's intent-extra env delivery
    // (day/src/lib.rs): `day launch` passes them as `aa start --ps` string parameters.
    EntryAbility.applyLaunchEnv(want);
    // DAY_THEME=light|dark (a `day launch --env` passthrough, delivered as a `--ps` want
    // parameter) forces the app's color mode — themed screenshot runs and local theme checks;
    // unset ⇒ follow the system.
    const theme: Object | undefined = want.parameters?.['day.env.DAY_THEME'];
    if (theme === 'dark') {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_DARK);
    } else if (theme === 'light') {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_LIGHT);
    }
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    // Bundled custom fonts (§18.4) must be registered before the native UI loads, so the
    // NODE_FONT_FAMILY attribute day-arkui sets can resolve the family names.
    this.registerBundledFonts();
    windowStage.loadContent('pages/Index');
  }

  // Register every font `day build` staged into rawfile `day/fonts/` (manifest: `day/fonts.json`).
  // `$rawfile()` only accepts compile-time literals, so build the equivalent rawfile Resource
  // object (type 30000) for each dynamic path — the documented pattern for runtime rawfile refs.
  private registerBundledFonts(): void {
    let manifest: DayFontEntry[];
    try {
      const bytes: Uint8Array = this.context.resourceManager.getRawFileContentSync('day/fonts.json');
      const decoder: util.TextDecoder = util.TextDecoder.create('utf-8');
      manifest = JSON.parse(decoder.decodeToString(bytes)) as DayFontEntry[];
    } catch (_e) {
      return; // No day/fonts.json staged — the app bundles no custom fonts.
    }
    for (const entry of manifest) {
      const src: Resource = {
        id: -1,
        type: 30000, // rawfile
        params: [`day/fonts/${entry.file}`],
        bundleName: this.context.applicationInfo.name,
        moduleName: 'entry',
      };
      font.registerFont({ familyName: entry.family, familySrc: src });
    }
  }

  // `day launch` passes each value as a distinct `aa start --ps` string parameter (all shell-safe
  // single tokens — no blob to quote): the dayscript engine port + token and the locale / autodrive.
  private static applyLaunchEnv(want: Want): void {
    const params: Record<string, Object> | undefined = want.parameters;
    if (!params) {
      return;
    }
    const set = (paramKey: string, envKey: string): void => {
      const value: Object | undefined = params[paramKey];
      if (typeof value === 'string' && value.length > 0) {
        nativeEntry.setEnv(envKey, value);
      }
    };
    set('day.dayscript.port', 'DAYSCRIPT_PORT');
    set('day.dayscript.token', 'DAYSCRIPT_TOKEN');
    set('day.locale', 'DAY_LOCALE');
    set('day.autodrive', 'DAY_AUTODRIVE');
    // Generic `day launch --env KEY=VALUE` passthrough: `day.env.KEY` → process env `KEY`.
    for (const key of Object.keys(params)) {
      if (key.startsWith('day.env.')) {
        const value: Object | undefined = params[key];
        if (typeof value === 'string' && value.length > 0) {
          nativeEntry.setEnv(key.slice('day.env.'.length), value);
        }
      }
    }
  }

  onWindowStageDestroy(): void {}
  onForeground(): void {}
  onBackground(): void {}
  onDestroy(): void {}
}
