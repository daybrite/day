// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { res } from '../generated/strings.mjs';
export function initClipboard() {
  document.querySelectorAll('[data-cmd], [data-copy-text]').forEach(button => {
    if (button.dataset.clipboardReady) return;
    button.dataset.clipboardReady = 'true';
    button.hidden = !navigator.clipboard?.writeText;
    if (button.hidden) return;
    const label = button.querySelector('.qs-copy-label') ?? button;
    const original = label.textContent;
    const announcement = document.createElement('span');
    announcement.className = 'copy-status';
    announcement.setAttribute('role', 'status');
    button.after(announcement);
    let reset;
    let revision = 0;
    button.addEventListener('click', async () => {
      const request = ++revision;
      clearTimeout(reset);
      let copied = false;
      try {
        await navigator.clipboard.writeText(button.dataset.cmd ?? decodeURIComponent(button.dataset.copyText));
        copied = true;
      } catch { /* Report denial without an unhandled promise rejection. */ }
      if (request !== revision) return;
      label.textContent = copied ? res.str.copied() : res.str.copy_failed();
      announcement.textContent = label.textContent;
      button.classList.toggle('is-copied', copied);
      reset = setTimeout(() => {
        label.textContent = original;
        announcement.textContent = '';
        button.classList.remove('is-copied');
      }, 1800);
    });
  });
}
