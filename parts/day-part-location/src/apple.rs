//! iOS and macOS: CoreLocation's `CLLocationManager` with a delegate.
//!
//! Unlike an authorization READ — which `day-part-permissions` polls, because a delegate needs a
//! run loop it cannot assume — position updates have no polling equivalent worth the trade: the
//! `location` property only refreshes while updates are running anyway, so this arm installs a real
//! delegate with `objc2::define_class!`. It is the first part in the tree to define an Objective-C
//! class; pieces do it routinely.
//!
//! # The run-loop requirement, stated plainly
//!
//! CoreLocation delivers to the run loop of the thread the manager was created on. `start` therefore
//! creates it on the MAIN thread when called from there — which is where a Day app calls it — and
//! best-effort elsewhere. In a plain `main` or under `cargo test` with no run loop, no fix is ever
//! delivered; `is_available` still answers true, because CoreLocation exists.
//!
//! Raw `objc2` + `msg_send!` throughout: this needs `CLLocationManager`, `CLLocation` and one
//! delegate, which is not worth a framework wrapper crate (see the crate's dependency note).

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyClass, AnyObject};
use objc2::{AllocAnyThread, Encode, Encoding, RefEncode, define_class, msg_send};
use objc2_foundation::NSObject;

use crate::{Accuracy, Fix, LocationError};

#[link(name = "CoreLocation", kind = "framework")]
unsafe extern "C" {}

/// `CLLocationCoordinate2D` — two doubles, returned by value from `-[CLLocation coordinate]`.
#[repr(C)]
#[derive(Clone, Copy)]
struct Coordinate {
    latitude: f64,
    longitude: f64,
}

// SAFETY: the layout and encoding match CoreLocation's struct exactly (two C doubles), which is what
// lets `msg_send!` return it by value.
unsafe impl Encode for Coordinate {
    const ENCODING: Encoding =
        Encoding::Struct("CLLocationCoordinate2D", &[f64::ENCODING, f64::ENCODING]);
}
unsafe impl RefEncode for Coordinate {
    const ENCODING_REF: Encoding = Encoding::Pointer(&Self::ENCODING);
}

define_class!(
    #[unsafe(super(NSObject))]
    // Creatable from any thread: `start` runs wherever the app called it — the main thread in a
    // Day app, which is also the run loop CoreLocation then delivers on.
    #[thread_kind = AllocAnyThread]
    #[name = "DayLocationDelegate"]
    #[ivars = ()]
    struct Delegate;

    /// The `CLLocationManagerDelegate` methods this arm needs. Objective-C dispatches through
    /// `respondsToSelector:`, so implementing the selectors is enough — no formal conformance
    /// declaration (and no framework wrapper crate) is required.
    impl Delegate {
        #[unsafe(method(locationManager:didUpdateLocations:))]
        fn did_update(&self, _manager: *mut AnyObject, locations: *mut AnyObject) {
            if locations.is_null() {
                return;
            }
            // SAFETY: the delegate contract hands us a non-empty NSArray<CLLocation *>; `lastObject`
            // is the newest and is nil only for an empty array, which is checked.
            let fix = unsafe {
                let last: *mut AnyObject = msg_send![locations, lastObject];
                if last.is_null() {
                    return;
                }
                read_fix(last)
            };
            crate::deliver(Ok(fix));
        }

        #[unsafe(method(locationManager:didFailWithError:))]
        fn did_fail(&self, _manager: *mut AnyObject, error: *mut AnyObject) {
            crate::deliver(Err(map_error(error)));
        }
    }
);

impl Delegate {
    fn new() -> Retained<Self> {
        let this = Self::alloc().set_ivars(());
        // SAFETY: NSObject's designated initializer.
        unsafe { msg_send![super(this), init] }
    }
}

