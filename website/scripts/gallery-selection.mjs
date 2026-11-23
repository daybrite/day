// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { platformsById } from '../src/lib/platforms.mjs';

// Named device refinements get their own column; other devices fold into the target.
export function columnFor(platform, device) {
  if (platform === 'android-mdc') platform = 'android-material';
  if (device && platformsById[`${platform}-${device}`]) return `${platform}-${device}`;
  return platform;
}

// Apply the gallery's presentation policy BEFORE decoding or encoding image bytes.
// Preserve capture order and device variants: archive order decides which unnamed
// device represents a column when several captures share its theme/locale combination.
export function selectCaptures(app, screenshots) {
  return screenshots.filter(shot => {
    if (!shot.shot || !shot.platform) return false;
    const column = columnFor(shot.platform, shot.device);
    if (app.platforms && !app.platforms.includes(column)) return false;
    return !app.hide?.includes(shot.shot);
  });
}
