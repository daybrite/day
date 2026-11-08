// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// Missing theme variants are excluded from that rotation, never replaced with another theme.
export function heroCapture(shot, theme) {
  const dark = theme === 'dark';
  const src = dark ? shot.srcDark : shot.src;
  if (!src) return null;
  return {
    src,
    width: dark ? shot.widthDark : shot.width,
    height: dark ? shot.heightDark : shot.height,
  };
}
