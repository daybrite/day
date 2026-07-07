import { AbilityConstant, UIAbility, Want } from '@kit.AbilityKit';
import { window } from '@kit.ArkUI';
import nativeEntry from 'libentry.so';

export default class EntryAbility extends UIAbility {
  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    // Deliver launcher-provided environment (the dayscript engine port + token, plus locale /
    // autodrive) into the native process BEFORE the UI — and thus `day_script::init()` inside
    // `start()` — runs. This is the HarmonyOS analogue of Android's intent-extra env delivery
    // (day/src/lib.rs): `day launch` passes them as `aa start --ps` string parameters.
    EntryAbility.applyLaunchEnv(want);
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    windowStage.loadContent('pages/Index');
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
  }

  onWindowStageDestroy(): void {}
  onForeground(): void {}
  onBackground(): void {}
  onDestroy(): void {}
}