/// Read every field of a `CLLocation`. CoreLocation signals "not measured" with a NEGATIVE accuracy
/// and with `-1` for speed and course, so those become `None` rather than nonsense numbers.
unsafe fn read_fix(loc: *mut AnyObject) -> Fix {
    unsafe {
        let coord: Coordinate = msg_send![loc, coordinate];
        let altitude: f64 = msg_send![loc, altitude];
        let h_acc: f64 = msg_send![loc, horizontalAccuracy];
        let v_acc: f64 = msg_send![loc, verticalAccuracy];
        let speed: f64 = msg_send![loc, speed];
        let course: f64 = msg_send![loc, course];
        let date: *mut AnyObject = msg_send![loc, timestamp];
        let timestamp_ms = if date.is_null() {
            None
        } else {
            let secs: f64 = msg_send![date, timeIntervalSince1970];
            Some((secs * 1000.0) as i64)
        };
        Fix {
            latitude: coord.latitude,
            longitude: coord.longitude,
            altitude: (v_acc >= 0.0).then_some(altitude),
            accuracy_m: (h_acc >= 0.0).then_some(h_acc),
            vertical_accuracy_m: (v_acc >= 0.0).then_some(v_acc),
            speed_mps: (speed >= 0.0).then_some(speed),
            course_deg: (course >= 0.0).then_some(course),
            timestamp_ms,
        }
    }
}

/// Map `kCLErrorDomain` codes: 1 = denied, 0 = location unknown (a transient "still trying").
fn map_error(error: *mut AnyObject) -> LocationError {
    if error.is_null() {
        return LocationError::Io("unknown CoreLocation error".into());
    }
    // SAFETY: the delegate contract hands us an NSError.
    let code: isize = unsafe { msg_send![error, code] };
    match code {
        1 => LocationError::PermissionDenied,
        0 => LocationError::Timeout,
        _ => LocationError::Io(format!("CoreLocation error {code}")),
    }
}

thread_local! {
    /// The live manager and its delegate. Both must outlive the updates, and neither is `Send`, so
    /// they live in the thread that started them — the main thread in a Day app.
    static ACTIVE: RefCell<Option<(Retained<AnyObject>, Retained<Delegate>)>> =
        const { RefCell::new(None) };
}

fn class(name: &str) -> Option<&'static AnyClass> {
    let c = std::ffi::CString::new(name).ok()?;
    AnyClass::get(&c)
}

pub fn is_available() -> bool {
    class("CLLocationManager").is_some()
}

/// CoreLocation's `desiredAccuracy` constants, as their documented values.
fn desired(acc: Accuracy) -> f64 {
    match acc {
        Accuracy::Best => -1.0,      // kCLLocationAccuracyBest
        Accuracy::Balanced => 100.0, // kCLLocationAccuracyHundredMeters
        Accuracy::Coarse => 1000.0,  // kCLLocationAccuracyKilometer
    }
}

pub fn start(acc: Accuracy) {
    let Some(cls) = class("CLLocationManager") else {
        crate::deliver(Err(LocationError::Unavailable));
        return;
    };
    ACTIVE.with(|active| {
        let mut active = active.borrow_mut();
        if active.is_some() {
            return;
        }
        // SAFETY: `+new` returns +1; the delegate is retained alongside the manager, and
        // `startUpdatingLocation` takes no arguments.
        unsafe {
            let manager: Option<Retained<AnyObject>> = msg_send![cls, new];
            let Some(manager) = manager else {
                crate::deliver(Err(LocationError::Unavailable));
                return;
            };
            let delegate = Delegate::new();
            let _: () = msg_send![&*manager, setDelegate: &*delegate];
            let _: () = msg_send![&*manager, setDesiredAccuracy: desired(acc)];
            let _: () = msg_send![&*manager, startUpdatingLocation];
            *active = Some((manager, delegate));
        }
    });
}

pub fn stop() {
    ACTIVE.with(|active| {
        if let Some((manager, _delegate)) = active.borrow_mut().take() {
            // SAFETY: takes no arguments; the manager is still alive here.
            unsafe {
                let _: () = msg_send![&*manager, stopUpdatingLocation];
                let _: () = msg_send![&*manager, setDelegate: std::ptr::null_mut::<AnyObject>()];
            }
        }
    });
}
