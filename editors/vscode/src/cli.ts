// Resolve how to invoke the `day` CLI, and build its argument vectors.
//
// Resolution order for the default `day.cliPath` ("day"):
//   1. an explicit `day.cliPath` set to something other than "day" → use it verbatim;
//   2. otherwise, if the workspace is the Day repo (a Cargo workspace with a `day-cli` member),
//      fall back to `cargo run -q -p day-cli --` (so it works in-repo with no released binary);
//   3. otherwise `day` (expected on PATH).

import * as fs from "fs";
import * as path from "path";
import * as vscode from "vscode";

export interface DayCli {
  /** The executable to spawn (e.g. "day" or "cargo"). */
  command: string;
  /** Args that precede the day subcommand (e.g. ["run","-q","-p","day-cli","--"] for the fallback). */
  baseArgs: string[];
  /** Working directory for the process (the day repo root, for the cargo fallback). */
  cwd?: string;
  /** A human-readable rendering of the resolved command, for logs/tooltips. */
  display: string;
}

/** Walk up from `start` (and across workspace roots) to a dir containing `crates/day-cli/Cargo.toml`. */
export function findDayRepoRoot(start?: string): string | undefined {
  const seeds: string[] = [];
  if (start) {
    seeds.push(start);
  }
  for (const f of vscode.workspace.workspaceFolders ?? []) {
    seeds.push(f.uri.fsPath);
  }
  for (const seed of seeds) {
    let dir = seed;
    // Bound the walk to avoid touching the whole filesystem.
    for (let i = 0; i < 12; i++) {
      if (fs.existsSync(path.join(dir, "crates", "day-cli", "Cargo.toml"))) {
        return dir;
      }
      const parent = path.dirname(dir);
      if (parent === dir) {
        break;
      }
      dir = parent;
    }
  }
  return undefined;
}

export function resolveCli(projectDir?: string): DayCli {
  const cfg = vscode.workspace.getConfiguration("day");
  const cliPath = (cfg.get<string>("cliPath") ?? "day").trim();

  if (cliPath && cliPath !== "day") {
    return { command: cliPath, baseArgs: [], display: cliPath };
  }

  const repo = findDayRepoRoot(projectDir);
  if (repo) {
    return {
      command: "cargo",
      baseArgs: ["run", "-q", "-p", "day-cli", "--"],
      cwd: repo,
      display: "cargo run -q -p day-cli --",
    };
  }

  return { command: "day", baseArgs: [], display: "day" };
}

export interface LaunchOptions {
  projectRoot: string;
  target: string;
  profile: "debug" | "release";
  locale?: string;
  script?: string;
  env?: Record<string, string>;
}

function projectArgs(projectRoot: string): string[] {
  // Omit --project when unknown so the CLI falls back to cwd-based day.yaml discovery.
  return projectRoot && projectRoot.length > 0 ? ["--project", projectRoot] : [];
}

/** Args for `day launch` (a single target). */
export function launchArgs(o: LaunchOptions): string[] {
  const args = [...projectArgs(o.projectRoot), "launch", "-p", o.target, "--profile", o.profile];
  if (o.locale && o.locale.length > 0) {
    args.push("--locale", o.locale);
  }
  if (o.script && o.script.length > 0) {
    args.push("--script", o.script);
  }
  for (const [k, v] of Object.entries(o.env ?? {})) {
    args.push("--env", `${k}=${v}`);
  }
  return args;
}

/** Args for `day build` (a single target). */
export function buildArgs(projectRoot: string, target: string, profile: "debug" | "release"): string[] {
  return [...projectArgs(projectRoot), "build", "-p", target, "--profile", profile];
}

/** A shell-safe rendering of a command for display in a terminal/log line. */
export function renderCommand(cli: DayCli, args: string[]): string {
  const all = [cli.command, ...cli.baseArgs, ...args];
  return all.map((a) => (/\s/.test(a) ? JSON.stringify(a) : a)).join(" ");
}
