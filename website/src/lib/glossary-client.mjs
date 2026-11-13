// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
export function initGlossary() {
  // Plain glossary links remain usable when popovers are unavailable.
  if (!('showPopover' in HTMLElement.prototype)) return;
  const anchored = CSS.supports('position-anchor: --term') && CSS.supports('position-area: bottom');
  let current;
  let dismiss;
  let changing = false;
  const place = () => {
    if (anchored || !current) return;
    const { term, pop } = current;
    const r = term.getBoundingClientRect();
    const p = pop.getBoundingClientRect();
    pop.style.left = `${Math.max(12, Math.min(r.left, innerWidth - p.width - 12))}px`;
    const top = r.bottom + 8 + p.height > innerHeight - 12 ? r.top - p.height - 8 : r.bottom + 8;
    pop.style.top = `${Math.max(12, top)}px`;
  };
  document.querySelectorAll('[data-gloss-pop]').forEach(term => {
    const pop = document.getElementById(term.dataset.glossPop);
    if (!pop) return;
    pop.hidden = false;
    let closing = false;
    pop.addEventListener('beforetoggle', event => {
      if (event.newState !== 'closed') return;
      closing = true;
      // Native Escape/light dismissal may restore focus while still closing.
      setTimeout(() => { closing = false; }, 0);
    });
    const cancel = () => clearTimeout(dismiss);
    const open = () => {
      // Hiding another popover can synchronously restore focus to its term.
      if (changing) return;
      cancel();
      changing = true;
      try {
        if (current?.pop !== pop) current?.pop.hidePopover();
        current = { term, pop };
        if (!pop.matches(':popover-open')) pop.showPopover();
      } finally { changing = false; }
      place();
    };
    const close = () => {
      cancel();
      dismiss = setTimeout(() => {
        if (term.matches(':hover') || pop.matches(':hover') || term === document.activeElement || pop.contains(document.activeElement)) return;
        pop.hidePopover();
      }, 160);
    };
    term.addEventListener('mouseenter', open);
    term.addEventListener('focus', () => {
      queueMicrotask(() => { if (!closing && document.activeElement === term) open(); });
    });
    term.addEventListener('mouseleave', close);
    term.addEventListener('blur', close);
    pop.addEventListener('mouseenter', cancel);
    pop.addEventListener('mouseleave', close);
    pop.addEventListener('focusin', cancel);
    pop.addEventListener('focusout', close);
    pop.addEventListener('toggle', () => {
      if (!pop.matches(':popover-open') && current?.pop === pop) current = undefined;
    });
  });
  window.addEventListener('resize', place);
  document.addEventListener('scroll', place, { capture: true, passive: true });
}
