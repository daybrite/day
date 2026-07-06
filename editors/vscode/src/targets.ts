// The Day target catalog — mirrors crates/day-cli/src/targets.rs. Each `<os>-<toolkit>` target
// declares the host OS that can build it, so the UI can dim/disable targets this machine can't run.

export type TargetKind = "desktop" | "iosSim" | "android" | "harmonyOs";
export type HostOs = "macos" | "linux" | "windows" | "any";

export interface Target {
  name: string;
  toolkit: string;
  kind: TargetKind;
  host: HostOs;
}

export const TARGETS: Target[] = [
  { name: "macos-appkit", toolkit: "appkit", kind: "desktop", host: "macos" },
  { name: "macos-gtk", toolkit: "gtk", kind: "desktop", host: "macos" },
  { name: "macos-qt", toolkit: "qt", kind: "desktop", host: "macos" },
  { name: "linux-gtk", toolkit: "gtk", kind: "desktop", host: "linux" },
  { name: "windows-winui", toolkit: "winui", kind: "desktop", host: "windows" },
  { name: "windows-qt", toolkit: "qt", kind: "desktop", host: "windows" },
  { name: "windows-gtk", toolkit: "gtk", kind: "desktop", host: "windows" },
  { name: "linux-qt", toolkit: "qt", kind: "desktop", host: "linux" },
  { name: "ios-uikit", toolkit: "uikit", kind: "iosSim", host: "macos" },
  { name: "android-widget", toolkit: "widget", kind: "android", host: "any" },
  { name: "harmonyos-arkui", toolkit: "arkui", kind: "harmonyOs", host: "any" },
];

export function findTarget(name: string): Target | undefined {
  return TARGETS.find((t) => t.name === name);
}

export function hostOs(): HostOs | "other" {
  switch (process.platform) {
    case "darwin":
      return "macos";
    case "linux":
      return "linux";
    case "win32":
      return "windows";
    default:
      return "other";
  }
}

/** Whether the current host can build/run this target. */
export function isBuildableHere(t: Target): boolean {
  return t.host === "any" || t.host === hostOs();
}

/** A short, human label for a target's kind (shown as the tree item description). */
export function kindLabel(t: Target): string {
  switch (t.kind) {
    case "desktop":
      return "desktop";
    case "iosSim":
      return "iOS simulator";
    case "android":
      return "Android";
    case "harmonyOs":
      return "HarmonyOS";
  }
}
