// Copyright © The Daybrite Project
// SPDX-License-Identifier: CC-BY-SA-4.0

/** Scripts that have an uppercase to move to. Georgian's `toLocaleUpperCase` produces Mtavruli
 *  capitals, which is not how a language name is written, and casing a Japanese or Arabic name
 *  does nothing at all — so the rule below is applied only where it means something. */
const BICAMERAL = /^[\p{Script=Latin}\p{Script=Cyrillic}\p{Script=Greek}\p{Script=Armenian}]/u;

/**
 * A locale tag in its own language: `fr` → `Français`, `pt-BR` → `Português (Brasil)`,
 * `zh-CN` → `中文（中国）`.
 *
 * ICU ships the names, so a language an app adds is named the moment its captures appear — there
 * is no table here to extend. `languageDisplay: 'dialect'` is what turns `en-GB` into
 * `British English` rather than `English (United Kingdom)`, and `fallback: 'none'` keeps ICU from
 * inventing a name for a tag it does not know, which comes back as itself. The first letter is
 * uppercased in the tag's own locale, the form the gallery has always shown (`Français`, not
 * `français`).
 */
export function localeLabel(tag: string): string {
  if (!tag || tag === 'default') return tag;
  try {
    const name = new Intl.DisplayNames([tag], {
      type: 'language',
      languageDisplay: 'dialect',
      fallback: 'none',
    }).of(tag);
    if (!name || name === tag) return tag;
    if (!BICAMERAL.test(name)) return name;
    return name.charAt(0).toLocaleUpperCase(tag) + name.slice(1);
  } catch {
    return tag;
  }
}
