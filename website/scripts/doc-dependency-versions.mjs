// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
// Published documentation names no Day release. Every crate here releases often, so a version in
// an example goes stale with the next one: install commands are `cargo add <crate>`, which takes
// the newest release; CLI examples say `--day-version latest` (or an `X.Y.Z` placeholder); API
// links use docs.rs's `latest`. Path, git, and workspace dependencies carry no release and pass.
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'smol-toml';
import yaml from 'js-yaml';

export const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url));
const text = (path) => readFileSync(path, 'utf8');
const json = (path) => JSON.parse(text(path));
const lineOf = (source, offset) => source.slice(0, offset).split('\n').length;
const isDay = (pkg) => pkg === 'day' || pkg.startsWith('day-');

export function checkCargo(source, location = 'Cargo example') {
  const errors = [];
  let document;
  try { document = parse(source); }
  catch (error) { return [`${location}: invalid Cargo TOML: ${error.message}`]; }
  const dependency = (alias, value) => {
    const pkg = value?.package ?? (alias === 'day' ? 'day-app' : alias);
    if (!isDay(pkg)) return;
    const requirement = typeof value === 'string' ? value : value?.version;
    if (requirement === undefined) return;
    const add = pkg === 'day-app' && alias === 'day' ? 'cargo add day-app --rename day' : `cargo add ${pkg}`;
    errors.push(`${location}: ${pkg} names the release ${JSON.stringify(requirement)}; show \`${add}\`, which takes the newest one.`);
  };
  const visit = (object) => {
    if (!object || typeof object !== 'object' || Array.isArray(object)) return;
    for (const [key, value] of Object.entries(object)) {
      if (['dependencies', 'build-dependencies', 'dev-dependencies'].includes(key)) {
        for (const [alias, spec] of Object.entries(value)) dependency(alias, spec);
      } else visit(value);
    }
  };
  visit(document);
  // Short snippets often omit the [dependencies] heading.
  for (const [alias, spec] of Object.entries(document)) {
    if (typeof spec === 'string' || spec?.package || spec?.version) dependency(alias, spec);
  }
  return errors;
}

const hasDayDependency = (source) => /(?:^|\n)\s*(?:["']?day(?:[-_][\w-]+)?["']?\s*=|\[(?:[^\]\n]*\.)?(?:dependencies|build-dependencies|dev-dependencies)\.["']?(?:day|day-[\w-]+)["']?\])|\bpackage\s*=\s*["']day-[\w-]+["']/.test(source);

export function checkDocument(source, location = 'document') {
  const errors = [];
  for (const match of source.matchAll(/^ {0,3}(?:```|~~~)toml\s*\n([\s\S]*?)^ {0,3}(?:```|~~~)\s*$/gm)) {
    if (hasDayDependency(match[1])) errors.push(...checkCargo(match[1], `${location}:${lineOf(source, match.index)}`));
  }
  // Inline Cargo entries are copyable too. Fenced blocks were already handled above.
  const prose = source.replace(/^ {0,3}(?:```|~~~)[\s\S]*?^ {0,3}(?:```|~~~)\s*$/gm, '');
  for (const match of prose.matchAll(/`([^`\n]+)`/g)) {
    if (hasDayDependency(match[1])) errors.push(...checkCargo(match[1], location));
  }
  for (const match of source.matchAll(/--day-version(?:\s+|=)["']?(v?\d+\.\d+\.\d+)\b/g)) {
    errors.push(`${location}:${lineOf(source, match.index)}: --day-version ${match[1]} names a release; use latest, main, or an X.Y.Z placeholder.`);
  }
  for (const match of source.matchAll(/https:\/\/docs\.rs\/(day-[\w-]+)\/(\d+\.\d+\.\d+)\//g)) {
    errors.push(`${location}:${lineOf(source, match.index)}: ${match[1]} API URL names ${match[2]}; use https://docs.rs/${match[1]}/latest/.`);
  }
  return errors;
}

function filesUnder(directory) {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const path = resolve(directory, entry.name);
    return entry.isDirectory() ? filesUnder(path) : [path];
  });
}

export function checkRepository(root = repositoryRoot) {
  const routes = json(resolve(root, 'website/src/data/content-routes.json'));
  const sources = new Set(Object.values(routes).filter((p) => p.published !== false).map((p) => p.source));
  for (const name of ['README.md', 'CONTRIBUTING.md']) if (existsSync(resolve(root, name))) sources.add(name);
  for (const file of filesUnder(resolve(root, 'crates'))) {
    if (/\/README\.md$/i.test(file)) sources.add(relative(root, file));
  }
  for (const file of filesUnder(resolve(root, 'website/authoring'))) {
    if (/\.md$/.test(file)) sources.add(relative(root, file));
  }
  const errors = [];
  for (const source of [...sources].sort()) errors.push(...checkDocument(text(resolve(root, source)), source));
  const visitRecipe = (value, path) => {
    if (typeof value === 'string') {
      if (hasDayDependency(value)) errors.push(...checkCargo(value, path));
      errors.push(...checkDocument(value, path));
    } else if (value && typeof value === 'object') {
      for (const [key, child] of Object.entries(value)) visitRecipe(child, `${path}.${key}`);
    }
  };
  const recipes = filesUnder(resolve(root, 'website/tutorials')).filter((f) => /\.ya?ml$/.test(f));
  for (const file of recipes) visitRecipe(yaml.load(text(file)), relative(root, file));
  // The site's own catalog of external crates links their API pages too.
  const external = resolve(root, 'website/src/data/external-crates.json');
  if (existsSync(external)) {
    for (const crate of json(external)) errors.push(...checkDocument(crate.api ?? '', `external-crates.json ${crate.pkg}`));
  }
  if (errors.length) throw new Error(`Documentation names Day releases:\n${errors.join('\n')}`);
  return { documents: sources.size, recipes: recipes.length };
}
