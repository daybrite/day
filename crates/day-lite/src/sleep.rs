//! A minimal timer future for `setTimeout` (docs/lite.md §7): a helper thread parks for
//! the duration and wakes the waker; `day_core::task` then resumes the future on the main
//! thread, so the JS callback always runs there.

use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

struct Shared {
    done: bool,
    waker: Option<Waker>,
}

pub struct Sleep {
    shared: Arc<Mutex<Shared>>,
    started: bool,
    ms: u64,
}

pub fn sleep_ms(ms: u64) -> Sleep {
    Sleep {
        shared: Arc::new(Mutex::new(Shared {
            done: false,
            waker: None,
        })),
        started: false,
        ms,
    }
}

impl Future for Sleep {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let mut s = self.shared.lock().expect("sleep mutex");
        if s.done {
            return Poll::Ready(());
        }
        s.waker = Some(cx.waker().clone());
        drop(s);
        if !self.started {
            self.started = true;
            let shared = self.shared.clone();
            let ms = self.ms;
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(ms));
                let mut s = shared.lock().expect("sleep mutex");
                s.done = true;
                if let Some(w) = s.waker.take() {
                    w.wake();
                }
            });
        }
        Poll::Pending
    }
}
