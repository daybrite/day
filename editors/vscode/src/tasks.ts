// Construct a vscode.Task for a `day` build/launch. Shared by the runner (which builds a definition
// from the current selection) and the TaskProvider (which resolves a definition from tasks.json), so
// interactive runs and hand-written tasks behave identically.

import * as fs from "fs";
import * as os from "os";
import * as path from "path";
import * as vscode from "vscode";

import { buildArgs, launchArgs, renderCommand, resolveCli } from "./cli";
import { Profile } from "./config";
import { findTarget } from "./targets";

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

/** Expand a leading `~` (settings values are often written that way). */
function expandHome(p: string): string {
  return p.startsWith("~") ? path.join(os.homedir(), p.slice(1)) : p;
}

/**
 * Process environment for the task itself (NOT the launched app — that's `extraEnv`/`--env`).
 * The `ohos-arkui` target needs `OHOS_NDK_HOME` at BUILD time (day-arkui-sys compiles its C++ shim
 * with the NDK clang), and a GUI-launched VS Code usually doesn't carry it. Resolve it from the
 * `day.ohosNdkHome` setting or the common install locations, and put the SDK's sibling
 * `toolchains/` (hdc) on the task PATH. Returns {} for non-OHOS targets, deferring to the parent
 * environment.
 */
export function taskEnv(target: string): Record<string, string> {
  if (findTarget(target)?.kind !== "harmonyOs") {return {};}
  const configured = vscode.workspace.getConfiguration("day").get<string>("ohosNdkHome") ?? "";
  const candidates = [
    expandHome(configured),
    process.env.OHOS_NDK_HOME ?? "",
    path.join(os.homedir(), "ohos/ndk-extract/native"),
    path.join(os.homedir(), "ohos-sdk/native"),
  ].filter((c) => c.length > 0);
  const ndk = candidates.find((c) => fs.existsSync(path.join(c, "llvm", "bin")));
  if (!ndk) {return {};} // let the day CLI report what to install
  const env: Record<string, string> = { OHOS_NDK_HOME: ndk };
  const toolchains = path.join(path.dirname(ndk), "toolchains");
  if (fs.existsSync(toolchains)) {
    env.PATH = `${toolchains}${path.delimiter}${process.env.PATH ?? ""}`;
  }
  return env;
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

  const env = taskEnv(def.target);
  const exec = new vscode.ProcessExecution(cli.command, [...cli.baseArgs, ...args], {
    ...(cli.cwd ? { cwd: cli.cwd } : {}),
    ...(Object.keys(env).length ? { env } : {}),
  });
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
