// QuickPick / input flows for the editable configuration: build mode, locale, dayscript, and project.

import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

import { Profile } from "./config";
import { DayProject } from "./project";

export async function pickMode(current: Profile): Promise<Profile | undefined> {
  const items: (vscode.QuickPickItem & { value: Profile })[] = [
    { label: "debug", description: "fast builds, unoptimized", value: "debug", picked: current === "debug" },
    { label: "release", description: "optimized", value: "release", picked: current === "release" },
  ];
  const chosen = await vscode.window.showQuickPick(items, { title: "Day: Build Mode", placeHolder: current });
  return chosen?.value;
}

/** Returns "" for the default locale, a BCP-47 string, or undefined if cancelled. */
export async function pickLocale(project: DayProject | undefined, current: string): Promise<string | undefined> {
  const known = new Set<string>(["en", "fr", "en-XA", "fr-XA"]);
  // Add any locales the project ships (folders under <root>/locales).
  if (project) {
    const dir = path.join(project.root, "locales");
    try {
      for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        if (e.isDirectory()) {
          known.add(e.name);
        }
      }
    } catch {
      /* no locales dir */
    }
  }
  const CUSTOM = "$(edit) Custom…";
  const items: vscode.QuickPickItem[] = [
    { label: "(default)", description: "the app/system default" },
    ...[...known].sort().map((l) => ({ label: l } as vscode.QuickPickItem)),
    { label: CUSTOM },
  ];
  const chosen = await vscode.window.showQuickPick(items, {
    title: "Day: Locale",
    placeHolder: current.length > 0 ? current : "(default)",
  });
  if (!chosen) {
    return undefined;
  }
  if (chosen.label === "(default)") {
    return "";
  }
  if (chosen.label === CUSTOM) {
    const typed = await vscode.window.showInputBox({
      title: "Day: Locale",
      prompt: "BCP-47 locale tag (e.g. de, ja, en-XA)",
      value: current,
    });
    return typed === undefined ? undefined : typed.trim();
  }
  return chosen.label;
}

/** Returns "" for no script, an absolute script path, or undefined if cancelled. */
export async function pickScript(project: DayProject | undefined, current: string): Promise<string | undefined> {
  const scripts: string[] = [];
  if (project) {
    const dir = path.join(project.root, "scripts");
    try {
      for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
        if (e.isFile() && /\.(ya?ml|day)$/.test(e.name)) {
          scripts.push(path.join(dir, e.name));
        }
      }
    } catch {
      /* no scripts dir */
    }
  }
  const BROWSE = "$(folder-opened) Browse…";
  const items: vscode.QuickPickItem[] = [
    { label: "(none)", description: "run without a dayscript" },
    ...scripts.map((s) => ({ label: path.basename(s), description: s } as vscode.QuickPickItem)),
    { label: BROWSE },
  ];
  const chosen = await vscode.window.showQuickPick(items, {
    title: "Day: Dayscript",
    placeHolder: current.length > 0 ? path.basename(current) : "(none)",
  });
  if (!chosen) {
    return undefined;
  }
  if (chosen.label === "(none)") {
    return "";
  }
  if (chosen.label === BROWSE) {
    const uris = await vscode.window.showOpenDialog({
      title: "Select a dayscript",
      canSelectMany: false,
      filters: { Dayscript: ["yaml", "yml", "day"] },
      defaultUri: project ? vscode.Uri.file(project.root) : undefined,
    });
    return uris && uris.length > 0 ? uris[0].fsPath : undefined;
  }
  return chosen.description ?? chosen.label;
}

export async function pickProject(projects: DayProject[], current?: string): Promise<DayProject | undefined> {
  if (projects.length === 0) {
    return undefined;
  }
  const items = projects.map((p) => ({
    label: p.name,
    description: p.id,
    detail: p.root,
    picked: p.root === current,
    project: p,
  }));
  const chosen = await vscode.window.showQuickPick(items, { title: "Day: Select Project" });
  return chosen?.project;
}
