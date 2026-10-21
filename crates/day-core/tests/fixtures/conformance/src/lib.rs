// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#[cfg(feature = "provider")]
use registry_widgets as _;

#[day_macros::test(day_core)]
fn app_test() -> day_core::conformance::Case {
    day_core::conformance::Case::new()
}

#[day_macros::test(day_core)]
#[cfg(any())]
fn excluded_after_attribute() -> day_core::conformance::Case {
    compile_error!("cfg must exclude both the function and its registration");
}

#[cfg(any())]
#[day_macros::test(day_core)]
fn excluded_before_attribute() -> day_core::conformance::Case {
    compile_error!("cfg must exclude both the function and its registration");
}

// Fault injection: rustc declares a test export but its inventory constructor is absent.
// It must fail even when other tests survive and a caller requests a different test.
#[cfg(feature = "missing-registration")]
#[unsafe(export_name = "__day_test_v1__day-conformance-registry-fixture@0.0.0::fixture::missing")]
pub extern "C" fn missing_registration() {}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use std::cell::RefCell;
    thread_local! { static ERROR: RefCell<String> = const { RefCell::new(String::new()) }; }

    #[unsafe(no_mangle)]
    pub extern "C" fn allocate(len: usize) -> *mut u8 {
        let mut bytes = Vec::<u8>::with_capacity(len);
        let ptr = bytes.as_mut_ptr();
        std::mem::forget(bytes);
        ptr
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn initialize(ptr: *mut u8, len: usize) -> i32 {
        // SAFETY: test driver allocated and initialized exactly len bytes.
        let bytes = unsafe { Vec::from_raw_parts(ptr, len, len) };
        record(day_core::conformance::install_web_registry(&bytes))
    }

    fn record(result: Result<(), String>) -> i32 {
        match result {
            Ok(()) => {
                ERROR.with(|s| s.borrow_mut().clear());
                1
            }
            Err(e) => {
                ERROR.with(|s| *s.borrow_mut() = e);
                0
            }
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn validate() -> i32 {
        record(day_core::conformance::validate_registry())
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn error_ptr() -> *const u8 {
        ERROR.with(|s| s.borrow().as_ptr())
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn error_len() -> usize {
        ERROR.with(|s| s.borrow().len())
    }
    #[unsafe(no_mangle)]
    pub extern "C" fn count() -> usize {
        day_core::conformance::tests().len()
    }
}

// Exercise the real dayscript gate, including a filter that would otherwise hide the loss.
#[cfg(all(target_arch = "wasm32", feature = "missing-registration"))]
#[unsafe(no_mangle)]
pub extern "C" fn rejected_filtered_run() -> i32 {
    let reply = day_script::conformance::run_tests(
        &["app-test".to_owned()],
        day_script::conformance::ShotPolicy::Never,
        None,
    );
    i32::from(
        matches!(reply, Err(ref r) if !r.retryable && r.error.as_ref().is_some_and(|e| e.contains("fixture::missing")))
            && day_script::conformance::listing().is_err(),
    )
}
