// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! Incremental per-invocation reports. A socket reader saves telemetry even while the
//! runner sleeps or captures a device screenshot; reports survive an app/runner crash.
use crate::targets::{Target, TargetKind};
use day_script_proto::{MemorySample, Reply};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

static RUN_NUMBER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
fn sequence() -> u64 {
    RUN_NUMBER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

type Record = Arc<Mutex<Live>>;
#[derive(Clone, Default)]
pub(crate) struct Active(Arc<Mutex<Option<Record>>>);

struct Live {
    path: PathBuf,
    value: Value,
    start: Option<Instant>,
    sum: f64,
    warned: bool,
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Live {
    fn save(&mut self) {
        if let Some(start) = self.start {
            self.value["duration_ms"] = json!(start.elapsed().as_millis() as u64);
        }
        // Every checkpoint is a complete document; a killed writer leaves the previous one.
        let tmp = self.path.with_extension("tmp");
        let result = (|| -> std::io::Result<()> {
            let mut file = std::fs::File::create(&tmp)?;
            serde_json::to_writer(&mut file, &self.value)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            std::fs::rename(&tmp, &self.path)
        })();
        if let Err(error) = result
            && !self.warned
        {
            self.warned = true;
            eprintln!("warning: dayscript report {}: {error}", self.path.display());
        }
    }

    fn sample(&mut self, reading: MemorySample) {
        let elapsed = self
            .start
            .map(|s| s.elapsed().as_millis() as u64)
            .unwrap_or(0);
        let memory = &mut self.value["memory"];
        memory["metric"] = json!(reading.metric);
        memory["samples"]
            .as_array_mut()
            .unwrap()
            .push(json!({"elapsed_ms":elapsed,"bytes":reading.bytes}));
        memory["end_bytes"] = json!(reading.bytes);
        memory["end_elapsed_ms"] = json!(elapsed);
        if let Some(bytes) = reading.bytes {
            let count = memory["sample_count"].as_u64().unwrap() + 1;
            memory["sample_count"] = json!(count);
            memory["min_bytes"] = json!(memory["min_bytes"].as_u64().unwrap_or(bytes).min(bytes));
            memory["max_bytes"] = json!(memory["max_bytes"].as_u64().unwrap_or(bytes).max(bytes));
            self.sum += bytes as f64;
            memory["avg_bytes"] = json!(self.sum / count as f64);
        } else {
            memory["unavailable_count"] = json!(memory["unavailable_count"].as_u64().unwrap() + 1);
        }
        self.save();
    }

    fn finish(&mut self, error: Option<&str>) {
        if self.value["status"] != "running" {
            return;
        }
        let steps = &mut self.value["steps"];
        let accounted = ["passed", "skipped", "failed"]
            .iter()
            .map(|k| steps[*k].as_u64().unwrap())
            .sum::<u64>();
        steps["aborted"] = json!(steps["planned"].as_u64().unwrap().saturating_sub(accounted));
        self.value["status"] = json!(if error.is_some() {
            "interrupted"
        } else if steps["failed"].as_u64().unwrap() > 0 {
            "failed"
        } else {
            "passed"
        });
        self.value["error"] = json!(error);
        self.value["finished_at_unix_ms"] = json!(unix_ms());
        self.save();
    }
}

impl Active {
    fn record(&self) -> Option<Record> {
        self.0.lock().unwrap().clone()
    }
    fn sample(&self, sample: MemorySample) {
        if let Some(record) = self.record() {
            record.lock().unwrap().sample(sample);
        }
    }
}

pub(crate) struct Reports {
    records: Vec<Record>,
    pub active: Active,
    pub interval_ms: u64,
    summarize: bool,
}

impl Reports {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        project: &crate::meta::Project,
        target: &Target,
        scripts: &[PathBuf],
        locale: Option<&str>,
        variant: Option<&str>,
        device: Option<&str>,
        summarize: bool,
    ) -> Self {
        let mut reports = Self {
            records: Vec::new(),
            active: Active::default(),
            interval_ms: 1000,
            summarize,
        };
        if !summarize
            && !matches!(
                std::env::var("DAY_SCRIPT_REPORT").as_deref(),
                Ok("1" | "true")
            )
        {
            return reports;
        }
        reports.interval_ms = std::env::var("DAY_SCRIPT_MEMORY_INTERVAL_MS")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|v| (100..=60_000).contains(v))
            .unwrap_or(1000);
        let root = crate::ops::staged_root(project).join("reports/dayscript");
        if let Err(error) = std::fs::create_dir_all(&root) {
            eprintln!("warning: cannot create dayscript report directory: {error}");
            return reports;
        }
        let hardware = hardware(target, &root);
        let default_locale = crate::store::default_locale(&crate::store::app_locales(project));
        let locale = locale.or(default_locale.as_deref());
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        for (index, script) in scripts.iter().enumerate() {
            let script_name = script
                .strip_prefix(&project.root)
                .unwrap_or(script)
                .to_string_lossy()
                .replace('\\', "/");
            let parsed = std::fs::read_to_string(script)
                .ok()
                .and_then(|s| day_script_proto::Script::from_yaml(&s).ok());
            let planned = parsed.as_ref().map(|s| s.steps.len()).unwrap_or(0);
            let run_id = format!(
                "{}-{}-{stamp}-{}-{index}",
                target.name,
                std::process::id(),
                sequence()
            );
            let mut live = Live {
                path: root.join(format!("{run_id}.json")),
                start: None,
                sum: 0.0,
                warned: false,
                value: json!({
                    "schema_version":1, "run_id":run_id, "script":script_name,
                    "name":parsed.as_ref().and_then(|s| s.metadata.get("name")),
                    "target":target.name, "locale":locale, "variant":variant, "device_slug":device,
                    "flavor":crate::flavor::active(), "hardware":hardware,
                    "ci":{"run_id":std::env::var("GITHUB_RUN_ID").ok(),"run_attempt":std::env::var("GITHUB_RUN_ATTEMPT").ok(),"job":std::env::var("GITHUB_JOB").ok()},
                    "status":"not_run", "started_at_unix_ms":null, "finished_at_unix_ms":null, "duration_ms":0,
                    "steps":{"planned":planned,"passed":0,"skipped":0,"failed":0,"aborted":0},
                    "current_step":null, "step_results":[], "error":null,
                    "memory":{"interval_ms":reports.interval_ms,"metric":null,"sample_count":0,"unavailable_count":0,"min_bytes":null,"max_bytes":null,"avg_bytes":null,"end_bytes":null,"end_elapsed_ms":null,"samples":[],"error":null}
                }),
            };
            live.save();
            reports.records.push(Arc::new(Mutex::new(live)));
        }
        reports.begin(0);
        reports
    }

    pub fn enabled(&self) -> bool {
        !self.records.is_empty()
    }
    pub fn begin(&self, index: usize) {
        if let Some(record) = self.records.get(index) {
            *self.active.0.lock().unwrap() = Some(record.clone());
            let mut live = record.lock().unwrap();
            if live.start.is_none() {
                live.start = Some(Instant::now());
                live.value["status"] = json!("running");
                live.value["started_at_unix_ms"] = json!(unix_ms());
                live.save();
            }
        }
    }
    pub fn memory_error(&self, message: &str) {
        if let Some(record) = self.active.record() {
            let mut live = record.lock().unwrap();
            live.value["memory"]["error"] = json!(message);
            live.save();
        }
    }
    pub fn step(&self, index: usize, op: &str) -> StepGuard {
        let record = self.active.record();
        if let Some(record) = &record {
            let mut live = record.lock().unwrap();
            live.value["current_step"] = json!({"index":index+1,"op":op});
            live.save();
        }
        StepGuard {
            record,
            index: index + 1,
            op: op.into(),
            outcome: "failed",
            error: Some("runner interrupted while executing this step".into()),
        }
    }
    pub fn complete(&self) {
        if let Some(record) = self.active.0.lock().unwrap().take() {
            record.lock().unwrap().finish(None);
        }
    }
    pub fn finish(&self, error: Option<&str>) {
        if let Some(record) = self.active.0.lock().unwrap().take() {
            record.lock().unwrap().finish(error);
        }
        if self.summarize {
            let mut samples = Vec::new();
            let mut end = None;
            let mut metric = String::from("unavailable");
            for record in &self.records {
                let live = record.lock().unwrap();
                if let Some(name) = live.value["memory"]["metric"].as_str() {
                    metric = name.into();
                }
                for sample in live.value["memory"]["samples"]
                    .as_array()
                    .into_iter()
                    .flatten()
                {
                    end = sample["bytes"].as_u64();
                    if let Some(bytes) = end {
                        samples.push(bytes);
                    }
                }
            }
            let mb = |value: Option<f64>| {
                value
                    .map(|v| format!("{:.2}", v / 1e6))
                    .unwrap_or_else(|| "unknown".into())
            };
            let avg = (!samples.is_empty())
                .then(|| samples.iter().map(|v| *v as f64).sum::<f64>() / samples.len() as f64);
            eprintln!(
                "Memory profile [{metric}]: min {} MB, max {} MB, avg {} MB, end {} MB ({} samples{})",
                mb(samples.iter().min().map(|v| *v as f64)),
                mb(samples.iter().max().map(|v| *v as f64)),
                mb(avg),
                mb(end.map(|v| v as f64)),
                samples.len(),
                if error.is_some() {
                    "; interrupted, last observation"
                } else {
                    ""
                }
            );
        }
    }
}

