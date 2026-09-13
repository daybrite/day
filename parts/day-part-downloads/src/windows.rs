// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The system tier on Windows: the Background Intelligent Transfer Service (BITS). BITS runs each
//! download as a job owned by the user, so the transfer continues after the app quits, and the
//! journal keeps the job's id to follow it on the next run. One worker thread owns every COM
//! object this module makes: it takes commands over a channel and polls the jobs it follows about
//! four times a second while any are running. Events reach the manager from a second thread, so
//! a watch callback may call back into the manager without waiting on the worker.

use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, mpsc};
use std::time::{Duration, Instant};

use day_part_http::HttpError;
use windows::Win32::Networking::BackgroundIntelligentTransferService::{
    BG_ERROR_CONTEXT, BG_JOB_PROGRESS, BG_JOB_STATE_ACKNOWLEDGED, BG_JOB_STATE_CANCELLED,
    BG_JOB_STATE_ERROR, BG_JOB_STATE_SUSPENDED, BG_JOB_STATE_TRANSFERRED,
    BG_JOB_STATE_TRANSIENT_ERROR, BG_JOB_TYPE_DOWNLOAD, BackgroundCopyManager,
    IBackgroundCopyError, IBackgroundCopyJob, IBackgroundCopyJobHttpOptions,
    IBackgroundCopyManager,
};
use windows::Win32::System::Com::{
    CLSCTX_LOCAL_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
};
use windows::core::{GUID, HRESULT, Interface, PCWSTR};

use crate::{DownloadError, SystemEvent, SystemEvents, SystemJob};

/// How often the worker reads the state of the jobs it follows.
const POLL: Duration = Duration::from_millis(250);

/// BITS reports an HTTP status as an HRESULT in this facility, with the status in the low word.
const HTTP_FACILITY: u32 = 0x8019_0000;

enum Command {
    Start {
        reference: Option<String>,
        url: String,
        headers: String,
        title: String,
        part: PathBuf,
        events: SystemEvents,
        /// The job's reference, or `None` when the job to follow is gone and a failure is on its
        /// way.
        reply: mpsc::Sender<Result<Option<String>, DownloadError>>,
    },
    Pause {
        reference: String,
        reply: mpsc::Sender<Result<(), DownloadError>>,
    },
    Cancel {
        reference: String,
        reply: mpsc::Sender<()>,
    },
}

/// A job the worker polls for a manager.
struct Followed {
    job: IBackgroundCopyJob,
    reference: String,
    /// Where BITS puts the body when the job completes.
    temp: PathBuf,
    part: PathBuf,
    events: SystemEvents,
    last: Option<(u64, Option<u64>)>,
}

type Deliveries = mpsc::Sender<(SystemEvents, SystemEvent)>;

/// The worker's command channel, or `None` when BITS is not available.
fn worker() -> Option<&'static mpsc::Sender<Command>> {
    static WORKER: OnceLock<Option<mpsc::Sender<Command>>> = OnceLock::new();
    WORKER
        .get_or_init(|| {
            let (commands, inbox) = mpsc::channel();
            let (ready, readiness) = mpsc::channel();
            std::thread::Builder::new()
                .name("day-downloads-bits".into())
                .spawn(move || run(inbox, ready))
                .ok()?;
            readiness.recv().ok()?.then_some(commands)
        })
        .as_ref()
}

/// Send one command and wait for its reply.
fn ask<T>(command: impl FnOnce(mpsc::Sender<T>) -> Command) -> Option<T> {
    let worker = worker()?;
    let (reply, answer) = mpsc::channel();
    worker.send(command(reply)).ok()?;
    answer.recv().ok()
}

pub(crate) fn available() -> bool {
    worker().is_some()
}

pub(crate) fn start(job: SystemJob<'_>, events: SystemEvents) -> Result<(), DownloadError> {
    // BITS takes custom headers as one CRLF-separated block.
    let headers: String = job
        .request
        .headers()
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let answer = ask(|reply| Command::Start {
        reference: job.reference.map(str::to_string),
        url: job.request.url().to_string(),
        headers,
        title: job.title.to_string(),
        part: job.part.to_path_buf(),
        events: events.clone(),
        reply,
    });
    match answer {
        None => Err(DownloadError::Unsupported),
        Some(Err(error)) => Err(error),
        Some(Ok(Some(reference))) => {
            events(SystemEvent::Started(reference));
            Ok(())
        }
        Some(Ok(None)) => Ok(()),
    }
}

