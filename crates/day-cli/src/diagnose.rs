// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Post-mortem for an app that died under a dayscript — what the runner prints when the engine
//! connection is lost (docs/agent.md, docs/break.md).
//!
//! A scripted run that ends in "engine connection lost" says only that the app is gone. The
//! evidence for WHY is on the machine and nobody looks at it: day-break's crash artifacts in the
//! app's own store, the OS crash report, the emulator's crash buffer. In CI nobody can look — the
//! runner is deleted minutes later — so this gathers what it can and prints it into the job log
//! while the machine still exists.
//!
//! Every source is best-effort and every one is announced: a diagnosis that silently found
//! nothing is indistinguishable from one that was never run, which is the state this replaces.
//! Nothing here can fail the run — the script's own verdict already did that.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime};

use anstream::eprintln;

use crate::meta::Project;
use crate::targets::{Target, TargetKind};
use crate::term::DIM;

/// How much of a crash report to print. Enough for the header and the faulting thread, which is
/// where the answer is; the whole `.ips` is hundreds of lines of loaded-image addresses.
const MAX_LINES: usize = 60;

/// Deadlines for the two kinds of tool this reaches for. Every one of them is optional to the
/// diagnosis — a post-mortem that cannot finish must not outlive the crash it is describing, and
/// a missing section says so in the output.
///
/// `DEVICE_CMD` covers `adb` and `simctl`, which wait indefinitely for a device that stopped
/// answering (the emulator wedge this whole path exists to report on). `DEBUGGER` covers the
/// host-side readers, where a large core legitimately takes a while.
const DEVICE_CMD: Duration = Duration::from_secs(30);
const DEBUGGER: Duration = Duration::from_secs(90);
/// Stack frames to print from the faulting thread — past this it is runtime plumbing.
const MAX_FRAMES: usize = 25;

/// One thing worth reading, and where it came from.
struct Finding {
    /// What produced it, for the header: `day-break report`, `macOS crash report (…ips)`.
    source: String,
    body: String,
}

/// Print everything this host can say about an app that just died. `since` bounds the search to
/// artifacts this run produced — an `.ips` from last week describes a different crash.
///
/// Returns whether it found EVIDENCE OF A CRASH, which is a different question from "did the
/// engine connection drop": a dropped connection can be a slow emulator, and the caller keeps
/// going for that; a crash artifact means this build dies on this machine, and relaunching it for
/// every remaining variant only spends minutes to fail the same way.
pub fn after_app_death(project: &Project, target: &'static Target, since: SystemTime) -> bool {
    let app_id = project.manifest.resolve(target.name).id;
    let mut findings = Vec::new();
    let mut looked: Vec<String> = Vec::new();

    day_break_findings(project, target, &app_id, since, &mut findings, &mut looked);
    os_crash_findings(project, target, since, &mut findings, &mut looked);

    if findings.is_empty() {
        crate::ops::status(
            "Diagnosis",
            &format!(
                "no crash artifact found for {} — looked at: {}",
                target.name,
                looked.join(", ")
            ),
        );
        return false;
    }
    for f in &findings {
        crate::ops::status("Diagnosis", &f.source);
        eprintln!("{DIM}{}{DIM:#}", indent(&f.body));
    }
    // The job page should name the crash without anyone opening the log.
    if crate::ops::github_actions() {
        let headline = findings
            .iter()
            .find_map(|f| headline_of(&f.body))
            .unwrap_or_else(|| findings[0].source.clone());
        println!(
            "::error title=day: {} crashed under a dayscript::{}",
            target.name,
            crate::ops::gha_escape(&headline)
        );
    }
    true
}

