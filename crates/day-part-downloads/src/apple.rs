// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The system tier on macOS and iOS: one background `URLSession` for the process. Foundation runs
//! its transfers out of process, so a download continues while the app is suspended or after it
//! quits. Each task's description names the manager's directory and the download's id, so the
//! delegate can put a finished body in place and keep resume data even when the event arrives
//! before the manager that asked for the download has opened again.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::ptr::NonNull;
use std::sync::{LazyLock, Mutex, MutexGuard, OnceLock, mpsc};
use std::time::Duration;

use block2::RcBlock;
use day_part_http::{HttpError, Request};
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{AllocAnyThread, define_class, msg_send};
use objc2_foundation::{
    NSArray, NSBundle, NSData, NSError, NSHTTPURLResponse, NSMutableURLRequest, NSObject,
    NSObjectProtocol, NSString, NSURL, NSURLSession, NSURLSessionConfiguration,
    NSURLSessionDelegate, NSURLSessionDownloadDelegate, NSURLSessionDownloadTask,
    NSURLSessionDownloadTaskResumeData, NSURLSessionTask, NSURLSessionTaskDelegate,
    NSURLSessionTaskState,
};

use crate::{DownloadError, SystemEvent, SystemEvents, SystemJob};

/// A Foundation object used from several threads.
struct Shared<T>(Retained<T>);

// SAFETY: URLSession and its tasks are thread-safe (the URL Loading System documentation), and
// this module calls only those objects' methods through the wrapper.
unsafe impl<T> Send for Shared<T> {}
// SAFETY: as above.
unsafe impl<T> Sync for Shared<T> {}

#[derive(Default)]
struct Registry {
    /// The managers listening, by task description.
    listeners: HashMap<String, SystemEvents>,
    /// The task each description names in this process.
    tasks: HashMap<String, Shared<NSURLSessionDownloadTask>>,
    /// Why a finished task's body was not put in place, until the task completes.
    failures: HashMap<String, (DownloadError, bool)>,
}

static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

fn registry() -> MutexGuard<'static, Registry> {
    REGISTRY.lock().unwrap_or_else(|p| p.into_inner())
}

/// The process's background session, made on first use. A bare executable has no bundle
/// identifier to key one to, and so no system tier.
fn session() -> Option<&'static NSURLSession> {
    static SESSION: OnceLock<Option<Shared<NSURLSession>>> = OnceLock::new();
    SESSION
        .get_or_init(|| {
            let bundle = NSBundle::mainBundle().bundleIdentifier()?;
            let identifier = NSString::from_str(&format!("{bundle}.day-downloads"));
            let conf = NSURLSessionConfiguration::backgroundSessionConfigurationWithIdentifier(
                &identifier,
            );
            conf.setDiscretionary(false);
            conf.setSessionSendsLaunchEvents(true);
            let delegate = DownloadDelegate::new();
            // SAFETY: a delegate session that lives as long as the process; `None` gives it its
            // own serial delegate queue.
            let session = unsafe {
                NSURLSession::sessionWithConfiguration_delegate_delegateQueue(
                    &conf,
                    Some(ProtocolObject::from_ref(&*delegate)),
                    None,
                )
            };
            Some(Shared(session))
        })
        .as_ref()
        .map(|s| &*s.0)
}

fn key(dir: &Path, id: u64) -> String {
    format!("{}\n{id}", dir.display())
}

fn parse_key(key: &str) -> Option<(PathBuf, u64)> {
    let (dir, id) = key.rsplit_once('\n')?;
    Some((PathBuf::from(dir), id.parse().ok()?))
}

fn part_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.part"))
}

fn resume_path(dir: &Path, id: u64) -> PathBuf {
    dir.join(format!("{id}.resume"))
}

fn description(task: &NSURLSessionTask) -> Option<String> {
    task.taskDescription().map(|d| d.to_string())
}

fn listener(key: &str) -> Option<SystemEvents> {
    registry().listeners.get(key).cloned()
}

pub(crate) fn available() -> bool {
    session().is_some()
}

pub(crate) fn start(job: SystemJob<'_>, events: SystemEvents) -> Result<(), DownloadError> {
    let session = session().ok_or(DownloadError::Unsupported)?;
    let key = key(job.dir, job.id);
    registry().listeners.insert(key.clone(), events.clone());
    events(SystemEvent::Started(key.clone()));
    if job.reference.is_some() {
        let running = all_tasks(session).into_iter().find(|task| {
            description(&task.0).as_deref() == Some(key.as_str())
                && matches!(
                    task.0.state(),
                    NSURLSessionTaskState::Running | NSURLSessionTaskState::Suspended
                )
        });
        if let Some(task) = running {
            task.0.resume();
            registry().tasks.insert(key, task);
            return Ok(());
        }
        if job.part.exists() {
            events(SystemEvent::Finished);
            return Ok(());
        }
    }
    let resume = resume_path(job.dir, job.id);
    let task = match std::fs::read(&resume) {
        Ok(bytes) if !bytes.is_empty() => {
            let _ = std::fs::remove_file(&resume);
            session.downloadTaskWithResumeData(&NSData::with_bytes(&bytes))
        }
        _ => {
            let request = native_request(job.request)?;
            session.downloadTaskWithRequest(&request)
        }
    };
    task.setTaskDescription(Some(&NSString::from_str(&key)));
    registry().tasks.insert(key, Shared(task.clone()));
    task.resume();
    Ok(())
}

