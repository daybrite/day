// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

//! AppKit window grouping for toolkits hosted by NSWindow. The delegate proxy forwards
//! all toolkit-owned behavior; Day handles only document add/close requests.
#![cfg(target_os = "macos")]

use day_spec::NodeId;
use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject, Sel};
use objc2::{DefinedClass, MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSView, NSWindow, NSWindowController, NSWindowDelegate, NSWindowOrderingMode,
    NSWindowTabbingMode,
};
use objc2_foundation::{NSNotification, NSString};
use std::{cell::RefCell, ffi::c_void};

struct DelegateState {
    original: Option<Retained<AnyObject>>,
    id: NodeId,
    group: String,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[ivars = DelegateState]
    struct DayDocumentWindowDelegate;
    unsafe impl NSObjectProtocol for DayDocumentWindowDelegate {
        #[unsafe(method(respondsToSelector:))]
        fn responds(&self, selector: Sel) -> bool {
            // SAFETY: NSObject implements this selector with the declared argument/result types.
            let own: bool = unsafe { msg_send![super(self), respondsToSelector: selector] };
            own || self.ivars().original.as_ref().is_some_and(|original| {
                // SAFETY: the retained original delegate is an NSObject.
                unsafe { msg_send![&**original, respondsToSelector: selector] }
            })
        }
    }
    impl DayDocumentWindowDelegate {
        #[unsafe(method_id(forwardingTargetForSelector:))]
        fn forwarding(&self, _selector: Sel) -> Option<Retained<AnyObject>> {
            self.ivars().original.clone()
        }
        #[unsafe(method(newWindowForTab:))]
        fn new_tab(&self, _sender: Option<&AnyObject>) {
            day_spec::ffi_guard::contain((), || {
                day_core::windows::open_new_window_for_group(&self.ivars().group);
            });
        }
    }
    unsafe impl NSWindowDelegate for DayDocumentWindowDelegate {
        #[unsafe(method(windowShouldClose:))]
        fn should_close(&self, window: &NSWindow) -> bool {
            day_spec::ffi_guard::contain(false, || {
                if !day_core::windows::native_close_requested(self.ivars().id) {
                    return false;
                }
                if let Some(original) = &self.ivars().original {
                    // SAFETY: NSObject's query is valid for the retained toolkit delegate.
                    let responds: bool = unsafe {
                        msg_send![&**original, respondsToSelector: sel!(windowShouldClose:)]
                    };
                    if responds {
                        // SAFETY: the selector was checked; NSWindowDelegate specifies this ABI.
                        return unsafe { msg_send![&**original, windowShouldClose: window] };
                    }
                }
                true
            })
        }
        #[unsafe(method(windowWillClose:))]
        fn will_close(&self, notification: &NSNotification) {
            day_spec::ffi_guard::contain((), || {
                if let Some(original) = &self.ivars().original {
                    // SAFETY: NSObject's query is valid for the retained toolkit delegate.
                    let responds: bool = unsafe {
                        msg_send![&**original, respondsToSelector: sel!(windowWillClose:)]
                    };
                    if responds {
                        // SAFETY: NSWindowDelegate defines this checked selector's notification ABI.
                        unsafe { let _: () = msg_send![&**original, windowWillClose: notification]; }
                    }
                }
                let id = self.ivars().id;
                let delegate = self as *const Self as usize;
                day_reactive::on_main(move || {
                    let current = WINDOWS.with(|w| w.borrow().iter().any(|e| {
                        e.id == id && Retained::as_ptr(&e.delegate) as usize == delegate
                    }));
                    if current {
                        unregister(id);
                    }
                });
            });
        }
    }
);
define_class!(
    #[unsafe(super(NSWindowController))]
    #[thread_kind = MainThreadOnly]
    #[ivars = String]
    struct DayDocumentWindowController;
    unsafe impl NSObjectProtocol for DayDocumentWindowController {}
    impl DayDocumentWindowController {
        #[unsafe(method(newWindowForTab:))]
        fn new_tab(&self, _sender: Option<&AnyObject>) {
            day_spec::ffi_guard::contain((), || {
                day_core::windows::open_new_window_for_group(self.ivars());
            });
        }
    }
);
struct Entry {
    id: NodeId,
    window: Retained<NSWindow>,
    delegate: Retained<DayDocumentWindowDelegate>,
    controller: Option<Retained<DayDocumentWindowController>>,
}
thread_local! { static WINDOWS: RefCell<Vec<Entry>> = const { RefCell::new(Vec::new()) }; }

