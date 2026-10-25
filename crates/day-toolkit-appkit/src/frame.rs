// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! View-associated CADisplayLink on macOS 14+, Core Video's display link on macOS 13.
use super::*;
use day_core::frame::native;
use day_spec::{CancelFrame, FrameCallback, FrameStamp};
use objc2_quartz_core::CADisplayLink;

struct DisplaySource {
    link: Retained<CADisplayLink>,
    target: Retained<FrameTarget>,
}
impl Drop for DisplaySource {
    fn drop(&mut self) {
        unsafe { self.link.invalidate() };
    }
}
thread_local! {
    static SOURCES: RefCell<HashMap<usize, DisplaySource>> = RefCell::new(HashMap::new());
}
struct TargetState {
    key: usize,
    token: Cell<u64>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "DayAppKitFrameTarget"]
    #[ivars = TargetState]
    struct FrameTarget;
    unsafe impl NSObjectProtocol for FrameTarget {}
    impl FrameTarget {
        #[unsafe(method(step:))]
        fn step(&self, link: &CADisplayLink) {
            ffi_guard::contain((), || {
                unsafe { link.setPaused(true) };
                let token = self.ivars().token.replace(0);
                native::deliver(token, FrameStamp {
                    timestamp: unsafe { link.timestamp() },
                    target_timestamp: Some(unsafe { link.targetTimestamp() }),
                });
                // A continuing consumer has re-armed this same link during delivery.
                if self.ivars().token.get() == 0 {
                    SOURCES.with(|s| s.borrow_mut().remove(&self.ivars().key));
                }
            });
        }
    }
);
pub(super) fn request(host: &NSView, cb: FrameCallback) -> CancelFrame {
    // Avoid even referencing the new NSView method on macOS 13. That supported OS has a real
    // Core Video vsync source too, so it never needs a synthetic 16ms timer.
    if !unsafe { host.respondsToSelector(sel!(displayLinkWithTarget:selector:)) } {
        return legacy::request(host, cb);
    }
    let key = ptr_of(host);
    let token = native::register(cb);
    SOURCES.with(|s| {
        let mut sources = s.borrow_mut();
        let source = sources.entry(key).or_insert_with(|| {
            let this =
                FrameTarget::alloc(MainThreadMarker::new().unwrap()).set_ivars(TargetState {
                    key,
                    token: Cell::new(0),
                });
            let target: Retained<FrameTarget> = unsafe { msg_send![super(this), init] };
            let link: Retained<CADisplayLink> =
                unsafe { msg_send![host, displayLinkWithTarget: &*target, selector: sel!(step:)] };
            unsafe {
                link.addToRunLoop_forMode(
                    &objc2_foundation::NSRunLoop::mainRunLoop(),
                    objc2_foundation::NSRunLoopCommonModes,
                );
            }
            DisplaySource { link, target }
        });
        source.target.ivars().token.set(token);
        unsafe { source.link.setPaused(false) };
    });
    Box::new(move || {
        native::cancel(token);
        SOURCES.with(|s| {
            let mut s = s.borrow_mut();
            if s.get(&key)
                .is_some_and(|s| s.target.ivars().token.get() == token)
            {
                s.remove(&key);
            }
        });
    })
}

mod legacy {
    use super::*;
    use std::ffi::c_void;
    use std::sync::atomic::{AtomicBool, Ordering};
    type Link = *mut c_void;
    type Callback =
        unsafe extern "C" fn(Link, *const c_void, *const c_void, u64, *mut u64, *mut c_void) -> i32;
    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        fn CVDisplayLinkCreateWithCGDisplay(display: u32, link: *mut Link) -> i32;
        fn CVDisplayLinkSetOutputCallback(link: Link, cb: Callback, context: *mut c_void) -> i32;
        fn CVDisplayLinkStart(link: Link) -> i32;
        fn CVDisplayLinkStop(link: Link) -> i32;
        fn CVDisplayLinkRelease(link: Link);
    }
    struct Context {
        token: u64,
        sent: AtomicBool,
    }
    struct Guard {
        link: Link,
        context: Box<Context>,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            // Stop joins outstanding callbacks before the context is released.
            unsafe {
                CVDisplayLinkStop(self.link);
                CVDisplayLinkRelease(self.link);
            }
            native::cancel(self.context.token);
        }
    }
    unsafe extern "C" fn fire(
        _: Link,
        _: *const c_void,
        _: *const c_void,
        _: u64,
        _: *mut u64,
        context: *mut c_void,
    ) -> i32 {
        let ctx = unsafe { &*context.cast::<Context>() };
        if !ctx.sent.swap(true, Ordering::AcqRel) {
            let token = ctx.token;
            let stamp = FrameStamp::new(day_spec::frame_timestamp());
            dispatch2::DispatchQueue::main().exec_async(move || native::deliver(token, stamp));
        }
        0
    }
    pub(super) fn request(host: &NSView, cb: FrameCallback) -> CancelFrame {
        let screen = host
            .window()
            .and_then(|w| w.screen())
            .or_else(|| objc2_app_kit::NSScreen::mainScreen(MainThreadMarker::new().unwrap()));
        let display = screen
            .and_then(|s| {
                let value = unsafe {
                    s.deviceDescription()
                        .objectForKey(&NSString::from_str("NSScreenNumber"))
                }?;
                Some(unsafe { msg_send![&*value, unsignedIntValue] })
            })
            .unwrap_or(0);
        let token = native::register(cb);
        let mut link = std::ptr::null_mut();
        if unsafe { CVDisplayLinkCreateWithCGDisplay(display, &mut link) } != 0 {
            native::cancel(token);
            log::error!("could not create Core Video display link");
            return Box::new(|| {});
        }
        let mut guard = Guard {
            link,
            context: Box::new(Context {
                token,
                sent: AtomicBool::new(false),
            }),
        };
        let context = (&mut *guard.context as *mut Context).cast();
        if unsafe { CVDisplayLinkSetOutputCallback(link, fire, context) } != 0
            || unsafe { CVDisplayLinkStart(link) } != 0
        {
            log::error!("could not start Core Video display link");
        }
        Box::new(move || drop(guard))
    }
}
