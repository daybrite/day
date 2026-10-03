import { AbilityConstant, Configuration, ConfigurationConstant, UIAbility, Want } from '@kit.AbilityKit';
import { window, KeyboardAvoidMode } from '@kit.ArkUI';
import { util } from '@kit.ArkTS';
import { fileIo as fs } from '@kit.CoreFileKit';
import nativeEntry from 'libentry.so';
import { registerDayBridges } from '../daybridge/DayBridges';

// The app's main ability — the framework's, staged into `entry/src/main/ets/day/` by `day build`
// from the day-arkui crate (docs/harmonyos.md). An app never edits this copy.

// One entry of the rawfile `day/fonts.json` manifest `day build` stages from the project's
// `fonts/` directory (§18.4).
interface DayFontEntry {
  family: string;
  file: string;
}

export default class EntryAbility extends UIAbility {
  private mainWindow: window.Window | undefined = undefined;
  // Whether the window stage last reported ACTIVE: the lifecycle's active pair is a transition.
  private active = false;
  private darkMode: boolean = false;

  onCreate(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    // Deliver launcher-provided environment (the dayscript engine port + token, plus locale /
    // autodrive) into the native process before the UI — and thus `day_script::init()` inside
    // `start()` — runs. This is the HarmonyOS analogue of Android's intent-extra env delivery
    // (day/src/lib.rs): `day launch` passes them as `aa start --ps` string parameters.
    EntryAbility.applyLaunchEnv(want);
    // Match native navigation, popup resources and text direction to the locale Day was
    // launched with. Setting only DAY_LOCALE translated the Rust tree but left ArkUI's
    // title bar/back button in the device language (notably LTR for Arabic test runs).
    const locale = want.parameters?.['day.locale'];
    if (typeof locale === 'string' && locale.length > 0) {
      this.context.getApplicationContext().setLanguage(locale);
    }
    // Every bridged crate's ArkTS arm (docs/bridge.md "Callbacks"), by symbol, before the
    // native side can ask for one. `day build` generates the aggregator, empty when no crate
    // in the app carries an ArkTS arm.
    nativeEntry.registerDayBridges(registerDayBridges());
    // The app's private files directory, for day-part-fs (docs/fs.md) — here rather than in
    // applyLaunchEnv, which is static and has no ability context.
    nativeEntry.setEnv('DAY_DATA_DIR', this.context.filesDir);
    // Report the ACTIVE color mode so day-arkui themes its neutral paints (DAY_THEME, when
    // present, wins on the Rust side): the C-API nodes don't re-theme hardcoded colors.
    nativeEntry.setEnv(
      'DAY_ARKUI_DARK',
      this.context.config.colorMode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK ? '1' : '0'
    );
    // DAY_THEME=light|dark (a `day launch --env` passthrough, delivered as a `--ps` want
    // parameter) forces the app's color mode — themed screenshot runs and local theme checks;
    // unset ⇒ follow the system.
    const theme: Object | undefined = want.parameters?.['day.env.DAY_THEME'];
    this.darkMode = theme === 'dark' || (theme !== 'light' &&
      this.context.config.colorMode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK);
    if (theme === 'dark') {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_DARK);
    } else if (theme === 'light') {
      this.context.getApplicationContext().setColorMode(ConfigurationConstant.ColorMode.COLOR_MODE_LIGHT);
    }
    // Cold deep link (docs/deep-links.md): the launching want's uri, or the `day.uri`
    // parameter a [[shortcuts]] want carries. Safe before start() — the native side buffers
    // it until the first mount, and it wins over `.restore`.
    const uri = want.uri || (want.parameters?.['day.uri'] as string | undefined);
    if (uri) {
      this.deliverUri(uri);
    }
  }

  // Warm deep link (docs/deep-links.md): the app was running and the system delivered a new
  // want (singleton launch mode). Same call as the cold path; the native side navigates.
  onNewWant(want: Want, _launchParam: AbilityConstant.LaunchParam): void {
    const uri = want.uri || (want.parameters?.['day.uri'] as string | undefined);
    if (uri) {
      this.deliverUri(uri);
    }
  }

  private documentSerial: number = 0;
  private deliverUri(uri: string): void {
    if (!uri.startsWith('file://')) { nativeEntry.deepLink(uri); return; }
    this.importDocument(uri);
  }
  private async importDocument(uri: string): Promise<void> {
    const path = `${this.context.cacheDir}/day-document-${Date.now()}-${++this.documentSerial}`;
    let input: fs.File | undefined = undefined;
    let output: fs.File | undefined = undefined;
    try {
      input = await fs.open(uri, fs.OpenMode.READ_ONLY);
      const stat = await fs.stat(input.fd);
      if (stat.size > 512 * 1024 * 1024) throw new Error('Document exceeds staging limit');
      output = await fs.open(path, fs.OpenMode.CREATE | fs.OpenMode.WRITE_ONLY | fs.OpenMode.TRUNC);
      const buffer = new ArrayBuffer(65536);
      let total: number = 0;
      for (;;) {
        const count = await fs.read(input.fd, buffer);
        if (count === 0) break;
        total += count;
        if (total > 512 * 1024 * 1024) throw new Error('Document exceeds staging limit');
        let offset: number = 0;
        while (offset < count) {
          const written = await fs.write(output.fd, buffer.slice(offset, count));
          if (written <= 0) throw new Error('Document write failed');
          offset += written;
        }
      }
      // The native intake recognizes file URLs as documents, not navigation routes.
      nativeEntry.deepLink(`file://${path}`);
    } catch (error) {
      console.warn('Day document import failed', error);
      nativeEntry.deepLink('file:///day-document-unavailable');
      try { await fs.unlink(path); } catch (_) {}
    } finally {
      if (input) await fs.close(input);
      if (output) await fs.close(output);
    }
  }

  onWindowStageCreate(windowStage: window.WindowStage): void {
    this.mainWindow = windowStage.getMainWindowSync();
    this.updateSystemBars();
    // The active/inactive pair of day's lifecycle (docs/lifecycle.md): the stage gains and
    // loses focus around every app switch, dialog and lock, which `onForeground`/`onBackground`
    // (the visible pair, below) do not report. Delivered as a transition only, like UIKit's
    // scene states, so a repeated ACTIVE never doubles DidBecomeActive.
    windowStage.on('windowStageEvent', (event: window.WindowStageEventType) => {
      const active = event === window.WindowStageEventType.ACTIVE;
      const inactive = event === window.WindowStageEventType.INACTIVE;
      if ((active || inactive) && active !== this.active) {
        this.active = active;
        nativeEntry.lifecycle(active ? 2 : 3);
      }
    });
    windowStage.loadContent('day/Index', () => {
      // Bundled custom fonts (§18.4): registered as soon as the content is loaded — before
      // the Day tree mounts (it starts on the page's first onAreaChange, after layout) — so the
      // NODE_FONT_FAMILY attribute day-arkui sets can resolve the family names. Font
      // registration is UIContext-scoped (the global font.registerFont is deprecated since
      // API 18), and the window's UIContext only exists once the content has loaded.
      this.registerBundledFonts(windowStage);
      // Keyboard avoidance (docs/focus.md): RESIZE shrinks the page when the soft keyboard
      // rises (instead of the default translate), so the page's onAreaChange fires and Day
      // relayouts into the reduced area — the focused field stays visible.
      windowStage.getMainWindowSync().getUIContext()
        .setKeyboardAvoidMode(KeyboardAvoidMode.RESIZE);
    });
  }

  onConfigurationUpdate(config: Configuration): void {
    if (config.colorMode !== undefined) {
      this.darkMode = config.colorMode === ConfigurationConstant.ColorMode.COLOR_MODE_DARK;
      this.updateSystemBars();
    }
  }

  private updateSystemBars(): void {
    const color: string = this.darkMode ? '#FFFFFF' : '#000000';
    this.mainWindow?.setWindowSystemBarProperties({
      statusBarContentColor: color,
      navigationBarContentColor: color
    }).catch(() => {
      console.warn('day-arkui: could not update system bar appearance');
    });
  }

  // Register every font `day build` staged into rawfile `day/fonts/` (manifest: `day/fonts.json`).
  // `$rawfile()` only accepts compile-time literals, so build the equivalent rawfile Resource
  // object (type 30000) for each dynamic path — the documented pattern for runtime rawfile refs.
  private registerBundledFonts(windowStage: window.WindowStage): void {
    let manifest: DayFontEntry[];
    try {
      const bytes: Uint8Array = this.context.resourceManager.getRawFileContentSync('day/fonts.json');
      const decoder: util.TextDecoder = util.TextDecoder.create('utf-8');
      manifest = JSON.parse(decoder.decodeToString(bytes)) as DayFontEntry[];
    } catch (_e) {
      return; // No day/fonts.json staged — the app bundles no custom fonts.
    }
    const uiFont = windowStage.getMainWindowSync().getUIContext().getFont();
    for (const entry of manifest) {
      // A STRING rawfile URI — a hand-built Resource object (id -1 / type 30000) does not
      // resolve through registerFont, and the family silently falls back to the system
      // font. The resource://RAWFILE form is the same contract day's bundled images use.
      uiFont.registerFont({
        familyName: entry.family,
        familySrc: `resource://RAWFILE/day/fonts/${entry.file}`,
      });
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
    // A dayscript engine drives this session (docs/agent.md): the Index page reads this to
    // push/pop without transition animation — scripted bursts outrun an in-flight transition,
    // and ArkUI Navigation drops operations issued across one (the stack wedges on a stale
    // destination). Human sessions keep the native transitions.
    AppStorage.setOrCreate('dayDriven', typeof params['day.dayscript.port'] === 'string');
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

  onWindowStageDestroy(): void { this.mainWindow = undefined; }

  // The visible pair: the ability left the screen (DidEnterBackground, where day-core parks the
  // frame clock so a game's animation stops costing anything) or is about to return to it
  // (WillEnterForeground). The Android host maps onStart/onStop the same way.
  onForeground(): void { nativeEntry.lifecycle(4); }
  onBackground(): void { nativeEntry.lifecycle(5); }
  // The system's memory pressure signal, at any level: DidReceiveMemoryWarning.
  onMemoryLevel(_level: AbilityConstant.MemoryLevel): void { nativeEntry.lifecycle(6); }
  // The ability instance is being torn down (the user closed it from recents, or the OS
  // reclaimed it): WillTerminate, the app's last chance to save.
  // DidExit follows WillTerminate: nothing of the app runs after onDestroy.
  onDestroy(): void { nativeEntry.lifecycle(7); nativeEntry.lifecycle(8); }
}
