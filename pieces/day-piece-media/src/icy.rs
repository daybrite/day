// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! The side probe of a stream's in-band metadata (docs/media.md), for the arms whose native
//! player keeps it to itself (Android's `MediaPlayer`, GStreamer behind `GtkVideo`, Qt, XAML,
//! ArkUI). Icecast and Shoutcast servers interleave a `StreamTitle='…';` block into an MP3 or
//! AAC stream every `icy-metaint` bytes when the client asks with `Icy-MetaData: 1` — which
//! every player does, and then decodes and discards. This probe asks the same way on a second,
//! short-lived connection, reads exactly one block (a few kilobytes), hangs up, and comes back
//! every [`ICY_PROBE_SECS`] seconds. A stream that answers without `icy-metaint` has no such
//! metadata, and the probe stops asking it.
//!
//! Apple's AVFoundation surfaces the same block as timed metadata, so the Apple arms report
//! from the player and this probe stays off there (lib.rs decides).

use super::{ICY_PROBE_SECS, StreamMetadata};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use day_reactive::Signal;

/// One player's probe: at most one worker at a time, restarted per source.
pub struct Probe {
    id: u64,
    cancel: RefCell<Option<Arc<AtomicBool>>>,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// Where a worker's finding lands: the signal of the probe that started it, looked up on
    /// the UI thread by id (a `Signal` never crosses a thread; an id does).
    static SINKS: RefCell<HashMap<u64, Signal<Option<StreamMetadata>>>> = RefCell::new(HashMap::new());
}

impl Probe {
    pub fn new(sink: Signal<Option<StreamMetadata>>) -> Rc<Probe> {
        let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
        SINKS.with(|s| s.borrow_mut().insert(id, sink));
        Rc::new(Probe {
            id,
            cancel: RefCell::new(None),
        })
    }

    /// Probe `url` from now on (replacing any earlier source), and clear what the last one
    /// said: a new station has not said anything yet.
    pub fn start(&self, url: String) {
        self.stop();
        SINKS.with(|s| {
            if let Some(sink) = s.borrow().get(&self.id)
                && sink.get_untracked().is_some()
            {
                sink.set(None);
            }
        });
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self.cancel.borrow_mut() = Some(cancel.clone());
        let id = self.id;
        log::info!("day-piece-media: ICY probe starting for {url}");
        #[cfg(not(target_arch = "wasm32"))]
        std::thread::Builder::new()
            .name("day-media-icy".into())
            .spawn(move || worker(id, url, cancel))
            .map(|_| ())
            .unwrap_or_else(|e| log::warn!("day-piece-media: no thread for the ICY probe: {e}"));
        #[cfg(target_arch = "wasm32")]
        let _ = (id, cancel);
    }

    pub fn stop(&self) {
        if let Some(c) = self.cancel.borrow_mut().take() {
            c.store(true, Ordering::Relaxed);
        }
    }
}

impl Drop for Probe {
    fn drop(&mut self) {
        self.stop();
        SINKS.with(|s| {
            s.borrow_mut().remove(&self.id);
        });
    }
}

