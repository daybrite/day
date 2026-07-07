// ---------------------------------------------------------------------------
// UIKit: MKMapView (MapKit) as a UIView. objc2-map-kit binds the MKMapView STRUCT for macOS only
// (the AppKit NSView subclass), so on iOS we hand-roll the class via `extern_class!` + `msg_send!`
// (exactly how the media piece hand-rolls AVPlayerViewController), reusing the crate's cross-platform
// MKCoordinateRegion / MKPointAnnotation. MapKit.framework must be linked or `+[MKMapView alloc]`
// aborts — declared via this crate's `[package.metadata.day.ios].frameworks = ["MapKit","CoreLocation"]`.
// ---------------------------------------------------------------------------

use super::*;
use day_spec::NodeId;
use day_uikit::Uikit;
use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, extern_class, msg_send};
use objc2_core_location::CLLocationCoordinate2D;
use objc2_map_kit::{MKCoordinateRegion, MKCoordinateSpan, MKPointAnnotation};
use objc2_ui_kit::{UIResponder, UIView};

// The iOS MKMapView (a UIView subclass). We only need a handful of methods, called via msg_send!.
extern_class!(
    #[unsafe(super(UIView, UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    struct MKMapView;
);

fn region(lat: f64, lon: f64, span: f64) -> MKCoordinateRegion {
    MKCoordinateRegion {
        center: CLLocationCoordinate2D {
            latitude: lat,
            longitude: lon,
        },
        span: MKCoordinateSpan {
            latitudeDelta: span,
            longitudeDelta: span,
        },
    }
}

fn make(_backend: &mut Uikit, p: &MapProps, _id: NodeId) -> Retained<UIView> {
    let mtm = MainThreadMarker::new().unwrap();
    // SAFETY: creates an MKMapView on the main thread and configures its region/annotation.
    let map: Retained<MKMapView> = unsafe { msg_send![MKMapView::alloc(mtm), init] };
    unsafe {
        let _: () = msg_send![&map, setRegion: region(p.lat, p.lon, p.span), animated: false];
    }
    if let Some((mlat, mlon)) = p.marker {
        let ann = unsafe { MKPointAnnotation::new() };
        unsafe {
            ann.setCoordinate(CLLocationCoordinate2D {
                latitude: mlat,
                longitude: mlon,
            });
            let _: () = msg_send![&map, addAnnotation: &*ann];
        }
    }
    Retained::from(<MKMapView as AsRef<UIView>>::as_ref(&map))
}

fn update(_backend: &mut Uikit, h: &Retained<UIView>, patch: &MapPatch) {
    let Some(map) = h.downcast_ref::<MKMapView>() else {
        return;
    };
    match patch {
        MapPatch::Center { lat, lon } => unsafe {
            let coord = CLLocationCoordinate2D {
                latitude: *lat,
                longitude: *lon,
            };
            let _: () = msg_send![map, setCenterCoordinate: coord, animated: true];
        },
    }
}

day_pieces::renderer!(day_uikit::RENDERERS, Uikit,
    kind: KIND, props: MapProps, patch: MapPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