/// Register a toolkit's NSWindow after realization.
/// # Safety
/// `raw` must be a live NSWindow on the main thread. The original delegate remains retained.
pub unsafe fn register(raw: *mut c_void, id: NodeId, group: &str) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    // SAFETY: the caller supplies a live NSWindow; retain accepts null as no window.
    let Some(window) = (unsafe { Retained::retain(raw.cast::<NSWindow>()) }) else {
        return;
    };
    if WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .any(|e| e.id == id && std::ptr::eq(&*e.window, &*window))
    }) {
        return;
    }
    unregister(id);
    // SAFETY: erasing an Objective-C protocol object to AnyObject preserves its identity.
    let original = window
        .delegate()
        .map(|d| unsafe { Retained::cast_unchecked(d) });
    let allocated = DayDocumentWindowDelegate::alloc(mtm).set_ivars(DelegateState {
        original,
        id,
        group: group.into(),
    });
    // SAFETY: NSObject's init initializes the allocated subclass with its installed ivars.
    let delegate: Retained<DayDocumentWindowDelegate> =
        unsafe { msg_send![super(allocated), init] };
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    let controller = if window.windowController().is_none() {
        let allocated = DayDocumentWindowController::alloc(mtm).set_ivars(group.to_owned());
        // SAFETY: the allocated NSWindowController subclass receives a live main-thread window.
        let controller: Retained<DayDocumentWindowController> =
            unsafe { msg_send![super(allocated), initWithWindow: &*window] };
        window.setWindowController(Some(&controller));
        Some(controller)
    } else {
        None
    };
    window.setTabbingMode(NSWindowTabbingMode::Preferred);
    window.setTabbingIdentifier(&NSString::from_str(group));
    NSWindow::setAllowsAutomaticWindowTabbing(true, mtm);
    WINDOWS.with(|w| {
        w.borrow_mut().push(Entry {
            id,
            window,
            delegate,
            controller,
        })
    });
}
/// # Safety
/// `view` must be a live NSView on the main thread.
pub unsafe fn window_from_view(view: *mut c_void) -> *mut c_void {
    // SAFETY: the caller guarantees a live NSView; as_ref handles null.
    let Some(view) = (unsafe { view.cast::<NSView>().as_ref() }) else {
        return std::ptr::null_mut();
    };
    view.window()
        .map(|w| Retained::as_ptr(&w).cast_mut().cast())
        .unwrap_or(std::ptr::null_mut())
}
/// Restore the toolkit delegate before releasing Day's ownership.
pub fn unregister(id: NodeId) {
    let entry = WINDOWS.with(|w| {
        let mut w = w.borrow_mut();
        w.iter().position(|e| e.id == id).map(|i| w.remove(i))
    });
    if let Some(entry) = entry {
        if entry.controller.is_some() {
            entry.window.setWindowController(None);
        }
        let original = entry.delegate.ivars().original.as_deref();
        // SAFETY: original is the retained NSWindowDelegate saved at registration, or nil.
        unsafe {
            let _: () = msg_send![&*entry.window, setDelegate: original];
        }
    }
}
/// Explicitly join and reorder document windows; native contents are untouched.
pub fn group(ids: &[NodeId]) {
    let windows = WINDOWS.with(|w| {
        ids.iter()
            .filter_map(|id| {
                w.borrow()
                    .iter()
                    .find(|e| e.id == *id)
                    .map(|e| e.window.clone())
            })
            .collect::<Vec<_>>()
    });
    let Some(first) = windows.first() else {
        return;
    };
    for window in windows.iter().skip(1) {
        first.addTabbedWindow_ordered(window, NSWindowOrderingMode::Above);
    }
    if first
        .tabGroup()
        .is_none_or(|group| !group.isTabBarVisible())
    {
        first.toggleTabBar(None);
    }
    if let Some(group) = first.tabGroup() {
        for (i, window) in windows.iter().enumerate() {
            group.insertWindow_atIndex(window, i as isize);
        }
        if !group.isTabBarVisible() {
            first.toggleTabBar(None);
        }
    }
}
/// Return known window IDs in native tab-group order.
pub fn order(id: NodeId) -> Vec<NodeId> {
    let window = WINDOWS.with(|w| {
        w.borrow()
            .iter()
            .find(|e| e.id == id)
            .map(|e| e.window.clone())
    });
    let Some(group) = window.and_then(|w| w.tabGroup()) else {
        return Vec::new();
    };
    WINDOWS.with(|w| {
        group
            .windows()
            .iter()
            .filter_map(|window| {
                w.borrow()
                    .iter()
                    .find(|e| std::ptr::eq(&*e.window, &*window))
                    .map(|e| e.id)
            })
            .collect()
    })
}
