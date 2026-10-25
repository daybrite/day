// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! The app delegate: Finder/Open With document delivery (including events received before the
//! root mounts) and the Dock menu.
use super::*;
use objc2_app_kit::NSApplicationDelegate;
use objc2_foundation::NSURL;
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    pub(super) struct DocumentDelegate;
    unsafe impl NSObjectProtocol for DocumentDelegate {}
    unsafe impl NSApplicationDelegate for DocumentDelegate {
        #[unsafe(method(application:openURLs:))]
        fn open_urls(&self, _app: &NSApplication, urls: &NSArray<NSURL>) {
            ffi_guard::contain((), || {
                for url in urls.iter().filter(|u| !u.isFileURL()) {
                    if let Some(s) = url.absoluteString() {
                        day_core::request_open_url(&s.to_string());
                    }
                }
                let files = urls
                    .iter()
                    .filter(|u| u.isFileURL())
                    .filter_map(|u| u.absoluteString().map(|s| s.to_string()))
                    .collect();
                day_core::request_open_files(files);
            });
        }

        /// A click on the Dock icon with no window open (a status-item app, `KeepRunning::
        /// Always`): open one through the app's New Window builder, as every Mac app does.
        #[unsafe(method(applicationShouldHandleReopen:hasVisibleWindows:))]
        fn should_handle_reopen(&self, _app: &NSApplication, visible: bool) -> bool {
            ffi_guard::contain(true, || {
                if !visible {
                    let _ = day_core::windows::open_new_window();
                }
                true
            })
        }

        #[unsafe(method_id(applicationDockMenu:))]
        fn application_dock_menu(&self, _app: &NSApplication) -> Option<Retained<NSMenu>> {
            ffi_guard::contain(None, super::dock::menu)
        }
    }
);
pub(super) fn delegate(mtm: MainThreadMarker) -> Retained<DocumentDelegate> {
    unsafe { msg_send![DocumentDelegate::alloc(mtm), init] }
}
