// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
// This module only connects site appearance to the Rust app. All control state lives in Rust.
export function start(runtime) {
  let ready = false;
  function applyTint() {
    // Read the rendered control's accent, so this app helper follows day.css without
    // copying its light/dark palette into Rust. Slider and opacity state stay in Rust.
    const slider = document.querySelector('input[type=range]');
    if (!slider) return;
    const rgb = getComputedStyle(slider).accentColor.match(/^rgb\((\d+),\s*(\d+),\s*(\d+)\)$/);
    if (rgb) runtime.exports().website_demo_tint((Number(rgb[1]) << 16) | (Number(rgb[2]) << 8) | Number(rgb[3]));
  }
  function apply(theme) {
    if (ready && (theme === 'light' || theme === 'dark')) {
      runtime.exports().website_demo_theme(theme === 'dark' ? 1 : 0);
      applyTint();
    }
  }
  addEventListener('message', event => {
    if (event.source !== parent || event.origin !== location.origin) return;
    if (event.data?.type === 'day-demo-theme') apply(event.data.theme);
  });
  addEventListener('day-demo-ready', () => {
    ready = true;
    apply(new URLSearchParams(location.search).get('theme'));
    applyTint();
    parent.postMessage({ type: 'day-demo-ready' }, location.origin);
  }, { once: true });
}
