// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// The explorer and gallery share one release manifest. Prefer the Paris city capture;
// legacy releases require both phone scroll positions in the exact theme/language.
export const travelTargets = [
  ['ios-uikit', 'iOS · UIKit', true],
  ['android-material', 'Android · Material', true],
  ['harmony-arkui', 'HarmonyOS · ArkUI', true],
  ['macos-appkit', 'macOS · AppKit', false],
  ['windows-winui', 'Windows · WinUI', false],
  ['linux-gtk', 'Linux · GTK', false],
  ['linux-qt', 'Linux · Qt', false],
  ['web-dom', 'Web · DOM', false],
];
export const travelLocales = [
  ['en', 'English'], ['fr', 'Français'], ['de', 'Deutsch'], ['ja', '日本語'],
  ['zh-CN', '简体中文'], ['ko', '한국어'], ['ar', 'العربية'],
];
export const travelThemes = [['light', 'Light'], ['dark', 'Dark']];
export const travelPositions = { overview: 'Full window', top: 'Scrolled to top', bottom: 'Scrolled to bottom' };

export function travelCaptures(app) {
  if (!app) return [];
  const captures = [];
  const city = app.shots.find(s => s.id === 'paris');
  for (const [target, platform, mobile] of travelTargets) {
    for (const [locale, language] of travelLocales) {
      for (const [theme, appearance] of travelThemes) {
        const images = (city ? [city.id] : mobile ? ['top', 'bottom'] : ['overview']).map(position => {
          const image = app.shots.find(s => s.id === position)?.byColumn
            .find(c => c.column === target)?.captures[`${theme}|${locale}`];
          return image?.src && image.width > 0 && image.height > 0
            ? { ...image, position, label: city?.label ?? travelPositions[position] } : null;
        });
        if (images.every(Boolean)) captures.push({ target, platform, locale, language, theme, appearance, mobile, images });
      }
    }
  }
  return captures;
}
