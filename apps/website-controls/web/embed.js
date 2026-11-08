// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// This module only connects site appearance to the Rust app. All control state lives in Rust.
export function start(runtime) {
  let ready = false;
  function apply(theme) {
    if (ready && (theme === 'light' || theme === 'dark')) {
      runtime.exports().website_demo_theme(theme === 'dark' ? 1 : 0);
    }
  }
  addEventListener('message', event => {
    if (event.source !== parent || event.origin !== location.origin) return;
    if (event.data?.type === 'day-demo-theme') apply(event.data.theme);
  });
  addEventListener('day-demo-ready', () => {
    ready = true;
    apply(new URLSearchParams(location.search).get('theme'));
    parent.postMessage({ type: 'day-demo-ready' }, location.origin);
  }, { once: true });
}
