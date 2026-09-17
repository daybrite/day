// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

use crate::{ClipboardFuture, Content, Error, Representation};
use std::{
    cell::RefCell,
    collections::HashMap,
    rc::Rc,
    task::{Poll, Waker},
};
struct Pending {
    value: Option<Result<Vec<u8>, Error>>,
    waker: Option<Waker>,
}
type Requests = (u32, HashMap<u32, Rc<RefCell<Pending>>>);
thread_local! {
    static REQUESTS: RefCell<Requests> = RefCell::new((0, HashMap::new()));
}
#[link(wasm_import_module = "env")]
unsafe extern "C" {
    fn day_dom_clipboard_read_bytes(req: u32, p: *const u8, len: usize);
    fn day_dom_clipboard_write_bytes(req: u32, p: *const u8, len: usize);
}
struct Request(u32);
impl Drop for Request {
    fn drop(&mut self) {
        REQUESTS.with(|r| {
            r.borrow_mut().1.remove(&self.0);
        });
    }
}
fn request(start: impl FnOnce(u32)) -> ClipboardFuture<Vec<u8>> {
    let pending = Rc::new(RefCell::new(Pending {
        value: None,
        waker: None,
    }));
    let id = REQUESTS.with(|r| {
        let mut r = r.borrow_mut();
        r.0 += 1;
        let id = r.0;
        r.1.insert(id, pending.clone());
        id
    });
    start(id); // Capture event data/activation before returning to the caller.
    let guard = Request(id);
    Box::pin(async move {
        let _guard = guard;
        std::future::poll_fn(move |cx| {
            let mut p = pending.borrow_mut();
            if let Some(v) = p.value.take() {
                Poll::Ready(v)
            } else {
                p.waker = Some(cx.waker().clone());
                Poll::Pending
            }
        })
        .await
    })
}
#[unsafe(no_mangle)]
/// Complete a request with an owned buffer from the matching Day web shim.
///
/// # Safety
/// For nonzero `len`, `p` must be an allocation from `day_dom_alloc(len)` which
/// has not been freed or previously transferred. This function takes ownership.
pub unsafe extern "C" fn day_clipboard_result(req: u32, status: u32, p: *mut u8, len: usize) {
    // JS allocated with day_dom_alloc; ownership transfers even for a canceled request.
    let bytes = if len == 0 {
        Vec::new()
    } else {
        unsafe { Vec::from_raw_parts(p, len, len) }
    };
    let pending = REQUESTS.with(|r| r.borrow_mut().1.remove(&req));
    if let Some(pending) = pending {
        let waker = {
            let mut pending = pending.borrow_mut();
            pending.value = Some(match status {
                0 => Ok(bytes),
                2 => Err(Error::Unsupported),
                3 => Err(Error::TooLarge),
                _ => Err(Error::Unavailable),
            });
            pending.waker.take()
        };
        if let Some(w) = waker {
            w.wake();
        }
    }
}
pub fn read(preferred: &[&str]) -> ClipboardFuture<Option<Representation>> {
    let preferred = preferred.join("\n");
    let future = request(|req| unsafe {
        day_dom_clipboard_read_bytes(req, preferred.as_ptr(), preferred.len())
    });
    Box::pin(async move { Ok(crate::content::unpack(&future.await?)?.0.into_iter().next()) })
}
pub fn write(content: Content) -> ClipboardFuture<Vec<String>> {
    let data = crate::content::pack(&content);
    let future =
        request(|req| unsafe { day_dom_clipboard_write_bytes(req, data.as_ptr(), data.len()) });
    Box::pin(async move {
        Ok(String::from_utf8(future.await?)
            .map_err(|_| Error::InvalidData)?
            .split('\n')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect())
    })
}
