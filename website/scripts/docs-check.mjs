// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

// A deliberately scoped editorial check using the website's existing Markdown parser.
// Rule intent and limits: website/authoring/authoring.md.
import { createMarkdownProcessor, parseFrontmatter } from '@astrojs/markdown-remark';
import { readFile, readdir, realpath, stat } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { execFileSync } from 'node:child_process';
import { extname, isAbsolute, relative, resolve } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';
import { glossary } from '../src/lib/glossary.mjs';

export const root = fileURLToPath(new URL('../../', import.meta.url));
const requiredTerms = ['piece', 'part', 'signal', 'target', 'toolkit', 'backend', 'widget', 'control', 'window'];
const textOf = (node) => node.value ?? (node.children ?? []).map(textOf).join('');

export async function inspectMarkdown(source, { bodyTitle = false } = {}) {
  const issues = [];
  const links = [];
  const issue = (node, rule, message) => issues.push({ line: node.position?.start.line ?? 1, rule, message });
  const { frontmatter, content } = parseFrontmatter(source, { frontmatter: 'empty-with-lines' });
  // Empty front matter lines preserve source diagnostics.
  const definitions = new Map();
  let previous = frontmatter.title ? 1 : 0;
  let titles = frontmatter.title && !bodyTitle ? 1 : 0;
  const processor = await createMarkdownProcessor({
    syntaxHighlight: false,
    smartypants: false,
    remarkPlugins: [() => (tree) => {
      const collect = (node) => {
        if (node.type === 'definition') definitions.set(node.identifier, node.url);
        (node.children ?? []).forEach(collect);
      };
      collect(tree);
      const visit = (node) => {
        if (node.type === 'heading') {
          if (node.depth === 1) titles++;
          if (node.depth > previous + 1) issue(node, 'heading-order', 'Do not skip heading levels.');
          previous = node.depth;
        }
        if (node.type === 'code') {
          if (!node.lang) issue(node, 'code-language', 'Specify a code fence language; use text for conceptual output.');
          // CommonMark accepts unclosed fences. Require an explicit matching closing fence.
          const lines = content.split('\n');
          const first = lines[node.position.start.line - 1].slice(node.position.start.column - 1);
          const fence = /^(`{3,}|~{3,})/.exec(first)?.[1];
          const last = lines[node.position.end.line - 1];
          if (fence && (node.position.start.line === node.position.end.line ||
              !new RegExp(`^\\s*(?:>\\s*)*${fence[0]}{${fence.length},}\\s*$`).test(last))) {
            issue(node, 'code-closure', 'Close the code fence.');
          }
          return;
        }
        if (['inlineCode', 'html'].includes(node.type)) return;
        if (node.type === 'text' && /\b(simply|obviously|easy|easily|seamlessly)\b/i.test(node.value)) {
          issue(node, 'prose', 'Replace difficulty judgments or hype with concrete behavior.');
        }
        if (node.type === 'text' && /\bwhy\s+it\s+matters\b/i.test(node.value)) {
          issue(node, 'prose', 'Name the topic or concrete benefit instead of using a generic importance heading.');
        }
        if (['link', 'linkReference', 'image', 'imageReference'].includes(node.type)) {
          const image = node.type.startsWith('image');
          const label = image ? node.alt : textOf(node);
          if (!label?.trim() || (!image && /^(click here|here|read more)$/i.test(label.trim()))) {
            issue(node, image ? 'image-alt' : 'link-label', 'Provide a descriptive label.');
          }
          const url = node.url ?? definitions.get(node.identifier);
          if (url !== undefined) links.push({ url, line: node.position.start.line });
          else issue(node, 'link-definition', 'Define the reference link.');
        }
        (node.children ?? []).forEach(visit);
      };
      visit(tree);
    }],
  });
  const rendered = await processor.render(content);
  if (titles !== 1) issues.push({ line: 1, rule: 'title', message: 'Provide exactly one title (H1 or front matter title).' });
  return { issues, links, anchors: new Set(rendered.metadata.headings.map((h) => h.slug)) };
}

export function checkGlossary(terms) {
  const issues = [];
  const ids = new Set();
  for (const term of terms) {
    if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(term.id) || ids.has(term.id)) issues.push(`Invalid or duplicate id: ${term.id}`);
    ids.add(term.id);
    if (!term.term?.trim() || !term.definition?.trim() || !term.see?.title?.trim() ||
        !/^\/(?:docs|contributing)\//.test(term.see?.href ?? '')) issues.push(`Incomplete glossary entry: ${term.id}`);
  }
  for (const id of requiredTerms) if (!ids.has(id)) issues.push(`Missing core term: ${id}`);
  return issues;
}

async function markdownFiles(directory) {
  const files = [];
  for (const entry of await readdir(directory, { withFileTypes: true })) {
    const path = resolve(directory, entry.name);
    if (entry.isDirectory()) files.push(...await markdownFiles(path));
    else if (entry.name.endsWith('.md')) files.push(path);
  }
  return files.sort();
}

function insideRoot(path) {
  const rel = relative(root, path);
  return rel !== '..' && !rel.startsWith('../') && !isAbsolute(rel);
}

function repositoryPaths() {
  // Source archives have no Git metadata; their on-disk checks already use only exported files.
  if (!existsSync(resolve(root, '.git'))) return null;
  // Include new, non-ignored files so authors can validate before staging. Ignored local files
  // must not make public links pass when a clean CI checkout would lack their destinations.
  const names = execFileSync('git', ['ls-files', '-z', '--cached', '--others', '--exclude-standard'],
    { cwd: root, encoding: 'utf8' }).split('\0').filter(Boolean);
  const paths = new Set(['']);
  for (const name of names) {
    const segments = name.split('/');
    while (segments.length) {
      paths.add(segments.join('/'));
      segments.pop();
    }
  }
  return paths;
}

export async function checkFiles(paths, { sourcePaths = null } = {}) {
  const cache = new Map();
  const parse = async (path) => {
    if (!cache.has(path)) cache.set(path, inspectMarkdown(await readFile(path, 'utf8'), { bodyTitle: relative(root, path).startsWith('docs/') }));
    return cache.get(path);
  };
  const errors = [];
  let checkedLinks = 0;
  let skippedLinks = 0;
  for (const path of paths) {
    const label = relative(root, path);
    if (!insideRoot(await realpath(path)) || extname(path) !== '.md') {
      throw new Error(`Only repository-local plain Markdown is supported: ${label}`);
    }
    const result = await parse(path);
    for (const issue of result.issues) errors.push(`${label}:${issue.line}: ${issue.rule}: ${issue.message}`);
    for (const link of result.links) {
      if (/^(?:[a-z][a-z0-9+.-]*:|\/)/i.test(link.url)) { skippedLinks++; continue; }
      try {
        const url = new URL(link.url, pathToFileURL(path));
        const target = fileURLToPath(url);
        const canonical = await realpath(target);
        if (!insideRoot(target) || !insideRoot(canonical)) throw new Error('link escapes repository');
        if (sourcePaths && [target, canonical].some((p) => !sourcePaths.has(relative(root, p).split('\\').join('/')))) {
          throw new Error('link targets an ignored local file outside the repository sources');
        }
        const info = await stat(target);
        if (url.hash && extname(target) === '.md') {
          const destination = await parse(target);
          if (!destination.anchors.has(decodeURIComponent(url.hash.slice(1)))) throw new Error('missing Markdown heading fragment');
        } else if (url.hash) {
          // Source line anchors and HTML/MDX ids need separate review or rendered validation.
          skippedLinks++;
        }
        if (!info.isFile() && !info.isDirectory()) throw new Error('not a file or directory');
        checkedLinks++;
      } catch (error) {
        errors.push(`${label}:${link.line}: local-link: ${link.url}: ${error.message}`);
      }
    }
  }
  return { errors, checkedLinks, skippedLinks };
}

async function main() {
  const args = process.argv.slice(2);
  const paths = args.length ? args.map((p) => resolve(root, p)) : [...new Set([
    ...await markdownFiles(resolve(root, 'website/authoring')),
    ...await markdownFiles(resolve(root, 'docs')),
    ...await markdownFiles(resolve(root, 'website/src/content/docs')),
    ...['CONTRIBUTING.md'].map((name) => resolve(root, name)),
  ])];
  const result = await checkFiles(paths, { sourcePaths: repositoryPaths() });
  const glossaryIssues = checkGlossary(glossary);
  const errors = [...result.errors, ...glossaryIssues.map((message) => `glossary: ${message}`)];
  for (const message of errors) console.error(message);
  console.log(`docs:check: ${errors.length ? 'FAIL' : 'PASS'} — ${paths.length} Markdown files, ${glossary.length} glossary entries, ${result.checkedLinks} local links; ${result.skippedLinks} external/site/unsupported fragment checks skipped.`);
  if (errors.length) process.exitCode = 1;
}

if (process.argv[1] && pathToFileURL(resolve(process.argv[1])).href === import.meta.url) {
  main().catch((error) => { console.error(`docs:check: ${error.message}`); process.exitCode = 1; });
}
