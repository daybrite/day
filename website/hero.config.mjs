// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Reviewed gallery captures for the landing carousel. Choose complete, populated screens;
// avoid loading states, empty editors, and placeholder artwork. Preview both themes when
// changing a pick. A missing pick is reported rather than replaced with an unreviewed screen.
// `framed` means the screenshot already contains desktop window chrome; do not add it twice.
export const heroPicks = [
  { app: 'Day-Travel', shot: 'overview', platform: 'macos-appkit', framed: true },
  { app: 'Day-Skies', shot: 'san-francisco', platform: 'ios-uikit' },
  { app: 'Day-Trader', shot: 'detail', platform: 'windows-winui', framed: true },
  { app: 'Games-Fair', shot: 'solitaire', platform: 'android-material' },
  { app: 'Day-Skies', shot: 'tokyo', platform: 'linux-qt' },
  { app: 'Day-News', shot: 'seeded-demo', platform: 'macos-appkit', framed: true },
  { app: 'Day-Travel', shot: 'top', platform: 'harmony-arkui' },
  { app: 'Day-Trader', shot: 'watchlist', platform: 'linux-gtk' },
  { app: 'Day-Showcase', shot: 'canvas', platform: 'web-dom' },
];
