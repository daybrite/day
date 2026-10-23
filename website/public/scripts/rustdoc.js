// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Account for the shared header's responsive height without changing Rustdoc's navigation.
(() => {
  const header = document.querySelector('body > header.nav');
  if (!header) return;
  const update = () => document.documentElement.style.setProperty('--day-header-height', `${header.offsetHeight}px`);
  update();
  new ResizeObserver(update).observe(header);
})();
