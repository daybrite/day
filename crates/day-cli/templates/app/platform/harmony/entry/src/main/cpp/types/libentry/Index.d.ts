// ArkTS type declaration for the Rust native module (libentry.so), registered by the C++ shim's
// NAPI init. `start` mounts the Day tree; the file-picker pair bridges Day's native open/save
// requests to the ArkTS @kit.CoreFileKit DocumentViewPicker (docs/files.md).
export const start: (content: Object, widthVp: number, heightVp: number, density: number) => void;
/** A deep link (docs/deep-links.md): a cold want.uri or a warm onNewWant one. Safe before
 *  start() — buffered until the first mount. */
export const deepLink: (uri: string) => void;
/** Root-area change after start (keyboard RESIZE avoidance, rotation), in vp. */
export const resized: (widthVp: number, heightVp: number) => void;

// Set a process environment variable BEFORE `start()`. The launcher (`day launch` → hdc
// `aa start --ps`) hands the app its dayscript engine port + token (and locale / autodrive) this
// way, and the EntryAbility applies them so the walkthrough runner can drive the running app —
// the HarmonyOS analogue of Android's intent-extra env delivery.
export const setEnv: (key: string, value: string) => void;

// Register the ArkTS file picker + the app cache dir. The callback is invoked (on the JS thread)
// when Day requests an open (mode 0) or save (mode 1); it must answer via `onFileResult`.
export const registerFilePicker: (
  callback: (req: number, mode: number, name: string, src: string, filters: string) => void,
  cacheDir: string
) => void;

// Report a picker result back to Day: the chosen local path, or "" if the user cancelled.
export const onFileResult: (req: number, path: string) => void;

// Hand the native side the app's ResourceManager so Day can read staged rawfile data resources
// (§18.3) via OH_ResourceManager_*. Call once, before or after `start()`; until then the rawfile
// resource opener returns nothing (day_ark_res_available == 0).
export const registerResourceManager: (resourceManager: Object) => void;

// Register the ArkTS URL opener for Day's `link` piece: opening a URL lives in the ArkTS layer
// (an implicit viewData Want via UIAbilityContext.startAbility — the native NodeAPI has no
// equivalent). The callback is invoked on the JS thread with every URL Day wants opened.
export const registerOpenUrl: (callback: (url: string) => void) => void;

// --- Navigation bridge (docs/navigation.md) ---------------------------------
// Day drives HarmonyOS's own Navigation/NavPathStack. `registerNav` wires the ArkTS side BEFORE
// `start()`: `push` must create a fresh NodeContent, push a NavDestination for it, and return
// the content (Day mounts the page's native node into it); `pop` pops the top destination;
// `setTitle` retitles it. The ArkTS side reports every destination disappearance (`navPopped`)
// and the destination content area (`navPageArea`) so Day lays the page out in its real bounds.
export const registerNav: (
  push: (key: number, title: string) => Object,
  pop: () => void,
  setTitle: (title: string) => void,
  setGuard: (on: boolean) => void,
  // One call carries ALL of the host's trailing actions (NavProps::bar_actions): four
  // `\n`-joined parallel fields, one entry per action. The dispatch ids travel as text with the
  // rest rather than as numbers, since a u64 id is not exactly representable as a double.
  // `rootOnly` is "1"/"0" per action — "1" rides the root page alone (NavBarScope::RootPage).
  setMenu: (icons: string, labels: string, actions: string, rootOnly: string) => void
) => void;
export const navPopped: (key: number) => void;
// A guarded NavDestination's back was pressed: defer to Rust's guard (docs/navigation.md).
export const navBackRequested: () => void;
export const navPageArea: (key: number, w: number, h: number) => void;
// A trailing title-bar action was tapped (one of NavProps::bar_actions): dispatch it by its
// own id (docs/navigation.md).
export const navMenuAction: (action: number) => void;

// Secondary day windows (docs/windows.md). The registered `open` launches a multiton
// DayWindowAbility (the day node id + title as want parameters); `close` terminates one.
// The ability page completes an open with `windowStart` (false = closed before connecting)
// and reports lifecycle through `windowResized` / `windowFocused` / `windowClosed`.
export const registerWindows: (
  open: (node: number, title: string) => void,
  close: (node: number) => void
) => void;
export const windowStart: (content: Object, node: number, widthVp: number, heightVp: number) => boolean;
export const windowResized: (node: number, widthVp: number, heightVp: number) => void;
export const windowFocused: (node: number, active: number) => void;
export const windowClosed: (node: number) => void;

// --- ArkTS-built piece components (docs/extending.md) -----------------------
// Some components exist only in ArkTS — the ArkUI C node API has no `Web` node kind — so a
// standalone piece ships its own .ets and `day build` generates the aggregator that calls this
// once, before `start()`. `make` returns the component's FrameNode (undefined declines the kind,
// leaving Day's placeholder leaf); `update` carries the piece's own command vocabulary; `dispose`
// releases what the piece holds for a node Day tore down.
export const registerPiece: (
  make: (kind: string, id: number, props: string) => Object | undefined,
  update: (id: number, cmd: string, arg: string) => void,
  dispose: (id: number) => void
) => void;

// An ArkTS-built component reporting back to its piece's Rust front-end, as an `Event::Custom`
// whose payload is the whole message (the cross-boundary Custom carries no tag).
export const pieceEvent: (id: number, text: string, num?: number) => void;