/// The app's own day-break store (docs/break.md). Richest when it is there: the panic message or
/// signal, the location, and the backtrace the app itself captured — the same text the user would
/// have been shown on the next launch.
///
/// Reports are FINALIZED on the next launch, so a crash seconds ago has left raw session
/// artifacts (`session-…kv`, `sig-…kv`, `pending-…kv`) rather than `reports/report-*.json`. Both
/// are plain text; whichever exists is printed, newest first.
fn day_break_findings(
    project: &Project,
    target: &'static Target,
    app_id: &str,
    since: SystemTime,
    out: &mut Vec<Finding>,
    looked: &mut Vec<String>,
) {
    let Some(dir) = break_store_dir(project, target, app_id) else {
        return;
    };
    looked.push(format!("day-break store ({})", dir.display()));
    if !dir.is_dir() {
        return;
    }
    // Both shapes, ranked together by mtime and filtered to THIS run. Reports are finalized on
    // the app's NEXT launch, so a crash seconds ago has left raw artifacts while `reports/` still
    // holds the PREVIOUS crash — reading reports/ first would confidently describe the wrong
    // death. `since` is what makes that impossible.
    let mut files = newest_files(&dir.join("reports"), 4);
    files.extend(newest_files(&dir, 6));
    let fresh: Vec<PathBuf> = files
        .into_iter()
        .filter(|p| modified_since(p, since))
        // A `sig-*.sig` marker is zero bytes: the evidence is in the `.kv` beside it.
        .filter(|p| std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false))
        .collect();
    if fresh.is_empty() {
        return;
    }
    for path in fresh {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        // A finalized report says when its session STARTED and which backend it ran — far better
        // than the file's mtime, because a launch finalizes every stale session it finds, which
        // stamps another target's old crash with today's time. Raw artifacts have no such field
        // and keep the mtime test above.
        if !describes_this_run(&text, target, since) {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        out.push(Finding {
            source: format!("day-break {name}"),
            body: summarize_break(&text).unwrap_or_else(|| head(&text, MAX_LINES)),
        });
        if out.len() >= 2 {
            break;
        }
    }
}

/// Whether an artifact describes the session this run just launched: same backend, started no
/// earlier than the launch. Both matter — the store is keyed by APP id, so every target's crashes
/// and every leftover instance's share one directory, and a launch FINALIZES every stale session
/// it finds, which stamps another target's old crash with today's mtime.
///
/// Reads either shape: a finalized report's JSON, or the `k=v` lines of a raw session artifact.
fn describes_this_run(text: &str, target: &'static Target, since: SystemTime) -> bool {
    let (backend, started) = match serde_json::from_str::<serde_json::Value>(text.trim()) {
        Ok(v) if v.get("kind").is_some() => (
            v.pointer("/day/backend")
                .and_then(|b| b.as_str())
                .map(str::to_string),
            v.pointer("/session/started_at_ms").and_then(|s| s.as_u64()),
        ),
        _ => {
            let field = |key: &str| {
                text.lines()
                    .find_map(|l| l.trim().strip_prefix(&format!("{key}=")))
                    .map(str::to_string)
            };
            (
                field("backend"),
                field("started_at_ms").and_then(|v| v.parse().ok()),
            )
        }
    };
    if let Some(b) = backend
        && b != target.name
    {
        return false;
    }
    let Some(started) = started else {
        return true; // nothing to judge by: keep it rather than hide a real crash
    };
    let since_ms = since
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    // A second of slack: the session starts inside the app, either side of the parent's stamp.
    started + 1000 >= since_ms
}

/// Where this target keeps its day-break store. Desktop is the host's own path; the iOS simulator
/// keeps it inside the app's data container, which `simctl` can resolve. Android and OpenHarmony
/// hold theirs in a device sandbox this does not reach into — their OS crash buffer is the source
/// [`os_crash_findings`] uses instead.
fn break_store_dir(project: &Project, target: &'static Target, app_id: &str) -> Option<PathBuf> {
    match target.kind {
        TargetKind::Desktop => {
            // The same layout day-break computes (day-break/src/store.rs `store_dir`), for the
            // host it is running on. Kept as a copy rather than a dependency: day-break pulls in
            // day-pieces for its consent surface, which has no business inside the CLI.
            let home = std::env::var_os("HOME").map(PathBuf::from)?;
            let slug = slug(app_id);
            if cfg!(target_os = "macos") {
                Some(
                    home.join("Library/Application Support")
                        .join(slug)
                        .join("day-break"),
                )
            } else {
                Some(home.join(format!(".{slug}")).join("day-break"))
            }
        }
        TargetKind::IosSim => {
            let out = crate::ops::output_within(
                Command::new("xcrun").args([
                    "simctl",
                    "get_app_container",
                    "booted",
                    app_id,
                    "data",
                ]),
                DEVICE_CMD,
            )?;
            if !out.status.success() {
                return None;
            }
            let container = String::from_utf8_lossy(&out.stdout).trim().to_string();
            (!container.is_empty()).then(|| {
                Path::new(&container)
                    .join("Library/Application Support")
                    .join(slug(app_id))
                    .join("day-break")
            })
        }
        _ => {
            let _ = project;
            None
        }
    }
}

