// Status-bar affordances: a Run/Stop toggle and the current build mode, mirroring CMake Tools / Flutter.

import * as vscode from "vscode";

import { State } from "./config";
import { Runner } from "./runner";

export class StatusBar implements vscode.Disposable {
  private run: vscode.StatusBarItem;
  private mode: vscode.StatusBarItem;
  private subs: vscode.Disposable[] = [];

  constructor(
    private readonly state: State,
    private readonly runner: Runner,
    private readonly hasProject: () => boolean,
  ) {
    this.run = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 100);
    this.mode = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 99);
    this.subs.push(this.run, this.mode);
    this.subs.push(state.onDidChange(() => this.update()));
    this.subs.push(runner.onDidChange(() => this.update()));
    this.update();
  }

  private update(): void {
    if (!this.hasProject()) {
      this.run.hide();
      this.mode.hide();
      return;
    }
    const runningCount = this.runner.runningTargets().length;
    if (runningCount > 0) {
      this.run.text = `$(debug-stop) Day: stop`;
      this.run.tooltip = `Stop ${runningCount} running Day target(s)`;
      this.run.command = "day.stopAll";
    } else {
      const n = this.state.selection.targets.length;
      this.run.text = `$(rocket) Day: run`;
      this.run.tooltip = n > 0 ? `Run ${n} selected target(s)` : "Select targets in the Day view, then run";
      this.run.command = "day.run";
    }
    this.run.show();

    this.mode.text = `$(gear) ${this.state.selection.profile}`;
    this.mode.tooltip = "Day build mode";
    this.mode.command = "day.selectMode";
    this.mode.show();
  }

  dispose(): void {
    this.subs.forEach((d) => d.dispose());
  }
}
