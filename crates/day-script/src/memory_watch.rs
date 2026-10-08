// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use crate::Reply;
use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

pub(crate) fn valid_interval(ms: u64) -> bool {
    ms == 0 || (100..=60_000).contains(&ms)
}

pub(crate) fn sample() -> Reply {
    Reply {
        memory_sample: Some(Box::new(crate::memory::reading())),
        ..Reply::ok()
    }
}

pub(crate) struct Watch {
    stop: mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Watch {
    pub(crate) fn start(writer: Arc<Mutex<TcpStream>>, ms: u64) -> Self {
        let (stop, stopped) = mpsc::channel();
        // Initial sample precedes the control acknowledgment, even for a very short script.
        write_sample(&writer);
        let thread = std::thread::spawn(move || {
            loop {
                let done = stopped.recv_timeout(Duration::from_millis(ms)).is_ok();
                if !write_sample(&writer) || done {
                    break;
                }
            }
        });
        Self {
            stop,
            thread: Some(thread),
        }
    }
}

fn write_sample(writer: &Mutex<TcpStream>) -> bool {
    let mut line = serde_json::to_string(&sample()).unwrap();
    line.push('\n');
    writer.lock().unwrap().write_all(line.as_bytes()).is_ok()
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    use day_script_proto::{Request, Step};
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;

    fn control(reader: &mut BufReader<TcpStream>, token: &str, interval_ms: u64) {
        let request = Request {
            token: token.into(),
            step: Step::MemoryWatch { interval_ms },
        };
        writeln!(
            reader.get_mut(),
            "{}",
            serde_json::to_string(&request).unwrap()
        )
        .unwrap();
    }
    fn read(reader: &mut BufReader<TcpStream>) -> Reply {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn authenticated_stream_samples_without_ui_steps_and_stops_before_acknowledging() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            crate::handle_conn(stream, "fixture");
        });
        let mut reader = BufReader::new(client);
        control(&mut reader, "wrong", 100);
        assert!(!read(&mut reader).ok);
        control(&mut reader, "fixture", 1);
        assert!(!read(&mut reader).ok);
        control(&mut reader, "fixture", 100);
        assert!(read(&mut reader).memory_sample.is_some()); // Initial sample before ack.
        assert!(read(&mut reader).ok);
        assert!(read(&mut reader).memory_sample.is_some()); // No UI request/main loop.
        control(&mut reader, "fixture", 0);
        let mut final_samples = 0;
        loop {
            let reply = read(&mut reader);
            if reply.memory_sample.is_some() {
                final_samples += 1;
            } else {
                assert!(reply.ok);
                break;
            }
        }
        assert!(final_samples >= 1);
        reader
            .get_mut()
            .set_read_timeout(Some(Duration::from_millis(200)))
            .unwrap();
        let mut line = String::new();
        assert!(
            reader.read_line(&mut line).is_err(),
            "sampling continued after stop"
        );
        drop(reader);
        server.join().unwrap();
    }
}
