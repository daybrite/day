// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { readRecipe, platformIds, resolveStep, validateLessonSource } from '../src/lib/tutorials.mjs';

/** Expand Markdown step references from the same recipe consumed by CI. */
export default function tutorialSteps() {
  const recipe = readRecipe();
  return (tree, file) => {
    const source = String(file.value ?? '');
    if (!/^<!-- tutorial-step:/m.test(source)) return;
    const name = (file.path ?? '').split(/[\\/]/).pop();
    const lesson = recipe.lessons.find(l => l.source === name);
    if (!lesson) throw new Error(`Tutorial marker in an unregistered lesson: ${name}`);
    validateLessonSource(lesson, source);
    const expand = node => {
      if (!node.children) return;
      node.children = node.children.flatMap(child => {
        if (child.type !== 'html') { expand(child); return [child]; }
        const match = /^<!-- tutorial-step: ([a-z0-9-]+)\/([a-z0-9-]+) -->$/.exec(child.value.trim());
        if (!match) return [child];
        const step = lesson.steps.find(s => s.id === match[2]);
        if (!step || match[1] !== lesson.id) throw new Error(`Unknown tutorial step ${match[0]}`);
        const variants = platformIds.map(p => resolveStep(recipe, lesson, step, p)).filter(Boolean);
        const shared = variants.length === platformIds.length && variants.every(s => s.body === variants[0].body && s.shell === variants[0].shell);
        // Files and notes can be shared even when the command shells differ.
        const common = variants.length === platformIds.length && variants.every(s => s.body === variants[0].body) && !['run', 'launch'].includes(step.kind);
        return (shared || common ? [variants[0]] : variants).flatMap(s => {
          const platform = shared || common ? 'all' : s.platform;
          const heading = platform === 'all' ? [] : [{type:'html', value:`<p class="tutorial-platform-label">${recipe.platforms[platform].label}</p>`}];
          const caption = s.kind === 'file' ? [{type:'paragraph', children:[
            {type:'text', value:s.mode === 'append' ? 'Append to ' : 'Write '},
            {type:'inlineCode', value:s.path}, {type:'text', value:':'},
          ]}] : [];
          const copy = s.kind === 'note' ? [] : [{type:'html', value:
            `<button type="button" class="tutorial-copy" data-copy-text="${encodeURIComponent(s.body)}" hidden>Copy ${s.kind === 'file' ? 'code' : 'commands'}</button>`}];
          const content = s.kind === 'note'
            ? { type: 'paragraph', children: [{type:'text', value:s.body}] }
            : { type: 'code', lang: s.kind === 'file' ? s.language : s.shell === 'powershell' ? 'powershell' : 'sh', value:s.body.replace(/\n$/, '') };
          return [{type:'html', value:`<div class="tutorial-step" data-tutorial-platform="${platform}" data-tutorial-step="${s.key}">`}, ...heading, ...caption, ...copy, content, {type:'html', value:'</div>'}];
        });
      });
    };
    expand(tree);
  };
}