/// The operating system's own account of the death, for the crashes day-break cannot catch (a
/// kill, a fault inside the toolkit, an app that never armed it).
fn os_crash_findings(
    project: &Project,
    target: &'static Target,
    since: SystemTime,
    out: &mut Vec<Finding>,
    looked: &mut Vec<String>,
) {
    match target.kind {
        // macOS and the iOS simulator both write `.ips` reports into the HOST's DiagnosticReports
        // directory, named after the process.
        TargetKind::Desktop | TargetKind::IosSim if cfg!(target_os = "macos") => {
            let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
                return;
            };
            let dir = home.join("Library/Logs/DiagnosticReports");
            looked.push(format!("macOS crash reports ({})", dir.display()));
            let stem = process_stem(project, target);
            // ReportCrash writes the `.ips` a second or two AFTER the process dies, which is
            // after the engine loss that brought us here. Looking once finds the PREVIOUS run's
            // report or nothing at all, so wait for this run's — briefly, and only on a run that
            // has already failed.
            // On a desktop launch the app IS this process's child, so its pid picks the right
            // report out of a directory where macos-gtk and macos-qt builds file under the same
            // process name. Without one (a simulator, or a launch that recorded nothing) the
            // name-and-freshness match stands, which is what it always was.
            let pid = matches!(target.kind, TargetKind::Desktop)
                .then(crate::signals::last_child)
                .flatten();
            if !wait_for_fresh_ips(&dir, &stem, since, pid) {
                crate::ops::status(
                    "Diagnosis",
                    "no macOS crash report yet — ReportCrash can take a minute; \
                     ~/Library/Logs/DiagnosticReports will have it",
                );
            }
            for path in newest_files(&dir, 8) {
                let name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let fresh = std::fs::metadata(&path)
                    .and_then(|m| m.modified())
                    .map(|m| m >= since)
                    .unwrap_or(false);
                if !fresh || !name.to_lowercase().starts_with(&stem.to_lowercase()) {
                    continue;
                }
                let Ok(text) = std::fs::read_to_string(&path) else {
                    continue;
                };
                if let Some(pid) = pid
                    && ips_pid(&text).is_some_and(|reported| reported != pid)
                {
                    continue; // another instance of this app died in the same window
                }
                out.push(Finding {
                    source: format!("macOS crash report ({name})"),
                    body: summarize_ips(&text).unwrap_or_else(|| head(&text, MAX_LINES)),
                });
                break; // the newest match is this run's
            }
        }
        // Linux has no per-crash report file: what exists is a core, if the kernel was asked for
        // one, and a debugger to read it with. day-break's artifact already names the signal and
        // the faulting address; this is the half that gets FRAMES.
        TargetKind::Desktop if cfg!(target_os = "linux") => {
            looked.push("systemd-coredump (coredumpctl)".into());
            let stem = process_stem(project, target);
            if let Some(o) = crate::ops::output_within(
                Command::new("coredumpctl").args(["info", "--no-pager", &stem]),
                DEBUGGER,
            ) && o.status.success()
            {
                let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !text.is_empty() {
                    out.push(Finding {
                        source: format!("coredumpctl info {stem}"),
                        body: head(&text, MAX_LINES),
                    });
                    return;
                }
            }
            // No systemd-coredump (the GitHub runners use apport, and containers often disable
            // both). Say what to turn on rather than leave a Linux crash with no frames at all —
            // the kernel wrote "core dumped", so the core exists somewhere the pattern decides.
            looked.push("a core file beside the app".into());
            let core = newest_files(&project.root, 12).into_iter().find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n == "core" || n.starts_with("core."))
                    && modified_since(p, since)
            });
            match (core, which("gdb")) {
                (Some(core), true) => {
                    if let Some(o) = crate::ops::output_within(
                        Command::new("gdb")
                            .args(["-batch", "-ex", "thread apply all bt"])
                            .arg(process_path(project, target).unwrap_or_default())
                            .arg(&core),
                        DEBUGGER,
                    ) {
                        out.push(Finding {
                            source: format!("gdb backtrace ({})", core.display()),
                            body: head(&String::from_utf8_lossy(&o.stdout), MAX_LINES),
                        });
                    }
                }
                (Some(core), false) => out.push(Finding {
                    source: format!("core file ({})", core.display()),
                    body: "no gdb on this host to read it — `apt install gdb`, then                            `gdb -batch -ex bt <exe> <core>`"
                        .into(),
                }),
                (None, _) => crate::ops::status(
                    "Diagnosis",
                    "no core file — a Linux crash carries no frames without one. In CI:                      `ulimit -c unlimited` and `sudo sysctl -w kernel.core_pattern=core.%p`                      before the launch, then re-run.",
                ),
            }
        }
        // Windows writes nothing on its own: no core file, no crash report on disk. What it DOES
        // record is a WER event in the Application log naming the faulting module and the
        // exception code, which is most of the diagnosis — and a full minidump if (and only if)
        // LocalDumps was switched on beforehand. Both are read here, and the advice for enabling
        // dumps is printed when there is none, exactly as the Linux arm does for core files.
        TargetKind::Desktop if cfg!(windows) => {
            let stem = process_stem(project, target);
            let pid = crate::signals::last_child();

            // 1. The minidump, if this machine opted in.
            let dumps = std::env::var_os("LOCALAPPDATA")
                .map(PathBuf::from)
                .map(|p| p.join("CrashDumps"));
            let mut have_dump = false;
            if let Some(dir) = dumps.as_ref() {
                looked.push(format!("WER local dumps ({})", dir.display()));
                for path in newest_files(dir, 8) {
                    let name = path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let mine = name.starts_with(&stem)
                        || pid.is_some_and(|p| name.contains(&format!(".{p}.")));
                    if mine && modified_since(&path, since) {
                        have_dump = true;
                        out.push(Finding {
                            source: format!("minidump ({})", path.display()),
                            body: format!(
                                "open in WinDbg/Visual Studio for frames:\n  \
                                 windbg -z \"{}\" -c \"!analyze -v; q\"",
                                path.display()
                            ),
                        });
                    }
                }
            }

            // 2. The WER event: faulting module + exception code, which is what a Windows crash
            //    reports without a debugger attached. Scoped to this run by start time so a stale
            //    record from an earlier crash cannot be read as this one's.
            looked.push("Windows Application event log (Application Error)".into());
            let secs = SystemTime::now()
                .duration_since(since)
                .map(|d| d.as_secs() + 5)
                .unwrap_or(120);
            let ps = format!(
                "$s=(Get-Date).AddSeconds(-{secs}); \
                 Get-WinEvent -FilterHashtable @{{LogName='Application'; \
                 ProviderName='Application Error','Windows Error Reporting','.NET Runtime'; \
                 StartTime=$s}} -MaxEvents 5 -ErrorAction SilentlyContinue | \
                 ForEach-Object {{ $_.TimeCreated.ToString('s') + '  ' + $_.ProviderName; \
                 $_.Message }}"
            );
            if let Some(o) = crate::ops::output_within(
                Command::new("powershell").args(["-NoProfile", "-NonInteractive", "-Command", &ps]),
                DEBUGGER,
            ) {
                let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
                // Only OUR process: the runner's other apps file here too.
                let mine = text.contains(&stem)
                    || pid.is_some_and(|p| {
                        text.contains(&format!("{p:x}")) || text.contains(&p.to_string())
                    });
                if !text.is_empty() && mine {
                    out.push(Finding {
                        source: "Windows Error Reporting (Application event log)".into(),
                        body: head(&text, MAX_LINES),
                    });
                } else if !have_dump {
                    crate::ops::status(
                        "Diagnosis",
                        "no WER record for this process yet — the event log can lag a few \
                         seconds behind the exit",
                    );
                }
            }

            if !have_dump {
                crate::ops::status(
                    "Diagnosis",
                    "no minidump — Windows writes one only when LocalDumps is enabled. In CI, \
                     before the launch:\r\n    reg add \
                     \"HKCU\\Software\\Microsoft\\Windows\\Windows Error Reporting\\LocalDumps\" \
                     /v DumpType /t REG_DWORD /d 2 /f\r\n  then re-run; the .dmp lands in \
                     %LOCALAPPDATA%\\CrashDumps and carries full frames.",
                );
            }
        }
        TargetKind::Android => {
            looked.push("adb logcat -b crash".into());
            let out_cmd = crate::ops::output_within(
                Command::new("adb").args(["logcat", "-b", "crash", "-d", "-t", "200"]),
                DEVICE_CMD,
            );
            if let Some(o) = out_cmd
                && o.status.success()
            {
                let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
                if !text.is_empty() {
                    out.push(Finding {
                        source: "android crash buffer (adb logcat -b crash)".into(),
                        body: head(&text, MAX_LINES),
                    });
                }
            }
        }
        _ => {}
    }
}

