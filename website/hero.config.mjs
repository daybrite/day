// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Reviewed gallery captures for the landing carousel. Choose complete, populated screens;
// avoid loading states, empty editors, and placeholder artwork. Preview both themes when
// changing a pick. A missing pick is reported rather than replaced with an unreviewed screen.
// `darkOnly` adds a reviewed dark selection without expanding the light rotation.
// `framed` means the screenshot already contains desktop window chrome; do not add it twice.
export const heroPicks = [
  { app: 'Day-Travel', shot: 'overview', platform: 'macos-appkit', framed: true },
  { app: 'Day-Skies', shot: 'san-francisco', platform: 'ios-uikit' },
  { app: 'Day-Trader', shot: 'detail', platform: 'windows-winui', framed: true },
  { app: 'Games-Fair', shot: 'solitaire', platform: 'android-material' },
  { app: 'Day-Skies', shot: 'tokyo', platform: 'linux-qt' },
  { app: 'Fleet-Reader', shot: 'seeded-demo', platform: 'macos-appkit', framed: true },
  { app: 'Day-Travel', shot: 'top', platform: 'harmony-arkui' },
  { app: 'Day-Trader', shot: 'watchlist', platform: 'linux-gtk' },
  { app: 'Day-Showcase', shot: 'canvas', platform: 'web-dom' },
  // Feature variety: populated application views and demonstrations of native controls,
  // rich text, custom drawing, tablet composition, media, and interactive game canvases.
  { app: 'Day-Showcase', shot: 'physics-running', platform: 'windows-winui', framed: true },
  { app: 'Day-Showcase', shot: 'textareas-code', platform: 'windows-winui', framed: true },
  { app: 'Day-Showcase', shot: 'tree-final', platform: 'macos-appkit', framed: true },
  { app: 'Day-Showcase', shot: 'dates', platform: 'ios-uikit' },
  { app: 'Day-Showcase', shot: 'split', platform: 'ios-uikit-ipad' },
  { app: 'Day-Showcase', shot: 'media', platform: 'macos-appkit', framed: true },
  { app: 'Stanza-Redux', shot: 'book-details', platform: 'macos-appkit', framed: true },
  { app: 'Stanza-Redux', shot: 'reading-frankenstein', platform: 'ios-uikit-ipad', darkOnly: true },
  { app: 'Fleet-Reader', shot: 'dashboard-unread', platform: 'macos-appkit', framed: true },
  { app: 'Day-Trader', shot: 'detail-monthly', platform: 'windows-winui', framed: true },
  { app: 'Games-Fair', shot: 'pipes', platform: 'android-material' },
  { app: 'Games-Fair', shot: 'pacpac', platform: 'harmony-arkui' },
  { app: 'Games-Fair', shot: 'flipside', platform: 'linux-qt' },
  // Complement apps that currently publish only default-theme captures. Each dark image is
  // reviewed independently; it need not be a counterpart of a light-rotation selection.
  { app: 'Stanza-Redux', shot: 'bookshelf', platform: 'ios-uikit', darkOnly: true },
  { app: 'Stanza-Redux', shot: 'discover', platform: 'windows-winui', framed: true, darkOnly: true },
  { app: 'Games-Fair', shot: 'blockblast', platform: 'linux-qt', darkOnly: true },
  { app: 'Games-Fair', shot: 'home', platform: 'web-dom', darkOnly: true },
  { app: 'Day-Travel', shot: 'overview', platform: 'linux-gtk', darkOnly: true },
  { app: 'Day-Showcase', shot: 'charts', platform: 'windows-winui', framed: true, darkOnly: true },
];
