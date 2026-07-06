// The Day sidebar tree: the current project, a Configuration section (build mode / locale / dayscript),
// and a Targets section (selectable via checkboxes, with running badges + inline run/stop/restart).

import * as path from "path";
import * as vscode from "vscode";

import { State } from "./config";
import { DayProject } from "./project";
import { Runner } from "./runner";
import { findTarget, isBuildableHere, kindLabel, TARGETS } from "./targets";

export type Node =
  | { kind: "project" }
  | { kind: "section"; id: "config" | "targets"; label: string }
  | { kind: "config"; which: "mode" | "locale" | "script" }
  | { kind: "target"; name: string };

export interface TreeDeps {
  state: State;
  runner: Runner;
  project: () => DayProject | undefined;
}

export class DayTree implements vscode.TreeDataProvider<Node> {
  private emitter = new vscode.EventEmitter<Node | undefined>();
  readonly onDidChangeTreeData = this.emitter.event;

  constructor(private readonly deps: TreeDeps) {
    deps.state.onDidChange(() => this.refresh());
    deps.runner.onDidChange(() => this.refresh());
  }

  refresh(): void {
    this.emitter.fire(undefined);
  }

  private targetNames(): string[] {
    const project = this.deps.project();
    const declared = project?.targets ?? [];
    return declared.length > 0 ? declared : TARGETS.map((t) => t.name);
  }

  getChildren(element?: Node): Node[] {
    if (!element) {
      if (!this.deps.project()) {
        return []; // triggers the viewsWelcome content
      }
      return [
        { kind: "project" },
        { kind: "section", id: "config", label: "Configuration" },
        { kind: "section", id: "targets", label: "Targets" },
      ];
    }
    if (element.kind === "section" && element.id === "config") {
      return [
        { kind: "config", which: "mode" },
        { kind: "config", which: "locale" },
        { kind: "config", which: "script" },
      ];
    }
    if (element.kind === "section" && element.id === "targets") {
      return this.targetNames().map((name) => ({ kind: "target", name }));
    }
    return [];
  }

  getTreeItem(node: Node): vscode.TreeItem {
    switch (node.kind) {
      case "project":
        return this.projectItem();
      case "section":
        return this.sectionItem(node.label);
      case "config":
        return this.configItem(node.which);
      case "target":
        return this.targetItem(node.name);
    }
  }

  private projectItem(): vscode.TreeItem {
    const p = this.deps.project()!;
    const item = new vscode.TreeItem(p.name, vscode.TreeItemCollapsibleState.None);
    item.description = p.id;
    item.tooltip = `${p.title ?? p.name}\n${p.root}`;
    item.iconPath = new vscode.ThemeIcon("package");
    item.contextValue = "dayProject";
    item.command = { command: "day.selectProject", title: "Select Project" };
    return item;
  }

  private sectionItem(label: string): vscode.TreeItem {
    const item = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.Expanded);
    item.contextValue = "daySection";
    return item;
  }

  private configItem(which: "mode" | "locale" | "script"): vscode.TreeItem {
    const sel = this.deps.state.selection;
    let label: string;
    let value: string;
    let icon: string;
    let command: string;
    switch (which) {
      case "mode":
        label = "Build mode";
        value = sel.profile;
        icon = "gear";
        command = "day.selectMode";
        break;
      case "locale":
        label = "Locale";
        value = sel.locale.length > 0 ? sel.locale : "(default)";
        icon = "globe";
        command = "day.selectLocale";
        break;
      case "script":
        label = "Dayscript";
        value = sel.script.length > 0 ? path.basename(sel.script) : "(none)";
        icon = "play-circle";
        command = "day.selectScript";
        break;
    }
    const item = new vscode.TreeItem(label, vscode.TreeItemCollapsibleState.None);
    item.description = value;
    item.iconPath = new vscode.ThemeIcon(icon);
    item.contextValue = "dayConfig";
    item.command = { command, title: label };
    return item;
  }

  private targetItem(name: string): vscode.TreeItem {
    const target = findTarget(name);
    const running = this.deps.runner.isRunning(name);
    const buildable = target ? isBuildableHere(target) : true;
    const selected = this.deps.state.selection.targets.includes(name);

    const item = new vscode.TreeItem(name, vscode.TreeItemCollapsibleState.None);

    const parts: string[] = [];
    if (target) {
      parts.push(kindLabel(target));
    }
    if (running) {
      parts.push("running");
    } else if (!buildable) {
      parts.push(`needs a ${target?.host} host`);
    }
    item.description = parts.join(" · ");

    if (running) {
      item.iconPath = new vscode.ThemeIcon("circle-filled", new vscode.ThemeColor("charts.green"));
      item.contextValue = "dayTargetRunning";
    } else if (!buildable) {
      item.iconPath = new vscode.ThemeIcon("circle-slash", new vscode.ThemeColor("disabledForeground"));
      item.contextValue = "dayTargetDisabled";
    } else {
      const kindIcon = target && target.kind === "desktop" ? "device-desktop" : "device-mobile";
      item.iconPath = new vscode.ThemeIcon(kindIcon);
      item.contextValue = "dayTarget";
    }

    // Only buildable targets get a selection checkbox + a toggle-on-click.
    if (buildable) {
      item.checkboxState = selected
        ? vscode.TreeItemCheckboxState.Checked
        : vscode.TreeItemCheckboxState.Unchecked;
      item.command = {
        command: "day.toggleTarget",
        title: "Toggle Target",
        arguments: [{ kind: "target", name } as Node],
      };
    }
    item.tooltip = target
      ? `${name} — ${kindLabel(target)}${buildable ? "" : ` (requires a ${target.host} host)`}`
      : name;
    return item;
  }
}
