// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The system tier on Android and HarmonyOS (docs/bridge.md "Streams"): Android's
//! `DownloadManager` through a Java arm, HarmonyOS's request agent through an ArkTS arm. A
//! transfer is one `Emit<Vec<u8>>` stream of tagged frames: the OS's name for it, progress, the
//! finished file's path or a failure. The Rust half moves the finished file onto the manager's
//! partial file and reports to the manager.

use std::path::{Path, PathBuf};

use day_bridge::Item;
use day_part_http::HttpError;

use crate::{DownloadError, SystemEvent, SystemEvents, SystemJob};

day_bridge::bridge! {
    #[day_bridge::declare]
    extern "day" {
        /// Whether the OS can run a download here.
        fn system_ready_native() -> Result<bool, day_bridge::Error>;
        /// Start a download the OS runs, or follow the one `reference` names (empty for none).
        /// `headers` is `k\nv\n…`.
        fn system_start_native(
            reference: &str,
            url: &str,
            headers: &str,
            title: &str,
            emit: day_bridge::Emit<Vec<u8>>,
        ) -> Result<(), day_bridge::Error>;
        /// Pause a transfer; `false` where the OS cannot.
        fn system_pause_native(reference: &str) -> Result<bool, day_bridge::Error>;
        /// Stop a transfer for good and discard its file.
        fn system_cancel_native(reference: &str);
    }

    // Android: DownloadManager, which shows the transfer in a notification and continues it across
    // process death. It offers no callbacks for progress, so one handler thread polls each
    // transfer's row. The file lands in the app's external files directory; the Rust half moves
    // it, and the row is removed once the stream ends.
    #[day_bridge::impl(java, platforms = [android])]
    java!(
        prelude = r#"
            import android.app.DownloadManager;
            import android.content.Context;
            import android.database.Cursor;
            import android.net.Uri;
            import android.os.Environment;
            import android.os.Handler;
            import android.os.HandlerThread;
            import java.io.ByteArrayOutputStream;
            import java.io.DataOutputStream;
            import java.io.IOException;
            import java.nio.charset.StandardCharsets;
            import dev.daybrite.day.bridge.DayBridge;
        "#,
        body = r#"
            private static final int DAY_STARTED = 1;
            private static final int DAY_PROGRESS = 2;
            private static final int DAY_DONE = 3;
            private static final int DAY_FAILED = 4;
            private static final int DAY_GONE = 5;

            private static Handler dayPoller;

            private static final class DayFrame {
                private final ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                private final DataOutputStream out = new DataOutputStream(bytes);

                DayFrame(int tag) {
                    bytes.write(tag);
                }

                DayFrame i32(int value) {
                    try {
                        out.writeInt(value);
                    } catch (IOException e) {
                        // A ByteArrayOutputStream does not throw.
                    }
                    return this;
                }

                DayFrame i64(long value) {
                    try {
                        out.writeLong(value);
                    } catch (IOException e) {
                        // A ByteArrayOutputStream does not throw.
                    }
                    return this;
                }

                DayFrame str(String value) {
                    byte[] utf8 = value.getBytes(StandardCharsets.UTF_8);
                    i32(utf8.length);
                    bytes.write(utf8, 0, utf8.length);
                    return this;
                }

                byte[] done() {
                    return bytes.toByteArray();
                }
            }

            private static synchronized Handler dayPoller() {
                if (dayPoller == null) {
                    HandlerThread thread = new HandlerThread("day-downloads");
                    thread.start();
                    dayPoller = new Handler(thread.getLooper());
                }
                return dayPoller;
            }

            private static DownloadManager dayManager() {
                Context ctx = DayBridge.ctx;
                if (ctx == null) {
                    return null;
                }
                return (DownloadManager) ctx.getApplicationContext().getSystemService(Context.DOWNLOAD_SERVICE);
            }

            private static String dayReason(int reason) {
                switch (reason) {
                    case DownloadManager.ERROR_FILE_ERROR: return "the download could not be stored";
                    case DownloadManager.ERROR_UNHANDLED_HTTP_CODE: return "the server sent an unexpected response";
                    case DownloadManager.ERROR_HTTP_DATA_ERROR: return "the connection failed";
                    case DownloadManager.ERROR_TOO_MANY_REDIRECTS: return "too many redirects";
                    case DownloadManager.ERROR_INSUFFICIENT_SPACE: return "not enough space";
                    case DownloadManager.ERROR_DEVICE_NOT_FOUND: return "no storage for the download";
                    case DownloadManager.ERROR_CANNOT_RESUME: return "the download could not continue";
                    case DownloadManager.ERROR_FILE_ALREADY_EXISTS: return "the file already exists";
                    default: return reason >= 100 && reason < 600 ? "status " + reason : "the download failed";
                }
            }

            public static boolean system_ready_native() {
                return dayManager() != null;
            }

            public static void system_start_native(String reference, String url, String headers, String title, long emit) {
                DownloadManager dm = dayManager();
                if (dm == null) {
                    throw new IllegalStateException("DownloadManager is not available");
                }
                long id = -1;
                if (reference.isEmpty()) {
                    DownloadManager.Request request = new DownloadManager.Request(Uri.parse(url));
                    String[] lines = headers.split("\n", -1);
                    for (int i = 0; i + 1 < lines.length; i += 2) {
                        request.addRequestHeader(lines[i], lines[i + 1]);
                    }
                    request.setTitle(title);
                    request.setNotificationVisibility(DownloadManager.Request.VISIBILITY_VISIBLE);
                    request.setDestinationInExternalFilesDir(DayBridge.ctx, Environment.DIRECTORY_DOWNLOADS,
                        "day-" + System.nanoTime() + ".part");
                    id = dm.enqueue(request);
                } else {
                    try {
                        id = Long.parseLong(reference);
                    } catch (NumberFormatException e) {
                        system_start_native_emit(emit, new DayFrame(DAY_GONE).done());
                        system_start_native_end(emit);
                        return;
                    }
                }
                final long download = id;
                system_start_native_emit(emit, new DayFrame(DAY_STARTED).str(Long.toString(download)).done());
                dayPoller().post(new Runnable() {
                    private long reported = -2;

                    @Override
                    public void run() {
                        DownloadManager dm = dayManager();
                        if (dm == null) {
                            system_start_native_fail(emit, "DownloadManager is not available");
                            return;
                        }
                        Cursor row = null;
                        try {
                            row = dm.query(new DownloadManager.Query().setFilterById(download));
                            if (row == null || !row.moveToFirst()) {
                                system_start_native_emit(emit, new DayFrame(DAY_GONE).done());
                                system_start_native_end(emit);
                                return;
                            }
                            int status = row.getInt(row.getColumnIndexOrThrow(DownloadManager.COLUMN_STATUS));
                            long received = row.getLong(row.getColumnIndexOrThrow(DownloadManager.COLUMN_BYTES_DOWNLOADED_SO_FAR));
                            long total = row.getLong(row.getColumnIndexOrThrow(DownloadManager.COLUMN_TOTAL_SIZE_BYTES));
                            if (received != reported || status == DownloadManager.STATUS_SUCCESSFUL) {
                                reported = received;
                                system_start_native_emit(emit, new DayFrame(DAY_PROGRESS).i64(received).i64(total).done());
                            }
                            if (status == DownloadManager.STATUS_SUCCESSFUL) {
                                String local = row.getString(row.getColumnIndexOrThrow(DownloadManager.COLUMN_LOCAL_URI));
                                String path = local == null ? null : Uri.parse(local).getPath();
                                // The Rust half moves the file before this returns.
                                system_start_native_emit(emit, new DayFrame(DAY_DONE).str(path == null ? "" : path).done());
                                system_start_native_end(emit);
                                dm.remove(download);
                                return;
                            }
                            if (status == DownloadManager.STATUS_FAILED) {
                                int reason = row.getInt(row.getColumnIndexOrThrow(DownloadManager.COLUMN_REASON));
                                boolean retry = reason == DownloadManager.ERROR_HTTP_DATA_ERROR
                                    || reason == DownloadManager.ERROR_CANNOT_RESUME
                                    || reason == 408 || reason == 429 || (reason >= 500 && reason < 600);
                                system_start_native_emit(emit, new DayFrame(DAY_FAILED)
                                    .i32(reason).i32(retry ? 1 : 0).str(dayReason(reason)).done());
                                system_start_native_end(emit);
                                dm.remove(download);
                                return;
                            }
                        } catch (RuntimeException e) {
                            system_start_native_fail(emit, String.valueOf(e.getMessage()));
                            return;
                        } finally {
                            if (row != null) {
                                row.close();
                            }
                        }
                        dayPoller().postDelayed(this, 200);
                    }
                });
            }

            public static boolean system_pause_native(String reference) {
                return false;
            }

            public static void system_cancel_native(String reference) {
                DownloadManager dm = dayManager();
                if (dm == null) {
                    return;
                }
                try {
                    dm.remove(Long.parseLong(reference));
                } catch (NumberFormatException e) {
                    // Not a DownloadManager id: nothing to remove.
                }
            }
        "#,
    );

    // HarmonyOS: the request agent's background tasks, which the system runs with its own
    // notification and continues while the app is away. The file lands in the app's cache
    // directory; the Rust half moves it, and the task is removed once the stream ends.
    #[day_bridge::impl(arkts, platforms = [ohos])]
    arkts!(
        prelude = r#"
            import { request, BusinessError } from '@kit.BasicServicesKit';
            import { common } from '@kit.AbilityKit';
            import { util } from '@kit.ArkTS';
        "#,
        body = r#"
            const DAY_STARTED: number = 1;
            const DAY_PROGRESS: number = 2;
            const DAY_DONE: number = 3;
            const DAY_FAILED: number = 4;
            const DAY_GONE: number = 5;
            const DAY_HIGH: number = 4294967296;

            class DayFrame {
              private parts: Array<Uint8Array> = new Array<Uint8Array>();
              private size: number = 0;

              constructor(tag: number) {
                this.push(new Uint8Array([tag]));
              }

              push(b: Uint8Array): DayFrame {
                this.parts.push(b);
                this.size += b.length;
                return this;
              }

              i32(n: number): DayFrame {
                const b = new Uint8Array(4);
                new DataView(b.buffer).setInt32(0, n);
                return this.push(b);
              }

              i64(n: number): DayFrame {
                const b = new Uint8Array(8);
                const v = new DataView(b.buffer);
                if (n < 0) {
                  v.setInt32(0, -1);
                  v.setInt32(4, n);
                } else {
                  v.setUint32(0, Math.floor(n / DAY_HIGH));
                  v.setUint32(4, n % DAY_HIGH);
                }
                return this.push(b);
              }

              str(s: string): DayFrame {
                const b = new util.TextEncoder().encodeInto(s);
                return this.i32(b.length).push(b);
              }

              done(): Uint8Array {
                const out = new Uint8Array(this.size);
                let at = 0;
                for (const p of this.parts) {
                  out.set(p, at);
                  at += p.length;
                }
                return out;
              }
            }

            const dayTasks: Map<string, request.agent.Task> = new Map<string, request.agent.Task>();

            function daySavedPath(ctx: common.UIAbilityContext, saveas: string): string {
              return saveas.startsWith('./') ? ctx.cacheDir + '/' + saveas.substring(2) : saveas;
            }

            function dayFinish(tid: string, emit: number, frame: DayFrame): void {
              system_start_native_emit(emit, frame.done());
              system_start_native_end(emit);
              dayTasks.delete(tid);
              request.agent.remove(tid).catch((e: BusinessError) => {});
            }

            function dayFollow(ctx: common.UIAbilityContext, task: request.agent.Task, saveas: string,
                               emit: number, fresh: boolean): void {
              dayTasks.set(task.tid, task);
              system_start_native_emit(emit, new DayFrame(DAY_STARTED).str(task.tid).done());
              let finished = false;
              task.off('progress');
              task.off('completed');
              task.off('failed');
              task.on('progress', (p: request.agent.Progress) => {
                if (!finished) {
                  const total = p.sizes.length > 0 ? p.sizes[0] : -1;
                  system_start_native_emit(emit, new DayFrame(DAY_PROGRESS).i64(p.processed).i64(total).done());
                }
              });
              task.on('completed', (p: request.agent.Progress) => {
                if (!finished) {
                  finished = true;
                  dayFinish(task.tid, emit, new DayFrame(DAY_DONE).str(daySavedPath(ctx, saveas)));
                }
              });
              task.on('failed', (p: request.agent.Progress) => {
                if (!finished) {
                  finished = true;
                  dayFinish(task.tid, emit, new DayFrame(DAY_FAILED).i32(-1).i32(1).str('the download failed'));
                }
              });
              const begun: Promise<void> = fresh ? task.start() : task.resume();
              begun.catch((e: BusinessError) => {
                // A followed task that is already running refuses to resume.
                if (fresh && !finished) {
                  finished = true;
                  dayFinish(task.tid, emit, new DayFrame(DAY_FAILED).i32(-1).i32(0).str(`${e.code}: ${e.message}`));
                }
              });
            }

            export function system_ready_native(): boolean {
              return true;
            }

            export function system_start_native(reference: string, url: string, headers: string, title: string,
                                                emit: number): void {
              const ctx = getContext() as common.UIAbilityContext;
              if (reference.length > 0) {
                request.agent.show(reference).then((info: request.agent.TaskInfo) => {
                  const saveas = info.saveas ?? '';
                  const state = info.progress.state;
                  if (state === request.agent.State.COMPLETED) {
                    system_start_native_emit(emit, new DayFrame(DAY_STARTED).str(reference).done());
                    dayFinish(reference, emit, new DayFrame(DAY_DONE).str(daySavedPath(ctx, saveas)));
                    return;
                  }
                  if (state === request.agent.State.FAILED || state === request.agent.State.STOPPED
                    || state === request.agent.State.REMOVED) {
                    dayFinish(reference, emit, new DayFrame(DAY_GONE));
                    return;
                  }
                  request.agent.getTask(ctx, reference).then((task: request.agent.Task) => {
                    dayFollow(ctx, task, saveas, emit, false);
                  }).catch((e: BusinessError) => {
                    dayFinish(reference, emit, new DayFrame(DAY_GONE));
                  });
                }).catch((e: BusinessError) => {
                  system_start_native_emit(emit, new DayFrame(DAY_GONE).done());
                  system_start_native_end(emit);
                });
                return;
              }
              const header: Record<string, string> = {};
              const lines = headers.split('\n');
              for (let i = 0; i + 1 < lines.length; i += 2) {
                header[lines[i]] = lines[i + 1];
              }
              const saveas = './day-download-' + Date.now() + '.part';
              const config: request.agent.Config = {
                action: request.agent.Action.DOWNLOAD,
                url: url,
                title: title,
                mode: request.agent.Mode.BACKGROUND,
                overwrite: true,
                headers: header,
                saveas: saveas,
                gauge: true,
                retry: true,
              };
              request.agent.create(ctx, config).then((task: request.agent.Task) => {
                dayFollow(ctx, task, saveas, emit, true);
              }).catch((e: BusinessError) => {
                system_start_native_emit(emit, new DayFrame(DAY_FAILED).i32(-1).i32(0).str(`${e.code}: ${e.message}`).done());
                system_start_native_end(emit);
              });
            }

            export function system_pause_native(reference: string): boolean {
              const task = dayTasks.get(reference);
              if (task === undefined) {
                return false;
              }
              task.pause().catch((e: BusinessError) => {});
              return true;
            }

            export function system_cancel_native(reference: string): void {
              const task = dayTasks.get(reference);
              dayTasks.delete(reference);
              if (task !== undefined) {
                task.stop().catch((e: BusinessError) => {});
              }
              request.agent.remove(reference).catch((e: BusinessError) => {});
            }
        "#,
    );

    #[day_bridge::impl(rust, platforms = [other])]
    fn system_ready_native() -> Result<bool, day_bridge::Error> {
        Ok(false)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn system_start_native(
        _reference: &str,
        _url: &str,
        _headers: &str,
        _title: &str,
        _emit: day_bridge::Emit<Vec<u8>>,
    ) -> Result<(), day_bridge::Error> {
        Err(day_bridge::Error::Unsupported)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn system_pause_native(_reference: &str) -> Result<bool, day_bridge::Error> {
        Ok(false)
    }

    #[day_bridge::impl(rust, platforms = [other])]
    fn system_cancel_native(_reference: &str) {}
}

const STARTED: u8 = 1;
const PROGRESS: u8 = 2;
const DONE: u8 = 3;
const FAILED: u8 = 4;
const GONE: u8 = 5;

/// Reads a frame's fields in order; a short frame reads as zeros.
struct Fields<'a> {
    bytes: &'a [u8],
}

impl<'a> Fields<'a> {
    fn take(&mut self, n: usize) -> &'a [u8] {
        let (head, rest) = self.bytes.split_at(n.min(self.bytes.len()));
        self.bytes = rest;
        head
    }

    fn u8(&mut self) -> u8 {
        self.take(1).first().copied().unwrap_or(0)
    }

    fn i32(&mut self) -> i32 {
        self.take(4).try_into().map(i32::from_be_bytes).unwrap_or(0)
    }

    fn i64(&mut self) -> i64 {
        self.take(8).try_into().map(i64::from_be_bytes).unwrap_or(0)
    }

    fn str(&mut self) -> String {
        let len = usize::try_from(self.i32()).unwrap_or(0);
        String::from_utf8_lossy(self.take(len)).into_owned()
    }
}

