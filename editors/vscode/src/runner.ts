// Launches Day apps through the Tasks API and tracks the running executions so individual targets can
// be stopped and restarted. Each launch is its own Task (one integrated terminal per target), so
// output is filtered per-target automatically; multi-target = one task per selected target.

import * as vscode from "vscode";

import { State } from "./config";
import { buildDayTask, DayTaskDefinition } from "./tasks";

interface Running {
  target: string;
  execution: vscode.TaskExecution;
  pid?: number;
}

export class Runner implements vscode.Disposable {
  private running = new Map<string, Running>();
  private emitter = new vscode.EventEmitter<void>();
  readonly onDidChange = this.emitter.event;
  private subs: vscode.Disposable[] = [];

  constructor(private readonly state: State) {
    this.subs.push(
      vscode.tasks.onDidStartTaskProcess((e) => {
        const def = e.execution.task.definition as DayTaskDefinition;
        if (this.isTrackedLaunch(def)) {
          const r = this.running.get(def.target);
          if (r) {
            r.pid = e.processId;
            this.emitter.fire();
          }
        }
      }),
      vscode.tasks.onDidEndTaskProcess((e) => {
        const def = e.execution.task.definition as DayTaskDefinition;
        if (this.isTrackedLaunch(def) && this.running.has(def.target)) {
          this.running.delete(def.target);
          this.emitter.fire();
        }
      }),
    );
  }

  private isTrackedLaunch(def: DayTaskDefinition): boolean {
    return def?.type === "day" && def.command === "launch" && typeof def.target === "string";
  }

  isRunning(target: string): boolean {
    return this.running.has(target);
  }

  runningTargets(): string[] {
    return [...this.running.keys()];
  }

  private definition(command: "build" | "launch", target: string): DayTaskDefinition {
    const sel = this.state.selection;
    return {
      type: "day",
      command,
      target,
      profile: sel.profile,
      locale: sel.locale || undefined,
      script: sel.script || undefined,
      project: sel.projectRoot,
    };
  }

  async runTargets(targets: string[]): Promise<void> {
    if (!this.state.selection.projectRoot) {
      throw new Error("No Day project selected.");
    }
    if (targets.length === 0) {
      throw new Error("No targets selected. Tick one or more targets in the Day view.");
    }
    for (const target of targets) {
      // Re-running a live target restarts it rather than stacking a second instance.
      if (this.running.has(target)) {
        await this.stop(target);
      }
      const exec = await vscode.tasks.executeTask(buildDayTask(this.definition("launch", target)));
      this.running.set(target, { target, execution: exec });
    }
    this.emitter.fire();
  }

  async buildTargets(targets: string[]): Promise<void> {
    if (!this.state.selection.projectRoot) {
      throw new Error("No Day project selected.");
    }
    if (targets.length === 0) {
      throw new Error("No targets selected. Tick one or more targets in the Day view.");
    }
    for (const target of targets) {
      await vscode.tasks.executeTask(buildDayTask(this.definition("build", target)));
    }
  }

  async stop(target: string): Promise<void> {
    const r = this.running.get(target);
    if (r) {
      r.execution.terminate();
      this.running.delete(target);
      this.emitter.fire();
    }
  }

  async stopAll(): Promise<void> {
    for (const target of [...this.running.keys()]) {
      await this.stop(target);
    }
  }

  async restart(target: string): Promise<void> {
    await this.stop(target);
    await this.runTargets([target]);
  }

  dispose(): void {
    this.subs.forEach((d) => d.dispose());
    this.emitter.dispose();
  }
}
