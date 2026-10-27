// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import { resolve, relative } from 'node:path';
import { fileURLToPath } from 'node:url';
import { parse } from 'smol-toml';
import yaml from 'js-yaml';

export const repositoryRoot = fileURLToPath(new URL('../../', import.meta.url));
const text = (path) => readFileSync(path, 'utf8');
const json = (path) => JSON.parse(text(path));
const lineOf = (source, offset) => source.slice(0, offset).split('\n').length;

export function versionCatalog(root = repositoryRoot) {
  const workspace = parse(text(resolve(root, 'Cargo.toml')));
  const versions = new Map();
  for (const member of workspace.workspace.members) {
    const path = resolve(root, member, 'Cargo.toml');
    if (!existsSync(path)) throw new Error(`Missing workspace manifest: ${path}`);
    const pkg = parse(text(path)).package;
    const version = pkg.version?.workspace ? workspace.workspace.package.version : pkg.version;
    if (typeof version !== 'string') throw new Error(`Unresolved version for ${pkg.name}`);
    versions.set(pkg.name, { version, core: true });
  }
  for (const pkg of json(resolve(root, 'website/src/data/external-crates.json'))) {
    if (versions.has(pkg.pkg)) throw new Error(`Duplicate documentation version: ${pkg.pkg}`);
    versions.set(pkg.pkg, { version: pkg.version, core: false });
  }
  for (const [pkg, version] of Object.entries(json(resolve(root, 'website/src/data/doc-dependencies.json')))) {
    if (versions.has(pkg)) throw new Error(`Duplicate documentation version: ${pkg}`);
    versions.set(pkg, { version, core: false });
  }
  return versions;
}

export function acceptableVersions({ version, core }) {
  const [major, minor, patch] = version.split('.');
  // Workspace guides use the current release line. External examples require the catalog's
  // minimum patch, except an initial .0 release can use its minor shorthand.
  return new Set([version, ...(core || patch === '0' ? [`${major}.${minor}`] : [])]);
}

export function checkCargo(source, versions, location = 'Cargo example') {
  const errors = [];
  let document;
  try { document = parse(source); }
  catch (error) { return [`${location}: invalid Cargo TOML: ${error.message}`]; }
  const dependency = (alias, value) => {
    const pkg = value?.package ?? (alias === 'day' ? 'day-app' : alias);
    if (!pkg.startsWith('day-') && !versions.has(pkg)) return;
    const requirement = typeof value === 'string' ? value : value?.version;
    // Source checkouts and workspace-inherited examples have no registry version to check.
    if (!requirement) return;
    const expected = versions.get(pkg);
    if (!expected) {
      errors.push(`${location}: ${pkg} has no documented release; add its verified version to the catalog.`);
      return;
    }
    const base = requirement.replace(/^[~^=]/, '');
    const exactMinor = requirement.startsWith('=') && base.split('.').length < 3;
    if (exactMinor || !acceptableVersions(expected).has(base)) {
      errors.push(`${location}: ${pkg} uses ${JSON.stringify(requirement)}; expected ${[...acceptableVersions(expected)].join(' or ')} (catalog ${expected.version}).`);
    }
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

export function checkDocument(source, versions, location = 'document') {
  const errors = [];
  for (const match of source.matchAll(/^ {0,3}(?:```|~~~)toml\s*\n([\s\S]*?)^ {0,3}(?:```|~~~)\s*$/gm)) {
    if (hasDayDependency(match[1])) errors.push(...checkCargo(match[1], versions, `${location}:${lineOf(source, match.index)}`));
  }
  // Inline Cargo entries are copyable too. Fenced blocks were already handled above.
  const prose = source.replace(/^ {0,3}(?:```|~~~)[\s\S]*?^ {0,3}(?:```|~~~)\s*$/gm, '');
  for (const match of prose.matchAll(/`([^`\n]+)`/g)) {
    if (hasDayDependency(match[1])) errors.push(...checkCargo(match[1], versions, location));
  }
  const current = versions.get('day-cli')?.version;
  for (const match of source.matchAll(/--day-version(?:\s+|=)["']?(v?\d+\.\d+\.\d+)\b/g)) {
    if (match[1].replace(/^v/, '') !== current) errors.push(`${location}:${lineOf(source, match.index)}: --day-version ${match[1]} is stale; use ${current}, latest, or a named source.`);
  }
  for (const match of source.matchAll(/https:\/\/docs\.rs\/(day-[\w-]+)\/([^/\s)]+)\//g)) {
    const expected = versions.get(match[1]);
    if (expected && match[2] !== expected.version) errors.push(`${location}:${lineOf(source, match.index)}: ${match[1]} API URL uses ${match[2]}; expected ${expected.version}.`);
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
  const versions = versionCatalog(root);
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
  for (const source of [...sources].sort()) errors.push(...checkDocument(text(resolve(root, source)), versions, source));
  const visitRecipe = (value, path) => {
    if (typeof value === 'string') {
      if (hasDayDependency(value)) errors.push(...checkCargo(value, versions, path));
      errors.push(...checkDocument(value, versions, path));
    } else if (value && typeof value === 'object') {
      for (const [key, child] of Object.entries(value)) visitRecipe(child, `${path}.${key}`);
    }
  };
  for (const file of filesUnder(resolve(root, 'website/tutorials')).filter((f) => /\.ya?ml$/.test(f))) {
    visitRecipe(yaml.load(text(file)), relative(root, file));
  }
  if (errors.length) throw new Error(`Documentation dependency versions:\n${errors.join('\n')}`);
  return { documents: sources.size, crates: versions.size };
}
