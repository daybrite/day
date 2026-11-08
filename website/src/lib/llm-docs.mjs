// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { unified } from 'unified';
import remarkParse from 'remark-parse';
import remarkStringify from 'remark-stringify';
import remarkGfm from 'remark-gfm';
import { visit } from 'unist-util-visit';
import yaml from 'js-yaml';
import { selection, markdownRoute } from './llm-routes.mjs';
import { routes, isPublished } from './content-routes.mjs';
import { readRecipe, resolveStep, platformIds, validateLessonSource } from './tutorials.mjs';
import { resolveContentLink } from '../../plugins/content-links.mjs';

export const repository = fileURLToPath(new URL('../../../', import.meta.url));
const markdown = unified().use(remarkParse).use(remarkGfm).use(remarkStringify, { fences: true });
const text = (node) => node.value ?? node.children?.map(text).join('') ?? '';
export { hasLlmGuide } from './llm-routes.mjs';

export function readGuide(key) {
  const page = routes[key];
  if (!page || !isPublished(page) || page.audience !== 'app developer' || !page.source.endsWith('.md')) {
    throw new Error(`LLM guide must be a published app-development Markdown source: ${key}`);
  }
  const source = readFileSync(new URL(page.source, new URL('../../../', import.meta.url)), 'utf8');
  const match = /^---\r?\n([\s\S]*?)\r?\n---\r?\n/.exec(source);
  if (!match) throw new Error(`LLM guide needs title and description frontmatter: ${page.source}`);
  const data = yaml.load(match[1], { schema: yaml.JSON_SCHEMA });
  if (!data.title || !data.description) throw new Error(`Missing LLM guide metadata: ${key}`);
  return { ...page, key, title: data.title, description: data.description, raw: source, body: source.slice(match[0].length) };
}

/** Explicit sections keep unvalidated reference examples out of the app-building bundle. */
export function selectSections(tree, sections) {
  if (!sections) return tree;
  const selected = new Set(), children = [];
  let include = false;
  for (const node of tree.children) {
    if (node.type === 'heading' && node.depth <= 2) {
      const title = text(node);
      include = sections.includes(title);
      if (include) {
        if (selected.has(title)) throw new Error(`Duplicate LLM section: ${title}`);
        selected.add(title);
      }
    }
    if (include) children.push(node);
  }
  for (const title of sections) if (!selected.has(title)) throw new Error(`Missing LLM section: ${title}`);
  return { ...tree, children };
}

/** Expand the exact same recipe that renders the website and drives native walkthroughs. */
export function expandTutorial(body, lesson, recipe) {
  validateLessonSource(lesson, body);
  return body.replace(/<!-- tutorial-step: ([a-z0-9-]+)\/([a-z0-9-]+) -->/g, (_, id, stepId) => {
    const step = lesson.steps.find((step) => step.id === stepId);
    if (id !== lesson.id || !step) throw new Error(`Unknown LLM tutorial step: ${id}/${stepId}`);
    const variants = platformIds.map((platform) => resolveStep(recipe, lesson, step, platform)).filter(Boolean);
    const common = variants.length === platformIds.length && variants.every((s) => s.body === variants[0].body) &&
      (!['run', 'launch'].includes(step.kind) || variants.every((s) => s.shell === variants[0].shell));
    return (common ? [variants[0]] : variants).map((s) => {
      const label = common ? 'All desktop platforms' : recipe.platforms[s.platform].label;
      if (s.kind === 'note') return `**${label}:** ${s.body.trim()}\n`;
      const caption = s.kind === 'file' ? `${s.mode === 'append' ? 'Append to' : 'Write'} \`${s.path}\`` : `Run in \`${s.cwd}\``;
      const language = s.kind === 'file' ? s.language : s.shell === 'powershell' ? 'powershell' : 'sh';
      const fence = '`'.repeat(Math.max(3, ...[...s.body.matchAll(/`+/g)].map((m) => m[0].length + 1)));
      return `**${label} — ${caption}:**\n\n${fence}${language}\n${s.body.replace(/\n$/, '')}\n${fence}\n`;
    }).join('\n');
  });
}

export function generateLlmDocs(origin = 'https://daybrite.dev') {
  const absolute = (path) => new URL(path, origin).href;
  const recipe = readRecipe();
  const seen = new Set();
  const guides = selection.guides.map((entry) => {
    if (seen.has(entry.key)) throw new Error(`Duplicate LLM guide: ${entry.key}`);
    seen.add(entry.key);
    const guide = readGuide(entry.key);
    let body = guide.body;
    if (entry.validation === 'tutorial') {
      const lesson = recipe.lessons.find((lesson) => lesson.route === guide.route);
      if (!lesson || !guide.source.endsWith(`/` + lesson.source)) throw new Error(`Unregistered LLM tutorial: ${entry.key}`);
      if (entry.sections) throw new Error('Tutorials must include all steps');
      body = expandTutorial(body, lesson, recipe);
    } else if (entry.validation !== 'guidance') throw new Error(`Unknown LLM validation: ${entry.validation}`);
    const tree = selectSections(markdown.parse(body), entry.sections);
    visit(tree, 'code', () => {
      if (entry.validation !== 'tutorial') throw new Error(`Unvalidated code example in LLM guide: ${entry.key}`);
    });
    visit(tree, (node) => {
      if (['link', 'image', 'definition'].includes(node.type)) {
        const resolved = resolveContentLink(node.url, guide.source);
        if (!resolved) throw new Error(`Unpublished link in LLM guide ${entry.key}: ${node.url}`);
        node.url = absolute(resolved.startsWith('#') ? guide.route + resolved : resolved);
      }
      if (node.children) node.children = node.children.filter((child) => child.type !== 'html' || !/^<!--/.test(child.value));
    });
    // Raw HTML/MDX needs an explicit export strategy rather than silently dropping content.
    visit(tree, 'html', () => { throw new Error(`Unsupported HTML in LLM guide: ${entry.key}`); });
    const scope = entry.sections ? 'Selected agent conventions. The complete reference is linked above.\n\n' : '';
    return { ...guide, markdown: `# ${guide.title}\n\nSource: ${absolute(guide.route)}\n\n${scope}${markdown.stringify(tree)}` };
  });
  for (const lesson of recipe.lessons) {
    if (!guides.some((guide) => guide.route === lesson.route)) throw new Error(`LLM bundle omits tutorial lesson: ${lesson.id}`);
  }
  const description = readGuide(selection.introduction).description;
  const scope = 'This bundle contains selected agent conventions and the complete six-lesson desktop app tutorial. Choose one host platform and follow its commands in order. Additional linked references are outside this bundle.';
  const validation = 'Website builds compile every tutorial stage against the checked-out Day sources and replay its interaction assertions with the mock toolkit. These checks do not prove native rendering, SDK installation, packaging, or the latest published crates work. Native macOS, Windows, and Linux walkthroughs run separately in doctor-verify CI.';
  const index = `# Day\n\n> ${description}\n\n${scope}\n\n${validation}\n\n## Start here\n\n` +
    guides.map((guide) => `- [${guide.title}](${absolute(markdownRoute(guide.route))}): ${guide.description}`).join('\n') +
    `\n\n## Combined guide\n\n- [Complete app-building bundle](${absolute('/llms-full.txt')}): The same selected guidance and all tutorial steps in one document.\n`;
  const full = `# Day app-building guidance\n\n> ${description}\n\n${scope}\n\n${validation}\n\n` + guides.map((guide) => `---\n\n${guide.markdown}`).join('\n');
  return { index, full, guides };
}

