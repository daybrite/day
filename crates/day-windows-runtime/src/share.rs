// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Desktop sharing through DataTransferManager, shared by WinUI, GTK and Qt.
use std::cell::RefCell;
use windows::{
    ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager},
    Foundation::{TypedEventHandler, Uri},
    Win32::{
        Foundation::{E_HANDLE, E_POINTER},
        UI::{Input::KeyboardAndMouse::GetActiveWindow, Shell::IDataTransferManagerInterop},
    },
    core::{HSTRING, Result, factory},
};
thread_local! {
    // Keep one live registration. Replace it before each presentation, so old URLs cannot
    // leak into a later share; removing the token drops the payload captured by the handler.
    static CURRENT: RefCell<Option<(DataTransferManager, i64)>> = const { RefCell::new(None) };
}
/// Present the Windows share UI for a URL using the active window on this thread.
///
/// Call from the toolkit's UI thread, with its COM/WinRT apartment initialized and
/// a live active window. Returns `false` if Windows cannot present the share UI.
/// A new presentation replaces the previous payload and its event registration.
pub fn share(url: &str, title: &str) -> bool {
    fn present(url: &str, title: &str) -> Result<()> {
        let interop: IDataTransferManagerInterop =
            factory::<DataTransferManager, IDataTransferManagerInterop>()?;
        // SAFETY: GetActiveWindow borrows the calling thread's active HWND; it does not own it.
        let hwnd = unsafe { GetActiveWindow() };
        if hwnd.is_invalid() {
            return Err(windows::core::Error::from_hresult(E_HANDLE));
        }
        // SAFETY: the active HWND is checked above and used synchronously on its UI thread.
        let manager: DataTransferManager = unsafe { interop.GetForWindow(hwnd)? };
        let uri = Uri::CreateUri(&HSTRING::from(url))?;
        let title = HSTRING::from(if title.is_empty() { url } else { title });
        CURRENT.with(|slot| {
            if let Some((manager, token)) = slot.borrow_mut().take() {
                let _ = manager.RemoveDataRequested(token);
            }
        });
        let token = manager.DataRequested(&TypedEventHandler::<
            DataTransferManager,
            DataRequestedEventArgs,
        >::new(move |_, args| {
            let args = args
                .as_ref()
                .ok_or_else(|| windows::core::Error::from_hresult(E_POINTER))?;
            let data = args.Request()?.Data()?;
            data.Properties()?.SetTitle(&title)?;
            data.SetWebLink(&uri)?;
            Ok(())
        }))?;
        CURRENT.with(|slot| *slot.borrow_mut() = Some((manager, token)));
        // SAFETY: the same live HWND and UI apartment used to create the manager above.
        unsafe { interop.ShowShareUIForWindow(hwnd) }
    }
    present(url, title).is_ok()
}

#[cfg(test)]
mod tests {
    #[test]
    fn sharing_without_an_active_window_fails_cleanly() {
        // Synthetic URL/title; a fresh worker thread has no active toolkit window.
        let presented = std::thread::spawn(|| super::share("https://example.invalid", "fixture"))
            .join()
            .expect("share worker must not panic");
        assert!(!presented);
    }
}
