// A TaskProvider for the `day` task type. It auto-detects build/launch tasks for the current
// project's buildable targets (so they appear under "Run Task…"), and resolves tasks written by
// hand in tasks.json. This is the standard-conventions hook: `day` tasks integrate with the Tasks
// system, Ctrl+Shift+B, and key bindings.

import * as vscode from "vscode";

import { State } from "./config";
import { DayProject } from "./project";
import { isBuildableHere, findTarget } from "./targets";
import { buildDayTask, DayTaskDefinition } from "./tasks";

export class DayTaskProvider implements vscode.TaskProvider {
  static readonly type = "day";

  constructor(
    private readonly state: State,
    private readonly currentProject: () => DayProject | undefined,
  ) {}

  provideTasks(): vscode.Task[] {
    const project = this.currentProject();
    if (!project) {
      return [];
    }
    const tasks: vscode.Task[] = [];
    for (const name of project.targets) {
      const target = findTarget(name);
      if (!target || !isBuildableHere(target)) {
        continue;
      }
      const profile = this.state.selection.profile;
      for (const command of ["launch", "build"] as const) {
        tasks.push(
          buildDayTask({ type: "day", command, target: name, profile, project: project.root }),
        );
      }
    }
    return tasks;
  }

  resolveTask(task: vscode.Task): vscode.Task | undefined {
    const def = task.definition as DayTaskDefinition;
    if (def.type !== "day" || !def.target || (def.command !== "build" && def.command !== "launch")) {
      return undefined;
    }
    // Default the project to the current one if the hand-written task omitted it.
    const project = def.project ?? this.currentProject()?.root;
    return buildDayTask({ ...def, project });
  }
}