pub(crate) fn pause(_dir: &Path, _id: u64, reference: Option<&str>) -> Result<(), DownloadError> {
    let reference =
        reference.ok_or_else(|| DownloadError::Io("the transfer has not started yet".into()))?;
    ask(|reply| Command::Pause {
        reference: reference.to_string(),
        reply,
    })
    .unwrap_or(Err(DownloadError::Unsupported))
}

pub(crate) fn cancel(dir: &Path, id: u64, reference: Option<&str>) {
    if let Some(reference) = reference {
        let _ = ask(|reply| Command::Cancel {
            reference: reference.to_string(),
            reply,
        });
    }
    // The manager's partial file for `id` is `{id}.part`; a completed job left its body beside it.
    let _ = std::fs::remove_file(dir.join(format!("{id}.part.bits")));
}

/// The worker: COM, the BITS manager, and the jobs it follows.
fn run(inbox: mpsc::Receiver<Command>, ready: mpsc::Sender<bool>) {
    // SAFETY: initializes COM on this new thread, which owns every BITS object this module makes.
    let initialized = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if initialized.is_err() {
        let _ = ready.send(false);
        return;
    }
    // SAFETY: COM is initialized on this thread, and the class id is BITS's own manager.
    let manager: IBackgroundCopyManager =
        match unsafe { CoCreateInstance(&BackgroundCopyManager, None, CLSCTX_LOCAL_SERVER) } {
            Ok(manager) => manager,
            Err(_) => {
                let _ = ready.send(false);
                return;
            }
        };
    let (deliver, deliveries) = mpsc::channel::<(SystemEvents, SystemEvent)>();
    let delivering = std::thread::Builder::new()
        .name("day-downloads-bits-events".into())
        .spawn(move || {
            for (events, event) in deliveries {
                events(event);
            }
        });
    if delivering.is_err() {
        let _ = ready.send(false);
        return;
    }
    let _ = ready.send(true);

    let mut jobs: Vec<Followed> = Vec::new();
    let mut polled = Instant::now();
    loop {
        let received = if jobs.is_empty() {
            inbox
                .recv()
                .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
        } else {
            inbox.recv_timeout(POLL.saturating_sub(polled.elapsed()))
        };
        match received {
            Ok(command) => handle(&manager, &mut jobs, &deliver, command),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
        if !jobs.is_empty() && polled.elapsed() >= POLL {
            jobs.retain_mut(|followed| step(followed, &deliver));
            polled = Instant::now();
        }
    }
}

fn handle(
    manager: &IBackgroundCopyManager,
    jobs: &mut Vec<Followed>,
    deliver: &Deliveries,
    command: Command,
) {
    match command {
        Command::Start {
            reference,
            url,
            headers,
            title,
            part,
            events,
            reply,
        } => {
            let temp = temp_of(&part);
            let opened = match &reference {
                Some(reference) => match follow(manager, reference) {
                    Ok(job) => Ok((job, reference.clone())),
                    Err(error) => {
                        // The job is gone: a new transfer starts after the retry.
                        let _ = deliver.send((
                            events,
                            SystemEvent::Failed {
                                error,
                                transient: true,
                            },
                        ));
                        let _ = reply.send(Ok(None));
                        return;
                    }
                },
                None => create(manager, &url, &headers, &title, &temp),
            };
            match opened {
                Ok((job, reference)) => {
                    // A resume after a pause replaces the follower the pause dropped.
                    jobs.retain(|f| f.reference != reference);
                    jobs.push(Followed {
                        job,
                        reference: reference.clone(),
                        temp,
                        part,
                        events,
                        last: None,
                    });
                    let _ = reply.send(Ok(Some(reference)));
                }
                Err(error) => {
                    let _ = reply.send(Err(error));
                }
            }
        }
        Command::Pause { reference, reply } => {
            jobs.retain(|f| f.reference != reference);
            let paused = job_of(manager, &reference).and_then(|job| {
                // SAFETY: a job this thread holds.
                unsafe { job.Suspend() }.map_err(com)
            });
            let _ = reply.send(paused);
        }
        Command::Cancel { reference, reply } => {
            jobs.retain(|f| f.reference != reference);
            if let Ok(job) = job_of(manager, &reference) {
                // SAFETY: a job this thread holds.
                let _ = unsafe { job.Cancel() };
            }
            let _ = reply.send(());
        }
    }
}

/// Create a download job for `url` into `temp` and start it.
fn create(
    manager: &IBackgroundCopyManager,
    url: &str,
    headers: &str,
    title: &str,
    temp: &Path,
) -> Result<(IBackgroundCopyJob, String), DownloadError> {
    let _ = std::fs::remove_file(temp);
    let title = wide(title);
    let mut id = GUID::zeroed();
    let mut created: Option<IBackgroundCopyJob> = None;
    // SAFETY: a NUL-terminated display name and two out-parameters valid for writes.
    unsafe {
        manager.CreateJob(
            PCWSTR(title.as_ptr()),
            BG_JOB_TYPE_DOWNLOAD,
            &mut id,
            &mut created,
        )
    }
    .map_err(com)?;
    let job = created.ok_or_else(|| DownloadError::Io("BITS created no job".into()))?;
    let url = wide(url);
    let headers = wide(headers);
    let local: Vec<u16> = temp.as_os_str().encode_wide().chain(Some(0)).collect();
    let prepared = (|| -> windows::core::Result<()> {
        if headers.len() > 1 {
            let options: IBackgroundCopyJobHttpOptions = job.cast()?;
            // SAFETY: a NUL-terminated header block that outlives the call.
            unsafe { options.SetCustomHeaders(PCWSTR(headers.as_ptr())) }?;
        }
        // SAFETY: NUL-terminated URL and local path that outlive the call.
        unsafe { job.AddFile(PCWSTR(url.as_ptr()), PCWSTR(local.as_ptr())) }?;
        // SAFETY: a job this thread holds.
        unsafe { job.Resume() }
    })();
    if let Err(error) = prepared {
        // SAFETY: a job this thread holds; a job that never started is discarded.
        let _ = unsafe { job.Cancel() };
        return Err(com(error));
    }
    Ok((job, reference_of(&id)))
}

/// The job an earlier run started, running again if it was suspended.
fn follow(
    manager: &IBackgroundCopyManager,
    reference: &str,
) -> Result<IBackgroundCopyJob, DownloadError> {
    let job = job_of(manager, reference)?;
    // SAFETY: a job this thread holds.
    if unsafe { job.GetState() }.map_err(com)? == BG_JOB_STATE_SUSPENDED {
        // SAFETY: as above.
        unsafe { job.Resume() }.map_err(com)?;
    }
    Ok(job)
}

fn job_of(
    manager: &IBackgroundCopyManager,
    reference: &str,
) -> Result<IBackgroundCopyJob, DownloadError> {
    let id = parse_reference(reference)
        .ok_or_else(|| DownloadError::Io(format!("{reference} is not a BITS job id")))?;
    // SAFETY: a valid GUID the call reads.
    unsafe { manager.GetJob(&id) }
        .map_err(|_| DownloadError::Io("the system no longer has this download".into()))
}

/// Read one job's state, report it, and say whether to keep following it.
fn step(followed: &mut Followed, deliver: &Deliveries) -> bool {
    let send = |event| {
        let _ = deliver.send((followed.events.clone(), event));
    };
    // SAFETY: a job this thread holds.
    let state = match unsafe { followed.job.GetState() } {
        Ok(state) => state,
        Err(error) => {
            send(SystemEvent::Failed {
                error: com(error),
                transient: true,
            });
            return false;
        }
    };
    let mut progress = BG_JOB_PROGRESS::default();
    // SAFETY: a job this thread holds and an out-parameter valid for writes.
    if unsafe { followed.job.GetProgress(&mut progress) }.is_ok() {
        // An unknown size reads as all ones.
        let total = (progress.BytesTotal != u64::MAX).then_some(progress.BytesTotal);
        let now = (progress.BytesTransferred, total);
        if followed.last != Some(now) {
            followed.last = Some(now);
            send(SystemEvent::Progress {
                received: progress.BytesTransferred,
                total,
            });
        }
    }
    if state == BG_JOB_STATE_TRANSFERRED {
        // Complete renames BITS's temporary file to the job's local name.
        // SAFETY: a job this thread holds.
        let completed = unsafe { followed.job.Complete() }
            .map_err(com)
            .and_then(|()| crate::move_file(&followed.temp, &followed.part));
        send(match completed {
            Ok(()) => SystemEvent::Finished,
            Err(error) => SystemEvent::Failed {
                error,
                transient: false,
            },
        });
        return false;
    }
    if state == BG_JOB_STATE_ERROR || state == BG_JOB_STATE_TRANSIENT_ERROR {
        let (error, transient) = job_error(&followed.job, state == BG_JOB_STATE_TRANSIENT_ERROR);
        // SAFETY: a job this thread holds.
        let _ = unsafe { followed.job.Cancel() };
        let _ = std::fs::remove_file(&followed.temp);
        send(SystemEvent::Failed { error, transient });
        return false;
    }
    if state == BG_JOB_STATE_CANCELLED || state == BG_JOB_STATE_ACKNOWLEDGED {
        send(SystemEvent::Failed {
            error: DownloadError::Io("the system no longer has this download".into()),
            transient: true,
        });
        return false;
    }
    true
}

/// A failed job's error, and whether another attempt may succeed.
fn job_error(job: &IBackgroundCopyJob, transient: bool) -> (DownloadError, bool) {
    let unknown = || (DownloadError::Io("the transfer failed".into()), transient);
    // SAFETY: a job this thread holds.
    let Ok(error) = (unsafe { job.GetError() }) else {
        return unknown();
    };
    let mut context = BG_ERROR_CONTEXT::default();
    let mut code = HRESULT(0);
    // SAFETY: two out-parameters valid for writes.
    if unsafe { error.GetError(&mut context, &mut code) }.is_err() {
        return unknown();
    }
    let hresult = code.0 as u32;
    if hresult & 0xFFFF_0000 == HTTP_FACILITY {
        let status = (hresult & 0xFFFF) as u16;
        return (
            DownloadError::Http(HttpError::Status(status)),
            transient || matches!(status, 408 | 429 | 500..=599),
        );
    }
    let message = description(&error).unwrap_or_else(|| format!("BITS error {hresult:#010x}"));
    (DownloadError::Io(message), transient)
}

fn description(error: &IBackgroundCopyError) -> Option<String> {
    // SAFETY: an error object this thread holds; 0 asks for the user's default language.
    let text = unsafe { error.GetErrorDescription(0) }.ok()?;
    // SAFETY: BITS returns a NUL-terminated string it allocated with CoTaskMemAlloc; it is read
    // once and then freed.
    let message = unsafe { text.to_string() }.ok();
    // SAFETY: as above.
    unsafe { CoTaskMemFree(Some(text.0 as *const std::ffi::c_void)) };
    message.map(|m| m.trim().to_string())
}

fn com(error: windows::core::Error) -> DownloadError {
    DownloadError::Io(format!("BITS: {error}"))
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(Some(0)).collect()
}

/// Where BITS writes the body for the partial file at `part`.
fn temp_of(part: &Path) -> PathBuf {
    let mut name = part.as_os_str().to_owned();
    name.push(".bits");
    PathBuf::from(name)
}

/// A job id as the journal keeps it: 32 hex digits.
fn reference_of(id: &GUID) -> String {
    format!("{:032x}", id.to_u128())
}

fn parse_reference(reference: &str) -> Option<GUID> {
    (reference.len() == 32)
        .then(|| u128::from_str_radix(reference, 16).ok())
        .flatten()
        .map(GUID::from_u128)
}
