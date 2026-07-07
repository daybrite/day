// ---------------------------------------------------------------------------
// AppKit: MKMapView (MapKit) — objc2-map-kit binds it as an NSView subclass for macOS. `setRegion`
// takes an MKCoordinateRegion (center + span) built from the props; a marker is an MKPointAnnotation
// added to the map. The bound-center patch recenters via `setCenterCoordinate:animated:` (keeps the
// current zoom). MapKit renders keyless (no API token needed).
// ---------------------------------------------------------------------------

use super::*;
use day_appkit::AppKit;
use day_spec::NodeId;
use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::NSView;
use objc2_core_location::CLLocationCoordinate2D;
use objc2_map_kit::{MKCoordinateRegion, MKCoordinateSpan, MKMapView, MKPointAnnotation};

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

fn make(backend: &mut AppKit, p: &MapProps, _id: NodeId) -> Retained<NSView> {
    let mtm = backend.mtm();
    // SAFETY: creates an MKMapView on the main thread and configures its region/annotation.
    let view = unsafe { MKMapView::new(mtm) };
    unsafe { view.setRegion(region(p.lat, p.lon, p.span)) };
    if let Some((mlat, mlon)) = p.marker {
        let ann = unsafe { MKPointAnnotation::new() };
        unsafe {
            ann.setCoordinate(CLLocationCoordinate2D {
                latitude: mlat,
                longitude: mlon,
            });
            view.addAnnotation(ProtocolObject::from_ref(&*ann));
        }
    }
    Retained::from(<MKMapView as AsRef<NSView>>::as_ref(&view))
}

fn update(_backend: &mut AppKit, h: &Retained<NSView>, patch: &MapPatch) {
    let Some(view) = h.downcast_ref::<MKMapView>() else {
        return;
    };
    match patch {
        MapPatch::Center { lat, lon } => unsafe {
            view.setCenterCoordinate_animated(
                CLLocationCoordinate2D {
                    latitude: *lat,
                    longitude: *lon,
                },
                true,
            );
        },
    }
}

day_pieces::renderer!(day_appkit::RENDERERS, AppKit,
    kind: KIND, props: MapProps, patch: MapPatch,
    make: make, update: update, measure: day_pieces::fill_measure);
