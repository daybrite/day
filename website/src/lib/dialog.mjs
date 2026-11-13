// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Older dialog implementations need light dismissal; padding is still inside the dialog.
export function lightDismiss(dialog) {
  if ('closedBy' in HTMLDialogElement.prototype) return;
  let startedOutside = false;
  const outside = event => {
    const r = dialog.getBoundingClientRect();
    return event.clientX < r.left || event.clientX > r.right || event.clientY < r.top || event.clientY > r.bottom;
  };
  dialog.addEventListener('pointerdown', event => { startedOutside = event.target === dialog && outside(event); });
  dialog.addEventListener('pointerup', event => {
    if (startedOutside && event.target === dialog && outside(event)) dialog.close();
    startedOutside = false;
  });
  dialog.addEventListener('pointercancel', () => { startedOutside = false; });
}
