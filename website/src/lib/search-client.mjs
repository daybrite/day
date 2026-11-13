// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { res } from '../generated/strings.mjs';
import { lightDismiss } from './dialog.mjs';
export function initSearch() {
  const dialog = document.querySelector('[data-search-dialog]');
  if (!dialog) return;
  const mount = dialog.querySelector('[data-search-mount]');
  const status = dialog.querySelector('[data-search-status]');
  const retry = dialog.querySelector('[data-search-retry]');
  let loading;
  let ready = false;
  let revision = 0;
  function load() {
    if (ready) return Promise.resolve();
    if (loading) return loading;
    status.hidden = false;
    status.textContent = res.str.search_loading();
    retry.hidden = true;
    loading = new Promise((resolve, reject) => {
      if (!document.querySelector('[data-pagefind-style]')) {
        const css = document.createElement('link');
        css.rel = 'stylesheet'; css.href = dialog.dataset.pagefindCss;
        css.dataset.pagefindStyle = '';
        css.onerror = () => css.remove();
        document.head.append(css);
      }
      const script = document.createElement('script');
      script.src = dialog.dataset.pagefindJs;
      script.onload = () => {
        try {
          mount.replaceChildren();
          new window.PagefindUI({ element: mount, showSubResults: true, showImages: false, resetStyles: false });
          ready = true;
          resolve();
        } catch (error) { reject(error); }
        script.remove();
      };
      script.onerror = () => { script.remove(); reject(new Error('Pagefind unavailable')); };
      document.head.append(script);
    }).then(() => { status.hidden = true; }, () => {
      status.textContent = res.str.search_unavailable();
      retry.hidden = false;
    }).finally(() => { loading = undefined; });
    return loading;
  }
  async function open() {
    const request = ++revision;
    if (!dialog.open) dialog.showModal();
    await load();
    if (dialog.open && request === revision && ready) mount.querySelector('input')?.focus();
  }
  dialog.addEventListener('close', () => { revision++; });
  retry.addEventListener('click', open);
  document.querySelectorAll('[data-search-open]').forEach(button => button.addEventListener('click', open));
  document.addEventListener('keydown', event => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
      event.preventDefault();
      dialog.open ? dialog.close() : open();
    }
  });
  lightDismiss(dialog);
}