/// The pid an `.ips` reports, for telling two instances of one app apart.
fn ips_pid(text: &str) -> Option<u32> {
    let (_header, body) = text.split_once('\n')?;
    serde_json::from_str::<serde_json::Value>(body)
        .ok()?
        .get("pid")?
        .as_u64()
        .map(|p| p as u32)
}

/// Whether `path` was written by the run that started at `since`.
fn modified_since(path: &Path, since: SystemTime) -> bool {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .map(|m| m >= since)
        .unwrap_or(false)
}

/// Block until this run's crash report appears, or the budget runs out; `true` when one landed.
///
/// ReportCrash writes the `.ips` well AFTER the process dies — measured at 20–40 s on this
/// machine — which is always after the engine loss that brought us here. Looking once finds the
/// previous run's report or nothing at all. Waiting costs only a run that has already failed, and
/// a run that fails without saying why costs someone an afternoon.
fn wait_for_fresh_ips(dir: &Path, stem: &str, since: SystemTime, pid: Option<u32>) -> bool {
    const BUDGET: std::time::Duration = std::time::Duration::from_secs(30);
    let start = std::time::Instant::now();
    let stem = stem.to_lowercase();
    while start.elapsed() < BUDGET {
        let found = newest_files(dir, 8).into_iter().any(|p| {
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            if !name.starts_with(&stem) || !modified_since(&p, since) {
                return false;
            }
            // With a pid in hand, wait for THAT process's report rather than any fresh one — a
            // sibling instance's crash would otherwise end the wait early.
            match (
                pid,
                std::fs::read_to_string(&p)
                    .ok()
                    .as_deref()
                    .and_then(ips_pid),
            ) {
                (Some(want), Some(got)) => want == got,
                _ => true,
            }
        });
        if found {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    false
}

/// Whether `bin` resolves on PATH.
fn which(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(bin).is_file()))
}

