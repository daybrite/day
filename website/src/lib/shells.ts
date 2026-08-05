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

// Which shell each target wears is part of its identity, so it lives with the rest of it in
// platforms.mjs. This module keeps the two named exports every caller already imports.
export { bezelOf, chromeOf } from './platforms.mjs';
