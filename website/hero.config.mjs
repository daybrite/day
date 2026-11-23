// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Reviewed against the actual full-release captures. Alternate operating systems, form
// factors, and visual density: native controls and useful apps alongside vivid canvas demos.
// Prefer populated screens over empty lists, setup forms, duplicate views, or loading states.
// Missing selections are reported, never replaced with an unreviewed screen.
// `darkOnly` adds an independently reviewed dark capture without lengthening the light rotation.
// `framed` means the capture already contains window chrome; do not add it twice.
export const heroPicks = [
  { app: 'Day-Travel', shot: 'tokyo', platform: 'windows-winui', framed: true },
  { app: 'Day-Skies', shot: 'san-francisco', platform: 'ios-uikit' },
  { app: 'Day-Showcase', shot: 'physics-running', platform: 'windows-winui', framed: true },
  { app: 'Games-Fair', shot: 'blockblast', platform: 'linux-qt' },
  { app: 'Fleet-Reader', shot: 'dashboard-unread', platform: 'macos-appkit', framed: true },
  { app: 'Day-Showcase', shot: 'split', platform: 'ios-uikit-ipad' },
  { app: 'Day-Travel', shot: 'london', platform: 'android-material' },
  { app: 'Day-Trader', shot: 'detail', platform: 'macos-appkit', framed: true },
  { app: 'Day-Showcase', shot: 'canvas-arcs', platform: 'web-dom' },
  { app: 'Day-Travel', shot: 'paris', platform: 'harmony-arkui' },
  { app: 'Day-Showcase', shot: 'textareas-code', platform: 'windows-winui', framed: true },
  { app: 'Day-Showcase', shot: 'media', platform: 'macos-appkit', framed: true },
  { app: 'Day-Showcase', shot: 'dates', platform: 'ios-uikit' },
  { app: 'Day-Skies', shot: 'tokyo', platform: 'linux-qt' },
  { app: 'Games-Fair', shot: 'solitaire', platform: 'android-material' },
  { app: 'Stanza-Redux', shot: 'library-overview', platform: 'macos-appkit', framed: true },
  { app: 'Day-Travel', shot: 'new_york', platform: 'linux-gtk' },
  // A calm typographic screen and a vivid chart complement the native dark app captures.
  { app: 'Stanza-Redux', shot: 'reading-frankenstein', platform: 'ios-uikit-ipad', darkOnly: true },
  { app: 'Day-Showcase', shot: 'charts', platform: 'windows-winui', framed: true, darkOnly: true },
];