/// The built executable, for a debugger that needs symbols to go with a core.
fn process_path(project: &Project, target: &'static Target) -> Option<PathBuf> {
    let stamp = project
        .root
        .join("build/day/artifacts")
        .join(format!("{}-debug.path", target.name));
    let path = std::fs::read_to_string(stamp).ok()?.trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

/// The process name a crash report is filed under: the built artifact's own file name, with a
/// `.app` bundle reduced to the executable inside it.
fn process_stem(project: &Project, target: &'static Target) -> String {
    let name = project.manifest.app.name.clone();
    let _ = target;
    name
}

/// Render an `.ips` as the four things a reader wants: what died, why, and the faulting thread's
/// stack. The raw file is a JSON header line followed by a JSON body of a few hundred lines, most
/// of it loaded-image addresses — printing its head shows `userID` and `deployVersion` and stops
/// well before the frames, which is no use to anyone.
///
/// Frames carry a symbol plus an image index; the image list turns that into a name, so a line
/// reads `showcase  day_core::pump::run + 42`. `None` when the file is not the shape this expects
/// — the caller falls back to printing the head, which is still better than nothing.
fn summarize_ips(text: &str) -> Option<String> {
    let (_header, body) = text.split_once('\n')?;
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    let s = |key: &str| -> String {
        v.get(key)
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .to_string()
    };
    let mut out = vec![format!(
        "process {} (pid {}) — {}",
        s("procName"),
        v.get("pid").and_then(|p| p.as_i64()).unwrap_or(0),
        s("captureTime"),
    )];
    if let Some(e) = v.get("exception") {
        out.push(format!(
            "exception {}{}",
            e.get("type").and_then(|t| t.as_str()).unwrap_or("?"),
            e.get("signal")
                .and_then(|t| t.as_str())
                .map(|sig| format!(" ({sig})"))
                .unwrap_or_default(),
        ));
    }
    if let Some(ind) = v.pointer("/termination/indicator").and_then(|x| x.as_str()) {
        out.push(format!("termination {ind}"));
    }
    // The faulting thread's frames, symbolicated as far as the report itself goes.
    let faulting = v
        .get("faultingThread")
        .and_then(|f| f.as_u64())
        .unwrap_or(0) as usize;
    let images: Vec<&str> = v
        .get("usedImages")
        .and_then(|i| i.as_array())
        .map(|a| {
            a.iter()
                .map(|i| i.get("name").and_then(|n| n.as_str()).unwrap_or("?"))
                .collect()
        })
        .unwrap_or_default();
    let frames = v
        .get("threads")
        .and_then(|t| t.as_array())
        .and_then(|t| t.get(faulting))
        .and_then(|t| t.get("frames"))
        .and_then(|f| f.as_array());
    if let Some(frames) = frames {
        out.push(format!("faulting thread {faulting}:"));
        for f in frames.iter().take(MAX_FRAMES) {
            let image = f
                .get("imageIndex")
                .and_then(|i| i.as_u64())
                .and_then(|i| images.get(i as usize).copied())
                .unwrap_or("?");
            let offset = f.get("imageOffset").and_then(|o| o.as_u64()).unwrap_or(0);
            match f.get("symbol").and_then(|s| s.as_str()) {
                Some(sym) => {
                    let at = f
                        .get("symbolLocation")
                        .and_then(|l| l.as_u64())
                        .unwrap_or(0);
                    out.push(format!("  {image}  {sym} + {at}"));
                }
                None => out.push(format!("  {image}  +0x{offset:x}")),
            }
        }
        if frames.len() > MAX_FRAMES {
            out.push(format!("  … {} more frame(s)", frames.len() - MAX_FRAMES));
        }
    }
    Some(out.join("\n"))
}

/// Render a day-break report (docs/break.md) as its answer rather than its JSON: what kind of
/// death, the message and location a panic carries, the signal a fault carries, and the backtrace
/// the app captured for itself. `None` when the text is not a finalized report — the raw session
/// artifacts are `k=v` lines, which read fine as they are.
fn summarize_break(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text.trim()).ok()?;
    let kind = v.get("kind")?.as_str().unwrap_or("?");
    let contained = if v.get("fatal").and_then(|f| f.as_bool()) == Some(false) {
        " (contained)"
    } else {
        ""
    };
    let mut out = vec![format!(
        "{kind}{contained} — {} {} on {}",
        v.pointer("/app/id").and_then(|x| x.as_str()).unwrap_or("?"),
        v.pointer("/app/version")
            .and_then(|x| x.as_str())
            .unwrap_or("?"),
        v.pointer("/day/backend")
            .and_then(|x| x.as_str())
            .unwrap_or("?"),
    )];
    for (label, ptr) in [("message", "/message"), ("location", "/location")] {
        if let Some(t) = v.pointer(ptr).and_then(|x| x.as_str())
            && !t.is_empty()
        {
            out.push(format!("{label}: {t}"));
        }
    }
    if let Some(name) = v.pointer("/signal/name").and_then(|n| n.as_str()) {
        out.push(format!(
            "signal: {name} (code {})",
            v.pointer("/signal/code")
                .and_then(|c| c.as_i64())
                .unwrap_or(0)
        ));
    }
    if let Some(uptime) = v.pointer("/session/uptime_ms").and_then(|u| u.as_u64()) {
        out.push(format!("died {uptime} ms after launch"));
    }
    match v.get("backtrace_text").and_then(|b| b.as_str()) {
        Some(bt) if !bt.trim().is_empty() => {
            out.push("backtrace:".into());
            out.push(head(bt, MAX_LINES));
        }
        // A signal death has no Rust backtrace to give: the handler runs on a broken stack and
        // day-break deliberately does not walk it. The OS report carries the frames instead.
        _ => out
            .push("backtrace: none recorded (the OS crash report below carries the frames)".into()),
    }
    Some(out.join("\n"))
}

