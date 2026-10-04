// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0
//! Desktop sharing through DataTransferManager, shared by XAML, GTK and Qt.
use std::cell::RefCell;
use windows::{
    ApplicationModel::DataTransfer::{DataRequestedEventArgs, DataTransferManager},
    Foundation::{TypedEventHandler, Uri},
    Win32::UI::{Input::KeyboardAndMouse::GetActiveWindow, Shell::IDataTransferManagerInterop},
    core::{HSTRING, Result, factory},
};
thread_local! {
    // Keep one live registration. Replace it before each presentation, so old URLs cannot
    // leak into a later share; removing the token drops the payload captured by the handler.
    static CURRENT: RefCell<Option<(DataTransferManager, i64)>> = const { RefCell::new(None) };
}
pub fn share(url: &str, title: &str) -> bool {
    fn present(url: &str, title: &str) -> Result<()> {
        let interop: IDataTransferManagerInterop =
            factory::<DataTransferManager, IDataTransferManagerInterop>()?;
        let hwnd = unsafe { GetActiveWindow() };
        if hwnd.is_invalid() {
            return Err(windows::core::Error::from_win32());
        }
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
            let args = args.as_ref().ok_or_else(windows::core::Error::from_win32)?;
            let data = args.Request()?.Data()?;
            data.Properties()?.SetTitle(&title)?;
            data.SetWebLink(&uri)?;
            Ok(())
        }))?;
        CURRENT.with(|slot| *slot.borrow_mut() = Some((manager, token)));
        unsafe { interop.ShowShareUIForWindow(hwnd) }
    }
    present(url, title).is_ok()
}