pub(crate) struct StepGuard {
    record: Option<Record>,
    index: usize,
    op: String,
    outcome: &'static str,
    error: Option<String>,
}
impl StepGuard {
    pub fn result(&mut self, outcome: &'static str, error: Option<String>) {
        self.outcome = outcome;
        self.error = error;
    }
}
impl Drop for StepGuard {
    fn drop(&mut self) {
        if let Some(record) = &self.record {
            let mut live = record.lock().unwrap();
            let count = live.value["steps"][self.outcome].as_u64().unwrap() + 1;
            live.value["steps"][self.outcome] = json!(count);
            live.value["step_results"].as_array_mut().unwrap().push(
                json!({"index":self.index,"op":self.op,"status":self.outcome,"error":self.error}),
            );
            live.value["current_step"] = Value::Null;
            live.save();
        }
    }
}

const READER_POLL: Duration = Duration::from_millis(250);

/// A single socket reader demultiplexes telemetry and ordinary responses. Each sample is
/// checkpointed immediately, including during host-side pauses and long-running UI steps.
pub(crate) struct Reader {
    replies: mpsc::Receiver<std::io::Result<String>>,
    stream: TcpStream,
    timeout: Duration,
    stopping: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl Reader {
    pub fn new(stream: TcpStream, active: Active) -> std::io::Result<Self> {
        let input = stream.try_clone()?;
        // A socket shutdown does not reliably interrupt an already pending Winsock read.
        // Poll cancellation independently of the (potentially minutes-long) reply budget.
        input.set_read_timeout(Some(READER_POLL))?;
        let (tx, replies) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stopping = stopping.clone();
        let thread = std::thread::spawn(move || {
            let mut reader = BufReader::new(input);
            // Keep raw bytes across poll timeouts, including a timeout halfway through a
            // UTF-8 character. Decode only once the complete reply has arrived.
            let mut line = Vec::new();
            while !worker_stopping.load(Ordering::Acquire) {
                match reader.read_until(b'\n', &mut line) {
                    Ok(0) => {
                        let _ = tx.send(Ok(String::new()));
                        break;
                    }
                    Ok(_) => {
                        let line = match String::from_utf8(std::mem::take(&mut line)) {
                            Ok(line) => line,
                            Err(error) => {
                                let _ = tx.send(Err(std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    error,
                                )));
                                break;
                            }
                        };
                        if line.contains("\"memory_sample\"")
                            && let Ok(reply) = serde_json::from_str::<Reply>(&line)
                            && let Some(sample) = reply.memory_sample
                        {
                            active.sample(*sample);
                        } else if tx.send(Ok(line)).is_err() {
                            break;
                        }
                    }
                    Err(error)
                        if matches!(
                            error.kind(),
                            std::io::ErrorKind::TimedOut
                                | std::io::ErrorKind::WouldBlock
                                | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(error) => {
                        let _ = tx.send(Err(error));
                        break;
                    }
                }
            }
        });
        Ok(Self {
            replies,
            stream,
            timeout: Duration::from_secs(60),
            stopping,
            thread: Some(thread),
        })
    }
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }
    pub fn read_line(&mut self, line: &mut String) -> std::io::Result<usize> {
        let reply = self.replies.recv_timeout(self.timeout).map_err(|e| {
            std::io::Error::new(
                if matches!(e, mpsc::RecvTimeoutError::Timeout) {
                    std::io::ErrorKind::TimedOut
                } else {
                    std::io::ErrorKind::UnexpectedEof
                },
                e,
            )
        })??;
        let len = reply.len();
        line.push_str(&reply);
        Ok(len)
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        // Close both directions before joining: the peer must see EOF even while the runner
        // retains its writer clone. Receive-only shutdown leaves the peer waiting for input.
        let _ = self.stream.shutdown(Shutdown::Both);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Bounded, allowlisted hardware probes. Capture to a temporary file so a command cannot
/// deadlock on a full stdout pipe, and never dump the host's environment or serial inventory.
fn probe(root: &Path, mut command: std::process::Command) -> Option<String> {
    use std::process::Stdio;
    let path = root.join(format!("probe-{}.tmp", std::process::id()));
    let file = std::fs::File::create(&path).ok()?;
    let mut child = command
        .stdout(Stdio::from(file))
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .ok()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    let success = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    let text = std::fs::read_to_string(&path).ok();
    let _ = std::fs::remove_file(path);
    text.filter(|s| success && !s.trim().is_empty())
        .map(|s| s.trim().chars().take(16_384).collect())
}
fn command(program: &str, args: &[&str]) -> std::process::Command {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    cmd
}
fn hardware(target: &Target, root: &Path) -> Value {
    let mut host = json!({"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"logical_cpus":std::thread::available_parallelism().ok().map(|n| n.get()),"model":null,"cpu":null,"memory_bytes":null,"os_version":null});
    if cfg!(target_os = "macos") {
        host["model"] = json!(probe(root, command("sysctl", &["-n", "hw.model"])));
        host["cpu"] = json!(probe(
            root,
            command("sysctl", &["-n", "machdep.cpu.brand_string"])
        ));
        host["memory_bytes"] = json!(
            probe(root, command("sysctl", &["-n", "hw.memsize"]))
                .and_then(|v| v.parse::<u64>().ok())
        );
        host["os_version"] = json!(probe(root, command("sw_vers", &["-productVersion"])));
    } else if cfg!(target_os = "linux") {
        host["model"] = json!(
            std::fs::read_to_string("/sys/class/dmi/id/product_name")
                .ok()
                .map(|s| s.trim().to_owned())
        );
        host["cpu"] =
            json!(
                std::fs::read_to_string("/proc/cpuinfo")
                    .ok()
                    .and_then(|s| s.lines().find_map(|l| l
                        .strip_prefix("model name")
                        .and_then(|l| l.split_once(':'))
                        .map(|(_, v)| v.trim().to_owned())))
            );
        host["memory_bytes"] =
            json!(
                std::fs::read_to_string("/proc/meminfo")
                    .ok()
                    .and_then(|s| s.lines().find_map(|l| l
                        .strip_prefix("MemTotal:")
                        .and_then(|v| v.split_whitespace().next())
                        .and_then(|v| v.parse::<u64>().ok())
                        .map(|v| v * 1024)))
            );
        host["os_version"] = json!(probe(root, command("uname", &["-sr"])));
    } else if cfg!(windows)
        && let Some(data) = probe(
            root,
            command(
                "powershell",
                &[
                    "-NoProfile",
                    "-Command",
                    "$c=Get-CimInstance Win32_ComputerSystem; $p=Get-CimInstance Win32_Processor | Select-Object -First 1; $o=Get-CimInstance Win32_OperatingSystem; @{model=$c.Model;cpu=$p.Name;memory_bytes=$c.TotalPhysicalMemory;os_version=$o.Version} | ConvertTo-Json -Compress",
                ],
            ),
        )
        && let Ok(data) = serde_json::from_str::<Value>(&data)
    {
        for key in ["model", "cpu", "memory_bytes", "os_version"] {
            host[key] = data[key].clone();
        }
    }
    let context = std::env::var("DAY_SCRIPT_REPORT_CONTEXT")
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|value| value.as_object().cloned())
        .map(|mut fields| {
            fields.retain(|key, _| {
                [
                    "target",
                    "label",
                    "os",
                    "profile",
                    "device_slug",
                    "device_os",
                    "device_orientation",
                    "device_density",
                    "flavor",
                    "tier",
                    "optional",
                ]
                .contains(&key.as_str())
            });
            Value::Object(fields)
        });
    let mut device = json!({"kind":format!("{:?}",target.kind),"id":null,"model":null,"os_version":null,"arch":null});
    match target.kind {
        TargetKind::Android => {
            let serial = crate::ops::selected_android_serial()
                .map(str::to_owned)
                .or_else(|| std::env::var("ANDROID_SERIAL").ok());
            device["id"] = json!(serial);
            if let Some(serial) = serial {
                for (key, prop) in [
                    ("model", "ro.product.model"),
                    ("os_version", "ro.build.version.release"),
                    ("arch", "ro.product.cpu.abi"),
                    ("build_fingerprint", "ro.build.fingerprint"),
                    ("emulator", "ro.kernel.qemu"),
                ] {
                    let mut cmd = std::process::Command::new(day_toolchain::adb_bin());
                    cmd.args(["-s", &serial, "shell", "getprop", prop]);
                    device[key] = json!(probe(root, cmd));
                }
            }
        }
        TargetKind::IosSim => {
            let id = crate::ops::selected_ios_simulator();
            device["id"] = json!(id);
            if let Some(id) = id
                && let Some(text) = probe(
                    root,
                    command("xcrun", &["simctl", "list", "devices", "booted", "--json"]),
                )
                && let Ok(data) = serde_json::from_str::<Value>(&text)
            {
                for (runtime, list) in data["devices"].as_object().into_iter().flatten() {
                    if let Some(found) = list
                        .as_array()
                        .and_then(|a| a.iter().find(|d| d["udid"] == id))
                    {
                        device["model"] = found["name"].clone();
                        device["os_version"] = json!(runtime);
                        device["device_type"] = found["deviceTypeIdentifier"].clone();
                    }
                }
            }
        }
        TargetKind::HarmonyOs => {
            device["id"] = json!(crate::ops::selected_ohos_key());
            for (key, prop) in [
                ("model", "const.product.model"),
                ("os_version", "const.ohos.fullname"),
                ("arch", "const.product.cpu.abilist"),
            ] {
                let mut cmd = crate::ohos::hdc();
                if let Some(id) = crate::ops::selected_ohos_key() {
                    cmd = crate::ohos::hdc_for(id);
                }
                cmd.args(["shell", "param", "get", prop]);
                device[key] = json!(probe(root, cmd));
            }
        }
        _ => {
            device["model"] = host["model"].clone();
            device["arch"] = host["arch"].clone();
            device["os_version"] = host["os_version"].clone();
        }
    }
    json!({"host":host,"device":device,"ci_profile":context})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Reports, PathBuf) {
        let root = std::env::temp_dir().join(format!(
            "day-report-fixture-{}-{}-{}",
            std::process::id(),
            sequence(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let live = Live {
            path: root.join("report.json"),
            start: None,
            sum: 0.0,
            warned: false,
            value: json!({"schema_version":1,"status":"not_run","duration_ms":0,"steps":{"planned":4,"passed":0,"skipped":0,"failed":0,"aborted":0},"step_results":[],"memory":{"samples":[],"sample_count":0,"unavailable_count":0}}),
        };
        let reports = Reports {
            records: vec![Arc::new(Mutex::new(live))],
            active: Active::default(),
            interval_ms: 100,
            summarize: false,
        };
        reports.begin(0);
        (reports, root)
    }

    #[test]
    fn partial_reports_preserve_samples_and_exact_step_outcomes() {
        let (reports, root) = fixture();
        {
            let mut step = reports.step(0, "wait_for");
            step.result("passed", None);
        }
        {
            let mut step = reports.step(1, "tap");
            step.result("skipped", None);
        }
        {
            let _inflight = reports.step(2, "assert_visible");
        } // Unwinding/transport failure.
        for bytes in [Some(100), Some(300), Some(200)] {
            reports.active.sample(MemorySample {
                bytes,
                metric: "fixture".into(),
            });
        }
        reports.finish(Some("fixture app crash"));
        let data: Value =
            serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
        assert_eq!(data["status"], "interrupted");
        assert_eq!(
            data["steps"],
            json!({"planned":4,"passed":1,"skipped":1,"failed":1,"aborted":1})
        );
        assert_eq!(data["memory"]["min_bytes"], 100);
        assert_eq!(data["memory"]["max_bytes"], 300);
        assert_eq!(data["memory"]["avg_bytes"], 200.0);
        assert_eq!(data["memory"]["end_bytes"], 200);
        assert_eq!(data["memory"]["samples"].as_array().unwrap().len(), 3);
        assert!(!root.join("report.tmp").exists());
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reader_checkpoints_during_host_waits_and_keeps_telemetry_out_of_replies() {
        let (reports, root) = fixture();
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (finish, done) = mpsc::channel();
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            writeln!(
                socket,
                "{}",
                serde_json::to_string(&Reply {
                    memory_sample: Some(Box::new(MemorySample {
                        bytes: Some(1234),
                        metric: "fixture".into()
                    })),
                    ..Reply::ok()
                })
                .unwrap()
            )
            .unwrap();
            done.recv_timeout(Duration::from_secs(3)).unwrap();
            writeln!(socket, "{{\"ok\":false,\"error\":\"fixture failure\"}}").unwrap();
        });
        let mut reader = Reader::new(socket, reports.active.clone()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            let data: Value =
                serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
            if data["memory"]["sample_count"] == 1 {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "sample was not persisted during a host wait"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        finish.send(()).unwrap();
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let reply: Reply = serde_json::from_str(&line).unwrap();
        assert_eq!(reply.error.as_deref(), Some("fixture failure"));
        assert!(reply.memory_sample.is_none());
        server.join().unwrap();
        drop(reader);
        reports.finish(Some("fixture connection closed"));
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn dropping_reader_disconnects_an_idle_peer_even_with_a_writer_clone() {
        use std::io::Read;

        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let writer = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        peer.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let mut reader = Reader::new(writer.try_clone().unwrap(), Active::default()).unwrap();
        // Confirm the worker has started before dropping it with the peer still connected.
        writeln!(peer, "{{\"ok\":true}}").unwrap();
        reader.read_line(&mut String::new()).unwrap();
        let (finished, completion) = mpsc::channel();
        let dropper = std::thread::spawn(move || {
            drop(reader);
            finished.send(()).unwrap();
        });
        // Neither completion nor peer EOF may depend on the peer closing first. The runner
        // also retains a separate writer until after its Reader has been dropped.
        completion
            .recv_timeout(Duration::from_secs(3))
            .expect("reader teardown must not wait for an idle peer or a step timeout");
        let mut byte = [0];
        assert_eq!(peer.read(&mut byte).unwrap(), 0, "the peer must see EOF");
        drop(writer);
        dropper.join().unwrap();
    }

    #[test]
    fn reader_preserves_fragmented_utf8_across_poll_and_reply_timeouts() {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let mut reader = Reader::new(socket, Active::default()).unwrap();
        let expected = "{\"ok\":false,\"error\":\"fixture café\"}\n";
        let split = expected.find('é').unwrap() + 1;
        peer.write_all(&expected.as_bytes()[..split]).unwrap();
        reader.set_timeout(READER_POLL * 3);
        let mut line = String::new();
        assert_eq!(
            reader.read_line(&mut line).unwrap_err().kind(),
            std::io::ErrorKind::TimedOut
        );
        assert!(line.is_empty(), "partial replies stay inside the worker");
        peer.write_all(&expected.as_bytes()[split..]).unwrap();
        reader.set_timeout(Duration::from_secs(3));
        assert_eq!(reader.read_line(&mut line).unwrap(), expected.len());
        assert_eq!(line, expected);
    }

    #[test]
    fn unavailable_final_reading_is_not_replaced_with_an_older_value() {
        let (reports, root) = fixture();
        reports.active.sample(MemorySample {
            bytes: Some(100),
            metric: "fixture".into(),
        });
        reports.active.sample(MemorySample {
            bytes: None,
            metric: "fixture".into(),
        });
        reports.complete();
        let data: Value =
            serde_json::from_slice(&std::fs::read(root.join("report.json")).unwrap()).unwrap();
        assert!(data["memory"]["end_bytes"].is_null());
        assert_eq!(data["memory"]["avg_bytes"], 100.0);
        assert_eq!(data["memory"]["unavailable_count"], 1);
        std::fs::remove_dir_all(root).unwrap();
    }
}