/// The `<slug>` day-break namespaces its store under (day-break/src/store.rs `slug`).
fn slug(app_id: &str) -> String {
    let s: String = app_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if s.is_empty() { "day-break".into() } else { s }
}

/// The `n` most recently modified regular files directly inside `dir`, newest first.
fn newest_files(dir: &Path, n: usize) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<(SystemTime, PathBuf)> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter_map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok()?;
            Some((t, p))
        })
        .collect();
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files.into_iter().take(n).map(|(_, p)| p).collect()
}

/// The first `max` lines, with a note when there was more.
fn head(text: &str, max: usize) -> String {
    let total = text.lines().count();
    let mut s: String = text.lines().take(max).collect::<Vec<_>>().join("\n");
    if total > max {
        s.push_str(&format!("\n… {} more line(s)", total - max));
    }
    s
}

/// The one line worth putting in a CI annotation: a day-break `message`/`kind`, or an `.ips`
/// termination reason. Falls back to the first non-empty line.
fn headline_of(body: &str) -> Option<String> {
    let keyed = body.lines().find_map(|l| {
        let line = l.trim().trim_end_matches(',');
        // The quote is stripped for MATCHING a JSON key and kept in what is returned — the
        // annotation should read like the report it came from.
        let probe = line.trim_start_matches('"').to_lowercase();
        ["message", "kind", "exception", "termination"]
            .iter()
            .any(|k| probe.starts_with(k))
            .then(|| line.to_string())
    });
    let signal = body.lines().find_map(|l| {
        let n: i32 = l.trim().strip_prefix("sig=")?.trim().parse().ok()?;
        Some(format!("fatal signal {}", signal_name(n)))
    });
    keyed.or(signal).or_else(|| {
        body.lines()
            .find(|l| !l.trim().is_empty())
            .map(|l| l.trim().to_string())
    })
}