pub(crate) fn available() -> bool {
    system_ready_native().unwrap_or(false)
}

pub(crate) fn start(job: SystemJob<'_>, events: SystemEvents) -> Result<(), DownloadError> {
    let part = job.part.to_path_buf();
    let headers: String = job
        .request
        .headers()
        .iter()
        .map(|(name, value)| format!("{name}\n{value}\n"))
        .collect();
    system_start_native_stream(
        job.reference.unwrap_or(""),
        job.request.url(),
        &headers,
        job.title,
        move |item| deliver(&part, &events, item),
    )
    .map(|_| ())
    .map_err(|e| DownloadError::Io(e.to_string()))
}

fn deliver(part: &Path, events: &SystemEvents, item: Item<Vec<u8>>) {
    let frame = match item {
        Item::Value(frame) => frame,
        Item::End => return,
        Item::Failed(e) => {
            return events(SystemEvent::Failed {
                error: DownloadError::Io(e.to_string()),
                transient: true,
            });
        }
    };
    let mut f = Fields { bytes: &frame };
    match f.u8() {
        STARTED => events(SystemEvent::Started(f.str())),
        PROGRESS => {
            let received = u64::try_from(f.i64()).unwrap_or(0);
            let total = u64::try_from(f.i64()).ok().filter(|t| *t > 0);
            events(SystemEvent::Progress { received, total });
        }
        DONE => {
            let from = PathBuf::from(f.str());
            events(match crate::move_file(&from, part) {
                Ok(()) => SystemEvent::Finished,
                Err(error) => SystemEvent::Failed {
                    error,
                    transient: false,
                },
            });
        }
        FAILED => {
            let code = f.i32();
            let transient = f.i32() != 0;
            let message = f.str();
            let error = match u16::try_from(code) {
                Ok(status @ 100..=599) => DownloadError::Http(HttpError::Status(status)),
                _ => DownloadError::Io(message),
            };
            events(SystemEvent::Failed { error, transient });
        }
        GONE => events(SystemEvent::Failed {
            error: DownloadError::Io("the system no longer has this download".into()),
            transient: true,
        }),
        _ => {}
    }
}

pub(crate) fn pause(_dir: &Path, _id: u64, reference: Option<&str>) -> Result<(), DownloadError> {
    match reference.map(system_pause_native) {
        Some(Ok(true)) => Ok(()),
        _ => Err(DownloadError::Unsupported),
    }
}

pub(crate) fn cancel(_dir: &Path, _id: u64, reference: Option<&str>) {
    if let Some(reference) = reference {
        system_cancel_native(reference);
    }
}