pub(crate) fn pause(dir: &Path, id: u64, _reference: Option<&str>) -> Result<(), DownloadError> {
    let key = key(dir, id);
    let task = {
        let mut registry = registry();
        // The cancellation that produces the resume data is the app's own: no manager hears it.
        registry.listeners.remove(&key);
        registry.tasks.remove(&key)
    };
    let Some(task) = task else {
        return Ok(());
    };
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(Some(tx));
    let block = RcBlock::new(move |data: *mut NSData| {
        // SAFETY: Foundation passes resume data or null, valid for the duration of the call.
        let bytes = unsafe { data.as_ref() }.map(|d| d.to_vec());
        if let Some(tx) = tx.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = tx.send(bytes);
        }
    });
    // SAFETY: the block owns everything it captures, and Foundation calls it once.
    unsafe { task.0.cancelByProducingResumeData(&block) };
    // Without resume data the next attempt starts over.
    if let Ok(Some(bytes)) = rx.recv_timeout(Duration::from_secs(5)) {
        std::fs::write(resume_path(dir, id), bytes)
            .map_err(|e| DownloadError::Io(e.to_string()))?;
    }
    Ok(())
}

pub(crate) fn cancel(dir: &Path, id: u64, _reference: Option<&str>) {
    let key = key(dir, id);
    let task = {
        let mut registry = registry();
        registry.listeners.remove(&key);
        registry.failures.remove(&key);
        registry.tasks.remove(&key)
    };
    match task {
        Some(task) => task.0.cancel(),
        // A task an earlier run of the app started.
        None => {
            if let Some(session) = session() {
                for task in all_tasks(session) {
                    if description(&task.0).as_deref() == Some(key.as_str()) {
                        task.0.cancel();
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_file(resume_path(dir, id));
}

/// The session's download tasks.
fn all_tasks(session: &NSURLSession) -> Vec<Shared<NSURLSessionDownloadTask>> {
    let (tx, rx) = mpsc::channel();
    let tx = Mutex::new(Some(tx));
    let block = RcBlock::new(move |tasks: NonNull<NSArray<NSURLSessionTask>>| {
        // SAFETY: Foundation hands the block an array that is valid for the duration of the call.
        let tasks = unsafe { tasks.as_ref() };
        let found: Vec<Shared<NSURLSessionDownloadTask>> = tasks
            .iter()
            .filter_map(|task| task.downcast::<NSURLSessionDownloadTask>().ok())
            .map(Shared)
            .collect();
        if let Some(tx) = tx.lock().unwrap_or_else(|p| p.into_inner()).take() {
            let _ = tx.send(found);
        }
    });
    // SAFETY: the block owns everything it captures, and Foundation calls it once.
    unsafe { session.getAllTasksWithCompletionHandler(&block) };
    rx.recv_timeout(Duration::from_secs(5)).unwrap_or_default()
}

/// The native request: a download sends only its URL, method and headers.
fn native_request(request: &Request) -> Result<Retained<NSMutableURLRequest>, DownloadError> {
    let url = NSURL::URLWithString(&NSString::from_str(request.url()))
        .filter(|u| u.scheme().is_some())
        .ok_or_else(|| DownloadError::Http(HttpError::BadUrl(request.url().to_string())))?;
    let native = NSMutableURLRequest::requestWithURL(&url);
    native.setHTTPMethod(&NSString::from_str(request.method().as_str()));
    for (name, value) in request.headers() {
        native.addValue_forHTTPHeaderField(&NSString::from_str(value), &NSString::from_str(name));
    }
    Ok(native)
}

/// The body is whole at `location`, which Foundation deletes when this returns.
fn finished(task: &NSURLSessionDownloadTask, location: &NSURL) {
    let Some(key) = description(task) else {
        return;
    };
    let Some((dir, id)) = parse_key(&key) else {
        return;
    };
    let status = task
        .response()
        .and_then(|r| r.downcast::<NSHTTPURLResponse>().ok())
        .map(|r| r.statusCode());
    let failure = match status {
        // A download task saves any response's body, so the status decides.
        Some(code) if !(200..300).contains(&code) => {
            let code = u16::try_from(code).unwrap_or(0);
            Some((
                DownloadError::Http(HttpError::Status(code)),
                matches!(code, 408 | 429 | 500..=599),
            ))
        }
        _ => match location.path() {
            Some(path) => crate::move_file(Path::new(&path.to_string()), &part_path(&dir, id))
                .err()
                .map(|e| (e, false)),
            None => Some((
                DownloadError::Io("the finished download has no file".into()),
                false,
            )),
        },
    };
    if let Some(failure) = failure {
        registry().failures.insert(key, failure);
    }
}

fn completed(task: &NSURLSessionTask, error: Option<&NSError>) {
    let Some(key) = description(task) else {
        return;
    };
    let (ours, failure, events) = {
        let mut registry = registry();
        // A task this process replaced (a pause, then a resume) reports nothing.
        let ours = registry
            .tasks
            .get(&key)
            .is_none_or(|t| t.0.taskIdentifier() == task.taskIdentifier());
        if !ours {
            return;
        }
        registry.tasks.remove(&key);
        (
            ours,
            registry.failures.remove(&key),
            registry.listeners.get(&key).cloned(),
        )
    };
    let event = match (error, failure) {
        (Some(error), _) => {
            // Kept for the next attempt, which continues from it.
            if let (true, Some(data), Some((dir, id))) = (ours, resume_data(error), parse_key(&key))
            {
                let _ = std::fs::write(resume_path(&dir, id), data);
            }
            let (error, transient) = error_of(error);
            SystemEvent::Failed { error, transient }
        }
        (None, Some((error, transient))) => SystemEvent::Failed { error, transient },
        (None, None) => SystemEvent::Finished,
    };
    if let Some(events) = events {
        events(event);
    }
}

fn resume_data(error: &NSError) -> Option<Vec<u8>> {
    // SAFETY: a Foundation constant, valid for the life of the process.
    let key = unsafe { NSURLSessionDownloadTaskResumeData };
    error
        .userInfo()
        .objectForKey(key)?
        .downcast::<NSData>()
        .ok()
        .map(|data| data.to_vec())
}

/// A URL Loading System error, and whether another attempt may succeed.
fn error_of(error: &NSError) -> (DownloadError, bool) {
    let message = error.localizedDescription().to_string();
    let http = match error.code() {
        -999 => HttpError::Cancelled,
        -1001 => HttpError::Timeout,
        -1003 | -1006 => HttpError::Dns,
        -1004 => HttpError::Connect,
        -1000 | -1002 => return (DownloadError::Http(HttpError::BadUrl(message)), false),
        code @ -1206..=-1200 => {
            return (
                DownloadError::Http(HttpError::Tls(format!("{message} ({code})"))),
                false,
            );
        }
        _ => HttpError::Io(message),
    };
    (DownloadError::Http(http), true)
}

define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = AllocAnyThread]
    #[name = "DayDownloadsSessionDelegate"]
    struct DownloadDelegate;

    unsafe impl NSObjectProtocol for DownloadDelegate {}
    unsafe impl NSURLSessionDelegate for DownloadDelegate {}

    unsafe impl NSURLSessionTaskDelegate for DownloadDelegate {
        #[unsafe(method(URLSession:task:didCompleteWithError:))]
        fn did_complete(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionTask,
            error: Option<&NSError>,
        ) {
            completed(task, error);
        }
    }

    unsafe impl NSURLSessionDownloadDelegate for DownloadDelegate {
        #[unsafe(method(URLSession:downloadTask:didFinishDownloadingToURL:))]
        fn did_finish(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionDownloadTask,
            location: &NSURL,
        ) {
            finished(task, location);
        }

        #[unsafe(method(URLSession:downloadTask:didWriteData:totalBytesWritten:totalBytesExpectedToWrite:))]
        fn did_write(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionDownloadTask,
            _bytes_written: i64,
            total_written: i64,
            expected: i64,
        ) {
            if let Some(events) = description(task).and_then(|key| listener(&key)) {
                events(SystemEvent::Progress {
                    received: u64::try_from(total_written).unwrap_or(0),
                    total: u64::try_from(expected).ok().filter(|t| *t > 0),
                });
            }
        }

        #[unsafe(method(URLSession:downloadTask:didResumeAtOffset:expectedTotalBytes:))]
        fn did_resume(
            &self,
            _session: &NSURLSession,
            task: &NSURLSessionDownloadTask,
            offset: i64,
            _expected: i64,
        ) {
            if let Some(events) = description(task).and_then(|key| listener(&key)) {
                events(SystemEvent::Resumed(u64::try_from(offset).unwrap_or(0)));
            }
        }
    }
);

impl DownloadDelegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's designated initializer on a freshly allocated instance.
        unsafe { msg_send![super(this), init] }
    }
}
