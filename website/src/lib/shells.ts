// Which presentational shell a platform's screenshot wears — used by the gallery tiles and the
// front-page carousel, so the two always dress the same capture the same way.
//
// Captures are the WINDOW CONTENT by design (docs/testing): offscreen snapshots are
// deterministic, headless, and permission-free, and Linux CI runs under bare xvfb where no
// window manager exists to draw real decorations. So the desktop frame is drawn in CSS at
// display time — traffic lights for macOS, caption glyphs for Windows, an Adwaita headerbar for
// GNOME, Breeze for KDE, a browser bar for the web build.
//
// Phones get the opposite treatment: their captures ALREADY hold the real screen chrome (status
// bars; the iOS simulator even renders the Dynamic Island's black pill), so the shell adds only
// what a screenshot cannot contain — the hardware around the glass. Differentiated lightly by
// corner radius and button nubs rather than imitating one specific model.

/** The hardware bezel for a phone-class target, if it has one. */
export const bezelOf = (id: string): string | undefined =>
  ({ 'ios-uikit': 'iphone', 'android-mdc': 'android', 'harmony-arkui': 'harmony' })[id];

/** The window decoration for a desktop/web target, if it has one. */
export const chromeOf = (id: string): string | undefined =>
  ({
    'macos-appkit': 'macos',
    'macos-gtk': 'macos',
    'macos-qt': 'macos',
    'windows-xaml': 'windows',
    'windows-gtk': 'windows',
    'windows-qt': 'windows',
    'linux-gtk': 'gnome',
    'linux-qt': 'kde',
    'web-dom': 'browser',
  })[id];
