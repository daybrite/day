// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

// A service worker can supply policies to a navigation, but cannot change the isolation
// of the document that installed it. Reload only after our controlling worker confirms
// it supports the policies. An older Day offline worker can already control this tab.
export async function prepareIsolation() {
  const marker = 'dayIsolationReload';
  if (globalThis.crossOriginIsolated) {
    if (history.state?.[marker]) {
      const state = {...history.state};
      delete state[marker];
      history.replaceState(state, '');
    }
    return true;
  }
  if (self !== top || !globalThis.isSecureContext
      || history.state?.[marker] === window.__DAY_BUNDLE) return true;
  try {
    const workers = navigator.serviceWorker;
    if (!workers) return true;
    const supported = await new Promise(resolve => {
      let port;
      let finished = false;
      const finish = result => {
        finished = true;
        clearTimeout(timer);
        workers.removeEventListener('controllerchange', check);
        port?.close();
        resolve(result);
      };
      const check = () => {
        port?.close();
        const controller = workers.controller;
        if (!controller) return;
        const channel = new MessageChannel();
        port = channel.port1;
        port.onmessage = event => {
          if (event.data === true && workers.controller === controller) finish(true);
        };
        try {
          controller.postMessage({type: 'day-cross-origin-isolation'}, [channel.port2]);
        } catch { /* An updating worker may have become redundant. Wait for controllerchange. */ }
      };
      const timer = setTimeout(() => finish(false), 30000);
      workers.addEventListener('controllerchange', check);
      check();
      workers.register(new URL('./sw.js', document.baseURI)).then(registration => {
        if (!registration) { finish(false); return; }
        if (!finished && workers.controller && !registration.installing) registration.update().catch(() => {});
      }, () => finish(false));
    });
    if (supported) {
      history.replaceState({...history.state, [marker]: window.__DAY_BUNDLE}, '');
      location.reload();
      return false;
    }
  } catch { /* Browser policy can disable service workers. Let the app report unavailable storage. */ }
  return true;
}
