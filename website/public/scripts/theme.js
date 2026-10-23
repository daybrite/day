// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Shared by the guides and Rustdoc, including the preference applied before first paint.
(() => {
  const choices = ['light', 'system', 'dark'];
  const root = document.documentElement;
  const media = matchMedia('(prefers-color-scheme: dark)');
  const rustdoc = !!document.querySelector('meta[name="rustdoc-vars"]');
  const read = key => { try { return localStorage.getItem(key); } catch { return null; } };
  const write = (key, value) => { try { localStorage.setItem(key, value); } catch { /* session only */ } };
  let preference = read('day-theme') || 'system';
  if (!choices.includes(preference)) preference = 'system';
  const resolved = () => preference === 'dark' || (preference === 'system' && media.matches) ? 'dark' : 'light';

  function apply() {
    const theme = resolved();
    if (root.dataset.theme !== theme) root.dataset.theme = theme;
    // Keep Rustdoc's native settings and OS-theme listener in agreement with the site control.
    if (rustdoc) {
      write('rustdoc-theme', theme);
      write('rustdoc-use-system-theme', String(preference === 'system'));
      write('rustdoc-preferred-light-theme', 'light');
      write('rustdoc-preferred-dark-theme', 'dark');
    }
    for (const button of document.querySelectorAll('[data-theme-choice]')) {
      const selected = button.dataset.themeChoice === preference;
      button.setAttribute('aria-checked', String(selected));
      button.tabIndex = selected ? 0 : -1;
      button.classList.toggle('on', selected);
    }
  }
  function choose(value) {
    preference = choices.includes(value) ? value : 'system';
    write('day-theme', preference);
    apply();
  }
  apply();

  function bind() {
    apply();
    for (const group of document.querySelectorAll('[data-theme-toggle]')) {
      const buttons = [...group.querySelectorAll('[data-theme-choice]')];
      for (const button of buttons) {
        button.addEventListener('click', () => {
          const collapsed = buttons.some(other => other !== button && other.offsetParent === null);
          const value = button.dataset.themeChoice;
          choose(collapsed && value === preference ? choices[(choices.indexOf(preference) + 1) % choices.length] : value);
          if (collapsed) group.querySelector('[aria-checked="true"]').focus();
        });
        button.addEventListener('keydown', event => {
          const direction = ['ArrowRight', 'ArrowDown'].includes(event.key) ? 1 :
            ['ArrowLeft', 'ArrowUp'].includes(event.key) ? -1 : 0;
          if (!direction) return;
          event.preventDefault();
          choose(choices[(choices.indexOf(preference) + direction + choices.length) % choices.length]);
          group.querySelector('[aria-checked="true"]').focus();
        });
      }
    }
  }
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', bind, { once: true });
  else bind();
  media.addEventListener('change', apply);
  window.addEventListener('pageshow', () => {
    const stored = read('day-theme');
    if (choices.includes(stored)) preference = stored;
    apply();
  });
  window.addEventListener('storage', event => {
    if (event.key === 'day-theme') {
      preference = choices.includes(event.newValue) ? event.newValue : 'system';
      apply();
    }
  });
  if (rustdoc) {
    new MutationObserver(() => {
      if (root.dataset.theme === resolved()) return;
      choose(read('rustdoc-use-system-theme') === 'true' ? 'system' :
        root.dataset.theme === 'light' ? 'light' : 'dark');
    }).observe(root, { attributes: true, attributeFilter: ['data-theme'] });
  }
})();