/// The POSIX name for a signal number, for the raw `sig=<n>` artifacts (day-break spells these
/// out itself in a finalized report; this is the same table for the pre-finalize shape).
fn signal_name(signo: i32) -> String {
    match signo {
        4 => "SIGILL".into(),
        6 => "SIGABRT".into(),
        7 => "SIGBUS".into(),
        8 => "SIGFPE".into(),
        10 => "SIGBUS".into(),
        11 => "SIGSEGV".into(),
        _ => format!("signal {signo}"),
    }
}

/// Indent a block so it reads as evidence under its header rather than as day's own output.
fn indent(body: &str) -> String {
    body.lines()
        .map(|l| format!("    {l}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The store path day-break itself computes, for the two host layouts.
    #[test]
    fn the_slug_matches_day_breaks_own_rule() {
        assert_eq!(slug("dev.daybrite.showcase"), "dev.daybrite.showcase");
        assert_eq!(slug("my app/id"), "my-app-id");
        assert_eq!(slug(""), "day-break");
    }

    /// A long report is cut with the count of what was dropped, so nobody reads 40 lines and
    /// assumes that was all of it.
    #[test]
    fn head_says_what_it_dropped() {
        let text = (1..=100)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let cut = head(&text, 10);
        assert!(cut.starts_with("1\n2\n"));
        assert!(cut.ends_with("… 90 more line(s)"));
        assert_eq!(head("one\ntwo", 10), "one\ntwo");
    }

    /// The annotation headline prefers the line that names the crash.
    #[test]
    fn the_headline_names_the_crash() {
        let report = "{\n  \"app_id\": \"dev.x\",\n  \"message\": \"intentional abort\",\n}";
        assert_eq!(
            headline_of(report).unwrap(),
            "\"message\": \"intentional abort\""
        );
        // Nothing keyed: the first non-empty line still beats an empty annotation.
        assert_eq!(headline_of("\n\n  a fault  \nb").unwrap(), "a fault");
    }
}
