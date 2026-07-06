// The current build/run selection (project, targets, mode, locale, script), persisted per-workspace
// in the extension's Memento, plus accessors for the `day.*` settings. A change event lets the tree
// and status bar refresh when the selection is edited.

import * as vscode from "vscode";

export type Profile = "debug" | "release";

export interface Selection {
  projectRoot?: string;
  targets: string[];
  profile: Profile;
  /** BCP-47 locale; "" = the app/system default. */
  locale: string;
  /** Dayscript path; "" = none. */
  script: string;
}

const KEY = "day.selection";

export class State {
  private emitter = new vscode.EventEmitter<void>();
  readonly onDidChange = this.emitter.event;

  constructor(private readonly mem: vscode.Memento) {}

  get selection(): Selection {
    const stored = this.mem.get<Partial<Selection>>(KEY) ?? {};
    const cfg = vscode.workspace.getConfiguration("day");
    return {
      projectRoot: stored.projectRoot,
      targets: stored.targets ?? [],
      profile: stored.profile ?? (cfg.get<Profile>("defaultProfile") ?? "debug"),
      locale: stored.locale ?? (cfg.get<string>("defaultLocale") ?? ""),
      script: stored.script ?? "",
    };
  }

  async update(patch: Partial<Selection>): Promise<void> {
    const next = { ...this.selection, ...patch };
    await this.mem.update(KEY, next);
    this.emitter.fire();
  }

  toggleTarget(name: string): Promise<void> {
    const cur = this.selection.targets;
    const next = cur.includes(name) ? cur.filter((t) => t !== name) : [...cur, name];
    return this.update({ targets: next });
  }

  extraEnv(): Record<string, string> {
    return vscode.workspace.getConfiguration("day").get<Record<string, string>>("extraEnv") ?? {};
  }
}
