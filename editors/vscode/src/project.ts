// Discover and parse Day projects (folders with a `day.yaml` manifest). We read only the handful of
// fields the extension needs (app name/id/title + the declared targets) with a tiny, dependency-free
// parser for the known, flat manifest schema (crates/day-cli/src/meta.rs) — no YAML runtime dep.

import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

export interface DayProject {
  /** Directory containing day.yaml. */
  root: string;
  name: string;
  id: string;
  title?: string;
  /** Targets declared in `targets:` (may be empty). */
  targets: string[];
}

export async function findProjects(): Promise<DayProject[]> {
  const uris = await vscode.workspace.findFiles("**/day.yaml", "**/{node_modules,target,build,out}/**", 100);
  const projects: DayProject[] = [];
  for (const uri of uris) {
    const p = parseDayYaml(uri.fsPath);
    if (p) {
      projects.push(p);
    }
  }
  projects.sort((a, b) => a.name.localeCompare(b.name) || a.root.localeCompare(b.root));
  return projects;
}

function stripComment(line: string): string {
  // Drop a trailing comment: a `#` at line start or preceded by whitespace, ignoring quoted `#`.
  let inSingle = false;
  let inDouble = false;
  for (let i = 0; i < line.length; i++) {
    const c = line[i];
    if (c === "'" && !inDouble) {
      inSingle = !inSingle;
    } else if (c === '"' && !inSingle) {
      inDouble = !inDouble;
    } else if (c === "#" && !inSingle && !inDouble && (i === 0 || /\s/.test(line[i - 1]))) {
      return line.slice(0, i);
    }
  }
  return line;
}

function unquote(v: string): string {
  const s = v.trim();
  if ((s.startsWith('"') && s.endsWith('"')) || (s.startsWith("'") && s.endsWith("'"))) {
    return s.slice(1, -1);
  }
  return s;
}

export function parseDayYaml(file: string): DayProject | undefined {
  let text: string;
  try {
    text = fs.readFileSync(file, "utf8");
  } catch {
    return undefined;
  }
  const root = path.dirname(file);
  let section: "app" | "targets" | "" = "";
  const app: Record<string, string> = {};
  const targets: string[] = [];

  for (const raw of text.split(/\r?\n/)) {
    const line = stripComment(raw);
    if (line.trim().length === 0) {
      continue;
    }
    const indent = line.length - line.trimStart().length;
    const body = line.trim();

    if (indent === 0) {
      const m = /^([A-Za-z0-9_-]+):(.*)$/.exec(body);
      section = m ? (m[1] === "app" ? "app" : m[1] === "targets" ? "targets" : "") : "";
      continue;
    }
    if (section === "app") {
      const m = /^([A-Za-z0-9_-]+):\s*(.*)$/.exec(body);
      if (m && m[2].length > 0) {
        app[m[1]] = unquote(m[2]);
      }
    } else if (section === "targets") {
      const m = /^-\s*(.+)$/.exec(body);
      if (m) {
        targets.push(unquote(m[1]));
      }
    }
  }

  return {
    root,
    name: app["name"] ?? path.basename(root),
    id: app["id"] ?? "",
    title: app["title"],
    targets,
  };
}
