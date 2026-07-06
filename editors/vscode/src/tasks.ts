// Construct a vscode.Task for a `day` build/launch. Shared by the runner (which builds a definition
// from the current selection) and the TaskProvider (which resolves a definition from tasks.json), so
// interactive runs and hand-written tasks behave identically.

import * as vscode from "vscode";

import { buildArgs, launchArgs, renderCommand, resolveCli } from "./cli";
import { Profile } from "./config";

export interface DayTaskDefinition extends vscode.TaskDefinition {
  type: "day";
  command: "build" | "launch";
  target: string;
  profile?: Profile;
  locale?: string;
  script?: string;
  project?: string;
}

export function extraEnv(): Record<string, string> {
  return vscode.workspace.getConfiguration("day").get<Record<string, string>>("extraEnv") ?? {};
}

export function buildDayTask(def: DayTaskDefinition): vscode.Task {
  const projectRoot = def.project ?? "";
  const cli = resolveCli(projectRoot || undefined);
  const profile: Profile = def.profile ?? "debug";

  const args =
    def.command === "launch"
      ? launchArgs({
          projectRoot,
          target: def.target,
          profile,
          locale: def.locale,
          script: def.script,
          env: extraEnv(),
        })
      : buildArgs(projectRoot, def.target, profile);

  const exec = new vscode.ProcessExecution(cli.command, [...cli.baseArgs, ...args], cli.cwd ? { cwd: cli.cwd } : {});
  const name = def.command === "launch" ? `run ${def.target}` : `build ${def.target}`;
  const matchers = def.command === "build" ? ["$rustc"] : [];

  const task = new vscode.Task(def, vscode.TaskScope.Workspace, name, "day", exec, matchers);
  task.presentationOptions = {
    reveal: vscode.TaskRevealKind.Always,
    // A dedicated panel per task identity: each target keeps its own terminal, reused on restart.
    panel: vscode.TaskPanelKind.Dedicated,
    clear: true,
    focus: false,
    showReuseMessage: false,
  };
  task.detail = renderCommand(cli, args);
  return task;
}
