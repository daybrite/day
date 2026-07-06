// Day extension entry point: discovers day.yaml projects, wires the sidebar tree, status bar, task
// provider, and commands, and drives build/run/stop/restart through the Runner (Tasks API).

import * as vscode from "vscode";

import { renderCommand, resolveCli } from "./cli";
import { State } from "./config";
import { DayProject, findProjects } from "./project";
import { pickLocale, pickMode, pickProject, pickScript } from "./quickpicks";
import { Runner } from "./runner";
import { StatusBar } from "./statusbar";
import { DayTaskProvider } from "./taskProvider";
import { findTarget, isBuildableHere } from "./targets";
import { DayTree, Node } from "./tree";

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  const state = new State(context.workspaceState);
  const runner = new Runner(state);
  context.subscriptions.push(runner);

  let projects: DayProject[] = [];

  const currentProject = (): DayProject | undefined => {
    const root = state.selection.projectRoot;
    return projects.find((p) => p.root === root) ?? projects[0];
  };

  const refreshProjects = async (): Promise<void> => {
    projects = await findProjects();
    const root = state.selection.projectRoot;
    if ((!root || !projects.find((p) => p.root === root)) && projects.length > 0) {
      await state.update({ projectRoot: projects[0].root });
    }
  };

  await refreshProjects();

  const tree = new DayTree({ state, runner, project: currentProject });
  const view = vscode.window.createTreeView("dayTargets", {
    treeDataProvider: tree,
    showCollapseAll: false,
    manageCheckboxStateManually: true,
  });
  context.subscriptions.push(view);
  context.subscriptions.push(
    view.onDidChangeCheckboxState(async (e) => {
      for (const [node] of e.items) {
        if (node.kind === "target") {
          await state.toggleTarget(node.name);
        }
      }
    }),
  );

  const statusBar = new StatusBar(state, runner, () => currentProject() !== undefined);
  context.subscriptions.push(statusBar);

  context.subscriptions.push(
    vscode.tasks.registerTaskProvider(DayTaskProvider.type, new DayTaskProvider(state, currentProject)),
  );

  // The user's selected targets that this host can actually build and that belong to the project.
  const selectedRunnable = (): string[] => {
    const declared = currentProject()?.targets ?? [];
    return state.selection.targets.filter((name) => {
      const target = findTarget(name);
      const okHost = target ? isBuildableHere(target) : true;
      const inProject = declared.length === 0 || declared.includes(name);
      return okHost && inProject;
    });
  };

  const requireProject = (): boolean => {
    if (currentProject()) {
      return true;
    }
    vscode.window.showWarningMessage("No Day project (day.yaml) found in this workspace.");
    return false;
  };

  const targetOf = (node?: Node): string | undefined =>
    node && node.kind === "target" ? node.name : undefined;

  const guard = async (fn: () => Promise<void>): Promise<void> => {
    try {
      await fn();
    } catch (err: any) {
      vscode.window.showErrorMessage(`Day: ${err?.message ?? err}`);
    }
  };

  const register = (id: string, fn: (...args: any[]) => any): void => {
    context.subscriptions.push(vscode.commands.registerCommand(id, fn));
  };

  register("day.run", () =>
    guard(async () => {
      if (!requireProject()) {
        return;
      }
      const targets = selectedRunnable();
      if (targets.length === 0) {
        vscode.window.showInformationMessage("Tick one or more targets in the Day view, then Run.");
        return;
      }
      await runner.runTargets(targets);
    }),
  );

  register("day.build", () =>
    guard(async () => {
      if (!requireProject()) {
        return;
      }
      const targets = selectedRunnable();
      if (targets.length === 0) {
        vscode.window.showInformationMessage("Tick one or more targets in the Day view, then Build.");
        return;
      }
      await runner.buildTargets(targets);
    }),
  );

  register("day.runTarget", (node?: Node) =>
    guard(async () => {
      const name = targetOf(node);
      if (name) {
        await runner.runTargets([name]);
      }
    }),
  );

  register("day.stop", (node?: Node) =>
    guard(async () => {
      const name = targetOf(node);
      if (name) {
        await runner.stop(name);
      }
    }),
  );

  register("day.restart", (node?: Node) =>
    guard(async () => {
      const name = targetOf(node);
      if (name) {
        await runner.restart(name);
      }
    }),
  );

  register("day.stopAll", () => guard(() => runner.stopAll()));

  register("day.toggleTarget", (node?: Node) =>
    guard(async () => {
      const name = targetOf(node);
      if (name) {
        await state.toggleTarget(name);
      }
    }),
  );

  register("day.selectMode", () =>
    guard(async () => {
      const mode = await pickMode(state.selection.profile);
      if (mode) {
        await state.update({ profile: mode });
      }
    }),
  );

  register("day.selectLocale", () =>
    guard(async () => {
      const locale = await pickLocale(currentProject(), state.selection.locale);
      if (locale !== undefined) {
        await state.update({ locale });
      }
    }),
  );

  register("day.selectScript", () =>
    guard(async () => {
      const script = await pickScript(currentProject(), state.selection.script);
      if (script !== undefined) {
        await state.update({ script });
      }
    }),
  );

  register("day.selectProject", () =>
    guard(async () => {
      await refreshProjects();
      const chosen = await pickProject(projects, state.selection.projectRoot);
      if (chosen) {
        await state.update({ projectRoot: chosen.root });
      }
    }),
  );

  register("day.doctor", () =>
    guard(async () => {
      const cli = resolveCli(currentProject()?.root);
      const term = vscode.window.createTerminal({ name: "day doctor", cwd: cli.cwd });
      term.show(true);
      term.sendText(renderCommand(cli, ["doctor"]));
    }),
  );

  register("day.refresh", () =>
    guard(async () => {
      await refreshProjects();
      tree.refresh();
    }),
  );

  // Re-scan when a day.yaml appears/changes/disappears.
  const watcher = vscode.workspace.createFileSystemWatcher("**/day.yaml");
  const rescan = () => guard(async () => {
    await refreshProjects();
    tree.refresh();
  });
  watcher.onDidCreate(rescan);
  watcher.onDidChange(rescan);
  watcher.onDidDelete(rescan);
  context.subscriptions.push(watcher);

  tree.refresh();
}

export function deactivate(): void {
  /* the Runner and status bar are disposed via context.subscriptions */
}
