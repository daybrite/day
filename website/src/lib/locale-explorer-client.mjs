// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
export function initializeLocaleExplorer(root) {
  const captures = JSON.parse(root.querySelector('[data-captures]').textContent);
  const stage = root.querySelector('.explorer-stage');
  const choices = [...root.querySelectorAll('[data-axis]')];
  const current = { theme: stage.lastElementChild.dataset.theme };
  const siteTheme = () => document.documentElement.dataset.theme === 'dark' ? 'dark' : 'light';
  for (const button of choices) if (button.getAttribute('aria-pressed') === 'true') current[button.dataset.axis] = button.dataset.value;
  const error = root.querySelector('.explorer-error');
  let revision = 0, hoverTimer;
  const matches = (selection) => captures.find(c => ['target', 'locale', 'theme'].every(axis => c[axis] === selection[axis]));
  function updateControls() {
    for (const button of choices) {
      const { axis, value } = button.dataset;
      button.setAttribute('aria-pressed', String(current[axis] === value));
      button.disabled = !matches({ ...current, [axis]: value });
    }
  }
  async function select(axis, value) {
    clearTimeout(hoverTimer);
    const next = { ...current, [axis]: value, theme: siteTheme() };
    if (Object.keys(next).every(key => current[key] === next[key]) && error.hidden) return;
    const capture = matches(next);
    if (!capture) return;
    Object.assign(current, next);
    updateControls();
    const request = ++revision;
    stage.setAttribute('aria-busy', 'true'); error.hidden = true;
    const slide = document.createElement('div');
    slide.className = 'explorer-slide'; slide.dataset.theme = capture.theme;
    slide.dataset.capture = `${capture.target}/${capture.theme}/${capture.locale}`;
    const images = document.createElement('div');
    images.className = 'explorer-images' + (capture.mobile ? ' is-mobile' : '');
    const pending = [];
    for (const shot of capture.images) {
      const figure = document.createElement('figure'), link = document.createElement('a');
      const img = new Image(shot.width, shot.height);
      figure.style.setProperty('--shot-ratio', shot.width / shot.height);
      link.href = shot.src; link.target = '_blank'; link.rel = 'noopener';
      img.alt = `Day Travel · ${capture.platform} · ${capture.language} · ${capture.appearance} · ${shot.label}`;
      img.src = shot.src; pending.push(img.decode());
      const template = root.querySelector(`template[data-explorer-shell="${capture.target}"]`);
      if (template) {
        // Import into the live document so moving the image into the shell does not cancel decoding.
        const shell = document.importNode(template.content, true);
        shell.querySelector('[data-screen-slot]').replaceWith(img);
        const window = shell.querySelector('.sh-win');
        if (window) {
          window.style.setProperty('--ar', shot.width / shot.height);
          window.toggleAttribute('data-dark', capture.theme === 'dark');
        }
        link.append(shell);
      } else link.append(img);
      figure.append(link); images.append(figure);
    }
    const loaded = await Promise.allSettled(pending);
    if (request !== revision) return; // A slower image must not override a later selection.
    stage.setAttribute('aria-busy', 'false');
    if (loaded.some(result => result.status === 'rejected')) {
      // Keep the last complete screenshot and restore its matching controls on failure.
      const [target, theme, locale] = stage.lastElementChild.dataset.capture.split('/');
      Object.assign(current, { target, theme, locale }); updateControls(); error.hidden = false; return;
    }
    const caption = document.createElement('p');
    caption.className = 'explorer-caption';
    caption.textContent = `${capture.platform} · ${capture.language} · ${capture.appearance}`;
    slide.append(caption, images);
    const old = [...stage.children];
    for (const node of old) { node.inert = true; node.setAttribute('aria-hidden', 'true'); }
    stage.append(slide);
    // Fade an opaque composition OVER the old one. Fading both layers out/in exposes
    // the empty stage and flashes. Keep interrupted fades intact beneath their successor.
    if (!matchMedia('(prefers-reduced-motion: reduce)').matches) {
      const animation = slide.animate([{ opacity: 0 }, { opacity: 1 }], { duration: 480, easing: 'cubic-bezier(.4, 0, .2, 1)' });
      animation.finished.then(() => old.forEach(node => node.remove()));
    } else old.forEach(node => node.remove());
    root.querySelector('[data-platform]').textContent = capture.platform;
    root.querySelector('[data-detail]').textContent = `${capture.language} · ${capture.appearance}`;
  }
  for (const button of choices) {
    const choose = () => select(button.dataset.axis, button.dataset.value);
    button.addEventListener('pointerenter', event => {
      clearTimeout(hoverTimer);
      if (event.pointerType === 'mouse' && !button.disabled) hoverTimer = setTimeout(choose, 90);
    });
    button.addEventListener('pointerleave', () => clearTimeout(hoverTimer));
    button.addEventListener('click', choose);
    button.addEventListener('focus', choose);
  }
  for (const group of root.querySelectorAll('.explorer-controls')) {
    group.hidden = false;
    group.addEventListener('keydown', event => {
      const buttons = [...group.querySelectorAll('button:not(:disabled)')], index = buttons.indexOf(document.activeElement);
      if (index < 0) return;
      const delta = ['ArrowRight', 'ArrowDown'].includes(event.key) ? 1 : ['ArrowLeft', 'ArrowUp'].includes(event.key) ? -1 : 0;
      const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1 : delta ? (index + delta + buttons.length) % buttons.length : null;
      if (next !== null) { event.preventDefault(); buttons[next].focus(); }
    });
  }
  updateControls();
  // The shared theme script resolves saved preferences, system appearance, and
  // changes in other tabs onto <html data-theme>. Follow that single source.
  new MutationObserver(() => select('theme', siteTheme())).observe(document.documentElement, {
    attributes: true, attributeFilter: ['data-theme'],
  });
  select('theme', siteTheme());
}