/// The worker: probe, deliver, sleep, until cancelled or the stream proves silent.
#[cfg(not(target_arch = "wasm32"))]
fn worker(id: u64, url: String, cancel: Arc<AtomicBool>) {
    let mut last: Option<String> = None;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        log::debug!("day-piece-media: ICY probe of {url}");
        match probe_once(&url, &cancel) {
            Outcome::Title(title) => {
                log::debug!("day-piece-media: ICY StreamTitle {title:?}");
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                if last.as_deref() != Some(title.as_str()) {
                    last = Some(title.clone());
                    day_reactive::on_main(move || deliver(id, &title));
                }
            }
            Outcome::NoMetadata => {
                log::debug!("day-piece-media: {url} carries no ICY metadata");
                return;
            }
            Outcome::Failed(e) => log::debug!("day-piece-media: ICY probe of {url}: {e}"),
        }
        // Sleep in slices so a stop lands within a second.
        for _ in 0..(ICY_PROBE_SECS * 4) {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
}

/// On the UI thread: the probe's signal, if it still exists.
fn deliver(id: u64, title: &str) {
    let parsed = StreamMetadata::from_report(title);
    let next = if parsed.is_empty() {
        None
    } else {
        Some(parsed)
    };
    SINKS.with(|s| {
        if let Some(sink) = s.borrow().get(&id)
            && sink.get_untracked() != next
        {
            sink.set(next);
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
enum Outcome {
    Title(String),
    NoMetadata,
    Failed(String),
}

/// One request: the response headers say the interval, the body yields one block.
#[cfg(not(target_arch = "wasm32"))]
fn probe_once(url: &str, cancel: &Arc<AtomicBool>) -> Outcome {
    use day_part_http::{HttpError, Request, StreamSink, fetch_streamed};

    /// The largest interval worth reading through: past this a stream is video or odd.
    const MAX_INTERVAL: usize = 1 << 20;

    struct Sink<'a> {
        interval: Option<usize>,
        buffer: Vec<u8>,
        title: Option<String>,
        cancel: &'a Arc<AtomicBool>,
    }
    impl StreamSink for Sink<'_> {
        fn head(&mut self, status: u16, headers: &[(String, String)]) -> bool {
            if status != 200 {
                return false;
            }
            self.interval = headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("icy-metaint"))
                .and_then(|(_, v)| v.trim().parse::<usize>().ok())
                .filter(|n| *n > 0 && *n <= MAX_INTERVAL);
            log::debug!(
                "day-piece-media: ICY head {status}, interval {:?}, {} header(s)",
                self.interval,
                headers.len()
            );
            // No interval: the server does not do ICY. Hang up rather than read the stream.
            self.interval.is_some()
        }
        fn chunk(&mut self, data: &[u8]) -> Result<(), HttpError> {
            if self.cancel.load(Ordering::Relaxed) {
                return Err(HttpError::Io("cancelled".into()));
            }
            let interval = self.interval.unwrap_or(0);
            self.buffer.extend_from_slice(data);
            // The block: one length byte (×16) after `interval` audio bytes, then the text.
            if self.buffer.len() > interval {
                let len = self.buffer[interval] as usize * 16;
                if self.buffer.len() >= interval + 1 + len {
                    let block = &self.buffer[interval + 1..interval + 1 + len];
                    self.title = Some(stream_title(block));
                    return Err(HttpError::Io("done".into()));
                }
            }
            Ok(())
        }
    }

    let req = Request::get(url)
        .header("Icy-MetaData", "1")
        .header("Accept", "*/*")
        .timeout(std::time::Duration::from_secs(20));
    let mut sink = Sink {
        interval: None,
        buffer: Vec::new(),
        title: None,
        cancel,
    };
    let result = fetch_streamed(&req, &mut sink);
    if let Some(title) = sink.title {
        return Outcome::Title(title);
    }
    match result {
        Ok(_) if sink.interval.is_none() => Outcome::NoMetadata,
        Ok(_) => Outcome::Failed("the stream ended before its first metadata block".into()),
        Err(HttpError::Io(m)) if m == "aborted" && sink.interval.is_none() => Outcome::NoMetadata,
        Err(e) => Outcome::Failed(format!("{e:?}")),
    }
}

/// `StreamTitle='…';StreamUrl='…';` (NUL-padded) → the title, or empty when there is none.
pub fn stream_title(block: &[u8]) -> String {
    let text = String::from_utf8_lossy(block);
    let text = text.trim_end_matches('\0');
    let Some(start) = text.find("StreamTitle='") else {
        return String::new();
    };
    let rest = &text[start + "StreamTitle='".len()..];
    // The title may itself contain a quote; the field ends at the LAST `';` before the next
    // field, or at the end of the block.
    let end = rest
        .find("';StreamUrl=")
        .or_else(|| rest.find("';StreamNext="))
        .or_else(|| rest.rfind("';"))
        .unwrap_or(rest.len());
    rest[..end].trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Against a live Icecast server; run by hand: `cargo test -p day-piece-media -- --ignored
    /// --nocapture probe_live`.
    #[test]
    #[ignore]
    #[cfg(not(target_arch = "wasm32"))]
    fn probe_live() {
        let cancel = Arc::new(AtomicBool::new(false));
        match probe_once("https://ice5.somafm.com/groovesalad-128-mp3", &cancel) {
            Outcome::Title(t) => println!("title: {t}"),
            Outcome::NoMetadata => println!("no metadata"),
            Outcome::Failed(e) => println!("failed: {e}"),
        }
    }

    #[test]
    fn stream_titles_parse() {
        assert_eq!(
            stream_title(b"StreamTitle='Miles Davis - So What';StreamUrl='';\0\0\0\0"),
            "Miles Davis - So What"
        );
        assert_eq!(
            stream_title(b"StreamTitle='Rock 'n' Roll - Someone';\0"),
            "Rock 'n' Roll - Someone"
        );
        assert_eq!(stream_title(b"StreamTitle='';StreamUrl='';"), "");
        assert_eq!(stream_title(b"\0\0"), "");
    }
}
