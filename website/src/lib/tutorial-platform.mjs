// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
export const tutorialPlatforms = ['macos', 'windows', 'linux'];
export const validPlatform = value => tutorialPlatforms.includes(value) ? value : null;
export function guessPlatform(nav) {
  const ua = nav.userAgent ?? '';
  if (/Android|iPhone|iPad|iPod/i.test(ua) || nav.userAgentData?.mobile ||
      /Mac/i.test(nav.platform ?? '') && nav.maxTouchPoints > 1) return null;
  const platform = nav.userAgentData?.platform || nav.platform || ua;
  if (/win/i.test(platform)) return 'windows';
  if (/mac/i.test(platform)) return 'macos';
  if (/linux/i.test(platform)) return 'linux';
  return null;
}
export function initialPlatform(address, saved, nav) {
  return validPlatform(new URL(address).searchParams.get('platform')) || validPlatform(saved) || guessPlatform(nav);
}
export function tutorialLink(href, origin, platform) {
  const address = new URL(href, origin);
  if (address.origin === new URL(origin).origin && /\/docs\/tutorials(?:\/|$)/.test(address.pathname)) {
    address.searchParams.set('platform', platform);
    return address.pathname + address.search + address.hash;
  }
  return href;
}
