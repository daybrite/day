// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Finder/Open With document delivery, including events received before the root mounts.
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
                        day_core::request_route(&day_spec::route_of_url(&s.to_string()));
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
    }
);
pub(super) fn delegate(mtm: MainThreadMarker) -> Retained<DocumentDelegate> {
    unsafe { msg_send![DocumentDelegate::alloc(mtm), init] }
}
